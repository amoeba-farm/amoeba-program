use super::*;

fn checkpoint_from_candidate(
    candidate: &CheckpointCandidateV1,
    binding: &CheckpointBinding,
    config_key: Pubkey,
    config: &ControllerConfigV1,
    finalization: CheckpointFinalizationFields,
) -> StateCheckpointV1 {
    let (proposal, emergency_resolution) = match &binding.subject {
        CheckpointSubject::Proposal(_) => (binding.subject_key, Pubkey::default()),
        CheckpointSubject::Emergency(_) => (Pubkey::default(), binding.subject_key),
    };
    StateCheckpointV1 {
        discriminator: STATE_CHECKPOINT_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: binding.checkpoint_bump,
        initialized: true,
        phase: candidate.phase,
        controller_config: config_key,
        proposal,
        emergency_resolution,
        subject_digest: binding.subject_digest,
        target_program: config.target_program,
        target_programdata: config.target_programdata,
        finalized_observation_slot: candidate.finalized_observation_slot,
        gate_epoch: candidate.expected_gate_epoch,
        target_programdata_slot: candidate.target_programdata_slot,
        target_payload_commitment: candidate.target_payload_commitment,
        target_raw_programdata_commitment: candidate.target_raw_programdata_commitment,
        target_capacity: candidate.target_capacity,
        program_owned_state_root: candidate.program_owned_state_root,
        program_owned_state_count: candidate.program_owned_state_count,
        logical_compressed_state_root: candidate.logical_compressed_state_root,
        logical_compressed_state_count: candidate.logical_compressed_state_count,
        semantic_custody_accounting_root: candidate.semantic_custody_accounting_root,
        hard_combined_root: candidate.hard_combined_root,
        external_metadata_observation_root: candidate.external_metadata_observation_root,
        external_raw_balance_observation_root: candidate.external_raw_balance_observation_root,
        schema_identifier: candidate.schema_identifier,
        admitted_positive_donation_root: candidate.admitted_positive_donation_root,
        admitted_positive_donation_count: candidate.admitted_positive_donation_count,
        forbidden_drift_count: candidate.forbidden_drift_count,
        approval_council_version: finalization.council_version,
        approval_council_hash: finalization.council_hash,
        checkpoint_digest: candidate.expected_checkpoint_digest,
        approval_bitset: finalization.approval_bitset,
        approval_count: finalization.approval_bitset.count_ones() as u8,
        accepted: candidate.forbidden_drift_count == 0,
        finalized_slot: finalization.finalized_slot,
        reserved: [0; STATE_CHECKPOINT_V1_RESERVED_LEN],
    }
}

