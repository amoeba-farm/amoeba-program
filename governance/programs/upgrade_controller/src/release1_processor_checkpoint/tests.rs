use borsh::BorshSerialize;
use solana_program::account_info::AccountInfo;

use super::*;
use crate::{
    artifact_merkle::{ARTIFACT_MERKLE_SCHEME_ID, RELEASE1_ARTIFACT_CHUNK_SIZE_V1},
    instruction::{CouncilSeatTermV1, EmergencyResolutionExpectationV1},
    policy::compute_policy_hash,
    release1_state::{
        EmergencyFreezeResolutionKindV1, EMERGENCY_FREEZE_RESOLUTION_V1_DISCRIMINATOR,
        EMERGENCY_FREEZE_RESOLUTION_V1_RESERVED_LEN,
        PROGRAMDATA_FAILURE_OBSERVATION_V1_DISCRIMINATOR,
        PROGRAMDATA_FAILURE_OBSERVATION_V1_RESERVED_LEN, PROGRAMDATA_VERIFICATION_V1_DISCRIMINATOR,
        PROGRAMDATA_VERIFICATION_V1_RESERVED_LEN, VERIFICATION_BITMAP_BYTES_V1,
    },
    state::{
        OptionalPubkeyV1, CONTROLLER_CONFIG_DISCRIMINATOR, CONTROLLER_CONFIG_RESERVED_LEN,
        GOVERNANCE_POLICY_DISCRIMINATOR, GOVERNANCE_POLICY_RESERVED_LEN,
        PROTOCOL_GATE_DISCRIMINATOR, PROTOCOL_GATE_RESERVED_LEN,
    },
};

fn key(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}

fn dummy_candidate(phase: StateCheckpointPhaseV1) -> CheckpointCandidateV1 {
    CheckpointCandidateV1 {
        phase,
        expected_subject_state: match phase {
            StateCheckpointPhaseV1::Prestate => CheckpointSubjectStateV1::ProposalFrozen,
            StateCheckpointPhaseV1::Poststate => {
                CheckpointSubjectStateV1::ProposalProgramDataVerified
            }
            StateCheckpointPhaseV1::Emergency => {
                CheckpointSubjectStateV1::EmergencyResolutionTimelocked
            }
        },
        expected_subject_digest: [1; 32],
        expected_gate_status: if phase == StateCheckpointPhaseV1::Emergency {
            GateStatusV1::EmergencyFrozen
        } else {
            GateStatusV1::FrozenForUpgrade
        },
        expected_gate_epoch: 7,
        finalized_observation_slot: 20,
        target_programdata_slot: 9,
        target_payload_commitment: [2; 32],
        target_raw_programdata_commitment: [3; 32],
        target_capacity: 64,
        program_owned_state_root: [4; 32],
        program_owned_state_count: 2,
        logical_compressed_state_root: [5; 32],
        logical_compressed_state_count: 3,
        semantic_custody_accounting_root: [6; 32],
        hard_combined_root: [7; 32],
        external_metadata_observation_root: [8; 32],
        external_raw_balance_observation_root: [9; 32],
        schema_identifier: [10; 32],
        admitted_positive_donation_root: [0; 32],
        admitted_positive_donation_count: 0,
        forbidden_drift_count: 0,
        expected_checkpoint_digest: [11; 32],
    }
}

#[derive(Clone)]
struct RollbackEvidenceFixture {
    program_id: Pubkey,
    config_key: Pubkey,
    config: ControllerConfigV1,
    evidence_key: Pubkey,
    evidence: ProgramDataVerificationV1,
    commitment: RollbackPrestateCommitment,
    candidate: CheckpointCandidateV1,
}

impl RollbackEvidenceFixture {
    fn validate(&self) -> Result<u64, ProgramError> {
        let mut config_lamports = 1;
        let mut config_data = [];
        let config_info = AccountInfo::new(
            &self.config_key,
            false,
            false,
            &mut config_lamports,
            &mut config_data,
            &self.program_id,
            false,
            0,
        );
        let mut evidence_lamports = 1;
        let mut evidence_data = self.evidence.try_to_vec().unwrap();
        let evidence_info = AccountInfo::new(
            &self.evidence_key,
            false,
            false,
            &mut evidence_lamports,
            &mut evidence_data,
            &self.program_id,
            false,
            0,
        );
        validate_rollback_prestate_evidence_commitment(
            &self.program_id,
            &evidence_info,
            &config_info,
            &self.config,
            &self.commitment,
            &self.candidate,
        )
    }
}

fn sample_rollback_evidence_fixture() -> RollbackEvidenceFixture {
    let program_id = key(120);
    let (config_key, config) = sample_config(&program_id, key(121));
    let primary_proposal = key(122);
    let (evidence_key, bump) = derive_programdata_check_pda(&program_id, &primary_proposal);
    let mut payload_bitmap = [0; VERIFICATION_BITMAP_BYTES_V1];
    payload_bitmap[0] = 1;
    let expected_candidate_full_payload_sha256 = [2; 32];
    let artifact_sha256 = [16; 32];
    let artifact_chunk_merkle_root = [12; 32];
    let artifact_length = 64;
    let capacity = 128;
    let mut tail_bitmap = [0; VERIFICATION_BITMAP_BYTES_V1];
    tail_bitmap[0] = 1;
    let evidence = ProgramDataVerificationV1 {
        discriminator: PROGRAMDATA_VERIFICATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        status: ProgramDataVerificationStatusV1::Verified,
        controller_config: config_key,
        proposal: primary_proposal,
        target_program: config.target_program,
        target_programdata: config.target_programdata,
        upgradeable_loader: config.upgradeable_loader,
        controller_authority: config.authority_pda,
        artifact_length,
        artifact_sha256,
        artifact_chunk_merkle_root,
        chunk_hash_domain: ARTIFACT_MERKLE_SCHEME_ID,
        chunk_size: RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        payload_chunk_count: 1,
        verified_payload_chunk_bitmap: payload_bitmap,
        verified_payload_chunk_count: 1,
        deployed_slot: 9,
        capacity,
        tail_length: capacity - artifact_length,
        tail_chunk_count: 1,
        verified_tail_chunk_bitmap: tail_bitmap,
        verified_tail_chunk_count: 1,
        raw_programdata_hash: [3; 32],
        zero_tail_verified: true,
        finalized_slot: 10,
        reserved: [0; PROGRAMDATA_VERIFICATION_V1_RESERVED_LEN],
    };
    let commitment = RollbackPrestateCommitment {
        primary_proposal,
        expected_candidate_full_payload_sha256,
        artifact_chunk_merkle_root,
        chunk_hash_domain: ARTIFACT_MERKLE_SCHEME_ID,
        capacity,
    };
    let mut candidate = dummy_candidate(StateCheckpointPhaseV1::Prestate);
    candidate.target_capacity = capacity;
    RollbackEvidenceFixture {
        program_id,
        config_key,
        config,
        evidence_key,
        evidence,
        commitment,
        candidate,
    }
}

