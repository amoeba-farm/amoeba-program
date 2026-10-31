//! Channels: the long-term agreement (LTA) layer on heads v2 (`oracle/docs/lta.md`; the brief is
//! `oracle/docs/lta-spec.md`).
//!
//! A board's items (its basket members) are each observed in three channels, Spot, Contract and
//! LTA, and every `(item, channel)` pair is one component with its own frozen target sentence,
//! sources and weighted median. Nothing about how a component settles changes: it is still the
//! weighted median of its eligible sources' changes under the joint caps, with the same
//! degraded path. This module adds what the channels need on top:
//!
//! - **classification** ([`classify`]): which channel a purchase belongs to, LTA first;
//! - **origins** ([`duplicate_flags`], [`distinct_origins`]): thresholds count independent
//!   origins, and a source repeating an earlier source's origin carries no weight;
//! - **listing and freeze checks** ([`settles`], [`component_lists`], [`origins_pass`],
//!   [`listing_allowed`]);
//! - **LTA prints** ([`LtaTerms`], [`lta_level`]): a price past its own next reset is no level;
//! - **the fixed-mix procurement reference** ([`reference`]): per item, its Spot, Contract and
//!   LTA channels weighted against each other by the item's fixed mix, each channel at its own
//!   settlement (live, or its degraded path's last valid value), so a failing channel keeps
//!   its weight and the mix never shifts;
//! - **what a contract settles on** ([`settlement_change_ppm`]): the fixed-mix reference, on an
//!   assumed or a matched mix (the user, 2026-10-29: "NO WE DONT WANT THAT. SPOT LTA AND
//!   CONTRACT NEED TO BE WEIGHTED AGAINST EACHOTHER."), or, still available as a target, the
//!   Contract channel alone (the brief's Stage 1).
//!
//! The codes of every enum here are the on-chain bytes; they never change.

use crate::settle::ComponentSettlement;
use crate::BPS;

/// Channels per item.
pub const CHANNELS: usize = 3;

/// The channel a component observes. A purchase belongs to the first that fits, in the order
/// LTA, Contract, Spot ([`classify`]), or to none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Channel {
    /// Immediately available inventory bought outside both others.
    Spot = 0,
    /// A negotiated wholesale purchase, supplier to OEM/ODM, delivered in the observation
    /// period and not under an LTA.
    Contract = 1,
    /// Made under a supply agreement of at least a year with committed volume.
    Lta = 2,
}

impl Channel {
    /// In code order (the order of a [`Mix`]).
    pub const ALL: [Channel; CHANNELS] = [Channel::Spot, Channel::Contract, Channel::Lta];

    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Channel::Spot),
            1 => Some(Channel::Contract),
            2 => Some(Channel::Lta),
            _ => None,
        }
    }
}

/// What backs an item's mix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MixBasis {
    /// No mix: the reference is unavailable.
    None = 0,
    /// An assumed scenario: the reference settles and is flagged as assumed wherever it shows
    /// (the user's decision of 2026-10-29 supersedes the brief's "nothing settles on an
    /// assumed scenario").
    Assumed = 1,
    /// Matched evidence (same product population, delivery period, buyer and supplier
    /// population, volume unit and transaction stage).
    Matched = 2,
}

impl MixBasis {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(MixBasis::None),
            1 => Some(MixBasis::Assumed),
            2 => Some(MixBasis::Matched),
            _ => None,
        }
    }
}

/// What a listed contract settles on, fixed at listing and never changed mid-life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SettlementTarget {
    /// Every item's Contract channel alone (the brief's Stage 1). Still a target a listing may
    /// name; no mold names it since 2026-10-29.
    ContractChannel = 0,
    /// Every item's fixed-mix reference: its Spot, Contract and LTA channels weighted against
    /// each other by its mix, assumed or matched. The molds' target since 2026-10-29.
    FixedMixReference = 1,
}

impl SettlementTarget {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(SettlementTarget::ContractChannel),
            1 => Some(SettlementTarget::FixedMixReference),
            _ => None,
        }
    }
}

