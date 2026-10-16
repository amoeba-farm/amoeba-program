use super::*;
use borsh::BorshDeserialize;
use solana_program::instruction::AccountMeta;

use crate::{
    artifact_merkle::{
        artifact_merkle_proof, artifact_merkle_root, RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
    },
    instruction::{OptionalInstructionPubkeyV1, MAX_FIXED_MERKLE_PROOF_NODES_V1},
    release1_digest::{
        compute_state_checkpoint_digest_v1, compute_state_checkpoint_hard_combined_root_v1,
    },
    state::{
        CouncilSeatV1, GovernanceModeV1, OptionalPubkeyV1, CONTROLLER_CONFIG_RESERVED_LEN,
        GOVERNANCE_COUNCIL_RESERVED_LEN, GOVERNANCE_POLICY_RESERVED_LEN,
    },
};

fn key(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}

fn bytes(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = core::str::from_utf8(pair).unwrap();
            u8::from_str_radix(pair, 16).unwrap()
        })
        .collect()
}

fn fixture_account<T: BorshDeserialize>(name: &str) -> T {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/upgrade_governance_release1.json"
    ))
    .unwrap();
    let encoded = fixture["accounts"][name]["encoded_hex"].as_str().unwrap();
    T::try_from_slice(&decode_hex(encoded)).unwrap()
}

fn proposal_expectation() -> ProposalExpectationV2 {
    ProposalExpectationV2 {
        expected_proposal_digest: bytes(1),
        expected_policy_version: 1,
        expected_policy_hash: bytes(2),
        expected_council_version: 1,
        expected_council_hash: bytes(3),
        expected_gate_status: GateStatusV1::FrozenForUpgrade,
        expected_gate_epoch: 4,
        expected_target_nonce: 5,
        expected_state: ProposalStateV2::Timelocked,
        expected_review_start_slot: 6,
        expected_review_end_slot: 7,
        expected_not_before_slot: 8,
        expected_expiry_slot: 9,
    }
}

fn unfreeze_expectation() -> UnfreezeExpectationV1 {
    UnfreezeExpectationV1 {
        expected_proposal_digest: bytes(10),
        expected_policy_version: 1,
        expected_policy_hash: bytes(11),
        expected_current_council_version: 2,
        expected_current_council_hash: bytes(12),
        expected_frozen_gate_epoch: 13,
        expected_target_nonce: 14,
        expected_proposal_state: ProposalStateV2::PoststateAccepted,
        expected_poststate_checkpoint_digest: bytes(15),
        expected_programdata_authority: key(16),
        expected_programdata_deployed_slot: 17,
        expected_programdata_capacity: 18,
        expected_raw_programdata_hash: bytes(19),
        expected_unfreeze_approval_bitset: 0,
        expected_unfreeze_approval_count: 0,
        expected_programdata_verification_finalized_slot: 20,
    }
}

fn envelope() -> EnvelopeExpectationV1 {
    EnvelopeExpectationV1 {
        compute_unit_limit: 1_200_000,
        compute_unit_price_micro_lamports: 17,
        durable_nonce_account: OptionalInstructionPubkeyV1::none(),
        durable_nonce_authority: OptionalInstructionPubkeyV1::none(),
    }
}

fn empty_observation_instruction() -> ObserveProgramDataFailureV1 {
    ObserveProgramDataFailureV1 {
        expected: proposal_expectation(),
        expected_program_owner: key(21),
        expected_program_executable: false,
        expected_program_data_length: 0,
        expected_program_header_present: false,
        expected_linked_programdata: OptionalInstructionPubkeyV1::none(),
        expected_programdata_owner: key(22),
        expected_programdata_executable: false,
        expected_programdata_data_length: 0,
        expected_programdata_header_present: false,
        expected_programdata_slot: 0,
        expected_raw_hash_complete: true,
        expected_raw_programdata_hash: bytes(23),
        expected_capacity: 0,
        expected_programdata_authority: OptionalInstructionPubkeyV1::none(),
        mismatch_class: ProgramDataMismatchClassV1::Header,
        failing_chunk_index: u32::MAX,
        expected_leaf_hash: [0; 32],
        proof: FixedMerkleProofV1::empty(),
    }
}

