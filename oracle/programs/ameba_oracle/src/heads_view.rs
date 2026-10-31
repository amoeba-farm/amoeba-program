//! Oracle v2 ("heads on pikes", `docs/heads.md`) on the host: decodes a board's accounts into
//! one view and computes its live, indicative numbers with the shared arithmetic
//! (`ameba-oracle-heads`), so the API, ops (the maker's live index) and the chain read a board
//! the same way. Nothing here settles anything: settlement is the program's, on the
//! time-weighted settlement weights it collects at freeze. This is the board *now*.
//!
//! Per component (an `(item, channel)` pair, `docs/lta.md`):
//! - its live sources, their origins and duplicates (`channels::duplicate_flags`: a later
//!   source repeating an earlier one's origin carries no weight), and each live source's share
//!   of the component's weight now: its delegation over the non-duplicate sources' (no source or
//!   address cap since 2026-10-29);
//! - against a listed contract, its indicative change over the contract's openings, with each
//!   member's raw weight in it so far: before the freeze a provisional one, the settlement rule
//!   (`settle::settle_component`) on the members' latest levels ([`level_now`]: each head's
//!   latest print by source time, as the program keeps it; an LTA price past its own reset is no
//!   level); from the freeze on, settlement's own calculation on what it would record now
//!   ([`level_record`]: the medians of each member's samples and the origins deciding them;
//!   `settle::settle_recorded`: on its degraded path, the freeze fallback) (round 14 review, N2).
//!
//! Per item, the fixed-mix procurement reference (`channels::reference`: its channels weighted
//! by its mix, each at its reading now, one on its degraded path at its last valid value then:
//! its listing value, 0, before the freeze, its freeze fallback after; flagged when the mix is
//! an assumed scenario), and per
//! contract the indicative index change on its settlement target (the references, or every
//! item's Contract component; `channels::settlement_change_ppm`), or the recorded one once it
//! settled.
//!
//! Callers read the accounts (by layout, `getProgramAccounts` or a scan cache) and hand them in
//! decoded with [`decode`]; this module reads no chain. Host only.

use crate::state::heads::{
    heads_level_record, heads_rebuild_slot, heads_sample_instants, Delegation, Head, HeadStatus,
    HeadsAccount, HeadsBoard, HeadsContract, HeadsContractStatus, HeadsLevelRecord,
    HeadsWeightSource, HeadsWeightsHeader, CAPTURE_FREEZE, CAPTURE_JOINED, CAPTURE_LISTING,
    COMPONENT_FREEZE_CHECKED, COMPONENT_FREEZE_PASSED, COMPONENT_LIVE, COMPONENT_SETTLED,
    DEGRADED_FREEZE_CHECK_FAILED, DEGRADED_TOO_FEW_ORIGINS, DEGRADED_TOO_LITTLE_WEIGHT,
    HEADS_CHANNELS, HEADS_CONTRACT_RING, HEADS_MAX_COMPONENTS, HEADS_MAX_ITEMS,
    HEAD_PRINT_CAPACITY, REFERENCE_ASSUMED, REFERENCE_AVAILABLE, REFERENCE_COMPUTED,
    RING_FREEZE_CAPTURED,
};
use ameba_oracle_heads::{
    channels::{
        self, Channel, ChannelReading, ItemSettlement, LtaPrint, LtaTerms, MixBasis, OriginEntry,
        Reference, Refusal, SettlementTarget, Unavailable, CHANNELS,
    },
    rewards, settle, thresholds, time, Params, BPS,
};
use solana_program::pubkey::Pubkey;

// ---------------------------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------------------------

/// A heads account of type `T` at `key`: owned by `program`, exactly `T::LEN` bytes, every
/// layout check of the program's own load (discriminator, version, field domains, zero tail,
/// invariants), and the canonical PDA of its stored identities. `None` for anything else.
pub fn decode<T: HeadsAccount>(
    program: &Pubkey,
    key: &Pubkey,
    owner: &Pubkey,
    data: &[u8],
) -> Option<Box<T>> {
    if owner != program || data.len() != T::LEN {
        return None;
    }
    let value = T::unpack(data).ok()?;
    (value.address(program).as_ref() == Some(key)).then_some(value)
}

/// One `HeadsWeights` account: its header and source records (the cells are not needed here).
#[derive(Clone, Debug)]
pub struct Weights {
    pub header: HeadsWeightsHeader,
    pub sources: Vec<HeadsWeightSource>,
}

impl Weights {
    /// The settlement weight recorded for `head_id` (its head's time-weighted delegation over
    /// the window), once `SettleComponent` recorded its level.
    pub fn weight(&self, head_id: u32) -> Option<u64> {
        let recorded = usize::from(self.header.levels_recorded);
        self.sources[..recorded.min(self.sources.len())]
            .iter()
            .find(|s| s.head_id == head_id)
            .map(|s| s.weight)
    }
}

/// A `HeadsWeights` account at `key` owned by `program`: its length is what its header's
/// capacities lay out, its PDA is canonical, and every source record decodes.
pub fn decode_weights(
    program: &Pubkey,
    key: &Pubkey,
    owner: &Pubkey,
    data: &[u8],
) -> Option<Weights> {
    if owner != program {
        return None;
    }
    let (header, layout) = HeadsWeightsHeader::unpack_account(data).ok()?;
    if header.address(program).as_ref() != Some(key) {
        return None;
    }
    let sources = (0..usize::from(header.source_count))
        .map(|i| HeadsWeightSource::unpack(data.get(layout.source_range(i))?).ok())
        .collect::<Option<Vec<_>>>()?;
    Some(Weights { header, sources })
}

