use tokio::sync::mpsc::{Sender, Receiver};
use tracing::{info, debug, warn};
use serde::{Serialize, Deserialize};
use serde_json::json;
use uniswap_v3_math::{tick_math, swap_math, tick_bitmap};
use alloy::sol;
use alloy_sol_types::SolCall;
use alloy::network::Ethereum;
use alloy::providers::RootProvider;
use alloy_transport_http::{Http, Client as AlloyClient};
use alloy::primitives::U256;

use std::collections::HashSet;
use std::sync::Arc;
use std::str::FromStr;

use crate::gen::{FlashQuoter, FlashSwap};
use crate::events::Event;
use crate::market_state::MarketState;
use crate::simulator::Quoter;
use crate::calculator::Calculator;
use crate::constants::AMOUNT;


/// Simulates arbitrage paths passed from the searcher and sends viable ones to the tx sender.
pub async fn simulate_paths(
    tx_sender: Sender<Event>,
    mut arb_receiver: Receiver<Event>,
    market_state: Arc<MarketState<HttpClient, Ethereum>>,
) {
    let sim: bool = match std::env::var("SIM") {
        Ok(v) => v.parse().unwrap_or(false),
        Err(_) => false,
    };

    let mut blacklisted_paths: HashSet<u64> = HashSet::new();

    wwhile let Some(Event::ArbPath((arb_path, expected_out, block_number))) = arb_receiver.recv().await {
        if blacklisted_paths.contains(&arb_path.hash) {
            continue;
        }

        info!("Received path for simulation...");

        let mut converted_path: FlashQuoter::SwapParams = arb_path.clone().into();

        // Quote path
        match Quoter::quote_path(converted_path.clone(), market_state.clone()) {
            Ok(quote) => {
                let last_quote = match quote.last() {
                    Some(q) => *q,
                    None => {
                        warn!("Quote was empty for path {:?}", arb_path.hash);
                        blacklisted_paths.insert(arb_path.hash);
                        continue;
                    }
                };

                if sim {
                    // Just simulating, compare result
                    if last_quote == expected_out {
                        info!(
                            "✅ Sim match! Expected: {}, Quoted: {}, Hash: {}",
                            expected_out, last_quote, arb_path.hash
                        );
                    } else {
                        info!(
                            "❌ Sim mismatch. Expected: {}, Got: {}, Hash: {}",
                            expected_out, last_quote, arb_path.hash
                        );
                        let calculator = Calculator::new(market_state.clone());
                        calculator.debug_calculation(&arb_path);
                    }
                } else {
                    // Only continue if expected value is big enough
                    if last_quote < U256::from_str("1000000000000000000").unwrap() {
                        continue;
                    }

                    info!(
                        "✅ Sim successful. Output: {}, Block: {}",
                        expected_out, block_number
                    );

                    let optimized = Quoter::optimize_input(
                        converted_path.clone(),
                        last_quote,
                        market_state.clone(),
                    );
                    info!("💰 Optimized input: {}, output: {}", optimized.0, optimized.1);

                    let profit = expected_out.saturating_sub(*AMOUNT.read().unwrap()); // ✅ Prevent underflow
                    converted_path.amountIn = optimized.0;

                   match tx_sender.send(Event::ValidPath((converted_path, profit, block_number))).await {
                      Ok(_) => debug!("✔️ Sent to tx sender"),
                      Err(e) => warn!("⚠️ Failed to send to tx sender: {:?}", e),
                   }
                }
            }
            Err(err) => {
                warn!("Simulation error on hash {}: {:?}", arb_path.hash, err);
                blacklisted_paths.insert(arb_path.hash);
            }
        }
    }
}
