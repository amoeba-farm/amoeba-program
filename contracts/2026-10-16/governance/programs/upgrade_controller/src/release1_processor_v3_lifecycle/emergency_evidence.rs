use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn load_emergency_resolution(
    program_id: &Pubkey,
    resolution_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    deployment_info: &AccountInfo<'_>,
    context: &LifecycleContext,
) -> Result<Box<EmergencyFreezeResolutionV2>, ProgramError> {
    let resolution = load_fixed_controller_account::<EmergencyFreezeResolutionV2>(
        program_id,
        resolution_info,
        EmergencyFreezeResolutionV2::LEN,
    )?;
    validate_emergency_freeze_resolution_digest_v2(&resolution)?;
    if derive_emergency_resolution_v2_pda(
        program_id,
        &context.config.target_program,
        resolution.frozen_epoch,
        resolution.approval_council_version,
    ) != (*resolution_info.key, resolution.bump)
        || resolution.controller_config != *config_info.key
        || resolution.protocol_gate != *gate_info.key
        || resolution.target_program != context.config.target_program
        || resolution.target_programdata != context.config.target_programdata
        || resolution.upgradeable_loader != context.config.upgradeable_loader
        || resolution.controller_authority != context.config.authority_pda
        || resolution.capacity_policy != *capacity_info.key
        || resolution.capacity_policy_digest != context.capacity.policy_digest
        || resolution.current_deployment_state != *deployment_info.key
        || resolution.current_deployment_digest != context.deployment.deployment_digest
        || resolution.current_deployment_generation != context.deployment.deployment_generation
        || resolution.frozen_epoch != context.gate.epoch
        || resolution.freeze_slot != context.gate.freeze_slot
        || resolution.freeze_reason_code != context.gate.freeze_reason_code
        || resolution.target_nonce != context.config.target_nonce
        || resolution.artifact_length != context.deployment.artifact_length
        || resolution.artifact_sha256 != context.deployment.artifact_sha256
        || resolution.artifact_merkle_root != context.deployment.artifact_merkle_root
        || resolution.artifact_scheme_id != context.deployment.artifact_scheme_id
        || resolution.minimum_required_capacity != context.deployment.artifact_length
        || resolution.emergency_checkpoint
            != derive_emergency_checkpoint_v2_pda(program_id, resolution_info.key).0
        || context.gate.status != GateStatusV1::EmergencyFrozen
        || context.gate.active_proposal != Pubkey::default()
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(resolution)
}