fn sample_config(proposal: &UpgradeProposalV2) -> ControllerConfigV1 {
    ControllerConfigV1 {
        discriminator: [0; 8],
        version: 1,
        bump: 1,
        initialized: true,
        cluster_domain: proposal.cluster_domain,
        target_program: proposal.target_program,
        target_programdata: proposal.target_programdata,
        upgradeable_loader: proposal.upgradeable_loader,
        authority_pda: proposal.authority_pda,
        gate_pda: proposal.protocol_gate,
        canonical_spill_treasury: proposal.canonical_spill_treasury,
        current_council_version: 2,
        current_policy_version: proposal.policy_version,
        next_proposal_id: proposal.proposal_id + 2,
        target_nonce: proposal.target_nonce + 1,
        guardian: key(24),
        vote_program: Pubkey::default(),
        vote_programdata: Pubkey::default(),
        vote_config: Pubkey::default(),
        vote_mint: Pubkey::default(),
        token_governance_enabled: false,
        routine_delay_slots: 20,
        major_delay_slots: 30,
        rollback_delay_slots: 10,
        terminal_delay_slots: 40,
        vote_review_slots: 10,
        proposal_expiry_slots: 190,
        policy_flags: 0,
        reserved: [0; CONTROLLER_CONFIG_RESERVED_LEN],
    }
}

fn sample_policy(activation_slot: u64) -> GovernancePolicyV1 {
    GovernancePolicyV1 {
        discriminator: [0; 8],
        account_version: 1,
        bump: 1,
        initialized: true,
        controller_config: key(25),
        version: 1,
        target_program: key(26),
        activation_slot,
        council_size: 5,
        routine_threshold: 3,
        terminal_threshold: 4,
        governance_mode: GovernanceModeV1::BootstrapCouncilOnly,
        policy_flags: 0,
        veto_quorum_bps: 0,
        affirmative_quorum_bps: 0,
        affirmative_approval_bps: 0,
        routine_requires_vote: false,
        economic_requires_vote: false,
        constitutional_requires_vote: false,
        rotation_requires_vote: false,
        immutability_requires_vote: false,
        policy_hash: bytes(27),
        reserved: [0; GOVERNANCE_POLICY_RESERVED_LEN],
    }
}

fn sample_council(version: u64, set_hash: [u8; 32]) -> GovernanceCouncilSetV1 {
    GovernanceCouncilSetV1 {
        discriminator: [0; 8],
        account_version: 1,
        bump: 1,
        initialized: true,
        controller_config: key(28),
        version,
        target_program: key(29),
        activation_slot: 1,
        deactivation_slot: 0,
        seats: [CouncilSeatV1::default(); 5],
        routine_threshold: 3,
        terminal_threshold: 4,
        policy_flags: 0,
        set_hash,
        reserved: [0; GOVERNANCE_COUNCIL_RESERVED_LEN],
    }
}

fn reciprocal_pair() -> (UpgradeProposalV2, Pubkey, UpgradeProposalV2, Pubkey) {
    let mut primary: UpgradeProposalV2 = fixture_account("upgrade_proposal_v2");
    let primary_key = key(30);
    let rollback_key = key(31);
    let mut rollback = primary.clone();
    rollback.proposal_class = ProposalClassV1::EmergencyRollback;
    rollback.state = ProposalStateV2::Timelocked;
    rollback.buffer_pubkey = key(32);
    rollback.artifact_sha256 = bytes(33);
    rollback.artifact_chunk_merkle_root = bytes(34);
    rollback.primary_proposal = OptionalPubkeyV1::some(primary_key).unwrap();
    rollback.rollback_proposal = OptionalPubkeyV1::none();
    rollback.rollback_buffer = OptionalPubkeyV1::none();
    rollback.rollback_artifact_sha256 = [0; 32];
    rollback.rollback_artifact_chunk_root = [0; 32];
    rollback.target_nonce = primary.target_nonce;
    rollback.expected_execution_pre_payload_hash = bytes(35);
    rollback.expected_execution_pre_chunk_root = primary.artifact_chunk_merkle_root;
    rollback.current_capacity = primary.expected_post_capacity;
    rollback.expected_post_capacity = primary.expected_post_capacity;
    rollback.extension_delta = 0;
    rollback.creation_slot = 10;
    rollback.review_start_slot = 11;
    rollback.review_end_slot = 21;
    rollback.not_before_slot = 31;
    rollback.expiry_slot = 200;
    rollback.council_approval_bitset = 0b0_0111;
    rollback.council_approval_count = 3;
    rollback.governance_satisfied_slot = 22;
    rollback.queued_slot = 23;
    rollback.deployed_slot = 0;
    rollback.current_raw_programdata_hash = [0; 32];

    primary.rollback_proposal = OptionalPubkeyV1::some(rollback_key).unwrap();
    primary.rollback_buffer = OptionalPubkeyV1::some(rollback.buffer_pubkey).unwrap();
    primary.rollback_artifact_sha256 = rollback.artifact_sha256;
    primary.rollback_artifact_chunk_root = rollback.artifact_chunk_merkle_root;
    (primary, primary_key, rollback, rollback_key)
}

