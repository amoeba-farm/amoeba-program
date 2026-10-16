use super::*;

fn phase_to_pda_phase(phase: StateCheckpointPhaseV1) -> GovernanceResult<CheckpointPhaseV1> {
    match phase {
        StateCheckpointPhaseV1::Prestate => Ok(CheckpointPhaseV1::Prestate),
        StateCheckpointPhaseV1::Poststate => Ok(CheckpointPhaseV1::Poststate),
        StateCheckpointPhaseV1::Emergency => Err(GovernanceError::InvalidRelease1Account),
    }
}

fn validate_proposal_subject(
    context: &CheckpointSubjectContext<'_, '_>,
) -> Result<CheckpointBinding, ProgramError> {
    let program_id = context.program_id;
    let subject_info = context.subject_info;
    let checkpoint_info = context.checkpoint_info;
    let config_key = context.config_key;
    let config = context.config;
    let gate_key = context.gate_key;
    let gate = context.gate;
    let candidate = context.candidate;
    let proposal = load_fixed_controller_account::<UpgradeProposalV2>(
        program_id,
        subject_info,
        UpgradeProposalV2::LEN,
    )?;
    validate_proposal_digest_v2(&proposal)?;
    let (proposal_pda, bump) =
        derive_proposal_pda(program_id, &config.target_program, proposal.proposal_id);
    if *subject_info.key != proposal_pda
        || proposal.bump != bump
        || proposal.controller_program != *program_id
        || proposal.controller_config != *config_key
        || proposal.protocol_gate != *gate_key
        || proposal.target_program != config.target_program
        || proposal.target_programdata != config.target_programdata
        || proposal.upgradeable_loader != config.upgradeable_loader
        || proposal.authority_pda != config.authority_pda
        || proposal.target_nonce.checked_add(1) != Some(config.target_nonce)
        || proposal.freeze_gate_epoch != gate.epoch
        || gate.status != GateStatusV1::FrozenForUpgrade
        || gate.active_proposal != *subject_info.key
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let (required_state, required_subject_state, required_checkpoint) = match candidate.phase {
        StateCheckpointPhaseV1::Prestate => (
            ProposalStateV2::Frozen,
            CheckpointSubjectStateV1::ProposalFrozen,
            proposal.prestate_checkpoint,
        ),
        StateCheckpointPhaseV1::Poststate => (
            ProposalStateV2::ProgramDataVerified,
            CheckpointSubjectStateV1::ProposalProgramDataVerified,
            proposal.required_poststate_checkpoint,
        ),
        StateCheckpointPhaseV1::Emergency => {
            return Err(GovernanceError::InvalidStateTransition.into())
        }
    };
    if proposal.state != required_state
        || candidate.expected_subject_state != required_subject_state
        || candidate.expected_subject_digest != proposal.proposal_digest
        || candidate.expected_gate_status != GateStatusV1::FrozenForUpgrade
        || candidate.expected_gate_epoch != gate.epoch
        || candidate.schema_identifier != proposal.checkpoint_schema_id
        || candidate.finalized_observation_slot < gate.freeze_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let pda_phase = phase_to_pda_phase(candidate.phase)?;
    let (checkpoint, checkpoint_bump) =
        derive_checkpoint_pda(program_id, subject_info.key, pda_phase);
    if *checkpoint_info.key != checkpoint || checkpoint != required_checkpoint {
        return Err(GovernanceError::InvalidPda.into());
    }
    Ok(CheckpointBinding {
        subject: CheckpointSubject::Proposal(proposal),
        subject_key: *subject_info.key,
        subject_digest: candidate.expected_subject_digest,
        checkpoint,
        checkpoint_bump,
    })
}

fn validate_emergency_subject(
    context: &CheckpointSubjectContext<'_, '_>,
) -> Result<CheckpointBinding, ProgramError> {
    let program_id = context.program_id;
    let subject_info = context.subject_info;
    let checkpoint_info = context.checkpoint_info;
    let config_key = context.config_key;
    let config = context.config;
    let gate_key = context.gate_key;
    let gate = context.gate;
    let candidate = context.candidate;
    let resolution = load_fixed_controller_account::<EmergencyFreezeResolutionV1>(
        program_id,
        subject_info,
        EmergencyFreezeResolutionV1::LEN,
    )?;
    validate_emergency_resolution_digest_v1(&resolution)?;
    let (resolution_pda, bump) = derive_emergency_resolution_pda(
        program_id,
        &config.target_program,
        resolution.frozen_epoch,
    );
    if *subject_info.key != resolution_pda
        || resolution.bump != bump
        || resolution.controller_config != *config_key
        || resolution.protocol_gate != *gate_key
        || resolution.target_program != config.target_program
        || resolution.target_programdata != config.target_programdata
        || resolution.target_nonce != config.target_nonce
        || resolution.frozen_epoch != gate.epoch
        || resolution.freeze_slot != gate.freeze_slot
        || resolution.freeze_reason_code != gate.freeze_reason_code
        || gate.status != GateStatusV1::EmergencyFrozen
        || gate.active_proposal != Pubkey::default()
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    if resolution.state != EmergencyFreezeResolutionStateV1::Timelocked
        || candidate.phase != StateCheckpointPhaseV1::Emergency
        || candidate.expected_subject_state
            != CheckpointSubjectStateV1::EmergencyResolutionTimelocked
        || candidate.expected_subject_digest != resolution.resolution_digest
        || candidate.expected_gate_status != GateStatusV1::EmergencyFrozen
        || candidate.expected_gate_epoch != gate.epoch
        || candidate.finalized_observation_slot < gate.freeze_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let (checkpoint, checkpoint_bump) = derive_emergency_checkpoint_pda(
        program_id,
        &config.target_program,
        resolution.frozen_epoch,
    );
    if *checkpoint_info.key != checkpoint || checkpoint != resolution.emergency_checkpoint {
        return Err(GovernanceError::InvalidPda.into());
    }
    Ok(CheckpointBinding {
        subject: CheckpointSubject::Emergency(resolution),
        subject_key: *subject_info.key,
        subject_digest: candidate.expected_subject_digest,
        checkpoint,
        checkpoint_bump,
    })
}

pub(super) fn validate_checkpoint_subject(
    context: &CheckpointSubjectContext<'_, '_>,
) -> Result<CheckpointBinding, ProgramError> {
    match context.candidate.phase {
        StateCheckpointPhaseV1::Prestate | StateCheckpointPhaseV1::Poststate => {
            validate_proposal_subject(context)
        }
        StateCheckpointPhaseV1::Emergency => validate_emergency_subject(context),
    }
}
