use super::*;

pub(super) fn validate_bootstrap_gate(
    gate: &ProtocolGateV1,
    config: &ControllerConfigV1,
) -> ProgramResult {
    if gate.status != GateStatusV1::EmergencyFrozen
        || gate.epoch == 0
        || gate.active_proposal != Pubkey::default()
        || gate.freeze_slot == 0
        || gate.freeze_reason_code != BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        || gate.target_program != config.target_program
        || gate.target_programdata != config.target_programdata
    {
        return Err(GovernanceError::InvalidGateState.into());
    }
    Ok(())
}

pub(super) fn validate_controller_immutability_transition(
    controller_program: &AccountInfo<'_>,
    controller_programdata: &AccountInfo<'_>,
    release: &ControllerReleaseCommitmentV1,
    pre: &ProgramDataObservationV1,
    post: &ProgramDataObservationV1,
) -> ProgramResult {
    if pre.generation >= post.generation
        || pre.target_program != *controller_program.key
        || post.target_program != *controller_program.key
        || pre.target_programdata != *controller_programdata.key
        || post.target_programdata != *controller_programdata.key
        || pre.program_header_snapshot != post.program_header_snapshot
        || pre.deployed_slot > post.deployed_slot
        || pre.raw_data_length > post.raw_data_length
        || pre.actual_capacity > post.actual_capacity
        || pre.expected_artifact_length != post.expected_artifact_length
        || pre.expected_artifact_sha256 != post.expected_artifact_sha256
        || pre.expected_artifact_merkle_root != post.expected_artifact_merkle_root
        || pre.expected_artifact_scheme_id != post.expected_artifact_scheme_id
        || pre.final_raw_merkle_root == post.final_raw_merkle_root
        || pre.observation_digest == post.observation_digest
        || pre.upgrade_authority != release.pre_immutability_authority
        || post.upgrade_authority != OptionalPubkeyV1::none()
        || release.minimum_programdata_capacity > pre.actual_capacity
        || release.minimum_programdata_capacity > post.actual_capacity
        || release.artifact_length != post.expected_artifact_length
        || release.artifact_sha256 != post.expected_artifact_sha256
        || release.artifact_merkle_root != post.expected_artifact_merkle_root
        || release.artifact_scheme_id != post.expected_artifact_scheme_id
    {
        return Err(GovernanceError::ControllerNotImmutable.into());
    }
    validate_some_to_none_header_delta(
        &pre.programdata_header_snapshot,
        &post.programdata_header_snapshot,
        release.pre_immutability_authority.value,
    )?;
    require_live_observation(controller_program, controller_programdata, post, None)
}

