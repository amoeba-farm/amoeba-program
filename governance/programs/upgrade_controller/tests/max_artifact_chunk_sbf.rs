use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{hash::hashv, instruction::Instruction, pubkey::Pubkey, rent::Rent};
use solana_program_test::{BanksClientError, ProgramTest, ProgramTestContext};
use solana_sdk::{
    account::Account,
    compute_budget::ComputeBudgetInstruction,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use solana_sdk_ids::system_program;
use upgrade_controller::{
    artifact_merkle::{
        artifact_chunk_count, artifact_merkle_proof, artifact_merkle_root,
        ARTIFACT_MERKLE_SCHEME_ID, MAX_ARTIFACT_BYTES_V1, MAX_ARTIFACT_PROOF_DEPTH_V1,
        MAX_SELECTED_ARTIFACT_CHUNKS_V1, RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
    },
    instruction::{
        adopt_buffer_v1_instruction, verify_buffer_chunk_v1_instruction, AdoptBufferV1,
        AdoptBufferV1Accounts, FixedMerkleProofV1, ProposalExpectationV2, VerifyBufferChunkV1,
        VerifyBufferChunkV1Accounts,
    },
    pda::{
        derive_authority_pda, derive_buffer_check_pda, derive_checkpoint_pda,
        derive_controller_config_pda, derive_gate_pda, derive_programdata_check_pda,
        derive_proposal_pda, derive_upgradeable_programdata_address, UPGRADEABLE_LOADER_ID,
    },
    release1_digest::compute_proposal_digest_v2,
    release1_loader_accounts::{
        LOADER_BUFFER_METADATA_LEN, LOADER_PROGRAMDATA_METADATA_LEN, LOADER_PROGRAM_ACCOUNT_LEN,
        LOADER_STATE_TAG_BUFFER, LOADER_STATE_TAG_PROGRAM, LOADER_STATE_TAG_PROGRAMDATA,
    },
    release1_state::{
        BufferVerificationStatusV1, BufferVerificationV1, ProposalStateV2, UpgradeProposalV2,
        ACCOUNT_VERSION_V2, UPGRADE_PROPOSAL_V2_DISCRIMINATOR, UPGRADE_PROPOSAL_V2_RESERVED_LEN,
    },
    state::{
        CheckpointPhaseV1, ControllerConfigV1, GateStatusV1, OptionalPubkeyV1, ProposalClassV1,
        ProtocolGateV1, VoteRequirementV1, ACCOUNT_VERSION_V1, CONTROLLER_CONFIG_DISCRIMINATOR,
        CONTROLLER_CONFIG_RESERVED_LEN, PROTOCOL_GATE_DISCRIMINATOR, PROTOCOL_GATE_RESERVED_LEN,
    },
};

const TEST_SLOT: u64 = 50;
const INITIAL_PROGRAMDATA_SLOT: u64 = 1;
const MAX_INTEGRATED_VERIFY_CHUNK_CU_V1: u64 = 200_000;
const CONTROLLER_IDENTITY_BYTE: u8 = 0xA2;
const TARGET_IDENTITY_BYTE: u8 = 0xB2;

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

fn read_controller_sbf() -> Vec<u8> {
    let output_dir = std::env::var_os("BPF_OUT_DIR")
        .expect("set BPF_OUT_DIR to the exact checked controller SBF output directory");
    let artifact_path = std::path::PathBuf::from(output_dir).join("upgrade_controller.so");
    let artifact = std::fs::read(&artifact_path).expect("read exact controller SBF ELF");
    assert!(
        artifact.starts_with(b"\x7fELF"),
        "controller must be an ELF"
    );
    artifact
}

fn controller_config(
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
    config_key: Pubkey,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    buffer: Pubkey,
    uploader: Pubkey,
    artifact: &[u8],
) -> (Pubkey, UpgradeProposalV2) {
    let proposal_id = 1;
    let (proposal_key, bump) =
        derive_proposal_pda(&controller, &config.target_program, proposal_id);
    let review_start_slot = TEST_SLOT + 1;
    let review_end_slot = review_start_slot + config.vote_review_slots;
    let rollback_artifact = b"release-1-max-artifact-evidence-rollback";
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
        creation_slot: TEST_SLOT,
        cluster_domain: config.cluster_domain,
        controller_program: controller,
        controller_config: config_key,
        protocol_gate: config.gate_pda,
        policy_version: config.current_policy_version,
        policy_hash: [9; 32],
        creation_council_version: config.current_council_version,
        creation_council_hash: [8; 32],
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
            .expect("maximum artifact root"),
        chunk_hash_domain: ARTIFACT_MERKLE_SCHEME_ID,
        chunk_size: RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        chunk_count: artifact_chunk_count(artifact.len() as u64, RELEASE1_ARTIFACT_CHUNK_SIZE_V1)
            .expect("maximum artifact chunk count"),
        source_commit_hash: [10; 32],
        source_tree_hash: [11; 32],
        build_input_inventory_hash: [12; 32],
        reproducible_build_receipt_hash: [13; 32],
        package_receipt_hash: [14; 32],
        release_intent_hash: [15; 32],
        expected_execution_pre_payload_hash: hashv(&[artifact]).to_bytes(),
        expected_execution_pre_chunk_root: artifact_merkle_root(
            artifact,
            RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        )
        .expect("current payload root"),
        current_raw_programdata_hash: [7; 32],
        deployed_slot: INITIAL_PROGRAMDATA_SLOT,
        current_capacity: artifact.len() as u64,
        extension_delta: 0,
        expected_post_capacity: artifact.len() as u64,
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
        rollback_proposal: OptionalPubkeyV1::some(Pubkey::new_from_array([71; 32]))
            .expect("rollback proposal"),
        rollback_buffer: OptionalPubkeyV1::some(Pubkey::new_from_array([72; 32]))
            .expect("rollback buffer"),
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
        expiry_slot: TEST_SLOT + config.proposal_expiry_slots,
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
    proposal.validate_schema().expect("valid maximum proposal");
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

fn fixed_proof(artifact: &[u8], chunk_index: u32) -> FixedMerkleProofV1 {
    let proof = artifact_merkle_proof(artifact, RELEASE1_ARTIFACT_CHUNK_SIZE_V1, chunk_index)
        .expect("bounded proof");
    let mut fixed = FixedMerkleProofV1::empty();
    fixed.proof_len = proof.len() as u8;
    fixed.nodes[..proof.len()].copy_from_slice(&proof);
    fixed
}

async fn state<T: BorshDeserialize>(context: &mut ProgramTestContext, key: Pubkey) -> T {
    let bytes = context
        .banks_client
        .get_account(key)
        .await
        .expect("banks read")
        .expect("state account")
        .data;
    T::try_from_slice(&bytes).expect("fixed state decode")
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

fn envelope_prefix() -> [Instruction; 2] {
    [
        ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
        ComputeBudgetInstruction::set_compute_unit_price(0),
    ]
}

fn deterministic_max_artifact() -> Vec<u8> {
    (0..MAX_ARTIFACT_BYTES_V1)
        .map(|index| ((index.wrapping_mul(31).wrapping_add(17)) & 0xff) as u8)
        .collect()
}

fn evidence_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut value, "{byte:02x}").expect("format evidence hex");
    }
    value
}

