use alloy::{
    eips::{ BlockId, Encodable2718 },
    consensus::Transaction,
    network::{ TransactionBuilder, EthereumWallet, Ethereum, Network },
    primitives::{hex, address, U256, Address, FixedBytes, Bytes},
    providers::{Provider, ProviderBuilder, RootProvider},
    rpc::types::{ TransactionRequest, BlockNumberOrTag },
    rpc::types::{
        trace::geth::{ GethDebugTracingCallOptions, Bundle, StateContext, TransactionRequest, GethTrace, GethDebugTracerType, GethDebugBuiltInTracerType, PreStateConfig, GethDebugTracingOptions, GethDefaultTracingOptions, PreStateFrame, AccountState }
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
use anyhow::{anyhow, Result};
use revm::primitives::{ExecutionResult, TransactTo};
use revm::Evm;
use std::sync::Arc;
use gen1::FlashQuoter;
use market_state::MarketState;
use main::AMOUNT;

// Quoter. This is used to get a simulation quote before sending off a transaction.
// This will confirm that our offchain calculations are reasonable and make sure we can swap the tokens
pub struct Quoter;
impl Quoter {
    // get a quote for the path
    pub fn quote_path(
        quote_params: FlashQuoter::SwapParams,
        market_state: Arc<MarketState<Http<Client>, Ethereum, RootProvider<Http<Client>>>>,
    ) -> Result<Vec<U256>> {
        let mut guard = market_state.db.write().unwrap();
        // need to pass this as mut somehow
        let mut evm = Evm::builder().with_db(&mut *guard).build();
        evm.tx_mut().caller = address!("d8da6bf26964af9d7eed9e03e53415d37aa96045");
        evm.tx_mut().transact_to =
            TransactTo::Call(address!("0000000000000000000000000000000000001000"));
        // get read access to the db
        // setup the calldata
        
        let quote_calldata = FlashQuoter::quoteArbitrageCall {
            params: quote_params,
        }
        .abi_encode();
        evm.tx_mut().data = quote_calldata.into();

        // transact
        let ref_tx = evm.transact().unwrap();
        let result = ref_tx.result;

        match result {
            ExecutionResult::Success { output: value, .. } => {
                if let Ok(amount) = Vec::<U256>::abi_decode(value.data(), false) {
                    Ok(amount)
                } else {
                    Err(anyhow!("Failed to decode"))
                }
            }
            ExecutionResult::Revert { output, .. } => Err(anyhow!("Simulation reverted {output}")),
            _ => Err(anyhow!("Failed to simulate")),
        }
    }

    /// Optimizes the input amount using binary search to find the maximum profitable input
    /// Returns the optimal input amount and its corresponding output amounts
    pub fn optimize_input(
        quote_path: FlashQuoter::SwapParams,
        initial_out: U256,
        market_state: Arc<MarketState<Http<Client>, Ethereum, RootProvider<Http<Client>>>>,
    ) -> (U256, U256) {
        let mut quote_path = quote_path.clone();
        let mut curr_input = *AMOUNT;
        let mut best_input = *AMOUNT;
        let mut best_output = initial_out;

        for _ in 0..50 {
            curr_input = curr_input + U256::from(2e14);
            quote_path.amountIn = curr_input;

            match Self::quote_path(quote_path.clone(), market_state.clone()) {
                Ok(amounts) => {
                    let output = *amounts.last().unwrap();
                    if output > curr_input && output > best_output {
                        best_output = output;
                        best_input = curr_input;
                    } else {
                        break;
                    }
                } 
                Err(_) => break
            }
        }
        (best_input, best_output)
    }
}
