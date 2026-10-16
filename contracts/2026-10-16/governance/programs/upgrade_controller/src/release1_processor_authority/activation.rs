use super::*;

/// Creates the activation proposal and preallocates its receipt and deployment-state PDAs.  The
/// preallocation is what lets execution remain a strict no-CPI transaction.
pub fn process_create_bootstrap_activation_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CreateBootstrapActivationV1,
) -> ProgramResult {
    let [payer, creator, controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, handoff_receipt_info, observation_info, target_program, target_programdata, authority_info, proposal_info, activation_receipt_info, deployment_info, loader_info, system_program_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    debug_assert_eq!(accounts.len(), CREATE_BOOTSTRAP_ACTIVATION_V1_ACCOUNT_COUNT);
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
        handoff_receipt_info,
        observation_info,
        target_programdata,
        authority_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(target_program, false, false, true)?;
    for account in [proposal_info, activation_receipt_info, deployment_info] {
        validate_exact_privileges(account, true, false, false)?;
    }
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&accounts.iter().collect::<Vec<_>>())?;

    let slot = Clock::get()?.slot;
    let validated = validate_activation_evidence(
        program_id,
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        observation_info,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        slot,
    )?;
    reject_guardian(&validated.context.config, creator.key)?;
    require_active_seat(&validated.context.council, creator.key, slot)?;
    if *system_program_info.key != system_program::ID
        || instruction.expected_controller_immutability_digest != validated.immutable.receipt_digest
        || instruction.expected_handoff_receipt_digest != validated.handoff.receipt_digest
        || instruction.expected_bridge_observation_digest
            != validated.observation.observation_digest
        || instruction.expected_gate_epoch != validated.context.gate.epoch
        || instruction.expected_target_nonce != validated.context.config.target_nonce
        || instruction.expected_council_version != validated.context.council.version
        || instruction.plan_valid_until_slot == 0
        || slot > instruction.plan_valid_until_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let timing = derive_major_timing(&validated.context.config, slot)?;
    let (expected_proposal, proposal_bump) = derive_bootstrap_activation_pda(
        program_id,
        &validated.context.config.target_program,
        validated.context.council.version,
    );
    let (expected_receipt, receipt_bump) = derive_bootstrap_activation_receipt_pda(
        program_id,
        &validated.context.config.target_program,
    );
    let (expected_deployment, deployment_bump) =
        derive_current_deployment_state_pda(program_id, &validated.context.config.target_program);
    if *proposal_info.key != expected_proposal
        || *activation_receipt_info.key != expected_receipt
        || *deployment_info.key != expected_deployment
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    let proposal = BootstrapActivationProposalV1 {
        discriminator: BOOTSTRAP_ACTIVATION_PROPOSAL_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: proposal_bump,
        initialized: true,
        state: CeremonyProposalStateV1::Draft,
        cluster_domain: validated.context.config.cluster_domain,
        controller_program: *program_id,
        controller_programdata: *controller_programdata.key,
        controller_config: *config_info.key,
        governance_policy: *policy_info.key,
        governance_policy_hash: validated.context.policy.policy_hash,
        capacity_policy: *capacity_info.key,
        capacity_policy_digest: validated.context.capacity.policy_digest,
        controller_immutability_receipt: *immutability_info.key,
        controller_immutability_digest: validated.immutable.receipt_digest,
        target_handoff_receipt: *handoff_receipt_info.key,
        target_handoff_digest: validated.handoff.receipt_digest,
        gate: *gate_info.key,
        target_program: validated.context.config.target_program,
        target_programdata: validated.context.config.target_programdata,
        upgradeable_loader: validated.context.config.upgradeable_loader,
        controller_authority: validated.context.config.authority_pda,
        bridge_artifact_length: validated.handoff.artifact_length,
        bridge_artifact_sha256: validated.handoff.artifact_sha256,
        bridge_artifact_merkle_root: validated.handoff.artifact_merkle_root,
        bridge_artifact_scheme_id: validated.handoff.artifact_scheme_id,
        bridge_source_commitment: validated.handoff.bridge_source_commitment,
        bridge_build_inputs_commitment: validated.handoff.bridge_build_inputs_commitment,
        bridge_package_commitment: validated.handoff.bridge_package_commitment,
        bridge_release_manifest_commitment: validated.handoff.bridge_release_manifest_commitment,
        bridge_observation: *observation_info.key,
        bridge_observation_generation: validated.observation.generation,
        bridge_observation_root: validated.observation.final_raw_merkle_root,
        bridge_observation_digest: validated.observation.observation_digest,
        minimum_target_deployed_slot: validated.observation.deployed_slot,
        minimum_target_capacity: validated.observation.actual_capacity,
        minimum_target_raw_length: validated.observation.raw_data_length,
        bootstrap_gate_status: validated.context.gate.status,
        bootstrap_gate_epoch: validated.context.gate.epoch,
        bootstrap_freeze_reason_code: validated.context.gate.freeze_reason_code,
        bootstrap_freeze_slot: validated.context.gate.freeze_slot,
        target_nonce: validated.context.config.target_nonce,
        council_version: validated.context.council.version,
        council_hash: validated.context.council.set_hash,
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
        reserved: [0; BOOTSTRAP_ACTIVATION_PROPOSAL_V1_RESERVED_LEN],
    };
    let mut proposal = proposal;
    proposal.proposal_digest = compute_bootstrap_activation_proposal_digest_v1(&proposal)?;
    validate_bootstrap_activation_proposal_digest_v1(&proposal)?;
    let proposal_bytes = encode_fixed_account(&proposal, BootstrapActivationProposalV1::LEN)?;
    let rent = Rent::get()?;
    let proposal_bump_seed = [proposal_bump];
    let council_version_seed = validated.context.council.version.to_le_bytes();
    let receipt_bump_seed = [receipt_bump];
    let deployment_bump_seed = [deployment_bump];
    create_fixed_pda_account(
        program_id,
        payer,
        proposal_info,
        system_program_info,
        &rent,
        BootstrapActivationProposalV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            BOOTSTRAP_ACTIVATION_SEED,
            validated.context.config.target_program.as_ref(),
            &council_version_seed,
            &proposal_bump_seed,
        ],
    )?;
    create_or_reuse_zero_fixed_pda(
        program_id,
        payer,
        activation_receipt_info,
        system_program_info,
        &rent,
        BootstrapActivationReceiptV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            BOOTSTRAP_ACTIVATION_RECEIPT_SEED,
            validated.context.config.target_program.as_ref(),
            &receipt_bump_seed,
        ],
    )?;
    create_or_reuse_zero_fixed_pda(
        program_id,
        payer,
        deployment_info,
        system_program_info,
        &rent,
        CurrentDeploymentStateV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            DEPLOYMENT_STATE_SEED,
            validated.context.config.target_program.as_ref(),
            &deployment_bump_seed,
        ],
    )?;
    commit_preencoded(program_id, &[(proposal_info, &proposal_bytes)])
}

