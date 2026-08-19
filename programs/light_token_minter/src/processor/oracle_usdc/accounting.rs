use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::processor) struct OracleUsdcSourceRewardAllocation {
    pub source_reward_budget: u64,
    pub proposer_reward: u64,
    pub supporter_reward_budget: u64,
}

/// Returns the equal floor share for every eligible recipient. The unallocated
/// remainder stays in the schedule's explicit dust liability; no transaction or
/// registration order can select a larger share.
pub(in crate::processor) fn calculate_oracle_usdc_equal_share(
    budget: u64,
    recipient_count: u32,
) -> Result<u64, ProgramError> {
    if recipient_count == 0 {
        return Err(VaultError::InvalidOracleState.into());
    }
    Ok(budget / u64::from(recipient_count))
}

/// Splits one SKU's finite source-reward budget after accepted-source finality.
/// With no eligible supporter, the proposer receives the complete source
/// allocation. Otherwise the proposer receives the frozen protected percentage
/// and supporters divide the remainder equally.
pub(in crate::processor) fn calculate_oracle_usdc_source_reward_allocation(
    sku_source_reward_budget: u64,
    accepted_source_count: u32,
    proposer_reward_bps: u16,
    eligible_supporter_count: u32,
) -> Result<OracleUsdcSourceRewardAllocation, ProgramError> {
    if proposer_reward_bps == 0 || proposer_reward_bps > 10_000 {
        return Err(VaultError::InvalidOracleState.into());
    }
    let source_reward_budget =
        calculate_oracle_usdc_equal_share(sku_source_reward_budget, accepted_source_count)?;
    if eligible_supporter_count == 0 {
        return Ok(OracleUsdcSourceRewardAllocation {
            source_reward_budget,
            proposer_reward: source_reward_budget,
            supporter_reward_budget: 0,
        });
    }
    let proposer_reward_u128 = u128::from(source_reward_budget)
        .checked_mul(u128::from(proposer_reward_bps))
        .ok_or(VaultError::ArithmeticOverflow)?
        / 10_000;
    let proposer_reward =
        u64::try_from(proposer_reward_u128).map_err(|_| VaultError::ArithmeticOverflow)?;
    let supporter_reward_budget = source_reward_budget
        .checked_sub(proposer_reward)
        .ok_or(VaultError::ArithmeticOverflow)?;
    Ok(OracleUsdcSourceRewardAllocation {
        source_reward_budget,
        proposer_reward,
        supporter_reward_budget,
    })
}

pub(in crate::processor) fn calculate_oracle_usdc_supporter_reward(
    supporter_reward_budget: u64,
    eligible_supporter_count: u32,
) -> Result<u64, ProgramError> {
    calculate_oracle_usdc_equal_share(supporter_reward_budget, eligible_supporter_count)
}

/// `clamp(floor(backing * bps / 10_000), minimum, maximum)`.
pub(in crate::processor) fn calculate_oracle_usdc_challenge_bond(
    backing: u64,
    challenge_bond_bps: u16,
    minimum: u64,
    maximum: u64,
) -> Result<u64, ProgramError> {
    if backing == 0
        || challenge_bond_bps > 10_000
        || minimum == 0
        || maximum == 0
        || minimum > maximum
    {
        return Err(VaultError::InvalidOracleState.into());
    }
    let proportional = u64::try_from(
        u128::from(backing)
            .checked_mul(u128::from(challenge_bond_bps))
            .ok_or(VaultError::ArithmeticOverflow)?
            / 10_000,
    )
    .map_err(|_| VaultError::ArithmeticOverflow)?;
    Ok(proportional.max(minimum).min(maximum))
}

pub(in crate::processor) fn oracle_source_challenge_bond_backing(
    primary_backing: u64,
    comparison_backing: Option<u64>,
) -> u64 {
    comparison_backing
        .map(|backing| primary_backing.max(backing))
        .unwrap_or(primary_backing)
}

/// Returns both the winning principal and losing bond to the canonical winner.
/// This is a closed, zero-sum ordinary challenge; it never draws from the
/// reward schedule.

pub(in crate::processor) fn debit_oracle_usdc_available(
    collateral: &mut UserCollateral,
    exact_bond: u64,
) -> ProgramResult {
    if exact_bond == 0 || collateral.available_balance < exact_bond {
        return Err(VaultError::InsufficientAvailableCollateral.into());
    }
    collateral.available_balance = collateral
        .available_balance
        .checked_sub(exact_bond)
        .ok_or(VaultError::ArithmeticOverflow)?;
    collateral.last_action_slot = Clock::get()?.slot;
    Ok(())
}

