use super::*;

pub(in crate::processor) fn ensure_oracle_usdc_schedule_build_window(
    market: &Market,
    month: &OracleMonthState,
) -> ProgramResult {
    ensure_oracle_usdc_schedule_build_window_at(
        market.instrument.expiry_ts,
        month,
        current_unix_timestamp()?,
    )
}

pub(in crate::processor) fn ensure_oracle_usdc_schedule_build_window_at(
    expiry_ts: u64,
    month: &OracleMonthState,
    now: u64,
) -> ProgramResult {
    let (placement_end, _, _) = rulebook_schedule_boundaries(month)?;
    match month.phase {
        OraclePhase::Scramble => {
            if now >= placement_end {
                return Err(VaultError::OracleTimingWindowClosed.into());
            }
        }
        OraclePhase::SourceSubmission => {
            // On-time completed coverage returns to Scramble and retains the
            // planned placement deadline. A late bootstrap or incomplete SKU
            // set may build its finite reward schedule only while immutable
            // expiry can still fit Challenge, Resolution, and Opening in full.
            if now >= oracle_source_submission_latest_safe_ts(expiry_ts)? {
                return Err(VaultError::OracleTimingWindowClosed.into());
            }
        }
        _ => return Err(VaultError::InvalidOraclePhase.into()),
    }
    Ok(())
}

pub(in crate::processor) fn process_begin_oracle_usdc_reward_schedule(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let config_info = &accounts[1];
    let market_info = &accounts[2];
    let month_info = &accounts[3];
    let reward_vault_info = &accounts[4];
    let schedule_info = &accounts[5];
    let system_program_info = &accounts[6];
    if !authority_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_system_program(system_program_info)?;
    let config = load_current_canonical_vault_config(program_id, config_info)?;
    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_usdc_schedule_build_window(&market, &month)?;
    if *authority_info.key != month.authority {
        return Err(VaultError::Unauthorized.into());
    }
    let reward_vault = load_oracle_usdc_reward_vault(program_id, reward_vault_info)?;
    if reward_vault.mint != config.usdc_mint {
        return Err(VaultError::InvalidOracleUsdcRewardVault.into());
    }
    let (expected, bump) = derive_oracle_usdc_reward_schedule_pda(program_id, month_info.key);
    if *schedule_info.key != expected {
        return Err(VaultError::InvalidOracleUsdcRewardSchedule.into());
    }
    validate_create_only_program_account_target(program_id, schedule_info)?;
    create_program_account(
        authority_info,
        schedule_info,
        system_program_info,
        program_id,
        OracleUsdcRewardSchedule::LEN,
        &[
            ORACLE_USDC_REWARD_SCHEDULE_PDA_SEED,
            month_info.key.as_ref(),
            &[bump],
        ],
    )?;
    let slot = Clock::get()?.slot;
    let schedule = OracleUsdcRewardSchedule {
        is_initialized: true,
        bump,
        account_discriminator: OracleUsdcRewardSchedule::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUsdcRewardSchedule::ACCOUNT_VERSION,
        month: *month_info.key,
        authority: *authority_info.key,
        reward_vault: *reward_vault_info.key,
        phase: OracleUsdcRewardSchedulePhase::Building,
        sku_pool_count: 0,
        registered_source_count: 0,
        registered_opening_count: 0,
        registered_update_count: 0,
        total_reward_budget: 0,
        remaining_reward_budget: 0,
        last_updated_slot: slot,
        outstanding_prelisting_escrow_count: 0,
        registered_update_reward_units: 0,
        trading_fee_bounty_total: 0,
        bounty_fee_sweep_finalized: false,
    };
    month.last_updated_slot = slot;
    store_state(schedule_info, &schedule)?;
    store_oracle_month_state(month_info, &month)
}