/// The ground a challenge names. Labels for the existing challenge: every reason has the same
/// bond, the same resolution ([`crate::outcome`]) and the same slash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ChallengeReason {
    /// The value or the evidence is wrong, or another ground.
    ValueOrOther = 0,
    /// The purchase classifies in another channel, or in none.
    OffChannel = 1,
    /// The evidence traces to an earlier live source's origin and the claim did not declare
    /// it (a source that declares a shared origin is a duplicate without any challenge).
    DuplicateOrigin = 2,
    /// An LTA price carried past its own next reset ([`LtaTerms::expired_at`]).
    ExpiredObservation = 3,
}

impl ChallengeReason {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(ChallengeReason::ValueOrOther),
            1 => Some(ChallengeReason::OffChannel),
            2 => Some(ChallengeReason::DuplicateOrigin),
            3 => Some(ChallengeReason::ExpiredObservation),
            _ => None,
        }
    }
}

/// An item's mix `[spot, contract, lta]` in basis points, in [`Channel`] code order.
pub type Mix = [u16; CHANNELS];

/// A mix is valid when its shares add up to exactly 10,000.
pub fn mix_is_valid(mix: &Mix) -> bool {
    mix.iter().map(|&m| u64::from(m)).sum::<u64>() == BPS
}

// ------------------------------------------------------------------------------------------
// Classification
// ------------------------------------------------------------------------------------------

/// An LTA is a supply agreement of at least a year (the brief, section 3.1).
pub const LTA_MIN_MONTHS: u16 = 12;

/// What a purchase's evidence declares, for [`classify`]. Challengers check these facts; the
/// channel follows from them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PurchaseFacts {
    /// A price both sides agreed, with its terms stated. False for an unterminated quote, a
    /// listing, or a resale without terms.
    pub agreed: bool,
    /// The term of the supply agreement the purchase is made under, in months (0: none).
    pub agreement_months: u16,
    /// That agreement commits volume.
    pub committed_volume: bool,
    /// A negotiated wholesale purchase from the supplier by an OEM or ODM (for BATX, a cell
    /// maker's sale to a vehicle OEM or an energy-storage integrator, a tender award included).
    pub negotiated_wholesale: bool,
    /// Delivered in the observation period.
    pub delivered_in_period: bool,
    /// Bought from immediately available inventory.
    pub immediate_inventory: bool,
}

/// The channel of a purchase, by the brief's hierarchy (section 3.1), first match wins:
/// LTA (made under an agreement of at least a year with committed volume, including a newly
/// signed one and a price newly reset under an old one), then Contract (a negotiated wholesale
/// purchase delivered in the period, not under an LTA), then Spot (immediate inventory outside
/// both). `None`: eligible nowhere (an unterminated quote, a resale without terms).
pub fn classify(facts: &PurchaseFacts) -> Option<Channel> {
    if !facts.agreed {
        return None;
    }
    if facts.agreement_months >= LTA_MIN_MONTHS && facts.committed_volume {
        return Some(Channel::Lta);
    }
    if facts.negotiated_wholesale && facts.delivered_in_period {
        return Some(Channel::Contract);
    }
    if facts.immediate_inventory {
        return Some(Channel::Spot);
    }
    None
}

// ------------------------------------------------------------------------------------------
// Origins
// ------------------------------------------------------------------------------------------

/// A claim's `origin_id`: the fetcher-declared hash of the underlying report's identity
/// (`ameba_commodity_spec::origin_id` is the canonical one). Equal bytes are one origin; zero
/// is not special.
pub type Origin = [u8; 32];

/// One source of a component, for the origin rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OriginEntry {
    /// The origin its current claim declares.
    pub origin: Origin,
    /// When it went live (its fill stood).
    pub live_at: u64,
    pub head_id: u32,
    /// Whether it is in the set being judged: live, for thresholds and rewards; eligible
    /// (live, with a level and raw weight), at settlement. Entries that don't count are never
    /// duplicates and never make another one a duplicate.
    pub counts: bool,
}

impl OriginEntry {
    fn precedence(&self) -> (u64, u32) {
        (self.live_at, self.head_id)
    }
}

