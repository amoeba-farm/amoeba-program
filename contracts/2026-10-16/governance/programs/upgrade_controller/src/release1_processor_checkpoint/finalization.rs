use super::*;

fn validate_attestation_for_finalization(
    attestation_info: &AccountInfo<'_>,
    context: &AttestationFinalizationContext<'_>,
) -> Result<u8, ProgramError> {
    let attestation = load_fixed_controller_account::<CheckpointAttestationV1>(
        context.program_id,
        attestation_info,
        CheckpointAttestationV1::LEN,
    )?;
    validate_checkpoint_attestation_digest_v1(&attestation)?;
    let (expected, bump) = derive_checkpoint_attestation_pda(
        context.program_id,
        context.checkpoint_key,
        context.council.version,
        attestation.seat_index,
    );
    if *attestation_info.key != expected
        || attestation.bump != bump
        || attestation.controller_program != *context.program_id
        || attestation.controller_config != context.checkpoint.controller_config
        || attestation.checkpoint != *context.checkpoint_key
        || attestation.subject != *context.subject_key
        || attestation.subject_digest != context.checkpoint.subject_digest
        || attestation.phase != context.checkpoint.phase
        || attestation.checkpoint_digest != context.checkpoint.checkpoint_digest
        || attestation.council != *context.council_key
        || attestation.council_version != context.council.version
        || attestation.council_hash != context.council.set_hash
        || attestation.gate_epoch != context.checkpoint.gate_epoch
        || attestation.attested_slot < context.minimum_slot
        || attestation.attested_slot > context.finalization_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_current_seat_at_index(
        context.council,
        attestation.seat_index,
        &attestation.seat_authority,
        attestation.attested_slot,
    )?;
    validate_current_seat_at_index(
        context.council,
        attestation.seat_index,
        &attestation.seat_authority,
        context.finalization_slot,
    )?;
    Ok(attestation.seat_index)
}

fn create_checkpoint_pda_account<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    checkpoint_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    checkpoint: &StateCheckpointV1,
) -> ProgramResult {
    let encoded = encode_fixed_account(checkpoint, StateCheckpointV1::LEN)?;
    match checkpoint.phase {
        StateCheckpointPhaseV1::Prestate | StateCheckpointPhaseV1::Poststate => {
            let phase = [checkpoint.phase as u8];
            let bump = [checkpoint.bump];
            let seeds: &[&[u8]] = &[
                UPGRADE_SEED_DOMAIN_V1,
                CHECKPOINT_SEED,
                checkpoint.proposal.as_ref(),
                &phase,
                &bump,
            ];
            create_fixed_pda_account(
                program_id,
                payer,
                checkpoint_info,
                system_program_info,
                &Rent::get()?,
                StateCheckpointV1::LEN,
                seeds,
            )?;
        }
        StateCheckpointPhaseV1::Emergency => {
            let epoch = checkpoint.gate_epoch.to_le_bytes();
            let bump = [checkpoint.bump];
            let seeds: &[&[u8]] = &[
                UPGRADE_SEED_DOMAIN_V1,
                EMERGENCY_CHECKPOINT_SEED,
                checkpoint.target_program.as_ref(),
                &epoch,
                &bump,
            ];
            create_fixed_pda_account(
                program_id,
                payer,
                checkpoint_info,
                system_program_info,
                &Rent::get()?,
                StateCheckpointV1::LEN,
                seeds,
            )?;
        }
    }
    let mut data = checkpoint_info.try_borrow_mut_data()?;
    data.copy_from_slice(&encoded);
    Ok(())
}

