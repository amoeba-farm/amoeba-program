use borsh::to_vec;
use solana_program::pubkey::Pubkey;

use crate::{
    release1_model::{
        model_council_hash, proposal_edge_allowed, routine_mask_satisfied, terminal_mask_satisfied,
        ModelDelays, ModelGuardianProgramDataObservation, ModelHardStateObservation,
        ModelIdentityGraph, ModelInitialization, ModelProgramDataFailureKind,
        ModelProgramDataObservation, ModelProposalRequest, ModelSeatTerm, Release1Model,
        Release1ModelAction, Release1ModelError, Release1ModelOutcome,
        MODEL_EXPIRED_TERMINAL_REASON, MODEL_ROUTINE_THRESHOLD, MODEL_TERMINAL_THRESHOLD,
    },
    release1_state::{
        CouncilRotationStateV1, EmergencyFreezeResolutionStateV1, ProposalStateV2,
        StateCheckpointPhaseV1, BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
        COUNCIL_ROTATION_ACTIVATED_TERMINAL_REASON_V1, COUNCIL_ROTATION_EXPIRED_TERMINAL_REASON_V1,
        EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V1,
        EMERGENCY_RESOLUTION_EXPIRED_TERMINAL_REASON_V1, LOADER_V3_PROGRAMDATA_METADATA_LEN_V1,
        MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
    },
    state::{GateStatusV1, ProposalClassV1},
};

const INITIALIZATION_SLOT: u64 = 10;
const PROPOSAL_SLOT: u64 = 20;
const PRE_HANDOFF_TARGET_AUTHORITY: u8 = 12;

fn key(value: u8) -> Pubkey {
    Pubkey::new_from_array([value; 32])
}

fn seats(first: u8) -> [Pubkey; 5] {
    [
        key(first),
        key(first + 1),
        key(first + 2),
        key(first + 3),
        key(first + 4),
    ]
}

fn seat_terms(start_slot: u64, end_slot: u64) -> [ModelSeatTerm; 5] {
    [ModelSeatTerm {
        start_slot,
        end_slot,
    }; 5]
}

fn graph() -> ModelIdentityGraph {
    ModelIdentityGraph {
        controller_program: key(1),
        controller_programdata: key(2),
        target_program: key(3),
        target_programdata: key(4),
        upgradeable_loader: key(5),
        authority_pda: key(6),
        gate_pda: key(7),
        policy_pda: key(8),
        council_pda: key(9),
        canonical_spill_treasury: key(10),
        guardian: key(11),
    }
}

fn delays() -> ModelDelays {
    ModelDelays {
        review_slots: 5,
        rollback_slots: 7,
        routine_slots: 10,
        major_slots: 20,
        terminal_slots: 30,
        proposal_expiry_slots: 100,
    }
}

fn observation(authority: Pubkey) -> ModelProgramDataObservation {
    ModelProgramDataObservation {
        slot: 7,
        payload_hash: [41; 32],
        raw_hash: [42; 32],
        capacity: 1_572_864,
        authority,
    }
}

fn canonical_guardian_observation(
    observation: &ModelProgramDataObservation,
) -> ModelGuardianProgramDataObservation {
    ModelGuardianProgramDataObservation::canonical(
        &graph(),
        observation,
        Some(observation.authority),
    )
    .expect("canonical guardian observation")
}

fn hard_state(seed: u8) -> ModelHardStateObservation {
    let mut observation = ModelHardStateObservation {
        schema_identifier: [seed; 32],
        program_owned_root: [seed.wrapping_add(1); 32],
        program_owned_count: 17,
        logical_compressed_root: [seed.wrapping_add(2); 32],
        logical_compressed_count: 23,
        semantic_custody_root: [seed.wrapping_add(3); 32],
        custody_identity_root: [seed.wrapping_add(4); 32],
        hard_combined_root: [0; 32],
    };
    observation.hard_combined_root = observation.recompute_hard_combined_root();
    observation
}

fn initialization_with_delays(configured_delays: ModelDelays) -> ModelInitialization {
    let identities = graph();
    ModelInitialization {
        slot: INITIALIZATION_SLOT,
        seats: seats(20),
        seat_terms: seat_terms(1, 10_000),
        programdata: observation(key(PRE_HANDOFF_TARGET_AUTHORITY)),
        graph: identities,
        delays: configured_delays,
        controller_programdata_linked: true,
        initializer_is_controller_upgrade_authority: true,
        target_programdata_linked: true,
        canonical_pdas_verified: true,
        seat_accounts_readonly: true,
        seat_accounts_nonexecutable: true,
    }
}

fn initialized_model_with_delays(configured_delays: ModelDelays) -> Release1Model {
    let mut model = Release1Model::default();
    assert_eq!(
        model
            .apply(Release1ModelAction::Initialize(Box::new(
                initialization_with_delays(configured_delays),
            )))
            .expect("valid model initialization"),
        Release1ModelOutcome::Applied
    );
    model
}

fn initialized_model() -> Release1Model {
    initialized_model_with_delays(delays())
}

fn simulate_post_handoff_activation(model: &mut Release1Model) {
    let controller_authority = model.graph.authority_pda;
    apply_ok(
        model,
        Release1ModelAction::SetProgramDataObservation(observation(controller_authority)),
    );
    apply_ok(
        model,
        Release1ModelAction::SimulatePostHandoffActivation {
            slot: INITIALIZATION_SLOT + 1,
            bridge_and_authority_graph_verified: true,
        },
    );
}

fn active_model_with_delays(configured_delays: ModelDelays) -> Release1Model {
    let mut model = initialized_model_with_delays(configured_delays);
    simulate_post_handoff_activation(&mut model);
    model
}

fn active_model() -> Release1Model {
    active_model_with_delays(delays())
}

fn apply_ok(model: &mut Release1Model, action: Release1ModelAction) {
    assert_eq!(
        model.apply(action).expect("model action must succeed"),
        Release1ModelOutcome::Applied
    );
}

fn assert_atomic_error(
    model: &mut Release1Model,
    action: Release1ModelAction,
    expected: Release1ModelError,
) {
    let before_model = model.clone();
    let before_bytes = to_vec(&*model).expect("serialize model before failure");
    assert_eq!(
        model.apply(action).expect_err("model action must fail"),
        expected
    );
    assert_eq!(*model, before_model);
    assert_eq!(
        to_vec(&*model).expect("serialize model after failure"),
        before_bytes
    );
}

fn create_request(
    class: ProposalClassV1,
    extension_required: bool,
    primary_proposal: Option<u64>,
    rollback_proposal: Option<u64>,
    slot: u64,
) -> ModelProposalRequest {
    ModelProposalRequest {
        class,
        extension_required,
        primary_proposal,
        rollback_proposal,
        proposer_seat: 0,
        proposer_signed: true,
        payer_is_separate: true,
        slot,
    }
}

fn create_proposal(model: &mut Release1Model, request: ModelProposalRequest) -> u64 {
    match model
        .apply(Release1ModelAction::CreateProposal(request))
        .expect("proposal creation")
    {
        Release1ModelOutcome::ProposalCreated(id) => id,
        outcome => panic!("unexpected proposal outcome: {outcome:?}"),
    }
}

fn create_pair(
    model: &mut Release1Model,
    primary_class: ProposalClassV1,
    extension_required: bool,
    slot: u64,
) -> (u64, u64) {
    assert_ne!(primary_class, ProposalClassV1::EmergencyRollback);
    let primary_id = model.next_proposal_id;
    let rollback_id = primary_id.checked_add(1).expect("test proposal id");
    let rollback_creation_slot = slot
        .checked_add(model.delays.rollback_slots)
        .and_then(|value| value.checked_add(1))
        .expect("test rollback creation slot");
    let rollback_proposer_seat = model
        .council
        .seat_terms
        .iter()
        .position(|term| {
            term.start_slot <= rollback_creation_slot && rollback_creation_slot < term.end_slot
        })
        .expect("active rollback proposer seat") as u8;
    assert_eq!(
        create_proposal(
            model,
            create_request(
                primary_class,
                extension_required,
                None,
                Some(rollback_id),
                slot,
            ),
        ),
        primary_id
    );
    assert_eq!(
        create_proposal(model, {
            let mut request = create_request(
                ProposalClassV1::EmergencyRollback,
                false,
                Some(primary_id),
                None,
                rollback_creation_slot,
            );
            request.proposer_seat = rollback_proposer_seat;
            request
        }),
        rollback_id
    );
    (primary_id, rollback_id)
}

fn prepare_proposal(model: &mut Release1Model, proposal_id: u64) {
    apply_ok(model, Release1ModelAction::AdoptBuffer { proposal_id });
    apply_ok(model, Release1ModelAction::VerifyBuffer { proposal_id });
    let timing = model.proposals[&proposal_id].timing;
    for seat in 0..3 {
        apply_ok(
            model,
            Release1ModelAction::ApproveProposal {
                proposal_id,
                seat,
                slot: timing.review_start_slot,
            },
        );
    }
    assert_eq!(
        model.proposals[&proposal_id].state,
        ProposalStateV2::CouncilApproved
    );
    apply_ok(
        model,
        Release1ModelAction::SatisfyGovernance {
            proposal_id,
            slot: timing.review_end_slot,
        },
    );
    apply_ok(
        model,
        Release1ModelAction::QueueProposal {
            proposal_id,
            slot: timing.review_end_slot,
        },
    );
}

fn freeze_prepared_primary(model: &mut Release1Model, primary_id: u64) -> u64 {
    let slot = model.proposals[&primary_id].timing.not_before_slot;
    apply_ok(
        model,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary_id,
            slot,
        },
    );
    slot
}

fn primary_execution_slot(model: &Release1Model, primary_id: u64, minimum_slot: u64) -> u64 {
    let rollback_id = model.proposals[&primary_id]
        .rollback_proposal
        .expect("primary rollback proposal");
    minimum_slot.max(model.proposals[&rollback_id].timing.not_before_slot)
}

fn approve_and_finalize_checkpoint(
    model: &mut Release1Model,
    proposal_id: u64,
    phase: StateCheckpointPhaseV1,
    slot: u64,
) {
    for seat in 0..3 {
        apply_ok(
            model,
            Release1ModelAction::AttestCheckpoint {
                proposal_id,
                phase,
                seat,
                hard_state: hard_state(1),
                forbidden_drift_count: 0,
                slot,
            },
        );
    }
    let canonical = match phase {
        StateCheckpointPhaseV1::Prestate => &model.proposals[&proposal_id].prestate,
        StateCheckpointPhaseV1::Poststate => &model.proposals[&proposal_id].poststate,
        StateCheckpointPhaseV1::Emergency => panic!("use emergency checkpoint helper"),
    };
    assert!(
        canonical.is_none(),
        "attestations do not squat the canonical PDA"
    );
    apply_ok(
        model,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id,
            phase,
            attesting_seats: [0, 1, 2],
            slot: slot + 1,
        },
    );
}

fn create_candidate_council(
    model: &mut Release1Model,
    candidate_version: u64,
    candidate_seats: [Pubkey; 5],
    slot: u64,
) -> u64 {
    let activation_slot = slot
        .checked_add(model.delays.major_slots)
        .expect("candidate activation slot");
    match model
        .apply(Release1ModelAction::CreateCandidateCouncilSet {
            candidate_version,
            candidate_seats,
            candidate_seat_terms: seat_terms(1, 10_000),
            activation_slot,
            slot,
        })
        .expect("candidate council creation")
    {
        Release1ModelOutcome::CandidateCouncilCreated(version) => version,
        outcome => panic!("unexpected candidate council outcome: {outcome:?}"),
    }
}

fn create_rotation(model: &mut Release1Model, candidate_seats: [Pubkey; 5], slot: u64) -> u64 {
    let candidate_version = model.next_candidate_council_version;
    assert_eq!(
        create_candidate_council(model, candidate_version, candidate_seats, slot),
        candidate_version
    );
    match model
        .apply(Release1ModelAction::CreateCouncilRotation {
            candidate_version,
            slot,
        })
        .expect("rotation creation")
    {
        Release1ModelOutcome::CouncilRotationCreated(id) => id,
        outcome => panic!("unexpected rotation outcome: {outcome:?}"),
    }
}

fn approve_and_queue_rotation(model: &mut Release1Model, rotation_id: u64, slot: u64) {
    for seat in 0..3 {
        apply_ok(
            model,
            Release1ModelAction::ApproveCouncilRotation {
                rotation_id,
                seat,
                slot,
            },
        );
    }
    apply_ok(
        model,
        Release1ModelAction::QueueCouncilRotation { rotation_id, slot },
    );
}

fn create_and_activate_rotation(
    model: &mut Release1Model,
    candidate_seats: [Pubkey; 5],
    slot: u64,
) -> u64 {
    let rotation_id = create_rotation(model, candidate_seats, slot);
    approve_and_queue_rotation(model, rotation_id, slot);
    let not_before = model.rotations[&rotation_id].not_before_slot;
    apply_ok(
        model,
        Release1ModelAction::ActivateCouncilRotation {
            rotation_id,
            slot: not_before,
        },
    );
    rotation_id
}

#[test]
fn initialization_is_exact_frozen_and_failure_atomic() {
    let mut empty = Release1Model::default();
    assert_atomic_error(
        &mut empty,
        Release1ModelAction::EnableTokenGovernance,
        Release1ModelError::NotInitialized,
    );

    let valid = initialization_with_delays(delays());
    let mut invalid_cases = Vec::new();
    let mut invalid = valid.clone();
    invalid.slot = 0;
    invalid_cases.push((invalid, Release1ModelError::InvalidInitialization));
    for evidence_index in 0..6 {
        let mut invalid = valid.clone();
        match evidence_index {
            0 => invalid.controller_programdata_linked = false,
            1 => invalid.initializer_is_controller_upgrade_authority = false,
            2 => invalid.target_programdata_linked = false,
            3 => invalid.canonical_pdas_verified = false,
            4 => invalid.seat_accounts_readonly = false,
            5 => invalid.seat_accounts_nonexecutable = false,
            _ => unreachable!(),
        }
        invalid_cases.push((invalid, Release1ModelError::InvalidInitialization));
    }
    let mut invalid = valid.clone();
    invalid.graph.guardian = invalid.graph.gate_pda;
    invalid_cases.push((invalid, Release1ModelError::InvalidInitialization));
    let mut invalid = valid.clone();
    invalid.seats[4] = invalid.seats[0];
    invalid_cases.push((invalid, Release1ModelError::InvalidCouncil));
    let mut invalid = valid.clone();
    invalid.delays.review_slots = 0;
    invalid_cases.push((invalid, Release1ModelError::InvalidTiming));
    let mut invalid = valid.clone();
    invalid.programdata.payload_hash = [0; 32];
    invalid_cases.push((invalid, Release1ModelError::InvalidInitialization));
    let mut invalid = valid.clone();
    invalid.programdata.authority = invalid.graph.authority_pda;
    invalid_cases.push((invalid, Release1ModelError::InvalidInitialization));

    for (invalid, error) in invalid_cases {
        let mut model = Release1Model::default();
        assert_atomic_error(
            &mut model,
            Release1ModelAction::Initialize(Box::new(invalid)),
            error,
        );
    }

    let mut overflow_delays = delays();
    overflow_delays.review_slots = u64::MAX;
    let mut overflow_model = Release1Model::default();
    assert_atomic_error(
        &mut overflow_model,
        Release1ModelAction::Initialize(Box::new(initialization_with_delays(overflow_delays))),
        Release1ModelError::ArithmeticOverflow,
    );

    let mut model = initialized_model();
    assert!(model.initialized);
    assert_eq!(model.next_proposal_id, 1);
    assert_eq!(model.target_nonce, 1);
    assert_eq!(model.council.version, 1);
    assert_eq!(model.council.seats, seats(20));
    assert_eq!(model.gate.status, GateStatusV1::EmergencyFrozen);
    assert_eq!(model.gate.epoch, 1);
    assert_eq!(
        model.gate.freeze_reason,
        BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
    );
    assert!(!model.token_governance_enabled);
    assert!(!model.target_immutability_enabled);
    assert_eq!(
        model.programdata.authority,
        key(PRE_HANDOFF_TARGET_AUTHORITY)
    );
    assert_eq!(
        model.current_programdata_authority,
        Some(key(PRE_HANDOFF_TARGET_AUTHORITY))
    );
    assert_ne!(model.programdata.authority, model.graph.authority_pda);

    assert_atomic_error(
        &mut model,
        Release1ModelAction::Initialize(Box::new(initialization_with_delays(delays()))),
        Release1ModelError::AlreadyInitialized,
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::SimulatePostHandoffActivation {
            slot: INITIALIZATION_SLOT + 1,
            bridge_and_authority_graph_verified: false,
        },
        Release1ModelError::InvalidGate,
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::SimulatePostHandoffActivation {
            slot: INITIALIZATION_SLOT + 1,
            bridge_and_authority_graph_verified: true,
        },
        Release1ModelError::InvalidGate,
    );

    let controller_authority = model.graph.authority_pda;
    apply_ok(
        &mut model,
        Release1ModelAction::SetProgramDataAuthorityObservation(Some(controller_authority)),
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::SimulatePostHandoffActivation {
            slot: INITIALIZATION_SLOT + 1,
            bridge_and_authority_graph_verified: true,
        },
        Release1ModelError::InvalidGate,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::SetProgramDataObservation(observation(controller_authority)),
    );
    apply_ok(
        &mut model,
        Release1ModelAction::SimulatePostHandoffActivation {
            slot: INITIALIZATION_SLOT + 1,
            bridge_and_authority_graph_verified: true,
        },
    );
    assert_eq!(model.gate.status, GateStatusV1::Active);
    assert_eq!(model.gate.epoch, 2);
    assert_eq!(model.gate.freeze_slot, 0);
    assert_eq!(model.gate.freeze_reason, 0);
}