pub fn process_approve_bootstrap_activation_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ApproveBootstrapActivationV1,
) -> ProgramResult {
    let [controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, handoff_receipt_info, observation_info, target_program, target_programdata, authority_info, proposal_info, loader_info, seat] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    debug_assert_eq!(
        accounts.len(),
        APPROVE_BOOTSTRAP_ACTIVATION_V1_ACCOUNT_COUNT
    );
    validate_activation_review_privileges(
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        observation_info,
        target_program,
        target_programdata,
        authority_info,
        proposal_info,
        loader_info,
    )?;
    validate_seat_authority(seat)?;
    require_distinct_accounts(&accounts.iter().collect::<Vec<_>>())?;
    let slot = Clock::get()?.slot;
    let evidence = validate_activation_evidence(
        program_id,
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        observation_info,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        slot,
    )?;
    reject_guardian(&evidence.context.config, seat.key)?;
    let mut proposal = load_activation_proposal(
        program_id,
        proposal_info,
        config_info,
        policy_info,
        capacity_info,
        gate_info,
        immutability_info,
        handoff_receipt_info,
        &evidence.context,
    )?;
    validate_activation_proposal_evidence(
        &proposal,
        &evidence,
        observation_info,
        immutability_info,
        handoff_receipt_info,
    )?;
    if proposal.state != CeremonyProposalStateV1::Draft
        || instruction.expected_proposal_digest != proposal.proposal_digest
        || instruction.expected_council_version != evidence.context.council.version
        || instruction.expected_gate_epoch != evidence.context.gate.epoch
        || instruction.expected_target_nonce != evidence.context.config.target_nonce
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_approval_window_activation(&proposal, slot)?;
    let (bitset, count) = record_seat_approval(
        &evidence.context.council,
        proposal.approval_bitset,
        proposal.approval_count,
        seat.key,
        slot,
    )?;
    proposal.approval_bitset = bitset;
    proposal.approval_count = count;
    if proposal.first_approval_slot == 0 {
        proposal.first_approval_slot = slot;
    }
    if count == RELEASE1_APPROVAL_THRESHOLD {
        proposal.state = CeremonyProposalStateV1::CouncilApproved;
        proposal.council_approved_slot = slot;
    }
    validate_bootstrap_activation_proposal_digest_v1(&proposal)?;
    let bytes = encode_fixed_account(&proposal, BootstrapActivationProposalV1::LEN)?;
    commit_preencoded(program_id, &[(proposal_info, &bytes)])
}

