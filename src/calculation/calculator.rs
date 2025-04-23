use alloy::{
    consensus::Transaction,
    network::{TransactionBuilder, EthereumWallet, Ethereum, Network, BlockResponse, HeaderResponse}
    primitives::{BlockNumber, B256, I256, hex, address, U256, U160, Address, FixedBytes, Bytes},
    providers::{Provider, ProviderBuilder, RootProvider},
      rpc::client::RpcClient,
    transports::http::{
        reqwest::{
            header::{HeaderMap, HeaderValue, AUTHORIZATION},
            Client,
        },
        Http,
      Transport,
      TransportError 
    },
 sol,
 sol_types::{SolCall, SolValue, SolType},
};
use pool_sync::PoolType;
use std::collections::HashSet;
use std::sync::Arc;

use crate::cache::Cache;
use crate::market_state::MarketState;
use crate::swap::SwapPath;
use crate::AMOUNT;

// Calculator for getting the amount out
pub struct Calculator<T, N, P>
where
    T: Transport + Clone,
    N: Network,
    P: Provider<T, N>,
{
    pub market_state: Arc<MarketState<T, N, P>>,
    pub cache: Arc<Cache>,
}

impl<T, N, P> Calculator<T, N, P>
where
    T: Transport + Clone,
    N: Network,
    P: Provider<T, N>,
{
    // construct a new calculator
    // contains the market state to access pool info and a cache for calculations
    pub fn new(market_state: Arc<MarketState<T, N, P>>) -> Self {
        Self {
            market_state,
            cache: Arc::new(Cache::new(500)),
        }
    }

    // calculate the output amount
    // we can get read access to the db since we know it will not change for duration of calculation
    #[inline]
    pub fn calculate_output(&self, path: &SwapPath) -> U256 {
        let mut amount = *AMOUNT;

        // for each step, calculate the amount out
        for swap_step in &path.steps {
            let pool_address = swap_step.pool_address;

            // check to see if we have a up to date cache
            if let Some(cached_amount) = self.cache.get(amount, pool_address) {
                amount = cached_amount;
            } else {
                // compute the output amount and then store it in cache
                let output_amount = self.compute_amount_out(
                    amount,
                    pool_address,
                    swap_step.token_in,
                    swap_step.protocol,
                    swap_step.fee,
                );
                self.cache.set(amount, pool_address, output_amount);
                amount = output_amount;
            }

            if amount == U256::ZERO {
               return U256::ZERO;
            }
        }

        // all good, return the output amount of the path
        amount
    }

    pub fn debug_calculation(&self, path: &SwapPath) -> Vec<U256> {
        let mut path_calc: Vec<U256> = Vec::new();
        let mut amount = *AMOUNT;
        path_calc.push(amount);

        for swap_step in &path.steps {
            let pool_address = swap_step.pool_address;
            let output_amount = self.compute_amount_out(
                amount,
                pool_address,
                swap_step.token_in,
                swap_step.protocol,
                swap_step.fee,
            );
            path_calc.push(output_amount);
            amount = output_amount;
        }

        path_calc
    }

    pub fn compute_pool_output(&self, pool_addr: Address, token_in: Address, protocol: PoolType, fee: u32, input: U256) -> U256 {
        self.compute_amount_out(
            input,
            pool_addr,
            token_in,
            protocol,
            fee
        )
    }

    // calculate the ratio for the pool
    pub fn compute_amount_out(
        &self,
        input_amount: U256,
        pool_address: Address,
        token_in: Address,
        pool_type: PoolType,
        fee: u32,
    ) -> U256 {
        match pool_type {
            PoolType::UniswapV2 | PoolType::SushiSwapV2 | PoolType::SwapBasedV2 => {
                self.uniswap_v2_out(input_amount, &pool_address, &token_in, U256::from(9970))
            }
            PoolType::PancakeSwapV2 | PoolType::BaseSwapV2 | PoolType::DackieSwapV2 => {
                self.uniswap_v2_out(input_amount, &pool_address, &token_in, U256::from(9975))
            }
            PoolType::AlienBaseV2 => {
                self.uniswap_v2_out(input_amount, &pool_address, &token_in, U256::from(9984))
            }
            PoolType::UniswapV3
            | PoolType::SushiSwapV3
            | PoolType::BaseSwapV3
            | PoolType::Slipstream
            | PoolType::PancakeSwapV3
            | PoolType::AlienBaseV3
            | PoolType::SwapBasedV3
            | PoolType::DackieSwapV3 => self
                .uniswap_v3_out(input_amount, &pool_address, &token_in, fee)
                .unwrap(),
            PoolType::Aerodrome => self.aerodrome_out(input_amount, token_in, pool_address),
            PoolType::MaverickV1 | PoolType::MaverickV2 => todo!(),
            PoolType::BalancerV2 => todo!(),
            PoolType::CurveTwoCrypto | PoolType::CurveTriCrypto => todo!()
        }
    }

    #[inline]
    pub fn invalidate_cache(&self, updated_pools: &HashSet<Address>) {
        for pool in updated_pools {
            self.cache.invalidate(*pool)
        }
    }
}
