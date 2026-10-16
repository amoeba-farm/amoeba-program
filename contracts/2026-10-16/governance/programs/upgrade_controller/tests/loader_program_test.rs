use borsh::{BorshDeserialize, BorshSerialize};
use solana_loader_v3_interface::instruction::{
    set_upgrade_authority, upgrade as direct_loader_upgrade,
};
use solana_program::{hash::hashv, instruction::Instruction, pubkey::Pubkey, rent::Rent};
use solana_program_test::{processor, BanksClientError, ProgramTest, ProgramTestContext};
use solana_sdk::{
    account::{Account, AccountSharedData},
    compute_budget::ComputeBudgetInstruction,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use solana_sdk_ids::{system_program, sysvar as sysvar_ids};
use upgrade_controller::{
    artifact_merkle::{
        artifact_chunk_count, artifact_chunk_leaf_hash, artifact_merkle_proof,
        artifact_merkle_root, ARTIFACT_MERKLE_SCHEME_ID, RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
    },
    council::{compute_council_set_hash, validate_council_set},
    instruction::{
        activate_rollback_v1_instruction, adopt_buffer_v1_instruction,
        approve_proposal_v2_instruction, approve_unfreeze_v1_instruction,
        close_abandoned_buffer_v1_instruction, create_checkpoint_attestation_v1_instruction,
        create_proposal_v2_instruction, execute_unfreeze_v1_instruction,
        execute_upgrade_v1_instruction, extend_target_v1_instruction,
        finalize_buffer_verification_v1_instruction, finalize_checkpoint_v1_instruction,
        finalize_governance_v2_instruction, finalize_programdata_verification_v1_instruction,
        freeze_proposal_v2_instruction, initialize_controller_v1_instruction,
        observe_programdata_failure_v1_instruction, queue_proposal_v2_instruction,
        verify_buffer_chunk_v1_instruction, verify_programdata_chunk_v1_instruction,
        ActivateRollbackV1, ActivateRollbackV1Accounts, AdoptBufferV1, AdoptBufferV1Accounts,
        ApproveProposalV2, ApproveProposalV2Accounts, ApproveUnfreezeV1, ApproveUnfreezeV1Accounts,
        CheckpointCandidateV1, CheckpointSubjectStateV1, CloseAbandonedBufferV1,
        CloseAbandonedBufferV1Accounts, CouncilSeatTermV1, CreateCheckpointAttestationV1,
        CreateCheckpointAttestationV1Accounts, CreateProposalV2, CreateProposalV2Accounts,
        EnvelopeExpectationV1, ExecuteUnfreezeV1, ExecuteUnfreezeV1Accounts, ExecuteUpgradeV1,
        ExecuteUpgradeV1Accounts, ExtendTargetV1, ExtendTargetV1Accounts,
        FinalizeBufferVerificationV1, FinalizeBufferVerificationV1Accounts, FinalizeCheckpointV1,
        FinalizeCheckpointV1Accounts, FinalizeGovernanceV2, FinalizeGovernanceV2Accounts,
        FinalizeProgramDataVerificationV1, FinalizeProgramDataVerificationV1Accounts,
        FixedMerkleProofV1, FreezeProposalV2, FreezeProposalV2Accounts, InitializeControllerV1,
        InitializeControllerV1Accounts, ObserveProgramDataFailureV1,
        ObserveProgramDataFailureV1Accounts, OptionalInstructionPubkeyV1, ProgramDataChunkPhaseV1,
        ProposalExpectationV2, QueueProposalV2, QueueProposalV2Accounts, UnfreezeExpectationV1,
        VerifyBufferChunkV1, VerifyBufferChunkV1Accounts, VerifyProgramDataChunkV1,
        VerifyProgramDataChunkV1Accounts,
    },
    pda::{
        derive_authority_pda, derive_buffer_check_pda, derive_checkpoint_attestation_pda,
        derive_checkpoint_pda, derive_controller_config_pda, derive_council_pda, derive_gate_pda,
        derive_policy_pda, derive_programdata_check_pda,
        derive_programdata_failure_observation_pda, derive_proposal_pda,
        derive_upgradeable_programdata_address, UPGRADEABLE_LOADER_ID,
    },
    policy::compute_policy_hash,
    processor::process_instruction,
    release1_digest::{
        compute_programdata_failure_observation_digest_v1, compute_proposal_digest_v2,
        compute_state_checkpoint_digest_v1, compute_state_checkpoint_hard_combined_root_v1,
    },
    release1_loader_accounts::{
        loader_account_data_hash, parse_upgradeable_buffer, parse_upgradeable_programdata,
        LOADER_BUFFER_METADATA_LEN, LOADER_PROGRAMDATA_METADATA_LEN, LOADER_PROGRAM_ACCOUNT_LEN,
        LOADER_STATE_TAG_BUFFER, LOADER_STATE_TAG_PROGRAM, LOADER_STATE_TAG_PROGRAMDATA,
    },
    release1_model::{
        ModelDelays, ModelHardStateObservation, ModelIdentityGraph, ModelInitialization,
        ModelProgramDataObservation, ModelProposalRequest, ModelSeatTerm, Release1Model,
        Release1ModelAction, Release1ModelOutcome,
    },
    release1_processor_proposal::GOVERNED_UPGRADE_FREEZE_REASON_V1,
    release1_state::{
        BufferVerificationStatusV1, BufferVerificationV1, ProgramDataFailureObservationV1,
        ProgramDataMismatchClassV1, ProgramDataVerificationStatusV1, ProgramDataVerificationV1,
        ProposalStateV2, StateCheckpointPhaseV1, StateCheckpointV1, UpgradeProposalV2,
        ACCOUNT_VERSION_V2, BUFFER_VERIFICATION_V1_DISCRIMINATOR,
        BUFFER_VERIFICATION_V1_RESERVED_LEN, PROGRAMDATA_FAILURE_OBSERVATION_V1_DISCRIMINATOR,
        PROGRAMDATA_FAILURE_OBSERVATION_V1_RESERVED_LEN, PROGRAMDATA_VERIFICATION_V1_DISCRIMINATOR,
        PROGRAMDATA_VERIFICATION_V1_RESERVED_LEN, RELEASE1_ACCOUNT_VERSION_V1,
        STATE_CHECKPOINT_V1_DISCRIMINATOR, STATE_CHECKPOINT_V1_RESERVED_LEN,
        UPGRADE_PROPOSAL_V2_DISCRIMINATOR, UPGRADE_PROPOSAL_V2_RESERVED_LEN,
        VERIFICATION_BITMAP_BYTES_V1,
    },
    state::{
        CheckpointPhaseV1, ControllerConfigV1, CouncilSeatV1, GateStatusV1, GovernanceCouncilSetV1,
        GovernanceModeV1, GovernancePolicyV1, OptionalPubkeyV1, ProposalClassV1, ProtocolGateV1,
        VoteRequirementV1, ACCOUNT_VERSION_V1, CONTROLLER_CONFIG_DISCRIMINATOR,
        CONTROLLER_CONFIG_RESERVED_LEN, GOVERNANCE_COUNCIL_DISCRIMINATOR,
        GOVERNANCE_COUNCIL_RESERVED_LEN, GOVERNANCE_POLICY_DISCRIMINATOR,
        GOVERNANCE_POLICY_RESERVED_LEN, PROTOCOL_GATE_DISCRIMINATOR, PROTOCOL_GATE_RESERVED_LEN,
    },
};

const TEST_SLOT: u64 = 50;
const INITIAL_PROGRAMDATA_SLOT: u64 = 1;
const ROLLBACK_ARTIFACT: &[u8] = b"release-1-local-rollback";

fn read_attested_loader_artifact() -> Vec<u8> {
    use std::fmt::Write as _;

    let artifact_path = std::env::var("AMOEBA_LOADER_TEST_ARTIFACT")
        .expect("set AMOEBA_LOADER_TEST_ARTIFACT to the exact local SBF ELF");
    let expected_sha256 = std::env::var("AMOEBA_LOADER_TEST_ARTIFACT_SHA256")
        .expect("set AMOEBA_LOADER_TEST_ARTIFACT_SHA256 to the exact lowercase SHA-256");
    let artifact = std::fs::read(&artifact_path).expect("read local SBF artifact");
    assert!(artifact.starts_with(b"\x7fELF"), "artifact must be an ELF");
    let mut actual_sha256 = String::with_capacity(64);
    for byte in hashv(&[&artifact]).to_bytes() {
        write!(&mut actual_sha256, "{byte:02x}").expect("format SHA-256");
    }
    assert_eq!(
        actual_sha256,
        expected_sha256.to_ascii_lowercase(),
        "artifact SHA-256 must match the attested CI input"
    );
    artifact
}

struct BufferHarness {
    controller: Pubkey,
    target: Pubkey,
    target_programdata: Pubkey,
    config: Pubkey,
    gate: Pubkey,
    authority: Pubkey,
    spill: Pubkey,
    proposal: Pubkey,
    council: Pubkey,
    verification: Pubkey,
    buffer: Pubkey,
    uploader: Keypair,
    seats: [Keypair; 5],
    artifact: Vec<u8>,
    current_payload: Vec<u8>,
    rollback_artifact: Vec<u8>,
}

fn account(owner: Pubkey, data: Vec<u8>, executable: bool) -> Account {
    Account {
        lamports: Rent::default().minimum_balance(data.len()).max(1),
        data,
        owner,
        executable,
        rent_epoch: 0,
    }
}

fn state_account<T: BorshSerialize>(owner: Pubkey, value: &T) -> Account {
    account(
        owner,
        value.try_to_vec().expect("fixed state encoding"),
        false,
    )
}

fn buffer_bytes(authority: Pubkey, payload: &[u8]) -> Vec<u8> {
    let mut data = vec![0; LOADER_BUFFER_METADATA_LEN + payload.len()];
    data[..4].copy_from_slice(&LOADER_STATE_TAG_BUFFER.to_le_bytes());
    data[4] = 1;
    data[5..37].copy_from_slice(authority.as_ref());
    data[LOADER_BUFFER_METADATA_LEN..].copy_from_slice(payload);
    data
}

fn program_bytes(programdata: Pubkey) -> Vec<u8> {
    let mut data = vec![0; LOADER_PROGRAM_ACCOUNT_LEN];
    data[..4].copy_from_slice(&LOADER_STATE_TAG_PROGRAM.to_le_bytes());
    data[4..].copy_from_slice(programdata.as_ref());
    data
}

fn programdata_bytes(slot: u64, authority: Pubkey, payload: &[u8]) -> Vec<u8> {
    let mut data = vec![0; LOADER_PROGRAMDATA_METADATA_LEN + payload.len()];
    data[..4].copy_from_slice(&LOADER_STATE_TAG_PROGRAMDATA.to_le_bytes());
    data[4..12].copy_from_slice(&slot.to_le_bytes());
    data[12] = 1;
    data[13..45].copy_from_slice(authority.as_ref());
    data[LOADER_PROGRAMDATA_METADATA_LEN..].copy_from_slice(payload);
    data
}

fn config(
    controller: Pubkey,
    target: Pubkey,
    target_programdata: Pubkey,
    spill: Pubkey,
    guardian: Pubkey,
) -> ControllerConfigV1 {
    let (_, bump) = derive_controller_config_pda(&controller, &target);
    ControllerConfigV1 {
        discriminator: CONTROLLER_CONFIG_DISCRIMINATOR,
        version: ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        cluster_domain: [1; 32],
        target_program: target,
        target_programdata,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        authority_pda: derive_authority_pda(&controller, &target).0,
        gate_pda: derive_gate_pda(&controller, &target).0,
        canonical_spill_treasury: spill,
        current_council_version: 1,
        current_policy_version: 1,
        next_proposal_id: 2,
        target_nonce: 1,
        guardian,
        vote_program: Pubkey::default(),
        vote_programdata: Pubkey::default(),
        vote_config: Pubkey::default(),
        vote_mint: Pubkey::default(),
        token_governance_enabled: false,
        routine_delay_slots: 5,
        major_delay_slots: 8,
        rollback_delay_slots: 2,
        terminal_delay_slots: 10,
        vote_review_slots: 8,
        proposal_expiry_slots: 100,
        policy_flags: 0,
        reserved: [0; CONTROLLER_CONFIG_RESERVED_LEN],
    }
}

fn governance_policy(
    controller: Pubkey,
    controller_config: Pubkey,
    target: Pubkey,
) -> GovernancePolicyV1 {
    let mut policy = GovernancePolicyV1 {
        discriminator: GOVERNANCE_POLICY_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump: upgrade_controller::pda::derive_policy_pda(&controller, &target, 1).1,
        initialized: true,
        controller_config,
        version: 1,
        target_program: target,
        activation_slot: 1,
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
        policy_hash: [0; 32],
        reserved: [0; GOVERNANCE_POLICY_RESERVED_LEN],
    };
    policy.policy_hash = compute_policy_hash(&policy);
    policy
}

fn governance_council(
    controller: Pubkey,
    controller_config: Pubkey,
    target: Pubkey,
    policy: &GovernancePolicyV1,
    seats: &[Keypair; 5],
) -> (Pubkey, GovernanceCouncilSetV1) {
    let (council_key, bump) = derive_council_pda(&controller, &target, 1);
    let mut council = GovernanceCouncilSetV1 {
        discriminator: GOVERNANCE_COUNCIL_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        controller_config,
        version: 1,
        target_program: target,
        activation_slot: 1,
        deactivation_slot: 0,
        seats: std::array::from_fn(|index| CouncilSeatV1 {
            seat_authority: seats[index].pubkey(),
            term_start_slot: 1,
            term_end_slot: 10_000,
            active: true,
            reserved: [0; upgrade_controller::state::COUNCIL_SEAT_RESERVED_LEN],
        }),
        routine_threshold: 3,
        terminal_threshold: 4,
        policy_flags: 0,
        set_hash: [0; 32],
        reserved: [0; GOVERNANCE_COUNCIL_RESERVED_LEN],
    };
    council.set_hash = compute_council_set_hash(&council);
    validate_council_set(&council, policy).expect("valid five-seat test council");
    (council_key, council)
}

fn active_gate(
    controller: Pubkey,
    config: Pubkey,
    target: Pubkey,
    target_programdata: Pubkey,
) -> ProtocolGateV1 {
    ProtocolGateV1 {
        discriminator: PROTOCOL_GATE_DISCRIMINATOR,
        version: ACCOUNT_VERSION_V1,
        bump: derive_gate_pda(&controller, &target).1,
        initialized: true,
        status: GateStatusV1::Active,
        controller_config: config,
        target_program: target,
        target_programdata,
        epoch: 2,
        active_proposal: Pubkey::default(),
        freeze_slot: 0,
        freeze_reason_code: 0,
        last_completed_proposal: Pubkey::default(),
        reserved: [0; PROTOCOL_GATE_RESERVED_LEN],
    }
}

#[allow(clippy::too_many_arguments)]
fn draft_proposal(
    controller: Pubkey,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    buffer: Pubkey,
    uploader: Pubkey,
    artifact: &[u8],
    current_payload: &[u8],
    extension_delta: u64,
    creation_council_hash: [u8; 32],
    rollback_buffer: Pubkey,
    rollback_artifact: &[u8],
) -> (Pubkey, UpgradeProposalV2) {
    let proposal_id = 1;
    let (proposal_key, bump) =
        derive_proposal_pda(&controller, &config.target_program, proposal_id);
    let creation_slot = TEST_SLOT;
    let review_start_slot = creation_slot + 1;
    let review_end_slot = review_start_slot + config.vote_review_slots;
    let mut proposal = UpgradeProposalV2 {
        discriminator: UPGRADE_PROPOSAL_V2_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V2,
        bump,
        initialized: true,
        proposal_class: ProposalClassV1::RoutineUpgrade,
        state: ProposalStateV2::Draft,
        creation_gate_status: gate.status,
        zero_tail_required: true,
        proposal_flags: 0,
        proposal_id,
        target_nonce: config.target_nonce,
        creation_slot,
        cluster_domain: config.cluster_domain,
        controller_program: controller,
        controller_config: derive_controller_config_pda(&controller, &config.target_program).0,
        protocol_gate: config.gate_pda,
        policy_version: config.current_policy_version,
        policy_hash: governance_policy(
            controller,
            derive_controller_config_pda(&controller, &config.target_program).0,
            config.target_program,
        )
        .policy_hash,
        creation_council_version: config.current_council_version,
        creation_council_hash,
        creation_gate_epoch: gate.epoch,
        freeze_gate_epoch: 0,
        target_program: config.target_program,
        target_programdata: config.target_programdata,
        upgradeable_loader: config.upgradeable_loader,
        authority_pda: config.authority_pda,
        canonical_spill_treasury: config.canonical_spill_treasury,
        buffer_pubkey: buffer,
        buffer_loader_owner: UPGRADEABLE_LOADER_ID,
        buffer_uploader_authority: uploader,
        buffer_final_authority: config.authority_pda,
        buffer_verification: derive_buffer_check_pda(&controller, &proposal_key).0,
        programdata_verification: derive_programdata_check_pda(&controller, &proposal_key).0,
        artifact_length: artifact.len() as u64,
        artifact_sha256: hashv(&[artifact]).to_bytes(),
        artifact_chunk_merkle_root: artifact_merkle_root(artifact, RELEASE1_ARTIFACT_CHUNK_SIZE_V1)
            .expect("artifact root"),
        chunk_hash_domain: ARTIFACT_MERKLE_SCHEME_ID,
        chunk_size: RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        chunk_count: artifact_chunk_count(artifact.len() as u64, RELEASE1_ARTIFACT_CHUNK_SIZE_V1)
            .expect("chunk count"),
        source_commit_hash: [10; 32],
        source_tree_hash: [11; 32],
        build_input_inventory_hash: [12; 32],
        reproducible_build_receipt_hash: [13; 32],
        package_receipt_hash: [14; 32],
        release_intent_hash: [15; 32],
        expected_execution_pre_payload_hash: hashv(&[current_payload]).to_bytes(),
        expected_execution_pre_chunk_root: artifact_merkle_root(
            current_payload,
            RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        )
        .expect("current payload root"),
        current_raw_programdata_hash: loader_account_data_hash(&programdata_bytes(
            INITIAL_PROGRAMDATA_SLOT,
            config.authority_pda,
            current_payload,
        )),
        deployed_slot: INITIAL_PROGRAMDATA_SLOT,
        current_capacity: current_payload.len() as u64,
        extension_delta,
        expected_post_capacity: current_payload.len() as u64 + extension_delta,
        prestate_checkpoint: derive_checkpoint_pda(
            &controller,
            &proposal_key,
            CheckpointPhaseV1::Prestate,
        )
        .0,
        required_poststate_checkpoint: derive_checkpoint_pda(
            &controller,
            &proposal_key,
            CheckpointPhaseV1::Poststate,
        )
        .0,
        checkpoint_schema_id: [20; 32],
        checkpoint_policy_hash: [21; 32],
        primary_proposal: OptionalPubkeyV1::none(),
        rollback_proposal: OptionalPubkeyV1::some(
            derive_proposal_pda(&controller, &config.target_program, 2).0,
        )
        .expect("rollback"),
        rollback_buffer: OptionalPubkeyV1::some(rollback_buffer).expect("rollback buffer"),
        rollback_artifact_sha256: hashv(&[rollback_artifact]).to_bytes(),
        rollback_artifact_chunk_root: artifact_merkle_root(
            rollback_artifact,
            RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        )
        .expect("rollback root"),
        vote_requirement: VoteRequirementV1::None,
        vote_program: Pubkey::default(),
        vote_result_pda: Pubkey::default(),
        review_start_slot,
        review_end_slot,
        not_before_slot: review_end_slot + config.routine_delay_slots,
        expiry_slot: creation_slot + config.proposal_expiry_slots,
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
        reserved: [0; UPGRADE_PROPOSAL_V2_RESERVED_LEN],
    };
    proposal.proposal_digest = compute_proposal_digest_v2(&proposal).expect("proposal digest");
    (proposal_key, proposal)
}

#[allow(clippy::too_many_arguments)]
fn draft_rollback_proposal(
    controller: Pubkey,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    primary_key: Pubkey,
    primary: &UpgradeProposalV2,
    buffer: Pubkey,
    uploader: Pubkey,
    artifact: &[u8],
    creation_slot: u64,
    creation_council_hash: [u8; 32],
) -> (Pubkey, UpgradeProposalV2) {
    let proposal_id = config.next_proposal_id;
    let (proposal_key, bump) =
        derive_proposal_pda(&controller, &config.target_program, proposal_id);
    let review_start_slot = creation_slot + 1;
    let review_end_slot = review_start_slot + config.vote_review_slots;
    let mut proposal = primary.clone();
    proposal.bump = bump;
    proposal.proposal_class = ProposalClassV1::EmergencyRollback;
    proposal.state = ProposalStateV2::Draft;
    proposal.creation_gate_status = gate.status;
    proposal.proposal_id = proposal_id;
    proposal.target_nonce = config.target_nonce;
    proposal.creation_slot = creation_slot;
    proposal.creation_council_version = config.current_council_version;
    proposal.creation_council_hash = creation_council_hash;
    proposal.creation_gate_epoch = gate.epoch;
    proposal.freeze_gate_epoch = 0;
    proposal.buffer_pubkey = buffer;
    proposal.buffer_uploader_authority = uploader;
    proposal.buffer_verification = derive_buffer_check_pda(&controller, &proposal_key).0;
    proposal.programdata_verification = derive_programdata_check_pda(&controller, &proposal_key).0;
    proposal.artifact_length = artifact.len() as u64;
    proposal.artifact_sha256 = hashv(&[artifact]).to_bytes();
    proposal.artifact_chunk_merkle_root =
        artifact_merkle_root(artifact, RELEASE1_ARTIFACT_CHUNK_SIZE_V1)
            .expect("rollback artifact root");
    proposal.chunk_count =
        artifact_chunk_count(artifact.len() as u64, RELEASE1_ARTIFACT_CHUNK_SIZE_V1)
            .expect("rollback chunk count");
    proposal.expected_execution_pre_payload_hash = primary.artifact_sha256;
    proposal.expected_execution_pre_chunk_root = primary.artifact_chunk_merkle_root;
    proposal.current_raw_programdata_hash = [0; 32];
    proposal.deployed_slot = 0;
    proposal.current_capacity = primary.expected_post_capacity;
    proposal.extension_delta = 0;
    proposal.expected_post_capacity = primary.expected_post_capacity;
    proposal.prestate_checkpoint =
        derive_checkpoint_pda(&controller, &proposal_key, CheckpointPhaseV1::Prestate).0;
    proposal.required_poststate_checkpoint =
        derive_checkpoint_pda(&controller, &proposal_key, CheckpointPhaseV1::Poststate).0;
    proposal.primary_proposal = OptionalPubkeyV1::some(primary_key).expect("primary proposal");
    proposal.rollback_proposal = OptionalPubkeyV1::none();
    proposal.rollback_buffer = OptionalPubkeyV1::none();
    proposal.rollback_artifact_sha256 = [0; 32];
    proposal.rollback_artifact_chunk_root = [0; 32];
    proposal.review_start_slot = review_start_slot;
    proposal.review_end_slot = review_end_slot;
    proposal.not_before_slot = review_end_slot + config.rollback_delay_slots;
    proposal.expiry_slot = creation_slot + config.proposal_expiry_slots;
    proposal.first_approval_slot = 0;
    proposal.council_approved_slot = 0;
    proposal.governance_satisfied_slot = 0;
    proposal.queued_slot = 0;
    proposal.frozen_slot = 0;
    proposal.extension_executed_slot = 0;
    proposal.upgrade_executed_slot = 0;
    proposal.programdata_verified_slot = 0;
    proposal.poststate_accepted_slot = 0;
    proposal.unfreeze_approved_slot = 0;
    proposal.terminal_slot = 0;
    proposal.council_approval_bitset = 0;
    proposal.council_approval_count = 0;
    proposal.cancellation_council_version = 0;
    proposal.cancellation_council_hash = [0; 32];
    proposal.cancellation_approval_bitset = 0;
    proposal.cancellation_approval_count = 0;
    proposal.unfreeze_council_version = 0;
    proposal.unfreeze_council_hash = [0; 32];
    proposal.unfreeze_approval_bitset = 0;
    proposal.unfreeze_approval_count = 0;
    proposal.cancellation_reason_code = 0;
    proposal.terminal_reason_code = 0;
    proposal.proposal_digest = [0; 32];
    proposal.proposal_digest =
        compute_proposal_digest_v2(&proposal).expect("rollback proposal digest");
    proposal
        .validate_schema()
        .expect("valid draft rollback proposal");
    (proposal_key, proposal)
}

fn expectation(
    proposal: &UpgradeProposalV2,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
) -> ProposalExpectationV2 {
    ProposalExpectationV2 {
        expected_proposal_digest: proposal.proposal_digest,
        expected_policy_version: proposal.policy_version,
        expected_policy_hash: proposal.policy_hash,
        expected_council_version: proposal.creation_council_version,
        expected_council_hash: proposal.creation_council_hash,
        expected_gate_status: gate.status,
        expected_gate_epoch: gate.epoch,
        expected_target_nonce: config.target_nonce,
        expected_state: proposal.state,
        expected_review_start_slot: proposal.review_start_slot,
        expected_review_end_slot: proposal.review_end_slot,
        expected_not_before_slot: proposal.not_before_slot,
        expected_expiry_slot: proposal.expiry_slot,
    }
}

fn instruction_optional(value: OptionalPubkeyV1) -> OptionalInstructionPubkeyV1 {
    if value.present {
        OptionalInstructionPubkeyV1::some(value.value).expect("nondefault optional pubkey")
    } else {
        OptionalInstructionPubkeyV1::none()
    }
}

#[allow(clippy::too_many_arguments)]
fn create_instruction_from_expected(
    controller: Pubkey,
    payer: Pubkey,
    creator_seat_authority: Pubkey,
    config: Pubkey,
    policy: Pubkey,
    council: Pubkey,
    gate: Pubkey,
    proposal: Pubkey,
    expected: &UpgradeProposalV2,
) -> Instruction {
    create_proposal_v2_instruction(
        controller,
        CreateProposalV2Accounts {
            payer,
            creator_seat_authority,
            controller_config: config,
            policy,
            council,
            protocol_gate: gate,
            target_program: expected.target_program,
            target_programdata: expected.target_programdata,
            upgradeable_loader: expected.upgradeable_loader,
            authority_pda: expected.authority_pda,
            canonical_spill_treasury: expected.canonical_spill_treasury,
            buffer: expected.buffer_pubkey,
            buffer_uploader_authority: expected.buffer_uploader_authority,
            proposal,
            system_program: system_program::ID,
        },
        CreateProposalV2 {
            proposal_class: expected.proposal_class,
            creation_gate_status: expected.creation_gate_status,
            expected_proposal_id: expected.proposal_id,
            expected_target_nonce: expected.target_nonce,
            creation_slot: expected.creation_slot,
            expected_policy_version: expected.policy_version,
            expected_policy_hash: expected.policy_hash,
            expected_creation_council_version: expected.creation_council_version,
            expected_creation_council_hash: expected.creation_council_hash,
            expected_creation_gate_epoch: expected.creation_gate_epoch,
            expected_freeze_gate_epoch: expected.freeze_gate_epoch,
            artifact_length: expected.artifact_length,
            artifact_sha256: expected.artifact_sha256,
            artifact_chunk_merkle_root: expected.artifact_chunk_merkle_root,
            source_commit_hash: expected.source_commit_hash,
            source_tree_hash: expected.source_tree_hash,
            build_input_inventory_hash: expected.build_input_inventory_hash,
            reproducible_build_receipt_hash: expected.reproducible_build_receipt_hash,
            package_receipt_hash: expected.package_receipt_hash,
            release_intent_hash: expected.release_intent_hash,
            expected_execution_pre_payload_hash: expected.expected_execution_pre_payload_hash,
            expected_execution_pre_chunk_root: expected.expected_execution_pre_chunk_root,
            current_raw_programdata_hash: expected.current_raw_programdata_hash,
            deployed_slot: expected.deployed_slot,
            current_capacity: expected.current_capacity,
            extension_delta: expected.extension_delta,
            expected_post_capacity: expected.expected_post_capacity,
            checkpoint_schema_id: expected.checkpoint_schema_id,
            checkpoint_policy_hash: expected.checkpoint_policy_hash,
            primary_proposal: instruction_optional(expected.primary_proposal),
            rollback_proposal: instruction_optional(expected.rollback_proposal),
            rollback_buffer: instruction_optional(expected.rollback_buffer),
            rollback_artifact_sha256: expected.rollback_artifact_sha256,
            rollback_artifact_chunk_root: expected.rollback_artifact_chunk_root,
            review_start_slot: expected.review_start_slot,
            review_end_slot: expected.review_end_slot,
            not_before_slot: expected.not_before_slot,
            expiry_slot: expected.expiry_slot,
            expected_proposal_digest: expected.proposal_digest,
        },
    )
}

fn fixed_proof(artifact: &[u8], chunk_index: u32) -> FixedMerkleProofV1 {
    let proof = artifact_merkle_proof(artifact, RELEASE1_ARTIFACT_CHUNK_SIZE_V1, chunk_index)
        .expect("bounded proof");
    let mut fixed = FixedMerkleProofV1::empty();
    fixed.proof_len = proof.len() as u8;
    fixed.nodes[..proof.len()].copy_from_slice(&proof);
    fixed
}

async fn bytes(context: &mut ProgramTestContext, key: Pubkey) -> Vec<u8> {
    context
        .banks_client
        .get_account(key)
        .await
        .expect("banks read")
        .expect("account")
        .data
}

async fn state<T: BorshDeserialize>(context: &mut ProgramTestContext, key: Pubkey) -> T {
    T::try_from_slice(&bytes(context, key).await).expect("fixed state decode")
}

fn set_account(context: &mut ProgramTestContext, key: Pubkey, value: Account) {
    context.set_account(&key, &AccountSharedData::from(value));
}

async fn submit(
    context: &mut ProgramTestContext,
    instructions: &[Instruction],
    signers: &[&Keypair],
) -> Result<(), BanksClientError> {
    context.get_new_latest_blockhash().await.expect("blockhash");
    let mut all_signers: Vec<&dyn Signer> = vec![&context.payer];
    all_signers.extend(signers.iter().map(|signer| *signer as &dyn Signer));
    let transaction = Transaction::new_signed_with_payer(
        instructions,
        Some(&context.payer.pubkey()),
        &all_signers,
        context.last_blockhash,
    );
    context.banks_client.process_transaction(transaction).await
}

async fn assert_loader_matrix_failure(
    context: &mut ProgramTestContext,
    instruction: Instruction,
    signers: &[&Keypair],
    writable: &[Pubkey],
    enveloped: bool,
) {
    let mut unique_writable = Vec::new();
    for key in writable {
        if !unique_writable.contains(key) {
            unique_writable.push(*key);
        }
    }
    // Negative account substitutions can introduce a different writable
    // account. Snapshot the instruction after mutation so those accounts are
    // covered by the same failure-atomicity assertion.
    for meta in &instruction.accounts {
        if meta.is_writable && !unique_writable.contains(&meta.pubkey) {
            unique_writable.push(meta.pubkey);
        }
    }
    let mut before = Vec::with_capacity(unique_writable.len());
    for key in &unique_writable {
        before.push(
            context
                .banks_client
                .get_account(*key)
                .await
                .expect("pre-failure account read")
                .map(|account| account.data),
        );
    }
    let instructions = if enveloped {
        let [limit, price] = envelope_prefix();
        vec![limit, price, instruction]
    } else {
        vec![instruction]
    };
    assert!(submit(context, &instructions, signers).await.is_err());
    for (key, expected) in unique_writable.iter().zip(before) {
        let actual = context
            .banks_client
            .get_account(*key)
            .await
            .expect("post-failure account read")
            .map(|account| account.data);
        assert_eq!(actual, expected, "mutated writable account {key}");
    }
}

fn loader_typed_account_header_corruptions(canonical: &Account) -> Vec<(&'static str, Account)> {
    assert!(
        canonical.data.len() >= 10,
        "typed account must contain discriminator/version/bump"
    );
    let mut wrong_owner = canonical.clone();
    wrong_owner.owner = system_program::ID;
    let mut wrong_size = canonical.clone();
    wrong_size.data.pop();
    let mut wrong_discriminator = canonical.clone();
    wrong_discriminator.data[..8].fill(0);
    let mut wrong_version = canonical.clone();
    wrong_version.data[8] ^= 0x7f;
    let mut wrong_bump = canonical.clone();
    wrong_bump.data[9] ^= 1;
    let mut executable = canonical.clone();
    executable.executable = true;
    vec![
        ("owner", wrong_owner),
        ("size", wrong_size),
        ("discriminator", wrong_discriminator),
        ("version", wrong_version),
        ("bump", wrong_bump),
        ("executable", executable),
    ]
}

async fn start_buffer_harness(
    artifact: Vec<u8>,
    current_payload: Vec<u8>,
    extension_delta: u64,
    rollback_artifact: Vec<u8>,
    register_loaded_target: bool,
    controller_sbf: bool,
) -> (ProgramTestContext, BufferHarness) {
    let controller = Pubkey::new_unique();
    let target = Pubkey::new_unique();
    let target_programdata = derive_upgradeable_programdata_address(&target).0;
    let config_key = derive_controller_config_pda(&controller, &target).0;
    let gate_key = derive_gate_pda(&controller, &target).0;
    let authority = derive_authority_pda(&controller, &target).0;
    let buffer = Pubkey::new_unique();
    let uploader = Keypair::new();
    let seats = std::array::from_fn(|_| Keypair::new());
    let spill = Pubkey::new_unique();
    let value_config = config(
        controller,
        target,
        target_programdata,
        spill,
        Pubkey::new_unique(),
    );
    value_config.validate_static().expect("valid test config");
    let value_policy = governance_policy(controller, config_key, target);
    let (council_key, value_council) =
        governance_council(controller, config_key, target, &value_policy, &seats);
    let value_gate = active_gate(controller, config_key, target, target_programdata);
    value_gate.validate_static().expect("valid active gate");
    let (proposal, value_proposal) = draft_proposal(
        controller,
        &value_config,
        &value_gate,
        buffer,
        uploader.pubkey(),
        &artifact,
        &current_payload,
        extension_delta,
        value_council.set_hash,
        Pubkey::new_from_array([77; 32]),
        &rollback_artifact,
    );
    let verification = derive_buffer_check_pda(&controller, &proposal).0;

    let mut test = if controller_sbf {
        ProgramTest::new("upgrade_controller", controller, None)
    } else {
        let mut native = ProgramTest::default();
        native.prefer_bpf(false);
        native.add_program(
            "upgrade_controller",
            controller,
            processor!(process_instruction),
        );
        native
    };
    test.add_account(config_key, state_account(controller, &value_config));
    test.add_account(council_key, state_account(controller, &value_council));
    test.add_account(gate_key, state_account(controller, &value_gate));
    test.add_account(proposal, state_account(controller, &value_proposal));
    if !register_loaded_target {
        test.add_account(
            target,
            account(
                UPGRADEABLE_LOADER_ID,
                program_bytes(target_programdata),
                true,
            ),
        );
    }
    if !register_loaded_target {
        test.add_account(
            target_programdata,
            account(
                UPGRADEABLE_LOADER_ID,
                programdata_bytes(INITIAL_PROGRAMDATA_SLOT, authority, &current_payload),
                false,
            ),
        );
    }
    test.add_account(
        buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(uploader.pubkey(), &artifact),
            false,
        ),
    );
    test.add_account(
        uploader.pubkey(),
        account(system_program::ID, Vec::new(), false),
    );
    test.add_account(authority, account(system_program::ID, Vec::new(), false));
    test.add_account(spill, account(system_program::ID, Vec::new(), false));
    for seat in &seats {
        test.add_account(
            seat.pubkey(),
            account(system_program::ID, Vec::new(), false),
        );
    }

    let mut context = test.start_with_context().await;
    if register_loaded_target {
        // Inject the exact Loader-v3 Program/ProgramData pair after bank startup.
        // Agave 2.3 ProgramTest eagerly caches genesis executable programs and
        // debug-panics when ExtendProgramChecked legitimately reloads the same
        // deployment slot. Late injection keeps the sacrificial accounts exact
        // while making the real checked extension the first cache population.
        set_account(
            &mut context,
            target,
            account(
                UPGRADEABLE_LOADER_ID,
                program_bytes(target_programdata),
                true,
            ),
        );
        set_account(
            &mut context,
            target_programdata,
            account(
                UPGRADEABLE_LOADER_ID,
                programdata_bytes(INITIAL_PROGRAMDATA_SLOT, authority, &current_payload),
                false,
            ),
        );
    }
    let mut clock = context
        .banks_client
        .get_sysvar::<solana_sdk::clock::Clock>()
        .await
        .expect("clock");
    clock.slot = TEST_SLOT;
    context.set_sysvar(&clock);
    (
        context,
        BufferHarness {
            controller,
            target,
            target_programdata,
            config: config_key,
            gate: gate_key,
            authority,
            spill,
            proposal,
            council: council_key,
            verification,
            buffer,
            uploader,
            seats,
            artifact,
            current_payload,
            rollback_artifact,
        },
    )
}

