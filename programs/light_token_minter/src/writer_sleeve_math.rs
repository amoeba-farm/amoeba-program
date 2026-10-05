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
    InvalidClaimAmount,
    Insolvent,
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
    pub(crate) fn same_instrument(&self, other: &Self) -> bool {
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
    /// Legacy caller field; not used as an exposure admission limit.
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

#[inline(never)]
fn checked_ceil_div(numerator: u128, denominator: u128) -> WriterMathResult<u128> {
    if denominator == 0 {
        return Err(WriterMathError::DivisionByZero);
    }
    let quotient = numerator / denominator;
    quotient
        .checked_add(u128::from(!numerator.is_multiple_of(denominator)))
        .ok_or(WriterMathError::ArithmeticOverflow)
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
    for (index, item) in series.iter().enumerate() {
        validate_series(item)?;
        if series[..index]
            .iter()
            .any(|previous| item.same_instrument(previous))
        {
            return Err(WriterMathError::InvalidSeries);
        }
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

/// Complete-book liability before the single canonical contract-scale division.
pub fn aggregate_liability_numerator(
    series: &[WriterSeries],
    settlement_price_atomic: u64,
) -> WriterMathResult<u128> {
    validate_series_book(series)?;
    aggregate_liability_numerator_unchecked(series, settlement_price_atomic)
}

#[inline(never)]
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

/// Frozen geometry and other-series numerators for repeated changes to one OI.
/// The complete-book numerator is still rounded once, exactly as `exact_reserve`.
pub(crate) struct PreparedWriterReserve {
    points: Vec<(u64, u128, u64)>,
    lower: u64,
    upper: u64,
    target_max: u64,
    other_gross: u64,
    pub(crate) target: usize,
    pub(crate) initial_oi: u64,
}

/// One immutable payoff grid for a progressive, internally authenticated strip.
/// Numerators retain whole-book rounding; every update is an exact OI delta.
pub(crate) struct PreparedWriterStrip {
    book: Vec<WriterSeries>,
    points: Vec<(u64, u128)>,
    payouts: Vec<u64>,
    columns: [u8; WRITER_MAX_SERIES],
    column_count: usize,
    lower: u64,
    upper: u64,
    gross: u64,
}

impl PreparedWriterStrip {
    pub(crate) fn for_targets(
        book: &[WriterSeries],
        lower: u64,
        upper: u64,
        targets: &[usize],
    ) -> WriterMathResult<Self> {
        let candidates = canonical_candidate_points(book, lower, upper)?;
        if targets.is_empty() || targets.len() > book.len() {
            return Err(WriterMathError::InvalidSeries);
        }
        let mut columns = [u8::MAX; WRITER_MAX_SERIES];
        for (column, &target) in targets.iter().enumerate() {
            if target >= book.len() || columns[target] != u8::MAX {
                return Err(WriterMathError::InvalidSeries);
            }
            columns[target] = column as u8;
        }
        let mut points = Vec::with_capacity(candidates.len());
        let mut payouts = Vec::with_capacity(candidates.len() * targets.len());
        for &settlement in candidates.as_slice() {
            let mut numerator = 0u128;
            let row_start = payouts.len();
            for &target in targets {
                payouts.push(payout_per_contract_unchecked(&book[target], settlement));
            }
            // Every existing liability remains in the full-book numerator.
            // Zero OI contributes exactly zero and needs no multiplication.
            for (index, item) in book
                .iter()
                .enumerate()
                .filter(|(_, item)| item.external_oi_atoms != 0)
            {
                let column = usize::from(columns[index]);
                let payout = if column < targets.len() {
                    payouts[row_start + column]
                } else {
                    payout_per_contract_unchecked(item, settlement)
                };
                numerator = numerator
                    .checked_add(u128::from(item.external_oi_atoms) * u128::from(payout))
                    .ok_or(WriterMathError::ArithmeticOverflow)?;
            }
            points.push((settlement, numerator));
        }
        Ok(Self {
            book: book.to_vec(),
            points,
            payouts,
            columns,
            column_count: targets.len(),
            lower,
            upper,
            gross: gross_external_maximum_payout(book)?,
        })
    }

    pub(crate) fn prepare(
        &self,
        book: &[WriterSeries],
        target: usize,
    ) -> WriterMathResult<PreparedWriterReserve> {
        if book != self.book {
            return Err(WriterMathError::InvalidSeries);
        }
        let item = book.get(target).ok_or(WriterMathError::InvalidSeries)?;
        let column = usize::from(self.columns[target]);
        if column >= self.column_count {
            return Err(WriterMathError::InvalidSeries);
        }
        let points = self
            .points
            .iter()
            .enumerate()
            .map(|(point, (settlement, total))| {
                let payout = self.payouts[point * self.column_count + column];
                let other = total
                    .checked_sub(u128::from(item.external_oi_atoms) * u128::from(payout))
                    .ok_or(WriterMathError::ArithmeticOverflow)?;
                Ok((*settlement, other, payout))
            })
            .collect::<WriterMathResult<Vec<_>>>()?;
        let own_gross = liability_atoms_from_numerator(
            u128::from(item.external_oi_atoms) * u128::from(item.max_payout_per_contract_atoms),
        )?;
        Ok(PreparedWriterReserve {
            points,
            lower: self.lower,
            upper: self.upper,
            target_max: item.max_payout_per_contract_atoms,
            other_gross: self
                .gross
                .checked_sub(own_gross)
                .ok_or(WriterMathError::ArithmeticOverflow)?,
            target,
            initial_oi: item.external_oi_atoms,
        })
    }

    pub(crate) fn update(&mut self, target: usize, oi: u64) -> WriterMathResult<()> {
        let item = self
            .book
            .get(target)
            .ok_or(WriterMathError::InvalidSeries)?;
        let old = item.external_oi_atoms;
        let column = usize::from(self.columns[target]);
        if column >= self.column_count {
            return Err(WriterMathError::InvalidSeries);
        }
        let old_gross = liability_atoms_from_numerator(
            u128::from(old) * u128::from(item.max_payout_per_contract_atoms),
        )?;
        let new_gross = liability_atoms_from_numerator(
            u128::from(oi) * u128::from(item.max_payout_per_contract_atoms),
        )?;
        let gross = self
            .gross
            .checked_sub(old_gross)
            .and_then(|v| v.checked_add(new_gross))
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        let totals = self
            .points
            .iter()
            .enumerate()
            .map(|(point, (_, total))| {
                let payout = self.payouts[point * self.column_count + column];
                total
                    .checked_sub(u128::from(old) * u128::from(payout))
                    .and_then(|v| v.checked_add(u128::from(oi) * u128::from(payout)))
                    .ok_or(WriterMathError::ArithmeticOverflow)
            })
            .collect::<WriterMathResult<Vec<_>>>()?;
        for (point, total) in self.points.iter_mut().zip(totals) {
            point.1 = total;
        }
        self.book[target].external_oi_atoms = oi;
        self.gross = gross;
        Ok(())
    }
}

impl PreparedWriterReserve {
    pub(crate) fn matches_bounds(&self, lower: u64, upper: u64) -> bool {
        self.lower == lower && self.upper == upper
    }
    pub(crate) fn new(
        book: &[WriterSeries],
        target: usize,
        lower: u64,
        upper: u64,
    ) -> WriterMathResult<Self> {
        let candidates = canonical_candidate_points(book, lower, upper)?;
        let selected = book.get(target).ok_or(WriterMathError::InvalidSeries)?;
        let mut other_gross = 0u64;
        for (index, item) in book
            .iter()
            .enumerate()
            .filter(|(index, item)| *index != target && item.external_oi_atoms != 0)
        {
            let _ = index;
            other_gross = other_gross
                .checked_add(liability_atoms_from_numerator(
                    u128::from(item.external_oi_atoms)
                        * u128::from(item.max_payout_per_contract_atoms),
                )?)
                .ok_or(WriterMathError::ArithmeticOverflow)?;
        }
        let mut points = Vec::with_capacity(candidates.len());
        for &settlement in candidates.as_slice() {
            let mut other = 0u128;
            for (index, item) in book.iter().enumerate() {
                if index != target && item.external_oi_atoms != 0 {
                    other = other
                        .checked_add(
                            u128::from(item.external_oi_atoms)
                                * u128::from(payout_per_contract_unchecked(item, settlement)),
                        )
                        .ok_or(WriterMathError::ArithmeticOverflow)?;
                }
            }
            points.push((
                settlement,
                other,
                payout_per_contract_unchecked(selected, settlement),
            ));
        }
        Ok(Self {
            points,
            lower,
            upper,
            target_max: selected.max_payout_per_contract_atoms,
            other_gross,
            target,
            initial_oi: selected.external_oi_atoms,
        })
    }

    pub(crate) fn reserve(&self, target_oi: u64) -> WriterMathResult<WriterReserveSummary> {
        let mut maxima = [0u128; 3];
        let mut saw_lower = false;
        let mut saw_upper = false;
        for &(settlement, other, payout) in &self.points {
            let numerator = other
                .checked_add(u128::from(target_oi) * u128::from(payout))
                .ok_or(WriterMathError::ArithmeticOverflow)?;
            maxima[0] = maxima[0].max(numerator);
            if settlement <= self.lower {
                saw_lower = true;
                maxima[1] = maxima[1].max(numerator);
            }
            if settlement >= self.upper {
                saw_upper = true;
                maxima[2] = maxima[2].max(numerator);
            }
        }
        if !saw_lower || !saw_upper {
            return Err(WriterMathError::InvalidTailBoundaries);
        }
        let reserves = [
            liability_atoms_from_numerator(maxima[0])?,
            liability_atoms_from_numerator(maxima[1])?,
            liability_atoms_from_numerator(maxima[2])?,
        ];
        let thresholds = reserves.map(|reserve| {
            u128::from(reserve.saturating_sub(1)) * u128::from(WRITER_CONTRACT_ATOMIC_SCALE)
        });
        let mut result = WriterReserveSummary {
            candidate_count: self.points.len() as u16,
            reserve_atoms: reserves[0],
            lower_tail_reserve_atoms: reserves[1],
            upper_tail_reserve_atoms: reserves[2],
            ..Default::default()
        };
        // Ceil ties need the earliest candidate attaining the rounded maximum,
        // not necessarily the candidate with the largest unrounded numerator.
        let mut found = [reserves[0] == 0, false, false];
        for &(settlement, other, payout) in &self.points {
            // The immutable first pass already checked every addition.
            let numerator = other + u128::from(target_oi) * u128::from(payout);
            if !found[0] && numerator > thresholds[0] {
                found[0] = true;
                result.reserve_settlement_atomic = settlement;
            }
            if !found[1]
                && settlement <= self.lower
                && (reserves[1] == 0 || numerator > thresholds[1])
            {
                found[1] = true;
                result.lower_tail_settlement_atomic = settlement;
            }
            if !found[2]
                && settlement >= self.upper
                && (reserves[2] == 0 || numerator > thresholds[2])
            {
                found[2] = true;
                result.upper_tail_settlement_atomic = settlement;
            }
            if found == [true; 3] {
                break;
            }
        }
        Ok(result)
    }
    pub(crate) fn gross(&self, target_oi: u64) -> WriterMathResult<u64> {
        self.other_gross
            .checked_add(liability_atoms_from_numerator(
                u128::from(target_oi) * u128::from(self.target_max),
            )?)
            .ok_or(WriterMathError::ArithmeticOverflow)
    }
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
        || !maximum_quantity_atoms.is_multiple_of(WRITER_CONTRACT_ATOMIC_SCALE)
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
        return Err(WriterMathError::InvalidClaimAmount);
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
