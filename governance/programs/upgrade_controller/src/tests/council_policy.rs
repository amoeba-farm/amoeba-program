use solana_program::{hash::hashv, pubkey::Pubkey};

use crate::{
    council::{
        canonical_council_hash_material, compute_council_set_hash, evaluate_proposal_quorum,
        evaluate_quorum, record_proposal_approval, record_seat_approval,
        validate_council_guardian_separation, validate_council_set, ApprovalRequirementV1,
        COUNCIL_SET_HASH_MATERIAL_LEN, VALID_APPROVAL_MASK,
    },
    policy::{
        canonical_policy_hash_material, compute_policy_hash, validate_policy,
        validate_policy_against_config, vote_requirement_for_class, POLICY_HASH_DOMAIN_V1,
    },
    state::{
        GovernanceCouncilSetV1, GovernancePolicyV1, ProposalClassV1, ProposalStateV1,
        VoteRequirementV1,
    },
    GovernanceError,
};

use super::support::{controller_config, council, key, policy};

const PROPOSAL_CLASSES: [ProposalClassV1; 6] = [
    ProposalClassV1::RoutineUpgrade,
    ProposalClassV1::EmergencyRollback,
    ProposalClassV1::EconomicChange,
    ProposalClassV1::ConstitutionalChange,
    ProposalClassV1::CouncilSetRotation,
    ProposalClassV1::TargetImmutability,
];

fn evaluate(
    mask: u8,
    requirement: ApprovalRequirementV1,
) -> crate::GovernanceResult<crate::council::QuorumEvaluationV1> {
    let policy = policy();
    let council = council();
    evaluate_quorum(
        &council,
        &policy,
        council.version,
        council.version,
        &council.set_hash,
        mask,
        mask.count_ones() as u8,
        500,
        requirement,
    )
}

fn assert_policy_hash_changes(mutator: impl FnOnce(&mut GovernancePolicyV1)) {
    let baseline = policy();
    let baseline_hash = compute_policy_hash(&baseline);
    let mut changed = baseline;
    mutator(&mut changed);
    assert_ne!(compute_policy_hash(&changed), baseline_hash);
}

fn assert_council_hash_changes(mutator: impl FnOnce(&mut GovernanceCouncilSetV1)) {
    let baseline = council();
    let baseline_hash = compute_council_set_hash(&baseline);
    let mut changed = baseline;
    mutator(&mut changed);
    assert_ne!(compute_council_set_hash(&changed), baseline_hash);
}

#[test]
fn canonical_bootstrap_policy_config_and_council_are_valid() {
    let policy = policy();
    let council = council();
    assert_eq!(validate_policy(&policy), Ok(()));
    assert_eq!(
        validate_policy_against_config(&policy, &controller_config()),
        Ok(())
    );
    assert_eq!(validate_council_set(&council, &policy), Ok(()));
    assert_eq!(
        canonical_council_hash_material(&council).len(),
        COUNCIL_SET_HASH_MATERIAL_LEN
    );
    assert_eq!(COUNCIL_SET_HASH_MATERIAL_LEN, 336);
}

#[test]
fn guardian_is_never_a_council_seat_capability() {
    let council = council();
    let guardian = controller_config().guardian;
    assert_eq!(
        validate_council_guardian_separation(&council, &guardian),
        Ok(())
    );

    assert_eq!(
        validate_council_guardian_separation(&council, &Pubkey::default()),
        Err(GovernanceError::InvalidCouncilComposition)
    );

    for seat in &council.seats {
        assert_eq!(
            validate_council_guardian_separation(&council, &seat.seat_authority),
            Err(GovernanceError::InvalidCouncilComposition)
        );
    }
}

