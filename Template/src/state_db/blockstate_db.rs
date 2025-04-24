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
use tokio::runtime::Handle;

#[derive(Debug)]
pub struct Application {
    event_loop: EventLoop<()>,
    window: Window,
}

#[derive(Debug)]
pub enum HandleOrRuntime {
    Handle(Handle),
}

impl HandleOrRuntime {
    #[inline]
    pub fn block_on<F>(&self, f: F) -> F::Output
    where
        F: std::future::Future + Send,
        F::Output: Send,
    {
        match self {
            Self::Handle(handle) => tokio::task::block_in_place(move || handle.block_on(f)),
            Self::Runtime(rt) => rt.block_on(f),
        }
    }
}

#[derive(Debug)]
pub struct BlockStateDB<N: Network, P: Provider<N>> BlockStateDB<N, P> {
    // All of the accounts
    pub accounts: HashMap<Address, BlockStateDBAccount>,

    // The contracts that this database holds
    pub contracts: HashMap<B256, Bytecode>,
    // Logs??
    pub _logs: Vec<Log>,
    // Block hashs???
    pub block_hashes: HashMap<BlockNumber, B256>,

    // The pools that are in our working set
    pub pools: HashSet<Address>,
    //
    pub pool_info: HashMap<Address, Pool>,
    // provider for fetching information
    provider: P,
    runtime: HandleOrRuntime,
    _marker: std::marker::PhantomData<fn() -> (T, N)>,
}

impl<N: Network, P: Provider<N>> BlockStateDB<N, P>{
    // Construct a new BlockStateDB
    pub fn new(provider: P) -> Option<Self> {
        debug!("Creating new BlockStateDB");
        let contracts = HashMap::new();
        contracts.insert(KECCAK_EMPTY, Bytecode::default());
        contracts.insert(B256::ZERO, Bytecode::default());

        // get our runtime handle
        let rt = match Handle::try_current() {
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
            runtime: rt,
            _marker: std::marker::PhantomData,
        })
    }

    // Record a new pool in our working set
    pub fn add_pool(
        &mut self,
        pool: Pool
    ) {

        let pool_address = pool.address();
        trace!("Adding pool {} to database", pool_address);

        // track the pool and the pool information
        self.pools.insert(pool_address);
        self.pool_info.insert(pool_address, pool);

        // fetch the onchain pool account and insert it into database
        // this is onchain because it has onchain state, the slots will be custom
        let pool_account = <Self as DatabaseRef>::basic_ref(self, pool_address);
        let new_db_account = BlockStateDBAccount {
            info: pool_account.unwrap().unwrap(),
            insertion_type: InsertionType::OnChain,
            ..Default::default()
        };
        self.accounts.insert(pool_address, new_db_account);
    }


    // Get a pool corresponding to an address
    pub fn get_pool(&self, pool_address: &Address) -> &Pool {
        self.pool_info.get(pool_address).unwrap()
    }


    // Check if we are tracking the pool. This is our working set
    #[inline]
    pub fn tracking_pool(&self, pool: &Address) -> bool {
        self.pools.contains(pool)
    }

    // Compute zero to one for amount out computations
    #[inline]
    pub fn zero_to_one(&self, pool: &Address, token_in: Address) -> Option<bool> {
        self.pool_info.get(pool).map(|info| info.token0_address() == token_in)
    }

    // Go through a block trace and update all relevant slots
    #[inline]
    pub fn update_all_slots(
        &mut self,
        address: Address,
        account_state: GethAccountState,
    ) -> Result<()> {
        trace!(
            "Update all slots: updating all storage slots for adddress {}",
            address
        );
        let storage = account_state.storage;
        for (slot, value) in storage {
            if let Some(account) = self.accounts.get_mut(&address) {
                let new_slot_val = BlockStateDBSlot {
                    value: value.into(),
                    insertion_type: InsertionType::Custom
                };
                account.storage.insert(slot.into(), new_slot_val);
            }
        }
        Ok(())
    }

      // Insert account information into the database
      pub fn insert_account_info(
        &mut self,
        account_address: Address,
        account_info: AccountInfo,
        insertion_type: InsertionType,
    ) {
        let mut new_account = BlockStateDBAccount::new(insertion_type);
        new_account.info = account_info;
        self.accounts.insert(account_address, new_account);
    }

    // Insert storage info into the database.
    pub fn insert_account_storage(
        &mut self,
        account_address: Address,
        slot: U256,
        value: U256,
        insertion_type: InsertionType,
    ) -> Result<()> {
        // If this account already exists, just update the storage slot
        if let Some(account) = self.accounts.get_mut(&account_address) {
            // slot value is marked as custom since this is a custom insertion
            let slot_value = BlockStateDBSlot {
                value,
                insertion_type: InsertionType::Custom,
            };
            account.storage.insert(slot, slot_value);
            return Ok(());
        }

        // The account does not exist. Fetch account information from provider and insert account
        // into database
        let account = self.basic(account_address)?.unwrap();
        self.insert_account_info(account_address, account, insertion_type);

        // The account is now in the database, so fetch and insert the storage value
        let node_db_account = self.accounts.get_mut(&account_address).unwrap();
        let slot_value = BlockStateDBSlot {
            value,
            insertion_type: InsertionType::Custom,
        };
        node_db_account.storage.insert(slot, slot_value);

        Ok(())
    }

}

