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
    WriterDlmmBuybackLimits, WriterDlmmCash, WriterDlmmRetirement, WriterDlmmRiskLimits,
    WriterDlmmSeriesLimits,
};
use crate::writer_sleeve_math::{WriterReserveSummary, WriterSeries, WRITER_MAX_SERIES};

/// Full candidate, six geometric reductions, then the smallest input. This is
/// a bounded feasibility search, not a claim of monotonicity or maximum size.
pub const MAX_WRITER_ADMISSION_PROBES: usize = 8;

#[derive(Clone, Copy)]
pub struct WriterDlmmRouteConfig {
    pub direction: AmoebaDlmmSwapDirection,
    pub amount_in: u64,
    pub minimum_amount_out: u64,
    pub limit_bin_id: u16,
    pub tick_size_quote_atomic: u64,
    pub maximum_bin_id: u16,
    pub maximum_bins: u8,
    /// Earliest possible ordinary price omitted from the canonical page prefix.
    pub unloaded_ordinary_boundary: Option<u16>,
}

pub struct WriterDlmmSwapPolicy<'a> {
    pub eligible: bool,
    pub participation: Option<crate::writer_participation_math::ParticipationTotals>,
    pub book: &'a [WriterSeries],
    pub series_index: usize,
    pub cash: WriterDlmmCash,
    pub risk: WriterDlmmRiskLimits,
    pub buyback: WriterDlmmBuybackLimits,
    pub series_limits: &'a [WriterDlmmSeriesLimits],
    pub month_spent_atoms: u64,
    pub series_month_spent_atoms: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WriterDlmmFillTotals {
    pub gross_premium_atoms: u64,
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
    pub order_fills: Vec<PublicOrderFill>,
    admitted_cash: Option<AdmittedWriterCash>,
}

/// Internal quote evidence. No instruction or caller-supplied bytes can create
/// it; reuse requires the complete final mathematical book and cash to match.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AdmittedWriterCash {
    summary: WriterReserveSummary,
    exposure: u64,
    cash: WriterDlmmCash,
    risk: WriterDlmmRiskLimits,
    series: Vec<WriterSeries>,
}

impl AdmittedWriterCash {
    pub(crate) fn for_final_state(
        &self,
        series: &[WriterSeries],
        cash: WriterDlmmCash,
        risk: &WriterDlmmRiskLimits,
    ) -> Option<(WriterReserveSummary, u64)> {
        (series == self.series && cash == self.cash && *risk == self.risk)
            .then_some((self.summary, self.exposure))
    }
}

