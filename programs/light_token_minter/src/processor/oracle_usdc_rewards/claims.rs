use super::*;

#[inline(never)]
pub(in crate::processor) fn validate_claim_custody(
    config: &VaultConfig,
    config_info: &AccountInfo,
    reward_vault: &OracleUsdcRewardVault,
    reward_vault_info: &AccountInfo,
    reward_token_info: &AccountInfo,
    primary_vault_token_info: &AccountInfo,
    mint_info: &AccountInfo,
    token_program_info: &AccountInfo,
) -> Result<(TokenAccount, Mint), ProgramError> {
    if *token_program_info.key != spl_token_program_id()
        || reward_vault.mint != config.usdc_mint
        || *mint_info.key != config.usdc_mint
        || reward_vault.token_account != *reward_token_info.key
        || *primary_vault_token_info.key != config.vault_token_account
    {
        return Err(VaultError::InvalidOracleUsdcRewardVault.into());
    }
    let (reward_token, mint) = oracle_usdc::validate_oracle_usdc_reward_custody(
        reward_vault,
        reward_vault_info.key,
        reward_token_info,
        mint_info,
        token_program_info,
    )?;
    validate_vault_token_account(primary_vault_token_info, mint_info.key, config_info.key)?;
    Ok((reward_token, mint))
}

pub(in crate::processor) fn validate_claim_source_family(
    program_id: &Pubkey,
    month: &Pubkey,
    schedule: &Pubkey,
    sku_info: &AccountInfo,
    source_info: &AccountInfo,
    source_reward_info: &AccountInfo,
) -> Result<(OracleUsdcSkuPool, OracleSourceState, OracleUsdcSourceReward), ProgramError> {
    let sku = load_oracle_usdc_sku_pool(program_id, schedule, month, sku_info)?;
    let source = load_valid_oracle_source(program_id, month, source_info)?;
    let source_reward = load_source_reward(
        program_id,
        schedule,
        sku_info.key,
        month,
        source_info.key,
        source_reward_info,
    )?;
    if !source_reward.registered
        || source_reward.source_id != source.source_id
        || source_reward.proposer != source.proposer
        || source_reward.terminal_status != source.status
        || !crate::pubkey_is_default(&source_reward.merged_into_source)
        || source_reward.max_merge_depth > MAX_ORACLE_USDC_MERGE_DEPTH
        || sku.bucket_id != source.bucket_id
        || !matches!(
            source.status,
            OracleSourceStatus::Active | OracleSourceStatus::Inactive
        )
        || sku.registered_source_count == 0
    {
        return Err(VaultError::InvalidOracleUsdcSourceReward.into());
    }
    Ok((sku, source, source_reward))
}

pub(in crate::processor) struct ComputedCashReward {
    subject: Pubkey,
    amount: u64,
    sku: OracleUsdcSkuPool,
}

