//! Executable bounded observation of canonical Loader-v3 ProgramData bytes.
//!
//! No instruction in this module accepts a caller-supplied byte chunk or leaf
//! hash. Every raw leaf, artifact leaf, header snapshot, and zero-tail check is
//! derived from the read-only ProgramData account supplied under the canonical
//! Program/ProgramData/Loader graph.

use solana_program::{
    account_info::AccountInfo, clock::Clock, entrypoint::ProgramResult,
    program_error::ProgramError, pubkey::Pubkey, rent::Rent, sysvar::Sysvar,
};
use solana_sdk_ids::system_program;

use crate::{
    artifact_merkle::{artifact_chunk_count, verify_artifact_chunk_proof},
    pda::{
        derive_authority_pda, derive_capacity_policy_pda, derive_controller_config_pda,
        derive_gate_pda, derive_programdata_observation_pda,
        derive_upgradeable_programdata_address, PROGRAMDATA_OBSERVATION_SEED,
        UPGRADEABLE_LOADER_ID, UPGRADE_SEED_DOMAIN_V1,
    },
    programdata_observation_merkle::{
        append_programdata_observation_chunk, finalize_programdata_observation_frontier,
        programdata_observation_chunk_count, ProgramDataObservationMerkleFrontierV1,
        PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
    },
    release1_account_io::{
        create_fixed_pda_account, encode_fixed_account, load_fixed_controller_account,
        require_distinct_accounts, store_fixed_controller_account, validate_exact_privileges,
    },
    release1_ceremony_digest::{
        compute_programdata_observation_digest_v1,
        compute_programdata_observation_subject_digest_v1, validate_capacity_policy_digest_v1,
        validate_programdata_observation_digest_v1,
    },
    release1_ceremony_instruction::{
        AppendProgramDataObservationChunkV1, BeginProgramDataObservationV1,
        FinalizeProgramDataObservationV1, ObservationAuthorityV1, ProgramDataObservationGuardV1,
        VerifyObservedArtifactChunkV1, BEGIN_PROGRAMDATA_OBSERVATION_V1_ACCOUNT_COUNT,
        PROGRAMDATA_OBSERVATION_STEP_V1_ACCOUNT_COUNT,
    },
    release1_ceremony_state::{
        ProgramDataCapacityPolicyV1, ProgramDataObservationPurposeV1,
        ProgramDataObservationStatusV1, ProgramDataObservationV1, ARTIFACT_BINDING_CHUNK_SIZE_V1,
        CEREMONY_ACCOUNT_VERSION_V1, PROGRAMDATA_OBSERVATION_V1_DISCRIMINATOR,
        PROGRAMDATA_OBSERVATION_V1_RESERVED_LEN, PROGRAMDATA_PAYLOAD_OFFSET_V1,
    },
    release1_loader_accounts::{
        parse_upgradeable_program, parse_upgradeable_programdata,
        validate_program_programdata_linkage, LOADER_PROGRAMDATA_METADATA_LEN,
        LOADER_PROGRAM_ACCOUNT_LEN,
    },
    release1_state::BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
    state::{ControllerConfigV1, GateStatusV1, OptionalPubkeyV1, ProtocolGateV1},
    GovernanceError,
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct ObservedProgramDataGraphV1 {
    program_owner: Pubkey,
    program_executable: bool,
    program_data_length: u64,
    program_header_snapshot: [u8; LOADER_PROGRAM_ACCOUNT_LEN],
    linked_programdata: Pubkey,
    programdata_owner: Pubkey,
    programdata_executable: bool,
    programdata_header_snapshot: [u8; LOADER_PROGRAMDATA_METADATA_LEN],
    deployed_slot: u64,
    upgrade_authority: OptionalPubkeyV1,
    raw_data_length: u64,
    actual_capacity: u64,
}

/// Create one immutable observation generation. Existing accounts, including a
/// stale or finalized older generation, cannot be reused.
pub fn process_begin_programdata_observation_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: BeginProgramDataObservationV1,
) -> ProgramResult {
    if accounts.len() != BEGIN_PROGRAMDATA_OBSERVATION_V1_ACCOUNT_COUNT {
        return Err(GovernanceError::InvalidAccountCount.into());
    }
    let [payer, config_info, gate_info, capacity_policy_info, subject_info, observed_program_info, observed_programdata_info, observation_info, loader_info, system_program_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };

    validate_exact_privileges(payer, true, true, false)?;
    validate_exact_privileges(config_info, false, false, false)?;
    validate_exact_privileges(gate_info, false, false, false)?;
    validate_exact_privileges(capacity_policy_info, false, false, false)?;
    validate_exact_privileges(subject_info, false, false, false)?;
    validate_exact_privileges(observed_program_info, false, false, true)?;
    validate_exact_privileges(observed_programdata_info, false, false, false)?;
    validate_exact_privileges(observation_info, true, false, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&[
        payer,
        config_info,
        gate_info,
        capacity_policy_info,
        subject_info,
        observed_program_info,
        observed_programdata_info,
        observation_info,
        loader_info,
        system_program_info,
    ])?;
    if *loader_info.key != UPGRADEABLE_LOADER_ID || *system_program_info.key != system_program::ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_subject_account(program_id, subject_info)?;

    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    validate_guard(&instruction.guard, &gate)?;
    validate_observation_purpose_gate(instruction.guard.purpose, subject_info.key, &gate)?;
    let capacity_policy =
        load_capacity_policy(program_id, capacity_policy_info, config_info, &config)?;
    if instruction.expected_capacity_policy_digest != capacity_policy.policy_digest {
        return Err(GovernanceError::CapacityPolicyMismatch.into());
    }
    validate_observed_identity(
        program_id,
        instruction.guard.purpose,
        &config,
        observed_program_info.key,
        observed_programdata_info.key,
    )?;

    let graph = read_observed_graph(
        observed_program_info,
        observed_programdata_info,
        loader_info,
        &capacity_policy,
    )?;
    // A multi-transaction observation may begin only after the deployment
    // slot is in the past. Any later Loader upgrade must then change the
    // ProgramData slot, including an upgrade in the same slot as the begin
    // instruction, and every subsequent step will fail the graph snapshot.
    let slot = current_slot()?;
    require_observation_begins_after_deployment(graph.deployed_slot, slot)?;
    let expected_authority =
        instruction_authority_to_state(&instruction.expected_upgrade_authority)?;
    if graph.deployed_slot != instruction.expected_deployed_slot
        || graph.actual_capacity != instruction.expected_actual_capacity
        || graph.upgrade_authority != expected_authority
        || instruction.expected_artifact_scheme_id != capacity_policy.artifact_scheme_id
        || instruction.expected_artifact_length > graph.actual_capacity
        || instruction.minimum_required_capacity > graph.actual_capacity
    {
        return Err(GovernanceError::InvalidProgramDataObservation.into());
    }

    let subject_digest = compute_programdata_observation_subject_digest_v1(
        program_id,
        config_info.key,
        observed_program_info.key,
        observed_programdata_info.key,
        instruction.guard.purpose,
        subject_info.key,
        instruction.guard.generation,
        gate_info.key,
        gate.status,
        gate.epoch,
        &gate.active_proposal,
        gate.freeze_slot,
        gate.freeze_reason_code,
        &capacity_policy.policy_digest,
        instruction.expected_artifact_length,
        &instruction.expected_artifact_sha256,
        &instruction.expected_artifact_merkle_root,
        &instruction.expected_artifact_scheme_id,
        instruction.minimum_required_capacity,
    )?;
    if subject_digest != instruction.guard.expected_subject_digest {
        return Err(GovernanceError::InvalidProgramDataObservation.into());
    }

    let (expected_observation, observation_bump) = derive_programdata_observation_pda(
        program_id,
        observed_program_info.key,
        instruction.guard.purpose as u8,
        &subject_digest,
        instruction.guard.generation,
    );
    if expected_observation != *observation_info.key {
        return Err(GovernanceError::InvalidPda.into());
    }
    if observation_info.owner != &system_program::ID || observation_info.data_len() != 0 {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }

    let raw_chunk_count = programdata_observation_chunk_count(
        graph.raw_data_length,
        capacity_policy.observation_chunk_size,
    )?;
    let raw_padded_leaf_count = raw_chunk_count
        .checked_next_power_of_two()
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let raw_tree_depth = u8::try_from(raw_padded_leaf_count.trailing_zeros())
        .map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let artifact_chunk_count = artifact_chunk_count(
        instruction.expected_artifact_length,
        ARTIFACT_BINDING_CHUNK_SIZE_V1,
    )?;
    let observation = ProgramDataObservationV1 {
        discriminator: PROGRAMDATA_OBSERVATION_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: observation_bump,
        initialized: true,
        controller_program: *program_id,
        controller_config: *config_info.key,
        capacity_policy: *capacity_policy_info.key,
        capacity_policy_digest: capacity_policy.policy_digest,
        purpose: instruction.guard.purpose,
        subject: *subject_info.key,
        subject_digest,
        generation: instruction.guard.generation,
        protocol_gate: *gate_info.key,
        gate_status: gate.status,
        gate_epoch: gate.epoch,
        gate_active_proposal: gate.active_proposal,
        gate_freeze_slot: gate.freeze_slot,
        gate_freeze_reason_code: gate.freeze_reason_code,
        target_program: *observed_program_info.key,
        target_programdata: *observed_programdata_info.key,
        upgradeable_loader: *loader_info.key,
        program_owner: graph.program_owner,
        program_executable: graph.program_executable,
        program_data_length: graph.program_data_length,
        program_header_present: true,
        program_header_snapshot: graph.program_header_snapshot,
        linked_programdata: graph.linked_programdata,
        programdata_owner: graph.programdata_owner,
        programdata_executable: graph.programdata_executable,
        programdata_header_present: true,
        programdata_header_snapshot: graph.programdata_header_snapshot,
        deployed_slot: graph.deployed_slot,
        upgrade_authority: graph.upgrade_authority,
        raw_data_length: graph.raw_data_length,
        payload_offset: PROGRAMDATA_PAYLOAD_OFFSET_V1,
        actual_capacity: graph.actual_capacity,
        expected_artifact_length: instruction.expected_artifact_length,
        expected_artifact_sha256: instruction.expected_artifact_sha256,
        expected_artifact_merkle_root: instruction.expected_artifact_merkle_root,
        expected_artifact_scheme_id: instruction.expected_artifact_scheme_id,
        artifact_chunk_size: ARTIFACT_BINDING_CHUNK_SIZE_V1,
        artifact_chunk_count,
        minimum_required_capacity: instruction.minimum_required_capacity,
        raw_observation_scheme_id: PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
        raw_chunk_size: capacity_policy.observation_chunk_size,
        raw_chunk_count,
        raw_padded_leaf_count,
        raw_tree_depth,
        next_raw_chunk_index: 0,
        raw_frontier: [[0; 32]; 11],
        raw_frontier_mask: 0,
        next_artifact_chunk_index: 0,
        tail_bytes_verified: 0,
        start_slot: slot,
        last_observed_slot: slot,
        finalized_slot: 0,
        final_raw_merkle_root: [0; 32],
        observation_digest: [0; 32],
        status: ProgramDataObservationStatusV1::Accumulating,
        reserved: [0; PROGRAMDATA_OBSERVATION_V1_RESERVED_LEN],
    };
    observation.validate_static()?;
    let encoded = encode_fixed_account(&observation, ProgramDataObservationV1::LEN)?;

    let purpose_seed = [instruction.guard.purpose as u8];
    let generation_seed = instruction.guard.generation.to_le_bytes();
    let bump_seed = [observation_bump];
    let signer_seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        PROGRAMDATA_OBSERVATION_SEED,
        observed_program_info.key.as_ref(),
        &purpose_seed,
        &subject_digest,
        &generation_seed,
        &bump_seed,
    ];
    create_fixed_pda_account(
        program_id,
        payer,
        observation_info,
        system_program_info,
        &Rent::get()?,
        ProgramDataObservationV1::LEN,
        signer_seeds,
    )?;
    let mut data = observation_info.try_borrow_mut_data()?;
    data.copy_from_slice(&encoded);
    Ok(())
}