async fn seal_buffer(
    context: &mut ProgramTestContext,
    harness: &BufferHarness,
) -> (UpgradeProposalV2, BufferVerificationV1) {
    let value_config: ControllerConfigV1 = state(context, harness.config).await;
    let value_gate: ProtocolGateV1 = state(context, harness.gate).await;
    let proposal: UpgradeProposalV2 = state(context, harness.proposal).await;
    let adopt = adopt_buffer_v1_instruction(
        harness.controller,
        AdoptBufferV1Accounts {
            payer: context.payer.pubkey(),
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: harness.proposal,
            buffer: harness.buffer,
            uploader_authority: harness.uploader.pubkey(),
            authority_pda: harness.authority,
            buffer_verification: harness.verification,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            system_program: system_program::ID,
        },
        AdoptBufferV1 {
            expected: expectation(&proposal, &value_config, &value_gate),
        },
    );
    let [limit, price] = envelope_prefix();
    submit(context, &[limit, price, adopt], &[&harness.uploader])
        .await
        .expect("real SetAuthorityChecked CPI");
    let adopted: UpgradeProposalV2 = state(context, harness.proposal).await;
    let mut verification: BufferVerificationV1 = state(context, harness.verification).await;
    for chunk_index in 0..adopted.chunk_count {
        let verify = verify_buffer_chunk_v1_instruction(
            harness.controller,
            VerifyBufferChunkV1Accounts {
                controller_config: harness.config,
                protocol_gate: harness.gate,
                proposal: harness.proposal,
                buffer: harness.buffer,
                buffer_verification: harness.verification,
                authority_pda: harness.authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
            },
            VerifyBufferChunkV1 {
                expected: expectation(&adopted, &value_config, &value_gate),
                chunk_index,
                proof: fixed_proof(&harness.artifact, chunk_index),
                expected_verification_status: verification.status,
                expected_verified_chunk_bitmap: verification.verified_chunk_bitmap,
                expected_verified_chunk_count: verification.verified_chunk_count,
            },
        );
        let [limit, price] = envelope_prefix();
        submit(context, &[limit, price, verify], &[])
            .await
            .expect("exact sealed chunk proof");
        verification = state(context, harness.verification).await;
    }
    let finalize = finalize_buffer_verification_v1_instruction(
        harness.controller,
        FinalizeBufferVerificationV1Accounts {
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: harness.proposal,
            buffer: harness.buffer,
            buffer_verification: harness.verification,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
        },
        FinalizeBufferVerificationV1 {
            expected: expectation(&adopted, &value_config, &value_gate),
            expected_verification_status: verification.status,
            expected_verified_chunk_bitmap: verification.verified_chunk_bitmap,
            expected_verified_chunk_count: verification.verified_chunk_count,
        },
    );
    let [limit, price] = envelope_prefix();
    submit(context, &[limit, price, finalize], &[])
        .await
        .expect("full sealed payload SHA-256 finalization");
    (
        state(context, harness.proposal).await,
        state(context, harness.verification).await,
    )
}

#[allow(clippy::too_many_arguments)]
async fn seal_additional_buffer(
    context: &mut ProgramTestContext,
    harness: &BufferHarness,
    proposal_key: Pubkey,
    verification_key: Pubkey,
    buffer: Pubkey,
    uploader: &Keypair,
    artifact: &[u8],
) -> (UpgradeProposalV2, BufferVerificationV1) {
    let value_config: ControllerConfigV1 = state(context, harness.config).await;
    let value_gate: ProtocolGateV1 = state(context, harness.gate).await;
    let proposal: UpgradeProposalV2 = state(context, proposal_key).await;
    let adopt = adopt_buffer_v1_instruction(
        harness.controller,
        AdoptBufferV1Accounts {
            payer: context.payer.pubkey(),
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: proposal_key,
            buffer,
            uploader_authority: uploader.pubkey(),
            authority_pda: harness.authority,
            buffer_verification: verification_key,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            system_program: system_program::ID,
        },
        AdoptBufferV1 {
            expected: expectation(&proposal, &value_config, &value_gate),
        },
    );
    let [limit, price] = envelope_prefix();
    submit(context, &[limit, price, adopt], &[uploader])
        .await
        .expect("actual-SBF rollback SetAuthorityChecked CPI");
    let adopted: UpgradeProposalV2 = state(context, proposal_key).await;
    let mut verification: BufferVerificationV1 = state(context, verification_key).await;
    for chunk_index in 0..adopted.chunk_count {
        let verify = verify_buffer_chunk_v1_instruction(
            harness.controller,
            VerifyBufferChunkV1Accounts {
                controller_config: harness.config,
                protocol_gate: harness.gate,
                proposal: proposal_key,
                buffer,
                buffer_verification: verification_key,
                authority_pda: harness.authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
            },
            VerifyBufferChunkV1 {
                expected: expectation(&adopted, &value_config, &value_gate),
                chunk_index,
                proof: fixed_proof(artifact, chunk_index),
                expected_verification_status: verification.status,
                expected_verified_chunk_bitmap: verification.verified_chunk_bitmap,
                expected_verified_chunk_count: verification.verified_chunk_count,
            },
        );
        let [limit, price] = envelope_prefix();
        submit(context, &[limit, price, verify], &[])
            .await
            .expect("actual-SBF rollback sealed chunk proof");
        verification = state(context, verification_key).await;
    }
    let finalize = finalize_buffer_verification_v1_instruction(
        harness.controller,
        FinalizeBufferVerificationV1Accounts {
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: proposal_key,
            buffer,
            buffer_verification: verification_key,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
        },
        FinalizeBufferVerificationV1 {
            expected: expectation(&adopted, &value_config, &value_gate),
            expected_verification_status: verification.status,
            expected_verified_chunk_bitmap: verification.verified_chunk_bitmap,
            expected_verified_chunk_count: verification.verified_chunk_count,
        },
    );
    let [limit, price] = envelope_prefix();
    submit(context, &[limit, price, finalize], &[])
        .await
        .expect("actual-SBF rollback Buffer finalization");
    (
        state(context, proposal_key).await,
        state(context, verification_key).await,
    )
}

fn complete_bitmap(count: u32) -> [u8; VERIFICATION_BITMAP_BYTES_V1] {
    let mut bitmap = [0; VERIFICATION_BITMAP_BYTES_V1];
    for index in 0..count {
        bitmap[(index / 8) as usize] |= 1 << (index % 8);
    }
    bitmap
}

fn rollback_proposal(
    harness: &BufferHarness,
    config: &ControllerConfigV1,
    primary: &UpgradeProposalV2,
) -> UpgradeProposalV2 {
    let key = primary.rollback_proposal.value;
    // A prepared rollback must remain executable beyond the primary expiry
    // plus the configured rollback delay. Mirror the production lifecycle by
    // creating it one slot after that minimum offset instead of cloning the
    // primary proposal's timing verbatim.
    let creation_slot = primary
        .creation_slot
        .checked_add(config.rollback_delay_slots)
        .and_then(|slot| slot.checked_add(1))
        .expect("rollback creation slot");
    let review_start_slot = creation_slot.checked_add(1).expect("rollback review start");
    let review_end_slot = review_start_slot
        .checked_add(config.vote_review_slots)
        .expect("rollback review end");
    let mut rollback = primary.clone();
    rollback.bump = derive_proposal_pda(&harness.controller, &harness.target, 2).1;
    rollback.proposal_class = ProposalClassV1::EmergencyRollback;
    rollback.state = ProposalStateV2::Timelocked;
    rollback.proposal_id = 2;
    rollback.freeze_gate_epoch = 0;
    rollback.buffer_pubkey = primary.rollback_buffer.value;
    rollback.buffer_uploader_authority = Pubkey::new_from_array([78; 32]);
    rollback.buffer_verification = derive_buffer_check_pda(&harness.controller, &key).0;
    rollback.programdata_verification = derive_programdata_check_pda(&harness.controller, &key).0;
    rollback.artifact_length = harness.rollback_artifact.len() as u64;
    rollback.artifact_sha256 = hashv(&[&harness.rollback_artifact]).to_bytes();
    rollback.artifact_chunk_merkle_root =
        artifact_merkle_root(&harness.rollback_artifact, RELEASE1_ARTIFACT_CHUNK_SIZE_V1)
            .expect("rollback root");
    rollback.chunk_count = artifact_chunk_count(
        harness.rollback_artifact.len() as u64,
        RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
    )
    .expect("rollback chunk count");
    rollback.expected_execution_pre_payload_hash = primary.artifact_sha256;
    rollback.expected_execution_pre_chunk_root = primary.artifact_chunk_merkle_root;
    rollback.current_raw_programdata_hash = [0; 32];
    rollback.deployed_slot = 0;
    rollback.current_capacity = primary.expected_post_capacity;
    rollback.extension_delta = 0;
    rollback.expected_post_capacity = primary.expected_post_capacity;
    rollback.prestate_checkpoint =
        derive_checkpoint_pda(&harness.controller, &key, CheckpointPhaseV1::Prestate).0;
    rollback.required_poststate_checkpoint =
        derive_checkpoint_pda(&harness.controller, &key, CheckpointPhaseV1::Poststate).0;
    rollback.primary_proposal = OptionalPubkeyV1::some(harness.proposal).expect("primary");
    rollback.rollback_proposal = OptionalPubkeyV1::none();
    rollback.rollback_buffer = OptionalPubkeyV1::none();
    rollback.rollback_artifact_sha256 = [0; 32];
    rollback.rollback_artifact_chunk_root = [0; 32];
    rollback.creation_slot = creation_slot;
    rollback.review_start_slot = review_start_slot;
    rollback.review_end_slot = review_end_slot;
    rollback.not_before_slot = review_end_slot
        .checked_add(config.rollback_delay_slots)
        .expect("rollback not-before slot");
    rollback.expiry_slot = creation_slot
        .checked_add(config.proposal_expiry_slots)
        .expect("rollback expiry slot");
    rollback.first_approval_slot = review_start_slot;
    rollback.council_approved_slot = review_start_slot;
    rollback.governance_satisfied_slot = review_start_slot;
    rollback.queued_slot = review_start_slot;
    rollback.frozen_slot = 0;
    rollback.extension_executed_slot = 0;
    rollback.upgrade_executed_slot = 0;
    rollback.programdata_verified_slot = 0;
    rollback.poststate_accepted_slot = 0;
    rollback.unfreeze_approved_slot = 0;
    rollback.terminal_slot = 0;
    rollback.council_approval_bitset = 0b00111;
    rollback.council_approval_count = 3;
    rollback.cancellation_council_version = 0;
    rollback.cancellation_council_hash = [0; 32];
    rollback.cancellation_approval_bitset = 0;
    rollback.cancellation_approval_count = 0;
    rollback.unfreeze_council_version = 0;
    rollback.unfreeze_council_hash = [0; 32];
    rollback.unfreeze_approval_bitset = 0;
    rollback.unfreeze_approval_count = 0;
    rollback.cancellation_reason_code = 0;
    rollback.terminal_reason_code = 0;
    rollback.proposal_digest = [0; 32];
    rollback.proposal_digest = compute_proposal_digest_v2(&rollback).expect("rollback digest");
    rollback.validate_schema().expect("valid rollback proposal");
    rollback
}

