use super::*;

pub(super) fn check_proposal_guard(
    expected: &ProposalGuardV3,
    proposal: &UpgradeProposalV3,
    context: &LifecycleContext,
) -> ProgramResult {
    require_guard_deployment(
        &context.capacity,
        &context.deployment,
        &expected.expected_capacity_policy_digest,
        &expected.expected_current_deployment_digest,
        expected.expected_current_deployment_generation,
    )?;
    if expected.expected_proposal_digest != proposal.proposal_digest
        || expected.expected_state != proposal.state
        || expected.expected_gate_status != context.gate.status
        || expected.expected_gate_epoch != context.gate.epoch
        || expected.expected_target_nonce != context.config.target_nonce
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn require_creation_gate(
    proposal: &UpgradeProposalV3,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
) -> ProgramResult {
    if proposal.target_nonce != config.target_nonce
        || proposal.creation_gate_status != gate.status
        || proposal.creation_gate_epoch != gate.epoch
        || gate.active_proposal != Pubkey::default()
        || (gate.status == GateStatusV1::EmergencyFrozen
            && gate.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1)
        || !matches!(
            gate.status,
            GateStatusV1::Active | GateStatusV1::EmergencyFrozen
        )
    {
        return Err(GovernanceError::InvalidProposalEpoch.into());
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
    proposal: &UpgradeProposalV3,
    config: &ControllerConfigV1,
) -> ProgramResult {
    if derive_proposal_timing(config, proposal.proposal_class, proposal.creation_slot)?
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

pub(super) fn require_approval_window(proposal: &UpgradeProposalV3, slot: u64) -> ProgramResult {
    if slot < proposal.review_start_slot
        || slot > proposal.review_end_slot
        || slot >= proposal.expiry_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(())
}

pub(super) fn require_exact_recorded_quorum(
    council: &GovernanceCouncilSetV1,
    bitset: u8,
    count: u8,
    approval_slot: u64,
) -> ProgramResult {
    if bitset & !VALID_APPROVAL_MASK != 0
        || bitset.count_ones() as u8 != count
        || count != RELEASE1_APPROVAL_THRESHOLD
        || !council.active_at(approval_slot)
    {
        return Err(GovernanceError::QuorumNotSatisfied.into());
    }
    for (index, seat) in council.seats.iter().enumerate() {
        if bitset & (1u8 << index) != 0 && !seat.term_covers(approval_slot) {
            return Err(GovernanceError::InactiveCouncilSeat.into());
        }
    }
    Ok(())
}

pub(super) fn is_prefreeze_state(state: ProposalStateV2) -> bool {
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
