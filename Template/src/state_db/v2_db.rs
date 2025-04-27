use tracing::{info, error, debug, warn};
use alloy::sol;
use serde::{Serialize, Deserialize};
use serde_json::json;
use super::BlockStateDB;
use alloy::network::Network;
use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use alloy::transports::Transport;
use lazy_static::lazy_static;
use log::trace;
use pool_sync::{Pool, PoolInfo};
use revm::DatabaseRef;

use crate::state_db::blockstate_db::{InsertionType, BlockStateDBSlot};

lazy_static! {
    // Uniswap V2 reserves are stored as two packed U112 values
    static ref U112_MASK: U256 = (U256::from(1) << 112) - 1;
}

impl<N, P> BlockStateDB<N, P>
where
    N: Network,
    P: Provider<N>,
{
    /// Inserts UniswapV2-style pool into the simulated state DB
    pub fn insert_v2(&mut self, pool: Pool) {
        trace!("V2 DB: inserting pool {}", pool.address());
        let address = pool.address();
        let token0 = pool.token0_address();
        let token1 = pool.token1_address();

        self.add_pool(pool.clone());

        let v2_info = pool.get_v2().expect("Expected V2 pool");
        let reserve0 = U256::from(v2_info.token0_reserves);
        let reserve1 = U256::from(v2_info.token1_reserves);

        self.insert_reserves(address, reserve0, reserve1);
        self.insert_token0(address, token0);
        self.insert_token1(address, token1);
    }

    /// Reads packed V2-style reserves from storage slot 8
    pub fn get_reserves(&self, pool: &Address) -> (U256, U256) {
        let value = self.storage_ref(*pool, U256::from(8)).unwrap();
        let reserve0 = value & *U112_MASK;
        let reserve1 = (value >> 112) & *U112_MASK;
        (reserve0, reserve1)
    }

    /// Reads token0 from storage slot 6
    pub fn get_token0(&self, pool: Address) -> Address {
        let raw = self.storage_ref(pool, U256::from(6)).unwrap();
        Address::from_word(raw.into())
    }

    /// Reads token1 from storage slot 7
    pub fn get_token1(&self, pool: Address) -> Address {
        let raw = self.storage_ref(pool, U256::from(7)).unwrap();
        Address::from_word(raw.into())
    }

    /// [Future] Add V2 token fetch logic via full ABI if needed
    #[allow(dead_code)]
    pub fn get_tokens(&self, _pool: &Address) -> (Address, Address) {
        todo!("If needed for ABI resolution or extra asserts")
    }

    /// Helper: inserts packed reserve0 + reserve1 into storage slot 8
    fn insert_reserves(&mut self, pool: Address, reserve0: U256, reserve1: U256) {
        let packed = (reserve1 << 112) | reserve0;
        trace!("Inserting reserves: {:?}, {:?}", reserve0, reserve1);
        let slot = BlockStateDBSlot {
            value: packed,
            insertion_type: InsertionType::Custom,
        };
        self.accounts.get_mut(&pool).unwrap().storage.insert(U256::from(8), slot);
    }

    /// Helper: inserts token0 address into slot 6 (right-aligned)
    fn insert_token0(&mut self, pool: Address, token: Address) {
        trace!("Inserting token0: {}", token);
        let slot = BlockStateDBSlot {
            value: U256::from_be_bytes(token_to_storage(token)),
            insertion_type: InsertionType::Custom,
        };
        self.accounts.get_mut(&pool).unwrap().storage.insert(U256::from(6), slot);
    }

    /// Helper: inserts token1 address into slot 7 (right-aligned)
    fn insert_token1(&mut self, pool: Address, token: Address) {
        trace!("Inserting token1: {}", token);
        let slot = BlockStateDBSlot {
            value: U256::from_be_bytes(token_to_storage(token)),
            insertion_type: InsertionType::Custom,
        };
        self.accounts.get_mut(&pool).unwrap().storage.insert(U256::from(7), slot);
    }
}

/// Converts an `Address` into a BE-encoded 32-byte slot (right-aligned)
fn token_to_storage(token: Address) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[12..].copy_from_slice(token.as_bytes());
    bytes
}


