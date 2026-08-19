use super::*;

pub(super) fn process_queue_stake_amba_for_samba(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: QueueStakeAmbaForSambaParams,
) -> ProgramResult {
    if params.amba_amount == 0 {
        return Err(VaultError::AmountMustBePositive.into());
    }
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let owner_info = &accounts[0];
    let config_info = &accounts[1];
    let major_config_info = &accounts[2];
    let ledger_info = &accounts[3];
    let staking_pool_info = &accounts[4];
    let activation_info = &accounts[5];
    let system_program_info = &accounts[6];
    if !owner_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_system_program(system_program_info)?;
    let config = load_active_oracle_vault_config(program_id, config_info)?;
    load_separated_oracle_major_token_config(program_id, major_config_info, &config)?;
    let mut ledger = load_canonical_oracle_player_ledger(program_id, ledger_info, owner_info.key)?;
    load_canonical_oracle_staking_pool(program_id, staking_pool_info, major_config_info.key)?;
    let mut activation = load_or_create_oracle_stake_activation(
        program_id,
        owner_info,
        activation_info,
        owner_info.key,
        system_program_info,
    )?;
    if activation.queued_amba != 0 || activation.activate_after_ts != 0 {
        return Err(VaultError::InvalidOracleStakeActivation.into());
    }
    debit_oracle_available(&mut ledger, params.amba_amount)?;
    let (slot, now) = current_slot_and_unix_timestamp()?;
    activation.queued_amba = params.amba_amount;
    activation.activate_after_ts = now
        .checked_add(ORACLE_SAMBA_STAKE_ACTIVATION_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    activation.last_updated_slot = slot;
    ledger.last_updated_slot = slot;
    ledger.last_balance_change_slot = slot;

    store_state(ledger_info, &ledger)?;
    store_state(activation_info, &activation)
}

pub(super) fn process_activate_queued_stake_amba_for_samba(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ActivateQueuedStakeAmbaForSambaParams,
) -> ProgramResult {
    if accounts.len() != 10 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let owner_info = &accounts[0];
    let config_info = &accounts[1];
    let major_config_info = &accounts[2];
    let staking_pool_info = &accounts[3];
    let activation_info = &accounts[4];
    let funnel_info = &accounts[5];
    let funnel_token_info = &accounts[6];
    let samba_mint_info = &accounts[7];
    let owner_samba_token_info = &accounts[8];
    let token_program_info = &accounts[9];
    if !owner_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }

    let config = load_active_oracle_vault_config(program_id, config_info)?;
    let major_config =
        load_separated_oracle_major_token_config(program_id, major_config_info, &config)?;
    let mut pool =
        load_canonical_oracle_staking_pool(program_id, staking_pool_info, major_config_info.key)?;
    if pool.governance_lock_count != 0 {
        return Err(VaultError::SambaSupplyLockedForVoting.into());
    }
    let mut activation =
        load_canonical_oracle_stake_activation(program_id, activation_info, owner_info.key)?;
    let (slot, now) = current_slot_and_unix_timestamp()?;
    ensure_oracle_stake_activation_ready_at(&activation, now)?;

    load_canonical_oracle_reward_funnel(
        program_id,
        funnel_info,
        major_config_info.key,
        &major_config.mint,
        funnel_token_info.key,
    )?;
    let funnel_token = validate_strict_token_account(
        funnel_token_info,
        &major_config.mint,
        funnel_info.key,
        VaultError::InvalidOracleRewardFunnel,
    )?;
    if funnel_token.amount != 0 {
        return Err(VaultError::InvalidOracleStakeActivation.into());
    }

    let samba_mint = validate_oracle_samba_mint(&pool, samba_mint_info, config_info.key)?;
    synchronize_oracle_samba_supply(&mut pool, samba_mint.supply)?;
    validate_owner_samba_token_account(
        owner_samba_token_info,
        owner_info.key,
        samba_mint_info.key,
    )?;
    let queued_amba = activation.queued_amba;
    let samba_out = calculate_samba_shares_for_amba(&pool, queued_amba)?;
    if samba_out < params.min_samba_out {
        return Err(VaultError::InvalidSambaExchangeRate.into());
    }

    invoke_token_mint_to_checked(
        token_program_info,
        samba_mint_info,
        owner_samba_token_info,
        config_info,
        samba_out,
        samba_mint.decimals,
        &[&[CURRENT_STATE_NAMESPACE_SEED, VAULT_PDA_SEED, &[config.bump]]],
    )?;

    pool.active_amba_backing = pool
        .active_amba_backing
        .checked_add(queued_amba)
        .ok_or(VaultError::ArithmeticOverflow)?;
    pool.samba_supply = pool
        .samba_supply
        .checked_add(samba_out)
        .ok_or(VaultError::ArithmeticOverflow)?;
    pool.last_updated_slot = slot;
    activation.queued_amba = 0;
    activation.activate_after_ts = 0;
    activation.last_updated_slot = slot;
    validate_oracle_staking_pool_accounting(&pool)?;
    let reloaded_mint = validate_mint_account(samba_mint_info, token_program_info.key)?;
    if reloaded_mint.supply != pool.samba_supply {
        return Err(VaultError::InvalidOracleStakingPool.into());
    }

    store_state(staking_pool_info, &pool)?;
    store_state(activation_info, &activation)
}