pub(super) fn validate_controller_still_immutable(
    program_id: &Pubkey,
    controller_program: &AccountInfo<'_>,
    controller_programdata: &AccountInfo<'_>,
    receipt: &ControllerImmutabilityReceiptV1,
) -> ProgramResult {
    if *controller_program.key != *program_id
        || *controller_programdata.key != receipt.controller_programdata
    {
        return Err(GovernanceError::ControllerNotImmutable.into());
    }
    let header = validate_program_programdata_linkage(
        controller_program,
        controller_programdata,
        &receipt.upgradeable_loader,
    )?;
    if header.upgrade_authority.is_some()
        || header.deployed_slot != receipt.deployed_slot
        || u64::try_from(header.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != receipt.programdata_capacity
    {
        return Err(GovernanceError::ControllerNotImmutable.into());
    }
    Ok(())
}

pub(super) fn require_live_observation(
    program: &AccountInfo<'_>,
    programdata: &AccountInfo<'_>,
    observation: &ProgramDataObservationV1,
    expected_authority: Option<Pubkey>,
) -> ProgramResult {
    if *program.key != observation.target_program
        || *programdata.key != observation.target_programdata
        || program.owner != &observation.program_owner
        || programdata.owner != &observation.programdata_owner
        || program.executable != observation.program_executable
        || programdata.executable != observation.programdata_executable
        || u64::try_from(program.data_len()).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != observation.program_data_length
        || u64::try_from(programdata.data_len()).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != observation.raw_data_length
        || copy_program_header(program)? != observation.program_header_snapshot
        || copy_programdata_header(programdata)? != observation.programdata_header_snapshot
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    let header = validate_program_programdata_linkage(
        program,
        programdata,
        &observation.upgradeable_loader,
    )?;
    if header.deployed_slot != observation.deployed_slot
        || u64::try_from(header.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != observation.actual_capacity
        || header.upgrade_authority != expected_authority
        || !optional_matches(&observation.upgrade_authority, expected_authority)
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(())
}

pub(super) fn validate_target_graph(
    config: &ControllerConfigV1,
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    authority: &AccountInfo<'_>,
    loader: &AccountInfo<'_>,
) -> ProgramResult {
    if *target_program.key != config.target_program
        || *target_programdata.key != config.target_programdata
        || *authority.key != config.authority_pda
        || *loader.key != config.upgradeable_loader
        || config.upgradeable_loader != UPGRADEABLE_LOADER_ID
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_handoff_proposal_evidence(
    proposal: &TargetAuthorityHandoffProposalV1,
    context: &CeremonyContext,
    immutability_info: &AccountInfo<'_>,
    immutable: &ControllerImmutabilityReceiptV1,
    observation_info: &AccountInfo<'_>,
    observation: &ProgramDataObservationV1,
    legacy_authority: &Pubkey,
) -> ProgramResult {
    let same_observation_generation =
        proposal.bridge_observation_generation == observation.generation;
    if proposal.controller_immutability_digest != immutable.receipt_digest
        || observation.generation < proposal.bridge_observation_generation
        || (same_observation_generation
            && (proposal.bridge_observation != *observation_info.key
                || proposal.bridge_observation_root != observation.final_raw_merkle_root
                || proposal.bridge_observation_digest != observation.observation_digest))
        || proposal.legacy_target_authority != *legacy_authority
        || proposal.bridge_artifact_length != observation.expected_artifact_length
        || proposal.bridge_artifact_sha256 != observation.expected_artifact_sha256
        || proposal.bridge_artifact_merkle_root != observation.expected_artifact_merkle_root
        || proposal.bridge_artifact_scheme_id != observation.expected_artifact_scheme_id
        || proposal.minimum_target_deployed_slot > observation.deployed_slot
        || proposal.minimum_target_capacity > observation.actual_capacity
        || proposal.minimum_target_raw_length > observation.raw_data_length
        || proposal.bootstrap_gate_status != context.gate.status
        || proposal.bootstrap_freeze_reason_code != context.gate.freeze_reason_code
        || proposal.bootstrap_freeze_slot != context.gate.freeze_slot
        || proposal.controller_immutability_receipt != *immutability_info.key
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_handoff_review_state(
    program_id: &Pubkey,
    controller_program: &AccountInfo<'_>,
    controller_programdata: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    policy_info: &AccountInfo<'_>,
    council_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    immutability_info: &AccountInfo<'_>,
    observation_info: &AccountInfo<'_>,
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    legacy_authority: &AccountInfo<'_>,
    authority_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    loader_info: &AccountInfo<'_>,
    slot: u64,
) -> Result<HandoffReviewState, ProgramError> {
    let context = load_ceremony_context(
        program_id,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        slot,
    )?;
    validate_bootstrap_gate(&context.gate, &context.config)?;
    let immutable = load_immutability_receipt(
        program_id,
        immutability_info,
        capacity_info,
        &context.capacity,
        &context.config,
    )?;
    validate_controller_still_immutable(
        program_id,
        controller_program,
        controller_programdata,
        &immutable,
    )?;
    let observation = load_observation(
        program_id,
        observation_info,
        config_info,
        &context.config,
        capacity_info,
        &context.capacity,
        target_program.key,
        ProgramDataObservationPurposeV1::TargetHandoffBridge,
        immutability_info.key,
    )?;
    require_live_observation(
        target_program,
        target_programdata,
        &observation,
        Some(*legacy_authority.key),
    )?;
    validate_target_graph(
        &context.config,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
    )?;
    let proposal = load_handoff_proposal(
        program_id,
        proposal_info,
        config_info,
        policy_info,
        capacity_info,
        gate_info,
        immutability_info,
        &context,
    )?;
    validate_handoff_proposal_evidence(
        &proposal,
        &context,
        immutability_info,
        &immutable,
        observation_info,
        &observation,
        legacy_authority.key,
    )?;
    Ok(HandoffReviewState { context, proposal })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_activation_evidence(
    program_id: &Pubkey,
    controller_program: &AccountInfo<'_>,
    controller_programdata: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    policy_info: &AccountInfo<'_>,
    council_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    immutability_info: &AccountInfo<'_>,
    handoff_receipt_info: &AccountInfo<'_>,
    observation_info: &AccountInfo<'_>,
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    authority_info: &AccountInfo<'_>,
    loader_info: &AccountInfo<'_>,
    slot: u64,
) -> Result<ActivationEvidence, ProgramError> {
    let context = load_ceremony_context(
        program_id,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        slot,
    )?;
    validate_bootstrap_gate(&context.gate, &context.config)?;
    let immutable = load_immutability_receipt(
        program_id,
        immutability_info,
        capacity_info,
        &context.capacity,
        &context.config,
    )?;
    validate_controller_still_immutable(
        program_id,
        controller_program,
        controller_programdata,
        &immutable,
    )?;
    let handoff = load_handoff_receipt(
        program_id,
        handoff_receipt_info,
        config_info,
        immutability_info,
        &context,
    )?;
    if handoff.controller_immutability_digest != immutable.receipt_digest {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let observation = load_observation(
        program_id,
        observation_info,
        config_info,
        &context.config,
        capacity_info,
        &context.capacity,
        target_program.key,
        ProgramDataObservationPurposeV1::BootstrapActivation,
        handoff_receipt_info.key,
    )?;
    validate_target_graph(
        &context.config,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
    )?;
    require_live_observation(
        target_program,
        target_programdata,
        &observation,
        Some(context.config.authority_pda),
    )?;
    if observation.expected_artifact_length != handoff.artifact_length
        || observation.expected_artifact_sha256 != handoff.artifact_sha256
        || observation.expected_artifact_merkle_root != handoff.artifact_merkle_root
        || observation.expected_artifact_scheme_id != handoff.artifact_scheme_id
        || observation.deployed_slot < handoff.deployed_slot
        || observation.actual_capacity < handoff.programdata_capacity
        || observation.raw_data_length < handoff.raw_programdata_length
        || *authority_info.key != handoff.controller_authority
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(ActivationEvidence {
        context,
        immutable,
        handoff,
        observation,
    })
}

pub(super) fn validate_activation_proposal_evidence(
    proposal: &BootstrapActivationProposalV1,
    evidence: &ActivationEvidence,
    observation_info: &AccountInfo<'_>,
    immutability_info: &AccountInfo<'_>,
    handoff_receipt_info: &AccountInfo<'_>,
) -> ProgramResult {
    let same_observation_generation =
        proposal.bridge_observation_generation == evidence.observation.generation;
    if proposal.controller_immutability_receipt != *immutability_info.key
        || proposal.controller_immutability_digest != evidence.immutable.receipt_digest
        || proposal.target_handoff_receipt != *handoff_receipt_info.key
        || proposal.target_handoff_digest != evidence.handoff.receipt_digest
        || evidence.observation.generation < proposal.bridge_observation_generation
        || (same_observation_generation
            && (proposal.bridge_observation != *observation_info.key
                || proposal.bridge_observation_root != evidence.observation.final_raw_merkle_root
                || proposal.bridge_observation_digest != evidence.observation.observation_digest))
        || proposal.bridge_artifact_length != evidence.handoff.artifact_length
        || proposal.bridge_artifact_sha256 != evidence.handoff.artifact_sha256
        || proposal.bridge_artifact_merkle_root != evidence.handoff.artifact_merkle_root
        || proposal.bridge_artifact_scheme_id != evidence.handoff.artifact_scheme_id
        || proposal.bridge_source_commitment != evidence.handoff.bridge_source_commitment
        || proposal.bridge_build_inputs_commitment
            != evidence.handoff.bridge_build_inputs_commitment
        || proposal.bridge_package_commitment != evidence.handoff.bridge_package_commitment
        || proposal.bridge_release_manifest_commitment
            != evidence.handoff.bridge_release_manifest_commitment
        || proposal.minimum_target_deployed_slot > evidence.observation.deployed_slot
        || proposal.minimum_target_capacity > evidence.observation.actual_capacity
        || proposal.minimum_target_raw_length > evidence.observation.raw_data_length
        || proposal.bootstrap_gate_status != evidence.context.gate.status
        || proposal.bootstrap_freeze_reason_code != evidence.context.gate.freeze_reason_code
        || proposal.bootstrap_freeze_slot != evidence.context.gate.freeze_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_handoff_review_privileges<'a>(
    controller_program: &AccountInfo<'a>,
    controller_programdata: &AccountInfo<'a>,
    config: &AccountInfo<'a>,
    policy: &AccountInfo<'a>,
    council: &AccountInfo<'a>,
    gate: &AccountInfo<'a>,
    capacity: &AccountInfo<'a>,
    immutable: &AccountInfo<'a>,
    observation: &AccountInfo<'a>,
    target_program: &AccountInfo<'a>,
    target_programdata: &AccountInfo<'a>,
    legacy_authority: &AccountInfo<'a>,
    authority: &AccountInfo<'a>,
    proposal: &AccountInfo<'a>,
    loader: &AccountInfo<'a>,
) -> ProgramResult {
    validate_exact_privileges(controller_program, false, false, true)?;
    for account in [
        controller_programdata,
        config,
        policy,
        council,
        gate,
        capacity,
        immutable,
        observation,
        target_programdata,
        legacy_authority,
        authority,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(target_program, false, false, true)?;
    validate_exact_privileges(proposal, true, false, false)?;
    validate_exact_privileges(loader, false, false, true)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_activation_review_privileges<'a>(
    controller_program: &AccountInfo<'a>,
    controller_programdata: &AccountInfo<'a>,
    config: &AccountInfo<'a>,
    policy: &AccountInfo<'a>,
    council: &AccountInfo<'a>,
    gate: &AccountInfo<'a>,
    capacity: &AccountInfo<'a>,
    immutable: &AccountInfo<'a>,
    handoff: &AccountInfo<'a>,
    observation: &AccountInfo<'a>,
    target_program: &AccountInfo<'a>,
    target_programdata: &AccountInfo<'a>,
    authority: &AccountInfo<'a>,
    proposal: &AccountInfo<'a>,
    loader: &AccountInfo<'a>,
) -> ProgramResult {
    validate_exact_privileges(controller_program, false, false, true)?;
    for account in [
        controller_programdata,
        config,
        policy,
        council,
        gate,
        capacity,
        immutable,
        handoff,
        observation,
        target_programdata,
        authority,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(target_program, false, false, true)?;
    validate_exact_privileges(proposal, true, false, false)?;
    validate_exact_privileges(loader, false, false, true)
}
