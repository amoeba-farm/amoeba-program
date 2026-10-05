//! Leaf-to-leaf option retirement and compressed USDC payout. No holder account
//! is created or restored. Light authenticates and nullifies the consumed leaf.
use super::*;
use crate::{
    compressed_custody::{self as custody, CustodyKind},
    compressed_option_settlement::{self as compressed, CompressedCashOptionClaim},
    instruction::ScopedSettlementActionV1,
};

pub(super) fn process(program: &Pubkey, a: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    let action: ScopedSettlementActionV1 = decode_instruction_payload(payload)?;
    match action {
        ScopedSettlementActionV1::RedeemCompressedCash(claim) => {
            settle(program, a, claim, false, false)
        }
        ScopedSettlementActionV1::SettleCompressedCash(claim) => {
            settle(program, a, claim, true, false)
        }
        ScopedSettlementActionV1::RedeemCompressedCashSponsored(claim) => {
            settle(program, a, claim, false, true)
        }
        ScopedSettlementActionV1::ExpireUnredeemed { series_index } => {
            expire(program, a, usize::from(series_index))
        }
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

pub(super) fn check_deadline(book: &WriterSeriesBookV1, index: usize) -> ProgramResult {
    let end = compressed::deadline(
        book.records[index].reserved,
        book.individual.settlement_finalized_ts,
    )
    .ok_or(VaultError::InvalidWriterLifecycle)?;
    if current_unix_timestamp()? >= end {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    Ok(())
}

fn series_index(
    book: &WriterSeriesBookV1,
    market: &Pubkey,
    mint: &Pubkey,
) -> Result<usize, ProgramError> {
    book.records[..usize::from(book.series_count)]
        .iter()
        .position(|r| r.market == *market && r.contract_mint == *mint)
        .ok_or_else(|| VaultError::InvalidWriterSeriesBook.into())
}

#[inline(never)]
fn settle(
    program: &Pubkey,
    a: &[AccountInfo],
    cash_claim: CompressedCashOptionClaim,
    keeper: bool,
    sponsored: bool,
) -> ProgramResult {
    settle_with_trading_owner(program, a, cash_claim, keeper, sponsored, None)
}

pub(in crate::processor) fn process_trading_session_close(
    program: &Pubkey,
    a: &[AccountInfo],
    claim: CompressedCashOptionClaim,
    owner: Pubkey,
    sponsored: bool,
) -> ProgramResult {
    if a.get(1).map(|i| *i.key) != Some(crate::trading_session::derive(program, &owner).0) {
        return Err(VaultError::InvalidAccountList.into());
    }
    settle_with_trading_owner(program, a, claim, false, sponsored, Some(owner))
}

fn settle_with_trading_owner(
    program: &Pubkey,
    a: &[AccountInfo],
    cash_claim: CompressedCashOptionClaim,
    keeper: bool,
    sponsored: bool,
    trading_owner: Option<Pubkey>,
) -> ProgramResult {
    let claim = cash_claim.option;
    let sponsor_index = 28;
    if a.len() != sponsor_index + usize::from(sponsored)
        || !cash_claim.valid()
        || !a[0].is_signer
        || !a[0].is_writable
        || (!keeper && !a[1].is_signer)
        || *a[12].key != light_token_program_id()
        || *a[13].key != cpi_authority()
        || *a[14].key != spl_token_program_id()
        || *a[15].key != system_program::id()
        || *a[16].key != Pubkey::new_from_array(light_sdk::constants::LIGHT_SYSTEM_PROGRAM_ID)
        || [3, 5, 6, 9, 11, 20, 21, 22]
            .iter()
            .any(|&i| !a[i].is_writable)
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    // The owner explicitly selects action10. Only its transaction payer may
    // receive the fixed fee; the keeper action never deducts a holder fee.
    if sponsored && (a[sponsor_index].key != a[0].key || a[0].key == a[1].key) {
        return Err(VaultError::InvalidAccountList.into());
    }
    let (delegate, delegate_bump) = crate::scoped_settlement::derive_collective_settlement_delegate(
        program, a[1].key, a[7].key,
    );
    if *a[8].key != delegate
        || *a[24].key != compressed::retirement_owner(program, a[3].key, a[6].key)
        || (!claim.has_delegate && *a[23].key != system_program::id())
        || (keeper && (!claim.has_delegate || *a[23].key != delegate))
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let config = load_canonical_vault_config(program, &a[2])?;
    let WriterBookContext {
        group,
        mut sleeve,
        mut book,
    } = load_writer_book_context(program, &a[3], &a[4], &a[5])?;
    if sleeve.vault_config != *a[2].key
        || sleeve.status != WriterSleeveStatus::SettlementFinalized
        || group.status != WriterSettlementGroupStatus::Settled
        || sleeve.usdc_vault != *a[9].key
        || sleeve.settlement_mint != *a[10].key
        || config.usdc_mint != *a[10].key
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let index = series_index(&book, a[6].key, a[7].key)?;
    check_deadline(&book, index)?;
    let record = book.records[index];
    if record.settlement_status != WriterSeriesSettlementStatus::Frozen
        || claim.amount > record.external_open_interest_atoms
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mut market = load_valid_market(program, &a[6])?;
    if market.market_id != record.series_id
        || market.long_contract_mint != Some(*a[7].key)
        || market_outstanding_contract_amount(&market)? != record.external_open_interest_atoms
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let mint = validate_canonical_market_mint(&a[6], &mut market, &a[7], 0)?;
    if mint.supply != record.total_physical_supply_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let payout = crate::writer_sleeve_math::cumulative_allocation_delta(
        record.settlement_external_oi_snapshot_atoms,
        record.external_open_interest_atoms,
        claim.amount,
        record.settlement_liability_initial_atoms,
    )
    .map_err(writer_math_error)?;
    if payout > record.settlement_liability_remaining_atoms
        || payout > sleeve.long_liability_remaining_atoms
        || payout > sleeve.accounted_asset_atoms
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let (holder_payout, sponsor_fee) =
        compressed::payout_parts(payout, sponsored).ok_or(ProgramError::InsufficientFunds)?;
    validate_vault_token_account(&a[9], a[10].key, a[3].key)?;
    validate_collateral_mint_account(&a[10], a[14].key)?;
    validate_spl_interface_account(a[10].key, &a[11])?;
    let cash_before = validate_token_account(&a[9])?.amount;
    let cash_key = custody::derive_compressed_custody(program, CustodyKind::WriterCash, a[9].key).0;
    let cash_absent =
        a[25].key == &cash_key && a[25].owner == &system_program::id() && a[25].data_is_empty();
    let mut cash_custody = custody::load(
        program,
        if cash_absent { None } else { Some(&a[25]) },
        CustodyKind::WriterCash,
        a[9].key,
        &Pubkey::default(),
        a[10].key,
    )?;
    if !custody::backs(
        cash_custody.as_ref(),
        0,
        cash_before,
        0,
        sleeve.accounted_asset_atoms,
    ) || cash_custody.as_ref().is_some_and(|c| c.option_atoms != 0)
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let cash_input = cash_claim.cash_amount;
    if cash_input > cash_custody.as_ref().map_or(0, |c| c.quote_atoms) {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let compressed_draw = payout.min(cash_input);
    let hot_draw = payout - compressed_draw;
    if cash_before < hot_draw {
        return Err(ProgramError::InsufficientFunds);
    }
    let compressed_holder = holder_payout.min(compressed_draw);
    let compressed_fee = compressed_draw - compressed_holder;
    {
        let c = &cash_claim;
        if (c.cash_amount == 0
            && (a[26].key != &system_program::id() || a[27].key != &system_program::id()))
            || (c.cash_amount != 0 && (!a[26].is_writable || !a[27].is_writable))
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }

    // The instruction constructs both owner and mint from authenticated series state.
    // A false amount, delegate, tree position or proof fails inside Light before payout.
    let bump = [delegate_bump];
    let seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::scoped_settlement::COLLECTIVE_SETTLEMENT_DELEGATE_SEED,
        a[1].key.as_ref(),
        a[7].key.as_ref(),
        &bump,
    ];
    let trading_bump = [trading_owner
        .map(|owner| crate::trading_session::derive(program, &owner).1)
        .unwrap_or(0)];
    let trading_seeds: Option<Vec<&[u8]>> = trading_owner.as_ref().map(|owner| {
        vec![
            CURRENT_STATE_NAMESPACE_SEED,
            crate::trading_session::SEED,
            owner.as_ref(),
            &trading_bump,
        ]
    });
    if cash_input != 0 {
        retire_and_pay_cash(
            a,
            &cash_claim,
            keeper,
            cash_custody.as_ref().unwrap().bump,
            compressed_holder,
            compressed_fee,
            seeds,
            trading_seeds.as_deref(),
        )?;
    } else {
        let keys = core::array::from_fn(|i| *a[i].key);
        let retirement = compressed::retire_instruction(&keys, &claim, keeper);
        let indices = [16, 0, 13, 17, 18, 19, 15, 20, 21, 22, 1, 7, 23, 24];
        let mut infos: Vec<_> = indices.iter().map(|&i| a[i].clone()).collect();
        infos.push(a[12].clone());
        if keeper {
            invoke_signed(&retirement, &infos, &[seeds])?;
        } else {
            if let Some(trading) = trading_seeds.as_deref() {
                invoke_signed(&retirement, &infos, &[trading])?;
            } else {
                invoke(&retirement, &infos)?;
            }
        }
    }
    for (recipient, amount) in [
        (1usize, holder_payout - compressed_holder),
        (sponsor_index, sponsor_fee - compressed_fee),
    ] {
        if amount == 0 {
            continue;
        }
        let ix = light_token_instruction::compress_to_wallet(
            amount,
            MarketMintAccounting::CANONICAL_DECIMALS,
            a[9].key,
            a[9].owner,
            a[3].key,
            a[0].key,
            a[10].key,
            a[recipient].key,
            a[11].key,
            [a[16].key, a[17].key, a[18].key, a[19].key, a[22].key],
        )?;
        let bump = [sleeve.bump];
        let signer = writer_sleeve_signer_seeds(&sleeve.settlement_group, &bump);
        let indices = [
            16, 0, 13, 17, 18, 19, 15, 22, 10, 9, 3, recipient, 11, 14, 12,
        ];
        let infos: Vec<_> = indices.iter().map(|&i| a[i].clone()).collect();
        invoke_signed(&ix, &infos, &[&signer])?;
    }
    if cash_before.checked_sub(validate_token_account(&a[9])?.amount) != Some(hot_draw)
        || validate_mint_account(&a[7], a[14].key)?.supply != mint.supply
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    if let Some(state) = cash_custody.as_mut() {
        state.quote_atoms = state
            .quote_atoms
            .checked_sub(compressed_draw)
            .ok_or(VaultError::ArithmeticOverflow)?;
        custody::store(&a[25], state)?;
    }
    apply_retirement(
        &mut book,
        &mut sleeve,
        &mut market,
        index,
        claim.amount,
        payout,
    )?;
    let slot = Clock::get()?.slot;
    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = slot;
    sleeve.last_updated_slot = slot;
    store_state(&a[6], &market)?;
    store_state(&a[5], &book)?;
    store_state(&a[3], &sleeve)
}

/// One Light invocation consumes both mints. It cannot retire an option while
/// leaving its cash input unspent, or pay cash without nullifying the option.
#[inline(never)]
fn retire_and_pay_cash<'a>(
    a: &[AccountInfo<'a>],
    cash: &CompressedCashOptionClaim,
    keeper: bool,
    custody_bump: u8,
    holder_amount: u64,
    fee: u64,
    delegate_seeds: &[&[u8]],
    trading_seeds: Option<&[&[u8]]>,
) -> ProgramResult {
    use crate::regular_compressed_transfer::{self as transfer, InputLeaf, OutputLeaf};
    // Seven fixed Light metas followed by option context, cash context and identities.
    let indices = [
        16, 0, 13, 17, 18, 19, 15, 20, 21, 22, 26, 27, 1, 7, 23, 24, 25, 10, 0,
    ];
    let metas = indices
        .iter()
        .enumerate()
        .map(|(i, &n)| solana_program::instruction::AccountMeta {
            pubkey: *a[n].key,
            is_writable: matches!(i, 1 | 7 | 8 | 9 | 10 | 11),
            is_signer: i == 1 || i == 16 || (i == 12 && !keeper) || (i == 14 && keeper),
        })
        .collect();
    let inputs = [
        InputLeaf {
            owner: 5,
            amount: cash.option.amount,
            has_delegate: cash.option.has_delegate,
            delegate: 7,
            mint: 6,
            tree: 0,
            queue: 1,
            leaf_index: cash.option.leaf_index,
            prove_by_index: cash.option.prove_by_index,
            root_index: cash.option.root_index,
        },
        InputLeaf {
            owner: 9,
            amount: cash.cash_amount,
            has_delegate: false,
            delegate: 0,
            mint: 10,
            tree: 3,
            queue: 4,
            leaf_index: cash.cash_leaf_index,
            prove_by_index: cash.cash_prove_by_index,
            root_index: cash.cash_root_index,
        },
    ];
    let mut outputs = vec![OutputLeaf {
        owner: 8,
        amount: cash.option.amount,
        has_delegate: false,
        delegate: 0,
        mint: 6,
    }];
    for (owner, amount) in [
        (5, holder_amount),
        (11, fee),
        (9, cash.cash_amount - holder_amount - fee),
    ] {
        if amount != 0 {
            outputs.push(OutputLeaf {
                owner,
                amount,
                has_delegate: false,
                delegate: 0,
                mint: 10,
            });
        }
    }
    let ix = transfer::instruction(*a[12].key, metas, 2, cash.option.proof, &inputs, &outputs)?;
    let mut infos: Vec<_> = indices.iter().map(|&i| a[i].clone()).collect();
    infos.push(a[12].clone());
    let bump = [custody_bump];
    let kind = [CustodyKind::WriterCash as u8];
    let cash_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        custody::COMPRESSED_CUSTODY_SEED,
        &kind,
        a[9].key.as_ref(),
        &bump,
    ];
    if keeper {
        invoke_signed(&ix, &infos, &[cash_seeds, delegate_seeds])
    } else {
        if let Some(trading) = trading_seeds {
            invoke_signed(&ix, &infos, &[cash_seeds, trading])
        } else {
            invoke_signed(&ix, &infos, &[cash_seeds])
        }
    }
}

pub(super) fn apply_retirement(
    book: &mut WriterSeriesBookV1,
    sleeve: &mut WriterSleeveV1,
    market: &mut Market,
    index: usize,
    amount: u64,
    payout: u64,
) -> ProgramResult {
    let record = &mut book.records[index];
    record.external_open_interest_atoms = record
        .external_open_interest_atoms
        .checked_sub(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    record.settlement_liability_remaining_atoms = record
        .settlement_liability_remaining_atoms
        .checked_sub(payout)
        .ok_or(VaultError::ArithmeticOverflow)?;
    book.individual.compressed_retired_atoms[index] = book.individual.compressed_retired_atoms
        [index]
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    market.mint_accounting.total_consumed = market
        .mint_accounting
        .total_consumed
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if record.external_open_interest_atoms == 0 {
        record.settlement_status = WriterSeriesSettlementStatus::Exhausted;
    }
    sleeve.long_liability_remaining_atoms = sleeve
        .long_liability_remaining_atoms
        .checked_sub(payout)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.accounted_asset_atoms = sleeve
        .accounted_asset_atoms
        .checked_sub(payout)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.exact_reserve_atoms = sleeve
        .long_liability_remaining_atoms
        .checked_add(book.individual.remaining_portfolio_credit)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if market_outstanding_contract_amount(market)? != record.external_open_interest_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    Ok(())
}

/// Terminalizes only the predeclared 72-hour series. Never moves any user's tokens.
fn expire(program: &Pubkey, a: &[AccountInfo], index: usize) -> ProgramResult {
    if a.len() != 7 || !a[0].is_signer || [2, 4, 5].iter().any(|&i| !a[i].is_writable) {
        return Err(VaultError::InvalidAccountList.into());
    }
    let config = load_canonical_vault_config(program, &a[1])?;
    let WriterBookContext {
        group,
        mut sleeve,
        mut book,
    } = load_writer_book_context(program, &a[2], &a[3], &a[4])?;
    if index >= usize::from(book.series_count)
        || sleeve.vault_config != *a[1].key
        || sleeve.settlement_mint != config.usdc_mint
        || sleeve.status != WriterSleeveStatus::SettlementFinalized
        || group.status != WriterSettlementGroupStatus::Settled
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let record = &mut book.records[index];
    let end = compressed::deadline(record.reserved, book.individual.settlement_finalized_ts)
        .ok_or(VaultError::InvalidWriterLifecycle)?;
    if !matches!(record.reserved, [1, 0 | 1, 0, 0])
        || current_unix_timestamp()? < end
        || record.market != *a[5].key
        || record.contract_mint != *a[6].key
        || record.settlement_status != WriterSeriesSettlementStatus::Frozen
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let mut market = load_valid_market(program, &a[5])?;
    if market.long_contract_mint != Some(*a[6].key)
        || market_outstanding_contract_amount(&market)? != record.external_open_interest_atoms
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let amount = record.external_open_interest_atoms;
    let released = record.settlement_liability_remaining_atoms;
    book.individual.forfeited_atoms[index] = book.individual.forfeited_atoms[index]
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    market.mint_accounting.total_consumed = market
        .mint_accounting
        .total_consumed
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    record.external_open_interest_atoms = 0;
    record.settlement_liability_remaining_atoms = 0;
    record.settlement_status = WriterSeriesSettlementStatus::Exhausted;
    sleeve.long_liability_remaining_atoms = sleeve
        .long_liability_remaining_atoms
        .checked_sub(released)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.accounted_asset_atoms = sleeve
        .accounted_asset_atoms
        .checked_sub(released)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.stranded_surplus_atoms = sleeve
        .stranded_surplus_atoms
        .checked_add(released)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.exact_reserve_atoms = sleeve
        .long_liability_remaining_atoms
        .checked_add(book.individual.remaining_portfolio_credit)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let slot = Clock::get()?.slot;
    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = slot;
    sleeve.last_updated_slot = slot;
    store_state(&a[5], &market)?;
    store_state(&a[4], &book)?;
    store_state(&a[2], &sleeve)
}
