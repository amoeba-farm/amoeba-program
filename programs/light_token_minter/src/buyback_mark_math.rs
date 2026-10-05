//! Earn Fund buy-back mark: pure arithmetic with no Solana types.
//!
//! The fund is short the options its sleeves sold. Their buy-back cost is
//! priced from the fund's own executable ask ladder (the cost of an
//! identical long), aggregated over time so that it rises fast and falls
//! slowly:
//!
//! * one sample `A_i(t)`: the bin of the `D_i`-th cheapest option atom on the
//!   fund lane's ask ladder, `D_i = max(depth_min contracts, depth_bps × OI_i)`;
//!   a short or unsellable ladder, or a lane the swap path would not execute
//!   (`observe_writer_lane`), is an invalid sample priced at max payout;
//! * every 45-minute bucket keeps the maximum of its samples (extra samples
//!   can never lower it);
//! * `B_i = min(max_payout, tick × max(Q75, R))` where `Q75` is the
//!   nearest-rank upper quartile of the sampled bucket maxima of the last 24h
//!   and `R` the maximum of the current and previous buckets;
//! * the slot liability is `L = ceil(Σ OI_i × B_i / 10^6)`.
//!
//! The same samples record the mirror-image bid side (two-sided pricing):
//!
//! * `b_i(t)`: the bin where the fund lane's bids, walked from the highest,
//!   could buy back `D_i` contracts — counted only if the policy's remaining
//!   buy-back budget covers `D_i` at the highest bid and that bid is within
//!   the buy-back path's per-contract bounds (otherwise `NO_BID`, a
//!   worthless liability: the cautious side for entrants);
//! * every bucket keeps the minimum of its samples (extra samples can never
//!   raise it);
//! * `b_i = tick × min(Q25, R_min)` with `Q25` the nearest-rank lower quartile
//!   of the bucket minima and `R_min` the minimum of the current and previous
//!   buckets;
//! * the upper liability is `L_hi = min(L, floor(Σ OI_i × b_i / 10^6))`.
//!
//! A slot is priced only while every series with open interest is viable
//! (tradable now, executable depth, ≥ `min_valid_buckets` clean buckets and
//! balanced two-sided flow in the last 6h). Its lower (exit) value is the
//! claim payout at residual `assets − L`, with unrealized gains credited at
//! `gain_credit_bps` (zero at launch, so the value never exceeds principal);
//! its upper (entry) value is the claim payout at `assets − L_hi` with gains
//! fully credited.
use crate::writer_participation_math::{
    range_payout, ContributionInterval, ParticipationError, ParticipationTotals,
};

pub const BUCKET_SECS: u64 = 2_700;
pub const RING_BUCKETS: usize = 32;
/// Buckets in the two-sided flow window (6h).
pub const FLOW_BUCKETS: u64 = 8;
/// Sleeves with more series keep every exit lean (13 + 6 n crank accounts
/// plus the program, gate and compute-budget keys stay within 64).
/// An eighteen-series sample uses at most 124 transaction accounts, including
/// the governance envelope and compute budget, on a 128-lock runtime.
pub const MAX_MARK_SERIES: usize = 18;
/// "No executable ask": priced at the series' max payout.
pub const NO_ASK: u16 = u16::MAX;
/// "No executable bid": the liability is worth nothing to entrants (bin ids
/// start at 1, so 0 is never a real bin).
pub const NO_BID: u16 = 0;
pub const CONTRACT_ATOMS: u64 = 1_000_000;
pub const USD_ATOMS: u64 = 1_000_000;
pub const BUCKET_SAMPLED: u8 = 1;
pub const BUCKET_ALL_VALID: u8 = 2;
const BPS: u64 = 10_000;

/// Admin-set buy-back pricing settings (`EarnFundV1`, 32 bytes on the wire
/// with 9 trailing zero bytes).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuybackParams {
    /// Kill switch: false keeps every exit of invested money queued (lean).
    pub enabled: bool,
    /// Share of an unrealized marked gain paid early (0 at launch).
    pub gain_credit_bps: u16,
    pub min_valid_buckets: u8,
    pub max_head_age_secs: u16,
    pub depth_min_contracts: u16,
    pub depth_bps: u16,
    /// Two-sided flow floor per side and series over the last 6h: the larger
    /// of `flow_min_usd` and `flow_bps` of the sleeve's pooled principal —
    /// never of the marked liability, which the manager's quotes move.
    pub flow_min_usd: u16,
    pub flow_bps: u16,
    pub flow_imbalance_x: u8,
    pub expiry_guard_secs: u32,
    /// Marked exits per 24h, in bps of the post-roll assets.
    pub buyback_cap_bps: u16,
    /// Immediate conversions of deposits while invested per 24h, in bps of
    /// the post-roll assets (the entry bucket).
    pub entry_cap_bps: u16,
}

