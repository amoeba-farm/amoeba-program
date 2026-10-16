use super::*;

pub fn process_freeze_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: FreezeProposalV2,
) -> ProgramResult {
    let [config_info, policy_info, council_info, gate_info, proposal_info, target_program, target_programdata, loader, authority, rollback_info, rollback_verification_info, rollback_buffer] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_exact_privileges(config_info, true, false, false)?;
    validate_readonly_state_accounts(&[policy_info, council_info])?;
    validate_exact_privileges(gate_info, true, false, false)?;
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(target_program, false, false, true)?;
    for account in [
        target_programdata,
        authority,
        rollback_info,
        rollback_verification_info,
        rollback_buffer,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(loader, false, false, true)?;
    require_distinct_accounts(&[
        config_info,
        policy_info,
        council_info,
        gate_info,
        proposal_info,
        target_program,
        target_programdata,
        loader,
        authority,
        rollback_info,
        rollback_verification_info,
        rollback_buffer,
    ])?;
    freeze_or_convert(
        program_id,
        FreezeInputs {
            config_info,
            policy_info,
            council_info,
            gate_info,
            proposal_info,
            target_program,
            target_programdata,
            loader,
            authority,
            rollback_info,
            rollback_verification_info,
            rollback_buffer,
            emergency_observation: None,
        },
        &instruction.expected,
        instruction.expected_next_gate_epoch,
        None,
    )
}

pub fn process_cancel_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CancelProposalV2,
) -> ProgramResult {
    let [config_info, policy_info, council_info, gate_info, proposal_info, seat] = accounts else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_readonly_state_accounts(&[config_info, policy_info, council_info, gate_info])?;
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_seat_authority(seat)?;
    require_distinct_accounts(&[
        config_info,
        policy_info,
        council_info,
        gate_info,
        proposal_info,
        seat,
    ])?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
    let council = load_current_council(program_id, council_info, config_info, &config, &policy)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let mut proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    let slot = Clock::get()?.slot;
    require_policy_active(&policy, slot)?;
    check_proposal_expectation(
        &instruction.expected,
        &proposal,
        &config,
        &policy,
        &gate,
        council.version,
        &council.set_hash,
    )?;
    reject_locked_reciprocal_rollback(&proposal, &config, &gate)?;
    if !is_pre_freeze_state(proposal.state)
        || slot >= proposal.expiry_slot
        || instruction.cancellation_reason_code == 0
        || instruction.expected_cancellation_approval_bitset
            != proposal.cancellation_approval_bitset
        || instruction.expected_cancellation_approval_count != proposal.cancellation_approval_count
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    reject_guardian_authority(&config, seat.key)?;
    let stale_accumulator = proposal.cancellation_approval_count != 0
        && (proposal.cancellation_council_version != council.version
            || proposal.cancellation_council_hash != council.set_hash);
    if stale_accumulator {
        proposal.cancellation_council_version = 0;
        proposal.cancellation_council_hash = [0; 32];
        proposal.cancellation_approval_bitset = 0;
        proposal.cancellation_approval_count = 0;
        proposal.cancellation_reason_code = 0;
    }
    if proposal.cancellation_reason_code != 0
        && proposal.cancellation_reason_code != instruction.cancellation_reason_code
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    if proposal.cancellation_approval_count == 0 {
        proposal.cancellation_council_version = council.version;
        proposal.cancellation_council_hash = council.set_hash;
        proposal.cancellation_reason_code = instruction.cancellation_reason_code;
    }
    let (bitset, count) = record_seat_approval(
        &council,
        proposal.cancellation_approval_bitset,
        proposal.cancellation_approval_count,
        seat.key,
        slot,
    )?;
    proposal.cancellation_approval_bitset = bitset;
    proposal.cancellation_approval_count = count;
    if count == RELEASE1_APPROVAL_THRESHOLD {
        proposal.state = ProposalStateV2::Cancelled;
        proposal.terminal_slot = slot;
        proposal.terminal_reason_code = proposal.cancellation_reason_code;
    }
    validate_proposal_digest_v2(&proposal)?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        UpgradeProposalV2::LEN,
    )
}

