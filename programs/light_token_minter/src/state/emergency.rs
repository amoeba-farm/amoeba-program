use super::*;

/// Current sAMBA emergency-dispute state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleEmergencyDisputeV3 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub dispute_id: [u8; 32],
    pub kind: OracleEmergencyDisputeKind,
    pub target_id: [u8; 32],
    pub target_account: Pubkey,
    pub opened_by: Pubkey,
    pub status: OracleChallengeStatus,
    pub fallback_choice: u8,
    pub resolved_choice: u8,
    pub snapshot_slot: u64,
    pub snapshot_total_major_tokens: u64,
    pub committed_power: u64,
    pub committed_vote_count: u32,
    pub revealed_power: u64,
    pub revealed_vote_count: u32,
    pub winning_power: u64,
    pub winning_vote_count: u32,
    pub winning_choice: u8,
    pub choice_power: [u64; 3],
    pub choice_vote_count: [u32; 3],
    pub supermajority_bps: u16,
    pub commit_deadline_slot: u64,
    pub reveal_deadline_slot: u64,
    pub pot: Pubkey,
    pub resolved_slot: u64,
    /// Frozen `ceil(snapshot_sAMBA_supply / ORACLE_MAX_V3_VOTERS)` admission floor.
    pub minimum_vote_amount: u64,
    /// Frozen number of semantically valid choices: two for ordinary source challenges, three
    /// for differentiation source challenges and every update/opening dispute.
    pub choice_count: u8,
}

impl Default for OracleEmergencyDisputeV3 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            month: Pubkey::default(),
            dispute_id: [0; 32],
            kind: OracleEmergencyDisputeKind::Source,
            target_id: [0; 32],
            target_account: Pubkey::default(),
            opened_by: Pubkey::default(),
            status: OracleChallengeStatus::Open,
            fallback_choice: 0,
            resolved_choice: 0,
            snapshot_slot: 0,
            snapshot_total_major_tokens: 0,
            committed_power: 0,
            committed_vote_count: 0,
            revealed_power: 0,
            revealed_vote_count: 0,
            winning_power: 0,
            winning_vote_count: 0,
            winning_choice: 0,
            choice_power: [0; 3],
            choice_vote_count: [0; 3],
            supermajority_bps: 0,
            commit_deadline_slot: 0,
            reveal_deadline_slot: 0,
            pot: Pubkey::default(),
            resolved_slot: 0,
            minimum_vote_amount: 0,
            choice_count: 0,
        }
    }
}

impl OracleEmergencyDisputeV3 {
    pub const LEN: usize = 382;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OED";
    pub const ACCOUNT_VERSION: u8 = 3;

    pub fn has_exact_v3_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
    }
}

/// Canonical overflow-safe current vote floor.
pub fn oracle_emergency_v3_minimum_vote_amount(snapshot_samba_supply: u64) -> Option<u64> {
    if snapshot_samba_supply == 0 {
        return None;
    }
    let denominator = u64::from(crate::constants::ORACLE_MAX_V3_VOTERS);
    Some(
        snapshot_samba_supply / denominator
            + if snapshot_samba_supply.is_multiple_of(denominator) {
                0
            } else {
                1
            },
    )
}

/// Current vote records bind principal to one dispute-specific pot and one sAMBA mint.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleEmergencyVoteRecordV3 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub dispute: Pubkey,
    pub pot: Pubkey,
    pub voter: Pubkey,
    pub samba_mint: Pubkey,
    pub commit_hash: [u8; 32],
    pub locked_amount: u64,
    pub snapshot_power: u64,
    pub voting_power: u64,
    pub choice: u8,
    pub status: OracleEmergencyVoteStatus,
    pub escrow_disposition: OracleEscrowDisposition,
    pub committed_slot: u64,
    pub revealed_slot: u64,
}

impl OracleEmergencyVoteRecordV3 {
    pub const LEN: usize = 288;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OEV";
    pub const ACCOUNT_VERSION: u8 = 3;

