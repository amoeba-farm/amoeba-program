#![cfg(feature = "programdata-observation-chunk-matrix")]

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{hash::hashv, pubkey::Pubkey, rent::Rent};
use solana_program_test::{ProgramTest, ProgramTestContext};
use solana_sdk::{
    account::Account, compute_budget::ComputeBudgetInstruction, signature::Signer,
    transaction::Transaction,
};
use upgrade_controller::{
    artifact_merkle::{artifact_chunk_count, artifact_merkle_root, ARTIFACT_MERKLE_SCHEME_ID},
    pda::{
        derive_authority_pda, derive_capacity_policy_pda, derive_controller_config_pda,
        derive_gate_pda, derive_programdata_observation_pda,
        derive_upgradeable_programdata_address, UPGRADEABLE_LOADER_ID,
    },
    programdata_observation_merkle::{
        append_programdata_observation_chunk, programdata_observation_chunk_count,
        ProgramDataObservationMerkleFrontierV1, MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
        PROGRAMDATA_OBSERVATION_CHUNK_SIZE_CANDIDATES_V1,
        PROGRAMDATA_OBSERVATION_FRONTIER_SLOTS_V1, PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
    },
    release1_ceremony_digest::{
        compute_capacity_policy_digest_v1, compute_programdata_observation_subject_digest_v1,
    },
    release1_ceremony_instruction::{
        append_programdata_observation_chunk_instruction, AppendProgramDataObservationChunkV1,
        ProgramDataObservationGuardV1,
    },
    release1_ceremony_state::{
        ProgramDataCapacityPolicyV1, ProgramDataObservationPurposeV1,
        ProgramDataObservationStatusV1, ProgramDataObservationV1, ARTIFACT_BINDING_CHUNK_SIZE_V1,
        CEREMONY_ACCOUNT_VERSION_V1, EXTEND_PROGRAM_CHECKED_FEATURE_ID_V1,
        MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1, PROGRAMDATA_CAPACITY_POLICY_V1_DISCRIMINATOR,
        PROGRAMDATA_CAPACITY_POLICY_V1_RESERVED_LEN, PROGRAMDATA_OBSERVATION_V1_DISCRIMINATOR,
        PROGRAMDATA_OBSERVATION_V1_RESERVED_LEN, PROGRAMDATA_PAYLOAD_OFFSET_V1,
        SET_AUTHORITY_CHECKED_FEATURE_ID_V1,
    },
    release1_loader_accounts::{
        LOADER_PROGRAMDATA_METADATA_LEN, LOADER_PROGRAM_ACCOUNT_LEN, LOADER_STATE_TAG_PROGRAM,
        LOADER_STATE_TAG_PROGRAMDATA,
    },
    release1_state::BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
    state::{
        ControllerConfigV1, GateStatusV1, OptionalPubkeyV1, ProtocolGateV1, ACCOUNT_VERSION_V1,
        CONTROLLER_CONFIG_DISCRIMINATOR, CONTROLLER_CONFIG_RESERVED_LEN,
        PROTOCOL_GATE_DISCRIMINATOR, PROTOCOL_GATE_RESERVED_LEN,
    },
};

const DEPLOYED_SLOT: u64 = 1;
const OBSERVATION_SLOT: u64 = 2;
const MAX_TRANSACTION_COMPUTE_UNITS: u64 = 1_400_000;
const RUNTIME_DEFAULT_INSTRUCTION_COMPUTE_UNITS: u64 = 200_000;
const CONSERVATIVE_CHUNK_COMPUTE_CEILING: u64 = 200_000;
const EXPECTED_COMPUTE_BUDGET_ENVELOPE_UNITS: u64 = 300;
const DETERMINISM_SAMPLES: usize = 2;

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

fn program_bytes(programdata: Pubkey) -> Vec<u8> {
    let mut data = vec![0; LOADER_PROGRAM_ACCOUNT_LEN];
    data[..4].copy_from_slice(&LOADER_STATE_TAG_PROGRAM.to_le_bytes());
    data[4..].copy_from_slice(programdata.as_ref());
    data
}

