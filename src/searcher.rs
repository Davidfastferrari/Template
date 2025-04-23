use alloy::{
    eips::{ BlockId, Encodable2718 },
    consensus::Transaction,
    network::{ TransactionBuilder, EthereumWallet, Ethereum, Network }
    primitives::{ hex, address, U256, Address, FixedBytes, Bytes },
    providers::{Provider, ProviderBuilder, RootProvider},
    rpc::types::{TransactionRequest, BlockNumberOrTag },
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
   sol_types::sol;
};
use log::{debug, info};
use rayon::prelude;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;
use calculation::Calculator;
use estimator::Estimator;
use events::Event;
use market_state::MarketState;
use swap::SwapPath;
use main::AMOUNT;

// top level sercher struct
// contains the calculator and all path information
pub struct Searchoor<T, N, P>
where
    T: Transport + Clone,
    N: Network,
    P: Provider<T, N>,
{
    calculator: Calculator<T, N, P>,
    estimator: Estimator<T, N, P>,
    path_index: HashMap<Address, Vec<usize>>,
    cycles: Vec<SwapPath>,
    min_profit: U256,
}

impl<T, N, P> Searchoor<T, N, P>
where
    T: Transport + Clone,
    N: Network,
    P: Provider<T, N>,
{
    // Construct the searcher with the calculator and all the swap paths
    pub fn new(
        cycles: Vec<SwapPath>,
        market_state: Arc<MarketState<T, N, P>>,
        estimator: Estimator<T, N, P>,
    ) -> Self {
        let calculator = Calculator::new(market_state);

        // make our path mapper for easily getting touched paths
        let mut index: HashMap<Address, Vec<usize>> = HashMap::new();
        for (path_index, path) in cycles.iter().enumerate() {
            for step in &path.steps {
                index.entry(step.pool_address).or_default().push(path_index)
            }
        }

        // calculate the min profit percentage
        let initial_amount = *AMOUNT;
        let flash_loan_fee = (initial_amount * U256::from(9)) / U256::from(10000);
        let repayment_amount = initial_amount + flash_loan_fee;
        let min_profit_percentage = (initial_amount * U256::from(1)) / U256::from(100);
        let min_profit = repayment_amount + min_profit_percentage;

        Self {
            calculator,
            estimator,
            cycles,
            path_index: index,
            min_profit,
        }
    }

    pub fn search_paths(&mut self, paths_tx: Sender<Event>, address_rx: Receiver<Event>) {
        let _sim: bool = std::env::var("SIM").unwrap().parse().unwrap();

        // wait for a new single with the pools that have reserved updated
        while let Ok(Event::PoolsTouched(pools, block_number)) = address_rx.recv() {
            info!("Searching for arbs in block {}...", block_number);
            let res = Instant::now();

            // invalidate all updated pools in the cache
            self.calculator.invalidate_cache(&pools);

            // update all the rates for the pools that were touched
            self.estimator.update_rates(&pools);
            info!("Updated estimations");

            // from the updated pools, get all paths that we want to recheck
            let affected_paths: HashSet<&SwapPath> = pools
                .iter()
                .filter_map(|pool| self.path_index.get(pool))
                .flatten()
                .map(|&index| &self.cycles[index])
                .collect();
            info!("{} touched paths", affected_paths.len());

            // get the output amount and check for profitability
            let profitable_paths: Vec<(SwapPath, U256)> = affected_paths
                .par_iter()
                .filter_map(|path| {
                    // estimate if the path is profitable
                    let output_est = self.estimator.estimate_output_amount(path);
                    if output_est >= self.min_profit && output_est < U256::from(1e18) {
                        Some(((*path).clone(), output_est))
                    } else {
                        None
                    }
                })
                .collect();

            info!("{:?} elapsed estimating paths", res.elapsed());
            info!("{} estimated profitable paths", profitable_paths.len());

            if !profitable_paths.is_empty() {
                // get the best estimated quote and confirm that it is actual in profit
                let best_path = profitable_paths.iter().max_by_key(|(_, amt)| amt).unwrap();
                let calculated_out = self.calculator.calculate_output(&best_path.0);

                if calculated_out >= self.min_profit {
                    info!("Estimated {}. Calculated {}", best_path.1, calculated_out);
                    match paths_tx.send(Event::ArbPath((
                        best_path.0.clone(),
                        calculated_out,
                        block_number,
                    ))) {
                        Ok(_) => debug!("Sent path"),
                        Err(_) => debug!("Failed to send path"),
                    }
                }
            }
        }
    }
}
