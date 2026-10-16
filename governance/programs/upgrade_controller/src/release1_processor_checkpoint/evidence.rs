use super::*;

pub(super) fn validate_target_snapshot(
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    candidate: &CheckpointCandidateV1,
) -> ProgramResult {
    if *target_program.key != config.target_program
        || *target_programdata.key != config.target_programdata
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let header = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &config.upgradeable_loader,
    )?;
    let data = target_programdata.try_borrow_data()?;
    // One full-account SHA pass is the mechanical live-byte check.  The
    // checkpoint's payload commitment remains governance-attested and is
    // cross-bound by the phase-specific proposal or verification evidence;
    // recomputing the convenience payload SHA here would exceed the maximum
    // transaction compute budget for a maximum-size ProgramData account.
    let raw_hash = loader_account_data_hash(&data);
    if header.deployed_slot != candidate.target_programdata_slot
        || header.capacity as u64 != candidate.target_capacity
        || raw_hash != candidate.target_raw_programdata_commitment
        || header.upgrade_authority != Some(config.authority_pda)
    {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    Ok(())
}

fn validate_buffer_phase_evidence(
    program_id: &Pubkey,
    evidence_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV2,
    candidate: &CheckpointCandidateV1,
) -> Result<u64, ProgramError> {
    let evidence = load_fixed_controller_account::<BufferVerificationV1>(
        program_id,
        evidence_info,
        BufferVerificationV1::LEN,
    )?;
    evidence.validate_schema()?;
    let (expected, bump) = derive_buffer_check_pda(program_id, &proposal_key(program_id, proposal));
    if *evidence_info.key != expected
        || evidence.bump != bump
        || evidence.status != BufferVerificationStatusV1::Verified
        || evidence.controller_config != *config_info.key
        || evidence.proposal != proposal_key(program_id, proposal)
        || evidence.upgradeable_loader != config.upgradeable_loader
        || evidence.buffer != proposal.buffer_pubkey
        || evidence.expected_uploader_authority != proposal.buffer_uploader_authority
        || evidence.controller_authority != config.authority_pda
        || evidence.artifact_length != proposal.artifact_length
        || evidence.artifact_sha256 != proposal.artifact_sha256
        || evidence.artifact_chunk_merkle_root != proposal.artifact_chunk_merkle_root
        || evidence.chunk_hash_domain != proposal.chunk_hash_domain
        || evidence.chunk_size != proposal.chunk_size
        || evidence.chunk_count != proposal.chunk_count
        || evidence.finalized_slot == 0
        || evidence.finalized_slot > candidate.finalized_observation_slot
        || proposal.deployed_slot != candidate.target_programdata_slot
        || proposal.current_capacity != candidate.target_capacity
        || proposal.current_raw_programdata_hash != candidate.target_raw_programdata_commitment
        || proposal.expected_execution_pre_payload_hash != candidate.target_payload_commitment
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(evidence.finalized_slot)
}

fn validate_rollback_prestate_evidence(
    program_id: &Pubkey,
    evidence_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    rollback: &UpgradeProposalV2,
    candidate: &CheckpointCandidateV1,
) -> Result<u64, ProgramError> {
    if rollback.proposal_class != ProposalClassV1::EmergencyRollback
        || !rollback.primary_proposal.present
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    validate_rollback_prestate_evidence_commitment(
        program_id,
        evidence_info,
        config_info,
        config,
        &RollbackPrestateCommitment {
            primary_proposal: rollback.primary_proposal.value,
            expected_candidate_full_payload_sha256: rollback.expected_execution_pre_payload_hash,
            artifact_chunk_merkle_root: rollback.expected_execution_pre_chunk_root,
            chunk_hash_domain: rollback.chunk_hash_domain,
            capacity: rollback.current_capacity,
        },
        candidate,
    )
}

pub(super) fn validate_rollback_prestate_evidence_commitment(
    program_id: &Pubkey,
    evidence_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    commitment: &RollbackPrestateCommitment,
    candidate: &CheckpointCandidateV1,
) -> Result<u64, ProgramError> {
    match evidence_info.data_len() {
        ProgramDataVerificationV1::LEN => validate_rollback_verified_programdata_evidence(
            program_id,
            evidence_info,
            config_info,
            config,
            commitment,
            candidate,
        ),
        ProgramDataFailureObservationV1::LEN => validate_rollback_programdata_failure_evidence(
            program_id,
            evidence_info,
            config_info,
            config,
            commitment,
            candidate,
        ),
        _ => Err(GovernanceError::InvalidAccountSize.into()),
    }
}