#[derive(Clone)]
struct RollbackFailureEvidenceFixture {
    program_id: Pubkey,
    config_key: Pubkey,
    config: ControllerConfigV1,
    evidence_key: Pubkey,
    evidence: ProgramDataFailureObservationV1,
    commitment: RollbackPrestateCommitment,
    candidate: CheckpointCandidateV1,
}

impl RollbackFailureEvidenceFixture {
    fn refresh_digest(&mut self) {
        self.evidence.observation_digest =
            crate::release1_digest::compute_programdata_failure_observation_digest_v1(
                &self.evidence,
            )
            .unwrap();
    }

    fn validate(&self) -> Result<u64, ProgramError> {
        let mut config_lamports = 1;
        let mut config_data = [];
        let config_info = AccountInfo::new(
            &self.config_key,
            false,
            false,
            &mut config_lamports,
            &mut config_data,
            &self.program_id,
            false,
            0,
        );
        let mut evidence_lamports = 1;
        let mut evidence_data = self.evidence.try_to_vec().unwrap();
        let evidence_info = AccountInfo::new(
            &self.evidence_key,
            false,
            false,
            &mut evidence_lamports,
            &mut evidence_data,
            &self.program_id,
            false,
            0,
        );
        validate_rollback_prestate_evidence_commitment(
            &self.program_id,
            &evidence_info,
            &config_info,
            &self.config,
            &self.commitment,
            &self.candidate,
        )
    }
}

fn sample_rollback_failure_evidence_fixture() -> RollbackFailureEvidenceFixture {
    let program_id = key(130);
    let (config_key, config) = sample_config(&program_id, key(131));
    let primary_proposal = key(132);
    let frozen_epoch = 6;
    let (evidence_key, bump) =
        derive_programdata_failure_observation_pda(&program_id, &primary_proposal, frozen_epoch);
    let capacity = 128;
    let mut evidence = ProgramDataFailureObservationV1 {
        discriminator: PROGRAMDATA_FAILURE_OBSERVATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        finalized: true,
        controller_config: config_key,
        protocol_gate: config.gate_pda,
        primary_proposal,
        target_program: config.target_program,
        target_programdata: config.target_programdata,
        frozen_epoch,
        actual_program_owner: config.upgradeable_loader,
        actual_program_executable: true,
        actual_program_data_length: LOADER_V3_PROGRAM_ACCOUNT_LEN_V1,
        program_header_present: true,
        actual_linked_programdata: OptionalPubkeyV1::some(config.target_programdata).unwrap(),
        raw_hash_complete: true,
        actual_raw_programdata_sha256: [3; 32],
        actual_owner: config.upgradeable_loader,
        actual_executable: false,
        actual_data_length: capacity + LOADER_V3_PROGRAMDATA_METADATA_LEN_V1,
        programdata_header_present: true,
        actual_programdata_slot: 9,
        actual_capacity: capacity,
        actual_authority: OptionalPubkeyV1::some(config.authority_pda).unwrap(),
        mismatch_class: ProgramDataMismatchClassV1::PayloadLeaf,
        failing_chunk_index: 0,
        expected_leaf_hash: [40; 32],
        actual_leaf_hash: [41; 32],
        finalized_slot: 10,
        observation_digest: [0; 32],
        reserved: [0; PROGRAMDATA_FAILURE_OBSERVATION_V1_RESERVED_LEN],
    };
    evidence.observation_digest =
        crate::release1_digest::compute_programdata_failure_observation_digest_v1(&evidence)
            .unwrap();
    let commitment = RollbackPrestateCommitment {
        primary_proposal,
        expected_candidate_full_payload_sha256: [2; 32],
        artifact_chunk_merkle_root: [12; 32],
        chunk_hash_domain: ARTIFACT_MERKLE_SCHEME_ID,
        capacity,
    };
    let mut candidate = dummy_candidate(StateCheckpointPhaseV1::Prestate);
    candidate.target_capacity = capacity;
    RollbackFailureEvidenceFixture {
        program_id,
        config_key,
        config,
        evidence_key,
        evidence,
        commitment,
        candidate,
    }
}

#[derive(Clone)]
struct RollbackBaselineFixture {
    program_id: Pubkey,
    config_key: Pubkey,
    config: ControllerConfigV1,
    baseline_key: Pubkey,
    baseline: StateCheckpointV1,
    rollback_key: Pubkey,
    rollback: UpgradeProposalV2,
    candidate: CheckpointCandidateV1,
}

impl RollbackBaselineFixture {
    fn validate(&self) -> Result<u64, ProgramError> {
        let mut config_lamports = 1;
        let mut config_data = [];
        let config_info = AccountInfo::new(
            &self.config_key,
            false,
            false,
            &mut config_lamports,
            &mut config_data,
            &self.program_id,
            false,
            0,
        );
        let mut baseline_lamports = 1;
        let mut baseline_data = self.baseline.try_to_vec().unwrap();
        let baseline_info = AccountInfo::new(
            &self.baseline_key,
            false,
            false,
            &mut baseline_lamports,
            &mut baseline_data,
            &self.program_id,
            false,
            0,
        );
        let binding = CheckpointBinding {
            subject: CheckpointSubject::Proposal(Box::new(self.rollback.clone())),
            subject_key: self.rollback_key,
            subject_digest: self.rollback.proposal_digest,
            checkpoint: self.rollback.prestate_checkpoint,
            checkpoint_bump: 1,
        };
        validate_rollback_prestate_baseline(
            &self.program_id,
            &baseline_info,
            &config_info,
            &self.config,
            &binding,
            &self.candidate,
        )
    }
}

fn sample_rollback_baseline_fixture() -> RollbackBaselineFixture {
    let program_id = key(140);
    let (config_key, config) = sample_config(&program_id, key(141));
    let primary_proposal = key(142);
    let (baseline_key, baseline_bump) =
        derive_checkpoint_pda(&program_id, &primary_proposal, CheckpointPhaseV1::Prestate);
    let mut baseline = baseline_checkpoint();
    baseline.bump = baseline_bump;
    baseline.controller_config = config_key;
    baseline.proposal = primary_proposal;
    baseline.target_program = config.target_program;
    baseline.target_programdata = config.target_programdata;
    baseline.gate_epoch = 6;
    baseline.hard_combined_root =
        compute_state_checkpoint_hard_combined_root_v1(&baseline).unwrap();
    baseline.checkpoint_digest = compute_state_checkpoint_digest_v1(&baseline).unwrap();

    let rollback_key = key(143);
    let rollback = UpgradeProposalV2 {
        proposal_class: ProposalClassV1::EmergencyRollback,
        primary_proposal: OptionalPubkeyV1::some(primary_proposal).unwrap(),
        checkpoint_schema_id: baseline.schema_identifier,
        proposal_digest: [44; 32],
        prestate_checkpoint: derive_checkpoint_pda(
            &program_id,
            &rollback_key,
            CheckpointPhaseV1::Prestate,
        )
        .0,
        ..UpgradeProposalV2::default()
    };

    let mut candidate = candidate_from_baseline(&baseline);
    candidate.phase = StateCheckpointPhaseV1::Prestate;
    candidate.expected_subject_state = CheckpointSubjectStateV1::ProposalFrozen;
    candidate.expected_gate_epoch = 7;
    candidate.finalized_observation_slot = baseline.finalized_slot;
    RollbackBaselineFixture {
        program_id,
        config_key,
        config,
        baseline_key,
        baseline,
        rollback_key,
        rollback,
        candidate,
    }
}

