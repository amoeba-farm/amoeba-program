use super::*;

pub(super) fn emergency_checkpoint_subjects(
    resolution: Pubkey,
    resolution_digest: [u8; 32],
    freeze_observation: Pubkey,
    observation_subject_digest: [u8; 32],
) -> (Pubkey, [u8; 32], Pubkey, [u8; 32]) {
    (
        resolution,
        resolution_digest,
        freeze_observation,
        observation_subject_digest,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_checkpoint_subject_binding(
    program_id: &Pubkey,
    subject_info: &AccountInfo<'_>,
    linked_primary_or_authority: &AccountInfo<'_>,
    checkpoint_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    deployment_info: &AccountInfo<'_>,
    context: &LifecycleContext,
    manifest: &CheckpointManifestV2,
) -> Result<CheckpointBindingV2, ProgramError> {
    match manifest.phase {
        StateCheckpointPhaseV1::Prestate | StateCheckpointPhaseV1::Poststate => {
            let proposal = load_proposal(
                program_id,
                subject_info,
                config_info,
                gate_info,
                capacity_info,
                deployment_info,
                context,
            )?;
            let phase = match manifest.phase {
                StateCheckpointPhaseV1::Prestate => CheckpointPhaseV1::Prestate,
                StateCheckpointPhaseV1::Poststate => CheckpointPhaseV1::Poststate,
                StateCheckpointPhaseV1::Emergency => unreachable!(),
            };
            let expected = derive_checkpoint_pda(program_id, subject_info.key, phase);
            let expected_state = match manifest.phase {
                StateCheckpointPhaseV1::Prestate => ProposalStateV2::Frozen,
                StateCheckpointPhaseV1::Poststate => ProposalStateV2::ProgramDataVerified,
                StateCheckpointPhaseV1::Emergency => unreachable!(),
            };
            let consumed_nonce = proposal
                .target_nonce
                .checked_add(1)
                .ok_or(GovernanceError::ArithmeticOverflow)?;
            if expected.0 != *checkpoint_info.key
                || proposal.state != expected_state
                || proposal.freeze_gate_epoch != context.gate.epoch
                || context.gate.status != GateStatusV1::FrozenForUpgrade
                || context.gate.active_proposal != *subject_info.key
                || context.gate.freeze_slot != proposal.frozen_slot
                || context.config.target_nonce != consumed_nonce
                || manifest.expected_subject_digest != proposal.proposal_digest
                || (manifest.phase == StateCheckpointPhaseV1::Prestate
                    && proposal.prestate_checkpoint != *checkpoint_info.key)
                || (manifest.phase == StateCheckpointPhaseV1::Poststate
                    && proposal.required_poststate_checkpoint != *checkpoint_info.key)
            {
                return Err(GovernanceError::InvalidStateTransition.into());
            }
            let purpose = match (manifest.phase, proposal.proposal_class) {
                (StateCheckpointPhaseV1::Prestate, _) => {
                    ProgramDataObservationPurposeV1::ProposalPrestate
                }
                (StateCheckpointPhaseV1::Poststate, ProposalClassV1::EmergencyRollback) => {
                    ProgramDataObservationPurposeV1::Rollback
                }
                (StateCheckpointPhaseV1::Poststate, _) => {
                    ProgramDataObservationPurposeV1::PostUpgrade
                }
                (StateCheckpointPhaseV1::Emergency, _) => unreachable!(),
            };
            let (minimum_required_capacity, observation_expectation) = match manifest.phase {
                StateCheckpointPhaseV1::Prestate
                    if proposal.proposal_class == ProposalClassV1::EmergencyRollback =>
                {
                    if !proposal.primary_proposal.present
                        || proposal.primary_proposal.value != *linked_primary_or_authority.key
                    {
                        return Err(GovernanceError::InvalidProposalCommitment.into());
                    }
                    let primary = load_proposal(
                        program_id,
                        linked_primary_or_authority,
                        config_info,
                        gate_info,
                        capacity_info,
                        deployment_info,
                        context,
                    )?;
                    let primary_next_epoch =
                        checked_nonterminal_increment(primary.freeze_gate_epoch)?;
                    if primary.proposal_class == ProposalClassV1::EmergencyRollback
                        || primary.state != ProposalStateV2::UpgradeExecuted
                        || primary.upgrade_executed_slot == 0
                        || primary.upgrade_executed_slot > proposal.frozen_slot
                        || primary_next_epoch != proposal.freeze_gate_epoch
                        || !primary.rollback_proposal.present
                        || primary.rollback_proposal.value != *subject_info.key
                        || !primary.rollback_buffer.present
                        || primary.rollback_buffer.value != proposal.buffer_pubkey
                        || primary.rollback_artifact_length != proposal.artifact_length
                        || primary.rollback_artifact_sha256 != proposal.artifact_sha256
                        || primary.rollback_artifact_chunk_root
                            != proposal.artifact_chunk_merkle_root
                        || primary.rollback_artifact_scheme_id != proposal.artifact_scheme_id
                        || primary.target_nonce != proposal.target_nonce
                        || primary.checkpoint_schema_id != proposal.checkpoint_schema_id
                        || primary.checkpoint_policy_hash != proposal.checkpoint_policy_hash
                    {
                        return Err(GovernanceError::InvalidProposalCommitment.into());
                    }
                    (
                        primary.minimum_required_capacity,
                        failed_primary_observation_expectation(&primary),
                    )
                }
                StateCheckpointPhaseV1::Prestate => {
                    if *linked_primary_or_authority.key != context.config.authority_pda {
                        return Err(GovernanceError::CrossAccountMismatch.into());
                    }
                    (
                        context.deployment.artifact_length,
                        trusted_observation_expectation(&context.deployment),
                    )
                }
                StateCheckpointPhaseV1::Poststate => {
                    if *linked_primary_or_authority.key != context.config.authority_pda
                        || proposal.upgrade_executed_slot == 0
                    {
                        return Err(GovernanceError::CrossAccountMismatch.into());
                    }
                    (
                        proposal.minimum_required_capacity,
                        candidate_observation_expectation(
                            proposal.artifact_length,
                            proposal.artifact_sha256,
                            proposal.artifact_chunk_merkle_root,
                            proposal.artifact_scheme_id,
                            proposal.upgrade_executed_slot,
                        ),
                    )
                }
                StateCheckpointPhaseV1::Emergency => unreachable!(),
            };
            let observation_subject_digest = expected_programdata_observation_subject_digest(
                program_id,
                config_info,
                gate_info,
                subject_info.key,
                &context.config,
                &context.gate,
                &context.capacity,
                &observation_expectation,
                purpose,
                manifest.expected_observation_generation,
                minimum_required_capacity,
            )?;
            Ok(CheckpointBindingV2 {
                checkpoint: expected.0,
                bump: expected.1,
                checkpoint_subject: *subject_info.key,
                subject_digest: proposal.proposal_digest,
                observation_subject: *subject_info.key,
                observation_subject_digest,
                expected_observation: None,
                purpose,
                minimum_required_capacity,
                observation_expectation,
                record: CheckpointSubjectRecord::Proposal(proposal),
            })
        }
        StateCheckpointPhaseV1::Emergency => {
            if *linked_primary_or_authority.key != context.config.authority_pda {
                return Err(GovernanceError::CrossAccountMismatch.into());
            }
            let resolution = load_emergency_resolution(
                program_id,
                subject_info,
                config_info,
                gate_info,
                capacity_info,
                deployment_info,
                context,
            )?;
            let expected = derive_emergency_checkpoint_v2_pda(program_id, subject_info.key);
            if expected.0 != *checkpoint_info.key
                || resolution.state != EmergencyFreezeResolutionStateV1::Draft
                || resolution.approval_count != 0
                || resolution.emergency_checkpoint_digest != [0; 32]
                || resolution.emergency_checkpoint != *checkpoint_info.key
                || manifest.expected_subject_digest != resolution.resolution_digest
                || manifest.expected_observation_generation != resolution.observation_generation
                || manifest.expected_observation_digest != resolution.observation_digest
            {
                return Err(GovernanceError::InvalidStateTransition.into());
            }
            let observation_expectation = trusted_observation_expectation(&context.deployment);
            let observation_subject_digest = expected_programdata_observation_subject_digest(
                program_id,
                config_info,
                gate_info,
                &resolution.emergency_freeze_observation,
                &context.config,
                &context.gate,
                &context.capacity,
                &observation_expectation,
                ProgramDataObservationPurposeV1::EmergencyResolution,
                manifest.expected_observation_generation,
                context.deployment.artifact_length,
            )?;
            let (
                checkpoint_subject,
                subject_digest,
                observation_subject,
                observation_subject_digest,
            ) = emergency_checkpoint_subjects(
                *subject_info.key,
                resolution.resolution_digest,
                resolution.emergency_freeze_observation,
                observation_subject_digest,
            );
            Ok(CheckpointBindingV2 {
                checkpoint: expected.0,
                bump: expected.1,
                checkpoint_subject,
                subject_digest,
                observation_subject,
                observation_subject_digest,
                expected_observation: Some(resolution.programdata_observation),
                purpose: ProgramDataObservationPurposeV1::EmergencyResolution,
                minimum_required_capacity: context.deployment.artifact_length,
                observation_expectation,
                record: CheckpointSubjectRecord::Emergency(resolution),
            })
        }
    }
}

fn compute_checkpoint_hard_root_v2(manifest: &CheckpointManifestV2) -> [u8; 32] {
    hashv(&[
        STATE_CHECKPOINT_HARD_ROOT_DOMAIN_V1,
        &manifest.schema_identifier,
        &manifest.program_owned_state_root,
        &manifest.program_owned_state_count.to_le_bytes(),
        &manifest.logical_compressed_state_root,
        &manifest.logical_compressed_state_count.to_le_bytes(),
        &manifest.semantic_custody_accounting_root,
        &manifest.external_metadata_observation_root,
    ])
    .to_bytes()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn derive_checkpoint_candidate(
    _program_id: &Pubkey,
    manifest: &CheckpointManifestV2,
    binding: &CheckpointBindingV2,
    config_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    deployment_info: &AccountInfo<'_>,
    observation_info: &AccountInfo<'_>,
    context: &LifecycleContext,
    council: &GovernanceCouncilSetV1,
    observation: &ProgramDataObservationV1,
) -> Result<Box<StateCheckpointV2>, ProgramError> {
    if manifest.expected_gate_epoch != context.gate.epoch
        || manifest.expected_capacity_policy_digest != context.capacity.policy_digest
        || manifest.expected_current_deployment_digest != context.deployment.deployment_digest
        || manifest.expected_current_deployment_generation
            != context.deployment.deployment_generation
        || manifest.expected_observation_digest != observation.observation_digest
        || manifest.expected_observation_generation != observation.generation
        || manifest.expected_council_version != council.version
        || manifest.expected_council_hash != council.set_hash
        || manifest.expected_subject_digest != binding.subject_digest
        || manifest.forbidden_drift_count != 0
        || manifest.hard_combined_root != compute_checkpoint_hard_root_v2(manifest)
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let (proposal, emergency_resolution) = match &binding.record {
        CheckpointSubjectRecord::Proposal(_) => (binding.checkpoint_subject, Pubkey::default()),
        CheckpointSubjectRecord::Emergency(_) => (Pubkey::default(), binding.checkpoint_subject),
    };
    let mut checkpoint = Box::new(StateCheckpointV2 {
        discriminator: STATE_CHECKPOINT_V2_DISCRIMINATOR,
        account_version: CAPACITY_SAFE_ACCOUNT_VERSION_V2,
        bump: binding.bump,
        initialized: true,
        phase: manifest.phase,
        controller_config: *config_info.key,
        proposal,
        emergency_resolution,
        subject_digest: binding.subject_digest,
        target_program: context.config.target_program,
        target_programdata: context.config.target_programdata,
        capacity_policy: *capacity_info.key,
        capacity_policy_digest: context.capacity.policy_digest,
        current_deployment_state: *deployment_info.key,
        current_deployment_digest: context.deployment.deployment_digest,
        current_deployment_generation: context.deployment.deployment_generation,
        checkpoint_generation: manifest.checkpoint_generation,
        previous_checkpoint_digest: manifest.previous_checkpoint_digest,
        observation_scheme_id: context.capacity.observation_scheme_id,
        programdata_observation: *observation_info.key,
        observation_purpose: binding.purpose,
        observation_generation: observation.generation,
        observation_subject_digest: observation.subject_digest,
        observation_root: observation.final_raw_merkle_root,
        observation_digest: observation.observation_digest,
        observation_finalized_slot: observation.finalized_slot,
        gate_epoch: context.gate.epoch,
        target_programdata_slot: observation.deployed_slot,
        artifact_length: binding.observation_expectation.artifact_length,
        artifact_sha256: binding.observation_expectation.artifact_sha256,
        artifact_merkle_root: binding.observation_expectation.artifact_merkle_root,
        artifact_scheme_id: binding.observation_expectation.artifact_scheme_id,
        minimum_required_capacity: binding.minimum_required_capacity,
        observed_raw_data_length: observation.raw_data_length,
        actual_capacity: observation.actual_capacity,
        observed_authority: observation.upgrade_authority,
        program_owned_state_root: manifest.program_owned_state_root,
        program_owned_state_count: manifest.program_owned_state_count,
        logical_compressed_state_root: manifest.logical_compressed_state_root,
        logical_compressed_state_count: manifest.logical_compressed_state_count,
        semantic_custody_accounting_root: manifest.semantic_custody_accounting_root,
        hard_combined_root: manifest.hard_combined_root,
        external_metadata_observation_root: manifest.external_metadata_observation_root,
        external_raw_balance_observation_root: manifest.external_raw_balance_observation_root,
        schema_identifier: manifest.schema_identifier,
        admitted_positive_donation_root: manifest.admitted_positive_donation_root,
        admitted_positive_donation_count: manifest.admitted_positive_donation_count,
        forbidden_drift_count: manifest.forbidden_drift_count,
        approval_council_version: council.version,
        approval_council_hash: council.set_hash,
        checkpoint_digest_domain_id: STATE_CHECKPOINT_V2_DIGEST_DOMAIN_ID,
        checkpoint_digest: [0; 32],
        approval_bitset: 0,
        approval_count: 0,
        accepted: false,
        // This value is permissionless lifecycle evidence and is cleared from
        // the digest. At attestation time use the finalized observation slot so
        // the fully reconstructed candidate itself still passes strict schema.
        finalized_slot: observation.finalized_slot,
        reserved: [0; STATE_CHECKPOINT_V2_RESERVED_LEN],
    });
    checkpoint.checkpoint_digest = compute_state_checkpoint_digest_v2(&checkpoint)?;
    if checkpoint.checkpoint_digest != manifest.expected_checkpoint_digest {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    validate_state_checkpoint_digest_v2(&checkpoint)?;
    Ok(checkpoint)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_checkpoint_attestation(
    program_id: &Pubkey,
    manifest: &CheckpointManifestV2,
    seat_index: u8,
    config_info: &AccountInfo<'_>,
    policy_info: &AccountInfo<'_>,
    council_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    subject_info: &AccountInfo<'_>,
    linked_primary_or_authority: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    deployment_info: &AccountInfo<'_>,
    observation_info: &AccountInfo<'_>,
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    checkpoint_info: &AccountInfo<'_>,
    seat: &AccountInfo<'_>,
) -> Result<PreparedCheckpointAttestation, ProgramError> {
    let slot = current_slot()?;
    if slot > manifest.plan_valid_until_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let context = load_lifecycle_context(
        program_id,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
    )?;
    let policy = load_policy(program_id, policy_info, config_info, &context.config, slot)?;
    let council = load_current_council(
        program_id,
        council_info,
        config_info,
        &context.config,
        &policy,
        slot,
    )?;
    let seat_record = council
        .seats
        .get(usize::from(seat_index))
        .ok_or(GovernanceError::InactiveCouncilSeat)?;
    if *seat.key == context.config.guardian
        || seat_record.seat_authority != *seat.key
        || !seat_record.term_covers(slot)
    {
        return Err(GovernanceError::InactiveCouncilSeat.into());
    }
    let binding = load_checkpoint_subject_binding(
        program_id,
        subject_info,
        linked_primary_or_authority,
        checkpoint_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
        manifest,
    )?;
    if binding
        .expected_observation
        .is_some_and(|expected| expected != *observation_info.key)
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    if binding.checkpoint != *checkpoint_info.key
        || checkpoint_info.owner != &system_program::ID
        || checkpoint_info.data_len() != 0
    {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let observation = load_fresh_programdata_observation(
        program_id,
        observation_info,
        config_info,
        gate_info,
        capacity_info,
        &binding.observation_subject,
        &context.config,
        &context.gate,
        &context.capacity,
        &binding.observation_expectation,
        binding.purpose,
        &binding.observation_subject_digest,
        manifest.expected_observation_generation,
        &manifest.expected_observation_digest,
        binding.minimum_required_capacity,
        slot,
    )?;
    require_observation_matches_runtime(
        &observation,
        target_program,
        target_programdata,
        &context.config,
        &context.capacity,
    )?;
    let checkpoint = derive_checkpoint_candidate(
        program_id,
        manifest,
        &binding,
        config_info,
        capacity_info,
        deployment_info,
        observation_info,
        &context,
        &council,
        &observation,
    )?;
    Ok((context, council, binding, checkpoint, slot))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_checkpoint_attestation(
    program_id: &Pubkey,
    config_info: &AccountInfo<'_>,
    council_info: &AccountInfo<'_>,
    subject_info: &AccountInfo<'_>,
    checkpoint: &StateCheckpointV2,
    council: &GovernanceCouncilSetV1,
    seat_index: u8,
    seat: &AccountInfo<'_>,
    slot: u64,
    bump: u8,
) -> GovernanceResult<CheckpointAttestationV1> {
    let mut attestation = CheckpointAttestationV1 {
        discriminator: CHECKPOINT_ATTESTATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        controller_program: *program_id,
        controller_config: *config_info.key,
        checkpoint: derive_checkpoint_identity(checkpoint, program_id),
        subject: *subject_info.key,
        subject_digest: checkpoint.subject_digest,
        phase: checkpoint.phase,
        checkpoint_digest: checkpoint.checkpoint_digest,
        council: *council_info.key,
        council_version: council.version,
        council_hash: council.set_hash,
        gate_epoch: checkpoint.gate_epoch,
        seat_index,
        seat_authority: *seat.key,
        attested_slot: slot,
        attestation_digest: [0; 32],
        reserved: [0; CHECKPOINT_ATTESTATION_V1_RESERVED_LEN],
    };
    attestation.attestation_digest = compute_checkpoint_attestation_digest_v1(&attestation)?;
    validate_checkpoint_attestation_digest_v1(&attestation)?;
    Ok(attestation)
}

fn derive_checkpoint_identity(checkpoint: &StateCheckpointV2, program_id: &Pubkey) -> Pubkey {
    match checkpoint.phase {
        StateCheckpointPhaseV1::Prestate => {
            derive_checkpoint_pda(
                program_id,
                &checkpoint.proposal,
                CheckpointPhaseV1::Prestate,
            )
            .0
        }
        StateCheckpointPhaseV1::Poststate => {
            derive_checkpoint_pda(
                program_id,
                &checkpoint.proposal,
                CheckpointPhaseV1::Poststate,
            )
            .0
        }
        StateCheckpointPhaseV1::Emergency => {
            derive_emergency_checkpoint_v2_pda(program_id, &checkpoint.emergency_resolution).0
        }
    }
}
