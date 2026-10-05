//! Owner-scoped collateral for written options and options locked in program custody.
//! A single signed payoff function covers every strategy; no wallet balance is a hedge.
use crate::fixed_codec::{
    fixed_state_deserialize, invalid_fixed_borsh, FixedCursor, FixedField, FixedStateDecode,
    FixedStateEncode, FixedWriter,
};
use crate::writer_sleeve_math::{
    canonical_candidate_points, payout_per_contract, WriterMathError, WriterMathResult,
    WriterSeries, WRITER_CONTRACT_ATOMIC_SCALE, WRITER_MAX_SERIES,
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

pub const PORTFOLIO_SEED: &[u8] = b"writer-portfolio-v1";
pub const PORTFOLIO_SERIES: usize = 20;

pub fn derive_portfolio(program: &Pubkey, book: &Pubkey, owner: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            PORTFOLIO_SEED,
            book.as_ref(),
            owner.as_ref(),
        ],
        program,
    )
}

/// Mathematical ceiling, including negative numerators. Rust division truncates toward zero.
#[inline(never)]
pub fn signed_liability_ceil(numerator: i128) -> i128 {
    let scale = i128::from(WRITER_CONTRACT_ATOMIC_SCALE);
    numerator / scale + i128::from(numerator % scale > 0)
}

fn validate_quantities(
    series: &[WriterSeries],
    short: &[u64; 20],
    long: &[u64; 20],
) -> WriterMathResult<()> {
    if series.is_empty() {
        return Err(WriterMathError::EmptySeriesBook);
    }
    if series.len() > WRITER_MAX_SERIES || series.len() > PORTFOLIO_SERIES {
        return Err(WriterMathError::TooManySeries);
    }
    if short[series.len()..]
        .iter()
        .chain(&long[series.len()..])
        .any(|q| *q != 0)
    {
        return Err(WriterMathError::InvalidSeries);
    }
    Ok(())
}

fn numerator_at(
    series: &[WriterSeries],
    short: &[u64; 20],
    long: &[u64; 20],
    price: u64,
) -> WriterMathResult<i128> {
    series.iter().enumerate().try_fold(0i128, |sum, (i, s)| {
        let net = i128::from(short[i]) - i128::from(long[i]);
        let term = net
            .checked_mul(i128::from(payout_per_contract(s, price)?))
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        sum.checked_add(term)
            .ok_or(WriterMathError::ArithmeticOverflow)
    })
}

pub fn portfolio_liability_numerator(
    series: &[WriterSeries],
    short: &[u64; 20],
    long: &[u64; 20],
    price: u64,
) -> WriterMathResult<i128> {
    validate_quantities(series, short, long)?;
    // Authenticates canonical terms and rejects duplicate instruments too.
    canonical_candidate_points(series, 0, u64::MAX)?;
    numerator_at(series, short, long, price)
}

/// Exact minimum cash against all possible settlement prices. Signed payoff is
/// piecewise affine, so its maximum occurs at a payoff breakpoint or a flat tail.
pub fn portfolio_reserve(
    series: &[WriterSeries],
    committed: &[u64; 20],
    locked: &[u64; 20],
) -> WriterMathResult<u64> {
    validate_quantities(series, committed, locked)?;
    let candidates = canonical_candidate_points(series, 0, u64::MAX)?;
    let mut reserve = 0i128;
    for price in candidates.as_slice() {
        reserve = reserve.max(signed_liability_ceil(numerator_at(
            series, committed, locked, *price,
        )?));
    }
    u64::try_from(reserve).map_err(|_| WriterMathError::ArithmeticOverflow)
}

