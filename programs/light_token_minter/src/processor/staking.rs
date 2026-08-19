use super::*;

pub(super) enum OracleMajorTokenMovement {
    Deposit(DepositOracleMajorTokensParams),
    Withdraw(WithdrawOracleMajorTokensParams),
}

#[inline(never)]
pub(super) fn process_oracle_major_token_movement(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    movement: OracleMajorTokenMovement,
) -> ProgramResult {
    let (amount, is_deposit) = match movement {
        OracleMajorTokenMovement::Deposit(params) => (params.amount, true),
        OracleMajorTokenMovement::Withdraw(params) => (params.amount, false),
    };
    if amount == 0 {
        return Err(VaultError::AmountMustBePositive.into());
    }
    if (is_deposit && accounts.len() != 10) || (!is_deposit && accounts.len() < 10) {
        return Err(VaultError::InvalidAccountList.into());
    }
    let owner_info = &accounts[0];
    let config_info = &accounts[1];
    let major_config_info = &accounts[2];
    let ledger_info = &accounts[3];
    let treasury_info = &accounts[4];
    let user_token_info = &accounts[5];
    let vault_token_info = &accounts[6];
    let mint_info = &accounts[7];
    let token_program_info = &accounts[8];
    let system_program_info = &accounts[9];

    if !owner_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }

    let config = load_active_oracle_vault_config(program_id, config_info)?;
    let major_config =
        load_separated_oracle_major_token_config(program_id, major_config_info, &config)?;
    let mint = validate_mint_account(mint_info, token_program_info.key)?;
    validate_oracle_major_token_accounts(
        &major_config,
        owner_info.key,
        user_token_info,
        vault_token_info,
        mint_info,
        config_info.key,
    )?;

    if is_deposit {
        invoke_token_transfer_checked(
            token_program_info,
            user_token_info,
            mint_info,
            vault_token_info,
            owner_info,
            amount,
            mint.decimals,
            &[],
        )?;
    }

    let mut ledger = load_or_create_oracle_player_ledger(
        program_id,
        owner_info,
        ledger_info,
        owner_info.key,
        system_program_info,
    )?;
    let mut treasury =
        load_or_create_oracle_treasury(program_id, owner_info, treasury_info, system_program_info)?;
    let slot;
    if is_deposit {
        slot = Clock::get()?.slot;
        ledger.major_tokens = ledger
            .major_tokens
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        treasury.major_tokens = treasury
            .major_tokens
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
    } else {
        oracle_treasury_protocol_sleeve_total(&treasury)?;
        if ledger.major_tokens < amount || treasury.major_tokens < amount {
            return Err(VaultError::InsufficientAvailableCollateral.into());
        }
        slot = Clock::get()?.slot;
        ledger.major_tokens = ledger
            .major_tokens
            .checked_sub(amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        treasury.major_tokens = treasury
            .major_tokens
            .checked_sub(amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        oracle_treasury_protocol_sleeve_total(&treasury)?;

        invoke_token_transfer_checked(
            token_program_info,
            vault_token_info,
            mint_info,
            user_token_info,
            config_info,
            amount,
            mint.decimals,
            &[&[CURRENT_STATE_NAMESPACE_SEED, VAULT_PDA_SEED, &[config.bump]]],
        )?;
    }
    ledger.last_updated_slot = slot;
    ledger.last_balance_change_slot = slot;
    treasury.last_balance_change_slot = slot;

    store_state(ledger_info, &ledger)?;
    store_state(treasury_info, &treasury)
}

pub(super) fn process_initialize_oracle_samba_pool(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 10 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let payer_info = &accounts[1];
    let config_info = &accounts[2];
    let major_config_info = &accounts[3];
    let amba_mint_info = &accounts[4];
    let staking_pool_info = &accounts[5];
    let samba_mint_info = &accounts[6];
    let samba_vote_vault_info = &accounts[7];
    let token_program_info = &accounts[8];
    let system_program_info = &accounts[9];
    let config =
        validate_current_oracle_authority_allow_paused(program_id, authority_info, config_info)?;
    validate_current_account_creation_payer(payer_info)?;
    validate_system_program(system_program_info)?;
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    let major_config =
        load_separated_oracle_major_token_config(program_id, major_config_info, &config)?;
    if major_config.mint != *amba_mint_info.key {
        return Err(VaultError::InvalidOracleMajorTokenConfig.into());
    }
    let amba_mint = validate_mint_account(amba_mint_info, token_program_info.key)?;
    let (expected_pool, pool_bump) = derive_oracle_staking_pool_pda(program_id);
    let (expected_samba_mint, samba_mint_bump) = derive_oracle_samba_mint_pda(program_id);
    let (expected_vote_vault, vote_vault_bump) = derive_oracle_samba_vote_vault_pda(program_id);
    if *staking_pool_info.key != expected_pool
        || *samba_mint_info.key != expected_samba_mint
        || *samba_vote_vault_info.key != expected_vote_vault
    {
        return Err(VaultError::InvalidSambaMint.into());
    }
    if staking_pool_info.owner == program_id
        || samba_mint_info.owner == token_program_info.key
        || samba_vote_vault_info.owner == token_program_info.key
    {
        return Err(VaultError::AlreadyInitialized.into());
    }

    create_program_account(
        payer_info,
        samba_mint_info,
        system_program_info,
        token_program_info.key,
        Mint::LEN,
        &[ORACLE_SAMBA_MINT_PDA_SEED, &[samba_mint_bump]],
    )?;
    invoke_token_initialize_mint2(
        token_program_info,
        samba_mint_info,
        config_info.key,
        None,
        amba_mint.decimals,
    )?;

    create_program_account(
        payer_info,
        samba_vote_vault_info,
        system_program_info,
        token_program_info.key,
        TokenAccount::LEN,
        &[ORACLE_SAMBA_VOTE_VAULT_PDA_SEED, &[vote_vault_bump]],
    )?;
    invoke_token_initialize_account3(
        token_program_info,
        samba_vote_vault_info,
        samba_mint_info,
        config_info.key,
    )?;

    create_program_account(
        payer_info,
        staking_pool_info,
        system_program_info,
        program_id,
        OracleStakingPool::LEN,
        &[ORACLE_STAKING_POOL_PDA_SEED, &[pool_bump]],
    )?;
    let pool = OracleStakingPool {
        is_initialized: true,
        bump: pool_bump,
        account_discriminator: OracleStakingPool::ACCOUNT_DISCRIMINATOR,
        account_version: OracleStakingPool::ACCOUNT_VERSION,
        major_token_config: *major_config_info.key,
        samba_mint: *samba_mint_info.key,
        samba_vote_vault: *samba_vote_vault_info.key,
        ..OracleStakingPool::default()
    };
    validate_oracle_staking_pool_accounting(&pool)?;
    store_state(staking_pool_info, &pool)
}

pub(super) fn process_initialize_oracle_reward_funnel(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 9 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let payer_info = &accounts[0];
    let config_info = &accounts[1];
    let major_config_info = &accounts[2];
    let amba_mint_info = &accounts[3];
    let funnel_info = &accounts[4];
    let funnel_token_info = &accounts[5];
    let token_program_info = &accounts[6];
    let associated_token_program_info = &accounts[7];
    let system_program_info = &accounts[8];
    if !payer_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    if *associated_token_program_info.key != crate::associated_token::id() {
        return Err(VaultError::InvalidOracleRewardFunnel.into());
    }
    validate_system_program(system_program_info)?;
    let config = load_current_canonical_vault_config(program_id, config_info)?;
    let major_config =
        load_separated_oracle_major_token_config(program_id, major_config_info, &config)?;
    if major_config.mint != *amba_mint_info.key {
        return Err(VaultError::InvalidOracleMajorTokenConfig.into());
    }
    let amba_mint = validate_mint_account(amba_mint_info, token_program_info.key)?;
    if !amba_mint.is_initialized {
        return Err(VaultError::InvalidMint.into());
    }

    let (expected_funnel, funnel_bump) = derive_oracle_reward_funnel_pda(program_id);
    let expected_funnel_token =
        crate::associated_token::get_associated_token_address_with_program_id(
            &expected_funnel,
            amba_mint_info.key,
            token_program_info.key,
        );
    if *funnel_info.key != expected_funnel || *funnel_token_info.key != expected_funnel_token {
        return Err(VaultError::InvalidOracleRewardFunnel.into());
    }
    if funnel_info.owner == program_id {
        return Err(VaultError::AlreadyInitialized.into());
    }

    create_program_account(
        payer_info,
        funnel_info,
        system_program_info,
        program_id,
        OracleRewardFunnel::LEN,
        &[ORACLE_REWARD_FUNNEL_PDA_SEED, &[funnel_bump]],
    )?;

    // Idempotent creation deliberately supports a valid ATA that an upstream fee router created
    // and funded before the typed funnel state was initialized.
    invoke_create_associated_token_account_idempotent(
        payer_info,
        funnel_token_info,
        funnel_info,
        amba_mint_info,
        system_program_info,
        token_program_info,
        associated_token_program_info,
    )?;
    validate_strict_token_account(
        funnel_token_info,
        amba_mint_info.key,
        funnel_info.key,
        VaultError::InvalidOracleRewardFunnel,
    )?;

    let funnel = OracleRewardFunnel {
        is_initialized: true,
        bump: funnel_bump,
        account_discriminator: OracleRewardFunnel::ACCOUNT_DISCRIMINATOR,
        account_version: OracleRewardFunnel::ACCOUNT_VERSION,
        major_token_config: *major_config_info.key,
        amba_mint: *amba_mint_info.key,
        funnel_token_account: *funnel_token_info.key,
        ..OracleRewardFunnel::default()
    };
    store_state(funnel_info, &funnel)
}

pub(super) fn process_sweep_oracle_reward_funnel(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 11 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let config_info = &accounts[1];
    let major_config_info = &accounts[2];
    let funnel_info = &accounts[3];
    let funnel_token_info = &accounts[4];
    let vault_token_info = &accounts[5];
    let amba_mint_info = &accounts[6];
    let treasury_info = &accounts[7];
    let staking_pool_info = &accounts[8];
    let samba_mint_info = &accounts[9];
    let token_program_info = &accounts[10];
    if !cranker_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    let config = load_current_canonical_vault_config(program_id, config_info)?;
    let major_config =
        load_separated_oracle_major_token_config(program_id, major_config_info, &config)?;
    if major_config.mint != *amba_mint_info.key
        || major_config.vault_token_account != *vault_token_info.key
        || funnel_token_info.key == vault_token_info.key
    {
        return Err(VaultError::InvalidOracleMajorTokenConfig.into());
    }
    let amba_mint = validate_mint_account(amba_mint_info, token_program_info.key)?;
    if !amba_mint.is_initialized {
        return Err(VaultError::InvalidMint.into());
    }
    validate_vault_token_account(vault_token_info, amba_mint_info.key, config_info.key)?;

    let mut funnel = load_canonical_oracle_reward_funnel(
        program_id,
        funnel_info,
        major_config_info.key,
        amba_mint_info.key,
        funnel_token_info.key,
    )?;
    let funnel_token = validate_strict_token_account(
        funnel_token_info,
        amba_mint_info.key,
        funnel_info.key,
        VaultError::InvalidOracleRewardFunnel,
    )?;
    let amount = funnel_token.amount;
    if amount == 0 {
        return Err(VaultError::OracleRewardFunnelEmpty.into());
    }

    let mut staking_pool =
        load_canonical_oracle_staking_pool(program_id, staking_pool_info, major_config_info.key)?;
    let samba_mint = validate_oracle_samba_mint(&staking_pool, samba_mint_info, config_info.key)?;
    if staking_pool.governance_lock_count == 0 {
        synchronize_oracle_samba_supply(&mut staking_pool, samba_mint.supply)?;
    } else if samba_mint.supply > staking_pool.samba_supply {
        return Err(VaultError::InvalidOracleStakingPool.into());
    }
    let staking_active = samba_mint.supply != 0
        && staking_pool.samba_supply != 0
        && staking_pool.active_amba_backing != 0;
    let allocation = calculate_oracle_reward_allocation(amount, staking_active)?;

    let mut treasury = load_valid_oracle_treasury(program_id, treasury_info)?;
    oracle_treasury_protocol_sleeve_total(&treasury)?;
    let slot = Clock::get()?.slot;
    credit_oracle_treasury_sleeves(&mut treasury, allocation)?;
    treasury.major_tokens = treasury
        .major_tokens
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    treasury.last_balance_change_slot = slot;
    credit_oracle_staking_backing(&mut staking_pool, allocation.staking, slot)?;
    staking_pool.last_updated_slot = slot;
    funnel.total_swept = funnel
        .total_swept
        .checked_add(u128::from(amount))
        .ok_or(VaultError::ArithmeticOverflow)?;
    funnel.total_game_funded = funnel
        .total_game_funded
        .checked_add(u128::from(allocation.game))
        .ok_or(VaultError::ArithmeticOverflow)?;
    funnel.total_scramble_funded = funnel
        .total_scramble_funded
        .checked_add(u128::from(allocation.scramble))
        .ok_or(VaultError::ArithmeticOverflow)?;
    funnel.total_challenge_funded = funnel
        .total_challenge_funded
        .checked_add(u128::from(allocation.challenge))
        .ok_or(VaultError::ArithmeticOverflow)?;
    funnel.total_staking_funded = funnel
        .total_staking_funded
        .checked_add(u128::from(allocation.staking))
        .ok_or(VaultError::ArithmeticOverflow)?;
    funnel.total_reserve_funded = funnel
        .total_reserve_funded
        .checked_add(u128::from(allocation.reserve))
        .ok_or(VaultError::ArithmeticOverflow)?;
    funnel.last_updated_slot = slot;
    validate_oracle_staking_pool_accounting(&staking_pool)?;
    oracle_treasury_protocol_sleeve_total(&treasury)?;

    let vault_before = validate_token_account(vault_token_info)?.amount;
    let expected_vault_after = vault_before
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    invoke_token_transfer_checked(
        token_program_info,
        funnel_token_info,
        amba_mint_info,
        vault_token_info,
        funnel_info,
        amount,
        amba_mint.decimals,
        &[&[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_REWARD_FUNNEL_PDA_SEED,
            &[funnel.bump],
        ]],
    )?;

    let funnel_after = validate_strict_token_account(
        funnel_token_info,
        amba_mint_info.key,
        funnel_info.key,
        VaultError::InvalidOracleRewardFunnel,
    )?;
    let vault_after = validate_token_account(vault_token_info)?;
    if funnel_after.amount != 0 || vault_after.amount != expected_vault_after {
        return Err(VaultError::InvalidOracleRewardFunnel.into());
    }

    store_state(treasury_info, &treasury)?;
    store_state(staking_pool_info, &staking_pool)?;
    store_state(funnel_info, &funnel)
}
