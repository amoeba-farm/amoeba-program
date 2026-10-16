use super::*;

pub fn process_create_checkpoint_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: Box<CreateCheckpointV2>,
) -> ProgramResult {
    exact_account_count(accounts, 16)?;
    all_distinct(accounts)?;
    let [payer, config_info, policy_info, council_info, gate_info, subject_info, linked_primary_or_authority, capacity_info, deployment_info, observation_info, target_program, target_programdata, checkpoint_info, attestation_info, seat, system_program_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    signer_writable(payer)?;
    for info in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        subject_info,
        linked_primary_or_authority,
        capacity_info,
        deployment_info,
        observation_info,
        target_programdata,
        checkpoint_info,
    ] {
        readonly(info)?;
    }
    executable_readonly(target_program)?;
    writable(attestation_info)?;
    validate_seat_authority(seat)?;
    system_program_account(system_program_info)?;

    let (_, council, binding, checkpoint, slot) = prepare_checkpoint_attestation(
        program_id,
        &instruction.attestation.manifest,
        instruction.attestation.seat_index,
        config_info,
        policy_info,
        council_info,
        gate_info,
        subject_info,
        linked_primary_or_authority,
        capacity_info,
        deployment_info,
        observation_info,
        target_program,
        target_programdata,
        checkpoint_info,
        seat,
    )?;
    let (expected_attestation, attestation_bump) = derive_checkpoint_attestation_pda(
        program_id,
        &binding.checkpoint,
        council.version,
        instruction.attestation.seat_index,
    );
    if *attestation_info.key != expected_attestation
        || attestation_info.owner != &system_program::ID
        || attestation_info.data_len() != 0
    {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let attestation = build_checkpoint_attestation(
        program_id,
        config_info,
        council_info,
        subject_info,
        &checkpoint,
        &council,
        instruction.attestation.seat_index,
        seat,
        slot,
        attestation_bump,
    )?;
    let bytes = encode_fixed_account(&attestation, CheckpointAttestationV1::LEN)?;
    let council_version = council.version.to_le_bytes();
    let seat_index = [instruction.attestation.seat_index];
    let bump = [attestation_bump];
    let seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        CHECKPOINT_ATTESTATION_SEED,
        binding.checkpoint.as_ref(),
        &council_version,
        &seat_index,
        &bump,
    ];
    create_fixed_pda_account(
        program_id,
        payer,
        attestation_info,
        system_program_info,
        &Rent::get()?,
        CheckpointAttestationV1::LEN,
        seeds,
    )?;
    attestation_info
        .try_borrow_mut_data()?
        .copy_from_slice(&bytes);
    Ok(())
}

pub fn process_recast_checkpoint_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: Box<RecastCheckpointV2>,
) -> ProgramResult {
    exact_account_count(accounts, 14)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, subject_info, linked_primary_or_authority, capacity_info, deployment_info, observation_info, target_program, target_programdata, checkpoint_info, attestation_info, seat] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for info in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        subject_info,
        linked_primary_or_authority,
        capacity_info,
        deployment_info,
        observation_info,
        target_programdata,
        checkpoint_info,
    ] {
        readonly(info)?;
    }
    executable_readonly(target_program)?;
    writable(attestation_info)?;
    validate_seat_authority(seat)?;

    let (_, council, binding, checkpoint, slot) = prepare_checkpoint_attestation(
        program_id,
        &instruction.attestation.manifest,
        instruction.attestation.seat_index,
        config_info,
        policy_info,
        council_info,
        gate_info,
        subject_info,
        linked_primary_or_authority,
        capacity_info,
        deployment_info,
        observation_info,
        target_program,
        target_programdata,
        checkpoint_info,
        seat,
    )?;
    let current = load_fixed_controller_account::<CheckpointAttestationV1>(
        program_id,
        attestation_info,
        CheckpointAttestationV1::LEN,
    )?;
    validate_checkpoint_attestation_digest_v1(&current)?;
    let (expected_attestation, bump) = derive_checkpoint_attestation_pda(
        program_id,
        &binding.checkpoint,
        council.version,
        instruction.attestation.seat_index,
    );
    if *attestation_info.key != expected_attestation
        || current.bump != bump
        || current.controller_program != *program_id
        || current.controller_config != *config_info.key
        || current.checkpoint != binding.checkpoint
        || current.subject != *subject_info.key
        || current.council != *council_info.key
        || current.council_version != council.version
        || current.council_hash != council.set_hash
        || current.gate_epoch != checkpoint.gate_epoch
        || current.seat_index != instruction.attestation.seat_index
        || current.seat_authority != *seat.key
        || current.attestation_digest
            != instruction.attestation.expected_previous_attestation_digest
        || instruction.attestation.manifest.previous_checkpoint_digest != current.checkpoint_digest
        || current.checkpoint_digest == checkpoint.checkpoint_digest
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let recast = build_checkpoint_attestation(
        program_id,
        config_info,
        council_info,
        subject_info,
        &checkpoint,
        &council,
        instruction.attestation.seat_index,
        seat,
        slot,
        bump,
    )?;
    store_fixed_controller_account(
        program_id,
        attestation_info,
        &recast,
        CheckpointAttestationV1::LEN,
    )
}

