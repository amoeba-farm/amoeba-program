//! Tag 130: the council-approved CFM launch of a product's first month (`crate::cfm`).
//!
//! The user decided on 2026-10-15 that the first mainnet launch month forms "only for the first
//! mainnet launch month, like the old Sep/Oct one": Spread's CFM council bootstrap, ported. The
//! receipt PDA is keyed by product and created once, so each product launches this way once per
//! oracle deployment; later months form through the ordinary instructions.
//!
//! - **Begin** (three current council seats sign): the receipt, the month in Opening with its
//!   listing at the later of the planned listing and now and its finalized coverage over the
//!   registry's parent rows, the recipe and active-weight manifests (collecting) and the
//!   zero-budget reward schedule.
//! - **Append** (permissionless, one registry row per call, in registry order): the row's source,
//!   Active with the plan's print as its opening and first observation (journal and checkpoint
//!   through the carry writer), its SKU pool on the plan's terms and its bucket median, Live, as
//!   tag 122 leaves a one-source bucket. The last row freezes the recipe (as tag 86), finalizes
//!   the active weights (as tag 124) and the schedule, moves the month to Game and writes the
//!   period Live through the writer tag 80 uses.
//! - The ordinary tag 200 then indexes the recipe, as settlement (197, 113) needs.
//!
//! Every account is the ordinary layout of a month formed by the ordinary instructions; only the
//! receipt is new. The council's bucket settlement of a launched month may apply as soon as three
//! current seats agree (`council`, operation 2, with the finished receipt).

use super::*;
use crate::cfm::{self, CfmLaunchReceipt, CfmPlanView, CfmProduct, CfmRegistry, RegistryRow};
use crate::constants::{
    ORACLE_SKU_COVERAGE_MANIFEST_PDA_SEED, ORACLE_USDC_REWARD_SCHEDULE_PDA_SEED,
    ORACLE_USDC_SKU_POOL_PDA_SEED,
};
use crate::instruction::{CfmLaunchV1Params, CFM_LAUNCH_APPEND, CFM_LAUNCH_BEGIN};
use crate::state::{
    derive_oracle_usdc_reward_schedule_pda, derive_oracle_usdc_reward_vault_pda,
    derive_oracle_usdc_sku_pool_pda, OracleUsdcRewardSchedule, OracleUsdcRewardSchedulePhase,
    OracleUsdcSkuPool,
};

/// The month's weight scheme while the launch collects its rows (as after tag 187).
const COLLECTING_WEIGHT_SCHEME: u8 = 255;

fn bad_receipt<T>() -> Result<T, ProgramError> {
    Err(OracleError::InvalidCfmLaunchReceipt.into())
}

fn bad_plan<T>() -> Result<T, ProgramError> {
    Err(OracleError::InvalidCfmLaunchPlan.into())
}

#[inline(never)]
pub(super) fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: CfmLaunchV1Params,
) -> ProgramResult {
    let expected = match params.operation {
        CFM_LAUNCH_BEGIN if params.row == 0 => 17,
        CFM_LAUNCH_APPEND => 16,
        _ => return Err(OracleError::InvalidInstructionData.into()),
    };
    if accounts.len() != expected {
        return Err(OracleError::InvalidAccountList.into());
    }
    // No two roles share an account, absent PDAs and the payer included.
    for (index, info) in accounts.iter().enumerate() {
        if accounts[..index]
            .iter()
            .any(|prior| crate::pubkey_eq(prior.key, info.key))
        {
            return Err(OracleError::InvalidAccountList.into());
        }
    }
    validate_current_account_creation_payer(&accounts[0])?;
    validate_system_program(&accounts[expected - 1])?;
    let product = CfmProduct::by_code(params.product)?;
    if params.operation == CFM_LAUNCH_BEGIN {
        begin(program_id, accounts, &product, &params.plan_hash)
    } else {
        append(
            program_id,
            accounts,
            &product,
            params.row,
            &params.plan_hash,
        )
    }
}

