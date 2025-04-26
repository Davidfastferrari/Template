use alloy::network::Network;
use alloy::primitives::{Address, BlockNumber, B256, U256};
use alloy::providers::Provider;
use alloy::rpc::types::{BlockId};
use alloy::rpc::types::trace::geth::AccountState as GethAccountState;
use alloy::transports::{Transport, TransportError};
use anyhow::Result;
use log::{debug, trace, warn};
use pool_sync::{Pool, PoolInfo};
use revm::{Database, DatabaseCommit, DatabaseRef};
use revm::db::AccountState;
use revm::primitives::{Account, AccountInfo, Bytecode, Log, KECCAK_EMPTY};
use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use tokio::runtime::{Handle, Runtime};
use revm::primitives::Account as RevmAccount;
use revm::revm_database::alloydb::DBTransportError; // ✅ fix: valid DBErrorMarker implementor

type Error = DBTransportError;

// Handles either a current thread Handle or a dedicated Runtime
#[derive(Debug)]
pub enum HandleOrRuntime {
    Handle(Handle),
    Runtime(Runtime),
}

impl HandleOrRuntime {
    pub fn block_on<F: std::future::Future + Send>(&self, fut: F) -> F::Output
    where F::Output: Send {
        match self {
            HandleOrRuntime::Handle(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
            HandleOrRuntime::Runtime(rt) => rt.block_on(fut),
        }
    }
}

#[derive(Debug)]
pub struct BlockStateDB< N: Network, P: Provider<T, N>> {
    pub accounts: HashMap<Address, BlockStateDBAccount>,
    pub contracts: HashMap<B256, Bytecode>,
    pub _logs: Vec<Log>,
    pub block_hashes: HashMap<BlockNumber, B256>,
    pub pools: HashSet<Address>,
    pub pool_info: HashMap<Address, Pool>,
    provider: P,
    runtime: HandleOrRuntime,
    _marker: PhantomData<fn() -> (N)>,
}

impl<N, P> BlockStateDB<N, P>
where
    N: Network,
    P: Provider<N>,
{
    /// Construct a new BlockStateDB with appropriate runtime handle
    pub fn new(provider: P) -> Option<Self> {
        debug!("Creating new BlockStateDB");

        let mut contracts = HashMap::new();
        contracts.insert(KECCAK_EMPTY, Bytecode::default());
        contracts.insert(B256::ZERO, Bytecode::default());

        let runtime = match Handle::try_current() {
            Ok(handle) => match handle.runtime_flavor() {
                tokio::runtime::RuntimeFlavor::CurrentThread => return None,
                _ => HandleOrRuntime::Handle(handle),
            },
            Err(_) => return None,
        };

        Some(Self {
            accounts: HashMap::new(),
            contracts,
            _logs: Vec::new(),
            block_hashes: HashMap::new(),
            pools: HashSet::new(),
            pool_info: HashMap::new(),
            provider,
            runtime,
            _marker: PhantomData,
        })
    }

    /// Add a new pool to the DB (fetch on-chain account, store it with type)
    pub fn add_pool(&mut self, pool: Pool) {
        let pool_address = pool.address();
        trace!("Adding pool {} to database", pool_address);

        self.pools.insert(pool_address);
        self.pool_info.insert(pool_address, pool.clone());

        if let Ok(Some(account_info)) = <Self as DatabaseRef>::basic_ref(self, pool_address) {
            self.accounts.insert(pool_address, BlockStateDBAccount {
                info: account_info,
                insertion_type: InsertionType::OnChain,
                ..Default::default()
            });
        } else {
            warn!("Failed to fetch or insert account info for pool {pool_address}");
        }
    }

    #[inline]
    pub fn get_pool(&self, addr: &Address) -> &Pool {
        self.pool_info.get(addr).expect("Missing pool info")
    }

    #[inline]
    pub fn tracking_pool(&self, addr: &Address) -> bool {
        self.pools.contains(addr)
    }

    #[inline]
    pub fn zero_to_one(&self, pool: &Address, token_in: Address) -> Option<bool> {
        self.pool_info.get(pool).map(|info| info.token0_address() == token_in)
    }

    /// Update all storage slots for a given account from a block trace
    #[inline]
    pub fn update_all_slots(
        &mut self,
        address: Address,
        account_state: GethAccountState,
    ) -> Result<()> {
        trace!("Updating storage for address {}", address);
        for (slot, value) in account_state.storage {
            if let Some(account) = self.accounts.get_mut(&address) {
                account.storage.insert(slot.into(), BlockStateDBSlot {
                    value: value.into(),
                    insertion_type: InsertionType::Custom,
                });
            }
        }
        Ok(())
    }

    /// Direct insert of an account into the state DB
    pub fn insert_account_info(
        &mut self,
        address: Address,
        info: AccountInfo,
        insertion_type: InsertionType,
    ) {
        self.accounts.insert(address, BlockStateDBAccount {
            info,
            insertion_type,
            ..Default::default()
        });
    }

    /// Insert a specific storage value
    pub fn insert_account_storage(
        &mut self,
        address: Address,
        slot: U256,
        value: U256,
        insertion_type: InsertionType,
    ) -> Result<()> {
        if let Some(account) = self.accounts.get_mut(&address) {
            account.storage.insert(slot, BlockStateDBSlot {
                value,
                insertion_type,
            });
            return Ok(());
        }

        let account_info = self.basic(address)?.unwrap();
        self.insert_account_info(address, account_info, insertion_type);
        self.accounts.get_mut(&address).unwrap().storage.insert(slot, BlockStateDBSlot {
            value,
            insertion_type,
        });
        Ok(())
    }
}

impl<N, P> Database for BlockStateDB<N, P>
where
    N: Network,
    P: Provider<N>,
{
    type Error = DBTransportError;

    /// Return account info or query from provider and insert if missing.
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        trace!("Fetching account: {address}");
        if let Some(acc) = self.accounts.get(&address) {
            return Ok(Some(acc.info.clone()));
        }

        // Not in DB, query provider.
        let info = <Self as DatabaseRef>::basic_ref(self, address)?.unwrap();
        self.insert_account_info(address, info.clone(), InsertionType::OnChain);
        Ok(Some(info))
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        if let Some(code) = self.contracts.get(&code_hash) {
            return Ok(code.clone());
        }

        // Fetch fallback — though this shouldn't normally happen due to preload
        let code = <Self as DatabaseRef>::code_by_hash_ref(self, code_hash)?;
        self.contracts.insert(code_hash, code.clone());
        Ok(code)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        if let Some(acc) = self.accounts.get(&address) {
            if let Some(slot) = acc.storage.get(&index) {
                return Ok(slot.value);
            }
        }

        let value = <Self as DatabaseRef>::storage_ref(self, address, index)?;
        let account = self.accounts.entry(address).or_default();
        account.storage.insert(index, BlockStateDBSlot {
            value,
            insertion_type: InsertionType::OnChain,
        });
        Ok(value)
    }

