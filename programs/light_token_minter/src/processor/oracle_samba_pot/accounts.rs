use super::*;

pub(in crate::processor) fn ensure_emergency_resolver_coverage_lane(
    _month: &OracleMonthState,
    kind: OracleEmergencyDisputeKind,
    coverage_aware: bool,
) -> ProgramResult {
    if coverage_aware != (kind == OracleEmergencyDisputeKind::Source) {
        return Err(VaultError::OracleSkuCoverageInstructionRequired.into());
    }
    Ok(())
}

pub(in crate::processor) fn remove_emergency_supported_candidate_coverage(
    month: &mut OracleMonthState,
    coverage: Option<&mut OracleSkuCoverageManifest>,
    coverage_record: Option<&mut OracleSkuCoverageRecord>,
    source_still_contributes: bool,
) -> Result<bool, ProgramError> {
    if coverage.is_some() != coverage_record.is_some() {
        return Err(VaultError::InvalidAccountList.into());
    }
    if !source_still_contributes {
        return Ok(false);
    }
    decrement_oracle_supported_candidate_count(month)?;
    if let (Some(coverage), Some(record)) = (coverage, coverage_record) {
        remove_supported_source_from_sku_coverage(coverage, record)?;
    }
    Ok(true)
}

pub(in crate::processor) fn reopen_coverage_after_delayed_emergency_resolution(
    month: &mut OracleMonthState,
    coverage: &mut OracleSkuCoverageManifest,
    now: u64,
) -> ProgramResult {
    if now >= rulebook_schedule_boundaries(month)?.2 {
        coverage.coverage_finalized = false;
        coverage.coverage_complete_ts = 0;
        month.phase = OraclePhase::SourceSubmission;
    }
    Ok(())
}

pub(in crate::processor) fn load_dispute_v3(
    program_id: &Pubkey,
    dispute_info: &AccountInfo,
) -> Result<OracleEmergencyDisputeV3, ProgramError> {
    let dispute: OracleEmergencyDisputeV3 = load_exact_zero_padded_state(
        dispute_info,
        program_id,
        OracleEmergencyDisputeV3::LEN,
        VaultError::InvalidOracleEmergencyDispute,
    )?;
    let minimum_vote_amount =
        oracle_emergency_v3_minimum_vote_amount(dispute.snapshot_total_major_tokens)
            .ok_or(VaultError::InvalidOracleEmergencyDispute)?;
    let valid_choice_count = match dispute.kind {
        OracleEmergencyDisputeKind::Source => matches!(dispute.choice_count, 2 | 3),
        OracleEmergencyDisputeKind::Update | OracleEmergencyDisputeKind::Opening => {
            dispute.choice_count == 3
        }
        OracleEmergencyDisputeKind::BucketMedian => dispute.choice_count == 2,
    };
    let (expected, bump) =
        derive_oracle_emergency_dispute_v3_pda(program_id, &dispute.month, &dispute.dispute_id);
    if !dispute.is_initialized
        || !dispute.has_exact_v3_layout()
        || dispute.bump != bump
        || *dispute_info.key != expected
        || crate::pubkey_is_default(&dispute.month)
        || crate::bytes32_is_zero(&dispute.dispute_id)
        || crate::bytes32_is_zero(&dispute.target_id)
        || crate::pubkey_is_default(&dispute.target_account)
        || crate::pubkey_is_default(&dispute.pot)
        || dispute.minimum_vote_amount != minimum_vote_amount
        || dispute.committed_vote_count > ORACLE_MAX_V3_VOTERS
        || dispute.committed_power > dispute.snapshot_total_major_tokens
        || (dispute.committed_power == 0) != (dispute.committed_vote_count == 0)
        || u128::from(dispute.committed_vote_count) * u128::from(dispute.minimum_vote_amount)
            > u128::from(dispute.committed_power)
        || !valid_choice_count
        || dispute.fallback_choice >= dispute.choice_count
        || dispute.resolved_choice >= dispute.choice_count
        || dispute.winning_choice >= dispute.choice_count
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    Ok(dispute)
}

