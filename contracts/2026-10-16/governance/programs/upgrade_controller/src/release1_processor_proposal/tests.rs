use super::*;
use crate::state::{
    CouncilSeatV1, COUNCIL_SEAT_RESERVED_LEN, GOVERNANCE_COUNCIL_DISCRIMINATOR,
    GOVERNANCE_COUNCIL_RESERVED_LEN,
};
use solana_program::instruction::{AccountMeta, Instruction};

fn key(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}

fn bytes(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn leaked_account(
    account_key: Pubkey,
    writable: bool,
    signer: bool,
    data: Vec<u8>,
) -> AccountInfo<'static> {
    let key_ref = Box::leak(Box::new(account_key));
    let owner_ref = Box::leak(Box::new(key(250)));
    let lamports = Box::leak(Box::new(1u64));
    let data = Box::leak(data.into_boxed_slice());
    AccountInfo::new(
        key_ref, signer, writable, lamports, data, owner_ref, false, 0,
    )
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

fn instruction_sysvar_data(transaction: &[Instruction], current_index: u16) -> Vec<u8> {
    let borrowed: Vec<_> = transaction.iter().map(borrowed_instruction).collect();
    let mut data = instructions::construct_instructions_data(&borrowed);
    let offset = data.len() - 2;
    data[offset..].copy_from_slice(&current_index.to_le_bytes());
    data
}

fn compute_limit(units: u32) -> Instruction {
    let mut data = [0u8; 5];
    data[0] = 2;
    data[1..].copy_from_slice(&units.to_le_bytes());
    Instruction {
        program_id: compute_budget::ID,
        accounts: vec![],
        data: data.to_vec(),
    }
}

fn compute_price(micro_lamports: u64) -> Instruction {
    let mut data = [0u8; 9];
    data[0] = 3;
    data[1..].copy_from_slice(&micro_lamports.to_le_bytes());
    Instruction {
        program_id: compute_budget::ID,
        accounts: vec![],
        data: data.to_vec(),
    }
}

fn timing_config() -> ControllerConfigV1 {
    ControllerConfigV1 {
        discriminator: crate::state::CONTROLLER_CONFIG_DISCRIMINATOR,
        version: crate::state::ACCOUNT_VERSION_V1,
        bump: 1,
        initialized: true,
        cluster_domain: [1; 32],
        target_program: key(1),
        target_programdata: key(2),
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        authority_pda: key(3),
        gate_pda: key(4),
        canonical_spill_treasury: key(5),
        current_council_version: 1,
        current_policy_version: 1,
        next_proposal_id: 1,
        target_nonce: 1,
        guardian: key(6),
        vote_program: Pubkey::default(),
        vote_programdata: Pubkey::default(),
        vote_config: Pubkey::default(),
        vote_mint: Pubkey::default(),
        token_governance_enabled: false,
        routine_delay_slots: 10,
        major_delay_slots: 20,
        rollback_delay_slots: 5,
        terminal_delay_slots: 30,
        vote_review_slots: 4,
        proposal_expiry_slots: 100,
        policy_flags: 0,
        reserved: [0; crate::state::CONTROLLER_CONFIG_RESERVED_LEN],
    }
}

fn active_council() -> GovernanceCouncilSetV1 {
    let seats = std::array::from_fn(|index| CouncilSeatV1 {
        seat_authority: key(40 + index as u8),
        term_start_slot: 1,
        term_end_slot: 100,
        active: true,
        reserved: [0; COUNCIL_SEAT_RESERVED_LEN],
    });
    GovernanceCouncilSetV1 {
        discriminator: GOVERNANCE_COUNCIL_DISCRIMINATOR,
        account_version: crate::state::ACCOUNT_VERSION_V1,
        bump: 1,
        initialized: true,
        controller_config: key(30),
        version: 1,
        target_program: key(31),
        activation_slot: 1,
        deactivation_slot: 0,
        seats,
        routine_threshold: RELEASE1_APPROVAL_THRESHOLD,
        terminal_threshold: 4,
        policy_flags: 0,
        set_hash: [1; 32],
        reserved: [0; GOVERNANCE_COUNCIL_RESERVED_LEN],
    }
}

fn proposal_expectation() -> ProposalExpectationV2 {
    ProposalExpectationV2 {
        expected_proposal_digest: [1; 32],
        expected_policy_version: 1,
        expected_policy_hash: [2; 32],
        expected_council_version: 1,
        expected_council_hash: [3; 32],
        expected_gate_status: GateStatusV1::Active,
        expected_gate_epoch: 1,
        expected_target_nonce: 1,
        expected_state: ProposalStateV2::Draft,
        expected_review_start_slot: 2,
        expected_review_end_slot: 3,
        expected_not_before_slot: 4,
        expected_expiry_slot: 5,
    }
}

fn create_proposal_instruction() -> CreateProposalV2 {
    CreateProposalV2 {
        proposal_class: ProposalClassV1::RoutineUpgrade,
        creation_gate_status: GateStatusV1::Active,
        expected_proposal_id: 1,
        expected_target_nonce: 2,
        creation_slot: 3,
        expected_policy_version: 4,
        expected_policy_hash: bytes(4),
        expected_creation_council_version: 5,
        expected_creation_council_hash: bytes(5),
        expected_creation_gate_epoch: 6,
        expected_freeze_gate_epoch: 0,
        artifact_length: 8,
        artifact_sha256: bytes(8),
        artifact_chunk_merkle_root: bytes(9),
        source_commit_hash: bytes(10),
        source_tree_hash: bytes(11),
        build_input_inventory_hash: bytes(12),
        reproducible_build_receipt_hash: bytes(13),
        package_receipt_hash: bytes(14),
        release_intent_hash: bytes(15),
        expected_execution_pre_payload_hash: bytes(16),
        expected_execution_pre_chunk_root: bytes(17),
        current_raw_programdata_hash: bytes(18),
        deployed_slot: 19,
        current_capacity: 20,
        extension_delta: 21,
        expected_post_capacity: 41,
        checkpoint_schema_id: bytes(22),
        checkpoint_policy_hash: bytes(23),
        primary_proposal: crate::instruction::OptionalInstructionPubkeyV1::none(),
        rollback_proposal: crate::instruction::OptionalInstructionPubkeyV1::some(key(24)).unwrap(),
        rollback_buffer: crate::instruction::OptionalInstructionPubkeyV1::some(key(25)).unwrap(),
        rollback_artifact_sha256: bytes(26),
        rollback_artifact_chunk_root: bytes(27),
        review_start_slot: 28,
        review_end_slot: 29,
        not_before_slot: 30,
        expiry_slot: 31,
        expected_proposal_digest: bytes(32),
    }
}

fn guardian_freeze_instruction() -> GuardianFreezeV1 {
    GuardianFreezeV1 {
        expected_gate_status: GateStatusV1::Active,
        expected_gate_epoch: 44,
        expected_next_gate_epoch: 45,
        expected_target_nonce: 46,
        expected_program_owner: key(43),
        expected_program_executable: true,
        expected_program_data_length: 36,
        expected_program_header_present: true,
        expected_linked_programdata: crate::instruction::OptionalInstructionPubkeyV1::some(key(44))
            .unwrap(),
        expected_programdata_owner: key(47),
        expected_programdata_executable: false,
        expected_programdata_data_length: 4_141,
        expected_programdata_header_present: true,
        expected_programdata_slot: 47,
        expected_raw_hash_complete: true,
        expected_raw_programdata_hash: bytes(48),
        expected_capacity: 4_096,
        expected_programdata_authority: crate::instruction::OptionalInstructionPubkeyV1::some(key(
            50,
        ))
        .unwrap(),
        freeze_reason_code: 51,
        expected_observation_digest: bytes(52),
    }
}

fn create_emergency_resolution_instruction() -> CreateEmergencyResolutionV1 {
    CreateEmergencyResolutionV1 {
        resolution_kind: EmergencyFreezeResolutionKindV1::ResumeWithoutUpgrade,
        creation_slot: 51,
        not_before_slot: 52,
        expiry_slot: 53,
        expected_policy_version: 54,
        expected_policy_hash: bytes(55),
        expected_council_version: 56,
        expected_council_hash: bytes(57),
        expected_gate_epoch: 58,
        expected_freeze_slot: 59,
        expected_freeze_reason_code: 60,
        expected_target_nonce: 61,
        expected_freeze_observation_digest: bytes(62),
        observed_program_owner: key(58),
        observed_program_executable: true,
        observed_program_data_length: 36,
        observed_program_header_present: true,
        observed_linked_programdata: crate::instruction::OptionalInstructionPubkeyV1::some(key(59))
            .unwrap(),
        observed_programdata_owner: key(63),
        observed_programdata_executable: false,
        observed_programdata_data_length: 4_141,
        observed_programdata_header_present: true,
        observed_programdata_slot: 63,
        observed_raw_hash_complete: true,
        observed_raw_programdata_hash: bytes(64),
        observed_capacity: 4_096,
        observed_programdata_authority: crate::instruction::OptionalInstructionPubkeyV1::some(key(
            66,
        ))
        .unwrap(),
        expected_resolution_digest: bytes(67),
    }
}

fn emergency_expectation() -> EmergencyResolutionExpectationV1 {
    EmergencyResolutionExpectationV1 {
        expected_resolution_digest: bytes(67),
        expected_policy_version: 68,
        expected_policy_hash: bytes(69),
        expected_council_version: 70,
        expected_council_hash: bytes(71),
        expected_gate_status: GateStatusV1::EmergencyFrozen,
        expected_gate_epoch: 72,
        expected_freeze_slot: 73,
        expected_freeze_reason_code: 74,
        expected_target_nonce: 75,
        expected_state: EmergencyFreezeResolutionStateV1::Timelocked,
        expected_not_before_slot: 76,
        expected_expiry_slot: 77,
    }
}

fn execute_emergency_resolution_instruction() -> ExecuteEmergencyResolutionV1 {
    ExecuteEmergencyResolutionV1 {
        expected: emergency_expectation(),
        expected_freeze_observation_digest: bytes(127),
        expected_checkpoint_digest: bytes(128),
        expected_program_owner: UPGRADEABLE_LOADER_ID,
        expected_program_executable: true,
        expected_program_data_length: 36,
        expected_program_header_present: true,
        expected_linked_programdata: crate::instruction::OptionalInstructionPubkeyV1::some(key(
            126,
        ))
        .unwrap(),
        expected_programdata_owner: UPGRADEABLE_LOADER_ID,
        expected_programdata_executable: false,
        expected_programdata_data_length: 4_141,
        expected_programdata_header_present: true,
        expected_programdata_slot: 129,
        expected_raw_hash_complete: true,
        expected_raw_programdata_hash: bytes(130),
        expected_capacity: 4_096,
        expected_programdata_authority: crate::instruction::OptionalInstructionPubkeyV1::some(key(
            132,
        ))
        .unwrap(),
    }
}

fn invalid_account_count() -> ProgramResult {
    Err(ProgramError::Custom(
        GovernanceError::InvalidAccountCount as u32,
    ))
}

#[test]
fn every_export_rejects_wrong_account_count_before_clock_or_account_data() {
    let program_id = key(200);
    let proposal_expected = proposal_expectation();
    let emergency_expected = emergency_expectation();
    let results = [
        process_create_proposal_v2(&program_id, &[], create_proposal_instruction()),
        process_approve_proposal_v2(
            &program_id,
            &[],
            ApproveProposalV2 {
                expected: proposal_expected,
                expected_approval_bitset: 0,
                expected_approval_count: 0,
            },
        ),
        process_finalize_governance_v2(
            &program_id,
            &[],
            FinalizeGovernanceV2 {
                expected: proposal_expected,
                expected_approval_bitset: 0,
                expected_approval_count: 0,
            },
        ),
        process_queue_proposal_v2(
            &program_id,
            &[],
            QueueProposalV2 {
                expected: proposal_expected,
            },
        ),
        process_freeze_proposal_v2(
            &program_id,
            &[],
            FreezeProposalV2 {
                expected: proposal_expected,
                expected_next_gate_epoch: 2,
            },
        ),
        process_cancel_proposal_v2(
            &program_id,
            &[],
            CancelProposalV2 {
                expected: proposal_expected,
                expected_cancellation_approval_bitset: 0,
                expected_cancellation_approval_count: 0,
                cancellation_reason_code: 1,
            },
        ),
        process_expire_proposal_v2(
            &program_id,
            &[],
            ExpireProposalV2 {
                expected: proposal_expected,
            },
        ),
        process_guardian_freeze_v1(&program_id, &[], guardian_freeze_instruction()),
        process_create_emergency_resolution_v1(
            &program_id,
            &[],
            create_emergency_resolution_instruction(),
        ),
        process_approve_emergency_resolution_v1(
            &program_id,
            &[],
            ApproveEmergencyResolutionV1 {
                expected: emergency_expected,
                expected_approval_bitset: 0,
                expected_approval_count: 0,
            },
        ),
        process_queue_emergency_resolution_v1(
            &program_id,
            &[],
            QueueEmergencyResolutionV1 {
                expected: emergency_expected,
                expected_approval_bitset: 0b00111,
                expected_approval_count: RELEASE1_APPROVAL_THRESHOLD,
            },
        ),
        process_execute_emergency_resolution_v1(
            &program_id,
            &[],
            execute_emergency_resolution_instruction(),
        ),
        process_convert_emergency_freeze_v2(
            &program_id,
            &[],
            ConvertEmergencyFreezeV2 {
                expected: proposal_expected,
                expected_next_gate_epoch: 2,
                expected_freeze_observation_digest: bytes(201),
            },
        ),
    ];
    for result in results {
        assert_eq!(result, invalid_account_count());
    }
}

#[test]
fn proposal_timing_is_class_selected_and_checked() {
    let config = timing_config();
    assert_eq!(
        derive_proposal_timing(&config, ProposalClassV1::EmergencyRollback, 100).unwrap(),
        (101, 105, 110, 200)
    );
    assert_eq!(
        derive_proposal_timing(&config, ProposalClassV1::RoutineUpgrade, 100).unwrap(),
        (101, 105, 115, 200)
    );
    assert_eq!(
        derive_proposal_timing(&config, ProposalClassV1::EconomicChange, 100).unwrap(),
        (101, 105, 125, 200)
    );
    assert_eq!(
        derive_proposal_timing(&config, ProposalClassV1::TargetImmutability, 100),
        Err(GovernanceError::UnsupportedProposalClass)
    );
}

#[test]
fn timing_overflow_and_equality_fail_closed() {
    let mut config = timing_config();
    assert_eq!(
        derive_proposal_timing(&config, ProposalClassV1::RoutineUpgrade, u64::MAX),
        Err(GovernanceError::ArithmeticOverflow)
    );
    config.proposal_expiry_slots = 15;
    assert_eq!(
        derive_proposal_timing(&config, ProposalClassV1::RoutineUpgrade, 100),
        Err(GovernanceError::InvalidProposalTiming)
    );
    assert_eq!(
        checked_freeze_counters(u64::MAX, 1),
        Err(GovernanceError::ArithmeticOverflow)
    );
    assert_eq!(
        checked_freeze_counters(1, u64::MAX),
        Err(GovernanceError::ArithmeticOverflow)
    );
    assert_eq!(checked_freeze_counters(8, 9), Ok((9, 10)));
}

#[test]
fn freeze_runway_reserves_checkpoint_and_extension_slots() {
    // With a four-slot checkpoint window, a no-extension proposal needs
    // one additional execution slot and an extension proposal needs two.
    assert_eq!(require_freeze_execution_runway(0, 100, 4, 94), Ok(()));
    assert_eq!(
        require_freeze_execution_runway(0, 100, 4, 95),
        Err(GovernanceError::InvalidProposalTiming.into())
    );
    assert_eq!(require_freeze_execution_runway(1, 100, 4, 93), Ok(()));
    assert_eq!(
        require_freeze_execution_runway(1, 100, 4, 94),
        Err(GovernanceError::InvalidProposalTiming.into())
    );
    assert_eq!(
        require_freeze_execution_runway(0, u64::MAX, 4, u64::MAX - 2),
        Err(GovernanceError::ArithmeticOverflow.into())
    );
}

#[test]
fn emergency_resume_envelope_is_compute_bounded_and_has_no_sibling() {
    let program_id = key(200);
    let account_key = key(201);
    let expected_data = vec![13, 7, 8];
    let current = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(account_key, false),
            AccountMeta::new_readonly(sysvar_ids::instructions::ID, false),
        ],
        data: expected_data.clone(),
    };
    let canonical = vec![
        compute_limit(MAX_ENVELOPE_COMPUTE_UNIT_LIMIT_V1),
        compute_price(MAX_ENVELOPE_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1),
        current.clone(),
    ];
    let state_info = leaked_account(account_key, false, false, vec![]);
    let sysvar_info = leaked_account(
        sysvar_ids::instructions::ID,
        false,
        false,
        instruction_sysvar_data(&canonical, 2),
    );
    assert_eq!(
        validate_bounded_emergency_resolution_envelope(
            &program_id,
            &[state_info.clone(), sysvar_info],
            &leaked_account(
                sysvar_ids::instructions::ID,
                false,
                false,
                instruction_sysvar_data(&canonical, 2),
            ),
            &expected_data,
        ),
        Ok(())
    );

    let mut with_sibling = canonical.clone();
    with_sibling.push(Instruction {
        program_id: key(202),
        accounts: vec![],
        data: vec![],
    });
    let sibling_sysvar = leaked_account(
        sysvar_ids::instructions::ID,
        false,
        false,
        instruction_sysvar_data(&with_sibling, 2),
    );
    assert_eq!(
        validate_bounded_emergency_resolution_envelope(
            &program_id,
            &[state_info.clone(), sibling_sysvar.clone()],
            &sibling_sysvar,
            &expected_data,
        ),
        Err(GovernanceError::InvalidAccountCount.into())
    );

    let underflow_limit = vec![compute_limit(0), compute_price(0), current];
    let underflow_sysvar = leaked_account(
        sysvar_ids::instructions::ID,
        false,
        false,
        instruction_sysvar_data(&underflow_limit, 2),
    );
    assert_eq!(
        validate_bounded_emergency_resolution_envelope(
            &program_id,
            &[state_info, underflow_sysvar.clone()],
            &underflow_sysvar,
            &expected_data,
        ),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
}