#[cfg(test)]
mod test_db_v2 {
    use super::*;
    use alloy::network::Ethereum;
    use alloy::primitives::{address, Address, U256};
    use alloy::providers::{ProviderBuilder, RootProvider};
    use alloy::transports::http::{Client, Http};
    use env_logger::Env;
    use log::LevelFilter;
    use pool_sync::{Pool, UniswapV2Pool};
    use revm::primitives::{Bytecode, AccountInfo, TransactTo};
    use revm::Evm;
    use crate::gen::FlashQuoter;

    fn mock_uni_v2_pool() -> Pool {
        Pool::UniswapV2(UniswapV2Pool {
            address: address!("88A43bbDF9D098eEC7bCEda4e2494615dfD9bB9C"),
            token0: address!("4200000000000000000000000000000000000006"),
            token1: address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
            token0_name: "WETH".to_string(),
            token1_name: "USDC".to_string(),
            token0_decimals: 18,
            token1_decimals: 6,
            token0_reserves: U256::from(1_000_000_000_000_000_000u128),
            token1_reserves: U256::from(1_000_000_000),
            stable: None,
            fee: None,
        })
    }

    fn init_logger() {
        let _ = env_logger::Builder::from_env(Env::default().default_filter_or("trace"))
            .is_test(true)
            .filter_module("v2_db", LevelFilter::Trace)
            .try_init();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_insert_and_read_v2_state() {
        dotenv::dotenv().ok();
        init_logger();

        let url = std::env::var("FULL").unwrap().parse().unwrap();
        let provider = ProviderBuilder::new().on_http(url);
        let mut db = BlockStateDB::new(provider).unwrap();

        let pool = mock_uni_v2_pool();
        let pool_addr = pool.address();
        db.insert_v2(pool);

        let (res0, res1) = db.get_reserves(&pool_addr);
        assert!(res0 > U256::ZERO && res1 > U256::ZERO, "Reserves should be non-zero");

        let token0 = db.get_token0(pool_addr);
        let token1 = db.get_token1(pool_addr);

        assert_eq!(token0, address!("4200000000000000000000000000000000000006"));
        assert_eq!(token1, address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_quote_arbitrage_offchain_evm() {
        dotenv::dotenv().ok();
        init_logger();

        let url = std::env::var("FULL").unwrap().parse().unwrap();
        let provider = ProviderBuilder::new().on_http(url);
        let mut db = BlockStateDB::new(provider.clone()).unwrap();

        let pool = mock_uni_v2_pool();
        let pool_addr = pool.address();
        db.insert_v2(pool);

        let quoter_addr = address!("0000000000000000000000000000000000001000");
        let quoter_code = FlashQuoter::DEPLOYED_BYTECODE.clone();

        let quoter_info = AccountInfo {
            nonce: 0,
            balance: U256::ZERO,
            code_hash: revm::primitives::keccak256(&quoter_code),
            code: Some(Bytecode::new_raw(quoter_code)),
        };
        db.insert_account_info(quoter_addr, quoter_info, InsertionType::Custom);

        let quote_path = FlashQuoter::SwapParams {
            pools: vec![pool_addr],
            poolVersions: vec![0],
            amountIn: U256::from(1_000_000),
        };

        let calldata = FlashQuoter::quoteArbitrageCall { params: quote_path }.abi_encode();

        let mut evm = Evm::builder()
            .with_db(&mut db)
            .modify_tx_env(|tx| {
                tx.caller = address!("0000000000000000000000000000000000000001");
                tx.transact_to = TransactTo::Call(quoter_addr);
                tx.data = calldata.into();
                tx.value = U256::ZERO;
            })
            .build();

        let result = evm.transact().unwrap();
        if let revm::primitives::ExecutionResult::Success { output, .. } = result.result {
            let decoded: Vec<U256> = <Vec<U256>>::abi_decode(output.data(), false).unwrap();
            println!("Flash quote decoded output: {:?}", decoded);
            assert!(!decoded.is_empty(), "Flash quote output should not be empty");
        } else {
            panic!("Flash quote call failed in EVM sim");
        }
    }
}