fn rotation_expectation() -> CouncilRotationExpectationV1 {
    CouncilRotationExpectationV1 {
        expected_rotation_digest: [1; 32],
        expected_current_council_version: 1,
        expected_current_council_hash: [2; 32],
        expected_candidate_council_version: 2,
        expected_candidate_council_hash: [3; 32],
        expected_gate_status: GateStatusV1::Active,
        expected_gate_epoch: 5,
        expected_target_nonce: 7,
        expected_state: CouncilRotationStateV1::Draft,
        expected_not_before_slot: 30,
        expected_expiry_slot: 100,
    }
}

fn emergency_expectation() -> EmergencyResolutionExpectationV1 {
    EmergencyResolutionExpectationV1 {
        expected_resolution_digest: [1; 32],
        expected_policy_version: 1,
        expected_policy_hash: [2; 32],
        expected_council_version: 1,
        expected_council_hash: [3; 32],
        expected_gate_status: GateStatusV1::EmergencyFrozen,
        expected_gate_epoch: 5,
        expected_freeze_slot: 10,
        expected_freeze_reason_code: 2,
        expected_target_nonce: 7,
        expected_state: EmergencyFreezeResolutionStateV1::Draft,
        expected_not_before_slot: 30,
        expected_expiry_slot: 100,
    }
}

fn invalid_account_count() -> ProgramError {
    GovernanceError::InvalidAccountCount.into()
}

#[test]
fn every_export_rejects_the_wrong_account_count_before_clock_or_data_access() {
    let program_id = key(99);
    let candidate = dummy_candidate(StateCheckpointPhaseV1::Prestate);
    let rotation = rotation_expectation();
    let emergency = emergency_expectation();
    let cases = [
        process_create_checkpoint_attestation_v1(
            &program_id,
            &[],
            CreateCheckpointAttestationV1 {
                candidate,
                expected_council_version: 1,
                expected_council_hash: [1; 32],
                seat_index: 0,
            },
        ),
        process_recast_checkpoint_attestation_v1(
            &program_id,
            &[],
            RecastCheckpointAttestationV1 {
                candidate,
                expected_council_version: 1,
                expected_council_hash: [1; 32],
                seat_index: 0,
                expected_previous_attestation_digest: [2; 32],
            },
        ),
        process_finalize_checkpoint_v1(
            &program_id,
            &[],
            FinalizeCheckpointV1 {
                candidate,
                expected_council_version: 1,
                expected_council_hash: [1; 32],
            },
        ),
        process_create_candidate_council_set_v1(
            &program_id,
            &[],
            CreateCandidateCouncilSetV1 {
                expected_current_council_version: 1,
                expected_current_council_hash: [1; 32],
                candidate_council_version: 2,
                activation_slot: 30,
                expected_target_nonce: 7,
                expected_gate_status: GateStatusV1::Active,
                expected_gate_epoch: 5,
                expected_candidate_council_hash: [2; 32],
                seat_terms: [CouncilSeatTermV1::default(); 5],
            },
        ),
        process_create_council_rotation_v1(
            &program_id,
            &[],
            CreateCouncilRotationV1 {
                creation_slot: 10,
                not_before_slot: 30,
                expiry_slot: 100,
                expected_current_council_version: 1,
                expected_current_council_hash: [1; 32],
                expected_candidate_council_version: 2,
                expected_candidate_council_hash: [2; 32],
                expected_gate_status: GateStatusV1::Active,
                expected_gate_epoch: 5,
                expected_target_nonce: 7,
                expected_rotation_digest: [3; 32],
            },
        ),
        process_approve_council_rotation_v1(
            &program_id,
            &[],
            ApproveCouncilRotationV1 {
                expected: rotation,
                expected_approval_bitset: 0,
                expected_approval_count: 0,
            },
        ),
        process_activate_council_rotation_v1(
            &program_id,
            &[],
            ActivateCouncilRotationV1 {
                expected: rotation,
                expected_approval_bitset: 7,
                expected_approval_count: 3,
            },
        ),
        process_queue_council_rotation_v1(
            &program_id,
            &[],
            QueueCouncilRotationV1 {
                expected: rotation,
                expected_approval_bitset: 7,
                expected_approval_count: 3,
            },
        ),
        process_expire_emergency_resolution_v1(
            &program_id,
            &[],
            ExpireEmergencyResolutionV1 {
                expected: emergency,
            },
        ),
        process_cancel_council_rotation_v1(
            &program_id,
            &[],
            CancelCouncilRotationV1 {
                expected: rotation,
                expected_cancellation_approval_bitset: 0,
                expected_cancellation_approval_count: 0,
                cancellation_reason_code: 9,
            },
        ),
        process_expire_council_rotation_v1(
            &program_id,
            &[],
            ExpireCouncilRotationV1 { expected: rotation },
        ),
    ];
    for result in cases {
        assert_eq!(result, Err(invalid_account_count()));
    }
}