fn padded_controller_programdata(authority: Pubkey, controller_sbf: &[u8]) -> Vec<u8> {
    let mut data = vec![0; MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 as usize];
    data[..4].copy_from_slice(&LOADER_STATE_TAG_PROGRAMDATA.to_le_bytes());
    data[4..12].copy_from_slice(&DEPLOYED_SLOT.to_le_bytes());
    data[12] = 1;
    data[13..45].copy_from_slice(authority.as_ref());
    let payload_end = LOADER_PROGRAMDATA_METADATA_LEN + controller_sbf.len();
    assert!(
        payload_end <= data.len(),
        "controller must fit runtime ceiling"
    );
    data[LOADER_PROGRAMDATA_METADATA_LEN..payload_end].copy_from_slice(controller_sbf);
    data
}

fn read_controller_sbf() -> Vec<u8> {
    let output_dir = std::env::var_os("BPF_OUT_DIR")
        .expect("set BPF_OUT_DIR to the exact matrix controller SBF output directory");
    let artifact_path = std::path::PathBuf::from(output_dir).join("upgrade_controller.so");
    let artifact = std::fs::read(&artifact_path).expect("read exact controller SBF ELF");
    assert!(
        artifact.starts_with(b"\x7fELF"),
        "controller must be an ELF"
    );
    artifact
}

fn controller_config(controller: Pubkey, spill: Pubkey, guardian: Pubkey) -> ControllerConfigV1 {
    let target_programdata = derive_upgradeable_programdata_address(&controller).0;
    let (_, bump) = derive_controller_config_pda(&controller, &controller);
    ControllerConfigV1 {
        discriminator: CONTROLLER_CONFIG_DISCRIMINATOR,
        version: ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        cluster_domain: [0x31; 32],
        target_program: controller,
        target_programdata,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        authority_pda: derive_authority_pda(&controller, &controller).0,
        gate_pda: derive_gate_pda(&controller, &controller).0,
        canonical_spill_treasury: spill,
        current_council_version: 1,
        current_policy_version: 1,
        next_proposal_id: 1,
        target_nonce: 1,
        guardian,
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
    }
}

fn bootstrap_gate(controller: Pubkey, config: Pubkey) -> ProtocolGateV1 {
    let target_programdata = derive_upgradeable_programdata_address(&controller).0;
    ProtocolGateV1 {
        discriminator: PROTOCOL_GATE_DISCRIMINATOR,
        version: ACCOUNT_VERSION_V1,
        bump: derive_gate_pda(&controller, &controller).1,
        initialized: true,
        status: GateStatusV1::EmergencyFrozen,
        controller_config: config,
        target_program: controller,
        target_programdata,
        epoch: 1,
        active_proposal: Pubkey::default(),
        freeze_slot: 1,
        freeze_reason_code: BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
        last_completed_proposal: Pubkey::default(),
        reserved: [0; PROTOCOL_GATE_RESERVED_LEN],
    }
}

fn capacity_policy(
    controller: Pubkey,
    config: Pubkey,
    chunk_size: u32,
) -> ProgramDataCapacityPolicyV1 {
    let chunk_count =
        programdata_observation_chunk_count(MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1, chunk_size)
            .expect("candidate maximum geometry");
    let padded = chunk_count.next_power_of_two();
    let mut value = ProgramDataCapacityPolicyV1 {
        discriminator: PROGRAMDATA_CAPACITY_POLICY_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: derive_capacity_policy_pda(&controller, &controller).1,
        initialized: true,
        controller_program: controller,
        controller_config: config,
        target_program: controller,
        target_programdata: derive_upgradeable_programdata_address(&controller).0,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        loader_programdata_metadata_len: LOADER_PROGRAMDATA_METADATA_LEN as u64,
        maximum_raw_programdata_length: MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
        maximum_payload_capacity: MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1,
        maximum_artifact_length: upgrade_controller::artifact_merkle::MAX_ARTIFACT_BYTES_V1,
        observation_scheme_id: PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
        observation_chunk_size: chunk_size,
        observation_max_chunk_count: chunk_count,
        observation_padded_leaf_count: padded,
        observation_tree_depth: padded.ilog2() as u8,
        observation_frontier_hash_count: PROGRAMDATA_OBSERVATION_FRONTIER_SLOTS_V1 as u8,
        artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
        artifact_chunk_size: ARTIFACT_BINDING_CHUNK_SIZE_V1,
        zero_tail_required: true,
        extend_program_checked_feature: EXTEND_PROGRAM_CHECKED_FEATURE_ID_V1,
        set_authority_checked_feature: SET_AUTHORITY_CHECKED_FEATURE_ID_V1,
        policy_digest: [0; 32],
        creation_slot: 1,
        reserved: [0; PROGRAMDATA_CAPACITY_POLICY_V1_RESERVED_LEN],
    };
    value.policy_digest = compute_capacity_policy_digest_v1(&value).expect("capacity digest");
    value.validate_static().expect("matrix capacity policy");
    value
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut value, "{byte:02x}").expect("hex encoding");
    }
    value
}

