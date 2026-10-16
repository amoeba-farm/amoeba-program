//! Tag 244 (`localnet-dev-authority` only): complete the formation of a month made by tags 240
//! and 241, in place, so it forms and settles through the ordinary instructions from then on.
//!
//! A bootstrapped month is in Game with Active, opened sources but none of the state the ordinary
//! recipe and active-weight steps leave behind. These steps write exactly the part that needs the
//! (already past) formation calendar or candidate support:
//!
//! - **Begin** creates the recipe manifest and the reward schedule and reopens the month's
//!   recipe and active-weight state for building, as tag 187 and the schedule's begin do.
//! - **Bucket** collects one bucket's sources into the recipe chain in ascending id order, as
//!   tag 84 does, without changing them (they stay Active with their openings), and creates the
//!   bucket's SKU pool with the given bonds and no reward budget, as tag 157 does.
//! - **Finalize** freezes the recipe and derives the month's canonical recipe hash, as tag 86
//!   does, but keeps the month in Game with its openings, and funds the zero-budget schedule.
//!
//! The ordinary tags then finish it: 200 indexes the recipe, 121, 122 and 124 build the bucket
//! medians and the active-weight manifest. From then on the month carries the canonical recipe
//! hash, so tags 241 and 242 no longer apply to it, and its updates (166, 198, 199) and settlement
//! (197, 75, 113, 116) run through the ordinary instructions.

use super::dev_bootstrap::{dev_month_marker, is_dev_bootstrapped_source};
use super::*;
use crate::constants::{ORACLE_USDC_REWARD_SCHEDULE_PDA_SEED, ORACLE_USDC_SKU_POOL_PDA_SEED};
use crate::instruction::{DevCompleteFormationParams, DevFormationBucketParams};
use crate::state::{
    derive_oracle_usdc_reward_schedule_pda, derive_oracle_usdc_reward_vault_pda,
    derive_oracle_usdc_sku_pool_pda, OracleUsdcRewardSchedule, OracleUsdcRewardSchedulePhase,
    OracleUsdcSkuPool,
};

#[inline(never)]
pub(super) fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: DevCompleteFormationParams,
) -> ProgramResult {
    match params {
        DevCompleteFormationParams::Begin {
            expected_source_count,
            expected_bucket_count,
        } => begin(
            program_id,
            accounts,
            expected_source_count,
            expected_bucket_count,
        ),
        DevCompleteFormationParams::Bucket(params) => bucket(program_id, accounts, params),
        DevCompleteFormationParams::Finalize => finalize(program_id, accounts),
    }
}

/// A bootstrapped month still in Game and not yet settling.
fn load_bootstrapped_month(
    program_id: &Pubkey,
    period_info: &AccountInfo,
    month_info: &AccountInfo,
) -> Result<(PeriodAnchor, OracleMonthState), ProgramError> {
    let (anchor, month) = load_valid_period_and_oracle_month(program_id, period_info, month_info)?;
    if month.recipe_hash != dev_month_marker(month_info.key)
        || month.phase != OraclePhase::Game
        || month.settlement_record.is_some()
        || month.settlement_status == OracleSettlementStatus::Final
        || month.finalized_at_ts != 0
        || month.pending_resolution_count != 0
    {
        return Err(OracleError::InvalidOraclePhase.into());
    }
    if current_unix_timestamp()? >= anchor.expiry_ts {
        return Err(OracleError::OracleTimingWindowClosed.into());
    }
    Ok((anchor, month))
}

