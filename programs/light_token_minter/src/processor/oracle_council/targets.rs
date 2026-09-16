use super::*;

#[allow(clippy::too_many_arguments)]
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
    let (snapshot_slot, authority_version, choice_count) = match kind {
        OracleEmergencyDisputeKind::Source => {
            let resolving_existing_dispute = expected_active_dispute.is_some();
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
                || challenge.emergency_snapshot_version != 3
                || challenge.council_authority_version != 1
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
                challenge.council_authority_version,
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
                || !claim.council_review_pending
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
                || challenge.council_authority_version != 1
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
                challenge.council_authority_version,
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
                || challenge.council_authority_version != 1
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
                challenge.council_authority_version,
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
                || bucket.council_authority_version != 1
            {
                return Err(VaultError::InvalidOracleMedian.into());
            }
            (
                bucket.emergency_snapshot_slot,
                bucket.council_authority_version,
                2,
            )
        }
    };
    Ok(DerivedEmergencyPacket {
        fallback_choice,
        choice_count,
        snapshot_slot,
        authority_version,
    })
}

#[allow(clippy::too_many_arguments)]
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
#[allow(clippy::too_many_arguments)]
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