pub fn process_queue_bootstrap_activation_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: QueueBootstrapActivationV1,
) -> ProgramResult {
    let [controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, handoff_receipt_info, observation_info, target_program, target_programdata, authority_info, proposal_info, loader_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    debug_assert_eq!(accounts.len(), QUEUE_BOOTSTRAP_ACTIVATION_V1_ACCOUNT_COUNT);
    validate_activation_review_privileges(
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        observation_info,
        target_program,
        target_programdata,
        authority_info,
        proposal_info,
        loader_info,
    )?;
    require_distinct_accounts(&accounts.iter().collect::<Vec<_>>())?;
    let slot = Clock::get()?.slot;
    let evidence = validate_activation_evidence(
        program_id,
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        observation_info,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        slot,
    )?;
    let mut proposal = load_activation_proposal(
        program_id,
        proposal_info,
        config_info,
        policy_info,
        capacity_info,
        gate_info,
        immutability_info,
        handoff_receipt_info,
        &evidence.context,
    )?;
    validate_activation_proposal_evidence(
        &proposal,
        &evidence,
        observation_info,
        immutability_info,
        handoff_receipt_info,
    )?;
    if proposal.state != CeremonyProposalStateV1::CouncilApproved
        || instruction.expected_proposal_digest != proposal.proposal_digest
        || instruction.expected_council_version != evidence.context.council.version
        || instruction.expected_gate_epoch != evidence.context.gate.epoch
        || instruction.expected_target_nonce != evidence.context.config.target_nonce
        || slot >= proposal.expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_exact_quorum(
        &evidence.context.council,
        proposal.approval_bitset,
        proposal.approval_count,
        slot,
    )?;
    proposal.state = CeremonyProposalStateV1::Timelocked;
    proposal.queued_slot = slot;
    validate_bootstrap_activation_proposal_digest_v1(&proposal)?;
    let bytes = encode_fixed_account(&proposal, BootstrapActivationProposalV1::LEN)?;
    commit_preencoded(program_id, &[(proposal_info, &bytes)])
}

/// Activates the bootstrap gate in a separate, closed, no-CPI transaction.
pub fn process_execute_bootstrap_activation_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExecuteBootstrapActivationV1,
) -> ProgramResult {
    let [controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, handoff_receipt_info, proposal_info, observation_info, target_program, target_programdata, authority_info, loader_info, activation_receipt_info, deployment_info, instructions_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    debug_assert_eq!(
        accounts.len(),
        EXECUTE_BOOTSTRAP_ACTIVATION_V1_ACCOUNT_COUNT
    );
    validate_exact_privileges(controller_program, false, false, true)?;
    for account in [
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        observation_info,
        target_programdata,
        authority_info,
        instructions_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(gate_info, true, false, false)?;
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(target_program, false, false, true)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(activation_receipt_info, true, false, false)?;
    validate_exact_privileges(deployment_info, true, false, false)?;
    require_distinct_accounts(&accounts.iter().collect::<Vec<_>>())?;

    let slot = Clock::get()?.slot;
    let evidence = validate_activation_evidence(
        program_id,
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        observation_info,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        slot,
    )?;
    let mut proposal = load_activation_proposal(
        program_id,
        proposal_info,
        config_info,
        policy_info,
        capacity_info,
        gate_info,
        immutability_info,
        handoff_receipt_info,
        &evidence.context,
    )?;
    validate_activation_proposal_evidence(
        &proposal,
        &evidence,
        observation_info,
        immutability_info,
        handoff_receipt_info,
    )?;
    if *instructions_info.key != sysvar_ids::instructions::ID
        || proposal.state != CeremonyProposalStateV1::Timelocked
        || instruction.expected_proposal_digest != proposal.proposal_digest
        || instruction.expected_bridge_observation_digest != evidence.observation.observation_digest
        || instruction.expected_gate_epoch != evidence.context.gate.epoch
        || instruction.expected_target_nonce != evidence.context.config.target_nonce
        || slot < proposal.not_before_slot
        || slot >= proposal.expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_exact_quorum(
        &evidence.context.council,
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

    let (expected_receipt, receipt_bump) = derive_bootstrap_activation_receipt_pda(
        program_id,
        &evidence.context.config.target_program,
    );
    let (expected_deployment, deployment_bump) =
        derive_current_deployment_state_pda(program_id, &evidence.context.config.target_program);
    if *activation_receipt_info.key != expected_receipt
        || *deployment_info.key != expected_deployment
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    require_zero_initialized_destination(
        program_id,
        activation_receipt_info,
        BootstrapActivationReceiptV1::LEN,
    )?;
    require_zero_initialized_destination(
        program_id,
        deployment_info,
        CurrentDeploymentStateV1::LEN,
    )?;

    let next_epoch = evidence
        .context
        .gate
        .epoch
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let mut next_gate = (*evidence.context.gate).clone();
    next_gate.status = GateStatusV1::Active;
    next_gate.epoch = next_epoch;
    next_gate.active_proposal = Pubkey::default();
    next_gate.freeze_slot = 0;
    next_gate.freeze_reason_code = 0;
    next_gate.last_completed_proposal = *proposal_info.key;
    next_gate.validate_static()?;

    proposal.state = CeremonyProposalStateV1::Completed;
    proposal.executed_slot = slot;
    proposal.terminal_slot = slot;
    proposal.terminal_reason_code = CEREMONY_PROPOSAL_COMPLETED_REASON_V1;
    validate_bootstrap_activation_proposal_digest_v1(&proposal)?;

    let mut deployment = CurrentDeploymentStateV1 {
        discriminator: CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: deployment_bump,
        initialized: true,
        controller_program: *program_id,
        controller_config: *config_info.key,
        capacity_policy: *capacity_info.key,
        capacity_policy_digest: evidence.context.capacity.policy_digest,
        target_program: evidence.context.config.target_program,
        target_programdata: evidence.context.config.target_programdata,
        upgradeable_loader: evidence.context.config.upgradeable_loader,
        controller_authority: evidence.context.config.authority_pda,
        artifact_length: evidence.handoff.artifact_length,
        artifact_sha256: evidence.handoff.artifact_sha256,
        artifact_merkle_root: evidence.handoff.artifact_merkle_root,
        artifact_scheme_id: evidence.handoff.artifact_scheme_id,
        actual_programdata_capacity: evidence.observation.actual_capacity,
        programdata_observation: *observation_info.key,
        observation_generation: evidence.observation.generation,
        observation_root: evidence.observation.final_raw_merkle_root,
        observation_digest: evidence.observation.observation_digest,
        deployed_slot: evidence.observation.deployed_slot,
        installed_authority: evidence.context.config.authority_pda,
        source_commitment: evidence.handoff.bridge_source_commitment,
        build_inputs_commitment: evidence.handoff.bridge_build_inputs_commitment,
        package_commitment: evidence.handoff.bridge_package_commitment,
        release_manifest_commitment: evidence.handoff.bridge_release_manifest_commitment,
        release_commitment: *handoff_receipt_info.key,
        release_commitment_digest: evidence.handoff.receipt_digest,
        activation_receipt: OptionalPubkeyV1::some(*activation_receipt_info.key)?,
        completed_proposal: OptionalPubkeyV1::none(),
        gate_epoch_at_activation: next_epoch,
        deployment_generation: 1,
        deployment_digest: [0; 32],
        last_updated_slot: slot,
        reserved: [0; CURRENT_DEPLOYMENT_STATE_V1_RESERVED_LEN],
    };
    if compute_bootstrap_activation_deployment_plan_digest_v1(&deployment)?
        != instruction.expected_deployment_plan_digest
    {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    deployment.deployment_digest = compute_current_deployment_digest_v1(&deployment)?;
    validate_current_deployment_digest_v1(&deployment)?;

    let mut receipt = BootstrapActivationReceiptV1 {
        discriminator: BOOTSTRAP_ACTIVATION_RECEIPT_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: receipt_bump,
        initialized: true,
        proposal: *proposal_info.key,
        proposal_digest: proposal.proposal_digest,
        controller_program: *program_id,
        controller_config: *config_info.key,
        governance_policy: *policy_info.key,
        governance_policy_hash: evidence.context.policy.policy_hash,
        capacity_policy: *capacity_info.key,
        capacity_policy_digest: evidence.context.capacity.policy_digest,
        controller_immutability_receipt: *immutability_info.key,
        controller_immutability_digest: evidence.immutable.receipt_digest,
        target_handoff_receipt: *handoff_receipt_info.key,
        target_handoff_digest: evidence.handoff.receipt_digest,
        gate: *gate_info.key,
        target_program: evidence.context.config.target_program,
        target_programdata: evidence.context.config.target_programdata,
        upgradeable_loader: evidence.context.config.upgradeable_loader,
        controller_authority: evidence.context.config.authority_pda,
        bridge_observation: *observation_info.key,
        bridge_observation_generation: evidence.observation.generation,
        bridge_observation_root: evidence.observation.final_raw_merkle_root,
        bridge_observation_digest: evidence.observation.observation_digest,
        bridge_artifact_length: evidence.handoff.artifact_length,
        bridge_artifact_sha256: evidence.handoff.artifact_sha256,
        bridge_artifact_merkle_root: evidence.handoff.artifact_merkle_root,
        bridge_artifact_scheme_id: evidence.handoff.artifact_scheme_id,
        actual_target_capacity: evidence.observation.actual_capacity,
        target_deployed_slot: evidence.observation.deployed_slot,
        previous_gate_status: GateStatusV1::EmergencyFrozen,
        previous_gate_epoch: evidence.context.gate.epoch,
        previous_freeze_reason_code: evidence.context.gate.freeze_reason_code,
        previous_freeze_slot: evidence.context.gate.freeze_slot,
        activated_gate_status: GateStatusV1::Active,
        activated_gate_epoch: next_epoch,
        target_nonce: evidence.context.config.target_nonce,
        council_version: evidence.context.council.version,
        council_hash: evidence.context.council.set_hash,
        current_deployment_state: *deployment_info.key,
        current_deployment_digest: deployment.deployment_digest,
        deployment_generation: deployment.deployment_generation,
        finalized_slot: slot,
        receipt_digest: [0; 32],
        finalized: true,
        reserved: [0; BOOTSTRAP_ACTIVATION_RECEIPT_V1_RESERVED_LEN],
    };
    if compute_bootstrap_activation_receipt_plan_digest_v1(&receipt)?
        != instruction.expected_receipt_plan_digest
    {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    receipt.receipt_digest = compute_bootstrap_activation_receipt_digest_v1(&receipt)?;
    crate::release1_ceremony_digest::validate_bootstrap_activation_receipt_digest_v1(&receipt)?;

    let gate_bytes = encode_fixed_account(&next_gate, ProtocolGateV1::LEN)?;
    let proposal_bytes = encode_fixed_account(&proposal, BootstrapActivationProposalV1::LEN)?;
    let receipt_bytes = encode_fixed_account(&receipt, BootstrapActivationReceiptV1::LEN)?;
    let deployment_bytes = encode_fixed_account(&deployment, CurrentDeploymentStateV1::LEN)?;
    commit_preencoded(
        program_id,
        &[
            (gate_info, &gate_bytes),
            (proposal_info, &proposal_bytes),
            (activation_receipt_info, &receipt_bytes),
            (deployment_info, &deployment_bytes),
        ],
    )
}
