mod account_addresses;
mod account_io;
mod account_io_tokens;
mod active_weights;
mod ameba_dlmm;
mod ameba_dlmm_light;
mod bucket_medians;
mod collateral_accounts;
mod compressed_state;
#[cfg(feature = "devnet-solo-backfill-2026")]
mod devnet_solo_backfill_2026;
mod instruction_adapters;
mod instruction_dispatch;
mod instruction_payloads;
mod market_admin;
mod market_settlement;
mod median;
mod oracle_economics;
mod oracle_rules;
mod oracle_samba_pot;
mod oracle_schedule_validation;
mod oracle_state_validation;
mod oracle_usdc;
mod oracle_usdc_rewards;
mod oracle_validation;
mod recipe_weights;
mod settlement_signers;
mod settlement_sources;
mod settlement_validation;
mod sku_coverage;
mod sku_coverage_resolution;
mod sku_manifest;
mod staking;
mod staking_lifecycle;
mod staking_validation;
mod vault_core;
mod writer_sleeve;

use account_addresses::*;
use account_io::*;
use account_io_tokens::*;
use active_weights::*;
use bucket_medians::*;
use collateral_accounts::*;
use instruction_adapters::*;
use instruction_dispatch::*;
use instruction_payloads::*;
use market_admin::*;
use market_settlement::*;
use median::{
    advance_oracle_bucket_source_snapshot, append_oracle_source_observation,
    initial_oracle_bucket_source_snapshot, oracle_bucket_bounty_share,
    update_month_bucket_contribution, validate_oracle_observation_shape,
};
pub use median::{
    bucket_index_contribution_bps, deterministic_bucket_median,
    minimum_oracle_bucket_eligible_sources, oracle_bucket_security_cap,
    oracle_settlement_window_start, oracle_temporal_median_state,
};
use oracle_economics::*;
pub use oracle_rules::{
    advance_oracle_settlement_source_digest, advance_oracle_weight_manifest_hash,
    canonical_recipe_digest, initial_oracle_settlement_source_digest,
    initial_oracle_weight_manifest_hash, oracle_update_claim_required_bond,
    oracle_update_claim_v2_commitment_hash, settle_expired_oracle_update_commitment_bond,
    validate_active_group_collection_completion, validate_active_group_source_identity,
    validate_canonical_settlement_provenance, validate_oracle_active_manifest_begin_membership,
    validate_oracle_active_manifest_completion, validate_oracle_update_claim_v2_commit_bond,
    validate_oracle_update_claim_v2_reveal,
};
use oracle_rules::{
    apply_oracle_source_merge, ensure_live_revealed_oracle_update_claim,
    oracle_emergency_case_hash, oracle_emergency_fallback_choice, oracle_opening_start_ts,
    process_expire_oracle_opening_source, process_finalize_oracle_opening_phase,
    validate_oracle_emergency_choice, DerivedEmergencyPacket,
};
use oracle_schedule_validation::*;
use oracle_state_validation::*;
use oracle_validation::*;
use recipe_weights::*;
use settlement_signers::*;
use settlement_sources::*;
use settlement_validation::*;
use sku_coverage::*;
use sku_coverage_resolution::*;
use sku_manifest::*;
use staking::*;
use staking_lifecycle::*;
use staking_validation::*;
use vault_core::*;

