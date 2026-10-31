//! Oracle v2, "heads on pikes" (`docs/heads.md`): the family's account layouts.
//!
//! They compile in every build, so consumers that link this crate without the `heads` feature
//! (the API, petri, ops, the chain) read the same layouts the program writes; only tag 210's
//! routing is feature-gated.
//!
//! Every account starts with an eight-byte discriminator, a version byte and its canonical PDA
//! bump, and every PDA starts with `ameba-oracle-v1`. Each layout has a fixed wire length
//! (`WIRE_LEN`) inside a larger allocation (`LEN`): the bytes past the wire are reserved and
//! must be zero, which the codec checks on every load. `HeadsWeights` is the one variable
//! account: a fixed header followed by three record sections that grow by realloc.
//!
//! The capacities below (components, live sources, contract ring, prints, strikes, name bytes)
//! are structural sizes of the accounts, not policy: a full account is a structural error
//! (`HeadsCapacityExhausted`), never a rule anyone can game. They are chosen generously.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{account_info::AccountInfo, pubkey::Pubkey};

#[cfg(test)]
use crate::fixed_codec::ReferenceBorsh;
use crate::fixed_codec::{
    decode_flat_fields, fixed_state_deserialize_flat, invalid_fixed_borsh, FixedCursor, FixedField,
    FixedStateDecode, FixedStateEncode, FixedWriter, FlatCodecDescriptor,
};
use crate::{
    compact_error::CompactAccountInfo, constants::CURRENT_STATE_NAMESPACE_SEED, error::OracleError,
    ProgramError,
};

/// The SBF half of `fixed_state_deserialize_flat!` for the layouts too large for a 4 KiB stack
/// frame (`HEAP_LAYOUTS` in `gen_layouts.py`): the flat descriptor, its compile-time checks and the
/// in-place encoder. Their by-value decoders and `Default` (which build the value on the stack)
/// exist on the host only; on SBF they load through [`HeadsAccount::unpack`] onto the heap and are
/// built with [`zeroed_box`].
#[cfg(target_os = "solana")]
macro_rules! heads_heap_codec {
    ($type:ident, $serialized_len:expr, flat { $($($leaf:ident).+: $leaf_type:ty),+ $(,)? }) => {
        const _: () = {
            assert!(
                $serialized_len
                    == 0usize $(+ <$leaf_type as $crate::fixed_codec::FlatFieldType>::WIRE_LEN)+
            );
            $(
                assert!(
                    <$leaf_type as $crate::fixed_codec::FlatFieldType>::WIRE_LEN
                        == core::mem::size_of::<$leaf_type>()
                );
            )+
        };

        impl $type {
            const __FLAT_CODEC_FIELDS: &'static [$crate::fixed_codec::FlatFieldDescriptor] = &[
                $($crate::fixed_codec::FlatFieldDescriptor::new(
                    core::mem::offset_of!($type, $($leaf).+),
                    <$leaf_type as $crate::fixed_codec::FlatFieldType>::WIRE_LEN,
                    <$leaf_type as $crate::fixed_codec::FlatFieldType>::BYTE_DOMAIN,
                ),)+
            ];

            const __FLAT_CODEC: $crate::fixed_codec::FlatCodecDescriptor =
                $crate::fixed_codec::FlatCodecDescriptor::new(
                    core::mem::size_of::<$type>(),
                    $serialized_len,
                    Self::__FLAT_CODEC_FIELDS,
                );
        }

        impl FixedStateEncode for $type {
            #[inline(always)]
            fn maximum_encoded_len(&self) -> usize {
                $serialized_len
            }

            #[inline(never)]
            fn encode_fixed(&self, data: &mut [u8]) {
                // SAFETY: the descriptor admits only fields with a byte-identical native
                // representation and checks both allocation bounds.
                let encoded = unsafe {
                    $crate::fixed_codec::encode_flat_fields(
                        &Self::__FLAT_CODEC,
                        (self as *const Self).cast::<u8>(),
                        data.as_mut_ptr(),
                        data.len(),
                    )
                };
                debug_assert!(encoded);
            }
        }
    };
}

// The structs and their codecs, generated from one field table by `gen_layouts.py` so every
// declared wire length is the exact sum of its fields. Included here (not a child module) so the
// account impls below reach the codecs' private descriptors.
include!("layouts.rs");

#[cfg(test)]
mod tests;

mod settlement;
pub use settlement::{
    heads_settlement_digest, heads_settlement_record, HEADS_SETTLEMENT_DIGEST_DOMAIN,
};

/// `[ameba-oracle-v1, heads-config]`: the singleton.
pub const HEADS_CONFIG_SEED: &[u8] = b"heads-config";
/// `[ameba-oracle-v1, heads-authority]`: owns the AMBA vault; no data.
pub const HEADS_AUTHORITY_SEED: &[u8] = b"heads-authority";
/// `[ameba-oracle-v1, heads-board, product_id]`.
pub const HEADS_BOARD_SEED: &[u8] = b"heads-board";
/// `[ameba-oracle-v1, heads-stake, owner]`.
pub const HEADS_STAKE_SEED: &[u8] = b"heads-stake";
/// `[ameba-oracle-v1, head, board, head_id (u32 LE)]`.
pub const HEAD_SEED: &[u8] = b"head";
/// `[ameba-oracle-v1, head-delegation, head, owner]`.
pub const HEAD_DELEGATION_SEED: &[u8] = b"head-delegation";
/// `[ameba-oracle-v1, head-claim, head, seq (u32 LE)]`.
pub const HEAD_CLAIM_SEED: &[u8] = b"head-claim";
/// `[ameba-oracle-v1, head-challenge, target, challenger]`; the target is the claim, or the head
/// for a source challenge.
pub const HEAD_CHALLENGE_SEED: &[u8] = b"head-challenge";
/// `[ameba-oracle-v1, heads-contract, period]`.
pub const HEADS_CONTRACT_SEED: &[u8] = b"heads-contract";
/// `[ameba-oracle-v1, heads-weights, contract, component (u8)]`.
pub const HEADS_WEIGHTS_SEED: &[u8] = b"heads-weights";
/// `[ameba-oracle-v1, heads-epoch, board, month_id (u32 LE)]`.
pub const HEADS_EPOCH_SEED: &[u8] = b"heads-epoch";

/// Items (basket members) per board (structural capacity).
pub const HEADS_MAX_ITEMS: usize = 8;
/// Channels an item's price is observed in: spot, contract, LTA (`HeadsChannel`).
pub const HEADS_CHANNELS: usize = 3;
/// Components (`(item, channel)` pairs) per board (structural capacity: three per item).
pub const HEADS_MAX_COMPONENTS: usize = HEADS_MAX_ITEMS * HEADS_CHANNELS;
/// Entries of a board's live-source list (structural capacity).
pub const HEADS_MAX_LIVE_SOURCES: usize = 96;
/// Heads one `SettleComponent` call records at most: its completing call, which also settles the
/// largest record (192 members eligible, the fallback's median included), stays well under the
/// compute limit on the production artifact (round 12 and 13 reviews, CU; `heads_claims`
/// benchmarks). Callers split to it.
pub const HEADS_SETTLE_MAX_BATCH: usize = 8;
/// Open contracts per board, and the per-contract capture slots of heads and delegations
/// (structural capacity: the rolling ladder keeps three maturities open).
pub const HEADS_CONTRACT_RING: usize = 4;
/// Accepted prints a head keeps (a ring; structural capacity).
pub const HEAD_PRINT_CAPACITY: usize = 32;
/// Strikes a head can record: a board's `max_strikes` is at most this (structural capacity).
pub const HEAD_STRIKE_CAPACITY: usize = 8;
/// UTF-8 bytes of a head's name (structural capacity).
pub const HEAD_NAME_MAX_BYTES: usize = 64;

/// Capture flags (`capture_flags` of heads and delegations).
pub const CAPTURE_LISTING: u8 = 1;
pub const CAPTURE_FREEZE: u8 = 2;
/// The head went live inside the contract's accumulation window: one of its members
/// (`HeadsBoard::contract_joined`); heads only. (Bits 4 and 8 were the expiry and level
/// captures that the contract samples replaced on 2026-10-30.)
pub const CAPTURE_JOINED: u8 = 16;
/// The head is recorded in the contract's weights (`SettleComponent`): each source once, checked
/// on the head, so appending one reads no record (round 3 R5); heads only.
pub const CAPTURE_RECORDED: u8 = 32;
/// Round 6 marked a head the listing left out with an expired LTA price (bit 64) so a renewal
/// could join it; renewal joins are gone (round 6 review, finding 2), nothing sets or reads it
/// now, and a head that carries it still loads.
pub const CAPTURE_RETIRED_EXCLUDED: u8 = 64;
/// The flags only heads carry.
/// The head is a member of the contract (listed by it, or joined): its delegators' stake weighs
/// in it, and only then (round 7 review, B2); heads only.
pub const CAPTURE_MEMBER: u8 = 128;
const HEAD_CAPTURES: u8 =
    CAPTURE_JOINED | CAPTURE_RECORDED | CAPTURE_RETIRED_EXCLUDED | CAPTURE_MEMBER;
/// Samples a head keeps per contract-ring slot (`Head::sample_values`): five in the freeze
/// level's window, five in the settlement window.
pub const HEAD_SLOT_SAMPLES: usize = 10;
const CAPTURE_MASK: u8 = CAPTURE_LISTING | CAPTURE_FREEZE;

