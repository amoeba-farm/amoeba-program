use super::*;

#[inline(never)]
pub(super) fn process_begin_oracle_recipe_weights_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: BeginOracleRecipeWeightsV2Params,
) -> ProgramResult {
    if accounts.len() != 8 || !accounts[0].is_signer {
        return Err(VaultError::InvalidAccountList.into());
    }
    crate::processor::oracle_carry::require_import_complete(
        program_id,
        accounts[3].key,
        &accounts[7],
    )?;
    let authority_info = &accounts[0];
    let market_info = &accounts[1];
    let config_info = &accounts[2];
    let month_info = &accounts[3];
    let manifest_info = &accounts[4];
    let system_program_info = &accounts[5];
    let coverage_info = &accounts[6];
    validate_system_program(system_program_info)?;
    let config = load_active_oracle_vault_config(program_id, config_info)?;
    validate_oracle_recipe_weight_actor(&config, authority_info.key)?;
    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    if params.expected_source_count == 0
        || params.expected_bucket_count == 0
        || params.expected_bucket_count > params.expected_source_count
        || month.pending_resolution_count != 0
    {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    let coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    if !coverage.coverage_finalized
        || coverage.coverage_complete_ts == 0
        || coverage.covered_sku_count != coverage.required_sku_count
        || params.expected_bucket_count != coverage.required_sku_count
    {
        return Err(VaultError::OracleSkuCoverageIncomplete.into());
    }
    ensure_oracle_freeze_window(&market, &month)?;
    if params.expected_source_count != month.source_count || month.weight_scheme_version != 0 {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }

    let (expected_manifest, manifest_bump) =
        derive_oracle_recipe_weight_manifest_pda(program_id, month_info.key);
    if *manifest_info.key != expected_manifest {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    validate_create_only_program_account_target(program_id, manifest_info)?;
    create_oracle_manifest_account(
        authority_info,
        manifest_info,
        system_program_info,
        program_id,
        month_info.key.as_ref(),
        OracleRecipeWeightManifest::LEN,
        ORACLE_RECIPE_WEIGHT_MANIFEST_PDA_SEED,
        manifest_bump,
    )?;

    let manifest = OracleRecipeWeightManifest {
        is_initialized: true,
        bump: manifest_bump,
        account_discriminator: OracleRecipeWeightManifest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleRecipeWeightManifest::ACCOUNT_VERSION,
        month: *month_info.key,
        phase: OracleRecipeWeightPhase::Collecting,
        expected_source_count: params.expected_source_count,
        expected_bucket_count: params.expected_bucket_count,
        recipe_hash: [0; 32],
        rolling_manifest_hash: initial_oracle_weight_manifest_hash(
            month_info.key,
            params.expected_source_count,
            params.expected_bucket_count,
        ),
        ..OracleRecipeWeightManifest::default()
    };
    month.weight_scheme_version = 255;
    month.effective_weight_total_bps = 0;
    month.weight_manifest_hash = [0; 32];
    month.last_updated_slot = Clock::get()?.slot;
    store_state(manifest_info, &manifest)?;
    store_oracle_month_state(month_info, &month)
}

pub(super) fn process_accumulate_oracle_recipe_bucket_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: AccumulateOracleRecipeBucketV2Params,
) -> ProgramResult {
    if accounts.len() < 5
        || accounts.len() - 5 > crate::constants::MAX_COMPRESSED_STATE_SESSION_RECORDS / 2
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let market_info = &accounts[1];
    let config_info = &accounts[2];
    let month_info = &accounts[3];
    let manifest_info = &accounts[4];
    if !authority_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if accounts.len() == 5
        || crate::bytes32_is_zero(&params.bucket_id)
        || params.bucket_weight_bps == 0
        || params.bucket_weight_bps > 10_000
    {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    let config = load_active_oracle_vault_config(program_id, config_info)?;
    let (market, month) = load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let mut manifest =
        load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, manifest_info)?;
    validate_oracle_recipe_weight_actor(&config, authority_info.key)?;
    validate_oracle_recipe_weight_build_context(&market, &month, &manifest)?;
    if manifest.phase != OracleRecipeWeightPhase::Collecting {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }

    if manifest.current_bucket_source_count == 0 {
        if manifest.processed_bucket_count >= manifest.expected_bucket_count
            || (manifest.processed_bucket_count > 0
                && params.bucket_id <= manifest.current_bucket_id)
        {
            return Err(VaultError::InvalidOracleWeightOrder.into());
        }
        manifest.current_bucket_id = params.bucket_id;
        manifest.current_bucket_weight_bps = params.bucket_weight_bps;
        manifest.current_bucket_source_count = 0;
        manifest.last_collected_source_id = [0; 32];
        manifest.declared_weight_total_bps = manifest
            .declared_weight_total_bps
            .checked_add(params.bucket_weight_bps)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if manifest.declared_weight_total_bps > 10_000 {
            return Err(VaultError::InvalidOracleWeightManifest.into());
        }
    } else if params.bucket_id != manifest.current_bucket_id
        || params.bucket_weight_bps != manifest.current_bucket_weight_bps
    {
        return Err(VaultError::InvalidOracleWeightOrder.into());
    }

    for source_info in &accounts[5..] {
        if !source_info.is_writable {
            return Err(VaultError::InvalidOracleWeightSource.into());
        }
        let mut source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
        validate_oracle_recipe_weight_source(&manifest, &source)?;
        if manifest.current_bucket_source_count > 0
            && source.source_id <= manifest.last_collected_source_id
        {
            return Err(VaultError::InvalidOracleWeightOrder.into());
        }
        if source.status != OracleSourceStatus::Candidate || source.support_stake_total == 0 {
            return Err(VaultError::InvalidOracleWeightSource.into());
        }
        manifest.current_bucket_source_count = manifest
            .current_bucket_source_count
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if usize::from(manifest.current_bucket_source_count)
            > crate::constants::MAX_ORACLE_BUCKET_SOURCES
        {
            return Err(VaultError::InvalidOracleWeightManifest.into());
        }
        if manifest
            .processed_source_count
            .checked_add(manifest.current_bucket_source_count)
            .ok_or(VaultError::ArithmeticOverflow)?
            > manifest.expected_source_count
        {
            return Err(VaultError::InvalidOracleWeightManifest.into());
        }
        manifest.rolling_manifest_hash = advance_oracle_weight_manifest_hash(
            &manifest.rolling_manifest_hash,
            &manifest.current_bucket_id,
            &source,
            manifest.current_bucket_weight_bps,
        );
        source.bucket_weight_bps = manifest.current_bucket_weight_bps;
        source.status = OracleSourceStatus::Frozen;
        source.baseline_state = 0;
        source.current_state = 0;
        store_state(source_info, &source)?;
        manifest.last_collected_source_id = source.source_id;
    }
    if params.finalize_collection {
        if manifest.current_bucket_source_count == 0 {
            return Err(VaultError::InvalidOracleWeightManifest.into());
        }
        manifest.processed_source_count = manifest
            .processed_source_count
            .checked_add(manifest.current_bucket_source_count)
            .ok_or(VaultError::ArithmeticOverflow)?;
        manifest.processed_bucket_count = manifest
            .processed_bucket_count
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if manifest.processed_source_count > manifest.expected_source_count
            || manifest.processed_bucket_count > manifest.expected_bucket_count
        {
            return Err(VaultError::InvalidOracleWeightManifest.into());
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
    store_state(manifest_info, &manifest)
}

pub(super) fn process_finalize_oracle_recipe_weights_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 5 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let market_info = &accounts[1];
    let config_info = &accounts[2];
    let month_info = &accounts[3];
    let manifest_info = &accounts[4];
    if !authority_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_active_oracle_vault_config(program_id, config_info)?;
    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let mut manifest =
        load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, manifest_info)?;
    validate_oracle_recipe_weight_actor(&config, authority_info.key)?;
    if manifest.phase == OracleRecipeWeightPhase::Finalized {
        return Err(VaultError::OracleWeightManifestFinalized.into());
    }
    validate_oracle_recipe_weight_build_context(&market, &month, &manifest)?;
    if manifest.phase != OracleRecipeWeightPhase::ReadyToFinalize
        || manifest.processed_source_count != manifest.expected_source_count
        || manifest.processed_bucket_count != manifest.expected_bucket_count
        || manifest.declared_weight_total_bps != 10_000
        || crate::bytes32_is_zero(&manifest.rolling_manifest_hash)
    {
        return Err(VaultError::OracleWeightManifestIncomplete.into());
    }
    ensure_oracle_freeze_window(&market, &month)?;
    let derived_recipe_hash =
        canonical_recipe_digest(month_info.key, &manifest.rolling_manifest_hash);
    month.recipe_hash = derived_recipe_hash;
    manifest.recipe_hash = derived_recipe_hash;
    month.source_count = manifest.expected_source_count;
    month.frozen_source_count = manifest.expected_source_count;
    month.opened_source_count = 0;
    month.opening_resolved_source_count = 0;
    month.pending_resolution_count = 0;
    month.index_delta_bps = 0;
    month.phase = OraclePhase::Opening;
    month.weight_scheme_version = 1;
    month.effective_weight_total_bps = manifest.declared_weight_total_bps;
    month.weight_manifest_hash = manifest.rolling_manifest_hash;
    month.last_updated_slot = Clock::get()?.slot;
    manifest.phase = OracleRecipeWeightPhase::Finalized;
    store_state(manifest_info, &manifest)?;
    store_oracle_month_state(month_info, &month)
}
