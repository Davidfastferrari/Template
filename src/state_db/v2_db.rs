use tracing::{info, debug, warn};
use alloy::sol;
use alloy::sol_types::SolCall;
use serde::{Serialize, Deserialize};
use serde_json::json;
use alloy::network::Network;
use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use alloy::transports::Transport;
use log::trace;
use pool_sync::{Pool, PoolInfo};
use revm::DatabaseRef;

use super::BlockStateDB;

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
