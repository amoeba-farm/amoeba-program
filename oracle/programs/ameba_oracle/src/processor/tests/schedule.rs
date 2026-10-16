use super::*;

#[test]
fn rulebook_schedule_rejects_an_expiry_anchored_or_gapped_listing() {
    let invalid = OracleMonthState {
        scramble_start_ts: 1_000_000,
        listing_ts: 1_000_000 + ORACLE_PRE_LISTING_WINDOW_SECONDS + 1,
        ..OracleMonthState::default()
    };
    assert_eq!(
        rulebook_schedule_boundaries(&invalid),
        Err(ProgramError::Custom(OracleError::InvalidOracleState as u32))
    );
}

#[test]
fn late_month_initialization_preserves_every_post_submission_window() {
    let preserved_post_submission_seconds = ORACLE_KILL_WINDOW_SECONDS
        + ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
        + ORACLE_OPENING_WINDOW_SECONDS;
    let expiry_ts = 2_000_000u64;

    assert_eq!(
        ensure_oracle_month_initialization_review_window(
            expiry_ts - preserved_post_submission_seconds - 1,
            expiry_ts,
        ),
        Ok(())
    );
    assert_eq!(
        ensure_oracle_month_initialization_review_window(
            expiry_ts - preserved_post_submission_seconds,
            expiry_ts,
        ),
        Err(ProgramError::Custom(
            OracleError::OracleSkuCoverageExtensionTooLate as u32,
        ))
    );
    assert_eq!(
        ensure_oracle_month_initialization_review_window(u64::MAX, expiry_ts),
        Err(ProgramError::Custom(OracleError::ArithmeticOverflow as u32))
    );
}

#[test]
fn rolling_three_month_calendar_uses_the_protocol_midnight_utc_boundary() {
    // 2030-11-01T00:00:00Z -> 2031-02-01T00:00:00Z.
    assert_eq!(
        validate_rolling_three_month_maturity(1_919_721_600, 1_927_670_400),
        Ok(())
    );
    // A self-consistent private 00:00:01 roll is still forbidden.
    assert_eq!(
        validate_rolling_three_month_maturity(1_919_721_601, 1_927_670_401),
        Err(ProgramError::Custom(
            OracleError::InvalidOracleMaturityLadder as u32
        ))
    );
}

#[test]
fn rolling_three_month_calendar_rejects_wrong_month_and_non_day_one() {
    assert_eq!(
        validate_rolling_three_month_maturity(1_919_721_600, 1_924_992_000),
        Err(ProgramError::Custom(
            OracleError::InvalidOracleMaturityLadder as u32
        ))
    );
    assert_eq!(
        validate_rolling_three_month_maturity(1_919_808_000, 1_927_670_400),
        Err(ProgramError::Custom(
            OracleError::InvalidOracleMaturityLadder as u32
        ))
    );
}

#[test]
fn rolling_three_month_calendar_crosses_year_ends_and_leap_days() {
    // 2026-12-01 -> 2027-03-01, the first futures-only cohort rung.
    assert_eq!(
        validate_rolling_three_month_maturity(1_796_083_200, 1_803_859_200),
        Ok(())
    );
    // 2027-11-01 -> 2028-02-01 and 2027-12-01 -> 2028-03-01 straddle a leap February.
    assert_eq!(
        validate_rolling_three_month_maturity(1_825_027_200, 1_832_976_000),
        Ok(())
    );
    assert_eq!(
        validate_rolling_three_month_maturity(1_827_619_200, 1_835_481_600),
        Ok(())
    );
    // One and two months are not the rolling rung; neither is four.
    for expiry in [1_798_761_600, 1_801_440_000, 1_806_537_600] {
        assert_eq!(
            validate_rolling_three_month_maturity(1_796_083_200, expiry),
            Err(ProgramError::Custom(
                OracleError::InvalidOracleMaturityLadder as u32
            )),
            "expiry {expiry}"
        );
    }
    assert_eq!(utc_calendar_parts(0), Ok((1970, 1, 1, 0)));
    assert_eq!(utc_calendar_parts(951_782_400), Ok((2000, 2, 29, 0)));
    assert_eq!(
        utc_calendar_parts(1_796_083_199),
        Ok((2026, 11, 30, 86_399))
    );
}

#[test]
fn maturity_ladder_allows_every_period_in_the_same_planned_rung() {
    let registry = maturity_ladder_fixture(1_919_721_600, 1_927_670_400);
    assert_eq!(
        validate_oracle_maturity_ladder_transition(
            &registry,
            registry.planned_listing_ts,
            registry.planned_expiry_ts,
            registry.planned_listing_ts - 1,
        ),
        Ok(OracleMaturityLadderTransition::SameRung)
    );
}

#[test]
fn maturity_ladder_advances_one_utc_month_only_after_prior_listing() {
    // 2030-11-01/2031-02-01 -> 2030-12-01/2031-03-01.
    let registry = maturity_ladder_fixture(1_919_721_600, 1_927_670_400);
    assert_eq!(
        validate_oracle_maturity_ladder_transition(
            &registry,
            1_922_313_600,
            1_930_089_600,
            registry.planned_listing_ts,
        ),
        Ok(OracleMaturityLadderTransition::Advance)
    );
    assert_eq!(
        validate_oracle_maturity_ladder_transition(
            &registry,
            1_922_313_600,
            1_930_089_600,
            registry.planned_listing_ts - 1,
        ),
        Err(ProgramError::Custom(
            OracleError::InvalidOracleMaturityLadder as u32
        ))
    );
}

#[test]
fn maturity_ladder_rejects_skips_backtracking_and_duplicate_future_staging() {
    let november = maturity_ladder_fixture(1_919_721_600, 1_927_670_400);
    // Skipping directly to January is forbidden even after November lists.
    assert_eq!(
        validate_oracle_maturity_ladder_transition(
            &november,
            1_924_992_000,
            1_932_768_000,
            november.planned_listing_ts,
        ),
        Err(ProgramError::Custom(
            OracleError::InvalidOracleMaturityLadder as u32
        ))
    );
    // A valid October/January pair still cannot move the cursor backward.
    assert_eq!(
        validate_oracle_maturity_ladder_transition(
            &november,
            1_917_043_200,
            1_924_992_000,
            november.planned_listing_ts,
        ),
        Err(ProgramError::Custom(
            OracleError::InvalidOracleMaturityLadder as u32
        ))
    );

    // Once December is staged, January cannot also be staged before December lists.
    let december = maturity_ladder_fixture(1_922_313_600, 1_930_089_600);
    assert_eq!(
        validate_oracle_maturity_ladder_transition(
            &december,
            1_924_992_000,
            1_932_768_000,
            november.planned_listing_ts,
        ),
        Err(ProgramError::Custom(
            OracleError::InvalidOracleMaturityLadder as u32
        ))
    );
}

#[test]
fn oracle_economics_validate_current_emergency_fields() {
    let current = OracleEconomicParams::default();
    assert_eq!(validate_oracle_economics(&current), Ok(()));
    for invalid in [
        OracleEconomicParams {
            emergency_supermajority_bps: 0,
            ..current.clone()
        },
        OracleEconomicParams {
            emergency_supermajority_bps: 10_001,
            ..current.clone()
        },
        OracleEconomicParams {
            emergency_commit_window_slots: 0,
            ..current.clone()
        },
        OracleEconomicParams {
            emergency_reveal_window_slots: 0,
            ..current.clone()
        },
    ] {
        assert_eq!(
            validate_oracle_economics(&invalid),
            Err(ProgramError::Custom(OracleError::InvalidOracleState as u32))
        );
    }
}
