//! One price-ordered route over ordinary and segregated writer liquidity.
//! Ordinary liquidity wins ties. Writer admission precedes each tentative fill;
//! a rejected writer candidate consumes neither trader input nor ordinary reserves.
use crate::ameba_dlmm_math::{
    self as dlmm, AmoebaDlmmBinFill, AmoebaDlmmBinLiquidity, AmoebaDlmmMathError,
    AmoebaDlmmSwapDirection, AmoebaDlmmSwapQuote,
};
use crate::state::WriterDlmmBinV1;
use crate::writer_dlmm_math::{
    admit_writer_dlmm_cash, admit_writer_dlmm_retirement, writer_dlmm_price_bounds,
    writer_dlmm_primary_fee, WriterDlmmBuybackLimits, WriterDlmmCash, WriterDlmmRetirement,
    WriterDlmmRiskLimits, WriterDlmmSeriesLimits,
};
use crate::writer_sleeve_math::WriterSeries;

#[derive(Clone, Copy)]
pub struct WriterDlmmRouteConfig {
    pub direction: AmoebaDlmmSwapDirection,
    pub amount_in: u64,
    pub minimum_amount_out: u64,
    pub limit_bin_id: u16,
    pub tick_size_quote_atomic: u64,
    pub maximum_bin_id: u16,
    pub swap_fee_bps: u16,
    pub protocol_fee_share_bps: u16,
    pub maximum_bins: u8,
    /// Earliest possible ordinary price omitted from the canonical page prefix.
    pub unloaded_ordinary_boundary: Option<u16>,
}

pub struct WriterDlmmSwapPolicy<'a> {
    pub eligible: bool,
    pub book: &'a [WriterSeries],
    pub series_index: usize,
    pub cash: WriterDlmmCash,
    pub risk: WriterDlmmRiskLimits,
    pub buyback: WriterDlmmBuybackLimits,
    pub series_limits: &'a [WriterDlmmSeriesLimits],
    pub month_spent_atoms: u64,
    pub series_month_spent_atoms: u64,
    pub primary_fee_bps: u16,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WriterDlmmFillTotals {
    pub gross_premium_atoms: u64,
    pub primary_fee_atoms: u64,
    pub net_premium_atoms: u64,
    pub lp_fee_atoms: u64,
    pub sold_option_atoms: u64,
    pub retired_option_atoms: u64,
    pub spent_quote_atoms: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WriterDlmmRouteQuote {
    pub quote: AmoebaDlmmSwapQuote,
    pub ordinary_fills: Vec<AmoebaDlmmBinFill>,
    /// Writer after-reserves exclude swept quote and atomically retired option claims.
    pub writer_fills: Vec<AmoebaDlmmBinFill>,
    pub writer: WriterDlmmFillTotals,
}

fn checked_add(a: u64, b: u64) -> Result<u64, AmoebaDlmmMathError> {
    a.checked_add(b)
        .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)
}
fn remaining_budget(policy: &WriterDlmmSwapPolicy, totals: &WriterDlmmFillTotals) -> u64 {
    let terms = &policy.series_limits[policy.series_index];
    policy
        .buyback
        .transaction_buyback_cap_atoms
        .saturating_sub(totals.spent_quote_atoms)
        .min(
            terms
                .transaction_buyback_cap_atoms
                .saturating_sub(totals.spent_quote_atoms),
        )
        .min(
            policy
                .buyback
                .monthly_buyback_cap_atoms
                .saturating_sub(policy.month_spent_atoms)
                .saturating_sub(totals.spent_quote_atoms),
        )
        .min(
            terms
                .monthly_buyback_cap_atoms
                .saturating_sub(policy.series_month_spent_atoms)
                .saturating_sub(totals.spent_quote_atoms),
        )
}