/// A product id as its symbol: zero-padded printable ASCII ("RAMX").
pub fn product_symbol(id: &[u8; 32]) -> Option<String> {
    let end = id.iter().position(|b| *b == 0).unwrap_or(32);
    let text = core::str::from_utf8(&id[..end]).ok()?;
    (end > 0 && id[end..].iter().all(|b| *b == 0) && text.bytes().all(|b| b.is_ascii_graphic()))
        .then(|| text.to_owned())
}

/// A head's accepted prints in the order they finalized, oldest first, each with its own LTA
/// terms (zero off the LTA channel): for display only, never a level ([`level_now`]).
pub fn head_prints(head: &Head) -> Vec<LtaPrint> {
    let count = head.print_count as usize;
    let kept = count.min(HEAD_PRINT_CAPACITY);
    (count - kept..count)
        .map(|i| {
            let s = i % HEAD_PRINT_CAPACITY;
            LtaPrint {
                source_ts: head.print_source_ts[s],
                value: head.print_values[s],
                terms: LtaTerms {
                    effective_from_ts: head.print_effective_from_ts[s],
                    next_reset_ts: head.print_next_reset_ts[s],
                    floor: head.print_floor[s],
                    ceiling: head.print_ceiling[s],
                },
            }
        })
        .collect()
}

/// The channel of a component byte (the layouts keep it within the three).
pub fn channel_of(code: u8) -> Channel {
    Channel::from_code(code).unwrap_or(Channel::Spot)
}

fn settlement_target(target: crate::state::heads::HeadsSettlementTarget) -> SettlementTarget {
    SettlementTarget::from_code(target as u8).unwrap_or(SettlementTarget::ContractChannel)
}

// ---------------------------------------------------------------------------------------------
// The input
// ---------------------------------------------------------------------------------------------

/// One board and the accounts of it the view reads, each decoded with [`decode`] (or
/// [`decode_weights`]) and keyed by its address.
pub struct BoardAccounts {
    pub address: Pubkey,
    pub board: Box<HeadsBoard>,
    /// The board's heads, any status.
    pub heads: Vec<(Pubkey, Box<Head>)>,
    /// The delegations on those heads (any amount).
    pub delegations: Vec<(Pubkey, Box<Delegation>)>,
    /// The board's contracts, any status.
    pub contracts: Vec<(Pubkey, Box<HeadsContract>)>,
    /// Weights accounts of those contracts' components, where read.
    pub weights: Vec<(Pubkey, Weights)>,
}

impl BoardAccounts {
    pub fn head(&self, head_id: u32) -> Option<&(Pubkey, Box<Head>)> {
        self.heads.iter().find(|(_, h)| h.head_id == head_id)
    }

    /// The delegations on the head at `head`.
    pub fn delegations_on<'a>(
        &'a self,
        head: &'a Pubkey,
    ) -> impl Iterator<Item = &'a (Pubkey, Box<Delegation>)> + 'a {
        self.delegations
            .iter()
            .filter(move |(_, d)| d.head == *head)
    }

    /// The weights account of `contract`'s component `component`, where read.
    pub fn weights_of(&self, contract: &Pubkey, component: u8) -> Option<&Weights> {
        self.weights
            .iter()
            .map(|(_, w)| w)
            .find(|w| w.header.contract == *contract && w.header.component == component)
    }

    /// The contract the board's current changes are measured against: the open contract with
    /// the nearest expiry that still accumulates, else the nearest one collecting weights.
    /// `None` while no contract is open.
    pub fn front_contract(&self) -> Option<usize> {
        let nearest = |status: HeadsContractStatus| {
            self.contracts
                .iter()
                .enumerate()
                .filter(|(_, (_, k))| k.status == status)
                .min_by_key(|(_, (_, k))| (k.expiry_ts, k.period))
                .map(|(i, _)| i)
        };
        nearest(HeadsContractStatus::Accumulating)
            .or_else(|| nearest(HeadsContractStatus::Collecting))
    }
}

// ---------------------------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------------------------

/// What posting and activation take now (spec §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thresholds {
    /// `max(floor_post, alpha * D / max(n, 1))`, `n` the live sources that carry weight.
    pub min_post: u64,
    /// `max(floor_activate, beta * m)`, `m` the median delegation of those sources.
    pub activation: u64,
}

/// One live source of a component, now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceView {
    pub head_id: u32,
    pub head: Pubkey,
    /// The origin its last accepted print declared; from a contract's freeze on, the origin of
    /// the print deciding its last sample (settlement's).
    pub origin: [u8; 32],
    pub live_at: u64,
    /// An earlier live source of the component has its origin: no weight, no share.
    pub duplicate: bool,
    /// Its current delegation (active and fresh, AMBA atoms).
    pub delegation: u64,
    /// Its weight: against a contract its raw weight in it so far (`head_raw_weight`, its
    /// time-weighted live delegation over the window), else its delegation now; 0 for a
    /// duplicate.
    pub weight: u64,
    /// Its share of the component's weight, in basis points (the shares of a component add up
    /// to 10,000 while any source carries weight; a duplicate's is 0).
    pub share_bps: u16,
    /// Its level now: before a contract's freeze its latest print ([`level_now`]), from the
    /// freeze on the median of its samples as settlement records it ([`level_record`]); `None`
    /// without one.
    pub level: Option<u64>,
    /// Its opening in the contract measured against: its value at listing, or its fill value
    /// if it went live inside the window; `None` without a contract or after the freeze.
    pub opening: Option<u64>,
    /// `settle::change_ppm(opening, level)`.
    pub change_ppm: Option<i64>,
}