/// Step 0. Accounts: 0 [signer] oracle authority; 1 [signer, writable] payer; 2 config;
/// 3 period; 4 [writable] month; 5 coverage manifest; 6 [writable] recipe manifest PDA;
/// 7 [writable] reward schedule PDA; 8 system program.
#[inline(never)]
fn begin(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    expected_source_count: u16,
    expected_bucket_count: u16,
) -> ProgramResult {
    let [authority_info, payer_info, config_info, period_info, month_info, coverage_info, manifest_info, schedule_info, system_program_info] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    validate_current_oracle_authority_allow_paused(program_id, authority_info, config_info)?;
    validate_current_account_creation_payer(payer_info)?;
    validate_system_program(system_program_info)?;
    let (_anchor, mut month) = load_bootstrapped_month(program_id, period_info, month_info)?;
    // Exactly the state tags 240 and 241 leave: every source opened, weights complete, no update.
    if month.weight_scheme_version != 1
        || month.effective_weight_total_bps != 10_000
        || month.active_weight_scheme_version != OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION
        || month.accepted_cash_update_count != 0
        || expected_source_count == 0
        || expected_source_count != month.source_count
        || expected_source_count != month.frozen_source_count
        || expected_source_count != month.opened_source_count
        || expected_source_count != month.opening_resolved_source_count
        || expected_bucket_count == 0
        || expected_bucket_count > expected_source_count
    {
        return Err(OracleError::InvalidOracleWeightManifest.into());
    }
    // One bucket per required SKU, as tag 187 requires.
    let coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    if !coverage.coverage_finalized
        || coverage.covered_sku_count != coverage.required_sku_count
        || coverage.required_sku_count != expected_bucket_count
    {
        return Err(OracleError::OracleSkuCoverageIncomplete.into());
    }

    let (expected_manifest, manifest_bump) =
        derive_oracle_recipe_weight_manifest_pda(program_id, month_info.key);
    let (expected_schedule, schedule_bump) =
        derive_oracle_usdc_reward_schedule_pda(program_id, month_info.key);
    if *manifest_info.key != expected_manifest || *schedule_info.key != expected_schedule {
        return Err(OracleError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, manifest_info)?;
    validate_create_only_program_account_target(program_id, schedule_info)?;
    create_oracle_manifest_account(
        payer_info,
        manifest_info,
        system_program_info,
        program_id,
        month_info.key.as_ref(),
        OracleRecipeWeightManifest::LEN,
        ORACLE_RECIPE_WEIGHT_MANIFEST_PDA_SEED,
        manifest_bump,
    )?;
    create_program_account(
        payer_info,
        schedule_info,
        system_program_info,
        program_id,
        OracleUsdcRewardSchedule::LEN,
        &[
            ORACLE_USDC_REWARD_SCHEDULE_PDA_SEED,
            month_info.key.as_ref(),
            &[schedule_bump],
        ],
    )?;
    let slot = crate::compact_error::slot()?;
    let manifest = OracleRecipeWeightManifest {
        is_initialized: true,
        bump: manifest_bump,
        account_discriminator: OracleRecipeWeightManifest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleRecipeWeightManifest::ACCOUNT_VERSION,
        month: *month_info.key,
        phase: OracleRecipeWeightPhase::Collecting,
        expected_source_count,
        expected_bucket_count,
        recipe_hash: [0; 32],
        rolling_manifest_hash: initial_oracle_weight_manifest_hash(
            month_info.key,
            expected_source_count,
            expected_bucket_count,
        ),
        ..OracleRecipeWeightManifest::default()
    };
    let schedule = OracleUsdcRewardSchedule {
        is_initialized: true,
        bump: schedule_bump,
        account_discriminator: OracleUsdcRewardSchedule::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUsdcRewardSchedule::ACCOUNT_VERSION,
        month: *month_info.key,
        authority: month.authority,
        reward_vault: derive_oracle_usdc_reward_vault_pda(program_id).0,
        phase: OracleUsdcRewardSchedulePhase::Building,
        sku_pool_count: 0,
        registered_source_count: 0,
        registered_opening_count: 0,
        registered_update_count: 0,
        total_reward_budget: 0,
        remaining_reward_budget: 0,
        last_updated_slot: slot,
        outstanding_prelisting_escrow_count: 0,
        registered_update_reward_units: 0,
        trading_fee_bounty_total: 0,
        bounty_fee_sweep_finalized: true,
    };
    // The recipe is building (as after tag 187) and the active weights have not begun (as before
    // tag 121). The effective weight stays complete, so tag 241 cannot add a source meanwhile.
    month.weight_scheme_version = 255;
    month.weight_manifest_hash = [0; 32];
    month.active_weight_scheme_version = 0;
    month.active_weight_group_count = 0;
    month.active_weight_manifest_hash = [0; 32];
    month.index_delta_bps = 0;
    month.last_updated_slot = slot;
    store_state(manifest_info, &manifest)?;
    store_state(schedule_info, &schedule)?;
    store_oracle_month_state(month_info, &month)
}

/// Step 1. Accounts: 0 [signer] oracle authority; 1 [signer, writable] payer; 2 config;
/// 3 period; 4 month; 5 [writable] recipe manifest; 6 [writable] reward schedule;
/// 7 [writable] the bucket's SKU pool PDA; 8 system program; 9.. one to four of the bucket's
/// sources in ascending id order.
#[inline(never)]
fn bucket(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: DevFormationBucketParams,
) -> ProgramResult {
    if accounts.len() < 10
        || accounts.len() - 9 > crate::constants::MAX_ORACLE_STATE_RECORDS_PER_STEP / 2
    {
        return Err(OracleError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let payer_info = &accounts[1];
    let config_info = &accounts[2];
    let period_info = &accounts[3];
    let month_info = &accounts[4];
    let manifest_info = &accounts[5];
    let schedule_info = &accounts[6];
    let sku_info = &accounts[7];
    let system_program_info = &accounts[8];
    validate_current_oracle_authority_allow_paused(program_id, authority_info, config_info)?;
    validate_current_account_creation_payer(payer_info)?;
    validate_system_program(system_program_info)?;
    if crate::bytes32_is_zero(&params.bucket_id)
        || params.bucket_weight_bps == 0
        || params.bucket_weight_bps > 10_000
        || params.proposer_reward_bps == 0
        || params.proposer_reward_bps > 10_000
        || params.listing_bond == 0
        || params.support_bond == 0
        || params.opening_bond == 0
        || params.update_min_bond == 0
        || params.challenge_min_bond == 0
        || params.challenge_max_bond == 0
        || params.challenge_min_bond > params.challenge_max_bond
        || params.challenge_bond_bps > 10_000
    {
        return Err(OracleError::InvalidOracleUsdcSkuPool.into());
    }
    let (_anchor, month) = load_bootstrapped_month(program_id, period_info, month_info)?;
    let mut manifest =
        load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, manifest_info)?;
    let mut schedule =
        usdc::load_oracle_usdc_reward_schedule(program_id, month_info.key, schedule_info)?;
    if month.weight_scheme_version != 255
        || manifest.phase != OracleRecipeWeightPhase::Collecting
        || schedule.phase != OracleUsdcRewardSchedulePhase::Building
    {
        return Err(OracleError::InvalidOracleWeightManifest.into());
    }

    let slot = crate::compact_error::slot()?;
    if manifest.current_bucket_source_count == 0 {
        if manifest.processed_bucket_count >= manifest.expected_bucket_count
            || (manifest.processed_bucket_count > 0
                && params.bucket_id <= manifest.current_bucket_id)
        {
            return Err(OracleError::InvalidOracleWeightOrder.into());
        }
        manifest.current_bucket_id = params.bucket_id;
        manifest.current_bucket_weight_bps = params.bucket_weight_bps;
        manifest.last_collected_source_id = [0; 32];
        manifest.declared_weight_total_bps = manifest
            .declared_weight_total_bps
            .checked_add(params.bucket_weight_bps)
            .ok_or(OracleError::ArithmeticOverflow)?;
        if manifest.declared_weight_total_bps > 10_000 {
            return Err(OracleError::InvalidOracleWeightManifest.into());
        }
        let (expected_sku, sku_bump) =
            derive_oracle_usdc_sku_pool_pda(program_id, schedule_info.key, &params.bucket_id);
        if *sku_info.key != expected_sku {
            return Err(OracleError::InvalidOracleUsdcSkuPool.into());
        }
        validate_create_only_program_account_target(program_id, sku_info)?;
        create_program_account(
            payer_info,
            sku_info,
            system_program_info,
            program_id,
            OracleUsdcSkuPool::LEN,
            &[
                ORACLE_USDC_SKU_POOL_PDA_SEED,
                schedule_info.key.as_ref(),
                &params.bucket_id,
                &[sku_bump],
            ],
        )?;
        let sku = OracleUsdcSkuPool {
            is_initialized: true,
            bump: sku_bump,
            account_discriminator: OracleUsdcSkuPool::ACCOUNT_DISCRIMINATOR,
            account_version: OracleUsdcSkuPool::ACCOUNT_VERSION,
            schedule: *schedule_info.key,
            month: *month_info.key,
            bucket_id: params.bucket_id,
            source_reward_budget: 0,
            remaining_source_reward_budget: 0,
            opening_reward_budget: 0,
            remaining_opening_reward_budget: 0,
            update_reward_budget: 0,
            remaining_update_reward_budget: 0,
            proposer_reward_bps: params.proposer_reward_bps,
            listing_bond: params.listing_bond,
            support_bond: params.support_bond,
            opening_bond: params.opening_bond,
            update_min_bond: params.update_min_bond,
            challenge_min_bond: params.challenge_min_bond,
            challenge_max_bond: params.challenge_max_bond,
            challenge_bond_bps: params.challenge_bond_bps,
            registered_source_count: 0,
            registered_opening_count: 0,
            registered_update_count: 0,
            last_updated_slot: slot,
            registered_update_reward_units: 0,
        };
        schedule.sku_pool_count = schedule
            .sku_pool_count
            .checked_add(1)
            .ok_or(OracleError::ArithmeticOverflow)?;
        schedule.last_updated_slot = slot;
        store_state(sku_info, &sku)?;
    } else if params.bucket_id != manifest.current_bucket_id
        || params.bucket_weight_bps != manifest.current_bucket_weight_bps
        || *sku_info.key
            != derive_oracle_usdc_sku_pool_pda(program_id, schedule_info.key, &params.bucket_id).0
    {
        return Err(OracleError::InvalidOracleWeightOrder.into());
    }

    for source_info in &accounts[9..] {
        if source_info.is_writable || source_info.is_signer {
            return Err(OracleError::InvalidOracleWeightSource.into());
        }
        let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
        // A bootstrapped source, untouched since tag 241: Active with only its opening.
        if !is_dev_bootstrapped_source(source_info.key, &source)
            || source.status != OracleSourceStatus::Active
            || source.bucket_id != manifest.current_bucket_id
            || source.bucket_weight_bps != manifest.current_bucket_weight_bps
            || source.observation_count != 1
            || source.baseline_state == 0
            || source.current_state != source.baseline_state
        {
            return Err(OracleError::InvalidOracleWeightSource.into());
        }
        if manifest.current_bucket_source_count > 0
            && source.source_id <= manifest.last_collected_source_id
        {
            return Err(OracleError::InvalidOracleWeightOrder.into());
        }
        manifest.current_bucket_source_count = manifest
            .current_bucket_source_count
            .checked_add(1)
            .ok_or(OracleError::ArithmeticOverflow)?;
        if manifest
            .processed_source_count
            .checked_add(manifest.current_bucket_source_count)
            .ok_or(OracleError::ArithmeticOverflow)?
            > manifest.expected_source_count
        {
            return Err(OracleError::InvalidOracleWeightManifest.into());
        }
        manifest.rolling_manifest_hash = advance_oracle_weight_manifest_hash(
            &manifest.rolling_manifest_hash,
            &manifest.current_bucket_id,
            &source,
            manifest.current_bucket_weight_bps,
        );
        manifest.last_collected_source_id = source.source_id;
    }
    if params.finalize_collection {
        manifest.processed_source_count = manifest
            .processed_source_count
            .checked_add(manifest.current_bucket_source_count)
            .ok_or(OracleError::ArithmeticOverflow)?;
        manifest.processed_bucket_count = manifest
            .processed_bucket_count
            .checked_add(1)
            .ok_or(OracleError::ArithmeticOverflow)?;
        if manifest.processed_source_count > manifest.expected_source_count
            || manifest.processed_bucket_count > manifest.expected_bucket_count
        {
            return Err(OracleError::InvalidOracleWeightManifest.into());
        }
        manifest.phase = if manifest.processed_bucket_count == manifest.expected_bucket_count {
            OracleRecipeWeightPhase::ReadyToFinalize
        } else {
            OracleRecipeWeightPhase::Collecting
        };
        manifest.current_bucket_weight_bps = 0;
        manifest.current_bucket_source_count = 0;
        manifest.last_collected_source_id = [0; 32];
    }
    store_state(schedule_info, &schedule)?;
    store_state(manifest_info, &manifest)
}

/// Step 2. Accounts: 0 [signer] oracle authority; 1 config; 2 period; 3 [writable] month;
/// 4 [writable] recipe manifest; 5 [writable] reward schedule.
#[inline(never)]
fn finalize(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let [authority_info, config_info, period_info, month_info, manifest_info, schedule_info] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    validate_current_oracle_authority_allow_paused(program_id, authority_info, config_info)?;
    if !month_info.is_writable || !manifest_info.is_writable || !schedule_info.is_writable {
        return Err(OracleError::InvalidAccountList.into());
    }
    let (_anchor, mut month) = load_bootstrapped_month(program_id, period_info, month_info)?;
    let mut manifest =
        load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, manifest_info)?;
    let mut schedule =
        usdc::load_oracle_usdc_reward_schedule(program_id, month_info.key, schedule_info)?;
    if month.weight_scheme_version != 255
        || manifest.phase != OracleRecipeWeightPhase::ReadyToFinalize
        || manifest.processed_source_count != manifest.expected_source_count
        || manifest.processed_bucket_count != manifest.expected_bucket_count
        || manifest.declared_weight_total_bps != 10_000
        || crate::bytes32_is_zero(&manifest.rolling_manifest_hash)
        || schedule.phase != OracleUsdcRewardSchedulePhase::Building
        || schedule.sku_pool_count != manifest.expected_bucket_count
    {
        return Err(OracleError::OracleWeightManifestIncomplete.into());
    }
    let slot = crate::compact_error::slot()?;
    let recipe_hash = canonical_recipe_digest(month_info.key, &manifest.rolling_manifest_hash);
    manifest.recipe_hash = recipe_hash;
    manifest.phase = OracleRecipeWeightPhase::Finalized;
    month.recipe_hash = recipe_hash;
    month.weight_scheme_version = 1;
    month.effective_weight_total_bps = manifest.declared_weight_total_bps;
    month.weight_manifest_hash = manifest.rolling_manifest_hash;
    month.index_delta_bps = 0;
    month.last_updated_slot = slot;
    schedule.phase = OracleUsdcRewardSchedulePhase::Funded;
    schedule.remaining_reward_budget = schedule.total_reward_budget;
    schedule.last_updated_slot = slot;
    store_state(manifest_info, &manifest)?;
    store_state(schedule_info, &schedule)?;
    store_oracle_month_state(month_info, &month)
}
