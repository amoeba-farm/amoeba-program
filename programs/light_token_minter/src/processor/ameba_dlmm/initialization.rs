use super::*;

#[inline(never)]
pub(super) fn process_initialize_collective_pool_core(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: InitializeAmoebaDlmmPoolV1Params,
) -> ProgramResult {
    if accounts.len() < 17 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let account_info_iter = &mut accounts.iter();
    let admin_info = next_account_info(account_info_iter)?;
    let config_info = next_account_info(account_info_iter)?;
    let market_info = next_account_info(account_info_iter)?;
    let month_info = next_account_info(account_info_iter)?;
    let pool_info = next_account_info(account_info_iter)?;
    let authority_info = next_account_info(account_info_iter)?;
    let option_mint_info = next_account_info(account_info_iter)?;
    let quote_mint_info = next_account_info(account_info_iter)?;
    let option_vault_info = next_account_info(account_info_iter)?;
    let quote_vault_info = next_account_info(account_info_iter)?;
    let light_token_program_info = next_account_info(account_info_iter)?;
    let compressed_token_authority_info = next_account_info(account_info_iter)?;
    let state_config_info = next_account_info(account_info_iter)?;
    let state_rent_sponsor_info = next_account_info(account_info_iter)?;
    let token_config_info = next_account_info(account_info_iter)?;
    let token_rent_sponsor_info = next_account_info(account_info_iter)?;
    let system_program_info = next_account_info(account_info_iter)?;
    let light_tail = account_info_iter.as_slice();

    if !admin_info.is_signer || !admin_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *light_token_program_info.key != light_token_program_id()
        || *compressed_token_authority_info.key != cpi_authority()
        || *system_program_info.key != system_program::id()
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    if crate::pubkey_is_default(&params.liquidity_manager) {
        return Err(VaultError::UnauthorizedAmoebaDlmmManager.into());
    }
    let mut market = load_valid_market(program_id, market_info)?;
    validate_instrument_definition(&market.instrument)
        .map_err(|_| VaultError::InvalidAmoebaDlmmGrid)?;
    validate_market_parameters(&market.params, market.instrument.max_payout_per_contract)
        .map_err(|_| VaultError::InvalidAmoebaDlmmGrid)?;
    let option_mint = market
        .long_contract_mint
        .ok_or(VaultError::InvalidContractMint)?;
    if option_mint != *option_mint_info.key
        || market.collateral_mint != *quote_mint_info.key
        || config.usdc_mint != *quote_mint_info.key
        || !market.mint_accounting.has_canonical_layout()
    {
        return Err(VaultError::InvalidMint.into());
    }
    // Pool initialization is read-only with respect to Market. Validate the
    // canonical mint identity and authorities here; mutable value-flow lanes
    // reconcile any monotonic holder-authorized burn before using supply.
    let _ = validate_canonical_market_mint(market_info, &mut market, option_mint_info, 0)?;
    validate_collateral_mint_account(quote_mint_info, &spl_token_program_id())?;
    let maximum_price = market.instrument.max_payout_per_contract;
    let tick = market.params.tick_size;
    if tick == 0
        || maximum_price == 0
        || maximum_price % tick != 0
        || maximum_price / tick > MAX_AMOEBA_DLMM_BIN_COUNT as u64
        || market.params.taker_fee_bps > MAX_AMOEBA_DLMM_SWAP_FEE_BPS
        || params.protocol_fee_share_bps > 10_000
    {
        return Err(VaultError::InvalidAmoebaDlmmGrid.into());
    }
    let maximum_bin_id =
        u16::try_from(maximum_price / tick).map_err(|_| VaultError::InvalidAmoebaDlmmGrid)?;
    let maximum_bins_per_swap = market
        .params
        .max_fills_per_instruction
        .min(MAX_AMOEBA_DLMM_BINS_PER_SWAP);
    if maximum_bin_id == 0 || maximum_bins_per_swap == 0 {
        return Err(VaultError::InvalidAmoebaDlmmGrid.into());
    }

    let (expected_pool, pool_bump) = derive_ameba_dlmm_pool_pda(program_id, market_info.key);
    if *pool_info.key != expected_pool {
        return Err(VaultError::InvalidAmoebaDlmmPool.into());
    }
    validate_create_only_program_account_target(program_id, pool_info)?;
    let (authority, _) = derive_ameba_dlmm_authority_pda(program_id, pool_info.key);
    if *authority_info.key != authority {
        return Err(VaultError::InvalidAmoebaDlmmPool.into());
    }
    let (option_vault, _) =
        derive_ameba_dlmm_vault_pda(program_id, pool_info.key, option_mint_info.key);
    let (quote_vault, _) =
        derive_ameba_dlmm_vault_pda(program_id, pool_info.key, quote_mint_info.key);
    if *option_vault_info.key != option_vault || *quote_vault_info.key != quote_vault {
        return Err(VaultError::InvalidAmoebaDlmmVault.into());
    }

    let pool_bump_bytes = [pool_bump];
    let pool_prefunded_lamports = pool_info.lamports();
    create_program_account(
        admin_info,
        pool_info,
        system_program_info,
        program_id,
        AmoebaDlmmPoolV1::LEN,
        &[
            AMOEBA_DLMM_POOL_PDA_SEED,
            market_info.key.as_ref(),
            &pool_bump_bytes,
        ],
    )?;
    create_light_vault(
        program_id,
        admin_info,
        pool_info.key,
        option_mint_info,
        &authority,
        option_vault_info,
        token_config_info,
        token_rent_sponsor_info,
        system_program_info,
        light_token_program_info,
    )?;
    create_light_vault(
        program_id,
        admin_info,
        pool_info.key,
        quote_mint_info,
        &authority,
        quote_vault_info,
        token_config_info,
        token_rent_sponsor_info,
        system_program_info,
        light_token_program_info,
    )?;

    let slot = Clock::get()?.slot;
    let mut pool = AmoebaDlmmPoolV1 {
        is_initialized: true,
        bump: pool_bump,
        market: *market_info.key,
        oracle_month: *month_info.key,
        liquidity_manager: params.liquidity_manager,
        option_mint,
        quote_mint: *quote_mint_info.key,
        option_vault,
        quote_vault,
        expiry_ts: market.instrument.expiry_ts,
        tick_size_quote_atomic: tick,
        maximum_price_quote_atomic: maximum_price,
        maximum_bin_id,
        swap_fee_bps: market.params.taker_fee_bps,
        protocol_fee_share_bps: params.protocol_fee_share_bps,
        maximum_bins_per_swap,
        last_updated_slot: slot,
        ..AmoebaDlmmPoolV1::default()
    };
    initialize_compression_info(program_id, &mut pool, state_config_info, slot)?;
    store_light_state(pool_info, &pool)?;
    register_initialized_pdas(
        program_id,
        admin_info,
        state_config_info,
        state_rent_sponsor_info,
        pool_info.lamports().saturating_sub(pool_prefunded_lamports),
        &[pool_info.clone()],
        light_tail,
        &params.create_accounts_proof,
    )?;
    let _ = validate_pool_vaults(
        program_id,
        pool_info,
        &pool,
        authority_info,
        option_vault_info,
        quote_vault_info,
    )?;
    emit_event(
        &EVENT_POOL_INITIALIZED,
        AmoebaDlmmEvent::PoolInitialized(PoolInitializedEvent {
            pool: *pool_info.key,
            market: *market_info.key,
            liquidity_manager: pool.liquidity_manager,
            maximum_bin_id,
            slot,
        }),
    )
}

