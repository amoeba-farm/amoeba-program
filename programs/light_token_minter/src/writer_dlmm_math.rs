//! Writer-owned DLMM admission. All values are observed state or frozen policy inputs.
//! Options are liabilities/inventory, never cash collateral. An acquired external claim
//! appears in `retired_atoms` only when the enclosing instruction retires it atomically.

use crate::writer_sleeve_math::{
    exact_reserve, security_exposure, WriterMathError, WriterReserveSummary, WriterSecurityMode,
    WriterSeries, WRITER_CONTRACT_ATOMIC_SCALE, WRITER_MAX_SERIES, WRITER_RATIO_SCALE_PPM,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterDlmmAdmissionError {
    Arithmetic,
    InvalidPolicy,
    InvalidBook,
    InvalidRetirement,
    NoReserveReduction,
    BudgetExceeded,
    PriceSeparation,
    Insolvent,
    Drawdown,
    SecurityCap,
    CloseHasPriority,
    Envelope(WriterMathError),
}

type Result<T> = core::result::Result<T, WriterDlmmAdmissionError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriterDlmmRiskLimits {
    pub operational_buffer_atoms: u64,
    pub shared_reserve: bool,
    pub worst_drawdown_ppm: u64,
    pub lower_drawdown_ppm: u64,
    pub upper_drawdown_ppm: u64,
    pub lower_tail_max_settlement_atomic: u64,
    pub upper_tail_min_settlement_atomic: u64,
    pub security_mode: WriterSecurityMode,
    /// Legacy caller field; not used as an exposure admission limit.
    pub security_cap_atoms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriterDlmmSeriesLimits {
    /// Immutable quote atoms per canonical contract, signed before capital commitment.
    pub conservative_claim_value_atoms: u64,
    pub seller_floor_quote_atoms: u64,
    pub monthly_buyback_cap_atoms: u64,
    pub transaction_buyback_cap_atoms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriterDlmmBuybackLimits {
    pub monthly_buyback_cap_atoms: u64,
    pub transaction_buyback_cap_atoms: u64,
    pub reserve_release_spend_ratio_ppm: u64,
    pub tick_size_quote_atoms: u64,
    pub price_separation_ticks: u16,
    /// Actual canonical fees for both legs, conservatively expressed in quote atoms
    /// per contract by the caller from the validated pool/policy fee schedule.
    pub round_trip_fee_quote_atoms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriterDlmmCash {
    pub assets_atoms: u64,
    pub principal_atoms: u64,
    /// Writer quote actively committed to bids; uncommitted cash is included only in A.
    pub allocated_lp_quote_atoms: u64,
    /// All writer-owned quote held in DLMM pools, including uncommitted quote.
    pub pooled_quote_atoms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriterDlmmRetirement {
    pub series_index: usize,
    pub retired_atoms: u64,
    /// Entire decrease in writer assets, including every fee paid by the writer.
    pub cost_atoms: u64,
    pub series_month_spent_atoms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriterDlmmAdmission {
    pub assets_after_atoms: u64,
    pub reserve_before_atoms: u64,
    pub reserve_after: WriterReserveSummary,
    pub released_reserve_atoms: u64,
    pub cost_atoms: u64,
    pub monthly_spent_after_atoms: u64,
    pub free_cash_after_atoms: u64,
    pub security_exposure_after_atoms: u64,
}

fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(WriterDlmmAdmissionError::Arithmetic)
}

fn mul_floor(a: u64, b: u64, scale: u64) -> Result<u64> {
    if scale == 0 {
        return Err(WriterDlmmAdmissionError::InvalidPolicy);
    }
    u64::try_from(u128::from(a) * u128::from(b) / u128::from(scale))
        .map_err(|_| WriterDlmmAdmissionError::Arithmetic)
}

fn mul_ceil(a: u64, b: u64, scale: u64) -> Result<u64> {
    if scale == 0 {
        return Err(WriterDlmmAdmissionError::InvalidPolicy);
    }
    let numerator = u128::from(a) * u128::from(b);
    u64::try_from(numerator.div_ceil(u128::from(scale)))
        .map_err(|_| WriterDlmmAdmissionError::Arithmetic)
}

/// Current ask and protected bid with the immutable quote-grid separation.
pub fn writer_dlmm_price_bounds(
    seller_floor: u64,
    tick: u64,
    separation_ticks: u16,
) -> Result<(u64, u64, u64)> {
    if seller_floor == 0 || tick == 0 || separation_ticks == 0 {
        return Err(WriterDlmmAdmissionError::InvalidPolicy);
    }
    let gap = tick
        .checked_mul(u64::from(separation_ticks))
        .ok_or(WriterDlmmAdmissionError::Arithmetic)?;
    let bid = seller_floor
        .checked_sub(gap)
        .ok_or(WriterDlmmAdmissionError::PriceSeparation)?;
    Ok((seller_floor, bid, 0))
}

fn reserve(book: &[WriterSeries], limits: &WriterDlmmRiskLimits) -> Result<WriterReserveSummary> {
    exact_reserve(
        book,
        limits.lower_tail_max_settlement_atomic,
        limits.upper_tail_min_settlement_atomic,
    )
    .map_err(WriterDlmmAdmissionError::Envelope)
}

/// Drawdown is max(0, W - (A - liability)), including prior approved expenditure.
/// Cumulative premium is an accounting history, not available cash or a loss offset.
fn check_drawdown(assets: u64, principal: u64, liability: u64, limit: u64) -> Result<()> {
    if limit > WRITER_RATIO_SCALE_PPM || principal == 0 {
        return Err(WriterDlmmAdmissionError::InvalidPolicy);
    }
    let equity = assets
        .checked_sub(liability)
        .ok_or(WriterDlmmAdmissionError::Insolvent)?;
    let loss = principal.saturating_sub(equity);
    // For integer loss, loss > floor(P * limit / scale) is exactly
    // loss * scale > P * limit. Both u64 products fit in u128. The policy
    // guard also proves the original quotient fits in u64, so this removes
    // no arithmetic error and preserves the checks above in their order.
    if u128::from(loss) * u128::from(WRITER_RATIO_SCALE_PPM)
        > u128::from(principal) * u128::from(limit)
    {
        return Err(WriterDlmmAdmissionError::Drawdown);
    }
    Ok(())
}

/// Common post-fill gate for asks, bids, placement, withdrawal and close accounting.
/// The returned free cash excludes both exact settlement reserve and the operating buffer.
pub fn admit_writer_dlmm_cash(
    book: &[WriterSeries],
    cash: WriterDlmmCash,
    limits: &WriterDlmmRiskLimits,
) -> Result<(WriterReserveSummary, u64, u64)> {
    let summary = reserve(book, limits)?;
    let (_, free_cash, _) = admit_writer_dlmm_cash_summary(summary, 0, cash, limits)?;
    let exposure = security_exposure(limits.security_mode, book, summary.reserve_atoms)
        .map_err(WriterDlmmAdmissionError::Envelope)?;
    Ok((summary, free_cash, exposure))
}

pub(crate) fn admit_writer_dlmm_cash_summary(
    summary: WriterReserveSummary,
    exposure: u64,
    cash: WriterDlmmCash,
    limits: &WriterDlmmRiskLimits,
) -> Result<(WriterReserveSummary, u64, u64)> {
    if cash.allocated_lp_quote_atoms > cash.pooled_quote_atoms {
        return Err(WriterDlmmAdmissionError::InvalidBook);
    }
    let protected = add(
        if limits.shared_reserve {
            summary.reserve_atoms.max(cash.pooled_quote_atoms)
        } else {
            summary.reserve_atoms
        },
        limits.operational_buffer_atoms,
    )?;
    let free_cash = cash
        .assets_atoms
        .checked_sub(protected)
        .ok_or(WriterDlmmAdmissionError::Insolvent)?;
    if !limits.shared_reserve && cash.allocated_lp_quote_atoms > free_cash {
        return Err(WriterDlmmAdmissionError::Insolvent);
    }
    check_drawdown(
        cash.assets_atoms,
        cash.principal_atoms,
        summary.reserve_atoms,
        limits.worst_drawdown_ppm,
    )?;
    check_drawdown(
        cash.assets_atoms,
        cash.principal_atoms,
        summary.lower_tail_reserve_atoms,
        limits.lower_drawdown_ppm,
    )?;
    check_drawdown(
        cash.assets_atoms,
        cash.principal_atoms,
        summary.upper_tail_reserve_atoms,
        limits.upper_drawdown_ppm,
    )?;
    // The legacy oracle cap is informational only. Cash, exact reserve and
    // drawdown checks above remain mandatory for every admitted operation.
    Ok((summary, free_cash, exposure))
}

/// Admit a complete atomic retirement set. Entries are unique in canonical series order.
/// Splitting calls cannot increase the sleeve/month or series/month spending allowances.
/// No redeployment credit exists: this implementation has no immediate permitted redeployment.
#[allow(clippy::too_many_arguments)]
pub fn admit_writer_dlmm_retirement(
    book: &[WriterSeries],
    cash: WriterDlmmCash,
    risk: &WriterDlmmRiskLimits,
    policy: &WriterDlmmBuybackLimits,
    series_limits: &[WriterDlmmSeriesLimits],
    retirements: &[WriterDlmmRetirement],
    month_spent_atoms: u64,
    allocated_lp_quote_after_atoms: u64,
    close_pending: bool,
) -> Result<WriterDlmmAdmission> {
    admit_writer_retirement_inner(
        book,
        cash,
        risk,
        policy,
        series_limits,
        retirements,
        month_spent_atoms,
        allocated_lp_quote_after_atoms,
        close_pending,
        false,
        None,
    )
}

/// The same retirement price, budget and full-book reserve checks when the
/// authenticated cash remains in the sleeve rather than a pool LP escrow.
pub fn admit_writer_sleeve_cash_retirement(
    book: &[WriterSeries],
    cash: WriterDlmmCash,
    risk: &WriterDlmmRiskLimits,
    policy: &WriterDlmmBuybackLimits,
    series_limits: &[WriterDlmmSeriesLimits],
    retirements: &[WriterDlmmRetirement],
    month_spent_atoms: u64,
) -> Result<WriterDlmmAdmission> {
    admit_writer_retirement_inner(
        book,
        cash,
        risk,
        policy,
        series_limits,
        retirements,
        month_spent_atoms,
        cash.allocated_lp_quote_atoms,
        false,
        true,
        None,
    )
}

// Keep the existing accounting interface and its explicit inputs.
#[allow(clippy::too_many_arguments)]
pub(crate) fn admit_writer_sleeve_retirement_prepared(
    book: &[WriterSeries],
    cash: WriterDlmmCash,
    risk: &WriterDlmmRiskLimits,
    policy: &WriterDlmmBuybackLimits,
    series_limits: &[WriterDlmmSeriesLimits],
    retirements: &[WriterDlmmRetirement],
    month_spent_atoms: u64,
    prepared: &crate::writer_sleeve_math::PreparedWriterReserve,
) -> Result<WriterDlmmAdmission> {
    validate_prepared_retirement(book, risk, retirements, prepared)?;
    admit_writer_retirement_inner(
        book,
        cash,
        risk,
        policy,
        series_limits,
        retirements,
        month_spent_atoms,
        cash.allocated_lp_quote_atoms,
        false,
        true,
        Some(prepared),
    )
}

/// Reuse the authenticated reserve grid for ordinary pooled-cash retirement.
/// Every pooled-cash, allocation, price, budget and reserve-release check remains
/// in the same admission path as `admit_writer_dlmm_retirement`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn admit_writer_dlmm_retirement_prepared(
    book: &[WriterSeries],
    cash: WriterDlmmCash,
    risk: &WriterDlmmRiskLimits,
    policy: &WriterDlmmBuybackLimits,
    series_limits: &[WriterDlmmSeriesLimits],
    retirements: &[WriterDlmmRetirement],
    month_spent_atoms: u64,
    allocated_lp_quote_after_atoms: u64,
    close_pending: bool,
    prepared: &crate::writer_sleeve_math::PreparedWriterReserve,
) -> Result<WriterDlmmAdmission> {
    validate_prepared_retirement(book, risk, retirements, prepared)?;
    admit_writer_retirement_inner(
        book,
        cash,
        risk,
        policy,
        series_limits,
        retirements,
        month_spent_atoms,
        allocated_lp_quote_after_atoms,
        close_pending,
        false,
        Some(prepared),
    )
}

fn validate_prepared_retirement(
    book: &[WriterSeries],
    risk: &WriterDlmmRiskLimits,
    retirements: &[WriterDlmmRetirement],
    prepared: &crate::writer_sleeve_math::PreparedWriterReserve,
) -> Result<()> {
    if retirements.len() != 1
        || !prepared.matches_bounds(
            risk.lower_tail_max_settlement_atomic,
            risk.upper_tail_min_settlement_atomic,
        )
        || retirements[0].series_index != prepared.target
        || book
            .get(prepared.target)
            .is_none_or(|s| s.external_oi_atoms != prepared.initial_oi)
    {
        return Err(WriterDlmmAdmissionError::InvalidPolicy);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn admit_writer_retirement_inner(
    book: &[WriterSeries],
    cash: WriterDlmmCash,
    risk: &WriterDlmmRiskLimits,
    policy: &WriterDlmmBuybackLimits,
    series_limits: &[WriterDlmmSeriesLimits],
    retirements: &[WriterDlmmRetirement],
    month_spent_atoms: u64,
    allocated_lp_quote_after_atoms: u64,
    close_pending: bool,
    sleeve_cash: bool,
    prepared: Option<&crate::writer_sleeve_math::PreparedWriterReserve>,
) -> Result<WriterDlmmAdmission> {
    if close_pending {
        return Err(WriterDlmmAdmissionError::CloseHasPriority);
    }
    if book.is_empty()
        || book.len() > WRITER_MAX_SERIES
        || series_limits.len() != book.len()
        || retirements.is_empty()
        || retirements.len() > book.len()
    {
        return Err(WriterDlmmAdmissionError::InvalidBook);
    }
    if policy.tick_size_quote_atoms == 0
        || policy.price_separation_ticks == 0
        || policy.reserve_release_spend_ratio_ppm > WRITER_RATIO_SCALE_PPM
    {
        return Err(WriterDlmmAdmissionError::InvalidPolicy);
    }
    let before = if let Some(cache) = prepared {
        cache
            .reserve(cache.initial_oi)
            .map_err(WriterDlmmAdmissionError::Envelope)?
    } else {
        reserve(book, risk)?
    };
    let mut after = [WriterSeries::EMPTY; WRITER_MAX_SERIES];
    after[..book.len()].copy_from_slice(book);
    let tick_gap = policy
        .tick_size_quote_atoms
        .checked_mul(u64::from(policy.price_separation_ticks))
        .ok_or(WriterDlmmAdmissionError::Arithmetic)?;
    let mut previous = None;
    let mut total_cost = 0;
    let mut conservative_value = 0;
    for retirement in retirements {
        let i = retirement.series_index;
        if i >= book.len() || previous.is_some_and(|p| i <= p) || retirement.retired_atoms == 0 {
            return Err(WriterDlmmAdmissionError::InvalidRetirement);
        }
        previous = Some(i);
        let limits = series_limits[i];
        if limits.conservative_claim_value_atoms > book[i].max_payout_per_contract_atoms
            || limits.seller_floor_quote_atoms > book[i].max_payout_per_contract_atoms
        {
            return Err(WriterDlmmAdmissionError::InvalidPolicy);
        }
        after[i].external_oi_atoms = after[i]
            .external_oi_atoms
            .checked_sub(retirement.retired_atoms)
            .ok_or(WriterDlmmAdmissionError::InvalidRetirement)?;
        if retirement.cost_atoms > limits.transaction_buyback_cap_atoms
            || add(retirement.series_month_spent_atoms, retirement.cost_atoms)?
                > limits.monthly_buyback_cap_atoms
        {
            return Err(WriterDlmmAdmissionError::BudgetExceeded);
        }
        let separated_price = limits
            .seller_floor_quote_atoms
            .checked_sub(tick_gap)
            .ok_or(WriterDlmmAdmissionError::PriceSeparation)?;
        let price_bound = mul_floor(
            retirement.retired_atoms,
            separated_price,
            WRITER_CONTRACT_ATOMIC_SCALE,
        )?
        .checked_sub(mul_ceil(
            retirement.retired_atoms,
            policy.round_trip_fee_quote_atoms,
            WRITER_CONTRACT_ATOMIC_SCALE,
        )?)
        .ok_or(WriterDlmmAdmissionError::PriceSeparation)?;
        if retirement.cost_atoms > price_bound {
            return Err(WriterDlmmAdmissionError::PriceSeparation);
        }
        total_cost = add(total_cost, retirement.cost_atoms)?;
        conservative_value = add(
            conservative_value,
            mul_floor(
                retirement.retired_atoms,
                limits.conservative_claim_value_atoms,
                WRITER_CONTRACT_ATOMIC_SCALE,
            )?,
        )?;
    }
    let assets_after = cash
        .assets_atoms
        .checked_sub(total_cost)
        .ok_or(WriterDlmmAdmissionError::Insolvent)?;
    // Compact execution spends the sleeve's cash. Independent-pool custody
    // stays reserved and its balances cannot fund this retirement.
    if sleeve_cash && assets_after < cash.pooled_quote_atoms {
        return Err(WriterDlmmAdmissionError::Insolvent);
    }
    let after_cash = WriterDlmmCash {
        assets_atoms: assets_after,
        principal_atoms: cash.principal_atoms,
        allocated_lp_quote_atoms: allocated_lp_quote_after_atoms,
        pooled_quote_atoms: if sleeve_cash {
            cash.pooled_quote_atoms
        } else {
            cash.pooled_quote_atoms
                .checked_sub(total_cost)
                .ok_or(WriterDlmmAdmissionError::Insolvent)?
        },
    };
    let (summary, free_cash, exposure) = if let Some(cache) = prepared {
        let oi = after[cache.target].external_oi_atoms;
        let summary = cache
            .reserve(oi)
            .map_err(WriterDlmmAdmissionError::Envelope)?;
        let exposure = match risk.security_mode {
            WriterSecurityMode::GrossExternalMaximumPayout => cache
                .gross(oi)
                .map_err(WriterDlmmAdmissionError::Envelope)?,
            WriterSecurityMode::ExactExternalEnvelope => summary.reserve_atoms,
        };
        admit_writer_dlmm_cash_summary(summary, exposure, after_cash, risk)?
    } else {
        admit_writer_dlmm_cash(&after[..book.len()], after_cash, risk)?
    };
    // A retirement need not free reserve. The reserve is the worst stress scenario, so retiring
    // contracts on the side that does not bind frees nothing even at fair value (a short's puts
    // while the call side binds); such a buyback is funded from the sleeve's free cash instead,
    // still within the price bound, the claim value, both buy-back caps and the cash left after
    // every remaining obligation, with the post-trade risk admission above.
    let released = before.reserve_atoms.saturating_sub(summary.reserve_atoms);
    let monthly_after = add(month_spent_atoms, total_cost)?;
    let post_retirement_available = cash
        .assets_atoms
        .checked_sub(add(summary.reserve_atoms, risk.operational_buffer_atoms)?)
        .ok_or(WriterDlmmAdmissionError::Insolvent)?;
    if total_cost > conservative_value
        || total_cost > policy.transaction_buyback_cap_atoms
        || monthly_after > policy.monthly_buyback_cap_atoms
        || total_cost > post_retirement_available
    {
        return Err(WriterDlmmAdmissionError::BudgetExceeded);
    }
    Ok(WriterDlmmAdmission {
        assets_after_atoms: assets_after,
        reserve_before_atoms: before.reserve_atoms,
        reserve_after: summary,
        released_reserve_atoms: released,
        cost_atoms: total_cost,
        monthly_spent_after_atoms: monthly_after,
        free_cash_after_atoms: free_cash,
        security_exposure_after_atoms: exposure,
    })
}