pub(super) fn validate_checkpoint_candidate(
    candidate: &CheckpointCandidateV1,
    binding: &CheckpointBinding,
    config_key: Pubkey,
    config: &ControllerConfigV1,
    council: &GovernanceCouncilSetV1,
    slot: u64,
) -> Result<StateCheckpointV1, ProgramError> {
    validate_checkpoint_acceptance_shape(candidate)?;
    if candidate.finalized_observation_slot == 0
        || candidate.finalized_observation_slot > slot
        || candidate.target_programdata_slot > candidate.finalized_observation_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let prototype = checkpoint_from_candidate(
        candidate,
        binding,
        config_key,
        config,
        CheckpointFinalizationFields {
            council_version: council.version,
            council_hash: council.set_hash,
            approval_bitset: 0b0000_0111,
            finalized_slot: slot,
        },
    );
    prototype.validate_schema()?;
    if compute_state_checkpoint_hard_combined_root_v1(&prototype)? != candidate.hard_combined_root
        || compute_state_checkpoint_digest_v1(&prototype)? != candidate.expected_checkpoint_digest
    {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    Ok(prototype)
}

pub(super) fn validate_checkpoint_acceptance_shape(
    candidate: &CheckpointCandidateV1,
) -> GovernanceResult<()> {
    // Prestate and emergency resolution are admissibility proofs and therefore
    // cannot be stored as rejected evidence.  Poststate has a typed rejected
    // lane for rollback, but finalization validates its concrete mismatch
    // against the accepted Prestate before creating the checkpoint PDA.
    if candidate.phase != StateCheckpointPhaseV1::Poststate && candidate.forbidden_drift_count != 0
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(())
}

fn build_attestation(
    context: &AttestationBuildContext<'_>,
) -> Result<CheckpointAttestationV1, ProgramError> {
    let mut attestation = CheckpointAttestationV1 {
        discriminator: CHECKPOINT_ATTESTATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: context.attestation_bump,
        initialized: true,
        controller_program: *context.program_id,
        controller_config: context.config_key,
        checkpoint: context.checkpoint,
        subject: context.subject,
        subject_digest: context.candidate.expected_subject_digest,
        phase: context.candidate.phase,
        checkpoint_digest: context.candidate.expected_checkpoint_digest,
        council: context.council_key,
        council_version: context.council.version,
        council_hash: context.council.set_hash,
        gate_epoch: context.candidate.expected_gate_epoch,
        seat_index: context.seat_index,
        seat_authority: context.seat_authority,
        attested_slot: context.slot,
        attestation_digest: [0; 32],
        reserved: [0; CHECKPOINT_ATTESTATION_V1_RESERVED_LEN],
    };
    attestation.attestation_digest = compute_checkpoint_attestation_digest_v1(&attestation)?;
    validate_checkpoint_attestation_digest_v1(&attestation)?;
    Ok(attestation)
}

fn create_attestation_pda<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    attestation_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    attestation: &CheckpointAttestationV1,
) -> ProgramResult {
    let council_version = attestation.council_version.to_le_bytes();
    let seat_index = [attestation.seat_index];
    let bump = [attestation.bump];
    let seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        CHECKPOINT_ATTESTATION_SEED,
        attestation.checkpoint.as_ref(),
        &council_version,
        &seat_index,
        &bump,
    ];
    let encoded = encode_fixed_account(attestation, CheckpointAttestationV1::LEN)?;
    create_fixed_pda_account(
        program_id,
        payer,
        attestation_info,
        system_program_info,
        &Rent::get()?,
        CheckpointAttestationV1::LEN,
        seeds,
    )?;
    let mut data = attestation_info.try_borrow_mut_data()?;
    data.copy_from_slice(&encoded);
    Ok(())
}

pub fn process_create_checkpoint_attestation_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CreateCheckpointAttestationV1,
) -> ProgramResult {
    exact_account_count(accounts, 10)?;
    all_distinct(accounts)?;
    let [payer, config_info, policy_info, council_info, gate_info, subject_info, checkpoint_info, attestation_info, seat_authority, system_program_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    validate_signer_writable(payer)?;
    for readonly in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        subject_info,
        checkpoint_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(attestation_info)?;
    validate_signer_readonly(seat_authority)?;
    validate_system_program(system_program_info)?;
    require_absent_system_account(checkpoint_info)?;
    require_absent_system_account(attestation_info)?;

    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info.key, &config, slot)?;
    let council = load_current_council(
        program_id,
        council_info,
        config_info.key,
        &config,
        &policy,
        slot,
    )?;
    let gate = load_gate(program_id, gate_info, config_info.key, &config)?;
    validate_checkpoint_council_guard(
        instruction.expected_council_version,
        &instruction.expected_council_hash,
        &council,
    )?;
    validate_gate_guard(
        &gate,
        instruction.candidate.expected_gate_status,
        instruction.candidate.expected_gate_epoch,
    )?;
    validate_current_seat_at_index(&council, instruction.seat_index, seat_authority.key, slot)?;
    let subject_context = CheckpointSubjectContext {
        program_id,
        subject_info,
        checkpoint_info,
        config_key: config_info.key,
        config: &config,
        gate_key: gate_info.key,
        gate: &gate,
        candidate: &instruction.candidate,
    };
    let binding = validate_checkpoint_subject(&subject_context)?;
    validate_checkpoint_candidate(
        &instruction.candidate,
        &binding,
        *config_info.key,
        &config,
        &council,
        slot,
    )?;
    let (expected_attestation, attestation_bump) = derive_checkpoint_attestation_pda(
        program_id,
        &binding.checkpoint,
        council.version,
        instruction.seat_index,
    );
    if *attestation_info.key != expected_attestation {
        return Err(GovernanceError::InvalidPda.into());
    }
    let attestation = build_attestation(&AttestationBuildContext {
        program_id,
        attestation_bump,
        checkpoint: binding.checkpoint,
        subject: binding.subject_key,
        candidate: &instruction.candidate,
        council_key: *council_info.key,
        council: &council,
        seat_index: instruction.seat_index,
        seat_authority: *seat_authority.key,
        slot,
        config_key: *config_info.key,
    })?;
    create_attestation_pda(
        program_id,
        payer,
        attestation_info,
        system_program_info,
        &attestation,
    )
}

