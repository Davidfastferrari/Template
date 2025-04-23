use alloy::{
    eips::{ BlockId, Encodable2718 },
    consensus::Transaction,
    network::{ TransactionBuilder, EthereumWallet, Ethereum, Network },
    primitives::{ hex, address, U256, U160, Address, FixedBytes, Bytes },
    providers::{ Provider, ProviderBuilder, RootProvider},
    rpc::types::{ TransactionRequest, BlockNumberOrTag },
    rpc::types::{
        trace::geth::{ GethDebugTracingCallOptions, Bundle, StateContext, TransactionRequest, GethTrace, GethDebugTracerType, GethDebugBuiltInTracerType, PreStateConfig, GethDebugTracingOptions, GethDefaultTracingOptions, PreStateFrame, AccountState
    },
   signer::local::PrivateKeySigner,
   signer::k256::SecretKey,
      rpc::client::RpcClient,
    transports::http::{
        reqwest::{
            header::{HeaderMap, HeaderValue, AUTHORIZATION},
            Client,
        },
        Http,
    },
    sol,
    sol_types::{SolCall, SolValue, SolType},
};
use lazy_static::lazy_static;
use pool_sync::{Pool, PoolInfo};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use log::debug;

use calculation::Calculator;
use market_state::MarketState;
use swap::SwapPath;
use main::AMOUNT;

// Calculation constants
const RATE_SCALE: u32 = 18; // 18 decimals for rate precision
lazy_static {
    pub static ref RATE_SCALE_VALUE: U256 = U256::from(1e18);
}

// Handles initial estimation of path profitability before moving onto
// precise calculations and simulation
pub struct Estimator<T, N, P>
where
    T: Transport + Clone,
    N: Network,
    P: Provider<T, N>,
{
    // Mapping from pool address => token => rate
    rates: HashMap<Address, HashMap<Address, U256>>,
    // Tracks if a pool is based in weth
    weth_based: HashMap<Address, bool>,
    // Reference to the market_state
    market_state: Arc<MarketState<T, N, P>>,
    // Calculator to calculate the outputs of swaps
    calculator: Calculator<T, N, P>,
    // Maps from a quote token to its aggregated weth rate
    aggregated_weth_rate: HashMap<Address, U256>,
    // Decimals in token
    token_decimals: HashMap<Address, u32>,
}

impl<T, N, P> Estimator<T, N, P>
where
    T: Transport + Clone,
    N: Network,
    P: Provider<T, N>,
{
    // Construct a new estimator
    pub fn new(market_state: Arc<MarketState<T, N, P>>) -> Self {
        Self {
            rates: HashMap::new(),
            weth_based: HashMap::new(),
            market_state: market_state.clone(),
            calculator: Calculator::new(market_state.clone()),
            aggregated_weth_rate: HashMap::new(),
            token_decimals: HashMap::new(),
        }
    }

    // If a pools reserves were touched, update the exchange rate
    pub fn update_rates(&mut self, pool_addrs: &HashSet<Address>) {
        // get all pools corresponding to updated pool addresses
        let db = self.market_state.db.read().unwrap();
        let pools: Vec<Pool> = pool_addrs.iter().map(|p| db.get_pool(p).clone()).collect();
        drop(db);

        self.process_pools(pools);
    }

    // Given a path, estimate the output
    pub fn estimate_output_amount(&self, swap_path: &SwapPath) -> U256 {
        let mut current_amount = *AMOUNT;

        // Follow the path and apply rates sequentially
        for step in &swap_path.steps {
            if let Some(pool_rates) = self.rates.get(&step.pool_address) {
                if let Some(&rate) = pool_rates.get(&step.token_in) {
                    // Calculate: amount * rate / RATE_SCALE_VALUE
                    current_amount = current_amount
                        .checked_mul(rate)
                        .and_then(|v| v.checked_div(*RATE_SCALE_VALUE))
                        .unwrap_or(U256::ZERO);
                } else {
                    return U256::ZERO;
                }
            } else {
                return U256::ZERO;
            }
        }
        current_amount
    }

    // Given a swappath, estimate if it is profitable based on calculated rates
    pub fn is_profitable(&self, swap_path: &SwapPath, min_profit_ratio: U256) -> bool {
        let mut cumulative_rate = *RATE_SCALE_VALUE; // Start with 1.0 in our scaled format

        // Calculate the cumulative rate along the path
        for pool in &swap_path.steps {
            if let Some(pool_rates) = self.rates.get(&pool.pool_address) {
                if let Some(&rate) = pool_rates.get(&pool.token_in) {
                    cumulative_rate = cumulative_rate
                        .checked_mul(rate)
                        .and_then(|v| v.checked_div(*RATE_SCALE_VALUE))
                        .unwrap_or(U256::ZERO);
                } else {
                    return false;
                }
            } else {
                return false;
            }
        }
        // Check if rate exceeds 1.0 + min_profit_ratio
        cumulative_rate > (*RATE_SCALE_VALUE + min_profit_ratio)
    }

    // Scale a number to our rate precision
    fn scale_to_rate(&self, amount: U256, token_decimals: u32) -> U256 {
        if token_decimals <= RATE_SCALE {
            amount * U256::from(10u64.pow(RATE_SCALE - token_decimals))
        } else {
            amount / U256::from(10u64.pow(token_decimals - RATE_SCALE))
        }
    }

    // Calculate the exchange rate with proper scaling
    fn calculate_rate(
        &self,
        input: U256,
        output: U256,
        input_decimals: u32,
        output_decimals: u32,
    ) -> U256 {
        let scaled_input = self.scale_to_rate(input, input_decimals);
        let scaled_output = self.scale_to_rate(output, output_decimals);

        // Calcualte rate: (output * RATE_SCALE_VALUE) / input
        scaled_output
            .checked_mul(*RATE_SCALE_VALUE)
            .and_then(|v| v.checked_div(scaled_input))
            .unwrap_or(U256::ZERO)
    }

    // Given an initial set of filtered pools, estimate the exchange rates
    pub fn process_pools(&mut self, pools: Vec<Pool>) {
        let weth: Address = std::env::var("WETH").unwrap().parse().unwrap();
        let mut alt_tokens: HashSet<Address> = HashSet::new();
        let mut weth_alt_cnt: HashMap<Address, u32> = HashMap::new();

        // amount is our arb input, this is to generalize the exchange rates to
        // whatever we are trying to initially arb with
        let eth_input = *AMOUNT;

        // calcualte the rate for all pools with weth as a base/quote, we are very confident in these quotes
        for pool in pools
            .iter()
            .filter(|p| p.token0_address() == weth || p.token1_address() == weth)
        {
            debug!("Processing pool {}", pool.address());
            self.weth_based.insert(pool.address(), true);
            self.process_eth_pool(pool, weth, *AMOUNT, &mut alt_tokens, &mut weth_alt_cnt);
        }

        // update the alt rates
        for token in alt_tokens {
            if let Some(&count) = weth_alt_cnt.get(&token) {
                if let Some(rate) = self.aggregated_weth_rate.get_mut(&token) {
                    *rate = rate.checked_div(U256::from(count)).unwrap_or(U256::ZERO);
                }
            }
        }
        // calculate the ratio for all pools that weth is neither a base/quote, this will use
        // an averaged input from the corresponding weth pair
        for pool in pools
            .iter()
            .filter(|p| p.token0_address() != weth && p.token1_address() != weth)
        {
            debug!("Processing pool {}", pool.address());
            self.process_nonweth_pool(pool, eth_input);
        }
        // calculate every pool that is
    }

    // Calculate the rate for an weth based pool
    fn process_eth_pool(
        &mut self,
        pool: &Pool,
        weth: Address,
        input: U256,
        alt_tokens: &mut HashSet<Address>,
        weth_alt_cnt: &mut HashMap<Address, u32>,
    ) {
        let pool_address = pool.address();
        let token0 = pool.token0_address();
        let token1 = pool.token1_address();

        // insert the decimals
        self.token_decimals
            .insert(token0, pool.token0_decimals().into());
        self.token_decimals
            .insert(token1, pool.token1_decimals().into());

        // Get which token is weth and which is the quote token
        let (weth, alt) = if token0 == weth {
            (token0, token1)
        } else {
            (token1, token0)
        };
        alt_tokens.insert(alt);


        // get the output quote and then determine the rates
        let alt_output = self.calculator.compute_pool_output(
            pool_address,
            weth,
            pool.pool_type(),
            pool.fee(),
            input,
        );

        // Get decimals for both tokens
        let weth_decimals = self.token_decimals.get(&weth).unwrap_or(&18);
        let alt_decimals = self.token_decimals.get(&alt).unwrap_or(&18);


        let other_output = self.calculator.compute_pool_output(
            pool_address,
            alt,
            pool.pool_type(),
            pool.fee(),
            alt_output
        );

        // Calculate rates with proper scaling
        let zero_one_rate =
            self.calculate_rate(input, alt_output, *weth_decimals, *alt_decimals);
        let one_zero_rate =
            self.calculate_rate(alt_output, other_output, *alt_decimals, *weth_decimals);

        // Store rates
        self.rates
            .entry(pool_address)
            .or_default()
            .insert(token0, zero_one_rate);
        self.rates
            .entry(pool_address)
            .or_default()
            .insert(token1, one_zero_rate);


        // Update aggregated rate
        if weth == token0 {
            *self.aggregated_weth_rate.entry(alt).or_insert(U256::ZERO) += zero_one_rate;
        } else {
            *self.aggregated_weth_rate.entry(alt).or_insert(U256::ZERO) += one_zero_rate;
        }
        *weth_alt_cnt.entry(alt).or_insert(0) += 1;
    }

    fn process_nonweth_pool(&mut self, pool: &Pool, _input: U256) {
        let pool_address = pool.address();
        let token0 = pool.token0_address();
        let token1 = pool.token1_address();

        if let Some(&_input_rate) = self.aggregated_weth_rate.get(&token0) {
            let token0_decimals = self.token_decimals.get(&token0).unwrap_or(&18);
            //let scaled_input = U256::from(10u128).pow(U256::from(*token0_decimals));

            let output = self.calculator.compute_pool_output(
                pool_address,
                token0,
                pool.pool_type(),
                pool.fee(),
                _input_rate,
            );

            let token1_decimals = self.token_decimals.get(&token1).unwrap_or(&18);

            let other_output = self.calculator.compute_pool_output(
                pool_address,
                token1,
                pool.pool_type(),
                pool.fee(),
                output,
            );

            let zero_one_rate =
                self.calculate_rate(_input_rate, output, *token0_decimals, *token1_decimals);
            let one_zero_rate =
                self.calculate_rate(output, other_output, *token1_decimals, *token0_decimals);

            self.rates
                .entry(pool_address)
                .or_default()
                .insert(token0, zero_one_rate);
            self.rates
                .entry(pool_address)
                .or_default()
                .insert(token1, one_zero_rate);
        }
    }
}

#[cfg(test)]
mod estimator_tests {
    use super::*;
    use swap::SwapStep;
    use alloy::network::Ethereum;
    use alloy::primitives::address;
    use alloy::providers::{Provider, ProviderBuilder, RootProvider};
    use alloy::transports::http::{Client, Http};
    use pool_sync::PoolType;
    use pool_sync::UniswapV2Pool;
    use std::sync::mpsc;
    use tokio::sync::broadcast;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;

    // Create mock uniswapv2 weth/usdc pool
    fn uni_v2_weth_usdc() -> Pool {
        let pool = UniswapV2Pool {
            address: address!("88A43bbDF9D098eEC7bCEda4e2494615dfD9bB9C"),
            token0: address!("4200000000000000000000000000000000000006"),
            token1: address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
            token0_name: "WETH".to_string(),
            token1_name: "USDC".to_string(),
            token0_decimals: 18,
            token1_decimals: 6,
            token0_reserves: U256::from(325032740126871996707_u128),
            token1_reserves: U256::from(1014189875851_u128),
            stable: None,
            fee: None,
        };
        Pool::UniswapV2(pool)
    }

    // Create mock sushiswapv2 weth/usdc pool
    fn sushi_v2_weth_usdc() -> Pool {
        let pool = UniswapV2Pool {
            address: address!("2F8818D1B0f3e3E295440c1C0cDDf40aAA21fA87"),
            token0: address!("4200000000000000000000000000000000000006"),
            token1: address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
            token0_name: "WETH".to_string(),
            token1_name: "USDC".to_string(),
            token0_decimals: 18,
            token1_decimals: 6,
            token0_reserves: U256::from(324239280299976672116_u128),
            token1_reserves: U256::from(1016689282374_u128),
            stable: None,
            fee: None,
        };
        Pool::SushiSwapV2(pool)
    }

    // Mock the estimator for calculation
    async fn mock_estimator() -> Estimator<Http<Client>, Ethereum, RootProvider<Http<Client>>> {
        dotenv::dotenv().ok();
        let endpoint = std::env::var("FULL").unwrap().parse().unwrap();

        let uni_pool = uni_v2_weth_usdc();
        let sushi_pool = sushi_v2_weth_usdc();
        let pools = vec![uni_pool, sushi_pool];

        let (_, block_rx) = broadcast::channel(10);
        let (address_tx, _) = mpsc::channel();

        let provider = ProviderBuilder::new().on_http(endpoint);
        let block = provider.get_block_number().await.unwrap();

        let is_caught_up = Arc::new(AtomicBool::new(false));
        let market_state =
            MarketState::init_state_and_start_stream(pools, block_rx, address_tx, block, provider, is_caught_up.clone())
                .await
                .unwrap();
        while is_caught_up.load(Ordering::Relaxed) == false {}
        Estimator::new(market_state)
    }

    // Test that we can properly scale values to a desired precision
    #[tokio::test(flavor = "multi_thread")]
    async fn test_scale_to_rate() {
        let estimator = mock_estimator().await;

        // Scale up from 6 decimals
        let amount = U256::from(1_000_000); // 1 USDC
        let scaled = estimator.scale_to_rate(amount, 6);
        assert_eq!(scaled, U256::from(1e18));

        // Scale down from 24 decimals
        let amount = U256::from(1_000_000_000_000_000_000_000_000_u128);
        let scaled = estimator.scale_to_rate(amount, 24);
        assert_eq!(scaled, U256::from(1e18));
    }

    // Test that we compute the correct rate for a given input/output
    #[tokio::test(flavor = "multi_thread")]
    async fn test_calculate_rate() {
        let estimator = mock_estimator().await;

        // Test USDC (6 decimals) to ETH (18 decimals) rate
        let input = U256::from(1_000_000); // 1 USDC
        let output = U256::from(500_000_000_000_000_000u128); // 0.5 ETH
        let rate = estimator.calculate_rate(input, output, 6, 18);

        // Expected rate: 0.5 * 1e18 (representing 0.5 in fixed point)
        assert_eq!(rate, U256::from(500_000_000_000_000_000u128));
    }

    // Test if we can find a profitable path via rate estimation
    #[tokio::test(flavor = "multi_thread")]
    async fn test_profitable() {
        let mut estimator = mock_estimator().await;

        let uni_pool = uni_v2_weth_usdc();
        let sushi_pool = sushi_v2_weth_usdc();
        let pools = vec![uni_pool, sushi_pool];
        estimator.process_pools(pools);

        let not_profitable = SwapPath {
            steps: vec![
                SwapStep {
                    pool_address: address!("88A43bbDF9D098eEC7bCEda4e2494615dfD9bB9C"),
                    token_in: address!("4200000000000000000000000000000000000006"),
                    token_out: address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
                    protocol: PoolType::UniswapV2,
                    fee: 0,
                },
                SwapStep {
                    pool_address: address!("2F8818D1B0f3e3E295440c1C0cDDf40aAA21fA87"),
                    token_in: address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
                    token_out: address!("4200000000000000000000000000000000000006"),
                    protocol: PoolType::SushiSwapV2,
                    fee: 0,
                },
            ],
            hash: 0,
        };
        let profitable = SwapPath {
            steps: vec![
                SwapStep {
                    pool_address: address!("2F8818D1B0f3e3E295440c1C0cDDf40aAA21fA87"),
                    token_in: address!("4200000000000000000000000000000000000006"),
                    token_out: address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
                    protocol: PoolType::SushiSwapV2,
                    fee: 0,
                },
                SwapStep {
                    pool_address: address!("88A43bbDF9D098eEC7bCEda4e2494615dfD9bB9C"),
                    token_in: address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
                    token_out: address!("4200000000000000000000000000000000000006"),
                    protocol: PoolType::UniswapV2,
                    fee: 0,
                },
            ],
            hash: 0,
        };

        let no_profit = estimator.is_profitable(&not_profitable, U256::ZERO);
        let profit = estimator.is_profitable(&profitable, U256::ZERO);
        assert!(!no_profit);
        assert!(profit);
    }

}
