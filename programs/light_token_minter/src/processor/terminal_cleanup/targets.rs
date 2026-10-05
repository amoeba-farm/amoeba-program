//! The closed set of nonfinancial native account types that may be reclaimed.
use super::*;
use crate::processor::oracle_usdc::load_oracle_usdc_reward_schedule;
use crate::state::OracleUsdcRewardSchedulePhase;

pub(super) fn validate(
    program: &Pubkey,
    month_key: &Pubkey,
    month: &OracleMonthState,
    a: &[AccountInfo],
    params: &TerminalCleanupParams,
) -> ProgramResult {
    let target = &a[0];
    if target.owner != program || target.executable || target.is_signer || !target.is_writable {
        return Err(VaultError::InvalidPda.into());
    }
    match params.kind {
        1 => {
            // Update reward registration reads this median until entitlements freeze.
            let schedule = load_oracle_usdc_reward_schedule(program, month_key, &a[1])?;
            let zero_only = schedule.total_reward_budget == 0
                && schedule.remaining_reward_budget == 0
                && schedule.trading_fee_bounty_total == 0
                && schedule.bounty_fee_sweep_finalized
                && schedule.outstanding_prelisting_escrow_count == 0;
            if schedule.phase != OracleUsdcRewardSchedulePhase::EntitlementsFinalized && !zero_only
            {
                return Err(VaultError::OracleUsdcRewardRegistrationIncomplete.into());
            }
            let bucket = load_valid_oracle_bucket_median(program, month_key, target)?;
            if !bucket.status.permits_settlement() {
                return Err(VaultError::InvalidOracleMedian.into());
            }
            Ok(())
        }
        2 => oracle_membership::validate_cleanup_bucket_index(
            program, month_key, month, &a[1], target,
        ),
        3 => oracle_membership::validate_cleanup_member_page(
            program,
            month_key,
            &params.bucket_id,
            target,
        ),
        4 => oracle_council::validate_cleanup_case(program, month_key, target),
        5 => oracle_council::validate_cleanup_round(program, month_key, target, &a[1]),
        6 => oracle_evidence::validate_cleanup_link(program, month_key, target),
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}
