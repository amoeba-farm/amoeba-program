use super::*;

#[inline(never)]
pub(in crate::processor) fn process_submit_oracle_opening_claim_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SubmitOracleOpeningClaimParams,
) -> ProgramResult {
    if accounts.len() != 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let claimant_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let sku_info = &accounts[3];
    let source_info = &accounts[4];
    let collateral_info = &accounts[5];
    let claim_info = &accounts[6];
    let system_program_info = &accounts[7];
    if !claimant_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_system_program(system_program_info)?;

    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_opening_window(&month)?;
    ensure_oracle_canonical_weight_scheme(&month)?;
    let mut source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let sku = load_oracle_usdc_sku_for_month(program_id, month_info.key, sku_info)?;
    if source.bucket_id != sku.bucket_id || params.stake != sku.opening_bond {
        return Err(VaultError::InvalidOracleUsdcBond.into());
    }
    if source.status == OracleSourceStatus::OpeningPending {
        return Err(VaultError::OracleOpeningClaimPending.into());
    }
    if source.status == OracleSourceStatus::Active || source.opening_submitted {
        return Err(VaultError::OracleOpeningAlreadySubmitted.into());
    }
    if source.status != OracleSourceStatus::Frozen {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    let evidence_hash = validate_oracle_opening_evidence(
        &market,
        &month,
        month_info.key,
        source_info.key,
        &source,
        &params,
    )?;
    let (expected_claim, claim_bump) =
        derive_oracle_opening_claim_pda(program_id, month_info.key, source_info.key);
    if *claim_info.key != expected_claim {
        return Err(VaultError::InvalidOracleOpeningClaim.into());
    }
    let attempt = if claim_info.owner != program_id {
        validate_create_only_program_account_target(program_id, claim_info)?;
        create_program_account(
            claimant_info,
            claim_info,
            system_program_info,
            program_id,
            OracleOpeningClaim::LEN,
            &[
                ORACLE_OPENING_CLAIM_PDA_SEED,
                month_info.key.as_ref(),
                source_info.key.as_ref(),
                &[claim_bump],
            ],
        )?;
        1
    } else {
        let existing: OracleOpeningClaim = load_state(claim_info, program_id)?;
        if !existing.is_initialized
            || existing.month != *month_info.key
            || existing.source != *source_info.key
            || existing.source_id != source.source_id
            || existing.status != OracleOpeningClaimStatus::Rejected
            || existing.escrow_disposition == OracleEscrowDisposition::Unsettled
        {
            return Err(VaultError::OracleOpeningClaimPending.into());
        }
        existing
            .attempt
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?
    };
    let mut collateral =
        load_canonical_user_collateral(program_id, collateral_info, claimant_info.key)?;
    debit_oracle_usdc_available(&mut collateral, sku.opening_bond)?;

    let slot = Clock::get()?.slot;
    let claim = OracleOpeningClaim {
        is_initialized: true,
        bump: claim_bump,
        month: *month_info.key,
        source: *source_info.key,
        source_id: source.source_id,
        attempt,
        claimant: *claimant_info.key,
        opening_state: params.opening_state,
        source_time: params.source_time,
        stake: sku.opening_bond,
        canonical_locator_hash: params.canonical_locator_hash,
        source_definition_hash: params.source_definition_hash,
        evidence_hash,
        archive_url_hash: derive_oracle_opening_archive_url_hash(&params.archive_url),
        submitted_slot: slot,
        challenge_deadline_slot: slot
            .checked_add(ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS)
            .ok_or(VaultError::ArithmeticOverflow)?,
        status: OracleOpeningClaimStatus::Pending,
        escrow_disposition: OracleEscrowDisposition::Unsettled,
    };
    source.status = OracleSourceStatus::OpeningPending;
    month.pending_resolution_count = month
        .pending_resolution_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    month.last_updated_slot = slot;
    store_state(source_info, &source)?;
    store_state(claim_info, &claim)?;
    store_state(collateral_info, &collateral)?;
    store_oracle_month_state(month_info, &month)
}

#[inline(never)]
pub(in crate::processor) fn process_challenge_oracle_opening_claim_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ChallengeOracleOpeningClaimParams,
) -> ProgramResult {
    if accounts.len() != 9 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let challenger_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let sku_info = &accounts[3];
    let source_info = &accounts[4];
    let claim_info = &accounts[5];
    let collateral_info = &accounts[6];
    let challenge_info = &accounts[7];
    let system_program_info = &accounts[8];
    if !challenger_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_system_program(system_program_info)?;
    if crate::bytes32_is_zero(&params.challenge_id) {
        return Err(VaultError::InvalidOracleOpeningEvidence.into());
    }

    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_opening_window(&month)?;
    ensure_oracle_canonical_weight_scheme(&month)?;
    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let sku = load_oracle_usdc_sku_for_month(program_id, month_info.key, sku_info)?;
    let mut claim = load_valid_oracle_opening_claim(
        program_id,
        month_info.key,
        source_info.key,
        claim_info,
        &source,
    )?;
    let slot = Clock::get()?.slot;
    if source.bucket_id != sku.bucket_id
        || claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || source.status != OracleSourceStatus::OpeningPending
        || claim.status != OracleOpeningClaimStatus::Pending
        || claim.claimant == *challenger_info.key
        || slot >= claim.challenge_deadline_slot
        || oracle_opening_challenge_is_true_noop(&claim, &params)
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    let evidence_hash = validate_oracle_opening_alternative_evidence(
        &market,
        &month,
        month_info.key,
        source_info.key,
        &source,
        &params,
    )?;
    let required_bond = calculate_oracle_usdc_challenge_bond(
        claim.stake,
        sku.challenge_bond_bps,
        sku.challenge_min_bond,
        sku.challenge_max_bond,
    )?;
    if params.bond != required_bond {
        return Err(VaultError::InvalidOracleUsdcBond.into());
    }
    let (expected, bump) = derive_oracle_opening_claim_challenge_pda(
        program_id,
        month_info.key,
        claim_info.key,
        &params.challenge_id,
    );
    if *challenge_info.key != expected {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    validate_create_only_program_account_target(program_id, challenge_info)?;
    let mut collateral =
        load_canonical_user_collateral(program_id, collateral_info, challenger_info.key)?;
    debit_oracle_usdc_available(&mut collateral, required_bond)?;
    create_program_account(
        challenger_info,
        challenge_info,
        system_program_info,
        program_id,
        OracleOpeningClaimChallenge::LEN,
        &[
            ORACLE_OPENING_CLAIM_CHALLENGE_PDA_SEED,
            month_info.key.as_ref(),
            claim_info.key.as_ref(),
            &params.challenge_id,
            &[bump],
        ],
    )?;

    let challenge = OracleOpeningClaimChallenge {
        is_initialized: true,
        bump,
        month: *month_info.key,
        challenge_id: params.challenge_id,
        claim: *claim_info.key,
        claim_attempt: claim.attempt,
        source: *source_info.key,
        source_id: source.source_id,
        challenger: *challenger_info.key,
        alternative_opening_state: params.alternative_opening_state,
        alternative_source_time: params.alternative_source_time,
        bond: required_bond,
        required_bond,
        status: OracleChallengeStatus::RuleReview,
        canonical_locator_hash: params.canonical_locator_hash,
        source_definition_hash: params.source_definition_hash,
        evidence_hash,
        archive_url_hash: derive_oracle_opening_archive_url_hash(&params.archive_url),
        rule_review_slot: slot,
        escrow_disposition: OracleEscrowDisposition::Unsettled,
        account_discriminator: OracleOpeningClaimChallenge::ACCOUNT_DISCRIMINATOR,
        account_version: OracleOpeningClaimChallenge::ACCOUNT_VERSION,
        emergency_snapshot_total_major_tokens: 0,
    };
    claim.status = OracleOpeningClaimStatus::Challenged;
    month.pending_resolution_count = month
        .pending_resolution_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    month.last_updated_slot = slot;
    store_state(claim_info, &claim)?;
    store_state(challenge_info, &challenge)?;
    store_state(collateral_info, &collateral)?;
    store_oracle_month_state(month_info, &month)
}

#[inline(never)]
pub(in crate::processor) fn process_resolve_oracle_opening_claim_challenge_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ResolveOracleOpeningClaimChallengeParams,
) -> ProgramResult {
    if accounts.len() < 9 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let config_info = &accounts[1];
    let month_info = &accounts[2];
    let sku_info = &accounts[3];
    let source_info = &accounts[4];
    let claim_info = &accounts[5];
    let claimant_collateral_info = &accounts[6];
    let challenger_collateral_info = &accounts[7];
    let challenge_info = &accounts[8];
    validate_oracle_authority(program_id, authority_info, config_info)?;
    let staking_pool_info = if params.outcome == OracleOpeningChallengeOutcome::RuleReviewUnresolved
    {
        Some(accounts.get(9).ok_or(VaultError::InvalidAccountList)?)
    } else {
        None
    };
    let samba_mint_info = if params.outcome == OracleOpeningChallengeOutcome::RuleReviewUnresolved {
        Some(accounts.get(10).ok_or(VaultError::InvalidAccountList)?)
    } else {
        None
    };
    let expected_account_count = if staking_pool_info.is_some() { 11 } else { 9 };
    if accounts.len() != expected_account_count {
        return Err(VaultError::InvalidAccountList.into());
    }

    if month_info.owner != program_id || month_info.data_len() != OracleMonthState::LEN {
        return Err(VaultError::InvalidOracleState.into());
    }
    let month_data = month_info.try_borrow_data()?;
    let mut month = crate::fixed_codec::decode_oracle_month(
        month_data.as_ref(),
        VaultError::InvalidOracleState,
    )?;
    drop(month_data);
    ensure_oracle_opening_window(&month)?;
    let mut source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let sku = load_oracle_usdc_sku_for_month(program_id, month_info.key, sku_info)?;
    let mut claim = load_valid_oracle_opening_claim(
        program_id,
        month_info.key,
        source_info.key,
        claim_info,
        &source,
    )?;
    let mut challenge =
        load_valid_oracle_opening_challenge(program_id, month_info.key, challenge_info)?;
    if source.bucket_id != sku.bucket_id
        || claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || challenge.escrow_disposition != OracleEscrowDisposition::Unsettled
        || challenge.claim != *claim_info.key
        || challenge.claim_attempt != claim.attempt
        || challenge.source != *source_info.key
        || challenge.source_id != source.source_id
        || challenge.canonical_locator_hash != source.canonical_locator_hash
        || challenge.source_definition_hash != source.source_definition_hash
        || challenge.status != OracleChallengeStatus::RuleReview
        || crate::bytes32_is_zero(&challenge.archive_url_hash)
        || crate::bytes32_is_zero(&challenge.evidence_hash)
        || claim.status != OracleOpeningClaimStatus::Challenged
        || source.status != OracleSourceStatus::OpeningPending
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    let _claimant_collateral =
        load_canonical_user_collateral(program_id, claimant_collateral_info, &claim.claimant)?;
    let _challenger_collateral = load_canonical_user_collateral(
        program_id,
        challenger_collateral_info,
        &challenge.challenger,
    )?;

    let slot = Clock::get()?.slot;
    match params.outcome {
        OracleOpeningChallengeOutcome::KeepOpening => {
            challenge.status = OracleChallengeStatus::Rejected;
            decrement_pending_oracle_resolution(&mut month)?;
        }
        OracleOpeningChallengeOutcome::AcceptAlternativeOpening => {
            challenge.status = OracleChallengeStatus::Accepted;
            decrement_pending_oracle_resolution(&mut month)?;
        }
        OracleOpeningChallengeOutcome::RejectOpeningForRetry => {
            claim.status = OracleOpeningClaimStatus::Rejected;
            source.status = OracleSourceStatus::Frozen;
            challenge.status = OracleChallengeStatus::Accepted;
            decrement_pending_oracle_resolution(&mut month)?;
            decrement_pending_oracle_resolution(&mut month)?;
        }
        OracleOpeningChallengeOutcome::SourceInactiveForMonth => {
            claim.status = OracleOpeningClaimStatus::Rejected;
            source.status = OracleSourceStatus::Inactive;
            challenge.status = OracleChallengeStatus::Accepted;
            month.opening_resolved_source_count = month
                .opening_resolved_source_count
                .checked_add(1)
                .ok_or(VaultError::ArithmeticOverflow)?;
            decrement_pending_oracle_resolution(&mut month)?;
            decrement_pending_oracle_resolution(&mut month)?;
        }
        OracleOpeningChallengeOutcome::RuleReviewUnresolved => {
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
            challenge.status = OracleChallengeStatus::RuleReviewUnresolved;
            challenge.rule_review_slot = slot;
            challenge.emergency_snapshot_total_major_tokens =
                prepare_oracle_samba_voting_snapshot(&mut staking_pool, samba_mint.supply, slot)?;
            challenge.account_version = OracleOpeningClaimChallenge::ACCOUNT_VERSION;
            store_state(staking_pool_info, &staking_pool)?;
        }
    }
    month.last_updated_slot = slot;
    store_state(source_info, &source)?;
    store_state(claim_info, &claim)?;
    store_state(challenge_info, &challenge)?;
    store_oracle_month_state(month_info, &month)
}