/// For each entry, the index of the first entry with its origin (itself when none comes
/// before it). The entries are sorted by origin, stably (`median::sorted_positions`: about
/// `n log2 n` four-word comparisons), and each run of one origin takes its first position: one pass shared by everything the origin rules decide (round 7 review: CU at full
/// size; round 13 review: a settlement's 192 members, which comparing each entry with every
/// origin before it took quadratically).
#[inline(never)]
pub fn first_of_origin(entries: &[OriginEntry]) -> Vec<usize> {
    let n = entries.len();
    let mut words: Vec<[u64; 4]> = Vec::with_capacity(n);
    for e in entries {
        let mut w = [0u64; 4];
        for (k, word) in w.iter_mut().enumerate() {
            *word = u64::from_le_bytes(e.origin[8 * k..8 * k + 8].try_into().unwrap_or_default());
        }
        words.push(w);
    }
    // By origin word by word, compared in place (no key is copied); the sort is stable, so each
    // run of one origin keeps its positions in order.
    let order = crate::median::sorted_positions(n, &|a, b| {
        let (x, y) = (&words[a], &words[b]);
        for k in 0..4 {
            if x[k] != y[k] {
                return x[k] < y[k];
            }
        }
        false
    });
    let mut first = vec![0; n];
    let mut leader = 0;
    for (at, &i) in order.iter().enumerate() {
        if at == 0 || words[order[at - 1]][..] != words[i][..] {
            leader = i;
        }
        first[i] = leader;
    }
    first
}

/// The origins (`first_of_origin`'s groups) with an entry `keep` marks: one grouping serves
/// several counts.
pub fn count_origins(first: &[usize], keep: &[bool]) -> usize {
    let mut seen = vec![false; first.len()];
    let mut origins = 0;
    for (k, &g) in first.iter().enumerate() {
        if keep[k] && !seen[g] {
            seen[g] = true;
            origins += 1;
        }
    }
    origins
}

/// Which counting entries are duplicates: the first counting entry of each origin (earliest
/// `live_at`, then lowest head id) carries it, and every later one with the same origin is a
/// duplicate; and the independent origins among the counting entries `also` marks. A duplicate
/// carries no weight, does not count toward `min_live_sources`, and its delegation earns no
/// delegator share (the brief, section 5 rule 1). The result does not depend on the order of
/// `entries`.
pub fn duplicates_and_origins(entries: &[OriginEntry], also: &[bool]) -> (Vec<bool>, usize) {
    let first = first_of_origin(entries);
    // Per origin (indexed by its first entry), its counting entry of least precedence.
    let mut lead = vec![usize::MAX; entries.len()];
    for (k, e) in entries.iter().enumerate() {
        let g = first[k];
        if e.counts && (lead[g] == usize::MAX || e.precedence() < entries[lead[g]].precedence()) {
            lead[g] = k;
        }
    }
    let mut flags = Vec::with_capacity(entries.len());
    let mut kept = Vec::with_capacity(entries.len());
    for (k, e) in entries.iter().enumerate() {
        flags.push(e.counts && lead[first[k]] != k);
        kept.push(e.counts && also[k]);
    }
    (flags, count_origins(&first, &kept))
}

/// Which counting entries are duplicates ([`duplicates_and_origins`]).
pub fn duplicate_flags(entries: &[OriginEntry]) -> Vec<bool> {
    duplicates_and_origins(entries, &vec![true; entries.len()]).0
}

/// The independent origins among the counting entries.
pub fn distinct_origins(entries: &[OriginEntry]) -> usize {
    let mut counting = Vec::with_capacity(entries.len());
    for e in entries {
        counting.push(e.counts);
    }
    count_origins(&first_of_origin(entries), &counting)
}

// ------------------------------------------------------------------------------------------
// Listing and freeze
// ------------------------------------------------------------------------------------------