#[inline(never)]
pub(super) fn process_initialize_bin_page(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: InitializeAmoebaDlmmBinPageV1Params,
) -> ProgramResult {
    if accounts.len() < 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let manager_info = &accounts[0];
    let pool_info = &accounts[1];
    let page_info = &accounts[2];
    let share_info = &accounts[3];
    let state_config_info = &accounts[4];
    let state_rent_sponsor_info = &accounts[5];
    let system_program_info = &accounts[6];
    let light_tail = &accounts[7..];
    if !manager_info.is_signer || !manager_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let mut pool = load_pool(program_id, pool_info)?;
    if pool.liquidity_manager != *manager_info.key {
        return Err(VaultError::UnauthorizedAmoebaDlmmManager.into());
    }
    if !matches!(
        pool.status,
        AmoebaDlmmPoolStatus::Pending | AmoebaDlmmPoolStatus::Paused
    ) {
        return Err(VaultError::InvalidAmoebaDlmmStatusTransition.into());
    }
    let page_count =
        (pool.maximum_bin_id as usize + AMOEBA_DLMM_BINS_PER_PAGE - 1) / AMOEBA_DLMM_BINS_PER_PAGE;
    if params.page_index as usize >= page_count
        || params.page_index >= MAX_AMOEBA_DLMM_PAGE_COUNT
        || page_bit(&pool.initialized_page_bitmap, params.page_index)
    {
        return Err(VaultError::InvalidAmoebaDlmmBinPage.into());
    }
    let (expected_page, page_bump) =
        derive_ameba_dlmm_bin_page_pda(program_id, pool_info.key, params.page_index);
    let (expected_share, share_bump) =
        derive_ameba_dlmm_share_page_pda(program_id, pool_info.key, params.page_index);
    if *page_info.key != expected_page || *share_info.key != expected_share {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, page_info)?;
    validate_create_only_program_account_target(program_id, share_info)?;
    let page_index_bytes = params.page_index.to_le_bytes();
    let page_bump_bytes = [page_bump];
    let share_bump_bytes = [share_bump];
    let page_prefunded_lamports = page_info.lamports();
    let share_prefunded_lamports = share_info.lamports();
    create_program_account(
        manager_info,
        page_info,
        system_program_info,
        program_id,
        AmoebaDlmmBinPageV1::LEN,
        &[
            AMOEBA_DLMM_BIN_PAGE_PDA_SEED,
            pool_info.key.as_ref(),
            &page_index_bytes,
            &page_bump_bytes,
        ],
    )?;
    create_program_account(
        manager_info,
        share_info,
        system_program_info,
        program_id,
        AmoebaDlmmSharePageV1::LEN,
        &[
            AMOEBA_DLMM_SHARE_PAGE_PDA_SEED,
            pool_info.key.as_ref(),
            &page_index_bytes,
            &share_bump_bytes,
        ],
    )?;
    let slot = Clock::get()?.slot;
    let first_bin_id = page_first_bin(params.page_index).map_err(math_error)?;
    let mut page = AmoebaDlmmBinPageV1 {
        is_initialized: true,
        bump: page_bump,
        pool: *pool_info.key,
        page_index: params.page_index,
        first_bin_id,
        last_updated_slot: slot,
        ..AmoebaDlmmBinPageV1::default()
    };
    let mut shares = AmoebaDlmmSharePageV1 {
        is_initialized: true,
        bump: share_bump,
        pool: *pool_info.key,
        page_index: params.page_index,
        first_bin_id,
        last_updated_slot: slot,
        ..AmoebaDlmmSharePageV1::default()
    };
    initialize_compression_info(program_id, &mut page, state_config_info, slot)?;
    initialize_compression_info(program_id, &mut shares, state_config_info, slot)?;
    store_light_state(page_info, &page)?;
    store_light_state(share_info, &shares)?;
    register_initialized_pdas(
        program_id,
        manager_info,
        state_config_info,
        state_rent_sponsor_info,
        page_info
            .lamports()
            .saturating_sub(page_prefunded_lamports)
            .checked_add(
                share_info
                    .lamports()
                    .saturating_sub(share_prefunded_lamports),
            )
            .ok_or(VaultError::ArithmeticOverflow)?,
        &[page_info.clone(), share_info.clone()],
        light_tail,
        &params.create_accounts_proof,
    )?;
    set_page_bit(&mut pool.initialized_page_bitmap, params.page_index, true)?;
    pool.last_updated_slot = slot;
    store_light_state(pool_info, &pool)?;
    emit_event(
        &EVENT_PAGE_INITIALIZED,
        AmoebaDlmmEvent::PageInitialized(PageInitializedEvent {
            pool: *pool_info.key,
            page_index: params.page_index,
            slot,
        }),
    )
}

