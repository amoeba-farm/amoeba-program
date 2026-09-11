use super::*;

#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn apply_cash_emergency_resolution(
    program_id: &Pubkey,
    remaining_accounts: &[AccountInfo],
    month: &mut OracleMonthState,
    dispute: &OracleEmergencyDisputeV3,
    target_info: &AccountInfo,
    resolved_choice: u8,
    coverage_context: Option<(&AccountInfo, &AccountInfo)>,
    merge_reward_context: Option<(&AccountInfo, &AccountInfo, &AccountInfo)>,
) -> ProgramResult {
    validate_oracle_emergency_choice(dispute.kind, resolved_choice)?;
    match dispute.kind {
        OracleEmergencyDisputeKind::Source => {
            let source_info = &remaining_accounts[0];
            let mut challenge =
                load_valid_oracle_source_challenge(program_id, &dispute.month, target_info)?;
            let mut source = load_valid_oracle_source(program_id, &dispute.month, source_info)?;
            let (mut coverage, mut coverage_record) =
                if let Some((coverage_info, coverage_record_info)) = coverage_context {
                    let coverage = load_valid_oracle_sku_coverage_manifest(
                        program_id,
                        &dispute.month,
                        coverage_info,
                    )?;
                    let record = load_valid_oracle_sku_coverage_record(
                        program_id,
                        &dispute.month,
                        &source.bucket_id,
                        coverage_record_info,
                    )?;
                    if source.status == OracleSourceStatus::Candidate
                        && source.support_stake_total > 0
                        && record.active_supported_source_count == 0
                    {
                        return Err(VaultError::InvalidOracleSkuCoverageRecord.into());
                    }
                    (Some(coverage), Some(record))
                } else {
                    (None, None)
                };
            if source.status != OracleSourceStatus::Candidate {
                challenge.status = OracleChallengeStatus::Rejected;
            } else {
                match resolved_choice {
                    0 => challenge.status = OracleChallengeStatus::Rejected,
                    1 => {
                        remove_emergency_supported_candidate_coverage(
                            month,
                            coverage.as_mut(),
                            coverage_record.as_mut(),
                            source.support_stake_total > 0,
                        )?;
                        source.status = OracleSourceStatus::Rejected;
                        challenge.status = OracleChallengeStatus::Accepted;
                    }
                    2 => {
                        if challenge.reason != ORACLE_SOURCE_REASON_NON_INDEPENDENT
                            || crate::pubkey_is_default(&challenge.comparison_source)
                        {
                            return Err(VaultError::InvalidOracleChallengeAccount.into());
                        }
                        let comparison_info = &remaining_accounts[2];
                        if *comparison_info.key != challenge.comparison_source {
                            return Err(VaultError::InvalidOracleSourceAccount.into());
                        }
                        let mut comparison =
                            load_valid_oracle_source(program_id, &dispute.month, comparison_info)?;
                        if comparison.source_id != challenge.comparison_source_id {
                            return Err(VaultError::InvalidOracleSourceAccount.into());
                        }
                        if comparison.status != OracleSourceStatus::Candidate {
                            challenge.status = OracleChallengeStatus::Rejected;
                        } else {
                            let both_supported = source.support_stake_total > 0
                                && comparison.support_stake_total > 0;
                            let (sku_info, source_reward_info, comparison_reward_info) =
                                merge_reward_context.ok_or(VaultError::InvalidAccountList)?;
                            oracle_usdc::merge_oracle_usdc_source_rewards(
                                program_id,
                                &dispute.month,
                                sku_info,
                                source_info,
                                &source,
                                source_reward_info,
                                comparison_info,
                                &comparison,
                                comparison_reward_info,
                            )?;
                            apply_oracle_source_merge(&mut source, &mut comparison)?;
                            remove_emergency_supported_candidate_coverage(
                                month,
                                coverage.as_mut(),
                                coverage_record.as_mut(),
                                both_supported,
                            )?;
                            challenge.status = OracleChallengeStatus::Accepted;
                            store_state(comparison_info, &comparison)?;
                        }
                    }
                    _ => return Err(VaultError::InvalidOracleEmergencyDispute.into()),
                }
            }
            decrement_pending_oracle_resolution(month)?;
            if let Some(coverage) = coverage.as_mut() {
                reopen_coverage_after_delayed_emergency_resolution(
                    month,
                    coverage,
                    current_unix_timestamp()?,
                )?;
            }
            store_state(source_info, &source)?;
            store_state(target_info, &challenge)?;
            if let (Some((coverage_info, record_info)), Some(coverage), Some(record)) = (
                coverage_context,
                coverage.as_mut(),
                coverage_record.as_mut(),
            ) {
                let slot = Clock::get()?.slot;
                coverage.last_updated_slot = slot;
                record.last_updated_slot = slot;
                store_state(coverage_info, coverage)?;
                store_state(record_info, record)?;
            }
            Ok(())
        }
        OracleEmergencyDisputeKind::Update => {
            let claim_info = &remaining_accounts[0];
            let source_info = &remaining_accounts[1];
            let observations_info = &remaining_accounts[2];
            let active_manifest_info = &remaining_accounts[3];
            let challenge_guard_info = &remaining_accounts[4];
            let bucket_info = &remaining_accounts[5];
            let mut challenge =
                load_valid_oracle_update_challenge(program_id, &dispute.month, target_info)?;
            let mut claim = load_valid_oracle_update_claim_v2_from_account(
                program_id,
                &dispute.month,
                source_info.key,
                claim_info,
            )?;
            let mut source = load_valid_oracle_source(program_id, &dispute.month, source_info)?;
            let mut observations = load_valid_oracle_source_observations(
                program_id,
                &dispute.month,
                source_info.key,
                observations_info,
            )?;
            validate_oracle_observation_shape(&source, &observations)?;
            let challenge_guard = load_canonical_update_challenge_guard(
                program_id,
                &dispute.month,
                claim_info.key,
                &claim.claim.claim_id,
                challenge_guard_info,
            )?;
            if challenge_guard.challenge != *target_info.key
                || challenge_guard.active_dispute
                    != derive_oracle_emergency_dispute_v3_pda(
                        program_id,
                        &dispute.month,
                        &dispute.dispute_id,
                    )
                    .0
                || challenge_guard.resolution_step == 0
                || challenge_guard.resolution_step <= source.last_finalized_step
            {
                return Err(VaultError::InvalidOracleUpdateAccount.into());
            }
            validate_active_oracle_source(
                program_id,
                month,
                &dispute.month,
                active_manifest_info,
                &source,
            )?;
            let mut bucket =
                load_valid_oracle_bucket_median(program_id, &dispute.month, bucket_info)?;
            if bucket.bucket_id != source.bucket_id
                || !matches!(
                    bucket.status,
                    OracleBucketMedianStatus::Live | OracleBucketMedianStatus::Dirty
                )
            {
                return Err(VaultError::InvalidOracleMedian.into());
            }
            match resolved_choice {
                0 => {
                    append_oracle_source_observation(
                        &mut source,
                        &mut observations,
                        claim.claim.new_state,
                        claim.claim.source_time,
                        &claim.claim.evidence_hash,
                        &claim.claim.archive_url_hash,
                    )?;
                    source.current_state = claim.claim.new_state;
                    source.last_finalized_step = challenge_guard.resolution_step;
                    claim.claim.status = OracleClaimStatus::Finalized;
                    challenge.status = OracleChallengeStatus::Rejected;
                    month.accepted_cash_update_count = month
                        .accepted_cash_update_count
                        .checked_add(1)
                        .ok_or(VaultError::ArithmeticOverflow)?;
                }
                1 => {
                    append_oracle_source_observation(
                        &mut source,
                        &mut observations,
                        challenge.alternative_state,
                        challenge.alternative_source_time,
                        &challenge.evidence_hash,
                        &challenge.archive_url_hash,
                    )?;
                    source.current_state = challenge.alternative_state;
                    source.last_finalized_step = challenge_guard.resolution_step;
                    claim.claim.status = OracleClaimStatus::Rejected;
                    challenge.status = OracleChallengeStatus::Accepted;
                }
                2 => {
                    claim.claim.status = OracleClaimStatus::Rejected;
                    challenge.status = OracleChallengeStatus::Rejected;
                }
                _ => return Err(VaultError::InvalidOracleEmergencyDispute.into()),
            }
            claim.samba_checkpoint_active = false;
            if matches!(resolved_choice, 0 | 1) {
                bucket.status = OracleBucketMedianStatus::Dirty;
                bucket.recompute_processed_source_count = 0;
                store_state(bucket_info, &bucket)?;
            }
            decrement_pending_oracle_resolution(month)?;
            let (c_settle_bps, settlement_status) =
                oracle_settlement_status(month.c_raw_bps, month.g_camo_bps, month.g_thin_bps);
            month.c_settle_bps = c_settle_bps;
            month.settlement_status = settlement_status;
            store_state(source_info, &source)?;
            store_state(observations_info, &observations)?;
            store_state(claim_info, &claim)?;
            store_state(target_info, &challenge)
        }
        OracleEmergencyDisputeKind::Opening => {
            let claim_info = &remaining_accounts[0];
            let source_info = &remaining_accounts[1];
            let mut challenge =
                load_valid_oracle_opening_challenge(program_id, &dispute.month, target_info)?;
            let mut source = load_valid_oracle_source(program_id, &dispute.month, source_info)?;
            let mut claim = load_valid_oracle_opening_claim(
                program_id,
                &dispute.month,
                source_info.key,
                claim_info,
                &source,
            )?;
            match resolved_choice {
                0 => {
                    challenge.status = OracleChallengeStatus::Rejected;
                    decrement_pending_oracle_resolution(month)?;
                }
                1 => {
                    challenge.status = OracleChallengeStatus::Accepted;
                    decrement_pending_oracle_resolution(month)?;
                }
                2 => {
                    challenge.status = OracleChallengeStatus::Accepted;
                    claim.status = OracleOpeningClaimStatus::Rejected;
                    source.status = OracleSourceStatus::Inactive;
                    month.opening_resolved_source_count = month
                        .opening_resolved_source_count
                        .checked_add(1)
                        .ok_or(VaultError::ArithmeticOverflow)?;
                    decrement_pending_oracle_resolution(month)?;
                    decrement_pending_oracle_resolution(month)?;
                }
                _ => return Err(VaultError::InvalidOracleEmergencyDispute.into()),
            }
            store_state(source_info, &source)?;
            store_state(claim_info, &claim)?;
            store_state(target_info, &challenge)
        }
        OracleEmergencyDisputeKind::BucketMedian => {
            if !remaining_accounts.is_empty() {
                return Err(VaultError::InvalidAccountList.into());
            }
            let mut bucket: OracleBucketMedianState = load_exact_zero_padded_state(
                target_info,
                program_id,
                OracleBucketMedianState::LEN,
                VaultError::InvalidOracleMedian,
            )?;
            if bucket.month != dispute.month
                || bucket.bucket_id != dispute.target_id
                || bucket.status != OracleBucketMedianStatus::EmergencyRequired
            {
                return Err(VaultError::InvalidOracleMedian.into());
            }
            match resolved_choice {
                0 => {
                    let fallback_delta = bucket.bucket_delta_bps;
                    update_month_bucket_contribution(month, &mut bucket, fallback_delta)?;
                    bucket.status = OracleBucketMedianStatus::EmergencyDefaulted;
                }
                1 => bucket.status = OracleBucketMedianStatus::EmergencyRejected,
                _ => return Err(VaultError::InvalidOracleEmergencyDispute.into()),
            }
            store_state(target_info, &bucket)
        }
    }
}

