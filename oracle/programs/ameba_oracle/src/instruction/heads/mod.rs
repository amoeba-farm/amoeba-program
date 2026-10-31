//! Tag 210 `OracleHeadsV1`: oracle v2, "heads on pikes" (`docs/heads.md`).
//!
//! One instruction byte, then a selector byte, then that selector's exact-length Borsh payload.
//! Selectors are fixed once on dev (the 2026-10-29 simplification compacted them before that). Truncated or trailing bytes, unknown
//! selectors and out-of-domain values reject in the decoder. The types and builders compile in
//! every build; only builds with the `heads` feature route the tag (every other build classifies
//! byte 210 as unknown and rejects it before reading any account). Every instruction runs under
//! the governance envelope where the build has one; builders return the bare instruction and the
//! sender appends the envelope, as for every governed instruction.

use super::*;
use crate::fixed_codec::{CheckedCursor, CursorField, FixedStateDecode};
use crate::state::heads::{
    heads_evidence_is_valid, heads_lta_terms_are_valid, heads_mix_is_valid, HeadAccessClass,
    HeadChallengeKind, HeadChallengeReason, HeadChallengeRuling, HeadClaimKind, HeadsChannel,
    HeadsMixBasis, HeadsParams, HeadsSettlementTarget, HEADS_CHANNELS, HEADS_CONTRACT_RING,
    HEADS_MAX_COMPONENTS, HEADS_MAX_ITEMS, HEAD_NAME_MAX_BYTES,
};
use solana_program::hash::hashv;

#[cfg(not(target_os = "solana"))]
mod builders;
#[cfg(not(target_os = "solana"))]
pub use builders::*;

#[cfg(test)]
pub(crate) mod tests;

/// The instruction byte of the family.
pub const ORACLE_HEADS_V1_TAG: u8 = crate::governance_manifest::HEADS_INSTRUCTION_TAG;

/// Domain of a claim's commitment ([`heads_claim_commitment`]).
pub const HEADS_CLAIM_COMMITMENT_DOMAIN: &[u8] = b"ameba-oracle-heads-claim-v1";
/// Domain of a challenge's reason commitment ([`heads_reason_hash`]).
pub const HEADS_REASON_DOMAIN: &[u8] = b"ameba-oracle-heads-reason-v1";

/// The selectors of tag 210, in wire order.
pub mod selector {
    pub const INIT_HEADS_CONFIG: u8 = 0;
    pub const INIT_BOARD: u8 = 1;
    pub const SET_BOARD_PARAMS: u8 = 2;
    pub const FUND_BOARD: u8 = 3;
    pub const STAKE: u8 = 4;
    pub const UNSTAKE: u8 = 5;
    pub const POST_HEAD: u8 = 6;
    pub const DELEGATE: u8 = 7;
    pub const UNDELEGATE: u8 = 8;
    pub const SYNC_DELEGATION: u8 = 9;
    pub const EXPIRE_HEAD: u8 = 10;
    pub const COMMIT_CLAIM: u8 = 11;
    pub const REVEAL_CLAIM: u8 = 12;
    pub const CHALLENGE_CLAIM: u8 = 13;
    pub const RULE_CHALLENGE: u8 = 14;
    pub const FINALIZE_CLAIM: u8 = 15;
    pub const SETTLE_CHALLENGE: u8 = 16;
    pub const REFUND_EXPIRED_COMMIT: u8 = 17;
    pub const CHALLENGE_SOURCE: u8 = 18;
    pub const FINALIZE_SOURCE_CHALLENGES: u8 = 19;
    pub const LIST_CONTRACT: u8 = 20;
    pub const SETTLE_COMPONENT: u8 = 21;
    pub const FINALIZE_CONTRACT: u8 = 22;
    pub const CLAIM_REWARDS: u8 = 23;
    pub const CLAIM_PRIZE: u8 = 24;
    pub const BURN_PENDING: u8 = 25;
    pub const SET_ITEM: u8 = 26;
    pub const SET_COMPONENT: u8 = 27;
    pub const CREATE_QUARTER_PERIOD: u8 = 28;
    pub const ROLL_OVER_PRIZES: u8 = 29;
    pub const MIGRATE_HEAD: u8 = 30;
    pub const SET_RULING_DEADLINE: u8 = 31;
    /// Selectors `0..COUNT` are assigned; every other byte rejects.
    pub const COUNT: u8 = 32;
}

/// Selector 0. The payload binds the mint and token program the admin approves; the accounts
/// must match.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct InitHeadsConfigParams {
    pub amba_mint: Pubkey,
    pub amba_token_program: Pubkey,
}

/// One item of a board (a basket member, today's commodity bucket): its basket weight and its
/// channel mix `[spot, contract, LTA]` with the mix's basis. The decoder requires the shares to add
/// up to 10,000.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct HeadsItemParams {
    pub bucket_id: [u8; 32],
    pub basket_weight_bps: u16,
    pub mix_bps: [u16; HEADS_CHANNELS],
    pub mix_basis: HeadsMixBasis,
}

impl BorshDeserialize for HeadsItemParams {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let value = Self {
            bucket_id: <[u8; 32]>::deserialize_reader(reader)?,
            basket_weight_bps: u16::deserialize_reader(reader)?,
            mix_bps: <[u16; HEADS_CHANNELS]>::deserialize_reader(reader)?,
            mix_basis: HeadsMixBasis::deserialize_reader(reader)?,
        };
        if !value.is_valid() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(value)
    }
}

