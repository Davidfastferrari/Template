use alloy::network::Network;
use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use alloy::transports::Transport;
use log::debug;
use once_cell::sync::Lazy;
use pool_sync::{Pool, PoolInfo};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::calculation::Calculator;
use crate::market_state::MarketState;
use crate::swap::SwapPath;
use crate::AMOUNT;

// Constants
const RATE_SCALE: u32 = 18;

// Using once_cell instead of lazy_static (more idiomatic and simpler)
pub static RATE_SCALE_VALUE: Lazy<U256> = Lazy::new(|| U256::from(10).pow(U256::from(RATE_SCALE)));

/// The `Estimator` is used to estimate profitability of paths via pre-calculated exchange rates.
pub struct Estimator<N, P>
where
    N: Network,
    P: Provider<N>,
{
    rates: HashMap<Address, HashMap<Address, U256>>,
    weth_based: HashMap<Address, bool>,
    market_state: Arc<MarketState< N, P>>,
    calculator: Calculator<N, P>,
    aggregated_weth_rate: HashMap<Address, U256>,
    token_decimals: HashMap<Address, u32>,
}

impl<N, P> Estimator<N, P>
where
    N: Network,
    P: Provider<N>,
{
    pub fn new(market_state: Arc<MarketState<N, P>>) -> Self {
        Self {
            rates: HashMap::new(),
            weth_based: HashMap::new(),
            market_state: Arc::clone(&market_state),
            calculator: Calculator::new(market_state),
            aggregated_weth_rate: HashMap::new(),
            token_decimals: HashMap::new(),
        }
    }

    pub fn update_rates(&mut self, pool_addrs: &HashSet<Address>) {
        let db = self.market_state.db.read().unwrap();
        let pools: Vec<Pool> = pool_addrs.iter().filter_map(|p| db.get_pool(p)).cloned().collect();
        drop(db);
        self.process_pools(pools);
    }

    pub fn estimate_output_amount(&self, path: &SwapPath) -> U256 {
        path.steps.iter().fold(*AMOUNT, |amount, step| {
            self.rates
                .get(&step.pool_address)
                .and_then(|m| m.get(&step.token_in))
                .and_then(|rate| amount.checked_mul(*rate))
                .and_then(|v| v.checked_div(*RATE_SCALE_VALUE))
                .unwrap_or(U256::ZERO)
        })
    }

    pub fn is_profitable(&self, path: &SwapPath, min_profit_ratio: U256) -> bool {
        let final_rate = path.steps.iter().fold(*RATE_SCALE_VALUE, |rate, step| {
            self.rates
                .get(&step.pool_address)
                .and_then(|m| m.get(&step.token_in))
                .and_then(|step_rate| rate.checked_mul(*step_rate))
                .and_then(|v| v.checked_div(*RATE_SCALE_VALUE))
                .unwrap_or(U256::ZERO)
        });
        final_rate > (*RATE_SCALE_VALUE + min_profit_ratio)
    }

    fn scale_to_rate(&self, amount: U256, token_decimals: u32) -> U256 {
        if token_decimals <= RATE_SCALE {
            amount * U256::exp10((RATE_SCALE - token_decimals) as usize)
        } else {
            amount / U256::exp10((token_decimals - RATE_SCALE) as usize)
        }
    }

    fn calculate_rate(
        &self,
        input: U256,
        output: U256,
        in_decimals: u32,
        out_decimals: u32,
    ) -> U256 {
        let input_scaled = self.scale_to_rate(input, in_decimals);
        let output_scaled = self.scale_to_rate(output, out_decimals);
        output_scaled
            .checked_mul(*RATE_SCALE_VALUE)
            .and_then(|v| v.checked_div(input_scaled))
            .unwrap_or(U256::ZERO)
    }

    pub fn process_pools(&mut self, pools: Vec<Pool>) {
        let weth: Address = std::env::var("WETH").unwrap().parse().unwrap();
        let mut alt_tokens: HashSet<Address> = HashSet::new();
        let mut weth_alt_cnt: HashMap<Address, u32> = HashMap::new();

        for pool in &pools {
            let has_weth = pool.token0_address() == weth || pool.token1_address() == weth;
            if has_weth {
                self.weth_based.insert(pool.address(), true);
                self.process_eth_pool(pool, weth, *AMOUNT, &mut alt_tokens, &mut weth_alt_cnt);
            }
        }

        for token in &alt_tokens {
            if let Some(cnt) = weth_alt_cnt.get(token) {
                if let Some(rate) = self.aggregated_weth_rate.get_mut(token) {
                    *rate /= U256::from(*cnt);
                }
            }
        }

        for pool in &pools {
            if pool.token0_address() != weth && pool.token1_address() != weth {
                self.process_nonweth_pool(pool, *AMOUNT);
            }
        }
    }

    fn process_eth_pool(
        &mut self,
        pool: &Pool,
        weth: Address,
        input: U256,
        alt_tokens: &mut HashSet<Address>,
        cnt_map: &mut HashMap<Address, u32>,
    ) {
        let (token0, token1) = (pool.token0_address(), pool.token1_address());
        self.token_decimals.insert(token0, pool.token0_decimals());
        self.token_decimals.insert(token1, pool.token1_decimals());

        let (eth_token, alt_token) = if token0 == weth { (token0, token1) } else { (token1, token0) };
        alt_tokens.insert(alt_token);

        let output = self.calculator.compute_pool_output(
            pool.address(),
            eth_token,
            pool.pool_type(),
            pool.fee(),
            input,
        );

        let back_output = self.calculator.compute_pool_output(
            pool.address(),
            alt_token,
            pool.pool_type(),
            pool.fee(),
            output,
        );

        let in_dec = *self.token_decimals.get(&eth_token).unwrap_or(&18);
        let out_dec = *self.token_decimals.get(&alt_token).unwrap_or(&18);

        let rate_eth_to_alt = self.calculate_rate(input, output, in_dec, out_dec);
        let rate_alt_to_eth = self.calculate_rate(output, back_output, out_dec, in_dec);

        self.rates.entry(pool.address()).or_default().insert(eth_token, rate_eth_to_alt);
        self.rates.entry(pool.address()).or_default().insert(alt_token, rate_alt_to_eth);

        *self.aggregated_weth_rate.entry(alt_token).or_insert(U256::ZERO) += rate_eth_to_alt;
        *cnt_map.entry(alt_token).or_insert(0) += 1;
    }

    fn process_nonweth_pool(&mut self, pool: &Pool, input: U256) {
        let (token0, token1) = (pool.token0_address(), pool.token1_address());
        let decimals0 = *self.token_decimals.get(&token0).unwrap_or(&18);
        let decimals1 = *self.token_decimals.get(&token1).unwrap_or(&18);

        if let Some(&input_rate) = self.aggregated_weth_rate.get(&token0) {
            let output = self.calculator.compute_pool_output(pool.address(), token0, pool.pool_type(), pool.fee(), input_rate);
            let back = self.calculator.compute_pool_output(pool.address(), token1, pool.pool_type(), pool.fee(), output);

            let rate0 = self.calculate_rate(input_rate, output, decimals0, decimals1);
            let rate1 = self.calculate_rate(output, back, decimals1, decimals0);

            self.rates.entry(pool.address()).or_default().insert(token0, rate0);
            self.rates.entry(pool.address()).or_default().insert(token1, rate1);
        }
    }
}

