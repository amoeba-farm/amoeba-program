use super::*;

pub(in crate::processor) fn require_signed_cranker(cranker_info: &AccountInfo) -> ProgramResult {
    if !cranker_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    Ok(())
}

pub(in crate::processor) fn ensure_failed_v5_reward_schedule_window(
    market: &Market,
    month: &OracleMonthState,
) -> ProgramResult {
    ensure_failed_v5_reward_schedule_window_at(
        market.instrument.expiry_ts,
        month,
        current_unix_timestamp()?,
    )
}

pub(in crate::processor) fn ensure_failed_v5_reward_schedule_window_at(
    expiry_ts: u64,
    month: &OracleMonthState,
    now: u64,
) -> ProgramResult {
    let preserved_post_submission_seconds = ORACLE_KILL_WINDOW_SECONDS
        .checked_add(ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS)
        .and_then(|value| value.checked_add(ORACLE_OPENING_WINDOW_SECONDS))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let latest_safe = expiry_ts
        .checked_sub(preserved_post_submission_seconds)
        .ok_or(VaultError::InvalidOracleState)?;
    if month.phase != OraclePhase::SourceSubmission
        || month.pending_resolution_count != 0
        || month.weight_scheme_version != 0
        || now < latest_safe
    {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    Ok(())
}

pub(in crate::processor) fn release_aborted_reward_reservation(
    reward_vault: &mut OracleUsdcRewardVault,
    schedule: &mut OracleUsdcRewardSchedule,
) -> ProgramResult {
    if schedule.phase != OracleUsdcRewardSchedulePhase::Funded
        || schedule.outstanding_prelisting_escrow_count != 0
        || schedule.total_reward_budget == 0
        || schedule.remaining_reward_budget != schedule.total_reward_budget
        || reward_vault.total_reserved < schedule.remaining_reward_budget
    {
        return Err(VaultError::InvalidOracleUsdcRewardSchedule.into());
    }
    reward_vault.total_reserved = reward_vault
        .total_reserved
        .checked_sub(schedule.remaining_reward_budget)
        .ok_or(VaultError::InvalidOracleUsdcRewardSchedule)?;
    schedule.remaining_reward_budget = 0;
    schedule.phase = OracleUsdcRewardSchedulePhase::Aborted;
    Ok(())
}

pub(in crate::processor) fn credit_cash_collateral_at_slot(
    collateral: &mut UserCollateral,
    amount: u64,
    slot: u64,
) -> ProgramResult {
    if amount == 0 {
        return Err(VaultError::InvalidOracleUsdcBond.into());
    }
    collateral.available_balance = collateral
        .available_balance
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    collateral.last_action_slot = slot;
    Ok(())
}

pub(in crate::processor) fn load_source_reward(
    program_id: &Pubkey,
    schedule: &Pubkey,
    sku: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    reward_info: &AccountInfo,
) -> Result<OracleUsdcSourceReward, ProgramError> {
    let reward = load_oracle_usdc_source_reward(program_id, schedule, month, source, reward_info)?;
    if reward.sku_pool != *sku {
        return Err(VaultError::InvalidOracleUsdcSourceReward.into());
    }
    Ok(reward)
}

pub(in crate::processor) fn load_reward_registration(
    program_id: &Pubkey,
    month: &Pubkey,
    schedule: &Pubkey,
    kind: OracleUsdcRewardKind,
    subject: &Pubkey,
    registration_info: &AccountInfo,
) -> Result<OracleUsdcRewardRegistration, ProgramError> {
    let registration: OracleUsdcRewardRegistration = load_exact_zero_padded_state(
        registration_info,
        program_id,
        OracleUsdcRewardRegistration::LEN,
        VaultError::InvalidOracleUsdcRewardRegistration,
    )?;
    let (expected, bump) =
        derive_oracle_usdc_reward_registration_pda(program_id, schedule, kind, subject);
    if *registration_info.key != expected
        || !registration.is_initialized
        || registration.bump != bump
        || registration.account_discriminator != OracleUsdcRewardRegistration::ACCOUNT_DISCRIMINATOR
        || registration.account_version != OracleUsdcRewardRegistration::ACCOUNT_VERSION
        || registration.month != *month
        || registration.schedule != *schedule
        || registration.kind != kind
        || registration.subject != *subject
        || !matches!(
            registration.reward_units,
            1 | crate::constants::ORACLE_FRESH_UPDATE_REWARD_MULTIPLIER
        )
    {
        return Err(VaultError::InvalidOracleUsdcRewardRegistration.into());
    }
    Ok(registration)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn create_reward_registration<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    month: &Pubkey,
    schedule: &Pubkey,
    sku_pool: Pubkey,
    kind: OracleUsdcRewardKind,
    subject: &Pubkey,
    recipient: Pubkey,
    reward_units: u8,
    registration_info: &AccountInfo<'a>,
    slot: u64,
) -> ProgramResult {
    if !matches!(
        reward_units,
        1 | crate::constants::ORACLE_FRESH_UPDATE_REWARD_MULTIPLIER
    ) {
        return Err(VaultError::InvalidOracleUsdcRewardRegistration.into());
    }
    validate_system_program(system_program_info)?;
    let (expected, bump) =
        derive_oracle_usdc_reward_registration_pda(program_id, schedule, kind, subject);
    if *registration_info.key != expected {
        return Err(VaultError::InvalidOracleUsdcRewardRegistration.into());
    }
    validate_create_only_program_account_target(program_id, registration_info)?;
    let kind_seed = [kind as u8];
    create_program_account(
        payer_info,
        registration_info,
        system_program_info,
        program_id,
        OracleUsdcRewardRegistration::LEN,
        &[
            ORACLE_USDC_REWARD_REGISTRATION_PDA_SEED,
            ORACLE_USDC_REWARD_REGISTRATION_VERSION_SEED,
            schedule.as_ref(),
            &kind_seed,
            subject.as_ref(),
            &[bump],
        ],
    )?;
    let registration = OracleUsdcRewardRegistration {
        is_initialized: true,
        bump,
        account_discriminator: OracleUsdcRewardRegistration::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUsdcRewardRegistration::ACCOUNT_VERSION,
        month: *month,
        schedule: *schedule,
        sku_pool,
        kind,
        subject: *subject,
        recipient,
        last_updated_slot: slot,
        reward_units,
    };
    store_state(registration_info, &registration)
}

pub(in crate::processor) fn validate_registration_phase(
    _month: &OracleMonthState,
    schedule: &OracleUsdcRewardSchedule,
) -> ProgramResult {
    if schedule.phase != OracleUsdcRewardSchedulePhase::Funded {
        return Err(VaultError::OracleUsdcRewardScheduleFrozen.into());
    }
    Ok(())
}

#[inline(never)]
pub(in crate::processor) fn process_register_oracle_usdc_reward_source(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() < 7 || accounts.len() > 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let payer_info = &accounts[0];
    let month_info = &accounts[1];
    let schedule_info = &accounts[2];
    let sku_info = &accounts[3];
    let active_manifest_info = &accounts[4];
    let source_info = &accounts[5];
    let source_reward_info = &accounts[6];
    let opening_claim_info = accounts.get(7);
    if !payer_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let month = load_oracle_month_state(month_info, program_id)?;
    let mut schedule = load_oracle_usdc_reward_schedule(program_id, month_info.key, schedule_info)?;
    validate_registration_phase(&month, &schedule)?;
    let mut sku =
        load_oracle_usdc_sku_pool(program_id, schedule_info.key, month_info.key, sku_info)?;
    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    validate_active_oracle_source(
        program_id,
        &month,
        month_info.key,
        active_manifest_info,
        &source,
    )?;
    let mut source_reward = load_source_reward(
        program_id,
        schedule_info.key,
        sku_info.key,
        month_info.key,
        source_info.key,
        source_reward_info,
    )?;
    if source_reward.registered
        || source_reward.source_id != source.source_id
        || source_reward.proposer != source.proposer
        || !crate::pubkey_is_default(&source_reward.merged_into_source)
        || source_reward.max_merge_depth > MAX_ORACLE_USDC_MERGE_DEPTH
        || sku.bucket_id != source.bucket_id
        || u64::from(source_reward.supporter_count)
            .checked_mul(sku.support_bond)
            .ok_or(VaultError::ArithmeticOverflow)?
            != source.support_stake_total
        || !matches!(
            source.status,
            OracleSourceStatus::Active | OracleSourceStatus::Inactive
        )
    {
        return Err(VaultError::InvalidOracleUsdcSourceReward.into());
    }

    let opening_claim = if source.status == OracleSourceStatus::Active {
        let claim_info = opening_claim_info.ok_or(VaultError::InvalidAccountList)?;
        let claim = load_valid_oracle_opening_claim(
            program_id,
            month_info.key,
            source_info.key,
            claim_info,
            &source,
        )?;
        if claim.status != OracleOpeningClaimStatus::Accepted {
            return Err(VaultError::InvalidOracleUsdcSourceReward.into());
        }
        *claim_info.key
    } else {
        if opening_claim_info.is_some() {
            return Err(VaultError::InvalidAccountList.into());
        }
        Pubkey::default()
    };

    let slot = Clock::get()?.slot;
    source_reward.registered = true;
    source_reward.terminal_status = source.status;
    source_reward.opening_claim = opening_claim;
    source_reward.last_updated_slot = slot;
    sku.registered_source_count = sku
        .registered_source_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    schedule.registered_source_count = schedule
        .registered_source_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if source.status == OracleSourceStatus::Active {
        sku.registered_opening_count = sku
            .registered_opening_count
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        schedule.registered_opening_count = schedule
            .registered_opening_count
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
    }
    if schedule.registered_source_count > u32::from(month.frozen_source_count)
        || schedule.registered_opening_count > u32::from(month.opened_source_count)
    {
        return Err(VaultError::OracleUsdcRewardRegistrationIncomplete.into());
    }
    sku.last_updated_slot = slot;
    schedule.last_updated_slot = slot;
    store_state(source_reward_info, &source_reward)?;
    store_state(sku_info, &sku)?;
    store_state(schedule_info, &schedule)
}

#[inline(never)]
pub(in crate::processor) fn process_register_oracle_usdc_reward_update(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 9 || !accounts[0].is_signer {
        return Err(VaultError::InvalidAccountList.into());
    }
    let payer_info = &accounts[0];
    let month_info = &accounts[1];
    let schedule_info = &accounts[2];
    let sku_info = &accounts[3];
    let source_info = &accounts[4];
    let claim_info = &accounts[5];
    let registration_info = &accounts[6];
    let bucket_info = &accounts[7];
    let system_program_info = &accounts[8];

    let month = load_oracle_month_state(month_info, program_id)?;
    let mut schedule = load_oracle_usdc_reward_schedule(program_id, month_info.key, schedule_info)?;
    validate_registration_phase(&month, &schedule)?;
    let mut sku =
        load_oracle_usdc_sku_pool(program_id, schedule_info.key, month_info.key, sku_info)?;
    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let bucket = load_valid_oracle_bucket_median(program_id, month_info.key, bucket_info)?;
    let claim = load_valid_oracle_update_claim_v2_from_account(
        program_id,
        month_info.key,
        source_info.key,
        claim_info,
    )?;
    if source.status != OracleSourceStatus::Active
        || sku.bucket_id != source.bucket_id
        || bucket.bucket_id != source.bucket_id
        || !schedule.bounty_fee_sweep_finalized
        || !matches!(
            bucket.status,
            OracleBucketMedianStatus::SettlementReady
                | OracleBucketMedianStatus::EmergencyDefaulted
        )
        || claim.claim.source != *source_info.key
        || claim.claim.source_id != source.source_id
        || claim.claim.status != OracleClaimStatus::Finalized
        || !matches!(
            claim.freshness_reward_multiplier,
            1 | crate::constants::ORACLE_FRESH_UPDATE_REWARD_MULTIPLIER
        )
    {
        return Err(VaultError::InvalidOracleUsdcRewardRegistration.into());
    }
    let bounty = oracle_bucket_bounty_share(
        schedule.trading_fee_bounty_total,
        bucket.bucket_weight_start_bps,
        bucket.bucket_weight_bps,
    )?;
    if sku.registered_update_count == 0 {
        sku.update_reward_budget = sku
            .update_reward_budget
            .checked_add(bounty)
            .ok_or(VaultError::ArithmeticOverflow)?;
        sku.remaining_update_reward_budget = sku
            .remaining_update_reward_budget
            .checked_add(bounty)
            .ok_or(VaultError::ArithmeticOverflow)?;
    }
    let slot = Clock::get()?.slot;
    create_reward_registration(
        program_id,
        payer_info,
        system_program_info,
        month_info.key,
        schedule_info.key,
        *sku_info.key,
        OracleUsdcRewardKind::Update,
        claim_info.key,
        claim.claim.claimant,
        claim.freshness_reward_multiplier,
        registration_info,
        slot,
    )?;
    sku.registered_update_count = sku
        .registered_update_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    schedule.registered_update_count = schedule
        .registered_update_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sku.registered_update_reward_units = sku
        .registered_update_reward_units
        .checked_add(u32::from(claim.freshness_reward_multiplier))
        .ok_or(VaultError::ArithmeticOverflow)?;
    schedule.registered_update_reward_units = schedule
        .registered_update_reward_units
        .checked_add(u32::from(claim.freshness_reward_multiplier))
        .ok_or(VaultError::ArithmeticOverflow)?;
    if schedule.registered_update_count > month.accepted_cash_update_count {
        return Err(VaultError::OracleUsdcRewardRegistrationIncomplete.into());
    }
    sku.last_updated_slot = slot;
    schedule.last_updated_slot = slot;
    store_state(sku_info, &sku)?;
    store_state(schedule_info, &schedule)
}

#[inline(never)]
pub(in crate::processor) fn process_finalize_oracle_usdc_reward_entitlements(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 5 || !accounts[0].is_signer {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let reward_vault_info = &accounts[3];
    let schedule_info = &accounts[4];

    let (_market, month) = load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let vault = load_oracle_usdc_reward_vault(program_id, reward_vault_info)?;
    let mut schedule = load_oracle_usdc_reward_schedule(program_id, month_info.key, schedule_info)?;
    if *authority_info.key != month.authority
        || schedule.authority != *authority_info.key
        || schedule.reward_vault != *reward_vault_info.key
        || schedule.phase != OracleUsdcRewardSchedulePhase::Funded
        || !matches!(month.phase, OraclePhase::Settled | OraclePhase::Closed)
        || month.pending_resolution_count != 0
        || month.finalized_at_ts == 0
        || schedule.registered_source_count != u32::from(month.frozen_source_count)
        || schedule.registered_opening_count != u32::from(month.opened_source_count)
        || schedule.registered_update_count != month.accepted_cash_update_count
        || schedule.registered_update_reward_units < schedule.registered_update_count
        || !schedule.bounty_fee_sweep_finalized
        || schedule.remaining_reward_budget != schedule.total_reward_budget
        || vault.total_reserved < schedule.remaining_reward_budget
    {
        return Err(VaultError::OracleUsdcRewardRegistrationIncomplete.into());
    }
    schedule.phase = OracleUsdcRewardSchedulePhase::EntitlementsFinalized;
    schedule.last_updated_slot = Clock::get()?.slot;
    store_state(schedule_info, &schedule)
}