pub const BUYBACK_PARAMS_LEN: usize = 32;

impl Default for BuybackParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl BuybackParams {
    pub const DEFAULT: Self = Self {
        enabled: true,
        gain_credit_bps: 0,
        min_valid_buckets: 16,
        max_head_age_secs: 600,
        depth_min_contracts: 5,
        depth_bps: 200,
        flow_min_usd: 25,
        flow_bps: 25,
        flow_imbalance_x: 4,
        expiry_guard_secs: 21_600,
        buyback_cap_bps: 500,
        entry_cap_bps: 1_000,
    };

    /// All zero: fails `is_valid` (stands in for undecodable stored bytes).
    pub const INVALID: Self = Self {
        enabled: false,
        gain_credit_bps: 0,
        min_valid_buckets: 0,
        max_head_age_secs: 0,
        depth_min_contracts: 0,
        depth_bps: 0,
        flow_min_usd: 0,
        flow_bps: 0,
        flow_imbalance_x: 0,
        expiry_guard_secs: 0,
        buyback_cap_bps: 0,
        entry_cap_bps: 0,
    };

    /// The 32-byte wire and account form: `enabled u8, gain_credit_bps u16,
    /// min_valid_buckets u8, max_head_age_secs u16, depth_min_contracts u16,
    /// depth_bps u16, flow_min_usd u16, flow_bps u16, flow_imbalance_x u8,
    /// expiry_guard_secs u32, buyback_cap_bps u16, entry_cap_bps u16`, then 9
    /// zero bytes.
    pub fn decode(bytes: &[u8; BUYBACK_PARAMS_LEN]) -> Option<Self> {
        let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        (bytes[0] <= 1 && bytes[23..] == [0; BUYBACK_PARAMS_LEN - 23]).then(|| Self {
            enabled: bytes[0] == 1,
            gain_credit_bps: u16_at(1),
            min_valid_buckets: bytes[3],
            max_head_age_secs: u16_at(4),
            depth_min_contracts: u16_at(6),
            depth_bps: u16_at(8),
            flow_min_usd: u16_at(10),
            flow_bps: u16_at(12),
            flow_imbalance_x: bytes[14],
            expiry_guard_secs: u32::from_le_bytes([bytes[15], bytes[16], bytes[17], bytes[18]]),
            buyback_cap_bps: u16_at(19),
            entry_cap_bps: u16_at(21),
        })
    }

    pub fn encode(&self) -> [u8; BUYBACK_PARAMS_LEN] {
        let mut bytes = [0; BUYBACK_PARAMS_LEN];
        bytes[0] = u8::from(self.enabled);
        bytes[1..3].copy_from_slice(&self.gain_credit_bps.to_le_bytes());
        bytes[3] = self.min_valid_buckets;
        bytes[4..6].copy_from_slice(&self.max_head_age_secs.to_le_bytes());
        bytes[6..8].copy_from_slice(&self.depth_min_contracts.to_le_bytes());
        bytes[8..10].copy_from_slice(&self.depth_bps.to_le_bytes());
        bytes[10..12].copy_from_slice(&self.flow_min_usd.to_le_bytes());
        bytes[12..14].copy_from_slice(&self.flow_bps.to_le_bytes());
        bytes[14] = self.flow_imbalance_x;
        bytes[15..19].copy_from_slice(&self.expiry_guard_secs.to_le_bytes());
        bytes[19..21].copy_from_slice(&self.buyback_cap_bps.to_le_bytes());
        bytes[21..23].copy_from_slice(&self.entry_cap_bps.to_le_bytes());
        bytes
    }

    pub fn is_valid(&self, instant_daily_cap_bps: u16) -> bool {
        self.gain_credit_bps <= 10_000
            && (8..=32).contains(&self.min_valid_buckets)
            && (60..=3_600).contains(&self.max_head_age_secs)
            && (1..=1_000).contains(&self.depth_min_contracts)
            && self.depth_bps <= 2_000
            // A zero floor with zero bps would switch the flow rule off.
            && self.flow_min_usd >= 1
            && self.flow_bps <= 1_000
            && (1..=10).contains(&self.flow_imbalance_x)
            && (3_600..=172_800).contains(&self.expiry_guard_secs)
            && self.buyback_cap_bps >= 100
            && self.buyback_cap_bps <= instant_daily_cap_bps
            && (100..=5_000).contains(&self.entry_cap_bps)
    }
}

