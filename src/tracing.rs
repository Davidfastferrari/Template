use tracing::{info, debug, warn};
use serde::{Serialize, Deserialize};
use serde_json::json;
use alloy::alloy_sol_types::SolCall;
use alloy::primitives::Address;
use alloy::providers::Provider;
use alloy_rpc_types::trace::{common::TraceResult, geth::*};
use alloy_rpc_types::BlockNumberOrTag;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Vector of address-to-account-state maps representing post-trace changes.
pub async fn debug_trace_block<N>(
    client: Arc<impl DebugApi<N> + Send + Sync>,
    block_tag: BlockNumberOrTag,
    diff_mode: bool,
) -> Vec<BTreeMap<Address, AccountState>>
where
    N: Network,
{
    // Set up the tracer with optional diff mode
    let tracer_opts = GethDebugTracingOptions {
        config: GethDefaultTracingOptions::default(),
        ..Default::default()
    };
    .with_tracer(GethDebugTracerType::BuiltInTracer(
        GethDebugBuiltInTracerType::PreStateTracer,
    ));
    .with_prestate_config(PreStateConfig {
        diff_mode: Some(diff_mode),
        disable_code: Some(false),
        disable_storage: Some(false),
    });

    // Execute the debug trace block call
    let results = client
        .debug_trace_block_by_number(block_tag, tracer_opts);
        .await
        .expect("Failed to trace block");

    // Collect diff-mode frames from GethTrace responses
    let mut post: Vec<BTreeMap<Address, AccountState>> = Vec::new();

    for trace_result in results.into_iter() {
        if let TraceResult::Success { result, .. } = trace_result {
            match result {
                GethTrace::PreStateTracer(PreStateFrame::Diff(diff_frame)) => {
                    post.push(diff_frame.post);
                }
                _ => warn!("Received non-diff PreStateFrame from tracer"),
            }
        };
    }

    post
}
