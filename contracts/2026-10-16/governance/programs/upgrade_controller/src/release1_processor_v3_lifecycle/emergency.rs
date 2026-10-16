use super::*;

pub fn process_create_emergency_resolution_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CreateEmergencyResolutionV2,
) -> ProgramResult {
    exact_account_count(accounts, 12)?;
    all_distinct(accounts)?;
    let [payer, creator, config_info, policy_info, council_info, gate_info, capacity_info, deployment_info, freeze_observation_info, programdata_observation_info, resolution_info, system_program_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    signer_writable(payer)?;
    validate_seat_authority(creator)?;
    for info in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        deployment_info,
        freeze_observation_info,
        programdata_observation_info,
    ] {
        readonly(info)?;
    }
    writable(resolution_info)?;
    system_program_account(system_program_info)?;

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
    if *creator.key == context.config.guardian
        || !council
            .seats
            .iter()
            .any(|seat| seat.seat_authority == *creator.key && seat.term_covers(slot))
    {
        return Err(GovernanceError::InactiveCouncilSeat.into());
    }
    let freeze_observation = load_emergency_freeze_observation(
        program_id,
        freeze_observation_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
    )?;
    let manifest = &instruction.manifest;
    require_guard_deployment(
        &context.capacity,
        &context.deployment,
        &manifest.expected_capacity_policy_digest,
        &manifest.expected_current_deployment_digest,
        manifest.expected_current_deployment_generation,
    )?;
    if slot > manifest.plan_valid_until_slot
        || context.gate.epoch != manifest.expected_gate_epoch
        || context.config.target_nonce != manifest.expected_target_nonce
        || freeze_observation.observation_digest != manifest.expected_freeze_observation_digest
        || policy.version != manifest.expected_policy_version
        || policy.policy_hash != manifest.expected_policy_hash
        || council.version != manifest.expected_council_version
        || council.set_hash != manifest.expected_council_hash
        || slot < context.gate.freeze_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let observation_expectation = trusted_observation_expectation(&context.deployment);
    let observation_subject_digest = expected_programdata_observation_subject_digest(
        program_id,
        config_info,
        gate_info,
        freeze_observation_info.key,
        &context.config,
        &context.gate,
        &context.capacity,
        &observation_expectation,
        ProgramDataObservationPurposeV1::EmergencyResolution,
        manifest.expected_programdata_observation_generation,
        context.deployment.artifact_length,
    )?;
    let observation = load_fresh_programdata_observation(
        program_id,
        programdata_observation_info,
        config_info,
        gate_info,
        capacity_info,
        freeze_observation_info.key,
        &context.config,
        &context.gate,
        &context.capacity,
        &observation_expectation,
        ProgramDataObservationPurposeV1::EmergencyResolution,
        &observation_subject_digest,
        manifest.expected_programdata_observation_generation,
        &manifest.expected_programdata_observation_digest,
        context.deployment.artifact_length,
        slot,
    )?;
    let (expected_resolution, resolution_bump) = derive_emergency_resolution_v2_pda(
        program_id,
        &context.config.target_program,
        context.gate.epoch,
        council.version,
    );
    if *resolution_info.key != expected_resolution {
        return Err(GovernanceError::InvalidPda.into());
    }
    let review_start_slot = slot
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let review_end_slot = review_start_slot
        .checked_add(context.config.council_review_slots())
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let delay_end = context
        .gate
        .freeze_slot
        .checked_add(context.config.routine_delay_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let not_before_slot = review_end_slot.max(delay_end);
    let expiry_slot = context
        .gate
        .freeze_slot
        .checked_add(context.config.proposal_expiry_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if not_before_slot >= expiry_slot || slot >= expiry_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let mut resolution = EmergencyFreezeResolutionV2 {
        discriminator: EMERGENCY_FREEZE_RESOLUTION_V2_DISCRIMINATOR,
        account_version: CAPACITY_SAFE_ACCOUNT_VERSION_V2,
        bump: resolution_bump,
        initialized: true,
        state: EmergencyFreezeResolutionStateV1::Draft,
        controller_config: *config_info.key,
        protocol_gate: *gate_info.key,
        target_program: context.config.target_program,
        target_programdata: context.config.target_programdata,
        upgradeable_loader: context.config.upgradeable_loader,
        controller_authority: context.config.authority_pda,
        capacity_policy: *capacity_info.key,
        capacity_policy_digest: context.capacity.policy_digest,
        current_deployment_state: *deployment_info.key,
        current_deployment_digest: context.deployment.deployment_digest,
        current_deployment_generation: context.deployment.deployment_generation,
        emergency_freeze_observation: *freeze_observation_info.key,
        emergency_freeze_observation_digest: freeze_observation.observation_digest,
        frozen_epoch: context.gate.epoch,
        freeze_slot: context.gate.freeze_slot,
        freeze_reason_code: context.gate.freeze_reason_code,
        resolution_kind: manifest.resolution_kind,
        creation_slot: slot,
        review_start_slot,
        review_end_slot,
        not_before_slot,
        expiry_slot,
        target_nonce: context.config.target_nonce,
        artifact_length: context.deployment.artifact_length,
        artifact_sha256: context.deployment.artifact_sha256,
        artifact_merkle_root: context.deployment.artifact_merkle_root,
        artifact_scheme_id: context.deployment.artifact_scheme_id,
        minimum_required_capacity: context.deployment.artifact_length,
        observation_scheme_id: context.capacity.observation_scheme_id,
        programdata_observation: *programdata_observation_info.key,
        observation_purpose: ProgramDataObservationPurposeV1::EmergencyResolution,
        observation_generation: observation.generation,
        observation_subject_digest: observation.subject_digest,
        observation_root: observation.final_raw_merkle_root,
        observation_digest: observation.observation_digest,
        observation_finalized_slot: observation.finalized_slot,
        observed_deployed_slot: observation.deployed_slot,
        observed_raw_data_length: observation.raw_data_length,
        actual_capacity: observation.actual_capacity,
        observed_authority: observation.upgrade_authority,
        emergency_checkpoint: derive_emergency_checkpoint_v2_pda(program_id, resolution_info.key).0,
        emergency_checkpoint_digest: [0; 32],
        approval_council_version: council.version,
        approval_council_hash: council.set_hash,
        approval_bitset: 0,
        approval_count: 0,
        approval_threshold: RELEASE1_APPROVAL_THRESHOLD,
        resolution_digest_domain_id: EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_DOMAIN_ID,
        resolution_digest: [0; 32],
        first_approval_slot: 0,
        council_approved_slot: 0,
        queued_slot: 0,
        executed_slot: 0,
        terminal_slot: 0,
        terminal_reason_code: 0,
        reserved: [0; EMERGENCY_FREEZE_RESOLUTION_V2_RESERVED_LEN],
    };
    resolution.resolution_digest = compute_emergency_freeze_resolution_digest_v2(&resolution)?;
    validate_emergency_freeze_resolution_digest_v2(&resolution)?;
    let resolution_bytes = encode_fixed_account(&resolution, EmergencyFreezeResolutionV2::LEN)?;
    let epoch = context.gate.epoch.to_le_bytes();
    let council_version = council.version.to_le_bytes();
    let bump = [resolution_bump];
    let seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        EMERGENCY_RESOLUTION_V2_SEED,
        context.config.target_program.as_ref(),
        &epoch,
        &council_version,
        &bump,
    ];
    create_fixed_pda_account(
        program_id,
        payer,
        resolution_info,
        system_program_info,
        &Rent::get()?,
        EmergencyFreezeResolutionV2::LEN,
        seeds,
    )?;
    resolution_info
        .try_borrow_mut_data()?
        .copy_from_slice(&resolution_bytes);
    Ok(())
}

pub fn process_approve_emergency_resolution_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ApproveEmergencyResolutionV2,
) -> ProgramResult {
    exact_account_count(accounts, 11)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, capacity_info, deployment_info, freeze_observation_info, programdata_observation_info, checkpoint_info, resolution_info, seat] =
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
        freeze_observation_info,
        programdata_observation_info,
        checkpoint_info,
    ] {
        readonly(info)?;
    }
    writable(resolution_info)?;
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
    let mut resolution = load_emergency_resolution(
        program_id,
        resolution_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
    )?;
    check_emergency_resolution_guard(&instruction.expected, &resolution, &context)?;
    if resolution.approval_council_version != council.version
        || resolution.approval_council_hash != council.set_hash
        || resolution.state != EmergencyFreezeResolutionStateV1::Draft
        || slot < resolution.review_start_slot
        || slot > resolution.review_end_slot
        || slot >= resolution.expiry_slot
        || instruction.expected_approval_bitset != resolution.approval_bitset
        || instruction.expected_approval_count != resolution.approval_count
        || *seat.key == context.config.guardian
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let (_, _, checkpoint) = validate_emergency_evidence(
        program_id,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        freeze_observation_info,
        programdata_observation_info,
        checkpoint_info,
        resolution_info,
        &context,
        &council,
        &resolution,
        slot,
    )?;
    if resolution.emergency_checkpoint_digest != checkpoint.checkpoint_digest {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let (bitset, count) = record_seat_approval(
        &council,
        resolution.approval_bitset,
        resolution.approval_count,
        seat.key,
        slot,
    )?;
    if resolution.approval_count == 0 {
        resolution.first_approval_slot = slot;
    }
    resolution.approval_bitset = bitset;
    resolution.approval_count = count;
    if count == RELEASE1_APPROVAL_THRESHOLD {
        resolution.state = EmergencyFreezeResolutionStateV1::CouncilApproved;
        resolution.council_approved_slot = slot;
    }
    validate_emergency_freeze_resolution_digest_v2(&resolution)?;
    store_fixed_controller_account(
        program_id,
        resolution_info,
        &*resolution,
        EmergencyFreezeResolutionV2::LEN,
    )
}