impl HeadsItemParams {
    pub fn is_valid(&self) -> bool {
        heads_mix_is_valid(&self.mix_bps) && u32::from(self.basket_weight_bps) <= 10_000
    }
}

/// Selector 1: the product, its parameters and 1 to `HEADS_MAX_ITEMS` items. Each item comes with
/// its three components (spot, contract, LTA) at `item * 3 + channel`, their sentences unset;
/// `SetComponent` sets each, with its sealed sentence object.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct InitBoardParams {
    pub product_id: [u8; 32],
    pub params: HeadsParams,
    pub items: Vec<HeadsItemParams>,
}

impl BorshDeserialize for InitBoardParams {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let product_id = <[u8; 32]>::deserialize_reader(reader)?;
        let params = HeadsParams::deserialize_reader(reader)?;
        let items = deserialize_bounded_vec(reader, HEADS_MAX_ITEMS, "items")?;
        if items.is_empty() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(Self {
            product_id,
            params,
            items,
        })
    }
}

/// Selector 26, admin: sets item `index` (below the board's `item_count`) or appends it (equal to
/// it), an appended item bringing its three components (sentences unset). Effective for contracts
/// listed afterwards; a listed contract keeps what it froze.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct SetItemParams {
    pub index: u8,
    pub item: HeadsItemParams,
}

impl BorshDeserialize for SetItemParams {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let value = Self {
            index: u8::deserialize_reader(reader)?,
            item: HeadsItemParams::deserialize_reader(reader)?,
        };
        if usize::from(value.index) >= HEADS_MAX_ITEMS {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(value)
    }
}

/// Selector 27, admin: sets the target sentence of component `index`, which is the mold's
/// `item * 3 + channel` (`heads_component_index`; the payload names all three so a client cannot
/// address the wrong one). `sentence_hash` is the commitment of the sealed kind-2 object holding
/// the sentence with its bracketed terms filled (`commodity-spec`'s `sentence_sha256`). A component
/// takes heads once its sentence is set; the sentence may be replaced between listings, never its
/// item or channel. Effective for contracts listed afterwards.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct SetComponentParams {
    pub index: u8,
    pub item: u8,
    pub channel: HeadsChannel,
    pub sentence_hash: [u8; 32],
}

impl BorshDeserialize for SetComponentParams {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let value = Self {
            index: u8::deserialize_reader(reader)?,
            item: u8::deserialize_reader(reader)?,
            channel: HeadsChannel::deserialize_reader(reader)?,
            sentence_hash: <[u8; 32]>::deserialize_reader(reader)?,
        };
        if !value.is_valid() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(value)
    }
}

impl SetComponentParams {
    pub fn is_valid(&self) -> bool {
        usize::from(self.item) < HEADS_MAX_ITEMS
            && usize::from(self.index)
                == crate::state::heads::heads_component_index(self.item, self.channel)
            && !crate::bytes32_is_zero(&self.sentence_hash)
    }
}

/// Selector 2: new parameters for contracts listed afterwards. `expected_params_version` is the
/// board's current version (compare-and-set).
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct SetBoardParamsParams {
    pub expected_params_version: u32,
    pub params: HeadsParams,
}

/// Selector 6. `expected_head_id` is the board's `head_count` (it seeds the head's PDA);
/// `amount` is delegated at once and must be at least `min_post`; `spec_hash` is the sealed
/// kind-2 evidence object's commitment; `name` is UTF-8 of at most `HEAD_NAME_MAX_BYTES`.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct PostHeadParams {
    pub expected_head_id: u32,
    pub component: u8,
    pub access_class: HeadAccessClass,
    pub amount: u64,
    pub spec_hash: [u8; 32],
    pub name: String,
}

impl BorshDeserialize for PostHeadParams {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let value = Self {
            expected_head_id: u32::deserialize_reader(reader)?,
            component: u8::deserialize_reader(reader)?,
            access_class: HeadAccessClass::deserialize_reader(reader)?,
            amount: u64::deserialize_reader(reader)?,
            spec_hash: <[u8; 32]>::deserialize_reader(reader)?,
            name: deserialize_bounded_string(reader, HEAD_NAME_MAX_BYTES, "name")?,
        };
        if usize::from(value.component) >= HEADS_MAX_COMPONENTS {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(value)
    }
}

/// Selector 11. `expected_seq` is the head's `claim_seq` (it seeds the claim's PDA);
/// `commit_hash` is [`heads_claim_commitment`] of what the reveal will carry; `bond` binds what
/// the fetcher agreed to lock and must equal the board's `claim_bond`.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct CommitClaimParams {
    pub expected_seq: u32,
    pub kind: HeadClaimKind,
    pub commit_hash: [u8; 32],
    pub bond: u64,
}

/// The terms an LTA-channel print carries (0 means none). Off the LTA channel every field is 0.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct HeadsLtaTerms {
    pub effective_from_ts: u64,
    pub next_reset_ts: u64,
    pub floor: u64,
    pub ceiling: u64,
}

impl HeadsLtaTerms {
    pub fn is_valid(&self) -> bool {
        heads_lta_terms_are_valid(
            self.effective_from_ts,
            self.next_reset_ts,
            self.floor,
            self.ceiling,
        )
    }