pub fn process_finalize_checkpoint_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: Box<FinalizeCheckpointV2>,
) -> ProgramResult {
    exact_account_count(accounts, 17)?;
    all_distinct(accounts)?;
    let [payer, config_info, policy_info, council_info, gate_info, subject_info, linked_primary_or_authority, capacity_info, deployment_info, observation_info, target_program, target_programdata, checkpoint_info, first_attestation, second_attestation, third_attestation, system_program_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    signer_writable(payer)?;
    for info in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        linked_primary_or_authority,
        capacity_info,
        deployment_info,
        observation_info,
        target_programdata,
        first_attestation,
        second_attestation,
        third_attestation,
    ] {
        readonly(info)?;
    }
    match instruction.manifest.phase {
        StateCheckpointPhaseV1::Prestate => readonly(subject_info)?,
        StateCheckpointPhaseV1::Poststate | StateCheckpointPhaseV1::Emergency => {
            writable(subject_info)?
        }
    }
    executable_readonly(target_program)?;
    writable(checkpoint_info)?;
    system_program_account(system_program_info)?;
    if checkpoint_info.owner != &system_program::ID || checkpoint_info.data_len() != 0 {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }

    let manifest = &instruction.manifest;
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
    let mut checkpoint = derive_checkpoint_candidate(
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
    let mut bitset = 0u8;
    let mut previous_index = None;
    for attestation_info in [first_attestation, second_attestation, third_attestation] {
        let attestation = load_fixed_controller_account::<CheckpointAttestationV1>(
            program_id,
            attestation_info,
            CheckpointAttestationV1::LEN,
        )?;
        validate_checkpoint_attestation_digest_v1(&attestation)?;
        let expected = derive_checkpoint_attestation_pda(
            program_id,
            &binding.checkpoint,
            council.version,
            attestation.seat_index,
        );
        let seat = council
            .seats
            .get(usize::from(attestation.seat_index))
            .ok_or(GovernanceError::InactiveCouncilSeat)?;
        if expected != (*attestation_info.key, attestation.bump)
            || attestation.controller_program != *program_id
            || attestation.controller_config != *config_info.key
            || attestation.checkpoint != binding.checkpoint
            || attestation.subject != *subject_info.key
            || attestation.subject_digest != binding.subject_digest
            || attestation.phase != manifest.phase
            || attestation.checkpoint_digest != checkpoint.checkpoint_digest
            || attestation.council != *council_info.key
            || attestation.council_version != council.version
            || attestation.council_hash != council.set_hash
            || attestation.gate_epoch != context.gate.epoch
            || attestation.seat_authority != seat.seat_authority
            || !seat.term_covers(attestation.attested_slot)
            || attestation.attested_slot < observation.finalized_slot
            || attestation.attested_slot > slot
            || previous_index.is_some_and(|index| attestation.seat_index <= index)
        {
            return Err(GovernanceError::CrossAccountMismatch.into());
        }
        bitset |= 1u8 << attestation.seat_index;
        previous_index = Some(attestation.seat_index);
    }
    if bitset.count_ones() as u8 != RELEASE1_APPROVAL_THRESHOLD {
        return Err(GovernanceError::QuorumNotSatisfied.into());
    }
    checkpoint.approval_bitset = bitset;
    checkpoint.approval_count = RELEASE1_APPROVAL_THRESHOLD;
    checkpoint.accepted = true;
    checkpoint.finalized_slot = slot;
    validate_state_checkpoint_digest_v2(&checkpoint)?;
    let checkpoint_bytes = encode_fixed_account(&*checkpoint, StateCheckpointV2::LEN)?;

    let subject_bytes = match binding.record {
        CheckpointSubjectRecord::Proposal(mut proposal) => {
            if manifest.phase == StateCheckpointPhaseV1::Poststate {
                proposal.state = ProposalStateV2::PoststateAccepted;
                proposal.poststate_accepted_slot = slot;
                validate_upgrade_proposal_digest_v3(&proposal)?;
                Some(encode_fixed_account(&*proposal, UpgradeProposalV3::LEN)?)
            } else {
                None
            }
        }
        CheckpointSubjectRecord::Emergency(mut resolution) => {
            resolution.emergency_checkpoint_digest = checkpoint.checkpoint_digest;
            validate_emergency_freeze_resolution_digest_v2(&resolution)?;
            Some(encode_fixed_account(
                &*resolution,
                EmergencyFreezeResolutionV2::LEN,
            )?)
        }
    };

    let bump = [binding.bump];
    let phase = [manifest.phase as u8];
    let proposal_seeds;
    let emergency_seeds;
    let seeds: &[&[u8]] = match manifest.phase {
        StateCheckpointPhaseV1::Prestate | StateCheckpointPhaseV1::Poststate => {
            proposal_seeds = [
                UPGRADE_SEED_DOMAIN_V1,
                CHECKPOINT_SEED,
                subject_info.key.as_ref(),
                &phase,
                &bump,
            ];
            &proposal_seeds
        }
        StateCheckpointPhaseV1::Emergency => {
            emergency_seeds = [
                UPGRADE_SEED_DOMAIN_V1,
                EMERGENCY_CHECKPOINT_V2_SEED,
                subject_info.key.as_ref(),
                &bump,
            ];
            &emergency_seeds
        }
    };
    create_fixed_pda_account(
        program_id,
        payer,
        checkpoint_info,
        system_program_info,
        &Rent::get()?,
        StateCheckpointV2::LEN,
        seeds,
    )?;
    checkpoint_info
        .try_borrow_mut_data()?
        .copy_from_slice(&checkpoint_bytes);
    if let Some(bytes) = subject_bytes {
        subject_info.try_borrow_mut_data()?.copy_from_slice(&bytes);
    }
    Ok(())
}