#[cfg(test)]
mod estimator_tests {
    use super::*;
    use crate::swap::SwapStep;
    use alloy::network::Ethereum;
    use alloy::primitives::address;
    use alloy::providers::{ProviderBuilder, RootProvider};
    use alloy::transports::http::{Client, Http};
    use pool_sync::{PoolType, UniswapV2Pool};
    use std::sync::{mpsc, Arc};
    use tokio::sync::broadcast;
    use std::sync::atomic::{AtomicBool, Ordering};
    use proptest::prelude::*;

    // Create a sample Uniswap V2 pool
    fn mock_uni_v2_pool() -> Pool {
        Pool::UniswapV2(UniswapV2Pool {
            address: address!("88A43bbDF9D098eEC7bCEda4e2494615dfD9bB9C"),
            token0: address!("4200000000000000000000000000000000000006"),
            token1: address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
            token0_name: "WETH".to_string(),
            token1_name: "USDC".to_string(),
            token0_decimals: 18,
            token1_decimals: 6,
            token0_reserves: U256::from(3e18 as u128),
            token1_reserves: U256::from(1_000_000_000 as u128),
            stable: None,
            fee: None,
        })
    }

    async fn setup_estimator() -> Estimator<Http<Client>, Ethereum, RootProvider<Http<Client>>> {
        dotenv::dotenv().ok();
        let endpoint = std::env::var("FULL").unwrap().parse().unwrap();
        let provider = ProviderBuilder::new().on_http(endpoint);

        let (block_tx, block_rx) = broadcast::channel(1);
        let (address_tx, _) = mpsc::channel();
        let pool = mock_uni_v2_pool();
        let last_block = provider.get_block_number().await.unwrap();
        let ready = Arc::new(AtomicBool::new(false));

        let state = MarketState::init_state_and_start_stream(
            vec![pool],
            block_rx,
            address_tx,
            last_block,
            provider,
            ready.clone(),
        )
        .await
        .unwrap();

        while !ready.load(Ordering::Relaxed) {}
        Estimator::new(state)
    }

    proptest! {
        #[test]
        fn test_scale_preserves_unit(amount in 1_000_000u64..1_000_000_000u64, decimals in 0u32..36) {
            let estimator = Estimator::<Http<Client>, Ethereum, RootProvider<Http<Client>>>::new(Arc::new(
                MarketState {
                    db: RwLock::new(MockBlockStateDB::default()) // You should define MockBlockStateDB
                }
            ));
            let scaled = estimator.scale_to_rate(U256::from(amount), decimals);
            prop_assert!(scaled > U256::ZERO);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_rate_and_scaling_consistency() {
        let estimator = setup_estimator().await;

        let input = U256::from(1_000_000); // 1 USDC
        let output = U256::from(500_000_000_000_000_000u128); // 0.5 ETH

        let rate = estimator.calculate_rate(input, output, 6, 18);
        assert_eq!(rate, U256::from(5e17 as u128));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_path_profitability_estimate() {
        let mut estimator = setup_estimator().await;
        estimator.process_pools(vec![mock_uni_v2_pool()]);

        let path = SwapPath {
            steps: vec![
                SwapStep {
                    pool_address: address!("88A43bbDF9D098eEC7bCEda4e2494615dfD9bB9C"),
                    token_in: address!("4200000000000000000000000000000000000006"),
                    token_out: address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
                    protocol: PoolType::UniswapV2,
                    fee: 0,
                },
            ],
            hash: 42,
        };

        let est_output = estimator.estimate_output_amount(&path);
        assert!(est_output > U256::zero());
    }
}
