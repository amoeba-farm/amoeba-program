//! Earn Fund v2 share ledger: pure integer accounting with no Solana types.
//!
//! One fund vault holds every bucket. Users own shares of the fund's free cash
//! plus its writer receipts; pending deposits and redeemed-but-unpaid atoms are
//! separate par liabilities that never share in fund P&L.
//!
//! Lean pricing (the fallback): with nothing invested (`deployed_atoms == 0`)
//! the NAV is exactly `free_cash / shares`. While invested, prices come from
//! the two-sided buy-back marked values of the open slots kept by `SlotBook`
//! (an O(1) aggregate of per-sleeve valuations): exits (instant or filled at
//! a roll) at the lower NAV `(free cash + Σ lower) / S`, entries (immediate
//! conversions and roll conversions of pending deposits) at the upper NAV
//! `(free cash + Σ upper) / S`. While the true value lies between the two,
//! no entry or exit moves value between holders.
//!
//! Every division floors in the fund's favour: deposits mint
//! `floor(atoms * S / A)` shares and redemptions pay `floor(shares * A / S)`
//! atoms. Queued redemptions form batches (one per epoch in which they were
//! requested). At each roll every open batch is filled by the same pro-rata
//! share of the cash available; a member of a batch owns the fixed fraction
//! `q / batch.shares` of everything filled in that batch, so lazy settlement
//! is exact and needs no per-roll history. Rounding never lets the sum of
//! member entitlements exceed what the batch was paid, and completed batches
//! return their rounding dust to free cash once every member has settled.

use crate::writer_participation_math::ContributionInterval;

pub const BPS_DENOMINATOR: u64 = 10_000;
pub const DAY_SECS: u64 = 86_400;
/// Open (partially filled) redemption batches kept in the fund.
pub const MAX_OPEN_BATCHES: usize = 4;
/// Batches one roll can complete: every open batch plus the accumulating one.
pub const MAX_COMPLETED_BATCHES: usize = MAX_OPEN_BATCHES + 1;
pub const INSTANT_WINDOW_SECS: u64 = DAY_SECS;
/// Admin bounds (spec v2 "Admin bounds").
pub const MIN_TENOR_FLOOR_SECS: u64 = 3_600;
pub const MAX_TENOR_SECS: u64 = 120 * DAY_SECS;
pub const MIN_EPOCH_SECS: u64 = 20 * DAY_SECS;
pub const MAX_EPOCH_SECS: u64 = 35 * DAY_SECS;
pub const MIN_BUFFER_BPS: u16 = 500;
pub const MAX_BUFFER_BPS: u16 = 5_000;
pub const MIN_INSTANT_CAP_BPS: u16 = 100;
pub const MAX_INSTANT_CAP_BPS: u16 = 5_000;
pub const MAX_THIRD_PARTY_BPS: u16 = 100;
pub const MIN_ALLOCATION_FLOOR_ATOMS: u64 = 1_000_000;
/// Epoch bound (overflow guard of the epoch counter and its record seeds).
pub const MAX_FUND_EPOCH: u64 = (1 << 56) - 1;
/// The first accumulating redemption batch; zero never names a batch.
pub const FIRST_BATCH_ID: u64 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FundMathError {
    Overflow,
    InvalidAmount,
    InsufficientCash,
    InstantCapExceeded,
    InvalidPrice,
    NotReady,
    InvalidState,
    BudgetExceeded,
    QueueBusy,
    /// Lean pricing: an open writer receipt makes the share price unknown
    /// (also: the last share cannot exit instantly while a slot is open).
    Invested,
}

pub type FundResult<T> = Result<T, FundMathError>;

#[inline(never)]
pub(crate) fn mul_div_floor(value: u64, numerator: u64, denominator: u64) -> FundResult<u64> {
    if denominator == 0 {
        return Err(FundMathError::InvalidPrice);
    }
    u64::try_from(u128::from(value) * u128::from(numerator) / u128::from(denominator))
        .map_err(|_| FundMathError::Overflow)
}

fn mul_div_ceil(value: u64, numerator: u64, denominator: u64) -> FundResult<u64> {
    if denominator == 0 {
        return Err(FundMathError::InvalidPrice);
    }
    u64::try_from((u128::from(value) * u128::from(numerator)).div_ceil(u128::from(denominator)))
        .map_err(|_| FundMathError::Overflow)
}

fn add(a: u64, b: u64) -> FundResult<u64> {
    a.checked_add(b).ok_or(FundMathError::Overflow)
}

fn sub(a: u64, b: u64) -> FundResult<u64> {
    a.checked_sub(b).ok_or(FundMathError::InvalidState)
}

/// `assets / shares`. Zero shares is par: one atom per share.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SharePrice {
    pub assets: u64,
    pub shares: u64,
}

impl SharePrice {
    pub const PAR: Self = Self {
        assets: 0,
        shares: 0,
    };

    pub const fn new(assets: u64, shares: u64) -> Self {
        Self { assets, shares }
    }

    pub const fn is_par(self) -> bool {
        self.shares == 0
    }

    fn ratio(self) -> (u128, u128) {
        if self.is_par() {
            (1, 1)
        } else {
            (u128::from(self.assets), u128::from(self.shares))
        }
    }

    /// Atoms owed for `shares`, rounded down.
    pub fn atoms_for_shares(self, shares: u64) -> FundResult<u64> {
        if self.is_par() {
            return Ok(shares);
        }
        mul_div_floor(shares, self.assets, self.shares)
    }

    /// Shares minted for `atoms`, rounded down. A zero-asset fund with shares
    /// outstanding has no meaningful price and cannot admit new capital.
    pub fn shares_for_atoms(self, atoms: u64) -> FundResult<u64> {
        if self.is_par() {
            return Ok(atoms);
        }
        if self.assets == 0 {
            return Err(FundMathError::InvalidPrice);
        }
        mul_div_floor(atoms, self.shares, self.assets)
    }

