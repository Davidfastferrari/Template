use crate::gen::ERC20Token::{self, approveCall};
use crate::gen::{V2Aerodrome, V2Swap, V3Swap, V3SwapDeadline, V3SwapDeadlineTick};
use crate::AMOUNT;

use alloy::primitives::{address, Address, U160, U256};
use alloy::sol_types::{SolCall, SolValue};
use lazy_static::lazy_static;
use log::{info, debug};
use node_db::{InsertionType, NodeDB};
use pool_sync::{Chain, Pool, PoolInfo, PoolType};
use reqwest::header::{HeaderMap, HeaderValue};
use revm::primitives::{Bytes, ExecutionResult, FixedBytes, TransactTo};
use revm::{inspector_handle_register, Evm};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{create_dir_all, File};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::str::FromStr;
use revm_inspectors::access_list::AccessListInspector;
use revm_inspectors::access_list::InspectEvm;
use rayon::prelude::*;
use anyhow::{Result, anyhow, Context}; // add this if not already

/// Represents the logical router + calldata type for different swap protocols
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SwapType {
    V2Basic,
    V3Basic,
    V3Deadline,
    V2Aerodrome,
    V3DeadlineTick,
}

// Blacklisted tokens we don’t want to consider (e.g. scams, malicious)
lazy_static! {
    static ref BLACKLIST: Vec<Address> = vec![address!("be5614875952b1683cb0a2c20e6509be46d353a4")];
    static ref WETH_ADDRESS: Address = address!("4200000000000000000000000000000000000006");
}

// Common constants
const DEFAULT_PRIORITY_DIVISOR: usize = 50;
const SIMULATED_ACCOUNT: Address = address!("0000000000000000000000000000000000000001");
const FAKE_TOKEN_AMOUNT: U256 = U256::from_dec_str("10000000000000000000000000000000000000000").unwrap(); // 1e70
const MIN_OUTPUT_RATIO: u64 = 95;
const SIMULATED_GAS_LIMIT: u64 = 500_000;


/// Filter and validate pools based on volume and simulated liquidity
pub async fn filter_pools(pools: Vec<Pool>, num_results: usize, chain: Chain) -> Vec<Pool> {
    info!("Initial pool count before filter: {}", pools.len());

    let top_volume_tokens = get_top_volume_tokens(chain, num_results)
        .await
        .expect("Failed to fetch top-volume tokens from Birdeye");

    let filtered_by_token: Vec<Pool> = pools
        .into_par_iter()
        .filter(|pool| {
            let token0 = pool.token0_address();
            let token1 = pool.token1_address();
            top_volume_tokens.contains(&token0)
                && top_volume_tokens.contains(&token1)
                && !BLACKLIST.contains(&token0)
                && !BLACKLIST.contains(&token1)
        })
        .collect();

    info!("Pool count after token match filter: {}", filtered_by_token.len());

    let slot_map = construct_slot_map(&filtered_by_token);
    let pools = filter_by_swap(filtered_by_token, slot_map).await;

    debug!("Pool count after simulated swap filter: {}", pools.len());

    pools
}

#[derive(Serialize, Deserialize)]
struct TopVolumeAddresses(Vec<Address>);

#[derive(Debug, Deserialize)]
struct BirdeyeResponse {
    data: ResponseData,
}

#[derive(Debug, Deserialize)]
struct ResponseData {
    tokens: Vec<Token>,
}

#[derive(Debug, Deserialize)]
struct Token {
    address: String,
}

/// Get top volume tokens from Birdeye or cache
async fn get_top_volume_tokens(chain: Chain, num_results: usize) -> Result<Vec<Address>> {
    let cache_file = format!("cache/top_volume_tokens_{}.json", chain);

    if Path::new(&cache_file).exists() {
        return read_addresses_from_file(&cache_file)
            .context("Failed to read cached volume tokens");
    }

    let tokens = fetch_top_volume_tokens(num_results, chain).await;
    create_dir_all("cache")?;
    write_addresses_to_file(&tokens, &cache_file)?;

    Ok(tokens)
}

fn write_addresses_to_file(addresses: &[Address], filename: &str) -> std::io::Result<()> {
    let file = File::create(filename)?;
    let writer = BufWriter::new(file);
    let address_set = TopVolumeAddresses(addresses.to_vec());
    serde_json::to_writer(writer, &address_set)?;
    Ok(())
}

fn read_addresses_from_file(filename: &str) -> Result<Vec<Address>> {
    let file = File::open(filename)?;
    let reader = BufReader::new(file);
    let address_set: TopVolumeAddresses = serde_json::from_reader(reader)?;
    Ok(address_set.0)
}