#[test]
fn all_five_seat_masks_have_exact_routine_and_reserved_terminal_semantics() {
    assert_eq!(MODEL_ROUTINE_THRESHOLD, 3);
    assert_eq!(MODEL_TERMINAL_THRESHOLD, 4);
    for mask in 0u8..32 {
        assert_eq!(
            routine_mask_satisfied(mask).expect("valid five-seat mask"),
            mask.count_ones() >= 3,
            "routine mask {mask:05b}"
        );
        assert_eq!(
            terminal_mask_satisfied(mask).expect("valid five-seat mask"),
            mask.count_ones() >= 4,
            "terminal mask {mask:05b}"
        );
    }
    for invalid in [32, 64, 128, 255] {
        assert_eq!(
            routine_mask_satisfied(invalid),
            Err(Release1ModelError::InvalidApproval)
        );
        assert_eq!(
            terminal_mask_satisfied(invalid),
            Err(Release1ModelError::InvalidApproval)
        );
    }
}

#[test]
fn council_terms_hash_proposer_authority_and_completed_quorum_expiry_are_mechanical() {
    let authorities = seats(20);
    let terms = seat_terms(1, 10_000);
    let canonical_hash = model_council_hash(1, INITIALIZATION_SLOT, &authorities, &terms);
    let mut changed_terms = terms;
    changed_terms[2].end_slot -= 1;
    assert_ne!(
        canonical_hash,
        model_council_hash(1, INITIALIZATION_SLOT, &authorities, &changed_terms)
    );
    assert_ne!(
        canonical_hash,
        model_council_hash(1, INITIALIZATION_SLOT + 1, &authorities, &terms)
    );

    let mut invalid_initialization = initialization_with_delays(delays());
    invalid_initialization.seat_terms[0].end_slot = INITIALIZATION_SLOT;
    let mut invalid_model = Release1Model::default();
    assert_atomic_error(
        &mut invalid_model,
        Release1ModelAction::Initialize(Box::new(invalid_initialization)),
        Release1ModelError::InactiveSeat,
    );

    let mut model = active_model();
    let mut unsigned = create_request(
        ProposalClassV1::RoutineUpgrade,
        false,
        None,
        Some(2),
        PROPOSAL_SLOT,
    );
    unsigned.proposer_signed = false;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::CreateProposal(unsigned),
        Release1ModelError::InvalidProposer,
    );
    let mut shared_payer = unsigned;
    shared_payer.proposer_signed = true;
    shared_payer.payer_is_separate = false;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::CreateProposal(shared_payer),
        Release1ModelError::InvalidProposer,
    );
    let mut unknown_seat = shared_payer;
    unknown_seat.payer_is_separate = true;
    unknown_seat.proposer_seat = 5;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::CreateProposal(unknown_seat),
        Release1ModelError::InvalidProposer,
    );

    let mut expiring_initialization = initialization_with_delays(delays());
    expiring_initialization.seat_terms[0].end_slot = PROPOSAL_SLOT + 6;
    let mut expiring = Release1Model::default();
    apply_ok(
        &mut expiring,
        Release1ModelAction::Initialize(Box::new(expiring_initialization)),
    );
    simulate_post_handoff_activation(&mut expiring);
    let (primary, _) = create_pair(
        &mut expiring,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    apply_ok(
        &mut expiring,
        Release1ModelAction::AdoptBuffer {
            proposal_id: primary,
        },
    );
    apply_ok(
        &mut expiring,
        Release1ModelAction::VerifyBuffer {
            proposal_id: primary,
        },
    );
    let timing = expiring.proposals[&primary].timing;
    for seat in 0..3 {
        apply_ok(
            &mut expiring,
            Release1ModelAction::ApproveProposal {
                proposal_id: primary,
                seat,
                slot: timing.review_start_slot,
            },
        );
    }
    apply_ok(
        &mut expiring,
        Release1ModelAction::SatisfyGovernance {
            proposal_id: primary,
            slot: timing.review_end_slot,
        },
    );
    apply_ok(
        &mut expiring,
        Release1ModelAction::QueueProposal {
            proposal_id: primary,
            slot: timing.review_end_slot,
        },
    );
    assert_eq!(
        expiring.proposals[&primary].state,
        ProposalStateV2::Timelocked,
        "a completed creation-council quorum remains durable after an approving seat term ends"
    );

    let mut incomplete_initialization = initialization_with_delays(delays());
    incomplete_initialization.seat_terms[2].end_slot = PROPOSAL_SLOT + 6;
    let mut incomplete = Release1Model::default();
    apply_ok(
        &mut incomplete,
        Release1ModelAction::Initialize(Box::new(incomplete_initialization)),
    );
    simulate_post_handoff_activation(&mut incomplete);
    let (incomplete_primary, _) = create_pair(
        &mut incomplete,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    apply_ok(
        &mut incomplete,
        Release1ModelAction::AdoptBuffer {
            proposal_id: incomplete_primary,
        },
    );
    apply_ok(
        &mut incomplete,
        Release1ModelAction::VerifyBuffer {
            proposal_id: incomplete_primary,
        },
    );
    let incomplete_timing = incomplete.proposals[&incomplete_primary].timing;
    for seat in 0..2 {
        apply_ok(
            &mut incomplete,
            Release1ModelAction::ApproveProposal {
                proposal_id: incomplete_primary,
                seat,
                slot: incomplete_timing.review_start_slot,
            },
        );
    }
    assert_atomic_error(
        &mut incomplete,
        Release1ModelAction::ApproveProposal {
            proposal_id: incomplete_primary,
            seat: 2,
            slot: incomplete_timing.review_end_slot,
        },
        Release1ModelError::InactiveSeat,
    );
    assert_eq!(
        incomplete.proposals[&incomplete_primary].state,
        ProposalStateV2::BufferVerified
    );
    assert_eq!(
        incomplete.proposals[&incomplete_primary]
            .initial_approvals
            .count,
        2
    );
}

#[test]
fn proposal_classes_ids_timelocks_and_unsupported_features_are_exact() {
    let configured = delays();
    assert_eq!(
        configured
            .class_delay(ProposalClassV1::EmergencyRollback)
            .unwrap(),
        configured.rollback_slots
    );
    assert_eq!(
        configured
            .class_delay(ProposalClassV1::RoutineUpgrade)
            .unwrap(),
        configured.routine_slots
    );
    for class in [
        ProposalClassV1::EconomicChange,
        ProposalClassV1::ConstitutionalChange,
    ] {
        assert_eq!(
            configured.class_delay(class).unwrap(),
            configured.major_slots
        );
    }
    for class in [
        ProposalClassV1::CouncilSetRotation,
        ProposalClassV1::TargetImmutability,
    ] {
        assert_eq!(
            configured.class_delay(class),
            Err(Release1ModelError::UnsupportedProposalClass)
        );
    }

    for (class, expected_delay) in [
        (ProposalClassV1::RoutineUpgrade, configured.routine_slots),
        (ProposalClassV1::EconomicChange, configured.major_slots),
        (
            ProposalClassV1::ConstitutionalChange,
            configured.major_slots,
        ),
    ] {
        let mut model = active_model();
        let (primary, rollback) = create_pair(&mut model, class, false, PROPOSAL_SLOT);
        assert_eq!((primary, rollback), (1, 2));
        assert_eq!(model.next_proposal_id, 3);
        for proposal in model.proposals.values() {
            assert_eq!(proposal.target_nonce, 1);
            assert_eq!(proposal.creation_gate_epoch, 2);
            assert_eq!(proposal.creation_council_version, 1);
        }
        let timing = model.proposals[&primary].timing;
        assert_eq!(timing.review_start_slot, PROPOSAL_SLOT + 1);
        assert_eq!(
            timing.review_end_slot,
            timing.review_start_slot + configured.review_slots
        );
        assert_eq!(
            timing.not_before_slot,
            timing.review_end_slot + expected_delay
        );
        assert_eq!(
            timing.expiry_slot,
            PROPOSAL_SLOT + configured.proposal_expiry_slots
        );
        let rollback_timing = model.proposals[&rollback].timing;
        assert_eq!(
            rollback_timing.not_before_slot,
            rollback_timing.review_end_slot + configured.rollback_slots
        );
        assert!(
            rollback_timing.expiry_slot > timing.expiry_slot + configured.rollback_slots,
            "the precommitted rollback must outlive the full primary window plus rollback delay"
        );
    }

    let mut model = active_model();
    let rollback_id = create_proposal(
        &mut model,
        create_request(
            ProposalClassV1::EmergencyRollback,
            false,
            Some(777),
            None,
            PROPOSAL_SLOT,
        ),
    );
    assert_eq!(rollback_id, 1);
    assert_eq!(
        model.proposals[&rollback_id].timing.not_before_slot,
        PROPOSAL_SLOT + 1 + delays().review_slots + delays().rollback_slots
    );

    for class in [
        ProposalClassV1::CouncilSetRotation,
        ProposalClassV1::TargetImmutability,
    ] {
        assert_atomic_error(
            &mut model,
            Release1ModelAction::CreateProposal(create_request(
                class,
                false,
                None,
                Some(99),
                PROPOSAL_SLOT,
            )),
            Release1ModelError::UnsupportedProposalClass,
        );
    }
    assert_atomic_error(
        &mut model,
        Release1ModelAction::EnableTokenGovernance,
        Release1ModelError::TokenGovernanceDisabled,
    );

    model.next_proposal_id = u64::MAX;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::CreateProposal(create_request(
            ProposalClassV1::RoutineUpgrade,
            false,
            None,
            Some(1),
            PROPOSAL_SLOT,
        )),
        Release1ModelError::ArithmeticOverflow,
    );
    model.next_proposal_id = 2;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::CreateProposal(create_request(
            ProposalClassV1::RoutineUpgrade,
            false,
            None,
            Some(2),
            u64::MAX,
        )),
        Release1ModelError::InvalidProposer,
    );

    let mut expiry_overflow_delays = delays();
    expiry_overflow_delays.proposal_expiry_slots = u64::MAX;
    let mut expiry_overflow = active_model_with_delays(expiry_overflow_delays);
    assert_atomic_error(
        &mut expiry_overflow,
        Release1ModelAction::CreateProposal(create_request(
            ProposalClassV1::RoutineUpgrade,
            false,
            None,
            Some(2),
            PROPOSAL_SLOT,
        )),
        Release1ModelError::ArithmeticOverflow,
    );
}

#[test]
fn approval_windows_quorum_duplicates_and_immutable_timing_are_enforced() {
    let mut model = active_model();
    let (primary, rollback) = create_pair(
        &mut model,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::AdoptBuffer {
            proposal_id: primary,
        },
    );
    apply_ok(
        &mut model,
        Release1ModelAction::VerifyBuffer {
            proposal_id: primary,
        },
    );
    let timing = model.proposals[&primary].timing;
    assert_eq!(model.proposals[&primary].freeze_gate_epoch, 0);
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ApproveProposal {
            proposal_id: primary,
            seat: 0,
            slot: timing.review_start_slot - 1,
        },
        Release1ModelError::TimingViolation,
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ApproveProposal {
            proposal_id: primary,
            seat: 5,
            slot: timing.review_start_slot,
        },
        Release1ModelError::InvalidApproval,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ApproveProposal {
            proposal_id: primary,
            seat: 0,
            slot: timing.review_start_slot,
        },
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ApproveProposal {
            proposal_id: primary,
            seat: 0,
            slot: timing.review_start_slot,
        },
        Release1ModelError::DuplicateApproval,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ApproveProposal {
            proposal_id: primary,
            seat: 1,
            slot: timing.review_end_slot,
        },
    );
    assert_eq!(
        model.proposals[&primary].state,
        ProposalStateV2::BufferVerified
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ApproveProposal {
            proposal_id: primary,
            seat: 2,
            slot: timing.review_end_slot + 1,
        },
        Release1ModelError::TimingViolation,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ApproveProposal {
            proposal_id: primary,
            seat: 2,
            slot: timing.review_end_slot,
        },
    );
    assert_eq!(model.proposals[&primary].initial_approvals.count, 3);
    assert_eq!(
        model.proposals[&primary].state,
        ProposalStateV2::CouncilApproved
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ApproveProposal {
            proposal_id: primary,
            seat: 3,
            slot: timing.review_end_slot,
        },
        Release1ModelError::TimingViolation,
    );

    let committed_timing = model.proposals[&primary].timing;
    apply_ok(
        &mut model,
        Release1ModelAction::SatisfyGovernance {
            proposal_id: primary,
            slot: timing.review_end_slot,
        },
    );
    apply_ok(
        &mut model,
        Release1ModelAction::QueueProposal {
            proposal_id: primary,
            slot: timing.review_end_slot,
        },
    );
    assert_eq!(model.proposals[&primary].timing, committed_timing);
    assert_atomic_error(
        &mut model,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary,
            slot: timing.not_before_slot - 1,
        },
        Release1ModelError::TimingViolation,
    );

    prepare_proposal(&mut model, rollback);
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ConvertEmergencyFreeze {
            proposal_id: primary,
            slot: timing.not_before_slot,
        },
        Release1ModelError::InvalidGate,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary,
            slot: timing.not_before_slot,
        },
    );
    assert_ne!(model.proposals[&primary].freeze_gate_epoch, 0);
    assert_eq!(
        model.proposals[&primary].freeze_gate_epoch,
        model.gate.epoch
    );
}

#[test]
fn ordinary_freeze_rechecks_exact_programdata_after_queue() {
    let mut model = active_model();
    let (primary, rollback) = create_pair(
        &mut model,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    prepare_proposal(&mut model, primary);
    prepare_proposal(&mut model, rollback);
    let freeze_slot = model.proposals[&primary]
        .timing
        .not_before_slot
        .max(model.proposals[&rollback].timing.not_before_slot);
    let creation_programdata = model.proposals[&primary].creation_programdata.clone();
    let mut drifted = creation_programdata.clone();
    drifted.raw_hash = [98; 32];
    apply_ok(
        &mut model,
        Release1ModelAction::SetProgramDataObservation(drifted),
    );
    let epoch_before = model.gate.epoch;
    let nonce_before = model.target_nonce;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary,
            slot: freeze_slot,
        },
        Release1ModelError::ProgramDataChanged,
    );
    assert_eq!(model.gate.status, GateStatusV1::Active);
    assert_eq!(model.gate.epoch, epoch_before);
    assert_eq!(model.target_nonce, nonce_before);
    assert_eq!(model.proposals[&primary].state, ProposalStateV2::Timelocked);
    apply_ok(
        &mut model,
        Release1ModelAction::SetProgramDataObservation(creation_programdata),
    );
    apply_ok(
        &mut model,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary,
            slot: freeze_slot,
        },
    );
}