/// Whether a contract with this target settles on this channel of an item with this mix: the
/// fixed-mix reference on every channel the mix weights (share > 0), the Contract-channel target
/// on Contract only. A channel it does not settle on is displayed.
pub fn settles(target: SettlementTarget, channel: Channel, mix: &Mix) -> bool {
    match target {
        SettlementTarget::ContractChannel => channel == Channel::Contract,
        SettlementTarget::FixedMixReference => mix[channel.index()] > 0,
    }
}

/// Whether a component's independent origins meet `min_live_sources`: the check at listing and
/// again at freeze. A component that passed at listing and fails at freeze settles on its
/// degraded path ([`crate::settle::settle_component`] with `freeze_passed = false`), never on
/// stale sources.
pub fn origins_pass(origins: usize, min_live_sources: u16) -> bool {
    origins >= usize::from(min_live_sources.max(1))
}

/// Whether a component lets its contract list: a component the target settles on needs
/// `min_live_sources` independent origins ([`origins_pass`]); a display component ([`settles`]
/// false: a channel the mix gives no share, or Spot and LTA under the Contract-channel target)
/// lists however few it has, so it can show a median once sources arrive.
pub fn component_lists(origins: usize, min_live_sources: u16, settling: bool) -> bool {
    !settling || origins_pass(origins, min_live_sources)
}

/// Whether a contract may list with this target on items with these mixes and bases: the
/// Contract-channel target always (its reference is displayed); the fixed-mix reference when
/// every item has a valid mix ([`mix_is_valid`]) on an assumed or a matched basis, never on
/// none. Each settling component must also have `min_live_sources` independent origins
/// ([`component_lists`]).
pub fn listing_allowed(target: SettlementTarget, items: &[(Mix, MixBasis)]) -> bool {
    match target {
        SettlementTarget::ContractChannel => true,
        SettlementTarget::FixedMixReference => {
            !items.is_empty()
                && items
                    .iter()
                    .all(|(mix, basis)| *basis != MixBasis::None && mix_is_valid(mix))
        }
    }
}

// ------------------------------------------------------------------------------------------
// LTA prints
// ------------------------------------------------------------------------------------------

/// The terms an LTA claim carries with its value (0 = none for each).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LtaTerms {
    /// When the price takes effect (unix seconds).
    pub effective_from_ts: u64,
    /// The agreement's next reset (unix seconds): past it, the price is no longer in effect.
    pub next_reset_ts: u64,
    /// The band, at the value's scale.
    pub floor: u64,
    pub ceiling: u64,
}

impl LtaTerms {
    /// The terms are consistent: a reset after the effective date, and a floor at most the
    /// ceiling, where both are set.
    pub fn valid(&self) -> bool {
        (self.effective_from_ts == 0
            || self.next_reset_ts == 0
            || self.effective_from_ts < self.next_reset_ts)
            && (self.floor == 0 || self.ceiling == 0 || self.floor <= self.ceiling)
    }

    /// `value` lies inside the declared band. A claim outside its own band contradicts its
    /// evidence (challengeable as a value).
    pub fn within_band(&self, value: u64) -> bool {
        (self.floor == 0 || value >= self.floor) && (self.ceiling == 0 || value <= self.ceiling)
    }

    /// Carried past its own next reset at `at`: an observation claimed at or after its reset is
    /// expired, a false claim (challenge reason `ExpiredObservation`).
    pub fn expired_at(&self, at: u64) -> bool {
        self.next_reset_ts != 0 && at >= self.next_reset_ts
    }

    /// The price is in effect at `at`: effective, and not past its reset. An old publication of
    /// a price still in effect is current.
    pub fn current_at(&self, at: u64) -> bool {
        self.effective_from_ts <= at && !self.expired_at(at)
    }
}

/// One accepted print of an LTA source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LtaPrint {
    pub source_ts: u64,
    pub value: u64,
    pub terms: LtaTerms,
}

/// An LTA source's level over the window `[start, end]`: the oracle's temporal median
/// ([`crate::settle::temporal_level`]) over only the prints in effect at `end`. A print past
/// its own next reset, or not yet effective, is no current level; a source with none has no
/// level and is not eligible.
pub fn lta_level(prints: &[LtaPrint], start: u64, end: u64) -> Option<u64> {
    let current: Vec<(u64, u64)> = prints
        .iter()
        .filter(|p| p.terms.current_at(end))
        .map(|p| (p.source_ts, p.value))
        .collect();
    crate::settle::temporal_level(&current, start, end)
}