/// Append exactly the next raw ProgramData range. Its intersection with the
/// payload tail is scanned in the same transaction, so raw completion also
/// proves every post-artifact byte is zero.
pub fn process_append_programdata_observation_chunk_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: AppendProgramDataObservationChunkV1,
) -> ProgramResult {
    let ObservationStepContext {
        observed_programdata_info,
        observation_info,
        mut observation,
    } = load_step_context(program_id, accounts, &instruction.guard)?;
    if instruction.expected_status != observation.status
        || instruction.chunk_index != observation.next_raw_chunk_index
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    let slot = current_slot()?;
    let data = observed_programdata_info.try_borrow_data()?;
    let chunk = exact_raw_chunk(&observation, instruction.chunk_index, &data)?;
    append_raw_record_in_place(&mut observation, instruction.chunk_index, chunk, slot)?;
    drop(data);
    store_fixed_controller_account(
        program_id,
        observation_info,
        &*observation,
        ProgramDataObservationV1::LEN,
    )
}

/// Verify exactly the next artifact payload chunk against the immutable
/// release root. The chunk is read directly from ProgramData.
pub fn process_verify_observed_artifact_chunk_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: VerifyObservedArtifactChunkV1,
) -> ProgramResult {
    let ObservationStepContext {
        observed_programdata_info,
        observation_info,
        mut observation,
    } = load_step_context(program_id, accounts, &instruction.guard)?;
    if instruction.expected_status != observation.status
        || instruction.chunk_index != observation.next_artifact_chunk_index
        || instruction.expected_next_artifact_chunk_index != observation.next_artifact_chunk_index
        || instruction.expected_tail_bytes_verified != observation.tail_bytes_verified
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    let slot = current_slot()?;
    let data = observed_programdata_info.try_borrow_data()?;
    let chunk = exact_artifact_chunk(&observation, instruction.chunk_index, &data)?;
    let proof = &instruction.proof.nodes[..usize::from(instruction.proof.proof_len)];
    verify_artifact_record_in_place(
        &mut observation,
        instruction.chunk_index,
        chunk,
        proof,
        slot,
    )?;
    drop(data);
    store_fixed_controller_account(
        program_id,
        observation_info,
        &*observation,
        ProgramDataObservationV1::LEN,
    )
}

