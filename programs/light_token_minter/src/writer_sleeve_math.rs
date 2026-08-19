//! Pure, bounded arithmetic for collective writer sleeves.
//!
//! This module deliberately has no account, clock, token, or transport dependencies. Every
//! instruction that changes collective assets or liabilities must eventually call this module
//! rather than reproducing a financial formula in a processor.
//!
//! A collective liability is rounded exactly once, after summing the complete book numerator.
//! Rounding each series before summation makes breakpoint-only maximization false for fractional
//! contract OI. The aggregate ceiling below both preserves the piecewise-linear candidate proof
//! and defines the exact atomic amount allocated to collective long holders at settlement.

use core::cmp::min;

use crate::state::{MarketMintAccounting, OptionKind};

pub use crate::constants::{
    WRITER_MAX_CANDIDATE_POINTS, WRITER_MAX_LIVE_SERIES as WRITER_MAX_SERIES,
    WRITER_RATIO_SCALE_PPM, WRITER_SERIES_STORAGE_CAPACITY as WRITER_SERIES_CAPACITY,
};
pub const WRITER_CONTRACT_ATOMIC_SCALE: u64 = MarketMintAccounting::CANONICAL_ATOMIC_SCALE;
#[cfg(feature = "writer-math-benchmark")]
pub const WRITER_MATH_BENCHMARK_DOMAIN: &[u8] = b"ameba-writer-math-benchmark-v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterMathError {
    ArithmeticOverflow,
    DivisionByZero,
    EmptySeriesBook,
    TooManySeries,
    TooManyCandidatePoints,
    InvalidSeries,
    InvalidTailBoundaries,
    InvalidRiskLimit,
    InvalidFlatSupply,
    InvalidCloseAmount,
    IncompatibleSeriesBooks,
    CloseBasketExceedsOpenInterest,
    Insolvent,
    WithdrawalIsZero,
}

pub type WriterMathResult<T> = Result<T, WriterMathError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriterSeries {
    pub kind: OptionKind,
    pub strike_price_atomic: u64,
    /// Call cap or put floor, matching the current `InstrumentDefinition.cap_price` meaning.
    pub cap_price_atomic: u64,
    pub contract_size_atoms: u64,
    pub max_payout_per_contract_atoms: u64,
    pub external_oi_atoms: u64,
}

impl WriterSeries {
    pub const EMPTY: Self = Self {
        kind: OptionKind::CallSpread,
        strike_price_atomic: 0,
        cap_price_atomic: 0,
        contract_size_atoms: 0,
        max_payout_per_contract_atoms: 0,
        external_oi_atoms: 0,
    };

