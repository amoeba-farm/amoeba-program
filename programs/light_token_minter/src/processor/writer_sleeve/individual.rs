use super::*;
use crate::individual_writer::{
    amount, derive_position, IndividualWriterAction as Action,
    IndividualWriterPosition as Position, POSITION_SEED,
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
            || a[..i].iter().any(|other| other.key == info.key)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    Ok(())
}

fn load_position(
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
        || p.version != 1
        || p.book != *a[4].key
        || index >= usize::from(book.series_count)
        || p.expiry_ts != group.expiry_ts
        || p.payoff_digest != book.records[index].payoff_digest
        || p.quantity == 0
        || p.price == 0
        || p.price > book.records[index].max_payout_per_contract_atoms
        || p.filled > p.quantity
        || crate::pubkey_is_default(&p.owner)
        || (p.claimed && (p.collateral != 0 || p.premium != 0 || !p.cancelled))
    {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    if !p.claimed {
        let backed = if p.cancelled { p.filled } else { p.quantity };
        if Some(p.collateral) != amount(backed, book.records[index].max_payout_per_contract_atoms) {
            return Err(VaultError::WriterSolvencyViolation.into());
        }
    }
    Ok(p)
}

fn persist_book(a: &[AccountInfo], book: &mut WriterSeriesBookV1) -> ProgramResult {
    book.last_updated_slot = Clock::get()?.slot;
    book.book_digest = writer_book_digest(book);
    store_state(&a[4], book)
}

/// Common 16 roles: actor, config, sleeve, group, book, position, book quote ATA,
/// actor quote ATA, quote mint/interface, Light, CPI authority, SPL, system,
/// compressible config, rent sponsor. Only the actor can cancel/claim/close.
#[inline(never)]
pub(super) fn process(program: &Pubkey, a: &[AccountInfo], action: Action) -> ProgramResult {
    if action == Action::FundSettlement {
        return fund_settlement(program, a);
    }
    let count = match action {
        Action::Open { .. } => 18,
        Action::Fill { .. } => 26,
        _ => 16,
    };
    let mut writable = vec![0, 4, 5, 6, 7, 9, 15];
    if matches!(action, Action::Fill { .. }) {
        writable.extend([16, 18, 19, 20, 21, 22]);
    }
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
    let mut position;
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
            deposit = arithmetic(amount(
                quantity,
                book.records[index].max_payout_per_contract_atoms,
            ))?;
            if deposit == 0 || deposit > maximum_collateral {
                return Err(VaultError::WriterSolvencyViolation.into());
            }
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
                version: 1,
                book: *a[4].key,
                owner: *a[0].key,
                nonce,
                series_index,
                payoff_digest: book.records[index].payoff_digest,
                expiry_ts: group.expiry_ts,
                price,
                quantity,
                collateral: deposit,
                ..Default::default()
            };
            book.individual.open_positions =
                arithmetic(book.individual.open_positions.checked_add(1))?;
        }
        Action::Fill {
            quantity,
            maximum_payment,
        } => {
            position = load_position(program, a, &book, &group)?;
            let index = usize::from(position.series_index);
            live_market(program, a, &config, &sleeve, &group, &book, index)?;
            deposit = position
                .fill(quantity, maximum_payment)
                .ok_or(VaultError::InvalidInstructionData)?;
            issue(program, a, &sleeve, &group, &mut book, index, quantity)?;
        }
        Action::Cancel | Action::Claim | Action::Close => {
            position = load_position(program, a, &book, &group)?;
            if position.owner != *a[0].key {
                return Err(VaultError::Unauthorized.into());
            }
            let index = usize::from(position.series_index);
            if action == Action::Close {
                if !position.claimed {
                    return Err(VaultError::InvalidWriterLifecycle.into());
                }
                return close_program_account(program, &a[5], &a[0]);
            }
            if action == Action::Cancel {
                // After funding, the sold portion has already paid its liability.
                // Claim it instead; never release the same backing twice.
                if book.individual.funded && position.filled > 0 {
                    return Err(VaultError::InvalidWriterLifecycle.into());
                }
                withdrawal = position
                    .cancel(book.records[index].max_payout_per_contract_atoms)
                    .ok_or(VaultError::InvalidWriterLifecycle)?;
            } else {
                let payout = if position.filled == 0 {
                    0
                } else {
                    if !book.individual.funded
                        || !matches!(
                            group.status,
                            WriterSettlementGroupStatus::Settled
                                | WriterSettlementGroupStatus::Closed
                        )
                        || crate::bytes32_is_zero(&group.final_settlement_commitment)
                    {
                        return Err(VaultError::InvalidWriterLifecycle.into());
                    }
                    crate::writer_sleeve_math::payout_per_contract(
                        &writer_book_math_series(&book)?[index],
                        group.settlement_price_atomic,
                    )
                    .map_err(writer_math_error)?
                };
                withdrawal = position
                    .claim(payout)
                    .ok_or(VaultError::InvalidWriterLifecycle)?;
                book.individual.open_positions =
                    arithmetic(book.individual.open_positions.checked_sub(1))?;
            }
        }
        _ => return Err(VaultError::InvalidInstructionData.into()),
    }
    book.individual.cash_obligations = arithmetic(
        book.individual
            .cash_obligations
            .checked_add(deposit)
            .and_then(|v| v.checked_sub(withdrawal)),
    )?;
    let actor_before = if withdrawal > 0 {
        load_or_create_light_associated_token_account(
            &a[0], &a[0], &a[8], &a[7], &a[10], &a[14], &a[15], &a[13],
        )?
        .amount
    } else {
        load_canonical_light_token_account(&a[7], a[0].key, a[8].key)?.amount
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
    let actor_after = load_canonical_light_token_account(&a[7], a[0].key, a[8].key)?.amount;
    if cash_before
        .checked_add(deposit)
        .and_then(|v| v.checked_sub(withdrawal))
        != Some(cash_after)
        || actor_before
            .checked_sub(deposit)
            .and_then(|v| v.checked_add(withdrawal))
            != Some(actor_after)
        || cash_after < book.individual.cash_obligations
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    // Sum of per-owner liability ceilings may exceed the aggregate funding by
    // dust. Once every owner has claimed, that dust has no remaining claimant.
    if book.individual.open_positions == 0 {
        book.individual.cash_obligations = 0;
    }
    store_state(&a[5], &position)?;
    persist_book(a, &mut book)
}

