use alloy::primitives::Address;
use alloy::primitives::U256;
use alloy::rpc::types::Header;
use std::collections::HashSet;

use crate::swap::SwapPath;
use crate::gen::FlashQuoter::SwapParams;

#[derive(Debug, Clone)]
pub enum Event {
    ArbPath((SwapPath, U256, u64)),
    ValidPath((SwapParams, U256, u64)),
    PoolsTouched(HashSet<Address>, u64),
    NewBlock(Header),
}
