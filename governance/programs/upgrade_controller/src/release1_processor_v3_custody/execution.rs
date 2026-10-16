use super::*;

/// Performs one exact `ExtendProgramChecked` CPI.  The accepted prestate and a
/// live finalized observation establish the old byte graph; the controller
/// proves the appended range is all zero and advances the deployment evidence
/// generation while leaving the gate frozen.
pub fn process_extend_target_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExtendTargetV2,
) -> ProgramResult {
    let [payer, config_info, gate_info, proposal_info, capacity_info, deployment_info, observation_info, prestate_info, target_programdata, target_program, authority_info, loader_info, system_program_info, rent_info, instructions_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_exact_privileges(payer, true, true, false)?;
    for account in [
        config_info,
        gate_info,
        capacity_info,
        observation_info,
        prestate_info,
        rent_info,
        instructions_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(deployment_info, false, false, false)?;
    validate_exact_privileges(target_programdata, true, false, false)?;
    validate_exact_privileges(target_program, true, false, true)?;
    validate_exact_privileges(authority_info, true, false, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&[
        payer,
        config_info,
        gate_info,
        proposal_info,
        capacity_info,
        deployment_info,
        observation_info,
        prestate_info,
        target_programdata,
        target_program,
        authority_info,
        loader_info,
        system_program_info,
        rent_info,
        instructions_info,
    ])?;
    require_program_ids(
        loader_info,
        Some(system_program_info),
        Some(rent_info),
        None,
        Some(instructions_info),
    )?;
    let _rent = Rent::from_account_info(rent_info)?;
    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    validate_proposal_guard(
        &instruction.expected,
        &proposal,
        &config,
        &gate,
        proposal_info.key,
    )?;
    if proposal.state != ProposalStateV2::Frozen {
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
    require_deployment_guard(&instruction.expected, &proposal, &deployment)?;
    let prestate = load_accepted_prestate_at_epoch(
        program_id,
        prestate_info,
        proposal_info,
        config_info,
        capacity_info,
        deployment_info,
        observation_info,
        &config,
        &capacity,
        &proposal,
        gate.epoch,
        &deployment,
        &instruction.expected_prestate_checkpoint_digest,
        instruction.expected_prestate_checkpoint_generation,
    )?;
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
        ProgramDataObservationPurposeV1::ProposalPrestate,
        deployment.artifact_length,
        &deployment.artifact_sha256,
        &deployment.artifact_merkle_root,
        deployment.artifact_length,
    )?;
    require_observation_guard(
        &observation,
        &instruction.expected_observation_digest,
        instruction.expected_observation_generation,
        &instruction.expected_observation_root,
        instruction.expected_observation_finalized_slot,
        instruction.expected_current_capacity,
    )?;
    if prestate.programdata_observation != *observation_info.key
        || prestate.observation_digest != observation.observation_digest
        || prestate.actual_capacity != observation.actual_capacity
        || prestate.target_programdata_slot != observation.deployed_slot
        || instruction.expected_current_capacity != deployment.actual_programdata_capacity
        || instruction.expected_current_capacity != observation.actual_capacity
        || instruction.expected_post_capacity != proposal.minimum_required_capacity
        || instruction.expected_extension_delta
            != proposal
                .minimum_required_capacity
                .checked_sub(observation.actual_capacity)
                .ok_or(GovernanceError::InvalidCapacityPlan)?
        || instruction.expected_post_capacity > capacity.maximum_payload_capacity
        || instruction.expected_post_capacity <= instruction.expected_current_capacity
    {
        return Err(GovernanceError::InvalidCapacityPlan.into());
    }
    validate_target_keys(&config, target_program, target_programdata, authority_info)?;
    let slot = current_frozen_slot(&proposal)?;
    if slot >= proposal.expiry_slot || slot < prestate.finalized_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let before = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &config.upgradeable_loader,
    )?;
    if before.deployed_slot != observation.deployed_slot
        || before.upgrade_authority != Some(config.authority_pda)
        || u64::try_from(before.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != instruction.expected_current_capacity
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    validate_canonical_envelope(
        program_id,
        accounts,
        instructions_info,
        &instruction.pack()?,
        &instruction.envelope,
    )?;
    let delta = u32::try_from(instruction.expected_extension_delta)
        .map_err(|_| GovernanceError::InvalidCapacityPlan)?;
    let extend = extend_program_checked(
        target_program.key,
        authority_info.key,
        Some(payer.key),
        delta,
    );
    validate_extend_cpi_shape(
        &extend,
        target_programdata.key,
        target_program.key,
        authority_info.key,
        system_program_info.key,
        payer.key,
    )?;

    let mut next_proposal = (*proposal).clone();
    next_proposal.state = ProposalStateV2::Extended;
    next_proposal.extension_executed_slot = slot;
    validate_upgrade_proposal_digest_v3(&next_proposal)?;
    let mut next_deployment = (*deployment).clone();
    next_deployment.actual_programdata_capacity = instruction.expected_post_capacity;
    next_deployment.deployed_slot = slot;
    next_deployment.deployment_generation = next_deployment
        .deployment_generation
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    next_deployment.last_updated_slot = slot;
    next_deployment.deployment_digest = instruction.expected_next_deployment_digest;
    if next_deployment.deployment_generation != instruction.expected_next_deployment_generation
        || compute_current_deployment_digest_v1(&next_deployment)?
            != instruction.expected_next_deployment_digest
    {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    validate_current_deployment_digest_v1(&next_deployment)?;
    let proposal_bytes = encode_fixed_account(&next_proposal, UpgradeProposalV3::LEN)?;
    let deployment_bytes = encode_fixed_account(&next_deployment, CurrentDeploymentStateV1::LEN)?;

    let authority_bump = derive_authority_pda(program_id, &config.target_program).1;
    let bump_seed = [authority_bump];
    let signer_seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        AUTHORITY_SEED,
        config.target_program.as_ref(),
        &bump_seed,
    ];
    invoke_signed(
        &extend,
        &[
            target_programdata.clone(),
            target_program.clone(),
            authority_info.clone(),
            system_program_info.clone(),
            payer.clone(),
            loader_info.clone(),
        ],
        &[signer_seeds],
    )?;
    let after = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &config.upgradeable_loader,
    )?;
    if after.deployed_slot != slot
        || after.upgrade_authority != Some(config.authority_pda)
        || u64::try_from(after.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != instruction.expected_post_capacity
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_zero_appended_extension(
        target_programdata,
        after.payload_offset,
        instruction.expected_current_capacity,
        instruction.expected_extension_delta,
    )?;
    commit_two_fixed_accounts(
        program_id,
        proposal_info,
        &proposal_bytes,
        UpgradeProposalV3::LEN,
        deployment_info,
        &deployment_bytes,
        CurrentDeploymentStateV1::LEN,
    )
}

/// Executes one exact Loader-v3 Upgrade CPI after re-observing the current
/// ProgramData graph.  Success consumes the locked buffer and advances only to
/// `UpgradeExecuted`; the gate remains frozen and no deployment is trusted yet.
pub fn process_execute_upgrade_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExecuteUpgradeV2,
) -> ProgramResult {
    let [config_info, policy_info, gate_info, proposal_info, counterpart_info, counterpart_verification_info, capacity_info, deployment_info, prestate_observation_info, prestate_info, current_observation_info, buffer_verification_info, target_programdata, target_program, buffer_info, spill_info, rent_info, clock_info, authority_info, loader_info, instructions_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    for account in [
        config_info,
        policy_info,
        gate_info,
        counterpart_info,
        counterpart_verification_info,
        capacity_info,
        deployment_info,
        prestate_observation_info,
        prestate_info,
        current_observation_info,
        rent_info,
        clock_info,
        authority_info,
        instructions_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(buffer_verification_info, true, false, false)?;
    validate_exact_privileges(target_programdata, true, false, false)?;
    validate_exact_privileges(target_program, true, false, true)?;
    validate_exact_privileges(buffer_info, true, false, false)?;
    validate_exact_privileges(spill_info, true, false, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    require_distinct_accounts(&[
        config_info,
        policy_info,
        gate_info,
        proposal_info,
        counterpart_info,
        counterpart_verification_info,
        capacity_info,
        deployment_info,
        prestate_observation_info,
        prestate_info,
        current_observation_info,
        buffer_verification_info,
        target_programdata,
        target_program,
        buffer_info,
        spill_info,
        rent_info,
        clock_info,
        authority_info,
        loader_info,
        instructions_info,
    ])?;
    require_program_ids(
        loader_info,
        None,
        Some(rent_info),
        Some(clock_info),
        Some(instructions_info),
    )?;
    let _rent = Rent::from_account_info(rent_info)?;
    let clock = Clock::from_account_info(clock_info)?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    validate_proposal_guard(
        &instruction.expected,
        &proposal,
        &config,
        &gate,
        proposal_info.key,
    )?;
    if policy.version != proposal.policy_version
        || policy.policy_hash != proposal.policy_hash
        || clock.slot == 0
        || clock.slot < proposal.not_before_slot
        || clock.slot >= proposal.expiry_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
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
    require_deployment_guard(&instruction.expected, &proposal, &deployment)?;
    let rollback_primary = if proposal.proposal_class == ProposalClassV1::EmergencyRollback {
        let primary = load_proposal(program_id, counterpart_info, config_info, &config)?;
        if !proposal.primary_proposal.present
            || proposal.primary_proposal.value != *counterpart_info.key
            || primary.proposal_class == ProposalClassV1::EmergencyRollback
        {
            return Err(GovernanceError::InvalidProposalCommitment.into());
        }
        Some(primary)
    } else {
        None
    };
    // A rollback reuses the primary proposal's accepted protected prestate.
    // That checkpoint predates the failed candidate, is bound to the prior
    // frozen epoch, and remains the only exact known-good state anchor while
    // the gate stays continuously frozen.
    let (prestate_subject_info, prestate_proposal, prestate_epoch) =
        if let Some(primary) = rollback_primary.as_deref() {
            (counterpart_info, primary, primary.freeze_gate_epoch)
        } else {
            (proposal_info, proposal.as_ref(), gate.epoch)
        };
    let prestate = load_accepted_prestate_at_epoch(
        program_id,
        prestate_info,
        prestate_subject_info,
        config_info,
        capacity_info,
        deployment_info,
        prestate_observation_info,
        &config,
        &capacity,
        prestate_proposal,
        prestate_epoch,
        &deployment,
        &instruction.expected_prestate_checkpoint_digest,
        instruction.expected_prestate_checkpoint_generation,
    )?;
    let (
        expected_prestate_length,
        expected_prestate_sha,
        expected_prestate_root,
        expected_prestate_minimum,
        observed_deployed_slot,
        observed_actual_capacity,
    ) = if let Some(primary) = rollback_primary.as_deref() {
        let failure = load_and_revalidate_rollback_failure(
            program_id,
            current_observation_info,
            config_info,
            gate_info,
            counterpart_info,
            capacity_info,
            deployment_info,
            proposal_info,
            target_program,
            target_programdata,
            &config,
            &capacity,
            &gate,
            &deployment,
            primary,
            &proposal,
        )?;
        require_rollback_failure_guard(&failure, &instruction)?;
        (
            proposal.artifact_length,
            proposal.artifact_sha256,
            proposal.artifact_chunk_merkle_root,
            proposal.minimum_required_capacity,
            failure.actual_programdata_slot,
            failure.actual_capacity,
        )
    } else {
        let observation = load_fresh_observation(
            program_id,
            current_observation_info,
            config_info,
            capacity_info,
            proposal_info,
            target_program,
            target_programdata,
            loader_info,
            &config,
            &capacity,
            &gate,
            ProgramDataObservationPurposeV1::ProposalPrestate,
            deployment.artifact_length,
            &deployment.artifact_sha256,
            &deployment.artifact_merkle_root,
            deployment.artifact_length,
        )?;
        require_observation_guard(
            &observation,
            &instruction.expected_observation_digest,
            instruction.expected_observation_generation,
            &instruction.expected_observation_root,
            instruction.expected_observation_finalized_slot,
            instruction.expected_actual_capacity,
        )?;
        (
            deployment.artifact_length,
            deployment.artifact_sha256,
            deployment.artifact_merkle_root,
            deployment.artifact_length,
            observation.deployed_slot,
            observation.actual_capacity,
        )
    };
    if observed_actual_capacity != deployment.actual_programdata_capacity
        || !prestate.accepted
        || prestate.programdata_observation != *prestate_observation_info.key
        || prestate.artifact_length != expected_prestate_length
        || prestate.artifact_sha256 != expected_prestate_sha
        || prestate.artifact_merkle_root != expected_prestate_root
        || prestate.minimum_required_capacity != expected_prestate_minimum
        || !prestate.observed_authority.present
        || prestate.observed_authority.value != config.authority_pda
        || *spill_info.key != config.canonical_spill_treasury
        || *spill_info.key != proposal.canonical_spill_treasury
        || *current_observation_info.key == *prestate_observation_info.key
        || *buffer_info.key != proposal.buffer_pubkey
        || *buffer_verification_info.key != proposal.buffer_verification
        || proposal.minimum_required_capacity > observed_actual_capacity
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    if !matches!(
        proposal.state,
        ProposalStateV2::Frozen | ProposalStateV2::Extended
    ) || (proposal.state == ProposalStateV2::Frozen && proposal.extension_executed_slot != 0)
        || (proposal.state == ProposalStateV2::Extended
            && (proposal.extension_executed_slot == 0
                || proposal.extension_executed_slot >= clock.slot))
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    validate_target_keys(&config, target_program, target_programdata, authority_info)?;
    let before = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &config.upgradeable_loader,
    )?;
    if before.deployed_slot != observed_deployed_slot
        || before.upgrade_authority != Some(config.authority_pda)
        || u64::try_from(before.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != observed_actual_capacity
        || clock.slot <= before.deployed_slot
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    let mut buffer_verification = load_buffer_verification(
        program_id,
        buffer_verification_info,
        proposal_info,
        buffer_info,
        authority_info,
        config_info,
        &config,
        &proposal,
    )?;
    if buffer_verification.status != instruction.expected_buffer_verification_status
        || buffer_verification.status != BufferVerificationStatusV1::Verified
        || buffer_verification.verified_chunk_count != instruction.expected_verified_chunk_count
        || buffer_verification.verified_chunk_count != proposal.artifact_chunk_count
        || buffer_verification.sealed_buffer_header_hash
            != instruction.expected_sealed_buffer_header_hash
        || buffer_verification.finalized_slot == 0
        || buffer_verification.finalized_slot > clock.slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_sealed_buffer(
        buffer_info,
        authority_info.key,
        &config,
        &buffer_verification,
    )?;
    validate_counterpart(
        program_id,
        counterpart_info,
        counterpart_verification_info,
        config_info,
        authority_info,
        &config,
        &proposal,
        proposal_info.key,
        &instruction,
        clock.slot,
    )?;
    validate_canonical_envelope(
        program_id,
        accounts,
        instructions_info,
        &instruction.pack()?,
        &instruction.envelope,
    )?;
    let mut next_proposal = (*proposal).clone();
    next_proposal.state = ProposalStateV2::UpgradeExecuted;
    next_proposal.upgrade_executed_slot = clock.slot;
    validate_upgrade_proposal_digest_v3(&next_proposal)?;
    buffer_verification.status = BufferVerificationStatusV1::ConsumedByUpgrade;
    buffer_verification.terminal_slot = clock.slot;
    buffer_verification.validate_schema()?;
    let proposal_bytes = encode_fixed_account(&next_proposal, UpgradeProposalV3::LEN)?;
    let buffer_bytes = encode_fixed_account(&*buffer_verification, BufferVerificationV1::LEN)?;
    let upgrade_instruction = upgrade(
        target_program.key,
        buffer_info.key,
        authority_info.key,
        spill_info.key,
    );
    validate_upgrade_cpi_shape(
        &upgrade_instruction,
        target_programdata.key,
        target_program.key,
        buffer_info.key,
        spill_info.key,
        rent_info.key,
        clock_info.key,
        authority_info.key,
    )?;
    let authority_bump = derive_authority_pda(program_id, &config.target_program).1;
    let bump_seed = [authority_bump];
    let signer_seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        AUTHORITY_SEED,
        config.target_program.as_ref(),
        &bump_seed,
    ];
    invoke_signed(
        &upgrade_instruction,
        &[
            target_programdata.clone(),
            target_program.clone(),
            buffer_info.clone(),
            spill_info.clone(),
            rent_info.clone(),
            clock_info.clone(),
            authority_info.clone(),
            loader_info.clone(),
        ],
        &[signer_seeds],
    )?;
    let after = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &config.upgradeable_loader,
    )?;
    if after.deployed_slot != clock.slot
        || after.upgrade_authority != Some(config.authority_pda)
        || u64::try_from(after.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != observed_actual_capacity
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let consumed = buffer_info.try_borrow_data()?;
    if buffer_info.lamports() != 0
        || consumed.len() != LOADER_BUFFER_METADATA_LEN
        || parse_upgradeable_buffer(&consumed)?.authority != Some(config.authority_pda)
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    drop(consumed);
    commit_two_fixed_accounts(
        program_id,
        proposal_info,
        &proposal_bytes,
        UpgradeProposalV3::LEN,
        buffer_verification_info,
        &buffer_bytes,
        BufferVerificationV1::LEN,
    )
}
