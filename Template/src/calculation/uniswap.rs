use super::Calculator;
use alloy::{
    consensus::Transaction,
    network::{ TransactionBuilder, EthereumWallet, Ethereum, Network, BlockResponse, HeaderResponse },
    primitives::{ BlockNumber, B256, I256, hex, address, U256, U160, Address, FixedBytes, Bytes },
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
};
use uniswap_v3_math::{
     tick_math::{ MAX_SQRT_RATIO, MAX_TICK, MIN_SQRT_RATIO, MIN_TICK },
     tick_bitmap,
     swap_math::{ compute_swap_step },
}
use anyhow::Result;
use std::collections::HashMap;
use uniswap_v3_sdk::prelude::*;

pub const U256_1: U256 = U256::from_limbs([1, 0, 0, 0]);

pub struct CurrentState {
    amount_specified_remaining: I256,
    amount_calculated: I256,
    sqrt_price_x_96: U256,
    tick: i32,
    liquidity: u128,
}

#[derive(Default)]
pub struct StepComputations {
    pub sqrt_price_start_x_96: U256,
    pub tick_next: i32,
    pub initialized: bool,
    pub sqrt_price_next_x96: U256,
    pub amount_in: U256,
    pub amount_out: U256,
    pub fee_amount: U256,
}

//Computes the position in the mapping where the initialized bit for a tick lives
pub fn position(tick: i32) -> (i16, u8) {
    ((tick >> 8) as i16, (tick % 256) as u8)
}

