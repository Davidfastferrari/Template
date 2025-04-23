use alloy::{
    primitives::{U256, Address},
    rpc::types::Header,
};
use std::collections::HashSet;
use swap::SwapPath;
use gen1::FlashQuoter::SwapParams;

#[derive(Debug, Clone)]
pub enum Event {
    ArbPath((SwapPath, U256, u64)),
    ValidPath((SwapParams, U256, u64)),
    PoolsTouched(HashSet<Address>, u64),
    NewBlock(Header),
}
