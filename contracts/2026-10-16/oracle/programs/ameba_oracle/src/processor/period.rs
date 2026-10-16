//! The consumer-facing `Period` lifecycle. Periods start Forming here; the Live, Settled and
//! Failed transitions are written by the instructions that complete each stage of formation.
//! Each transition has exactly one writer below.

use super::*;

/// Tag 1. Creates one Forming period for a product and fixed expiry. The listing and expiry sit
/// on the rolling three-month ladder, and every period publishes six-decimal values. The localnet
/// dev profile accepts any expiry after now and a planned listing before it.
#[inline(never)]
pub(super) fn process_create_period(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: CreatePeriodParams,
) -> ProgramResult {
    let [payer, config, authority, period, system] = accounts else {
        return Err(OracleError::InvalidAccountList.into());
    };
    if !payer.is_signer
        || !payer.is_writable
        || config.is_signer
        || config.is_writable
        || !authority.is_signer
        || period.is_signer
        || !period.is_writable
        || system.key != &system_program::ID
    {
        return Err(OracleError::InvalidAccountList.into());
    }
    validate_current_oracle_authority_allow_paused(program_id, authority, config)?;
    let CreatePeriodParams {
        product_id,
        planned_listing_ts,
        expiry_ts,
        value_decimals,
    } = params;
    let now = current_unix_timestamp().map_err(|_| OracleError::InvalidPeriodSchedule)?;
    if product_id == [0; 32]
        || planned_listing_ts >= expiry_ts
        || now >= expiry_ts
        || value_decimals != PERIOD_VALUE_DECIMALS
    {
        return Err(OracleError::InvalidPeriodSchedule.into());
    }
    #[cfg(not(any(feature = "localnet-dev-authority", feature = "localnet-governance")))]
    validate_rolling_three_month_maturity(planned_listing_ts, expiry_ts)?;
    #[cfg(any(feature = "localnet-dev-authority", feature = "localnet-governance"))]
    solana_program::msg!(super::dev_bootstrap::DEV_AUTHORITY_LOG_MARKER);
    let (period_key, bump) =
        ameba_oracle_interface::find_period_address(program_id, &product_id, expiry_ts);
    if period.key != &period_key {
        return Err(ProgramError::InvalidSeeds);
    }
    let state = Period::new_forming(
        bump,
        product_id,
        planned_listing_ts,
        expiry_ts,
        value_decimals,
    );
    state
        .validate()
        .map_err(|_| OracleError::InvalidPeriodSchedule)?;
    create_new_program_account(
        payer,
        period,
        system,
        program_id,
        Period::LEN,
        &[
            ameba_oracle_interface::PERIOD_SEED,
            &product_id,
            &expiry_ts.to_le_bytes(),
            &[bump],
        ],
    )?;
    period
        .try_borrow_mut_data()?
        .copy_from_slice(&state.encode());
    Ok(())
}

/// Forming -> Live, written by tag 80 when its month enters Game. The period becomes tradable from
/// the month's listing, which late coverage can only move later than the planned listing and which
/// stays before expiry; `Period::validate` re-checks both before the write.
pub(super) fn write_period_live(
    program_id: &Pubkey,
    period_info: &AccountInfo,
    anchor: &PeriodAnchor,
    listing_ts: u64,
) -> ProgramResult {
    if !period_info.is_writable || period_info.owner != program_id {
        return Err(OracleError::InvalidPeriodAccount.into());
    }
    if anchor.period.status != PeriodStatus::Forming || anchor.period.effective_listing_ts != 0 {
        return Err(OracleError::InvalidPeriodTransition.into());
    }
    let live = Period {
        status: PeriodStatus::Live,
        effective_listing_ts: listing_ts,
        ..anchor.period.clone()
    };
    live.validate()
        .map_err(|_| OracleError::InvalidPeriodTransition)?;
    period_info
        .try_borrow_mut_data()?
        .copy_from_slice(&live.encode());
    Ok(())
}