#[test]
fn competing_proposals_consume_one_nonce_and_all_old_candidates_go_stale() {
    let mut model = active_model();
    let (first_primary, first_rollback) = create_pair(
        &mut model,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    let (second_primary, second_rollback) = create_pair(
        &mut model,
        ProposalClassV1::EconomicChange,
        false,
        PROPOSAL_SLOT,
    );
    let (third_primary, _) = create_pair(
        &mut model,
        ProposalClassV1::ConstitutionalChange,
        false,
        PROPOSAL_SLOT,
    );
    for proposal_id in [
        first_primary,
        first_rollback,
        second_primary,
        second_rollback,
    ] {
        prepare_proposal(&mut model, proposal_id);
    }
    let initial_epoch = model.gate.epoch;
    let frozen_slot = freeze_prepared_primary(&mut model, first_primary);
    assert_eq!(model.target_nonce, 2);
    assert_eq!(model.gate.epoch, initial_epoch + 1);
    assert_eq!(model.gate.active_proposal, Some(first_primary));
    assert_eq!(model.gate.freeze_slot, frozen_slot);
    let second_not_before = model.proposals[&second_primary].timing.not_before_slot;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::FreezeProposal {
            proposal_id: second_primary,
            slot: second_not_before,
        },
        Release1ModelError::StaleBinding,
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::AdoptBuffer {
            proposal_id: third_primary,
        },
        Release1ModelError::StaleBinding,
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::FreezeProposal {
            proposal_id: 999,
            slot: frozen_slot,
        },
        Release1ModelError::NotFound,
    );

    let mut nonce_overflow = active_model();
    let (primary, rollback) = create_pair(
        &mut nonce_overflow,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    prepare_proposal(&mut nonce_overflow, primary);
    prepare_proposal(&mut nonce_overflow, rollback);
    nonce_overflow.target_nonce = u64::MAX;
    nonce_overflow
        .proposals
        .get_mut(&primary)
        .unwrap()
        .target_nonce = u64::MAX;
    nonce_overflow
        .proposals
        .get_mut(&rollback)
        .unwrap()
        .target_nonce = u64::MAX;
    let nonce_overflow_slot = nonce_overflow.proposals[&primary].timing.not_before_slot;
    assert_atomic_error(
        &mut nonce_overflow,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary,
            slot: nonce_overflow_slot,
        },
        Release1ModelError::ArithmeticOverflow,
    );

    let mut epoch_overflow = active_model();
    let (primary, rollback) = create_pair(
        &mut epoch_overflow,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    prepare_proposal(&mut epoch_overflow, primary);
    prepare_proposal(&mut epoch_overflow, rollback);
    epoch_overflow.gate.epoch = u64::MAX;
    epoch_overflow
        .proposals
        .get_mut(&primary)
        .unwrap()
        .creation_gate_epoch = u64::MAX;
    epoch_overflow
        .proposals
        .get_mut(&rollback)
        .unwrap()
        .creation_gate_epoch = u64::MAX;
    let epoch_overflow_slot = epoch_overflow.proposals[&primary].timing.not_before_slot;
    assert_atomic_error(
        &mut epoch_overflow,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary,
            slot: epoch_overflow_slot,
        },
        Release1ModelError::ArithmeticOverflow,
    );
}

#[test]
fn cancellation_expiry_and_postfreeze_recovery_are_terminal() {
    let mut cancelled = active_model();
    let (primary, _) = create_pair(
        &mut cancelled,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    apply_ok(
        &mut cancelled,
        Release1ModelAction::ApproveCancellation {
            proposal_id: primary,
            seat: 0,
            slot: PROPOSAL_SLOT + 1,
            reason: 77,
        },
    );
    assert_eq!(cancelled.proposals[&primary].cancellation_reason, 77);
    assert_eq!(cancelled.proposals[&primary].terminal_reason, 0);
    assert_atomic_error(
        &mut cancelled,
        Release1ModelAction::ApproveCancellation {
            proposal_id: primary,
            seat: 1,
            slot: PROPOSAL_SLOT + 1,
            reason: 0,
        },
        Release1ModelError::InvalidStateTransition,
    );
    assert_atomic_error(
        &mut cancelled,
        Release1ModelAction::ApproveCancellation {
            proposal_id: primary,
            seat: 1,
            slot: PROPOSAL_SLOT + 1,
            reason: 78,
        },
        Release1ModelError::InvalidStateTransition,
    );
    assert_atomic_error(
        &mut cancelled,
        Release1ModelAction::ApproveCancellation {
            proposal_id: primary,
            seat: 0,
            slot: PROPOSAL_SLOT + 1,
            reason: 77,
        },
        Release1ModelError::DuplicateApproval,
    );
    for seat in 1..3 {
        apply_ok(
            &mut cancelled,
            Release1ModelAction::ApproveCancellation {
                proposal_id: primary,
                seat,
                slot: PROPOSAL_SLOT + 1,
                reason: 77,
            },
        );
    }
    assert_eq!(
        cancelled.proposals[&primary].state,
        ProposalStateV2::Cancelled
    );
    assert_eq!(cancelled.proposals[&primary].cancellation_reason, 77);
    assert_eq!(cancelled.proposals[&primary].terminal_reason, 77);
    assert_atomic_error(
        &mut cancelled,
        Release1ModelAction::ApproveCancellation {
            proposal_id: primary,
            seat: 3,
            slot: PROPOSAL_SLOT + 2,
            reason: 77,
        },
        Release1ModelError::InvalidStateTransition,
    );

    let mut expired = active_model();
    let (primary, _) = create_pair(
        &mut expired,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    let expiry_slot = expired.proposals[&primary].timing.expiry_slot;
    assert_atomic_error(
        &mut expired,
        Release1ModelAction::ExpireProposal {
            proposal_id: primary,
            slot: expiry_slot - 1,
        },
        Release1ModelError::InvalidStateTransition,
    );
    apply_ok(
        &mut expired,
        Release1ModelAction::ExpireProposal {
            proposal_id: primary,
            slot: expiry_slot,
        },
    );
    assert_eq!(expired.proposals[&primary].state, ProposalStateV2::Expired);
    assert_eq!(expired.proposals[&primary].cancellation_reason, 0);
    assert_eq!(
        expired.proposals[&primary].terminal_reason,
        MODEL_EXPIRED_TERMINAL_REASON
    );
    assert_atomic_error(
        &mut expired,
        Release1ModelAction::ExpireProposal {
            proposal_id: primary,
            slot: expiry_slot + 1,
        },
        Release1ModelError::InvalidStateTransition,
    );

    let mut frozen = active_model();
    let (primary, rollback) = create_pair(
        &mut frozen,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    prepare_proposal(&mut frozen, primary);
    prepare_proposal(&mut frozen, rollback);
    let frozen_slot = freeze_prepared_primary(&mut frozen, primary);
    assert_atomic_error(
        &mut frozen,
        Release1ModelAction::ApproveCancellation {
            proposal_id: primary,
            seat: 0,
            slot: frozen_slot + 1,
            reason: 77,
        },
        Release1ModelError::InvalidStateTransition,
    );
    let frozen_expiry = frozen.proposals[&primary].timing.expiry_slot;
    assert_atomic_error(
        &mut frozen,
        Release1ModelAction::ExpireProposal {
            proposal_id: primary,
            slot: frozen_expiry,
        },
        Release1ModelError::InvalidStateTransition,
    );
    assert_eq!(frozen.gate.status, GateStatusV1::FrozenForUpgrade);
}

#[test]
fn checkpoints_finalize_separately_and_extension_unfreeze_is_complete() {
    let mut model = active_model();
    let (primary, rollback) = create_pair(
        &mut model,
        ProposalClassV1::RoutineUpgrade,
        true,
        PROPOSAL_SLOT,
    );
    prepare_proposal(&mut model, primary);
    prepare_proposal(&mut model, rollback);
    let rollback_ordinary_freeze_slot = model.proposals[&rollback].timing.not_before_slot;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::FreezeProposal {
            proposal_id: rollback,
            slot: rollback_ordinary_freeze_slot,
        },
        Release1ModelError::InvalidRollbackLink,
    );
    let frozen_slot = freeze_prepared_primary(&mut model, primary);
    let frozen_epoch = model.gate.epoch;

    assert_atomic_error(
        &mut model,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            attesting_seats: [0, 1, 2],
            slot: frozen_slot + 1,
        },
        Release1ModelError::QuorumNotSatisfied,
    );
    for seat in 0..3 {
        apply_ok(
            &mut model,
            Release1ModelAction::AttestCheckpoint {
                proposal_id: primary,
                phase: StateCheckpointPhaseV1::Prestate,
                seat,
                hard_state: if seat == 2 {
                    hard_state(2)
                } else {
                    hard_state(1)
                },
                forbidden_drift_count: 0,
                slot: frozen_slot + 1,
            },
        );
    }
    assert_eq!(
        model.proposals[&primary].state,
        ProposalStateV2::Frozen,
        "third approval must not finalize"
    );
    assert!(model.proposals[&primary].prestate.is_none());
    assert_atomic_error(
        &mut model,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            attesting_seats: [0, 1, 2],
            slot: frozen_slot + 1,
        },
        Release1ModelError::QuorumNotSatisfied,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::AttestCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            seat: 2,
            hard_state: hard_state(1),
            forbidden_drift_count: 0,
            slot: frozen_slot + 1,
        },
    );
    apply_ok(
        &mut model,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            attesting_seats: [0, 1, 2],
            slot: frozen_slot + 2,
        },
    );
    assert_eq!(
        model.proposals[&primary]
            .prestate
            .as_ref()
            .unwrap()
            .gate_epoch,
        frozen_epoch
    );

    assert_atomic_error(
        &mut model,
        Release1ModelAction::ExtendTarget {
            proposal_id: primary,
            slot: frozen_slot - 1,
        },
        Release1ModelError::InvalidStateTransition,
    );
    let proposal_expiry = model.proposals[&primary].timing.expiry_slot;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ExtendTarget {
            proposal_id: primary,
            slot: proposal_expiry,
        },
        Release1ModelError::InvalidStateTransition,
    );
    assert_eq!(model.gate.status, GateStatusV1::FrozenForUpgrade);
    let extension_slot = primary_execution_slot(&model, primary, frozen_slot + 3);
    apply_ok(
        &mut model,
        Release1ModelAction::ExtendTarget {
            proposal_id: primary,
            slot: extension_slot,
        },
    );
    assert_eq!(model.gate.status, GateStatusV1::FrozenForUpgrade);
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: primary,
            slot: extension_slot,
        },
        Release1ModelError::InvalidStateTransition,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: primary,
            slot: extension_slot + 1,
        },
    );
    assert_eq!(model.gate.status, GateStatusV1::FrozenForUpgrade);
    let mut wrong_active_proposal = model.clone();
    wrong_active_proposal.gate.active_proposal = Some(rollback);
    assert_atomic_error(
        &mut wrong_active_proposal,
        Release1ModelAction::VerifyProgramData {
            proposal_id: primary,
            slot: extension_slot + 1,
        },
        Release1ModelError::InvalidGate,
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::VerifyProgramData {
            proposal_id: primary,
            slot: extension_slot,
        },
        Release1ModelError::InvalidStateTransition,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::VerifyProgramData {
            proposal_id: primary,
            slot: extension_slot + 1,
        },
    );
    approve_and_finalize_checkpoint(
        &mut model,
        primary,
        StateCheckpointPhaseV1::Poststate,
        extension_slot + 2,
    );
    assert_eq!(
        model.proposals[&primary].state,
        ProposalStateV2::PoststateAccepted
    );
    assert_eq!(model.gate.status, GateStatusV1::FrozenForUpgrade);
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ExecuteUnfreeze {
            proposal_id: primary,
            slot: extension_slot + 4,
        },
        Release1ModelError::InvalidStateTransition,
    );
    for seat in 0..3 {
        apply_ok(
            &mut model,
            Release1ModelAction::ApproveUnfreeze {
                proposal_id: primary,
                seat,
                slot: extension_slot + 4,
            },
        );
    }
    assert_eq!(
        model.proposals[&primary].state,
        ProposalStateV2::UnfreezeApproved
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ExecuteUnfreeze {
            proposal_id: primary,
            slot: extension_slot + 5,
        },
    );
    assert_eq!(model.gate.status, GateStatusV1::Active);
    assert_eq!(model.gate.epoch, frozen_epoch + 1);
    assert_eq!(model.gate.last_completed_proposal, Some(primary));
    assert_eq!(model.proposals[&primary].state, ProposalStateV2::Completed);
    assert_eq!(model.proposals[&rollback].state, ProposalStateV2::Retired);
    assert_eq!(model.target_nonce, 2);
}

#[test]
fn checkpoint_attestations_prevent_canonical_squatting() {
    let mut model = active_model();
    let (primary, rollback) = create_pair(
        &mut model,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    prepare_proposal(&mut model, primary);
    prepare_proposal(&mut model, rollback);
    let frozen_slot = freeze_prepared_primary(&mut model, primary);

    let mut inconsistent_combined_root = hard_state(9);
    inconsistent_combined_root.program_owned_count += 1;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::AttestCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            seat: 0,
            hard_state: inconsistent_combined_root,
            forbidden_drift_count: 0,
            slot: frozen_slot + 1,
        },
        Release1ModelError::InvalidInitialization,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::AttestCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            seat: 0,
            hard_state: hard_state(9),
            forbidden_drift_count: 0,
            slot: frozen_slot + 1,
        },
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            attesting_seats: [0, 1, 2],
            slot: frozen_slot + 2,
        },
        Release1ModelError::QuorumNotSatisfied,
    );
    for seat in 1..2 {
        apply_ok(
            &mut model,
            Release1ModelAction::AttestCheckpoint {
                proposal_id: primary,
                phase: StateCheckpointPhaseV1::Prestate,
                seat,
                hard_state: hard_state(1),
                forbidden_drift_count: 0,
                slot: frozen_slot + 1,
            },
        );
    }
    assert_atomic_error(
        &mut model,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            attesting_seats: [0, 1, 2],
            slot: frozen_slot + 2,
        },
        Release1ModelError::QuorumNotSatisfied,
    );
    for seat in 2..4 {
        apply_ok(
            &mut model,
            Release1ModelAction::AttestCheckpoint {
                proposal_id: primary,
                phase: StateCheckpointPhaseV1::Prestate,
                seat,
                hard_state: hard_state(1),
                forbidden_drift_count: 0,
                slot: frozen_slot + 1,
            },
        );
    }
    assert!(model.proposals[&primary].prestate.is_none());
    assert_atomic_error(
        &mut model,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            attesting_seats: [0, 1, 2],
            slot: frozen_slot + 2,
        },
        Release1ModelError::QuorumNotSatisfied,
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            attesting_seats: [1, 1, 2],
            slot: frozen_slot + 2,
        },
        Release1ModelError::InvalidApproval,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            attesting_seats: [1, 2, 3],
            slot: frozen_slot + 2,
        },
    );
    let checkpoint = model.proposals[&primary]
        .prestate
        .as_ref()
        .expect("three matching attestations create the canonical checkpoint");
    assert!(checkpoint.accepted);
    assert_eq!(checkpoint.hard_state, hard_state(1));
    assert_eq!(checkpoint.approvals.bitset, 0b0000_1110);
    assert_atomic_error(
        &mut model,
        Release1ModelAction::AttestCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            seat: 4,
            hard_state: hard_state(2),
            forbidden_drift_count: 0,
            slot: frozen_slot + 3,
        },
        Release1ModelError::InvalidStateTransition,
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            attesting_seats: [1, 2, 3],
            slot: frozen_slot + 3,
        },
        Release1ModelError::InvalidStateTransition,
    );
}