pub(in crate::processor) fn process_add_oracle_usdc_sku_budget(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: AddOracleUsdcSkuBudgetParams,
) -> ProgramResult {
    let sku_reward_budget = params
        .source_reward_budget
        .checked_add(params.opening_reward_budget)
        .and_then(|value| value.checked_add(params.update_reward_budget))
        .ok_or(VaultError::ArithmeticOverflow)?;
    if crate::bytes32_is_zero(&params.bucket_id)
        || sku_reward_budget == 0
        || params.proposer_reward_bps == 0
        || params.proposer_reward_bps > 10_000
        || params.listing_bond == 0
        || params.support_bond == 0
        || params.opening_bond == 0
        || params.update_min_bond == 0
        || params.challenge_min_bond == 0
        || params.challenge_max_bond == 0
        || params.challenge_min_bond > params.challenge_max_bond
        || params.challenge_bond_bps > 10_000
    {
        return Err(VaultError::InvalidOracleUsdcSkuPool.into());
    }
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let config_info = &accounts[1];
    let market_info = &accounts[2];
    let month_info = &accounts[3];
    let schedule_info = &accounts[4];
    let sku_info = &accounts[5];
    let system_program_info = &accounts[6];
    if !authority_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_system_program(system_program_info)?;
    let _config = load_current_canonical_vault_config(program_id, config_info)?;
    let (market, month) = load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_usdc_schedule_build_window(&market, &month)?;
    let mut schedule = load_oracle_usdc_reward_schedule(program_id, month_info.key, schedule_info)?;
    if *authority_info.key != month.authority
        || schedule.authority != *authority_info.key
        || schedule.phase != OracleUsdcRewardSchedulePhase::Building
    {
        return Err(VaultError::OracleUsdcRewardScheduleFrozen.into());
    }
    let (expected, bump) =
        derive_oracle_usdc_sku_pool_pda(program_id, schedule_info.key, &params.bucket_id);
    if *sku_info.key != expected {
        return Err(VaultError::InvalidOracleUsdcSkuPool.into());
    }
    validate_create_only_program_account_target(program_id, sku_info)?;
    create_program_account(
        authority_info,
        sku_info,
        system_program_info,
        program_id,
        OracleUsdcSkuPool::LEN,
        &[
            ORACLE_USDC_SKU_POOL_PDA_SEED,
            schedule_info.key.as_ref(),
            &params.bucket_id,
            &[bump],
        ],
    )?;
    let slot = Clock::get()?.slot;
    let sku = OracleUsdcSkuPool {
        is_initialized: true,
        bump,
        account_discriminator: OracleUsdcSkuPool::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUsdcSkuPool::ACCOUNT_VERSION,
        schedule: *schedule_info.key,
        month: *month_info.key,
        bucket_id: params.bucket_id,
        source_reward_budget: params.source_reward_budget,
        remaining_source_reward_budget: params.source_reward_budget,
        opening_reward_budget: params.opening_reward_budget,
        remaining_opening_reward_budget: params.opening_reward_budget,
        update_reward_budget: params.update_reward_budget,
        remaining_update_reward_budget: params.update_reward_budget,
        proposer_reward_bps: params.proposer_reward_bps,
        listing_bond: params.listing_bond,
        support_bond: params.support_bond,
        opening_bond: params.opening_bond,
        update_min_bond: params.update_min_bond,
        challenge_min_bond: params.challenge_min_bond,
        challenge_max_bond: params.challenge_max_bond,
        challenge_bond_bps: params.challenge_bond_bps,
        registered_source_count: 0,
        registered_opening_count: 0,
        registered_update_count: 0,
        last_updated_slot: slot,
        registered_update_reward_units: 0,
    };
    schedule.total_reward_budget = schedule
        .total_reward_budget
        .checked_add(
            sku.total_reward_budget()
                .ok_or(VaultError::ArithmeticOverflow)?,
        )
        .ok_or(VaultError::ArithmeticOverflow)?;
    schedule.sku_pool_count = schedule
        .sku_pool_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    schedule.last_updated_slot = slot;
    store_state(sku_info, &sku)?;
    store_state(schedule_info, &schedule)
}

pub(in crate::processor) fn process_finalize_oracle_usdc_reward_schedule(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 9 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let config_info = &accounts[1];
    let market_info = &accounts[2];
    let month_info = &accounts[3];
    let mint_info = &accounts[4];
    let reward_vault_info = &accounts[5];
    let reward_token_info = &accounts[6];
    let schedule_info = &accounts[7];
    let token_program_info = &accounts[8];
    if !authority_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_current_canonical_vault_config(program_id, config_info)?;
    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_usdc_schedule_build_window(&market, &month)?;
    let mut vault = load_oracle_usdc_reward_vault(program_id, reward_vault_info)?;
    let mut schedule = load_oracle_usdc_reward_schedule(program_id, month_info.key, schedule_info)?;
    if *authority_info.key != month.authority
        || schedule.authority != *authority_info.key
        || schedule.phase != OracleUsdcRewardSchedulePhase::Building
        || schedule.sku_pool_count == 0
        || schedule.total_reward_budget == 0
        || vault.mint != config.usdc_mint
    {
        return Err(VaultError::InvalidOracleUsdcRewardSchedule.into());
    }
    let (reward_token, _) = validate_oracle_usdc_reward_custody(
        &vault,
        reward_vault_info.key,
        reward_token_info,
        mint_info,
        token_program_info,
    )?;
    let required_custody = vault
        .total_reserved
        .checked_add(schedule.total_reward_budget)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if reward_token.amount < required_custody {
        return Err(VaultError::OracleUsdcRewardScheduleUnderfunded.into());
    }
    let slot = Clock::get()?.slot;
    vault.total_reserved = required_custody;
    vault.last_updated_slot = slot;
    schedule.phase = OracleUsdcRewardSchedulePhase::Funded;
    schedule.remaining_reward_budget = schedule.total_reward_budget;
    schedule.last_updated_slot = slot;
    month.last_updated_slot = slot;
    store_state(reward_vault_info, &vault)?;
    store_state(schedule_info, &schedule)?;
    store_oracle_month_state(month_info, &month)
}
