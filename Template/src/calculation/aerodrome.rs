use tracing::{info, error, debug, warn};
use alloy_sol_types::sol;
use serde::{Serialize, Deserialize};
use serde_json::json;
use super::Calculator;
use alloy::sol;
use alloy::network::Network;
use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use alloy::transports::Transport;
use std::collections::{HashMap, HashSet};
use once_cell::sync::Lazy;

pub static WETH: Lazy<Address> = Lazy::new(|| {
    Address::from_str("0x4200000000000000000000000000000000000006").expect("Invalid WETH address")
});

pub static USDC: Lazy<Address> = Lazy::new(|| {
    Address::from_str("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913").expect("Invalid USDC address")
});

pub static INITIAL_AMT: Lazy<U256> = Lazy::new(|| {
    U256::from_dec_str("1000000000000000000").unwrap() // 1 ETH
});

sol! {
    #[sol(rpc)]
    contract V2State {
        function getReserves() external view returns (
            uint112 reserve0,
            uint112 reserve1,
            uint32 blockTimestampLast
        );
    }
}

impl<N, P> Calculator<N, P>
where
    N: Network,
    P: Provider<N, P>,
{
    /// Calculate output amount for Aerodrome-style pool
    /// Supports both stable and volatile formulas
    pub fn aerodrome_out(&self, amount_in: U256, token_in: Address, pool_address: Address) -> U256 {
        let db = self.market_state.db.read().expect("DB read poisoned");

        // Load state from DB
        let (reserve0, reserve1) = db.get_reserves(&pool_address);
        let (dec0, dec1) = db.get_decimals(&pool_address);
        let fee = db.get_fee(&pool_address);
        let stable = db.get_stable(&pool_address);
        let token0 = db.get_token0(pool_address);

        // Convert reserve to U256
        let mut res0 = U256::from(reserve0);
        let mut res1 = U256::from(reserve1);

        // Apply swap fee
        let mut amount_in = amount_in - (amount_in * fee / U256::from(10_000));

        let token0_decimals = U256::from(10).pow(U256::from(dec0));
        let token1_decimals = U256::from(10).pow(U256::from(dec1));

        if stable {
            // Normalize to 1e18 scale
            res0 = (res0 * U256::from(1e18)) / token0_decimals;
            res1 = (res1 * U256::from(1e18)) / token1_decimals;

            // Order tokens
            let (res_a, res_b) = if token_in == token0 { (res0, res1) } else { (res1, res0) };
            amount_in = if token_in == token0 {
                (amount_in * U256::from(1e18)) / token0_decimals
            } else {
                (amount_in * U256::from(1e18)) / token1_decimals
            };

            let xy = Self::_k(res0, res1, token0_decimals, token1_decimals);
            let y = res_b - Self::_get_y(amount_in + res_a, xy, res_b);

            // Rescale back to token decimals
            if token_in == token0 {
                (y * token1_decimals) / U256::from(1e18)
            } else {
                (y * token0_decimals) / U256::from(1e18)
            }
        } else {
            let (res_a, res_b) = if token_in == token0 { (res0, res1) } else { (res1, res0) };
            (amount_in * res_b) / (res_a + amount_in)
        }
    }

    /// Custom stable AMM invariant
    fn _k(x: U256, y: U256, dec0: U256, dec1: U256) -> U256 {
        let x = (x * U256::from(1e18)) / dec0;
        let y = (y * U256::from(1e18)) / dec1;
        let a = (x * y) / U256::from(1e18);
        let b = ((x * x) / U256::from(1e18)) + ((y * y) / U256::from(1e18));
        (a * b) / U256::from(1e18)
    }

    /// Iteratively solve y for invariant equation
    fn _get_y(x0: U256, xy: U256, mut y: U256) -> U256 {
        for _ in 0..255 {
            let k = Self::_f(x0, y);
            let d = Self::_d(x0, y);
            if d.is_zero() {
                return U256::ZERO;
            }

            if k < xy {
                let mut dy = ((xy - k) * U256::from(1e18)) / d;
                if dy.is_zero() {
                    if k == xy || Self::_k(x0, y + U256::from(1), U256::from(1e18), U256::from(1e18)) > xy {
                        return y + U256::from(1);
                    }
                    dy = U256::from(1);
                }
                y += dy;
            } else {
                let mut dy = ((k - xy) * U256::from(1e18)) / d;
                if dy.is_zero() {
                    if k == xy || Self::_f(x0, y - U256::from(1)) < xy {
                        return y;
                    }
                    dy = U256::from(1);
                }
                y -= dy;
            }
        }
        U256::ZERO
    }

    /// Stable function: f(x, y) = x*y * (x^2 + y^2)
    fn _f(x: U256, y: U256) -> U256 {
        let a = (x * y) / U256::from(1e18);
        let b = ((x * x) + (y * y)) / U256::from(1e18);
        (a * b) / U256::from(1e18)
    }

    /// Derivative for Newton-Raphson
    fn _d(x: U256, y: U256) -> U256 {
        U256::from(3) * x * ((y * y) / U256::from(1e18)) / U256::from(1e18)
            + (((x * x) / U256::from(1e18)) * x) / U256::from(1e18)
    }
}

