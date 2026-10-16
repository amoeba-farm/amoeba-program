use solana_program::pubkey::Pubkey;

use crate::{
    council::VALID_APPROVAL_MASK,
    digest::compute_proposal_digest,
    policy::{validate_policy, vote_requirement_for_class},
    state::{
        validate_reserved, GovernancePolicyV1, ProposalClassV1, ProposalStateV1, UpgradeProposalV1,
        VoteRequirementV1, ACCOUNT_VERSION_V1, UPGRADE_PROPOSAL_DISCRIMINATOR,
    },
    GovernanceError, GovernanceResult,
};

pub fn validate_proposal_static(proposal: &UpgradeProposalV1) -> GovernanceResult<()> {
    if proposal.discriminator != UPGRADE_PROPOSAL_DISCRIMINATOR {
        return Err(GovernanceError::InvalidDiscriminator);
    }
    if proposal.account_version != ACCOUNT_VERSION_V1 {
        return Err(GovernanceError::UnsupportedVersion);
    }
    if !proposal.initialized {
        return Err(GovernanceError::Uninitialized);
    }
    validate_reserved(&proposal.reserved)?;

    for key in [
        proposal.controller_program,
        proposal.controller_config,
        proposal.protocol_gate,
        proposal.target_program,
        proposal.target_programdata,
        proposal.upgradeable_loader,
        proposal.authority_pda,
        proposal.canonical_spill_treasury,
        proposal.prestate_checkpoint,
        proposal.required_poststate_checkpoint,
    ] {
        require_pubkey(key)?;
    }
    proposal.rollback_proposal.validate()?;
    proposal.rollback_buffer.validate()?;
    if proposal.rollback_proposal.present != proposal.rollback_buffer.present {
        return Err(GovernanceError::InvalidProposalCommitment);
    }
    if proposal.rollback_proposal.present
        != proposal
            .rollback_artifact_hash
            .iter()
            .any(|byte| *byte != 0)
    {
        return Err(GovernanceError::InvalidProposalCommitment);
    }

    let expected_freeze_epoch = proposal
        .creation_gate_epoch
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if proposal.freeze_gate_epoch != expected_freeze_epoch {
        return Err(GovernanceError::InvalidProposalEpoch);
    }
    if proposal.review_start_slot > proposal.review_end_slot
        || proposal.review_end_slot > proposal.not_before_slot
        || proposal.not_before_slot >= proposal.expiry_slot
    {
        return Err(GovernanceError::InvalidProposalTiming);
    }
    let expected_capacity = proposal
        .current_capacity
        .checked_add(proposal.extension_delta)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if expected_capacity != proposal.expected_post_capacity {
        return Err(GovernanceError::InvalidCapacityPlan);
    }

    if proposal.is_code_upgrade() {
        for key in [
            proposal.buffer_pubkey,
            proposal.buffer_loader_owner,
            proposal.buffer_authority,
        ] {
            require_pubkey(key)?;
        }
        if proposal.artifact_sha256 == [0; 32]
            || proposal.artifact_length > proposal.expected_post_capacity
            || proposal.buffer_loader_owner != proposal.upgradeable_loader
            || proposal.buffer_authority != proposal.authority_pda
        {
            return Err(GovernanceError::InvalidProposalCommitment);
        }
    } else if proposal.buffer_pubkey != Pubkey::default()
        || proposal.buffer_loader_owner != Pubkey::default()
        || proposal.buffer_authority != Pubkey::default()
        || proposal.artifact_sha256 != [0; 32]
        || proposal.extension_delta != 0
    {
        return Err(GovernanceError::InvalidProposalCommitment);
    }

    if proposal.vote_requirement != VoteRequirementV1::None
        || proposal.vote_program != Pubkey::default()
        || proposal.vote_result_pda != Pubkey::default()
    {
        return Err(GovernanceError::InvalidProposalCommitment);
    }

    validate_approval_pair(
        proposal.council_approval_bitset,
        proposal.council_approval_count,
    )?;
    validate_approval_pair(
        proposal.poststate_approval_bitset,
        proposal.poststate_approval_count,
    )?;
    validate_approval_pair(
        proposal.unfreeze_approval_bitset,
        proposal.unfreeze_approval_count,
    )?;

    if compute_proposal_digest(proposal)? != proposal.proposal_digest {
        return Err(GovernanceError::ProposalDigestMismatch);
    }
    Ok(())
}

