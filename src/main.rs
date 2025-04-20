use alloy::{eips::{BlockId, BlockNumberOrTag}, primitives::U256, providers::Provider, rpc::types::{BlockTransactions, BlockTransactionsKind}};
use anyhow::Result;
use ignition::start_workers;
use lazy_static::lazy_static;
use log::{info, LevelFilter};
use pool_sync::{PoolSync, PoolType, Chain, PoolInfo};

mod bytecode;
mod cache;
mod calculation;
mod estimator;
mod events;
mod filter;
mod gas_station;
mod gen;
mod graph;
mod ignition;
mod market_state;
mod quoter;
mod searcher;
mod simulator;
mod state_db;
mod stream;
mod swap;
mod tests;
mod tracing;
mod tx_sender;
mod history_db;

// initial amount we are trying to arb over
lazy_static! {
    pub static ref AMOUNT: U256 = U256::from(1e15); 
}

#[tokio::main]
async fn main() -> Result<()> {
    // init dots and logger
    dotenv::dotenv().ok();
    env_logger::Builder::new()
        .filter_module("BaseBuster", LevelFilter::Info)
        .init();

    // Load in all the pools
    info!("Loading and syncing pools...");
    let pool_sync = PoolSync::builder()
        .add_pools(&[
            PoolType::UniswapV2,
            PoolType::PancakeSwapV2,
            PoolType::SushiSwapV2,
            PoolType::UniswapV3,
            PoolType::SushiSwapV3,
            PoolType::BaseSwapV2,
            PoolType::BaseSwapV3,
            PoolType::Aerodrome,
            PoolType::Slipstream,
            PoolType::AlienBaseV2,
            PoolType::AlienBaseV3
        ])
        .chain(Chain::Base)
        .rate_limit(1000)
        .build()?;
    let (pools, last_synced_block) = pool_sync.sync_pools().await?;

    start_workers(pools, last_synced_block).await;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1000)).await;
    }
    Ok(())
}