pub fn process_queue_emergency_resolution_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: QueueEmergencyResolutionV2,
) -> ProgramResult {
    exact_account_count(accounts, 10)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, capacity_info, deployment_info, freeze_observation_info, programdata_observation_info, checkpoint_info, resolution_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for info in &accounts[..9] {
        readonly(info)?;
    }
    writable(resolution_info)?;
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
    let mut resolution = load_emergency_resolution(
        program_id,
        resolution_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
    )?;
    check_emergency_resolution_guard(&instruction.expected, &resolution, &context)?;
    if resolution.state != EmergencyFreezeResolutionStateV1::CouncilApproved
        || resolution.approval_council_version != council.version
        || resolution.approval_council_hash != council.set_hash
        || slot >= resolution.expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_exact_recorded_quorum(
        &council,
        resolution.approval_bitset,
        resolution.approval_count,
        resolution.council_approved_slot,
    )?;
    validate_emergency_evidence(
        program_id,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        freeze_observation_info,
        programdata_observation_info,
        checkpoint_info,
        resolution_info,
        &context,
        &council,
        &resolution,
        slot,
    )?;
    resolution.state = EmergencyFreezeResolutionStateV1::Timelocked;
    resolution.queued_slot = slot;
    validate_emergency_freeze_resolution_digest_v2(&resolution)?;
    store_fixed_controller_account(
        program_id,
        resolution_info,
        &*resolution,
        EmergencyFreezeResolutionV2::LEN,
    )
}

