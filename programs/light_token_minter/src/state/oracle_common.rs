use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OptionKind {
    #[default]
    CallSpread = 0,
    PutSpread = 1,
}
stable_borsh_enum!(OptionKind { CallSpread = 0, PutSpread = 1 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum SettlementStyle {
    #[default]
    CashSettledMonthly = 0,
}
stable_borsh_enum!(SettlementStyle { CashSettledMonthly = 0 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OraclePhase {
    #[default]
    Uninitialized = 0,
    Scramble = 1,
    Game = 2,
    Settled = 3,
    Closed = 4,
    /// The source recipe is frozen, but per-source opening claims are not yet final.
    Opening = 5,
    /// Coverage-aware source placement remains open until every committed terminal SKU has a
    /// supported source. It transitions into the ordinary Scramble calendar only after coverage.
    SourceSubmission = 6,
}
stable_borsh_enum!(OraclePhase { Uninitialized = 0, Scramble = 1, Game = 2, Settled = 3, Closed = 4, Opening = 5, SourceSubmission = 6 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleSettlementStatus {
    Final = 0,
    Provisional = 1,
    #[default]
    FrozenPendingEvidence = 2,
}
stable_borsh_enum!(OracleSettlementStatus { Final = 0, Provisional = 1, FrozenPendingEvidence = 2 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleSourceStatus {
    #[default]
    Candidate = 0,
    Frozen = 1,
    Inactive = 2,
    Rejected = 3,
    /// A bonded opening claim exists and is still inside challenge/finality processing.
    OpeningPending = 4,
    /// An evidence-backed opening claim finalized and set the source denominator.
    Active = 5,
    /// A duplicate candidate merged into a canonical source; its listing bond is refundable.
    Merged = 6,
    /// A candidate was omitted through the full freeze window; its listing bond is refundable.
    TimedOut = 7,
}
stable_borsh_enum!(OracleSourceStatus { Candidate = 0, Frozen = 1, Inactive = 2, Rejected = 3, OpeningPending = 4, Active = 5, Merged = 6, TimedOut = 7 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleChallengeStatus {
    #[default]
    Open = 0,
    RuleReview = 1,
    RuleReviewUnresolved = 2,
    Accepted = 3,
    Rejected = 4,
    Cancelled = 5,
}
stable_borsh_enum!(OracleChallengeStatus { Open = 0, RuleReview = 1, RuleReviewUnresolved = 2, Accepted = 3, Rejected = 4, Cancelled = 5 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleClaimStatus {
    #[default]
    Open = 0,
    Committed = 1,
    Revealed = 2,
    Finalized = 3,
    Rejected = 4,
    /// No merits decision occurred before the public terminal timeout.
    TimedOut = 5,
}
stable_borsh_enum!(OracleClaimStatus { Open = 0, Committed = 1, Revealed = 2, Finalized = 3, Rejected = 4, TimedOut = 5 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleOpeningClaimStatus {
    #[default]
    Empty = 0,
    Pending = 1,
    Challenged = 2,
    Accepted = 3,
    Rejected = 4,
    /// No merits decision occurred before the public terminal timeout.
    TimedOut = 5,
}
stable_borsh_enum!(OracleOpeningClaimStatus { Empty = 0, Pending = 1, Challenged = 2, Accepted = 3, Rejected = 4, TimedOut = 5 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleSourceChallengeOutcome {
    #[default]
    KeepSource = 0,
    RejectSource = 1,
    RuleReviewUnresolved = 2,
    MergeSource = 3,
}
stable_borsh_enum!(OracleSourceChallengeOutcome { KeepSource = 0, RejectSource = 1, RuleReviewUnresolved = 2, MergeSource = 3 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleUpdateClaimOutcome {
    #[default]
    AcceptClaim = 0,
    RejectClaim = 1,
    RuleReviewUnresolved = 2,
}
stable_borsh_enum!(OracleUpdateClaimOutcome { AcceptClaim = 0, RejectClaim = 1, RuleReviewUnresolved = 2 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleOpeningChallengeOutcome {
    #[default]
    KeepOpening = 0,
    AcceptAlternativeOpening = 1,
    SourceInactiveForMonth = 2,
    RuleReviewUnresolved = 3,
    /// Reject the pending claim without inactivating the frozen source, allowing a replacement.
    RejectOpeningForRetry = 4,
}
stable_borsh_enum!(OracleOpeningChallengeOutcome { KeepOpening = 0, AcceptAlternativeOpening = 1, SourceInactiveForMonth = 2, RuleReviewUnresolved = 3, RejectOpeningForRetry = 4 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleEmergencyDisputeKind {
    #[default]
    Source = 0,
    Update = 1,
    Opening = 2,
    BucketMedian = 3,
}
stable_borsh_enum!(OracleEmergencyDisputeKind {
    Source = 0,
    Update = 1,
    Opening = 2,
    BucketMedian = 3
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleEmergencyVoteStatus {
    #[default]
    Empty = 0,
    Committed = 1,
    Revealed = 2,
    /// The reveal window closed without a reveal; the principal lock was refunded.
    Expired = 3,
}
stable_borsh_enum!(OracleEmergencyVoteStatus { Empty = 0, Committed = 1, Revealed = 2, Expired = 3 });

/// Finite cash-reward entitlements.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleUsdcRewardKind {
    #[default]
    SourceProposer = 0,
    SourceSupport = 1,
    Opening = 2,
    Update = 3,
}
stable_borsh_enum!(OracleUsdcRewardKind {
    SourceProposer = 0,
    SourceSupport = 1,
    Opening = 2,
    Update = 3
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleUsdcRewardSchedulePhase {
    #[default]
    Building = 0,
    Funded = 1,
    EntitlementsFinalized = 2,
    /// The month remained fail-closed past its latest safe coverage boundary. Its complete
    /// reservation was released exactly once after every tracked pre-listing escrow settled.
    Aborted = 3,
}
stable_borsh_enum!(OracleUsdcRewardSchedulePhase {
    Building = 0,
    Funded = 1,
    EntitlementsFinalized = 2,
    Aborted = 3
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleEscrowKind {
    #[default]
    ListingBond = 0,
    SupportStake = 1,
    SourceChallenge = 2,
    OpeningClaim = 3,
    OpeningChallenge = 4,
    UpdateClaim = 5,
    UpdateChallenge = 6,
}
stable_borsh_enum!(OracleEscrowKind { ListingBond = 0, SupportStake = 1, SourceChallenge = 2, OpeningClaim = 3, OpeningChallenge = 4, UpdateClaim = 5, UpdateChallenge = 6 });

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleEscrowDisposition {
    #[default]
    Unsettled = 0,
    Refunded = 1,
    Slashed = 2,
    /// The lock became another live escrow, as when an accepted opening challenge replaces a claim.
    Transferred = 3,
}
stable_borsh_enum!(OracleEscrowDisposition { Unsettled = 0, Refunded = 1, Slashed = 2, Transferred = 3 });

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleEconomicParams {
    pub emergency_supermajority_bps: u16,
    pub emergency_commit_window_slots: u64,
    pub emergency_reveal_window_slots: u64,
}

impl Default for OracleEconomicParams {
    fn default() -> Self {
        Self {
            emergency_supermajority_bps: 6_000,
            emergency_commit_window_slots: 216_000,
            emergency_reveal_window_slots: 216_000,
        }
    }
}

impl OracleEconomicParams {
    pub const LEN: usize = 2 + 8 + 8;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleEconomicsConfig {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub config_version: u64,
    pub economics: OracleEconomicParams,
    pub last_updated_slot: u64,
}

#[allow(clippy::derivable_impls)]
impl Default for OracleEconomicsConfig {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            config_version: 0,
            economics: OracleEconomicParams::default(),
            last_updated_slot: 0,
        }
    }
}

impl OracleEconomicsConfig {
    pub const LEN: usize = 1 + 1 + 3 + 1 + 8 + OracleEconomicParams::LEN + 8;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OEC";
    pub const ACCOUNT_VERSION: u8 = 1;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleMonthState {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub market: Pubkey,
    pub authority: Pubkey,
    /// Unix timestamp at which rulebook Scramble day 1 begins.
    pub scramble_start_ts: u64,
    /// Unix timestamp at which the option is listed and Game Mode begins.
    pub listing_ts: u64,
    pub phase: OraclePhase,
    pub source_count: u16,
    pub frozen_source_count: u16,
    pub opened_source_count: u16,
    pub recipe_hash: [u8; 32],
    pub index_delta_bps: i64,
    pub c_raw_bps: u16,
    pub g_camo_bps: u16,
    pub g_thin_bps: u16,
    pub c_settle_bps: u16,
    pub settlement_status: OracleSettlementStatus,
    pub settlement_record: Option<Pubkey>,
    pub economics: OracleEconomicParams,
    pub last_updated_slot: u64,
    pub settlement_base_oracle_atomic: u64,
    /// Opening challenges and update claims that have not reached a terminal outcome.
    pub pending_resolution_count: u16,
    /// Unix timestamp at which the month irreversibly reached settlement finality.
    pub finalized_at_ts: u64,
    /// Frozen sources whose opening lifecycle ended as either accepted or inactive.
    /// `opened_source_count` remains accepted-only.
    pub opening_resolved_source_count: u16,
    /// 0 = not started, 1 = canonical effective weights, 255 = manifest building.
    pub weight_scheme_version: u8,
    /// Canonical product-weight coverage. This is independent of source cash support and is
    /// exactly 10_000 only after the immutable recipe manifest is complete.
    pub effective_weight_total_bps: u16,
    /// Domain-separated commitment to the canonical ordered bucket/source weight manifest.
    pub weight_manifest_hash: [u8; 32],
    /// Current nonzero-supported source-count invariant.
    pub candidate_count_tracking_version: u8,
    /// Current active-manifest initialization invariant.
    pub active_weight_initialization_version: u8,
    /// 0 = not started, 2 = finalized cash-independent bucket medians, 255 = building.
    pub active_weight_scheme_version: u8,
    /// Number of canonical product groups accumulated into the active-source manifest.
    pub active_weight_group_count: u16,
    /// Domain-separated commitment to the finalized canonical OAW/v1 manifest.
    pub active_weight_manifest_hash: [u8; 32],
    /// Current coverage-aware prelisting calendar generation.
    pub schedule_version: u8,
    /// Current finite-USDC work-economics generation.
    pub work_reward_currency_version: u8,
    /// Accepted, changed cash update claims expected in the post-finality reward registration.
    pub accepted_cash_update_count: u32,
}

impl Default for OracleMonthState {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            market: Pubkey::default(),
            authority: Pubkey::default(),
            scramble_start_ts: 0,
            listing_ts: 0,
            phase: OraclePhase::Uninitialized,
            source_count: 0,
            frozen_source_count: 0,
            opened_source_count: 0,
            recipe_hash: [0; 32],
            index_delta_bps: 0,
            c_raw_bps: 0,
            g_camo_bps: 0,
            g_thin_bps: 0,
            c_settle_bps: 0,
            settlement_status: OracleSettlementStatus::FrozenPendingEvidence,
            settlement_record: None,
            economics: OracleEconomicParams::default(),
            last_updated_slot: 0,
            settlement_base_oracle_atomic: 0,
            pending_resolution_count: 0,
            finalized_at_ts: 0,
            opening_resolved_source_count: 0,
            weight_scheme_version: 0,
            effective_weight_total_bps: 0,
            weight_manifest_hash: [0; 32],
            candidate_count_tracking_version: Self::CANDIDATE_COUNT_TRACKING_VERSION,
            active_weight_initialization_version: Self::ACTIVE_WEIGHT_INITIALIZATION_VERSION,
            active_weight_scheme_version: 0,
            active_weight_group_count: 0,
            active_weight_manifest_hash: [0; 32],
            schedule_version: Self::SKU_COVERAGE_SCHEDULE_VERSION,
            work_reward_currency_version: Self::WORK_REWARD_CURRENCY_USDC_V1,
            accepted_cash_update_count: 0,
        }
    }
}

impl OracleMonthState {
    /// One canonical wire layout is shared by the already-created and future current months.
    pub const LEN: usize = 299;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OMS";
    pub const ACCOUNT_VERSION: u8 = 1;
    pub const CANDIDATE_COUNT_TRACKING_VERSION: u8 = 1;
    pub const ACTIVE_WEIGHT_INITIALIZATION_VERSION: u8 = 1;
    pub const ACTIVE_MEDIAN_SCHEME_VERSION: u8 = 2;
    pub const SKU_COVERAGE_SCHEDULE_VERSION: u8 = 2;
    pub const WORK_REWARD_CURRENCY_USDC_V1: u8 = 1;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && self.candidate_count_tracking_version == Self::CANDIDATE_COUNT_TRACKING_VERSION
            && self.active_weight_initialization_version
                == Self::ACTIVE_WEIGHT_INITIALIZATION_VERSION
            && self.schedule_version == Self::SKU_COVERAGE_SCHEDULE_VERSION
            && self.work_reward_currency_version == Self::WORK_REWARD_CURRENCY_USDC_V1
    }

    pub fn has_rulebook_schedule(&self) -> bool {
        self.scramble_start_ts != 0 && self.listing_ts != 0
    }

    #[inline(always)]
    pub fn requires_sku_coverage(&self) -> bool {
        true
    }

    /// Semantic alias for the wire-stable `listing_ts` field: listing and Game begin atomically.
    pub const fn game_start_ts(&self) -> u64 {
        self.listing_ts
    }
}

/// Canonical product-level cursor for the latest planned rolling-maturity rung.
///
/// Every strike/market for the same underlying may initialize a month only for the exact
/// `planned_listing_ts`/`planned_expiry_ts` pair stored here. Once the prior planned listing
/// boundary has arrived, tag 181 may advance this cursor by exactly one UTC calendar month.
/// Coverage-driven delays never rewrite these planned timestamps.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleMaturityLadderRegistry {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub underlying_id: [u8; 32],
    pub planned_listing_ts: u64,
    pub planned_expiry_ts: u64,
    pub last_updated_slot: u64,
}

#[allow(clippy::derivable_impls)]
impl Default for OracleMaturityLadderRegistry {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            underlying_id: [0; 32],
            planned_listing_ts: 0,
            planned_expiry_ts: 0,
            last_updated_slot: 0,
        }
    }
}

impl OracleMaturityLadderRegistry {
    pub const LEN: usize = 96;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OML";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_canonical_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
    }
}

pub fn derive_oracle_maturity_ladder_registry_pda(
    program_id: &Pubkey,
    underlying_id: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_MATURITY_LADDER_PDA_SEED,
            underlying_id,
        ],
        program_id,
    )
}
