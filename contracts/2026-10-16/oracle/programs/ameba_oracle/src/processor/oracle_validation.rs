use super::*;
use crate::compact_error::CompactAccountInfo;

pub(super) fn validate_system_program(system_program_info: &AccountInfo) -> ProgramResult {
    if !crate::is_system_program(system_program_info.key) {
        return Err(OracleError::InvalidSystemProgram.into());
    }
    Ok(())
}

/// Authorizes create-only oracle setup both during the paused bootstrap and after activation.
/// Unlike ordinary dual-role maintenance lanes, these protocol identities remain oracle-only.
pub(super) fn validate_current_oracle_authority_allow_paused(
    program_id: &Pubkey,
    signer_info: &AccountInfo,
    config_info: &AccountInfo,
) -> Result<OracleConfig, ProgramError> {
    if !signer_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_current_canonical_vault_config(program_id, config_info)?;
    if *signer_info.key != config.oracle_authority {
        return Err(OracleError::Unauthorized.into());
    }
    Ok(config)
}

/// Rent funding carries no protocol authority; semantic authorization is checked separately.
pub(super) fn validate_current_account_creation_payer(payer_info: &AccountInfo) -> ProgramResult {
    if !payer_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    Ok(())
}

/// Admin or oracle authority on an unpaused config. The value-flow and maintenance lanes of
/// later instructions use this guard.
pub(super) fn validate_oracle_authority(
    program_id: &Pubkey,
    signer_info: &AccountInfo,
    config_info: &AccountInfo,
) -> ProgramResult {
    if !signer_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.paused {
        return Err(OracleError::ContractPaused.into());
    }
    if *signer_info.key != config.admin && *signer_info.key != config.oracle_authority {
        return Err(OracleError::Unauthorized.into());
    }
    Ok(())
}

pub(super) fn validate_oracle_recipe_weight_actor(
    config: &OracleConfig,
    actor: &Pubkey,
) -> ProgramResult {
    if *actor != config.admin && *actor != config.oracle_authority {
        return Err(OracleError::Unauthorized.into());
    }
    Ok(())
}

/// The oracle `Period` that a month forms and settles. It sits in the account slot that carries
/// the month's anchor; its product is the month's underlying and its immutable expiry is the
/// month's expiry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PeriodAnchor {
    pub(super) underlying_id: [u8; 32],
    pub(super) expiry_ts: u64,
    pub(super) period: Period,
}

/// Loads the period through the consumer contract's reader bound to this program id: owner,
/// exact length, discriminator, version, stored-bump PDA, status invariants and zero reserved
/// bytes. Any failure is the one invalid-anchor error.
#[inline(never)]
pub(super) fn load_valid_period_anchor(
    program_id: &Pubkey,
    period_info: &AccountInfo,
) -> Result<PeriodAnchor, ProgramError> {
    let period = ameba_oracle_interface::load_period(program_id, period_info)
        .map_err(|_| OracleError::InvalidPeriodAccount)?;
    Ok(PeriodAnchor {
        underlying_id: period.product_id,
        expiry_ts: period.expiry_ts,
        period,
    })
}

/// Loads the period in the month's anchor slot and the month bound to it. The month PDA is
/// derived from the period key and the period's expiry; the month's anchor field must name the
/// same period.
#[inline(never)]
pub(super) fn load_valid_period_and_oracle_month(
    program_id: &Pubkey,
    period_info: &AccountInfo,
    month_info: &AccountInfo,
) -> Result<(PeriodAnchor, OracleMonthState), ProgramError> {
    let anchor = load_valid_period_anchor(program_id, period_info)?;
    let month = load_valid_oracle_month(program_id, period_info, month_info, &anchor)?;
    Ok((anchor, month))
}