#[inline(never)]
pub(super) fn process_initialize_position(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: InitializeAmoebaDlmmPositionV1Params,
) -> ProgramResult {
    if accounts.len() < 6 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let owner_info = &accounts[0];
    let pool_info = &accounts[1];
    let position_info = &accounts[2];
    let state_config_info = &accounts[3];
    let state_rent_sponsor_info = &accounts[4];
    let system_program_info = &accounts[5];
    let light_tail = &accounts[6..];
    if !owner_info.is_signer || !owner_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let mut pool = load_pool(program_id, pool_info)?;
    if pool.liquidity_manager != *owner_info.key {
        return Err(VaultError::UnauthorizedAmoebaDlmmManager.into());
    }
    if !matches!(
        pool.status,
        AmoebaDlmmPoolStatus::Pending | AmoebaDlmmPoolStatus::Paused
    ) || !(1..=32).contains(&params.bin_count)
        || params.lower_bin_id == 0
    {
        return Err(VaultError::InvalidAmoebaDlmmPosition.into());
    }
    let upper = params
        .lower_bin_id
        .checked_add(params.bin_count as u16 - 1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if upper > pool.maximum_bin_id {
        return Err(VaultError::InvalidAmoebaDlmmPosition.into());
    }
    let (expected, bump) = derive_ameba_dlmm_position_pda(
        program_id,
        pool_info.key,
        owner_info.key,
        params.position_nonce,
    );
    if *position_info.key != expected {
        return Err(VaultError::InvalidAmoebaDlmmPosition.into());
    }
    validate_create_only_program_account_target(program_id, position_info)?;
    let nonce_bytes = params.position_nonce.to_le_bytes();
    let bump_bytes = [bump];
    let position_prefunded_lamports = position_info.lamports();
    create_program_account(
        owner_info,
        position_info,
        system_program_info,
        program_id,
        AmoebaDlmmPositionV1::LEN,
        &[
            AMOEBA_DLMM_POSITION_PDA_SEED,
            pool_info.key.as_ref(),
            owner_info.key.as_ref(),
            &nonce_bytes,
            &bump_bytes,
        ],
    )?;
    let slot = Clock::get()?.slot;
    let mut position = AmoebaDlmmPositionV1 {
        is_initialized: true,
        bump,
        pool: *pool_info.key,
        owner: *owner_info.key,
        position_nonce: params.position_nonce,
        lower_bin_id: params.lower_bin_id,
        bin_count: params.bin_count,
        last_updated_slot: slot,
        ..AmoebaDlmmPositionV1::default()
    };
    initialize_compression_info(program_id, &mut position, state_config_info, slot)?;
    store_light_state(position_info, &position)?;
    register_initialized_pdas(
        program_id,
        owner_info,
        state_config_info,
        state_rent_sponsor_info,
        position_info
            .lamports()
            .saturating_sub(position_prefunded_lamports),
        &[position_info.clone()],
        light_tail,
        &params.create_accounts_proof,
    )?;
    pool.position_count = pool
        .position_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    pool.last_updated_slot = slot;
    store_light_state(pool_info, &pool)?;
    emit_event(
        &EVENT_POSITION_INITIALIZED,
        AmoebaDlmmEvent::PositionInitialized(PositionInitializedEvent {
            pool: *pool_info.key,
            position: *position_info.key,
            owner: *owner_info.key,
            position_nonce: params.position_nonce,
            lower_bin_id: params.lower_bin_id,
            bin_count: params.bin_count,
            slot,
        }),
    )
}
