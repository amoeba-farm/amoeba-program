use super::*;

#[derive(Clone)]
pub(super) struct LoadedPagePair {
    pub(super) page_account_index: usize,
    pub(super) share_account_index: usize,
    pub(super) page: AmoebaDlmmBinPageV1,
    pub(super) shares: AmoebaDlmmSharePageV1,
}

pub(super) fn position_bin_index(
    position: &AmoebaDlmmPositionV1,
    bin_id: u16,
) -> Result<usize, ProgramError> {
    let offset = bin_id
        .checked_sub(position.lower_bin_id)
        .ok_or(VaultError::InvalidAmoebaDlmmPosition)?;
    if offset >= position.bin_count as u16 || offset >= 32 {
        return Err(VaultError::InvalidAmoebaDlmmPosition.into());
    }
    Ok(offset as usize)
}

pub(super) fn load_page_pairs(
    program_id: &Pubkey,
    pool: &Pubkey,
    initialized_page_bitmap: &u64,
    accounts: &[AccountInfo],
    first_account_index: usize,
) -> Result<Vec<LoadedPagePair>, ProgramError> {
    let tail = accounts
        .get(first_account_index..)
        .ok_or(VaultError::InvalidAccountList)?;
    if tail.is_empty() || !tail.len().is_multiple_of(2) {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut result = Vec::with_capacity(tail.len() / 2);
    let mut previous_page = None;
    for (pair_index, pair) in tail.chunks_exact(2).enumerate() {
        if !pair[0].is_writable || !pair[1].is_writable {
            return Err(VaultError::AmoebaDlmmAccountNotHot.into());
        }
        let page = load_bin_page(program_id, pool, &pair[0])?;
        let shares = load_share_page(program_id, pool, &pair[1])?;
        if shares.page_index != page.page_index || shares.first_bin_id != page.first_bin_id {
            return Err(VaultError::InvalidAmoebaDlmmSharePage.into());
        }
        if !page_bit(initialized_page_bitmap, page.page_index) {
            return Err(VaultError::InvalidAmoebaDlmmBinPage.into());
        }
        if previous_page.is_some_and(|previous| previous >= page.page_index) {
            return Err(VaultError::UnorderedAmoebaDlmmPage.into());
        }
        previous_page = Some(page.page_index);
        result.push(LoadedPagePair {
            page_account_index: first_account_index + pair_index * 2,
            share_account_index: first_account_index + pair_index * 2 + 1,
            page,
            shares,
        });
    }
    Ok(result)
}

pub(super) fn validate_exact_liquidity_page_route(
    pages: &[LoadedPagePair],
    expected_page_indexes: &[u16],
) -> ProgramResult {
    if pages.len() < expected_page_indexes.len() {
        return Err(VaultError::InvalidAccountList.into());
    }
    if pages.len() > expected_page_indexes.len() {
        return Err(VaultError::UnexpectedAmoebaDlmmWritableAccount.into());
    }
    if pages
        .iter()
        .zip(expected_page_indexes)
        .any(|(loaded, expected)| loaded.page.page_index != *expected)
    {
        return Err(VaultError::InvalidAmoebaDlmmRoute.into());
    }
    Ok(())
}

pub(super) fn validate_liquidity_page_route_contains_touched(
    pages: &[LoadedPagePair],
    touched_page_indexes: &[u16],
) -> ProgramResult {
    if pages.len() < touched_page_indexes.len()
        || touched_page_indexes.iter().any(|page_index| {
            pages
                .binary_search_by_key(page_index, |loaded| loaded.page.page_index)
                .is_err()
        })
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    if pages.len() > touched_page_indexes.len().saturating_add(2) {
        return Err(VaultError::UnexpectedAmoebaDlmmWritableAccount.into());
    }
    Ok(())
}

pub(super) fn validate_final_liquidity_page_route(
    pages: &[LoadedPagePair],
    touched_page_indexes: &[u16],
    prior_best_ask: u16,
    prior_best_bid: u16,
    pool: &AmoebaDlmmPoolV1,
) -> ProgramResult {
    let mut required = touched_page_indexes.to_vec();
    for (prior, current) in [
        (prior_best_ask, pool.best_ask_bin_id),
        (prior_best_bid, pool.best_bid_bin_id),
    ] {
        if current == AMOEBA_DLMM_EMPTY_BIN_ID {
            continue;
        }
        let current_page = bin_to_page(current).map_err(math_error)?.0;
        let prior_page = (prior != AMOEBA_DLMM_EMPTY_BIN_ID)
            .then(|| bin_to_page(prior).ok().map(|value| value.0))
            .flatten();
        if prior_page != Some(current_page) {
            match required.binary_search(&current_page) {
                Ok(_) => {}
                Err(insert_at) => required.insert(insert_at, current_page),
            }
        }
    }
    validate_exact_liquidity_page_route(pages, &required)
}

pub(super) fn page_pair_index(
    pages: &[LoadedPagePair],
    page_index: u16,
) -> Result<usize, ProgramError> {
    pages
        .binary_search_by_key(&page_index, |page| page.page.page_index)
        .map_err(|_| VaultError::InvalidAccountList.into())
}

pub(super) fn refresh_pool_page_and_best_bits(
    pool: &mut AmoebaDlmmPoolV1,
    pages: &[LoadedPagePair],
) -> ProgramResult {
    let prior_best_ask = pool.best_ask_bin_id;
    let prior_best_bid = pool.best_bid_bin_id;
    for loaded in pages {
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
    }

    pool.best_ask_bin_id = match first_set_page(&pool.ask_page_bitmap) {
        None => AMOEBA_DLMM_EMPTY_BIN_ID,
        Some(page_index) => {
            if let Ok(index) = page_pair_index(pages, page_index) {
                let loaded = &pages[index];
                let local = crate::ameba_dlmm_math::lowest_set_bit(loaded.page.ask_bitmap)
                    .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
                loaded
                    .page
                    .first_bin_id
                    .checked_add(local as u16)
                    .ok_or(VaultError::ArithmeticOverflow)?
            } else if prior_best_ask != AMOEBA_DLMM_EMPTY_BIN_ID
                && bin_to_page(prior_best_ask).map_err(math_error)?.0 == page_index
            {
                prior_best_ask
            } else {
                return Err(VaultError::InvalidAmoebaDlmmRoute.into());
            }
        }
    };
    pool.best_bid_bin_id = match last_set_page(&pool.bid_page_bitmap) {
        None => AMOEBA_DLMM_EMPTY_BIN_ID,
        Some(page_index) => {
            if let Ok(index) = page_pair_index(pages, page_index) {
                let loaded = &pages[index];
                let local = crate::ameba_dlmm_math::highest_set_bit(loaded.page.bid_bitmap)
                    .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
                loaded
                    .page
                    .first_bin_id
                    .checked_add(local as u16)
                    .ok_or(VaultError::ArithmeticOverflow)?
            } else if prior_best_bid != AMOEBA_DLMM_EMPTY_BIN_ID
                && bin_to_page(prior_best_bid).map_err(math_error)?.0 == page_index
            {
                prior_best_bid
            } else {
                return Err(VaultError::InvalidAmoebaDlmmRoute.into());
            }
        }
    };
    Ok(())
}

pub(super) fn validate_position_owner_and_nonce(
    owner_info: &AccountInfo,
    pool: &AmoebaDlmmPoolV1,
    position: &AmoebaDlmmPositionV1,
    nonce: u64,
) -> ProgramResult {
    if !owner_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *owner_info.key != pool.liquidity_manager
        || position.owner != *owner_info.key
        || position.position_nonce != nonce
    {
        return Err(VaultError::UnauthorizedAmoebaDlmmManager.into());
    }
    Ok(())
}

pub(super) fn validate_liquidity_fixed_accounts(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    pool_info: &AccountInfo,
    pool: &AmoebaDlmmPoolV1,
) -> Result<(TokenAccount, TokenAccount), ProgramError> {
    let authority_info = &accounts[3];
    let option_mint_info = &accounts[4];
    let quote_mint_info = &accounts[5];
    let option_vault_info = &accounts[6];
    let quote_vault_info = &accounts[7];
    let owner_option_info = &accounts[8];
    let owner_quote_info = &accounts[9];
    let light_token_program_info = &accounts[10];
    let compressed_token_authority_info = &accounts[11];
    let option_interface_info = &accounts[12];
    let quote_interface_info = &accounts[13];
    let spl_token_program_info = &accounts[14];
    let system_program_info = &accounts[15];
    assert_program_accounts(
        light_token_program_info,
        compressed_token_authority_info,
        spl_token_program_info,
        system_program_info,
    )?;
    if *option_mint_info.key != pool.option_mint || *quote_mint_info.key != pool.quote_mint {
        return Err(VaultError::InvalidMint.into());
    }
    validate_collateral_mint_account(option_mint_info, &spl_token_program_id())?;
    validate_collateral_mint_account(quote_mint_info, &spl_token_program_id())?;
    validate_spl_interface_account(option_mint_info.key, option_interface_info)?;
    validate_spl_interface_account(quote_mint_info.key, quote_interface_info)?;
    let _ = load_user_transfer_account(
        owner_option_info,
        &pool.liquidity_manager,
        option_mint_info.key,
    )?;
    let _ = load_user_transfer_account(
        owner_quote_info,
        &pool.liquidity_manager,
        quote_mint_info.key,
    )?;
    let vaults = validate_pool_vaults(
        program_id,
        pool_info,
        pool,
        authority_info,
        option_vault_info,
        quote_vault_info,
    )?;
    ensure_custody(pool, &vaults.0, &vaults.1)?;
    Ok(vaults)
}

pub(super) fn transfer_owner_to_pool(
    accounts: &[AccountInfo],
    option_amount: u64,
    quote_amount: u64,
) -> ProgramResult {
    if option_amount > 0 {
        invoke_light_token_account_transfer(
            option_amount,
            MarketMintAccounting::CANONICAL_DECIMALS,
            &accounts[10],
            &accounts[11],
            &accounts[0],
            &accounts[8],
            &accounts[6],
            &accounts[0],
            &accounts[4],
            &accounts[12],
            &accounts[14],
            &accounts[15],
        )?;
    }
    if quote_amount > 0 {
        invoke_light_token_account_transfer(
            quote_amount,
            MarketMintAccounting::CANONICAL_DECIMALS,
            &accounts[10],
            &accounts[11],
            &accounts[0],
            &accounts[9],
            &accounts[7],
            &accounts[0],
            &accounts[5],
            &accounts[13],
            &accounts[14],
            &accounts[15],
        )?;
    }
    Ok(())
}

pub(super) fn transfer_pool_to_owner(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    option_amount: u64,
    quote_amount: u64,
) -> ProgramResult {
    let (_, authority_bump) = derive_ameba_dlmm_authority_pda(program_id, accounts[1].key);
    let authority_bump_bytes = [authority_bump];
    let authority_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        AMOEBA_DLMM_AUTHORITY_PDA_SEED,
        accounts[1].key.as_ref(),
        &authority_bump_bytes,
    ];
    let signers: &[&[&[u8]]] = &[authority_seeds];
    if option_amount > 0 {
        invoke_light_token_account_transfer_with_signer_seeds(
            option_amount,
            MarketMintAccounting::CANONICAL_DECIMALS,
            &accounts[10],
            &accounts[11],
            &accounts[0],
            &accounts[6],
            &accounts[8],
            &accounts[3],
            &accounts[4],
            &accounts[12],
            &accounts[14],
            &accounts[15],
            signers,
        )?;
    }
    if quote_amount > 0 {
        invoke_light_token_account_transfer_with_signer_seeds(
            quote_amount,
            MarketMintAccounting::CANONICAL_DECIMALS,
            &accounts[10],
            &accounts[11],
            &accounts[0],
            &accounts[7],
            &accounts[9],
            &accounts[3],
            &accounts[5],
            &accounts[13],
            &accounts[14],
            &accounts[15],
            signers,
        )?;
    }
    Ok(())
}