/// Board contract-ring slot states (`contract_status`).
pub const RING_EMPTY: u8 = 0;
pub const RING_ACCUMULATING: u8 = 1;
pub const RING_COLLECTING: u8 = 2;
pub const RING_SETTLED: u8 = 3;
/// Board contract-ring flags (`contract_flags`): the components' origins at freeze are captured
/// in `contract_freeze_origins`.
pub const RING_FREEZE_CAPTURED: u8 = 1;

/// Live-source flags (`live_flags` of boards, `listed_flags` of contracts).
/// An earlier live source of the same component has the same origin: the source carries no
/// weight, does not count toward `min_live_sources`, and its delegation earns no delegator share.
pub const LIVE_DUPLICATE: u8 = 1;
/// A live source's membership of ring slot `i`'s contract (board `live_flags` only, bit
/// `LIVE_MEMBER << i`): listed by it, or gone live inside its window. Only members count toward
/// its freeze coverage (`HeadsBoard::member_coverage`, round 7 review, B1).
pub const LIVE_MEMBER: u8 = 2;
/// Every flag a board's live entry can carry.
const LIVE_FLAGS: u8 = LIVE_DUPLICATE | (0b1111 * LIVE_MEMBER);

/// The index of item `item`'s component on `channel`: the mold's order, three per item.
pub const fn heads_component_index(item: u8, channel: HeadsChannel) -> usize {
    item as usize * HEADS_CHANNELS + channel as usize
}

/// Per-item reference flags (`item_reference_flags`).
pub const REFERENCE_COMPUTED: u8 = 1;
/// The fixed-mix reference is available (`item_reference_ppm` holds it).
pub const REFERENCE_AVAILABLE: u8 = 2;
/// Its mix is an assumed scenario: it settles a fixed-mix contract, flagged as assumed.
pub const REFERENCE_ASSUMED: u8 = 4;
const REFERENCE_MASK: u8 = 7;
/// Why a reference is unavailable (`item_reference_reason`, `channels::Unavailable::code`).
/// Codes 1 (a channel short of origins) and 3 (a channel degraded) are retired since
/// 2026-10-29: such a channel enters the reference at its last valid value
/// (`item_degraded_channels`); they are never reused.
pub const REFERENCE_REASON_NONE: u8 = 0;
/// The item's mix basis is none.
pub const REFERENCE_REASON_MIX_BASIS_NONE: u8 = 2;
/// The mix does not add up to 10,000 (a listing never freezes one).
pub const REFERENCE_REASON_INVALID_MIX: u8 = 4;
/// The channel bits of `item_degraded_channels` (`1 << channel`).
const DEGRADED_CHANNELS_MASK: u8 = (1 << HEADS_CHANNELS) - 1;

/// Source case states (`source_case_status`).
pub const SOURCE_CASE_NONE: u8 = 0;
pub const SOURCE_CASE_OPEN: u8 = 1;
pub const SOURCE_CASE_STOOD: u8 = 2;
pub const SOURCE_CASE_FELL: u8 = 3;
/// The ruling deadline of a board that sets none (`HeadsBoard::ruling_deadline_secs`): seven days
/// after a claim's or a case's first window, seven times the default window. Rulings come from
/// the council; past this an unruled challenge would hold bonds, escrow and its head's later
/// claims for ever, so it times out instead (its bond back), and the case resolves on the
/// rulings it has (re-review #13, early review B2, B5, B6).
pub const DEFAULT_RULING_DEADLINE_SECS: u64 = 7 * 86_400;

/// Contract component flags (`component_flags`): the settlement record.
pub const COMPONENT_SETTLED: u8 = 1;
/// Settled on live data.
pub const COMPONENT_LIVE: u8 = 2;
/// Degraded rule 2: settled at the last valid value.
pub const COMPONENT_LAST_VALID: u8 = 16;
/// The change at freeze is recorded.
pub const COMPONENT_FREEZE_RECORDED: u8 = 32;
/// The origins at freeze are recorded (`component_freeze_origins`).
pub const COMPONENT_FREEZE_CHECKED: u8 = 64;
/// They passed `channels::origins_pass`; a component that passed at listing and failed here
/// settles on its degraded path (`settle::Degraded::FreezeCheckFailed`).
pub const COMPONENT_FREEZE_PASSED: u8 = 128;

/// Why a component settled on its last valid value (`component_degraded`, `settle::Degraded`).
pub const DEGRADED_NONE: u8 = 0;
pub const DEGRADED_FREEZE_CHECK_FAILED: u8 = 1;
pub const DEGRADED_TOO_FEW_ORIGINS: u8 = 2;
pub const DEGRADED_TOO_LITTLE_WEIGHT: u8 = 3;

/// `HeadsWeightSource::flags`.
pub const WEIGHT_SOURCE_LISTED: u8 = 1;
/// Went live inside the accumulation window: its opening is its fill value.
pub const WEIGHT_SOURCE_JOINED: u8 = 2;
pub const WEIGHT_SOURCE_LEVEL: u8 = 8;
pub const WEIGHT_SOURCE_FREEZE_LEVEL: u8 = 16;
pub const WEIGHT_SOURCE_ELIGIBLE: u8 = 32;
/// Retired by the contract's eligibility cutoff (its expiry plus the challenge window, when
/// settlement opens): its weight stays in the record, settlement drops it.
pub const WEIGHT_SOURCE_RETIRED: u8 = 64;
/// Its origin repeats an earlier source's: no weight, not counted as an origin.
pub const WEIGHT_SOURCE_DUPLICATE: u8 = 128;

/// Evidence kinds a claim or challenge may reference: none, or a sealed evidence object of kind
/// 2 (opaque: an offline quote), 3 (Wayback) or 4 (authenticated record).
pub const HEADS_EVIDENCE_NONE: u8 = 0;

/// Whether `kind` may be referenced, and its commitment is consistent with it (zero exactly
/// when there is no evidence).
pub fn heads_evidence_is_valid(kind: u8, commitment: &[u8; 32]) -> bool {
    match kind {
        HEADS_EVIDENCE_NONE => crate::bytes32_is_zero(commitment),
        2 | 3 | crate::constants::ORACLE_EVIDENCE_RECORD_KIND => {
            !crate::bytes32_is_zero(commitment)
        }
        _ => false,
    }
}

/// The channel a component observes (`docs/heads.md`, long-term agreements). A purchase belongs to
/// the first that fits: LTA, then contract, then spot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum HeadsChannel {
    /// Immediate inventory outside both others.
    #[default]
    Spot = 0,
    /// Negotiated wholesale supplier to OEM/ODM purchase, not under an LTA.
    Contract = 1,
    /// Under a supply agreement of at least a year with committed volume.
    Lta = 2,
}
stable_borsh_enum!(HeadsChannel { Spot = 0, Contract = 1, Lta = 2 });

impl HeadsChannel {
    pub fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0 => Self::Spot,
            1 => Self::Contract,
            2 => Self::Lta,
            _ => return None,
        })
    }
}

/// What backs an item's channel mix.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum HeadsMixBasis {
    /// No mix: the reference is unavailable.
    #[default]
    None = 0,
    /// An assumed scenario: the reference settles a fixed-mix contract, flagged as assumed (the
    /// user, 2026-10-29).
    Assumed = 1,
    /// Matched evidence.
    Matched = 2,
}
stable_borsh_enum!(HeadsMixBasis { None = 0, Assumed = 1, Matched = 2 });

impl HeadsMixBasis {
    pub fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0 => Self::None,
            1 => Self::Assumed,
            2 => Self::Matched,
            _ => return None,
        })
    }
}

/// What a contract settles on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum HeadsSettlementTarget {
    /// Each item's Contract channel alone (the brief's Stage 1). The zero byte, so a zeroed
    /// record's value; no mold names it since 2026-10-29.
    #[default]
    ContractChannel = 0,
    /// Each item's fixed-mix reference: its three channels weighted against each other by its
    /// mix, assumed or matched (the molds' target).
    FixedMixReference = 1,
}
stable_borsh_enum!(HeadsSettlementTarget { ContractChannel = 0, FixedMixReference = 1 });

/// The ground a challenge names. Labels only: every reason has the same bond, resolution and
/// slash.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum HeadChallengeReason {
    /// The value or the evidence is wrong, or another ground.
    #[default]
    ValueOrOther = 0,
    /// The purchase belongs to another channel, or none.
    OffChannel = 1,
    /// The source republishes another live source's origin.
    DuplicateOrigin = 2,
    /// The observation was carried past its reset.
    ExpiredObservation = 3,
}
stable_borsh_enum!(HeadChallengeReason { ValueOrOther = 0, OffChannel = 1, DuplicateOrigin = 2, ExpiredObservation = 3 });

/// LTA terms are consistent: a reset after the effective date and a floor at most the ceiling
/// when both are set (0 means none).
pub fn heads_lta_terms_are_valid(
    effective_from_ts: u64,
    next_reset_ts: u64,
    floor: u64,
    ceiling: u64,
) -> bool {
    (effective_from_ts == 0 || next_reset_ts == 0 || effective_from_ts < next_reset_ts)
        && (floor == 0 || ceiling == 0 || floor <= ceiling)
}

