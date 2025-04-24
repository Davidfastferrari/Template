use alloy::primitives::{Address, B256, StorageKey, U256};
use eyre::Result;
use reth::api::NodeTypesWithDBAdapter;
use reth::providers::{
    providers::StaticFileProvider,
    AccountReader,
    DatabaseProviderFactory,
    StateProviderFactory,
    HistoricalStateProvider,
    StateProviderBox,
    BlockNumReader,
    ProviderFactory
};
use reth::utils::open_db_read_only;
use reth_chainspec::ChainSpecBuilder;
use reth_db::{mdbx::DatabaseArguments, ClientVersion, DatabaseEnv};
use reth_node_ethereum::EthereumNode;
use revm::{Database, DatabaseCommit, DatabaseRef};
use revm::db::AccountState;
use revm::primitives::{Account, AccountInfo, Bytecode, KECCAK_EMPTY};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicU64, Ordering};


// Main structure for the Node Database
pub struct HistoryDB {
    db_provider: StateProviderBox,
    provider_factory: ProviderFactory<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>,
}

impl HistoryDB {
    // Constructor for NodeDB
    pub fn new(db_path: String, block: u64) -> Result<Self> {
        // Open the database in read-only mode
        let db_path = Path::new(&db_path);
        let db = Arc::new(open_db_read_only(
            db_path.join("db").as_path(),
            DatabaseArguments::new(ClientVersion::default()),
        )?);

        // Create a ProviderFactory
        let spec = Arc::new(ChainSpecBuilder::mainnet().build());
        let factory =
            ProviderFactory::<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>::new(
                db.clone(),
                spec.clone(),
                StaticFileProvider::read_only(db_path.join("static_files"), true)?,
            );


        let provider = factory.history_by_block_number(block).expect("Unable to create provider");


        Ok(Self {
            db_provider: provider,
            provider_factory: factory
        })
    }
}


impl Database for HistoryDB {
    type Error = eyre::Error;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Self::basic_ref(self, address)
    }

    fn code_by_hash(&mut self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        panic!("This should not be called, as the code is already loaded");
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Self::storage_ref(self, address, index)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        Self::block_hash_ref(self, number)
    }
}

impl DatabaseRef for HistoryDB {
    type Error = eyre::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let account = self.db_provider.basic_account(&address).unwrap_or_default().unwrap_or_default();
        let code = self.db_provider.account_code(&address).unwrap_or_default();
        let account_info = if let Some(code) = code {
            AccountInfo::new(
                account.balance,
                account.nonce,
                code.hash_slow(),
                Bytecode::new_raw(code.original_bytes()),
            )
        } else {
            AccountInfo::new(
                account.balance,
                account.nonce,
                KECCAK_EMPTY,
                Bytecode::new(),
            )
        };
        Ok(Some(account_info))
    }

    fn code_by_hash_ref(&self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        panic!("This should not be called, as the code is already loaded");
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let value = self.db_provider.storage(address, StorageKey::from(index))?;

        Ok(value.unwrap_or_default())
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let blockhash = self.db_provider.block_hash(number).unwrap_or_default();

        if let Some(hash) = blockhash {
            Ok(B256::new(hash.0))
        } else {
            Ok(KECCAK_EMPTY)
        }
    }
}
