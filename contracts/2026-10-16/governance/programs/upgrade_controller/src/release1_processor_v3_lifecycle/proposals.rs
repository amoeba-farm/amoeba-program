use super::*;

pub fn process_create_proposal_v3(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CreateProposalV3,
) -> ProgramResult {
    exact_account_count(accounts, 17)?;
    all_distinct(accounts)?;
    let [payer, creator, config_info, policy_info, council_info, gate_info, capacity_info, deployment_info, target_program, target_programdata, loader, authority, spill, buffer, uploader, proposal_info, system_program_info] =
        accounts
    else {
        unreachable!("account count checked")
    };

    signer_writable(payer)?;
    validate_seat_authority(creator)?;
    writable(config_info)?;
    for info in [
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        deployment_info,
        authority,
        spill,
        buffer,
        uploader,
    ] {
        readonly(info)?;
    }
    executable_readonly(target_program)?;
    readonly(target_programdata)?;
    executable_readonly(loader)?;
    writable(proposal_info)?;
    system_program_account(system_program_info)?;

    let slot = current_slot()?;
    let mut config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config, slot)?;
    let council = load_current_council(
        program_id,
        council_info,
        config_info,
        &config,
        &policy,
        slot,
    )?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let capacity = load_capacity_policy(program_id, capacity_info, config_info, &config)?;
    let deployment = load_current_deployment(
        program_id,
        deployment_info,
        config_info,
        capacity_info,
        &config,
        &capacity,
    )?;
    if *creator.key == config.guardian
        || !council
            .seats
            .iter()
            .any(|seat| seat.seat_authority == *creator.key && seat.term_covers(slot))
        || !council.active_at(slot)
    {
        return Err(GovernanceError::InactiveCouncilSeat.into());
    }
    if !matches!(
        gate.status,
        GateStatusV1::Active | GateStatusV1::EmergencyFrozen
    ) || gate.active_proposal != Pubkey::default()
        || (gate.status == GateStatusV1::EmergencyFrozen
            && gate.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1)
    {
        return Err(GovernanceError::InvalidGateState.into());
    }
    if *loader.key != config.upgradeable_loader
        || *authority.key != config.authority_pda
        || *spill.key != config.canonical_spill_treasury
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let runtime =
        read_runtime_programdata_header(target_program, target_programdata, &config, &capacity)?;
    require_runtime_matches_trusted_deployment(&runtime, &deployment, &config)?;
    let buffer_header = validate_buffer_account(buffer, &config.upgradeable_loader)?;

    let manifest = &instruction.manifest;
    let expected_proposal =
        derive_proposal_pda(program_id, &config.target_program, config.next_proposal_id);
    let expected_prestate =
        derive_checkpoint_pda(program_id, proposal_info.key, CheckpointPhaseV1::Prestate).0;
    let expected_poststate =
        derive_checkpoint_pda(program_id, proposal_info.key, CheckpointPhaseV1::Poststate).0;
    let (review_start_slot, review_end_slot, not_before_slot, expiry_slot) =
        derive_proposal_timing(&config, manifest.proposal_class, slot)?;
    if slot > manifest.plan_valid_until_slot
        || expected_proposal.0 != *proposal_info.key
        || manifest.expected_proposal_id != config.next_proposal_id
        || manifest.expected_target_nonce != config.target_nonce
        || manifest.expected_gate_status != gate.status
        || manifest.expected_gate_epoch != gate.epoch
        || manifest.expected_capacity_policy_digest != capacity.policy_digest
        || manifest.expected_current_deployment_digest != deployment.deployment_digest
        || manifest.expected_current_deployment_generation != deployment.deployment_generation
        || manifest.expected_policy_version != policy.version
        || manifest.expected_policy_hash != policy.policy_hash
        || manifest.expected_council_version != council.version
        || manifest.expected_council_hash != council.set_hash
        || manifest.artifact_length > capacity.maximum_artifact_length
        || manifest.minimum_required_capacity < manifest.artifact_length
        || manifest.minimum_required_capacity > capacity.maximum_payload_capacity
        || buffer_header.authority != Some(*uploader.key)
        || u64::try_from(buffer_header.payload_length)
            .map_err(|_| GovernanceError::ArithmeticOverflow)?
            != manifest.artifact_length
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let rollback_artifact_scheme_id = if manifest.rollback_proposal.present {
        capacity.artifact_scheme_id
    } else {
        [0; 32]
    };
    let mut candidate = UpgradeProposalV3 {
        discriminator: UPGRADE_PROPOSAL_V3_DISCRIMINATOR,
        account_version: CAPACITY_SAFE_ACCOUNT_VERSION_V3,
        bump: expected_proposal.1,
        initialized: true,
        proposal_class: manifest.proposal_class,
        state: ProposalStateV2::Draft,
        creation_gate_status: gate.status,
        zero_tail_required: capacity.zero_tail_required,
        proposal_flags: 0,
        proposal_id: config.next_proposal_id,
        target_nonce: config.target_nonce,
        creation_slot: slot,
        cluster_domain: config.cluster_domain,
        controller_program: *program_id,
        controller_config: *config_info.key,
        protocol_gate: *gate_info.key,
        capacity_policy: *capacity_info.key,
        capacity_policy_digest: capacity.policy_digest,
        policy_version: policy.version,
        policy_hash: policy.policy_hash,
        creation_council_version: council.version,
        creation_council_hash: council.set_hash,
        creation_gate_epoch: gate.epoch,
        freeze_gate_epoch: 0,
        target_program: config.target_program,
        target_programdata: config.target_programdata,
        upgradeable_loader: config.upgradeable_loader,
        authority_pda: config.authority_pda,
        canonical_spill_treasury: config.canonical_spill_treasury,
        current_deployment_state: *deployment_info.key,
        current_deployment_digest: deployment.deployment_digest,
        current_deployment_generation: deployment.deployment_generation,
        buffer_pubkey: *buffer.key,
        buffer_loader_owner: config.upgradeable_loader,
        buffer_uploader_authority: *uploader.key,
        buffer_final_authority: config.authority_pda,
        buffer_verification: derive_buffer_check_pda(program_id, proposal_info.key).0,
        programdata_verification: derive_programdata_check_pda(program_id, proposal_info.key).0,
        artifact_length: manifest.artifact_length,
        artifact_sha256: manifest.artifact_sha256,
        artifact_chunk_merkle_root: manifest.artifact_chunk_merkle_root,
        artifact_scheme_id: capacity.artifact_scheme_id,
        artifact_chunk_size: capacity.artifact_chunk_size,
        artifact_chunk_count: artifact_chunk_count(
            manifest.artifact_length,
            capacity.artifact_chunk_size,
        )?,
        source_commit_hash: manifest.source_commit_hash,
        source_tree_hash: manifest.source_tree_hash,
        build_input_inventory_hash: manifest.build_input_inventory_hash,
        reproducible_build_receipt_hash: manifest.reproducible_build_receipt_hash,
        package_receipt_hash: manifest.package_receipt_hash,
        release_intent_hash: manifest.release_intent_hash,
        minimum_required_capacity: manifest.minimum_required_capacity,
        maximum_supported_raw_programdata_length: capacity.maximum_raw_programdata_length,
        programdata_observation_scheme_id: capacity.observation_scheme_id,
        prestate_checkpoint: expected_prestate,
        required_poststate_checkpoint: expected_poststate,
        checkpoint_schema_id: manifest.checkpoint_schema_id,
        checkpoint_policy_hash: manifest.checkpoint_policy_hash,
        primary_proposal: manifest.primary_proposal,
        rollback_proposal: manifest.rollback_proposal,
        rollback_buffer: manifest.rollback_buffer,
        rollback_artifact_length: manifest.rollback_artifact_length,
        rollback_artifact_sha256: manifest.rollback_artifact_sha256,
        rollback_artifact_chunk_root: manifest.rollback_artifact_chunk_root,
        rollback_artifact_scheme_id,
        vote_requirement: VoteRequirementV1::None,
        vote_program: Pubkey::default(),
        vote_result_pda: Pubkey::default(),
        review_start_slot,
        review_end_slot,
        not_before_slot,
        expiry_slot,
        first_approval_slot: 0,
        council_approved_slot: 0,
        governance_satisfied_slot: 0,
        queued_slot: 0,
        frozen_slot: 0,
        extension_executed_slot: 0,
        upgrade_executed_slot: 0,
        programdata_verified_slot: 0,
        poststate_accepted_slot: 0,
        unfreeze_approved_slot: 0,
        terminal_slot: 0,
        council_approval_bitset: 0,
        council_approval_count: 0,
        cancellation_council_version: 0,
        cancellation_council_hash: [0; 32],
        cancellation_approval_bitset: 0,
        cancellation_approval_count: 0,
        unfreeze_council_version: 0,
        unfreeze_council_hash: [0; 32],
        unfreeze_approval_bitset: 0,
        unfreeze_approval_count: 0,
        proposal_digest_domain_id: UPGRADE_PROPOSAL_V3_DIGEST_DOMAIN_ID,
        proposal_digest: [0; 32],
        cancellation_reason_code: 0,
        terminal_reason_code: 0,
        reserved: [0; UPGRADE_PROPOSAL_V3_RESERVED_LEN],
    };
    candidate.proposal_digest = compute_upgrade_proposal_digest_v3(&candidate)?;
    validate_upgrade_proposal_digest_v3(&candidate)?;
    let next_proposal_id = config
        .next_proposal_id
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if next_proposal_id == u64::MAX {
        return Err(GovernanceError::ArithmeticOverflow.into());
    }
    config.next_proposal_id = next_proposal_id;
    config.validate_static()?;
    let proposal_bytes = encode_fixed_account(&candidate, UpgradeProposalV3::LEN)?;
    let config_bytes = encode_fixed_account(&*config, ControllerConfigV1::LEN)?;

    let proposal_id = candidate.proposal_id.to_le_bytes();
    let bump = [candidate.bump];
    let seeds: [&[u8]; 5] = [
        UPGRADE_SEED_DOMAIN_V1,
        PROPOSAL_SEED,
        config.target_program.as_ref(),
        &proposal_id,
        &bump,
    ];
    create_fixed_pda_account(
        program_id,
        payer,
        proposal_info,
        system_program_info,
        &Rent::get()?,
        UpgradeProposalV3::LEN,
        &seeds,
    )?;
    proposal_info
        .try_borrow_mut_data()?
        .copy_from_slice(&proposal_bytes);
    config_info
        .try_borrow_mut_data()?
        .copy_from_slice(&config_bytes);
    Ok(())
}

