use super::*;

pub(super) fn oracle_sku_merkle_proof_depth(
    required_sku_count: u16,
) -> Result<usize, ProgramError> {
    if required_sku_count == 0 || required_sku_count > MAX_ORACLE_REQUIRED_SKUS {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    let mut width = 1u16;
    let mut depth = 0usize;
    while width < required_sku_count {
        width = width.checked_mul(2).ok_or(VaultError::ArithmeticOverflow)?;
        depth = depth.checked_add(1).ok_or(VaultError::ArithmeticOverflow)?;
    }
    if depth > MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    Ok(depth)
}

pub(super) fn verify_oracle_sku_membership(
    coverage: &OracleSkuCoverageManifest,
    sku_id: &[u8; 32],
    sku_index: u16,
    proof: &[[u8; 32]],
) -> ProgramResult {
    if crate::bytes32_is_zero(&*sku_id)
        || sku_index >= coverage.required_sku_count
        || proof.len() != oracle_sku_merkle_proof_depth(coverage.required_sku_count)?
    {
        return Err(VaultError::InvalidOracleSkuMembershipProof.into());
    }
    let index_bytes = sku_index.to_le_bytes();
    let mut current = hashv(&[ORACLE_SKU_LEAF_HASH_DOMAIN, &index_bytes, sku_id]).to_bytes();
    for (level, sibling) in proof.iter().enumerate() {
        current = if ((usize::from(sku_index) >> level) & 1) == 0 {
            hashv(&[ORACLE_SKU_NODE_HASH_DOMAIN, &current, sibling]).to_bytes()
        } else {
            hashv(&[ORACLE_SKU_NODE_HASH_DOMAIN, sibling, &current]).to_bytes()
        };
    }
    if current != coverage.required_sku_root {
        return Err(VaultError::InvalidOracleSkuMembershipProof.into());
    }
    Ok(())
}

pub(super) fn ensure_oracle_coverage_source_submission_window(
    market: &Market,
    month: &OracleMonthState,
    coverage: &OracleSkuCoverageManifest,
) -> ProgramResult {
    let expected_planned_listing = coverage
        .planned_scramble_start_ts
        .checked_add(ORACLE_PRE_LISTING_WINDOW_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if coverage.planned_scramble_start_ts == 0
        || coverage.planned_listing_ts == 0
        || coverage.planned_listing_ts != expected_planned_listing
    {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    match month.phase {
        OraclePhase::SourceSubmission => {
            if coverage.coverage_finalized {
                return Err(VaultError::OracleSkuCoverageAlreadyFinalized.into());
            }
            let now = current_unix_timestamp()?;
            let preserved_review_end = now
                .checked_add(
                    ORACLE_KILL_WINDOW_SECONDS
                        + ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
                        + ORACLE_OPENING_WINDOW_SECONDS,
                )
                .ok_or(VaultError::ArithmeticOverflow)?;
            if now < coverage.planned_scramble_start_ts
                || preserved_review_end >= market.instrument.expiry_ts
            {
                return Err(VaultError::OracleSkuCoverageExtensionTooLate.into());
            }
            Ok(())
        }
        OraclePhase::Scramble => {
            if !coverage.coverage_finalized {
                return Err(VaultError::OracleSkuCoverageIncomplete.into());
            }
            ensure_oracle_placement_window(month)
        }
        _ => Err(VaultError::InvalidOraclePhase.into()),
    }
}

pub(super) fn finalized_oracle_sku_coverage_schedule(
    coverage: &OracleSkuCoverageManifest,
    market_expiry_ts: u64,
    now: u64,
) -> Result<(u64, u64), ProgramError> {
    let planned_placement_end = coverage
        .planned_scramble_start_ts
        .checked_add(ORACLE_PLACEMENT_WINDOW_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if coverage.planned_listing_ts
        != coverage
            .planned_scramble_start_ts
            .checked_add(ORACLE_PRE_LISTING_WINDOW_SECONDS)
            .ok_or(VaultError::ArithmeticOverflow)?
    {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    if now < coverage.planned_scramble_start_ts {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    if now <= planned_placement_end {
        return Ok((
            coverage.planned_scramble_start_ts,
            coverage.planned_listing_ts,
        ));
    }
    let preserved_post_submission_seconds = ORACLE_KILL_WINDOW_SECONDS
        .checked_add(ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS)
        .and_then(|value| value.checked_add(ORACLE_OPENING_WINDOW_SECONDS))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let shifted_scramble_start = now
        .checked_sub(ORACLE_PLACEMENT_WINDOW_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let shifted_listing = now
        .checked_add(preserved_post_submission_seconds)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if shifted_listing >= market_expiry_ts {
        return Err(VaultError::OracleSkuCoverageExtensionTooLate.into());
    }
    Ok((shifted_scramble_start, shifted_listing))
}

#[inline(never)]
pub(super) fn process_finalize_oracle_sku_coverage(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 4 || !accounts[0].is_signer {
        return Err(VaultError::InvalidAccountList.into());
    }
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let coverage_info = &accounts[3];
    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let mut coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    if month.phase != OraclePhase::SourceSubmission
        || coverage.coverage_finalized
        || coverage.covered_sku_count != coverage.required_sku_count
        || month.pending_resolution_count != 0
        || month.weight_scheme_version != 0
    {
        return Err(if coverage.coverage_finalized {
            VaultError::OracleSkuCoverageAlreadyFinalized.into()
        } else {
            VaultError::OracleSkuCoverageIncomplete.into()
        });
    }
    let (slot, now) = current_slot_and_unix_timestamp()?;
    let (final_scramble_start, final_listing) =
        finalized_oracle_sku_coverage_schedule(&coverage, market.instrument.expiry_ts, now)?;
    if final_scramble_start == coverage.planned_scramble_start_ts
        && (month.scramble_start_ts != coverage.planned_scramble_start_ts
            || month.listing_ts != coverage.planned_listing_ts)
    {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    month.scramble_start_ts = final_scramble_start;
    month.listing_ts = final_listing;
    rulebook_schedule_boundaries(&month)?;
    month.phase = OraclePhase::Scramble;
    month.last_updated_slot = slot;
    coverage.coverage_finalized = true;
    coverage.coverage_complete_ts = now;
    coverage.last_updated_slot = slot;
    store_state(coverage_info, &coverage)?;
    store_oracle_month_state(month_info, &month)
}

#[inline(never)]
pub(super) fn process_reopen_oracle_sku_coverage(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 4 || !accounts[0].is_signer {
        return Err(VaultError::InvalidAccountList.into());
    }
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let coverage_info = &accounts[3];
    let (_market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let mut coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    let (slot, now) = current_slot_and_unix_timestamp()?;
    reopen_oracle_sku_coverage_state(&mut month, &mut coverage, now, slot)?;
    store_state(coverage_info, &coverage)?;
    store_oracle_month_state(month_info, &month)
}

pub(super) fn reopen_oracle_sku_coverage_state(
    month: &mut OracleMonthState,
    coverage: &mut OracleSkuCoverageManifest,
    now: u64,
    slot: u64,
) -> ProgramResult {
    let (placement_end, _, scramble_end) = rulebook_schedule_boundaries(month)?;
    if month.phase != OraclePhase::Scramble
        || month.pending_resolution_count != 0
        || month.weight_scheme_version != 0
    {
        return Err(VaultError::OracleSkuCoverageIncomplete.into());
    }
    let incomplete_submission_stalled = !coverage.coverage_finalized
        && coverage.covered_sku_count < coverage.required_sku_count
        && now >= placement_end;
    let finalized_scramble_stalled = coverage.coverage_finalized
        && coverage.covered_sku_count == coverage.required_sku_count
        && now >= scramble_end;
    if !incomplete_submission_stalled && !finalized_scramble_stalled {
        return Err(VaultError::OracleSkuCoverageIncomplete.into());
    }

    month.phase = OraclePhase::SourceSubmission;
    month.last_updated_slot = slot;
    coverage.coverage_finalized = false;
    coverage.coverage_complete_ts = 0;
    coverage.last_updated_slot = slot;
    Ok(())
}

pub(super) fn remove_supported_source_from_sku_coverage(
    coverage: &mut OracleSkuCoverageManifest,
    record: &mut OracleSkuCoverageRecord,
) -> ProgramResult {
    record.active_supported_source_count = record
        .active_supported_source_count
        .checked_sub(1)
        .ok_or(VaultError::InvalidOracleSkuCoverageRecord)?;
    if record.active_supported_source_count == 0 {
        coverage.covered_sku_count = coverage
            .covered_sku_count
            .checked_sub(1)
            .ok_or(VaultError::InvalidOracleSkuCoverageManifest)?;
        coverage.coverage_finalized = false;
        coverage.coverage_complete_ts = 0;
    }
    Ok(())
}

pub(super) fn oracle_source_submission_latest_safe_ts(expiry_ts: u64) -> Result<u64, ProgramError> {
    let preserved_post_submission_seconds = ORACLE_KILL_WINDOW_SECONDS
        .checked_add(ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS)
        .and_then(|value| value.checked_add(ORACLE_OPENING_WINDOW_SECONDS))
        .ok_or(VaultError::ArithmeticOverflow)?;
    expiry_ts
        .checked_sub(preserved_post_submission_seconds)
        .ok_or_else(|| VaultError::InvalidOracleState.into())
}

pub(super) fn oracle_source_submission_is_unlistable(
    expiry_ts: u64,
    now: u64,
) -> Result<bool, ProgramError> {
    Ok(now >= oracle_source_submission_latest_safe_ts(expiry_ts)?)
}

pub(super) fn oracle_stale_source_challenge_cleanup_ready(
    month: &OracleMonthState,
    now: u64,
) -> Result<bool, ProgramError> {
    Ok(now >= rulebook_schedule_boundaries(month)?.2)
}

pub(super) fn oracle_unlistable_source_cleanup_ready(
    month: &OracleMonthState,
    expiry_ts: u64,
    now: u64,
) -> Result<bool, ProgramError> {
    if !oracle_source_submission_is_unlistable(expiry_ts, now)? {
        return Ok(false);
    }
    match month.phase {
        OraclePhase::SourceSubmission => Ok(true),
        OraclePhase::Scramble => oracle_stale_source_challenge_cleanup_ready(month, now),
        _ => Ok(false),
    }
}

pub(super) fn cancel_stale_oracle_source_challenge_state(
    month: &mut OracleMonthState,
    coverage: &mut OracleSkuCoverageManifest,
    challenge_key: &Pubkey,
    challenge: &mut OracleSourceChallenge,
    source_guard: &mut OracleSourceChallengeGuard,
    comparison_guard: Option<&mut OracleSourceChallengeGuard>,
    staking_pool: Option<&mut OracleStakingPool>,
    now: u64,
    slot: u64,
) -> ProgramResult {
    let valid_fail_closed_phase = match month.phase {
        // A prior merited resolution can already have removed the last supported source for one
        // SKU and cleared this latch while other challenges remain pending.
        OraclePhase::Scramble => true,
        OraclePhase::SourceSubmission => !coverage.coverage_finalized,
        _ => false,
    };
    if !valid_fail_closed_phase
        || month.weight_scheme_version != 0
        || month.pending_resolution_count == 0
        || !oracle_stale_source_challenge_cleanup_ready(month, now)?
    {
        return Err(VaultError::InvalidOraclePhase.into());
    }

    let has_comparison = !crate::pubkey_is_default(&challenge.comparison_source)
        && !crate::bytes32_is_zero(&challenge.comparison_source_id);
    if !matches!(
        challenge.status,
        OracleChallengeStatus::Open
            | OracleChallengeStatus::RuleReview
            | OracleChallengeStatus::RuleReviewUnresolved
    ) || challenge.bond == 0
        || challenge.bond != challenge.required_bond
        || challenge.escrow_disposition != OracleEscrowDisposition::Unsettled
        || crate::pubkey_is_default(&challenge.comparison_source)
            != (crate::bytes32_is_zero(&challenge.comparison_source_id))
        || (challenge.reason == ORACLE_SOURCE_REASON_NON_INDEPENDENT) != has_comparison
        || comparison_guard.is_some() != has_comparison
        || source_guard.active_challenge != *challenge_key
        || source_guard.active_challenge_id != challenge.challenge_id
        || !crate::pubkey_is_default(&source_guard.active_dispute)
        || staking_pool.is_some()
            != (challenge.status == OracleChallengeStatus::RuleReviewUnresolved)
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    if let Some(guard) = comparison_guard.as_ref() {
        if guard.active_challenge != *challenge_key
            || guard.active_challenge_id != challenge.challenge_id
            || !crate::pubkey_is_default(&guard.active_dispute)
        {
            return Err(VaultError::InvalidOracleChallengeAccount.into());
        }
    }
    if let Some(pool) = staking_pool.as_ref() {
        if challenge.emergency_snapshot_version != 2
            || challenge.rule_review_slot == 0
            || challenge.emergency_snapshot_total_major_tokens == 0
            || pool.governance_lock_count == 0
            || pool.samba_supply != challenge.emergency_snapshot_total_major_tokens
        {
            return Err(VaultError::InvalidOracleStakingPool.into());
        }
    }

    release_source_challenge_guard(
        source_guard,
        challenge_key,
        &challenge.challenge_id,
        &Pubkey::default(),
        slot,
    )?;
    if let Some(guard) = comparison_guard {
        release_source_challenge_guard(
            guard,
            challenge_key,
            &challenge.challenge_id,
            &Pubkey::default(),
            slot,
        )?;
    }
    if let Some(pool) = staking_pool {
        pool.governance_lock_count = pool
            .governance_lock_count
            .checked_sub(1)
            .ok_or(VaultError::InvalidOracleStakingPool)?;
        pool.last_updated_slot = slot;
    }
    decrement_pending_oracle_resolution(month)?;
    challenge.status = OracleChallengeStatus::Cancelled;
    month.phase = OraclePhase::SourceSubmission;
    month.last_updated_slot = slot;
    coverage.coverage_finalized = false;
    coverage.coverage_complete_ts = 0;
    coverage.last_updated_slot = slot;
    Ok(())
}

pub(super) fn expire_unlistable_oracle_source_state(
    month: &mut OracleMonthState,
    coverage: &mut OracleSkuCoverageManifest,
    source: &mut OracleSourceState,
    coverage_record: Option<&mut OracleSkuCoverageRecord>,
    expiry_ts: u64,
    now: u64,
    slot: u64,
) -> ProgramResult {
    if !matches!(
        month.phase,
        OraclePhase::SourceSubmission | OraclePhase::Scramble
    ) || month.pending_resolution_count != 0
        || month.weight_scheme_version != 0
        || source.status != OracleSourceStatus::Candidate
        || coverage_record.is_some() != (source.support_stake_total > 0)
    {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    if !oracle_unlistable_source_cleanup_ready(month, expiry_ts, now)? {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    if let Some(record) = coverage_record {
        if record.active_supported_source_count == 0 {
            return Err(VaultError::InvalidOracleSkuCoverageRecord.into());
        }
        decrement_oracle_supported_candidate_count(month)?;
        remove_supported_source_from_sku_coverage(coverage, record)?;
        record.last_updated_slot = slot;
    }

    source.status = OracleSourceStatus::TimedOut;
    month.phase = OraclePhase::SourceSubmission;
    month.last_updated_slot = slot;
    coverage.coverage_finalized = false;
    coverage.coverage_complete_ts = 0;
    coverage.last_updated_slot = slot;
    Ok(())
}

#[inline(never)]
pub(super) fn process_expire_unlistable_oracle_source_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() < 5 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let coverage_info = &accounts[3];
    let source_info = &accounts[4];
    if !cranker_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let mut coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    let mut source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let (slot, now) = current_slot_and_unix_timestamp()?;

    let record_context = if source.support_stake_total > 0 {
        if accounts.len() != 6 {
            return Err(VaultError::InvalidAccountList.into());
        }
        let record_info = &accounts[5];
        Some((
            load_valid_oracle_sku_coverage_record(
                program_id,
                month_info.key,
                &source.bucket_id,
                record_info,
            )?,
            record_info,
        ))
    } else if accounts.len() != 5 {
        return Err(VaultError::InvalidAccountList.into());
    } else {
        None
    };

    let mut record_context = record_context;
    expire_unlistable_oracle_source_state(
        &mut month,
        &mut coverage,
        &mut source,
        record_context.as_mut().map(|(record, _)| record),
        market.instrument.expiry_ts,
        now,
        slot,
    )?;
    if let Some((record, info)) = record_context.as_ref() {
        store_state(info, record)?;
    }
    store_state(source_info, &source)?;
    store_state(coverage_info, &coverage)?;
    store_oracle_month_state(month_info, &month)
}

#[inline(never)]
pub(super) fn process_cancel_stale_oracle_source_challenge_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() < 6 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let coverage_info = &accounts[3];
    let challenge_info = &accounts[4];
    let source_guard_info = &accounts[5];
    if !cranker_info.is_signer
        || !month_info.is_writable
        || !coverage_info.is_writable
        || !challenge_info.is_writable
        || !source_guard_info.is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }

    let (_market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let mut coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;

    let mut challenge =
        load_valid_oracle_source_challenge(program_id, month_info.key, challenge_info)?;
    let has_comparison = !crate::pubkey_is_default(&challenge.comparison_source)
        && !crate::bytes32_is_zero(&challenge.comparison_source_id);
    let comparison_guard_info = if has_comparison {
        let info = accounts.get(6).ok_or(VaultError::InvalidAccountList)?;
        if !info.is_writable || info.key == source_guard_info.key {
            return Err(VaultError::InvalidAccountList.into());
        }
        Some(info)
    } else {
        None
    };
    let staking_index = 6 + usize::from(has_comparison);
    let staking_pool_info = if challenge.status == OracleChallengeStatus::RuleReviewUnresolved {
        let info = accounts
            .get(staking_index)
            .ok_or(VaultError::InvalidAccountList)?;
        if !info.is_writable
            || info.key == source_guard_info.key
            || comparison_guard_info.is_some_and(|comparison| comparison.key == info.key)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
        Some(info)
    } else {
        None
    };
    let expected_len = staking_index + usize::from(staking_pool_info.is_some());
    if accounts.len() != expected_len {
        return Err(VaultError::InvalidAccountList.into());
    }

    let mut source_guard = load_canonical_source_challenge_guard(
        program_id,
        month_info.key,
        &challenge.source,
        &challenge.source_id,
        source_guard_info,
    )?;
    let mut comparison_guard = if let Some(info) = comparison_guard_info {
        let guard = load_canonical_source_challenge_guard(
            program_id,
            month_info.key,
            &challenge.comparison_source,
            &challenge.comparison_source_id,
            info,
        )?;
        Some((guard, info))
    } else {
        None
    };
    let mut staking_pool = if let Some(info) = staking_pool_info {
        Some(load_canonical_oracle_staking_pool(
            program_id,
            info,
            &derive_oracle_major_token_config_pda(program_id).0,
        )?)
    } else {
        None
    };

    let (slot, now) = current_slot_and_unix_timestamp()?;
    cancel_stale_oracle_source_challenge_state(
        &mut month,
        &mut coverage,
        challenge_info.key,
        &mut challenge,
        &mut source_guard,
        comparison_guard.as_mut().map(|(guard, _)| guard),
        staking_pool.as_mut(),
        now,
        slot,
    )?;
    store_state(source_guard_info, &source_guard)?;
    if let Some((guard, info)) = comparison_guard.as_ref() {
        store_state(info, guard)?;
    }
    if let (Some(pool), Some(info)) = (staking_pool.as_ref(), staking_pool_info) {
        store_state(info, pool)?;
    }
    store_state(challenge_info, &challenge)?;
    store_state(coverage_info, &coverage)?;
    store_oracle_month_state(month_info, &month)
}