#[test]
fn rollback_is_prepared_reciprocally_linked_and_never_auto_unfreezes() {
    let mut missing_rollback = active_model();
    let (primary, rollback) = create_pair(
        &mut missing_rollback,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    prepare_proposal(&mut missing_rollback, primary);
    let attempted_freeze_slot = missing_rollback.proposals[&primary].timing.not_before_slot;
    assert_atomic_error(
        &mut missing_rollback,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary,
            slot: attempted_freeze_slot,
        },
        Release1ModelError::InvalidRollbackLink,
    );
    assert_eq!(missing_rollback.gate.status, GateStatusV1::Active);
    assert_eq!(missing_rollback.target_nonce, 1);
    assert_eq!(missing_rollback.gate.active_proposal, None);
    assert_eq!(
        missing_rollback.proposals[&primary].state,
        ProposalStateV2::Timelocked
    );
    assert_eq!(
        missing_rollback.proposals[&rollback].state,
        ProposalStateV2::Draft
    );

    let mut model = active_model();
    let (primary, rollback) = create_pair(
        &mut model,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    prepare_proposal(&mut model, primary);
    prepare_proposal(&mut model, rollback);
    let frozen_slot = freeze_prepared_primary(&mut model, primary);
    approve_and_finalize_checkpoint(
        &mut model,
        primary,
        StateCheckpointPhaseV1::Prestate,
        frozen_slot + 1,
    );
    let primary_upgrade_slot = primary_execution_slot(&model, primary, frozen_slot + 3);
    apply_ok(
        &mut model,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: primary,
            slot: primary_upgrade_slot,
        },
    );
    assert_eq!(
        model.proposals[&primary].state,
        ProposalStateV2::UpgradeExecuted
    );
    assert_eq!(model.gate.status, GateStatusV1::FrozenForUpgrade);
    let primary_epoch = model.gate.epoch;
    let consumed_nonce = model.target_nonce;
    let upgraded_slot = model.proposals[&primary].upgraded_slot;
    let rollback_ready_slot = upgraded_slot + model.delays.rollback_slots;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ActivateRollback {
            primary_proposal_id: primary,
            rollback_proposal_id: 999,
            slot: frozen_slot + 4,
        },
        Release1ModelError::NotFound,
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ActivateRollback {
            primary_proposal_id: primary,
            rollback_proposal_id: rollback,
            slot: rollback_ready_slot,
        },
        Release1ModelError::ProgramDataFailureEvidenceRequired,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::RecordProgramDataFailure {
            proposal_id: primary,
            kind: ModelProgramDataFailureKind::MerkleRoot,
            slot: upgraded_slot,
        },
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::RecordProgramDataFailure {
            proposal_id: primary,
            kind: ModelProgramDataFailureKind::NonzeroTail,
            slot: upgraded_slot + 1,
        },
        Release1ModelError::InvalidStateTransition,
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::VerifyProgramData {
            proposal_id: primary,
            slot: upgraded_slot + 1,
        },
        Release1ModelError::InvalidStateTransition,
    );
    let mut drifted_after_failure = model.clone();
    let mut drifted_observation = drifted_after_failure.programdata.clone();
    drifted_observation.raw_hash = [99; 32];
    apply_ok(
        &mut drifted_after_failure,
        Release1ModelAction::SetProgramDataObservation(drifted_observation),
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ActivateRollback {
            primary_proposal_id: primary,
            rollback_proposal_id: rollback,
            slot: rollback_ready_slot - 1,
        },
        Release1ModelError::TimingViolation,
    );
    assert_atomic_error(
        &mut drifted_after_failure,
        Release1ModelAction::ActivateRollback {
            primary_proposal_id: primary,
            rollback_proposal_id: rollback,
            slot: rollback_ready_slot,
        },
        Release1ModelError::ProgramDataChanged,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ActivateRollback {
            primary_proposal_id: primary,
            rollback_proposal_id: rollback,
            slot: rollback_ready_slot,
        },
    );
    assert_eq!(model.target_nonce, consumed_nonce);
    assert_eq!(model.gate.epoch, primary_epoch + 1);
    assert_eq!(model.gate.active_proposal, Some(rollback));
    assert_eq!(model.proposals[&rollback].state, ProposalStateV2::Frozen);

    approve_and_finalize_checkpoint(
        &mut model,
        rollback,
        StateCheckpointPhaseV1::Prestate,
        rollback_ready_slot + 1,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: rollback,
            slot: rollback_ready_slot + 3,
        },
    );
    assert_eq!(model.gate.status, GateStatusV1::FrozenForUpgrade);
    apply_ok(
        &mut model,
        Release1ModelAction::VerifyProgramData {
            proposal_id: rollback,
            slot: rollback_ready_slot + 3,
        },
    );
    approve_and_finalize_checkpoint(
        &mut model,
        rollback,
        StateCheckpointPhaseV1::Poststate,
        rollback_ready_slot + 4,
    );
    for seat in 0..3 {
        apply_ok(
            &mut model,
            Release1ModelAction::ApproveUnfreeze {
                proposal_id: rollback,
                seat,
                slot: rollback_ready_slot + 6,
            },
        );
    }
    assert_eq!(model.gate.status, GateStatusV1::FrozenForUpgrade);
    apply_ok(
        &mut model,
        Release1ModelAction::ExecuteUnfreeze {
            proposal_id: rollback,
            slot: rollback_ready_slot + 7,
        },
    );
    assert_eq!(model.proposals[&rollback].state, ProposalStateV2::Completed);
    assert_eq!(
        model.proposals[&primary].state,
        ProposalStateV2::SupersededByRollback
    );
    assert_eq!(model.gate.status, GateStatusV1::Active);
}

#[test]
fn primary_execution_requires_an_unexpired_actionable_rollback_window() {
    let mut ready = active_model();
    let (primary, rollback) = create_pair(
        &mut ready,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    prepare_proposal(&mut ready, primary);
    prepare_proposal(&mut ready, rollback);
    let frozen_slot = freeze_prepared_primary(&mut ready, primary);
    approve_and_finalize_checkpoint(
        &mut ready,
        primary,
        StateCheckpointPhaseV1::Prestate,
        frozen_slot + 1,
    );
    let rollback_timing = ready.proposals[&rollback].timing;
    let pre_not_before_slot = rollback_timing.not_before_slot - 1;
    assert_atomic_error(
        &mut ready,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: primary,
            slot: pre_not_before_slot,
        },
        Release1ModelError::TimingViolation,
    );

    let execute_slot = rollback_timing.not_before_slot;
    let first_rollback_slot = execute_slot + ready.delays.rollback_slots;
    let primary_expiry_with_delay =
        ready.proposals[&primary].timing.expiry_slot + ready.delays.rollback_slots;

    let mut expired = ready.clone();
    expired
        .proposals
        .get_mut(&rollback)
        .unwrap()
        .timing
        .expiry_slot = execute_slot;
    assert_atomic_error(
        &mut expired,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: primary,
            slot: execute_slot,
        },
        Release1ModelError::TimingViolation,
    );

    let mut no_executable_slot = ready.clone();
    no_executable_slot
        .proposals
        .get_mut(&rollback)
        .unwrap()
        .timing
        .expiry_slot = first_rollback_slot;
    assert_atomic_error(
        &mut no_executable_slot,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: primary,
            slot: execute_slot,
        },
        Release1ModelError::TimingViolation,
    );

    let mut no_full_primary_runway = ready.clone();
    no_full_primary_runway
        .proposals
        .get_mut(&rollback)
        .unwrap()
        .timing
        .expiry_slot = primary_expiry_with_delay;
    assert_atomic_error(
        &mut no_full_primary_runway,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: primary,
            slot: execute_slot,
        },
        Release1ModelError::TimingViolation,
    );

    let mut overflow = ready.clone();
    overflow
        .proposals
        .get_mut(&primary)
        .unwrap()
        .timing
        .expiry_slot = u64::MAX;
    overflow
        .proposals
        .get_mut(&rollback)
        .unwrap()
        .timing
        .expiry_slot = u64::MAX;
    assert_atomic_error(
        &mut overflow,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: primary,
            slot: u64::MAX - 1,
        },
        Release1ModelError::ArithmeticOverflow,
    );

    let recovery_slots = ready.delays.rollback_slots + ready.delays.review_slots + 1;
    let council_review_runway_boundary = rollback_timing.expiry_slot - recovery_slots;
    assert!(council_review_runway_boundary >= rollback_timing.not_before_slot);
    assert_atomic_error(
        &mut ready,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: primary,
            slot: council_review_runway_boundary,
        },
        Release1ModelError::TimingViolation,
    );
    apply_ok(
        &mut ready,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: primary,
            slot: council_review_runway_boundary - 1,
        },
    );
}

#[test]
fn approved_hard_poststate_mismatches_authorize_delayed_rollback_but_never_accept() {
    let mut verified = active_model();
    let (primary, rollback) = create_pair(
        &mut verified,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    prepare_proposal(&mut verified, primary);
    prepare_proposal(&mut verified, rollback);
    let frozen_slot = freeze_prepared_primary(&mut verified, primary);
    approve_and_finalize_checkpoint(
        &mut verified,
        primary,
        StateCheckpointPhaseV1::Prestate,
        frozen_slot + 1,
    );
    let upgraded_slot = primary_execution_slot(&verified, primary, frozen_slot + 3);
    apply_ok(
        &mut verified,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: primary,
            slot: upgraded_slot,
        },
    );
    apply_ok(
        &mut verified,
        Release1ModelAction::VerifyProgramData {
            proposal_id: primary,
            slot: upgraded_slot,
        },
    );
    let rollback_ready_slot = upgraded_slot + verified.delays.rollback_slots;

    let mut mismatches = Vec::new();
    let baseline = hard_state(1);
    let mut mismatch = baseline.clone();
    mismatch.schema_identifier = [90; 32];
    mismatch.hard_combined_root = mismatch.recompute_hard_combined_root();
    mismatches.push((mismatch, 1));
    let mut mismatch = baseline.clone();
    mismatch.program_owned_root = [91; 32];
    mismatch.hard_combined_root = mismatch.recompute_hard_combined_root();
    mismatches.push((mismatch, 1));
    let mut mismatch = baseline.clone();
    mismatch.program_owned_count += 1;
    mismatch.hard_combined_root = mismatch.recompute_hard_combined_root();
    mismatches.push((mismatch, 1));
    let mut mismatch = baseline.clone();
    mismatch.logical_compressed_root = [92; 32];
    mismatch.hard_combined_root = mismatch.recompute_hard_combined_root();
    mismatches.push((mismatch, 1));
    let mut mismatch = baseline.clone();
    mismatch.logical_compressed_count += 1;
    mismatch.hard_combined_root = mismatch.recompute_hard_combined_root();
    mismatches.push((mismatch, 1));
    let mut mismatch = baseline.clone();
    mismatch.semantic_custody_root = [93; 32];
    mismatch.hard_combined_root = mismatch.recompute_hard_combined_root();
    mismatches.push((mismatch, 1));
    let mut mismatch = baseline;
    mismatch.custody_identity_root = [94; 32];
    mismatch.hard_combined_root = mismatch.recompute_hard_combined_root();
    mismatches.push((mismatch, 1));
    mismatches.push((hard_state(1), 1));

    let mut zero_count_mismatch = verified.clone();
    for seat in 0..3 {
        apply_ok(
            &mut zero_count_mismatch,
            Release1ModelAction::AttestCheckpoint {
                proposal_id: primary,
                phase: StateCheckpointPhaseV1::Poststate,
                seat,
                hard_state: mismatches[0].0.clone(),
                forbidden_drift_count: 0,
                slot: upgraded_slot + 1,
            },
        );
    }
    assert_atomic_error(
        &mut zero_count_mismatch,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Poststate,
            attesting_seats: [0, 1, 2],
            slot: upgraded_slot + 2,
        },
        Release1ModelError::HardPoststateMismatch,
    );
    assert!(zero_count_mismatch.proposals[&primary].poststate.is_none());

    for (hard_mismatch, forbidden_drift_count) in mismatches {
        let mut case = verified.clone();
        for seat in 0..2 {
            apply_ok(
                &mut case,
                Release1ModelAction::AttestCheckpoint {
                    proposal_id: primary,
                    phase: StateCheckpointPhaseV1::Poststate,
                    seat,
                    hard_state: hard_mismatch.clone(),
                    forbidden_drift_count,
                    slot: upgraded_slot + 1,
                },
            );
        }
        assert_atomic_error(
            &mut case,
            Release1ModelAction::ActivateRollback {
                primary_proposal_id: primary,
                rollback_proposal_id: rollback,
                slot: rollback_ready_slot,
            },
            Release1ModelError::ProgramDataFailureEvidenceRequired,
        );
        apply_ok(
            &mut case,
            Release1ModelAction::AttestCheckpoint {
                proposal_id: primary,
                phase: StateCheckpointPhaseV1::Poststate,
                seat: 2,
                hard_state: hard_mismatch,
                forbidden_drift_count,
                slot: upgraded_slot + 1,
            },
        );
        apply_ok(
            &mut case,
            Release1ModelAction::FinalizeCheckpoint {
                proposal_id: primary,
                phase: StateCheckpointPhaseV1::Poststate,
                attesting_seats: [0, 1, 2],
                slot: upgraded_slot + 2,
            },
        );
        let poststate = case.proposals[&primary]
            .poststate
            .as_ref()
            .expect("bound poststate");
        assert!(!poststate.accepted);
        assert_eq!(poststate.approvals.count, 3);
        assert_eq!(
            case.proposals[&primary].state,
            ProposalStateV2::ProgramDataVerified
        );
        assert_eq!(case.gate.status, GateStatusV1::FrozenForUpgrade);
        assert_atomic_error(
            &mut case,
            Release1ModelAction::ActivateRollback {
                primary_proposal_id: primary,
                rollback_proposal_id: rollback,
                slot: rollback_ready_slot - 1,
            },
            Release1ModelError::TimingViolation,
        );
        apply_ok(
            &mut case,
            Release1ModelAction::ActivateRollback {
                primary_proposal_id: primary,
                rollback_proposal_id: rollback,
                slot: rollback_ready_slot,
            },
        );
        assert_eq!(case.gate.status, GateStatusV1::FrozenForUpgrade);
        assert_eq!(case.gate.active_proposal, Some(rollback));
        assert_eq!(case.proposals[&rollback].state, ProposalStateV2::Frozen);
    }
}

#[test]
fn guardian_freeze_and_governed_emergency_resume_require_unchanged_programdata() {
    let mut model = active_model();
    let initial_epoch = model.gate.epoch;
    let initial_nonce = model.target_nonce;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::GuardianFreeze {
            guardian: key(99),
            slot: 30,
            reason: 9,
        },
        Release1ModelError::InvalidGate,
    );
    for reason in [0, BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1] {
        assert_atomic_error(
            &mut model,
            Release1ModelAction::GuardianFreeze {
                guardian: graph().guardian,
                slot: 30,
                reason,
            },
            Release1ModelError::InvalidGate,
        );
    }
    apply_ok(
        &mut model,
        Release1ModelAction::GuardianFreeze {
            guardian: graph().guardian,
            slot: 30,
            reason: 9,
        },
    );
    assert_eq!(model.gate.status, GateStatusV1::EmergencyFrozen);
    assert_eq!(model.gate.epoch, initial_epoch + 1);
    assert_eq!(model.target_nonce, initial_nonce);
    assert_eq!(model.gate.active_proposal, None);
    let frozen_epoch = model.gate.epoch;
    let freeze_time_programdata = model.programdata.clone();
    let recorded_freeze = model
        .emergency_freeze_observations
        .get(&frozen_epoch)
        .expect("guardian freeze observation");
    assert_eq!(recorded_freeze.epoch, frozen_epoch);
    assert_eq!(recorded_freeze.freeze_slot, 30);
    assert_eq!(recorded_freeze.freeze_reason, 9);
    assert_eq!(
        recorded_freeze.programdata,
        canonical_guardian_observation(&freeze_time_programdata)
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::GuardianFreeze {
            guardian: graph().guardian,
            slot: 31,
            reason: 9,
        },
        Release1ModelError::InvalidGate,
    );

    let mut payload_only = model.clone();
    let mut changed_payload_commitment = freeze_time_programdata.clone();
    changed_payload_commitment.payload_hash = [76; 32];
    apply_ok(
        &mut payload_only,
        Release1ModelAction::SetProgramDataObservation(changed_payload_commitment),
    );
    payload_only
        .apply(Release1ModelAction::CreateEmergencyResolution { slot: 31 })
        .expect(
            "guardian baseline intentionally binds the full raw account hash, not payload_hash",
        );

    let mut late_creator = model.clone();
    let late_resolution_id = match late_creator
        .apply(Release1ModelAction::CreateEmergencyResolution { slot: 100 })
        .expect("late permissionless creator cannot move canonical deadlines")
    {
        Release1ModelOutcome::EmergencyResolutionCreated(id) => id,
        outcome => panic!("unexpected emergency outcome: {outcome:?}"),
    };
    let late_resolution = &late_creator.emergency_resolutions[&late_resolution_id];
    assert_eq!(late_resolution.creation_slot, 100);
    assert_eq!(late_resolution.not_before_slot, 40);
    assert_eq!(late_resolution.expiry_slot, 130);
    assert_atomic_error(
        &mut late_creator,
        Release1ModelAction::ApproveEmergencyResolution {
            resolution_id: late_resolution_id,
            seat: 0,
            slot: 39,
        },
        Release1ModelError::TimingViolation,
    );
    let mut expired_creator = model.clone();
    assert_atomic_error(
        &mut expired_creator,
        Release1ModelAction::CreateEmergencyResolution { slot: 130 },
        Release1ModelError::TimingViolation,
    );

    let mut drifted_creator = model.clone();
    let mut changed_before_resolution = freeze_time_programdata.clone();
    changed_before_resolution.raw_hash = [76; 32];
    apply_ok(
        &mut drifted_creator,
        Release1ModelAction::SetProgramDataObservation(changed_before_resolution),
    );
    let drifted_resolution_id = match drifted_creator
        .apply(Release1ModelAction::CreateEmergencyResolution { slot: 31 })
        .expect("resolution creation binds the immutable freeze observation despite later drift")
    {
        Release1ModelOutcome::EmergencyResolutionCreated(id) => id,
        outcome => panic!("unexpected emergency outcome: {outcome:?}"),
    };
    assert_eq!(
        drifted_creator.emergency_resolutions[&drifted_resolution_id].observed_programdata,
        canonical_guardian_observation(&freeze_time_programdata),
        "post-freeze drift must not rewrite the resolution baseline"
    );

    let resolution_id = match model
        .apply(Release1ModelAction::CreateEmergencyResolution { slot: 31 })
        .expect("emergency resolution creation")
    {
        Release1ModelOutcome::EmergencyResolutionCreated(id) => id,
        outcome => panic!("unexpected emergency outcome: {outcome:?}"),
    };
    assert_eq!(resolution_id, frozen_epoch);
    assert_eq!(
        model.emergency_resolutions[&resolution_id].not_before_slot,
        40
    );
    assert_eq!(model.emergency_resolutions[&resolution_id].expiry_slot, 130);
    assert_atomic_error(
        &mut model,
        Release1ModelAction::CreateEmergencyResolution { slot: 39 },
        Release1ModelError::InvalidStateTransition,
    );
    assert_eq!(
        model.emergency_resolutions[&resolution_id].frozen_epoch,
        frozen_epoch
    );
    assert_eq!(
        model.emergency_resolutions[&resolution_id].observed_programdata,
        canonical_guardian_observation(&freeze_time_programdata)
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ExecuteEmergencyResume {
            resolution_id,
            slot: 41,
        },
        Release1ModelError::InvalidStateTransition,
    );
    for seat in 0..3 {
        apply_ok(
            &mut model,
            Release1ModelAction::ApproveEmergencyResolution {
                resolution_id,
                seat,
                slot: 40,
            },
        );
    }
    assert_eq!(
        model.emergency_resolutions[&resolution_id].state,
        EmergencyFreezeResolutionStateV1::CouncilApproved
    );
    apply_ok(
        &mut model,
        Release1ModelAction::QueueEmergencyResolution {
            resolution_id,
            slot: 40,
        },
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::FinalizeEmergencyCheckpoint {
            resolution_id,
            attesting_seats: [0, 1, 2],
            slot: 41,
        },
        Release1ModelError::QuorumNotSatisfied,
    );
    for seat in 0..3 {
        apply_ok(
            &mut model,
            Release1ModelAction::AttestEmergencyCheckpoint {
                resolution_id,
                seat,
                hard_state: hard_state(1),
                forbidden_drift_count: 0,
                slot: 41,
            },
        );
    }
    assert!(model.emergency_resolutions[&resolution_id]
        .checkpoint
        .is_none());
    apply_ok(
        &mut model,
        Release1ModelAction::FinalizeEmergencyCheckpoint {
            resolution_id,
            attesting_seats: [0, 1, 2],
            slot: 42,
        },
    );
    let not_before = model.emergency_resolutions[&resolution_id].not_before_slot;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ExecuteEmergencyResume {
            resolution_id,
            slot: not_before - 1,
        },
        Release1ModelError::InvalidStateTransition,
    );

    let original = model.programdata.clone();
    let mut changed = original.clone();
    changed.raw_hash = [77; 32];
    apply_ok(
        &mut model,
        Release1ModelAction::SetProgramDataObservation(changed),
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ExecuteEmergencyResume {
            resolution_id,
            slot: 43,
        },
        Release1ModelError::ProgramDataChanged,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::SetProgramDataObservation(original),
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ExecuteEmergencyResume {
            resolution_id,
            slot: 43,
        },
    );
    assert_eq!(model.gate.status, GateStatusV1::Active);
    assert_eq!(model.gate.epoch, frozen_epoch + 1);
    assert_eq!(model.target_nonce, initial_nonce);
    assert_eq!(
        model.emergency_resolutions[&resolution_id].state,
        EmergencyFreezeResolutionStateV1::Executed
    );
    assert_eq!(
        model.emergency_resolutions[&resolution_id].terminal_reason,
        EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V1
    );
}

