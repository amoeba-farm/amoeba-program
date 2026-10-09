//! Exact compact supply admission shared by source quoting and native execution.
use crate::state::WriterSeriesBookV1;

pub fn compact_supply_reconciled(
    book: &WriterSeriesBookV1,
    index: usize,
    pool_inventory: u64,
    mint_supply: u64,
    outstanding: u64,
) -> bool {
    let Some(record) = book.records.get(index) else {
        return false;
    };
    let Some(retired) = book.individual.compressed_retired_atoms.get(index) else {
        return false;
    };
    let Some(forfeited) = book.individual.forfeited_atoms.get(index) else {
        return false;
    };
    record.issuer_controlled_atoms == pool_inventory
        && record.total_physical_supply_atoms == mint_supply
        && book
            .external_total(index)
            .and_then(|v| v.checked_add(pool_inventory))
            .and_then(|v| v.checked_add(*retired))
            .and_then(|v| v.checked_add(*forfeited))
            == Some(mint_supply)
        && book
            .external_total(index)
            .and_then(|v| v.checked_add(pool_inventory))
            == Some(outstanding)
}
