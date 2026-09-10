//! Pure integer arithmetic for the in-program Amoeba DLMM.
//!
//! This module deliberately knows nothing about Solana accounts, token programs,
//! clocks, or Light.  The on-chain processor and off-chain parity fixtures use
//! the same deterministic formulas defined here.

use core::cmp::min;

pub const AMOEBA_DLMM_PRICE_SCALE: u64 = 1_000_000;
pub const AMOEBA_DLMM_BPS_SCALE: u64 = 10_000;
pub const AMOEBA_DLMM_BINS_PER_PAGE: u16 = 32;
pub use crate::constants::MAX_AMOEBA_DLMM_BINS_PER_SWAP as AMOEBA_DLMM_MAXIMUM_BINS_PER_SWAP;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmoebaDlmmMathError {
    ArithmeticOverflow,
    DivisionByZero,
    InvalidAmount,
    InvalidBin,
    InvalidFee,
    InvalidLiquidity,
    InvalidRoute,
    InsufficientLiquidity,
    MinimumOutputNotMet,
    PriceLimitExceeded,
    TooManyBins,
}

pub type MathResult<T> = Result<T, AmoebaDlmmMathError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AmoebaDlmmSwapDirection {
    QuoteForOption = 0,
    OptionForQuote = 1,
}

impl TryFrom<u8> for AmoebaDlmmSwapDirection {
    type Error = AmoebaDlmmMathError;

