use alloy::{
    eips::{BlockId, Encodable2718 },
    consensus::Transaction,
    network::{ TransactionBuilder, EthereumWallet, Ethereum, Network }
    primitives::{ hex, address, U256, Address, FixedBytes, Bytes },
    providers::{ Provider, ProviderBuilder, RootProvider },
    rpc::types::{ TransactionRequest, BlockNumberOrTag },
    rpc::types::{
        trace::geth::{ GethDebugTracingCallOptions, Bundle, StateContext, TransactionRequest, GethTrace, GethDebugTracerType, GethDebugBuiltInTracerType, PreStateConfig, GethDebugTracingOptions, GethDefaultTracingOptions, PreStateFrame, AccountState
    },
   signer::local::PrivateKeySigner,
   signer::k256::SecretKey,
      rpc::client::RpcClient,
    transports::http::{
        reqwest::{
            header::{HeaderMap, HeaderValue, AUTHORIZATION},
            Client,
        },
        Http,
    },
   sol_types::sol;
};
use anyhow::Result;
use log::{debug, error, info};
use pool_sync::Pool;
use pool_sync::PoolInfo;
use revm::primitives::keccak256;
use revm::primitives::{AccountInfo, Bytecode, TransactTo};
use revm::Evm;
use std::collections::HashSet;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Instant;
use tokio::sync::broadcast::Receiver;
use std::sync::atomic::Ordering;
use std::sync::atomic::AtomicBool;

use events::Event;
use gen::ERC20Token;
use gen::FlashQuoter;
use state_db::{BlockStateDB, InsertionType};
use tracing::debug_trace_block;
use main::AMOUNT;

// Internal representation of the current state of the blockchain
pub struct MarketState<T, N, P>
where
    T: Transport + Clone,
    N: Network,
    P: Provider<T, N>,
{
    pub db: RwLock<BlockStateDB<T, N, P>>,
}

impl<T, N, P> MarketState<T, N, P>
where
    T: Transport + Clone,
    N: Network + Clone,
    P: Provider<T, N> + 'static + Clone,
{
    // constuct the market state with a populated db
    pub async fn init_state_and_start_stream(
        pools: Vec<Pool>,          // the pools we are searching over
        block_rx: Receiver<Event>, // receiver for new blocks
        address_tx: Sender<Event>, // sender for touched addresses in a block
        last_synced_block: u64,    // the last block that was synced too
        provider: P,
        caught_up: Arc<AtomicBool>
    ) -> Result<Arc<Self>> {
        debug!("Populating the db with {} pools", pools.len());

        // construct, warm up, and populate the db
        let mut db = BlockStateDB::new(provider).unwrap();
        Self::warm_up_database(&pools, &mut db);
        Self::populate_db_with_pools(pools.clone(), &mut db);

        // init the market state with the db
        let market_state = Arc::new(Self {
            db: RwLock::new(db),
        });

        // start the state updater
        tokio::spawn(Self::state_updater(
            market_state.clone(),
            block_rx,
            address_tx,
            last_synced_block,
            caught_up,
        ));

        Ok(market_state)
    }

    // task to retrieve new blockchain state and update our db
    async fn state_updater(
        self: Arc<Self>,
        mut block_rx: Receiver<Event>,
        address_tx: Sender<Event>,
        mut last_synced_block: u64,
        caught_up: Arc<AtomicBool>
    ) {
        // setup a provider for tracing
        let http_url = std::env::var("FULL").unwrap().parse().unwrap();
        let http = Arc::new(ProviderBuilder::new().on_http(http_url));

        // fast block times mean we can fall behind while initializing
        // catch up to the head to we are not missing any state
        let mut current_block = http.get_block_number().await.unwrap();

        while last_synced_block < current_block {
            debug!(
                "Catching up. Last synced block {}, Current block {}",
                last_synced_block, current_block
            );
            for block_num in (last_synced_block + 1)..=current_block {
                debug!("Processing block {block_num}");
                let _ = self.update_state(http.clone(), block_num).await;
            }
            last_synced_block = current_block;
            current_block = http.get_block_number().await.unwrap();
        }

        // signal that we are caught up
        caught_up.store(true, Ordering::Relaxed);

        // stream in new blocks
        while let Ok(Event::NewBlock(block_header)) = block_rx.recv().await {
            let start = Instant::now();
            let block_number = block_header.inner.number;

            // make sure we dont reprocess blocks we caught up with
            if block_number <= last_synced_block {
                debug!("Already processed block {}. Skipping", block_number);
                continue;
            }
            info!("Got new block: {block_number}");

            // update the state and get the list of updated pools
            debug!("Processing block {block_number}");
            let updated_pools = self.update_state(http.clone(), block_number).await;
            debug!("Processed the block {block_number}");

            // send the updated pools
            info!(
                "Block processed {} updates and sent in {:?}",
                updated_pools.len(),
                start.elapsed()
            );
            if let Err(e) = address_tx.send(Event::PoolsTouched(updated_pools, block_number)) {
                error!("Failed to send updated pools: {}", e);
            } else {
                debug!("Sent updated addresses for block {}", block_number);
            }

            last_synced_block = block_number;
        }
    }

    // after getting a new block, update our market state
    async fn update_state(
        &self,
        provider: Arc<RootProvider<Http<Client>>>,
        block_num: u64,
    ) -> HashSet<Address> {
        // all of the pools that were updated in this block
        let mut updated_pools: HashSet<Address> = HashSet::new();

        // trace the block to get all post state changes
        let updates = debug_trace_block(provider, BlockNumberOrTag::Number(block_num), true).await;

        // aquire write access so we can update the db and go over all updates
        let mut db = self.db.write().unwrap();
        for (address, account_state) in updates.iter().flat_map(|btree_map| btree_map.iter()) {
            if db.tracking_pool(address) {
                debug!("Updating state for pool {address}");
                db.update_all_slots(*address, account_state.clone())
                    .unwrap();
                updated_pools.insert(*address);
            }
        }

        updated_pools
    }

    // Insert pool information into the database
    fn populate_db_with_pools(pools: Vec<Pool>, db: &mut BlockStateDB<T, N, P>) {
        for pool in pools {
            if pool.is_v2() {
                db.insert_v2(pool);
            } else if pool.is_v3() {
                db.insert_v3(pool).unwrap();
            }
        }
    }

    // this function will insert any approvals/balances we need and also
    // fetch extraneous contracts/values needed for simulation swaps and
    // insert into the db
    fn warm_up_database(pools: &Vec<Pool>, db: &mut BlockStateDB<T, N, P>) {
        // state addresses
        let account = address!("d8da6bf26964af9d7eed9e03e53415d37aa96045");
        let quoter: Address = address!("0000000000000000000000000000000000001000");

        // how many tokens we want to insert and ERC20 balance slot
        let ten_units = U256::from(10_000_000_000_000_000_000u128);
        let balance_slot = keccak256((account, U256::from(3)).abi_encode());

        // insert the quoter bytecode so we can make calles to it
        let quoter_bytecode = FlashQuoter::DEPLOYED_BYTECODE.clone();
        let quoter_acc_info = AccountInfo {
            nonce: 0_u64,
            balance: U256::ZERO,
            code_hash: keccak256(&quoter_bytecode),
            code: Some(Bytecode::new_raw(quoter_bytecode)),
        };
        db.insert_account_info(quoter, quoter_acc_info, InsertionType::Custom);

        // go over all the pools and try to simulate a swap.
        // we have already filtered all of these pools, so we can assume
        // that these are good to go and load up db with info
        for pool in pools {
            // give some balance of the input token
            db.insert_account_storage(
                pool.token0_address(),
                balance_slot.into(),
                ten_units,
                InsertionType::OnChain,
            )
            .unwrap();

            // approve the quoter to spend the input token
            let approve_calldata = ERC20Token::approveCall {
                spender: quoter,
                amount: U256::from(1e18),
            }
            .abi_encode();
            let mut evm = Evm::builder()
                .with_db(&mut *db)
                .modify_tx_env(|tx| {
                    tx.caller = account;
                    tx.data = approve_calldata.into();
                    tx.transact_to = TransactTo::Call(pool.token0_address());
                })
                .build();
            evm.transact_commit().unwrap();

            // Try to do the swap from input to output token
            let is_v3 = if pool.is_v3() {
                1
            } else {
                0
            };

            let quote_path = FlashQuoter::SwapParams {
                pools: vec![pool.address()],
                poolVersions: vec![is_v3],
                amountIn: *AMOUNT
            };

            let quote_calldata = FlashQuoter::quoteArbitrageCall {
                params: quote_path
            }
            .abi_encode()
            .abi_encode();
            evm.tx_mut().data = quote_calldata.into();
            evm.tx_mut().transact_to = TransactTo::Call(quoter);

            // transact
            evm.transact().unwrap();
        }
    }
}
