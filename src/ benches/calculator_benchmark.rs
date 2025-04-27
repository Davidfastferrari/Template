use criterion::{criterion_group, criterion_main, Criterion};
use std::sync::Arc;
use alloy::primitives::{address, U256};
use pool_sync::{Pool, PoolType};
use crate::calculator::Calculator;
use crate::market_state::MarketState;
use crate::swap::{SwapPath, SwapStep};

fn bench_calculator(c: &mut Criterion) {
    // Setup dummy path
    let path = SwapPath {
        steps: vec![
            SwapStep {
                pool_address: address!("4200000000000000000000000000000000000006"),
                token_in: address!("1234567890abcdef1234567890abcdef12345678"),
                token_out: address!("abcdef1234567890abcdef1234567890abcdef12"),
                protocol: PoolType::UniswapV2,
                fee: 3000,
            },
        ],
        hash: 0,
    };

    let market_state = Arc::new(MarketState::mock());
    let calculator = Calculator::new(market_state);

    c.bench_function("calculate_output_single_v2", |b| {
        b.iter(|| {
            let _ = calculator.calculate_output(&path);
        })
    });
}

criterion_group!(benches, bench_calculator);
criterion_main!(benches);
