use super::Calculator;
use alloy::primitives::{address, Address, U256};
use alloy::sol;
use alloy::sol_types::{SolCall, SolValue};
use revm::primitives::{ExecutionResult, TransactTo};
use revm::Evm;
use log::{warn, debug, info};

sol! {
    #[sol(rpc)]
    contract MaverickOut {
        function calculateSwap(
            address pool,
            uint128 amount,
            bool tokenAIn,
            bool exactOutput,
            int32 tickLimit
        ) external view returns (uint256 amountIn, uint256 amountOut, uint256 gasEstimate);
    }
}

impl<T, N, P> Calculator<T, N, P>
where
    T: alloy::transports::Transport + Clone,
    N: alloy::network::Network,
    P: alloy::providers::Provider<T, N>,
{
    /// 🧠 Simulate Maverick V2 swap and return output amount
    pub fn maverick_v2_out(
        &self,
        amount_in: U256,
        pool: Address,
        zero_for_one: bool,
        tick_limit: i32,
    ) -> U256 {
        let (_, out, _) = self._simulate_maverick(amount_in, pool, zero_for_one, tick_limit);
        out
    }

    /// 🔁 Run multiple pool paths and score their output
    pub fn path_scoremap(
        &self,
        pools: &[Address],
        amount_in: U256,
        zero_for_one: bool,
    ) -> Vec<(Address, U256)> {
        pools
            .iter()
            .map(|&pool| {
                let tick_limit = self.optimize_tick_limit(pool, amount_in, zero_for_one);
                let (_, out, _) = self._simulate_maverick(amount_in, pool, zero_for_one, tick_limit);
                (pool, out)
            })
            .collect()
    }

    /// 🧠 Dynamically optimize tickLimit for max output
    pub fn optimize_tick_limit(
        &self,
        pool: Address,
        amount_in: U256,
        zero_for_one: bool,
    ) -> i32 {
        let mut best_tick = 0;
        let mut best_output = U256::ZERO;

        for tick in [-10000, -5000, 0, 5000, 10000] {
            let (_, output, _) = self._simulate_maverick(amount_in, pool, zero_for_one, tick);
            if output > best_output {
                best_output = output;
                best_tick = tick;
            }
        }

        info!("Optimized tickLimit for {} → {}", pool, best_tick);
        best_tick
    }

    /// 🧪 Snapshot EVM state before and after simulation
    pub fn state_diff_inspect(
        &self,
        amount_in: U256,
        pool: Address,
        zero_for_one: bool,
        tick_limit: i32,
    ) -> (Vec<u8>, Vec<u8>) {
        let calldata = self.build_meta_swap_calldata(amount_in, pool, zero_for_one, tick_limit);

        let mut db = self.db.write().expect("lock DB");
        let mut evm = Evm::builder()
            .with_db(&mut *db)
            .modify_tx_env(|tx| {
                tx.caller = address!("0000000000000000000000000000000000000001");
                tx.transact_to = TransactTo::Call(pool);
                tx.data = calldata.clone().into();
                tx.value = U256::ZERO;
            })
            .build();

        let state_before = format!("{:?}", evm.db().accounts()).into_bytes();

        let _ = evm.transact(); // discard output

        let state_after = format!("{:?}", evm.db().accounts()).into_bytes();
        (state_before, state_after)
    }

    /// 🧬 Gas heatmap for input sweep
    pub fn gas_estimate_heatmap(
        &self,
        pool: Address,
        token_a_in: bool,
        tick_limit: i32,
        min: U256,
        max: U256,
        steps: usize,
    ) -> Vec<(U256, U256)> {
        let mut results = Vec::with_capacity(steps);
        let step_size = (max - min) / U256::from(steps as u64);

        for i in 0..steps {
            let input = min + step_size * U256::from(i as u64);
            let (_, _, gas_est) = self._simulate_maverick(input, pool, token_a_in, tick_limit);
            results.push((input, gas_est));
        }

        results
    }

    /// ⛽ Meta-TX builder
    pub fn build_meta_swap_calldata(
        &self,
        amount: U256,
        pool: Address,
        token_a_in: bool,
        tick_limit: i32,
    ) -> Vec<u8> {
        MaverickOut::calculateSwapCall {
            pool,
            amount: amount.try_into().expect("u128 overflow"),
            tokenAIn: token_a_in,
            exactOutput: false,
            tickLimit: tick_limit,
        }
        .abi_encode()
    }

    /// Internal helper for all swap simulation logic
    fn _simulate_maverick(
        &self,
        amount_in: U256,
        pool: Address,
        token_a_in: bool,
        tick_limit: i32,
    ) -> (U256, U256, U256) {
        let calldata = self.build_meta_swap_calldata(amount_in, pool, token_a_in, tick_limit);

        let mut db = self.db.write().expect("lock DB");
        let mut evm = Evm::builder()
            .with_db(&mut *db)
            .modify_tx_env(|tx| {
                tx.caller = address!("0000000000000000000000000000000000000001");
                tx.transact_to = TransactTo::Call(pool);
                tx.data = calldata.into();
                tx.value = U256::ZERO;
            })
            .build();

        match evm.transact() {
            Ok(ref_tx) => match ref_tx.result {
                ExecutionResult::Success { output, .. } => {
                    match <(U256, U256, U256)>::abi_decode(output.data(), false) {
                        Ok((amt_in, amt_out, gas_est)) => {
                            debug!("✅ Maverick Sim: out={} gas={}", amt_out, gas_est);
                            (amt_in, amt_out, gas_est)
                        }
                        Err(e) => {
                            warn!("⚠️ Decode error: {:?}", e);
                            (U256::ZERO, U256::ZERO, U256::ZERO)
                        }
                    }
                }
                ExecutionResult::Revert { output, .. } => {
                    warn!("⚠️ Reverted: {:?}", output);
                    (U256::ZERO, U256::ZERO, U256::ZERO)
                }
                other => {
                    warn!("⚠️ Unknown result: {:?}", other);
                    (U256::ZERO, U256::ZERO, U256::ZERO)
                }
            },
            Err(e) => {
                warn!("❌ EVM error: {:?}", e);
                (U256::ZERO, U256::ZERO, U256::ZERO)
            }
        }
    }
}
