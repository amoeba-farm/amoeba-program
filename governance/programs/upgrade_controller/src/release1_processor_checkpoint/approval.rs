use super::*;

pub(super) fn validate_gate_guard(
    gate: &ProtocolGateV1,
    expected_status: GateStatusV1,
    expected_epoch: u64,
) -> GovernanceResult<()> {
    if gate.status != expected_status || gate.epoch != expected_epoch {
        return Err(GovernanceError::InvalidProposalEpoch);
    }
    Ok(())
}

pub(super) fn validate_current_seat_at_index(
    council: &GovernanceCouncilSetV1,
    seat_index: u8,
    authority: &Pubkey,
    slot: u64,
) -> GovernanceResult<()> {
    let seat = council
        .seats
        .get(usize::from(seat_index))
        .ok_or(GovernanceError::UnknownSeatAuthority)?;
    if seat.seat_authority != *authority {
        return Err(GovernanceError::UnknownSeatAuthority);
    }
    if !council.active_at(slot) || !seat.term_covers(slot) {
        return Err(GovernanceError::InactiveCouncilSeat);
    }
    Ok(())
}

pub(super) fn validate_approval_mask_at(
    council: &GovernanceCouncilSetV1,
    bitset: u8,
    count: u8,
    slot: u64,
) -> GovernanceResult<()> {
    if bitset & !VALID_APPROVAL_MASK != 0 {
        return Err(GovernanceError::InvalidApprovalBitset);
    }
    if bitset.count_ones() as u8 != count {
        return Err(GovernanceError::ApprovalCountMismatch);
    }
    if count != EXACT_ROUTINE_APPROVAL_MASK_COUNT || !council.active_at(slot) {
        return Err(GovernanceError::QuorumNotSatisfied);
    }
    for (index, seat) in council.seats.iter().enumerate() {
        if bitset & (1 << index) != 0 && !seat.term_covers(slot) {
            return Err(GovernanceError::InactiveCouncilSeat);
        }
    }
    Ok(())
}

pub(super) fn validate_checkpoint_council_guard(
    instruction_council_version: u64,
    instruction_council_hash: &[u8; 32],
    council: &GovernanceCouncilSetV1,
) -> GovernanceResult<()> {
    if instruction_council_version != council.version {
        return Err(GovernanceError::StaleCouncilVersion);
    }
    if instruction_council_hash != &council.set_hash {
        return Err(GovernanceError::ProposalCouncilHashMismatch);
    }
    Ok(())
}
