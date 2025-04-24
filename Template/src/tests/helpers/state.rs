#[cfg(test)]
mod state {

    use alloy::providers::ProviderBuilder;
    use super::super::utils::{load_and_filter_pools, construct_market};
    use pool_sync::PoolType;


    macro_rules! test_v2_state {
        ($test_name:ident, $pool_type:ident) => {
            #[tokio::test(flavor = "multi_thread")]
            async fn test_name() {
                dotenv::dotenv().ok();
                //setup a provider
                let provider = ProviderBuilder::new().on_http(std::env::var("FULL").unwrap().parse().unwrap());
                // load and filter pools
                let (pools, last_synced_block) = load_and_filter_pools(PoolType::$pool_type).await;
                // init a market state with the new relevant pools
                let (market, address_rx) = construct_market(pools.clone(), last_synced_block).await;
                // while we get an update (new block), test onchain and offchain for all pools
                while let Ok(Event::PoolsTouched(addresses, _)) = address_rx.recv() {
                    println!("{} touched pools", addresses.len());
                    for address in addresses {
                        let pool = pool_map.get(&address).unwrap();
                        let offchain = offchain_quote(&pool, market.clone());
                        let onchain = onchain_quote(&pool).await;
                        assert_eq!(offchain, onchain, "failed with pool {:#?}", pool);
                    }
                    println!("Iteration finished");
                }
            }
        }
    }

    test_v2_state!(test_uniswapv2_state, UniswapV2);
}
