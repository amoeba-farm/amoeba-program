use std::{fmt::Debug, fs, path::PathBuf};

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{hash::hashv, pubkey::Pubkey};

use crate::{
    artifact_merkle::{
        artifact_chunk_count, artifact_chunk_empty_hash, artifact_chunk_leaf_hash,
        artifact_chunk_node_hash, artifact_merkle_proof, artifact_merkle_root,
        verify_artifact_chunk_proof, ARTIFACT_CHUNK_SIZE_16_KIB, ARTIFACT_CHUNK_SIZE_4_KIB,
        ARTIFACT_CHUNK_SIZE_8_KIB, ARTIFACT_MERKLE_SCHEME_ID, ARTIFACT_MERKLE_SCHEME_MATERIAL_V1,
        BENCHMARK_ARTIFACT_CHUNK_SIZE_CANDIDATES_V1, MAX_ARTIFACT_BYTES_V1, MAX_ARTIFACT_CHUNKS_V1,
        MAX_ARTIFACT_PROOF_DEPTH_V1, MAX_PADDED_ARTIFACT_CHUNKS_V1,
        MAX_SELECTED_ARTIFACT_CHUNKS_V1, RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
    },
    digest::{
        PROPOSAL_DIGEST_DOMAIN_V1, PROPOSAL_DIGEST_MATERIAL_LEN, PROPOSAL_DIGEST_PREIMAGE_LEN,
    },
    pda::{
        derive_authority_pda, derive_buffer_check_pda, derive_checkpoint_attestation_pda,
        derive_checkpoint_pda, derive_controller_config_pda, derive_council_pda,
        derive_council_rotation_pda, derive_emergency_checkpoint_pda,
        derive_emergency_freeze_observation_pda, derive_emergency_resolution_pda, derive_gate_pda,
        derive_policy_pda, derive_programdata_check_pda,
        derive_programdata_failure_observation_pda, derive_proposal_pda,
        derive_upgradeable_programdata_address, UPGRADEABLE_LOADER_ID,
    },
    release1_digest::{
        canonical_checkpoint_attestation_digest_material_v1,
        canonical_council_rotation_digest_material_v1,
        canonical_emergency_freeze_observation_digest_material_v1,
        canonical_emergency_resolution_digest_material_v1,
        canonical_programdata_failure_observation_digest_material_v1,
        canonical_proposal_digest_material_v2, canonical_state_checkpoint_digest_material_v1,
        canonical_state_checkpoint_hard_root_material_v1, compute_checkpoint_attestation_digest_v1,
        compute_council_rotation_digest_v1, compute_emergency_freeze_observation_digest_v1,
        compute_emergency_resolution_digest_v1, compute_programdata_failure_observation_digest_v1,
        compute_proposal_digest_v2, compute_state_checkpoint_digest_v1,
        compute_state_checkpoint_hard_combined_root_v1, validate_checkpoint_attestation_digest_v1,
        validate_council_rotation_digest_v1, validate_emergency_freeze_observation_digest_v1,
        validate_emergency_resolution_digest_v1,
        validate_programdata_failure_observation_digest_v1, validate_proposal_digest_v2,
        validate_state_checkpoint_digest_v1, validate_state_checkpoint_hard_combined_root_v1,
        CHECKPOINT_ATTESTATION_DIGEST_DOMAIN_V1, CHECKPOINT_ATTESTATION_DIGEST_MATERIAL_LEN_V1,
        CHECKPOINT_ATTESTATION_DIGEST_PREIMAGE_LEN_V1, COUNCIL_ROTATION_DIGEST_DOMAIN_V1,
        COUNCIL_ROTATION_DIGEST_MATERIAL_LEN_V1, COUNCIL_ROTATION_DIGEST_PREIMAGE_LEN_V1,
        EMERGENCY_FREEZE_OBSERVATION_DIGEST_DOMAIN_V1,
        EMERGENCY_FREEZE_OBSERVATION_DIGEST_MATERIAL_LEN_V1,
        EMERGENCY_FREEZE_OBSERVATION_DIGEST_PREIMAGE_LEN_V1, EMERGENCY_RESOLUTION_DIGEST_DOMAIN_V1,
        EMERGENCY_RESOLUTION_DIGEST_MATERIAL_LEN_V1, EMERGENCY_RESOLUTION_DIGEST_PREIMAGE_LEN_V1,
        PROGRAMDATA_FAILURE_OBSERVATION_DIGEST_DOMAIN_V1,
        PROGRAMDATA_FAILURE_OBSERVATION_DIGEST_MATERIAL_LEN_V1,
        PROGRAMDATA_FAILURE_OBSERVATION_DIGEST_PREIMAGE_LEN_V1, PROPOSAL_DIGEST_DOMAIN_V2,
        PROPOSAL_DIGEST_MATERIAL_LEN_V2, PROPOSAL_DIGEST_PREIMAGE_LEN_V2,
        STATE_CHECKPOINT_DIGEST_DOMAIN_V1, STATE_CHECKPOINT_DIGEST_MATERIAL_LEN_V1,
        STATE_CHECKPOINT_DIGEST_PREIMAGE_LEN_V1, STATE_CHECKPOINT_HARD_ROOT_DOMAIN_V1,
        STATE_CHECKPOINT_HARD_ROOT_MATERIAL_LEN_V1, STATE_CHECKPOINT_HARD_ROOT_PREIMAGE_LEN_V1,
    },
    release1_state::{
        buffer_verification_v1_offset, checkpoint_attestation_v1_offset,
        council_rotation_v1_offset, emergency_freeze_observation_v1_offset,
        emergency_resolution_v1_offset, programdata_failure_observation_v1_offset,
        programdata_verification_v1_offset, state_checkpoint_v1_offset, upgrade_proposal_v2_offset,
        BufferVerificationStatusV1, BufferVerificationV1, CheckpointAttestationV1,
        CouncilRotationProposalV1, CouncilRotationStateV1, EmergencyFreezeObservationV1,
        EmergencyFreezeResolutionKindV1, EmergencyFreezeResolutionStateV1,
        EmergencyFreezeResolutionV1, ProgramDataFailureObservationV1, ProgramDataMismatchClassV1,
        ProgramDataVerificationStatusV1, ProgramDataVerificationV1, ProposalStateV2,
        StateCheckpointPhaseV1, StateCheckpointV1, UpgradeProposalV2,
        BUFFER_VERIFICATION_V1_DISCRIMINATOR, CHECKPOINT_ATTESTATION_V1_DISCRIMINATOR,
        COUNCIL_ROTATION_ACTIVATED_TERMINAL_REASON_V1, COUNCIL_ROTATION_EXPIRED_TERMINAL_REASON_V1,
        COUNCIL_ROTATION_PROPOSAL_V1_DISCRIMINATOR, EMERGENCY_FREEZE_OBSERVATION_V1_DISCRIMINATOR,
        EMERGENCY_FREEZE_RESOLUTION_V1_DISCRIMINATOR,
        EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V1,
        EMERGENCY_RESOLUTION_EXPIRED_TERMINAL_REASON_V1, LOADER_V3_PROGRAMDATA_METADATA_LEN_V1,
        LOADER_V3_PROGRAM_ACCOUNT_LEN_V1, MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
        NO_FAILING_CHUNK_INDEX_V1, PROGRAMDATA_FAILURE_OBSERVATION_V1_DISCRIMINATOR,
        PROGRAMDATA_VERIFICATION_V1_DISCRIMINATOR, PROPOSAL_COMPLETED_TERMINAL_REASON_V1,
        PROPOSAL_EXPIRED_TERMINAL_REASON_V1, PROPOSAL_RETIRED_ROLLBACK_TERMINAL_REASON_V1,
        PROPOSAL_SUPERSEDED_BY_ROLLBACK_TERMINAL_REASON_V1, RELEASE1_ACCOUNT_VERSION_V1,
        STATE_CHECKPOINT_V1_DISCRIMINATOR, UPGRADE_PROPOSAL_V2_DISCRIMINATOR,
        VERIFICATION_BITMAP_BYTES_V1,
    },
    state::{
        CheckpointPhaseV1, GateStatusV1, OptionalPubkeyV1, ProposalClassV1, VoteRequirementV1,
        CONTROLLER_CONFIG_RESERVED_LEN, GOVERNANCE_COUNCIL_RESERVED_LEN,
        GOVERNANCE_POLICY_RESERVED_LEN, PROTOCOL_GATE_RESERVED_LEN, UPGRADE_PROPOSAL_RESERVED_LEN,
    },
    GovernanceError,
};

const GOLDEN_PROPOSAL_DIGEST_V2: [u8; 32] = [
    64, 4, 147, 74, 78, 246, 244, 175, 178, 120, 210, 205, 85, 148, 86, 190, 17, 190, 41, 60, 3,
    65, 209, 205, 170, 138, 111, 73, 200, 221, 49, 123,
];
const GOLDEN_CHECKPOINT_DIGEST_V1: [u8; 32] = [
    222, 65, 28, 3, 118, 189, 183, 23, 213, 136, 184, 147, 187, 88, 107, 130, 85, 35, 86, 130, 0,
    78, 164, 204, 37, 68, 240, 122, 230, 51, 33, 181,
];
const GOLDEN_ROTATION_DIGEST_V1: [u8; 32] = [
    210, 45, 232, 22, 160, 241, 237, 150, 17, 12, 202, 181, 162, 145, 69, 124, 97, 48, 128, 89, 81,
    61, 148, 131, 40, 33, 160, 113, 91, 180, 87, 145,
];
const GOLDEN_EMERGENCY_DIGEST_V1: [u8; 32] = [
    228, 179, 71, 186, 157, 96, 112, 191, 159, 135, 20, 181, 16, 234, 53, 238, 101, 11, 12, 105,
    43, 64, 15, 4, 106, 83, 85, 105, 196, 239, 24, 40,
];
const GOLDEN_FREEZE_OBSERVATION_DIGEST_V1: [u8; 32] = [
    234, 62, 149, 216, 99, 141, 76, 255, 24, 2, 195, 152, 137, 231, 187, 37, 239, 194, 251, 225,
    81, 135, 36, 136, 42, 198, 141, 55, 194, 141, 149, 92,
];
const GOLDEN_PROGRAMDATA_FAILURE_DIGEST_V1: [u8; 32] = [
    65, 179, 128, 233, 74, 31, 100, 223, 237, 67, 80, 250, 88, 86, 209, 127, 138, 112, 29, 153,
    135, 53, 184, 227, 37, 216, 226, 190, 31, 26, 26, 32,
];
// Regenerated from the shared Rust/TypeScript synthetic fixture whenever the
// attestation wire contract changes.
const GOLDEN_CHECKPOINT_ATTESTATION_DIGEST_V1: [u8; 32] = [
    107, 0, 100, 234, 134, 40, 72, 220, 1, 140, 96, 207, 119, 130, 20, 68, 238, 158, 220, 40, 170,
    23, 191, 176, 53, 69, 193, 94, 210, 156, 117, 103,
];
const GOLDEN_CHECKPOINT_HARD_ROOT_V1: [u8; 32] = [
    251, 77, 41, 90, 30, 197, 181, 182, 146, 170, 139, 131, 131, 136, 241, 37, 219, 2, 161, 172, 5,
    241, 124, 191, 39, 85, 43, 166, 243, 205, 136, 208,
];
const GOLDEN_ARTIFACT_MERKLE_ROOT_V1: [u8; 32] = [
    154, 204, 2, 215, 73, 144, 43, 58, 19, 51, 111, 162, 120, 187, 169, 160, 104, 96, 135, 195,
    230, 106, 224, 42, 172, 8, 222, 123, 30, 10, 171, 170,
];

fn key(value: u8) -> Pubkey {
    Pubkey::new_from_array([value; 32])
}

fn synthetic_controller_program() -> Pubkey {
    key(1)
}

fn ameba_spread_program() -> Pubkey {
    solana_program::pubkey!("9ipkBCjEfeJDMF6AFrezRmDDHmbnmeyv45cfXNqAnWsH")
}

fn release1_proposal_key() -> Pubkey {
    derive_proposal_pda(&synthetic_controller_program(), &ameba_spread_program(), 7).0
}

fn bytes(value: u8) -> [u8; 32] {
    [value; 32]
}

fn artifact(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index % 251) as u8).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn release1_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("upgrade_governance_release1.json")
}

fn phase3_bridge_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("spread_gate_bridge_v1.json")
}

fn complete_bitmap(count: u32) -> [u8; VERIFICATION_BITMAP_BYTES_V1] {
    let mut bitmap = [0u8; VERIFICATION_BITMAP_BYTES_V1];
    for index in 0..count {
        bitmap[(index / 8) as usize] |= 1 << (index % 8);
    }
    bitmap
}

fn sample_proposal(chunk_size: u32) -> Box<UpgradeProposalV2> {
    let artifact = artifact(5_000);
    let chunk_count = artifact_chunk_count(artifact.len() as u64, chunk_size).unwrap();
    let controller_program = synthetic_controller_program();
    let target_program = ameba_spread_program();
    let target_programdata = derive_upgradeable_programdata_address(&target_program).0;
    let authority_pda = derive_authority_pda(&controller_program, &target_program).0;
    let proposal_key = derive_proposal_pda(&controller_program, &target_program, 7).0;
    let mut proposal = Box::new(UpgradeProposalV2 {
        discriminator: UPGRADE_PROPOSAL_V2_DISCRIMINATOR,
        account_version: 2,
        bump: derive_proposal_pda(&controller_program, &target_program, 7).1,
        initialized: true,
        proposal_class: ProposalClassV1::RoutineUpgrade,
        state: ProposalStateV2::Draft,
        creation_gate_status: GateStatusV1::Active,
        zero_tail_required: true,
        proposal_flags: 0,
        proposal_id: 7,
        target_nonce: 11,
        creation_slot: 10,
        cluster_domain: bytes(1),
        controller_program,
        controller_config: derive_controller_config_pda(&controller_program, &target_program).0,
        protocol_gate: derive_gate_pda(&controller_program, &target_program).0,
        policy_version: 1,
        policy_hash: bytes(5),
        creation_council_version: 1,
        creation_council_hash: bytes(6),
        creation_gate_epoch: 9,
        freeze_gate_epoch: 0,
        target_program,
        target_programdata,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        authority_pda,
        canonical_spill_treasury: key(11),
        buffer_pubkey: key(12),
        buffer_loader_owner: UPGRADEABLE_LOADER_ID,
        buffer_uploader_authority: key(13),
        buffer_final_authority: authority_pda,
        buffer_verification: derive_buffer_check_pda(&controller_program, &proposal_key).0,
        programdata_verification: derive_programdata_check_pda(&controller_program, &proposal_key)
            .0,
        artifact_length: artifact.len() as u64,
        artifact_sha256: hashv(&[&artifact]).to_bytes(),
        artifact_chunk_merkle_root: artifact_merkle_root(&artifact, chunk_size).unwrap(),
        chunk_hash_domain: ARTIFACT_MERKLE_SCHEME_ID,
        chunk_size,
        chunk_count,
        source_commit_hash: bytes(16),
        source_tree_hash: bytes(17),
        build_input_inventory_hash: bytes(18),
        reproducible_build_receipt_hash: bytes(19),
        package_receipt_hash: bytes(20),
        release_intent_hash: bytes(21),
        expected_execution_pre_payload_hash: bytes(22),
        expected_execution_pre_chunk_root: bytes(23),
        current_raw_programdata_hash: bytes(24),
        deployed_slot: 2,
        current_capacity: 4_096,
        extension_delta: 4_096,
        expected_post_capacity: 8_192,
        prestate_checkpoint: derive_checkpoint_pda(
            &controller_program,
            &proposal_key,
            CheckpointPhaseV1::Prestate,
        )
        .0,
        required_poststate_checkpoint: derive_checkpoint_pda(
            &controller_program,
            &proposal_key,
            CheckpointPhaseV1::Poststate,
        )
        .0,
        checkpoint_schema_id: bytes(27),
        checkpoint_policy_hash: bytes(28),
        primary_proposal: OptionalPubkeyV1::none(),
        rollback_proposal: OptionalPubkeyV1::some(key(29)).unwrap(),
        rollback_buffer: OptionalPubkeyV1::some(key(30)).unwrap(),
        rollback_artifact_sha256: bytes(31),
        rollback_artifact_chunk_root: bytes(32),
        vote_requirement: VoteRequirementV1::None,
        vote_program: Pubkey::default(),
        vote_result_pda: Pubkey::default(),
        review_start_slot: 20,
        review_end_slot: 30,
        not_before_slot: 40,
        expiry_slot: 50,
        first_approval_slot: 0,
        council_approved_slot: 0,
        governance_satisfied_slot: 0,
        queued_slot: 0,
        frozen_slot: 0,
        extension_executed_slot: 0,
        upgrade_executed_slot: 0,
        programdata_verified_slot: 0,
        poststate_accepted_slot: 0,
        unfreeze_approved_slot: 0,
        terminal_slot: 0,
        council_approval_bitset: 0,
        council_approval_count: 0,
        cancellation_council_version: 0,
        cancellation_council_hash: [0; 32],
        cancellation_approval_bitset: 0,
        cancellation_approval_count: 0,
        unfreeze_council_version: 0,
        unfreeze_council_hash: [0; 32],
        unfreeze_approval_bitset: 0,
        unfreeze_approval_count: 0,
        proposal_digest: [0; 32],
        cancellation_reason_code: 0,
        terminal_reason_code: 0,
        reserved: [0; 146],
    });
    proposal.proposal_digest = compute_proposal_digest_v2(&proposal).unwrap();
    proposal
}

