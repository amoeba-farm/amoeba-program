use super::*;
use crate::instruction::IndexOracleRecipeSourceV1Params;
use crate::state::{
    derive_oracle_bucket_source_index_pda, derive_oracle_recipe_source_index_pda,
    OracleBucketSourceIndex, OracleRecipeSourceIndex, ORACLE_BUCKET_SOURCE_INDEX_SEED,
    ORACLE_RECIPE_SOURCE_INDEX_SEED,
};

fn load_recipe_index(
    program_id: &Pubkey,
    month_key: &Pubkey,
    month: &OracleMonthState,
    info: &AccountInfo,
) -> Result<OracleRecipeSourceIndex, ProgramError> {
    let index: OracleRecipeSourceIndex = load_exact_zero_padded_state(
        info,
        program_id,
        OracleRecipeSourceIndex::LEN,
        VaultError::InvalidOracleWeightManifest,
    )?;
    let (address, bump) = derive_oracle_recipe_source_index_pda(program_id, month_key);
    if info.executable
        || *info.key != address
        || !index.is_initialized
        || index.bump != bump
        || index.account_discriminator != OracleRecipeSourceIndex::ACCOUNT_DISCRIMINATOR
        || index.account_version != OracleRecipeSourceIndex::ACCOUNT_VERSION
        || index.month != *month_key
        || index.recipe_hash != month.recipe_hash
        || index.manifest_hash != month.weight_manifest_hash
        || crate::bytes32_is_zero(&index.recipe_hash)
        || crate::bytes32_is_zero(&index.manifest_hash)
        || crate::bytes32_is_zero(&index.remaining_hash)
        || index.expected_source_count == 0
        || index.expected_source_count != month.frozen_source_count
        || index.expected_bucket_count == 0
        || index.expected_bucket_count > index.expected_source_count
        || index.remaining_source_count >= index.expected_source_count
        || index.indexed_bucket_count == 0
        || index.indexed_bucket_count > index.expected_bucket_count
        || index.indexed_bucket_weight_bps == 0
        || index.indexed_bucket_weight_bps > 10_000
        || crate::bytes32_is_zero(&index.last_bucket_id)
        || crate::bytes32_is_zero(&index.last_source_id)
        || index.complete != (index.remaining_source_count == 0)
    {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    if index.complete {
        validate_index_completion(month_key, &index)?;
    }
    Ok(index)
}

fn validate_index_completion(month: &Pubkey, index: &OracleRecipeSourceIndex) -> ProgramResult {
    if index.remaining_source_count != 0
        || index.indexed_bucket_count != index.expected_bucket_count
        || index.indexed_bucket_weight_bps != 10_000
        || index.remaining_hash
            != initial_oracle_weight_manifest_hash(
                month,
                index.expected_source_count,
                index.expected_bucket_count,
            )
    {
        return Err(VaultError::OracleWeightManifestHashMismatch.into());
    }
    Ok(())
}

fn load_bucket_index(
    program_id: &Pubkey,
    index: &OracleRecipeSourceIndex,
    bucket_id: &[u8; 32],
    info: &AccountInfo,
) -> Result<OracleBucketSourceIndex, ProgramError> {
    let bucket: OracleBucketSourceIndex = load_exact_zero_padded_state(
        info,
        program_id,
        OracleBucketSourceIndex::LEN,
        VaultError::InvalidOracleWeightManifest,
    )?;
    let (address, bump) =
        derive_oracle_bucket_source_index_pda(program_id, &index.month, bucket_id);
    let count = usize::from(bucket.source_count);
    if info.executable
        || *info.key != address
        || !bucket.is_initialized
        || bucket.bump != bump
        || bucket.account_discriminator != OracleBucketSourceIndex::ACCOUNT_DISCRIMINATOR
        || bucket.account_version != OracleBucketSourceIndex::ACCOUNT_VERSION
        || bucket.month != index.month
        || bucket.recipe_hash != index.recipe_hash
        || bucket.bucket_id != *bucket_id
        || crate::bytes32_is_zero(bucket_id)
        || bucket.group_index >= index.expected_bucket_count
        || bucket.bucket_weight_bps == 0
        || bucket.bucket_weight_bps > 10_000
        || count == 0
        || count > crate::constants::MAX_ORACLE_BUCKET_SOURCES
        || bucket
            .first_source_index
            .checked_add(bucket.source_count)
            .is_none_or(|end| end > index.expected_source_count)
    {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    if bucket.source_ids[..count]
        .iter()
        .any(crate::bytes32_is_zero)
        || bucket.source_ids[..count]
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || bucket.source_ids[count..]
            .iter()
            .any(|id| !crate::bytes32_is_zero(id))
    {
        return Err(VaultError::InvalidOracleWeightOrder.into());
    }
    Ok(bucket)
}

/// No partial index is a membership proof. Both identities are pinned to the current recipe,
/// and completion proves the reverse walk reached its exact count-bound initial hash.
pub(super) fn load_complete_bucket_source_index(
    program_id: &Pubkey,
    month_key: &Pubkey,
    month: &OracleMonthState,
    bucket_id: &[u8; 32],
    index_info: &AccountInfo,
    bucket_info: &AccountInfo,
) -> Result<OracleBucketSourceIndex, ProgramError> {
    if index_info.is_signer
        || index_info.is_writable
        || bucket_info.is_signer
        || bucket_info.is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let index = load_recipe_index(program_id, month_key, month, index_info)?;
    if !index.complete {
        return Err(VaultError::OracleWeightManifestIncomplete.into());
    }
    load_bucket_index(program_id, &index, bucket_id, bucket_info)
}

/// Authenticate one source before persisting any progress. Walking the existing hash chain
/// backwards makes each next step independently verifiable: a later/earlier source cannot
/// start a competing prefix or choose the shared cursor. This works with the one current
/// frozen recipe format and never rewrites a recipe, source, active manifest, or median.
#[inline(never)]
pub(super) fn process_index_oracle_recipe_source(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: IndexOracleRecipeSourceV1Params,
) -> ProgramResult {
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    for (position, account) in accounts.iter().enumerate() {
        if account.is_signer != (position == 0)
            || account.is_writable != matches!(position, 0 | 4 | 5)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    let payer = &accounts[0];
    let month_info = &accounts[2];
    let index_info = &accounts[4];
    let bucket_info = &accounts[5];
    let system_info = &accounts[6];
    validate_system_program(system_info)?;
    let (_, month) = load_valid_market_and_oracle_month(program_id, &accounts[1], month_info)?;
    ensure_oracle_canonical_weight_scheme(&month)?;
    let recipe =
        load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, &accounts[3])?;
    validate_oracle_active_manifest_begin_membership(
        &month,
        &recipe,
        recipe.expected_source_count,
        recipe.expected_bucket_count,
    )?;
    if recipe.processed_source_count != recipe.expected_source_count
        || recipe.processed_bucket_count != recipe.expected_bucket_count
        || recipe.declared_weight_total_bps != 10_000
        || canonical_recipe_digest(month_info.key, &recipe.rolling_manifest_hash)
            != month.recipe_hash
        || params.bucket_weight_bps == 0
        || params.bucket_weight_bps > 10_000
        || [
            &params.previous_hash,
            &params.bucket_id,
            &params.source_id,
            &params.source_type_hash,
            &params.canonical_locator_hash,
            &params.source_definition_hash,
        ]
        .into_iter()
        .any(crate::bytes32_is_zero)
    {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    let (index_address, index_bump) =
        derive_oracle_recipe_source_index_pda(program_id, month_info.key);
    let (bucket_address, bucket_bump) =
        derive_oracle_bucket_source_index_pda(program_id, month_info.key, &params.bucket_id);
    if *index_info.key != index_address || *bucket_info.key != bucket_address {
        return Err(VaultError::InvalidPda.into());
    }
    let create_index = index_info.owner != program_id;
    let mut index = if create_index {
        validate_create_only_program_account_target(program_id, index_info)?;
        OracleRecipeSourceIndex {
            is_initialized: true,
            bump: index_bump,
            account_discriminator: OracleRecipeSourceIndex::ACCOUNT_DISCRIMINATOR,
            account_version: OracleRecipeSourceIndex::ACCOUNT_VERSION,
            month: *month_info.key,
            recipe_hash: month.recipe_hash,
            manifest_hash: month.weight_manifest_hash,
            remaining_hash: month.weight_manifest_hash,
            expected_source_count: recipe.expected_source_count,
            expected_bucket_count: recipe.expected_bucket_count,
            remaining_source_count: recipe.expected_source_count,
            ..OracleRecipeSourceIndex::default()
        }
    } else {
        load_recipe_index(program_id, month_info.key, &month, index_info)?
    };
    if index.complete || index.expected_bucket_count != recipe.expected_bucket_count {
        return Err(VaultError::OracleWeightManifestFinalized.into());
    }
    let frozen_source = OracleSourceState {
        source_id: params.source_id,
        source_type_hash: params.source_type_hash,
        canonical_locator_hash: params.canonical_locator_hash,
        source_definition_hash: params.source_definition_hash,
        ..OracleSourceState::default()
    };
    if advance_oracle_weight_manifest_hash(
        &params.previous_hash,
        &params.bucket_id,
        &frozen_source,
        params.bucket_weight_bps,
    ) != index.remaining_hash
    {
        return Err(VaultError::OracleWeightManifestHashMismatch.into());
    }

    let new_bucket = index.indexed_bucket_count == 0 || params.bucket_id != index.last_bucket_id;
    let mut bucket = if new_bucket {
        if index.indexed_bucket_count >= index.expected_bucket_count
            || (index.indexed_bucket_count > 0 && params.bucket_id >= index.last_bucket_id)
        {
            return Err(VaultError::InvalidOracleWeightOrder.into());
        }
        validate_create_only_program_account_target(program_id, bucket_info)?;
        index.indexed_bucket_count = index
            .indexed_bucket_count
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        index.indexed_bucket_weight_bps = index
            .indexed_bucket_weight_bps
            .checked_add(params.bucket_weight_bps)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if index.indexed_bucket_weight_bps > 10_000 {
            return Err(VaultError::InvalidOracleWeightManifest.into());
        }
        OracleBucketSourceIndex {
            is_initialized: true,
            bump: bucket_bump,
            account_discriminator: OracleBucketSourceIndex::ACCOUNT_DISCRIMINATOR,
            account_version: OracleBucketSourceIndex::ACCOUNT_VERSION,
            month: *month_info.key,
            recipe_hash: month.recipe_hash,
            bucket_id: params.bucket_id,
            group_index: index.expected_bucket_count - index.indexed_bucket_count,
            bucket_weight_bps: params.bucket_weight_bps,
            ..OracleBucketSourceIndex::default()
        }
    } else {
        let bucket = load_bucket_index(program_id, &index, &params.bucket_id, bucket_info)?;
        if bucket.group_index != index.expected_bucket_count - index.indexed_bucket_count
            || bucket.first_source_index != index.remaining_source_count
            || bucket.source_ids[0] != index.last_source_id
            || params.source_id >= index.last_source_id
            || bucket.bucket_weight_bps != params.bucket_weight_bps
        {
            return Err(VaultError::InvalidOracleWeightOrder.into());
        }
        bucket
    };
    let count = usize::from(bucket.source_count);
    if count >= crate::constants::MAX_ORACLE_BUCKET_SOURCES {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    bucket.source_ids.copy_within(0..count, 1);
    bucket.source_ids[0] = params.source_id;
    bucket.source_count = bucket
        .source_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    index.remaining_source_count = index
        .remaining_source_count
        .checked_sub(1)
        .ok_or(VaultError::InvalidOracleWeightManifest)?;
    bucket.first_source_index = index.remaining_source_count;
    index.remaining_hash = params.previous_hash;
    index.last_bucket_id = params.bucket_id;
    index.last_source_id = params.source_id;
    if index.remaining_source_count == 0 {
        validate_index_completion(month_info.key, &index)?;
        index.complete = true;
    }

    if create_index {
        create_oracle_manifest_account(
            payer,
            index_info,
            system_info,
            program_id,
            month_info.key.as_ref(),
            OracleRecipeSourceIndex::LEN,
            ORACLE_RECIPE_SOURCE_INDEX_SEED,
            index_bump,
        )?;
    }
    if new_bucket {
        create_program_account(
            payer,
            bucket_info,
            system_info,
            program_id,
            OracleBucketSourceIndex::LEN,
            &[
                ORACLE_BUCKET_SOURCE_INDEX_SEED,
                month_info.key.as_ref(),
                &params.bucket_id,
                &[bucket_bump],
            ],
        )?;
    }
    store_state(bucket_info, &bucket)?;
    store_state(index_info, &index)
}