/// A component's reading against a contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reading {
    /// No contract is open: there are no openings to measure a change from.
    NoContract,
    /// The settlement rule on the sources now settles on live data.
    Live {
        change_ppm: i64,
        /// Eligible sources (one per independent origin, with a level and weight).
        eligible: u16,
        /// Otherwise eligible sources dropped as a repeated origin.
        duplicates: u16,
        /// The eligible sources' weight in the median.
        weight: u64,
    },
    /// The settlement rule on the sources now falls to the component's last valid value.
    Degraded { reason: settle::Degraded },
    /// Recorded by the contract: the component settled.
    Settled {
        change_ppm: i64,
        /// On live data (else at its last valid value, for `reason`).
        live: bool,
        reason: Option<settle::Degraded>,
    },
}

impl Reading {
    /// The change on live data (now, or as settled), the one a reference reads.
    pub fn live_change_ppm(&self) -> Option<i64> {
        match self {
            Reading::Live { change_ppm, .. } => Some(*change_ppm),
            Reading::Settled {
                change_ppm,
                live: true,
                ..
            } => Some(*change_ppm),
            _ => None,
        }
    }
}

/// One component, now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentView {
    pub index: u8,
    pub item: u8,
    pub channel: Channel,
    /// Its target sentence's commitment (the contract's, frozen at listing, when measured
    /// against one; else the board's).
    pub sentence_hash: [u8; 32],
    /// The settlement target settles on it (`channels::settles`: a channel its item's mix
    /// weights, or Contract alone under the Contract-channel target).
    pub settles: bool,
    /// Live sources, duplicates included.
    pub live_sources: u16,
    /// Independent origins among them (`min_live_sources` counts these).
    pub live_origins: u16,
    /// The live sources, by head id.
    pub sources: Vec<SourceView>,
    pub reading: Reading,
    /// The change of its degraded path, the last valid value it settles at off live data: on a
    /// `Degraded` reading, its listing value (0) before the freeze or after a failed freeze check,
    /// else the settlement on its members' freeze levels (0 when those fall short too), as
    /// `SettleComponent` computes it; the recorded change once it settled on that path; 0 on live
    /// data.
    pub degraded_change_ppm: i64,
}

/// Why an item's reference is not available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceGap {
    /// The reference arithmetic's own reasons (`channels::Unavailable`).
    Reference(Unavailable),
    /// No contract is open, so the channels have no changes yet.
    NoContract,
}

/// An item's fixed-mix procurement reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceView {
    Available {
        change_ppm: i64,
        /// An assumed scenario: it settles, flagged as assumed.
        assumed: bool,
        /// Per channel (code order): weighted by the mix and on its degraded path, so held at
        /// its last valid value (before it settles, its listing value, 0).
        degraded: [bool; CHANNELS],
        /// Recorded by the contract (else indicative, from the channels now).
        recorded: bool,
    },
    Unavailable(ReferenceGap),
}

impl ReferenceView {
    fn as_reference(&self) -> Reference {
        match *self {
            ReferenceView::Available {
                change_ppm,
                assumed,
                degraded,
                ..
            } => Reference::Available {
                change_ppm,
                assumed,
                degraded,
            },
            ReferenceView::Unavailable(ReferenceGap::Reference(u)) => Reference::Unavailable(u),
            ReferenceView::Unavailable(ReferenceGap::NoContract) => {
                Reference::Unavailable(Unavailable::MixBasisNone)
            }
        }
    }
}

/// One item (basket member).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemView {
    pub index: u8,
    pub bucket_id: [u8; 32],
    pub basket_weight_bps: u16,
    /// `[spot, contract, LTA]` in basis points.
    pub mix_bps: [u16; HEADS_CHANNELS],
    pub mix_basis: MixBasis,
    pub reference: ReferenceView,
}

/// Why a contract's index change cannot be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexGap {
    NoContract,
    /// The target refuses (the fixed mix on an unavailable reference, or the basket weights not
    /// whole).
    Refused(Refusal),
}

/// A contract's index change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexReading {
    /// What its target would settle on now; `degraded` are the settling components that would
    /// fall to their last valid value (`ComponentView::degraded_change_ppm`: their listing value,
    /// zero, before the freeze, their freeze fallback after).
    Indicative {
        change_ppm: i64,
        value: Option<u64>,
        degraded: Vec<u8>,
    },
    /// Settled: the published change and value.
    Settled {
        change_ppm: i64,
        value: u64,
    },
    Unavailable(IndexGap),
}

/// One contract, read now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractView {
    pub address: Pubkey,
    pub period: Pubkey,
    pub target: SettlementTarget,
    pub components: Vec<ComponentView>,
    pub items: Vec<ItemView>,
    pub index: IndexReading,
}

/// A board, now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardView {
    pub product: Option<String>,
    pub params: Params,
    pub max_liability_bps: u64,
    pub thresholds: Thresholds,
    /// Live sources that carry weight (not duplicates), over every component.
    pub weighted_sources: usize,
    /// The contract the components' changes are measured against
    /// ([`BoardAccounts::front_contract`]), by its index in the accounts' contracts.
    pub front: Option<usize>,
    /// The front contract's target, else the molds' (the fixed-mix reference).
    pub settlement_target: SettlementTarget,
    /// The board's components, against the front contract.
    pub components: Vec<ComponentView>,
    /// The board's items, against the front contract.
    pub items: Vec<ItemView>,
    /// Every contract of the accounts, in their order.
    pub contracts: Vec<ContractView>,
}

impl BoardView {
    /// A live source's share of its component's weight now; `None` for a head that is not a
    /// live source.
    pub fn share_bps(&self, head_id: u32) -> Option<u16> {
        self.components
            .iter()
            .flat_map(|c| &c.sources)
            .find(|s| s.head_id == head_id)
            .map(|s| s.share_bps)
    }

    pub fn component(&self, index: u8) -> Option<&ComponentView> {
        self.components.get(usize::from(index))
    }

    /// The front contract's index reading.
    pub fn front_index(&self) -> IndexReading {
        self.front
            .and_then(|i| self.contracts.get(i))
            .map(|c| c.index.clone())
            .unwrap_or(IndexReading::Unavailable(IndexGap::NoContract))
    }
}