    fn try_from(value: u8) -> MathResult<Self> {
        match value {
            0 => Ok(Self::QuoteForOption),
            1 => Ok(Self::OptionForQuote),
            _ => Err(AmoebaDlmmMathError::InvalidRoute),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AmoebaDlmmFeeBreakdown {
    pub total_fee: u64,
    pub protocol_fee: u64,
    pub lp_fee: u64,
    pub trade_input: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AmoebaDlmmBinLiquidity {
    pub bin_id: u16,
    pub option_reserve: u64,
    pub quote_reserve: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AmoebaDlmmBinFill {
    pub bin_id: u16,
    pub trade_input: u64,
    pub amount_out: u64,
    pub lp_fee: u64,
    pub option_reserve_after: u64,
    pub quote_reserve_after: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AmoebaDlmmSwapQuote {
    pub amount_in: u64,
    pub amount_out: u64,
    pub total_fee: u64,
    pub protocol_fee: u64,
    pub lp_fee: u64,
    pub first_bin_id: u16,
    pub last_bin_id: u16,
    pub fills: Vec<AmoebaDlmmBinFill>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AmoebaDlmmShareDeposit {
    pub option_amount: u64,
    pub quote_amount: u64,
    pub minted_shares: u128,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AmoebaDlmmShareWithdrawal {
    pub option_amount: u64,
    pub quote_amount: u64,
}

#[inline]
pub fn floor_mul_div(a: u64, b: u64, denominator: u64) -> MathResult<u64> {
    if denominator == 0 {
        return Err(AmoebaDlmmMathError::DivisionByZero);
    }
    let value = (a as u128)
        .checked_mul(b as u128)
        .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?
        / denominator as u128;
    u64::try_from(value).map_err(|_| AmoebaDlmmMathError::ArithmeticOverflow)
}

#[inline]
pub fn ceil_mul_div(a: u64, b: u64, denominator: u64) -> MathResult<u64> {
    if denominator == 0 {
        return Err(AmoebaDlmmMathError::DivisionByZero);
    }
    let product = (a as u128)
        .checked_mul(b as u128)
        .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?;
    let value = product
        .checked_add(denominator as u128 - 1)
        .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?
        / denominator as u128;
    u64::try_from(value).map_err(|_| AmoebaDlmmMathError::ArithmeticOverflow)
}

#[inline]
pub fn floor_mul_div_u128(a: u128, b: u128, denominator: u128) -> MathResult<u128> {
    if denominator == 0 {
        return Err(AmoebaDlmmMathError::DivisionByZero);
    }
    a.checked_mul(b)
        .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)
        .map(|value| value / denominator)
}

#[inline]
pub fn ceil_mul_div_u128(a: u128, b: u128, denominator: u128) -> MathResult<u128> {
    if denominator == 0 {
        return Err(AmoebaDlmmMathError::DivisionByZero);
    }
    a.checked_mul(b)
        .and_then(|value| value.checked_add(denominator - 1))
        .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)
        .map(|value| value / denominator)
}

#[inline]
pub fn bin_to_page(bin_id: u16) -> MathResult<(u16, u8)> {
    if bin_id == 0 {
        return Err(AmoebaDlmmMathError::InvalidBin);
    }
    let zero_based = bin_id - 1;
    Ok((
        zero_based / AMOEBA_DLMM_BINS_PER_PAGE,
        (zero_based % AMOEBA_DLMM_BINS_PER_PAGE) as u8,
    ))
}

#[inline]
pub fn page_first_bin(page_index: u16) -> MathResult<u16> {
    page_index
        .checked_mul(AMOEBA_DLMM_BINS_PER_PAGE)
        .and_then(|value| value.checked_add(1))
        .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)
}

#[inline]
pub fn price_from_bin(
    tick_size_quote_atomic: u64,
    maximum_bin_id: u16,
    bin_id: u16,
) -> MathResult<u64> {
    if tick_size_quote_atomic == 0 || bin_id == 0 || bin_id > maximum_bin_id {
        return Err(AmoebaDlmmMathError::InvalidBin);
    }
    tick_size_quote_atomic
        .checked_mul(bin_id as u64)
        .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)
}

#[inline]
pub fn calculate_fees(
    amount_in: u64,
    swap_fee_bps: u16,
    protocol_fee_share_bps: u16,
) -> MathResult<AmoebaDlmmFeeBreakdown> {
    if amount_in == 0 {
        return Err(AmoebaDlmmMathError::InvalidAmount);
    }
    if swap_fee_bps as u64 > AMOEBA_DLMM_BPS_SCALE
        || protocol_fee_share_bps as u64 > AMOEBA_DLMM_BPS_SCALE
    {
        return Err(AmoebaDlmmMathError::InvalidFee);
    }
    let total_fee = ceil_mul_div(amount_in, swap_fee_bps as u64, AMOEBA_DLMM_BPS_SCALE)?;
    let trade_input = amount_in
        .checked_sub(total_fee)
        .ok_or(AmoebaDlmmMathError::InvalidAmount)?;
    if trade_input == 0 {
        return Err(AmoebaDlmmMathError::InvalidAmount);
    }
    let protocol_fee = floor_mul_div(
        total_fee,
        protocol_fee_share_bps as u64,
        AMOEBA_DLMM_BPS_SCALE,
    )?;
    Ok(AmoebaDlmmFeeBreakdown {
        total_fee,
        protocol_fee,
        lp_fee: total_fee
            .checked_sub(protocol_fee)
            .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?,
        trade_input,
    })
}

fn validate_route_bin(
    direction: AmoebaDlmmSwapDirection,
    previous_bin: Option<u16>,
    current_bin: u16,
    limit_bin_id: u16,
) -> MathResult<()> {
    if current_bin == 0 {
        return Err(AmoebaDlmmMathError::InvalidBin);
    }
    match direction {
        AmoebaDlmmSwapDirection::QuoteForOption => {
            if current_bin > limit_bin_id {
                return Err(AmoebaDlmmMathError::PriceLimitExceeded);
            }
            if previous_bin.is_some_and(|previous| current_bin <= previous) {
                return Err(AmoebaDlmmMathError::InvalidRoute);
            }
        }
        AmoebaDlmmSwapDirection::OptionForQuote => {
            if current_bin < limit_bin_id {
                return Err(AmoebaDlmmMathError::PriceLimitExceeded);
            }
            if previous_bin.is_some_and(|previous| current_bin >= previous) {
                return Err(AmoebaDlmmMathError::InvalidRoute);
            }
        }
    }
    Ok(())
}

/// Shared per-bin arithmetic after the one swap-level fee has been removed.
/// The caller allocates LP fees once across the final executed route.
pub fn fill_bin_exact_input_without_fee(
    direction: AmoebaDlmmSwapDirection,
    remaining: u64,
    price: u64,
    bin: &AmoebaDlmmBinLiquidity,
) -> MathResult<AmoebaDlmmBinFill> {
    if remaining == 0 || price == 0 {
        return Err(AmoebaDlmmMathError::InvalidAmount);
    }
    let (input, output, options, quote) = match direction {
        AmoebaDlmmSwapDirection::QuoteForOption => {
            if bin.option_reserve == 0 {
                return Err(AmoebaDlmmMathError::InvalidRoute);
            }
            let full = ceil_mul_div(bin.option_reserve, price, AMOEBA_DLMM_PRICE_SCALE)?;
            let input = min(remaining, full);
            let output = if remaining >= full {
                bin.option_reserve
            } else {
                floor_mul_div(remaining, AMOEBA_DLMM_PRICE_SCALE, price)?
            };
            (
                input,
                output,
                bin.option_reserve.checked_sub(output),
                bin.quote_reserve.checked_add(input),
            )
        }
        AmoebaDlmmSwapDirection::OptionForQuote => {
            if bin.quote_reserve == 0 {
                return Err(AmoebaDlmmMathError::InvalidRoute);
            }
            let full = ceil_mul_div(bin.quote_reserve, AMOEBA_DLMM_PRICE_SCALE, price)?;
            let input = min(remaining, full);
            let output = if remaining >= full {
                bin.quote_reserve
            } else {
                floor_mul_div(remaining, price, AMOEBA_DLMM_PRICE_SCALE)?
            };
            (
                input,
                output,
                bin.option_reserve.checked_add(input),
                bin.quote_reserve.checked_sub(output),
            )
        }
    };
    Ok(AmoebaDlmmBinFill {
        bin_id: bin.bin_id,
        trade_input: input,
        amount_out: output,
        lp_fee: 0,
        option_reserve_after: options.ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?,
        quote_reserve_after: quote.ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?,
    })
}

/// Quote a complete exact-input swap over the canonical nonempty bins supplied
/// in traversal order.  Failure never returns a partial quote.
#[allow(clippy::too_many_arguments)]
pub fn quote_exact_in(
    direction: AmoebaDlmmSwapDirection,
    amount_in: u64,
    minimum_amount_out: u64,
    limit_bin_id: u16,
    tick_size_quote_atomic: u64,
    maximum_bin_id: u16,
    swap_fee_bps: u16,
    protocol_fee_share_bps: u16,
    maximum_bins: u8,
    bins: &[AmoebaDlmmBinLiquidity],
) -> MathResult<AmoebaDlmmSwapQuote> {
    if minimum_amount_out == 0 || maximum_bins == 0 || bins.is_empty() {
        return Err(AmoebaDlmmMathError::InvalidAmount);
    }
    if maximum_bins > AMOEBA_DLMM_MAXIMUM_BINS_PER_SWAP {
        return Err(AmoebaDlmmMathError::TooManyBins);
    }
    if bins.len() > maximum_bins as usize {
        return Err(AmoebaDlmmMathError::TooManyBins);
    }

    let fees = calculate_fees(amount_in, swap_fee_bps, protocol_fee_share_bps)?;
    let mut remaining = fees.trade_input;
    let mut amount_out = 0u64;
    let mut fills = Vec::with_capacity(bins.len());
    let mut previous_bin = None;

    for bin in bins {
        validate_route_bin(direction, previous_bin, bin.bin_id, limit_bin_id)?;
        previous_bin = Some(bin.bin_id);
        let price = price_from_bin(tick_size_quote_atomic, maximum_bin_id, bin.bin_id)?;

        let (consumed, output, option_after, quote_after) = match direction {
            AmoebaDlmmSwapDirection::QuoteForOption => {
                if bin.option_reserve == 0 {
                    return Err(AmoebaDlmmMathError::InvalidRoute);
                }
                let full_input = ceil_mul_div(bin.option_reserve, price, AMOEBA_DLMM_PRICE_SCALE)?;
                if remaining >= full_input {
                    (
                        full_input,
                        bin.option_reserve,
                        0,
                        bin.quote_reserve
                            .checked_add(full_input)
                            .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?,
                    )
                } else {
                    let output = floor_mul_div(remaining, AMOEBA_DLMM_PRICE_SCALE, price)?;
                    (
                        remaining,
                        output,
                        bin.option_reserve
                            .checked_sub(output)
                            .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?,
                        bin.quote_reserve
                            .checked_add(remaining)
                            .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?,
                    )
                }
            }
            AmoebaDlmmSwapDirection::OptionForQuote => {
                if bin.quote_reserve == 0 {
                    return Err(AmoebaDlmmMathError::InvalidRoute);
                }
                let full_input = ceil_mul_div(bin.quote_reserve, AMOEBA_DLMM_PRICE_SCALE, price)?;
                if remaining >= full_input {
                    (
                        full_input,
                        bin.quote_reserve,
                        bin.option_reserve
                            .checked_add(full_input)
                            .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?,
                        0,
                    )
                } else {
                    let output = floor_mul_div(remaining, price, AMOEBA_DLMM_PRICE_SCALE)?;
                    (
                        remaining,
                        output,
                        bin.option_reserve
                            .checked_add(remaining)
                            .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?,
                        bin.quote_reserve
                            .checked_sub(output)
                            .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?,
                    )
                }
            }
        };

        if consumed == 0 {
            return Err(AmoebaDlmmMathError::InvalidLiquidity);
        }
        remaining = remaining
            .checked_sub(consumed)
            .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?;
        amount_out = amount_out
            .checked_add(output)
            .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?;
        fills.push(AmoebaDlmmBinFill {
            bin_id: bin.bin_id,
            trade_input: consumed,
            amount_out: output,
            lp_fee: 0,
            option_reserve_after: option_after,
            quote_reserve_after: quote_after,
        });
        if remaining == 0 {
            break;
        }
    }

    if remaining != 0 {
        return Err(AmoebaDlmmMathError::InsufficientLiquidity);
    }
    if amount_out < minimum_amount_out {
        return Err(AmoebaDlmmMathError::MinimumOutputNotMet);
    }

    let final_index = fills
        .len()
        .checked_sub(1)
        .ok_or(AmoebaDlmmMathError::InsufficientLiquidity)?;
    let mut allocated_lp_fee = 0u64;
    for (index, fill) in fills.iter_mut().enumerate() {
        let allocation = if index == final_index {
            fees.lp_fee
                .checked_sub(allocated_lp_fee)
                .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?
        } else {
            floor_mul_div(fill.trade_input, fees.lp_fee, fees.trade_input)?
        };
        allocated_lp_fee = allocated_lp_fee
            .checked_add(allocation)
            .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?;
        fill.lp_fee = allocation;
        match direction {
            AmoebaDlmmSwapDirection::QuoteForOption => {
                fill.quote_reserve_after = fill
                    .quote_reserve_after
                    .checked_add(allocation)
                    .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?;
            }
            AmoebaDlmmSwapDirection::OptionForQuote => {
                fill.option_reserve_after = fill
                    .option_reserve_after
                    .checked_add(allocation)
                    .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?;
            }
        }
    }

    Ok(AmoebaDlmmSwapQuote {
        amount_in,
        amount_out,
        total_fee: fees.total_fee,
        protocol_fee: fees.protocol_fee,
        lp_fee: fees.lp_fee,
        first_bin_id: fills[0].bin_id,
        last_bin_id: fills[final_index].bin_id,
        fills,
    })
}

/// Calculate the exact amounts consumed and shares minted for a single bin.
pub fn calculate_share_deposit(
    option_reserve: u64,
    quote_reserve: u64,
    total_shares: u128,
    maximum_option_amount: u64,
    maximum_quote_amount: u64,
    price_quote_atomic: u64,
) -> MathResult<AmoebaDlmmShareDeposit> {
    if price_quote_atomic == 0 {
        return Err(AmoebaDlmmMathError::InvalidBin);
    }
    if total_shares == 0 {
        if option_reserve != 0 || quote_reserve != 0 {
            return Err(AmoebaDlmmMathError::InvalidLiquidity);
        }
        let option_value = floor_mul_div(
            maximum_option_amount,
            price_quote_atomic,
            AMOEBA_DLMM_PRICE_SCALE,
        )?;
        let value = maximum_quote_amount
            .checked_add(option_value)
            .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?;
        if value == 0 {
            return Err(AmoebaDlmmMathError::InvalidLiquidity);
        }
        return Ok(AmoebaDlmmShareDeposit {
            option_amount: maximum_option_amount,
            quote_amount: maximum_quote_amount,
            minted_shares: value as u128,
        });
    }

    let (minted_shares, option_amount, quote_amount) = match (option_reserve > 0, quote_reserve > 0)
    {
        (true, true) => {
            let option_shares = floor_mul_div_u128(
                maximum_option_amount as u128,
                total_shares,
                option_reserve as u128,
            )?;
            let quote_shares = floor_mul_div_u128(
                maximum_quote_amount as u128,
                total_shares,
                quote_reserve as u128,
            )?;
            let shares = min(option_shares, quote_shares);
            let option = ceil_mul_div_u128(shares, option_reserve as u128, total_shares)?;
            let quote = ceil_mul_div_u128(shares, quote_reserve as u128, total_shares)?;
            (
                shares,
                u64::try_from(option).map_err(|_| AmoebaDlmmMathError::ArithmeticOverflow)?,
                u64::try_from(quote).map_err(|_| AmoebaDlmmMathError::ArithmeticOverflow)?,
            )
        }
        (true, false) => {
            if maximum_quote_amount != 0 {
                return Err(AmoebaDlmmMathError::InvalidLiquidity);
            }
            let shares = floor_mul_div_u128(
                maximum_option_amount as u128,
                total_shares,
                option_reserve as u128,
            )?;
            let option = ceil_mul_div_u128(shares, option_reserve as u128, total_shares)?;
            (
                shares,
                u64::try_from(option).map_err(|_| AmoebaDlmmMathError::ArithmeticOverflow)?,
                0,
            )
        }
        (false, true) => {
            if maximum_option_amount != 0 {
                return Err(AmoebaDlmmMathError::InvalidLiquidity);
            }
            let shares = floor_mul_div_u128(
                maximum_quote_amount as u128,
                total_shares,
                quote_reserve as u128,
            )?;
            let quote = ceil_mul_div_u128(shares, quote_reserve as u128, total_shares)?;
            (
                shares,
                0,
                u64::try_from(quote).map_err(|_| AmoebaDlmmMathError::ArithmeticOverflow)?,
            )
        }
        (false, false) => return Err(AmoebaDlmmMathError::InvalidLiquidity),
    };

    if minted_shares == 0
        || option_amount > maximum_option_amount
        || quote_amount > maximum_quote_amount
    {
        return Err(AmoebaDlmmMathError::InvalidLiquidity);
    }
    Ok(AmoebaDlmmShareDeposit {
        option_amount,
        quote_amount,
        minted_shares,
    })
}

pub fn calculate_share_withdrawal(
    option_reserve: u64,
    quote_reserve: u64,
    total_shares: u128,
    shares_to_burn: u128,
) -> MathResult<AmoebaDlmmShareWithdrawal> {
    if shares_to_burn == 0 || total_shares == 0 || shares_to_burn > total_shares {
        return Err(AmoebaDlmmMathError::InvalidLiquidity);
    }
    if shares_to_burn == total_shares {
        return Ok(AmoebaDlmmShareWithdrawal {
            option_amount: option_reserve,
            quote_amount: quote_reserve,
        });
    }
    let option = floor_mul_div_u128(shares_to_burn, option_reserve as u128, total_shares)?;
    let quote = floor_mul_div_u128(shares_to_burn, quote_reserve as u128, total_shares)?;
    Ok(AmoebaDlmmShareWithdrawal {
        option_amount: u64::try_from(option)
            .map_err(|_| AmoebaDlmmMathError::ArithmeticOverflow)?,
        quote_amount: u64::try_from(quote).map_err(|_| AmoebaDlmmMathError::ArithmeticOverflow)?,
    })
}

#[inline]
pub fn refresh_local_liquidity_bits(
    option_reserve: &[u64; 32],
    quote_reserve: &[u64; 32],
) -> (u32, u32) {
    let mut bid_bitmap = 0u32;
    let mut ask_bitmap = 0u32;
    for index in 0..32 {
        if quote_reserve[index] > 0 {
            bid_bitmap |= 1u32 << index;
        }
        if option_reserve[index] > 0 {
            ask_bitmap |= 1u32 << index;
        }
    }
    (bid_bitmap, ask_bitmap)
}

#[inline]
pub fn lowest_set_bit(bitmap: u32) -> Option<u8> {
    (bitmap != 0).then(|| bitmap.trailing_zeros() as u8)
}

#[inline]
pub fn highest_set_bit(bitmap: u32) -> Option<u8> {
    (bitmap != 0).then(|| (31 - bitmap.leading_zeros()) as u8)
}
