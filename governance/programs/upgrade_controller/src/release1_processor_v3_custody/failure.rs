use super::*;

/// Constructs immutable V2 failure evidence from live accounts and a compact
/// authenticated witness.  Structural fields are never operator-trusted;
/// payload leaves authenticate against the proposal root and zero-tail hashes
/// are derived from the exact on-chain bytes.
pub fn process_observe_programdata_failure_witness_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ObserveProgramDataFailureV2,
) -> ProgramResult {
    let instruction = instruction.witness;
    let [payer, config_info, gate_info, primary_info, verification_info, capacity_info, deployment_info, observation_info, target_program, target_programdata, authority_info, loader_info, failure_info, system_program_info] =
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
        capacity_info,
        deployment_info,
        observation_info,
        authority_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    // Executable state is itself failure evidence for these two accounts, so
    // only their signer/writable privileges are fixed at the ABI boundary.
    if target_program.is_signer
        || target_program.is_writable
        || target_programdata.is_signer
        || target_programdata.is_writable
    {
        return Err(GovernanceError::InvalidAccountPrivileges.into());
    }
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(failure_info, true, false, false)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&[
        payer,
        config_info,
        gate_info,
        primary_info,
        verification_info,
        capacity_info,
        deployment_info,
        observation_info,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        failure_info,
        system_program_info,
    ])?;
    require_program_ids(loader_info, Some(system_program_info), None, None, None)?;
    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let proposal = load_proposal(program_id, primary_info, config_info, &config)?;
    validate_proposal_guard(
        &instruction.expected_proposal,
        &proposal,
        &config,
        &gate,
        primary_info.key,
    )?;
    if proposal.proposal_class == ProposalClassV1::EmergencyRollback
        || proposal.state != ProposalStateV2::UpgradeExecuted
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let capacity = load_capacity_policy(program_id, capacity_info, config_info, &config)?;
    let deployment = load_current_deployment(
        program_id,
        deployment_info,
        config_info,
        capacity_info,
        &config,
        &capacity,
    )?;
    require_deployment_guard(&instruction.expected_proposal, &proposal, &deployment)?;
    let verification_present = instruction.expected_verification_digest != [0; 32];
    if verification_present {
        let verification = load_programdata_verification(
            program_id,
            verification_info,
            primary_info,
            config_info,
            &proposal,
        )?;
        if verification.status == ProgramDataVerificationStatusV2::Verified
            || verification.verification_digest != instruction.expected_verification_digest
            || verification.verification_generation != instruction.expected_verification_generation
        {
            return Err(GovernanceError::InvalidStateTransition.into());
        }
    } else if *verification_info.key != proposal.programdata_verification
        || verification_info.owner != &system_program::ID
        || verification_info.data_len() != 0
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let observation = load_observation_any_status(
        program_id,
        observation_info,
        config_info,
        capacity_info,
        primary_info,
        &config,
        &capacity,
    )?;
    let observation_bytes = observation_info.try_borrow_data()?;
    let observation_state_hash = hashv(&[&observation_bytes]).to_bytes();
    drop(observation_bytes);
    if instruction.expected_observation_state_hash == [0; 32]
        || observation.generation != instruction.expected_observation_generation
        || observation_state_hash != instruction.expected_observation_state_hash
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    validate_failure_observation_binding(&observation, &proposal, &capacity)?;
    if *target_program.key != config.target_program
        || *target_programdata.key != config.target_programdata
        || *authority_info.key != config.authority_pda
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let runtime = capture_runtime_graph(target_program, target_programdata)?;
    let (expected_leaf_hash, actual_leaf_hash) = validate_failure_witness(
        &instruction,
        &proposal,
        &capacity,
        &observation,
        target_programdata,
        &runtime,
        &config,
    )?;
    let slot = Clock::get()?.slot;
    if slot == 0
        || slot < proposal.upgrade_executed_slot
        || slot > instruction.plan_valid_until_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let (expected_failure, failure_bump) =
        derive_programdata_failure_observation_pda(program_id, primary_info.key, gate.epoch);
    if expected_failure != *failure_info.key
        || failure_info.owner != &system_program::ID
        || failure_info.data_len() != 0
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    let observation_finalized = observation.status == ProgramDataObservationStatusV1::Finalized;
    let mut failure = ProgramDataFailureObservationV2 {
        discriminator: PROGRAMDATA_FAILURE_OBSERVATION_V2_DISCRIMINATOR,
        account_version: CAPACITY_SAFE_ACCOUNT_VERSION_V2,
        bump: failure_bump,
        initialized: true,
        finalized: true,
        controller_config: *config_info.key,
        protocol_gate: *gate_info.key,
        primary_proposal: *primary_info.key,
        proposal_digest: proposal.proposal_digest,
        capacity_policy: *capacity_info.key,
        capacity_policy_digest: capacity.policy_digest,
        current_deployment_state: *deployment_info.key,
        current_deployment_digest: deployment.deployment_digest,
        current_deployment_generation: deployment.deployment_generation,
        target_program: *target_program.key,
        target_programdata: *target_programdata.key,
        upgradeable_loader: *loader_info.key,
        frozen_epoch: gate.epoch,
        target_nonce: config.target_nonce,
        expected_artifact_length: proposal.artifact_length,
        expected_artifact_sha256: proposal.artifact_sha256,
        expected_artifact_merkle_root: proposal.artifact_chunk_merkle_root,
        expected_artifact_scheme_id: proposal.artifact_scheme_id,
        minimum_required_capacity: proposal.minimum_required_capacity,
        observation_scheme_id: capacity.observation_scheme_id,
        programdata_observation: *observation_info.key,
        observation_purpose: observation.purpose,
        observation_generation: observation.generation,
        observation_subject_digest: observation.subject_digest,
        observation_finalized,
        observation_root: if observation_finalized {
            observation.final_raw_merkle_root
        } else {
            [0; 32]
        },
        observation_digest: if observation_finalized {
            observation.observation_digest
        } else {
            [0; 32]
        },
        actual_program_owner: runtime.program_owner,
        actual_program_executable: runtime.program_executable,
        actual_program_data_length: runtime.program_data_length,
        program_header_present: runtime.program_header_present,
        actual_linked_programdata: runtime.linked_programdata,
        actual_programdata_owner: runtime.programdata_owner,
        actual_programdata_executable: runtime.programdata_executable,
        actual_programdata_data_length: runtime.programdata_data_length,
        programdata_header_present: runtime.programdata_header_present,
        actual_programdata_slot: runtime.deployed_slot,
        actual_capacity: runtime.actual_capacity,
        actual_authority: runtime.authority,
        mismatch_class: instruction.mismatch_class,
        failing_chunk_index: instruction.failing_chunk_index,
        expected_leaf_hash,
        actual_leaf_hash,
        failure_digest_domain_id: PROGRAMDATA_FAILURE_OBSERVATION_V2_DIGEST_DOMAIN_ID,
        failure_digest: [0; 32],
        finalized_slot: slot,
        reserved: [0; PROGRAMDATA_FAILURE_OBSERVATION_V2_RESERVED_LEN],
    };
    failure.failure_digest = compute_programdata_failure_observation_digest_v2(&failure)?;
    validate_programdata_failure_observation_digest_v2(&failure)?;
    let bytes = encode_fixed_account(&failure, ProgramDataFailureObservationV2::LEN)?;
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
        ProgramDataFailureObservationV2::LEN,
        signer_seeds,
    )?;
    commit_one_fixed_account(
        program_id,
        failure_info,
        &bytes,
        ProgramDataFailureObservationV2::LEN,
    )
}

