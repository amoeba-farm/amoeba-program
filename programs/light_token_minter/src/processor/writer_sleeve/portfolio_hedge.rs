//! Owner-authenticated regular compressed hedge custody. A wallet balance is
//! never collateral: only a leaf actually moved to the portfolio PDA counts.
use super::*;
use crate::{
    compressed_option_settlement::retirement_owner,
    individual_writer::{IndividualHedgeTransferV1, IndividualWriterAction},
    regular_compressed_transfer::{self as compressed_transfer, InputLeaf, OutputLeaf},
    writer_portfolio::PORTFOLIO_SEED,
};
use solana_program::instruction::AccountMeta;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Mode {
    Lock,
    Unlock,
    Retire,
}

fn checked<T>(value: Option<T>) -> Result<T, ProgramError> {
    value.ok_or_else(|| VaultError::ArithmeticOverflow.into())
}

fn accounts<'a>(
    program: &Pubkey,
    a: &[AccountInfo<'a>],
    mode: Mode,
    w: &IndividualHedgeTransferV1,
) -> ProgramResult {
    if a.len() != 26 + usize::from(w.merkle_account_count)
        || w.merkle_account_count == 0
        || w.merkle_account_count > 16
        || w.quantity == 0
        || w.input.amount < w.quantity
        || usize::from(w.input.tree_index) >= usize::from(w.merkle_account_count)
        || usize::from(w.input.queue_index) >= usize::from(w.merkle_account_count)
        || usize::from(w.output_queue_index) >= usize::from(w.merkle_account_count)
        || (!w.input.prove_by_index && w.proof.is_none())
        || !a[0].is_signer
        || !a[0].is_writable
        || !a[4].is_writable
        || !a[5].is_writable
        || (mode == Mode::Retire && !a[16].is_writable)
        || (mode != Mode::Retire && !a[25].is_signer)
        || (mode != Mode::Unlock && w.maximum_topup != 0)
        || (mode != Mode::Lock && w.input_has_delegate)
        || *a[19].key != retirement_owner(program, a[2].key, a[16].key)
        || !crate::light_token_instruction::is_light_system_program(a[20].key)
        || !crate::light_token_instruction::is_registered_program(a[21].key)
        || !crate::light_token_instruction::is_compression_authority(a[22].key)
        || !crate::light_token_instruction::is_compression_program(a[23].key)
        || a[26..].iter().any(|info| !info.is_writable)
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    validate_writer_compression_accounts(&a[10], &a[11], &a[12], &a[13], &a[14], &a[15])?;
    validate_spl_interface_account(a[17].key, &a[18])?;
    let scope = crate::scoped_settlement::derive_collective_settlement_delegate(
        program, a[25].key, a[17].key,
    )
    .0;
    if (mode == Mode::Retire && !crate::is_system_program(a[24].key))
        || (mode != Mode::Retire && *a[24].key != scope)
        || a[26..]
            .iter()
            .enumerate()
            .any(|(i, x)| a[26..26 + i].iter().any(|y| crate::pubkey_eq(y.key, x.key)))
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    Ok(())
}

