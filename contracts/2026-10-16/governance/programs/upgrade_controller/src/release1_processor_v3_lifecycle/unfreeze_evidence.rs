use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn load_verified_programdata_for_unfreeze_v2(
    program_id: &Pubkey,
    verification_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    deployment_info: &AccountInfo<'_>,
    context: &LifecycleContext,
    proposal: &UpgradeProposalV3,
) -> Result<Box<ProgramDataVerificationV2>, ProgramError> {
    let verification = load_fixed_controller_account::<ProgramDataVerificationV2>(
        program_id,
        verification_info,
        ProgramDataVerificationV2::LEN,
    )?;
    validate_programdata_verification_digest_v2(&verification)?;
    if derive_programdata_check_pda(program_id, proposal_info.key)
        != (*verification_info.key, verification.bump)
        || proposal.programdata_verification != *verification_info.key
        || verification.status != ProgramDataVerificationStatusV2::Verified
        || !verification.zero_tail_verified
        || verification.finalized_slot == 0
        || verification.finalized_slot != proposal.programdata_verified_slot
        || verification.controller_config != *config_info.key
        || verification.proposal != *proposal_info.key
        || verification.proposal_digest != proposal.proposal_digest
        || verification.protocol_gate != *gate_info.key
        || verification.freeze_gate_epoch != context.gate.epoch
        || verification.target_nonce != context.config.target_nonce
        || verification.capacity_policy != *capacity_info.key
        || verification.capacity_policy_digest != context.capacity.policy_digest
        || verification.current_deployment_state != *deployment_info.key
        || verification.target_program != context.config.target_program
        || verification.target_programdata != context.config.target_programdata
        || verification.upgradeable_loader != context.config.upgradeable_loader
        || verification.controller_authority != context.config.authority_pda
        || verification.artifact_length != proposal.artifact_length
        || verification.artifact_sha256 != proposal.artifact_sha256
        || verification.artifact_merkle_root != proposal.artifact_chunk_merkle_root
        || verification.artifact_scheme_id != proposal.artifact_scheme_id
        || verification.minimum_required_capacity != proposal.minimum_required_capacity
        || verification.maximum_supported_raw_programdata_length
            != context.capacity.maximum_raw_programdata_length
        || verification.observation_scheme_id != context.capacity.observation_scheme_id
        || verification.observed_deployed_slot != proposal.upgrade_executed_slot
        || verification.observed_authority != OptionalPubkeyV1::some(context.config.authority_pda)?
        || verification.actual_capacity < proposal.minimum_required_capacity
        || verification.observation_finalized_slot > verification.finalized_slot
        || verification.current_deployment_digest != context.deployment.deployment_digest
        || verification.current_deployment_generation != context.deployment.deployment_generation
        || context.deployment.deployment_digest != proposal.current_deployment_digest
        || context.deployment.deployment_generation != proposal.current_deployment_generation
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(verification)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_accepted_poststate_for_unfreeze_v2(
    program_id: &Pubkey,
    checkpoint_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    deployment_info: &AccountInfo<'_>,
    context: &LifecycleContext,
    proposal: &UpgradeProposalV3,
    verification: &ProgramDataVerificationV2,
) -> Result<Box<StateCheckpointV2>, ProgramError> {
    let checkpoint = load_fixed_controller_account::<StateCheckpointV2>(
        program_id,
        checkpoint_info,
        StateCheckpointV2::LEN,
    )?;
    validate_state_checkpoint_digest_v2(&checkpoint)?;
    if derive_checkpoint_pda(program_id, proposal_info.key, CheckpointPhaseV1::Poststate)
        != (*checkpoint_info.key, checkpoint.bump)
        || proposal.required_poststate_checkpoint != *checkpoint_info.key
        || checkpoint.phase != StateCheckpointPhaseV1::Poststate
        || checkpoint.controller_config != *config_info.key
        || checkpoint.proposal != *proposal_info.key
        || checkpoint.emergency_resolution != Pubkey::default()
        || checkpoint.subject_digest != proposal.proposal_digest
        || checkpoint.target_program != context.config.target_program
        || checkpoint.target_programdata != context.config.target_programdata
        || checkpoint.capacity_policy != *capacity_info.key
        || checkpoint.capacity_policy_digest != context.capacity.policy_digest
        || checkpoint.current_deployment_state != *deployment_info.key
        || checkpoint.current_deployment_digest != context.deployment.deployment_digest
        || checkpoint.current_deployment_generation != context.deployment.deployment_generation
        || checkpoint.observation_scheme_id != context.capacity.observation_scheme_id
        || checkpoint.programdata_observation != verification.programdata_observation
        || checkpoint.observation_purpose != verification.observation_purpose
        || checkpoint.observation_generation != verification.observation_generation
        || checkpoint.observation_subject_digest != verification.observation_subject_digest
        || checkpoint.observation_root != verification.observation_root
        || checkpoint.observation_digest != verification.observation_digest
        || checkpoint.observation_finalized_slot != verification.observation_finalized_slot
        || checkpoint.gate_epoch != context.gate.epoch
        || checkpoint.target_programdata_slot != verification.observed_deployed_slot
        || checkpoint.artifact_length != proposal.artifact_length
        || checkpoint.artifact_sha256 != proposal.artifact_sha256
        || checkpoint.artifact_merkle_root != proposal.artifact_chunk_merkle_root
        || checkpoint.artifact_scheme_id != proposal.artifact_scheme_id
        || checkpoint.minimum_required_capacity != proposal.minimum_required_capacity
        || checkpoint.observed_raw_data_length != verification.observed_raw_data_length
        || checkpoint.actual_capacity != verification.actual_capacity
        || checkpoint.observed_authority != verification.observed_authority
        || checkpoint.schema_identifier != proposal.checkpoint_schema_id
        || checkpoint.approval_count != RELEASE1_APPROVAL_THRESHOLD
        || !checkpoint.accepted
        || checkpoint.forbidden_drift_count != 0
        || checkpoint.finalized_slot == 0
        || checkpoint.finalized_slot != proposal.poststate_accepted_slot
        || checkpoint.finalized_slot < verification.finalized_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(checkpoint)
}

pub(super) fn require_unfreeze_gate_binding(
    proposal_key: &Pubkey,
    proposal: &UpgradeProposalV3,
    context: &LifecycleContext,
) -> ProgramResult {
    let consumed_nonce = proposal
        .target_nonce
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if context.gate.status != GateStatusV1::FrozenForUpgrade
        || context.gate.active_proposal != *proposal_key
        || context.gate.epoch != proposal.freeze_gate_epoch
        || context.gate.freeze_slot != proposal.frozen_slot
        || context.config.target_nonce != consumed_nonce
    {
        return Err(GovernanceError::InvalidProposalEpoch.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn check_unfreeze_guard_v2(
    expected: &UnfreezeGuardV2,
    proposal: &UpgradeProposalV3,
    checkpoint: &StateCheckpointV2,
    verification: &ProgramDataVerificationV2,
    context: &LifecycleContext,
    council: &GovernanceCouncilSetV1,
) -> ProgramResult {
    if expected.expected_proposal_digest != proposal.proposal_digest
        || expected.expected_checkpoint_digest != checkpoint.checkpoint_digest
        || expected.expected_checkpoint_generation != checkpoint.checkpoint_generation
        || expected.expected_verification_digest != verification.verification_digest
        || expected.expected_verification_generation != verification.verification_generation
        || expected.expected_original_council_version != proposal.creation_council_version
        || expected.expected_original_council_hash != proposal.creation_council_hash
        || expected.expected_current_council_version != council.version
        || expected.expected_current_council_hash != council.set_hash
        || expected.expected_gate_epoch != context.gate.epoch
        || expected.expected_target_nonce != context.config.target_nonce
        || expected.expected_current_deployment_digest != context.deployment.deployment_digest
        || expected.expected_current_deployment_generation
            != context.deployment.deployment_generation
        || expected.expected_artifact_sha256 != proposal.artifact_sha256
        || expected.expected_artifact_merkle_root != proposal.artifact_chunk_merkle_root
        || expected.expected_actual_capacity != verification.actual_capacity
        || expected.expected_approval_bitset != proposal.unfreeze_approval_bitset
        || expected.expected_approval_count != proposal.unfreeze_approval_count
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn validate_live_verified_programdata_v2(
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    authority: &AccountInfo<'_>,
    loader: &AccountInfo<'_>,
    context: &LifecycleContext,
    verification: &ProgramDataVerificationV2,
) -> ProgramResult {
    if *loader.key != context.config.upgradeable_loader
        || *authority.key != context.config.authority_pda
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let runtime = read_runtime_programdata_header(
        target_program,
        target_programdata,
        &context.config,
        &context.capacity,
    )?;
    if runtime.deployed_slot != verification.observed_deployed_slot
        || runtime.raw_data_length != verification.observed_raw_data_length
        || runtime.capacity != verification.actual_capacity
        || runtime.authority != Some(context.config.authority_pda)
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(())
}

pub(super) fn classify_unfreeze_accumulator_v2(
    state: ProposalStateV2,
    stored_version: u64,
    stored_hash: &[u8; 32],
    count: u8,
    current_version: u64,
    current_hash: &[u8; 32],
) -> GovernanceResult<UnfreezeAccumulatorActionV2> {
    if count != 0 && (stored_version != current_version || stored_hash != current_hash) {
        return Ok(UnfreezeAccumulatorActionV2::ResetForCurrentCouncil);
    }
    if state == ProposalStateV2::UnfreezeApproved {
        return Err(GovernanceError::DuplicateApproval);
    }
    Ok(UnfreezeAccumulatorActionV2::Continue)
}

pub(super) fn prepare_unfreeze_accumulator_v2(
    proposal: &mut UpgradeProposalV3,
    council: &GovernanceCouncilSetV1,
) -> ProgramResult {
    if classify_unfreeze_accumulator_v2(
        proposal.state,
        proposal.unfreeze_council_version,
        &proposal.unfreeze_council_hash,
        proposal.unfreeze_approval_count,
        council.version,
        &council.set_hash,
    )? == UnfreezeAccumulatorActionV2::ResetForCurrentCouncil
    {
        proposal.unfreeze_council_version = 0;
        proposal.unfreeze_council_hash = [0; 32];
        proposal.unfreeze_approval_bitset = 0;
        proposal.unfreeze_approval_count = 0;
        proposal.unfreeze_approved_slot = 0;
        proposal.state = ProposalStateV2::PoststateAccepted;
    }
    Ok(())
}

pub(super) fn require_current_unfreeze_quorum_v2(
    proposal: &UpgradeProposalV3,
    council: &GovernanceCouncilSetV1,
    slot: u64,
) -> ProgramResult {
    if proposal.unfreeze_council_version != council.version
        || proposal.unfreeze_council_hash != council.set_hash
        || proposal.unfreeze_approval_bitset & !VALID_APPROVAL_MASK != 0
        || proposal.unfreeze_approval_bitset.count_ones() as u8 != proposal.unfreeze_approval_count
        || proposal.unfreeze_approval_count != RELEASE1_APPROVAL_THRESHOLD
        || !council.active_at(slot)
    {
        return Err(GovernanceError::QuorumNotSatisfied.into());
    }
    for (index, seat) in council.seats.iter().enumerate() {
        if proposal.unfreeze_approval_bitset & (1u8 << index) != 0 && !seat.term_covers(slot) {
            return Err(GovernanceError::InactiveCouncilSeat.into());
        }
    }
    Ok(())
}
