use super::*;

pub(super) fn trusted_observation_expectation(
    deployment: &CurrentDeploymentStateV1,
) -> ProgramDataObservationExpectationV2 {
    ProgramDataObservationExpectationV2 {
        artifact_length: deployment.artifact_length,
        artifact_sha256: deployment.artifact_sha256,
        artifact_merkle_root: deployment.artifact_merkle_root,
        artifact_scheme_id: deployment.artifact_scheme_id,
        deployed_slot: deployment.deployed_slot,
        exact_capacity: Some(deployment.actual_programdata_capacity),
    }
}

pub(super) fn failed_primary_observation_expectation(
    primary: &UpgradeProposalV3,
) -> ProgramDataObservationExpectationV2 {
    candidate_observation_expectation(
        primary.artifact_length,
        primary.artifact_sha256,
        primary.artifact_chunk_merkle_root,
        primary.artifact_scheme_id,
        primary.upgrade_executed_slot,
    )
}

pub(super) fn candidate_observation_expectation(
    artifact_length: u64,
    artifact_sha256: [u8; 32],
    artifact_merkle_root: [u8; 32],
    artifact_scheme_id: [u8; 32],
    deployed_slot: u64,
) -> ProgramDataObservationExpectationV2 {
    ProgramDataObservationExpectationV2 {
        artifact_length,
        artifact_sha256,
        artifact_merkle_root,
        artifact_scheme_id,
        deployed_slot,
        // The failed candidate can have a checked extension which was never
        // committed to CurrentDeploymentState. Its live capacity is therefore
        // bound by the finalized observation and checkpoint, not guessed from
        // the last known-good deployment record.
        exact_capacity: None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn expected_programdata_observation_subject_digest(
    program_id: &Pubkey,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    expected_subject: &Pubkey,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    capacity: &ProgramDataCapacityPolicyV1,
    expectation: &ProgramDataObservationExpectationV2,
    purpose: ProgramDataObservationPurposeV1,
    generation: u64,
    minimum_required_capacity: u64,
) -> GovernanceResult<[u8; 32]> {
    compute_programdata_observation_subject_digest_v1(
        program_id,
        config_info.key,
        &config.target_program,
        &config.target_programdata,
        purpose,
        expected_subject,
        generation,
        gate_info.key,
        gate.status,
        gate.epoch,
        &gate.active_proposal,
        gate.freeze_slot,
        gate.freeze_reason_code,
        &capacity.policy_digest,
        expectation.artifact_length,
        &expectation.artifact_sha256,
        &expectation.artifact_merkle_root,
        &expectation.artifact_scheme_id,
        minimum_required_capacity,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_fresh_programdata_observation(
    program_id: &Pubkey,
    observation_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    expected_subject: &Pubkey,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    capacity: &ProgramDataCapacityPolicyV1,
    expectation: &ProgramDataObservationExpectationV2,
    purpose: ProgramDataObservationPurposeV1,
    subject_digest: &[u8; 32],
    expected_generation: u64,
    expected_digest: &[u8; 32],
    minimum_required_capacity: u64,
    slot: u64,
) -> Result<Box<ProgramDataObservationV1>, ProgramError> {
    let observation = load_fixed_controller_account::<ProgramDataObservationV1>(
        program_id,
        observation_info,
        ProgramDataObservationV1::LEN,
    )?;
    validate_programdata_observation_digest_v1(&observation)?;
    if derive_programdata_observation_pda(
        program_id,
        &config.target_program,
        purpose as u8,
        subject_digest,
        expected_generation,
    ) != (*observation_info.key, observation.bump)
        || observation.status != ProgramDataObservationStatusV1::Finalized
        || observation.controller_program != *program_id
        || observation.controller_config != *config_info.key
        || observation.capacity_policy != *capacity_info.key
        || observation.capacity_policy_digest != capacity.policy_digest
        || observation.purpose != purpose
        || observation.subject != *expected_subject
        || observation.subject_digest != *subject_digest
        || observation.generation != expected_generation
        || observation.protocol_gate != *gate_info.key
        || observation.gate_status != gate.status
        || observation.gate_epoch != gate.epoch
        || observation.gate_active_proposal != gate.active_proposal
        || observation.gate_freeze_slot != gate.freeze_slot
        || observation.gate_freeze_reason_code != gate.freeze_reason_code
        || observation.target_program != config.target_program
        || observation.target_programdata != config.target_programdata
        || observation.upgradeable_loader != config.upgradeable_loader
        || observation.expected_artifact_length != expectation.artifact_length
        || observation.expected_artifact_sha256 != expectation.artifact_sha256
        || observation.expected_artifact_merkle_root != expectation.artifact_merkle_root
        || observation.expected_artifact_scheme_id != expectation.artifact_scheme_id
        || expectation.artifact_scheme_id != capacity.artifact_scheme_id
        || observation.artifact_chunk_size != capacity.artifact_chunk_size
        || observation.minimum_required_capacity != minimum_required_capacity
        || observation.raw_observation_scheme_id != capacity.observation_scheme_id
        || observation.raw_chunk_size != capacity.observation_chunk_size
        || observation.deployed_slot != expectation.deployed_slot
        || observation.actual_capacity > capacity.maximum_payload_capacity
        || expectation
            .exact_capacity
            .is_some_and(|expected| observation.actual_capacity != expected)
        || observation.upgrade_authority != OptionalPubkeyV1::some(config.authority_pda)?
        || observation.observation_digest != *expected_digest
        || observation.finalized_slot == 0
        || observation.finalized_slot > slot
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    let expected_subject = expected_programdata_observation_subject_digest(
        program_id,
        config_info,
        gate_info,
        expected_subject,
        config,
        gate,
        capacity,
        expectation,
        purpose,
        expected_generation,
        minimum_required_capacity,
    )?;
    if expected_subject != observation.subject_digest {
        return Err(GovernanceError::InvalidProgramDataObservation.into());
    }
    Ok(observation)
}

pub(super) fn require_observation_matches_runtime(
    observation: &ProgramDataObservationV1,
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    capacity: &ProgramDataCapacityPolicyV1,
) -> ProgramResult {
    let runtime =
        read_runtime_programdata_header(target_program, target_programdata, config, capacity)?;
    let program_data = target_program.try_borrow_data()?;
    if program_data.len() != LOADER_PROGRAM_ACCOUNT_LEN {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    let mut program_header = [0u8; LOADER_PROGRAM_ACCOUNT_LEN];
    program_header.copy_from_slice(&program_data);
    drop(program_data);
    let programdata_data = target_programdata.try_borrow_data()?;
    let mut programdata_header = [0u8; LOADER_PROGRAMDATA_METADATA_LEN];
    programdata_header.copy_from_slice(
        programdata_data
            .get(..LOADER_PROGRAMDATA_METADATA_LEN)
            .ok_or(GovernanceError::StaleProgramDataObservation)?,
    );
    drop(programdata_data);
    if observation.program_owner != *target_program.owner
        || observation.program_executable != target_program.executable
        || observation.program_data_length != LOADER_PROGRAM_ACCOUNT_LEN as u64
        || observation.program_header_snapshot != program_header
        || observation.linked_programdata != config.target_programdata
        || observation.programdata_owner != *target_programdata.owner
        || observation.programdata_executable != target_programdata.executable
        || observation.programdata_header_snapshot != programdata_header
        || observation.deployed_slot != runtime.deployed_slot
        || observation.raw_data_length != runtime.raw_data_length
        || observation.actual_capacity != runtime.capacity
        || observation.upgrade_authority != OptionalPubkeyV1::some(config.authority_pda)?
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_emergency_freeze_observation(
    program_id: &Pubkey,
    observation_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    deployment_info: &AccountInfo<'_>,
    context: &LifecycleContext,
) -> Result<Box<EmergencyFreezeObservationV2>, ProgramError> {
    let observation = load_fixed_controller_account::<EmergencyFreezeObservationV2>(
        program_id,
        observation_info,
        EmergencyFreezeObservationV2::LEN,
    )?;
    validate_emergency_freeze_observation_digest_v2(&observation)?;
    if derive_emergency_freeze_observation_pda(
        program_id,
        &context.config.target_program,
        context.gate.epoch,
    ) != (*observation_info.key, observation.bump)
        || context.gate.status != GateStatusV1::EmergencyFrozen
        || context.gate.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        || context.gate.active_proposal != Pubkey::default()
        || observation.controller_program != *program_id
        || observation.controller_config != *config_info.key
        || observation.protocol_gate != *gate_info.key
        || observation.target_program != context.config.target_program
        || observation.target_programdata != context.config.target_programdata
        || observation.upgradeable_loader != context.config.upgradeable_loader
        || observation.controller_authority != context.config.authority_pda
        || observation.capacity_policy != *capacity_info.key
        || observation.capacity_policy_digest != context.capacity.policy_digest
        || observation.current_deployment_state != *deployment_info.key
        || observation.current_deployment_digest != context.deployment.deployment_digest
        || observation.current_deployment_generation != context.deployment.deployment_generation
        || observation.trusted_artifact_length != context.deployment.artifact_length
        || observation.trusted_artifact_sha256 != context.deployment.artifact_sha256
        || observation.trusted_artifact_merkle_root != context.deployment.artifact_merkle_root
        || observation.trusted_artifact_scheme_id != context.deployment.artifact_scheme_id
        || observation.minimum_required_capacity != context.deployment.artifact_length
        || observation.frozen_epoch != context.gate.epoch
        || observation.freeze_slot != context.gate.freeze_slot
        || observation.freeze_reason_code != context.gate.freeze_reason_code
        || observation.target_nonce != context.config.target_nonce
        || observation.deployed_programdata_slot != context.deployment.deployed_slot
        || observation.actual_capacity != context.deployment.actual_programdata_capacity
        || observation.observed_authority != OptionalPubkeyV1::some(context.config.authority_pda)?
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(observation)
}
