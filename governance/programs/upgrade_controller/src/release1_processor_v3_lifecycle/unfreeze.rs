use super::*;

pub fn process_approve_unfreeze_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ApproveUnfreezeV2,
) -> ProgramResult {
    exact_account_count(accounts, 14)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, proposal_info, checkpoint_info, verification_info, capacity_info, deployment_info, target_program, target_programdata, authority, loader, seat] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for info in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        checkpoint_info,
        verification_info,
        capacity_info,
        deployment_info,
        target_programdata,
        authority,
    ] {
        readonly(info)?;
    }
    writable(proposal_info)?;
    executable_readonly(target_program)?;
    executable_readonly(loader)?;
    validate_seat_authority(seat)?;

    let slot = current_slot()?;
    let context = load_lifecycle_context(
        program_id,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
    )?;
    let policy = load_policy(program_id, policy_info, config_info, &context.config, slot)?;
    let council = load_current_council(
        program_id,
        council_info,
        config_info,
        &context.config,
        &policy,
        slot,
    )?;
    let mut proposal = load_proposal(
        program_id,
        proposal_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
    )?;
    require_unfreeze_gate_binding(proposal_info.key, &proposal, &context)?;
    if !matches!(
        proposal.state,
        ProposalStateV2::PoststateAccepted | ProposalStateV2::UnfreezeApproved
    ) || proposal.policy_hash != policy.policy_hash
        || *seat.key == context.config.guardian
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let verification = load_verified_programdata_for_unfreeze_v2(
        program_id,
        verification_info,
        proposal_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
        &proposal,
    )?;
    let checkpoint = load_accepted_poststate_for_unfreeze_v2(
        program_id,
        checkpoint_info,
        proposal_info,
        config_info,
        capacity_info,
        deployment_info,
        &context,
        &proposal,
        &verification,
    )?;
    validate_live_verified_programdata_v2(
        target_program,
        target_programdata,
        authority,
        loader,
        &context,
        &verification,
    )?;
    prepare_unfreeze_accumulator_v2(&mut proposal, &council)?;
    if proposal.state != ProposalStateV2::PoststateAccepted {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    check_unfreeze_guard_v2(
        &instruction.expected,
        &proposal,
        &checkpoint,
        &verification,
        &context,
        &council,
    )?;
    if proposal.unfreeze_approval_count == 0 {
        proposal.unfreeze_council_version = council.version;
        proposal.unfreeze_council_hash = council.set_hash;
    }
    let (bitset, count) = record_seat_approval(
        &council,
        proposal.unfreeze_approval_bitset,
        proposal.unfreeze_approval_count,
        seat.key,
        slot,
    )?;
    proposal.unfreeze_approval_bitset = bitset;
    proposal.unfreeze_approval_count = count;
    if count == RELEASE1_APPROVAL_THRESHOLD {
        proposal.state = ProposalStateV2::UnfreezeApproved;
        proposal.unfreeze_approved_slot = slot;
    }
    validate_upgrade_proposal_digest_v3(&proposal)?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        UpgradeProposalV3::LEN,
    )
}

pub fn process_execute_unfreeze_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExecuteUnfreezeV2,
) -> ProgramResult {
    exact_account_count(accounts, 15)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, proposal_info, linked_info, checkpoint_info, verification_info, capacity_info, deployment_info, target_program, target_programdata, authority, loader, instructions_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for info in [
        config_info,
        policy_info,
        council_info,
        checkpoint_info,
        verification_info,
        capacity_info,
        target_programdata,
        authority,
        instructions_info,
    ] {
        readonly(info)?;
    }
    for info in [gate_info, proposal_info, linked_info, deployment_info] {
        writable(info)?;
    }
    executable_readonly(target_program)?;
    executable_readonly(loader)?;
    if *loader.key != UPGRADEABLE_LOADER_ID
        || *instructions_info.key != sysvar_ids::instructions::ID
        || instruction.linked_proposal != *linked_info.key
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let slot = current_slot()?;
    let mut context = load_lifecycle_context(
        program_id,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
    )?;
    let policy = load_policy(program_id, policy_info, config_info, &context.config, slot)?;
    let council = load_current_council(
        program_id,
        council_info,
        config_info,
        &context.config,
        &policy,
        slot,
    )?;
    let mut proposal = load_proposal(
        program_id,
        proposal_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
    )?;
    let mut linked = load_proposal(
        program_id,
        linked_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
    )?;
    require_unfreeze_gate_binding(proposal_info.key, &proposal, &context)?;
    if proposal.state != ProposalStateV2::UnfreezeApproved
        || proposal.policy_hash != policy.policy_hash
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let verification = load_verified_programdata_for_unfreeze_v2(
        program_id,
        verification_info,
        proposal_info,
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        &context,
        &proposal,
    )?;
    let checkpoint = load_accepted_poststate_for_unfreeze_v2(
        program_id,
        checkpoint_info,
        proposal_info,
        config_info,
        capacity_info,
        deployment_info,
        &context,
        &proposal,
        &verification,
    )?;
    check_unfreeze_guard_v2(
        &instruction.expected,
        &proposal,
        &checkpoint,
        &verification,
        &context,
        &council,
    )?;
    require_current_unfreeze_quorum_v2(&proposal, &council, slot)?;
    validate_live_verified_programdata_v2(
        target_program,
        target_programdata,
        authority,
        loader,
        &context,
        &verification,
    )?;
    validate_canonical_envelope(
        program_id,
        accounts,
        instructions_info,
        &instruction.pack()?,
        &instruction.envelope,
    )?;

    let next_epoch = checked_nonterminal_increment(context.gate.epoch)?;
    terminalize_unfreeze_pair_v3(
        &proposal,
        proposal_info.key,
        &mut linked,
        linked_info.key,
        slot,
    )?;
    proposal.state = ProposalStateV2::Completed;
    proposal.terminal_slot = slot;
    proposal.terminal_reason_code = PROPOSAL_COMPLETED_TERMINAL_REASON_V1;
    context.gate.epoch = next_epoch;
    context.gate.status = GateStatusV1::Active;
    context.gate.active_proposal = Pubkey::default();
    context.gate.freeze_slot = 0;
    context.gate.freeze_reason_code = 0;
    context.gate.last_completed_proposal = *proposal_info.key;
    let next_deployment = build_activated_deployment_v2(
        &context.deployment,
        &context.config,
        proposal_info.key,
        &proposal,
        &verification,
        next_epoch,
        slot,
    )?;
    validate_upgrade_proposal_digest_v3(&proposal)?;
    validate_upgrade_proposal_digest_v3(&linked)?;
    context.gate.validate_static()?;
    let gate_bytes = encode_fixed_account(&*context.gate, ProtocolGateV1::LEN)?;
    let proposal_bytes = encode_fixed_account(&*proposal, UpgradeProposalV3::LEN)?;
    let linked_bytes = encode_fixed_account(&*linked, UpgradeProposalV3::LEN)?;
    let deployment_bytes = encode_fixed_account(&next_deployment, CurrentDeploymentStateV1::LEN)?;
    commit_four_fixed_accounts(
        program_id,
        gate_info,
        &gate_bytes,
        ProtocolGateV1::LEN,
        proposal_info,
        &proposal_bytes,
        UpgradeProposalV3::LEN,
        linked_info,
        &linked_bytes,
        UpgradeProposalV3::LEN,
        deployment_info,
        &deployment_bytes,
        CurrentDeploymentStateV1::LEN,
    )
}
