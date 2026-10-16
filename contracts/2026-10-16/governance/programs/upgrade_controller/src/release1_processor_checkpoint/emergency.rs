use super::*;

pub(super) fn validate_emergency_expiry_expectation(
    expected: &crate::instruction::EmergencyResolutionExpectationV1,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
    gate: &ProtocolGateV1,
    resolution: &EmergencyFreezeResolutionV1,
) -> GovernanceResult<()> {
    if expected.expected_resolution_digest != resolution.resolution_digest
        || expected.expected_policy_version != config.current_policy_version
        || expected.expected_policy_version != policy.version
        || expected.expected_policy_hash != policy.policy_hash
        || expected.expected_council_version != resolution.approval_council_version
        || expected.expected_council_hash != resolution.approval_council_hash
        || expected.expected_gate_status != gate.status
        || expected.expected_gate_epoch != gate.epoch
        || expected.expected_freeze_slot != gate.freeze_slot
        || expected.expected_freeze_reason_code != gate.freeze_reason_code
        || expected.expected_target_nonce != resolution.target_nonce
        || expected.expected_target_nonce != config.target_nonce
        || expected.expected_state != resolution.state
        || expected.expected_not_before_slot != resolution.not_before_slot
        || expected.expected_expiry_slot != resolution.expiry_slot
    {
        return Err(GovernanceError::CrossAccountMismatch);
    }
    Ok(())
}

pub fn process_expire_emergency_resolution_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExpireEmergencyResolutionV1,
) -> ProgramResult {
    exact_account_count(accounts, 4)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, gate_info, resolution_info] = accounts else {
        unreachable!("account count checked")
    };
    validate_readonly(config_info)?;
    validate_readonly(policy_info)?;
    validate_readonly(gate_info)?;
    validate_writable(resolution_info)?;
    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info.key, &config, slot)?;
    let gate = load_gate(program_id, gate_info, config_info.key, &config)?;
    let mut resolution = load_fixed_controller_account::<EmergencyFreezeResolutionV1>(
        program_id,
        resolution_info,
        EmergencyFreezeResolutionV1::LEN,
    )?;
    validate_emergency_resolution_digest_v1(&resolution)?;
    let (expected_pda, bump) = derive_emergency_resolution_pda(
        program_id,
        &config.target_program,
        resolution.frozen_epoch,
    );
    if *resolution_info.key != expected_pda
        || resolution.bump != bump
        || resolution.controller_config != *config_info.key
        || resolution.protocol_gate != *gate_info.key
        || resolution.target_program != config.target_program
        || resolution.target_programdata != config.target_programdata
        || resolution.frozen_epoch != gate.epoch
        || resolution.freeze_slot != gate.freeze_slot
        || resolution.freeze_reason_code != gate.freeze_reason_code
        || gate.status != GateStatusV1::EmergencyFrozen
        || gate.active_proposal != Pubkey::default()
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_emergency_expiry_expectation(
        &instruction.expected,
        &config,
        &policy,
        &gate,
        &resolution,
    )?;
    if !matches!(
        resolution.state,
        EmergencyFreezeResolutionStateV1::Draft
            | EmergencyFreezeResolutionStateV1::CouncilApproved
            | EmergencyFreezeResolutionStateV1::Timelocked
    ) || slot < resolution.expiry_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    resolution.state = EmergencyFreezeResolutionStateV1::Expired;
    resolution.terminal_reason_code = EMERGENCY_RESOLUTION_EXPIRED_TERMINAL_REASON_V1;
    validate_emergency_resolution_digest_v1(&resolution)?;
    store_fixed_controller_account(
        program_id,
        resolution_info,
        &*resolution,
        EmergencyFreezeResolutionV1::LEN,
    )
}
