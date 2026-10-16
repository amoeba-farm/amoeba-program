use super::*;

fn load_candidate_council(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    config_key: &Pubkey,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
) -> Result<Box<GovernanceCouncilSetV1>, ProgramError> {
    let candidate = load_fixed_controller_account::<GovernanceCouncilSetV1>(
        program_id,
        account,
        GovernanceCouncilSetV1::LEN,
    )?;
    validate_council_set(&candidate, policy)?;
    validate_council_guardian_separation(&candidate, &config.guardian)?;
    let (expected, bump) =
        derive_council_pda(program_id, &config.target_program, candidate.version);
    if *account.key != expected
        || candidate.bump != bump
        || candidate.controller_config != *config_key
        || candidate.target_program != config.target_program
        || candidate.version <= config.current_council_version
        || candidate.version == u64::MAX
        || candidate.deactivation_slot != 0
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    Ok(candidate)
}

fn create_rotation_pda_account<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    rotation_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    rotation: &CouncilRotationProposalV1,
) -> ProgramResult {
    let version = rotation.candidate_council_version.to_le_bytes();
    let bump = [rotation.bump];
    let seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        COUNCIL_ROTATION_SEED,
        rotation.target_program.as_ref(),
        &version,
        &bump,
    ];
    let encoded = encode_fixed_account(rotation, CouncilRotationProposalV1::LEN)?;
    create_fixed_pda_account(
        program_id,
        payer,
        rotation_info,
        system_program_info,
        &Rent::get()?,
        CouncilRotationProposalV1::LEN,
        seeds,
    )?;
    let mut data = rotation_info.try_borrow_mut_data()?;
    data.copy_from_slice(&encoded);
    Ok(())
}

