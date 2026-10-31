//! Tag 30 selector 25 ("FMC1"): end a month that never listed at once. The user, 2026-10-30, for
//! mainnet: "dont hide stuff, just annihilate. no dec 1st expiry markets! only nov 1st legacy and
//! jan 1st for Q4 expiry".
//!
//! - **Kind 0** fails a Forming period that never listed (`effective_listing_ts == 0`) whatever
//!   its month's calendar says; a Live or listed period is refused (`write_period_failed`). Its
//!   month, in source submission or in Scramble (tag 184 is permissionless on the program
//!   deployed before the heads upgrade, and a Scramble takes source challenges, 161), goes back
//!   to source submission with its coverage unfinalized, as 186 reopens a stalled one (`parent`
//!   is then the coverage manifest). Its failure lanes then run at once (tags 191, 194, 195:
//!   `period::failure_lane_now`): every bond and support goes back to its own owner's
//!   collateral exactly as after the submission deadline, and every open source challenge's bond
//!   to its own challenger (194 in a Failed period; one the oracle authority sent to the council
//!   once its case ends by 30/18 operation 3, `council::cancel_failed_source`). The admin moves
//!   nobody's USDC. On a period
//!   already Failed (tag 79 at expiry, its month left in Scramble) kind 0 only reopens, which
//!   the production build has no other way to do (186 is v1 formation).
//! - **Kinds 1-12** close one account of the failed month into the admin's recipient, once every
//!   pre-listing escrow of the month is settled and its reward reservation released (the
//!   schedule Aborted, or never funded, with nothing outstanding; once the schedule is closed,
//!   that held when it closed). Each kind loads its account bound to the month and refuses one
//!   that still holds funds: 1 support position (released), 2 source reward (bond settled),
//!   3 source observations, 4 evidence link, 5 source (no bond locked), 6 coverage record,
//!   7 SKU pool, 8 the schedule, 9 the coverage manifest (schedule closed), 10 the month
//!   (schedule and coverage manifest closed), 11 source challenge (its bond settled), 12 a
//!   source's challenge guard (no challenge or council case holds it). A source council case
//!   and its rounds (after 185 and 30/18) have no close here; their rent stays. Sealed evidence
//!   objects are their writers', and permanent: other months may cite them, so they stay. The
//!   period stays Failed.
//! - **No order strands rent** (independent review of e25cd610, finding 2): a child closes
//!   whether its parent is still open or already closed. The month (kind 10, once the schedule
//!   and the coverage manifest closed) and a source (kind 5) may go first: the month's other
//!   accounts then stand on its canonical address, and a source's support, reward and
//!   observations (kinds 1-3) on their own canonical addresses and bindings to that source; a
//!   challenge and a guard (kinds 11, 12) name their month and source themselves.
//!
//! Accounts: 0 [signer] admin; 1 config; 2 [writable] period; 3 [writable] month (the canonical
//! absent account for kind 0 when it was never made); 4 [writable] the month's reward schedule
//! PDA; 5 [writable] rent recipient; 6 [writable] the target (the system program for kind 0);
//! 7 the target's source for kinds 1-3, its sponsorship agreement for a sponsored source (kind 5),
//! the coverage manifest PDA for kind 10 and (writable) for kind 0 on a month in Scramble (in
//! source submission, it or the system program), else the system program; 8 the product's carry
//! registry PDA (absent or not, it is checked); 9 for a sponsored source (kind 5) its agreement's
//! `UserCollateral`, else the system program.
//!
//! Kind 0 takes only a month its failure lanes unwind completely (source submission or Scramble,
//! no weights, a reward reservation 195 releases, or none; a pending count is source challenges,
//! which 194 settles in a Failed period), and neither kind 0 nor kind 10 takes the product's
//! latest registered carry month (re-review N8-N10, 2026-10-30). A sponsored
//! source closes only once its sponsorship reconciled and its agreement's ledger is empty: what
//! an escrow settled into it after reconciling (a rejected challenge's bond) is swept on to the
//! funder by reconciling again first (early review B3).

use super::usdc::{
    load_oracle_usdc_reward_schedule, load_oracle_usdc_sku_pool, load_oracle_usdc_source_reward,
};
use super::*;
use super::{carry, sponsorship};
use crate::state::{
    derive_oracle_sku_coverage_manifest_pda, derive_oracle_usdc_reward_schedule_pda,
    OracleEscrowDisposition, OracleSkuCoverageRecord, OracleSourceChallengeGuard,
    OracleUsdcRewardSchedulePhase,
};

fn invalid() -> ProgramError {
    OracleError::InvalidAccountList.into()
}

/// Whether `info` holds nothing (never made, or closed).
fn closed(info: &AccountInfo) -> bool {
    crate::is_system_program(info.owner) && info.data_len() == 0
}

/// Whether `info` is `key` and absent (never made, or closed).
fn absent_at(info: &AccountInfo, key: &Pubkey) -> bool {
    info.key == key && closed(info)
}

