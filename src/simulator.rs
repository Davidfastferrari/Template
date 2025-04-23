use alloy::{
    eips::{ BlockId, Encodable2718 },
    consensus::Transaction,
    network::{ TransactionBuilder, EthereumWallet, Ethereum, Network },
    primitives::{ hex, address, U256, Address, FixedBytes, Bytes },
    providers::{ Provider, ProviderBuilder, RootProvider },
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
use log::{debug, info, warn};
use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use calculation::Calculator;
use events::Event;
use gen1::FlashQuoter;
use market_state::MarketState;
use quoter::Quoter;
use main::AMOUNT;

// recieve a stream of potential arbitrage paths from the searcher and
// simulate them against the contract to determine if they are actually viable
pub async fn simulate_paths(
    tx_sender: Sender<Event>,
    arb_receiver: Receiver<Event>,
    market_state: Arc<MarketState<Http<Client>, Ethereum, RootProvider<Http<Client>>>>,
) {
    // if this is just a sim run or not
    let sim: bool = std::env::var("SIM").unwrap().parse().unwrap();

    // blacklisted paths, some error in swapping that wasnt caught during filter
    let mut blacklisted_paths: HashSet<u64> = HashSet::new();

    // recieve new paths from the searcher
    while let Ok(Event::ArbPath((arb_path, expected_out, block_number))) = arb_receiver.recv() {
        // convert from searcher format into quoter format
        let mut converted_path: FlashQuoter::SwapParams = arb_path.clone().into();
        println!("{:?}", converted_path);

        // get the quote for the path and handle it appropriately
        // if we have not blacklisted the path
        if !blacklisted_paths.contains(&arb_path.hash) {
            info!("Simulating a new path...");
            // get an initial quote to see if we can swap
            // get read access to the db so we can quote the path
            match Quoter::quote_path(converted_path.clone(), market_state.clone()) {
                Ok(quote) => {
                    // if we are just simulated, compare to the expected amount
                    if sim {
                        if *(quote.last().unwrap()) == expected_out {
                            info!(
                                "Success.. Calculated {expected_out}, Quoted: {}, Path Hash {}",
                                quote.last().unwrap(),
                                arb_path.hash
                            );
                        } else {
                            // get a full debug quote path
                            let calculator = Calculator::new(market_state.clone());
                            calculator.debug_calculation(&arb_path);
                        }
                    } else {
                        if *quote.last().unwrap() > U256::from(1e18) {
                            continue;
                        };

                        info!(
                            "Sim successful... Estimated output: {}, Block {}",
                            expected_out, block_number
                        );



                        // now optimize the input
                        let optimized_amounts = Quoter::optimize_input(converted_path.clone(), *quote.last().unwrap(), market_state.clone());
                        info!("Optimized input: {}. Optimized output: {}", optimized_amounts.0, optimized_amounts.1);
                        let profit = expected_out - *AMOUNT;
                        converted_path.amountIn = optimized_amounts.0;

                        match tx_sender.send(Event::ValidPath((converted_path, profit, block_number))) {
                            Ok(_) => debug!("Simulator sent path to Tx Sender"),
                            Err(_) => warn!("Simulator: failed to send path to tx sender"),
                        }
                    }
                }
                Err(quote_err) => {
                    info!(
                        "Failed to simulate quote {}, {:#?} ",
                        quote_err, arb_path.hash
                    );
                    blacklisted_paths.insert(arb_path.hash);
                }
            }
        }
    }
}
