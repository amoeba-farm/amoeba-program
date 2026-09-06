use super::*;

#[inline(never)]
pub(in crate::processor) fn process_commit_oracle_update_claim_v3(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: CommitOracleUpdateClaimV2Params,
) -> ProgramResult {
    if accounts.len() != 9 || !accounts[0].is_signer {
        return Err(VaultError::InvalidAccountList.into());
    }
    let claimant_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let source_info = &accounts[3];
    let active_manifest_info = &accounts[4];
    let sku_info = &accounts[5];
    let claim_info = &accounts[6];
    let collateral_info = &accounts[7];
    let system_program_info = &accounts[8];
    validate_system_program(system_program_info)?;
    if crate::bytes32_is_zero(&params.claim_id)
        || crate::bytes32_is_zero(&params.commit_hash)
        || params.stake == 0
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }

    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_game_window(&market, &month)?;
    ensure_oracle_opening_resolution_complete(&month)?;
    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    validate_active_oracle_source(
        program_id,
        &month,
        month_info.key,
        active_manifest_info,
        &source,
    )?;
    let sku = load_oracle_usdc_sku_for_month(program_id, month_info.key, sku_info)?;
    if source.status != OracleSourceStatus::Active
        || source.bucket_id != sku.bucket_id
        || source.current_state == 0
        || params.stake != sku.update_min_bond
    {
        return Err(VaultError::InvalidOracleUsdcBond.into());
    }
    let (expected, bump) = derive_oracle_update_claim_v2_pda(
        program_id,
        month_info.key,
        source_info.key,
        claimant_info.key,
        &params.claim_id,
    );
    if *claim_info.key != expected {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    validate_create_only_program_account_target(program_id, claim_info)?;
    let mut collateral =
        load_canonical_user_collateral(program_id, collateral_info, claimant_info.key)?;
    debit_oracle_usdc_available(&mut collateral, sku.update_min_bond)?;
    create_program_account(
        claimant_info,
        claim_info,
        system_program_info,
        program_id,
        OracleUpdateClaimV2::LEN,
        &[
            ORACLE_UPDATE_CLAIM_V2_PDA_SEED,
            month_info.key.as_ref(),
            source_info.key.as_ref(),
            claimant_info.key.as_ref(),
            &params.claim_id,
            &[bump],
        ],
    )?;

    let slot = Clock::get()?.slot;
    let earliest_reveal_slot = slot
        .checked_add(ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let reveal_deadline_slot = earliest_reveal_slot
        .checked_add(ORACLE_UPDATE_REVEAL_WINDOW_SLOTS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let claim = OracleUpdateClaimV2 {
        claim: OracleUpdateClaimData {
            is_initialized: true,
            bump,
            month: *month_info.key,
            claim_id: params.claim_id,
            source: *source_info.key,
            source_id: source.source_id,
            claimant: *claimant_info.key,
            prior_state: 0,
            new_state: 0,
            stake: sku.update_min_bond,
            status: OracleClaimStatus::Committed,
            evidence_hash: [0; 32],
            source_time: 0,
            archive_url_hash: [0; 32],
            escrow_disposition: OracleEscrowDisposition::Unsettled,
            account_discriminator: OracleUpdateClaimV2::ACCOUNT_DISCRIMINATOR,
            account_version: OracleUpdateClaimV2::ACCOUNT_VERSION,
        },
        commit_hash: params.commit_hash,
        commit_slot: slot,
        earliest_reveal_slot,
        reveal_deadline_slot,
        revealed_slot: 0,
        samba_checkpoint_active: false,
        freshness_reward_multiplier: 1,
    };
    month.pending_resolution_count = month
        .pending_resolution_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    month.last_updated_slot = slot;
    store_state(claim_info, &claim)?;
    store_state(collateral_info, &collateral)?;
    store_oracle_month_state(month_info, &month)
}

#[inline(never)]
pub(in crate::processor) fn process_reveal_oracle_update_claim_v3(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: RevealOracleUpdateClaimV3Params,
) -> ProgramResult {
    if accounts.len() != 6
        || !accounts[0].is_signer
        || accounts[1].is_signer
        || accounts[1].is_writable
        || accounts[2].is_signer
        || accounts[2].is_writable
        || accounts[3].is_signer
        || accounts[3].is_writable
        || accounts[4].is_signer
        || accounts[4].is_writable
        || accounts[5].is_signer
        || !accounts[5].is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let claimant_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let source_info = &accounts[3];
    let active_manifest_info = &accounts[4];
    let claim_info = &accounts[5];
    if crate::bytes32_is_zero(&params.claim_id)
        || params.prior_state == 0
        || params.new_state == 0
        || params.source_time == 0
        || crate::bytes32_is_zero(&params.evidence_hash)
        || params.archive_url.is_empty()
        || crate::bytes32_is_zero(&params.secret_salt)
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }

    let (market, month) = load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_game_window(&market, &month)?;
    ensure_oracle_opening_resolution_complete(&month)?;
    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let archive_url_hash = validate_oracle_update_evidence(
        &market,
        &month,
        month_info.key,
        source_info.key,
        &source,
        params.new_state,
        params.source_time,
        &params.evidence_hash,
        &params.archive_url,
    )?;
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
    let slot = Clock::get()?.slot;
    validate_oracle_update_claim_v2_reveal(
        program_id,
        month_info.key,
        source_info.key,
        claimant_info.key,
        claim_info.key,
        &source,
        &claim,
        &params,
        slot,
    )?;
    claim.claim.prior_state = params.prior_state;
    claim.claim.new_state = params.new_state;
    claim.claim.evidence_hash = params.evidence_hash;
    claim.claim.source_time = params.source_time;
    claim.claim.archive_url_hash = archive_url_hash;
    claim.claim.status = OracleClaimStatus::Revealed;
    claim.revealed_slot = slot;
    let freshness_start = oracle_settlement_window_start(
        market.instrument.expiry_ts,
        crate::constants::ORACLE_SETTLEMENT_FRESHNESS_BUSINESS_DAYS,
    )?;
    claim.freshness_reward_multiplier =
        if (freshness_start..=market.instrument.expiry_ts).contains(&params.source_time) {
            crate::constants::ORACLE_FRESH_UPDATE_REWARD_MULTIPLIER
        } else {
            1
        };
    store_state(claim_info, &claim)
}

pub(in crate::processor) fn ensure_current_cash_update_resolution_window(
    market: &Market,
    month: &OracleMonthState,
) -> ProgramResult {
    if month.phase != OraclePhase::Game {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    rulebook_schedule_boundaries(month)?;
    let now = current_unix_timestamp()?;
    let resolution_end = market
        .instrument
        .expiry_ts
        .checked_add(ORACLE_SETTLEMENT_GRACE_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if month.listing_ts >= market.instrument.expiry_ts
        || now < month.listing_ts
        || now >= resolution_end
    {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    Ok(())
}

pub(in crate::processor) fn ensure_current_cash_update_emergency_window_open(
    market: &Market,
) -> ProgramResult {
    let cleanup_boundary = market
        .instrument
        .expiry_ts
        .checked_add(ORACLE_SETTLEMENT_GRACE_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if current_unix_timestamp()? >= cleanup_boundary {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    Ok(())
}

pub(in crate::processor) fn ensure_oracle_update_cleanup_ready_at(
    market: &Market,
    source: &OracleSourceState,
    claim: &OracleUpdateClaimV2,
    guard: Option<&OracleUpdateChallengeGuard>,
    now: u64,
) -> ProgramResult {
    let cleanup_boundary = market
        .instrument
        .expiry_ts
        .checked_add(ORACLE_SETTLEMENT_GRACE_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let source_is_stale = source.status != OracleSourceStatus::Active
        || !source.opening_submitted
        || claim.claim.prior_state != source.current_state;
    let guarded_step_is_stale = guard.is_some_and(|guard| {
        guard.resolution_step != 0 && guard.resolution_step <= source.last_finalized_step
    });
    if now < cleanup_boundary && !source_is_stale && !guarded_step_is_stale {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    Ok(())
}

pub(in crate::processor) fn load_exact_current_oracle_update_challenge(
    program_id: &Pubkey,
    month: &Pubkey,
    challenge_info: &AccountInfo,
) -> Result<OracleUpdateChallenge, ProgramError> {
    load_valid_oracle_update_challenge(program_id, month, challenge_info)
}