#[inline(never)]
pub(super) fn process(program: &Pubkey, a: &[AccountInfo], kind: u8) -> ProgramResult {
    let [admin, config_info, period_info, month_info, schedule_info, recipient, target, parent, registry, agreement_cash] =
        a
    else {
        return Err(invalid());
    };
    if !admin.is_signer || !recipient.is_writable || !period_info.is_writable {
        return Err(invalid());
    }
    let config = load_canonical_vault_config(program, config_info)?;
    if config.admin != *admin.key {
        return Err(OracleError::Unauthorized.into());
    }
    let anchor = load_valid_period_anchor(program, period_info)?;
    let (month_key, _) = derive_oracle_month_pda(program, period_info.key, anchor.expiry_ts);
    // The next rung registers against the latest carry month alone: it is never failed or closed
    // here.
    if matches!(kind, 0 | 10)
        && carry::is_latest_carry_month(program, &anchor, &month_key, registry)?
    {
        return Err(OracleError::InvalidOraclePhase.into());
    }
    if kind != 5 {
        validate_system_program(agreement_cash)?;
    }
    if kind == 0 {
        validate_system_program(target)?;
        let failed = anchor.period.status == PeriodStatus::Failed;
        if month_info.owner == program {
            // Only what the failure lanes (191, 194, 195) unwind completely: source submission
            // or Scramble, no weights (a pending count is then source challenges only, which 194
            // settles in a Failed period), and a reward reservation 195 releases (or none).
            let mut month = load_valid_oracle_month(program, period_info, month_info, &anchor)?;
            let scramble = month.phase == OraclePhase::Scramble;
            if !(scramble || month.phase == OraclePhase::SourceSubmission)
                || month.weight_scheme_version != 0
                || (failed && !scramble)
            {
                return Err(OracleError::InvalidOraclePhase.into());
            }
            let (schedule_key, _) = derive_oracle_usdc_reward_schedule_pda(program, &month_key);
            if !absent_at(schedule_info, &schedule_key) {
                let s = load_oracle_usdc_reward_schedule(program, &month_key, schedule_info)?;
                let unwinds = match s.phase {
                    OracleUsdcRewardSchedulePhase::Funded => {
                        s.remaining_reward_budget == s.total_reward_budget
                    }
                    OracleUsdcRewardSchedulePhase::Building => s.total_reward_budget == 0,
                    OracleUsdcRewardSchedulePhase::Aborted => true,
                    OracleUsdcRewardSchedulePhase::EntitlementsFinalized => false,
                };
                if !unwinds {
                    return Err(OracleError::InvalidOracleUsdcRewardSchedule.into());
                }
            }
            // Back to source submission, coverage unfinalized: the failure lanes' phase.
            let manifest = derive_oracle_sku_coverage_manifest_pda(program, &month_key).0;
            if scramble {
                if parent.key != &manifest || !parent.is_writable || !month_info.is_writable {
                    return Err(invalid());
                }
                let mut coverage =
                    load_valid_oracle_sku_coverage_manifest(program, &month_key, parent)?;
                let slot = crate::compact_error::slot()?;
                month.phase = OraclePhase::SourceSubmission;
                month.last_updated_slot = slot;
                coverage.coverage_finalized = false;
                coverage.coverage_complete_ts = 0;
                coverage.last_updated_slot = slot;
                store_state(parent, &coverage)?;
                store_oracle_month_state(month_info, &month)?;
            } else if parent.key != &manifest {
                validate_system_program(parent)?;
            }
        } else {
            validate_system_program(parent)?;
            validate_canonical_system_zero_pda_proof(&month_key, month_info)
                .map_err(|_| ProgramError::from(OracleError::InvalidOracleMonthAccount))?;
            if failed {
                return Err(OracleError::InvalidPeriodTransition.into());
            }
        }
        if failed {
            return Ok(());
        }
        // Forming, never listed; a Live or listed period is refused there.
        return write_period_failed(program, period_info, &anchor);
    }
    if anchor.period.status != PeriodStatus::Failed
        || !target.is_writable
        || target.owner != program
        || target.key == recipient.key
    {
        return Err(OracleError::InvalidPeriodTransition.into());
    }
    // The month itself closes only as kind 10 (after the schedule and the coverage manifest);
    // once it has, its other accounts still close, against its canonical address.
    if kind == 10 || !closed(month_info) {
        load_valid_oracle_month(program, period_info, month_info, &anchor)?;
    } else {
        validate_canonical_system_zero_pda_proof(&month_key, month_info)
            .map_err(|_| ProgramError::from(OracleError::InvalidOracleMonthAccount))?;
    }
    // Every pre-listing escrow settled and the reward reservation released, or the schedule
    // closed once that held.
    let (schedule_key, _) = derive_oracle_usdc_reward_schedule_pda(program, &month_key);
    let schedule_open = !absent_at(schedule_info, &schedule_key);
    if schedule_open {
        let schedule = load_oracle_usdc_reward_schedule(program, &month_key, schedule_info)?;
        let released = schedule.phase == OracleUsdcRewardSchedulePhase::Aborted
            || (schedule.phase == OracleUsdcRewardSchedulePhase::Building
                && schedule.total_reward_budget == 0);
        if !released || schedule.outstanding_prelisting_escrow_count != 0 {
            return Err(OracleError::InvalidOracleUsdcRewardSchedule.into());
        }
    }
    let source_id = |p: &AccountInfo| -> Result<[u8; 32], ProgramError> {
        Ok(load_valid_oracle_source(program, &month_key, p)?.source_id)
    };
    // Kinds 1-3 name their source as `parent`: open, it is the month's source; closed already
    // (kind 5 first), it is the canonical source address the child's own id derives (kinds 1, 2),
    // and each child's loader binds it to that source and the month.
    let parent_closed = closed(parent);
    let closed_source = |id: &[u8; 32]| -> ProgramResult {
        if derive_oracle_source_pda(program, &month_key, id).0 != *parent.key {
            return Err(OracleError::InvalidOracleSourceAccount.into());
        }
        Ok(())
    };
    match kind {
        1 => {
            let id = if parent_closed {
                let support: OracleSupportPosition = load_exact_zero_padded_state(
                    target,
                    program,
                    OracleSupportPosition::LEN,
                    OracleError::OracleEscrowNotSettleable,
                )?;
                closed_source(&support.source_id)?;
                support.source_id
            } else {
                source_id(parent)?
            };
            let support = load_valid_oracle_support_position(
                program,
                &month_key,
                parent.key,
                &id,
                target,
                OracleError::OracleEscrowNotSettleable,
            )?;
            if !support.released {
                return Err(OracleError::OracleEscrowNotSettleable.into());
            }
        }
        2 => {
            if !parent_closed {
                source_id(parent)?;
            }
            let reward = load_oracle_usdc_source_reward(
                program,
                &schedule_key,
                &month_key,
                parent.key,
                target,
            )?;
            if parent_closed {
                closed_source(&reward.source_id)?;
            }
            if !reward.listing_escrow_counted {
                return Err(OracleError::OracleEscrowNotSettleable.into());
            }
        }
        3 => {
            if !parent_closed {
                source_id(parent)?;
            }
            load_valid_oracle_source_observations(program, &month_key, parent.key, target)?;
        }
        4 => oracle_evidence::validate_cleanup_link(program, &month_key, target)?,
        5 => {
            let source = load_valid_oracle_source(program, &month_key, target)?;
            if source.listing_bond_locked != 0 {
                return Err(OracleError::OracleEscrowNotSettleable.into());
            }
            // A sponsored source (its proposer the agreement PDA): only once reconciled, which
            // pays the refunded bond on to its funder and needs the source, with nothing left in
            // the agreement's ledger (a later escrow's proceeds swept by reconciling again).
            if source.proposer == crate::sponsorship::agreement_address(program, target.key).0 {
                if parent.key != &source.proposer
                    || !sponsorship::reconciled(program, parent, target.key)?
                {
                    return Err(OracleError::OracleEscrowNotSettleable.into());
                }
                let cash = load_canonical_user_collateral(program, agreement_cash, parent.key)?;
                if cash.available_balance != 0 || cash.position_locked_balance != 0 {
                    return Err(OracleError::OracleEscrowNotSettleable.into());
                }
            } else {
                validate_system_program(agreement_cash)?;
            }
        }
        6 => {
            let record: OracleSkuCoverageRecord = load_exact_zero_padded_state(
                target,
                program,
                OracleSkuCoverageRecord::LEN,
                OracleError::InvalidOracleSkuCoverageRecord,
            )?;
            load_valid_oracle_sku_coverage_record(program, &month_key, &record.sku_id, target)?;
        }
        7 => {
            load_oracle_usdc_sku_pool(program, &schedule_key, &month_key, target)?;
        }
        8 => {
            if !schedule_open || target.key != schedule_info.key {
                return Err(invalid());
            }
        }
        9 => {
            if schedule_open {
                return Err(invalid());
            }
            load_valid_oracle_sku_coverage_manifest(program, &month_key, target)?;
        }
        10 => {
            let manifest = derive_oracle_sku_coverage_manifest_pda(program, &month_key).0;
            if schedule_open || target.key != month_info.key || !absent_at(parent, &manifest) {
                return Err(invalid());
            }
        }
        11 => {
            let challenge = load_valid_oracle_source_challenge(program, &month_key, target)?;
            if challenge.escrow_disposition == OracleEscrowDisposition::Unsettled {
                return Err(OracleError::OracleEscrowNotSettleable.into());
            }
        }
        12 => {
            let guard: OracleSourceChallengeGuard = load_exact_zero_padded_state(
                target,
                program,
                OracleSourceChallengeGuard::LEN,
                OracleError::InvalidOracleChallengeAccount,
            )?;
            let guard = load_canonical_source_challenge_guard(
                program,
                &month_key,
                &guard.source,
                &guard.source_id,
                target,
            )?;
            if !crate::pubkey_is_default(&guard.active_challenge)
                || !crate::pubkey_is_default(&guard.active_dispute)
            {
                return Err(OracleError::OracleEscrowNotSettleable.into());
            }
        }
        _ => return Err(OracleError::InvalidInstructionData.into()),
    }
    close_program_account(program, target, recipient)
}