pub(super) fn process_cancel_queued_stake_amba(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 4 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let owner_info = &accounts[0];
    let config_info = &accounts[1];
    let ledger_info = &accounts[2];
    let activation_info = &accounts[3];
    if !owner_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    load_current_canonical_vault_config(program_id, config_info)?;
    let mut ledger = load_canonical_oracle_player_ledger(program_id, ledger_info, owner_info.key)?;
    let mut activation =
        load_canonical_oracle_stake_activation(program_id, activation_info, owner_info.key)?;
    if activation.queued_amba == 0 || activation.activate_after_ts == 0 {
        return Err(VaultError::InvalidOracleStakeActivation.into());
    }

    let slot = Clock::get()?.slot;
    ledger.major_tokens = ledger
        .major_tokens
        .checked_add(activation.queued_amba)
        .ok_or(VaultError::ArithmeticOverflow)?;
    ledger.last_updated_slot = slot;
    ledger.last_balance_change_slot = slot;
    activation.queued_amba = 0;
    activation.activate_after_ts = 0;
    activation.last_updated_slot = slot;

    store_state(ledger_info, &ledger)?;
    store_state(activation_info, &activation)
}

pub(super) fn process_request_unstake_samba(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: RequestUnstakeSambaParams,
) -> ProgramResult {
    if params.samba_amount == 0 {
        return Err(VaultError::AmountMustBePositive.into());
    }
    if accounts.len() != 9 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let owner_info = &accounts[0];
    let config_info = &accounts[1];
    let major_config_info = &accounts[2];
    let staking_pool_info = &accounts[3];
    let unstake_request_info = &accounts[4];
    let samba_mint_info = &accounts[5];
    let owner_samba_token_info = &accounts[6];
    let token_program_info = &accounts[7];
    let system_program_info = &accounts[8];
    if !owner_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    validate_system_program(system_program_info)?;
    let config = load_active_oracle_vault_config(program_id, config_info)?;
    load_separated_oracle_major_token_config(program_id, major_config_info, &config)?;
    let mut pool =
        load_canonical_oracle_staking_pool(program_id, staking_pool_info, major_config_info.key)?;
    if pool.governance_lock_count != 0 {
        return Err(VaultError::SambaSupplyLockedForVoting.into());
    }
    let samba_mint = validate_oracle_samba_mint(&pool, samba_mint_info, config_info.key)?;
    synchronize_oracle_samba_supply(&mut pool, samba_mint.supply)?;
    let owner_samba = validate_owner_samba_token_account(
        owner_samba_token_info,
        owner_info.key,
        samba_mint_info.key,
    )?;
    if owner_samba.amount < params.samba_amount {
        return Err(VaultError::InsufficientSambaTokens.into());
    }
    let amba_out = calculate_amba_for_samba_shares(&pool, params.samba_amount)?;
    if amba_out < params.min_amba_out {
        return Err(VaultError::InvalidSambaExchangeRate.into());
    }
    let mut request = load_or_create_oracle_unstake_request(
        program_id,
        owner_info,
        unstake_request_info,
        owner_info.key,
        system_program_info,
    )?;
    if request.pending_amba != 0 || request.claimable_at_ts != 0 {
        return Err(VaultError::InvalidOracleUnstakeRequest.into());
    }

    invoke_token_burn_checked(
        token_program_info,
        owner_samba_token_info,
        samba_mint_info,
        owner_info,
        params.samba_amount,
        samba_mint.decimals,
        &[],
    )?;

    let (slot, now) = current_slot_and_unix_timestamp()?;
    pool.active_amba_backing = pool
        .active_amba_backing
        .checked_sub(amba_out)
        .ok_or(VaultError::ArithmeticOverflow)?;
    pool.samba_supply = pool
        .samba_supply
        .checked_sub(params.samba_amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    pool.pending_unstake_amba = pool
        .pending_unstake_amba
        .checked_add(amba_out)
        .ok_or(VaultError::ArithmeticOverflow)?;
    pool.last_updated_slot = slot;
    request.pending_amba = amba_out;
    request.claimable_at_ts = now
        .checked_add(ORACLE_SAMBA_UNBONDING_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    request.last_updated_slot = slot;
    validate_oracle_staking_pool_accounting(&pool)?;
    let reloaded_mint = validate_mint_account(samba_mint_info, token_program_info.key)?;
    if reloaded_mint.supply != pool.samba_supply {
        return Err(VaultError::InvalidOracleStakingPool.into());
    }

    store_state(unstake_request_info, &request)?;
    store_state(staking_pool_info, &pool)
}

pub(super) fn process_complete_unstake_samba(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 6 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let owner_info = &accounts[0];
    let config_info = &accounts[1];
    let ledger_info = &accounts[2];
    let staking_pool_info = &accounts[3];
    let unstake_request_info = &accounts[4];
    let system_program_info = &accounts[5];
    if !owner_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_system_program(system_program_info)?;
    load_active_oracle_vault_config(program_id, config_info)?;
    let major_config_key = derive_oracle_major_token_config_pda(program_id).0;
    let mut ledger = load_or_create_oracle_player_ledger(
        program_id,
        owner_info,
        ledger_info,
        owner_info.key,
        system_program_info,
    )?;
    let mut pool =
        load_canonical_oracle_staking_pool(program_id, staking_pool_info, &major_config_key)?;
    let mut request =
        load_canonical_oracle_unstake_request(program_id, unstake_request_info, owner_info.key)?;
    let (slot, now) = current_slot_and_unix_timestamp()?;
    ensure_oracle_unstake_ready_at(&request, now)?;
    let claim_amount = request.pending_amba;
    pool.pending_unstake_amba = pool
        .pending_unstake_amba
        .checked_sub(claim_amount)
        .ok_or(VaultError::InvalidOracleStakingPool)?;
    pool.last_updated_slot = slot;
    request.pending_amba = 0;
    request.claimable_at_ts = 0;
    request.last_updated_slot = slot;
    ledger.major_tokens = ledger
        .major_tokens
        .checked_add(claim_amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    ledger.last_updated_slot = slot;
    ledger.last_balance_change_slot = slot;
    validate_oracle_staking_pool_accounting(&pool)?;

    store_state(ledger_info, &ledger)?;
    store_state(unstake_request_info, &request)?;
    store_state(staking_pool_info, &pool)
}
