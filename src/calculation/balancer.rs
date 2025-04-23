use std::ops::Neg;
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
use std::str::FromStr;
use super::Calculator;

impl Calculator<T, N, P>{

    pub fn balancer_v2_out(
        &self, 
        amount_in: U256,
        token_in: Address,
        token_out: Address,
        pool_address: Address,
    ) -> U256 {
        let pool = self.pool_manager.get_balancer_pool(&pool_address);
        let token_in_index = pool.get_token_index(&token_in).unwrap();
        let token_out_index = pool.get_token_index(&token_out).unwrap();
        let balance_in = pool.balances[token_in_index];
        let balance_out = pool.balances[token_out_index];
        let weight_in = pool.weights[token_in_index];
        let weight_out = pool.weights[token_out_index];
        let swap_fee_percentage = pool.swap_fee;

        let scaling_factor = 18 - pool.token0_decimals as i8;
        let scaled_amount_in = Self::scale(amount_in, scaling_factor);
        let scaled_amount_in_without_fees = Self::sub(scaled_amount_in, Self::mul_up(scaled_amount_in, swap_fee_percentage));
        let amount_in = Self::scale(scaled_amount_in_without_fees, scaling_factor);

        let denominator = Self::add(balance_in, amount_in);
        let base = Self::div_up(balance_in, denominator);
        let exponent = Self::div_down(weight_in, weight_out);
        let power = Self::pow_up(base, exponent);

        Self::mul_down(balance_out, Self::complement(power))
    }

    fn scale(value: U256, decimals: i8) -> U256 {
        value * (U256::from(10).pow(U256::from(decimals)))
    }

    fn add(a: U256, b: U256) -> U256 {
        a + b
    }

    fn sub(a: U256, b: U256) -> U256 {
        a - b
    }

    fn div_up(a: U256, b: U256) -> U256 {
        let one = U256::from(1e18);
        if a == U256::ZERO {
            return U256::ZERO;
        }
        let a_inflated = a * one;
        ((a_inflated - U256::from(1)) / b) + U256::from(1)
    }

    fn div_down(a: U256, b: U256) -> U256 {
        let one = U256::from(1e18);
        if a == U256::ZERO {
            return U256::ZERO;
        }
        let a_inflated = a * one;
        a_inflated / b
    }

    fn mul_up(a: U256, b: U256) -> U256 {
        let one = U256::from(1e18);
        let product = a * b;

        if product == U256::ZERO {
            U256::ZERO
        } else {
            ((product - U256::from(1)) / one) + U256::from(1)
        }
    }

    fn mul_down(a: U256, b: U256) -> U256 {
        let one = U256::from(1e18);
        let product = a * b;
        product / one
    }

    fn pow_up(x: U256, y: U256) -> U256 {
        let max_pow_relative_error = U256::from(10000);
        let one = U256::from(1e18);
        let two = one * U256::from(2);
        let four = one * U256::from(4);
        if y == one {
            x
        } else if y == two {
            Self::mul_up(x, x)
        } else if y == four {
            let square = Self::mul_up(x, x);
            return Self::mul_up(square, square);
        } else {
            let raw = LogExpMath::pow(x, y);

            let max_error = Self::add(Self::mul_up(raw, max_pow_relative_error), U256::from(1));
            return Self::add(raw, max_error);
        }
    }

    fn complement(x: U256) -> U256 {
        let one = U256::from(1e18);
        if x < one {
            one - x
        } else {
            U256::ZERO
        }
    }
}

pub struct LogExpMath;
impl LogExpMath {
    // Constants
    fn one_18() -> I256 { I256::from_raw(U256::from(1e18)) }
    fn one_20() -> I256 { I256::from_raw(U256::from(1e20)) }
    fn one_36() -> I256 { I256::from_str("1000000000000000000000000000000000000").unwrap() }

    fn max_natural_exponent() -> I256 { I256::from_raw(U256::from(130e18)) }
    fn min_natural_exponent() -> I256 { -I256::from_raw(U256::from(41e18)) }

    fn ln_36_lower_bound() -> I256 { I256::from_raw(U256::from(1e18) - U256::from(1e17)) }
    fn ln_36_upper_bound() -> I256 { I256::from_raw(U256::from(1e18) + U256::from(1e17)) }

    fn mild_exponent_bound() -> U256 { U256::from(2).pow(U256::from(254)) / U256::from(1e20)}

    fn x0() -> I256 { I256::from_raw(U256::from(128000000000000000000_u128)) }
    fn a0() -> I256 { I256::from_raw(U256::from_str("38877084059945950922200000000000000000000000000000000000").unwrap()) }
    fn x1() -> I256 { I256::from_raw(U256::from(64000000000000000000_u128)) }
    fn a1() -> I256 { I256::from_raw(U256::from(6235149080811616882910000000_u128)) }