fn canonical_proposal_in_state(state: ProposalStateV2) -> Box<UpgradeProposalV2> {
    let mut proposal = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1);
    proposal.state = state;

    let council_approved = matches!(
        state,
        ProposalStateV2::CouncilApproved
            | ProposalStateV2::GovernanceSatisfied
            | ProposalStateV2::Timelocked
            | ProposalStateV2::Frozen
            | ProposalStateV2::Extended
            | ProposalStateV2::UpgradeExecuted
            | ProposalStateV2::ProgramDataVerified
            | ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
            | ProposalStateV2::Completed
            | ProposalStateV2::SupersededByRollback
            | ProposalStateV2::Retired
    );
    if council_approved {
        proposal.council_approval_bitset = 0b0_0111;
        proposal.council_approval_count = 3;
        proposal.first_approval_slot = 20;
        proposal.council_approved_slot = 20;
    }

    let governance_satisfied = matches!(
        state,
        ProposalStateV2::GovernanceSatisfied
            | ProposalStateV2::Timelocked
            | ProposalStateV2::Frozen
            | ProposalStateV2::Extended
            | ProposalStateV2::UpgradeExecuted
            | ProposalStateV2::ProgramDataVerified
            | ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
            | ProposalStateV2::Completed
            | ProposalStateV2::SupersededByRollback
            | ProposalStateV2::Retired
    );
    if governance_satisfied {
        proposal.governance_satisfied_slot = 21;
    }
    let queued = matches!(
        state,
        ProposalStateV2::Timelocked
            | ProposalStateV2::Frozen
            | ProposalStateV2::Extended
            | ProposalStateV2::UpgradeExecuted
            | ProposalStateV2::ProgramDataVerified
            | ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
            | ProposalStateV2::Completed
            | ProposalStateV2::SupersededByRollback
            | ProposalStateV2::Retired
    );
    if queued {
        proposal.queued_slot = 22;
    }
    let frozen = matches!(
        state,
        ProposalStateV2::Frozen
            | ProposalStateV2::Extended
            | ProposalStateV2::UpgradeExecuted
            | ProposalStateV2::ProgramDataVerified
            | ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
            | ProposalStateV2::Completed
            | ProposalStateV2::SupersededByRollback
    );
    if frozen {
        proposal.freeze_gate_epoch = 10;
        proposal.frozen_slot = 40;
    }
    let extended = matches!(
        state,
        ProposalStateV2::Extended
            | ProposalStateV2::UpgradeExecuted
            | ProposalStateV2::ProgramDataVerified
            | ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
            | ProposalStateV2::Completed
            | ProposalStateV2::SupersededByRollback
    );
    if extended {
        proposal.extension_executed_slot = 41;
    }
    let upgraded = matches!(
        state,
        ProposalStateV2::UpgradeExecuted
            | ProposalStateV2::ProgramDataVerified
            | ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
            | ProposalStateV2::Completed
            | ProposalStateV2::SupersededByRollback
    );
    if upgraded {
        proposal.upgrade_executed_slot = 42;
    }
    let programdata_verified = matches!(
        state,
        ProposalStateV2::ProgramDataVerified
            | ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
            | ProposalStateV2::Completed
            | ProposalStateV2::SupersededByRollback
    );
    if programdata_verified {
        proposal.programdata_verified_slot = 43;
    }
    let poststate_accepted = matches!(
        state,
        ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
            | ProposalStateV2::Completed
    );
    if poststate_accepted {
        proposal.poststate_accepted_slot = 44;
    }
    if matches!(
        state,
        ProposalStateV2::UnfreezeApproved | ProposalStateV2::Completed
    ) {
        proposal.unfreeze_council_version = 1;
        proposal.unfreeze_council_hash = bytes(96);
        proposal.unfreeze_approval_bitset = 0b0_0111;
        proposal.unfreeze_approval_count = 3;
        proposal.unfreeze_approved_slot = 44;
    }

    match state {
        ProposalStateV2::Completed => {
            proposal.terminal_slot = 45;
            proposal.terminal_reason_code = PROPOSAL_COMPLETED_TERMINAL_REASON_V1;
        }
        ProposalStateV2::Cancelled => {
            proposal.cancellation_council_version = 1;
            proposal.cancellation_council_hash = bytes(97);
            proposal.cancellation_approval_bitset = 0b0_0111;
            proposal.cancellation_approval_count = 3;
            proposal.cancellation_reason_code = 77;
            proposal.terminal_slot = 20;
            proposal.terminal_reason_code = 77;
        }
        ProposalStateV2::Expired => {
            proposal.terminal_slot = proposal.expiry_slot;
            proposal.terminal_reason_code = PROPOSAL_EXPIRED_TERMINAL_REASON_V1;
        }
        ProposalStateV2::SupersededByRollback => {
            proposal.terminal_slot = 45;
            proposal.terminal_reason_code = PROPOSAL_SUPERSEDED_BY_ROLLBACK_TERMINAL_REASON_V1;
        }
        ProposalStateV2::Retired => {
            proposal.proposal_class = ProposalClassV1::EmergencyRollback;
            proposal.primary_proposal = OptionalPubkeyV1::some(key(93)).unwrap();
            proposal.rollback_proposal = OptionalPubkeyV1::none();
            proposal.rollback_buffer = OptionalPubkeyV1::none();
            proposal.rollback_artifact_sha256 = [0; 32];
            proposal.rollback_artifact_chunk_root = [0; 32];
            proposal.deployed_slot = 0;
            proposal.current_raw_programdata_hash = [0; 32];
            proposal.terminal_slot = 45;
            proposal.terminal_reason_code = PROPOSAL_RETIRED_ROLLBACK_TERMINAL_REASON_V1;
            proposal.proposal_digest = compute_proposal_digest_v2(&proposal).unwrap();
        }
        _ => {}
    }
    proposal
}

fn sample_buffer_verification() -> Box<BufferVerificationV1> {
    let proposal = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1);
    Box::new(BufferVerificationV1 {
        discriminator: BUFFER_VERIFICATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_buffer_check_pda(&proposal.controller_program, &release1_proposal_key()).1,
        initialized: true,
        status: BufferVerificationStatusV1::Verified,
        controller_config: proposal.controller_config,
        proposal: release1_proposal_key(),
        upgradeable_loader: proposal.upgradeable_loader,
        buffer: proposal.buffer_pubkey,
        expected_uploader_authority: proposal.buffer_uploader_authority,
        controller_authority: proposal.authority_pda,
        artifact_length: proposal.artifact_length,
        artifact_sha256: proposal.artifact_sha256,
        artifact_chunk_merkle_root: proposal.artifact_chunk_merkle_root,
        chunk_hash_domain: proposal.chunk_hash_domain,
        chunk_size: proposal.chunk_size,
        chunk_count: proposal.chunk_count,
        verified_chunk_bitmap: complete_bitmap(proposal.chunk_count),
        verified_chunk_count: proposal.chunk_count,
        adopted_slot: 21,
        finalized_slot: 22,
        sealed_buffer_header_hash: bytes(34),
        terminal_slot: 0,
        reserved: [0; 72],
    })
}

fn sample_programdata_verification() -> Box<ProgramDataVerificationV1> {
    let proposal = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1);
    let tail_length = proposal.expected_post_capacity - proposal.artifact_length;
    let tail_chunk_count = artifact_chunk_count(tail_length, proposal.chunk_size).unwrap();
    Box::new(ProgramDataVerificationV1 {
        discriminator: PROGRAMDATA_VERIFICATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_programdata_check_pda(&proposal.controller_program, &release1_proposal_key())
            .1,
        initialized: true,
        status: ProgramDataVerificationStatusV1::Verified,
        controller_config: proposal.controller_config,
        proposal: release1_proposal_key(),
        target_program: proposal.target_program,
        target_programdata: proposal.target_programdata,
        upgradeable_loader: proposal.upgradeable_loader,
        controller_authority: proposal.authority_pda,
        artifact_length: proposal.artifact_length,
        artifact_sha256: proposal.artifact_sha256,
        artifact_chunk_merkle_root: proposal.artifact_chunk_merkle_root,
        chunk_hash_domain: proposal.chunk_hash_domain,
        chunk_size: proposal.chunk_size,
        payload_chunk_count: proposal.chunk_count,
        verified_payload_chunk_bitmap: complete_bitmap(proposal.chunk_count),
        verified_payload_chunk_count: proposal.chunk_count,
        deployed_slot: 88,
        capacity: proposal.expected_post_capacity,
        tail_length,
        tail_chunk_count,
        verified_tail_chunk_bitmap: complete_bitmap(tail_chunk_count),
        verified_tail_chunk_count: tail_chunk_count,
        raw_programdata_hash: bytes(35),
        zero_tail_verified: true,
        finalized_slot: 89,
        reserved: [0; 119],
    })
}

fn sample_checkpoint() -> Box<StateCheckpointV1> {
    let controller_program = synthetic_controller_program();
    let target_program = ameba_spread_program();
    let mut checkpoint = Box::new(StateCheckpointV1 {
        discriminator: STATE_CHECKPOINT_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_checkpoint_pda(
            &controller_program,
            &release1_proposal_key(),
            CheckpointPhaseV1::Prestate,
        )
        .1,
        initialized: true,
        phase: StateCheckpointPhaseV1::Prestate,
        controller_config: derive_controller_config_pda(&controller_program, &target_program).0,
        proposal: release1_proposal_key(),
        emergency_resolution: Pubkey::default(),
        subject_digest: bytes(36),
        target_program,
        target_programdata: derive_upgradeable_programdata_address(&target_program).0,
        finalized_observation_slot: 90,
        gate_epoch: 10,
        target_programdata_slot: 2,
        target_payload_commitment: bytes(37),
        target_raw_programdata_commitment: bytes(38),
        target_capacity: 4_096,
        program_owned_state_root: bytes(39),
        program_owned_state_count: 17,
        logical_compressed_state_root: bytes(40),
        logical_compressed_state_count: 19,
        semantic_custody_accounting_root: bytes(41),
        hard_combined_root: [0; 32],
        external_metadata_observation_root: bytes(43),
        external_raw_balance_observation_root: bytes(44),
        schema_identifier: bytes(45),
        admitted_positive_donation_root: bytes(46),
        admitted_positive_donation_count: 1,
        forbidden_drift_count: 0,
        approval_council_version: 2,
        approval_council_hash: bytes(47),
        checkpoint_digest: [0; 32],
        approval_bitset: 0b0_0111,
        approval_count: 3,
        accepted: true,
        finalized_slot: 91,
        reserved: [0; 37],
    });
    checkpoint.hard_combined_root =
        compute_state_checkpoint_hard_combined_root_v1(&checkpoint).unwrap();
    checkpoint.checkpoint_digest = compute_state_checkpoint_digest_v1(&checkpoint).unwrap();
    checkpoint
}

fn sample_rotation() -> Box<CouncilRotationProposalV1> {
    let controller_program = synthetic_controller_program();
    let target_program = ameba_spread_program();
    let mut rotation = Box::new(CouncilRotationProposalV1 {
        discriminator: COUNCIL_ROTATION_PROPOSAL_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_council_rotation_pda(&controller_program, &target_program, 3).1,
        initialized: true,
        state: CouncilRotationStateV1::Timelocked,
        controller_config: derive_controller_config_pda(&controller_program, &target_program).0,
        target_program,
        current_council: derive_council_pda(&controller_program, &target_program, 2).0,
        current_council_version: 2,
        current_council_hash: bytes(49),
        candidate_council: derive_council_pda(&controller_program, &target_program, 3).0,
        candidate_council_version: 3,
        candidate_council_hash: bytes(51),
        creation_slot: 100,
        not_before_slot: 120,
        expiry_slot: 140,
        target_nonce: 11,
        approval_bitset: 0b0_0111,
        approval_count: 3,
        cancellation_approval_bitset: 0,
        cancellation_approval_count: 0,
        rotation_digest: [0; 32],
        activated_slot: 0,
        cancellation_reason_code: 0,
        terminal_reason_code: 0,
        reserved: [0; 84],
    });
    rotation.rotation_digest = compute_council_rotation_digest_v1(&rotation).unwrap();
    rotation
}

fn sample_emergency_resolution() -> Box<EmergencyFreezeResolutionV1> {
    let controller_program = synthetic_controller_program();
    let target_program = ameba_spread_program();
    let mut resolution = Box::new(EmergencyFreezeResolutionV1 {
        discriminator: EMERGENCY_FREEZE_RESOLUTION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_emergency_resolution_pda(&controller_program, &target_program, 12).1,
        initialized: true,
        state: EmergencyFreezeResolutionStateV1::Timelocked,
        controller_config: derive_controller_config_pda(&controller_program, &target_program).0,
        protocol_gate: derive_gate_pda(&controller_program, &target_program).0,
        target_program,
        target_programdata: derive_upgradeable_programdata_address(&target_program).0,
        emergency_freeze_observation: derive_emergency_freeze_observation_pda(
            &controller_program,
            &target_program,
            12,
        )
        .0,
        frozen_epoch: 12,
        freeze_slot: 150,
        freeze_reason_code: 2,
        resolution_kind: EmergencyFreezeResolutionKindV1::ResumeWithoutUpgrade,
        creation_slot: 151,
        not_before_slot: 170,
        expiry_slot: 190,
        target_nonce: 11,
        observed_program_owner: UPGRADEABLE_LOADER_ID,
        observed_program_executable: true,
        observed_program_data_length: 36,
        observed_program_header_present: true,
        observed_linked_programdata: OptionalPubkeyV1::some(
            derive_upgradeable_programdata_address(&target_program).0,
        )
        .unwrap(),
        observed_programdata_owner: UPGRADEABLE_LOADER_ID,
        observed_programdata_executable: false,
        observed_programdata_data_length: 4_141,
        observed_programdata_header_present: true,
        observed_programdata_slot: 2,
        observed_raw_hash_complete: true,
        observed_raw_programdata_hash: bytes(53),
        observed_capacity: 4_096,
        observed_authority: OptionalPubkeyV1::some(
            derive_authority_pda(&controller_program, &target_program).0,
        )
        .unwrap(),
        emergency_checkpoint: derive_emergency_checkpoint_pda(
            &controller_program,
            &target_program,
            12,
        )
        .0,
        approval_council_version: 2,
        approval_council_hash: bytes(55),
        approval_bitset: 0b0_0111,
        approval_count: 3,
        resolution_digest: [0; 32],
        executed_slot: 0,
        cancellation_reason_code: 0,
        terminal_reason_code: 0,
        reserved: [0; 100],
    });
    resolution.resolution_digest = compute_emergency_resolution_digest_v1(&resolution).unwrap();
    resolution
}

