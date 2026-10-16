use super::*;
use super::{fixed_state_deserialize_compact, fixed_state_deserialize_flat};

fixed_state_deserialize_flat!(OracleSkuCoverageManifest, 107, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    month: Pubkey,
    required_sku_root: [u8; 32],
    required_sku_count: u16,
    covered_sku_count: u16,
    planned_scramble_start_ts: u64,
    planned_listing_ts: u64,
    coverage_finalized: bool,
    coverage_complete_ts: u64,
    last_updated_slot: u64,
});

fixed_state_deserialize_flat!(SettlementSignerRegistry, 126, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    current_set: Pubkey,
    current_version: u64,
    pending_set: Pubkey,
    pending_version: u64,
    recovery_authority: Pubkey,
    proposal_nonce: u64,
});

fixed_state_deserialize_flat!(OracleProductSkuManifest, 86, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    underlying_id: [u8; 32],
    required_sku_root: [u8; 32],
    required_sku_count: u16,
    reserved: [u8; 6],
    last_updated_slot: u64,
});

fixed_state_deserialize_flat!(OracleSkuCoverageRecord, 82, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    month: Pubkey,
    sku_id: [u8; 32],
    sku_index: u16,
    active_supported_source_count: u16,
    last_updated_slot: u64,
});

fixed_state_deserialize_flat!(OracleMaturityLadderRegistry, 62, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    underlying_id: [u8; 32],
    planned_listing_ts: u64,
    planned_expiry_ts: u64,
    last_updated_slot: u64,
});

fixed_state_deserialize_flat!(UserCollateral, 58, {
    is_initialized: bool,
    bump: u8,
    owner: Pubkey,
    available_balance: u64,
    position_locked_balance: u64,
    last_action_slot: u64,
});

fixed_state_deserialize_flat!(OracleEconomicsConfig, 40, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    config_version: u64,
    economics: OracleEconomicParams,
    last_updated_slot: u64,
}, flat {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    config_version: u64,
    economics.emergency_supermajority_bps: u16,
    economics.emergency_commit_window_slots: u64,
    economics.emergency_reveal_window_slots: u64,
    last_updated_slot: u64,
});

fixed_state_deserialize_flat!(SettlementSignerSet, 393, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    registry: Pubkey,
    version: u64,
    threshold: u8,
    signer_count: u8,
    signers: [Pubkey; MAX_SETTLEMENT_SIGNER_COUNT],
    set_hash: [u8; 32],
    rotation_delay_slots: u64,
    proposed_slot: u64,
    activate_after_slot: u64,
    emergency: bool,
    proposer: Pubkey,
});

fixed_state_deserialize_flat!(OracleProductSkuDraft, 412, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    underlying_id: [u8; 32],
    draft_nonce: u64,
    expected_sku_count: u16,
    expected_sku_root: [u8; 32],
    appended_sku_count: u16,
    frontier_mask: u16,
    last_sku_id: [u8; 32],
    merkle_frontier: [[u8; 32]; ORACLE_PRODUCT_SKU_FRONTIER_NODE_COUNT],
    last_updated_slot: u64,
});

fixed_state_deserialize_flat!(OracleSupportPosition, 141, {
    is_initialized: bool,
    bump: u8,
    month: Pubkey,
    supporter: Pubkey,
    source: Pubkey,
    source_id: [u8; 32],
    support_stake: u64,
    released: bool,
    escrow_disposition: OracleEscrowDisposition,
    failed_schedule_escrow_counted: bool,
});

fixed_state_deserialize_flat!(OracleUsdcRewardVault, 94, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    mint: Pubkey,
    token_account: Pubkey,
    total_reserved: u64,
    total_paid: u64,
    last_updated_slot: u64,
});

fixed_state_deserialize_flat!(OracleUpdateClaimV2, 346, {
    claim: OracleUpdateClaimData,
    commit_hash: [u8; 32],
    commit_slot: u64,
    earliest_reveal_slot: u64,
    reveal_deadline_slot: u64,
    revealed_slot: u64,
    council_review_pending: bool,
    freshness_reward_multiplier: u8,
    revealed_at_ts: u64,
    prior_finalized_step: u64,
}, flat {
    claim.is_initialized: bool,
    claim.bump: u8,
    claim.month: Pubkey,
    claim.claim_id: [u8; 32],
    claim.source: Pubkey,
    claim.source_id: [u8; 32],
    claim.claimant: Pubkey,
    claim.prior_state: u64,
    claim.new_state: u64,
    claim.source_time: u64,
    claim.stake: u64,
    claim.status: OracleClaimStatus,
    claim.evidence_hash: [u8; 32],
    claim.archive_url_hash: [u8; 32],
    claim.escrow_disposition: OracleEscrowDisposition,
    claim.account_discriminator: [u8; 3],
    claim.account_version: u8,
    commit_hash: [u8; 32],
    commit_slot: u64,
    earliest_reveal_slot: u64,
    reveal_deadline_slot: u64,
    revealed_slot: u64,
    council_review_pending: bool,
    freshness_reward_multiplier: u8,
    revealed_at_ts: u64,
    prior_finalized_step: u64,
});

fixed_state_deserialize_compact!(
    OracleOpeningClaim,
    144,
    {
    is_initialized: bool,
    bump: u8,
    attempt: u32,
    claimant: Pubkey,
    opening_state: u64,
    source_time: u64,
    stake: u64,
    evidence_hash: [u8; 32],
    archive_url_hash: [u8; 32],
    submitted_slot: u64,
    challenge_deadline_slot: u64,
    status: OracleOpeningClaimStatus,
    escrow_disposition: OracleEscrowDisposition,
    },
    defaults {
        month: Pubkey,
        source: Pubkey,
        source_id: [u8; 32],
        canonical_locator_hash: [u8; 32],
        source_definition_hash: [u8; 32],
    }
);

fixed_state_deserialize_flat!(OracleOpeningClaimChallenge, 380, {
    is_initialized: bool,
    bump: u8,
    month: Pubkey,
    challenge_id: [u8; 32],
    claim: Pubkey,
    claim_attempt: u32,
    source: Pubkey,
    source_id: [u8; 32],
    challenger: Pubkey,
    alternative_opening_state: u64,
    alternative_source_time: u64,
    bond: u64,
    required_bond: u64,
    status: OracleChallengeStatus,
    canonical_locator_hash: [u8; 32],
    source_definition_hash: [u8; 32],
    evidence_hash: [u8; 32],
    archive_url_hash: [u8; 32],
    rule_review_slot: u64,
    escrow_disposition: OracleEscrowDisposition,
    account_discriminator: [u8; 3],
    account_version: u8,
    council_authority_version: u64,
});

#[cfg(test)]
#[path = "oracle_flat_tests.rs"]
mod oracle_flat_codec_tests;

#[cfg(test)]
#[path = "nested_leaf_tests.rs"]
mod nested_leaf_tests;

#[cfg(test)]
#[path = "compact_flat_tests.rs"]
mod compact_flat_tests;