/// A source's level in its channel: [`lta_level`] for LTA, the temporal median of every print
/// otherwise (Spot and Contract prints carry no terms).
pub fn channel_level(channel: Channel, prints: &[LtaPrint], start: u64, end: u64) -> Option<u64> {
    match channel {
        Channel::Lta => lta_level(prints, start, end),
        Channel::Spot | Channel::Contract => {
            let all: Vec<(u64, u64)> = prints.iter().map(|p| (p.source_ts, p.value)).collect();
            crate::settle::temporal_level(&all, start, end)
        }
    }
}

// ------------------------------------------------------------------------------------------
// The fixed-mix procurement reference
// ------------------------------------------------------------------------------------------

/// One channel of an item, as the reference reads it: its component's own settlement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelReading {
    /// The component's change: on live data, or its degraded path's last valid value (the
    /// change at the freeze when that settled live, else the listing value, 0).
    pub change_ppm: i64,
    /// It settled on its degraded path.
    pub degraded: bool,
}

impl ChannelReading {
    pub fn new(settlement: &ComponentSettlement) -> Self {
        Self {
            change_ppm: settlement.change_ppm(),
            degraded: !settlement.is_live(),
        }
    }
}

/// Why an item's reference is unavailable. Codes are the on-chain `item_reference_reason`
/// (0 is "none"). Codes 1 (a channel below `min_live_sources` origins at freeze) and 3 (a
/// channel settled degraded) are retired since 2026-10-29: such a channel now enters the
/// reference at its last valid value, and the codes are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// The item's mix basis is none.
    MixBasisNone,
    /// The mix does not add up to 10,000 (a listing never freezes one).
    InvalidMix,
}

impl Unavailable {
    pub const fn code(self) -> u8 {
        match self {
            Unavailable::MixBasisNone => 2,
            Unavailable::InvalidMix => 4,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            2 => Some(Unavailable::MixBasisNone),
            4 => Some(Unavailable::InvalidMix),
            _ => None,
        }
    }
}

/// An item's fixed-mix procurement reference over a contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reference {
    Available {
        /// `Σ m_c · change_c` in ppm: on the contract's base, the brief's
        /// `m_spot · Spot + m_contract · Contract + m_lta · LTA`.
        change_ppm: i64,
        /// The mix is an assumed scenario: it settles, flagged as assumed.
        assumed: bool,
        /// Per channel (code order): the mix weights it and it settled on its degraded path, so
        /// the reference holds its last valid value at its fixed share.
        degraded: [bool; CHANNELS],
    },
    Unavailable(Unavailable),
}

impl Reference {
    pub fn change_ppm(&self) -> Option<i64> {
        match self {
            Reference::Available { change_ppm, .. } => Some(*change_ppm),
            Reference::Unavailable(_) => None,
        }
    }

    /// The degraded channels as the on-chain bit mask (`1 << channel code`); 0 when unavailable.
    pub fn degraded_mask(&self) -> u8 {
        match self {
            Reference::Available { degraded, .. } => channel_mask(degraded),
            Reference::Unavailable(_) => 0,
        }
    }
}

/// Per-channel flags as a bit mask, bit `1 << channel code`.
pub fn channel_mask(flags: &[bool; CHANNELS]) -> u8 {
    Channel::ALL
        .iter()
        .filter(|c| flags[c.index()])
        .fold(0, |mask, c| mask | 1 << c.code())
}

/// `Σ_c mix_c · change_c / 10,000`, rounded toward zero.
pub fn mix_change_ppm(mix: &Mix, changes: &[i64; CHANNELS]) -> i64 {
    let sum: i128 = mix
        .iter()
        .zip(changes)
        .map(|(&m, &c)| i128::from(m) * i128::from(c))
        .sum();
    // |Σ| <= 10,000 * max|c|, so the quotient fits i64.
    (sum / i128::from(BPS)) as i64
}

