//! Oracle rent may retire only after all writer and holder cash obligations are paid.
use super::*;

pub(in crate::processor) fn require_paid_terminal_market(
    program: &Pubkey,
    config: &Pubkey,
    market_key: &Pubkey,
    market: &Market,
    a: &[AccountInfo],
) -> ProgramResult {
    if a.len() != 4 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let WriterPolicyContext {
        sleeve,
        group,
        book,
        snapshot,
    } = load_writer_policy_context(program, &a[0], &a[1], &a[2], &a[3], None)?;
    let record = book.records[..usize::from(book.series_count)]
        .iter()
        .find(|record| record.market == *market_key)
        .ok_or(VaultError::InvalidWriterLifecycle)?;
    if sleeve.vault_config != *config
        || sleeve.expiry_ts != market.instrument.expiry_ts
        || sleeve.settlement_mint != market.collateral_mint
        || !matches!(
            sleeve.status,
            WriterSleeveStatus::SettlementFinalized | WriterSleeveStatus::Closed
        )
        || !matches!(
            group.status,
            WriterSettlementGroupStatus::Settled | WriterSettlementGroupStatus::Closed
        )
        || sleeve.policy_snapshot != *a[3].key
        || sleeve.policy_hash != snapshot.policy_hash
        || !book.frozen
        || sleeve.accounted_asset_atoms != 0
        || sleeve.exact_reserve_atoms != 0
        || sleeve.long_liability_remaining_atoms != 0
        || sleeve.writer_residual_remaining_atoms != 0
        || sleeve.unclaimed_principal_atoms != 0
        || book.individual.cash_obligations != 0
        || book.individual.open_positions != 0
        || book.individual.pending_portfolio_funding != 0
        || book.individual.remaining_portfolio_credit != 0
        || record.settlement_status == WriterSeriesSettlementStatus::Open
        || record.settlement_liability_remaining_atoms != 0
        || (record.external_open_interest_atoms != 0
            && record.settlement_liability_initial_atoms != 0)
        || record.custody_status == WriterSeriesCustodyStatus::Open
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    // Zero-payoff tokens may still be held externally. Their original market,
    // settlement, sleeve and book remain available to all late holder operations.
    Ok(())
}