use crate::{
    compression::{
        apply_market_page_leaf_mutations, apply_settlement_leaf_mutations, MarketPageLeafCreate,
        MarketPageLeafUpdate, SettlementLeafCreate,
    },
    constants::{
        CONTRACT_MINT_PDA_SEED, CONTRACT_MINT_STAGING_PDA_SEED, CURRENT_STATE_NAMESPACE_SEED,
        EMERGENCY_SETTLEMENT_SIGNER_DELAY_MULTIPLIER, MARKET_PDA_SEED, MAX_INSTRUCTION_DATA_BYTES,
        MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS, MAX_ORACLE_REQUIRED_SKUS,
        MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH, MAX_SETTLEMENT_SIGNER_COUNT,
        MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS, NANDX_ORACLE_PRODUCT_SKU_COUNT,
        ORACLE_ACTIVE_WEIGHT_MANIFEST_HASH_DOMAIN, ORACLE_ACTIVE_WEIGHT_MANIFEST_PDA_SEED,
        ORACLE_BUCKET_MEDIAN_PDA_SEED, ORACLE_CANONICAL_RECIPE_HASH_DOMAIN,
        ORACLE_ECONOMICS_CONFIG_PDA_SEED, ORACLE_KILL_WINDOW_SECONDS,
        ORACLE_MAJOR_TOKEN_CONFIG_PDA_SEED, ORACLE_MATURITY_LADDER_PDA_SEED, ORACLE_MONTH_PDA_SEED,
        ORACLE_MONTH_ROLL_SECOND_UTC, ORACLE_OPENING_ARCHIVE_URL_HASH_DOMAIN,
        ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES, ORACLE_OPENING_ARCHIVE_URL_PREFIX,
        ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS, ORACLE_OPENING_CLAIM_CHALLENGE_PDA_SEED,
        ORACLE_OPENING_CLAIM_PDA_SEED, ORACLE_OPENING_EVIDENCE_HASH_DOMAIN,
        ORACLE_OPENING_WINDOW_SECONDS, ORACLE_PLACEMENT_WINDOW_SECONDS,
        ORACLE_PLAYER_LEDGER_PDA_SEED, ORACLE_PRE_LISTING_WINDOW_SECONDS,
        ORACLE_PRODUCT_SKU_DRAFT_PDA_SEED, ORACLE_PRODUCT_SKU_MANIFEST_PDA_SEED,
        ORACLE_RECIPE_WEIGHT_MANIFEST_HASH_DOMAIN, ORACLE_RECIPE_WEIGHT_MANIFEST_PDA_SEED,
        ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS, ORACLE_REWARD_CHALLENGE_BPS,
        ORACLE_REWARD_FUNNEL_PDA_SEED, ORACLE_REWARD_GAME_BPS, ORACLE_REWARD_SCRAMBLE_BPS,
        ORACLE_REWARD_STAKING_BPS, ORACLE_ROLLING_MATURITY_MONTHS, ORACLE_SAMBA_MINT_PDA_SEED,
        ORACLE_SAMBA_STAKE_ACTIVATION_SECONDS, ORACLE_SAMBA_UNBONDING_SECONDS,
        ORACLE_SAMBA_VOTE_VAULT_PDA_SEED, ORACLE_SCRAMBLE_WINDOW_SECONDS,
        ORACLE_SETTLEMENT_GRACE_SECONDS, ORACLE_SETTLEMENT_SOURCE_DIGEST_DOMAIN,
        ORACLE_SETTLEMENT_SOURCE_MANIFEST_PDA_SEED, ORACLE_SKU_EMPTY_HASH_DOMAIN,
        ORACLE_SKU_LEAF_HASH_DOMAIN, ORACLE_SKU_NODE_HASH_DOMAIN,
        ORACLE_SOURCE_CHALLENGE_GUARD_PDA_SEED, ORACLE_SOURCE_CHALLENGE_PDA_SEED,
        ORACLE_SOURCE_OBSERVATIONS_PDA_SEED, ORACLE_SOURCE_PDA_SEED,
        ORACLE_STAKE_ACTIVATION_PDA_SEED, ORACLE_STAKING_POOL_PDA_SEED,
        ORACLE_SUPPORT_POSITION_PDA_SEED, ORACLE_TREASURY_PDA_SEED,
        ORACLE_UNSTAKE_REQUEST_PDA_SEED, ORACLE_UPDATE_CHALLENGE_GUARD_PDA_SEED,
        ORACLE_UPDATE_CHALLENGE_PDA_SEED, ORACLE_UPDATE_CLAIM_V2_PDA_SEED,
        ORACLE_UPDATE_COMMITMENT_V2_DOMAIN, ORACLE_UPDATE_EVIDENCE_HASH_DOMAIN,
        ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS, ORACLE_UPDATE_REVEAL_WINDOW_SLOTS,
        RAMX_ORACLE_PRODUCT_SKU_COUNT, SETTLEMENT_SIGNER_REGISTRY_PDA_SEED,
        SETTLEMENT_SIGNER_SET_PDA_SEED, SETTLEMENT_V2_PDA_SEED, USER_COLLATERAL_PDA_SEED,
        VAULT_CONFIG_BOOTSTRAP_AUTHORITY, VAULT_PDA_SEED,
    },
    error::VaultError,
    fixed_codec::deserialize_slice_bytes,
    instruction::{
        deserialize_compressed_account_meta, deserialize_compression_output,
        deserialize_validity_proof, AccumulateOracleActiveWeightGroupParams,
        AccumulateOracleRecipeBucketV2Params, AccumulateOracleSettlementSourceBucketParams,
        ActivateQueuedStakeAmbaForSambaParams, ActivateVaultV2Params, AddOracleUsdcSkuBudgetParams,
        BeginOracleActiveWeightsParams, BeginOracleRecipeWeightsV2Params,
        BootstrapVaultGovernanceV2Params, ChallengeOracleOpeningClaimParams,
        ChallengeOracleSourceParams, ChallengeOracleUpdateClaimParams, ClaimOracleUsdcRewardParams,
        CommitOracleEmergencyVoteV2Params, CommitOracleUpdateClaimV2Params,
        CompressedMarketPageWitness, CompressedSettlementWitness, CompressionOutput,
        ConfigureOracleEconomicsTemplateV2Params, ConfigureOracleProductSkuManifestParams,
        DepositOracleMajorTokensParams, DepositOracleUsdcRewardsParams,
        ExecuteCompressedStateParams, FinalizeOracleUpdateClaimV2Params, InitMarketV2Params,
        InitializeOracleMonthV3Params, InitializeOracleMonthV5Params,
        InitializeSettlementSignerRegistryParams, ProposeOracleSourceV3Params,
        ProposeSettlementSignerRotationParams, QueueStakeAmbaForSambaParams,
        RecomputeOracleBucketMedianV1Params, RequestUnstakeSambaParams,
        ResolveOracleOpeningClaimChallengeParams, ResolveOracleSourceChallengeParams,
        RevealOracleEmergencyVoteParams, RevealOracleUpdateClaimV3Params,
        RotateVaultAuthoritiesV2Params, SetMarketPausedParams, SettleOracleEscrowParams,
        SubmitOracleOpeningClaimParams, SupportOracleSourceV3Params,
        TryOpenOracleEmergencyDisputeParams, UpsertMarketPageParams, UpsertSettlementParams,
        VaultInstructionTag, WithdrawOracleMajorTokensParams,
    },
    light_token_instruction::{
        self, cpi_authority, has_canonical_compressible_token_layout, light_token_program_id,
    },
    state::{
        derive_oracle_active_weight_manifest_pda, derive_oracle_bucket_median_pda,
        derive_oracle_maturity_ladder_registry_pda, derive_oracle_product_sku_draft_pda,
        derive_oracle_product_sku_manifest_pda, derive_oracle_recipe_weight_manifest_pda,
        derive_oracle_sku_coverage_manifest_pda, derive_oracle_sku_coverage_record_pda,
        derive_oracle_source_challenge_guard_pda, derive_oracle_update_challenge_guard_pda,
        derive_oracle_update_claim_v2_pda, derive_settlement_signer_registry_pda,
        derive_settlement_signer_set_pda, CompressedMarketPageLeaf, CompressedSettlementLeaf,
        Market, MarketMintAccounting, OracleActiveWeightManifest, OracleBucketMedianState,
        OracleBucketMedianStatus, OracleChallengeStatus, OracleClaimStatus, OracleEconomicParams,
        OracleEconomicsConfig, OracleEmergencyDisputeKind, OracleEmergencyVoteStatus,
        OracleEscrowDisposition, OracleEscrowKind, OracleMajorTokenConfig,
        OracleMaturityLadderRegistry, OracleMonthState, OracleOpeningChallengeOutcome,
        OracleOpeningClaim, OracleOpeningClaimChallenge, OracleOpeningClaimStatus, OraclePhase,
        OraclePlayerLedger, OracleProductSkuDraft, OracleProductSkuManifest,
        OracleRecipeWeightManifest, OracleRecipeWeightPhase, OracleRewardFunnel,
        OracleSettlementSourceManifest, OracleSettlementStatus, OracleSkuCoverageManifest,
        OracleSkuCoverageRecord, OracleSourceChallenge, OracleSourceChallengeGuard,
        OracleSourceChallengeOutcome, OracleSourceObservations, OracleSourceState,
        OracleSourceStatus, OracleStakeActivation, OracleStakingPool, OracleSupportPosition,
        OracleTreasuryState, OracleUnstakeRequest, OracleUpdateChallenge,
        OracleUpdateChallengeGuard, OracleUpdateClaimData, OracleUpdateClaimOutcome,
        OracleUpdateClaimV2, OracleUsdcRewardKind, OracleUsdcRewardSchedulePhase,
        SettlementComputation, SettlementRecordV2, SettlementSignerRegistry, SettlementSignerSet,
        UserCollateral, VaultConfig,
    },
    system_instruction, token_instruction,
    token_state::{AccountState, Mint, TokenAccount},
};
use borsh::BorshDeserialize;
use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    hash::hashv,
    instruction::Instruction,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    program_option::COption,
    pubkey::Pubkey,
    rent::Rent,
    sysvar::{clock::Clock, instructions, Sysvar},
};
use solana_sdk_ids::{ed25519_program, system_program};
use token_instruction::id as spl_token_program_id;