impl WriterDlmmRouteQuote {
    pub(crate) fn admitted_cash(&self) -> Option<&AdmittedWriterCash> {
        self.admitted_cash.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicOrderFill {
    pub sequence: u64,
    pub quantity: u64,
    pub quote: u64,
    pub balance_after: crate::dlmm_order_math::OrderBalance,
}

#[derive(Clone, Copy, Debug)]
pub struct PublicOrderRouteLimits {
    pub allow_partial: bool,
    pub maximum_option_output: u64,
    pub maximum_order_fills: usize,
}

#[inline(never)]
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

#[inline(never)]
fn admit_totals_with_metrics(
    config: &WriterDlmmRouteConfig,
    policy: &WriterDlmmSwapPolicy,
    totals: &mut WriterDlmmFillTotals,
    prepared: Option<&crate::writer_sleeve_math::PreparedWriterReserve>,
) -> Option<AdmittedWriterCash> {
    admit_totals_with_cash_mode(config, policy, totals, prepared, false)
}

#[inline(never)]
fn admit_totals_with_cash_mode(
    config: &WriterDlmmRouteConfig,
    policy: &WriterDlmmSwapPolicy,
    totals: &mut WriterDlmmFillTotals,
    prepared: Option<&crate::writer_sleeve_math::PreparedWriterReserve>,
    sleeve_cash: bool,
) -> Option<AdmittedWriterCash> {
    if !policy.eligible
        || policy.series_index >= policy.book.len()
        || policy.book.len() != policy.series_limits.len()
        || policy.book.len() > WRITER_MAX_SERIES
    {
        return None;
    }
    let terms = policy.series_limits[policy.series_index];
    match config.direction {
        AmoebaDlmmSwapDirection::QuoteForOption => {
            let net = totals.gross_premium_atoms;
            let Ok(floor) = dlmm::ceil_mul_div(
                totals.sold_option_atoms,
                terms.seller_floor_quote_atoms,
                dlmm::AMOEBA_DLMM_PRICE_SCALE,
            ) else {
                return None;
            };
            if net < floor {
                return None;
            }
            let Some(assets) = policy
                .cash
                .assets_atoms
                .checked_add(net)
                .and_then(|value| value.checked_add(totals.lp_fee_atoms))
            else {
                return None;
            };
            let Some(oi) = policy.book[policy.series_index]
                .external_oi_atoms
                .checked_add(totals.sold_option_atoms)
            else {
                return None;
            };
            let admission = if let Some(prepared) = prepared {
                prepared.reserve(oi).and_then(|summary| {
                    let exposure=match policy.risk.security_mode {
                        crate::writer_sleeve_math::WriterSecurityMode::GrossExternalMaximumPayout => prepared.gross(oi)?,
                        crate::writer_sleeve_math::WriterSecurityMode::ExactExternalEnvelope => summary.reserve_atoms,
                    };
                    Ok((summary,exposure))
                }).map_err(crate::writer_dlmm_math::WriterDlmmAdmissionError::Envelope).and_then(|(summary,exposure)| {
                    crate::writer_dlmm_math::admit_writer_dlmm_cash_summary(summary,exposure,WriterDlmmCash {assets_atoms:assets,..policy.cash},&policy.risk)
                })
            } else {
                // Invalid preparation follows the unchanged full-book admission path.
                let mut storage = [WriterSeries::EMPTY; WRITER_MAX_SERIES];
                let book = &mut storage[..policy.book.len()];
                book.copy_from_slice(policy.book);
                book[policy.series_index].external_oi_atoms = oi;
                admit_writer_dlmm_cash(
                    book,
                    WriterDlmmCash {
                        assets_atoms: assets,
                        ..policy.cash
                    },
                    &policy.risk,
                )
            };
            let Ok((reserve, _, exposure)) = admission else {
                return None;
            };
            if policy.participation.is_some_and(|participation| {
                participation.admit(assets, reserve.reserve_atoms).is_err()
            }) {
                return None;
            }
            totals.net_premium_atoms = net;
            Some(AdmittedWriterCash {
                summary: reserve,
                exposure,
                cash: WriterDlmmCash {
                    assets_atoms: assets,
                    ..policy.cash
                },
                risk: policy.risk,
                series: Vec::new(),
            })
        }
        AmoebaDlmmSwapDirection::OptionForQuote => {
            let allocated_after = if sleeve_cash {
                policy.cash.allocated_lp_quote_atoms
            } else {
                policy
                    .cash
                    .allocated_lp_quote_atoms
                    .checked_sub(totals.spent_quote_atoms)?
            };
            let retirements = [WriterDlmmRetirement {
                series_index: policy.series_index,
                retired_atoms: totals.retired_option_atoms,
                cost_atoms: totals.spent_quote_atoms,
                series_month_spent_atoms: policy.series_month_spent_atoms,
            }];
            let admission = if sleeve_cash && prepared.is_some() {
                crate::writer_dlmm_math::admit_writer_sleeve_retirement_prepared(
                    policy.book,
                    policy.cash,
                    &policy.risk,
                    &policy.buyback,
                    policy.series_limits,
                    &retirements,
                    policy.month_spent_atoms,
                    prepared.unwrap(),
                )
            } else if sleeve_cash {
                crate::writer_dlmm_math::admit_writer_sleeve_cash_retirement(
                    policy.book,
                    policy.cash,
                    &policy.risk,
                    &policy.buyback,
                    policy.series_limits,
                    &retirements,
                    policy.month_spent_atoms,
                )
            } else {
                admit_writer_dlmm_retirement(
                    policy.book,
                    policy.cash,
                    &policy.risk,
                    &policy.buyback,
                    policy.series_limits,
                    &retirements,
                    policy.month_spent_atoms,
                    allocated_after,
                    false,
                )
            };
            let Ok(admission) = admission else {
                return None;
            };
            let participating = policy.participation.is_none_or(|participation| {
                participation
                    .admit(
                        admission.assets_after_atoms,
                        admission.reserve_after.reserve_atoms,
                    )
                    .is_ok()
            });
            if !participating {
                return None;
            }
            Some(AdmittedWriterCash {
                summary: admission.reserve_after,
                exposure: admission.security_exposure_after_atoms,
                cash: WriterDlmmCash {
                    assets_atoms: admission.assets_after_atoms,
                    allocated_lp_quote_atoms: allocated_after,
                    pooled_quote_atoms: if sleeve_cash {
                        policy.cash.pooled_quote_atoms
                    } else {
                        policy
                            .cash
                            .pooled_quote_atoms
                            .checked_sub(totals.spent_quote_atoms)?
                    },
                    ..policy.cash
                },
                risk: policy.risk,
                series: Vec::new(),
            })
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
/// independently admitted against the full current book. Rejected candidates use a
/// bounded smaller-size search; no later or other-series retirement is assumed.
pub fn quote_writer_dlmm_exact_in(
    config: WriterDlmmRouteConfig,
    ordinary: &[AmoebaDlmmBinLiquidity],
    writer: &[WriterDlmmBinV1],
    policy: Option<&WriterDlmmSwapPolicy>,
) -> Result<WriterDlmmRouteQuote, AmoebaDlmmMathError> {
    quote_dlmm_with_orders(
        config,
        ordinary,
        writer,
        policy,
        &[],
        PublicOrderRouteLimits {
            allow_partial: false,
            maximum_option_output: u64::MAX,
            maximum_order_fills: crate::dlmm_order_math::MAX_ORDER_FILLS,
        },
    )
}

pub fn quote_dlmm_with_orders(
    config: WriterDlmmRouteConfig,
    ordinary: &[AmoebaDlmmBinLiquidity],
    writer: &[WriterDlmmBinV1],
    policy: Option<&WriterDlmmSwapPolicy>,
    orders: &[crate::dlmm_order_state::DlmmOrder],
    limits: PublicOrderRouteLimits,
) -> Result<WriterDlmmRouteQuote, AmoebaDlmmMathError> {
    quote_with_cash_mode(
        config, ordinary, writer, policy, orders, limits, false, None,
    )
}

pub fn quote_shared_strip_writer(
    config: WriterDlmmRouteConfig,
    writer: &[WriterDlmmBinV1],
    policy: &WriterDlmmSwapPolicy,
    limits: PublicOrderRouteLimits,
) -> Result<WriterDlmmRouteQuote, AmoebaDlmmMathError> {
    quote_with_cash_mode(config, &[], writer, Some(policy), &[], limits, true, None)
}

pub(crate) fn quote_shared_strip_writer_prepared(
    config: WriterDlmmRouteConfig,
    writer: &[WriterDlmmBinV1],
    policy: &WriterDlmmSwapPolicy,
    limits: PublicOrderRouteLimits,
    prepared: &crate::writer_sleeve_math::PreparedWriterReserve,
) -> Result<WriterDlmmRouteQuote, AmoebaDlmmMathError> {
    if prepared.target != policy.series_index
        || !prepared.matches_bounds(
            policy.risk.lower_tail_max_settlement_atomic,
            policy.risk.upper_tail_min_settlement_atomic,
        )
        || policy
            .book
            .get(policy.series_index)
            .is_none_or(|s| s.external_oi_atoms != prepared.initial_oi)
    {
        return Err(AmoebaDlmmMathError::InvalidRoute);
    }
    quote_with_cash_mode(
        config,
        &[],
        writer,
        Some(policy),
        &[],
        limits,
        true,
        Some(prepared),
    )
}

fn quote_with_cash_mode(
    config: WriterDlmmRouteConfig,
    ordinary: &[AmoebaDlmmBinLiquidity],
    writer: &[WriterDlmmBinV1],
    policy: Option<&WriterDlmmSwapPolicy>,
    orders: &[crate::dlmm_order_state::DlmmOrder],
    limits: PublicOrderRouteLimits,
    sleeve_cash: bool,
    prepared_override: Option<&crate::writer_sleeve_math::PreparedWriterReserve>,
) -> Result<WriterDlmmRouteQuote, AmoebaDlmmMathError> {
    if config.maximum_bins == 0
        || config.maximum_bins > dlmm::AMOEBA_DLMM_MAXIMUM_BINS_PER_SWAP
        || (!limits.allow_partial && config.minimum_amount_out == 0)
        || writer.len() > crate::state::WRITER_DLMM_POSITION_BINS
        || ordinary.len() > 32 * usize::from(crate::constants::MAX_AMOEBA_DLMM_PAGE_HOPS_PER_SWAP)
        || orders.len() > crate::dlmm_order_state::MAX_ORDER_WITNESSES
        || limits.maximum_order_fills == 0
        || limits.maximum_order_fills > crate::dlmm_order_math::MAX_ORDER_FILLS
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
    if orders.iter().any(|order| {
        order.order_side() != if ascending { 1 } else { 0 }
            || order.remaining_input == 0
            || order.remaining_quantity == 0
    }) || orders.windows(2).any(|pair| {
        let a = &pair[0];
        let b = &pair[1];
        (if ascending {
            a.limit_bin > b.limit_bin
        } else {
            a.limit_bin < b.limit_bin
        }) || (a.limit_bin == b.limit_bin && a.sequence >= b.sequence)
    }) {
        return Err(AmoebaDlmmMathError::InvalidRoute);
    }
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
    let fees = dlmm::calculate_fees(config.amount_in)?;
    let prepared_storage = policy
        .filter(|policy| {
            prepared_override.is_none()
                && policy.eligible
                && config.direction == AmoebaDlmmSwapDirection::QuoteForOption
        })
        .and_then(|policy| {
            crate::writer_sleeve_math::PreparedWriterReserve::new(
                policy.book,
                policy.series_index,
                policy.risk.lower_tail_max_settlement_atomic,
                policy.risk.upper_tail_min_settlement_atomic,
            )
            .ok()
        });
    let prepared = prepared_override.or(prepared_storage.as_ref());
    let mut result = WriterDlmmRouteQuote::default();
    let mut remaining = fees.trade_input;
    let mut allocated = 0u64;
    let mut ordinary_index = 0;
    let mut writer_cursor = 0;
    let mut order_cursor = 0;
    while remaining > 0
        && (ordinary_index < ordinary.len()
            || writer_cursor < writer.len()
            || order_cursor < orders.len())
    {
        if result.quote.fills.len() == usize::from(config.maximum_bins)
            || (ascending && result.quote.amount_out == limits.maximum_option_output)
        {
            break;
        }
        let ordinary_bin = ordinary.get(ordinary_index);
        let writer_index = if ascending {
            writer_cursor
        } else {
            writer.len().saturating_sub(writer_cursor + 1)
        };
        let writer_bin = (writer_cursor < writer.len()).then(|| &writer[writer_index]);
        let mut bin_id = match (ordinary_bin, writer_bin) {
            (Some(a), Some(b)) => {
                if ascending {
                    a.bin_id.min(b.bin_id)
                } else {
                    a.bin_id.max(b.bin_id)
                }
            }
            (Some(a), None) => a.bin_id,
            (None, Some(b)) => b.bin_id,
            _ => {
                orders
                    .get(order_cursor)
                    .ok_or(AmoebaDlmmMathError::InvalidRoute)?
                    .limit_bin
            }
        };
        if let Some(order) = orders.get(order_cursor) {
            bin_id = if ascending {
                bin_id.min(order.limit_bin)
            } else {
                bin_id.max(order.limit_bin)
            };
        }
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
            let mut offered = *bin;
            if ascending {
                offered.option_reserve = offered
                    .option_reserve
                    .min(limits.maximum_option_output - result.quote.amount_out);
            }
            let mut fill = dlmm::fill_bin_exact_input_without_fee(
                config.direction,
                remaining,
                price,
                &offered,
            )?;
            if ascending {
                fill.option_reserve_after = bin.option_reserve - fill.amount_out;
            }
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
        let mut order_bound_hit = false;
        while remaining > 0
            && orders
                .get(order_cursor)
                .is_some_and(|order| order.limit_bin == bin_id)
        {
            if result.order_fills.len() == limits.maximum_order_fills {
                order_bound_hit = true;
                break;
            }
            let order = &orders[order_cursor];
            let mut balance = order
                .balance(config.tick_size_quote_atomic)
                .map_err(|_| AmoebaDlmmMathError::InvalidRoute)?;
            let affordable = |cash: u64| -> u64 {
                u64::try_from(
                    u128::from(cash) * u128::from(dlmm::AMOEBA_DLMM_PRICE_SCALE)
                        / u128::from(price),
                )
                .unwrap_or(u64::MAX)
            };
            let quantity = if ascending {
                affordable(remaining).min(balance.remaining_quantity).min(
                    limits.maximum_option_output - result.quote.amount_out - combined.amount_out,
                )
            } else {
                remaining
                    .min(balance.remaining_quantity)
                    .min(affordable(balance.remaining_input))
            };
            if quantity == 0 {
                order_bound_hit = true;
                break;
            }
            let quote = balance
                .fill(quantity, price)
                .map_err(|_| AmoebaDlmmMathError::InvalidRoute)?;
            let (input, output) = if ascending {
                (quote, quantity)
            } else {
                (quantity, quote)
            };
            remaining = remaining
                .checked_sub(input)
                .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?;
            combined.trade_input = checked_add(combined.trade_input, input)?;
            combined.amount_out = checked_add(combined.amount_out, output)?;
            result.order_fills.push(PublicOrderFill {
                sequence: order.sequence,
                quantity,
                quote,
                balance_after: balance,
            });
            order_cursor += 1;
        }
        if remaining > 0 && !order_bound_hit {
            if let (Some(bin), Some(policy)) = (writer_here, policy) {
                let relevant_reserve = if ascending {
                    bin.option_atoms.min(
                        limits.maximum_option_output
                            - result.quote.amount_out
                            - combined.amount_out,
                    )
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
                        let full_fill = dlmm::fill_bin_exact_input_without_fee(
                            config.direction,
                            remaining,
                            price,
                            &temporary,
                        )?;
                        let mut probe_input = full_fill.trade_input;
                        for probe in 0..MAX_WRITER_ADMISSION_PROBES {
                            if probe == MAX_WRITER_ADMISSION_PROBES - 1 {
                                probe_input = 1;
                            }
                            let mut fill = dlmm::fill_bin_exact_input_without_fee(
                                config.direction,
                                probe_input,
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
                            let admitted = (fill.amount_out > 0)
                                .then(|| {
                                    admit_totals_with_cash_mode(
                                        &config,
                                        policy,
                                        &mut totals,
                                        prepared,
                                        sleeve_cash,
                                    )
                                })
                                .flatten();
                            if let Some(admitted) = admitted {
                                result.admitted_cash = Some(admitted);
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
                                break;
                            }
                            if probe_input <= 1 {
                                break;
                            }
                            probe_input /= 2;
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
        if order_bound_hit {
            break;
        }
    }
    // An empty supplied route is not evidence that the canonical LP side is
    // empty. In particular, post-only placement must not hide a crossing page.
    if remaining > 0
        && result.quote.amount_out < limits.maximum_option_output
        && result.quote.fills.len() < usize::from(config.maximum_bins)
        && result.order_fills.len() < limits.maximum_order_fills
        && config.unloaded_ordinary_boundary.is_some_and(|boundary| {
            if ascending {
                boundary <= config.limit_bin_id
            } else {
                boundary >= config.limit_bin_id
            }
        })
    {
        return Err(AmoebaDlmmMathError::InvalidRoute);
    }
    if remaining != 0 && !limits.allow_partial {
        return Err(AmoebaDlmmMathError::InsufficientLiquidity);
    }
    if result.quote.amount_out < config.minimum_amount_out {
        return Err(AmoebaDlmmMathError::MinimumOutputNotMet);
    }
    if result.quote.fills.is_empty() && limits.allow_partial {
        return Ok(result);
    }
    if allocated != fees.lp_fee || result.quote.fills.is_empty() {
        return Err(AmoebaDlmmMathError::InvalidRoute);
    }
    result.quote.amount_in = config.amount_in - remaining;
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
    if let Some(admitted) = result.admitted_cash.as_mut() {
        let policy = policy.ok_or(AmoebaDlmmMathError::InvalidRoute)?;
        admitted.series = policy.book.to_vec();
        let target = &mut admitted.series[policy.series_index];
        target.external_oi_atoms = if ascending {
            target
                .external_oi_atoms
                .checked_add(result.writer.sold_option_atoms)
        } else {
            target
                .external_oi_atoms
                .checked_sub(result.writer.retired_option_atoms)
        }
        .ok_or(AmoebaDlmmMathError::ArithmeticOverflow)?;
    }
    Ok(result)
}