fn verified_rollback_buffer(
    harness: &BufferHarness,
    rollback_key: Pubkey,
    rollback: &UpgradeProposalV2,
    finalized_slot: u64,
) -> BufferVerificationV1 {
    let sealed = buffer_bytes(harness.authority, &harness.rollback_artifact);
    let verification = BufferVerificationV1 {
        discriminator: BUFFER_VERIFICATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_buffer_check_pda(&harness.controller, &rollback_key).1,
        initialized: true,
        status: BufferVerificationStatusV1::Verified,
        controller_config: harness.config,
        proposal: rollback_key,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        buffer: rollback.buffer_pubkey,
        expected_uploader_authority: rollback.buffer_uploader_authority,
        controller_authority: harness.authority,
        artifact_length: rollback.artifact_length,
        artifact_sha256: rollback.artifact_sha256,
        artifact_chunk_merkle_root: rollback.artifact_chunk_merkle_root,
        chunk_hash_domain: rollback.chunk_hash_domain,
        chunk_size: rollback.chunk_size,
        chunk_count: rollback.chunk_count,
        verified_chunk_bitmap: complete_bitmap(rollback.chunk_count),
        verified_chunk_count: rollback.chunk_count,
        adopted_slot: finalized_slot - 1,
        finalized_slot,
        sealed_buffer_header_hash: hashv(&[&sealed[..LOADER_BUFFER_METADATA_LEN]]).to_bytes(),
        terminal_slot: 0,
        reserved: [0; BUFFER_VERIFICATION_V1_RESERVED_LEN],
    };
    verification.validate_schema().expect("valid rollback seal");
    verification
}

fn accepted_prestate(
    harness: &BufferHarness,
    primary: &UpgradeProposalV2,
    gate: &ProtocolGateV1,
    finalized_slot: u64,
) -> StateCheckpointV1 {
    let mut checkpoint = StateCheckpointV1 {
        discriminator: STATE_CHECKPOINT_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_checkpoint_pda(
            &harness.controller,
            &harness.proposal,
            CheckpointPhaseV1::Prestate,
        )
        .1,
        initialized: true,
        phase: StateCheckpointPhaseV1::Prestate,
        controller_config: harness.config,
        proposal: harness.proposal,
        emergency_resolution: Pubkey::default(),
        subject_digest: primary.proposal_digest,
        target_program: harness.target,
        target_programdata: harness.target_programdata,
        finalized_observation_slot: finalized_slot,
        gate_epoch: gate.epoch,
        target_programdata_slot: INITIAL_PROGRAMDATA_SLOT,
        target_payload_commitment: hashv(&[&harness.current_payload]).to_bytes(),
        target_raw_programdata_commitment: loader_account_data_hash(&programdata_bytes(
            INITIAL_PROGRAMDATA_SLOT,
            harness.authority,
            &harness.current_payload,
        )),
        target_capacity: harness.current_payload.len() as u64,
        program_owned_state_root: [31; 32],
        program_owned_state_count: 5,
        logical_compressed_state_root: [32; 32],
        logical_compressed_state_count: 6,
        semantic_custody_accounting_root: [33; 32],
        hard_combined_root: [0; 32],
        external_metadata_observation_root: [34; 32],
        external_raw_balance_observation_root: [35; 32],
        schema_identifier: primary.checkpoint_schema_id,
        admitted_positive_donation_root: [0; 32],
        admitted_positive_donation_count: 0,
        forbidden_drift_count: 0,
        approval_council_version: primary.creation_council_version,
        approval_council_hash: primary.creation_council_hash,
        checkpoint_digest: [1; 32],
        approval_bitset: 0b00111,
        approval_count: 3,
        accepted: true,
        finalized_slot,
        reserved: [0; STATE_CHECKPOINT_V1_RESERVED_LEN],
    };
    checkpoint.hard_combined_root =
        compute_state_checkpoint_hard_combined_root_v1(&checkpoint).expect("hard root");
    checkpoint.checkpoint_digest =
        compute_state_checkpoint_digest_v1(&checkpoint).expect("checkpoint digest");
    checkpoint
        .validate_schema()
        .expect("valid accepted prestate");
    checkpoint
}

fn accepted_poststate(
    harness: &BufferHarness,
    proposal: &UpgradeProposalV2,
    gate: &ProtocolGateV1,
    verification: &ProgramDataVerificationV1,
    baseline: &StateCheckpointV1,
    council: &GovernanceCouncilSetV1,
    finalized_slot: u64,
) -> StateCheckpointV1 {
    let mut checkpoint = baseline.clone();
    checkpoint.bump = derive_checkpoint_pda(
        &harness.controller,
        &harness.proposal,
        CheckpointPhaseV1::Poststate,
    )
    .1;
    checkpoint.phase = StateCheckpointPhaseV1::Poststate;
    checkpoint.subject_digest = proposal.proposal_digest;
    checkpoint.finalized_observation_slot = verification.finalized_slot;
    checkpoint.gate_epoch = gate.epoch;
    checkpoint.target_programdata_slot = verification.deployed_slot;
    checkpoint.target_payload_commitment = proposal.artifact_sha256;
    checkpoint.target_raw_programdata_commitment = verification.raw_programdata_hash;
    checkpoint.target_capacity = verification.capacity;
    checkpoint.approval_council_version = council.version;
    checkpoint.approval_council_hash = council.set_hash;
    checkpoint.checkpoint_digest = [0; 32];
    checkpoint.approval_bitset = 0b00111;
    checkpoint.approval_count = 3;
    checkpoint.accepted = true;
    checkpoint.finalized_slot = finalized_slot;
    checkpoint.checkpoint_digest =
        compute_state_checkpoint_digest_v1(&checkpoint).expect("poststate checkpoint digest");
    checkpoint
        .validate_schema()
        .expect("valid accepted poststate");
    checkpoint
}

#[allow(clippy::too_many_arguments)]
fn checkpoint_candidate(
    harness: &BufferHarness,
    proposal_key: Pubkey,
    proposal: &UpgradeProposalV2,
    council: &GovernanceCouncilSetV1,
    gate: &ProtocolGateV1,
    phase: StateCheckpointPhaseV1,
    observation_slot: u64,
    target_programdata_slot: u64,
    target_payload: &[u8],
    target_raw_programdata: &[u8],
    baseline: Option<&StateCheckpointV1>,
) -> CheckpointCandidateV1 {
    let checkpoint_phase = match phase {
        StateCheckpointPhaseV1::Prestate => CheckpointPhaseV1::Prestate,
        StateCheckpointPhaseV1::Poststate => CheckpointPhaseV1::Poststate,
        StateCheckpointPhaseV1::Emergency => {
            unreachable!("code-upgrade harness does not create emergency checkpoints")
        }
    };
    let (program_owned_state_root, program_owned_state_count) = baseline
        .map(|value| {
            (
                value.program_owned_state_root,
                value.program_owned_state_count,
            )
        })
        .unwrap_or(([31; 32], 5));
    let (logical_compressed_state_root, logical_compressed_state_count) = baseline
        .map(|value| {
            (
                value.logical_compressed_state_root,
                value.logical_compressed_state_count,
            )
        })
        .unwrap_or(([32; 32], 6));
    let semantic_custody_accounting_root = baseline
        .map(|value| value.semantic_custody_accounting_root)
        .unwrap_or([33; 32]);
    let external_metadata_observation_root = baseline
        .map(|value| value.external_metadata_observation_root)
        .unwrap_or([34; 32]);
    let external_raw_balance_observation_root = baseline
        .map(|value| value.external_raw_balance_observation_root)
        .unwrap_or([35; 32]);
    let mut prototype = StateCheckpointV1 {
        discriminator: STATE_CHECKPOINT_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_checkpoint_pda(&harness.controller, &proposal_key, checkpoint_phase).1,
        initialized: true,
        phase,
        controller_config: harness.config,
        proposal: proposal_key,
        emergency_resolution: Pubkey::default(),
        subject_digest: proposal.proposal_digest,
        target_program: harness.target,
        target_programdata: harness.target_programdata,
        finalized_observation_slot: observation_slot,
        gate_epoch: gate.epoch,
        target_programdata_slot,
        target_payload_commitment: hashv(&[target_payload]).to_bytes(),
        target_raw_programdata_commitment: loader_account_data_hash(target_raw_programdata),
        target_capacity: (target_raw_programdata.len() - LOADER_PROGRAMDATA_METADATA_LEN) as u64,
        program_owned_state_root,
        program_owned_state_count,
        logical_compressed_state_root,
        logical_compressed_state_count,
        semantic_custody_accounting_root,
        hard_combined_root: [0; 32],
        external_metadata_observation_root,
        external_raw_balance_observation_root,
        schema_identifier: proposal.checkpoint_schema_id,
        admitted_positive_donation_root: [0; 32],
        admitted_positive_donation_count: 0,
        forbidden_drift_count: 0,
        approval_council_version: council.version,
        approval_council_hash: council.set_hash,
        checkpoint_digest: [0; 32],
        approval_bitset: 0b0_0111,
        approval_count: 3,
        accepted: true,
        finalized_slot: observation_slot,
        reserved: [0; STATE_CHECKPOINT_V1_RESERVED_LEN],
    };
    prototype.hard_combined_root =
        compute_state_checkpoint_hard_combined_root_v1(&prototype).expect("hard checkpoint root");
    prototype.checkpoint_digest =
        compute_state_checkpoint_digest_v1(&prototype).expect("checkpoint digest");
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
        expected_subject_digest: proposal.proposal_digest,
        expected_gate_status: gate.status,
        expected_gate_epoch: gate.epoch,
        finalized_observation_slot: observation_slot,
        target_programdata_slot,
        target_payload_commitment: prototype.target_payload_commitment,
        target_raw_programdata_commitment: prototype.target_raw_programdata_commitment,
        target_capacity: prototype.target_capacity,
        program_owned_state_root,
        program_owned_state_count,
        logical_compressed_state_root,
        logical_compressed_state_count,
        semantic_custody_accounting_root,
        hard_combined_root: prototype.hard_combined_root,
        external_metadata_observation_root,
        external_raw_balance_observation_root,
        schema_identifier: prototype.schema_identifier,
        admitted_positive_donation_root: [0; 32],
        admitted_positive_donation_count: 0,
        forbidden_drift_count: 0,
        expected_checkpoint_digest: prototype.checkpoint_digest,
    }
}

fn unfreeze_expectation(
    proposal: &UpgradeProposalV2,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
    council: &GovernanceCouncilSetV1,
    gate: &ProtocolGateV1,
    checkpoint: &StateCheckpointV1,
    verification: &ProgramDataVerificationV1,
) -> UnfreezeExpectationV1 {
    UnfreezeExpectationV1 {
        expected_proposal_digest: proposal.proposal_digest,
        expected_policy_version: policy.version,
        expected_policy_hash: policy.policy_hash,
        expected_current_council_version: council.version,
        expected_current_council_hash: council.set_hash,
        expected_frozen_gate_epoch: gate.epoch,
        expected_target_nonce: config.target_nonce,
        expected_proposal_state: proposal.state,
        expected_poststate_checkpoint_digest: checkpoint.checkpoint_digest,
        expected_programdata_authority: config.authority_pda,
        expected_programdata_deployed_slot: verification.deployed_slot,
        expected_programdata_capacity: verification.capacity,
        expected_raw_programdata_hash: verification.raw_programdata_hash,
        expected_unfreeze_approval_bitset: proposal.unfreeze_approval_bitset,
        expected_unfreeze_approval_count: proposal.unfreeze_approval_count,
        expected_programdata_verification_finalized_slot: verification.finalized_slot,
    }
}

async fn seed_frozen_loader_state(
    context: &mut ProgramTestContext,
    harness: &BufferHarness,
    mut primary: UpgradeProposalV2,
) -> (
    ControllerConfigV1,
    GovernancePolicyV1,
    ProtocolGateV1,
    UpgradeProposalV2,
    UpgradeProposalV2,
    StateCheckpointV1,
) {
    let mut config: ControllerConfigV1 = state(context, harness.config).await;
    config.next_proposal_id = 3;
    config.target_nonce = primary.target_nonce + 1;
    let policy = governance_policy(harness.controller, harness.config, harness.target);
    let mut gate: ProtocolGateV1 = state(context, harness.gate).await;
    gate.status = GateStatusV1::FrozenForUpgrade;
    gate.epoch += 1;
    gate.active_proposal = harness.proposal;
    gate.freeze_slot = primary.not_before_slot;
    gate.freeze_reason_code = GOVERNED_UPGRADE_FREEZE_REASON_V1;
    primary.state = ProposalStateV2::Frozen;
    primary.freeze_gate_epoch = gate.epoch;
    primary.first_approval_slot = primary.review_start_slot;
    primary.council_approved_slot = primary.review_start_slot;
    primary.governance_satisfied_slot = primary.review_start_slot;
    primary.queued_slot = primary.review_start_slot;
    primary.frozen_slot = gate.freeze_slot;
    primary.council_approval_bitset = 0b00111;
    primary.council_approval_count = 3;
    primary.validate_schema().expect("valid frozen primary");
    let rollback = rollback_proposal(harness, &config, &primary);
    let rollback_verification = verified_rollback_buffer(
        harness,
        primary.rollback_proposal.value,
        &rollback,
        gate.freeze_slot,
    );
    let prestate = accepted_prestate(harness, &primary, &gate, gate.freeze_slot);

    set_account(
        context,
        harness.config,
        state_account(harness.controller, &config),
    );
    set_account(
        context,
        harness.gate,
        state_account(harness.controller, &gate),
    );
    set_account(
        context,
        harness.proposal,
        state_account(harness.controller, &primary),
    );
    let policy_key = derive_policy_pda(&harness.controller, &harness.target, 1).0;
    set_account(
        context,
        policy_key,
        state_account(harness.controller, &policy),
    );
    let rollback_key = primary.rollback_proposal.value;
    set_account(
        context,
        rollback_key,
        state_account(harness.controller, &rollback),
    );
    set_account(
        context,
        rollback.buffer_verification,
        state_account(harness.controller, &rollback_verification),
    );
    set_account(
        context,
        rollback.buffer_pubkey,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(harness.authority, &harness.rollback_artifact),
            false,
        ),
    );
    set_account(
        context,
        primary.prestate_checkpoint,
        state_account(harness.controller, &prestate),
    );
    let mut clock = context
        .banks_client
        .get_sysvar::<solana_sdk::clock::Clock>()
        .await
        .expect("clock");
    clock.slot = gate.freeze_slot + 1;
    context.set_sysvar(&clock);
    (config, policy, gate, primary, rollback, prestate)
}

#[tokio::test]
#[ignore = "historical V1/V2 custody execution is intentionally closed; dispatcher and codec regressions cover tags 27-38"]
async fn real_loader_checked_buffer_adoption_and_exact_chunk_finalization() {
    let artifact = (0..(RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as usize * 2 + 17))
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let (mut context, harness) = start_buffer_harness(
        artifact.clone(),
        artifact,
        0,
        ROLLBACK_ARTIFACT.to_vec(),
        false,
        false,
    )
    .await;
    let value_config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let value_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let proposal: UpgradeProposalV2 = state(&mut context, harness.proposal).await;

    let adopt = adopt_buffer_v1_instruction(
        harness.controller,
        AdoptBufferV1Accounts {
            payer: context.payer.pubkey(),
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: harness.proposal,
            buffer: harness.buffer,
            uploader_authority: harness.uploader.pubkey(),
            authority_pda: harness.authority,
            buffer_verification: harness.verification,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            system_program: system_program::ID,
        },
        AdoptBufferV1 {
            expected: expectation(&proposal, &value_config, &value_gate),
        },
    );
    submit(&mut context, &[adopt], &[&harness.uploader])
        .await
        .expect("real SetAuthorityChecked CPI");

    let sealed = bytes(&mut context, harness.buffer).await;
    assert_eq!(
        parse_upgradeable_buffer(&sealed)
            .expect("canonical Loader-v3 Buffer")
            .authority,
        Some(harness.authority)
    );
    assert_eq!(
        &sealed[LOADER_BUFFER_METADATA_LEN..],
        harness.artifact.as_slice()
    );
    let adopted: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
    let mut verification: BufferVerificationV1 = state(&mut context, harness.verification).await;
    assert_eq!(adopted.state, ProposalStateV2::BufferAdopted);
    assert_eq!(verification.status, BufferVerificationStatusV1::Adopted);
    assert_eq!(verification.verified_chunk_count, 0);

    for chunk_index in 0..adopted.chunk_count {
        let before = verification.clone();
        let verify = verify_buffer_chunk_v1_instruction(
            harness.controller,
            VerifyBufferChunkV1Accounts {
                controller_config: harness.config,
                protocol_gate: harness.gate,
                proposal: harness.proposal,
                buffer: harness.buffer,
                buffer_verification: harness.verification,
                authority_pda: harness.authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
            },
            VerifyBufferChunkV1 {
                expected: expectation(&adopted, &value_config, &value_gate),
                chunk_index,
                proof: fixed_proof(&harness.artifact, chunk_index),
                expected_verification_status: before.status,
                expected_verified_chunk_bitmap: before.verified_chunk_bitmap,
                expected_verified_chunk_count: before.verified_chunk_count,
            },
        );
        submit(&mut context, std::slice::from_ref(&verify), &[])
            .await
            .expect("exact sealed chunk proof");
        verification = state(&mut context, harness.verification).await;

        if chunk_index == 0 {
            let before_duplicate = bytes(&mut context, harness.verification).await;
            let duplicate = verify_buffer_chunk_v1_instruction(
                harness.controller,
                VerifyBufferChunkV1Accounts {
                    controller_config: harness.config,
                    protocol_gate: harness.gate,
                    proposal: harness.proposal,
                    buffer: harness.buffer,
                    buffer_verification: harness.verification,
                    authority_pda: harness.authority,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                },
                VerifyBufferChunkV1 {
                    expected: expectation(&adopted, &value_config, &value_gate),
                    chunk_index,
                    proof: fixed_proof(&harness.artifact, chunk_index),
                    expected_verification_status: verification.status,
                    expected_verified_chunk_bitmap: verification.verified_chunk_bitmap,
                    expected_verified_chunk_count: verification.verified_chunk_count,
                },
            );
            assert!(submit(&mut context, &[duplicate], &[]).await.is_err());
            assert_eq!(
                bytes(&mut context, harness.verification).await,
                before_duplicate,
                "duplicate verification must be failure-atomic"
            );
        }
    }
    assert_eq!(
        verification.status,
        BufferVerificationStatusV1::ReadyToFinalize
    );

    let finalize = finalize_buffer_verification_v1_instruction(
        harness.controller,
        FinalizeBufferVerificationV1Accounts {
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: harness.proposal,
            buffer: harness.buffer,
            buffer_verification: harness.verification,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
        },
        FinalizeBufferVerificationV1 {
            expected: expectation(&adopted, &value_config, &value_gate),
            expected_verification_status: verification.status,
            expected_verified_chunk_bitmap: verification.verified_chunk_bitmap,
            expected_verified_chunk_count: verification.verified_chunk_count,
        },
    );
    submit(&mut context, &[finalize], &[])
        .await
        .expect("full sealed payload SHA-256 finalization");
    let finalized_proposal: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
    let finalized: BufferVerificationV1 = state(&mut context, harness.verification).await;
    assert_eq!(finalized_proposal.state, ProposalStateV2::BufferVerified);
    assert_eq!(finalized.status, BufferVerificationStatusV1::Verified);
    assert_eq!(
        finalized.verified_chunk_count,
        finalized_proposal.chunk_count
    );
    assert_eq!(
        finalized.artifact_sha256,
        hashv(&[&harness.artifact]).to_bytes()
    );
    assert!(finalized.finalized_slot >= finalized.adopted_slot);
    assert_eq!(
        parse_upgradeable_buffer(&bytes(&mut context, harness.buffer).await)
            .expect("sealed Buffer remains canonical")
            .authority,
        Some(harness.authority)
    );
    assert_ne!(harness.target, Pubkey::default());
    assert_ne!(harness.target_programdata, Pubkey::default());
    assert_eq!(finalized.reserved, [0; BUFFER_VERIFICATION_V1_RESERVED_LEN]);
    let mut expected_bitmap = [0; VERIFICATION_BITMAP_BYTES_V1];
    for index in 0..finalized_proposal.chunk_count {
        expected_bitmap[(index / 8) as usize] |= 1 << (index % 8);
    }
    assert_eq!(finalized.verified_chunk_bitmap, expected_bitmap);
}

