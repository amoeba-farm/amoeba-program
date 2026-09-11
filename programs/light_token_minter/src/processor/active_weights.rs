use super::*;

#[inline]
/// Tighten by economically active (nonzero-weight) buckets only.
///
/// Current terminal-source validation rejects zero bucket weight before this fold, so the zero
/// branch defines the audit mutation without admitting a new manifest shape.
pub(super) fn fold_economically_active_bucket_security_cap(
    current_cap: u64,
    bucket_weight_bps: u16,
    bucket_cap: u64,
) -> u64 {
    if bucket_weight_bps == 0 {
        current_cap
    } else {
        current_cap.min(bucket_cap)
    }
}

pub(super) fn initial_oracle_active_weight_hash(
    month: &Pubkey,
    recipe_hash: &[u8; 32],
    frozen_manifest_hash: &[u8; 32],
    source_count: u16,
    group_count: u16,
) -> [u8; 32] {
    hashv(&[
        ORACLE_ACTIVE_WEIGHT_MANIFEST_HASH_DOMAIN,
        month.as_ref(),
        recipe_hash,
        frozen_manifest_hash,
        &source_count.to_le_bytes(),
        &group_count.to_le_bytes(),
    ])
    .to_bytes()
}

pub(super) fn advance_oracle_active_manifest_hash(
    previous: &[u8; 32],
    source: &OracleSourceState,
) -> [u8; 32] {
    hashv(&[
        ORACLE_ACTIVE_WEIGHT_MANIFEST_HASH_DOMAIN,
        previous,
        &source.source_id,
        &source.bucket_id,
        &source.bucket_weight_bps.to_le_bytes(),
        &[source.status as u8],
        &[source.observation_count],
        &source.rolling_observation_hash,
    ])
    .to_bytes()
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(super) fn create_oracle_manifest_account<'a>(
    payer_info: &AccountInfo<'a>,
    manifest_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    program_id: &Pubkey,
    scope: &[u8],
    account_len: usize,
    pda_seed: &[u8],
    bump: u8,
) -> ProgramResult {
    let bump_seed = [bump];
    create_program_account(
        payer_info,
        manifest_info,
        system_program_info,
        program_id,
        account_len,
        &[pda_seed, scope, &bump_seed],
    )
}

fn validate_oracle_terminal_source(source: &OracleSourceState) -> ProgramResult {
    let terminal_state_valid = match source.status {
        OracleSourceStatus::Active => {
            source.opening_submitted
                && !crate::bytes32_is_zero(&source.opening_evidence_hash)
                && source.baseline_state > 0
                && source.current_state > 0
        }
        OracleSourceStatus::Inactive => {
            !source.opening_submitted
                && crate::bytes32_is_zero(&source.opening_evidence_hash)
                && source.baseline_state == 0
                && source.current_state == 0
        }
        _ => false,
    };
    if !terminal_state_valid
        || crate::bytes32_is_zero(&source.source_id)
        || crate::bytes32_is_zero(&source.bucket_id)
        || crate::bytes32_is_zero(&source.source_type_hash)
        || crate::bytes32_is_zero(&source.canonical_locator_hash)
        || crate::bytes32_is_zero(&source.source_definition_hash)
        || source.support_stake_total == 0
        || source.bucket_weight_bps == 0
    {
        return Err(VaultError::InvalidOracleWeightSource.into());
    }
    Ok(())
}