// Implement the database trait for the BlockStateDB
impl<N: Network, P: Provider<N>> BlockStateDB<N, P>{
    type Error = TransportError;

    // Get basic account information
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        trace!("Database Basic: Looking for account {}", address);
        // Look if we already have the account
        if let Some(account) = self.accounts.get(&address) {
            trace!("Database Basic: Account {} found in database", address);
            return Ok(Some(account.info.clone()));
        }

        // Fetch the account data if we don't have the account and insert it into database
        trace!(
            "Database Basic: Account {} not found in cache. Fetching info via basic_ref",
            address
        );
        // Fetch the account from the chain
        let account_info = Self::basic_ref(self, address)?.unwrap();
        match self.accounts.get_mut(&address) {
            Some(account) => account.info = account_info.clone(),
            None => self.insert_account_info(address, account_info.clone(), InsertionType::OnChain),
        }

        Ok(Some(account_info))
    }

    // Get account code by its hash
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        trace!(
            "Database Code By Hash: Fetching code for hash {}",
            code_hash
        );
        // Look if we already have the code
        if let Some(code) = self.contracts.get(&code_hash) {
            trace!(
                "Database Code By Hash: Code for hash {} found in database",
                code_hash
            );
            return Ok(code.clone());
        }

        trace!("Database Code By Hash: Code for hash {} not found in cache. Fetching code via code_by_hash_ref", code_hash);
        let bytecode = <Self as DatabaseRef>::code_by_hash_ref(self, code_hash);
        let bytecode = match bytecode {
            Ok(bytecode) => {
                trace!(
                    "Database Code By Hash: Code for hash {} fetched from code_by_hash_ref",
                    code_hash
                );
                bytecode
            }
            Err(_) => {
                trace!(
                    "Database Code By Hash: Unable to fetch code for hash {} from code_by_hash_ref",
                    code_hash
                );
                Bytecode::new()
            }
        };

        self.contracts.insert(code_hash, bytecode.clone());
        trace!(
            "Database Code By Hash: Inserted code for hash {} into database",
            code_hash
        );
        Ok(bytecode)
    }

    // Get storage value of address at index
    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        trace!(
            "Database Storage: Fetching storage for address {}, slot {}",
            address,
            index
        );

        // Check if the account exists
        if let Some(account) = self.accounts.get(&address) {
            if let Some(value) = account.storage.get(&index) {
                trace!(
                    "Database Storage: Storage for address {}, slot {} found in database",
                    address,
                    index
                );
                return Ok(value.value);
            }
        }

        // The account exists, but the storage slot does not, fetch it
        trace!("Database Storage: Account {} found, but slot {} missing. Fetching slot via storage_ref", address, index);
        // todo!() make sure this is valid
        let value = <Self as DatabaseRef>::storage_ref(self, address, index)?;
        match self.accounts.get_mut(&address) {
            Some(account) => {
                account.storage.insert(
                    index,
                    BlockStateDBSlot {
                        value,
                        insertion_type: InsertionType::OnChain,
                    },
                );
            }
            None => {
                let _ = Self::basic(self, address)?;
                let account = self.accounts.get_mut(&address).unwrap();
                account.storage.insert(
                    index,
                    BlockStateDBSlot {
                        value,
                        insertion_type: InsertionType::OnChain,
                    },
                );
            }
        }

        trace!(
            "Database Storage: Fetched slot {} for account {}, with value {}. Inserting storage into database",
            index,
            address,
            value
        );

        // get a mutable ref to the account and insert the storage value in
        Ok(value)
    }

    fn block_hash(&mut self, number: BlockNumber) -> Result<B256, Self::Error> {
        // todo!(), thisisnt really used but should make it better
        debug!("Fetching block hash for block number: {:?}", number);
        if let Some(hash) = self.block_hashes.get(&number) {
            debug!(
                "Block hash found in database for block number: {:?}",
                number
            );
            return Ok(*hash);
        }

        debug!(
            "Block hash not found in cache, fetching from provider for block number: {:?}",
            number
        );
        let hash = <Self as DatabaseRef>::block_hash_ref(self, number)?;
        self.block_hashes.insert(number, hash);
        Ok(hash)
    }
}

// Implement required DatabaseRef trait, read references to the database (fetch from provider)
impl<N: Network, P: Provider<N>> BlockStateDB<N, P>{
    type Error = TransportError;

    // Get basic account information
    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        trace!("Database Basic Ref: Looking for account {}", address);
        if let Some(account) = self.accounts.get(&address) {
            trace!("Database Basic Ref: Account {} found in database", address);
            return Ok(Some(account.info.clone()));
        }

