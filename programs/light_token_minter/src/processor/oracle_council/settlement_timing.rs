use super::*;

/// Bucket settlement alone accepts one optional immutable bootstrap receipt.
/// Other case kinds and ordinary cohorts retain the exact existing account ABI.
#[allow(clippy::too_many_arguments)]
pub(super) fn application_accounts<'a, 'info>(
    program: &Pubkey,
    market: &Market,
    month_key: &Pubkey,
    month: &OracleMonthState,
    kind: OracleEmergencyDisputeKind,
    remaining: &'a [AccountInfo<'info>],
    slot: u64,
    opened_slot: u64,
    deadline: u64,
    majority: bool,
) -> Result<&'a [AccountInfo<'info>], ProgramError> {
    let authenticated_cohort =
        if kind == OracleEmergencyDisputeKind::BucketMedian && remaining.len() == 1 {
            september_bootstrap::validate_settlement_council_cohort(
                program,
                market,
                month_key,
                month,
                &remaining[0],
            )?;
            true
        } else {
            false
        };
    validate_application_window(slot, opened_slot, deadline, authenticated_cohort, majority)?;
    Ok(if authenticated_cohort {
        &remaining[..0]
    } else {
        remaining
    })
}

fn validate_application_window(
    slot: u64,
    opened_slot: u64,
    deadline: u64,
    authenticated_cohort: bool,
    majority: bool,
) -> ProgramResult {
    if slot < opened_slot || (slot <= deadline && !(authenticated_cohort && majority)) {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    Ok(())
}