#[test]
fn emergency_resolution_expiry_is_permissionless_terminal_and_keeps_gate_frozen() {
    let mut model = active_model();
    apply_ok(
        &mut model,
        Release1ModelAction::GuardianFreeze {
            guardian: graph().guardian,
            slot: 30,
            reason: 9,
        },
    );
    let frozen_gate = model.gate.clone();
    let frozen_nonce = model.target_nonce;
    let resolution_id = match model
        .apply(Release1ModelAction::CreateEmergencyResolution { slot: 31 })
        .expect("emergency resolution creation")
    {
        Release1ModelOutcome::EmergencyResolutionCreated(id) => id,
        outcome => panic!("unexpected emergency outcome: {outcome:?}"),
    };
    let expiry_slot = model.emergency_resolutions[&resolution_id].expiry_slot;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ExpireEmergencyResolution {
            resolution_id,
            slot: expiry_slot - 1,
        },
        Release1ModelError::TimingViolation,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ExpireEmergencyResolution {
            resolution_id,
            slot: expiry_slot,
        },
    );
    let resolution = &model.emergency_resolutions[&resolution_id];
    assert_eq!(resolution.state, EmergencyFreezeResolutionStateV1::Expired);
    assert_eq!(resolution.executed_slot, 0);
    assert_eq!(
        resolution.terminal_reason,
        EMERGENCY_RESOLUTION_EXPIRED_TERMINAL_REASON_V1
    );
    assert_eq!(model.gate, frozen_gate);
    assert_eq!(model.gate.status, GateStatusV1::EmergencyFrozen);
    assert_eq!(model.target_nonce, frozen_nonce);
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ExpireEmergencyResolution {
            resolution_id,
            slot: expiry_slot + 1,
        },
        Release1ModelError::InvalidStateTransition,
    );
}

#[test]
fn guardian_freeze_captures_none_or_drifted_authority_but_resume_requires_controller() {
    for observed_authority in [None, Some(key(99))] {
        let mut model = active_model();
        apply_ok(
            &mut model,
            Release1ModelAction::SetProgramDataAuthorityObservation(observed_authority),
        );
        assert_atomic_error(
            &mut model,
            Release1ModelAction::CreateProposal(create_request(
                ProposalClassV1::RoutineUpgrade,
                false,
                None,
                Some(2),
                PROPOSAL_SLOT,
            )),
            Release1ModelError::ProgramDataChanged,
        );

        apply_ok(
            &mut model,
            Release1ModelAction::GuardianFreeze {
                guardian: graph().guardian,
                slot: 30,
                reason: 9,
            },
        );
        let frozen_epoch = model.gate.epoch;
        assert_eq!(
            model.emergency_freeze_observations[&frozen_epoch]
                .programdata
                .authority,
            observed_authority,
            "guardian freeze records the actual optional authority without requiring it to be canonical"
        );

        let resolution_id = match model
            .apply(Release1ModelAction::CreateEmergencyResolution { slot: 31 })
            .expect("unchanged freeze observation permits resolution creation")
        {
            Release1ModelOutcome::EmergencyResolutionCreated(id) => id,
            outcome => panic!("unexpected emergency outcome: {outcome:?}"),
        };
        for seat in 0..3 {
            apply_ok(
                &mut model,
                Release1ModelAction::ApproveEmergencyResolution {
                    resolution_id,
                    seat,
                    slot: 40,
                },
            );
        }
        apply_ok(
            &mut model,
            Release1ModelAction::QueueEmergencyResolution {
                resolution_id,
                slot: 40,
            },
        );
        for seat in 0..3 {
            apply_ok(
                &mut model,
                Release1ModelAction::AttestEmergencyCheckpoint {
                    resolution_id,
                    seat,
                    hard_state: hard_state(1),
                    forbidden_drift_count: 0,
                    slot: 41,
                },
            );
        }
        apply_ok(
            &mut model,
            Release1ModelAction::FinalizeEmergencyCheckpoint {
                resolution_id,
                attesting_seats: [0, 1, 2],
                slot: 42,
            },
        );
        assert_atomic_error(
            &mut model,
            Release1ModelAction::ExecuteEmergencyResume {
                resolution_id,
                slot: 43,
            },
            Release1ModelError::ProgramDataChanged,
        );
        assert_eq!(model.gate.status, GateStatusV1::EmergencyFrozen);
        assert_eq!(model.gate.epoch, frozen_epoch);
    }
}

#[test]
fn guardian_freeze_persists_malformed_or_oversized_runtime_evidence_but_resume_fails_closed() {
    let canonical = active_model().current_guardian_runtime_observation;
    let mut malformed_program_header = canonical.clone();
    malformed_program_header.program_header_present = false;
    malformed_program_header.linked_programdata = None;

    let mut oversized_programdata = canonical;
    oversized_programdata.programdata_data_length = MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1;
    oversized_programdata.capacity =
        oversized_programdata.programdata_data_length - LOADER_V3_PROGRAMDATA_METADATA_LEN_V1;
    oversized_programdata.raw_hash_complete = false;
    oversized_programdata.raw_hash = [0; 32];

    for runtime_observation in [malformed_program_header, oversized_programdata] {
        let mut model = active_model();
        apply_ok(
            &mut model,
            Release1ModelAction::SetGuardianRuntimeObservation(runtime_observation.clone()),
        );
        apply_ok(
            &mut model,
            Release1ModelAction::GuardianFreeze {
                guardian: graph().guardian,
                slot: 30,
                reason: 9,
            },
        );
        let frozen_epoch = model.gate.epoch;
        assert_eq!(
            model.emergency_freeze_observations[&frozen_epoch].programdata, runtime_observation,
            "guardian freeze must persist fail-closed evidence instead of becoming unavailable"
        );
        let resolution_id = match model
            .apply(Release1ModelAction::CreateEmergencyResolution { slot: 31 })
            .expect("resolution creation binds malformed freeze evidence")
        {
            Release1ModelOutcome::EmergencyResolutionCreated(id) => id,
            outcome => panic!("unexpected emergency outcome: {outcome:?}"),
        };
        for seat in 0..3 {
            apply_ok(
                &mut model,
                Release1ModelAction::ApproveEmergencyResolution {
                    resolution_id,
                    seat,
                    slot: 40,
                },
            );
        }
        apply_ok(
            &mut model,
            Release1ModelAction::QueueEmergencyResolution {
                resolution_id,
                slot: 40,
            },
        );
        for seat in 0..3 {
            apply_ok(
                &mut model,
                Release1ModelAction::AttestEmergencyCheckpoint {
                    resolution_id,
                    seat,
                    hard_state: hard_state(1),
                    forbidden_drift_count: 0,
                    slot: 41,
                },
            );
        }
        apply_ok(
            &mut model,
            Release1ModelAction::FinalizeEmergencyCheckpoint {
                resolution_id,
                attesting_seats: [0, 1, 2],
                slot: 42,
            },
        );
        assert_atomic_error(
            &mut model,
            Release1ModelAction::ExecuteEmergencyResume {
                resolution_id,
                slot: 43,
            },
            Release1ModelError::ProgramDataChanged,
        );
        assert_eq!(model.gate.status, GateStatusV1::EmergencyFrozen);
        assert_eq!(model.gate.epoch, frozen_epoch);
    }
}

#[test]
fn guardian_observation_shape_matches_persisted_raw_hash_boundaries() {
    let canonical = active_model().current_guardian_runtime_observation;

    let mut zero_length_incomplete = canonical.clone();
    zero_length_incomplete.programdata_header_present = false;
    zero_length_incomplete.programdata_data_length = 0;
    zero_length_incomplete.slot = 0;
    zero_length_incomplete.capacity = 0;
    zero_length_incomplete.authority = None;
    zero_length_incomplete.raw_hash_complete = false;
    zero_length_incomplete.raw_hash = [0; 32];
    let mut invalid = active_model();
    assert_atomic_error(
        &mut invalid,
        Release1ModelAction::SetGuardianRuntimeObservation(zero_length_incomplete.clone()),
        Release1ModelError::InvalidInitialization,
    );

    let mut zero_length_complete = zero_length_incomplete;
    zero_length_complete.raw_hash_complete = true;
    zero_length_complete.raw_hash = [91; 32];
    let mut missing_programdata = active_model();
    apply_ok(
        &mut missing_programdata,
        Release1ModelAction::SetGuardianRuntimeObservation(zero_length_complete.clone()),
    );
    apply_ok(
        &mut missing_programdata,
        Release1ModelAction::GuardianFreeze {
            guardian: graph().guardian,
            slot: 30,
            reason: 9,
        },
    );
    assert_eq!(
        missing_programdata.emergency_freeze_observations[&missing_programdata.gate.epoch]
            .programdata,
        zero_length_complete
    );

    let mut zero_slot_header = canonical.clone();
    zero_slot_header.slot = 0;
    let mut zero_capacity_header = canonical.clone();
    zero_capacity_header.programdata_data_length = LOADER_V3_PROGRAMDATA_METADATA_LEN_V1;
    zero_capacity_header.capacity = 0;
    zero_capacity_header.raw_hash = [92; 32];
    for runtime_observation in [zero_slot_header, zero_capacity_header] {
        let mut model = active_model();
        apply_ok(
            &mut model,
            Release1ModelAction::SetGuardianRuntimeObservation(runtime_observation.clone()),
        );
        apply_ok(
            &mut model,
            Release1ModelAction::GuardianFreeze {
                guardian: graph().guardian,
                slot: 30,
                reason: 9,
            },
        );
        assert_eq!(
            model.emergency_freeze_observations[&model.gate.epoch].programdata, runtime_observation,
            "schema-valid malformed header evidence must remain recordable"
        );
    }

    let mut at_ceiling_but_incomplete = canonical;
    assert_eq!(
        at_ceiling_but_incomplete.programdata_data_length,
        MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
    );
    at_ceiling_but_incomplete.raw_hash_complete = false;
    at_ceiling_but_incomplete.raw_hash = [0; 32];
    let mut invalid = active_model();
    assert_atomic_error(
        &mut invalid,
        Release1ModelAction::SetGuardianRuntimeObservation(at_ceiling_but_incomplete),
        Release1ModelError::InvalidInitialization,
    );
}

#[test]
fn emergency_to_upgrade_conversion_is_continuously_frozen_and_stales_resolution() {
    let mut model = active_model();
    apply_ok(
        &mut model,
        Release1ModelAction::GuardianFreeze {
            guardian: graph().guardian,
            slot: 30,
            reason: 9,
        },
    );
    let emergency_epoch = model.gate.epoch;
    let resolution_id = match model
        .apply(Release1ModelAction::CreateEmergencyResolution { slot: 31 })
        .unwrap()
    {
        Release1ModelOutcome::EmergencyResolutionCreated(id) => id,
        outcome => panic!("unexpected emergency outcome: {outcome:?}"),
    };
    let guardian_observation = model.emergency_freeze_observations[&emergency_epoch].clone();
    let mut drifted_programdata = model.programdata.clone();
    drifted_programdata.raw_hash = [88; 32];
    apply_ok(
        &mut model,
        Release1ModelAction::SetProgramDataObservation(drifted_programdata.clone()),
    );
    let (primary, rollback) = create_pair(&mut model, ProposalClassV1::RoutineUpgrade, false, 32);
    assert_eq!(
        model.proposals[&primary].creation_gate_status,
        GateStatusV1::EmergencyFrozen
    );
    assert_eq!(
        model.proposals[&primary].creation_gate_epoch,
        emergency_epoch
    );
    assert_eq!(
        model.proposals[&primary].creation_programdata, drifted_programdata,
        "the repair proposal binds the exact post-freeze current ProgramData"
    );
    assert_eq!(
        model.proposals[&primary].creation_emergency_observation,
        Some(guardian_observation.clone())
    );
    assert_ne!(
        guardian_observation.programdata,
        canonical_guardian_observation(&model.proposals[&primary].creation_programdata),
        "continuous conversion preserves the guardian baseline and separately binds current drift"
    );
    prepare_proposal(&mut model, primary);
    let conversion_slot = model.proposals[&primary].timing.not_before_slot;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ConvertEmergencyFreeze {
            proposal_id: primary,
            slot: conversion_slot,
        },
        Release1ModelError::InvalidRollbackLink,
    );
    assert_eq!(model.gate.status, GateStatusV1::EmergencyFrozen);
    assert_eq!(model.gate.epoch, emergency_epoch);
    assert_eq!(model.target_nonce, 1);
    prepare_proposal(&mut model, rollback);
    assert_eq!(model.gate.status, GateStatusV1::EmergencyFrozen);
    let proposal_creation_programdata = model.proposals[&primary].creation_programdata.clone();
    let mut drifted_after_queue = proposal_creation_programdata.clone();
    drifted_after_queue.raw_hash = [89; 32];
    apply_ok(
        &mut model,
        Release1ModelAction::SetProgramDataObservation(drifted_after_queue),
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ConvertEmergencyFreeze {
            proposal_id: primary,
            slot: conversion_slot,
        },
        Release1ModelError::ProgramDataChanged,
    );
    assert_eq!(model.gate.status, GateStatusV1::EmergencyFrozen);
    assert_eq!(model.gate.epoch, emergency_epoch);
    assert_eq!(model.target_nonce, 1);
    apply_ok(
        &mut model,
        Release1ModelAction::SetProgramDataObservation(proposal_creation_programdata),
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary,
            slot: conversion_slot,
        },
        Release1ModelError::InvalidGate,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ConvertEmergencyFreeze {
            proposal_id: primary,
            slot: conversion_slot,
        },
    );
    assert_eq!(model.gate.status, GateStatusV1::FrozenForUpgrade);
    assert_eq!(model.gate.epoch, emergency_epoch + 1);
    assert_eq!(model.target_nonce, 2);
    assert_eq!(model.gate.active_proposal, Some(primary));
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ApproveEmergencyResolution {
            resolution_id,
            seat: 0,
            slot: conversion_slot + 1,
        },
        Release1ModelError::StaleBinding,
    );
}