/// Finalize a complete generation into its canonical raw root and immutable
/// domain-separated observation digest.
pub fn process_finalize_programdata_observation_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: FinalizeProgramDataObservationV1,
) -> ProgramResult {
    let ObservationStepContext {
        observation_info,
        mut observation,
        ..
    } = load_step_context(program_id, accounts, &instruction.guard)?;
    if instruction.expected_status != observation.status
        || instruction.expected_next_raw_chunk_index != observation.next_raw_chunk_index
        || instruction.expected_next_artifact_chunk_index != observation.next_artifact_chunk_index
        || instruction.expected_tail_bytes_verified != observation.tail_bytes_verified
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    finalize_observation_record_in_place(&mut observation, current_slot()?)?;
    store_fixed_controller_account(
        program_id,
        observation_info,
        &*observation,
        ProgramDataObservationV1::LEN,
    )
}

struct ObservationStepContext<'a, 'info> {
    observed_programdata_info: &'a AccountInfo<'info>,
    observation_info: &'a AccountInfo<'info>,
    observation: Box<ProgramDataObservationV1>,
}

fn load_step_context<'a, 'info>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'info>],
    guard: &ProgramDataObservationGuardV1,
) -> Result<ObservationStepContext<'a, 'info>, ProgramError> {
    if accounts.len() != PROGRAMDATA_OBSERVATION_STEP_V1_ACCOUNT_COUNT {
        return Err(GovernanceError::InvalidAccountCount.into());
    }
    let [config_info, gate_info, capacity_policy_info, subject_info, observed_program_info, observed_programdata_info, observation_info, loader_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_exact_privileges(config_info, false, false, false)?;
    validate_exact_privileges(gate_info, false, false, false)?;
    validate_exact_privileges(capacity_policy_info, false, false, false)?;
    validate_exact_privileges(subject_info, false, false, false)?;
    validate_exact_privileges(observed_program_info, false, false, true)?;
    validate_exact_privileges(observed_programdata_info, false, false, false)?;
    validate_exact_privileges(observation_info, true, false, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    require_distinct_accounts(&[
        config_info,
        gate_info,
        capacity_policy_info,
        subject_info,
        observed_program_info,
        observed_programdata_info,
        observation_info,
        loader_info,
    ])?;
    if *loader_info.key != UPGRADEABLE_LOADER_ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_subject_account(program_id, subject_info)?;

    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    validate_guard(guard, &gate)?;
    let capacity_policy =
        load_capacity_policy(program_id, capacity_policy_info, config_info, &config)?;
    validate_observed_identity(
        program_id,
        guard.purpose,
        &config,
        observed_program_info.key,
        observed_programdata_info.key,
    )?;
    let observation = load_observation(
        program_id,
        observation_info,
        config_info,
        capacity_policy_info,
        subject_info,
        observed_program_info,
        observed_programdata_info,
        loader_info,
        guard,
        &capacity_policy,
    )?;
    validate_observation_gate_snapshot(&observation, gate_info.key, &gate)?;
    let current_graph = read_observed_graph(
        observed_program_info,
        observed_programdata_info,
        loader_info,
        &capacity_policy,
    )
    .map_err(|_| ProgramError::from(GovernanceError::StaleProgramDataObservation))?;
    validate_graph_fresh(&observation, &current_graph)?;

    Ok(ObservationStepContext {
        observed_programdata_info,
        observation_info,
        observation,
    })
}

fn load_config(
    program_id: &Pubkey,
    config_info: &AccountInfo<'_>,
) -> Result<Box<ControllerConfigV1>, ProgramError> {
    let config = load_fixed_controller_account::<ControllerConfigV1>(
        program_id,
        config_info,
        ControllerConfigV1::LEN,
    )?;
    config.validate_static()?;
    if derive_controller_config_pda(program_id, &config.target_program)
        != (*config_info.key, config.bump)
        || derive_authority_pda(program_id, &config.target_program).0 != config.authority_pda
        || derive_gate_pda(program_id, &config.target_program).0 != config.gate_pda
        || config.upgradeable_loader != UPGRADEABLE_LOADER_ID
        || derive_upgradeable_programdata_address(&config.target_program).0
            != config.target_programdata
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    Ok(config)
}

fn load_gate(
    program_id: &Pubkey,
    gate_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<ProtocolGateV1>, ProgramError> {
    let gate = load_fixed_controller_account::<ProtocolGateV1>(
        program_id,
        gate_info,
        ProtocolGateV1::LEN,
    )?;
    gate.validate_static()?;
    if derive_gate_pda(program_id, &config.target_program) != (*gate_info.key, gate.bump)
        || *gate_info.key != config.gate_pda
        || gate.controller_config != *config_info.key
        || gate.target_program != config.target_program
        || gate.target_programdata != config.target_programdata
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(gate)
}

fn load_capacity_policy(
    program_id: &Pubkey,
    capacity_policy_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<ProgramDataCapacityPolicyV1>, ProgramError> {
    let policy = load_fixed_controller_account::<ProgramDataCapacityPolicyV1>(
        program_id,
        capacity_policy_info,
        ProgramDataCapacityPolicyV1::LEN,
    )?;
    validate_capacity_policy_digest_v1(&policy)?;
    if derive_capacity_policy_pda(program_id, &config.target_program)
        != (*capacity_policy_info.key, policy.bump)
        || policy.controller_program != *program_id
        || policy.controller_config != *config_info.key
        || policy.target_program != config.target_program
        || policy.target_programdata != config.target_programdata
        || policy.upgradeable_loader != config.upgradeable_loader
    {
        return Err(GovernanceError::CapacityPolicyMismatch.into());
    }
    Ok(policy)
}

#[allow(clippy::too_many_arguments)]
fn load_observation(
    program_id: &Pubkey,
    observation_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    capacity_policy_info: &AccountInfo<'_>,
    subject_info: &AccountInfo<'_>,
    observed_program_info: &AccountInfo<'_>,
    observed_programdata_info: &AccountInfo<'_>,
    loader_info: &AccountInfo<'_>,
    guard: &ProgramDataObservationGuardV1,
    capacity_policy: &ProgramDataCapacityPolicyV1,
) -> Result<Box<ProgramDataObservationV1>, ProgramError> {
    let observation = load_fixed_controller_account::<ProgramDataObservationV1>(
        program_id,
        observation_info,
        ProgramDataObservationV1::LEN,
    )?;
    observation.validate_static()?;
    if derive_programdata_observation_pda(
        program_id,
        observed_program_info.key,
        guard.purpose as u8,
        &guard.expected_subject_digest,
        guard.generation,
    ) != (*observation_info.key, observation.bump)
        || observation.controller_program != *program_id
        || observation.controller_config != *config_info.key
        || observation.capacity_policy != *capacity_policy_info.key
        || observation.capacity_policy_digest != capacity_policy.policy_digest
        || observation.purpose != guard.purpose
        || observation.subject != *subject_info.key
        || observation.subject_digest != guard.expected_subject_digest
        || observation.generation != guard.generation
        || observation.target_program != *observed_program_info.key
        || observation.target_programdata != *observed_programdata_info.key
        || observation.upgradeable_loader != *loader_info.key
        || observation.expected_artifact_scheme_id != capacity_policy.artifact_scheme_id
        || observation.artifact_chunk_size != capacity_policy.artifact_chunk_size
        || observation.raw_observation_scheme_id != capacity_policy.observation_scheme_id
        || observation.raw_chunk_size != capacity_policy.observation_chunk_size
    {
        return Err(GovernanceError::InvalidProgramDataObservation.into());
    }
    let subject_digest = compute_programdata_observation_subject_digest_v1(
        program_id,
        config_info.key,
        observed_program_info.key,
        observed_programdata_info.key,
        observation.purpose,
        subject_info.key,
        observation.generation,
        &observation.protocol_gate,
        observation.gate_status,
        observation.gate_epoch,
        &observation.gate_active_proposal,
        observation.gate_freeze_slot,
        observation.gate_freeze_reason_code,
        &capacity_policy.policy_digest,
        observation.expected_artifact_length,
        &observation.expected_artifact_sha256,
        &observation.expected_artifact_merkle_root,
        &observation.expected_artifact_scheme_id,
        observation.minimum_required_capacity,
    )?;
    if subject_digest != observation.subject_digest {
        return Err(GovernanceError::InvalidProgramDataObservation.into());
    }
    Ok(observation)
}

fn validate_guard(guard: &ProgramDataObservationGuardV1, gate: &ProtocolGateV1) -> ProgramResult {
    guard.validate()?;
    if gate.status != guard.expected_gate_status
        || gate.epoch != guard.expected_gate_epoch
        || gate.freeze_reason_code != guard.expected_freeze_reason_code
        || gate.freeze_slot != guard.expected_freeze_slot
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(())
}

fn validate_observation_gate_snapshot(
    observation: &ProgramDataObservationV1,
    gate_key: &Pubkey,
    gate: &ProtocolGateV1,
) -> ProgramResult {
    if observation.protocol_gate != *gate_key
        || observation.gate_status != gate.status
        || observation.gate_epoch != gate.epoch
        || observation.gate_active_proposal != gate.active_proposal
        || observation.gate_freeze_slot != gate.freeze_slot
        || observation.gate_freeze_reason_code != gate.freeze_reason_code
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(())
}

fn validate_observation_purpose_gate(
    purpose: ProgramDataObservationPurposeV1,
    subject: &Pubkey,
    gate: &ProtocolGateV1,
) -> ProgramResult {
    let valid = match purpose {
        ProgramDataObservationPurposeV1::ControllerImmutability
        | ProgramDataObservationPurposeV1::TargetHandoffBridge
        | ProgramDataObservationPurposeV1::BootstrapActivation => {
            gate.status == GateStatusV1::EmergencyFrozen
                && gate.active_proposal == Pubkey::default()
                && gate.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        }
        ProgramDataObservationPurposeV1::ProposalPrestate
        | ProgramDataObservationPurposeV1::PostUpgrade
        | ProgramDataObservationPurposeV1::Rollback => {
            gate.status == GateStatusV1::FrozenForUpgrade
                && gate.active_proposal == *subject
                && gate.freeze_reason_code != BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        }
        ProgramDataObservationPurposeV1::EmergencyResolution => {
            gate.status == GateStatusV1::EmergencyFrozen
                && gate.active_proposal == Pubkey::default()
                && gate.freeze_reason_code != BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        }
    };
    if !valid {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(())
}

fn validate_subject_account(program_id: &Pubkey, subject_info: &AccountInfo<'_>) -> ProgramResult {
    if subject_info.owner != program_id || subject_info.data_len() == 0 {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    Ok(())
}

fn validate_observed_identity(
    program_id: &Pubkey,
    purpose: ProgramDataObservationPurposeV1,
    config: &ControllerConfigV1,
    observed_program: &Pubkey,
    observed_programdata: &Pubkey,
) -> ProgramResult {
    let (expected_program, expected_programdata) =
        if purpose == ProgramDataObservationPurposeV1::ControllerImmutability {
            (
                *program_id,
                derive_upgradeable_programdata_address(program_id).0,
            )
        } else {
            (config.target_program, config.target_programdata)
        };
    if *observed_program != expected_program || *observed_programdata != expected_programdata {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

fn read_observed_graph(
    program_info: &AccountInfo<'_>,
    programdata_info: &AccountInfo<'_>,
    loader_info: &AccountInfo<'_>,
    capacity_policy: &ProgramDataCapacityPolicyV1,
) -> Result<ObservedProgramDataGraphV1, ProgramError> {
    if *loader_info.key != UPGRADEABLE_LOADER_ID
        || capacity_policy.upgradeable_loader != *loader_info.key
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let header =
        validate_program_programdata_linkage(program_info, programdata_info, loader_info.key)?;
    let program_data = program_info.try_borrow_data()?;
    let parsed_program = parse_upgradeable_program(&program_data)?;
    let mut program_header_snapshot = [0; LOADER_PROGRAM_ACCOUNT_LEN];
    program_header_snapshot.copy_from_slice(&program_data);
    let program_data_length =
        u64::try_from(program_data.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    drop(program_data);

    let programdata_data = programdata_info.try_borrow_data()?;
    let parsed_programdata = parse_upgradeable_programdata(&programdata_data)?;
    let raw_data_length =
        u64::try_from(programdata_data.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    if raw_data_length > capacity_policy.maximum_raw_programdata_length
        || parsed_programdata.payload_offset != LOADER_PROGRAMDATA_METADATA_LEN
    {
        return Err(GovernanceError::InvalidProgramDataObservation.into());
    }
    let mut programdata_header_snapshot = [0; LOADER_PROGRAMDATA_METADATA_LEN];
    programdata_header_snapshot.copy_from_slice(
        programdata_data
            .get(..LOADER_PROGRAMDATA_METADATA_LEN)
            .ok_or(GovernanceError::InvalidProgramDataObservation)?,
    );
    drop(programdata_data);
    let actual_capacity = u64::try_from(parsed_programdata.capacity)
        .map_err(|_| GovernanceError::ArithmeticOverflow)?;
    if actual_capacity > capacity_policy.maximum_payload_capacity || header != parsed_programdata {
        return Err(GovernanceError::InvalidProgramDataObservation.into());
    }

    Ok(ObservedProgramDataGraphV1 {
        program_owner: *program_info.owner,
        program_executable: program_info.executable,
        program_data_length,
        program_header_snapshot,
        linked_programdata: parsed_program.programdata_address,
        programdata_owner: *programdata_info.owner,
        programdata_executable: programdata_info.executable,
        programdata_header_snapshot,
        deployed_slot: parsed_programdata.deployed_slot,
        upgrade_authority: option_to_state_authority(parsed_programdata.upgrade_authority)?,
        raw_data_length,
        actual_capacity,
    })
}

fn validate_graph_fresh(
    observation: &ProgramDataObservationV1,
    graph: &ObservedProgramDataGraphV1,
) -> ProgramResult {
    if observation.program_owner != graph.program_owner
        || observation.program_executable != graph.program_executable
        || observation.program_data_length != graph.program_data_length
        || observation.program_header_snapshot != graph.program_header_snapshot
        || observation.linked_programdata != graph.linked_programdata
        || observation.programdata_owner != graph.programdata_owner
        || observation.programdata_executable != graph.programdata_executable
        || observation.programdata_header_snapshot != graph.programdata_header_snapshot
        || observation.deployed_slot != graph.deployed_slot
        || observation.upgrade_authority != graph.upgrade_authority
        || observation.raw_data_length != graph.raw_data_length
        || observation.actual_capacity != graph.actual_capacity
        || graph.actual_capacity < observation.minimum_required_capacity
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(())
}

fn append_raw_record_in_place(
    observation: &mut ProgramDataObservationV1,
    chunk_index: u32,
    exact_chunk: &[u8],
    slot: u64,
) -> ProgramResult {
    require_accumulating(observation, slot)?;
    if chunk_index != observation.next_raw_chunk_index {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let mut frontier = ProgramDataObservationMerkleFrontierV1 {
        hashes: observation.raw_frontier,
        occupied_mask: observation.raw_frontier_mask,
        next_index: observation.next_raw_chunk_index,
    };
    append_programdata_observation_chunk(
        &mut frontier,
        &observation.subject_digest,
        observation.raw_data_length,
        observation.raw_chunk_size,
        chunk_index,
        exact_chunk,
    )?;
    scan_tail_intersection(observation, chunk_index, exact_chunk)?;
    observation.raw_frontier = frontier.hashes;
    observation.raw_frontier_mask = frontier.occupied_mask;
    observation.next_raw_chunk_index = frontier.next_index;
    observation.last_observed_slot = slot;
    set_ready_if_complete(observation)?;
    observation.validate_static()?;
    Ok(())
}

fn verify_artifact_record_in_place(
    observation: &mut ProgramDataObservationV1,
    chunk_index: u32,
    exact_chunk: &[u8],
    proof: &[[u8; 32]],
    slot: u64,
) -> ProgramResult {
    require_accumulating(observation, slot)?;
    if chunk_index != observation.next_artifact_chunk_index {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    verify_artifact_chunk_proof(
        &observation.expected_artifact_merkle_root,
        observation.expected_artifact_length,
        observation.artifact_chunk_size,
        chunk_index,
        exact_chunk,
        proof,
    )?;
    observation.next_artifact_chunk_index = observation
        .next_artifact_chunk_index
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    observation.last_observed_slot = slot;
    set_ready_if_complete(observation)?;
    observation.validate_static()?;
    Ok(())
}

fn finalize_observation_record_in_place(
    observation: &mut ProgramDataObservationV1,
    slot: u64,
) -> ProgramResult {
    if observation.status != ProgramDataObservationStatusV1::ReadyToFinalize
        || slot < observation.last_observed_slot
    {
        return Err(GovernanceError::IncompleteProgramDataObservation.into());
    }
    let frontier = ProgramDataObservationMerkleFrontierV1 {
        hashes: observation.raw_frontier,
        occupied_mask: observation.raw_frontier_mask,
        next_index: observation.next_raw_chunk_index,
    };
    let root = finalize_programdata_observation_frontier(
        &frontier,
        &observation.subject_digest,
        observation.raw_data_length,
        observation.raw_chunk_size,
    )?;
    observation.final_raw_merkle_root = root;
    observation.finalized_slot = slot;
    observation.last_observed_slot = slot;
    observation.status = ProgramDataObservationStatusV1::Finalized;
    observation.observation_digest = compute_programdata_observation_digest_v1(observation)?;
    validate_programdata_observation_digest_v1(observation)?;
    Ok(())
}

#[cfg(test)]
fn append_raw_record(
    observation: &ProgramDataObservationV1,
    chunk_index: u32,
    exact_chunk: &[u8],
    slot: u64,
) -> Result<ProgramDataObservationV1, ProgramError> {
    let mut next = observation.clone();
    append_raw_record_in_place(&mut next, chunk_index, exact_chunk, slot)?;
    Ok(next)
}

#[cfg(test)]
fn verify_artifact_record(
    observation: &ProgramDataObservationV1,
    chunk_index: u32,
    exact_chunk: &[u8],
    proof: &[[u8; 32]],
    slot: u64,
) -> Result<ProgramDataObservationV1, ProgramError> {
    let mut next = observation.clone();
    verify_artifact_record_in_place(&mut next, chunk_index, exact_chunk, proof, slot)?;
    Ok(next)
}

#[cfg(test)]
fn finalize_observation_record(
    observation: &ProgramDataObservationV1,
    slot: u64,
) -> Result<ProgramDataObservationV1, ProgramError> {
    let mut next = observation.clone();
    finalize_observation_record_in_place(&mut next, slot)?;
    Ok(next)
}

fn require_accumulating(observation: &ProgramDataObservationV1, slot: u64) -> ProgramResult {
    if observation.status != ProgramDataObservationStatusV1::Accumulating
        || slot < observation.last_observed_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    Ok(())
}

fn set_ready_if_complete(observation: &mut ProgramDataObservationV1) -> ProgramResult {
    let expected_tail = observation
        .actual_capacity
        .checked_sub(observation.expected_artifact_length)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if observation.next_raw_chunk_index == observation.raw_chunk_count
        && observation.next_artifact_chunk_index == observation.artifact_chunk_count
        && observation.tail_bytes_verified == expected_tail
    {
        observation.status = ProgramDataObservationStatusV1::ReadyToFinalize;
    }
    Ok(())
}

fn scan_tail_intersection(
    observation: &mut ProgramDataObservationV1,
    chunk_index: u32,
    exact_chunk: &[u8],
) -> ProgramResult {
    let raw_start = u64::from(chunk_index)
        .checked_mul(u64::from(observation.raw_chunk_size))
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let raw_end = raw_start
        .checked_add(
            u64::try_from(exact_chunk.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?,
        )
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let tail_start = u64::from(observation.payload_offset)
        .checked_add(observation.expected_artifact_length)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let intersection_start = raw_start.max(tail_start);
    let intersection_end = raw_end.min(observation.raw_data_length);
    if intersection_start >= intersection_end {
        return Ok(());
    }
    let expected_start = tail_start
        .checked_add(observation.tail_bytes_verified)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if intersection_start != expected_start {
        return Err(GovernanceError::InvalidProgramDataObservation.into());
    }
    let local_start = usize::try_from(intersection_start - raw_start)
        .map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let local_end = usize::try_from(intersection_end - raw_start)
        .map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let tail = exact_chunk
        .get(local_start..local_end)
        .ok_or(GovernanceError::InvalidProgramDataObservation)?;
    if tail.iter().any(|byte| *byte != 0) {
        return Err(GovernanceError::InvalidProgramDataObservation.into());
    }
    observation.tail_bytes_verified = observation
        .tail_bytes_verified
        .checked_add(u64::try_from(tail.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    Ok(())
}

fn exact_raw_chunk<'a>(
    observation: &ProgramDataObservationV1,
    chunk_index: u32,
    raw_programdata: &'a [u8],
) -> Result<&'a [u8], ProgramError> {
    if u64::try_from(raw_programdata.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?
        != observation.raw_data_length
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    let start = u64::from(chunk_index)
        .checked_mul(u64::from(observation.raw_chunk_size))
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let end = start
        .checked_add(u64::from(observation.raw_chunk_size))
        .ok_or(GovernanceError::ArithmeticOverflow)?
        .min(observation.raw_data_length);
    let start = usize::try_from(start).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let end = usize::try_from(end).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    if start >= end {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    raw_programdata
        .get(start..end)
        .ok_or_else(|| GovernanceError::StaleProgramDataObservation.into())
}

fn exact_artifact_chunk<'a>(
    observation: &ProgramDataObservationV1,
    chunk_index: u32,
    raw_programdata: &'a [u8],
) -> Result<&'a [u8], ProgramError> {
    if chunk_index >= observation.artifact_chunk_count {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let payload_start = u64::from(observation.payload_offset);
    let chunk_start = u64::from(chunk_index)
        .checked_mul(u64::from(observation.artifact_chunk_size))
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let start = payload_start
        .checked_add(chunk_start)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let end = start
        .checked_add(u64::from(observation.artifact_chunk_size))
        .ok_or(GovernanceError::ArithmeticOverflow)?
        .min(
            payload_start
                .checked_add(observation.expected_artifact_length)
                .ok_or(GovernanceError::ArithmeticOverflow)?,
        );
    let start = usize::try_from(start).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let end = usize::try_from(end).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    raw_programdata
        .get(start..end)
        .ok_or_else(|| GovernanceError::StaleProgramDataObservation.into())
}

fn option_to_state_authority(authority: Option<Pubkey>) -> Result<OptionalPubkeyV1, ProgramError> {
    match authority {
        Some(authority) => Ok(OptionalPubkeyV1::some(authority)?),
        None => Ok(OptionalPubkeyV1::none()),
    }
}

fn instruction_authority_to_state(
    authority: &ObservationAuthorityV1,
) -> Result<OptionalPubkeyV1, ProgramError> {
    authority.validate()?;
    if authority.present {
        Ok(OptionalPubkeyV1::some(authority.value)?)
    } else {
        Ok(OptionalPubkeyV1::none())
    }
}

fn current_slot() -> Result<u64, ProgramError> {
    let slot = Clock::get()?.slot;
    if slot == 0 {
        return Err(GovernanceError::InvalidProgramDataObservation.into());
    }
    Ok(slot)
}

fn require_observation_begins_after_deployment(
    deployed_slot: u64,
    observation_slot: u64,
) -> ProgramResult {
    if deployed_slot == 0 || observation_slot <= deployed_slot {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        artifact_merkle::{artifact_merkle_proof, artifact_merkle_root, ARTIFACT_MERKLE_SCHEME_ID},
        programdata_observation_merkle::{
            programdata_observation_merkle_root, PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB,
        },
    };
    use solana_program::hash::hashv;

    fn key(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    fn raw_programdata(payload: &[u8], capacity: usize) -> Vec<u8> {
        let mut raw = vec![0; LOADER_PROGRAMDATA_METADATA_LEN + capacity];
        raw[..4].copy_from_slice(&3u32.to_le_bytes());
        raw[4..12].copy_from_slice(&7u64.to_le_bytes());
        raw[12] = 1;
        raw[13..45].copy_from_slice(key(9).as_ref());
        raw[45..45 + payload.len()].copy_from_slice(payload);
        raw
    }

    fn observation(payload: &[u8], capacity: usize, generation: u64) -> ProgramDataObservationV1 {
        let controller = key(1);
        let config = key(2);
        let target = key(3);
        let target_programdata = key(4);
        let subject = key(5);
        let capacity_policy_digest = [6; 32];
        let artifact_root = artifact_merkle_root(payload, ARTIFACT_BINDING_CHUNK_SIZE_V1).unwrap();
        let artifact_sha = hashv(&[payload]).to_bytes();
        let minimum = payload.len() as u64;
        let subject_digest = compute_programdata_observation_subject_digest_v1(
            &controller,
            &config,
            &target,
            &target_programdata,
            ProgramDataObservationPurposeV1::TargetHandoffBridge,
            &subject,
            generation,
            &key(20),
            GateStatusV1::EmergencyFrozen,
            21,
            &Pubkey::default(),
            23,
            BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
            &capacity_policy_digest,
            payload.len() as u64,
            &artifact_sha,
            &artifact_root,
            &ARTIFACT_MERKLE_SCHEME_ID,
            minimum,
        )
        .unwrap();
        let raw = raw_programdata(payload, capacity);
        let raw_len = raw.len() as u64;
        let raw_chunk_size = PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB;
        let raw_count = programdata_observation_chunk_count(raw_len, raw_chunk_size).unwrap();
        let padded = raw_count.next_power_of_two();
        let mut program_header = [0; LOADER_PROGRAM_ACCOUNT_LEN];
        program_header[..4].copy_from_slice(&2u32.to_le_bytes());
        program_header[4..].copy_from_slice(target_programdata.as_ref());
        let mut programdata_header = [0; LOADER_PROGRAMDATA_METADATA_LEN];
        programdata_header.copy_from_slice(&raw[..LOADER_PROGRAMDATA_METADATA_LEN]);
        let value = ProgramDataObservationV1 {
            discriminator: PROGRAMDATA_OBSERVATION_V1_DISCRIMINATOR,
            version: CEREMONY_ACCOUNT_VERSION_V1,
            bump: 1,
            initialized: true,
            controller_program: controller,
            controller_config: config,
            capacity_policy: key(7),
            capacity_policy_digest,
            purpose: ProgramDataObservationPurposeV1::TargetHandoffBridge,
            subject,
            subject_digest,
            generation,
            protocol_gate: key(20),
            gate_status: GateStatusV1::EmergencyFrozen,
            gate_epoch: 21,
            gate_active_proposal: Pubkey::default(),
            gate_freeze_slot: 23,
            gate_freeze_reason_code: BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
            target_program: target,
            target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            program_owner: UPGRADEABLE_LOADER_ID,
            program_executable: true,
            program_data_length: LOADER_PROGRAM_ACCOUNT_LEN as u64,
            program_header_present: true,
            program_header_snapshot: program_header,
            linked_programdata: target_programdata,
            programdata_owner: UPGRADEABLE_LOADER_ID,
            programdata_executable: false,
            programdata_header_present: true,
            programdata_header_snapshot: programdata_header,
            deployed_slot: 7,
            upgrade_authority: OptionalPubkeyV1::some(key(9)).unwrap(),
            raw_data_length: raw_len,
            payload_offset: PROGRAMDATA_PAYLOAD_OFFSET_V1,
            actual_capacity: capacity as u64,
            expected_artifact_length: payload.len() as u64,
            expected_artifact_sha256: artifact_sha,
            expected_artifact_merkle_root: artifact_root,
            expected_artifact_scheme_id: crate::artifact_merkle::ARTIFACT_MERKLE_SCHEME_ID,
            artifact_chunk_size: ARTIFACT_BINDING_CHUNK_SIZE_V1,
            artifact_chunk_count: artifact_chunk_count(
                payload.len() as u64,
                ARTIFACT_BINDING_CHUNK_SIZE_V1,
            )
            .unwrap(),
            minimum_required_capacity: minimum,
            raw_observation_scheme_id: PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
            raw_chunk_size,
            raw_chunk_count: raw_count,
            raw_padded_leaf_count: padded,
            raw_tree_depth: padded.trailing_zeros() as u8,
            next_raw_chunk_index: 0,
            raw_frontier: [[0; 32]; 11],
            raw_frontier_mask: 0,
            next_artifact_chunk_index: 0,
            tail_bytes_verified: 0,
            start_slot: 1,
            last_observed_slot: 1,
            finalized_slot: 0,
            final_raw_merkle_root: [0; 32],
            observation_digest: [0; 32],
            status: ProgramDataObservationStatusV1::Accumulating,
            reserved: [0; PROGRAMDATA_OBSERVATION_V1_RESERVED_LEN],
        };
        value.validate_static().unwrap();
        value
    }

    #[test]
    fn raw_artifact_tail_and_finalization_form_one_complete_proof() {
        let payload = b"artifact";
        let raw = raw_programdata(payload, 32);
        let original = observation(payload, 32, 1);
        let raw_done = append_raw_record(&original, 0, &raw, 2).unwrap();
        assert_eq!(raw_done.next_raw_chunk_index, 1);
        assert_eq!(raw_done.tail_bytes_verified, 24);
        assert_eq!(
            raw_done.status,
            ProgramDataObservationStatusV1::Accumulating
        );

        let proof = artifact_merkle_proof(payload, ARTIFACT_BINDING_CHUNK_SIZE_V1, 0).unwrap();
        let ready = verify_artifact_record(&raw_done, 0, payload, &proof, 3).unwrap();
        assert_eq!(
            ready.status,
            ProgramDataObservationStatusV1::ReadyToFinalize
        );
        let finalized = finalize_observation_record(&ready, 4).unwrap();
        assert_eq!(finalized.status, ProgramDataObservationStatusV1::Finalized);
        assert_eq!(
            finalized.final_raw_merkle_root,
            programdata_observation_merkle_root(
                &original.subject_digest,
                &raw,
                original.raw_chunk_size,
            )
            .unwrap()
        );
        validate_programdata_observation_digest_v1(&finalized).unwrap();
    }

    #[test]
    fn nonzero_tail_fails_without_mutating_the_record() {
        let payload = b"artifact";
        let mut raw = raw_programdata(payload, 32);
        raw[45 + payload.len()] = 1;
        let original = observation(payload, 32, 1);
        let before = original.clone();
        assert!(append_raw_record(&original, 0, &raw, 2).is_err());
        assert_eq!(original, before);
    }

    #[test]
    fn duplicate_out_of_order_and_wrong_proof_fail_closed() {
        let payload = vec![7; ARTIFACT_BINDING_CHUNK_SIZE_V1 as usize + 3];
        let raw = raw_programdata(&payload, payload.len() + 5);
        let original = observation(&payload, payload.len() + 5, 1);
        assert!(append_raw_record(&original, 1, &raw, 2).is_err());
        let first = exact_raw_chunk(&original, 0, &raw).unwrap();
        let after_first = append_raw_record(&original, 0, first, 2).unwrap();
        assert!(append_raw_record(&after_first, 0, first, 3).is_err());

        let mut proof = artifact_merkle_proof(&payload, ARTIFACT_BINDING_CHUNK_SIZE_V1, 0).unwrap();
        proof[0][0] ^= 1;
        let artifact_first = &payload[..ARTIFACT_BINDING_CHUNK_SIZE_V1 as usize];
        assert!(verify_artifact_record(&original, 0, artifact_first, &proof, 2).is_err());
        assert_eq!(original.next_artifact_chunk_index, 0);
    }

    #[test]
    fn incomplete_or_repeated_finalization_is_rejected() {
        let payload = b"artifact";
        let original = observation(payload, 16, 1);
        assert!(finalize_observation_record(&original, 2).is_err());
        let raw = raw_programdata(payload, 16);
        let raw_done = append_raw_record(&original, 0, &raw, 2).unwrap();
        let proof = artifact_merkle_proof(payload, ARTIFACT_BINDING_CHUNK_SIZE_V1, 0).unwrap();
        let ready = verify_artifact_record(&raw_done, 0, payload, &proof, 3).unwrap();
        let finalized = finalize_observation_record(&ready, 4).unwrap();
        assert!(finalize_observation_record(&finalized, 5).is_err());
    }

    #[test]
    fn every_bound_graph_field_is_stale_sensitive() {
        let observation = observation(b"artifact", 16, 1);
        let base = ObservedProgramDataGraphV1 {
            program_owner: observation.program_owner,
            program_executable: observation.program_executable,
            program_data_length: observation.program_data_length,
            program_header_snapshot: observation.program_header_snapshot,
            linked_programdata: observation.linked_programdata,
            programdata_owner: observation.programdata_owner,
            programdata_executable: observation.programdata_executable,
            programdata_header_snapshot: observation.programdata_header_snapshot,
            deployed_slot: observation.deployed_slot,
            upgrade_authority: observation.upgrade_authority,
            raw_data_length: observation.raw_data_length,
            actual_capacity: observation.actual_capacity,
        };
        validate_graph_fresh(&observation, &base).unwrap();
        let mut changed = base.clone();
        changed.deployed_slot += 1;
        assert!(validate_graph_fresh(&observation, &changed).is_err());
        changed = base.clone();
        changed.actual_capacity += 1;
        changed.raw_data_length += 1;
        assert!(validate_graph_fresh(&observation, &changed).is_err());
        changed = base;
        changed.programdata_header_snapshot[44] ^= 1;
        assert!(validate_graph_fresh(&observation, &changed).is_err());
    }

    #[test]
    fn generations_have_distinct_canonical_restart_addresses() {
        let controller = key(1);
        let target = key(2);
        let subject_digest = [3; 32];
        let first = derive_programdata_observation_pda(
            &controller,
            &target,
            ProgramDataObservationPurposeV1::TargetHandoffBridge as u8,
            &subject_digest,
            1,
        );
        let second = derive_programdata_observation_pda(
            &controller,
            &target,
            ProgramDataObservationPurposeV1::TargetHandoffBridge as u8,
            &subject_digest,
            2,
        );
        let other_purpose = derive_programdata_observation_pda(
            &controller,
            &target,
            ProgramDataObservationPurposeV1::BootstrapActivation as u8,
            &subject_digest,
            1,
        );
        let other_subject = derive_programdata_observation_pda(
            &controller,
            &target,
            ProgramDataObservationPurposeV1::TargetHandoffBridge as u8,
            &[4; 32],
            1,
        );
        assert_ne!(first.0, second.0);
        assert_ne!(first.0, other_purpose.0);
        assert_ne!(first.0, other_subject.0);
    }

    #[test]
    fn observation_cannot_begin_in_the_deployment_slot() {
        assert_eq!(
            require_observation_begins_after_deployment(41, 41),
            Err(GovernanceError::StaleProgramDataObservation.into())
        );
        assert_eq!(
            require_observation_begins_after_deployment(41, 40),
            Err(GovernanceError::StaleProgramDataObservation.into())
        );
        require_observation_begins_after_deployment(41, 42).unwrap();
    }
}