    /// `self <= other` as exact rationals.
    pub fn at_most(self, other: Self) -> bool {
        let (a1, s1) = self.ratio();
        let (a2, s2) = other.ratio();
        a1 * s2 <= a2 * s1
    }
}

/// Admin-configured, bounded fund parameters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FundParams {
    pub buffer_bps: u16,
    pub instant_daily_cap_bps: u16,
    /// Largest third-party share of a sleeve's pooled principal that still
    /// admits a fund top-up into an exposed (selling) sleeve.
    pub max_third_party_bps: u16,
    pub min_tenor_secs: u64,
    pub max_tenor_secs: u64,
    pub min_epoch_secs: u64,
    pub min_allocation_atoms: u64,
    /// Keeper authority that must sign every Allocate.
    pub allocator: [u8; 32],
    pub paused: bool,
    /// Buy-back pricing of instant exits while invested, in its 32-byte form
    /// (`BuybackParams::decode`); validated by `is_valid`.
    pub buyback: [u8; crate::buyback_mark_math::BUYBACK_PARAMS_LEN],
}

impl FundParams {
    /// The decoded buy-back settings (`INVALID` when undecodable).
    pub fn buyback_params(&self) -> crate::buyback_mark_math::BuybackParams {
        crate::buyback_mark_math::BuybackParams::decode(&self.buyback)
            .unwrap_or(crate::buyback_mark_math::BuybackParams::INVALID)
    }

    pub fn is_valid(&self) -> bool {
        let buyback = self.buyback_params();
        (MIN_BUFFER_BPS..=MAX_BUFFER_BPS).contains(&self.buffer_bps)
            && (MIN_INSTANT_CAP_BPS..=MAX_INSTANT_CAP_BPS).contains(&self.instant_daily_cap_bps)
            && self.max_third_party_bps <= MAX_THIRD_PARTY_BPS
            && self.min_tenor_secs >= MIN_TENOR_FLOOR_SECS
            && self.min_tenor_secs <= self.max_tenor_secs
            // A lot younger than the expiry guard could never be priced.
            && self.min_tenor_secs > u64::from(buyback.expiry_guard_secs)
            && self.max_tenor_secs <= MAX_TENOR_SECS
            && (MIN_EPOCH_SECS..=MAX_EPOCH_SECS).contains(&self.min_epoch_secs)
            && self.min_allocation_atoms >= MIN_ALLOCATION_FLOOR_ATOMS
            && self.allocator.iter().any(|byte| *byte != 0)
            && buyback.is_valid(self.instant_daily_cap_bps)
    }

    /// Whether the fund may hold a sleeve of `series_count` series: while
    /// buy-back pricing is on, one mark must be able to cover every series
    /// (otherwise the slot never prices and every priced exit waits).
    #[inline(always)]
    pub fn sleeve_priceable(&self, series_count: u8) -> bool {
        // Byte 0 is the enabled flag of valid (decodable) settings.
        self.buyback[0] == 0
            || usize::from(series_count) <= crate::buyback_mark_math::MAX_MARK_SERIES
    }

    pub fn tenor_admissible(&self, now: u64, expiry: u64) -> bool {
        expiry
            .checked_sub(now)
            .is_some_and(|tenor| tenor >= self.min_tenor_secs && tenor <= self.max_tenor_secs)
    }

    /// `third_party * 10000 <= max_third_party_bps * pooled_principal`, where
    /// third-party principal is every pooled atom not owned by fund lots.
    pub fn sole_writer_admissible(&self, pooled_principal: u64, fund_principal: u64) -> bool {
        pooled_principal
            .checked_sub(fund_principal)
            .is_some_and(|third| {
                u128::from(third) * u128::from(BPS_DENOMINATOR)
                    <= u128::from(self.max_third_party_bps) * u128::from(pooled_principal)
            })
    }
}

/// A redemption batch while it is still being filled. Members own the fixed
/// fraction `q / shares` of every share and atom filled in the batch.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QueueBatch {
    pub id: u64,
    pub shares: u64,
    pub filled_shares: u64,
    pub filled_atoms: u64,
    /// Atoms already credited to members' claimable balances.
    pub paid_atoms: u64,
}

impl QueueBatch {
    pub fn is_valid(&self) -> bool {
        self.id != 0
            && self.shares != 0
            && self.filled_shares <= self.shares
            && self.paid_atoms <= self.filled_atoms
    }

    pub fn remaining(&self) -> u64 {
        self.shares.saturating_sub(self.filled_shares)
    }

    /// Atoms a member queued with `q` shares is entitled to so far.
    pub fn entitled(&self, q: u64) -> FundResult<u64> {
        if q > self.shares {
            return Err(FundMathError::InvalidState);
        }
        mul_div_floor(q, self.filled_atoms, self.shares)
    }

    /// A member's still-queued shares, rounded down (its filled part rounds up).
    pub fn remaining_of(&self, q: u64) -> FundResult<u64> {
        if q > self.shares {
            return Err(FundMathError::InvalidState);
        }
        sub(q, mul_div_ceil(q, self.filled_shares, self.shares)?)
    }
}

/// A fully filled batch, kept in the record of the roll that completed it
/// until every member has settled out.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CompletedBatch {
    pub id: u64,
    pub shares: u64,
    pub filled_atoms: u64,
    pub paid_atoms: u64,
    pub settled_shares: u64,
}

impl CompletedBatch {
    pub fn is_valid(&self) -> bool {
        self.id != 0
            && self.shares != 0
            && self.paid_atoms <= self.filled_atoms
            && self.settled_shares <= self.shares
    }
}

