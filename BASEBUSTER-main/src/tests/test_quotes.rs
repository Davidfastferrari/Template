#[cfg(test)]
mod offchain_calculations {

    use super::super::helpers::offchain_quote::offchain_quote::offchain_quote;
    use super::super::helpers::onchain_quote::onchain::onchain_quote;
    use super::super::helpers::test_utils::utils::{
        construct_market, construct_pool_map, load_and_filter_pools,
    };
    use crate::events::Event;
    use crate::gen::{ERC20Token, FlashQuoter};
    use crate::state_db::{BlockStateDB, InsertionType};
    use alloy::primitives::{address, U256};
    use alloy::providers::ProviderBuilder;
    use alloy::sol_types::{SolCall, SolValue};
    use pool_sync::PoolType;
    use revm::primitives::keccak256;
    use revm::primitives::{AccountInfo, Bytecode, TransactTo};
    use revm::Evm;

    // Test to make sure that the quoter contract works
    #[tokio::test(flavor = "multi_thread")]
    async fn test_quoter_contract() {
        dotenv::dotenv().ok();
        // EOA, Quoter, WETH
        let account = address!("d8da6bf26964af9d7eed9e03e53415d37aa96045");
        let quoter = address!("0000000000000000000000000000000000001000");
        let weth = address!("4200000000000000000000000000000000000006");

        // how many tokens we want to insert and ERC20 balance slot
        let ten_units = U256::from(10_000_000_000_000_000_000u128);
        let balance_slot = keccak256((account, U256::from(3)).abi_encode());

        // Insert the quoter bytecode so we can make calls to it
        let quoter_bytecode = FlashQuoter::DEPLOYED_BYTECODE.clone();
        let quoter_acc_info = AccountInfo {
            nonce: 0_u64,
            balance: U256::ZERO,
            code_hash: keccak256(&quoter_bytecode),
            code: Some(Bytecode::new_raw(quoter_bytecode)),
        };

        // Build the DB
        let http_url = std::env::var("FULL").unwrap().parse().unwrap();
        let provider = ProviderBuilder::new().on_http(http_url);
        let mut db = BlockStateDB::new(provider).unwrap();

        db.insert_account_info(quoter, quoter_acc_info, InsertionType::Custom);

        // give some balance of the input token
        db.insert_account_storage(weth, balance_slot.into(), ten_units, InsertionType::OnChain)
            .unwrap();
        // approve the quoter to spend the input token
        let approve_calldata = ERC20Token::approveCall {
            spender: quoter,
            amount: U256::from(1e18),
        }
        .abi_encode();
        let mut evm = Evm::builder()
            .with_db(&mut db)
            .modify_tx_env(|tx| {
                tx.caller = account;
                tx.data = approve_calldata.into();
                tx.transact_to = TransactTo::Call(weth);
            })
            .build();
        evm.transact_commit().unwrap();

        // Setup SwapParams and do call

        // weth_usdc pool for all of the protocols we support. Test all permuatations
        let uni_v2 = (address!("88A43bbDF9D098eEC7bCEda4e2494615dfD9bB9C"), 0);
        let uni_v3 = (address!("b4CB800910B228ED3d0834cF79D697127BBB00e5"), 1);

        let pancake_v2 = (address!("79474223AEdD0339780baCcE75aBDa0BE84dcBF9"), 0);
        let pancake_v3 = (address!("B775272E537cc670C65DC852908aD47015244EaF"), 1);

        let sushi_v2 = (address!("2F8818D1B0f3e3E295440c1C0cDDf40aAA21fA87"), 0);
        let sushi_v3 = (address!("57713F7716e0b0F65ec116912F834E49805480d2"), 1);

        let slipstream = (address!("b2cc224c1c9feE385f8ad6a55b4d94E92359DC59"), 1);
        let aerodrome = address!("cDAC0d6c6C59727a65F871236188350531885C43");

        // alienbase
        let alienbase_v2 = (address!("B16D2257643fdBB32d12b9d73faB784eB4f1Bee4"), 0);
        let alienbase_v3 = (address!("0a13F0a67583dEFFf04b734c34975e6BDCF6736D"), 1);

        let pools = [
            uni_v2, uni_v3, pancake_v2, pancake_v2, sushi_v2, sushi_v3, slipstream, alienbase_v3, alienbase_v2
        ];

        for i in 0..pools.len() {
            for j in 0..pools.len() {
                if i != j {
                    let swap_params = FlashQuoter::SwapParams {
                        pools: vec![pools[i].0, pools[j].0],
                        poolVersions: vec![pools[i].1, pools[j].1],
                        amountIn: U256::from(1e16),
                    };
                    let quote_call = FlashQuoter::quoteArbitrageCall {
                        params: swap_params.clone(),
                    }
                    .abi_encode();

                    evm.tx_mut().data = quote_call.into();
                    evm.tx_mut().transact_to = TransactTo::Call(quoter);
                    let output = evm.transact().unwrap().result;
                    if !output.is_success() {
                        println!("{:#?}", swap_params);
                    }
                }
            }
        }
    }

    // Test the outputs for all pools
    macro_rules! test_pool_out {
        ($test_name:ident, $pool_type:ident) => {
            #[tokio::test(flavor = "multi_thread")]
            pub async fn $test_name() {
                dotenv::dotenv().ok();
                // load and filter pools
                let (pools, last_synced_block) =
                    load_and_filter_pools(vec![PoolType::$pool_type]).await;
                // Pool map for references
                let pool_map = construct_pool_map(pools.clone());
                // init a market state with the new relevant pools
                let (market, address_rx) = construct_market(pools.clone(), last_synced_block).await;
                // while we get an update (new block), test onchain and offchain for all pools
                while let Ok(Event::PoolsTouched(addresses, _)) = address_rx.recv() {
                    println!("{} touched pools", addresses.len());
                    for address in addresses {
                        let pool = pool_map.get(&address).unwrap();
                        // Get both and offchain and an onchain amount out, they should be the same
                        let offchain = offchain_quote(&pool, market.clone());
                        let onchain = onchain_quote(&pool).await;
                        assert_eq!(offchain, onchain, "failed with pool {:#?}", pool);
                    }
                    println!("Iteration finished");
                }
            }
        };
    }

    test_pool_out!(test_uniswapv2_out, UniswapV2);
    test_pool_out!(test_sushiswapv2_out, SushiSwapV2);
    test_pool_out!(test_pancakeswapv2_out, PancakeSwapV2);
    test_pool_out!(test_baseswapv2_out, BaseSwapV2);
    test_pool_out!(test_swapbasedv2_out, SwapBasedV2);
    test_pool_out!(test_alienbasev2_out, AlienBaseV2);
    test_pool_out!(test_dackieswapv2_out, DackieSwapV2);
    test_pool_out!(test_uniswapv3_out, UniswapV3);
    test_pool_out!(test_sushiswapv3_out, SushiSwapV3);
    test_pool_out!(test_pancakeswapv3_out, PancakeSwapV3);
    test_pool_out!(test_alienbasev3_out, AlienBaseV3);
    test_pool_out!(test_dackieswapv3_out, DackieSwapV3);
    test_pool_out!(test_swapbasedv3_out, SwapBasedV3);
    test_pool_out!(test_baseswapv3_out, BaseSwapV3);
    test_pool_out!(test_slipstream_out, Slipstream);
    test_pool_out!(test_aerodrome_out, Aerodrome);
}
