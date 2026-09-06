use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleSourceState {
    pub is_initialized: bool,
    pub bump: u8,
    pub month: Pubkey,
    pub source_id: [u8; 32],
    pub bucket_id: [u8; 32],
    pub source_type_hash: [u8; 32],
    pub canonical_locator_hash: [u8; 32],
    pub source_definition_hash: [u8; 32],
    pub proposer: Pubkey,
    pub baseline_state: u64,
    pub current_state: u64,
    pub listing_bond_locked: u64,
    pub support_stake_total: u64,
    pub bucket_weight_bps: u16,
    pub status: OracleSourceStatus,
    pub opening_submitted: bool,
    pub opening_evidence_hash: [u8; 32],
    pub last_finalized_step: u64,
    pub observation_count: u8,
    pub rolling_observation_hash: [u8; 32],
}

impl Default for OracleSourceState {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            month: Pubkey::default(),
            source_id: [0; 32],
            bucket_id: [0; 32],
            source_type_hash: [0; 32],
            canonical_locator_hash: [0; 32],
            source_definition_hash: [0; 32],
            proposer: Pubkey::default(),
            baseline_state: 0,
            current_state: 0,
            listing_bond_locked: 0,
            support_stake_total: 0,
            bucket_weight_bps: 0,
            status: OracleSourceStatus::Candidate,
            opening_submitted: false,
            opening_evidence_hash: [0; 32],
            last_finalized_step: 0,
            observation_count: 0,
            rolling_observation_hash: [0; 32],
        }
    }
}

impl OracleSourceState {
    pub const LEN: usize = 336;
}

/// Canonical timestamped accepted-state history for one current source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleSourceObservations {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub source: Pubkey,
    pub states: [u64; crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS],
    pub source_times: [u64; crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS],
}

impl Default for OracleSourceObservations {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            month: Pubkey::default(),
            source: Pubkey::default(),
            states: [0; crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS],
            source_times: [0; crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS],
        }
    }
}

impl OracleSourceObservations {
    pub const LEN: usize = 592;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OSO";
    pub const ACCOUNT_VERSION: u8 = 1;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleSupportPosition {
    pub is_initialized: bool,
    pub bump: u8,
    pub month: Pubkey,
    pub supporter: Pubkey,
    pub source: Pubkey,
    pub source_id: [u8; 32],
    pub support_stake: u64,
    pub released: bool,
    /// Principal disposition mirror used by the current failed-month cleanup lane.
    pub escrow_disposition: OracleEscrowDisposition,
    /// Makes V5 reward-schedule accounting for this support principal exactly once.
    pub failed_schedule_escrow_counted: bool,
}

impl OracleSupportPosition {
    pub const LEN: usize = 144;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleSourceChallenge {
    pub is_initialized: bool,
    pub bump: u8,
    pub month: Pubkey,
    pub challenge_id: [u8; 32],
    pub source: Pubkey,
    pub source_id: [u8; 32],
    pub comparison_source: Pubkey,
    pub comparison_source_id: [u8; 32],
    pub challenger: Pubkey,
    pub reason: u8,
    pub bond: u64,
    pub required_bond: u64,
    pub status: OracleChallengeStatus,
    pub evidence_hash: [u8; 32],
    pub rule_review_slot: u64,
    pub escrow_disposition: OracleEscrowDisposition,
    /// Immutable eligible-player voting supply captured when review becomes unresolved.
    /// Frozen sAMBA share supply.
    pub emergency_snapshot_total_major_tokens: u64,
    /// Current exact-sAMBA-supply snapshot marker.
    pub emergency_snapshot_version: u8,
    /// Makes V5 reward-schedule accounting for this challenge principal exactly once.
    pub failed_schedule_escrow_counted: bool,
}

impl Default for OracleSourceChallenge {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            month: Pubkey::default(),
            challenge_id: [0; 32],
            source: Pubkey::default(),
            source_id: [0; 32],
            comparison_source: Pubkey::default(),
            comparison_source_id: [0; 32],
            challenger: Pubkey::default(),
            reason: 0,
            bond: 0,
            required_bond: 0,
            status: OracleChallengeStatus::Open,
            evidence_hash: [0; 32],
            rule_review_slot: 0,
            escrow_disposition: OracleEscrowDisposition::Unsettled,
            emergency_snapshot_total_major_tokens: 0,
            emergency_snapshot_version: 0,
            failed_schedule_escrow_counted: false,
        }
    }
}

impl OracleSourceChallenge {
    pub const LEN: usize = 304;
}

/// Reusable typed lock preventing overlapping source challenges and emergency disputes.
///
/// A differentiation challenge acquires both canonical source guards atomically. Terminal
/// resolution clears both bindings; unresolved review retains the challenge binding, and current
/// escalation additionally binds `active_dispute` until resolution.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleSourceChallengeGuard {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub source: Pubkey,
    pub source_id: [u8; 32],
    pub active_challenge: Pubkey,
    pub active_challenge_id: [u8; 32],
    pub active_dispute: Pubkey,
    pub last_updated_slot: u64,
}