#[inline(never)]
pub(in crate::processor) fn compute_source_family_reward(
    program_id: &Pubkey,
    month: &Pubkey,
    schedule: &Pubkey,
    recipient: &Pubkey,
    kind: OracleUsdcRewardKind,
    trailing: &[AccountInfo],
) -> Result<ComputedCashReward, ProgramError> {
    let valid_len = match kind {
        OracleUsdcRewardKind::SourceProposer => trailing.len() == 3,
        OracleUsdcRewardKind::SourceSupport => {
            trailing.len() >= 4
                && trailing.len() <= 4 + 2 * usize::from(MAX_ORACLE_USDC_MERGE_DEPTH)
                && (trailing.len() - 4) % 2 == 0
        }
        OracleUsdcRewardKind::Opening => trailing.len() == 4,
        OracleUsdcRewardKind::Update => false,
    };
    if !valid_len {
        return Err(VaultError::InvalidAccountList.into());
    }
    let sku_info = &trailing[0];
    let source_info = &trailing[1];
    let source_reward_info = &trailing[2];
    let (sku, source, source_reward) = validate_claim_source_family(
        program_id,
        month,
        schedule,
        sku_info,
        source_info,
        source_reward_info,
    )?;
    let allocation = calculate_oracle_usdc_source_reward_allocation(
        sku.source_reward_budget,
        sku.registered_source_count,
        sku.proposer_reward_bps,
        source_reward.supporter_count,
    )?;
    let (subject, amount) = match kind {
        OracleUsdcRewardKind::SourceProposer => {
            if *recipient != source.proposer {
                return Err(VaultError::Unauthorized.into());
            }
            (*source_info.key, allocation.proposer_reward)
        }
        OracleUsdcRewardKind::SourceSupport => {
            let support_info = trailing.last().ok_or(VaultError::InvalidAccountList)?;
            let lineage_hops = (trailing.len() - 4) / 2;
            let origin_source_info = if lineage_hops == 0 {
                source_info
            } else {
                &trailing[3]
            };
            let mut origin_source_id = source.source_id;
            for hop in 0..lineage_hops {
                let pair_index = 3 + 2 * hop;
                let lineage_source_info = &trailing[pair_index];
                let lineage_reward_info = &trailing[pair_index + 1];
                if lineage_source_info.key == source_info.key
                    || (0..hop)
                        .any(|prior_hop| trailing[3 + 2 * prior_hop].key == lineage_source_info.key)
                {
                    return Err(VaultError::InvalidOracleUsdcSourceReward.into());
                }
                let lineage_reward = load_source_reward(
                    program_id,
                    schedule,
                    sku_info.key,
                    month,
                    lineage_source_info.key,
                    lineage_reward_info,
                )?;
                let next_source = if hop + 1 == lineage_hops {
                    source_info.key
                } else {
                    trailing[3 + 2 * (hop + 1)].key
                };
                if lineage_reward.registered
                    || lineage_reward.terminal_status != OracleSourceStatus::Candidate
                    || lineage_reward.merged_into_source != *next_source
                    || lineage_reward.max_merge_depth > MAX_ORACLE_USDC_MERGE_DEPTH
                {
                    return Err(VaultError::InvalidOracleUsdcSourceReward.into());
                }
                // A merged reward is retained as classic program-owned state when its source
                // leaf is compacted. Its immutable source identity, SKU membership, proposer,
                // and next hop were all checked by the atomic merge transition; those fields
                // therefore carry the same lineage proof without keeping the source account
                // rent-funded solely for a future supporter claim.
                if hop == 0 {
                    origin_source_id = lineage_reward.source_id;
                }
            }
            if source_reward.max_merge_depth
                < u8::try_from(lineage_hops)
                    .map_err(|_| VaultError::InvalidOracleUsdcSourceReward)?
            {
                return Err(VaultError::InvalidOracleUsdcSourceReward.into());
            }
            let support = load_valid_oracle_support_position(
                program_id,
                month,
                origin_source_info.key,
                &origin_source_id,
                support_info,
                VaultError::InvalidOracleState,
            )?;
            if support.supporter != *recipient || source_reward.supporter_count == 0 {
                return Err(VaultError::InvalidOracleState.into());
            }
            (
                *support_info.key,
                calculate_oracle_usdc_supporter_reward(
                    allocation.supporter_reward_budget,
                    source_reward.supporter_count,
                )?,
            )
        }
        OracleUsdcRewardKind::Opening => {
            let claim_info = &trailing[3];
            let claim = load_valid_oracle_opening_claim(
                program_id,
                month,
                source_info.key,
                claim_info,
                &source,
            )?;
            if source.status != OracleSourceStatus::Active
                || source_reward.opening_claim != *claim_info.key
                || claim.status != OracleOpeningClaimStatus::Accepted
                || claim.claimant != *recipient
                || sku.registered_opening_count == 0
            {
                return Err(VaultError::InvalidOracleUsdcSourceReward.into());
            }
            (
                *claim_info.key,
                calculate_oracle_usdc_equal_share(
                    sku.opening_reward_budget,
                    sku.registered_opening_count,
                )?,
            )
        }
        OracleUsdcRewardKind::Update => return Err(VaultError::InvalidOracleState.into()),
    };
    Ok(ComputedCashReward {
        subject,
        amount,
        sku,
    })
}