/// The fund-wide ledger.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FundLedger {
    pub total_shares: u64,
    pub epoch: u64,
    pub epoch_started_ts: u64,
    /// Price fixed by the most recent roll (par before the first roll).
    pub price: SharePrice,
    /// Fund NAV immediately after the most recent roll; the instant cap base.
    pub cap_base_atoms: u64,
    pub free_cash_atoms: u64,
    pub pending_deposit_atoms: u64,
    /// Oldest epoch whose pending deposits are still unconverted.
    pub pending_carry_from: u64,
    pub reserved_withdrawal_atoms: u64,
    /// Accumulating plus every open batch's remaining shares.
    pub queued_shares: u64,
    pub accumulating_batch_id: u64,
    pub accumulating_shares: u64,
    pub open_batch_count: u8,
    pub open_batches: [QueueBatch; MAX_OPEN_BATCHES],
    /// Principal of every open fund lot.
    pub deployed_atoms: u64,
    /// Token bucket of instant payouts: `level` drains linearly at the cap
    /// per 24h, so any interval of length T pays at most `cap * (1 + T/24h)`.
    pub window_updated_ts: u64,
    pub window_level_atoms: u64,
}

/// What one roll fixed. The processor records it in the closed epoch's account.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RollOutcome {
    pub closed_epoch: u64,
    pub price: SharePrice,
    pub pending_converted: bool,
    pub carry_from: u64,
    pub converted_atoms: u64,
    pub converted_shares: u64,
    pub queue_before_shares: u64,
    pub fill_shares: u64,
    pub fill_atoms: u64,
    pub completed_count: u8,
    pub completed: [CompletedBatch; MAX_COMPLETED_BATCHES],
    pub post_assets: u64,
    pub post_shares: u64,
}

/// An instant withdrawal admitted against the current cash and the bucket.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InstantQuote {
    pub shares: u64,
    pub payout: u64,
    pub price: SharePrice,
    now: u64,
    level_after: u64,
}

impl FundLedger {
    /// A fresh fund at epoch zero.
    pub fn new(now: u64) -> Self {
        Self {
            epoch_started_ts: now,
            accumulating_batch_id: FIRST_BATCH_ID,
            window_updated_ts: now,
            ..Self::default()
        }
    }

    /// Structural consistency of the queue and buckets.
    pub fn is_consistent(&self) -> bool {
        let count = usize::from(self.open_batch_count);
        if count > MAX_OPEN_BATCHES
            || self.accumulating_batch_id == 0
            || self.pending_carry_from > self.epoch
            || self.queued_shares > self.total_shares
        {
            return false;
        }
        let mut queued = u128::from(self.accumulating_shares);
        let mut previous = 0u64;
        for (index, batch) in self.open_batches.iter().enumerate() {
            if index < count {
                if !batch.is_valid()
                    || batch.remaining() == 0
                    || batch.id <= previous
                    || batch.id >= self.accumulating_batch_id
                {
                    return false;
                }
                previous = batch.id;
                queued += u128::from(batch.remaining());
            } else if *batch != QueueBatch::default() {
                return false;
            }
        }
        queued == u128::from(self.queued_shares)
    }

    pub fn open_batch_index(&self, id: u64) -> Option<usize> {
        self.open_batches[..usize::from(self.open_batch_count).min(MAX_OPEN_BATCHES)]
            .iter()
            .position(|batch| batch.id == id)
    }

    /// Total cash the vault must physically hold for every bucket.
    pub fn required_vault_atoms(&self) -> FundResult<u64> {
        add(
            add(self.free_cash_atoms, self.pending_deposit_atoms)?,
            self.reserved_withdrawal_atoms,
        )
    }

    /// The price per share: free cash plus a valuation of open receipts
    /// (zero when none is open).
    pub fn nav_price(&self, open_value: u64) -> FundResult<SharePrice> {
        let assets = add(self.free_cash_atoms, open_value)?;
        Ok(if self.total_shares == 0 {
            SharePrice::PAR
        } else {
            SharePrice::new(assets, self.total_shares)
        })
    }

    /// The entry price at the open slots' upper value. Never par while
    /// anything is invested: with no shares outstanding the open lots' value
    /// would go to the entrant.
    pub fn entry_price(&self, open_upper: u64) -> FundResult<SharePrice> {
        if self.total_shares == 0 && self.deployed_atoms != 0 {
            return Err(FundMathError::Invested);
        }
        self.nav_price(open_upper)
    }

    /// Shares `atoms` of pending money buy now at the entry price (rounded
    /// down); a conversion into zero shares is refused.
    pub fn entry_shares(&self, atoms: u64, open_upper: u64) -> FundResult<u64> {
        let shares = self.entry_price(open_upper)?.shares_for_atoms(atoms)?;
        if shares == 0 {
            return Err(FundMathError::InvalidAmount);
        }
        Ok(shares)
    }

    /// Move `atoms` from pending to free cash and mint `shares` for them.
    /// Nothing changes on error.
    pub fn apply_conversion(&mut self, atoms: u64, shares: u64) -> FundResult<()> {
        let pending = sub(self.pending_deposit_atoms, atoms)?;
        let free = add(self.free_cash_atoms, atoms)?;
        let total = add(self.total_shares, shares)?;
        self.pending_deposit_atoms = pending;
        self.free_cash_atoms = free;
        self.total_shares = total;
        Ok(())
    }

    /// Convert `atoms` of pending deposits to shares now at the entry price;
    /// returns the shares minted. Nothing changes on error.
    pub fn convert_pending(&mut self, atoms: u64, open_upper: u64) -> FundResult<u64> {
        let shares = self.entry_shares(atoms, open_upper)?;
        self.apply_conversion(atoms, shares)?;
        Ok(shares)
    }

    pub fn deposit(&mut self, amount: u64) -> FundResult<()> {
        if amount == 0 {
            return Err(FundMathError::InvalidAmount);
        }
        self.pending_deposit_atoms = add(self.pending_deposit_atoms, amount)?;
        Ok(())
    }

