use tracing::{info, error, debug, warn};
use serde::{Serialize, Deserialize};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast::Receiver;
use alloy::primitives::U256;
use alloy::eips::eip1559::{BaseFeeParams, calc_next_block_base_fee};

use crate::events::Event;

/// Handles dynamic gas fee estimation using EIP-1559-style base fees.
pub struct GasStation {
    base_fee: AtomicU64,
}

// Constants for gas price calculation
const DEFAULT_PRIORITY_DIVISOR: u128 = 350_000;
const PROFIT_PERCENTAGE_FOR_GAS: u128 = 2; // Spend up to 50% of profit

impl GasStation {
    /// Create a new gas estimator with initial base_fee set to 0
    pub fn new() -> Self {
        Self {
            base_fee: AtomicU64::new(0),
        }
    }

    /// Compute max fee and priority fee based on profit.
    /// Will spend up to 50% of the profit on gas (split between base + priority).
    pub fn get_gas_fees(&self, profit: U256) -> (u128, u128) {
        let base_fee = self.base_fee.load(Ordering::Relaxed) as u128;

        let max_total_gas_spend = (profit / U256::from(PROFIT_PERCENTAGE_FOR_GAS)).as_u128();
        let priority_fee = max_total_gas_spend / DEFAULT_PRIORITY_DIVISOR;

        (base_fee + priority_fee, priority_fee)
    }

    /// Asynchronously updates the base fee based on new block headers from the event stream.
    pub async fn update_gas(&self, mut block_rx: Receiver<Event>) {
        let base_fee_params = BaseFeeParams::optimism_canyon();

        while let Ok(event) = block_rx.recv().await {
            if let Event::NewBlock(header) = event {
                // Safe unwrap with context in case of None
                let base_fee = header
                    .inner
                    .base_fee_per_gas
                    .expect("Base fee missing in block header");

                let gas_used = header.inner.gas_used;
                let gas_limit = header.inner.gas_limit;

                let next_base_fee = calc_next_block_base_fee(
                    gas_used,
                    gas_limit,
                    base_fee,
                    base_fee_params,
                );

                self.base_fee.store(next_base_fee, Ordering::Relaxed);
                tracing::debug!(
                    target: "gas_station",
                    base_fee = %base_fee,
                    gas_used = %gas_used,
                    gas_limit = %gas_limit,
                    next_base_fee = %next_base_fee,
                    "Updated base fee"
                );
            }
        }
    }
}
