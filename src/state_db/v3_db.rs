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

// Bitmasks for storage insertion
lazy_static {
    static ref U112_MASK: U256 = (U256::from(1) << 112) - U256::from(1);
}

lazy_static {
    static ref BITS160MASK: U256 = U256::from(1).shl(160) - U256::from(1);
    static ref BITS128MASK: U256 = U256::from(1).shl(128) - U256::from(1);
    static ref BITS24MASK: U256 = U256::from(1).shl(24) - U256::from(1);
    static ref BITS16MASK: U256 = U256::from(1).shl(16) - U256::from(1);
    static ref BITS8MASK: U256 = U256::from(1).shl(8) - U256::from(1);
    static ref BITS1MASK: U256 = U256::from(1);
}

// Function signature for Slot0 call
sol(
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

/// uniswapv3 db read/write related methods
// UniswapV3 DB read and write related methods
impl<T, N, P> BlockStateDB<T, N, P>
where
    T: Transport + Clone,
    N: Network,
    P: Provider<T, N>,
{
    // Insert a new uniswapv3 pool into the database
    pub fn insert_v3(&mut self, pool: Pool) -> Result<()> {
        trace!("Adding new v3 pool {}", pool.address());
        let address = pool.address();

        // track the pool
        self.add_pool(pool.clone());

        // extract the v3 pool
        let v3_pool = pool.get_v3().unwrap();


        // Insert slot, liquidity, tick spacing
        self.insert_slot0(address, U160::from(v3_pool.sqrt_price), v3_pool.tick)?;
        self.insert_liquidity(address, v3_pool.liquidity)?;
        self.insert_tick_spacing(address, v3_pool.tick_spacing)?;

        // Insert tick-related data
        for (tick, liquidity_net) in v3_pool.ticks.clone() {
            self.insert_tick_liquidity_net(address, tick, liquidity_net.liquidity_net)?;
        }

        // Insert tick bitmap
        for (word_pos, bitmap) in v3_pool.tick_bitmap.clone() {
            self.insert_tick_bitmap(address, word_pos, bitmap)?;
        }

        Ok(())
    }

    // Insert tick bitmap
    fn insert_tick_bitmap(&mut self, pool: Address, tick: i16, bitmap: U256) -> Result<()> {
        trace!(
            "V3 Database: Inserting tick bitmap for tick {} in pool {}",
            tick,
            pool
        );
        // Hash slot exactly as in read operation
        let tick_bytes = I256::try_from(tick)?.to_be_bytes::<32>();
        let mut buf = tick_bytes.to_vec();
        buf.append(&mut U256::from(6).to_be_bytes::<32>().to_vec());
        let slot = keccak256(buf.as_slice());

        let account = self.accounts.get_mut(&pool).unwrap();
        let new_db_slot = BlockStateDBSlot {
            value: bitmap,
            insertion_type: InsertionType::Custom
        };
        account
            .storage
            .insert(U256::from_be_bytes(slot.into()), new_db_slot);
        Ok(())
    }

    // Insert the pool liquidity
    fn insert_liquidity(&mut self, pool: Address, liquidity: u128) -> Result<()> {
        trace!("V3 Database: Inserting liquidity for {}", pool);
        let account = self.accounts.get_mut(&pool).unwrap();
        let new_db_slot = BlockStateDBSlot {
            value: U256::from(liquidity),
            insertion_type: InsertionType::Custom
        };
        account.storage.insert(U256::from(4), new_db_slot);
        Ok(())
    }

    // Insert tick liquidity
    fn insert_tick_liquidity_net(
        &mut self,
        pool: Address,
        tick: i32,
        liquidity_net: i128,
    ) -> Result<()> {
        trace!(
            "V3 Database: Inserting tick liquidity net for tick {} in pool {}",
            tick,
            pool
        );
        // Convert signed 128-bit to unsigned representation matching the read operation
        let unsigned_liquidity = liquidity_net as u128;

        // Hash slot
        let tick_bytes = I256::try_from(tick)?.to_be_bytes::<32>();
        let mut buf = tick_bytes.to_vec();
        buf.append(&mut U256::from(5).to_be_bytes::<32>().to_vec());
        let slot = keccak256(buf.as_slice());

        // Convert to U256 and shift left by 128 bits (inverse of the right shift in read)
        let value = U256::from(unsigned_liquidity) << 128;

        let account = self.accounts.get_mut(&pool).unwrap();
        let new_db_slot = BlockStateDBSlot {
            value,
            insertion_type: InsertionType::Custom
        };
        account
            .storage
            .insert(U256::from_be_bytes(slot.into()), new_db_slot);
        Ok(())
    }

    // Insert slot0
    fn insert_slot0(&mut self, pool: Address, sqrt_price: U160, tick: i32) -> Result<()> {
        trace!("V3 Database: Inserting slot0 for {}", pool);

        // Extract from read operation:
        // feeProtocol: ((Shr::<U256>::shr(cell, U256::from(160 + 24 + 16 + 16 + 16))) & *BITS8MASK)
        let slot0 = U256::from(sqrt_price)
            | ((U256::from(tick as u32) & *BITS24MASK) << 160)
            | (U256::from(0) << (160 + 24))
            | (U256::from(0) << (160 + 24 + 16))
            | (U256::from(0) << (160 + 24 + 16 + 16))
            | ((U256::from(0) & *BITS8MASK) << (160 + 24 + 16 + 16 + 16))
            | (U256::from(1u8) << (160 + 24 + 16 + 16 + 16 + 8));

        let account = self.accounts.get_mut(&pool).unwrap();
        let new_db_slot = BlockStateDBSlot {
            value: slot0,
            insertion_type: InsertionType::Custom
        };
        account.storage.insert(U256::from(0), new_db_slot);
        Ok(())
    }

    fn insert_tick_spacing(&mut self, pool: Address, tick_spacing: i32) -> Result<()> {
        trace!("V3 Database: Inserting tick spacing for {}", pool);

        // get the account and insert into slot 14
        let account = self.accounts.get_mut(&pool).unwrap();
        let new_db_slot = BlockStateDBSlot {
            value: U256::from(tick_spacing),
            insertion_type: InsertionType::Custom
        };
        account.storage.insert(U256::from(14), new_db_slot);
        Ok(())
    }

    #[inline]
    pub fn tick_spacing(&self, address: &Address) -> Result<i32> {
        let data = self.accounts.get(address).unwrap().storage.get(&U256::from(14)).unwrap();
        let data = data.value;
        let tick_spacing: i32 = data.saturating_to();
        Ok(tick_spacing)
    }

    // Get slot 0
    #[inline]
    pub fn slot0(&self, address: Address) -> Result<UniswapV3::slot0Return> {
        let cell = *self.accounts.get(&address).unwrap().storage.get(&U256::from(0)).unwrap();
        let cell = cell.value;
        let tick: Uint<24, 1> = ((Shr::<U256>::shr(cell, U256::from(160))) & *BITS24MASK).to();
        let tick: Signed<24, 1> = Signed::<24, 1>::from_raw(tick);
        let tick: i32 = tick.as_i32();

        let sqrt_price_x96: U160 = cell.bitand(*BITS160MASK).to();

        Ok(UniswapV3::slot0Return {
            sqrtPriceX96: sqrt_price_x96,
            tick: tick.try_into()?,
            observationIndex: ((Shr::<U256>::shr(cell, U256::from(160 + 24))) & *BITS16MASK).to(),
            observationCardinality: ((Shr::<U256>::shr(cell, U256::from(160 + 24 + 16)))
                & *BITS16MASK)
                .to(),
            observationCardinalityNext: ((Shr::<U256>::shr(cell, U256::from(160 + 24 + 16 + 16)))
                & *BITS16MASK)
                .to(),
            feeProtocol: ((Shr::<U256>::shr(cell, U256::from(160 + 24 + 16 + 16 + 16)))
                & *BITS8MASK)
                .to(),
            unlocked: ((Shr::<U256>::shr(cell, U256::from(160 + 24 + 16 + 16 + 16 + 8)))
                & *BITS1MASK)
                .to(),
        })
    }

    #[inline]
    pub fn liquidity(&self, address: Address) -> Result<u128> {
        let cell = self.accounts.get(&address).unwrap().storage.get(&U256::from(4)).unwrap();
        let cell = cell.value;
        let cell: u128 = cell.saturating_to();
        Ok(cell)
    }

    #[inline]
    pub fn ticks_liquidity_net(&self, address: Address, tick: i32) -> Result<i128> {
        //i24
        let cell = self.read_hashed_slot(
            &address,
            &U256::from(5),
            &U256::from_be_bytes(I256::try_from(tick)?.to_be_bytes::<32>()),
        )?;
        let unsigned_liqudity: Uint<128, 2> = cell.shr(U256::from(128)).to();
        let lu128: u128 = unsigned_liqudity.to();
        let li128: i128 = lu128 as i128;

        Ok(li128)
    }

    #[inline]
    pub fn tick_bitmap(&self, address: Address, tick: i16) -> Result<U256> {
        //i16
        let cell = self.read_hashed_slot(
            &address,
            &U256::from(6),
            &U256::from_be_bytes(I256::try_from(tick)?.to_be_bytes::<32>()),
        )?;
        Ok(cell)
    }

    #[inline]
    fn read_hashed_slot(
        &self,
        account: &Address,
        hashmap_offset: &U256,
        item: &U256,
    ) -> Result<U256> {
        let mut buf = item.to_be_bytes::<32>().to_vec();
        buf.append(&mut hashmap_offset.to_be_bytes::<32>().to_vec());
        let slot: U256 = keccak256(buf.as_slice()).into();
        Ok(self.storage_ref(*account, slot)?)
    }
}

#[cfg(test)]
mod v3_db_test {
    use super::*;
    use alloy::primitives::address;
    use alloy::primitives::aliases::I24;
    use alloy::providers::ProviderBuilder;
    use pool_sync::{TickInfo, UniswapV3Pool};
    use std::collections::HashMap;

    fn create_test_pool() -> UniswapV3Pool {
        // Set up tick bitmap
        let mut tick_bitmap = HashMap::new();
        tick_bitmap.insert(-58, U256::from(2305843009213693952_u128));

        // Set up ticks
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
    async fn test_insert_and_retrieve() -> Result<()> {
        // Initialize environment and provider
        dotenv::dotenv().ok();
        let url = std::env::var("FULL")
            .expect("FULL env var not set")
            .parse()?;
        let provider = ProviderBuilder::new().on_http(url);
        let mut db = BlockStateDB::new(provider).unwrap();

        // Create and insert test pool
        let pool = create_test_pool();
        let pool_addr = pool.address;
        let expected_liquidity = pool.liquidity;
        let expected_sqrt_price = U160::from(pool.sqrt_price);
        let expected_tick = I24::try_from(pool.tick).unwrap();

        db.insert_v3(Pool::UniswapV3(pool))?;

        // Test slot0 values
        let slot0 = db.slot0(pool_addr).unwrap();
        assert_eq!(
            slot0.sqrtPriceX96, expected_sqrt_price,
            "Incorrect sqrt price"
        );
        assert_eq!(slot0.tick, expected_tick, "Incorrect tick");
        assert_eq!(slot0.feeProtocol, 0, "Fee protocol should be 0");
        assert!(slot0.unlocked, "Pool should be unlocked");

        // Test liquidity
        let liquidity = db.liquidity(pool_addr)?;
        assert_eq!(liquidity, expected_liquidity, "Incorrect liquidity value");

        // Test tick liquidityNet
        let tick_liquidity = db.ticks_liquidity_net(pool_addr, -887220)?;
        assert_eq!(
            tick_liquidity, 14809333843350818121657,
            "Incorrect tick liquidity net"
        );

        // Test tick bitmap
        let tick_bitmap = db.tick_bitmap(pool_addr, -58)?;
        assert_eq!(
            tick_bitmap,
            U256::from(2305843009213693952_u128),
            "Incorrect tick bitmap"
        );

        Ok(())
    }
}
