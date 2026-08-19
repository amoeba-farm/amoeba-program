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
pub(crate) fn market_encoded_len(data: &[u8]) -> std::io::Result<usize> {
    const CONTRACT_MINT_TAG_OFFSET: usize = 98;
    const MINIMUM_LEN: usize = 253;
    let contract_mint_extension = optional_pubkey_extension(
        *data
            .get(CONTRACT_MINT_TAG_OFFSET)
            .ok_or_else(invalid_fixed_borsh)?,
    )?;
    let encoded_len = MINIMUM_LEN + contract_mint_extension;
    if data.len() < encoded_len {
        return Err(invalid_fixed_borsh());
    }
    Ok(encoded_len)
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

#[inline(always)]
pub(crate) fn writer_sleeve_encoded_len(data: &[u8]) -> std::io::Result<usize> {
    const ACTIVE_AUCTION_TAG_OFFSET: usize = 650;
    const SECOND_OPTION_BASE_OFFSET: usize = ACTIVE_AUCTION_TAG_OFFSET + 1;
    const MINIMUM_LEN: usize = 700;
    let auction_extension = optional_pubkey_extension(
        *data
            .get(ACTIVE_AUCTION_TAG_OFFSET)
            .ok_or_else(invalid_fixed_borsh)?,
    )?;
    let close_extension = optional_pubkey_extension(
        *data
            .get(SECOND_OPTION_BASE_OFFSET + auction_extension)
            .ok_or_else(invalid_fixed_borsh)?,
    )?;
    let encoded_len = MINIMUM_LEN + auction_extension + close_extension;
    if data.len() < encoded_len {
        return Err(invalid_fixed_borsh());
    }
    Ok(encoded_len)
}

variable_state_codec!(Market, 285, market_encoded_len, {
    is_initialized: bool,
    bump: u8,
    created_by: Pubkey,
    market_id: [u8; 32],
    collateral_mint: Pubkey,
    long_contract_mint: Option<Pubkey>,
    instrument: InstrumentDefinition,
    params: MarketParameters,
    total_position_collateral_locked: u64,
    paused: bool,
    mint_accounting: MarketMintAccounting,
});

variable_state_codec!(OracleMonthState, 299, oracle_month_encoded_len, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    market: Pubkey,
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

variable_state_codec!(WriterSleeveV1, 764, writer_sleeve_encoded_len, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    vault_config: Pubkey,
    underlying_id: [u8; 32],
    expiry_ts: u64,
    settlement_mint: Pubkey,
    settlement_group: Pubkey,
    series_book: Pubkey,
    usdc_vault: Pubkey,
    flat_mint: Pubkey,
    flat_spl_interface: Pubkey,
    flat_staging: Pubkey,
    flat_burn_custody: Pubkey,
    policy_registry: Pubkey,
    policy_snapshot: Pubkey,
    policy_version: u64,
    policy_hash: [u8; 32],
    scenario_set_hash: [u8; 32],
    risk_limit_hash: [u8; 32],
    writer_principal_atoms: u64,
    locked_primary_premium_atoms: u64,
    accounted_asset_atoms: u64,
    exact_reserve_atoms: u64,
    upper_tail_reserve_atoms: u64,
    lower_tail_reserve_atoms: u64,
    flat_par_supply_atoms: u64,
    security_exposure_atoms: u64,
    long_liability_initial_atoms: u64,
    long_liability_remaining_atoms: u64,
    flat_residual_initial_atoms: u64,
    flat_residual_remaining_atoms: u64,
    flat_supply_snapshot_atoms: u64,
    flat_claim_supply_remaining_atoms: u64,
    stranded_surplus_atoms: u64,
    operational_buffer_atoms: u64,
    auction_nonce: u64,
    close_nonce: u64,
    series_count: u8,
    status: WriterSleeveStatus,
    security_mode: WriterSecurityMode,
    v2_feature_flags: u8,
    active_auction: Option<Pubkey>,
    active_close_request: Option<Pubkey>,
    settlement_finalized_slot: u64,
    last_updated_slot: u64,
    reserved: [u8; 32],
});

#[inline(never)]
pub(crate) fn decode_oracle_month(
    data: &[u8],
    error: VaultError,
) -> Result<OracleMonthState, VaultError> {
    if data.len() != OracleMonthState::LEN {
        return Err(error);
    }
    // SAFETY: The exact canonical allocation was checked above.
    unsafe { <OracleMonthState as FixedStateDecode>::decode_fixed(data) }.map_err(|_| error)
}
