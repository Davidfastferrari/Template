use alloy::{
    eips::{BlockId, BlockNumberOrTag},
    primitives::U256,
};
use anyhow::Result;
use ignition::start_workers;
use lazy_static::lazy_static;
use log::{info, LevelFilter};
use std::collections::HashMap;
use std::thread::Builder;
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
pub const AMOUNT_USD: u64 = 100_000; // $100,000

// Example token metadata
lazy_static! {
    pub static ref TOKEN_DECIMALS: HashMap<&'static str, u8> = {
        let mut map = HashMap::new();
        map.insert("USDC", 6);
        map.insert("WETH", 18);
        map.insert("DAI", 18);
        map.insert("USDT", 6);
        map
    };
   pub fn amount_for_token(token_symbol: &str) -> U256 {
    let decimals = TOKEN_DECIMALS.get(token_symbol).copied().unwrap_or(18);
    let multiplier = U256::exp10(decimals as usize); // Safe and correct
    U256::from(AMOUNT_USD) * multiplier
}
pub static ref AMOUNT: U256 = TOKEN_DECIMALS; 
}


#[tokio::main]
async fn main() -> Result<()> {
    // init dots and logger
       dotenv::dotenv().ok();
       env_logger::Builder::new()
       Builder::new()
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