pub(in crate::processor) fn load_pot(
    program_id: &Pubkey,
    dispute_info: &AccountInfo,
    dispute: &OracleEmergencyDisputeV3,
    pot_info: &AccountInfo,
) -> Result<OracleSambaEmergencyPot, ProgramError> {
    let pot: OracleSambaEmergencyPot = load_exact_zero_padded_state(
        pot_info,
        program_id,
        OracleSambaEmergencyPot::LEN,
        VaultError::InvalidOracleEmergencyDispute,
    )?;
    let (expected, bump) = derive_oracle_samba_emergency_pot_pda(program_id, dispute_info.key);
    if !pot.is_initialized
        || pot.account_discriminator != OracleSambaEmergencyPot::ACCOUNT_DISCRIMINATOR
        || pot.account_version != OracleSambaEmergencyPot::ACCOUNT_VERSION
        || pot.bump != bump
        || *pot_info.key != expected
        || dispute.pot != *pot_info.key
        || pot.dispute != *dispute_info.key
        || pot.month != dispute.month
        || crate::pubkey_is_default(&pot.samba_mint)
        || crate::pubkey_is_default(&pot.token_account)
        || pot.total_committed != dispute.committed_power
        || pot.committed_vote_count != dispute.committed_vote_count
        || pot.committed_vote_count > ORACLE_MAX_V3_VOTERS
        || (pot.total_committed == 0) != (pot.committed_vote_count == 0)
        || pot.settled_vote_count > pot.committed_vote_count
        || pot.winning_power > pot.total_committed
        || pot.winning_vote_count > pot.committed_vote_count
        || pot.registered_power > pot.winning_power
        || pot.registered_vote_count > pot.winning_vote_count
        || pot.registered_base_total > pot.total_committed
        || pot.total_paid.checked_add(pot.remaining_liability) != Some(pot.total_committed)
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let valid_mode_shape = match pot.payout_mode {
        OracleSambaEmergencyPayoutMode::Open => {
            dispute.status == OracleChallengeStatus::Open
                && pot.remaining_liability == pot.total_committed
                && pot.total_paid == 0
                && pot.settled_vote_count == 0
                && pot.winning_power == 0
                && pot.winning_vote_count == 0
                && pot.registered_power == 0
                && pot.registered_vote_count == 0
                && pot.registered_base_total == 0
                && crate::pubkey_is_default(&pot.dust_recipient_vote)
                && pot.dust_amount == 0
                && !pot.registration_finalized
                && pot.resolved_slot == 0
        }
        OracleSambaEmergencyPayoutMode::RefundAll => {
            dispute.status == OracleChallengeStatus::Accepted
                && pot.winning_power == 0
                && pot.winning_vote_count == 0
                && pot.registered_power == 0
                && pot.registered_vote_count == 0
                && pot.registered_base_total == 0
                && crate::pubkey_is_default(&pot.dust_recipient_vote)
                && pot.dust_amount == 0
                && pot.registration_finalized
                && pot.resolved_slot != 0
                && pot.resolved_slot == dispute.resolved_slot
        }
        OracleSambaEmergencyPayoutMode::Redistribute => {
            let registration_shape = if pot.registration_finalized {
                pot.registered_power == pot.winning_power
                    && pot.registered_vote_count == pot.winning_vote_count
                    && !crate::pubkey_is_default(&pot.dust_recipient_vote)
                    && pot.dust_amount
                        == pot
                            .total_committed
                            .checked_sub(pot.registered_base_total)
                            .unwrap_or(u64::MAX)
                    && u64::from(pot.winning_vote_count) > pot.dust_amount
            } else {
                pot.registered_power < pot.winning_power
                    && pot.registered_vote_count < pot.winning_vote_count
                    && pot.dust_amount == 0
                    && ((pot.registered_vote_count == 0
                        && crate::pubkey_is_default(&pot.dust_recipient_vote))
                        || (pot.registered_vote_count > 0
                            && !crate::pubkey_is_default(&pot.dust_recipient_vote)))
            };
            dispute.status == OracleChallengeStatus::Accepted
                && pot.winning_choice == dispute.winning_choice
                && pot.winning_power == dispute.winning_power
                && pot.winning_vote_count == dispute.winning_vote_count
                && pot.winning_power > 0
                && pot.winning_vote_count > 0
                && pot.winning_power <= pot.total_committed
                && pot.resolved_slot != 0
                && pot.resolved_slot == dispute.resolved_slot
                && registration_shape
        }
    };
    if !valid_mode_shape {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    Ok(pot)
}

pub(in crate::processor) fn load_vote_v3(
    program_id: &Pubkey,
    dispute_info: &AccountInfo,
    dispute: &OracleEmergencyDisputeV3,
    pot_info: &AccountInfo,
    vote_info: &AccountInfo,
) -> Result<OracleEmergencyVoteRecordV3, ProgramError> {
    let vote: OracleEmergencyVoteRecordV3 = load_exact_zero_padded_state(
        vote_info,
        program_id,
        OracleEmergencyVoteRecordV3::LEN,
        VaultError::InvalidOracleEmergencyDispute,
    )?;
    let (expected, bump) =
        derive_oracle_emergency_vote_v3_pda(program_id, dispute_info.key, &vote.voter);
    if !vote.is_initialized
        || !vote.has_exact_v3_layout()
        || vote.bump != bump
        || *vote_info.key != expected
        || vote.month != dispute.month
        || vote.dispute != *dispute_info.key
        || vote.pot != *pot_info.key
        || crate::pubkey_is_default(&vote.samba_mint)
        || vote.locked_amount == 0
        || vote.locked_amount < dispute.minimum_vote_amount
        || vote.voting_power != vote.locked_amount
        || vote.snapshot_power < vote.locked_amount
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    Ok(vote)
}

pub(in crate::processor) fn validate_pot_token_account(
    pot: &OracleSambaEmergencyPot,
    pot_info: &AccountInfo,
    pot_token_info: &AccountInfo,
) -> Result<TokenAccount, ProgramError> {
    let expected = crate::associated_token::get_associated_token_address_with_program_id(
        pot_info.key,
        &pot.samba_mint,
        &spl_token_program_id(),
    );
    if *pot_token_info.key != expected
        || *pot_token_info.key != pot.token_account
        || pot_token_info.owner != &spl_token_program_id()
    {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    let token = validate_vault_token_account(pot_token_info, &pot.samba_mint, pot_info.key)?;
    // The ATA is donation-capable. Only a shortfall is invalid.
    if token.amount < pot.remaining_liability {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    Ok(token)
}

pub(in crate::processor) fn validate_cash_emergency_target(
    program_id: &Pubkey,
    month_key: &Pubkey,
    month: &OracleMonthState,
    kind: OracleEmergencyDisputeKind,
    target_id: &[u8; 32],
    target_info: &AccountInfo,
    remaining_accounts: &[AccountInfo],
    resolved_choice: Option<u8>,
    expected_active_dispute: Option<&Pubkey>,
) -> Result<DerivedEmergencyPacket, ProgramError> {
    let fallback_choice = oracle_emergency_fallback_choice(kind);
    validate_oracle_emergency_choice(kind, fallback_choice)?;
    let (snapshot_slot, snapshot_total_major_tokens, choice_count) = match kind {
        OracleEmergencyDisputeKind::Source => {
            let resolving_existing_dispute =
                resolved_choice.is_some() && expected_active_dispute.is_some();
            let valid_phase = month.phase == OraclePhase::Scramble
                || (resolving_existing_dispute && month.phase == OraclePhase::SourceSubmission);
            if !valid_phase || remaining_accounts.len() < 2 {
                return Err(VaultError::InvalidOraclePhase.into());
            }
            let source_info = &remaining_accounts[0];
            let source_guard_info = &remaining_accounts[1];
            let challenge = load_valid_oracle_source_challenge(program_id, month_key, target_info)?;
            let source = load_valid_oracle_source(program_id, month_key, source_info)?;
            let has_comparison = !crate::pubkey_is_default(&challenge.comparison_source)
                && !crate::bytes32_is_zero(&challenge.comparison_source_id);
            if remaining_accounts.len() != if has_comparison { 4 } else { 2 } {
                return Err(VaultError::InvalidAccountList.into());
            }
            let choice_count = if has_comparison { 3 } else { 2 };
            if resolved_choice.is_some_and(|choice| choice >= choice_count) {
                return Err(VaultError::InvalidOracleEmergencyDispute.into());
            }
            if challenge.source != *source_info.key
                || challenge.source_id != source.source_id
                || challenge.challenge_id != *target_id
                || challenge.status != OracleChallengeStatus::RuleReviewUnresolved
                || challenge.emergency_snapshot_version != 2
                || challenge.emergency_snapshot_total_major_tokens == 0
                || challenge.rule_review_slot == 0
                || challenge.escrow_disposition != OracleEscrowDisposition::Unsettled
                || (!resolving_existing_dispute && source.status != OracleSourceStatus::Candidate)
                || !source_guard_info.is_writable
                || source_guard_info.key == source_info.key
                || source_guard_info.key == target_info.key
            {
                return Err(VaultError::InvalidOracleChallengeAccount.into());
            }
            let expected_dispute = expected_active_dispute.copied().unwrap_or_default();
            let source_guard = load_canonical_source_challenge_guard(
                program_id,
                month_key,
                source_info.key,
                &source.source_id,
                source_guard_info,
            )?;
            if source_guard.active_challenge != *target_info.key
                || source_guard.active_challenge_id != challenge.challenge_id
                || source_guard.active_dispute != expected_dispute
            {
                return Err(VaultError::InvalidOracleChallengeAccount.into());
            }
            if has_comparison {
                let comparison_info = &remaining_accounts[2];
                let comparison_guard_info = &remaining_accounts[3];
                let comparison = load_valid_oracle_source(program_id, month_key, comparison_info)?;
                if *comparison_info.key != challenge.comparison_source
                    || comparison.source_id != challenge.comparison_source_id
                    || (!resolving_existing_dispute
                        && comparison.status != OracleSourceStatus::Candidate)
                    || comparison.bucket_id != source.bucket_id
                    || comparison_info.key == source_info.key
                    || comparison.source_id == source.source_id
                    || !comparison_guard_info.is_writable
                    || comparison_guard_info.key == source_guard_info.key
                    || comparison_guard_info.key == source_info.key
                    || comparison_guard_info.key == comparison_info.key
                    || comparison_guard_info.key == target_info.key
                {
                    return Err(VaultError::InvalidOracleSourceAccount.into());
                }
                let comparison_guard = load_canonical_source_challenge_guard(
                    program_id,
                    month_key,
                    comparison_info.key,
                    &comparison.source_id,
                    comparison_guard_info,
                )?;
                if comparison_guard.active_challenge != *target_info.key
                    || comparison_guard.active_challenge_id != challenge.challenge_id
                    || comparison_guard.active_dispute != expected_dispute
                {
                    return Err(VaultError::InvalidOracleChallengeAccount.into());
                }
            }
            (
                challenge.rule_review_slot,
                challenge.emergency_snapshot_total_major_tokens,
                choice_count,
            )
        }
        OracleEmergencyDisputeKind::Update => {
            if month.phase != OraclePhase::Game || remaining_accounts.len() != 6 {
                return Err(VaultError::InvalidOraclePhase.into());
            }
            ensure_oracle_opening_resolution_complete(month)?;
            let claim_info = &remaining_accounts[0];
            let source_info = &remaining_accounts[1];
            let observations_info = &remaining_accounts[2];
            let active_manifest_info = &remaining_accounts[3];
            let challenge_guard_info = &remaining_accounts[4];
            let bucket_info = &remaining_accounts[5];
            let challenge = load_valid_oracle_update_challenge(program_id, month_key, target_info)?;
            let claim = load_valid_oracle_update_claim_v2_from_account(
                program_id,
                month_key,
                source_info.key,
                claim_info,
            )?;
            if claim.claim.account_discriminator != OracleUpdateClaimV2::ACCOUNT_DISCRIMINATOR
                || claim.claim.account_version != OracleUpdateClaimV2::ACCOUNT_VERSION
                || !claim.samba_checkpoint_active
            {
                return Err(VaultError::InvalidOracleUpdateAccount.into());
            }
            let source = load_valid_oracle_source(program_id, month_key, source_info)?;
            let observations = load_valid_oracle_source_observations(
                program_id,
                month_key,
                source_info.key,
                observations_info,
            )?;
            validate_oracle_observation_shape(&source, &observations)?;
            validate_active_oracle_source(
                program_id,
                month,
                month_key,
                active_manifest_info,
                &source,
            )?;
            let bucket = load_valid_oracle_bucket_median(program_id, month_key, bucket_info)?;
            ensure_live_revealed_oracle_update_claim(source_info.key, &source, &claim.claim)?;
            if challenge.claim != *claim_info.key
                || challenge.claim_id != claim.claim.claim_id
                || challenge.challenge_id != *target_id
                || challenge.status != OracleChallengeStatus::RuleReviewUnresolved
                || challenge.emergency_snapshot_total_major_tokens == 0
                || challenge.rule_review_slot == 0
                || challenge.bond != challenge.required_bond
                || challenge.escrow_disposition != OracleEscrowDisposition::Unsettled
                || claim.claim.escrow_disposition != OracleEscrowDisposition::Unsettled
                || claim.claim.source_time == 0
                || crate::bytes32_is_zero(&claim.claim.archive_url_hash)
                || challenge.alternative_source_time == 0
                || crate::bytes32_is_zero(&challenge.archive_url_hash)
                || bucket.bucket_id != source.bucket_id
                || !matches!(
                    bucket.status,
                    OracleBucketMedianStatus::Live | OracleBucketMedianStatus::Dirty
                )
                || (resolved_choice.is_some()
                    && (!observations_info.is_writable || !bucket_info.is_writable))
                || !challenge_guard_info.is_writable
            {
                return Err(VaultError::InvalidOracleUpdateAccount.into());
            }
            let challenge_guard = load_canonical_update_challenge_guard(
                program_id,
                month_key,
                claim_info.key,
                &claim.claim.claim_id,
                challenge_guard_info,
            )?;
            if challenge_guard.challenge != *target_info.key
                || challenge_guard.challenge_id != challenge.challenge_id
                || challenge_guard.resolution_step == 0
                || challenge_guard.resolution_step <= source.last_finalized_step
                || challenge_guard.active_dispute
                    != expected_active_dispute.copied().unwrap_or_default()
            {
                return Err(VaultError::InvalidOracleUpdateAccount.into());
            }
            (
                challenge.rule_review_slot,
                challenge.emergency_snapshot_total_major_tokens,
                3,
            )
        }
        OracleEmergencyDisputeKind::Opening => {
            if month.phase != OraclePhase::Opening || remaining_accounts.len() != 2 {
                return Err(VaultError::InvalidOraclePhase.into());
            }
            let claim_info = &remaining_accounts[0];
            let source_info = &remaining_accounts[1];
            let challenge =
                load_valid_oracle_opening_challenge(program_id, month_key, target_info)?;
            let source = load_valid_oracle_source(program_id, month_key, source_info)?;
            let claim = load_valid_oracle_opening_claim(
                program_id,
                month_key,
                source_info.key,
                claim_info,
                &source,
            )?;
            validate_oracle_opening_claim_challenge_pda(
                program_id,
                target_info.key,
                month_key,
                claim_info.key,
                &challenge.challenge_id,
            )?;
            if challenge.month != *month_key
                || challenge.claim != *claim_info.key
                || challenge.claim_attempt != claim.attempt
                || challenge.source != *source_info.key
                || challenge.source_id != source.source_id
                || challenge.challenge_id != *target_id
                || challenge.status != OracleChallengeStatus::RuleReviewUnresolved
                || challenge.emergency_snapshot_total_major_tokens == 0
                || challenge.rule_review_slot == 0
                || crate::bytes32_is_zero(&challenge.evidence_hash)
                || challenge.bond == 0
                || challenge.bond != challenge.required_bond
                || challenge.escrow_disposition != OracleEscrowDisposition::Unsettled
                || claim.escrow_disposition != OracleEscrowDisposition::Unsettled
                || challenge.canonical_locator_hash != source.canonical_locator_hash
                || challenge.source_definition_hash != source.source_definition_hash
                || claim.status != OracleOpeningClaimStatus::Challenged
                || source.status != OracleSourceStatus::OpeningPending
                || crate::bytes32_is_zero(&challenge.archive_url_hash)
            {
                return Err(VaultError::InvalidOracleChallengeAccount.into());
            }
            (
                challenge.rule_review_slot,
                challenge.emergency_snapshot_total_major_tokens,
                3,
            )
        }
        OracleEmergencyDisputeKind::BucketMedian => {
            if month.phase != OraclePhase::Game || !remaining_accounts.is_empty() {
                return Err(VaultError::InvalidOraclePhase.into());
            }
            let bucket: OracleBucketMedianState = load_exact_zero_padded_state(
                target_info,
                program_id,
                OracleBucketMedianState::LEN,
                VaultError::InvalidOracleMedian,
            )?;
            let (expected, bump) =
                derive_oracle_bucket_median_pda(program_id, month_key, &bucket.bucket_id);
            if *target_info.key != expected
                || !target_info.is_writable
                || !bucket.is_initialized
                || bucket.bump != bump
                || bucket.account_discriminator != OracleBucketMedianState::ACCOUNT_DISCRIMINATOR
                || bucket.account_version != OracleBucketMedianState::ACCOUNT_VERSION
                || bucket.month != *month_key
                || bucket.bucket_id != *target_id
                || bucket.status != OracleBucketMedianStatus::EmergencyRequired
                || bucket.eligible_source_count
                    >= minimum_oracle_bucket_eligible_sources(bucket.frozen_source_count)
                || bucket.emergency_snapshot_slot == 0
                || bucket.emergency_snapshot_total_samba == 0
            {
                return Err(VaultError::InvalidOracleMedian.into());
            }
            (
                bucket.emergency_snapshot_slot,
                bucket.emergency_snapshot_total_samba,
                2,
            )
        }
    };
    Ok(DerivedEmergencyPacket {
        fallback_choice,
        choice_count,
        snapshot_slot,
        snapshot_total_major_tokens,
    })
}

pub(in crate::processor) fn set_cash_emergency_guard_dispute_binding(
    program_id: &Pubkey,
    kind: OracleEmergencyDisputeKind,
    month: &Pubkey,
    target_info: &AccountInfo,
    remaining_accounts: &[AccountInfo],
    expected_active_dispute: &Pubkey,
    new_active_dispute: &Pubkey,
    slot: u64,
) -> ProgramResult {
    match kind {
        OracleEmergencyDisputeKind::Source => {
            let challenge = load_valid_oracle_source_challenge(program_id, month, target_info)?;
            transition_source_challenge_guard_dispute(
                program_id,
                month,
                &challenge.source,
                &challenge.source_id,
                &remaining_accounts[1],
                target_info.key,
                &challenge.challenge_id,
                expected_active_dispute,
                new_active_dispute,
                slot,
            )?;
            if !crate::pubkey_is_default(&challenge.comparison_source) {
                transition_source_challenge_guard_dispute(
                    program_id,
                    month,
                    &challenge.comparison_source,
                    &challenge.comparison_source_id,
                    &remaining_accounts[3],
                    target_info.key,
                    &challenge.challenge_id,
                    expected_active_dispute,
                    new_active_dispute,
                    slot,
                )?;
            }
        }
        OracleEmergencyDisputeKind::Update => {
            let challenge = load_valid_oracle_update_challenge(program_id, month, target_info)?;
            let guard_info = &remaining_accounts[4];
            let mut guard = load_canonical_update_challenge_guard(
                program_id,
                month,
                &challenge.claim,
                &challenge.claim_id,
                guard_info,
            )?;
            if guard.challenge != *target_info.key
                || guard.challenge_id != challenge.challenge_id
                || guard.active_dispute != *expected_active_dispute
            {
                return Err(VaultError::InvalidOracleUpdateAccount.into());
            }
            guard.active_dispute = *new_active_dispute;
            guard.last_updated_slot = slot;
            store_state(guard_info, &guard)?;
        }
        OracleEmergencyDisputeKind::Opening => {}
        OracleEmergencyDisputeKind::BucketMedian => {
            return Err(VaultError::InvalidOracleEmergencyDispute.into())
        }
    }
    Ok(())
}

#[inline(never)]
fn transition_source_challenge_guard_dispute(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    source_id: &[u8; 32],
    guard_info: &AccountInfo,
    challenge: &Pubkey,
    challenge_id: &[u8; 32],
    expected_dispute: &Pubkey,
    new_dispute: &Pubkey,
    slot: u64,
) -> ProgramResult {
    let mut guard =
        load_canonical_source_challenge_guard(program_id, month, source, source_id, guard_info)?;
    if crate::pubkey_is_default(new_dispute) {
        release_source_challenge_guard(
            &mut guard,
            challenge,
            challenge_id,
            expected_dispute,
            slot,
        )?;
    } else {
        bind_source_challenge_guard_dispute(
            &mut guard,
            challenge,
            challenge_id,
            expected_dispute,
            new_dispute,
            slot,
        )?;
    }
    store_state(guard_info, &guard)
}