#[inline(never)]
pub(in crate::processor) fn process_resolve_oracle_emergency_dispute_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    process_resolve_oracle_emergency_dispute_v3_inner(program_id, accounts, false)
}

#[inline(never)]
pub(in crate::processor) fn process_resolve_oracle_emergency_dispute_v4(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    process_resolve_oracle_emergency_dispute_v3_inner(program_id, accounts, true)
}

pub(in crate::processor) fn process_resolve_oracle_emergency_dispute_v3_inner(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    coverage_aware: bool,
) -> ProgramResult {
    if accounts.len() < 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let dispute_info = &accounts[3];
    let target_info = &accounts[4];
    let pot_info = &accounts[5];
    let pot_token_info = &accounts[6];
    let staking_pool_info = &accounts[7];
    if !cranker_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let remaining_accounts = &accounts[8..];
    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let mut dispute = load_dispute_v3(program_id, dispute_info)?;
    let (remaining_accounts, checkpoint_accounts) =
        if dispute.kind == OracleEmergencyDisputeKind::Update {
            if remaining_accounts.len() != 9 {
                return Err(VaultError::InvalidAccountList.into());
            }
            remaining_accounts.split_at(6)
        } else {
            (
                remaining_accounts,
                &remaining_accounts[remaining_accounts.len()..],
            )
        };

    if dispute.kind == OracleEmergencyDisputeKind::Update {
        oracle_usdc::ensure_current_cash_update_emergency_window_open(&market)?;
    }
    ensure_emergency_resolver_coverage_lane(&month, dispute.kind, coverage_aware)?;
    let (resolution_accounts, coverage_context, merge_reward_context) = if coverage_aware {
        if remaining_accounts.len() < 4 {
            return Err(VaultError::InvalidAccountList.into());
        }
        let coverage_index = remaining_accounts.len() - 2;
        let coverage_info = &remaining_accounts[coverage_index];
        let coverage_record_info = &remaining_accounts[coverage_index + 1];
        if !coverage_info.is_writable || !coverage_record_info.is_writable {
            return Err(VaultError::InvalidAccountList.into());
        }
        let pre_coverage = &remaining_accounts[..coverage_index];
        if dispute.kind == OracleEmergencyDisputeKind::Source {
            let challenge =
                load_valid_oracle_source_challenge(program_id, month_info.key, target_info)?;
            let has_comparison = !crate::pubkey_is_default(&challenge.comparison_source)
                && !crate::bytes32_is_zero(&challenge.comparison_source_id);
            let resolution_len = if has_comparison { 4 } else { 2 };
            let merge_len = if has_comparison { 3 } else { 0 };
            if pre_coverage.len() != resolution_len + merge_len {
                return Err(VaultError::InvalidAccountList.into());
            }
            (
                &pre_coverage[..resolution_len],
                Some((coverage_info, coverage_record_info)),
                if has_comparison {
                    Some((
                        &pre_coverage[resolution_len],
                        &pre_coverage[resolution_len + 1],
                        &pre_coverage[resolution_len + 2],
                    ))
                } else {
                    None
                },
            )
        } else {
            (
                pre_coverage,
                Some((coverage_info, coverage_record_info)),
                None,
            )
        }
    } else {
        (remaining_accounts, None, None)
    };
    let mut pot = load_pot(program_id, dispute_info, &dispute, pot_info)?;
    let pot_token = validate_pot_token_account(&pot, pot_info, pot_token_info)?;
    let slot = Clock::get()?.slot;
    if dispute.month != *month_info.key
        || dispute.target_account != *target_info.key
        || dispute.status != OracleChallengeStatus::Open
        || pot.payout_mode != OracleSambaEmergencyPayoutMode::Open
        || dispute.committed_power != pot.total_committed
        || dispute.committed_vote_count != pot.committed_vote_count
        || pot.remaining_liability != pot.total_committed
        || slot <= dispute.reveal_deadline_slot
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let decision = derive_decision_v3(&dispute)?;
    let packet = validate_cash_emergency_target(
        program_id,
        month_info.key,
        &month,
        dispute.kind,
        &dispute.target_id,
        target_info,
        resolution_accounts,
        Some(decision.resolved_choice),
        Some(dispute_info.key),
    )?;
    if packet.snapshot_slot != dispute.snapshot_slot
        || packet.snapshot_total_major_tokens != dispute.snapshot_total_major_tokens
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let mut staking_pool = load_canonical_oracle_staking_pool(
        program_id,
        staking_pool_info,
        &derive_oracle_major_token_config_pda(program_id).0,
    )?;
    if staking_pool.governance_lock_count == 0
        || staking_pool.samba_supply != dispute.snapshot_total_major_tokens
    {
        return Err(VaultError::InvalidOracleStakingPool.into());
    }
    let source_before =
        if dispute.kind == OracleEmergencyDisputeKind::Update && decision.resolved_choice < 2 {
            Some(load_valid_oracle_source(
                program_id,
                month_info.key,
                &resolution_accounts[1],
            )?)
        } else {
            None
        };
    apply_cash_emergency_resolution(
        program_id,
        resolution_accounts,
        &mut month,
        &dispute,
        target_info,
        decision.resolved_choice,
        coverage_context,
        merge_reward_context,
    )?;
    if let Some(before) = source_before {
        let source_info = &resolution_accounts[1];
        let after = load_valid_oracle_source(program_id, month_info.key, source_info)?;
        let observations = load_valid_oracle_source_observations(
            program_id,
            month_info.key,
            source_info.key,
            &resolution_accounts[2],
        )?;
        let event = if decision.resolved_choice == 0 {
            let claim = load_valid_oracle_update_claim_v2_from_account(
                program_id,
                month_info.key,
                source_info.key,
                &resolution_accounts[0],
            )?;
            crate::processor::oracle_carry::AcceptedEvent {
                event: *dispute_info.key,
                value: claim.claim.new_state,
                observed_at: claim.claim.source_time,
                evidence_hash: claim.claim.evidence_hash,
                archive_hash: claim.claim.archive_url_hash,
                contributor: claim.claim.claimant,
            }
        } else {
            let challenge =
                load_valid_oracle_update_challenge(program_id, month_info.key, target_info)?;
            crate::processor::oracle_carry::AcceptedEvent {
                event: *dispute_info.key,
                value: challenge.alternative_state,
                observed_at: challenge.alternative_source_time,
                evidence_hash: challenge.evidence_hash,
                archive_hash: challenge.archive_url_hash,
                contributor: challenge.challenger,
            }
        };
        crate::processor::oracle_carry::record_fresh_accept(
            program_id,
            cranker_info,
            source_info.key,
            &before,
            &after,
            &observations,
            event,
            checkpoint_accounts,
        )?;
    }
    if dispute.kind == OracleEmergencyDisputeKind::Source {
        set_cash_emergency_guard_dispute_binding(
            program_id,
            dispute.kind,
            month_info.key,
            target_info,
            resolution_accounts,
            dispute_info.key,
            &Pubkey::default(),
            slot,
        )?;
    }
    staking_pool.governance_lock_count = staking_pool
        .governance_lock_count
        .checked_sub(1)
        .ok_or(VaultError::InvalidOracleStakingPool)?;
    staking_pool.last_updated_slot = slot;
    dispute.winning_choice = decision.winning_choice;
    dispute.winning_power = if decision.redistribute {
        decision.winning_power
    } else {
        0
    };
    dispute.winning_vote_count = decision.winning_vote_count;
    dispute.resolved_choice = decision.resolved_choice;
    dispute.resolved_slot = slot;
    dispute.status = OracleChallengeStatus::Accepted;
    pot.payout_mode = if decision.redistribute {
        OracleSambaEmergencyPayoutMode::Redistribute
    } else {
        OracleSambaEmergencyPayoutMode::RefundAll
    };
    pot.winning_choice = decision.winning_choice;
    pot.winning_power = dispute.winning_power;
    pot.winning_vote_count = dispute.winning_vote_count;
    pot.resolved_slot = slot;
    pot.last_updated_slot = slot;
    if !decision.redistribute {
        pot.registration_finalized = true;
    }
    // Donations do not enlarge P, and a balance larger than the tracked liability is valid.
    if pot_token.amount < pot.remaining_liability {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    month.last_updated_slot = slot;
    store_state(staking_pool_info, &staking_pool)?;
    store_oracle_month_state(month_info, &month)?;
    store_state(pot_info, &pot)?;
    store_state(dispute_info, &dispute)
}

#[inline(never)]
pub(in crate::processor) fn process_abort_stale_oracle_update_emergency_dispute_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 11
        || !accounts[0].is_signer
        || !accounts[2].is_writable
        || !accounts[4].is_writable
        || !accounts[5].is_writable
        || !accounts[6].is_writable
        || !accounts[7].is_writable
        || !accounts[8].is_writable
        || !accounts[10].is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let source_info = &accounts[3];
    let claim_info = &accounts[4];
    let challenge_info = &accounts[5];
    let guard_info = &accounts[6];
    let dispute_info = &accounts[7];
    let pot_info = &accounts[8];
    let pot_token_info = &accounts[9];
    let staking_pool_info = &accounts[10];

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
    let mut challenge = oracle_usdc::load_exact_current_oracle_update_challenge(
        program_id,
        month_info.key,
        challenge_info,
    )?;
    let mut guard = load_canonical_update_challenge_guard(
        program_id,
        month_info.key,
        claim_info.key,
        &claim.claim.claim_id,
        guard_info,
    )?;
    let mut dispute = load_dispute_v3(program_id, dispute_info)?;
    let mut pot = load_pot(program_id, dispute_info, &dispute, pot_info)?;
    let pot_token = validate_pot_token_account(&pot, pot_info, pot_token_info)?;
    let mut staking_pool = load_canonical_oracle_staking_pool(
        program_id,
        staking_pool_info,
        &derive_oracle_major_token_config_pda(program_id).0,
    )?;

    let case_hash = oracle_emergency_case_hash(
        month_info.key,
        OracleEmergencyDisputeKind::Update,
        &challenge.challenge_id,
    );
    let choice_power_total = dispute.choice_power.iter().try_fold(0u64, |total, value| {
        total
            .checked_add(*value)
            .ok_or(VaultError::ArithmeticOverflow)
    })?;
    let choice_vote_count_total =
        dispute
            .choice_vote_count
            .iter()
            .try_fold(0u32, |total, value| {
                total
                    .checked_add(*value)
                    .ok_or(VaultError::ArithmeticOverflow)
            })?;
    if claim.claim.source_id != source.source_id
        || claim.claim.status != OracleClaimStatus::Revealed
        || claim.claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || claim.claim.stake == 0
        || claim.claim.prior_state == 0
        || claim.claim.new_state == 0
        || claim.claim.prior_state == claim.claim.new_state
        || crate::bytes32_is_zero(&claim.claim.evidence_hash)
        || claim.revealed_slot == 0
        || !claim.samba_checkpoint_active
        || month.pending_resolution_count == 0
        || challenge.claim != *claim_info.key
        || challenge.claim_id != claim.claim.claim_id
        || challenge.status != OracleChallengeStatus::RuleReviewUnresolved
        || challenge.bond == 0
        || challenge.bond != challenge.required_bond
        || crate::bytes32_is_zero(&challenge.evidence_hash)
        || challenge.escrow_disposition != OracleEscrowDisposition::Unsettled
        || challenge.rule_review_slot == 0
        || challenge.emergency_snapshot_total_major_tokens == 0
        || guard.challenge != *challenge_info.key
        || guard.challenge_id != challenge.challenge_id
        || guard.active_dispute != *dispute_info.key
        || guard.resolution_step == 0
        || dispute.month != *month_info.key
        || dispute.dispute_id != case_hash
        || dispute.kind != OracleEmergencyDisputeKind::Update
        || dispute.target_id != challenge.challenge_id
        || dispute.target_account != *challenge_info.key
        || crate::pubkey_is_default(&dispute.opened_by)
        || dispute.status != OracleChallengeStatus::Open
        || dispute.fallback_choice != 2
        || dispute.choice_count != 3
        || dispute.resolved_choice != 2
        || dispute.winning_choice != 2
        || dispute.winning_power != 0
        || dispute.winning_vote_count != 0
        || dispute.resolved_slot != 0
        || dispute.snapshot_slot != challenge.rule_review_slot
        || dispute.snapshot_total_major_tokens != challenge.emergency_snapshot_total_major_tokens
        || dispute.commit_deadline_slot == 0
        || dispute.reveal_deadline_slot <= dispute.commit_deadline_slot
        || dispute.revealed_power > dispute.committed_power
        || dispute.revealed_vote_count > dispute.committed_vote_count
        || choice_power_total != dispute.revealed_power
        || choice_vote_count_total != dispute.revealed_vote_count
        || pot.payout_mode != OracleSambaEmergencyPayoutMode::Open
        || pot.remaining_liability != pot.total_committed
        || pot.samba_mint != staking_pool.samba_mint
        || pot_token.amount < pot.remaining_liability
        || staking_pool.governance_lock_count == 0
        || staking_pool.samba_supply != dispute.snapshot_total_major_tokens
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let clock = Clock::get()?;
    let now = u64::try_from(clock.unix_timestamp)
        .map_err(|_| ProgramError::from(VaultError::InvalidSettlementRecord))?;
    oracle_usdc::ensure_oracle_update_cleanup_ready_at(
        &market,
        &source,
        &claim,
        Some(&guard),
        now,
    )?;

    let slot = clock.slot;
    claim.claim.status = OracleClaimStatus::TimedOut;
    claim.samba_checkpoint_active = false;
    challenge.status = OracleChallengeStatus::Cancelled;
    guard.last_updated_slot = slot;
    staking_pool.governance_lock_count = staking_pool
        .governance_lock_count
        .checked_sub(1)
        .ok_or(VaultError::InvalidOracleStakingPool)?;
    staking_pool.last_updated_slot = slot;
    decrement_pending_oracle_resolution(&mut month)?;
    month.last_updated_slot = slot;

    dispute.status = OracleChallengeStatus::Accepted;
    dispute.resolved_choice = 2;
    dispute.winning_choice = 0;
    dispute.winning_power = 0;
    dispute.winning_vote_count = 0;
    dispute.resolved_slot = slot;
    pot.payout_mode = OracleSambaEmergencyPayoutMode::RefundAll;
    pot.winning_choice = 0;
    pot.winning_power = 0;
    pot.winning_vote_count = 0;
    pot.registered_power = 0;
    pot.registered_vote_count = 0;
    pot.registered_base_total = 0;
    pot.dust_recipient_vote = Pubkey::default();
    pot.dust_amount = 0;
    pot.registration_finalized = true;
    pot.resolved_slot = slot;
    pot.last_updated_slot = slot;

    store_state(claim_info, &claim)?;
    store_state(challenge_info, &challenge)?;
    store_state(guard_info, &guard)?;
    store_state(staking_pool_info, &staking_pool)?;
    store_oracle_month_state(month_info, &month)?;
    store_state(pot_info, &pot)?;
    store_state(dispute_info, &dispute)
}