pub(super) fn process_begin_oracle_active_weights(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: BeginOracleActiveWeightsParams,
) -> ProgramResult {
    if accounts.len() != 7 || !accounts[0].is_signer {
        return Err(VaultError::InvalidAccountList.into());
    }
    let actor_info = &accounts[0];
    let market_info = &accounts[1];
    let config_info = &accounts[2];
    let month_info = &accounts[3];
    let recipe_manifest_info = &accounts[4];
    let active_manifest_info = &accounts[5];
    let system_program_info = &accounts[6];
    validate_system_program(system_program_info)?;
    let config = load_active_oracle_vault_config(program_id, config_info)?;
    validate_oracle_recipe_weight_actor(&config, actor_info.key)?;
    let (_market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_opening_sources_terminal(&month)?;
    if params.expected_source_count == 0
        || params.expected_source_count != month.frozen_source_count
        || params.expected_group_count == 0
        || params.expected_group_count > params.expected_source_count
        || month.active_weight_scheme_version != 0
        || month.active_weight_group_count != 0
        || !crate::bytes32_is_zero(&month.active_weight_manifest_hash)
    {
        return Err(VaultError::InvalidOracleActiveWeightManifest.into());
    }
    let recipe =
        load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, recipe_manifest_info)?;
    validate_oracle_active_manifest_begin_membership(
        &month,
        &recipe,
        params.expected_source_count,
        params.expected_group_count,
    )?;
    let (expected, bump) = derive_oracle_active_weight_manifest_pda(program_id, month_info.key);
    if *active_manifest_info.key != expected {
        return Err(VaultError::InvalidOracleActiveWeightManifest.into());
    }
    validate_create_only_program_account_target(program_id, active_manifest_info)?;
    create_oracle_manifest_account(
        actor_info,
        active_manifest_info,
        system_program_info,
        program_id,
        month_info.key.as_ref(),
        OracleActiveWeightManifest::LEN,
        ORACLE_ACTIVE_WEIGHT_MANIFEST_PDA_SEED,
        bump,
    )?;
    let manifest = OracleActiveWeightManifest {
        is_initialized: true,
        bump,
        account_discriminator: OracleActiveWeightManifest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleActiveWeightManifest::ACCOUNT_VERSION,
        month: *month_info.key,
        phase: OracleRecipeWeightPhase::Collecting,
        expected_source_count: params.expected_source_count,
        expected_group_count: params.expected_group_count,
        max_open_interest_payout: u64::MAX,
        rolling_manifest_hash: initial_oracle_active_weight_hash(
            month_info.key,
            &month.recipe_hash,
            &month.weight_manifest_hash,
            params.expected_source_count,
            params.expected_group_count,
        ),
        ..OracleActiveWeightManifest::default()
    };
    month.active_weight_scheme_version = 255;
    month.active_weight_group_count = 0;
    month.active_weight_manifest_hash = [0; 32];
    month.last_updated_slot = Clock::get()?.slot;
    store_state(active_manifest_info, &manifest)?;
    store_oracle_month_state(month_info, &month)
}

