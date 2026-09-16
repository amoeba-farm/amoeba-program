use super::*;
use super::{
    fixed_state_deserialize, fixed_state_deserialize_compact, fixed_state_deserialize_flat,
};

fixed_state_deserialize!(OracleSupportPosition, 141, {
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

fixed_state_deserialize_flat!(PositionRecord, 115, {
    is_initialized: bool,
    bump: u8,
    market: Pubkey,
    owner: Pubkey,
    long_qty: u64,
    short_qty: u64,
    short_collateral_locked: u64,
    long_tokens_minted: u64,
    settlement_claimed: bool,
    settlement_claimed_slot: u64,
    last_updated_slot: u64,
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

fixed_state_deserialize!(OracleUpdateClaimV2, 346, {
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
});

fixed_state_deserialize!(InstrumentDefinition, 74, {
    underlying_id: [u8; 32],
    expiry_ts: u64,
    strike_price: u64,
    cap_price: u64,
    contract_size: u64,
    max_payout_per_contract: u64,
    kind: OptionKind,
    settlement: SettlementStyle,
});

fixed_state_deserialize_flat!(MarketParameters, 33, {
    tick_size: u64,
    lot_size: u64,
    min_order_qty: u64,
    min_cancel_slots: u64,
    max_fills_per_instruction: u8,
});

fixed_state_deserialize_flat!(UserCollateral, 58, {
    is_initialized: bool,
    bump: u8,
    owner: Pubkey,
    available_balance: u64,
    position_locked_balance: u64,
    last_action_slot: u64,
});

fixed_state_deserialize_flat!(MarketMintAccounting, 32, {
    account_discriminator: [u8; 3],
    account_version: u8,
    decimals: u8,
    reserved: [u8; 3],
    total_issued: u64,
    total_consumed: u64,
    total_burned: u64,
});

fixed_state_deserialize!(OracleEconomicsConfig, 40, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    config_version: u64,
    economics: OracleEconomicParams,
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

fixed_state_deserialize_flat!(OracleProductSkuDraft, 380, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    underlying_id: [u8; 32],
    draft_nonce: u64,
    expected_sku_count: u16,
    appended_sku_count: u16,
    frontier_mask: u16,
    last_sku_id: [u8; 32],
    merkle_frontier: [[u8; 32]; ORACLE_PRODUCT_SKU_FRONTIER_NODE_COUNT],
    last_updated_slot: u64,
});

fixed_state_deserialize_flat!(SettlementObservation, 16, {
    observed_at_ts: u64,
    price_atomic: u64,
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

fixed_state_deserialize!(OracleOpeningClaimChallenge, 380, {
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
