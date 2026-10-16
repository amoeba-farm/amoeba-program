use super::*;

pub(super) fn validate_readonly_state_accounts(accounts: &[&AccountInfo<'_>]) -> ProgramResult {
    for account in accounts {
        validate_exact_privileges(account, false, false, false)?;
    }
    Ok(())
}

pub(super) fn validate_loader_identity(
    loader: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> ProgramResult {
    if *loader.key != UPGRADEABLE_LOADER_ID || config.upgradeable_loader != UPGRADEABLE_LOADER_ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn require_policy_active(policy: &GovernancePolicyV1, slot: u64) -> ProgramResult {
    if slot == 0 || slot < policy.activation_slot {
        return Err(GovernanceError::InactivePolicy.into());
    }
    Ok(())
}

pub(super) fn require_active_seat(
    council: &GovernanceCouncilSetV1,
    authority: &Pubkey,
    slot: u64,
) -> ProgramResult {
    let seat = council
        .seats
        .iter()
        .find(|seat| seat.seat_authority == *authority)
        .ok_or(GovernanceError::UnknownSeatAuthority)?;
    if !council.active_at(slot) || !seat.term_covers(slot) {
        return Err(GovernanceError::InactiveCouncilSeat.into());
    }
    Ok(())
}

pub(super) fn reject_guardian_authority(
    config: &ControllerConfigV1,
    authority: &Pubkey,
) -> ProgramResult {
    if *authority == config.guardian {
        return Err(GovernanceError::UnknownSeatAuthority.into());
    }
    Ok(())
}

pub(super) fn require_approval_window(
    review_start_slot: u64,
    review_end_slot: u64,
    expiry_slot: u64,
    slot: u64,
) -> ProgramResult {
    if slot < review_start_slot || slot > review_end_slot || slot >= expiry_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(())
}

pub(super) fn checked_freeze_counters(
    target_nonce: u64,
    gate_epoch: u64,
) -> GovernanceResult<(u64, u64)> {
    let next_target_nonce = target_nonce
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let next_gate_epoch = gate_epoch
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    Ok((next_target_nonce, next_gate_epoch))
}

pub(super) fn require_exact_recorded_quorum(
    council: &GovernanceCouncilSetV1,
    bitset: u8,
    count: u8,
    slot: u64,
) -> ProgramResult {
    if bitset & !VALID_APPROVAL_MASK != 0
        || bitset.count_ones() as u8 != count
        || count != RELEASE1_APPROVAL_THRESHOLD
        || !council.active_at(slot)
    {
        return Err(GovernanceError::QuorumNotSatisfied.into());
    }
    for (index, seat) in council.seats.iter().enumerate() {
        if bitset & (1 << index) != 0 && !seat.term_covers(slot) {
            return Err(GovernanceError::InactiveCouncilSeat.into());
        }
    }
    Ok(())
}

pub(super) fn derive_proposal_timing(
    config: &ControllerConfigV1,
    class: ProposalClassV1,
    creation_slot: u64,
) -> GovernanceResult<(u64, u64, u64, u64)> {
    let delay = match class {
        ProposalClassV1::EmergencyRollback => config.rollback_delay_slots,
        ProposalClassV1::RoutineUpgrade => config.routine_delay_slots,
        ProposalClassV1::EconomicChange | ProposalClassV1::ConstitutionalChange => {
            config.major_delay_slots
        }
        ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability => {
            return Err(GovernanceError::UnsupportedProposalClass)
        }
    };
    let review_start = creation_slot
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let review_end = review_start
        .checked_add(config.council_review_slots())
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let not_before = review_end
        .checked_add(delay)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let expiry = creation_slot
        .checked_add(config.proposal_expiry_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if not_before >= expiry {
        return Err(GovernanceError::InvalidProposalTiming);
    }
    Ok((review_start, review_end, not_before, expiry))
}

pub(super) fn verify_exact_proposal_timing(
    proposal: &UpgradeProposalV2,
    config: &ControllerConfigV1,
) -> ProgramResult {
    let exact = derive_proposal_timing(config, proposal.proposal_class, proposal.creation_slot)?;
    if exact
        != (
            proposal.review_start_slot,
            proposal.review_end_slot,
            proposal.not_before_slot,
            proposal.expiry_slot,
        )
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(())
}

pub(super) fn check_proposal_expectation(
    expected: &ProposalExpectationV2,
    proposal: &UpgradeProposalV2,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
    gate: &ProtocolGateV1,
    council_version: u64,
    council_hash: &[u8; 32],
) -> ProgramResult {
    if expected.expected_policy_version != policy.version
        || expected.expected_policy_hash != policy.policy_hash
    {
        return Err(GovernanceError::PolicyHashMismatch.into());
    }
    check_proposal_expectation_without_policy(
        expected,
        proposal,
        config,
        gate,
        council_version,
        council_hash,
    )
}

pub(super) fn check_proposal_expectation_without_policy(
    expected: &ProposalExpectationV2,
    proposal: &UpgradeProposalV2,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    council_version: u64,
    council_hash: &[u8; 32],
) -> ProgramResult {
    if expected.expected_proposal_digest != proposal.proposal_digest
        || expected.expected_policy_version != proposal.policy_version
        || expected.expected_policy_hash != proposal.policy_hash
        || expected.expected_council_version != council_version
        || expected.expected_council_hash != *council_hash
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
    Ok(())
}

pub(super) fn require_creation_gate(
    proposal: &UpgradeProposalV2,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
) -> ProgramResult {
    require_creation_gate_values(
        proposal.target_nonce,
        proposal.creation_gate_status,
        proposal.creation_gate_epoch,
        config.target_nonce,
        gate.status,
        gate.epoch,
        gate.freeze_reason_code,
    )
}

pub(super) fn require_creation_gate_values(
    proposal_target_nonce: u64,
    creation_gate_status: GateStatusV1,
    creation_gate_epoch: u64,
    current_target_nonce: u64,
    current_gate_status: GateStatusV1,
    current_gate_epoch: u64,
    current_freeze_reason_code: u16,
) -> ProgramResult {
    if proposal_target_nonce != current_target_nonce
        || creation_gate_status != current_gate_status
        || creation_gate_epoch != current_gate_epoch
        || current_gate_status == GateStatusV1::FrozenForUpgrade
        || current_gate_status == GateStatusV1::EmergencyFrozen
            && current_freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
    {
        return Err(GovernanceError::InvalidProposalEpoch.into());
    }
    Ok(())
}

pub(super) fn is_pre_freeze_state(state: ProposalStateV2) -> bool {
    matches!(
        state,
        ProposalStateV2::Draft
            | ProposalStateV2::BufferAdopted
            | ProposalStateV2::BufferVerified
            | ProposalStateV2::CouncilApproved
            | ProposalStateV2::GovernanceSatisfied
            | ProposalStateV2::Timelocked
    )
}

/// A rollback proposal is ordinary pre-freeze state only until its exact
/// linked primary consumes the target nonce and becomes the active frozen
/// proposal. From that point the rollback buffer is a mandatory recovery
/// capability and cannot be cancelled or expired independently.
pub(super) fn reject_locked_reciprocal_rollback(
    proposal: &UpgradeProposalV2,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
) -> ProgramResult {
    if proposal.proposal_class != ProposalClassV1::EmergencyRollback {
        return Ok(());
    }
    let consumed_nonce = proposal
        .target_nonce
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if proposal.primary_proposal.present
        && config.target_nonce == consumed_nonce
        && gate.status == GateStatusV1::FrozenForUpgrade
        && gate.active_proposal == proposal.primary_proposal.value
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    Ok(())
}
