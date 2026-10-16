use super::*;

/// Creates the one canonical governed target-authority handoff proposal.
///
/// Accounts: payer W/S; creator seat RO/S; controller Program RO/X; controller ProgramData RO;
/// config, policy, council, gate, capacity, immutability receipt, bridge observation RO;
/// target Program RO/X; target ProgramData RO; legacy authority RO; controller authority RO;
/// handoff proposal W; Loader RO/X; System Program RO/X.
pub fn process_create_target_authority_handoff_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CreateTargetAuthorityHandoffV1,
) -> ProgramResult {
    let [payer, creator, controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, observation_info, target_program, target_programdata, legacy_authority, authority_info, proposal_info, loader_info, system_program_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    debug_assert_eq!(
        accounts.len(),
        CREATE_TARGET_AUTHORITY_HANDOFF_V1_ACCOUNT_COUNT
    );
    validate_exact_privileges(payer, true, true, false)?;
    validate_seat_authority(creator)?;
    validate_exact_privileges(controller_program, false, false, true)?;
    for account in [
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        observation_info,
        target_programdata,
        legacy_authority,
        authority_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(target_program, false, false, true)?;
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&accounts.iter().collect::<Vec<_>>())?;

    let slot = Clock::get()?.slot;
    let context = load_ceremony_context(
        program_id,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        slot,
    )?;
    reject_guardian(&context.config, creator.key)?;
    require_active_seat(&context.council, creator.key, slot)?;
    validate_bootstrap_gate(&context.gate, &context.config)?;
    let immutable = load_immutability_receipt(
        program_id,
        immutability_info,
        capacity_info,
        &context.capacity,
        &context.config,
    )?;
    validate_controller_still_immutable(
        program_id,
        controller_program,
        controller_programdata,
        &immutable,
    )?;
    let observation = load_observation(
        program_id,
        observation_info,
        config_info,
        &context.config,
        capacity_info,
        &context.capacity,
        target_program.key,
        ProgramDataObservationPurposeV1::TargetHandoffBridge,
        immutability_info.key,
    )?;
    require_live_observation(
        target_program,
        target_programdata,
        &observation,
        Some(*legacy_authority.key),
    )?;
    validate_target_graph(
        &context.config,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
    )?;
    if *system_program_info.key != system_program::ID
        || *legacy_authority.key == Pubkey::default()
        || *legacy_authority.key == context.config.guardian
        || instruction.plan_valid_until_slot == 0
        || slot > instruction.plan_valid_until_slot
        || instruction.expected_gate_epoch != context.gate.epoch
        || instruction.expected_target_nonce != context.config.target_nonce
        || instruction.expected_council_version != context.council.version
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let timing = derive_major_timing(&context.config, slot)?;
    let (expected_proposal, proposal_bump) = derive_target_authority_handoff_pda(
        program_id,
        &context.config.target_program,
        context.council.version,
    );
    if *proposal_info.key != expected_proposal {
        return Err(GovernanceError::InvalidPda.into());
    }
    let proposal = TargetAuthorityHandoffProposalV1 {
        discriminator: TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: proposal_bump,
        initialized: true,
        state: CeremonyProposalStateV1::Draft,
        cluster_domain: context.config.cluster_domain,
        controller_program: *program_id,
        controller_programdata: *controller_programdata.key,
        controller_immutability_receipt: *immutability_info.key,
        controller_immutability_digest: immutable.receipt_digest,
        controller_config: *config_info.key,
        governance_policy: *policy_info.key,
        governance_policy_hash: context.policy.policy_hash,
        capacity_policy: *capacity_info.key,
        capacity_policy_digest: context.capacity.policy_digest,
        gate: *gate_info.key,
        controller_authority: context.config.authority_pda,
        target_program: context.config.target_program,
        target_programdata: context.config.target_programdata,
        upgradeable_loader: context.config.upgradeable_loader,
        legacy_target_authority: *legacy_authority.key,
        bridge_artifact_length: observation.expected_artifact_length,
        bridge_artifact_sha256: observation.expected_artifact_sha256,
        bridge_artifact_merkle_root: observation.expected_artifact_merkle_root,
        bridge_artifact_scheme_id: observation.expected_artifact_scheme_id,
        bridge_source_commitment: instruction.bridge_source_commitment,
        bridge_build_inputs_commitment: instruction.bridge_build_inputs_commitment,
        bridge_package_commitment: instruction.bridge_package_commitment,
        bridge_release_manifest_commitment: instruction.bridge_release_manifest_commitment,
        bridge_observation: *observation_info.key,
        bridge_observation_generation: observation.generation,
        bridge_observation_root: observation.final_raw_merkle_root,
        bridge_observation_digest: observation.observation_digest,
        minimum_target_deployed_slot: observation.deployed_slot,
        minimum_target_capacity: observation.actual_capacity,
        minimum_target_raw_length: observation.raw_data_length,
        bootstrap_gate_status: context.gate.status,
        bootstrap_gate_epoch: context.gate.epoch,
        bootstrap_freeze_reason_code: context.gate.freeze_reason_code,
        bootstrap_freeze_slot: context.gate.freeze_slot,
        target_nonce: context.config.target_nonce,
        council_version: context.council.version,
        council_hash: context.council.set_hash,
        review_start_slot: timing.0,
        review_end_slot: timing.1,
        not_before_slot: timing.2,
        expiry_slot: timing.3,
        approval_bitset: 0,
        approval_count: 0,
        approval_threshold: RELEASE1_APPROVAL_THRESHOLD,
        first_approval_slot: 0,
        council_approved_slot: 0,
        queued_slot: 0,
        executed_slot: 0,
        terminal_slot: 0,
        terminal_reason_code: 0,
        proposal_digest: [0; 32],
        creation_slot: slot,
        reserved: [0; TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_RESERVED_LEN],
    };
    let mut proposal = proposal;
    proposal.proposal_digest = compute_target_handoff_proposal_digest_v1(&proposal)?;
    validate_target_handoff_proposal_digest_v1(&proposal)?;
    let proposal_bytes = encode_fixed_account(&proposal, TargetAuthorityHandoffProposalV1::LEN)?;
    let bump_seed = [proposal_bump];
    let council_version_seed = context.council.version.to_le_bytes();
    create_fixed_pda_account(
        program_id,
        payer,
        proposal_info,
        system_program_info,
        &Rent::get()?,
        TargetAuthorityHandoffProposalV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            TARGET_HANDOFF_SEED,
            context.config.target_program.as_ref(),
            &council_version_seed,
            &bump_seed,
        ],
    )?;
    commit_preencoded(program_id, &[(proposal_info, &proposal_bytes)])
}

