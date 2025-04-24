use alloy::{
    alloy-consensus ::Transaction,
    alloy-network::{ TransactionBuilder, Network },
    primitives::{ Address, address, U256 },
    alloy-provider::{ Provider, ProviderBuilder },
    alloy_rpc_types_trace::{
           pre_state::AccountState,
           geth::{ PreStateConfig, GethTrace, GethDebugTracerType, GethDebugBuiltInTracerType, GethDebugTracingOptions, GethDefaultTracingOptions },
           common::TraceResult,
           BlockNumberOrTag,
           TransactionRequest,
    },
   alloy-transport-http::{
        reqwest::{
            header::{ HeaderMap, HeaderValue, AUTHORIZATION },
            Client,
        },
        Http,
      Transport
    },
};
use log::warn;
use std::collections::BTreeMap;
use std::sync::Arc;

// Trace the block to get all addresses with storage changes
pub async fn debug_trace_block<T: Transport + Clone, N: Network, P: Provider<T, N>>(
    client: Arc<P>,
    block_tag: BlockNumberOrTag,
    diff_mode: bool,
) -> Vec<BTreeMap<Address, AccountState>> {
    let tracer_opts = GethDebugTracingOptions {
        config: GethDefaultTracingOptions::default(),
        ..GethDebugTracingOptions::default()
    }
   .with_tracer(GethDebugTracerType::BuiltInTracer(
    GethDebugBuiltInTracerType::PreStateTracer,
  ))
    .with_prestate_config(PreStateConfig {
        diff_mode: Some(diff_mode),
        disable_code: Some(false),
        disable_storage: Some(false),
    });
    let results = client
        .debug_trace_block_by_number(block_tag, tracer_opts)
        .await
        .unwrap();

    let mut post: Vec<BTreeMap<Address, AccountState>> = Vec::new();

    for trace_result in results.into_iter() {
        if let TraceResult::Success { result, .. } = trace_result {
            match result {
                GethTrace::PreStateTracer(PreStateFrame::Diff(diff_frame)) => {
                    post.push(diff_frame.post)
                }
                _ => warn!("Invalid trace"),
            }
        }
    }
    post
}