fn sample_emergency_freeze_observation() -> Box<EmergencyFreezeObservationV1> {
    let controller_program = synthetic_controller_program();
    let target_program = ameba_spread_program();
    let controller_authority = derive_authority_pda(&controller_program, &target_program).0;
    let mut observation = Box::new(EmergencyFreezeObservationV1 {
        discriminator: EMERGENCY_FREEZE_OBSERVATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_emergency_freeze_observation_pda(&controller_program, &target_program, 12).1,
        initialized: true,
        finalized: true,
        controller_program,
        controller_config: derive_controller_config_pda(&controller_program, &target_program).0,
        protocol_gate: derive_gate_pda(&controller_program, &target_program).0,
        target_program,
        target_programdata: derive_upgradeable_programdata_address(&target_program).0,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        controller_authority,
        frozen_epoch: 12,
        freeze_slot: 150,
        freeze_reason_code: 2,
        actual_program_owner: UPGRADEABLE_LOADER_ID,
        actual_program_executable: true,
        actual_program_data_length: 36,
        program_header_present: true,
        actual_linked_programdata: OptionalPubkeyV1::some(
            derive_upgradeable_programdata_address(&target_program).0,
        )
        .unwrap(),
        actual_programdata_owner: UPGRADEABLE_LOADER_ID,
        actual_programdata_executable: false,
        actual_programdata_data_length: 4_141,
        programdata_header_present: true,
        deployed_programdata_slot: 2,
        raw_hash_complete: true,
        raw_programdata_sha256: bytes(56),
        capacity: 4_096,
        observed_authority: OptionalPubkeyV1::some(controller_authority).unwrap(),
        observation_digest: [0; 32],
        finalized_slot: 150,
        reserved: [0; 19],
    });
    observation.observation_digest =
        compute_emergency_freeze_observation_digest_v1(&observation).unwrap();
    observation
}

fn sample_programdata_failure_observation() -> Box<ProgramDataFailureObservationV1> {
    let controller_program = synthetic_controller_program();
    let target_program = ameba_spread_program();
    let mut observation = Box::new(ProgramDataFailureObservationV1 {
        discriminator: PROGRAMDATA_FAILURE_OBSERVATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_programdata_failure_observation_pda(
            &controller_program,
            &release1_proposal_key(),
            13,
        )
        .1,
        initialized: true,
        finalized: true,
        controller_config: derive_controller_config_pda(&controller_program, &target_program).0,
        protocol_gate: derive_gate_pda(&controller_program, &target_program).0,
        primary_proposal: release1_proposal_key(),
        target_program,
        target_programdata: derive_upgradeable_programdata_address(&target_program).0,
        frozen_epoch: 13,
        actual_program_owner: UPGRADEABLE_LOADER_ID,
        actual_program_executable: true,
        actual_program_data_length: 36,
        program_header_present: true,
        actual_linked_programdata: OptionalPubkeyV1::some(
            derive_upgradeable_programdata_address(&target_program).0,
        )
        .unwrap(),
        raw_hash_complete: true,
        actual_raw_programdata_sha256: bytes(57),
        actual_owner: UPGRADEABLE_LOADER_ID,
        actual_executable: false,
        actual_data_length: 8_237,
        programdata_header_present: true,
        actual_programdata_slot: 88,
        actual_capacity: 8_192,
        actual_authority: OptionalPubkeyV1::some(
            derive_authority_pda(&controller_program, &target_program).0,
        )
        .unwrap(),
        mismatch_class: ProgramDataMismatchClassV1::PayloadLeaf,
        failing_chunk_index: 1,
        expected_leaf_hash: bytes(58),
        actual_leaf_hash: bytes(59),
        finalized_slot: 200,
        observation_digest: [0; 32],
        reserved: [0; 24],
    });
    observation.observation_digest =
        compute_programdata_failure_observation_digest_v1(&observation).unwrap();
    observation
}

fn sample_checkpoint_attestation() -> Box<CheckpointAttestationV1> {
    let controller_program = synthetic_controller_program();
    let target_program = ameba_spread_program();
    let checkpoint = sample_checkpoint();
    let mut attestation = Box::new(CheckpointAttestationV1 {
        discriminator: CHECKPOINT_ATTESTATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_checkpoint_attestation_pda(
            &controller_program,
            &derive_checkpoint_pda(
                &controller_program,
                &release1_proposal_key(),
                CheckpointPhaseV1::Prestate,
            )
            .0,
            2,
            2,
        )
        .1,
        initialized: true,
        controller_program,
        controller_config: checkpoint.controller_config,
        checkpoint: derive_checkpoint_pda(
            &controller_program,
            &release1_proposal_key(),
            CheckpointPhaseV1::Prestate,
        )
        .0,
        subject: checkpoint.proposal,
        subject_digest: checkpoint.subject_digest,
        phase: checkpoint.phase,
        checkpoint_digest: checkpoint.checkpoint_digest,
        council: derive_council_pda(&controller_program, &target_program, 2).0,
        council_version: checkpoint.approval_council_version,
        council_hash: checkpoint.approval_council_hash,
        gate_epoch: checkpoint.gate_epoch,
        seat_index: 2,
        seat_authority: key(62),
        attested_slot: 90,
        attestation_digest: [0; 32],
        reserved: [0; 27],
    });
    attestation.attestation_digest =
        compute_checkpoint_attestation_digest_v1(&attestation).unwrap();
    attestation
}

fn assert_exact_roundtrip<T>(value: &T, expected_len: usize)
where
    T: BorshDeserialize + BorshSerialize + Debug + Eq,
{
    let encoded = value.try_to_vec().unwrap();
    assert_eq!(encoded.len(), expected_len);
    assert_eq!(T::try_from_slice(&encoded).unwrap(), *value);

    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(T::try_from_slice(&trailing).is_err());
    assert!(T::try_from_slice(&encoded[..encoded.len() - 1]).is_err());
}

#[test]
fn release1_layouts_are_exact_fixed_width_and_roundtrip() {
    let proposal = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1);
    let buffer = sample_buffer_verification();
    let programdata = sample_programdata_verification();
    let checkpoint = sample_checkpoint();
    let rotation = sample_rotation();
    let resolution = sample_emergency_resolution();
    let freeze_observation = sample_emergency_freeze_observation();
    let failure_observation = sample_programdata_failure_observation();
    let checkpoint_attestation = sample_checkpoint_attestation();

    assert_exact_roundtrip(&*proposal, UpgradeProposalV2::LEN);
    assert_exact_roundtrip(&*buffer, BufferVerificationV1::LEN);
    assert_exact_roundtrip(&*programdata, ProgramDataVerificationV1::LEN);
    assert_exact_roundtrip(&*checkpoint, StateCheckpointV1::LEN);
    assert_exact_roundtrip(&*rotation, CouncilRotationProposalV1::LEN);
    assert_exact_roundtrip(&*resolution, EmergencyFreezeResolutionV1::LEN);
    assert_exact_roundtrip(&*freeze_observation, EmergencyFreezeObservationV1::LEN);
    assert_exact_roundtrip(&*failure_observation, ProgramDataFailureObservationV1::LEN);
    assert_exact_roundtrip(&*checkpoint_attestation, CheckpointAttestationV1::LEN);

    assert_eq!(
        proposal.try_to_vec().unwrap()[upgrade_proposal_v2_offset::STATE],
        0
    );
    assert_eq!(
        &proposal.try_to_vec().unwrap()[upgrade_proposal_v2_offset::PROPOSAL_DIGEST..][..32],
        &proposal.proposal_digest
    );
    assert_eq!(
        buffer.try_to_vec().unwrap()[buffer_verification_v1_offset::STATUS],
        BufferVerificationStatusV1::Verified as u8
    );
    assert_eq!(
        programdata.try_to_vec().unwrap()[programdata_verification_v1_offset::ZERO_TAIL_VERIFIED],
        1
    );
    assert_eq!(
        checkpoint.try_to_vec().unwrap()[state_checkpoint_v1_offset::PHASE],
        StateCheckpointPhaseV1::Prestate as u8
    );
    assert_eq!(
        rotation.try_to_vec().unwrap()[council_rotation_v1_offset::STATE],
        CouncilRotationStateV1::Timelocked as u8
    );
    assert_eq!(
        resolution.try_to_vec().unwrap()[emergency_resolution_v1_offset::RESOLUTION_KIND],
        EmergencyFreezeResolutionKindV1::ResumeWithoutUpgrade as u8
    );
    assert_eq!(
        resolution.try_to_vec().unwrap()[emergency_resolution_v1_offset::PROGRAM_EXECUTABLE],
        1
    );
    assert_eq!(
        resolution.try_to_vec().unwrap()[emergency_resolution_v1_offset::LINKED_PROGRAMDATA],
        1
    );
    assert_eq!(
        resolution.try_to_vec().unwrap()[emergency_resolution_v1_offset::RAW_HASH_COMPLETE],
        1
    );
    assert_eq!(
        &freeze_observation.try_to_vec().unwrap()
            [emergency_freeze_observation_v1_offset::OBSERVATION_DIGEST..][..32],
        &freeze_observation.observation_digest
    );
    assert_eq!(
        freeze_observation.try_to_vec().unwrap()
            [emergency_freeze_observation_v1_offset::PROGRAM_HEADER_PRESENT],
        1
    );
    assert_eq!(
        freeze_observation.try_to_vec().unwrap()
            [emergency_freeze_observation_v1_offset::RAW_HASH_COMPLETE],
        1
    );
    assert_eq!(
        failure_observation.try_to_vec().unwrap()
            [programdata_failure_observation_v1_offset::MISMATCH_CLASS],
        ProgramDataMismatchClassV1::PayloadLeaf as u8
    );
    assert_eq!(
        failure_observation.try_to_vec().unwrap()
            [programdata_failure_observation_v1_offset::PROGRAM_HEADER_PRESENT],
        1
    );
    assert_eq!(
        failure_observation.try_to_vec().unwrap()
            [programdata_failure_observation_v1_offset::RAW_HASH_COMPLETE],
        1
    );
    assert_eq!(
        checkpoint_attestation.try_to_vec().unwrap()[checkpoint_attestation_v1_offset::PHASE],
        StateCheckpointPhaseV1::Prestate as u8
    );
    assert_eq!(
        &checkpoint_attestation.try_to_vec().unwrap()
            [checkpoint_attestation_v1_offset::ATTESTATION_DIGEST..][..32],
        &checkpoint_attestation.attestation_digest
    );
}

