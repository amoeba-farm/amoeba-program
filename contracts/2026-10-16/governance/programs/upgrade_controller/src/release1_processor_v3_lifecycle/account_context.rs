use super::*;

pub(super) fn current_slot() -> Result<u64, ProgramError> {
    let slot = Clock::get()?.slot;
    if slot == 0 {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(slot)
}

pub(super) fn checked_nonterminal_increment(value: u64) -> GovernanceResult<u64> {
    let next = value
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if next == u64::MAX {
        return Err(GovernanceError::ArithmeticOverflow);
    }
    Ok(next)
}

pub(super) fn exact_account_count(accounts: &[AccountInfo<'_>], expected: usize) -> ProgramResult {
    if accounts.len() != expected {
        return Err(GovernanceError::InvalidAccountCount.into());
    }
    Ok(())
}

pub(super) fn all_distinct(accounts: &[AccountInfo<'_>]) -> ProgramResult {
    let refs = accounts.iter().collect::<Vec<_>>();
    require_distinct_accounts(&refs)
}

pub(super) fn readonly(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, false, false, false)
}

pub(super) fn writable(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, true, false, false)
}

pub(super) fn signer_readonly(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, false, true, false)
}

pub(super) fn signer_writable(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, true, true, false)
}

pub(super) fn executable_readonly(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, false, false, true)
}