    pub fn is_none(&self) -> bool {
        *self == Self::default()
    }

    /// The price is in effect at `at` (`channels::LtaTerms::current_at`): effective by then and
    /// not past its reset. An LTA claim's price is in effect at its source time.
    pub fn in_effect_at(&self, at: u64) -> bool {
        ameba_oracle_heads::channels::LtaTerms {
            effective_from_ts: self.effective_from_ts,
            next_reset_ts: self.next_reset_ts,
            floor: self.floor,
            ceiling: self.ceiling,
        }
        .current_at(at)
    }
}

/// Selector 12. `origin_id` is the fetcher-declared hash of the underlying report's identity
/// (nonzero; challengeable as a duplicate); `lta` the LTA terms (all zero off the LTA channel);
/// `evidence_kind` is 0 (none) or the kind of the sealed evidence object passed (2, 3 or 4),
/// `evidence_commitment` its commitment (zero for none). For kinds 3 and 4 the capture time must
/// equal `source_ts`. [`heads_claim_commitment`] of these fields is what the commit carried.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct RevealClaimParams {
    pub value: u64,
    pub source_ts: u64,
    pub origin_id: [u8; 32],
    pub lta: HeadsLtaTerms,
    pub salt: [u8; 32],
    pub evidence_kind: u8,
    pub evidence_commitment: [u8; 32],
}

impl RevealClaimParams {
    pub fn is_valid(&self) -> bool {
        heads_evidence_is_valid(self.evidence_kind, &self.evidence_commitment)
            && !crate::bytes32_is_zero(&self.origin_id)
            && self.lta.is_valid()
    }
}

impl BorshDeserialize for RevealClaimParams {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let value = Self {
            value: u64::deserialize_reader(reader)?,
            source_ts: u64::deserialize_reader(reader)?,
            origin_id: <[u8; 32]>::deserialize_reader(reader)?,
            lta: HeadsLtaTerms::deserialize_reader(reader)?,
            salt: <[u8; 32]>::deserialize_reader(reader)?,
            evidence_kind: u8::deserialize_reader(reader)?,
            evidence_commitment: <[u8; 32]>::deserialize_reader(reader)?,
        };
        if !value.is_valid() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(value)
    }
}

/// Selector 13. `bond` is at least the board's `challenge_bond` (winners share the claim bond
/// pro rata to their bonds); `reason` labels the ground (same bond, resolution and slash for
/// every reason); `alternative_value` 0 means none (a competing fill carries its
/// value here); evidence as in [`RevealClaimParams`]; `reason_hash` is [`heads_reason_hash`] of
/// the reason text.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct ChallengeClaimParams {
    pub bond: u64,
    pub reason: HeadChallengeReason,
    pub alternative_value: u64,
    pub alternative_source_ts: u64,
    pub evidence_kind: u8,
    pub evidence_commitment: [u8; 32],
    pub reason_hash: [u8; 32],
}

impl BorshDeserialize for ChallengeClaimParams {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let value = Self {
            bond: u64::deserialize_reader(reader)?,
            reason: HeadChallengeReason::deserialize_reader(reader)?,
            alternative_value: u64::deserialize_reader(reader)?,
            alternative_source_ts: u64::deserialize_reader(reader)?,
            evidence_kind: u8::deserialize_reader(reader)?,
            evidence_commitment: <[u8; 32]>::deserialize_reader(reader)?,
            reason_hash: <[u8; 32]>::deserialize_reader(reader)?,
        };
        if !heads_evidence_is_valid(value.evidence_kind, &value.evidence_commitment)
            || (value.alternative_value == 0) != (value.alternative_source_ts == 0)
        {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(value)
    }
}

/// Selector 14: the council's ruling on one challenge, `Won` or `Lost`, under the council epoch
/// the seats sign for.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct RuleChallengeParams {
    pub kind: HeadChallengeKind,
    pub ruling: HeadChallengeRuling,
    pub expected_epoch: u64,
}

impl BorshDeserialize for RuleChallengeParams {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let value = Self {
            kind: HeadChallengeKind::deserialize_reader(reader)?,
            ruling: HeadChallengeRuling::deserialize_reader(reader)?,
            expected_epoch: u64::deserialize_reader(reader)?,
        };
        if value.ruling == HeadChallengeRuling::Pending || value.expected_epoch == 0 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(value)
    }
}

/// Selector 18. Bond, reason and evidence as in [`ChallengeClaimParams`].
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct ChallengeSourceParams {
    pub bond: u64,
    pub reason: HeadChallengeReason,
    pub evidence_kind: u8,
    pub evidence_commitment: [u8; 32],
    pub reason_hash: [u8; 32],
}

impl BorshDeserialize for ChallengeSourceParams {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let value = Self {
            bond: u64::deserialize_reader(reader)?,
            reason: HeadChallengeReason::deserialize_reader(reader)?,
            evidence_kind: u8::deserialize_reader(reader)?,
            evidence_commitment: <[u8; 32]>::deserialize_reader(reader)?,
            reason_hash: <[u8; 32]>::deserialize_reader(reader)?,
        };
        if !heads_evidence_is_valid(value.evidence_kind, &value.evidence_commitment) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(value)
    }
}