async fn fetch_top_volume_tokens(num_results: usize, chain: Chain) -> Vec<Address> {
    let client = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(10))
    .build()
    .expect("Failed to build HTTP client");
    let mut headers = HeaderMap::new();
    let api_key = std::env::var("BIRDEYE_KEY").expect("BIRDEYE_KEY not set");

    headers.insert("X-API-KEY", HeaderValue::from_str(&api_key).unwrap());
    headers.insert("x-chain", HeaderValue::from_str(match chain {
        Chain::Ethereum => "ethereum",
        Chain::Base => "base",
        _ => "unknown",
    }).unwrap());

    let mut query_params = vec![];
    for offset in (0..num_results).step_by(DEFAULT_PRIORITY_DIVISOR) {
        let limit = std::cmp::min(DEFAULT_PRIORITY_DIVISOR, num_results - offset);
        query_params.push((offset, limit));
    }

    let mut addresses = vec![];

    for (offset, limit) in query_params {
        let response = client
            .get("https://public-api.birdeye.so/defi/tokenlist")
            .headers(headers.clone())
            .query(&[
                ("sort_by", "v24hUSD"),
                ("sort_type", "desc"),
                ("offset", &offset.to_string()),
                ("limit", &limit.to_string()),
            ])
            .send()
            .await
            .unwrap();

        if response.status().is_success() {
            let parsed: BirdeyeResponse = response
    .json()
    .await
    .with_context(|| format!("Failed to decode Birdeye response at offset {}, limit {}", offset, limit))?;
            addresses.extend(parsed.data.tokens.into_iter().map(|t| t.address));
        }
    }

    addresses
        .into_iter()
        .filter_map(|addr| Address::from_str(&addr).ok())
        .collect()
}


async fn filter_by_swap(
    pools: Vec<Pool>,
    slot_map: HashMap<Address, FixedBytes<32>>
) -> Result<Vec<Pool>> {
    let mut filtered = Vec::with_capacity(pools.len());
    let db_path = std::env::var("DB_PATH").expect("DB_PATH must be set");
    let mut nodedb = NodeDB::new(&db_path).expect("Failed to open NodeDB");

    for pool in pools {
        let (router, swap_type) = match resolve_router_and_type(pool.pool_type()) {
            Some(x) => x,
            None => continue,
        };

        let zero_to_one = determine_swap_direction(&pool);

        // Skip if slots unknown
        let slot0 = match slot_map.get(&pool.token0_address()) {
            Some(s) => *s,
            None => continue,
        };
        let slot1 = match slot_map.get(&pool.token1_address()) {
            Some(s) => *s,
            None => continue,
        };

        // Inject big balance into both tokens
        for (token, slot) in [(pool.token0_address(), slot0), (pool.token1_address(), slot1)] {
            nodedb.insert_account_storage(token, slot.into(), FAKE_TOKEN_AMOUNT, InsertionType::OnChain)
                .map_err(|e| anyhow!("Failed to insert account storage: {}", e))?;
        }

        // Prepare EVM
        let mut evm = InspectEvm::builder()
            .with_db(&mut nodedb)
            .modify_tx_env(|tx| {
                tx.caller = SIMULATED_ACCOUNT;
                tx.value = U256::ZERO;
                tx.gas_limit = SIMULATED_GAS_LIMIT;
            })
            .build();

        // Approve router for token0 and token1
        for token in [pool.token0_address(), pool.token1_address()] {
            evm.tx_mut().data = approveCall {
                spender: router,
                amount: FAKE_TOKEN_AMOUNT,
            }.abi_encode().into();

            evm.tx_mut().transact_to = TransactTo::Call(token);

            evm.transact_commit()
                .ok_or(anyhow!("Approval transaction failed"))?;
        }

        // Simulate forward + backward swap
        let amt = *AMOUNT;
        let min_expected = amt * U256::from(MIN_OUTPUT_RATIO) / U256::from(100);

        let forward = simulate_swap(&mut evm, &pool, swap_type, router, SIMULATED_ACCOUNT, amt, zero_to_one)
            .ok_or(anyhow!("Forward swap simulation failed"))?;

        let backward = simulate_swap(&mut evm, &pool, swap_type, router, SIMULATED_ACCOUNT, forward, !zero_to_one)
            .ok_or(anyhow!("Backward swap simulation failed"))?;

        if backward >= min_expected {
            filtered.push(pool.clone());
        }
    }

    Ok(filtered)
}

fn simulate_swap(
    evm: &mut InspectEvm,
    pool: &Pool,
    swap_type: SwapType,
    router: Address,
    account: Address,
    amount: U256,
    zero_to_one: bool,
) -> Option<U256> {
    let (calldata, is_vec) = setup_router_calldata(pool.clone(), account, amount, swap_type, zero_to_one);
    evm.tx_mut().transact_to = TransactTo::Call(router);
    evm.tx_mut().data = calldata.into();

    let res = evm.transact().ok()?.result;

    match res {
        ExecutionResult::Success { .. } => {
            let out = res.output()?;
            Some(decode_swap_return(out, is_vec))
        }
        _ => None,
    }
}