/// Prefix allocation telescopes even when a portfolio is a net creditor. The
/// processor records credits and defers their payment until every owner funds.
pub fn portfolio_funding_delta(
    prefix: i128,
    owner_numerator: i128,
) -> WriterMathResult<(i128, i128)> {
    let next = prefix
        .checked_add(owner_numerator)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let delta = signed_liability_ceil(next)
        .checked_sub(signed_liability_ceil(prefix))
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    Ok((next, delta))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortfolioSettlementPartition {
    pub managed_liability: u64,
    pub writer_residual: u64,
    pub accounted_assets: u64,
    pub protected_reserve: u64,
    pub stranded: u64,
}

/// Separates managed writer P&L, ordinary holders, and deferred hedge credits.
/// Positive portfolio deposits never become profit for unrelated managed writers.
pub fn portfolio_settlement_partition(
    managed_assets: u64,
    managed_numerator: i128,
    owner_net_numerator: i128,
    positive_debits: u64,
    owner_credits: u64,
    outside_liability: u64,
) -> WriterMathResult<PortfolioSettlementPartition> {
    if managed_numerator < 0 {
        return Err(WriterMathError::InvalidClaimAmount);
    }
    let combined = managed_numerator
        .checked_add(owner_net_numerator)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    if combined < 0 {
        return Err(WriterMathError::InvalidClaimAmount);
    }
    let managed_liability = u64::try_from(signed_liability_ceil(managed_numerator))
        .map_err(|_| WriterMathError::ArithmeticOverflow)?;
    let combined_liability = u64::try_from(signed_liability_ceil(combined))
        .map_err(|_| WriterMathError::ArithmeticOverflow)?;
    if i128::from(positive_debits) - i128::from(owner_credits)
        != i128::from(combined_liability) - i128::from(managed_liability)
    {
        return Err(WriterMathError::Insolvent);
    }
    let writer_residual = managed_assets
        .checked_sub(managed_liability)
        .ok_or(WriterMathError::Insolvent)?;
    let protected_reserve = outside_liability
        .checked_add(owner_credits)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let accounted_assets = protected_reserve
        .checked_add(writer_residual)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let physical_assets = managed_assets
        .checked_add(positive_debits)
        .ok_or(WriterMathError::ArithmeticOverflow)?;
    let stranded = physical_assets
        .checked_sub(accounted_assets)
        .ok_or(WriterMathError::Insolvent)?;
    Ok(PortfolioSettlementPartition {
        managed_liability,
        writer_residual,
        accounted_assets,
        protected_reserve,
        stranded,
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IndividualWriterPortfolio {
    pub initialized: bool,
    pub bump: u8,
    pub discriminator: [u8; 3],
    pub version: u8,
    pub book: Pubkey,
    pub owner: Pubkey,
    pub expiry_ts: u64,
    pub cash_atoms: u64,
    pub committed: [u64; 20],
    pub filled: [u64; 20],
    pub open_positions: u64,
    pub settlement_funded: bool,
    pub claimed: bool,
    pub funding_debit: u64,
    pub premium_received: u64,
    /// Authenticated tokens transferred into this portfolio's compressed custody.
    pub locked: [u64; 20],
    /// Locked tokens irrevocably retired at expiry, before net funding.
    pub retired: [u64; 20],
    pub settlement_credit: u64,
    /// Prevents counting an owner twice when it first fills or locks a hedge.
    pub funding_registered: bool,
}

impl IndividualWriterPortfolio {
    pub const LEN: usize = 761;
    pub const DISCRIMINATOR: [u8; 3] = *b"IWF";
    pub const VERSION: u8 = 1;

    pub fn validate(&self) -> WriterMathResult<()> {
        if (0..20).any(|i| self.filled[i] > self.committed[i] || self.retired[i] > self.locked[i])
            || (self.funding_debit != 0 && self.settlement_credit != 0)
            || (self.settlement_funded && self.retired != self.locked)
            || (!self.settlement_funded && (self.funding_debit != 0 || self.settlement_credit != 0))
            || (self.claimed
                && (!self.settlement_funded || self.cash_atoms != 0 || self.settlement_credit != 0))
        {
            return Err(WriterMathError::InvalidClaimAmount);
        }
        Ok(())
    }

    fn active(&self) -> WriterMathResult<()> {
        self.validate()?;
        if self.settlement_funded || self.claimed || self.retired.iter().any(|q| *q != 0) {
            return Err(WriterMathError::InvalidClaimAmount);
        }
        Ok(())
    }

    fn top_up(mut self, series: &[WriterSeries], maximum: u64) -> WriterMathResult<(Self, u64)> {
        let reserve = portfolio_reserve(series, &self.committed, &self.locked)?;
        let debit = reserve.saturating_sub(self.cash_atoms);
        if debit > maximum {
            return Err(WriterMathError::Insolvent);
        }
        self.cash_atoms = self
            .cash_atoms
            .checked_add(debit)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        Ok((self, debit))
    }

    fn release_excess(mut self, series: &[WriterSeries]) -> WriterMathResult<(Self, u64)> {
        let reserve = portfolio_reserve(series, &self.committed, &self.locked)?;
        let refund = self
            .cash_atoms
            .checked_sub(reserve)
            .ok_or(WriterMathError::Insolvent)?;
        self.cash_atoms = reserve;
        Ok((self, refund))
    }

    pub fn opened(
        &self,
        series: &[WriterSeries],
        index: usize,
        quantity: u64,
        maximum: u64,
    ) -> WriterMathResult<(Self, u64)> {
        self.active()?;
        if quantity == 0 || index >= series.len() || index >= 20 {
            return Err(WriterMathError::InvalidSeries);
        }
        let mut next = self.clone();
        next.committed[index] = next.committed[index]
            .checked_add(quantity)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        next.top_up(series, maximum)
    }

    /// A fill never adds risk beyond already reserved commitments. There is no
    /// candidate-price walk or owner scan on the buyer's foreground path.
    pub fn filled(&self, index: usize, quantity: u64, premium: u64) -> WriterMathResult<Self> {
        self.active()?;
        if quantity == 0 || index >= 20 {
            return Err(WriterMathError::InvalidSeries);
        }
        let mut next = self.clone();
        next.filled[index] = next.filled[index]
            .checked_add(quantity)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        if next.filled[index] > next.committed[index] {
            return Err(WriterMathError::InvalidClaimAmount);
        }
        next.cash_atoms = next
            .cash_atoms
            .checked_add(premium)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        next.premium_received = next
            .premium_received
            .checked_add(premium)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        Ok(next)
    }

    pub fn cancelled(
        &self,
        series: &[WriterSeries],
        index: usize,
        unfilled: u64,
    ) -> WriterMathResult<(Self, u64)> {
        self.active()?;
        if index >= series.len() || index >= 20 {
            return Err(WriterMathError::InvalidSeries);
        }
        let mut next = self.clone();
        next.committed[index] = next.committed[index]
            .checked_sub(unfilled)
            .ok_or(WriterMathError::InvalidClaimAmount)?;
        if next.committed[index] < next.filled[index] {
            return Err(WriterMathError::InvalidClaimAmount);
        }
        next.release_excess(series)
    }

    pub fn locked(
        &self,
        series: &[WriterSeries],
        index: usize,
        quantity: u64,
    ) -> WriterMathResult<(Self, u64)> {
        self.active()?;
        if quantity == 0 || index >= series.len() || index >= 20 {
            return Err(WriterMathError::InvalidSeries);
        }
        let mut next = self.clone();
        next.locked[index] = next.locked[index]
            .checked_add(quantity)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        next.release_excess(series)
    }

    pub fn unlocked(
        &self,
        series: &[WriterSeries],
        index: usize,
        quantity: u64,
        maximum: u64,
    ) -> WriterMathResult<(Self, u64)> {
        self.active()?;
        if quantity == 0 || index >= series.len() || index >= 20 {
            return Err(WriterMathError::InvalidSeries);
        }
        let mut next = self.clone();
        next.locked[index] = next.locked[index]
            .checked_sub(quantity)
            .ok_or(WriterMathError::InvalidClaimAmount)?;
        next.top_up(series, maximum)
    }

    /// An exact-series buyback cannot offset more filled shorts than remain
    /// unhedged. Its price comes from authenticated asks, not from the wallet.
    /// All legs are priced and locked before evaluating the final reserve, so
    /// a shared-collateral basket needs no temporary deposit between its legs.
    pub fn bought_back(
        &self,
        series: &[WriterSeries],
        quantities: &[u64; 20],
        payment: u64,
        maximum_payment: u64,
        minimum_refund: u64,
    ) -> WriterMathResult<(Self, u64)> {
        self.active()?;
        if payment == 0 || payment > maximum_payment || quantities.iter().all(|q| *q == 0) {
            return Err(WriterMathError::InvalidClaimAmount);
        }
        let mut next = self.clone();
        next.cash_atoms = next
            .cash_atoms
            .checked_sub(payment)
            .ok_or(WriterMathError::Insolvent)?;
        for (index, quantity) in quantities.iter().enumerate() {
            if *quantity == 0 {
                continue;
            }
            if index >= series.len() {
                return Err(WriterMathError::InvalidSeries);
            }
            next.locked[index] = next.locked[index]
                .checked_add(*quantity)
                .ok_or(WriterMathError::ArithmeticOverflow)?;
            if next.locked[index] > next.filled[index] {
                return Err(WriterMathError::InvalidClaimAmount);
            }
        }
        let (next, refund) = next.release_excess(series)?;
        if refund < minimum_refund {
            return Err(WriterMathError::Insolvent);
        }
        Ok((next, refund))
    }

    pub fn retired(&self, index: usize, quantity: u64) -> WriterMathResult<Self> {
        self.validate()?;
        if self.settlement_funded || self.claimed || quantity == 0 || index >= 20 {
            return Err(WriterMathError::InvalidClaimAmount);
        }
        let mut next = self.clone();
        next.retired[index] = next.retired[index]
            .checked_add(quantity)
            .ok_or(WriterMathError::ArithmeticOverflow)?;
        if next.retired[index] > next.locked[index] {
            return Err(WriterMathError::InvalidClaimAmount);
        }
        Ok(next)
    }

    pub fn funded(
        &self,
        series: &[WriterSeries],
        price: u64,
        debit: u64,
        credit: u64,
    ) -> WriterMathResult<Self> {
        self.validate()?;
        if self.settlement_funded
            || self.claimed
            || self.retired != self.locked
            || (debit != 0 && credit != 0)
        {
            return Err(WriterMathError::InvalidClaimAmount);
        }
        let numerator = portfolio_liability_numerator(series, &self.filled, &self.locked, price)?;
        let delta = i128::from(debit) - i128::from(credit);
        if delta < numerator.div_euclid(i128::from(WRITER_CONTRACT_ATOMIC_SCALE))
            || delta > signed_liability_ceil(numerator)
        {
            return Err(WriterMathError::InvalidClaimAmount);
        }
        let mut next = self.clone();
        next.cash_atoms = next
            .cash_atoms
            .checked_sub(debit)
            .ok_or(WriterMathError::Insolvent)?;
        next.funding_debit = debit;
        next.settlement_credit = credit;
        next.settlement_funded = true;
        Ok(next)
    }

    /// The processor additionally requires group finalization before paying credits.
    pub fn claimed(&self) -> WriterMathResult<(Self, u64, u64)> {
        self.validate()?;
        if !self.settlement_funded || self.claimed {
            return Err(WriterMathError::InvalidClaimAmount);
        }
        let mut next = self.clone();
        next.cash_atoms = 0;
        next.settlement_credit = 0;
        next.claimed = true;
        Ok((next, self.cash_atoms, self.settlement_credit))
    }
}

fixed_state_deserialize!(IndividualWriterPortfolio, IndividualWriterPortfolio::LEN, {
    initialized: bool, bump: u8, discriminator: [u8; 3], version: u8, book: Pubkey, owner: Pubkey,
    expiry_ts: u64, cash_atoms: u64, committed: [u64; 20], filled: [u64; 20], open_positions: u64,
    settlement_funded: bool, claimed: bool, funding_debit: u64, premium_received: u64,
    locked: [u64; 20], retired: [u64; 20], settlement_credit: u64, funding_registered: bool,
});
