//! An ephemeral empty ordinary-source view for an authenticated primary Market.
//! The caller proves that the resident payload is absent and a writer context
//! exists. This view must never be written as an initialized Pool or payload.
use super::*;
use crate::ameba_dlmm_state::{derive_ameba_dlmm_pool_pda, AmoebaDlmmPoolV1};
use crate::constants::{MAX_AMOEBA_DLMM_BINS_PER_SWAP, MAX_AMOEBA_DLMM_BIN_COUNT};
use light_sdk_types::interface::account::compression_info::CompressionInfo;

pub(super) fn primary_only_resident(
    program: &Pubkey,
    market_key: &Pubkey,
    market: &Market,
    quote_mint: &Pubkey,
    oracle_month: &Pubkey,
) -> Result<router::ResidentRouterState, ProgramError> {
    // Reuse the same instrument and grid guards as collective market binding.
    validate_instrument_definition(&market.instrument)?;
    validate_market_parameters(&market.params, market.instrument.max_payout_per_contract)?;
    let option_mint = market
        .long_contract_mint
        .ok_or(VaultError::InvalidContractMint)?;
    let maximum_bin_id = market
        .instrument
        .max_payout_per_contract
        .checked_div(market.params.tick_size)
        .and_then(|n| u16::try_from(n).ok())
        .ok_or(VaultError::InvalidAmoebaDlmmGrid)?;
    let maximum_bins_per_swap = market
        .params
        .max_fills_per_instruction
        .min(MAX_AMOEBA_DLMM_BINS_PER_SWAP);
    if market.collateral_mint != *quote_mint
        || !market.mint_accounting.has_canonical_layout()
        || maximum_bin_id == 0
        || maximum_bin_id > MAX_AMOEBA_DLMM_BIN_COUNT
        || maximum_bins_per_swap == 0
    {
        return Err(VaultError::InvalidWriterSettlementGroup.into());
    }
    let (_, bump) = derive_ameba_dlmm_pool_pda(program, market_key);
    Ok(router::ResidentRouterState {
        pool: AmoebaDlmmPoolV1 {
            is_initialized: true,
            bump,
            market: *market_key,
            oracle_month: *oracle_month,
            option_mint,
            quote_mint: *quote_mint,
            expiry_ts: market.instrument.expiry_ts,
            tick_size_quote_atomic: market.params.tick_size,
            maximum_price_quote_atomic: market.instrument.max_payout_per_contract,
            maximum_bin_id,
            maximum_bins_per_swap,
            status: AmoebaDlmmPoolStatus::Active,
            compression_info: CompressionInfo::new_decompressed(0),
            ..AmoebaDlmmPoolV1::default()
        },
        book: None,
        records: vec![],
        bin_pages: vec![],
        writer_position: None,
        pool_option: 0,
        pool_quote: 0,
        book_option: 0,
        book_quote: 0,
        pool_hot_option: 0,
        pool_hot_quote: 0,
        book_hot_option: 0,
        book_hot_quote: 0,
    })
}