fn admit_totals(
    config: &WriterDlmmRouteConfig,
    policy: &WriterDlmmSwapPolicy,
    totals: &mut WriterDlmmFillTotals,
) -> bool {
    if !policy.eligible
        || policy.series_index >= policy.book.len()
        || policy.book.len() != policy.series_limits.len()
    {
        return false;
    }
    let terms = policy.series_limits[policy.series_index];
    match config.direction {
        AmoebaDlmmSwapDirection::QuoteForOption => {
            let Ok(fee) =
                writer_dlmm_primary_fee(totals.gross_premium_atoms, policy.primary_fee_bps)
            else {
                return false;
            };
            let Some(net) = totals.gross_premium_atoms.checked_sub(fee) else {
                return false;
            };
            let Ok(floor) = dlmm::ceil_mul_div(
                totals.sold_option_atoms,
                terms.seller_floor_quote_atoms,
                dlmm::AMOEBA_DLMM_PRICE_SCALE,
            ) else {
                return false;
            };
            if net < floor {
                return false;
            }
            let Some(assets) = policy
                .cash
                .assets_atoms
                .checked_add(net)
                .and_then(|value| value.checked_add(totals.lp_fee_atoms))
            else {
                return false;
            };
            let mut book = policy.book.to_vec();
            let Some(oi) = book[policy.series_index]
                .external_oi_atoms
                .checked_add(totals.sold_option_atoms)
            else {
                return false;
            };
            book[policy.series_index].external_oi_atoms = oi;
            if admit_writer_dlmm_cash(
                &book,
                WriterDlmmCash {
                    assets_atoms: assets,
                    ..policy.cash
                },
                &policy.risk,
            )
            .is_err()
            {
                return false;
            }
            totals.primary_fee_atoms = fee;
            totals.net_premium_atoms = net;
            true
        }
        AmoebaDlmmSwapDirection::OptionForQuote => {
            let Some(allocated_after) = policy
                .cash
                .allocated_lp_quote_atoms
                .checked_sub(totals.spent_quote_atoms)
            else {
                return false;
            };
            admit_writer_dlmm_retirement(
                policy.book,
                policy.cash,
                &policy.risk,
                &policy.buyback,
                policy.series_limits,
                &[WriterDlmmRetirement {
                    series_index: policy.series_index,
                    retired_atoms: totals.retired_option_atoms,
                    cost_atoms: totals.spent_quote_atoms,
                    series_month_spent_atoms: policy.series_month_spent_atoms,
                }],
                policy.month_spent_atoms,
                allocated_after,
                false,
            )
            .is_ok()
        }
    }
}

fn allocate_fee(
    input: u64,
    remaining: u64,
    trade_input: u64,
    total_lp_fee: u64,
    allocated: u64,
) -> Result<u64, AmoebaDlmmMathError> {
    if input == remaining {
        total_lp_fee
            .checked_sub(allocated)
            .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)
    } else {
        dlmm::floor_mul_div(input, total_lp_fee, trade_input)
    }
}