#[inline(never)]
fn validate_rollback_verified_programdata_evidence(
    program_id: &Pubkey,
    evidence_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    commitment: &RollbackPrestateCommitment,
    candidate: &CheckpointCandidateV1,
) -> Result<u64, ProgramError> {
    let evidence = load_fixed_controller_account::<ProgramDataVerificationV1>(
        program_id,
        evidence_info,
        ProgramDataVerificationV1::LEN,
    )?;
    evidence.validate_schema()?;
    let (expected, bump) = derive_programdata_check_pda(program_id, &commitment.primary_proposal);
    if *evidence_info.key != expected
        || evidence.bump != bump
        || evidence.status != ProgramDataVerificationStatusV1::Verified
        || evidence.controller_config != *config_info.key
        || evidence.proposal != commitment.primary_proposal
        || evidence.target_program != config.target_program
        || evidence.target_programdata != config.target_programdata
        || evidence.upgradeable_loader != config.upgradeable_loader
        || evidence.controller_authority != config.authority_pda
        || evidence.artifact_chunk_merkle_root != commitment.artifact_chunk_merkle_root
        || evidence.chunk_hash_domain != commitment.chunk_hash_domain
        || !evidence.zero_tail_verified
        || evidence.deployed_slot != candidate.target_programdata_slot
        || evidence.raw_programdata_hash != candidate.target_raw_programdata_commitment
        || evidence.capacity != commitment.capacity
        || evidence.capacity != candidate.target_capacity
        || candidate.target_payload_commitment != commitment.expected_candidate_full_payload_sha256
        || evidence.finalized_slot == 0
        || evidence.finalized_slot > candidate.finalized_observation_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(evidence.finalized_slot)
}