pub(super) fn load_valid_oracle_month(
    program_id: &Pubkey,
    period_info: &AccountInfo,
    month_info: &AccountInfo,
    anchor: &PeriodAnchor,
) -> Result<OracleMonthState, ProgramError> {
    let month = load_oracle_month_state(month_info, program_id)?;
    let (expected_month, expected_bump) =
        derive_oracle_month_pda(program_id, period_info.key, anchor.expiry_ts);
    if *month_info.key != expected_month
        || !month.is_initialized
        || month.bump != expected_bump
        || month.period != *period_info.key
    {
        return Err(OracleError::InvalidOracleMonthAccount.into());
    }
    Ok(month)
}

pub(super) fn load_oracle_month_state(
    account_info: &AccountInfo,
    program_id: &Pubkey,
) -> Result<OracleMonthState, ProgramError> {
    if account_info.owner != program_id || account_info.data_len() != OracleMonthState::LEN {
        return Err(OracleError::InvalidOracleMonthAccount.into());
    }
    let data = account_info.try_data()?;
    let month = crate::fixed_codec::decode_oracle_month(
        data.as_ref(),
        OracleError::InvalidOracleMonthAccount,
    )?;
    if !month.has_current_layout() {
        return Err(OracleError::InvalidOracleMonthAccount.into());
    }
    Ok(month)
}

pub(super) fn store_oracle_month_state(
    account_info: &AccountInfo,
    month: &OracleMonthState,
) -> ProgramResult {
    if account_info.data_len() != OracleMonthState::LEN {
        return Err(OracleError::InvalidOracleMonthAccount.into());
    }
    store_state(account_info, month)
}

pub(super) fn load_canonical_oracle_economics_config(
    program_id: &Pubkey,
    economics_info: &AccountInfo,
) -> Result<OracleEconomicsConfig, ProgramError> {
    let economics: OracleEconomicsConfig = load_exact_zero_padded_state(
        economics_info,
        program_id,
        OracleEconomicsConfig::LEN,
        OracleError::InvalidOracleState,
    )?;
    let (expected, bump) = derive_oracle_economics_config_pda(program_id);
    if *economics_info.key != expected
        || !economics.is_initialized
        || economics.bump != bump
        || economics.account_discriminator != OracleEconomicsConfig::ACCOUNT_DISCRIMINATOR
        || economics.account_version != OracleEconomicsConfig::ACCOUNT_VERSION
        || economics.config_version == 0
    {
        return Err(OracleError::InvalidOracleState.into());
    }
    validate_oracle_economics(&economics.economics)?;
    Ok(economics)
}

#[inline(always)]
pub(super) fn load_valid_oracle_source(
    program_id: &Pubkey,
    month: &Pubkey,
    source_info: &AccountInfo,
) -> Result<OracleSourceState, ProgramError> {
    load_oracle_source_inner(program_id, Some(month), source_info)
}

#[inline(never)]
fn load_oracle_source_inner(
    program_id: &Pubkey,
    expected_month: Option<&Pubkey>,
    source_info: &AccountInfo,
) -> Result<OracleSourceState, ProgramError> {
    let source: OracleSourceState = load_exact_zero_padded_state(
        source_info,
        program_id,
        OracleSourceState::LEN,
        OracleError::InvalidOracleSourceAccount,
    )?;
    let month = expected_month.unwrap_or(&source.month);
    let (expected_source, expected_bump) =
        derive_oracle_source_pda(program_id, month, &source.source_id);
    if *source_info.key != expected_source
        || !source.is_initialized
        || source.bump != expected_bump
        || source.month != *month
    {
        return Err(OracleError::InvalidOracleSourceAccount.into());
    }
    if source.status == OracleSourceStatus::Active {
        if !source.opening_submitted
            || source.baseline_state == 0
            || source.current_state == 0
            || source.observation_count == 0
            || crate::bytes32_is_zero(&source.rolling_observation_hash)
        {
            return Err(OracleError::InvalidOracleObservation.into());
        }
    } else if source.observation_count != 0
        || !crate::bytes32_is_zero(&source.rolling_observation_hash)
    {
        return Err(OracleError::InvalidOracleObservation.into());
    }
    Ok(source)
}

