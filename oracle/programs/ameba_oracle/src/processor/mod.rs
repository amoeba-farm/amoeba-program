mod account_addresses;
mod account_io;
mod account_io_tokens;
mod active_weights;
mod adapters;
mod bucket_medians;
mod carry;
mod cfm_launch;
pub use carry::{
    history_median_address, history_median_cursor, read_source_checkpoint, read_source_journal,
    CheckpointView, JournalView,
};
mod collateral_accounts;
mod config;
mod council;
#[cfg(any(feature = "localnet-dev-authority", feature = "localnet-governance"))]
mod dev_bootstrap;
#[cfg(any(feature = "localnet-dev-authority", feature = "localnet-governance"))]
mod dev_formation;
mod dispatch;
mod median;
mod month_terminal;
mod oracle_core;
mod oracle_economics;
mod oracle_evidence;
mod oracle_membership;
mod oracle_rules;
mod oracle_schedule_validation;
mod oracle_state_validation;
mod oracle_validation;
mod period;
mod recipe_weights;
mod rewards;
mod schedule;
mod settlement;
mod settlement_signers;
mod settlement_sources;
mod settlement_validation;
mod sku_coverage;
mod sku_coverage_resolution;
mod sku_manifest;
mod sponsorship;
mod terminal_cleanup;
mod usdc;

use account_addresses::*;
use account_io::*;
use account_io_tokens::*;
use active_weights::*;
use adapters::*;
use bucket_medians::*;
use collateral_accounts::*;
use config::*;
pub use council::{
    council_decision, decode_council, decode_council_v3_with_profile, CouncilCase, CouncilRound,
};
use dispatch::*;
use median::{
    advance_oracle_bucket_source_snapshot, append_oracle_source_observation,
    initial_oracle_bucket_source_snapshot, oracle_bucket_bounty_share,
    update_month_bucket_contribution, validate_oracle_observation_shape,
};
pub use median::{
    bucket_index_contribution_bps, bucket_settlement_floor, deterministic_bucket_median,
    minimum_oracle_bucket_eligible_sources, oracle_bucket_security_cap,
    oracle_settlement_window_start, oracle_temporal_median_state,
};
use month_terminal::*;
use oracle_core::*;
use oracle_economics::*;
pub use oracle_rules::{
    advance_oracle_settlement_source_digest, advance_oracle_weight_manifest_hash,
    canonical_recipe_digest, initial_oracle_settlement_source_digest,
    initial_oracle_weight_manifest_hash, oracle_update_claim_required_bond,
    oracle_update_claim_v2_commitment_hash, validate_active_group_collection_completion,
    validate_active_group_source_identity, validate_canonical_settlement_provenance,
    validate_oracle_active_manifest_begin_membership, validate_oracle_active_manifest_completion,
    validate_oracle_update_claim_v2_commit_bond, validate_oracle_update_claim_v2_reveal,
};
use oracle_rules::{
    apply_oracle_source_merge, ensure_live_revealed_oracle_update_claim,
    oracle_emergency_fallback_choice, oracle_opening_start_ts,
    process_expire_oracle_opening_source, process_finalize_oracle_opening_phase,
    validate_oracle_emergency_choice, DerivedEmergencyPacket,
};
use oracle_schedule_validation::*;
use oracle_state_validation::*;
use oracle_validation::*;
use period::*;
use recipe_weights::*;
use schedule::*;
use settlement_signers::*;
use settlement_sources::*;
use settlement_validation::*;
use sku_coverage::*;
use sku_coverage_resolution::*;
use sku_manifest::*;