pub fn process_recast_checkpoint_attestation_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: RecastCheckpointAttestationV1,
) -> ProgramResult {
    exact_account_count(accounts, 8)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, subject_info, checkpoint_info, attestation_info, seat_authority] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for readonly in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        subject_info,
        checkpoint_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(attestation_info)?;
    validate_signer_readonly(seat_authority)?;
    require_absent_system_account(checkpoint_info)?;

    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info.key, &config, slot)?;
    let council = load_current_council(
        program_id,
        council_info,
        config_info.key,
        &config,
        &policy,
        slot,
    )?;
    let gate = load_gate(program_id, gate_info, config_info.key, &config)?;
    validate_checkpoint_council_guard(
        instruction.expected_council_version,
        &instruction.expected_council_hash,
        &council,
    )?;
    validate_gate_guard(
        &gate,
        instruction.candidate.expected_gate_status,
        instruction.candidate.expected_gate_epoch,
    )?;
    validate_current_seat_at_index(&council, instruction.seat_index, seat_authority.key, slot)?;
    let subject_context = CheckpointSubjectContext {
        program_id,
        subject_info,
        checkpoint_info,
        config_key: config_info.key,
        config: &config,
        gate_key: gate_info.key,
        gate: &gate,
        candidate: &instruction.candidate,
    };
    let binding = validate_checkpoint_subject(&subject_context)?;
    validate_checkpoint_candidate(
        &instruction.candidate,
        &binding,
        *config_info.key,
        &config,
        &council,
        slot,
    )?;

    let current_attestation = load_fixed_controller_account::<CheckpointAttestationV1>(
        program_id,
        attestation_info,
        CheckpointAttestationV1::LEN,
    )?;
    validate_checkpoint_attestation_digest_v1(&current_attestation)?;
    let (expected_attestation, bump) = derive_checkpoint_attestation_pda(
        program_id,
        &binding.checkpoint,
        council.version,
        instruction.seat_index,
    );
    if *attestation_info.key != expected_attestation
        || current_attestation.bump != bump
        || current_attestation.controller_program != *program_id
        || current_attestation.controller_config != *config_info.key
        || current_attestation.checkpoint != binding.checkpoint
        || current_attestation.subject != binding.subject_key
        || current_attestation.council != *council_info.key
        || current_attestation.council_version != council.version
        || current_attestation.council_hash != council.set_hash
        || current_attestation.gate_epoch != gate.epoch
        || current_attestation.seat_index != instruction.seat_index
        || current_attestation.seat_authority != *seat_authority.key
        || current_attestation.attestation_digest
            != instruction.expected_previous_attestation_digest
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let recast = build_attestation(&AttestationBuildContext {
        program_id,
        attestation_bump: bump,
        checkpoint: binding.checkpoint,
        subject: binding.subject_key,
        candidate: &instruction.candidate,
        council_key: *council_info.key,
        council: &council,
        seat_index: instruction.seat_index,
        seat_authority: *seat_authority.key,
        slot,
        config_key: *config_info.key,
    })?;
    store_fixed_controller_account(
        program_id,
        attestation_info,
        &recast,
        CheckpointAttestationV1::LEN,
    )
}