#[inline(never)]
pub(in crate::processor) fn compute_update_reward(
    program_id: &Pubkey,
    month: &Pubkey,
    schedule: &Pubkey,
    recipient: &Pubkey,
    trailing: &[AccountInfo],
) -> Result<ComputedCashReward, ProgramError> {
    if trailing.len() != 3 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let sku_info = &trailing[0];
    let registration_info = &trailing[1];
    let claim_info = &trailing[2];
    let sku = load_oracle_usdc_sku_pool(program_id, schedule, month, sku_info)?;
    let registration = load_reward_registration(
        program_id,
        month,
        schedule,
        OracleUsdcRewardKind::Update,
        claim_info.key,
        registration_info,
    )?;
    let claim = load_valid_oracle_update_claim(program_id, month, claim_info)?;
    if registration.sku_pool != *sku_info.key
        || registration.recipient != *recipient
        || claim.claimant != *recipient
        || claim.status != OracleClaimStatus::Finalized
        || sku.registered_update_count == 0
        || sku.registered_update_reward_units == 0
        || registration.reward_units == 0
    {
        return Err(VaultError::InvalidOracleUsdcRewardRegistration.into());
    }
    Ok(ComputedCashReward {
        subject: *claim_info.key,
        amount: u64::try_from(
            u128::from(sku.update_reward_budget)
                .checked_mul(u128::from(registration.reward_units))
                .ok_or(VaultError::ArithmeticOverflow)?
                / u128::from(sku.registered_update_reward_units),
        )
        .map_err(|_| VaultError::ArithmeticOverflow)?,
        sku,
    })
}

#[inline(never)]
pub(in crate::processor) fn decrement_reward_sleeve(
    kind: OracleUsdcRewardKind,
    amount: u64,
    sku: &mut OracleUsdcSkuPool,
) -> ProgramResult {
    let remaining = match kind {
        OracleUsdcRewardKind::SourceProposer | OracleUsdcRewardKind::SourceSupport => {
            &mut sku.remaining_source_reward_budget
        }
        OracleUsdcRewardKind::Opening => &mut sku.remaining_opening_reward_budget,
        OracleUsdcRewardKind::Update => &mut sku.remaining_update_reward_budget,
    };
    *remaining = (*remaining)
        .checked_sub(amount)
        .ok_or(VaultError::InvalidOracleUsdcRewardSchedule)?;
    Ok(())
}