#[inline(never)]
fn validate_rollback_programdata_failure_evidence(
    program_id: &Pubkey,
    evidence_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    commitment: &RollbackPrestateCommitment,
    candidate: &CheckpointCandidateV1,
) -> Result<u64, ProgramError> {
    let evidence = load_fixed_controller_account::<ProgramDataFailureObservationV1>(
        program_id,
        evidence_info,
        ProgramDataFailureObservationV1::LEN,
    )?;
    validate_programdata_failure_observation_digest_v1(&evidence)?;

    // Rollback activation is a continuously frozen epoch transition. The
    // failure observation belongs to the primary epoch and the rollback
    // Prestate belongs to the immediately following epoch.
    let rollback_epoch = evidence
        .frozen_epoch
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let actual_data_length = evidence
        .actual_capacity
        .checked_add(LOADER_V3_PROGRAMDATA_METADATA_LEN_V1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let (expected, bump) = derive_programdata_failure_observation_pda(
        program_id,
        &commitment.primary_proposal,
        evidence.frozen_epoch,
    );
    let leaf_failure = matches!(
        evidence.mismatch_class,
        ProgramDataMismatchClassV1::PayloadLeaf | ProgramDataMismatchClassV1::ZeroTail
    );
    if *evidence_info.key != expected
        || evidence.bump != bump
        || evidence.controller_config != *config_info.key
        || evidence.protocol_gate != config.gate_pda
        || evidence.primary_proposal != commitment.primary_proposal
        || evidence.target_program != config.target_program
        || evidence.target_programdata != config.target_programdata
        || rollback_epoch != candidate.expected_gate_epoch
        || evidence.actual_program_owner != config.upgradeable_loader
        || !evidence.actual_program_executable
        || evidence.actual_program_data_length != LOADER_V3_PROGRAM_ACCOUNT_LEN_V1
        || !evidence.program_header_present
        || !evidence.actual_linked_programdata.present
        || evidence.actual_linked_programdata.value != config.target_programdata
        || !evidence.raw_hash_complete
        || evidence.actual_raw_programdata_sha256 != candidate.target_raw_programdata_commitment
        || evidence.actual_owner != config.upgradeable_loader
        || evidence.actual_executable
        || evidence.actual_data_length != actual_data_length
        || !evidence.programdata_header_present
        || evidence.actual_programdata_slot != candidate.target_programdata_slot
        || evidence.actual_capacity != commitment.capacity
        || evidence.actual_capacity != candidate.target_capacity
        || !evidence.actual_authority.present
        || evidence.actual_authority.value != config.authority_pda
        || !leaf_failure
        // A leaf mismatch proves that the deployed payload is not the expected
        // candidate payload. This checkpoint field intentionally records the
        // rollback proposal's expected candidate full-capacity payload
        // commitment; the separate raw ProgramData commitment records the
        // actual deployed bytes observed in the failure account.
        || candidate.target_payload_commitment
            != commitment.expected_candidate_full_payload_sha256
        || evidence.finalized_slot > candidate.finalized_observation_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(evidence.finalized_slot)
}

fn validate_programdata_phase_evidence(
    program_id: &Pubkey,
    evidence_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV2,
    candidate: &CheckpointCandidateV1,
) -> Result<u64, ProgramError> {
    let evidence = load_fixed_controller_account::<ProgramDataVerificationV1>(
        program_id,
        evidence_info,
        ProgramDataVerificationV1::LEN,
    )?;
    evidence.validate_schema()?;
    let proposal_key = proposal_key(program_id, proposal);
    let (expected, bump) = derive_programdata_check_pda(program_id, &proposal_key);
    if *evidence_info.key != expected
        || evidence.bump != bump
        || evidence.status != ProgramDataVerificationStatusV1::Verified
        || evidence.controller_config != *config_info.key
        || evidence.proposal != proposal_key
        || evidence.target_program != config.target_program
        || evidence.target_programdata != config.target_programdata
        || evidence.upgradeable_loader != config.upgradeable_loader
        || evidence.controller_authority != config.authority_pda
        || evidence.artifact_length != proposal.artifact_length
        || evidence.artifact_sha256 != proposal.artifact_sha256
        || evidence.artifact_chunk_merkle_root != proposal.artifact_chunk_merkle_root
        || evidence.chunk_hash_domain != proposal.chunk_hash_domain
        || evidence.chunk_size != proposal.chunk_size
        || evidence.payload_chunk_count != proposal.chunk_count
        || evidence.deployed_slot != candidate.target_programdata_slot
        || evidence.capacity != candidate.target_capacity
        || evidence.raw_programdata_hash != candidate.target_raw_programdata_commitment
        || evidence.finalized_slot == 0
        || evidence.finalized_slot != proposal.programdata_verified_slot
        || evidence.finalized_slot > candidate.finalized_observation_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(evidence.finalized_slot)
}

fn proposal_key(program_id: &Pubkey, proposal: &UpgradeProposalV2) -> Pubkey {
    derive_proposal_pda(program_id, &proposal.target_program, proposal.proposal_id).0
}

fn validate_emergency_phase_evidence(
    program_id: &Pubkey,
    evidence_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    resolution: &EmergencyFreezeResolutionV1,
    candidate: &CheckpointCandidateV1,
) -> Result<u64, ProgramError> {
    let observation = load_fixed_controller_account::<EmergencyFreezeObservationV1>(
        program_id,
        evidence_info,
        EmergencyFreezeObservationV1::LEN,
    )?;
    validate_emergency_freeze_observation_digest_v1(&observation)?;
    let (expected, bump) = derive_emergency_freeze_observation_pda(
        program_id,
        &config.target_program,
        resolution.frozen_epoch,
    );
    if *evidence_info.key != expected
        || observation.bump != bump
        || resolution.emergency_freeze_observation != *evidence_info.key
        || observation.controller_program != *program_id
        || observation.controller_config != *config_info.key
        || observation.protocol_gate != *gate_info.key
        || observation.target_program != config.target_program
        || observation.target_programdata != config.target_programdata
        || observation.upgradeable_loader != config.upgradeable_loader
        || observation.controller_authority != config.authority_pda
        || observation.frozen_epoch != resolution.frozen_epoch
        || observation.freeze_slot != resolution.freeze_slot
        || observation.freeze_reason_code != resolution.freeze_reason_code
        || observation.actual_program_owner != resolution.observed_program_owner
        || observation.actual_program_executable != resolution.observed_program_executable
        || observation.actual_program_data_length != resolution.observed_program_data_length
        || observation.program_header_present != resolution.observed_program_header_present
        || observation.actual_linked_programdata != resolution.observed_linked_programdata
        || observation.actual_programdata_owner != resolution.observed_programdata_owner
        || observation.actual_programdata_executable != resolution.observed_programdata_executable
        || observation.actual_programdata_data_length != resolution.observed_programdata_data_length
        || observation.programdata_header_present != resolution.observed_programdata_header_present
        || observation.deployed_programdata_slot != resolution.observed_programdata_slot
        || observation.raw_hash_complete != resolution.observed_raw_hash_complete
        || observation.raw_programdata_sha256 != resolution.observed_raw_programdata_hash
        || observation.capacity != resolution.observed_capacity
        || observation.observed_authority != resolution.observed_authority
        || observation.finalized_slot > candidate.finalized_observation_slot
        || resolution.creation_slot > candidate.finalized_observation_slot
        || (observation.raw_hash_complete
            && observation.raw_programdata_sha256 != candidate.target_raw_programdata_commitment)
        || (observation.programdata_header_present
            && (observation.deployed_programdata_slot != candidate.target_programdata_slot
                || observation.capacity != candidate.target_capacity))
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(observation.finalized_slot)
}

pub(super) fn validate_phase_evidence(
    program_id: &Pubkey,
    evidence_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    binding: &CheckpointBinding,
    candidate: &CheckpointCandidateV1,
) -> Result<u64, ProgramError> {
    match &binding.subject {
        CheckpointSubject::Proposal(proposal) => match candidate.phase {
            StateCheckpointPhaseV1::Prestate => {
                if proposal.proposal_class == ProposalClassV1::EmergencyRollback {
                    validate_rollback_prestate_evidence(
                        program_id,
                        evidence_info,
                        config_info,
                        config,
                        proposal,
                        candidate,
                    )
                } else {
                    validate_buffer_phase_evidence(
                        program_id,
                        evidence_info,
                        config_info,
                        config,
                        proposal,
                        candidate,
                    )
                }
            }
            StateCheckpointPhaseV1::Poststate => validate_programdata_phase_evidence(
                program_id,
                evidence_info,
                config_info,
                config,
                proposal,
                candidate,
            ),
            StateCheckpointPhaseV1::Emergency => {
                Err(GovernanceError::InvalidStateTransition.into())
            }
        },
        CheckpointSubject::Emergency(resolution) => {
            if candidate.phase != StateCheckpointPhaseV1::Emergency {
                return Err(GovernanceError::InvalidStateTransition.into());
            }
            validate_emergency_phase_evidence(
                program_id,
                evidence_info,
                config_info,
                gate_info,
                config,
                resolution,
                candidate,
            )
        }
    }
}

pub(super) fn validate_poststate_baseline(
    program_id: &Pubkey,
    baseline_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    binding: &CheckpointBinding,
    candidate: &CheckpointCandidateV1,
) -> Result<u64, ProgramError> {
    let CheckpointSubject::Proposal(proposal) = &binding.subject else {
        return Err(GovernanceError::InvalidStateTransition.into());
    };
    let baseline = load_fixed_controller_account::<StateCheckpointV1>(
        program_id,
        baseline_info,
        StateCheckpointV1::LEN,
    )?;
    validate_state_checkpoint_digest_v1(&baseline)?;
    let proposal_key = proposal_key(program_id, proposal);
    let (expected, bump) =
        derive_checkpoint_pda(program_id, &proposal_key, CheckpointPhaseV1::Prestate);
    if *baseline_info.key != expected
        || baseline.bump != bump
        || proposal.prestate_checkpoint != *baseline_info.key
        || baseline.phase != StateCheckpointPhaseV1::Prestate
        || !baseline.accepted
        || baseline.forbidden_drift_count != 0
        || baseline.controller_config != *config_info.key
        || baseline.proposal != proposal_key
        || baseline.emergency_resolution != Pubkey::default()
        || baseline.subject_digest != proposal.proposal_digest
        || baseline.target_program != config.target_program
        || baseline.target_programdata != config.target_programdata
        || baseline.gate_epoch != candidate.expected_gate_epoch
        || baseline.finalized_slot > candidate.finalized_observation_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    validate_poststate_outcome(&baseline, candidate)?;
    Ok(baseline.finalized_slot)
}

pub(super) fn validate_rollback_prestate_baseline(
    program_id: &Pubkey,
    baseline_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    binding: &CheckpointBinding,
    candidate: &CheckpointCandidateV1,
) -> Result<u64, ProgramError> {
    let CheckpointSubject::Proposal(rollback) = &binding.subject else {
        return Err(GovernanceError::InvalidStateTransition.into());
    };
    if candidate.phase != StateCheckpointPhaseV1::Prestate
        || rollback.proposal_class != ProposalClassV1::EmergencyRollback
        || !rollback.primary_proposal.present
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let baseline = load_fixed_controller_account::<StateCheckpointV1>(
        program_id,
        baseline_info,
        StateCheckpointV1::LEN,
    )?;
    validate_state_checkpoint_digest_v1(&baseline)?;
    let (expected, bump) = derive_checkpoint_pda(
        program_id,
        &rollback.primary_proposal.value,
        CheckpointPhaseV1::Prestate,
    );
    let rollback_epoch = baseline
        .gate_epoch
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if *baseline_info.key != expected
        || baseline.bump != bump
        || baseline.phase != StateCheckpointPhaseV1::Prestate
        || !baseline.accepted
        || baseline.forbidden_drift_count != 0
        || baseline.controller_config != *config_info.key
        || baseline.proposal != rollback.primary_proposal.value
        || baseline.emergency_resolution != Pubkey::default()
        || baseline.target_program != config.target_program
        || baseline.target_programdata != config.target_programdata
        || baseline.schema_identifier != rollback.checkpoint_schema_id
        || rollback_epoch != candidate.expected_gate_epoch
        || baseline.finalized_slot > candidate.finalized_observation_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    // A rollback is not allowed to establish a new protected-state baseline.
    // It inherits the primary's accepted Prestate exactly; only the existing,
    // explicitly attested nonnegative-donation lane may explain raw external
    // balance drift.
    validate_poststate_hard_invariants(&baseline, candidate)?;
    Ok(baseline.finalized_slot)
}

pub(super) fn validate_poststate_outcome(
    baseline: &StateCheckpointV1,
    candidate: &CheckpointCandidateV1,
) -> GovernanceResult<()> {
    if candidate.forbidden_drift_count == 0 {
        validate_poststate_hard_invariants(baseline, candidate)
    } else {
        validate_rejected_poststate_invariants(baseline, candidate)
    }
}

pub(super) fn validate_poststate_hard_invariants(
    baseline: &StateCheckpointV1,
    candidate: &CheckpointCandidateV1,
) -> GovernanceResult<()> {
    let hard_equal = baseline.schema_identifier == candidate.schema_identifier
        && baseline.program_owned_state_root == candidate.program_owned_state_root
        && baseline.program_owned_state_count == candidate.program_owned_state_count
        && baseline.logical_compressed_state_root == candidate.logical_compressed_state_root
        && baseline.logical_compressed_state_count == candidate.logical_compressed_state_count
        && baseline.semantic_custody_accounting_root == candidate.semantic_custody_accounting_root
        && baseline.external_metadata_observation_root
            == candidate.external_metadata_observation_root
        && baseline.hard_combined_root == candidate.hard_combined_root;
    let raw_balances_equal = baseline.external_raw_balance_observation_root
        == candidate.external_raw_balance_observation_root;
    let explicit_donation = candidate.admitted_positive_donation_count != 0
        && candidate.admitted_positive_donation_root != [0; 32];
    if candidate.forbidden_drift_count != 0
        || !hard_equal
        || (!raw_balances_equal && !explicit_donation)
        || (raw_balances_equal && explicit_donation)
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(())
}

fn validate_rejected_poststate_invariants(
    baseline: &StateCheckpointV1,
    candidate: &CheckpointCandidateV1,
) -> GovernanceResult<()> {
    let protected_state_differs = baseline.schema_identifier != candidate.schema_identifier
        || baseline.program_owned_state_root != candidate.program_owned_state_root
        || baseline.program_owned_state_count != candidate.program_owned_state_count
        || baseline.logical_compressed_state_root != candidate.logical_compressed_state_root
        || baseline.logical_compressed_state_count != candidate.logical_compressed_state_count
        || baseline.semantic_custody_accounting_root != candidate.semantic_custody_accounting_root
        || baseline.external_metadata_observation_root
            != candidate.external_metadata_observation_root
        || baseline.hard_combined_root != candidate.hard_combined_root;
    let external_balance_failure = baseline.external_raw_balance_observation_root
        != candidate.external_raw_balance_observation_root;
    if candidate.forbidden_drift_count == 0
        || (!protected_state_differs && !external_balance_failure)
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(())
}