#[test]
fn rotation_uses_old_council_and_stales_incomplete_initial_approvals() {
    let short_delays = ModelDelays {
        review_slots: 5,
        rollback_slots: 1,
        routine_slots: 1,
        major_slots: 1,
        terminal_slots: 1,
        proposal_expiry_slots: 100,
    };
    let mut model = active_model_with_delays(short_delays);
    let (primary, _) = create_pair(
        &mut model,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::AdoptBuffer {
            proposal_id: primary,
        },
    );
    apply_ok(
        &mut model,
        Release1ModelAction::VerifyBuffer {
            proposal_id: primary,
        },
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ApproveProposal {
            proposal_id: primary,
            seat: 0,
            slot: PROPOSAL_SLOT + 1,
        },
    );

    let rotation_id = create_rotation(&mut model, seats(80), PROPOSAL_SLOT);
    apply_ok(
        &mut model,
        Release1ModelAction::ApproveCouncilRotation {
            rotation_id,
            seat: 0,
            slot: PROPOSAL_SLOT,
        },
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ApproveCouncilRotation {
            rotation_id,
            seat: 0,
            slot: PROPOSAL_SLOT,
        },
        Release1ModelError::DuplicateApproval,
    );
    for seat in 1..3 {
        apply_ok(
            &mut model,
            Release1ModelAction::ApproveCouncilRotation {
                rotation_id,
                seat,
                slot: PROPOSAL_SLOT,
            },
        );
    }
    apply_ok(
        &mut model,
        Release1ModelAction::QueueCouncilRotation {
            rotation_id,
            slot: PROPOSAL_SLOT,
        },
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ActivateCouncilRotation {
            rotation_id,
            slot: PROPOSAL_SLOT + 1,
        },
    );
    assert_eq!(model.council.version, 2);
    assert_eq!(model.council.seats, seats(80));
    assert_eq!(
        model.rotations[&rotation_id].state,
        CouncilRotationStateV1::Activated
    );
    assert_eq!(model.proposals[&primary].creation_council_version, 1);
    assert_eq!(model.proposals[&primary].initial_approvals.bitset, 1);
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ApproveProposal {
            proposal_id: primary,
            seat: 1,
            slot: PROPOSAL_SLOT + 2,
        },
        Release1ModelError::StaleBinding,
    );

    let invalid_candidate_version = model.next_candidate_council_version;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::CreateCandidateCouncilSet {
            candidate_version: invalid_candidate_version,
            candidate_seats: [key(90), key(90), key(92), key(93), key(94)],
            candidate_seat_terms: seat_terms(1, 10_000),
            activation_slot: 31,
            slot: 30,
        },
        Release1ModelError::InvalidCouncil,
    );
}

#[test]
fn immutable_candidate_and_rotation_pdas_allow_safe_version_gaps() {
    let mut model = active_model();
    assert_eq!(create_candidate_council(&mut model, 2, seats(80), 21), 2);
    assert_atomic_error(
        &mut model,
        Release1ModelAction::CreateCandidateCouncilSet {
            candidate_version: 2,
            candidate_seats: seats(90),
            candidate_seat_terms: seat_terms(1, 10_000),
            activation_slot: 41,
            slot: 21,
        },
        Release1ModelError::InvalidCouncil,
    );
    assert_eq!(
        create_candidate_council(&mut model, 4, seats(100), 22),
        4,
        "an unused version gap remains available when the immediate next candidate is unusable"
    );
    assert_eq!(model.next_candidate_council_version, 5);
    assert_atomic_error(
        &mut model,
        Release1ModelAction::CreateCouncilRotation {
            candidate_version: 3,
            slot: 22,
        },
        Release1ModelError::NotFound,
    );
    let rotation_id = match model
        .apply(Release1ModelAction::CreateCouncilRotation {
            candidate_version: 4,
            slot: 22,
        })
        .expect("rotation binds the immutable gap candidate")
    {
        Release1ModelOutcome::CouncilRotationCreated(id) => id,
        outcome => panic!("unexpected rotation outcome: {outcome:?}"),
    };
    assert_eq!(rotation_id, 4);
    assert_atomic_error(
        &mut model,
        Release1ModelAction::CreateCouncilRotation {
            candidate_version: 4,
            slot: 22,
        },
        Release1ModelError::InvalidStateTransition,
    );
    approve_and_queue_rotation(&mut model, rotation_id, 23);
    let activation_slot = model.rotations[&rotation_id].not_before_slot;
    apply_ok(
        &mut model,
        Release1ModelAction::ActivateCouncilRotation {
            rotation_id,
            slot: activation_slot,
        },
    );
    assert_eq!(model.council.version, 4);
}

#[test]
fn completed_creation_council_quorum_survives_rotation_through_freeze() {
    let short_delays = ModelDelays {
        review_slots: 5,
        rollback_slots: 1,
        routine_slots: 1,
        major_slots: 1,
        terminal_slots: 1,
        proposal_expiry_slots: 100,
    };
    let mut model = active_model_with_delays(short_delays);
    let (primary, rollback) = create_pair(
        &mut model,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    for proposal_id in [primary, rollback] {
        apply_ok(&mut model, Release1ModelAction::AdoptBuffer { proposal_id });
        apply_ok(
            &mut model,
            Release1ModelAction::VerifyBuffer { proposal_id },
        );
        let review_start_slot = model.proposals[&proposal_id].timing.review_start_slot;
        for seat in 0..3 {
            apply_ok(
                &mut model,
                Release1ModelAction::ApproveProposal {
                    proposal_id,
                    seat,
                    slot: review_start_slot,
                },
            );
        }
        assert_eq!(
            model.proposals[&proposal_id].state,
            ProposalStateV2::CouncilApproved
        );
        assert_eq!(
            model.proposals[&proposal_id]
                .initial_approvals
                .council_version,
            1
        );
    }

    create_and_activate_rotation(&mut model, seats(80), 23);
    assert_eq!(model.council.version, 2);
    let completed_primary_approvals = model.proposals[&primary].initial_approvals.clone();
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ApproveProposal {
            proposal_id: primary,
            seat: 3,
            slot: 24,
        },
        Release1ModelError::TimingViolation,
    );
    assert_eq!(
        model.proposals[&primary].initial_approvals, completed_primary_approvals,
        "rotation cannot append a current-council vote to a completed creation-council quorum"
    );
    for proposal_id in [primary, rollback] {
        let review_end_slot = model.proposals[&proposal_id].timing.review_end_slot;
        apply_ok(
            &mut model,
            Release1ModelAction::SatisfyGovernance {
                proposal_id,
                slot: review_end_slot,
            },
        );
        apply_ok(
            &mut model,
            Release1ModelAction::QueueProposal {
                proposal_id,
                slot: review_end_slot,
            },
        );
        assert_eq!(
            model.proposals[&proposal_id]
                .initial_approvals
                .council_version,
            1,
            "a completed initial quorum stays pinned to the immutable creation council"
        );
    }
    let freeze_slot = model.proposals[&rollback]
        .timing
        .not_before_slot
        .max(model.proposals[&primary].timing.not_before_slot);
    apply_ok(
        &mut model,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary,
            slot: freeze_slot,
        },
    );
    assert_eq!(model.gate.active_proposal, Some(primary));
    assert_eq!(model.proposals[&primary].state, ProposalStateV2::Frozen);
}

#[test]
fn poststate_and_unfreeze_use_current_council_across_rotation() {
    let mut model = active_model();
    let (primary, rollback) = create_pair(
        &mut model,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    prepare_proposal(&mut model, primary);
    prepare_proposal(&mut model, rollback);
    let frozen_slot = freeze_prepared_primary(&mut model, primary);
    approve_and_finalize_checkpoint(
        &mut model,
        primary,
        StateCheckpointPhaseV1::Prestate,
        frozen_slot + 1,
    );
    let primary_upgrade_slot = primary_execution_slot(&model, primary, frozen_slot + 3);
    apply_ok(
        &mut model,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: primary,
            slot: primary_upgrade_slot,
        },
    );
    apply_ok(
        &mut model,
        Release1ModelAction::VerifyProgramData {
            proposal_id: primary,
            slot: primary_upgrade_slot,
        },
    );
    let accepted_prestate = model.proposals[&primary].prestate.clone().unwrap();
    create_and_activate_rotation(&mut model, seats(80), 50);
    assert_eq!(model.council.version, 2);
    assert_eq!(model.proposals[&primary].prestate, Some(accepted_prestate));
    assert_eq!(
        model.proposals[&primary].state,
        ProposalStateV2::ProgramDataVerified
    );
    approve_and_finalize_checkpoint(&mut model, primary, StateCheckpointPhaseV1::Poststate, 71);
    let accepted_poststate = model.proposals[&primary].poststate.clone().unwrap();
    assert_eq!(accepted_poststate.approvals.council_version, 2);
    for seat in 0..3 {
        apply_ok(
            &mut model,
            Release1ModelAction::ApproveUnfreeze {
                proposal_id: primary,
                seat,
                slot: 73,
            },
        );
    }
    assert_eq!(model.proposals[&primary].unfreeze_approvals.count, 3);
    assert_eq!(
        model.proposals[&primary].unfreeze_approvals.council_version,
        2
    );
    assert_eq!(
        model.proposals[&primary].state,
        ProposalStateV2::UnfreezeApproved
    );
    let proposal_before_rotation = model.proposals[&primary].clone();
    create_and_activate_rotation(&mut model, seats(100), 74);
    assert_eq!(model.council.version, 3);
    assert_eq!(model.proposals[&primary], proposal_before_rotation);
    assert_eq!(
        model.proposals[&primary].poststate,
        Some(accepted_poststate)
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ExecuteUnfreeze {
            proposal_id: primary,
            slot: 94,
        },
        Release1ModelError::InvalidStateTransition,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ApproveUnfreeze {
            proposal_id: primary,
            seat: 0,
            slot: 95,
        },
    );
    assert_eq!(
        model.proposals[&primary].state,
        ProposalStateV2::PoststateAccepted
    );
    assert_eq!(model.proposals[&primary].unfreeze_approvals.count, 1);
    assert_eq!(
        model.proposals[&primary].unfreeze_approvals.council_version,
        3
    );
    for seat in 1..3 {
        apply_ok(
            &mut model,
            Release1ModelAction::ApproveUnfreeze {
                proposal_id: primary,
                seat,
                slot: 95,
            },
        );
    }
    assert_eq!(
        model.proposals[&primary].unfreeze_approvals.council_version,
        3
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ExecuteUnfreeze {
            proposal_id: primary,
            slot: 96,
        },
    );
    assert_eq!(model.proposals[&primary].state, ProposalStateV2::Completed);
}

#[test]
fn rotation_preserves_governance_objects_and_repins_current_council_work_lazily() {
    let mut frozen = active_model();
    let (primary, rollback) = create_pair(
        &mut frozen,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    prepare_proposal(&mut frozen, primary);
    prepare_proposal(&mut frozen, rollback);
    let frozen_slot = freeze_prepared_primary(&mut frozen, primary);
    for seat in 0..2 {
        apply_ok(
            &mut frozen,
            Release1ModelAction::AttestCheckpoint {
                proposal_id: primary,
                phase: StateCheckpointPhaseV1::Prestate,
                seat,
                hard_state: hard_state(1),
                forbidden_drift_count: 0,
                slot: frozen_slot + 1,
            },
        );
    }
    let proposal_before_rotation = frozen.proposals[&primary].clone();
    create_and_activate_rotation(&mut frozen, seats(80), 40);
    assert_eq!(frozen.proposals[&primary], proposal_before_rotation);
    assert!(frozen.proposals[&primary].prestate.is_none());
    assert_atomic_error(
        &mut frozen,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            attesting_seats: [0, 1, 2],
            slot: 60,
        },
        Release1ModelError::QuorumNotSatisfied,
    );
    for seat in 0..3 {
        apply_ok(
            &mut frozen,
            Release1ModelAction::AttestCheckpoint {
                proposal_id: primary,
                phase: StateCheckpointPhaseV1::Prestate,
                seat,
                hard_state: hard_state(1),
                forbidden_drift_count: 0,
                slot: 61,
            },
        );
    }
    apply_ok(
        &mut frozen,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary,
            phase: StateCheckpointPhaseV1::Prestate,
            attesting_seats: [0, 1, 2],
            slot: 62,
        },
    );
    assert_eq!(
        frozen.proposals[&primary]
            .prestate
            .as_ref()
            .unwrap()
            .approvals
            .council_version,
        2
    );

    let mut emergency = active_model();
    apply_ok(
        &mut emergency,
        Release1ModelAction::GuardianFreeze {
            guardian: graph().guardian,
            slot: 30,
            reason: 9,
        },
    );
    let resolution_id = match emergency
        .apply(Release1ModelAction::CreateEmergencyResolution { slot: 31 })
        .unwrap()
    {
        Release1ModelOutcome::EmergencyResolutionCreated(id) => id,
        outcome => panic!("unexpected emergency outcome: {outcome:?}"),
    };
    for seat in 0..3 {
        apply_ok(
            &mut emergency,
            Release1ModelAction::ApproveEmergencyResolution {
                resolution_id,
                seat,
                slot: 40,
            },
        );
    }
    apply_ok(
        &mut emergency,
        Release1ModelAction::QueueEmergencyResolution {
            resolution_id,
            slot: 40,
        },
    );
    for seat in 0..2 {
        apply_ok(
            &mut emergency,
            Release1ModelAction::AttestEmergencyCheckpoint {
                resolution_id,
                seat,
                hard_state: hard_state(1),
                forbidden_drift_count: 0,
                slot: 41,
            },
        );
    }
    let resolution_before_rotation = emergency.emergency_resolutions[&resolution_id].clone();
    create_and_activate_rotation(&mut emergency, seats(80), 42);
    let resolution = &emergency.emergency_resolutions[&resolution_id];
    assert_eq!(resolution, &resolution_before_rotation);
    assert_eq!(
        resolution.state,
        EmergencyFreezeResolutionStateV1::Timelocked
    );
    assert_eq!(resolution.approvals.count, 3);
    assert_eq!(resolution.approvals.council_version, 1);
    assert!(resolution.checkpoint.is_none());
    assert_eq!(resolution.checkpoint_attestations.len(), 2);
    assert!(resolution
        .checkpoint_attestations
        .iter()
        .all(|attestation| attestation.council_version == 1));
    apply_ok(
        &mut emergency,
        Release1ModelAction::ApproveEmergencyResolution {
            resolution_id,
            seat: 0,
            slot: 63,
        },
    );
    assert_eq!(
        emergency.emergency_resolutions[&resolution_id].state,
        EmergencyFreezeResolutionStateV1::Draft
    );
    assert_eq!(
        emergency.emergency_resolutions[&resolution_id]
            .approvals
            .count,
        1
    );
    assert_eq!(
        emergency.emergency_resolutions[&resolution_id]
            .approvals
            .council_version,
        2
    );
    for seat in 1..3 {
        apply_ok(
            &mut emergency,
            Release1ModelAction::ApproveEmergencyResolution {
                resolution_id,
                seat,
                slot: 63,
            },
        );
    }
    apply_ok(
        &mut emergency,
        Release1ModelAction::QueueEmergencyResolution {
            resolution_id,
            slot: 63,
        },
    );
    assert_atomic_error(
        &mut emergency,
        Release1ModelAction::FinalizeEmergencyCheckpoint {
            resolution_id,
            attesting_seats: [0, 1, 2],
            slot: 64,
        },
        Release1ModelError::QuorumNotSatisfied,
    );
    for seat in 0..3 {
        apply_ok(
            &mut emergency,
            Release1ModelAction::AttestEmergencyCheckpoint {
                resolution_id,
                seat,
                hard_state: hard_state(1),
                forbidden_drift_count: 0,
                slot: 64,
            },
        );
    }
    apply_ok(
        &mut emergency,
        Release1ModelAction::FinalizeEmergencyCheckpoint {
            resolution_id,
            attesting_seats: [0, 1, 2],
            slot: 65,
        },
    );
    assert_eq!(
        emergency.emergency_resolutions[&resolution_id]
            .checkpoint
            .as_ref()
            .expect("current council checkpoint")
            .approvals
            .council_version,
        2
    );
}

