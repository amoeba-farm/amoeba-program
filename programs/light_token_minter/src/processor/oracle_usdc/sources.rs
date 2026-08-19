use super::*;

pub(in crate::processor) fn load_oracle_usdc_sku_for_month(
    program_id: &Pubkey,
    month: &Pubkey,
    sku_info: &AccountInfo,
) -> Result<OracleUsdcSkuPool, ProgramError> {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    load_oracle_usdc_sku_pool(program_id, &schedule, month, sku_info)
}

#[inline(never)]
pub(in crate::processor) fn process_propose_oracle_source_v3(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ProposeOracleSourceV3Params,
) -> ProgramResult {
    process_propose_oracle_source(
        program_id,
        accounts,
        ProposeOracleSourceParams {
            source_id: params.source_id,
            bucket_id: params.bucket_id,
            source_type_hash: params.source_type_hash,
            canonical_locator_hash: params.canonical_locator_hash,
            source_definition_hash: params.source_definition_hash,
            listing_bond: params.listing_bond,
        },
        (params.sku_index, params.sku_proof.as_slice()),
    )
}

pub(in crate::processor) fn process_propose_oracle_source(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ProposeOracleSourceParams,
    coverage_membership: (u16, &[[u8; 32]]),
) -> ProgramResult {
    if accounts.len() != 11 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let proposer_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let schedule_info = &accounts[3];
    let sku_info = &accounts[4];
    let source_info = &accounts[5];
    let observations_info = &accounts[6];
    let source_reward_info = &accounts[7];
    let collateral_info = &accounts[8];
    let system_program_info = &accounts[9];
    let coverage_info = &accounts[10];
    if !proposer_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_system_program(system_program_info)?;
    if crate::bytes32_is_zero(&params.source_id)
        || crate::bytes32_is_zero(&params.bucket_id)
        || crate::bytes32_is_zero(&params.source_type_hash)
        || crate::bytes32_is_zero(&params.canonical_locator_hash)
        || crate::bytes32_is_zero(&params.source_definition_hash)
    {
        return Err(VaultError::InvalidOracleState.into());
    }

    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let (sku_index, proof) = coverage_membership;
    let coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    ensure_oracle_coverage_source_submission_window(&market, &month, &coverage)?;
    verify_oracle_sku_membership(&coverage, &params.bucket_id, sku_index, proof)?;
    if month.weight_scheme_version == 255 {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    let mut schedule = load_oracle_usdc_reward_schedule(program_id, month_info.key, schedule_info)?;
    let sku = load_oracle_usdc_sku_pool(program_id, schedule_info.key, month_info.key, sku_info)?;
    if schedule.phase != OracleUsdcRewardSchedulePhase::Funded
        || sku.bucket_id != params.bucket_id
        || params.listing_bond != sku.listing_bond
    {
        return Err(VaultError::InvalidOracleUsdcBond.into());
    }

    let (expected_source, source_bump) =
        derive_oracle_source_pda(program_id, month_info.key, &params.source_id);
    let (expected_observations, observations_bump) =
        derive_oracle_source_observations_pda(program_id, source_info.key);
    let (expected_source_reward, source_reward_bump) =
        derive_oracle_usdc_source_reward_pda(program_id, schedule_info.key, source_info.key);
    if *source_info.key != expected_source
        || *observations_info.key != expected_observations
        || *source_reward_info.key != expected_source_reward
    {
        return Err(VaultError::InvalidOracleSourceAccount.into());
    }
    validate_create_only_program_account_target(program_id, source_info)?;
    validate_create_only_program_account_target(program_id, observations_info)?;
    validate_create_only_program_account_target(program_id, source_reward_info)?;
    let mut collateral =
        load_canonical_user_collateral(program_id, collateral_info, proposer_info.key)?;
    debit_oracle_usdc_available(&mut collateral, sku.listing_bond)?;

    create_program_account(
        proposer_info,
        source_info,
        system_program_info,
        program_id,
        OracleSourceState::LEN,
        &[
            ORACLE_SOURCE_PDA_SEED,
            month_info.key.as_ref(),
            &params.source_id,
            &[source_bump],
        ],
    )?;
    create_program_account(
        proposer_info,
        observations_info,
        system_program_info,
        program_id,
        OracleSourceObservations::LEN,
        &[
            ORACLE_SOURCE_OBSERVATIONS_PDA_SEED,
            source_info.key.as_ref(),
            &[observations_bump],
        ],
    )?;
    create_program_account(
        proposer_info,
        source_reward_info,
        system_program_info,
        program_id,
        OracleUsdcSourceReward::LEN,
        &[
            ORACLE_USDC_SOURCE_REWARD_PDA_SEED,
            schedule_info.key.as_ref(),
            source_info.key.as_ref(),
            &[source_reward_bump],
        ],
    )?;

    let slot = Clock::get()?.slot;
    let source = OracleSourceState {
        is_initialized: true,
        bump: source_bump,
        month: *month_info.key,
        source_id: params.source_id,
        bucket_id: params.bucket_id,
        source_type_hash: params.source_type_hash,
        canonical_locator_hash: params.canonical_locator_hash,
        source_definition_hash: params.source_definition_hash,
        proposer: *proposer_info.key,
        listing_bond_locked: sku.listing_bond,
        status: OracleSourceStatus::Candidate,
        ..OracleSourceState::default()
    };
    let observations = OracleSourceObservations {
        is_initialized: true,
        bump: observations_bump,
        account_discriminator: OracleSourceObservations::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSourceObservations::ACCOUNT_VERSION,
        month: *month_info.key,
        source: *source_info.key,
        states: [0; crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS],
        source_times: [0; crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS],
    };
    let source_reward = OracleUsdcSourceReward {
        is_initialized: true,
        bump: source_reward_bump,
        account_discriminator: OracleUsdcSourceReward::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUsdcSourceReward::ACCOUNT_VERSION,
        month: *month_info.key,
        schedule: *schedule_info.key,
        sku_pool: *sku_info.key,
        source: *source_info.key,
        source_id: params.source_id,
        proposer: *proposer_info.key,
        supporter_count: 0,
        registered: false,
        terminal_status: OracleSourceStatus::Candidate,
        opening_claim: Pubkey::default(),
        last_updated_slot: slot,
        merged_into_source: Pubkey::default(),
        max_merge_depth: 0,
        listing_escrow_counted: false,
    };
    schedule.outstanding_prelisting_escrow_count = schedule
        .outstanding_prelisting_escrow_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    schedule.last_updated_slot = slot;
    month.last_updated_slot = slot;
    store_state(source_info, &source)?;
    store_state(observations_info, &observations)?;
    store_state(source_reward_info, &source_reward)?;
    store_state(collateral_info, &collateral)?;
    store_state(schedule_info, &schedule)?;
    store_oracle_month_state(month_info, &month)
}

#[inline(never)]
pub(in crate::processor) fn process_support_oracle_source_v3(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SupportOracleSourceV3Params,
) -> ProgramResult {
    process_support_oracle_source(
        program_id,
        accounts,
        SupportOracleSourceParams {
            stake: params.stake,
        },
        (params.sku_index, params.sku_proof.as_slice()),
    )
}

pub(in crate::processor) fn process_support_oracle_source(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SupportOracleSourceParams,
    coverage_membership: (u16, &[[u8; 32]]),
) -> ProgramResult {
    if accounts.len() != 12 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let supporter_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let schedule_info = &accounts[3];
    let sku_info = &accounts[4];
    let source_info = &accounts[5];
    let source_reward_info = &accounts[6];
    let collateral_info = &accounts[7];
    let support_info = &accounts[8];
    let system_program_info = &accounts[9];
    let coverage_info = &accounts[10];
    let coverage_record_info = &accounts[11];
    if !supporter_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_system_program(system_program_info)?;

    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let (sku_index, proof) = coverage_membership;
    let mut coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    ensure_oracle_coverage_source_submission_window(&market, &month, &coverage)?;
    if month.weight_scheme_version == 255 {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    let mut schedule = load_oracle_usdc_reward_schedule(program_id, month_info.key, schedule_info)?;
    let sku = load_oracle_usdc_sku_pool(program_id, schedule_info.key, month_info.key, sku_info)?;
    let mut source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let mut source_reward = load_oracle_usdc_source_reward(
        program_id,
        schedule_info.key,
        month_info.key,
        source_info.key,
        source_reward_info,
    )?;
    if schedule.phase != OracleUsdcRewardSchedulePhase::Funded
        || source.status != OracleSourceStatus::Candidate
        || source.bucket_id != sku.bucket_id
        || source.proposer == *supporter_info.key
        || source_reward.sku_pool != *sku_info.key
        || source_reward.source_id != source.source_id
        || source_reward.proposer != source.proposer
        || source_reward.registered
        || params.stake != sku.support_bond
    {
        return Err(VaultError::InvalidOracleUsdcBond.into());
    }
    verify_oracle_sku_membership(&coverage, &source.bucket_id, sku_index, proof)?;
    let mut coverage_record = {
        let record_info = coverage_record_info;
        let (expected_record, record_bump) =
            derive_oracle_sku_coverage_record_pda(program_id, month_info.key, &source.bucket_id);
        if *record_info.key != expected_record {
            return Err(VaultError::InvalidOracleSkuCoverageRecord.into());
        }
        if record_info.owner == program_id {
            let existing = load_valid_oracle_sku_coverage_record(
                program_id,
                month_info.key,
                &source.bucket_id,
                record_info,
            )?;
            if existing.sku_index != sku_index {
                return Err(VaultError::InvalidOracleSkuCoverageRecord.into());
            }
            existing
        } else {
            if source.support_stake_total != 0 {
                return Err(VaultError::InvalidOracleSkuCoverageRecord.into());
            }
            create_program_account(
                supporter_info,
                record_info,
                system_program_info,
                program_id,
                OracleSkuCoverageRecord::LEN,
                &[
                    crate::constants::ORACLE_SKU_COVERAGE_RECORD_PDA_SEED,
                    month_info.key.as_ref(),
                    &source.bucket_id,
                    &[record_bump],
                ],
            )?;
            OracleSkuCoverageRecord {
                is_initialized: true,
                bump: record_bump,
                account_discriminator: OracleSkuCoverageRecord::ACCOUNT_DISCRIMINATOR,
                account_version: OracleSkuCoverageRecord::ACCOUNT_VERSION,
                month: *month_info.key,
                sku_id: source.bucket_id,
                sku_index,
                active_supported_source_count: 0,
                // The record is updated with the transaction slot before its only store below.
                last_updated_slot: 0,
            }
        }
    };

    let (expected_support, support_bump) = derive_oracle_support_pda(
        program_id,
        month_info.key,
        source_info.key,
        supporter_info.key,
    );
    if *support_info.key != expected_support {
        return Err(VaultError::InvalidOracleState.into());
    }
    validate_create_only_program_account_target(program_id, support_info)?;
    let mut collateral =
        load_canonical_user_collateral(program_id, collateral_info, supporter_info.key)?;
    debit_oracle_usdc_available(&mut collateral, sku.support_bond)?;
    create_program_account(
        supporter_info,
        support_info,
        system_program_info,
        program_id,
        OracleSupportPosition::LEN,
        &[
            ORACLE_SUPPORT_POSITION_PDA_SEED,
            month_info.key.as_ref(),
            source_info.key.as_ref(),
            supporter_info.key.as_ref(),
            &[support_bump],
        ],
    )?;

    let first_supported_stake = source.support_stake_total == 0;
    source.support_stake_total = source
        .support_stake_total
        .checked_add(sku.support_bond)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if first_supported_stake {
        increment_oracle_supported_candidate_count(&mut month)?;
        {
            if coverage_record.active_supported_source_count == 0 {
                coverage.covered_sku_count = coverage
                    .covered_sku_count
                    .checked_add(1)
                    .ok_or(VaultError::ArithmeticOverflow)?;
                if coverage.covered_sku_count > coverage.required_sku_count {
                    return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
                }
            }
            coverage_record.active_supported_source_count = coverage_record
                .active_supported_source_count
                .checked_add(1)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
    }
    source_reward.supporter_count = source_reward
        .supporter_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let support = OracleSupportPosition {
        is_initialized: true,
        bump: support_bump,
        month: *month_info.key,
        supporter: *supporter_info.key,
        source: *source_info.key,
        source_id: source.source_id,
        support_stake: sku.support_bond,
        released: false,
        escrow_disposition: OracleEscrowDisposition::Unsettled,
        failed_schedule_escrow_counted: false,
    };
    let slot = Clock::get()?.slot;
    schedule.outstanding_prelisting_escrow_count = schedule
        .outstanding_prelisting_escrow_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    schedule.last_updated_slot = slot;
    source_reward.last_updated_slot = slot;
    month.last_updated_slot = slot;
    coverage.last_updated_slot = slot;
    coverage_record.last_updated_slot = slot;
    store_state(support_info, &support)?;
    store_state(source_info, &source)?;
    store_state(source_reward_info, &source_reward)?;
    store_state(collateral_info, &collateral)?;
    store_state(schedule_info, &schedule)?;
    store_state(coverage_info, &coverage)?;
    store_state(coverage_record_info, &coverage_record)?;
    store_oracle_month_state(month_info, &month)
}

pub(in crate::processor) fn timeout_unsupported_oracle_source_state(
    month: &mut OracleMonthState,
    coverage: &OracleSkuCoverageManifest,
    source: &mut OracleSourceState,
    now: u64,
    slot: u64,
) -> ProgramResult {
    let (placement_end, _, _) = rulebook_schedule_boundaries(month)?;
    if month.phase != OraclePhase::Scramble
        || month.pending_resolution_count != 0
        || month.weight_scheme_version != 0
        || !coverage.coverage_finalized
        || coverage.covered_sku_count != coverage.required_sku_count
        || now < placement_end
        || source.status != OracleSourceStatus::Candidate
        || source.support_stake_total != 0
        || source.listing_bond_locked == 0
    {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    source.status = OracleSourceStatus::TimedOut;
    month.last_updated_slot = slot;
    Ok(())
}

#[inline(never)]
pub(in crate::processor) fn process_timeout_unsupported_oracle_source_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 5 || !accounts[0].is_signer {
        return Err(VaultError::InvalidAccountList.into());
    }
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let coverage_info = &accounts[3];
    let source_info = &accounts[4];

    let (_market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    let mut source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let clock = Clock::get()?;
    let now = u64::try_from(clock.unix_timestamp)
        .map_err(|_| ProgramError::from(VaultError::InvalidSettlementRecord))?;
    timeout_unsupported_oracle_source_state(&mut month, &coverage, &mut source, now, clock.slot)?;
    store_state(source_info, &source)?;
    store_oracle_month_state(month_info, &month)
}

pub(in crate::processor) fn acquireable_source_challenge_guard(
    program_id: &Pubkey,
    month: &Pubkey,
    source_info: &AccountInfo,
    source: &OracleSourceState,
    guard_info: &AccountInfo,
) -> Result<(OracleSourceChallengeGuard, bool), ProgramError> {
    let (expected, bump) =
        derive_oracle_source_challenge_guard_pda(program_id, month, source_info.key);
    if *guard_info.key != expected || !guard_info.is_writable {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    if guard_info.owner != program_id {
        validate_create_only_program_account_target(program_id, guard_info)?;
        return Ok((
            OracleSourceChallengeGuard {
                is_initialized: true,
                bump,
                account_discriminator: OracleSourceChallengeGuard::ACCOUNT_DISCRIMINATOR,
                account_version: OracleSourceChallengeGuard::ACCOUNT_VERSION,
                month: *month,
                source: *source_info.key,
                source_id: source.source_id,
                ..OracleSourceChallengeGuard::default()
            },
            true,
        ));
    }
    let guard: OracleSourceChallengeGuard = load_exact_zero_padded_state(
        guard_info,
        program_id,
        OracleSourceChallengeGuard::LEN,
        VaultError::InvalidOracleChallengeAccount,
    )?;
    if !guard.is_initialized
        || guard.bump != bump
        || guard.account_discriminator != OracleSourceChallengeGuard::ACCOUNT_DISCRIMINATOR
        || guard.account_version != OracleSourceChallengeGuard::ACCOUNT_VERSION
        || guard.month != *month
        || guard.source != *source_info.key
        || guard.source_id != source.source_id
        || !crate::pubkey_is_default(&guard.active_challenge)
        || !crate::bytes32_is_zero(&guard.active_challenge_id)
        || !crate::pubkey_is_default(&guard.active_dispute)
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    Ok((guard, false))
}

#[inline(never)]
pub(in crate::processor) fn process_challenge_oracle_source_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ChallengeOracleSourceParams,
) -> ProgramResult {
    if accounts.len() < 9 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let challenger_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let sku_info = &accounts[3];
    let source_info = &accounts[4];
    let collateral_info = &accounts[5];
    let challenge_info = &accounts[6];
    let system_program_info = &accounts[7];
    let source_guard_info = &accounts[8];
    if !challenger_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_system_program(system_program_info)?;
    if crate::bytes32_is_zero(&params.challenge_id) || crate::bytes32_is_zero(&params.evidence_hash)
    {
        return Err(VaultError::InvalidOracleState.into());
    }

    let (_market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_kill_window(&month)?;
    if month.weight_scheme_version == 255 {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let sku = load_oracle_usdc_sku_for_month(program_id, month_info.key, sku_info)?;
    if source.status != OracleSourceStatus::Candidate
        || source.bucket_id != sku.bucket_id
        || source.proposer == *challenger_info.key
    {
        return Err(VaultError::InvalidOraclePhase.into());
    }

    let (expected_challenge, challenge_bump) = derive_oracle_source_challenge_pda(
        program_id,
        month_info.key,
        source_info.key,
        &params.challenge_id,
    );
    if *challenge_info.key != expected_challenge {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    validate_create_only_program_account_target(program_id, challenge_info)?;
    if source_guard_info.key == source_info.key || source_guard_info.key == challenge_info.key {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    let (mut source_guard, create_source_guard) = acquireable_source_challenge_guard(
        program_id,
        month_info.key,
        source_info,
        &source,
        source_guard_info,
    )?;

    let (comparison_source, comparison_source_id, comparison_guard, bond_backing_total) =
        if params.reason == ORACLE_SOURCE_REASON_NON_INDEPENDENT
            || !crate::bytes32_is_zero(&params.comparison_source_id)
        {
            if params.reason != ORACLE_SOURCE_REASON_NON_INDEPENDENT
                || crate::bytes32_is_zero(&params.comparison_source_id)
            {
                return Err(VaultError::InvalidOracleSourceAccount.into());
            }
            let comparison_source_info = accounts.get(9).ok_or(VaultError::InvalidAccountList)?;
            let comparison_guard_info = accounts.get(10).ok_or(VaultError::InvalidAccountList)?;
            let comparison_source_state =
                load_valid_oracle_source(program_id, month_info.key, comparison_source_info)?;
            let (expected_comparison_source, _) =
                derive_oracle_source_pda(program_id, month_info.key, &params.comparison_source_id);
            if *comparison_source_info.key != expected_comparison_source
                || comparison_source_state.source_id != params.comparison_source_id
                || comparison_source_state.status != OracleSourceStatus::Candidate
                || comparison_source_state.bucket_id != source.bucket_id
                || comparison_source_info.key == source_info.key
                || params.comparison_source_id == source.source_id
                || comparison_source_state.source_id == source.source_id
                || comparison_guard_info.key == source_guard_info.key
                || comparison_guard_info.key == comparison_source_info.key
                || comparison_guard_info.key == source_info.key
                || comparison_guard_info.key == challenge_info.key
                || source_guard_info.key == comparison_source_info.key
            {
                return Err(VaultError::InvalidOracleSourceAccount.into());
            }
            let (guard, create_guard) = acquireable_source_challenge_guard(
                program_id,
                month_info.key,
                comparison_source_info,
                &comparison_source_state,
                comparison_guard_info,
            )?;
            (
                *comparison_source_info.key,
                params.comparison_source_id,
                Some((
                    comparison_source_info,
                    comparison_guard_info,
                    guard,
                    create_guard,
                )),
                oracle_source_challenge_bond_backing(
                    source.support_stake_total,
                    Some(comparison_source_state.support_stake_total),
                ),
            )
        } else {
            (
                Pubkey::default(),
                [0; 32],
                None,
                oracle_source_challenge_bond_backing(source.support_stake_total, None),
            )
        };
    let schedule_index = if comparison_guard.is_some() { 11 } else { 9 };
    let schedule_info = accounts
        .get(schedule_index)
        .ok_or(VaultError::InvalidAccountList)?;
    let mut schedule = load_oracle_usdc_reward_schedule(program_id, month_info.key, schedule_info)?;
    if schedule.phase != OracleUsdcRewardSchedulePhase::Funded || sku.schedule != *schedule_info.key
    {
        return Err(VaultError::InvalidOracleUsdcRewardSchedule.into());
    }
    if accounts.len() != schedule_index + 1 {
        return Err(VaultError::InvalidAccountList.into());
    }

    let required_bond = calculate_oracle_usdc_challenge_bond(
        bond_backing_total,
        sku.challenge_bond_bps,
        sku.challenge_min_bond,
        sku.challenge_max_bond,
    )?;
    if params.bond != required_bond {
        return Err(VaultError::InvalidOracleUsdcBond.into());
    }
    let mut collateral =
        load_canonical_user_collateral(program_id, collateral_info, challenger_info.key)?;
    debit_oracle_usdc_available(&mut collateral, required_bond)?;
    create_program_account(
        challenger_info,
        challenge_info,
        system_program_info,
        program_id,
        OracleSourceChallenge::LEN,
        &[
            ORACLE_SOURCE_CHALLENGE_PDA_SEED,
            month_info.key.as_ref(),
            source_info.key.as_ref(),
            &params.challenge_id,
            &[challenge_bump],
        ],
    )?;
    if create_source_guard {
        create_program_account(
            challenger_info,
            source_guard_info,
            system_program_info,
            program_id,
            OracleSourceChallengeGuard::LEN,
            &[
                ORACLE_SOURCE_CHALLENGE_GUARD_PDA_SEED,
                month_info.key.as_ref(),
                source_info.key.as_ref(),
                &[source_guard.bump],
            ],
        )?;
    }
    if let Some((comparison_source_info, comparison_guard_info, guard, create_guard)) =
        &comparison_guard
    {
        if *create_guard {
            create_program_account(
                challenger_info,
                comparison_guard_info,
                system_program_info,
                program_id,
                OracleSourceChallengeGuard::LEN,
                &[
                    ORACLE_SOURCE_CHALLENGE_GUARD_PDA_SEED,
                    month_info.key.as_ref(),
                    comparison_source_info.key.as_ref(),
                    &[guard.bump],
                ],
            )?;
        }
    }
    let slot = Clock::get()?.slot;
    let challenge = OracleSourceChallenge {
        is_initialized: true,
        bump: challenge_bump,
        month: *month_info.key,
        challenge_id: params.challenge_id,
        source: *source_info.key,
        source_id: source.source_id,
        comparison_source,
        comparison_source_id,
        challenger: *challenger_info.key,
        reason: params.reason,
        bond: required_bond,
        required_bond,
        status: OracleChallengeStatus::RuleReview,
        evidence_hash: params.evidence_hash,
        rule_review_slot: slot,
        escrow_disposition: OracleEscrowDisposition::Unsettled,
        emergency_snapshot_total_major_tokens: 0,
        emergency_snapshot_version: 0,
        failed_schedule_escrow_counted: false,
    };
    reserve_source_challenge_guard(
        &mut source_guard,
        challenge_info.key,
        &params.challenge_id,
        slot,
    )?;
    month.pending_resolution_count = month
        .pending_resolution_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    month.last_updated_slot = slot;
    schedule.outstanding_prelisting_escrow_count = schedule
        .outstanding_prelisting_escrow_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    schedule.last_updated_slot = slot;
    store_state(schedule_info, &schedule)?;
    store_state(challenge_info, &challenge)?;
    store_state(source_guard_info, &source_guard)?;
    if let Some((_comparison_source_info, comparison_guard_info, mut guard, _create_guard)) =
        comparison_guard
    {
        reserve_source_challenge_guard(&mut guard, challenge_info.key, &params.challenge_id, slot)?;
        store_state(comparison_guard_info, &guard)?;
    }
    store_state(collateral_info, &collateral)?;
    store_oracle_month_state(month_info, &month)
}
