use super::*;

#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn apply_cash_emergency_resolution(
    program_id: &Pubkey,
    remaining_accounts: &[AccountInfo],
    month: &mut OracleMonthState,
    dispute: &CouncilResolutionContext,
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
            let prior_state = source.current_state;
            if claim.prior_finalized_step != source.last_finalized_step {
                return Err(VaultError::InvalidOracleUpdateAccount.into());
            }
            oracle_usdc::ensure_update_challenge_elapsed_at(&claim, current_unix_timestamp()?)?;
            let challenge_guard = load_canonical_update_challenge_guard(
                program_id,
                &dispute.month,
                claim_info.key,
                &claim.claim.claim_id,
                challenge_guard_info,
            )?;
            if challenge_guard.challenge != *target_info.key
                || challenge_guard.active_dispute != dispute.case_key
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
                    if challenge.alternative_state != prior_state {
                        append_oracle_source_observation(
                            &mut source,
                            &mut observations,
                            challenge.alternative_state,
                            challenge.alternative_source_time,
                            &challenge.evidence_hash,
                            &challenge.archive_url_hash,
                        )?;
                        source.current_state = challenge.alternative_state;
                    }
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
            claim.council_review_pending = false;
            if matches!(resolved_choice, 0 | 1) && source.current_state != prior_state {
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
                1 => {
                    // Owner-approved terminal outcome: retain the last accepted
                    // on-chain bucket value, with no new sample or freshness.
                    // The rejected verdict and its vote payouts remain recorded.
                    bucket.status = OracleBucketMedianStatus::EmergencyRejected;
                }
                _ => return Err(VaultError::InvalidOracleEmergencyDispute.into()),
            }
            store_state(target_info, &bucket)
        }
    }
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_emergency_resolution_and_checkpoint<'info>(
    program_id: &Pubkey,
    resolution_accounts: &[AccountInfo<'info>],
    month_key: &Pubkey,
    month: &mut OracleMonthState,
    dispute_key: &Pubkey,
    dispute: &CouncilResolutionContext,
    target_info: &AccountInfo<'info>,
    cranker_info: &AccountInfo<'info>,
    resolved_choice: u8,
    coverage_context: Option<(&AccountInfo<'info>, &AccountInfo<'info>)>,
    merge_reward_context: Option<(
        &AccountInfo<'info>,
        &AccountInfo<'info>,
        &AccountInfo<'info>,
    )>,
    checkpoint_accounts: &[AccountInfo<'info>],
) -> ProgramResult {
    let source_before = if dispute.kind == OracleEmergencyDisputeKind::Update && resolved_choice < 2
    {
        Some(load_valid_oracle_source(
            program_id,
            month_key,
            &resolution_accounts[1],
        )?)
    } else {
        None
    };
    apply_cash_emergency_resolution(
        program_id,
        resolution_accounts,
        month,
        dispute,
        target_info,
        resolved_choice,
        coverage_context,
        merge_reward_context,
    )?;
    if let Some(before) = source_before {
        let source_info = &resolution_accounts[1];
        let after = load_valid_oracle_source(program_id, month_key, source_info)?;
        if after.current_state == before.current_state {
            if !checkpoint_accounts.is_empty() {
                return Err(VaultError::InvalidAccountList.into());
            }
            return Ok(());
        }
        let observations = load_valid_oracle_source_observations(
            program_id,
            month_key,
            source_info.key,
            &resolution_accounts[2],
        )?;
        let event = if resolved_choice == 0 {
            let claim = load_valid_oracle_update_claim_v2_from_account(
                program_id,
                month_key,
                source_info.key,
                &resolution_accounts[0],
            )?;
            crate::processor::oracle_carry::AcceptedEvent {
                event: *dispute_key,
                value: claim.claim.new_state,
                observed_at: claim.claim.source_time,
                evidence_hash: claim.claim.evidence_hash,
                archive_hash: claim.claim.archive_url_hash,
                contributor: claim.claim.claimant,
            }
        } else {
            let challenge = load_valid_oracle_update_challenge(program_id, month_key, target_info)?;
            crate::processor::oracle_carry::AcceptedEvent {
                event: *dispute_key,
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
    Ok(())
}
