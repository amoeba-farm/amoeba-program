use super::*;

#[inline(never)]
pub(in crate::processor) fn process_finalize_oracle_update_claim_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: FinalizeOracleUpdateClaimV2Params,
) -> ProgramResult {
    let (accounts, checkpoint_accounts) =
        if params.outcome == OracleUpdateClaimOutcome::RuleReviewUnresolved {
            (accounts, &accounts[accounts.len()..])
        } else {
            let core_len = accounts
                .len()
                .checked_sub(3)
                .ok_or(VaultError::InvalidAccountList)?;
            accounts.split_at(core_len)
        };
    if accounts.len() < 9
        || !accounts[0].is_signer
        || accounts[1].is_signer
        || accounts[1].is_writable
        || accounts[2].is_signer
        || accounts[2].is_writable
        || accounts[3].is_signer
        || !accounts[3].is_writable
        || accounts[4].is_signer
        || !accounts[4].is_writable
        || accounts[5].is_signer
        || !accounts[5].is_writable
        || accounts[6].is_signer
        || accounts[6].is_writable
        || accounts[7].is_signer
        || !accounts[7].is_writable
        || accounts[8].is_signer
        || !accounts[8].is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let trailing = &accounts[9..];
    let trailing_privileges_valid = match params.outcome {
        OracleUpdateClaimOutcome::RuleReviewUnresolved => {
            trailing.len() == 4
                && !trailing[0].is_signer
                && trailing[0].is_writable
                && !trailing[1].is_signer
                && trailing[1].is_writable
                && !trailing[2].is_signer
                && !trailing[2].is_writable
                && !trailing[3].is_signer
                && trailing[3].is_writable
        }
        OracleUpdateClaimOutcome::AcceptClaim | OracleUpdateClaimOutcome::RejectClaim => {
            (trailing.len() == 1 && !trailing[0].is_signer && !trailing[0].is_writable)
                || (trailing.len() == 2
                    && !trailing[0].is_signer
                    && trailing[0].is_writable
                    && !trailing[1].is_signer
                    && !trailing[1].is_writable)
        }
    };
    if !trailing_privileges_valid {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let market_info = &accounts[1];
    let config_info = &accounts[2];
    let month_info = &accounts[3];
    let source_info = &accounts[4];
    let observations_info = &accounts[5];
    let active_manifest_info = &accounts[6];
    let claim_info = &accounts[7];
    let bucket_info = &accounts[8];
    validate_oracle_authority(program_id, authority_info, config_info)?;

    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_current_cash_update_resolution_window(&market, &month)?;
    ensure_oracle_opening_resolution_complete(&month)?;
    let mut source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let source_before = source.clone();
    let mut observations = load_valid_oracle_source_observations(
        program_id,
        month_info.key,
        source_info.key,
        observations_info,
    )?;
    validate_oracle_observation_shape(&source, &observations)?;
    validate_active_oracle_source(
        program_id,
        &month,
        month_info.key,
        active_manifest_info,
        &source,
    )?;
    let mut claim = load_valid_oracle_update_claim_v2_from_account(
        program_id,
        month_info.key,
        source_info.key,
        claim_info,
    )?;
    let mut bucket = load_valid_oracle_bucket_median(program_id, month_info.key, bucket_info)?;
    if bucket.bucket_id != source.bucket_id
        || !matches!(
            bucket.status,
            OracleBucketMedianStatus::Live | OracleBucketMedianStatus::Dirty
        )
    {
        return Err(VaultError::InvalidOracleMedian.into());
    }
    if claim.samba_checkpoint_active {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }

    let mut update_guard = None;
    let mut update_guard_info = None;
    let (challenge_info, staking_pool_info, samba_mint_info) = match params.outcome {
        OracleUpdateClaimOutcome::RuleReviewUnresolved if trailing.len() == 4 => {
            let guard = load_canonical_update_challenge_guard(
                program_id,
                month_info.key,
                claim_info.key,
                &claim.claim.claim_id,
                &trailing[3],
            )?;
            update_guard = Some(guard);
            update_guard_info = Some(&trailing[3]);
            (Some(&trailing[0]), Some(&trailing[1]), Some(&trailing[2]))
        }
        OracleUpdateClaimOutcome::AcceptClaim | OracleUpdateClaimOutcome::RejectClaim
            if matches!(trailing.len(), 1 | 2) =>
        {
            let guard_info = trailing.last().ok_or(VaultError::InvalidAccountList)?;
            let (expected_guard, _) = derive_oracle_update_challenge_guard_pda(
                program_id,
                month_info.key,
                claim_info.key,
            );
            if guard_info.owner == program_id {
                if trailing.len() != 2 {
                    return Err(VaultError::InvalidAccountList.into());
                }
                let guard = load_canonical_update_challenge_guard(
                    program_id,
                    month_info.key,
                    claim_info.key,
                    &claim.claim.claim_id,
                    guard_info,
                )?;
                update_guard = Some(guard);
                update_guard_info = Some(guard_info);
                (Some(&trailing[0]), None, None)
            } else {
                if trailing.len() != 1 || params.outcome != OracleUpdateClaimOutcome::AcceptClaim {
                    return Err(VaultError::InvalidOracleUpdateAccount.into());
                }
                validate_canonical_system_zero_pda_proof(&expected_guard, guard_info)
                    .map_err(|_| ProgramError::from(VaultError::InvalidOracleUpdateAccount))?;
                (None, None, None)
            }
        }
        _ => return Err(VaultError::InvalidAccountList.into()),
    };

    let mut guarded_challenge = if let Some(challenge_info) = challenge_info {
        let challenge =
            load_valid_oracle_update_challenge(program_id, month_info.key, challenge_info)?;
        if challenge.claim != *claim_info.key
            || challenge.claim_id != claim.claim.claim_id
            || challenge.status != OracleChallengeStatus::RuleReview
        {
            return Err(VaultError::InvalidOracleUpdateAccount.into());
        }
        let guard = update_guard
            .as_ref()
            .ok_or(VaultError::InvalidOracleUpdateAccount)?;
        if guard.challenge != *challenge_info.key
            || guard.challenge_id != challenge.challenge_id
            || !crate::pubkey_is_default(&guard.active_dispute)
            || guard.resolution_step != 0
        {
            return Err(VaultError::InvalidOracleUpdateAccount.into());
        }
        Some(challenge)
    } else {
        None
    };

    ensure_live_revealed_oracle_update_claim(source_info.key, &source, &claim.claim)?;
    if params.current_step == 0 || params.current_step <= source.last_finalized_step {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    let slot = Clock::get()?.slot;
    match params.outcome {
        OracleUpdateClaimOutcome::AcceptClaim => {
            append_oracle_source_observation(
                &mut source,
                &mut observations,
                claim.claim.new_state,
                claim.claim.source_time,
                &claim.claim.evidence_hash,
                &claim.claim.archive_url_hash,
            )?;
            source.current_state = claim.claim.new_state;
            source.last_finalized_step = params.current_step;
            claim.claim.status = OracleClaimStatus::Finalized;
            crate::processor::oracle_carry::record_fresh_accept(
                program_id,
                authority_info,
                source_info.key,
                &source_before,
                &source,
                &observations,
                crate::processor::oracle_carry::AcceptedEvent {
                    event: *claim_info.key,
                    value: claim.claim.new_state,
                    observed_at: claim.claim.source_time,
                    evidence_hash: claim.claim.evidence_hash,
                    archive_hash: claim.claim.archive_url_hash,
                    contributor: claim.claim.claimant,
                },
                checkpoint_accounts,
            )?;
            if let Some(challenge) = guarded_challenge.as_mut() {
                challenge.status = OracleChallengeStatus::Rejected;
            }
            month.accepted_cash_update_count = month
                .accepted_cash_update_count
                .checked_add(1)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
        OracleUpdateClaimOutcome::RejectClaim => {
            let challenge = guarded_challenge
                .as_mut()
                .ok_or(VaultError::InvalidOracleUpdateAccount)?;
            append_oracle_source_observation(
                &mut source,
                &mut observations,
                challenge.alternative_state,
                challenge.alternative_source_time,
                &challenge.evidence_hash,
                &challenge.archive_url_hash,
            )?;
            source.current_state = challenge.alternative_state;
            source.last_finalized_step = params.current_step;
            claim.claim.status = OracleClaimStatus::Rejected;
            crate::processor::oracle_carry::record_fresh_accept(
                program_id,
                authority_info,
                source_info.key,
                &source_before,
                &source,
                &observations,
                crate::processor::oracle_carry::AcceptedEvent {
                    event: *claim_info.key,
                    value: challenge.alternative_state,
                    observed_at: challenge.alternative_source_time,
                    evidence_hash: challenge.evidence_hash,
                    archive_hash: challenge.archive_url_hash,
                    contributor: challenge.challenger,
                },
                checkpoint_accounts,
            )?;
            challenge.status = OracleChallengeStatus::Accepted;
        }
        OracleUpdateClaimOutcome::RuleReviewUnresolved => {
            let challenge = guarded_challenge
                .as_mut()
                .ok_or(VaultError::InvalidOracleUpdateAccount)?;
            challenge.status = OracleChallengeStatus::RuleReviewUnresolved;
            challenge.rule_review_slot = slot;
            let staking_pool_info = staking_pool_info.ok_or(VaultError::InvalidAccountList)?;
            let mut staking_pool = load_canonical_oracle_staking_pool(
                program_id,
                staking_pool_info,
                &derive_oracle_major_token_config_pda(program_id).0,
            )?;
            let samba_mint = validate_oracle_samba_mint(
                &staking_pool,
                samba_mint_info.ok_or(VaultError::InvalidAccountList)?,
                config_info.key,
            )?;
            challenge.emergency_snapshot_total_major_tokens =
                prepare_oracle_samba_voting_snapshot(&mut staking_pool, samba_mint.supply, slot)?;
            challenge.account_version = OracleUpdateChallenge::ACCOUNT_VERSION;
            let guard = update_guard
                .as_mut()
                .ok_or(VaultError::InvalidOracleUpdateAccount)?;
            guard.resolution_step = params.current_step;
            guard.last_updated_slot = slot;
            claim.samba_checkpoint_active = true;
            store_state(staking_pool_info, &staking_pool)?;
        }
    }
    if params.outcome != OracleUpdateClaimOutcome::RuleReviewUnresolved {
        bucket.status = OracleBucketMedianStatus::Dirty;
        bucket.recompute_processed_source_count = 0;
        decrement_pending_oracle_resolution(&mut month)?;
    }
    let (c_settle_bps, settlement_status) =
        oracle_settlement_status(month.c_raw_bps, month.g_camo_bps, month.g_thin_bps);
    month.c_settle_bps = c_settle_bps;
    month.settlement_status = settlement_status;
    month.last_updated_slot = slot;
    store_state(source_info, &source)?;
    store_state(observations_info, &observations)?;
    if params.outcome != OracleUpdateClaimOutcome::RuleReviewUnresolved {
        store_state(bucket_info, &bucket)?;
    }
    if let (Some(challenge_info), Some(challenge)) = (challenge_info, guarded_challenge.as_ref()) {
        store_state(challenge_info, challenge)?;
    }
    if params.outcome == OracleUpdateClaimOutcome::RuleReviewUnresolved {
        let guard_info = update_guard_info.ok_or(VaultError::InvalidAccountList)?;
        let guard = update_guard
            .as_ref()
            .ok_or(VaultError::InvalidOracleUpdateAccount)?;
        store_state(guard_info, guard)?;
    }
    store_state(claim_info, &claim)?;
    store_oracle_month_state(month_info, &month)
}

#[inline(never)]
pub(in crate::processor) fn process_cancel_stale_oracle_update_claim_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() < 6 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let source_info = &accounts[3];
    let claim_info = &accounts[4];
    let guard_info = &accounts[5];
    let trailing = &accounts[6..];
    if !cranker_info.is_signer || !month_info.is_writable || !claim_info.is_writable {
        return Err(VaultError::InvalidAccountList.into());
    }

    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    if month.phase != OraclePhase::Game || month.listing_ts >= market.instrument.expiry_ts {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    rulebook_schedule_boundaries(&month)?;
    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let mut claim = load_valid_oracle_update_claim_v2_from_account(
        program_id,
        month_info.key,
        source_info.key,
        claim_info,
    )?;
    if claim.claim.source_id != source.source_id
        || claim.claim.status != OracleClaimStatus::Revealed
        || claim.claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || claim.claim.stake == 0
        || claim.claim.prior_state == 0
        || claim.claim.new_state == 0
        || claim.claim.prior_state == claim.claim.new_state
        || crate::bytes32_is_zero(&claim.claim.evidence_hash)
        || claim.revealed_slot == 0
        || month.pending_resolution_count == 0
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }

    let (expected_guard, _) =
        derive_oracle_update_challenge_guard_pda(program_id, month_info.key, claim_info.key);
    let clock = Clock::get()?;
    let slot = clock.slot;
    let now = u64::try_from(clock.unix_timestamp)
        .map_err(|_| ProgramError::from(VaultError::InvalidSettlementRecord))?;
    if guard_info.owner != program_id {
        if !trailing.is_empty() || claim.samba_checkpoint_active {
            return Err(VaultError::InvalidAccountList.into());
        }
        validate_canonical_system_zero_pda_proof(&expected_guard, guard_info)
            .map_err(|_| ProgramError::from(VaultError::InvalidOracleUpdateAccount))?;
        ensure_oracle_update_cleanup_ready_at(&market, &source, &claim, None, now)?;
        claim.claim.status = OracleClaimStatus::TimedOut;
        decrement_pending_oracle_resolution(&mut month)?;
        month.last_updated_slot = slot;
        store_state(claim_info, &claim)?;
        return store_oracle_month_state(month_info, &month);
    }

    if !guard_info.is_writable || !matches!(trailing.len(), 1 | 2) {
        return Err(VaultError::InvalidAccountList.into());
    }
    let challenge_info = &trailing[0];
    if !challenge_info.is_writable {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut guard = load_canonical_update_challenge_guard(
        program_id,
        month_info.key,
        claim_info.key,
        &claim.claim.claim_id,
        guard_info,
    )?;
    let mut challenge =
        load_exact_current_oracle_update_challenge(program_id, month_info.key, challenge_info)?;
    if guard.challenge != *challenge_info.key
        || guard.challenge_id != challenge.challenge_id
        || !crate::pubkey_is_default(&guard.active_dispute)
        || challenge.claim != *claim_info.key
        || challenge.claim_id != claim.claim.claim_id
        || challenge.bond == 0
        || challenge.bond != challenge.required_bond
        || crate::bytes32_is_zero(&challenge.evidence_hash)
        || challenge.escrow_disposition != OracleEscrowDisposition::Unsettled
        || challenge.rule_review_slot == 0
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }

    let mut staking_pool = None;
    let staking_pool_info = if trailing.len() == 2 {
        let pool_info = &trailing[1];
        if !pool_info.is_writable
            || !claim.samba_checkpoint_active
            || guard.resolution_step == 0
            || challenge.status != OracleChallengeStatus::RuleReviewUnresolved
            || challenge.emergency_snapshot_total_major_tokens == 0
        {
            return Err(VaultError::InvalidOracleUpdateAccount.into());
        }
        let pool = load_canonical_oracle_staking_pool(
            program_id,
            pool_info,
            &derive_oracle_major_token_config_pda(program_id).0,
        )?;
        if pool.governance_lock_count == 0
            || pool.samba_supply != challenge.emergency_snapshot_total_major_tokens
        {
            return Err(VaultError::InvalidOracleStakingPool.into());
        }
        staking_pool = Some(pool);
        Some(pool_info)
    } else {
        if claim.samba_checkpoint_active
            || guard.resolution_step != 0
            || challenge.status != OracleChallengeStatus::RuleReview
            || challenge.emergency_snapshot_total_major_tokens != 0
        {
            return Err(VaultError::InvalidOracleUpdateAccount.into());
        }
        None
    };

    ensure_oracle_update_cleanup_ready_at(&market, &source, &claim, Some(&guard), now)?;
    claim.claim.status = OracleClaimStatus::TimedOut;
    claim.samba_checkpoint_active = false;
    challenge.status = OracleChallengeStatus::Cancelled;
    guard.last_updated_slot = slot;
    if let Some(pool) = staking_pool.as_mut() {
        pool.governance_lock_count = pool
            .governance_lock_count
            .checked_sub(1)
            .ok_or(VaultError::InvalidOracleStakingPool)?;
        pool.last_updated_slot = slot;
    }
    decrement_pending_oracle_resolution(&mut month)?;
    month.last_updated_slot = slot;

    store_state(claim_info, &claim)?;
    store_state(challenge_info, &challenge)?;
    store_state(guard_info, &guard)?;
    if let (Some(pool_info), Some(pool)) = (staking_pool_info, staking_pool.as_ref()) {
        store_state(pool_info, pool)?;
    }
    store_oracle_month_state(month_info, &month)
}

#[inline(never)]
pub(in crate::processor) fn process_settle_expired_oracle_update_commitment_v3(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 6 || !accounts[0].is_signer {
        return Err(VaultError::InvalidAccountList.into());
    }
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let source_info = &accounts[3];
    let claim_info = &accounts[4];
    let claimant_collateral_info = &accounts[5];
    let (_market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let mut claim = load_valid_oracle_update_claim_v2_from_account(
        program_id,
        month_info.key,
        source_info.key,
        claim_info,
    )?;
    let slot = Clock::get()?.slot;
    if claim.claim.source_id != source.source_id
        || claim.claim.status != OracleClaimStatus::Committed
        || claim.claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || claim.claim.prior_state != 0
        || claim.claim.new_state != 0
        || !crate::bytes32_is_zero(&claim.claim.evidence_hash)
        || slot <= claim.reveal_deadline_slot
    {
        return Err(VaultError::OracleEscrowNotSettleable.into());
    }
    let mut collateral = load_canonical_user_collateral(
        program_id,
        claimant_collateral_info,
        &claim.claim.claimant,
    )?;
    credit_oracle_usdc_available(&mut collateral, claim.claim.stake)?;
    claim.claim.escrow_disposition = OracleEscrowDisposition::Refunded;
    claim.claim.status = OracleClaimStatus::TimedOut;
    decrement_pending_oracle_resolution(&mut month)?;
    month.last_updated_slot = slot;
    store_state(claim_info, &claim)?;
    store_state(claimant_collateral_info, &collateral)?;
    store_oracle_month_state(month_info, &month)
}

#[inline(never)]
pub(in crate::processor) fn process_challenge_oracle_update_claim_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ChallengeOracleUpdateClaimParams,
) -> ProgramResult {
    if accounts.len() != 10 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let challenger_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let sku_info = &accounts[3];
    let claim_info = &accounts[4];
    let source_info = &accounts[5];
    let collateral_info = &accounts[6];
    let challenge_info = &accounts[7];
    let system_program_info = &accounts[8];
    let challenge_guard_info = &accounts[9];
    if !challenger_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_system_program(system_program_info)?;
    if crate::bytes32_is_zero(&params.challenge_id)
        || params.alternative_state == 0
        || params.alternative_source_time == 0
        || crate::bytes32_is_zero(&params.evidence_hash)
        || params.archive_url.is_empty()
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    let (market, month) = load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_game_window(&market, &month)?;
    ensure_oracle_opening_resolution_complete(&month)?;
    let claim = load_valid_oracle_update_claim_v2_from_account(
        program_id,
        month_info.key,
        source_info.key,
        claim_info,
    )?;
    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let archive_url_hash = validate_oracle_update_evidence(
        &market,
        &month,
        month_info.key,
        source_info.key,
        &source,
        params.alternative_state,
        params.alternative_source_time,
        &params.evidence_hash,
        &params.archive_url,
    )?;
    let sku = load_oracle_usdc_sku_for_month(program_id, month_info.key, sku_info)?;
    ensure_live_revealed_oracle_update_claim(source_info.key, &source, &claim.claim)?;
    if source.bucket_id != sku.bucket_id
        || claim.claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || claim.claim.claimant == *challenger_info.key
        || params.alternative_state == claim.claim.new_state
        || params.alternative_state == source.current_state
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    let required_bond = calculate_oracle_usdc_challenge_bond(
        claim.claim.stake,
        sku.challenge_bond_bps,
        sku.challenge_min_bond,
        sku.challenge_max_bond,
    )?;
    if params.bond != required_bond {
        return Err(VaultError::InvalidOracleUsdcBond.into());
    }
    let (expected, bump) = derive_oracle_update_challenge_pda(
        program_id,
        month_info.key,
        claim_info.key,
        &params.challenge_id,
    );
    if *challenge_info.key != expected {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    validate_create_only_program_account_target(program_id, challenge_info)?;
    let (expected_guard, guard_bump) =
        derive_oracle_update_challenge_guard_pda(program_id, month_info.key, claim_info.key);
    if *challenge_guard_info.key != expected_guard
        || challenge_guard_info.key == challenge_info.key
        || challenge_guard_info.key == claim_info.key
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    validate_create_only_program_account_target(program_id, challenge_guard_info)?;
    let mut collateral =
        load_canonical_user_collateral(program_id, collateral_info, challenger_info.key)?;
    debit_oracle_usdc_available(&mut collateral, required_bond)?;
    create_program_account(
        challenger_info,
        challenge_info,
        system_program_info,
        program_id,
        OracleUpdateChallenge::LEN,
        &[
            ORACLE_UPDATE_CHALLENGE_PDA_SEED,
            month_info.key.as_ref(),
            claim_info.key.as_ref(),
            &params.challenge_id,
            &[bump],
        ],
    )?;
    create_program_account(
        challenger_info,
        challenge_guard_info,
        system_program_info,
        program_id,
        OracleUpdateChallengeGuard::LEN,
        &[
            ORACLE_UPDATE_CHALLENGE_GUARD_PDA_SEED,
            month_info.key.as_ref(),
            claim_info.key.as_ref(),
            &[guard_bump],
        ],
    )?;
    let slot = Clock::get()?.slot;
    let challenge = OracleUpdateChallenge {
        is_initialized: true,
        bump,
        month: *month_info.key,
        challenge_id: params.challenge_id,
        claim: *claim_info.key,
        claim_id: claim.claim.claim_id,
        challenger: *challenger_info.key,
        alternative_state: params.alternative_state,
        bond: required_bond,
        required_bond,
        status: OracleChallengeStatus::RuleReview,
        evidence_hash: params.evidence_hash,
        alternative_source_time: params.alternative_source_time,
        archive_url_hash,
        rule_review_slot: slot,
        escrow_disposition: OracleEscrowDisposition::Unsettled,
        account_discriminator: OracleUpdateChallenge::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUpdateChallenge::ACCOUNT_VERSION,
        emergency_snapshot_total_major_tokens: 0,
    };
    let challenge_guard = OracleUpdateChallengeGuard {
        is_initialized: true,
        bump: guard_bump,
        account_discriminator: OracleUpdateChallengeGuard::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUpdateChallengeGuard::ACCOUNT_VERSION,
        month: *month_info.key,
        claim: *claim_info.key,
        claim_id: claim.claim.claim_id,
        challenge: *challenge_info.key,
        challenge_id: params.challenge_id,
        active_dispute: Pubkey::default(),
        resolution_step: 0,
        created_slot: slot,
        last_updated_slot: slot,
    };
    store_state(challenge_info, &challenge)?;
    store_state(challenge_guard_info, &challenge_guard)?;
    store_state(collateral_info, &collateral)
}
