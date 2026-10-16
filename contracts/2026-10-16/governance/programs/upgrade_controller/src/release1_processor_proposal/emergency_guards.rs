use super::*;

pub(super) fn check_emergency_expectation(
    expected: &EmergencyResolutionExpectationV1,
    resolution: &EmergencyFreezeResolutionV1,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
    council: &GovernanceCouncilSetV1,
    gate: &ProtocolGateV1,
) -> ProgramResult {
    if expected.expected_policy_version != policy.version
        || expected.expected_policy_hash != policy.policy_hash
        || expected.expected_council_version != council.version
        || expected.expected_council_hash != council.set_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    check_emergency_expectation_without_policy(expected, resolution, config, gate)
}

fn check_emergency_expectation_without_policy(
    expected: &EmergencyResolutionExpectationV1,
    resolution: &EmergencyFreezeResolutionV1,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
) -> ProgramResult {
    if expected.expected_resolution_digest != resolution.resolution_digest
        || expected.expected_gate_status != gate.status
        || expected.expected_gate_epoch != gate.epoch
        || expected.expected_freeze_slot != gate.freeze_slot
        || expected.expected_freeze_reason_code != gate.freeze_reason_code
        || expected.expected_target_nonce != config.target_nonce
        || expected.expected_state != resolution.state
        || expected.expected_not_before_slot != resolution.not_before_slot
        || expected.expected_expiry_slot != resolution.expiry_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn require_emergency_binding(
    resolution: &EmergencyFreezeResolutionV1,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    slot: u64,
    allow_expired_slot: bool,
) -> ProgramResult {
    if gate.status != GateStatusV1::EmergencyFrozen
        || gate.active_proposal != Pubkey::default()
        || gate.epoch != resolution.frozen_epoch
        || gate.freeze_slot != resolution.freeze_slot
        || gate.freeze_reason_code != resolution.freeze_reason_code
        || gate.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        || config.target_nonce != resolution.target_nonce
        || (!allow_expired_slot && slot >= resolution.expiry_slot)
    {
        return Err(GovernanceError::InvalidProposalEpoch.into());
    }
    Ok(())
}

/// Admits only the two canonical bounded ComputeBudget instructions followed
/// by the exact emergency-resolution call.  The raw ProgramData hash can use a
/// substantial fraction of the default transaction budget at the Release 1
/// maximum account size, so rejecting ComputeBudget here would make the
/// otherwise valid recovery capability non-executable.
pub(super) fn validate_bounded_emergency_resolution_envelope(
    program_id: &Pubkey,
    account_infos: &[AccountInfo<'_>],
    instructions_sysvar: &AccountInfo<'_>,
    expected_data: &[u8],
) -> ProgramResult {
    if *instructions_sysvar.key != sysvar_ids::instructions::ID
        || instructions::load_current_index_checked(instructions_sysvar)? != 2
    {
        return Err(GovernanceError::InvalidAccountPrivileges.into());
    }

    let limit = instructions::load_instruction_at_checked(0, instructions_sysvar)?;
    let mut expected_limit_prefix = [0u8; 1];
    expected_limit_prefix[0] = 2;
    if limit.program_id != compute_budget::ID
        || !limit.accounts.is_empty()
        || limit.data.len() != 5
        || limit.data[..1] != expected_limit_prefix
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let mut limit_bytes = [0u8; 4];
    limit_bytes.copy_from_slice(&limit.data[1..]);
    let compute_unit_limit = u32::from_le_bytes(limit_bytes);
    if compute_unit_limit == 0 || compute_unit_limit > MAX_ENVELOPE_COMPUTE_UNIT_LIMIT_V1 {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let price = instructions::load_instruction_at_checked(1, instructions_sysvar)?;
    let mut expected_price_prefix = [0u8; 1];
    expected_price_prefix[0] = 3;
    if price.program_id != compute_budget::ID
        || !price.accounts.is_empty()
        || price.data.len() != 9
        || price.data[..1] != expected_price_prefix
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let mut price_bytes = [0u8; 8];
    price_bytes.copy_from_slice(&price.data[1..]);
    if u64::from_le_bytes(price_bytes) > MAX_ENVELOPE_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1 {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let current = instructions::load_instruction_at_checked(2, instructions_sysvar)?;
    if current.program_id != *program_id
        || current.data != expected_data
        || current.accounts.len() != account_infos.len()
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    for (meta, info) in current.accounts.iter().zip(account_infos) {
        if meta.pubkey != *info.key
            || meta.is_signer != info.is_signer
            || meta.is_writable != info.is_writable
        {
            return Err(GovernanceError::InvalidAccountPrivileges.into());
        }
    }
    if instructions::load_instruction_at_checked(3, instructions_sysvar).is_ok() {
        return Err(GovernanceError::InvalidAccountCount.into());
    }
    Ok(())
}
