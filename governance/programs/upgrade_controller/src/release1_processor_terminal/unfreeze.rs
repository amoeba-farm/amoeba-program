use super::*;

/// Records one current-council approval in the proposal's dedicated unfreeze
/// accumulator. A council rotation invalidates the old accumulator and starts
/// a fresh one; proposal and checkpoint approvals are never reused.
pub fn process_approve_unfreeze_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ApproveUnfreezeV1,
) -> ProgramResult {
    let [config_info, policy_info, council_info, gate_info, proposal_info, poststate_info, verification_info, target_program, target_programdata, authority_info, loader_info, seat] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    for account in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        poststate_info,
        verification_info,
        target_programdata,
        authority_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(target_program, false, false, true)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_seat_authority(seat)?;
    require_distinct_accounts(&[
        config_info,
        policy_info,
        council_info,
        gate_info,
        proposal_info,
        poststate_info,
        verification_info,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        seat,
    ])?;
    if *loader_info.key != UPGRADEABLE_LOADER_ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
    let council = load_current_council(program_id, council_info, config_info, &config, &policy)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let mut proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    let slot = Clock::get()?.slot;
    require_policy_and_council_active(&policy, &council, slot)?;
    validate_unfreeze_expectation(
        &instruction.expected,
        &proposal,
        &config,
        &policy,
        &council,
        &gate,
        proposal_info.key,
    )?;
    if !matches!(
        proposal.state,
        ProposalStateV2::PoststateAccepted | ProposalStateV2::UnfreezeApproved
    ) || *seat.key == config.guardian
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }

    let verification = load_verified_programdata(
        program_id,
        verification_info,
        proposal_info,
        config_info,
        &config,
        &proposal,
    )?;
    let poststate = load_accepted_poststate(
        program_id,
        poststate_info,
        proposal_info,
        config_info,
        &config,
        &proposal,
        &gate,
        &verification,
    )?;
    validate_unfreeze_evidence_expectation(
        &instruction.expected,
        &poststate,
        &verification,
        &config,
    )?;
    validate_live_verified_programdata(
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        &config,
        &proposal,
        &verification,
    )?;

    prepare_unfreeze_accumulator(&mut proposal, &council)?;
    if proposal.unfreeze_approval_count == 0 {
        proposal.unfreeze_council_version = council.version;
        proposal.unfreeze_council_hash = council.set_hash;
    }
    let (bitset, count) = record_seat_approval(
        &council,
        proposal.unfreeze_approval_bitset,
        proposal.unfreeze_approval_count,
        seat.key,
        slot,
    )?;
    proposal.unfreeze_approval_bitset = bitset;
    proposal.unfreeze_approval_count = count;
    if count == RELEASE1_APPROVAL_THRESHOLD {
        proposal.state = ProposalStateV2::UnfreezeApproved;
        proposal.unfreeze_approved_slot = slot;
    }
    validate_proposal_digest_v2(&proposal)?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        UpgradeProposalV2::LEN,
    )
}