impl<T, N, P> Calculator<T, N, P>
where
    T: Transport + Clone,
    N: Network,
    P: Provider<T, N>,
{
    // Calcualte the amount out for a uniswapv2 swap
    #[inline]
    pub fn uniswap_v2_out(
        &self,
        amount_in: U256,
        pool_address: &Address,
        token_in: &Address,
        fee: U256,
    ) -> U256 {
        // get read access to db
        let db_read = self.market_state.db.read().unwrap();
        let zero_to_one = db_read.zero_to_one(pool_address, *token_in).unwrap();
        let (reserve0, reserve1) = db_read.get_reserves(pool_address);

        let scalar = U256::from(10000);

        let (reserve0, reserve1) = if zero_to_one {
            (reserve0, reserve1)
        } else {
            (reserve1, reserve0)
        };

        let amount_in_with_fee = amount_in * fee;
        let numerator = amount_in_with_fee * reserve1;
        let denominator = reserve0 * scalar + amount_in_with_fee;
        numerator / denominator
    }

    // calculate the amount out for a uniswapv3 swap
    #[inline]
    pub fn uniswap_v3_out(
        &self,
        amount_in: U256,
        pool_address: &Address,
        token_in: &Address,
        fee: u32,
    ) -> Result<U256> {
        if amount_in.is_zero() {
            return Ok(U256::ZERO);
        }

        // acquire db read access and get all our state information
        let db_read = self.market_state.db.read().unwrap();
        let zero_to_one = db_read.zero_to_one(pool_address, *token_in).unwrap();
        let slot0 = db_read.slot0(*pool_address)?;
        let liquidity = db_read.liquidity(*pool_address)?;
        let tick_spacing = db_read.tick_spacing(pool_address)?;

        // Set sqrt_price_limit_x_96 to the max or min sqrt price in the pool depending on zero_for_one
        let sqrt_price_limit_x_96 = if zero_to_one {
            U256::from(MIN_SQRT_RATIO) + U256_1
        } else {
            MAX_SQRT_RATIO - U256_1
        };

        // Initialize a mutable state state struct to hold the dynamic simulated state of the pool
        let mut current_state = CurrentState {
            sqrt_price_x_96: slot0.sqrtPriceX96.to(), //Active price on the pool
            amount_calculated: I256::ZERO,            //Amount of token_out that has been calculated
            amount_specified_remaining: I256::from_raw(amount_in), //Amount of token_in that has not been swapped
            tick: slot0.tick.as_i32(),
            liquidity, //Current available liquidity in the tick range
        };

        let exact_input = true; // We're always doing exact input when calculating output
        
        while current_state.amount_specified_remaining != I256::ZERO 
            && current_state.sqrt_price_x_96 != sqrt_price_limit_x_96 
        {
            // Initialize a new step struct to hold the dynamic state of the pool at each step
            let mut step = StepComputations {
                // Set the sqrt_price_start_x_96 to the current sqrt_price_x_96
                sqrt_price_start_x_96: current_state.sqrt_price_x_96,
                ..Default::default()
            };

            let mut tick_bitmap: HashMap<i16, U256> = HashMap::new();
            let (word_pos, _bit_pos) = position(current_state.tick / (tick_spacing));

            for i in word_pos - 1..=word_pos + 1 {
                tick_bitmap.insert(i, db_read.tick_bitmap(*pool_address, i).unwrap_or_default());
            }

            // Get the next tick from the current tick
            (step.tick_next, step.initialized) =
                tick_bitmap::next_initialized_tick_within_one_word(
                    &tick_bitmap,
                    current_state.tick,
                    tick_spacing,
                    zero_to_one,
                )?;

            // ensure that we do not overshoot the min/max tick, as the tick bitmap is not aware of these bounds
            // Note: this could be removed as we are clamping in the batch contract
            step.tick_next = step.tick_next.clamp(MIN_TICK, MAX_TICK);

            // Get the next sqrt price from the input amount
            step.sqrt_price_next_x96 =
                tick_math::get_sqrt_ratio_at_tick(step.tick_next)?;

            // Target spot price
            let swap_target_sqrt_ratio = if zero_to_one {
                if step.sqrt_price_next_x96 < sqrt_price_limit_x_96 {
                    sqrt_price_limit_x_96
                } else {
                    step.sqrt_price_next_x96
                }
            } else if step.sqrt_price_next_x96 > sqrt_price_limit_x_96 {
                sqrt_price_limit_x_96
            } else {
                step.sqrt_price_next_x96
            };

            // Compute swap step and update the current state
            let (sqrt_price_next_x96, amount_in, amount_out, fee_amount) = 
                swap_math::compute_swap_step(
                    current_state.sqrt_price_x_96,
                    swap_target_sqrt_ratio,
                    current_state.liquidity,
                    current_state.amount_specified_remaining,
                    fee,
                )?;

            // Update state using exact input logic from on-chain code
            current_state.amount_specified_remaining -= I256::from_raw(
                amount_in.overflowing_add(fee_amount).0
            );
            current_state.amount_calculated -= I256::from_raw(amount_out);
            current_state.sqrt_price_x_96 = sqrt_price_next_x96;

            // Update tick and liquidity only if needed for next iteration
            if current_state.sqrt_price_x_96 == step.sqrt_price_next_x96 {
                if step.initialized {
                    let mut liquidity_net: i128 = 
                        db_read.ticks_liquidity_net(*pool_address, step.tick_next)?;
                    
                    if zero_to_one {
                        liquidity_net = -liquidity_net;
                    }
                    
                    current_state.liquidity = if liquidity_net < 0 {
                        current_state.liquidity.checked_sub(-liquidity_net as u128)
                            .ok_or_else(|| anyhow::anyhow!("Insufficient liquidity"))?
                    } else {
                        current_state.liquidity.checked_add(liquidity_net as u128)
                            .ok_or_else(|| anyhow::anyhow!("Liquidity overflow"))?
                    };
                }
                current_state.tick = if zero_to_one {
                    step.tick_next - 1
                } else {
                    step.tick_next
                };
            } else if current_state.sqrt_price_x_96 != step.sqrt_price_start_x_96 {
                current_state.tick = uniswap_v3_math::tick_math::get_tick_at_sqrt_ratio(
                    current_state.sqrt_price_x_96,
                )?;
            }
        }

        Ok((-current_state.amount_calculated).into_raw())
    }
}