    /// Return not-yet-converted pending capital at par.
    pub fn refund_pending(&mut self, amount: u64) -> FundResult<()> {
        if amount == 0 {
            return Err(FundMathError::InvalidAmount);
        }
        self.pending_deposit_atoms = sub(self.pending_deposit_atoms, amount)?;
        Ok(())
    }

    /// Add `shares` to the accumulating batch; returns its id.
    pub fn enqueue(&mut self, shares: u64) -> FundResult<u64> {
        let queued = add(self.queued_shares, shares)?;
        if shares == 0 || queued > self.total_shares {
            return Err(FundMathError::InvalidAmount);
        }
        self.accumulating_shares = add(self.accumulating_shares, shares)?;
        self.queued_shares = queued;
        Ok(self.accumulating_batch_id)
    }

    pub fn instant_cap_atoms(&self, cap_bps: u16) -> FundResult<u64> {
        mul_div_floor(self.cap_base_atoms, u64::from(cap_bps), BPS_DENOMINATOR)
    }

    /// The bucket level at `now`: drained linearly by `cap` per 24h.
    pub fn window_level_at(&self, cap: u64, now: u64) -> FundResult<u64> {
        let elapsed = now
            .saturating_sub(self.window_updated_ts)
            .min(INSTANT_WINDOW_SECS);
        Ok(self.window_level_atoms.saturating_sub(mul_div_floor(
            cap,
            elapsed,
            INSTANT_WINDOW_SECS,
        )?))
    }

    /// Lean instant redemption: only while nothing is invested, at the exact
    /// NAV `free_cash / shares`, within free cash and the 24h bucket.
    pub fn quote_instant_lean(
        &self,
        shares: u64,
        cap_bps: u16,
        now: u64,
    ) -> FundResult<InstantQuote> {
        if self.deployed_atoms != 0 {
            return Err(FundMathError::Invested);
        }
        self.quote_instant(shares, self.nav_price(0)?, cap_bps, now)
    }

    /// Price, payout and bucket admission for an instant redemption at
    /// `mark`, paid only from free cash.
    pub fn quote_instant(
        &self,
        shares: u64,
        mark: SharePrice,
        cap_bps: u16,
        now: u64,
    ) -> FundResult<InstantQuote> {
        let unqueued = sub(self.total_shares, self.queued_shares)?;
        if shares == 0 || shares > unqueued {
            return Err(FundMathError::InvalidAmount);
        }
        // The last share never exits instantly while anything is invested:
        // with no shares left the open lots' value above the price paid would
        // be orphaned (the next depositor buys in at par and collects it).
        if self.deployed_atoms != 0 && shares == self.total_shares {
            return Err(FundMathError::Invested);
        }
        let payout = mark.atoms_for_shares(shares)?;
        if payout == 0 {
            return Err(FundMathError::InvalidAmount);
        }
        let cap = self.instant_cap_atoms(cap_bps)?;
        let level_after = add(self.window_level_at(cap, now)?, payout)?;
        if level_after > cap {
            return Err(FundMathError::InstantCapExceeded);
        }
        if payout > self.free_cash_atoms {
            return Err(FundMathError::InsufficientCash);
        }
        Ok(InstantQuote {
            shares,
            payout,
            price: mark,
            now,
            level_after,
        })
    }

    pub fn apply_instant(&mut self, quote: InstantQuote) -> FundResult<()> {
        let mut next = *self;
        next.total_shares = sub(next.total_shares, quote.shares)?;
        next.free_cash_atoms = sub(next.free_cash_atoms, quote.payout)?;
        next.window_updated_ts = quote.now;
        next.window_level_atoms = quote.level_after;
        *self = next;
        Ok(())
    }

    /// Lean roll: every receipt collected, so `P = free cash / S` exactly.
    pub fn roll_lean(&mut self, params: &FundParams, now: u64) -> FundResult<RollOutcome> {
        if self.deployed_atoms != 0 {
            return Err(FundMathError::NotReady);
        }
        self.roll(params, now, 0, 0)
    }

