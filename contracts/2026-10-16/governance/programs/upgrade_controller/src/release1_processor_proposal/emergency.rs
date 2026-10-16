use super::*;

pub fn process_guardian_freeze_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: GuardianFreezeV1,
) -> ProgramResult {
    let [payer, config_info, gate_info, target_program, target_programdata, loader, authority, guardian, observation_info, system_program_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_exact_privileges(payer, true, true, false)?;
    validate_exact_privileges(config_info, false, false, false)?;
    validate_exact_privileges(gate_info, true, false, false)?;
    validate_exact_privileges(
        target_program,
        false,
        false,
        instruction.expected_program_executable,
    )?;
    validate_exact_privileges(
        target_programdata,
        false,
        false,
        instruction.expected_programdata_executable,
    )?;
    validate_exact_privileges(loader, false, false, true)?;
    validate_exact_privileges(authority, false, false, false)?;
    validate_exact_privileges(guardian, false, true, false)?;
    validate_exact_privileges(observation_info, true, false, false)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&[
        payer,
        config_info,
        gate_info,
        target_program,
        target_programdata,
        loader,
        authority,
        guardian,
        observation_info,
        system_program_info,
    ])?;
    let config = load_config(program_id, config_info)?;
    let mut gate = load_gate(program_id, gate_info, config_info, &config)?;
    let slot = Clock::get()?.slot;
    let next_epoch = gate
        .epoch
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if gate.status != GateStatusV1::Active
        || instruction.expected_gate_status != gate.status
        || instruction.expected_gate_epoch != gate.epoch
        || instruction.expected_next_gate_epoch != next_epoch
        || instruction.expected_target_nonce != config.target_nonce
        || instruction.freeze_reason_code == 0
        || instruction.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        || *guardian.key != config.guardian
        || *target_program.key != config.target_program
        || *target_programdata.key != config.target_programdata
        || *loader.key != config.upgradeable_loader
        || *authority.key != config.authority_pda
        || *system_program_info.key != system_program::ID
        || slot == 0
    {
        return Err(GovernanceError::InvalidGateState.into());
    }
    validate_loader_identity(loader, &config)?;
    let runtime = capture_runtime_observation(target_program, target_programdata)?;
    compare_guardian_instruction_observation(&instruction, &runtime)?;

    let (expected_observation, observation_bump) =
        derive_emergency_freeze_observation_pda(program_id, &config.target_program, next_epoch);
    if *observation_info.key != expected_observation {
        return Err(GovernanceError::InvalidPda.into());
    }
    let mut observation = EmergencyFreezeObservationV1 {
        discriminator: EMERGENCY_FREEZE_OBSERVATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: observation_bump,
        initialized: true,
        finalized: true,
        controller_program: *program_id,
        controller_config: *config_info.key,
        protocol_gate: *gate_info.key,
        target_program: config.target_program,
        target_programdata: config.target_programdata,
        upgradeable_loader: config.upgradeable_loader,
        controller_authority: config.authority_pda,
        frozen_epoch: next_epoch,
        freeze_slot: slot,
        freeze_reason_code: instruction.freeze_reason_code,
        actual_program_owner: runtime.program_owner,
        actual_program_executable: runtime.program_executable,
        actual_program_data_length: runtime.program_data_length,
        program_header_present: runtime.program_header_present,
        actual_linked_programdata: runtime.linked_programdata,
        actual_programdata_owner: runtime.programdata_owner,
        actual_programdata_executable: runtime.programdata_executable,
        actual_programdata_data_length: runtime.programdata_data_length,
        programdata_header_present: runtime.programdata_header_present,
        deployed_programdata_slot: runtime.programdata_slot,
        raw_hash_complete: runtime.raw_hash_complete,
        raw_programdata_sha256: runtime.raw_programdata_hash,
        capacity: runtime.capacity,
        observed_authority: runtime.authority,
        observation_digest: [0; 32],
        finalized_slot: slot,
        reserved: [0; EMERGENCY_FREEZE_OBSERVATION_V1_RESERVED_LEN],
    };
    observation.observation_digest = compute_emergency_freeze_observation_digest_v1(&observation)?;
    if instruction.expected_observation_digest != observation.observation_digest {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    validate_emergency_freeze_observation_digest_v1(&observation)?;
    gate.status = GateStatusV1::EmergencyFrozen;
    gate.epoch = next_epoch;
    gate.active_proposal = Pubkey::default();
    gate.freeze_slot = slot;
    gate.freeze_reason_code = instruction.freeze_reason_code;
    gate.validate_static()?;
    let _observation_bytes = encode_fixed_account(&observation, EmergencyFreezeObservationV1::LEN)?;
    let _gate_bytes = encode_fixed_account(&*gate, ProtocolGateV1::LEN)?;

    let epoch_bytes = next_epoch.to_le_bytes();
    let bump_seed = [observation_bump];
    let signer_seeds: [&[u8]; 5] = [
        UPGRADE_SEED_DOMAIN_V1,
        EMERGENCY_FREEZE_OBSERVATION_SEED,
        config.target_program.as_ref(),
        &epoch_bytes,
        &bump_seed,
    ];
    create_fixed_pda_account(
        program_id,
        payer,
        observation_info,
        system_program_info,
        &Rent::get()?,
        EmergencyFreezeObservationV1::LEN,
        &signer_seeds,
    )?;
    store_fixed_controller_account(
        program_id,
        observation_info,
        &observation,
        EmergencyFreezeObservationV1::LEN,
    )?;
    store_fixed_controller_account(program_id, gate_info, &*gate, ProtocolGateV1::LEN)
}

pub fn process_create_emergency_resolution_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CreateEmergencyResolutionV1,
) -> ProgramResult {
    let [payer, config_info, policy_info, council_info, gate_info, observation_info, resolution_info, system_program_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_exact_privileges(payer, true, true, false)?;
    validate_readonly_state_accounts(&[
        config_info,
        policy_info,
        council_info,
        gate_info,
        observation_info,
    ])?;
    validate_exact_privileges(resolution_info, true, false, false)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&[
        payer,
        config_info,
        policy_info,
        council_info,
        gate_info,
        observation_info,
        resolution_info,
        system_program_info,
    ])?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
    let council = load_current_council(program_id, council_info, config_info, &config, &policy)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let observation = load_emergency_observation(
        program_id,
        observation_info,
        config_info,
        gate_info,
        &config,
        &gate,
    )?;
    let slot = Clock::get()?.slot;
    require_policy_active(&policy, slot)?;
    let not_before_slot = gate
        .freeze_slot
        .checked_add(config.routine_delay_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let expiry_slot = gate
        .freeze_slot
        .checked_add(config.proposal_expiry_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if gate.status != GateStatusV1::EmergencyFrozen
        || gate.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        || instruction.resolution_kind != EmergencyFreezeResolutionKindV1::ResumeWithoutUpgrade
        || instruction.creation_slot != slot
        || instruction.not_before_slot != not_before_slot
        || instruction.expiry_slot != expiry_slot
        || slot < gate.freeze_slot
        || slot >= expiry_slot
        || instruction.expected_policy_version != policy.version
        || instruction.expected_policy_hash != policy.policy_hash
        || instruction.expected_council_version != council.version
        || instruction.expected_council_hash != council.set_hash
        || instruction.expected_gate_epoch != gate.epoch
        || instruction.expected_freeze_slot != gate.freeze_slot
        || instruction.expected_freeze_reason_code != gate.freeze_reason_code
        || instruction.expected_target_nonce != config.target_nonce
        || instruction.expected_freeze_observation_digest != observation.observation_digest
        || *system_program_info.key != system_program::ID
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    compare_resolution_instruction_observation(&instruction, &observation)?;
    let (expected_resolution, resolution_bump) =
        derive_emergency_resolution_pda(program_id, &config.target_program, gate.epoch);
    if *resolution_info.key != expected_resolution {
        return Err(GovernanceError::InvalidPda.into());
    }
    let mut resolution = EmergencyFreezeResolutionV1 {
        discriminator: EMERGENCY_FREEZE_RESOLUTION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: resolution_bump,
        initialized: true,
        state: EmergencyFreezeResolutionStateV1::Draft,
        controller_config: *config_info.key,
        protocol_gate: *gate_info.key,
        target_program: config.target_program,
        target_programdata: config.target_programdata,
        emergency_freeze_observation: *observation_info.key,
        frozen_epoch: gate.epoch,
        freeze_slot: gate.freeze_slot,
        freeze_reason_code: gate.freeze_reason_code,
        resolution_kind: instruction.resolution_kind,
        creation_slot: slot,
        not_before_slot,
        expiry_slot,
        target_nonce: config.target_nonce,
        observed_program_owner: observation.actual_program_owner,
        observed_program_executable: observation.actual_program_executable,
        observed_program_data_length: observation.actual_program_data_length,
        observed_program_header_present: observation.program_header_present,
        observed_linked_programdata: observation.actual_linked_programdata,
        observed_programdata_owner: observation.actual_programdata_owner,
        observed_programdata_executable: observation.actual_programdata_executable,
        observed_programdata_data_length: observation.actual_programdata_data_length,
        observed_programdata_header_present: observation.programdata_header_present,
        observed_programdata_slot: observation.deployed_programdata_slot,
        observed_raw_hash_complete: observation.raw_hash_complete,
        observed_raw_programdata_hash: observation.raw_programdata_sha256,
        observed_capacity: observation.capacity,
        observed_authority: observation.observed_authority,
        emergency_checkpoint: derive_emergency_checkpoint_pda(
            program_id,
            &config.target_program,
            gate.epoch,
        )
        .0,
        approval_council_version: 0,
        approval_council_hash: [0; 32],
        approval_bitset: 0,
        approval_count: 0,
        resolution_digest: [0; 32],
        executed_slot: 0,
        cancellation_reason_code: 0,
        terminal_reason_code: 0,
        reserved: [0; EMERGENCY_FREEZE_RESOLUTION_V1_RESERVED_LEN],
    };
    resolution.resolution_digest = compute_emergency_resolution_digest_v1(&resolution)?;
    if resolution.resolution_digest != instruction.expected_resolution_digest {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    validate_emergency_resolution_digest_v1(&resolution)?;
    let _resolution_bytes = encode_fixed_account(&resolution, EmergencyFreezeResolutionV1::LEN)?;
    let epoch_bytes = gate.epoch.to_le_bytes();
    let bump_seed = [resolution_bump];
    let signer_seeds: [&[u8]; 5] = [
        UPGRADE_SEED_DOMAIN_V1,
        EMERGENCY_RESOLUTION_SEED,
        config.target_program.as_ref(),
        &epoch_bytes,
        &bump_seed,
    ];
    create_fixed_pda_account(
        program_id,
        payer,
        resolution_info,
        system_program_info,
        &Rent::get()?,
        EmergencyFreezeResolutionV1::LEN,
        &signer_seeds,
    )?;
    store_fixed_controller_account(
        program_id,
        resolution_info,
        &resolution,
        EmergencyFreezeResolutionV1::LEN,
    )
}

pub fn process_approve_emergency_resolution_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ApproveEmergencyResolutionV1,
) -> ProgramResult {
    let [config_info, policy_info, council_info, gate_info, resolution_info, seat] = accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_readonly_state_accounts(&[config_info, policy_info, council_info, gate_info])?;
    validate_exact_privileges(resolution_info, true, false, false)?;
    validate_seat_authority(seat)?;
    require_distinct_accounts(&[
        config_info,
        policy_info,
        council_info,
        gate_info,
        resolution_info,
        seat,
    ])?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
    let council = load_current_council(program_id, council_info, config_info, &config, &policy)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let mut resolution =
        load_resolution(program_id, resolution_info, config_info, gate_info, &config)?;
    let slot = Clock::get()?.slot;
    require_policy_active(&policy, slot)?;
    check_emergency_expectation(
        &instruction.expected,
        &resolution,
        &config,
        &policy,
        &council,
        &gate,
    )?;
    require_emergency_binding(&resolution, &config, &gate, slot, false)?;
    if slot < resolution.not_before_slot
        || instruction.expected_approval_bitset != resolution.approval_bitset
        || instruction.expected_approval_count != resolution.approval_count
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    reject_guardian_authority(&config, seat.key)?;
    let stale = resolution.approval_count != 0
        && (resolution.approval_council_version != council.version
            || resolution.approval_council_hash != council.set_hash);
    if resolution.state != EmergencyFreezeResolutionStateV1::Draft
        && !(stale
            && matches!(
                resolution.state,
                EmergencyFreezeResolutionStateV1::CouncilApproved
                    | EmergencyFreezeResolutionStateV1::Timelocked
            ))
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    if stale {
        resolution.state = EmergencyFreezeResolutionStateV1::Draft;
        resolution.approval_council_version = 0;
        resolution.approval_council_hash = [0; 32];
        resolution.approval_bitset = 0;
        resolution.approval_count = 0;
    }
    if resolution.approval_count == 0 {
        resolution.approval_council_version = council.version;
        resolution.approval_council_hash = council.set_hash;
    }
    let (bitset, count) = record_seat_approval(
        &council,
        resolution.approval_bitset,
        resolution.approval_count,
        seat.key,
        slot,
    )?;
    resolution.approval_bitset = bitset;
    resolution.approval_count = count;
    if count == RELEASE1_APPROVAL_THRESHOLD {
        resolution.state = EmergencyFreezeResolutionStateV1::CouncilApproved;
    }
    validate_emergency_resolution_digest_v1(&resolution)?;
    store_fixed_controller_account(
        program_id,
        resolution_info,
        &*resolution,
        EmergencyFreezeResolutionV1::LEN,
    )
}

pub fn process_queue_emergency_resolution_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: QueueEmergencyResolutionV1,
) -> ProgramResult {
    let [config_info, policy_info, council_info, gate_info, resolution_info] = accounts else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_readonly_state_accounts(&[config_info, policy_info, council_info, gate_info])?;
    validate_exact_privileges(resolution_info, true, false, false)?;
    require_distinct_accounts(&[
        config_info,
        policy_info,
        council_info,
        gate_info,
        resolution_info,
    ])?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
    let council = load_current_council(program_id, council_info, config_info, &config, &policy)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let mut resolution =
        load_resolution(program_id, resolution_info, config_info, gate_info, &config)?;
    let slot = Clock::get()?.slot;
    require_policy_active(&policy, slot)?;
    check_emergency_expectation(
        &instruction.expected,
        &resolution,
        &config,
        &policy,
        &council,
        &gate,
    )?;
    require_emergency_binding(&resolution, &config, &gate, slot, false)?;
    if resolution.state != EmergencyFreezeResolutionStateV1::CouncilApproved
        || slot < resolution.not_before_slot
        || instruction.expected_approval_bitset != resolution.approval_bitset
        || instruction.expected_approval_count != resolution.approval_count
        || resolution.approval_council_version != council.version
        || resolution.approval_council_hash != council.set_hash
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_exact_recorded_quorum(
        &council,
        resolution.approval_bitset,
        resolution.approval_count,
        slot,
    )?;
    resolution.state = EmergencyFreezeResolutionStateV1::Timelocked;
    validate_emergency_resolution_digest_v1(&resolution)?;
    store_fixed_controller_account(
        program_id,
        resolution_info,
        &*resolution,
        EmergencyFreezeResolutionV1::LEN,
    )
}

pub fn process_execute_emergency_resolution_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExecuteEmergencyResolutionV1,
) -> ProgramResult {
    let [config_info, policy_info, council_info, gate_info, resolution_info, observation_info, checkpoint_info, target_program, target_programdata, loader, authority, instructions_sysvar] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_readonly_state_accounts(&[config_info, policy_info, council_info])?;
    validate_exact_privileges(gate_info, true, false, false)?;
    validate_exact_privileges(resolution_info, true, false, false)?;
    for account in [
        observation_info,
        checkpoint_info,
        target_programdata,
        authority,
        instructions_sysvar,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(target_program, false, false, true)?;
    validate_exact_privileges(loader, false, false, true)?;
    require_distinct_accounts(&[
        config_info,
        policy_info,
        council_info,
        gate_info,
        resolution_info,
        observation_info,
        checkpoint_info,
        target_program,
        target_programdata,
        loader,
        authority,
        instructions_sysvar,
    ])?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
    let council = load_current_council(program_id, council_info, config_info, &config, &policy)?;
    let mut gate = load_gate(program_id, gate_info, config_info, &config)?;
    let mut resolution =
        load_resolution(program_id, resolution_info, config_info, gate_info, &config)?;
    let observation = load_emergency_observation(
        program_id,
        observation_info,
        config_info,
        gate_info,
        &config,
        &gate,
    )?;
    let checkpoint = load_fixed_controller_account::<StateCheckpointV1>(
        program_id,
        checkpoint_info,
        StateCheckpointV1::LEN,
    )?;
    validate_state_checkpoint_digest_v1(&checkpoint)?;
    let checkpoint_pda =
        derive_emergency_checkpoint_pda(program_id, &config.target_program, gate.epoch);
    let slot = Clock::get()?.slot;
    require_policy_active(&policy, slot)?;
    check_emergency_expectation(
        &instruction.expected,
        &resolution,
        &config,
        &policy,
        &council,
        &gate,
    )?;
    require_emergency_binding(&resolution, &config, &gate, slot, false)?;
    if resolution.state != EmergencyFreezeResolutionStateV1::Timelocked
        || slot < resolution.not_before_slot
        || resolution.approval_council_version != council.version
        || resolution.approval_council_hash != council.set_hash
        || instruction.expected_freeze_observation_digest != observation.observation_digest
        || instruction.expected_checkpoint_digest != checkpoint.checkpoint_digest
        || *checkpoint_info.key != resolution.emergency_checkpoint
        || *checkpoint_info.key != checkpoint_pda.0
        || checkpoint.bump != checkpoint_pda.1
        || checkpoint.phase != StateCheckpointPhaseV1::Emergency
        || checkpoint.proposal != Pubkey::default()
        || checkpoint.emergency_resolution != *resolution_info.key
        || checkpoint.subject_digest != resolution.resolution_digest
        || checkpoint.controller_config != *config_info.key
        || checkpoint.target_program != config.target_program
        || checkpoint.target_programdata != config.target_programdata
        || checkpoint.gate_epoch != gate.epoch
        || !checkpoint.accepted
        || checkpoint.forbidden_drift_count != 0
        || checkpoint.approval_count != RELEASE1_APPROVAL_THRESHOLD
        || checkpoint.finalized_slot > slot
        || checkpoint.finalized_observation_slot < gate.freeze_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_exact_recorded_quorum(
        &council,
        resolution.approval_bitset,
        resolution.approval_count,
        slot,
    )?;
    let runtime = capture_runtime_observation(target_program, target_programdata)?;
    compare_execute_instruction_observation(&instruction, &runtime)?;
    require_resolution_observation_match(&resolution, &observation)?;
    require_canonical_unchanged_runtime(&runtime, &observation, &config)?;
    if checkpoint.target_programdata_slot != runtime.programdata_slot
        // The accepted emergency checkpoint is the council-attested payload
        // audit anchor.  Re-hashing the payload here would make this path take
        // a second full pass over ProgramData.  The mechanical raw-account
        // hash already covers the exact Loader header, payload, and zero tail,
        // so equality with the frozen observation proves that the checkpoint's
        // subject bytes have not changed since the guardian freeze.
        || checkpoint.target_payload_commitment == [0; 32]
        || checkpoint.target_raw_programdata_commitment != runtime.raw_programdata_hash
        || checkpoint.target_capacity != runtime.capacity
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    if *loader.key != config.upgradeable_loader
        || *authority.key != config.authority_pda
        || *target_program.key != config.target_program
        || *target_programdata.key != config.target_programdata
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_loader_identity(loader, &config)?;
    validate_bounded_emergency_resolution_envelope(
        program_id,
        accounts,
        instructions_sysvar,
        &instruction.pack(),
    )?;
    gate.epoch = gate
        .epoch
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    gate.status = GateStatusV1::Active;
    gate.active_proposal = Pubkey::default();
    gate.freeze_slot = 0;
    gate.freeze_reason_code = 0;
    resolution.state = EmergencyFreezeResolutionStateV1::Executed;
    resolution.executed_slot = slot;
    resolution.terminal_reason_code = EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V1;
    gate.validate_static()?;
    validate_emergency_resolution_digest_v1(&resolution)?;
    let _gate_bytes = encode_fixed_account(&*gate, ProtocolGateV1::LEN)?;
    let _resolution_bytes = encode_fixed_account(&*resolution, EmergencyFreezeResolutionV1::LEN)?;
    store_fixed_controller_account(program_id, gate_info, &*gate, ProtocolGateV1::LEN)?;
    store_fixed_controller_account(
        program_id,
        resolution_info,
        &*resolution,
        EmergencyFreezeResolutionV1::LEN,
    )
}

pub fn process_convert_emergency_freeze_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ConvertEmergencyFreezeV2,
) -> ProgramResult {
    let [config_info, policy_info, council_info, gate_info, proposal_info, observation_info, target_program, target_programdata, loader, authority, rollback_info, rollback_verification_info, rollback_buffer] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_exact_privileges(config_info, true, false, false)?;
    validate_readonly_state_accounts(&[policy_info, council_info])?;
    validate_exact_privileges(gate_info, true, false, false)?;
    validate_exact_privileges(proposal_info, true, false, false)?;
    for account in [
        observation_info,
        target_programdata,
        authority,
        rollback_info,
        rollback_verification_info,
        rollback_buffer,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(target_program, false, false, true)?;
    validate_exact_privileges(loader, false, false, true)?;
    require_distinct_accounts(&[
        config_info,
        policy_info,
        council_info,
        gate_info,
        proposal_info,
        observation_info,
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
            emergency_observation: Some(observation_info),
        },
        &instruction.expected,
        instruction.expected_next_gate_epoch,
        Some(&instruction.expected_freeze_observation_digest),
    )
}
