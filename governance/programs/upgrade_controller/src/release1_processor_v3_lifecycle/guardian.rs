use super::*;

pub fn process_guardian_freeze_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: GuardianFreezeV2,
) -> ProgramResult {
    exact_account_count(accounts, 12)?;
    all_distinct(accounts)?;
    let [payer, config_info, gate_info, capacity_info, deployment_info, target_program, target_programdata, loader, authority, guardian, observation_info, system_program_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    signer_writable(payer)?;
    for info in [
        config_info,
        capacity_info,
        deployment_info,
        target_programdata,
        authority,
    ] {
        readonly(info)?;
    }
    writable(gate_info)?;
    executable_readonly(target_program)?;
    executable_readonly(loader)?;
    signer_readonly(guardian)?;
    writable(observation_info)?;
    system_program_account(system_program_info)?;

    let slot = current_slot()?;
    let mut context = load_lifecycle_context(
        program_id,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
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
        || context.gate.status != GateStatusV1::Active
        || context.gate.active_proposal != Pubkey::default()
        || context.gate.epoch != manifest.expected_gate_epoch
        || context.config.target_nonce != manifest.expected_target_nonce
        || manifest.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        || *guardian.key != context.config.guardian
        || *loader.key != context.config.upgradeable_loader
        || *authority.key != context.config.authority_pda
    {
        return Err(GovernanceError::InvalidGateState.into());
    }
    let runtime = read_runtime_programdata_header(
        target_program,
        target_programdata,
        &context.config,
        &context.capacity,
    )?;
    require_runtime_matches_trusted_deployment(&runtime, &context.deployment, &context.config)?;
    if runtime.capacity != context.deployment.actual_programdata_capacity {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    let next_epoch = checked_nonterminal_increment(context.gate.epoch)?;
    let (expected_observation, observation_bump) = derive_emergency_freeze_observation_pda(
        program_id,
        &context.config.target_program,
        next_epoch,
    );
    if *observation_info.key != expected_observation {
        return Err(GovernanceError::InvalidPda.into());
    }
    let mut observation = EmergencyFreezeObservationV2 {
        discriminator: EMERGENCY_FREEZE_OBSERVATION_V2_DISCRIMINATOR,
        account_version: CAPACITY_SAFE_ACCOUNT_VERSION_V2,
        bump: observation_bump,
        initialized: true,
        finalized: true,
        controller_program: *program_id,
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
        trusted_artifact_length: context.deployment.artifact_length,
        trusted_artifact_sha256: context.deployment.artifact_sha256,
        trusted_artifact_merkle_root: context.deployment.artifact_merkle_root,
        trusted_artifact_scheme_id: context.deployment.artifact_scheme_id,
        minimum_required_capacity: context.deployment.artifact_length,
        frozen_epoch: next_epoch,
        freeze_slot: slot,
        freeze_reason_code: manifest.freeze_reason_code,
        target_nonce: context.config.target_nonce,
        actual_program_owner: *target_program.owner,
        actual_program_executable: target_program.executable,
        actual_program_data_length: u64::try_from(target_program.data_len())
            .map_err(|_| GovernanceError::ArithmeticOverflow)?,
        program_header_present: true,
        actual_linked_programdata: context.config.target_programdata,
        actual_programdata_owner: *target_programdata.owner,
        actual_programdata_executable: target_programdata.executable,
        actual_programdata_data_length: runtime.raw_data_length,
        programdata_header_present: true,
        deployed_programdata_slot: runtime.deployed_slot,
        actual_capacity: runtime.capacity,
        observed_authority: OptionalPubkeyV1::some(context.config.authority_pda)?,
        observation_digest_domain_id: EMERGENCY_FREEZE_OBSERVATION_V2_DIGEST_DOMAIN_ID,
        observation_digest: [0; 32],
        finalized_slot: slot,
        reserved: [0; EMERGENCY_FREEZE_OBSERVATION_V2_RESERVED_LEN],
    };
    observation.observation_digest = compute_emergency_freeze_observation_digest_v2(&observation)?;
    validate_emergency_freeze_observation_digest_v2(&observation)?;
    context.gate.status = GateStatusV1::EmergencyFrozen;
    context.gate.epoch = next_epoch;
    context.gate.active_proposal = Pubkey::default();
    context.gate.freeze_slot = slot;
    context.gate.freeze_reason_code = manifest.freeze_reason_code;
    context.gate.validate_static()?;
    let observation_bytes = encode_fixed_account(&observation, EmergencyFreezeObservationV2::LEN)?;
    let gate_bytes = encode_fixed_account(&*context.gate, ProtocolGateV1::LEN)?;
    let epoch = next_epoch.to_le_bytes();
    let bump = [observation_bump];
    let seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        EMERGENCY_FREEZE_OBSERVATION_SEED,
        context.config.target_program.as_ref(),
        &epoch,
        &bump,
    ];
    create_fixed_pda_account(
        program_id,
        payer,
        observation_info,
        system_program_info,
        &Rent::get()?,
        EmergencyFreezeObservationV2::LEN,
        seeds,
    )?;
    observation_info
        .try_borrow_mut_data()?
        .copy_from_slice(&observation_bytes);
    gate_info
        .try_borrow_mut_data()?
        .copy_from_slice(&gate_bytes);
    Ok(())
}
