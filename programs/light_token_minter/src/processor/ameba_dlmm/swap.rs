use super::*;

#[derive(Clone)]
pub(super) struct LoadedSwapPage {
    account_index: usize,
    page: AmoebaDlmmBinPageV1,
}

pub(super) fn find_swap_page(pages: &[LoadedSwapPage], page_index: u16) -> Option<usize> {
    pages
        .iter()
        .position(|loaded| loaded.page.page_index == page_index)
}

pub(super) fn refresh_swap_best_side(
    pool: &AmoebaDlmmPoolV1,
    pages: &[LoadedSwapPage],
    asks: bool,
) -> Result<u16, ProgramError> {
    let page_bitmap = if asks {
        &pool.ask_page_bitmap
    } else {
        &pool.bid_page_bitmap
    };
    let target_page = if asks {
        first_set_page(page_bitmap)
    } else {
        last_set_page(page_bitmap)
    };
    let Some(target_page) = target_page else {
        return Ok(AMOEBA_DLMM_EMPTY_BIN_ID);
    };
    if let Some(index) = find_swap_page(pages, target_page) {
        let page = &pages[index].page;
        let local = if asks {
            crate::ameba_dlmm_math::lowest_set_bit(page.ask_bitmap)
        } else {
            crate::ameba_dlmm_math::highest_set_bit(page.bid_bitmap)
        }
        .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
        return page
            .first_bin_id
            .checked_add(local as u16)
            .ok_or(VaultError::ArithmeticOverflow.into());
    }

    let prior = if asks {
        pool.best_ask_bin_id
    } else {
        pool.best_bid_bin_id
    };
    if prior != AMOEBA_DLMM_EMPTY_BIN_ID && bin_to_page(prior).map_err(math_error)?.0 == target_page
    {
        Ok(prior)
    } else {
        Err(VaultError::InvalidAmoebaDlmmRoute.into())
    }
}

#[inline(never)]
pub(super) fn load_swap_config(
    program_id: &Pubkey,
    config_info: &AccountInfo,
) -> Result<Box<VaultConfig>, ProgramError> {
    load_canonical_vault_config(program_id, config_info).map(Box::new)
}

#[inline(never)]
pub(super) fn load_swap_market(
    program_id: &Pubkey,
    market_info: &AccountInfo,
) -> Result<Box<Market>, ProgramError> {
    let market = load_valid_market(program_id, market_info)?;
    Ok(Box::new(market))
}

#[inline(never)]
pub(super) fn load_swap_pool(
    program_id: &Pubkey,
    pool_info: &AccountInfo,
) -> Result<Box<AmoebaDlmmPoolV1>, ProgramError> {
    load_pool(program_id, pool_info).map(Box::new)
}

