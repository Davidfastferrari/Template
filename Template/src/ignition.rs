use std::{
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc,
    },
    time::Duration,
};

use tokio::sync::{broadcast, mpsc::{Sender, Receiver}};
use alloy::providers::ProviderBuilder;
use log::{info, error};
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
    let (block_sender, _) = broadcast::channel::<Event>(100);
    let (block_tx, mut block_rx): (Sender<Event>, Receiver<Event>) = channel(100);
    let (address_sender, address_receiver): (Sender<Event>, Receiver<Event>) = channel(100);
    let (paths_sender, paths_receiver): (Sender<Event>, Receiver<Event>) = channel(100);
    let (profitable_sender, profitable_receiver): (Sender<Event>, Receiver<Event>) = channel(100);

    // Graceful shutdown channel
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

    // --- Pool Filtering ---
    info!("Pool count before filtering: {}", pools.len());
    let pools = filter_pools(pools, 4000, Chain::Base).await;
    info!("Pool count after filtering: {}", pools.len());

    // --- Block Event Proxy ---
    {
        let mut block_subscriber = block_sender.subscribe();
        let block_tx = block_tx.clone();
        tokio::spawn(async move {
            while let Ok(event) = block_subscriber.recv().await {
                if block_tx.send(event).await.is_err() {
                    break;
                }
            }
        });
    }

    // --- Streamer to push new blocks into broadcast ---
    tokio::spawn(stream_new_blocks(block_sender.clone()));

    // --- Gas Station ---
    let gas_station = Arc::new(GasStation::new());
    {
        let gas_station = Arc::clone(&gas_station);
        let mut block_gas_sub = block_sender.subscribe();
        tokio::spawn(async move {
              gas_station.update_gas(block_gas_sub).await;
        });
    }

    // --- State Catch-up Flag ---
    let caught_up = Arc::new(AtomicBool::new(false));

    // --- Market State ---
    info!("Initializing market state...");
    let http_url = std::env::var("FULL").unwrap().parse().unwrap();
    let provider = ProviderBuilder::new().on_http(http_url);

    let market_state = MarketState::init_state_and_start_stream(
        pools.clone(),
        block_rx,
        address_sender.clone(),
        last_synced_block,
        provider,
        Arc::clone(&caught_up),
    )
    .await
    .expect("Failed to initialize market state");

    info!("Market state initialized!");

    // --- Wait for catch-up ---
    info!("Waiting for block sync before initializing estimator...");
    while !caught_up.load(Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // --- Estimator Init ---
    info!("Calculating initial rates...");
    let mut estimator = Estimator::new(Arc::clone(&market_state));
    estimator.process_pools(pools.clone());
    info!("Initial rates calculated!");

    // --- Arbitrage Cycles ---
    info!("Generating arbitrage cycles...");
    let cycles = ArbGraph::generate_cycles(pools.clone()).await;
    info!("Generated {} arbitrage cycles", cycles.len());

    // --- Simulator ---
    {
        let ms = Arc::clone(&market_state);
        let profitable_sender = profitable_sender.clone();
        tokio::spawn(simulate_paths(profitable_sender, paths_receiver, ms));
    }

    // --- Searcher ---
    {
        let mut searcher = Searchoor::new(cycles, Arc::clone(&market_state), estimator);
        tokio::spawn(async move {
            if let Err(e) = searcher.search_paths(paths_sender, address_receiver).await {
                error!("Searcher failed: {:?}", e);
            }
        });
    }

    // --- Transaction Sender ---
    {
        let mut tx_sender = TransactionSender::new(Arc::clone(&gas_station)).await;
        tokio::spawn(async move {
            tx_sender.send_transactions(profitable_receiver).await;
        });
    }

    // --- Graceful Shutdown Handler ---
    tokio::spawn(async move {
        if let Err(err) = signal::ctrl_c().await {
            error!("Failed to listen for shutdown: {:?}", err);
        }
        info!("🛑 Ctrl-C detected. Shutting down...");
        let _ = shutdown_tx.send(());
    });

    // --- Await Shutdown Signal ---
    let _ = shutdown_rx.recv().await;
    info!("🚪 All workers will now terminate.");
}
