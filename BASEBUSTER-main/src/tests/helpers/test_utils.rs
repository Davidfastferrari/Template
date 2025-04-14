#[cfg(test)]
pub mod utils {
    use alloy::network::Ethereum;
    use alloy::providers::{ProviderBuilder, RootProvider};
    use alloy::sol_types::SolCall;
    use alloy::transports::http::{Client, Http};
    use pool_sync::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use alloy::sol_types::SolValue;
    use std::collections::HashMap;
    use tokio::sync::broadcast;
    use std::sync::mpsc;
    use alloy::primitives::Address;
    use revm::primitives::{address, U256, keccak256, TransactTo};
    use revm::Evm;
    use node_db::{NodeDB, InsertionType};

    use super::super::contract_gen::ERC20;
    use crate::events::Event;
    use crate::filter::filter_pools;
    use crate::market_state::MarketState;
    use crate::stream::stream_new_blocks;

    // Load in all of the pools we want to use for testing
    pub async fn load_and_filter_pools(pool_type: Vec<PoolType>) -> (Vec<Pool>, u64) {
        dotenv::dotenv().ok();
        let pool_sync = PoolSync::builder()
            .add_pools(&pool_type)
            .chain(pool_sync::Chain::Base)
            .rate_limit(1000)
            .build()
            .unwrap();
        let (pools, last_synced_block) = pool_sync.sync_pools().await.unwrap();
        let pools = filter_pools(pools, 500, Chain::Base).await;
        (pools, last_synced_block)
    }

    // map to easlier go from address to pool
    pub fn construct_pool_map(pools: Vec<Pool>) -> HashMap<Address, Pool> {
        let mut map = HashMap::new();
        for pool in pools {
            map.insert(pool.address(), pool.clone());
        }
        map
    }

    // Construct a new market from a set of pools
    pub async fn construct_market(pools: Vec<Pool>, last_synced_block: u64) -> (
        Arc<MarketState<Http<Client>, Ethereum, RootProvider<Http<Client>>>>,
        mpsc::Receiver<Event>,
    ) {
        // Create channels for communication
        let (block_sender, block_receiver) = broadcast::channel(10);
        let (address_sender, address_receiver) = mpsc::channel();

        // Setup provider
        let http_url = std::env::var("FULL").unwrap().parse().unwrap();
        let provider = ProviderBuilder::new().on_http(http_url);

        // Start the block stream
        tokio::task::spawn(stream_new_blocks(block_sender));

        let is_caught_up = Arc::new(AtomicBool::new(false));
        // Initialize market state with pools and channels
        let market_state = MarketState::init_state_and_start_stream(
            pools,
            block_receiver.resubscribe(),
            address_sender,
            last_synced_block,
            provider,
            is_caught_up.clone()
        )
        .await
        .unwrap();

        // Return the market state and address receiver instead of block receiver
        (market_state, address_receiver)
    }


    // setup an evnm instance with some weth and approve the router to spend it
    pub fn evm_with_balance_and_approval(router: Address, token: Address) -> Evm<'static, (), NodeDB> {
        // construct the db
        dotenv::dotenv().ok();
        let database_path = std::env::var("DB_PATH").unwrap();
        let mut node_db = NodeDB::new(database_path).unwrap();
    
        let account = address!("18B06aaF27d44B756FCF16Ca20C1f183EB49111f");
        let balance_slot = U256::from(3);
        // give our test account some fake WETH and ETH
        let one_ether = U256::from(1_000_000_000_000_000_000u128);
        let hashed_acc_balance_slot = keccak256((account, balance_slot).abi_encode());
        node_db
            .insert_account_storage(token, hashed_acc_balance_slot.into(), one_ether, InsertionType::OnChain)
            .unwrap();

        let mut evm = Evm::builder()
            .with_db(node_db)
            .modify_tx_env(|tx| {
                tx.caller = account;
                tx.value = U256::ZERO;
            })
            .build();

        // setup approval call and transact
        let approve_calldata = ERC20::approveCall {
            spender: router,
            amount: U256::from(10e18),
        }.abi_encode();
        evm.tx_mut().transact_to = TransactTo::Call(token);
        evm.tx_mut().data = approve_calldata.into();
        evm.transact_commit().unwrap();
        evm
    }

   

}
