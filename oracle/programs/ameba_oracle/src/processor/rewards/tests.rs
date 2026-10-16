use super::*;

fn failed_v5_month() -> OracleMonthState {
    OracleMonthState {
        phase: OraclePhase::SourceSubmission,
        pending_resolution_count: 0,
        weight_scheme_version: 0,
        ..OracleMonthState::default()
    }
}

#[test]
fn latest_safe_boundary_is_abort_eligible_even_if_coverage_count_is_full() {
    let month = failed_v5_month();
    let expiry = 1_000_000;
    let preserved = ORACLE_KILL_WINDOW_SECONDS
        + ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
        + ORACLE_OPENING_WINDOW_SECONDS;
    let latest_safe = expiry - preserved;
    assert!(ensure_failed_v5_reward_schedule_window_at(expiry, &month, latest_safe - 1).is_err());
    assert!(ensure_failed_v5_reward_schedule_window_at(expiry, &month, latest_safe).is_ok());
    // Coverage count is intentionally not an input: a full-but-unfinalized manifest at this
    // boundary is still unlistable and must be able to release its reservation.
}

#[test]
fn abort_releases_the_reservation_once_only_after_all_counted_escrows() {
    let mut vault = OracleUsdcRewardVault {
        total_reserved: 700,
        ..OracleUsdcRewardVault::default()
    };
    let mut schedule = OracleUsdcRewardSchedule {
        phase: OracleUsdcRewardSchedulePhase::Funded,
        total_reward_budget: 500,
        remaining_reward_budget: 500,
        outstanding_prelisting_escrow_count: 1,
        ..OracleUsdcRewardSchedule::default()
    };
    assert!(release_aborted_reward_reservation(&mut vault, &mut schedule).is_err());
    schedule.outstanding_prelisting_escrow_count = 0;
    release_aborted_reward_reservation(&mut vault, &mut schedule).unwrap();
    assert_eq!(vault.total_reserved, 200);
    assert_eq!(schedule.remaining_reward_budget, 0);
    assert_eq!(schedule.phase, OracleUsdcRewardSchedulePhase::Aborted);
    assert!(release_aborted_reward_reservation(&mut vault, &mut schedule).is_err());
    assert_eq!(vault.total_reserved, 200);
}

#[test]
fn counted_escrow_decrement_is_exact_and_cannot_underflow() {
    let mut schedule = OracleUsdcRewardSchedule {
        outstanding_prelisting_escrow_count: 1,
        ..OracleUsdcRewardSchedule::default()
    };
    decrement_counted_v5_schedule_at(&mut schedule, 1, 7).unwrap();
    assert_eq!(schedule.outstanding_prelisting_escrow_count, 0);
    assert!(decrement_counted_v5_schedule_at(&mut schedule, 1, 8).is_err());
}