#[inline(never)]
pub(super) fn process_collective_swap_exact_in_core(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SwapAmoebaDlmmExactInV1Params,
) -> ProgramResult {
    const FIXED_ACCOUNTS: usize = 20;
    if accounts.len() <= FIXED_ACCOUNTS {
        return Err(VaultError::InvalidAccountList.into());
    }
    let trader_info = &accounts[0];
    let config_info = &accounts[1];
    let market_info = &accounts[2];
    let month_info = &accounts[3];
    let pool_info = &accounts[4];
    let authority_info = &accounts[5];
    let option_mint_info = &accounts[6];
    let quote_mint_info = &accounts[7];
    let option_vault_info = &accounts[8];
    let quote_vault_info = &accounts[9];
    let trader_option_info = &accounts[10];
    let trader_quote_info = &accounts[11];
    let light_token_program_info = &accounts[12];
    let compressed_token_authority_info = &accounts[13];
    let option_interface_info = &accounts[14];
    let quote_interface_info = &accounts[15];
    let spl_token_program_info = &accounts[16];
    let system_program_info = &accounts[17];
    let compressible_config_info = &accounts[18];
    let rent_sponsor_info = &accounts[19];
    if *compressible_config_info.key != light_token_instruction::compressible_config()
        || *rent_sponsor_info.key != light_token_instruction::rent_sponsor()
        || !rent_sponsor_info.is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    if !trader_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !pool_info.is_writable {
        return Err(VaultError::AmoebaDlmmAccountNotHot.into());
    }
    assert_program_accounts(
        light_token_program_info,
        compressed_token_authority_info,
        spl_token_program_info,
        system_program_info,
    )?;
    let config = load_swap_config(program_id, config_info)?;
    let market = load_swap_market(program_id, market_info)?;
    let month = Box::new(load_oracle_month_state(month_info, program_id)?);
    let mut pool = load_swap_pool(program_id, pool_info)?;
    if pool.status != AmoebaDlmmPoolStatus::Active {
        return Err(VaultError::InvalidAmoebaDlmmStatusTransition.into());
    }
    ensure_market_value_flow_unpaused(&config, &market)?;
    ensure_oracle_game_window(&market, &month)?;
    if month.settlement_record.is_some() || month.settlement_status == OracleSettlementStatus::Final
    {
        return Err(VaultError::AmoebaDlmmMarketNotTradable.into());
    }
    let now = current_unix_timestamp()?;
    if now > params.deadline_ts {
        return Err(VaultError::AmoebaDlmmDeadlineElapsed.into());
    }
    if now >= pool.expiry_ts {
        return Err(VaultError::AmoebaDlmmMarketNotTradable.into());
    }
    if *option_mint_info.key != pool.option_mint || *quote_mint_info.key != pool.quote_mint {
        return Err(VaultError::InvalidMint.into());
    }
    validate_collateral_mint_account(option_mint_info, &spl_token_program_id())?;
    validate_collateral_mint_account(quote_mint_info, &spl_token_program_id())?;
    validate_spl_interface_account(option_mint_info.key, option_interface_info)?;
    validate_spl_interface_account(quote_mint_info.key, quote_interface_info)?;
    let direction = match params.direction {
        WireSwapDirection::QuoteForOption => AmoebaDlmmSwapDirection::QuoteForOption,
        WireSwapDirection::OptionForQuote => AmoebaDlmmSwapDirection::OptionForQuote,
    };
    let (input_user_info, input_mint_info, output_user_info, output_mint_info) = match direction {
        AmoebaDlmmSwapDirection::QuoteForOption => (
            trader_quote_info,
            quote_mint_info,
            trader_option_info,
            option_mint_info,
        ),
        AmoebaDlmmSwapDirection::OptionForQuote => (
            trader_option_info,
            option_mint_info,
            trader_quote_info,
            quote_mint_info,
        ),
    };
    let _ = load_user_transfer_account(input_user_info, trader_info.key, input_mint_info.key)?;
    if output_user_info.owner == &system_program::id() {
        if output_user_info.executable || output_user_info.data_len() != 0 {
            return Err(VaultError::InvalidLightTokenAccount.into());
        }
        validate_light_associated_token_address(
            trader_info.key,
            output_mint_info.key,
            output_user_info,
        )?;
    } else {
        let _ =
            load_user_transfer_account(output_user_info, trader_info.key, output_mint_info.key)?;
    }
    let before_vault_amounts = validate_pool_vault_amounts(
        program_id,
        pool_info,
        &pool,
        authority_info,
        option_vault_info,
        quote_vault_info,
    )?;

    let (route_bitmap, ascending) = match direction {
        AmoebaDlmmSwapDirection::QuoteForOption => (pool.ask_page_bitmap, true),
        AmoebaDlmmSwapDirection::OptionForQuote => (pool.bid_page_bitmap, false),
    };
    let mut expected_page = if ascending {
        first_set_page(&route_bitmap)
    } else {
        last_set_page(&route_bitmap)
    }
    .ok_or(VaultError::InsufficientAmoebaDlmmLiquidity)?;
    if accounts.len() - FIXED_ACCOUNTS > MAX_AMOEBA_DLMM_PAGE_HOPS_PER_SWAP as usize {
        return Err(VaultError::AmoebaDlmmRouteTooLarge.into());
    }
    let mut pages = Vec::with_capacity(accounts.len() - FIXED_ACCOUNTS);
    let mut bins = Vec::with_capacity(pool.maximum_bins_per_swap as usize);
    for (offset, page_info) in accounts[FIXED_ACCOUNTS..].iter().enumerate() {
        if !page_info.is_writable {
            return Err(VaultError::UnexpectedAmoebaDlmmWritableAccount.into());
        }
        let page = load_bin_page(program_id, pool_info.key, page_info)?;
        if !page_bit(&pool.initialized_page_bitmap, page.page_index)
            || page.page_index != expected_page
            || pages
                .iter()
                .any(|existing: &LoadedSwapPage| existing.page.page_index == page.page_index)
        {
            return Err(VaultError::InvalidAmoebaDlmmRoute.into());
        }
        let local_bitmap = if ascending {
            page.ask_bitmap
        } else {
            page.bid_bitmap
        };
        for local in 0..32usize {
            let local_index = if ascending { local } else { 31 - local };
            if local_bitmap & (1u32 << local_index) == 0 {
                continue;
            }
            let bin_id = page
                .first_bin_id
                .checked_add(local_index as u16)
                .ok_or(VaultError::ArithmeticOverflow)?;
            if bin_id > pool.maximum_bin_id {
                return Err(VaultError::InvalidAmoebaDlmmGrid.into());
            }
            if bins.len() < pool.maximum_bins_per_swap as usize {
                bins.push(AmoebaDlmmBinLiquidity {
                    bin_id,
                    option_reserve: page.option_reserve[local_index],
                    quote_reserve: page.quote_reserve[local_index],
                });
            }
        }
        pages.push(LoadedSwapPage {
            account_index: FIXED_ACCOUNTS + offset,
            page,
        });
        expected_page = match next_set_page(&route_bitmap, expected_page, ascending) {
            Some(next) => next,
            None => {
                if offset + 1 != accounts.len() - FIXED_ACCOUNTS {
                    return Err(VaultError::InvalidAmoebaDlmmRoute.into());
                }
                expected_page
            }
        };
    }
    let quote = quote_exact_in(
        direction,
        params.amount_in,
        params.minimum_amount_out,
        params.limit_bin_id,
        pool.tick_size_quote_atomic,
        pool.maximum_bin_id,
        pool.swap_fee_bps,
        pool.protocol_fee_share_bps,
        pool.maximum_bins_per_swap,
        &bins,
    )
    .map_err(math_error)?;

    for fill in &quote.fills {
        let (page_index, local_index) = bin_to_page(fill.bin_id).map_err(math_error)?;
        let loaded_index =
            find_swap_page(&pages, page_index).ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
        pages[loaded_index].page.option_reserve[local_index as usize] = fill.option_reserve_after;
        pages[loaded_index].page.quote_reserve[local_index as usize] = fill.quote_reserve_after;
    }
    let slot = Clock::get()?.slot;
    for loaded in &mut pages {
        (loaded.page.bid_bitmap, loaded.page.ask_bitmap) =
            refresh_local_liquidity_bits(&loaded.page.option_reserve, &loaded.page.quote_reserve);
        set_page_bit(
            &mut pool.bid_page_bitmap,
            loaded.page.page_index,
            loaded.page.bid_bitmap != 0,
        )?;
        set_page_bit(
            &mut pool.ask_page_bitmap,
            loaded.page.page_index,
            loaded.page.ask_bitmap != 0,
        )?;
        loaded.page.last_updated_slot = slot;
    }
    let (last_fill_page_index, _) = bin_to_page(quote.last_bin_id).map_err(math_error)?;
    let last_fill_page_offset =
        find_swap_page(&pages, last_fill_page_index).ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
    let last_fill_has_output_liquidity = if ascending {
        pages[last_fill_page_offset].page.ask_bitmap != 0
    } else {
        pages[last_fill_page_offset].page.bid_bitmap != 0
    };
    let next_output_page = next_set_page(&route_bitmap, last_fill_page_index, ascending);
    let required_page_count = last_fill_page_offset
        .checked_add(1)
        .and_then(|count| {
            count.checked_add(usize::from(
                !last_fill_has_output_liquidity && next_output_page.is_some(),
            ))
        })
        .ok_or(VaultError::ArithmeticOverflow)?;
    if pages.len() < required_page_count
        || (!last_fill_has_output_liquidity
            && next_output_page.is_some()
            && pages
                .get(last_fill_page_offset + 1)
                .is_none_or(|loaded| Some(loaded.page.page_index) != next_output_page))
    {
        return Err(VaultError::InvalidAmoebaDlmmRoute.into());
    }
    if pages.len() > required_page_count {
        return Err(VaultError::UnexpectedAmoebaDlmmWritableAccount.into());
    }
    let (input_reserve, output_reserve) = match direction {
        AmoebaDlmmSwapDirection::QuoteForOption => (
            &mut pool.accounted_quote_reserve,
            &mut pool.accounted_option_reserve,
        ),
        AmoebaDlmmSwapDirection::OptionForQuote => (
            &mut pool.accounted_option_reserve,
            &mut pool.accounted_quote_reserve,
        ),
    };
    let net_input = quote
        .amount_in
        .checked_sub(quote.protocol_fee)
        .ok_or(VaultError::ArithmeticOverflow)?;
    *output_reserve = output_reserve
        .checked_sub(quote.amount_out)
        .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
    *input_reserve = input_reserve
        .checked_add(net_input)
        .ok_or(VaultError::ArithmeticOverflow)?;
    match direction {
        AmoebaDlmmSwapDirection::QuoteForOption => {
            pool.protocol_fee_quote = pool
                .protocol_fee_quote
                .checked_add(quote.protocol_fee)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
        AmoebaDlmmSwapDirection::OptionForQuote => {
            pool.protocol_fee_option = pool
                .protocol_fee_option
                .checked_add(quote.protocol_fee)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
    }
    pool.best_ask_bin_id = refresh_swap_best_side(&pool, &pages, true)?;
    pool.best_bid_bin_id = refresh_swap_best_side(&pool, &pages, false)?;
    pool.last_trade_bin_id = quote.last_bin_id;
    pool.last_updated_slot = slot;

    let (_, authority_bump) = derive_ameba_dlmm_authority_pda(program_id, pool_info.key);
    let authority_bump_bytes = [authority_bump];
    let authority_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        AMOEBA_DLMM_AUTHORITY_PDA_SEED,
        pool_info.key.as_ref(),
        &authority_bump_bytes,
    ];
    let signers: &[&[&[u8]]] = &[authority_seeds];
    let (
        input_user,
        input_vault,
        input_mint,
        input_interface,
        output_vault,
        output_user,
        output_mint,
        output_interface,
    ) = match direction {
        AmoebaDlmmSwapDirection::QuoteForOption => (
            trader_quote_info,
            quote_vault_info,
            quote_mint_info,
            quote_interface_info,
            option_vault_info,
            trader_option_info,
            option_mint_info,
            option_interface_info,
        ),
        AmoebaDlmmSwapDirection::OptionForQuote => (
            trader_option_info,
            option_vault_info,
            option_mint_info,
            option_interface_info,
            quote_vault_info,
            trader_quote_info,
            quote_mint_info,
            quote_interface_info,
        ),
    };
    if output_user.owner == &system_program::id() {
        let _ = load_or_create_light_associated_token_account(
            trader_info,
            trader_info,
            output_mint,
            output_user,
            light_token_program_info,
            compressible_config_info,
            rent_sponsor_info,
            system_program_info,
        )?;
    }
    invoke_light_token_account_transfer(
        quote.amount_in,
        MarketMintAccounting::CANONICAL_DECIMALS,
        light_token_program_info,
        compressed_token_authority_info,
        trader_info,
        input_user,
        input_vault,
        trader_info,
        input_mint,
        input_interface,
        spl_token_program_info,
        system_program_info,
    )?;
    invoke_light_token_account_transfer_with_signer_seeds(
        quote.amount_out,
        MarketMintAccounting::CANONICAL_DECIMALS,
        light_token_program_info,
        compressed_token_authority_info,
        trader_info,
        output_vault,
        output_user,
        authority_info,
        output_mint,
        output_interface,
        spl_token_program_info,
        system_program_info,
        signers,
    )?;
    let after_vault_amounts = validate_pool_vault_amounts(
        program_id,
        pool_info,
        &pool,
        authority_info,
        option_vault_info,
        quote_vault_info,
    )?;
    let (before_input, before_output, after_input, after_output) = match direction {
        AmoebaDlmmSwapDirection::QuoteForOption => (
            before_vault_amounts.1,
            before_vault_amounts.0,
            after_vault_amounts.1,
            after_vault_amounts.0,
        ),
        AmoebaDlmmSwapDirection::OptionForQuote => (
            before_vault_amounts.0,
            before_vault_amounts.1,
            after_vault_amounts.0,
            after_vault_amounts.1,
        ),
    };
    let physical_delta_ok = after_input
        == before_input
            .checked_add(quote.amount_in)
            .ok_or(VaultError::ArithmeticOverflow)?
        && before_output
            == after_output
                .checked_add(quote.amount_out)
                .ok_or(VaultError::ArithmeticOverflow)?;
    if !physical_delta_ok {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    for loaded in &pages {
        store_light_state(&accounts[loaded.account_index], &loaded.page)?;
    }
    store_light_state(pool_info, pool.as_ref())?;
    emit_event(
        &EVENT_SWAP_EXECUTED,
        AmoebaDlmmEvent::Swap(SwapEvent {
            pool: *pool_info.key,
            trader: *trader_info.key,
            direction: direction as u8,
            amount_in: quote.amount_in,
            amount_out: quote.amount_out,
            total_fee: quote.total_fee,
            protocol_fee: quote.protocol_fee,
            first_bin: quote.first_bin_id,
            last_bin: quote.last_bin_id,
            bins_crossed: quote.fills.len() as u8,
            slot,
        }),
    )
}