pub fn process_approve_target_authority_handoff_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ApproveTargetAuthorityHandoffV1,
) -> ProgramResult {
    let [controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, observation_info, target_program, target_programdata, legacy_authority, authority_info, proposal_info, loader_info, seat] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    debug_assert_eq!(
        accounts.len(),
        APPROVE_TARGET_AUTHORITY_HANDOFF_V1_ACCOUNT_COUNT
    );
    validate_handoff_review_privileges(
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        observation_info,
        target_program,
        target_programdata,
        legacy_authority,
        authority_info,
        proposal_info,
        loader_info,
    )?;
    validate_seat_authority(seat)?;
    require_distinct_accounts(&accounts.iter().collect::<Vec<_>>())?;
    let slot = Clock::get()?.slot;
    let mut validated = validate_handoff_review_state(
        program_id,
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        observation_info,
        target_program,
        target_programdata,
        legacy_authority,
        authority_info,
        proposal_info,
        loader_info,
        slot,
    )?;
    reject_guardian(&validated.context.config, seat.key)?;
    if validated.proposal.state != CeremonyProposalStateV1::Draft
        || instruction.expected_proposal_digest != validated.proposal.proposal_digest
        || instruction.expected_council_version != validated.context.council.version
        || instruction.expected_gate_epoch != validated.context.gate.epoch
        || instruction.expected_target_nonce != validated.context.config.target_nonce
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_approval_window(&validated.proposal, slot)?;
    let (bitset, count) = record_seat_approval(
        &validated.context.council,
        validated.proposal.approval_bitset,
        validated.proposal.approval_count,
        seat.key,
        slot,
    )?;
    validated.proposal.approval_bitset = bitset;
    validated.proposal.approval_count = count;
    if validated.proposal.first_approval_slot == 0 {
        validated.proposal.first_approval_slot = slot;
    }
    if count == RELEASE1_APPROVAL_THRESHOLD {
        validated.proposal.state = CeremonyProposalStateV1::CouncilApproved;
        validated.proposal.council_approved_slot = slot;
    }
    validate_target_handoff_proposal_digest_v1(&validated.proposal)?;
    let bytes = encode_fixed_account(&validated.proposal, TargetAuthorityHandoffProposalV1::LEN)?;
    commit_preencoded(program_id, &[(proposal_info, &bytes)])
}

