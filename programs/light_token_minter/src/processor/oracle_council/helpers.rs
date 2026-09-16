use super::*;

pub(in crate::processor) fn ensure_emergency_resolver_coverage_lane(
    _month: &OracleMonthState,
    kind: OracleEmergencyDisputeKind,
    coverage_aware: bool,
) -> ProgramResult {
    if coverage_aware != (kind == OracleEmergencyDisputeKind::Source) {
        return Err(VaultError::OracleSkuCoverageInstructionRequired.into());
    }
    Ok(())
}

pub(in crate::processor) fn remove_emergency_supported_candidate_coverage(
    month: &mut OracleMonthState,
    coverage: Option<&mut OracleSkuCoverageManifest>,
    coverage_record: Option<&mut OracleSkuCoverageRecord>,
    source_still_contributes: bool,
) -> Result<bool, ProgramError> {
    if coverage.is_some() != coverage_record.is_some() {
        return Err(VaultError::InvalidAccountList.into());
    }
    if !source_still_contributes {
        return Ok(false);
    }
    decrement_oracle_supported_candidate_count(month)?;
    if let (Some(coverage), Some(record)) = (coverage, coverage_record) {
        remove_supported_source_from_sku_coverage(coverage, record)?;
    }
    Ok(true)
}

pub(in crate::processor) fn reopen_coverage_after_delayed_emergency_resolution(
    month: &mut OracleMonthState,
    coverage: &mut OracleSkuCoverageManifest,
    now: u64,
) -> ProgramResult {
    if now >= rulebook_schedule_boundaries(month)?.2 {
        coverage.coverage_finalized = false;
        coverage.coverage_complete_ts = 0;
        month.phase = OraclePhase::SourceSubmission;
    }
    Ok(())
}
