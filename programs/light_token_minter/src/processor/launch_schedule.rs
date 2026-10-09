//! Mainnet-only timing opt-in. No demo identities, source shortcuts or risk overrides.
use super::*;

pub(super) const LAUNCH_SCHEDULE_VERSION: u8 = 3;
pub(super) const LAUNCH_PHASE_SECONDS: u64 = 3_600;
/// Written only by the consumed September council bootstrap. Original launch
/// timestamps remain in its receipt; no historical review phases are invented.
pub(super) const COUNCIL_BOOTSTRAP_SCHEDULE_VERSION: u8 = 4;
#[cfg(not(feature = "mainnet-v3"))]
pub(super) const OCTOBER_BOOTSTRAP_PENDING_SCHEDULE_VERSION: u8 = 5;

pub(super) fn schedule_windows(month: &OracleMonthState) -> Result<[u64; 4], ProgramError> {
    match month.schedule_version {
        COUNCIL_BOOTSTRAP_SCHEDULE_VERSION if cfg!(feature = "mainnet-v3") => Ok([0; 4]),
        OracleMonthState::SKU_COVERAGE_SCHEDULE_VERSION => Ok([
            ORACLE_PLACEMENT_WINDOW_SECONDS,
            ORACLE_KILL_WINDOW_SECONDS,
            ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS,
            ORACLE_OPENING_WINDOW_SECONDS,
        ]),
        LAUNCH_SCHEDULE_VERSION if cfg!(all(feature = "mainnet-v3")) => {
            Ok([LAUNCH_PHASE_SECONDS; 4])
        }
        _ => Err(VaultError::InvalidOracleState.into()),
    }
}

pub(super) fn schedule_total(month: &OracleMonthState) -> Result<u64, ProgramError> {
    schedule_windows(month)?
        .into_iter()
        .try_fold(0u64, |sum, x| {
            sum.checked_add(x)
                .ok_or(VaultError::ArithmeticOverflow.into())
        })
}

pub(super) fn schedule_review_seconds(month: &OracleMonthState) -> Result<u64, ProgramError> {
    let w = schedule_windows(month)?;
    w[1].checked_add(w[2])
        .and_then(|x| x.checked_add(w[3]))
        .ok_or(VaultError::ArithmeticOverflow.into())
}

pub(super) fn validate_launch_market(market: &Market) -> ProgramResult {
    let product: &[u8] = if padded_ascii_underlying_matches(
        &market.instrument.underlying_id,
        b"ram-standardized-baskets",
    ) {
        b"RAMX"
    } else if padded_ascii_underlying_matches(
        &market.instrument.underlying_id,
        b"nand-standardized-baskets",
    ) {
        b"NANDX"
    } else {
        return Err(VaultError::InvalidOracleState.into());
    };
    let month: &[u8] = match market.instrument.expiry_ts {
        1_790_812_800 => b"202609",
        1_793_491_200 => b"202610",
        _ => return Err(VaultError::InvalidOracleState.into()),
    };
    let side: &[u8] = match market.instrument.kind {
        crate::state::OptionKind::CallSpread => b"CALL",
        crate::state::OptionKind::PutSpread => b"PUT",
    };
    // `{product}-{month}-{side}-01` assembled in place (at most 20 ASCII bytes) instead of
    // through `format!`, which linked the core formatting machinery into the artifact.
    let mut id = [0u8; 32];
    let mut len = 0usize;
    for part in [product, b"-", month, b"-", side, b"-01"] {
        id[len..len + part.len()].copy_from_slice(part);
        len += part.len();
    }
    if !cfg!(all(feature = "mainnet-v3"))
        || !padded_ascii_underlying_matches(&market.market_id, &id[..len])
    {
        return Err(VaultError::InvalidOracleState.into());
    }
    Ok(())
}

/// No new placement/reopening after a launch deadline. Existing dispute settlement remains required.
pub(super) fn source_submission_deadline(
    month: &OracleMonthState,
    expiry: u64,
) -> Result<u64, ProgramError> {
    if month.schedule_version == LAUNCH_SCHEDULE_VERSION {
        Ok(rulebook_schedule_boundaries(month)?.0)
    } else {
        expiry
            .checked_sub(schedule_review_seconds(month)?)
            .ok_or(VaultError::ArithmeticOverflow.into())
    }
}