#[test]
fn proposal_approval_window_boundaries_fail_closed() {
    assert_eq!(require_approval_window(10, 20, 30, 10), Ok(()));
    assert_eq!(require_approval_window(10, 20, 30, 20), Ok(()));
    for slot in [9, 21, 30, 31] {
        assert_eq!(
            require_approval_window(10, 20, 30, slot),
            Err(ProgramError::Custom(
                GovernanceError::InvalidProposalTiming as u32
            )),
            "slot {slot} must be outside the immutable approval window"
        );
    }
}

#[test]
fn stale_nonce_gate_status_epoch_and_bootstrap_freeze_fail_closed() {
    assert_eq!(
        require_creation_gate_values(7, GateStatusV1::Active, 11, 7, GateStatusV1::Active, 11, 0),
        Ok(())
    );
    for result in [
        require_creation_gate_values(6, GateStatusV1::Active, 11, 7, GateStatusV1::Active, 11, 0),
        require_creation_gate_values(
            7,
            GateStatusV1::EmergencyFrozen,
            11,
            7,
            GateStatusV1::Active,
            11,
            0,
        ),
        require_creation_gate_values(7, GateStatusV1::Active, 10, 7, GateStatusV1::Active, 11, 0),
        require_creation_gate_values(
            7,
            GateStatusV1::FrozenForUpgrade,
            11,
            7,
            GateStatusV1::FrozenForUpgrade,
            11,
            GOVERNED_UPGRADE_FREEZE_REASON_V1,
        ),
        require_creation_gate_values(
            7,
            GateStatusV1::EmergencyFrozen,
            11,
            7,
            GateStatusV1::EmergencyFrozen,
            11,
            BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
        ),
    ] {
        assert_eq!(
            result,
            Err(ProgramError::Custom(
                GovernanceError::InvalidProposalEpoch as u32
            ))
        );
    }
    assert_eq!(
        require_creation_gate_values(
            7,
            GateStatusV1::EmergencyFrozen,
            11,
            7,
            GateStatusV1::EmergencyFrozen,
            11,
            99,
        ),
        Ok(())
    );
}