pub(super) fn store_page_pairs(
    accounts: &[AccountInfo],
    pages: &[LoadedPagePair],
) -> ProgramResult {
    for loaded in pages {
        store_light_state(&accounts[loaded.page_account_index], &loaded.page)?;
        store_light_state(&accounts[loaded.share_account_index], &loaded.shares)?;
    }
    Ok(())
}

pub(super) enum LiquidityChange {
    Add(AddAmoebaDlmmLiquidityV1Params),
    Remove(RemoveAmoebaDlmmLiquidityV1Params),
}

#[inline(never)]
pub(super) fn process_liquidity_change(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    change: LiquidityChange,
) -> ProgramResult {
    if accounts.len() < 18 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let owner_info = &accounts[0];
    let pool_info = &accounts[1];
    let position_info = &accounts[2];
    if !pool_info.is_writable || !position_info.is_writable {
        return Err(VaultError::AmoebaDlmmAccountNotHot.into());
    }
    let mut pool = load_pool(program_id, pool_info)?;
    let mut position = load_position(program_id, pool_info.key, position_info)?;
    let (position_nonce, entry_count, is_add) = match &change {
        LiquidityChange::Add(params) => (params.position_nonce, params.entries.len(), true),
        LiquidityChange::Remove(params) => (params.position_nonce, params.entries.len(), false),
    };
    validate_position_owner_and_nonce(owner_info, &pool, &position, position_nonce)?;
    if (is_add && !pool.status.allows_liquidity_add())
        || (!is_add && !pool.status.allows_liquidity_remove())
    {
        return Err(VaultError::InvalidAmoebaDlmmStatusTransition.into());
    }
    let before_vaults = validate_liquidity_fixed_accounts(program_id, accounts, pool_info, &pool)?;
    let mut pages = load_page_pairs(
        program_id,
        pool_info.key,
        &pool.initialized_page_bitmap,
        accounts,
        16,
    )?;
    let mut expected_page_indexes = Vec::with_capacity(entry_count);
    for index in 0..entry_count {
        let bin_id = match &change {
            LiquidityChange::Add(params) => params.entries[index].bin_id,
            LiquidityChange::Remove(params) => params.entries[index].bin_id,
        };
        let page_index = bin_to_page(bin_id).map_err(math_error)?.0;
        if expected_page_indexes.last() != Some(&page_index) {
            expected_page_indexes.push(page_index);
        }
    }
    validate_liquidity_page_route_contains_touched(&pages, &expected_page_indexes)?;
    let mut total_option = 0u64;
    let mut total_quote = 0u64;
    let mut total_shares = 0u128;
    let slot = Clock::get()?.slot;

    for index in 0..entry_count {
        let bin_id = match &change {
            LiquidityChange::Add(params) => params.entries[index].bin_id,
            LiquidityChange::Remove(params) => params.entries[index].bin_id,
        };
        if is_add && (bin_id == 0 || bin_id > pool.maximum_bin_id) {
            return Err(VaultError::InvalidAmoebaDlmmGrid.into());
        }
        let position_index = position_bin_index(&position, bin_id)?;
        let (page_index, local_index) = bin_to_page(bin_id).map_err(math_error)?;
        let loaded_index = page_pair_index(&pages, page_index)?;
        let loaded = &mut pages[loaded_index];
        let local = local_index as usize;
        let (option_amount, quote_amount, shares) = match &change {
            LiquidityChange::Add(params) => {
                let entry = &params.entries[index];
                let price = price_from_bin(
                    pool.tick_size_quote_atomic,
                    pool.maximum_bin_id,
                    entry.bin_id,
                )
                .map_err(math_error)?;
                let deposit = calculate_share_deposit(
                    loaded.page.option_reserve[local],
                    loaded.page.quote_reserve[local],
                    loaded.shares.total_liquidity_shares[local],
                    entry.maximum_option_amount,
                    entry.maximum_quote_amount,
                    price,
                )
                .map_err(math_error)?;
                if deposit.minted_shares < entry.minimum_shares {
                    return Err(VaultError::AmoebaDlmmSlippageExceeded.into());
                }
                loaded.page.option_reserve[local] = loaded.page.option_reserve[local]
                    .checked_add(deposit.option_amount)
                    .ok_or(VaultError::ArithmeticOverflow)?;
                loaded.page.quote_reserve[local] = loaded.page.quote_reserve[local]
                    .checked_add(deposit.quote_amount)
                    .ok_or(VaultError::ArithmeticOverflow)?;
                loaded.shares.total_liquidity_shares[local] = loaded.shares.total_liquidity_shares
                    [local]
                    .checked_add(deposit.minted_shares)
                    .ok_or(VaultError::ArithmeticOverflow)?;
                position.liquidity_shares[position_index] = position.liquidity_shares
                    [position_index]
                    .checked_add(deposit.minted_shares)
                    .ok_or(VaultError::ArithmeticOverflow)?;
                position.initialized_bitmap |= 1u32 << position_index;
                (
                    deposit.option_amount,
                    deposit.quote_amount,
                    deposit.minted_shares,
                )
            }
            LiquidityChange::Remove(params) => {
                let entry = &params.entries[index];
                if entry.shares == 0 || entry.shares > position.liquidity_shares[position_index] {
                    return Err(VaultError::AmoebaDlmmInvariantViolation.into());
                }
                let withdrawal = calculate_share_withdrawal(
                    loaded.page.option_reserve[local],
                    loaded.page.quote_reserve[local],
                    loaded.shares.total_liquidity_shares[local],
                    entry.shares,
                )
                .map_err(math_error)?;
                if withdrawal.option_amount < entry.minimum_option_out
                    || withdrawal.quote_amount < entry.minimum_quote_out
                {
                    return Err(VaultError::AmoebaDlmmSlippageExceeded.into());
                }
                loaded.page.option_reserve[local] = loaded.page.option_reserve[local]
                    .checked_sub(withdrawal.option_amount)
                    .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
                loaded.page.quote_reserve[local] = loaded.page.quote_reserve[local]
                    .checked_sub(withdrawal.quote_amount)
                    .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
                loaded.shares.total_liquidity_shares[local] = loaded.shares.total_liquidity_shares
                    [local]
                    .checked_sub(entry.shares)
                    .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
                position.liquidity_shares[position_index] = position.liquidity_shares
                    [position_index]
                    .checked_sub(entry.shares)
                    .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
                if position.liquidity_shares[position_index] == 0 {
                    position.initialized_bitmap &= !(1u32 << position_index);
                }
                (
                    withdrawal.option_amount,
                    withdrawal.quote_amount,
                    entry.shares,
                )
            }
        };
        total_option = total_option
            .checked_add(option_amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        total_quote = total_quote
            .checked_add(quote_amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        total_shares = total_shares
            .checked_add(shares)
            .ok_or(VaultError::ArithmeticOverflow)?;
    }

    for loaded in &mut pages {
        if !is_add {
            for index in 0..32 {
                if loaded.shares.total_liquidity_shares[index] == 0
                    && (loaded.page.option_reserve[index] != 0
                        || loaded.page.quote_reserve[index] != 0)
                {
                    return Err(VaultError::AmoebaDlmmInvariantViolation.into());
                }
            }
        }
        (loaded.page.bid_bitmap, loaded.page.ask_bitmap) =
            refresh_local_liquidity_bits(&loaded.page.option_reserve, &loaded.page.quote_reserve);
        loaded.page.last_updated_slot = slot;
        loaded.shares.last_updated_slot = slot;
    }
    if is_add {
        pool.accounted_option_reserve = pool
            .accounted_option_reserve
            .checked_add(total_option)
            .ok_or(VaultError::ArithmeticOverflow)?;
        pool.accounted_quote_reserve = pool
            .accounted_quote_reserve
            .checked_add(total_quote)
            .ok_or(VaultError::ArithmeticOverflow)?;
    } else {
        pool.accounted_option_reserve = pool
            .accounted_option_reserve
            .checked_sub(total_option)
            .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
        pool.accounted_quote_reserve = pool
            .accounted_quote_reserve
            .checked_sub(total_quote)
            .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
    }
    let prior_best_ask = pool.best_ask_bin_id;
    let prior_best_bid = pool.best_bid_bin_id;
    refresh_pool_page_and_best_bits(&mut pool, &pages)?;
    validate_final_liquidity_page_route(
        &pages,
        &expected_page_indexes,
        prior_best_ask,
        prior_best_bid,
        &pool,
    )?;
    pool.last_updated_slot = slot;
    position.last_updated_slot = slot;

    if let LiquidityChange::Remove(params) = &change {
        let close_position = params.close_position_when_empty && position.is_empty();
        if params.close_position_when_empty && !close_position {
            return Err(VaultError::InvalidAmoebaDlmmPosition.into());
        }
        if close_position {
            position.is_initialized = false;
            pool.position_count = pool
                .position_count
                .checked_sub(1)
                .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
        }
    }

    if is_add {
        transfer_owner_to_pool(accounts, total_option, total_quote)?;
    } else {
        transfer_pool_to_owner(program_id, accounts, total_option, total_quote)?;
    }
    let after_vaults = validate_pool_vaults(
        program_id,
        pool_info,
        &pool,
        &accounts[3],
        &accounts[6],
        &accounts[7],
    )?;
    let physical_delta_ok = if is_add {
        after_vaults.0.amount
            == before_vaults
                .0
                .amount
                .checked_add(total_option)
                .ok_or(VaultError::ArithmeticOverflow)?
            && after_vaults.1.amount
                == before_vaults
                    .1
                    .amount
                    .checked_add(total_quote)
                    .ok_or(VaultError::ArithmeticOverflow)?
    } else {
        before_vaults.0.amount
            == after_vaults
                .0
                .amount
                .checked_add(total_option)
                .ok_or(VaultError::ArithmeticOverflow)?
            && before_vaults.1.amount
                == after_vaults
                    .1
                    .amount
                    .checked_add(total_quote)
                    .ok_or(VaultError::ArithmeticOverflow)?
    };
    if !physical_delta_ok {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    ensure_custody(&pool, &after_vaults.0, &after_vaults.1)?;
    store_page_pairs(accounts, &pages)?;
    store_light_state(position_info, &position)?;
    store_light_state(pool_info, &pool)?;
    let event_discriminator = if is_add {
        &EVENT_LIQUIDITY_ADDED
    } else {
        &EVENT_LIQUIDITY_REMOVED
    };
    emit_event(
        event_discriminator,
        AmoebaDlmmEvent::Liquidity(LiquidityEvent {
            pool: *pool_info.key,
            position: *position_info.key,
            owner: *owner_info.key,
            option_amount: total_option,
            quote_amount: total_quote,
            shares: total_shares,
            entry_count: entry_count as u8,
            slot,
        }),
    )
}