/// An item's channel mix `[spot, contract, LTA]` in basis points: every share within one and the
/// shares adding up to 10,000.
pub fn heads_mix_is_valid(mix_bps: &[u16; HEADS_CHANNELS]) -> bool {
    mix_bps.iter().map(|m| u32::from(*m)).sum::<u32>() == 10_000
}

/// A head's status (`docs/heads.md`, "Status").
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum HeadStatus {
    /// Posted, gathering delegation; earns nothing.
    #[default]
    Open = 0,
    /// Cleared the threshold with enough delegators; fetchers may fill it.
    Active = 1,
    /// Filled: a source, weighted in contracts and earning delegator revenue.
    Live = 2,
    /// `max_strikes` reached: no new claims, delegations cool, open liabilities stay.
    Retired = 3,
    /// An Open head that did not activate within `head_ttl`.
    Expired = 4,
}
stable_borsh_enum!(HeadStatus { Open = 0, Active = 1, Live = 2, Retired = 3, Expired = 4 });

/// How a claim obtains a head's value (spec section 1).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum HeadAccessClass {
    #[default]
    PublicPage = 0,
    LoginZkTls = 1,
    OfflineQuote = 2,
    Physical = 3,
}
stable_borsh_enum!(HeadAccessClass { PublicPage = 0, LoginZkTls = 1, OfflineQuote = 2, Physical = 3 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum HeadClaimKind {
    /// Fills an Active head; anyone may file it.
    #[default]
    Fill = 0,
    /// Updates a Live source; anyone may file it (no seats or votes since 2026-10-29: challenges
    /// deal with bad claims).
    Update = 1,
}
stable_borsh_enum!(HeadClaimKind { Fill = 0, Update = 1 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum HeadClaimStatus {
    #[default]
    Committed = 0,
    /// Revealed: in its challenge window.
    Revealed = 1,
    Stood = 2,
    Fell = 3,
    /// Not revealed by its deadline; the bond was refunded.
    Expired = 4,
}
stable_borsh_enum!(HeadClaimStatus { Committed = 0, Revealed = 1, Stood = 2, Fell = 3, Expired = 4 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum HeadChallengeKind {
    /// Disputes one claim's value or evidence.
    #[default]
    Claim = 0,
    /// Disputes the source itself.
    Source = 1,
}
stable_borsh_enum!(HeadChallengeKind { Claim = 0, Source = 1 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum HeadChallengeRuling {
    #[default]
    Pending = 0,
    Won = 1,
    Lost = 2,
}
stable_borsh_enum!(HeadChallengeRuling { Pending = 0, Won = 1, Lost = 2 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum HeadsContractStatus {
    /// Listed; delegation accumulates until the freeze.
    #[default]
    Accumulating = 0,
    /// After the freeze: components settle.
    Collecting = 1,
    /// The period is Settled.
    Settled = 2,
}
stable_borsh_enum!(HeadsContractStatus { Accumulating = 0, Collecting = 1, Settled = 2 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum HeadsWeightsPhase {
    /// `SettleComponent` records the sources' levels.
    #[default]
    Recording = 0,
    /// The component settled.
    Settled = 1,
}
stable_borsh_enum!(HeadsWeightsPhase { Recording = 0, Settled = 1 });

impl HeadsParams {
    /// All zero: an unset parameter block (account defaults).
    pub const fn zero() -> Self {
        Self {
            slash_rate_bps: 0,
            max_strikes: 0,
            proposer_slash_bps: 0,
            prize_cut_bps: 0,
            base_fill_prize: 0,
            fill_prize_slope_ppb: 0,
            base_update_prize: 0,
            update_prize_slope_ppb: 0,
            claim_bond: 0,
            challenge_bond: 0,
            challenge_window_secs: 0,
            challenge_extension_secs: 0,
            head_ttl_secs: 0,
            cooldown_secs: 0,
            min_delegators: 0,
            min_live_sources: 0,
            min_settle_weight: 0,
            alpha_bps: 0,
            floor_post: 0,
            beta_bps: 0,
            floor_activate: 0,
            settlement_business_days: 0,
        }
    }

    /// The spec's defaults (`ameba_oracle_heads::Params::spec_defaults`).
    pub fn spec_defaults() -> Self {
        Self::from(&ameba_oracle_heads::Params::spec_defaults())
    }

    /// The shared crate's parameter type, which every rule takes.
    pub fn to_params(&self) -> ameba_oracle_heads::Params {
        ameba_oracle_heads::Params::from(self)
    }

    /// What a board accepts: the shared checks (`Params::validate`), and `max_strikes` within
    /// the head's strike capacity.
    pub fn validate(&self) -> bool {
        self.to_params().validate() && usize::from(self.max_strikes) <= HEAD_STRIKE_CAPACITY
    }
}

/// The common contract of the heads accounts: fixed allocation, discriminator, version and
/// canonical PDA.
pub trait HeadsAccount: Sized {
    const LEN: usize;
    /// Bytes of the encoded fields; the rest of `LEN` is reserved and zero.
    const WIRE_LEN: usize;
    const DISCRIMINATOR: [u8; 8];
    const VERSION: u8;

    /// Decodes exactly `LEN` bytes onto the heap (the larger layouts would not fit an SBF stack
    /// frame): field domains, zero reserved bytes, discriminator, version and the layout's own
    /// invariants (counts within capacity, unused slots zero).
    fn unpack(data: &[u8]) -> Result<Box<Self>, ProgramError>;

    /// Encodes into exactly `LEN` bytes, zeroing the reserved tail.
    fn pack(&self, data: &mut [u8]) -> Result<(), ProgramError>;

    /// The PDA these stored identities and bump derive, `None` if they derive no valid address.
    fn address(&self, program_id: &Pubkey) -> Option<Pubkey>;

    /// Invariants of the decoded layout itself.
    fn is_consistent(&self) -> bool;
}

/// Layouts whose all-zero value is valid: every field is an integer, a byte array, a key, a
/// `bool` or an enum with a zero variant. They can be built straight on the heap, which the
/// larger ones must be on SBF (a 10 KiB board does not fit a 4 KiB stack frame).
///
/// # Safety
/// Implement only for a type whose all-zero bit pattern is a valid value.
pub unsafe trait ZeroValid: Sized {}

// SAFETY: these layouts hold only integers, byte arrays, `Pubkey`s, `bool`s and enums whose
// discriminant 0 is a variant (`HeadStatus::Open`, `HeadAccessClass::PublicPage`,
// `HeadsChannel::Spot`, `HeadClaimKind::Fill`, `HeadClaimStatus::Committed`,
// `HeadChallengeKind::Claim`, `HeadChallengeReason::ValueOrOther`, `HeadChallengeRuling::Pending`,
// `HeadsContractStatus::Accumulating`, `HeadsSettlementTarget::ContractChannel`,
// `HeadsWeightsPhase::Recording`).
unsafe impl ZeroValid for HeadsConfig {}
unsafe impl ZeroValid for HeadsBoard {}
unsafe impl ZeroValid for HeadsStake {}
unsafe impl ZeroValid for Head {}
unsafe impl ZeroValid for Delegation {}
unsafe impl ZeroValid for HeadClaim {}
unsafe impl ZeroValid for HeadChallenge {}
unsafe impl ZeroValid for HeadsContract {}
unsafe impl ZeroValid for HeadsEpoch {}
unsafe impl ZeroValid for HeadsWeightsHeader {}

/// An all-zero value on the heap (no stack copy).
pub fn zeroed_box<T: ZeroValid>() -> Box<T> {
    let layout = std::alloc::Layout::new::<T>();
    // SAFETY: `T` is a sized struct with a nonzero size, so the layout is valid for `alloc_zeroed`;
    // a null result is handled; `T: ZeroValid` promises the zeroed bytes are a valid `T`, and the
    // pointer came from the global allocator with `T`'s layout, as `Box::from_raw` requires.
    unsafe {
        let ptr = std::alloc::alloc_zeroed(layout).cast::<T>();
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        Box::from_raw(ptr)
    }
}

fn invalid_account() -> ProgramError {
    OracleError::InvalidHeadsAccount.into()
}

/// The fields of a Head alone, before the invariants hold: the dev migration
/// (`processor::heads::migrate`) restores them on a grown old-layout Head, then loads it checked;
/// its planner (`futures-ops heads migrate`, any host build) reads old heads with it.
#[cfg(any(
    feature = "heads-migrate",
    feature = "localnet-dev-authority",
    not(target_os = "solana")
))]
pub fn decode_head_fields(data: &[u8]) -> Result<Box<Head>, ProgramError> {
    if data.len() != Head::LEN {
        return Err(invalid_account());
    }
    decode_boxed(&Head::__FLAT_CODEC, data).ok_or_else(invalid_account)
}

/// The fields of a Delegation alone, before the invariants hold: the dev migration grows a
/// version-1 delegation (640 bytes) to the current layout and restores them.
#[cfg(any(
    feature = "heads-migrate",
    feature = "localnet-dev-authority",
    not(target_os = "solana")
))]
pub fn decode_delegation_fields(data: &[u8]) -> Result<Box<Delegation>, ProgramError> {
    if data.len() != Delegation::LEN {
        return Err(invalid_account());
    }
    decode_boxed(&Delegation::__FLAT_CODEC, data).ok_or_else(invalid_account)
}

/// Decodes a flat layout straight into a heap allocation.
fn decode_boxed<T>(codec: &FlatCodecDescriptor, data: &[u8]) -> Option<Box<T>> {
    let mut boxed = Box::<T>::new_uninit();
    // SAFETY: the descriptor covers every field of `T` (checked at compile time by
    // `fixed_state_deserialize_flat!`), and `decode_flat_fields` validates the input's length,
    // bool and enum domains and zero tail before writing any field. Padding stays uninitialized
    // and is never read.
    unsafe { decode_flat_fields(codec, data, boxed.as_mut_ptr().cast::<u8>()) }.ok()?;
    // SAFETY: every field was initialized above.
    Some(unsafe { boxed.assume_init() })
}

fn derive(program_id: &Pubkey, seeds: &[&[u8]], bump: u8) -> Option<Pubkey> {
    let bump = [bump];
    let mut all: [&[u8]; 6] = [&[]; 6];
    all[0] = CURRENT_STATE_NAMESPACE_SEED;
    let count = seeds.len();
    if count + 2 > all.len() {
        return None;
    }
    all[1..=count].copy_from_slice(seeds);
    all[count + 1] = &bump;
    Pubkey::create_program_address(&all[..count + 2], program_id).ok()
}

fn find(program_id: &Pubkey, seeds: &[&[u8]]) -> (Pubkey, u8) {
    let mut all: Vec<&[u8]> = Vec::with_capacity(seeds.len() + 1);
    all.push(CURRENT_STATE_NAMESPACE_SEED);
    all.extend_from_slice(seeds);
    Pubkey::find_program_address(&all, program_id)
}

macro_rules! heads_account {
    ($type:ident, |$value:ident, $program:ident| $address:expr) => {
        impl HeadsAccount for $type {
            const LEN: usize = $type::LEN;
            const WIRE_LEN: usize = $type::WIRE_LEN;
            const DISCRIMINATOR: [u8; 8] = $type::DISCRIMINATOR;
            const VERSION: u8 = $type::VERSION;

            fn unpack(data: &[u8]) -> Result<Box<Self>, ProgramError> {
                if data.len() != Self::LEN {
                    return Err(invalid_account());
                }
                let value: Box<Self> =
                    decode_boxed(&$type::__FLAT_CODEC, data).ok_or_else(invalid_account)?;
                if value.discriminator != Self::DISCRIMINATOR
                    || value.version != Self::VERSION
                    || !value.is_consistent()
                {
                    return Err(invalid_account());
                }
                Ok(value)
            }

            fn pack(&self, data: &mut [u8]) -> Result<(), ProgramError> {
                if data.len() != Self::LEN {
                    return Err(invalid_account());
                }
                let wire = <Self as HeadsAccount>::WIRE_LEN;
                // A byte loop costs ~4 CU a byte on SBF; the memset syscall a small fixed cost.
                solana_program::program_memory::sol_memset(&mut data[wire..], 0, Self::LEN - wire);
                self.encode_fixed(data);
                Ok(())
            }

            fn address(&self, $program: &Pubkey) -> Option<Pubkey> {
                let $value = self;
                $address
            }

            fn is_consistent(&self) -> bool {
                consistent::$type(self)
            }
        }
    };
}

heads_account!(HeadsConfig, |v, p| derive(p, &[HEADS_CONFIG_SEED], v.bump));
heads_account!(HeadsBoard, |v, p| derive(
    p,
    &[HEADS_BOARD_SEED, &v.product_id],
    v.bump
));
heads_account!(HeadsStake, |v, p| derive(
    p,
    &[HEADS_STAKE_SEED, v.owner.as_ref()],
    v.bump
));
heads_account!(Head, |v, p| derive(
    p,
    &[HEAD_SEED, v.board.as_ref(), &v.head_id.to_le_bytes()],
    v.bump
));
heads_account!(Delegation, |v, p| derive(
    p,
    &[HEAD_DELEGATION_SEED, v.head.as_ref(), v.owner.as_ref()],
    v.bump
));
heads_account!(HeadClaim, |v, p| derive(
    p,
    &[HEAD_CLAIM_SEED, v.head.as_ref(), &v.seq.to_le_bytes()],
    v.bump
));
heads_account!(HeadChallenge, |v, p| derive(
    p,
    &[
        HEAD_CHALLENGE_SEED,
        v.target.as_ref(),
        v.challenger.as_ref()
    ],
    v.bump
));
heads_account!(HeadsContract, |v, p| derive(
    p,
    &[HEADS_CONTRACT_SEED, v.period.as_ref()],
    v.bump
));
heads_account!(HeadsEpoch, |v, p| derive(
    p,
    &[
        HEADS_EPOCH_SEED,
        v.board.as_ref(),
        &v.month_id.to_le_bytes()
    ],
    v.bump
));

/// The layouts' own invariants, checked on every unpack.
#[allow(non_snake_case)]
mod consistent {
    use super::*;

    fn zero_from<T: Copy + PartialEq + Default>(values: &[T], used: usize) -> bool {
        values.get(used..).is_some_and(|rest| {
            let zero = T::default();
            rest.iter().all(|v| *v == zero)
        })
    }

    fn captures(
        periods: &[Pubkey; 4],
        flags: &[u8; 4],
        listing: &[u128; 4],
        freeze: &[u128; 4],
    ) -> bool {
        (0..HEADS_CONTRACT_RING).all(|i| {
            flags[i] & !CAPTURE_MASK == 0
                && (!crate::pubkey_is_default(&periods[i])
                    || (flags[i] == 0 && listing[i] == 0 && freeze[i] == 0))
        })
    }

    pub(super) fn HeadsConfig(v: &super::HeadsConfig) -> bool {
        !crate::pubkey_is_default(&v.amba_mint)
            && !crate::pubkey_is_default(&v.amba_token_program)
            && !crate::pubkey_is_default(&v.amba_vault)
    }

    /// Items `[0, items)` with valid mixes and bases, components `[0, components)` on those
    /// items with valid channels and distinct `(item, channel)` pairs, the rest zero.
    #[allow(clippy::too_many_arguments)]
    fn items_and_components(
        items: usize,
        bucket_ids: &[[u8; 32]; HEADS_MAX_ITEMS],
        weights: &[u16; HEADS_MAX_ITEMS],
        mix: &[u16; HEADS_MAX_COMPONENTS],
        basis: &[u8; HEADS_MAX_ITEMS],
        components: usize,
        component_items: &[u8; HEADS_MAX_COMPONENTS],
        channels: &[u8; HEADS_MAX_COMPONENTS],
        sentences: &[[u8; 32]; HEADS_MAX_COMPONENTS],
    ) -> bool {
        items <= HEADS_MAX_ITEMS
            && components == items * HEADS_CHANNELS
            && zero_from(bucket_ids, items)
            && zero_from(weights, items)
            && zero_from(mix, items * HEADS_CHANNELS)
            && zero_from(basis, items)
            && (0..items).all(|i| {
                let shares = [mix[i * 3], mix[i * 3 + 1], mix[i * 3 + 2]];
                heads_mix_is_valid(&shares) && HeadsMixBasis::from_byte(basis[i]).is_some()
            })
            && zero_from(component_items, components)
            && zero_from(channels, components)
            && zero_from(sentences, components)
            && (0..components).all(|c| {
                usize::from(component_items[c]) == c / HEADS_CHANNELS
                    && usize::from(channels[c]) == c % HEADS_CHANNELS
            })
    }

    pub(super) fn HeadsBoard(v: &super::HeadsBoard) -> bool {
        let items = usize::from(v.item_count);
        let components = usize::from(v.component_count);
        let live = usize::from(v.live_count);
        items >= 1
            && v.params_version >= 1
            && items_and_components(
                items,
                &v.item_bucket_ids,
                &v.item_weight_bps,
                &v.item_mix_bps,
                &v.item_mix_basis,
                components,
                &v.component_items,
                &v.component_channels,
                &v.component_sentence_hashes,
            )
            && zero_from(&v.component_live_sources, components)
            && zero_from(&v.component_live_origins, components)
            && live <= HEADS_MAX_LIVE_SOURCES
            && zero_from(&v.live_head_ids, live)
            && zero_from(&v.live_components, live)
            && zero_from(&v.live_flags, live)
            && zero_from(&v.live_delegations, live)
            && zero_from(&v.live_values, live)
            && zero_from(&v.live_origin_ids, live)
            && v.live_components[..live]
                .iter()
                .all(|c| usize::from(*c) < components)
            && v.live_flags[..live].iter().all(|f| f & !LIVE_FLAGS == 0)
            && zero_from(&v.live_since_ts, live)
            && zero_from(&v.live_reward_offsets, live)
            && zero_from(&v.live_reset_ts, live)
            && zero_from(&v.live_delegators, live)
            && (0..HEADS_CONTRACT_RING).all(|i| {
                v.contract_status[i] <= RING_SETTLED
                    && ((v.contract_status[i] == RING_EMPTY)
                        == crate::pubkey_is_default(&v.contract_periods[i]))
                    && v.contract_flags[i] & !RING_FREEZE_CAPTURED == 0
                    && (v.contract_status[i] != RING_EMPTY
                        || (v.contract_flags[i] == 0
                            && v.contract_freeze_origins
                                [i * HEADS_MAX_COMPONENTS..(i + 1) * HEADS_MAX_COMPONENTS]
                                .iter()
                                .all(|o| *o == 0)
                            && v.contract_listing_ts[i] == 0
                            && v.contract_freeze_ts[i] == 0
                            && v.contract_expiry_ts[i] == 0
                            && v.contract_freeze_window_ts[i] == 0
                            && v.contract_gens[i] == 0
                            && v.contract_joined
                                [i * HEADS_MAX_COMPONENTS..(i + 1) * HEADS_MAX_COMPONENTS]
                                .iter()
                                .all(|o| *o == 0)))
            })
    }

    pub(super) fn HeadsStake(v: &super::HeadsStake) -> bool {
        !crate::pubkey_is_default(&v.owner)
    }

    pub(super) fn Head(v: &super::Head) -> bool {
        let name = usize::from(v.name_len);
        let strikes = usize::from(v.strike_count);
        usize::from(v.component) < HEADS_MAX_COMPONENTS
            && name <= HEAD_NAME_MAX_BYTES
            && zero_from(&v.name, name)
            && core::str::from_utf8(&v.name[..name]).is_ok()
            && strikes <= HEAD_STRIKE_CAPACITY
            && zero_from(&v.strike_opened_ts, strikes)
            && zero_from(&v.strike_ts, strikes)
            && v.source_case_status <= SOURCE_CASE_FELL
            && v.source_case_ruled_bonds <= v.source_case_bond_total
            && (v.channel == HeadsChannel::Lta
                || (v.lta_effective_from_ts == 0
                    && v.lta_next_reset_ts == 0
                    && v.lta_floor == 0
                    && v.lta_ceiling == 0
                    && v.print_effective_from_ts.iter().all(|t| *t == 0)
                    && v.print_next_reset_ts.iter().all(|t| *t == 0)
                    && v.print_floor.iter().all(|t| *t == 0)
                    && v.print_ceiling.iter().all(|t| *t == 0)))
            && captures(
                &v.capture_periods,
                &v.capture_flags.map(|f| f & !HEAD_CAPTURES),
                &v.capture_listing,
                &v.capture_freeze,
            )
            && (0..HEADS_CONTRACT_RING)
                .all(|i| v.adm_open[i] <= v.open_claims && v.restart_reveals[i] <= v.reveals)
            // A sample without a deciding print has no level.
            && (0..HEADS_CONTRACT_RING * HEAD_SLOT_SAMPLES)
                .all(|k| v.sample_ts[k] != 0 || (v.sample_values[k] == 0 && v.sample_resets[k] == 0))
            && v.samples_reserved == [0; 4]
            && v.finalized_reveals <= v.reveals
            && v.print_claim_seq
                .iter()
                .all(|q| *q < v.claim_seq || *q == 0)
            && !crate::pubkey_is_default(&v.board)
            && !crate::pubkey_is_default(&v.proposer)
    }

    pub(super) fn Delegation(v: &super::Delegation) -> bool {
        let masks = 0..super::HELD_MASKS;
        let empty = v.held == 0
            && v.held_principal == 0
            && v.case_ref == 0
            && v.held_exposed.iter().all(|x| *x == 0)
            && v.held_case_adjust.iter().all(|x| *x == 0);
        v.held_free == 0
            && v.held_exempt == 0
            && (empty || v.held > 0)
            && v.held <= v.held_principal
            && masks
                .clone()
                .map(|m| u128::from(v.held_exposed[m]))
                .sum::<u128>()
                == u128::from(v.held_principal)
            && masks.clone().all(|m| {
                v.held_case_adjust[m] <= v.held_exposed[m]
                    && (v.case_ref != 0 || v.held_case_adjust[m] == 0)
                    && (0..HEADS_CONTRACT_RING)
                        .all(|i| m & (1 << i) == 0 || v.held_gens[i] != 0 || v.held_exposed[m] == 0)
            })
            && (0..HEADS_CONTRACT_RING).all(|i| {
                v.weighed[i] <= v.active.saturating_add(v.fresh)
                    && (v.held_gens[i] != 0 || v.weighed[i] == 0)
            })
            && (0..super::HELD_COOLING).all(|k| (v.cooling[k] == 0) == (v.cooling_end[k] == 0))
            && captures(
                &v.capture_periods,
                &v.capture_flags,
                &v.capture_listing,
                &v.capture_freeze,
            )
            && !crate::pubkey_is_default(&v.head)
            && !crate::pubkey_is_default(&v.owner)
    }

    pub(super) fn HeadClaim(v: &super::HeadClaim) -> bool {
        heads_evidence_is_valid(v.evidence_kind, &v.evidence_commitment)
            && heads_lta_terms_are_valid(
                v.lta_effective_from_ts,
                v.lta_next_reset_ts,
                v.lta_floor,
                v.lta_ceiling,
            )
            && (v.evidence_kind == HEADS_EVIDENCE_NONE)
                == crate::pubkey_is_default(&v.evidence_object)
            && !crate::pubkey_is_default(&v.head)
            && !crate::pubkey_is_default(&v.fetcher)
    }

    pub(super) fn HeadChallenge(v: &super::HeadChallenge) -> bool {
        heads_evidence_is_valid(v.evidence_kind, &v.evidence_commitment)
            && (v.evidence_kind == HEADS_EVIDENCE_NONE)
                == crate::pubkey_is_default(&v.evidence_object)
            && !crate::pubkey_is_default(&v.target)
            && !crate::pubkey_is_default(&v.challenger)
    }

    pub(super) fn HeadsContract(v: &super::HeadsContract) -> bool {
        let items = usize::from(v.item_count);
        let components = usize::from(v.component_count);
        let listed = usize::from(v.listed_count);
        items >= 1
            && usize::from(v.ring_slot) < HEADS_CONTRACT_RING
            && usize::from(v.components_settled) <= components
            && items_and_components(
                items,
                &v.item_bucket_ids,
                &v.item_weight_bps,
                &v.item_mix_bps,
                &v.item_mix_basis,
                components,
                &v.component_items,
                &v.component_channels,
                &v.component_sentence_hashes,
            )
            && zero_from(&v.item_change_ppm, items)
            && zero_from(&v.item_reference_ppm, items)
            && zero_from(&v.item_reference_flags, items)
            && zero_from(&v.item_reference_reason, items)
            && v.item_reference_flags
                .iter()
                .all(|f| f & !REFERENCE_MASK == 0)
            && v.item_reference_reason.iter().all(|r| {
                matches!(
                    *r,
                    REFERENCE_REASON_NONE
                        | REFERENCE_REASON_MIX_BASIS_NONE
                        | REFERENCE_REASON_INVALID_MIX
                )
            })
            && zero_from(&v.item_degraded_channels, items)
            && v.item_degraded_channels
                .iter()
                .all(|c| c & !DEGRADED_CHANNELS_MASK == 0)
            && zero_from(&v.component_listed_sources, components)
            && zero_from(&v.component_listed_origins, components)
            && zero_from(&v.component_freeze_origins, components)
            && zero_from(&v.component_degraded, components)
            && zero_from(&v.component_duplicates, components)
            && v.component_degraded
                .iter()
                .all(|d| *d <= DEGRADED_TOO_LITTLE_WEIGHT)
            && zero_from(&v.component_flags, components)
            && zero_from(&v.component_change_ppm, components)
            && zero_from(&v.component_freeze_change_ppm, components)
            && zero_from(&v.component_eligible, components)
            && zero_from(&v.component_weight, components)
            && listed <= HEADS_MAX_LIVE_SOURCES
            && zero_from(&v.listed_head_ids, listed)
            && zero_from(&v.listed_components, listed)
            && zero_from(&v.listed_flags, listed)
            && zero_from(&v.listed_openings, listed)
            && v.listed_components[..listed]
                .iter()
                .all(|c| usize::from(*c) < components)
            && v.listed_flags[..listed]
                .iter()
                .all(|f| f & !LIVE_DUPLICATE == 0)
            && !crate::pubkey_is_default(&v.period)
            && !crate::pubkey_is_default(&v.board)
    }

    pub(super) fn HeadsEpoch(v: &super::HeadsEpoch) -> bool {
        !crate::pubkey_is_default(&v.board) && v.end_ts >= v.start_ts
    }
}

/// Loads a heads account owned by this program: owner, not executable, exact length, decode,
/// discriminator, version, invariants, and the stored-bump PDA of its stored identities. The
/// caller then binds those identities to the other accounts of its instruction.
// The handlers load and store through these (the tests exercise them already).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn load_heads_account<T: HeadsAccount>(
    program_id: &Pubkey,
    info: &AccountInfo,
) -> Result<Box<T>, ProgramError> {
    owned_with_len(program_id, info, T::LEN)?;
    let value = T::unpack(&info.try_data()?)?;
    expect_address(value.address(program_id), info)?;
    Ok(value)
}

// The type-independent checks of [`load_heads_account`], kept out of line so each layout's
// instance stays small on SBF.
#[inline(never)]
fn owned_with_len(program_id: &Pubkey, info: &AccountInfo, len: usize) -> Result<(), ProgramError> {
    if info.owner != program_id || info.executable || info.data_len() != len {
        return Err(invalid_account());
    }
    Ok(())
}

#[inline(never)]
fn expect_address(address: Option<Pubkey>, info: &AccountInfo) -> Result<(), ProgramError> {
    if address.as_ref() != Some(info.key) {
        return Err(invalid_account());
    }
    Ok(())
}

/// Writes a heads account back (the account's allocation must already be `T::LEN`).
// The handlers load and store through these (the tests exercise them already).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn store_heads_account<T: HeadsAccount>(
    info: &AccountInfo,
    value: &T,
) -> Result<(), ProgramError> {
    // Never write what no instruction could load again: the instruction fails instead, and the
    // account stays as it was (round 4 N1).
    if !value.is_consistent() {
        return Err(OracleError::InvalidHeadsState.into());
    }
    value.pack(&mut info.try_data_mut()?)
}

/// `[ameba-oracle-v1, heads-config]`.
pub fn find_heads_config_address(program_id: &Pubkey) -> (Pubkey, u8) {
    find(program_id, &[HEADS_CONFIG_SEED])
}

/// `[ameba-oracle-v1, heads-authority]`: the AMBA vault's owner.
pub fn find_heads_authority_address(program_id: &Pubkey) -> (Pubkey, u8) {
    find(program_id, &[HEADS_AUTHORITY_SEED])
}

/// The heads-authority address at a stored bump.
pub fn heads_authority_address(program_id: &Pubkey, bump: u8) -> Option<Pubkey> {
    derive(program_id, &[HEADS_AUTHORITY_SEED], bump)
}

/// The AMBA vault: the heads authority's associated token account for `mint` under
/// `token_program` (classic SPL Token or Token-2022).
pub fn find_heads_amba_vault_address(
    program_id: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Pubkey {
    crate::associated_token::get_associated_token_address_with_program_id(
        &find_heads_authority_address(program_id).0,
        mint,
        token_program,
    )
}

/// `[ameba-oracle-v1, heads-board, product_id]`.
pub fn find_heads_board_address(program_id: &Pubkey, product_id: &[u8; 32]) -> (Pubkey, u8) {
    find(program_id, &[HEADS_BOARD_SEED, product_id])
}

/// `[ameba-oracle-v1, heads-stake, owner]`.
pub fn find_heads_stake_address(program_id: &Pubkey, owner: &Pubkey) -> (Pubkey, u8) {
    find(program_id, &[HEADS_STAKE_SEED, owner.as_ref()])
}

/// `[ameba-oracle-v1, head, board, head_id (u32 LE)]`.
pub fn find_head_address(program_id: &Pubkey, board: &Pubkey, head_id: u32) -> (Pubkey, u8) {
    find(
        program_id,
        &[HEAD_SEED, board.as_ref(), &head_id.to_le_bytes()],
    )
}

/// `[ameba-oracle-v1, head-delegation, head, owner]`.
pub fn find_head_delegation_address(
    program_id: &Pubkey,
    head: &Pubkey,
    owner: &Pubkey,
) -> (Pubkey, u8) {
    find(
        program_id,
        &[HEAD_DELEGATION_SEED, head.as_ref(), owner.as_ref()],
    )
}

/// `[ameba-oracle-v1, head-claim, head, seq (u32 LE)]`.
pub fn find_head_claim_address(program_id: &Pubkey, head: &Pubkey, seq: u32) -> (Pubkey, u8) {
    find(
        program_id,
        &[HEAD_CLAIM_SEED, head.as_ref(), &seq.to_le_bytes()],
    )
}

/// `[ameba-oracle-v1, head-challenge, target, challenger]`.
pub fn find_head_challenge_address(
    program_id: &Pubkey,
    target: &Pubkey,
    challenger: &Pubkey,
) -> (Pubkey, u8) {
    find(
        program_id,
        &[HEAD_CHALLENGE_SEED, target.as_ref(), challenger.as_ref()],
    )
}

/// `[ameba-oracle-v1, heads-contract, period]`.
pub fn find_heads_contract_address(program_id: &Pubkey, period: &Pubkey) -> (Pubkey, u8) {
    find(program_id, &[HEADS_CONTRACT_SEED, period.as_ref()])
}

/// `[ameba-oracle-v1, heads-weights, contract, component (u8)]`.
pub fn find_heads_weights_address(
    program_id: &Pubkey,
    contract: &Pubkey,
    component: u8,
) -> (Pubkey, u8) {
    find(
        program_id,
        &[HEADS_WEIGHTS_SEED, contract.as_ref(), &[component]],
    )
}

/// `[ameba-oracle-v1, heads-epoch, board, month_id (u32 LE)]`.
pub fn find_heads_epoch_address(
    program_id: &Pubkey,
    board: &Pubkey,
    month_id: u32,
) -> (Pubkey, u8) {
    find(
        program_id,
        &[HEADS_EPOCH_SEED, board.as_ref(), &month_id.to_le_bytes()],
    )
}

impl Head {
    /// The name as text (the layout check keeps it UTF-8).
    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..usize::from(self.name_len).min(HEAD_NAME_MAX_BYTES)])
            .unwrap_or("")
    }

    /// The accepted prints in the ring, oldest first, as `(source_ts, value)`.
    pub fn prints(&self) -> Vec<(u64, u64)> {
        let count = self.print_count as usize;
        let kept = count.min(HEAD_PRINT_CAPACITY);
        (count - kept..count)
            .map(|i| {
                let slot = i % HEAD_PRINT_CAPACITY;
                (self.print_source_ts[slot], self.print_values[slot])
            })
            .collect()
    }
}

impl HeadsBoard {
    /// Whether live entry `entry`'s price is in effect at `at`: an LTA price before its next
    /// reset, any other always (round 4 N5).
    pub fn live_in_effect(&self, entry: usize, at: u64) -> bool {
        let reset = u64::from(self.live_reset_ts[entry]);
        reset == 0 || at < reset
    }

    /// Whether live entry `entry` still meets activation backing under the board's parameters
    /// now: `floor_activate` and `min_delegators` (round 6 review, finding 1). An entry that does
    /// not stays live (a member, earning, recorded at settlement by its weight) but is no origin
    /// toward `min_live_sources` at a listing or a freeze.
    pub fn live_backed(&self, entry: usize) -> bool {
        heads_backed(
            self.live_delegations[entry],
            u16::from(self.live_delegators[entry]),
            self.params.floor_activate,
            self.params.min_delegators,
        )
    }

    /// Component `component`'s live entries: their positions in the list, and their origin
    /// entries (all counting).
    pub fn origin_entries(
        &self,
        component: usize,
    ) -> (Vec<usize>, Vec<ameba_oracle_heads::channels::OriginEntry>) {
        // Exactly this component's population: a board of 24 components allocates no more
        // (round 9 review, heap).
        let live = usize::from(self.live_count);
        let n = (0..live)
            .filter(|&e| usize::from(self.live_components[e]) == component)
            .count();
        let mut members = Vec::with_capacity(n);
        let mut entries = Vec::with_capacity(n);
        for e in 0..live {
            if usize::from(self.live_components[e]) == component {
                members.push(e);
                entries.push(ameba_oracle_heads::channels::OriginEntry {
                    origin: self.live_origin_ids[e],
                    live_at: self.live_since_ts[e],
                    head_id: self.live_head_ids[e],
                    counts: true,
                });
            }
        }
        (members, entries)
    }

    /// The independent origins among component `component`'s live entries `keep` marks.
    fn origins_where(&self, component: usize, keep: &dyn Fn(usize) -> bool) -> u16 {
        let (members, entries) = self.origin_entries(component);
        let first = ameba_oracle_heads::channels::first_of_origin(&entries);
        let mut kept = Vec::with_capacity(members.len());
        for &e in &members {
            kept.push(keep(e));
        }
        ameba_oracle_heads::channels::count_origins(&first, &kept) as u16
    }

    /// Whether live entry `entry` is a member of ring slot `slot`'s contract.
    pub fn live_member(&self, entry: usize, slot: usize) -> bool {
        self.live_flags[entry] & (LIVE_MEMBER << slot) != 0
    }

    /// The freeze coverage of ring slot `slot`'s contract for `component`: the independent
    /// origins among the contract's live members still backed whose price is in effect at `at`
    /// (its freeze). A source the contract excluded, or one unbacked or expired then, never
    /// supplies one (round 7 review, B1).
    pub fn member_coverage(&self, slot: usize, component: usize, at: u64) -> u16 {
        self.origins_where(component, &|e| {
            self.live_member(e, slot) && self.live_backed(e) && self.live_in_effect(e, at)
        })
    }

    /// Component `component`'s live sources in effect at `at`, and the independent origins
    /// (`channels::distinct_origins`) among those of them still backed: what a listing at `at`
    /// opens and counts (round 4 N5: an LTA price past its reset is no opening and no origin;
    /// round 6 review, finding 1: an entry below activation backing is no origin).
    pub fn listing_coverage(&self, component: usize, at: u64) -> (u16, u16) {
        let sources = (0..usize::from(self.live_count))
            .filter(|&e| {
                usize::from(self.live_components[e]) == component && self.live_in_effect(e, at)
            })
            .count();
        let origins = self.origins_where(component, &|e| {
            self.live_in_effect(e, at) && self.live_backed(e)
        });
        (sources as u16, origins)
    }
}

/// An LTA reset as the board's live list keeps it (`HeadsBoard::live_reset_ts`).
pub fn live_reset(reset_ts: u64) -> u32 {
    u32::try_from(reset_ts).unwrap_or(u32::MAX)
}

/// Held masks a delegation keeps (`Delegation::held_exposed`): every subset of the ring's four
/// slots, so each exposure keeps its own principal (round 6 review, finding 4).
pub const HELD_MASKS: usize = 1 << HEADS_CONTRACT_RING;

/// Cooling withdrawals a delegation keeps apart (`Delegation::cooling`).
pub const HELD_COOLING: usize = 4;

impl Head {
    /// The update-prize watermark a fill starts at: the day before the later of its fill's
    /// source day and its posting day, so only observations from both on earn (round 6 review,
    /// finding 5; round 7 review, B3). 0 before a fill.
    pub fn prize_floor(&self) -> u32 {
        if self.fill_source_ts == 0 {
            return 0;
        }
        ((self.fill_source_ts.max(self.posted_ts) / 86_400) as u32).saturating_sub(1)
    }
}

/// Whether `delegation` from `delegators` meets activation backing at `threshold`: as
/// `thresholds::activates` on a board needing two delegators or more; needing one, by the total
/// alone, positive and at the threshold. Every positive delegation counts there (the user's
/// decision, 2026-10-31), so the count is never read: it only saturates, and no number of
/// one-atom delegations, nor a delegation flagged dust under the earlier rule, moves backing
/// (round 18 review, N1 and N2). Activation, the live list's backing (listing and freeze
/// coverage) and eviction's unbacked time all read this.
pub fn heads_backed(delegation: u64, delegators: u16, threshold: u64, min_delegators: u16) -> bool {
    if min_delegators >= 2 {
        ameba_oracle_heads::thresholds::activates(delegation, delegators, threshold, min_delegators)
    } else {
        delegation > 0 && delegation >= threshold
    }
}

impl HeadsBoard {
    /// Whether ring slot `i` holds the unsettled contract of generation `gen` and `head` is one
    /// of its members (listed by it, or joined): only then does stake behind the head weigh in
    /// it (round 7 review, B2).
    pub fn weighs_in(&self, head: &Head, i: usize, gen: u32) -> bool {
        self.unsettled(i, gen)
            && head.capture_periods[i] == self.contract_periods[i]
            && head.capture_flags[i] & CAPTURE_MEMBER != 0
    }

    /// Whether ring slot `i` holds the unsettled contract of generation `gen`.
    pub fn unsettled(&self, i: usize, gen: u32) -> bool {
        gen != 0
            && self.contract_gens[i] == gen
            && matches!(self.contract_status[i], RING_ACCUMULATING | RING_COLLECTING)
    }
}

impl Delegation {
    /// Whether held mask `m` names a contract among `gens` (generations by ring slot).
    pub fn mask_meets(&self, m: usize, gens: &[u32]) -> bool {
        (0..HEADS_CONTRACT_RING)
            .any(|i| m & (1 << i) != 0 && self.held_gens[i] != 0 && gens[i] == self.held_gens[i])
    }

    /// Of held mask `m`'s principal, what case `case`, opened with `gens` unsettled, strikes:
    /// all of a mask meeting `gens` but what is exempt from it, and of any other mask what
    /// answers it by number (`held_case_adjust`, while `case_ref` is that case).
    pub fn held_answering(&self, m: usize, case: u32, gens: &[u32]) -> u64 {
        let adjust = if case != 0 && self.case_ref == case {
            self.held_case_adjust[m]
        } else {
            0
        };
        if self.mask_meets(m, gens) {
            self.held_exposed[m] - adjust
        } else {
            adjust
        }
    }

    /// The held principal some strike can still reach: every mask naming an unsettled contract
    /// (a case opening now strikes it), and what the open case strikes of the others. The
    /// remaining strikes' escrow is sized on it, exactly (one base, so rounding is never split).
    pub fn held_exposure(&self, board: &HeadsBoard, head: &Head) -> u64 {
        let open = head.unresolved_cases > 0;
        (0..HELD_MASKS)
            .map(|m| {
                if (0..HEADS_CONTRACT_RING)
                    .any(|i| m & (1 << i) != 0 && board.weighs_in(head, i, self.held_gens[i]))
                {
                    self.held_exposed[m]
                } else if open {
                    self.held_answering(m, head.source_case, &head.source_case_open_gens)
                } else {
                    0
                }
            })
            .sum()
    }

    /// What the delegation must keep held now: what the head's remaining strikes can take of
    /// its exposed principal (`held_exposure`), capped at what it holds.
    pub fn held_escrow(
        &self,
        params: &ameba_oracle_heads::Params,
        board: &HeadsBoard,
        head: &Head,
    ) -> u64 {
        let exposed = self.held_exposure(board, head);
        let left = params.max_strikes.saturating_sub(head.strike_count);
        let escrow = if self.is_proposer {
            ameba_oracle_heads::liability::proposer_escrow_required(
                exposed,
                params.slash_rate_bps,
                left,
                params.proposer_slash_bps,
            )
        } else {
            ameba_oracle_heads::liability::escrow_required(exposed, params.slash_rate_bps, left)
        };
        escrow.min(self.held)
    }

    /// What may go back to free stake at `now`: what is held above both its escrow and what is
    /// still cooling.
    pub fn held_releasable(
        &self,
        params: &ameba_oracle_heads::Params,
        board: &HeadsBoard,
        head: &Head,
        now: u64,
    ) -> u64 {
        let cooling: u64 = (0..HELD_COOLING)
            .filter(|&k| self.cooling_end[k] > now)
            .map(|k| self.cooling[k])
            .sum();
        let locked = self
            .held_escrow(params, board, head)
            .max(cooling.min(self.held));
        self.held - locked
    }

    /// The undelegated part as the shared escrow type.
    pub fn held(&self) -> ameba_oracle_heads::liability::Held {
        ameba_oracle_heads::liability::Held {
            principal: self.held_principal,
            held: self.held,
        }
    }

    /// Stores the shared escrow type back.
    pub fn set_held(&mut self, held: ameba_oracle_heads::liability::Held) {
        self.held_principal = held.principal;
        self.held = held.held;
    }
}

// Samples and levels
// ------------------------------------------------------------------------------------------
//
// What a contract's settlement records of a head (`processor::heads::contracts`), here so that the
// off-chain indication (`heads_view`) computes exactly the same (round 14 review, N2).

/// Samples per window of a contract slot (`HEAD_SLOT_SAMPLES` holds the freeze window's, then the
/// settlement window's).
pub const HEAD_WINDOW_SAMPLES: usize = 5;

/// The instants of a contract: five from `freeze_window` to `freeze`, five from `freeze` to
/// `expiry`, the last of each at its window's end.
pub fn heads_sample_instants(
    freeze_window: u64,
    freeze: u64,
    expiry: u64,
) -> [u64; HEAD_SLOT_SAMPLES] {
    core::array::from_fn(|j| {
        let (start, end, k) = if j < HEAD_WINDOW_SAMPLES {
            (freeze_window, freeze, j)
        } else {
            (freeze, expiry, j - HEAD_WINDOW_SAMPLES)
        };
        start + end.saturating_sub(start) * (k as u64 + 1) / HEAD_WINDOW_SAMPLES as u64
    })
}

/// A print's level at `at`: its value, unless past its own reset (0: none).
pub fn heads_print_level(value: u64, reset: u64, at: u64) -> u64 {
    if reset == 0 || at < reset {
        value
    } else {
        0
    }
}

/// Ring slot `i`'s samples and origins from the head's latest print alone, at the instants `at`:
/// what a slot starting now has (`samples::restart`).
pub fn heads_rebuild_slot(head: &mut Head, i: usize, at: &[u64; HEAD_SLOT_SAMPLES]) {
    for (j, &tau) in at.iter().enumerate() {
        let k = i * HEAD_SLOT_SAMPLES + j;
        let none = head.value_source_ts == 0 || head.value_source_ts > tau;
        head.sample_ts[k] = if none { 0 } else { head.value_source_ts };
        head.sample_values[k] = if none { 0 } else { head.value };
        head.sample_resets[k] = if none {
            0
        } else {
            u32::try_from(head.lta_next_reset_ts).unwrap_or(u32::MAX)
        };
    }
    let origin = if head.value_source_ts != 0 {
        head.origin_id
    } else {
        [0; 32]
    };
    head.standing_origin[i] = origin;
    head.freeze_origin[i] = origin;
}

/// What a contract's settlement records of a head (`SettleComponent`, the head synced): its
/// levels, the medians of its samples in each window that have a level, and the origins deciding
/// the expiry and the freeze; the changes from `opening`; whether it is eligible (not retired by
/// the cutoff K) and was live at the freeze (only then is its freeze change kept).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeadsLevelRecord {
    pub eligible: bool,
    pub live_at_freeze: bool,
    pub level: Option<u64>,
    pub freeze_level: Option<u64>,
    pub origin: [u8; 32],
    pub freeze_origin: [u8; 32],
    pub change_ppm: Option<i64>,
    pub freeze_change_ppm: Option<i64>,
}

/// A contract's `(level, freeze level, origin, freeze origin)` for `head` (its slot on the
/// contract): the medians of its samples in each window that have a level (the deciding print's
/// value at its instant, none past its reset), and the origins of the prints deciding its expiry
/// and its freeze.
pub fn heads_sampled_levels(
    head: &Head,
    contract: &HeadsContract,
    freeze_window: u64,
) -> (Option<u64>, Option<u64>, [u8; 32], [u8; 32]) {
    let i = usize::from(contract.ring_slot);
    let at = heads_sample_instants(freeze_window, contract.freeze_ts, contract.expiry_ts);
    let samples: [u64; HEAD_SLOT_SAMPLES] = core::array::from_fn(|j| {
        let k = i * HEAD_SLOT_SAMPLES + j;
        if head.sample_ts[k] == 0 {
            0
        } else {
            heads_print_level(
                head.sample_values[k],
                u64::from(head.sample_resets[k]),
                at[j],
            )
        }
    });
    let median = |w: &[u64]| {
        let mut v: Vec<u64> = w.iter().copied().filter(|s| *s > 0).collect();
        ameba_oracle_heads::median::median_u64(&mut v)
    };
    (
        median(&samples[HEAD_WINDOW_SAMPLES..]),
        median(&samples[..HEAD_WINDOW_SAMPLES]),
        head.standing_origin[i],
        head.freeze_origin[i],
    )
}

/// [`HeadsLevelRecord`] of `head` (its slot on `contract`) against `opening`.
pub fn heads_level_record(head: &Head, contract: &HeadsContract, opening: u64) -> HeadsLevelRecord {
    let p = &contract.params;
    let cutoff = contract.expiry_ts.saturating_add(p.challenge_window_secs);
    let live_at_freeze = head.live_ts != 0
        && head.live_ts <= contract.freeze_ts
        && (head.retired_ts == 0 || head.retired_ts > contract.freeze_ts);
    let freeze_window = ameba_oracle_heads::time::settlement_window_start(
        contract.freeze_ts,
        p.settlement_business_days,
    );
    let (level, freeze_level, origin, freeze_origin) =
        heads_sampled_levels(head, contract, freeze_window);
    let change =
        |l: Option<u64>| l.and_then(|l| ameba_oracle_heads::settle::change_ppm(opening, l));
    HeadsLevelRecord {
        eligible: head.retired_ts == 0 || head.retired_ts > cutoff,
        live_at_freeze,
        level,
        freeze_level,
        origin,
        freeze_origin,
        change_ppm: change(level),
        freeze_change_ppm: change(freeze_level).filter(|_| live_at_freeze),
    }
}

/// Domain of [`heads_listing_mix_commitment`].
pub const HEADS_LISTING_MIX_DOMAIN: &[u8] = b"amoeba-oracle-listing-mix-v1\n";

/// The commitment of a contract's quarter mix, which its listing terms document carries as
/// `"mixCommitment":"<hex>"` (`commodity-spec`'s `listing_mix_commitment` builds the same bytes):
/// `sha256(domain ‖ product id ‖ quarter start (u64 LE) ‖ item count (u8) ‖ per item: bucket id
/// ‖ spot, contract, LTA shares (u16 LE) ‖ basis byte)`.
pub fn heads_listing_mix_commitment(
    product_id: &[u8; 32],
    quarter_start_ts: u64,
    bucket_ids: &[[u8; 32]],
    mixes: &[([u16; HEADS_CHANNELS], HeadsMixBasis)],
) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(41 + bucket_ids.len() * 39);
    bytes.extend_from_slice(product_id);
    bytes.extend_from_slice(&quarter_start_ts.to_le_bytes());
    bytes.push(bucket_ids.len() as u8);
    for (bucket, (mix, basis)) in bucket_ids.iter().zip(mixes) {
        bytes.extend_from_slice(bucket);
        for share in mix {
            bytes.extend_from_slice(&share.to_le_bytes());
        }
        bytes.push(*basis as u8);
    }
    solana_program::hash::hashv(&[HEADS_LISTING_MIX_DOMAIN, &bytes]).to_bytes()
}