pub(super) fn load_verified_buffer_for_proposal(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    proposal: &UpgradeProposalV3,
    slot: u64,
) -> Result<Box<BufferVerificationV1>, ProgramError> {
    let verification = load_fixed_controller_account::<BufferVerificationV1>(
        program_id,
        info,
        BufferVerificationV1::LEN,
    )?;
    verification.validate_schema()?;
    if derive_buffer_check_pda(program_id, proposal_info.key) != (*info.key, verification.bump)
        || proposal.buffer_verification != *info.key
        || verification.controller_config != proposal.controller_config
        || verification.proposal != *proposal_info.key
        || verification.upgradeable_loader != proposal.upgradeable_loader
        || verification.buffer != proposal.buffer_pubkey
        || verification.expected_uploader_authority != proposal.buffer_uploader_authority
        || verification.controller_authority != proposal.authority_pda
        || verification.status != BufferVerificationStatusV1::Verified
        || verification.artifact_length != proposal.artifact_length
        || verification.artifact_sha256 != proposal.artifact_sha256
        || verification.artifact_chunk_merkle_root != proposal.artifact_chunk_merkle_root
        || verification.chunk_hash_domain != proposal.artifact_scheme_id
        || verification.chunk_size != proposal.artifact_chunk_size
        || verification.chunk_count != proposal.artifact_chunk_count
        || verification.finalized_slot == 0
        || verification.finalized_slot > slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(verification)
}