pub fn process_queue_target_authority_handoff_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: QueueTargetAuthorityHandoffV1,
) -> ProgramResult {
    let [controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, observation_info, target_program, target_programdata, legacy_authority, authority_info, proposal_info, loader_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    debug_assert_eq!(
        accounts.len(),
        QUEUE_TARGET_AUTHORITY_HANDOFF_V1_ACCOUNT_COUNT
    );
    validate_handoff_review_privileges(
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        observation_info,
        target_program,
        target_programdata,
        legacy_authority,
        authority_info,
        proposal_info,
        loader_info,
    )?;
    require_distinct_accounts(&accounts.iter().collect::<Vec<_>>())?;
    let slot = Clock::get()?.slot;
    let mut validated = validate_handoff_review_state(
        program_id,
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        observation_info,
        target_program,
        target_programdata,
        legacy_authority,
        authority_info,
        proposal_info,
        loader_info,
        slot,
    )?;
    if validated.proposal.state != CeremonyProposalStateV1::CouncilApproved
        || instruction.expected_proposal_digest != validated.proposal.proposal_digest
        || instruction.expected_council_version != validated.context.council.version
        || instruction.expected_gate_epoch != validated.context.gate.epoch
        || instruction.expected_target_nonce != validated.context.config.target_nonce
        || slot >= validated.proposal.expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_exact_quorum(
        &validated.context.council,
        validated.proposal.approval_bitset,
        validated.proposal.approval_count,
        slot,
    )?;
    validated.proposal.state = CeremonyProposalStateV1::Timelocked;
    validated.proposal.queued_slot = slot;
    validate_target_handoff_proposal_digest_v1(&validated.proposal)?;
    let bytes = encode_fixed_account(&validated.proposal, TargetAuthorityHandoffProposalV1::LEN)?;
    commit_preencoded(program_id, &[(proposal_info, &bytes)])
}

/// Performs one exact checked Loader-v3 authority transfer and atomically finalizes the receipt.
/// The gate is read-only and remains in its initialization freeze.
pub fn process_accept_target_authority_checked_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: AcceptTargetAuthorityCheckedV1,
) -> ProgramResult {
    let [payer, controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, proposal_info, observation_info, target_program, target_programdata, legacy_authority, authority_info, loader_info, receipt_info, system_program_info, instructions_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    debug_assert_eq!(
        accounts.len(),
        ACCEPT_TARGET_AUTHORITY_CHECKED_V1_ACCOUNT_COUNT
    );
    validate_exact_privileges(payer, true, true, false)?;
    validate_exact_privileges(controller_program, false, false, true)?;
    for account in [
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        observation_info,
        authority_info,
        instructions_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(target_program, false, false, true)?;
    validate_exact_privileges(target_programdata, true, false, false)?;
    validate_exact_privileges(legacy_authority, false, true, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(receipt_info, true, false, false)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&accounts.iter().collect::<Vec<_>>())?;

    let slot = Clock::get()?.slot;
    let context = load_ceremony_context(
        program_id,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        slot,
    )?;
    validate_bootstrap_gate(&context.gate, &context.config)?;
    let immutable = load_immutability_receipt(
        program_id,
        immutability_info,
        capacity_info,
        &context.capacity,
        &context.config,
    )?;
    validate_controller_still_immutable(
        program_id,
        controller_program,
        controller_programdata,
        &immutable,
    )?;
    let mut proposal = load_handoff_proposal(
        program_id,
        proposal_info,
        config_info,
        policy_info,
        capacity_info,
        gate_info,
        immutability_info,
        &context,
    )?;
    let observation = load_observation(
        program_id,
        observation_info,
        config_info,
        &context.config,
        capacity_info,
        &context.capacity,
        target_program.key,
        ProgramDataObservationPurposeV1::TargetHandoffBridge,
        immutability_info.key,
    )?;
    validate_target_graph(
        &context.config,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
    )?;
    require_live_observation(
        target_program,
        target_programdata,
        &observation,
        Some(*legacy_authority.key),
    )?;
    validate_handoff_proposal_evidence(
        &proposal,
        &context,
        immutability_info,
        &immutable,
        observation_info,
        &observation,
        legacy_authority.key,
    )?;
    if *system_program_info.key != system_program::ID
        || *instructions_info.key != sysvar_ids::instructions::ID
        || *legacy_authority.key == context.config.guardian
        || proposal.state != CeremonyProposalStateV1::Timelocked
        || instruction.expected_proposal_digest != proposal.proposal_digest
        || instruction.expected_bridge_observation_digest != observation.observation_digest
        || instruction.expected_gate_epoch != context.gate.epoch
        || instruction.expected_target_nonce != context.config.target_nonce
        || slot < proposal.not_before_slot
        || slot >= proposal.expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_exact_quorum(
        &context.council,
        proposal.approval_bitset,
        proposal.approval_count,
        slot,
    )?;
    let expected_current_data = instruction.pack()?;
    validate_canonical_envelope(
        program_id,
        accounts,
        instructions_info,
        &expected_current_data,
        &instruction.envelope,
    )?;

    let (expected_receipt, receipt_bump) =
        derive_target_handoff_receipt_pda(program_id, &context.config.target_program);
    if *receipt_info.key != expected_receipt {
        return Err(GovernanceError::InvalidPda.into());
    }
    let pre_header = observation.programdata_header_snapshot;
    validate_some_to_some_header_delta(
        &pre_header,
        &expected_handoff_post_header(
            &pre_header,
            *legacy_authority.key,
            context.config.authority_pda,
        )?,
        *legacy_authority.key,
        context.config.authority_pda,
    )?;

    let authority_bump = derive_authority_pda(program_id, &context.config.target_program).1;
    let authority_bump_seed = [authority_bump];
    let authority_seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        AUTHORITY_SEED,
        context.config.target_program.as_ref(),
        &authority_bump_seed,
    ];
    let cpi =
        set_upgrade_authority_checked(target_program.key, legacy_authority.key, authority_info.key);
    validate_checked_handoff_cpi_shape(
        &cpi,
        target_programdata.key,
        legacy_authority.key,
        authority_info.key,
    )?;
    invoke_signed(
        &cpi,
        &[
            target_programdata.clone(),
            legacy_authority.clone(),
            authority_info.clone(),
            loader_info.clone(),
        ],
        &[authority_seeds],
    )?;

    let post_linkage = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &context.config.upgradeable_loader,
    )?;
    let post_header = copy_programdata_header(target_programdata)?;
    validate_some_to_some_header_delta(
        &pre_header,
        &post_header,
        *legacy_authority.key,
        context.config.authority_pda,
    )?;
    if post_linkage.upgrade_authority != Some(context.config.authority_pda)
        || post_linkage.deployed_slot != observation.deployed_slot
        || u64::try_from(post_linkage.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != observation.actual_capacity
    {
        return Err(GovernanceError::InvalidAuthorityTransition.into());
    }

    proposal.state = CeremonyProposalStateV1::Completed;
    proposal.executed_slot = slot;
    proposal.terminal_slot = slot;
    proposal.terminal_reason_code = CEREMONY_PROPOSAL_COMPLETED_REASON_V1;
    validate_target_handoff_proposal_digest_v1(&proposal)?;
    let mut receipt = TargetAuthorityHandoffReceiptV1 {
        discriminator: TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: receipt_bump,
        initialized: true,
        proposal: *proposal_info.key,
        proposal_digest: proposal.proposal_digest,
        controller_program: *program_id,
        controller_config: *config_info.key,
        controller_authority: context.config.authority_pda,
        controller_immutability_receipt: *immutability_info.key,
        controller_immutability_digest: immutable.receipt_digest,
        target_program: context.config.target_program,
        target_programdata: context.config.target_programdata,
        upgradeable_loader: context.config.upgradeable_loader,
        pre_observation: *observation_info.key,
        pre_observation_generation: observation.generation,
        pre_observation_root: observation.final_raw_merkle_root,
        pre_observation_digest: observation.observation_digest,
        pre_upgrade_authority: OptionalPubkeyV1::some(*legacy_authority.key)?,
        pre_programdata_header_snapshot: pre_header,
        post_programdata_header_snapshot: post_header,
        post_upgrade_authority: OptionalPubkeyV1::some(context.config.authority_pda)?,
        deployed_slot: observation.deployed_slot,
        raw_programdata_length: observation.raw_data_length,
        programdata_capacity: observation.actual_capacity,
        artifact_length: proposal.bridge_artifact_length,
        artifact_sha256: proposal.bridge_artifact_sha256,
        artifact_merkle_root: proposal.bridge_artifact_merkle_root,
        artifact_scheme_id: proposal.bridge_artifact_scheme_id,
        bridge_source_commitment: proposal.bridge_source_commitment,
        bridge_build_inputs_commitment: proposal.bridge_build_inputs_commitment,
        bridge_package_commitment: proposal.bridge_package_commitment,
        bridge_release_manifest_commitment: proposal.bridge_release_manifest_commitment,
        bootstrap_gate_epoch: proposal.bootstrap_gate_epoch,
        target_nonce: proposal.target_nonce,
        council_version: proposal.council_version,
        accepted_slot: slot,
        receipt_digest: [0; 32],
        finalized: true,
        reserved: [0; TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_RESERVED_LEN],
    };
    receipt.receipt_digest = compute_target_handoff_receipt_digest_v1(&receipt)?;
    validate_target_handoff_receipt_digest_v1(&receipt)?;
    let proposal_bytes = encode_fixed_account(&proposal, TargetAuthorityHandoffProposalV1::LEN)?;
    let receipt_bytes = encode_fixed_account(&receipt, TargetAuthorityHandoffReceiptV1::LEN)?;
    let receipt_bump_seed = [receipt_bump];
    create_fixed_pda_account(
        program_id,
        payer,
        receipt_info,
        system_program_info,
        &Rent::get()?,
        TargetAuthorityHandoffReceiptV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            TARGET_HANDOFF_RECEIPT_SEED,
            context.config.target_program.as_ref(),
            &receipt_bump_seed,
        ],
    )?;
    commit_preencoded(
        program_id,
        &[
            (proposal_info, &proposal_bytes),
            (receipt_info, &receipt_bytes),
        ],
    )
}