/// Reads `accounts` at `now` (unix seconds of the chain clock).
pub fn view(accounts: &BoardAccounts, now: u64) -> BoardView {
    let board = &accounts.board;
    let params = board.params.to_params();
    let contracts: Vec<ContractView> = accounts
        .contracts
        .iter()
        .map(|(address, k)| contract_view(accounts, address, k, now))
        .collect();
    let front = accounts.front_contract();
    // The live list, apart from any contract's members: the thresholds are the list's.
    let list = component_views(
        accounts,
        &params,
        None,
        SettlementTarget::FixedMixReference,
        now,
    );
    let (components, items, settlement_target) = match front.and_then(|i| contracts.get(i)) {
        Some(k) => (k.components.clone(), k.items.clone(), k.target),
        None => {
            let items = item_views(&Frozen::board(board), &list, None);
            (list.clone(), items, SettlementTarget::FixedMixReference)
        }
    };
    let mut weighted: Vec<u64> = list
        .iter()
        .flat_map(|c| &c.sources)
        .filter(|s| !s.duplicate)
        .map(|s| s.delegation)
        .collect();
    let thresholds = Thresholds {
        min_post: thresholds::min_post(
            params.floor_post,
            params.alpha_bps,
            board.total_delegation,
            weighted.len(),
        ),
        activation: thresholds::activation_threshold(
            params.floor_activate,
            params.beta_bps,
            &mut weighted,
        ),
    };
    BoardView {
        product: product_symbol(&board.product_id),
        params,
        max_liability_bps: params.max_liability_bps(),
        thresholds,
        weighted_sources: weighted.len(),
        front,
        settlement_target,
        components,
        items,
        contracts,
    }
}

/// The items and the frozen methodology a reading uses: the contract's, frozen at listing, or
/// the board's current ones.
struct Frozen<'a> {
    item_count: usize,
    bucket_ids: &'a [[u8; 32]; HEADS_MAX_ITEMS],
    weights: &'a [u16; HEADS_MAX_ITEMS],
    mix: &'a [u16; HEADS_MAX_COMPONENTS],
    basis: &'a [u8; HEADS_MAX_ITEMS],
}

impl<'a> Frozen<'a> {
    fn board(b: &'a HeadsBoard) -> Self {
        Self {
            item_count: usize::from(b.item_count).min(HEADS_MAX_ITEMS),
            bucket_ids: &b.item_bucket_ids,
            weights: &b.item_weight_bps,
            mix: &b.item_mix_bps,
            basis: &b.item_mix_basis,
        }
    }

    fn contract(k: &'a HeadsContract) -> Self {
        Self {
            item_count: usize::from(k.item_count).min(HEADS_MAX_ITEMS),
            bucket_ids: &k.item_bucket_ids,
            weights: &k.item_weight_bps,
            mix: &k.item_mix_bps,
            basis: &k.item_mix_basis,
        }
    }

    fn mix(&self, item: usize) -> [u16; HEADS_CHANNELS] {
        [
            self.mix[item * 3],
            self.mix[item * 3 + 1],
            self.mix[item * 3 + 2],
        ]
    }
}

fn contract_view(
    accounts: &BoardAccounts,
    address: &Pubkey,
    k: &HeadsContract,
    now: u64,
) -> ContractView {
    let params = k.params.to_params();
    let target = settlement_target(k.settlement_target);
    let components = component_views(accounts, &params, Some(k), target, now);
    let items = item_views(&Frozen::contract(k), &components, Some(k));
    let index = index_reading(k, target, &items, &components);
    ContractView {
        address: *address,
        period: k.period,
        target,
        components,
        items,
        index,
    }
}

fn degraded(code: u8) -> Option<settle::Degraded> {
    match code {
        DEGRADED_FREEZE_CHECK_FAILED => Some(settle::Degraded::FreezeCheckFailed),
        DEGRADED_TOO_FEW_ORIGINS => Some(settle::Degraded::TooFewOrigins),
        DEGRADED_TOO_LITTLE_WEIGHT => Some(settle::Degraded::TooLittleWeight),
        _ => None,
    }
}

/// The shares of `weights` in basis points, largest remainder: they add up to exactly 10,000
/// while any weight is positive (ties to the lower index), all zero otherwise.
pub fn shares_bps(weights: &[u64]) -> Vec<u16> {
    let total: u128 = weights.iter().map(|w| u128::from(*w)).sum();
    if total == 0 {
        return vec![0; weights.len()];
    }
    let scaled: Vec<(u128, u128)> = weights
        .iter()
        .map(|w| {
            let x = u128::from(*w) * u128::from(BPS);
            (x / total, x % total)
        })
        .collect();
    let mut shares: Vec<u16> = scaled.iter().map(|(q, _)| *q as u16).collect();
    let given: u128 = scaled.iter().map(|(q, _)| *q).sum();
    let mut order: Vec<usize> = (0..weights.len()).filter(|i| weights[*i] > 0).collect();
    order.sort_by(|a, b| scaled[*b].1.cmp(&scaled[*a].1).then(a.cmp(b)));
    for i in order.into_iter().take((u128::from(BPS) - given) as usize) {
        shares[i] += 1;
    }
    shares
}

fn component_views(
    accounts: &BoardAccounts,
    params: &Params,
    contract: Option<&HeadsContract>,
    target: SettlementTarget,
    now: u64,
) -> Vec<ComponentView> {
    let count = match contract {
        Some(k) => k.component_count,
        None => accounts.board.component_count,
    };
    (0..usize::from(count).min(HEADS_MAX_COMPONENTS))
        .map(|c| component_view(accounts, params, contract, target, c, now))
        .collect()
}