/// Selector 20: the contract's six-decimal base value (as tag 181 sets a v1 month's base), what it
/// settles on, and the commitment of its listing terms document (a sealed kind-2 object: products,
/// basket weights, sentences with terms, mix with basis and citation, settlement target, the
/// disclosure sentence; `commodity-spec`'s `listing_terms_sha256`). The listing gates only the
/// components its target settles on (`channels::settles`), each needing `min_live_sources`
/// independent live origins: under `FixedMixReference` every channel the item's mix weights,
/// under `ContractChannel` each item's Contract component. Other components list for display at a
/// 1/3 cap however few sources they have. A `FixedMixReference` listing needs every item's mix
/// valid on an assumed or matched basis, never none (`channels::listing_allowed`).
///
/// `items` are the board's items' mixes for the contract's quarter (the user, 2026-10-29: "LTA
/// %ages of weight need to adjust dynamically depending on the Q."), one per item in the board's
/// order; the sealed listing terms document must carry their commitment
/// (`state::heads::heads_listing_mix_commitment`, its `mixCommitment` field), and the contract
/// freezes them. The board's own item mixes are the default and display only.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct ListContractParams {
    pub base: u64,
    pub settlement_target: HeadsSettlementTarget,
    pub terms_hash: [u8; 32],
    pub items: Vec<HeadsListedMix>,
}

impl BorshDeserialize for ListContractParams {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let value = Self {
            base: u64::deserialize_reader(reader)?,
            settlement_target: HeadsSettlementTarget::deserialize_reader(reader)?,
            terms_hash: <[u8; 32]>::deserialize_reader(reader)?,
            items: deserialize_bounded_vec(reader, HEADS_MAX_ITEMS, "items")?,
        };
        if !value.is_valid() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(value)
    }
}

impl ListContractParams {
    pub fn is_valid(&self) -> bool {
        !crate::bytes32_is_zero(&self.terms_hash)
            && !self.items.is_empty()
            && self.items.len() <= HEADS_MAX_ITEMS
            && self.items.iter().all(HeadsListedMix::is_valid)
    }
}

/// One item's mix as a listing freezes it: `[spot, contract, LTA]` shares adding up to 10,000,
/// and their basis.
#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct HeadsListedMix {
    pub mix_bps: [u16; HEADS_CHANNELS],
    pub mix_basis: HeadsMixBasis,
}

impl BorshDeserialize for HeadsListedMix {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let value = Self {
            mix_bps: <[u16; HEADS_CHANNELS]>::deserialize_reader(reader)?,
            mix_basis: HeadsMixBasis::deserialize_reader(reader)?,
        };
        if !value.is_valid() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(value)
    }
}

impl HeadsListedMix {
    pub fn is_valid(&self) -> bool {
        heads_mix_is_valid(&self.mix_bps)
    }

    pub fn as_pair(&self) -> ([u16; HEADS_CHANNELS], HeadsMixBasis) {
        (self.mix_bps, self.mix_basis)
    }
}

/// Selector 28: a Forming period of a heads board's product expiring at a calendar-quarter
/// boundary, 00:00 UTC on Jan 1, Apr 1, Jul 1 or Oct 1 (the user, 2026-10-29: "after the november
/// 1st expiry market i want it to work in Quarters. so Q1 Q2 etc."), independent of v1's monthly
/// ladder (tag 1): the same Period PDA and consumer layout. The contract covers the quarter
/// before the expiry and is named by it ("RAMX Q1 2027" expires 2027-04-01).
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct CreateQuarterPeriodParams {
    pub product_id: [u8; 32],
    pub planned_listing_ts: u64,
    pub expiry_ts: u64,
}

impl BorshDeserialize for CreateQuarterPeriodParams {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let value = Self {
            product_id: <[u8; 32]>::deserialize_reader(reader)?,
            planned_listing_ts: u64::deserialize_reader(reader)?,
            expiry_ts: u64::deserialize_reader(reader)?,
        };
        if !value.is_valid() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(value)
    }
}

impl CreateQuarterPeriodParams {
    pub fn is_valid(&self) -> bool {
        !crate::bytes32_is_zero(&self.product_id)
            && ameba_oracle_heads::time::is_quarter_boundary(self.expiry_ts)
            && self.planned_listing_ts < self.expiry_ts
    }
}

/// Selector 21. The remaining accounts are the component's source heads in source order from
/// `first_source`; the component settles once every source's level is recorded, so a component
/// with more sources than one transaction carries settles over several calls.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct SettleComponentParams {
    pub component: u8,
    pub first_source: u16,
}

impl BorshDeserialize for SettleComponentParams {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let value = Self {
            component: u8::deserialize_reader(reader)?,
            first_source: u16::deserialize_reader(reader)?,
        };
        if usize::from(value.component) >= HEADS_MAX_COMPONENTS {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(value)
    }
}

