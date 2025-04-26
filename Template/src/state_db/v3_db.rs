// === External Imports ===
use super::BlockStateDB;
use alloy::network::Network;
use alloy::primitives::{keccak256, Address, Signed, Uint, I256, U160, U256};
use alloy::providers::Provider;
use alloy::sol;
use alloy::transports::Transport;
use anyhow::Result;
use lazy_static::lazy_static;
use log::trace;
use pool_sync::{Pool, PoolInfo};
use revm::DatabaseRef;
use std::ops::{BitAnd, Shl, Shr};
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

#[cfg(test)]
mod v3_db_test {
    use super::*;
    use alloy::primitives::address;
    use alloy::primitives::aliases::{I24, U24};
    use alloy::providers::{ProviderBuilder, RootProvider};
    use alloy::transports::http::{Http, Client};
    use pool_sync::{TickInfo, UniswapV3Pool, Pool};
    use std::collections::HashMap;
    use proptest::prelude::*;
    use std::sync::Arc;

    fn create_test_pool() -> UniswapV3Pool {
        let mut tick_bitmap = HashMap::new();
        tick_bitmap.insert(-58, U256::from(0b10_0000));

        let mut ticks = HashMap::new();
        ticks.insert(
            -887220,
            TickInfo {
                liquidity_net: 14809333843350818121657,
                initialized: true,
                liquidity_gross: 14809333843350818121657,
            },
        );

        UniswapV3Pool {
            address: address!("e375e4dd3fc5bf117aa00c5241dd89ddd979a2c4"),
            token0: address!("0578d8a44db98b23bf096a382e016e29a5ce0ffe"),
            token1: address!("27501bdd6a4753dffc399ee20eb02b304f670f50"),
            token0_name: "USDC".to_string(),
            token1_name: "WETH".to_string(),
            token0_decimals: 6,
            token1_decimals: 18,
            liquidity: 21775078430692230315408,
            sqrt_price: U256::from(4654106501023758788420274431_u128),
            fee: 3000,
            tick: -56695,
            tick_spacing: 60,
            tick_bitmap,
            ticks,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_insert_and_query_v3_db() -> Result<()> {
        dotenv::dotenv().ok();
        let url = std::env::var("FULL").expect("FULL node URL required").parse()?;
        let provider = ProviderBuilder::new().on_http(url);
        let mut db = BlockStateDB::new(provider)?;

        let pool = create_test_pool();
        let addr = pool.address;
        let expected_liq = pool.liquidity;
        let expected_tick = I24::try_from(pool.tick).unwrap();
        let expected_sqrt = U160::from(pool.sqrt_price);

        db.insert_v3(Pool::UniswapV3(pool))?;

        let slot0 = db.slot0(addr)?;
        assert_eq!(slot0.sqrtPriceX96, expected_sqrt);
        assert_eq!(slot0.tick, expected_tick);
        assert!(slot0.unlocked);

        assert_eq!(db.liquidity(addr)?, expected_liq);
        assert_eq!(db.ticks_liquidity_net(addr, -887220)?, 14809333843350818121657);
        assert_eq!(db.tick_bitmap(addr, -58)?, U256::from(0b10_0000));

        Ok(())
    }

    proptest! {
        #[test]
        fn prop_tick_storage_alignment(tick in -887_272i32..=887_272) {
            let tick_bytes = I256::try_from(tick).unwrap().to_be_bytes::<32>();
            let offset = U256::from(5);
            let mut concat = tick_bytes.to_vec();
            concat.extend(offset.to_be_bytes::<32>());
            let slot = keccak256(&concat);
            prop_assert_eq!(slot.len(), 32);
        }

        #[test]
        fn prop_liquidity_insert_extract(v in any::<u128>()) {
            let db_slot = BlockStateDBSlot {
                value: U256::from(v),
                insertion_type: InsertionType::Custom,
            };
            let raw = db_slot.value.saturating_to::<u128>();
            prop_assert_eq!(raw, v);
        }

        #[test]
        fn prop_sqrt_price_bitmasking(sqrt_price in any::<u160>()) {
            let full = U256::from(sqrt_price);
            let masked: U160 = full.bitand(*BITS160MASK).to();
            prop_assert_eq!(masked, sqrt_price);
        }
    }
}