pub fn process_expire_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExpireProposalV2,
) -> ProgramResult {
    let [config_info, gate_info, proposal_info] = accounts else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_readonly_state_accounts(&[config_info, gate_info])?;
    validate_exact_privileges(proposal_info, true, false, false)?;
    require_distinct_accounts(&[config_info, gate_info, proposal_info])?;
    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let mut proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    check_proposal_expectation_without_policy(
        &instruction.expected,
        &proposal,
        &config,
        &gate,
        proposal.creation_council_version,
        &proposal.creation_council_hash,
    )?;
    reject_locked_reciprocal_rollback(&proposal, &config, &gate)?;
    let slot = Clock::get()?.slot;
    if !is_pre_freeze_state(proposal.state) || slot < proposal.expiry_slot {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    proposal.state = ProposalStateV2::Expired;
    proposal.terminal_slot = slot;
    proposal.terminal_reason_code = PROPOSAL_EXPIRED_TERMINAL_REASON_V1;
    validate_proposal_digest_v2(&proposal)?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        UpgradeProposalV2::LEN,
    )
}

pub(super) fn freeze_or_convert(
    program_id: &Pubkey,
    inputs: FreezeInputs<'_, '_>,
    expectation: &ProposalExpectationV2,
    expected_next_epoch: u64,
    expected_observation_digest: Option<&[u8; 32]>,
) -> ProgramResult {
    let mut config = load_config(program_id, inputs.config_info)?;
    let policy = load_policy(program_id, inputs.policy_info, inputs.config_info, &config)?;
    let mut gate = load_gate(program_id, inputs.gate_info, inputs.config_info, &config)?;
    let mut proposal = load_proposal(
        program_id,
        inputs.proposal_info,
        inputs.config_info,
        &config,
    )?;
    let council = load_pinned_council(
        program_id,
        inputs.council_info,
        inputs.config_info,
        &config,
        &policy,
        proposal.creation_council_version,
        &proposal.creation_council_hash,
    )?;
    let slot = Clock::get()?.slot;
    require_policy_active(&policy, slot)?;
    check_proposal_expectation(
        expectation,
        &proposal,
        &config,
        &policy,
        &gate,
        council.version,
        &council.set_hash,
    )?;
    require_creation_gate(&proposal, &config, &gate)?;
    verify_exact_proposal_timing(&proposal, &config)?;
    let (next_target_nonce, next_epoch) = checked_freeze_counters(config.target_nonce, gate.epoch)?;
    if proposal.state != ProposalStateV2::Timelocked
        || proposal.proposal_class == ProposalClassV1::EmergencyRollback
        || slot < proposal.not_before_slot
        || slot >= proposal.expiry_slot
        || expected_next_epoch != next_epoch
        || proposal.council_approval_count != RELEASE1_APPROVAL_THRESHOLD
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_freeze_execution_runway(
        proposal.extension_delta,
        proposal.expiry_slot,
        config.council_review_slots(),
        slot,
    )?;
    require_exact_recorded_quorum(
        &council,
        proposal.council_approval_bitset,
        proposal.council_approval_count,
        proposal.council_approved_slot,
    )?;
    let converting = inputs.emergency_observation.is_some();
    if converting {
        let observation = load_emergency_observation(
            program_id,
            inputs.emergency_observation.expect("checked"),
            inputs.config_info,
            inputs.gate_info,
            &config,
            &gate,
        )?;
        if proposal.creation_gate_status != GateStatusV1::EmergencyFrozen
            || gate.status != GateStatusV1::EmergencyFrozen
            || gate.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
            || expected_observation_digest != Some(&observation.observation_digest)
        {
            return Err(GovernanceError::InvalidGateState.into());
        }
    } else if proposal.creation_gate_status != GateStatusV1::Active
        || gate.status != GateStatusV1::Active
    {
        return Err(GovernanceError::InvalidGateState.into());
    }

    validate_loader_identity(inputs.loader, &config)?;
    if *inputs.target_program.key != config.target_program
        || *inputs.target_programdata.key != config.target_programdata
        || *inputs.authority.key != config.authority_pda
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let live = read_canonical_programdata_snapshot(
        inputs.target_program,
        inputs.target_programdata,
        &config,
        Some(config.authority_pda),
    )?;
    if live.deployed_slot != proposal.deployed_slot
        || live.capacity != proposal.current_capacity
        || live.raw_hash != proposal.current_raw_programdata_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_prepared_rollback(
        program_id,
        &config,
        inputs.proposal_info,
        &proposal,
        inputs.rollback_info,
        inputs.rollback_verification_info,
        inputs.rollback_buffer,
        slot,
    )?;

    config.target_nonce = next_target_nonce;
    gate.epoch = next_epoch;
    gate.status = GateStatusV1::FrozenForUpgrade;
    gate.active_proposal = *inputs.proposal_info.key;
    gate.freeze_slot = slot;
    gate.freeze_reason_code = GOVERNED_UPGRADE_FREEZE_REASON_V1;
    proposal.state = ProposalStateV2::Frozen;
    proposal.freeze_gate_epoch = next_epoch;
    proposal.frozen_slot = slot;
    config.validate_static()?;
    gate.validate_static()?;
    validate_proposal_digest_v2(&proposal)?;
    let _config_bytes = encode_fixed_account(&*config, ControllerConfigV1::LEN)?;
    let _gate_bytes = encode_fixed_account(&*gate, ProtocolGateV1::LEN)?;
    let _proposal_bytes = encode_fixed_account(&*proposal, UpgradeProposalV2::LEN)?;
    store_fixed_controller_account(
        program_id,
        inputs.config_info,
        &*config,
        ControllerConfigV1::LEN,
    )?;
    store_fixed_controller_account(program_id, inputs.gate_info, &*gate, ProtocolGateV1::LEN)?;
    store_fixed_controller_account(
        program_id,
        inputs.proposal_info,
        &*proposal,
        UpgradeProposalV2::LEN,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_prepared_rollback(
    program_id: &Pubkey,
    config: &ControllerConfigV1,
    primary_info: &AccountInfo<'_>,
    primary: &UpgradeProposalV2,
    rollback_info: &AccountInfo<'_>,
    verification_info: &AccountInfo<'_>,
    buffer_info: &AccountInfo<'_>,
    slot: u64,
) -> ProgramResult {
    let rollback = load_fixed_controller_account::<UpgradeProposalV2>(
        program_id,
        rollback_info,
        UpgradeProposalV2::LEN,
    )?;
    validate_proposal_digest_v2(&rollback)?;
    let verification = load_fixed_controller_account::<BufferVerificationV1>(
        program_id,
        verification_info,
        BufferVerificationV1::LEN,
    )?;
    verification.validate_schema()?;
    verify_exact_proposal_timing(&rollback, config)?;
    let rollback_pda =
        derive_proposal_pda(program_id, &config.target_program, rollback.proposal_id);
    let verification_pda = derive_buffer_check_pda(program_id, rollback_info.key);
    if primary.rollback_proposal.value != *rollback_info.key
        || primary.rollback_buffer.value != *buffer_info.key
        || !primary.rollback_proposal.present
        || !primary.rollback_buffer.present
        || rollback_pda.0 != *rollback_info.key
        || rollback.bump != rollback_pda.1
        || rollback.controller_program != *program_id
        || rollback.cluster_domain != config.cluster_domain
        || rollback.policy_version != config.current_policy_version
        || verification_pda.0 != *verification_info.key
        || verification.bump != verification_pda.1
        || rollback.proposal_class != ProposalClassV1::EmergencyRollback
        || rollback.primary_proposal.value != *primary_info.key
        || !rollback.primary_proposal.present
        || rollback.rollback_proposal.present
        || rollback.rollback_buffer.present
        || rollback.state != ProposalStateV2::Timelocked
        || rollback.target_nonce != primary.target_nonce
        || rollback.checkpoint_schema_id != primary.checkpoint_schema_id
        || rollback.checkpoint_policy_hash != primary.checkpoint_policy_hash
        || rollback.controller_config != primary.controller_config
        || rollback.protocol_gate != primary.protocol_gate
        || rollback.target_program != primary.target_program
        || rollback.target_programdata != primary.target_programdata
        || rollback.upgradeable_loader != primary.upgradeable_loader
        || rollback.authority_pda != primary.authority_pda
        || rollback.canonical_spill_treasury != primary.canonical_spill_treasury
        || rollback.buffer_loader_owner != config.upgradeable_loader
        || rollback.buffer_final_authority != config.authority_pda
        || rollback.buffer_pubkey != *buffer_info.key
        || rollback.buffer_verification != *verification_info.key
        || rollback.programdata_verification
            != derive_programdata_check_pda(program_id, rollback_info.key).0
        || rollback.prestate_checkpoint
            != derive_checkpoint_pda(program_id, rollback_info.key, CheckpointPhaseV1::Prestate).0
        || rollback.required_poststate_checkpoint
            != derive_checkpoint_pda(program_id, rollback_info.key, CheckpointPhaseV1::Poststate).0
        || primary.rollback_artifact_sha256 != rollback.artifact_sha256
        || primary.rollback_artifact_chunk_root != rollback.artifact_chunk_merkle_root
        || rollback.expected_execution_pre_chunk_root != primary.artifact_chunk_merkle_root
        || rollback.current_capacity != primary.expected_post_capacity
        || rollback.expected_post_capacity != primary.expected_post_capacity
        || rollback.extension_delta != 0
        || rollback.council_approval_count != RELEASE1_APPROVAL_THRESHOLD
        || rollback.governance_satisfied_slot == 0
        || rollback.queued_slot == 0
        || slot >= rollback.expiry_slot
        || verification.status != BufferVerificationStatusV1::Verified
        || verification.controller_config != primary.controller_config
        || verification.proposal != *rollback_info.key
        || verification.upgradeable_loader != config.upgradeable_loader
        || verification.buffer != *buffer_info.key
        || verification.expected_uploader_authority != rollback.buffer_uploader_authority
        || verification.controller_authority != config.authority_pda
        || verification.artifact_length != rollback.artifact_length
        || verification.artifact_sha256 != rollback.artifact_sha256
        || verification.artifact_chunk_merkle_root != rollback.artifact_chunk_merkle_root
        || verification.chunk_hash_domain != rollback.chunk_hash_domain
        || verification.chunk_size != rollback.chunk_size
        || verification.chunk_count != rollback.chunk_count
        || verification.finalized_slot > slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let rollback_ready = slot
        .checked_add(config.rollback_delay_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let primary_runway = primary
        .expiry_slot
        .checked_add(config.rollback_delay_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if rollback_ready >= rollback.expiry_slot || rollback.expiry_slot <= primary_runway {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let header = validate_buffer_account(buffer_info, &config.upgradeable_loader)?;
    let data = buffer_info.try_borrow_data()?;
    let header_hash = hashv(&[&data[..LOADER_BUFFER_METADATA_LEN]]).to_bytes();
    if header.authority != Some(config.authority_pda)
        || u64::try_from(header.payload_length).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != rollback.artifact_length
        || header_hash != verification.sealed_buffer_header_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

/// A permissionless caller may submit a fully approved freeze, so the
/// controller must not let that caller wait until the last admissible slot and
/// strand the target.  Preserve one complete council-review window for the
/// protected-state checkpoint, one later execution slot, and one additional
/// slot when checked extension is required because extension and upgrade must
/// be strictly separated.
pub(super) fn require_freeze_execution_runway(
    extension_delta: u64,
    expiry_slot: u64,
    council_review_slots: u64,
    slot: u64,
) -> ProgramResult {
    let execution_slots = if extension_delta == 0 { 1 } else { 2 };
    let last_admissible_freeze_horizon = slot
        .checked_add(council_review_slots)
        .and_then(|value| value.checked_add(execution_slots))
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if last_admissible_freeze_horizon >= expiry_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(())
}