/// The instructions of tag 210, one per selector (`docs/heads.md`, the selector table). Each
/// builder in this module documents its accounts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HeadsInstruction {
    /// 0, admin: the singleton config with the AMBA mint, token program and validated vault.
    InitHeadsConfig(InitHeadsConfigParams),
    /// 1, admin: a product's board with its items (no components yet).
    InitBoard(InitBoardParams),
    /// 2, admin: parameters for contracts listed afterwards.
    SetBoardParams(SetBoardParamsParams),
    /// 3, funder: USDC from the funder's `UserCollateral` into the board; `prize_cut` to the
    /// month's prize pool, the rest through the reward index.
    FundBoard { amount: u64 },
    /// 4, owner: AMBA into the vault; credits what arrived (transfer fee off).
    Stake { amount: u64 },
    /// 5, owner: free AMBA out.
    Unstake { amount: u64 },
    /// 6, proposer: a head, with its first delegation.
    PostHead(PostHeadParams),
    /// 7, owner: free stake onto a head.
    Delegate { amount: u64 },
    /// 8, owner: active delegation into cooldown; the escrow part stays held.
    Undelegate { amount: u64 },
    /// 9, anyone: strikes, captures, rewards, releases; closes an empty unexposed delegation.
    SyncDelegation,
    /// 10, anyone: an Open head past `head_ttl` expires.
    ExpireHead,
    /// 11, anyone: commit a fill (an Active head) or an update (a Live head) with its bond.
    CommitClaim(CommitClaimParams),
    /// 12, the claimant: reveal it; the challenge window starts.
    RevealClaim(RevealClaimParams),
    /// 13, anyone: a bonded claim challenge.
    ChallengeClaim(ChallengeClaimParams),
    /// 14, three council seats: rule one challenge.
    RuleChallenge(RuleChallengeParams),
    /// 15, anyone: resolve a claim once its window ended and every challenge is ruled.
    FinalizeClaim,
    /// 16, anyone: pay one resolved challenge and close it.
    SettleChallenge,
    /// 17, anyone: refund an unrevealed commit after its reveal deadline.
    RefundExpiredCommit,
    /// 18, anyone: a bonded source challenge.
    ChallengeSource(ChallengeSourceParams),
    /// 19, anyone: resolve the source case once its window ended and every challenge is ruled.
    FinalizeSourceChallenges,
    /// 20, oracle authority: list a Forming period on the board; the period becomes Live.
    ListContract(ListContractParams),
    /// 21, anyone: after expiry, record levels and settle one component.
    SettleComponent(SettleComponentParams),
    /// 22, anyone: every component settled; the period becomes Settled.
    FinalizeContract,
    /// 23, owner: delegator rewards into `UserCollateral`.
    ClaimRewards,
    /// 24, the claimant: a standing claim's prize after its month ended.
    ClaimPrize,
    /// 25, anyone: burn the pending AMBA and USDC.
    BurnPending,
    /// 26, admin: set or append an item, for contracts listed afterwards.
    SetItem(SetItemParams),
    /// 27, admin: append a component or replace its sentence, for contracts listed afterwards.
    SetComponent(SetComponentParams),
    /// 28, oracle authority: a Forming period at a calendar-quarter expiry.
    CreateQuarterPeriod(CreateQuarterPeriodParams),
    /// 29, anyone: an ended month's prize money no prize can claim joins this month's pool, once.
    RollOverPrizes,
    /// 30, anyone, dev builds only (`heads-migrate`): grows a Head of the 2,688-byte layout to the
    /// current one; `contracts` of the remaining accounts are the board's ring contracts, the
    /// rest the head's open claims.
    MigrateHead { contracts: u8 },
    /// 31, admin: the board's ruling deadline (seconds after a window, nonzero).
    SetRulingDeadline { secs: u64 },
}

impl HeadsInstruction {
    /// The selector byte.
    pub fn selector(&self) -> u8 {
        use selector::*;
        match self {
            Self::InitHeadsConfig(_) => INIT_HEADS_CONFIG,
            Self::InitBoard(_) => INIT_BOARD,
            Self::SetBoardParams(_) => SET_BOARD_PARAMS,
            Self::FundBoard { .. } => FUND_BOARD,
            Self::Stake { .. } => STAKE,
            Self::Unstake { .. } => UNSTAKE,
            Self::PostHead(_) => POST_HEAD,
            Self::Delegate { .. } => DELEGATE,
            Self::Undelegate { .. } => UNDELEGATE,
            Self::SyncDelegation => SYNC_DELEGATION,
            Self::ExpireHead => EXPIRE_HEAD,
            Self::CommitClaim(_) => COMMIT_CLAIM,
            Self::RevealClaim(_) => REVEAL_CLAIM,
            Self::ChallengeClaim(_) => CHALLENGE_CLAIM,
            Self::RuleChallenge(_) => RULE_CHALLENGE,
            Self::FinalizeClaim => FINALIZE_CLAIM,
            Self::SettleChallenge => SETTLE_CHALLENGE,
            Self::RefundExpiredCommit => REFUND_EXPIRED_COMMIT,
            Self::ChallengeSource(_) => CHALLENGE_SOURCE,
            Self::FinalizeSourceChallenges => FINALIZE_SOURCE_CHALLENGES,
            Self::ListContract(_) => LIST_CONTRACT,
            Self::SettleComponent(_) => SETTLE_COMPONENT,
            Self::FinalizeContract => FINALIZE_CONTRACT,
            Self::ClaimRewards => CLAIM_REWARDS,
            Self::ClaimPrize => CLAIM_PRIZE,
            Self::BurnPending => BURN_PENDING,
            Self::SetItem(_) => SET_ITEM,
            Self::SetComponent(_) => SET_COMPONENT,
            Self::CreateQuarterPeriod(_) => CREATE_QUARTER_PERIOD,
            Self::RollOverPrizes => ROLL_OVER_PRIZES,
            Self::MigrateHead { .. } => MIGRATE_HEAD,
            Self::SetRulingDeadline { .. } => SET_RULING_DEADLINE,
        }
    }