/// The two trailing read-only accounts are the completed recipe source index and this
/// bucket's authenticated source index. Sources retain their original indices starting at 7.
#[inline(never)]
pub(super) fn process_accumulate_oracle_active_weight_group(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: AccumulateOracleActiveWeightGroupParams,
) -> ProgramResult {
    if accounts.len() < 10
        || accounts.len() - 9 > crate::constants::MAX_ORACLE_ACTIVE_WEIGHT_SOURCES_PER_STEP
        || !accounts[0].is_signer
        || !accounts[0].is_writable
        || !accounts[2].is_writable
        || !accounts[3].is_writable
        || accounts[5].is_signer
        || !accounts[5].is_writable
        || crate::bytes32_is_zero(&params.group_id)
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let manifest_info = &accounts[3];
    let system_program_info = &accounts[4];
    let sku_info = &accounts[5];
    let bucket_info = &accounts[6];
    let source_end = accounts.len() - 2;
    validate_system_program(system_program_info)?;
    let (_market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_opening_sources_terminal(&month)?;
    let mut manifest =
        load_valid_oracle_active_weight_manifest(program_id, month_info.key, manifest_info)?;
    if manifest.phase != OracleRecipeWeightPhase::Collecting
        || !crate::bytes32_is_zero(&month.active_weight_manifest_hash)
        || month.active_weight_scheme_version != 255
        || month.active_weight_group_count != manifest.processed_group_count
        || (manifest.current_group_source_count > 0 && params.group_id != manifest.current_group_id)
    {
        return Err(VaultError::InvalidOracleActiveWeightManifest.into());
    }
    let membership = oracle_membership::load_complete_bucket_source_index(
        program_id,
        month_info.key,
        &month,
        &params.group_id,
        &accounts[source_end],
        &accounts[source_end + 1],
    )?;
    let next_count = usize::from(manifest.current_group_source_count) + source_end - 7;
    if membership.group_index != manifest.processed_group_count
        || membership.first_source_index != manifest.processed_source_count
        || next_count > usize::from(membership.source_count)
        // Reaching the exact group end must finalize in this call. Otherwise there would
        // be no next source with which a permissionless caller could complete the group.
        || params.finalize_collection != (next_count == usize::from(membership.source_count))
        || (manifest.current_group_source_count > 0
            && (manifest.current_group_bucket_weight_bps != membership.bucket_weight_bps
                || manifest.last_collected_source_id
                    != membership.source_ids[usize::from(manifest.current_group_source_count - 1)]))
    {
        return Err(VaultError::InvalidOracleWeightOrder.into());
    }
    let sku = oracle_usdc::load_oracle_usdc_sku_for_month(program_id, month_info.key, sku_info)?;
    if sku.bucket_id != params.group_id {
        return Err(VaultError::InvalidOracleUsdcSkuPool.into());
    }
    let (expected_bucket, bucket_bump) =
        derive_oracle_bucket_median_pda(program_id, month_info.key, &params.group_id);
    if *bucket_info.key != expected_bucket || !bucket_info.is_writable {
        return Err(VaultError::InvalidOracleMedian.into());
    }
    let mut bucket = if manifest.current_group_active_count == 0 {
        validate_create_only_program_account_target(program_id, bucket_info)?;
        None
    } else {
        let current = load_valid_oracle_bucket_median(program_id, month_info.key, bucket_info)?;
        if current.bucket_id != params.group_id
            || current.group_index != manifest.processed_group_count
            || current.frozen_source_count != manifest.current_group_source_count
            || current.active_source_count != manifest.current_group_active_count
            || current.status != OracleBucketMedianStatus::Dirty
        {
            return Err(VaultError::InvalidOracleMedian.into());
        }
        Some(current)
    };

    for source_info in &accounts[7..source_end] {
        let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
        validate_oracle_terminal_source(&source)?;
        if source.source_id
            != membership.source_ids[usize::from(manifest.current_group_source_count)]
            || source.bucket_weight_bps != membership.bucket_weight_bps
        {
            return Err(VaultError::InvalidOracleWeightOrder.into());
        }
        if manifest.current_group_source_count == 0 {
            if manifest.processed_group_count >= manifest.expected_group_count
                || (manifest.processed_group_count > 0
                    && params.group_id <= manifest.current_group_id)
            {
                return Err(VaultError::InvalidOracleWeightOrder.into());
            }
            manifest.current_group_id = params.group_id;
            manifest.current_group_bucket_weight_bps = source.bucket_weight_bps;
        }
        validate_active_group_source_identity(
            &manifest.current_group_id,
            manifest.current_group_bucket_weight_bps,
            manifest.current_group_source_count,
            &manifest.last_collected_source_id,
            &source,
        )?;
        manifest.current_group_source_count = manifest
            .current_group_source_count
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if usize::from(manifest.current_group_source_count)
            > crate::constants::MAX_ORACLE_BUCKET_SOURCES
            || manifest
                .processed_source_count
                .checked_add(manifest.current_group_source_count)
                .ok_or(VaultError::ArithmeticOverflow)?
                > manifest.expected_source_count
        {
            return Err(VaultError::InvalidOracleActiveWeightManifest.into());
        }

        if source.status == OracleSourceStatus::Active {
            if source.observation_count == 0
                || crate::bytes32_is_zero(&source.rolling_observation_hash)
            {
                return Err(VaultError::InvalidOracleObservation.into());
            }
            manifest.current_group_active_count = manifest
                .current_group_active_count
                .checked_add(1)
                .ok_or(VaultError::ArithmeticOverflow)?;
            if bucket.is_none() {
                create_program_account(
                    cranker_info,
                    bucket_info,
                    system_program_info,
                    program_id,
                    OracleBucketMedianState::LEN,
                    &[
                        ORACLE_BUCKET_MEDIAN_PDA_SEED,
                        month_info.key.as_ref(),
                        &params.group_id,
                        &[bucket_bump],
                    ],
                )?;
                bucket = Some(OracleBucketMedianState {
                    is_initialized: true,
                    bump: bucket_bump,
                    account_discriminator: OracleBucketMedianState::ACCOUNT_DISCRIMINATOR,
                    account_version: OracleBucketMedianState::ACCOUNT_VERSION,
                    month: *month_info.key,
                    bucket_id: params.group_id,
                    group_index: manifest.processed_group_count,
                    bucket_weight_start_bps: manifest.processed_bucket_weight_bps,
                    bucket_weight_bps: manifest.current_group_bucket_weight_bps,
                    frozen_source_count: 0,
                    active_source_count: 0,
                    eligible_source_count: 0,
                    status: OracleBucketMedianStatus::Dirty,
                    bucket_delta_bps: 0,
                    last_recomputed_ts: 0,
                    source_snapshot_hash: [0; 32],
                    recompute_processed_source_count: 0,
                    last_recompute_source_id: [0; 32],
                    emergency_snapshot_slot: 0,
                    emergency_snapshot_total_samba: 0,
                    opening_source_deltas_bps: [0; crate::constants::MAX_ORACLE_BUCKET_SOURCES],
                });
            }
            let bucket = bucket.as_mut().ok_or(VaultError::InvalidOracleMedian)?;
            let delta_index = usize::from(manifest.current_group_active_count - 1);
            bucket.opening_source_deltas_bps[delta_index] =
                source_delta_bps(source.baseline_state, source.current_state)?;
        } else if source.observation_count != 0
            || !crate::bytes32_is_zero(&source.rolling_observation_hash)
        {
            return Err(VaultError::InvalidOracleObservation.into());
        }
        if let Some(bucket) = bucket.as_mut() {
            bucket.frozen_source_count = manifest.current_group_source_count;
            bucket.active_source_count = manifest.current_group_active_count;
        }
        manifest.rolling_manifest_hash =
            advance_oracle_active_manifest_hash(&manifest.rolling_manifest_hash, &source);
        manifest.last_collected_source_id = source.source_id;
    }

    let max_open_interest_payout = if bucket.is_some() {
        let minimum = minimum_oracle_bucket_eligible_sources(manifest.current_group_source_count);
        let security_source_count = manifest.current_group_active_count.min(minimum);
        let (_, cap) = oracle_bucket_security_cap(
            security_source_count,
            sku.listing_bond,
            sku.support_bond,
            crate::constants::ORACLE_OI_CAP_KAPPA_BPS,
        )?;
        Some(cap)
    } else {
        None
    };
    if params.finalize_collection {
        validate_active_group_collection_completion(&manifest)?;
        let bucket = bucket.as_mut().ok_or(VaultError::InvalidOracleMedian)?;
        if bucket.active_source_count != manifest.current_group_active_count
            || bucket.frozen_source_count != manifest.current_group_source_count
        {
            return Err(VaultError::OracleWeightManifestHashMismatch.into());
        }
        manifest.max_open_interest_payout = fold_economically_active_bucket_security_cap(
            manifest.max_open_interest_payout,
            bucket.bucket_weight_bps,
            max_open_interest_payout.ok_or(VaultError::InvalidOracleMedian)?,
        );
        let active_count = usize::from(bucket.active_source_count);
        let mut values = bucket.opening_source_deltas_bps;
        bucket.bucket_delta_bps = deterministic_bucket_median(&mut values[..active_count])?;
        bucket.eligible_source_count = bucket.active_source_count;
        bucket.last_recomputed_ts = u64::try_from(Clock::get()?.unix_timestamp)
            .map_err(|_| VaultError::InvalidOracleMedian)?;
        bucket.status = OracleBucketMedianStatus::Live;
        month.index_delta_bps = month
            .index_delta_bps
            .checked_add(bucket_index_contribution_bps(
                bucket.bucket_weight_bps,
                bucket.bucket_delta_bps,
            )?)
            .ok_or(VaultError::ArithmeticOverflow)?;
        manifest.processed_source_count = manifest
            .processed_source_count
            .checked_add(manifest.current_group_source_count)
            .ok_or(VaultError::ArithmeticOverflow)?;
        manifest.processed_group_count = manifest
            .processed_group_count
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        month.active_weight_group_count = month
            .active_weight_group_count
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        manifest.processed_bucket_weight_bps = manifest
            .processed_bucket_weight_bps
            .checked_add(manifest.current_group_bucket_weight_bps)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if manifest.processed_bucket_weight_bps > 10_000 {
            return Err(VaultError::InvalidOracleActiveWeightManifest.into());
        }
        manifest.phase = if manifest.processed_group_count == manifest.expected_group_count {
            OracleRecipeWeightPhase::ReadyToFinalize
        } else {
            OracleRecipeWeightPhase::Collecting
        };
        manifest.current_group_source_count = 0;
        manifest.current_group_active_count = 0;
        manifest.current_group_bucket_weight_bps = 0;
        manifest.last_collected_source_id = [0; 32];
    }
    if let Some(bucket) = bucket.as_ref() {
        store_state(bucket_info, bucket)?;
    }
    store_state(manifest_info, &manifest)?;
    store_oracle_month_state(month_info, &month)
}

pub(super) fn process_finalize_oracle_active_weights(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 5 || !accounts[0].is_signer {
        return Err(VaultError::InvalidAccountList.into());
    }
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let recipe_manifest_info = &accounts[3];
    let manifest_info = &accounts[4];
    let (_market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_opening_sources_terminal(&month)?;
    let recipe =
        load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, recipe_manifest_info)?;
    let mut manifest =
        load_valid_oracle_active_weight_manifest(program_id, month_info.key, manifest_info)?;
    validate_oracle_active_manifest_completion(&month, &recipe, &manifest)?;
    manifest.phase = OracleRecipeWeightPhase::Finalized;
    month.active_weight_scheme_version = OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION;
    month.active_weight_manifest_hash = manifest.rolling_manifest_hash;
    month.last_updated_slot = Clock::get()?.slot;
    store_state(manifest_info, &manifest)?;
    store_oracle_month_state(month_info, &month)
}