#[test]
fn policy_hash_is_sensitive_to_every_material_field() {
    assert_policy_hash_changes(|value| value.controller_config = key(40));
    assert_policy_hash_changes(|value| value.target_program = key(41));
    assert_policy_hash_changes(|value| value.version += 1);
    assert_policy_hash_changes(|value| value.activation_slot += 1);
    assert_policy_hash_changes(|value| value.council_size -= 1);
    assert_policy_hash_changes(|value| value.routine_threshold -= 1);
    assert_policy_hash_changes(|value| value.terminal_threshold -= 1);
    assert_policy_hash_changes(|value| value.policy_flags = 1);
    assert_policy_hash_changes(|value| value.veto_quorum_bps = 1);
    assert_policy_hash_changes(|value| value.affirmative_quorum_bps = 1);
    assert_policy_hash_changes(|value| value.affirmative_approval_bps = 1);
    assert_policy_hash_changes(|value| value.routine_requires_vote = true);
    assert_policy_hash_changes(|value| value.economic_requires_vote = true);
    assert_policy_hash_changes(|value| value.constitutional_requires_vote = true);
    assert_policy_hash_changes(|value| value.rotation_requires_vote = true);
    assert_policy_hash_changes(|value| value.immutability_requires_vote = true);

    // BootstrapCouncilOnly is the sole typed V1 mode, so exercise the encoded
    // mode byte directly to prove that its material position is committed.
    let baseline_material = canonical_policy_hash_material(&policy());
    let baseline_hash = hashv(&[POLICY_HASH_DOMAIN_V1, &baseline_material]).to_bytes();
    let mut changed_material = baseline_material;
    changed_material[83] = 1;
    assert_ne!(
        hashv(&[POLICY_HASH_DOMAIN_V1, &changed_material]).to_bytes(),
        baseline_hash
    );
}

#[test]
fn council_hash_is_sensitive_to_every_material_field() {
    assert_council_hash_changes(|value| value.controller_config = key(40));
    assert_council_hash_changes(|value| value.version += 1);
    assert_council_hash_changes(|value| value.target_program = key(41));
    assert_council_hash_changes(|value| value.activation_slot += 1);
    assert_council_hash_changes(|value| value.deactivation_slot = 9_999);
    for seat_index in 0..5 {
        assert_council_hash_changes(|value| value.seats[seat_index].seat_authority = key(50));
        assert_council_hash_changes(|value| value.seats[seat_index].term_start_slot += 1);
        assert_council_hash_changes(|value| value.seats[seat_index].term_end_slot -= 1);
        assert_council_hash_changes(|value| value.seats[seat_index].active = false);
    }
    assert_council_hash_changes(|value| value.routine_threshold -= 1);
    assert_council_hash_changes(|value| value.terminal_threshold -= 1);
    assert_council_hash_changes(|value| value.policy_flags = 1);
}

#[test]
fn reserved_bytes_are_excluded_from_hashes_but_rejected_by_validation() {
    let canonical_policy = policy();
    let mut changed_policy = canonical_policy.clone();
    changed_policy.reserved[0] = 1;
    assert_eq!(
        compute_policy_hash(&changed_policy),
        compute_policy_hash(&canonical_policy)
    );
    assert_eq!(
        validate_policy(&changed_policy),
        Err(GovernanceError::NonzeroReserved)
    );

    let canonical_council = council();
    let mut changed_council = canonical_council.clone();
    changed_council.reserved[0] = 1;
    assert_eq!(
        compute_council_set_hash(&changed_council),
        compute_council_set_hash(&canonical_council)
    );
    assert_eq!(
        validate_council_set(&changed_council, &canonical_policy),
        Err(GovernanceError::NonzeroReserved)
    );

    let mut changed_seat_reserved = canonical_council.clone();
    changed_seat_reserved.seats[0].reserved[0] = 1;
    assert_eq!(
        compute_council_set_hash(&changed_seat_reserved),
        compute_council_set_hash(&canonical_council)
    );
    assert_eq!(
        validate_council_set(&changed_seat_reserved, &canonical_policy),
        Err(GovernanceError::NonzeroReserved)
    );
}

#[test]
fn all_32_masks_match_equal_vote_routine_and_terminal_predicates() {
    for mask in 0u8..=VALID_APPROVAL_MASK {
        let approvals = mask.count_ones() as u8;
        let routine = evaluate(mask, ApprovalRequirementV1::Routine);
        let terminal = evaluate(mask, ApprovalRequirementV1::Terminal);
        assert_eq!(routine.is_ok(), approvals >= 3, "routine mask {mask:05b}");
        assert_eq!(terminal.is_ok(), approvals >= 4, "terminal mask {mask:05b}");
        if let Ok(result) = routine {
            assert_eq!(result.total_approvals, approvals);
            assert_eq!(result.required_approvals, 3);
        }
        if let Ok(result) = terminal {
            assert_eq!(result.total_approvals, approvals);
            assert_eq!(result.required_approvals, 4);
        }
    }
}