#[test]
fn rotation_mutations_enforce_creation_expiry_and_current_nonce() {
    let mut timing = active_model();
    let rotation_id = create_rotation(&mut timing, seats(80), 21);
    let expiry_slot = timing.rotations[&rotation_id].expiry_slot;
    assert_atomic_error(
        &mut timing,
        Release1ModelAction::ApproveCouncilRotation {
            rotation_id,
            seat: 0,
            slot: 20,
        },
        Release1ModelError::StaleBinding,
    );
    assert_atomic_error(
        &mut timing,
        Release1ModelAction::ApproveCouncilRotationCancellation {
            rotation_id,
            seat: 0,
            slot: 20,
            reason: 71,
        },
        Release1ModelError::StaleBinding,
    );
    let mut approval_after_expiry = timing.clone();
    assert_atomic_error(
        &mut approval_after_expiry,
        Release1ModelAction::ApproveCouncilRotation {
            rotation_id,
            seat: 0,
            slot: expiry_slot,
        },
        Release1ModelError::StaleBinding,
    );
    for seat in 0..3 {
        apply_ok(
            &mut timing,
            Release1ModelAction::ApproveCouncilRotation {
                rotation_id,
                seat,
                slot: 21,
            },
        );
    }
    assert_atomic_error(
        &mut timing,
        Release1ModelAction::QueueCouncilRotation {
            rotation_id,
            slot: 20,
        },
        Release1ModelError::InvalidStateTransition,
    );
    assert_atomic_error(
        &mut timing,
        Release1ModelAction::QueueCouncilRotation {
            rotation_id,
            slot: expiry_slot,
        },
        Release1ModelError::InvalidStateTransition,
    );

    let mut nonce = active_model();
    let draft_rotation = create_rotation(&mut nonce, seats(80), 21);
    let approved_rotation = create_rotation(&mut nonce, seats(90), 22);
    for seat in 0..3 {
        apply_ok(
            &mut nonce,
            Release1ModelAction::ApproveCouncilRotation {
                rotation_id: approved_rotation,
                seat,
                slot: 22,
            },
        );
    }
    let (primary, rollback) = create_pair(&mut nonce, ProposalClassV1::RoutineUpgrade, false, 24);
    prepare_proposal(&mut nonce, primary);
    prepare_proposal(&mut nonce, rollback);
    let freeze_slot = freeze_prepared_primary(&mut nonce, primary);
    assert_atomic_error(
        &mut nonce,
        Release1ModelAction::ApproveCouncilRotation {
            rotation_id: draft_rotation,
            seat: 0,
            slot: freeze_slot + 1,
        },
        Release1ModelError::StaleBinding,
    );
    assert_atomic_error(
        &mut nonce,
        Release1ModelAction::QueueCouncilRotation {
            rotation_id: approved_rotation,
            slot: freeze_slot + 1,
        },
        Release1ModelError::InvalidStateTransition,
    );
}

#[test]
fn delayed_rotation_rejects_candidate_seats_expired_after_planned_activation() {
    let mut model = active_model();
    let candidate_version = model.next_candidate_council_version;
    assert_eq!(
        model
            .apply(Release1ModelAction::CreateCandidateCouncilSet {
                candidate_version,
                candidate_seats: seats(80),
                candidate_seat_terms: seat_terms(1, 42),
                activation_slot: 41,
                slot: 21,
            })
            .expect("candidate council creation"),
        Release1ModelOutcome::CandidateCouncilCreated(candidate_version)
    );
    let rotation_id = match model
        .apply(Release1ModelAction::CreateCouncilRotation {
            candidate_version,
            slot: 21,
        })
        .expect("rotation creation")
    {
        Release1ModelOutcome::CouncilRotationCreated(id) => id,
        outcome => panic!("unexpected rotation outcome: {outcome:?}"),
    };
    approve_and_queue_rotation(&mut model, rotation_id, 21);
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ActivateCouncilRotation {
            rotation_id,
            slot: 42,
        },
        Release1ModelError::InactiveSeat,
    );
    assert_eq!(model.council.version, 1);
    assert_eq!(
        model.rotations[&rotation_id].state,
        CouncilRotationStateV1::Timelocked
    );
}

#[test]
fn rotation_nonce_drift_and_old_rotation_identity_fail_closed() {
    let mut model = active_model();
    let rotation_id = create_rotation(&mut model, seats(80), 21);
    for seat in 0..3 {
        apply_ok(
            &mut model,
            Release1ModelAction::ApproveCouncilRotation {
                rotation_id,
                seat,
                slot: 21,
            },
        );
    }
    apply_ok(
        &mut model,
        Release1ModelAction::QueueCouncilRotation {
            rotation_id,
            slot: 21,
        },
    );

    let (primary, rollback) = create_pair(
        &mut model,
        ProposalClassV1::RoutineUpgrade,
        false,
        PROPOSAL_SLOT,
    );
    prepare_proposal(&mut model, primary);
    prepare_proposal(&mut model, rollback);
    freeze_prepared_primary(&mut model, primary);
    let rotation_not_before = model.rotations[&rotation_id].not_before_slot;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ActivateCouncilRotation {
            rotation_id,
            slot: rotation_not_before,
        },
        Release1ModelError::StaleBinding,
    );
}

#[test]
fn candidate_versions_are_monotonic_across_cancelled_expired_and_losing_rotations() {
    let mut model = active_model();
    assert_eq!(model.council.version, 1);
    assert_eq!(model.next_candidate_council_version, 2);

    let cancelled = create_rotation(&mut model, seats(80), 21);
    assert_eq!(model.rotations[&cancelled].candidate.version, 2);
    assert_eq!(model.next_candidate_council_version, 3);
    for seat in 0..2 {
        apply_ok(
            &mut model,
            Release1ModelAction::ApproveCouncilRotationCancellation {
                rotation_id: cancelled,
                seat,
                slot: 22,
                reason: 71,
            },
        );
    }
    assert_eq!(model.rotations[&cancelled].cancellation_reason, 71);
    assert_eq!(model.rotations[&cancelled].terminal_reason, 0);
    apply_ok(
        &mut model,
        Release1ModelAction::ApproveCouncilRotationCancellation {
            rotation_id: cancelled,
            seat: 2,
            slot: 22,
            reason: 71,
        },
    );
    assert_eq!(
        model.rotations[&cancelled].state,
        CouncilRotationStateV1::Cancelled
    );
    assert_eq!(model.rotations[&cancelled].cancellation_reason, 71);
    assert_eq!(model.rotations[&cancelled].terminal_reason, 71);
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ApproveCouncilRotationCancellation {
            rotation_id: cancelled,
            seat: 3,
            slot: 23,
            reason: 71,
        },
        Release1ModelError::StaleBinding,
    );

    let expired = create_rotation(&mut model, seats(90), 23);
    assert_eq!(model.rotations[&expired].candidate.version, 3);
    let expiry_slot = model.rotations[&expired].expiry_slot;
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ExpireCouncilRotation {
            rotation_id: expired,
            slot: expiry_slot - 1,
        },
        Release1ModelError::TimingViolation,
    );
    apply_ok(
        &mut model,
        Release1ModelAction::ExpireCouncilRotation {
            rotation_id: expired,
            slot: expiry_slot,
        },
    );
    assert_eq!(
        model.rotations[&expired].state,
        CouncilRotationStateV1::Expired
    );
    assert_eq!(model.rotations[&expired].terminal_slot, expiry_slot);
    assert_eq!(
        model.rotations[&expired].terminal_reason,
        COUNCIL_ROTATION_EXPIRED_TERMINAL_REASON_V1
    );

    let losing = create_rotation(&mut model, seats(100), expiry_slot + 1);
    let winning = create_rotation(&mut model, seats(110), expiry_slot + 2);
    assert_eq!(model.rotations[&losing].candidate.version, 4);
    assert_eq!(model.rotations[&winning].candidate.version, 5);
    approve_and_queue_rotation(&mut model, losing, expiry_slot + 3);
    approve_and_queue_rotation(&mut model, winning, expiry_slot + 3);
    let winning_not_before = model.rotations[&winning].not_before_slot;
    apply_ok(
        &mut model,
        Release1ModelAction::ActivateCouncilRotation {
            rotation_id: winning,
            slot: winning_not_before,
        },
    );
    assert_eq!(model.council.version, 5, "candidate-version gaps are valid");
    assert_eq!(model.rotations[&winning].terminal_slot, winning_not_before);
    assert_eq!(
        model.rotations[&winning].terminal_reason,
        COUNCIL_ROTATION_ACTIVATED_TERMINAL_REASON_V1
    );
    assert_atomic_error(
        &mut model,
        Release1ModelAction::ActivateCouncilRotation {
            rotation_id: losing,
            slot: winning_not_before,
        },
        Release1ModelError::StaleBinding,
    );

    let replacement = create_rotation(&mut model, seats(120), winning_not_before + 1);
    assert_eq!(model.rotations[&replacement].candidate.version, 6);
    assert_eq!(model.next_candidate_council_version, 7);
    approve_and_queue_rotation(&mut model, replacement, winning_not_before + 2);
    let replacement_not_before = model.rotations[&replacement].not_before_slot;
    apply_ok(
        &mut model,
        Release1ModelAction::ActivateCouncilRotation {
            rotation_id: replacement,
            slot: replacement_not_before,
        },
    );
    assert_eq!(model.council.version, 6);
    assert_eq!(
        model.rotations[&replacement].terminal_reason,
        COUNCIL_ROTATION_ACTIVATED_TERMINAL_REASON_V1
    );

    let mut version_overflow = active_model();
    version_overflow.next_candidate_council_version = u64::MAX;
    assert_atomic_error(
        &mut version_overflow,
        Release1ModelAction::CreateCandidateCouncilSet {
            candidate_version: u64::MAX,
            candidate_seats: seats(80),
            candidate_seat_terms: seat_terms(1, 10_000),
            activation_slot: 41,
            slot: 21,
        },
        Release1ModelError::ArithmeticOverflow,
    );
}

#[test]
fn proposal_transition_relation_exhaustively_allows_only_release1_edges() {
    let states = [
        ProposalStateV2::Draft,
        ProposalStateV2::BufferAdopted,
        ProposalStateV2::BufferVerified,
        ProposalStateV2::CouncilApproved,
        ProposalStateV2::TokenReviewOpen,
        ProposalStateV2::GovernanceSatisfied,
        ProposalStateV2::Timelocked,
        ProposalStateV2::Frozen,
        ProposalStateV2::Extended,
        ProposalStateV2::UpgradeExecuted,
        ProposalStateV2::ProgramDataVerified,
        ProposalStateV2::PoststateAccepted,
        ProposalStateV2::UnfreezeApproved,
        ProposalStateV2::Completed,
        ProposalStateV2::Cancelled,
        ProposalStateV2::Expired,
        ProposalStateV2::SupersededByRollback,
        ProposalStateV2::Retired,
    ];
    for extension_required in [false, true] {
        for current in states {
            for next in states {
                let expected = match (current, next) {
                    (ProposalStateV2::Draft, ProposalStateV2::BufferAdopted)
                    | (ProposalStateV2::BufferAdopted, ProposalStateV2::BufferVerified)
                    | (ProposalStateV2::BufferVerified, ProposalStateV2::CouncilApproved)
                    | (ProposalStateV2::CouncilApproved, ProposalStateV2::GovernanceSatisfied)
                    | (ProposalStateV2::GovernanceSatisfied, ProposalStateV2::Timelocked)
                    | (ProposalStateV2::Timelocked, ProposalStateV2::Frozen)
                    | (ProposalStateV2::Extended, ProposalStateV2::UpgradeExecuted)
                    | (ProposalStateV2::UpgradeExecuted, ProposalStateV2::ProgramDataVerified)
                    | (ProposalStateV2::ProgramDataVerified, ProposalStateV2::PoststateAccepted)
                    | (ProposalStateV2::PoststateAccepted, ProposalStateV2::UnfreezeApproved)
                    | (ProposalStateV2::UnfreezeApproved, ProposalStateV2::Completed) => true,
                    (ProposalStateV2::Frozen, ProposalStateV2::Extended) => extension_required,
                    (ProposalStateV2::Frozen, ProposalStateV2::UpgradeExecuted) => {
                        !extension_required
                    }
                    _ => false,
                };
                assert_eq!(
                    proposal_edge_allowed(current, next, extension_required),
                    expected,
                    "edge {current:?} -> {next:?}, extension={extension_required}"
                );
            }
        }
    }
}

#[test]
fn rotation_can_be_evaluated_at_every_proposal_lifecycle_state() {
    let states = [
        ProposalStateV2::Draft,
        ProposalStateV2::BufferAdopted,
        ProposalStateV2::BufferVerified,
        ProposalStateV2::CouncilApproved,
        ProposalStateV2::TokenReviewOpen,
        ProposalStateV2::GovernanceSatisfied,
        ProposalStateV2::Timelocked,
        ProposalStateV2::Frozen,
        ProposalStateV2::Extended,
        ProposalStateV2::UpgradeExecuted,
        ProposalStateV2::ProgramDataVerified,
        ProposalStateV2::PoststateAccepted,
        ProposalStateV2::UnfreezeApproved,
        ProposalStateV2::Completed,
        ProposalStateV2::Cancelled,
        ProposalStateV2::Expired,
        ProposalStateV2::SupersededByRollback,
        ProposalStateV2::Retired,
    ];
    for state in states {
        let mut model = active_model();
        let (primary, _) = create_pair(
            &mut model,
            ProposalClassV1::RoutineUpgrade,
            false,
            PROPOSAL_SLOT,
        );
        let rotation_id = create_rotation(&mut model, seats(80), 30);
        for seat in 0..3 {
            apply_ok(
                &mut model,
                Release1ModelAction::ApproveCouncilRotation {
                    rotation_id,
                    seat,
                    slot: 30,
                },
            );
        }
        apply_ok(
            &mut model,
            Release1ModelAction::QueueCouncilRotation {
                rotation_id,
                slot: 30,
            },
        );
        model.proposals.get_mut(&primary).unwrap().state = state;
        apply_ok(
            &mut model,
            Release1ModelAction::ActivateCouncilRotation {
                rotation_id,
                slot: 50,
            },
        );
        assert_eq!(model.council.version, 2, "rotation during {state:?}");
        assert_eq!(
            model.proposals[&primary].creation_council_version, 1,
            "historical creation council during {state:?}"
        );
    }
}

fn replay_checked_apply(
    model: &mut Release1Model,
    action: Release1ModelAction,
) -> Result<Release1ModelOutcome, Release1ModelError> {
    let before = model.clone();
    let before_bytes = to_vec(&before).expect("serialize generated trace prestate");
    let mut replay = before.clone();
    let result = model.apply(action.clone());
    let replay_result = replay.apply(action);
    assert_eq!(result, replay_result, "reference-model replay diverged");
    assert_eq!(
        to_vec(&*model).expect("serialize generated trace result"),
        to_vec(&replay).expect("serialize generated trace replay"),
        "reference-model replay produced different consensus bytes"
    );
    if result.is_err() {
        assert_eq!(
            *model, before,
            "failed generated action mutated model state"
        );
        assert_eq!(
            to_vec(&*model).expect("serialize generated failure result"),
            before_bytes,
            "failed generated action changed serialized model bytes"
        );
    }
    result
}

fn replay_checked_ok(model: &mut Release1Model, action: Release1ModelAction) {
    assert_eq!(
        replay_checked_apply(model, action).expect("generated action must succeed"),
        Release1ModelOutcome::Applied
    );
}

fn generated_quorum(mut seed: u64) -> [u8; 3] {
    // A tiny deterministic generator is preferable to a new test dependency.
    // Rejection sampling covers different three-of-five masks and orderings.
    let mut seats = [u8::MAX; 3];
    let mut filled = 0;
    while filled < seats.len() {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let candidate = ((seed >> 32) % 5) as u8;
        if !seats[..filled].contains(&candidate) {
            seats[filled] = candidate;
            filled += 1;
        }
    }
    seats
}

fn replay_checked_create(model: &mut Release1Model, request: ModelProposalRequest) -> u64 {
    match replay_checked_apply(model, Release1ModelAction::CreateProposal(request))
        .expect("generated proposal creation")
    {
        Release1ModelOutcome::ProposalCreated(id) => id,
        outcome => panic!("unexpected generated proposal outcome: {outcome:?}"),
    }
}

fn replay_checked_pair(model: &mut Release1Model, seed: u64) -> (u64, u64) {
    let primary_id = model.next_proposal_id;
    let rollback_id = primary_id.checked_add(1).expect("generated rollback id");
    let extension_required = seed & 1 != 0;
    assert_eq!(
        replay_checked_create(
            model,
            create_request(
                ProposalClassV1::RoutineUpgrade,
                extension_required,
                None,
                Some(rollback_id),
                PROPOSAL_SLOT,
            ),
        ),
        primary_id
    );
    let rollback_slot = PROPOSAL_SLOT + model.delays.rollback_slots + 1;
    assert_eq!(
        replay_checked_create(
            model,
            create_request(
                ProposalClassV1::EmergencyRollback,
                false,
                Some(primary_id),
                None,
                rollback_slot,
            ),
        ),
        rollback_id
    );
    (primary_id, rollback_id)
}

