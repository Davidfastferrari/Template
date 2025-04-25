use alloy::network::Network;
use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use alloy::transports::Transport;
use pool_sync::PoolType;
use std::collections::HashSet;
use std::sync::Arc;
use revm::revm_database::alloydb::DBTransportError; // ✅ fix: valid DBErrorMarker implementor

use crate::cache::Cache;
use crate::market_state::MarketState;
use crate::swap::{SwapPath, SwapStep};
use crate::AMOUNT;

pub fn uniswap_v2_out(&self, input: U256, pool: &Address, token: &Address, fee: U256) -> U256 {
    // TODO: Actual implementation
    U256::ZERO
}

pub fn uniswap_v3_out(&self, input: U256, pool: &Address, token: &Address, fee: u32) -> Option<U256> {
    // TODO: Actual implementation
    Some(U256::ZERO)
}

pub fn aerodrome_out(&self, input: U256, token: Address, pool: Address) -> U256 {
    // TODO: Actual implementation
    U256::ZERO
}

pub fn balancer_v2_out(&self, input: U256, token_in: Address, token_out: Address, pool: Address) -> U256 {
    // TODO: Actual implementation
    U256::ZERO
}

/// Calculator handles swap output computations across supported AMM types.
/// Calculator handles swap output computations across supported AMM types.
pub struct Calculator<N, P>
where
    N: Network,
    P: Provider<N>,
{
    pub market_state: Arc<MarketState<N, P>>,
    pub cache: Arc<Cache>,
}

impl<N, P> Calculator<N, P>
where
    N: Network,
    P: Provider<N>,
{
    /// Create a new calculator instance with market state and internal cache.
     pub fn new(market_state: Arc<MarketState<N, P>>) -> Self {
        Self {
            market_state,
            cache: Arc::new(Cache::new(500)),
        }

    /// Perform output amount calculation for a given swap path
    #[inline(always)]
    pub fn calculate_output(&self, path: &SwapPath) -> U256 {
        let mut amount = *AMOUNT;

        for swap_step in &path.steps {
            let pool_address = swap_step.pool_address;

            // Fast path: use cached result
            if let Some(cached_amount) = self.cache.get(amount, pool_address) {
                amount = cached_amount;
            } else {
                // Compute fresh and store in cache
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

            if amount.is_zero() {
                return U256::ZERO;
            }
        }

        amount
    }

    /// Return a debug trace of intermediate amounts at each swap step
    pub fn debug_calculation(&self, path: &SwapPath) -> Vec<U256> {
        let mut amount = *AMOUNT;
        let mut path_trace = vec![amount];

        for swap_step in &path.steps {
            let output_amount = self.compute_amount_out(
                amount,
                swap_step.pool_address,
                swap_step.token_in,
                swap_step.protocol,
                swap_step.fee,
            );
            path_trace.push(output_amount);
            amount = output_amount;
        }

        path_trace
    }

    /// Calculate pool output with arguments unwrapped from path logic
    pub fn compute_pool_output(
        &self,
        pool_addr: Address,
        token_in: Address,
        protocol: PoolType,
        fee: u32,
        input: U256,
    ) -> U256 {
        self.compute_amount_out(input, pool_addr, token_in, protocol, fee)
    }

    /// Main dispatcher for computing output amount based on AMM type
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
            | PoolType::DackieSwapV3 => {
                self.uniswap_v3_out(input_amount, &pool_address, &token_in, fee)
                    .expect("Uniswap V3 computation failed")
            }
            PoolType::Aerodrome => self.aerodrome_out(input_amount, token_in, pool_address),
            PoolType::BalancerV2 => self.balancer_v2_out(input_amount, token_in, token_in, pool_address),
            PoolType::MaverickV1 | PoolType::MaverickV2 => {
                tracing::warn!("Maverick pool logic not implemented");
                U256::ZERO
            }
            PoolType::CurveTwoCrypto | PoolType::CurveTriCrypto => {
                tracing::warn!("Curve pool logic not implemented");
                U256::ZERO
            }
        }
    }

    /// Invalidate cache for specific pools (e.g. after state mutation)
    #[inline(always)]
    pub fn invalidate_cache(&self, updated_pools: &HashSet<Address>) {
        for pool in updated_pools {
            self.cache.invalidate(*pool)
        }
    }
}

impl<T, N, P> Calculator<T, N, P>
where
    T: Transport + Clone,
    N: Network,
    P: Provider<T, N>,
{
    /// Simulate sandwich backrun MEV strategy
    ///
    /// A -> B (user trade) -> A' (MEV backrun)
    pub fn simulate_sandwich_mev(
        &self,
        frontrun: &SwapPath,
        victim: &SwapPath,
        backrun: &SwapPath,
    ) -> (U256, U256, U256) {
        let front_output = self.calculate_output(frontrun);
        let victim_output = self.calculate_output(victim);
        let back_output = self.calculate_output(backrun);

        let total_profit = back_output.saturating_sub(frontrun.steps.first().map(|s| *AMOUNT).unwrap_or_default());

        (front_output, victim_output, total_profit)
    }

    /// Generate backrun path from a base path by flipping it
    pub fn reverse_path(&self, original: &SwapPath) -> SwapPath {
        let mut steps = original.steps.clone();
        steps.reverse();

        let reversed_steps = steps
            .into_iter()
            .map(|step| SwapStep {
                pool_address: step.pool_address,
                token_in: step.token_out,
                token_out: step.token_in,
                protocol: step.protocol,
                fee: step.fee,
            })
            .collect();

        SwapPath {
            steps: reversed_steps,
            hash: original.hash,
        }
    }
}
