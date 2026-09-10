use super::*;
use crate::ameba_dlmm_state::{derive_ameba_dlmm_authority_pda, AmoebaDlmmPoolStatus};
use crate::processor::ameba_dlmm::{
    load_writer_dlmm_pool, store_writer_dlmm_pool, validate_writer_dlmm_pool_binding,
    writer_dlmm_vault_amounts,
};
use crate::state::{WriterDlmmBinV1, WRITER_DLMM_POSITION_BINS, WRITER_DLMM_POSITION_SEED};

fn validate_privileges(
    accounts: &[AccountInfo],
    count: usize,
    writable: &[usize],
) -> ProgramResult {
    if accounts.len() != count {
        return Err(VaultError::InvalidAccountList.into());
    }
    for (index, account) in accounts.iter().enumerate() {
        if account.is_signer != (index == 0)
            || account.is_writable != writable.contains(&index)
            || accounts[..index]
                .iter()
                .any(|earlier| earlier.key == account.key)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    Ok(())
}

pub(in crate::processor) fn process_initialize_position(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    series_index: u8,
) -> ProgramResult {
    validate_privileges(accounts, 12, &[0, 7, 8])?;
    let actor = &accounts[0];
    let config = load_canonical_vault_config(program_id, &accounts[1])?;
    let context = load_writer_policy_context(
        program_id,
        &accounts[2],
        &accounts[3],
        &accounts[4],
        &accounts[5],
        None,
    )?;
    let policy = load_policy(
        program_id,
        &accounts[6],
        &accounts[2],
        &context.snapshot,
        &context.book,
        true,
    )?;
    if policy.management_authority != *actor.key
        || context.sleeve.vault_config != *accounts[1].key
        || *accounts[11].key != system_program::id()
        || usize::from(series_index) >= usize::from(context.book.series_count)
        || context.book.records[usize::from(series_index)].market != *accounts[9].key
        || matches!(
            context.sleeve.status,
            WriterSleeveStatus::Expired
                | WriterSleeveStatus::SettlementFinalized
                | WriterSleeveStatus::Closed
        )
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let pool = load_writer_dlmm_pool(program_id, &accounts[7])?;
    let binding = load_collective_dlmm_context(
        program_id,
        &accounts[2],
        &accounts[3],
        &accounts[4],
        &accounts[9],
        &accounts[10],
    )?;
    validate_writer_dlmm_pool_binding(&config, &accounts[9], &accounts[10], &binding, &pool)?;
    if matches!(
        pool.status,
        AmoebaDlmmPoolStatus::Settled | AmoebaDlmmPoolStatus::Closed
    ) {
        return Err(VaultError::InvalidAmoebaDlmmStatusTransition.into());
    }
    let (position, bump) =
        derive_writer_dlmm_position_pda(program_id, accounts[7].key, accounts[2].key);
    if *accounts[8].key != position {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, &accounts[8])?;
    create_program_account(
        actor,
        &accounts[8],
        &accounts[11],
        program_id,
        WriterDlmmPositionV1::LEN,
        &[
            WRITER_DLMM_POSITION_SEED,
            accounts[7].key.as_ref(),
            accounts[2].key.as_ref(),
            &[bump],
        ],
    )?;
    store_state(
        &accounts[8],
        &WriterDlmmPositionV1 {
            is_initialized: true,
            bump,
            account_discriminator: WriterDlmmPositionV1::ACCOUNT_DISCRIMINATOR,
            account_version: WriterDlmmPositionV1::ACCOUNT_VERSION,
            pool: *accounts[7].key,
            sleeve: *accounts[2].key,
            policy: *accounts[6].key,
            market: *accounts[9].key,
            series_index,
            last_updated_slot: Clock::get()?.slot,
            ..WriterDlmmPositionV1::default()
        },
    )
}

fn apply_entries(
    position: &mut WriterDlmmPositionV1,
    entries: &[WriterDlmmBinV1],
    maximum_bin: u16,
    add: bool,
) -> Result<(u64, u64), ProgramError> {
    if entries.is_empty() || entries.len() > crate::state::WRITER_DLMM_ACTION_ENTRIES {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let mut option_total = 0u64;
    let mut quote_total = 0u64;
    let mut previous = 0u16;
    for entry in entries {
        if entry.bin_id <= previous
            || entry.bin_id > maximum_bin
            || (entry.option_atoms == 0 && entry.quote_atoms == 0)
        {
            return Err(VaultError::InvalidAmoebaDlmmGrid.into());
        }
        previous = entry.bin_id;
        option_total = option_total
            .checked_add(entry.option_atoms)
            .ok_or(VaultError::ArithmeticOverflow)?;
        quote_total = quote_total
            .checked_add(entry.quote_atoms)
            .ok_or(VaultError::ArithmeticOverflow)?;
        let count = usize::from(position.bin_count);
        let index =
            match position.bins[..count].binary_search_by_key(&entry.bin_id, |bin| bin.bin_id) {
                Ok(index) => index,
                Err(index) => {
                    if !add || count == WRITER_DLMM_POSITION_BINS {
                        return Err(VaultError::InvalidAmoebaDlmmGrid.into());
                    }
                    for offset in (index..count).rev() {
                        position.bins[offset + 1] = position.bins[offset];
                    }
                    position.bins[index] = WriterDlmmBinV1 {
                        bin_id: entry.bin_id,
                        ..WriterDlmmBinV1::default()
                    };
                    position.bin_count += 1;
                    index
                }
            };
        let bin = &mut position.bins[index];
        bin.option_atoms = if add {
            bin.option_atoms.checked_add(entry.option_atoms)
        } else {
            bin.option_atoms.checked_sub(entry.option_atoms)
        }
        .ok_or(VaultError::ArithmeticOverflow)?;
        bin.quote_atoms = if add {
            bin.quote_atoms.checked_add(entry.quote_atoms)
        } else {
            bin.quote_atoms.checked_sub(entry.quote_atoms)
        }
        .ok_or(VaultError::ArithmeticOverflow)?;
        if bin.option_atoms == 0 && bin.quote_atoms == 0 {
            let count = usize::from(position.bin_count);
            for offset in index + 1..count {
                position.bins[offset - 1] = position.bins[offset];
            }
            position.bins[count - 1] = WriterDlmmBinV1::default();
            position.bin_count -= 1;
        }
    }
    position.option_inventory_atoms = if add {
        position.option_inventory_atoms.checked_add(option_total)
    } else {
        position.option_inventory_atoms.checked_sub(option_total)
    }
    .ok_or(VaultError::ArithmeticOverflow)?;
    position.allocated_quote_atoms = if add {
        position.allocated_quote_atoms.checked_add(quote_total)
    } else {
        position.allocated_quote_atoms.checked_sub(quote_total)
    }
    .ok_or(VaultError::ArithmeticOverflow)?;
    Ok((option_total, quote_total))
}

/// Add/remove use exact canonical token deltas. No ordinary reserve page or share is changed.
pub(in crate::processor) fn process_liquidity_action(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    action: ManageWriterDlmmV1Params,
) -> ProgramResult {
    validate_privileges(
        accounts,
        27,
        &[0, 2, 4, 6, 7, 9, 11, 12, 14, 15, 16, 17, 18, 21, 22, 26],
    )?;
    let (series_index, issue_amount, entries, add, sweep) = match action {
        ManageWriterDlmmV1Params::AddLiquidity {
            series_index,
            issue_amount_atoms,
            entries,
        } => (series_index, issue_amount_atoms, entries, true, false),
        ManageWriterDlmmV1Params::RemoveLiquidity {
            series_index,
            entries,
        } => (series_index, 0, entries, false, false),
        ManageWriterDlmmV1Params::SweepCash { series_index } => {
            (series_index, 0, Vec::new(), false, true)
        }
        _ => return Err(VaultError::InvalidInstructionData.into()),
    };
    let actor = &accounts[0];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let policy_info = &accounts[6];
    let market_info = &accounts[7];
    let month_info = &accounts[8];
    let pool_info = &accounts[9];
    let authority_info = &accounts[10];
    let position_info = &accounts[11];
    let option_mint_info = &accounts[12];
    let quote_mint_info = &accounts[13];
    let option_vault_info = &accounts[14];
    let quote_vault_info = &accounts[15];
    let sleeve_vault_info = &accounts[16];
    let staging_info = &accounts[17];
    let retirement_info = &accounts[18];
    let light_info = &accounts[19];
    let cpi_info = &accounts[20];
    let option_interface_info = &accounts[21];
    let quote_interface_info = &accounts[22];
    let token_info = &accounts[23];
    let system_info = &accounts[24];
    validate_writer_compression_accounts(
        light_info,
        cpi_info,
        token_info,
        system_info,
        &accounts[25],
        &accounts[26],
    )?;
    let config = load_canonical_vault_config(program_id, &accounts[1])?;
    let WriterPolicyContext {
        group,
        mut sleeve,
        mut book,
        snapshot,
    } = load_writer_policy_context(
        program_id,
        sleeve_info,
        group_info,
        book_info,
        &accounts[5],
        None,
    )?;
    let mut policy = load_policy(program_id, policy_info, sleeve_info, &snapshot, &book, true)?;
    let index = usize::from(series_index);
    if index >= usize::from(book.series_count)
        || book.records[index].market != *market_info.key
        || sleeve.vault_config != *accounts[1].key
        || sleeve.usdc_vault != *sleeve_vault_info.key
        || sleeve.policy_snapshot != *accounts[5].key
        || sleeve.settlement_mint != config.usdc_mint
        || config.usdc_mint != *quote_mint_info.key
        || sleeve.active_auction.is_some()
        || sleeve.status == WriterSleeveStatus::Closed
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let closing = sleeve.active_close_request.is_some()
        || matches!(
            sleeve.status,
            WriterSleeveStatus::CloseStaging
                | WriterSleeveStatus::Expired
                | WriterSleeveStatus::SettlementFinalized
        );
    if (!sweep && !closing && policy.management_authority != *actor.key) || (add && closing) {
        return Err(VaultError::Unauthorized.into());
    }
    let mut pool = load_writer_dlmm_pool(program_id, pool_info)?;
    let binding = load_collective_dlmm_context(
        program_id,
        sleeve_info,
        group_info,
        book_info,
        market_info,
        month_info,
    )?;
    validate_writer_dlmm_pool_binding(&config, market_info, month_info, &binding, &pool)?;
    let mut market = load_valid_market(program_id, market_info)?;
    let month = load_oracle_month_state(month_info, program_id)?;
    if add {
        if sleeve.status != WriterSleeveStatus::Active
            || group.status != WriterSettlementGroupStatus::Active
            || !pool.status.allows_liquidity_add()
            || binding.anchor_month_settled
            || current_unix_timestamp()? >= pool.expiry_ts
            || !issue_amount.is_multiple_of(MarketMintAccounting::CANONICAL_ATOMIC_SCALE)
            || issue_amount > snapshot.max_auction_issue_atoms
        {
            return Err(VaultError::InvalidWriterLifecycle.into());
        }
        ensure_market_value_flow_unpaused(&config, &market)?;
        ensure_oracle_game_window(&market, &month)?;
    } else if !pool.status.allows_liquidity_remove() {
        return Err(VaultError::InvalidAmoebaDlmmStatusTransition.into());
    }
    let mut position = load_position(
        program_id,
        position_info,
        pool_info.key,
        sleeve_info.key,
        policy_info.key,
        market_info.key,
        series_index,
    )?;
    if policy.series_pool_inventory_atoms[index] != position.option_inventory_atoms
        || policy.total_pool_quote_atoms
            < position
                .allocated_quote_atoms
                .checked_add(position.uncommitted_quote_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let mint_before =
        validate_canonical_market_mint(market_info, &mut market, option_mint_info, 0)?;
    validate_collateral_mint_account(quote_mint_info, token_info.key)?;
    validate_spl_interface_account(option_mint_info.key, option_interface_info)?;
    validate_spl_interface_account(quote_mint_info.key, quote_interface_info)?;
    let before_pool = writer_dlmm_vault_amounts(
        program_id,
        pool_info,
        &pool,
        authority_info,
        option_vault_info,
        quote_vault_info,
    )?;
    validate_vault_token_account(sleeve_vault_info, quote_mint_info.key, sleeve_info.key)?;
    let before_cash = validate_token_account(sleeve_vault_info)?.amount;
    let accounted_vault_cash = sleeve
        .accounted_asset_atoms
        .checked_sub(policy.total_pool_quote_atoms)
        .ok_or(VaultError::WriterSolvencyViolation)?;
    if before_cash < accounted_vault_cash
        || pool.option_mint != *option_mint_info.key
        || pool.quote_mint != *quote_mint_info.key
        || book.records[index].contract_mint != *option_mint_info.key
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let staged_before = auction::observe_market_staging_amount(
        program_id,
        market_info,
        staging_info,
        option_mint_info,
        token_info,
    )?;
    let retired_before = auction::observe_writer_retirement_custody_amount(
        program_id,
        sleeve_info,
        market_info,
        retirement_info,
        option_mint_info,
        token_info,
    )?;
    let observed_issuer = position
        .option_inventory_atoms
        .checked_add(staged_before)
        .and_then(|value| value.checked_add(retired_before))
        .ok_or(VaultError::ArithmeticOverflow)?;
    if mint_before.supply != book.records[index].total_physical_supply_atoms
        || observed_issuer != book.records[index].issuer_controlled_atoms
        || mint_before.supply.checked_sub(observed_issuer)
            != Some(book.records[index].external_open_interest_atoms)
        || market_outstanding_contract_amount(&market)?
            != book.records[index]
                .external_open_interest_atoms
                .checked_add(position.option_inventory_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let (option_amount, quote_amount) = if sweep {
        let quote = position.uncommitted_quote_atoms;
        position.uncommitted_quote_atoms = 0;
        policy.total_uncommitted_quote_atoms = policy
            .total_uncommitted_quote_atoms
            .checked_sub(quote)
            .ok_or(VaultError::ArithmeticOverflow)?;
        (0, quote)
    } else {
        apply_entries(&mut position, &entries, pool.maximum_bin_id, add)?
    };
    if add && option_amount != issue_amount {
        return Err(VaultError::InvalidInstructionData.into());
    }
    if add {
        let terms = &policy.series[index];
        let (minimum_ask, maximum_bid, _) = crate::writer_dlmm_math::writer_dlmm_price_bounds(
            terms.seller_floor_quote_atoms,
            pool.tick_size_quote_atomic,
            policy.price_separation_ticks,
            pool.swap_fee_bps,
            snapshot.primary_fee_bps,
        )
        .map_err(|_| VaultError::InvalidWriterPolicySnapshot)?;
        for entry in &entries {
            let price = u64::from(entry.bin_id)
                .checked_mul(pool.tick_size_quote_atomic)
                .ok_or(VaultError::ArithmeticOverflow)?;
            if entry.option_atoms > 0 && price < minimum_ask {
                return Err(VaultError::InvalidWriterPolicySnapshot.into());
            }
            if entry.quote_atoms > 0 {
                if price > maximum_bid {
                    return Err(VaultError::InvalidWriterPolicySnapshot.into());
                }
            }
        }
        if quote_amount > accounted_vault_cash {
            return Err(VaultError::WriterSolvencyViolation.into());
        }
        policy.total_pool_quote_atoms = policy
            .total_pool_quote_atoms
            .checked_add(quote_amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        pool.accounted_quote_reserve = pool
            .accounted_quote_reserve
            .checked_add(quote_amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        pool.accounted_option_reserve = pool
            .accounted_option_reserve
            .checked_add(option_amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        book.records[index].total_physical_supply_atoms = book.records[index]
            .total_physical_supply_atoms
            .checked_add(option_amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        book.records[index].issuer_controlled_atoms = book.records[index]
            .issuer_controlled_atoms
            .checked_add(option_amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        market.mint_accounting.total_issued = market
            .mint_accounting
            .total_issued
            .checked_add(option_amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
    } else {
        policy.total_pool_quote_atoms = policy
            .total_pool_quote_atoms
            .checked_sub(quote_amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        pool.accounted_quote_reserve = pool
            .accounted_quote_reserve
            .checked_sub(quote_amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        pool.accounted_option_reserve = pool
            .accounted_option_reserve
            .checked_sub(option_amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        book.records[index].total_physical_supply_atoms = book.records[index]
            .total_physical_supply_atoms
            .checked_sub(option_amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        book.records[index].issuer_controlled_atoms = book.records[index]
            .issuer_controlled_atoms
            .checked_sub(option_amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if option_amount > 0 {
            consume_market_contracts(&mut market, option_amount, true)?;
        }
    }
    policy.series_pool_inventory_atoms[index] = position.option_inventory_atoms;
    update_cash_metrics(&mut sleeve, &group, &book, &snapshot, &policy, add)?;
    let sleeve_bump = [sleeve.bump];
    let sleeve_seeds = writer_sleeve_signer_seeds(&sleeve.settlement_group, &sleeve_bump);
    let (_, authority_bump) = derive_ameba_dlmm_authority_pda(program_id, pool_info.key);
    let authority_bump_bytes = [authority_bump];
    let authority_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::AMOEBA_DLMM_AUTHORITY_PDA_SEED,
        pool_info.key.as_ref(),
        &authority_bump_bytes,
    ];
    if quote_amount > 0 {
        let (source, destination, authority, signers): (
            &AccountInfo,
            &AccountInfo,
            &AccountInfo,
            &[&[u8]],
        ) = if add {
            (
                sleeve_vault_info,
                quote_vault_info,
                sleeve_info,
                &sleeve_seeds,
            )
        } else {
            (
                quote_vault_info,
                sleeve_vault_info,
                authority_info,
                authority_seeds,
            )
        };
        invoke_light_token_account_transfer_with_signer_seeds(
            quote_amount,
            MarketMintAccounting::CANONICAL_DECIMALS,
            light_info,
            cpi_info,
            actor,
            source,
            destination,
            authority,
            quote_mint_info,
            quote_interface_info,
            token_info,
            system_info,
            &[signers],
        )?;
    }
    if option_amount > 0 {
        if add {
            let staging = auction::load_or_create_market_staging(
                program_id,
                actor,
                market_info,
                &market,
                staging_info,
                option_mint_info,
                token_info,
                system_info,
            )?;
            if staging.amount != staged_before {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            let market_bump = [market.bump];
            let market_seeds = auction::market_signer_seeds(&market, &market_bump);
            if staged_before > 0 {
                let retirement = load_or_create_writer_retirement_custody(
                    program_id,
                    actor,
                    sleeve_info,
                    market_info,
                    retirement_info,
                    option_mint_info,
                    token_info,
                    system_info,
                )?;
                if retirement.amount != retired_before {
                    return Err(VaultError::WriterSupplyMismatch.into());
                }
                invoke_token_transfer_checked(
                    token_info,
                    staging_info,
                    option_mint_info,
                    retirement_info,
                    market_info,
                    staged_before,
                    MarketMintAccounting::CANONICAL_DECIMALS,
                    &[&market_seeds],
                )?;
            }
            invoke_token_mint_to_checked(
                token_info,
                option_mint_info,
                staging_info,
                market_info,
                option_amount,
                MarketMintAccounting::CANONICAL_DECIMALS,
                &[&market_seeds],
            )?;
            invoke_light_token_account_transfer_with_signer_seeds(
                option_amount,
                MarketMintAccounting::CANONICAL_DECIMALS,
                light_info,
                cpi_info,
                actor,
                staging_info,
                option_vault_info,
                market_info,
                option_mint_info,
                option_interface_info,
                token_info,
                system_info,
                &[&market_seeds],
            )?;
            if validate_token_account(staging_info)?.amount != 0 {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            invoke_token_close_account(
                token_info,
                staging_info,
                actor,
                market_info,
                &[&market_seeds],
            )?;
        } else {
            let retirement = load_or_create_writer_retirement_custody(
                program_id,
                actor,
                sleeve_info,
                market_info,
                retirement_info,
                option_mint_info,
                token_info,
                system_info,
            )?;
            if retirement.amount != retired_before {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            invoke_light_token_account_transfer_with_signer_seeds(
                option_amount,
                MarketMintAccounting::CANONICAL_DECIMALS,
                light_info,
                cpi_info,
                actor,
                option_vault_info,
                retirement_info,
                authority_info,
                option_mint_info,
                option_interface_info,
                token_info,
                system_info,
                &[authority_seeds],
            )?;
            invoke_token_burn_checked(
                token_info,
                retirement_info,
                option_mint_info,
                sleeve_info,
                option_amount,
                MarketMintAccounting::CANONICAL_DECIMALS,
                &[&sleeve_seeds],
            )?;
            if validate_token_account(retirement_info)?.amount != retired_before {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            if retired_before == 0 {
                funding::close_sleeve_token_custody(
                    &sleeve,
                    sleeve_info,
                    retirement_info,
                    actor,
                    token_info,
                )?;
            }
        }
    }
    let after_pool = writer_dlmm_vault_amounts(
        program_id,
        pool_info,
        &pool,
        authority_info,
        option_vault_info,
        quote_vault_info,
    )?;
    let after_cash = validate_token_account(sleeve_vault_info)?.amount;
    let after_mint = validate_mint_account(option_mint_info, token_info.key)?;
    let expected_options = if add {
        before_pool.0.checked_add(option_amount)
    } else {
        before_pool.0.checked_sub(option_amount)
    };
    let expected_quote = if add {
        before_pool.1.checked_add(quote_amount)
    } else {
        before_pool.1.checked_sub(quote_amount)
    };
    let expected_cash = if add {
        before_cash.checked_sub(quote_amount)
    } else {
        before_cash.checked_add(quote_amount)
    };
    if Some(after_pool.0) != expected_options
        || Some(after_pool.1) != expected_quote
        || Some(after_cash) != expected_cash
        || after_mint.supply != book.records[index].total_physical_supply_atoms
        || market_outstanding_contract_amount(&market)?
            != book.records[index]
                .external_open_interest_atoms
                .checked_add(position.option_inventory_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    book.records[index].custody_status = if book.records[index].issuer_controlled_atoms == 0 {
        WriterSeriesCustodyStatus::Closed
    } else {
        WriterSeriesCustodyStatus::Open
    };
    let slot = Clock::get()?.slot;
    position.last_updated_slot = slot;
    pool.last_updated_slot = slot;
    sleeve.last_updated_slot = slot;
    book.last_updated_slot = slot;
    book.book_digest = writer_book_digest(&book);
    store_state(position_info, position.as_ref())?;
    store_state(policy_info, policy.as_ref())?;
    store_state(sleeve_info, sleeve.as_ref())?;
    store_state(book_info, book.as_ref())?;
    store_state(market_info, &market)?;
    store_writer_dlmm_pool(pool_info, &pool)
}