/// The bytes after `domain` (and the program id, when `bound`) of a sealed kind-2 evidence
/// object committing to `commitment`.
#[inline(never)]
fn sealed_bytes(
    program_id: &Pubkey,
    info: &AccountInfo,
    commitment: &[u8; 32],
    domain: &[u8],
    bound: bool,
) -> Result<Vec<u8>, ProgramError> {
    oracle_evidence::with_sealed_definition_preimage(program_id, info, commitment, |preimage| {
        let rest = preimage
            .strip_prefix(domain)
            .ok_or(OracleError::InvalidCfmLaunchPlan)?;
        let rest = if bound {
            rest.strip_prefix(program_id.as_ref())
                .ok_or(OracleError::InvalidCfmLaunchPlan)?
        } else {
            rest
        };
        Ok(rest.to_vec())
    })
    .map_err(|_| ProgramError::from(OracleError::InvalidCfmLaunchPlan))
}

/// The council-approved plan and the product's pinned registry, from their sealed objects.
#[inline(never)]
fn load_plan_and_registry(
    program_id: &Pubkey,
    plan_info: &AccountInfo,
    registry_info: &AccountInfo,
    plan_hash: &[u8; 32],
    product: &CfmProduct,
    expiry_ts: u64,
    now: u64,
) -> Result<(CfmPlanView, CfmRegistry), ProgramError> {
    let plan = CfmPlanView::decode(
        sealed_bytes(program_id, plan_info, plan_hash, cfm::PLAN_DOMAIN, true)?,
        now,
    )?;
    let registry = CfmRegistry::authenticated(
        *product,
        sealed_bytes(
            program_id,
            registry_info,
            &product.registry_hash,
            cfm::REGISTRY_DOMAIN,
            false,
        )?,
    )?;
    if plan.product() != *product || plan.expiry_ts() != expiry_ts {
        return bad_plan();
    }
    Ok((plan, registry))
}

fn load_receipt(
    program_id: &Pubkey,
    info: &AccountInfo,
    product: &CfmProduct,
) -> Result<CfmLaunchReceipt, ProgramError> {
    let receipt: CfmLaunchReceipt = load_exact_zero_padded_state(
        info,
        program_id,
        CfmLaunchReceipt::LEN,
        OracleError::InvalidCfmLaunchReceipt,
    )?;
    let (key, bump) = cfm::find_receipt_address(program_id, &product.product_id);
    if info.executable
        || *info.key != key
        || receipt.discriminator != cfm::RECEIPT_DISCRIMINATOR
        || receipt.version != cfm::RECEIPT_VERSION
        || receipt.bump != bump
        || receipt.product != product.code
        || receipt.row_count != product.parent_count
        || receipt.cursor > receipt.row_count
        || receipt.finished != (receipt.finished_at != 0)
        || (receipt.finished && receipt.cursor != receipt.row_count)
        || receipt.approving_seats.count_ones() != 3
        || receipt.approving_seats & !0b1_1111 != 0
        || receipt.council_epoch == 0
        || receipt.begun_at == 0
        || crate::bytes32_is_zero(&receipt.plan_hash)
    {
        return bad_receipt();
    }
    Ok(receipt)
}

/// The council's bucket settlement of a launched month may apply early only with this: the
/// finished receipt of the period's product, naming this period and month.
pub(super) fn validate_finished_launch(
    program_id: &Pubkey,
    period_info: &AccountInfo,
    month_key: &Pubkey,
    receipt_info: &AccountInfo,
) -> ProgramResult {
    if receipt_info.is_signer || receipt_info.is_writable {
        return Err(OracleError::InvalidAccountList.into());
    }
    let anchor = load_valid_period_anchor(program_id, period_info)?;
    let product = if anchor.underlying_id == cfm::RAMX.product_id {
        cfm::RAMX
    } else if anchor.underlying_id == cfm::NANDX.product_id {
        cfm::NANDX
    } else {
        return bad_receipt();
    };
    let receipt = load_receipt(program_id, receipt_info, &product)?;
    if !receipt.finished || receipt.period != *period_info.key || receipt.month != *month_key {
        return bad_receipt();
    }
    Ok(())
}

