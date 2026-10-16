use super::*;

#[allow(clippy::too_many_arguments)]
fn validate_prepared_rollback_v3(
    program_id: &Pubkey,
    context: &LifecycleContext,
    primary_info: &AccountInfo<'_>,
    primary: &UpgradeProposalV3,
    rollback_info: &AccountInfo<'_>,
    verification_info: &AccountInfo<'_>,
    buffer_info: &AccountInfo<'_>,
    slot: u64,
) -> ProgramResult {
    // The builder carries no duplicate singleton accounts for the rollback, so
    // validate its embedded identities against the already validated primary
    // context after strict decoding and digest verification.
    let rollback = load_upgrade_proposal_v3(program_id, rollback_info)?;
    validate_upgrade_proposal_digest_v3(&rollback)?;
    let rollback_pda = derive_proposal_pda(
        program_id,
        &context.config.target_program,
        rollback.proposal_id,
    );
    if rollback_pda != (*rollback_info.key, rollback.bump)
        || rollback.controller_program != *program_id
        || rollback.controller_config != primary.controller_config
        || rollback.protocol_gate != primary.protocol_gate
        || rollback.capacity_policy != primary.capacity_policy
        || rollback.capacity_policy_digest != context.capacity.policy_digest
        || rollback.current_deployment_state != primary.current_deployment_state
        || rollback.current_deployment_digest != primary.current_deployment_digest
        || rollback.current_deployment_generation != primary.current_deployment_generation
        || rollback.target_program != context.config.target_program
        || rollback.target_programdata != context.config.target_programdata
        || rollback.upgradeable_loader != context.config.upgradeable_loader
        || rollback.authority_pda != context.config.authority_pda
        || rollback.canonical_spill_treasury != context.config.canonical_spill_treasury
        || rollback.maximum_supported_raw_programdata_length
            != context.capacity.maximum_raw_programdata_length
        || rollback.programdata_observation_scheme_id != context.capacity.observation_scheme_id
        || rollback.artifact_scheme_id != context.capacity.artifact_scheme_id
        || rollback.artifact_chunk_size != context.capacity.artifact_chunk_size
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let verification = load_verified_buffer_for_proposal(
        program_id,
        verification_info,
        rollback_info,
        &rollback,
        slot,
    )?;
    verify_exact_proposal_timing(&rollback, &context.config)?;
    if !primary.rollback_proposal.present
        || primary.rollback_proposal.value != *rollback_info.key
        || !primary.rollback_buffer.present
        || primary.rollback_buffer.value != *buffer_info.key
        || rollback.proposal_class != ProposalClassV1::EmergencyRollback
        || !rollback.primary_proposal.present
        || rollback.primary_proposal.value != *primary_info.key
        || rollback.rollback_proposal.present
        || rollback.rollback_buffer.present
        || rollback.state != ProposalStateV2::Timelocked
        || rollback.target_nonce != primary.target_nonce
        || rollback.creation_council_version != primary.creation_council_version
        || rollback.creation_council_hash != primary.creation_council_hash
        || rollback.checkpoint_schema_id != primary.checkpoint_schema_id
        || rollback.checkpoint_policy_hash != primary.checkpoint_policy_hash
        || rollback.buffer_pubkey != *buffer_info.key
        || rollback.buffer_verification != *verification_info.key
        || rollback.programdata_verification
            != derive_programdata_check_pda(program_id, rollback_info.key).0
        || rollback.prestate_checkpoint
            != derive_checkpoint_pda(program_id, rollback_info.key, CheckpointPhaseV1::Prestate).0
        || rollback.required_poststate_checkpoint
            != derive_checkpoint_pda(program_id, rollback_info.key, CheckpointPhaseV1::Poststate).0
        || primary.rollback_artifact_length != rollback.artifact_length
        || primary.rollback_artifact_sha256 != rollback.artifact_sha256
        || primary.rollback_artifact_chunk_root != rollback.artifact_chunk_merkle_root
        || primary.rollback_artifact_scheme_id != rollback.artifact_scheme_id
        || rollback.council_approval_count != RELEASE1_APPROVAL_THRESHOLD
        || rollback.governance_satisfied_slot == 0
        || rollback.queued_slot == 0
        || slot >= rollback.expiry_slot
        || verification.status != BufferVerificationStatusV1::Verified
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let rollback_ready = slot
        .checked_add(context.config.rollback_delay_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let primary_runway = primary
        .expiry_slot
        .checked_add(context.config.rollback_delay_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if rollback_ready >= rollback.expiry_slot || rollback.expiry_slot <= primary_runway {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let header = validate_buffer_account(buffer_info, &context.config.upgradeable_loader)?;
    let data = buffer_info.try_borrow_data()?;
    let header_hash = hashv(&[&data[..LOADER_BUFFER_METADATA_LEN]]).to_bytes();
    if header.authority != Some(context.config.authority_pda)
        || u64::try_from(header.payload_length).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != rollback.artifact_length
        || header_hash != verification.sealed_buffer_header_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn require_freeze_execution_runway(
    needs_extension: bool,
    expiry_slot: u64,
    review_slots: u64,
    slot: u64,
) -> ProgramResult {
    let execution_slots = if needs_extension { 2 } else { 1 };
    let horizon = slot
        .checked_add(review_slots)
        .and_then(|value| value.checked_add(execution_slots))
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if horizon >= expiry_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(())
}

pub fn process_freeze_proposal_v3(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: FreezeProposalV3,
) -> ProgramResult {
    exact_account_count(accounts, 14)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, capacity_info, deployment_info, proposal_info, target_program, target_programdata, loader, authority, rollback_info, rollback_verification_info, rollback_buffer] =
        accounts
    else {
        unreachable!("account count checked")
    };
    writable(config_info)?;
    for info in [policy_info, council_info, capacity_info, deployment_info] {
        readonly(info)?;
    }
    writable(gate_info)?;
    writable(proposal_info)?;
    executable_readonly(target_program)?;
    readonly(target_programdata)?;
    executable_readonly(loader)?;
    for info in [
        authority,
        rollback_info,
        rollback_verification_info,
        rollback_buffer,
    ] {
        readonly(info)?;
    }
    let slot = current_slot()?;
    let mut context = load_lifecycle_context(
        program_id,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
    )?;
    let policy = load_policy(program_id, policy_info, config_info, &context.config, slot)?;
    let mut proposal = load_proposal(
        program_id,
        proposal_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
    )?;
    require_proposal_deployment_current(&proposal, &context.deployment)?;
    let council = load_council(
        program_id,
        council_info,
        config_info,
        &context.config,
        &policy,
        proposal.creation_council_version,
        &proposal.creation_council_hash,
        slot,
        false,
    )?;
    check_proposal_guard(&instruction.expected, &proposal, &context)?;
    require_creation_gate(&proposal, &context.config, &context.gate)?;
    verify_exact_proposal_timing(&proposal, &context.config)?;
    let next_nonce = context
        .config
        .target_nonce
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let next_epoch = checked_nonterminal_increment(context.gate.epoch)?;
    if next_nonce == u64::MAX
        || next_epoch == u64::MAX
        || instruction.expected_next_gate_epoch != next_epoch
        || proposal.state != ProposalStateV2::Timelocked
        || proposal.proposal_class == ProposalClassV1::EmergencyRollback
        || slot < proposal.not_before_slot
        || slot >= proposal.expiry_slot
        || proposal.council_approval_count != RELEASE1_APPROVAL_THRESHOLD
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_exact_recorded_quorum(
        &council,
        proposal.council_approval_bitset,
        proposal.council_approval_count,
        proposal.council_approved_slot,
    )?;
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
    require_runtime_matches_trusted_deployment(&runtime, &context.deployment, &context.config)?;
    require_freeze_execution_runway(
        runtime.capacity < proposal.minimum_required_capacity,
        proposal.expiry_slot,
        context.config.council_review_slots(),
        slot,
    )?;
    validate_prepared_rollback_v3(
        program_id,
        &context,
        proposal_info,
        &proposal,
        rollback_info,
        rollback_verification_info,
        rollback_buffer,
        slot,
    )?;

    context.config.target_nonce = next_nonce;
    context.gate.epoch = next_epoch;
    context.gate.status = GateStatusV1::FrozenForUpgrade;
    context.gate.active_proposal = *proposal_info.key;
    context.gate.freeze_slot = slot;
    context.gate.freeze_reason_code = GOVERNED_UPGRADE_FREEZE_REASON_V3;
    proposal.state = ProposalStateV2::Frozen;
    proposal.freeze_gate_epoch = next_epoch;
    proposal.frozen_slot = slot;
    context.config.validate_static()?;
    context.gate.validate_static()?;
    validate_upgrade_proposal_digest_v3(&proposal)?;
    let config_bytes = encode_fixed_account(&*context.config, ControllerConfigV1::LEN)?;
    let gate_bytes = encode_fixed_account(&*context.gate, ProtocolGateV1::LEN)?;
    let proposal_bytes = encode_fixed_account(&*proposal, UpgradeProposalV3::LEN)?;
    config_info
        .try_borrow_mut_data()?
        .copy_from_slice(&config_bytes);
    gate_info
        .try_borrow_mut_data()?
        .copy_from_slice(&gate_bytes);
    proposal_info
        .try_borrow_mut_data()?
        .copy_from_slice(&proposal_bytes);
    Ok(())
}

pub fn process_cancel_proposal_v3(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CancelProposalV3,
) -> ProgramResult {
    exact_account_count(accounts, 8)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, capacity_info, deployment_info, proposal_info, seat] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for info in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        deployment_info,
    ] {
        readonly(info)?;
    }
    writable(proposal_info)?;
    validate_seat_authority(seat)?;
    let slot = current_slot()?;
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
    let mut proposal = load_proposal(
        program_id,
        proposal_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
    )?;
    check_proposal_guard(&instruction.expected, &proposal, &context)?;
    if !is_prefreeze_state(proposal.state)
        || slot >= proposal.expiry_slot
        || instruction.cancellation_reason_code == 0
        || instruction.expected_cancellation_approval_bitset
            != proposal.cancellation_approval_bitset
        || instruction.expected_cancellation_approval_count != proposal.cancellation_approval_count
        || *seat.key == context.config.guardian
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let stale = proposal.cancellation_approval_count != 0
        && (proposal.cancellation_council_version != council.version
            || proposal.cancellation_council_hash != council.set_hash);
    if proposal.cancellation_approval_count == 0 {
        if instruction.expected_cancellation_council_version != council.version
            || instruction.expected_cancellation_council_hash != council.set_hash
        {
            return Err(GovernanceError::StaleCouncilVersion.into());
        }
    } else if instruction.expected_cancellation_council_version
        != proposal.cancellation_council_version
        || instruction.expected_cancellation_council_hash != proposal.cancellation_council_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    if stale {
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
    validate_upgrade_proposal_digest_v3(&proposal)?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        UpgradeProposalV3::LEN,
    )
}

pub fn process_expire_proposal_v3(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExpireProposalV3,
) -> ProgramResult {
    exact_account_count(accounts, 5)?;
    all_distinct(accounts)?;
    let [config_info, gate_info, capacity_info, deployment_info, proposal_info] = accounts else {
        unreachable!("account count checked")
    };
    for info in [config_info, gate_info, capacity_info, deployment_info] {
        readonly(info)?;
    }
    writable(proposal_info)?;
    let context = load_lifecycle_context(
        program_id,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
    )?;
    let mut proposal = load_proposal(
        program_id,
        proposal_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
    )?;
    check_proposal_guard(&instruction.expected, &proposal, &context)?;
    let slot = current_slot()?;
    if !is_prefreeze_state(proposal.state) || slot < proposal.expiry_slot {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    proposal.state = ProposalStateV2::Expired;
    proposal.terminal_slot = slot;
    proposal.terminal_reason_code = PROPOSAL_EXPIRED_TERMINAL_REASON_V1;
    validate_upgrade_proposal_digest_v3(&proposal)?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        UpgradeProposalV3::LEN,
    )
}