#[test]
fn prepared_rollback_cannot_be_cancelled_or_expired_after_primary_freeze() {
    let primary_key = key(170);
    let rollback = UpgradeProposalV2 {
        proposal_class: ProposalClassV1::EmergencyRollback,
        target_nonce: 7,
        primary_proposal: OptionalPubkeyV1::some(primary_key).unwrap(),
        ..UpgradeProposalV2::default()
    };
    let mut config = timing_config();
    config.target_nonce = 8;
    let mut gate = ProtocolGateV1 {
        discriminator: [0; 8],
        version: 1,
        bump: 1,
        initialized: true,
        status: GateStatusV1::FrozenForUpgrade,
        controller_config: key(171),
        target_program: config.target_program,
        target_programdata: config.target_programdata,
        epoch: 2,
        active_proposal: primary_key,
        freeze_slot: 10,
        freeze_reason_code: GOVERNED_UPGRADE_FREEZE_REASON_V1,
        last_completed_proposal: Pubkey::default(),
        reserved: [0; crate::state::PROTOCOL_GATE_RESERVED_LEN],
    };
    let before = rollback.clone();

    // Both CancelProposalV2 and ExpireProposalV2 call this guard before
    // modifying their detached proposal value or committing account bytes.
    for operation in ["cancel", "expire"] {
        assert_eq!(
            reject_locked_reciprocal_rollback(&rollback, &config, &gate),
            Err(GovernanceError::InvalidStateTransition.into()),
            "{operation} must preserve the sealed recovery capability"
        );
        assert_eq!(rollback, before, "{operation} changed proposal state");
    }

    gate.active_proposal = key(172);
    assert_eq!(
        reject_locked_reciprocal_rollback(&rollback, &config, &gate),
        Ok(())
    );
    gate.active_proposal = primary_key;
    config.target_nonce = rollback.target_nonce;
    assert_eq!(
        reject_locked_reciprocal_rollback(&rollback, &config, &gate),
        Ok(())
    );
}