#[test]
fn buffer_all_chunks_complete_waits_in_ready_to_finalize() {
    let mut ready = sample_buffer_verification();
    ready.status = BufferVerificationStatusV1::ReadyToFinalize;
    ready.finalized_slot = 0;
    assert_eq!(ready.validate_schema(), Ok(()));

    let mut complete_but_still_verifying = ready.clone();
    complete_but_still_verifying.status = BufferVerificationStatusV1::Verifying;
    assert_eq!(
        complete_but_still_verifying.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut ready_missing_chunk = ready;
    ready_missing_chunk.verified_chunk_bitmap = [0; VERIFICATION_BITMAP_BYTES_V1];
    ready_missing_chunk.verified_chunk_count = 0;
    assert_eq!(
        ready_missing_chunk.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
}

#[test]
fn programdata_raw_hash_is_finalization_evidence_not_caller_seeded_progress() {
    let mut verifying = sample_programdata_verification();
    verifying.status = ProgramDataVerificationStatusV1::Verifying;
    verifying.verified_payload_chunk_bitmap = [0; VERIFICATION_BITMAP_BYTES_V1];
    verifying.verified_payload_chunk_count = 0;
    verifying.raw_programdata_hash = [0; 32];
    verifying.zero_tail_verified = false;
    verifying.finalized_slot = 0;
    assert_eq!(verifying.validate_schema(), Ok(()));

    let mut ready = sample_programdata_verification();
    ready.status = ProgramDataVerificationStatusV1::ReadyToFinalize;
    ready.raw_programdata_hash = [0; 32];
    ready.zero_tail_verified = false;
    ready.finalized_slot = 0;
    assert_eq!(ready.validate_schema(), Ok(()));

    let mut complete_but_still_verifying = ready.clone();
    complete_but_still_verifying.status = ProgramDataVerificationStatusV1::Verifying;
    assert_eq!(
        complete_but_still_verifying.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    verifying.raw_programdata_hash = bytes(99);
    assert_eq!(
        verifying.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut verified = sample_programdata_verification();
    verified.raw_programdata_hash = [0; 32];
    assert_eq!(
        verified.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
}

#[test]
fn release1_enum_discriminants_are_exhaustive_and_unknown_values_fail() {
    let proposal_states = [
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
    for (expected, state) in proposal_states.into_iter().enumerate() {
        assert_eq!(state.try_to_vec().unwrap(), [expected as u8]);
        assert_eq!(
            ProposalStateV2::try_from_slice(&[expected as u8]).unwrap(),
            state
        );
    }
    assert!(ProposalStateV2::try_from_slice(&[18]).is_err());

    let buffer_states = [
        BufferVerificationStatusV1::Adopted,
        BufferVerificationStatusV1::Verifying,
        BufferVerificationStatusV1::ReadyToFinalize,
        BufferVerificationStatusV1::Verified,
        BufferVerificationStatusV1::ConsumedByUpgrade,
        BufferVerificationStatusV1::ClosedAbandoned,
    ];
    for (expected, state) in buffer_states.into_iter().enumerate() {
        assert_eq!(state.try_to_vec().unwrap(), [expected as u8]);
        assert_eq!(
            BufferVerificationStatusV1::try_from_slice(&[expected as u8]).unwrap(),
            state
        );
    }
    assert!(BufferVerificationStatusV1::try_from_slice(&[6]).is_err());

    for (expected, state) in [
        CouncilRotationStateV1::Draft,
        CouncilRotationStateV1::CouncilApproved,
        CouncilRotationStateV1::Timelocked,
        CouncilRotationStateV1::Activated,
        CouncilRotationStateV1::Cancelled,
        CouncilRotationStateV1::Expired,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(state.try_to_vec().unwrap(), [expected as u8]);
    }
    assert!(CouncilRotationStateV1::try_from_slice(&[6]).is_err());

    for (expected, state) in [
        EmergencyFreezeResolutionStateV1::Draft,
        EmergencyFreezeResolutionStateV1::CouncilApproved,
        EmergencyFreezeResolutionStateV1::Timelocked,
        EmergencyFreezeResolutionStateV1::Executed,
        EmergencyFreezeResolutionStateV1::Cancelled,
        EmergencyFreezeResolutionStateV1::Expired,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(state.try_to_vec().unwrap(), [expected as u8]);
    }
    assert!(EmergencyFreezeResolutionStateV1::try_from_slice(&[6]).is_err());
    assert!(StateCheckpointPhaseV1::try_from_slice(&[3]).is_err());
    for (expected, status) in [
        ProgramDataVerificationStatusV1::Verifying,
        ProgramDataVerificationStatusV1::ReadyToFinalize,
        ProgramDataVerificationStatusV1::Verified,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(status.try_to_vec().unwrap(), [expected as u8]);
    }
    assert!(ProgramDataVerificationStatusV1::try_from_slice(&[3]).is_err());
    assert!(EmergencyFreezeResolutionKindV1::try_from_slice(&[1]).is_err());
    for (expected, class) in [
        ProgramDataMismatchClassV1::Header,
        ProgramDataMismatchClassV1::Authority,
        ProgramDataMismatchClassV1::Capacity,
        ProgramDataMismatchClassV1::PayloadLeaf,
        ProgramDataMismatchClassV1::ZeroTail,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(class.try_to_vec().unwrap(), [expected as u8]);
        assert_eq!(
            ProgramDataMismatchClassV1::try_from_slice(&[expected as u8]).unwrap(),
            class
        );
    }
    assert!(ProgramDataMismatchClassV1::try_from_slice(&[5]).is_err());
}

#[test]
fn every_release1_account_rejects_bad_header_reserved_and_enum_bytes() {
    let mut proposal = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1);
    proposal.discriminator[0] ^= 1;
    assert_eq!(
        proposal.validate_schema(),
        Err(GovernanceError::InvalidDiscriminator)
    );
    proposal.discriminator = UPGRADE_PROPOSAL_V2_DISCRIMINATOR;
    proposal.account_version = 1;
    assert_eq!(
        proposal.validate_schema(),
        Err(GovernanceError::UnsupportedVersion)
    );
    proposal.account_version = 2;
    proposal.initialized = false;
    assert_eq!(
        proposal.validate_schema(),
        Err(GovernanceError::Uninitialized)
    );
    proposal.initialized = true;
    proposal.reserved[0] = 1;
    assert_eq!(
        proposal.validate_schema(),
        Err(GovernanceError::NonzeroReserved)
    );

    let mut buffer = sample_buffer_verification();
    buffer.reserved[0] = 1;
    assert_eq!(
        buffer.validate_schema(),
        Err(GovernanceError::NonzeroReserved)
    );
    let mut programdata = sample_programdata_verification();
    programdata.reserved[0] = 1;
    assert_eq!(
        programdata.validate_schema(),
        Err(GovernanceError::NonzeroReserved)
    );
    let mut checkpoint = sample_checkpoint();
    checkpoint.reserved[0] = 1;
    assert_eq!(
        checkpoint.validate_schema(),
        Err(GovernanceError::NonzeroReserved)
    );
    let mut rotation = sample_rotation();
    rotation.reserved[0] = 1;
    assert_eq!(
        rotation.validate_schema(),
        Err(GovernanceError::NonzeroReserved)
    );
    let mut resolution = sample_emergency_resolution();
    resolution.reserved[0] = 1;
    assert_eq!(
        resolution.validate_schema(),
        Err(GovernanceError::NonzeroReserved)
    );
    let mut freeze_observation = sample_emergency_freeze_observation();
    freeze_observation.reserved[0] = 1;
    assert_eq!(
        freeze_observation.validate_schema(),
        Err(GovernanceError::NonzeroReserved)
    );
    let mut failure_observation = sample_programdata_failure_observation();
    failure_observation.reserved[0] = 1;
    assert_eq!(
        failure_observation.validate_schema(),
        Err(GovernanceError::NonzeroReserved)
    );
    let mut checkpoint_attestation = sample_checkpoint_attestation();
    checkpoint_attestation.reserved[0] = 1;
    assert_eq!(
        checkpoint_attestation.validate_schema(),
        Err(GovernanceError::NonzeroReserved)
    );
    let mut freeze_observation = sample_emergency_freeze_observation();
    freeze_observation.discriminator[0] ^= 1;
    assert_eq!(
        freeze_observation.validate_schema(),
        Err(GovernanceError::InvalidDiscriminator)
    );
    let mut freeze_observation = sample_emergency_freeze_observation();
    freeze_observation.account_version = 2;
    assert_eq!(
        freeze_observation.validate_schema(),
        Err(GovernanceError::UnsupportedVersion)
    );
    let mut failure_observation = sample_programdata_failure_observation();
    failure_observation.discriminator[0] ^= 1;
    assert_eq!(
        failure_observation.validate_schema(),
        Err(GovernanceError::InvalidDiscriminator)
    );
    let mut failure_observation = sample_programdata_failure_observation();
    failure_observation.account_version = 2;
    assert_eq!(
        failure_observation.validate_schema(),
        Err(GovernanceError::UnsupportedVersion)
    );
    let mut checkpoint_attestation = sample_checkpoint_attestation();
    checkpoint_attestation.discriminator[0] ^= 1;
    assert_eq!(
        checkpoint_attestation.validate_schema(),
        Err(GovernanceError::InvalidDiscriminator)
    );
    let mut checkpoint_attestation = sample_checkpoint_attestation();
    checkpoint_attestation.account_version = 2;
    assert_eq!(
        checkpoint_attestation.validate_schema(),
        Err(GovernanceError::UnsupportedVersion)
    );

    let mut encoded = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1)
        .try_to_vec()
        .unwrap();
    encoded[upgrade_proposal_v2_offset::STATE] = u8::MAX;
    assert!(UpgradeProposalV2::try_from_slice(&encoded).is_err());
    let mut encoded = sample_buffer_verification().try_to_vec().unwrap();
    encoded[buffer_verification_v1_offset::STATUS] = u8::MAX;
    assert!(BufferVerificationV1::try_from_slice(&encoded).is_err());
    let mut encoded = sample_programdata_verification().try_to_vec().unwrap();
    encoded[programdata_verification_v1_offset::STATUS] = u8::MAX;
    assert!(ProgramDataVerificationV1::try_from_slice(&encoded).is_err());
    let mut encoded = sample_checkpoint().try_to_vec().unwrap();
    encoded[state_checkpoint_v1_offset::PHASE] = u8::MAX;
    assert!(StateCheckpointV1::try_from_slice(&encoded).is_err());
    let mut encoded = sample_rotation().try_to_vec().unwrap();
    encoded[council_rotation_v1_offset::STATE] = u8::MAX;
    assert!(CouncilRotationProposalV1::try_from_slice(&encoded).is_err());
    let mut encoded = sample_emergency_resolution().try_to_vec().unwrap();
    encoded[emergency_resolution_v1_offset::STATE] = u8::MAX;
    assert!(EmergencyFreezeResolutionV1::try_from_slice(&encoded).is_err());
    let mut encoded = sample_programdata_failure_observation()
        .try_to_vec()
        .unwrap();
    encoded[programdata_failure_observation_v1_offset::MISMATCH_CLASS] = u8::MAX;
    assert!(ProgramDataFailureObservationV1::try_from_slice(&encoded).is_err());
    let mut encoded = sample_checkpoint_attestation().try_to_vec().unwrap();
    encoded[checkpoint_attestation_v1_offset::PHASE] = u8::MAX;
    assert!(CheckpointAttestationV1::try_from_slice(&encoded).is_err());

    let mut invalid_bool = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1)
        .try_to_vec()
        .unwrap();
    invalid_bool[14] = 2;
    assert!(UpgradeProposalV2::try_from_slice(&invalid_bool).is_err());
    let mut invalid_bool = sample_emergency_freeze_observation().try_to_vec().unwrap();
    invalid_bool[emergency_freeze_observation_v1_offset::FINALIZED] = 2;
    assert!(EmergencyFreezeObservationV1::try_from_slice(&invalid_bool).is_err());
    let mut invalid_bool = sample_programdata_failure_observation()
        .try_to_vec()
        .unwrap();
    invalid_bool[programdata_failure_observation_v1_offset::PROGRAMDATA_HEADER_PRESENT] = 2;
    assert!(ProgramDataFailureObservationV1::try_from_slice(&invalid_bool).is_err());
    let mut invalid_bool = sample_programdata_failure_observation()
        .try_to_vec()
        .unwrap();
    invalid_bool[programdata_failure_observation_v1_offset::PROGRAMDATA_EXECUTABLE] = 2;
    assert!(ProgramDataFailureObservationV1::try_from_slice(&invalid_bool).is_err());
    let mut invalid_bool = sample_checkpoint_attestation().try_to_vec().unwrap();
    invalid_bool[10] = 2;
    assert!(CheckpointAttestationV1::try_from_slice(&invalid_bool).is_err());
}

#[test]
fn checkpoint_attestations_are_fixed_per_seat_and_fail_closed() {
    let attestation = sample_checkpoint_attestation();
    assert_eq!(attestation.validate_schema(), Ok(()));
    assert_eq!(
        validate_checkpoint_attestation_digest_v1(&attestation),
        Ok(())
    );

    let mut invalid = attestation.clone();
    invalid.controller_program = Pubkey::default();
    assert_eq!(
        invalid.validate_schema(),
        Err(GovernanceError::DefaultPubkey)
    );
    let mut invalid = attestation.clone();
    invalid.subject = Pubkey::default();
    assert_eq!(
        invalid.validate_schema(),
        Err(GovernanceError::DefaultPubkey)
    );
    let mut invalid = attestation.clone();
    invalid.subject_digest = [0; 32];
    assert_eq!(
        invalid.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut invalid = attestation.clone();
    invalid.checkpoint_digest = [0; 32];
    assert_eq!(
        invalid.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut invalid = attestation.clone();
    invalid.council_version = 0;
    assert_eq!(
        invalid.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut invalid = attestation.clone();
    invalid.seat_index = 5;
    assert_eq!(
        invalid.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut invalid = attestation.clone();
    invalid.attested_slot = 0;
    assert_eq!(
        invalid.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let baseline = compute_checkpoint_attestation_digest_v1(&attestation).unwrap();
    for changed in [
        {
            let mut changed = attestation.clone();
            changed.checkpoint_digest[0] ^= 1;
            changed
        },
        {
            let mut changed = attestation.clone();
            changed.subject_digest[0] ^= 1;
            changed
        },
        {
            let mut changed = attestation.clone();
            changed.phase = StateCheckpointPhaseV1::Poststate;
            changed
        },
        {
            let mut changed = attestation.clone();
            changed.council_version += 1;
            changed
        },
        {
            let mut changed = attestation.clone();
            changed.gate_epoch += 1;
            changed
        },
        {
            let mut changed = attestation.clone();
            changed.seat_index = 3;
            changed
        },
        {
            let mut changed = attestation.clone();
            changed.seat_authority = key(63);
            changed
        },
        {
            let mut changed = attestation.clone();
            changed.attested_slot += 1;
            changed
        },
    ] {
        assert_ne!(
            compute_checkpoint_attestation_digest_v1(&changed).unwrap(),
            baseline
        );
    }
}

#[test]
fn schema_validators_enforce_commitments_bitmaps_and_independent_zero_tail() {
    assert_eq!(
        sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1).validate_schema(),
        Ok(())
    );
    for rejected_chunk_size in [ARTIFACT_CHUNK_SIZE_4_KIB, ARTIFACT_CHUNK_SIZE_8_KIB] {
        let mut proposal = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1);
        proposal.chunk_size = rejected_chunk_size;
        assert_eq!(
            proposal.validate_schema(),
            Err(GovernanceError::InvalidProposalCommitment)
        );
    }
    assert_eq!(sample_buffer_verification().validate_schema(), Ok(()));
    assert_eq!(sample_programdata_verification().validate_schema(), Ok(()));
    assert_eq!(sample_checkpoint().validate_schema(), Ok(()));
    assert_eq!(sample_rotation().validate_schema(), Ok(()));
    assert_eq!(sample_emergency_resolution().validate_schema(), Ok(()));
    assert_eq!(
        sample_emergency_freeze_observation().validate_schema(),
        Ok(())
    );
    assert_eq!(
        sample_programdata_failure_observation().validate_schema(),
        Ok(())
    );

    let mut proposal = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1);
    proposal.chunk_hash_domain[0] ^= 1;
    assert_eq!(
        proposal.validate_schema(),
        Err(GovernanceError::InvalidProposalCommitment)
    );
    let mut proposal = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1);
    proposal.state = ProposalStateV2::TokenReviewOpen;
    assert_eq!(
        proposal.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut proposal = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1);
    proposal.proposal_class = ProposalClassV1::TargetImmutability;
    assert_eq!(
        proposal.validate_schema(),
        Err(GovernanceError::UnsupportedProposalClass)
    );

    let mut buffer = sample_buffer_verification();
    buffer.verified_chunk_count -= 1;
    assert_eq!(
        buffer.validate_schema(),
        Err(GovernanceError::InvalidRelease1Bitmap)
    );
    let mut buffer = sample_buffer_verification();
    buffer.verified_chunk_bitmap[VERIFICATION_BITMAP_BYTES_V1 - 1] = 0x80;
    buffer.verified_chunk_count += 1;
    assert_eq!(
        buffer.validate_schema(),
        Err(GovernanceError::InvalidRelease1Bitmap)
    );

    let mut programdata = sample_programdata_verification();
    programdata.verified_tail_chunk_bitmap = [0; VERIFICATION_BITMAP_BYTES_V1];
    programdata.verified_tail_chunk_count = 0;
    assert_eq!(
        programdata.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut programdata = sample_programdata_verification();
    programdata.verified_payload_chunk_bitmap = [0; VERIFICATION_BITMAP_BYTES_V1];
    programdata.verified_payload_chunk_count = 0;
    assert_eq!(
        programdata.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut checkpoint = sample_checkpoint();
    checkpoint.forbidden_drift_count = 1;
    assert_eq!(
        checkpoint.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    checkpoint.accepted = false;
    checkpoint.checkpoint_digest = compute_state_checkpoint_digest_v1(&checkpoint).unwrap();
    assert_eq!(checkpoint.validate_schema(), Ok(()));
    assert_eq!(validate_state_checkpoint_digest_v1(&checkpoint), Ok(()));
    let mut checkpoint = sample_checkpoint();
    checkpoint.admitted_positive_donation_count = 0;
    assert_eq!(
        checkpoint.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut resolution = sample_emergency_resolution();
    resolution.freeze_reason_code = 1;
    assert_eq!(
        resolution.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
}

#[test]
fn failure_observation_mismatch_shapes_are_exhaustive_and_fail_closed() {
    let mut header = sample_programdata_failure_observation();
    header.mismatch_class = ProgramDataMismatchClassV1::Header;
    header.programdata_header_present = false;
    header.actual_programdata_slot = 0;
    header.actual_capacity = 0;
    header.actual_authority = OptionalPubkeyV1::none();
    header.failing_chunk_index = NO_FAILING_CHUNK_INDEX_V1;
    header.expected_leaf_hash = [0; 32];
    header.actual_leaf_hash = [0; 32];
    assert_eq!(header.validate_schema(), Ok(()));

    let mut authority = sample_programdata_failure_observation();
    authority.mismatch_class = ProgramDataMismatchClassV1::Authority;
    authority.actual_authority = OptionalPubkeyV1::none();
    authority.failing_chunk_index = NO_FAILING_CHUNK_INDEX_V1;
    authority.expected_leaf_hash = [0; 32];
    authority.actual_leaf_hash = [0; 32];
    assert_eq!(authority.validate_schema(), Ok(()));

    let mut capacity = sample_programdata_failure_observation();
    capacity.mismatch_class = ProgramDataMismatchClassV1::Capacity;
    capacity.actual_capacity = 0;
    capacity.actual_data_length = LOADER_V3_PROGRAMDATA_METADATA_LEN_V1;
    capacity.failing_chunk_index = NO_FAILING_CHUNK_INDEX_V1;
    capacity.expected_leaf_hash = [0; 32];
    capacity.actual_leaf_hash = [0; 32];
    assert_eq!(capacity.validate_schema(), Ok(()));

    let payload = sample_programdata_failure_observation();
    assert_eq!(payload.validate_schema(), Ok(()));
    let mut zero_tail = sample_programdata_failure_observation();
    zero_tail.mismatch_class = ProgramDataMismatchClassV1::ZeroTail;
    assert_eq!(zero_tail.validate_schema(), Ok(()));

    let mut wrong_absence = header.clone();
    wrong_absence.mismatch_class = ProgramDataMismatchClassV1::Capacity;
    assert_eq!(
        wrong_absence.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut missing_index = payload.clone();
    missing_index.failing_chunk_index = NO_FAILING_CHUNK_INDEX_V1;
    assert_eq!(
        missing_index.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut equal_leaf = payload.clone();
    equal_leaf.actual_leaf_hash = equal_leaf.expected_leaf_hash;
    assert_eq!(
        equal_leaf.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut spurious_leaf = authority.clone();
    spurious_leaf.expected_leaf_hash = bytes(58);
    assert_eq!(
        spurious_leaf.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut unfinalized = payload.clone();
    unfinalized.finalized = false;
    assert_eq!(
        unfinalized.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut zero_data_length = payload.clone();
    zero_data_length.actual_data_length = 0;
    assert_eq!(
        zero_data_length.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
}

#[test]
fn loader_header_classification_preserves_malformed_freeze_evidence() {
    let mut default_link_program = sample_emergency_freeze_observation();
    default_link_program.program_header_present = false;
    default_link_program.actual_linked_programdata = OptionalPubkeyV1::none();
    // The exact 36-byte length remains recorded. A structurally decodable
    // Program tag with a default link is classified as noncanonical/absent.
    default_link_program.actual_program_data_length = LOADER_V3_PROGRAM_ACCOUNT_LEN_V1;
    default_link_program.observation_digest =
        compute_emergency_freeze_observation_digest_v1(&default_link_program).unwrap();
    assert_eq!(
        validate_emergency_freeze_observation_digest_v1(&default_link_program),
        Ok(())
    );

    let mut invalid_present_link = default_link_program.clone();
    invalid_present_link.program_header_present = true;
    assert_eq!(
        invalid_present_link.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut invalid_absent_header = default_link_program.clone();
    invalid_absent_header.actual_linked_programdata = OptionalPubkeyV1::some(key(98)).unwrap();
    assert_eq!(
        invalid_absent_header.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut wrong_program_length = sample_emergency_freeze_observation();
    wrong_program_length.actual_program_data_length = LOADER_V3_PROGRAM_ACCOUNT_LEN_V1 + 1;
    assert_eq!(
        wrong_program_length.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut noncanonical_programdata_header = sample_emergency_freeze_observation();
    noncanonical_programdata_header.programdata_header_present = false;
    noncanonical_programdata_header.deployed_programdata_slot = 0;
    noncanonical_programdata_header.capacity = 0;
    noncanonical_programdata_header.observed_authority = OptionalPubkeyV1::none();
    // Raw length/hash metadata still identifies the malformed ProgramData
    // account. A decoded Some(default) authority is represented by the
    // canonical absent-header classification rather than an invalid optional.
    noncanonical_programdata_header.observation_digest =
        compute_emergency_freeze_observation_digest_v1(&noncanonical_programdata_header).unwrap();
    assert_eq!(
        validate_emergency_freeze_observation_digest_v1(&noncanonical_programdata_header),
        Ok(())
    );

    let mut resolution = sample_emergency_resolution();
    resolution.observed_program_header_present = false;
    resolution.observed_linked_programdata = OptionalPubkeyV1::none();
    resolution.resolution_digest = compute_emergency_resolution_digest_v1(&resolution).unwrap();
    assert_eq!(validate_emergency_resolution_digest_v1(&resolution), Ok(()));
    resolution.state = EmergencyFreezeResolutionStateV1::Executed;
    resolution.executed_slot = resolution.not_before_slot;
    resolution.terminal_reason_code = EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V1;
    assert_eq!(
        resolution.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut failure = sample_programdata_failure_observation();
    failure.program_header_present = false;
    failure.actual_linked_programdata = OptionalPubkeyV1::none();
    failure.observation_digest =
        compute_programdata_failure_observation_digest_v1(&failure).unwrap();
    assert_eq!(
        validate_programdata_failure_observation_digest_v1(&failure),
        Ok(())
    );
}

#[test]
fn raw_programdata_hash_completeness_is_an_exact_ceiling_relation() {
    let mut freeze = sample_emergency_freeze_observation();
    freeze.actual_programdata_data_length = MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1;
    freeze.capacity =
        MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 - LOADER_V3_PROGRAMDATA_METADATA_LEN_V1;
    assert_eq!(freeze.validate_schema(), Ok(()));

    let mut incomplete_at_boundary = freeze.clone();
    incomplete_at_boundary.raw_hash_complete = false;
    incomplete_at_boundary.raw_programdata_sha256 = [0; 32];
    assert_eq!(
        incomplete_at_boundary.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut oversized = freeze.clone();
    oversized.actual_programdata_data_length = MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1;
    oversized.capacity =
        oversized.actual_programdata_data_length - LOADER_V3_PROGRAMDATA_METADATA_LEN_V1;
    oversized.raw_hash_complete = false;
    oversized.raw_programdata_sha256 = [0; 32];
    oversized.observation_digest =
        compute_emergency_freeze_observation_digest_v1(&oversized).unwrap();
    assert_eq!(
        validate_emergency_freeze_observation_digest_v1(&oversized),
        Ok(())
    );

    let mut complete_oversized = oversized.clone();
    complete_oversized.raw_hash_complete = true;
    complete_oversized.raw_programdata_sha256 = bytes(56);
    assert_eq!(
        complete_oversized.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut nonzero_incomplete = oversized.clone();
    nonzero_incomplete.raw_programdata_sha256 = bytes(56);
    assert_eq!(
        nonzero_incomplete.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut resolution = sample_emergency_resolution();
    resolution.observed_programdata_data_length = MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1;
    resolution.observed_capacity =
        resolution.observed_programdata_data_length - LOADER_V3_PROGRAMDATA_METADATA_LEN_V1;
    resolution.observed_raw_hash_complete = false;
    resolution.observed_raw_programdata_hash = [0; 32];
    resolution.resolution_digest = compute_emergency_resolution_digest_v1(&resolution).unwrap();
    assert_eq!(validate_emergency_resolution_digest_v1(&resolution), Ok(()));
    resolution.state = EmergencyFreezeResolutionStateV1::Executed;
    resolution.executed_slot = resolution.not_before_slot;
    resolution.terminal_reason_code = EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V1;
    assert_eq!(
        resolution.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    for mismatch_class in [
        ProgramDataMismatchClassV1::Header,
        ProgramDataMismatchClassV1::Authority,
        ProgramDataMismatchClassV1::Capacity,
    ] {
        let mut failure = sample_programdata_failure_observation();
        failure.mismatch_class = mismatch_class;
        failure.failing_chunk_index = NO_FAILING_CHUNK_INDEX_V1;
        failure.expected_leaf_hash = [0; 32];
        failure.actual_leaf_hash = [0; 32];
        failure.actual_data_length = MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1;
        failure.actual_capacity =
            failure.actual_data_length - LOADER_V3_PROGRAMDATA_METADATA_LEN_V1;
        failure.raw_hash_complete = false;
        failure.actual_raw_programdata_sha256 = [0; 32];
        if mismatch_class == ProgramDataMismatchClassV1::Authority {
            failure.actual_authority = OptionalPubkeyV1::none();
        }
        failure.observation_digest =
            compute_programdata_failure_observation_digest_v1(&failure).unwrap();
        assert_eq!(
            validate_programdata_failure_observation_digest_v1(&failure),
            Ok(()),
            "{mismatch_class:?} must persist oversized incomplete evidence"
        );
    }

    for mismatch_class in [
        ProgramDataMismatchClassV1::PayloadLeaf,
        ProgramDataMismatchClassV1::ZeroTail,
    ] {
        let mut failure = sample_programdata_failure_observation();
        failure.mismatch_class = mismatch_class;
        failure.actual_data_length = MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1;
        failure.actual_capacity =
            failure.actual_data_length - LOADER_V3_PROGRAMDATA_METADATA_LEN_V1;
        failure.raw_hash_complete = false;
        failure.actual_raw_programdata_sha256 = [0; 32];
        assert_eq!(
            failure.validate_schema(),
            Err(GovernanceError::InvalidRelease1Account),
            "{mismatch_class:?} cannot rely on an incomplete raw hash"
        );
    }
}

#[test]
fn guardian_observation_authority_is_exact_optional_evidence_not_a_freeze_precondition() {
    let mut absent = sample_emergency_freeze_observation();
    absent.observed_authority = OptionalPubkeyV1::none();
    absent.observation_digest = compute_emergency_freeze_observation_digest_v1(&absent).unwrap();
    assert_eq!(absent.validate_schema(), Ok(()));
    assert_eq!(
        validate_emergency_freeze_observation_digest_v1(&absent),
        Ok(())
    );

    let mut drifted = sample_emergency_freeze_observation();
    drifted.observed_authority = OptionalPubkeyV1::some(key(99)).unwrap();
    drifted.observation_digest = compute_emergency_freeze_observation_digest_v1(&drifted).unwrap();
    assert_eq!(drifted.validate_schema(), Ok(()));

    let mut malformed_header = sample_emergency_freeze_observation();
    malformed_header.actual_programdata_owner = key(98);
    malformed_header.actual_programdata_executable = true;
    malformed_header.actual_programdata_data_length = 17;
    malformed_header.programdata_header_present = false;
    malformed_header.deployed_programdata_slot = 0;
    malformed_header.capacity = 0;
    malformed_header.observed_authority = OptionalPubkeyV1::none();
    malformed_header.observation_digest =
        compute_emergency_freeze_observation_digest_v1(&malformed_header).unwrap();
    assert_eq!(malformed_header.validate_schema(), Ok(()));

    let mut contradictory_header = malformed_header.clone();
    contradictory_header.deployed_programdata_slot = 1;
    assert_eq!(
        contradictory_header.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut resolution = sample_emergency_resolution();
    resolution.observed_authority = OptionalPubkeyV1::none();
    resolution.resolution_digest = compute_emergency_resolution_digest_v1(&resolution).unwrap();
    assert_eq!(resolution.validate_schema(), Ok(()));
    assert_eq!(validate_emergency_resolution_digest_v1(&resolution), Ok(()));

    let mut malformed_resolution = sample_emergency_resolution();
    malformed_resolution.observed_programdata_owner = key(98);
    malformed_resolution.observed_programdata_executable = true;
    malformed_resolution.observed_programdata_data_length = 17;
    malformed_resolution.observed_programdata_header_present = false;
    malformed_resolution.observed_programdata_slot = 0;
    malformed_resolution.observed_capacity = 0;
    malformed_resolution.observed_authority = OptionalPubkeyV1::none();
    malformed_resolution.resolution_digest =
        compute_emergency_resolution_digest_v1(&malformed_resolution).unwrap();
    assert_eq!(malformed_resolution.validate_schema(), Ok(()));
}

#[test]
fn rollback_shape_and_rotation_safe_current_council_accumulators_are_canonical() {
    let mut rollback = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1);
    rollback.proposal_class = ProposalClassV1::EmergencyRollback;
    rollback.primary_proposal = OptionalPubkeyV1::some(key(93)).unwrap();
    rollback.rollback_proposal = OptionalPubkeyV1::none();
    rollback.rollback_buffer = OptionalPubkeyV1::none();
    rollback.rollback_artifact_sha256 = [0; 32];
    rollback.rollback_artifact_chunk_root = [0; 32];
    rollback.deployed_slot = 0;
    rollback.current_raw_programdata_hash = [0; 32];
    rollback.proposal_digest = compute_proposal_digest_v2(&rollback).unwrap();
    assert_eq!(rollback.validate_schema(), Ok(()));
    assert_eq!(validate_proposal_digest_v2(&rollback), Ok(()));

    let mut checkpoint = sample_checkpoint();
    checkpoint.accepted = false;
    checkpoint.forbidden_drift_count = 1;
    checkpoint.approval_bitset = 0;
    checkpoint.approval_count = 0;
    assert_eq!(
        checkpoint.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    checkpoint.approval_council_version = 0;
    checkpoint.approval_council_hash = [0; 32];
    assert_eq!(
        checkpoint.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    checkpoint.approval_council_version = 3;
    checkpoint.approval_council_hash = bytes(94);
    checkpoint.approval_bitset = 0b0_0111;
    checkpoint.approval_count = 3;
    assert_eq!(checkpoint.validate_schema(), Ok(()));
    checkpoint.approval_bitset = 0b0_1111;
    checkpoint.approval_count = 4;
    assert_eq!(
        checkpoint.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut proposal = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1);
    proposal.state = ProposalStateV2::PoststateAccepted;
    proposal.freeze_gate_epoch = 10;
    proposal.first_approval_slot = 20;
    proposal.council_approved_slot = 20;
    proposal.governance_satisfied_slot = 21;
    proposal.queued_slot = 22;
    proposal.frozen_slot = 40;
    proposal.extension_executed_slot = 41;
    proposal.upgrade_executed_slot = 42;
    proposal.programdata_verified_slot = 43;
    proposal.poststate_accepted_slot = 44;
    proposal.council_approval_bitset = 0b0_0111;
    proposal.council_approval_count = 3;
    proposal.unfreeze_council_version = 3;
    proposal.unfreeze_council_hash = bytes(95);
    assert_eq!(
        proposal.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    proposal.unfreeze_approval_bitset = 1;
    proposal.unfreeze_approval_count = 1;
    assert_eq!(proposal.validate_schema(), Ok(()));
}

#[test]
fn proposal_lifecycle_accounts_have_one_canonical_shape_per_state() {
    for state in [
        ProposalStateV2::Draft,
        ProposalStateV2::BufferAdopted,
        ProposalStateV2::BufferVerified,
        ProposalStateV2::CouncilApproved,
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
    ] {
        let proposal = canonical_proposal_in_state(state);
        assert_eq!(proposal.validate_schema(), Ok(()), "state {state:?}");
    }

    let mut draft_with_approval = canonical_proposal_in_state(ProposalStateV2::Draft);
    draft_with_approval.council_approval_bitset = 1;
    draft_with_approval.council_approval_count = 1;
    draft_with_approval.first_approval_slot = draft_with_approval.review_start_slot;
    assert_eq!(
        draft_with_approval.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut threshold_without_transition =
        canonical_proposal_in_state(ProposalStateV2::BufferVerified);
    threshold_without_transition.council_approval_bitset = 0b0_0111;
    threshold_without_transition.council_approval_count = 3;
    threshold_without_transition.first_approval_slot = 20;
    threshold_without_transition.council_approved_slot = 20;
    assert_eq!(
        threshold_without_transition.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut cancelled_reason_drift = canonical_proposal_in_state(ProposalStateV2::Cancelled);
    cancelled_reason_drift.terminal_reason_code += 1;
    assert_eq!(
        cancelled_reason_drift.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut completed_with_missing_poststate =
        canonical_proposal_in_state(ProposalStateV2::Completed);
    completed_with_missing_poststate.poststate_accepted_slot = 0;
    assert_eq!(
        completed_with_missing_poststate.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut reversed_slots = canonical_proposal_in_state(ProposalStateV2::UpgradeExecuted);
    reversed_slots.upgrade_executed_slot = reversed_slots.extension_executed_slot - 1;
    assert_eq!(
        reversed_slots.validate_schema(),
        Err(GovernanceError::InvalidProposalTiming)
    );
    let mut same_slot_extension = canonical_proposal_in_state(ProposalStateV2::UpgradeExecuted);
    same_slot_extension.upgrade_executed_slot = same_slot_extension.extension_executed_slot;
    assert_eq!(
        same_slot_extension.validate_schema(),
        Err(GovernanceError::InvalidProposalTiming)
    );

    let mut primary_retired = canonical_proposal_in_state(ProposalStateV2::Timelocked);
    primary_retired.state = ProposalStateV2::Retired;
    primary_retired.terminal_slot = 45;
    primary_retired.terminal_reason_code = PROPOSAL_RETIRED_ROLLBACK_TERMINAL_REASON_V1;
    assert_eq!(
        primary_retired.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
}

#[test]
fn auxiliary_release1_accounts_reject_unreachable_states_and_reversed_slots() {
    let rotation = sample_rotation();
    for state in [
        CouncilRotationStateV1::Draft,
        CouncilRotationStateV1::CouncilApproved,
        CouncilRotationStateV1::Timelocked,
        CouncilRotationStateV1::Activated,
        CouncilRotationStateV1::Cancelled,
        CouncilRotationStateV1::Expired,
    ] {
        let mut candidate = rotation.clone();
        candidate.state = state;
        match state {
            CouncilRotationStateV1::Draft => {
                candidate.approval_bitset = 0;
                candidate.approval_count = 0;
            }
            CouncilRotationStateV1::Activated => {
                candidate.activated_slot = candidate.not_before_slot;
                candidate.terminal_reason_code = COUNCIL_ROTATION_ACTIVATED_TERMINAL_REASON_V1;
            }
            CouncilRotationStateV1::Cancelled => {
                candidate.cancellation_approval_bitset = 0b0_0111;
                candidate.cancellation_approval_count = 3;
                candidate.cancellation_reason_code = 78;
                candidate.terminal_reason_code = 78;
            }
            CouncilRotationStateV1::Expired => {
                candidate.terminal_reason_code = COUNCIL_ROTATION_EXPIRED_TERMINAL_REASON_V1;
            }
            CouncilRotationStateV1::CouncilApproved | CouncilRotationStateV1::Timelocked => {}
        }
        assert_eq!(candidate.validate_schema(), Ok(()), "rotation {state:?}");
    }
    let mut draft_with_quorum = rotation.clone();
    draft_with_quorum.state = CouncilRotationStateV1::Draft;
    assert_eq!(
        draft_with_quorum.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut late_activation = rotation.clone();
    late_activation.state = CouncilRotationStateV1::Activated;
    late_activation.activated_slot = late_activation.expiry_slot;
    late_activation.terminal_reason_code = COUNCIL_ROTATION_ACTIVATED_TERMINAL_REASON_V1;
    assert_eq!(
        late_activation.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let resolution = sample_emergency_resolution();
    for state in [
        EmergencyFreezeResolutionStateV1::Draft,
        EmergencyFreezeResolutionStateV1::CouncilApproved,
        EmergencyFreezeResolutionStateV1::Timelocked,
        EmergencyFreezeResolutionStateV1::Executed,
        EmergencyFreezeResolutionStateV1::Expired,
    ] {
        let mut candidate = resolution.clone();
        candidate.state = state;
        match state {
            EmergencyFreezeResolutionStateV1::Draft => {
                candidate.approval_council_version = 0;
                candidate.approval_council_hash = [0; 32];
                candidate.approval_bitset = 0;
                candidate.approval_count = 0;
            }
            EmergencyFreezeResolutionStateV1::Executed => {
                candidate.executed_slot = candidate.not_before_slot;
                candidate.terminal_reason_code = EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V1;
            }
            EmergencyFreezeResolutionStateV1::Expired => {
                candidate.terminal_reason_code = EMERGENCY_RESOLUTION_EXPIRED_TERMINAL_REASON_V1;
            }
            EmergencyFreezeResolutionStateV1::CouncilApproved
            | EmergencyFreezeResolutionStateV1::Timelocked => {}
            EmergencyFreezeResolutionStateV1::Cancelled => unreachable!(),
        }
        assert_eq!(candidate.validate_schema(), Ok(()), "resolution {state:?}");
    }
    let mut unreachable_cancel = resolution.clone();
    unreachable_cancel.state = EmergencyFreezeResolutionStateV1::Cancelled;
    assert_eq!(
        unreachable_cancel.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut late_execution = resolution.clone();
    late_execution.state = EmergencyFreezeResolutionStateV1::Executed;
    late_execution.executed_slot = late_execution.expiry_slot;
    late_execution.terminal_reason_code = EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V1;
    assert_eq!(
        late_execution.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut buffer = sample_buffer_verification();
    buffer.finalized_slot = buffer.adopted_slot - 1;
    assert_eq!(
        buffer.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut programdata = sample_programdata_verification();
    programdata.finalized_slot = programdata.deployed_slot - 1;
    assert_eq!(
        programdata.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    let mut checkpoint = sample_checkpoint();
    checkpoint.finalized_observation_slot = checkpoint.finalized_slot + 1;
    assert_eq!(
        checkpoint.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
}

#[test]
fn council_rotation_candidate_versions_are_strictly_monotonic_but_may_skip() {
    let mut rotation = sample_rotation();
    rotation.candidate_council_version = 9;
    rotation.rotation_digest = compute_council_rotation_digest_v1(&rotation).unwrap();
    assert_eq!(rotation.validate_schema(), Ok(()));
    assert_eq!(validate_council_rotation_digest_v1(&rotation), Ok(()));

    rotation.candidate_council_version = rotation.current_council_version;
    assert_eq!(
        rotation.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
    rotation.candidate_council_version = rotation.current_council_version - 1;
    assert_eq!(
        rotation.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );

    let mut maximum = sample_rotation();
    maximum.current_council_version = u64::MAX - 1;
    maximum.candidate_council_version = u64::MAX;
    maximum.rotation_digest = compute_council_rotation_digest_v1(&maximum).unwrap();
    assert_eq!(
        maximum.validate_schema(),
        Err(GovernanceError::InvalidRelease1Account)
    );
}

#[test]
fn digest_domains_lengths_and_inclusion_boundaries_are_exact() {
    assert_eq!(PROPOSAL_DIGEST_DOMAIN_V2.len(), 26);
    assert_eq!(PROPOSAL_DIGEST_MATERIAL_LEN_V2, 1_416);
    assert_eq!(PROPOSAL_DIGEST_PREIMAGE_LEN_V2, 1_442);
    assert_eq!(STATE_CHECKPOINT_DIGEST_DOMAIN_V1.len(), 26);
    assert_eq!(STATE_CHECKPOINT_DIGEST_MATERIAL_LEN_V1, 573);
    assert_eq!(STATE_CHECKPOINT_DIGEST_PREIMAGE_LEN_V1, 599);
    assert_eq!(STATE_CHECKPOINT_HARD_ROOT_DOMAIN_V1.len(), 30);
    assert_eq!(STATE_CHECKPOINT_HARD_ROOT_MATERIAL_LEN_V1, 176);
    assert_eq!(STATE_CHECKPOINT_HARD_ROOT_PREIMAGE_LEN_V1, 206);
    assert_eq!(COUNCIL_ROTATION_DIGEST_DOMAIN_V1.len(), 26);
    assert_eq!(COUNCIL_ROTATION_DIGEST_MATERIAL_LEN_V1, 240);
    assert_eq!(COUNCIL_ROTATION_DIGEST_PREIMAGE_LEN_V1, 266);
    assert_eq!(EMERGENCY_RESOLUTION_DIGEST_DOMAIN_V1.len(), 30);
    assert_eq!(EMERGENCY_RESOLUTION_DIGEST_MATERIAL_LEN_V1, 442);
    assert_eq!(EMERGENCY_RESOLUTION_DIGEST_PREIMAGE_LEN_V1, 472);
    assert_eq!(EMERGENCY_FREEZE_OBSERVATION_DIGEST_DOMAIN_V1.len(), 38);
    assert_eq!(EMERGENCY_FREEZE_OBSERVATION_DIGEST_MATERIAL_LEN_V1, 450);
    assert_eq!(EMERGENCY_FREEZE_OBSERVATION_DIGEST_PREIMAGE_LEN_V1, 488);
    assert_eq!(PROGRAMDATA_FAILURE_OBSERVATION_DIGEST_DOMAIN_V1.len(), 41);
    assert_eq!(PROGRAMDATA_FAILURE_OBSERVATION_DIGEST_MATERIAL_LEN_V1, 444);
    assert_eq!(PROGRAMDATA_FAILURE_OBSERVATION_DIGEST_PREIMAGE_LEN_V1, 485);
    assert_eq!(CHECKPOINT_ATTESTATION_DIGEST_DOMAIN_V1.len(), 32);
    assert_eq!(CHECKPOINT_ATTESTATION_DIGEST_MATERIAL_LEN_V1, 314);
    assert_eq!(CHECKPOINT_ATTESTATION_DIGEST_PREIMAGE_LEN_V1, 346);

    let proposal = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1);
    let checkpoint = sample_checkpoint();
    let rotation = sample_rotation();
    let resolution = sample_emergency_resolution();
    let freeze_observation = sample_emergency_freeze_observation();
    let failure_observation = sample_programdata_failure_observation();
    let checkpoint_attestation = sample_checkpoint_attestation();
    assert_eq!(
        canonical_proposal_digest_material_v2(&proposal)
            .unwrap()
            .len(),
        1_416
    );
    assert_eq!(
        canonical_state_checkpoint_digest_material_v1(&checkpoint)
            .unwrap()
            .len(),
        573
    );
    assert_eq!(
        canonical_state_checkpoint_hard_root_material_v1(&checkpoint)
            .unwrap()
            .len(),
        176
    );
    assert_eq!(
        canonical_council_rotation_digest_material_v1(&rotation)
            .unwrap()
            .len(),
        240
    );
    assert_eq!(
        canonical_emergency_resolution_digest_material_v1(&resolution)
            .unwrap()
            .len(),
        442
    );
    assert_eq!(
        canonical_emergency_freeze_observation_digest_material_v1(&freeze_observation)
            .unwrap()
            .len(),
        450
    );
    assert_eq!(
        canonical_programdata_failure_observation_digest_material_v1(&failure_observation)
            .unwrap()
            .len(),
        444
    );
    assert_eq!(
        canonical_checkpoint_attestation_digest_material_v1(&checkpoint_attestation)
            .unwrap()
            .len(),
        314
    );
    assert_eq!(validate_proposal_digest_v2(&proposal), Ok(()));
    assert_eq!(validate_state_checkpoint_digest_v1(&checkpoint), Ok(()));
    assert_eq!(
        validate_state_checkpoint_hard_combined_root_v1(&checkpoint),
        Ok(())
    );
    assert_eq!(validate_council_rotation_digest_v1(&rotation), Ok(()));
    assert_eq!(validate_emergency_resolution_digest_v1(&resolution), Ok(()));
    assert_eq!(
        validate_emergency_freeze_observation_digest_v1(&freeze_observation),
        Ok(())
    );
    assert_eq!(
        validate_programdata_failure_observation_digest_v1(&failure_observation),
        Ok(())
    );
    assert_eq!(
        validate_checkpoint_attestation_digest_v1(&checkpoint_attestation),
        Ok(())
    );
    assert_eq!(proposal.proposal_digest, GOLDEN_PROPOSAL_DIGEST_V2);
    assert_eq!(checkpoint.checkpoint_digest, GOLDEN_CHECKPOINT_DIGEST_V1);
    assert_eq!(
        checkpoint.hard_combined_root,
        GOLDEN_CHECKPOINT_HARD_ROOT_V1
    );
    assert_eq!(rotation.rotation_digest, GOLDEN_ROTATION_DIGEST_V1);
    assert_eq!(resolution.resolution_digest, GOLDEN_EMERGENCY_DIGEST_V1);
    assert_eq!(
        checkpoint_attestation.attestation_digest,
        GOLDEN_CHECKPOINT_ATTESTATION_DIGEST_V1
    );

    let freeze_baseline =
        compute_emergency_freeze_observation_digest_v1(&freeze_observation).unwrap();
    let mut changed_freeze = freeze_observation.clone();
    changed_freeze.raw_programdata_sha256[0] ^= 1;
    assert_ne!(
        compute_emergency_freeze_observation_digest_v1(&changed_freeze).unwrap(),
        freeze_baseline
    );
    let mut changed_freeze = freeze_observation.clone();
    changed_freeze.raw_hash_complete = false;
    assert_ne!(
        compute_emergency_freeze_observation_digest_v1(&changed_freeze).unwrap(),
        freeze_baseline
    );
    let failure_baseline =
        compute_programdata_failure_observation_digest_v1(&failure_observation).unwrap();
    let mut changed_failure = failure_observation.clone();
    changed_failure.actual_leaf_hash[0] ^= 1;
    assert_ne!(
        compute_programdata_failure_observation_digest_v1(&changed_failure).unwrap(),
        failure_baseline
    );
    let mut changed_failure = failure_observation.clone();
    changed_failure.actual_owner = key(98);
    assert_ne!(
        compute_programdata_failure_observation_digest_v1(&changed_failure).unwrap(),
        failure_baseline
    );
    let mut changed_failure = failure_observation.clone();
    changed_failure.actual_executable = true;
    assert_ne!(
        compute_programdata_failure_observation_digest_v1(&changed_failure).unwrap(),
        failure_baseline
    );
    let mut changed_failure = failure_observation.clone();
    changed_failure.actual_data_length += 1;
    assert_ne!(
        compute_programdata_failure_observation_digest_v1(&changed_failure).unwrap(),
        failure_baseline
    );
    let mut changed_failure = failure_observation.clone();
    changed_failure.raw_hash_complete = false;
    assert_ne!(
        compute_programdata_failure_observation_digest_v1(&changed_failure).unwrap(),
        failure_baseline
    );

    let baseline = compute_proposal_digest_v2(&proposal).unwrap();
    let mut changed = proposal.clone();
    changed.proposal_class = ProposalClassV1::EconomicChange;
    assert_ne!(compute_proposal_digest_v2(&changed).unwrap(), baseline);
    let mut changed = proposal.clone();
    changed.creation_gate_status = GateStatusV1::EmergencyFrozen;
    assert_ne!(compute_proposal_digest_v2(&changed).unwrap(), baseline);
    let mut changed = proposal.clone();
    changed.proposal_id += 1;
    assert_ne!(compute_proposal_digest_v2(&changed).unwrap(), baseline);
    let mut changed = proposal.clone();
    changed.cluster_domain[0] ^= 1;
    assert_ne!(compute_proposal_digest_v2(&changed).unwrap(), baseline);
    let mut changed = proposal.clone();
    changed.policy_hash[0] ^= 1;
    assert_ne!(compute_proposal_digest_v2(&changed).unwrap(), baseline);
    let mut changed = proposal.clone();
    changed.creation_council_hash[0] ^= 1;
    assert_ne!(compute_proposal_digest_v2(&changed).unwrap(), baseline);
    let mut changed = proposal.clone();
    changed.target_program = key(90);
    assert_ne!(compute_proposal_digest_v2(&changed).unwrap(), baseline);
    let mut changed = proposal.clone();
    changed.buffer_pubkey = key(91);
    assert_ne!(compute_proposal_digest_v2(&changed).unwrap(), baseline);
    let mut changed = proposal.clone();
    changed.artifact_chunk_merkle_root[0] ^= 1;
    assert_ne!(compute_proposal_digest_v2(&changed).unwrap(), baseline);
    let mut changed = proposal.clone();
    changed.source_commit_hash[0] ^= 1;
    assert_ne!(compute_proposal_digest_v2(&changed).unwrap(), baseline);
    let mut changed = proposal.clone();
    changed.current_capacity += 1;
    assert_ne!(compute_proposal_digest_v2(&changed).unwrap(), baseline);
    let mut changed = proposal.clone();
    changed.checkpoint_policy_hash[0] ^= 1;
    assert_ne!(compute_proposal_digest_v2(&changed).unwrap(), baseline);
    let mut changed = proposal.clone();
    changed.rollback_artifact_sha256[0] ^= 1;
    assert_ne!(compute_proposal_digest_v2(&changed).unwrap(), baseline);
    let mut changed = proposal.clone();
    changed.review_start_slot += 1;
    assert_ne!(compute_proposal_digest_v2(&changed).unwrap(), baseline);

    let mut mutable_only = proposal.clone();
    mutable_only.state = ProposalStateV2::Completed;
    mutable_only.freeze_gate_epoch = 10;
    mutable_only.first_approval_slot = 1;
    mutable_only.terminal_slot = 999;
    mutable_only.council_approval_bitset = 7;
    mutable_only.council_approval_count = 3;
    mutable_only.unfreeze_council_version = 7;
    mutable_only.unfreeze_council_hash = bytes(92);
    mutable_only.unfreeze_approval_bitset = 7;
    mutable_only.unfreeze_approval_count = 3;
    mutable_only.cancellation_reason_code = 4;
    mutable_only.terminal_reason_code = 5;
    mutable_only.proposal_digest = bytes(93);
    mutable_only.bump ^= 1;
    assert_eq!(
        canonical_proposal_digest_material_v2(&mutable_only).unwrap(),
        canonical_proposal_digest_material_v2(&proposal).unwrap()
    );

    let checkpoint_baseline = compute_state_checkpoint_digest_v1(&checkpoint).unwrap();
    let hard_root_baseline = compute_state_checkpoint_hard_combined_root_v1(&checkpoint).unwrap();
    for changed_hard_input in [
        {
            let mut changed = checkpoint.clone();
            changed.schema_identifier[0] ^= 1;
            changed
        },
        {
            let mut changed = checkpoint.clone();
            changed.program_owned_state_root[0] ^= 1;
            changed
        },
        {
            let mut changed = checkpoint.clone();
            changed.program_owned_state_count += 1;
            changed
        },
        {
            let mut changed = checkpoint.clone();
            changed.logical_compressed_state_root[0] ^= 1;
            changed
        },
        {
            let mut changed = checkpoint.clone();
            changed.logical_compressed_state_count += 1;
            changed
        },
        {
            let mut changed = checkpoint.clone();
            changed.semantic_custody_accounting_root[0] ^= 1;
            changed
        },
        {
            let mut changed = checkpoint.clone();
            changed.external_metadata_observation_root[0] ^= 1;
            changed
        },
    ] {
        assert_ne!(
            compute_state_checkpoint_hard_combined_root_v1(&changed_hard_input).unwrap(),
            hard_root_baseline
        );
        assert_eq!(
            changed_hard_input.validate_schema(),
            Err(GovernanceError::Release1DigestMismatch)
        );
    }
    let mut changed_checkpoint = checkpoint.clone();
    changed_checkpoint.hard_combined_root[0] ^= 1;
    assert_ne!(
        compute_state_checkpoint_digest_v1(&changed_checkpoint).unwrap(),
        checkpoint_baseline
    );
    let mut mutable_checkpoint = checkpoint.clone();
    mutable_checkpoint.approval_council_version += 1;
    mutable_checkpoint.approval_council_hash[0] ^= 1;
    mutable_checkpoint.approval_bitset = 31;
    mutable_checkpoint.approval_count = 5;
    mutable_checkpoint.finalized_slot += 1;
    mutable_checkpoint.checkpoint_digest[0] ^= 1;
    assert_eq!(
        canonical_state_checkpoint_digest_material_v1(&mutable_checkpoint).unwrap(),
        canonical_state_checkpoint_digest_material_v1(&checkpoint).unwrap()
    );

    let rotation_baseline = compute_council_rotation_digest_v1(&rotation).unwrap();
    let mut changed_rotation = rotation.clone();
    changed_rotation.target_nonce += 1;
    assert_ne!(
        compute_council_rotation_digest_v1(&changed_rotation).unwrap(),
        rotation_baseline
    );
    let mut mutable_rotation = rotation.clone();
    mutable_rotation.state = CouncilRotationStateV1::Activated;
    mutable_rotation.approval_bitset = 31;
    mutable_rotation.approval_count = 5;
    mutable_rotation.activated_slot = 121;
    mutable_rotation.rotation_digest[0] ^= 1;
    assert_eq!(
        canonical_council_rotation_digest_material_v1(&mutable_rotation).unwrap(),
        canonical_council_rotation_digest_material_v1(&rotation).unwrap()
    );

    let resolution_baseline = compute_emergency_resolution_digest_v1(&resolution).unwrap();
    let mut changed_resolution = resolution.clone();
    changed_resolution.observed_raw_programdata_hash[0] ^= 1;
    assert_ne!(
        compute_emergency_resolution_digest_v1(&changed_resolution).unwrap(),
        resolution_baseline
    );
    let mut changed_resolution = resolution.clone();
    changed_resolution.observed_raw_hash_complete = false;
    assert_ne!(
        compute_emergency_resolution_digest_v1(&changed_resolution).unwrap(),
        resolution_baseline
    );
    let mut mutable_resolution = resolution.clone();
    mutable_resolution.state = EmergencyFreezeResolutionStateV1::Executed;
    mutable_resolution.approval_bitset = 31;
    mutable_resolution.approval_count = 5;
    mutable_resolution.executed_slot = 171;
    mutable_resolution.resolution_digest[0] ^= 1;
    assert_eq!(
        canonical_emergency_resolution_digest_material_v1(&mutable_resolution).unwrap(),
        canonical_emergency_resolution_digest_material_v1(&resolution).unwrap()
    );
}

#[test]
fn digest_mismatch_is_rejected_for_each_release1_subject() {
    let mut proposal = sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1);
    proposal.proposal_digest[0] ^= 1;
    assert_eq!(
        validate_proposal_digest_v2(&proposal),
        Err(GovernanceError::Release1DigestMismatch)
    );
    let mut checkpoint = sample_checkpoint();
    checkpoint.checkpoint_digest[0] ^= 1;
    assert_eq!(
        validate_state_checkpoint_digest_v1(&checkpoint),
        Err(GovernanceError::Release1DigestMismatch)
    );
    let mut rotation = sample_rotation();
    rotation.rotation_digest[0] ^= 1;
    assert_eq!(
        validate_council_rotation_digest_v1(&rotation),
        Err(GovernanceError::Release1DigestMismatch)
    );
    let mut resolution = sample_emergency_resolution();
    resolution.resolution_digest[0] ^= 1;
    assert_eq!(
        validate_emergency_resolution_digest_v1(&resolution),
        Err(GovernanceError::Release1DigestMismatch)
    );
    let mut freeze_observation = sample_emergency_freeze_observation();
    freeze_observation.observation_digest[0] ^= 1;
    assert_eq!(
        validate_emergency_freeze_observation_digest_v1(&freeze_observation),
        Err(GovernanceError::Release1DigestMismatch)
    );
    let mut failure_observation = sample_programdata_failure_observation();
    failure_observation.observation_digest[0] ^= 1;
    assert_eq!(
        validate_programdata_failure_observation_digest_v1(&failure_observation),
        Err(GovernanceError::Release1DigestMismatch)
    );
    let mut checkpoint_attestation = sample_checkpoint_attestation();
    checkpoint_attestation.attestation_digest[0] ^= 1;
    assert_eq!(
        validate_checkpoint_attestation_digest_v1(&checkpoint_attestation),
        Err(GovernanceError::Release1DigestMismatch)
    );
}

#[test]
fn merkle_scheme_id_selected_size_padding_and_proofs_are_exact() {
    assert_eq!(ARTIFACT_MERKLE_SCHEME_MATERIAL_V1.len(), 149);
    assert_eq!(
        hashv(&[ARTIFACT_MERKLE_SCHEME_MATERIAL_V1]).to_bytes(),
        ARTIFACT_MERKLE_SCHEME_ID
    );
    assert_eq!(
        BENCHMARK_ARTIFACT_CHUNK_SIZE_CANDIDATES_V1,
        [4_096, 8_192, 16_384]
    );
    assert_eq!(RELEASE1_ARTIFACT_CHUNK_SIZE_V1, 16_384);
    assert_eq!(MAX_ARTIFACT_BYTES_V1, 1_572_864);
    assert_eq!(MAX_ARTIFACT_CHUNKS_V1, 512);
    assert_eq!(MAX_SELECTED_ARTIFACT_CHUNKS_V1, 96);
    assert_eq!(MAX_PADDED_ARTIFACT_CHUNKS_V1, 128);
    assert_eq!(MAX_ARTIFACT_PROOF_DEPTH_V1, 7);

    let chunk_size = RELEASE1_ARTIFACT_CHUNK_SIZE_V1;
    let data = artifact(chunk_size as usize * 2 + 17);
    let root = artifact_merkle_root(&data, chunk_size).unwrap();
    assert_eq!(root, GOLDEN_ARTIFACT_MERKLE_ROOT_V1);
    for index in [0u32, 1, 2] {
        let start = index as usize * chunk_size as usize;
        let end = (start + chunk_size as usize).min(data.len());
        let proof = artifact_merkle_proof(&data, chunk_size, index).unwrap();
        assert_eq!(proof.len(), 2);
        assert_eq!(
            verify_artifact_chunk_proof(
                &root,
                data.len() as u64,
                chunk_size,
                index,
                &data[start..end],
                &proof,
            ),
            Ok(())
        );
    }

    let leaf0 = artifact_chunk_leaf_hash(0, &data[..chunk_size as usize]).unwrap();
    let leaf1 =
        artifact_chunk_leaf_hash(1, &data[chunk_size as usize..chunk_size as usize * 2]).unwrap();
    let leaf2 = artifact_chunk_leaf_hash(2, &data[chunk_size as usize * 2..]).unwrap();
    let empty3 = artifact_chunk_empty_hash(3).unwrap();
    let manual = artifact_chunk_node_hash(
        &artifact_chunk_node_hash(&leaf0, &leaf1),
        &artifact_chunk_node_hash(&leaf2, &empty3),
    );
    assert_eq!(root, manual);

    let last = &data[chunk_size as usize * 2..];
    let proof = artifact_merkle_proof(&data, chunk_size, 2).unwrap();
    let mut wrong_root = root;
    wrong_root[0] ^= 1;
    assert_eq!(
        verify_artifact_chunk_proof(&wrong_root, data.len() as u64, chunk_size, 2, last, &proof,),
        Err(GovernanceError::InvalidMerkleProof)
    );
    let mut wrong_proof = proof.clone();
    wrong_proof[0][0] ^= 1;
    assert_eq!(
        verify_artifact_chunk_proof(&root, data.len() as u64, chunk_size, 2, last, &wrong_proof,),
        Err(GovernanceError::InvalidMerkleProof)
    );
    assert_eq!(
        verify_artifact_chunk_proof(&root, data.len() as u64, chunk_size, 3, last, &proof,),
        Err(GovernanceError::InvalidMerkleProof)
    );
    assert_eq!(
        verify_artifact_chunk_proof(
            &root,
            data.len() as u64,
            chunk_size,
            2,
            &last[..last.len() - 1],
            &proof,
        ),
        Err(GovernanceError::InvalidMerkleProof)
    );
    assert_eq!(
        verify_artifact_chunk_proof(
            &root,
            data.len() as u64,
            chunk_size,
            2,
            last,
            &proof[..proof.len() - 1],
        ),
        Err(GovernanceError::InvalidMerkleProof)
    );

    let one_chunk = artifact(17);
    let root = artifact_merkle_root(&one_chunk, RELEASE1_ARTIFACT_CHUNK_SIZE_V1).unwrap();
    let proof = artifact_merkle_proof(&one_chunk, RELEASE1_ARTIFACT_CHUNK_SIZE_V1, 0).unwrap();
    assert!(proof.is_empty());
    assert_eq!(
        verify_artifact_chunk_proof(
            &root,
            one_chunk.len() as u64,
            RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
            0,
            &one_chunk,
            &proof,
        ),
        Ok(())
    );
}

#[test]
fn merkle_three_chunk_padding_golden_blocks_complete_finalization() {
    let chunk_size = RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as usize;
    let data = [
        vec![0x01; chunk_size],
        vec![0x02; chunk_size],
        vec![0x03; 7],
    ]
    .concat();
    let canonical_root = artifact_merkle_root(&data, RELEASE1_ARTIFACT_CHUNK_SIZE_V1).unwrap();
    assert_eq!(
        hex(&canonical_root),
        "5c25ae10679556cc57d6590fd44719c2a418ec3a58d6441bdefca444e0d596bd"
    );

    let leaf0 = artifact_chunk_leaf_hash(0, &data[..chunk_size]).unwrap();
    let leaf1 = artifact_chunk_leaf_hash(1, &data[chunk_size..chunk_size * 2]).unwrap();
    let leaf2 = artifact_chunk_leaf_hash(2, &data[chunk_size * 2..]).unwrap();
    let arbitrary_padding = [0xa5; 32];
    let adversarial_root = artifact_chunk_node_hash(
        &artifact_chunk_node_hash(&leaf0, &leaf1),
        &artifact_chunk_node_hash(&leaf2, &arbitrary_padding),
    );
    assert_eq!(
        hex(&adversarial_root),
        "b038867e7ff1557a45e02de6963f58483a1a722a2bde07d36a7da4b91869d98f"
    );

    let malicious_mixed_subtree = artifact_chunk_node_hash(&leaf2, &arbitrary_padding);
    let mut adversarial_outcomes = [false; 3];
    for index in 0..3u32 {
        let start = index as usize * chunk_size;
        let end = (start + chunk_size).min(data.len());
        let canonical_proof =
            artifact_merkle_proof(&data, RELEASE1_ARTIFACT_CHUNK_SIZE_V1, index).unwrap();
        assert_eq!(
            verify_artifact_chunk_proof(
                &canonical_root,
                data.len() as u64,
                RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
                index,
                &data[start..end],
                &canonical_proof,
            ),
            Ok(())
        );

        let mut adversarial_proof = canonical_proof;
        if index < 2 {
            // These proofs see one opaque sibling containing real chunk 2 and
            // padded leaf 3, so they cannot locally separate the padding.
            adversarial_proof[1] = malicious_mixed_subtree;
        } else {
            // The final real chunk exposes padded leaf 3 directly and must
            // enforce empty(3). Complete verification requires this chunk, so
            // the first two local successes cannot complete the bitmap.
            adversarial_proof[0] = arbitrary_padding;
        }
        adversarial_outcomes[index as usize] = verify_artifact_chunk_proof(
            &adversarial_root,
            data.len() as u64,
            RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
            index,
            &data[start..end],
            &adversarial_proof,
        )
        .is_ok();
    }
    assert_eq!(adversarial_outcomes, [true, true, false]);
}

#[test]
fn merkle_proofs_reject_arbitrary_multi_leaf_padding_subtrees() {
    let chunk_size = RELEASE1_ARTIFACT_CHUNK_SIZE_V1;
    let data = artifact(chunk_size as usize * 4 + 17);
    let root = artifact_merkle_root(&data, chunk_size).unwrap();
    let proof = artifact_merkle_proof(&data, chunk_size, 4).unwrap();
    let final_chunk = &data[chunk_size as usize * 4..];
    assert_eq!(proof.len(), 3);
    assert_eq!(
        verify_artifact_chunk_proof(&root, data.len() as u64, chunk_size, 4, final_chunk, &proof,),
        Ok(())
    );

    // At proof level 1, the sibling covers padded leaf indexes 6 and 7.  An
    // opaque arbitrary subtree used to be accepted when paired with its own
    // malicious root; the verifier must derive that subtree from empty(6/7).
    let arbitrary_padding_subtree = [0xa5; 32];
    let leaf4 = artifact_chunk_leaf_hash(4, final_chunk).unwrap();
    let node_4_5 = artifact_chunk_node_hash(&leaf4, &proof[0]);
    let node_4_7 = artifact_chunk_node_hash(&node_4_5, &arbitrary_padding_subtree);
    let malicious_root = artifact_chunk_node_hash(&proof[2], &node_4_7);
    assert_eq!(
        hex(&malicious_root),
        "df84137f11277170a460c500de78125c12b00bf39826e897df7a21f015439259"
    );
    let mut malicious_proof = proof;
    malicious_proof[1] = arbitrary_padding_subtree;
    assert_eq!(
        verify_artifact_chunk_proof(
            &malicious_root,
            data.len() as u64,
            chunk_size,
            4,
            final_chunk,
            &malicious_proof,
        ),
        Err(GovernanceError::InvalidMerkleProof)
    );

    // A 65-chunk artifact has the maximum seven-level proof and a final
    // partial right half. The last real chunk exposes canonical padding
    // siblings of 1, 2, 4, 8, 16, and 32 leaves.
    let max_depth_data = artifact(chunk_size as usize * 64 + 17);
    let max_depth_root = artifact_merkle_root(&max_depth_data, chunk_size).unwrap();
    let max_depth_chunk = &max_depth_data[chunk_size as usize * 64..];
    let max_depth_proof = artifact_merkle_proof(&max_depth_data, chunk_size, 64).unwrap();
    assert_eq!(max_depth_proof.len(), MAX_ARTIFACT_PROOF_DEPTH_V1);
    assert_eq!(
        verify_artifact_chunk_proof(
            &max_depth_root,
            max_depth_data.len() as u64,
            chunk_size,
            64,
            max_depth_chunk,
            &max_depth_proof,
        ),
        Ok(())
    );

    let mut malicious_max_depth_proof = max_depth_proof;
    malicious_max_depth_proof[5] = arbitrary_padding_subtree;
    let mut malicious_max_depth_root = artifact_chunk_leaf_hash(64, max_depth_chunk).unwrap();
    let mut index = 64u32;
    for sibling in &malicious_max_depth_proof {
        malicious_max_depth_root = if index & 1 == 0 {
            artifact_chunk_node_hash(&malicious_max_depth_root, sibling)
        } else {
            artifact_chunk_node_hash(sibling, &malicious_max_depth_root)
        };
        index >>= 1;
    }
    assert_eq!(
        verify_artifact_chunk_proof(
            &malicious_max_depth_root,
            max_depth_data.len() as u64,
            chunk_size,
            64,
            max_depth_chunk,
            &malicious_max_depth_proof,
        ),
        Err(GovernanceError::InvalidMerkleProof)
    );
}

#[test]
fn merkle_bounds_cover_the_one_and_a_half_mibibyte_worst_case() {
    assert_eq!(
        artifact_chunk_count(0, RELEASE1_ARTIFACT_CHUNK_SIZE_V1),
        Err(GovernanceError::InvalidMerkleParameters)
    );
    assert_eq!(
        artifact_chunk_count(MAX_ARTIFACT_BYTES_V1 + 1, RELEASE1_ARTIFACT_CHUNK_SIZE_V1,),
        Err(GovernanceError::InvalidMerkleParameters)
    );
    assert_eq!(
        artifact_chunk_count(1, 2_048),
        Err(GovernanceError::InvalidMerkleParameters)
    );
    assert_eq!(
        artifact_chunk_count(MAX_ARTIFACT_BYTES_V1, ARTIFACT_CHUNK_SIZE_4_KIB),
        Err(GovernanceError::InvalidMerkleParameters)
    );
    assert_eq!(
        artifact_chunk_count(MAX_ARTIFACT_BYTES_V1, ARTIFACT_CHUNK_SIZE_8_KIB),
        Err(GovernanceError::InvalidMerkleParameters)
    );
    assert_eq!(
        artifact_chunk_count(MAX_ARTIFACT_BYTES_V1, ARTIFACT_CHUNK_SIZE_16_KIB).unwrap(),
        96
    );
    assert_eq!(
        artifact_chunk_leaf_hash(96, &[1]),
        Err(GovernanceError::InvalidMerkleParameters)
    );

    let data = vec![0x5a; MAX_ARTIFACT_BYTES_V1 as usize];
    let root = artifact_merkle_root(&data, RELEASE1_ARTIFACT_CHUNK_SIZE_V1).unwrap();
    let proof = artifact_merkle_proof(&data, RELEASE1_ARTIFACT_CHUNK_SIZE_V1, 95).unwrap();
    assert_eq!(proof.len(), MAX_ARTIFACT_PROOF_DEPTH_V1);
    assert!(artifact_chunk_empty_hash(127).is_ok());
    assert_eq!(
        artifact_chunk_empty_hash(128),
        Err(GovernanceError::InvalidMerkleParameters)
    );
    let start = data.len() - RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as usize;
    assert_eq!(
        verify_artifact_chunk_proof(
            &root,
            data.len() as u64,
            RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
            95,
            &data[start..],
            &proof,
        ),
        Ok(())
    );
}

#[test]
fn new_pdas_are_deterministic_separated_and_bind_numeric_seeds() {
    let controller = synthetic_controller_program();
    let target = ameba_spread_program();
    let proposal = release1_proposal_key();
    let checkpoint = derive_checkpoint_pda(&controller, &proposal, CheckpointPhaseV1::Prestate).0;
    let addresses = [
        derive_upgradeable_programdata_address(&target).0,
        derive_controller_config_pda(&controller, &target).0,
        derive_authority_pda(&controller, &target).0,
        derive_gate_pda(&controller, &target).0,
        derive_policy_pda(&controller, &target, 1).0,
        derive_council_pda(&controller, &target, 1).0,
        proposal,
        derive_programdata_check_pda(&controller, &proposal).0,
        derive_checkpoint_pda(&controller, &proposal, CheckpointPhaseV1::Prestate).0,
        derive_checkpoint_pda(&controller, &proposal, CheckpointPhaseV1::Poststate).0,
        derive_emergency_resolution_pda(&controller, &target, 7).0,
        derive_emergency_checkpoint_pda(&controller, &target, 7).0,
        derive_council_rotation_pda(&controller, &target, 8).0,
        derive_emergency_freeze_observation_pda(&controller, &target, 7).0,
        derive_programdata_failure_observation_pda(&controller, &proposal, 7).0,
        derive_checkpoint_attestation_pda(&controller, &checkpoint, 2, 2).0,
    ];
    for (index, address) in addresses.iter().enumerate() {
        assert!(addresses[..index].iter().all(|prior| prior != address));
    }
    assert_ne!(
        derive_emergency_resolution_pda(&controller, &target, 7).0,
        derive_emergency_resolution_pda(&controller, &target, 8).0
    );
    assert_ne!(
        derive_council_rotation_pda(&controller, &target, 8).0,
        derive_council_rotation_pda(&controller, &target, 8u64.swap_bytes()).0
    );
    let baseline = derive_checkpoint_attestation_pda(&controller, &checkpoint, 2, 2).0;
    for seat_index in 0..5 {
        let address = derive_checkpoint_attestation_pda(&controller, &checkpoint, 2, seat_index).0;
        if seat_index != 2 {
            assert_ne!(address, baseline);
        }
    }
    assert_ne!(
        baseline,
        derive_checkpoint_attestation_pda(&controller, &checkpoint, 3, 2).0
    );
}

#[test]
fn release1_identity_graph_exactly_reuses_the_phase3_bridge() {
    let bridge: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(phase3_bridge_fixture_path()).expect("read Phase 3 bridge fixture"),
    )
    .expect("parse Phase 3 bridge fixture");
    let release1: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(release1_fixture_path()).expect("read Release 1 fixture"),
    )
    .expect("parse Release 1 fixture");

    let controller = synthetic_controller_program();
    let target = ameba_spread_program();
    let proposal = release1_proposal_key();
    assert_eq!(
        controller.to_string(),
        "4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi"
    );
    assert_eq!(
        target.to_string(),
        "9ipkBCjEfeJDMF6AFrezRmDDHmbnmeyv45cfXNqAnWsH"
    );
    assert_eq!(
        bridge["controllerProgram"].as_str(),
        Some(controller.to_string().as_str())
    );
    assert_eq!(
        bridge["targetProgram"].as_str(),
        Some(target.to_string().as_str())
    );
    assert_eq!(
        release1["pda_inputs"]["controller_program"].as_str(),
        Some(controller.to_string().as_str())
    );
    assert_eq!(
        release1["pda_inputs"]["target_program"].as_str(),
        Some(target.to_string().as_str())
    );

    let canonical = [
        (
            "target_programdata",
            derive_upgradeable_programdata_address(&target),
        ),
        (
            "controller_config",
            derive_controller_config_pda(&controller, &target),
        ),
        ("authority", derive_authority_pda(&controller, &target)),
        ("protocol_gate", derive_gate_pda(&controller, &target)),
        ("policy_v1", derive_policy_pda(&controller, &target, 1)),
        ("council_v1", derive_council_pda(&controller, &target, 1)),
        ("council_v2", derive_council_pda(&controller, &target, 2)),
        ("proposal_v2", derive_proposal_pda(&controller, &target, 7)),
        (
            "buffer_verification",
            derive_buffer_check_pda(&controller, &proposal),
        ),
        (
            "programdata_verification",
            derive_programdata_check_pda(&controller, &proposal),
        ),
        (
            "prestate_checkpoint",
            derive_checkpoint_pda(&controller, &proposal, CheckpointPhaseV1::Prestate),
        ),
        (
            "poststate_checkpoint",
            derive_checkpoint_pda(&controller, &proposal, CheckpointPhaseV1::Poststate),
        ),
    ];
    for (name, (address, bump)) in canonical {
        assert_eq!(
            release1["pdas"][name]["address"].as_str(),
            Some(address.to_string().as_str()),
            "{name}"
        );
        assert_eq!(
            release1["pdas"][name]["bump"].as_u64(),
            Some(u64::from(bump)),
            "{name}"
        );
    }
    assert_eq!(
        bridge["targetProgramdata"]["address"].as_str(),
        release1["pdas"]["target_programdata"]["address"].as_str()
    );
    assert_eq!(
        bridge["pdas"]["controllerConfig"]["address"].as_str(),
        release1["pdas"]["controller_config"]["address"].as_str()
    );
    assert_eq!(
        bridge["pdas"]["protocolGate"]["address"].as_str(),
        release1["pdas"]["protocol_gate"]["address"].as_str()
    );
}

#[test]
fn v1_abi_and_digest_constants_remain_frozen() {
    assert_eq!(CONTROLLER_CONFIG_RESERVED_LEN, 28);
    assert_eq!(GOVERNANCE_POLICY_RESERVED_LEN, 21);
    assert_eq!(GOVERNANCE_COUNCIL_RESERVED_LEN, 26);
    assert_eq!(PROTOCOL_GATE_RESERVED_LEN, 2);
    assert_eq!(UPGRADE_PROPOSAL_RESERVED_LEN, 142);
    assert_eq!(PROPOSAL_DIGEST_DOMAIN_V1, b"AMOEBA_UPGRADE_PROPOSAL_V1");
    assert_eq!(PROPOSAL_DIGEST_MATERIAL_LEN, 1_084);
    assert_eq!(PROPOSAL_DIGEST_PREIMAGE_LEN, 1_110);
}

#[test]
fn shared_release1_json_fixture_is_consumed_by_rust_and_cannot_drift() {
    let fixture_text = fs::read_to_string(release1_fixture_path()).expect("read Release 1 fixture");
    let fixture: serde_json::Value =
        serde_json::from_str(&fixture_text).expect("parse Release 1 fixture");
    assert_eq!(fixture["fixture_version"].as_u64(), Some(1));
    assert!(fixture["warning"]
        .as_str()
        .expect("fixture warning")
        .contains("SYNTHETIC NON-PRODUCTION"));

    let fixture_sha256 = fixture["fixture_sha256"].as_str().expect("fixture SHA-256");
    let hash_field = format!("\"fixture_sha256\": \"{fixture_sha256}\"");
    let zeroed_hash_field = format!("\"fixture_sha256\": \"{}\"", "0".repeat(64));
    let canonical_zeroed = fixture_text.replacen(&hash_field, &zeroed_hash_field, 1);
    assert_ne!(canonical_zeroed, fixture_text);
    assert_eq!(
        hex(&hashv(&[canonical_zeroed.as_bytes()]).to_bytes()),
        fixture_sha256
    );

    let account_encodings = [
        (
            "upgrade_proposal_v2",
            sample_proposal(RELEASE1_ARTIFACT_CHUNK_SIZE_V1)
                .try_to_vec()
                .unwrap(),
        ),
        (
            "buffer_verification_v1",
            sample_buffer_verification().try_to_vec().unwrap(),
        ),
        (
            "programdata_verification_v1",
            sample_programdata_verification().try_to_vec().unwrap(),
        ),
        (
            "state_checkpoint_v1",
            sample_checkpoint().try_to_vec().unwrap(),
        ),
        (
            "council_rotation_proposal_v1",
            sample_rotation().try_to_vec().unwrap(),
        ),
        (
            "emergency_freeze_resolution_v1",
            sample_emergency_resolution().try_to_vec().unwrap(),
        ),
        (
            "emergency_freeze_observation_v1",
            sample_emergency_freeze_observation().try_to_vec().unwrap(),
        ),
        (
            "programdata_failure_observation_v1",
            sample_programdata_failure_observation()
                .try_to_vec()
                .unwrap(),
        ),
        (
            "checkpoint_attestation_v1",
            sample_checkpoint_attestation().try_to_vec().unwrap(),
        ),
    ];
    for (name, encoded) in account_encodings {
        let vector = &fixture["accounts"][name];
        assert_eq!(vector["len"].as_u64(), Some(encoded.len() as u64), "{name}");
        assert_eq!(
            vector["encoded_hex"].as_str(),
            Some(hex(&encoded).as_str()),
            "{name}"
        );
    }

    let digest_vectors = [
        ("upgrade_proposal_v2", GOLDEN_PROPOSAL_DIGEST_V2),
        ("state_checkpoint_v1", GOLDEN_CHECKPOINT_DIGEST_V1),
        ("council_rotation_proposal_v1", GOLDEN_ROTATION_DIGEST_V1),
        ("emergency_freeze_resolution_v1", GOLDEN_EMERGENCY_DIGEST_V1),
        (
            "emergency_freeze_observation_v1",
            GOLDEN_FREEZE_OBSERVATION_DIGEST_V1,
        ),
        (
            "programdata_failure_observation_v1",
            GOLDEN_PROGRAMDATA_FAILURE_DIGEST_V1,
        ),
        (
            "checkpoint_attestation_v1",
            GOLDEN_CHECKPOINT_ATTESTATION_DIGEST_V1,
        ),
    ];
    for (name, digest) in digest_vectors {
        assert_eq!(
            fixture["accounts"][name]["digest"]["sha256_hex"].as_str(),
            Some(hex(&digest).as_str()),
            "{name}"
        );
    }
    let checkpoint = sample_checkpoint();
    let hard_root_vector = &fixture["checkpoint_hard_combined_root"];
    assert_eq!(
        hard_root_vector["domain_ascii"].as_str(),
        Some(std::str::from_utf8(STATE_CHECKPOINT_HARD_ROOT_DOMAIN_V1).unwrap())
    );
    assert_eq!(
        hard_root_vector["material_length"].as_u64(),
        Some(STATE_CHECKPOINT_HARD_ROOT_MATERIAL_LEN_V1 as u64)
    );
    assert_eq!(
        hard_root_vector["preimage_length"].as_u64(),
        Some(STATE_CHECKPOINT_HARD_ROOT_PREIMAGE_LEN_V1 as u64)
    );
    assert_eq!(
        hard_root_vector["material_hex"].as_str(),
        Some(hex(&canonical_state_checkpoint_hard_root_material_v1(&checkpoint).unwrap()).as_str())
    );
    assert_eq!(
        hard_root_vector["sha256_hex"].as_str(),
        Some(hex(&checkpoint.hard_combined_root).as_str())
    );
    assert_eq!(
        fixture["merkle"]["root_sha256_hex"].as_str(),
        Some(hex(&GOLDEN_ARTIFACT_MERKLE_ROOT_V1).as_str())
    );

    let v1_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("upgrade_governance_v1.json");
    let v1_bytes = fs::read(v1_path).expect("read V1 fixture");
    assert_eq!(
        fixture["v1_regression"]["fixture_sha256"].as_str(),
        Some(hex(&hashv(&[&v1_bytes]).to_bytes()).as_str())
    );
}
