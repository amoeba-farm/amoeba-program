use super::*;

pub(in crate::processor) fn load_collective_settlement_group_for_dlmm(
    program_id: &Pubkey,
    group_info: &AccountInfo,
) -> Result<Box<WriterSettlementGroupV1>, ProgramError> {
    load_writer_settlement_group(program_id, group_info)
}

#[inline(never)]
fn load_collective_group_binding(
    program_id: &Pubkey,
    sleeve_info: &AccountInfo,
    group_info: &AccountInfo,
    book_info: &AccountInfo,
    market_key: &Pubkey,
) -> Result<CollectiveGroupBinding, ProgramError> {
    let WriterBookContext {
        group,
        sleeve,
        book,
    } = load_writer_book_context(program_id, sleeve_info, group_info, book_info)?;
    let record = book.records[..usize::from(book.series_count)]
        .iter()
        .find(|record| record.market == *market_key)
        .ok_or(VaultError::InvalidWriterSeriesBook)?;
    if group.sleeve != *sleeve_info.key || sleeve.series_book != *book_info.key {
        return Err(VaultError::InvalidWriterSettlementGroup.into());
    }
    Ok(CollectiveGroupBinding {
        sleeve_status: sleeve.status,
        group_status: group.status,
        active_weight_manifest_hash: group.active_weight_manifest_hash,
        underlying_id: group.underlying_id,
        expiry_ts: group.expiry_ts,
        settlement_mint: group.settlement_mint,
        anchor_market: group.anchor_market,
        anchor_oracle_month: group.anchor_oracle_month,
        series_id: record.series_id,
        contract_mint: record.contract_mint,
    })
}

#[inline(never)]
fn load_collective_market_binding(
    program_id: &Pubkey,
    market_info: &AccountInfo,
    binding: &CollectiveGroupBinding,
) -> Result<CollectiveMarketBinding, ProgramError> {
    let market = load_valid_market(program_id, market_info)?;
    validate_instrument_definition(&market.instrument)
        .map_err(|_| VaultError::InvalidAmoebaDlmmGrid)?;
    validate_market_parameters(&market.params, market.instrument.max_payout_per_contract)
        .map_err(|_| VaultError::InvalidAmoebaDlmmGrid)?;
    let option_mint = market
        .long_contract_mint
        .ok_or(VaultError::InvalidContractMint)?;
    let maximum_bin_id = market
        .instrument
        .max_payout_per_contract
        .checked_div(market.params.tick_size)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or(VaultError::InvalidAmoebaDlmmGrid)?;
    let maximum_bins_per_swap = market
        .params
        .max_fills_per_instruction
        .min(MAX_AMOEBA_DLMM_BINS_PER_SWAP);
    if market.market_id != binding.series_id
        || option_mint != binding.contract_mint
        || market.instrument.underlying_id != binding.underlying_id
        || market.instrument.expiry_ts != binding.expiry_ts
        || market.collateral_mint != binding.settlement_mint
        || market.params.tick_size == 0
        || market.instrument.max_payout_per_contract == 0
        || !market
            .instrument
            .max_payout_per_contract
            .is_multiple_of(market.params.tick_size)
        || maximum_bin_id == 0
        || maximum_bin_id > MAX_AMOEBA_DLMM_BIN_COUNT
        || maximum_bins_per_swap == 0
        || market.params.taker_fee_bps > MAX_AMOEBA_DLMM_SWAP_FEE_BPS
        || !market.mint_accounting.has_canonical_layout()
    {
        return Err(VaultError::InvalidWriterSettlementGroup.into());
    }
    Ok(CollectiveMarketBinding {
        option_mint,
        quote_mint: market.collateral_mint,
        expiry_ts: market.instrument.expiry_ts,
        tick_size_quote_atomic: market.params.tick_size,
        maximum_price_quote_atomic: market.instrument.max_payout_per_contract,
        maximum_bin_id,
        swap_fee_bps: market.params.taker_fee_bps,
        maximum_bins_per_swap,
    })
}

#[inline(never)]
fn load_collective_anchor_month_status(
    program_id: &Pubkey,
    anchor_month_info: &AccountInfo,
    binding: &CollectiveGroupBinding,
) -> Result<bool, ProgramError> {
    let anchor_month = load_oracle_month_state(anchor_month_info, program_id)?;
    let (expected_month, expected_bump) =
        derive_oracle_month_pda(program_id, &binding.anchor_market, binding.expiry_ts);
    if binding.anchor_oracle_month != *anchor_month_info.key
        || *anchor_month_info.key != expected_month
        || !anchor_month.is_initialized
        || anchor_month.bump != expected_bump
        || anchor_month.market != binding.anchor_market
    {
        return Err(VaultError::InvalidWriterSettlementGroup.into());
    }
    Ok(anchor_month.settlement_record.is_some()
        || anchor_month.settlement_status == OracleSettlementStatus::Final)
}

/// Load the immutable collective binding needed by the secondary DLMM lanes. The pool remains
/// Market-addressed, while this companion context proves that its Market belongs to exactly one
/// sleeve and that its oracle clock is the sleeve's canonical anchor month.
#[inline(never)]
pub(in crate::processor) fn load_collective_dlmm_context(
    program_id: &Pubkey,
    sleeve_info: &AccountInfo,
    group_info: &AccountInfo,
    book_info: &AccountInfo,
    market_info: &AccountInfo,
    anchor_month_info: &AccountInfo,
) -> Result<CollectiveDlmmContext, ProgramError> {
    let binding = load_collective_group_binding(
        program_id,
        sleeve_info,
        group_info,
        book_info,
        market_info.key,
    )?;
    let market = load_collective_market_binding(program_id, market_info, &binding)?;
    let anchor_month_settled =
        load_collective_anchor_month_status(program_id, anchor_month_info, &binding)?;
    Ok(CollectiveDlmmContext {
        sleeve_status: binding.sleeve_status,
        group_status: binding.group_status,
        active_weight_manifest_hash: binding.active_weight_manifest_hash,
        anchor_month_settled,
        option_mint: market.option_mint,
        quote_mint: market.quote_mint,
        expiry_ts: market.expiry_ts,
        tick_size_quote_atomic: market.tick_size_quote_atomic,
        maximum_price_quote_atomic: market.maximum_price_quote_atomic,
        maximum_bin_id: market.maximum_bin_id,
        swap_fee_bps: market.swap_fee_bps,
        maximum_bins_per_swap: market.maximum_bins_per_swap,
    })
}
