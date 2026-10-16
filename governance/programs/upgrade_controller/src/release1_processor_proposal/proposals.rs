use super::*;

pub fn process_create_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CreateProposalV2,
) -> ProgramResult {
    let [payer, creator, config_info, policy_info, council_info, gate_info, target_program, target_programdata, loader, authority, spill, buffer, uploader, proposal_info, system_program_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };

    validate_exact_privileges(payer, true, true, false)?;
    validate_seat_authority(creator)?;
    // Proposal creation atomically consumes `next_proposal_id`; the config is
    // therefore writable in both the frozen instruction ABI and the processor
    // contract. Requiring it read-only would make the typed builder
    // unexecutable and was caught by the real ProgramTest lifecycle.
    validate_exact_privileges(config_info, true, false, false)?;
    for account in [
        policy_info,
        council_info,
        gate_info,
        authority,
        spill,
        buffer,
        uploader,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(target_program, false, false, true)?;
    validate_exact_privileges(target_programdata, false, false, false)?;
    validate_exact_privileges(loader, false, false, true)?;
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&[
        payer,
        creator,
        config_info,
        policy_info,
        council_info,
        gate_info,
        target_program,
        target_programdata,
        loader,
        authority,
        spill,
        buffer,
        uploader,
        proposal_info,
        system_program_info,
    ])?;

    let mut config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
    let council = load_current_council(program_id, council_info, config_info, &config, &policy)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let slot = Clock::get()?.slot;
    require_policy_active(&policy, slot)?;
    reject_guardian_authority(&config, creator.key)?;
    require_active_seat(&council, creator.key, slot)?;
    if instruction.creation_slot != slot
        || instruction.expected_proposal_id != config.next_proposal_id
        || instruction.expected_target_nonce != config.target_nonce
        || instruction.expected_policy_version != policy.version
        || instruction.expected_policy_hash != policy.policy_hash
        || instruction.expected_creation_council_version != council.version
        || instruction.expected_creation_council_hash != council.set_hash
        || instruction.creation_gate_status != gate.status
        || instruction.expected_creation_gate_epoch != gate.epoch
        || instruction.expected_freeze_gate_epoch != 0
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    if gate.status == GateStatusV1::FrozenForUpgrade
        || gate.status == GateStatusV1::EmergencyFrozen
            && gate.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
    {
        return Err(GovernanceError::InvalidGateState.into());
    }

    validate_loader_identity(loader, &config)?;
    if *target_program.key != config.target_program
        || *target_programdata.key != config.target_programdata
        || *authority.key != config.authority_pda
        || *spill.key != config.canonical_spill_treasury
        || *system_program_info.key != system_program::ID
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let live = read_canonical_programdata_snapshot(
        target_program,
        target_programdata,
        &config,
        Some(config.authority_pda),
    )?;
    let buffer_header = validate_buffer_account(buffer, &config.upgradeable_loader)?;
    if buffer_header.authority != Some(*uploader.key)
        || *uploader.key == Pubkey::default()
        || u64::try_from(buffer_header.payload_length)
            .map_err(|_| GovernanceError::ArithmeticOverflow)?
            != instruction.artifact_length
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let (expected_proposal, proposal_bump) =
        derive_proposal_pda(program_id, &config.target_program, config.next_proposal_id);
    if *proposal_info.key != expected_proposal {
        return Err(GovernanceError::InvalidPda.into());
    }
    let (review_start_slot, review_end_slot, not_before_slot, expiry_slot) =
        derive_proposal_timing(&config, instruction.proposal_class, slot)?;
    if instruction.review_start_slot != review_start_slot
        || instruction.review_end_slot != review_end_slot
        || instruction.not_before_slot != not_before_slot
        || instruction.expiry_slot != expiry_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }

    let primary_proposal = optional_pubkey(instruction.primary_proposal.value())?;
    let rollback_proposal = optional_pubkey(instruction.rollback_proposal.value())?;
    let rollback_buffer = optional_pubkey(instruction.rollback_buffer.value())?;
    let chunk_count =
        artifact_chunk_count(instruction.artifact_length, RELEASE1_ARTIFACT_CHUNK_SIZE_V1)?;
    let (prestate_checkpoint, _) =
        derive_checkpoint_pda(program_id, proposal_info.key, CheckpointPhaseV1::Prestate);
    let (poststate_checkpoint, _) =
        derive_checkpoint_pda(program_id, proposal_info.key, CheckpointPhaseV1::Poststate);
    let rollback_class = instruction.proposal_class == ProposalClassV1::EmergencyRollback;
    if (!rollback_class
        && (instruction.deployed_slot != live.deployed_slot
            || instruction.current_raw_programdata_hash != live.raw_hash
            || instruction.current_capacity != live.capacity))
        || (rollback_class
            && (instruction.deployed_slot != 0
                || instruction.current_raw_programdata_hash != [0; 32]))
    {
        return Err(GovernanceError::InvalidProposalCommitment.into());
    }

    let proposal = UpgradeProposalV2 {
        discriminator: UPGRADE_PROPOSAL_V2_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V2,
        bump: proposal_bump,
        initialized: true,
        proposal_class: instruction.proposal_class,
        state: ProposalStateV2::Draft,
        creation_gate_status: instruction.creation_gate_status,
        zero_tail_required: true,
        proposal_flags: 0,
        proposal_id: instruction.expected_proposal_id,
        target_nonce: instruction.expected_target_nonce,
        creation_slot: slot,
        cluster_domain: config.cluster_domain,
        controller_program: *program_id,
        controller_config: *config_info.key,
        protocol_gate: *gate_info.key,
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
        buffer_pubkey: *buffer.key,
        buffer_loader_owner: config.upgradeable_loader,
        buffer_uploader_authority: *uploader.key,
        buffer_final_authority: config.authority_pda,
        buffer_verification: derive_buffer_check_pda(program_id, proposal_info.key).0,
        programdata_verification: derive_programdata_check_pda(program_id, proposal_info.key).0,
        artifact_length: instruction.artifact_length,
        artifact_sha256: instruction.artifact_sha256,
        artifact_chunk_merkle_root: instruction.artifact_chunk_merkle_root,
        chunk_hash_domain: ARTIFACT_MERKLE_SCHEME_ID,
        chunk_size: RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        chunk_count,
        source_commit_hash: instruction.source_commit_hash,
        source_tree_hash: instruction.source_tree_hash,
        build_input_inventory_hash: instruction.build_input_inventory_hash,
        reproducible_build_receipt_hash: instruction.reproducible_build_receipt_hash,
        package_receipt_hash: instruction.package_receipt_hash,
        release_intent_hash: instruction.release_intent_hash,
        expected_execution_pre_payload_hash: instruction.expected_execution_pre_payload_hash,
        expected_execution_pre_chunk_root: instruction.expected_execution_pre_chunk_root,
        current_raw_programdata_hash: instruction.current_raw_programdata_hash,
        deployed_slot: instruction.deployed_slot,
        current_capacity: instruction.current_capacity,
        extension_delta: instruction.extension_delta,
        expected_post_capacity: instruction.expected_post_capacity,
        prestate_checkpoint,
        required_poststate_checkpoint: poststate_checkpoint,
        checkpoint_schema_id: instruction.checkpoint_schema_id,
        checkpoint_policy_hash: instruction.checkpoint_policy_hash,
        primary_proposal,
        rollback_proposal,
        rollback_buffer,
        rollback_artifact_sha256: instruction.rollback_artifact_sha256,
        rollback_artifact_chunk_root: instruction.rollback_artifact_chunk_root,
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
        proposal_digest: instruction.expected_proposal_digest,
        cancellation_reason_code: 0,
        terminal_reason_code: 0,
        reserved: [0; UPGRADE_PROPOSAL_V2_RESERVED_LEN],
    };
    validate_proposal_digest_v2(&proposal)?;
    if compute_proposal_digest_v2(&proposal)? != instruction.expected_proposal_digest {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    config.next_proposal_id = config
        .next_proposal_id
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    config.validate_static()?;
    let _proposal_bytes = encode_fixed_account(&proposal, UpgradeProposalV2::LEN)?;
    let _config_bytes = encode_fixed_account(&*config, ControllerConfigV1::LEN)?;

    let proposal_id_bytes = instruction.expected_proposal_id.to_le_bytes();
    let bump_seed = [proposal_bump];
    let signer_seeds: [&[u8]; 5] = [
        UPGRADE_SEED_DOMAIN_V1,
        PROPOSAL_SEED,
        config.target_program.as_ref(),
        &proposal_id_bytes,
        &bump_seed,
    ];
    create_fixed_pda_account(
        program_id,
        payer,
        proposal_info,
        system_program_info,
        &Rent::get()?,
        UpgradeProposalV2::LEN,
        &signer_seeds,
    )?;
    store_fixed_controller_account(program_id, proposal_info, &proposal, UpgradeProposalV2::LEN)?;
    store_fixed_controller_account(program_id, config_info, &*config, ControllerConfigV1::LEN)
}