pub(super) fn system_program_account(account: &AccountInfo<'_>) -> ProgramResult {
    executable_readonly(account)?;
    if *account.key != system_program::ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn load_config(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
) -> Result<Box<ControllerConfigV1>, ProgramError> {
    let config = load_fixed_controller_account::<ControllerConfigV1>(
        program_id,
        info,
        ControllerConfigV1::LEN,
    )?;
    config.validate_static()?;
    let expected = derive_controller_config_pda(program_id, &config.target_program);
    if expected != (*info.key, config.bump)
        || config.upgradeable_loader != UPGRADEABLE_LOADER_ID
        || derive_upgradeable_programdata_address(&config.target_program).0
            != config.target_programdata
        || derive_authority_pda(program_id, &config.target_program).0 != config.authority_pda
        || derive_gate_pda(program_id, &config.target_program).0 != config.gate_pda
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    Ok(config)
}

pub(super) fn load_policy(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    slot: u64,
) -> Result<Box<GovernancePolicyV1>, ProgramError> {
    let policy = load_fixed_controller_account::<GovernancePolicyV1>(
        program_id,
        info,
        GovernancePolicyV1::LEN,
    )?;
    validate_policy_against_config(&policy, config)?;
    let expected = derive_policy_pda(
        program_id,
        &config.target_program,
        config.current_policy_version,
    );
    if expected != (*info.key, policy.bump)
        || policy.controller_config != *config_info.key
        || policy.activation_slot > slot
    {
        return Err(GovernanceError::InactivePolicy.into());
    }
    Ok(policy)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_council(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
    expected_version: u64,
    expected_hash: &[u8; 32],
    slot: u64,
    require_current: bool,
) -> Result<Box<GovernanceCouncilSetV1>, ProgramError> {
    let council = load_fixed_controller_account::<GovernanceCouncilSetV1>(
        program_id,
        info,
        GovernanceCouncilSetV1::LEN,
    )?;
    validate_council_set(&council, policy)?;
    validate_council_guardian_separation(&council, &config.guardian)?;
    let expected = derive_council_pda(program_id, &config.target_program, expected_version);
    if expected != (*info.key, council.bump)
        || council.controller_config != *config_info.key
        || council.target_program != config.target_program
        || council.version != expected_version
        || council.set_hash != *expected_hash
        || !council.active_at(slot)
        || (require_current && council.version != config.current_council_version)
    {
        return Err(GovernanceError::StaleCouncilVersion.into());
    }
    Ok(council)
}

pub(super) fn load_current_council(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
    slot: u64,
) -> Result<Box<GovernanceCouncilSetV1>, ProgramError> {
    let council = load_fixed_controller_account::<GovernanceCouncilSetV1>(
        program_id,
        info,
        GovernanceCouncilSetV1::LEN,
    )?;
    load_council(
        program_id,
        info,
        config_info,
        config,
        policy,
        config.current_council_version,
        &council.set_hash,
        slot,
        true,
    )
}

pub(super) fn load_gate(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<ProtocolGateV1>, ProgramError> {
    let gate =
        load_fixed_controller_account::<ProtocolGateV1>(program_id, info, ProtocolGateV1::LEN)?;
    gate.validate_static()?;
    if derive_gate_pda(program_id, &config.target_program) != (*info.key, gate.bump)
        || *info.key != config.gate_pda
        || gate.controller_config != *config_info.key
        || gate.target_program != config.target_program
        || gate.target_programdata != config.target_programdata
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(gate)
}

pub(super) fn load_capacity_policy(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<ProgramDataCapacityPolicyV1>, ProgramError> {
    let capacity = load_fixed_controller_account::<ProgramDataCapacityPolicyV1>(
        program_id,
        info,
        ProgramDataCapacityPolicyV1::LEN,
    )?;
    validate_capacity_policy_digest_v1(&capacity)?;
    if derive_capacity_policy_pda(program_id, &config.target_program) != (*info.key, capacity.bump)
        || capacity.controller_program != *program_id
        || capacity.controller_config != *config_info.key
        || capacity.target_program != config.target_program
        || capacity.target_programdata != config.target_programdata
        || capacity.upgradeable_loader != config.upgradeable_loader
    {
        return Err(GovernanceError::CapacityPolicyMismatch.into());
    }
    Ok(capacity)
}

pub(super) fn load_current_deployment(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    capacity: &ProgramDataCapacityPolicyV1,
) -> Result<Box<CurrentDeploymentStateV1>, ProgramError> {
    let deployment = load_fixed_controller_account::<CurrentDeploymentStateV1>(
        program_id,
        info,
        CurrentDeploymentStateV1::LEN,
    )?;
    validate_current_deployment_digest_v1(&deployment)?;
    if derive_current_deployment_state_pda(program_id, &config.target_program)
        != (*info.key, deployment.bump)
        || deployment.controller_program != *program_id
        || deployment.controller_config != *config_info.key
        || deployment.capacity_policy != *capacity_info.key
        || deployment.capacity_policy_digest != capacity.policy_digest
        || deployment.target_program != config.target_program
        || deployment.target_programdata != config.target_programdata
        || deployment.upgradeable_loader != config.upgradeable_loader
        || deployment.controller_authority != config.authority_pda
        || deployment.installed_authority != config.authority_pda
        || deployment.actual_programdata_capacity > capacity.maximum_payload_capacity
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(deployment)
}

pub(super) fn load_lifecycle_context(
    program_id: &Pubkey,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    deployment_info: &AccountInfo<'_>,
) -> Result<LifecycleContext, ProgramError> {
    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let capacity = load_capacity_policy(program_id, capacity_info, config_info, &config)?;
    let deployment = load_current_deployment(
        program_id,
        deployment_info,
        config_info,
        capacity_info,
        &config,
        &capacity,
    )?;
    Ok(LifecycleContext {
        config,
        gate,
        capacity,
        deployment,
    })
}

pub(super) fn read_runtime_programdata_header(
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    capacity: &ProgramDataCapacityPolicyV1,
) -> Result<RuntimeProgramDataHeaderV2, ProgramError> {
    if *target_program.key != config.target_program
        || *target_programdata.key != config.target_programdata
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let header = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &config.upgradeable_loader,
    )?;
    let raw_data_length = u64::try_from(target_programdata.data_len())
        .map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let runtime = RuntimeProgramDataHeaderV2 {
        deployed_slot: header.deployed_slot,
        raw_data_length,
        capacity: u64::try_from(header.capacity)
            .map_err(|_| GovernanceError::ArithmeticOverflow)?,
        authority: header.upgrade_authority,
    };
    if runtime.deployed_slot == 0
        || runtime.raw_data_length > capacity.maximum_raw_programdata_length
        || runtime.capacity > capacity.maximum_payload_capacity
        || runtime.raw_data_length
            != runtime
                .capacity
                .checked_add(capacity.loader_programdata_metadata_len)
                .ok_or(GovernanceError::ArithmeticOverflow)?
    {
        return Err(GovernanceError::InvalidCapacityPlan.into());
    }
    Ok(runtime)
}

pub(super) fn require_runtime_matches_trusted_deployment(
    runtime: &RuntimeProgramDataHeaderV2,
    deployment: &CurrentDeploymentStateV1,
    config: &ControllerConfigV1,
) -> ProgramResult {
    // A larger capacity with the same deployed slot and controller authority is
    // the only admitted header drift here. Loader-v3 extension is monotonic and
    // zero-fills appended bytes; byte-level consumers still require a fresh
    // finalized ProgramDataObservationV1 before checkpoint or Loader work.
    if runtime.deployed_slot != deployment.deployed_slot
        || runtime.capacity < deployment.actual_programdata_capacity
        || runtime.authority != Some(config.authority_pda)
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(())
}

pub(super) fn require_guard_deployment(
    capacity: &ProgramDataCapacityPolicyV1,
    deployment: &CurrentDeploymentStateV1,
    expected_capacity_digest: &[u8; 32],
    expected_deployment_digest: &[u8; 32],
    expected_deployment_generation: u64,
) -> ProgramResult {
    if capacity.policy_digest != *expected_capacity_digest
        || deployment.deployment_digest != *expected_deployment_digest
        || deployment.deployment_generation != expected_deployment_generation
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn load_proposal(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    deployment_info: &AccountInfo<'_>,
    context: &LifecycleContext,
) -> Result<Box<UpgradeProposalV3>, ProgramError> {
    let proposal = load_upgrade_proposal_v3(program_id, info)?;
    validate_upgrade_proposal_digest_v3(&proposal)?;
    let expected = derive_proposal_pda(
        program_id,
        &context.config.target_program,
        proposal.proposal_id,
    );
    if expected != (*info.key, proposal.bump)
        || proposal.controller_program != *program_id
        || proposal.controller_config != *config_info.key
        || proposal.protocol_gate != *gate_info.key
        || proposal.capacity_policy != *capacity_info.key
        || proposal.capacity_policy_digest != context.capacity.policy_digest
        || proposal.current_deployment_state != *deployment_info.key
        || proposal.cluster_domain != context.config.cluster_domain
        || proposal.target_program != context.config.target_program
        || proposal.target_programdata != context.config.target_programdata
        || proposal.upgradeable_loader != context.config.upgradeable_loader
        || proposal.authority_pda != context.config.authority_pda
        || proposal.canonical_spill_treasury != context.config.canonical_spill_treasury
        || proposal.policy_version != context.config.current_policy_version
        || proposal.maximum_supported_raw_programdata_length
            != context.capacity.maximum_raw_programdata_length
        || proposal.programdata_observation_scheme_id != context.capacity.observation_scheme_id
        || proposal.artifact_scheme_id != context.capacity.artifact_scheme_id
        || proposal.artifact_chunk_size != context.capacity.artifact_chunk_size
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(proposal)
}

pub(super) fn require_proposal_deployment_current(
    proposal: &UpgradeProposalV3,
    deployment: &CurrentDeploymentStateV1,
) -> ProgramResult {
    if proposal.current_deployment_digest != deployment.deployment_digest
        || proposal.current_deployment_generation != deployment.deployment_generation
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(())
}