/// An item's fixed-mix procurement reference (the brief, section 4, as the user decided it on
/// 2026-10-29): `Σ_c mix_c · change_c`, each channel at its own component's settlement, live or
/// its degraded path's last valid value. Weight is never redistributed and the mix never shifts
/// (the brief's rule 6): a channel short of origins at freeze, or with too few eligible
/// origins or too little weight at settlement, keeps its fixed share at its last valid value,
/// and is flagged. Unavailable only when the mix basis is none or the mix does not add up to
/// 10,000, neither of which a fixed-mix listing accepts ([`listing_allowed`]).
pub fn reference(mix: &Mix, basis: MixBasis, readings: &[ChannelReading; CHANNELS]) -> Reference {
    if basis == MixBasis::None {
        return Reference::Unavailable(Unavailable::MixBasisNone);
    }
    if !mix_is_valid(mix) {
        return Reference::Unavailable(Unavailable::InvalidMix);
    }
    let changes = readings.map(|r| r.change_ppm);
    let degraded = Channel::ALL.map(|c| mix[c.index()] > 0 && readings[c.index()].degraded);
    Reference::Available {
        change_ppm: mix_change_ppm(mix, &changes),
        assumed: basis == MixBasis::Assumed,
        degraded,
    }
}

// ------------------------------------------------------------------------------------------
// What settles
// ------------------------------------------------------------------------------------------

/// One item at settlement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemSettlement {
    pub basket_weight_bps: u16,
    /// Its Contract component's change (live, or its degraded value).
    pub contract_change_ppm: i64,
    pub reference: Reference,
}

impl ItemSettlement {
    /// The change this item settles on under `target`: its Contract component's, or its
    /// reference's.
    pub fn change_ppm(&self, target: SettlementTarget) -> Result<i64, Unavailable> {
        match (target, self.reference) {
            (SettlementTarget::ContractChannel, _) => Ok(self.contract_change_ppm),
            (SettlementTarget::FixedMixReference, Reference::Available { change_ppm, .. }) => {
                Ok(change_ppm)
            }
            (SettlementTarget::FixedMixReference, Reference::Unavailable(reason)) => Err(reason),
        }
    }
}

/// Why a contract cannot settle on its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The basket weights do not add up to 10,000.
    BasketWeights,
    /// Fixed-mix reference: this item's reference is unavailable (its mix basis is none or its
    /// mix is invalid, which a fixed-mix listing refuses, so a listed contract never meets it).
    Unavailable { item: usize, reason: Unavailable },
}

/// The Contract-channel target (the brief's Stage 1): `Σ items basket_w · Contract change`.
/// Each Contract component keeps the heads degraded path, so this always settles while the
/// basket weights are whole.
pub fn stage1_change_ppm(items: &[ItemSettlement]) -> Result<i64, Refusal> {
    change_ppm_on(SettlementTarget::ContractChannel, items)
}

/// The fixed-mix reference target: `Σ items basket_w · reference`, on assumed and matched mixes
/// alike. Every channel keeps the heads degraded path inside the reference, so this always
/// settles a listed contract.
pub fn stage2_change_ppm(items: &[ItemSettlement]) -> Result<i64, Refusal> {
    change_ppm_on(SettlementTarget::FixedMixReference, items)
}

fn change_ppm_on(target: SettlementTarget, items: &[ItemSettlement]) -> Result<i64, Refusal> {
    let mut pairs = Vec::with_capacity(items.len());
    for (item, i) in items.iter().enumerate() {
        let change = i
            .change_ppm(target)
            .map_err(|reason| Refusal::Unavailable { item, reason })?;
        pairs.push((i.basket_weight_bps, change));
    }
    crate::settle::index_change_ppm(&pairs).ok_or(Refusal::BasketWeights)
}

/// The index change a contract settles on, by its target.
pub fn settlement_change_ppm(
    target: SettlementTarget,
    items: &[ItemSettlement],
) -> Result<i64, Refusal> {
    change_ppm_on(target, items)
}