impl OracleSourceChallengeGuard {
    pub const LEN: usize = 224;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OCG";
    pub const ACCOUNT_VERSION: u8 = 1;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleOpeningClaim {
    pub is_initialized: bool,
    pub bump: u8,
    pub month: Pubkey,
    pub source: Pubkey,
    pub source_id: [u8; 32],
    pub attempt: u32,
    pub claimant: Pubkey,
    pub opening_state: u64,
    pub source_time: u64,
    pub stake: u64,
    pub canonical_locator_hash: [u8; 32],
    pub source_definition_hash: [u8; 32],
    pub evidence_hash: [u8; 32],
    /// Commitment to the fully validated Wayback URL supplied by the instruction. The raw URL is
    /// transport/evidence data and is not retained in this rent-bearing active escrow header.
    pub archive_url_hash: [u8; 32],
    pub submitted_slot: u64,
    pub challenge_deadline_slot: u64,
    pub status: OracleOpeningClaimStatus,
    pub escrow_disposition: OracleEscrowDisposition,
}

impl Default for OracleOpeningClaim {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            month: Pubkey::default(),
            source: Pubkey::default(),
            source_id: [0; 32],
            attempt: 0,
            claimant: Pubkey::default(),
            opening_state: 0,
            source_time: 0,
            stake: 0,
            canonical_locator_hash: [0; 32],
            source_definition_hash: [0; 32],
            evidence_hash: [0; 32],
            archive_url_hash: [0; 32],
            submitted_slot: 0,
            challenge_deadline_slot: 0,
            status: OracleOpeningClaimStatus::Empty,
            escrow_disposition: OracleEscrowDisposition::Unsettled,
        }
    }
}

impl OracleOpeningClaim {
    pub const LEN: usize = 320;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleOpeningClaimChallenge {
    pub is_initialized: bool,
    pub bump: u8,
    pub month: Pubkey,
    pub challenge_id: [u8; 32],
    pub claim: Pubkey,
    pub claim_attempt: u32,
    pub source: Pubkey,
    pub source_id: [u8; 32],
    pub challenger: Pubkey,
    pub alternative_opening_state: u64,
    pub alternative_source_time: u64,
    pub bond: u64,
    pub required_bond: u64,
    pub status: OracleChallengeStatus,
    pub canonical_locator_hash: [u8; 32],
    pub source_definition_hash: [u8; 32],
    pub evidence_hash: [u8; 32],
    /// Commitment to the fully validated alternative Wayback URL.
    pub archive_url_hash: [u8; 32],
    pub rule_review_slot: u64,
    pub escrow_disposition: OracleEscrowDisposition,
    /// Current OCH account marker.
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    /// Immutable eligible-player voting supply captured when review becomes unresolved.
    /// Frozen sAMBA share supply used by the emergency voting threshold.
    pub emergency_snapshot_total_major_tokens: u64,
}

impl Default for OracleOpeningClaimChallenge {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            month: Pubkey::default(),
            challenge_id: [0; 32],
            claim: Pubkey::default(),
            claim_attempt: 0,
            source: Pubkey::default(),
            source_id: [0; 32],
            challenger: Pubkey::default(),
            alternative_opening_state: 0,
            alternative_source_time: 0,
            bond: 0,
            required_bond: 0,
            status: OracleChallengeStatus::Open,
            canonical_locator_hash: [0; 32],
            source_definition_hash: [0; 32],
            evidence_hash: [0; 32],
            archive_url_hash: [0; 32],
            rule_review_slot: 0,
            escrow_disposition: OracleEscrowDisposition::Unsettled,
            account_discriminator: [0; 3],
            account_version: 0,
            emergency_snapshot_total_major_tokens: 0,
        }
    }
}

impl OracleOpeningClaimChallenge {
    pub const LEN: usize = 384;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OCH";
    /// Version 3 proves that any unresolved voting snapshot was taken from sAMBA supply.
    pub const ACCOUNT_VERSION: u8 = 3;

