use super::BlockStateDB;
use alloy::{
    consensus::Transaction,
    network::{ TransactionBuilder, EthereumWallet, Ethereum, Network, BlockResponse, HeaderResponse },
    primitives::{ BlockNumber, B256, I256, hex, address, U256, U160, Address, FixedBytes, Bytes,keccak256 },
    providers::{ Provider, ProviderBuilder, RootProvider },
      rpc::client::RpcClient,
    transports::http::{
        reqwest::{
            header::{HeaderMap, HeaderValue, AUTHORIZATION},
            Client,
        },
        Http,
      Transport,
      TransportError 
    },
 sol,
 sol_types::{ SolCall, SolValue, SolType },
};
use lazy_static::lazy_static;
use log::trace;
use pool_sync::{Pool, PoolInfo};
use revm::DatabaseRef;
use zerocopy::IntoBytes;
use crate::state_db::blockstate_db::{InsertionType, BlockStateDBSlot};

lazy_static {
    static ref U112_MASK: U256 = (U256::from(1) << 112) - U256::from(1);
}

/// uniswapv2 db read/write related methods
impl<T, N, P> BlockStateDB<T, N, P>
where
    T: Transport + Clone,
    N: Network,
    P: Provider<T, N>,
{
    // insert a new uniswapv2 pool into the database
    pub fn insert_v2(&mut self, pool: Pool) {
        trace!("Adding new v2 pool {}", pool.address());
        let address = pool.address();
        let token0 = pool.token0_address();
        let token1 = pool.token1_address();

        // track the pool
        self.add_pool(pool.clone());


        // get v2 info
        let v2_pool = pool.get_v2().unwrap();
        let reserve0 = U256::from(v2_pool.token0_reserves);
        let reserve1 = U256::from(v2_pool.token1_reserves);


        // create account and insert storage values
        self.insert_reserves(address, reserve0, reserve1);
        self.insert_token0(address, token0);
        self.insert_token1(address, token1);
    }

    // Function to retrieve V2 Pool state
    #[inline]
    pub fn get_reserves(&self, pool: &Address) -> (U256, U256) {
        let value = self.storage_ref(*pool, U256::from(8)).unwrap();
        ((value >> 0) & *U112_MASK, (value >> (112)) & *U112_MASK)
    }

    // get token 0
    pub fn get_token0(&self, pool: Address) -> Address {
        let token0 = self.storage_ref(pool, U256::from(6)).unwrap();
        Address::from_word(token0.into())
    }

    #[warn(dead_code)]
    pub fn get_token1(&self, pool: Address) -> Address {
        let token1 = self.storage_ref(pool, U256::from(7)).unwrap();
        Address::from_word(token1.into())
    }

    #[warn(dead_code)]
    pub fn get_tokens(&self, _pool: &Address) -> (Address, Address) {
        todo!()
    }

    // Functions to insert v2 pool state

    // insert pool reserves into the database
    fn insert_reserves(&mut self, pool: Address, reserve0: U256, reserve1: U256) {
        let packed_reserves = (reserve1 << 112) | reserve0;
        trace!("V2 Database: Inserting reserves for {}", pool);
        let account = self.accounts.get_mut(&pool).unwrap();
        let new_db_slot = BlockStateDBSlot {
            value: packed_reserves,
            insertion_type: InsertionType::Custom
        };
        account.storage.insert(U256::from(8), new_db_slot);
    }

    // insert token0 into the database
    fn insert_token0(&mut self, pool: Address, token: Address) {
        let mut bytes = [0u8; 32];
        bytes[12..].copy_from_slice(token.as_bytes());
        trace!("V2 Database: Inserting token 0 for {}", pool);
        let account = self.accounts.get_mut(&pool).unwrap();
        let new_db_slot = BlockStateDBSlot {
            value: U256::from_be_bytes(bytes),
            insertion_type: InsertionType::Custom
        };
        account
            .storage
            .insert(U256::from(6), new_db_slot);
    }

    // insert token1 into the database
    fn insert_token1(&mut self, pool: Address, token: Address) {
        let mut bytes = [0u8; 32];
        bytes[12..].copy_from_slice(token.as_bytes());
        trace!("V2 Database: Inserting token 1 for {}", pool);
        let account = self.accounts.get_mut(&pool).unwrap();
        let new_db_slot = BlockStateDBSlot {
            value: U256::from_be_bytes(bytes),
            insertion_type: InsertionType::Custom
        };
        account
            .storage
            .insert(U256::from(7), new_db_slot);
    }
}

#[cfg(test)]
mod test_db_v2 {
    use super::*;
    use alloy::network::Ethereum;
    use alloy::primitives::address;
    use alloy::providers::ProviderBuilder;
    use alloy::providers::RootProvider;
    use alloy::sol;
    use alloy::sol_types::{SolCall, SolValue};
    use alloy::transports::http::{Client, Http};
    use node_db::{NodeDB, InsertionType};
    use revm::primitives::{AccountInfo, Bytecode, TransactTo};
    use revm::db::{AlloyDB, CacheDB};
    use crate::gen::FlashQuoter::{self, SwapStep};
    use log::LevelFilter;
    use revm::primitives::keccak256;
    use pool_sync::UniswapV2Pool;