/// A source's level now, as the program has it: its latest print by source time (`value`, with
/// its LTA terms), unless an LTA price past its own reset (then none). The 32-print ring, in
/// finalization order, is display only: replays of an old observation that fill it move no level
/// (round 4 N2).
pub fn level_now(channel: Channel, head: &Head, now: u64) -> Option<u64> {
    let lapsed =
        channel == Channel::Lta && head.lta_next_reset_ts != 0 && now >= head.lta_next_reset_ts;
    (head.value > 0 && !lapsed).then_some(head.value)
}

/// Whether `head` joined contract `k`'s accumulation window (went live after the listing and
/// before the freeze): with the listed sources, the members `SettleComponent` must record before
/// it settles the component, retired or not.
pub fn joined(k: &HeadsContract, head: &Head) -> bool {
    let slot = usize::from(k.ring_slot);
    head.capture_periods.get(slot) == Some(&k.period)
        && head.capture_flags[slot] & CAPTURE_JOINED != 0
}

/// Whether `head` is a member of contract `k`, a source its settlement records: listed by it, or
/// joined its window (and did not leave it before the freeze), evicted or not.
pub fn member(k: &HeadsContract, head: &Head) -> bool {
    let listed = usize::from(k.listed_count).min(k.listed_head_ids.len());
    (0..listed)
        .any(|i| k.listed_head_ids[i] == head.head_id && k.listed_components[i] == head.component)
        || joined(k, head)
}

/// Whether `SettleComponent` must still wait before it records `head` for contract `k`, as the
/// program decides (`record_level`, `samples::waits`): a head eligible in `k` or live at its freeze
/// (the fallback reads its freeze level) with a claim open that `k` admits, or one revealed before
/// the head's slot started on `k` (`adm_open` counts both; a head untouched since the listing: any
/// claim it revealed). Either finalizes first.
pub fn level_waits(k: &HeadsContract, head: &Head) -> bool {
    let slot = usize::from(k.ring_slot);
    let ours = head.capture_periods.get(slot) == Some(&k.period);
    // Eligible in `k`: not retired by its cutoff (expiry plus the challenge window).
    let cutoff = k.expiry_ts.saturating_add(k.params.challenge_window_secs);
    let eligible = head.retired_ts == 0 || head.retired_ts > cutoff;
    let live_at_freeze = head.live_ts != 0
        && head.live_ts <= k.freeze_ts
        && (head.retired_ts == 0 || head.retired_ts > k.freeze_ts);
    let waits = if ours {
        head.adm_open[slot] > 0
    } else {
        head.finalized_reveals < head.reveals
    };
    (eligible || live_at_freeze) && waits
}

/// The opening of `head` in contract `k`: its listed opening, or its fill value when it went
/// live inside the window; `None` for a source that went live after the freeze.
fn opening(k: &HeadsContract, head: &Head) -> Option<u64> {
    let listed = usize::from(k.listed_count).min(k.listed_head_ids.len());
    if let Some(i) = (0..listed).find(|i| {
        k.listed_head_ids[*i] == head.head_id && k.listed_components[*i] == head.component
    }) {
        return Some(k.listed_openings[i]).filter(|v| *v > 0);
    }
    (head.live_ts >= k.listing_ts && head.live_ts < k.freeze_ts && head.fill_value > 0)
        .then_some(head.fill_value)
}

