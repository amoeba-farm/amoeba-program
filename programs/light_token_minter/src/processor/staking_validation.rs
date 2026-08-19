use super::*;

pub(super) fn load_or_create_oracle_player_ledger<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    ledger_info: &AccountInfo<'a>,
    owner: &Pubkey,
    system_program_info: &AccountInfo<'a>,
) -> Result<OraclePlayerLedger, ProgramError> {
    let (expected, bump) = derive_oracle_player_ledger_pda(program_id, owner);
    if *ledger_info.key != expected {
        return Err(VaultError::InvalidOraclePlayerLedger.into());
    }
    if ledger_info.owner != program_id {
        create_program_account(
            payer_info,
            ledger_info,
            system_program_info,
            program_id,
            OraclePlayerLedger::LEN,
            &[ORACLE_PLAYER_LEDGER_PDA_SEED, owner.as_ref(), &[bump]],
        )?;
        return Ok(OraclePlayerLedger {
            is_initialized: true,
            bump,
            owner: *owner,
            ..OraclePlayerLedger::default()
        });
    }
    load_canonical_oracle_player_ledger(program_id, ledger_info, owner)
}

pub(super) fn load_or_create_oracle_unstake_request<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    request_info: &AccountInfo<'a>,
    owner: &Pubkey,
    system_program_info: &AccountInfo<'a>,
) -> Result<OracleUnstakeRequest, ProgramError> {
    let (expected, bump) = derive_oracle_unstake_request_pda(program_id, owner);
    if *request_info.key != expected {
        return Err(VaultError::InvalidOracleUnstakeRequest.into());
    }
    if request_info.owner == program_id {
        return load_canonical_oracle_unstake_request(program_id, request_info, owner);
    }
    create_program_account(
        payer_info,
        request_info,
        system_program_info,
        program_id,
        OracleUnstakeRequest::LEN,
        &[ORACLE_UNSTAKE_REQUEST_PDA_SEED, owner.as_ref(), &[bump]],
    )?;
    Ok(OracleUnstakeRequest {
        is_initialized: true,
        bump,
        account_discriminator: OracleUnstakeRequest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUnstakeRequest::ACCOUNT_VERSION,
        owner: *owner,
        ..OracleUnstakeRequest::default()
    })
}

pub(super) fn load_or_create_oracle_stake_activation<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    activation_info: &AccountInfo<'a>,
    owner: &Pubkey,
    system_program_info: &AccountInfo<'a>,
) -> Result<OracleStakeActivation, ProgramError> {
    let (expected, bump) = derive_oracle_stake_activation_pda(program_id, owner);
    if *activation_info.key != expected {
        return Err(VaultError::InvalidOracleStakeActivation.into());
    }
    if activation_info.owner == program_id {
        return load_canonical_oracle_stake_activation(program_id, activation_info, owner);
    }
    create_program_account(
        payer_info,
        activation_info,
        system_program_info,
        program_id,
        OracleStakeActivation::LEN,
        &[ORACLE_STAKE_ACTIVATION_PDA_SEED, owner.as_ref(), &[bump]],
    )?;
    Ok(OracleStakeActivation {
        is_initialized: true,
        bump,
        account_discriminator: OracleStakeActivation::ACCOUNT_DISCRIMINATOR,
        account_version: OracleStakeActivation::ACCOUNT_VERSION,
        owner: *owner,
        ..OracleStakeActivation::default()
    })
}

pub(super) fn load_or_create_oracle_treasury<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    treasury_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
) -> Result<OracleTreasuryState, ProgramError> {
    let (expected, bump) = derive_oracle_treasury_pda(program_id);
    if *treasury_info.key != expected {
        return Err(VaultError::InvalidOracleTreasury.into());
    }
    if treasury_info.owner != program_id {
        create_program_account(
            payer_info,
            treasury_info,
            system_program_info,
            program_id,
            OracleTreasuryState::LEN,
            &[ORACLE_TREASURY_PDA_SEED, &[bump]],
        )?;
        return Ok(OracleTreasuryState {
            is_initialized: true,
            bump,
            ..OracleTreasuryState::default()
        });
    }
    load_valid_oracle_treasury(program_id, treasury_info)
}

pub(super) fn load_active_oracle_vault_config(
    program_id: &Pubkey,
    config_info: &AccountInfo,
) -> Result<VaultConfig, ProgramError> {
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.paused {
        return Err(VaultError::ContractPaused.into());
    }
    Ok(config)
}