#[tokio::test]
#[ignore = "historical V1/V2 custody execution is intentionally closed; dispatcher and codec regressions cover tags 27-38"]
async fn loader_tags_27_through_38_account_contract_matrix_is_failure_atomic() {
    let artifact = (0..(RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as usize + 17))
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let (mut context, harness) = start_buffer_harness(
        artifact.clone(),
        artifact.clone(),
        0,
        artifact.clone(),
        true,
        false,
    )
    .await;
    let (sealed, sealed_verification) = seal_buffer(&mut context, &harness).await;
    let (config, policy, gate, primary, rollback, prestate) =
        seed_frozen_loader_state(&mut context, &harness, sealed).await;
    let policy_key = derive_policy_pda(&harness.controller, &harness.target, 1).0;
    let programdata_verification = primary.programdata_verification;
    let failure_observation = derive_programdata_failure_observation_pda(
        &harness.controller,
        &harness.proposal,
        gate.epoch,
    )
    .0;
    let alternate_config = Pubkey::new_unique();
    set_account(
        &mut context,
        alternate_config,
        state_account(harness.controller, &config),
    );
    let raw_programdata = bytes(&mut context, harness.target_programdata).await;
    let programdata_state = ProgramDataVerificationV1 {
        discriminator: PROGRAMDATA_VERIFICATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_programdata_check_pda(&harness.controller, &harness.proposal).1,
        initialized: true,
        status: ProgramDataVerificationStatusV1::Verifying,
        controller_config: harness.config,
        proposal: harness.proposal,
        target_program: harness.target,
        target_programdata: harness.target_programdata,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        controller_authority: harness.authority,
        artifact_length: primary.artifact_length,
        artifact_sha256: primary.artifact_sha256,
        artifact_chunk_merkle_root: primary.artifact_chunk_merkle_root,
        chunk_hash_domain: primary.chunk_hash_domain,
        chunk_size: primary.chunk_size,
        payload_chunk_count: primary.chunk_count,
        verified_payload_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
        verified_payload_chunk_count: 0,
        deployed_slot: INITIAL_PROGRAMDATA_SLOT,
        capacity: primary.expected_post_capacity,
        tail_length: primary.expected_post_capacity - primary.artifact_length,
        tail_chunk_count: 0,
        verified_tail_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
        verified_tail_chunk_count: 0,
        raw_programdata_hash: [0; 32],
        zero_tail_verified: false,
        finalized_slot: 0,
        reserved: [0; PROGRAMDATA_VERIFICATION_V1_RESERVED_LEN],
    };
    // A one-chunk artifact would make an all-zero bitmap complete only when
    // chunk_count were zero, which Release 1 forbids. Keep this assertion next
    // to the fixture so changes in chunk policy cannot weaken the matrix.
    assert!(programdata_state.payload_chunk_count > 0);
    programdata_state
        .validate_schema()
        .expect("typed ProgramData verification fixture");
    set_account(
        &mut context,
        programdata_verification,
        state_account(harness.controller, &programdata_state),
    );

    let mut poststate = prestate.clone();
    poststate.bump = derive_checkpoint_pda(
        &harness.controller,
        &harness.proposal,
        CheckpointPhaseV1::Poststate,
    )
    .1;
    poststate.phase = StateCheckpointPhaseV1::Poststate;
    poststate.subject_digest = primary.proposal_digest;
    poststate.finalized_observation_slot = gate.freeze_slot.max(TEST_SLOT);
    poststate.target_payload_commitment = primary.artifact_sha256;
    poststate.target_raw_programdata_commitment = loader_account_data_hash(&raw_programdata);
    poststate.target_capacity = primary.expected_post_capacity;
    poststate.hard_combined_root = [0; 32];
    poststate.checkpoint_digest = [0; 32];
    poststate.hard_combined_root =
        compute_state_checkpoint_hard_combined_root_v1(&poststate).unwrap();
    poststate.checkpoint_digest = compute_state_checkpoint_digest_v1(&poststate).unwrap();
    poststate
        .validate_schema()
        .expect("typed poststate checkpoint fixture");
    set_account(
        &mut context,
        primary.required_poststate_checkpoint,
        state_account(harness.controller, &poststate),
    );

    let mut failure_state = ProgramDataFailureObservationV1 {
        discriminator: PROGRAMDATA_FAILURE_OBSERVATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_programdata_failure_observation_pda(
            &harness.controller,
            &harness.proposal,
            gate.epoch,
        )
        .1,
        initialized: true,
        finalized: true,
        controller_config: harness.config,
        protocol_gate: harness.gate,
        primary_proposal: harness.proposal,
        target_program: harness.target,
        target_programdata: harness.target_programdata,
        frozen_epoch: gate.epoch,
        actual_program_owner: UPGRADEABLE_LOADER_ID,
        actual_program_executable: true,
        actual_program_data_length: LOADER_PROGRAM_ACCOUNT_LEN as u64,
        program_header_present: true,
        actual_linked_programdata: OptionalPubkeyV1::some(harness.target_programdata).unwrap(),
        raw_hash_complete: true,
        actual_raw_programdata_sha256: loader_account_data_hash(&raw_programdata),
        actual_owner: UPGRADEABLE_LOADER_ID,
        actual_executable: false,
        actual_data_length: raw_programdata.len() as u64,
        programdata_header_present: true,
        actual_programdata_slot: INITIAL_PROGRAMDATA_SLOT,
        actual_capacity: primary.expected_post_capacity,
        actual_authority: OptionalPubkeyV1::some(harness.authority).unwrap(),
        mismatch_class: ProgramDataMismatchClassV1::PayloadLeaf,
        failing_chunk_index: 0,
        expected_leaf_hash: [1; 32],
        actual_leaf_hash: [2; 32],
        finalized_slot: gate.freeze_slot.max(TEST_SLOT),
        observation_digest: [0; 32],
        reserved: [0; PROGRAMDATA_FAILURE_OBSERVATION_V1_RESERVED_LEN],
    };
    failure_state.observation_digest =
        compute_programdata_failure_observation_digest_v1(&failure_state).unwrap();
    failure_state
        .validate_schema()
        .expect("typed ProgramData failure fixture");
    set_account(
        &mut context,
        failure_observation,
        state_account(harness.controller, &failure_state),
    );
    let typed_keys = vec![
        harness.config,
        policy_key,
        harness.council,
        harness.gate,
        harness.proposal,
        primary.rollback_proposal.value,
        harness.verification,
        rollback.buffer_verification,
        primary.prestate_checkpoint,
        primary.required_poststate_checkpoint,
        programdata_verification,
        failure_observation,
    ];
    for key in &typed_keys {
        let account = context
            .banks_client
            .get_account(*key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            account.owner, harness.controller,
            "typed fixture owner {key}"
        );
    }

    let alternate_target = Pubkey::new_unique();
    let alternate_programdata = Pubkey::new_unique();
    let alternate_loader = Pubkey::new_unique();
    let alternate_authority = Pubkey::new_unique();
    let alternate_spill = Pubkey::new_unique();
    let alternate_sysvar = Pubkey::new_unique();
    set_account(
        &mut context,
        alternate_target,
        account(
            UPGRADEABLE_LOADER_ID,
            program_bytes(alternate_programdata),
            true,
        ),
    );
    set_account(
        &mut context,
        alternate_programdata,
        account(
            UPGRADEABLE_LOADER_ID,
            programdata_bytes(
                INITIAL_PROGRAMDATA_SLOT,
                alternate_authority,
                &harness.current_payload,
            ),
            false,
        ),
    );
    set_account(
        &mut context,
        alternate_loader,
        account(system_program::ID, Vec::new(), true),
    );
    for key in [alternate_authority, alternate_spill, alternate_sysvar] {
        set_account(
            &mut context,
            key,
            account(system_program::ID, Vec::new(), false),
        );
    }

    let mut identity_alternates = vec![
        (harness.config, alternate_config),
        (harness.target, alternate_target),
        (harness.target_programdata, alternate_programdata),
        (UPGRADEABLE_LOADER_ID, alternate_loader),
        (harness.authority, alternate_authority),
        (harness.spill, alternate_spill),
        (sysvar_ids::instructions::ID, alternate_sysvar),
    ];
    for canonical in [
        harness.buffer,
        rollback.buffer_pubkey,
        policy_key,
        harness.council,
        harness.gate,
        harness.proposal,
        primary.rollback_proposal.value,
        harness.verification,
        rollback.buffer_verification,
        primary.prestate_checkpoint,
        primary.required_poststate_checkpoint,
        programdata_verification,
        failure_observation,
    ] {
        let alternate = Pubkey::new_unique();
        let canonical_account = context
            .banks_client
            .get_account(canonical)
            .await
            .unwrap()
            .unwrap();
        set_account(&mut context, alternate, canonical_account);
        identity_alternates.push((canonical, alternate));
    }
    let first_chunk_len = artifact.len().min(RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as usize);
    let first_leaf = artifact_chunk_leaf_hash(0, &artifact[..first_chunk_len]).unwrap();
    let unfreeze_expected = UnfreezeExpectationV1 {
        expected_proposal_digest: primary.proposal_digest,
        expected_policy_version: policy.version,
        expected_policy_hash: policy.policy_hash,
        expected_current_council_version: primary.creation_council_version,
        expected_current_council_hash: primary.creation_council_hash,
        expected_frozen_gate_epoch: gate.epoch,
        expected_target_nonce: config.target_nonce,
        expected_proposal_state: primary.state,
        expected_poststate_checkpoint_digest: prestate.checkpoint_digest,
        expected_programdata_authority: harness.authority,
        expected_programdata_deployed_slot: INITIAL_PROGRAMDATA_SLOT,
        expected_programdata_capacity: primary.expected_post_capacity,
        expected_raw_programdata_hash: loader_account_data_hash(&raw_programdata),
        expected_unfreeze_approval_bitset: primary.unfreeze_approval_bitset,
        expected_unfreeze_approval_count: primary.unfreeze_approval_count,
        expected_programdata_verification_finalized_slot: 0,
    };

    let cases: Vec<(&str, Instruction, Vec<&Keypair>, bool)> = vec![
        (
            "tag27-adopt-buffer",
            adopt_buffer_v1_instruction(
                harness.controller,
                AdoptBufferV1Accounts {
                    payer: context.payer.pubkey(),
                    controller_config: harness.config,
                    protocol_gate: harness.gate,
                    proposal: harness.proposal,
                    buffer: harness.buffer,
                    uploader_authority: harness.uploader.pubkey(),
                    authority_pda: harness.authority,
                    buffer_verification: harness.verification,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                    system_program: system_program::ID,
                },
                AdoptBufferV1 {
                    expected: expectation(&primary, &config, &gate),
                },
            ),
            vec![&harness.uploader],
            false,
        ),
        (
            "tag28-verify-buffer-chunk",
            verify_buffer_chunk_v1_instruction(
                harness.controller,
                VerifyBufferChunkV1Accounts {
                    controller_config: harness.config,
                    protocol_gate: harness.gate,
                    proposal: harness.proposal,
                    buffer: harness.buffer,
                    buffer_verification: harness.verification,
                    authority_pda: harness.authority,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                },
                VerifyBufferChunkV1 {
                    expected: expectation(&primary, &config, &gate),
                    chunk_index: 0,
                    proof: fixed_proof(&artifact, 0),
                    expected_verification_status: sealed_verification.status,
                    expected_verified_chunk_bitmap: sealed_verification.verified_chunk_bitmap,
                    expected_verified_chunk_count: sealed_verification.verified_chunk_count,
                },
            ),
            vec![],
            false,
        ),
        (
            "tag29-finalize-buffer-verification",
            finalize_buffer_verification_v1_instruction(
                harness.controller,
                FinalizeBufferVerificationV1Accounts {
                    controller_config: harness.config,
                    protocol_gate: harness.gate,
                    proposal: harness.proposal,
                    buffer: harness.buffer,
                    buffer_verification: harness.verification,
                    authority_pda: harness.authority,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                },
                FinalizeBufferVerificationV1 {
                    expected: expectation(&primary, &config, &gate),
                    expected_verification_status: sealed_verification.status,
                    expected_verified_chunk_bitmap: sealed_verification.verified_chunk_bitmap,
                    expected_verified_chunk_count: sealed_verification.verified_chunk_count,
                },
            ),
            vec![],
            false,
        ),
        (
            "tag30-extend-target",
            extend_instruction(
                &context,
                &harness,
                &config,
                &gate,
                &primary,
                &prestate,
                harness.authority,
                primary.current_capacity,
            ),
            vec![],
            true,
        ),
        (
            "tag31-execute-upgrade",
            execute_instruction(
                &context,
                &harness,
                &config,
                &gate,
                &primary,
                &rollback,
                &prestate,
                &sealed_verification,
                loader_account_data_hash(&raw_programdata),
                INITIAL_PROGRAMDATA_SLOT,
                harness.spill,
            ),
            vec![],
            true,
        ),
        (
            "tag32-verify-programdata-chunk",
            verify_programdata_chunk_v1_instruction(
                harness.controller,
                VerifyProgramDataChunkV1Accounts {
                    controller_config: harness.config,
                    protocol_gate: harness.gate,
                    proposal: harness.proposal,
                    target_program: harness.target,
                    target_programdata: harness.target_programdata,
                    authority_pda: harness.authority,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                    programdata_verification,
                },
                VerifyProgramDataChunkV1 {
                    expected: expectation(&primary, &config, &gate),
                    phase: ProgramDataChunkPhaseV1::Payload,
                    chunk_index: 0,
                    proof: fixed_proof(&artifact, 0),
                    expected_verification_status: ProgramDataVerificationStatusV1::Verifying,
                    expected_verified_payload_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
                    expected_verified_payload_chunk_count: 0,
                    expected_verified_tail_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
                    expected_verified_tail_chunk_count: 0,
                },
            ),
            vec![],
            false,
        ),
        (
            "tag33-finalize-programdata-verification",
            finalize_programdata_verification_v1_instruction(
                harness.controller,
                FinalizeProgramDataVerificationV1Accounts {
                    controller_config: harness.config,
                    protocol_gate: harness.gate,
                    proposal: harness.proposal,
                    target_program: harness.target,
                    target_programdata: harness.target_programdata,
                    authority_pda: harness.authority,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                    programdata_verification,
                },
                FinalizeProgramDataVerificationV1 {
                    expected: expectation(&primary, &config, &gate),
                    expected_verification_status: ProgramDataVerificationStatusV1::Verifying,
                    expected_verified_payload_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
                    expected_verified_payload_chunk_count: 0,
                    expected_verified_tail_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
                    expected_verified_tail_chunk_count: 0,
                    expected_deployed_slot: INITIAL_PROGRAMDATA_SLOT,
                    expected_capacity: primary.expected_post_capacity,
                },
            ),
            vec![],
            false,
        ),
        (
            "tag34-approve-unfreeze",
            approve_unfreeze_v1_instruction(
                harness.controller,
                ApproveUnfreezeV1Accounts {
                    controller_config: harness.config,
                    policy: policy_key,
                    current_council: harness.council,
                    protocol_gate: harness.gate,
                    proposal: harness.proposal,
                    poststate_checkpoint: primary.required_poststate_checkpoint,
                    programdata_verification,
                    target_program: harness.target,
                    target_programdata: harness.target_programdata,
                    authority_pda: harness.authority,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                    seat_authority: harness.seats[0].pubkey(),
                },
                ApproveUnfreezeV1 {
                    expected: unfreeze_expected,
                },
            ),
            vec![&harness.seats[0]],
            false,
        ),
        (
            "tag35-execute-unfreeze",
            execute_unfreeze_v1_instruction(
                harness.controller,
                ExecuteUnfreezeV1Accounts {
                    controller_config: harness.config,
                    policy: policy_key,
                    current_council: harness.council,
                    protocol_gate: harness.gate,
                    proposal: harness.proposal,
                    linked_proposal: primary.rollback_proposal.value,
                    poststate_checkpoint: primary.required_poststate_checkpoint,
                    programdata_verification,
                    target_program: harness.target,
                    target_programdata: harness.target_programdata,
                    authority_pda: harness.authority,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                    instructions_sysvar: sysvar_ids::instructions::ID,
                },
                ExecuteUnfreezeV1 {
                    expected: unfreeze_expected,
                    linked_proposal: primary.rollback_proposal.value,
                    envelope: loader_envelope(),
                },
            ),
            vec![],
            true,
        ),
        (
            "tag36-close-abandoned-buffer",
            close_abandoned_buffer_v1_instruction(
                harness.controller,
                CloseAbandonedBufferV1Accounts {
                    controller_config: harness.config,
                    protocol_gate: harness.gate,
                    proposal: primary.rollback_proposal.value,
                    buffer_verification: rollback.buffer_verification,
                    buffer: rollback.buffer_pubkey,
                    canonical_spill_treasury: harness.spill,
                    authority_pda: harness.authority,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                },
                CloseAbandonedBufferV1 {
                    expected: expectation(&rollback, &config, &gate),
                    expected_verification_status: BufferVerificationStatusV1::Verified,
                    expected_verified_chunk_bitmap: complete_bitmap(rollback.chunk_count),
                    expected_verified_chunk_count: rollback.chunk_count,
                    expected_buffer_verification_finalized_slot: gate.freeze_slot,
                },
            ),
            vec![],
            false,
        ),
        (
            "tag37-activate-rollback",
            activate_rollback_v1_instruction(
                harness.controller,
                ActivateRollbackV1Accounts {
                    controller_config: harness.config,
                    policy: policy_key,
                    protocol_gate: harness.gate,
                    primary_proposal: harness.proposal,
                    rollback_proposal: primary.rollback_proposal.value,
                    rollback_buffer_verification: rollback.buffer_verification,
                    primary_programdata_verification: programdata_verification,
                    failure_evidence: failure_observation,
                    target_program: harness.target,
                    target_programdata: harness.target_programdata,
                    authority_pda: harness.authority,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                },
                ActivateRollbackV1 {
                    expected_primary: expectation(&primary, &config, &gate),
                    expected_rollback: expectation(&rollback, &config, &gate),
                    expected_failure_evidence_digest: [91; 32],
                    expected_primary_programdata_verification_status:
                        ProgramDataVerificationStatusV1::Verifying,
                    expected_primary_programdata_verification_finalized_slot: 0,
                    expected_rollback_buffer_verification_status:
                        BufferVerificationStatusV1::Verified,
                    expected_rollback_verified_chunk_bitmap: complete_bitmap(rollback.chunk_count),
                    expected_rollback_verified_chunk_count: rollback.chunk_count,
                },
            ),
            vec![],
            false,
        ),
        (
            "tag38-observe-programdata-failure",
            observe_programdata_failure_v1_instruction(
                harness.controller,
                ObserveProgramDataFailureV1Accounts {
                    payer: context.payer.pubkey(),
                    controller_config: harness.config,
                    protocol_gate: harness.gate,
                    primary_proposal: harness.proposal,
                    programdata_verification,
                    target_program: harness.target,
                    target_programdata: harness.target_programdata,
                    authority_pda: harness.authority,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                    failure_observation,
                    system_program: system_program::ID,
                },
                ObserveProgramDataFailureV1 {
                    expected: expectation(&primary, &config, &gate),
                    expected_program_owner: UPGRADEABLE_LOADER_ID,
                    expected_program_executable: true,
                    expected_program_data_length: LOADER_PROGRAM_ACCOUNT_LEN as u64,
                    expected_program_header_present: true,
                    expected_linked_programdata: OptionalInstructionPubkeyV1::some(
                        harness.target_programdata,
                    )
                    .unwrap(),
                    expected_programdata_owner: UPGRADEABLE_LOADER_ID,
                    expected_programdata_executable: false,
                    expected_programdata_data_length: raw_programdata.len() as u64,
                    expected_programdata_header_present: true,
                    expected_programdata_slot: INITIAL_PROGRAMDATA_SLOT,
                    expected_raw_hash_complete: true,
                    expected_raw_programdata_hash: loader_account_data_hash(&raw_programdata),
                    expected_capacity: primary.expected_post_capacity,
                    expected_programdata_authority: OptionalInstructionPubkeyV1::some(
                        harness.authority,
                    )
                    .unwrap(),
                    mismatch_class: ProgramDataMismatchClassV1::PayloadLeaf,
                    failing_chunk_index: 0,
                    expected_leaf_hash: first_leaf,
                    proof: fixed_proof(&artifact, 0),
                },
            ),
            vec![],
            false,
        ),
    ];

    let payer_key = context.payer.pubkey();
    for (name, valid_shape, signers, enveloped) in &cases {
        let writable = valid_shape
            .accounts
            .iter()
            .filter(|meta| meta.is_writable)
            .map(|meta| meta.pubkey)
            .collect::<Vec<_>>();

        let mut wrong_count = valid_shape.clone();
        let removable = wrong_count
            .accounts
            .iter()
            .position(|meta| !meta.is_signer && !meta.is_writable)
            .unwrap_or_else(|| panic!("{name} lacks removable readonly role"));
        wrong_count.accounts.remove(removable);
        assert_loader_matrix_failure(&mut context, wrong_count, signers, &writable, *enveloped)
            .await;

        let mut extra_account = valid_shape.clone();
        extra_account
            .accounts
            .push(solana_program::instruction::AccountMeta::new_readonly(
                alternate_authority,
                false,
            ));
        assert_loader_matrix_failure(&mut context, extra_account, signers, &writable, *enveloped)
            .await;

        let config_index = valid_shape
            .accounts
            .iter()
            .position(|meta| meta.pubkey == harness.config)
            .unwrap_or_else(|| panic!("{name} lacks controller config"));
        let writable_index = valid_shape
            .accounts
            .iter()
            .position(|meta| meta.is_writable && meta.pubkey != harness.config)
            .unwrap_or_else(|| panic!("{name} lacks writable role"));
        let mut wrong_order = valid_shape.clone();
        wrong_order.accounts.swap(config_index, writable_index);
        assert_loader_matrix_failure(&mut context, wrong_order, signers, &writable, *enveloped)
            .await;

        let privilege_indices = [true, false].map(|expected_writable| {
            valid_shape
                .accounts
                .iter()
                .enumerate()
                .find(|(_, meta)| {
                    meta.is_writable == expected_writable
                        && meta.pubkey != payer_key
                        && ![
                            harness.controller,
                            harness.target,
                            UPGRADEABLE_LOADER_ID,
                            system_program::ID,
                        ]
                        .contains(&meta.pubkey)
                })
                .map(|(index, _)| index)
                .unwrap_or_else(|| panic!("{name} lacks {expected_writable} writable role"))
        });
        for index in privilege_indices {
            let meta = &valid_shape.accounts[index];
            let mut wrong_writable = valid_shape.clone();
            wrong_writable.accounts[index].is_writable = !meta.is_writable;
            assert_loader_matrix_failure(
                &mut context,
                wrong_writable,
                signers,
                &writable,
                *enveloped,
            )
            .await;
        }

        for (signer_index, signer_meta) in valid_shape
            .accounts
            .iter()
            .enumerate()
            .filter(|(_, meta)| meta.is_signer && meta.pubkey != payer_key)
        {
            let mut wrong_signer = valid_shape.clone();
            wrong_signer.accounts[signer_index].is_signer = false;
            let remaining_signers = signers
                .iter()
                .copied()
                .filter(|signer| signer.pubkey() != signer_meta.pubkey)
                .collect::<Vec<_>>();
            assert_loader_matrix_failure(
                &mut context,
                wrong_signer,
                &remaining_signers,
                &writable,
                *enveloped,
            )
            .await;
        }

        let executable_index = valid_shape
            .accounts
            .iter()
            .position(|meta| typed_keys.contains(&meta.pubkey))
            .unwrap_or_else(|| panic!("{name} lacks a typed non-executable role"));
        let mut executable_alias = valid_shape.clone();
        executable_alias.accounts[executable_index].pubkey = harness.target;
        assert_loader_matrix_failure(
            &mut context,
            executable_alias,
            signers,
            &writable,
            *enveloped,
        )
        .await;

        let duplicate_pair = valid_shape
            .accounts
            .iter()
            .enumerate()
            .find_map(|(left, left_meta)| {
                valid_shape
                    .accounts
                    .iter()
                    .enumerate()
                    .skip(left + 1)
                    .find(|(_, right_meta)| {
                        left_meta.is_writable == right_meta.is_writable
                            && left_meta.is_signer == right_meta.is_signer
                    })
                    .map(|(right, _)| (left, right))
            })
            .unwrap_or_else(|| panic!("{name} lacks same-privilege duplicate roles"));
        let mut duplicate = valid_shape.clone();
        duplicate.accounts[duplicate_pair.1].pubkey = duplicate.accounts[duplicate_pair.0].pubkey;
        assert_loader_matrix_failure(&mut context, duplicate, signers, &writable, *enveloped).await;

        let mut exercised_identity = false;
        for (index, meta) in valid_shape.accounts.iter().enumerate() {
            let Some((_, alternate)) = identity_alternates
                .iter()
                .find(|(canonical, _)| *canonical == meta.pubkey)
            else {
                continue;
            };
            exercised_identity = true;
            let mut alternate_identity = valid_shape.clone();
            alternate_identity.accounts[index].pubkey = *alternate;
            assert_loader_matrix_failure(
                &mut context,
                alternate_identity,
                signers,
                &writable,
                *enveloped,
            )
            .await;
        }
        assert!(exercised_identity, "{name} lacks an identity-bound role");
    }

    for typed_key in typed_keys {
        let canonical_account = context
            .banks_client
            .get_account(typed_key)
            .await
            .unwrap()
            .unwrap();
        for (corruption, corrupted_account) in
            loader_typed_account_header_corruptions(&canonical_account)
        {
            set_account(&mut context, typed_key, corrupted_account);
            let (name, instruction, signers, enveloped) = cases
                .iter()
                .find(|(_, instruction, _, _)| {
                    instruction
                        .accounts
                        .iter()
                        .any(|meta| meta.pubkey == typed_key)
                })
                .unwrap_or_else(|| panic!("typed fixture {typed_key} is not in the matrix"));
            let writable = instruction
                .accounts
                .iter()
                .filter(|meta| meta.is_writable)
                .map(|meta| meta.pubkey)
                .collect::<Vec<_>>();
            assert_loader_matrix_failure(
                &mut context,
                instruction.clone(),
                signers,
                &writable,
                *enveloped,
            )
            .await;
            let _ = (corruption, name);
            set_account(&mut context, typed_key, canonical_account.clone());
        }
    }
}

fn loader_envelope() -> EnvelopeExpectationV1 {
    EnvelopeExpectationV1 {
        compute_unit_limit: 1_400_000,
        compute_unit_price_micro_lamports: 0,
        durable_nonce_account: OptionalInstructionPubkeyV1::none(),
        durable_nonce_authority: OptionalInstructionPubkeyV1::none(),
    }
}