const ORACLE_LOCAL_FILE_ABS_MIN: u64 = 1;
const ORACLE_UPDATE_FILE_PPM: u64 = 25;
const ORACLE_UPDATE_CAP_PPM: u64 = 25_000;
const ORACLE_UPDATE_BASE_BOND: u64 = 1;
/// Two purported sources are not independent (same operator/upstream feed or provably mirrored).
const ORACLE_SOURCE_REASON_NON_INDEPENDENT: u8 = 8;
/// One creator-fee contribution split into the protocol's shared reward destinations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OracleRewardAllocation {
    pub game: u64,
    pub scramble: u64,
    pub challenge: u64,
    pub staking: u64,
    pub reserve: u64,
}

impl OracleRewardAllocation {
    fn total(self) -> Result<u64, ProgramError> {
        self.game
            .checked_add(self.scramble)
            .and_then(|value| value.checked_add(self.challenge))
            .and_then(|value| value.checked_add(self.staking))
            .and_then(|value| value.checked_add(self.reserve))
            .ok_or(VaultError::ArithmeticOverflow.into())
    }
}

/// Apply the immutable 45/20/15/15/5 creator-fee split. All floor-division dust belongs to
/// Reserve. If there is no live sAMBA generation, its 15% share also remains in Reserve instead
/// of becoming backing that a future first staker could capture.
pub fn calculate_oracle_reward_allocation(
    amount: u64,
    staking_active: bool,
) -> Result<OracleRewardAllocation, ProgramError> {
    let game = mul_bps(amount, ORACLE_REWARD_GAME_BPS)?;
    let scramble = mul_bps(amount, ORACLE_REWARD_SCRAMBLE_BPS)?;
    let challenge = mul_bps(amount, ORACLE_REWARD_CHALLENGE_BPS)?;
    let candidate_staking = mul_bps(amount, ORACLE_REWARD_STAKING_BPS)?;
    let allocated_before_reserve = game
        .checked_add(scramble)
        .and_then(|value| value.checked_add(challenge))
        .and_then(|value| value.checked_add(candidate_staking))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let base_reserve = amount
        .checked_sub(allocated_before_reserve)
        .ok_or(VaultError::InvalidOracleRewardFunnel)?;
    let (staking, reserve) = if staking_active {
        (candidate_staking, base_reserve)
    } else {
        (
            0,
            base_reserve
                .checked_add(candidate_staking)
                .ok_or(VaultError::ArithmeticOverflow)?,
        )
    };
    let allocation = OracleRewardAllocation {
        game,
        scramble,
        challenge,
        staking,
        reserve,
    };
    if allocation.total()? != amount {
        return Err(VaultError::InvalidOracleRewardFunnel.into());
    }
    Ok(allocation)
}

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    process_instruction_with_context(program_id, accounts, instruction_data, false)
}

pub(super) fn process_compressed_inner_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    process_instruction_with_context(program_id, accounts, instruction_data, true)
}

/// Native ProgramTest adapter for exercising the unchanged business handlers
/// against their full account views. The production SBF artifact never exports
/// this entrypoint; on chain, compressed families remain reachable only through
/// the authenticated compressed-state transport.
#[cfg(not(target_os = "solana"))]
#[doc(hidden)]
pub fn process_instruction_with_classic_compression_views_for_tests(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    process_instruction_with_context(program_id, accounts, instruction_data, true)
}