pub(super) fn load_current_canonical_vault_config(
    program_id: &Pubkey,
    config_info: &AccountInfo,
) -> Result<VaultConfig, ProgramError> {
    if config_info.data_len() != VaultConfig::LEN {
        return Err(VaultError::InvalidConfigAccount.into());
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if !config.has_current_layout() {
        return Err(VaultError::InvalidConfigAccount.into());
    }
    Ok(config)
}

pub(super) fn load_valid_oracle_major_token_config(
    program_id: &Pubkey,
    major_config_info: &AccountInfo,
) -> Result<OracleMajorTokenConfig, ProgramError> {
    let (expected, expected_bump) = derive_oracle_major_token_config_pda(program_id);
    if *major_config_info.key != expected {
        return Err(VaultError::InvalidOracleMajorTokenConfig.into());
    }
    let major_config: OracleMajorTokenConfig = load_exact_zero_padded_state(
        major_config_info,
        program_id,
        OracleMajorTokenConfig::LEN,
        VaultError::InvalidOracleMajorTokenConfig,
    )?;
    if !major_config.is_initialized || major_config.bump != expected_bump {
        return Err(VaultError::InvalidOracleMajorTokenConfig.into());
    }
    Ok(major_config)
}

/// Loads the canonical AMBA configuration and proves that its physical custody cannot alias the
/// primary collateral domain. Repeating this invariant at every live AMBA entry point quarantines
/// a configuration mismatch instead of trusting that initialization was performed correctly.
pub(super) fn load_separated_oracle_major_token_config(
    program_id: &Pubkey,
    major_config_info: &AccountInfo,
    vault_config: &VaultConfig,
) -> Result<OracleMajorTokenConfig, ProgramError> {
    let major_config = load_valid_oracle_major_token_config(program_id, major_config_info)?;
    if major_config.mint == vault_config.usdc_mint
        || major_config.vault_token_account == vault_config.vault_token_account
    {
        return Err(VaultError::InvalidOracleMajorTokenConfig.into());
    }
    Ok(major_config)
}

pub(super) fn validate_oracle_major_token_accounts(
    major_config: &OracleMajorTokenConfig,
    expected_owner: &Pubkey,
    user_token_info: &AccountInfo,
    vault_token_info: &AccountInfo,
    mint_info: &AccountInfo,
    config_key: &Pubkey,
) -> ProgramResult {
    if *mint_info.key != major_config.mint
        || *vault_token_info.key != major_config.vault_token_account
    {
        return Err(VaultError::InvalidOracleMajorTokenConfig.into());
    }
    let user_token = validate_token_account(user_token_info)?;
    if user_token.mint != major_config.mint || user_token.owner != *expected_owner {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    validate_vault_token_account(vault_token_info, &major_config.mint, config_key)?;
    Ok(())
}

pub(super) fn debit_oracle_available(
    ledger: &mut OraclePlayerLedger,
    amount: u64,
) -> ProgramResult {
    if ledger.major_tokens < amount {
        return Err(VaultError::InsufficientAvailableCollateral.into());
    }
    ledger.major_tokens = ledger
        .major_tokens
        .checked_sub(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    ledger.last_updated_slot = Clock::get()?.slot;
    Ok(())
}

pub(super) fn credit_oracle_treasury_sleeves(
    treasury: &mut OracleTreasuryState,
    allocation: OracleRewardAllocation,
) -> ProgramResult {
    treasury.game_pool = treasury
        .game_pool
        .checked_add(allocation.game)
        .ok_or(VaultError::ArithmeticOverflow)?;
    treasury.scramble_pool = treasury
        .scramble_pool
        .checked_add(allocation.scramble)
        .ok_or(VaultError::ArithmeticOverflow)?;
    treasury.challenge_pool = treasury
        .challenge_pool
        .checked_add(allocation.challenge)
        .ok_or(VaultError::ArithmeticOverflow)?;
    treasury.reserve_pool = treasury
        .reserve_pool
        .checked_add(allocation.reserve)
        .ok_or(VaultError::ArithmeticOverflow)?;
    Ok(())
}

pub(super) fn credit_oracle_staking_backing(
    pool: &mut OracleStakingPool,
    amount: u64,
    slot: u64,
) -> ProgramResult {
    if amount == 0 {
        return Ok(());
    }
    if pool.samba_supply == 0 || pool.active_amba_backing == 0 {
        return Err(VaultError::InvalidOracleStakingPool.into());
    }
    pool.active_amba_backing = pool
        .active_amba_backing
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    // This is lifetime observability only; backing and supply are the consensus accounting.
    // Saturate rather than permanently disabling valid sweeps after enough token recirculation.
    pool.total_rewards_funded = pool.total_rewards_funded.saturating_add(amount);
    pool.last_updated_slot = slot;
    validate_oracle_staking_pool_accounting(pool)
}

pub(super) fn validate_oracle_staking_pool_accounting(pool: &OracleStakingPool) -> ProgramResult {
    if (pool.samba_supply == 0) != (pool.active_amba_backing == 0) {
        return Err(VaultError::InvalidOracleStakingPool.into());
    }
    Ok(())
}

pub(super) fn synchronize_oracle_samba_supply(
    pool: &mut OracleStakingPool,
    live_samba_supply: u64,
) -> ProgramResult {
    if live_samba_supply > pool.samba_supply {
        return Err(VaultError::InvalidOracleStakingPool.into());
    }
    if live_samba_supply == pool.samba_supply {
        return Ok(());
    }
    if pool.governance_lock_count != 0 {
        return Err(VaultError::SambaSupplyLockedForVoting.into());
    }
    // Classic SPL holders may burn their own receipt tokens without invoking this program.
    // A partial burn donates value to the remaining shares. If every share is burned, quarantine
    // that generation's backing so the first staker in a later generation cannot capture it.
    if live_samba_supply == 0 {
        pool.orphaned_amba_backing = pool
            .orphaned_amba_backing
            .checked_add(pool.active_amba_backing)
            .ok_or(VaultError::ArithmeticOverflow)?;
        pool.active_amba_backing = 0;
    }
    pool.samba_supply = live_samba_supply;
    validate_oracle_staking_pool_accounting(pool)
}

pub(super) fn prepare_oracle_samba_voting_snapshot(
    pool: &mut OracleStakingPool,
    live_samba_supply: u64,
    current_slot: u64,
) -> Result<u64, ProgramError> {
    if pool.governance_lock_count == 0 {
        synchronize_oracle_samba_supply(pool, live_samba_supply)?;
    } else if live_samba_supply > pool.samba_supply {
        return Err(VaultError::InvalidOracleStakingPool.into());
    }
    if live_samba_supply == 0 || pool.samba_supply == 0 || pool.active_amba_backing == 0 {
        return Err(VaultError::InsufficientSambaTokens.into());
    }
    pool.governance_lock_count = pool
        .governance_lock_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    pool.last_updated_slot = current_slot;
    Ok(pool.samba_supply)
}

pub(super) fn calculate_samba_shares_for_amba(
    pool: &OracleStakingPool,
    amba_amount: u64,
) -> Result<u64, ProgramError> {
    validate_oracle_staking_pool_accounting(pool)?;
    let shares = if pool.samba_supply == 0 {
        amba_amount
    } else {
        let shares = u128::from(amba_amount)
            .checked_mul(u128::from(pool.samba_supply))
            .ok_or(VaultError::ArithmeticOverflow)?
            .checked_div(u128::from(pool.active_amba_backing))
            .ok_or(VaultError::InvalidSambaExchangeRate)?;
        u64::try_from(shares).map_err(|_| VaultError::ArithmeticOverflow)?
    };
    if shares == 0 {
        return Err(VaultError::InvalidSambaExchangeRate.into());
    }
    Ok(shares)
}

pub(super) fn calculate_amba_for_samba_shares(
    pool: &OracleStakingPool,
    samba_amount: u64,
) -> Result<u64, ProgramError> {
    validate_oracle_staking_pool_accounting(pool)?;
    if samba_amount == 0 || samba_amount > pool.samba_supply {
        return Err(VaultError::InsufficientSambaTokens.into());
    }
    let amba = if samba_amount == pool.samba_supply {
        // Give the final redeemed share all remaining active backing so rounding cannot strand dust.
        pool.active_amba_backing
    } else {
        let amount = u128::from(samba_amount)
            .checked_mul(u128::from(pool.active_amba_backing))
            .ok_or(VaultError::ArithmeticOverflow)?
            .checked_div(u128::from(pool.samba_supply))
            .ok_or(VaultError::InvalidSambaExchangeRate)?;
        u64::try_from(amount).map_err(|_| VaultError::ArithmeticOverflow)?
    };
    if amba == 0 {
        return Err(VaultError::InvalidSambaExchangeRate.into());
    }
    Ok(amba)
}

pub(super) fn ensure_oracle_unstake_ready_at(
    request: &OracleUnstakeRequest,
    current_unix_timestamp: u64,
) -> ProgramResult {
    if request.pending_amba == 0 || request.claimable_at_ts == 0 {
        return Err(VaultError::InvalidOracleUnstakeRequest.into());
    }
    if current_unix_timestamp < request.claimable_at_ts {
        return Err(VaultError::UnstakeCooldownActive.into());
    }
    Ok(())
}

pub(super) fn ensure_oracle_stake_activation_ready_at(
    activation: &OracleStakeActivation,
    current_unix_timestamp: u64,
) -> ProgramResult {
    if activation.queued_amba == 0 || activation.activate_after_ts == 0 {
        return Err(VaultError::InvalidOracleStakeActivation.into());
    }
    if current_unix_timestamp < activation.activate_after_ts {
        return Err(VaultError::StakeActivationCooldownActive.into());
    }
    Ok(())
}

pub(super) fn validate_oracle_samba_mint(
    pool: &OracleStakingPool,
    mint_info: &AccountInfo,
    config_key: &Pubkey,
) -> Result<Mint, ProgramError> {
    if *mint_info.key != pool.samba_mint {
        return Err(VaultError::InvalidSambaMint.into());
    }
    let mint = validate_mint_account(mint_info, &spl_token_program_id())?;
    if !mint.is_initialized
        || mint.mint_authority != COption::Some(*config_key)
        || mint.freeze_authority != COption::None
    {
        return Err(VaultError::InvalidSambaMint.into());
    }
    Ok(mint)
}

pub(super) fn validate_owner_samba_token_account(
    token_info: &AccountInfo,
    owner: &Pubkey,
    samba_mint: &Pubkey,
) -> Result<TokenAccount, ProgramError> {
    let token = validate_token_account(token_info)?;
    if token.state != AccountState::Initialized
        || token.owner != *owner
        || token.mint != *samba_mint
        || token.is_native != COption::None
    {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    Ok(token)
}

pub(super) fn oracle_treasury_protocol_sleeve_total(
    treasury: &OracleTreasuryState,
) -> Result<u64, ProgramError> {
    let sleeve_total = treasury
        .game_pool
        .checked_add(treasury.scramble_pool)
        .and_then(|value| value.checked_add(treasury.challenge_pool))
        .and_then(|value| value.checked_add(treasury.reserve_pool))
        .ok_or(VaultError::ArithmeticOverflow)?;
    if sleeve_total > treasury.major_tokens {
        return Err(VaultError::InvalidOracleTreasury.into());
    }
    Ok(sleeve_total)
}

pub(super) fn mul_bps(amount: u64, bps: u16) -> Result<u64, ProgramError> {
    let value = (amount as u128)
        .checked_mul(bps as u128)
        .ok_or(VaultError::ArithmeticOverflow)?
        / 10_000;
    u64::try_from(value).map_err(|_| VaultError::ArithmeticOverflow.into())
}

pub(super) fn source_delta_bps(
    opening_state: u64,
    current_state: u64,
) -> Result<i64, ProgramError> {
    if opening_state == 0 || current_state == 0 {
        return Err(VaultError::InvalidOracleState.into());
    }
    let difference = current_state as i128 - opening_state as i128;
    let delta = (difference * 10_000) / opening_state as i128;
    i64::try_from(delta).map_err(|_| VaultError::ArithmeticOverflow.into())
}

pub(super) fn oracle_settlement_status(
    c_raw_bps: u16,
    g_camo_bps: u16,
    g_thin_bps: u16,
) -> (u16, OracleSettlementStatus) {
    let guard = g_camo_bps.max(g_thin_bps).min(10_000) as u32;
    let c_raw = c_raw_bps.min(10_000) as u32;
    let c_settle = ((c_raw * (10_000 - guard)) / 10_000) as u16;
    let status = if c_settle >= 5_500 {
        OracleSettlementStatus::Final
    } else if c_settle >= 3_500 {
        OracleSettlementStatus::Provisional
    } else {
        OracleSettlementStatus::FrozenPendingEvidence
    };
    (c_settle, status)
}