    /// The whole instruction data: the tag, the selector, the payload.
    pub fn data(&self) -> io::Result<Vec<u8>> {
        let mut data = vec![ORACLE_HEADS_V1_TAG];
        self.serialize(&mut data)?;
        Ok(data)
    }

    /// Decodes what follows the tag byte: exactly one selector and its payload, every value
    /// domain checked ([`Self::is_valid`]). On chain this reads through the program's shared
    /// cursor (`fixed_codec::CheckedCursor`), never Borsh's reader, whose error path cost about
    /// 10 KB of the program (size audit, 2026-10-30); `BorshDeserialize` decodes the same bytes
    /// off chain.
    pub fn unpack(payload: &[u8]) -> Result<Self, crate::ProgramError> {
        let mut c = CheckedCursor::new(payload);
        let ix = Self::read(&mut c);
        match ix {
            Some(ix) if c.finish_exact().is_ok() && ix.is_valid() => Ok(ix),
            _ => Err(crate::error::OracleError::InvalidInstructionData.into()),
        }
    }

    /// The fields of one selector's payload, in wire order (`None`: an unknown selector or a
    /// parameter block out of its domain; a short payload poisons the cursor).
    #[inline(never)]
    fn read(c: &mut CheckedCursor<'_>) -> Option<Self> {
        use selector::*;
        Some(match c.u8() {
            INIT_HEADS_CONFIG => Self::InitHeadsConfig(InitHeadsConfigParams {
                amba_mint: c.pubkey(),
                amba_token_program: c.pubkey(),
            }),
            INIT_BOARD => Self::InitBoard(InitBoardParams {
                product_id: c.bytes(),
                params: read_params(c)?,
                items: {
                    let n = c.bounded_count(HEADS_MAX_ITEMS);
                    (0..n).map(|_| read_item(c)).collect()
                },
            }),
            SET_BOARD_PARAMS => Self::SetBoardParams(SetBoardParamsParams {
                expected_params_version: c.u32(),
                params: read_params(c)?,
            }),
            FUND_BOARD => Self::FundBoard { amount: c.u64() },
            STAKE => Self::Stake { amount: c.u64() },
            UNSTAKE => Self::Unstake { amount: c.u64() },
            POST_HEAD => Self::PostHead(PostHeadParams {
                expected_head_id: c.u32(),
                component: c.u8(),
                access_class: CursorField::read(c),
                amount: c.u64(),
                spec_hash: c.bytes(),
                name: c.bounded_string(HEAD_NAME_MAX_BYTES),
            }),
            DELEGATE => Self::Delegate { amount: c.u64() },
            UNDELEGATE => Self::Undelegate { amount: c.u64() },
            SYNC_DELEGATION => Self::SyncDelegation,
            EXPIRE_HEAD => Self::ExpireHead,
            COMMIT_CLAIM => Self::CommitClaim(CommitClaimParams {
                expected_seq: c.u32(),
                kind: CursorField::read(c),
                commit_hash: c.bytes(),
                bond: c.u64(),
            }),
            REVEAL_CLAIM => Self::RevealClaim(RevealClaimParams {
                value: c.u64(),
                source_ts: c.u64(),
                origin_id: c.bytes(),
                lta: HeadsLtaTerms {
                    effective_from_ts: c.u64(),
                    next_reset_ts: c.u64(),
                    floor: c.u64(),
                    ceiling: c.u64(),
                },
                salt: c.bytes(),
                evidence_kind: c.u8(),
                evidence_commitment: c.bytes(),
            }),
            CHALLENGE_CLAIM => Self::ChallengeClaim(ChallengeClaimParams {
                bond: c.u64(),
                reason: CursorField::read(c),
                alternative_value: c.u64(),
                alternative_source_ts: c.u64(),
                evidence_kind: c.u8(),
                evidence_commitment: c.bytes(),
                reason_hash: c.bytes(),
            }),
            RULE_CHALLENGE => Self::RuleChallenge(RuleChallengeParams {
                kind: CursorField::read(c),
                ruling: CursorField::read(c),
                expected_epoch: c.u64(),
            }),
            FINALIZE_CLAIM => Self::FinalizeClaim,
            SETTLE_CHALLENGE => Self::SettleChallenge,
            REFUND_EXPIRED_COMMIT => Self::RefundExpiredCommit,
            CHALLENGE_SOURCE => Self::ChallengeSource(ChallengeSourceParams {
                bond: c.u64(),
                reason: CursorField::read(c),
                evidence_kind: c.u8(),
                evidence_commitment: c.bytes(),
                reason_hash: c.bytes(),
            }),
            FINALIZE_SOURCE_CHALLENGES => Self::FinalizeSourceChallenges,
            LIST_CONTRACT => Self::ListContract(ListContractParams {
                base: c.u64(),
                settlement_target: CursorField::read(c),
                terms_hash: c.bytes(),
                items: {
                    let n = c.bounded_count(HEADS_MAX_ITEMS);
                    (0..n)
                        .map(|_| HeadsListedMix {
                            mix_bps: core::array::from_fn(|_| c.u16()),
                            mix_basis: CursorField::read(c),
                        })
                        .collect()
                },
            }),
            SETTLE_COMPONENT => Self::SettleComponent(SettleComponentParams {
                component: c.u8(),
                first_source: c.u16(),
            }),
            FINALIZE_CONTRACT => Self::FinalizeContract,
            CLAIM_REWARDS => Self::ClaimRewards,
            CLAIM_PRIZE => Self::ClaimPrize,
            BURN_PENDING => Self::BurnPending,
            SET_ITEM => Self::SetItem(SetItemParams {
                index: c.u8(),
                item: read_item(c),
            }),
            SET_COMPONENT => Self::SetComponent(SetComponentParams {
                index: c.u8(),
                item: c.u8(),
                channel: CursorField::read(c),
                sentence_hash: c.bytes(),
            }),
            CREATE_QUARTER_PERIOD => Self::CreateQuarterPeriod(CreateQuarterPeriodParams {
                product_id: c.bytes(),
                planned_listing_ts: c.u64(),
                expiry_ts: c.u64(),
            }),
            ROLL_OVER_PRIZES => Self::RollOverPrizes,
            MIGRATE_HEAD => Self::MigrateHead { contracts: c.u8() },
            SET_RULING_DEADLINE => Self::SetRulingDeadline { secs: c.u64() },
            _ => return None,
        })
    }