#[test]
fn duplicate_account_aliases_fail_before_privilege_or_clock_checks() {
    let program_id = key(90);
    let account_key = key(91);
    let owner = key(92);
    let mut lamports = 1;
    let mut data = [];
    let account = AccountInfo::new(
        &account_key,
        false,
        false,
        &mut lamports,
        &mut data,
        &owner,
        false,
        0,
    );
    let aliases = vec![account; 13];
    let instruction = CreateCandidateCouncilSetV1 {
        expected_current_council_version: 1,
        expected_current_council_hash: [1; 32],
        candidate_council_version: 2,
        activation_slot: 30,
        expected_target_nonce: 7,
        expected_gate_status: GateStatusV1::Active,
        expected_gate_epoch: 5,
        expected_candidate_council_hash: [2; 32],
        seat_terms: [CouncilSeatTermV1::default(); 5],
    };
    assert_eq!(
        process_create_candidate_council_set_v1(&program_id, &aliases, instruction),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
}

fn sample_config(program_id: &Pubkey, target_program: Pubkey) -> (Pubkey, ControllerConfigV1) {
    let (config_key, bump) = derive_controller_config_pda(program_id, &target_program);
    let target_programdata = derive_upgradeable_programdata_address(&target_program).0;
    let authority_pda = derive_authority_pda(program_id, &target_program).0;
    let gate_pda = derive_gate_pda(program_id, &target_program).0;
    (
        config_key,
        ControllerConfigV1 {
            discriminator: CONTROLLER_CONFIG_DISCRIMINATOR,
            version: ACCOUNT_VERSION_V1,
            bump,
            initialized: true,
            cluster_domain: [1; 32],
            target_program,
            target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            authority_pda,
            gate_pda,
            canonical_spill_treasury: key(10),
            current_council_version: 1,
            current_policy_version: 1,
            next_proposal_id: 2,
            target_nonce: 7,
            guardian: key(11),
            vote_program: Pubkey::default(),
            vote_programdata: Pubkey::default(),
            vote_config: Pubkey::default(),
            vote_mint: Pubkey::default(),
            token_governance_enabled: false,
            routine_delay_slots: 10,
            major_delay_slots: 20,
            rollback_delay_slots: 5,
            terminal_delay_slots: 30,
            vote_review_slots: 7,
            proposal_expiry_slots: 100,
            policy_flags: 0,
            reserved: [0; CONTROLLER_CONFIG_RESERVED_LEN],
        },
    )
}

fn sample_policy(
    program_id: &Pubkey,
    config_key: Pubkey,
    config: &ControllerConfigV1,
) -> (Pubkey, GovernancePolicyV1) {
    let (policy_key, bump) = derive_policy_pda(
        program_id,
        &config.target_program,
        config.current_policy_version,
    );
    let mut policy = GovernancePolicyV1 {
        discriminator: GOVERNANCE_POLICY_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        controller_config: config_key,
        version: config.current_policy_version,
        target_program: config.target_program,
        activation_slot: 1,
        council_size: 5,
        routine_threshold: 3,
        terminal_threshold: 4,
        governance_mode: crate::state::GovernanceModeV1::BootstrapCouncilOnly,
        policy_flags: 0,
        veto_quorum_bps: 0,
        affirmative_quorum_bps: 0,
        affirmative_approval_bps: 0,
        routine_requires_vote: false,
        economic_requires_vote: false,
        constitutional_requires_vote: false,
        rotation_requires_vote: false,
        immutability_requires_vote: false,
        policy_hash: [0; 32],
        reserved: [0; GOVERNANCE_POLICY_RESERVED_LEN],
    };
    policy.policy_hash = compute_policy_hash(&policy);
    (policy_key, policy)
}

#[test]
fn config_loader_rejects_owner_size_discriminator_version_and_bump_drift() {
    let program_id = key(50);
    let target = key(51);
    let (config_key, config) = sample_config(&program_id, target);
    let mut lamports = 1;
    let mut bytes = config.try_to_vec().unwrap();
    let account = AccountInfo::new(
        &config_key,
        false,
        false,
        &mut lamports,
        &mut bytes,
        &program_id,
        false,
        0,
    );
    assert_eq!(*load_config(&program_id, &account).unwrap(), config);

    let wrong_owner = key(52);
    let mut lamports = 1;
    let mut bytes = config.try_to_vec().unwrap();
    let account = AccountInfo::new(
        &config_key,
        false,
        false,
        &mut lamports,
        &mut bytes,
        &wrong_owner,
        false,
        0,
    );
    assert_eq!(
        load_config(&program_id, &account),
        Err(GovernanceError::IncorrectAccountOwner.into())
    );

    let mut lamports = 1;
    let mut bytes = config.try_to_vec().unwrap();
    bytes.pop();
    let account = AccountInfo::new(
        &config_key,
        false,
        false,
        &mut lamports,
        &mut bytes,
        &program_id,
        false,
        0,
    );
    assert_eq!(
        load_config(&program_id, &account),
        Err(GovernanceError::InvalidAccountSize.into())
    );

    let mut wrong_discriminator = config.clone();
    wrong_discriminator.discriminator = [0; 8];
    let mut lamports = 1;
    let mut bytes = wrong_discriminator.try_to_vec().unwrap();
    let account = AccountInfo::new(
        &config_key,
        false,
        false,
        &mut lamports,
        &mut bytes,
        &program_id,
        false,
        0,
    );
    assert_eq!(
        load_config(&program_id, &account),
        Err(GovernanceError::InvalidDiscriminator.into())
    );

    let mut wrong_version = config.clone();
    wrong_version.version = ACCOUNT_VERSION_V1 + 1;
    let mut lamports = 1;
    let mut bytes = wrong_version.try_to_vec().unwrap();
    let account = AccountInfo::new(
        &config_key,
        false,
        false,
        &mut lamports,
        &mut bytes,
        &program_id,
        false,
        0,
    );
    assert_eq!(
        load_config(&program_id, &account),
        Err(GovernanceError::UnsupportedVersion.into())
    );

    let mut wrong_bump = config;
    wrong_bump.bump ^= 1;
    let mut lamports = 1;
    let mut bytes = wrong_bump.try_to_vec().unwrap();
    let account = AccountInfo::new(
        &config_key,
        false,
        false,
        &mut lamports,
        &mut bytes,
        &program_id,
        false,
        0,
    );
    assert_eq!(
        load_config(&program_id, &account),
        Err(GovernanceError::InvalidPda.into())
    );
}

#[test]
fn policy_loader_and_emergency_expiry_guard_reject_missing_wrong_and_stale_policy() {
    let program_id = key(53);
    let (config_key, config) = sample_config(&program_id, key(54));
    let (policy_key, policy) = sample_policy(&program_id, config_key, &config);
    let mut policy_lamports = 1;
    let mut policy_bytes = policy.try_to_vec().unwrap();
    let policy_info = AccountInfo::new(
        &policy_key,
        false,
        false,
        &mut policy_lamports,
        &mut policy_bytes,
        &program_id,
        false,
        0,
    );
    assert_eq!(
        *load_policy(&program_id, &policy_info, &config_key, &config, 50).unwrap(),
        policy
    );

    let mut wrong_policy = policy.clone();
    wrong_policy.policy_hash = [99; 32];
    let mut wrong_lamports = 1;
    let mut wrong_bytes = wrong_policy.try_to_vec().unwrap();
    let wrong_info = AccountInfo::new(
        &policy_key,
        false,
        false,
        &mut wrong_lamports,
        &mut wrong_bytes,
        &program_id,
        false,
        0,
    );
    assert_eq!(
        load_policy(&program_id, &wrong_info, &config_key, &config, 50),
        Err(GovernanceError::PolicyHashMismatch.into())
    );

    let gate = ProtocolGateV1 {
        discriminator: PROTOCOL_GATE_DISCRIMINATOR,
        version: ACCOUNT_VERSION_V1,
        bump: 1,
        initialized: true,
        status: GateStatusV1::EmergencyFrozen,
        controller_config: config_key,
        target_program: config.target_program,
        target_programdata: config.target_programdata,
        epoch: 5,
        active_proposal: Pubkey::default(),
        freeze_slot: 10,
        freeze_reason_code: 2,
        last_completed_proposal: Pubkey::default(),
        reserved: [0; PROTOCOL_GATE_RESERVED_LEN],
    };
    let resolution = EmergencyFreezeResolutionV1 {
        discriminator: EMERGENCY_FREEZE_RESOLUTION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: 1,
        initialized: true,
        state: EmergencyFreezeResolutionStateV1::Draft,
        controller_config: config_key,
        protocol_gate: config.gate_pda,
        target_program: config.target_program,
        target_programdata: config.target_programdata,
        emergency_freeze_observation: key(60),
        frozen_epoch: 5,
        freeze_slot: 10,
        freeze_reason_code: 2,
        resolution_kind: EmergencyFreezeResolutionKindV1::ResumeWithoutUpgrade,
        creation_slot: 11,
        not_before_slot: 30,
        expiry_slot: 100,
        target_nonce: 7,
        observed_program_owner: UPGRADEABLE_LOADER_ID,
        observed_program_executable: true,
        observed_program_data_length: 36,
        observed_program_header_present: true,
        observed_linked_programdata: crate::state::OptionalPubkeyV1::some(
            config.target_programdata,
        )
        .unwrap(),
        observed_programdata_owner: UPGRADEABLE_LOADER_ID,
        observed_programdata_executable: false,
        observed_programdata_data_length: 109,
        observed_programdata_header_present: true,
        observed_programdata_slot: 9,
        observed_raw_hash_complete: true,
        observed_raw_programdata_hash: [4; 32],
        observed_capacity: 64,
        observed_authority: crate::state::OptionalPubkeyV1::some(config.authority_pda).unwrap(),
        emergency_checkpoint: key(61),
        approval_council_version: 1,
        approval_council_hash: [3; 32],
        approval_bitset: 0,
        approval_count: 0,
        resolution_digest: [1; 32],
        executed_slot: 0,
        cancellation_reason_code: 0,
        terminal_reason_code: 0,
        reserved: [0; EMERGENCY_FREEZE_RESOLUTION_V1_RESERVED_LEN],
    };
    let mut expected = emergency_expectation();
    expected.expected_policy_hash = policy.policy_hash;
    assert_eq!(
        validate_emergency_expiry_expectation(&expected, &config, &policy, &gate, &resolution,),
        Ok(())
    );
    expected.expected_policy_hash = [0; 32];
    assert_eq!(
        validate_emergency_expiry_expectation(&expected, &config, &policy, &gate, &resolution,),
        Err(GovernanceError::CrossAccountMismatch)
    );

    let resolution_before = vec![0x3c; EmergencyFreezeResolutionV1::LEN];
    let missing_policy_accounts = vec![
        leaked_account(80, false, false, false, vec![]),
        leaked_account(81, false, false, false, vec![]),
        leaked_account(82, false, true, false, resolution_before.clone()),
    ];
    assert_eq!(
        process_expire_emergency_resolution_v1(
            &program_id,
            &missing_policy_accounts,
            ExpireEmergencyResolutionV1 { expected },
        ),
        Err(invalid_account_count())
    );
    assert_eq!(
        &**missing_policy_accounts[2].try_borrow_data().unwrap(),
        resolution_before.as_slice()
    );
    expected.expected_policy_hash = policy.policy_hash;
    expected.expected_policy_version += 1;
    assert_eq!(
        validate_emergency_expiry_expectation(&expected, &config, &policy, &gate, &resolution,),
        Err(GovernanceError::CrossAccountMismatch)
    );
}

fn leaked_account(
    byte: u8,
    signer: bool,
    writable: bool,
    executable: bool,
    data: Vec<u8>,
) -> AccountInfo<'static> {
    let account_key = Box::leak(Box::new(key(byte)));
    let owner = Box::leak(Box::new(key(byte.wrapping_add(100))));
    let lamports = Box::leak(Box::new(1u64));
    let data = Box::leak(data.into_boxed_slice());
    AccountInfo::new(
        account_key,
        signer,
        writable,
        lamports,
        data,
        owner,
        executable,
        0,
    )
}

