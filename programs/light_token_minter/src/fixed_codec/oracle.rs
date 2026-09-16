use super::*;
use super::{fixed_state_deserialize, fixed_state_deserialize_flat};

fixed_state_deserialize!(OracleUpdateChallenge, 280, {
    is_initialized: bool,
    bump: u8,
    month: Pubkey,
    challenge_id: [u8; 32],
    claim: Pubkey,
    claim_id: [u8; 32],
    challenger: Pubkey,
    alternative_state: u64,
    alternative_source_time: u64,
    bond: u64,
    required_bond: u64,
    status: OracleChallengeStatus,
    evidence_hash: [u8; 32],
    archive_url_hash: [u8; 32],
    rule_review_slot: u64,
    escrow_disposition: OracleEscrowDisposition,
    account_discriminator: [u8; 3],
    account_version: u8,
    council_authority_version: u64,
});

fixed_state_deserialize!(OracleUpdateClaimData, 264, {
    is_initialized: bool,
    bump: u8,
    month: Pubkey,
    claim_id: [u8; 32],
    source: Pubkey,
    source_id: [u8; 32],
    claimant: Pubkey,
    prior_state: u64,
    new_state: u64,
    source_time: u64,
    stake: u64,
    status: OracleClaimStatus,
    evidence_hash: [u8; 32],
    archive_url_hash: [u8; 32],
    escrow_disposition: OracleEscrowDisposition,
    account_discriminator: [u8; 3],
    account_version: u8,
});

fixed_state_deserialize_flat!(OracleUpdateChallengeGuard, 222, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    month: Pubkey,
    claim: Pubkey,
    claim_id: [u8; 32],
    challenge: Pubkey,
    challenge_id: [u8; 32],
    active_dispute: Pubkey,
    resolution_step: u64,
    created_slot: u64,
    last_updated_slot: u64,
});

fixed_state_deserialize_flat!(OracleSourceChallengeGuard, 206, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    month: Pubkey,
    source: Pubkey,
    source_id: [u8; 32],
    active_challenge: Pubkey,
    active_challenge_id: [u8; 32],
    active_dispute: Pubkey,
    last_updated_slot: u64,
});

fixed_state_deserialize!(OracleUsdcRewardRegistration, 176, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    month: Pubkey,
    schedule: Pubkey,
    sku_pool: Pubkey,
    kind: OracleUsdcRewardKind,
    subject: Pubkey,
    recipient: Pubkey,
    reward_units: u8,
    last_updated_slot: u64,
});

fixed_state_deserialize!(OracleUsdcRewardReceipt, 151, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    month: Pubkey,
    schedule: Pubkey,
    recipient: Pubkey,
    kind: OracleUsdcRewardKind,
    subject: Pubkey,
    amount: u64,
    claimed_slot: u64,
});

fixed_state_deserialize_flat!(SettlementRecordV2, 158, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    market: Pubkey,
    oracle_month: Pubkey,
    settlement_ts: u64,
    settlement_price_atomic: u64,
    signed_leaf_commitment: [u8; 32],
    submitted_by: Pubkey,
    signer_set_version: u64,
});