struct ObservationPreparation<'a> {
    controller: Pubkey,
    config_key: Pubkey,
    gate_key: Pubkey,
    capacity_key: Pubkey,
    capacity: &'a ProgramDataCapacityPolicyV1,
    subject: Pubkey,
    initializer: Pubkey,
    controller_sbf: &'a [u8],
    raw: &'a [u8],
}

fn prepared_observation(
    preparation: ObservationPreparation<'_>,
) -> (
    Pubkey,
    ProgramDataObservationGuardV1,
    ProgramDataObservationV1,
    u32,
) {
    let ObservationPreparation {
        controller,
        config_key,
        gate_key,
        capacity_key,
        capacity,
        subject,
        initializer,
        controller_sbf,
        raw,
    } = preparation;
    let artifact_sha256 = hashv(&[controller_sbf]).to_bytes();
    let artifact_merkle_root = artifact_merkle_root(controller_sbf, ARTIFACT_BINDING_CHUNK_SIZE_V1)
        .expect("controller artifact root");
    let purpose = ProgramDataObservationPurposeV1::ControllerImmutability;
    let generation = 1;
    let subject_digest = compute_programdata_observation_subject_digest_v1(
        &controller,
        &config_key,
        &controller,
        &derive_upgradeable_programdata_address(&controller).0,
        purpose,
        &subject,
        generation,
        &gate_key,
        GateStatusV1::EmergencyFrozen,
        1,
        &Pubkey::default(),
        1,
        BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
        &capacity.policy_digest,
        controller_sbf.len() as u64,
        &artifact_sha256,
        &artifact_merkle_root,
        &ARTIFACT_MERKLE_SCHEME_ID,
        controller_sbf.len() as u64,
    )
    .expect("observation subject digest");
    let (observation_key, bump) = derive_programdata_observation_pda(
        &controller,
        &controller,
        purpose as u8,
        &subject_digest,
        generation,
    );
    let raw_chunk_count =
        programdata_observation_chunk_count(raw.len() as u64, capacity.observation_chunk_size)
            .expect("raw chunk count");
    let merge_width = raw_chunk_count.next_power_of_two() / 2;
    let benchmark_chunk_index = merge_width - 1;
    let mut frontier = ProgramDataObservationMerkleFrontierV1::default();
    for (index, chunk) in raw
        .chunks(capacity.observation_chunk_size as usize)
        .take(benchmark_chunk_index as usize)
        .enumerate()
    {
        append_programdata_observation_chunk(
            &mut frontier,
            &subject_digest,
            raw.len() as u64,
            capacity.observation_chunk_size,
            index as u32,
            chunk,
        )
        .expect("canonical prefix frontier");
    }
    assert_eq!(frontier.next_index, benchmark_chunk_index);
    let processed_raw_bytes = u64::from(benchmark_chunk_index)
        .checked_mul(u64::from(capacity.observation_chunk_size))
        .expect("processed raw bytes");
    let tail_start = u64::from(PROGRAMDATA_PAYLOAD_OFFSET_V1)
        .checked_add(controller_sbf.len() as u64)
        .expect("tail start");
    let tail_bytes_verified = processed_raw_bytes.saturating_sub(tail_start);
    let mut program_header_snapshot = [0; LOADER_PROGRAM_ACCOUNT_LEN];
    program_header_snapshot.copy_from_slice(&program_bytes(
        derive_upgradeable_programdata_address(&controller).0,
    ));
    let mut programdata_header_snapshot = [0; LOADER_PROGRAMDATA_METADATA_LEN];
    programdata_header_snapshot.copy_from_slice(&raw[..LOADER_PROGRAMDATA_METADATA_LEN]);
    let raw_padded_leaf_count = raw_chunk_count.next_power_of_two();
    let observation = ProgramDataObservationV1 {
        discriminator: PROGRAMDATA_OBSERVATION_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        controller_program: controller,
        controller_config: config_key,
        capacity_policy: capacity_key,
        capacity_policy_digest: capacity.policy_digest,
        purpose,
        subject,
        subject_digest,
        generation,
        protocol_gate: gate_key,
        gate_status: GateStatusV1::EmergencyFrozen,
        gate_epoch: 1,
        gate_active_proposal: Pubkey::default(),
        gate_freeze_slot: 1,
        gate_freeze_reason_code: BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
        target_program: controller,
        target_programdata: derive_upgradeable_programdata_address(&controller).0,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        program_owner: UPGRADEABLE_LOADER_ID,
        program_executable: true,
        program_data_length: LOADER_PROGRAM_ACCOUNT_LEN as u64,
        program_header_present: true,
        program_header_snapshot,
        linked_programdata: derive_upgradeable_programdata_address(&controller).0,
        programdata_owner: UPGRADEABLE_LOADER_ID,
        programdata_executable: false,
        programdata_header_present: true,
        programdata_header_snapshot,
        deployed_slot: DEPLOYED_SLOT,
        upgrade_authority: OptionalPubkeyV1::some(initializer).expect("initializer authority"),
        raw_data_length: raw.len() as u64,
        payload_offset: PROGRAMDATA_PAYLOAD_OFFSET_V1,
        actual_capacity: MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1,
        expected_artifact_length: controller_sbf.len() as u64,
        expected_artifact_sha256: artifact_sha256,
        expected_artifact_merkle_root: artifact_merkle_root,
        expected_artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
        artifact_chunk_size: ARTIFACT_BINDING_CHUNK_SIZE_V1,
        artifact_chunk_count: artifact_chunk_count(
            controller_sbf.len() as u64,
            ARTIFACT_BINDING_CHUNK_SIZE_V1,
        )
        .expect("artifact chunk count"),
        minimum_required_capacity: controller_sbf.len() as u64,
        raw_observation_scheme_id: PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
        raw_chunk_size: capacity.observation_chunk_size,
        raw_chunk_count,
        raw_padded_leaf_count,
        raw_tree_depth: raw_padded_leaf_count.ilog2() as u8,
        next_raw_chunk_index: frontier.next_index,
        raw_frontier: frontier.hashes,
        raw_frontier_mask: frontier.occupied_mask,
        next_artifact_chunk_index: 0,
        tail_bytes_verified,
        start_slot: OBSERVATION_SLOT,
        last_observed_slot: OBSERVATION_SLOT,
        finalized_slot: 0,
        final_raw_merkle_root: [0; 32],
        observation_digest: [0; 32],
        status: ProgramDataObservationStatusV1::Accumulating,
        reserved: [0; PROGRAMDATA_OBSERVATION_V1_RESERVED_LEN],
    };
    observation.validate_static().expect("prepared observation");
    let guard = ProgramDataObservationGuardV1 {
        purpose,
        generation,
        expected_subject_digest: subject_digest,
        expected_gate_status: GateStatusV1::EmergencyFrozen,
        expected_gate_epoch: 1,
        expected_freeze_reason_code: BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
        expected_freeze_slot: 1,
    };
    (observation_key, guard, observation, benchmark_chunk_index)
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

fn controller_units_from_logs(logs: &[String], controller: Pubkey) -> u64 {
    let prefix = format!("Program {controller} consumed ");
    logs.iter()
        .find_map(|line| {
            line.strip_prefix(&prefix)
                .and_then(|rest| rest.split_once(" of "))
                .and_then(|(units, _)| units.parse::<u64>().ok())
        })
        .expect("controller compute log")
}

async fn measure_candidate(controller_sbf: &[u8], chunk_size: u32, sbpf_target: &str) {
    let controller = Pubkey::new_from_array([0xa7; 32]);
    let controller_programdata = derive_upgradeable_programdata_address(&controller).0;
    let initializer = Pubkey::new_from_array([0xb7; 32]);
    let spill = Pubkey::new_from_array([0xc7; 32]);
    let guardian = Pubkey::new_from_array([0xd7; 32]);
    let subject = Pubkey::new_from_array([0xe7; 32]);
    let config_key = derive_controller_config_pda(&controller, &controller).0;
    let gate_key = derive_gate_pda(&controller, &controller).0;
    let capacity_key = derive_capacity_policy_pda(&controller, &controller).0;
    let config = controller_config(controller, spill, guardian);
    config.validate_static().expect("valid matrix config");
    let gate = bootstrap_gate(controller, config_key);
    gate.validate_static().expect("valid bootstrap gate");
    let capacity = capacity_policy(controller, config_key, chunk_size);
    let raw = padded_controller_programdata(initializer, controller_sbf);
    let (observation_key, guard, observation, benchmark_chunk_index) =
        prepared_observation(ObservationPreparation {
            controller,
            config_key,
            gate_key,
            capacity_key,
            capacity: &capacity,
            subject,
            initializer,
            controller_sbf,
            raw: &raw,
        });

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
        account(UPGRADEABLE_LOADER_ID, raw, false),
    );
    test.add_account(config_key, state_account(controller, &config));
    test.add_account(gate_key, state_account(controller, &gate));
    test.add_account(capacity_key, state_account(controller, &capacity));
    test.add_account(subject, account(controller, vec![1], false));
    test.add_account(observation_key, state_account(controller, &observation));

    let mut context = test.start_with_context().await;
    context
        .warp_to_slot(OBSERVATION_SLOT)
        .expect("activate genesis controller SBF");
    let append = append_programdata_observation_chunk_instruction(
        controller,
        config_key,
        gate_key,
        capacity_key,
        subject,
        controller,
        controller_programdata,
        observation_key,
        UPGRADEABLE_LOADER_ID,
        AppendProgramDataObservationChunkV1 {
            guard,
            expected_status: ProgramDataObservationStatusV1::Accumulating,
            chunk_index: benchmark_chunk_index,
        },
    )
    .expect("append matrix chunk");
    context
        .get_new_latest_blockhash()
        .await
        .expect("fresh blockhash");
    let transaction = Transaction::new_signed_with_payer(
        &[
            ComputeBudgetInstruction::set_compute_unit_limit(MAX_TRANSACTION_COMPUTE_UNITS as u32),
            ComputeBudgetInstruction::set_compute_unit_price(0),
            append,
        ],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    let mut full_transaction_samples = [0u64; DETERMINISM_SAMPLES];
    let mut controller_samples = [0u64; DETERMINISM_SAMPLES];
    for sample_index in 0..DETERMINISM_SAMPLES {
        let simulation = context
            .banks_client
            .simulate_transaction(transaction.clone())
            .await
            .expect("simulate actual controller SBF append");
        simulation
            .result
            .expect("simulation result")
            .expect("successful actual controller SBF append");
        let details = simulation
            .simulation_details
            .expect("simulation compute details");
        full_transaction_samples[sample_index] = details.units_consumed;
        controller_samples[sample_index] = controller_units_from_logs(&details.logs, controller);
    }
    assert!(
        full_transaction_samples
            .iter()
            .all(|units| *units == full_transaction_samples[0]),
        "full-transaction compute measurement must be deterministic"
    );
    assert!(
        controller_samples
            .iter()
            .all(|units| *units == controller_samples[0]),
        "controller compute measurement must be deterministic"
    );
    let units_consumed = full_transaction_samples[0];
    let controller_units_consumed = controller_samples[0];
    let envelope_units = units_consumed
        .checked_sub(controller_units_consumed)
        .expect("full transaction includes controller execution");
    assert_eq!(
        envelope_units, EXPECTED_COMPUTE_BUDGET_ENVELOPE_UNITS,
        "two canonical ComputeBudget instructions have pinned overhead"
    );
    assert!(
        units_consumed < MAX_TRANSACTION_COMPUTE_UNITS,
        "candidate must retain transaction compute margin"
    );
    context
        .banks_client
        .process_transaction(transaction)
        .await
        .expect("commit actual controller SBF append");
    let after: ProgramDataObservationV1 = state(&mut context, observation_key).await;
    assert_eq!(
        after.next_raw_chunk_index,
        benchmark_chunk_index + 1,
        "exact candidate chunk must commit"
    );
    let actual_chunk_length = u64::from(chunk_size).min(
        MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
            - u64::from(benchmark_chunk_index) * u64::from(chunk_size),
    );
    let merge_depth = benchmark_chunk_index.trailing_ones();
    println!(
        "AMOEBA_PROGRAMDATA_OBSERVATION_CHUNK_MATRIX_SBF_EVIDENCE={{\"schema\":\"ameba-programdata-observation-chunk-matrix-measurement-v1\",\"execution\":\"actual_controller_sbf_programtest\",\"sbpf_target\":\"{}\",\"test_only_feature\":true,\"controller_elf_length\":{},\"controller_elf_sha256\":\"{}\",\"raw_programdata_length\":{},\"chunk_size\":{},\"chunk_count\":{},\"benchmark_chunk_index\":{},\"actual_chunk_length\":{},\"merge_depth\":{},\"determinism_samples\":{},\"full_transaction_units_samples\":{:?},\"controller_units_samples\":{:?},\"units_consumed\":{},\"controller_units_consumed\":{},\"compute_budget_envelope_units\":{},\"compute_limit\":{},\"runtime_default_instruction_compute_limit\":{},\"default_budget_margin_units\":{},\"compute_margin\":{},\"within_conservative_ceiling\":{},\"selected_release1_size\":{},\"runtime_stack_fault\":false}}",
        sbpf_target,
        controller_sbf.len(),
        hex(&hashv(&[controller_sbf]).to_bytes()),
        MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
        chunk_size,
        capacity.observation_max_chunk_count,
        benchmark_chunk_index,
        actual_chunk_length,
        merge_depth,
        DETERMINISM_SAMPLES,
        full_transaction_samples,
        controller_samples,
        units_consumed,
        controller_units_consumed,
        envelope_units,
        MAX_TRANSACTION_COMPUTE_UNITS,
        RUNTIME_DEFAULT_INSTRUCTION_COMPUTE_UNITS,
        i128::from(RUNTIME_DEFAULT_INSTRUCTION_COMPUTE_UNITS) - i128::from(units_consumed),
        MAX_TRANSACTION_COMPUTE_UNITS - units_consumed,
        units_consumed <= CONSERVATIVE_CHUNK_COMPUTE_CEILING,
        chunk_size == PROGRAMDATA_OBSERVATION_CHUNK_SIZE_CANDIDATES_V1[0],
    );
    if chunk_size == PROGRAMDATA_OBSERVATION_CHUNK_SIZE_CANDIDATES_V1[0] {
        assert!(
            units_consumed <= CONSERVATIVE_CHUNK_COMPUTE_CEILING,
            "selected 16 KiB chunk exceeds conservative ceiling"
        );
    }
}

/// Measures the production append processor from the exact controller ELF for
/// every predeclared candidate. The feature only relaxes capacity-policy
/// admission for this test; the normal Release 1 schema still admits 16 KiB
/// exclusively.
#[tokio::test]
#[ignore = "requires matrix-feature controller SBF in BPF_OUT_DIR"]
async fn actual_controller_sbf_programdata_observation_chunk_matrix() {
    let controller_sbf = read_controller_sbf();
    let sbpf_target =
        std::env::var("AMOEBA_SBPF_TARGET").expect("set AMOEBA_SBPF_TARGET to v0 or v2");
    assert!(matches!(sbpf_target.as_str(), "v0" | "v2"));
    for chunk_size in PROGRAMDATA_OBSERVATION_CHUNK_SIZE_CANDIDATES_V1 {
        measure_candidate(&controller_sbf, chunk_size, &sbpf_target).await;
    }
}