    /// Every value domain a payload must satisfy beyond its encoding: what the decoder refuses
    /// and the encoder never writes.
    pub fn is_valid(&self) -> bool {
        match self {
            Self::InitBoard(p) => {
                !p.items.is_empty()
                    && p.items.len() <= HEADS_MAX_ITEMS
                    && p.items.iter().all(HeadsItemParams::is_valid)
            }
            Self::SetItem(p) => usize::from(p.index) < HEADS_MAX_ITEMS && p.item.is_valid(),
            Self::SetComponent(p) => p.is_valid(),
            Self::MigrateHead { contracts } => usize::from(*contracts) <= HEADS_CONTRACT_RING,
            Self::SetRulingDeadline { secs } => *secs != 0,
            Self::PostHead(p) => {
                p.name.len() <= HEAD_NAME_MAX_BYTES
                    && usize::from(p.component) < HEADS_MAX_COMPONENTS
            }
            Self::RevealClaim(p) => p.is_valid(),
            Self::ChallengeClaim(p) => {
                heads_evidence_is_valid(p.evidence_kind, &p.evidence_commitment)
                    && (p.alternative_value == 0) == (p.alternative_source_ts == 0)
            }
            Self::RuleChallenge(p) => {
                p.ruling != HeadChallengeRuling::Pending && p.expected_epoch != 0
            }
            Self::ChallengeSource(p) => {
                heads_evidence_is_valid(p.evidence_kind, &p.evidence_commitment)
            }
            Self::ListContract(p) => p.is_valid(),
            Self::CreateQuarterPeriod(p) => p.is_valid(),
            Self::SettleComponent(p) => usize::from(p.component) < HEADS_MAX_COMPONENTS,
            _ => true,
        }
    }
}

/// A board's parameter block, as its account stores it (`HeadsParams::LEN` bytes).
fn read_params(c: &mut CheckedCursor<'_>) -> Option<HeadsParams> {
    let bytes: [u8; HeadsParams::LEN] = c.bytes();
    // SAFETY: `bytes` holds exactly the block's wire length.
    unsafe { <HeadsParams as FixedStateDecode>::decode_fixed_option(&bytes) }
}

fn read_item(c: &mut CheckedCursor<'_>) -> HeadsItemParams {
    HeadsItemParams {
        bucket_id: c.bytes(),
        basket_weight_bps: c.u16(),
        mix_bps: core::array::from_fn(|_| c.u16()),
        mix_basis: CursorField::read(c),
    }
}

fn invalid<T>() -> io::Result<T> {
    Err(io::ErrorKind::InvalidData.into())
}

impl BorshSerialize for HeadsInstruction {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        if !self.is_valid() {
            return invalid();
        }
        self.selector().serialize(writer)?;
        match self {
            Self::InitHeadsConfig(p) => p.serialize(writer),
            Self::InitBoard(p) => p.serialize(writer),
            Self::SetItem(p) => p.serialize(writer),
            Self::SetComponent(p) => p.serialize(writer),
            Self::SetBoardParams(p) => p.serialize(writer),
            Self::MigrateHead { contracts } => contracts.serialize(writer),
            Self::FundBoard { amount }
            | Self::Stake { amount }
            | Self::Unstake { amount }
            | Self::Delegate { amount }
            | Self::Undelegate { amount } => amount.serialize(writer),
            Self::SetRulingDeadline { secs } => secs.serialize(writer),
            Self::PostHead(p) => p.serialize(writer),
            Self::CommitClaim(p) => p.serialize(writer),
            Self::RevealClaim(p) => p.serialize(writer),
            Self::ChallengeClaim(p) => p.serialize(writer),
            Self::RuleChallenge(p) => p.serialize(writer),
            Self::ChallengeSource(p) => p.serialize(writer),
            Self::ListContract(p) => p.serialize(writer),
            Self::CreateQuarterPeriod(p) => p.serialize(writer),
            Self::SettleComponent(p) => p.serialize(writer),
            Self::SyncDelegation
            | Self::ExpireHead
            | Self::FinalizeClaim
            | Self::SettleChallenge
            | Self::RefundExpiredCommit
            | Self::FinalizeSourceChallenges
            | Self::FinalizeContract
            | Self::ClaimRewards
            | Self::ClaimPrize
            | Self::BurnPending
            | Self::RollOverPrizes => Ok(()),
        }
    }
}