/// Live -> Settled, written once by the month's settlement. The value is nonzero and at the
/// period's decimals, `settled_ts` is at or after expiry and the digest commits to the settlement
/// computation; `Period::validate` re-checks all three before the write.
pub(super) fn write_period_settled(
    program_id: &Pubkey,
    period_info: &AccountInfo,
    anchor: &PeriodAnchor,
    settlement_value: u64,
    settled_ts: u64,
    settlement_digest: [u8; 32],
) -> ProgramResult {
    if !period_info.is_writable || period_info.owner != program_id {
        return Err(OracleError::InvalidPeriodAccount.into());
    }
    if anchor.period.status != PeriodStatus::Live {
        return Err(OracleError::InvalidPeriodTransition.into());
    }
    let settled = Period {
        status: PeriodStatus::Settled,
        settlement_value,
        settled_ts,
        settlement_digest,
        ..anchor.period.clone()
    };
    settled
        .validate()
        .map_err(|_| OracleError::InvalidPeriodTransition)?;
    period_info
        .try_borrow_mut_data()?
        .copy_from_slice(&settled.encode());
    Ok(())
}

/// Forming -> Failed, written by tag 79 once the period can no longer list: a period that never
/// listed stays unsettled forever.
pub(super) fn write_period_failed(
    program_id: &Pubkey,
    period_info: &AccountInfo,
    anchor: &PeriodAnchor,
) -> ProgramResult {
    if !period_info.is_writable || period_info.owner != program_id {
        return Err(OracleError::InvalidPeriodAccount.into());
    }
    if anchor.period.status != PeriodStatus::Forming || anchor.period.effective_listing_ts != 0 {
        return Err(OracleError::InvalidPeriodTransition.into());
    }
    let failed = Period {
        status: PeriodStatus::Failed,
        ..anchor.period.clone()
    };
    failed
        .validate()
        .map_err(|_| OracleError::InvalidPeriodTransition)?;
    period_info
        .try_borrow_mut_data()?
        .copy_from_slice(&failed.encode());
    Ok(())
}

/// Whether a Forming period can no longer list, given its month (or the month's absence):
/// (a) no month exists and the remaining time before expiry can no longer hold a month's
/// post-submission review calendar, so tag 181 can no longer create it; (b) the month is still in
/// source submission at or after its submission deadline, the predicate tags 191 and 195 use; or
/// (c) the month never reached Game by expiry.
pub(super) fn period_failure_predicate_holds(
    month: Option<&OracleMonthState>,
    expiry_ts: u64,
    now: u64,
) -> Result<bool, ProgramError> {
    let Some(month) = month else {
        return Ok(ensure_oracle_month_initialization_review_window(now, expiry_ts).is_err());
    };
    let pre_game = matches!(
        month.phase,
        OraclePhase::SourceSubmission | OraclePhase::Scramble | OraclePhase::Opening
    );
    if !pre_game {
        return Ok(false);
    }
    if month.phase == OraclePhase::SourceSubmission
        && now >= source_submission_deadline(month, expiry_ts)?
    {
        return Ok(true);
    }
    Ok(now >= expiry_ts)
}

/// Tag 79. Accounts: 0 [signer] cranker; 1 [writable] Forming period; 2 the period's month PDA,
/// either the month or the canonical absent account.
#[inline(never)]
pub(super) fn process_fail_period(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let [cranker, period_info, month_info] = accounts else {
        return Err(OracleError::InvalidAccountList.into());
    };
    if !cranker.is_signer || !period_info.is_writable || month_info.is_writable {
        return Err(OracleError::InvalidAccountList.into());
    }
    let anchor = load_valid_period_anchor(program_id, period_info)?;
    if anchor.period.status != PeriodStatus::Forming {
        return Err(OracleError::InvalidPeriodTransition.into());
    }
    let now = current_unix_timestamp()?;
    let failing = if month_info.owner == program_id {
        let month = load_valid_oracle_month(program_id, period_info, month_info, &anchor)?;
        period_failure_predicate_holds(Some(&month), anchor.expiry_ts, now)?
    } else {
        let (expected_month, _) =
            derive_oracle_month_pda(program_id, period_info.key, anchor.expiry_ts);
        validate_canonical_system_zero_pda_proof(&expected_month, month_info)
            .map_err(|_| ProgramError::from(OracleError::InvalidOracleMonthAccount))?;
        period_failure_predicate_holds(None, anchor.expiry_ts, now)?
    };
    if !failing {
        return Err(OracleError::PeriodNotFailable.into());
    }
    write_period_failed(program_id, period_info, &anchor)
}