        // we do not have the account, fetch from the provider
        trace!(
            "Database BasicRef: Account {} not found in cache. Fetching info from provider",
            address
        );
        let f = async {
            let nonce = self
                .provider
                .get_transaction_count(address)
                .block_id(BlockId::latest());
            let balance = self
                .provider
                .get_balance(address)
                .block_id(BlockId::latest());
            let code = self
                .provider
                .get_code_at(address)
                .block_id(BlockId::latest());
            tokio::join!(nonce, balance, code)
        };
        let (nonce, balance, code) = self.runtime.block_on(f);

        match (nonce, balance, code) {
            (Ok(nonce_val), Ok(balance_val), Ok(code_val)) => {
                trace!(
                    "Database BasicRef: Fetched account {} from provider",
                    address
                );
                let balance = balance_val;
                let code = Bytecode::new_raw(code_val.0.into());
                let code_hash = code.hash_slow();
                let nonce = nonce_val;

                Ok(Some(AccountInfo::new(balance, nonce, code_hash, code)))
            }
            _ => {
                trace!(
                    "Database BasicRef: Unable to fetch account {} from provider",
                    address
                );
                Ok(None)
            }
        }
    }

    // Get account code by its hash
    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        trace!(
            "Database Code By Hash Ref: Fetching code for hash {}",
            code_hash
        );
        // Look if we already have the code
        if let Some(code) = self.contracts.get(&code_hash) {
            trace!(
                "Database Code By Hash Ref: Code for hash {} found in cache",
                code_hash
            );
            return Ok(code.clone());
        }

        // the code should already be loaded??
        panic!("The code should already be loaded");
    }

    // Get storage value of address at index
    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        trace!(
            "Database Storage Ref: Fetching storage for address {}, slot {}",
            address,
            index
        );
        if let Some(account) = self.accounts.get(&address) {
            if let Some(value) = account.storage.get(&index) {
                trace!(
                    "Database Storage Ref: Storage for address {}, slot {} found in database",
                    address,
                    index
                );
                return Ok(value.value);
            }
        }

        trace!(
            "Database Storage Ref: Account {} not found. Fetching slot {} from provider",
            address,
            index
        );
        let f = self.provider.get_storage_at(address, index);
        let slot_val = self.runtime.block_on(f.into_future())?;
        trace!("Database Storage Ref: Fetched slot {} with value {} for account {} from provider", index, slot_val, address);
        Ok(slot_val)
    }

    fn block_hash_ref(&self, number: BlockNumber) -> Result<B256, Self::Error> {
        debug!("Fetching block_hash_ref for block number: {:?}", number);
        match self.block_hashes.get(&number) {
            Some(entry) => {
                debug!(
                    "Block hash found in cache for block number: {:?}, hash: {:?}",
                    number, entry
                );
                Ok(*entry)
            }
            None => {
                debug!(
                    "Block hash not found in cache, fetching from provider for block number: {:?}",
                    number
                );
                let block = self
                    .runtime
                    .block_on(self.provider.get_block_by_number(number.into(), false.into()))?;
                match block {
                    Some(block_data) => {
                        let hash = B256::new(*block_data.header().hash());
                        debug!(
                            "Fetched block hash from provider for block number: {:?}, hash: {:?}",
                            number, hash
                        );
                        Ok(hash)
                    }
                    None => {
                        warn!("No block found for block number: {:?}", number);
                        Ok(B256::ZERO)
                    }
                }
            }
        }
    }
}

impl<N: Network, P: Provider<N>> BlockStateDB<N, P>{
    fn commit(&mut self, changes: HashMap<Address, Account, foldhash::fast::RandomState>) {
        for (address, mut account) in changes {
            if !account.is_touched() {
                continue;
            }
            if account.is_selfdestructed() {
                let db_account = self.accounts.entry(address).or_default();
                db_account.storage.clear();
                db_account.state = AccountState::NotExisting;
                db_account.info = AccountInfo::default();
                continue;
            }
            let is_newly_created = account.is_created();

            if let Some(code) = &mut account.info.code {
                if !code.is_empty() {
                    if account.info.code_hash == KECCAK_EMPTY {
                        account.info.code_hash = code.hash_slow();
                    }
                    self.contracts
                        .entry(account.info.code_hash)
                        .or_insert_with(|| code.clone());
                }
            }

            let db_account = self.accounts.entry(address).or_default();
            db_account.info = account.info;

            db_account.state = if is_newly_created {
                db_account.storage.clear();
                AccountState::StorageCleared
            } else if db_account.state.is_storage_cleared() {
                // Preserve old account state if it already exists
                AccountState::StorageCleared
            } else {
                AccountState::Touched
            };
            db_account
                .storage
                .extend(account.storage.into_iter().map(|(key, value)| {
                    (
                        key,
                        BlockStateDBSlot {
                            value: value.present_value(),
                            insertion_type: InsertionType::Custom,
                        },
                    )
                }));
        }
    }
}

#[derive(Default, Eq, PartialEq, Clone, Debug)]
pub enum InsertionType {
    Custom,
    #[default]
    OnChain,
}

#[derive(Default, Eq, PartialEq, Clone, Debug)]
pub struct BlockStateDBSlot {
    pub value: U256,
    pub insertion_type: InsertionType,
}

#[allow(dead_code)]
#[derive(Default, Eq, PartialEq, Clone, Debug)]
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
