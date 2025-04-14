#[cfg(test)]
pub mod onchain {
    use alloy::primitives::{address, U160, U256};
    use alloy::providers::{ProviderBuilder, RootProvider};
    use alloy::sol_types::{SolCall, SolValue};
    use alloy::transports::http::{Client, Http};
    use pool_sync::{Pool, PoolInfo, PoolType, UniswapV2Pool, UniswapV3Pool};
    use revm::primitives::TransactTo;

    use super::super::contract_gen::*;
    use super::super::test_utils::utils::evm_with_balance_and_approval;
    use crate::AMOUNT;

    type ProviderType = RootProvider<Http<Client>>;

    // Call the onchain quoter contract to get a quote
    pub async fn onchain_quote(pool: &Pool) -> U256 {
        dotenv::dotenv().ok();
        // construct a provider
        let provider =
            ProviderBuilder::new().on_http(std::env::var("FULL").unwrap().parse().unwrap());

        let pool_type = pool.pool_type();

        match pool.pool_type() {
            PoolType::UniswapV2
            | PoolType::SushiSwapV2
            | PoolType::PancakeSwapV2
            | PoolType::AlienBaseV2
            | PoolType::BaseSwapV2
            | PoolType::SwapBasedV2
            | PoolType::DackieSwapV2
            | PoolType::Aerodrome => {
                let pool = pool.get_v2().unwrap();
                onchain_v2(pool, pool_type, provider).await
            }
            PoolType::UniswapV3
            | PoolType::SushiSwapV3
            | PoolType::PancakeSwapV3
            | PoolType::Slipstream => {
                let pool = pool.get_v3().unwrap();
                onchain_v3(pool, pool_type, provider).await
            }
            _ => todo!(),
        }
    }

    // Quote the amount out for V2 Pool
    async fn onchain_v2(pool: &UniswapV2Pool, pool_type: PoolType, provider: ProviderType) -> U256 {
        // Get the router address
        let address = match pool_type {
            PoolType::UniswapV2 => address!("4752ba5dbc23f44d87826276bf6fd6b1c372ad24"),
            PoolType::SushiSwapV2 => address!("6BDED42c6DA8FBf0d2bA55B2fa120C5e0c8D7891"),
            PoolType::PancakeSwapV2 => address!("8cFe327CEc66d1C090Dd72bd0FF11d690C33a2Eb"),
            PoolType::BaseSwapV2 => address!("327Df1E6de05895d2ab08513aaDD9313Fe505d86"),
            PoolType::SwapBasedV2 => address!("aaa3b1F1bd7BCc97fD1917c18ADE665C5D31F066"),
            PoolType::DackieSwapV2 => address!("Ca4EAa32E7081b0c4Ba47e2bDF9B7163907Fe56f"),
            PoolType::AlienBaseV2 => address!("8c1A3cF8f83074169FE5D7aD50B978e1cD6b37c7"),
            _ => panic!("will not reach here"),
        };

        if pool_type == PoolType::Aerodrome {
            let contract = Aerodrome::new(pool.address, provider);
            let Aerodrome::getAmountOutReturn { _0: amount_out } = contract
                .getAmountOut(*AMOUNT, pool.token0)
                .call()
                .await
                .unwrap();
            amount_out
        } else {
            let v2_router = V2Router::new(address, provider);
            let V2Router::getAmountsOutReturn { amounts } = v2_router
                .getAmountsOut(*AMOUNT, vec![pool.token0, pool.token1])
                .call()
                .await
                .unwrap();
            *amounts.last().unwrap()
        }
    }

