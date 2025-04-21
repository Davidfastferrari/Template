use crate::gen::FlashQuoter;
use crate::gen::FlashSwap;
use crate::AMOUNT;
use alloy::primitives::Address;
use pool_sync::PoolType;
use serde::{Deserialize, Serialize};
use std::convert::From;
use std::hash::Hash;

// A full representation of a path that we can swap along with its hash
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SwapPath {
    pub steps: Vec<SwapStep>,
    pub hash: u64,
}

// A step representing an individual swap
#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct SwapStep {
    pub pool_address: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub protocol: PoolType,
    pub fee: u32,
}

// Convert from Quoter format into SwapFormat. The same thing
impl From<FlashQuoter::SwapParams> for FlashSwap::SwapParams {
    fn from(params: FlashQuoter::SwapParams) -> Self {
        FlashSwap::SwapParams {
            pools: params.pools,
            poolVersions: params.poolVersions,
            amountIn: params.amountIn
        }
    }
}

// Convert from arb SwapPath into Quoter format
impl From<SwapPath> for FlashQuoter::SwapParams {
    fn from(path: SwapPath) -> Self {
        let mut pools: Vec<Address> = Vec::new();
        let mut protocol: Vec<u8> = Vec::new();
        for step in path.steps {
            pools.push(step.pool_address);
            if step.protocol.is_v3() {
                protocol.push(1);
            } else {
                protocol.push(0);
            }
        }
        FlashQuoter::SwapParams {
            pools,
            poolVersions: protocol,
            amountIn: AMOUNT
        }
    }
}


