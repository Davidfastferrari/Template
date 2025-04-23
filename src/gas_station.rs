use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use tokio::sync::broadcast::Receiver;
use alloy::{
    eips::{ BlockId, Encodable2718, calc_next_block_base_fee, eip1559::{BaseFeeParams} },
    consensus::Transaction,
    network::{ TransactionBuilder, EthereumWallet, Ethereum, Network },
    primitives::{ hex, address, U256, U160, Address, FixedBytes, Bytes },
    providers::{ Provider, ProviderBuilder, RootProvider },
    rpc::types::{ TransactionRequest, BlockNumberOrTag },
    rpc::types::{
        trace::geth::{ GethDebugTracingCallOptions, Bundle, StateContext, TransactionRequest, GethTrace, GethDebugTracerType, GethDebugBuiltInTracerType, PreStateConfig, GethDebugTracingOptions, GethDefaultTracingOptions, PreStateFrame, AccountState },
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
    sol,
    sol_types::{SolCall, SolValue, SolType},
};
use events::Event;

// Handles all gas state and calculations
pub struct GasStation {
    base_fee: AtomicU64,
}

impl GasStation {
    pub fn new() -> Self {
        Self {
            base_fee: AtomicU64::new(0),
        }
    }

    // Get gas fees based off percentage of total profit
    pub fn get_gas_fees(&self, profit: U256) -> (u128, u128) {
        let base_fee = self.base_fee.load(Ordering::Relaxed) as u128;
        let max_total_gas_spend: u128 = (profit / U256::from(2)).try_into().unwrap();
        let priority_fee = max_total_gas_spend / 350_000;
        
        (base_fee + priority_fee, priority_fee)
    }

    // Continuously update the gas fees
    pub async fn update_gas(&self, mut block_rx: Receiver<Event>) {
        let base_fee_params = BaseFeeParams::optimism_canyon();

        while let Ok(Event::NewBlock(header)) = block_rx.recv().await {
            let base_fee = header.inner.base_fee_per_gas.unwrap();
            let gas_used = header.inner.gas_used;
            let gas_limit = header.inner.gas_limit;

            let next_base_fee = calc_next_block_base_fee(gas_used, gas_limit, base_fee, base_fee_params);

            self.base_fee.store(next_base_fee, Ordering::Relaxed);
        }
    }
}
