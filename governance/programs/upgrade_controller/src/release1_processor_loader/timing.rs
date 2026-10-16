use super::*;

pub(super) fn current_frozen_slot(proposal: &UpgradeProposalV2) -> Result<u64, ProgramError> {
    let slot = Clock::get()?.slot;
    if slot == 0 || slot < proposal.frozen_slot || slot >= proposal.expiry_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(slot)
}

pub(super) fn require_extension_execution_runway(
    proposal: &UpgradeProposalV2,
    slot: u64,
) -> ProgramResult {
    let earliest_upgrade_slot = slot
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if earliest_upgrade_slot >= proposal.expiry_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(())
}

pub(super) fn current_post_execution_frozen_slot(
    proposal: &UpgradeProposalV2,
) -> Result<u64, ProgramError> {
    let slot = Clock::get()?.slot;
    validate_post_execution_frozen_slot(proposal, slot)?;
    Ok(slot)
}

pub(super) fn validate_post_execution_frozen_slot(
    proposal: &UpgradeProposalV2,
    slot: u64,
) -> ProgramResult {
    // Expiry is an admission deadline for crossing the loader boundary. Once
    // an upgrade has executed while timely, mechanical ProgramData checking,
    // checkpoint acceptance, rollback, and governed unfreeze must remain
    // available while the gate stays frozenâ€”even after expiry.
    if slot == 0
        || proposal.frozen_slot == 0
        || proposal.upgrade_executed_slot == 0
        || slot < proposal.frozen_slot
        || slot < proposal.upgrade_executed_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(())
}

fn derive_exact_timing(
    config: &ControllerConfigV1,
    class: ProposalClassV1,
    creation_slot: u64,
) -> Result<(u64, u64, u64, u64), ProgramError> {
    let delay = match class {
        ProposalClassV1::EmergencyRollback => config.rollback_delay_slots,
        ProposalClassV1::RoutineUpgrade => config.routine_delay_slots,
        ProposalClassV1::EconomicChange | ProposalClassV1::ConstitutionalChange => {
            config.major_delay_slots
        }
        ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability => {
            return Err(GovernanceError::UnsupportedProposalClass.into());
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
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok((review_start, review_end, not_before, expiry))
}

pub(super) fn verify_exact_proposal_timing(
    proposal: &UpgradeProposalV2,
    config: &ControllerConfigV1,
) -> ProgramResult {
    if derive_exact_timing(config, proposal.proposal_class, proposal.creation_slot)?
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
