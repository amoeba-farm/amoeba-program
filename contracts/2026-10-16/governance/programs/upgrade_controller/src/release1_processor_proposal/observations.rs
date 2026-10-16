use super::*;

pub(super) fn read_canonical_programdata_snapshot(
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    expected_authority: Option<Pubkey>,
) -> Result<ProgramDataSnapshotV1, ProgramError> {
    let header = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &config.upgradeable_loader,
    )?;
    let capacity =
        u64::try_from(header.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    if header.upgrade_authority != expected_authority {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let data = target_programdata.try_borrow_data()?;
    if u64::try_from(data.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?
        > MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
    {
        return Err(GovernanceError::InvalidCapacityPlan.into());
    }
    Ok(ProgramDataSnapshotV1 {
        deployed_slot: header.deployed_slot,
        capacity,
        raw_hash: loader_account_data_hash(&data),
    })
}

pub(super) fn optional_pubkey(value: Option<Pubkey>) -> GovernanceResult<OptionalPubkeyV1> {
    match value {
        Some(value) => OptionalPubkeyV1::some(value),
        None => Ok(OptionalPubkeyV1::none()),
    }
}

pub(super) fn capture_runtime_observation(
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
) -> Result<RuntimeObservationV1, ProgramError> {
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
    Ok(RuntimeObservationV1 {
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

pub(super) fn compare_guardian_instruction_observation(
    instruction: &GuardianFreezeV1,
    actual: &RuntimeObservationV1,
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

pub(super) fn compare_resolution_instruction_observation(
    instruction: &CreateEmergencyResolutionV1,
    actual: &EmergencyFreezeObservationV1,
) -> ProgramResult {
    if instruction.observed_program_owner != actual.actual_program_owner
        || instruction.observed_program_executable != actual.actual_program_executable
        || instruction.observed_program_data_length != actual.actual_program_data_length
        || instruction.observed_program_header_present != actual.program_header_present
        || instruction.observed_linked_programdata.value()
            != linked_value(&actual.actual_linked_programdata)
        || instruction.observed_programdata_owner != actual.actual_programdata_owner
        || instruction.observed_programdata_executable != actual.actual_programdata_executable
        || instruction.observed_programdata_data_length != actual.actual_programdata_data_length
        || instruction.observed_programdata_header_present != actual.programdata_header_present
        || instruction.observed_programdata_slot != actual.deployed_programdata_slot
        || instruction.observed_raw_hash_complete != actual.raw_hash_complete
        || instruction.observed_raw_programdata_hash != actual.raw_programdata_sha256
        || instruction.observed_capacity != actual.capacity
        || instruction.observed_programdata_authority.value()
            != linked_value(&actual.observed_authority)
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn compare_execute_instruction_observation(
    instruction: &ExecuteEmergencyResolutionV1,
    actual: &RuntimeObservationV1,
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

fn linked_value(optional: &OptionalPubkeyV1) -> Option<Pubkey> {
    optional.present.then_some(optional.value)
}

pub(super) fn require_resolution_observation_match(
    resolution: &EmergencyFreezeResolutionV1,
    observation: &EmergencyFreezeObservationV1,
) -> ProgramResult {
    if resolution.emergency_freeze_observation == Pubkey::default()
        || resolution.frozen_epoch != observation.frozen_epoch
        || resolution.freeze_slot != observation.freeze_slot
        || resolution.freeze_reason_code != observation.freeze_reason_code
        || resolution.observed_program_owner != observation.actual_program_owner
        || resolution.observed_program_executable != observation.actual_program_executable
        || resolution.observed_program_data_length != observation.actual_program_data_length
        || resolution.observed_program_header_present != observation.program_header_present
        || resolution.observed_linked_programdata != observation.actual_linked_programdata
        || resolution.observed_programdata_owner != observation.actual_programdata_owner
        || resolution.observed_programdata_executable != observation.actual_programdata_executable
        || resolution.observed_programdata_data_length != observation.actual_programdata_data_length
        || resolution.observed_programdata_header_present != observation.programdata_header_present
        || resolution.observed_programdata_slot != observation.deployed_programdata_slot
        || resolution.observed_raw_hash_complete != observation.raw_hash_complete
        || resolution.observed_raw_programdata_hash != observation.raw_programdata_sha256
        || resolution.observed_capacity != observation.capacity
        || resolution.observed_authority != observation.observed_authority
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn require_canonical_unchanged_runtime(
    actual: &RuntimeObservationV1,
    frozen: &EmergencyFreezeObservationV1,
    config: &ControllerConfigV1,
) -> ProgramResult {
    if !actual.raw_hash_complete
        || actual.program_owner != UPGRADEABLE_LOADER_ID
        || !actual.program_executable
        || !actual.program_header_present
        || linked_value(&actual.linked_programdata) != Some(config.target_programdata)
        || actual.programdata_owner != UPGRADEABLE_LOADER_ID
        || actual.programdata_executable
        || !actual.programdata_header_present
        || actual.programdata_slot == 0
        || actual.capacity == 0
        || linked_value(&actual.authority) != Some(config.authority_pda)
        || actual.program_owner != frozen.actual_program_owner
        || actual.program_executable != frozen.actual_program_executable
        || actual.program_data_length != frozen.actual_program_data_length
        || actual.program_header_present != frozen.program_header_present
        || linked_value(&actual.linked_programdata)
            != linked_value(&frozen.actual_linked_programdata)
        || actual.programdata_owner != frozen.actual_programdata_owner
        || actual.programdata_executable != frozen.actual_programdata_executable
        || actual.programdata_data_length != frozen.actual_programdata_data_length
        || actual.programdata_header_present != frozen.programdata_header_present
        || actual.programdata_slot != frozen.deployed_programdata_slot
        || actual.raw_hash_complete != frozen.raw_hash_complete
        || actual.raw_programdata_hash != frozen.raw_programdata_sha256
        || actual.capacity != frozen.capacity
        || linked_value(&actual.authority) != linked_value(&frozen.observed_authority)
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}