pub fn process_execute_emergency_resolution_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExecuteEmergencyResolutionV2,
) -> ProgramResult {
    exact_account_count(accounts, 15)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, capacity_info, deployment_info, resolution_info, freeze_observation_info, programdata_observation_info, checkpoint_info, target_program, target_programdata, loader, authority, instructions_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for info in [
        config_info,
        policy_info,
        council_info,
        capacity_info,
        freeze_observation_info,
        programdata_observation_info,
        checkpoint_info,
        target_programdata,
        authority,
        instructions_info,
    ] {
        readonly(info)?;
    }
    for info in [gate_info, deployment_info, resolution_info] {
        writable(info)?;
    }
    executable_readonly(target_program)?;
    executable_readonly(loader)?;
    if *loader.key != UPGRADEABLE_LOADER_ID
        || *instructions_info.key != sysvar_ids::instructions::ID
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
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
    let council = load_current_council(
        program_id,
        council_info,
        config_info,
        &context.config,
        &policy,
        slot,
    )?;
    let mut resolution = load_emergency_resolution(
        program_id,
        resolution_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
    )?;
    check_emergency_resolution_guard(&instruction.expected, &resolution, &context)?;
    if resolution.state != EmergencyFreezeResolutionStateV1::Timelocked
        || resolution.resolution_kind != EmergencyFreezeResolutionKindV1::ResumeWithoutUpgrade
        || resolution.approval_council_version != council.version
        || resolution.approval_council_hash != council.set_hash
        || slot < resolution.not_before_slot
        || slot >= resolution.expiry_slot
        || *authority.key != context.config.authority_pda
        || *loader.key != context.config.upgradeable_loader
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_exact_recorded_quorum(
        &council,
        resolution.approval_bitset,
        resolution.approval_count,
        resolution.council_approved_slot,
    )?;
    let (_, observation, checkpoint) = validate_emergency_evidence(
        program_id,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        freeze_observation_info,
        programdata_observation_info,
        checkpoint_info,
        resolution_info,
        &context,
        &council,
        &resolution,
        slot,
    )?;
    if checkpoint.finalized_slot > slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    require_observation_matches_runtime(
        &observation,
        target_program,
        target_programdata,
        &context.config,
        &context.capacity,
    )?;
    validate_canonical_envelope(
        program_id,
        accounts,
        instructions_info,
        &instruction.pack()?,
        &instruction.envelope,
    )?;

    let next_epoch = checked_nonterminal_increment(context.gate.epoch)?;
    let next_deployment_generation =
        checked_nonterminal_increment(context.deployment.deployment_generation)?;
    context.gate.status = GateStatusV1::Active;
    context.gate.epoch = next_epoch;
    context.gate.active_proposal = Pubkey::default();
    context.gate.freeze_slot = 0;
    context.gate.freeze_reason_code = 0;
    context.gate.validate_static()?;

    context.deployment.programdata_observation = *programdata_observation_info.key;
    context.deployment.observation_generation = observation.generation;
    context.deployment.observation_root = observation.final_raw_merkle_root;
    context.deployment.observation_digest = observation.observation_digest;
    context.deployment.deployed_slot = observation.deployed_slot;
    context.deployment.actual_programdata_capacity = observation.actual_capacity;
    context.deployment.installed_authority = context.config.authority_pda;
    context.deployment.gate_epoch_at_activation = next_epoch;
    context.deployment.deployment_generation = next_deployment_generation;
    context.deployment.last_updated_slot = slot;
    context.deployment.deployment_digest =
        compute_current_deployment_digest_v1(&context.deployment)?;
    validate_current_deployment_digest_v1(&context.deployment)?;

    resolution.state = EmergencyFreezeResolutionStateV1::Executed;
    resolution.executed_slot = slot;
    resolution.terminal_slot = slot;
    resolution.terminal_reason_code = EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V2;
    validate_emergency_freeze_resolution_digest_v2(&resolution)?;

    let gate_bytes = encode_fixed_account(&*context.gate, ProtocolGateV1::LEN)?;
    let deployment_bytes =
        encode_fixed_account(&*context.deployment, CurrentDeploymentStateV1::LEN)?;
    let resolution_bytes = encode_fixed_account(&*resolution, EmergencyFreezeResolutionV2::LEN)?;
    gate_info
        .try_borrow_mut_data()?
        .copy_from_slice(&gate_bytes);
    deployment_info
        .try_borrow_mut_data()?
        .copy_from_slice(&deployment_bytes);
    resolution_info
        .try_borrow_mut_data()?
        .copy_from_slice(&resolution_bytes);
    Ok(())
}

