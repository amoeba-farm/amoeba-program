use super::*;

/// Switches the continuously frozen gate from a failed primary to its exact
/// precommitted rollback.  No nonce is consumed and no Loader CPI occurs.
pub fn process_activate_rollback_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ActivateRollbackV2,
) -> ProgramResult {
    let [config_info, policy_info, gate_info, primary_info, rollback_info, rollback_buffer_verification_info, primary_verification_info, failure_info, capacity_info, deployment_info, observation_info, target_program, target_programdata, authority_info, loader_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    for account in [
        config_info,
        policy_info,
        primary_info,
        rollback_buffer_verification_info,
        primary_verification_info,
        failure_info,
        capacity_info,
        deployment_info,
        observation_info,
        target_programdata,
        authority_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(gate_info, true, false, false)?;
    validate_exact_privileges(rollback_info, true, false, false)?;
    if target_program.is_signer || target_program.is_writable {
        return Err(GovernanceError::InvalidAccountPrivileges.into());
    }
    validate_exact_privileges(loader_info, false, false, true)?;
    require_distinct_accounts(&[
        config_info,
        policy_info,
        gate_info,
        primary_info,
        rollback_info,
        rollback_buffer_verification_info,
        primary_verification_info,
        failure_info,
        capacity_info,
        deployment_info,
        observation_info,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
    ])?;
    require_program_ids(loader_info, None, None, None, None)?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
    let mut gate = load_gate(program_id, gate_info, config_info, &config)?;
    let primary = load_proposal(program_id, primary_info, config_info, &config)?;
    let rollback = load_proposal(program_id, rollback_info, config_info, &config)?;
    validate_proposal_guard(
        &instruction.expected_primary,
        &primary,
        &config,
        &gate,
        primary_info.key,
    )?;
    validate_nonactive_rollback_guard(&instruction.expected_rollback, &rollback, &config, &gate)?;
    if gate.status != GateStatusV1::FrozenForUpgrade
        || gate.active_proposal != *primary_info.key
        || primary.state != ProposalStateV2::UpgradeExecuted
        || rollback.state != ProposalStateV2::Timelocked
        || rollback.proposal_class != ProposalClassV1::EmergencyRollback
        || !rollback.primary_proposal.present
        || rollback.primary_proposal.value != *primary_info.key
        || !primary.rollback_proposal.present
        || primary.rollback_proposal.value != *rollback_info.key
        || !primary.rollback_buffer.present
        || primary.rollback_buffer.value != rollback.buffer_pubkey
        || primary.rollback_artifact_length != rollback.artifact_length
        || primary.rollback_artifact_sha256 != rollback.artifact_sha256
        || primary.rollback_artifact_chunk_root != rollback.artifact_chunk_merkle_root
        || primary.rollback_artifact_scheme_id != rollback.artifact_scheme_id
        || primary.target_nonce != rollback.target_nonce
    {
        return Err(GovernanceError::InvalidProposalCommitment.into());
    }
    let slot = Clock::get()?.slot;
    let rollback_ready_slot = primary
        .upgrade_executed_slot
        .checked_add(config.rollback_delay_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if slot == 0
        || slot < policy.activation_slot
        || slot < rollback.not_before_slot
        || slot >= rollback.expiry_slot
        || primary.upgrade_executed_slot == 0
        || slot < rollback_ready_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let capacity = load_capacity_policy(program_id, capacity_info, config_info, &config)?;
    let deployment = load_current_deployment(
        program_id,
        deployment_info,
        config_info,
        capacity_info,
        &config,
        &capacity,
    )?;
    require_deployment_guard(&instruction.expected_primary, &primary, &deployment)?;
    require_deployment_guard(&instruction.expected_rollback, &rollback, &deployment)?;
    let primary_verification_generation =
        if instruction.expected_primary_verification_generation == 0 {
            if *primary_verification_info.key != primary.programdata_verification
                || primary_verification_info.owner != &system_program::ID
                || primary_verification_info.data_len() != 0
            {
                return Err(GovernanceError::CrossAccountMismatch.into());
            }
            0
        } else {
            let verification = load_programdata_verification(
                program_id,
                primary_verification_info,
                primary_info,
                config_info,
                &primary,
            )?;
            if verification.status == ProgramDataVerificationStatusV2::Verified {
                return Err(GovernanceError::InvalidStateTransition.into());
            }
            verification.verification_generation
        };
    let failure = load_fixed_controller_account::<ProgramDataFailureObservationV2>(
        program_id,
        failure_info,
        ProgramDataFailureObservationV2::LEN,
    )?;
    validate_programdata_failure_observation_digest_v2(&failure)?;
    validate_rollback_activation_mismatch_class(failure.mismatch_class)?;
    if derive_programdata_failure_observation_pda(program_id, primary_info.key, gate.epoch)
        != (*failure_info.key, failure.bump)
        || failure.primary_proposal != *primary_info.key
        || failure.proposal_digest != primary.proposal_digest
        || failure.protocol_gate != *gate_info.key
        || failure.frozen_epoch != gate.epoch
        || failure.failure_digest != instruction.expected_failure_evidence_digest
        || failure.programdata_observation != *observation_info.key
        || failure.observation_generation != instruction.expected_programdata_observation_generation
        || primary_verification_generation != instruction.expected_primary_verification_generation
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let observation = load_observation_any_status(
        program_id,
        observation_info,
        config_info,
        capacity_info,
        primary_info,
        &config,
        &capacity,
    )?;
    let observation_bytes = observation_info.try_borrow_data()?;
    let observation_state_hash = hashv(&[&observation_bytes]).to_bytes();
    drop(observation_bytes);
    if observation.generation != instruction.expected_programdata_observation_generation
        || observation_state_hash != instruction.expected_programdata_observation_state_hash
        || observation.purpose != failure.observation_purpose
        || observation.subject_digest != failure.observation_subject_digest
        || (observation.status == ProgramDataObservationStatusV1::Finalized)
            != failure.observation_finalized
        || (failure.observation_finalized
            && (observation.final_raw_merkle_root != failure.observation_root
                || observation.observation_digest != failure.observation_digest))
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    if *target_program.key != config.target_program
        || *target_programdata.key != config.target_programdata
        || *authority_info.key != config.authority_pda
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let runtime = capture_runtime_graph(target_program, target_programdata)?;
    if !runtime_matches_failure_observation(&runtime, &failure) {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    let rollback_verification = load_buffer_verification_without_live_buffer(
        program_id,
        rollback_buffer_verification_info,
        rollback_info,
        config_info,
        &config,
        &rollback,
    )?;
    if rollback_verification.status != instruction.expected_rollback_buffer_verification_status
        || rollback_verification.status != BufferVerificationStatusV1::Verified
        || rollback_verification.verified_chunk_bitmap
            != instruction.expected_rollback_verified_chunk_bitmap
        || rollback_verification.verified_chunk_count
            != instruction.expected_rollback_verified_chunk_count
        || rollback_verification.verified_chunk_count != rollback.artifact_chunk_count
        || rollback_verification.finalized_slot
            != instruction.expected_rollback_buffer_finalized_slot
        || rollback_verification.finalized_slot == 0
        || rollback_verification.finalized_slot > primary.upgrade_executed_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let next_epoch = gate
        .epoch
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if next_epoch != instruction.expected_next_gate_epoch {
        return Err(GovernanceError::InvalidProposalEpoch.into());
    }
    let mut next_rollback = (*rollback).clone();
    next_rollback.state = ProposalStateV2::Frozen;
    next_rollback.freeze_gate_epoch = next_epoch;
    next_rollback.frozen_slot = slot;
    validate_upgrade_proposal_digest_v3(&next_rollback)?;
    gate.epoch = next_epoch;
    gate.status = GateStatusV1::FrozenForUpgrade;
    gate.active_proposal = *rollback_info.key;
    gate.freeze_slot = slot;
    gate.freeze_reason_code = ROLLBACK_ACTIVATION_FREEZE_REASON_V1;
    gate.validate_static()?;
    let gate_bytes = encode_fixed_account(&*gate, ProtocolGateV1::LEN)?;
    let rollback_bytes = encode_fixed_account(&next_rollback, UpgradeProposalV3::LEN)?;
    commit_two_fixed_accounts(
        program_id,
        gate_info,
        &gate_bytes,
        ProtocolGateV1::LEN,
        rollback_info,
        &rollback_bytes,
        UpgradeProposalV3::LEN,
    )
}

/// Loads the immutable primary-failure PDA selected by an EmergencyRollback
/// execution and proves that the same Loader-executable mismatch is still
/// present in the live ProgramData.  This is deliberately narrower than the
/// failure-observation surface: malformed linkage, ownership, header, or
/// authority cannot safely reach Loader Upgrade and therefore cannot be used
/// as an execution capability.
#[allow(clippy::too_many_arguments)]
pub(super) fn load_and_revalidate_rollback_failure(
    program_id: &Pubkey,
    failure_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    primary_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    deployment_info: &AccountInfo<'_>,
    rollback_info: &AccountInfo<'_>,
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    capacity: &ProgramDataCapacityPolicyV1,
    gate: &ProtocolGateV1,
    deployment: &CurrentDeploymentStateV1,
    primary: &UpgradeProposalV3,
    rollback: &UpgradeProposalV3,
) -> Result<Box<ProgramDataFailureObservationV2>, ProgramError> {
    let failure = load_fixed_controller_account::<ProgramDataFailureObservationV2>(
        program_id,
        failure_info,
        ProgramDataFailureObservationV2::LEN,
    )?;
    validate_programdata_failure_observation_digest_v2(&failure)?;
    let next_epoch = failure
        .frozen_epoch
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let consumed_nonce = primary
        .target_nonce
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if derive_programdata_failure_observation_pda(
        program_id,
        primary_info.key,
        failure.frozen_epoch,
    ) != (*failure_info.key, failure.bump)
        || !failure.finalized
        || failure.controller_config != *config_info.key
        || failure.protocol_gate != *gate_info.key
        || failure.primary_proposal != *primary_info.key
        || failure.proposal_digest != primary.proposal_digest
        || failure.capacity_policy != *capacity_info.key
        || failure.capacity_policy_digest != capacity.policy_digest
        || failure.current_deployment_state != *deployment_info.key
        || failure.current_deployment_digest != primary.current_deployment_digest
        || failure.current_deployment_generation != primary.current_deployment_generation
        || failure.target_program != config.target_program
        || failure.target_programdata != config.target_programdata
        || failure.upgradeable_loader != config.upgradeable_loader
        || failure.frozen_epoch != primary.freeze_gate_epoch
        || next_epoch != gate.epoch
        || rollback.freeze_gate_epoch != gate.epoch
        || gate.status != GateStatusV1::FrozenForUpgrade
        || gate.active_proposal != *rollback_info.key
        || failure.target_nonce != consumed_nonce
        || failure.target_nonce != config.target_nonce
        || failure.expected_artifact_length != primary.artifact_length
        || failure.expected_artifact_sha256 != primary.artifact_sha256
        || failure.expected_artifact_merkle_root != primary.artifact_chunk_merkle_root
        || failure.expected_artifact_scheme_id != primary.artifact_scheme_id
        || failure.minimum_required_capacity != primary.minimum_required_capacity
        || failure.observation_scheme_id != capacity.observation_scheme_id
        || failure.observation_purpose != ProgramDataObservationPurposeV1::PostUpgrade
        || failure.actual_programdata_slot != primary.upgrade_executed_slot
        || failure.actual_capacity != deployment.actual_programdata_capacity
        || !failure.actual_authority.present
        || failure.actual_authority.value != config.authority_pda
        || failure.finalized_slot < primary.upgrade_executed_slot
        || failure.finalized_slot > rollback.frozen_slot
        || !is_loader_executable_rollback_failure(failure.mismatch_class)
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let runtime = capture_runtime_graph(target_program, target_programdata)?;
    if !runtime_matches_failure_observation(&runtime, &failure)
        || !runtime.program_header_present
        || !runtime.linked_programdata.present
        || runtime.linked_programdata.value != config.target_programdata
        || runtime.program_owner != config.upgradeable_loader
        || !runtime.program_executable
        || runtime.programdata_owner != config.upgradeable_loader
        || runtime.programdata_executable
        || !runtime.programdata_header_present
        || !runtime.authority.present
        || runtime.authority.value != config.authority_pda
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    let data = target_programdata.try_borrow_data()?;
    let payload = data
        .get(LOADER_PROGRAMDATA_METADATA_LEN..)
        .ok_or(GovernanceError::InvalidRelease1Account)?;
    let (expected_leaf, actual_leaf) = match failure.mismatch_class {
        ProgramDataMismatchClassV2::ArtifactPayload => {
            let exact = exact_region_chunk(
                payload,
                0,
                primary.artifact_length,
                primary.artifact_chunk_size,
                failure.failing_chunk_index,
            )?;
            (
                failure.expected_leaf_hash,
                artifact_chunk_leaf_hash(failure.failing_chunk_index, exact)?,
            )
        }
        ProgramDataMismatchClassV2::ZeroTail => {
            let tail_length = runtime
                .actual_capacity
                .checked_sub(primary.artifact_length)
                .ok_or(GovernanceError::InvalidCapacityPlan)?;
            let exact = exact_region_chunk(
                payload,
                primary.artifact_length,
                tail_length,
                primary.artifact_chunk_size,
                failure.failing_chunk_index,
            )?;
            (
                programdata_zero_tail_zero_hash(failure.failing_chunk_index, exact.len())?,
                programdata_zero_tail_chunk_hash(failure.failing_chunk_index, exact)?,
            )
        }
        _ => return Err(GovernanceError::InvalidRelease1Account.into()),
    };
    if expected_leaf != failure.expected_leaf_hash
        || actual_leaf != failure.actual_leaf_hash
        || expected_leaf == actual_leaf
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    drop(data);
    Ok(failure)
}

pub(super) fn is_loader_executable_rollback_failure(
    mismatch_class: ProgramDataMismatchClassV2,
) -> bool {
    matches!(
        mismatch_class,
        ProgramDataMismatchClassV2::ArtifactPayload | ProgramDataMismatchClassV2::ZeroTail
    )
}

pub(super) fn require_rollback_failure_guard(
    failure: &ProgramDataFailureObservationV2,
    instruction: &ExecuteUpgradeV2,
) -> ProgramResult {
    if instruction.expected_observation_digest != failure.failure_digest
        || instruction.expected_observation_generation != failure.observation_generation
        || instruction.expected_observation_root != failure.actual_leaf_hash
        || instruction.expected_observation_finalized_slot != failure.finalized_slot
        || instruction.expected_actual_capacity != failure.actual_capacity
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_counterpart(
    program_id: &Pubkey,
    counterpart_info: &AccountInfo<'_>,
    counterpart_verification_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    _authority_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV3,
    proposal_key: &Pubkey,
    instruction: &ExecuteUpgradeV2,
    slot: u64,
) -> ProgramResult {
    let counterpart = load_proposal(program_id, counterpart_info, config_info, config)?;
    if counterpart.proposal_digest != instruction.expected_counterpart_proposal_digest
        || counterpart.checkpoint_schema_id != proposal.checkpoint_schema_id
        || counterpart.checkpoint_policy_hash != proposal.checkpoint_policy_hash
        || counterpart.capacity_policy != proposal.capacity_policy
        || counterpart.capacity_policy_digest != proposal.capacity_policy_digest
        || counterpart.target_nonce != proposal.target_nonce
    {
        return Err(GovernanceError::InvalidProposalCommitment.into());
    }
    let verification = load_buffer_verification_without_live_buffer(
        program_id,
        counterpart_verification_info,
        counterpart_info,
        config_info,
        config,
        &counterpart,
    )?;
    if verification.status != instruction.expected_counterpart_buffer_verification_status {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    match proposal.proposal_class {
        ProposalClassV1::EmergencyRollback => {
            if !proposal.primary_proposal.present
                || proposal.primary_proposal.value != *counterpart_info.key
                || !counterpart.rollback_proposal.present
                || counterpart.rollback_proposal.value != *proposal_key
                || counterpart.proposal_class == ProposalClassV1::EmergencyRollback
                || verification.status != BufferVerificationStatusV1::ConsumedByUpgrade
                || !matches!(
                    counterpart.state,
                    ProposalStateV2::UpgradeExecuted
                        | ProposalStateV2::ProgramDataVerified
                        | ProposalStateV2::PoststateAccepted
                )
            {
                return Err(GovernanceError::InvalidProposalCommitment.into());
            }
        }
        ProposalClassV1::RoutineUpgrade
        | ProposalClassV1::EconomicChange
        | ProposalClassV1::ConstitutionalChange => {
            require_primary_execution_rollback_runway(
                slot,
                config.rollback_delay_slots,
                config.council_review_slots(),
                counterpart.expiry_slot,
            )?;
            if !proposal.rollback_proposal.present
                || proposal.rollback_proposal.value != *counterpart_info.key
                || !counterpart.primary_proposal.present
                || counterpart.primary_proposal.value != *proposal_key
                || counterpart.proposal_class != ProposalClassV1::EmergencyRollback
                || counterpart.state != ProposalStateV2::Timelocked
                || counterpart.not_before_slot > slot
                || counterpart.expiry_slot <= slot
                || verification.status != BufferVerificationStatusV1::Verified
                || verification.finalized_slot == 0
                || verification.finalized_slot > slot
                || !proposal.rollback_buffer.present
                || proposal.rollback_buffer.value != counterpart.buffer_pubkey
                || proposal.rollback_artifact_length != counterpart.artifact_length
                || proposal.rollback_artifact_sha256 != counterpart.artifact_sha256
                || proposal.rollback_artifact_chunk_root != counterpart.artifact_chunk_merkle_root
                || proposal.rollback_artifact_scheme_id != counterpart.artifact_scheme_id
            {
                return Err(GovernanceError::InvalidProposalCommitment.into());
            }
        }
        ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability => {
            return Err(GovernanceError::UnsupportedProposalClass.into());
        }
    }
    Ok(())
}

fn require_primary_execution_rollback_runway(
    slot: u64,
    rollback_delay_slots: u64,
    council_review_slots: u64,
    rollback_expiry_slot: u64,
) -> ProgramResult {
    let recovery_horizon = slot
        .checked_add(rollback_delay_slots)
        .and_then(|value| value.checked_add(council_review_slots))
        .and_then(|value| value.checked_add(1))
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if recovery_horizon >= rollback_expiry_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(())
}

fn validate_nonactive_rollback_guard(
    expected: &ProposalGuardV3,
    rollback: &UpgradeProposalV3,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
) -> ProgramResult {
    if expected.expected_proposal_digest != rollback.proposal_digest
        || expected.expected_state != rollback.state
        || expected.expected_gate_status != gate.status
        || expected.expected_gate_epoch != gate.epoch
        || expected.expected_target_nonce != config.target_nonce
        || expected.expected_capacity_policy_digest != rollback.capacity_policy_digest
        || rollback.state != ProposalStateV2::Timelocked
        || rollback.proposal_class != ProposalClassV1::EmergencyRollback
        || config.target_nonce
            != rollback
                .target_nonce
                .checked_add(1)
                .ok_or(GovernanceError::ArithmeticOverflow)?
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}