fn account_data_snapshot(accounts: &[AccountInfo<'_>]) -> Vec<Vec<u8>> {
    accounts
        .iter()
        .map(|account| account.try_borrow_data().unwrap().to_vec())
        .collect()
}

#[test]
fn candidate_creation_allows_only_the_creator_candidate_role_alias() {
    let intentional_creator_alias = vec![
        leaked_account(1, true, true, false, vec![]),
        leaked_account(2, true, false, false, vec![]),
        leaked_account(3, false, false, false, vec![]),
        leaked_account(4, false, false, false, vec![]),
        leaked_account(5, false, false, false, vec![]),
        leaked_account(6, false, false, false, vec![]),
        leaked_account(7, false, true, false, vec![]),
        leaked_account(2, true, false, false, vec![]),
        leaked_account(8, false, false, false, vec![]),
        leaked_account(9, false, false, false, vec![]),
        leaked_account(10, false, false, false, vec![]),
        leaked_account(11, false, false, false, vec![]),
        leaked_account(12, false, false, true, vec![]),
    ];
    let before = account_data_snapshot(&intentional_creator_alias);
    assert_eq!(
        validate_candidate_creation_authority_contract(
            &intentional_creator_alias[0],
            &intentional_creator_alias[1],
            &intentional_creator_alias[2],
            &intentional_creator_alias[3],
            &intentional_creator_alias[4],
            &intentional_creator_alias[5],
            &intentional_creator_alias[6],
            &intentional_creator_alias[7..12],
            &intentional_creator_alias[12],
        ),
        Ok(())
    );
    assert_eq!(account_data_snapshot(&intentional_creator_alias), before);

    let mut illegal_fixed_role_alias = intentional_creator_alias;
    illegal_fixed_role_alias[8] = illegal_fixed_role_alias[2].clone();
    let before = account_data_snapshot(&illegal_fixed_role_alias);
    assert_eq!(
        validate_candidate_creation_authority_contract(
            &illegal_fixed_role_alias[0],
            &illegal_fixed_role_alias[1],
            &illegal_fixed_role_alias[2],
            &illegal_fixed_role_alias[3],
            &illegal_fixed_role_alias[4],
            &illegal_fixed_role_alias[5],
            &illegal_fixed_role_alias[6],
            &illegal_fixed_role_alias[7..12],
            &illegal_fixed_role_alias[12],
        ),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
    assert_eq!(account_data_snapshot(&illegal_fixed_role_alias), before);
}

#[test]
fn unit_privilege_failures_leave_checkpoint_and_rotation_bytes_identical() {
    let program_id = key(70);
    let rotation_before = vec![0x5a; CouncilRotationProposalV1::LEN];
    let rotation_accounts = vec![
        leaked_account(1, false, true, false, vec![]), // config must be read-only
        leaked_account(2, false, false, false, vec![]),
        leaked_account(3, false, false, false, vec![]),
        leaked_account(4, false, false, false, vec![]),
        leaked_account(5, false, false, false, vec![]),
        leaked_account(6, false, true, false, rotation_before.clone()),
    ];
    let result = process_queue_council_rotation_v1(
        &program_id,
        &rotation_accounts,
        QueueCouncilRotationV1 {
            expected: rotation_expectation(),
            expected_approval_bitset: 7,
            expected_approval_count: 3,
        },
    );
    assert_eq!(
        result,
        Err(GovernanceError::InvalidAccountPrivileges.into())
    );
    assert_eq!(
        &**rotation_accounts[5].try_borrow_data().unwrap(),
        rotation_before.as_slice()
    );

    let attestation_before = vec![0xa5; CheckpointAttestationV1::LEN];
    let checkpoint_accounts = vec![
        leaked_account(10, false, true, false, vec![]), // payer must sign
        leaked_account(11, false, false, false, vec![]),
        leaked_account(12, false, false, false, vec![]),
        leaked_account(13, false, false, false, vec![]),
        leaked_account(14, false, false, false, vec![]),
        leaked_account(15, false, false, false, vec![]),
        leaked_account(16, false, false, false, vec![]),
        leaked_account(17, false, true, false, attestation_before.clone()),
        leaked_account(18, true, false, false, vec![]),
        leaked_account(19, false, false, true, vec![]),
    ];
    let result = process_create_checkpoint_attestation_v1(
        &program_id,
        &checkpoint_accounts,
        CreateCheckpointAttestationV1 {
            candidate: dummy_candidate(StateCheckpointPhaseV1::Prestate),
            expected_council_version: 1,
            expected_council_hash: [1; 32],
            seat_index: 0,
        },
    );
    assert_eq!(
        result,
        Err(GovernanceError::InvalidAccountPrivileges.into())
    );
    assert_eq!(
        &**checkpoint_accounts[7].try_borrow_data().unwrap(),
        attestation_before.as_slice()
    );
}

fn baseline_checkpoint() -> StateCheckpointV1 {
    StateCheckpointV1 {
        discriminator: STATE_CHECKPOINT_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: 1,
        initialized: true,
        phase: StateCheckpointPhaseV1::Prestate,
        controller_config: key(1),
        proposal: key(2),
        emergency_resolution: Pubkey::default(),
        subject_digest: [1; 32],
        target_program: key(3),
        target_programdata: key(4),
        finalized_observation_slot: 10,
        gate_epoch: 7,
        target_programdata_slot: 9,
        target_payload_commitment: [2; 32],
        target_raw_programdata_commitment: [3; 32],
        target_capacity: 64,
        program_owned_state_root: [4; 32],
        program_owned_state_count: 2,
        logical_compressed_state_root: [5; 32],
        logical_compressed_state_count: 3,
        semantic_custody_accounting_root: [6; 32],
        hard_combined_root: [7; 32],
        external_metadata_observation_root: [8; 32],
        external_raw_balance_observation_root: [9; 32],
        schema_identifier: [10; 32],
        admitted_positive_donation_root: [0; 32],
        admitted_positive_donation_count: 0,
        forbidden_drift_count: 0,
        approval_council_version: 1,
        approval_council_hash: [11; 32],
        checkpoint_digest: [12; 32],
        approval_bitset: 7,
        approval_count: 3,
        accepted: true,
        finalized_slot: 20,
        reserved: [0; STATE_CHECKPOINT_V1_RESERVED_LEN],
    }
}

fn candidate_from_baseline(baseline: &StateCheckpointV1) -> CheckpointCandidateV1 {
    let mut candidate = dummy_candidate(StateCheckpointPhaseV1::Poststate);
    candidate.program_owned_state_root = baseline.program_owned_state_root;
    candidate.program_owned_state_count = baseline.program_owned_state_count;
    candidate.logical_compressed_state_root = baseline.logical_compressed_state_root;
    candidate.logical_compressed_state_count = baseline.logical_compressed_state_count;
    candidate.semantic_custody_accounting_root = baseline.semantic_custody_accounting_root;
    candidate.hard_combined_root = baseline.hard_combined_root;
    candidate.external_metadata_observation_root = baseline.external_metadata_observation_root;
    candidate.external_raw_balance_observation_root =
        baseline.external_raw_balance_observation_root;
    candidate.schema_identifier = baseline.schema_identifier;
    candidate
}

#[test]
fn poststate_hard_roots_block_while_only_explicit_positive_donation_drift_is_admitted() {
    let baseline = baseline_checkpoint();
    let mut candidate = candidate_from_baseline(&baseline);
    assert_eq!(validate_poststate_outcome(&baseline, &candidate), Ok(()));

    candidate.external_raw_balance_observation_root = [90; 32];
    assert_eq!(
        validate_poststate_hard_invariants(&baseline, &candidate),
        Err(GovernanceError::InvalidRelease1Account)
    );
    candidate.admitted_positive_donation_root = [91; 32];
    candidate.admitted_positive_donation_count = 1;
    assert_eq!(validate_poststate_outcome(&baseline, &candidate), Ok(()));

    candidate.program_owned_state_root = [92; 32];
    assert_eq!(
        validate_poststate_outcome(&baseline, &candidate),
        Err(GovernanceError::InvalidRelease1Account)
    );
    candidate.forbidden_drift_count = 1;
    assert_eq!(
        validate_poststate_hard_invariants(&baseline, &candidate),
        Err(GovernanceError::InvalidRelease1Account)
    );
}

#[test]
fn rejected_poststate_requires_forbidden_drift_and_a_concrete_prestate_mismatch() {
    let baseline = baseline_checkpoint();
    let mut candidate = candidate_from_baseline(&baseline);

    // An explicit non-donation external-balance failure is immutable
    // rejected evidence usable by the rollback path.
    candidate.forbidden_drift_count = 1;
    candidate.external_raw_balance_observation_root = [93; 32];
    assert_eq!(validate_checkpoint_acceptance_shape(&candidate), Ok(()));
    assert_eq!(validate_poststate_outcome(&baseline, &candidate), Ok(()));

    // A forbidden count alone cannot fabricate a rejected checkpoint when
    // every observed protected-state field still matches Prestate.
    candidate.external_raw_balance_observation_root =
        baseline.external_raw_balance_observation_root;
    assert_eq!(
        validate_poststate_outcome(&baseline, &candidate),
        Err(GovernanceError::InvalidRelease1Account)
    );

    // Identity drift is also a concrete rejected outcome when attested as
    // forbidden; it is never normalized as a donation.
    candidate.external_metadata_observation_root = [95; 32];
    candidate.hard_combined_root = [96; 32];
    assert_eq!(validate_poststate_outcome(&baseline, &candidate), Ok(()));

    // Without forbidden evidence the same identity drift cannot enter the
    // accepted lane.
    candidate.forbidden_drift_count = 0;
    assert_eq!(
        validate_poststate_outcome(&baseline, &candidate),
        Err(GovernanceError::InvalidRelease1Account)
    );

    // Prestate and Emergency never have a rejected form.
    let mut prestate = dummy_candidate(StateCheckpointPhaseV1::Prestate);
    prestate.forbidden_drift_count = 1;
    assert_eq!(
        validate_checkpoint_acceptance_shape(&prestate),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut emergency = dummy_candidate(StateCheckpointPhaseV1::Emergency);
    emergency.forbidden_drift_count = 1;
    assert_eq!(
        validate_checkpoint_acceptance_shape(&emergency),
        Err(GovernanceError::InvalidRelease1Account)
    );

    // The persisted schema independently prevents accepted=true from being
    // paired with any forbidden drift.
    let mut impossible = baseline;
    impossible.forbidden_drift_count = 1;
    assert_eq!(
        impossible.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
}

#[test]
fn target_snapshot_is_raw_hash_and_authority_hard_without_a_second_payload_hash() {
    let program_id = key(100);
    let target_program = key(101);
    let (_config_key, config) = sample_config(&program_id, target_program);
    let target_programdata = config.target_programdata;
    let wrong_authority = key(102);

    let mut program_bytes = vec![0u8; 36];
    program_bytes[..4].copy_from_slice(&2u32.to_le_bytes());
    program_bytes[4..36].copy_from_slice(target_programdata.as_ref());
    let mut programdata_bytes = vec![0u8; 45 + 64];
    programdata_bytes[..4].copy_from_slice(&3u32.to_le_bytes());
    programdata_bytes[4..12].copy_from_slice(&9u64.to_le_bytes());
    programdata_bytes[12] = 1;
    programdata_bytes[13..45].copy_from_slice(config.authority_pda.as_ref());
    programdata_bytes[45..].fill(7);

    let mut candidate = dummy_candidate(StateCheckpointPhaseV1::Poststate);
    candidate.target_programdata_slot = 9;
    candidate.target_capacity = 64;
    candidate.target_raw_programdata_commitment = loader_account_data_hash(&programdata_bytes);
    candidate.target_payload_commitment = [17; 32];

    let mut program_lamports = 1;
    let mut programdata_lamports = 1;
    let program_info = AccountInfo::new(
        &target_program,
        false,
        false,
        &mut program_lamports,
        &mut program_bytes,
        &UPGRADEABLE_LOADER_ID,
        true,
        0,
    );
    let programdata_info = AccountInfo::new(
        &target_programdata,
        false,
        false,
        &mut programdata_lamports,
        &mut programdata_bytes,
        &UPGRADEABLE_LOADER_ID,
        false,
        0,
    );
    assert_eq!(
        validate_target_snapshot(&program_info, &programdata_info, &config, &candidate),
        Ok(())
    );

    {
        let mut data = programdata_info.try_borrow_mut_data().unwrap();
        data[13..45].copy_from_slice(wrong_authority.as_ref());
        candidate.target_raw_programdata_commitment = loader_account_data_hash(&data);
    }
    assert_eq!(
        validate_target_snapshot(&program_info, &programdata_info, &config, &candidate),
        Err(GovernanceError::Release1DigestMismatch.into())
    );

    {
        let mut data = programdata_info.try_borrow_mut_data().unwrap();
        data[13..45].copy_from_slice(config.authority_pda.as_ref());
    }
    candidate.target_raw_programdata_commitment = [99; 32];
    assert_eq!(
        validate_target_snapshot(&program_info, &programdata_info, &config, &candidate),
        Err(GovernanceError::Release1DigestMismatch.into())
    );
}

#[test]
fn rollback_prestate_requires_the_linked_primary_programdata_verification() {
    let fixture = sample_rollback_evidence_fixture();
    assert!(fixture.evidence.capacity > fixture.evidence.artifact_length);
    assert_ne!(
        fixture.evidence.artifact_sha256,
        fixture.commitment.expected_candidate_full_payload_sha256
    );
    assert_eq!(fixture.validate(), Ok(10));

    let mut wrong_primary = fixture.clone();
    wrong_primary.commitment.primary_proposal = key(123);
    assert_eq!(
        wrong_primary.validate(),
        Err(GovernanceError::CrossAccountMismatch.into())
    );

    let mut wrong_full_hash = fixture.clone();
    wrong_full_hash
        .commitment
        .expected_candidate_full_payload_sha256 = [13; 32];
    assert_eq!(
        wrong_full_hash.validate(),
        Err(GovernanceError::CrossAccountMismatch.into())
    );

    let mut wrong_root = fixture.clone();
    wrong_root.evidence.artifact_chunk_merkle_root = [14; 32];
    assert_eq!(
        wrong_root.validate(),
        Err(GovernanceError::CrossAccountMismatch.into())
    );

    let mut wrong_authority = fixture.clone();
    wrong_authority.evidence.controller_authority = key(124);
    assert_eq!(
        wrong_authority.validate(),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
}

#[test]
fn rollback_failure_prestate_separates_expected_payload_from_actual_raw_programdata() {
    let fixture = sample_rollback_failure_evidence_fixture();
    assert_eq!(
        fixture.candidate.target_payload_commitment,
        fixture.commitment.expected_candidate_full_payload_sha256
    );
    assert_eq!(
        fixture.candidate.target_raw_programdata_commitment,
        fixture.evidence.actual_raw_programdata_sha256
    );
    assert_ne!(
        fixture.candidate.target_payload_commitment,
        fixture.candidate.target_raw_programdata_commitment
    );
    assert_eq!(fixture.validate(), Ok(10));

    let mut stale_epoch = fixture.clone();
    stale_epoch.evidence.frozen_epoch -= 1;
    stale_epoch.evidence_key = derive_programdata_failure_observation_pda(
        &stale_epoch.program_id,
        &stale_epoch.commitment.primary_proposal,
        stale_epoch.evidence.frozen_epoch,
    )
    .0;
    stale_epoch.evidence.bump = derive_programdata_failure_observation_pda(
        &stale_epoch.program_id,
        &stale_epoch.commitment.primary_proposal,
        stale_epoch.evidence.frozen_epoch,
    )
    .1;
    stale_epoch.refresh_digest();
    assert_eq!(
        stale_epoch.validate(),
        Err(GovernanceError::CrossAccountMismatch.into())
    );

    let mut wrong_live_hash = fixture.clone();
    wrong_live_hash.evidence.actual_raw_programdata_sha256 = [42; 32];
    wrong_live_hash.refresh_digest();
    assert_eq!(
        wrong_live_hash.validate(),
        Err(GovernanceError::CrossAccountMismatch.into())
    );

    let mut wrong_authority = fixture.clone();
    wrong_authority.evidence.actual_authority = OptionalPubkeyV1::some(key(133)).unwrap();
    wrong_authority.refresh_digest();
    assert_eq!(
        wrong_authority.validate(),
        Err(GovernanceError::CrossAccountMismatch.into())
    );

    let mut structural_failure = fixture;
    structural_failure.evidence.mismatch_class = ProgramDataMismatchClassV1::Header;
    structural_failure.evidence.failing_chunk_index =
        crate::release1_state::NO_FAILING_CHUNK_INDEX_V1;
    structural_failure.evidence.expected_leaf_hash = [0; 32];
    structural_failure.evidence.actual_leaf_hash = [0; 32];
    structural_failure.refresh_digest();
    assert_eq!(
        structural_failure.validate(),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
}

#[test]
fn rollback_prestate_inherits_the_primary_accepted_prestate_roots() {
    let fixture = sample_rollback_baseline_fixture();
    assert_eq!(fixture.validate(), Ok(fixture.baseline.finalized_slot));

    let mut changed_program_state = fixture.clone();
    changed_program_state.candidate.program_owned_state_root = [50; 32];
    assert_eq!(
        changed_program_state.validate(),
        Err(GovernanceError::InvalidRelease1Account.into())
    );

    let mut changed_schema = fixture.clone();
    changed_schema.candidate.schema_identifier = [51; 32];
    assert_eq!(
        changed_schema.validate(),
        Err(GovernanceError::InvalidRelease1Account.into())
    );

    let mut stale_primary_epoch = fixture.clone();
    stale_primary_epoch.baseline.gate_epoch -= 1;
    stale_primary_epoch.baseline.checkpoint_digest =
        compute_state_checkpoint_digest_v1(&stale_primary_epoch.baseline).unwrap();
    assert_eq!(
        stale_primary_epoch.validate(),
        Err(GovernanceError::CrossAccountMismatch.into())
    );

    let mut donation = fixture;
    donation.candidate.external_raw_balance_observation_root = [52; 32];
    donation.candidate.admitted_positive_donation_root = [53; 32];
    donation.candidate.admitted_positive_donation_count = 1;
    assert_eq!(donation.validate(), Ok(donation.baseline.finalized_slot));
}

#[test]
fn rollback_zero_creation_observations_cannot_bypass_live_prestate_evidence() {
    let fixture = sample_rollback_evidence_fixture();

    let mut zero_observation = fixture.clone();
    zero_observation.candidate.target_programdata_slot = 0;
    zero_observation.candidate.target_raw_programdata_commitment = [0; 32];
    assert_eq!(
        zero_observation.validate(),
        Err(GovernanceError::CrossAccountMismatch.into())
    );

    let mut wrong_live_payload = fixture.clone();
    wrong_live_payload.candidate.target_payload_commitment = [15; 32];
    assert_eq!(
        wrong_live_payload.validate(),
        Err(GovernanceError::CrossAccountMismatch.into())
    );

    let mut wrong_capacity = fixture.clone();
    wrong_capacity.candidate.target_capacity += 1;
    assert_eq!(
        wrong_capacity.validate(),
        Err(GovernanceError::CrossAccountMismatch.into())
    );

    let mut late_evidence = fixture;
    late_evidence.evidence.finalized_slot = late_evidence.candidate.finalized_observation_slot + 1;
    assert_eq!(
        late_evidence.validate(),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
}

#[test]
fn candidate_seat_terms_are_rechecked_at_the_actual_activation_slot() {
    let seats = std::array::from_fn(|index| CouncilSeatV1 {
        seat_authority: key(index as u8 + 20),
        term_start_slot: 5,
        term_end_slot: 100,
        active: true,
        reserved: [0; COUNCIL_SEAT_RESERVED_LEN],
    });
    let mut candidate = GovernanceCouncilSetV1 {
        discriminator: GOVERNANCE_COUNCIL_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump: 1,
        initialized: true,
        controller_config: key(30),
        version: 2,
        target_program: key(31),
        activation_slot: 10,
        deactivation_slot: 0,
        seats,
        routine_threshold: 3,
        terminal_threshold: 4,
        policy_flags: 0,
        set_hash: [1; 32],
        reserved: [0; GOVERNANCE_COUNCIL_RESERVED_LEN],
    };
    assert_eq!(validate_candidate_seats_at_slot(&candidate, 50), Ok(()));
    candidate.seats[4].term_end_slot = 50;
    assert_eq!(
        validate_candidate_seats_at_slot(&candidate, 50),
        Err(GovernanceError::InactiveCouncilSeat)
    );
}

#[test]
fn candidate_council_cannot_turn_the_guardian_into_a_voting_seat() {
    let guardian = key(90);
    let seats = std::array::from_fn(|index| CouncilSeatV1 {
        seat_authority: key(index as u8 + 40),
        term_start_slot: 5,
        term_end_slot: 100,
        active: true,
        reserved: [0; COUNCIL_SEAT_RESERVED_LEN],
    });
    let mut candidate = GovernanceCouncilSetV1 {
        discriminator: GOVERNANCE_COUNCIL_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump: 1,
        initialized: true,
        controller_config: key(50),
        version: 2,
        target_program: key(51),
        activation_slot: 10,
        deactivation_slot: 0,
        seats,
        routine_threshold: 3,
        terminal_threshold: 4,
        policy_flags: 0,
        set_hash: [1; 32],
        reserved: [0; GOVERNANCE_COUNCIL_RESERVED_LEN],
    };
    assert_eq!(
        validate_council_guardian_separation(&candidate, &guardian),
        Ok(())
    );
    candidate.seats[2].seat_authority = guardian;
    assert_eq!(
        validate_council_guardian_separation(&candidate, &guardian),
        Err(GovernanceError::InvalidCouncilComposition)
    );
}

#[test]
fn rotation_guards_bind_nonce_gate_state_timing_and_both_council_hashes() {
    let program_id = key(40);
    let (_config_key, config) = sample_config(&program_id, key(41));
    let gate = ProtocolGateV1 {
        discriminator: PROTOCOL_GATE_DISCRIMINATOR,
        version: ACCOUNT_VERSION_V1,
        bump: 1,
        initialized: true,
        status: GateStatusV1::Active,
        controller_config: key(42),
        target_program: config.target_program,
        target_programdata: config.target_programdata,
        epoch: 5,
        active_proposal: Pubkey::default(),
        freeze_slot: 0,
        freeze_reason_code: 0,
        last_completed_proposal: Pubkey::default(),
        reserved: [0; PROTOCOL_GATE_RESERVED_LEN],
    };
    let rotation = CouncilRotationProposalV1 {
        discriminator: COUNCIL_ROTATION_PROPOSAL_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: 1,
        initialized: true,
        state: CouncilRotationStateV1::Draft,
        controller_config: key(42),
        target_program: config.target_program,
        current_council: key(43),
        current_council_version: 1,
        current_council_hash: [2; 32],
        candidate_council: key(44),
        candidate_council_version: 2,
        candidate_council_hash: [3; 32],
        creation_slot: 10,
        not_before_slot: 30,
        expiry_slot: 100,
        target_nonce: 7,
        approval_bitset: 0,
        approval_count: 0,
        cancellation_approval_bitset: 0,
        cancellation_approval_count: 0,
        rotation_digest: [1; 32],
        activated_slot: 0,
        cancellation_reason_code: 0,
        terminal_reason_code: 0,
        reserved: [0; COUNCIL_ROTATION_PROPOSAL_V1_RESERVED_LEN],
    };
    let expected = rotation_expectation();
    assert_eq!(
        validate_rotation_expectation(&expected, &config, &gate, &rotation),
        Ok(())
    );
    let mut stale = expected;
    stale.expected_target_nonce += 1;
    assert_eq!(
        validate_rotation_expectation(&stale, &config, &gate, &rotation),
        Err(GovernanceError::CrossAccountMismatch)
    );
    let mut stale = expected;
    stale.expected_candidate_council_hash = [99; 32];
    assert_eq!(
        validate_rotation_expectation(&stale, &config, &gate, &rotation),
        Err(GovernanceError::CrossAccountMismatch)
    );
}