#[inline(never)]
pub(in crate::processor) fn process_claim_oracle_usdc_reward(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ClaimOracleUsdcRewardParams,
) -> ProgramResult {
    if accounts.len() < 13 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let recipient_info = &accounts[0];
    let config_info = &accounts[1];
    let market_info = &accounts[2];
    let month_info = &accounts[3];
    let schedule_info = &accounts[4];
    let reward_vault_info = &accounts[5];
    let reward_token_info = &accounts[6];
    let primary_vault_token_info = &accounts[7];
    let mint_info = &accounts[8];
    let collateral_info = &accounts[9];
    let receipt_info = &accounts[10];
    let token_program_info = &accounts[11];
    let system_program_info = &accounts[12];
    let trailing = &accounts[13..];
    if !recipient_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_system_program(system_program_info)?;

    let config = load_current_canonical_vault_config(program_id, config_info)?;
    let (_market, _month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let mut schedule = load_oracle_usdc_reward_schedule(program_id, month_info.key, schedule_info)?;
    let mut reward_vault = load_oracle_usdc_reward_vault(program_id, reward_vault_info)?;
    if schedule.phase != OracleUsdcRewardSchedulePhase::EntitlementsFinalized
        || schedule.reward_vault != *reward_vault_info.key
        || reward_vault.mint != config.usdc_mint
    {
        return Err(VaultError::OracleUsdcRewardEntitlementsNotFinalized.into());
    }
    let (reward_token, mint) = validate_claim_custody(
        &config,
        config_info,
        &reward_vault,
        reward_vault_info,
        reward_token_info,
        primary_vault_token_info,
        mint_info,
        token_program_info,
    )?;
    if reward_token.amount < reward_vault.total_reserved {
        return Err(VaultError::InsufficientOracleUsdcRewardCustody.into());
    }
    let computed = match params.kind {
        OracleUsdcRewardKind::SourceProposer
        | OracleUsdcRewardKind::SourceSupport
        | OracleUsdcRewardKind::Opening => compute_source_family_reward(
            program_id,
            month_info.key,
            schedule_info.key,
            recipient_info.key,
            params.kind,
            trailing,
        )?,
        OracleUsdcRewardKind::Update => compute_update_reward(
            program_id,
            month_info.key,
            schedule_info.key,
            recipient_info.key,
            trailing,
        )?,
    };
    if computed.amount == 0
        || computed.amount > schedule.remaining_reward_budget
        || computed.amount > reward_vault.total_reserved
        || computed.amount > reward_token.amount
    {
        return Err(VaultError::InsufficientOracleUsdcRewardCustody.into());
    }

    let (expected_receipt, receipt_bump) = derive_oracle_usdc_reward_receipt_pda(
        program_id,
        schedule_info.key,
        params.kind,
        &computed.subject,
        recipient_info.key,
    );
    if *receipt_info.key != expected_receipt {
        return Err(VaultError::InvalidOracleUsdcRewardReceipt.into());
    }
    validate_create_only_program_account_target(program_id, receipt_info)?;
    let kind_seed = [params.kind as u8];
    create_program_account(
        recipient_info,
        receipt_info,
        system_program_info,
        program_id,
        OracleUsdcRewardReceipt::LEN,
        &[
            ORACLE_USDC_REWARD_RECEIPT_PDA_SEED,
            schedule_info.key.as_ref(),
            &kind_seed,
            computed.subject.as_ref(),
            recipient_info.key.as_ref(),
            &[receipt_bump],
        ],
    )?;

    let mut collateral =
        load_canonical_user_collateral(program_id, collateral_info, recipient_info.key)?;
    invoke_token_transfer_checked(
        token_program_info,
        reward_token_info,
        mint_info,
        primary_vault_token_info,
        reward_vault_info,
        computed.amount,
        mint.decimals,
        &[&[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_USDC_REWARD_VAULT_PDA_SEED,
            &[reward_vault.bump],
        ]],
    )?;

    let slot = Clock::get()?.slot;
    collateral.available_balance = collateral
        .available_balance
        .checked_add(computed.amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    collateral.last_action_slot = slot;
    reward_vault.total_reserved = reward_vault
        .total_reserved
        .checked_sub(computed.amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    reward_vault.total_paid = reward_vault
        .total_paid
        .checked_add(computed.amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    reward_vault.last_updated_slot = slot;
    schedule.remaining_reward_budget = schedule
        .remaining_reward_budget
        .checked_sub(computed.amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    schedule.last_updated_slot = slot;

    let mut sku = computed.sku;
    decrement_reward_sleeve(params.kind, computed.amount, &mut sku)?;
    sku.last_updated_slot = slot;
    store_state(&trailing[0], &sku)?;
    let receipt = OracleUsdcRewardReceipt {
        is_initialized: true,
        bump: receipt_bump,
        account_discriminator: OracleUsdcRewardReceipt::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUsdcRewardReceipt::ACCOUNT_VERSION,
        month: *month_info.key,
        schedule: *schedule_info.key,
        recipient: *recipient_info.key,
        kind: params.kind,
        subject: computed.subject,
        amount: computed.amount,
        claimed_slot: slot,
    };
    store_state(receipt_info, &receipt)?;
    store_state(collateral_info, &collateral)?;
    store_state(reward_vault_info, &reward_vault)?;
    store_state(schedule_info, &schedule)
}
