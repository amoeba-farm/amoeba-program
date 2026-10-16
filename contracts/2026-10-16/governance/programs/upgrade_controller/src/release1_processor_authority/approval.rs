use super::*;

pub(super) fn derive_major_timing(
    config: &ControllerConfigV1,
    creation_slot: u64,
) -> Result<(u64, u64, u64, u64), ProgramError> {
    derive_major_timing_from_values(
        config.council_review_slots(),
        config.major_delay_slots,
        config.proposal_expiry_slots,
        creation_slot,
    )
}

pub(super) fn derive_major_timing_from_values(
    review_slots: u64,
    major_delay_slots: u64,
    proposal_expiry_slots: u64,
    creation_slot: u64,
) -> Result<(u64, u64, u64, u64), ProgramError> {
    let review_start = creation_slot
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let review_end = review_start
        .checked_add(review_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let not_before = review_end
        .checked_add(major_delay_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let expiry = creation_slot
        .checked_add(proposal_expiry_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if not_before >= expiry {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok((review_start, review_end, not_before, expiry))
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

pub(super) fn reject_guardian(config: &ControllerConfigV1, authority: &Pubkey) -> ProgramResult {
    if *authority == config.guardian {
        Err(GovernanceError::UnknownSeatAuthority.into())
    } else {
        Ok(())
    }
}

pub(super) fn require_approval_window(
    proposal: &TargetAuthorityHandoffProposalV1,
    slot: u64,
) -> ProgramResult {
    if slot < proposal.review_start_slot
        || slot > proposal.review_end_slot
        || slot >= proposal.expiry_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(())
}

pub(super) fn require_approval_window_activation(
    proposal: &BootstrapActivationProposalV1,
    slot: u64,
) -> ProgramResult {
    if slot < proposal.review_start_slot
        || slot > proposal.review_end_slot
        || slot >= proposal.expiry_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(())
}

pub(super) fn require_exact_quorum(
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