pub(in crate::processor) fn credit_oracle_usdc_available(
    collateral: &mut UserCollateral,
    amount: u64,
) -> ProgramResult {
    collateral.available_balance = collateral
        .available_balance
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    collateral.last_action_slot = Clock::get()?.slot;
    Ok(())
}

pub(in crate::processor) fn load_oracle_usdc_reward_vault(
    program_id: &Pubkey,
    vault_info: &AccountInfo,
) -> Result<OracleUsdcRewardVault, ProgramError> {
    let (expected, bump) = derive_oracle_usdc_reward_vault_pda(program_id);
    let vault: OracleUsdcRewardVault = load_exact_zero_padded_state(
        vault_info,
        program_id,
        OracleUsdcRewardVault::LEN,
        VaultError::InvalidOracleUsdcRewardVault,
    )?;
    if *vault_info.key != expected
        || !vault.is_initialized
        || vault.bump != bump
        || vault.account_discriminator != OracleUsdcRewardVault::ACCOUNT_DISCRIMINATOR
        || vault.account_version != OracleUsdcRewardVault::ACCOUNT_VERSION
        || crate::pubkey_is_default(&vault.mint)
        || crate::pubkey_is_default(&vault.token_account)
    {
        return Err(VaultError::InvalidOracleUsdcRewardVault.into());
    }
    Ok(vault)
}

pub(in crate::processor) fn load_oracle_usdc_reward_schedule(
    program_id: &Pubkey,
    month: &Pubkey,
    schedule_info: &AccountInfo,
) -> Result<OracleUsdcRewardSchedule, ProgramError> {
    let (expected, bump) = derive_oracle_usdc_reward_schedule_pda(program_id, month);
    let schedule: OracleUsdcRewardSchedule = load_exact_zero_padded_state(
        schedule_info,
        program_id,
        OracleUsdcRewardSchedule::LEN,
        VaultError::InvalidOracleUsdcRewardSchedule,
    )?;
    if *schedule_info.key != expected
        || !schedule.is_initialized
        || schedule.bump != bump
        || schedule.account_discriminator != OracleUsdcRewardSchedule::ACCOUNT_DISCRIMINATOR
        || schedule.account_version != OracleUsdcRewardSchedule::ACCOUNT_VERSION
        || schedule.month != *month
        || crate::pubkey_is_default(&schedule.authority)
        || schedule.reward_vault != derive_oracle_usdc_reward_vault_pda(program_id).0
    {
        return Err(VaultError::InvalidOracleUsdcRewardSchedule.into());
    }
    Ok(schedule)
}

pub(in crate::processor) fn load_oracle_usdc_sku_pool(
    program_id: &Pubkey,
    schedule: &Pubkey,
    month: &Pubkey,
    sku_info: &AccountInfo,
) -> Result<OracleUsdcSkuPool, ProgramError> {
    let sku: OracleUsdcSkuPool = load_exact_zero_padded_state(
        sku_info,
        program_id,
        OracleUsdcSkuPool::LEN,
        VaultError::InvalidOracleUsdcSkuPool,
    )?;
    let (expected, bump) = derive_oracle_usdc_sku_pool_pda(program_id, schedule, &sku.bucket_id);
    if *sku_info.key != expected
        || !sku.is_initialized
        || sku.bump != bump
        || sku.account_discriminator != OracleUsdcSkuPool::ACCOUNT_DISCRIMINATOR
        || sku.account_version != OracleUsdcSkuPool::ACCOUNT_VERSION
        || sku.schedule != *schedule
        || sku.month != *month
        || crate::bytes32_is_zero(&sku.bucket_id)
        || sku.total_reward_budget().is_none()
    {
        return Err(VaultError::InvalidOracleUsdcSkuPool.into());
    }
    Ok(sku)
}

pub(in crate::processor) fn load_oracle_usdc_source_reward(
    program_id: &Pubkey,
    schedule: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    source_reward_info: &AccountInfo,
) -> Result<OracleUsdcSourceReward, ProgramError> {
    let (expected, bump) = derive_oracle_usdc_source_reward_pda(program_id, schedule, source);
    let reward: OracleUsdcSourceReward = load_exact_zero_padded_state(
        source_reward_info,
        program_id,
        OracleUsdcSourceReward::LEN,
        VaultError::InvalidOracleUsdcSourceReward,
    )?;
    if *source_reward_info.key != expected
        || !reward.is_initialized
        || reward.bump != bump
        || reward.account_discriminator != OracleUsdcSourceReward::ACCOUNT_DISCRIMINATOR
        || reward.account_version != OracleUsdcSourceReward::ACCOUNT_VERSION
        || reward.month != *month
        || reward.schedule != *schedule
        || reward.source != *source
        || crate::bytes32_is_zero(&reward.source_id)
        || crate::pubkey_is_default(&reward.proposer)
    {
        return Err(VaultError::InvalidOracleUsdcSourceReward.into());
    }
    Ok(reward)
}