pub fn process_expire_emergency_resolution_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExpireEmergencyResolutionV2,
) -> ProgramResult {
    exact_account_count(accounts, 5)?;
    all_distinct(accounts)?;
    let [config_info, gate_info, capacity_info, deployment_info, resolution_info] = accounts else {
        unreachable!("account count checked")
    };
    for info in [config_info, gate_info, capacity_info, deployment_info] {
        readonly(info)?;
    }
    writable(resolution_info)?;
    let context = load_lifecycle_context(
        program_id,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
    )?;
    let mut resolution = load_emergency_resolution(
        program_id,
        resolution_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
    )?;
    check_emergency_resolution_guard(&instruction.expected, &resolution, &context)?;
    let slot = current_slot()?;
    if !matches!(
        resolution.state,
        EmergencyFreezeResolutionStateV1::Draft
            | EmergencyFreezeResolutionStateV1::CouncilApproved
            | EmergencyFreezeResolutionStateV1::Timelocked
    ) || slot < resolution.expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    resolution.state = EmergencyFreezeResolutionStateV1::Expired;
    resolution.terminal_slot = slot;
    resolution.terminal_reason_code = EMERGENCY_RESOLUTION_EXPIRED_TERMINAL_REASON_V2;
    validate_emergency_freeze_resolution_digest_v2(&resolution)?;
    store_fixed_controller_account(
        program_id,
        resolution_info,
        &*resolution,
        EmergencyFreezeResolutionV2::LEN,
    )
}