/// A registry row as the month's source before its opening (bucket id = source id).
fn registry_source(row: &RegistryRow, month: &Pubkey, proposer: &Pubkey) -> OracleSourceState {
    OracleSourceState {
        month: *month,
        source_id: row.source_id,
        bucket_id: row.source_id,
        source_type_hash: cfm::SOURCE_TYPE_HASH,
        canonical_locator_hash: row.locator_hash,
        source_definition_hash: row.definition_hash,
        proposer: *proposer,
        bucket_weight_bps: row.weight_bps,
        ..OracleSourceState::default()
    }
}

/// The frozen recipe's weight-manifest hash and canonical recipe hash over every registry row.
#[inline(never)]
fn recipe_hashes(
    month: &Pubkey,
    registry: &CfmRegistry,
) -> Result<([u8; 32], [u8; 32]), ProgramError> {
    let count = u16::from(registry.len());
    let mut hash = initial_oracle_weight_manifest_hash(month, count, count);
    for index in 0..registry.len() {
        let row = registry.row(index)?;
        let source = registry_source(&row, month, &Pubkey::default());
        hash = advance_oracle_weight_manifest_hash(&hash, &row.source_id, &source, row.weight_bps);
    }
    Ok((hash, canonical_recipe_digest(month, &hash)))
}