/// Executes only the separate, exact unfreeze transaction. It performs no CPI,
/// increments the epoch, clears every freeze field, completes the active
/// proposal, and terminalizes exactly the reciprocal linked proposal.
pub fn process_execute_unfreeze_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExecuteUnfreezeV1,
) -> ProgramResult {
    let [config_info, policy_info, council_info, gate_info, proposal_info, linked_info, poststate_info, verification_info, target_program, target_programdata, authority_info, loader_info, instructions_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    for account in [
        config_info,
        policy_info,
        council_info,
        poststate_info,
        verification_info,
        target_programdata,
        authority_info,
        instructions_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(gate_info, true, false, false)?;
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(linked_info, true, false, false)?;
    validate_exact_privileges(target_program, false, false, true)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    require_distinct_accounts(&[
        config_info,
        policy_info,
        council_info,
        gate_info,
        proposal_info,
        linked_info,
        poststate_info,
        verification_info,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        instructions_info,
    ])?;
    if *loader_info.key != UPGRADEABLE_LOADER_ID
        || *instructions_info.key != sysvar_ids::instructions::ID
        || instruction.linked_proposal != *linked_info.key
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
    let council = load_current_council(program_id, council_info, config_info, &config, &policy)?;
    let mut gate = load_gate(program_id, gate_info, config_info, &config)?;
    let mut proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    let mut linked = load_proposal(program_id, linked_info, config_info, &config)?;
    let slot = Clock::get()?.slot;
    require_policy_and_council_active(&policy, &council, slot)?;
    validate_unfreeze_expectation(
        &instruction.expected,
        &proposal,
        &config,
        &policy,
        &council,
        &gate,
        proposal_info.key,
    )?;
    if proposal.state != ProposalStateV2::UnfreezeApproved {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_exact_current_quorum(
        &council,
        proposal.unfreeze_council_version,
        &proposal.unfreeze_council_hash,
        proposal.unfreeze_approval_bitset,
        proposal.unfreeze_approval_count,
        slot,
    )?;

    let verification = load_verified_programdata(
        program_id,
        verification_info,
        proposal_info,
        config_info,
        &config,
        &proposal,
    )?;
    let poststate = load_accepted_poststate(
        program_id,
        poststate_info,
        proposal_info,
        config_info,
        &config,
        &proposal,
        &gate,
        &verification,
    )?;
    validate_unfreeze_evidence_expectation(
        &instruction.expected,
        &poststate,
        &verification,
        &config,
    )?;
    validate_live_verified_programdata(
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        &config,
        &proposal,
        &verification,
    )?;
    validate_canonical_envelope(
        program_id,
        accounts,
        instructions_info,
        &instruction.pack(),
        &instruction.envelope,
    )?;

    let next_epoch = gate
        .epoch
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    terminalize_unfreeze_pair(
        &proposal,
        proposal_info.key,
        &mut linked,
        linked_info.key,
        slot,
    )?;
    proposal.state = ProposalStateV2::Completed;
    proposal.terminal_slot = slot;
    proposal.terminal_reason_code = PROPOSAL_COMPLETED_TERMINAL_REASON_V1;
    validate_proposal_digest_v2(&proposal)?;
    validate_proposal_digest_v2(&linked)?;

    gate.epoch = next_epoch;
    gate.status = GateStatusV1::Active;
    gate.active_proposal = Pubkey::default();
    gate.freeze_slot = 0;
    gate.freeze_reason_code = 0;
    gate.last_completed_proposal = *proposal_info.key;
    gate.validate_static()?;
    let gate_bytes = encode_fixed_account(&*gate, ProtocolGateV1::LEN)?;
    let proposal_bytes = encode_fixed_account(&*proposal, UpgradeProposalV2::LEN)?;
    let linked_bytes = encode_fixed_account(&*linked, UpgradeProposalV2::LEN)?;
    commit_three_fixed_accounts(
        program_id,
        gate_info,
        &gate_bytes,
        ProtocolGateV1::LEN,
        proposal_info,
        &proposal_bytes,
        UpgradeProposalV2::LEN,
        linked_info,
        &linked_bytes,
        UpgradeProposalV2::LEN,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_unfreeze_expectation(
    expected: &UnfreezeExpectationV1,
    proposal: &UpgradeProposalV2,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
    council: &GovernanceCouncilSetV1,
    gate: &ProtocolGateV1,
    proposal_key: &Pubkey,
) -> ProgramResult {
    if expected.expected_proposal_digest != proposal.proposal_digest
        || expected.expected_policy_version != policy.version
        || expected.expected_policy_hash != policy.policy_hash
        || expected.expected_policy_version != proposal.policy_version
        || expected.expected_policy_hash != proposal.policy_hash
        || expected.expected_current_council_version != council.version
        || expected.expected_current_council_hash != council.set_hash
        || expected.expected_frozen_gate_epoch != gate.epoch
        || expected.expected_target_nonce != config.target_nonce
        || expected.expected_proposal_state != proposal.state
        || expected.expected_unfreeze_approval_bitset != proposal.unfreeze_approval_bitset
        || expected.expected_unfreeze_approval_count != proposal.unfreeze_approval_count
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_frozen_proposal_binding(proposal, proposal_key, config, gate)
}

fn validate_unfreeze_evidence_expectation(
    expected: &UnfreezeExpectationV1,
    poststate: &StateCheckpointV1,
    verification: &ProgramDataVerificationV1,
    config: &ControllerConfigV1,
) -> ProgramResult {
    if expected.expected_poststate_checkpoint_digest != poststate.checkpoint_digest
        || expected.expected_programdata_authority != config.authority_pda
        || expected.expected_programdata_deployed_slot != verification.deployed_slot
        || expected.expected_programdata_capacity != verification.capacity
        || expected.expected_raw_programdata_hash != verification.raw_programdata_hash
        || expected.expected_programdata_verification_finalized_slot != verification.finalized_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

fn require_exact_current_quorum(
    council: &GovernanceCouncilSetV1,
    approval_council_version: u64,
    approval_council_hash: &[u8; 32],
    bitset: u8,
    count: u8,
    slot: u64,
) -> ProgramResult {
    if approval_council_version != council.version
        || approval_council_hash != &council.set_hash
        || bitset & !VALID_APPROVAL_MASK != 0
        || bitset.count_ones() as u8 != count
        || count != RELEASE1_APPROVAL_THRESHOLD
        || !council.active_at(slot)
    {
        return Err(GovernanceError::QuorumNotSatisfied.into());
    }
    for (index, seat) in council.seats.iter().enumerate() {
        if bitset & (1u8 << index) != 0 && !seat.term_covers(slot) {
            return Err(GovernanceError::InactiveCouncilSeat.into());
        }
    }
    Ok(())
}

pub(super) fn prepare_unfreeze_accumulator(
    proposal: &mut UpgradeProposalV2,
    council: &GovernanceCouncilSetV1,
) -> ProgramResult {
    let stale_accumulator = proposal.unfreeze_approval_count != 0
        && (proposal.unfreeze_council_version != council.version
            || proposal.unfreeze_council_hash != council.set_hash);
    if stale_accumulator {
        proposal.unfreeze_council_version = 0;
        proposal.unfreeze_council_hash = [0; 32];
        proposal.unfreeze_approval_bitset = 0;
        proposal.unfreeze_approval_count = 0;
        proposal.unfreeze_approved_slot = 0;
        proposal.state = ProposalStateV2::PoststateAccepted;
    } else if proposal.state == ProposalStateV2::UnfreezeApproved {
        return Err(GovernanceError::DuplicateApproval.into());
    }
    Ok(())
}

fn load_verified_programdata(
    program_id: &Pubkey,
    verification_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV2,
) -> Result<Box<ProgramDataVerificationV1>, ProgramError> {
    let verification = load_programdata_verification_for_failure(
        program_id,
        verification_info,
        proposal_info,
        config_info,
        config,
        proposal,
    )?;
    if verification.status != ProgramDataVerificationStatusV1::Verified
        || !verification.zero_tail_verified
        || verification.raw_programdata_hash == [0; 32]
        || verification.finalized_slot == 0
        || proposal.programdata_verified_slot != verification.finalized_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    Ok(verification)
}

#[allow(clippy::too_many_arguments)]
fn load_accepted_poststate(
    program_id: &Pubkey,
    checkpoint_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV2,
    gate: &ProtocolGateV1,
    verification: &ProgramDataVerificationV1,
) -> Result<Box<StateCheckpointV1>, ProgramError> {
    let checkpoint = load_fixed_controller_account::<StateCheckpointV1>(
        program_id,
        checkpoint_info,
        StateCheckpointV1::LEN,
    )?;
    validate_state_checkpoint_digest_v1(&checkpoint)?;
    if derive_checkpoint_pda(program_id, proposal_info.key, CheckpointPhaseV1::Poststate)
        != (*checkpoint_info.key, checkpoint.bump)
        || *checkpoint_info.key != proposal.required_poststate_checkpoint
        || checkpoint.phase != StateCheckpointPhaseV1::Poststate
        || checkpoint.controller_config != *config_info.key
        || checkpoint.proposal != *proposal_info.key
        || checkpoint.emergency_resolution != Pubkey::default()
        || checkpoint.subject_digest != proposal.proposal_digest
        || checkpoint.target_program != config.target_program
        || checkpoint.target_programdata != config.target_programdata
        || checkpoint.gate_epoch != gate.epoch
        || checkpoint.target_programdata_slot != verification.deployed_slot
        || checkpoint.target_raw_programdata_commitment != verification.raw_programdata_hash
        || checkpoint.target_capacity != verification.capacity
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

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_live_verified_programdata(
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    authority_info: &AccountInfo<'_>,
    loader_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV2,
    verification: &ProgramDataVerificationV1,
) -> ProgramResult {
    if *target_program.key != config.target_program
        || *target_programdata.key != config.target_programdata
        || *authority_info.key != config.authority_pda
        || *loader_info.key != config.upgradeable_loader
        || *loader_info.key != UPGRADEABLE_LOADER_ID
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let header = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &config.upgradeable_loader,
    )?;
    let data = target_programdata.try_borrow_data()?;
    if u64::try_from(data.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?
        > MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
        || header.upgrade_authority != Some(config.authority_pda)
        || header.deployed_slot != verification.deployed_slot
        || header.capacity as u64 != verification.capacity
        || verification.capacity != proposal.expected_post_capacity
        || loader_account_data_hash(&data) != verification.raw_programdata_hash
    {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    Ok(())
}

pub(super) fn terminalize_unfreeze_pair(
    proposal: &UpgradeProposalV2,
    proposal_key: &Pubkey,
    linked: &mut UpgradeProposalV2,
    linked_key: &Pubkey,
    slot: u64,
) -> ProgramResult {
    match proposal.proposal_class {
        ProposalClassV1::EmergencyRollback => {
            if !proposal.primary_proposal.present
                || proposal.primary_proposal.value != *linked_key
                || linked.proposal_class == ProposalClassV1::EmergencyRollback
                || !linked.rollback_proposal.present
                || linked.rollback_proposal.value != *proposal_key
                || !linked.rollback_buffer.present
                || linked.rollback_buffer.value != proposal.buffer_pubkey
                || linked.rollback_artifact_sha256 != proposal.artifact_sha256
                || linked.rollback_artifact_chunk_root != proposal.artifact_chunk_merkle_root
                || linked.target_nonce != proposal.target_nonce
                || linked.checkpoint_schema_id != proposal.checkpoint_schema_id
                || linked.checkpoint_policy_hash != proposal.checkpoint_policy_hash
                || !matches!(
                    linked.state,
                    ProposalStateV2::UpgradeExecuted | ProposalStateV2::ProgramDataVerified
                )
            {
                return Err(GovernanceError::InvalidProposalCommitment.into());
            }
            linked.state = ProposalStateV2::SupersededByRollback;
            linked.terminal_slot = slot;
            linked.terminal_reason_code = PROPOSAL_SUPERSEDED_BY_ROLLBACK_TERMINAL_REASON_V1;
        }
        ProposalClassV1::RoutineUpgrade
        | ProposalClassV1::EconomicChange
        | ProposalClassV1::ConstitutionalChange => {
            if !proposal.rollback_proposal.present
                || proposal.rollback_proposal.value != *linked_key
                || !proposal.rollback_buffer.present
                || proposal.rollback_buffer.value != linked.buffer_pubkey
                || proposal.rollback_artifact_sha256 != linked.artifact_sha256
                || proposal.rollback_artifact_chunk_root != linked.artifact_chunk_merkle_root
                || linked.proposal_class != ProposalClassV1::EmergencyRollback
                || !linked.primary_proposal.present
                || linked.primary_proposal.value != *proposal_key
                || linked.rollback_proposal.present
                || linked.rollback_buffer.present
                || linked.target_nonce != proposal.target_nonce
                || linked.checkpoint_schema_id != proposal.checkpoint_schema_id
                || linked.checkpoint_policy_hash != proposal.checkpoint_policy_hash
                || linked.state != ProposalStateV2::Timelocked
            {
                return Err(GovernanceError::InvalidProposalCommitment.into());
            }
            linked.state = ProposalStateV2::Retired;
            linked.terminal_slot = slot;
            linked.terminal_reason_code = PROPOSAL_RETIRED_ROLLBACK_TERMINAL_REASON_V1;
        }
        ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability => {
            return Err(GovernanceError::UnsupportedProposalClass.into());
        }
    }
    Ok(())
}
