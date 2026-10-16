use super::*;

pub(super) fn load_config(
    program_id: &Pubkey,
    config_info: &AccountInfo<'_>,
) -> Result<Box<ControllerConfigV1>, ProgramError> {
    let config = load_fixed_controller_account::<ControllerConfigV1>(
        program_id,
        config_info,
        ControllerConfigV1::LEN,
    )?;
    config.validate_static()?;
    let expected = derive_controller_config_pda(program_id, &config.target_program);
    if expected.0 != *config_info.key
        || expected.1 != config.bump
        || config.upgradeable_loader != UPGRADEABLE_LOADER_ID
        || derive_upgradeable_programdata_address(&config.target_program).0
            != config.target_programdata
        || derive_authority_pda(program_id, &config.target_program).0 != config.authority_pda
        || derive_gate_pda(program_id, &config.target_program).0 != config.gate_pda
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    Ok(config)
}

pub(super) fn load_policy(
    program_id: &Pubkey,
    policy_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<GovernancePolicyV1>, ProgramError> {
    let policy = load_fixed_controller_account::<GovernancePolicyV1>(
        program_id,
        policy_info,
        GovernancePolicyV1::LEN,
    )?;
    validate_policy_against_config(&policy, config)?;
    let expected = derive_policy_pda(program_id, &config.target_program, policy.version);
    if expected.0 != *policy_info.key
        || expected.1 != policy.bump
        || policy.controller_config != *config_info.key
        || policy.version != config.current_policy_version
        || policy.target_program != config.target_program
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(policy)
}

pub(super) fn load_current_council(
    program_id: &Pubkey,
    council_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
) -> Result<Box<GovernanceCouncilSetV1>, ProgramError> {
    load_pinned_council(
        program_id,
        council_info,
        config_info,
        config,
        policy,
        config.current_council_version,
        &[0; 32],
    )
}

pub(super) fn load_pinned_council(
    program_id: &Pubkey,
    council_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
    expected_version: u64,
    expected_hash: &[u8; 32],
) -> Result<Box<GovernanceCouncilSetV1>, ProgramError> {
    let council = load_fixed_controller_account::<GovernanceCouncilSetV1>(
        program_id,
        council_info,
        GovernanceCouncilSetV1::LEN,
    )?;
    validate_council_set(&council, policy)?;
    validate_council_guardian_separation(&council, &config.guardian)?;
    let expected = derive_council_pda(program_id, &config.target_program, council.version);
    if expected.0 != *council_info.key
        || expected.1 != council.bump
        || council.controller_config != *config_info.key
        || council.target_program != config.target_program
        || council.version != expected_version
        || (*expected_hash != [0; 32] && council.set_hash != *expected_hash)
    {
        return Err(GovernanceError::StaleCouncilVersion.into());
    }
    Ok(council)
}

pub(super) fn load_gate(
    program_id: &Pubkey,
    gate_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<ProtocolGateV1>, ProgramError> {
    let gate = load_fixed_controller_account::<ProtocolGateV1>(
        program_id,
        gate_info,
        ProtocolGateV1::LEN,
    )?;
    gate.validate_static()?;
    let expected = derive_gate_pda(program_id, &config.target_program);
    if expected.0 != *gate_info.key
        || expected.1 != gate.bump
        || *gate_info.key != config.gate_pda
        || gate.controller_config != *config_info.key
        || gate.target_program != config.target_program
        || gate.target_programdata != config.target_programdata
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(gate)
}

pub(super) fn load_proposal(
    program_id: &Pubkey,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<UpgradeProposalV2>, ProgramError> {
    let proposal = load_fixed_controller_account::<UpgradeProposalV2>(
        program_id,
        proposal_info,
        UpgradeProposalV2::LEN,
    )?;
    validate_proposal_digest_v2(&proposal)?;
    let expected = derive_proposal_pda(program_id, &config.target_program, proposal.proposal_id);
    if expected.0 != *proposal_info.key
        || expected.1 != proposal.bump
        || proposal.controller_program != *program_id
        || proposal.controller_config != *config_info.key
        || proposal.protocol_gate != config.gate_pda
        || proposal.cluster_domain != config.cluster_domain
        || proposal.target_program != config.target_program
        || proposal.target_programdata != config.target_programdata
        || proposal.upgradeable_loader != config.upgradeable_loader
        || proposal.authority_pda != config.authority_pda
        || proposal.canonical_spill_treasury != config.canonical_spill_treasury
        || proposal.policy_version != config.current_policy_version
        || proposal.proposal_id >= config.next_proposal_id
        || proposal.buffer_verification != derive_buffer_check_pda(program_id, proposal_info.key).0
        || proposal.programdata_verification
            != derive_programdata_check_pda(program_id, proposal_info.key).0
        || proposal.prestate_checkpoint
            != derive_checkpoint_pda(program_id, proposal_info.key, CheckpointPhaseV1::Prestate).0
        || proposal.required_poststate_checkpoint
            != derive_checkpoint_pda(program_id, proposal_info.key, CheckpointPhaseV1::Poststate).0
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    verify_exact_proposal_timing(&proposal, config)?;
    Ok(proposal)
}

pub(super) fn load_resolution(
    program_id: &Pubkey,
    resolution_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<EmergencyFreezeResolutionV1>, ProgramError> {
    let resolution = load_fixed_controller_account::<EmergencyFreezeResolutionV1>(
        program_id,
        resolution_info,
        EmergencyFreezeResolutionV1::LEN,
    )?;
    validate_emergency_resolution_digest_v1(&resolution)?;
    let expected = derive_emergency_resolution_pda(
        program_id,
        &config.target_program,
        resolution.frozen_epoch,
    );
    if expected.0 != *resolution_info.key
        || expected.1 != resolution.bump
        || resolution.controller_config != *config_info.key
        || resolution.protocol_gate != *gate_info.key
        || resolution.target_program != config.target_program
        || resolution.target_programdata != config.target_programdata
        || resolution.emergency_checkpoint
            != derive_emergency_checkpoint_pda(
                program_id,
                &config.target_program,
                resolution.frozen_epoch,
            )
            .0
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(resolution)
}

pub(super) fn load_emergency_observation(
    program_id: &Pubkey,
    observation_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
) -> Result<Box<EmergencyFreezeObservationV1>, ProgramError> {
    let observation = load_fixed_controller_account::<EmergencyFreezeObservationV1>(
        program_id,
        observation_info,
        EmergencyFreezeObservationV1::LEN,
    )?;
    validate_emergency_freeze_observation_digest_v1(&observation)?;
    let expected =
        derive_emergency_freeze_observation_pda(program_id, &config.target_program, gate.epoch);
    if expected.0 != *observation_info.key
        || expected.1 != observation.bump
        || observation.controller_program != *program_id
        || observation.controller_config != *config_info.key
        || observation.protocol_gate != *gate_info.key
        || observation.target_program != config.target_program
        || observation.target_programdata != config.target_programdata
        || observation.upgradeable_loader != config.upgradeable_loader
        || observation.controller_authority != config.authority_pda
        || observation.frozen_epoch != gate.epoch
        || observation.freeze_slot != gate.freeze_slot
        || observation.freeze_reason_code != gate.freeze_reason_code
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(observation)
}