/// Step 0. Accounts: see `instruction::cfm_launch_begin`.
#[inline(never)]
fn begin(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    product: &CfmProduct,
    plan_hash: &[u8; 32],
) -> ProgramResult {
    let [payer_info, period_info, month_info, coverage_info, receipt_info, recipe_info, active_info, schedule_info, config_info, economics_info, plan_info, registry_info, council_info, first_seat, second_seat, third_seat, system_program_info] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    // Three distinct current seats of the council Config (the governance controller's, read
    // through the same adapter as the council's dispute cases).
    let view = council::council(council_info)?;
    let mut mask = 0u8;
    for seat in [first_seat, second_seat, third_seat] {
        let index = view
            .seats
            .iter()
            .position(|key| key == seat.key)
            .filter(|_| seat.is_signer)
            .ok_or(OracleError::CfmLaunchCouncilRequired)?;
        mask |= 1 << index;
    }
    let anchor = load_valid_period_anchor(program_id, period_info)?;
    if anchor.period.status != PeriodStatus::Forming
        || anchor.period.effective_listing_ts != 0
        || !period_info.is_writable
    {
        return Err(OracleError::InvalidPeriodTransition.into());
    }
    if anchor.underlying_id != product.product_id {
        return bad_plan();
    }
    let (month_key, month_bump) =
        derive_oracle_month_pda(program_id, period_info.key, anchor.expiry_ts);
    let (receipt_key, receipt_bump) = cfm::find_receipt_address(program_id, &product.product_id);
    let (coverage_key, coverage_bump) =
        derive_oracle_sku_coverage_manifest_pda(program_id, &month_key);
    let (recipe_key, recipe_bump) =
        derive_oracle_recipe_weight_manifest_pda(program_id, &month_key);
    let (active_key, active_bump) =
        derive_oracle_active_weight_manifest_pda(program_id, &month_key);
    let (schedule_key, schedule_bump) =
        derive_oracle_usdc_reward_schedule_pda(program_id, &month_key);
    let expiry_seed = anchor.expiry_ts.to_le_bytes();
    let bumps = [
        [receipt_bump],
        [month_bump],
        [coverage_bump],
        [recipe_bump],
        [active_bump],
        [schedule_bump],
    ];
    // The receipt comes first: an existing receipt is the one-shot rule's AlreadyInitialized.
    let targets: [(&AccountInfo, &Pubkey, usize, [&[u8]; 4]); 6] = [
        (
            receipt_info,
            &receipt_key,
            CfmLaunchReceipt::LEN,
            [cfm::RECEIPT_SEED, &product.product_id, &bumps[0], &[]],
        ),
        (
            month_info,
            &month_key,
            OracleMonthState::LEN,
            [
                ORACLE_MONTH_PDA_SEED,
                period_info.key.as_ref(),
                &expiry_seed,
                &bumps[1],
            ],
        ),
        (
            coverage_info,
            &coverage_key,
            OracleSkuCoverageManifest::LEN,
            [
                ORACLE_SKU_COVERAGE_MANIFEST_PDA_SEED,
                month_key.as_ref(),
                &bumps[2],
                &[],
            ],
        ),
        (
            recipe_info,
            &recipe_key,
            OracleRecipeWeightManifest::LEN,
            [
                ORACLE_RECIPE_WEIGHT_MANIFEST_PDA_SEED,
                month_key.as_ref(),
                &bumps[3],
                &[],
            ],
        ),
        (
            active_info,
            &active_key,
            OracleActiveWeightManifest::LEN,
            [
                ORACLE_ACTIVE_WEIGHT_MANIFEST_PDA_SEED,
                month_key.as_ref(),
                &bumps[4],
                &[],
            ],
        ),
        (
            schedule_info,
            &schedule_key,
            OracleUsdcRewardSchedule::LEN,
            [
                ORACLE_USDC_REWARD_SCHEDULE_PDA_SEED,
                month_key.as_ref(),
                &bumps[5],
                &[],
            ],
        ),
    ];
    for (info, key, _, _) in &targets {
        if info.key != *key {
            return Err(OracleError::InvalidPda.into());
        }
        validate_create_only_program_account_target(program_id, info)?;
    }

    let config = load_current_canonical_vault_config(program_id, config_info)?;
    let economics = load_canonical_oracle_economics_config(program_id, economics_info)?;
    let (slot, now) = current_slot_and_unix_timestamp()?;
    let listing_ts = anchor.period.planned_listing_ts.max(now);
    if listing_ts >= anchor.expiry_ts {
        return Err(OracleError::OracleTimingWindowClosed.into());
    }
    let scramble_start_ts = listing_ts
        .checked_sub(ORACLE_PRE_LISTING_WINDOW_SECONDS)
        .ok_or(OracleError::ArithmeticOverflow)?;
    let (plan, registry) = load_plan_and_registry(
        program_id,
        plan_info,
        registry_info,
        plan_hash,
        product,
        anchor.expiry_ts,
        now,
    )?;
    let (manifest_hash, recipe_hash) = recipe_hashes(&month_key, &registry)?;
    for (info, _, len, seeds) in &targets {
        let used = if seeds[3].is_empty() { 3 } else { 4 };
        create_program_account(
            payer_info,
            info,
            system_program_info,
            program_id,
            *len,
            &seeds[..used],
        )?;
    }

    let count = u16::from(product.parent_count);
    let month = Box::new(OracleMonthState {
        is_initialized: true,
        bump: month_bump,
        account_discriminator: OracleMonthState::ACCOUNT_DISCRIMINATOR,
        account_version: OracleMonthState::ACCOUNT_VERSION,
        period: *period_info.key,
        authority: config.oracle_authority,
        scramble_start_ts,
        listing_ts,
        phase: OraclePhase::Opening,
        weight_scheme_version: COLLECTING_WEIGHT_SCHEME,
        last_updated_slot: slot,
        settlement_base_oracle_atomic: plan.settlement_base_oracle_atomic(),
        economics: economics.economics,
        candidate_count_tracking_version: OracleMonthState::CANDIDATE_COUNT_TRACKING_VERSION,
        active_weight_initialization_version:
            OracleMonthState::ACTIVE_WEIGHT_INITIALIZATION_VERSION,
        schedule_version: OracleMonthState::SKU_COVERAGE_SCHEDULE_VERSION,
        work_reward_currency_version: OracleMonthState::WORK_REWARD_CURRENCY_USDC_V1,
        ..OracleMonthState::default()
    });
    rulebook_schedule_boundaries(&month)?;
    store_state(
        receipt_info,
        &CfmLaunchReceipt {
            discriminator: cfm::RECEIPT_DISCRIMINATOR,
            version: cfm::RECEIPT_VERSION,
            bump: receipt_bump,
            product: product.code,
            row_count: product.parent_count,
            cursor: 0,
            finished: false,
            period: *period_info.key,
            month: month_key,
            plan_hash: *plan_hash,
            registry_hash: product.registry_hash,
            document_sha256: plan.document_sha256(),
            proposer: *payer_info.key,
            council_epoch: view.epoch,
            seats_hash: view.digest,
            approving_seats: mask,
            begun_at: now,
            finished_at: 0,
        },
    )?;
    store_state(
        coverage_info,
        &OracleSkuCoverageManifest {
            is_initialized: true,
            bump: coverage_bump,
            account_discriminator: OracleSkuCoverageManifest::ACCOUNT_DISCRIMINATOR,
            account_version: OracleSkuCoverageManifest::ACCOUNT_VERSION,
            month: month_key,
            required_sku_root: product.coverage_root,
            required_sku_count: count,
            covered_sku_count: count,
            planned_scramble_start_ts: scramble_start_ts,
            planned_listing_ts: listing_ts,
            coverage_finalized: true,
            coverage_complete_ts: now,
            last_updated_slot: slot,
        },
    )?;
    store_state(
        recipe_info,
        &OracleRecipeWeightManifest {
            is_initialized: true,
            bump: recipe_bump,
            account_discriminator: OracleRecipeWeightManifest::ACCOUNT_DISCRIMINATOR,
            account_version: OracleRecipeWeightManifest::ACCOUNT_VERSION,
            month: month_key,
            phase: OracleRecipeWeightPhase::Collecting,
            expected_source_count: count,
            expected_bucket_count: count,
            rolling_manifest_hash: initial_oracle_weight_manifest_hash(&month_key, count, count),
            ..OracleRecipeWeightManifest::default()
        },
    )?;
    store_state(
        active_info,
        &OracleActiveWeightManifest {
            is_initialized: true,
            bump: active_bump,
            account_discriminator: OracleActiveWeightManifest::ACCOUNT_DISCRIMINATOR,
            account_version: OracleActiveWeightManifest::ACCOUNT_VERSION,
            month: month_key,
            phase: OracleRecipeWeightPhase::Collecting,
            expected_source_count: count,
            expected_group_count: count,
            max_open_interest_payout: u64::MAX,
            rolling_manifest_hash: initial_oracle_active_weight_hash(
                &month_key,
                &recipe_hash,
                &manifest_hash,
                count,
                count,
            ),
            ..OracleActiveWeightManifest::default()
        },
    )?;
    store_state(
        schedule_info,
        &OracleUsdcRewardSchedule {
            is_initialized: true,
            bump: schedule_bump,
            account_discriminator: OracleUsdcRewardSchedule::ACCOUNT_DISCRIMINATOR,
            account_version: OracleUsdcRewardSchedule::ACCOUNT_VERSION,
            month: month_key,
            authority: month.authority,
            reward_vault: derive_oracle_usdc_reward_vault_pda(program_id).0,
            phase: OracleUsdcRewardSchedulePhase::Building,
            last_updated_slot: slot,
            bounty_fee_sweep_finalized: true,
            ..OracleUsdcRewardSchedule::default()
        },
    )?;
    store_oracle_month_state(month_info, &month)
}

