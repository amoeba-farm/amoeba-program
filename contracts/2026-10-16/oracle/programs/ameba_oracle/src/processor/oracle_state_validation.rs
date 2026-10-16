use super::*;

pub(super) fn decrement_pending_oracle_resolution(month: &mut OracleMonthState) -> ProgramResult {
    month.pending_resolution_count = month
        .pending_resolution_count
        .checked_sub(1)
        .ok_or(OracleError::InvalidOracleState)?;
    Ok(())
}

pub(super) fn increment_oracle_supported_candidate_count(
    month: &mut OracleMonthState,
) -> ProgramResult {
    month.source_count = month
        .source_count
        .checked_add(1)
        .ok_or(OracleError::ArithmeticOverflow)?;
    Ok(())
}

pub(super) fn decrement_oracle_supported_candidate_count(
    month: &mut OracleMonthState,
) -> ProgramResult {
    month.source_count = month
        .source_count
        .checked_sub(1)
        .ok_or(OracleError::InvalidOracleState)?;
    Ok(())
}

pub(super) fn decrement_pending_oracle_source_challenge_if_terminal(
    month: &mut OracleMonthState,
    outcome: OracleSourceChallengeOutcome,
) -> ProgramResult {
    if outcome != OracleSourceChallengeOutcome::RuleReviewUnresolved {
        decrement_pending_oracle_resolution(month)?;
    }
    Ok(())
}

pub(super) fn ensure_oracle_opening_resolution_complete(month: &OracleMonthState) -> ProgramResult {
    ensure_oracle_opening_sources_terminal(month)?;
    ensure_oracle_active_weight_scheme(month)
}

pub(super) fn ensure_finalized_oracle_active_weight_manifest(
    month: &OracleMonthState,
    manifest: &OracleActiveWeightManifest,
) -> ProgramResult {
    ensure_oracle_opening_resolution_complete(month)?;
    if month.pending_resolution_count != 0 {
        return Err(OracleError::InvalidOracleState.into());
    }
    if manifest.phase != OracleRecipeWeightPhase::Finalized
        || manifest.rolling_manifest_hash != month.active_weight_manifest_hash
        || manifest.expected_source_count != month.frozen_source_count
        || manifest.processed_source_count != manifest.expected_source_count
        || manifest.expected_group_count != month.active_weight_group_count
        || manifest.processed_group_count != manifest.expected_group_count
        || month.opened_source_count == 0
        || manifest.processed_bucket_weight_bps != 10_000
    {
        return Err(OracleError::OracleActiveWeightSchemeUnverified.into());
    }
    Ok(())
}

/// Complete, on-time coverage of a month that is about to issue. Kept with its unit test; no
/// instruction of this program calls it yet.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn ensure_finalized_oracle_issue_sku_coverage(
    month: &OracleMonthState,
    coverage: &OracleSkuCoverageManifest,
    active_manifest: &OracleActiveWeightManifest,
) -> ProgramResult {
    if !coverage.coverage_finalized
        || coverage.coverage_complete_ts == 0
        || coverage.covered_sku_count != coverage.required_sku_count
    {
        return Err(OracleError::OracleSkuCoverageIncomplete.into());
    }
    let planned_listing_ts = coverage
        .planned_scramble_start_ts
        .checked_add(schedule_total(month)?)
        .ok_or(OracleError::InvalidOracleSkuCoverageManifest)?;
    if coverage.planned_scramble_start_ts == 0
        || coverage.planned_listing_ts != planned_listing_ts
        || coverage.planned_listing_ts > month.game_start_ts()
        || coverage.coverage_complete_ts < coverage.planned_scramble_start_ts
        || coverage.coverage_complete_ts >= month.game_start_ts()
    {
        return Err(OracleError::InvalidOracleSkuCoverageManifest.into());
    }
    if active_manifest.expected_group_count != coverage.required_sku_count {
        return Err(OracleError::InvalidOracleSkuCoverageManifest.into());
    }
    Ok(())
}

pub(super) fn ensure_oracle_opening_sources_terminal(month: &OracleMonthState) -> ProgramResult {
    ensure_oracle_canonical_weight_scheme(month)?;
    if crate::bytes32_is_zero(&month.recipe_hash)
        || month.frozen_source_count == 0
        || month.opening_resolved_source_count != month.frozen_source_count
    {
        return Err(OracleError::InvalidOracleState.into());
    }
    Ok(())
}

pub(super) fn ensure_oracle_active_weight_scheme(month: &OracleMonthState) -> ProgramResult {
    if month.active_weight_scheme_version != OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION
        || month.active_weight_group_count == 0
        || crate::bytes32_is_zero(&month.active_weight_manifest_hash)
    {
        return Err(OracleError::OracleActiveWeightSchemeUnverified.into());
    }
    Ok(())
}

pub(super) fn ensure_oracle_canonical_weight_scheme(month: &OracleMonthState) -> ProgramResult {
    if month.weight_scheme_version != 1
        || month.effective_weight_total_bps != 10_000
        || crate::bytes32_is_zero(&month.weight_manifest_hash)
    {
        return Err(OracleError::OracleWeightSchemeUnverified.into());
    }
    Ok(())
}

pub(super) fn ensure_oracle_month_ready_for_settlement(month: &OracleMonthState) -> ProgramResult {
    ensure_oracle_opening_resolution_complete(month)?;
    if month.settlement_status != OracleSettlementStatus::Final {
        return Err(OracleError::InvalidOracleState.into());
    }
    Ok(())
}

pub(super) fn ensure_oracle_month_ready_to_close(
    month: &OracleMonthState,
    settlement_record: &Pubkey,
) -> ProgramResult {
    ensure_oracle_opening_resolution_complete(month)?;
    if month.phase != OraclePhase::Settled {
        return Err(OracleError::InvalidOraclePhase.into());
    }
    if month.settlement_record != Some(*settlement_record) {
        return Err(OracleError::InvalidOracleState.into());
    }
    Ok(())
}
