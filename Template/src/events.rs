use std::collections::HashSet;

use alloy::primitives::{Address, U256};
use alloy::rpc::types::Header;

use crate::gen::FlashQuoter::SwapParams;
use crate::swap::SwapPath;

/// Represents messages passed across the bot's internal event pipeline
#[derive(Debug, Clone)]
pub enum Event {
    /// Arbitrage path found (SwapPath, estimated profit, block number)
    ArbPath((SwapPath, U256, u64)),

    /// A path validated by quoting engine (params, expected output, block number)
    ValidPath((SwapParams, U256, u64)),

    /// Set of pools involved in a previous swap or touched in state update (with block number)
    PoolsTouched(HashSet<Address>, u64),

    /// New block received (raw header)
    NewBlock(Header),
}