/// `D_i`: option atoms the ladder must offer.
pub fn depth_atoms(params: &BuybackParams, open_interest: u64) -> u64 {
    let floor = u64::from(params.depth_min_contracts) * CONTRACT_ATOMS;
    let share = u128::from(open_interest) * u128::from(params.depth_bps) / u128::from(BPS);
    floor.max(u64::try_from(share).unwrap_or(u64::MAX))
}

/// The bin of the `depth`-th cheapest option atom of an ascending ladder of
/// `(bin_id, option_atoms)`; `None` when the ladder is shorter.
pub fn ask_bin_at_depth(ladder: impl IntoIterator<Item = (u16, u64)>, depth: u64) -> Option<u16> {
    let mut cumulative = 0u64;
    for (bin, atoms) in ladder {
        if atoms == 0 {
            continue;
        }
        cumulative = cumulative.saturating_add(atoms);
        if cumulative >= depth {
            return Some(bin);
        }
    }
    None
}

/// The highest bid bin of a descending ladder of `(bin_id, quote_atoms)` and
/// the bin where its bids could buy back `depth` option atoms (each bin buys
/// `quote × 10^6 / (tick × bin)` atoms, saturating); `None` when the ladder
/// is shorter. Buying the depth costs at most `depth × tick × top / 10^6`.
pub fn bid_at_depth(
    ladder: impl IntoIterator<Item = (u16, u64)>,
    depth: u64,
    tick: u64,
) -> Option<(u16, u16)> {
    let mut top = None;
    let mut cumulative = 0u64;
    for (bin, quote) in ladder {
        let price = tick.saturating_mul(u64::from(bin));
        if quote == 0 || price == 0 {
            continue;
        }
        let top = *top.get_or_insert(bin);
        cumulative = cumulative.saturating_add(quote.saturating_mul(CONTRACT_ATOMS) / price);
        if cumulative >= depth {
            return Some((top, bin));
        }
    }
    None
}

/// Per-contract price of a bin, capped at the series' max payout.
#[inline(never)]
pub fn bin_price(bin: u16, tick: u64, max_payout: u64) -> u64 {
    if bin == NO_ASK {
        return max_payout;
    }
    tick.saturating_mul(u64::from(bin)).min(max_payout)
}

/// One series' entry in one ring bucket.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BucketEntry {
    /// Absolute bucket index `ts / BUCKET_SECS`.
    pub index: u64,
    pub max_ask_bin: u16,
    /// The lowest bid bin sampled in the bucket (`NO_BID` once any sample's
    /// bid was not executable).
    pub min_bid_bin: u16,
    pub flags: u8,
    pub sale_usd: u16,
    pub buy_usd: u16,
}

impl BucketEntry {
    fn in_window(&self, current: u64, width: u64) -> bool {
        self.flags & BUCKET_SAMPLED != 0 && self.index <= current && current - self.index < width
    }
}

/// The time-aggregated head of one series at bucket `current`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SeriesHead {
    /// `B_i` in quote atoms per contract.
    pub price: u64,
    /// `b_i` in quote atoms per contract (zero without an executable bid).
    pub bid_price: u64,
    pub valid_buckets: u8,
    pub sale_usd: u32,
    pub buy_usd: u32,
}

/// Insertion sort of at most 32 bucket statistics (no core sort machinery
/// on SBF), returning the nearest-rank `ceil(n × quarters / 4)`-th smallest.
#[inline(never)]
fn nearest_rank(bins: &mut [u16], quarters: usize) -> u16 {
    for next in 1..bins.len() {
        let mut at = next;
        while at > 0 && bins[at - 1] > bins[at] {
            bins.swap(at - 1, at);
            at -= 1;
        }
    }
    bins[(quarters * bins.len()).div_ceil(4) - 1]
}

