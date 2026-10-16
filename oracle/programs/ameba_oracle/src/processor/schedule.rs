//! The rulebook calendar generation every month uses: 4/2/1/1 days of placement, challenge,
//! resolution-freeze and opening before listing.
use super::*;

pub(super) fn schedule_windows(month: &OracleMonthState) -> Result<[u64; 4], ProgramError> {
    match month.schedule_version {
        OracleMonthState::SKU_COVERAGE_SCHEDULE_VERSION => Ok([
            ORACLE_PLACEMENT_WINDOW_SECONDS,
            ORACLE_KILL_WINDOW_SECONDS,
            ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS,
            ORACLE_OPENING_WINDOW_SECONDS,
        ]),
        _ => Err(OracleError::InvalidOracleState.into()),
    }
}

pub(super) fn schedule_total(month: &OracleMonthState) -> Result<u64, ProgramError> {
    schedule_windows(month)?
        .into_iter()
        .try_fold(0u64, |sum, x| {
            sum.checked_add(x)
                .ok_or(OracleError::ArithmeticOverflow.into())
        })
}

pub(super) fn schedule_review_seconds(month: &OracleMonthState) -> Result<u64, ProgramError> {
    let w = schedule_windows(month)?;
    w[1].checked_add(w[2])
        .and_then(|x| x.checked_add(w[3]))
        .ok_or(OracleError::ArithmeticOverflow.into())
}

/// No new placement or reopening once the remaining time before expiry can no longer hold the
/// full post-submission review calendar. Existing dispute settlement remains required.
pub(super) fn source_submission_deadline(
    month: &OracleMonthState,
    expiry: u64,
) -> Result<u64, ProgramError> {
    expiry
        .checked_sub(schedule_review_seconds(month)?)
        .ok_or(OracleError::ArithmeticOverflow.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_current_schedule_generation_has_windows() {
        let ordinary = OracleMonthState {
            scramble_start_ts: 100,
            listing_ts: 100 + ORACLE_PRE_LISTING_WINDOW_SECONDS,
            ..OracleMonthState::default()
        };
        assert_eq!(
            schedule_total(&ordinary).unwrap(),
            ORACLE_PRE_LISTING_WINDOW_SECONDS
        );
        assert_eq!(
            schedule_review_seconds(&ordinary).unwrap(),
            ORACLE_KILL_WINDOW_SECONDS
                + ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
                + ORACLE_OPENING_WINDOW_SECONDS
        );
        assert!(rulebook_schedule_boundaries(&ordinary).is_ok());
        for schedule_version in [0u8, 1, 3, 4, 5, u8::MAX] {
            let unknown = OracleMonthState {
                schedule_version,
                ..ordinary.clone()
            };
            assert_eq!(
                schedule_windows(&unknown),
                Err(OracleError::InvalidOracleState.into())
            );
            assert!(source_submission_deadline(&unknown, 1_000_000).is_err());
        }
    }

    #[test]
    fn source_submission_deadline_preserves_the_full_review_calendar() {
        let month = OracleMonthState::default();
        let expiry = 2_000_000;
        let review = ORACLE_KILL_WINDOW_SECONDS
            + ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
            + ORACLE_OPENING_WINDOW_SECONDS;
        assert_eq!(
            source_submission_deadline(&month, expiry),
            Ok(expiry - review)
        );
        assert_eq!(
            source_submission_deadline(&month, review - 1),
            Err(OracleError::ArithmeticOverflow.into())
        );
    }
}