fn frozen_gate(active_proposal: Pubkey, proposal: &UpgradeProposalV2) -> ProtocolGateV1 {
    ProtocolGateV1 {
        discriminator: [0; 8],
        version: 1,
        bump: 1,
        initialized: true,
        status: GateStatusV1::FrozenForUpgrade,
        controller_config: proposal.controller_config,
        target_program: proposal.target_program,
        target_programdata: proposal.target_programdata,
        epoch: 2,
        active_proposal,
        freeze_slot: 100,
        freeze_reason_code: 2,
        last_completed_proposal: Pubkey::default(),
        reserved: [0; crate::state::PROTOCOL_GATE_RESERVED_LEN],
    }
}

fn leaked_account(
    account_key: Pubkey,
    owner: Pubkey,
    writable: bool,
    signer: bool,
    executable: bool,
    data: Vec<u8>,
) -> AccountInfo<'static> {
    let key_ref = Box::leak(Box::new(account_key));
    let owner_ref = Box::leak(Box::new(owner));
    let lamports = Box::leak(Box::new(1u64));
    let data = Box::leak(data.into_boxed_slice());
    AccountInfo::new(
        key_ref, signer, writable, lamports, data, owner_ref, executable, 0,
    )
}

fn account(
    account_key: Pubkey,
    writable: bool,
    signer: bool,
    executable: bool,
) -> AccountInfo<'static> {
    let key_ref = Box::leak(Box::new(account_key));
    let owner_ref = Box::leak(Box::new(key(250)));
    let lamports = Box::leak(Box::new(1u64));
    let data = Box::leak(vec![account_key.to_bytes()[0]; 4].into_boxed_slice());
    AccountInfo::new(
        key_ref, signer, writable, lamports, data, owner_ref, executable, 0,
    )
}

fn snapshot(accounts: &[AccountInfo<'_>]) -> Vec<Vec<u8>> {
    accounts
        .iter()
        .map(|account| account.try_borrow_data().unwrap().to_vec())
        .collect()
}

fn borrowed_instruction(instruction: &Instruction) -> instructions::BorrowedInstruction<'_> {
    instructions::BorrowedInstruction {
        program_id: &instruction.program_id,
        accounts: instruction
            .accounts
            .iter()
            .map(|meta| instructions::BorrowedAccountMeta {
                pubkey: &meta.pubkey,
                is_signer: meta.is_signer,
                is_writable: meta.is_writable,
            })
            .collect(),
        data: &instruction.data,
    }
}

fn instructions_sysvar(transaction: &[Instruction], current_index: u16) -> AccountInfo<'static> {
    let borrowed: Vec<_> = transaction.iter().map(borrowed_instruction).collect();
    let mut data = instructions::construct_instructions_data(&borrowed);
    let offset = data.len() - 2;
    data[offset..].copy_from_slice(&current_index.to_le_bytes());
    account_with_data(sysvar_ids::instructions::ID, false, false, false, data)
}

fn account_with_data(
    account_key: Pubkey,
    writable: bool,
    signer: bool,
    executable: bool,
    data: Vec<u8>,
) -> AccountInfo<'static> {
    let key_ref = Box::leak(Box::new(account_key));
    let owner_ref = Box::leak(Box::new(key(251)));
    let lamports = Box::leak(Box::new(1u64));
    let data = Box::leak(data.into_boxed_slice());
    AccountInfo::new(
        key_ref, signer, writable, lamports, data, owner_ref, executable, 0,
    )
}

fn compute_limit_instruction(expectation: &EnvelopeExpectationV1) -> Instruction {
    let mut data = [0u8; 5];
    data[0] = 2;
    data[1..].copy_from_slice(&expectation.compute_unit_limit.to_le_bytes());
    Instruction {
        program_id: compute_budget::ID,
        accounts: vec![],
        data: data.to_vec(),
    }
}

fn compute_price_instruction(expectation: &EnvelopeExpectationV1) -> Instruction {
    let mut data = [0u8; 9];
    data[0] = 3;
    data[1..].copy_from_slice(&expectation.compute_unit_price_micro_lamports.to_le_bytes());
    Instruction {
        program_id: compute_budget::ID,
        accounts: vec![],
        data: data.to_vec(),
    }
}

