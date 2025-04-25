use super::Calculator;
use alloy::network::Network;
use alloy::primitives::{Address, I256, U256};
use alloy::providers::Provider;
use alloy::transports::Transport;
use anyhow::Result;
use std::collections::HashMap;
// tests/uniswap_fuzz.rs
use super::*;
use proptest::prelude::*;
use uniswap_v3_math::tick_math::{MAX_SQRT_RATIO, MAX_TICK, MIN_SQRT_RATIO, MIN_TICK};

pub const U256_1: U256 = U256::from_limbs([1, 0, 0, 0]);

/// Mock DB access interface
struct MockDB {
    liquidity: u128,
    sqrt_price_x_96: U256,
    tick: i32,
}

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

impl<N, P> Calculator<N, P>
where
    N: Network,
    P: Provider<N, P>,
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

impl MockDB {
    fn build(liquidity: u128, tick: i32) -> Self {
        let sqrt_price = tick_math::get_sqrt_ratio_at_tick(tick).unwrap_or(U256::from(1));
        Self {
            liquidity,
            sqrt_price_x_96: sqrt_price,
            tick,
        }
    }

    fn simulate_v3_swap(
        &self,
        amount_in: U256,
        zero_to_one: bool,
        fee: u32,
    ) -> Result<U256> {
        let tick_spacing = 60;
        let price_limit = if zero_to_one {
            U256::from(MIN_SQRT_RATIO) + U256_ONE
        } else {
            MAX_SQRT_RATIO - U256_ONE
        };

        let mut state = CurrentState {
            sqrt_price_x_96: self.sqrt_price_x_96,
            tick: self.tick,
            liquidity: self.liquidity,
            amount_specified_remaining: I256::from_raw(amount_in),
            amount_calculated: I256::ZERO,
        };

        while state.amount_specified_remaining != I256::ZERO
            && state.sqrt_price_x_96 != price_limit
        {
            let mut step = StepComputations {
                sqrt_price_start_x_96: state.sqrt_price_x_96,
                ..Default::default()
            };

            let next_tick = if zero_to_one {
                state.tick - tick_spacing
            } else {
                state.tick + tick_spacing
            };

            step.tick_next = next_tick.clamp(MIN_TICK, MAX_TICK);
            step.sqrt_price_next_x96 = tick_math::get_sqrt_ratio_at_tick(step.tick_next)?;

            let target = if zero_to_one {
                step.sqrt_price_next_x96.min(price_limit)
            } else {
                step.sqrt_price_next_x96.max(price_limit)
            };

            let (sqrt_next, amt_in, amt_out, fee_amt) = swap_math::compute_swap_step(
                state.sqrt_price_x_96,
                target,
                state.liquidity,
                state.amount_specified_remaining,
                fee,
            )?;

            state.amount_specified_remaining -= I256::from_raw(amt_in + fee_amt);
            state.amount_calculated -= I256::from_raw(amt_out);
            state.sqrt_price_x_96 = sqrt_next;
            state.tick = step.tick_next;
        }

        Ok((-state.amount_calculated).into_raw())
    }
}

/// Fuzz test for UniswapV3 simulation range
proptest! {
    #[test]
    fn test_uniswap_v3_simulation_does_not_panic(
        amount_in in 1u128..1_000_000_000_000_000_000u128,  // [0.000001 ETH .. 1 ETH]
        liquidity in 1_000_000u128..1_000_000_000_000u128,   // simulate wide range
        tick in -887272i32..887272i32,
        fee in 1_000u32..10_000u32,
        zero_to_one in any::<bool>()
    ) {
        let db = MockDB::build(liquidity, tick);
        let result = db.simulate_v3_swap(U256::from(amount_in), zero_to_one, fee);
        prop_assert!(result.is_ok(), "Simulation should not error or panic");
    }
}