#[test]
fn every_three_equal_seats_pass_routine_and_no_two_seats_do() {
    let mut three_seat_coalitions = 0;
    let mut two_seat_coalitions = 0;
    for mask in 0u8..=VALID_APPROVAL_MASK {
        match mask.count_ones() {
            3 => {
                three_seat_coalitions += 1;
                assert!(evaluate(mask, ApprovalRequirementV1::Routine).is_ok());
            }
            2 => {
                two_seat_coalitions += 1;
                assert_eq!(
                    evaluate(mask, ApprovalRequirementV1::Routine),
                    Err(GovernanceError::QuorumNotSatisfied)
                );
            }
            _ => {}
        }
    }
    assert_eq!(three_seat_coalitions, 10);
    assert_eq!(two_seat_coalitions, 10);
}

#[test]
fn every_four_equal_seats_pass_terminal_and_no_three_seats_do() {
    let mut four_seat_coalitions = 0;
    let mut three_seat_coalitions = 0;
    for mask in 0u8..=VALID_APPROVAL_MASK {
        match mask.count_ones() {
            4 => {
                four_seat_coalitions += 1;
                assert!(evaluate(mask, ApprovalRequirementV1::Terminal).is_ok());
            }
            3 => {
                three_seat_coalitions += 1;
                assert_eq!(
                    evaluate(mask, ApprovalRequirementV1::Terminal),
                    Err(GovernanceError::QuorumNotSatisfied)
                );
            }
            _ => {}
        }
    }
    assert_eq!(four_seat_coalitions, 5);
    assert_eq!(three_seat_coalitions, 10);
}

#[test]
fn invalid_authorities_terms_and_council_policy_fail_closed() {
    let policy = policy();

    let mut value = council();
    value.seats[1].seat_authority = value.seats[0].seat_authority;
    value.set_hash = crate::council::compute_council_set_hash(&value);
    assert_eq!(
        validate_council_set(&value, &policy),
        Err(GovernanceError::DuplicateSeatAuthority)
    );

    let mut value = council();
    value.seats[4].seat_authority = Default::default();
    value.set_hash = crate::council::compute_council_set_hash(&value);
    assert_eq!(
        validate_council_set(&value, &policy),
        Err(GovernanceError::DefaultPubkey)
    );

    let mut value = council();
    value.seats[4].term_end_slot = value.activation_slot;
    value.set_hash = crate::council::compute_council_set_hash(&value);
    assert_eq!(
        validate_council_set(&value, &policy),
        Err(GovernanceError::InactiveCouncilSeat)
    );

    let mut value = council();
    value.policy_flags = 1;
    value.set_hash = crate::council::compute_council_set_hash(&value);
    assert_eq!(
        validate_council_set(&value, &policy),
        Err(GovernanceError::InvalidCouncilThreshold)
    );

    let mut value = council();
    value.set_hash[0] ^= 1;
    assert_eq!(
        validate_council_set(&value, &policy),
        Err(GovernanceError::CouncilHashMismatch)
    );
}

#[test]
fn stale_versions_hashes_and_approval_encoding_fail_closed() {
    let policy = policy();
    let council = council();
    let args = |current_version, proposal_version, hash, bitset, count| {
        evaluate_quorum(
            &council,
            &policy,
            current_version,
            proposal_version,
            hash,
            bitset,
            count,
            500,
            ApprovalRequirementV1::Routine,
        )
    };
    assert_eq!(
        args(
            council.version + 1,
            council.version,
            &council.set_hash,
            0b00111,
            3
        ),
        Err(GovernanceError::StaleCouncilVersion)
    );
    assert_eq!(
        args(
            council.version,
            council.version + 1,
            &council.set_hash,
            0b00111,
            3
        ),
        Err(GovernanceError::StaleCouncilVersion)
    );
    assert_eq!(
        args(council.version, council.version, &[99; 32], 0b00111, 3),
        Err(GovernanceError::ProposalCouncilHashMismatch)
    );
    assert_eq!(
        args(
            council.version,
            council.version,
            &council.set_hash,
            0b1000_0111,
            4
        ),
        Err(GovernanceError::InvalidApprovalBitset)
    );
    assert_eq!(
        args(
            council.version,
            council.version,
            &council.set_hash,
            0b00111,
            2
        ),
        Err(GovernanceError::ApprovalCountMismatch)
    );
}

#[test]
fn duplicate_and_unknown_authority_approvals_do_not_change_state() {
    let council = council();
    let authority = council.seats[2].seat_authority;
    let (bits, count) = record_seat_approval(&council, 0, 0, &authority, 500).unwrap();
    assert_eq!((bits, count), (0b00100, 1));
    assert_eq!(
        record_seat_approval(&council, bits, count, &authority, 500),
        Err(GovernanceError::DuplicateApproval)
    );
    assert_eq!(
        record_seat_approval(&council, bits, count, &key(99), 500),
        Err(GovernanceError::UnknownSeatAuthority)
    );
}