pub fn process_create_council_rotation_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CreateCouncilRotationV1,
) -> ProgramResult {
    exact_account_count(accounts, 9)?;
    all_distinct(accounts)?;
    let [payer, creator, config_info, policy_info, current_council_info, candidate_info, gate_info, rotation_info, system_program_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    validate_signer_writable(payer)?;
    validate_signer_readonly(creator)?;
    for readonly in [
        config_info,
        policy_info,
        current_council_info,
        candidate_info,
        gate_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(rotation_info)?;
    validate_system_program(system_program_info)?;
    require_absent_system_account(rotation_info)?;

    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info.key, &config, slot)?;
    let current_council = load_current_council(
        program_id,
        current_council_info,
        config_info.key,
        &config,
        &policy,
        slot,
    )?;
    let candidate = load_candidate_council(
        program_id,
        candidate_info,
        config_info.key,
        &config,
        &policy,
    )?;
    let gate = load_gate(program_id, gate_info, config_info.key, &config)?;
    require_active_seat_authority(&current_council, creator.key, slot)?;
    if instruction.creation_slot != slot
        || instruction.expected_current_council_version != current_council.version
        || instruction.expected_current_council_hash != current_council.set_hash
        || instruction.expected_candidate_council_version != candidate.version
        || instruction.expected_candidate_council_hash != candidate.set_hash
        || instruction.expected_gate_status != gate.status
        || instruction.expected_gate_epoch != gate.epoch
        || instruction.expected_target_nonce != config.target_nonce
        || instruction.not_before_slot != candidate.activation_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let minimum_not_before = slot
        .checked_add(config.major_delay_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let exact_expiry = slot
        .checked_add(config.proposal_expiry_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if instruction.not_before_slot < minimum_not_before
        || instruction.expiry_slot != exact_expiry
        || instruction.not_before_slot >= instruction.expiry_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let (rotation_pda, rotation_bump) = crate::pda::derive_council_rotation_pda(
        program_id,
        &config.target_program,
        candidate.version,
    );
    if *rotation_info.key != rotation_pda {
        return Err(GovernanceError::InvalidPda.into());
    }
    let mut rotation = CouncilRotationProposalV1 {
        discriminator: COUNCIL_ROTATION_PROPOSAL_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: rotation_bump,
        initialized: true,
        state: CouncilRotationStateV1::Draft,
        controller_config: *config_info.key,
        target_program: config.target_program,
        current_council: *current_council_info.key,
        current_council_version: current_council.version,
        current_council_hash: current_council.set_hash,
        candidate_council: *candidate_info.key,
        candidate_council_version: candidate.version,
        candidate_council_hash: candidate.set_hash,
        creation_slot: slot,
        not_before_slot: instruction.not_before_slot,
        expiry_slot: instruction.expiry_slot,
        target_nonce: config.target_nonce,
        approval_bitset: 0,
        approval_count: 0,
        cancellation_approval_bitset: 0,
        cancellation_approval_count: 0,
        rotation_digest: [0; 32],
        activated_slot: 0,
        cancellation_reason_code: 0,
        terminal_reason_code: 0,
        reserved: [0; COUNCIL_ROTATION_PROPOSAL_V1_RESERVED_LEN],
    };
    rotation.rotation_digest = compute_council_rotation_digest_v1(&rotation)?;
    if rotation.rotation_digest != instruction.expected_rotation_digest {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    validate_council_rotation_digest_v1(&rotation)?;
    create_rotation_pda_account(
        program_id,
        payer,
        rotation_info,
        system_program_info,
        &rotation,
    )
}

pub(super) fn load_rotation(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    config_key: &Pubkey,
    config: &ControllerConfigV1,
) -> Result<Box<CouncilRotationProposalV1>, ProgramError> {
    let rotation = load_fixed_controller_account::<CouncilRotationProposalV1>(
        program_id,
        account,
        CouncilRotationProposalV1::LEN,
    )?;
    validate_council_rotation_digest_v1(&rotation)?;
    let (expected, bump) = crate::pda::derive_council_rotation_pda(
        program_id,
        &config.target_program,
        rotation.candidate_council_version,
    );
    if *account.key != expected
        || rotation.bump != bump
        || rotation.controller_config != *config_key
        || rotation.target_program != config.target_program
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    Ok(rotation)
}

pub(super) fn validate_rotation_expectation(
    expected: &CouncilRotationExpectationV1,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    rotation: &CouncilRotationProposalV1,
) -> GovernanceResult<()> {
    if expected.expected_rotation_digest != rotation.rotation_digest
        || expected.expected_current_council_version != rotation.current_council_version
        || expected.expected_current_council_hash != rotation.current_council_hash
        || expected.expected_candidate_council_version != rotation.candidate_council_version
        || expected.expected_candidate_council_hash != rotation.candidate_council_hash
        || expected.expected_gate_status != gate.status
        || expected.expected_gate_epoch != gate.epoch
        || expected.expected_target_nonce != rotation.target_nonce
        || expected.expected_target_nonce != config.target_nonce
        || expected.expected_state != rotation.state
        || expected.expected_not_before_slot != rotation.not_before_slot
        || expected.expected_expiry_slot != rotation.expiry_slot
        || config.current_council_version != rotation.current_council_version
    {
        return Err(GovernanceError::CrossAccountMismatch);
    }
    Ok(())
}

fn validate_rotation_graph(
    rotation: &CouncilRotationProposalV1,
    current_council_key: &Pubkey,
    current_council: &GovernanceCouncilSetV1,
    candidate_key: &Pubkey,
    candidate: &GovernanceCouncilSetV1,
) -> GovernanceResult<()> {
    if rotation.current_council != *current_council_key
        || rotation.current_council_version != current_council.version
        || rotation.current_council_hash != current_council.set_hash
        || rotation.candidate_council != *candidate_key
        || rotation.candidate_council_version != candidate.version
        || rotation.candidate_council_hash != candidate.set_hash
        || rotation.not_before_slot != candidate.activation_slot
    {
        return Err(GovernanceError::CrossAccountMismatch);
    }
    Ok(())
}

pub(super) fn load_rotation_context(
    context: &RotationAccountContext<'_, '_>,
) -> Result<LoadedRotationContext, ProgramError> {
    let config = load_config(context.program_id, context.config_info)?;
    let policy = load_policy(
        context.program_id,
        context.policy_info,
        context.config_info.key,
        &config,
        context.slot,
    )?;
    let current_council = load_current_council(
        context.program_id,
        context.current_council_info,
        context.config_info.key,
        &config,
        &policy,
        context.slot,
    )?;
    let candidate = load_candidate_council(
        context.program_id,
        context.candidate_info,
        context.config_info.key,
        &config,
        &policy,
    )?;
    let gate = load_gate(
        context.program_id,
        context.gate_info,
        context.config_info.key,
        &config,
    )?;
    let rotation = load_rotation(
        context.program_id,
        context.rotation_info,
        context.config_info.key,
        &config,
    )?;
    validate_rotation_expectation(context.expected, &config, &gate, &rotation)?;
    validate_rotation_graph(
        &rotation,
        context.current_council_info.key,
        &current_council,
        context.candidate_info.key,
        &candidate,
    )?;
    Ok(LoadedRotationContext {
        config,
        current_council,
        candidate,
        rotation,
    })
}
