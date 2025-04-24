// All offchain calculation tests
#[cfg(test)]
mod estimation {
    use super::super::helpers::test_utils::utils::{construct_market, load_and_filter_pools};
    use crate::calculation::Calculator;
    use crate::estimator::Estimator;
    use crate::events::Event;
    use crate::graph::ArbGraph;

    use alloy::primitives::address;
    use pool_sync::{Pool, PoolType, PoolInfo};

    // Manually compare swap path estimations to their calculated rate
    #[tokio::test(flavor = "multi_thread")]
    pub async fn test_path_estimations() {
        dotenv::dotenv().ok();
        // load pools and get cycles
        let (pools, last_synced_block) =
            load_and_filter_pools(vec![
                PoolType::UniswapV2,
                PoolType::PancakeSwapV2,
                PoolType::SushiSwapV2,
                PoolType::UniswapV3,
                PoolType::SushiSwapV3,
                //PoolType::PancakeSwapV3,
                PoolType::BaseSwapV2,
                PoolType::BaseSwapV3,
                //PoolType::Aerodrome,
                PoolType::Slipstream
            ]).await;
        let cycles = ArbGraph::generate_cycles(pools.clone()).await;
        println!("Generated {} cycles", cycles.len());

        // init a market state with the new relevant pools
        let (market, address_rx) = construct_market(pools.clone(), last_synced_block).await;

        // construct the calculator and estimator
        let mut estimator = Estimator::new(market.clone());
        estimator.process_pools(pools.clone());
        let calculator = Calculator::new(market.clone());

        // while we get an update (new block), test onchain and offchain for all pools
        while let Ok(Event::PoolsTouched(addresses, _)) = address_rx.recv() {
            estimator.update_rates(&addresses);
            println!("Touched {} addresses", addresses.len());
            for path in &cycles {
                let offchain_amt = calculator.calculate_output(&path.clone());
                let est_amt = estimator.estimate_output_amount(path);
                println!("offchain {:?}, estimation {:?}", offchain_amt, est_amt);
            }
        }
    }

    // Manual print based test to find out why a path may diverge from its 
    // estimated and quoted rates
    #[tokio::test(flavor = "multi_thread")]
    async fn test_calculated_to_estimated() {
        dotenv::dotenv().ok();

        // get the pools that we want to arb over
        let pool_addrs = [
            address!("b2839134b8151964f19f6f3c7d59c70ae52852f5"),
            address!("d035d4c8f848ddE156ba097fA33DF20f6068E29D"),
        ];
        let (pools, last_synced_block) = load_and_filter_pools(vec![PoolType::UniswapV2, PoolType::SushiSwapV2]).await;
        let pools: Vec<Pool> = pools.iter()
            .filter(|p| pool_addrs.contains(&p.address()))
            .cloned()
            .collect();

        // construct the market with the new pools
        let (market, _ ) = construct_market(pools.clone(), last_synced_block).await;

        // construct estimator and calculator
        let mut estimator = Estimator::new(market.clone());
        estimator.process_pools(pools.clone());
        let calculator = Calculator::new(market.clone());

        // there should be only 1 cycle 
        let cycles = ArbGraph::generate_cycles(pools.clone()).await;
        let path = cycles.first().unwrap();

        let offchain = calculator.calculate_output(&path.clone());
        let est = estimator.estimate_output_amount(path);
        println!("offchain {:?}, estimation {:?}", offchain, est);
    }
}





