#[inline(never)]
pub(in crate::processor) fn process_finalize_oracle_opening_claim_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 7 || !accounts[0].is_signer || !accounts[4].is_writable {
        return Err(VaultError::InvalidAccountList.into());
    }
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let source_info = &accounts[3];
    let observations_info = &accounts[4];
    let claim_info = &accounts[5];
    let claimant_collateral_info = &accounts[6];

    let (_market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_opening_window(&month)?;
    let mut source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let mut observations = load_valid_oracle_source_observations(
        program_id,
        month_info.key,
        source_info.key,
        observations_info,
    )?;
    let mut claim = load_valid_oracle_opening_claim(
        program_id,
        month_info.key,
        source_info.key,
        claim_info,
        &source,
    )?;
    let slot = Clock::get()?.slot;
    if claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || source.status != OracleSourceStatus::OpeningPending
        || source.opening_submitted
        || claim.status != OracleOpeningClaimStatus::Pending
        || claim.challenge_deadline_slot == 0
        || slot < claim.challenge_deadline_slot
        || claim.opening_state == 0
        || claim.source_time == 0
        || crate::bytes32_is_zero(&claim.evidence_hash)
        || crate::bytes32_is_zero(&claim.archive_url_hash)
    {
        return Err(VaultError::OracleOpeningClaimNotFinalizable.into());
    }
    let _claimant_collateral =
        load_canonical_user_collateral(program_id, claimant_collateral_info, &claim.claimant)?;
    append_oracle_source_observation(
        &mut source,
        &mut observations,
        claim.opening_state,
        claim.source_time,
        &claim.evidence_hash,
        &claim.archive_url_hash,
    )?;
    source.baseline_state = claim.opening_state;
    source.current_state = claim.opening_state;
    source.opening_submitted = true;
    source.opening_evidence_hash = claim.evidence_hash;
    source.status = OracleSourceStatus::Active;
    claim.status = OracleOpeningClaimStatus::Accepted;
    month.opened_source_count = month
        .opened_source_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    month.opening_resolved_source_count = month
        .opening_resolved_source_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    decrement_pending_oracle_resolution(&mut month)?;
    month.last_updated_slot = slot;
    store_state(source_info, &source)?;
    store_state(observations_info, &observations)?;
    store_state(claim_info, &claim)?;
    store_oracle_month_state(month_info, &month)
}