    pub fn has_exact_v3_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleSambaEmergencyPayoutMode {
    #[default]
    Open = 0,
    Redistribute = 1,
    RefundAll = 2,
}
stable_borsh_enum!(OracleSambaEmergencyPayoutMode {
    Open = 0,
    Redistribute = 1,
    RefundAll = 2
});

/// Donation-safe accounting for one isolated sAMBA emergency-vote ATA. `total_committed` is the
/// immutable payout numerator after resolution; unsolicited ATA deposits never enlarge it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleSambaEmergencyPot {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub dispute: Pubkey,
    pub month: Pubkey,
    pub samba_mint: Pubkey,
    pub token_account: Pubkey,
    pub payout_mode: OracleSambaEmergencyPayoutMode,
    pub total_committed: u64,
    pub committed_vote_count: u32,
    pub winning_choice: u8,
    pub winning_power: u64,
    pub winning_vote_count: u32,
    pub registered_power: u64,
    pub registered_vote_count: u32,
    pub registered_base_total: u64,
    pub dust_recipient_vote: Pubkey,
    pub dust_amount: u64,
    pub registration_finalized: bool,
    pub remaining_liability: u64,
    pub settled_vote_count: u32,
    pub total_paid: u64,
    pub resolved_slot: u64,
    pub last_updated_slot: u64,
}

impl OracleSambaEmergencyPot {
    pub const LEN: usize = 288;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OEP";
    pub const ACCOUNT_VERSION: u8 = 1;
}

/// Immutable registration of one revealed vote for the unique threshold-passing winner.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleSambaWinningVote {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub pot: Pubkey,
    pub dispute: Pubkey,
    pub vote: Pubkey,
    pub voter: Pubkey,
    pub voting_power: u64,
    pub base_entitlement: u64,
    pub registered_slot: u64,
}

impl OracleSambaWinningVote {
    pub const LEN: usize = 192;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OSW";
    pub const ACCOUNT_VERSION: u8 = 1;
}

/// Immutable replay barrier and audit record for one current vote's terminal disposition.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleSambaVoteSettlementReceipt {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub pot: Pubkey,
    pub dispute: Pubkey,
    pub vote: Pubkey,
    pub voter: Pubkey,
    pub destination: Pubkey,
    pub amount: u64,
    pub disposition: OracleEscrowDisposition,
    pub settled_slot: u64,
}

impl OracleSambaVoteSettlementReceipt {
    pub const LEN: usize = 224;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OER";
    pub const ACCOUNT_VERSION: u8 = 1;
}

pub fn derive_oracle_samba_emergency_pot_pda(
    program_id: &Pubkey,
    dispute: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_SAMBA_EMERGENCY_POT_PDA_SEED,
            dispute.as_ref(),
        ],
        program_id,
    )
}

pub fn derive_oracle_source_challenge_guard_pda(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_SOURCE_CHALLENGE_GUARD_PDA_SEED,
            month.as_ref(),
            source.as_ref(),
        ],
        program_id,
    )
}

pub fn derive_oracle_update_challenge_guard_pda(
    program_id: &Pubkey,
    month: &Pubkey,
    claim: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_UPDATE_CHALLENGE_GUARD_PDA_SEED,
            month.as_ref(),
            claim.as_ref(),
        ],
        program_id,
    )
}

pub fn derive_oracle_emergency_dispute_v3_pda(
    program_id: &Pubkey,
    month: &Pubkey,
    dispute_id: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_EMERGENCY_DISPUTE_V3_PDA_SEED,
            month.as_ref(),
            dispute_id,
        ],
        program_id,
    )
}

pub fn derive_oracle_emergency_vote_v3_pda(
    program_id: &Pubkey,
    dispute: &Pubkey,
    voter: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_EMERGENCY_VOTE_V3_PDA_SEED,
            dispute.as_ref(),
            voter.as_ref(),
        ],
        program_id,
    )
}

pub fn derive_oracle_samba_winning_vote_pda(
    program_id: &Pubkey,
    pot: &Pubkey,
    vote: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_SAMBA_WINNING_VOTE_PDA_SEED,
            pot.as_ref(),
            vote.as_ref(),
        ],
        program_id,
    )
}

pub fn derive_oracle_samba_vote_settlement_pda(
    program_id: &Pubkey,
    pot: &Pubkey,
    vote: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_SAMBA_VOTE_SETTLEMENT_PDA_SEED,
            pot.as_ref(),
            vote.as_ref(),
        ],
        program_id,
    )
}