/// Step 1. Accounts: see `instruction::cfm_launch_append`.
#[inline(never)]
fn append(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    product: &CfmProduct,
    row: u8,
    plan_hash: &[u8; 32],
) -> ProgramResult {
    let [payer_info, period_info, month_info, receipt_info, plan_info, registry_info, recipe_info, active_info, schedule_info, source_info, observations_info, sku_info, bucket_info, _journal_info, _checkpoint_info, system_program_info] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    let (anchor, month) = load_valid_period_and_oracle_month(program_id, period_info, month_info)?;
    let mut month = Box::new(month);
    let mut receipt = load_receipt(program_id, receipt_info, product)?;
    if receipt.finished
        || receipt.cursor != row
        || receipt.plan_hash != *plan_hash
        || receipt.period != *period_info.key
        || receipt.month != *month_info.key
    {
        return bad_receipt();
    }
    let (slot, now) = current_slot_and_unix_timestamp()?;
    if now >= anchor.expiry_ts {
        return Err(OracleError::OracleTimingWindowClosed.into());
    }
    let (plan, registry) = load_plan_and_registry(
        program_id,
        plan_info,
        registry_info,
        plan_hash,
        product,
        anchor.expiry_ts,
        now,
    )?;
    if month.phase != OraclePhase::Opening
        || month.weight_scheme_version != COLLECTING_WEIGHT_SCHEME
        || month.source_count != u16::from(row)
        || month.pending_resolution_count != 0
    {
        return Err(OracleError::InvalidOraclePhase.into());
    }
    let mut recipe =
        load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, recipe_info)?;
    let mut active =
        load_valid_oracle_active_weight_manifest(program_id, month_info.key, active_info)?;
    let mut schedule =
        usdc::load_oracle_usdc_reward_schedule(program_id, month_info.key, schedule_info)?;
    if recipe.phase != OracleRecipeWeightPhase::Collecting
        || active.phase != OracleRecipeWeightPhase::Collecting
        || recipe.processed_source_count != u16::from(row)
        || active.processed_source_count != u16::from(row)
        || schedule.phase != OracleUsdcRewardSchedulePhase::Building
        || schedule.sku_pool_count != u16::from(row)
    {
        return Err(OracleError::InvalidOracleWeightManifest.into());
    }
    let meta = registry.row(row)?;
    let (value, print_ts) = plan.row(row)?;

    let (source_key, source_bump) =
        derive_oracle_source_pda(program_id, month_info.key, &meta.source_id);
    let (observations_key, observations_bump) =
        derive_oracle_source_observations_pda(program_id, &source_key);
    let (sku_key, sku_bump) =
        derive_oracle_usdc_sku_pool_pda(program_id, schedule_info.key, &meta.source_id);
    let (bucket_key, bucket_bump) =
        derive_oracle_bucket_median_pda(program_id, month_info.key, &meta.source_id);
    let bumps = [
        [source_bump],
        [observations_bump],
        [sku_bump],
        [bucket_bump],
    ];
    let targets: [(&AccountInfo, &Pubkey, usize, [&[u8]; 4]); 4] = [
        (
            source_info,
            &source_key,
            OracleSourceState::LEN,
            [
                ORACLE_SOURCE_PDA_SEED,
                month_info.key.as_ref(),
                &meta.source_id,
                &bumps[0],
            ],
        ),
        (
            observations_info,
            &observations_key,
            OracleSourceObservations::LEN,
            [
                ORACLE_SOURCE_OBSERVATIONS_PDA_SEED,
                source_key.as_ref(),
                &bumps[1],
                &[],
            ],
        ),
        (
            sku_info,
            &sku_key,
            OracleUsdcSkuPool::LEN,
            [
                ORACLE_USDC_SKU_POOL_PDA_SEED,
                schedule_info.key.as_ref(),
                &meta.source_id,
                &bumps[2],
            ],
        ),
        (
            bucket_info,
            &bucket_key,
            OracleBucketMedianState::LEN,
            [
                ORACLE_BUCKET_MEDIAN_PDA_SEED,
                month_info.key.as_ref(),
                &meta.source_id,
                &bumps[3],
            ],
        ),
    ];
    for (info, key, len, seeds) in &targets {
        if info.key != *key {
            return Err(OracleError::InvalidPda.into());
        }
        validate_create_only_program_account_target(program_id, info)?;
        let used = if seeds[3].is_empty() { 3 } else { 4 };
        create_program_account(
            payer_info,
            info,
            system_program_info,
            program_id,
            *len,
            &seeds[..used],
        )?;
    }

    let source = open_source(
        program_id,
        accounts,
        &receipt,
        &meta,
        source_bump,
        observations_bump,
        value,
        print_ts,
    )?;
    write_row_pool_and_bucket(
        accounts,
        &plan,
        &source,
        row,
        active.processed_bucket_weight_bps,
        (sku_bump, bucket_bump),
        slot,
        now,
    )?;

    recipe.rolling_manifest_hash = advance_oracle_weight_manifest_hash(
        &recipe.rolling_manifest_hash,
        &meta.source_id,
        &source,
        meta.weight_bps,
    );
    recipe.processed_source_count += 1;
    recipe.processed_bucket_count += 1;
    recipe.declared_weight_total_bps = recipe
        .declared_weight_total_bps
        .checked_add(meta.weight_bps)
        .ok_or(OracleError::ArithmeticOverflow)?;
    recipe.current_bucket_id = meta.source_id;
    active.rolling_manifest_hash =
        advance_oracle_active_manifest_hash(&active.rolling_manifest_hash, &source);
    active.processed_source_count += 1;
    active.processed_group_count += 1;
    active.processed_bucket_weight_bps = recipe.declared_weight_total_bps;
    active.current_group_id = meta.source_id;
    schedule.sku_pool_count += 1;
    schedule.last_updated_slot = slot;
    month.source_count += 1;
    month.frozen_source_count = month.source_count;
    month.opened_source_count = month.source_count;
    month.opening_resolved_source_count = month.source_count;
    month.active_weight_group_count += 1;
    month.last_updated_slot = slot;
    receipt.cursor += 1;
    if receipt.cursor == receipt.row_count {
        // The last row freezes the recipe (as tag 86), finalizes the active weights (as tag 124)
        // and the zero-budget schedule, and lists the month (as tag 80) at its listing.
        let (manifest_hash, recipe_hash) = recipe_hashes(month_info.key, &registry)?;
        if recipe.rolling_manifest_hash != manifest_hash
            || recipe.declared_weight_total_bps != 10_000
        {
            return Err(OracleError::OracleWeightManifestHashMismatch.into());
        }
        recipe.phase = OracleRecipeWeightPhase::Finalized;
        recipe.recipe_hash = recipe_hash;
        month.weight_scheme_version = 1;
        month.effective_weight_total_bps = 10_000;
        month.weight_manifest_hash = manifest_hash;
        month.recipe_hash = recipe_hash;
        active.phase = OracleRecipeWeightPhase::Finalized;
        month.active_weight_scheme_version = OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION;
        month.active_weight_manifest_hash = active.rolling_manifest_hash;
        month.phase = OraclePhase::Game;
        ensure_finalized_oracle_active_weight_manifest(&month, &active)?;
        schedule.phase = OracleUsdcRewardSchedulePhase::Funded;
        receipt.finished = true;
        receipt.finished_at = now;
        write_period_live(program_id, period_info, &anchor, month.listing_ts)?;
    }
    store_state(receipt_info, &receipt)?;
    store_state(recipe_info, &recipe)?;
    store_state(active_info, &active)?;
    store_state(schedule_info, &schedule)?;
    store_oracle_month_state(month_info, &month)
}

