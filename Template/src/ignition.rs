use std::{
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc,
    },
    time::Duration,
};
use tokio::sync::mpsc::{self, Sender, Receiver};
use tokio::sync::broadcast;
use alloy::providers::ProviderBuilder;
use log::info;
use pool_sync::{Chain, Pool};

use crate::{
    events::Event,
    estimator::Estimator,
    filter::filter_pools,
    gas_station::GasStation,
    graph::ArbGraph,
    market_state::MarketState,
    searcher::Searchoor,
    simulator::simulate_paths,
    stream::stream_new_blocks,
    tx_sender::TransactionSender,
};

/// Bootstraps the entire system: syncing, simulation, and arbitrage search
pub async fn start_workers(pools: Vec<Pool>, last_synced_block: u64) {
    // --- Channel Setup ---
    let (block_sender, block_receiver) = broadcast::channel::<Event>(100);
    let (address_sender, address_receiver): (Sender<Event>, Receiver<Event>) = mpsc::channel(100);
    let (paths_sender, paths_receiver): (Sender<Event>, Receiver<Event>) = mpsc::channel(100);
    let (profitable_sender, profitable_receiver): (Sender<Event>, Receiver<Event>) = mpsc::channel(100);

    // --- Pool Filtering ---
    info!("Pool count before filtering: {}", pools.len());
    let pools = filter_pools(pools, 4000, Chain::Base).await;
    info!("Pool count after filtering: {}", pools.len());

    // --- Block Streamer ---
    tokio::spawn(stream_new_blocks(block_sender));

    // --- Gas Station ---
    let gas_station = Arc::new(GasStation::new());
    {
        let gas_station = Arc::clone(&gas_station);
        let block_rx = block_receiver.resubscribe();
        tokio::spawn(async move {
            gas_station.update_gas(block_rx).await;
        });
    }

    // --- State Catch-up Flag ---
    let caught_up = Arc::new(AtomicBool::new(false));

    // --- Market State Initialization ---
    info!("Initializing market state...");
    let http_url = std::env::var("FULL").unwrap().parse().unwrap();
    let provider = ProviderBuilder::new().on_http(http_url);
    let market_state = MarketState::init_state_and_start_stream(
         pools.clone(),
         block_receiver,
         address_sender.clone(), // ✅ tokio::sync::mpsc::Sender<Event>
         last_synced_block,
         provider,
         Arc::clone(&caught_up),
      )
     .await
     .expect("Failed to initialize market state");
      info!("Market state initialized!");

    // --- Estimator Initialization ---
    info!("Waiting for block sync before initializing estimator...");
    while !caught_up.load(Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    info!("Calculating initial rates...");
    let mut estimator = Estimator::new(Arc::clone(&market_state));
    estimator.process_pools(pools.clone());
    info!("Initial rates calculated!");

    // --- Arbitrage Graph + Cycles ---
    info!("Generating arbitrage cycles...");
    let cycles = ArbGraph::generate_cycles(pools.clone()).await;
    info!("Generated {} arbitrage cycles", cycles.len());

    // --- Simulator ---
    info!("Starting the simulator...");
    tokio::spawn(simulate_paths(
        profitable_sender,
        paths_receiver,
        Arc::clone(&market_state),
    ));

    // --- Arbitrage Searcher ---
    info!("Starting arbitrage searcher...");
    let mut searcher = Searchoor::new(cycles, Arc::clone(&market_state), estimator);
    tokio::spawn(async move {
        if let Err(e) = searcher.search_paths(paths_sender, address_receiver).await {
            log::error!("Searcher failed: {:?}", e);
        }
    });

    // --- Transaction Sender ---
    info!("Starting transaction sender...");
    let mut tx_sender = TransactionSender::new(Arc::clone(&gas_station)).await;
    tokio::spawn(async move {
        tx_sender.send_transactions(profitable_receiver).await;
    });
}