#[tokio::test]
#[ignore = "requires AMOEBA_LOADER_TEST_ARTIFACT pointing to a local SBF ELF"]
async fn real_loader_rejects_invalid_elf_without_partial_state() {
    let current_payload = read_attested_loader_artifact();
    let artifact = vec![0xA5; current_payload.len()];
    let (mut context, harness) = start_buffer_harness(
        artifact,
        current_payload,
        0,
        ROLLBACK_ARTIFACT.to_vec(),
        true,
        false,
    )
    .await;
    let (sealed_primary, sealed_verification) = seal_buffer(&mut context, &harness).await;
    let (config, _policy, gate, primary, rollback, prestate) =
        seed_frozen_loader_state(&mut context, &harness, sealed_primary).await;

    let proposal_before = bytes(&mut context, harness.proposal).await;
    let gate_before = bytes(&mut context, harness.gate).await;
    let buffer_before = bytes(&mut context, harness.buffer).await;
    let buffer_verification_before = bytes(&mut context, harness.verification).await;
    let programdata_before = bytes(&mut context, harness.target_programdata).await;
    let spill_before = bytes(&mut context, harness.spill).await;
    assert!(context
        .banks_client
        .get_account(primary.programdata_verification)
        .await
        .expect("programdata verification read")
        .is_none());

    let upgrade = execute_instruction(
        &context,
        &harness,
        &config,
        &gate,
        &primary,
        &rollback,
        &prestate,
        &sealed_verification,
        loader_account_data_hash(&programdata_before),
        INITIAL_PROGRAMDATA_SLOT,
        harness.spill,
    );
    let [limit, price] = envelope_prefix();
    assert!(
        submit(&mut context, &[limit, price, upgrade], &[])
            .await
            .is_err(),
        "the real loader must reject non-ELF candidate bytes"
    );

    assert_eq!(bytes(&mut context, harness.proposal).await, proposal_before);
    assert_eq!(bytes(&mut context, harness.gate).await, gate_before);
    assert_eq!(bytes(&mut context, harness.buffer).await, buffer_before);
    assert_eq!(
        bytes(&mut context, harness.verification).await,
        buffer_verification_before
    );
    assert_eq!(
        bytes(&mut context, harness.target_programdata).await,
        programdata_before
    );
    assert_eq!(bytes(&mut context, harness.spill).await, spill_before);
    assert!(context
        .banks_client
        .get_account(primary.programdata_verification)
        .await
        .expect("programdata verification read")
        .is_none());
}

fn envelope_prefix() -> [Instruction; 2] {
    [
        ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
        ComputeBudgetInstruction::set_compute_unit_price(0),
    ]
}

async fn clock_slot(context: &mut ProgramTestContext) -> u64 {
    context
        .banks_client
        .get_sysvar::<solana_sdk::clock::Clock>()
        .await
        .expect("clock")
        .slot
}

async fn set_clock_slot(context: &mut ProgramTestContext, slot: u64) {
    assert!(slot > clock_slot(context).await);
    let mut clock = context
        .banks_client
        .get_sysvar::<solana_sdk::clock::Clock>()
        .await
        .expect("clock");
    clock.slot = slot;
    context.set_sysvar(&clock);
    assert_eq!(clock_slot(context).await, slot);
}

async fn approve_and_queue_proposal(
    context: &mut ProgramTestContext,
    harness: &BufferHarness,
    policy: Pubkey,
    proposal_key: Pubkey,
) -> UpgradeProposalV2 {
    for index in 0..3 {
        let proposal: UpgradeProposalV2 = state(context, proposal_key).await;
        let config: ControllerConfigV1 = state(context, harness.config).await;
        let gate: ProtocolGateV1 = state(context, harness.gate).await;
        let approve = approve_proposal_v2_instruction(
            harness.controller,
            ApproveProposalV2Accounts {
                controller_config: harness.config,
                policy,
                council: harness.council,
                protocol_gate: harness.gate,
                proposal: proposal_key,
                seat_authority: harness.seats[index].pubkey(),
            },
            ApproveProposalV2 {
                expected: expectation(&proposal, &config, &gate),
                expected_approval_bitset: proposal.council_approval_bitset,
                expected_approval_count: proposal.council_approval_count,
            },
        );
        let [limit, price] = envelope_prefix();
        submit(context, &[limit, price, approve], &[&harness.seats[index]])
            .await
            .expect("actual-SBF proposal approval");
    }
    let approved: UpgradeProposalV2 = state(context, proposal_key).await;
    assert_eq!(approved.state, ProposalStateV2::CouncilApproved);
    assert_eq!(approved.council_approval_bitset, 0b0_0111);
    assert_eq!(approved.council_approval_count, 3);
    let config: ControllerConfigV1 = state(context, harness.config).await;
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let finalize = finalize_governance_v2_instruction(
        harness.controller,
        FinalizeGovernanceV2Accounts {
            controller_config: harness.config,
            policy,
            council: harness.council,
            protocol_gate: harness.gate,
            proposal: proposal_key,
        },
        FinalizeGovernanceV2 {
            expected: expectation(&approved, &config, &gate),
            expected_approval_bitset: approved.council_approval_bitset,
            expected_approval_count: approved.council_approval_count,
        },
    );
    let [limit, price] = envelope_prefix();
    submit(context, &[limit, price, finalize], &[])
        .await
        .expect("actual-SBF governance finalization");
    let satisfied: UpgradeProposalV2 = state(context, proposal_key).await;
    assert_eq!(satisfied.state, ProposalStateV2::GovernanceSatisfied);
    let queue = queue_proposal_v2_instruction(
        harness.controller,
        QueueProposalV2Accounts {
            controller_config: harness.config,
            policy,
            protocol_gate: harness.gate,
            proposal: proposal_key,
        },
        QueueProposalV2 {
            expected: expectation(&satisfied, &config, &gate),
        },
    );
    let [limit, price] = envelope_prefix();
    submit(context, &[limit, price, queue], &[])
        .await
        .expect("actual-SBF proposal queue");
    let queued: UpgradeProposalV2 = state(context, proposal_key).await;
    assert_eq!(queued.state, ProposalStateV2::Timelocked);
    queued
}

#[allow(clippy::too_many_arguments)]
async fn attest_and_finalize_checkpoint(
    context: &mut ProgramTestContext,
    harness: &BufferHarness,
    policy: Pubkey,
    proposal_key: Pubkey,
    proposal: &UpgradeProposalV2,
    council: &GovernanceCouncilSetV1,
    candidate: CheckpointCandidateV1,
    phase_evidence: Pubkey,
    baseline_checkpoint: Option<Pubkey>,
) -> StateCheckpointV1 {
    let checkpoint_phase = match candidate.phase {
        StateCheckpointPhaseV1::Prestate => CheckpointPhaseV1::Prestate,
        StateCheckpointPhaseV1::Poststate => CheckpointPhaseV1::Poststate,
        StateCheckpointPhaseV1::Emergency => {
            unreachable!("code-upgrade harness does not create emergency checkpoints")
        }
    };
    let checkpoint = derive_checkpoint_pda(&harness.controller, &proposal_key, checkpoint_phase).0;
    let attestations = std::array::from_fn(|index| {
        derive_checkpoint_attestation_pda(
            &harness.controller,
            &checkpoint,
            council.version,
            index as u8,
        )
        .0
    });
    for (index, attestation) in attestations.iter().enumerate().take(3) {
        let create = create_checkpoint_attestation_v1_instruction(
            harness.controller,
            CreateCheckpointAttestationV1Accounts {
                payer: context.payer.pubkey(),
                controller_config: harness.config,
                policy,
                council: harness.council,
                protocol_gate: harness.gate,
                subject: proposal_key,
                checkpoint,
                checkpoint_attestation: *attestation,
                seat_authority: harness.seats[index].pubkey(),
                system_program: system_program::ID,
            },
            CreateCheckpointAttestationV1 {
                candidate,
                expected_council_version: council.version,
                expected_council_hash: council.set_hash,
                seat_index: index as u8,
            },
        );
        let [limit, price] = envelope_prefix();
        submit(context, &[limit, price, create], &[&harness.seats[index]])
            .await
            .expect("actual-SBF checkpoint attestation");
    }
    let finalize = finalize_checkpoint_v1_instruction(
        harness.controller,
        FinalizeCheckpointV1Accounts {
            payer: context.payer.pubkey(),
            controller_config: harness.config,
            policy,
            council: harness.council,
            protocol_gate: harness.gate,
            subject: proposal_key,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            phase_evidence,
            baseline_checkpoint,
            checkpoint,
            checkpoint_attestations: attestations,
            system_program: system_program::ID,
        },
        FinalizeCheckpointV1 {
            candidate,
            expected_council_version: council.version,
            expected_council_hash: council.set_hash,
        },
    );
    let [limit, price] = envelope_prefix();
    submit(context, &[limit, price, finalize], &[])
        .await
        .expect("actual-SBF checkpoint finalization");
    let finalized: StateCheckpointV1 = state(context, checkpoint).await;
    assert!(finalized.accepted);
    assert_eq!(finalized.approval_bitset, 0b0_0111);
    assert_eq!(finalized.approval_count, 3);
    assert_eq!(finalized.proposal, proposal_key);
    assert_eq!(finalized.subject_digest, proposal.proposal_digest);
    finalized
}