    /// Close the epoch with the open slots valued at `[lower, upper]` (both
    /// zero with nothing open: the exact lean roll), both prices taken before
    /// any conversion.
    ///
    /// Pending deposits convert at the entry price `(free cash + upper) / S`
    /// unless the conversion cannot be represented (zero assets, u64
    /// overflow, zero shares — a few shares owning a large value would
    /// otherwise absorb the deposit — or no shares while invested), in which
    /// case they stay pending at par and remain refundable; the roll itself
    /// never fails for that reason. Queued batches are then filled at the
    /// exit price `(free cash + lower) / S` from the cash after conversion,
    /// pro rata when the cash is insufficient (always covered with nothing
    /// open), never burning the last share while invested. The record keeps
    /// the entry price (lazy conversion); the fund keeps the exit price.
    pub fn roll(
        &mut self,
        params: &FundParams,
        now: u64,
        lower: u64,
        upper: u64,
    ) -> FundResult<RollOutcome> {
        if now < self.epoch_started_ts.saturating_add(params.min_epoch_secs)
            || self.epoch >= MAX_FUND_EPOCH
        {
            return Err(FundMathError::NotReady);
        }
        if !self.is_consistent() {
            return Err(FundMathError::InvalidState);
        }
        let mut next = *self;
        let price = self.nav_price(lower)?;
        let entry = self.entry_price(upper);
        let mut outcome = RollOutcome {
            closed_epoch: self.epoch,
            price: entry.unwrap_or(price),
            carry_from: self.pending_carry_from,
            pending_converted: true,
            ..RollOutcome::default()
        };

        // Pending deposits convert at the entry price, or are carried at par.
        let pending = next.pending_deposit_atoms;
        if pending != 0 {
            let converted = entry
                .and_then(|entry| entry.shares_for_atoms(pending))
                .ok()
                .filter(|shares| *shares != 0)
                .and_then(|shares| {
                    next.total_shares
                        .checked_add(shares)
                        .map(|total| (shares, total))
                });
            match converted {
                Some((shares, total)) => {
                    next.total_shares = total;
                    next.free_cash_atoms = add(next.free_cash_atoms, pending)?;
                    next.pending_deposit_atoms = 0;
                    outcome.converted_atoms = pending;
                    outcome.converted_shares = shares;
                }
                None => outcome.pending_converted = false,
            }
        }
        if outcome.pending_converted {
            next.pending_carry_from = self.epoch + 1;
        }

        // Queued batches redeem at the exit price from the cash available
        // after conversion.
        let cash = next.free_cash_atoms;
        let count = usize::from(next.open_batch_count);
        let mut batches = [QueueBatch::default(); MAX_COMPLETED_BATCHES];
        batches[..count].copy_from_slice(&next.open_batches[..count]);
        let ring_remaining = batches[..count]
            .iter()
            .try_fold(0u64, |sum, batch| add(sum, batch.remaining()))?;
        let accumulating = next.accumulating_shares;
        let queued_all = add(ring_remaining, accumulating)?;
        let all_cost = price.atoms_for_shares(queued_all)?;
        let mut n = count;
        // A full ring takes the newest batch only when every batch completes:
        // while invested the last share is never burned, so a fill of every
        // share would leave all of them open.
        let completes =
            cash >= all_cost && (self.deployed_atoms == 0 || queued_all != next.total_shares);
        if accumulating != 0 && (completes || count < MAX_OPEN_BATCHES) {
            batches[n] = QueueBatch {
                id: next.accumulating_batch_id,
                shares: accumulating,
                ..QueueBatch::default()
            };
            n += 1;
            next.accumulating_shares = 0;
            next.accumulating_batch_id = add(next.accumulating_batch_id, 1)?;
        }
        let total_remaining = batches[..n]
            .iter()
            .try_fold(0u64, |sum, batch| add(sum, batch.remaining()))?;
        let mut burned = 0u64;
        let mut paid = 0u64;
        if total_remaining != 0 {
            let cost = price.atoms_for_shares(total_remaining)?;
            // While invested no fill burns the last share (see `quote_instant`).
            let keep = u64::from(self.deployed_atoms != 0 && total_remaining == next.total_shares);
            let fill = if cash >= cost && keep == 0 {
                total_remaining
            } else {
                // cash < cost implies cash * S / A < total_remaining.
                price
                    .shares_for_atoms(cash)
                    .unwrap_or(0)
                    .min(total_remaining - keep)
            };
            for batch in &mut batches[..n] {
                let remaining = batch.remaining();
                let shares = if fill == total_remaining {
                    remaining
                } else {
                    mul_div_floor(remaining, fill, total_remaining)?
                };
                let atoms = price.atoms_for_shares(shares)?;
                batch.filled_shares = add(batch.filled_shares, shares)?;
                batch.filled_atoms = add(batch.filled_atoms, atoms)?;
                burned = add(burned, shares)?;
                paid = add(paid, atoms)?;
            }
        }
        if paid > cash {
            return Err(FundMathError::InvalidState);
        }
        next.total_shares = sub(next.total_shares, burned)?;
        next.queued_shares = sub(next.queued_shares, burned)?;
        next.free_cash_atoms = sub(next.free_cash_atoms, paid)?;
        next.reserved_withdrawal_atoms = add(next.reserved_withdrawal_atoms, paid)?;
        let mut open = 0usize;
        next.open_batches = [QueueBatch::default(); MAX_OPEN_BATCHES];
        for batch in &batches[..n] {
            if batch.remaining() == 0 {
                let slot = usize::from(outcome.completed_count);
                outcome.completed[slot] = CompletedBatch {
                    id: batch.id,
                    shares: batch.shares,
                    filled_atoms: batch.filled_atoms,
                    paid_atoms: batch.paid_atoms,
                    settled_shares: 0,
                };
                outcome.completed_count += 1;
            } else {
                if open == MAX_OPEN_BATCHES {
                    return Err(FundMathError::InvalidState);
                }
                next.open_batches[open] = *batch;
                open += 1;
            }
        }
        next.open_batch_count = open as u8;

        let post_assets = add(next.free_cash_atoms, lower)?;
        next.epoch = self.epoch + 1;
        next.epoch_started_ts = now;
        next.price = price;
        next.cap_base_atoms = post_assets;
        outcome.queue_before_shares = total_remaining;
        outcome.fill_shares = burned;
        outcome.fill_atoms = paid;
        outcome.post_assets = post_assets;
        outcome.post_shares = next.total_shares;
        if !next.is_consistent() {
            return Err(FundMathError::InvalidState);
        }
        *self = next;
        Ok(outcome)
    }

    /// Deploy free cash into a writer sleeve, keeping the minimum cash buffer
    /// live: afterwards free cash is still at least `buffer_bps` of free cash
    /// plus deployed principal (principal stands in for NAV, so no mark is
    /// needed; in a losing month this keeps more cash, which is conservative).
    pub fn allocate(&mut self, params: &FundParams, amount: u64) -> FundResult<()> {
        if amount < params.min_allocation_atoms || amount == 0 {
            return Err(FundMathError::InvalidAmount);
        }
        let free = self
            .free_cash_atoms
            .checked_sub(amount)
            .ok_or(FundMathError::BudgetExceeded)?;
        let deployed = add(self.deployed_atoms, amount)?;
        if u128::from(free) * u128::from(BPS_DENOMINATOR)
            < u128::from(params.buffer_bps) * (u128::from(free) + u128::from(deployed))
        {
            return Err(FundMathError::BudgetExceeded);
        }
        self.free_cash_atoms = free;
        self.deployed_atoms = deployed;
        Ok(())
    }