fn transfer<'a>(
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

fn live_market(
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
    let binding = load_collective_dlmm_context(program, &a[2], &a[3], &a[4], &a[16], &a[17])?;
    if binding.anchor_month_settled {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let market = load_valid_market(program, &a[16])?;
    let month = load_oracle_month_state(&a[17], program)?;
    ensure_market_value_flow_unpaused(config, &market)?;
    ensure_oracle_game_window(&market, &month)
}

#[inline(never)]
fn issue(
    program: &Pubkey,
    a: &[AccountInfo],
    sleeve: &WriterSleeveV1,
    group: &WriterSettlementGroupV1,
    book: &mut WriterSeriesBookV1,
    index: usize,
    quantity: u64,
) -> ProgramResult {
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
    let policy = dlmm::load_optional_policy(program, &a[25], &a[2], sleeve)?;
    let staged = custody::observe_market_staging_amount(program, &a[16], &a[20], &a[18], &a[12])?;
    let retired = custody::observe_writer_retirement_custody_amount(
        program, &a[2], &a[16], &a[21], &a[18], &a[12],
    )?;
    let inventory = policy
        .as_ref()
        .map_or(0, |p| p.series_pool_inventory_atoms[index]);
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
    book.individual.series[index].issued =
        arithmetic(book.individual.series[index].issued.checked_add(quantity))?;
    book.individual.series[index].outstanding = arithmetic(
        book.individual.series[index]
            .outstanding
            .checked_add(quantity),
    )?;
    // Existing buyers may already authorize the wallet/mint-scoped settlement
    // delegate. Preserve that capability; custody accounts still forbid delegates.
    let before = if a[19].owner == &light_token_program_id() {
        crate::processor::scoped_settlement::load_scoped_holder_token_account(
            program, &a[19], a[0].key, a[18].key,
        )?
    } else {
        load_or_create_light_associated_token_account(
            &a[0], &a[0], &a[18], &a[19], &a[10], &a[14], &a[15], &a[13],
        )?
    }
    .amount;
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
    invoke_light_token_account_transfer_with_signer_seeds(
        quantity,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &a[10],
        &a[11],
        &a[0],
        &a[20],
        &a[19],
        &a[16],
        &a[18],
        &a[22],
        &a[12],
        &a[13],
        &[&seeds],
    )?;
    if validate_token_account(&a[20])?.amount != 0
        || validate_mint_account(&a[18], a[12].key)?.supply
            != arithmetic(mint.supply.checked_add(quantity))?
        || crate::processor::scoped_settlement::load_scoped_holder_token_account(
            program, &a[19], a[0].key, a[18].key,
        )?
        .amount
            != arithmetic(before.checked_add(quantity))?
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

/// Permissionless after oracle settlement. Funds all individual long liabilities
/// once, before collective finalization; owner residuals remain in the book ATA.
#[inline(never)]
fn fund_settlement(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    // actor/config/sleeve/group/book, individual vault/sleeve vault,
    // quote mint/interface, Light/CPI/SPL/system.
    privileges(a, 13, &[0, 4, 5, 6, 8])?;
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
        || sleeve.usdc_vault != *a[6].key
        || sleeve.settlement_mint != *a[7].key
        || config.usdc_mint != *a[7].key
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    validate_collateral_mint_account(&a[7], a[11].key)?;
    validate_spl_interface_account(a[7].key, &a[8])?;
    validate_vault_token_account(&a[6], a[7].key, a[2].key)?;
    let cash_before = load_canonical_light_token_account(&a[5], a[4].key, a[7].key)?.amount;
    let sleeve_before = validate_token_account(&a[6])?.amount;
    if cash_before < book.individual.cash_obligations {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let managed = writer_book_math_series(&book)?;
    let (_, managed_total) = crate::writer_sleeve_math::settlement_series_liabilities(
        &managed,
        group.settlement_price_atomic,
    )
    .map_err(writer_math_error)?;
    let mut combined = managed.clone();
    let mut funding = 0u64;
    for (index, series) in combined.iter_mut().enumerate() {
        series.external_oi_atoms = arithmetic(book.external_total(index))?;
        let payout =
            crate::writer_sleeve_math::payout_per_contract(series, group.settlement_price_atomic)
                .map_err(writer_math_error)?;
        funding = arithmetic(funding.checked_add(arithmetic(amount(
            book.individual.series[index].issued,
            payout,
        ))?))?;
    }
    let (_, combined_total) = crate::writer_sleeve_math::settlement_series_liabilities(
        &combined,
        group.settlement_price_atomic,
    )
    .map_err(writer_math_error)?;
    let liability = arithmetic(combined_total.checked_sub(managed_total))?;
    book.individual.funded_long_liability = liability;
    book.individual.funded_stranded = arithmetic(funding.checked_sub(liability))?;
    book.individual.cash_obligations =
        arithmetic(book.individual.cash_obligations.checked_sub(funding))?;
    let bump = [book.bump];
    let seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::WRITER_SERIES_BOOK_PDA_SEED,
        a[2].key.as_ref(),
        &bump,
    ];
    if funding > 0 {
        invoke_light_token_account_transfer_with_signer_seeds(
            funding,
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
    if cash_before.checked_sub(funding)
        != Some(load_canonical_light_token_account(&a[5], a[4].key, a[7].key)?.amount)
        || sleeve_before.checked_add(funding) != Some(validate_token_account(&a[6])?.amount)
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    book.individual.funded = true;
    persist_book(a, &mut book)
}
