//! Cash settlement of authenticated, program-owned classic order custody.
//! The caller owns the DOR obligation; no wallet leaf or delegate is forged.
use super::*;

pub(in crate::processor) fn settle(
    program: &Pubkey,
    a: &[AccountInfo],
    amount: u64,
    order_seeds: &[&[u8]],
) -> ProgramResult {
    let config = load_canonical_vault_config(program, &a[2])?;
    let WriterBookContext {
        group,
        mut sleeve,
        mut book,
    } = load_writer_book_context(program, &a[12], &a[13], &a[14])?;
    if sleeve.vault_config != *a[2].key
        || group.sleeve != *a[12].key
        || sleeve.series_book != *a[14].key
        || !book.frozen
        || sleeve.status != WriterSleeveStatus::SettlementFinalized
        || group.status != WriterSettlementGroupStatus::Settled
        || sleeve.usdc_vault != *a[15].key
        || sleeve.settlement_mint != *a[9].key
        || config.usdc_mint != *a[9].key
        || current_unix_timestamp()? < group.expiry_ts
        || *a[16].key
            != crate::compressed_option_settlement::retirement_owner(program, a[12].key, a[7].key)
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let index = book.records[..usize::from(book.series_count)]
        .iter()
        .position(|r| r.market == *a[7].key && r.contract_mint == *a[8].key)
        .ok_or(VaultError::InvalidWriterSeriesBook)?;
    let record = book.records[index];
    // Deployed classic series predate a forfeiture window. Their zero policy
    // retains the original unlimited claim right; new declared windows stay exact.
    match record.reserved {
        [0, 0, 0, 0] => (),
        [1, 1, 0, 0] => compressed_settlement::check_deadline(&book, index)?,
        _ => return Err(VaultError::InvalidWriterLifecycle.into()),
    }
    if record.settlement_status != WriterSeriesSettlementStatus::Frozen
        || amount > record.external_open_interest_atoms
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mut market = load_valid_market(program, &a[7])?;
    if market.market_id != record.series_id
        || market.long_contract_mint != Some(*a[8].key)
        || market.instrument.expiry_ts != group.expiry_ts
        || market.instrument.underlying_id != group.underlying_id
        || market_outstanding_contract_amount(&market)? != record.external_open_interest_atoms
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let mint = validate_canonical_market_mint(&a[7], &mut market, &a[8], 0)?;
    if mint.supply != record.total_physical_supply_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let payout = crate::writer_sleeve_math::cumulative_allocation_delta(
        record.settlement_external_oi_snapshot_atoms,
        record.external_open_interest_atoms,
        amount,
        record.settlement_liability_initial_atoms,
    )
    .map_err(writer_math_error)?;
    if payout > record.settlement_liability_remaining_atoms
        || payout > sleeve.long_liability_remaining_atoms
        || payout > sleeve.accounted_asset_atoms
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    validate_vault_token_account(&a[15], a[9].key, a[12].key)?;
    let hot_cash = validate_token_account(&a[15])?.amount;
    use crate::compressed_custody::{self as custody, CustodyKind};
    let cash_key =
        custody::derive_compressed_custody(program, CustodyKind::WriterCash, a[15].key).0;
    if *a[26].key != cash_key {
        return Err(VaultError::InvalidAccountList.into());
    }
    let absent = a[26].owner == &system_program::id() && a[26].data_is_empty();
    let cash = custody::load_observation(
        program,
        if absent { None } else { Some(&a[26]) },
        CustodyKind::WriterCash,
        a[15].key,
        &Pubkey::default(),
        a[9].key,
    )?;
    if !custody::backs(cash.as_ref(), 0, hot_cash, 0, sleeve.accounted_asset_atoms)
        || cash.as_ref().is_some_and(|c| c.option_atoms != 0)
        || hot_cash < payout
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    // Retirement produces the same canonical, non-withdrawable option sink as
    // regular compressed long settlement. Supply remains physically unchanged.
    crate::processor::ameba_dlmm::orders::expired::compress(
        a,
        5,
        8,
        10,
        4,
        16,
        amount,
        order_seeds,
    )?;
    let bump = [sleeve.bump];
    let seeds = writer_sleeve_signer_seeds(&sleeve.settlement_group, &bump);
    crate::processor::ameba_dlmm::orders::expired::compress(a, 15, 9, 11, 12, 1, payout, &seeds)?;
    if validate_mint_account(&a[8], a[19].key)?.supply != mint.supply {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    compressed_settlement::apply_retirement(
        &mut book,
        &mut sleeve,
        &mut market,
        index,
        amount,
        payout,
    )?;
    let slot = Clock::get()?.slot;
    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = slot;
    sleeve.last_updated_slot = slot;
    store_state(&a[7], &market)?;
    store_state(&a[14], &book)?;
    store_state(&a[12], &sleeve)
}