    // Quote the amount out for V3 Pool
    async fn onchain_v3(pool: &UniswapV3Pool, pool_type: PoolType, provider: ProviderType) -> U256 {
        // Get the quoter address
        let address = match pool_type {
            PoolType::UniswapV3 => address!("3d4e44Eb1374240CE5F1B871ab261CD16335B76a"),
            PoolType::PancakeSwapV3 => address!("B048Bbc1Ee6b733FFfCFb9e9CeF7375518e25997"),
            PoolType::SushiSwapV3 => address!("b1E835Dc2785b52265711e17fCCb0fd018226a6e"),
            PoolType::Slipstream => address!("254cF9E1E6e233aa1AC962CB9B05b2cfeAaE15b0"),
            PoolType::DackieSwapV3 => address!("195FBc5B8Fbd5Ac739C1BA57D4Ef6D5a704F34f7"),
            PoolType::SwapBasedV3 => address!("756C6BbDd915202adac7beBB1c6C89aC0886503f"),
            PoolType::AlienBaseV3 => address!("B20C411FC84FBB27e78608C24d0056D974ea9411"),
            PoolType::BaseSwapV3 => address!("1B8eea9315bE495187D873DA7773a874545D9D48"),
            _ => panic!("Invalid pool type"),
        };

        if pool_type == PoolType::Slipstream {
            // Query the tickSpacing from the pool
            let pool_contract = SlipstreamPool::new(pool.address, provider.clone());
            let SlipstreamPool::tickSpacingReturn { _0: tick_spacing } =
                pool_contract.tickSpacing().call().await.unwrap();

            let params = V3QuoterSlipstream::QuoteExactInputSingleParams {
                tokenIn: pool.token0,
                tokenOut: pool.token1,
                tickSpacing: tick_spacing.try_into().unwrap(),
                amountIn: *AMOUNT,
                sqrtPriceLimitX96: U160::ZERO,
            };
            let contract = V3QuoterSlipstream::new(address, provider.clone());
            let V3QuoterSlipstream::quoteExactInputSingleReturn { amountOut, .. } =
                contract.quoteExactInputSingle(params).call().await.unwrap();
            return amountOut;
        } else if pool_type == PoolType::PancakeSwapV3
            || pool_type == PoolType::UniswapV3
            || pool_type == PoolType::SushiSwapV3
        {
            let params = V3Quoter::QuoteExactInputSingleParams {
                tokenIn: pool.token0,
                tokenOut: pool.token1,
                fee: pool.fee.try_into().unwrap(),
                amountIn: *AMOUNT,
                sqrtPriceLimitX96: U160::ZERO,
            };
            let contract = V3Quoter::new(address, provider.clone());
            let V3Quoter::quoteExactInputSingleReturn { amountOut, .. } =
                contract.quoteExactInputSingle(params).call().await.unwrap();
            return amountOut;
        } else {
            let mut evm = evm_with_balance_and_approval(address, pool.token0);

            // generate the calldata
            let calldata = match pool_type {
                PoolType::BaseSwapV3 | PoolType::SwapBasedV3 => {
                    let params = RouterDeadline::ExactInputSingleParams {
                        tokenIn: pool.token0,
                        tokenOut: pool.token1,
                        fee: pool.fee.try_into().unwrap(),
                        recipient: address!("18B06aaF27d44B756FCF16Ca20C1f183EB49111f"),
                        amountIn: *AMOUNT,
                        deadline: U256::MAX,
                        amountOutMinimum: U256::ZERO,
                        sqrtPriceLimitX96: U160::ZERO,
                    };
                    RouterDeadline::exactInputSingleCall { params }.abi_encode()
                }
                PoolType::AlienBaseV3 | PoolType::DackieSwapV3 => {
                    let params = Router::ExactInputSingleParams {
                        tokenIn: pool.token0,
                        tokenOut: pool.token1,
                        fee: pool.fee.try_into().unwrap(),
                        recipient: address!("18B06aaF27d44B756FCF16Ca20C1f183EB49111f"),
                        amountIn: *AMOUNT,
                        amountOutMinimum: U256::ZERO,
                        sqrtPriceLimitX96: U160::ZERO,
                    };
                    Router::exactInputSingleCall { params }.abi_encode()
                }
                PoolType::Slipstream => {
                    todo!()
                }
                _ => panic!("Will not reach here"),
            };

            evm.tx_mut().data = calldata.into();
            evm.tx_mut().transact_to = TransactTo::Call(address);
            let ref_tx = evm.transact().unwrap();
            let result = ref_tx.result;
            let output = result.output().unwrap();
            <U256>::abi_decode(output, false).unwrap()
        }
    }
}
