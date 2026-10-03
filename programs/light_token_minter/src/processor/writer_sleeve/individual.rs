use super::*;
use crate::individual_writer::{
    derive_position, IndividualPortfolioCashWitnessV1, IndividualWriterAction as Action,
    IndividualWriterPosition as Position, POSITION_SEED,
};
use crate::writer_portfolio::{
    derive_portfolio, IndividualWriterPortfolio as Portfolio, PORTFOLIO_SEED,
};

fn arithmetic<T>(value: Option<T>) -> Result<T, ProgramError> {
    value.ok_or_else(|| VaultError::ArithmeticOverflow.into())
}

fn privileges(a: &[AccountInfo], count: usize, writable: &[usize]) -> ProgramResult {
    if a.len() != count {
        return Err(VaultError::InvalidAccountList.into());
    }
    for (i, info) in a.iter().enumerate() {
        if info.is_signer != (i == 0)
            || info.is_writable != writable.contains(&i)
            || a[..i]
                .iter()
                .any(|other| crate::pubkey_eq(other.key, info.key))
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    Ok(())
}

pub(super) fn load_position(
    program: &Pubkey,
    a: &[AccountInfo],
    book: &WriterSeriesBookV1,
    group: &WriterSettlementGroupV1,
) -> Result<Position, ProgramError> {
    let p = load_exact_zero_padded_state::<Position>(
        &a[5],
        program,
        Position::LEN,
        VaultError::InvalidWriterSleeve,
    )?;
    let (key, bump) = derive_position(program, a[4].key, &p.owner, p.nonce);
    let index = usize::from(p.series_index);
    if key != *a[5].key
        || a[5].executable
        || !p.initialized
        || p.bump != bump
        || p.discriminator != *b"IWP"
        || p.version != 2
        || p.book != *a[4].key
        || index >= usize::from(book.series_count)
        || p.expiry_ts != group.expiry_ts
        || p.payoff_digest != book.records[index].payoff_digest
        || p.quantity == 0
        || p.price == 0
        || p.price > book.records[index].max_payout_per_contract_atoms
        || p.filled > p.quantity
        || crate::pubkey_is_default(&p.owner)
        || (p.claimed && !p.cancelled)
    {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    Ok(p)
}

pub(super) fn load_portfolio(
    program: &Pubkey,
    info: &AccountInfo,
    book: &Pubkey,
    owner: &Pubkey,
    expiry_ts: u64,
) -> Result<Portfolio, ProgramError> {
    let portfolio = load_exact_zero_padded_state::<Portfolio>(
        info,
        program,
        Portfolio::LEN,
        VaultError::InvalidWriterSleeve,
    )?;
    let (key, bump) = derive_portfolio(program, book, owner);
    if *info.key != key
        || !portfolio.initialized
        || portfolio.bump != bump
        || portfolio.discriminator != Portfolio::DISCRIMINATOR
        || portfolio.version != Portfolio::VERSION
        || portfolio.book != *book
        || portfolio.owner != *owner
        || portfolio.expiry_ts != expiry_ts
        || portfolio.validate().is_err()
    {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    Ok(portfolio)
}

pub(super) fn load_or_create_portfolio<'a>(
    program: &Pubkey,
    payer: &AccountInfo<'a>,
    owner_signer: &AccountInfo<'a>,
    info: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    book: &Pubkey,
    owner: &Pubkey,
    expiry_ts: u64,
) -> Result<Portfolio, ProgramError> {
    let (key, bump) = derive_portfolio(program, book, owner);
    if *info.key != key {
        return Err(VaultError::InvalidPda.into());
    }
    if info.owner == program {
        return load_portfolio(program, info, book, owner, expiry_ts);
    }
    if owner_signer.key != owner || !owner_signer.is_signer || !payer.is_signer {
        return Err(VaultError::Unauthorized.into());
    }
    validate_create_only_program_account_target(program, info)?;
    create_program_account(
        payer,
        info,
        system,
        program,
        Portfolio::LEN,
        &[PORTFOLIO_SEED, book.as_ref(), owner.as_ref(), &[bump]],
    )?;
    let portfolio = Portfolio {
        initialized: true,
        bump,
        discriminator: Portfolio::DISCRIMINATOR,
        version: Portfolio::VERSION,
        book: *book,
        owner: *owner,
        expiry_ts,
        ..Default::default()
    };
    store_state(info, &portfolio)?;
    Ok(portfolio)
}

pub(super) fn persist_book(a: &[AccountInfo], book: &mut WriterSeriesBookV1) -> ProgramResult {
    book.last_updated_slot = Clock::get()?.slot;
    book.book_digest = writer_book_digest(book);
    store_state(&a[4], book)
}

#[inline(never)]
fn claim_portfolio(
    program: &Pubkey,
    a: &[AccountInfo],
    cash_witness: Option<IndividualPortfolioCashWitnessV1>,
) -> ProgramResult {
    use crate::{
        compressed_custody::{self as custody, CustodyKind},
        regular_compressed_transfer::{self as compressed_transfer, InputLeaf, OutputLeaf},
    };
    use solana_program::instruction::AccountMeta;
    if a.len() != 25
        || !a[0].is_signer
        || !a[0].is_writable
        || !a[1].is_signer
        || [3, 5, 6, 7, 8, 10, 16, 21, 24]
            .iter()
            .any(|&i| !a[i].is_writable)
        || *a[11].key != light_token_program_id()
        || *a[12].key != cpi_authority()
        || *a[13].key != spl_token_program_id()
        || *a[14].key != system_program::id()
        || *a[15].key != light_token_instruction::compressible_config()
        || *a[16].key != light_token_instruction::rent_sponsor()
        || *a[17].key != Pubkey::new_from_array(light_sdk::constants::LIGHT_SYSTEM_PROGRAM_ID)
        || *a[18].key != Pubkey::new_from_array(light_sdk::constants::REGISTERED_PROGRAM_PDA)
        || *a[19].key
            != Pubkey::new_from_array(light_sdk::constants::ACCOUNT_COMPRESSION_AUTHORITY_PDA)
        || *a[20].key
            != Pubkey::new_from_array(light_sdk::constants::ACCOUNT_COMPRESSION_PROGRAM_ID)
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    if cash_witness.is_some_and(|w| w.amount == 0 || (!w.prove_by_index && w.proof.is_none()))
        || (cash_witness.is_some() && (!a[22].is_writable || !a[23].is_writable))
        || (cash_witness.is_none()
            && (*a[22].key != system_program::id() || *a[23].key != system_program::id()))
    {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let config = load_canonical_vault_config(program, &a[2])?;
    let WriterBookContext {
        mut sleeve,
        group,
        mut book,
    } = load_writer_book_context(program, &a[3], &a[4], &a[5])?;
    if sleeve.vault_config != *a[2].key
        || sleeve.status != WriterSleeveStatus::SettlementFinalized
        || group.status != WriterSettlementGroupStatus::Settled
        || !book.individual.funded
        || book.individual.pending_portfolio_funding != 0
        || sleeve.usdc_vault != *a[8].key
        || sleeve.settlement_mint != *a[9].key
        || config.usdc_mint != *a[9].key
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    validate_collateral_mint_account(&a[9], a[13].key)?;
    validate_spl_interface_account(a[9].key, &a[10])?;
    validate_vault_token_account(&a[8], a[9].key, a[3].key)?;
    let mut portfolio = load_portfolio(program, &a[6], a[5].key, a[1].key, group.expiry_ts)?;
    // An owner who only posted unfilled asks was never included in pending
    // funding and owes zero at settlement. Their cash remains claimable.
    if !portfolio.settlement_funded
        && !portfolio.funding_registered
        && portfolio.filled.iter().all(|q| *q == 0)
        && portfolio.locked.iter().all(|q| *q == 0)
    {
        portfolio.settlement_funded = true;
    }
    let (next, book_refund, credit) = portfolio.claimed().map_err(writer_math_error)?;
    if credit > book.individual.remaining_portfolio_credit
        || credit > sleeve.accounted_asset_atoms
        || credit > sleeve.exact_reserve_atoms
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let book_before = if a[7].owner == &system_program::id() && a[7].data_is_empty() {
        validate_light_associated_token_address(a[5].key, a[9].key, &a[7])?;
        0
    } else {
        load_canonical_light_token_account(&a[7], a[5].key, a[9].key)?.amount
    };
    if book_before < book_refund || book_before < book.individual.cash_obligations {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let sleeve_before = validate_token_account(&a[8])?.amount;
    let cash_key = custody::derive_compressed_custody(program, CustodyKind::WriterCash, a[8].key).0;
    let absent =
        *a[21].key == cash_key && a[21].owner == &system_program::id() && a[21].data_is_empty();
    let mut cash_custody = custody::load(
        program,
        if absent { None } else { Some(&a[21]) },
        CustodyKind::WriterCash,
        a[8].key,
        &Pubkey::default(),
        a[9].key,
    )?;
    if !custody::backs(
        cash_custody.as_ref(),
        0,
        sleeve_before,
        0,
        sleeve.accounted_asset_atoms,
    ) || cash_custody.as_ref().is_some_and(|c| c.option_atoms != 0)
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let input_amount = cash_witness.map_or(0, |w| w.amount);
    if input_amount > cash_custody.as_ref().map_or(0, |c| c.quote_atoms)
        || (input_amount > 0 && credit == 0)
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let compressed_draw = credit.min(input_amount);
    let hot_draw = credit - compressed_draw;
    if sleeve_before < hot_draw {
        return Err(ProgramError::InsufficientFunds);
    }
    if let Some(w) = cash_witness {
        let indices = [17, 0, 12, 18, 19, 20, 14, 22, 23, 24, 1, 21, 9];
        let metas = indices
            .iter()
            .enumerate()
            .map(|(i, n)| AccountMeta {
                pubkey: *a[*n].key,
                is_writable: i == 1 || (7..=9).contains(&i),
                is_signer: i == 1 || i == 11,
            })
            .collect();
        let input = InputLeaf {
            owner: 4,
            amount: w.amount,
            has_delegate: false,
            delegate: 0,
            mint: 5,
            tree: 0,
            queue: 1,
            leaf_index: w.leaf_index,
            prove_by_index: w.prove_by_index,
            root_index: w.root_index,
        };
        let mut outputs = vec![OutputLeaf {
            owner: 3,
            amount: compressed_draw,
            has_delegate: false,
            delegate: 0,
            mint: 5,
        }];
        if w.amount > compressed_draw {
            outputs.push(OutputLeaf {
                owner: 4,
                amount: w.amount - compressed_draw,
                has_delegate: false,
                delegate: 0,
                mint: 5,
            });
        }
        let ix =
            compressed_transfer::instruction(*a[11].key, metas, 2, w.proof, &[input], &outputs)?;
        let mut infos: Vec<_> = indices.iter().map(|n| a[*n].clone()).collect();
        infos.push(a[11].clone());
        let bump = [cash_custody.as_ref().unwrap().bump];
        let kind = [CustodyKind::WriterCash as u8];
        let seeds: &[&[u8]] = &[
            CURRENT_STATE_NAMESPACE_SEED,
            custody::COMPRESSED_CUSTODY_SEED,
            &kind,
            a[8].key.as_ref(),
            &bump,
        ];
        invoke_signed(&ix, &infos, &[seeds])?;
        let state = cash_custody.as_mut().unwrap();
        state.quote_atoms = state
            .quote_atoms
            .checked_sub(compressed_draw)
            .ok_or(VaultError::ArithmeticOverflow)?;
        custody::store(&a[21], state)?;
    }
    let book_bump = [book.bump];
    let book_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::WRITER_SERIES_BOOK_PDA_SEED,
        a[3].key.as_ref(),
        &book_bump,
    ];
    let sleeve_bump = [sleeve.bump];
    let sleeve_seeds = writer_sleeve_signer_seeds(&sleeve.settlement_group, &sleeve_bump);
    for (amount, source, authority, seeds) in [
        (book_refund, 7usize, 5usize, book_seeds),
        (hot_draw, 8usize, 3usize, sleeve_seeds.as_slice()),
    ] {
        if amount == 0 {
            continue;
        }
        let ix = light_token_instruction::compress_to_wallet(
            amount,
            MarketMintAccounting::CANONICAL_DECIMALS,
            a[source].key,
            a[source].owner,
            a[authority].key,
            a[0].key,
            a[9].key,
            a[1].key,
            a[10].key,
            [a[17].key, a[18].key, a[19].key, a[20].key, a[24].key],
        )?;
        let indices = [
            17, 0, 12, 18, 19, 20, 14, 24, 9, source, authority, 1, 10, 13, 11,
        ];
        let infos: Vec<_> = indices.iter().map(|i| a[*i].clone()).collect();
        invoke_signed(&ix, &infos, &[seeds])?;
    }
    if book_refund > 0
        && load_canonical_light_token_account(&a[7], a[5].key, a[9].key)?.amount
            != book_before - book_refund
        || validate_token_account(&a[8])?.amount != sleeve_before - hot_draw
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    book.individual.cash_obligations = book
        .individual
        .cash_obligations
        .checked_sub(book_refund)
        .ok_or(VaultError::ArithmeticOverflow)?;
    book.individual.remaining_portfolio_credit = book
        .individual
        .remaining_portfolio_credit
        .checked_sub(credit)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.accounted_asset_atoms = sleeve
        .accounted_asset_atoms
        .checked_sub(credit)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.exact_reserve_atoms = sleeve
        .long_liability_remaining_atoms
        .checked_add(book.individual.remaining_portfolio_credit)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.last_updated_slot = Clock::get()?.slot;
    store_state(&a[6], &next)?;
    book.last_updated_slot = Clock::get()?.slot;
    book.book_digest = writer_book_digest(&book);
    store_state(&a[5], &book)?;
    store_state(&a[3], &sleeve)
}

/// Common 16 roles: actor, config, sleeve, group, book, position, book quote ATA,
/// actor quote ATA, quote mint/interface, Light, CPI authority, SPL, system,
/// compressible config, rent sponsor. Only the actor can cancel/claim/close.
#[inline(never)]
pub(super) fn process(program: &Pubkey, a: &[AccountInfo], action: Action) -> ProgramResult {
    if let Action::BuybackFromAsks(wire) = action {
        return super::individual_buyback::process(program, a, wire);
    }
    if matches!(
        action,
        Action::LockHedge(_) | Action::UnlockHedge(_) | Action::RetireHedge(_)
    ) {
        return super::portfolio_hedge::process(program, a, action);
    }
    if let Action::ClaimPortfolio(witness) = action {
        return claim_portfolio(program, a, witness);
    }
    if action == Action::FundSettlement {
        return fund_settlement(program, a);
    }
    // Selector 1 remains decodable for historical provenance, but future
    // purchases must carry the owner's explicit compressed expiry grant.
    if matches!(action, Action::Fill { .. }) {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let count = match action {
        Action::Open { .. } => 19,
        Action::FillCompressed { .. } => 32,
        _ => 17,
    };
    let mut writable = vec![0, 4, 5, 6, 7, 9, 15];
    if matches!(action, Action::FillCompressed { .. }) {
        writable.extend([16, 18, 20, 21, 22, 30]);
    }
    writable.push(count - 1);
    privileges(a, count, &writable)?;
    validate_writer_compression_accounts(&a[10], &a[11], &a[12], &a[13], &a[14], &a[15])?;
    let config = load_canonical_vault_config(program, &a[1])?;
    let WriterBookContext {
        sleeve,
        group,
        mut book,
    } = load_writer_book_context(program, &a[2], &a[3], &a[4])?;
    if sleeve.vault_config != *a[1].key
        || group.sleeve != *a[2].key
        || !book.frozen
        || sleeve.settlement_mint != *a[8].key
        || config.usdc_mint != *a[8].key
    {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    validate_collateral_mint_account(&a[8], a[12].key)?;
    validate_spl_interface_account(a[8].key, &a[9])?;
    let opening = matches!(action, Action::Open { .. });
    let cash_before = if opening {
        load_or_create_light_associated_token_account(
            &a[0], &a[4], &a[8], &a[6], &a[10], &a[14], &a[15], &a[13],
        )?
        .amount
    } else {
        load_canonical_light_token_account(&a[6], a[4].key, a[8].key)?.amount
    };
    if cash_before < book.individual.cash_obligations {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let series = writer_book_math_series(&book)?;
    let portfolio_info = &a[count - 1];
    let mut position;
    let mut portfolio;
    let mut deposit = 0;
    let mut withdrawal = 0;
    match action {
        Action::Open {
            nonce,
            series_index,
            quantity,
            price,
            maximum_collateral,
        } => {
            let index = usize::from(series_index);
            if index >= usize::from(book.series_count)
                || quantity == 0
                || price == 0
                || price > book.records[index].max_payout_per_contract_atoms
            {
                return Err(VaultError::InvalidInstructionData.into());
            }
            live_market(program, a, &config, &sleeve, &group, &book, index)?;
            portfolio = load_or_create_portfolio(
                program,
                &a[0],
                &a[0],
                portfolio_info,
                &a[13],
                a[4].key,
                a[0].key,
                group.expiry_ts,
            )?;
            (portfolio, deposit) = portfolio
                .opened(&series, index, quantity, maximum_collateral)
                .map_err(writer_math_error)?;
            portfolio.open_positions = arithmetic(portfolio.open_positions.checked_add(1))?;
            let (key, bump) = derive_position(program, a[4].key, a[0].key, nonce);
            if key != *a[5].key {
                return Err(VaultError::InvalidPda.into());
            }
            validate_create_only_program_account_target(program, &a[5])?;
            create_program_account(
                &a[0],
                &a[5],
                &a[13],
                program,
                Position::LEN,
                &[
                    POSITION_SEED,
                    a[4].key.as_ref(),
                    a[0].key.as_ref(),
                    &nonce.to_le_bytes(),
                    &[bump],
                ],
            )?;
            position = Position {
                initialized: true,
                bump,
                discriminator: *b"IWP",
                version: 2,
                book: *a[4].key,
                owner: *a[0].key,
                nonce,
                series_index,
                payoff_digest: book.records[index].payoff_digest,
                expiry_ts: group.expiry_ts,
                price,
                quantity,
                // Informational top-up only; portfolio cash is the sole backing ledger.
                collateral: deposit,
                ..Default::default()
            };
            book.individual.open_positions =
                arithmetic(book.individual.open_positions.checked_add(1))?;
        }
        Action::FillCompressed {
            quantity,
            maximum_payment,
        } => {
            position = load_position(program, a, &book, &group)?;
            portfolio = load_portfolio(
                program,
                portfolio_info,
                a[4].key,
                &position.owner,
                group.expiry_ts,
            )?;
            let index = usize::from(position.series_index);
            live_market(program, a, &config, &sleeve, &group, &book, index)?;
            deposit = position
                .fill(quantity, maximum_payment)
                .ok_or(VaultError::InvalidInstructionData)?;
            portfolio = portfolio
                .filled(index, quantity, deposit)
                .map_err(writer_math_error)?;
            if !portfolio.funding_registered {
                portfolio.funding_registered = true;
                book.individual.pending_portfolio_funding =
                    arithmetic(book.individual.pending_portfolio_funding.checked_add(1))?;
            }
            issue_to(
                program, a, &sleeve, &group, &mut book, index, quantity, &a[0], true,
            )?;
        }
        Action::Cancel | Action::Claim | Action::Close => {
            position = load_position(program, a, &book, &group)?;
            portfolio = load_portfolio(
                program,
                portfolio_info,
                a[4].key,
                &position.owner,
                group.expiry_ts,
            )?;
            if position.owner != *a[0].key {
                return Err(VaultError::Unauthorized.into());
            }
            let index = usize::from(position.series_index);
            if action == Action::Close {
                if !position.claimed || !portfolio.claimed {
                    return Err(VaultError::InvalidWriterLifecycle.into());
                }
                return close_program_account(program, &a[5], &a[0]);
            }
            if action == Action::Cancel {
                if position.cancelled || portfolio.settlement_funded {
                    return Err(VaultError::InvalidWriterLifecycle.into());
                }
                let unfilled = arithmetic(position.quantity.checked_sub(position.filled))?;
                (portfolio, withdrawal) = portfolio
                    .cancelled(&series, index, unfilled)
                    .map_err(writer_math_error)?;
                position.cancelled = true;
            } else {
                if !portfolio.claimed || position.claimed {
                    return Err(VaultError::InvalidWriterLifecycle.into());
                }
                position.claimed = true;
                position.cancelled = true;
                book.individual.open_positions =
                    arithmetic(book.individual.open_positions.checked_sub(1))?;
                portfolio.open_positions = arithmetic(portfolio.open_positions.checked_sub(1))?;
            }
        }
        Action::Fill { .. } => unreachable!("retired selector rejected above"),
        _ => return Err(VaultError::InvalidInstructionData.into()),
    }
    book.individual.cash_obligations = arithmetic(
        book.individual
            .cash_obligations
            .checked_add(deposit)
            .and_then(|v| v.checked_sub(withdrawal)),
    )?;
    let actor_before = if deposit == 0 && withdrawal == 0 {
        None
    } else if withdrawal > 0 {
        Some(
            load_or_create_light_associated_token_account(
                &a[0], &a[0], &a[8], &a[7], &a[10], &a[14], &a[15], &a[13],
            )?
            .amount,
        )
    } else {
        Some(load_canonical_light_token_account(&a[7], a[0].key, a[8].key)?.amount)
    };
    let bump = [book.bump];
    let seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::WRITER_SERIES_BOOK_PDA_SEED,
        a[2].key.as_ref(),
        &bump,
    ];
    if deposit > 0 {
        transfer(a, deposit, &a[7], &a[6], &a[0], &[])?;
    } else if withdrawal > 0 {
        transfer(a, withdrawal, &a[6], &a[7], &a[4], &[seeds])?;
    }
    let cash_after = load_canonical_light_token_account(&a[6], a[4].key, a[8].key)?.amount;
    let actor_after = if actor_before.is_some() {
        Some(load_canonical_light_token_account(&a[7], a[0].key, a[8].key)?.amount)
    } else {
        None
    };
    if cash_before
        .checked_add(deposit)
        .and_then(|v| v.checked_sub(withdrawal))
        != Some(cash_after)
        || actor_before
            .map(|before| {
                before
                    .checked_sub(deposit)
                    .and_then(|v| v.checked_add(withdrawal))
            })
            .is_some_and(|expected| expected != actor_after)
        || cash_after < book.individual.cash_obligations
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    store_state(&a[5], &position)?;
    store_state(portfolio_info, &portfolio)?;
    persist_book(a, &mut book)
}

pub(super) fn transfer<'a>(
    a: &[AccountInfo<'a>],
    amount: u64,
    source: &AccountInfo<'a>,
    destination: &AccountInfo<'a>,
    authority: &AccountInfo<'a>,
    seeds: &[&[&[u8]]],
) -> ProgramResult {
    invoke_light_token_account_transfer_with_signer_seeds(
        amount,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &a[10],
        &a[11],
        &a[0],
        source,
        destination,
        authority,
        &a[8],
        &a[9],
        &a[12],
        &a[13],
        seeds,
    )
}

pub(super) fn live_market(
    program: &Pubkey,
    a: &[AccountInfo],
    config: &VaultConfig,
    sleeve: &WriterSleeveV1,
    group: &WriterSettlementGroupV1,
    book: &WriterSeriesBookV1,
    index: usize,
) -> ProgramResult {
    if sleeve.status != WriterSleeveStatus::Active
        || group.status != WriterSettlementGroupStatus::Active
        || book.individual.funded
        || current_unix_timestamp()? >= group.expiry_ts
        || book.records[index].market != *a[16].key
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let binding = super::collective_binding::collective_dlmm_context_from_validated_book(
        program, &a[2], &a[4], &a[16], &a[17], sleeve, group, book,
    )?;
    if binding.anchor_month_settled {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let market = load_valid_market(program, &a[16])?;
    let month = load_oracle_month_state(&a[17], program)?;
    ensure_market_value_flow_unpaused(config, &market)?;
    ensure_oracle_game_window(&market, &month)
}

/// Shared canonical issuance checks; callers authenticate the immutable policy first.
#[inline(never)]
pub(super) fn validate_issue(
    program: &Pubkey,
    a: &[AccountInfo],
    group: &WriterSettlementGroupV1,
    book: &WriterSeriesBookV1,
    index: usize,
    policy: Option<&crate::state::WriterDlmmPolicyV1>,
) -> Result<(Market, crate::token_state::Mint, u64), ProgramError> {
    let month = load_oracle_month_state(&a[17], program)?;
    let coverage = load_valid_oracle_sku_coverage_manifest(program, a[17].key, &a[23])?;
    let active = load_valid_oracle_active_weight_manifest(program, a[17].key, &a[24])?;
    ensure_finalized_oracle_active_weight_manifest(&month, &active)?;
    ensure_finalized_oracle_issue_sku_coverage(&month, &coverage, &active)?;
    if active.rolling_manifest_hash != group.active_weight_manifest_hash {
        return Err(VaultError::InvalidWriterSettlementGroup.into());
    }
    let mut market = load_valid_market(program, &a[16])?;
    let mint = validate_canonical_market_mint(&a[16], &mut market, &a[18], 0)?;
    if book.records[index].contract_mint != *a[18].key
        || mint.supply != book.records[index].total_physical_supply_atoms
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    validate_spl_interface_account(a[18].key, &a[22])?;
    let staged = custody::observe_market_staging_amount(program, &a[16], &a[20], &a[18], &a[12])?;
    let retired = custody::observe_writer_retirement_custody_amount(
        program, &a[2], &a[16], &a[21], &a[18], &a[12],
    )?;
    let inventory = policy.map_or(0, |p| p.series_pool_inventory_atoms[index]);
    if staged
        .checked_add(retired)
        .and_then(|v| v.checked_add(inventory))
        != Some(book.records[index].issuer_controlled_atoms)
        || market_outstanding_contract_amount(&market)?
            != arithmetic(
                book.external_total(index)
                    .and_then(|v| v.checked_add(inventory)),
            )?
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    Ok((market, mint, staged))
}

#[inline(never)]
pub(super) fn issue_to<'a>(
    program: &Pubkey,
    a: &[AccountInfo<'a>],
    sleeve: &WriterSleeveV1,
    group: &WriterSettlementGroupV1,
    book: &mut WriterSeriesBookV1,
    index: usize,
    quantity: u64,
    recipient: &AccountInfo<'a>,
    delegated: bool,
) -> ProgramResult {
    let policy = dlmm::load_optional_policy(program, &a[25], &a[2], sleeve)?;
    let (mut market, mint, staged) =
        validate_issue(program, a, group, book, index, policy.as_deref())?;
    book.individual.series[index].issued =
        arithmetic(book.individual.series[index].issued.checked_add(quantity))?;
    book.individual.series[index].outstanding = arithmetic(
        book.individual.series[index]
            .outstanding
            .checked_add(quantity),
    )?;
    if delegated
        && *a[19].key
            != crate::scoped_settlement::derive_collective_settlement_delegate(
                program,
                recipient.key,
                a[18].key,
            )
            .0
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    custody::load_or_create_market_staging(
        program, &a[0], &a[16], &market, &a[20], &a[18], &a[12], &a[13],
    )?;
    let bump = [market.bump];
    let seeds = custody::market_signer_seeds(&market, &bump);
    if staged > 0 {
        load_or_create_writer_retirement_custody(
            program, &a[0], &a[2], &a[16], &a[21], &a[18], &a[12], &a[13],
        )?;
        invoke_token_transfer_checked(
            &a[12],
            &a[20],
            &a[18],
            &a[21],
            &a[16],
            staged,
            MarketMintAccounting::CANONICAL_DECIMALS,
            &[&seeds],
        )?;
    }
    invoke_token_mint_to_checked(
        &a[12],
        &a[18],
        &a[20],
        &a[16],
        quantity,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &[&seeds],
    )?;
    crate::processor::ameba_dlmm::compressed_delivery::deliver(
        program,
        quantity,
        &a[20],
        &a[16],
        &seeds,
        &a[0],
        recipient,
        &a[18],
        &a[22],
        &a[10],
        &a[11],
        &a[12],
        &a[13],
        &a[26..31],
        if delegated { Some(&a[19]) } else { None },
    )?;
    if validate_token_account(&a[20])?.amount != 0
        || validate_mint_account(&a[18], a[12].key)?.supply
            != arithmetic(mint.supply.checked_add(quantity))?
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    invoke_token_close_account(&a[12], &a[20], &a[0], &a[16], &[&seeds])?;
    book.records[index].total_physical_supply_atoms =
        arithmetic(mint.supply.checked_add(quantity))?;
    market.mint_accounting.total_issued =
        arithmetic(market.mint_accounting.total_issued.checked_add(quantity))?;
    store_state(&a[16], &market)
}

/// Funds exactly one owner's net short-versus-locked-long settlement once. A
/// negative delta is a deferred sleeve credit, never a payment before finalization.
#[inline(never)]
fn fund_settlement(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    use crate::writer_portfolio::{portfolio_funding_delta, portfolio_liability_numerator};
    privileges(a, 14, &[0, 4, 5, 6, 8, 13])?;
    validate_writer_program_accounts(&a[9], &a[10], &a[11], &a[12])?;
    let config = load_canonical_vault_config(program, &a[1])?;
    let WriterBookContext {
        sleeve,
        group,
        mut book,
    } = load_writer_book_context(program, &a[2], &a[3], &a[4])?;
    if sleeve.vault_config != *a[1].key
        || group.sleeve != *a[2].key
        || group.status != WriterSettlementGroupStatus::Settled
        || sleeve.status != WriterSleeveStatus::Expired
        || crate::bytes32_is_zero(&group.final_settlement_commitment)
        || book.individual.funded
        || book.individual.pending_portfolio_funding == 0
        || sleeve.usdc_vault != *a[6].key
        || sleeve.settlement_mint != *a[7].key
        || config.usdc_mint != *a[7].key
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    validate_collateral_mint_account(&a[7], a[11].key)?;
    validate_spl_interface_account(a[7].key, &a[8])?;
    validate_vault_token_account(&a[6], a[7].key, a[2].key)?;
    let raw = load_exact_zero_padded_state::<Portfolio>(
        &a[13],
        program,
        Portfolio::LEN,
        VaultError::InvalidWriterSleeve,
    )?;
    let portfolio = load_portfolio(program, &a[13], a[4].key, &raw.owner, group.expiry_ts)?;
    if !portfolio.funding_registered
        || portfolio.settlement_funded
        || portfolio.locked != portfolio.retired
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let cash_before = if a[5].owner == &solana_program::system_program::id() && a[5].data_len() == 0
    {
        validate_light_associated_token_address(a[4].key, a[7].key, &a[5])?;
        0
    } else {
        load_canonical_light_token_account(&a[5], a[4].key, a[7].key)?.amount
    };
    let sleeve_before = validate_token_account(&a[6])?.amount;
    if cash_before < book.individual.cash_obligations || cash_before < portfolio.cash_atoms {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let series = writer_book_math_series(&book)?;
    let managed_numerator = if book.individual.funding_base_initialized {
        i128::from_le_bytes(book.individual.funding_managed_numerator_le)
    } else {
        if book.individual.hedges_consolidated {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        i128::try_from(
            crate::writer_sleeve_math::aggregate_liability_numerator(
                &series,
                group.settlement_price_atomic,
            )
            .map_err(writer_math_error)?,
        )
        .map_err(|_| VaultError::ArithmeticOverflow)?
    };
    let prefix = i128::from_le_bytes(book.individual.funding_prefix_numerator_le);
    let owner_numerator = portfolio_liability_numerator(
        &series,
        &portfolio.filled,
        &portfolio.locked,
        group.settlement_price_atomic,
    )
    .map_err(writer_math_error)?;
    let baseline = managed_numerator
        .checked_add(prefix)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let (_, delta) =
        portfolio_funding_delta(baseline, owner_numerator).map_err(writer_math_error)?;
    let debit = u64::try_from(delta.max(0)).map_err(|_| VaultError::ArithmeticOverflow)?;
    let credit = u64::try_from(if delta < 0 {
        delta.checked_neg().ok_or(VaultError::ArithmeticOverflow)?
    } else {
        0
    })
    .map_err(|_| VaultError::ArithmeticOverflow)?;
    let next = portfolio
        .funded(&series, group.settlement_price_atomic, debit, credit)
        .map_err(writer_math_error)?;
    book.individual.funding_base_initialized = true;
    book.individual.funding_managed_numerator_le = managed_numerator.to_le_bytes();
    book.individual.funding_prefix_numerator_le = prefix
        .checked_add(owner_numerator)
        .ok_or(VaultError::ArithmeticOverflow)?
        .to_le_bytes();
    book.individual.pending_portfolio_funding =
        arithmetic(book.individual.pending_portfolio_funding.checked_sub(1))?;
    book.individual.funded_long_liability =
        arithmetic(book.individual.funded_long_liability.checked_add(debit))?;
    book.individual.total_portfolio_credit =
        arithmetic(book.individual.total_portfolio_credit.checked_add(credit))?;
    book.individual.remaining_portfolio_credit = arithmetic(
        book.individual
            .remaining_portfolio_credit
            .checked_add(credit),
    )?;
    book.individual.cash_obligations =
        arithmetic(book.individual.cash_obligations.checked_sub(debit))?;
    let bump = [book.bump];
    let seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::WRITER_SERIES_BOOK_PDA_SEED,
        a[2].key.as_ref(),
        &bump,
    ];
    if debit > 0 {
        if cash_before < debit {
            return Err(VaultError::WriterSolvencyViolation.into());
        }
        invoke_light_token_account_transfer_with_signer_seeds(
            debit,
            MarketMintAccounting::CANONICAL_DECIMALS,
            &a[9],
            &a[10],
            &a[0],
            &a[5],
            &a[6],
            &a[4],
            &a[7],
            &a[8],
            &a[11],
            &a[12],
            &[seeds],
        )?;
    }
    if debit > 0
        && (cash_before.checked_sub(debit)
            != Some(load_canonical_light_token_account(&a[5], a[4].key, a[7].key)?.amount)
            || sleeve_before.checked_add(debit) != Some(validate_token_account(&a[6])?.amount))
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    if book.individual.pending_portfolio_funding == 0 {
        book.individual.funded = true;
    }
    store_state(&a[13], &next)?;
    persist_book(a, &mut book)
}
