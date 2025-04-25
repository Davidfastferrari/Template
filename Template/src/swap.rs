use crate::gen::{FlashQuoter, FlashSwap};
use crate::AMOUNT;
use alloy::primitives::Address;
use pool_sync::PoolType;
use serde::{Deserialize, Serialize};
use std::convert::From;
use std::hash::Hash;

#[derive(Serialize, Deserialize, Debug)]
struct Point {
    x: i32,
    y: i32,
}

/// Represents an individual swap step in a multi-hop path.
#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct SwapStep {
    pub pool_address: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub protocol: PoolType,
    pub fee: u32,
}

/// Full swap path that the bot will evaluate and potentially execute.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SwapPath {
    pub steps: Vec<SwapStep>,
    pub hash: u64,
}

/// Converts a [`FlashQuoter::SwapParams`] into a [`FlashSwap::SwapParams`] for execution.
///
/// This conversion is useful after estimating quotes from a flash quoter and preparing a swap call.
impl From<FlashQuoter::SwapParams> for FlashSwap::SwapParams {
    fn from(params: FlashQuoter::SwapParams) -> Self {
        FlashSwap::SwapParams {
            pools: params.pools,
            poolVersions: params.poolVersions,
            amountIn: params.amountIn,
        }
    }
}

/// Converts a [`SwapPath`] into a [`FlashQuoter::SwapParams`] for quote estimation.
///
/// This builds the vector of pool addresses and their corresponding protocol version
/// (encoded as `u8` where V3 = 1 and others = 0).
impl From<SwapPath> for FlashQuoter::SwapParams {
    fn from(path: SwapPath) -> Self {
        let mut pools: Vec<Address> = Vec::with_capacity(path.steps.len());
        let mut protocols: Vec<u8> = Vec::with_capacity(path.steps.len());

        for step in path.steps {
            pools.push(step.pool_address);
            protocols.push(if step.protocol.is_v3() { 1 } else { 0 });
        }

        FlashQuoter::SwapParams {
            pools,
            poolVersions: protocols,
            amountIn: AMOUNT,
        }
    }
}