fn replay_checked_prepare(model: &mut Release1Model, proposal_id: u64, seed: u64) {
    replay_checked_ok(model, Release1ModelAction::AdoptBuffer { proposal_id });
    assert_eq!(
        replay_checked_apply(model, Release1ModelAction::AdoptBuffer { proposal_id }),
        Err(Release1ModelError::InvalidStateTransition)
    );
    replay_checked_ok(model, Release1ModelAction::VerifyBuffer { proposal_id });
    let timing = model.proposals[&proposal_id].timing;
    let quorum = generated_quorum(seed);
    for seat in quorum {
        replay_checked_ok(
            model,
            Release1ModelAction::ApproveProposal {
                proposal_id,
                seat,
                slot: timing.review_start_slot,
            },
        );
    }
    assert_eq!(
        replay_checked_apply(
            model,
            Release1ModelAction::ApproveProposal {
                proposal_id,
                seat: quorum[0],
                slot: timing.review_start_slot,
            },
        ),
        Err(Release1ModelError::TimingViolation)
    );
    replay_checked_ok(
        model,
        Release1ModelAction::SatisfyGovernance {
            proposal_id,
            slot: timing.review_end_slot,
        },
    );
    replay_checked_ok(
        model,
        Release1ModelAction::QueueProposal {
            proposal_id,
            slot: timing.review_end_slot,
        },
    );
}

fn replay_checked_checkpoint(
    model: &mut Release1Model,
    proposal_id: u64,
    phase: StateCheckpointPhaseV1,
    slot: u64,
    seed: u64,
) {
    let quorum = generated_quorum(seed);
    for seat in quorum {
        replay_checked_ok(
            model,
            Release1ModelAction::AttestCheckpoint {
                proposal_id,
                phase,
                seat,
                hard_state: hard_state(1),
                forbidden_drift_count: 0,
                slot,
            },
        );
    }
    replay_checked_ok(
        model,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id,
            phase,
            attesting_seats: quorum,
            slot: slot + 1,
        },
    );
    assert_eq!(
        replay_checked_apply(
            model,
            Release1ModelAction::FinalizeCheckpoint {
                proposal_id,
                phase,
                attesting_seats: quorum,
                slot: slot + 1,
            },
        ),
        Err(Release1ModelError::InvalidStateTransition)
    );
}

fn replay_checked_complete_frozen_proposal(model: &mut Release1Model, proposal_id: u64, seed: u64) {
    let frozen_slot = model.proposals[&proposal_id].frozen_slot;
    replay_checked_checkpoint(
        model,
        proposal_id,
        StateCheckpointPhaseV1::Prestate,
        frozen_slot + 1,
        seed,
    );
    let mut execution_slot =
        if model.proposals[&proposal_id].class == ProposalClassV1::EmergencyRollback {
            frozen_slot + 3
        } else {
            primary_execution_slot(model, proposal_id, frozen_slot + 3)
        };
    if model.proposals[&proposal_id].extension_required {
        assert_eq!(
            replay_checked_apply(
                model,
                Release1ModelAction::ExecuteUpgrade {
                    proposal_id,
                    slot: execution_slot,
                },
            ),
            Err(Release1ModelError::InvalidStateTransition)
        );
        replay_checked_ok(
            model,
            Release1ModelAction::ExtendTarget {
                proposal_id,
                slot: execution_slot,
            },
        );
        assert_eq!(
            replay_checked_apply(
                model,
                Release1ModelAction::ExecuteUpgrade {
                    proposal_id,
                    slot: execution_slot,
                },
            ),
            Err(Release1ModelError::TimingViolation)
        );
        execution_slot += 1;
    }
    replay_checked_ok(
        model,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id,
            slot: execution_slot,
        },
    );
    assert_eq!(
        replay_checked_apply(
            model,
            Release1ModelAction::ExecuteUpgrade {
                proposal_id,
                slot: execution_slot,
            },
        ),
        Err(Release1ModelError::InvalidStateTransition)
    );
    replay_checked_ok(
        model,
        Release1ModelAction::VerifyProgramData {
            proposal_id,
            slot: execution_slot,
        },
    );
    replay_checked_checkpoint(
        model,
        proposal_id,
        StateCheckpointPhaseV1::Poststate,
        execution_slot + 1,
        seed.rotate_left(17),
    );
    let quorum = generated_quorum(seed.rotate_right(11));
    for seat in quorum {
        replay_checked_ok(
            model,
            Release1ModelAction::ApproveUnfreeze {
                proposal_id,
                seat,
                slot: execution_slot + 3,
            },
        );
    }
    replay_checked_ok(
        model,
        Release1ModelAction::ExecuteUnfreeze {
            proposal_id,
            slot: execution_slot + 4,
        },
    );
    assert_eq!(
        replay_checked_apply(
            model,
            Release1ModelAction::ExecuteUnfreeze {
                proposal_id,
                slot: execution_slot + 4,
            },
        ),
        Err(Release1ModelError::InvalidGate)
    );
}

fn generated_ordinary_trace(seed: u64) {
    let mut model = active_model();
    let (primary, rollback) = replay_checked_pair(&mut model, seed);
    replay_checked_prepare(&mut model, primary, seed);
    replay_checked_prepare(&mut model, rollback, seed.rotate_left(7));
    let freeze_slot = model.proposals[&primary].timing.not_before_slot;
    assert_eq!(
        replay_checked_apply(
            &mut model,
            Release1ModelAction::FreezeProposal {
                proposal_id: primary,
                slot: freeze_slot - 1,
            },
        ),
        Err(Release1ModelError::TimingViolation)
    );
    replay_checked_ok(
        &mut model,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary,
            slot: freeze_slot,
        },
    );
    replay_checked_complete_frozen_proposal(&mut model, primary, seed);
    assert_eq!(model.proposals[&primary].state, ProposalStateV2::Completed);
    assert_eq!(model.proposals[&rollback].state, ProposalStateV2::Retired);
    assert_eq!(model.gate.status, GateStatusV1::Active);
}

fn generated_rollback_trace(seed: u64) {
    let mut model = active_model();
    let (primary, rollback) = replay_checked_pair(&mut model, seed & !1);
    replay_checked_prepare(&mut model, primary, seed);
    replay_checked_prepare(&mut model, rollback, seed.rotate_left(7));
    let freeze_slot = model.proposals[&primary].timing.not_before_slot;
    replay_checked_ok(
        &mut model,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary,
            slot: freeze_slot,
        },
    );
    replay_checked_checkpoint(
        &mut model,
        primary,
        StateCheckpointPhaseV1::Prestate,
        freeze_slot + 1,
        seed,
    );
    let execute_slot = primary_execution_slot(&model, primary, freeze_slot + 3);
    replay_checked_ok(
        &mut model,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: primary,
            slot: execute_slot,
        },
    );
    let rollback_delay_slots = model.delays.rollback_slots;
    assert_eq!(
        replay_checked_apply(
            &mut model,
            Release1ModelAction::ActivateRollback {
                primary_proposal_id: primary,
                rollback_proposal_id: rollback,
                slot: execute_slot + rollback_delay_slots,
            },
        ),
        Err(Release1ModelError::ProgramDataFailureEvidenceRequired)
    );
    replay_checked_ok(
        &mut model,
        Release1ModelAction::RecordProgramDataFailure {
            proposal_id: primary,
            kind: ModelProgramDataFailureKind::PayloadChunk,
            slot: execute_slot,
        },
    );
    let rollback_slot = execute_slot + rollback_delay_slots;
    replay_checked_ok(
        &mut model,
        Release1ModelAction::ActivateRollback {
            primary_proposal_id: primary,
            rollback_proposal_id: rollback,
            slot: rollback_slot,
        },
    );
    assert_eq!(model.gate.status, GateStatusV1::FrozenForUpgrade);
    assert_eq!(model.gate.active_proposal, Some(rollback));
    replay_checked_complete_frozen_proposal(&mut model, rollback, seed.rotate_left(13));
    assert_eq!(model.proposals[&rollback].state, ProposalStateV2::Completed);
    assert_eq!(
        model.proposals[&primary].state,
        ProposalStateV2::SupersededByRollback
    );
}

fn generated_guardian_resume_trace(seed: u64) {
    let mut model = active_model();
    let freeze_slot = 30 + (seed % 3);
    replay_checked_ok(
        &mut model,
        Release1ModelAction::GuardianFreeze {
            guardian: graph().guardian,
            slot: freeze_slot,
            reason: 70 + (seed % 100) as u16,
        },
    );
    assert_eq!(
        replay_checked_apply(
            &mut model,
            Release1ModelAction::GuardianFreeze {
                guardian: graph().guardian,
                slot: freeze_slot,
                reason: 70,
            },
        ),
        Err(Release1ModelError::InvalidGate)
    );
    let resolution_id = match replay_checked_apply(
        &mut model,
        Release1ModelAction::CreateEmergencyResolution {
            slot: freeze_slot + 1,
        },
    )
    .expect("generated emergency resolution")
    {
        Release1ModelOutcome::EmergencyResolutionCreated(id) => id,
        outcome => panic!("unexpected generated emergency outcome: {outcome:?}"),
    };
    let approval_slot = model.emergency_resolutions[&resolution_id].not_before_slot;
    assert!(replay_checked_apply(
        &mut model,
        Release1ModelAction::ApproveEmergencyResolution {
            resolution_id,
            seat: 0,
            slot: approval_slot - 1,
        },
    )
    .is_err());
    let quorum = generated_quorum(seed);
    for seat in quorum {
        replay_checked_ok(
            &mut model,
            Release1ModelAction::ApproveEmergencyResolution {
                resolution_id,
                seat,
                slot: approval_slot,
            },
        );
    }
    replay_checked_ok(
        &mut model,
        Release1ModelAction::QueueEmergencyResolution {
            resolution_id,
            slot: approval_slot,
        },
    );
    for seat in generated_quorum(seed.rotate_left(9)) {
        replay_checked_ok(
            &mut model,
            Release1ModelAction::AttestEmergencyCheckpoint {
                resolution_id,
                seat,
                hard_state: hard_state(1),
                forbidden_drift_count: 0,
                slot: approval_slot + 1,
            },
        );
    }
    replay_checked_ok(
        &mut model,
        Release1ModelAction::FinalizeEmergencyCheckpoint {
            resolution_id,
            attesting_seats: generated_quorum(seed.rotate_left(9)),
            slot: approval_slot + 2,
        },
    );
    replay_checked_ok(
        &mut model,
        Release1ModelAction::ExecuteEmergencyResume {
            resolution_id,
            slot: approval_slot + 3,
        },
    );
    assert_eq!(model.gate.status, GateStatusV1::Active);
    assert_eq!(
        model.emergency_resolutions[&resolution_id].state,
        EmergencyFreezeResolutionStateV1::Executed
    );
}

fn generated_rotation_and_terminal_trace(seed: u64) {
    let mut model = active_model();
    let primary_id = model.next_proposal_id;
    let rollback_id = primary_id + 1;
    let primary = replay_checked_create(
        &mut model,
        create_request(
            ProposalClassV1::RoutineUpgrade,
            false,
            None,
            Some(rollback_id),
            PROPOSAL_SLOT,
        ),
    );
    let cancellation_slot = model.proposals[&primary].timing.review_start_slot;
    let cancellation_quorum = generated_quorum(seed);
    for seat in cancellation_quorum {
        replay_checked_ok(
            &mut model,
            Release1ModelAction::ApproveCancellation {
                proposal_id: primary,
                seat,
                slot: cancellation_slot,
                reason: 81,
            },
        );
    }
    assert_eq!(model.proposals[&primary].state, ProposalStateV2::Cancelled);
    assert_eq!(
        replay_checked_apply(
            &mut model,
            Release1ModelAction::ApproveCancellation {
                proposal_id: primary,
                seat: cancellation_quorum[0],
                slot: cancellation_slot,
                reason: 81,
            },
        ),
        Err(Release1ModelError::InvalidStateTransition)
    );

    let creation_slot = PROPOSAL_SLOT + 1;
    let expiring = replay_checked_create(
        &mut model,
        create_request(
            ProposalClassV1::RoutineUpgrade,
            false,
            None,
            Some(rollback_id + 1),
            creation_slot,
        ),
    );
    let expiry = model.proposals[&expiring].timing.expiry_slot;
    assert_eq!(
        replay_checked_apply(
            &mut model,
            Release1ModelAction::ExpireProposal {
                proposal_id: expiring,
                slot: expiry - 1,
            },
        ),
        Err(Release1ModelError::InvalidStateTransition)
    );
    replay_checked_ok(
        &mut model,
        Release1ModelAction::ExpireProposal {
            proposal_id: expiring,
            slot: expiry,
        },
    );

    let rotation_slot = expiry + 1;
    let candidate_version = model.next_candidate_council_version;
    let activation_slot = rotation_slot + model.delays.major_slots;
    assert_eq!(
        replay_checked_apply(
            &mut model,
            Release1ModelAction::CreateCandidateCouncilSet {
                candidate_version,
                candidate_seats: seats(80),
                candidate_seat_terms: seat_terms(1, 10_000),
                activation_slot,
                slot: rotation_slot,
            },
        )
        .expect("generated candidate council"),
        Release1ModelOutcome::CandidateCouncilCreated(candidate_version)
    );
    let rotation_id = match replay_checked_apply(
        &mut model,
        Release1ModelAction::CreateCouncilRotation {
            candidate_version,
            slot: rotation_slot,
        },
    )
    .expect("generated rotation")
    {
        Release1ModelOutcome::CouncilRotationCreated(id) => id,
        outcome => panic!("unexpected generated rotation outcome: {outcome:?}"),
    };
    let rotation_quorum = generated_quorum(seed.rotate_right(5));
    for seat in rotation_quorum {
        replay_checked_ok(
            &mut model,
            Release1ModelAction::ApproveCouncilRotation {
                rotation_id,
                seat,
                slot: rotation_slot,
            },
        );
    }
    replay_checked_ok(
        &mut model,
        Release1ModelAction::QueueCouncilRotation {
            rotation_id,
            slot: rotation_slot,
        },
    );
    assert!(replay_checked_apply(
        &mut model,
        Release1ModelAction::ActivateCouncilRotation {
            rotation_id,
            slot: activation_slot - 1,
        },
    )
    .is_err());
    replay_checked_ok(
        &mut model,
        Release1ModelAction::ActivateCouncilRotation {
            rotation_id,
            slot: activation_slot,
        },
    );
    assert_eq!(model.council.version, candidate_version);
}

#[test]
fn generated_release1_transition_traces_are_deterministic_atomic_and_complete() {
    // These generated traces vary quorum masks/order, extension choice, slots,
    // and terminal path while replaying every action against an independent
    // clone.  They supplement the exhaustive edge and mask tables above with
    // long stateful sequences that mix accepted and rejected actions.
    for seed in 0..48 {
        match seed % 4 {
            0 => generated_ordinary_trace(seed),
            1 => generated_rollback_trace(seed),
            2 => generated_guardian_resume_trace(seed),
            _ => generated_rotation_and_terminal_trace(seed),
        }
    }

    let mut proposal_overflow = active_model();
    proposal_overflow.next_proposal_id = u64::MAX;
    assert_eq!(
        replay_checked_apply(
            &mut proposal_overflow,
            Release1ModelAction::CreateProposal(create_request(
                ProposalClassV1::RoutineUpgrade,
                false,
                None,
                Some(1),
                PROPOSAL_SLOT,
            )),
        ),
        Err(Release1ModelError::ArithmeticOverflow)
    );

    let mut epoch_overflow = active_model();
    epoch_overflow.gate.epoch = u64::MAX;
    assert_eq!(
        replay_checked_apply(
            &mut epoch_overflow,
            Release1ModelAction::GuardianFreeze {
                guardian: graph().guardian,
                slot: 30,
                reason: 91,
            },
        ),
        Err(Release1ModelError::ArithmeticOverflow)
    );

    let mut nonce_overflow = active_model();
    nonce_overflow.target_nonce = u64::MAX;
    let (primary, rollback) = replay_checked_pair(&mut nonce_overflow, 0);
    replay_checked_prepare(&mut nonce_overflow, primary, 1);
    replay_checked_prepare(&mut nonce_overflow, rollback, 2);
    let freeze_slot = nonce_overflow.proposals[&primary].timing.not_before_slot;
    assert_eq!(
        replay_checked_apply(
            &mut nonce_overflow,
            Release1ModelAction::FreezeProposal {
                proposal_id: primary,
                slot: freeze_slot,
            },
        ),
        Err(Release1ModelError::ArithmeticOverflow)
    );
}
