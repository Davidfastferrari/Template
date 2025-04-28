use tracing::{info, debug, warn, error};
use alloy::sol;
use alloy::sol_types::SolCall;
use serde::{Serialize, Deserialize};
use serde_json::json;
use alloy::network::Network;
use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use alloy::transports::Transport;
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::str::FromStr; // ✅ Make sure this is imported

use super::Calculator;

pub static INITIAL_AMT: Lazy<U256> = Lazy::new(|| U256::from_str("1000000000000000000").unwrap()); // ✅ fixed
pub static WETH: Lazy<Address> = Lazy::new(|| Address::from_str("0x4200000000000000000000000000000000000006").unwrap());
pub static USDC: Lazy<Address> = Lazy::new(|| Address::from_str("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913").unwrap());

// --- Aerodrome V2State contract
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
    P: Provider<N>, // ✅ FIXED
{

pub fn aerodrome_out(&self, amount_in: U256, token_in: Address, pool_address: Address) -> U256 {
        let db = self.market_state.db.read().expect("DB read poisoned");

        let (reserve0, reserve1) = db.get_reserves(&pool_address);
        let (dec0, dec1) = db.get_decimals(&pool_address);
        let fee = db.get_fee(&pool_address);
        let stable = db.get_stable(&pool_address);
        let token0 = db.get_token0(pool_address);

        let mut res0 = U256::from(reserve0);
        let mut res1 = U256::from(reserve1);

        let mut amount_in = amount_in - (amount_in * fee / U256::from(10_000));

        let token0_decimals = U256::from(10).pow(U256::from(dec0));
        let token1_decimals = U256::from(10).pow(U256::from(dec1));

        if stable {
            res0 = (res0 * U256::from(1e18 as u128)) / token0_decimals;
            res1 = (res1 * U256::from(1e18 as u128)) / token1_decimals;

            let (res_a, res_b) = if token_in == token0 { (res0, res1) } else { (res1, res0) };
            amount_in = if token_in == token0 {
                (amount_in * U256::from(1e18 as u128)) / token0_decimals
            } else {
                (amount_in * U256::from(1e18 as u128)) / token1_decimals
            };

            let xy = Self::_k(res0, res1, token0_decimals, token1_decimals);
            let y = res_b - Self::_get_y(amount_in + res_a, xy, res_b);

            if token_in == token0 {
                (y * token1_decimals) / U256::from(1e18 as u128)
            } else {
                (y * token0_decimals) / U256::from(1e18 as u128)
            }
        } else {
            let (res_a, res_b) = if token_in == token0 { (res0, res1) } else { (res1, res0) };
            (amount_in * res_b) / (res_a + amount_in)
        }
    }

    fn _k(x: U256, y: U256, dec0: U256, dec1: U256) -> U256 {
        let x = (x * U256::from(1e18 as u128)) / dec0;
        let y = (y * U256::from(1e18 as u128)) / dec1;
        let a = (x * y) / U256::from(1e18 as u128);
        let b = ((x * x) / U256::from(1e18 as u128)) + ((y * y) / U256::from(1e18 as u128));
        (a * b) / U256::from(1e18 as u128)
    }

    fn _get_y(x0: U256, xy: U256, mut y: U256) -> U256 {
        for _ in 0..255 {
            let k = Self::_f(x0, y);
            let d = Self::_d(x0, y);
            if d.is_zero() {
                return U256::ZERO;
            }
            if k < xy {
                let mut dy = ((xy - k) * U256::from(1e18 as u128)) / d;
                if dy.is_zero() {
                    if k == xy || Self::_k(x0, y + U256::from(1), U256::from(1e18 as u128), U256::from(1e18 as u128)) > xy {
                        return y + U256::from(1);
                    }
                    dy = U256::from(1);
                }
                y += dy;
            } else {
                let mut dy = ((k - xy) * U256::from(1e18 as u128)) / d;
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

    fn _f(x: U256, y: U256) -> U256 {
        let a = (x * y) / U256::from(1e18 as u128);
        let b = ((x * x) + (y * y)) / U256::from(1e18 as u128);
        (a * b) / U256::from(1e18 as u128)
    }

    fn _d(x: U256, y: U256) -> U256 {
        U256::from(3) * x * ((y * y) / U256::from(1e18 as u128)) / U256::from(1e18 as u128)
            + (((x * x) / U256::from(1e18 as u128)) * x) / U256::from(1e18 as u128)
    }
}

// === Extra Utility Functions ===

/// Simulate a MEV sandwich attack on Aerodrome + Uniswap
pub fn simulate_bundle_profit<N, P>(
    calculator: &Calculator<N, P>,
    aerodrome_pool_address: Address,
    uniswap_pool_address: Address,
) -> U256 {
    let profit = calculator.simulate_mev_bundle(
        *INITIAL_AMT,
        *WETH,
        *USDC,
        aerodrome_pool_address,
        uniswap_pool_address,
    );
    profit
}

/// Example usage to print best route
pub fn ample_best_route<N, P>(
    calculator: &Calculator<N, P>,
    initial_amt: U256,
    weth: Address,
    usdc: Address,
) {
    let best_route = calculator.find_best_route(initial_amt, weth, usdc, 3);
    if let Some((path, amount_out)) = best_route {
        println!("Best route: {:?}, Amount out: {}", path, amount_out);
    } else {
        println!("No route found!");
    }
}
