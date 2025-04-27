use once_cell::sync::Lazy;
use std::sync::RwLock;
use alloy::primitives::U256;

/// Global amount used across modules
pub static AMOUNT: Lazy<RwLock<U256>> = Lazy::new(|| RwLock::new(U256::from(1_000_000_000_000_000_000u128)));

pub static U256_ONE: Lazy<U256> = Lazy::new(|| U256::from(1u64));