pub(in crate::processor) fn carry_merged_supporter_rewards(
    challenged_source: &Pubkey,
    challenged_reward: &mut OracleUsdcSourceReward,
    canonical_source: &Pubkey,
    canonical_reward: &mut OracleUsdcSourceReward,
) -> ProgramResult {
    let next_depth = challenged_reward
        .max_merge_depth
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if challenged_source == canonical_source
        || !crate::pubkey_is_default(&challenged_reward.merged_into_source)
        || !crate::pubkey_is_default(&canonical_reward.merged_into_source)
        || challenged_reward.registered
        || canonical_reward.registered
        || next_depth > MAX_ORACLE_USDC_MERGE_DEPTH
    {
        return Err(VaultError::InvalidOracleUsdcSourceReward.into());
    }
    canonical_reward.supporter_count = canonical_reward
        .supporter_count
        .checked_add(challenged_reward.supporter_count)
        .ok_or(VaultError::ArithmeticOverflow)?;
    canonical_reward.max_merge_depth = canonical_reward.max_merge_depth.max(next_depth);
    challenged_reward.merged_into_source = *canonical_source;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn merge_oracle_usdc_source_rewards(
    program_id: &Pubkey,
    month: &Pubkey,
    sku_info: &AccountInfo,
    challenged_source_info: &AccountInfo,
    challenged_source: &OracleSourceState,
    challenged_reward_info: &AccountInfo,
    canonical_source_info: &AccountInfo,
    canonical_source: &OracleSourceState,
    canonical_reward_info: &AccountInfo,
) -> ProgramResult {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let sku = load_oracle_usdc_sku_pool(program_id, &schedule, month, sku_info)?;
    let mut challenged_reward = load_oracle_usdc_source_reward(
        program_id,
        &schedule,
        month,
        challenged_source_info.key,
        challenged_reward_info,
    )?;
    let mut canonical_reward = load_oracle_usdc_source_reward(
        program_id,
        &schedule,
        month,
        canonical_source_info.key,
        canonical_reward_info,
    )?;
    let challenged_supported_principal = u64::from(challenged_reward.supporter_count)
        .checked_mul(sku.support_bond)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let canonical_supported_principal = u64::from(canonical_reward.supporter_count)
        .checked_mul(sku.support_bond)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if challenged_source.status != OracleSourceStatus::Candidate
        || canonical_source.status != OracleSourceStatus::Candidate
        || challenged_source.bucket_id != sku.bucket_id
        || canonical_source.bucket_id != sku.bucket_id
        || challenged_reward.sku_pool != *sku_info.key
        || canonical_reward.sku_pool != *sku_info.key
        || challenged_reward.source_id != challenged_source.source_id
        || canonical_reward.source_id != canonical_source.source_id
        || challenged_reward.proposer != challenged_source.proposer
        || canonical_reward.proposer != canonical_source.proposer
        || challenged_reward.registered
        || canonical_reward.registered
        || challenged_reward.terminal_status != OracleSourceStatus::Candidate
        || canonical_reward.terminal_status != OracleSourceStatus::Candidate
        || challenged_supported_principal != challenged_source.support_stake_total
        || canonical_supported_principal != canonical_source.support_stake_total
    {
        return Err(VaultError::InvalidOracleUsdcSourceReward.into());
    }
    carry_merged_supporter_rewards(
        challenged_source_info.key,
        &mut challenged_reward,
        canonical_source_info.key,
        &mut canonical_reward,
    )?;
    let slot = Clock::get()?.slot;
    challenged_reward.last_updated_slot = slot;
    canonical_reward.last_updated_slot = slot;
    store_state(challenged_reward_info, &challenged_reward)?;
    store_state(canonical_reward_info, &canonical_reward)
}

pub(in crate::processor) fn validate_oracle_usdc_reward_custody(
    vault: &OracleUsdcRewardVault,
    vault_key: &Pubkey,
    vault_token_info: &AccountInfo,
    mint_info: &AccountInfo,
    token_program_info: &AccountInfo,
) -> Result<(TokenAccount, Mint), ProgramError> {
    if *token_program_info.key != spl_token_program_id()
        || *mint_info.key != vault.mint
        || *vault_token_info.key != vault.token_account
    {
        return Err(VaultError::InvalidOracleUsdcRewardVault.into());
    }
    let mint = validate_collateral_mint_account(mint_info, token_program_info.key)?;
    let token = validate_token_account(vault_token_info)?;
    if token.owner != *vault_key
        || token.mint != vault.mint
        || token.state != AccountState::Initialized
        || token.delegate != COption::None
        || token.delegated_amount != 0
        || token.is_native != COption::None
        || token.close_authority != COption::None
    {
        return Err(VaultError::InvalidOracleUsdcRewardVault.into());
    }
    Ok((token, mint))
}

pub(in crate::processor) fn process_initialize_oracle_usdc_reward_vault(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let payer_info = &accounts[0];
    let config_info = &accounts[1];
    let mint_info = &accounts[2];
    let reward_vault_info = &accounts[3];
    let reward_token_info = &accounts[4];
    let token_program_info = &accounts[5];
    let associated_token_program_info = &accounts[6];
    let system_program_info = &accounts[7];
    if !payer_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id()
        || *associated_token_program_info.key != crate::associated_token::id()
    {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    validate_system_program(system_program_info)?;
    let config = load_current_canonical_vault_config(program_id, config_info)?;
    if *mint_info.key != config.usdc_mint {
        return Err(VaultError::InvalidMint.into());
    }
    let _mint = validate_collateral_mint_account(mint_info, token_program_info.key)?;
    let (expected_vault, bump) = derive_oracle_usdc_reward_vault_pda(program_id);
    let expected_token = crate::associated_token::get_associated_token_address_with_program_id(
        &expected_vault,
        mint_info.key,
        token_program_info.key,
    );
    if *reward_vault_info.key != expected_vault || *reward_token_info.key != expected_token {
        return Err(VaultError::InvalidOracleUsdcRewardVault.into());
    }
    validate_create_only_program_account_target(program_id, reward_vault_info)?;
    create_program_account(
        payer_info,
        reward_vault_info,
        system_program_info,
        program_id,
        OracleUsdcRewardVault::LEN,
        &[ORACLE_USDC_REWARD_VAULT_PDA_SEED, &[bump]],
    )?;
    invoke_create_associated_token_account_idempotent(
        payer_info,
        reward_token_info,
        reward_vault_info,
        mint_info,
        system_program_info,
        token_program_info,
        associated_token_program_info,
    )?;
    validate_vault_token_account(reward_token_info, mint_info.key, reward_vault_info.key)?;
    let vault = OracleUsdcRewardVault {
        is_initialized: true,
        bump,
        account_discriminator: OracleUsdcRewardVault::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUsdcRewardVault::ACCOUNT_VERSION,
        mint: *mint_info.key,
        token_account: *reward_token_info.key,
        total_reserved: 0,
        total_paid: 0,
        last_updated_slot: Clock::get()?.slot,
    };
    store_state(reward_vault_info, &vault)
}

pub(in crate::processor) fn process_deposit_oracle_usdc_rewards(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: DepositOracleUsdcRewardsParams,
) -> ProgramResult {
    if params.amount == 0 {
        return Err(VaultError::AmountMustBePositive.into());
    }
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let funder_info = &accounts[0];
    let config_info = &accounts[1];
    let mint_info = &accounts[2];
    let reward_vault_info = &accounts[3];
    let funder_token_info = &accounts[4];
    let reward_token_info = &accounts[5];
    let token_program_info = &accounts[6];
    if !funder_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_current_canonical_vault_config(program_id, config_info)?;
    let mut vault = load_oracle_usdc_reward_vault(program_id, reward_vault_info)?;
    if vault.mint != config.usdc_mint {
        return Err(VaultError::InvalidOracleUsdcRewardVault.into());
    }
    let (reward_token_before, mint) = validate_oracle_usdc_reward_custody(
        &vault,
        reward_vault_info.key,
        reward_token_info,
        mint_info,
        token_program_info,
    )?;
    validate_vault_token_account(funder_token_info, mint_info.key, funder_info.key)?;
    invoke_token_transfer_checked(
        token_program_info,
        funder_token_info,
        mint_info,
        reward_token_info,
        funder_info,
        params.amount,
        mint.decimals,
        &[],
    )?;
    let expected_after = reward_token_before
        .amount
        .checked_add(params.amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let reward_token_after = validate_token_account(reward_token_info)?;
    if reward_token_after.amount != expected_after {
        return Err(VaultError::InvalidOracleUsdcRewardVault.into());
    }
    vault.last_updated_slot = Clock::get()?.slot;
    store_state(reward_vault_info, &vault)
}