pub(super) fn check_emergency_resolution_guard(
    expected: &EmergencyResolutionGuardV2,
    resolution: &EmergencyFreezeResolutionV2,
    context: &LifecycleContext,
) -> ProgramResult {
    require_guard_deployment(
        &context.capacity,
        &context.deployment,
        &expected.expected_capacity_policy_digest,
        &expected.expected_current_deployment_digest,
        expected.expected_current_deployment_generation,
    )?;
    if expected.expected_resolution_digest != resolution.resolution_digest
        || expected.expected_state != resolution.state
        || expected.expected_gate_status != context.gate.status
        || expected.expected_gate_epoch != context.gate.epoch
        || expected.expected_target_nonce != context.config.target_nonce
        || expected.expected_freeze_observation_digest
            != resolution.emergency_freeze_observation_digest
        || expected.expected_programdata_observation_digest != resolution.observation_digest
        || expected.expected_observation_generation != resolution.observation_generation
        || expected.expected_checkpoint_digest != resolution.emergency_checkpoint_digest
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn load_accepted_emergency_checkpoint(
    program_id: &Pubkey,
    checkpoint_info: &AccountInfo<'_>,
    resolution_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    deployment_info: &AccountInfo<'_>,
    observation_info: &AccountInfo<'_>,
    council: &GovernanceCouncilSetV1,
    context: &LifecycleContext,
    resolution: &EmergencyFreezeResolutionV2,
) -> Result<Box<StateCheckpointV2>, ProgramError> {
    let checkpoint = load_fixed_controller_account::<StateCheckpointV2>(
        program_id,
        checkpoint_info,
        StateCheckpointV2::LEN,
    )?;
    validate_state_checkpoint_digest_v2(&checkpoint)?;
    if derive_emergency_checkpoint_v2_pda(program_id, resolution_info.key)
        != (*checkpoint_info.key, checkpoint.bump)
        || resolution.emergency_checkpoint != *checkpoint_info.key
        || resolution.emergency_checkpoint_digest != checkpoint.checkpoint_digest
        || checkpoint.phase != StateCheckpointPhaseV1::Emergency
        || checkpoint.controller_config != *config_info.key
        || checkpoint.proposal != Pubkey::default()
        || checkpoint.emergency_resolution != *resolution_info.key
        || checkpoint.subject_digest != resolution.resolution_digest
        || checkpoint.target_program != context.config.target_program
        || checkpoint.target_programdata != context.config.target_programdata
        || checkpoint.capacity_policy != *capacity_info.key
        || checkpoint.capacity_policy_digest != context.capacity.policy_digest
        || checkpoint.current_deployment_state != *deployment_info.key
        || checkpoint.current_deployment_digest != context.deployment.deployment_digest
        || checkpoint.current_deployment_generation != context.deployment.deployment_generation
        || checkpoint.programdata_observation != *observation_info.key
        || checkpoint.observation_purpose != ProgramDataObservationPurposeV1::EmergencyResolution
        || checkpoint.observation_generation != resolution.observation_generation
        || checkpoint.observation_subject_digest != resolution.observation_subject_digest
        || checkpoint.observation_root != resolution.observation_root
        || checkpoint.observation_digest != resolution.observation_digest
        || checkpoint.observation_finalized_slot != resolution.observation_finalized_slot
        || checkpoint.gate_epoch != context.gate.epoch
        || checkpoint.target_programdata_slot != resolution.observed_deployed_slot
        || checkpoint.actual_capacity != resolution.actual_capacity
        || checkpoint.observed_authority != resolution.observed_authority
        || checkpoint.approval_council_version != council.version
        || checkpoint.approval_council_hash != council.set_hash
        || checkpoint.approval_count != RELEASE1_APPROVAL_THRESHOLD
        || !checkpoint.accepted
        || checkpoint.forbidden_drift_count != 0
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(checkpoint)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_emergency_evidence(
    program_id: &Pubkey,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    deployment_info: &AccountInfo<'_>,
    freeze_observation_info: &AccountInfo<'_>,
    programdata_observation_info: &AccountInfo<'_>,
    checkpoint_info: &AccountInfo<'_>,
    resolution_info: &AccountInfo<'_>,
    context: &LifecycleContext,
    council: &GovernanceCouncilSetV1,
    resolution: &EmergencyFreezeResolutionV2,
    slot: u64,
) -> Result<EmergencyEvidenceV2, ProgramError> {
    let freeze_observation = load_emergency_freeze_observation(
        program_id,
        freeze_observation_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        context,
    )?;
    if resolution.emergency_freeze_observation != *freeze_observation_info.key
        || resolution.emergency_freeze_observation_digest != freeze_observation.observation_digest
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let observation_expectation = trusted_observation_expectation(&context.deployment);
    let observation = load_fresh_programdata_observation(
        program_id,
        programdata_observation_info,
        config_info,
        gate_info,
        capacity_info,
        freeze_observation_info.key,
        &context.config,
        &context.gate,
        &context.capacity,
        &observation_expectation,
        ProgramDataObservationPurposeV1::EmergencyResolution,
        &resolution.observation_subject_digest,
        resolution.observation_generation,
        &resolution.observation_digest,
        context.deployment.artifact_length,
        slot,
    )?;
    if resolution.programdata_observation != *programdata_observation_info.key
        || resolution.observation_subject_digest != observation.subject_digest
        || resolution.observation_root != observation.final_raw_merkle_root
        || resolution.observation_finalized_slot != observation.finalized_slot
        || resolution.observed_deployed_slot != observation.deployed_slot
        || resolution.observed_raw_data_length != observation.raw_data_length
        || resolution.actual_capacity != observation.actual_capacity
        || resolution.observed_authority != observation.upgrade_authority
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    let checkpoint = load_accepted_emergency_checkpoint(
        program_id,
        checkpoint_info,
        resolution_info,
        config_info,
        capacity_info,
        deployment_info,
        programdata_observation_info,
        council,
        context,
        resolution,
    )?;
    Ok((freeze_observation, observation, checkpoint))
}