/// `B_i`, `b_i`, coverage and recent flow from the 32 ring entries of one
/// series.
pub fn series_head(
    ring: impl IntoIterator<Item = BucketEntry>,
    current: u64,
    tick: u64,
    max_payout: u64,
) -> SeriesHead {
    let mut asks = [0u16; RING_BUCKETS];
    let mut bids = [0u16; RING_BUCKETS];
    let mut count = 0usize;
    let (mut recent, mut recent_ask, mut recent_bid) = (false, NO_BID, NO_ASK);
    let mut head = SeriesHead::default();
    for entry in ring.into_iter().take(RING_BUCKETS) {
        if !entry.in_window(current, RING_BUCKETS as u64) {
            continue;
        }
        asks[count] = entry.max_ask_bin;
        bids[count] = entry.min_bid_bin;
        count += 1;
        if entry.flags & BUCKET_ALL_VALID != 0 {
            head.valid_buckets += 1;
        }
        if current - entry.index < 2 {
            recent = true;
            recent_ask = recent_ask.max(entry.max_ask_bin);
            recent_bid = recent_bid.min(entry.min_bid_bin);
        }
        if current - entry.index < FLOW_BUCKETS {
            head.sale_usd += u32::from(entry.sale_usd);
            head.buy_usd += u32::from(entry.buy_usd);
        }
    }
    // Asks: the larger of the upper quartile and the recent maximum; bids:
    // the smaller of the lower quartile and the recent minimum. With no
    // recent bucket the ask is unpriceable and the bid worthless.
    let (ask, bid) = if recent {
        (
            nearest_rank(&mut asks[..count], 3).max(recent_ask),
            nearest_rank(&mut bids[..count], 1).min(recent_bid),
        )
    } else {
        (NO_ASK, NO_BID)
    };
    head.price = bin_price(ask, tick, max_payout);
    head.bid_price = bin_price(bid, tick, max_payout);
    head
}

/// Fold one sample into a ring entry. A newer bucket replaces the entry.
pub fn fold_sample(
    entry: &mut BucketEntry,
    current: u64,
    ask_bin: u16,
    bid_bin: u16,
    valid: bool,
    sale_usd: u64,
    buy_usd: u64,
) {
    let narrow = |value: u64| u16::try_from(value).unwrap_or(u16::MAX);
    if entry.index != current || entry.flags & BUCKET_SAMPLED == 0 {
        *entry = BucketEntry {
            index: current,
            max_ask_bin: ask_bin,
            min_bid_bin: bid_bin,
            flags: BUCKET_SAMPLED | if valid { BUCKET_ALL_VALID } else { 0 },
            sale_usd: narrow(sale_usd),
            buy_usd: narrow(buy_usd),
        };
        return;
    }
    entry.max_ask_bin = entry.max_ask_bin.max(ask_bin);
    entry.min_bid_bin = entry.min_bid_bin.min(bid_bin);
    if !valid {
        entry.flags &= !BUCKET_ALL_VALID;
    }
    entry.sale_usd = narrow(u64::from(entry.sale_usd) + sale_usd);
    entry.buy_usd = narrow(u64::from(entry.buy_usd) + buy_usd);
}

/// Whole-USD turnover since `counted`, and the counter to store: only whole
/// dollars are consumed, so sub-dollar remainders carry to the next sample.
pub fn whole_usd_delta(total: u64, counted: u64) -> (u64, u64) {
    if total < counted {
        return (0, total);
    }
    let usd = (total - counted) / USD_ATOMS;
    (usd, counted + usd * USD_ATOMS)
}

/// Rules V2–V4 for one series with open interest at its head, given the
/// sleeve's pooled principal (quote atoms).
pub fn series_viable(
    params: &BuybackParams,
    head: &SeriesHead,
    principal_atoms: u64,
    sample_valid: bool,
) -> bool {
    let threshold = u64::from(params.flow_min_usd)
        .max((principal_atoms / USD_ATOMS).saturating_mul(u64::from(params.flow_bps)) / BPS);
    let (sale, buy) = (u64::from(head.sale_usd), u64::from(head.buy_usd));
    sample_valid
        && head.valid_buckets >= params.min_valid_buckets
        && sale >= threshold
        && buy >= threshold
        && sale.max(buy) <= u64::from(params.flow_imbalance_x) * sale.min(buy)
}

/// `ceil(Σ OI_i × B_i / 10^6)`.
pub fn slot_liability(series: impl IntoIterator<Item = (u64, u64)>) -> Option<u64> {
    let mut numerator = 0u128;
    for (open_interest, price) in series {
        numerator = numerator.checked_add(u128::from(open_interest) * u128::from(price))?;
    }
    u64::try_from(numerator.div_ceil(u128::from(CONTRACT_ATOMS))).ok()
}

