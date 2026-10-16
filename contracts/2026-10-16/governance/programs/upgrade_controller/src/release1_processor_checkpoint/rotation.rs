use super::*;

pub fn process_approve_council_rotation_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ApproveCouncilRotationV1,
) -> ProgramResult {
    exact_account_count(accounts, 7)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, current_council_info, candidate_info, gate_info, rotation_info, seat_authority] =
        accounts
    else {
        unreachable!("account count checked")
    };
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
    validate_signer_readonly(seat_authority)?;
    let slot = current_slot()?;
    let rotation_context = RotationAccountContext {
        program_id,
        config_info,
        policy_info,
        current_council_info,
        candidate_info,
        gate_info,
        rotation_info,
        expected: &instruction.expected,
        slot,
    };
    let LoadedRotationContext {
        current_council: council,
        mut rotation,
        ..
    } = load_rotation_context(&rotation_context)?;
    if rotation.state != CouncilRotationStateV1::Draft
        || rotation.approval_bitset != instruction.expected_approval_bitset
        || rotation.approval_count != instruction.expected_approval_count
        || slot < rotation.creation_slot
        || slot >= rotation.expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    validate_mask_members_active(
        &council,
        rotation.approval_bitset,
        rotation.approval_count,
        slot,
    )?;
    let (next_bitset, next_count) = record_seat_approval(
        &council,
        rotation.approval_bitset,
        rotation.approval_count,
        seat_authority.key,
        slot,
    )?;
    rotation.approval_bitset = next_bitset;
    rotation.approval_count = next_count;
    if next_count == RELEASE1_APPROVAL_THRESHOLD {
        rotation.state = CouncilRotationStateV1::CouncilApproved;
    }
    validate_mask_members_active(&council, next_bitset, next_count, slot)?;
    validate_council_rotation_digest_v1(&rotation)?;
    store_fixed_controller_account(
        program_id,
        rotation_info,
        &*rotation,
        CouncilRotationProposalV1::LEN,
    )
}

pub fn process_queue_council_rotation_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: QueueCouncilRotationV1,
) -> ProgramResult {
    exact_account_count(accounts, 6)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, current_council_info, candidate_info, gate_info, rotation_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
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
    let slot = current_slot()?;
    let rotation_context = RotationAccountContext {
        program_id,
        config_info,
        policy_info,
        current_council_info,
        candidate_info,
        gate_info,
        rotation_info,
        expected: &instruction.expected,
        slot,
    };
    let LoadedRotationContext {
        current_council: council,
        mut rotation,
        ..
    } = load_rotation_context(&rotation_context)?;
    if rotation.state != CouncilRotationStateV1::CouncilApproved
        || rotation.approval_bitset != instruction.expected_approval_bitset
        || rotation.approval_count != instruction.expected_approval_count
        || slot < rotation.creation_slot
        || slot >= rotation.expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    validate_approval_mask_at(
        &council,
        rotation.approval_bitset,
        rotation.approval_count,
        slot,
    )?;
    rotation.state = CouncilRotationStateV1::Timelocked;
    validate_council_rotation_digest_v1(&rotation)?;
    store_fixed_controller_account(
        program_id,
        rotation_info,
        &*rotation,
        CouncilRotationProposalV1::LEN,
    )
}