/// The row's source, Active with the printed value as its opening and first observation, and
/// its journal and checkpoint (the receipt is the accepted event).
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn open_source(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    receipt: &CfmLaunchReceipt,
    meta: &RegistryRow,
    source_bump: u8,
    observations_bump: u8,
    value: u64,
    print_ts: u64,
) -> Result<Box<OracleSourceState>, ProgramError> {
    let payer_info = &accounts[0];
    let month_key = accounts[2].key;
    let receipt_key = accounts[3].key;
    let source_info = &accounts[9];
    let observations_info = &accounts[10];
    let before = Box::new(OracleSourceState {
        is_initialized: true,
        bump: source_bump,
        status: OracleSourceStatus::Active,
        ..registry_source(meta, month_key, &receipt.proposer)
    });
    let mut source = before.clone();
    let mut observations = Box::new(OracleSourceObservations {
        is_initialized: true,
        bump: observations_bump,
        account_discriminator: OracleSourceObservations::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSourceObservations::ACCOUNT_VERSION,
        month: *month_key,
        source: *source_info.key,
        ..OracleSourceObservations::default()
    });
    let snapshot = cfm::snapshot_hash(&receipt.plan_hash, &meta.source_id, value, print_ts);
    append_oracle_source_observation(
        &mut source,
        &mut observations,
        value,
        print_ts,
        &receipt.plan_hash,
        &snapshot,
    )?;
    source.baseline_state = value;
    source.current_state = value;
    source.opening_submitted = true;
    source.opening_evidence_hash = receipt.plan_hash;
    carry::record_fresh_accept(
        program_id,
        payer_info,
        source_info.key,
        &before,
        &source,
        &observations,
        carry::AcceptedEvent {
            event: *receipt_key,
            value,
            observed_at: print_ts,
            evidence_hash: receipt.plan_hash,
            archive_hash: snapshot,
            contributor: receipt.proposer,
        },
        &accounts[13..16],
    )?;
    store_state(source_info, source.as_ref())?;
    store_state(observations_info, observations.as_ref())?;
    Ok(source)
}