pub(super) fn load_valid_oracle_source_observations(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    observations_info: &AccountInfo,
) -> Result<Box<OracleSourceObservations>, ProgramError> {
    let observations = Box::new(load_exact_zero_padded_state::<OracleSourceObservations>(
        observations_info,
        program_id,
        OracleSourceObservations::LEN,
        OracleError::InvalidOracleObservation,
    )?);
    let (expected, bump) = derive_oracle_source_observations_pda(program_id, source);
    if *observations_info.key != expected
        || !observations.is_initialized
        || observations.bump != bump
        || observations.account_discriminator != OracleSourceObservations::ACCOUNT_DISCRIMINATOR
        || !matches!(
            observations.account_version,
            OracleSourceObservations::ACCOUNT_VERSION
                | OracleSourceObservations::INHERITED_ANCHOR_VERSION
        )
        || observations.month != *month
        || observations.source != *source
    {
        return Err(OracleError::InvalidOracleObservation.into());
    }
    Ok(observations)
}

pub(super) fn load_valid_oracle_sku_coverage_manifest(
    program_id: &Pubkey,
    month: &Pubkey,
    coverage_info: &AccountInfo,
) -> Result<OracleSkuCoverageManifest, ProgramError> {
    let coverage: OracleSkuCoverageManifest = load_exact_zero_padded_state(
        coverage_info,
        program_id,
        OracleSkuCoverageManifest::LEN,
        OracleError::InvalidOracleSkuCoverageManifest,
    )?;
    let (expected, expected_bump) = derive_oracle_sku_coverage_manifest_pda(program_id, month);
    if *coverage_info.key != expected
        || !coverage.is_initialized
        || !coverage.has_canonical_layout()
        || coverage.bump != expected_bump
        || coverage.month != *month
        || crate::bytes32_is_zero(&coverage.required_sku_root)
        || coverage.required_sku_count == 0
        || coverage.required_sku_count > MAX_ORACLE_REQUIRED_SKUS
        || coverage.covered_sku_count > coverage.required_sku_count
    {
        return Err(OracleError::InvalidOracleSkuCoverageManifest.into());
    }
    Ok(coverage)
}

pub(super) fn load_valid_oracle_sku_coverage_record(
    program_id: &Pubkey,
    month: &Pubkey,
    sku_id: &[u8; 32],
    record_info: &AccountInfo,
) -> Result<OracleSkuCoverageRecord, ProgramError> {
    let record: OracleSkuCoverageRecord = load_exact_zero_padded_state(
        record_info,
        program_id,
        OracleSkuCoverageRecord::LEN,
        OracleError::InvalidOracleSkuCoverageRecord,
    )?;
    let (expected, expected_bump) =
        derive_oracle_sku_coverage_record_pda(program_id, month, sku_id);
    if *record_info.key != expected
        || !record.is_initialized
        || !record.has_canonical_layout()
        || record.bump != expected_bump
        || record.month != *month
        || record.sku_id != *sku_id
    {
        return Err(OracleError::InvalidOracleSkuCoverageRecord.into());
    }
    Ok(record)
}

pub(super) fn load_valid_oracle_recipe_weight_manifest(
    program_id: &Pubkey,
    month: &Pubkey,
    manifest_info: &AccountInfo,
) -> Result<OracleRecipeWeightManifest, ProgramError> {
    let manifest: OracleRecipeWeightManifest = load_exact_zero_padded_state(
        manifest_info,
        program_id,
        OracleRecipeWeightManifest::LEN,
        OracleError::InvalidOracleWeightManifest,
    )?;
    let (expected, expected_bump) = derive_oracle_recipe_weight_manifest_pda(program_id, month);
    if *manifest_info.key != expected
        || !manifest.is_initialized
        || manifest.account_discriminator != OracleRecipeWeightManifest::ACCOUNT_DISCRIMINATOR
        || manifest.account_version != OracleRecipeWeightManifest::ACCOUNT_VERSION
        || manifest.bump != expected_bump
        || manifest.month != *month
    {
        return Err(OracleError::InvalidOracleWeightManifest.into());
    }
    Ok(manifest)
}

