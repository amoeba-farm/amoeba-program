use super::variable_state_codec;
use super::*;

#[inline(always)]
pub(crate) fn optional_pubkey_extension(tag: u8) -> std::io::Result<usize> {
    match tag {
        0 => Ok(0),
        1 => Ok(32),
        _ => Err(invalid_fixed_borsh()),
    }
}

#[inline(always)]
pub(crate) fn oracle_month_encoded_len(data: &[u8]) -> std::io::Result<usize> {
    const SETTLEMENT_RECORD_TAG_OFFSET: usize = 142;
    const MINIMUM_LEN: usize = 267;
    let extension = optional_pubkey_extension(
        *data
            .get(SETTLEMENT_RECORD_TAG_OFFSET)
            .ok_or_else(invalid_fixed_borsh)?,
    )?;
    let encoded_len = MINIMUM_LEN + extension;
    if data.len() < encoded_len {
        return Err(invalid_fixed_borsh());
    }
    Ok(encoded_len)
}

variable_state_codec!(OracleMonthState, 299, oracle_month_encoded_len, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    period: Pubkey,
    authority: Pubkey,
    scramble_start_ts: u64,
    listing_ts: u64,
    phase: OraclePhase,
    source_count: u16,
    frozen_source_count: u16,
    opened_source_count: u16,
    recipe_hash: [u8; 32],
    index_delta_bps: i64,
    c_raw_bps: u16,
    g_camo_bps: u16,
    g_thin_bps: u16,
    c_settle_bps: u16,
    settlement_status: OracleSettlementStatus,
    settlement_record: Option<Pubkey>,
    economics: OracleEconomicParams,
    last_updated_slot: u64,
    settlement_base_oracle_atomic: u64,
    pending_resolution_count: u16,
    finalized_at_ts: u64,
    opening_resolved_source_count: u16,
    weight_scheme_version: u8,
    effective_weight_total_bps: u16,
    weight_manifest_hash: [u8; 32],
    candidate_count_tracking_version: u8,
    active_weight_initialization_version: u8,
    active_weight_scheme_version: u8,
    active_weight_group_count: u16,
    active_weight_manifest_hash: [u8; 32],
    schedule_version: u8,
    work_reward_currency_version: u8,
    accepted_cash_update_count: u32,
});

#[inline(never)]
pub(crate) fn decode_oracle_month(
    data: &[u8],
    error: OracleError,
) -> Result<OracleMonthState, OracleError> {
    if data.len() != OracleMonthState::LEN {
        return Err(error);
    }
    // SAFETY: The exact canonical allocation was checked above.
    unsafe { <OracleMonthState as FixedStateDecode>::decode_fixed(data) }.map_err(|_| error)
}