    pub fn has_valid_account_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
    }

    pub fn stamp_current_account_layout(&mut self) {
        self.account_discriminator = Self::ACCOUNT_DISCRIMINATOR;
        self.account_version = Self::ACCOUNT_VERSION;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleUpdateClaimData {
    pub is_initialized: bool,
    pub bump: u8,
    pub month: Pubkey,
    pub claim_id: [u8; 32],
    pub source: Pubkey,
    pub source_id: [u8; 32],
    pub claimant: Pubkey,
    pub prior_state: u64,
    pub new_state: u64,
    pub source_time: u64,
    pub stake: u64,
    pub status: OracleClaimStatus,
    pub evidence_hash: [u8; 32],
    pub archive_url_hash: [u8; 32],
    pub escrow_disposition: OracleEscrowDisposition,
    /// Current UC2 account marker.
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
}

impl Default for OracleUpdateClaimData {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            month: Pubkey::default(),
            claim_id: [0; 32],
            source: Pubkey::default(),
            source_id: [0; 32],
            claimant: Pubkey::default(),
            prior_state: 0,
            new_state: 0,
            source_time: 0,
            stake: 0,
            status: OracleClaimStatus::Open,
            evidence_hash: [0; 32],
            archive_url_hash: [0; 32],
            escrow_disposition: OracleEscrowDisposition::Unsettled,
            account_discriminator: [0; 3],
            account_version: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleUpdateClaimV2 {
    /// Current claim body with the UC2/v1 marker.
    pub claim: OracleUpdateClaimData,
    pub commit_hash: [u8; 32],
    pub commit_slot: u64,
    pub earliest_reveal_slot: u64,
    pub reveal_deadline_slot: u64,
    pub revealed_slot: u64,
    /// True only while this revealed claim owns one unresolved sAMBA voting checkpoint.
    pub samba_checkpoint_active: bool,
    /// One for an ordinary accepted print; `ORACLE_FRESH_UPDATE_REWARD_MULTIPLIER` for a print
    /// whose archive timestamp falls inside the primary settlement freshness window.
    pub freshness_reward_multiplier: u8,
}

#[allow(clippy::derivable_impls)]
impl Default for OracleUpdateClaimV2 {
    fn default() -> Self {
        Self {
            claim: OracleUpdateClaimData::default(),
            commit_hash: [0; 32],
            commit_slot: 0,
            earliest_reveal_slot: 0,
            reveal_deadline_slot: 0,
            revealed_slot: 0,
            samba_checkpoint_active: false,
            freshness_reward_multiplier: 0,
        }
    }
}

impl OracleUpdateClaimV2 {
    pub const LEN: usize = 384;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"UC2";
    pub const ACCOUNT_VERSION: u8 = 2;
}

pub fn derive_oracle_update_claim_v2_pda(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    claimant: &Pubkey,
    claim_id: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_UPDATE_CLAIM_V2_PDA_SEED,
            month.as_ref(),
            source.as_ref(),
            claimant.as_ref(),
            claim_id,
        ],
        program_id,
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleUpdateChallenge {
    pub is_initialized: bool,
    pub bump: u8,
    pub month: Pubkey,
    pub challenge_id: [u8; 32],
    pub claim: Pubkey,
    pub claim_id: [u8; 32],
    pub challenger: Pubkey,
    pub alternative_state: u64,
    pub alternative_source_time: u64,
    pub bond: u64,
    pub required_bond: u64,
    pub status: OracleChallengeStatus,
    pub evidence_hash: [u8; 32],
    pub archive_url_hash: [u8; 32],
    pub rule_review_slot: u64,
    pub escrow_disposition: OracleEscrowDisposition,
    /// Current UCH account marker.
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    /// Immutable eligible-player voting supply captured when review becomes unresolved.
    /// Frozen sAMBA share supply.
    pub emergency_snapshot_total_major_tokens: u64,
}

impl Default for OracleUpdateChallenge {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            month: Pubkey::default(),
            challenge_id: [0; 32],
            claim: Pubkey::default(),
            claim_id: [0; 32],
            challenger: Pubkey::default(),
            alternative_state: 0,
            alternative_source_time: 0,
            bond: 0,
            required_bond: 0,
            status: OracleChallengeStatus::Open,
            evidence_hash: [0; 32],
            archive_url_hash: [0; 32],
            rule_review_slot: 0,
            escrow_disposition: OracleEscrowDisposition::Unsettled,
            account_discriminator: [0; 3],
            account_version: 0,
            emergency_snapshot_total_major_tokens: 0,
        }
    }
}

impl OracleUpdateChallenge {
    pub const LEN: usize = 320;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"UCH";
    /// Version 3 proves that any unresolved voting snapshot was taken from sAMBA supply.
    pub const ACCOUNT_VERSION: u8 = 4;

    pub fn has_valid_account_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
    }

    pub fn stamp_current_account_layout(&mut self) {
        self.account_discriminator = Self::ACCOUNT_DISCRIMINATOR;
        self.account_version = Self::ACCOUNT_VERSION;
    }
}

/// Immutable one-challenge-per-claim proof. `active_dispute` is initially zero and may be bound
/// once when the exact unresolved challenge escalates into the current sAMBA dispute.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleUpdateChallengeGuard {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub claim: Pubkey,
    pub claim_id: [u8; 32],
    pub challenge: Pubkey,
    pub challenge_id: [u8; 32],
    pub active_dispute: Pubkey,
    /// Zero until current escalation; then the immutable step used by choices 0/1.
    pub resolution_step: u64,
    pub created_slot: u64,
    pub last_updated_slot: u64,
}

impl OracleUpdateChallengeGuard {
    pub const LEN: usize = 224;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OUG";
    pub const ACCOUNT_VERSION: u8 = 1;
}