/// Binds the proposal to the exact immutable bootstrap policy instead of
/// accepting a caller-selected vote mode. This permits approval accumulation
/// for every scaffolded class; later transition/execution code still rejects
/// any class whose required evidence graph is not implemented.
pub fn validate_proposal_against_policy(
    proposal: &UpgradeProposalV1,
    policy: &GovernancePolicyV1,
) -> GovernanceResult<()> {
    validate_proposal_static(proposal)?;
    validate_policy(policy)?;
    if proposal.controller_config != policy.controller_config
        || proposal.target_program != policy.target_program
        || proposal.policy_version != policy.version
        || proposal.policy_hash != policy.policy_hash
    {
        return Err(GovernanceError::InvalidProposalCommitment);
    }
    if proposal.vote_requirement != vote_requirement_for_class(policy, proposal.proposal_class)? {
        return Err(GovernanceError::InvalidProposalCommitment);
    }
    Ok(())
}

/// Validates only the shape of the Phase 1 code-upgrade lifecycle without
/// mutating the proposal. This is not transition authorization: the later full
/// lifecycle phase must additionally prove quorum, time, gate, and account
/// evidence. Governance-only class-specific execution paths remain
/// unsupported.
pub fn validate_proposal_transition(
    proposal: &UpgradeProposalV1,
    policy: &GovernancePolicyV1,
    next: ProposalStateV1,
) -> GovernanceResult<()> {
    validate_proposal_against_policy(proposal, policy)?;
    if !proposal.is_code_upgrade()
        || matches!(
            proposal.proposal_class,
            ProposalClassV1::EmergencyRollback
                | ProposalClassV1::CouncilSetRotation
                | ProposalClassV1::TargetImmutability
        )
    {
        return Err(GovernanceError::InvalidStateTransition);
    }
    let current = proposal.state;
    if current == next || current.is_terminal() {
        return Err(GovernanceError::InvalidStateTransition);
    }
    if current == ProposalStateV1::TokenReviewOpen || next == ProposalStateV1::TokenReviewOpen {
        return Err(GovernanceError::InvalidStateTransition);
    }
    if matches!(next, ProposalStateV1::Cancelled | ProposalStateV1::Expired) {
        return if current.is_frozen_or_later() {
            Err(GovernanceError::InvalidStateTransition)
        } else {
            Ok(())
        };
    }
    let allowed = match (current, next) {
        (ProposalStateV1::Draft, ProposalStateV1::BufferAdopted)
        | (ProposalStateV1::BufferAdopted, ProposalStateV1::BufferVerified)
        | (ProposalStateV1::BufferVerified, ProposalStateV1::CouncilApproved)
        | (ProposalStateV1::CouncilApproved, ProposalStateV1::GovernanceSatisfied)
        | (ProposalStateV1::GovernanceSatisfied, ProposalStateV1::Timelocked)
        | (ProposalStateV1::Timelocked, ProposalStateV1::Frozen)
        | (ProposalStateV1::Extended, ProposalStateV1::UpgradeExecuted)
        | (ProposalStateV1::UpgradeExecuted, ProposalStateV1::ProgramDataVerified)
        | (ProposalStateV1::ProgramDataVerified, ProposalStateV1::PoststateAccepted)
        | (ProposalStateV1::PoststateAccepted, ProposalStateV1::UnfreezeApproved)
        | (ProposalStateV1::UnfreezeApproved, ProposalStateV1::Completed) => true,
        (ProposalStateV1::Frozen, ProposalStateV1::Extended) => proposal.extension_required(),
        (ProposalStateV1::Frozen, ProposalStateV1::UpgradeExecuted) => {
            !proposal.extension_required()
        }
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(GovernanceError::InvalidStateTransition)
    }
}

fn validate_approval_pair(bitset: u8, count: u8) -> GovernanceResult<()> {
    if bitset & !VALID_APPROVAL_MASK != 0 {
        return Err(GovernanceError::InvalidApprovalBitset);
    }
    if bitset.count_ones() as u8 != count {
        return Err(GovernanceError::ApprovalCountMismatch);
    }
    Ok(())
}

fn require_pubkey(value: Pubkey) -> GovernanceResult<()> {
    if value == Pubkey::default() {
        Err(GovernanceError::DefaultPubkey)
    } else {
        Ok(())
    }
}