    /// A settled receipt returned `payout` for `principal`.
    pub fn collect(&mut self, principal: u64, payout: u64) -> FundResult<()> {
        let deployed = sub(self.deployed_atoms, principal)?;
        self.free_cash_atoms = add(self.free_cash_atoms, payout)?;
        self.deployed_atoms = deployed;
        Ok(())
    }
}

/// One owner's position. `deposited_atoms` and `withdrawn_atoms` are the
/// owner's cumulative cost basis: atoms credited to pending, and atoms
/// actually paid to the owner.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PositionLedger {
    pub shares: u64,
    pub pending_atoms: u64,
    pub pending_epoch: u64,
    /// Shares this position queued into batch `queue_batch_id`.
    pub queue_shares: u64,
    pub queue_batch_id: u64,
    /// Atoms of that batch already credited to `claimable_atoms`.
    pub queue_paid_atoms: u64,
    pub claimable_atoms: u64,
    pub deposited_atoms: u64,
    pub withdrawn_atoms: u64,
}

/// Where a position's queued shares currently are.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueState {
    Empty,
    Accumulating,
    Open(usize),
    Completed,
}

/// The mutable per-roll record used by lazy settlement.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EpochLedger {
    pub epoch: u64,
    pub price: SharePrice,
    pub pending_converted: bool,
    pub carry_from: u64,
    pub converted_remaining_atoms: u64,
    pub converted_remaining_shares: u64,
    pub completed_count: u8,
    pub completed: [CompletedBatch; MAX_COMPLETED_BATCHES],
}

impl EpochLedger {
    pub fn from_roll(outcome: &RollOutcome) -> Self {
        Self {
            epoch: outcome.closed_epoch,
            price: outcome.price,
            pending_converted: outcome.pending_converted,
            carry_from: outcome.carry_from,
            converted_remaining_atoms: outcome.converted_atoms,
            converted_remaining_shares: outcome.converted_shares,
            completed_count: outcome.completed_count,
            completed: outcome.completed,
        }
    }

    /// Does this record convert pending made in `epoch`?
    pub fn converts(&self, epoch: u64) -> bool {
        self.pending_converted && self.carry_from <= epoch && epoch <= self.epoch
    }

    pub fn completed_index(&self, id: u64) -> Option<usize> {
        self.completed[..usize::from(self.completed_count).min(MAX_COMPLETED_BATCHES)]
            .iter()
            .position(|batch| batch.id == id)
    }
}

impl PositionLedger {
    /// Pending deposits converted by a roll this position has not applied.
    pub fn pending_owed(&self, fund: &FundLedger) -> bool {
        self.pending_atoms != 0 && self.pending_epoch < fund.pending_carry_from
    }

    pub fn queue_state(&self, fund: &FundLedger) -> FundResult<QueueState> {
        if self.queue_shares == 0 {
            return Ok(QueueState::Empty);
        }
        if self.queue_batch_id == fund.accumulating_batch_id {
            return Ok(QueueState::Accumulating);
        }
        if let Some(index) = fund.open_batch_index(self.queue_batch_id) {
            return Ok(QueueState::Open(index));
        }
        if self.queue_batch_id != 0 && self.queue_batch_id < fund.accumulating_batch_id {
            return Ok(QueueState::Completed);
        }
        Err(FundMathError::InvalidState)
    }

    /// Convert this position's pending deposit with the roll that converted it.
    pub fn settle_pending(&mut self, record: &mut EpochLedger) -> FundResult<()> {
        if self.pending_atoms == 0 || !record.converts(self.pending_epoch) {
            return Err(FundMathError::InvalidState);
        }
        let shares = record.price.shares_for_atoms(self.pending_atoms)?;
        let mut next = *self;
        let mut epoch = *record;
        epoch.converted_remaining_atoms = sub(epoch.converted_remaining_atoms, next.pending_atoms)?;
        epoch.converted_remaining_shares = sub(epoch.converted_remaining_shares, shares)?;
        next.shares = add(next.shares, shares)?;
        next.pending_atoms = 0;
        next.pending_epoch = 0;
        *self = next;
        *record = epoch;
        Ok(())
    }

    /// Credit what an open batch has filled for this position so far.
    pub fn settle_open_batch(&mut self, fund: &mut FundLedger, index: usize) -> FundResult<()> {
        let batch = fund
            .open_batches
            .get_mut(index)
            .ok_or(FundMathError::InvalidState)?;
        if batch.id != self.queue_batch_id {
            return Err(FundMathError::InvalidState);
        }
        let entitled = batch.entitled(self.queue_shares)?;
        let owed = sub(entitled, self.queue_paid_atoms)?;
        let paid = add(batch.paid_atoms, owed)?;
        if paid > batch.filled_atoms {
            return Err(FundMathError::InvalidState);
        }
        let claimable = add(self.claimable_atoms, owed)?;
        batch.paid_atoms = paid;
        self.claimable_atoms = claimable;
        self.queue_paid_atoms = entitled;
        Ok(())
    }

    /// Final credit from a batch completed at `record`'s roll; the position
    /// leaves the batch. Returns the batch dust moved back to free cash once
    /// every member has settled out.
    pub fn settle_completed_batch(
        &mut self,
        record: &mut EpochLedger,
        fund: &mut FundLedger,
    ) -> FundResult<u64> {
        let index = record
            .completed_index(self.queue_batch_id)
            .ok_or(FundMathError::InvalidState)?;
        let mut next = *self;
        let mut epoch = *record;
        let mut ledger = *fund;
        let batch = &mut epoch.completed[index];
        if next.queue_shares > batch.shares {
            return Err(FundMathError::InvalidState);
        }
        let entitled = mul_div_floor(next.queue_shares, batch.filled_atoms, batch.shares)?;
        let owed = sub(entitled, next.queue_paid_atoms)?;
        batch.paid_atoms = add(batch.paid_atoms, owed)?;
        batch.settled_shares = add(batch.settled_shares, next.queue_shares)?;
        if batch.paid_atoms > batch.filled_atoms || batch.settled_shares > batch.shares {
            return Err(FundMathError::InvalidState);
        }
        let mut dust = 0;
        if batch.settled_shares == batch.shares {
            dust = batch.filled_atoms - batch.paid_atoms;
            batch.paid_atoms = batch.filled_atoms;
            ledger.reserved_withdrawal_atoms = sub(ledger.reserved_withdrawal_atoms, dust)?;
            ledger.free_cash_atoms = add(ledger.free_cash_atoms, dust)?;
        }
        next.claimable_atoms = add(next.claimable_atoms, owed)?;
        next.queue_shares = 0;
        next.queue_batch_id = 0;
        next.queue_paid_atoms = 0;
        *self = next;
        *record = epoch;
        *fund = ledger;
        Ok(dust)
    }