/// The listing terms document's field that carries a mix commitment:
/// `"mixCommitment":"<64 lowercase hex digits>"`.
pub fn heads_mix_commitment_field(commitment: &[u8; 32]) -> [u8; 82] {
    const PREFIX: &[u8] = b"\"mixCommitment\":\"";
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut field = [b'"'; 82];
    field[..PREFIX.len()].copy_from_slice(PREFIX);
    for (i, byte) in commitment.iter().enumerate() {
        field[PREFIX.len() + 2 * i] = HEX[usize::from(byte >> 4)];
        field[PREFIX.len() + 2 * i + 1] = HEX[usize::from(byte & 15)];
    }
    field
}

/// Bytes of the `HeadsWeights` header section.
pub const HEADS_WEIGHTS_HEADER_LEN: usize = HeadsWeightsHeader::LEN;
/// Bytes of one source record.
pub const HEADS_WEIGHT_SOURCE_STRIDE: usize = HeadsWeightSource::LEN;

/// Where the source records of a `HeadsWeights` account lie: `[header][sources]`, the section
/// `capacity * stride` bytes, the records past the count zero. Growing it reallocates the
/// account (at most 10 KiB per instruction, Solana's limit).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeadsWeightsLayout {
    pub source_capacity: usize,
}

impl HeadsWeightsLayout {
    pub fn of(header: &HeadsWeightsHeader) -> Self {
        Self {
            source_capacity: usize::from(header.source_capacity),
        }
    }