#[allow(clippy::too_many_arguments)]
fn extend_instruction(
    context: &ProgramTestContext,
    harness: &BufferHarness,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    proposal: &UpgradeProposalV2,
    prestate: &StateCheckpointV1,
    authority: Pubkey,
    expected_current_capacity: u64,
) -> Instruction {
    extend_target_v1_instruction(
        harness.controller,
        ExtendTargetV1Accounts {
            payer: context.payer.pubkey(),
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: harness.proposal,
            prestate_checkpoint: proposal.prestate_checkpoint,
            target_programdata: harness.target_programdata,
            target_program: harness.target,
            authority_pda: authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            system_program: system_program::ID,
            rent_sysvar: sysvar_ids::rent::ID,
            instructions_sysvar: sysvar_ids::instructions::ID,
        },
        ExtendTargetV1 {
            expected: expectation(proposal, config, gate),
            expected_prestate_checkpoint_digest: prestate.checkpoint_digest,
            expected_current_capacity,
            expected_extension_delta: proposal.extension_delta,
            expected_post_capacity: proposal.expected_post_capacity,
            envelope: loader_envelope(),
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_instruction(
    context: &ProgramTestContext,
    harness: &BufferHarness,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    proposal: &UpgradeProposalV2,
    rollback: &UpgradeProposalV2,
    prestate: &StateCheckpointV1,
    buffer_verification: &BufferVerificationV1,
    current_raw_hash: [u8; 32],
    programdata_slot: u64,
    spill: Pubkey,
) -> Instruction {
    execute_named_upgrade_instruction(
        context,
        harness,
        config,
        gate,
        harness.proposal,
        proposal,
        proposal.rollback_proposal.value,
        rollback,
        proposal.prestate_checkpoint,
        harness.verification,
        harness.buffer,
        prestate,
        buffer_verification,
        current_raw_hash,
        programdata_slot,
        spill,
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_named_upgrade_instruction(
    context: &ProgramTestContext,
    harness: &BufferHarness,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    proposal_key: Pubkey,
    proposal: &UpgradeProposalV2,
    counterpart_key: Pubkey,
    counterpart: &UpgradeProposalV2,
    prestate_key: Pubkey,
    buffer_verification_key: Pubkey,
    buffer: Pubkey,
    prestate: &StateCheckpointV1,
    buffer_verification: &BufferVerificationV1,
    current_raw_hash: [u8; 32],
    programdata_slot: u64,
    spill: Pubkey,
) -> Instruction {
    let expected_counterpart_status =
        if proposal.proposal_class == ProposalClassV1::EmergencyRollback {
            BufferVerificationStatusV1::ConsumedByUpgrade
        } else {
            BufferVerificationStatusV1::Verified
        };
    execute_upgrade_v1_instruction(
        harness.controller,
        ExecuteUpgradeV1Accounts {
            payer: context.payer.pubkey(),
            controller_config: harness.config,
            policy: derive_policy_pda(&harness.controller, &harness.target, 1).0,
            protocol_gate: harness.gate,
            proposal: proposal_key,
            counterpart_proposal: counterpart_key,
            counterpart_buffer_verification: counterpart.buffer_verification,
            prestate_checkpoint: prestate_key,
            buffer_verification: buffer_verification_key,
            programdata_verification: proposal.programdata_verification,
            target_programdata: harness.target_programdata,
            target_program: harness.target,
            buffer,
            canonical_spill_treasury: spill,
            rent_sysvar: sysvar_ids::rent::ID,
            clock_sysvar: sysvar_ids::clock::ID,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            system_program: system_program::ID,
            instructions_sysvar: sysvar_ids::instructions::ID,
        },
        ExecuteUpgradeV1 {
            expected: expectation(proposal, config, gate),
            expected_prestate_checkpoint_digest: prestate.checkpoint_digest,
            expected_current_raw_programdata_hash: current_raw_hash,
            expected_sealed_buffer_header_hash: buffer_verification.sealed_buffer_header_hash,
            expected_counterpart_proposal_digest: counterpart.proposal_digest,
            expected_programdata_slot: programdata_slot,
            expected_capacity: proposal.expected_post_capacity,
            expected_verified_chunk_count: proposal.chunk_count,
            expected_buffer_verification_status: BufferVerificationStatusV1::Verified,
            expected_counterpart_buffer_verification_status: expected_counterpart_status,
            envelope: loader_envelope(),
        },
    )
}

async fn verify_deployed_programdata(
    context: &mut ProgramTestContext,
    harness: &BufferHarness,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    artifact: &[u8],
    deployed_slot: u64,
) -> (UpgradeProposalV2, ProgramDataVerificationV1) {
    verify_named_deployed_programdata(
        context,
        harness,
        config,
        gate,
        harness.proposal,
        artifact,
        deployed_slot,
    )
    .await
}

async fn verify_named_deployed_programdata(
    context: &mut ProgramTestContext,
    harness: &BufferHarness,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    proposal_key: Pubkey,
    artifact: &[u8],
    deployed_slot: u64,
) -> (UpgradeProposalV2, ProgramDataVerificationV1) {
    let executed: UpgradeProposalV2 = state(context, proposal_key).await;
    assert_eq!(executed.state, ProposalStateV2::UpgradeExecuted);
    let mut verification: ProgramDataVerificationV1 =
        state(context, executed.programdata_verification).await;
    for chunk_index in 0..executed.chunk_count {
        let verify = verify_programdata_chunk_v1_instruction(
            harness.controller,
            VerifyProgramDataChunkV1Accounts {
                controller_config: harness.config,
                protocol_gate: harness.gate,
                proposal: proposal_key,
                target_program: harness.target,
                target_programdata: harness.target_programdata,
                authority_pda: harness.authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
                programdata_verification: executed.programdata_verification,
            },
            VerifyProgramDataChunkV1 {
                expected: expectation(&executed, config, gate),
                phase: ProgramDataChunkPhaseV1::Payload,
                chunk_index,
                proof: fixed_proof(artifact, chunk_index),
                expected_verification_status: verification.status,
                expected_verified_payload_chunk_bitmap: verification.verified_payload_chunk_bitmap,
                expected_verified_payload_chunk_count: verification.verified_payload_chunk_count,
                expected_verified_tail_chunk_bitmap: verification.verified_tail_chunk_bitmap,
                expected_verified_tail_chunk_count: verification.verified_tail_chunk_count,
            },
        );
        let [limit, price] = envelope_prefix();
        submit(context, &[limit, price, verify], &[])
            .await
            .expect("actual-SBF deployed payload proof");
        verification = state(context, executed.programdata_verification).await;
    }
    for chunk_index in 0..verification.tail_chunk_count {
        let verify = verify_programdata_chunk_v1_instruction(
            harness.controller,
            VerifyProgramDataChunkV1Accounts {
                controller_config: harness.config,
                protocol_gate: harness.gate,
                proposal: proposal_key,
                target_program: harness.target,
                target_programdata: harness.target_programdata,
                authority_pda: harness.authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
                programdata_verification: executed.programdata_verification,
            },
            VerifyProgramDataChunkV1 {
                expected: expectation(&executed, config, gate),
                phase: ProgramDataChunkPhaseV1::ZeroTail,
                chunk_index,
                proof: FixedMerkleProofV1::empty(),
                expected_verification_status: verification.status,
                expected_verified_payload_chunk_bitmap: verification.verified_payload_chunk_bitmap,
                expected_verified_payload_chunk_count: verification.verified_payload_chunk_count,
                expected_verified_tail_chunk_bitmap: verification.verified_tail_chunk_bitmap,
                expected_verified_tail_chunk_count: verification.verified_tail_chunk_count,
            },
        );
        let [limit, price] = envelope_prefix();
        submit(context, &[limit, price, verify], &[])
            .await
            .expect("actual-SBF deployed zero-tail proof");
        verification = state(context, executed.programdata_verification).await;
    }
    assert_eq!(
        verification.status,
        ProgramDataVerificationStatusV1::ReadyToFinalize
    );
    let finalize = finalize_programdata_verification_v1_instruction(
        harness.controller,
        FinalizeProgramDataVerificationV1Accounts {
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: proposal_key,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            programdata_verification: executed.programdata_verification,
        },
        FinalizeProgramDataVerificationV1 {
            expected: expectation(&executed, config, gate),
            expected_verification_status: verification.status,
            expected_verified_payload_chunk_bitmap: verification.verified_payload_chunk_bitmap,
            expected_verified_payload_chunk_count: verification.verified_payload_chunk_count,
            expected_verified_tail_chunk_bitmap: verification.verified_tail_chunk_bitmap,
            expected_verified_tail_chunk_count: verification.verified_tail_chunk_count,
            expected_deployed_slot: deployed_slot,
            expected_capacity: executed.expected_post_capacity,
        },
    );
    let [limit, price] = envelope_prefix();
    submit(context, &[limit, price, finalize], &[])
        .await
        .expect("actual-SBF ProgramData finalization");
    let proposal: UpgradeProposalV2 = state(context, proposal_key).await;
    let verification: ProgramDataVerificationV1 =
        state(context, executed.programdata_verification).await;
    assert_eq!(proposal.state, ProposalStateV2::ProgramDataVerified);
    assert_eq!(
        verification.status,
        ProgramDataVerificationStatusV1::Verified
    );
    assert_eq!(verification.deployed_slot, deployed_slot);
    assert_eq!(verification.capacity, executed.expected_post_capacity);
    (proposal, verification)
}

/// This runs only when a locally built SBF artifact is explicitly supplied.
/// Example:
///
/// `BPF_OUT_DIR=/tmp/ameba-gov-loader-sbf
///  AMOEBA_LOADER_TEST_ARTIFACT=/tmp/ameba-gov-loader-sbf/upgrade_controller.so cargo test
///  --test loader_program_test -- --ignored --nocapture`
#[tokio::test]
#[ignore = "requires AMOEBA_LOADER_TEST_ARTIFACT pointing to a local SBF ELF"]
async fn real_loader_checked_extend_upgrade_and_programdata_verification() {
    let artifact = read_attested_loader_artifact();
    let extension_delta = 128u64;
    let current_payload = artifact.clone();
    let (mut context, harness) = start_buffer_harness(
        artifact.clone(),
        current_payload,
        extension_delta,
        ROLLBACK_ARTIFACT.to_vec(),
        true,
        false,
    )
    .await;
    let (sealed_primary, sealed_verification) = seal_buffer(&mut context, &harness).await;
    assert_eq!(sealed_primary.state, ProposalStateV2::BufferVerified);
    let (config, policy, gate, primary, rollback, prestate) =
        seed_frozen_loader_state(&mut context, &harness, sealed_primary).await;
    assert_eq!(primary.state, ProposalStateV2::Frozen);

    // Capacity and authority drift fail before CPI and leave all writable state
    // byte-identical.
    let wrong_authority = Pubkey::new_unique();
    set_account(
        &mut context,
        wrong_authority,
        account(system_program::ID, Vec::new(), false),
    );
    let before_proposal = bytes(&mut context, harness.proposal).await;
    let before_programdata = bytes(&mut context, harness.target_programdata).await;
    let wrong_capacity = extend_instruction(
        &context,
        &harness,
        &config,
        &gate,
        &primary,
        &prestate,
        harness.authority,
        primary.current_capacity + 1,
    );
    let [limit, price] = envelope_prefix();
    assert!(submit(&mut context, &[limit, price, wrong_capacity], &[])
        .await
        .is_err());
    assert_eq!(bytes(&mut context, harness.proposal).await, before_proposal);
    assert_eq!(
        bytes(&mut context, harness.target_programdata).await,
        before_programdata
    );
    let wrong_authority_ix = extend_instruction(
        &context,
        &harness,
        &config,
        &gate,
        &primary,
        &prestate,
        wrong_authority,
        primary.current_capacity,
    );
    let [limit, price] = envelope_prefix();
    assert!(
        submit(&mut context, &[limit, price, wrong_authority_ix], &[])
            .await
            .is_err()
    );
    assert_eq!(bytes(&mut context, harness.proposal).await, before_proposal);
    assert_eq!(
        bytes(&mut context, harness.target_programdata).await,
        before_programdata
    );

    let extend = extend_instruction(
        &context,
        &harness,
        &config,
        &gate,
        &primary,
        &prestate,
        harness.authority,
        primary.current_capacity,
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, extend], &[])
        .await
        .expect("real ExtendProgramChecked CPI");
    let extended: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
    assert_eq!(extended.state, ProposalStateV2::Extended);
    assert_eq!(
        bytes(&mut context, harness.target_programdata).await.len(),
        LOADER_PROGRAMDATA_METADATA_LEN + extended.expected_post_capacity as usize
    );
    let extension_slot = extended.extension_executed_slot;
    assert_eq!(extension_slot, clock_slot(&mut context).await);
    let extended_raw =
        loader_account_data_hash(&bytes(&mut context, harness.target_programdata).await);

    let wrong_spill = Pubkey::new_unique();
    set_account(
        &mut context,
        wrong_spill,
        account(system_program::ID, Vec::new(), false),
    );
    let before_upgrade_proposal = bytes(&mut context, harness.proposal).await;
    let before_upgrade_buffer = bytes(&mut context, harness.buffer).await;
    let before_upgrade_programdata = bytes(&mut context, harness.target_programdata).await;
    let wrong_spill_ix = execute_instruction(
        &context,
        &harness,
        &config,
        &gate,
        &extended,
        &rollback,
        &prestate,
        &sealed_verification,
        extended_raw,
        extension_slot,
        wrong_spill,
    );
    let [limit, price] = envelope_prefix();
    assert!(submit(&mut context, &[limit, price, wrong_spill_ix], &[])
        .await
        .is_err());
    assert_eq!(
        bytes(&mut context, harness.proposal).await,
        before_upgrade_proposal
    );
    assert_eq!(
        bytes(&mut context, harness.buffer).await,
        before_upgrade_buffer
    );
    assert_eq!(
        bytes(&mut context, harness.target_programdata).await,
        before_upgrade_programdata
    );

    // The checked extension and upgrade are forbidden in the same slot.
    let same_slot_upgrade = execute_instruction(
        &context,
        &harness,
        &config,
        &gate,
        &extended,
        &rollback,
        &prestate,
        &sealed_verification,
        extended_raw,
        extension_slot,
        harness.spill,
    );
    let [limit, price] = envelope_prefix();
    assert!(
        submit(&mut context, &[limit, price, same_slot_upgrade], &[])
            .await
            .is_err()
    );
    assert_eq!(
        bytes(&mut context, harness.proposal).await,
        before_upgrade_proposal
    );
    assert_eq!(
        bytes(&mut context, harness.buffer).await,
        before_upgrade_buffer
    );

    set_clock_slot(&mut context, extension_slot + 1).await;
    let upgrade_slot = clock_slot(&mut context).await;
    let upgrade = execute_instruction(
        &context,
        &harness,
        &config,
        &gate,
        &extended,
        &rollback,
        &prestate,
        &sealed_verification,
        extended_raw,
        extension_slot,
        harness.spill,
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, upgrade], &[])
        .await
        .expect("one real Loader-v3 Upgrade CPI");
    let executed: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
    assert_eq!(executed.state, ProposalStateV2::UpgradeExecuted);
    assert_eq!(executed.upgrade_executed_slot, upgrade_slot);
    // Loader-v3 drains and truncates the consumed Buffer. ProgramTest may
    // immediately purge a zero-lamport account, so both canonical runtime
    // observations are accepted here.
    if let Some(consumed) = context
        .banks_client
        .get_account(harness.buffer)
        .await
        .expect("buffer read")
    {
        assert_eq!(consumed.lamports, 0);
        assert_eq!(consumed.data.len(), LOADER_BUFFER_METADATA_LEN);
    }
    let deployed = bytes(&mut context, harness.target_programdata).await;
    assert_eq!(
        &deployed
            [LOADER_PROGRAMDATA_METADATA_LEN..LOADER_PROGRAMDATA_METADATA_LEN + artifact.len()],
        artifact.as_slice()
    );

    let mut programdata_verification: ProgramDataVerificationV1 =
        state(&mut context, executed.programdata_verification).await;
    for chunk_index in 0..executed.chunk_count {
        let verify = verify_programdata_chunk_v1_instruction(
            harness.controller,
            VerifyProgramDataChunkV1Accounts {
                controller_config: harness.config,
                protocol_gate: harness.gate,
                proposal: harness.proposal,
                target_program: harness.target,
                target_programdata: harness.target_programdata,
                authority_pda: harness.authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
                programdata_verification: executed.programdata_verification,
            },
            VerifyProgramDataChunkV1 {
                expected: expectation(&executed, &config, &gate),
                phase: ProgramDataChunkPhaseV1::Payload,
                chunk_index,
                proof: fixed_proof(&artifact, chunk_index),
                expected_verification_status: programdata_verification.status,
                expected_verified_payload_chunk_bitmap: programdata_verification
                    .verified_payload_chunk_bitmap,
                expected_verified_payload_chunk_count: programdata_verification
                    .verified_payload_chunk_count,
                expected_verified_tail_chunk_bitmap: programdata_verification
                    .verified_tail_chunk_bitmap,
                expected_verified_tail_chunk_count: programdata_verification
                    .verified_tail_chunk_count,
            },
        );
        submit(&mut context, &[verify], &[])
            .await
            .expect("deployed payload chunk proof");
        programdata_verification = state(&mut context, executed.programdata_verification).await;
    }
    assert_eq!(
        programdata_verification.status,
        ProgramDataVerificationStatusV1::Verifying
    );
    assert_eq!(programdata_verification.tail_chunk_count, 1);

    for chunk_index in 0..programdata_verification.tail_chunk_count {
        let verify = verify_programdata_chunk_v1_instruction(
            harness.controller,
            VerifyProgramDataChunkV1Accounts {
                controller_config: harness.config,
                protocol_gate: harness.gate,
                proposal: harness.proposal,
                target_program: harness.target,
                target_programdata: harness.target_programdata,
                authority_pda: harness.authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
                programdata_verification: executed.programdata_verification,
            },
            VerifyProgramDataChunkV1 {
                expected: expectation(&executed, &config, &gate),
                phase: ProgramDataChunkPhaseV1::ZeroTail,
                chunk_index,
                proof: FixedMerkleProofV1::empty(),
                expected_verification_status: programdata_verification.status,
                expected_verified_payload_chunk_bitmap: programdata_verification
                    .verified_payload_chunk_bitmap,
                expected_verified_payload_chunk_count: programdata_verification
                    .verified_payload_chunk_count,
                expected_verified_tail_chunk_bitmap: programdata_verification
                    .verified_tail_chunk_bitmap,
                expected_verified_tail_chunk_count: programdata_verification
                    .verified_tail_chunk_count,
            },
        );
        submit(&mut context, &[verify], &[])
            .await
            .expect("deployed zero-tail chunk");
        programdata_verification = state(&mut context, executed.programdata_verification).await;
    }
    assert_eq!(
        programdata_verification.status,
        ProgramDataVerificationStatusV1::ReadyToFinalize
    );

    let finalize = finalize_programdata_verification_v1_instruction(
        harness.controller,
        FinalizeProgramDataVerificationV1Accounts {
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: harness.proposal,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            programdata_verification: executed.programdata_verification,
        },
        FinalizeProgramDataVerificationV1 {
            expected: expectation(&executed, &config, &gate),
            expected_verification_status: programdata_verification.status,
            expected_verified_payload_chunk_bitmap: programdata_verification
                .verified_payload_chunk_bitmap,
            expected_verified_payload_chunk_count: programdata_verification
                .verified_payload_chunk_count,
            expected_verified_tail_chunk_bitmap: programdata_verification
                .verified_tail_chunk_bitmap,
            expected_verified_tail_chunk_count: programdata_verification.verified_tail_chunk_count,
            expected_deployed_slot: upgrade_slot,
            expected_capacity: executed.expected_post_capacity,
        },
    );
    submit(&mut context, &[finalize], &[])
        .await
        .expect("mechanical raw ProgramData finalization");
    let verified_proposal: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
    let verified: ProgramDataVerificationV1 =
        state(&mut context, executed.programdata_verification).await;
    assert_eq!(
        verified_proposal.state,
        ProposalStateV2::ProgramDataVerified
    );
    assert_eq!(verified.status, ProgramDataVerificationStatusV1::Verified);
    assert_eq!(
        verified.raw_programdata_hash,
        loader_account_data_hash(&deployed)
    );
    assert!(verified.zero_tail_verified);
    assert_eq!(
        verified.reserved,
        [0; PROGRAMDATA_VERIFICATION_V1_RESERVED_LEN]
    );

    // Seed the already quorum-attested poststate anchor so the closed terminal
    // processors can be exercised independently from the checkpoint-attestor
    // integration test. The anchor preserves every hard prestate root and
    // binds the exact mechanically verified deployed ProgramData.
    let council: GovernanceCouncilSetV1 = state(&mut context, harness.council).await;
    let frozen_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let poststate_slot = verified.finalized_slot + 1;
    set_clock_slot(&mut context, poststate_slot).await;
    let mut poststate_proposal: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
    let poststate = accepted_poststate(
        &harness,
        &poststate_proposal,
        &frozen_gate,
        &verified,
        &prestate,
        &council,
        poststate_slot,
    );
    poststate_proposal.state = ProposalStateV2::PoststateAccepted;
    poststate_proposal.poststate_accepted_slot = poststate_slot;
    poststate_proposal
        .validate_schema()
        .expect("poststate-accepted proposal");
    set_account(
        &mut context,
        poststate_proposal.required_poststate_checkpoint,
        state_account(harness.controller, &poststate),
    );
    set_account(
        &mut context,
        harness.proposal,
        state_account(harness.controller, &poststate_proposal),
    );

    for seat_index in 0..3 {
        let observed: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
        let approve = approve_unfreeze_v1_instruction(
            harness.controller,
            ApproveUnfreezeV1Accounts {
                controller_config: harness.config,
                policy: derive_policy_pda(&harness.controller, &harness.target, 1).0,
                current_council: harness.council,
                protocol_gate: harness.gate,
                proposal: harness.proposal,
                poststate_checkpoint: observed.required_poststate_checkpoint,
                programdata_verification: observed.programdata_verification,
                target_program: harness.target,
                target_programdata: harness.target_programdata,
                authority_pda: harness.authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
                seat_authority: harness.seats[seat_index].pubkey(),
            },
            ApproveUnfreezeV1 {
                expected: unfreeze_expectation(
                    &observed,
                    &config,
                    &policy,
                    &council,
                    &frozen_gate,
                    &poststate,
                    &verified,
                ),
            },
        );
        submit(&mut context, &[approve], &[&harness.seats[seat_index]])
            .await
            .expect("separate current-council unfreeze approval");
    }
    let unfreeze_approved: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
    assert_eq!(unfreeze_approved.state, ProposalStateV2::UnfreezeApproved);
    assert_eq!(unfreeze_approved.unfreeze_approval_bitset, 0b00111);
    assert_eq!(unfreeze_approved.unfreeze_approval_count, 3);

    let unfreeze = execute_unfreeze_v1_instruction(
        harness.controller,
        ExecuteUnfreezeV1Accounts {
            controller_config: harness.config,
            policy: derive_policy_pda(&harness.controller, &harness.target, 1).0,
            current_council: harness.council,
            protocol_gate: harness.gate,
            proposal: harness.proposal,
            linked_proposal: unfreeze_approved.rollback_proposal.value,
            poststate_checkpoint: unfreeze_approved.required_poststate_checkpoint,
            programdata_verification: unfreeze_approved.programdata_verification,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            instructions_sysvar: sysvar_ids::instructions::ID,
        },
        ExecuteUnfreezeV1 {
            expected: unfreeze_expectation(
                &unfreeze_approved,
                &config,
                &policy,
                &council,
                &frozen_gate,
                &poststate,
                &verified,
            ),
            linked_proposal: unfreeze_approved.rollback_proposal.value,
            envelope: loader_envelope(),
        },
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, unfreeze], &[])
        .await
        .expect("separate governed unfreeze");
    let active_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let completed: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
    let retired: UpgradeProposalV2 =
        state(&mut context, unfreeze_approved.rollback_proposal.value).await;
    assert_eq!(active_gate.status, GateStatusV1::Active);
    assert_eq!(active_gate.epoch, frozen_gate.epoch + 1);
    assert_eq!(active_gate.active_proposal, Pubkey::default());
    assert_eq!(active_gate.last_completed_proposal, harness.proposal);
    assert_eq!(completed.state, ProposalStateV2::Completed);
    assert_eq!(retired.state, ProposalStateV2::Retired);

    // The linked rollback Buffer remains controller-owned until its separate
    // terminal close. Tag 36 invokes the real Loader-v3 Close CPI and permits
    // only the canonical spill treasury.
    let rollback_verification: BufferVerificationV1 =
        state(&mut context, retired.buffer_verification).await;
    let treasury_before = context
        .banks_client
        .get_account(harness.spill)
        .await
        .expect("treasury read")
        .expect("treasury account")
        .lamports;
    let close = close_abandoned_buffer_v1_instruction(
        harness.controller,
        CloseAbandonedBufferV1Accounts {
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: unfreeze_approved.rollback_proposal.value,
            buffer_verification: retired.buffer_verification,
            buffer: retired.buffer_pubkey,
            canonical_spill_treasury: harness.spill,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
        },
        CloseAbandonedBufferV1 {
            expected: expectation(&retired, &config, &active_gate),
            expected_verification_status: rollback_verification.status,
            expected_verified_chunk_bitmap: rollback_verification.verified_chunk_bitmap,
            expected_verified_chunk_count: rollback_verification.verified_chunk_count,
            expected_buffer_verification_finalized_slot: rollback_verification.finalized_slot,
        },
    );
    submit(&mut context, &[close], &[])
        .await
        .expect("real Loader-v3 abandoned rollback Buffer close");
    let closed: BufferVerificationV1 = state(&mut context, retired.buffer_verification).await;
    assert_eq!(closed.status, BufferVerificationStatusV1::ClosedAbandoned);
    assert!(closed.terminal_slot >= completed.terminal_slot);
    let treasury_after = context
        .banks_client
        .get_account(harness.spill)
        .await
        .expect("treasury reread")
        .expect("treasury account")
        .lamports;
    assert!(treasury_after > treasury_before);
    if let Some(buffer) = context
        .banks_client
        .get_account(retired.buffer_pubkey)
        .await
        .expect("closed rollback buffer read")
    {
        assert_eq!(buffer.lamports, 0);
    }
}

/// A primary Loader-v3 Upgrade can succeed while later byte verification
/// discovers a recoverable payload mismatch. This rehearsal proves that the
/// frozen controller records that exact mismatch, activates only the already
/// sealed reciprocal rollback, performs a second real Loader-v3 Upgrade, and
/// still requires a fresh accepted poststate plus a separate 3-of-5 unfreeze.
#[tokio::test]
#[ignore = "requires AMOEBA_LOADER_TEST_ARTIFACT pointing to an attested local SBF ELF"]
async fn actual_controller_sbf_recoverable_payload_failure_rolls_back_and_unfreezes() {
    let artifact = read_attested_loader_artifact();
    let (mut context, harness) = start_buffer_harness(
        artifact.clone(),
        artifact.clone(),
        0,
        artifact.clone(),
        true,
        true,
    )
    .await;
    let (sealed_primary, sealed_primary_verification) = seal_buffer(&mut context, &harness).await;
    let (config, policy, primary_gate, primary, rollback, primary_prestate) =
        seed_frozen_loader_state(&mut context, &harness, sealed_primary).await;

    let primary_preupgrade_raw = bytes(&mut context, harness.target_programdata).await;
    let primary_upgrade_slot = clock_slot(&mut context).await;
    let primary_upgrade = execute_instruction(
        &context,
        &harness,
        &config,
        &primary_gate,
        &primary,
        &rollback,
        &primary_prestate,
        &sealed_primary_verification,
        loader_account_data_hash(&primary_preupgrade_raw),
        INITIAL_PROGRAMDATA_SLOT,
        harness.spill,
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, primary_upgrade], &[])
        .await
        .expect("primary real Loader-v3 Upgrade before recoverable failure");
    let executed_primary: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
    let primary_verification: ProgramDataVerificationV1 =
        state(&mut context, executed_primary.programdata_verification).await;
    assert_eq!(executed_primary.state, ProposalStateV2::UpgradeExecuted);
    assert_eq!(executed_primary.upgrade_executed_slot, primary_upgrade_slot);
    assert_eq!(
        primary_verification.status,
        ProgramDataVerificationStatusV1::Verifying
    );

    // Model an independently detected post-loader corruption without changing
    // the Loader-v3 header, authority, capacity, or deployment slot. The first
    // leaf is deliberately changed so no verified-prefix prerequisite exists.
    let mut corrupted_raw = bytes(&mut context, harness.target_programdata).await;
    let original_first_payload_byte = corrupted_raw[LOADER_PROGRAMDATA_METADATA_LEN];
    corrupted_raw[LOADER_PROGRAMDATA_METADATA_LEN] ^= 1;
    set_account(
        &mut context,
        harness.target_programdata,
        account(UPGRADEABLE_LOADER_ID, corrupted_raw.clone(), false),
    );
    let corrupted_payload = &corrupted_raw
        [LOADER_PROGRAMDATA_METADATA_LEN..LOADER_PROGRAMDATA_METADATA_LEN + artifact.len()];
    assert_ne!(corrupted_payload[0], original_first_payload_byte);

    let failure_observation = derive_programdata_failure_observation_pda(
        &harness.controller,
        &harness.proposal,
        primary_gate.epoch,
    )
    .0;
    let first_chunk_len = artifact.len().min(RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as usize);
    let expected_leaf_hash =
        artifact_chunk_leaf_hash(0, &artifact[..first_chunk_len]).expect("first artifact leaf");
    let observe = observe_programdata_failure_v1_instruction(
        harness.controller,
        ObserveProgramDataFailureV1Accounts {
            payer: context.payer.pubkey(),
            controller_config: harness.config,
            protocol_gate: harness.gate,
            primary_proposal: harness.proposal,
            programdata_verification: executed_primary.programdata_verification,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            failure_observation,
            system_program: system_program::ID,
        },
        ObserveProgramDataFailureV1 {
            expected: expectation(&executed_primary, &config, &primary_gate),
            expected_program_owner: UPGRADEABLE_LOADER_ID,
            expected_program_executable: true,
            expected_program_data_length: LOADER_PROGRAM_ACCOUNT_LEN as u64,
            expected_program_header_present: true,
            expected_linked_programdata: OptionalInstructionPubkeyV1::some(
                harness.target_programdata,
            )
            .expect("linked ProgramData"),
            expected_programdata_owner: UPGRADEABLE_LOADER_ID,
            expected_programdata_executable: false,
            expected_programdata_data_length: corrupted_raw.len() as u64,
            expected_programdata_header_present: true,
            expected_programdata_slot: primary_upgrade_slot,
            expected_raw_hash_complete: true,
            expected_raw_programdata_hash: loader_account_data_hash(&corrupted_raw),
            expected_capacity: (corrupted_raw.len() - LOADER_PROGRAMDATA_METADATA_LEN) as u64,
            expected_programdata_authority: OptionalInstructionPubkeyV1::some(harness.authority)
                .expect("controller authority"),
            mismatch_class: ProgramDataMismatchClassV1::PayloadLeaf,
            failing_chunk_index: 0,
            expected_leaf_hash,
            proof: fixed_proof(&artifact, 0),
        },
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, observe], &[])
        .await
        .expect("tag38 exact recoverable payload observation");
    let failure: ProgramDataFailureObservationV1 = state(&mut context, failure_observation).await;
    assert!(failure.finalized);
    assert_eq!(
        failure.mismatch_class,
        ProgramDataMismatchClassV1::PayloadLeaf
    );
    assert_eq!(failure.failing_chunk_index, 0);
    assert_eq!(failure.expected_leaf_hash, expected_leaf_hash);
    assert_ne!(failure.actual_leaf_hash, expected_leaf_hash);
    assert_eq!(
        failure.actual_raw_programdata_sha256,
        loader_account_data_hash(&corrupted_raw)
    );

    let rollback_ready_slot = executed_primary
        .upgrade_executed_slot
        .checked_add(config.rollback_delay_slots)
        .expect("rollback-ready slot")
        .max(rollback.not_before_slot);
    set_clock_slot(&mut context, rollback_ready_slot).await;
    let observed_primary: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
    let observed_rollback: UpgradeProposalV2 =
        state(&mut context, observed_primary.rollback_proposal.value).await;
    let rollback_verification: BufferVerificationV1 =
        state(&mut context, observed_rollback.buffer_verification).await;
    let observed_primary_verification: ProgramDataVerificationV1 =
        state(&mut context, observed_primary.programdata_verification).await;
    let activate = activate_rollback_v1_instruction(
        harness.controller,
        ActivateRollbackV1Accounts {
            controller_config: harness.config,
            policy: derive_policy_pda(&harness.controller, &harness.target, 1).0,
            protocol_gate: harness.gate,
            primary_proposal: harness.proposal,
            rollback_proposal: observed_primary.rollback_proposal.value,
            rollback_buffer_verification: observed_rollback.buffer_verification,
            primary_programdata_verification: observed_primary.programdata_verification,
            failure_evidence: failure_observation,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
        },
        ActivateRollbackV1 {
            expected_primary: expectation(&observed_primary, &config, &primary_gate),
            expected_rollback: expectation(&observed_rollback, &config, &primary_gate),
            expected_failure_evidence_digest: failure.observation_digest,
            expected_primary_programdata_verification_status: observed_primary_verification.status,
            expected_primary_programdata_verification_finalized_slot: observed_primary_verification
                .finalized_slot,
            expected_rollback_buffer_verification_status: rollback_verification.status,
            expected_rollback_verified_chunk_bitmap: rollback_verification.verified_chunk_bitmap,
            expected_rollback_verified_chunk_count: rollback_verification.verified_chunk_count,
        },
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, activate], &[])
        .await
        .expect("tag37 continuously frozen rollback activation");
    let rollback_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let frozen_rollback: UpgradeProposalV2 =
        state(&mut context, observed_primary.rollback_proposal.value).await;
    assert_eq!(rollback_gate.status, GateStatusV1::FrozenForUpgrade);
    assert_eq!(rollback_gate.epoch, primary_gate.epoch + 1);
    assert_eq!(
        rollback_gate.active_proposal,
        observed_primary.rollback_proposal.value
    );
    assert_eq!(frozen_rollback.state, ProposalStateV2::Frozen);
    assert_eq!(frozen_rollback.freeze_gate_epoch, rollback_gate.epoch);

    let council: GovernanceCouncilSetV1 = state(&mut context, harness.council).await;
    let rollback_prestate_candidate = checkpoint_candidate(
        &harness,
        observed_primary.rollback_proposal.value,
        &frozen_rollback,
        &council,
        &rollback_gate,
        StateCheckpointPhaseV1::Prestate,
        clock_slot(&mut context).await,
        primary_upgrade_slot,
        &artifact,
        &corrupted_raw,
        Some(&primary_prestate),
    );
    let rollback_prestate = attest_and_finalize_checkpoint(
        &mut context,
        &harness,
        derive_policy_pda(&harness.controller, &harness.target, 1).0,
        observed_primary.rollback_proposal.value,
        &frozen_rollback,
        &council,
        rollback_prestate_candidate,
        failure_observation,
        Some(observed_primary.prestate_checkpoint),
    )
    .await;
    assert_eq!(rollback_prestate.phase, StateCheckpointPhaseV1::Prestate);
    assert_eq!(
        rollback_prestate.hard_combined_root,
        primary_prestate.hard_combined_root
    );
    assert_eq!(
        rollback_prestate.target_raw_programdata_commitment,
        loader_account_data_hash(&corrupted_raw)
    );

    let rollback_execution_slot = clock_slot(&mut context).await + 1;
    set_clock_slot(&mut context, rollback_execution_slot).await;
    let rollback_upgrade_slot = clock_slot(&mut context).await;
    let current_primary: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
    let current_rollback: UpgradeProposalV2 =
        state(&mut context, observed_primary.rollback_proposal.value).await;
    let current_rollback_verification: BufferVerificationV1 =
        state(&mut context, current_rollback.buffer_verification).await;
    let rollback_upgrade = execute_named_upgrade_instruction(
        &context,
        &harness,
        &config,
        &rollback_gate,
        observed_primary.rollback_proposal.value,
        &current_rollback,
        harness.proposal,
        &current_primary,
        current_rollback.prestate_checkpoint,
        current_rollback.buffer_verification,
        current_rollback.buffer_pubkey,
        &rollback_prestate,
        &current_rollback_verification,
        loader_account_data_hash(&corrupted_raw),
        primary_upgrade_slot,
        harness.spill,
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, rollback_upgrade], &[])
        .await
        .expect("second real Loader-v3 Upgrade restores sealed rollback artifact");
    let rollback_executed: UpgradeProposalV2 =
        state(&mut context, observed_primary.rollback_proposal.value).await;
    assert_eq!(rollback_executed.state, ProposalStateV2::UpgradeExecuted);
    assert_eq!(
        rollback_executed.upgrade_executed_slot,
        rollback_upgrade_slot
    );
    let restored_raw = bytes(&mut context, harness.target_programdata).await;
    assert_eq!(
        &restored_raw
            [LOADER_PROGRAMDATA_METADATA_LEN..LOADER_PROGRAMDATA_METADATA_LEN + artifact.len()],
        artifact.as_slice()
    );

    let (rollback_verified, rollback_programdata_verification) = verify_named_deployed_programdata(
        &mut context,
        &harness,
        &config,
        &rollback_gate,
        observed_primary.rollback_proposal.value,
        &artifact,
        rollback_upgrade_slot,
    )
    .await;
    assert_eq!(
        rollback_verified.state,
        ProposalStateV2::ProgramDataVerified
    );
    assert_eq!(
        rollback_programdata_verification.raw_programdata_hash,
        loader_account_data_hash(&restored_raw)
    );

    set_clock_slot(&mut context, rollback_upgrade_slot + 1).await;
    let rollback_poststate_candidate = checkpoint_candidate(
        &harness,
        observed_primary.rollback_proposal.value,
        &rollback_verified,
        &council,
        &rollback_gate,
        StateCheckpointPhaseV1::Poststate,
        clock_slot(&mut context).await,
        rollback_programdata_verification.deployed_slot,
        &artifact,
        &restored_raw,
        Some(&rollback_prestate),
    );
    let rollback_poststate = attest_and_finalize_checkpoint(
        &mut context,
        &harness,
        derive_policy_pda(&harness.controller, &harness.target, 1).0,
        observed_primary.rollback_proposal.value,
        &rollback_verified,
        &council,
        rollback_poststate_candidate,
        rollback_verified.programdata_verification,
        Some(rollback_verified.prestate_checkpoint),
    )
    .await;
    let poststate_accepted: UpgradeProposalV2 =
        state(&mut context, observed_primary.rollback_proposal.value).await;
    assert_eq!(poststate_accepted.state, ProposalStateV2::PoststateAccepted);
    assert_eq!(
        rollback_poststate.hard_combined_root,
        primary_prestate.hard_combined_root
    );

    for index in 0..3 {
        let observed: UpgradeProposalV2 =
            state(&mut context, observed_primary.rollback_proposal.value).await;
        let approve = approve_unfreeze_v1_instruction(
            harness.controller,
            ApproveUnfreezeV1Accounts {
                controller_config: harness.config,
                policy: derive_policy_pda(&harness.controller, &harness.target, 1).0,
                current_council: harness.council,
                protocol_gate: harness.gate,
                proposal: observed_primary.rollback_proposal.value,
                poststate_checkpoint: observed.required_poststate_checkpoint,
                programdata_verification: observed.programdata_verification,
                target_program: harness.target,
                target_programdata: harness.target_programdata,
                authority_pda: harness.authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
                seat_authority: harness.seats[index].pubkey(),
            },
            ApproveUnfreezeV1 {
                expected: unfreeze_expectation(
                    &observed,
                    &config,
                    &policy,
                    &council,
                    &rollback_gate,
                    &rollback_poststate,
                    &rollback_programdata_verification,
                ),
            },
        );
        let [limit, price] = envelope_prefix();
        submit(
            &mut context,
            &[limit, price, approve],
            &[&harness.seats[index]],
        )
        .await
        .expect("rollback separate unfreeze approval");
    }
    let unfreeze_approved: UpgradeProposalV2 =
        state(&mut context, observed_primary.rollback_proposal.value).await;
    assert_eq!(unfreeze_approved.state, ProposalStateV2::UnfreezeApproved);
    let unfreeze = execute_unfreeze_v1_instruction(
        harness.controller,
        ExecuteUnfreezeV1Accounts {
            controller_config: harness.config,
            policy: derive_policy_pda(&harness.controller, &harness.target, 1).0,
            current_council: harness.council,
            protocol_gate: harness.gate,
            proposal: observed_primary.rollback_proposal.value,
            linked_proposal: harness.proposal,
            poststate_checkpoint: unfreeze_approved.required_poststate_checkpoint,
            programdata_verification: unfreeze_approved.programdata_verification,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            instructions_sysvar: sysvar_ids::instructions::ID,
        },
        ExecuteUnfreezeV1 {
            expected: unfreeze_expectation(
                &unfreeze_approved,
                &config,
                &policy,
                &council,
                &rollback_gate,
                &rollback_poststate,
                &rollback_programdata_verification,
            ),
            linked_proposal: harness.proposal,
            envelope: loader_envelope(),
        },
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, unfreeze], &[])
        .await
        .expect("rollback separate governed unfreeze");
    let final_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let completed_rollback: UpgradeProposalV2 =
        state(&mut context, observed_primary.rollback_proposal.value).await;
    let superseded_primary: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
    assert_eq!(final_gate.status, GateStatusV1::Active);
    assert_eq!(final_gate.epoch, rollback_gate.epoch + 1);
    assert_eq!(
        final_gate.last_completed_proposal,
        observed_primary.rollback_proposal.value
    );
    assert_eq!(completed_rollback.state, ProposalStateV2::Completed);
    assert_eq!(
        superseded_primary.state,
        ProposalStateV2::SupersededByRollback
    );
}

