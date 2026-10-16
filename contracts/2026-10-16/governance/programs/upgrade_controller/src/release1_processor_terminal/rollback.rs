use super::*;

/// Keeps the target continuously frozen while switching the active proposal to
/// the exact, precommitted rollback. The already-consumed target nonce is not
/// incremented again.
pub fn process_activate_rollback_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ActivateRollbackV1,
) -> ProgramResult {
    let [config_info, policy_info, gate_info, primary_info, rollback_info, rollback_buffer_verification_info, primary_programdata_verification_info, failure_info, target_program, target_programdata, authority_info, loader_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    for account in [
        config_info,
        policy_info,
        primary_info,
        rollback_buffer_verification_info,
        primary_programdata_verification_info,
        failure_info,
        target_programdata,
        authority_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(gate_info, true, false, false)?;
    validate_exact_privileges(rollback_info, true, false, false)?;
    validate_exact_privileges(target_program, false, false, true)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    require_distinct_accounts(&[
        config_info,
        policy_info,
        gate_info,
        primary_info,
        rollback_info,
        rollback_buffer_verification_info,
        primary_programdata_verification_info,
        failure_info,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
    ])?;
    if *loader_info.key != UPGRADEABLE_LOADER_ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
    let mut gate = load_gate(program_id, gate_info, config_info, &config)?;
    let primary = load_proposal(program_id, primary_info, config_info, &config)?;
    let rollback = load_proposal(program_id, rollback_info, config_info, &config)?;
    check_proposal_expectation_with_policy(
        &instruction.expected_primary,
        &primary,
        &config,
        &policy,
        &gate,
    )?;
    check_proposal_expectation_with_policy(
        &instruction.expected_rollback,
        &rollback,
        &config,
        &policy,
        &gate,
    )?;
    validate_active_primary_binding(&primary, primary_info.key, &config, &gate)?;
    validate_reciprocal_rollback(
        &primary,
        primary_info.key,
        &rollback,
        rollback_info.key,
        &config,
    )?;
    if !matches!(
        primary.state,
        ProposalStateV2::UpgradeExecuted | ProposalStateV2::ProgramDataVerified
    ) || rollback.state != ProposalStateV2::Timelocked
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }

    let slot = Clock::get()?.slot;
    require_policy_active(&policy, slot)?;
    validate_rollback_activation_timing(&primary, &rollback, &config, slot)?;

    let primary_verification = load_programdata_verification_for_failure(
        program_id,
        primary_programdata_verification_info,
        primary_info,
        config_info,
        &config,
        &primary,
    )?;
    if primary_verification.status != instruction.expected_primary_programdata_verification_status
        || primary_verification.finalized_slot
            != instruction.expected_primary_programdata_verification_finalized_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    if *authority_info.key != config.authority_pda
        || *target_program.key != config.target_program
        || *target_programdata.key != config.target_programdata
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_rollback_failure_evidence(
        program_id,
        failure_info,
        primary_info,
        config_info,
        gate_info,
        &config,
        &gate,
        &primary,
        &primary_verification,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        &instruction.expected_failure_evidence_digest,
        slot,
    )?;

    let rollback_verification = load_buffer_verification_without_live_buffer(
        program_id,
        rollback_buffer_verification_info,
        rollback_info,
        config_info,
        &config,
        &rollback,
    )?;
    if rollback_verification.status != instruction.expected_rollback_buffer_verification_status
        || rollback_verification.verified_chunk_bitmap
            != instruction.expected_rollback_verified_chunk_bitmap
        || rollback_verification.verified_chunk_count
            != instruction.expected_rollback_verified_chunk_count
        || rollback_verification.status != BufferVerificationStatusV1::Verified
        || rollback_verification.verified_chunk_count != rollback.chunk_count
        || rollback_verification.finalized_slot == 0
        || rollback_verification.finalized_slot > primary.upgrade_executed_slot
        || rollback_verification.finalized_slot > slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let next_epoch = gate
        .epoch
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let mut next_rollback = (*rollback).clone();
    next_rollback.state = ProposalStateV2::Frozen;
    next_rollback.freeze_gate_epoch = next_epoch;
    next_rollback.frozen_slot = slot;
    validate_proposal_digest_v2(&next_rollback)?;
    gate.epoch = next_epoch;
    gate.status = GateStatusV1::FrozenForUpgrade;
    gate.active_proposal = *rollback_info.key;
    gate.freeze_slot = slot;
    gate.freeze_reason_code = ROLLBACK_ACTIVATION_FREEZE_REASON_V1;
    gate.validate_static()?;
    let gate_bytes = encode_fixed_account(&*gate, ProtocolGateV1::LEN)?;
    let rollback_bytes = encode_fixed_account(&next_rollback, UpgradeProposalV2::LEN)?;
    commit_two_fixed_accounts(
        program_id,
        gate_info,
        &gate_bytes,
        ProtocolGateV1::LEN,
        rollback_info,
        &rollback_bytes,
        UpgradeProposalV2::LEN,
    )
}

pub(super) fn validate_rollback_activation_timing(
    primary: &UpgradeProposalV2,
    rollback: &UpgradeProposalV2,
    config: &ControllerConfigV1,
    slot: u64,
) -> ProgramResult {
    let rollback_ready_slot = primary
        .upgrade_executed_slot
        .checked_add(config.rollback_delay_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    // Activation is permissionless once the failure evidence and delay are
    // satisfied. Preserve one full checkpoint review window plus the later
    // no-extension rollback execution slot, otherwise a caller could activate
    // at the tail of expiry and strand the target in the new frozen epoch.
    let execution_horizon = slot
        .checked_add(config.council_review_slots())
        .and_then(|value| value.checked_add(1))
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if slot == 0
        || primary.upgrade_executed_slot == 0
        || slot < rollback.not_before_slot
        || slot < rollback_ready_slot
        || slot >= rollback.expiry_slot
        || execution_horizon >= rollback.expiry_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(())
}

pub(super) fn validate_reciprocal_rollback(
    primary: &UpgradeProposalV2,
    primary_key: &Pubkey,
    rollback: &UpgradeProposalV2,
    rollback_key: &Pubkey,
    config: &ControllerConfigV1,
) -> ProgramResult {
    verify_exact_proposal_timing(rollback, config)?;
    let consumed_nonce = primary
        .target_nonce
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if primary.proposal_class == ProposalClassV1::EmergencyRollback
        || !primary.rollback_proposal.present
        || primary.rollback_proposal.value != *rollback_key
        || !primary.rollback_buffer.present
        || primary.rollback_buffer.value != rollback.buffer_pubkey
        || primary.rollback_artifact_sha256 != rollback.artifact_sha256
        || primary.rollback_artifact_chunk_root != rollback.artifact_chunk_merkle_root
        || rollback.proposal_class != ProposalClassV1::EmergencyRollback
        || !rollback.primary_proposal.present
        || rollback.primary_proposal.value != *primary_key
        || rollback.rollback_proposal.present
        || rollback.rollback_buffer.present
        || rollback.target_nonce != primary.target_nonce
        || rollback.checkpoint_schema_id != primary.checkpoint_schema_id
        || rollback.checkpoint_policy_hash != primary.checkpoint_policy_hash
        || config.target_nonce != consumed_nonce
        || rollback.controller_config != primary.controller_config
        || rollback.protocol_gate != primary.protocol_gate
        || rollback.target_program != primary.target_program
        || rollback.target_programdata != primary.target_programdata
        || rollback.upgradeable_loader != primary.upgradeable_loader
        || rollback.authority_pda != primary.authority_pda
        || rollback.canonical_spill_treasury != primary.canonical_spill_treasury
        || rollback.buffer_loader_owner != config.upgradeable_loader
        || rollback.buffer_final_authority != config.authority_pda
        || rollback.expected_execution_pre_chunk_root != primary.artifact_chunk_merkle_root
        || rollback.current_capacity != primary.expected_post_capacity
        || rollback.expected_post_capacity != primary.expected_post_capacity
        || rollback.extension_delta != 0
        || rollback.council_approval_bitset & !VALID_APPROVAL_MASK != 0
        || rollback.council_approval_bitset.count_ones() as u8 != rollback.council_approval_count
        || rollback.council_approval_count != RELEASE1_APPROVAL_THRESHOLD
        || rollback.governance_satisfied_slot == 0
        || rollback.queued_slot == 0
    {
        return Err(GovernanceError::InvalidProposalCommitment.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_rollback_failure_evidence(
    program_id: &Pubkey,
    evidence_info: &AccountInfo<'_>,
    primary_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    primary: &UpgradeProposalV2,
    verification: &ProgramDataVerificationV1,
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    authority_info: &AccountInfo<'_>,
    loader_info: &AccountInfo<'_>,
    expected_digest: &[u8; 32],
    slot: u64,
) -> ProgramResult {
    match evidence_info.data_len() {
        ProgramDataFailureObservationV1::LEN => {
            if primary.state != ProposalStateV2::UpgradeExecuted
                || verification.status == ProgramDataVerificationStatusV1::Verified
                || verification.finalized_slot != 0
            {
                return Err(GovernanceError::InvalidStateTransition.into());
            }
            let failure = load_failure_observation(
                program_id,
                evidence_info,
                primary_info,
                config_info,
                gate_info,
                config,
                gate,
                primary,
            )?;
            if failure.observation_digest != *expected_digest
                || failure.finalized_slot < primary.upgrade_executed_slot
                || failure.finalized_slot > slot
            {
                return Err(GovernanceError::CrossAccountMismatch.into());
            }
            require_recoverable_rollback_failure_observation(&failure, config, verification)?;
            let runtime = capture_runtime_observation(target_program, target_programdata)?;
            require_observation_unchanged(&runtime, &failure)
        }
        StateCheckpointV1::LEN => {
            if primary.state != ProposalStateV2::ProgramDataVerified
                || verification.status != ProgramDataVerificationStatusV1::Verified
                || verification.finalized_slot == 0
                || verification.finalized_slot != primary.programdata_verified_slot
            {
                return Err(GovernanceError::InvalidStateTransition.into());
            }
            let checkpoint = load_rejected_poststate_failure(
                program_id,
                evidence_info,
                primary_info,
                config_info,
                config,
                gate,
                primary,
                verification,
                slot,
            )?;
            if checkpoint.checkpoint_digest != *expected_digest {
                return Err(GovernanceError::Release1DigestMismatch.into());
            }
            validate_live_verified_programdata(
                target_program,
                target_programdata,
                authority_info,
                loader_info,
                config,
                primary,
                verification,
            )
        }
        _ => Err(GovernanceError::InvalidAccountSize.into()),
    }
}

/// Only byte failures leave the canonical Loader-v3 graph usable by the typed
/// rollback CPI. Header, authority, and capacity failures are immutable audit
/// evidence, but activating a rollback for them would consume the one-way gate
/// transition into a lifecycle that cannot bind an executable Prestate.
pub(super) fn require_recoverable_rollback_failure_observation(
    failure: &ProgramDataFailureObservationV1,
    config: &ControllerConfigV1,
    verification: &ProgramDataVerificationV1,
) -> ProgramResult {
    if !matches!(
        failure.mismatch_class,
        ProgramDataMismatchClassV1::PayloadLeaf | ProgramDataMismatchClassV1::ZeroTail
    ) {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let expected_data_length = failure
        .actual_capacity
        .checked_add(LOADER_PROGRAMDATA_METADATA_LEN as u64)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if failure.actual_program_owner != config.upgradeable_loader
        || !failure.actual_program_executable
        || failure.actual_program_data_length != LOADER_PROGRAM_ACCOUNT_LEN as u64
        || !failure.program_header_present
        || linked_value(&failure.actual_linked_programdata) != Some(config.target_programdata)
        || !failure.raw_hash_complete
        || failure.actual_raw_programdata_sha256 == [0; 32]
        || failure.actual_owner != config.upgradeable_loader
        || failure.actual_executable
        || failure.actual_data_length != expected_data_length
        || !failure.programdata_header_present
        || failure.actual_programdata_slot != verification.deployed_slot
        || failure.actual_capacity != verification.capacity
        || linked_value(&failure.actual_authority) != Some(config.authority_pda)
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn load_rejected_poststate_failure(
    program_id: &Pubkey,
    checkpoint_info: &AccountInfo<'_>,
    primary_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    primary: &UpgradeProposalV2,
    verification: &ProgramDataVerificationV1,
    slot: u64,
) -> Result<Box<StateCheckpointV1>, ProgramError> {
    let checkpoint = load_fixed_controller_account::<StateCheckpointV1>(
        program_id,
        checkpoint_info,
        StateCheckpointV1::LEN,
    )?;
    validate_state_checkpoint_digest_v1(&checkpoint)?;
    if derive_checkpoint_pda(program_id, primary_info.key, CheckpointPhaseV1::Poststate)
        != (*checkpoint_info.key, checkpoint.bump)
        || *checkpoint_info.key != primary.required_poststate_checkpoint
        || checkpoint.phase != StateCheckpointPhaseV1::Poststate
        || checkpoint.controller_config != *config_info.key
        || checkpoint.proposal != *primary_info.key
        || checkpoint.emergency_resolution != Pubkey::default()
        || checkpoint.subject_digest != primary.proposal_digest
        || checkpoint.target_program != config.target_program
        || checkpoint.target_programdata != config.target_programdata
        || checkpoint.gate_epoch != gate.epoch
        || checkpoint.gate_epoch != primary.freeze_gate_epoch
        || checkpoint.target_programdata_slot != verification.deployed_slot
        || checkpoint.target_raw_programdata_commitment != verification.raw_programdata_hash
        || checkpoint.target_capacity != verification.capacity
        || checkpoint.schema_identifier != primary.checkpoint_schema_id
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_rejected_checkpoint_finalization(&checkpoint, verification.finalized_slot, slot)?;
    Ok(checkpoint)
}

pub(super) fn validate_rejected_checkpoint_finalization(
    checkpoint: &StateCheckpointV1,
    verification_finalized_slot: u64,
    current_slot: u64,
) -> ProgramResult {
    let approval_mask_is_exact = checkpoint.approval_bitset & !VALID_APPROVAL_MASK == 0
        && checkpoint.approval_bitset.count_ones() as u8 == checkpoint.approval_count
        && checkpoint.approval_count == RELEASE1_APPROVAL_THRESHOLD;
    if checkpoint.accepted
        || checkpoint.forbidden_drift_count == 0
        || checkpoint.approval_council_version == 0
        || checkpoint.approval_council_hash == [0; 32]
        || !approval_mask_is_exact
        || checkpoint.finalized_slot < verification_finalized_slot
        || checkpoint.finalized_slot > current_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}