    pub fn sources_offset(&self) -> usize {
        HEADS_WEIGHTS_HEADER_LEN
    }

    /// The account's length.
    pub fn len(&self) -> usize {
        self.sources_offset() + self.source_capacity * HEADS_WEIGHT_SOURCE_STRIDE
    }

    pub fn is_empty(&self) -> bool {
        self.source_capacity == 0
    }

    pub fn source_range(&self, index: usize) -> core::ops::Range<usize> {
        let start = self.sources_offset() + index * HEADS_WEIGHT_SOURCE_STRIDE;
        start..start + HEADS_WEIGHT_SOURCE_STRIDE
    }
}

macro_rules! heads_record {
    ($type:ident) => {
        impl $type {
            /// Decodes one record of exactly `LEN` bytes (domains and zero padding checked).
            pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
                if data.len() != Self::LEN {
                    return Err(invalid_account());
                }
                // SAFETY: `data` holds at least `WIRE_LEN` bytes.
                unsafe { <Self as FixedStateDecode>::decode_fixed_option(data) }
                    .ok_or_else(invalid_account)
            }

            /// Encodes into exactly `LEN` bytes, zeroing the padding.
            pub fn pack(&self, data: &mut [u8]) -> Result<(), ProgramError> {
                if data.len() != Self::LEN {
                    return Err(invalid_account());
                }
                data[Self::WIRE_LEN..].fill(0);
                self.encode_fixed(data);
                Ok(())
            }
        }
    };
}