    /// Shares this position still has queued (zero once its batch completed).
    pub fn queued_remaining(&self, fund: &FundLedger) -> FundResult<u64> {
        match self.queue_state(fund)? {
            QueueState::Empty | QueueState::Completed => Ok(0),
            QueueState::Accumulating => Ok(self.queue_shares),
            QueueState::Open(index) => fund.open_batches[index].remaining_of(self.queue_shares),
        }
    }

    pub fn add_pending(&mut self, amount: u64, current_epoch: u64) -> FundResult<()> {
        let pending = add(self.pending_atoms, amount)?;
        self.deposited_atoms = add(self.deposited_atoms, amount)?;
        if self.pending_atoms == 0 {
            self.pending_epoch = current_epoch;
        }
        self.pending_atoms = pending;
        Ok(())
    }

    /// Record atoms paid to the owner.
    pub fn record_paid(&mut self, atoms: u64) -> FundResult<()> {
        self.withdrawn_atoms = add(self.withdrawn_atoms, atoms)?;
        Ok(())
    }

    /// Value at `price` of a settled position: pending at par, shares and
    /// still-queued shares at `price` (floored), plus claimable atoms.
    pub fn value_at(&self, price: SharePrice, fund: &FundLedger) -> FundResult<u64> {
        add(
            add(self.pending_atoms, price.atoms_for_shares(self.shares)?)?,
            add(
                price.atoms_for_shares(self.queued_remaining(fund)?)?,
                self.claimable_atoms,
            )?,
        )
    }

    /// `value - (deposited - withdrawn)`: realized plus unrealized earnings.
    pub fn earnings_at(&self, price: SharePrice, fund: &FundLedger) -> FundResult<i128> {
        Ok(
            i128::from(self.value_at(price, fund)?) - i128::from(self.deposited_atoms)
                + i128::from(self.withdrawn_atoms),
        )
    }

    /// Unconverted pending (always the case after settlement) returns at par.
    pub fn take_pending(&mut self, amount: u64) -> FundResult<()> {
        if amount == 0 || amount > self.pending_atoms {
            return Err(FundMathError::InvalidAmount);
        }
        self.pending_atoms -= amount;
        if self.pending_atoms == 0 {
            self.pending_epoch = 0;
        }
        Ok(())
    }

    pub fn take_shares(&mut self, shares: u64) -> FundResult<()> {
        if shares == 0 || shares > self.shares {
            return Err(FundMathError::InvalidAmount);
        }
        self.shares -= shares;
        Ok(())
    }

    /// Queue `shares` into the fund's accumulating batch. A position may sit
    /// in only one batch: joining is refused while an older batch it belongs
    /// to is still being filled.
    pub fn queue(&mut self, shares: u64, fund: &mut FundLedger) -> FundResult<()> {
        match self.queue_state(fund)? {
            QueueState::Empty | QueueState::Accumulating => {}
            QueueState::Open(_) | QueueState::Completed => return Err(FundMathError::QueueBusy),
        }
        let mut next = *self;
        let mut ledger = *fund;
        next.take_shares(shares)?;
        let id = ledger.enqueue(shares)?;
        next.queue_shares = add(next.queue_shares, shares)?;
        next.queue_batch_id = id;
        *self = next;
        *fund = ledger;
        Ok(())
    }

    /// Release claimable atoms from the reserved bucket for payment.
    pub fn complete(&mut self, fund: &mut FundLedger) -> FundResult<u64> {
        let atoms = self.claimable_atoms;
        if atoms == 0 {
            return Err(FundMathError::InvalidAmount);
        }
        let withdrawn = add(self.withdrawn_atoms, atoms)?;
        fund.reserved_withdrawal_atoms = sub(fund.reserved_withdrawal_atoms, atoms)?;
        self.withdrawn_atoms = withdrawn;
        self.claimable_atoms = 0;
        Ok(atoms)
    }
}

/// A slot (one sleeve the fund holds) is live and unpriced.
pub const SLOT_UNPRICED: u64 = 0;
/// A live slot with a usable valuation.
pub const SLOT_PRICED: u64 = 1;
/// A settled or refunded slot: its value is exact and never ages.
pub const SLOT_FINAL: u64 = 2;

/// One open slot's recorded valuation: five little-endian words of
/// `EarnFundSlotV1`. Always `lower <= upper`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SlotMark {
    /// `SLOT_UNPRICED`, `SLOT_PRICED` or `SLOT_FINAL`.
    pub state: u64,
    /// The pricing round this valuation counts in (0: none).
    pub round: u64,
    /// Exit-side value (the ask-side buy-back mark, gains credited at `g`).
    pub lower: u64,
    /// Entry-side value (the bid-side mark, gains fully credited).
    pub upper: u64,
    /// The time the value reflects: the mark head's timestamp.
    pub valued_ts: u64,
}