    // 20 decimal constants
    fn x2() -> I256 { I256::from_raw(U256::from(3200000000000000000000_u128)) }
    fn a2() -> I256 { I256::from_raw(U256::from_str("7896296018268069516100000000000000").unwrap()) }
    fn x3() -> I256 { I256::from_raw(U256::from(1600000000000000000000_u128)) }
    fn a3() -> I256 { I256::from_raw(U256::from(888611052050787263676000000_u128)) }
    fn x4() -> I256 { I256::from_raw(U256::from(800000000000000000000_u128)) }
    fn a4() -> I256 { I256::from_raw(U256::from(298095798704172827474000_u128)) }
    fn x5() -> I256 { I256::from_raw(U256::from(400000000000000000000_u128)) }
    fn a5() -> I256 { I256::from_raw(U256::from(5459815003314423907810_u128)) }
    fn x6() -> I256 { I256::from_raw(U256::from(200000000000000000000_u128)) }
    fn a6() -> I256 { I256::from_raw(U256::from(738905609893065022723_u128)) }
    fn x7() -> I256 { I256::from_raw(U256::from(100000000000000000000_u128)) }
    fn a7() -> I256 { I256::from_raw(U256::from(271828182845904523536_u128)) }
    fn x8() -> I256 { I256::from_raw(U256::from(50000000000000000000_u128)) }
    fn a8() -> I256 { I256::from_raw(U256::from(164872127070012814685_u128)) }
    fn x9() -> I256 { I256::from_raw(U256::from(25000000000000000000_u128)) }
    fn a9() -> I256 { I256::from_raw(U256::from(128402541668774148407_u128)) }
    fn x10() -> I256 { I256::from_raw(U256::from(12500000000000000000_u128)) }
    fn a10() -> I256 { I256::from_raw(U256::from(113314845306682631683_u128)) }
    fn x11() -> I256 { I256::from_raw(U256::from(6250000000000000000_u128)) }
    fn a11() -> I256 { I256::from_raw(U256::from(106449445891785942956_u128)) }

    pub fn pow(x: U256, y: U256) -> U256 {
        if y == U256::ZERO {
            return U256::from(1e18);
        }

        if x == U256::ZERO {
            return U256::ZERO;
        }

        assert!(x < U256::from(2).pow(U256::from(255)), "X_OUT_OF_BOUNDS");
        let x_int256 = I256::from_raw(x);

        assert!(y < Self::mild_exponent_bound(), "Y_OUT_OF_BOUNDS");
        let y_int256 = I256::from_raw(y);

        let logx_times_y = if Self::ln_36_lower_bound() < x_int256 && x_int256 < Self::ln_36_upper_bound() {
            let ln_36_x = Self::_ln_36(x_int256);


            (ln_36_x / Self::one_18()) * y_int256 + ((ln_36_x % Self::one_18()) * y_int256) / Self::one_18()
        } else {
            Self::_ln(x_int256) * y_int256
        };
        let logx_times_y = logx_times_y / Self::one_18();

        // Finally, we compute exp(y * ln(x)) to arrive at x^y
        assert!(
            Self::min_natural_exponent() <= logx_times_y && logx_times_y <= Self::max_natural_exponent(),
            "PRODUCT_OUT_OF_BOUNDS"
        );

        let exp_result = Self::exp(logx_times_y);
        U256::try_from(exp_result.abs()).expect("Conversion to U256 failed")
    }