fn decode_swap_return(output: &Bytes, is_vec: bool) -> U256 {
    if is_vec {
        let vec_out = match <Vec<U256>>::abi_decode(output, false) {
           Ok(v) => v,
           Err(e) => {
           debug!("Vec decode failed: {:?}", e);
        return U256::ZERO;
        }
      };

        *vec_out.last().unwrap()
    } else {
        <U256>::abi_decode(output, false).unwrap()
    }
}

fn resolve_router_and_type(pt: PoolType) -> Option<(Address, SwapType)>{
    use PoolType::*;
    match pt {
        UniswapV2 => (address!("4752..."), SwapType::V2Basic),
        SushiSwapV2 => (address!("6BDE..."), SwapType::V2Basic),
        PancakeSwapV2 => (address!("8cFe..."), SwapType::V2Basic),
        UniswapV3 => (address!("2626..."), SwapType::V3Basic),
        SushiSwapV3 => (address!("FB7e..."), SwapType::V3Deadline),
        Aerodrome => (address!("cF77..."), SwapType::V2Aerodrome),
        Slipstream => (address!("BE6D..."), SwapType::V3DeadlineTick),
       _ => return None,
    }
}

fn determine_swap_direction(pool: &Pool) -> bool {
    if pool.token0_address() == *WETH_ADDRESS {
        true
    } else if pool.token1_address() == *WETH_ADDRESS {
        false
    } else {
        true // default
    }
}

fn setup_router_calldata(
    pool: Pool,
    account: Address,
    amt: U256,
    swap_type: SwapType,
    zero_to_one: bool,
) -> (Vec<u8>, bool) {
    // Determine the correct token order
    let (token0, token1) = if zero_to_one {
        (pool.token0_address(), pool.token1_address())
    } else {
        (pool.token1_address(), pool.token0_address())
    };

    match swap_type {
        SwapType::V2Basic => {
            let calldata = V2Swap::swapExactTokensForTokensCall {
                amountIn: amt,
                amountOutMin: U256::ZERO,
                path: vec![token0, token1],
                to: account,
                deadline: U256::MAX,
            }
            .abi_encode();
            (calldata, true)
        }
        SwapType::V3Basic => {
            let swap_fee = pool.get_v3().expect("Missing pool details for V3Basic").fee;
            let params = V3Swap::ExactInputSingleParams {
                tokenIn: token0,
                tokenOut: token1,
                fee: swap_fee.try_into().expect("Invalid fee conversion"),
                recipient: account,
                amountIn: amt,
                amountOutMinimum: U256::ZERO,
                sqrtPriceLimitX96: U160::ZERO,
            };
            (V3Swap::exactInputSingleCall { params }.abi_encode(), false)
        }
        SwapType::V3Deadline => {
            let swap_fee = pool.get_v3().expect("Missing pool details for V3Deadline").fee;
            let params = V3SwapDeadline::ExactInputSingleParams {
                tokenIn: token0,
                tokenOut: token1,
                fee: swap_fee.try_into().expect("Invalid fee conversion"),
                recipient: account,
                deadline: U256::MAX,
                amountIn: amt,
                amountOutMinimum: U256::ZERO,
                sqrtPriceLimitX96: U160::ZERO,
            };
            (
                V3SwapDeadline::exactInputSingleCall { params }.abi_encode(),
                false,
            )
        }
        SwapType::V2Aerodrome => {
            let is_stable = pool
                .get_v2()
                .expect("Missing pool details for V2Aerodrome")
                .stable
                .expect("Missing 'stable' flag for Aerodrome pool");

            let route = vec![V2Aerodrome::Route {
                from: token0,
                to: token1,
                stable: is_stable,
                factory: Address::ZERO,
            }];

            let calldata = V2Aerodrome::swapExactTokensForTokensCall {
                amountIn: amt,
                amountOutMin: U256::ZERO,
                routes: route,
                to: account,
                deadline: U256::MAX,
            }
            .abi_encode();
            (calldata, true)
        }
        SwapType::V3DeadlineTick => {
            let tick_spacing = pool
                .get_v3()
                .expect("Missing pool details for V3DeadlineTick")
                .tick_spacing;

            let params = V3SwapDeadlineTick::ExactInputSingleParams {
                tokenIn: token0,
                tokenOut: token1,
                tickSpacing: tick_spacing.try_into().expect("Invalid tick_spacing"),
                recipient: account,
                deadline: U256::MAX,
                amountIn: amt,
                amountOutMinimum: U256::ZERO,
                sqrtPriceLimitX96: U160::ZERO,
            };
            (
                V3SwapDeadlineTick::exactInputSingleCall { params }.abi_encode(),
                false,
            )
        }
    }
}