#[test]
fn guardian_cannot_authorize_any_nonfreeze_seat_path() {
    let config = timing_config();
    let council = active_council();
    for path in [
        "proposal creation",
        "proposal approval",
        "proposal cancellation",
        "emergency resolution approval",
    ] {
        assert_eq!(
            reject_guardian_authority(&config, &config.guardian),
            Err(ProgramError::Custom(
                GovernanceError::UnknownSeatAuthority as u32
            )),
            "guardian unexpectedly admitted for {path}"
        );
    }
    assert_eq!(
        require_active_seat(&council, &config.guardian, 10),
        Err(ProgramError::Custom(
            GovernanceError::UnknownSeatAuthority as u32
        ))
    );
    for seat in &council.seats {
        assert_eq!(
            reject_guardian_authority(&config, &seat.seat_authority),
            Ok(())
        );
        assert_eq!(
            require_active_seat(&council, &seat.seat_authority, 10),
            Ok(())
        );
    }
}

#[test]
fn runtime_capture_preserves_malformed_programdata_and_hashes_when_bounded() {
    let program_key = key(10);
    let programdata_key = key(11);
    let owner = key(12);
    let mut program_lamports = 1;
    let mut programdata_lamports = 1;
    let mut malformed_program = vec![9; 17];
    let mut malformed_programdata = vec![7; 19];
    let program = AccountInfo::new(
        &program_key,
        false,
        false,
        &mut program_lamports,
        &mut malformed_program,
        &owner,
        false,
        0,
    );
    let programdata = AccountInfo::new(
        &programdata_key,
        false,
        false,
        &mut programdata_lamports,
        &mut malformed_programdata,
        &owner,
        true,
        0,
    );
    let observed = capture_runtime_observation(&program, &programdata).unwrap();
    assert!(!observed.program_header_present);
    assert!(!observed.programdata_header_present);
    assert!(observed.raw_hash_complete);
    assert_eq!(observed.raw_programdata_hash, hashv(&[&[7; 19]]).to_bytes());
    assert_eq!(observed.authority, OptionalPubkeyV1::none());
}

