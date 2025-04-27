use std::{
    collections::HashMap,
    path::Path,
    sync::{atomic::{AtomicU64, Ordering}, Arc, RwLock},
};
use alloy::primitives::{Address, B256, StorageKey, U256};
use eyre::{Context, Result};
use reth::api::NodeTypesWithDBAdapter;
use reth::providers::{
    providers::StaticFileProvider,
    AccountReader, BlockNumReader, DatabaseProviderFactory,
    HistoricalStateProvider, ProviderFactory, StateProviderBox, StateProviderFactory,
};
use reth::utils::open_db_read_only;
use reth_chainspec::ChainSpecBuilder;
use reth_db::{mdbx::DatabaseArguments, ClientVersion, DatabaseEnv};
use reth_node_ethereum::EthereumNode;
use revm::{
    db::{AccountState, Database, DatabaseCommit, DatabaseRef},
    primitives::{Account, AccountInfo, Bytecode, KECCAK_EMPTY},
};

/// Core struct that provides access to historical state from Reth database.
pub struct HistoryDB {
    db_provider: StateProviderBox,
    provider_factory: ProviderFactory<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>,
}

impl HistoryDB {
    /// Constructs a new HistoryDB for a given database path and block number
    pub fn new(db_path: String, block: u64) -> Result<Self> {
        let db_path = Path::new(&db_path);

        // Open the database in read-only mode
        let db = Arc::new(open_db_read_only(
            db_path.join("db"),
            DatabaseArguments::new(ClientVersion::default()),
        ).wrap_err("Failed to open DB in read-only mode")?);

        // Construct the mainnet ChainSpec
         spec = Arc::new(ChainSpecBuilder::mainnet().build());

        // Load static file provider (used for history lookups)
        let static_provider = StaticFileProvider::read_only(db_path.join("static_files"), true)
            .wrap_err("Failed to open StaticFileProvider")?;

        // Construct ProviderFactory for state access
        let factory = ProviderFactory::new(db.clone(), spec.clone(), static_provider);

        let provider = factory.history_by_block_number(block)
            .wrap_err_with(|| format!("Failed to load historical state at block {}", block))?;

        Ok(Self {
            db_provider: provider,
            provider_factory: factory,
        })
    }
}

// === revm Database Implementation ===
impl Database for HistoryDB {
    type Error = eyre::Error;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        DatabaseRef::basic_ref(self, address)
    }

    fn code_by_hash(&mut self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        panic!("code_by_hash should never be called directly; code is preloaded via basic_ref")
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        DatabaseRef::storage_ref(self, address, index)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        DatabaseRef::block_hash_ref(self, number)
    }
}

// === revm DatabaseRef Implementation ===
impl DatabaseRef for HistoryDB {
    type Error = eyre::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let account = self
            .db_provider
            .basic_account(&address)?
            .unwrap_or_default(); // default to empty account if not found

        let code = self.db_provider.account_code(&address)?;

        let account_info = match code {
            Some(code) => AccountInfo::new(
                account.balance,
                account.nonce,
                code.hash_slow(),
                Bytecode::new_raw(code.original_bytes()),
            ),
            None => AccountInfo::new(account.balance, account.nonce, KECCAK_EMPTY, Bytecode::new()),
        };

        Ok(Some(account_info))
    }

    fn code_by_hash_ref(&self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        panic!("code_by_hash_ref should not be invoked directly; preloading expected")
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let key = StorageKey::from(index);
        let value = self.db_provider.storage(address, key)?;
        Ok(value.unwrap_or_default())
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        match self.db_provider.block_hash(number)? {
            Some(hash) => Ok(B256::from(hash.0)),
            None => Ok(KECCAK_EMPTY),
        }
    }
}