fn component_view(
    accounts: &BoardAccounts,
    params: &Params,
    contract: Option<&HeadsContract>,
    target: SettlementTarget,
    c: usize,
    now: u64,
) -> ComponentView {
    let board = &accounts.board;
    let (item, channel, sentence_hash) = match contract {
        Some(k) => (
            k.component_items[c],
            k.component_channels[c],
            k.component_sentence_hashes[c],
        ),
        None => (
            board.component_items[c],
            board.component_channels[c],
            board.component_sentence_hashes[c],
        ),
    };
    let channel = channel_of(channel);
    let mixes = match contract {
        Some(k) => &k.item_mix_bps,
        None => &board.item_mix_bps,
    };
    let mix: [u16; CHANNELS] = core::array::from_fn(|ch| {
        mixes
            .get(usize::from(item) * CHANNELS + ch)
            .copied()
            .unwrap_or(0)
    });
    // The component's live list (its counts), and the sources priced: against a contract its
    // members, what its settlement records (evicted ones included, non-members not: round 13
    // review, finding 3); else the live list. By head id.
    let on_list = |h: &Head| {
        usize::from(h.component) == c && h.status == HeadStatus::Live && h.evicted_ts == 0
    };
    let mut list: Vec<&Head> = accounts
        .heads
        .iter()
        .map(|(_, h)| h.as_ref())
        .filter(|h| on_list(h))
        .collect();
    list.sort_by_key(|h| h.head_id);
    let list_entries: Vec<OriginEntry> = list
        .iter()
        .map(|h| OriginEntry {
            origin: h.origin_id,
            live_at: h.live_ts,
            head_id: h.head_id,
            counts: true,
        })
        .collect();
    let origins = channels::distinct_origins(&list_entries);
    let mut live: Vec<(&Pubkey, &Head)> = accounts
        .heads
        .iter()
        .filter(|(_, h)| match contract {
            Some(k) => usize::from(h.component) == c && member(k, h),
            None => on_list(h),
        })
        .map(|(k, h)| (k, h.as_ref()))
        .collect();
    live.sort_by_key(|(_, h)| h.head_id);
    let n = live.len();
    // Past the freeze, what the contract's settlement records of each member: its levels, the
    // medians of its samples, the origins deciding them and its changes from its opening, by the
    // program's own calculation ([`level_record`]; round 14 review, N2). Before it, a provisional
    // reading on the members' latest prints ([`level_now`]).
    let records: Option<Vec<HeadsLevelRecord>> = contract
        .filter(|k| now >= k.freeze_ts)
        .map(|k| live.iter().map(|(_, h)| level_record(k, h)).collect());
    let delegation: Vec<u64> = live.iter().map(|(_, h)| h.delegation).collect();
    // Raw weights: against a contract, each source's raw weight in it so far (its time-weighted
    // live delegation over the window, what settlement weighs); without one, its delegation now.
    let raw: Vec<u64> = live
        .iter()
        .map(|(_, h)| {
            contract
                .and_then(|k| head_raw_weight(h, k, now))
                .unwrap_or(h.delegation)
        })
        .collect();
    let openings: Vec<Option<u64>> = live
        .iter()
        .map(|(_, h)| contract.and_then(|k| opening(k, h)))
        .collect();
    let levels: Vec<Option<u64>> = (0..n)
        .map(|s| match &records {
            Some(r) => r[s].level,
            None => level_now(channel, live[s].1, now),
        })
        .collect();
    let changes: Vec<Option<i64>> = (0..n)
        .map(|s| match &records {
            Some(r) => r[s].change_ppm,
            None => openings[s]
                .zip(levels[s])
                .and_then(|(o, l)| settle::change_ppm(o, l)),
        })
        .collect();
    let origin_of = |s: usize| {
        records
            .as_ref()
            .map_or(live[s].1.origin_id, |r| r[s].origin)
    };
    // The freeze check: as recorded, else past the freeze its coverage as settlement checks it:
    // the board's capture, or its members' coverage then (round 13 review, finding 3).
    let flags = contract.map_or(0, |k| k.component_flags[c]);
    let checked = flags & COMPONENT_FREEZE_CHECKED != 0;
    let at_freeze = contract.and_then(|k| {
        let slot = usize::from(k.ring_slot);
        (!checked && now >= k.freeze_ts && board.contract_periods[slot] == k.period).then(|| {
            if board.contract_flags[slot] & RING_FREEZE_CAPTURED != 0 {
                board.contract_freeze_origins[slot * HEADS_MAX_COMPONENTS + c]
            } else {
                board.member_coverage(slot, c, k.freeze_ts)
            }
        })
    });
    let freeze_passed = if checked {
        flags & COMPONENT_FREEZE_PASSED != 0
    } else {
        at_freeze.is_none_or(|o| channels::origins_pass(usize::from(o), params.min_live_sources))
    };
    let input =
        |s: usize, counts: bool, change_ppm: Option<i64>, origin: [u8; 32]| settle::SourceInput {
            live: counts,
            change_ppm,
            weight: raw[s],
            origin,
            live_at: live[s].1.live_ts,
            head_id: live[s].1.head_id,
        };
    // The settlement rule and the duplicates it finds.
    let (duplicate, settlement) = match &records {
        // Settlement's own (`settle::settle_recorded`): the members eligible at the cutoff K on
        // their levels; short of that, its last valid value, the members live at the freeze on
        // their freeze levels.
        Some(r) => {
            let counted: Vec<usize> = (0..n).filter(|&s| r[s].eligible).collect();
            let inputs: Vec<settle::SourceInput> = counted
                .iter()
                .map(|&s| input(s, true, r[s].change_ppm, r[s].origin))
                .collect();
            let then: Vec<settle::SourceInput> = (0..n)
                .filter(|&s| r[s].freeze_change_ppm.is_some())
                .map(|s| input(s, true, r[s].freeze_change_ppm, r[s].freeze_origin))
                .collect();
            let (settlement, _, dup) =
                settle::settle_recorded(params, &inputs, &then, freeze_passed);
            let mut duplicate = vec![false; n];
            for (&s, d) in counted.iter().zip(dup) {
                duplicate[s] = d;
            }
            (duplicate, Some(settlement))
        }
        // Provisional: the rule on the latest levels, a member retired by the cutoff counting
        // nowhere, falling to the listing value (0).
        None => {
            let entries: Vec<OriginEntry> = live
                .iter()
                .map(|(_, h)| OriginEntry {
                    origin: h.origin_id,
                    live_at: h.live_ts,
                    head_id: h.head_id,
                    counts: true,
                })
                .collect();
            let settlement = contract.map(|k| {
                let cutoff = k.expiry_ts.saturating_add(k.params.challenge_window_secs);
                let inputs: Vec<settle::SourceInput> = (0..n)
                    .map(|s| {
                        let h = live[s].1;
                        let eligible = h.retired_ts == 0 || h.retired_ts > cutoff;
                        input(s, eligible, changes[s], h.origin_id)
                    })
                    .collect();
                settle::settle_component(params, &inputs, 0, freeze_passed)
            });
            (channels::duplicate_flags(&entries), settlement)
        }
    };
    // A duplicate carries no weight.
    let weights: Vec<u64> = raw
        .iter()
        .zip(&duplicate)
        .map(|(&w, &dup)| if dup { 0 } else { w })
        .collect();
    let settles = channels::settles(target, channel, &mix);
    let shares = shares_bps(&weights);
    let sources = live
        .iter()
        .enumerate()
        .map(|(s, (key, h))| SourceView {
            head_id: h.head_id,
            head: **key,
            origin: origin_of(s),
            live_at: h.live_ts,
            duplicate: duplicate[s],
            delegation: delegation[s],
            weight: weights[s],
            share_bps: shares[s],
            level: levels[s],
            opening: openings[s],
            change_ppm: changes[s],
        })
        .collect();
    // Before the freeze, a component short of origins is short now (it would fail that check
    // unless sources arrive), not failed.
    let short = !checked
        && at_freeze.is_none()
        && records.is_none()
        && !channels::origins_pass(origins, params.min_live_sources);
    let (reading, degraded_change_ppm) = match (contract, settlement) {
        (Some(k), _) if flags & COMPONENT_SETTLED != 0 => {
            let live = flags & COMPONENT_LIVE != 0;
            (
                Reading::Settled {
                    change_ppm: k.component_change_ppm[c],
                    live,
                    reason: degraded(k.component_degraded[c]),
                },
                if live { 0 } else { k.component_change_ppm[c] },
            )
        }
        (Some(_), _) if short => (
            Reading::Degraded {
                reason: settle::Degraded::TooFewOrigins,
            },
            0,
        ),
        (
            Some(_),
            Some(settle::ComponentSettlement::Live {
                change_ppm,
                eligible_sources,
                duplicate_sources,
                weight,
            }),
        ) => (
            Reading::Live {
                change_ppm,
                eligible: eligible_sources,
                duplicates: duplicate_sources,
                weight,
            },
            0,
        ),
        (Some(_), Some(settle::ComponentSettlement::LastValid { change_ppm, reason })) => {
            (Reading::Degraded { reason }, change_ppm)
        }
        _ => (Reading::NoContract, 0),
    };
    ComponentView {
        index: c as u8,
        item,
        channel,
        sentence_hash,
        settles,
        live_sources: list.len() as u16,
        live_origins: origins as u16,
        sources,
        reading,
        degraded_change_ppm,
    }
}