impl BorshDeserialize for HeadsInstruction {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        use selector::*;
        Ok(match u8::deserialize_reader(reader)? {
            INIT_HEADS_CONFIG => {
                Self::InitHeadsConfig(InitHeadsConfigParams::deserialize_reader(reader)?)
            }
            INIT_BOARD => Self::InitBoard(InitBoardParams::deserialize_reader(reader)?),
            SET_BOARD_PARAMS => {
                Self::SetBoardParams(SetBoardParamsParams::deserialize_reader(reader)?)
            }
            FUND_BOARD => Self::FundBoard {
                amount: u64::deserialize_reader(reader)?,
            },
            STAKE => Self::Stake {
                amount: u64::deserialize_reader(reader)?,
            },
            UNSTAKE => Self::Unstake {
                amount: u64::deserialize_reader(reader)?,
            },
            POST_HEAD => Self::PostHead(PostHeadParams::deserialize_reader(reader)?),
            DELEGATE => Self::Delegate {
                amount: u64::deserialize_reader(reader)?,
            },
            UNDELEGATE => Self::Undelegate {
                amount: u64::deserialize_reader(reader)?,
            },
            SYNC_DELEGATION => Self::SyncDelegation,
            EXPIRE_HEAD => Self::ExpireHead,
            COMMIT_CLAIM => Self::CommitClaim(CommitClaimParams::deserialize_reader(reader)?),
            REVEAL_CLAIM => Self::RevealClaim(RevealClaimParams::deserialize_reader(reader)?),
            CHALLENGE_CLAIM => {
                Self::ChallengeClaim(ChallengeClaimParams::deserialize_reader(reader)?)
            }
            RULE_CHALLENGE => Self::RuleChallenge(RuleChallengeParams::deserialize_reader(reader)?),
            FINALIZE_CLAIM => Self::FinalizeClaim,
            SETTLE_CHALLENGE => Self::SettleChallenge,
            REFUND_EXPIRED_COMMIT => Self::RefundExpiredCommit,
            CHALLENGE_SOURCE => {
                Self::ChallengeSource(ChallengeSourceParams::deserialize_reader(reader)?)
            }
            FINALIZE_SOURCE_CHALLENGES => Self::FinalizeSourceChallenges,
            LIST_CONTRACT => Self::ListContract(ListContractParams::deserialize_reader(reader)?),
            SETTLE_COMPONENT => {
                Self::SettleComponent(SettleComponentParams::deserialize_reader(reader)?)
            }
            FINALIZE_CONTRACT => Self::FinalizeContract,
            CLAIM_REWARDS => Self::ClaimRewards,
            CLAIM_PRIZE => Self::ClaimPrize,
            BURN_PENDING => Self::BurnPending,
            SET_ITEM => Self::SetItem(SetItemParams::deserialize_reader(reader)?),
            SET_COMPONENT => Self::SetComponent(SetComponentParams::deserialize_reader(reader)?),
            CREATE_QUARTER_PERIOD => {
                Self::CreateQuarterPeriod(CreateQuarterPeriodParams::deserialize_reader(reader)?)
            }
            ROLL_OVER_PRIZES => Self::RollOverPrizes,
            SET_RULING_DEADLINE => {
                let secs = u64::deserialize_reader(reader)?;
                if secs == 0 {
                    return invalid();
                }
                Self::SetRulingDeadline { secs }
            }
            MIGRATE_HEAD => {
                let contracts = u8::deserialize_reader(reader)?;
                if usize::from(contracts) > HEADS_CONTRACT_RING {
                    return invalid();
                }
                Self::MigrateHead { contracts }
            }
            _ => return invalid(),
        })
    }
}

/// A claim's commitment: `sha256(domain ‖ head ‖ seq (u32 LE) ‖ fetcher ‖ borsh(reveal))`, the
/// reveal's whole payload (value, source time, origin, LTA terms, salt, evidence). `CommitClaim`
/// carries it and `RevealClaim` must reproduce it.
pub fn heads_claim_commitment(
    head: &Pubkey,
    seq: u32,
    fetcher: &Pubkey,
    reveal: &RevealClaimParams,
) -> [u8; 32] {
    // Borsh's encoding of the reveal, written by hand (every field is fixed-size).
    let l = &reveal.lta;
    let mut payload = [0u8; 145];
    let parts: [&[u8]; 10] = [
        &reveal.value.to_le_bytes(),
        &reveal.source_ts.to_le_bytes(),
        &reveal.origin_id,
        &l.effective_from_ts.to_le_bytes(),
        &l.next_reset_ts.to_le_bytes(),
        &l.floor.to_le_bytes(),
        &l.ceiling.to_le_bytes(),
        &reveal.salt,
        &[reveal.evidence_kind],
        &reveal.evidence_commitment,
    ];
    let mut at = 0;
    for part in parts {
        payload[at..at + part.len()].copy_from_slice(part);
        at += part.len();
    }
    hashv(&[
        HEADS_CLAIM_COMMITMENT_DOMAIN,
        head.as_ref(),
        &seq.to_le_bytes(),
        fetcher.as_ref(),
        &payload,
    ])
    .to_bytes()
}

/// A challenge's reason commitment: `sha256(domain ‖ reason)` of the reason text.
pub fn heads_reason_hash(reason: &[u8]) -> [u8; 32] {
    hashv(&[HEADS_REASON_DOMAIN, reason]).to_bytes()
}