heads_record!(HeadsWeightSource);

/// Where a source record keeps its `flags` byte (after its `u32` head id): the record that
/// completes a component reads the others by this byte alone (round 3 R5).
pub const HEADS_WEIGHT_SOURCE_FLAGS_OFFSET: usize = 4;

impl HeadsWeightsHeader {
    /// Decodes the header of a `HeadsWeights` account and checks that the account's length is
    /// exactly what its capacities lay out and that every count is within its capacity.
    pub fn unpack_account(data: &[u8]) -> Result<(Self, HeadsWeightsLayout), ProgramError> {
        let header_bytes = data
            .get(..HEADS_WEIGHTS_HEADER_LEN)
            .ok_or_else(invalid_account)?;
        // SAFETY: `header_bytes` holds `LEN >= WIRE_LEN` bytes.
        let header = unsafe { <Self as FixedStateDecode>::decode_fixed_option(header_bytes) }
            .ok_or_else(invalid_account)?;
        let layout = HeadsWeightsLayout::of(&header);
        if header.discriminator != Self::DISCRIMINATOR
            || header.version != Self::VERSION
            || usize::from(header.component) >= HEADS_MAX_COMPONENTS
            || header.source_count > header.source_capacity
            || header.levels_recorded > header.source_count
            || header.eligible_sources > header.source_count
            || header.duplicate_sources > header.source_count
            || u32::from(header.listed_recorded) + u32::from(header.joined_recorded)
                > u32::from(header.source_count)
            || data.len() != layout.len()
            || crate::pubkey_is_default(&header.contract)
        {
            return Err(invalid_account());
        }
        Ok((header, layout))
    }