/// What `SettleComponent` records of member `head` of contract `k` ([`HeadsLevelRecord`], the
/// program's `heads_level_record`) were it recorded now: from the head's slot on `k`, or, for a
/// head not synced since the listing, from the slot its sync starts there (its latest print,
/// `samples::restart`). Against its opening in `k` (0, so no change, for none).
pub fn level_record(k: &HeadsContract, head: &Head) -> HeadsLevelRecord {
    let slot = usize::from(k.ring_slot);
    let opening = opening(k, head).unwrap_or(0);
    if head.capture_periods.get(slot) == Some(&k.period) {
        return heads_level_record(head, k, opening);
    }
    let mut synced = head.clone();
    let window = time::settlement_window_start(k.freeze_ts, k.params.settlement_business_days);
    heads_rebuild_slot(
        &mut synced,
        slot,
        &heads_sample_instants(window, k.freeze_ts, k.expiry_ts),
    );
    heads_level_record(&synced, k, opening)
}

fn item_views(
    frozen: &Frozen,
    components: &[ComponentView],
    contract: Option<&HeadsContract>,
) -> Vec<ItemView> {
    (0..frozen.item_count)
        .map(|i| {
            let mix = frozen.mix(i);
            let basis = MixBasis::from_code(frozen.basis[i]).unwrap_or(MixBasis::None);
            let component = |channel: Channel| components.get(i * HEADS_CHANNELS + channel.index());
            let recorded = contract.filter(|k| k.item_reference_flags[i] & REFERENCE_COMPUTED != 0);
            let reference = match recorded {
                Some(k) => {
                    let flags = k.item_reference_flags[i];
                    if flags & REFERENCE_AVAILABLE != 0 {
                        let mask = k.item_degraded_channels[i];
                        ReferenceView::Available {
                            change_ppm: k.item_reference_ppm[i],
                            assumed: flags & REFERENCE_ASSUMED != 0,
                            degraded: Channel::ALL.map(|c| mask & (1 << c.code()) != 0),
                            recorded: true,
                        }
                    } else {
                        ReferenceView::Unavailable(ReferenceGap::Reference(
                            Unavailable::from_code(k.item_reference_reason[i])
                                .unwrap_or(Unavailable::InvalidMix),
                        ))
                    }
                }
                None => {
                    // Each channel at its reading now: live, settled, or on its degraded path
                    // at its last valid value then.
                    let readings =
                        Channel::ALL.map(|channel| match component(channel).map(|v| &v.reading) {
                            Some(Reading::Live { change_ppm, .. }) => ChannelReading {
                                change_ppm: *change_ppm,
                                degraded: false,
                            },
                            Some(Reading::Settled {
                                change_ppm, live, ..
                            }) => ChannelReading {
                                change_ppm: *change_ppm,
                                degraded: !*live,
                            },
                            Some(Reading::Degraded { .. }) => ChannelReading {
                                change_ppm: component(channel).map_or(0, |v| v.degraded_change_ppm),
                                degraded: true,
                            },
                            _ => ChannelReading {
                                change_ppm: 0,
                                degraded: true,
                            },
                        });
                    match (contract, channels::reference(&mix, basis, &readings)) {
                        (_, Reference::Unavailable(u)) => {
                            ReferenceView::Unavailable(ReferenceGap::Reference(u))
                        }
                        // Without a contract the channels have nothing to measure against.
                        (None, Reference::Available { .. }) => {
                            ReferenceView::Unavailable(ReferenceGap::NoContract)
                        }
                        (
                            Some(_),
                            Reference::Available {
                                change_ppm,
                                assumed,
                                degraded,
                            },
                        ) => ReferenceView::Available {
                            change_ppm,
                            assumed,
                            degraded,
                            recorded: false,
                        },
                    }
                }
            };
            ItemView {
                index: i as u8,
                bucket_id: frozen.bucket_ids[i],
                basket_weight_bps: frozen.weights[i],
                mix_bps: mix,
                mix_basis: basis,
                reference,
            }
        })
        .collect()
}

