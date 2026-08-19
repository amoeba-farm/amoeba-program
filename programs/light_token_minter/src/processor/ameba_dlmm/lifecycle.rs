use super::*;

#[inline(never)]
pub(super) fn process_set_collective_pool_status_core(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SetAmoebaDlmmPoolStatusV1Params,
) -> ProgramResult {
    if accounts.len() != 10 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let market_info = &accounts[2];
    let month_info = &accounts[3];
    let coverage_info = &accounts[4];
    let active_manifest_info = &accounts[5];
    let pool_info = &accounts[6];
    let authority_info = &accounts[7];
    let option_vault_info = &accounts[8];
    let quote_vault_info = &accounts[9];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !pool_info.is_writable {
        return Err(VaultError::AmoebaDlmmAccountNotHot.into());
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let market = load_valid_market(program_id, market_info)?;
    let month = load_oracle_month_state(month_info, program_id)?;
    let mut pool = load_pool(program_id, pool_info)?;
    if !pool.status.can_admin_transition_to(params.status) {
        return Err(VaultError::InvalidAmoebaDlmmStatusTransition.into());
    }
    let vaults = validate_pool_vaults(
        program_id,
        pool_info,
        &pool,
        authority_info,
        option_vault_info,
        quote_vault_info,
    )?;
    ensure_custody(&pool, &vaults.0, &vaults.1)?;
    if params.status == AmoebaDlmmPoolStatus::Active {
        ensure_market_value_flow_unpaused(&config, &market)?;
        ensure_oracle_game_window(&market, &month)?;
        let coverage =
            load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
        let active = load_valid_oracle_active_weight_manifest(
            program_id,
            month_info.key,
            active_manifest_info,
        )?;
        if !coverage.coverage_finalized
            || coverage.covered_sku_count != coverage.required_sku_count
            || active.phase != OracleRecipeWeightPhase::Finalized
            || active.rolling_manifest_hash != month.active_weight_manifest_hash
            || pool.best_bid_bin_id == AMOEBA_DLMM_EMPTY_BIN_ID
            || pool.best_ask_bin_id == AMOEBA_DLMM_EMPTY_BIN_ID
            || first_set_page(&pool.bid_page_bitmap).is_none()
            || first_set_page(&pool.ask_page_bitmap).is_none()
        {
            return Err(VaultError::InvalidAmoebaDlmmStatusTransition.into());
        }
    }
    let old_status = pool.status;
    let slot = Clock::get()?.slot;
    pool.status = params.status;
    pool.last_updated_slot = slot;
    store_light_state(pool_info, &pool)?;
    emit_event(
        &EVENT_STATUS_CHANGED,
        AmoebaDlmmEvent::Status(StatusEvent {
            pool: *pool_info.key,
            old_status,
            new_status: pool.status,
            slot,
        }),
    )
}

#[inline(never)]
pub(super) fn process_collect_protocol_fees(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: CollectAmoebaDlmmProtocolFeesV1Params,
) -> ProgramResult {
    if accounts.len() != 16 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let pool_info = &accounts[2];
    let authority_info = &accounts[3];
    let option_mint_info = &accounts[4];
    let quote_mint_info = &accounts[5];
    let option_vault_info = &accounts[6];
    let quote_vault_info = &accounts[7];
    let option_destination_info = &accounts[8];
    let quote_destination_info = &accounts[9];
    let light_token_program_info = &accounts[10];
    let compressed_token_authority_info = &accounts[11];
    let option_interface_info = &accounts[12];
    let quote_interface_info = &accounts[13];
    let spl_token_program_info = &accounts[14];
    let system_program_info = &accounts[15];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    assert_program_accounts(
        light_token_program_info,
        compressed_token_authority_info,
        spl_token_program_info,
        system_program_info,
    )?;
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let mut pool = load_pool(program_id, pool_info)?;
    if !pool.status.allows_protocol_fee_collection()
        || params.option_amount > pool.protocol_fee_option
        || params.quote_amount > pool.protocol_fee_quote
        || (params.quote_amount > 0
            && !matches!(
                pool.status,
                AmoebaDlmmPoolStatus::Settled | AmoebaDlmmPoolStatus::Closed
            ))
        || (params.option_amount == 0 && params.quote_amount == 0)
    {
        return Err(VaultError::InvalidAmoebaDlmmFees.into());
    }
    if *option_mint_info.key != pool.option_mint
        || *quote_mint_info.key != pool.quote_mint
        || *quote_destination_info.key != config.vault_token_account
    {
        return Err(VaultError::InvalidAmoebaDlmmVault.into());
    }
    validate_collateral_mint_account(option_mint_info, &spl_token_program_id())?;
    validate_collateral_mint_account(quote_mint_info, &spl_token_program_id())?;
    let expected_option_destination =
        crate::associated_token::get_associated_token_address_with_program_id(
            admin_info.key,
            option_mint_info.key,
            spl_token_program_info.key,
        );
    if *option_destination_info.key != expected_option_destination {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    let option_destination = validate_token_account(option_destination_info)?;
    if option_destination.owner != *admin_info.key || option_destination.mint != pool.option_mint {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    validate_vault_token_account(quote_destination_info, &pool.quote_mint, config_info.key)?;
    validate_spl_interface_account(option_mint_info.key, option_interface_info)?;
    validate_spl_interface_account(quote_mint_info.key, quote_interface_info)?;
    let before = validate_pool_vaults(
        program_id,
        pool_info,
        &pool,
        authority_info,
        option_vault_info,
        quote_vault_info,
    )?;
    ensure_custody(&pool, &before.0, &before.1)?;

    pool.protocol_fee_option = pool
        .protocol_fee_option
        .checked_sub(params.option_amount)
        .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
    pool.protocol_fee_quote = pool
        .protocol_fee_quote
        .checked_sub(params.quote_amount)
        .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
    let (_, authority_bump) = derive_ameba_dlmm_authority_pda(program_id, pool_info.key);
    let authority_bump_bytes = [authority_bump];
    let authority_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        AMOEBA_DLMM_AUTHORITY_PDA_SEED,
        pool_info.key.as_ref(),
        &authority_bump_bytes,
    ];
    let signers: &[&[&[u8]]] = &[authority_seeds];
    if params.option_amount > 0 {
        invoke_light_token_account_transfer_with_signer_seeds(
            params.option_amount,
            MarketMintAccounting::CANONICAL_DECIMALS,
            light_token_program_info,
            compressed_token_authority_info,
            admin_info,
            option_vault_info,
            option_destination_info,
            authority_info,
            option_mint_info,
            option_interface_info,
            spl_token_program_info,
            system_program_info,
            signers,
        )?;
    }
    if params.quote_amount > 0 {
        invoke_light_token_account_transfer_with_signer_seeds(
            params.quote_amount,
            MarketMintAccounting::CANONICAL_DECIMALS,
            light_token_program_info,
            compressed_token_authority_info,
            admin_info,
            quote_vault_info,
            quote_destination_info,
            authority_info,
            quote_mint_info,
            quote_interface_info,
            spl_token_program_info,
            system_program_info,
            signers,
        )?;
    }
    let after = validate_pool_vaults(
        program_id,
        pool_info,
        &pool,
        authority_info,
        option_vault_info,
        quote_vault_info,
    )?;
    if before.0.amount
        != after
            .0
            .amount
            .checked_add(params.option_amount)
            .ok_or(VaultError::ArithmeticOverflow)?
        || before.1.amount
            != after
                .1
                .amount
                .checked_add(params.quote_amount)
                .ok_or(VaultError::ArithmeticOverflow)?
    {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    ensure_custody(&pool, &after.0, &after.1)?;
    let slot = Clock::get()?.slot;
    pool.last_updated_slot = slot;
    store_light_state(pool_info, &pool)?;
    emit_event(
        &EVENT_FEES_COLLECTED,
        AmoebaDlmmEvent::Fees(FeesEvent {
            pool: *pool_info.key,
            option_amount: params.option_amount,
            quote_amount: params.quote_amount,
            slot,
        }),
    )
}

#[inline(always)]
pub(super) fn sweep_pool_oracle_bounty(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    month: &Pubkey,
    pool_info: &AccountInfo,
    pool: &mut AmoebaDlmmPoolV1,
    slot: u64,
) -> ProgramResult {
    let cranker_info = &accounts[0];
    let authority_info = &accounts[6];
    let quote_mint_info = &accounts[7];
    let quote_vault_info = &accounts[8];
    let reward_vault_info = &accounts[9];
    let reward_token_info = &accounts[10];
    let schedule_info = &accounts[11];
    let quote_interface_info = &accounts[12];
    let light_token_program_info = &accounts[13];
    let compressed_token_authority_info = &accounts[14];
    let spl_token_program_info = &accounts[15];
    let system_program_info = &accounts[16];
    assert_program_accounts(
        light_token_program_info,
        compressed_token_authority_info,
        spl_token_program_info,
        system_program_info,
    )?;
    let mut reward_vault =
        oracle_usdc::load_oracle_usdc_reward_vault(program_id, reward_vault_info)?;
    let mut schedule =
        oracle_usdc::load_oracle_usdc_reward_schedule(program_id, month, schedule_info)?;
    if reward_vault.mint != pool.quote_mint
        || schedule.reward_vault != *reward_vault_info.key
        || schedule.phase != OracleUsdcRewardSchedulePhase::Funded
        || schedule.bounty_fee_sweep_finalized
        || schedule.trading_fee_bounty_total != 0
    {
        return Err(VaultError::InvalidAmoebaDlmmSettlement.into());
    }
    validate_spl_interface_account(quote_mint_info.key, quote_interface_info)?;
    let (expected_authority, authority_bump) =
        derive_ameba_dlmm_authority_pda(program_id, pool_info.key);
    let quote_before = load_pool_vault(
        program_id,
        pool_info.key,
        &expected_authority,
        authority_info,
        quote_mint_info.key,
        &pool.quote_vault,
        quote_vault_info,
    )?;
    let required_quote = pool
        .accounted_quote_reserve
        .checked_add(pool.protocol_fee_quote)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if quote_before.amount < required_quote {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    let (reward_before, _) = oracle_usdc::validate_oracle_usdc_reward_custody(
        &reward_vault,
        reward_vault_info.key,
        reward_token_info,
        quote_mint_info,
        spl_token_program_info,
    )?;
    let bounty = mul_bps(
        pool.protocol_fee_quote,
        crate::constants::ORACLE_BOUNTY_PROTOCOL_FEE_SHARE_BPS,
    )?;
    let authority_bump_bytes = [authority_bump];
    let authority_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        AMOEBA_DLMM_AUTHORITY_PDA_SEED,
        pool_info.key.as_ref(),
        &authority_bump_bytes,
    ];
    let signers: &[&[&[u8]]] = &[authority_seeds];
    if bounty > 0 {
        invoke_light_token_account_transfer_with_signer_seeds(
            bounty,
            MarketMintAccounting::CANONICAL_DECIMALS,
            light_token_program_info,
            compressed_token_authority_info,
            cranker_info,
            quote_vault_info,
            reward_token_info,
            authority_info,
            quote_mint_info,
            quote_interface_info,
            spl_token_program_info,
            system_program_info,
            signers,
        )?;
    }
    let quote_after = load_canonical_light_token_account(
        quote_vault_info,
        &expected_authority,
        quote_mint_info.key,
    )?;
    let reward_after = validate_token_account(reward_token_info)?;
    if quote_before.amount
        != quote_after
            .amount
            .checked_add(bounty)
            .ok_or(VaultError::ArithmeticOverflow)?
        || reward_after.amount
            != reward_before
                .amount
                .checked_add(bounty)
                .ok_or(VaultError::ArithmeticOverflow)?
    {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    pool.protocol_fee_quote = pool
        .protocol_fee_quote
        .checked_sub(bounty)
        .ok_or(VaultError::ArithmeticOverflow)?;
    reward_vault.total_reserved = reward_vault
        .total_reserved
        .checked_add(bounty)
        .ok_or(VaultError::ArithmeticOverflow)?;
    reward_vault.last_updated_slot = slot;
    schedule.total_reward_budget = schedule
        .total_reward_budget
        .checked_add(bounty)
        .ok_or(VaultError::ArithmeticOverflow)?;
    schedule.remaining_reward_budget = schedule
        .remaining_reward_budget
        .checked_add(bounty)
        .ok_or(VaultError::ArithmeticOverflow)?;
    schedule.trading_fee_bounty_total = bounty;
    schedule.bounty_fee_sweep_finalized = true;
    schedule.last_updated_slot = slot;
    store_state(reward_vault_info, &reward_vault)?;
    store_state(schedule_info, &schedule)
}

pub(super) fn all_page_words_empty(bitmap: &u64) -> bool {
    *bitmap == 0
}

#[inline(never)]
pub(super) fn process_close_pool(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    if accounts.len() != 6 && accounts.len() != 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let pool_info = &accounts[2];
    let authority_info = &accounts[3];
    let option_vault_info = &accounts[4];
    let quote_vault_info = &accounts[5];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let mut pool = load_pool(program_id, pool_info)?;
    if pool.status != AmoebaDlmmPoolStatus::Settled
        || pool.position_count != 0
        || pool.accounted_option_reserve != 0
        || pool.accounted_quote_reserve != 0
        || pool.protocol_fee_option != 0
        || pool.protocol_fee_quote != 0
        || !all_page_words_empty(&pool.bid_page_bitmap)
        || !all_page_words_empty(&pool.ask_page_bitmap)
    {
        return Err(VaultError::AmoebaDlmmNotEmpty.into());
    }
    let _ = validate_pool_vaults(
        program_id,
        pool_info,
        &pool,
        authority_info,
        option_vault_info,
        quote_vault_info,
    )?;
    // Accounted reserves and fees are already proven zero above. Physical
    // surplus is an unsolicited donation and has no claimant; it must not keep
    // an otherwise terminal pool live forever. The canonical vault accounts
    // remain bound to the closed pool and no value-flow lane admits them after
    // this status transition.
    let slot = Clock::get()?.slot;
    if accounts.len() == 8 {
        let page_info = &accounts[6];
        let share_info = &accounts[7];
        let mut page = load_bin_page(program_id, pool_info.key, page_info)?;
        let mut shares = load_share_page(program_id, pool_info.key, share_info)?;
        if page.page_index != shares.page_index
            || page.option_reserve.iter().any(|amount| *amount != 0)
            || page.quote_reserve.iter().any(|amount| *amount != 0)
            || shares
                .total_liquidity_shares
                .iter()
                .any(|amount| *amount != 0)
            || page.bid_bitmap != 0
            || page.ask_bitmap != 0
            || !page_bit(&pool.initialized_page_bitmap, page.page_index)
        {
            return Err(VaultError::AmoebaDlmmNotEmpty.into());
        }
        set_page_bit(&mut pool.initialized_page_bitmap, page.page_index, false)?;
        page.is_initialized = false;
        shares.is_initialized = false;
        page.last_updated_slot = slot;
        shares.last_updated_slot = slot;
        pool.last_updated_slot = slot;
        store_light_state(page_info, &page)?;
        store_light_state(share_info, &shares)?;
        store_light_state(pool_info, &pool)?;
        return emit_event(
            &EVENT_POOL_CLOSED,
            AmoebaDlmmEvent::Closed(ClosedEvent {
                pool: *pool_info.key,
                page_index: Some(page.page_index),
                fully_closed: false,
                slot,
            }),
        );
    }
    if !all_page_words_empty(&pool.initialized_page_bitmap) {
        return Err(VaultError::AmoebaDlmmNotEmpty.into());
    }
    pool.status = AmoebaDlmmPoolStatus::Closed;
    pool.last_updated_slot = slot;
    store_light_state(pool_info, &pool)?;
    emit_event(
        &EVENT_POOL_CLOSED,
        AmoebaDlmmEvent::Closed(ClosedEvent {
            pool: *pool_info.key,
            page_index: None,
            fully_closed: true,
            slot,
        }),
    )
}
