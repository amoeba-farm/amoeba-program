//! Bounded, buyback-only issuance. Every ask is accounted separately; equal
//! series share one mint and compressed output after canonical role validation.
use super::*;
use crate::individual_writer::{IndividualBuybackV1, MAX_BUYBACK_LEGS};

const COMMON: usize = 25;
const LEG: usize = 7;

pub(super) fn validate_repeated_series_roles(
    a: &[AccountInfo],
    wire: &IndividualBuybackV1,
) -> ProgramResult {
    for i in 0..usize::from(wire.leg_count) {
        for j in 0..i {
            if wire.legs[i].series_index == wire.legs[j].series_index {
                let left = COMMON + i * LEG;
                let right = COMMON + j * LEG;
                // These witnesses remain authenticated even when issuance is grouped.
                if (2..7).any(|role| a[left + role].key != a[right + role].key) {
                    return Err(VaultError::InvalidAccountList.into());
                }
            }
        }
    }
    Ok(())
}

struct Prepared {
    index: usize,
    leg: usize,
    quantity: u64,
    market: Market,
    supply_before: u64,
    staged: u64,
}

#[inline(never)]
pub(super) fn issue(
    program: &Pubkey,
    a: &[AccountInfo],
    wire: &IndividualBuybackV1,
    sleeve: &WriterSleeveV1,
    group: &WriterSettlementGroupV1,
    book: &mut WriterSeriesBookV1,
    quantities: &[u64; 20],
) -> ProgramResult {
    if a[5].executable {
        return Err(VaultError::InvalidAccountList.into());
    }
    let policy = dlmm::load_optional_policy(program, &a[19], &a[2], sleeve)?;
    let count = usize::from(wire.leg_count);
    let mut prepared = Vec::with_capacity(count);
    for leg in 0..count {
        let index = usize::from(wire.legs[leg].series_index);
        if wire.legs[..leg]
            .iter()
            .any(|earlier| earlier.series_index == wire.legs[leg].series_index)
        {
            continue;
        }
        let view = individual_buyback::fill_accounts(a, leg);
        let (market, mint, staged) =
            individual::validate_issue(program, &view, group, book, index, policy.as_deref())?;
        prepared.push(Prepared {
            index,
            leg,
            quantity: quantities[index],
            market,
            supply_before: mint.supply,
            staged,
        });
    }
    // Complete all distinct-series checks before minting any part of the basket.
    for item in &prepared {
        let view = individual_buyback::fill_accounts(a, item.leg);
        book.individual.series[item.index].issued = book.individual.series[item.index]
            .issued
            .checked_add(item.quantity)
            .ok_or(VaultError::ArithmeticOverflow)?;
        book.individual.series[item.index].outstanding = book.individual.series[item.index]
            .outstanding
            .checked_add(item.quantity)
            .ok_or(VaultError::ArithmeticOverflow)?;
        custody::load_or_create_market_staging(
            program,
            &a[0],
            &view[16],
            &item.market,
            &view[20],
            &view[18],
            &a[12],
            &a[13],
        )?;
        let bump = [item.market.bump];
        let seeds = custody::market_signer_seeds(&item.market, &bump);
        if item.staged > 0 {
            load_or_create_writer_retirement_custody(
                program, &a[0], &a[2], &view[16], &view[21], &view[18], &a[12], &a[13],
            )?;
            invoke_token_transfer_checked(
                &a[12],
                &view[20],
                &view[18],
                &view[21],
                &view[16],
                item.staged,
                MarketMintAccounting::CANONICAL_DECIMALS,
                &[&seeds],
            )?;
        }
        invoke_token_mint_to_checked(
            &a[12],
            &view[18],
            &view[20],
            &view[16],
            item.quantity,
            MarketMintAccounting::CANONICAL_DECIMALS,
            &[&seeds],
        )?;
    }
    compress(a, &prepared)?;
    for item in &mut prepared {
        let view = individual_buyback::fill_accounts(a, item.leg);
        let supply = item
            .supply_before
            .checked_add(item.quantity)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if validate_token_account(&view[20])?.amount != 0
            || validate_mint_account(&view[18], a[12].key)?.supply != supply
        {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        let bump = [item.market.bump];
        let seeds = custody::market_signer_seeds(&item.market, &bump);
        invoke_token_close_account(&a[12], &view[20], &a[0], &view[16], &[&seeds])?;
        book.records[item.index].total_physical_supply_atoms = supply;
        book.individual.active_locked[item.index] = book.individual.active_locked[item.index]
            .checked_add(item.quantity)
            .ok_or(VaultError::ArithmeticOverflow)?;
        item.market.mint_accounting.total_issued = item
            .market
            .mint_accounting
            .total_issued
            .checked_add(item.quantity)
            .ok_or(VaultError::ArithmeticOverflow)?;
        store_state(&view[16], &item.market)?;
    }
    Ok(())
}

#[inline(never)]
fn compress(a: &[AccountInfo], prepared: &[Prepared]) -> ProgramResult {
    let entries = prepared
        .iter()
        .map(|item| {
            let n = COMMON + item.leg * LEG;
            light_token_instruction::BuybackCompression {
                amount: item.quantity,
                mint: a[n + 3].key,
                source: a[n + 4].key,
                authority: a[n + 2].key,
                interface: a[n + 6].key,
            }
        })
        .collect::<Vec<_>>();
    let ix = light_token_instruction::compress_buyback_basket(
        a[0].key,
        a[5].key,
        [a[20].key, a[21].key, a[22].key, a[23].key, a[24].key],
        &entries,
    )?;
    // One bounded stack buffer, including the invoked program after its metas.
    let mut infos: [AccountInfo; 11 + 4 * MAX_BUYBACK_LEGS] =
        std::array::from_fn(|_| a[13].clone());
    for (i, role) in [20, 0, 11, 21, 22, 23, 13, 24, 5, 12].iter().enumerate() {
        infos[i] = a[*role].clone();
    }
    let mut bumps = [[0u8; 1]; MAX_BUYBACK_LEGS];
    for (i, item) in prepared.iter().enumerate() {
        let n = COMMON + item.leg * LEG;
        for (j, role) in [n + 3, n + 4, n + 2, n + 6].iter().enumerate() {
            infos[10 + 4 * i + j] = a[*role].clone();
        }
        bumps[i][0] = item.market.bump;
    }
    let info_len = 10 + 4 * prepared.len();
    infos[info_len] = a[10].clone();
    let mut seed_arrays = [[&[][..]; 4]; MAX_BUYBACK_LEGS];
    for (i, item) in prepared.iter().enumerate() {
        seed_arrays[i] = custody::market_signer_seeds(&item.market, &bumps[i]);
    }
    let signers: [&[&[u8]]; MAX_BUYBACK_LEGS] = std::array::from_fn(|i| &seed_arrays[i][..]);
    invoke_signed(&ix, &infos[..=info_len], &signers[..prepared.len()])
}
