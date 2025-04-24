#[cfg(test)]
pub mod offchain_quote {
    use alloy::network::Ethereum;
    use pool_sync::{Pool, PoolInfo};
    use alloy::providers::RootProvider;
    use alloy::primitives::U256;
    use alloy::transports::http::{Client, Http};
    use std::sync::Arc;
    use std::time::Instant;

    use crate::market_state::MarketState;
    use crate::calculation::Calculator;
    use crate::AMOUNT;

    type Market = Arc<MarketState<Http<Client>, Ethereum, RootProvider<Http<Client>>>>;

    // Calcualte the output amount via offchain infra
    pub fn offchain_quote(pool: &Pool, market: Market) -> U256 {
        let calculator = Calculator::new(market);
        let res = calculator.compute_amount_out(
            *AMOUNT,
            pool.address(),
            pool.token0_address(),
            pool.pool_type(),
            pool.fee()
        );
        res
    }
}