/// The fund lot's marked value: the claim payout at residual `assets − L`
/// with the unrealized gain above principal credited at `gain_credit_bps`.
/// `None` for inconsistent inputs; an insolvent lot is worth zero.
pub fn marked_lot_value(
    lot: ContributionInterval,
    principal: u64,
    capital_seconds: u128,
    assets: u64,
    liability: u64,
    gain_credit_bps: u16,
) -> Option<u64> {
    let range = FundRange {
        principal: lot.principal,
        start: lot.weight_offset,
        end: lot.weight_offset.checked_add(lot.weight().ok()?)?,
    };
    marked_range_value(
        range,
        principal,
        capital_seconds,
        assets,
        liability,
        gain_credit_bps,
    )
}

/// The fund's open lots in one sleeve: `principal` owning the contiguous
/// capital-seconds range `[start, end)` (`EarnFundSlotV1`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FundRange {
    pub principal: u64,
    pub start: u128,
    pub end: u128,
}

/// `marked_lot_value` of a contiguous range of lots: exactly the sum of the
/// lots' own values when none is insolvent, otherwise lower (an insolvent lot
/// is netted instead of floored at zero), never higher.
pub fn marked_range_value(
    range: FundRange,
    principal: u64,
    capital_seconds: u128,
    assets: u64,
    liability: u64,
    gain_credit_bps: u16,
) -> Option<u64> {
    let residual = assets.saturating_sub(liability);
    let credited = if residual > principal {
        // An unrepresentable credit is dropped (never overpays).
        let gain = (residual - principal)
            .checked_mul(u64::from(gain_credit_bps))
            .map_or(0, |value| value / BPS);
        principal.saturating_add(gain)
    } else {
        residual
    };
    match range_payout(
        range.principal,
        range.start,
        range.end,
        principal,
        capital_seconds,
        credited,
    ) {
        Ok(value) => Some(value),
        Err(ParticipationError::Insolvent) => Some(0),
        Err(_) => None,
    }
}

/// Rule V2's capacity proxy: whether the sleeve stays solvent selling one
/// more depth at the ask (reserve up by `depth × max_payout`, the same
/// participation guard). An approximation of `admit_totals`, which also
/// applies the policy's risk limits — not a sufficient condition for it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SaleCapacity {
    pub accounted_assets: u64,
    pub exact_reserve: u64,
    pub shared_reserve: bool,
    pub pooled_quote: u64,
    pub allocated_lp_quote: u64,
    pub operational_buffer: u64,
    pub totals: ParticipationTotals,
}

impl SaleCapacity {
    /// Selling `depth` atoms at `ask` per contract keeps the sleeve solvent:
    /// the reserve rises by at most `depth × max_payout`.
    pub fn admits(&self, depth: u64, ask: u64, max_payout: u64) -> bool {
        // Unrepresentable sizes are never admitted.
        let (Some(premium), Some(added)) = (depth.checked_mul(ask), depth.checked_mul(max_payout))
        else {
            return false;
        };
        let (Some(assets), Some(reserve)) = (
            self.accounted_assets.checked_add(premium / CONTRACT_ATOMS),
            self.exact_reserve
                .checked_add(added.div_ceil(CONTRACT_ATOMS)),
        ) else {
            return false;
        };
        let base = if self.shared_reserve {
            reserve.max(self.pooled_quote)
        } else {
            reserve
        };
        let Some(protected) = base.checked_add(self.operational_buffer) else {
            return false;
        };
        let Some(free) = assets.checked_sub(protected) else {
            return false;
        };
        (self.shared_reserve || self.allocated_lp_quote <= free)
            && (self.totals.principal == 0 || self.totals.admit(assets, reserve).is_ok())
    }
}

/// A token bucket drained linearly by `cap` per 24h.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExitBucket {
    pub updated_ts: u64,
    pub level: u64,
}

impl ExitBucket {
    pub fn level_at(&self, cap: u64, now: u64) -> u64 {
        let elapsed = now.saturating_sub(self.updated_ts).min(86_400);
        let drained = u128::from(cap) * u128::from(elapsed) / 86_400;
        self.level
            .saturating_sub(u64::try_from(drained).unwrap_or(u64::MAX))
    }

    /// The bucket after paying `payout`, if it stays within `cap`.
    pub fn admit(&self, cap: u64, now: u64, payout: u64) -> Option<Self> {
        let level = self.level_at(cap, now).checked_add(payout)?;
        (level <= cap).then_some(Self {
            updated_ts: now,
            level,
        })
    }
}