pub fn process_approve_proposal_v3(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ApproveProposalV3,
) -> ProgramResult {
    exact_account_count(accounts, 9)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, capacity_info, deployment_info, proposal_info, buffer_verification_info, seat] =
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
        buffer_verification_info,
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
    let mut proposal = load_proposal(
        program_id,
        proposal_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
    )?;
    let council = load_council(
        program_id,
        council_info,
        config_info,
        &context.config,
        &policy,
        proposal.creation_council_version,
        &proposal.creation_council_hash,
        slot,
        true,
    )?;
    require_proposal_deployment_current(&proposal, &context.deployment)?;
    check_proposal_guard(&instruction.expected, &proposal, &context)?;
    require_creation_gate(&proposal, &context.config, &context.gate)?;
    require_approval_window(&proposal, slot)?;
    if *seat.key == context.config.guardian
        || proposal.state != ProposalStateV2::BufferVerified
        || instruction.expected_creation_council_version != proposal.creation_council_version
        || instruction.expected_creation_council_hash != proposal.creation_council_hash
        || instruction.expected_approval_bitset != proposal.council_approval_bitset
        || instruction.expected_approval_count != proposal.council_approval_count
        || proposal.council_approval_count >= RELEASE1_APPROVAL_THRESHOLD
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    load_verified_buffer_for_proposal(
        program_id,
        buffer_verification_info,
        proposal_info,
        &proposal,
        slot,
    )?;
    let (bitset, count) = record_seat_approval(
        &council,
        proposal.council_approval_bitset,
        proposal.council_approval_count,
        seat.key,
        slot,
    )?;
    proposal.council_approval_bitset = bitset;
    proposal.council_approval_count = count;
    if proposal.first_approval_slot == 0 {
        proposal.first_approval_slot = slot;
    }
    if count == RELEASE1_APPROVAL_THRESHOLD {
        proposal.state = ProposalStateV2::CouncilApproved;
        proposal.council_approved_slot = slot;
    }
    validate_upgrade_proposal_digest_v3(&proposal)?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        UpgradeProposalV3::LEN,
    )
}

