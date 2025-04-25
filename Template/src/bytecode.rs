use alloy::primitives::B256;
use lazy_static::lazy_static;
use revm::primitives::{Bytes, Bytecode};
use std::str::FromStr;

/// 🛠 Bytecode and code hash constants for Uniswap V2 pool contracts.
/// These are used for code comparison and verification at runtime.

lazy_static! {
    /// Raw bytecode for UniswapV2-style contracts, used for hash validation and simulation.
    /// ⚠️ NOTE: Replace the hex string with actual deployed bytecode.
    pub static ref UNISWAP_V2_BYTECODE: Bytecode = {
        // ⚠️ TODO: Replace this placeholder with actual bytecode
        let bytecode_hex = "6060604052341561000f57600080fd5b...";
        let raw_bytes = Bytes::from_str(bytecode_hex).expect("Invalid hex string for Uniswap V2 bytecode");
        Bytecode::new_raw(raw_bytes)
    };

    /// Hash of the bytecode, used for contract validation.
    pub static ref UNISWAP_V2_CODE_HASH: B256 = UNISWAP_V2_BYTECODE.hash_slow();
}