    use revm::Evm;

    /* 
    type QuoteEvm<'a> = Evm<
        'a,
        EthereumWiring<
            &'a mut BlockStateDB<Http<Client>, Ethereum, RootProvider<Http<Client>>>,
            (),
        >,
    >;
    */
    type AlloyCacheDB = CacheDB<AlloyDB<Http<Client>, Ethereum, RootProvider<Http<Client>>>>;


    fn uni_v2_weth_usdc() -> UniswapV2Pool {
        UniswapV2Pool {
            address: address!("88A43bbDF9D098eEC7bCEda4e2494615dfD9bB9C"),
            token0: address!("4200000000000000000000000000000000000006"),
            token1: address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
            token0_name: "WETH".to_string(),
            token1_name: "USDC".to_string(),
            token0_decimals: 18,
            token1_decimals: 6,
            token0_reserves: U256::from(409844320018255314839_u128),
            token1_reserves: U256::from(94038111875_u128),
            stable: None,
            fee: None,
        }
    }

    fn sushi_v2_weth_usdc() -> UniswapV2Pool {
        UniswapV2Pool {
            address: address!("2F8818D1B0f3e3E295440c1C0cDDf40aAA21fA87"),
            token0: address!("4200000000000000000000000000000000000006"),
            token1: address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
            token0_name: "WETH".to_string(),
            token1_name: "USDC".to_string(),
            token0_decimals: 18,
            token1_decimals: 6,
            token0_reserves: U256::from(678659086524752413_u128),
            token1_reserves: U256::from(1649070661),
            stable: None,
            fee: None,
        }
    }

    // Make sure we can insert a pool and get the proper pool addresses
    // and reserves back
    #[tokio::test(flavor = "multi_thread")]
    pub async fn test_insert_pool_and_retrieve() {
        env_logger::Builder::new()
            .filter_module("BaseBuster", LevelFilter::Info)
            .init();
        dotenv::dotenv().ok();

        let url = std::env::var("FULL").unwrap().parse().unwrap();
        let provider = ProviderBuilder::new().on_http(url);
        let mut db = BlockStateDB::new(provider).unwrap();

        let pool_addr = address!("B4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc");
        let token0 = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let token1 = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");

        let pool = uni_v2_weth_usdc();
        db.insert_v2(Pool::UniswapV2(pool));
        db.insert_reserves(pool_addr, U256::from(10), U256::from(20));
        let (res0, res1) = db.get_reserves(&pool_addr);

        assert_eq!(db.get_token0(pool_addr), token0, "");
        assert_eq!(db.get_token1(pool_addr), token1);
        assert_eq!(res0, U256::from(10));
        assert_eq!(res1, U256::from(20));
    }

    // Make sure out BlockStateDB can fetch the state
    #[tokio::test(flavor = "multi_thread")]
    pub async fn test_fetch_pool_data() {
        dotenv::dotenv().ok();
        env_logger::Builder::new()
            .filter_level(LevelFilter::Trace) // or Info, Warn, etc.
            .init();
        let url = std::env::var("FULL").unwrap().parse().unwrap();
        let provider = ProviderBuilder::new().on_http(url);
        let db = BlockStateDB::new(provider).unwrap();

        let pool_addr = address!("88A43bbDF9D098eEC7bCEda4e2494615dfD9bB9C");
        let expected_token0 = address!("4200000000000000000000000000000000000006");
        let expected_token1 = address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");

        // Fetch and assert token addresses
        let fetched_token1 = db.get_token1(pool_addr);
        let fetched_token0 = db.get_token0(pool_addr);
        assert_eq!(
            fetched_token0,
            expected_token0,
            "Token0 address mismatch"
        );
        assert_eq!(
            fetched_token1,
            expected_token1,
            "Token1 address mismatch"
        );

        // Fetch reserves
        let (reserve0, reserve1) = db.get_reserves(&pool_addr);
        assert!(reserve0 > U256::ZERO, "Reserve0 should be non-zero");
        assert!(reserve1 > U256::ZERO, "Reserve1 should be non-zero");

        println!(
            "Fetched reserves: reserve0 = {}, reserve1 = {}",
            reserve0, reserve1
        );
    }


