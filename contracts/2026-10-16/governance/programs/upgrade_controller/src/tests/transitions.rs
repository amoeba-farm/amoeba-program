use crate::{
    proposal::{
        validate_proposal_against_policy, validate_proposal_static, validate_proposal_transition,
    },
    state::{OptionalPubkeyV1, ProposalClassV1, ProposalStateV1, VoteRequirementV1},
    GovernanceError,
};

use super::support::proposal;

const STATES: [ProposalStateV1; 16] = [
    ProposalStateV1::Draft,
    ProposalStateV1::BufferAdopted,
    ProposalStateV1::BufferVerified,
    ProposalStateV1::CouncilApproved,
    ProposalStateV1::TokenReviewOpen,
    ProposalStateV1::GovernanceSatisfied,
    ProposalStateV1::Timelocked,
    ProposalStateV1::Frozen,
    ProposalStateV1::Extended,
    ProposalStateV1::UpgradeExecuted,
    ProposalStateV1::ProgramDataVerified,
    ProposalStateV1::PoststateAccepted,
    ProposalStateV1::UnfreezeApproved,
    ProposalStateV1::Completed,
    ProposalStateV1::Cancelled,
    ProposalStateV1::Expired,
];

fn expected(current: ProposalStateV1, next: ProposalStateV1, extension: bool) -> bool {
    if current == next || current.is_terminal() {
        return false;
    }
    if current == ProposalStateV1::TokenReviewOpen || next == ProposalStateV1::TokenReviewOpen {
        return false;
    }
    if matches!(next, ProposalStateV1::Cancelled | ProposalStateV1::Expired) {
        return !current.is_frozen_or_later();
    }
    match (current, next) {
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
        (ProposalStateV1::Frozen, ProposalStateV1::Extended) => extension,
        (ProposalStateV1::Frozen, ProposalStateV1::UpgradeExecuted) => !extension,
        _ => false,
    }
}

#[test]
fn all_256_state_pairs_match_the_frozen_code_upgrade_graph() {
    let policy = super::support::policy();
    for extension in [false, true] {
        for current in STATES {
            for next in STATES {
                let mut value = proposal();
                value.state = current;
                value.extension_delta = u64::from(extension) * 4_096;
                value.expected_post_capacity = value.current_capacity + value.extension_delta;
                value.proposal_digest = crate::digest::compute_proposal_digest(&value).unwrap();
                let actual = validate_proposal_transition(&value, &policy, next).is_ok();
                assert_eq!(
                    actual,
                    expected(current, next, extension),
                    "extension={extension} {current:?}->{next:?}"
                );
            }
        }
    }
}

#[test]
fn bad_static_epoch_timing_capacity_and_digest_fail() {
    let mut value = proposal();
    value.freeze_gate_epoch += 1;
    value.proposal_digest = crate::digest::compute_proposal_digest(&value).unwrap();
    assert_eq!(
        validate_proposal_static(&value),
        Err(GovernanceError::InvalidProposalEpoch)
    );

    let mut value = proposal();
    value.not_before_slot = value.review_end_slot - 1;
    value.proposal_digest = crate::digest::compute_proposal_digest(&value).unwrap();
    assert_eq!(
        validate_proposal_static(&value),
        Err(GovernanceError::InvalidProposalTiming)
    );

    let mut value = proposal();
    value.expected_post_capacity += 1;
    value.proposal_digest = crate::digest::compute_proposal_digest(&value).unwrap();
    assert_eq!(
        validate_proposal_static(&value),
        Err(GovernanceError::InvalidCapacityPlan)
    );

    let mut value = proposal();
    value.proposal_digest[0] ^= 1;
    assert_eq!(
        validate_proposal_static(&value),
        Err(GovernanceError::ProposalDigestMismatch)
    );
}

#[test]
fn class_policy_cannot_be_selected_by_the_caller() {
    let policy = super::support::policy();
    let mut value = proposal();
    value.controller_config = policy.controller_config;
    value.target_program = policy.target_program;
    value.policy_version = policy.version;
    value.policy_hash = policy.policy_hash;
    value.proposal_digest = crate::digest::compute_proposal_digest(&value).unwrap();
    assert_eq!(validate_proposal_against_policy(&value, &policy), Ok(()));

    value.vote_program = super::support::key(10);
    value.proposal_digest = crate::digest::compute_proposal_digest(&value).unwrap();
    assert_eq!(
        validate_proposal_against_policy(&value, &policy),
        Err(GovernanceError::InvalidProposalCommitment)
    );

    value.vote_requirement = VoteRequirementV1::Veto;
    value.vote_result_pda = super::support::key(18);
    value.proposal_digest = crate::digest::compute_proposal_digest(&value).unwrap();
    assert_eq!(
        validate_proposal_against_policy(&value, &policy),
        Err(GovernanceError::InvalidProposalCommitment)
    );

    value.proposal_class = ProposalClassV1::EconomicChange;
    value.vote_requirement = VoteRequirementV1::None;
    value.vote_program = Default::default();
    value.vote_result_pda = Default::default();
    value.proposal_digest = crate::digest::compute_proposal_digest(&value).unwrap();
    assert_eq!(validate_proposal_against_policy(&value, &policy), Ok(()));

    value.proposal_class = ProposalClassV1::EmergencyRollback;
    value.vote_requirement = VoteRequirementV1::None;
    value.vote_result_pda = Default::default();
    value.proposal_digest = crate::digest::compute_proposal_digest(&value).unwrap();
    assert_eq!(validate_proposal_against_policy(&value, &policy), Ok(()));
    value.rollback_proposal = OptionalPubkeyV1::some(super::support::key(90)).unwrap();
    value.rollback_buffer = OptionalPubkeyV1::some(super::support::key(91)).unwrap();
    value.rollback_artifact_hash = [92; 32];
    value.proposal_digest = crate::digest::compute_proposal_digest(&value).unwrap();
    assert_eq!(validate_proposal_against_policy(&value, &policy), Ok(()));
}

#[test]
fn incomplete_governance_only_class_graphs_are_explicitly_unsupported() {
    let policy = super::support::policy();
    for class in [
        ProposalClassV1::CouncilSetRotation,
        ProposalClassV1::TargetImmutability,
    ] {
        let mut value = proposal();
        value.proposal_class = class;
        value.proposal_digest = crate::digest::compute_proposal_digest(&value).unwrap();
        assert_eq!(
            validate_proposal_transition(&value, &policy, ProposalStateV1::BufferAdopted),
            Err(GovernanceError::InvalidStateTransition)
        );
    }
}