/// A bounded writer candidate is clipped to remaining fixed cash budgets first, then
/// independently admitted against the full current book. If that candidate cannot pass,
/// this route skips it; it never assumes a hypothetical later or other-series retirement.
pub fn quote_writer_dlmm_exact_in(
    config: WriterDlmmRouteConfig,
    ordinary: &[AmoebaDlmmBinLiquidity],
    writer: &[WriterDlmmBinV1],
    policy: Option<&WriterDlmmSwapPolicy>,
) -> Result<WriterDlmmRouteQuote, AmoebaDlmmMathError> {
    if config.maximum_bins == 0
        || config.maximum_bins > dlmm::AMOEBA_DLMM_MAXIMUM_BINS_PER_SWAP
        || config.minimum_amount_out == 0
        || writer.len() > crate::state::WRITER_DLMM_POSITION_BINS
        || ordinary.len() > 32 * usize::from(crate::constants::MAX_AMOEBA_DLMM_PAGE_HOPS_PER_SWAP)
    {
        return Err(AmoebaDlmmMathError::InvalidRoute);
    }
    if let Some(policy) = policy {
        if policy.series_index >= policy.book.len()
            || policy.book.len() != policy.series_limits.len()
        {
            return Err(AmoebaDlmmMathError::InvalidRoute);
        }
    } else if !writer.is_empty() {
        return Err(AmoebaDlmmMathError::InvalidRoute);
    }
    let ascending = config.direction == AmoebaDlmmSwapDirection::QuoteForOption;
    if ordinary.windows(2).any(|pair| {
        if ascending {
            pair[0].bin_id >= pair[1].bin_id
        } else {
            pair[0].bin_id <= pair[1].bin_id
        }
    }) || writer
        .windows(2)
        .any(|pair| pair[0].bin_id >= pair[1].bin_id)
    {
        return Err(AmoebaDlmmMathError::InvalidRoute);
    }
    let fees = dlmm::calculate_fees(
        config.amount_in,
        config.swap_fee_bps,
        config.protocol_fee_share_bps,
    )?;
    let mut result = WriterDlmmRouteQuote::default();
    let mut remaining = fees.trade_input;
    let mut allocated = 0u64;
    let mut ordinary_index = 0;
    let mut writer_cursor = 0;
    while remaining > 0 && (ordinary_index < ordinary.len() || writer_cursor < writer.len()) {
        let ordinary_bin = ordinary.get(ordinary_index);
        let writer_index = if ascending {
            writer_cursor
        } else {
            writer.len().saturating_sub(writer_cursor + 1)
        };
        let writer_bin = (writer_cursor < writer.len()).then(|| &writer[writer_index]);
        let bin_id = match (ordinary_bin, writer_bin) {
            (Some(a), Some(b)) => {
                if ascending {
                    a.bin_id.min(b.bin_id)
                } else {
                    a.bin_id.max(b.bin_id)
                }
            }
            (Some(a), None) => a.bin_id,
            (None, Some(b)) => b.bin_id,
            _ => break,
        };
        if (ascending && bin_id > config.limit_bin_id)
            || (!ascending && bin_id < config.limit_bin_id)
        {
            break;
        }
        if config.unloaded_ordinary_boundary.is_some_and(|boundary| {
            if ascending {
                bin_id >= boundary
            } else {
                bin_id <= boundary
            }
        }) {
            return Err(AmoebaDlmmMathError::InvalidRoute);
        }
        let price =
            dlmm::price_from_bin(config.tick_size_quote_atomic, config.maximum_bin_id, bin_id)?;
        let ordinary_here = ordinary_bin.filter(|bin| bin.bin_id == bin_id);
        let writer_here = writer_bin.filter(|bin| bin.bin_id == bin_id);
        let mut ordinary_after = ordinary_here.copied().unwrap_or(AmoebaDlmmBinLiquidity {
            bin_id,
            ..AmoebaDlmmBinLiquidity::default()
        });
        let mut writer_after = writer_here.copied().unwrap_or(WriterDlmmBinV1 {
            bin_id,
            ..WriterDlmmBinV1::default()
        });
        let mut combined = AmoebaDlmmBinFill {
            bin_id,
            ..AmoebaDlmmBinFill::default()
        };
        if let Some(bin) = ordinary_here {
            if result.quote.fills.len() == usize::from(config.maximum_bins) {
                break;
            }
            let mut fill =
                dlmm::fill_bin_exact_input_without_fee(config.direction, remaining, price, bin)?;
            fill.lp_fee = allocate_fee(
                fill.trade_input,
                remaining,
                fees.trade_input,
                fees.lp_fee,
                allocated,
            )?;
            match config.direction {
                AmoebaDlmmSwapDirection::QuoteForOption => {
                    fill.quote_reserve_after = checked_add(fill.quote_reserve_after, fill.lp_fee)?
                }
                AmoebaDlmmSwapDirection::OptionForQuote => {
                    fill.option_reserve_after = checked_add(fill.option_reserve_after, fill.lp_fee)?
                }
            }
            ordinary_after.option_reserve = fill.option_reserve_after;
            ordinary_after.quote_reserve = fill.quote_reserve_after;
            remaining = remaining
                .checked_sub(fill.trade_input)
                .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?;
            allocated = checked_add(allocated, fill.lp_fee)?;
            combined.trade_input = fill.trade_input;
            combined.amount_out = fill.amount_out;
            combined.lp_fee = fill.lp_fee;
            result.ordinary_fills.push(fill);
        }
        if remaining > 0 {
            if let (Some(bin), Some(policy)) = (writer_here, policy) {
                let relevant_reserve = if ascending {
                    bin.option_atoms
                } else {
                    bin.quote_atoms
                        .min(remaining_budget(policy, &result.writer))
                };
                if relevant_reserve > 0
                    && policy.eligible
                    && (combined.trade_input > 0
                        || result.quote.fills.len() < usize::from(config.maximum_bins))
                {
                    let bounds = writer_dlmm_price_bounds(
                        policy.series_limits[policy.series_index].seller_floor_quote_atoms,
                        config.tick_size_quote_atomic,
                        policy.buyback.price_separation_ticks,
                        config.swap_fee_bps,
                        policy.primary_fee_bps,
                    );
                    let price_eligible = bounds.is_ok_and(|(ask, bid, _)| {
                        if ascending {
                            price >= ask
                        } else {
                            price <= bid
                        }
                    });
                    if price_eligible {
                        let temporary = AmoebaDlmmBinLiquidity {
                            bin_id,
                            option_reserve: if ascending { relevant_reserve } else { 0 },
                            quote_reserve: if ascending { 0 } else { relevant_reserve },
                        };
                        let mut fill = dlmm::fill_bin_exact_input_without_fee(
                            config.direction,
                            remaining,
                            price,
                            &temporary,
                        )?;
                        fill.lp_fee = allocate_fee(
                            fill.trade_input,
                            remaining,
                            fees.trade_input,
                            fees.lp_fee,
                            allocated,
                        )?;
                        let mut totals = result.writer;
                        totals.lp_fee_atoms = checked_add(totals.lp_fee_atoms, fill.lp_fee)?;
                        match config.direction {
                            AmoebaDlmmSwapDirection::QuoteForOption => {
                                totals.gross_premium_atoms =
                                    checked_add(totals.gross_premium_atoms, fill.trade_input)?;
                                totals.sold_option_atoms =
                                    checked_add(totals.sold_option_atoms, fill.amount_out)?;
                            }
                            AmoebaDlmmSwapDirection::OptionForQuote => {
                                totals.retired_option_atoms = checked_add(
                                    totals.retired_option_atoms,
                                    checked_add(fill.trade_input, fill.lp_fee)?,
                                )?;
                                totals.spent_quote_atoms =
                                    checked_add(totals.spent_quote_atoms, fill.amount_out)?;
                            }
                        }
                        if fill.amount_out > 0 && admit_totals(&config, policy, &mut totals) {
                            if ascending {
                                writer_after.option_atoms = writer_after
                                    .option_atoms
                                    .checked_sub(fill.amount_out)
                                    .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?;
                            } else {
                                writer_after.quote_atoms = writer_after
                                    .quote_atoms
                                    .checked_sub(fill.amount_out)
                                    .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?;
                            }
                            fill.option_reserve_after = writer_after.option_atoms;
                            fill.quote_reserve_after = writer_after.quote_atoms;
                            remaining = remaining
                                .checked_sub(fill.trade_input)
                                .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?;
                            allocated = checked_add(allocated, fill.lp_fee)?;
                            combined.trade_input =
                                checked_add(combined.trade_input, fill.trade_input)?;
                            combined.amount_out =
                                checked_add(combined.amount_out, fill.amount_out)?;
                            combined.lp_fee = checked_add(combined.lp_fee, fill.lp_fee)?;
                            result.writer = totals;
                            result.writer_fills.push(fill);
                        }
                    }
                }
            }
        }
        if combined.trade_input > 0 {
            combined.option_reserve_after =
                checked_add(ordinary_after.option_reserve, writer_after.option_atoms)?;
            combined.quote_reserve_after =
                checked_add(ordinary_after.quote_reserve, writer_after.quote_atoms)?;
            result.quote.amount_out = checked_add(result.quote.amount_out, combined.amount_out)?;
            result.quote.fills.push(combined);
        }
        if ordinary_here.is_some() {
            ordinary_index += 1;
        }
        if writer_here.is_some() {
            writer_cursor += 1;
        }
    }
    if remaining != 0 {
        return Err(AmoebaDlmmMathError::InsufficientLiquidity);
    }
    if result.quote.amount_out < config.minimum_amount_out {
        return Err(AmoebaDlmmMathError::MinimumOutputNotMet);
    }
    if allocated != fees.lp_fee || result.quote.fills.is_empty() {
        return Err(AmoebaDlmmMathError::InvalidRoute);
    }
    result.quote.amount_in = config.amount_in;
    result.quote.total_fee = fees.total_fee;
    result.quote.protocol_fee = fees.protocol_fee;
    result.quote.lp_fee = fees.lp_fee;
    result.quote.first_bin_id = result.quote.fills[0].bin_id;
    result.quote.last_bin_id = result
        .quote
        .fills
        .last()
        .ok_or(AmoebaDlmmMathError::InvalidRoute)?
        .bin_id;
    Ok(result)
}