    // try a get amounts out call 
    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_amounts_out() {
        sol!(
            #[sol(rpc)]
            contract Uniswap {
                function getAmountsOut(uint amountIn, address[] memory path) external view returns (uint[] memory amounts);
            }
        );

        dotenv::dotenv().ok();
        env_logger::Builder::new()
            .filter_level(LevelFilter::Trace) // or Info, Warn, etc.
            .init();
        let url = std::env::var("FULL").unwrap().parse().unwrap();
        let provider = ProviderBuilder::new().on_http(url);
        let db = BlockStateDB::new(provider.clone()).unwrap();

        let weth = address!("4200000000000000000000000000000000000006");
        let usdc = address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");

        let amount_in = U256::from(1000000000); // 1 USDC (6 decimals)
        let calldata = Uniswap::getAmountsOutCall {
            amountIn: amount_in,
            path: vec![weth, usdc],
        }.abi_encode();

        // Create EVM instance
        let mut evm = Evm::builder()
        .with_db(db)
        .modify_tx_env(|tx| {
            tx.caller = address!("0000000000000000000000000000000000000001");
            tx.transact_to = TransactTo::Call(address!("4752ba5DBc23f44D87826276BF6Fd6b1C372aD24"));
            tx.data = calldata.into();
            tx.value = U256::ZERO;
        })
        .build();

        let ref_tx = evm.transact().unwrap();
        let result = ref_tx.result;
        let output = result.output().unwrap();
        let decoded_outputs = <Vec<U256>>::abi_decode(output, false).unwrap();
        println!("Output {:?}", decoded_outputs);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_flashquote_offchain_pool() {
        // initiation
        dotenv::dotenv().ok();
        env_logger::Builder::new()
            .filter_module("BaseBuster", LevelFilter::Trace)
            .init();
        sol!(
            #[derive(Debug)]
            contract Approval {
                function approve(address spender, uint256 amount) external returns (bool);
                function deposit(uint256 amount) external;
            }
        );

        // Accounts
        let account = address!("0250f06fc76297Fe28D0981F0566F1c0445B3cFE");
        let weth = address!("4200000000000000000000000000000000000006");
        let quoter: Address = address!("0000000000000000000000000000000000001000");

        dotenv::dotenv().ok();
        let database_path = std::env::var("DB_PATH").unwrap();
        let mut db = NodeDB::new(database_path).unwrap();

        let provider = ProviderBuilder::new().on_http(url);
        let mut db = BlockStateDB::new(provider.clone()).unwrap();

        //insert some pools 
        let uni_pool = uni_v2_weth_usdc();
        db.insert_v2(uni_pool);
        let sushi_pool = sushi_v2_weth_usdc();
        db.insert_v2(sushi_pool);

        // give the account some weth
        let one_ether = U256::from(1_000_000_000_000_000_000u128);
        let hashed_acc_balance_slot = keccak256((account, U256::from(3)).abi_encode());
        let _ = db.insert_account_storage(
            weth,
            hashed_acc_balance_slot.into(),
            one_ether,
            InsertionType::OnChain
        );

        // insert the quoter bytecode
        let quoter_bytecode = FlashQuoter::DEPLOYED_BYTECODE.clone();
        let quoter_acc_info = AccountInfo {
            nonce: 0_u64,
            balance: U256::ZERO,
            code_hash: keccak256(&quoter_bytecode),
            code: Some(Bytecode::new_raw(quoter_bytecode)),
        };
        db.insert_account_info(quoter, quoter_acc_info, InsertionType::Custom);

        let mut evm = Evm::builder()
            .with_db(&mut db)
            .modify_tx_env(|tx| {
                tx.caller = account;
            })
            .build();

        // approve quoter to spend the eth
        let approve_calldata = Approval::approveCall {
            spender: quoter,
            amount: U256::from(1e18),
        }
        .abi_encode();
        evm.tx_mut().data = approve_calldata.into();
        evm.tx_mut().transact_to = TransactTo::Call(weth);
        evm.transact_commit().unwrap();

        // drop the evm so we can access the db again
        drop(evm);

        // setup call address for quotes
        //let mut db = RwLock::new(db);
        //evm.tx_mut().transact_to = TransactTo::Call(quoter);
        let quote_path = vec![
            SwapStep {
                poolAddress: address!("377feeed4820b3b28d1ab429509e7a0789824fca"),
                tokenIn: address!("4200000000000000000000000000000000000006"),
                tokenOut: address!("9a26f5433671751c3276a065f57e5a02d2817973"),
                protocol: 0,
                fee: 0.try_into().unwrap(),
            },
            SwapStep {
                poolAddress: address!("55a49e01d4f7fde54a1d0522bc0b907687ed38dd"),
                tokenIn: address!("9a26f5433671751c3276a065f57e5a02d2817973"),
                tokenOut: address!("4ed4e862860bed51a9570b96d89af5e1b0efefed"),
                protocol: 0,
                fee: 0.try_into().unwrap(),
            },
        ];

        let quote_calldata = FlashQuoter::quoteArbitrageCall {
            steps: quote_path,
            amount: U256::from(1e14),
        }
        .abi_encode();

        let mut evm = Evm::builder()
            .with_db(&mut db)
            .build();
        evm.tx_mut().caller = account;
        evm.tx_mut().transact_to =
        TransactTo::Call(address!("0000000000000000000000000000000000001000"));
        evm.tx_mut().data = quote_calldata.into();


        // transact
        let ref_tx = evm.transact().unwrap();
        let result = ref_tx.result;
        println!("{:?}", result);
    }


}