fn fixed_proof(nodes: &[[u8; 32]]) -> FixedMerkleProofV1 {
    let mut fixed = [[0; 32]; MAX_FIXED_MERKLE_PROOF_NODES_V1];
    fixed[..nodes.len()].copy_from_slice(nodes);
    FixedMerkleProofV1 {
        proof_len: nodes.len() as u8,
        nodes: fixed,
    }
}

#[test]
fn all_exports_reject_wrong_account_count_before_state_access() {
    let program_id = key(99);
    let expected = Err(GovernanceError::InvalidAccountCount.into());
    assert_eq!(
        process_approve_unfreeze_v1(
            &program_id,
            &[],
            ApproveUnfreezeV1 {
                expected: unfreeze_expectation(),
            },
        ),
        expected
    );
    assert_eq!(
        process_execute_unfreeze_v1(
            &program_id,
            &[],
            ExecuteUnfreezeV1 {
                expected: unfreeze_expectation(),
                linked_proposal: key(1),
                envelope: envelope(),
            },
        ),
        expected
    );
    assert_eq!(
        process_close_abandoned_buffer_v1(
            &program_id,
            &[],
            CloseAbandonedBufferV1 {
                expected: proposal_expectation(),
                expected_verification_status: BufferVerificationStatusV1::Verified,
                expected_verified_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
                expected_verified_chunk_count: 0,
                expected_buffer_verification_finalized_slot: 0,
            },
        ),
        expected
    );
    assert_eq!(
        process_activate_rollback_v1(
            &program_id,
            &[],
            ActivateRollbackV1 {
                expected_primary: proposal_expectation(),
                expected_rollback: proposal_expectation(),
                expected_failure_evidence_digest: bytes(2),
                expected_primary_programdata_verification_status:
                    ProgramDataVerificationStatusV1::Verifying,
                expected_primary_programdata_verification_finalized_slot: 0,
                expected_rollback_buffer_verification_status: BufferVerificationStatusV1::Verified,
                expected_rollback_verified_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
                expected_rollback_verified_chunk_count: 0,
            },
        ),
        expected
    );
    assert_eq!(
        process_observe_programdata_failure_v1(&program_id, &[], empty_observation_instruction(),),
        expected
    );
}

