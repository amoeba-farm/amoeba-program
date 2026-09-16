use super::*;

#[inline(never)]
pub(super) fn process_set_collective_pool_status_core(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SetAmoebaDlmmPoolStatusV1Params,
    writer_sides: (bool, bool),
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
            || (!writer_sides.0
                && (pool.best_bid_bin_id == AMOEBA_DLMM_EMPTY_BIN_ID
                    || first_set_page(&pool.bid_page_bitmap).is_none()))
            || (!writer_sides.1
                && (pool.best_ask_bin_id == AMOEBA_DLMM_EMPTY_BIN_ID
                    || first_set_page(&pool.ask_page_bitmap).is_none()))
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

#[inline(always)]
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