    fn block_hash(&mut self, number: BlockNumber) -> Result<B256, Self::Error> {
        if let Some(hash) = self.block_hashes.get(&number) {
            return Ok(*hash);
        }

        let hash = <Self as DatabaseRef>::block_hash_ref(self, number)?;
        self.block_hashes.insert(number, hash);
        Ok(hash)
    }
}

impl<N, P> DatabaseRef for BlockStateDB<N, P>
where
    N: Network,
    P: Provider<N>,
{
    type Error = DBTransportError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        if let Some(acc) = self.accounts.get(&address) {
            return Ok(Some(acc.info.clone()));
        }

        // fetch fresh data from provider
        let fut = async {
            let nonce = self.provider.get_transaction_count(address).block_id(BlockId::latest());
            let balance = self.provider.get_balance(address).block_id(BlockId::latest());
            let code = self.provider.get_code_at(address).block_id(BlockId::latest());
            tokio::join!(nonce, balance, code)
        };
        let (nonce, balance, code) = self.runtime.block_on(fut);
        match (nonce, balance, code) {
            (Ok(n), Ok(b), Ok(c)) => {
                let bytecode = Bytecode::new_raw(c.0.into());
                let hash = bytecode.hash_slow();
                Ok(Some(AccountInfo::new(b, n, hash, bytecode)))
            }
            _ => Ok(None),
        }
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.contracts
            .get(&code_hash)
            .cloned()
            .ok_or_else(|| TransportError::Custom("Missing code hash".into()))
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let fut = self.provider.get_storage_at(address, index);
        Ok(self.runtime.block_on(fut.into_future())?)
    }

    fn block_hash_ref(&self, number: BlockNumber) -> Result<B256, Self::Error> {
        if let Some(hash) = self.block_hashes.get(&number) {
            return Ok(*hash);
        }

        let block = self.runtime.block_on(
            self.provider
                .get_block_by_number(number.into(), false.into()),
        )?;
        Ok(block.map(|b| B256::new(*b.header().hash())).unwrap_or(B256::ZERO))
    }
}

impl<N, P> BlockStateDB<N, P>
where
    N: Network,
    P: Provider<N>,
{
    /// Commit post-execution state changes from the EVM
    pub fn commit(&mut self, changes: HashMap<Address, RevmAccount>) {
        for (addr, mut acc) in changes {
            if !acc.is_touched() {
                continue;
            }

            let db_acc = self.accounts.entry(addr).or_default();

            if acc.is_selfdestructed() {
                db_acc.storage.clear();
                db_acc.info = AccountInfo::default();
                db_acc.state = AccountState::NotExisting;
                continue;
            }

            if acc.is_created() {
                db_acc.storage.clear();
                db_acc.state = AccountState::StorageCleared;
            } else if !db_acc.state.is_storage_cleared() {
                db_acc.state = AccountState::Touched;
            }

            // Inject any code updates
            if let Some(code) = &acc.info.code {
                if !code.is_empty() {
                    if acc.info.code_hash == KECCAK_EMPTY {
                        acc.info.code_hash = code.hash_slow();
                    }
                    self.contracts.entry(acc.info.code_hash).or_insert_with(|| code.clone());
                }
            }

            db_acc.info = acc.info;

            // Apply storage updates
            db_acc.storage.extend(acc.storage.into_iter().map(|(slot, value)| {
                (
                    slot,
                    BlockStateDBSlot {
                        value: value.present_value(),
                        insertion_type: InsertionType::Custom,
                    },
                )
            }));
        }
    }
}

#[derive(Default, Debug, Eq, PartialEq)]
pub struct BlockStateDBSlot {
    pub value: U256,
    pub insertion_type: InsertionType,
}

#[derive(Default, Debug, Eq, PartialEq)]
pub enum InsertionType {
    Custom,
    #[default]
    OnChain,
}

#[derive(Default, Debug, Eq, PartialEq)]
pub struct BlockStateDBAccount {
    pub info: AccountInfo,
    pub state: AccountState,
    pub storage: HashMap<U256, BlockStateDBSlot>,
    pub insertion_type: InsertionType,
}

impl BlockStateDBAccount {
    pub fn new(insertion_type: InsertionType) -> Self {
        Self {
            info: AccountInfo::default(),
            state: AccountState::NotExisting,
            storage: HashMap::new(),
            insertion_type,
        }
    }
}


