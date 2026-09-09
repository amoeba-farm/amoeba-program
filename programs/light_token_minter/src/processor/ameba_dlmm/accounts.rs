use super::*;

pub(super) fn load_light_state<T: AmoebaDlmmLightState>(
    account_info: &AccountInfo,
    program_id: &Pubkey,
    discriminator: &[u8; 8],
    invalid_error: VaultError,
) -> Result<T, ProgramError> {
    if account_info.owner != program_id
        || account_info.data_len() != T::ACCOUNT_LEN
        || !account_info.is_writable
    {
        return Err(invalid_error.into());
    }
    let data = account_info.try_borrow_data()?;
    if data.get(..8) != Some(discriminator.as_slice()) {
        return Err(invalid_error.into());
    }
    // SAFETY: the exact account-length check above covers the codec's rigid body.
    unsafe { T::decode_fixed(&data[8..]) }.map_err(|_| invalid_error.into())
}

pub(super) fn store_light_state<T: AmoebaDlmmLightState>(
    account_info: &AccountInfo,
    value: &T,
) -> ProgramResult {
    if value.maximum_encoded_len() + 8 != T::ACCOUNT_LEN
        || T::REQUIRED_DATA_LEN + 8 != T::ACCOUNT_LEN
    {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let mut data = account_info.try_borrow_mut_data()?;
    if data.len() != T::ACCOUNT_LEN {
        return Err(VaultError::InvalidInstructionData.into());
    }
    data[..8].copy_from_slice(&T::LIGHT_DISCRIMINATOR);
    value.encode_fixed(&mut data[8..]);
    Ok(())
}

pub(super) fn load_pool(
    program_id: &Pubkey,
    pool_info: &AccountInfo,
) -> Result<AmoebaDlmmPoolV1, ProgramError> {
    let pool: AmoebaDlmmPoolV1 = load_light_state(
        pool_info,
        program_id,
        &AMOEBA_DLMM_POOL_LIGHT_DISCRIMINATOR,
        VaultError::InvalidAmoebaDlmmPool,
    )?;
    let (expected, bump) = derive_ameba_dlmm_pool_pda(program_id, &pool.market);
    let grid_matches = pool
        .tick_size_quote_atomic
        .checked_mul(pool.maximum_bin_id as u64)
        == Some(pool.maximum_price_quote_atomic);
    let maximum_page = (pool.maximum_bin_id > 0)
        .then(|| bin_to_page(pool.maximum_bin_id).ok().map(|value| value.0))
        .flatten();
    let bitmaps_are_bounded = maximum_page.is_some_and(|maximum_page| {
        let retained_bits = maximum_page as u32 + 1;
        let allowed = if retained_bits == 64 {
            u64::MAX
        } else {
            (1u64 << retained_bits) - 1
        };
        pool.initialized_page_bitmap & !allowed == 0
    });
    let page_bitmaps_are_consistent =
        (pool.bid_page_bitmap | pool.ask_page_bitmap) & !pool.initialized_page_bitmap == 0;
    let best_ask_matches = match first_set_page(&pool.ask_page_bitmap) {
        None => pool.best_ask_bin_id == AMOEBA_DLMM_EMPTY_BIN_ID,
        Some(page_index) => {
            pool.best_ask_bin_id != AMOEBA_DLMM_EMPTY_BIN_ID
                && pool.best_ask_bin_id <= pool.maximum_bin_id
                && bin_to_page(pool.best_ask_bin_id).is_ok_and(|location| location.0 == page_index)
        }
    };
    let best_bid_matches = match last_set_page(&pool.bid_page_bitmap) {
        None => pool.best_bid_bin_id == AMOEBA_DLMM_EMPTY_BIN_ID,
        Some(page_index) => {
            pool.best_bid_bin_id != AMOEBA_DLMM_EMPTY_BIN_ID
                && pool.best_bid_bin_id <= pool.maximum_bin_id
                && bin_to_page(pool.best_bid_bin_id).is_ok_and(|location| location.0 == page_index)
        }
    };
    if *pool_info.key != expected
        || pool.bump != bump
        || !pool.has_current_layout()
        || crate::pubkey_is_default(&pool.market)
        || crate::pubkey_is_default(&pool.oracle_month)
        || crate::pubkey_is_default(&pool.liquidity_manager)
        || crate::pubkey_is_default(&pool.option_mint)
        || crate::pubkey_is_default(&pool.quote_mint)
        || pool.option_mint == pool.quote_mint
        || crate::pubkey_is_default(&pool.option_vault)
        || crate::pubkey_is_default(&pool.quote_vault)
        || pool.option_vault == pool.quote_vault
        || pool.expiry_ts == 0
        || pool.tick_size_quote_atomic == 0
        || pool.maximum_price_quote_atomic == 0
        || pool.maximum_bin_id == 0
        || pool.maximum_bin_id > MAX_AMOEBA_DLMM_BIN_COUNT
        || !grid_matches
        || !(1..=MAX_AMOEBA_DLMM_BINS_PER_SWAP).contains(&pool.maximum_bins_per_swap)
        || pool.swap_fee_bps > MAX_AMOEBA_DLMM_SWAP_FEE_BPS
        || pool.protocol_fee_share_bps > 10_000
        || !bitmaps_are_bounded
        || !page_bitmaps_are_consistent
        || !best_ask_matches
        || !best_bid_matches
        || (pool.last_trade_bin_id != AMOEBA_DLMM_EMPTY_BIN_ID
            && pool.last_trade_bin_id > pool.maximum_bin_id)
        || pool
            .accounted_option_reserve
            .checked_add(pool.protocol_fee_option)
            .is_none()
        || pool
            .accounted_quote_reserve
            .checked_add(pool.protocol_fee_quote)
            .is_none()
    {
        return Err(VaultError::InvalidAmoebaDlmmPool.into());
    }
    Ok(pool)
}

pub(super) fn load_bin_page(
    program_id: &Pubkey,
    pool: &Pubkey,
    page_info: &AccountInfo,
) -> Result<AmoebaDlmmBinPageV1, ProgramError> {
    let page: AmoebaDlmmBinPageV1 = load_light_state(
        page_info,
        program_id,
        &AMOEBA_DLMM_BIN_PAGE_LIGHT_DISCRIMINATOR,
        VaultError::InvalidAmoebaDlmmBinPage,
    )?;
    let (expected, bump) = derive_ameba_dlmm_bin_page_pda(program_id, pool, page.page_index);
    let expected_first = page_first_bin(page.page_index).map_err(math_error)?;
    if *page_info.key != expected
        || page.bump != bump
        || page.pool != *pool
        || page.page_index >= MAX_AMOEBA_DLMM_PAGE_COUNT
        || page.first_bin_id != expected_first
        || !page.has_current_layout()
        || refresh_local_liquidity_bits(&page.option_reserve, &page.quote_reserve)
            != (page.bid_bitmap, page.ask_bitmap)
    {
        return Err(VaultError::InvalidAmoebaDlmmBinPage.into());
    }
    Ok(page)
}

pub(super) fn load_share_page(
    program_id: &Pubkey,
    pool: &Pubkey,
    share_info: &AccountInfo,
) -> Result<AmoebaDlmmSharePageV1, ProgramError> {
    let page: AmoebaDlmmSharePageV1 = load_light_state(
        share_info,
        program_id,
        &AMOEBA_DLMM_SHARE_PAGE_LIGHT_DISCRIMINATOR,
        VaultError::InvalidAmoebaDlmmSharePage,
    )?;
    let (expected, bump) = derive_ameba_dlmm_share_page_pda(program_id, pool, page.page_index);
    if *share_info.key != expected
        || page.bump != bump
        || page.pool != *pool
        || page.page_index >= MAX_AMOEBA_DLMM_PAGE_COUNT
        || page.first_bin_id != page_first_bin(page.page_index).map_err(math_error)?
        || !page.has_current_layout()
    {
        return Err(VaultError::InvalidAmoebaDlmmSharePage.into());
    }
    Ok(page)
}

pub(super) fn load_position(
    program_id: &Pubkey,
    pool: &Pubkey,
    position_info: &AccountInfo,
) -> Result<AmoebaDlmmPositionV1, ProgramError> {
    let position: AmoebaDlmmPositionV1 = load_light_state(
        position_info,
        program_id,
        &AMOEBA_DLMM_POSITION_LIGHT_DISCRIMINATOR,
        VaultError::InvalidAmoebaDlmmPosition,
    )?;
    let (expected, bump) =
        derive_ameba_dlmm_position_pda(program_id, pool, &position.owner, position.position_nonce);
    let expected_initialized_bitmap = position
        .liquidity_shares
        .iter()
        .enumerate()
        .fold(0u32, |bitmap, (index, shares)| {
            bitmap | (u32::from(*shares != 0) << index)
        });
    if *position_info.key != expected
        || position.bump != bump
        || position.pool != *pool
        || !position.has_current_layout()
        || !(1..=32).contains(&position.bin_count)
        || position.lower_bin_id == 0
        || position
            .lower_bin_id
            .checked_add(position.bin_count as u16 - 1)
            .is_none()
        || position.initialized_bitmap != expected_initialized_bitmap
        || (position.bin_count < 32 && position.initialized_bitmap >> position.bin_count != 0)
    {
        return Err(VaultError::InvalidAmoebaDlmmPosition.into());
    }
    Ok(position)
}

pub(super) fn set_page_bit(bitmap: &mut u64, page_index: u16, value: bool) -> ProgramResult {
    if page_index >= MAX_AMOEBA_DLMM_PAGE_COUNT {
        return Err(VaultError::InvalidAmoebaDlmmGrid.into());
    }
    let mask = 1u64 << page_index;
    if value {
        *bitmap |= mask;
    } else {
        *bitmap &= !mask;
    }
    Ok(())
}

pub(super) fn page_bit(bitmap: &u64, page_index: u16) -> bool {
    page_index < MAX_AMOEBA_DLMM_PAGE_COUNT && bitmap & (1u64 << page_index) != 0
}

pub(super) fn first_set_page(bitmap: &u64) -> Option<u16> {
    (*bitmap != 0).then(|| bitmap.trailing_zeros() as u16)
}

pub(super) fn last_set_page(bitmap: &u64) -> Option<u16> {
    (*bitmap != 0).then(|| 63 - bitmap.leading_zeros() as u16)
}

pub(super) fn next_set_page(bitmap: &u64, current: u16, ascending: bool) -> Option<u16> {
    if ascending {
        ((current as u32 + 1)..MAX_AMOEBA_DLMM_PAGE_COUNT as u32)
            .find(|index| page_bit(bitmap, *index as u16))
            .map(|index| index as u16)
    } else {
        (0..current).rev().find(|index| page_bit(bitmap, *index))
    }
}

pub(super) fn assert_program_accounts(
    light_token_program_info: &AccountInfo,
    compressed_token_authority_info: &AccountInfo,
    spl_token_program_info: &AccountInfo,
    system_program_info: &AccountInfo,
) -> ProgramResult {
    if *light_token_program_info.key != light_token_program_id() {
        return Err(VaultError::InvalidLightTokenProgram.into());
    }
    if *compressed_token_authority_info.key != cpi_authority() {
        return Err(VaultError::InvalidCompressedTokenAuthority.into());
    }
    if *spl_token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    Ok(())
}

pub(super) fn load_user_transfer_account(
    program_id: &Pubkey,
    account_info: &AccountInfo,
    owner: &Pubkey,
    mint: &Pubkey,
) -> Result<TokenAccount, ProgramError> {
    if account_info.owner == &spl_token_program_id() {
        validate_vault_token_account(account_info, mint, owner)
    } else {
        super::super::scoped_settlement::load_scoped_holder_token_account(
            program_id,
            account_info,
            owner,
            mint,
        )
    }
}

pub(super) fn validate_pool_vaults(
    program_id: &Pubkey,
    pool_info: &AccountInfo,
    pool: &AmoebaDlmmPoolV1,
    authority_info: &AccountInfo,
    option_vault_info: &AccountInfo,
    quote_vault_info: &AccountInfo,
) -> Result<(TokenAccount, TokenAccount), ProgramError> {
    let (authority, _) = derive_ameba_dlmm_authority_pda(program_id, pool_info.key);
    let option = load_pool_vault(
        program_id,
        pool_info.key,
        &authority,
        authority_info,
        &pool.option_mint,
        &pool.option_vault,
        option_vault_info,
    )?;
    let quote = load_pool_vault(
        program_id,
        pool_info.key,
        &authority,
        authority_info,
        &pool.quote_mint,
        &pool.quote_vault,
        quote_vault_info,
    )?;
    Ok((option, quote))
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(super) fn load_pool_vault(
    program_id: &Pubkey,
    pool: &Pubkey,
    authority: &Pubkey,
    authority_info: &AccountInfo,
    mint: &Pubkey,
    stored_vault: &Pubkey,
    vault_info: &AccountInfo,
) -> Result<TokenAccount, ProgramError> {
    let (expected_vault, _) = derive_ameba_dlmm_vault_pda(program_id, pool, mint);
    if *authority_info.key != *authority
        || *vault_info.key != expected_vault
        || *stored_vault != expected_vault
    {
        return Err(VaultError::InvalidAmoebaDlmmVault.into());
    }
    load_canonical_light_token_account(vault_info, authority, mint)
        .map_err(|_| ProgramError::from(VaultError::InvalidAmoebaDlmmVault))
}

pub(super) fn ensure_custody(
    pool: &AmoebaDlmmPoolV1,
    option_vault: &TokenAccount,
    quote_vault: &TokenAccount,
) -> ProgramResult {
    let option_liability = pool
        .accounted_option_reserve
        .checked_add(pool.protocol_fee_option)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let quote_liability = pool
        .accounted_quote_reserve
        .checked_add(pool.protocol_fee_quote)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if option_vault.amount < option_liability || quote_vault.amount < quote_liability {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    Ok(())
}

#[inline(never)]
pub(super) fn validate_pool_vault_amounts(
    program_id: &Pubkey,
    pool_info: &AccountInfo,
    pool: &AmoebaDlmmPoolV1,
    authority_info: &AccountInfo,
    option_vault_info: &AccountInfo,
    quote_vault_info: &AccountInfo,
) -> Result<(u64, u64), ProgramError> {
    let (option_vault, quote_vault) = validate_pool_vaults(
        program_id,
        pool_info,
        pool,
        authority_info,
        option_vault_info,
        quote_vault_info,
    )?;
    ensure_custody(pool, &option_vault, &quote_vault)?;
    Ok((option_vault.amount, quote_vault.amount))
}

pub(super) fn validate_new_token_vault_target(account_info: &AccountInfo) -> ProgramResult {
    if account_info.owner != &system_program::id()
        || account_info.executable
        || account_info.data_len() != 0
    {
        return Err(VaultError::AlreadyInitialized.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn create_light_vault<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    pool: &Pubkey,
    mint: &AccountInfo<'a>,
    authority: &Pubkey,
    vault: &AccountInfo<'a>,
    token_config: &AccountInfo<'a>,
    token_rent_sponsor: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    light_token_program_info: &AccountInfo<'a>,
) -> ProgramResult {
    validate_new_token_vault_target(vault)?;
    let (expected, bump) = derive_ameba_dlmm_vault_pda(program_id, pool, mint.key);
    if *vault.key != expected {
        return Err(VaultError::InvalidAmoebaDlmmVault.into());
    }
    let bump_bytes = [bump];
    let seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        AMOEBA_DLMM_VAULT_PDA_SEED,
        pool.as_ref(),
        mint.key.as_ref(),
        &bump_bytes,
    ];
    let instruction = create_token_account_rent_free(
        payer.key,
        vault.key,
        mint.key,
        authority,
        token_config.key,
        token_rent_sponsor.key,
        system_program_info.key,
        program_id,
        seeds,
    )?;
    invoke_signed(
        &instruction,
        &[
            vault.clone(),
            mint.clone(),
            payer.clone(),
            token_config.clone(),
            system_program_info.clone(),
            token_rent_sponsor.clone(),
            light_token_program_info.clone(),
        ],
        &[seeds],
    )
    .map_err(|_| VaultError::InvalidAmoebaDlmmVault.into())
}
