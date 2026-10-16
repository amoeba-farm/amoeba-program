use super::*;

pub(super) fn require_program_ids(
    loader: &AccountInfo<'_>,
    system: Option<&AccountInfo<'_>>,
    rent: Option<&AccountInfo<'_>>,
    clock: Option<&AccountInfo<'_>>,
    instructions_info: Option<&AccountInfo<'_>>,
) -> ProgramResult {
    if *loader.key != UPGRADEABLE_LOADER_ID
        || system.is_some_and(|account| *account.key != system_program::ID)
        || rent.is_some_and(|account| *account.key != sysvar_ids::rent::ID)
        || clock.is_some_and(|account| *account.key != sysvar_ids::clock::ID)
        || instructions_info.is_some_and(|account| *account.key != sysvar_ids::instructions::ID)
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

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
    if derive_controller_config_pda(program_id, &config.target_program)
        != (*config_info.key, config.bump)
        || derive_authority_pda(program_id, &config.target_program).0 != config.authority_pda
        || derive_gate_pda(program_id, &config.target_program).0 != config.gate_pda
        || derive_upgradeable_programdata_address(&config.target_program).0
            != config.target_programdata
        || config.upgradeable_loader != UPGRADEABLE_LOADER_ID
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
    if derive_policy_pda(program_id, &config.target_program, policy.version)
        != (*policy_info.key, policy.bump)
        || policy.controller_config != *config_info.key
        || policy.target_program != config.target_program
        || policy.version != config.current_policy_version
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(policy)
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
    if derive_gate_pda(program_id, &config.target_program) != (*gate_info.key, gate.bump)
        || *gate_info.key != config.gate_pda
        || gate.controller_config != *config_info.key
        || gate.target_program != config.target_program
        || gate.target_programdata != config.target_programdata
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(gate)
}