pub fn process_approve_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ApproveProposalV2,
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
    require_creation_gate(&proposal, &config, &gate)?;
    require_approval_window(
        proposal.review_start_slot,
        proposal.review_end_slot,
        proposal.expiry_slot,
        slot,
    )?;
    if proposal.state != ProposalStateV2::BufferVerified
        || proposal.creation_council_version != config.current_council_version
        || proposal.creation_council_version != council.version
        || proposal.creation_council_hash != council.set_hash
        || instruction.expected_approval_bitset != proposal.council_approval_bitset
        || instruction.expected_approval_count != proposal.council_approval_count
        || proposal.council_approval_count >= RELEASE1_APPROVAL_THRESHOLD
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let (next_bitset, next_count) = record_seat_approval(
        &council,
        proposal.council_approval_bitset,
        proposal.council_approval_count,
        seat.key,
        slot,
    )?;
    proposal.council_approval_bitset = next_bitset;
    proposal.council_approval_count = next_count;
    if proposal.first_approval_slot == 0 {
        proposal.first_approval_slot = slot;
    }
    if next_count == RELEASE1_APPROVAL_THRESHOLD {
        proposal.state = ProposalStateV2::CouncilApproved;
        proposal.council_approved_slot = slot;
    }
    validate_proposal_digest_v2(&proposal)?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        UpgradeProposalV2::LEN,
    )
}