    #[inline]
    fn same_instrument(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.strike_price_atomic == other.strike_price_atomic
            && self.cap_price_atomic == other.cap_price_atomic
            && self.contract_size_atoms == other.contract_size_atoms
            && self.max_payout_per_contract_atoms == other.max_payout_per_contract_atoms
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriterCandidatePoints {
    /// The 128-point V1 bound occupies 1 KiB and remains stack-safe. Keeping the scratch inline
    /// is essential for auction admission, where the SBF bump allocator cannot reclaim a fresh
    /// heap buffer after each monotone-search probe.
    values: [u64; WRITER_MAX_CANDIDATE_POINTS],
    len: u16,
}

impl Default for WriterCandidatePoints {
    fn default() -> Self {
        Self {
            values: [0; WRITER_MAX_CANDIDATE_POINTS],
            len: 0,
        }
    }
}

impl WriterCandidatePoints {
    #[inline]
    pub fn as_slice(&self) -> &[u64] {
        &self.values[..usize::from(self.len)]
    }

    #[inline]
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn insert(&mut self, value: u64) -> WriterMathResult<()> {
        let values = self.as_slice();
        let index = match values.binary_search(&value) {
            Ok(_) => return Ok(()),
            Err(index) => index,
        };
        let len = self.len();
        if len == WRITER_MAX_CANDIDATE_POINTS {
            return Err(WriterMathError::TooManyCandidatePoints);
        }
        for destination in (index + 1..=len).rev() {
            self.values[destination] = self.values[destination - 1];
        }
        self.values[index] = value;
        self.len = self
            .len
            .checked_add(1)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        Ok(())
    }

    fn insert_adjacent(&mut self, breakpoint: u64) -> WriterMathResult<()> {
        if let Some(previous) = breakpoint.checked_sub(1) {
            self.insert(previous)?;
        }
        self.insert(breakpoint)?;
        if let Some(next) = breakpoint.checked_add(1) {
            self.insert(next)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WriterReserveSummary {
    pub reserve_atoms: u64,
    pub reserve_settlement_atomic: u64,
    pub lower_tail_reserve_atoms: u64,
    pub lower_tail_settlement_atomic: u64,
    pub upper_tail_reserve_atoms: u64,
    pub upper_tail_settlement_atomic: u64,
    pub candidate_count: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriterIssueAdmissionLimits {
    pub security_mode: WriterSecurityMode,
    pub security_cap_atoms: u64,
    pub accounted_asset_atoms: u64,
    pub locked_primary_premium_atoms: u64,
    pub writer_principal_atoms: u64,
    pub operational_buffer_atoms: u64,
    pub worst_drawdown_limit: u64,
    pub lower_drawdown_limit: u64,
    pub upper_drawdown_limit: u64,
    pub lower_tail_max_settlement_atomic: u64,
    pub upper_tail_min_settlement_atomic: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WriterSeriesAmounts {
    pub values: [u64; WRITER_SERIES_CAPACITY],
    pub len: u8,
}

impl WriterSeriesAmounts {
    #[inline]
    pub fn as_slice(&self) -> &[u64] {
        &self.values[..usize::from(self.len)]
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WriterDrawdownChecks {
    pub full_book_passes: bool,
    pub lower_tail_passes: bool,
    pub upper_tail_passes: bool,
}

impl WriterDrawdownChecks {
    #[inline]
    pub fn all_pass(&self) -> bool {
        self.full_book_passes && self.lower_tail_passes && self.upper_tail_passes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum WriterSecurityMode {
    GrossExternalMaximumPayout = 0,
    ExactExternalEnvelope = 1,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WriterClosePreview {
    pub flat_to_burn_atoms: u64,
    pub withdrawal_atoms: u64,
    pub reserve_before_atoms: u64,
    pub reserve_after_atoms: u64,
    pub lower_tail_reserve_before_atoms: u64,
    pub lower_tail_reserve_after_atoms: u64,
    pub upper_tail_reserve_before_atoms: u64,
    pub upper_tail_reserve_after_atoms: u64,
    pub reserve_released_atoms: u64,
    pub statewise_safe_withdrawal_atoms: u64,
    pub statewise_binding_settlement_atomic: u64,
    pub candidate_count: u16,
    pub required_claim_atoms: WriterSeriesAmounts,
}

#[inline]
fn checked_ceil_div(numerator: u128, denominator: u128) -> WriterMathResult<u128> {
    if denominator == 0 {
        return Err(WriterMathError::DivisionByZero);
    }
    let quotient = numerator / denominator;
    quotient
        .checked_add(u128::from(numerator % denominator != 0))
        .ok_or(WriterMathError::ArithmeticOverflow)
}

#[inline]
fn checked_ceil_mul_div(a: u64, b: u64, denominator: u64) -> WriterMathResult<u64> {
    let product = u128::from(a)
        .checked_mul(u128::from(b))
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    u64::try_from(checked_ceil_div(product, u128::from(denominator))?)
        .map_err(|_| WriterMathError::ArithmeticOverflow)
}

#[inline]
fn validate_series(series: &WriterSeries) -> WriterMathResult<()> {
    if series.contract_size_atoms != WRITER_CONTRACT_ATOMIC_SCALE
        || series.max_payout_per_contract_atoms == 0
    {
        return Err(WriterMathError::InvalidSeries);
    }
    let width = match series.kind {
        OptionKind::CallSpread => series
            .cap_price_atomic
            .checked_sub(series.strike_price_atomic),
        OptionKind::PutSpread => series
            .strike_price_atomic
            .checked_sub(series.cap_price_atomic),
    }
    .filter(|width| *width > 0)
    .ok_or(WriterMathError::InvalidSeries)?;
    if width != series.max_payout_per_contract_atoms {
        return Err(WriterMathError::InvalidSeries);
    }
    Ok(())
}

fn validate_series_book(series: &[WriterSeries]) -> WriterMathResult<()> {
    if series.is_empty() {
        return Err(WriterMathError::EmptySeriesBook);
    }
    if series.len() > WRITER_MAX_SERIES {
        return Err(WriterMathError::TooManySeries);
    }
    for item in series {
        validate_series(item)?;
    }
    Ok(())
}

#[inline]
fn payout_per_contract_unchecked(series: &WriterSeries, settlement_price_atomic: u64) -> u64 {
    match series.kind {
        OptionKind::CallSpread => settlement_price_atomic
            .min(series.cap_price_atomic)
            .saturating_sub(series.strike_price_atomic),
        OptionKind::PutSpread => series
            .strike_price_atomic
            .saturating_sub(settlement_price_atomic.max(series.cap_price_atomic)),
    }
}

/// Exact capped-call or capped-put payout for one whole canonical contract.
pub fn payout_per_contract(
    series: &WriterSeries,
    settlement_price_atomic: u64,
) -> WriterMathResult<u64> {
    validate_series(series)?;
    let payout = payout_per_contract_unchecked(series, settlement_price_atomic);
    if payout > series.max_payout_per_contract_atoms {
        return Err(WriterMathError::InvalidSeries);
    }
    Ok(payout)
}

fn aggregate_liability_numerator_unchecked(
    series: &[WriterSeries],
    settlement_price_atomic: u64,
) -> WriterMathResult<u128> {
    let mut numerator = 0u128;
    for item in series {
        let payout = payout_per_contract_unchecked(item, settlement_price_atomic);
        let term = u128::from(item.external_oi_atoms)
            .checked_mul(u128::from(payout))
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        numerator = numerator
            .checked_add(term)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
    }
    Ok(numerator)
}

fn paired_liability_numerators_unchecked(
    before: &[WriterSeries],
    after: &[WriterSeries],
    settlement_price_atomic: u64,
) -> WriterMathResult<(u128, u128)> {
    let mut before_numerator = 0u128;
    let mut after_numerator = 0u128;
    for (old, new) in before.iter().zip(after) {
        let payout = payout_per_contract_unchecked(old, settlement_price_atomic);
        before_numerator = before_numerator
            .checked_add(
                u128::from(old.external_oi_atoms)
                    .checked_mul(u128::from(payout))
                    .ok_or(WriterMathError::ArithmeticOverflow)?,
            )
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        after_numerator = after_numerator
            .checked_add(
                u128::from(new.external_oi_atoms)
                    .checked_mul(u128::from(payout))
                    .ok_or(WriterMathError::ArithmeticOverflow)?,
            )
            .ok_or(WriterMathError::ArithmeticOverflow)?;
    }
    Ok((before_numerator, after_numerator))
}

/// Complete-book liability before the single canonical contract-scale division.
pub fn aggregate_liability_numerator(
    series: &[WriterSeries],
    settlement_price_atomic: u64,
) -> WriterMathResult<u128> {
    validate_series_book(series)?;
    aggregate_liability_numerator_unchecked(series, settlement_price_atomic)
}

#[inline]
fn liability_atoms_from_numerator(numerator: u128) -> WriterMathResult<u64> {
    u64::try_from(checked_ceil_div(
        numerator,
        u128::from(WRITER_CONTRACT_ATOMIC_SCALE),
    )?)
    .map_err(|_| WriterMathError::ArithmeticOverflow)
}

/// Exact collective atomic liability, rounded upward once after summing the complete book.
pub fn aggregate_liability(
    series: &[WriterSeries],
    settlement_price_atomic: u64,
) -> WriterMathResult<u64> {
    liability_atoms_from_numerator(aggregate_liability_numerator(
        series,
        settlement_price_atomic,
    )?)
}

/// Build the canonical sorted, unique and bounded settlement candidate set.
pub fn canonical_candidate_points(
    series: &[WriterSeries],
    lower_tail_max_settlement_atomic: u64,
    upper_tail_min_settlement_atomic: u64,
) -> WriterMathResult<WriterCandidatePoints> {
    validate_series_book(series)?;
    if lower_tail_max_settlement_atomic >= upper_tail_min_settlement_atomic {
        return Err(WriterMathError::InvalidTailBoundaries);
    }

    let mut candidates = WriterCandidatePoints::default();
    candidates.insert(0)?;
    let mut largest_payoff_breakpoint = 0u64;
    for item in series {
        candidates.insert_adjacent(item.strike_price_atomic)?;
        candidates.insert_adjacent(item.cap_price_atomic)?;
        largest_payoff_breakpoint = largest_payoff_breakpoint
            .max(item.strike_price_atomic)
            .max(item.cap_price_atomic);
    }
    candidates.insert_adjacent(lower_tail_max_settlement_atomic)?;
    candidates.insert_adjacent(upper_tail_min_settlement_atomic)?;
    if let Some(beyond) = largest_payoff_breakpoint.checked_add(1) {
        candidates.insert(beyond)?;
    }
    Ok(candidates)
}

/// Compute full-book and inclusive tail reserves from one canonical liability walk.
pub fn exact_reserve(
    series: &[WriterSeries],
    lower_tail_max_settlement_atomic: u64,
    upper_tail_min_settlement_atomic: u64,
) -> WriterMathResult<WriterReserveSummary> {
    validate_series_book(series)?;
    let candidates = canonical_candidate_points(
        series,
        lower_tail_max_settlement_atomic,
        upper_tail_min_settlement_atomic,
    )?;
    let mut result = WriterReserveSummary {
        candidate_count: u16::try_from(candidates.len())
            .map_err(|_| WriterMathError::ArithmeticOverflow)?,
        ..WriterReserveSummary::default()
    };
    let mut saw_lower = false;
    let mut saw_upper = false;
    for settlement in candidates.as_slice() {
        let liability = liability_atoms_from_numerator(aggregate_liability_numerator_unchecked(
            series,
            *settlement,
        )?)?;
        if liability > result.reserve_atoms {
            result.reserve_atoms = liability;
            result.reserve_settlement_atomic = *settlement;
        }
        if *settlement <= lower_tail_max_settlement_atomic
            && (!saw_lower || liability > result.lower_tail_reserve_atoms)
        {
            saw_lower = true;
            result.lower_tail_reserve_atoms = liability;
            result.lower_tail_settlement_atomic = *settlement;
        }
        if *settlement >= upper_tail_min_settlement_atomic
            && (!saw_upper || liability > result.upper_tail_reserve_atoms)
        {
            saw_upper = true;
            result.upper_tail_reserve_atoms = liability;
            result.upper_tail_settlement_atomic = *settlement;
        }
    }
    if !saw_lower || !saw_upper {
        return Err(WriterMathError::InvalidTailBoundaries);
    }
    Ok(result)
}

#[inline]
fn restrict_linear_admission(
    maximum_contracts: &mut u64,
    liability_base_atoms: u64,
    liability_per_contract_atoms: u64,
    funding_base_atoms: u64,
    funding_per_contract_atoms: u64,
) {
    if liability_base_atoms > funding_base_atoms {
        *maximum_contracts = 0;
        return;
    }
    if liability_per_contract_atoms > funding_per_contract_atoms {
        *maximum_contracts = min(
            *maximum_contracts,
            funding_base_atoms.saturating_sub(liability_base_atoms)
                / liability_per_contract_atoms.saturating_sub(funding_per_contract_atoms),
        );
    }
}

#[inline]
fn drawdown_allowance_atoms(writer_principal_atoms: u64, limit_ppm: u64) -> WriterMathResult<u64> {
    if limit_ppm > WRITER_RATIO_SCALE_PPM {
        return Err(WriterMathError::InvalidRiskLimit);
    }
    u64::try_from(
        u128::from(writer_principal_atoms)
            .checked_mul(u128::from(limit_ppm))
            .ok_or(WriterMathError::ArithmeticOverflow)?
            / u128::from(WRITER_RATIO_SCALE_PPM),
    )
    .map_err(|_| WriterMathError::ArithmeticOverflow)
}

/// Largest whole-contract primary issue that satisfies every exact V1 admission inequality.
///
/// Adding a whole canonical contract contributes an integer payout at every settlement point.
/// Consequently, after the aggregate-book ceiling is applied to the existing book once, every
/// reserve, tail, solvency, drawdown and exact-envelope constraint is a linear inequality in the
/// number of new contracts. This single canonical-candidate walk is exactly equivalent to
/// repeatedly recomputing the complete reserve for each monotone-search probe.
#[allow(clippy::too_many_arguments)]
pub fn maximum_safe_issue_quantity(
    series: &[WriterSeries],
    series_index: usize,
    bid_price_per_contract_atoms: u64,
    maximum_quantity_atoms: u64,
    limits: WriterIssueAdmissionLimits,
) -> WriterMathResult<u64> {
    validate_series_book(series)?;
    if series_index >= series.len()
        || maximum_quantity_atoms % WRITER_CONTRACT_ATOMIC_SCALE != 0
        || limits.writer_principal_atoms == 0
    {
        return Err(WriterMathError::InvalidSeries);
    }
    let candidates = canonical_candidate_points(
        series,
        limits.lower_tail_max_settlement_atomic,
        limits.upper_tail_min_settlement_atomic,
    )?;
    let solvent_base = match limits
        .accounted_asset_atoms
        .checked_sub(limits.operational_buffer_atoms)
    {
        Some(value) => value,
        None => return Ok(0),
    };
    let full_drawdown_base = limits
        .locked_primary_premium_atoms
        .checked_add(drawdown_allowance_atoms(
            limits.writer_principal_atoms,
            limits.worst_drawdown_limit,
        )?)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let lower_drawdown_base = limits
        .locked_primary_premium_atoms
        .checked_add(drawdown_allowance_atoms(
            limits.writer_principal_atoms,
            limits.lower_drawdown_limit,
        )?)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let upper_drawdown_base = limits
        .locked_primary_premium_atoms
        .checked_add(drawdown_allowance_atoms(
            limits.writer_principal_atoms,
            limits.upper_drawdown_limit,
        )?)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let target = &series[series_index];
    let mut maximum_contracts = maximum_quantity_atoms / WRITER_CONTRACT_ATOMIC_SCALE;

    if limits.security_mode == WriterSecurityMode::GrossExternalMaximumPayout {
        let gross_base = gross_external_maximum_payout(series)?;
        restrict_linear_admission(
            &mut maximum_contracts,
            gross_base,
            target.max_payout_per_contract_atoms,
            limits.security_cap_atoms,
            0,
        );
    }

    for settlement in candidates.as_slice() {
        let liability_base = liability_atoms_from_numerator(
            aggregate_liability_numerator_unchecked(series, *settlement)?,
        )?;
        let liability_per_contract = payout_per_contract_unchecked(target, *settlement);
        restrict_linear_admission(
            &mut maximum_contracts,
            liability_base,
            liability_per_contract,
            solvent_base,
            bid_price_per_contract_atoms,
        );
        restrict_linear_admission(
            &mut maximum_contracts,
            liability_base,
            liability_per_contract,
            full_drawdown_base,
            bid_price_per_contract_atoms,
        );
        if *settlement <= limits.lower_tail_max_settlement_atomic {
            restrict_linear_admission(
                &mut maximum_contracts,
                liability_base,
                liability_per_contract,
                lower_drawdown_base,
                bid_price_per_contract_atoms,
            );
        }
        if *settlement >= limits.upper_tail_min_settlement_atomic {
            restrict_linear_admission(
                &mut maximum_contracts,
                liability_base,
                liability_per_contract,
                upper_drawdown_base,
                bid_price_per_contract_atoms,
            );
        }
        if limits.security_mode == WriterSecurityMode::ExactExternalEnvelope {
            restrict_linear_admission(
                &mut maximum_contracts,
                liability_base,
                liability_per_contract,
                limits.security_cap_atoms,
                0,
            );
        }
    }

    maximum_contracts
        .checked_mul(WRITER_CONTRACT_ATOMIC_SCALE)
        .ok_or(WriterMathError::ArithmeticOverflow)
}

#[inline]
fn drawdown_gate(
    reserve_atoms: u64,
    locked_premium_atoms: u64,
    writer_principal_atoms: u64,
    limit_ppm: u64,
) -> WriterMathResult<bool> {
    if limit_ppm > WRITER_RATIO_SCALE_PPM {
        return Err(WriterMathError::InvalidRiskLimit);
    }
    let exposed = reserve_atoms.saturating_sub(locked_premium_atoms);
    let left = u128::from(exposed)
        .checked_mul(u128::from(WRITER_RATIO_SCALE_PPM))
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let right = u128::from(writer_principal_atoms)
        .checked_mul(u128::from(limit_ppm))
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    Ok(left <= right)
}

pub fn drawdown_checks(
    reserves: &WriterReserveSummary,
    locked_premium_atoms: u64,
    writer_principal_atoms: u64,
    worst_limit_ppm: u64,
    lower_limit_ppm: u64,
    upper_limit_ppm: u64,
) -> WriterMathResult<WriterDrawdownChecks> {
    if writer_principal_atoms == 0 {
        return Err(WriterMathError::InvalidFlatSupply);
    }
    Ok(WriterDrawdownChecks {
        full_book_passes: drawdown_gate(
            reserves.reserve_atoms,
            locked_premium_atoms,
            writer_principal_atoms,
            worst_limit_ppm,
        )?,
        lower_tail_passes: drawdown_gate(
            reserves.lower_tail_reserve_atoms,
            locked_premium_atoms,
            writer_principal_atoms,
            lower_limit_ppm,
        )?,
        upper_tail_passes: drawdown_gate(
            reserves.upper_tail_reserve_atoms,
            locked_premium_atoms,
            writer_principal_atoms,
            upper_limit_ppm,
        )?,
    })
}

pub fn proportional_close_basket(
    series: &[WriterSeries],
    flat_supply_atoms: u64,
    flat_to_burn_atoms: u64,
) -> WriterMathResult<WriterSeriesAmounts> {
    validate_series_book(series)?;
    if flat_supply_atoms == 0 {
        return Err(WriterMathError::InvalidFlatSupply);
    }
    if flat_to_burn_atoms == 0 || flat_to_burn_atoms >= flat_supply_atoms {
        return Err(WriterMathError::InvalidCloseAmount);
    }
    let mut required = WriterSeriesAmounts {
        len: u8::try_from(series.len()).map_err(|_| WriterMathError::TooManySeries)?,
        ..WriterSeriesAmounts::default()
    };
    for (index, item) in series.iter().enumerate() {
        required.values[index] = checked_ceil_mul_div(
            flat_to_burn_atoms,
            item.external_oi_atoms,
            flat_supply_atoms,
        )?;
        if required.values[index] > item.external_oi_atoms {
            return Err(WriterMathError::CloseBasketExceedsOpenInterest);
        }
    }
    Ok(required)
}

fn validate_close_books(before: &[WriterSeries], after: &[WriterSeries]) -> WriterMathResult<()> {
    validate_series_book(before)?;
    validate_series_book(after)?;
    if before.len() != after.len() {
        return Err(WriterMathError::IncompatibleSeriesBooks);
    }
    for (old, new) in before.iter().zip(after) {
        if !old.same_instrument(new) || new.external_oi_atoms > old.external_oi_atoms {
            return Err(WriterMathError::IncompatibleSeriesBooks);
        }
    }
    Ok(())
}

/// Exact largest integer withdrawal safe at every canonical settlement state.
///
/// The comparison is performed in liability-numerator units, before liability rounding:
///
/// `S*x*C <= p*(A*C-N_before) + S*(N_before-N_after)`.
///
/// Both sides are piecewise linear between payoff breakpoints, so the bounded candidate walk is
/// complete. Delaying division also avoids the interior rounding extrema that appear when each
/// series or each state term is floored independently.
pub fn statewise_safe_withdrawal(
    assets_atoms: u64,
    flat_supply_atoms: u64,
    flat_to_burn_atoms: u64,
    before: &[WriterSeries],
    after: &[WriterSeries],
    candidates: &WriterCandidatePoints,
) -> WriterMathResult<(u64, u64)> {
    validate_close_books(before, after)?;
    if flat_supply_atoms == 0 {
        return Err(WriterMathError::InvalidFlatSupply);
    }
    if flat_to_burn_atoms == 0 || flat_to_burn_atoms >= flat_supply_atoms {
        return Err(WriterMathError::InvalidCloseAmount);
    }
    if candidates.is_empty() {
        return Err(WriterMathError::TooManyCandidatePoints);
    }

    let contract_scale = u128::from(WRITER_CONTRACT_ATOMIC_SCALE);
    let assets_numerator = u128::from(assets_atoms)
        .checked_mul(contract_scale)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let denominator = u128::from(flat_supply_atoms)
        .checked_mul(contract_scale)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let mut minimum = u64::MAX;
    let mut binding_settlement = 0u64;

    for settlement in candidates.as_slice() {
        let before_numerator = aggregate_liability_numerator_unchecked(before, *settlement)?;
        let after_numerator = aggregate_liability_numerator_unchecked(after, *settlement)?;
        if before_numerator > assets_numerator || after_numerator > before_numerator {
            return Err(WriterMathError::Insolvent);
        }
        let remaining_equity_numerator = assets_numerator
            .checked_sub(before_numerator)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        let retired_numerator = before_numerator
            .checked_sub(after_numerator)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        let numerator = u128::from(flat_to_burn_atoms)
            .checked_mul(remaining_equity_numerator)
            .and_then(|value| {
                u128::from(flat_supply_atoms)
                    .checked_mul(retired_numerator)
                    .and_then(|retired| value.checked_add(retired))
            })
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        let safe = u64::try_from(numerator / denominator)
            .map_err(|_| WriterMathError::ArithmeticOverflow)?;
        if safe < minimum {
            minimum = safe;
            binding_settlement = *settlement;
        }
    }
    Ok((minimum, binding_settlement))
}

pub fn proportional_close_preview(
    assets_atoms: u64,
    flat_supply_atoms: u64,
    flat_to_burn_atoms: u64,
    series: &[WriterSeries],
    lower_tail_max_settlement_atomic: u64,
    upper_tail_min_settlement_atomic: u64,
) -> WriterMathResult<WriterClosePreview> {
    let required = proportional_close_basket(series, flat_supply_atoms, flat_to_burn_atoms)?;
    // The complete post-close copy is fixed-capacity but heap-backed for the same SBF stack-frame
    // reason as the candidate array.
    let mut after_storage = vec![WriterSeries::EMPTY; WRITER_SERIES_CAPACITY].into_boxed_slice();
    for (index, item) in series.iter().enumerate() {
        after_storage[index] = *item;
        after_storage[index].external_oi_atoms = item
            .external_oi_atoms
            .checked_sub(required.values[index])
            .ok_or(WriterMathError::CloseBasketExceedsOpenInterest)?;
    }
    let after = &after_storage[..series.len()];
    let candidates = canonical_candidate_points(
        series,
        lower_tail_max_settlement_atomic,
        upper_tail_min_settlement_atomic,
    )?;
    let candidate_count =
        u16::try_from(candidates.len()).map_err(|_| WriterMathError::TooManyCandidatePoints)?;
    let contract_scale = u128::from(WRITER_CONTRACT_ATOMIC_SCALE);
    let assets_numerator = u128::from(assets_atoms)
        .checked_mul(contract_scale)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let safe_denominator = u128::from(flat_supply_atoms)
        .checked_mul(contract_scale)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let mut before_reserve = WriterReserveSummary {
        candidate_count,
        ..WriterReserveSummary::default()
    };
    let mut after_reserve = before_reserve;
    let mut saw_lower = false;
    let mut saw_upper = false;
    let mut safe = u64::MAX;
    let mut binding_settlement = 0u64;

    // One fused candidate walk supplies every value needed by economic finalization. This is not
    // an alternate reserve path: it applies the same aggregate numerator and ceiling as
    // `exact_reserve`, while avoiding two duplicate 32-series SBF walks.
    for settlement in candidates.as_slice() {
        let (before_numerator, after_numerator) =
            paired_liability_numerators_unchecked(series, after, *settlement)?;
        if before_numerator > assets_numerator || after_numerator > before_numerator {
            return Err(WriterMathError::Insolvent);
        }
        let before_liability = liability_atoms_from_numerator(before_numerator)?;
        let after_liability = liability_atoms_from_numerator(after_numerator)?;
        if before_liability > before_reserve.reserve_atoms {
            before_reserve.reserve_atoms = before_liability;
            before_reserve.reserve_settlement_atomic = *settlement;
        }
        if after_liability > after_reserve.reserve_atoms {
            after_reserve.reserve_atoms = after_liability;
            after_reserve.reserve_settlement_atomic = *settlement;
        }
        if *settlement <= lower_tail_max_settlement_atomic {
            if !saw_lower || before_liability > before_reserve.lower_tail_reserve_atoms {
                before_reserve.lower_tail_reserve_atoms = before_liability;
                before_reserve.lower_tail_settlement_atomic = *settlement;
            }
            if !saw_lower || after_liability > after_reserve.lower_tail_reserve_atoms {
                after_reserve.lower_tail_reserve_atoms = after_liability;
                after_reserve.lower_tail_settlement_atomic = *settlement;
            }
            saw_lower = true;
        }
        if *settlement >= upper_tail_min_settlement_atomic {
            if !saw_upper || before_liability > before_reserve.upper_tail_reserve_atoms {
                before_reserve.upper_tail_reserve_atoms = before_liability;
                before_reserve.upper_tail_settlement_atomic = *settlement;
            }
            if !saw_upper || after_liability > after_reserve.upper_tail_reserve_atoms {
                after_reserve.upper_tail_reserve_atoms = after_liability;
                after_reserve.upper_tail_settlement_atomic = *settlement;
            }
            saw_upper = true;
        }

        let remaining_equity_numerator = assets_numerator
            .checked_sub(before_numerator)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        let retired_numerator = before_numerator
            .checked_sub(after_numerator)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        let safe_numerator = u128::from(flat_to_burn_atoms)
            .checked_mul(remaining_equity_numerator)
            .and_then(|value| {
                u128::from(flat_supply_atoms)
                    .checked_mul(retired_numerator)
                    .and_then(|retired| value.checked_add(retired))
            })
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        let candidate_safe = u64::try_from(safe_numerator / safe_denominator)
            .map_err(|_| WriterMathError::ArithmeticOverflow)?;
        if candidate_safe < safe {
            safe = candidate_safe;
            binding_settlement = *settlement;
        }
    }
    if !saw_lower || !saw_upper {
        return Err(WriterMathError::InvalidTailBoundaries);
    }
    let reserve_released = before_reserve
        .reserve_atoms
        .checked_sub(after_reserve.reserve_atoms)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let withdrawal = min(flat_to_burn_atoms, min(reserve_released, safe));
    if withdrawal == 0 {
        return Err(WriterMathError::WithdrawalIsZero);
    }
    Ok(WriterClosePreview {
        flat_to_burn_atoms,
        withdrawal_atoms: withdrawal,
        reserve_before_atoms: before_reserve.reserve_atoms,
        reserve_after_atoms: after_reserve.reserve_atoms,
        lower_tail_reserve_before_atoms: before_reserve.lower_tail_reserve_atoms,
        lower_tail_reserve_after_atoms: after_reserve.lower_tail_reserve_atoms,
        upper_tail_reserve_before_atoms: before_reserve.upper_tail_reserve_atoms,
        upper_tail_reserve_after_atoms: after_reserve.upper_tail_reserve_atoms,
        reserve_released_atoms: reserve_released,
        statewise_safe_withdrawal_atoms: safe,
        statewise_binding_settlement_atomic: binding_settlement,
        candidate_count,
        required_claim_atoms: required,
    })
}

/// Conservative gross external maximum payout.
///
/// Unlike the exact shared-settlement envelope, gross mode deliberately rounds each series up
/// before summing. That makes it a conservative sum of independently payable maxima and keeps
/// the audited V1 oracle-security mode at least as large as the exact external envelope.
pub fn gross_external_maximum_payout(series: &[WriterSeries]) -> WriterMathResult<u64> {
    validate_series_book(series)?;
    let mut total = 0u64;
    for item in series {
        let numerator = u128::from(item.external_oi_atoms)
            .checked_mul(u128::from(item.max_payout_per_contract_atoms))
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        let series_max = liability_atoms_from_numerator(numerator)?;
        total = total
            .checked_add(series_max)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
    }
    Ok(total)
}

pub fn security_exposure(
    mode: WriterSecurityMode,
    series: &[WriterSeries],
    exact_external_reserve_atoms: u64,
) -> WriterMathResult<u64> {
    match mode {
        WriterSecurityMode::GrossExternalMaximumPayout => gross_external_maximum_payout(series),
        WriterSecurityMode::ExactExternalEnvelope => Ok(exact_external_reserve_atoms),
    }
}

/// Allocate the aggregate upward-rounded long liability to canonical series order.
///
/// Cumulative ceilings make every allocation nonnegative and make the series totals sum exactly
/// to the complete-book liability. Claimants within each series subsequently use cumulative
/// allocation, so consuming the full supply pays the exact assigned series ledger.
pub fn settlement_series_liabilities(
    series: &[WriterSeries],
    settlement_price_atomic: u64,
) -> WriterMathResult<(WriterSeriesAmounts, u64)> {
    validate_series_book(series)?;
    let mut result = WriterSeriesAmounts {
        len: u8::try_from(series.len()).map_err(|_| WriterMathError::TooManySeries)?,
        ..WriterSeriesAmounts::default()
    };
    let mut prefix_numerator = 0u128;
    let mut previous_allocation = 0u64;
    for (index, item) in series.iter().enumerate() {
        let term = u128::from(item.external_oi_atoms)
            .checked_mul(u128::from(payout_per_contract_unchecked(
                item,
                settlement_price_atomic,
            )))
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        prefix_numerator = prefix_numerator
            .checked_add(term)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        let prefix_allocation = liability_atoms_from_numerator(prefix_numerator)?;
        result.values[index] = prefix_allocation
            .checked_sub(previous_allocation)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        previous_allocation = prefix_allocation;
    }
    Ok((result, previous_allocation))
}

/// Cumulative claimant allocation. The final consumption receives all remaining class dust.
pub fn cumulative_allocation_delta(
    initial_quantity_atoms: u64,
    remaining_quantity_before_atoms: u64,
    consumed_quantity_atoms: u64,
    initial_liability_atoms: u64,
) -> WriterMathResult<u64> {
    if initial_quantity_atoms == 0
        || remaining_quantity_before_atoms > initial_quantity_atoms
        || consumed_quantity_atoms > remaining_quantity_before_atoms
    {
        return Err(WriterMathError::InvalidCloseAmount);
    }
    let consumed_before = initial_quantity_atoms
        .checked_sub(remaining_quantity_before_atoms)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let consumed_after = consumed_before
        .checked_add(consumed_quantity_atoms)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let before = u128::from(consumed_before)
        .checked_mul(u128::from(initial_liability_atoms))
        .ok_or(WriterMathError::ArithmeticOverflow)?
        / u128::from(initial_quantity_atoms);
    let after = u128::from(consumed_after)
        .checked_mul(u128::from(initial_liability_atoms))
        .ok_or(WriterMathError::ArithmeticOverflow)?
        / u128::from(initial_quantity_atoms);
    u64::try_from(
        after
            .checked_sub(before)
            .ok_or(WriterMathError::ArithmeticOverflow)?,
    )
    .map_err(|_| WriterMathError::ArithmeticOverflow)
}

/// Feature-gated SBF harness for the actual 32-series close-preview hot path.
///
/// The release artifact never enables `writer-math-benchmark`, so this exact byte string still
/// reaches the generic invalid-instruction path in production. The benchmark build is emitted to
/// a temporary directory and is never a deployment candidate.
#[cfg(feature = "writer-math-benchmark")]
pub fn process_sbf_benchmark(series_count: u8) -> solana_program::entrypoint::ProgramResult {
    use solana_program::{log::sol_log_compute_units, program::set_return_data};

    let series_count = usize::from(series_count);
    if series_count == 0 || series_count > WRITER_MAX_SERIES {
        return Err(solana_program::program_error::ProgramError::InvalidArgument);
    }
    let mut series = vec![WriterSeries::EMPTY; WRITER_SERIES_CAPACITY].into_boxed_slice();
    for (index, item) in series.iter_mut().take(series_count).enumerate() {
        let index_u64 = u64::try_from(index)
            .map_err(|_| solana_program::program_error::ProgramError::InvalidArgument)?;
        let external_oi_atoms = index_u64
            .checked_add(1)
            .and_then(|value| value.checked_mul(WRITER_CONTRACT_ATOMIC_SCALE))
            .ok_or(solana_program::program_error::ProgramError::InvalidArgument)?;
        *item = if index % 2 == 0 {
            let local_index = index_u64 / 2;
            let strike = 50_000_000u64
                .checked_add(
                    local_index
                        .checked_mul(5_000_000)
                        .ok_or(solana_program::program_error::ProgramError::InvalidArgument)?,
                )
                .ok_or(solana_program::program_error::ProgramError::InvalidArgument)?;
            WriterSeries {
                kind: OptionKind::CallSpread,
                strike_price_atomic: strike,
                cap_price_atomic: strike
                    .checked_add(2_000_000)
                    .ok_or(solana_program::program_error::ProgramError::InvalidArgument)?,
                contract_size_atoms: WRITER_CONTRACT_ATOMIC_SCALE,
                max_payout_per_contract_atoms: 2_000_000,
                external_oi_atoms,
            }
        } else {
            let local_index = index_u64 / 2;
            let strike = 200_000_000u64
                .checked_add(
                    local_index
                        .checked_mul(5_000_000)
                        .ok_or(solana_program::program_error::ProgramError::InvalidArgument)?,
                )
                .ok_or(solana_program::program_error::ProgramError::InvalidArgument)?;
            WriterSeries {
                kind: OptionKind::PutSpread,
                strike_price_atomic: strike,
                cap_price_atomic: strike
                    .checked_sub(3_000_000)
                    .ok_or(solana_program::program_error::ProgramError::InvalidArgument)?,
                contract_size_atoms: WRITER_CONTRACT_ATOMIC_SCALE,
                max_payout_per_contract_atoms: 3_000_000,
                external_oi_atoms,
            }
        };
    }

    sol_log_compute_units();
    let preview = proportional_close_preview(
        5_000_000_000,
        1_000_000_000,
        100_000_000,
        &series[..series_count],
        25_000_000,
        300_000_000,
    )
    .map_err(|_| solana_program::program_error::ProgramError::InvalidArgument)?;
    let gross = gross_external_maximum_payout(&series[..series_count])
        .map_err(|_| solana_program::program_error::ProgramError::InvalidArgument)?;
    sol_log_compute_units();

    let mut receipt = [0u8; 26];
    receipt[..2].copy_from_slice(&preview.candidate_count.to_le_bytes());
    receipt[2..10].copy_from_slice(&preview.reserve_before_atoms.to_le_bytes());
    receipt[10..18].copy_from_slice(&preview.withdrawal_atoms.to_le_bytes());
    receipt[18..26].copy_from_slice(&gross.to_le_bytes());
    set_return_data(&receipt);
    Ok(())
}
