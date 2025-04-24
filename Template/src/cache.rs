use alloy::primitives::{Address, U256};
use dashmap::DashMap;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

// Custom hasher for better performance
#[derive(Default)]
struct CacheHasher(FxHasher);

impl Hasher for CacheHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0.finish()
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes)
    }
}

// Efficient cache key
#[derive(PartialEq, Eq, Clone, Copy)]
struct CacheKey {
    pool_address: Address,
    amount_in: U256,
}

impl Hash for CacheKey {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.pool_address.hash(state);
        self.amount_in.hash(state);
    }
}

#[derive(Clone, Copy)]
struct CacheEntry {
    output_amount: U256,
}

pub struct Cache {
    entries: DashMap<CacheKey, CacheEntry, std::hash::BuildHasherDefault<CacheHasher>>,
}

impl Cache {
    pub fn new(num_pools: usize) -> Self {
        Self {
            entries: DashMap::with_capacity_and_hasher(
                num_pools * 100, // Assume 100 different input amounts per pool
                std::hash::BuildHasherDefault::default(),
            ),
        }
    }

    #[inline]
    pub fn get(&self, amount_in: U256, pool_address: Address) -> Option<U256> {
        let key = CacheKey {
            pool_address,
            amount_in,
        };
        self.entries.get(&key).map(|entry| entry.output_amount)
    }

    #[inline]
    pub fn invalidate(&self, pool_address: Address) {
        self.entries
            .retain(|key, _| key.pool_address != pool_address);
    }
}