pub(super) fn load_valid_oracle_settlement_source_manifest(
    program_id: &Pubkey,
    month: &Pubkey,
    manifest_info: &AccountInfo,
) -> Result<OracleSettlementSourceManifest, ProgramError> {
    let manifest: OracleSettlementSourceManifest = load_exact_zero_padded_state(
        manifest_info,
        program_id,
        OracleSettlementSourceManifest::LEN,
        OracleError::InvalidOracleWeightManifest,
    )?;
    let (expected, bump) =
        crate::state::derive_oracle_settlement_source_manifest_pda(program_id, month);
    if !manifest.is_initialized
        || manifest.account_discriminator != OracleSettlementSourceManifest::ACCOUNT_DISCRIMINATOR
        || manifest.account_version != OracleSettlementSourceManifest::ACCOUNT_VERSION
        || manifest.bump != bump
        || manifest.month != *month
        || *manifest_info.key != expected
    {
        return Err(OracleError::InvalidOracleWeightManifest.into());
    }
    Ok(manifest)
}

pub(super) fn validate_oracle_recipe_weight_build_context(
    anchor: &PeriodAnchor,
    month: &OracleMonthState,
    manifest: &OracleRecipeWeightManifest,
) -> ProgramResult {
    if month.weight_scheme_version != 255
        || month.pending_resolution_count != 0
        || manifest.phase == OracleRecipeWeightPhase::Finalized
    {
        return Err(OracleError::InvalidOracleWeightManifest.into());
    }
    ensure_oracle_freeze_window(anchor, month)
}

pub(super) fn validate_oracle_recipe_weight_source(
    manifest: &OracleRecipeWeightManifest,
    source: &OracleSourceState,
) -> ProgramResult {
    let valid_status = matches!(
        source.status,
        OracleSourceStatus::Candidate | OracleSourceStatus::Frozen
    );
    if !valid_status
        || source.bucket_id != manifest.current_bucket_id
        || crate::bytes32_is_zero(&source.source_id)
        || source.support_stake_total == 0
    {
        return Err(OracleError::InvalidOracleWeightSource.into());
    }
    Ok(())
}

pub(super) fn load_valid_oracle_opening_claim(
    program_id: &Pubkey,
    month: &Pubkey,
    source_key: &Pubkey,
    claim_info: &AccountInfo,
    source: &OracleSourceState,
) -> Result<OracleOpeningClaim, ProgramError> {
    let mut claim: OracleOpeningClaim = load_state(claim_info, program_id)?;
    bind_oracle_opening_claim_context(&mut claim, month, source_key, source);
    let (expected_claim, expected_bump) =
        derive_oracle_opening_claim_pda(program_id, month, source_key);
    if *claim_info.key != expected_claim
        || claim.bump != expected_bump
        || !claim.is_initialized
        || claim.month != *month
        || claim.source != *source_key
        || claim.source_id != source.source_id
        || claim.canonical_locator_hash != source.canonical_locator_hash
        || claim.source_definition_hash != source.source_definition_hash
        || crate::bytes32_is_zero(&claim.archive_url_hash)
        || crate::bytes32_is_zero(&claim.evidence_hash)
    {
        return Err(OracleError::InvalidOracleOpeningClaim.into());
    }
    Ok(claim)
}

/// Rehydrates the compact claim projection from identities already authenticated by the account
/// list and the source loader. These values are absent from the physical claim bytes, but remain
/// present in the logical state consumed by every caller.
pub(super) fn bind_oracle_opening_claim_context(
    claim: &mut OracleOpeningClaim,
    month: &Pubkey,
    source_key: &Pubkey,
    source: &OracleSourceState,
) {
    claim.month = *month;
    claim.source = *source_key;
    claim.source_id = source.source_id;
    claim.canonical_locator_hash = source.canonical_locator_hash;
    claim.source_definition_hash = source.source_definition_hash;
}