pub(super) fn load_capacity_policy(
    program_id: &Pubkey,
    capacity_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<ProgramDataCapacityPolicyV1>, ProgramError> {
    let capacity = load_fixed_controller_account::<ProgramDataCapacityPolicyV1>(
        program_id,
        capacity_info,
        ProgramDataCapacityPolicyV1::LEN,
    )?;
    validate_capacity_policy_digest_v1(&capacity)?;
    if derive_capacity_policy_pda(program_id, &config.target_program)
        != (*capacity_info.key, capacity.bump)
        || capacity.controller_program != *program_id
        || capacity.controller_config != *config_info.key
        || capacity.target_program != config.target_program
        || capacity.target_programdata != config.target_programdata
        || capacity.upgradeable_loader != config.upgradeable_loader
    {
        return Err(GovernanceError::CapacityPolicyMismatch.into());
    }
    Ok(capacity)
}

pub(super) fn load_current_deployment(
    program_id: &Pubkey,
    deployment_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    capacity: &ProgramDataCapacityPolicyV1,
) -> Result<Box<CurrentDeploymentStateV1>, ProgramError> {
    let deployment = load_fixed_controller_account::<CurrentDeploymentStateV1>(
        program_id,
        deployment_info,
        CurrentDeploymentStateV1::LEN,
    )?;
    validate_current_deployment_digest_v1(&deployment)?;
    if derive_current_deployment_state_pda(program_id, &config.target_program)
        != (*deployment_info.key, deployment.bump)
        || deployment.controller_program != *program_id
        || deployment.controller_config != *config_info.key
        || deployment.capacity_policy != *capacity_info.key
        || deployment.capacity_policy_digest != capacity.policy_digest
        || deployment.target_program != config.target_program
        || deployment.target_programdata != config.target_programdata
        || deployment.upgradeable_loader != config.upgradeable_loader
        || deployment.controller_authority != config.authority_pda
        || deployment.installed_authority != config.authority_pda
        || deployment.actual_programdata_capacity > capacity.maximum_payload_capacity
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(deployment)
}

pub(super) fn load_proposal(
    program_id: &Pubkey,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<UpgradeProposalV3>, ProgramError> {
    let proposal = load_upgrade_proposal_v3(program_id, proposal_info)?;
    validate_upgrade_proposal_digest_v3(&proposal)?;
    if derive_proposal_pda(program_id, &config.target_program, proposal.proposal_id)
        != (*proposal_info.key, proposal.bump)
        || proposal.controller_program != *program_id
        || proposal.controller_config != *config_info.key
        || proposal.protocol_gate != config.gate_pda
        || proposal.cluster_domain != config.cluster_domain
        || proposal.target_program != config.target_program
        || proposal.target_programdata != config.target_programdata
        || proposal.upgradeable_loader != config.upgradeable_loader
        || proposal.authority_pda != config.authority_pda
        || proposal.canonical_spill_treasury != config.canonical_spill_treasury
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

pub(super) fn validate_proposal_guard(
    expected: &ProposalGuardV3,
    proposal: &UpgradeProposalV3,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    proposal_key: &Pubkey,
) -> ProgramResult {
    if expected.expected_proposal_digest != proposal.proposal_digest
        || expected.expected_state != proposal.state
        || expected.expected_gate_status != gate.status
        || expected.expected_gate_epoch != gate.epoch
        || expected.expected_target_nonce != config.target_nonce
        || expected.expected_capacity_policy_digest != proposal.capacity_policy_digest
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let frozen_lifecycle = matches!(
        proposal.state,
        ProposalStateV2::Frozen
            | ProposalStateV2::Extended
            | ProposalStateV2::UpgradeExecuted
            | ProposalStateV2::ProgramDataVerified
            | ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
    );
    if frozen_lifecycle {
        let consumed = proposal
            .target_nonce
            .checked_add(1)
            .ok_or(GovernanceError::ArithmeticOverflow)?;
        if gate.status != GateStatusV1::FrozenForUpgrade
            || gate.active_proposal != *proposal_key
            || gate.epoch != proposal.freeze_gate_epoch
            || config.target_nonce != consumed
            || gate.freeze_slot != proposal.frozen_slot
            || gate.freeze_reason_code == 0
        {
            return Err(GovernanceError::InvalidProposalEpoch.into());
        }
    } else if !matches!(
        proposal.state,
        ProposalStateV2::Cancelled | ProposalStateV2::Expired | ProposalStateV2::Retired
    ) && (proposal.target_nonce != config.target_nonce
        || proposal.creation_gate_epoch != gate.epoch
        || proposal.creation_gate_status != gate.status)
    {
        return Err(GovernanceError::InvalidProposalEpoch.into());
    }
    Ok(())
}

pub(super) fn require_deployment_guard(
    expected: &ProposalGuardV3,
    proposal: &UpgradeProposalV3,
    deployment: &CurrentDeploymentStateV1,
) -> ProgramResult {
    if expected.expected_current_deployment_digest != deployment.deployment_digest
        || expected.expected_current_deployment_generation != deployment.deployment_generation
        || proposal.current_deployment_state == Pubkey::default()
        || deployment.deployment_generation < proposal.current_deployment_generation
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

fn derive_exact_timing(
    config: &ControllerConfigV1,
    class: ProposalClassV1,
    creation_slot: u64,
) -> Result<(u64, u64, u64, u64), ProgramError> {
    let delay = match class {
        ProposalClassV1::EmergencyRollback => config.rollback_delay_slots,
        ProposalClassV1::RoutineUpgrade => config.routine_delay_slots,
        ProposalClassV1::EconomicChange | ProposalClassV1::ConstitutionalChange => {
            config.major_delay_slots
        }
        ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability => {
            return Err(GovernanceError::UnsupportedProposalClass.into());
        }
    };
    let review_start = creation_slot
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let review_end = review_start
        .checked_add(config.council_review_slots())
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let not_before = review_end
        .checked_add(delay)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let expiry = creation_slot
        .checked_add(config.proposal_expiry_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if not_before >= expiry {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok((review_start, review_end, not_before, expiry))
}

fn verify_exact_proposal_timing(
    proposal: &UpgradeProposalV3,
    config: &ControllerConfigV1,
) -> ProgramResult {
    if derive_exact_timing(config, proposal.proposal_class, proposal.creation_slot)?
        != (
            proposal.review_start_slot,
            proposal.review_end_slot,
            proposal.not_before_slot,
            proposal.expiry_slot,
        )
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(())
}

pub(super) fn current_preexpiry_slot(proposal: &UpgradeProposalV3) -> Result<u64, ProgramError> {
    let slot = Clock::get()?.slot;
    if slot == 0 || slot >= proposal.expiry_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(slot)
}

pub(super) fn current_frozen_slot(proposal: &UpgradeProposalV3) -> Result<u64, ProgramError> {
    let slot = Clock::get()?.slot;
    if slot == 0 || slot < proposal.frozen_slot || slot >= proposal.expiry_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(slot)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_accepted_prestate_at_epoch(
    program_id: &Pubkey,
    checkpoint_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    deployment_info: &AccountInfo<'_>,
    observation_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    capacity: &ProgramDataCapacityPolicyV1,
    proposal: &UpgradeProposalV3,
    expected_gate_epoch: u64,
    deployment: &CurrentDeploymentStateV1,
    expected_digest: &[u8; 32],
    expected_generation: u64,
) -> Result<Box<StateCheckpointV2>, ProgramError> {
    let checkpoint = load_fixed_controller_account::<StateCheckpointV2>(
        program_id,
        checkpoint_info,
        StateCheckpointV2::LEN,
    )?;
    validate_state_checkpoint_digest_v2(&checkpoint)?;
    let observation = load_observation_any_status(
        program_id,
        observation_info,
        config_info,
        capacity_info,
        proposal_info,
        config,
        capacity,
    )?;
    if observation.status != ProgramDataObservationStatusV1::Finalized {
        return Err(GovernanceError::IncompleteProgramDataObservation.into());
    }
    if derive_checkpoint_pda(program_id, proposal_info.key, CheckpointPhaseV1::Prestate)
        != (*checkpoint_info.key, checkpoint.bump)
        || proposal.prestate_checkpoint != *checkpoint_info.key
        || checkpoint.phase != StateCheckpointPhaseV1::Prestate
        || checkpoint.controller_config != *config_info.key
        || checkpoint.proposal != *proposal_info.key
        || checkpoint.emergency_resolution != Pubkey::default()
        || checkpoint.subject_digest != proposal.proposal_digest
        || checkpoint.target_program != proposal.target_program
        || checkpoint.target_programdata != proposal.target_programdata
        || checkpoint.capacity_policy != *capacity_info.key
        || checkpoint.capacity_policy_digest != proposal.capacity_policy_digest
        || checkpoint.current_deployment_state != *deployment_info.key
        || checkpoint.current_deployment_digest != proposal.current_deployment_digest
        || checkpoint.current_deployment_generation != proposal.current_deployment_generation
        || checkpoint.checkpoint_generation != expected_generation
        || checkpoint.programdata_observation != *observation_info.key
        || checkpoint.observation_generation != observation.generation
        || checkpoint.observation_subject_digest != observation.subject_digest
        || checkpoint.observation_root != observation.final_raw_merkle_root
        || checkpoint.observation_digest != observation.observation_digest
        || checkpoint.observation_finalized_slot != observation.finalized_slot
        || checkpoint.gate_epoch != expected_gate_epoch
        || checkpoint.target_programdata_slot != observation.deployed_slot
        || checkpoint.artifact_length != observation.expected_artifact_length
        || checkpoint.artifact_sha256 != observation.expected_artifact_sha256
        || checkpoint.artifact_merkle_root != observation.expected_artifact_merkle_root
        || checkpoint.artifact_scheme_id != observation.expected_artifact_scheme_id
        || checkpoint.minimum_required_capacity != observation.minimum_required_capacity
        || checkpoint.observed_raw_data_length != observation.raw_data_length
        || checkpoint.actual_capacity != observation.actual_capacity
        || checkpoint.observed_authority != observation.upgrade_authority
        || checkpoint.checkpoint_digest != *expected_digest
        || !checkpoint.accepted
        || checkpoint.approval_count < 3
        || checkpoint.forbidden_drift_count != 0
        || deployment.deployment_generation < proposal.current_deployment_generation
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(checkpoint)
}

pub(super) fn commit_one_fixed_account(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    bytes: &[u8],
    expected_len: usize,
) -> ProgramResult {
    if account.owner != program_id {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let mut data = account.try_borrow_mut_data()?;
    if data.len() != expected_len || bytes.len() != expected_len {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    data.copy_from_slice(bytes);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn commit_two_fixed_accounts(
    program_id: &Pubkey,
    first: &AccountInfo<'_>,
    first_bytes: &[u8],
    first_len: usize,
    second: &AccountInfo<'_>,
    second_bytes: &[u8],
    second_len: usize,
) -> ProgramResult {
    if first.owner != program_id || second.owner != program_id {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let mut first_data = first.try_borrow_mut_data()?;
    let mut second_data = second.try_borrow_mut_data()?;
    if first_data.len() != first_len
        || second_data.len() != second_len
        || first_bytes.len() != first_len
        || second_bytes.len() != second_len
    {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    first_data.copy_from_slice(first_bytes);
    second_data.copy_from_slice(second_bytes);
    Ok(())
}