/// Actual-controller-SBF Gate F lifecycle. The controller Program and
/// ProgramData are installed as exact Loader-v3 genesis accounts so the
/// initializer is the real controller upgrade authority. No native controller
/// adapter is registered at any point.
#[tokio::test]
#[ignore = "requires AMOEBA_LOADER_TEST_ARTIFACT pointing to the exact controller SBF ELF"]
async fn actual_controller_sbf_full_governed_loader_lifecycle() {
    let artifact = read_attested_loader_artifact();

    let controller = Pubkey::new_from_array([0xA2; 32]);
    let controller_programdata = derive_upgradeable_programdata_address(&controller).0;
    let target = Pubkey::new_from_array([0xB2; 32]);
    let target_programdata = derive_upgradeable_programdata_address(&target).0;
    let config_key = derive_controller_config_pda(&controller, &target).0;
    let authority = derive_authority_pda(&controller, &target).0;
    let gate_key = derive_gate_pda(&controller, &target).0;
    let policy_key = derive_policy_pda(&controller, &target, 1).0;
    let initializer = Keypair::new();
    let old_target_authority = Keypair::new();
    let old_authority_buffer = Pubkey::new_unique();
    let guardian = Keypair::new();
    let seats = std::array::from_fn(|_| Keypair::new());
    let spill = Pubkey::new_unique();
    let uploader = Keypair::new();
    let buffer = Pubkey::new_unique();
    let rollback_uploader = Keypair::new();
    let rollback_buffer = Pubkey::new_unique();

    let expected_policy = governance_policy(controller, config_key, target);
    let (council_key, expected_council) =
        governance_council(controller, config_key, target, &expected_policy, &seats);

    let mut test = ProgramTest::default();
    test.prefer_bpf(true);
    test.add_account(
        controller,
        account(
            UPGRADEABLE_LOADER_ID,
            program_bytes(controller_programdata),
            true,
        ),
    );
    test.add_account(
        controller_programdata,
        account(
            UPGRADEABLE_LOADER_ID,
            programdata_bytes(INITIAL_PROGRAMDATA_SLOT, initializer.pubkey(), &artifact),
            false,
        ),
    );
    test.add_account(
        buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(uploader.pubkey(), &artifact),
            false,
        ),
    );
    test.add_account(
        rollback_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(rollback_uploader.pubkey(), &artifact),
            false,
        ),
    );
    test.add_account(
        old_authority_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(old_target_authority.pubkey(), &artifact),
            false,
        ),
    );
    for key in [
        initializer.pubkey(),
        old_target_authority.pubkey(),
        guardian.pubkey(),
        spill,
        uploader.pubkey(),
        rollback_uploader.pubkey(),
        authority,
    ] {
        test.add_account(key, account(system_program::ID, Vec::new(), false));
    }
    for seat in &seats {
        test.add_account(
            seat.pubkey(),
            account(system_program::ID, Vec::new(), false),
        );
    }

    let mut context = test.start_with_context().await;
    // Late injection keeps the sacrificial target out of ProgramTest's genesis
    // executable cache. The real controller still observes exact Loader-v3
    // Program/ProgramData accounts during Initialize, and the later real
    // Loader Upgrade is the cache's first deployment of this target identity.
    set_account(
        &mut context,
        target,
        account(
            UPGRADEABLE_LOADER_ID,
            program_bytes(target_programdata),
            true,
        ),
    );
    set_account(
        &mut context,
        target_programdata,
        account(
            UPGRADEABLE_LOADER_ID,
            programdata_bytes(
                INITIAL_PROGRAMDATA_SLOT,
                old_target_authority.pubkey(),
                &artifact,
            ),
            false,
        ),
    );
    context
        .warp_to_slot(2)
        .expect("activate exact genesis controller ELF");
    let mut clock = context
        .banks_client
        .get_sysvar::<solana_sdk::clock::Clock>()
        .await
        .expect("clock");
    clock.slot = TEST_SLOT;
    context.set_sysvar(&clock);

    let initialize = initialize_controller_v1_instruction(
        controller,
        InitializeControllerV1Accounts {
            payer: context.payer.pubkey(),
            initializer: initializer.pubkey(),
            controller_program: controller,
            controller_programdata,
            target_program: target,
            target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            controller_config: config_key,
            authority_pda: authority,
            protocol_gate: gate_key,
            policy: policy_key,
            council: council_key,
            canonical_spill_treasury: spill,
            guardian: guardian.pubkey(),
            seat_authorities: seats.each_ref().map(Signer::pubkey),
            system_program: system_program::ID,
        },
        InitializeControllerV1 {
            cluster_domain: [1; 32],
            initial_policy_version: 1,
            initial_council_version: 1,
            next_proposal_id: 1,
            target_nonce: 1,
            initial_gate_epoch: 1,
            policy_activation_slot: expected_policy.activation_slot,
            routine_delay_slots: 5,
            major_delay_slots: 8,
            rollback_delay_slots: 2,
            terminal_delay_slots: 10,
            vote_review_slots: 8,
            proposal_expiry_slots: 100,
            expected_policy_hash: expected_policy.policy_hash,
            expected_council_hash: expected_council.set_hash,
            seat_terms: [CouncilSeatTermV1 {
                term_start_slot: 1,
                term_end_slot: 10_000,
            }; 5],
        },
    );
    submit(&mut context, &[initialize], &[&initializer])
        .await
        .expect("actual controller SBF initialization");
    let initialized_config: ControllerConfigV1 = state(&mut context, config_key).await;
    let mut bootstrap_gate: ProtocolGateV1 = state(&mut context, gate_key).await;
    assert_eq!(bootstrap_gate.status, GateStatusV1::EmergencyFrozen);
    assert_eq!(initialized_config.next_proposal_id, 1);
    assert_eq!(initialized_config.target_nonce, 1);

    // Exercise the future handoff primitive against the real Loader before the
    // Release 1 lifecycle.  The former authority signs this sacrificial local
    // SetAuthority transaction; the controller authority PDA is deliberately
    // not asked to sign.  The future production ceremony remains out of scope.
    let handoff = set_upgrade_authority(&target, &old_target_authority.pubkey(), Some(&authority));
    submit(&mut context, &[handoff], &[&old_target_authority])
        .await
        .expect("real sacrificial Loader ProgramData authority handoff");
    let handed_off_programdata = bytes(&mut context, target_programdata).await;
    assert_eq!(
        parse_upgradeable_programdata(&handed_off_programdata)
            .expect("handed-off ProgramData header")
            .upgrade_authority,
        Some(authority)
    );

    // Phase 7 bridge activation is intentionally not part of Release 1. This
    // local-only bank patch models only that separately scoped gate activation
    // so the production SBF lifecycle can begin at Active.  Unlike the old
    // fixture, it does not fabricate the ProgramData authority handoff.
    bootstrap_gate.status = GateStatusV1::Active;
    bootstrap_gate.epoch += 1;
    bootstrap_gate.active_proposal = Pubkey::default();
    bootstrap_gate.freeze_slot = 0;
    bootstrap_gate.freeze_reason_code = 0;
    set_account(
        &mut context,
        gate_key,
        state_account(controller, &bootstrap_gate),
    );

    // The sacrificial handoff is one-way: even a genuine Loader-v3 Buffer
    // controlled by the former authority cannot directly replace ProgramData.
    // This is an actual Loader Upgrade attempt, not a controller-side mock.
    let target_before_old_authority_attempt = bytes(&mut context, target).await;
    let programdata_before_old_authority_attempt = bytes(&mut context, target_programdata).await;
    let buffer_before_old_authority_attempt = bytes(&mut context, old_authority_buffer).await;
    let old_authority_upgrade = direct_loader_upgrade(
        &target,
        &old_authority_buffer,
        &old_target_authority.pubkey(),
        &spill,
    );
    assert!(
        submit(
            &mut context,
            std::slice::from_ref(&old_authority_upgrade),
            &[&old_target_authority],
        )
        .await
        .is_err(),
        "former ProgramData authority must not retain direct upgrade power"
    );
    assert_eq!(
        bytes(&mut context, target).await,
        target_before_old_authority_attempt
    );
    assert_eq!(
        bytes(&mut context, target_programdata).await,
        programdata_before_old_authority_attempt
    );
    assert_eq!(
        bytes(&mut context, old_authority_buffer).await,
        buffer_before_old_authority_attempt
    );

    let (proposal_key, expected_proposal) = draft_proposal(
        controller,
        &initialized_config,
        &bootstrap_gate,
        buffer,
        uploader.pubkey(),
        &artifact,
        &artifact,
        0,
        expected_council.set_hash,
        rollback_buffer,
        &artifact,
    );
    let create = create_instruction_from_expected(
        controller,
        context.payer.pubkey(),
        seats[0].pubkey(),
        config_key,
        policy_key,
        council_key,
        gate_key,
        proposal_key,
        &expected_proposal,
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, create], &[&seats[0]])
        .await
        .expect("actual controller SBF proposal creation");
    let created: UpgradeProposalV2 = state(&mut context, proposal_key).await;
    assert_eq!(created, expected_proposal);

    let harness = BufferHarness {
        controller,
        target,
        target_programdata,
        config: config_key,
        gate: gate_key,
        authority,
        spill,
        proposal: proposal_key,
        council: council_key,
        verification: created.buffer_verification,
        buffer,
        uploader,
        seats,
        artifact: artifact.clone(),
        current_payload: artifact.clone(),
        rollback_artifact: artifact.clone(),
    };

    // Create the reciprocal EmergencyRollback late enough that its immutable
    // expiry preserves the required primary-expiry plus rollback-delay runway.
    set_clock_slot(&mut context, TEST_SLOT + 3).await;
    let config_after_primary: ControllerConfigV1 = state(&mut context, config_key).await;
    let active_gate: ProtocolGateV1 = state(&mut context, gate_key).await;
    let (rollback_key, expected_rollback) = draft_rollback_proposal(
        controller,
        &config_after_primary,
        &active_gate,
        proposal_key,
        &created,
        rollback_buffer,
        rollback_uploader.pubkey(),
        &artifact,
        clock_slot(&mut context).await,
        expected_council.set_hash,
    );
    assert_eq!(rollback_key, created.rollback_proposal.value);
    let create_rollback = create_instruction_from_expected(
        controller,
        context.payer.pubkey(),
        harness.seats[1].pubkey(),
        config_key,
        policy_key,
        council_key,
        gate_key,
        rollback_key,
        &expected_rollback,
    );
    let [limit, price] = envelope_prefix();
    submit(
        &mut context,
        &[limit, price, create_rollback],
        &[&harness.seats[1]],
    )
    .await
    .expect("actual controller SBF rollback proposal creation");
    let created_rollback: UpgradeProposalV2 = state(&mut context, rollback_key).await;
    assert_eq!(created_rollback, expected_rollback);

    let (sealed_primary, sealed_primary_verification) = seal_buffer(&mut context, &harness).await;
    let (sealed_rollback, sealed_rollback_verification) = seal_additional_buffer(
        &mut context,
        &harness,
        rollback_key,
        created_rollback.buffer_verification,
        rollback_buffer,
        &rollback_uploader,
        &artifact,
    )
    .await;
    assert_eq!(sealed_primary.state, ProposalStateV2::BufferVerified);
    assert_eq!(sealed_rollback.state, ProposalStateV2::BufferVerified);
    assert_eq!(
        sealed_primary_verification.status,
        BufferVerificationStatusV1::Verified
    );
    assert_eq!(
        sealed_rollback_verification.status,
        BufferVerificationStatusV1::Verified
    );

    set_clock_slot(&mut context, expected_rollback.review_start_slot).await;
    let primary_queued =
        approve_and_queue_proposal(&mut context, &harness, policy_key, proposal_key).await;
    let rollback_queued =
        approve_and_queue_proposal(&mut context, &harness, policy_key, rollback_key).await;
    assert_eq!(primary_queued.state, ProposalStateV2::Timelocked);
    assert_eq!(rollback_queued.state, ProposalStateV2::Timelocked);

    set_clock_slot(&mut context, primary_queued.not_before_slot).await;
    let prefreeze_config: ControllerConfigV1 = state(&mut context, config_key).await;
    let prefreeze_gate: ProtocolGateV1 = state(&mut context, gate_key).await;
    let freeze = freeze_proposal_v2_instruction(
        controller,
        FreezeProposalV2Accounts {
            controller_config: config_key,
            policy: policy_key,
            council: council_key,
            protocol_gate: gate_key,
            proposal: proposal_key,
            target_program: target,
            target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            authority_pda: authority,
            rollback_proposal: rollback_key,
            rollback_buffer_verification: rollback_queued.buffer_verification,
            rollback_buffer,
        },
        FreezeProposalV2 {
            expected: expectation(&primary_queued, &prefreeze_config, &prefreeze_gate),
            expected_next_gate_epoch: prefreeze_gate.epoch + 1,
        },
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, freeze], &[])
        .await
        .expect("actual-SBF governed freeze");
    let frozen_config: ControllerConfigV1 = state(&mut context, config_key).await;
    let frozen_gate: ProtocolGateV1 = state(&mut context, gate_key).await;
    let frozen: UpgradeProposalV2 = state(&mut context, proposal_key).await;
    assert_eq!(frozen.state, ProposalStateV2::Frozen);
    assert_eq!(frozen_config.target_nonce, frozen.target_nonce + 1);
    assert_eq!(frozen_gate.status, GateStatusV1::FrozenForUpgrade);
    assert_eq!(frozen_gate.active_proposal, proposal_key);

    let council: GovernanceCouncilSetV1 = state(&mut context, council_key).await;
    let preupgrade_programdata = bytes(&mut context, target_programdata).await;
    let prestate_candidate = checkpoint_candidate(
        &harness,
        proposal_key,
        &frozen,
        &council,
        &frozen_gate,
        StateCheckpointPhaseV1::Prestate,
        clock_slot(&mut context).await,
        INITIAL_PROGRAMDATA_SLOT,
        &artifact,
        &preupgrade_programdata,
        None,
    );
    let prestate = attest_and_finalize_checkpoint(
        &mut context,
        &harness,
        policy_key,
        proposal_key,
        &frozen,
        &council,
        prestate_candidate,
        frozen.buffer_verification,
        None,
    )
    .await;
    assert_eq!(prestate.phase, StateCheckpointPhaseV1::Prestate);

    set_clock_slot(&mut context, frozen.frozen_slot + 1).await;
    let upgrade_slot = clock_slot(&mut context).await;
    let rollback: UpgradeProposalV2 = state(&mut context, rollback_key).await;
    let upgrade = execute_instruction(
        &context,
        &harness,
        &frozen_config,
        &frozen_gate,
        &frozen,
        &rollback,
        &prestate,
        &sealed_primary_verification,
        loader_account_data_hash(&preupgrade_programdata),
        INITIAL_PROGRAMDATA_SLOT,
        spill,
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, upgrade], &[])
        .await
        .expect("actual controller-SBF typed real Loader-v3 Upgrade CPI");
    let executed: UpgradeProposalV2 = state(&mut context, proposal_key).await;
    assert_eq!(executed.state, ProposalStateV2::UpgradeExecuted);
    assert_eq!(executed.upgrade_executed_slot, upgrade_slot);
    let deployed_programdata = bytes(&mut context, target_programdata).await;
    assert_eq!(
        &deployed_programdata
            [LOADER_PROGRAMDATA_METADATA_LEN..LOADER_PROGRAMDATA_METADATA_LEN + artifact.len()],
        artifact.as_slice()
    );

    let (programdata_verified, verification) = verify_deployed_programdata(
        &mut context,
        &harness,
        &frozen_config,
        &frozen_gate,
        &artifact,
        upgrade_slot,
    )
    .await;
    assert_eq!(
        verification.raw_programdata_hash,
        loader_account_data_hash(&deployed_programdata)
    );

    set_clock_slot(&mut context, upgrade_slot + 1).await;
    let poststate_candidate = checkpoint_candidate(
        &harness,
        proposal_key,
        &programdata_verified,
        &council,
        &frozen_gate,
        StateCheckpointPhaseV1::Poststate,
        clock_slot(&mut context).await,
        verification.deployed_slot,
        &artifact,
        &deployed_programdata,
        Some(&prestate),
    );
    let poststate = attest_and_finalize_checkpoint(
        &mut context,
        &harness,
        policy_key,
        proposal_key,
        &programdata_verified,
        &council,
        poststate_candidate,
        programdata_verified.programdata_verification,
        Some(programdata_verified.prestate_checkpoint),
    )
    .await;
    assert_eq!(poststate.phase, StateCheckpointPhaseV1::Poststate);
    assert_eq!(poststate.hard_combined_root, prestate.hard_combined_root);
    let poststate_accepted: UpgradeProposalV2 = state(&mut context, proposal_key).await;
    assert_eq!(poststate_accepted.state, ProposalStateV2::PoststateAccepted);

    let policy: GovernancePolicyV1 = state(&mut context, policy_key).await;
    for index in 0..3 {
        let observed: UpgradeProposalV2 = state(&mut context, proposal_key).await;
        let approve = approve_unfreeze_v1_instruction(
            controller,
            ApproveUnfreezeV1Accounts {
                controller_config: config_key,
                policy: policy_key,
                current_council: council_key,
                protocol_gate: gate_key,
                proposal: proposal_key,
                poststate_checkpoint: observed.required_poststate_checkpoint,
                programdata_verification: observed.programdata_verification,
                target_program: target,
                target_programdata,
                authority_pda: authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
                seat_authority: harness.seats[index].pubkey(),
            },
            ApproveUnfreezeV1 {
                expected: unfreeze_expectation(
                    &observed,
                    &frozen_config,
                    &policy,
                    &council,
                    &frozen_gate,
                    &poststate,
                    &verification,
                ),
            },
        );
        let [limit, price] = envelope_prefix();
        submit(
            &mut context,
            &[limit, price, approve],
            &[&harness.seats[index]],
        )
        .await
        .expect("actual-SBF separate unfreeze approval");
    }
    let unfreeze_approved: UpgradeProposalV2 = state(&mut context, proposal_key).await;
    assert_eq!(unfreeze_approved.state, ProposalStateV2::UnfreezeApproved);
    let unfreeze = execute_unfreeze_v1_instruction(
        controller,
        ExecuteUnfreezeV1Accounts {
            controller_config: config_key,
            policy: policy_key,
            current_council: council_key,
            protocol_gate: gate_key,
            proposal: proposal_key,
            linked_proposal: rollback_key,
            poststate_checkpoint: unfreeze_approved.required_poststate_checkpoint,
            programdata_verification: unfreeze_approved.programdata_verification,
            target_program: target,
            target_programdata,
            authority_pda: authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            instructions_sysvar: sysvar_ids::instructions::ID,
        },
        ExecuteUnfreezeV1 {
            expected: unfreeze_expectation(
                &unfreeze_approved,
                &frozen_config,
                &policy,
                &council,
                &frozen_gate,
                &poststate,
                &verification,
            ),
            linked_proposal: rollback_key,
            envelope: loader_envelope(),
        },
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, unfreeze], &[])
        .await
        .expect("actual controller-SBF separate governed unfreeze");
    let final_gate: ProtocolGateV1 = state(&mut context, gate_key).await;
    let completed: UpgradeProposalV2 = state(&mut context, proposal_key).await;
    let retired_rollback: UpgradeProposalV2 = state(&mut context, rollback_key).await;
    assert_eq!(final_gate.status, GateStatusV1::Active);
    assert_eq!(final_gate.epoch, frozen_gate.epoch + 1);
    assert_eq!(final_gate.active_proposal, Pubkey::default());
    assert_eq!(final_gate.last_completed_proposal, proposal_key);
    assert_eq!(completed.state, ProposalStateV2::Completed);
    assert_eq!(retired_rollback.state, ProposalStateV2::Retired);
}

fn differential_model_apply(model: &mut Release1Model, action: Release1ModelAction) {
    assert_eq!(
        model.apply(action).expect("differential model action"),
        Release1ModelOutcome::Applied
    );
}

fn differential_model_atomic_failure(model: &mut Release1Model, action: Release1ModelAction) {
    let before = model.clone();
    let before_bytes = before.try_to_vec().expect("serialize model prestate");
    assert!(model.apply(action).is_err());
    assert_eq!(*model, before);
    assert_eq!(
        model.try_to_vec().expect("serialize model failure state"),
        before_bytes
    );
}

fn loader_model_hard_state() -> ModelHardStateObservation {
    let mut value = ModelHardStateObservation {
        schema_identifier: [20; 32],
        program_owned_root: [31; 32],
        program_owned_count: 5,
        logical_compressed_root: [32; 32],
        logical_compressed_count: 6,
        semantic_custody_root: [33; 32],
        custody_identity_root: [34; 32],
        hard_combined_root: [0; 32],
    };
    value.hard_combined_root = value.recompute_hard_combined_root();
    value
}

