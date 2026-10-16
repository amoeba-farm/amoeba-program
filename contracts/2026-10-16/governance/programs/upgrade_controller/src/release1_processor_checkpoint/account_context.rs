use super::*;

pub(super) fn current_slot() -> Result<u64, ProgramError> {
    let slot = Clock::get()?.slot;
    if slot == 0 {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(slot)
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

/// Candidate authorities remain real account inputs so executable accounts are
/// rejected and smart-account/PDA authorities preserve their runtime identity.
/// The only admitted duplicate role is the current-seat creator appearing once
/// in the candidate council. Solana coalesces duplicate-account privileges, so
/// that candidate occurrence must also be a read-only signer. Every other
/// candidate authority is exactly read-only/non-signer and may not alias any
/// other instruction role.
#[allow(clippy::too_many_arguments)]
pub(super) fn validate_candidate_creation_authority_contract<'info>(
    payer: &AccountInfo<'info>,
    creator: &AccountInfo<'info>,
    config_info: &AccountInfo<'info>,
    policy_info: &AccountInfo<'info>,
    current_council_info: &AccountInfo<'info>,
    gate_info: &AccountInfo<'info>,
    candidate_info: &AccountInfo<'info>,
    candidate_authorities: &[AccountInfo<'info>],
    system_program_info: &AccountInfo<'info>,
) -> ProgramResult {
    let fixed_roles = [
        payer,
        creator,
        config_info,
        policy_info,
        current_council_info,
        gate_info,
        candidate_info,
        system_program_info,
    ];
    require_distinct_accounts(&fixed_roles)?;
    let candidate_refs = candidate_authorities.iter().collect::<Vec<_>>();
    require_distinct_accounts(&candidate_refs)?;

    for authority in candidate_authorities {
        let creator_alias = authority.key == creator.key;
        if !creator_alias
            && fixed_roles
                .iter()
                .any(|fixed_role| fixed_role.key == authority.key)
        {
            return Err(GovernanceError::CrossAccountMismatch.into());
        }
        validate_exact_privileges(authority, false, creator_alias, false)?;
    }
    Ok(())
}

pub(super) fn validate_readonly(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, false, false, false)
}

pub(super) fn validate_writable(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, true, false, false)
}

pub(super) fn validate_signer_readonly(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, false, true, false)
}

pub(super) fn validate_signer_writable(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, true, true, false)
}

pub(super) fn validate_system_program(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, false, false, true)?;
    if *account.key != system_program::ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn require_absent_system_account(account: &AccountInfo<'_>) -> ProgramResult {
    if account.owner != &system_program::ID || account.data_len() != 0 || account.executable {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    Ok(())
}

pub(super) fn load_config(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
) -> Result<Box<ControllerConfigV1>, ProgramError> {
    let config = load_fixed_controller_account::<ControllerConfigV1>(
        program_id,
        account,
        ControllerConfigV1::LEN,
    )?;
    config.validate_static()?;
    let (expected, bump) = derive_controller_config_pda(program_id, &config.target_program);
    let (programdata, _) = derive_upgradeable_programdata_address(&config.target_program);
    let (authority, _) = derive_authority_pda(program_id, &config.target_program);
    let (gate, _) = derive_gate_pda(program_id, &config.target_program);
    if *account.key != expected
        || config.bump != bump
        || config.upgradeable_loader != UPGRADEABLE_LOADER_ID
        || config.target_programdata != programdata
        || config.authority_pda != authority
        || config.gate_pda != gate
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    Ok(config)
}

pub(super) fn load_gate(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    config_key: &Pubkey,
    config: &ControllerConfigV1,
) -> Result<Box<ProtocolGateV1>, ProgramError> {
    let gate =
        load_fixed_controller_account::<ProtocolGateV1>(program_id, account, ProtocolGateV1::LEN)?;
    gate.validate_static()?;
    let (expected, bump) = derive_gate_pda(program_id, &config.target_program);
    if *account.key != expected
        || *account.key != config.gate_pda
        || gate.bump != bump
        || gate.controller_config != *config_key
        || gate.target_program != config.target_program
        || gate.target_programdata != config.target_programdata
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(gate)
}

pub(super) fn load_policy(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    config_key: &Pubkey,
    config: &ControllerConfigV1,
    slot: u64,
) -> Result<Box<GovernancePolicyV1>, ProgramError> {
    let policy = load_fixed_controller_account::<GovernancePolicyV1>(
        program_id,
        account,
        GovernancePolicyV1::LEN,
    )?;
    validate_policy_against_config(&policy, config)?;
    let (expected, bump) = derive_policy_pda(
        program_id,
        &config.target_program,
        config.current_policy_version,
    );
    if *account.key != expected
        || policy.bump != bump
        || policy.controller_config != *config_key
        || policy.activation_slot > slot
    {
        return Err(GovernanceError::InactivePolicy.into());
    }
    Ok(policy)
}

pub(super) fn load_current_council(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    config_key: &Pubkey,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
    slot: u64,
) -> Result<Box<GovernanceCouncilSetV1>, ProgramError> {
    let council = load_fixed_controller_account::<GovernanceCouncilSetV1>(
        program_id,
        account,
        GovernanceCouncilSetV1::LEN,
    )?;
    validate_council_set(&council, policy)?;
    validate_council_guardian_separation(&council, &config.guardian)?;
    let (expected, bump) = derive_council_pda(
        program_id,
        &config.target_program,
        config.current_council_version,
    );
    if *account.key != expected
        || council.bump != bump
        || council.controller_config != *config_key
        || council.version != config.current_council_version
        || !council.active_at(slot)
    {
        return Err(GovernanceError::StaleCouncilVersion.into());
    }
    Ok(council)
}