/// The row's SKU pool on the plan's terms and its bucket median, Live with the single source's
/// opening delta (zero), as tags 157 and 122 leave them.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn write_row_pool_and_bucket(
    accounts: &[AccountInfo],
    plan: &CfmPlanView,
    source: &OracleSourceState,
    row: u8,
    weight_start_bps: u16,
    (sku_bump, bucket_bump): (u8, u8),
    slot: u64,
    now: u64,
) -> ProgramResult {
    let month_key = accounts[2].key;
    let terms = plan.terms();
    store_state(
        &accounts[11],
        &OracleUsdcSkuPool {
            is_initialized: true,
            bump: sku_bump,
            account_discriminator: OracleUsdcSkuPool::ACCOUNT_DISCRIMINATOR,
            account_version: OracleUsdcSkuPool::ACCOUNT_VERSION,
            schedule: *accounts[8].key,
            month: *month_key,
            bucket_id: source.bucket_id,
            proposer_reward_bps: terms.proposer_reward_bps,
            listing_bond: terms.listing_bond,
            support_bond: terms.support_bond,
            opening_bond: terms.opening_bond,
            update_min_bond: terms.update_min_bond,
            challenge_min_bond: terms.challenge_min_bond,
            challenge_max_bond: terms.challenge_max_bond,
            challenge_bond_bps: terms.challenge_bond_bps,
            last_updated_slot: slot,
            ..OracleUsdcSkuPool::default()
        },
    )?;
    // The opening equals the current state: the single source's delta, and the median, is zero.
    store_state(
        &accounts[12],
        &OracleBucketMedianState {
            is_initialized: true,
            bump: bucket_bump,
            account_discriminator: OracleBucketMedianState::ACCOUNT_DISCRIMINATOR,
            account_version: OracleBucketMedianState::ACCOUNT_VERSION,
            month: *month_key,
            bucket_id: source.bucket_id,
            group_index: u16::from(row),
            bucket_weight_start_bps: weight_start_bps,
            bucket_weight_bps: source.bucket_weight_bps,
            frozen_source_count: 1,
            active_source_count: 1,
            eligible_source_count: 1,
            status: OracleBucketMedianStatus::Live,
            last_recomputed_ts: now,
            source_snapshot_hash: advance_oracle_bucket_source_snapshot(
                &initial_oracle_bucket_source_snapshot(month_key, &source.bucket_id, 2),
                source,
            ),
            ..OracleBucketMedianState::default()
        },
    )
}