pub fn process_finalize_governance_v3(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: FinalizeGovernanceV3,
) -> ProgramResult {
    exact_account_count(accounts, 7)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, capacity_info, deployment_info, proposal_info] =
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
    let slot = current_slot()?;
    let context = load_lifecycle_context(
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
    require_proposal_deployment_current(&proposal, &context.deployment)?;
    check_proposal_guard(&instruction.expected, &proposal, &context)?;
    require_creation_gate(&proposal, &context.config, &context.gate)?;
    if proposal.state != ProposalStateV2::CouncilApproved
        || slot >= proposal.expiry_slot
        || instruction.expected_approval_bitset != proposal.council_approval_bitset
        || instruction.expected_approval_count != proposal.council_approval_count
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_exact_recorded_quorum(
        &council,
        proposal.council_approval_bitset,
        proposal.council_approval_count,
        proposal.council_approved_slot,
    )?;
    proposal.state = ProposalStateV2::GovernanceSatisfied;
    proposal.governance_satisfied_slot = slot;
    validate_upgrade_proposal_digest_v3(&proposal)?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        UpgradeProposalV3::LEN,
    )
}

pub fn process_queue_proposal_v3(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: QueueProposalV3,
) -> ProgramResult {
    exact_account_count(accounts, 6)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, gate_info, capacity_info, deployment_info, proposal_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for info in [
        config_info,
        policy_info,
        gate_info,
        capacity_info,
        deployment_info,
    ] {
        readonly(info)?;
    }
    writable(proposal_info)?;
    let slot = current_slot()?;
    let context = load_lifecycle_context(
        program_id,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
    )?;
    let _policy = load_policy(program_id, policy_info, config_info, &context.config, slot)?;
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
    check_proposal_guard(&instruction.expected, &proposal, &context)?;
    require_creation_gate(&proposal, &context.config, &context.gate)?;
    verify_exact_proposal_timing(&proposal, &context.config)?;
    if proposal.state != ProposalStateV2::GovernanceSatisfied
        || proposal.council_approval_count != RELEASE1_APPROVAL_THRESHOLD
        || slot >= proposal.expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    proposal.state = ProposalStateV2::Timelocked;
    proposal.queued_slot = slot;
    validate_upgrade_proposal_digest_v3(&proposal)?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        UpgradeProposalV3::LEN,
    )
}