    pub fn exp(x: I256) -> I256 {
        assert!(x >= Self::min_natural_exponent() && x <= Self::max_natural_exponent(), "INVALID_EXPONENT");

        if x.is_negative() {
            // We only handle positive exponents: e^(-x) is computed as 1 / e^x. We can safely make x positive since it
            // fits in the signed 256 bit range (as it is larger than MIN_NATURAL_EXPONENT).
            // Fixed point division requires multiplying by ONE_18.
            return (Self::one_18() * Self::one_18()) / Self::exp(x.neg());
        }

        // First, we use the fact that e^(x+y) = e^x * e^y to decompose x into a sum of powers of two, which we call x_n,
        // where x_n == 2^(7 - n), and e^x_n = a_n has been precomputed. We choose the first x_n, x0, to equal 2^7
        // because all larger powers are larger than MAX_NATURAL_EXPONENT, and therefore not present in the
        // decomposition.
        // At the end of this process we will have the product of all e^x_n = a_n that apply, and the remainder of this
        // decomposition, which will be lower than the smallest x_n.
        // exp(x) = k_0 * a_0 * k_1 * a_1 * ... + k_n * a_n * exp(remainder), where each k_n equals either 0 or 1.
        // We mutate x by subtracting x_n, making it the remainder of the decomposition.

        // The first two a_n (e^(2^7) and e^(2^6)) are too large if stored as 18 decimal numbers, and could cause
        // intermediate overflows. Instead we store them as plain integers, with 0 decimals.
        // Additionally, x0 + x1 is larger than MAX_NATURAL_EXPONENT, which means they will not both be present in the
        // decomposition.

        // For each x_n, we test if that term is present in the decomposition (if x is larger than it), and if so deduct
        // it and compute the accumulated product.

        let mut x = x;
        let first_an;
        if x >= Self::x0() {
            x -= Self::x0();
            first_an = Self::a0();
        } else if x >= Self::x1() {
            x -= Self::x1();
            first_an = Self::a1();
        } else {
            first_an = I256::from_str("1").unwrap(); // One with no decimal places
        }

        // We now transform x into a 20 decimal fixed point number, to have enhanced precision when computing the
        // smaller terms.
        x *= I256::from_raw(U256::from(100));

        // `product` is the accumulated product of all a_n (except a0 and a1), which starts at 20 decimal fixed point
        // one. Recall that fixed point multiplication requires dividing by ONE_20.
        let mut product = Self::one_20();

        if x >= Self::x2() {
            x -= Self::x2();
            product = (product * Self::a2()) / Self::one_20();
        }
        if x >= Self::x3() {
            x -= Self::x3();
            product = (product * Self::a3()) / Self::one_20();
        }
        if x >= Self::x4() {
            x -= Self::x4();
            product = (product * Self::a4()) / Self::one_20();
        }
        if x >= Self::x5() {
            x -= Self::x5();
            product = (product * Self::a5()) / Self::one_20();
        }
        if x >= Self::x6() {
            x -= Self::x6();
            product = (product * Self::a6()) / Self::one_20();
        }
        if x >= Self::x7() {
            x -= Self::x7();
            product = (product * Self::a7()) / Self::one_20();
        }
        if x >= Self::x8() {
            x -= Self::x8();
            product = (product * Self::a8()) / Self::one_20();
        }
        if x >= Self::x9() {
            x -= Self::x9();
            product = (product * Self::a9()) / Self::one_20();
        }

        // x10 and x11 are unnecessary here since we have high enough precision already.

        // Now we need to compute e^x, where x is small (in particular, it is smaller than x9). We use the Taylor series
        // expansion for e^x: 1 + x + (x^2 / 2!) + (x^3 / 3!) + ... + (x^n / n!).

        let mut series_sum = Self::one_20(); // The initial one in the sum, with 20 decimal places.
        let mut term; // Each term in the sum, where the nth term is (x^n / n!).

        // The first term is simply x.
        term = x;
        series_sum += term;

        // Each term (x^n / n!) equals the previous one times x, divided by n. Since x is a fixed point number,
        // multiplying by it requires dividing by ONE_20, but dividing by the non-fixed point n values does not.

        term = ((term * x) / Self::one_20()) / I256::from_raw(U256::from(2));
        series_sum += term;

        term = ((term * x) / Self::one_20()) / I256::from_raw(U256::from(3));
        series_sum += term;

        term = ((term * x) / Self::one_20()) / I256::from_raw(U256::from(4));
        series_sum += term;

        term = ((term * x) / Self::one_20()) / I256::from_raw(U256::from(5));
        series_sum += term;

        term = ((term * x) / Self::one_20()) / I256::from_raw(U256::from(6));
        series_sum += term;

        term = ((term * x) / Self::one_20()) / I256::from_raw(U256::from(7));
        series_sum += term;

        term = ((term * x) / Self::one_20()) / I256::from_raw(U256::from(8));
        series_sum += term;

        term = ((term * x) / Self::one_20()) / I256::from_raw(U256::from(9));
        series_sum += term;

        term = ((term * x) / Self::one_20()) / I256::from_raw(U256::from(10));
        series_sum += term;

        term = ((term * x) / Self::one_20()) / I256::from_raw(U256::from(11));
        series_sum += term;

        term = ((term * x) / Self::one_20()) / I256::from_raw(U256::from(12));
        series_sum += term;

        // 12 Taylor terms are sufficient for 18 decimal precision.

        // We now have the first a_n (with no decimals), and the product of all other a_n present, and the Taylor
        // approximation of the exponentiation of the remainder (both with 20 decimals). All that remains is to multiply
        // all three (one 20 decimal fixed point multiplication, dividing by ONE_20, and one integer multiplication),
        // and then drop two digits to return an 18 decimal value.

        (((product * series_sum) / Self::one_20()) * first_an) / I256::from_raw(U256::from(100))
    }