#[test]
fn oversized_runtime_capture_is_persistable_without_one_shot_hashing() {
    let program_key = key(13);
    let programdata_key = key(14);
    let owner = key(15);
    let mut program_lamports = 1;
    let mut programdata_lamports = 1;
    let mut malformed_program = vec![9; 17];
    let mut oversized = vec![0; MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 as usize + 1];
    let program = AccountInfo::new(
        &program_key,
        false,
        false,
        &mut program_lamports,
        &mut malformed_program,
        &owner,
        false,
        0,
    );
    let programdata = AccountInfo::new(
        &programdata_key,
        false,
        false,
        &mut programdata_lamports,
        &mut oversized,
        &owner,
        false,
        0,
    );
    let observed = capture_runtime_observation(&program, &programdata).unwrap();
    assert!(!observed.raw_hash_complete);
    assert_eq!(observed.raw_programdata_hash, [0; 32]);
}

#[test]
fn all_five_seat_masks_enforce_exact_three_of_five_and_duplicates_fail() {
    let council = active_council();
    for mask in 0u8..=VALID_APPROVAL_MASK {
        let count = mask.count_ones() as u8;
        assert_eq!(
            require_exact_recorded_quorum(&council, mask, count, 10).is_ok(),
            count == RELEASE1_APPROVAL_THRESHOLD,
            "mask {mask:05b}"
        );
    }
    assert_eq!(
        record_seat_approval(&council, 0b00001, 1, &council.seats[0].seat_authority, 10,),
        Err(GovernanceError::DuplicateApproval)
    );
    assert_eq!(
        require_exact_recorded_quorum(&council, 0b100000, 1, 10),
        Err(ProgramError::Custom(
            GovernanceError::QuorumNotSatisfied as u32
        ))
    );
}