#[test]
fn every_export_rejects_privilege_drift_and_aliases_without_mutation() {
    let program_id = key(99);
    let approve = || {
        vec![
            account(key(1), false, false, false),
            account(key(2), false, false, false),
            account(key(3), false, false, false),
            account(key(4), false, false, false),
            account(key(5), true, false, false),
            account(key(6), false, false, false),
            account(key(7), false, false, false),
            account(key(8), false, false, true),
            account(key(9), false, false, false),
            account(key(10), false, false, false),
            account(key(11), false, false, true),
            account(key(12), false, true, false),
        ]
    };
    let mut privilege = approve();
    privilege[0].is_writable = true;
    let before = snapshot(&privilege);
    assert_eq!(
        process_approve_unfreeze_v1(
            &program_id,
            &privilege,
            ApproveUnfreezeV1 {
                expected: unfreeze_expectation(),
            },
        ),
        Err(GovernanceError::InvalidAccountPrivileges.into())
    );
    assert_eq!(snapshot(&privilege), before);

    let mut alias = approve();
    alias[1] = alias[0].clone();
    let before = snapshot(&alias);
    assert_eq!(
        process_approve_unfreeze_v1(
            &program_id,
            &alias,
            ApproveUnfreezeV1 {
                expected: unfreeze_expectation(),
            },
        ),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
    assert_eq!(snapshot(&alias), before);

    let mut observe = vec![
        account(key(20), true, false, false),
        account(key(21), false, false, false),
        account(key(22), false, false, false),
        account(key(23), false, false, false),
        account(key(24), false, false, false),
        account(key(25), false, false, false),
        account(key(26), false, false, false),
        account(key(27), false, false, false),
        account(key(28), false, false, true),
        account(key(29), true, false, false),
        account(key(30), false, false, true),
    ];
    observe[0].is_signer = false;
    let before = snapshot(&observe);
    assert_eq!(
        process_observe_programdata_failure_v1(
            &program_id,
            &observe,
            empty_observation_instruction(),
        ),
        Err(GovernanceError::InvalidAccountPrivileges.into())
    );
    assert_eq!(snapshot(&observe), before);
}

#[test]
fn unfreeze_accumulator_resets_on_rotation_and_never_reuses_proposal_votes() {
    let mut proposal: UpgradeProposalV2 = fixture_account("upgrade_proposal_v2");
    proposal.state = ProposalStateV2::PoststateAccepted;
    proposal.council_approval_bitset = 0b0_0111;
    proposal.council_approval_count = 3;
    proposal.unfreeze_council_version = 1;
    proposal.unfreeze_council_hash = bytes(40);
    proposal.unfreeze_approval_bitset = 0b0_0011;
    proposal.unfreeze_approval_count = 2;
    proposal.unfreeze_approved_slot = 0;
    let council = sample_council(2, bytes(41));
    prepare_unfreeze_accumulator(&mut proposal, &council).unwrap();
    assert_eq!(proposal.state, ProposalStateV2::PoststateAccepted);
    assert_eq!(proposal.unfreeze_council_version, 0);
    assert_eq!(proposal.unfreeze_council_hash, [0; 32]);
    assert_eq!(proposal.unfreeze_approval_bitset, 0);
    assert_eq!(proposal.unfreeze_approval_count, 0);
    assert_eq!(proposal.council_approval_bitset, 0b0_0111);
    assert_eq!(proposal.council_approval_count, 3);

    proposal.state = ProposalStateV2::UnfreezeApproved;
    proposal.unfreeze_council_version = council.version;
    proposal.unfreeze_council_hash = council.set_hash;
    proposal.unfreeze_approval_bitset = 0b0_0111;
    proposal.unfreeze_approval_count = 3;
    assert_eq!(
        prepare_unfreeze_accumulator(&mut proposal, &council),
        Err(GovernanceError::DuplicateApproval.into())
    );
}

#[test]
fn structural_programdata_failures_are_rejected_before_rollback_activation_commit() {
    let proposal: UpgradeProposalV2 = fixture_account("upgrade_proposal_v2");
    let config = sample_config(&proposal);
    let mut verification: ProgramDataVerificationV1 =
        fixture_account("programdata_verification_v1");
    verification.deployed_slot = 77;
    verification.capacity = 128;

    let mut failure: ProgramDataFailureObservationV1 =
        fixture_account("programdata_failure_observation_v1");
    failure.actual_program_owner = config.upgradeable_loader;
    failure.actual_program_executable = true;
    failure.actual_program_data_length = LOADER_PROGRAM_ACCOUNT_LEN as u64;
    failure.program_header_present = true;
    failure.actual_linked_programdata = OptionalPubkeyV1::some(config.target_programdata).unwrap();
    failure.raw_hash_complete = true;
    failure.actual_raw_programdata_sha256 = bytes(70);
    failure.actual_owner = config.upgradeable_loader;
    failure.actual_executable = false;
    failure.actual_data_length = verification.capacity + LOADER_PROGRAMDATA_METADATA_LEN as u64;
    failure.programdata_header_present = true;
    failure.actual_programdata_slot = verification.deployed_slot;
    failure.actual_capacity = verification.capacity;
    failure.actual_authority = OptionalPubkeyV1::some(config.authority_pda).unwrap();

    for mismatch_class in [
        ProgramDataMismatchClassV1::PayloadLeaf,
        ProgramDataMismatchClassV1::ZeroTail,
    ] {
        failure.mismatch_class = mismatch_class;
        assert_eq!(
            require_recoverable_rollback_failure_observation(&failure, &config, &verification,),
            Ok(())
        );
    }

    for mismatch_class in [
        ProgramDataMismatchClassV1::Header,
        ProgramDataMismatchClassV1::Authority,
        ProgramDataMismatchClassV1::Capacity,
    ] {
        failure.mismatch_class = mismatch_class;
        let before = failure.clone();
        assert_eq!(
            require_recoverable_rollback_failure_observation(&failure, &config, &verification,),
            Err(GovernanceError::InvalidStateTransition.into())
        );
        assert_eq!(failure, before);
    }

    failure.mismatch_class = ProgramDataMismatchClassV1::PayloadLeaf;
    failure.actual_authority = OptionalPubkeyV1::none();
    assert_eq!(
        require_recoverable_rollback_failure_observation(&failure, &config, &verification),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
}

#[test]
fn rollback_policy_timing_nonce_and_full_payload_commitment_fail_closed() {
    let (mut primary, primary_key, rollback, rollback_key) = reciprocal_pair();
    let mut config = sample_config(&primary);
    let policy = sample_policy(100);
    assert_eq!(
        require_policy_active(&policy, 99),
        Err(GovernanceError::InactivePolicy.into())
    );
    require_policy_active(&policy, 100).unwrap();

    primary.upgrade_executed_slot = 100;
    assert_eq!(
        validate_rollback_activation_timing(&primary, &rollback, &config, 109),
        Err(GovernanceError::InvalidProposalTiming.into())
    );
    validate_rollback_activation_timing(&primary, &rollback, &config, 110).unwrap();
    validate_rollback_activation_timing(&primary, &rollback, &config, 188).unwrap();
    assert_eq!(
        validate_rollback_activation_timing(&primary, &rollback, &config, 189),
        Err(GovernanceError::InvalidProposalTiming.into())
    );
    assert_eq!(
        validate_rollback_activation_timing(&primary, &rollback, &config, 200),
        Err(GovernanceError::InvalidProposalTiming.into())
    );
    assert_eq!(
        validate_rollback_activation_timing(&primary, &rollback, &config, u64::MAX - 5),
        Err(GovernanceError::ArithmeticOverflow.into())
    );

    assert_ne!(
        rollback.expected_execution_pre_payload_hash,
        primary.artifact_sha256
    );
    validate_reciprocal_rollback(&primary, &primary_key, &rollback, &rollback_key, &config)
        .unwrap();
    config.target_nonce = primary.target_nonce;
    assert_eq!(
        validate_reciprocal_rollback(&primary, &primary_key, &rollback, &rollback_key, &config,),
        Err(GovernanceError::InvalidProposalCommitment.into())
    );
}

#[test]
fn reciprocal_rollback_requires_identical_checkpoint_schema_and_policy() {
    let (primary, primary_key, rollback, rollback_key) = reciprocal_pair();
    let config = sample_config(&primary);
    validate_reciprocal_rollback(&primary, &primary_key, &rollback, &rollback_key, &config)
        .unwrap();

    let primary_before = primary.clone();
    let rollback_before = rollback.clone();
    let mut wrong_schema = rollback.clone();
    wrong_schema.checkpoint_schema_id[0] ^= 1;
    assert_eq!(
        validate_reciprocal_rollback(
            &primary,
            &primary_key,
            &wrong_schema,
            &rollback_key,
            &config,
        ),
        Err(GovernanceError::InvalidProposalCommitment.into())
    );

    let mut wrong_policy = rollback.clone();
    wrong_policy.checkpoint_policy_hash[0] ^= 1;
    assert_eq!(
        validate_reciprocal_rollback(
            &primary,
            &primary_key,
            &wrong_policy,
            &rollback_key,
            &config,
        ),
        Err(GovernanceError::InvalidProposalCommitment.into())
    );
    assert_eq!(primary, primary_before);
    assert_eq!(rollback, rollback_before);
}

#[test]
fn primary_freeze_locks_cancelled_or_expired_rollback_buffer_until_retirement() {
    let (primary, primary_key, mut rollback, rollback_key) = reciprocal_pair();
    let config = sample_config(&primary);
    let gate = frozen_gate(primary_key, &primary);

    for terminal_state in [ProposalStateV2::Cancelled, ProposalStateV2::Expired] {
        rollback.state = terminal_state;
        let before = rollback.clone();
        assert_eq!(
            validate_abandoned_buffer_close_state(&rollback, &rollback_key, &config, &gate,),
            Err(GovernanceError::InvalidStateTransition.into())
        );
        assert_eq!(rollback, before);
    }

    rollback.state = ProposalStateV2::Retired;
    assert_eq!(
        validate_abandoned_buffer_close_state(&rollback, &rollback_key, &config, &gate),
        Ok(())
    );
}

#[test]
fn reciprocal_unfreeze_terminalization_is_class_specific() {
    let (mut primary, primary_key, mut rollback, rollback_key) = reciprocal_pair();
    primary.state = ProposalStateV2::UnfreezeApproved;
    terminalize_unfreeze_pair(&primary, &primary_key, &mut rollback, &rollback_key, 120).unwrap();
    assert_eq!(rollback.state, ProposalStateV2::Retired);
    assert_eq!(rollback.terminal_slot, 120);
    assert_eq!(
        rollback.terminal_reason_code,
        PROPOSAL_RETIRED_ROLLBACK_TERMINAL_REASON_V1
    );

    primary.state = ProposalStateV2::ProgramDataVerified;
    rollback.state = ProposalStateV2::UnfreezeApproved;
    terminalize_unfreeze_pair(&rollback, &rollback_key, &mut primary, &primary_key, 121).unwrap();
    assert_eq!(primary.state, ProposalStateV2::SupersededByRollback);
    assert_eq!(
        primary.terminal_reason_code,
        PROPOSAL_SUPERSEDED_BY_ROLLBACK_TERMINAL_REASON_V1
    );

    let mut wrong = rollback;
    wrong.primary_proposal = OptionalPubkeyV1::some(key(99)).unwrap();
    assert_eq!(
        terminalize_unfreeze_pair(&primary, &primary_key, &mut wrong, &rollback_key, 122,),
        Err(GovernanceError::InvalidProposalCommitment.into())
    );
}

#[test]
fn rejected_poststate_evidence_survives_rotation_but_false_rejection_fails() {
    let mut checkpoint: StateCheckpointV1 = fixture_account("state_checkpoint_v1");
    checkpoint.approval_council_version = 2;
    checkpoint.approval_council_hash = bytes(42);
    checkpoint.approval_bitset = 0b0_0111;
    checkpoint.approval_count = 3;
    checkpoint.accepted = false;
    checkpoint.forbidden_drift_count = 1;
    checkpoint.hard_combined_root =
        compute_state_checkpoint_hard_combined_root_v1(&checkpoint).unwrap();
    checkpoint.checkpoint_digest = compute_state_checkpoint_digest_v1(&checkpoint).unwrap();
    checkpoint.validate_schema().unwrap();
    let rotated_current_council_version = 3;
    assert_ne!(
        checkpoint.approval_council_version,
        rotated_current_council_version
    );
    validate_rejected_checkpoint_finalization(
        &checkpoint,
        checkpoint.finalized_slot - 1,
        checkpoint.finalized_slot,
    )
    .unwrap();

    let mut falsely_accepted = checkpoint.clone();
    falsely_accepted.accepted = true;
    assert_eq!(
        validate_rejected_checkpoint_finalization(
            &falsely_accepted,
            checkpoint.finalized_slot - 1,
            checkpoint.finalized_slot,
        ),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
    let mut no_forbidden_drift = checkpoint;
    no_forbidden_drift.forbidden_drift_count = 0;
    assert_eq!(
        validate_rejected_checkpoint_finalization(
            &no_forbidden_drift,
            no_forbidden_drift.finalized_slot - 1,
            no_forbidden_drift.finalized_slot,
        ),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
}

#[test]
fn failure_chunk_order_and_zero_tail_domain_are_canonical() {
    let mut bitmap = [0u8; VERIFICATION_BITMAP_BYTES_V1];
    bitmap[0] = 0b0000_0011;
    require_verified_prefix(&bitmap, 2).unwrap();
    bitmap[0] = 0b0000_0001;
    assert_eq!(
        require_verified_prefix(&bitmap, 2),
        Err(GovernanceError::InvalidRelease1Bitmap.into())
    );

    for length in [1usize, 1_023, 1_024, 1_025, 16 * 1024] {
        let index = 7u32;
        let length_bytes = (length as u32).to_le_bytes();
        let index_bytes = index.to_le_bytes();
        let zeroes = vec![0u8; length];
        let direct = hashv(&[
            PROGRAMDATA_ZERO_TAIL_CHUNK_DOMAIN_V1,
            &index_bytes,
            &length_bytes,
            &zeroes,
        ])
        .to_bytes();
        assert_eq!(
            programdata_zero_tail_zero_hash(index, length).unwrap(),
            direct
        );
        assert_eq!(
            programdata_zero_tail_chunk_hash(index, &zeroes).unwrap(),
            direct
        );
    }
    let nonzero = vec![1u8; 17];
    assert_ne!(
        programdata_zero_tail_chunk_hash(0, &nonzero).unwrap(),
        programdata_zero_tail_zero_hash(0, nonzero.len()).unwrap()
    );
}

#[test]
fn committed_leaf_proofs_reject_wrong_leaf_index_and_padding() {
    let chunk_size = RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as usize;
    let artifact: Vec<u8> = (0..chunk_size * 2 + 17)
        .map(|index| (index.wrapping_mul(31) & 0xff) as u8)
        .collect();
    let root = artifact_merkle_root(&artifact, RELEASE1_ARTIFACT_CHUNK_SIZE_V1).unwrap();
    let nodes = artifact_merkle_proof(&artifact, RELEASE1_ARTIFACT_CHUNK_SIZE_V1, 2).unwrap();
    let proof = fixed_proof(&nodes);
    let leaf = artifact_chunk_leaf_hash(2, &artifact[chunk_size * 2..]).unwrap();
    validate_expected_leaf_proof(
        &root,
        artifact.len() as u64,
        RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        2,
        &leaf,
        &proof,
    )
    .unwrap();

    let mut wrong_leaf = leaf;
    wrong_leaf[0] ^= 1;
    assert!(validate_expected_leaf_proof(
        &root,
        artifact.len() as u64,
        RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        2,
        &wrong_leaf,
        &proof,
    )
    .is_err());
    assert!(validate_expected_leaf_proof(
        &root,
        artifact.len() as u64,
        RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        3,
        &leaf,
        &proof,
    )
    .is_err());
    let mut wrong_padding = proof;
    wrong_padding.nodes[0][0] ^= 1;
    assert!(validate_expected_leaf_proof(
        &root,
        artifact.len() as u64,
        RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        2,
        &leaf,
        &wrong_padding,
    )
    .is_err());
}

#[test]
fn unfreeze_envelope_rejects_every_sibling_instruction() {
    let program_id = key(50);
    let expectation = envelope();
    let current_data = vec![35, 1, 2, 3];
    let current = Instruction {
        program_id,
        accounts: vec![],
        data: current_data.clone(),
    };
    let valid = vec![
        compute_limit_instruction(&expectation),
        compute_price_instruction(&expectation),
        current,
    ];
    let valid_sysvar = instructions_sysvar(&valid, 2);
    validate_canonical_envelope(&program_id, &[], &valid_sysvar, &current_data, &expectation)
        .unwrap();

    let mut sibling = valid;
    sibling.push(Instruction {
        program_id: key(51),
        accounts: vec![AccountMeta::new(key(52), false)],
        data: vec![1],
    });
    let sibling_sysvar = instructions_sysvar(&sibling, 2);
    assert_eq!(
        validate_canonical_envelope(
            &program_id,
            &[],
            &sibling_sysvar,
            &current_data,
            &expectation,
        ),
        Err(GovernanceError::InvalidAccountCount.into())
    );
}

#[test]
fn close_treasury_authority_and_loader_shape_are_exact_and_failure_atomic() {
    let proposal: UpgradeProposalV2 = fixture_account("upgrade_proposal_v2");
    let config = sample_config(&proposal);
    validate_close_identities(
        &config,
        &proposal,
        &proposal.buffer_verification,
        &proposal.buffer_pubkey,
        &proposal.canonical_spill_treasury,
        &proposal.authority_pda,
    )
    .unwrap();
    let proposal_before = proposal.clone();
    assert_eq!(
        validate_close_identities(
            &config,
            &proposal,
            &proposal.buffer_verification,
            &proposal.buffer_pubkey,
            &key(60),
            &proposal.authority_pda,
        ),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
    assert_eq!(proposal, proposal_before);
    assert_eq!(
        validate_close_identities(
            &config,
            &proposal,
            &proposal.buffer_verification,
            &proposal.buffer_pubkey,
            &proposal.canonical_spill_treasury,
            &key(61),
        ),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
    assert_eq!(proposal, proposal_before);

    let close_instruction = close(
        &proposal.buffer_pubkey,
        &proposal.canonical_spill_treasury,
        &proposal.authority_pda,
    );
    validate_close_cpi_shape(
        &close_instruction,
        &proposal.buffer_pubkey,
        &proposal.canonical_spill_treasury,
        &proposal.authority_pda,
    )
    .unwrap();
    assert!(validate_close_cpi_shape(
        &close_instruction,
        &proposal.buffer_pubkey,
        &key(62),
        &proposal.authority_pda,
    )
    .is_err());
    assert!(validate_close_cpi_shape(
        &close_instruction,
        &proposal.buffer_pubkey,
        &proposal.canonical_spill_treasury,
        &key(63),
    )
    .is_err());
}

#[test]
fn multi_account_commits_validate_every_target_before_first_write() {
    let program_id = key(70);
    let first = leaked_account(key(71), program_id, true, false, false, vec![1; 4]);
    let second = leaked_account(key(72), program_id, true, false, false, vec![2; 3]);
    let first_before = first.try_borrow_data().unwrap().to_vec();
    assert_eq!(
        commit_two_fixed_accounts(&program_id, &first, &[9; 4], 4, &second, &[8; 4], 4,),
        Err(GovernanceError::InvalidAccountSize.into())
    );
    assert_eq!(first.try_borrow_data().unwrap().to_vec(), first_before);

    let second = leaked_account(key(73), program_id, true, false, false, vec![2; 4]);
    let third = leaked_account(key(74), program_id, true, false, false, vec![3; 3]);
    let first_before = first.try_borrow_data().unwrap().to_vec();
    let second_before = second.try_borrow_data().unwrap().to_vec();
    assert_eq!(
        commit_three_fixed_accounts(
            &program_id,
            &first,
            &[9; 4],
            4,
            &second,
            &[8; 4],
            4,
            &third,
            &[7; 4],
            4,
        ),
        Err(GovernanceError::InvalidAccountSize.into())
    );
    assert_eq!(first.try_borrow_data().unwrap().to_vec(), first_before);
    assert_eq!(second.try_borrow_data().unwrap().to_vec(), second_before);
}
