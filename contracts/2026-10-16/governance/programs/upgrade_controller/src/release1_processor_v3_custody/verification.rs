use super::*;

/// Binds one finalized post-upgrade observation to the fixed verification PDA.
/// A stale accumulating generation may be replaced only by an exact digest
/// chain step; a verified generation is terminal.
pub fn process_bind_programdata_verification_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: BindProgramDataVerificationV2,
) -> ProgramResult {
    let [payer, config_info, gate_info, proposal_info, capacity_info, deployment_info, observation_info, target_program, target_programdata, authority_info, loader_info, verification_info, system_program_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_exact_privileges(payer, true, true, false)?;
    for account in [
        config_info,
        gate_info,
        proposal_info,
        capacity_info,
        deployment_info,
        observation_info,
        target_programdata,
        authority_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(target_program, false, false, true)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(verification_info, true, false, false)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&[
        payer,
        config_info,
        gate_info,
        proposal_info,
        capacity_info,
        deployment_info,
        observation_info,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        verification_info,
        system_program_info,
    ])?;
    require_program_ids(loader_info, Some(system_program_info), None, None, None)?;
    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    validate_proposal_guard(
        &instruction.manifest.expected,
        &proposal,
        &config,
        &gate,
        proposal_info.key,
    )?;
    if proposal.state != ProposalStateV2::UpgradeExecuted {
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
    require_deployment_guard(&instruction.manifest.expected, &proposal, &deployment)?;
    let purpose = if proposal.proposal_class == ProposalClassV1::EmergencyRollback {
        ProgramDataObservationPurposeV1::Rollback
    } else {
        ProgramDataObservationPurposeV1::PostUpgrade
    };
    let observation = load_fresh_observation(
        program_id,
        observation_info,
        config_info,
        capacity_info,
        proposal_info,
        target_program,
        target_programdata,
        loader_info,
        &config,
        &capacity,
        &gate,
        purpose,
        proposal.artifact_length,
        &proposal.artifact_sha256,
        &proposal.artifact_chunk_merkle_root,
        proposal.minimum_required_capacity,
    )?;
    require_observation_guard(
        &observation,
        &instruction.manifest.expected_observation_digest,
        instruction.manifest.expected_observation_generation,
        &instruction.manifest.expected_observation_root,
        instruction.manifest.expected_observation_finalized_slot,
        observation.actual_capacity,
    )?;
    validate_target_keys(&config, target_program, target_programdata, authority_info)?;
    let slot = Clock::get()?.slot;
    if slot == 0
        || slot < observation.finalized_slot
        || slot < proposal.upgrade_executed_slot
        || slot > instruction.manifest.plan_valid_until_slot
        || observation.deployed_slot != proposal.upgrade_executed_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let (expected_verification, verification_bump) =
        derive_programdata_check_pda(program_id, proposal_info.key);
    if expected_verification != *verification_info.key
        || proposal.programdata_verification != *verification_info.key
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    let is_new =
        verification_info.owner == &system_program::ID && verification_info.data_len() == 0;
    if is_new {
        if instruction.manifest.verification_generation != 1
            || instruction.manifest.previous_verification_digest != [0; 32]
        {
            return Err(GovernanceError::InvalidRelease1Account.into());
        }
    } else {
        let prior = load_fixed_controller_account::<ProgramDataVerificationV2>(
            program_id,
            verification_info,
            ProgramDataVerificationV2::LEN,
        )?;
        validate_programdata_verification_digest_v2(&prior)?;
        if prior.status != ProgramDataVerificationStatusV2::ObservationBound
            || instruction.manifest.verification_generation
                != prior
                    .verification_generation
                    .checked_add(1)
                    .ok_or(GovernanceError::ArithmeticOverflow)?
            || instruction.manifest.previous_verification_digest != prior.verification_digest
        {
            return Err(GovernanceError::InvalidStateTransition.into());
        }
    }
    let mut candidate = ProgramDataVerificationV2 {
        discriminator: PROGRAMDATA_VERIFICATION_V2_DISCRIMINATOR,
        account_version: CAPACITY_SAFE_ACCOUNT_VERSION_V2,
        bump: verification_bump,
        initialized: true,
        status: ProgramDataVerificationStatusV2::ObservationBound,
        controller_config: *config_info.key,
        proposal: *proposal_info.key,
        proposal_digest: proposal.proposal_digest,
        protocol_gate: *gate_info.key,
        freeze_gate_epoch: gate.epoch,
        target_nonce: config.target_nonce,
        capacity_policy: *capacity_info.key,
        capacity_policy_digest: capacity.policy_digest,
        current_deployment_state: *deployment_info.key,
        current_deployment_digest: deployment.deployment_digest,
        current_deployment_generation: deployment.deployment_generation,
        target_program: *target_program.key,
        target_programdata: *target_programdata.key,
        upgradeable_loader: *loader_info.key,
        controller_authority: *authority_info.key,
        artifact_length: proposal.artifact_length,
        artifact_sha256: proposal.artifact_sha256,
        artifact_merkle_root: proposal.artifact_chunk_merkle_root,
        artifact_scheme_id: proposal.artifact_scheme_id,
        minimum_required_capacity: proposal.minimum_required_capacity,
        maximum_supported_raw_programdata_length: capacity.maximum_raw_programdata_length,
        observation_scheme_id: capacity.observation_scheme_id,
        programdata_observation: *observation_info.key,
        observation_purpose: observation.purpose,
        observation_generation: observation.generation,
        observation_subject_digest: observation.subject_digest,
        observation_root: observation.final_raw_merkle_root,
        observation_digest: observation.observation_digest,
        observation_finalized_slot: observation.finalized_slot,
        observed_deployed_slot: observation.deployed_slot,
        observed_raw_data_length: observation.raw_data_length,
        actual_capacity: observation.actual_capacity,
        observed_authority: observation.upgrade_authority,
        zero_tail_verified: false,
        verification_generation: instruction.manifest.verification_generation,
        previous_verification_digest: instruction.manifest.previous_verification_digest,
        verification_digest_domain_id: PROGRAMDATA_VERIFICATION_V2_DIGEST_DOMAIN_ID,
        verification_digest: [0; 32],
        bound_slot: slot,
        finalized_slot: 0,
        reserved: [0; PROGRAMDATA_VERIFICATION_V2_RESERVED_LEN],
    };
    candidate.verification_digest = compute_programdata_verification_digest_v2(&candidate)?;
    validate_programdata_verification_digest_v2(&candidate)?;
    let bytes = encode_fixed_account(&candidate, ProgramDataVerificationV2::LEN)?;
    if is_new {
        let bump_seed = [verification_bump];
        let signer_seeds: &[&[u8]] = &[
            UPGRADE_SEED_DOMAIN_V1,
            PROGRAMDATA_CHECK_SEED,
            proposal_info.key.as_ref(),
            &bump_seed,
        ];
        create_fixed_pda_account(
            program_id,
            payer,
            verification_info,
            system_program_info,
            &Rent::get()?,
            ProgramDataVerificationV2::LEN,
            signer_seeds,
        )?;
    } else if verification_info.owner != program_id
        || verification_info.data_len() != ProgramDataVerificationV2::LEN
    {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    commit_one_fixed_account(
        program_id,
        verification_info,
        &bytes,
        ProgramDataVerificationV2::LEN,
    )
}

/// Finalizes exact deployed bytes and advances the frozen proposal. The
/// canonical deployment remains unchanged until the separate governed
/// `ExecuteUnfreezeV2` transaction activates the verified artifact.
pub fn process_finalize_programdata_verification_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: FinalizeProgramDataVerificationV2,
) -> ProgramResult {
    let [config_info, gate_info, proposal_info, capacity_info, deployment_info, observation_info, target_program, target_programdata, authority_info, loader_info, verification_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    for account in [
        config_info,
        gate_info,
        capacity_info,
        observation_info,
        target_programdata,
        authority_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(deployment_info, false, false, false)?;
    validate_exact_privileges(target_program, false, false, true)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(verification_info, true, false, false)?;
    require_distinct_accounts(&[
        config_info,
        gate_info,
        proposal_info,
        capacity_info,
        deployment_info,
        observation_info,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        verification_info,
    ])?;
    require_program_ids(loader_info, None, None, None, None)?;
    instruction.expected.validate()?;
    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    if proposal.state != ProposalStateV2::UpgradeExecuted
        || gate.status != GateStatusV1::FrozenForUpgrade
        || gate.active_proposal != *proposal_info.key
        || gate.epoch != proposal.freeze_gate_epoch
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
    let bound = load_programdata_verification(
        program_id,
        verification_info,
        proposal_info,
        config_info,
        &proposal,
    )?;
    if bound.status != ProgramDataVerificationStatusV2::ObservationBound
        || instruction.expected.expected_status != ProgramDataVerificationStatusV2::ObservationBound
        || bound.proposal_digest != instruction.expected.expected_proposal_digest
        || bound.verification_digest != instruction.expected.expected_verification_digest
        || bound.verification_generation != instruction.expected.expected_verification_generation
        || gate.epoch != instruction.expected.expected_gate_epoch
        || config.target_nonce != instruction.expected.expected_target_nonce
        || capacity.policy_digest != instruction.expected.expected_capacity_policy_digest
        || deployment.deployment_digest != instruction.expected.expected_current_deployment_digest
        || deployment.deployment_generation
            != instruction.expected.expected_current_deployment_generation
        || bound.protocol_gate != *gate_info.key
        || bound.freeze_gate_epoch != gate.epoch
        || bound.target_nonce != config.target_nonce
        || bound.capacity_policy != *capacity_info.key
        || bound.capacity_policy_digest != capacity.policy_digest
        || bound.current_deployment_state != *deployment_info.key
        || bound.current_deployment_digest != deployment.deployment_digest
        || bound.current_deployment_generation != deployment.deployment_generation
        || bound.target_program != config.target_program
        || bound.target_programdata != config.target_programdata
        || bound.upgradeable_loader != config.upgradeable_loader
        || bound.controller_authority != config.authority_pda
        || bound.maximum_supported_raw_programdata_length != capacity.maximum_raw_programdata_length
        || bound.observation_scheme_id != capacity.observation_scheme_id
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let purpose = if proposal.proposal_class == ProposalClassV1::EmergencyRollback {
        ProgramDataObservationPurposeV1::Rollback
    } else {
        ProgramDataObservationPurposeV1::PostUpgrade
    };
    let observation = load_fresh_observation(
        program_id,
        observation_info,
        config_info,
        capacity_info,
        proposal_info,
        target_program,
        target_programdata,
        loader_info,
        &config,
        &capacity,
        &gate,
        purpose,
        proposal.artifact_length,
        &proposal.artifact_sha256,
        &proposal.artifact_chunk_merkle_root,
        proposal.minimum_required_capacity,
    )?;
    validate_target_keys(&config, target_program, target_programdata, authority_info)?;
    let slot = Clock::get()?.slot;
    if slot == 0 || slot < observation.finalized_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    if observation.observation_digest != instruction.expected.expected_observation_digest
        || observation.generation != instruction.expected.expected_observation_generation
        || observation.actual_capacity != instruction.expected.expected_actual_capacity
        || *authority_info.key != instruction.expected.expected_authority
        || bound.programdata_observation != *observation_info.key
        || bound.observation_purpose != observation.purpose
        || bound.observation_generation != observation.generation
        || bound.observation_subject_digest != observation.subject_digest
        || bound.observation_root != observation.final_raw_merkle_root
        || bound.observation_digest != observation.observation_digest
        || bound.observation_finalized_slot != observation.finalized_slot
        || bound.observed_deployed_slot != observation.deployed_slot
        || bound.observed_raw_data_length != observation.raw_data_length
        || bound.actual_capacity != observation.actual_capacity
        || bound.observed_authority != observation.upgrade_authority
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    let mut expected_finalized = (*bound).clone();
    expected_finalized.status = ProgramDataVerificationStatusV2::Verified;
    expected_finalized.zero_tail_verified = true;
    expected_finalized.finalized_slot = slot;
    expected_finalized.verification_digest = [0; 32];
    expected_finalized.verification_digest =
        compute_programdata_verification_digest_v2(&expected_finalized)?;
    validate_programdata_verification_digest_v2(&expected_finalized)?;
    let mut next_proposal = (*proposal).clone();
    next_proposal.state = ProposalStateV2::ProgramDataVerified;
    next_proposal.programdata_verified_slot = slot;
    validate_upgrade_proposal_digest_v3(&next_proposal)?;
    let proposal_bytes = encode_fixed_account(&next_proposal, UpgradeProposalV3::LEN)?;
    let verification_bytes =
        encode_fixed_account(&expected_finalized, ProgramDataVerificationV2::LEN)?;
    // ProgramData verification proves the frozen candidate, but does not make
    // it the canonical active deployment. CurrentDeploymentStateV1 advances
    // only in the later, separately governed ExecuteUnfreezeV2 transaction.
    commit_two_fixed_accounts(
        program_id,
        proposal_info,
        &proposal_bytes,
        UpgradeProposalV3::LEN,
        verification_info,
        &verification_bytes,
        ProgramDataVerificationV2::LEN,
    )
}

pub(super) fn load_programdata_verification(
    program_id: &Pubkey,
    verification_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    proposal: &UpgradeProposalV3,
) -> Result<Box<ProgramDataVerificationV2>, ProgramError> {
    let verification = load_fixed_controller_account::<ProgramDataVerificationV2>(
        program_id,
        verification_info,
        ProgramDataVerificationV2::LEN,
    )?;
    validate_programdata_verification_digest_v2(&verification)?;
    if derive_programdata_check_pda(program_id, proposal_info.key)
        != (*verification_info.key, verification.bump)
        || proposal.programdata_verification != *verification_info.key
        || verification.controller_config != *config_info.key
        || verification.proposal != *proposal_info.key
        || verification.proposal_digest != proposal.proposal_digest
        || verification.protocol_gate != proposal.protocol_gate
        || verification.freeze_gate_epoch != proposal.freeze_gate_epoch
        || verification.target_nonce
            != proposal
                .target_nonce
                .checked_add(1)
                .ok_or(GovernanceError::ArithmeticOverflow)?
        || verification.target_program != proposal.target_program
        || verification.target_programdata != proposal.target_programdata
        || verification.upgradeable_loader != proposal.upgradeable_loader
        || verification.controller_authority != proposal.authority_pda
        || verification.artifact_length != proposal.artifact_length
        || verification.artifact_sha256 != proposal.artifact_sha256
        || verification.artifact_merkle_root != proposal.artifact_chunk_merkle_root
        || verification.artifact_scheme_id != proposal.artifact_scheme_id
        || verification.minimum_required_capacity != proposal.minimum_required_capacity
        || verification.observed_deployed_slot != proposal.upgrade_executed_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(verification)
}