pub fn process_finalize_governance_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: FinalizeGovernanceV2,
) -> ProgramResult {
    let [config_info, policy_info, council_info, gate_info, proposal_info] = accounts else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_readonly_state_accounts(&[config_info, policy_info, council_info, gate_info])?;
    validate_exact_privileges(proposal_info, true, false, false)?;
    require_distinct_accounts(&[
        config_info,
        policy_info,
        council_info,
        gate_info,
        proposal_info,
    ])?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let mut proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    let council = load_pinned_council(
        program_id,
        council_info,
        config_info,
        &config,
        &policy,
        proposal.creation_council_version,
        &proposal.creation_council_hash,
    )?;
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
    require_creation_gate(&proposal, &config, &gate)?;
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
    validate_proposal_digest_v2(&proposal)?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        UpgradeProposalV2::LEN,
    )
}

pub fn process_queue_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: QueueProposalV2,
) -> ProgramResult {
    let [config_info, policy_info, gate_info, proposal_info] = accounts else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_readonly_state_accounts(&[config_info, policy_info, gate_info])?;
    validate_exact_privileges(proposal_info, true, false, false)?;
    require_distinct_accounts(&[config_info, policy_info, gate_info, proposal_info])?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
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
        proposal.creation_council_version,
        &proposal.creation_council_hash,
    )?;
    require_creation_gate(&proposal, &config, &gate)?;
    if proposal.state != ProposalStateV2::GovernanceSatisfied
        || proposal.council_approval_count != RELEASE1_APPROVAL_THRESHOLD
        || slot >= proposal.expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    verify_exact_proposal_timing(&proposal, &config)?;
    proposal.state = ProposalStateV2::Timelocked;
    proposal.queued_slot = slot;
    validate_proposal_digest_v2(&proposal)?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        UpgradeProposalV2::LEN,
    )
}
