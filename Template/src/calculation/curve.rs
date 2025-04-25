use super::Calculator;
use alloy::primitives::{address, Address, U256};
use alloy::sol;
use revm::primitives::{ExecutionResult, TransactTo};
use revm::Evm;
use std::sync::Arc;
use anyhow::{Result, Context};
use log::{info, warn};

sol! {
    #[sol(rpc)]
    contract CurveOut {
        function get_dy(uint256 i, uint256 j, uint256 dx) external view returns (uint256);
    }
}

impl<N, P> Calculator<N, P>
where
    N: alloy::network::Network,
    P: alloy::providers::Provider<N, P>,
{
    /// Simulates Curve's `get_dy` offchain to estimate swap output
    pub fn curve_out(
        &self,
        index_in: U256,
        index_out: U256,
        amount_in: U256,
        pool: Address,
    ) -> U256 {
        let calldata = CurveOut::get_dyCall {
            i: index_in,
            j: index_out,
            dx: amount_in,
        }
        .abi_encode();

        let mut db = self.db.write().expect("Failed to acquire DB write lock");

        let mut evm = Evm::builder()
            .with_db(&mut *db)
            .modify_tx_env(|tx| {
                tx.caller = address!("0000000000000000000000000000000000000001");
                tx.transact_to = TransactTo::Call(pool);
                tx.data = calldata.into();
                tx.value = U256::ZERO;
            })
            .build();

        // 🧠 Snapshot before execution
        let pre_snapshot = db.accounts.clone();

        let tx_result = match evm.transact() {
            Ok(ref_tx) => ref_tx.result,
            Err(err) => {
                warn!("CurveOut simulation failed: {:?}", err);
                return U256::ZERO;
            }
        };

        // 🧪 State delta analysis
        self.analyze_state_changes(&pre_snapshot, &*db, pool);

        match tx_result {
            ExecutionResult::Success { output, .. } => {
                match <U256>::abi_decode(output.data(), false) {
                    Ok(amount_out) => amount_out,
                    Err(e) => {
                        warn!("CurveOut decoding failed: {:?}", e);
                        U256::ZERO
                    }
                }
            }
            ExecutionResult::Revert { output, .. } => {
                warn!("CurveOut reverted: {:?}", output);
                U256::ZERO
            }
            other => {
                warn!("Unexpected CurveOut exec result: {:?}", other);
                U256::ZERO
            }
        }
    }

    /// ✅ Property-based edge test: get_dy == 0 should be handled
    pub fn is_curve_edge_case_zero(
        &self,
        index_in: U256,
        index_out: U256,
        amount_in: U256,
        pool: Address,
    ) -> bool {
        let out = self.curve_out(index_in, index_out, amount_in, pool);
        if out == U256::ZERO {
            info!(
                "⚠️ Detected edge case in Curve pool {:?}: get_dy({}, {}, {}) == 0",
                pool, index_in, index_out, amount_in
            );
            true
        } else {
            false
        }
    }

    /// 🛠 Meta-pool or fee-adjusted Curve call support
    pub fn curve_out_with_fee_adjustment(
        &self,
        index_in: U256,
        index_out: U256,
        amount_in: U256,
        pool: Address,
        fee_basis_points: u64,
    ) -> U256 {
        let fee = (amount_in * U256::from(fee_basis_points)) / U256::from(10_000u64);
        let adjusted_amount = amount_in.saturating_sub(fee);
        self.curve_out(index_in, index_out, adjusted_amount, pool)
    }

    /// 🧪 Helper to analyze EVM state difference after call
    fn analyze_state_changes(
        &self,
        pre: &std::collections::HashMap<Address, crate::state_db::BlockStateDBAccount>,
        post: &crate::state_db::BlockStateDB<T, N, P>,
        pool: Address,
    ) {
        if let Some(post_acc) = post.accounts.get(&pool) {
            if let Some(pre_acc) = pre.get(&pool) {
                for (slot, post_val) in &post_acc.storage {
                    if let Some(pre_val) = pre_acc.storage.get(slot) {
                        if pre_val.value != post_val.value {
                            info!(
                                "🔍 Pool {} - Slot {} changed from {} -> {}",
                                pool, slot, pre_val.value, post_val.value
                            );
                        }
                    } else {
                        info!(
                            "🆕 New slot {} added to pool {}: {}",
                            slot, pool, post_val.value
                        );
                    }
                }
            } else {
                info!("⚠️ New account created for pool {}!", pool);
            }
        }
    }
}