#[test]
fn proposal_approval_is_bound_to_state_digest_policy_and_current_council() {
    let policy = policy();
    let council = council();
    let mut proposal = super::support::proposal();
    proposal.state = ProposalStateV1::BufferVerified;
    proposal.council_version = council.version;
    proposal.council_hash = council.set_hash;
    proposal.proposal_digest = crate::digest::compute_proposal_digest(&proposal).unwrap();
    let authority = council.seats[2].seat_authority;
    assert_eq!(
        record_proposal_approval(
            &council,
            &policy,
            council.version,
            &proposal,
            &authority,
            &proposal.proposal_digest,
            500,
        ),
        Ok((0b00100, 1))
    );
    assert_eq!(
        record_proposal_approval(
            &council,
            &policy,
            council.version,
            &proposal,
            &authority,
            &[99; 32],
            500,
        ),
        Err(GovernanceError::ProposalDigestMismatch)
    );
    assert_eq!(
        record_proposal_approval(
            &council,
            &policy,
            council.version + 1,
            &proposal,
            &authority,
            &proposal.proposal_digest,
            500,
        ),
        Err(GovernanceError::StaleCouncilVersion)
    );
    proposal.state = ProposalStateV1::Draft;
    assert_eq!(
        record_proposal_approval(
            &council,
            &policy,
            council.version,
            &proposal,
            &authority,
            &proposal.proposal_digest,
            500,
        ),
        Err(GovernanceError::InvalidStateTransition)
    );
}

#[test]
fn policy_hash_thresholds_and_token_disabled_form_are_mechanical() {
    let value = policy();
    assert_eq!(validate_policy(&value), Ok(()));
    for class in PROPOSAL_CLASSES {
        assert_eq!(
            vote_requirement_for_class(&value, class),
            Ok(VoteRequirementV1::None)
        );
    }

    let mut invalid = policy();
    invalid.routine_threshold = 2;
    invalid.policy_hash = compute_policy_hash(&invalid);
    assert_eq!(
        validate_policy(&invalid),
        Err(GovernanceError::InvalidPolicy)
    );

    let mut invalid = policy();
    invalid.routine_requires_vote = true;
    invalid.veto_quorum_bps = 1_500;
    invalid.policy_hash = compute_policy_hash(&invalid);
    assert_eq!(
        validate_policy(&invalid),
        Err(GovernanceError::InvalidPolicy)
    );

    let mut invalid = policy();
    invalid.policy_hash[0] ^= 1;
    assert_eq!(
        validate_policy(&invalid),
        Err(GovernanceError::PolicyHashMismatch)
    );

    let mut invalid_config = controller_config();
    invalid_config.token_governance_enabled = true;
    assert_eq!(
        validate_policy_against_config(&value, &invalid_config),
        Err(GovernanceError::InvalidControllerConfig)
    );
}

#[test]
fn proposal_class_selects_routine_or_terminal_without_seat_weighting() {
    let policy = policy();
    let council = council();
    let mut proposal = super::support::proposal();
    proposal.council_version = council.version;
    proposal.council_hash = council.set_hash;
    proposal.council_approval_bitset = 0b00111;
    proposal.council_approval_count = 3;
    proposal.proposal_digest = crate::digest::compute_proposal_digest(&proposal).unwrap();
    for class in PROPOSAL_CLASSES
        .into_iter()
        .filter(|class| *class != ProposalClassV1::TargetImmutability)
    {
        proposal.proposal_class = class;
        proposal.proposal_digest = crate::digest::compute_proposal_digest(&proposal).unwrap();
        assert!(
            evaluate_proposal_quorum(&council, &policy, council.version, &proposal, 500).is_ok(),
            "{class:?} must use routine quorum"
        );
    }

    proposal.proposal_class = ProposalClassV1::TargetImmutability;
    proposal.proposal_digest = crate::digest::compute_proposal_digest(&proposal).unwrap();
    assert_eq!(
        evaluate_proposal_quorum(&council, &policy, council.version, &proposal, 500),
        Err(GovernanceError::QuorumNotSatisfied)
    );
    proposal.council_approval_bitset = 0b10111;
    proposal.council_approval_count = 4;
    proposal.proposal_digest = crate::digest::compute_proposal_digest(&proposal).unwrap();
    assert!(evaluate_proposal_quorum(&council, &policy, council.version, &proposal, 500).is_ok());
}