/// The O(1) aggregate of every open slot, kept by the per-slot valuation
/// crank, Allocate and Collect, so Withdraw reads only the fund.
///
/// Freshness without reading the slots: valuations count in pricing rounds.
/// A live slot counts in round `round` once valued at a head no older than
/// `round_start_ts`; when every live slot counts, the round completes and
/// `fresh_since_ts` becomes the oldest head counted in it. Every live priced
/// slot's value is at least as recent as `fresh_since_ts` (a valuation older
/// than it is refused as unpriced), so `now - fresh_since_ts <= H` bounds
/// every value's age. A keeper revaluing every live slot each `C` seconds
/// keeps the bound within about `2 C`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SlotBook {
    /// Open slots.
    pub slots: u64,
    /// Open slots that are not final.
    pub live: u64,
    /// Live slots without a usable valuation.
    pub unpriced: u64,
    /// Live priced slots counted in the current round.
    pub counted: u64,
    /// Σ `SlotMark::lower` over every open slot.
    pub sum_lower: u64,
    /// Σ `SlotMark::upper` over every open slot (`>= sum_lower`).
    pub sum_upper: u64,
    pub round: u64,
    pub round_start_ts: u64,
    /// The oldest head counted in the current round.
    pub round_min_ts: u64,
    pub fresh_since_ts: u64,
    /// Contribution nonce of the next fund lot (its receipt's seed).
    pub next_lot_id: u64,
    /// Principal of every open fund lot (`FundLedger::deployed_atoms`).
    pub deployed_atoms: u64,
}

impl SlotBook {
    pub fn is_consistent(&self) -> bool {
        self.live <= self.slots
            && self.unpriced.saturating_add(self.counted) <= self.live
            && self.sum_lower <= self.sum_upper
    }

    /// Add (`add`) or remove one slot's share of the counters and sums.
    fn tally(&mut self, mark: &SlotMark, add: bool) -> FundResult<()> {
        let step = |count: u64| if add { count + 1 } else { count - 1 };
        let sum = |total: u64, value: u64| {
            if add {
                total.checked_add(value)
            } else {
                total.checked_sub(value)
            }
            .ok_or(FundMathError::InvalidState)
        };
        self.sum_lower = sum(self.sum_lower, mark.lower)?;
        self.sum_upper = sum(self.sum_upper, mark.upper)?;
        if mark.state != SLOT_FINAL {
            self.live = step(self.live);
            if mark.state == SLOT_UNPRICED {
                self.unpriced = step(self.unpriced);
            } else if mark.round == self.round {
                self.counted = step(self.counted);
            }
        }
        Ok(())
    }

    fn start_round(&mut self, now: u64) {
        self.round = self.round.saturating_add(1);
        self.round_start_ts = now;
        self.round_min_ts = u64::MAX;
        self.counted = 0;
    }

    fn advance(&mut self, now: u64) {
        if self.live != 0 && self.counted == self.live {
            self.fresh_since_ts = self.round_min_ts;
            self.start_round(now);
        }
    }

    /// A slot in a sleeve the fund did not hold: live and unpriced. The
    /// first live slot starts a new round.
    pub fn open(&mut self, now: u64) -> SlotMark {
        if self.live == 0 {
            self.start_round(now);
        }
        self.slots += 1;
        self.live += 1;
        self.unpriced += 1;
        SlotMark::default()
    }

    /// The slot's value can no longer be used (its sleeve or book changed, it
    /// cannot be priced now, or it was topped up). The value stays in the
    /// sums; the aggregate is unusable until the slot is valued again. A
    /// final (settled) slot is unaffected.
    pub fn invalidate(&mut self, mark: &mut SlotMark) {
        if mark.state == SLOT_PRICED {
            mark.state = SLOT_UNPRICED;
            self.unpriced += 1;
            if mark.round == self.round {
                self.counted -= 1;
            }
        }
        mark.round = 0;
    }

    /// Record a valuation `[lower, upper]` (an upper below the lower is
    /// raised to it). `exact` (settled or refunded) is final. A live value
    /// older than `fresh_since_ts` is refused: the slot is invalidated and
    /// `false` returned. A live value counts in the current round iff it is
    /// no older than the round's start.
    pub fn record(
        &mut self,
        mark: &mut SlotMark,
        lower: u64,
        upper: u64,
        valued_ts: u64,
        exact: bool,
        now: u64,
    ) -> FundResult<bool> {
        if mark.state == SLOT_FINAL && !exact {
            return Err(FundMathError::InvalidState);
        }
        if !exact && valued_ts < self.fresh_since_ts {
            self.invalidate(mark);
            return Ok(false);
        }
        self.tally(mark, false)?;
        let counts = !exact && valued_ts >= self.round_start_ts;
        *mark = SlotMark {
            state: if exact { SLOT_FINAL } else { SLOT_PRICED },
            round: if counts { self.round } else { 0 },
            lower,
            upper: upper.max(lower),
            valued_ts,
        };
        if counts {
            self.round_min_ts = self.round_min_ts.min(valued_ts);
        }
        self.tally(mark, true)?;
        self.advance(now);
        Ok(true)
    }

    /// The slot's last lot was collected.
    pub fn close(&mut self, mark: &SlotMark, now: u64) -> FundResult<()> {
        self.tally(mark, false)?;
        self.slots = sub(self.slots, 1)?;
        self.advance(now);
        Ok(())
    }

    /// `(Σ lower, Σ upper)` of every open slot, if usable: no live slot
    /// unpriced and every live value at most `max_age` seconds old.
    pub fn usable(&self, now: u64, max_age: u64) -> Option<(u64, u64)> {
        (self.unpriced == 0
            && (self.live == 0 || now.saturating_sub(self.fresh_since_ts) <= max_age))
            .then_some((self.sum_lower, self.sum_upper))
    }
}

/// A fund lot pinned by an open slot.
pub fn lot(
    principal: u64,
    entry_ts: u64,
    expiry_ts: u64,
    weight_offset: u128,
) -> ContributionInterval {
    ContributionInterval {
        principal,
        entry_ts,
        expiry_ts,
        weight_offset,
    }
}
