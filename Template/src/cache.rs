use tracing::{info, error, debug, warn};
use alloy_sol_types::sol;
use serde::{Serialize, Deserialize};
use serde_json::json;
use alloy::primitives::{Address, U256};
use dashmap::DashMap;
use rustc_hash::FxHasher;
use std::hash::{BuildHasherDefault, Hash, Hasher};

/// Custom hasher based on `FxHasher` (fast non-cryptographic hashing)
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

/// Composite key to cache a specific pool's quote with an exact input amount
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
struct CacheKey {
    pub pool_address: Address,
    pub amount_in: U256,
}

impl Hash for CacheKey {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.pool_address.hash(state);
        self.amount_in.hash(state);
    }
}

/// Represents a single output entry from a simulation or estimation
#[derive(Clone, Copy, Debug)]
struct CacheEntry {
    pub output_amount: U256,
}

/// A concurrent, fast read/write cache for pool simulations and estimations
pub struct Cache {
    entries: DashMap<CacheKey, CacheEntry, BuildHasherDefault<CacheHasher>>,
}

impl Cache {
    /// Construct a new cache sized based on the expected number of pools.
    /// We estimate 100 input variations per pool to preallocate capacity.
    pub fn new(num_pools: usize) -> Self {
        Self {
            entries: DashMap::with_capacity_and_hasher(
                num_pools * 100,
                BuildHasherDefault::<CacheHasher>::default(),
            ),
        }
    }

    /// Retrieves a cached output amount for a given pool + input amount.
    #[inline]
    pub fn get(&self, amount_in: U256, pool_address: Address) -> Option<U256> {
        let key = CacheKey {
            pool_address,
            amount_in,
        };
        self.entries.get(&key).map(|entry| entry.output_amount)
    }

    /// Stores a new output amount in the cache
    #[inline]
    pub fn insert(&self, amount_in: U256, pool_address: Address, output_amount: U256) {
        let key = CacheKey {
            pool_address,
            amount_in,
        };
        self.entries.insert(key, CacheEntry { output_amount });
    }

    /// Invalidate all cache entries for a given pool
    #[inline]
    pub fn invalidate(&self, pool_address: Address) {
        self.entries.retain(|key, _| key.pool_address != pool_address);
    }

    /// Clears all entries in the cache
    #[inline]
    pub fn clear(&self) {
        self.entries.clear();
    }

    /// Total entries in the cache
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Checks if the cache is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{address, U256};

    #[test]
    fn test_insert_get_invalidate() {
        let cache = Cache::new(10);
        let pool = address!("4200000000000000000000000000000000000006");
        let amount = U256::from(1000);
        let output = U256::from(2000);

        assert!(cache.get(amount, pool).is_none());
        cache.insert(amount, pool, output);
        assert_eq!(cache.get(amount, pool), Some(output));

        cache.invalidate(pool);
        assert!(cache.get(amount, pool).is_none());
    }

    #[test]
    fn test_clear_and_len() {
        let cache = Cache::new(2);
        let addr = address!("0000000000000000000000000000000000000001");

        for i in 0..5 {
            cache.insert(U256::from(i), addr, U256::from(i * 10));
        }

        assert_eq!(cache.len(), 5);
        cache.clear();
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
    }
}