/// Simulate a MEV sandwich attack on Aerodrome + Uniswap combo
pub fn simulate_mev_bundle(
    &self,
    frontrun_amount: U256,
    token_in: Address,
    token_out: Address,
    aerodrome_pool: Address,
    user_pool_out: Address,  // where the user swaps from token_out → token_in
) -> U256 {
    // 1. Frontrun: WETH → USDC (Aerodrome)
    let pre_usdc = self.aerodrome_out(frontrun_amount, token_in, aerodrome_pool);

    // 2. User swaps USDC → WETH (Uniswap etc.)
    let user_returns = self.simulate_uniswap_v2_out(pre_usdc, token_out, token_in, user_pool_out);

    // 3. Backrun: USDC → WETH (Aerodrome)
    let final_backrun = self.aerodrome_out(user_returns, token_out, aerodrome_pool);

    final_backrun
}

/// Simple x*y=k output simulator for UniswapV2-style pool
pub fn simulate_uniswap_v2_out(
    &self,
    amount_in: U256,
    token_in: Address,
    pool_address: Address,
) -> U256 {
    let db = self.market_state.db.read().unwrap();
    let (res0, res1) = db.get_reserves(&pool_address);
    let token0 = db.get_token0(pool_address);
    let fee = db.get_fee(&pool_address);

    let (res_in, res_out) = if token_in == token0 {
        (U256::from(res0), U256::from(res1))
    } else {
        (U256::from(res1), U256::from(res0))
    };

    let amount_in_after_fee = amount_in * (U256::from(10_000) - fee) / U256::from(10_000);
    (amount_in_after_fee * res_out) / (res_in + amount_in_after_fee)
}

/// Build a graph of token → token paths via Aerodrome pools
pub fn build_aerodrome_graph(&self) -> HashMap<Address, Vec<(Address, Address)>> {
    let db = self.market_state.db.read().unwrap();
    let mut graph = HashMap::new();

    for pool in db.all_pools().iter().filter(|p| p.is_aerodrome()) {
        let t0 = pool.token0_address();
        let t1 = pool.token1_address();
        let addr = pool.address();

        graph.entry(t0).or_default().push((t1, addr));
        graph.entry(t1).or_default().push((t0, addr));
    }

    graph
}

/// Estimate best return from token_in → token_out using max `max_hops` routes via Aerodrome pools
pub fn find_best_route(
    &self,
    amount_in: U256,
    token_in: Address,
    token_out: Address,
    max_hops: usize,
) -> Option<(Vec<Address>, U256)> {
    let graph = self.build_aerodrome_graph();
    let mut visited = HashSet::new();
    let mut best = None;

    fn dfs<'a, N, P>(
        calc: &Calculator<N, P>,
        graph: &HashMap<Address, Vec<(Address, Address)>>,
        current_token: Address,
        target: Address,
        amount_in: U256,
        hops: usize,
        max_hops: usize,
        path: Vec<Address>,
        visited: &mut HashSet<Address>,
        best: &mut Option<(Vec<Address>, U256)>,
    ) where
        T: Transport + Clone,
        N: Network,
        P: Provider<N>,
    {
        if hops > max_hops {
            return;
        }
        if current_token == target {
            if best.is_none() || amount_in > best.as_ref().unwrap().1 {
                *best = Some((path.clone(), amount_in));
            }
            return;
        }

        if let Some(edges) = graph.get(&current_token) {
            for (next_token, pool) in edges {
                if visited.contains(next_token) {
                    continue;
                }

                let output = calc.aerodrome_out(amount_in, current_token, *pool);
                if output.is_zero() {
                    continue;
                }

                visited.insert(*next_token);
                let mut new_path = path.clone();
                new_path.push(*next_token);

                dfs(
                    calc,
                    graph,
                    *next_token,
                    target,
                    output,
                    hops + 1,
                    max_hops,
                    new_path,
                    visited,
                    best,
                );
                visited.remove(next_token);
            }
        }
    }

    visited.insert(token_in);
    dfs(
        self,
        &graph,
        token_in,
        token_out,
        amount_in,
        0,
        max_hops,
        vec![token_in],
        &mut visited,
        &mut best,
    );

    best
}

// Sandwich simulation
let profit = calculator.simulate_mev_bundle(
    *INITIAL_AMT,
    *WETH, 
    *USDC,
    aerodrome_pool_address,
    uniswap_pool_address,
);

// Graph route estimate
let best_route = calculator.find_best_route(initial_amt, weth, usdc, 3);
if let Some((path, amount_out)) = best_route {
    println!("Best route: {:?}, Amount out: {}", path, amount_out);
}
