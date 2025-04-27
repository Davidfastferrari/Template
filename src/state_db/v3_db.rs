use tracing::{info, debug, warn};
use alloy::sol;
use alloy::sol_types::SolCall;
use serde::{Serialize, Deserialize};
use serde_json::json;
use alloy::network::Network;
use alloy::primitives::{keccak256, Address, Signed, Uint, I256, U160, U256};
use alloy::providers::Provider;
use alloy::transports::Transport;
use anyhow::Result;
use log::trace;
use pool_sync::{Pool, PoolInfo};
use revm::DatabaseRef;
use std::ops::{BitAnd, Shl, Shr};

use super::BlockStateDB;
use crate::state_db::blockstate_db::{InsertionType, BlockStateDBSlot};

// === Bitmasks used for packing slot0 ===
lazy_static! {
    static ref BITS160MASK: U256 = U256::from(1).shl(160) - U256::from(1);
    static ref BITS128MASK: U256 = U256::from(1).shl(128) - U256::from(1);
    static ref BITS24MASK: U256 = U256::from(1).shl(24) - U256::from(1);
    static ref BITS16MASK: U256 = U256::from(1).shl(16) - U256::from(1);
    static ref BITS8MASK: U256 = U256::from(1).shl(8) - U256::from(1);
    static ref BITS1MASK: U256 = U256::from(1);
}

// === Contract Slot0 Signature ===
sol!(
    #[derive(Debug)]
    contract UniswapV3 {
        function slot0() external view returns (
            uint160 sqrtPriceX96,
            int24 tick,
            uint16 observationIndex,
            uint16 observationCardinality,
            uint16 observationCardinalityNext,
            uint8 feeProtocol,
            bool unlocked
        );
    }
);

// === V3 Pool Insertion Logic ===
impl<N, P> BlockStateDB<N, P>
where
    N: Network,
    P: Provider<N>,
{
    pub fn insert_v3(&mut self, pool: Pool) -> Result<()> {
        trace!("Inserting V3 Pool: {}", pool.address());
        let address = pool.address();
        self.add_pool(pool.clone());
        let v3 = pool.get_v3().expect("Missing V3 pool details");

        self.insert_slot0(address, U160::from(v3.sqrt_price), v3.tick)?;
        self.insert_liquidity(address, v3.liquidity)?;
        self.insert_tick_spacing(address, v3.tick_spacing)?;

        for (tick, liq) in v3.ticks.iter() {
            self.insert_tick_liquidity_net(address, *tick, liq.liquidity_net)?;
        }

        for (tick, bitmap) in v3.tick_bitmap.iter() {
            self.insert_tick_bitmap(address, *tick, *bitmap)?;
        }

        Ok(())
    }

    fn insert_tick_bitmap(&mut self, pool: Address, tick: i16, bitmap: U256) -> Result<()> {
        trace!("Insert Tick Bitmap: {} @ Tick {}", pool, tick);
        let mut key = I256::try_from(tick)?.to_be_bytes::<32>().to_vec();
        key.extend(U256::from(6).to_be_bytes::<32>());
        let slot = keccak256(&key);

        let account = self.accounts.get_mut(&pool).expect("Pool not found in DB");
        account.storage.insert(U256::from_be_bytes(slot.into()), BlockStateDBSlot {
            value: bitmap,
            insertion_type: InsertionType::Custom,
        });

        Ok(())
    }

    fn insert_tick_liquidity_net(&mut self, pool: Address, tick: i32, liquidity_net: i128) -> Result<()> {
        trace!("Insert Tick Liquidity: {} @ Tick {}", pool, tick);
        let unsigned = liquidity_net as u128;

        let mut key = I256::try_from(tick)?.to_be_bytes::<32>().to_vec();
        key.extend(U256::from(5).to_be_bytes::<32>());
        let slot = keccak256(&key);

        let shifted = U256::from(unsigned) << 128;

        let account = self.accounts.get_mut(&pool).expect("Pool not found in DB");
        account.storage.insert(U256::from_be_bytes(slot.into()), BlockStateDBSlot {
            value: shifted,
            insertion_type: InsertionType::Custom,
        });

        Ok(())
    }

    fn insert_liquidity(&mut self, pool: Address, liquidity: u128) -> Result<()> {
        trace!("Insert Liquidity: {}", pool);
        let account = self.accounts.get_mut(&pool).expect("Pool not found in DB");
        account.storage.insert(U256::from(4), BlockStateDBSlot {
            value: U256::from(liquidity),
            insertion_type: InsertionType::Custom,
        });
        Ok(())
    }

    fn insert_slot0(&mut self, pool: Address, sqrt_price: U160, tick: i32) -> Result<()> {
        trace!("Insert Slot0: {} | sqrtPriceX96={}, tick={}", pool, sqrt_price, tick);
        let value = U256::from(sqrt_price)
            | ((U256::from(tick as u32) & *BITS24MASK) << 160)
            | (U256::ZERO << (160 + 24))  // observationIndex
            | (U256::ZERO << (160 + 24 + 16))  // observationCardinality
            | (U256::ZERO << (160 + 24 + 16 + 16))  // observationCardinalityNext
            | (U256::ZERO << (160 + 24 + 16 + 16 + 16))  // feeProtocol
            | (U256::from(1u8) << (160 + 24 + 16 + 16 + 16 + 8)); // unlocked=true

        let account = self.accounts.get_mut(&pool).expect("Pool not found in DB");
        account.storage.insert(U256::from(0), BlockStateDBSlot {
            value,
            insertion_type: InsertionType::Custom,
        });

        Ok(())
    }

    fn insert_tick_spacing(&mut self, pool: Address, tick_spacing: i32) -> Result<()> {
        trace!("Insert Tick Spacing: {} = {}", pool, tick_spacing);
        let account = self.accounts.get_mut(&pool).expect("Pool not found in DB");
        account.storage.insert(U256::from(14), BlockStateDBSlot {
            value: U256::from(tick_spacing),
            insertion_type: InsertionType::Custom,
        });
        Ok(())
    }
}
