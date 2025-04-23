use super::Calculator;
use alloy::{
    consensus::Transaction,
    network::{TransactionBuilder, EthereumWallet, Ethereum, Network, BlockResponse, HeaderResponse}
    primitives::{BlockNumber, B256, I256, hex, address, U256, U160, Address, FixedBytes, Bytes},
    providers::{Provider, ProviderBuilder, RootProvider},
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
 sol_types::{SolCall, SolValue, SolType},
};

sol! {
    #[sol(rpc)]
    contract v2state {
        function getreserves() external view returns (uint112 reserve0, uint112 reserve1, uint32 blocktimestamplast);
    }
}

impl<T, N, P> Calculator<T, N, P>
where
    T: Transport + Clone,
    N: Network,
    P: Provider<T, N>,
{
    // Amount out calculation for aerodrome pools
    pub fn aerodrome_out(&self, amount_in: U256, token_in: Address, pool_address: Address) -> U256 {
        // get all of the state
        let db_read = self.market_state.db.read().unwrap();
        let (reserve0, reserve1) = db_read.get_reserves(&pool_address);
        let pool_fee = db_read.get_fee(&pool_address);
        let (dec_0, dec_1) = db_read.get_decimals(&pool_address);
        let stable = db_read.get_stable(&pool_address);
        let token0 = db_read.get_token0(pool_address);

        let mut _reserve0 = U256::from(reserve0);
        let mut _reserve1 = U256::from(reserve1);

        let mut amount_in = amount_in;
        amount_in -= (amount_in * pool_fee) / U256::from(10000);

        let token0_decimals = U256::from(10).pow(U256::from(dec_0));
        let token1_decimals = U256::from(10).pow(U256::from(dec_1));

        if stable {
            let xy = Self::_k(
                _reserve0,
                _reserve1,
                stable,
                token0_decimals,
                token1_decimals,
            );
            _reserve0 = (_reserve0 * U256::from(1e18)) / token0_decimals;
            _reserve1 = (_reserve1 * U256::from(1e18)) / token1_decimals;
            let (reserve_a, reserve_b) = if token_in == token0 {
                (_reserve0, _reserve1)
            } else {
                (_reserve1, _reserve0)
            };
            amount_in = if token_in == token0 {
                (amount_in * U256::from(1e18)) / token0_decimals
            } else {
                (amount_in * U256::from(1e18)) / token1_decimals
            };
            let y = reserve_b
                - Self::_get_y(
                    amount_in + reserve_a,
                    xy,
                    reserve_b,
                    stable,
                    token0_decimals,
                    token1_decimals,
                );
            if token_in == token0 {
                (y * token1_decimals) / U256::from(1e18)
            } else {
                (y * token0_decimals) / U256::from(1e18)
            }
        } else {
            let (reserve_a, reserve_b) = if token_in == token0 {
                (_reserve0, _reserve1)
            } else {
                (_reserve1, _reserve0)
            };
            (amount_in * reserve_b) / (reserve_a + amount_in)
        }
    }

    fn _k(x: U256, y: U256, stable: bool, decimals0: U256, decimals1: U256) -> U256 {
        if stable {
            let _x = (x * U256::from(1e18)) / decimals0;
            let _y = (y * U256::from(1e18)) / decimals1;
            let _a = (_x * _y) / U256::from(1e18);
            let _b = (_x * _x) / U256::from(1e18) + (_y * _y) / U256::from(1e18);
            (_a * _b) / U256::from(1e18)
        } else {
            x * y
        }
    }

    fn _get_y(x0: U256, xy: U256, y: U256, stable: bool, decimals0: U256, decimals1: U256) -> U256 {
        let mut y = y;
        for _ in 0..255 {
            let k = Self::_f(x0, y);
            let d = Self::_d(x0, y);
            if d == U256::ZERO {
                return U256::ZERO;
            }
            if k < xy {
                let mut dy = ((xy - k) * U256::from(1e18)) / d;
                if dy == U256::ZERO {
                    if k == xy {
                        return y;
                    }
                    if Self::_k(x0, y + U256::from(1), stable, decimals0, decimals1) > xy {
                        return y + U256::from(1);
                    }
                    dy = U256::from(1);
                }
                y += dy;
            } else {
                let mut dy = ((k - xy) * U256::from(1e18)) / d;
                if dy == U256::ZERO {
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

    fn _f(x0: U256, y: U256) -> U256 {
        let _a = (x0 * y) / U256::from(1e18);
        let _b = (x0 * x0) / U256::from(1e18) + (y * y) / U256::from(1e18);
        (_a * _b) / U256::from(1e18)
    }

    fn _d(x0: U256, y: U256) -> U256 {
        U256::from(3) * x0 * ((y * y) / U256::from(1e18)) / U256::from(1e18)
            + (((x0 * x0) / U256::from(1e18)) * x0) / U256::from(1e18)
    }
}
