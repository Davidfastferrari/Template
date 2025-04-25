use std::{
    collections::HashMap,
    sync::RwLock,
    time::Duration,
};

use alloy::primitives::U256;
use anyhow::Result;
use ignition::start_workers;
use lazy_static::lazy_static;
use log::{info, LevelFilter};
use once_cell::sync::Lazy;
use pool_sync::{Chain, PoolSync, PoolType};

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

/// $100,000 in USD base units
pub const AMOUNT_USD: u64 = 100_000;

/// Global U256 value representing the input amount in base units (used across modules)
pub static AMOUNT: Lazy<RwLock<U256>> = Lazy::new(|| RwLock::new(U256::ZERO));

/// Token decimals map to convert $100k into base units
pub static TOKEN_DECIMALS: Lazy<HashMap<&'static str, u8>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert("USDC", 6);
    map.insert("USDT", 6);
    map.insert("WETH", 18);
    map.insert("DAI", 18);
    map
});

/// Converts a token symbol to an on-chain value in base units
pub fn amount_for_token(token_symbol: &str) -> U256 {
    let decimals = TOKEN_DECIMALS.get(token_symbol).copied().unwrap_or(18);
    let multiplier = U256::exp10(decimals as usize);
    U256::from(AMOUNT_USD) * multiplier
}

/// Updates the global `AMOUNT` based on the token
pub fn update_amount(token_symbol: &str) {
    let calculated = amount_for_token(token_symbol);
    let mut amount = AMOUNT.write().unwrap();
    *amount = calculated;
}

/// Entry point: starts the workers and main loop
#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables and logger
    dotenv::dotenv().ok();
    env_logger::Builder::new()
        .filter_module("BaseBuster", LevelFilter::Info)
        .init();

    info!("Loading and syncing pools...");

    // Initialize pool sync across all supported AMM protocols
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
            PoolType::AlienBaseV3,
            PoolType::BaseswapV2,
            PoolType::BaseswapV3,
            PoolType::MaverickV1,
            PoolType::MaverickV2,
        ])
        .chain(Chain::Base)
        .rate_limit(1000)
        .build()?;

    let (pools, last_synced_block) = pool_sync.sync_pools().await?;

    // Start async workers
    start_workers(pools, last_synced_block).await;

    // Loop to keep main thread alive if workers are spawned independently
    loop {
        tokio::time::sleep(Duration::from_secs(1000)).await;
    }

    // This is never reached unless loop is broken
    #[allow(unreachable_code)]
    Ok(())
}
