//! The month's terminal steps: tag 75 finalizes its index from the bucket medians after expiry
//! and grace, and tag 59 closes it once tag 116 has settled it.

use super::*;

/// Tag 59. Accounts: 0 [signer] admin or oracle authority; 1 config; 2 period; 3 [writable]
/// Settled month; 4 its settlement record.
pub(super) fn process_close_oracle_month(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 5 {
        return Err(OracleError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let config_info = &accounts[1];
    let period_info = &accounts[2];
    let month_info = &accounts[3];
    let settlement_record_info = &accounts[4];
    validate_oracle_authority(program_id, authority_info, config_info)?;
    let (anchor, mut month) =
        load_valid_period_and_oracle_month(program_id, period_info, month_info)?;
    ensure_oracle_month_ready_to_close(&month, settlement_record_info.key)?;
    let settlement = load_valid_settlement_record_v2(
        program_id,
        period_info.key,
        &anchor,
        settlement_record_info,
    )?;
    // The record commits to the signed leaf (recipe and source provenance) and is bound to the
    // same month by its canonical PDA and the checks made when it was created.
    if settlement.oracle_month != *month_info.key {
        return Err(OracleError::InvalidSettlementRecord.into());
    }
    month.phase = OraclePhase::Closed;
    month.last_updated_slot = crate::compact_error::slot()?;
    store_oracle_month_state(month_info, &month)
}

/// Tag 75. Accounts: 0 [signer] cranker; 1 period; 2 [writable] month in Game; then every
/// bucket median of the month in group order.
pub(super) fn process_finalize_oracle_month(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() < 4 {
        return Err(OracleError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let period_info = &accounts[1];
    let month_info = &accounts[2];
    if !cranker_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let (anchor, mut month) =
        load_valid_period_and_oracle_month(program_id, period_info, month_info)?;
    ensure_oracle_opening_resolution_complete(&month)?;
    if month.phase != OraclePhase::Game {
        return Err(OracleError::InvalidOraclePhase.into());
    }
    if month.settlement_status == OracleSettlementStatus::Final || month.finalized_at_ts != 0 {
        return Err(OracleError::InvalidOracleState.into());
    }
    let (slot, now) = current_slot_and_unix_timestamp()?;
    ensure_settlement_finalization_ready_at(anchor.expiry_ts, now)?;
    if month.pending_resolution_count != 0 {
        return Err(OracleError::InvalidOracleState.into());
    }
    if accounts.len() != 3 + usize::from(month.active_weight_group_count) {
        return Err(OracleError::InvalidAccountList.into());
    }
    let mut last_bucket_id = [0u8; 32];
    let mut weight_total = 0u16;
    let mut index_total = 0i128;
    for (group_index, bucket_info) in accounts[3..].iter().enumerate() {
        let bucket = load_valid_oracle_bucket_median(program_id, month_info.key, bucket_info)?;
        if usize::from(bucket.group_index) != group_index
            || bucket.bucket_id <= last_bucket_id
            || !bucket.status.permits_settlement()
            || bucket.eligible_source_count < bucket_settlement_floor(bucket.frozen_source_count)
                && !bucket.status.council_decided()
            || bucket.last_recomputed_ts < anchor.expiry_ts
        {
            return Err(OracleError::InvalidOracleMedian.into());
        }
        weight_total = weight_total
            .checked_add(bucket.bucket_weight_bps)
            .ok_or(OracleError::ArithmeticOverflow)?;
        index_total = index_total
            .checked_add(i128::from(bucket_index_contribution_bps(
                bucket.bucket_weight_bps,
                bucket.bucket_delta_bps,
            )?))
            .ok_or(OracleError::ArithmeticOverflow)?;
        last_bucket_id = bucket.bucket_id;
    }
    let canonical_index =
        i64::try_from(index_total).map_err(|_| OracleError::ArithmeticOverflow)?;
    if weight_total != 10_000 || canonical_index != month.index_delta_bps {
        return Err(OracleError::InvalidOracleMedian.into());
    }
    month.c_raw_bps = 10_000;
    month.g_camo_bps = 0;
    month.g_thin_bps = 0;
    month.c_settle_bps = 10_000;
    month.settlement_status = OracleSettlementStatus::Final;
    month.finalized_at_ts = now;
    month.last_updated_slot = slot;
    store_oracle_month_state(month_info, &month)
}