#[inline(never)]
pub(super) fn process(
    program: &Pubkey,
    a: &[AccountInfo],
    action: IndividualWriterAction,
) -> ProgramResult {
    let (mode, wire) = match action {
        IndividualWriterAction::LockHedge(w) => (Mode::Lock, w),
        IndividualWriterAction::UnlockHedge(w) => (Mode::Unlock, w),
        IndividualWriterAction::RetireHedge(w) => (Mode::Retire, w),
        _ => return Err(VaultError::InvalidInstructionData.into()),
    };
    accounts(program, a, mode, &wire)?;
    let config = load_canonical_vault_config(program, &a[1])?;
    let WriterBookContext {
        sleeve,
        group,
        mut book,
    } = load_writer_book_context(program, &a[2], &a[3], &a[4])?;
    let index = usize::from(wire.series_index);
    if index >= usize::from(book.series_count)
        || sleeve.vault_config != *a[1].key
        || group.sleeve != *a[2].key
        || sleeve.settlement_mint != *a[8].key
        || config.usdc_mint != *a[8].key
        || book.records[index].market != *a[16].key
        || book.records[index].contract_mint != *a[17].key
        || !book.frozen
    {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    validate_collateral_mint_account(&a[8], a[12].key)?;
    validate_spl_interface_account(a[8].key, &a[9])?;
    let mut market = load_valid_market(program, &a[16])?;
    let mint = validate_canonical_market_mint(&a[16], &mut market, &a[17], 0)?;
    if market.market_id != book.records[index].series_id
        || mint.supply != book.records[index].total_physical_supply_atoms
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let owner = a[25].key;
    let mut portfolio = if mode == Mode::Lock {
        super::individual::load_or_create_portfolio(
            program,
            &a[0],
            &a[25],
            &a[5],
            &a[13],
            a[4].key,
            owner,
            group.expiry_ts,
        )?
    } else {
        super::individual::load_portfolio(program, &a[5], a[4].key, owner, group.expiry_ts)?
    };
    let series = writer_book_math_series(&book)?;
    let mut cash_delta = 0i128;
    match mode {
        Mode::Lock | Mode::Unlock => {
            if sleeve.status != WriterSleeveStatus::Active
                || group.status != WriterSettlementGroupStatus::Active
                || book.individual.funded
                || current_unix_timestamp()? >= group.expiry_ts
            {
                return Err(VaultError::InvalidWriterLifecycle.into());
            }
            if mode == Mode::Lock {
                let (next, refund) = portfolio
                    .locked(&series, index, wire.quantity)
                    .map_err(writer_math_error)?;
                portfolio = next;
                cash_delta = -i128::from(refund);
                book.individual.active_locked[index] =
                    checked(book.individual.active_locked[index].checked_add(wire.quantity))?;
                if !portfolio.funding_registered {
                    portfolio.funding_registered = true;
                    book.individual.pending_portfolio_funding =
                        checked(book.individual.pending_portfolio_funding.checked_add(1))?;
                }
            } else {
                let (next, topup) = portfolio
                    .unlocked(&series, index, wire.quantity, wire.maximum_topup)
                    .map_err(writer_math_error)?;
                portfolio = next;
                cash_delta = i128::from(topup);
                book.individual.active_locked[index] =
                    checked(book.individual.active_locked[index].checked_sub(wire.quantity))?;
            }
        }
        Mode::Retire => {
            if sleeve.status != WriterSleeveStatus::Expired
                || group.status != WriterSettlementGroupStatus::Settled
                || book.individual.funded
                || portfolio.settlement_funded
                || crate::bytes32_is_zero(&group.final_settlement_commitment)
            {
                return Err(VaultError::InvalidWriterLifecycle.into());
            }
            if !book.individual.funding_base_initialized {
                if book.individual.hedges_consolidated {
                    return Err(VaultError::WriterSupplyMismatch.into());
                }
                let managed = i128::try_from(
                    crate::writer_sleeve_math::aggregate_liability_numerator(
                        &series,
                        group.settlement_price_atomic,
                    )
                    .map_err(writer_math_error)?,
                )
                .map_err(|_| VaultError::ArithmeticOverflow)?;
                book.individual.funding_managed_numerator_le = managed.to_le_bytes();
                book.individual.funding_base_initialized = true;
            }
            if !book.individual.hedges_consolidated {
                for i in 0..usize::from(book.series_count) {
                    book.records[i].external_open_interest_atoms = checked(
                        book.records[i]
                            .external_open_interest_atoms
                            .checked_add(book.individual.series[i].outstanding),
                    )?;
                    book.individual.series[i].outstanding = 0;
                }
                book.individual.hedges_consolidated = true;
            }
            if market_outstanding_contract_amount(&market)?
                != book.records[index].external_open_interest_atoms
                || wire.quantity > book.records[index].external_open_interest_atoms
            {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            portfolio = portfolio
                .retired(index, wire.quantity)
                .map_err(writer_math_error)?;
            book.individual.hedge_retired[index] =
                checked(book.individual.hedge_retired[index].checked_add(wire.quantity))?;
            book.individual.compressed_retired_atoms[index] = checked(
                book.individual.compressed_retired_atoms[index].checked_add(wire.quantity),
            )?;
            book.records[index].external_open_interest_atoms = checked(
                book.records[index]
                    .external_open_interest_atoms
                    .checked_sub(wire.quantity),
            )?;
            market.mint_accounting.total_consumed = checked(
                market
                    .mint_accounting
                    .total_consumed
                    .checked_add(wire.quantity),
            )?;
            if market_outstanding_contract_amount(&market)?
                != book.records[index].external_open_interest_atoms
            {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
        }
    }
    transfer_leaf(a, &portfolio, mode, &wire)?;
    if cash_delta != 0 {
        move_cash(program, a, &book, cash_delta, wire.output_queue_index)?;
    }
    let reserve = crate::writer_portfolio::portfolio_reserve(
        &series,
        &portfolio.committed,
        &portfolio.locked,
    )
    .map_err(writer_math_error)?;
    if portfolio.cash_atoms < reserve {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    book.individual.cash_obligations =
        if cash_delta >= 0 {
            checked(book.individual.cash_obligations.checked_add(
                u64::try_from(cash_delta).map_err(|_| VaultError::ArithmeticOverflow)?,
            ))?
        } else {
            checked(
                book.individual.cash_obligations.checked_sub(
                    u64::try_from(
                        cash_delta
                            .checked_neg()
                            .ok_or(VaultError::ArithmeticOverflow)?,
                    )
                    .map_err(|_| VaultError::ArithmeticOverflow)?,
                ),
            )?
        };
    store_state(&a[5], &portfolio)?;
    if mode == Mode::Retire {
        store_state(&a[16], &market)?;
    }
    super::individual::persist_book(a, &mut book)
}

fn transfer_leaf<'a>(
    a: &[AccountInfo<'a>],
    portfolio: &crate::writer_portfolio::IndividualWriterPortfolio,
    mode: Mode,
    w: &IndividualHedgeTransferV1,
) -> ProgramResult {
    let ids = [20, 0, 11, 21, 22, 23, 13];
    let mut indices = ids.to_vec();
    indices.extend(26..a.len());
    let owner_index =
        u8::try_from(indices.len() - 7).map_err(|_| VaultError::ArithmeticOverflow)?;
    indices.extend([25, 5, 19, 17, 24]);
    let wallet = owner_index;
    let custody = owner_index + 1;
    let retirement = owner_index + 2;
    let mint = owner_index + 3;
    let delegate = owner_index + 4;
    let metas = indices
        .iter()
        .enumerate()
        .map(|(i, n)| AccountMeta {
            pubkey: *a[*n].key,
            is_writable: i == 1 || (7..7 + usize::from(w.merkle_account_count)).contains(&i),
            is_signer: i == 1
                || (mode == Mode::Lock && *n == 25)
                || (mode != Mode::Lock && *n == 5),
        })
        .collect();
    let source = if mode == Mode::Lock { wallet } else { custody };
    let destination = match mode {
        Mode::Lock => custody,
        Mode::Unlock => wallet,
        Mode::Retire => retirement,
    };
    let input = InputLeaf {
        owner: source,
        amount: w.input.amount,
        has_delegate: w.input_has_delegate,
        delegate,
        mint,
        tree: w.input.tree_index,
        queue: w.input.queue_index,
        leaf_index: w.input.leaf_index,
        prove_by_index: w.input.prove_by_index,
        root_index: w.input.root_index,
    };
    let mut outputs = vec![OutputLeaf {
        owner: destination,
        amount: w.quantity,
        has_delegate: mode == Mode::Unlock,
        delegate,
        mint,
    }];
    if w.input.amount > w.quantity {
        outputs.push(OutputLeaf {
            owner: source,
            amount: w.input.amount - w.quantity,
            has_delegate: mode == Mode::Lock,
            delegate,
            mint,
        });
    }
    let ix = compressed_transfer::instruction(
        *a[10].key,
        metas,
        w.output_queue_index,
        w.proof,
        &[input],
        &outputs,
    )?;
    let mut infos: Vec<_> = indices.iter().map(|n| a[*n].clone()).collect();
    infos.push(a[10].clone());
    if mode == Mode::Lock {
        invoke(&ix, &infos)
    } else {
        let bump = [portfolio.bump];
        let seeds: &[&[u8]] = &[
            CURRENT_STATE_NAMESPACE_SEED,
            PORTFOLIO_SEED,
            a[4].key.as_ref(),
            a[25].key.as_ref(),
            &bump,
        ];
        invoke_signed(&ix, &infos, &[seeds])
    }
}

fn move_cash<'a>(
    program: &Pubkey,
    a: &[AccountInfo<'a>],
    book: &WriterSeriesBookV1,
    delta: i128,
    output_queue_index: u8,
) -> ProgramResult {
    let _ = program;
    let before = load_canonical_light_token_account(&a[6], a[4].key, a[8].key)?.amount;
    let bump = [book.bump];
    let seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::WRITER_SERIES_BOOK_PDA_SEED,
        a[2].key.as_ref(),
        &bump,
    ];
    if delta > 0 {
        let amount = u64::try_from(delta).map_err(|_| VaultError::ArithmeticOverflow)?;
        invoke_light_token_account_transfer_with_signer_seeds(
            amount,
            MarketMintAccounting::CANONICAL_DECIMALS,
            &a[10],
            &a[11],
            &a[0],
            &a[7],
            &a[6],
            &a[25],
            &a[8],
            &a[9],
            &a[12],
            &a[13],
            &[],
        )?;
        if before.checked_add(amount)
            != Some(load_canonical_light_token_account(&a[6], a[4].key, a[8].key)?.amount)
        {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
    } else {
        let amount = u64::try_from(delta.checked_neg().ok_or(VaultError::ArithmeticOverflow)?)
            .map_err(|_| VaultError::ArithmeticOverflow)?;
        let queue = &a[26 + usize::from(output_queue_index)];
        let ix = light_token_instruction::compress_to_wallet(
            amount,
            MarketMintAccounting::CANONICAL_DECIMALS,
            a[6].key,
            a[6].owner,
            a[4].key,
            a[0].key,
            a[8].key,
            a[25].key,
            a[9].key,
            [a[20].key, a[21].key, a[22].key, a[23].key, queue.key],
        )?;
        let indices = [
            20,
            0,
            11,
            21,
            22,
            23,
            13,
            26 + usize::from(output_queue_index),
            8,
            6,
            4,
            25,
            9,
            12,
            10,
        ];
        let infos: Vec<_> = indices.iter().map(|i| a[*i].clone()).collect();
        invoke_signed(&ix, &infos, &[seeds])?;
        if before.checked_sub(amount)
            != Some(load_canonical_light_token_account(&a[6], a[4].key, a[8].key)?.amount)
        {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
    }
    Ok(())
}
