use once_cell::sync::Lazy;
use alloy::primitives::U256;

pub static U256_ONE: Lazy<U256> = Lazy::new(|| U256::from(1u64));