    /// Encodes the header into the first `LEN` bytes of the account.
    pub fn pack_account(&self, data: &mut [u8]) -> Result<(), ProgramError> {
        let header = data
            .get_mut(..HEADS_WEIGHTS_HEADER_LEN)
            .ok_or_else(invalid_account)?;
        header[Self::WIRE_LEN..].fill(0);
        self.encode_fixed(header);
        Ok(())
    }

    /// `[ameba-oracle-v1, heads-weights, contract, component, bump]`.
    pub fn address(&self, program_id: &Pubkey) -> Option<Pubkey> {
        derive(
            program_id,
            &[
                HEADS_WEIGHTS_SEED,
                self.contract.as_ref(),
                &[self.component],
            ],
            self.bump,
        )
    }
}

/// Loads the header of a `HeadsWeights` account owned by this program and checks its PDA.
// The handlers load and store through these (the tests exercise them already).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn load_heads_weights_header(
    program_id: &Pubkey,
    info: &AccountInfo,
) -> Result<(HeadsWeightsHeader, HeadsWeightsLayout), ProgramError> {
    if info.owner != program_id || info.executable {
        return Err(invalid_account());
    }
    let (header, layout) = HeadsWeightsHeader::unpack_account(&info.try_data()?)?;
    if header.address(program_id).as_ref() != Some(info.key) {
        return Err(invalid_account());
    }
    Ok((header, layout))
}