pub(super) fn validate_failure_observation_binding(
    observation: &ProgramDataObservationV1,
    proposal: &UpgradeProposalV3,
    capacity: &ProgramDataCapacityPolicyV1,
) -> ProgramResult {
    if observation.purpose != ProgramDataObservationPurposeV1::PostUpgrade
        || observation.deployed_slot != proposal.upgrade_executed_slot
        || observation.start_slot <= proposal.upgrade_executed_slot
        || observation.expected_artifact_length != proposal.artifact_length
        || observation.expected_artifact_sha256 != proposal.artifact_sha256
        || observation.expected_artifact_merkle_root != proposal.artifact_chunk_merkle_root
        || observation.expected_artifact_scheme_id != proposal.artifact_scheme_id
        || observation.artifact_chunk_size != proposal.artifact_chunk_size
        || observation.artifact_chunk_count != proposal.artifact_chunk_count
        || observation.minimum_required_capacity != proposal.minimum_required_capacity
        || observation.raw_observation_scheme_id != capacity.observation_scheme_id
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_failure_witness(
    instruction: &ProgramDataFailureWitnessV2,
    proposal: &UpgradeProposalV3,
    capacity: &ProgramDataCapacityPolicyV1,
    observation: &ProgramDataObservationV1,
    target_programdata: &AccountInfo<'_>,
    runtime: &RuntimeProgramDataGraphV2,
    config: &ControllerConfigV1,
) -> Result<([u8; 32], [u8; 32]), ProgramError> {
    validate_recordable_programdata_mismatch_class(instruction.mismatch_class)?;
    let proof_present = instruction.proof.proof_len != 0
        || instruction.proof.nodes.iter().any(|node| *node != [0; 32]);
    match instruction.mismatch_class {
        ProgramDataMismatchClassV2::ArtifactPayload => {
            if instruction.failing_chunk_index == NO_FAILING_CHUNK_INDEX_V1
                || instruction.expected_leaf_hash == [0; 32]
            {
                return Err(GovernanceError::InvalidMerkleProof.into());
            }
        }
        ProgramDataMismatchClassV2::ZeroTail => {
            if instruction.failing_chunk_index == NO_FAILING_CHUNK_INDEX_V1
                || instruction.expected_leaf_hash != [0; 32]
                || proof_present
            {
                return Err(GovernanceError::InvalidMerkleProof.into());
            }
        }
        _ => {
            if instruction.failing_chunk_index != NO_FAILING_CHUNK_INDEX_V1
                || instruction.expected_leaf_hash != [0; 32]
                || proof_present
            {
                return Err(GovernanceError::InvalidMerkleProof.into());
            }
        }
    }
    let program_linkage = runtime.program_header_present
        && runtime.linked_programdata.present
        && runtime.linked_programdata.value == config.target_programdata;
    let program_owner = runtime.program_owner == config.upgradeable_loader;
    let program_executable = runtime.program_executable;
    let programdata_owner = runtime.programdata_owner == config.upgradeable_loader;
    let programdata_executable = !runtime.programdata_executable;
    let header_shape = runtime.programdata_header_present
        && runtime.deployed_slot != 0
        && runtime.programdata_data_length
            == runtime
                .actual_capacity
                .checked_add(LOADER_PROGRAMDATA_METADATA_LEN as u64)
                .ok_or(GovernanceError::ArithmeticOverflow)?;
    let authority = runtime.authority.present && runtime.authority.value == config.authority_pda;
    let canonical_prefix = program_linkage
        && program_owner
        && program_executable
        && programdata_owner
        && programdata_executable
        && header_shape;
    let observation_scheme_matches = observation.raw_observation_scheme_id
        == capacity.observation_scheme_id
        && observation.expected_artifact_scheme_id == proposal.artifact_scheme_id
        && observation.artifact_chunk_size == proposal.artifact_chunk_size
        && observation.artifact_chunk_count == proposal.artifact_chunk_count;
    let observation_artifact_matches = observation.expected_artifact_length
        == proposal.artifact_length
        && observation.expected_artifact_sha256 == proposal.artifact_sha256
        && observation.expected_artifact_merkle_root == proposal.artifact_chunk_merkle_root
        && observation.minimum_required_capacity == proposal.minimum_required_capacity;

    match instruction.mismatch_class {
        ProgramDataMismatchClassV2::ProgramLinkage => {
            if program_linkage {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok(([0; 32], [0; 32]))
        }
        ProgramDataMismatchClassV2::ProgramOwner => {
            if !program_linkage || program_owner {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok(([0; 32], [0; 32]))
        }
        ProgramDataMismatchClassV2::ProgramExecutable => {
            if !program_linkage || !program_owner || program_executable {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok(([0; 32], [0; 32]))
        }
        ProgramDataMismatchClassV2::ProgramDataOwner => {
            if !program_linkage || !program_owner || !program_executable || programdata_owner {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok(([0; 32], [0; 32]))
        }
        ProgramDataMismatchClassV2::ProgramDataExecutable => {
            if !program_linkage
                || !program_owner
                || !program_executable
                || !programdata_owner
                || programdata_executable
            {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok(([0; 32], [0; 32]))
        }
        ProgramDataMismatchClassV2::Header => {
            if program_linkage
                && program_owner
                && program_executable
                && programdata_owner
                && programdata_executable
                && header_shape
            {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            if !program_linkage
                || !program_owner
                || !program_executable
                || !programdata_owner
                || !programdata_executable
            {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok(([0; 32], [0; 32]))
        }
        ProgramDataMismatchClassV2::Authority => {
            if !canonical_prefix || authority {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok(([0; 32], [0; 32]))
        }
        ProgramDataMismatchClassV2::CapacityDecrease => {
            if !canonical_prefix
                || !authority
                || runtime.actual_capacity >= proposal.minimum_required_capacity
            {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok(([0; 32], [0; 32]))
        }
        ProgramDataMismatchClassV2::CapacityAboveRuntimeMaximum => {
            if !canonical_prefix
                || !authority
                || runtime.programdata_data_length <= capacity.maximum_raw_programdata_length
            {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok(([0; 32], [0; 32]))
        }
        ProgramDataMismatchClassV2::ArtifactLength => {
            // A Loader ProgramData account exposes capacity, not the logical
            // artifact length. The active proposal and PostUpgrade observation
            // are required to agree on the length before this helper runs, so
            // this class has no independent live proof in the compact V2 ABI.
            // Reject it instead of treating caller-authored observation fields
            // as rollback authority.
            Err(GovernanceError::InvalidRelease1Account.into())
        }
        ProgramDataMismatchClassV2::ObservationStale => {
            // Staleness is a restart signal, not rollback authority. In
            // particular, a benign zero-only extension must be handled by a
            // fresh observation generation.
            Err(GovernanceError::InvalidRelease1Account.into())
        }
        ProgramDataMismatchClassV2::ObservationScheme => {
            // The observation scheme is pinned by the capacity policy and has
            // no independent live mismatch witness. It cannot be downgraded
            // into a caller-created rollback trigger.
            Err(GovernanceError::InvalidRelease1Account.into())
        }
        ProgramDataMismatchClassV2::ArtifactPayload => {
            if !canonical_prefix
                || !authority
                || !observation_scheme_matches
                || !observation_artifact_matches
                || !runtime_matches_observation(runtime, observation)
                || runtime.actual_capacity < proposal.minimum_required_capacity
            {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            validate_expected_leaf_proof(
                &proposal.artifact_chunk_merkle_root,
                proposal.artifact_length,
                proposal.artifact_chunk_size,
                instruction.failing_chunk_index,
                &instruction.expected_leaf_hash,
                instruction.proof.proof_len,
                &instruction.proof.nodes,
            )?;
            let data = target_programdata.try_borrow_data()?;
            let payload = data
                .get(LOADER_PROGRAMDATA_METADATA_LEN..)
                .ok_or(GovernanceError::InvalidRelease1Account)?;
            let exact = exact_region_chunk(
                payload,
                0,
                proposal.artifact_length,
                proposal.artifact_chunk_size,
                instruction.failing_chunk_index,
            )?;
            let actual = artifact_chunk_leaf_hash(instruction.failing_chunk_index, exact)?;
            if actual == instruction.expected_leaf_hash {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            Ok((instruction.expected_leaf_hash, actual))
        }
        ProgramDataMismatchClassV2::ZeroTail => {
            if !canonical_prefix
                || !authority
                || !observation_scheme_matches
                || !observation_artifact_matches
                || !runtime_matches_observation(runtime, observation)
                || runtime.actual_capacity < proposal.minimum_required_capacity
            {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            let tail_length = runtime
                .actual_capacity
                .checked_sub(proposal.artifact_length)
                .ok_or(GovernanceError::InvalidCapacityPlan)?;
            let data = target_programdata.try_borrow_data()?;
            let payload = data
                .get(LOADER_PROGRAMDATA_METADATA_LEN..)
                .ok_or(GovernanceError::InvalidRelease1Account)?;
            let exact = exact_region_chunk(
                payload,
                proposal.artifact_length,
                tail_length,
                proposal.artifact_chunk_size,
                instruction.failing_chunk_index,
            )?;
            if exact.iter().all(|byte| *byte == 0) {
                return Err(GovernanceError::InvalidRelease1Account.into());
            }
            let expected =
                programdata_zero_tail_zero_hash(instruction.failing_chunk_index, exact.len())?;
            let actual = programdata_zero_tail_chunk_hash(instruction.failing_chunk_index, exact)?;
            Ok((expected, actual))
        }
    }
}

pub(super) fn validate_recordable_programdata_mismatch_class(
    mismatch_class: ProgramDataMismatchClassV2,
) -> ProgramResult {
    if matches!(
        mismatch_class,
        ProgramDataMismatchClassV2::ArtifactLength
            | ProgramDataMismatchClassV2::ObservationStale
            | ProgramDataMismatchClassV2::ObservationScheme
    ) {
        return Err(GovernanceError::InvalidRelease1Account.into());
    }
    Ok(())
}

pub(super) fn validate_rollback_activation_mismatch_class(
    mismatch_class: ProgramDataMismatchClassV2,
) -> ProgramResult {
    if !is_loader_executable_rollback_failure(mismatch_class) {
        return Err(GovernanceError::InvalidRelease1Account.into());
    }
    Ok(())
}
