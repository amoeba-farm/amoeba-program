use super::*;

/// Create, extend, and finally freeze the ordered settlement-source digest.
/// Expected counts and provenance are derived from canonical finalized state.
#[inline(never)]
pub(super) fn process_accumulate_oracle_settlement_source_bucket(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: AccumulateOracleSettlementSourceBucketParams,
) -> ProgramResult {
    if accounts.len() < 6
        || accounts.len() > 5 + crate::constants::MAX_COMPRESSED_STATE_SESSION_RECORDS
        || !accounts[0].is_signer
        || crate::bytes32_is_zero(&params.bucket_id)
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let manifest_info = &accounts[3];
    let system_program_info = &accounts[4];
    validate_system_program(system_program_info)?;

    let (_market, month) = load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_month_ready_for_settlement(&month)?;
    if month.finalized_at_ts == 0
        || month.frozen_source_count == 0
        || month.active_weight_group_count == 0
    {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }

    let mut manifest = if manifest_info.owner == program_id {
        load_valid_oracle_settlement_source_manifest(program_id, month_info.key, manifest_info)?
    } else {
        let (expected, bump) =
            crate::state::derive_oracle_settlement_source_manifest_pda(program_id, month_info.key);
        if *manifest_info.key != expected {
            return Err(VaultError::InvalidOracleWeightManifest.into());
        }
        create_oracle_manifest_account(
            cranker_info,
            manifest_info,
            system_program_info,
            program_id,
            month_info.key.as_ref(),
            OracleSettlementSourceManifest::LEN,
            ORACLE_SETTLEMENT_SOURCE_MANIFEST_PDA_SEED,
            bump,
        )?;
        OracleSettlementSourceManifest {
            is_initialized: true,
            bump,
            account_discriminator: OracleSettlementSourceManifest::ACCOUNT_DISCRIMINATOR,
            account_version: OracleSettlementSourceManifest::ACCOUNT_VERSION,
            month: *month_info.key,
            phase: OracleRecipeWeightPhase::Collecting,
            expected_source_count: month.frozen_source_count,
            expected_bucket_count: month.active_weight_group_count,
            rolling_source_digest: initial_oracle_settlement_source_digest(
                month_info.key,
                &month.recipe_hash,
                &month.active_weight_manifest_hash,
                month.frozen_source_count,
                month.active_weight_group_count,
            ),
            ..OracleSettlementSourceManifest::default()
        }
    };
    if manifest.phase != OracleRecipeWeightPhase::Collecting
        || manifest.expected_source_count != month.frozen_source_count
        || manifest.expected_bucket_count != month.active_weight_group_count
    {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }

    for (offset, bucket_info) in accounts[5..].iter().enumerate() {
        if bucket_info.is_writable {
            return Err(VaultError::InvalidOracleMedian.into());
        }
        let bucket = load_valid_oracle_bucket_median(program_id, month_info.key, bucket_info)?;
        let expected_index = manifest.processed_bucket_count;
        if bucket.group_index != expected_index
            || (offset == 0 && bucket.bucket_id != params.bucket_id)
            || (manifest.processed_bucket_count > 0
                && bucket.bucket_id <= manifest.current_bucket_id)
            || !matches!(
                bucket.status,
                OracleBucketMedianStatus::SettlementReady
                    | OracleBucketMedianStatus::EmergencyDefaulted
            )
            || crate::bytes32_is_zero(&bucket.source_snapshot_hash)
        {
            return Err(VaultError::InvalidOracleWeightOrder.into());
        }
        manifest.processed_source_count = manifest
            .processed_source_count
            .checked_add(bucket.frozen_source_count)
            .ok_or(VaultError::ArithmeticOverflow)?;
        manifest.processed_bucket_count = manifest
            .processed_bucket_count
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        manifest.declared_weight_total_bps = manifest
            .declared_weight_total_bps
            .checked_add(bucket.bucket_weight_bps)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if manifest.processed_source_count > manifest.expected_source_count
            || manifest.processed_bucket_count > manifest.expected_bucket_count
            || manifest.declared_weight_total_bps > 10_000
        {
            return Err(VaultError::InvalidOracleWeightManifest.into());
        }
        manifest.rolling_source_digest =
            advance_oracle_settlement_source_digest(&manifest.rolling_source_digest, &bucket);
        manifest.current_bucket_id = bucket.bucket_id;
    }
    if params.finalize_collection {
        if manifest.processed_source_count != manifest.expected_source_count
            || manifest.processed_bucket_count != manifest.expected_bucket_count
            || manifest.declared_weight_total_bps != 10_000
        {
            return Err(VaultError::InvalidOracleWeightManifest.into());
        }
        manifest.phase = OracleRecipeWeightPhase::Finalized;
    } else if manifest.processed_bucket_count == manifest.expected_bucket_count {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    store_state(manifest_info, &manifest)
}