#[test]
fn account_count_privilege_and_alias_failures_leave_writable_bytes_unchanged() {
    let instruction = QueueProposalV2 {
        expected: proposal_expectation(),
    };
    assert_eq!(
        process_queue_proposal_v2(&key(60), &[], instruction.clone()),
        Err(ProgramError::Custom(
            GovernanceError::InvalidAccountCount as u32
        ))
    );

    let owner = key(61);
    let config_key = key(62);
    let policy_key = key(63);
    let shared_key = key(64);
    let mut config_lamports = 1;
    let mut policy_lamports = 1;
    let mut gate_lamports = 1;
    let mut proposal_lamports = 1;
    let mut config_data = [1u8; 8];
    let mut policy_data = [2u8; 8];
    let mut gate_data = [3u8; 8];
    let mut proposal_data = [4u8; 8];
    let before = proposal_data;
    let config = AccountInfo::new(
        &config_key,
        false,
        false,
        &mut config_lamports,
        &mut config_data,
        &owner,
        false,
        0,
    );
    let policy = AccountInfo::new(
        &policy_key,
        false,
        false,
        &mut policy_lamports,
        &mut policy_data,
        &owner,
        false,
        0,
    );
    let gate = AccountInfo::new(
        &shared_key,
        false,
        false,
        &mut gate_lamports,
        &mut gate_data,
        &owner,
        false,
        0,
    );
    let proposal = AccountInfo::new(
        &shared_key,
        false,
        true,
        &mut proposal_lamports,
        &mut proposal_data,
        &owner,
        false,
        0,
    );
    assert_eq!(
        process_queue_proposal_v2(&key(60), &[config, policy, gate, proposal], instruction,),
        Err(ProgramError::Custom(
            GovernanceError::CrossAccountMismatch as u32
        ))
    );
    assert_eq!(proposal_data, before);

    let config_key = key(65);
    let policy_key = key(66);
    let gate_key = key(67);
    let proposal_key = key(68);
    let mut config_lamports = 1;
    let mut policy_lamports = 1;
    let mut gate_lamports = 1;
    let mut proposal_lamports = 1;
    let mut config_data = [5u8; 8];
    let mut policy_data = [6u8; 8];
    let mut gate_data = [7u8; 8];
    let mut proposal_data = [8u8; 8];
    let before = proposal_data;
    let config = AccountInfo::new(
        &config_key,
        false,
        false,
        &mut config_lamports,
        &mut config_data,
        &owner,
        false,
        0,
    );
    let policy = AccountInfo::new(
        &policy_key,
        false,
        false,
        &mut policy_lamports,
        &mut policy_data,
        &owner,
        false,
        0,
    );
    let gate = AccountInfo::new(
        &gate_key,
        false,
        false,
        &mut gate_lamports,
        &mut gate_data,
        &owner,
        false,
        0,
    );
    let proposal = AccountInfo::new(
        &proposal_key,
        false,
        false,
        &mut proposal_lamports,
        &mut proposal_data,
        &owner,
        false,
        0,
    );
    assert_eq!(
        process_queue_proposal_v2(
            &key(60),
            &[config, policy, gate, proposal],
            QueueProposalV2 {
                expected: proposal_expectation(),
            },
        ),
        Err(ProgramError::Custom(
            GovernanceError::InvalidAccountPrivileges as u32
        ))
    );
    assert_eq!(proposal_data, before);
}

