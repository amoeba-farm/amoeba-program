use super::*;

/// Creates an immutable, exact failure observation from live Program and
/// ProgramData bytes. The caller supplies only stale-plan guards and a Merkle
/// proof for the already-committed expected payload leaf.
pub fn process_observe_programdata_failure_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ObserveProgramDataFailureV1,
) -> ProgramResult {
    let [payer, config_info, gate_info, primary_info, verification_info, target_program, target_programdata, authority_info, loader_info, failure_info, system_program_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_exact_privileges(payer, true, true, false)?;
    for account in [
        config_info,
        gate_info,
        primary_info,
        verification_info,
        authority_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
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
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(failure_info, true, false, false)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&[
        payer,
        config_info,
        gate_info,
        primary_info,
        verification_info,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        failure_info,
        system_program_info,
    ])?;
    if *loader_info.key != UPGRADEABLE_LOADER_ID || *system_program_info.key != system_program::ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    instruction.validate_failure_shape()?;
    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let primary = load_proposal(program_id, primary_info, config_info, &config)?;
    check_proposal_expectation_without_live_council(
        &instruction.expected,
        &primary,
        &config,
        &gate,
    )?;
    validate_active_primary_binding(&primary, primary_info.key, &config, &gate)?;
    if primary.proposal_class == ProposalClassV1::EmergencyRollback
        || primary.state != ProposalStateV2::UpgradeExecuted
        || *target_program.key != config.target_program
        || *target_programdata.key != config.target_programdata
        || *authority_info.key != config.authority_pda
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let verification = load_programdata_verification_for_failure(
        program_id,
        verification_info,
        primary_info,
        config_info,
        &config,
        &primary,
    )?;
    if verification.status == ProgramDataVerificationStatusV1::Verified
        || verification.finalized_slot != 0
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let slot = Clock::get()?.slot;
    if slot == 0 || slot < primary.upgrade_executed_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let runtime = capture_runtime_observation(target_program, target_programdata)?;
    compare_failure_instruction_observation(&instruction, &runtime)?;
    let (expected_leaf_hash, actual_leaf_hash) = validate_mechanical_mismatch(
        &instruction,
        &runtime,
        target_programdata,
        &config,
        &primary,
        &verification,
    )?;

    let (expected_failure, failure_bump) =
        derive_programdata_failure_observation_pda(program_id, primary_info.key, gate.epoch);
    if *failure_info.key != expected_failure {
        return Err(GovernanceError::InvalidPda.into());
    }
    if failure_info.owner != &system_program::ID || failure_info.data_len() != 0 {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let mut failure = ProgramDataFailureObservationV1 {
        discriminator: PROGRAMDATA_FAILURE_OBSERVATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: failure_bump,
        initialized: true,
        finalized: true,
        controller_config: *config_info.key,
        protocol_gate: *gate_info.key,
        primary_proposal: *primary_info.key,
        target_program: *target_program.key,
        target_programdata: *target_programdata.key,
        frozen_epoch: gate.epoch,
        actual_program_owner: runtime.program_owner,
        actual_program_executable: runtime.program_executable,
        actual_program_data_length: runtime.program_data_length,
        program_header_present: runtime.program_header_present,
        actual_linked_programdata: runtime.linked_programdata,
        raw_hash_complete: runtime.raw_hash_complete,
        actual_raw_programdata_sha256: runtime.raw_programdata_hash,
        actual_owner: runtime.programdata_owner,
        actual_executable: runtime.programdata_executable,
        actual_data_length: runtime.programdata_data_length,
        programdata_header_present: runtime.programdata_header_present,
        actual_programdata_slot: runtime.programdata_slot,
        actual_capacity: runtime.capacity,
        actual_authority: runtime.authority,
        mismatch_class: instruction.mismatch_class,
        failing_chunk_index: instruction.failing_chunk_index,
        expected_leaf_hash,
        actual_leaf_hash,
        finalized_slot: slot,
        observation_digest: [0; 32],
        reserved: [0; PROGRAMDATA_FAILURE_OBSERVATION_V1_RESERVED_LEN],
    };
    failure.observation_digest = compute_programdata_failure_observation_digest_v1(&failure)?;
    validate_programdata_failure_observation_digest_v1(&failure)?;
    let failure_bytes = encode_fixed_account(&failure, ProgramDataFailureObservationV1::LEN)?;
    let epoch_seed = gate.epoch.to_le_bytes();
    let bump_seed = [failure_bump];
    let signer_seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        PROGRAMDATA_FAILURE_OBSERVATION_SEED,
        primary_info.key.as_ref(),
        &epoch_seed,
        &bump_seed,
    ];
    create_fixed_pda_account(
        program_id,
        payer,
        failure_info,
        system_program_info,
        &Rent::get()?,
        ProgramDataFailureObservationV1::LEN,
        signer_seeds,
    )?;
    commit_one_fixed_account(
        program_id,
        failure_info,
        &failure_bytes,
        ProgramDataFailureObservationV1::LEN,
    )
}

pub(super) fn load_programdata_verification_for_failure(
    program_id: &Pubkey,
    verification_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV2,
) -> Result<Box<ProgramDataVerificationV1>, ProgramError> {
    let verification = load_fixed_controller_account::<ProgramDataVerificationV1>(
        program_id,
        verification_info,
        ProgramDataVerificationV1::LEN,
    )?;
    verification.validate_schema()?;
    if derive_programdata_check_pda(program_id, proposal_info.key)
        != (*verification_info.key, verification.bump)
        || *verification_info.key != proposal.programdata_verification
        || verification.controller_config != *config_info.key
        || verification.proposal != *proposal_info.key
        || verification.target_program != config.target_program
        || verification.target_programdata != config.target_programdata
        || verification.upgradeable_loader != config.upgradeable_loader
        || verification.controller_authority != config.authority_pda
        || verification.artifact_length != proposal.artifact_length
        || verification.artifact_sha256 != proposal.artifact_sha256
        || verification.artifact_chunk_merkle_root != proposal.artifact_chunk_merkle_root
        || verification.chunk_hash_domain != proposal.chunk_hash_domain
        || verification.chunk_size != proposal.chunk_size
        || verification.payload_chunk_count != proposal.chunk_count
        || verification.deployed_slot != proposal.upgrade_executed_slot
        || verification.capacity != proposal.expected_post_capacity
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(verification)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_failure_observation(
    program_id: &Pubkey,
    failure_info: &AccountInfo<'_>,
    primary_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    primary: &UpgradeProposalV2,
) -> Result<Box<ProgramDataFailureObservationV1>, ProgramError> {
    let failure = load_fixed_controller_account::<ProgramDataFailureObservationV1>(
        program_id,
        failure_info,
        ProgramDataFailureObservationV1::LEN,
    )?;
    validate_programdata_failure_observation_digest_v1(&failure)?;
    if derive_programdata_failure_observation_pda(program_id, primary_info.key, gate.epoch)
        != (*failure_info.key, failure.bump)
        || failure.controller_config != *config_info.key
        || failure.protocol_gate != *gate_info.key
        || failure.primary_proposal != *primary_info.key
        || failure.target_program != config.target_program
        || failure.target_programdata != config.target_programdata
        || failure.frozen_epoch != gate.epoch
        || failure.frozen_epoch != primary.freeze_gate_epoch
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(failure)
}

pub(super) fn capture_runtime_observation(
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
) -> Result<RuntimeProgramDataObservationV1, ProgramError> {
    let program_data = target_program.try_borrow_data()?;
    let program_data_length =
        u64::try_from(program_data.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let parsed_program = parse_upgradeable_program(&program_data)
        .ok()
        .and_then(|header| OptionalPubkeyV1::some(header.programdata_address).ok());
    let (program_header_present, linked_programdata) = match parsed_program {
        Some(linked) => (true, linked),
        None => (false, OptionalPubkeyV1::none()),
    };
    drop(program_data);

    let programdata_data = target_programdata.try_borrow_data()?;
    let programdata_data_length =
        u64::try_from(programdata_data.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let raw_hash_complete = programdata_data_length <= MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1;
    let raw_programdata_hash = if raw_hash_complete {
        loader_account_data_hash(&programdata_data)
    } else {
        [0; 32]
    };
    let parsed_programdata = parse_upgradeable_programdata(&programdata_data).ok();
    let (programdata_header_present, programdata_slot, capacity, authority) =
        match parsed_programdata {
            Some(header) => (
                true,
                header.deployed_slot,
                u64::try_from(header.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?,
                optional_pubkey(header.upgrade_authority)?,
            ),
            None => (false, 0, 0, OptionalPubkeyV1::none()),
        };
    Ok(RuntimeProgramDataObservationV1 {
        program_owner: *target_program.owner,
        program_executable: target_program.executable,
        program_data_length,
        program_header_present,
        linked_programdata,
        programdata_owner: *target_programdata.owner,
        programdata_executable: target_programdata.executable,
        programdata_data_length,
        programdata_header_present,
        programdata_slot,
        raw_hash_complete,
        raw_programdata_hash,
        capacity,
        authority,
    })
}

fn optional_pubkey(value: Option<Pubkey>) -> GovernanceResult<OptionalPubkeyV1> {
    match value {
        Some(value) => OptionalPubkeyV1::some(value),
        None => Ok(OptionalPubkeyV1::none()),
    }
}

pub(super) fn linked_value(value: &OptionalPubkeyV1) -> Option<Pubkey> {
    value.present.then_some(value.value)
}

fn compare_failure_instruction_observation(
    instruction: &ObserveProgramDataFailureV1,
    actual: &RuntimeProgramDataObservationV1,
) -> ProgramResult {
    if instruction.expected_program_owner != actual.program_owner
        || instruction.expected_program_executable != actual.program_executable
        || instruction.expected_program_data_length != actual.program_data_length
        || instruction.expected_program_header_present != actual.program_header_present
        || instruction.expected_linked_programdata.value()
            != linked_value(&actual.linked_programdata)
        || instruction.expected_programdata_owner != actual.programdata_owner
        || instruction.expected_programdata_executable != actual.programdata_executable
        || instruction.expected_programdata_data_length != actual.programdata_data_length
        || instruction.expected_programdata_header_present != actual.programdata_header_present
        || instruction.expected_programdata_slot != actual.programdata_slot
        || instruction.expected_raw_hash_complete != actual.raw_hash_complete
        || instruction.expected_raw_programdata_hash != actual.raw_programdata_hash
        || instruction.expected_capacity != actual.capacity
        || instruction.expected_programdata_authority.value() != linked_value(&actual.authority)
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn require_observation_unchanged(
    actual: &RuntimeProgramDataObservationV1,
    observation: &ProgramDataFailureObservationV1,
) -> ProgramResult {
    if actual.program_owner != observation.actual_program_owner
        || actual.program_executable != observation.actual_program_executable
        || actual.program_data_length != observation.actual_program_data_length
        || actual.program_header_present != observation.program_header_present
        || actual.linked_programdata != observation.actual_linked_programdata
        || actual.programdata_owner != observation.actual_owner
        || actual.programdata_executable != observation.actual_executable
        || actual.programdata_data_length != observation.actual_data_length
        || actual.programdata_header_present != observation.programdata_header_present
        || actual.programdata_slot != observation.actual_programdata_slot
        || actual.raw_hash_complete != observation.raw_hash_complete
        || actual.raw_programdata_hash != observation.actual_raw_programdata_sha256
        || actual.capacity != observation.actual_capacity
        || actual.authority != observation.actual_authority
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

fn validate_mechanical_mismatch(
    instruction: &ObserveProgramDataFailureV1,
    actual: &RuntimeProgramDataObservationV1,
    target_programdata: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV2,
    verification: &ProgramDataVerificationV1,
) -> Result<([u8; 32], [u8; 32]), ProgramError> {
    let structural_match = actual.program_owner == config.upgradeable_loader
        && actual.program_executable
        && actual.program_data_length == LOADER_PROGRAM_ACCOUNT_LEN as u64
        && actual.program_header_present
        && linked_value(&actual.linked_programdata) == Some(config.target_programdata)
        && actual.programdata_owner == config.upgradeable_loader
        && !actual.programdata_executable
        && actual.programdata_header_present
        && actual.programdata_slot == verification.deployed_slot;
    let authority_match = linked_value(&actual.authority) == Some(config.authority_pda);
    let capacity_match = actual.capacity == verification.capacity;

    match instruction.mismatch_class {
        ProgramDataMismatchClassV1::Header => {
            if structural_match {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok(([0; 32], [0; 32]))
        }
        ProgramDataMismatchClassV1::Authority => {
            if !structural_match || authority_match {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok(([0; 32], [0; 32]))
        }
        ProgramDataMismatchClassV1::Capacity => {
            if !structural_match || !authority_match || capacity_match {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok(([0; 32], [0; 32]))
        }
        ProgramDataMismatchClassV1::PayloadLeaf => {
            if !structural_match || !authority_match || !capacity_match || !actual.raw_hash_complete
            {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            let data = target_programdata.try_borrow_data()?;
            let payload = data
                .get(LOADER_PROGRAMDATA_METADATA_LEN..)
                .ok_or(GovernanceError::InvalidRelease1Account)?;
            let exact = exact_region_chunk(
                payload,
                0,
                proposal.artifact_length,
                proposal.chunk_size,
                instruction.failing_chunk_index,
            )?;
            require_verified_prefix(
                &verification.verified_payload_chunk_bitmap,
                instruction.failing_chunk_index,
            )?;
            let actual_leaf = artifact_chunk_leaf_hash(instruction.failing_chunk_index, exact)?;
            validate_expected_leaf_proof(
                &proposal.artifact_chunk_merkle_root,
                proposal.artifact_length,
                proposal.chunk_size,
                instruction.failing_chunk_index,
                &instruction.expected_leaf_hash,
                &instruction.proof,
            )?;
            if actual_leaf == instruction.expected_leaf_hash {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok((instruction.expected_leaf_hash, actual_leaf))
        }
        ProgramDataMismatchClassV1::ZeroTail => {
            if !structural_match
                || !authority_match
                || !capacity_match
                || !actual.raw_hash_complete
                || !proposal.zero_tail_required
                || verification.verified_payload_chunk_count != verification.payload_chunk_count
            {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            let tail_length = verification
                .capacity
                .checked_sub(verification.artifact_length)
                .ok_or(GovernanceError::InvalidCapacityPlan)?;
            let data = target_programdata.try_borrow_data()?;
            let payload = data
                .get(LOADER_PROGRAMDATA_METADATA_LEN..)
                .ok_or(GovernanceError::InvalidRelease1Account)?;
            let exact = exact_region_chunk(
                payload,
                verification.artifact_length,
                tail_length,
                verification.chunk_size,
                instruction.failing_chunk_index,
            )?;
            require_verified_prefix(
                &verification.verified_tail_chunk_bitmap,
                instruction.failing_chunk_index,
            )?;
            let expected_leaf =
                programdata_zero_tail_zero_hash(instruction.failing_chunk_index, exact.len())?;
            let actual_leaf =
                programdata_zero_tail_chunk_hash(instruction.failing_chunk_index, exact)?;
            if instruction.expected_leaf_hash != expected_leaf
                || actual_leaf == expected_leaf
                || exact.iter().all(|byte| *byte == 0)
            {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok((expected_leaf, actual_leaf))
        }
    }
}