use crate::compact_error::cpi::{invoke, invoke_signed};
use crate::ProgramError;
use crate::ProgramResult;
use crate::{
    constants::{
        COLLATERAL_MINT_DECIMALS, CURRENT_STATE_NAMESPACE_SEED,
        EMERGENCY_SETTLEMENT_SIGNER_DELAY_MULTIPLIER, MAX_INSTRUCTION_DATA_BYTES,
        MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS, MAX_ORACLE_REQUIRED_SKUS,
        MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH, MAX_SETTLEMENT_SIGNER_COUNT,
        MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS, ORACLE_ACTIVE_WEIGHT_MANIFEST_HASH_DOMAIN,
        ORACLE_ACTIVE_WEIGHT_MANIFEST_PDA_SEED, ORACLE_BUCKET_MEDIAN_PDA_SEED,
        ORACLE_CANONICAL_RECIPE_HASH_DOMAIN, ORACLE_ECONOMICS_CONFIG_PDA_SEED,
        ORACLE_KILL_WINDOW_SECONDS, ORACLE_MATURITY_LADDER_PDA_SEED, ORACLE_MONTH_PDA_SEED,
        ORACLE_MONTH_ROLL_SECOND_UTC, ORACLE_OPENING_ARCHIVE_URL_HASH_DOMAIN,
        ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES, ORACLE_OPENING_ARCHIVE_URL_PREFIX,
        ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS, ORACLE_OPENING_CLAIM_CHALLENGE_PDA_SEED,
        ORACLE_OPENING_CLAIM_PDA_SEED, ORACLE_OPENING_EVIDENCE_HASH_DOMAIN,
        ORACLE_OPENING_WINDOW_SECONDS, ORACLE_PLACEMENT_WINDOW_SECONDS,
        ORACLE_PRE_LISTING_WINDOW_SECONDS, ORACLE_PRODUCT_SKU_DRAFT_PDA_SEED,
        ORACLE_PRODUCT_SKU_MANIFEST_PDA_SEED, ORACLE_RECIPE_WEIGHT_MANIFEST_HASH_DOMAIN,
        ORACLE_RECIPE_WEIGHT_MANIFEST_PDA_SEED, ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS,
        ORACLE_ROLLING_MATURITY_MONTHS, ORACLE_SETTLEMENT_GRACE_SECONDS,
        ORACLE_SETTLEMENT_SOURCE_DIGEST_DOMAIN, ORACLE_SETTLEMENT_SOURCE_MANIFEST_PDA_SEED,
        ORACLE_SKU_EMPTY_HASH_DOMAIN, ORACLE_SKU_LEAF_HASH_DOMAIN, ORACLE_SKU_NODE_HASH_DOMAIN,
        ORACLE_SOURCE_CHALLENGE_GUARD_PDA_SEED, ORACLE_SOURCE_CHALLENGE_PDA_SEED,
        ORACLE_SOURCE_OBSERVATIONS_PDA_SEED, ORACLE_SOURCE_PDA_SEED,
        ORACLE_SUPPORT_POSITION_PDA_SEED, ORACLE_UPDATE_CHALLENGE_GUARD_PDA_SEED,
        ORACLE_UPDATE_CHALLENGE_PDA_SEED, ORACLE_UPDATE_CLAIM_V2_PDA_SEED,
        ORACLE_UPDATE_COMMITMENT_V2_DOMAIN, ORACLE_UPDATE_EVIDENCE_HASH_DOMAIN,
        ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS, ORACLE_UPDATE_REVEAL_WINDOW_SLOTS,
        PERIOD_VALUE_DECIMALS, SETTLEMENT_SIGNER_REGISTRY_PDA_SEED, SETTLEMENT_SIGNER_SET_PDA_SEED,
        SETTLEMENT_V2_PDA_SEED, USER_COLLATERAL_PDA_SEED, VAULT_PDA_SEED,
    },
    error::OracleError,
    instruction::payloads::*,
    instruction::{
        AccumulateOracleActiveWeightGroupParams, AccumulateOracleRecipeBucketV2Params,
        AccumulateOracleSettlementSourceBucketParams, ActivateVaultV2Params,
        AddOracleUsdcSkuBudgetParams, BeginOracleActiveWeightsParams,
        BeginOracleRecipeWeightsV2Params, BootstrapVaultGovernanceV2Params,
        ChallengeOracleOpeningClaimParams, ChallengeOracleSourceParams,
        ChallengeOracleUpdateClaimParams, ClaimOracleUsdcRewardParams,
        CommitOracleUpdateClaimV2Params, ConfigureOracleEconomicsTemplateV2Params,
        ConfigureOracleProductSkuManifestParams, CreatePeriodParams,
        DepositOracleUsdcRewardsParams, FinalizeOracleUpdateClaimV2Params,
        IndexOracleRecipeSourceV1Params, InitializeOracleMonthV3Params,
        InitializeOracleMonthV5Params, InitializeSettlementSignerRegistryParams,
        OracleInstructionTag, ProposeOracleSourceV3Params, ProposeSettlementSignerRotationParams,
        RecomputeOracleBucketMedianV1Params, ResolveOracleOpeningClaimChallengeParams,
        ResolveOracleSourceChallengeParams, RevealOracleUpdateClaimV3Params,
        RotateVaultAuthoritiesV2Params, SettleOracleEscrowParams, SettlementComputation,
        SettlementLeaf, SubmitOracleOpeningClaimParams, SupportOracleSourceV3Params,
        UpsertSettlementParams,
    },
    state::{
        derive_oracle_active_weight_manifest_pda, derive_oracle_bucket_median_pda,
        derive_oracle_maturity_ladder_registry_pda, derive_oracle_product_sku_draft_pda,
        derive_oracle_product_sku_manifest_pda, derive_oracle_recipe_weight_manifest_pda,
        derive_oracle_sku_coverage_manifest_pda, derive_oracle_sku_coverage_record_pda,
        derive_oracle_source_challenge_guard_pda, derive_oracle_update_challenge_guard_pda,
        derive_oracle_update_claim_v2_pda, derive_settlement_signer_registry_pda,
        derive_settlement_signer_set_pda, derive_settlement_v2_pda, derive_user_collateral_pda,
        OracleActiveWeightManifest, OracleBucketMedianState, OracleBucketMedianStatus,
        OracleChallengeStatus, OracleClaimStatus, OracleConfig, OracleEconomicParams,
        OracleEconomicsConfig, OracleEmergencyDisputeKind, OracleEscrowDisposition,
        OracleEscrowKind, OracleMaturityLadderRegistry, OracleMonthState,
        OracleOpeningChallengeOutcome, OracleOpeningClaim, OracleOpeningClaimChallenge,
        OracleOpeningClaimStatus, OraclePhase, OracleProductSkuDraft, OracleProductSkuManifest,
        OracleRecipeWeightManifest, OracleRecipeWeightPhase, OracleSettlementSourceManifest,
        OracleSettlementStatus, OracleSkuCoverageManifest, OracleSkuCoverageRecord,
        OracleSourceChallenge, OracleSourceChallengeGuard, OracleSourceChallengeOutcome,
        OracleSourceObservations, OracleSourceState, OracleSourceStatus, OracleSupportPosition,
        OracleUpdateChallenge, OracleUpdateChallengeGuard, OracleUpdateClaimData,
        OracleUpdateClaimOutcome, OracleUpdateClaimV2, OracleUsdcRewardKind, SettlementRecordV2,
        SettlementSignerRegistry, SettlementSignerSet, UserCollateral,
    },
    system_instruction, token_instruction,
    token_state::{AccountState, Mint, TokenAccount},
};
use ameba_oracle_interface::{Period, PeriodStatus};
use solana_program::{
    account_info::AccountInfo, hash::hashv, instruction::Instruction, program_option::COption,
    pubkey::Pubkey, sysvar::instructions,
};
use solana_sdk_ids::{bpf_loader_upgradeable, ed25519_program, system_program};

const ORACLE_LOCAL_FILE_ABS_MIN: u64 = 1;
const ORACLE_UPDATE_FILE_PPM: u64 = 25;
const ORACLE_UPDATE_CAP_PPM: u64 = 25_000;
const ORACLE_UPDATE_BASE_BOND: u64 = 1;
/// Two purported sources are not independent (same operator/upstream feed or provably mirrored).
const ORACLE_SOURCE_REASON_NON_INDEPENDENT: u8 = 8;

/// Public processor entry with Solana's error type, for native harnesses and host callers.
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> solana_program::entrypoint::ProgramResult {
    process_instruction_compact(program_id, accounts, instruction_data).map_err(Into::into)
}

/// On-chain entry: the compact error carries the exact code returned to the runtime.
pub(crate) fn process_instruction_compact(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    process_top_level_instruction(program_id, accounts, instruction_data)
}

#[cfg(test)]
mod tests;