fn index_reading(
    k: &HeadsContract,
    target: SettlementTarget,
    items: &[ItemView],
    components: &[ComponentView],
) -> IndexReading {
    if k.status == HeadsContractStatus::Settled {
        return IndexReading::Settled {
            change_ppm: k.index_change_ppm,
            value: k.value,
        };
    }
    // The settling components on their degraded path (at their last valid value).
    let mut degraded = Vec::new();
    for item in items {
        for channel in Channel::ALL {
            let c = usize::from(item.index) * HEADS_CHANNELS + channel.index();
            let live = matches!(
                components.get(c).map(|v| &v.reading),
                Some(Reading::Live { .. }) | Some(Reading::Settled { live: true, .. })
            );
            if !live && channels::settles(target, channel, &item.mix_bps) {
                degraded.push(c as u8);
            }
        }
    }
    let settlements: Vec<ItemSettlement> = items
        .iter()
        .map(|item| {
            let c = usize::from(item.index) * HEADS_CHANNELS + Channel::Contract.index();
            let contract_change_ppm = match components.get(c).map(|v| &v.reading) {
                Some(Reading::Live { change_ppm, .. })
                | Some(Reading::Settled { change_ppm, .. }) => *change_ppm,
                Some(Reading::Degraded { .. }) => components[c].degraded_change_ppm,
                _ => 0,
            };
            ItemSettlement {
                basket_weight_bps: item.basket_weight_bps,
                contract_change_ppm,
                reference: item.reference.as_reference(),
            }
        })
        .collect();
    match channels::settlement_change_ppm(target, &settlements) {
        Ok(change_ppm) => IndexReading::Indicative {
            change_ppm,
            value: settle::settlement_value(k.base, change_ppm),
            degraded,
        },
        Err(refusal) => IndexReading::Unavailable(IndexGap::Refused(refusal)),
    }
}

// ---------------------------------------------------------------------------------------------
// Heads and delegations
// ---------------------------------------------------------------------------------------------

/// A head's live-gated delegation integral at `t` (no earlier than its last accrual): what it
/// held while live, in atom-seconds. `None` for a time before the last accrual.
pub fn head_integral_at(head: &Head, t: u64) -> Option<u128> {
    if t < head.accrual_since_ts {
        return None;
    }
    if head.live_ts == 0 {
        return Some(head.accrual_integral);
    }
    let live_to = if head.retired_ts >= head.live_ts && head.retired_ts != 0 {
        head.retired_ts
    } else {
        u64::MAX
    };
    let from = head.accrual_since_ts.max(head.live_ts);
    Some(head.accrual_integral + time::integral(head.delegation, from, t.min(live_to)))
}

fn captured(head: &Head, k: &HeadsContract, flag: u8) -> Option<u128> {
    let slot = usize::from(k.ring_slot);
    (slot < HEADS_CONTRACT_RING
        && head.capture_periods[slot] == k.period
        && head.capture_flags[slot] & flag != 0)
        .then(|| {
            if flag == CAPTURE_LISTING {
                head.capture_listing[slot]
            } else {
                head.capture_freeze[slot]
            }
        })
}

/// A head's raw weight in contract `k` so far: its time-weighted live delegation over the
/// accumulation window up to `now` (or the freeze), spec §3. `None` where its integral at the
/// listing is not known (it was neither captured nor accrued since).
pub fn head_raw_weight(head: &Head, k: &HeadsContract, now: u64) -> Option<u64> {
    let end = now.min(k.freeze_ts);
    if end <= k.listing_ts {
        return Some(0);
    }
    let at_listing =
        captured(head, k, CAPTURE_LISTING).or_else(|| head_integral_at(head, k.listing_ts))?;
    let at_end = match captured(head, k, CAPTURE_FREEZE) {
        Some(v) if end == k.freeze_ts => v,
        _ => head_integral_at(head, end)?,
    };
    Some(time::average(
        at_end.saturating_sub(at_listing),
        end - k.listing_ts,
    ))
}

/// One delegation's money now (spec §5), as the next `SyncDelegation` would see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    /// Delegated (active and fresh).
    pub amount: u64,
    /// Undelegated and coming back: after `cooldown_ends_at`, at the next sync.
    pub cooling: u64,
    /// Undelegated and held until its exposure clears: what the head's remaining strikes can
    /// still take while a contract it weighed in or a case opened while it was active is open.
    pub escrow: u64,
    /// When the cooldown of the last undelegation ends (`None` with nothing held).
    pub cooldown_ends_at: Option<u64>,
    /// Delegator rewards to claim (USDC atoms): accrued, plus what the reward index added since
    /// while the head is a live source that carries weight.
    pub rewards: u64,
}

/// The head's delegator reward index now (see `HeadsBoard::live_reward_offsets`).
pub fn head_reward_index(board: &HeadsBoard, head: &Head) -> u128 {
    let live = usize::from(board.live_count).min(board.live_head_ids.len());
    match board.live_head_ids[..live]
        .iter()
        .position(|id| *id == head.head_id)
    {
        Some(entry) if head.status == HeadStatus::Live => {
            if board.live_flags[entry] & crate::state::heads::LIVE_DUPLICATE != 0 {
                board.live_reward_offsets[entry]
            } else {
                board
                    .reward_index
                    .saturating_sub(board.live_reward_offsets[entry])
            }
        }
        _ => head.reward_index_frozen,
    }
}

/// `d`'s position on `head` of `board`.
pub fn position(board: &HeadsBoard, head: &Head, d: &Delegation) -> Position {
    let params = board.params.to_params();
    let amount = d.active.saturating_add(d.fresh);
    // The held escrow, as the program keeps it (`Delegation::held_escrow`).
    let escrow = d.held_escrow(&params, board, head).min(d.held);
    // The head's own reward index, as `processor::heads::transitions::head_reward_index` reads
    // it: its live entry's while Live (advancing only while it is not a duplicate), frozen once it
    // left the list.
    let pending = rewards::earned(amount, d.reward_index, head_reward_index(board, head));
    Position {
        amount,
        cooling: d.held - escrow,
        escrow,
        cooldown_ends_at: (d.held > 0).then_some(d.cooldown_end_ts),
        rewards: d.rewards_accrued.saturating_add(pending),
    }
}

#[cfg(test)]
mod tests;