#[test]
fn canonical_runtime_must_match_every_freeze_observation_field() {
    let config = timing_config();
    let runtime = RuntimeObservationV1 {
        program_owner: UPGRADEABLE_LOADER_ID,
        program_executable: true,
        program_data_length: 36,
        program_header_present: true,
        linked_programdata: OptionalPubkeyV1::some(config.target_programdata).unwrap(),
        programdata_owner: UPGRADEABLE_LOADER_ID,
        programdata_executable: false,
        programdata_data_length: 55,
        programdata_header_present: true,
        programdata_slot: 9,
        raw_hash_complete: true,
        raw_programdata_hash: [8; 32],
        capacity: 10,
        authority: OptionalPubkeyV1::some(config.authority_pda).unwrap(),
    };
    let frozen = EmergencyFreezeObservationV1 {
        discriminator: EMERGENCY_FREEZE_OBSERVATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: 1,
        initialized: true,
        finalized: true,
        controller_program: key(20),
        controller_config: key(21),
        protocol_gate: key(22),
        target_program: config.target_program,
        target_programdata: config.target_programdata,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        controller_authority: config.authority_pda,
        frozen_epoch: 2,
        freeze_slot: 3,
        freeze_reason_code: 9,
        actual_program_owner: runtime.program_owner,
        actual_program_executable: runtime.program_executable,
        actual_program_data_length: runtime.program_data_length,
        program_header_present: runtime.program_header_present,
        actual_linked_programdata: runtime.linked_programdata,
        actual_programdata_owner: runtime.programdata_owner,
        actual_programdata_executable: runtime.programdata_executable,
        actual_programdata_data_length: runtime.programdata_data_length,
        programdata_header_present: runtime.programdata_header_present,
        deployed_programdata_slot: runtime.programdata_slot,
        raw_hash_complete: runtime.raw_hash_complete,
        raw_programdata_sha256: runtime.raw_programdata_hash,
        capacity: runtime.capacity,
        observed_authority: runtime.authority,
        observation_digest: [7; 32],
        finalized_slot: 3,
        reserved: [0; EMERGENCY_FREEZE_OBSERVATION_V1_RESERVED_LEN],
    };
    assert_eq!(
        require_canonical_unchanged_runtime(&runtime, &frozen, &config),
        Ok(())
    );
    let mut drifted = runtime.clone();
    drifted.raw_programdata_hash[0] ^= 1;
    assert_eq!(
        require_canonical_unchanged_runtime(&drifted, &frozen, &config),
        Err(ProgramError::Custom(
            GovernanceError::CrossAccountMismatch as u32
        ))
    );
}

#[test]
fn prefreeze_set_is_closed_and_excludes_every_postfreeze_terminal() {
    for state in [
        ProposalStateV2::Draft,
        ProposalStateV2::BufferAdopted,
        ProposalStateV2::BufferVerified,
        ProposalStateV2::CouncilApproved,
        ProposalStateV2::GovernanceSatisfied,
        ProposalStateV2::Timelocked,
    ] {
        assert!(is_pre_freeze_state(state));
    }
    for state in [
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
        ProposalStateV2::TokenReviewOpen,
    ] {
        assert!(!is_pre_freeze_state(state));
    }
}
