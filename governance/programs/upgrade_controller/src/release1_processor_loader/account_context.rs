use super::*;

pub(super) fn load_config(
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
        || derive_upgradeable_programdata_address(&config.target_program).0
            != config.target_programdata
        || config.upgradeable_loader != UPGRADEABLE_LOADER_ID
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    Ok(config)
}

pub(super) fn load_policy(
    program_id: &Pubkey,
    policy_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<GovernancePolicyV1>, ProgramError> {
    let policy = load_fixed_controller_account::<GovernancePolicyV1>(
        program_id,
        policy_info,
        GovernancePolicyV1::LEN,
    )?;
    validate_policy_against_config(&policy, config)?;
    let expected =
        crate::pda::derive_policy_pda(program_id, &config.target_program, policy.version);
    if expected != (*policy_info.key, policy.bump)
        || policy.controller_config != *config_info.key
        || policy.target_program != config.target_program
        || policy.version != config.current_policy_version
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(policy)
}

pub(super) fn load_gate(
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

pub(super) fn load_proposal(
    program_id: &Pubkey,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<UpgradeProposalV2>, ProgramError> {
    let proposal = load_fixed_controller_account::<UpgradeProposalV2>(
        program_id,
        proposal_info,
        UpgradeProposalV2::LEN,
    )?;
    validate_proposal_digest_v2(&proposal)?;
    if derive_proposal_pda(program_id, &config.target_program, proposal.proposal_id)
        != (*proposal_info.key, proposal.bump)
        || proposal.controller_program != *program_id
        || proposal.controller_config != *config_info.key
        || proposal.protocol_gate != config.gate_pda
        || proposal.cluster_domain != config.cluster_domain
        || proposal.target_program != config.target_program
        || proposal.target_programdata != config.target_programdata
        || proposal.upgradeable_loader != config.upgradeable_loader
        || proposal.authority_pda != config.authority_pda
        || proposal.canonical_spill_treasury != config.canonical_spill_treasury
        || proposal.policy_version != config.current_policy_version
        || proposal.proposal_id >= config.next_proposal_id
        || proposal.buffer_verification != derive_buffer_check_pda(program_id, proposal_info.key).0
        || proposal.programdata_verification
            != derive_programdata_check_pda(program_id, proposal_info.key).0
        || proposal.prestate_checkpoint
            != derive_checkpoint_pda(program_id, proposal_info.key, CheckpointPhaseV1::Prestate).0
        || proposal.required_poststate_checkpoint
            != derive_checkpoint_pda(program_id, proposal_info.key, CheckpointPhaseV1::Poststate).0
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    verify_exact_proposal_timing(&proposal, config)?;
    Ok(proposal)
}

pub(super) fn validate_frozen_expectation(
    expected: &ProposalExpectationV2,
    proposal: &UpgradeProposalV2,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    proposal_key: &Pubkey,
) -> ProgramResult {
    if expected.expected_proposal_digest != proposal.proposal_digest
        || expected.expected_policy_version != proposal.policy_version
        || expected.expected_policy_hash != proposal.policy_hash
        || expected.expected_council_version != proposal.creation_council_version
        || expected.expected_council_hash != proposal.creation_council_hash
        || expected.expected_gate_status != gate.status
        || expected.expected_gate_epoch != gate.epoch
        || expected.expected_target_nonce != config.target_nonce
        || expected.expected_state != proposal.state
        || expected.expected_review_start_slot != proposal.review_start_slot
        || expected.expected_review_end_slot != proposal.review_end_slot
        || expected.expected_not_before_slot != proposal.not_before_slot
        || expected.expected_expiry_slot != proposal.expiry_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let consumed_nonce = proposal
        .target_nonce
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if gate.status != GateStatusV1::FrozenForUpgrade
        || gate.active_proposal != *proposal_key
        || gate.epoch != proposal.freeze_gate_epoch
        || config.target_nonce != consumed_nonce
        || gate.freeze_slot != proposal.frozen_slot
        || gate.freeze_reason_code == 0
    {
        return Err(GovernanceError::InvalidProposalEpoch.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_accepted_prestate(
    program_id: &Pubkey,
    checkpoint_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV2,
    gate: &ProtocolGateV1,
    expected_digest: &[u8; 32],
    operation_slot: u64,
) -> Result<Box<StateCheckpointV1>, ProgramError> {
    let checkpoint = load_fixed_controller_account::<StateCheckpointV1>(
        program_id,
        checkpoint_info,
        StateCheckpointV1::LEN,
    )?;
    validate_state_checkpoint_digest_v1(&checkpoint)?;
    let expected =
        derive_checkpoint_pda(program_id, proposal_info.key, CheckpointPhaseV1::Prestate);
    if expected != (*checkpoint_info.key, checkpoint.bump)
        || *checkpoint_info.key != proposal.prestate_checkpoint
        || checkpoint.phase != StateCheckpointPhaseV1::Prestate
        || checkpoint.controller_config != *config_info.key
        || checkpoint.proposal != *proposal_info.key
        || checkpoint.emergency_resolution != Pubkey::default()
        || checkpoint.subject_digest != proposal.proposal_digest
        || checkpoint.target_program != config.target_program
        || checkpoint.target_programdata != config.target_programdata
        || checkpoint.gate_epoch != gate.epoch
        || checkpoint.target_payload_commitment != proposal.expected_execution_pre_payload_hash
        || checkpoint.target_capacity != proposal.current_capacity
        || checkpoint.checkpoint_digest != *expected_digest
        || !checkpoint.accepted
        || checkpoint.forbidden_drift_count != 0
        || checkpoint.finalized_slot > operation_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    // Ordinary proposals bind the concrete pre-upgrade header/raw values in
    // their immutable digest. EmergencyRollback uses its linked primary's
    // mechanically verified ProgramData evidence, so its canonical proposal
    // fields remain zero by design.
    if proposal.proposal_class != ProposalClassV1::EmergencyRollback
        && (checkpoint.target_programdata_slot != proposal.deployed_slot
            || checkpoint.target_raw_programdata_commitment
                != proposal.current_raw_programdata_hash)
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(checkpoint)
}

pub(super) fn commit_one_fixed_account(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    bytes: &[u8],
    expected_len: usize,
) -> ProgramResult {
    if account.owner != program_id {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let mut data = account.try_borrow_mut_data()?;
    if data.len() != expected_len || bytes.len() != expected_len {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    data.copy_from_slice(bytes);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn commit_two_fixed_accounts(
    program_id: &Pubkey,
    first: &AccountInfo<'_>,
    first_bytes: &[u8],
    first_len: usize,
    second: &AccountInfo<'_>,
    second_bytes: &[u8],
    second_len: usize,
) -> ProgramResult {
    if first.owner != program_id || second.owner != program_id {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let mut first_data = first.try_borrow_mut_data()?;
    let mut second_data = second.try_borrow_mut_data()?;
    if first_data.len() != first_len
        || second_data.len() != second_len
        || first_bytes.len() != first_len
        || second_bytes.len() != second_len
    {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    first_data.copy_from_slice(first_bytes);
    second_data.copy_from_slice(second_bytes);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn commit_three_fixed_accounts(
    program_id: &Pubkey,
    first: &AccountInfo<'_>,
    first_bytes: &[u8],
    first_len: usize,
    second: &AccountInfo<'_>,
    second_bytes: &[u8],
    second_len: usize,
    third: &AccountInfo<'_>,
    third_bytes: &[u8],
    third_len: usize,
) -> ProgramResult {
    if first.owner != program_id || second.owner != program_id || third.owner != program_id {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let mut first_data = first.try_borrow_mut_data()?;
    let mut second_data = second.try_borrow_mut_data()?;
    let mut third_data = third.try_borrow_mut_data()?;
    if first_data.len() != first_len
        || second_data.len() != second_len
        || third_data.len() != third_len
        || first_bytes.len() != first_len
        || second_bytes.len() != second_len
        || third_bytes.len() != third_len
    {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    first_data.copy_from_slice(first_bytes);
    second_data.copy_from_slice(second_bytes);
    third_data.copy_from_slice(third_bytes);
    Ok(())
}