fn active_loader_model(
    harness: &BufferHarness,
    config: &ControllerConfigV1,
    council: &GovernanceCouncilSetV1,
) -> Release1Model {
    let pre_handoff_authority = Pubkey::new_from_array([0xee; 32]);
    let pre_handoff_programdata = ModelProgramDataObservation {
        slot: INITIAL_PROGRAMDATA_SLOT,
        payload_hash: hashv(&[&harness.current_payload]).to_bytes(),
        raw_hash: loader_account_data_hash(&programdata_bytes(
            INITIAL_PROGRAMDATA_SLOT,
            pre_handoff_authority,
            &harness.current_payload,
        )),
        capacity: harness.current_payload.len() as u64,
        authority: pre_handoff_authority,
    };
    let mut model = Release1Model::default();
    let initialization = ModelInitialization {
        slot: council.activation_slot,
        graph: ModelIdentityGraph {
            controller_program: harness.controller,
            controller_programdata: derive_upgradeable_programdata_address(&harness.controller).0,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            authority_pda: harness.authority,
            gate_pda: harness.gate,
            policy_pda: derive_policy_pda(&harness.controller, &harness.target, 1).0,
            council_pda: harness.council,
            canonical_spill_treasury: harness.spill,
            guardian: config.guardian,
        },
        seats: council.seats.map(|seat| seat.seat_authority),
        seat_terms: council.seats.map(|seat| ModelSeatTerm {
            start_slot: seat.term_start_slot,
            end_slot: seat.term_end_slot,
        }),
        delays: ModelDelays {
            review_slots: config.vote_review_slots,
            rollback_slots: config.rollback_delay_slots,
            routine_slots: config.routine_delay_slots,
            major_slots: config.major_delay_slots,
            terminal_slots: config.terminal_delay_slots,
            proposal_expiry_slots: config.proposal_expiry_slots,
        },
        programdata: pre_handoff_programdata,
        controller_programdata_linked: true,
        initializer_is_controller_upgrade_authority: true,
        target_programdata_linked: true,
        canonical_pdas_verified: true,
        seat_accounts_readonly: true,
        seat_accounts_nonexecutable: true,
    };
    differential_model_apply(
        &mut model,
        Release1ModelAction::Initialize(Box::new(initialization)),
    );
    differential_model_apply(
        &mut model,
        Release1ModelAction::SetProgramDataObservation(ModelProgramDataObservation {
            slot: INITIAL_PROGRAMDATA_SLOT,
            payload_hash: hashv(&[&harness.current_payload]).to_bytes(),
            raw_hash: loader_account_data_hash(&programdata_bytes(
                INITIAL_PROGRAMDATA_SLOT,
                harness.authority,
                &harness.current_payload,
            )),
            capacity: harness.current_payload.len() as u64,
            authority: harness.authority,
        }),
    );
    differential_model_apply(
        &mut model,
        Release1ModelAction::SimulatePostHandoffActivation {
            slot: TEST_SLOT,
            bridge_and_authority_graph_verified: true,
        },
    );
    model
}

fn prepare_model_proposal_for_freeze(model: &mut Release1Model, proposal: &UpgradeProposalV2) {
    let proposal_id = proposal.proposal_id;
    differential_model_apply(model, Release1ModelAction::AdoptBuffer { proposal_id });
    differential_model_apply(model, Release1ModelAction::VerifyBuffer { proposal_id });
    for seat in 0..3 {
        differential_model_apply(
            model,
            Release1ModelAction::ApproveProposal {
                proposal_id,
                seat,
                slot: proposal.review_start_slot,
            },
        );
    }
    differential_model_apply(
        model,
        Release1ModelAction::SatisfyGovernance {
            proposal_id,
            slot: proposal.review_start_slot,
        },
    );
    differential_model_apply(
        model,
        Release1ModelAction::QueueProposal {
            proposal_id,
            slot: proposal.review_start_slot,
        },
    );
}

fn prepare_loader_model_at_accepted_prestate(
    harness: &BufferHarness,
    config: &ControllerConfigV1,
    council: &GovernanceCouncilSetV1,
    primary: &UpgradeProposalV2,
    rollback: &UpgradeProposalV2,
    gate: &ProtocolGateV1,
) -> Release1Model {
    let mut model = active_loader_model(harness, config, council);
    let primary_id = match model
        .apply(Release1ModelAction::CreateProposal(ModelProposalRequest {
            class: primary.proposal_class,
            extension_required: primary.extension_delta != 0,
            primary_proposal: None,
            rollback_proposal: Some(rollback.proposal_id),
            proposer_seat: 0,
            proposer_signed: true,
            payer_is_separate: true,
            slot: primary.creation_slot,
        }))
        .expect("model primary creation")
    {
        Release1ModelOutcome::ProposalCreated(id) => id,
        outcome => panic!("unexpected primary model outcome: {outcome:?}"),
    };
    prepare_model_proposal_for_freeze(&mut model, primary);
    let rollback_id = match model
        .apply(Release1ModelAction::CreateProposal(ModelProposalRequest {
            class: rollback.proposal_class,
            extension_required: false,
            primary_proposal: Some(primary.proposal_id),
            rollback_proposal: None,
            proposer_seat: 0,
            proposer_signed: true,
            payer_is_separate: true,
            slot: rollback.creation_slot,
        }))
        .expect("model rollback creation")
    {
        Release1ModelOutcome::ProposalCreated(id) => id,
        outcome => panic!("unexpected rollback model outcome: {outcome:?}"),
    };
    assert_eq!((primary_id, rollback_id), (1, 2));
    prepare_model_proposal_for_freeze(&mut model, rollback);
    differential_model_apply(
        &mut model,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary.proposal_id,
            slot: primary.frozen_slot,
        },
    );
    for seat in 0..3 {
        differential_model_apply(
            &mut model,
            Release1ModelAction::AttestCheckpoint {
                proposal_id: primary.proposal_id,
                phase: StateCheckpointPhaseV1::Prestate,
                seat,
                hard_state: loader_model_hard_state(),
                forbidden_drift_count: 0,
                slot: gate.freeze_slot,
            },
        );
    }
    differential_model_apply(
        &mut model,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary.proposal_id,
            phase: StateCheckpointPhaseV1::Prestate,
            attesting_seats: [0, 1, 2],
            slot: gate.freeze_slot,
        },
    );
    model
}

fn assert_optional_model_proposal(
    actual: OptionalPubkeyV1,
    expected: Option<u64>,
    proposal_keys: &[(u64, Pubkey)],
) {
    match expected {
        Some(id) => {
            assert!(actual.present);
            assert_eq!(
                actual.value,
                proposal_keys
                    .iter()
                    .find_map(|(candidate, key)| (*candidate == id).then_some(*key))
                    .expect("model proposal key")
            );
        }
        None => assert!(!actual.present),
    }
}

fn assert_council_hash_projection(
    actual: [u8; 32],
    expected: [u8; 32],
    council: &GovernanceCouncilSetV1,
    model: &Release1Model,
) {
    match (actual == [0; 32], expected == [0; 32]) {
        (true, true) => {}
        (false, false) => {
            // The processor and pure model deliberately use distinct council
            // hash domains. Compare each hash to its canonical council while
            // the projection below compares every underlying consensus field.
            assert_eq!(actual, council.set_hash);
            assert_eq!(expected, model.council.hash);
        }
        _ => panic!("concrete/model council-hash presence must match"),
    }
}

async fn assert_loader_model_projection(
    context: &mut ProgramTestContext,
    harness: &BufferHarness,
    model: &Release1Model,
    proposal_keys: &[(u64, Pubkey)],
) {
    let config: ControllerConfigV1 = state(context, harness.config).await;
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let council: GovernanceCouncilSetV1 = state(context, harness.council).await;
    assert_eq!(config.next_proposal_id, model.next_proposal_id);
    assert_eq!(config.target_nonce, model.target_nonce);
    assert_eq!(config.current_council_version, model.council.version);
    assert_eq!(
        config.token_governance_enabled,
        model.token_governance_enabled
    );
    assert_eq!(config.guardian, model.graph.guardian);
    assert_eq!(council.version, model.council.version);
    assert_eq!(council.set_hash, compute_council_set_hash(&council));
    assert_eq!(council.activation_slot, model.council.activation_slot);
    assert_eq!(
        council.seats.map(|seat| seat.seat_authority),
        model.council.seats
    );
    assert_eq!(
        council.seats.map(|seat| ModelSeatTerm {
            start_slot: seat.term_start_slot,
            end_slot: seat.term_end_slot,
        }),
        model.council.seat_terms
    );
    assert!(council.seats.iter().all(|seat| seat.active));
    assert_eq!(gate.status, model.gate.status);
    assert_eq!(gate.epoch, model.gate.epoch);
    assert_eq!(gate.freeze_slot, model.gate.freeze_slot);
    assert_eq!(gate.freeze_reason_code, model.gate.freeze_reason);
    let key_for = |proposal_id: u64| {
        proposal_keys
            .iter()
            .find_map(|(id, key)| (*id == proposal_id).then_some(*key))
            .expect("mapped proposal key")
    };
    assert_eq!(
        gate.active_proposal,
        model.gate.active_proposal.map(key_for).unwrap_or_default()
    );
    assert_eq!(
        gate.last_completed_proposal,
        model
            .gate
            .last_completed_proposal
            .map(key_for)
            .unwrap_or_default()
    );

    let raw_programdata = bytes(context, harness.target_programdata).await;
    let programdata_header =
        parse_upgradeable_programdata(&raw_programdata).expect("canonical ProgramData projection");
    assert_eq!(programdata_header.deployed_slot, model.programdata.slot);
    assert_eq!(
        programdata_header.capacity as u64,
        model.programdata.capacity
    );
    assert_eq!(
        programdata_header.upgrade_authority,
        Some(model.programdata.authority)
    );
    assert_eq!(
        loader_account_data_hash(&raw_programdata),
        model.programdata.raw_hash
    );

    for (proposal_id, proposal_key) in proposal_keys {
        let expected = &model.proposals[proposal_id];
        let actual: UpgradeProposalV2 = state(context, *proposal_key).await;
        assert_eq!(actual.proposal_id, expected.id);
        assert_eq!(actual.proposal_class, expected.class);
        assert_eq!(actual.state, expected.state);
        assert_eq!(actual.target_nonce, expected.target_nonce);
        assert_eq!(actual.creation_gate_status, expected.creation_gate_status);
        assert_eq!(actual.creation_gate_epoch, expected.creation_gate_epoch);
        assert_eq!(actual.freeze_gate_epoch, expected.freeze_gate_epoch);
        assert_eq!(
            actual.creation_council_version,
            expected.creation_council_version
        );
        assert_council_hash_projection(
            actual.creation_council_hash,
            expected.creation_council_hash,
            &council,
            model,
        );
        assert_eq!(actual.creation_slot, expected.timing.creation_slot);
        assert_eq!(actual.review_start_slot, expected.timing.review_start_slot);
        assert_eq!(actual.review_end_slot, expected.timing.review_end_slot);
        assert_eq!(actual.not_before_slot, expected.timing.not_before_slot);
        assert_eq!(actual.expiry_slot, expected.timing.expiry_slot);
        assert_eq!(
            actual.council_approval_bitset,
            expected.initial_approvals.bitset
        );
        assert_eq!(
            actual.council_approval_count,
            expected.initial_approvals.count
        );
        assert_eq!(
            actual.cancellation_council_version,
            expected.cancellation_approvals.council_version
        );
        assert_council_hash_projection(
            actual.cancellation_council_hash,
            expected.cancellation_approvals.council_hash,
            &council,
            model,
        );
        assert_eq!(
            actual.cancellation_approval_bitset,
            expected.cancellation_approvals.bitset
        );
        assert_eq!(
            actual.cancellation_approval_count,
            expected.cancellation_approvals.count
        );
        assert_eq!(
            actual.unfreeze_council_version,
            expected.unfreeze_approvals.council_version
        );
        assert_council_hash_projection(
            actual.unfreeze_council_hash,
            expected.unfreeze_approvals.council_hash,
            &council,
            model,
        );
        assert_eq!(
            actual.unfreeze_approval_bitset,
            expected.unfreeze_approvals.bitset
        );
        assert_eq!(
            actual.unfreeze_approval_count,
            expected.unfreeze_approvals.count
        );
        assert_eq!(actual.frozen_slot, expected.frozen_slot);
        assert_eq!(actual.extension_executed_slot, expected.extended_slot);
        assert_eq!(actual.upgrade_executed_slot, expected.upgraded_slot);
        assert_eq!(
            actual.programdata_verified_slot,
            expected.programdata_verified_slot
        );
        assert_eq!(actual.terminal_slot, expected.terminal_slot);
        assert_eq!(
            actual.cancellation_reason_code,
            expected.cancellation_reason
        );
        assert_eq!(actual.terminal_reason_code, expected.terminal_reason);
        assert_optional_model_proposal(
            actual.primary_proposal,
            expected.primary_proposal,
            proposal_keys,
        );
        assert_optional_model_proposal(
            actual.rollback_proposal,
            expected.rollback_proposal,
            proposal_keys,
        );

        for (checkpoint_key, expected_checkpoint) in [
            (actual.prestate_checkpoint, expected.prestate.as_ref()),
            (
                actual.required_poststate_checkpoint,
                expected.poststate.as_ref(),
            ),
        ] {
            let Some(expected_checkpoint) = expected_checkpoint else {
                continue;
            };
            let checkpoint: StateCheckpointV1 = state(context, checkpoint_key).await;
            assert_eq!(checkpoint.proposal, *proposal_key);
            assert_eq!(checkpoint.phase, expected_checkpoint.phase);
            assert_eq!(checkpoint.gate_epoch, expected_checkpoint.gate_epoch);
            assert_eq!(
                checkpoint.program_owned_state_root,
                expected_checkpoint.hard_state.program_owned_root
            );
            assert_eq!(
                checkpoint.program_owned_state_count,
                expected_checkpoint.hard_state.program_owned_count
            );
            assert_eq!(
                checkpoint.logical_compressed_state_root,
                expected_checkpoint.hard_state.logical_compressed_root
            );
            assert_eq!(
                checkpoint.logical_compressed_state_count,
                expected_checkpoint.hard_state.logical_compressed_count
            );
            assert_eq!(
                checkpoint.semantic_custody_accounting_root,
                expected_checkpoint.hard_state.semantic_custody_root
            );
            assert_eq!(
                checkpoint.external_metadata_observation_root,
                expected_checkpoint.hard_state.custody_identity_root
            );
            assert_eq!(
                checkpoint.forbidden_drift_count,
                expected_checkpoint.forbidden_drift_count
            );
            assert_eq!(
                checkpoint.approval_council_version,
                expected_checkpoint.approvals.council_version
            );
            assert_council_hash_projection(
                checkpoint.approval_council_hash,
                expected_checkpoint.approvals.council_hash,
                &council,
                model,
            );
            assert_eq!(
                checkpoint.approval_bitset,
                expected_checkpoint.approvals.bitset
            );
            assert_eq!(
                checkpoint.approval_count,
                expected_checkpoint.approvals.count
            );
            assert_eq!(checkpoint.accepted, expected_checkpoint.accepted);
            assert_eq!(checkpoint.finalized_slot, expected_checkpoint.accepted_slot);
        }
    }
}

#[tokio::test]
#[ignore = "requires AMOEBA_LOADER_TEST_ARTIFACT pointing to an attested local SBF ELF"]
async fn release1_model_differential_through_real_loader_and_separate_unfreeze() {
    let artifact = read_attested_loader_artifact();
    let (mut context, harness) = start_buffer_harness(
        artifact.clone(),
        artifact.clone(),
        0,
        ROLLBACK_ARTIFACT.to_vec(),
        false,
        false,
    )
    .await;
    let (sealed_primary, sealed_verification) = seal_buffer(&mut context, &harness).await;
    let (frozen_config, policy, frozen_gate, primary, rollback, prestate) =
        seed_frozen_loader_state(&mut context, &harness, sealed_primary).await;
    let council: GovernanceCouncilSetV1 = state(&mut context, harness.council).await;
    let rollback_key = primary.rollback_proposal.value;
    let proposal_keys = [
        (primary.proposal_id, harness.proposal),
        (rollback.proposal_id, rollback_key),
    ];
    let mut model = prepare_loader_model_at_accepted_prestate(
        &harness,
        &frozen_config,
        &council,
        &primary,
        &rollback,
        &frozen_gate,
    );
    assert_loader_model_projection(&mut context, &harness, &model, &proposal_keys).await;

    let upgrade_slot = clock_slot(&mut context).await;
    let raw_prestate = bytes(&mut context, harness.target_programdata).await;
    let upgrade = execute_instruction(
        &context,
        &harness,
        &frozen_config,
        &frozen_gate,
        &primary,
        &rollback,
        &prestate,
        &sealed_verification,
        loader_account_data_hash(&raw_prestate),
        INITIAL_PROGRAMDATA_SLOT,
        harness.spill,
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, upgrade], &[])
        .await
        .expect("differential real Loader-v3 upgrade");
    differential_model_apply(
        &mut model,
        Release1ModelAction::ExecuteUpgrade {
            proposal_id: primary.proposal_id,
            slot: upgrade_slot,
        },
    );
    let deployed_programdata = bytes(&mut context, harness.target_programdata).await;
    let deployed_header = parse_upgradeable_programdata(&deployed_programdata)
        .expect("differential deployed ProgramData");
    differential_model_apply(
        &mut model,
        Release1ModelAction::SetProgramDataObservation(ModelProgramDataObservation {
            slot: deployed_header.deployed_slot,
            payload_hash: hashv(&[&artifact]).to_bytes(),
            raw_hash: loader_account_data_hash(&deployed_programdata),
            capacity: deployed_header.capacity as u64,
            authority: harness.authority,
        }),
    );
    assert_loader_model_projection(&mut context, &harness, &model, &proposal_keys).await;

    let (programdata_verified, verification) = verify_deployed_programdata(
        &mut context,
        &harness,
        &frozen_config,
        &frozen_gate,
        &artifact,
        deployed_header.deployed_slot,
    )
    .await;
    differential_model_apply(
        &mut model,
        Release1ModelAction::VerifyProgramData {
            proposal_id: primary.proposal_id,
            slot: verification.finalized_slot,
        },
    );
    assert_loader_model_projection(&mut context, &harness, &model, &proposal_keys).await;

    set_clock_slot(&mut context, verification.finalized_slot + 1).await;
    let checkpoint_slot = clock_slot(&mut context).await;
    let poststate_candidate = checkpoint_candidate(
        &harness,
        harness.proposal,
        &programdata_verified,
        &council,
        &frozen_gate,
        StateCheckpointPhaseV1::Poststate,
        checkpoint_slot,
        verification.deployed_slot,
        &artifact,
        &deployed_programdata,
        Some(&prestate),
    );
    let poststate = attest_and_finalize_checkpoint(
        &mut context,
        &harness,
        derive_policy_pda(&harness.controller, &harness.target, 1).0,
        harness.proposal,
        &programdata_verified,
        &council,
        poststate_candidate,
        programdata_verified.programdata_verification,
        Some(programdata_verified.prestate_checkpoint),
    )
    .await;
    for seat in 0..3 {
        differential_model_apply(
            &mut model,
            Release1ModelAction::AttestCheckpoint {
                proposal_id: primary.proposal_id,
                phase: StateCheckpointPhaseV1::Poststate,
                seat,
                hard_state: loader_model_hard_state(),
                forbidden_drift_count: 0,
                slot: checkpoint_slot,
            },
        );
    }
    differential_model_apply(
        &mut model,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary.proposal_id,
            phase: StateCheckpointPhaseV1::Poststate,
            attesting_seats: [0, 1, 2],
            slot: checkpoint_slot,
        },
    );
    assert_loader_model_projection(&mut context, &harness, &model, &proposal_keys).await;

    let observed_poststate: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
    let early_unfreeze = execute_unfreeze_v1_instruction(
        harness.controller,
        ExecuteUnfreezeV1Accounts {
            controller_config: harness.config,
            policy: derive_policy_pda(&harness.controller, &harness.target, 1).0,
            current_council: harness.council,
            protocol_gate: harness.gate,
            proposal: harness.proposal,
            linked_proposal: rollback_key,
            poststate_checkpoint: observed_poststate.required_poststate_checkpoint,
            programdata_verification: observed_poststate.programdata_verification,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            instructions_sysvar: sysvar_ids::instructions::ID,
        },
        ExecuteUnfreezeV1 {
            expected: unfreeze_expectation(
                &observed_poststate,
                &frozen_config,
                &policy,
                &council,
                &frozen_gate,
                &poststate,
                &verification,
            ),
            linked_proposal: rollback_key,
            envelope: loader_envelope(),
        },
    );
    assert_loader_matrix_failure(
        &mut context,
        early_unfreeze,
        &[],
        &[harness.gate, harness.proposal, rollback_key],
        true,
    )
    .await;
    differential_model_atomic_failure(
        &mut model,
        Release1ModelAction::ExecuteUnfreeze {
            proposal_id: primary.proposal_id,
            slot: checkpoint_slot,
        },
    );

    for index in 0..3 {
        let observed: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
        let approve = approve_unfreeze_v1_instruction(
            harness.controller,
            ApproveUnfreezeV1Accounts {
                controller_config: harness.config,
                policy: derive_policy_pda(&harness.controller, &harness.target, 1).0,
                current_council: harness.council,
                protocol_gate: harness.gate,
                proposal: harness.proposal,
                poststate_checkpoint: observed.required_poststate_checkpoint,
                programdata_verification: observed.programdata_verification,
                target_program: harness.target,
                target_programdata: harness.target_programdata,
                authority_pda: harness.authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
                seat_authority: harness.seats[index].pubkey(),
            },
            ApproveUnfreezeV1 {
                expected: unfreeze_expectation(
                    &observed,
                    &frozen_config,
                    &policy,
                    &council,
                    &frozen_gate,
                    &poststate,
                    &verification,
                ),
            },
        );
        let [limit, price] = envelope_prefix();
        submit(
            &mut context,
            &[limit, price, approve],
            &[&harness.seats[index]],
        )
        .await
        .expect("differential unfreeze approval");
        differential_model_apply(
            &mut model,
            Release1ModelAction::ApproveUnfreeze {
                proposal_id: primary.proposal_id,
                seat: index as u8,
                slot: checkpoint_slot,
            },
        );
        assert_loader_model_projection(&mut context, &harness, &model, &proposal_keys).await;
    }
    let unfreeze_approved: UpgradeProposalV2 = state(&mut context, harness.proposal).await;
    let unfreeze = execute_unfreeze_v1_instruction(
        harness.controller,
        ExecuteUnfreezeV1Accounts {
            controller_config: harness.config,
            policy: derive_policy_pda(&harness.controller, &harness.target, 1).0,
            current_council: harness.council,
            protocol_gate: harness.gate,
            proposal: harness.proposal,
            linked_proposal: rollback_key,
            poststate_checkpoint: unfreeze_approved.required_poststate_checkpoint,
            programdata_verification: unfreeze_approved.programdata_verification,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            instructions_sysvar: sysvar_ids::instructions::ID,
        },
        ExecuteUnfreezeV1 {
            expected: unfreeze_expectation(
                &unfreeze_approved,
                &frozen_config,
                &policy,
                &council,
                &frozen_gate,
                &poststate,
                &verification,
            ),
            linked_proposal: rollback_key,
            envelope: loader_envelope(),
        },
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, unfreeze], &[])
        .await
        .expect("differential governed unfreeze");
    differential_model_apply(
        &mut model,
        Release1ModelAction::ExecuteUnfreeze {
            proposal_id: primary.proposal_id,
            slot: checkpoint_slot,
        },
    );
    assert_loader_model_projection(&mut context, &harness, &model, &proposal_keys).await;
}