pub fn process_finalize_checkpoint_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: FinalizeCheckpointV1,
) -> ProgramResult {
    let poststate = instruction.candidate.phase == StateCheckpointPhaseV1::Poststate;
    match instruction.candidate.phase {
        StateCheckpointPhaseV1::Poststate if accounts.len() != 15 => {
            return Err(GovernanceError::InvalidAccountCount.into())
        }
        StateCheckpointPhaseV1::Emergency if accounts.len() != 14 => {
            return Err(GovernanceError::InvalidAccountCount.into())
        }
        StateCheckpointPhaseV1::Prestate if !matches!(accounts.len(), 14 | 15) => {
            return Err(GovernanceError::InvalidAccountCount.into())
        }
        _ => {}
    }
    all_distinct(accounts)?;

    let payer = &accounts[0];
    let config_info = &accounts[1];
    let policy_info = &accounts[2];
    let council_info = &accounts[3];
    let gate_info = &accounts[4];
    let subject_info = &accounts[5];
    let target_program = &accounts[6];
    let target_programdata = &accounts[7];
    let evidence_info = &accounts[8];
    let baseline_info = (accounts.len() == 15).then(|| &accounts[9]);
    let checkpoint_index = if baseline_info.is_some() { 10 } else { 9 };
    let checkpoint_info = &accounts[checkpoint_index];
    let attestation_infos = &accounts[checkpoint_index + 1..checkpoint_index + 4];
    let system_program_info = &accounts[checkpoint_index + 4];

    validate_signer_writable(payer)?;
    for readonly in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        evidence_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_exact_privileges(subject_info, poststate, false, false)?;
    validate_exact_privileges(target_program, false, false, true)?;
    validate_readonly(target_programdata)?;
    if let Some(baseline) = baseline_info {
        validate_readonly(baseline)?;
    }
    validate_writable(checkpoint_info)?;
    for attestation in attestation_infos {
        validate_readonly(attestation)?;
    }
    validate_system_program(system_program_info)?;
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
    let mut binding = validate_checkpoint_subject(&subject_context)?;
    let rollback_prestate = matches!(
        &binding.subject,
        CheckpointSubject::Proposal(proposal)
            if instruction.candidate.phase == StateCheckpointPhaseV1::Prestate
                && proposal.proposal_class == ProposalClassV1::EmergencyRollback
    );
    if baseline_info.is_some() != (poststate || rollback_prestate) {
        return Err(GovernanceError::InvalidAccountCount.into());
    }
    let mut checkpoint = validate_checkpoint_candidate(
        &instruction.candidate,
        &binding,
        *config_info.key,
        &config,
        &council,
        slot,
    )?;
    validate_target_snapshot(
        target_program,
        target_programdata,
        &config,
        &instruction.candidate,
    )?;
    let evidence_slot = validate_phase_evidence(
        program_id,
        evidence_info,
        config_info,
        gate_info,
        &config,
        &binding,
        &instruction.candidate,
    )?;
    let baseline_slot = match (baseline_info, poststate, rollback_prestate) {
        (Some(baseline), true, false) => validate_poststate_baseline(
            program_id,
            baseline,
            config_info,
            &config,
            &binding,
            &instruction.candidate,
        )?,
        (Some(baseline), false, true) => validate_rollback_prestate_baseline(
            program_id,
            baseline,
            config_info,
            &config,
            &binding,
            &instruction.candidate,
        )?,
        (None, false, false) => 0,
        _ => return Err(GovernanceError::InvalidAccountCount.into()),
    };
    let minimum_attestation_slot = instruction
        .candidate
        .finalized_observation_slot
        .max(evidence_slot)
        .max(baseline_slot);
    let attestation_context = AttestationFinalizationContext {
        program_id,
        checkpoint: &checkpoint,
        checkpoint_key: &binding.checkpoint,
        subject_key: &binding.subject_key,
        council_key: council_info.key,
        council: &council,
        minimum_slot: minimum_attestation_slot,
        finalization_slot: slot,
    };
    let mut approval_bitset = 0u8;
    for attestation in attestation_infos {
        let seat_index = validate_attestation_for_finalization(attestation, &attestation_context)?;
        let bit = 1u8 << seat_index;
        if approval_bitset & bit != 0 {
            return Err(GovernanceError::DuplicateApproval.into());
        }
        approval_bitset |= bit;
    }
    validate_approval_mask_at(
        &council,
        approval_bitset,
        approval_bitset.count_ones() as u8,
        slot,
    )?;
    checkpoint.approval_bitset = approval_bitset;
    checkpoint.approval_count = approval_bitset.count_ones() as u8;
    checkpoint.finalized_slot = slot;
    validate_state_checkpoint_digest_v1(&checkpoint)?;
    let proposal_bytes = if poststate && checkpoint.accepted {
        let CheckpointSubject::Proposal(proposal) = &mut binding.subject else {
            return Err(GovernanceError::InvalidStateTransition.into());
        };
        // The subject was decoded into a detached heap allocation. Mutate that
        // allocation only after every checkpoint/evidence/quorum check, then
        // encode before the first account write. A second 1,792-byte proposal
        // copy is unnecessary and unsafe for the SBPF-v0 stack.
        proposal.state = ProposalStateV2::PoststateAccepted;
        proposal.poststate_accepted_slot = slot;
        validate_proposal_digest_v2(proposal)?;
        Some(encode_fixed_account(&**proposal, UpgradeProposalV2::LEN)?)
    } else {
        None
    };

    // Account creation is the first mutation.  Every byte and every possible
    // poststate proposal update has already been validated and encoded.
    create_checkpoint_pda_account(
        program_id,
        payer,
        checkpoint_info,
        system_program_info,
        &checkpoint,
    )?;
    if let Some(bytes) = proposal_bytes {
        let mut proposal_data = subject_info.try_borrow_mut_data()?;
        proposal_data.copy_from_slice(&bytes);
    }
    Ok(())
}
