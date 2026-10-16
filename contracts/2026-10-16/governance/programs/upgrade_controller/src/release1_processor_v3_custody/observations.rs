use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn load_observation_any_status(
    program_id: &Pubkey,
    observation_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    subject_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    capacity: &ProgramDataCapacityPolicyV1,
) -> Result<Box<ProgramDataObservationV1>, ProgramError> {
    let observation = load_fixed_controller_account::<ProgramDataObservationV1>(
        program_id,
        observation_info,
        ProgramDataObservationV1::LEN,
    )?;
    observation.validate_static()?;
    if derive_programdata_observation_pda(
        program_id,
        &config.target_program,
        observation.purpose as u8,
        &observation.subject_digest,
        observation.generation,
    ) != (*observation_info.key, observation.bump)
        || observation.controller_program != *program_id
        || observation.controller_config != *config_info.key
        || observation.capacity_policy != *capacity_info.key
        || observation.capacity_policy_digest != capacity.policy_digest
        || observation.protocol_gate != config.gate_pda
        || observation.subject != *subject_info.key
        || observation.target_program != config.target_program
        || observation.target_programdata != config.target_programdata
        || observation.upgradeable_loader != config.upgradeable_loader
        || observation.expected_artifact_scheme_id != capacity.artifact_scheme_id
        || observation.artifact_chunk_size != capacity.artifact_chunk_size
        || observation.raw_observation_scheme_id != capacity.observation_scheme_id
        || observation.raw_chunk_size != capacity.observation_chunk_size
    {
        return Err(GovernanceError::InvalidProgramDataObservation.into());
    }
    let expected_subject = compute_programdata_observation_subject_digest_v1(
        program_id,
        config_info.key,
        &config.target_program,
        &config.target_programdata,
        observation.purpose,
        subject_info.key,
        observation.generation,
        &observation.protocol_gate,
        observation.gate_status,
        observation.gate_epoch,
        &observation.gate_active_proposal,
        observation.gate_freeze_slot,
        observation.gate_freeze_reason_code,
        &capacity.policy_digest,
        observation.expected_artifact_length,
        &observation.expected_artifact_sha256,
        &observation.expected_artifact_merkle_root,
        &observation.expected_artifact_scheme_id,
        observation.minimum_required_capacity,
    )?;
    if expected_subject != observation.subject_digest {
        return Err(GovernanceError::InvalidProgramDataObservation.into());
    }
    if observation.status == ProgramDataObservationStatusV1::Finalized {
        validate_programdata_observation_digest_v1(&observation)?;
    }
    Ok(observation)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_fresh_observation(
    program_id: &Pubkey,
    observation_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    subject_info: &AccountInfo<'_>,
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    loader_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    capacity: &ProgramDataCapacityPolicyV1,
    gate: &ProtocolGateV1,
    purpose: ProgramDataObservationPurposeV1,
    artifact_length: u64,
    artifact_sha256: &[u8; 32],
    artifact_root: &[u8; 32],
    minimum_required_capacity: u64,
) -> Result<Box<ProgramDataObservationV1>, ProgramError> {
    let observation = load_observation_any_status(
        program_id,
        observation_info,
        config_info,
        capacity_info,
        subject_info,
        config,
        capacity,
    )?;
    if observation.status != ProgramDataObservationStatusV1::Finalized
        || observation.purpose != purpose
        || observation.expected_artifact_length != artifact_length
        || observation.expected_artifact_sha256 != *artifact_sha256
        || observation.expected_artifact_merkle_root != *artifact_root
        || observation.minimum_required_capacity != minimum_required_capacity
        || observation.protocol_gate != config.gate_pda
        || observation.gate_status != gate.status
        || observation.gate_epoch != gate.epoch
        || observation.gate_active_proposal != gate.active_proposal
        || observation.gate_freeze_slot != gate.freeze_slot
        || observation.gate_freeze_reason_code != gate.freeze_reason_code
        || *target_program.key != config.target_program
        || *target_programdata.key != config.target_programdata
        || *loader_info.key != config.upgradeable_loader
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    let header = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &config.upgradeable_loader,
    )?;
    let program_data = target_program.try_borrow_data()?;
    let mut program_snapshot = [0u8; LOADER_PROGRAM_ACCOUNT_LEN];
    if program_data.len() != LOADER_PROGRAM_ACCOUNT_LEN {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    program_snapshot.copy_from_slice(&program_data);
    drop(program_data);
    let programdata_data = target_programdata.try_borrow_data()?;
    let raw_len =
        u64::try_from(programdata_data.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let mut programdata_snapshot = [0u8; LOADER_PROGRAMDATA_METADATA_LEN];
    programdata_snapshot.copy_from_slice(
        programdata_data
            .get(..LOADER_PROGRAMDATA_METADATA_LEN)
            .ok_or(GovernanceError::StaleProgramDataObservation)?,
    );
    drop(programdata_data);
    if observation.program_owner != *target_program.owner
        || observation.program_executable != target_program.executable
        || observation.program_data_length != LOADER_PROGRAM_ACCOUNT_LEN as u64
        || observation.program_header_snapshot != program_snapshot
        || observation.linked_programdata != config.target_programdata
        || observation.programdata_owner != *target_programdata.owner
        || observation.programdata_executable != target_programdata.executable
        || observation.programdata_header_snapshot != programdata_snapshot
        || observation.deployed_slot != header.deployed_slot
        || !observation.upgrade_authority.present
        || observation.upgrade_authority.value != config.authority_pda
        || observation.raw_data_length != raw_len
        || observation.actual_capacity
            != u64::try_from(header.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
        || raw_len
            != observation
                .actual_capacity
                .checked_add(LOADER_PROGRAMDATA_METADATA_LEN as u64)
                .ok_or(GovernanceError::ArithmeticOverflow)?
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(observation)
}

pub(super) fn require_observation_guard(
    observation: &ProgramDataObservationV1,
    expected_digest: &[u8; 32],
    expected_generation: u64,
    expected_root: &[u8; 32],
    expected_finalized_slot: u64,
    expected_capacity: u64,
) -> ProgramResult {
    if observation.observation_digest != *expected_digest
        || observation.generation != expected_generation
        || observation.final_raw_merkle_root != *expected_root
        || observation.finalized_slot != expected_finalized_slot
        || observation.actual_capacity != expected_capacity
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(())
}

fn state_optional_pubkey(value: Option<Pubkey>) -> Result<OptionalPubkeyV1, ProgramError> {
    value.map_or_else(
        || Ok(OptionalPubkeyV1::none()),
        |key| OptionalPubkeyV1::some(key).map_err(ProgramError::from),
    )
}

pub(super) fn capture_runtime_graph(
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
) -> Result<RuntimeProgramDataGraphV2, ProgramError> {
    let program_data = target_program.try_borrow_data()?;
    let parsed_program = parse_upgradeable_program(&program_data).ok();
    let program_data_length =
        u64::try_from(program_data.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    drop(program_data);
    let programdata_data = target_programdata.try_borrow_data()?;
    let parsed_programdata = parse_upgradeable_programdata(&programdata_data).ok();
    let programdata_data_length =
        u64::try_from(programdata_data.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    drop(programdata_data);
    Ok(RuntimeProgramDataGraphV2 {
        program_owner: *target_program.owner,
        program_executable: target_program.executable,
        program_data_length,
        program_header_present: parsed_program.is_some(),
        linked_programdata: state_optional_pubkey(
            parsed_program.map(|header| header.programdata_address),
        )?,
        programdata_owner: *target_programdata.owner,
        programdata_executable: target_programdata.executable,
        programdata_data_length,
        programdata_header_present: parsed_programdata.is_some(),
        deployed_slot: parsed_programdata
            .as_ref()
            .map_or(0, |header| header.deployed_slot),
        actual_capacity: parsed_programdata.as_ref().map_or(0, |header| {
            u64::try_from(header.capacity).unwrap_or(u64::MAX)
        }),
        authority: state_optional_pubkey(
            parsed_programdata.and_then(|header| header.upgrade_authority),
        )?,
    })
}

pub(super) fn runtime_matches_observation(
    runtime: &RuntimeProgramDataGraphV2,
    observation: &ProgramDataObservationV1,
) -> bool {
    runtime.program_owner == observation.program_owner
        && runtime.program_executable == observation.program_executable
        && runtime.program_data_length == observation.program_data_length
        && runtime.program_header_present == observation.program_header_present
        && runtime.linked_programdata.present
        && runtime.linked_programdata.value == observation.linked_programdata
        && runtime.programdata_owner == observation.programdata_owner
        && runtime.programdata_executable == observation.programdata_executable
        && runtime.programdata_header_present == observation.programdata_header_present
        && runtime.deployed_slot == observation.deployed_slot
        && runtime.actual_capacity == observation.actual_capacity
        && runtime.authority == observation.upgrade_authority
        && runtime.programdata_data_length == observation.raw_data_length
}

pub(super) fn runtime_matches_failure_observation(
    runtime: &RuntimeProgramDataGraphV2,
    failure: &ProgramDataFailureObservationV2,
) -> bool {
    runtime.program_owner == failure.actual_program_owner
        && runtime.program_executable == failure.actual_program_executable
        && runtime.program_data_length == failure.actual_program_data_length
        && runtime.program_header_present == failure.program_header_present
        && runtime.linked_programdata == failure.actual_linked_programdata
        && runtime.programdata_owner == failure.actual_programdata_owner
        && runtime.programdata_executable == failure.actual_programdata_executable
        && runtime.programdata_data_length == failure.actual_programdata_data_length
        && runtime.programdata_header_present == failure.programdata_header_present
        && runtime.deployed_slot == failure.actual_programdata_slot
        && runtime.actual_capacity == failure.actual_capacity
        && runtime.authority == failure.actual_authority
}