fn store_config_and_rotation(
    program_id: &Pubkey,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    rotation_info: &AccountInfo<'_>,
    rotation: &CouncilRotationProposalV1,
) -> ProgramResult {
    if config_info.owner != program_id || rotation_info.owner != program_id {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let config_bytes = encode_fixed_account(config, ControllerConfigV1::LEN)?;
    let rotation_bytes = encode_fixed_account(rotation, CouncilRotationProposalV1::LEN)?;
    let mut config_data = config_info.try_borrow_mut_data()?;
    let mut rotation_data = rotation_info.try_borrow_mut_data()?;
    if config_data.len() != ControllerConfigV1::LEN
        || rotation_data.len() != CouncilRotationProposalV1::LEN
    {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    config_data.copy_from_slice(&config_bytes);
    rotation_data.copy_from_slice(&rotation_bytes);
    Ok(())
}

pub fn process_activate_council_rotation_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ActivateCouncilRotationV1,
) -> ProgramResult {
    exact_account_count(accounts, 6)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, current_council_info, candidate_info, gate_info, rotation_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    validate_writable(config_info)?;
    for readonly in [policy_info, current_council_info, candidate_info, gate_info] {
        validate_readonly(readonly)?;
    }
    validate_writable(rotation_info)?;
    let slot = current_slot()?;
    let rotation_context = RotationAccountContext {
        program_id,
        config_info,
        policy_info,
        current_council_info,
        candidate_info,
        gate_info,
        rotation_info,
        expected: &instruction.expected,
        slot,
    };
    let LoadedRotationContext {
        mut config,
        current_council,
        candidate,
        mut rotation,
    } = load_rotation_context(&rotation_context)?;
    if rotation.state != CouncilRotationStateV1::Timelocked
        || rotation.approval_bitset != instruction.expected_approval_bitset
        || rotation.approval_count != instruction.expected_approval_count
        || slot < rotation.not_before_slot
        || slot >= rotation.expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    validate_approval_mask_at(
        &current_council,
        rotation.approval_bitset,
        rotation.approval_count,
        slot,
    )?;
    validate_candidate_seats_at_slot(&candidate, slot)?;
    config.current_council_version = candidate.version;
    config.validate_static()?;
    rotation.state = CouncilRotationStateV1::Activated;
    rotation.activated_slot = slot;
    rotation.terminal_reason_code = COUNCIL_ROTATION_ACTIVATED_TERMINAL_REASON_V1;
    validate_council_rotation_digest_v1(&rotation)?;
    store_config_and_rotation(program_id, config_info, &config, rotation_info, &rotation)
}

pub fn process_cancel_council_rotation_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CancelCouncilRotationV1,
) -> ProgramResult {
    exact_account_count(accounts, 7)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, current_council_info, candidate_info, gate_info, rotation_info, seat_authority] =
        accounts
    else {
        unreachable!("account count checked")
    };
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
    validate_signer_readonly(seat_authority)?;
    let slot = current_slot()?;
    let rotation_context = RotationAccountContext {
        program_id,
        config_info,
        policy_info,
        current_council_info,
        candidate_info,
        gate_info,
        rotation_info,
        expected: &instruction.expected,
        slot,
    };
    let LoadedRotationContext {
        current_council: council,
        mut rotation,
        ..
    } = load_rotation_context(&rotation_context)?;
    if !matches!(
        rotation.state,
        CouncilRotationStateV1::Draft
            | CouncilRotationStateV1::CouncilApproved
            | CouncilRotationStateV1::Timelocked
    ) || rotation.cancellation_approval_bitset
        != instruction.expected_cancellation_approval_bitset
        || rotation.cancellation_approval_count != instruction.expected_cancellation_approval_count
        || instruction.cancellation_reason_code == 0
        || (rotation.cancellation_reason_code != 0
            && rotation.cancellation_reason_code != instruction.cancellation_reason_code)
        || slot < rotation.creation_slot
        || slot >= rotation.expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    validate_mask_members_active(
        &council,
        rotation.cancellation_approval_bitset,
        rotation.cancellation_approval_count,
        slot,
    )?;
    let (next_bitset, next_count) = record_seat_approval(
        &council,
        rotation.cancellation_approval_bitset,
        rotation.cancellation_approval_count,
        seat_authority.key,
        slot,
    )?;
    rotation.cancellation_reason_code = instruction.cancellation_reason_code;
    rotation.cancellation_approval_bitset = next_bitset;
    rotation.cancellation_approval_count = next_count;
    if next_count == RELEASE1_APPROVAL_THRESHOLD {
        rotation.state = CouncilRotationStateV1::Cancelled;
        rotation.terminal_reason_code = instruction.cancellation_reason_code;
    }
    validate_mask_members_active(&council, next_bitset, next_count, slot)?;
    validate_council_rotation_digest_v1(&rotation)?;
    store_fixed_controller_account(
        program_id,
        rotation_info,
        &*rotation,
        CouncilRotationProposalV1::LEN,
    )
}

pub fn process_expire_council_rotation_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExpireCouncilRotationV1,
) -> ProgramResult {
    exact_account_count(accounts, 3)?;
    all_distinct(accounts)?;
    let [config_info, gate_info, rotation_info] = accounts else {
        unreachable!("account count checked")
    };
    validate_readonly(config_info)?;
    validate_readonly(gate_info)?;
    validate_writable(rotation_info)?;
    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info.key, &config)?;
    let mut rotation = load_rotation(program_id, rotation_info, config_info.key, &config)?;
    validate_rotation_expectation(&instruction.expected, &config, &gate, &rotation)?;
    if !matches!(
        rotation.state,
        CouncilRotationStateV1::Draft
            | CouncilRotationStateV1::CouncilApproved
            | CouncilRotationStateV1::Timelocked
    ) || slot < rotation.expiry_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    rotation.state = CouncilRotationStateV1::Expired;
    rotation.terminal_reason_code = COUNCIL_ROTATION_EXPIRED_TERMINAL_REASON_V1;
    validate_council_rotation_digest_v1(&rotation)?;
    store_fixed_controller_account(
        program_id,
        rotation_info,
        &*rotation,
        CouncilRotationProposalV1::LEN,
    )
}