/// Runs the exact checked controller SBF ELF from `BPF_OUT_DIR` as a
/// Loader-v3 Program/ProgramData pair. It invokes the real Loader-v3
/// SetAuthorityChecked path, then verifies every chunk of the maximum Release 1
/// artifact. The final chunk has the maximum proof depth and is simulated
/// separately so Agave reports its integrated compute consumption.
///
/// A successful run proves runtime stack safety for this path, not a numeric
/// linked-ELF stack margin. The checked build's linked-ELF analyzer remains the
/// separate source of static stack diagnostics.
#[tokio::test]
#[ignore = "requires BPF_OUT_DIR containing the exact checked upgrade_controller.so"]
async fn actual_controller_sbf_max_artifact_buffer_chunk_compute() {
    let controller_sbf = read_controller_sbf();
    let artifact = deterministic_max_artifact();
    assert_eq!(artifact.len() as u64, MAX_ARTIFACT_BYTES_V1);
    let expected_chunk_count =
        artifact_chunk_count(artifact.len() as u64, RELEASE1_ARTIFACT_CHUNK_SIZE_V1)
            .expect("maximum artifact chunk count");
    assert_eq!(
        expected_chunk_count as usize,
        MAX_SELECTED_ARTIFACT_CHUNKS_V1
    );
    assert_eq!(expected_chunk_count, 96);

    let controller = Pubkey::new_from_array([CONTROLLER_IDENTITY_BYTE; 32]);
    let controller_programdata = derive_upgradeable_programdata_address(&controller).0;
    let target = Pubkey::new_from_array([TARGET_IDENTITY_BYTE; 32]);
    let target_programdata = derive_upgradeable_programdata_address(&target).0;
    let config_key = derive_controller_config_pda(&controller, &target).0;
    let gate_key = derive_gate_pda(&controller, &target).0;
    let authority = derive_authority_pda(&controller, &target).0;
    let spill = Pubkey::new_unique();
    let guardian = Pubkey::new_unique();
    let uploader = Keypair::new();
    let initializer = Pubkey::new_unique();
    let buffer = Pubkey::new_unique();
    let value_config = controller_config(controller, target, target_programdata, spill, guardian);
    value_config.validate_static().expect("valid test config");
    let value_gate = active_gate(controller, config_key, target, target_programdata);
    value_gate.validate_static().expect("valid active gate");
    let (proposal_key, value_proposal) = draft_proposal(
        controller,
        config_key,
        &value_config,
        &value_gate,
        buffer,
        uploader.pubkey(),
        &artifact,
    );
    let verification_key = derive_buffer_check_pda(&controller, &proposal_key).0;

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
            programdata_bytes(INITIAL_PROGRAMDATA_SLOT, initializer, &controller_sbf),
            false,
        ),
    );
    test.add_account(config_key, state_account(controller, &value_config));
    test.add_account(gate_key, state_account(controller, &value_gate));
    test.add_account(proposal_key, state_account(controller, &value_proposal));
    test.add_account(
        buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(uploader.pubkey(), &artifact),
            false,
        ),
    );
    for key in [uploader.pubkey(), authority, spill] {
        test.add_account(key, account(system_program::ID, Vec::new(), false));
    }

    let mut context = test.start_with_context().await;
    // Genesis-loaded upgradeable programs become executable after the first
    // bank boundary. Match the full actual-SBF lifecycle harness before the
    // first controller instruction instead of only rewriting the Clock sysvar.
    context
        .warp_to_slot(2)
        .expect("activate genesis controller SBF");
    let controller_account = context
        .banks_client
        .get_account(controller)
        .await
        .expect("controller account read")
        .expect("actual controller SBF account");
    assert!(controller_account.executable);
    assert_eq!(controller_account.owner, UPGRADEABLE_LOADER_ID);
    let controller_programdata_account = context
        .banks_client
        .get_account(controller_programdata)
        .await
        .expect("controller ProgramData read")
        .expect("controller ProgramData account");
    assert_eq!(controller_programdata_account.owner, UPGRADEABLE_LOADER_ID);

    let mut clock = context
        .banks_client
        .get_sysvar::<solana_sdk::clock::Clock>()
        .await
        .expect("clock");
    clock.slot = TEST_SLOT;
    context.set_sysvar(&clock);

    let draft: UpgradeProposalV2 = state(&mut context, proposal_key).await;
    let adopt = adopt_buffer_v1_instruction(
        controller,
        AdoptBufferV1Accounts {
            payer: context.payer.pubkey(),
            controller_config: config_key,
            protocol_gate: gate_key,
            proposal: proposal_key,
            buffer,
            uploader_authority: uploader.pubkey(),
            authority_pda: authority,
            buffer_verification: verification_key,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            system_program: system_program::ID,
        },
        AdoptBufferV1 {
            expected: expectation(&draft, &value_config, &value_gate),
        },
    );
    let [limit, price] = envelope_prefix();
    submit(&mut context, &[limit, price, adopt], &[&uploader])
        .await
        .expect("actual controller SBF SetAuthorityChecked CPI");

    let adopted: UpgradeProposalV2 = state(&mut context, proposal_key).await;
    assert_eq!(adopted.state, ProposalStateV2::BufferAdopted);
    assert_eq!(adopted.chunk_count, expected_chunk_count);
    let last_chunk_index = expected_chunk_count - 1;
    let mut verification: BufferVerificationV1 = state(&mut context, verification_key).await;

    for chunk_index in 0..last_chunk_index {
        let verify = verify_buffer_chunk_v1_instruction(
            controller,
            VerifyBufferChunkV1Accounts {
                controller_config: config_key,
                protocol_gate: gate_key,
                proposal: proposal_key,
                buffer,
                buffer_verification: verification_key,
                authority_pda: authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
            },
            VerifyBufferChunkV1 {
                expected: expectation(&adopted, &value_config, &value_gate),
                chunk_index,
                proof: fixed_proof(&artifact, chunk_index),
                expected_verification_status: verification.status,
                expected_verified_chunk_bitmap: verification.verified_chunk_bitmap,
                expected_verified_chunk_count: verification.verified_chunk_count,
            },
        );
        let [limit, price] = envelope_prefix();
        submit(&mut context, &[limit, price, verify], &[])
            .await
            .expect("actual controller SBF maximum-artifact chunk verification");
        verification = state(&mut context, verification_key).await;
    }

    let final_proof = fixed_proof(&artifact, last_chunk_index);
    assert_eq!(final_proof.proof_len as usize, MAX_ARTIFACT_PROOF_DEPTH_V1);
    let verify_final = verify_buffer_chunk_v1_instruction(
        controller,
        VerifyBufferChunkV1Accounts {
            controller_config: config_key,
            protocol_gate: gate_key,
            proposal: proposal_key,
            buffer,
            buffer_verification: verification_key,
            authority_pda: authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
        },
        VerifyBufferChunkV1 {
            expected: expectation(&adopted, &value_config, &value_gate),
            chunk_index: last_chunk_index,
            proof: final_proof,
            expected_verification_status: verification.status,
            expected_verified_chunk_bitmap: verification.verified_chunk_bitmap,
            expected_verified_chunk_count: verification.verified_chunk_count,
        },
    );
    let [limit, price] = envelope_prefix();
    context
        .get_new_latest_blockhash()
        .await
        .expect("fresh simulation blockhash");
    let transaction = Transaction::new_signed_with_payer(
        &[limit, price, verify_final],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    let simulation = context
        .banks_client
        .simulate_transaction(transaction.clone())
        .await
        .expect("maximum-artifact SBF simulation");
    simulation
        .result
        .expect("simulation result")
        .expect("actual controller SBF maximum-artifact final chunk succeeds");
    let details = simulation
        .simulation_details
        .expect("runtime compute details for SBF simulation");
    assert!(
        details.units_consumed <= MAX_INTEGRATED_VERIFY_CHUNK_CU_V1,
        "integrated chunk verification consumed {} CU, exceeding conservative {} CU ceiling",
        details.units_consumed,
        MAX_INTEGRATED_VERIFY_CHUNK_CU_V1
    );

    context
        .banks_client
        .process_transaction(transaction)
        .await
        .expect("commit identical final-chunk transaction");
    let completed: BufferVerificationV1 = state(&mut context, verification_key).await;
    assert_eq!(completed.verified_chunk_count, expected_chunk_count);
    assert_eq!(
        completed.status,
        BufferVerificationStatusV1::ReadyToFinalize
    );
    assert_eq!(
        completed.verified_chunk_bitmap[(last_chunk_index / 8) as usize]
            & (1 << (last_chunk_index % 8)),
        1 << (last_chunk_index % 8)
    );

    let artifact_hash = hashv(&[&artifact]).to_bytes();
    let controller_hash = hashv(&[&controller_sbf]).to_bytes();
    let sbpf_target = std::env::var("AMOEBA_SBPF_TARGET").unwrap_or_else(|_| "unspecified".into());
    println!(
        "AMOEBA_MAX_ARTIFACT_CHUNK_SBF_EVIDENCE={{\"sbpf_target\":\"{}\",\"controller_elf_length\":{},\"controller_elf_sha256\":\"{}\",\"artifact_length\":{},\"artifact_sha256\":\"{}\",\"chunk_size\":{},\"chunk_count\":{},\"chunk_index\":{},\"proof_depth\":{},\"units_consumed\":{},\"conservative_ceiling\":{},\"runtime_stack_fault\":false,\"loader_set_authority_checked\":true}}",
        sbpf_target,
        controller_sbf.len(),
        evidence_hex(&controller_hash),
        artifact.len(),
        evidence_hex(&artifact_hash),
        RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        expected_chunk_count,
        last_chunk_index,
        MAX_ARTIFACT_PROOF_DEPTH_V1,
        details.units_consumed,
        MAX_INTEGRATED_VERIFY_CHUNK_CU_V1,
    );
}