    fn _ln(mut a: I256) -> I256 {
        if a < Self::one_18() {
            return -Self::_ln((Self::one_18() * Self::one_18()) / a);
        }

        let mut sum = I256::ZERO;
        if a >= Self::a0() * Self::one_18() {
            a /= Self::a0(); // Integer, not fixed point division
            sum += Self::x0();
        }

        if a >= Self::a1() * Self::one_18() {
            a /= Self::a1(); // Integer, not fixed point division
            sum += Self::x1();
        }

        // All other a_n and x_n are stored as 20 digit fixed point numbers, so we convert the sum and a to this format.
        sum *= I256::from_raw(U256::from(100));
        a *= I256::from_raw(U256::from(100));


        // Because further a_n are  20 digit fixed point numbers, we multiply by ONE_20 when dividing by them.

        if a >= Self::a2() {
            a = (a * Self::one_20()) / Self::a2();
            sum += Self::x2();
        }

        if a >= Self::a3() {
            a = (a * Self::one_20()) / Self::a3();
            sum += Self::x3();
        }

        if a >= Self::a4() {
            a = (a * Self::one_20()) / Self::a4();
            sum += Self::x4();
        }

        if a >= Self::a5() {
            a = (a * Self::one_20()) / Self::a5();
            sum += Self::x5();
        }

        if a >= Self::a6() {
            a = (a * Self::one_20()) / Self::a6();
            sum += Self::x6();
        }

        if a >= Self::a7() {
            a = (a * Self::one_20()) / Self::a7();
            sum += Self::x7();
        }

        if a >= Self::a8() {
            a = (a * Self::one_20()) / Self::a8();
            sum += Self::x8();
        }

        if a >= Self::a9() {
            a = (a * Self::one_20()) / Self::a9();
            sum += Self::x9();
        }

        if a >= Self::a10() {
            a = (a * Self::one_20()) / Self::a10();
            sum += Self::x10();
        }

        if a >= Self::a11() {
            a = (a * Self::one_20()) / Self::a11();
            sum += Self::x11();
        }

        // a is now a small number (smaller than a_11, which roughly equals 1.06). This means we can use a Taylor series
        // that converges rapidly for values of `a` close to one - the same one used in ln_36.
        // Let z = (a - 1) / (a + 1).
        // ln(a) = 2 * (z + z^3 / 3 + z^5 / 5 + z^7 / 7 + ... + z^(2 * n + 1) / (2 * n + 1))

        // Recall that 20 digit fixed point division requires multiplying by ONE_20, and multiplication requires
        // division by ONE_20.
        let z = ((a - Self::one_20()) * Self::one_20()) / (a + Self::one_20());
        let z_squared = (z * z) / Self::one_20();

        // num is the numerator of the series: the z^(2 * n + 1) term
        let mut num = z;

        // seriesSum holds the accumulated sum of each term in the series, starting with the initial z
        let mut series_sum = num;

        // In each step, the numerator is multiplied by z^2
        num = (num * z_squared) / Self::one_20();
        series_sum += num / I256::from_raw(U256::from(3));

        num = (num * z_squared) / Self::one_20();
        series_sum += num / I256::from_raw(U256::from(5));

        num = (num * z_squared) / Self::one_20();
        series_sum += num / I256::from_raw(U256::from(7));

        num = (num * z_squared) / Self::one_20();
        series_sum += num / I256::from_raw(U256::from(9));

        num = (num * z_squared) / Self::one_20();
        series_sum += num / I256::from_raw(U256::from(11));

        // 6 Taylor terms are sufficient for 36 decimal precision.

        // Finally, we multiply by 2 (non fixed point) to compute ln(remainder)
        series_sum *= I256::from_raw(U256::from(2));

        // We now have the sum of all x_n present, and the Taylor approximation of the logarithm of the remainder (both
        // with 20 decimals). All that remains is to sum these two, and then drop two digits to return a 18 decimal
        // value.

        (sum + series_sum) / I256::from_raw(U256::from(100))
    }

    fn _ln_36(x: I256) -> I256 {
        let x = x * Self::one_18();

        let z = ((x - Self::one_36()) * Self::one_36()) / (x + Self::one_36());
        let z_squared = (z * z) / Self::one_36();

        let mut num = z;
        let mut series_sum = num;

        for n in 1..=7 {
            num = (num * z_squared) / Self::one_36();
            series_sum += num / I256::from_raw(U256::from(2 * n + 1));
        }

        series_sum * I256::from_raw(U256::from(2))
    }
}
