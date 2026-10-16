use base64::Engine as _;
use borsh::BorshDeserialize;
use serde_json::json;
use solana_address_lookup_table_interface::instruction::{
    create_lookup_table, extend_lookup_table,
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_loader_v3_interface::instruction::{
    set_upgrade_authority, upgrade as direct_loader_upgrade,
};
use solana_nonce::{state::State as NonceState, versions::Versions as NonceVersions};
use solana_program::{
    hash::{hashv, Hash},
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    rent::Rent,
};
use solana_program_test::{BanksClientError, ProgramTest, ProgramTestContext};
use solana_sdk::{
    account::{Account, AccountSharedData},
    commitment_config::CommitmentConfig,
    compute_budget::ComputeBudgetInstruction,
    message::{v0, AddressLookupTableAccount, VersionedMessage},
    signature::{Keypair, Signer},
    transaction::{Transaction, VersionedTransaction},
};
use solana_sdk_ids::{system_program, sysvar as sysvar_ids};
use solana_system_interface::instruction as system_instruction;
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    str::FromStr,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use upgrade_controller::{
    artifact_merkle::{
        artifact_chunk_count, artifact_chunk_leaf_hash, artifact_merkle_proof,
        artifact_merkle_root, ARTIFACT_MERKLE_SCHEME_ID, MAX_ARTIFACT_BYTES_V1,
        MAX_ARTIFACT_PROOF_DEPTH_V1,
    },
    council::compute_council_set_hash,
    error::GovernanceError,
    pda::{
        derive_authority_pda, derive_bootstrap_activation_receipt_pda, derive_buffer_check_pda,
        derive_capacity_policy_pda, derive_checkpoint_attestation_pda, derive_checkpoint_pda,
        derive_controller_config_pda, derive_controller_immutability_receipt_pda,
        derive_controller_release_commitment_pda, derive_council_pda,
        derive_current_deployment_state_pda, derive_gate_pda, derive_policy_pda,
        derive_programdata_check_pda, derive_programdata_failure_observation_pda,
        derive_programdata_observation_pda, derive_proposal_pda, derive_target_handoff_receipt_pda,
        derive_upgradeable_programdata_address, UPGRADEABLE_LOADER_ID,
    },
    policy::compute_policy_hash,
    programdata_observation_merkle::{
        programdata_observation_chunk_count, MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
        PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB, PROGRAMDATA_OBSERVATION_FRONTIER_SLOTS_V1,
        PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
    },
    release1_authority_instruction::{CeremonyEnvelopeV1, RecordControllerImmutabilityV1},
    release1_ceremony_digest::{
        compute_bootstrap_activation_deployment_plan_digest_v1,
        compute_bootstrap_activation_receipt_digest_v1,
        compute_bootstrap_activation_receipt_plan_digest_v1, compute_capacity_policy_digest_v1,
        compute_controller_immutability_receipt_digest_v1, compute_controller_release_digest_v1,
        compute_current_deployment_digest_v1, compute_programdata_observation_subject_digest_v1,
    },
    release1_ceremony_instruction::{
        append_programdata_observation_chunk_instruction,
        begin_programdata_observation_instruction, finalize_programdata_observation_instruction,
        verify_observed_artifact_chunk_instruction, AppendProgramDataObservationChunkV1,
        BeginProgramDataObservationV1, FinalizeProgramDataObservationV1, ObservationAuthorityV1,
        ObservedArtifactMerkleProofV1, ProgramDataObservationGuardV1,
        VerifyObservedArtifactChunkV1,
    },
    release1_ceremony_state::{
        BootstrapActivationReceiptV1, ControllerImmutabilityReceiptV1,
        ControllerReleaseCommitmentV1, CurrentDeploymentStateV1, ProgramDataCapacityPolicyV1,
        ProgramDataObservationPurposeV1, ProgramDataObservationStatusV1, ProgramDataObservationV1,
        TargetAuthorityHandoffReceiptV1, ARTIFACT_BINDING_CHUNK_SIZE_V1,
        BOOTSTRAP_ACTIVATION_RECEIPT_V1_DISCRIMINATOR,
        BOOTSTRAP_ACTIVATION_RECEIPT_V1_RESERVED_LEN, CEREMONY_ACCOUNT_VERSION_V1,
        CONTROLLER_IMMUTABILITY_RECEIPT_V1_DISCRIMINATOR,
        CONTROLLER_IMMUTABILITY_RECEIPT_V1_RESERVED_LEN,
        CONTROLLER_RELEASE_COMMITMENT_V1_DISCRIMINATOR,
        CONTROLLER_RELEASE_COMMITMENT_V1_RESERVED_LEN, CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR,
        CURRENT_DEPLOYMENT_STATE_V1_RESERVED_LEN, EXTEND_PROGRAM_CHECKED_FEATURE_ID_V1,
        MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1, PROGRAMDATA_CAPACITY_POLICY_V1_DISCRIMINATOR,
        PROGRAMDATA_CAPACITY_POLICY_V1_RESERVED_LEN, SET_AUTHORITY_CHECKED_FEATURE_ID_V1,
    },
    release1_digest::STATE_CHECKPOINT_HARD_ROOT_DOMAIN_V1,
    release1_governance_v2::{
        derive_governance_action_proposal_v2, derive_governance_lifecycle_registry_v2,
        derive_governance_timing_profile_v1, nominal_governance_timing_profile_v1,
        ApproveBootstrapActivationProposalV2, ApproveTargetAuthorityHandoffProposalV2,
        BootstrapActivationProposalV2, CreateBootstrapActivationProposalV2,
        CreateTargetAuthorityHandoffProposalV2, ExecuteBootstrapActivationProposalV2,
        ExecuteTargetAuthorityHandoffProposalV2, GovernanceActionGuardV2, GovernanceActionKindV2,
        GovernanceLifecycleStateV2, GovernanceTimingProfileV1,
        InitializeGovernanceLifecycleRegistryV2, QueueBootstrapActivationProposalV2,
        QueueTargetAuthorityHandoffProposalV2, TargetAuthorityHandoffProposalV2,
    },
    release1_loader_accounts::{
        parse_upgradeable_programdata, LOADER_BUFFER_METADATA_LEN, LOADER_PROGRAMDATA_METADATA_LEN,
        LOADER_PROGRAM_ACCOUNT_LEN, LOADER_STATE_TAG_BUFFER, LOADER_STATE_TAG_PROGRAM,
        LOADER_STATE_TAG_PROGRAMDATA,
    },
    release1_state::{
        BufferVerificationStatusV1, BufferVerificationV1, CheckpointAttestationV1, ProposalStateV2,
        StateCheckpointPhaseV1, BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
    },
    release1_v3_custody_instruction::{
        activate_rollback_v2_instruction, adopt_buffer_v2_instruction,
        execute_upgrade_v2_instruction, finalize_buffer_verification_v2_instruction,
        verify_buffer_chunk_v2_instruction, ActivateRollbackV2, ActivateRollbackV2Accounts,
        AdoptBufferV2, AdoptBufferV2Accounts, ArtifactChunkProofV2, ExecuteUpgradeV2,
        ExecuteUpgradeV2Accounts, FinalizeBufferVerificationV2,
        FinalizeBufferVerificationV2Accounts, VerifyBufferChunkV2, VerifyBufferChunkV2Accounts,
    },
    release1_v3_digest::compute_state_checkpoint_digest_v2,
    release1_v3_instruction::{
        approve_proposal_v3_instruction, approve_unfreeze_v2_instruction,
        bind_programdata_verification_v2_instruction, create_checkpoint_v2_instruction,
        create_proposal_v3_instruction, execute_unfreeze_v2_instruction,
        finalize_checkpoint_v2_instruction, finalize_governance_v3_instruction,
        finalize_programdata_verification_v2_instruction, freeze_proposal_v3_instruction,
        initialize_controller_v2_instruction, observe_programdata_failure_v2_instruction,
        queue_proposal_v3_instruction, ApproveProposalV3, ApproveProposalV3Accounts,
        ApproveUnfreezeV2, ApproveUnfreezeV2Accounts, BindProgramDataVerificationV2,
        BindProgramDataVerificationV2Accounts, CapacityPolicyInputV1, CheckpointAttestationGuardV2,
        CheckpointManifestV2, ControllerReleaseInputV1, CreateCheckpointV2,
        CreateCheckpointV2Accounts, CreateProposalV3, CreateProposalV3Accounts, ExecuteUnfreezeV2,
        ExecuteUnfreezeV2Accounts, FinalizeCheckpointV2, FinalizeCheckpointV2Accounts,
        FinalizeGovernanceV3, FinalizeGovernanceV3Accounts, FinalizeProgramDataVerificationV2,
        FinalizeProgramDataVerificationV2Accounts, FreezeProposalV3, FreezeProposalV3Accounts,
        InitializeControllerV2, InitializeControllerV2Accounts, ObserveProgramDataFailureV2,
        ObserveProgramDataFailureV2Accounts, ProgramDataFailureProofV2,
        ProgramDataFailureWitnessV2, ProgramDataVerificationGuardV2,
        ProgramDataVerificationManifestV2, ProposalGuardV3, ProposalManifestV3, QueueProposalV3,
        QueueProposalV3Accounts, SeatTermV2, UnfreezeGuardV2,
    },
    release1_v3_state::{
        ProgramDataFailureObservationV2, ProgramDataMismatchClassV2,
        ProgramDataVerificationStatusV2, ProgramDataVerificationV2, StateCheckpointV2,
        UpgradeProposalV3, CAPACITY_SAFE_ACCOUNT_VERSION_V2, STATE_CHECKPOINT_V2_DIGEST_DOMAIN_ID,
        STATE_CHECKPOINT_V2_DISCRIMINATOR, STATE_CHECKPOINT_V2_RESERVED_LEN,
    },
    state::{
        CheckpointPhaseV1, ControllerConfigV1, CouncilSeatV1, GateStatusV1, GovernanceCouncilSetV1,
        GovernanceModeV1, GovernancePolicyV1, OptionalPubkeyV1, ProposalClassV1, ProtocolGateV1,
        ACCOUNT_VERSION_V1, COUNCIL_SEAT_RESERVED_LEN, GOVERNANCE_COUNCIL_DISCRIMINATOR,
        GOVERNANCE_COUNCIL_RESERVED_LEN, GOVERNANCE_POLICY_DISCRIMINATOR,
        GOVERNANCE_POLICY_RESERVED_LEN,
    },
};

const GENESIS_PROGRAMDATA_SLOT: u64 = 0;
const INITIALIZATION_SLOT: u64 = 2;
const ROUTINE_DELAY: u64 = 10;
const MAJOR_DELAY: u64 = 20;
const ROLLBACK_DELAY: u64 = 5;
const TERMINAL_DELAY: u64 = 30;
// This is a local rehearsal profile, not a production timing recommendation.
// It leaves enough naturally advancing slot runway to seal both the primary
// and precommitted rollback artifacts before council approval begins.
const REVIEW_SLOTS: u64 = 1_024;
const MAX_REHEARSED_ARTIFACT_LENGTH: u64 = MAX_ARTIFACT_BYTES_V1;
const BUFFER_REVIEW_FIXED_RUNWAY_SLOTS: u64 = 64;
// The standalone validator submits hundreds of exact chunk transactions. Keep
// the synthetic rehearsal expiry well beyond that real-RPC runway while the
// immutable class timelocks remain at their minimum accepted values.
const EXPIRY_SLOTS: u64 = 20_000;
const OBSERVATION_COMPUTE_LIMIT: u32 = 1_400_000;
const MAX_SELECTED_OBSERVATION_CHUNK_CU_V1: u64 = 200_000;

// Keeping the concrete ProgramTest context in this test-only enum makes the
// backend switch explicit and avoids an extra allocation in every account read.
#[allow(clippy::large_enum_variant)]
enum CeremonyBackend {
    ProgramTest(ProgramTestContext),
    Rpc(RpcCeremonyBackend),
}

struct RpcCeremonyBackend {
    client: RpcClient,
    lookup_table: Option<AddressLookupTableAccount>,
}

struct CeremonyContext {
    backend: CeremonyBackend,
    payer: Keypair,
    cluster_domain: [u8; 32],
}

impl CeremonyContext {
    fn program_test(context: ProgramTestContext) -> Self {
        let payer =
            Keypair::try_from(context.payer.to_bytes().as_ref()).expect("copy ProgramTest payer");
        Self {
            backend: CeremonyBackend::ProgramTest(context),
            payer,
            cluster_domain: digest("local-ceremony-genesis"),
        }
    }

    fn rpc(rpc_url: String, payer: Keypair, cluster_domain: [u8; 32]) -> Self {
        Self {
            backend: CeremonyBackend::Rpc(RpcCeremonyBackend {
                client: RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed()),
                lookup_table: None,
            }),
            payer,
            cluster_domain,
        }
    }
}

struct StandaloneValidator {
    child: Child,
    evidence_dir: PathBuf,
    rpc_url: String,
}

impl Drop for StandaloneValidator {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_validator_account_fixture(
    directory: &Path,
    label: &str,
    pubkey: Pubkey,
    account: &Account,
) -> PathBuf {
    let path = directory.join(format!("{label}.json"));
    let document = json!({
        "pubkey": pubkey.to_string(),
        "account": {
            "lamports": account.lamports,
            "data": [
                base64::engine::general_purpose::STANDARD.encode(&account.data),
                "base64"
            ],
            "owner": account.owner.to_string(),
            "executable": account.executable,
            "rentEpoch": account.rent_epoch,
            "space": account.data.len(),
        }
    });
    fs::write(
        &path,
        serde_json::to_vec_pretty(&document).expect("serialize validator account fixture"),
    )
    .unwrap_or_else(|error| panic!("write validator fixture {}: {error}", path.display()));
    path
}

#[allow(clippy::too_many_arguments)]
async fn start_standalone_validator(
    harness: &CeremonyHarness,
    payer: Keypair,
    primary_uploader: &Keypair,
    primary_buffer: Pubkey,
    rollback_uploader: &Keypair,
    rollback_buffer: Pubkey,
    controller_artifact: &[u8],
    spread_artifact: &[u8],
    primary_artifact: &[u8],
    rollback_artifact: &[u8],
) -> (CeremonyContext, StandaloneValidator) {
    let root = PathBuf::from(
        std::env::var_os("AMOEBA_STANDALONE_EVIDENCE_DIR")
            .expect("set AMOEBA_STANDALONE_EVIDENCE_DIR to a narrow local artifact directory"),
    );
    assert!(
        root.is_absolute(),
        "standalone evidence directory must be absolute"
    );
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_millis();
    let evidence_dir = root.join(format!("run-{}-{timestamp}", std::process::id()));
    let accounts_dir = evidence_dir.join("accounts");
    let ledger_dir = evidence_dir.join("ledger");
    fs::create_dir_all(&accounts_dir).expect("create standalone account fixture directory");
    fs::create_dir_all(&ledger_dir).expect("create standalone ledger directory");

    let mut fixtures = Vec::new();
    let mut add_fixture = |label: &str, key: Pubkey, value: Account| {
        fixtures.push((
            key,
            write_validator_account_fixture(&accounts_dir, label, key, &value),
        ));
    };
    add_fixture(
        "controller-bootstrap-buffer",
        harness.controller_bootstrap_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(harness.initializer.pubkey(), controller_artifact),
            false,
        ),
    );
    add_fixture(
        "target-bootstrap-buffer",
        harness.target_bootstrap_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(harness.legacy_authority.pubkey(), spread_artifact),
            false,
        ),
    );
    add_fixture(
        "former-authority-buffer",
        harness.former_authority_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(harness.legacy_authority.pubkey(), spread_artifact),
            false,
        ),
    );
    add_fixture(
        "primary-buffer",
        primary_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(primary_uploader.pubkey(), primary_artifact),
            false,
        ),
    );
    add_fixture(
        "rollback-buffer",
        rollback_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(rollback_uploader.pubkey(), rollback_artifact),
            false,
        ),
    );
    for (label, key) in [
        ("initializer", harness.initializer.pubkey()),
        ("legacy-authority", harness.legacy_authority.pubkey()),
        ("guardian", harness.guardian.pubkey()),
        ("spill", harness.spill),
        ("primary-uploader", primary_uploader.pubkey()),
        ("rollback-uploader", rollback_uploader.pubkey()),
    ] {
        add_fixture(label, key, account(system_program::ID, Vec::new(), false));
    }
    for (index, seat) in harness.seats.iter().enumerate() {
        add_fixture(
            &format!("seat-{index}"),
            seat.pubkey(),
            account(system_program::ID, Vec::new(), false),
        );
    }

    let controller_path =
        PathBuf::from(std::env::var_os("BPF_OUT_DIR").expect("controller SBF output directory"))
            .join("upgrade_controller.so");
    let spread_path = PathBuf::from(
        std::env::var_os("AMOEBA_SPREAD_TEST_ARTIFACT").expect("Spread SBF artifact path"),
    );
    assert_eq!(
        fs::read(&controller_path).expect("read controller artifact"),
        controller_artifact
    );
    assert_eq!(
        fs::read(&spread_path).expect("read Spread artifact"),
        spread_artifact
    );

    let port_seed = (std::process::id() % 10_000) as u16;
    let rpc_port = 20_000u16 + port_seed;
    let faucet_port = rpc_port + 1;
    let gossip_port = rpc_port + 2;
    let dynamic_start = rpc_port + 10;
    let dynamic_end = dynamic_start + 99;
    let rpc_url = format!("http://127.0.0.1:{rpc_port}");
    let log_path = evidence_dir.join("validator.log");
    let stdout = File::create(&log_path).expect("create standalone validator log");
    let stderr = stdout.try_clone().expect("clone standalone validator log");
    let mut command = Command::new("solana-test-validator");
    command
        .arg("--reset")
        .arg("--quiet")
        .arg("--ledger")
        .arg(&ledger_dir)
        .arg("--bind-address")
        .arg("127.0.0.1")
        .arg("--rpc-port")
        .arg(rpc_port.to_string())
        .arg("--faucet-port")
        .arg(faucet_port.to_string())
        .arg("--gossip-port")
        .arg(gossip_port.to_string())
        .arg("--dynamic-port-range")
        .arg(format!("{dynamic_start}-{dynamic_end}"))
        .arg("--mint")
        .arg(payer.pubkey().to_string())
        .arg("--upgradeable-program")
        .arg(harness.controller.to_string())
        .arg(&controller_path)
        .arg(harness.initializer.pubkey().to_string())
        .arg("--upgradeable-program")
        .arg(harness.target.to_string())
        .arg(&spread_path)
        .arg(harness.legacy_authority.pubkey().to_string());
    for (key, path) in &fixtures {
        command.arg("--account").arg(key.to_string()).arg(path);
    }
    let child = command
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn pinned standalone local validator");
    let mut validator = StandaloneValidator {
        child,
        evidence_dir,
        rpc_url: rpc_url.clone(),
    };
    let probe = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());
    let mut healthy = false;
    for _ in 0..240 {
        if validator
            .child
            .try_wait()
            .expect("poll validator child")
            .is_some()
        {
            panic!(
                "standalone validator exited early; inspect {}",
                log_path.display()
            );
        }
        if probe.get_health().await.is_ok() {
            healthy = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(healthy, "standalone validator did not become healthy");
    let genesis_hash = probe
        .get_genesis_hash()
        .await
        .expect("read standalone genesis hash");
    let cluster_domain = genesis_hash.to_bytes();
    fs::write(
        validator.evidence_dir.join("genesis.txt"),
        format!("{genesis_hash}\n"),
    )
    .expect("write standalone genesis evidence");
    (
        CeremonyContext::rpc(rpc_url, payer, cluster_domain),
        validator,
    )
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

fn program_bytes(programdata: Pubkey) -> Vec<u8> {
    let mut data = vec![0; LOADER_PROGRAM_ACCOUNT_LEN];
    data[..4].copy_from_slice(&LOADER_STATE_TAG_PROGRAM.to_le_bytes());
    data[4..].copy_from_slice(programdata.as_ref());
    data
}

fn programdata_bytes(
    deployed_slot: u64,
    authority: Pubkey,
    payload: &[u8],
    capacity: usize,
) -> Vec<u8> {
    assert!(capacity >= payload.len());
    let mut data = vec![0; LOADER_PROGRAMDATA_METADATA_LEN + capacity];
    data[..4].copy_from_slice(&LOADER_STATE_TAG_PROGRAMDATA.to_le_bytes());
    data[4..12].copy_from_slice(&deployed_slot.to_le_bytes());
    data[12] = 1;
    data[13..45].copy_from_slice(authority.as_ref());
    data[LOADER_PROGRAMDATA_METADATA_LEN..LOADER_PROGRAMDATA_METADATA_LEN + payload.len()]
        .copy_from_slice(payload);
    data
}

fn buffer_bytes(authority: Pubkey, payload: &[u8]) -> Vec<u8> {
    let mut data = vec![0; LOADER_BUFFER_METADATA_LEN + payload.len()];
    data[..4].copy_from_slice(&LOADER_STATE_TAG_BUFFER.to_le_bytes());
    data[4] = 1;
    data[5..37].copy_from_slice(authority.as_ref());
    data[LOADER_BUFFER_METADATA_LEN..].copy_from_slice(payload);
    data
}

fn digest(label: &str) -> [u8; 32] {
    hashv(&[label.as_bytes()]).to_bytes()
}

fn read_artifact(path: std::path::PathBuf, label: &str) -> Vec<u8> {
    let bytes = std::fs::read(&path).unwrap_or_else(|error| {
        panic!("read exact {label} SBF ELF at {}: {error}", path.display())
    });
    assert!(
        bytes.starts_with(b"\x7fELF"),
        "{label} artifact must be an ELF"
    );
    bytes
}

fn read_controller_sbf() -> Vec<u8> {
    let directory = std::env::var_os("BPF_OUT_DIR")
        .expect("set BPF_OUT_DIR to the exact controller SBF output directory");
    read_artifact(
        std::path::PathBuf::from(directory).join("upgrade_controller.so"),
        "controller",
    )
}

fn read_sacrificial_target_sbf() -> (Vec<u8>, bool) {
    if let Some(path) = std::env::var_os("AMOEBA_SPREAD_TEST_ARTIFACT") {
        return (
            read_artifact(std::path::PathBuf::from(path), "Spread bridge"),
            true,
        );
    }
    let path = std::env::var_os("AMOEBA_SACRIFICIAL_TARGET_ARTIFACT")
        .expect("set either AMOEBA_SPREAD_TEST_ARTIFACT or AMOEBA_SACRIFICIAL_TARGET_ARTIFACT");
    (
        read_artifact(std::path::PathBuf::from(path), "generic sacrificial target"),
        false,
    )
}

async fn maybe_account(context: &mut CeremonyContext, key: Pubkey) -> Option<Account> {
    match &mut context.backend {
        CeremonyBackend::ProgramTest(context) => context
            .banks_client
            .get_account(key)
            .await
            .expect("ProgramTest account read"),
        CeremonyBackend::Rpc(backend) => {
            backend
                .client
                .get_account_with_commitment(&key, CommitmentConfig::confirmed())
                .await
                .expect("confirmed RPC account read")
                .value
        }
    }
}

async fn wait_for_finalized_slot(context: &mut CeremonyContext, target: u64) {
    let CeremonyBackend::Rpc(backend) = &mut context.backend else {
        return;
    };
    let started = Instant::now();
    loop {
        let slot = backend
            .client
            .get_slot_with_commitment(CommitmentConfig::finalized())
            .await
            .expect("standalone finalized slot read");
        if slot >= target {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(600),
            "standalone finalized slot remained at {slot}; target {target}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

struct CeremonyAccountSnapshot {
    role: &'static str,
    pubkey: Pubkey,
    account: Account,
    finalized_slot: u64,
}

async fn capture_finalized_ceremony_accounts(
    context: &mut CeremonyContext,
    finalized_slot: u64,
    roles: &[(&'static str, Pubkey)],
) -> Vec<CeremonyAccountSnapshot> {
    wait_for_finalized_slot(context, finalized_slot).await;
    let mut snapshots = Vec::with_capacity(roles.len());
    for (role, pubkey) in roles {
        let account = match &mut context.backend {
            CeremonyBackend::ProgramTest(program_test) => program_test
                .banks_client
                .get_account(*pubkey)
                .await
                .expect("ProgramTest ceremony account read")
                .unwrap_or_else(|| panic!("missing ceremony account {pubkey}")),
            CeremonyBackend::Rpc(backend) => backend
                .client
                .get_account_with_commitment(pubkey, CommitmentConfig::finalized())
                .await
                .expect("finalized ceremony account read")
                .value
                .unwrap_or_else(|| panic!("missing finalized ceremony account {pubkey}")),
        };
        snapshots.push(CeremonyAccountSnapshot {
            role,
            pubkey: *pubkey,
            account,
            finalized_slot,
        });
    }
    snapshots
}

async fn bytes(context: &mut CeremonyContext, key: Pubkey) -> Vec<u8> {
    maybe_account(context, key)
        .await
        .unwrap_or_else(|| panic!("missing account {key}"))
        .data
}

async fn state<T: BorshDeserialize>(context: &mut CeremonyContext, key: Pubkey) -> T {
    T::try_from_slice(&bytes(context, key).await).expect("strict fixed account state")
}

async fn snapshot_accounts(context: &mut CeremonyContext, keys: &[Pubkey]) -> Vec<Option<Account>> {
    let mut snapshots = Vec::with_capacity(keys.len());
    for key in keys {
        snapshots.push(maybe_account(context, *key).await);
    }
    snapshots
}

async fn assert_accounts_unchanged(
    context: &mut CeremonyContext,
    keys: &[Pubkey],
    before: &[Option<Account>],
    label: &str,
) {
    assert_eq!(
        snapshot_accounts(context, keys).await,
        before,
        "{label} must leave every writable account byte-identical"
    );
}

async fn rpc_send_legacy(
    client: &RpcClient,
    payer: &Keypair,
    instructions: &[Instruction],
) -> Result<(), String> {
    let blockhash = client
        .get_latest_blockhash()
        .await
        .map_err(|error| error.to_string())?;
    let transaction = Transaction::new_signed_with_payer(
        instructions,
        Some(&payer.pubkey()),
        &[payer],
        blockhash,
    );
    client
        .send_and_confirm_transaction_with_spinner_and_commitment(
            &transaction,
            CommitmentConfig::confirmed(),
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

async fn rpc_submit_v0(
    backend: &mut RpcCeremonyBackend,
    payer: &Keypair,
    instructions: &[Instruction],
    signers: &[&Keypair],
) -> Result<(), String> {
    if backend.lookup_table.is_none() {
        let mut recent_slot = 0;
        for _ in 0..600 {
            let slot_hashes = backend
                .client
                .get_account_with_commitment(
                    &sysvar_ids::slot_hashes::ID,
                    CommitmentConfig::confirmed(),
                )
                .await
                .map_err(|error| error.to_string())?
                .value;
            if let Some(account) = slot_hashes {
                if account.data.len() >= 16 {
                    let count = u64::from_le_bytes(
                        account.data[..8].try_into().expect("slot-hash count bytes"),
                    );
                    if count > 0 {
                        let candidate = u64::from_le_bytes(
                            account.data[8..16]
                                .try_into()
                                .expect("first slot-hash slot bytes"),
                        );
                        if candidate > 0 {
                            recent_slot = candidate;
                            break;
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if recent_slot == 0 {
            return Err("local validator did not produce a recent lookup-table slot".into());
        }
        let (create, key) = create_lookup_table(payer.pubkey(), payer.pubkey(), recent_slot);
        rpc_send_legacy(&backend.client, payer, &[create]).await?;
        backend.lookup_table = Some(AddressLookupTableAccount {
            key,
            addresses: Vec::new(),
        });
    }

    let signer_keys = signers
        .iter()
        .map(|signer| signer.pubkey())
        .collect::<std::collections::HashSet<_>>();
    let lookup_table = backend
        .lookup_table
        .as_mut()
        .expect("lookup table initialized");
    let mut missing = Vec::new();
    for instruction in instructions {
        for meta in &instruction.accounts {
            if meta.is_signer
                || meta.pubkey == payer.pubkey()
                || signer_keys.contains(&meta.pubkey)
                || lookup_table.addresses.contains(&meta.pubkey)
                || missing.contains(&meta.pubkey)
            {
                continue;
            }
            missing.push(meta.pubkey);
        }
    }
    if !missing.is_empty() {
        for addresses in missing.chunks(20) {
            let extend = extend_lookup_table(
                lookup_table.key,
                payer.pubkey(),
                Some(payer.pubkey()),
                addresses.to_vec(),
            );
            rpc_send_legacy(&backend.client, payer, &[extend]).await?;
            lookup_table.addresses.extend_from_slice(addresses);
        }
        let extended_slot = backend
            .client
            .get_slot_with_commitment(CommitmentConfig::confirmed())
            .await
            .map_err(|error| error.to_string())?;
        for _ in 0..100 {
            if backend
                .client
                .get_slot_with_commitment(CommitmentConfig::confirmed())
                .await
                .map_err(|error| error.to_string())?
                > extended_slot
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    let blockhash = backend
        .client
        .get_latest_blockhash()
        .await
        .map_err(|error| error.to_string())?;
    let message = v0::Message::try_compile(
        &payer.pubkey(),
        instructions,
        std::slice::from_ref(lookup_table),
        blockhash,
    )
    .map_err(|error| error.to_string())?;
    let mut all = Vec::with_capacity(signers.len() + 1);
    all.push(payer);
    all.extend_from_slice(signers);
    let transaction = VersionedTransaction::try_new(VersionedMessage::V0(message), &all)
        .map_err(|error| error.to_string())?;
    backend
        .client
        .send_and_confirm_transaction_with_spinner_and_commitment(
            &transaction,
            CommitmentConfig::confirmed(),
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

async fn submit(
    context: &mut CeremonyContext,
    instructions: &[Instruction],
    signers: &[&Keypair],
) -> Result<(), String> {
    match &mut context.backend {
        CeremonyBackend::ProgramTest(context) => {
            context.last_blockhash = context
                .get_new_latest_blockhash()
                .await
                .map_err(|error| error.to_string())?;
            let mut all = Vec::with_capacity(signers.len() + 1);
            all.push(&context.payer);
            all.extend_from_slice(signers);
            let transaction = Transaction::new_signed_with_payer(
                instructions,
                Some(&context.payer.pubkey()),
                &all,
                context.last_blockhash,
            );
            context
                .banks_client
                .process_transaction(transaction)
                .await
                .map_err(|error: BanksClientError| error.to_string())
        }
        CeremonyBackend::Rpc(backend) => {
            rpc_submit_v0(backend, &context.payer, instructions, signers).await
        }
    }
}

async fn submit_program_test_with_compute_evidence(
    context: &mut CeremonyContext,
    instructions: &[Instruction],
) -> Result<u64, String> {
    let CeremonyBackend::ProgramTest(program_test) = &mut context.backend else {
        return Err("compute evidence requires ProgramTest".into());
    };
    program_test.last_blockhash = program_test
        .get_new_latest_blockhash()
        .await
        .map_err(|error| error.to_string())?;
    let transaction = Transaction::new_signed_with_payer(
        instructions,
        Some(&program_test.payer.pubkey()),
        &[&program_test.payer],
        program_test.last_blockhash,
    );
    let simulation = program_test
        .banks_client
        .simulate_transaction(transaction.clone())
        .await
        .map_err(|error| error.to_string())?;
    simulation
        .result
        .ok_or_else(|| "ProgramTest simulation omitted its result".to_string())?
        .map_err(|error| error.to_string())?;
    let units_consumed = simulation
        .simulation_details
        .ok_or_else(|| "ProgramTest simulation omitted compute details".to_string())?
        .units_consumed;
    program_test
        .banks_client
        .process_transaction(transaction)
        .await
        .map_err(|error| error.to_string())?;
    Ok(units_consumed)
}

fn lower_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[usize::from(byte >> 4)] as char);
        encoded.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn run_local_ceremony_cli(validator: &StandaloneValidator, cluster_domain: [u8; 32]) {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve governance repository root");
    let clients_dir = repository_root.join("clients/ts");
    let material_path = validator.evidence_dir.join("receipt-v4-material.json");
    let receipt_path = validator.evidence_dir.join("receipt-v4.json");
    let verification_path = validator.evidence_dir.join("receipt-v4-verification.json");
    let node_binary =
        std::env::var_os("AMOEBA_NODE_BINARY").unwrap_or_else(|| std::ffi::OsString::from("node"));
    let finalizer = Command::new(&node_binary)
        .current_dir(&clients_dir)
        .arg("--import")
        .arg("tsx")
        .arg("tools/finalize-local-ceremony-receipt.ts")
        .arg(&material_path)
        .arg(&receipt_path)
        .arg(&verification_path)
        .output()
        .expect("run checked-in receipt-v4 finalizer");
    fs::write(
        validator.evidence_dir.join("receipt-finalizer.stdout.json"),
        &finalizer.stdout,
    )
    .expect("persist receipt finalizer stdout");
    fs::write(
        validator.evidence_dir.join("receipt-finalizer.stderr.log"),
        &finalizer.stderr,
    )
    .expect("persist receipt finalizer stderr");
    assert!(
        finalizer.status.success(),
        "receipt finalizer failed: {}",
        String::from_utf8_lossy(&finalizer.stderr)
    );

    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt_path).expect("read finalized receipt v4"))
            .expect("decode finalized receipt v4");
    let payload = serde_json::to_string(&json!({
        "clusterDomainHex": lower_hex(&cluster_domain),
        "receipt": receipt,
    }))
    .expect("serialize local ceremony CLI payload");
    let journal_path = validator.evidence_dir.join("operator-journal.jsonl");
    let lock_path = validator.evidence_dir.join("operator.lock");
    let adapter_path = clients_dir.join("tools/local-ceremony-readonly-adapter.mjs");
    let cli_path = clients_dir.join("upgradeGovernance/cliMain.ts");

    for run in 1..=2 {
        let output = Command::new(&node_binary)
            .current_dir(&clients_dir)
            .arg("--import")
            .arg("tsx")
            .arg(&cli_path)
            .arg("--adapter-module")
            .arg(&adapter_path)
            .arg("verify-local-ceremony")
            .arg("--payload-json")
            .arg(&payload)
            .env("AMOEBA_LOCAL_CEREMONY_RPC", &validator.rpc_url)
            .env("AMOEBA_LOCAL_CEREMONY_JOURNAL", &journal_path)
            .env("AMOEBA_LOCAL_CEREMONY_LOCK", &lock_path)
            .output()
            .unwrap_or_else(|error| panic!("run local ceremony CLI pass {run}: {error}"));
        fs::write(
            validator
                .evidence_dir
                .join(format!("local-ceremony-cli-{run}.stdout.json")),
            &output.stdout,
        )
        .expect("persist local ceremony CLI stdout");
        fs::write(
            validator
                .evidence_dir
                .join(format!("local-ceremony-cli-{run}.stderr.log")),
            &output.stderr,
        )
        .expect("persist local ceremony CLI stderr");
        assert!(
            output.status.success(),
            "local ceremony CLI pass {run} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("decode local ceremony CLI pass {run}: {error}"));
        assert_eq!(result["status"], "verified");
        assert_eq!(result["command"], "verify-local-ceremony");
        assert_eq!(result["result"]["valid"], true);
        assert!(
            !lock_path.exists(),
            "local ceremony CLI left its lock behind"
        );
    }

    let journal = fs::read_to_string(&journal_path).expect("read durable local ceremony journal");
    assert_eq!(
        journal.lines().filter(|line| !line.is_empty()).count(),
        4,
        "two separate CLI processes must recover and extend one four-entry journal"
    );
}

async fn create_durable_nonce(context: &mut CeremonyContext) -> (Pubkey, Hash, Keypair) {
    let nonce_account = Keypair::new();
    let nonce_pubkey = nonce_account.pubkey();
    let nonce_authority = Keypair::new();
    let rent = Rent::default().minimum_balance(NonceState::size());
    let create = system_instruction::create_nonce_account(
        &context.payer.pubkey(),
        &nonce_pubkey,
        &nonce_authority.pubkey(),
        rent,
    );
    submit(context, &create, &[&nonce_account])
        .await
        .expect("create standalone durable nonce account");
    let nonce = maybe_account(context, nonce_pubkey)
        .await
        .expect("read initialized durable nonce account");
    let versions: NonceVersions =
        bincode::deserialize(&nonce.data).expect("decode initialized durable nonce state");
    let NonceState::Initialized(data) = versions.state() else {
        panic!("durable nonce account remained uninitialized");
    };
    (nonce_pubkey, data.blockhash(), nonce_authority)
}

async fn submit_with_durable_nonce(
    context: &mut CeremonyContext,
    instructions: &[Instruction],
    signers: &[&Keypair],
    nonce_blockhash: Hash,
) -> Result<String, String> {
    match &mut context.backend {
        CeremonyBackend::ProgramTest(program_test) => {
            let mut all = Vec::with_capacity(signers.len() + 1);
            all.push(&program_test.payer);
            all.extend_from_slice(signers);
            let transaction = Transaction::new_signed_with_payer(
                instructions,
                Some(&program_test.payer.pubkey()),
                &all,
                nonce_blockhash,
            );
            let signature = lower_hex(transaction.signatures[0].as_ref());
            program_test
                .banks_client
                .process_transaction(transaction)
                .await
                .map_err(|error: BanksClientError| error.to_string())?;
            Ok(signature)
        }
        CeremonyBackend::Rpc(backend) => {
            let lookup_tables = backend.lookup_table.clone().into_iter().collect::<Vec<_>>();
            let message = v0::Message::try_compile(
                &context.payer.pubkey(),
                instructions,
                &lookup_tables,
                nonce_blockhash,
            )
            .map_err(|error| error.to_string())?;
            let mut all = Vec::with_capacity(signers.len() + 1);
            all.push(&context.payer);
            all.extend_from_slice(signers);
            let transaction = VersionedTransaction::try_new(VersionedMessage::V0(message), &all)
                .map_err(|error| error.to_string())?;
            let signature = lower_hex(transaction.signatures[0].as_ref());
            backend
                .client
                .send_and_confirm_transaction_with_spinner_and_commitment(
                    &transaction,
                    CommitmentConfig::confirmed(),
                )
                .await
                .map_err(|error| error.to_string())?;
            Ok(signature)
        }
    }
}

struct FailedSubmissionEvidence {
    signature_hex: String,
    error: String,
    observed_slot: u64,
}

async fn submit_expected_failure_with_evidence(
    context: &mut CeremonyContext,
    instructions: &[Instruction],
    signers: &[&Keypair],
) -> FailedSubmissionEvidence {
    match &mut context.backend {
        CeremonyBackend::ProgramTest(program_test) => {
            program_test.last_blockhash = program_test
                .get_new_latest_blockhash()
                .await
                .expect("fresh ProgramTest failure-attempt blockhash");
            let mut all = Vec::with_capacity(signers.len() + 1);
            all.push(&program_test.payer);
            all.extend_from_slice(signers);
            let transaction = Transaction::new_signed_with_payer(
                instructions,
                Some(&program_test.payer.pubkey()),
                &all,
                program_test.last_blockhash,
            );
            let signature_hex = lower_hex(transaction.signatures[0].as_ref());
            let error = program_test
                .banks_client
                .process_transaction(transaction)
                .await
                .expect_err("failure-evidence transaction unexpectedly succeeded")
                .to_string();
            let observed_slot = program_test
                .banks_client
                .get_sysvar::<solana_sdk::clock::Clock>()
                .await
                .expect("failure-evidence ProgramTest slot")
                .slot;
            FailedSubmissionEvidence {
                signature_hex,
                error,
                observed_slot,
            }
        }
        CeremonyBackend::Rpc(backend) => {
            let blockhash = backend
                .client
                .get_latest_blockhash()
                .await
                .expect("failure-evidence RPC blockhash");
            let lookup_tables = backend.lookup_table.clone().into_iter().collect::<Vec<_>>();
            let message = v0::Message::try_compile(
                &context.payer.pubkey(),
                instructions,
                &lookup_tables,
                blockhash,
            )
            .expect("compile failure-evidence v0 message");
            let mut all = Vec::with_capacity(signers.len() + 1);
            all.push(&context.payer);
            all.extend_from_slice(signers);
            let transaction = VersionedTransaction::try_new(VersionedMessage::V0(message), &all)
                .expect("sign failure-evidence v0 message");
            let signature_hex = lower_hex(transaction.signatures[0].as_ref());
            let error = backend
                .client
                .send_and_confirm_transaction_with_spinner_and_commitment(
                    &transaction,
                    CommitmentConfig::confirmed(),
                )
                .await
                .expect_err("failure-evidence transaction unexpectedly succeeded")
                .to_string();
            let observed_slot = backend
                .client
                .get_slot_with_commitment(CommitmentConfig::confirmed())
                .await
                .expect("failure-evidence RPC slot");
            FailedSubmissionEvidence {
                signature_hex,
                error,
                observed_slot,
            }
        }
    }
}

async fn advance_to_slot(context: &mut CeremonyContext, target: u64) -> Result<(), String> {
    match &mut context.backend {
        CeremonyBackend::ProgramTest(context) => {
            let current = context
                .banks_client
                .get_sysvar::<solana_sdk::clock::Clock>()
                .await
                .map_err(|error| error.to_string())?
                .slot;
            if current < target {
                context
                    .warp_to_slot(target)
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        }
        CeremonyBackend::Rpc(backend) => {
            let start_slot = backend
                .client
                .get_slot_with_commitment(CommitmentConfig::confirmed())
                .await
                .map_err(|error| error.to_string())?;
            if start_slot >= target {
                return Ok(());
            }
            let remaining_slots = target - start_slot;
            let max_wait = Duration::from_millis(
                remaining_slots
                    .saturating_mul(700)
                    .saturating_add(30_000)
                    .min(1_800_000),
            );
            let started = Instant::now();
            loop {
                let current_slot = backend
                    .client
                    .get_slot_with_commitment(CommitmentConfig::confirmed())
                    .await
                    .map_err(|error| error.to_string())?;
                if current_slot >= target {
                    return Ok(());
                }
                if started.elapsed() >= max_wait {
                    return Err(format!(
                        "confirmed local-validator slot remained at {current_slot}; target {target}, start {start_slot}, waited {} ms",
                        started.elapsed().as_millis()
                    ));
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

fn ro(pubkey: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(pubkey, false)
}

fn sr(pubkey: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(pubkey, true)
}

fn rw(pubkey: Pubkey) -> AccountMeta {
    AccountMeta::new(pubkey, false)
}

fn sw(pubkey: Pubkey) -> AccountMeta {
    AccountMeta::new(pubkey, true)
}

fn typed_instruction(program_id: Pubkey, accounts: Vec<AccountMeta>, data: Vec<u8>) -> Instruction {
    Instruction {
        program_id,
        accounts,
        data,
    }
}

fn envelope() -> CeremonyEnvelopeV1 {
    CeremonyEnvelopeV1 {
        compute_unit_limit: OBSERVATION_COMPUTE_LIMIT,
        compute_unit_price_micro_lamports: 1,
        durable_nonce_account: OptionalPubkeyV1::none(),
        durable_nonce_authority: OptionalPubkeyV1::none(),
    }
}

fn envelope_with_nonce(nonce: Pubkey, authority: Pubkey) -> CeremonyEnvelopeV1 {
    CeremonyEnvelopeV1 {
        compute_unit_limit: OBSERVATION_COMPUTE_LIMIT,
        compute_unit_price_micro_lamports: 1,
        durable_nonce_account: OptionalPubkeyV1::some(nonce)
            .expect("nondefault durable nonce account"),
        durable_nonce_authority: OptionalPubkeyV1::some(authority)
            .expect("nondefault durable nonce authority"),
    }
}

fn envelope_prefix() -> [Instruction; 2] {
    [
        ComputeBudgetInstruction::set_compute_unit_limit(OBSERVATION_COMPUTE_LIMIT),
        ComputeBudgetInstruction::set_compute_unit_price(1),
    ]
}

struct CeremonyHarness {
    controller: Pubkey,
    controller_programdata: Pubkey,
    target: Pubkey,
    target_programdata: Pubkey,
    config: Pubkey,
    authority: Pubkey,
    gate: Pubkey,
    policy: Pubkey,
    council: Pubkey,
    capacity_policy: Pubkey,
    controller_release: Pubkey,
    immutability_receipt: Pubkey,
    lifecycle_registry: Pubkey,
    timing_profile: Pubkey,
    handoff_proposal: Pubkey,
    handoff_receipt: Pubkey,
    activation_proposal: Pubkey,
    activation_receipt: Pubkey,
    deployment: Pubkey,
    initializer: Keypair,
    legacy_authority: Keypair,
    guardian: Keypair,
    seats: [Keypair; 5],
    spill: Pubkey,
    controller_bootstrap_buffer: Pubkey,
    target_bootstrap_buffer: Pubkey,
    former_authority_buffer: Pubkey,
}

fn policy_and_council(
    harness: &CeremonyHarness,
    policy_activation_slot: u64,
) -> (GovernancePolicyV1, GovernanceCouncilSetV1) {
    let mut policy = GovernancePolicyV1 {
        discriminator: GOVERNANCE_POLICY_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump: derive_policy_pda(&harness.controller, &harness.target, 1).1,
        initialized: true,
        controller_config: harness.config,
        version: 1,
        target_program: harness.target,
        activation_slot: policy_activation_slot,
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

    let seats = std::array::from_fn(|index| CouncilSeatV1 {
        seat_authority: harness.seats[index].pubkey(),
        term_start_slot: policy_activation_slot,
        term_end_slot: u64::MAX,
        active: true,
        reserved: [0; COUNCIL_SEAT_RESERVED_LEN],
    });
    let mut council = GovernanceCouncilSetV1 {
        discriminator: GOVERNANCE_COUNCIL_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump: derive_council_pda(&harness.controller, &harness.target, 1).1,
        initialized: true,
        controller_config: harness.config,
        version: 1,
        target_program: harness.target,
        activation_slot: policy_activation_slot,
        deactivation_slot: 0,
        seats,
        routine_threshold: 3,
        terminal_threshold: 4,
        policy_flags: 0,
        set_hash: [0; 32],
        reserved: [0; GOVERNANCE_COUNCIL_RESERVED_LEN],
    };
    council.set_hash = compute_council_set_hash(&council);
    (policy, council)
}

fn capacity_policy_candidate(harness: &CeremonyHarness) -> ProgramDataCapacityPolicyV1 {
    let chunk_count = programdata_observation_chunk_count(
        MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
        PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB,
    )
    .expect("runtime maximum geometry");
    let padded = chunk_count.next_power_of_two();
    let mut value = ProgramDataCapacityPolicyV1 {
        discriminator: PROGRAMDATA_CAPACITY_POLICY_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: derive_capacity_policy_pda(&harness.controller, &harness.target).1,
        initialized: true,
        controller_program: harness.controller,
        controller_config: harness.config,
        target_program: harness.target,
        target_programdata: harness.target_programdata,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        loader_programdata_metadata_len: LOADER_PROGRAMDATA_METADATA_LEN as u64,
        maximum_raw_programdata_length: MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
        maximum_payload_capacity: MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1,
        maximum_artifact_length: upgrade_controller::artifact_merkle::MAX_ARTIFACT_BYTES_V1,
        observation_scheme_id: PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
        observation_chunk_size: PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB,
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
        creation_slot: INITIALIZATION_SLOT,
        reserved: [0; PROGRAMDATA_CAPACITY_POLICY_V1_RESERVED_LEN],
    };
    value.policy_digest = compute_capacity_policy_digest_v1(&value).expect("capacity digest");
    value
}

fn controller_release_candidate(
    harness: &CeremonyHarness,
    capacity: &ProgramDataCapacityPolicyV1,
    controller_artifact: &[u8],
) -> ControllerReleaseCommitmentV1 {
    let mut value = ControllerReleaseCommitmentV1 {
        discriminator: CONTROLLER_RELEASE_COMMITMENT_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: derive_controller_release_commitment_pda(&harness.controller, &harness.target).1,
        initialized: true,
        controller_program: harness.controller,
        controller_programdata: harness.controller_programdata,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        capacity_policy: harness.capacity_policy,
        capacity_policy_digest: capacity.policy_digest,
        artifact_length: controller_artifact.len() as u64,
        artifact_sha256: hashv(&[controller_artifact]).to_bytes(),
        artifact_merkle_root: artifact_merkle_root(
            controller_artifact,
            ARTIFACT_BINDING_CHUNK_SIZE_V1,
        )
        .expect("controller artifact root"),
        artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
        source_commitment: digest("controller-source"),
        source_tree_commitment: digest("controller-source-tree"),
        build_inputs_commitment: digest("controller-build-inputs"),
        toolchain_commitment: digest("controller-toolchain"),
        package_commitment: digest("controller-package"),
        release_manifest_commitment: digest("controller-release-manifest"),
        abi_commitment: digest("controller-abi"),
        pre_immutability_authority: OptionalPubkeyV1::some(harness.initializer.pubkey())
            .expect("initializer option"),
        minimum_programdata_capacity: controller_artifact.len() as u64,
        release_digest: [0; 32],
        creation_slot: INITIALIZATION_SLOT,
        finalized: true,
        reserved: [0; CONTROLLER_RELEASE_COMMITMENT_V1_RESERVED_LEN],
    };
    value.release_digest =
        compute_controller_release_digest_v1(&value).expect("controller release digest");
    value
}

async fn initialize_controller(
    context: &mut CeremonyContext,
    harness: &CeremonyHarness,
    controller_artifact: &[u8],
) {
    let capacity = capacity_policy_candidate(harness);
    let release = controller_release_candidate(harness, &capacity, controller_artifact);
    let (policy, council) = policy_and_council(harness, INITIALIZATION_SLOT);
    let instruction = InitializeControllerV2 {
        cluster_domain: context.cluster_domain,
        initial_policy_version: 1,
        initial_council_version: 1,
        next_proposal_id: 1,
        target_nonce: 1,
        initial_gate_epoch: 1,
        policy_activation_slot: INITIALIZATION_SLOT,
        routine_delay_slots: ROUTINE_DELAY,
        major_delay_slots: MAJOR_DELAY,
        rollback_delay_slots: ROLLBACK_DELAY,
        terminal_delay_slots: TERMINAL_DELAY,
        vote_review_slots: REVIEW_SLOTS,
        proposal_expiry_slots: EXPIRY_SLOTS,
        expected_policy_hash: policy.policy_hash,
        expected_council_hash: council.set_hash,
        seat_terms: std::array::from_fn(|_| SeatTermV2 {
            term_start_slot: INITIALIZATION_SLOT,
            term_end_slot: u64::MAX,
        }),
        capacity_policy: CapacityPolicyInputV1 {
            expected_policy_digest: capacity.policy_digest,
        },
        controller_release: ControllerReleaseInputV1 {
            artifact_length: release.artifact_length,
            artifact_sha256: release.artifact_sha256,
            artifact_merkle_root: release.artifact_merkle_root,
            source_commitment: release.source_commitment,
            source_tree_commitment: release.source_tree_commitment,
            build_inputs_commitment: release.build_inputs_commitment,
            toolchain_commitment: release.toolchain_commitment,
            package_commitment: release.package_commitment,
            release_manifest_commitment: release.release_manifest_commitment,
            abi_commitment: release.abi_commitment,
            expected_release_digest: release.release_digest,
        },
    };
    let initialize = initialize_controller_v2_instruction(
        harness.controller,
        InitializeControllerV2Accounts {
            payer: context.payer.pubkey(),
            initializer: harness.initializer.pubkey(),
            controller_program: harness.controller,
            controller_programdata: harness.controller_programdata,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            controller_config: harness.config,
            authority_pda: harness.authority,
            protocol_gate: harness.gate,
            policy: harness.policy,
            council: harness.council,
            capacity_policy: harness.capacity_policy,
            controller_release: harness.controller_release,
            canonical_spill_treasury: harness.spill,
            guardian: harness.guardian.pubkey(),
            seat_authorities: harness.seats.each_ref().map(Signer::pubkey),
            system_program: system_program::ID,
        },
        instruction,
    )
    .expect("initialize instruction");
    submit(context, &[initialize], &[&harness.initializer])
        .await
        .expect("actual-SBF controller initialization");
}

#[allow(clippy::too_many_arguments)]
async fn observe_programdata(
    context: &mut CeremonyContext,
    harness: &CeremonyHarness,
    observed_program: Pubkey,
    observed_programdata: Pubkey,
    subject: Pubkey,
    purpose: ProgramDataObservationPurposeV1,
    generation: u64,
    artifact: &[u8],
    expected_authority: Option<Pubkey>,
) -> (Pubkey, ProgramDataObservationV1) {
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let capacity: ProgramDataCapacityPolicyV1 = state(context, harness.capacity_policy).await;
    let raw = bytes(context, observed_programdata).await;
    let parsed = parse_upgradeable_programdata(&raw).expect("canonical observed ProgramData");
    assert_eq!(parsed.upgrade_authority, expected_authority);
    let artifact_sha256 = hashv(&[artifact]).to_bytes();
    let artifact_root =
        artifact_merkle_root(artifact, ARTIFACT_BINDING_CHUNK_SIZE_V1).expect("artifact root");
    let subject_digest = compute_programdata_observation_subject_digest_v1(
        &harness.controller,
        &harness.config,
        &observed_program,
        &observed_programdata,
        purpose,
        &subject,
        generation,
        &harness.gate,
        gate.status,
        gate.epoch,
        &gate.active_proposal,
        gate.freeze_slot,
        gate.freeze_reason_code,
        &capacity.policy_digest,
        artifact.len() as u64,
        &artifact_sha256,
        &artifact_root,
        &ARTIFACT_MERKLE_SCHEME_ID,
        artifact.len() as u64,
    )
    .expect("observation subject digest");
    let observation = derive_programdata_observation_pda(
        &harness.controller,
        &observed_program,
        purpose as u8,
        &subject_digest,
        generation,
    )
    .0;
    let guard = ProgramDataObservationGuardV1 {
        purpose,
        generation,
        expected_subject_digest: subject_digest,
        expected_gate_status: gate.status,
        expected_gate_epoch: gate.epoch,
        expected_freeze_reason_code: gate.freeze_reason_code,
        expected_freeze_slot: gate.freeze_slot,
    };
    let authority = match expected_authority {
        Some(value) => ObservationAuthorityV1::some(value).expect("nondefault authority"),
        None => ObservationAuthorityV1::none(),
    };
    let begin = begin_programdata_observation_instruction(
        harness.controller,
        context.payer.pubkey(),
        harness.config,
        harness.gate,
        harness.capacity_policy,
        subject,
        observed_program,
        observed_programdata,
        observation,
        UPGRADEABLE_LOADER_ID,
        system_program::ID,
        BeginProgramDataObservationV1 {
            guard,
            expected_capacity_policy_digest: capacity.policy_digest,
            expected_artifact_length: artifact.len() as u64,
            expected_artifact_sha256: artifact_sha256,
            expected_artifact_merkle_root: artifact_root,
            expected_artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            minimum_required_capacity: artifact.len() as u64,
            expected_deployed_slot: parsed.deployed_slot,
            expected_actual_capacity: parsed.capacity as u64,
            expected_upgrade_authority: authority,
        },
    )
    .expect("begin observation instruction");
    let [limit, price] = envelope_prefix();
    submit(context, &[limit, price, begin], &[])
        .await
        .expect("begin bounded ProgramData observation");

    let raw_chunk_count =
        programdata_observation_chunk_count(raw.len() as u64, capacity.observation_chunk_size)
            .expect("raw chunk count");
    for chunk_index in 0..raw_chunk_count {
        let append = append_programdata_observation_chunk_instruction(
            harness.controller,
            harness.config,
            harness.gate,
            harness.capacity_policy,
            subject,
            observed_program,
            observed_programdata,
            observation,
            UPGRADEABLE_LOADER_ID,
            AppendProgramDataObservationChunkV1 {
                guard,
                expected_status: ProgramDataObservationStatusV1::Accumulating,
                chunk_index,
            },
        )
        .expect("append observation instruction");
        let [limit, price] = envelope_prefix();
        if chunk_index == 0
            && std::env::var_os("AMOEBA_OBSERVATION_COMPUTE_EVIDENCE").is_some()
            && matches!(&context.backend, CeremonyBackend::ProgramTest(_))
        {
            let units_consumed =
                submit_program_test_with_compute_evidence(context, &[limit, price, append])
                    .await
                    .expect("simulate and commit exact ProgramData chunk");
            assert!(
                units_consumed <= MAX_SELECTED_OBSERVATION_CHUNK_CU_V1,
                "selected observation chunk consumed {units_consumed} CU, exceeding conservative {MAX_SELECTED_OBSERVATION_CHUNK_CU_V1} CU ceiling"
            );
            let sbpf_target =
                std::env::var("AMOEBA_SBPF_TARGET").unwrap_or_else(|_| "unspecified".into());
            println!(
                "AMOEBA_PROGRAMDATA_OBSERVATION_CHUNK_SBF_EVIDENCE={{\"sbpf_target\":\"{}\",\"chunk_size\":{},\"raw_programdata_length\":{},\"raw_chunk_count\":{},\"units_consumed\":{},\"conservative_ceiling\":{},\"runtime_stack_fault\":false}}",
                sbpf_target,
                capacity.observation_chunk_size,
                raw.len(),
                raw_chunk_count,
                units_consumed,
                MAX_SELECTED_OBSERVATION_CHUNK_CU_V1,
            );
        } else {
            submit(context, &[limit, price, append], &[])
                .await
                .expect("append exact ProgramData chunk");
        }
    }

    let artifact_chunk_count =
        artifact_chunk_count(artifact.len() as u64, ARTIFACT_BINDING_CHUNK_SIZE_V1)
            .expect("artifact chunk count");
    for chunk_index in 0..artifact_chunk_count {
        let before: ProgramDataObservationV1 = state(context, observation).await;
        let proof = artifact_merkle_proof(artifact, ARTIFACT_BINDING_CHUNK_SIZE_V1, chunk_index)
            .expect("artifact proof");
        let mut nodes = [[0; 32]; MAX_ARTIFACT_PROOF_DEPTH_V1];
        nodes[..proof.len()].copy_from_slice(&proof);
        let verify = verify_observed_artifact_chunk_instruction(
            harness.controller,
            harness.config,
            harness.gate,
            harness.capacity_policy,
            subject,
            observed_program,
            observed_programdata,
            observation,
            UPGRADEABLE_LOADER_ID,
            VerifyObservedArtifactChunkV1 {
                guard,
                expected_status: ProgramDataObservationStatusV1::Accumulating,
                chunk_index,
                expected_next_artifact_chunk_index: before.next_artifact_chunk_index,
                expected_tail_bytes_verified: before.tail_bytes_verified,
                proof: ObservedArtifactMerkleProofV1 {
                    proof_len: proof.len() as u8,
                    nodes,
                },
            },
        )
        .expect("verify observation artifact instruction");
        let [limit, price] = envelope_prefix();
        submit(context, &[limit, price, verify], &[])
            .await
            .expect("verify exact observed artifact chunk");
    }

    let ready: ProgramDataObservationV1 = state(context, observation).await;
    assert_eq!(
        ready.status,
        ProgramDataObservationStatusV1::ReadyToFinalize
    );
    let finalize = finalize_programdata_observation_instruction(
        harness.controller,
        harness.config,
        harness.gate,
        harness.capacity_policy,
        subject,
        observed_program,
        observed_programdata,
        observation,
        UPGRADEABLE_LOADER_ID,
        FinalizeProgramDataObservationV1 {
            guard,
            expected_status: ProgramDataObservationStatusV1::ReadyToFinalize,
            expected_next_raw_chunk_index: ready.next_raw_chunk_index,
            expected_next_artifact_chunk_index: ready.next_artifact_chunk_index,
            expected_tail_bytes_verified: ready.tail_bytes_verified,
        },
    )
    .expect("finalize observation instruction");
    let [limit, price] = envelope_prefix();
    submit(context, &[limit, price, finalize], &[])
        .await
        .expect("finalize exact ProgramData observation");
    let finalized: ProgramDataObservationV1 = state(context, observation).await;
    assert_eq!(finalized.status, ProgramDataObservationStatusV1::Finalized);
    assert_ne!(finalized.observation_digest, [0; 32]);
    (observation, finalized)
}

/// Begins the exact PostUpgrade observation used by the V3 mechanical failure
/// witness, but deliberately leaves it accumulating.  The failure instruction
/// authenticates the expected leaf and reads the mismatching live bytes itself;
/// an operator cannot convert an incomplete observation into rollback authority.
#[allow(clippy::too_many_arguments)]
async fn begin_failure_programdata_observation(
    context: &mut CeremonyContext,
    harness: &CeremonyHarness,
    proposal_key: Pubkey,
    artifact: &[u8],
    generation: u64,
) -> (Pubkey, ProgramDataObservationV1) {
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let capacity: ProgramDataCapacityPolicyV1 = state(context, harness.capacity_policy).await;
    let raw = bytes(context, harness.target_programdata).await;
    let parsed = parse_upgradeable_programdata(&raw).expect("canonical failed ProgramData");
    assert_eq!(parsed.upgrade_authority, Some(harness.authority));
    let artifact_sha256 = hashv(&[artifact]).to_bytes();
    let artifact_root = artifact_merkle_root(artifact, ARTIFACT_BINDING_CHUNK_SIZE_V1)
        .expect("failure observation artifact root");
    let purpose = ProgramDataObservationPurposeV1::PostUpgrade;
    let subject_digest = compute_programdata_observation_subject_digest_v1(
        &harness.controller,
        &harness.config,
        &harness.target,
        &harness.target_programdata,
        purpose,
        &proposal_key,
        generation,
        &harness.gate,
        gate.status,
        gate.epoch,
        &gate.active_proposal,
        gate.freeze_slot,
        gate.freeze_reason_code,
        &capacity.policy_digest,
        artifact.len() as u64,
        &artifact_sha256,
        &artifact_root,
        &ARTIFACT_MERKLE_SCHEME_ID,
        artifact.len() as u64,
    )
    .expect("failure observation subject digest");
    let observation = derive_programdata_observation_pda(
        &harness.controller,
        &harness.target,
        purpose as u8,
        &subject_digest,
        generation,
    )
    .0;
    let guard = ProgramDataObservationGuardV1 {
        purpose,
        generation,
        expected_subject_digest: subject_digest,
        expected_gate_status: gate.status,
        expected_gate_epoch: gate.epoch,
        expected_freeze_reason_code: gate.freeze_reason_code,
        expected_freeze_slot: gate.freeze_slot,
    };
    let begin = begin_programdata_observation_instruction(
        harness.controller,
        context.payer.pubkey(),
        harness.config,
        harness.gate,
        harness.capacity_policy,
        proposal_key,
        harness.target,
        harness.target_programdata,
        observation,
        UPGRADEABLE_LOADER_ID,
        system_program::ID,
        BeginProgramDataObservationV1 {
            guard,
            expected_capacity_policy_digest: capacity.policy_digest,
            expected_artifact_length: artifact.len() as u64,
            expected_artifact_sha256: artifact_sha256,
            expected_artifact_merkle_root: artifact_root,
            expected_artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            minimum_required_capacity: artifact.len() as u64,
            expected_deployed_slot: parsed.deployed_slot,
            expected_actual_capacity: parsed.capacity as u64,
            expected_upgrade_authority: ObservationAuthorityV1::some(harness.authority)
                .expect("controller authority"),
        },
    )
    .expect("begin failure observation instruction");
    let [limit, price] = envelope_prefix();
    submit(context, &[limit, price, begin], &[])
        .await
        .expect("begin exact accumulating failure observation through controller SBF");
    let accumulating: ProgramDataObservationV1 = state(context, observation).await;
    assert_eq!(
        accumulating.status,
        ProgramDataObservationStatusV1::Accumulating
    );
    (observation, accumulating)
}

#[allow(clippy::too_many_arguments)]
async fn build_programdata_failure_instruction(
    context: &mut CeremonyContext,
    harness: &CeremonyHarness,
    primary_key: Pubkey,
    observation_key: Pubkey,
    observation: &ProgramDataObservationV1,
    mismatch_class: ProgramDataMismatchClassV2,
    failing_chunk_index: u32,
    expected_leaf_hash: [u8; 32],
    proof: &[[u8; 32]],
) -> (Pubkey, Instruction) {
    let config: ControllerConfigV1 = state(context, harness.config).await;
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let capacity: ProgramDataCapacityPolicyV1 = state(context, harness.capacity_policy).await;
    let deployment: CurrentDeploymentStateV1 = state(context, harness.deployment).await;
    let primary: UpgradeProposalV3 = state(context, primary_key).await;
    let observation_state_hash = hashv(&[&bytes(context, observation_key).await]).to_bytes();
    let (failure_key, _) =
        derive_programdata_failure_observation_pda(&harness.controller, &primary_key, gate.epoch);
    let mut nodes = [[0; 32]; MAX_ARTIFACT_PROOF_DEPTH_V1];
    nodes[..proof.len()].copy_from_slice(proof);
    let instruction = observe_programdata_failure_v2_instruction(
        harness.controller,
        ObserveProgramDataFailureV2Accounts {
            payer: context.payer.pubkey(),
            controller_config: harness.config,
            protocol_gate: harness.gate,
            primary_proposal: primary_key,
            programdata_verification: primary.programdata_verification,
            capacity_policy: harness.capacity_policy,
            current_deployment: harness.deployment,
            programdata_observation: observation_key,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            failure_observation: failure_key,
            system_program: system_program::ID,
        },
        ObserveProgramDataFailureV2 {
            witness: ProgramDataFailureWitnessV2 {
                expected_proposal: proposal_guard(&primary, &config, &gate, &capacity, &deployment),
                expected_verification_digest: [0; 32],
                expected_verification_generation: 0,
                expected_observation_generation: observation.generation,
                expected_observation_state_hash: observation_state_hash,
                mismatch_class,
                failing_chunk_index,
                expected_leaf_hash,
                proof: ProgramDataFailureProofV2 {
                    proof_len: proof.len() as u8,
                    nodes,
                },
                plan_valid_until_slot: primary.expiry_slot,
            },
        },
    )
    .expect("build V3 ProgramData failure witness instruction");
    (failure_key, instruction)
}

async fn inject_programdata_payload_fault(
    context: &mut CeremonyContext,
    programdata: Pubkey,
    relative_payload_offset: usize,
) -> (u8, u8) {
    let CeremonyBackend::ProgramTest(program_test) = &mut context.backend else {
        panic!("payload fault injection is permitted only in isolated ProgramTest");
    };
    let mut account = program_test
        .banks_client
        .get_account(programdata)
        .await
        .expect("read ProgramData before local fault injection")
        .expect("ProgramData exists before local fault injection");
    let absolute_offset = LOADER_PROGRAMDATA_METADATA_LEN
        .checked_add(relative_payload_offset)
        .expect("fault offset arithmetic");
    let before = *account
        .data
        .get(absolute_offset)
        .expect("fault offset inside ProgramData payload");
    let after = before ^ 1;
    account.data[absolute_offset] = after;
    program_test.set_account(&programdata, &AccountSharedData::from(account));
    (before, after)
}

async fn record_controller_immutability(
    context: &mut CeremonyContext,
    harness: &CeremonyHarness,
    pre_key: Pubkey,
    pre: &ProgramDataObservationV1,
    post_key: Pubkey,
    post: &ProgramDataObservationV1,
) -> ControllerImmutabilityReceiptV1 {
    let capacity: ProgramDataCapacityPolicyV1 = state(context, harness.capacity_policy).await;
    let release: ControllerReleaseCommitmentV1 = state(context, harness.controller_release).await;
    let bump = derive_controller_immutability_receipt_pda(&harness.controller, &harness.target).1;
    let mut receipt = ControllerImmutabilityReceiptV1 {
        discriminator: CONTROLLER_IMMUTABILITY_RECEIPT_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        controller_program: harness.controller,
        controller_programdata: harness.controller_programdata,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        capacity_policy: harness.capacity_policy,
        capacity_policy_digest: capacity.policy_digest,
        release_commitment: harness.controller_release,
        release_commitment_digest: release.release_digest,
        pre_observation: pre_key,
        pre_observation_generation: pre.generation,
        pre_observation_root: pre.final_raw_merkle_root,
        pre_observation_digest: pre.observation_digest,
        pre_upgrade_authority: pre.upgrade_authority,
        post_observation: post_key,
        post_observation_generation: post.generation,
        post_observation_root: post.final_raw_merkle_root,
        post_observation_digest: post.observation_digest,
        post_upgrade_authority: post.upgrade_authority,
        deployed_slot: post.deployed_slot,
        raw_programdata_length: post.raw_data_length,
        programdata_capacity: post.actual_capacity,
        artifact_length: release.artifact_length,
        artifact_sha256: release.artifact_sha256,
        artifact_merkle_root: release.artifact_merkle_root,
        artifact_scheme_id: release.artifact_scheme_id,
        source_commitment: release.source_commitment,
        build_inputs_commitment: release.build_inputs_commitment,
        package_commitment: release.package_commitment,
        release_manifest_commitment: release.release_manifest_commitment,
        finalized_slot: post.finalized_slot,
        receipt_digest: [0; 32],
        finalized: true,
        reserved: [0; CONTROLLER_IMMUTABILITY_RECEIPT_V1_RESERVED_LEN],
    };
    receipt.receipt_digest =
        compute_controller_immutability_receipt_digest_v1(&receipt).expect("immutability digest");
    let instruction = typed_instruction(
        harness.controller,
        vec![
            sw(context.payer.pubkey()),
            ro(harness.controller),
            ro(harness.controller_programdata),
            ro(harness.config),
            ro(harness.capacity_policy),
            ro(harness.controller_release),
            ro(pre_key),
            ro(post_key),
            rw(harness.immutability_receipt),
            ro(UPGRADEABLE_LOADER_ID),
            ro(system_program::ID),
        ],
        RecordControllerImmutabilityV1 {
            expected_capacity_policy_digest: capacity.policy_digest,
            expected_release_digest: release.release_digest,
            expected_pre_observation_digest: pre.observation_digest,
            expected_post_observation_digest: post.observation_digest,
            expected_receipt_digest: receipt.receipt_digest,
        }
        .pack()
        .expect("record immutability payload"),
    );
    submit(context, &[instruction], &[])
        .await
        .expect("record immutable controller receipt");
    let observed: ControllerImmutabilityReceiptV1 =
        state(context, harness.immutability_receipt).await;
    assert_eq!(observed, receipt);
    observed
}

// Fresh controllers use the V2 registry and timing profile before authority handoff.
// This is isolated legacy-controller coverage, separate from the upgradeable V3 core.
async fn initialize_lifecycle_registry(context: &mut CeremonyContext, harness: &CeremonyHarness) {
    let config: ControllerConfigV1 = state(context, harness.config).await;
    let council: GovernanceCouncilSetV1 = state(context, harness.council).await;
    let profile = nominal_governance_timing_profile_v1(
        derive_governance_timing_profile_v1(&harness.controller, &harness.target, 1).1,
        harness.config,
        harness.target,
        council.version,
        current_slot(context).await,
    )
    .expect("nominal timing profile");
    let initialize = typed_instruction(
        harness.controller,
        vec![
            sw(context.payer.pubkey()),
            sr(harness.initializer.pubkey()),
            ro(harness.controller),
            ro(harness.controller_programdata),
            ro(harness.config),
            ro(harness.policy),
            ro(harness.council),
            ro(harness.gate),
            rw(harness.lifecycle_registry),
            rw(harness.timing_profile),
            ro(system_program::ID),
        ],
        InitializeGovernanceLifecycleRegistryV2 {
            expected_initial_timing_profile_version: profile.profile_version,
            expected_initial_timing_profile_hash: profile.profile_hash,
            expected_initial_next_proposal_id: config.next_proposal_id,
            expected_initial_rotation_nonce: 1,
        }
        .pack()
        .expect("initialize lifecycle registry payload"),
    );
    submit(context, &[initialize], &[&harness.initializer])
        .await
        .expect("initialize supported governance lifecycle");
}

fn handoff_review_accounts(harness: &CeremonyHarness, observation: Pubkey) -> Vec<AccountMeta> {
    vec![
        ro(harness.controller),
        ro(harness.controller_programdata),
        ro(harness.config),
        ro(harness.policy),
        ro(harness.council),
        ro(harness.gate),
        ro(harness.capacity_policy),
        ro(harness.immutability_receipt),
        ro(observation),
        ro(harness.target),
        ro(harness.target_programdata),
        ro(harness.legacy_authority.pubkey()),
        ro(harness.authority),
        ro(harness.lifecycle_registry),
        ro(harness.timing_profile),
        rw(harness.handoff_proposal),
        ro(UPGRADEABLE_LOADER_ID),
    ]
}

async fn governed_handoff(
    context: &mut CeremonyContext,
    harness: &CeremonyHarness,
    observation_key: Pubkey,
    observation: &ProgramDataObservationV1,
) -> TargetAuthorityHandoffReceiptV1 {
    let config: upgrade_controller::state::ControllerConfigV1 =
        state(context, harness.config).await;
    let council: GovernanceCouncilSetV1 = state(context, harness.council).await;
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let profile: GovernanceTimingProfileV1 = state(context, harness.timing_profile).await;
    let create = typed_instruction(
        harness.controller,
        vec![
            sw(context.payer.pubkey()),
            sr(harness.seats[0].pubkey()),
            ro(harness.controller),
            ro(harness.controller_programdata),
            ro(harness.config),
            ro(harness.policy),
            ro(harness.council),
            ro(harness.gate),
            ro(harness.capacity_policy),
            ro(harness.immutability_receipt),
            ro(observation_key),
            ro(harness.target),
            ro(harness.target_programdata),
            ro(harness.legacy_authority.pubkey()),
            ro(harness.authority),
            rw(harness.lifecycle_registry),
            ro(harness.timing_profile),
            rw(harness.handoff_proposal),
            ro(UPGRADEABLE_LOADER_ID),
            ro(system_program::ID),
        ],
        CreateTargetAuthorityHandoffProposalV2 {
            expected_proposal_id: 1,
            expected_timing_profile_version: profile.profile_version,
            expected_timing_profile_hash: profile.profile_hash,
            expected_gate_epoch: gate.epoch,
            expected_target_nonce: config.target_nonce,
            expected_council_version: council.version,
            bridge_source_commitment: digest("spread-bridge-source"),
            bridge_build_inputs_commitment: digest("spread-bridge-build-inputs"),
            bridge_package_commitment: digest("spread-bridge-package"),
            bridge_release_manifest_commitment: digest("spread-bridge-release-manifest"),
        }
        .pack()
        .expect("create handoff payload"),
    );
    submit(context, &[create], &[&harness.seats[0]])
        .await
        .expect("create governed handoff");
    let draft: TargetAuthorityHandoffProposalV2 = state(context, harness.handoff_proposal).await;
    advance_to_slot(context, draft.review_start_slot)
        .await
        .expect("handoff review slot");
    for index in 0..3 {
        let mut accounts = handoff_review_accounts(harness, observation_key);
        accounts.push(sr(harness.seats[index].pubkey()));
        let approve = typed_instruction(
            harness.controller,
            accounts,
            ApproveTargetAuthorityHandoffProposalV2 {
                guard: GovernanceActionGuardV2 {
                    proposal_id: draft.proposal_id,
                    expected_proposal_digest: draft.proposal_digest,
                    expected_council_version: council.version,
                    expected_timing_profile_version: profile.profile_version,
                    expected_timing_profile_hash: profile.profile_hash,
                },
            }
            .pack()
            .expect("approve handoff payload"),
        );
        submit(context, &[approve], &[&harness.seats[index]])
            .await
            .expect("approve governed handoff");
    }
    let approved: TargetAuthorityHandoffProposalV2 = state(context, harness.handoff_proposal).await;
    assert_eq!(approved.state, GovernanceLifecycleStateV2::CouncilApproved);
    let queue = typed_instruction(
        harness.controller,
        handoff_review_accounts(harness, observation_key),
        QueueTargetAuthorityHandoffProposalV2 {
            guard: GovernanceActionGuardV2 {
                proposal_id: approved.proposal_id,
                expected_proposal_digest: approved.proposal_digest,
                expected_council_version: council.version,
                expected_timing_profile_version: profile.profile_version,
                expected_timing_profile_hash: profile.profile_hash,
            },
        }
        .pack()
        .expect("queue handoff payload"),
    );
    submit(context, &[queue], &[])
        .await
        .expect("queue governed handoff");
    let queued: TargetAuthorityHandoffProposalV2 = state(context, harness.handoff_proposal).await;
    advance_to_slot(context, queued.not_before_slot)
        .await
        .expect("handoff execution slot");
    let accept_payload = ExecuteTargetAuthorityHandoffProposalV2 {
        guard: GovernanceActionGuardV2 {
            proposal_id: queued.proposal_id,
            expected_proposal_digest: queued.proposal_digest,
            expected_council_version: council.version,
            expected_timing_profile_version: profile.profile_version,
            expected_timing_profile_hash: profile.profile_hash,
        },
        expected_bridge_observation_digest: observation.observation_digest,
        expected_gate_epoch: gate.epoch,
        expected_target_nonce: config.target_nonce,
        envelope: envelope(),
    };
    let accept = typed_instruction(
        harness.controller,
        vec![
            sw(context.payer.pubkey()),
            ro(harness.controller),
            ro(harness.controller_programdata),
            ro(harness.config),
            ro(harness.policy),
            ro(harness.council),
            ro(harness.gate),
            ro(harness.capacity_policy),
            ro(harness.immutability_receipt),
            ro(harness.lifecycle_registry),
            ro(harness.timing_profile),
            rw(harness.handoff_proposal),
            ro(observation_key),
            ro(harness.target),
            rw(harness.target_programdata),
            sr(harness.legacy_authority.pubkey()),
            ro(harness.authority),
            ro(UPGRADEABLE_LOADER_ID),
            rw(harness.handoff_receipt),
            ro(system_program::ID),
            ro(sysvar_ids::instructions::ID),
        ],
        accept_payload.pack().expect("accept handoff payload"),
    );
    let [limit, price] = envelope_prefix();
    submit(
        context,
        &[limit, price, accept],
        &[&harness.legacy_authority],
    )
    .await
    .expect("one exact checked Loader-v3 authority handoff");
    let receipt: TargetAuthorityHandoffReceiptV1 = state(context, harness.handoff_receipt).await;
    assert!(receipt.finalized);
    assert_eq!(receipt.post_upgrade_authority.value, harness.authority);
    receipt
}

fn activation_review_accounts(harness: &CeremonyHarness, observation: Pubkey) -> Vec<AccountMeta> {
    vec![
        ro(harness.controller),
        ro(harness.controller_programdata),
        ro(harness.config),
        ro(harness.policy),
        ro(harness.council),
        ro(harness.gate),
        ro(harness.capacity_policy),
        ro(harness.immutability_receipt),
        ro(harness.handoff_receipt),
        ro(observation),
        ro(harness.target),
        ro(harness.target_programdata),
        ro(harness.authority),
        ro(harness.lifecycle_registry),
        ro(harness.timing_profile),
        rw(harness.activation_proposal),
        ro(UPGRADEABLE_LOADER_ID),
    ]
}

#[allow(clippy::too_many_arguments)]
fn activation_expected_accounts(
    harness: &CeremonyHarness,
    config: &upgrade_controller::state::ControllerConfigV1,
    policy: &GovernancePolicyV1,
    council: &GovernanceCouncilSetV1,
    gate: &ProtocolGateV1,
    immutable: &ControllerImmutabilityReceiptV1,
    handoff: &TargetAuthorityHandoffReceiptV1,
    proposal: &BootstrapActivationProposalV2,
    observation_key: Pubkey,
    observation: &ProgramDataObservationV1,
    execution_slot: u64,
) -> (CurrentDeploymentStateV1, BootstrapActivationReceiptV1) {
    let next_epoch = gate.epoch + 1;
    let mut deployment = CurrentDeploymentStateV1 {
        discriminator: CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: derive_current_deployment_state_pda(&harness.controller, &harness.target).1,
        initialized: true,
        controller_program: harness.controller,
        controller_config: harness.config,
        capacity_policy: harness.capacity_policy,
        capacity_policy_digest: observation.capacity_policy_digest,
        target_program: harness.target,
        target_programdata: harness.target_programdata,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        controller_authority: harness.authority,
        artifact_length: handoff.artifact_length,
        artifact_sha256: handoff.artifact_sha256,
        artifact_merkle_root: handoff.artifact_merkle_root,
        artifact_scheme_id: handoff.artifact_scheme_id,
        actual_programdata_capacity: observation.actual_capacity,
        programdata_observation: observation_key,
        observation_generation: observation.generation,
        observation_root: observation.final_raw_merkle_root,
        observation_digest: observation.observation_digest,
        deployed_slot: observation.deployed_slot,
        installed_authority: harness.authority,
        source_commitment: handoff.bridge_source_commitment,
        build_inputs_commitment: handoff.bridge_build_inputs_commitment,
        package_commitment: handoff.bridge_package_commitment,
        release_manifest_commitment: handoff.bridge_release_manifest_commitment,
        release_commitment: harness.handoff_receipt,
        release_commitment_digest: handoff.receipt_digest,
        activation_receipt: OptionalPubkeyV1::some(harness.activation_receipt)
            .expect("activation receipt option"),
        completed_proposal: OptionalPubkeyV1::none(),
        gate_epoch_at_activation: next_epoch,
        deployment_generation: 1,
        deployment_digest: [0; 32],
        last_updated_slot: execution_slot,
        reserved: [0; CURRENT_DEPLOYMENT_STATE_V1_RESERVED_LEN],
    };
    deployment.deployment_digest =
        compute_current_deployment_digest_v1(&deployment).expect("deployment digest");
    let mut receipt = BootstrapActivationReceiptV1 {
        discriminator: BOOTSTRAP_ACTIVATION_RECEIPT_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: derive_bootstrap_activation_receipt_pda(&harness.controller, &harness.target).1,
        initialized: true,
        proposal: harness.activation_proposal,
        proposal_digest: proposal.proposal_digest,
        controller_program: harness.controller,
        controller_config: harness.config,
        governance_policy: harness.policy,
        governance_policy_hash: policy.policy_hash,
        capacity_policy: harness.capacity_policy,
        capacity_policy_digest: observation.capacity_policy_digest,
        controller_immutability_receipt: harness.immutability_receipt,
        controller_immutability_digest: immutable.receipt_digest,
        target_handoff_receipt: harness.handoff_receipt,
        target_handoff_digest: handoff.receipt_digest,
        gate: harness.gate,
        target_program: harness.target,
        target_programdata: harness.target_programdata,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        controller_authority: harness.authority,
        bridge_observation: observation_key,
        bridge_observation_generation: observation.generation,
        bridge_observation_root: observation.final_raw_merkle_root,
        bridge_observation_digest: observation.observation_digest,
        bridge_artifact_length: handoff.artifact_length,
        bridge_artifact_sha256: handoff.artifact_sha256,
        bridge_artifact_merkle_root: handoff.artifact_merkle_root,
        bridge_artifact_scheme_id: handoff.artifact_scheme_id,
        actual_target_capacity: observation.actual_capacity,
        target_deployed_slot: observation.deployed_slot,
        previous_gate_status: GateStatusV1::EmergencyFrozen,
        previous_gate_epoch: gate.epoch,
        previous_freeze_reason_code: gate.freeze_reason_code,
        previous_freeze_slot: gate.freeze_slot,
        activated_gate_status: GateStatusV1::Active,
        activated_gate_epoch: next_epoch,
        target_nonce: config.target_nonce,
        council_version: council.version,
        council_hash: council.set_hash,
        current_deployment_state: harness.deployment,
        current_deployment_digest: deployment.deployment_digest,
        deployment_generation: deployment.deployment_generation,
        finalized_slot: execution_slot,
        receipt_digest: [0; 32],
        finalized: true,
        reserved: [0; BOOTSTRAP_ACTIVATION_RECEIPT_V1_RESERVED_LEN],
    };
    receipt.receipt_digest = compute_bootstrap_activation_receipt_digest_v1(&receipt)
        .expect("activation receipt digest");
    (deployment, receipt)
}

async fn governed_activation(
    context: &mut CeremonyContext,
    harness: &CeremonyHarness,
    observation_key: Pubkey,
    observation: &ProgramDataObservationV1,
    immutable: &ControllerImmutabilityReceiptV1,
    handoff: &TargetAuthorityHandoffReceiptV1,
) -> (CurrentDeploymentStateV1, BootstrapActivationReceiptV1) {
    let config: upgrade_controller::state::ControllerConfigV1 =
        state(context, harness.config).await;
    let policy: GovernancePolicyV1 = state(context, harness.policy).await;
    let council: GovernanceCouncilSetV1 = state(context, harness.council).await;
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let profile: GovernanceTimingProfileV1 = state(context, harness.timing_profile).await;
    let create = typed_instruction(
        harness.controller,
        vec![
            sw(context.payer.pubkey()),
            sr(harness.seats[0].pubkey()),
            ro(harness.controller),
            ro(harness.controller_programdata),
            ro(harness.config),
            ro(harness.policy),
            ro(harness.council),
            ro(harness.gate),
            ro(harness.capacity_policy),
            ro(harness.immutability_receipt),
            ro(harness.handoff_receipt),
            ro(observation_key),
            ro(harness.target),
            ro(harness.target_programdata),
            ro(harness.authority),
            rw(harness.lifecycle_registry),
            ro(harness.timing_profile),
            rw(harness.activation_proposal),
            ro(UPGRADEABLE_LOADER_ID),
            ro(system_program::ID),
        ],
        CreateBootstrapActivationProposalV2 {
            expected_proposal_id: 2,
            expected_timing_profile_version: profile.profile_version,
            expected_timing_profile_hash: profile.profile_hash,
            expected_controller_immutability_digest: immutable.receipt_digest,
            expected_handoff_receipt_digest: handoff.receipt_digest,
            expected_bridge_observation_digest: observation.observation_digest,
            expected_gate_epoch: gate.epoch,
            expected_target_nonce: config.target_nonce,
            expected_council_version: council.version,
        }
        .pack()
        .expect("create activation payload"),
    );
    submit(context, &[create], &[&harness.seats[0]])
        .await
        .expect("create governed bootstrap activation");
    let draft: BootstrapActivationProposalV2 = state(context, harness.activation_proposal).await;
    advance_to_slot(context, draft.review_start_slot)
        .await
        .expect("activation review slot");
    for index in 0..3 {
        let mut accounts = activation_review_accounts(harness, observation_key);
        accounts.push(sr(harness.seats[index].pubkey()));
        let approve = typed_instruction(
            harness.controller,
            accounts,
            ApproveBootstrapActivationProposalV2 {
                guard: GovernanceActionGuardV2 {
                    proposal_id: draft.proposal_id,
                    expected_proposal_digest: draft.proposal_digest,
                    expected_council_version: council.version,
                    expected_timing_profile_version: profile.profile_version,
                    expected_timing_profile_hash: profile.profile_hash,
                },
            }
            .pack()
            .expect("approve activation payload"),
        );
        submit(context, &[approve], &[&harness.seats[index]])
            .await
            .expect("approve bootstrap activation");
    }
    let approved: BootstrapActivationProposalV2 = state(context, harness.activation_proposal).await;
    let queue = typed_instruction(
        harness.controller,
        activation_review_accounts(harness, observation_key),
        QueueBootstrapActivationProposalV2 {
            guard: GovernanceActionGuardV2 {
                proposal_id: approved.proposal_id,
                expected_proposal_digest: approved.proposal_digest,
                expected_council_version: council.version,
                expected_timing_profile_version: profile.profile_version,
                expected_timing_profile_hash: profile.profile_hash,
            },
        }
        .pack()
        .expect("queue activation payload"),
    );
    submit(context, &[queue], &[])
        .await
        .expect("queue bootstrap activation");
    let queued: BootstrapActivationProposalV2 = state(context, harness.activation_proposal).await;
    advance_to_slot(context, queued.not_before_slot)
        .await
        .expect("activation execution slot");
    let (expected_deployment, expected_receipt) = activation_expected_accounts(
        harness,
        &config,
        &policy,
        &council,
        &gate,
        immutable,
        handoff,
        &queued,
        observation_key,
        observation,
        queued.not_before_slot,
    );
    let execute = typed_instruction(
        harness.controller,
        vec![
            sw(context.payer.pubkey()),
            ro(harness.controller),
            ro(harness.controller_programdata),
            ro(harness.config),
            ro(harness.policy),
            ro(harness.council),
            rw(harness.gate),
            ro(harness.capacity_policy),
            ro(harness.immutability_receipt),
            ro(harness.handoff_receipt),
            ro(harness.lifecycle_registry),
            ro(harness.timing_profile),
            rw(harness.activation_proposal),
            ro(observation_key),
            ro(harness.target),
            ro(harness.target_programdata),
            ro(harness.authority),
            ro(UPGRADEABLE_LOADER_ID),
            rw(harness.activation_receipt),
            rw(harness.deployment),
            ro(system_program::ID),
            ro(sysvar_ids::instructions::ID),
        ],
        ExecuteBootstrapActivationProposalV2 {
            guard: GovernanceActionGuardV2 {
                proposal_id: queued.proposal_id,
                expected_proposal_digest: queued.proposal_digest,
                expected_council_version: council.version,
                expected_timing_profile_version: profile.profile_version,
                expected_timing_profile_hash: profile.profile_hash,
            },
            expected_bridge_observation_digest: observation.observation_digest,
            expected_gate_epoch: gate.epoch,
            expected_target_nonce: config.target_nonce,
            expected_deployment_plan_digest:
                compute_bootstrap_activation_deployment_plan_digest_v1(&expected_deployment)
                    .expect("activation deployment plan digest"),
            expected_receipt_plan_digest: compute_bootstrap_activation_receipt_plan_digest_v1(
                &expected_receipt,
            )
            .expect("activation receipt plan digest"),
            envelope: envelope(),
        }
        .pack()
        .expect("execute activation payload"),
    );
    let [limit, price] = envelope_prefix();
    submit(context, &[limit, price, execute], &[])
        .await
        .expect("execute governed bootstrap activation");
    let deployment: CurrentDeploymentStateV1 = state(context, harness.deployment).await;
    let receipt: BootstrapActivationReceiptV1 = state(context, harness.activation_receipt).await;
    assert_eq!(
        compute_bootstrap_activation_deployment_plan_digest_v1(&deployment)
            .expect("actual activation deployment plan digest"),
        compute_bootstrap_activation_deployment_plan_digest_v1(&expected_deployment)
            .expect("expected activation deployment plan digest")
    );
    assert_eq!(
        compute_bootstrap_activation_receipt_plan_digest_v1(&receipt)
            .expect("actual activation receipt plan digest"),
        compute_bootstrap_activation_receipt_plan_digest_v1(&expected_receipt)
            .expect("expected activation receipt plan digest")
    );
    assert_eq!(
        compute_current_deployment_digest_v1(&deployment)
            .expect("actual current deployment digest"),
        deployment.deployment_digest
    );
    assert_eq!(
        compute_bootstrap_activation_receipt_digest_v1(&receipt)
            .expect("actual activation receipt digest"),
        receipt.receipt_digest
    );
    assert_eq!(deployment.last_updated_slot, receipt.finalized_slot);
    assert!(deployment.last_updated_slot >= queued.not_before_slot);
    (deployment, receipt)
}

async fn current_slot(context: &mut CeremonyContext) -> u64 {
    match &mut context.backend {
        CeremonyBackend::ProgramTest(context) => {
            context
                .banks_client
                .get_sysvar::<solana_sdk::clock::Clock>()
                .await
                .expect("clock sysvar")
                .slot
        }
        CeremonyBackend::Rpc(backend) => backend
            .client
            .get_slot_with_commitment(CommitmentConfig::confirmed())
            .await
            .expect("finalized RPC slot"),
    }
}

fn proposal_guard(
    proposal: &UpgradeProposalV3,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    capacity: &ProgramDataCapacityPolicyV1,
    deployment: &CurrentDeploymentStateV1,
) -> ProposalGuardV3 {
    ProposalGuardV3 {
        expected_proposal_digest: proposal.proposal_digest,
        expected_state: proposal.state,
        expected_gate_status: gate.status,
        expected_gate_epoch: gate.epoch,
        expected_target_nonce: config.target_nonce,
        expected_capacity_policy_digest: capacity.policy_digest,
        expected_current_deployment_digest: deployment.deployment_digest,
        expected_current_deployment_generation: deployment.deployment_generation,
    }
}

#[allow(clippy::too_many_arguments)]
async fn create_upgrade_proposal(
    context: &mut CeremonyContext,
    harness: &CeremonyHarness,
    creator_index: usize,
    proposal_key: Pubkey,
    proposal_class: ProposalClassV1,
    buffer: Pubkey,
    uploader: Pubkey,
    artifact: &[u8],
    primary_proposal: OptionalPubkeyV1,
    rollback_proposal: OptionalPubkeyV1,
    rollback_buffer: OptionalPubkeyV1,
    rollback_artifact: Option<&[u8]>,
) -> UpgradeProposalV3 {
    let config: ControllerConfigV1 = state(context, harness.config).await;
    let policy: GovernancePolicyV1 = state(context, harness.policy).await;
    let council: GovernanceCouncilSetV1 = state(context, harness.council).await;
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let capacity: ProgramDataCapacityPolicyV1 = state(context, harness.capacity_policy).await;
    let deployment: CurrentDeploymentStateV1 = state(context, harness.deployment).await;
    let rollback_identity = rollback_artifact.map(|value| {
        (
            value.len() as u64,
            hashv(&[value]).to_bytes(),
            artifact_merkle_root(value, ARTIFACT_BINDING_CHUNK_SIZE_V1)
                .expect("rollback artifact root"),
        )
    });
    let instruction = create_proposal_v3_instruction(
        harness.controller,
        CreateProposalV3Accounts {
            payer: context.payer.pubkey(),
            creator_seat_authority: harness.seats[creator_index].pubkey(),
            controller_config: harness.config,
            policy: harness.policy,
            council: harness.council,
            protocol_gate: harness.gate,
            capacity_policy: harness.capacity_policy,
            current_deployment: harness.deployment,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            authority_pda: harness.authority,
            canonical_spill_treasury: harness.spill,
            buffer,
            buffer_uploader_authority: uploader,
            proposal: proposal_key,
            system_program: system_program::ID,
        },
        CreateProposalV3 {
            manifest: ProposalManifestV3 {
                proposal_class,
                expected_proposal_id: config.next_proposal_id,
                expected_target_nonce: config.target_nonce,
                expected_gate_status: gate.status,
                expected_gate_epoch: gate.epoch,
                expected_capacity_policy_digest: capacity.policy_digest,
                expected_current_deployment_digest: deployment.deployment_digest,
                expected_current_deployment_generation: deployment.deployment_generation,
                expected_policy_version: policy.version,
                expected_policy_hash: policy.policy_hash,
                expected_council_version: council.version,
                expected_council_hash: council.set_hash,
                artifact_length: artifact.len() as u64,
                artifact_sha256: hashv(&[artifact]).to_bytes(),
                artifact_chunk_merkle_root: artifact_merkle_root(
                    artifact,
                    ARTIFACT_BINDING_CHUNK_SIZE_V1,
                )
                .expect("proposal artifact root"),
                source_commit_hash: digest("ceremony-proposal-source"),
                source_tree_hash: digest("ceremony-proposal-tree"),
                build_input_inventory_hash: digest("ceremony-proposal-build-inputs"),
                reproducible_build_receipt_hash: digest("ceremony-proposal-build-receipt"),
                package_receipt_hash: digest("ceremony-proposal-package"),
                release_intent_hash: digest("ceremony-proposal-release-intent"),
                minimum_required_capacity: artifact.len() as u64,
                checkpoint_schema_id: digest("ceremony-checkpoint-schema-v1"),
                checkpoint_policy_hash: digest("ceremony-checkpoint-policy-v1"),
                primary_proposal,
                rollback_proposal,
                rollback_buffer,
                rollback_artifact_length: rollback_identity.map_or(0, |value| value.0),
                rollback_artifact_sha256: rollback_identity.map_or([0; 32], |value| value.1),
                rollback_artifact_chunk_root: rollback_identity.map_or([0; 32], |value| value.2),
                plan_valid_until_slot: config
                    .proposal_expiry_slots
                    .checked_add(current_slot(context).await)
                    .expect("proposal plan horizon"),
            },
        },
    )
    .expect("create V3 proposal instruction");
    submit(context, &[instruction], &[&harness.seats[creator_index]])
        .await
        .expect("create V3 proposal through actual controller SBF");
    state(context, proposal_key).await
}

async fn seal_upgrade_buffer(
    context: &mut CeremonyContext,
    harness: &CeremonyHarness,
    proposal_key: Pubkey,
    buffer: Pubkey,
    uploader: &Keypair,
    artifact: &[u8],
) -> (UpgradeProposalV3, BufferVerificationV1) {
    let config: ControllerConfigV1 = state(context, harness.config).await;
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let capacity: ProgramDataCapacityPolicyV1 = state(context, harness.capacity_policy).await;
    let deployment: CurrentDeploymentStateV1 = state(context, harness.deployment).await;
    let proposal: UpgradeProposalV3 = state(context, proposal_key).await;
    let verification_key = derive_buffer_check_pda(&harness.controller, &proposal_key).0;
    let adopt = adopt_buffer_v2_instruction(
        harness.controller,
        AdoptBufferV2Accounts {
            payer: context.payer.pubkey(),
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: proposal_key,
            capacity_policy: harness.capacity_policy,
            current_deployment: harness.deployment,
            buffer,
            uploader_authority: uploader.pubkey(),
            authority_pda: harness.authority,
            buffer_verification: verification_key,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            system_program: system_program::ID,
        },
        AdoptBufferV2 {
            expected: proposal_guard(&proposal, &config, &gate, &capacity, &deployment),
        },
    )
    .expect("adopt buffer instruction");
    submit(context, &[adopt], &[uploader])
        .await
        .expect("adopt exact Loader buffer through actual controller SBF");

    let chunk_count = artifact_chunk_count(artifact.len() as u64, ARTIFACT_BINDING_CHUNK_SIZE_V1)
        .expect("artifact chunk count");
    for chunk_index in 0..chunk_count {
        let proposal: UpgradeProposalV3 = state(context, proposal_key).await;
        let verification: BufferVerificationV1 = state(context, verification_key).await;
        let proof = artifact_merkle_proof(artifact, ARTIFACT_BINDING_CHUNK_SIZE_V1, chunk_index)
            .expect("artifact proof");
        let mut nodes = [[0; 32]; upgrade_controller::instruction::MAX_FIXED_MERKLE_PROOF_NODES_V1];
        nodes[..proof.len()].copy_from_slice(&proof);
        let verify = verify_buffer_chunk_v2_instruction(
            harness.controller,
            VerifyBufferChunkV2Accounts {
                controller_config: harness.config,
                protocol_gate: harness.gate,
                proposal: proposal_key,
                buffer,
                buffer_verification: verification_key,
                authority_pda: harness.authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
            },
            VerifyBufferChunkV2 {
                expected: proposal_guard(&proposal, &config, &gate, &capacity, &deployment),
                chunk_index,
                proof: ArtifactChunkProofV2 {
                    proof_len: proof.len() as u8,
                    nodes,
                },
                expected_verification_status: verification.status,
                expected_verified_chunk_bitmap: verification.verified_chunk_bitmap,
                expected_verified_chunk_count: verification.verified_chunk_count,
            },
        )
        .expect("verify buffer chunk instruction");
        submit(context, &[verify], &[])
            .await
            .expect("verify exact sealed buffer chunk through actual controller SBF");
    }

    let proposal: UpgradeProposalV3 = state(context, proposal_key).await;
    let ready: BufferVerificationV1 = state(context, verification_key).await;
    assert_eq!(ready.status, BufferVerificationStatusV1::ReadyToFinalize);
    assert_eq!(ready.verified_chunk_count, chunk_count);
    let buffer_data = bytes(context, buffer).await;
    let sealed_header_hash = hashv(&[&buffer_data[..LOADER_BUFFER_METADATA_LEN]]).to_bytes();
    let finalize = finalize_buffer_verification_v2_instruction(
        harness.controller,
        FinalizeBufferVerificationV2Accounts {
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: proposal_key,
            buffer,
            buffer_verification: verification_key,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
        },
        FinalizeBufferVerificationV2 {
            expected: proposal_guard(&proposal, &config, &gate, &capacity, &deployment),
            expected_verification_status: ready.status,
            expected_verified_chunk_bitmap: ready.verified_chunk_bitmap,
            expected_verified_chunk_count: ready.verified_chunk_count,
            expected_sealed_buffer_header_hash: sealed_header_hash,
        },
    )
    .expect("finalize buffer verification instruction");
    submit(context, &[finalize], &[])
        .await
        .expect("finalize sealed buffer verification through actual controller SBF");
    (
        state(context, proposal_key).await,
        state(context, verification_key).await,
    )
}

async fn approve_and_queue_v3(
    context: &mut CeremonyContext,
    harness: &CeremonyHarness,
    proposal_key: Pubkey,
) -> UpgradeProposalV3 {
    let proposal: UpgradeProposalV3 = state(context, proposal_key).await;
    let slot = current_slot(context).await;
    println!(
        "proposal review diagnostics: proposal={proposal_key} current_slot={slot} creation_slot={} review_start_slot={} review_end_slot={} expiry_slot={}",
        proposal.creation_slot,
        proposal.review_start_slot,
        proposal.review_end_slot,
        proposal.expiry_slot,
    );
    if slot < proposal.review_start_slot {
        advance_to_slot(context, proposal.review_start_slot)
            .await
            .expect("proposal review start");
    }
    for index in 0..3 {
        let config: ControllerConfigV1 = state(context, harness.config).await;
        let gate: ProtocolGateV1 = state(context, harness.gate).await;
        let capacity: ProgramDataCapacityPolicyV1 = state(context, harness.capacity_policy).await;
        let deployment: CurrentDeploymentStateV1 = state(context, harness.deployment).await;
        let proposal: UpgradeProposalV3 = state(context, proposal_key).await;
        let approve = approve_proposal_v3_instruction(
            harness.controller,
            ApproveProposalV3Accounts {
                controller_config: harness.config,
                policy: harness.policy,
                creation_council: harness.council,
                protocol_gate: harness.gate,
                capacity_policy: harness.capacity_policy,
                current_deployment: harness.deployment,
                proposal: proposal_key,
                buffer_verification: proposal.buffer_verification,
                seat_authority: harness.seats[index].pubkey(),
            },
            ApproveProposalV3 {
                expected: proposal_guard(&proposal, &config, &gate, &capacity, &deployment),
                expected_creation_council_version: proposal.creation_council_version,
                expected_creation_council_hash: proposal.creation_council_hash,
                expected_approval_bitset: proposal.council_approval_bitset,
                expected_approval_count: proposal.council_approval_count,
            },
        )
        .expect("approve proposal instruction");
        submit(context, &[approve], &[&harness.seats[index]])
            .await
            .expect("approve proposal through actual controller SBF");
    }

    let config: ControllerConfigV1 = state(context, harness.config).await;
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let capacity: ProgramDataCapacityPolicyV1 = state(context, harness.capacity_policy).await;
    let deployment: CurrentDeploymentStateV1 = state(context, harness.deployment).await;
    let approved: UpgradeProposalV3 = state(context, proposal_key).await;
    assert_eq!(approved.state, ProposalStateV2::CouncilApproved);
    let finalize = finalize_governance_v3_instruction(
        harness.controller,
        FinalizeGovernanceV3Accounts {
            controller_config: harness.config,
            policy: harness.policy,
            creation_council: harness.council,
            protocol_gate: harness.gate,
            capacity_policy: harness.capacity_policy,
            current_deployment: harness.deployment,
            proposal: proposal_key,
        },
        FinalizeGovernanceV3 {
            expected: proposal_guard(&approved, &config, &gate, &capacity, &deployment),
            expected_approval_bitset: approved.council_approval_bitset,
            expected_approval_count: approved.council_approval_count,
        },
    )
    .expect("finalize governance instruction");
    submit(context, &[finalize], &[])
        .await
        .expect("finalize proposal governance through actual controller SBF");

    let satisfied: UpgradeProposalV3 = state(context, proposal_key).await;
    let queue = queue_proposal_v3_instruction(
        harness.controller,
        QueueProposalV3Accounts {
            controller_config: harness.config,
            policy: harness.policy,
            protocol_gate: harness.gate,
            capacity_policy: harness.capacity_policy,
            current_deployment: harness.deployment,
            proposal: proposal_key,
        },
        QueueProposalV3 {
            expected: proposal_guard(&satisfied, &config, &gate, &capacity, &deployment),
        },
    )
    .expect("queue proposal instruction");
    submit(context, &[queue], &[])
        .await
        .expect("queue proposal through actual controller SBF");
    state(context, proposal_key).await
}

fn checkpoint_hard_root(manifest: &CheckpointManifestV2) -> [u8; 32] {
    hashv(&[
        STATE_CHECKPOINT_HARD_ROOT_DOMAIN_V1,
        &manifest.schema_identifier,
        &manifest.program_owned_state_root,
        &manifest.program_owned_state_count.to_le_bytes(),
        &manifest.logical_compressed_state_root,
        &manifest.logical_compressed_state_count.to_le_bytes(),
        &manifest.semantic_custody_accounting_root,
        &manifest.external_metadata_observation_root,
    ])
    .to_bytes()
}

#[allow(clippy::too_many_arguments)]
fn checkpoint_candidate(
    harness: &CeremonyHarness,
    proposal_key: Pubkey,
    proposal: &UpgradeProposalV3,
    phase: StateCheckpointPhaseV1,
    checkpoint_key: Pubkey,
    checkpoint_bump: u8,
    manifest: &CheckpointManifestV2,
    observation_key: Pubkey,
    observation: &ProgramDataObservationV1,
    capacity: &ProgramDataCapacityPolicyV1,
    deployment: &CurrentDeploymentStateV1,
    council: &GovernanceCouncilSetV1,
) -> StateCheckpointV2 {
    let (artifact_length, artifact_sha256, artifact_merkle_root, minimum_required_capacity) =
        match phase {
            StateCheckpointPhaseV1::Prestate => (
                observation.expected_artifact_length,
                observation.expected_artifact_sha256,
                observation.expected_artifact_merkle_root,
                observation.minimum_required_capacity,
            ),
            StateCheckpointPhaseV1::Poststate => (
                proposal.artifact_length,
                proposal.artifact_sha256,
                proposal.artifact_chunk_merkle_root,
                proposal.minimum_required_capacity,
            ),
            StateCheckpointPhaseV1::Emergency => unreachable!("proposal checkpoint helper"),
        };
    let mut candidate = StateCheckpointV2 {
        discriminator: STATE_CHECKPOINT_V2_DISCRIMINATOR,
        account_version: CAPACITY_SAFE_ACCOUNT_VERSION_V2,
        bump: checkpoint_bump,
        initialized: true,
        phase,
        controller_config: harness.config,
        proposal: proposal_key,
        emergency_resolution: Pubkey::default(),
        subject_digest: proposal.proposal_digest,
        target_program: harness.target,
        target_programdata: harness.target_programdata,
        capacity_policy: harness.capacity_policy,
        capacity_policy_digest: capacity.policy_digest,
        current_deployment_state: harness.deployment,
        current_deployment_digest: deployment.deployment_digest,
        current_deployment_generation: deployment.deployment_generation,
        checkpoint_generation: manifest.checkpoint_generation,
        previous_checkpoint_digest: manifest.previous_checkpoint_digest,
        observation_scheme_id: capacity.observation_scheme_id,
        programdata_observation: observation_key,
        observation_purpose: observation.purpose,
        observation_generation: observation.generation,
        observation_subject_digest: observation.subject_digest,
        observation_root: observation.final_raw_merkle_root,
        observation_digest: observation.observation_digest,
        observation_finalized_slot: observation.finalized_slot,
        gate_epoch: manifest.expected_gate_epoch,
        target_programdata_slot: observation.deployed_slot,
        artifact_length,
        artifact_sha256,
        artifact_merkle_root,
        artifact_scheme_id: capacity.artifact_scheme_id,
        minimum_required_capacity,
        observed_raw_data_length: observation.raw_data_length,
        actual_capacity: observation.actual_capacity,
        observed_authority: observation.upgrade_authority,
        program_owned_state_root: manifest.program_owned_state_root,
        program_owned_state_count: manifest.program_owned_state_count,
        logical_compressed_state_root: manifest.logical_compressed_state_root,
        logical_compressed_state_count: manifest.logical_compressed_state_count,
        semantic_custody_accounting_root: manifest.semantic_custody_accounting_root,
        hard_combined_root: manifest.hard_combined_root,
        external_metadata_observation_root: manifest.external_metadata_observation_root,
        external_raw_balance_observation_root: manifest.external_raw_balance_observation_root,
        schema_identifier: manifest.schema_identifier,
        admitted_positive_donation_root: manifest.admitted_positive_donation_root,
        admitted_positive_donation_count: manifest.admitted_positive_donation_count,
        forbidden_drift_count: manifest.forbidden_drift_count,
        approval_council_version: council.version,
        approval_council_hash: council.set_hash,
        checkpoint_digest_domain_id: STATE_CHECKPOINT_V2_DIGEST_DOMAIN_ID,
        checkpoint_digest: [0; 32],
        approval_bitset: 0,
        approval_count: 0,
        accepted: false,
        finalized_slot: observation.finalized_slot,
        reserved: [0; STATE_CHECKPOINT_V2_RESERVED_LEN],
    };
    assert_eq!(
        checkpoint_key,
        derive_checkpoint_pda(
            &harness.controller,
            &proposal_key,
            match phase {
                StateCheckpointPhaseV1::Prestate => CheckpointPhaseV1::Prestate,
                StateCheckpointPhaseV1::Poststate => CheckpointPhaseV1::Poststate,
                StateCheckpointPhaseV1::Emergency => unreachable!(),
            },
        )
        .0
    );
    candidate.checkpoint_digest =
        compute_state_checkpoint_digest_v2(&candidate).expect("checkpoint digest");
    candidate
}

#[allow(clippy::too_many_arguments)]
async fn attest_and_finalize_checkpoint_v3(
    context: &mut CeremonyContext,
    harness: &CeremonyHarness,
    proposal_key: Pubkey,
    phase: StateCheckpointPhaseV1,
    observation_key: Pubkey,
    observation: &ProgramDataObservationV1,
    roots_label: &str,
) -> StateCheckpointV2 {
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let capacity: ProgramDataCapacityPolicyV1 = state(context, harness.capacity_policy).await;
    let deployment: CurrentDeploymentStateV1 = state(context, harness.deployment).await;
    let council: GovernanceCouncilSetV1 = state(context, harness.council).await;
    let proposal: UpgradeProposalV3 = state(context, proposal_key).await;
    let checkpoint_phase = match phase {
        StateCheckpointPhaseV1::Prestate => CheckpointPhaseV1::Prestate,
        StateCheckpointPhaseV1::Poststate => CheckpointPhaseV1::Poststate,
        StateCheckpointPhaseV1::Emergency => unreachable!("proposal checkpoint helper"),
    };
    let (checkpoint_key, checkpoint_bump) =
        derive_checkpoint_pda(&harness.controller, &proposal_key, checkpoint_phase);
    let root = |suffix: &str| digest(&format!("{roots_label}-{suffix}"));
    let mut manifest = CheckpointManifestV2 {
        phase,
        checkpoint_generation: 1,
        previous_checkpoint_digest: [0; 32],
        expected_subject_digest: proposal.proposal_digest,
        expected_gate_epoch: gate.epoch,
        expected_capacity_policy_digest: capacity.policy_digest,
        expected_current_deployment_digest: deployment.deployment_digest,
        expected_current_deployment_generation: deployment.deployment_generation,
        expected_observation_digest: observation.observation_digest,
        expected_observation_generation: observation.generation,
        program_owned_state_root: root("program-owned"),
        program_owned_state_count: 1,
        logical_compressed_state_root: root("compressed"),
        logical_compressed_state_count: 1,
        semantic_custody_accounting_root: root("semantic-custody"),
        hard_combined_root: [0; 32],
        external_metadata_observation_root: root("external-metadata"),
        external_raw_balance_observation_root: root("external-balance"),
        schema_identifier: proposal.checkpoint_schema_id,
        admitted_positive_donation_root: [0; 32],
        admitted_positive_donation_count: 0,
        forbidden_drift_count: 0,
        expected_council_version: council.version,
        expected_council_hash: council.set_hash,
        expected_checkpoint_digest: [0; 32],
        plan_valid_until_slot: proposal.expiry_slot,
    };
    manifest.hard_combined_root = checkpoint_hard_root(&manifest);
    let candidate = checkpoint_candidate(
        harness,
        proposal_key,
        &proposal,
        phase,
        checkpoint_key,
        checkpoint_bump,
        &manifest,
        observation_key,
        observation,
        &capacity,
        &deployment,
        &council,
    );
    manifest.expected_checkpoint_digest = candidate.checkpoint_digest;

    let linked = harness.authority;
    let attestations: [Pubkey; 3] = std::array::from_fn(|index| {
        derive_checkpoint_attestation_pda(
            &harness.controller,
            &checkpoint_key,
            council.version,
            index as u8,
        )
        .0
    });
    for (index, attestation) in attestations.iter().enumerate() {
        let create = create_checkpoint_v2_instruction(
            harness.controller,
            CreateCheckpointV2Accounts {
                payer: context.payer.pubkey(),
                controller_config: harness.config,
                policy: harness.policy,
                current_council: harness.council,
                protocol_gate: harness.gate,
                subject: proposal_key,
                linked_primary_or_authority: linked,
                capacity_policy: harness.capacity_policy,
                current_deployment: harness.deployment,
                programdata_observation: observation_key,
                target_program: harness.target,
                target_programdata: harness.target_programdata,
                checkpoint: checkpoint_key,
                checkpoint_attestation: *attestation,
                seat_authority: harness.seats[index].pubkey(),
                system_program: system_program::ID,
            },
            CreateCheckpointV2 {
                attestation: CheckpointAttestationGuardV2 {
                    manifest: manifest.clone(),
                    seat_index: index as u8,
                    expected_previous_attestation_digest: [0; 32],
                },
            },
        )
        .expect("checkpoint attestation instruction");
        submit(context, &[create], &[&harness.seats[index]])
            .await
            .expect("create checkpoint attestation through actual controller SBF");
    }
    for key in attestations {
        let attestation: CheckpointAttestationV1 = state(context, key).await;
        assert_eq!(attestation.checkpoint_digest, candidate.checkpoint_digest);
    }

    let finalize = finalize_checkpoint_v2_instruction(
        harness.controller,
        FinalizeCheckpointV2Accounts {
            payer: context.payer.pubkey(),
            controller_config: harness.config,
            policy: harness.policy,
            current_council: harness.council,
            protocol_gate: harness.gate,
            subject: proposal_key,
            linked_primary_or_authority: linked,
            capacity_policy: harness.capacity_policy,
            current_deployment: harness.deployment,
            programdata_observation: observation_key,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            checkpoint: checkpoint_key,
            checkpoint_attestations: attestations,
            system_program: system_program::ID,
        },
        FinalizeCheckpointV2 { manifest },
    )
    .expect("finalize checkpoint instruction");
    let [limit, price] = envelope_prefix();
    submit(context, &[limit, price, finalize], &[])
        .await
        .expect("finalize accepted checkpoint through actual controller SBF");
    let accepted: StateCheckpointV2 = state(context, checkpoint_key).await;
    assert!(accepted.accepted);
    assert_eq!(accepted.approval_count, 3);
    assert_eq!(accepted.checkpoint_digest, candidate.checkpoint_digest);
    accepted
}

async fn bind_and_finalize_programdata_v3(
    context: &mut CeremonyContext,
    harness: &CeremonyHarness,
    proposal_key: Pubkey,
    observation_key: Pubkey,
    observation: &ProgramDataObservationV1,
) -> (UpgradeProposalV3, ProgramDataVerificationV2) {
    let config: ControllerConfigV1 = state(context, harness.config).await;
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let capacity: ProgramDataCapacityPolicyV1 = state(context, harness.capacity_policy).await;
    let deployment: CurrentDeploymentStateV1 = state(context, harness.deployment).await;
    let proposal: UpgradeProposalV3 = state(context, proposal_key).await;
    let verification_key = derive_programdata_check_pda(&harness.controller, &proposal_key).0;
    let bind = bind_programdata_verification_v2_instruction(
        harness.controller,
        BindProgramDataVerificationV2Accounts {
            payer: context.payer.pubkey(),
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: proposal_key,
            capacity_policy: harness.capacity_policy,
            current_deployment: harness.deployment,
            programdata_observation: observation_key,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            programdata_verification: verification_key,
            system_program: system_program::ID,
        },
        BindProgramDataVerificationV2 {
            manifest: ProgramDataVerificationManifestV2 {
                expected: proposal_guard(&proposal, &config, &gate, &capacity, &deployment),
                expected_observation_digest: observation.observation_digest,
                expected_observation_generation: observation.generation,
                expected_observation_root: observation.final_raw_merkle_root,
                expected_observation_finalized_slot: observation.finalized_slot,
                verification_generation: 1,
                previous_verification_digest: [0; 32],
                plan_valid_until_slot: proposal.expiry_slot,
            },
        },
    )
    .expect("bind ProgramData verification instruction");
    submit(context, &[bind], &[])
        .await
        .expect("bind ProgramData verification through actual controller SBF");

    let bound: ProgramDataVerificationV2 = state(context, verification_key).await;
    assert_eq!(
        bound.status,
        ProgramDataVerificationStatusV2::ObservationBound
    );
    let finalize = finalize_programdata_verification_v2_instruction(
        harness.controller,
        FinalizeProgramDataVerificationV2Accounts {
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: proposal_key,
            capacity_policy: harness.capacity_policy,
            current_deployment: harness.deployment,
            programdata_observation: observation_key,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            programdata_verification: verification_key,
        },
        FinalizeProgramDataVerificationV2 {
            expected: ProgramDataVerificationGuardV2 {
                expected_proposal_digest: proposal.proposal_digest,
                expected_verification_digest: bound.verification_digest,
                expected_verification_generation: bound.verification_generation,
                expected_status: bound.status,
                expected_gate_epoch: gate.epoch,
                expected_target_nonce: config.target_nonce,
                expected_capacity_policy_digest: capacity.policy_digest,
                expected_current_deployment_digest: deployment.deployment_digest,
                expected_current_deployment_generation: deployment.deployment_generation,
                expected_observation_digest: observation.observation_digest,
                expected_observation_generation: observation.generation,
                expected_actual_capacity: observation.actual_capacity,
                expected_authority: harness.authority,
            },
        },
    )
    .expect("finalize ProgramData verification instruction");
    submit(context, &[finalize], &[])
        .await
        .expect("finalize exact ProgramData verification through actual controller SBF");
    (
        state(context, proposal_key).await,
        state(context, verification_key).await,
    )
}

fn unfreeze_guard(
    proposal: &UpgradeProposalV3,
    checkpoint: &StateCheckpointV2,
    verification: &ProgramDataVerificationV2,
    council: &GovernanceCouncilSetV1,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    deployment: &CurrentDeploymentStateV1,
) -> UnfreezeGuardV2 {
    UnfreezeGuardV2 {
        expected_proposal_digest: proposal.proposal_digest,
        expected_checkpoint_digest: checkpoint.checkpoint_digest,
        expected_checkpoint_generation: checkpoint.checkpoint_generation,
        expected_verification_digest: verification.verification_digest,
        expected_verification_generation: verification.verification_generation,
        expected_original_council_version: proposal.creation_council_version,
        expected_original_council_hash: proposal.creation_council_hash,
        expected_current_council_version: council.version,
        expected_current_council_hash: council.set_hash,
        expected_gate_epoch: gate.epoch,
        expected_target_nonce: config.target_nonce,
        expected_current_deployment_digest: deployment.deployment_digest,
        expected_current_deployment_generation: deployment.deployment_generation,
        expected_artifact_sha256: proposal.artifact_sha256,
        expected_artifact_merkle_root: proposal.artifact_chunk_merkle_root,
        expected_actual_capacity: verification.actual_capacity,
        expected_approval_bitset: proposal.unfreeze_approval_bitset,
        expected_approval_count: proposal.unfreeze_approval_count,
    }
}

async fn approve_and_execute_unfreeze_v3(
    context: &mut CeremonyContext,
    harness: &CeremonyHarness,
    proposal_key: Pubkey,
    linked_rollback: Pubkey,
    checkpoint: &StateCheckpointV2,
    verification: &ProgramDataVerificationV2,
) -> (UpgradeProposalV3, CurrentDeploymentStateV1, ProtocolGateV1) {
    for index in 0..3 {
        let config: ControllerConfigV1 = state(context, harness.config).await;
        let gate: ProtocolGateV1 = state(context, harness.gate).await;
        let council: GovernanceCouncilSetV1 = state(context, harness.council).await;
        let deployment: CurrentDeploymentStateV1 = state(context, harness.deployment).await;
        let proposal: UpgradeProposalV3 = state(context, proposal_key).await;
        let approve = approve_unfreeze_v2_instruction(
            harness.controller,
            ApproveUnfreezeV2Accounts {
                controller_config: harness.config,
                policy: harness.policy,
                current_council: harness.council,
                protocol_gate: harness.gate,
                proposal: proposal_key,
                poststate_checkpoint: proposal.required_poststate_checkpoint,
                programdata_verification: proposal.programdata_verification,
                capacity_policy: harness.capacity_policy,
                current_deployment: harness.deployment,
                target_program: harness.target,
                target_programdata: harness.target_programdata,
                authority_pda: harness.authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
                seat_authority: harness.seats[index].pubkey(),
            },
            ApproveUnfreezeV2 {
                expected: unfreeze_guard(
                    &proposal,
                    checkpoint,
                    verification,
                    &council,
                    &config,
                    &gate,
                    &deployment,
                ),
            },
        )
        .expect("approve unfreeze instruction");
        submit(context, &[approve], &[&harness.seats[index]])
            .await
            .expect("approve separate unfreeze through actual controller SBF");
    }

    let config: ControllerConfigV1 = state(context, harness.config).await;
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let council: GovernanceCouncilSetV1 = state(context, harness.council).await;
    let deployment: CurrentDeploymentStateV1 = state(context, harness.deployment).await;
    let approved: UpgradeProposalV3 = state(context, proposal_key).await;
    assert_eq!(approved.state, ProposalStateV2::UnfreezeApproved);
    let execute = execute_unfreeze_v2_instruction(
        harness.controller,
        ExecuteUnfreezeV2Accounts {
            controller_config: harness.config,
            policy: harness.policy,
            current_council: harness.council,
            protocol_gate: harness.gate,
            proposal: proposal_key,
            linked_proposal: linked_rollback,
            poststate_checkpoint: approved.required_poststate_checkpoint,
            programdata_verification: approved.programdata_verification,
            capacity_policy: harness.capacity_policy,
            current_deployment: harness.deployment,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            instructions_sysvar: sysvar_ids::instructions::ID,
        },
        ExecuteUnfreezeV2 {
            expected: unfreeze_guard(
                &approved,
                checkpoint,
                verification,
                &council,
                &config,
                &gate,
                &deployment,
            ),
            linked_proposal: linked_rollback,
            envelope: envelope(),
        },
    )
    .expect("execute unfreeze instruction");
    let [limit, price] = envelope_prefix();
    submit(context, &[limit, price, execute], &[])
        .await
        .expect("execute separate governed unfreeze through actual controller SBF");
    (
        state(context, proposal_key).await,
        state(context, harness.deployment).await,
        state(context, harness.gate).await,
    )
}

fn spread_init_user_collateral_instruction(
    target: Pubkey,
    user: Pubkey,
    collateral: Pubkey,
    gate: Pubkey,
    epoch: u64,
) -> Instruction {
    let mut data = vec![9u8];
    data.extend_from_slice(b"AGV1");
    data.push(1);
    data.extend_from_slice(&[0; 3]);
    data.extend_from_slice(&epoch.to_le_bytes());
    Instruction {
        program_id: target,
        accounts: vec![sw(user), rw(collateral), ro(system_program::ID), ro(gate)],
        data,
    }
}

#[test]
fn standalone_review_profile_covers_two_verified_artifacts() {
    let chunks = artifact_chunk_count(
        MAX_REHEARSED_ARTIFACT_LENGTH,
        ARTIFACT_BINDING_CHUNK_SIZE_V1,
    )
    .expect("maximum rehearsed artifact chunk count");
    let conservative_runway = u64::from(chunks)
        .checked_mul(6)
        .and_then(|slots| slots.checked_add(BUFFER_REVIEW_FIXED_RUNWAY_SLOTS))
        .expect("review runway arithmetic");
    assert!(
        REVIEW_SLOTS >= conservative_runway,
        "local review profile must cover two buffers at three slots per chunk plus fixed custody transactions"
    );
}

#[tokio::test]
#[ignore = "requires exact controller and sacrificial target SBF artifacts"]
async fn actual_controller_sbf_checked_handoff_and_governed_bootstrap_activation() {
    let standalone = std::env::var("AMOEBA_STANDALONE_VALIDATOR").as_deref() == Ok("1");
    let maximum_geometry = std::env::var("AMOEBA_MAXIMUM_GEOMETRY").as_deref() == Ok("1");
    let rollback_rehearsal = std::env::var("AMOEBA_V3_ROLLBACK_REHEARSAL").as_deref() == Ok("1");
    assert!(
        !(standalone && maximum_geometry),
        "maximum geometry is an actual-SBF ProgramTest proof; standalone evidence uses the real Spread artifact"
    );
    assert!(
        !(rollback_rehearsal && maximum_geometry),
        "rollback rehearsal uses two distinct exact ELFs and is separate from maximum geometry"
    );
    assert!(
        !(rollback_rehearsal && standalone),
        "the rollback fault trigger is explicitly ProgramTest-only; standalone evidence must not imply a natural failed artifact"
    );
    let controller_artifact = read_controller_sbf();
    let (mut spread_artifact, target_is_spread) = read_sacrificial_target_sbf();
    assert!(
        target_is_spread || !standalone,
        "generic sacrificial target coverage is ProgramTest-only and cannot stand in for Spread evidence"
    );
    if maximum_geometry {
        spread_artifact.resize(MAX_ARTIFACT_BYTES_V1 as usize, 0);
    }
    let target_programdata_capacity = if maximum_geometry {
        MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1 as usize
    } else {
        spread_artifact.len()
    };
    let primary_artifact = if rollback_rehearsal {
        assert!(
            controller_artifact.len() < spread_artifact.len(),
            "rollback rehearsal requires a shorter valid primary ELF than the rollback ELF"
        );
        controller_artifact.as_slice()
    } else {
        spread_artifact.as_slice()
    };
    let rollback_artifact = spread_artifact.as_slice();
    let phase3_manifest_controller =
        Pubkey::from_str("4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi")
            .expect("Phase 3 manifest controller id");
    let configured_controller = std::env::var("AMOEBA_SYNTHETIC_CONTROLLER_PROGRAM_ID")
        .unwrap_or_else(|_| "8qbHbw2BbbTHBW1sbeqakYXVKRQM8Ne7pLK7m6CVfeR".into());
    let controller =
        Pubkey::from_str(&configured_controller).expect("synthetic local ceremony controller id");
    if target_is_spread {
        assert!(
            std::env::var_os("AMOEBA_SYNTHETIC_CONTROLLER_PROGRAM_ID").is_some(),
            "exact Phase 3 Spread rehearsal must explicitly select its manifest controller"
        );
        assert_eq!(
            controller, phase3_manifest_controller,
            "exact Phase 3 Spread artifact is bound to its manifest controller"
        );
    } else {
        assert_ne!(
            controller, phase3_manifest_controller,
            "generic sacrificial CI cannot present itself as the Phase 3 Spread bridge"
        );
    }
    let controller_programdata = derive_upgradeable_programdata_address(&controller).0;
    let target =
        Pubkey::from_str("9ipkBCjEfeJDMF6AFrezRmDDHmbnmeyv45cfXNqAnWsH").expect("Spread target id");
    let target_programdata = derive_upgradeable_programdata_address(&target).0;
    let initializer = Keypair::new();
    let legacy_authority = Keypair::new();
    let guardian = Keypair::new();
    let seats = std::array::from_fn(|_| Keypair::new());
    let spill = Pubkey::new_unique();
    let controller_bootstrap_buffer = Pubkey::new_unique();
    let target_bootstrap_buffer = Pubkey::new_unique();
    let former_authority_buffer = Pubkey::new_unique();
    let primary_uploader = Keypair::new();
    let primary_buffer = Pubkey::new_unique();
    let rollback_uploader = Keypair::new();
    let rollback_buffer = Pubkey::new_unique();
    let harness = CeremonyHarness {
        controller,
        controller_programdata,
        target,
        target_programdata,
        config: derive_controller_config_pda(&controller, &target).0,
        authority: derive_authority_pda(&controller, &target).0,
        gate: derive_gate_pda(&controller, &target).0,
        policy: derive_policy_pda(&controller, &target, 1).0,
        council: derive_council_pda(&controller, &target, 1).0,
        capacity_policy: derive_capacity_policy_pda(&controller, &target).0,
        controller_release: derive_controller_release_commitment_pda(&controller, &target).0,
        immutability_receipt: derive_controller_immutability_receipt_pda(&controller, &target).0,
        lifecycle_registry: derive_governance_lifecycle_registry_v2(&controller, &target).0,
        timing_profile: derive_governance_timing_profile_v1(&controller, &target, 1).0,
        handoff_proposal: derive_governance_action_proposal_v2(
            &controller,
            &target,
            GovernanceActionKindV2::TargetAuthorityHandoff,
            1,
        )
        .0,
        handoff_receipt: derive_target_handoff_receipt_pda(&controller, &target).0,
        activation_proposal: derive_governance_action_proposal_v2(
            &controller,
            &target,
            GovernanceActionKindV2::BootstrapActivation,
            2,
        )
        .0,
        activation_receipt: derive_bootstrap_activation_receipt_pda(&controller, &target).0,
        deployment: derive_current_deployment_state_pda(&controller, &target).0,
        initializer,
        legacy_authority,
        guardian,
        seats,
        spill,
        controller_bootstrap_buffer,
        target_bootstrap_buffer,
        former_authority_buffer,
    };

    let mut test = ProgramTest::default();
    test.prefer_bpf(true);
    // Executable Loader-v3 programs must exist in genesis so the initial Bank
    // builds one coherent cache entry from the matching ProgramData account.
    // Injecting them through ProgramTest's post-genesis account list races the
    // loader cache and is rejected as an unexpected replacement.
    test.add_genesis_account(
        controller,
        account(
            UPGRADEABLE_LOADER_ID,
            program_bytes(controller_programdata),
            true,
        ),
    );
    test.add_genesis_account(
        controller_programdata,
        account(
            UPGRADEABLE_LOADER_ID,
            programdata_bytes(
                GENESIS_PROGRAMDATA_SLOT,
                harness.initializer.pubkey(),
                &controller_artifact,
                controller_artifact.len(),
            ),
            false,
        ),
    );
    test.add_genesis_account(
        target,
        account(
            UPGRADEABLE_LOADER_ID,
            program_bytes(target_programdata),
            true,
        ),
    );
    test.add_genesis_account(
        target_programdata,
        account(
            UPGRADEABLE_LOADER_ID,
            programdata_bytes(
                GENESIS_PROGRAMDATA_SLOT,
                harness.legacy_authority.pubkey(),
                &spread_artifact,
                target_programdata_capacity,
            ),
            false,
        ),
    );
    test.add_account(
        harness.controller_bootstrap_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(harness.initializer.pubkey(), &controller_artifact),
            false,
        ),
    );
    test.add_account(
        harness.target_bootstrap_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(harness.legacy_authority.pubkey(), &spread_artifact),
            false,
        ),
    );
    test.add_account(
        harness.former_authority_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(harness.legacy_authority.pubkey(), &spread_artifact),
            false,
        ),
    );
    test.add_account(
        primary_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(primary_uploader.pubkey(), primary_artifact),
            false,
        ),
    );
    test.add_account(
        rollback_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(rollback_uploader.pubkey(), rollback_artifact),
            false,
        ),
    );
    for key in [
        harness.initializer.pubkey(),
        harness.legacy_authority.pubkey(),
        harness.guardian.pubkey(),
        harness.spill,
        harness.authority,
        primary_uploader.pubkey(),
        rollback_uploader.pubkey(),
    ] {
        test.add_account(key, account(system_program::ID, Vec::new(), false));
    }
    for seat in &harness.seats {
        test.add_account(
            seat.pubkey(),
            account(system_program::ID, Vec::new(), false),
        );
    }
    let mut _standalone_validator = None;
    let mut context = if standalone {
        let (context, validator) = start_standalone_validator(
            &harness,
            Keypair::new(),
            &primary_uploader,
            primary_buffer,
            &rollback_uploader,
            rollback_buffer,
            &controller_artifact,
            &spread_artifact,
            primary_artifact,
            rollback_artifact,
        )
        .await;
        _standalone_validator = Some(validator);
        context
    } else {
        CeremonyContext::program_test(test.start_with_context().await)
    };

    // Genesis ProgramData uses deployment slot zero, exactly like
    // ProgramTest's upgradeable-program genesis helper. Re-upgrading each
    // synthetic program in slot one through the real Loader-v3 creates a
    // distinct cache version effective in slot two. This avoids asking Agave
    // to replace an identical cache entry while still deriving every later
    // ProgramData observation from Loader-owned transitions.
    let [controller_limit, controller_price] = envelope_prefix();
    let bootstrap_controller = direct_loader_upgrade(
        &harness.controller,
        &harness.controller_bootstrap_buffer,
        &harness.initializer.pubkey(),
        &harness.spill,
    );
    submit(
        &mut context,
        &[controller_limit, controller_price, bootstrap_controller],
        &[&harness.initializer],
    )
    .await
    .expect("bootstrap controller through real Loader-v3");

    let [target_limit, target_price] = envelope_prefix();
    let bootstrap_target = direct_loader_upgrade(
        &harness.target,
        &harness.target_bootstrap_buffer,
        &harness.legacy_authority.pubkey(),
        &harness.spill,
    );
    submit(
        &mut context,
        &[target_limit, target_price, bootstrap_target],
        &[&harness.legacy_authority],
    )
    .await
    .expect("bootstrap Spread fixture through real Loader-v3");

    advance_to_slot(&mut context, INITIALIZATION_SLOT)
        .await
        .expect("activate exact Loader-upgraded SBF programs");
    initialize_controller(&mut context, &harness, &controller_artifact).await;
    initialize_lifecycle_registry(&mut context, &harness).await;
    let bootstrap_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    assert_eq!(bootstrap_gate.status, GateStatusV1::EmergencyFrozen);
    assert_eq!(bootstrap_gate.epoch, 1);
    assert_eq!(
        bootstrap_gate.freeze_reason_code,
        BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
    );

    let (controller_pre_key, controller_pre) = observe_programdata(
        &mut context,
        &harness,
        harness.controller,
        harness.controller_programdata,
        harness.controller_release,
        ProgramDataObservationPurposeV1::ControllerImmutability,
        1,
        &controller_artifact,
        Some(harness.initializer.pubkey()),
    )
    .await;
    let make_immutable =
        set_upgrade_authority(&harness.controller, &harness.initializer.pubkey(), None);
    submit(&mut context, &[make_immutable], &[&harness.initializer])
        .await
        .expect("make ephemeral controller immutable through real Loader-v3");
    let controller_after = bytes(&mut context, harness.controller_programdata).await;
    assert_eq!(
        parse_upgradeable_programdata(&controller_after)
            .expect("immutable controller ProgramData")
            .upgrade_authority,
        None
    );
    let (controller_post_key, controller_post) = observe_programdata(
        &mut context,
        &harness,
        harness.controller,
        harness.controller_programdata,
        harness.controller_release,
        ProgramDataObservationPurposeV1::ControllerImmutability,
        2,
        &controller_artifact,
        None,
    )
    .await;
    let immutable = record_controller_immutability(
        &mut context,
        &harness,
        controller_pre_key,
        &controller_pre,
        controller_post_key,
        &controller_post,
    )
    .await;

    let (handoff_observation_key, handoff_observation) = observe_programdata(
        &mut context,
        &harness,
        harness.target,
        harness.target_programdata,
        harness.immutability_receipt,
        ProgramDataObservationPurposeV1::TargetHandoffBridge,
        1,
        &spread_artifact,
        Some(harness.legacy_authority.pubkey()),
    )
    .await;
    let handoff = governed_handoff(
        &mut context,
        &harness,
        handoff_observation_key,
        &handoff_observation,
    )
    .await;
    let handed_off_programdata = bytes(&mut context, harness.target_programdata).await;
    assert_eq!(
        parse_upgradeable_programdata(&handed_off_programdata)
            .expect("handed-off target ProgramData")
            .upgrade_authority,
        Some(harness.authority)
    );

    let programdata_before_old_authority_attempt = handed_off_programdata.clone();
    let buffer_before_old_authority_attempt =
        bytes(&mut context, harness.former_authority_buffer).await;
    let old_authority_upgrade = direct_loader_upgrade(
        &harness.target,
        &harness.former_authority_buffer,
        &harness.legacy_authority.pubkey(),
        &harness.spill,
    );
    let former_authority_failure = submit_expected_failure_with_evidence(
        &mut context,
        &[old_authority_upgrade],
        &[&harness.legacy_authority],
    )
    .await;
    let programdata_after_old_authority_attempt =
        bytes(&mut context, harness.target_programdata).await;
    assert_eq!(
        programdata_after_old_authority_attempt,
        programdata_before_old_authority_attempt
    );
    assert_eq!(
        bytes(&mut context, harness.former_authority_buffer).await,
        buffer_before_old_authority_attempt
    );

    let next_observation_slot = handoff.accepted_slot + 1;
    advance_to_slot(&mut context, next_observation_slot)
        .await
        .expect("post-handoff observation slot");
    let (activation_observation_key, activation_observation) = observe_programdata(
        &mut context,
        &harness,
        harness.target,
        harness.target_programdata,
        harness.handoff_receipt,
        ProgramDataObservationPurposeV1::BootstrapActivation,
        1,
        &spread_artifact,
        Some(harness.authority),
    )
    .await;
    let (deployment, activation) = governed_activation(
        &mut context,
        &harness,
        activation_observation_key,
        &activation_observation,
        &immutable,
        &handoff,
    )
    .await;
    let active_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    assert_eq!(active_gate.status, GateStatusV1::Active);
    assert_eq!(active_gate.epoch, bootstrap_gate.epoch + 1);
    assert_eq!(active_gate.active_proposal, Pubkey::default());
    assert_eq!(
        active_gate.last_completed_proposal,
        harness.activation_proposal
    );
    assert_eq!(deployment.installed_authority, harness.authority);
    assert_eq!(activation.activated_gate_epoch, active_gate.epoch);
    assert_eq!(
        parse_upgradeable_programdata(&bytes(&mut context, harness.target_programdata).await)
            .expect("activated target ProgramData")
            .upgrade_authority,
        Some(harness.authority)
    );

    let ceremony_account_snapshots = capture_finalized_ceremony_accounts(
        &mut context,
        activation.finalized_slot,
        &[
            ("capacity-policy", harness.capacity_policy),
            ("controller-release", harness.controller_release),
            ("controller-pre-observation", controller_pre_key),
            ("controller-post-observation", controller_post_key),
            (
                "controller-immutability-receipt",
                harness.immutability_receipt,
            ),
            ("handoff-proposal", harness.handoff_proposal),
            ("handoff-pre-observation", handoff_observation_key),
            ("handoff-receipt", harness.handoff_receipt),
            ("activation-proposal", harness.activation_proposal),
            ("activation-observation", activation_observation_key),
            ("activation-receipt", harness.activation_receipt),
            ("current-deployment-state", harness.deployment),
        ],
    )
    .await;

    let lifecycle_config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let primary_key = derive_proposal_pda(
        &harness.controller,
        &harness.target,
        lifecycle_config.next_proposal_id,
    )
    .0;
    let rollback_key = derive_proposal_pda(
        &harness.controller,
        &harness.target,
        lifecycle_config.next_proposal_id + 1,
    )
    .0;
    let created_primary = create_upgrade_proposal(
        &mut context,
        &harness,
        0,
        primary_key,
        ProposalClassV1::RoutineUpgrade,
        primary_buffer,
        primary_uploader.pubkey(),
        primary_artifact,
        OptionalPubkeyV1::none(),
        OptionalPubkeyV1::some(rollback_key).expect("rollback proposal option"),
        OptionalPubkeyV1::some(rollback_buffer).expect("rollback buffer option"),
        Some(rollback_artifact),
    )
    .await;
    assert_eq!(
        created_primary.proposal_id,
        lifecycle_config.next_proposal_id
    );
    let (sealed_primary, primary_verification) = seal_upgrade_buffer(
        &mut context,
        &harness,
        primary_key,
        primary_buffer,
        &primary_uploader,
        primary_artifact,
    )
    .await;
    assert_eq!(sealed_primary.state, ProposalStateV2::BufferVerified);
    assert_eq!(
        primary_verification.status,
        BufferVerificationStatusV1::Verified
    );

    let rollback_creation_slot = current_slot(&mut context).await + ROLLBACK_DELAY + 1;
    advance_to_slot(&mut context, rollback_creation_slot)
        .await
        .expect("rollback creation runway");
    let created_rollback = create_upgrade_proposal(
        &mut context,
        &harness,
        1,
        rollback_key,
        ProposalClassV1::EmergencyRollback,
        rollback_buffer,
        rollback_uploader.pubkey(),
        rollback_artifact,
        OptionalPubkeyV1::some(primary_key).expect("primary proposal option"),
        OptionalPubkeyV1::none(),
        OptionalPubkeyV1::none(),
        None,
    )
    .await;
    assert_eq!(
        created_rollback.proposal_id,
        created_primary.proposal_id + 1
    );
    let (sealed_rollback, rollback_verification) = seal_upgrade_buffer(
        &mut context,
        &harness,
        rollback_key,
        rollback_buffer,
        &rollback_uploader,
        rollback_artifact,
    )
    .await;
    assert_eq!(sealed_rollback.state, ProposalStateV2::BufferVerified);
    assert_eq!(
        rollback_verification.status,
        BufferVerificationStatusV1::Verified
    );

    let primary_queued = approve_and_queue_v3(&mut context, &harness, primary_key).await;
    let rollback_queued = approve_and_queue_v3(&mut context, &harness, rollback_key).await;
    assert_eq!(primary_queued.state, ProposalStateV2::Timelocked);
    assert_eq!(rollback_queued.state, ProposalStateV2::Timelocked);
    assert!(
        rollback_queued.expiry_slot
            > primary_queued.expiry_slot + lifecycle_config.rollback_delay_slots
    );

    let freeze_slot = primary_queued
        .not_before_slot
        .max(rollback_queued.not_before_slot);
    advance_to_slot(&mut context, freeze_slot)
        .await
        .expect("primary and rollback timelocks elapsed");
    let prefreeze_config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let prefreeze_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let capacity: ProgramDataCapacityPolicyV1 = state(&mut context, harness.capacity_policy).await;
    let prefreeze_deployment: CurrentDeploymentStateV1 =
        state(&mut context, harness.deployment).await;
    let freeze = freeze_proposal_v3_instruction(
        harness.controller,
        FreezeProposalV3Accounts {
            controller_config: harness.config,
            policy: harness.policy,
            creation_council: harness.council,
            protocol_gate: harness.gate,
            capacity_policy: harness.capacity_policy,
            current_deployment: harness.deployment,
            proposal: primary_key,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            authority_pda: harness.authority,
            rollback_proposal: rollback_key,
            rollback_buffer_verification: rollback_queued.buffer_verification,
            rollback_buffer,
        },
        FreezeProposalV3 {
            expected: proposal_guard(
                &primary_queued,
                &prefreeze_config,
                &prefreeze_gate,
                &capacity,
                &prefreeze_deployment,
            ),
            expected_next_gate_epoch: prefreeze_gate.epoch + 1,
        },
    )
    .expect("freeze proposal instruction");
    submit(&mut context, &[freeze], &[])
        .await
        .expect("cross governed freeze through actual controller SBF");
    let frozen_config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let frozen_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let frozen_primary: UpgradeProposalV3 = state(&mut context, primary_key).await;
    assert_eq!(frozen_gate.status, GateStatusV1::FrozenForUpgrade);
    assert_eq!(frozen_gate.active_proposal, primary_key);
    assert_eq!(frozen_primary.state, ProposalStateV2::Frozen);
    assert_eq!(frozen_config.target_nonce, frozen_primary.target_nonce + 1);

    let spread_user = context.payer.pubkey();
    let spread_user_collateral = Pubkey::find_program_address(
        &[b"ameba-spread-v2", b"user_collateral", spread_user.as_ref()],
        &harness.target,
    )
    .0;
    if target_is_spread {
        let frozen_mutation = spread_init_user_collateral_instruction(
            harness.target,
            spread_user,
            spread_user_collateral,
            harness.gate,
            frozen_gate.epoch,
        );
        assert!(
            submit(&mut context, &[frozen_mutation], &[]).await.is_err(),
            "Spread mutation must remain rejected throughout the frozen lifecycle"
        );
        assert!(maybe_account(&mut context, spread_user_collateral)
            .await
            .is_none());
    }

    let (prestate_observation_key, prestate_observation) = observe_programdata(
        &mut context,
        &harness,
        harness.target,
        harness.target_programdata,
        primary_key,
        ProgramDataObservationPurposeV1::ProposalPrestate,
        1,
        &spread_artifact,
        Some(harness.authority),
    )
    .await;
    let prestate = attest_and_finalize_checkpoint_v3(
        &mut context,
        &harness,
        primary_key,
        StateCheckpointPhaseV1::Prestate,
        prestate_observation_key,
        &prestate_observation,
        "ceremony-protected-state",
    )
    .await;
    assert_eq!(prestate.phase, StateCheckpointPhaseV1::Prestate);

    let (execution_observation_key, execution_observation) = observe_programdata(
        &mut context,
        &harness,
        harness.target,
        harness.target_programdata,
        primary_key,
        ProgramDataObservationPurposeV1::ProposalPrestate,
        2,
        &spread_artifact,
        Some(harness.authority),
    )
    .await;
    assert_ne!(execution_observation_key, prestate_observation_key);
    let execution_config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let execution_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let execution_capacity: ProgramDataCapacityPolicyV1 =
        state(&mut context, harness.capacity_policy).await;
    let execution_deployment: CurrentDeploymentStateV1 =
        state(&mut context, harness.deployment).await;
    let execution_primary: UpgradeProposalV3 = state(&mut context, primary_key).await;
    let execution_rollback: UpgradeProposalV3 = state(&mut context, rollback_key).await;
    let durable_nonce = if standalone {
        Some(create_durable_nonce(&mut context).await)
    } else {
        None
    };
    let durable_nonce_authority = durable_nonce.as_ref().map_or_else(
        || context.payer.pubkey(),
        |(_, _, authority)| authority.pubkey(),
    );
    let execution_envelope = durable_nonce
        .as_ref()
        .map(|(nonce, _, _)| envelope_with_nonce(*nonce, durable_nonce_authority))
        .unwrap_or_else(envelope);
    let execute_upgrade = execute_upgrade_v2_instruction(
        harness.controller,
        ExecuteUpgradeV2Accounts {
            controller_config: harness.config,
            policy: harness.policy,
            protocol_gate: harness.gate,
            proposal: primary_key,
            counterpart_proposal: rollback_key,
            counterpart_buffer_verification: rollback_queued.buffer_verification,
            capacity_policy: harness.capacity_policy,
            current_deployment: harness.deployment,
            prestate_programdata_observation: prestate_observation_key,
            prestate_checkpoint: execution_primary.prestate_checkpoint,
            current_programdata_observation: execution_observation_key,
            buffer_verification: execution_primary.buffer_verification,
            target_programdata: harness.target_programdata,
            target_program: harness.target,
            buffer: primary_buffer,
            canonical_spill_treasury: harness.spill,
            rent_sysvar: sysvar_ids::rent::ID,
            clock_sysvar: sysvar_ids::clock::ID,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            instructions_sysvar: sysvar_ids::instructions::ID,
        },
        ExecuteUpgradeV2 {
            expected: proposal_guard(
                &execution_primary,
                &execution_config,
                &execution_gate,
                &execution_capacity,
                &execution_deployment,
            ),
            expected_prestate_checkpoint_digest: prestate.checkpoint_digest,
            expected_prestate_checkpoint_generation: prestate.checkpoint_generation,
            expected_observation_digest: execution_observation.observation_digest,
            expected_observation_generation: execution_observation.generation,
            expected_observation_root: execution_observation.final_raw_merkle_root,
            expected_observation_finalized_slot: execution_observation.finalized_slot,
            expected_actual_capacity: execution_observation.actual_capacity,
            expected_sealed_buffer_header_hash: primary_verification.sealed_buffer_header_hash,
            expected_verified_chunk_count: primary_verification.verified_chunk_count,
            expected_buffer_verification_status: primary_verification.status,
            expected_counterpart_proposal_digest: execution_rollback.proposal_digest,
            expected_counterpart_buffer_verification_status: rollback_verification.status,
            envelope: execution_envelope,
        },
    )
    .expect("execute upgrade instruction");
    let upgrade_submission_slot = current_slot(&mut context).await;
    let [limit, price] = envelope_prefix();
    let upgrade_signature_hex = if let Some((
        durable_nonce_account,
        durable_nonce_blockhash,
        durable_nonce_authority_signer,
    )) = durable_nonce
    {
        let advance_nonce = system_instruction::advance_nonce_account(
            &durable_nonce_account,
            &durable_nonce_authority,
        );
        submit_with_durable_nonce(
            &mut context,
            &[advance_nonce, limit, price, execute_upgrade],
            &[&durable_nonce_authority_signer],
            durable_nonce_blockhash,
        )
        .await
        .expect("execute one durable-nonce typed real Loader-v3 Upgrade CPI through controller SBF")
    } else {
        submit(&mut context, &[limit, price, execute_upgrade], &[])
            .await
            .expect("execute one typed real Loader-v3 Upgrade CPI through controller SBF");
        "0".repeat(128)
    };
    assert_eq!(upgrade_signature_hex.len(), 128);
    let executed: UpgradeProposalV3 = state(&mut context, primary_key).await;
    assert_eq!(executed.state, ProposalStateV2::UpgradeExecuted);
    let deployed_bytes = bytes(&mut context, harness.target_programdata).await;
    let deployed_header = parse_upgradeable_programdata(&deployed_bytes)
        .expect("parse upgraded target ProgramData header");
    assert!(deployed_header.deployed_slot >= upgrade_submission_slot);
    assert_eq!(
        executed.upgrade_executed_slot,
        deployed_header.deployed_slot
    );
    assert_eq!(
        &deployed_bytes[LOADER_PROGRAMDATA_METADATA_LEN
            ..LOADER_PROGRAMDATA_METADATA_LEN + primary_artifact.len()],
        primary_artifact
    );
    if rollback_rehearsal {
        let tail = &deployed_bytes[LOADER_PROGRAMDATA_METADATA_LEN + primary_artifact.len()..];
        assert!(
            tail.iter().all(|byte| *byte == 0),
            "pinned Loader-v3 must retain its measured zero-tail behavior"
        );
        let (fault_before, fault_after) =
            inject_programdata_payload_fault(&mut context, harness.target_programdata, 0).await;
        assert_eq!(fault_before, primary_artifact[0]);
        assert_eq!(fault_after, primary_artifact[0] ^ 1);
        advance_to_slot(&mut context, deployed_header.deployed_slot + 1)
            .await
            .expect("failure observation must begin strictly after the primary Loader slot");
        let (failure_observation_key, failure_observation) = begin_failure_programdata_observation(
            &mut context,
            &harness,
            primary_key,
            primary_artifact,
            1,
        )
        .await;
        let first_chunk_len = primary_artifact
            .len()
            .min(ARTIFACT_BINDING_CHUNK_SIZE_V1 as usize);
        let expected_leaf = artifact_chunk_leaf_hash(0, &primary_artifact[..first_chunk_len])
            .expect("primary first artifact leaf");
        let failure_proof =
            artifact_merkle_proof(primary_artifact, ARTIFACT_BINDING_CHUNK_SIZE_V1, 0)
                .expect("primary first artifact proof");
        let (failure_key, observe_failure) = build_programdata_failure_instruction(
            &mut context,
            &harness,
            primary_key,
            failure_observation_key,
            &failure_observation,
            ProgramDataMismatchClassV2::ArtifactPayload,
            0,
            expected_leaf,
            &failure_proof,
        )
        .await;
        submit(&mut context, &[observe_failure], &[])
            .await
            .expect("controller SBF must create the exact immutable payload-failure witness");
        let failure: ProgramDataFailureObservationV2 = state(&mut context, failure_key).await;
        assert!(failure.finalized);
        assert_eq!(
            failure.mismatch_class,
            ProgramDataMismatchClassV2::ArtifactPayload
        );
        assert_eq!(failure.expected_leaf_hash, expected_leaf);
        assert_ne!(failure.actual_leaf_hash, failure.expected_leaf_hash);

        let activation_config: ControllerConfigV1 = state(&mut context, harness.config).await;
        let activation_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
        let activation_capacity: ProgramDataCapacityPolicyV1 =
            state(&mut context, harness.capacity_policy).await;
        let activation_deployment: CurrentDeploymentStateV1 =
            state(&mut context, harness.deployment).await;
        let activation_primary: UpgradeProposalV3 = state(&mut context, primary_key).await;
        let activation_rollback: UpgradeProposalV3 = state(&mut context, rollback_key).await;
        let live_rollback_verification: BufferVerificationV1 =
            state(&mut context, activation_rollback.buffer_verification).await;
        let observation_state_hash =
            hashv(&[&bytes(&mut context, failure_observation_key).await]).to_bytes();
        let activate = activate_rollback_v2_instruction(
            harness.controller,
            ActivateRollbackV2Accounts {
                controller_config: harness.config,
                policy: harness.policy,
                protocol_gate: harness.gate,
                primary_proposal: primary_key,
                rollback_proposal: rollback_key,
                rollback_buffer_verification: activation_rollback.buffer_verification,
                primary_programdata_verification: activation_primary.programdata_verification,
                failure_observation: failure_key,
                capacity_policy: harness.capacity_policy,
                current_deployment: harness.deployment,
                programdata_observation: failure_observation_key,
                target_program: harness.target,
                target_programdata: harness.target_programdata,
                authority_pda: harness.authority,
                upgradeable_loader: UPGRADEABLE_LOADER_ID,
            },
            ActivateRollbackV2 {
                expected_primary: proposal_guard(
                    &activation_primary,
                    &activation_config,
                    &activation_gate,
                    &activation_capacity,
                    &activation_deployment,
                ),
                expected_rollback: proposal_guard(
                    &activation_rollback,
                    &activation_config,
                    &activation_gate,
                    &activation_capacity,
                    &activation_deployment,
                ),
                expected_failure_evidence_digest: failure.failure_digest,
                expected_primary_verification_generation: 0,
                expected_programdata_observation_state_hash: observation_state_hash,
                expected_programdata_observation_generation: failure_observation.generation,
                expected_rollback_buffer_verification_status: live_rollback_verification.status,
                expected_rollback_verified_chunk_bitmap: live_rollback_verification
                    .verified_chunk_bitmap,
                expected_rollback_verified_chunk_count: live_rollback_verification
                    .verified_chunk_count,
                expected_rollback_buffer_finalized_slot: live_rollback_verification.finalized_slot,
                expected_next_gate_epoch: activation_gate.epoch + 1,
            },
        )
        .expect("activate exact rollback instruction");
        let rollback_ready_slot = activation_primary
            .upgrade_executed_slot
            .checked_add(activation_config.rollback_delay_slots)
            .expect("rollback readiness slot");
        let early_activation_slot = current_slot(&mut context).await;
        assert!(
            early_activation_slot < rollback_ready_slot,
            "rollback delay negative proof must execute before slot {rollback_ready_slot}, observed {early_activation_slot}"
        );
        let activation_writable_accounts = [harness.gate, rollback_key];
        let before = snapshot_accounts(&mut context, &activation_writable_accounts).await;
        let [activation_limit, activation_price] = envelope_prefix();
        let early_failure = submit_expected_failure_with_evidence(
            &mut context,
            &[activation_limit, activation_price, activate.clone()],
            &[],
        )
        .await;
        let timing_code = GovernanceError::InvalidProposalTiming as u32;
        assert!(
            early_failure
                .error
                .contains(&format!("Custom({timing_code})"))
                || early_failure
                    .error
                    .contains(&format!("custom program error: 0x{timing_code:x}")),
            "early rollback activation must fail with InvalidProposalTiming, got {}",
            early_failure.error
        );
        assert!(early_failure.observed_slot < rollback_ready_slot);
        assert_accounts_unchanged(
            &mut context,
            &activation_writable_accounts,
            &before,
            "early rollback activation rejection",
        )
        .await;
        advance_to_slot(&mut context, rollback_ready_slot)
            .await
            .expect("rollback activation must wait the full post-upgrade delay");
        let [activation_limit, activation_price] = envelope_prefix();
        submit(
            &mut context,
            &[activation_limit, activation_price, activate],
            &[],
        )
        .await
        .expect(
            "controller SBF must activate the precommitted rollback without an active interval",
        );
        let rollback_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
        let frozen_rollback: UpgradeProposalV3 = state(&mut context, rollback_key).await;
        assert_eq!(rollback_gate.status, GateStatusV1::FrozenForUpgrade);
        assert_eq!(rollback_gate.active_proposal, rollback_key);
        assert_eq!(rollback_gate.epoch, activation_gate.epoch + 1);
        assert_eq!(frozen_rollback.state, ProposalStateV2::Frozen);

        let rollback_config: ControllerConfigV1 = state(&mut context, harness.config).await;
        let rollback_capacity: ProgramDataCapacityPolicyV1 =
            state(&mut context, harness.capacity_policy).await;
        let rollback_deployment: CurrentDeploymentStateV1 =
            state(&mut context, harness.deployment).await;
        let consumed_primary_verification: BufferVerificationV1 =
            state(&mut context, activation_primary.buffer_verification).await;
        let rollback_execute_accounts = ExecuteUpgradeV2Accounts {
            controller_config: harness.config,
            policy: harness.policy,
            protocol_gate: harness.gate,
            proposal: rollback_key,
            counterpart_proposal: primary_key,
            counterpart_buffer_verification: activation_primary.buffer_verification,
            capacity_policy: harness.capacity_policy,
            current_deployment: harness.deployment,
            prestate_programdata_observation: prestate_observation_key,
            prestate_checkpoint: activation_primary.prestate_checkpoint,
            current_programdata_observation: failure_key,
            buffer_verification: frozen_rollback.buffer_verification,
            target_programdata: harness.target_programdata,
            target_program: harness.target,
            buffer: rollback_buffer,
            canonical_spill_treasury: harness.spill,
            rent_sysvar: sysvar_ids::rent::ID,
            clock_sysvar: sysvar_ids::clock::ID,
            authority_pda: harness.authority,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            instructions_sysvar: sysvar_ids::instructions::ID,
        };
        let rollback_execute_data = ExecuteUpgradeV2 {
            expected: proposal_guard(
                &frozen_rollback,
                &rollback_config,
                &rollback_gate,
                &rollback_capacity,
                &rollback_deployment,
            ),
            expected_prestate_checkpoint_digest: prestate.checkpoint_digest,
            expected_prestate_checkpoint_generation: prestate.checkpoint_generation,
            expected_observation_digest: failure.failure_digest,
            expected_observation_generation: failure.observation_generation,
            expected_observation_root: failure.actual_leaf_hash,
            expected_observation_finalized_slot: failure.finalized_slot,
            expected_actual_capacity: failure.actual_capacity,
            expected_sealed_buffer_header_hash: live_rollback_verification
                .sealed_buffer_header_hash,
            expected_verified_chunk_count: live_rollback_verification.verified_chunk_count,
            expected_buffer_verification_status: live_rollback_verification.status,
            expected_counterpart_proposal_digest: activation_primary.proposal_digest,
            expected_counterpart_buffer_verification_status: consumed_primary_verification.status,
            envelope: envelope(),
        };
        let rollback_writable_accounts = [
            rollback_key,
            frozen_rollback.buffer_verification,
            harness.target_programdata,
            harness.target,
            rollback_buffer,
            harness.spill,
        ];
        let failure_prefix = || envelope_prefix();

        let mut different_witness_accounts = rollback_execute_accounts;
        different_witness_accounts.current_programdata_observation = failure_observation_key;
        let different_witness = execute_upgrade_v2_instruction(
            harness.controller,
            different_witness_accounts,
            rollback_execute_data.clone(),
        )
        .expect("different witness negative instruction");
        let before = snapshot_accounts(&mut context, &rollback_writable_accounts).await;
        let [limit, price] = failure_prefix();
        let failure = submit_expected_failure_with_evidence(
            &mut context,
            &[limit, price, different_witness],
            &[],
        )
        .await;
        assert!(!failure.error.is_empty());
        assert_accounts_unchanged(
            &mut context,
            &rollback_writable_accounts,
            &before,
            "different rollback witness rejection",
        )
        .await;

        let mut observation_drift_data = rollback_execute_data.clone();
        observation_drift_data.expected_observation_generation += 1;
        observation_drift_data.expected_observation_root[0] ^= 1;
        let observation_drift = execute_upgrade_v2_instruction(
            harness.controller,
            rollback_execute_accounts,
            observation_drift_data,
        )
        .expect("observation drift negative instruction");
        let before = snapshot_accounts(&mut context, &rollback_writable_accounts).await;
        let [limit, price] = failure_prefix();
        let failure = submit_expected_failure_with_evidence(
            &mut context,
            &[limit, price, observation_drift],
            &[],
        )
        .await;
        assert!(!failure.error.is_empty());
        assert_accounts_unchanged(
            &mut context,
            &rollback_writable_accounts,
            &before,
            "rollback observation drift rejection",
        )
        .await;

        let mut epoch_drift_data = rollback_execute_data.clone();
        epoch_drift_data.expected.expected_gate_epoch += 1;
        let epoch_drift = execute_upgrade_v2_instruction(
            harness.controller,
            rollback_execute_accounts,
            epoch_drift_data,
        )
        .expect("epoch drift negative instruction");
        let before = snapshot_accounts(&mut context, &rollback_writable_accounts).await;
        let [limit, price] = failure_prefix();
        let failure =
            submit_expected_failure_with_evidence(&mut context, &[limit, price, epoch_drift], &[])
                .await;
        assert!(!failure.error.is_empty());
        assert_accounts_unchanged(
            &mut context,
            &rollback_writable_accounts,
            &before,
            "rollback epoch drift rejection",
        )
        .await;

        let mut wrong_prestate_accounts = rollback_execute_accounts;
        wrong_prestate_accounts.prestate_checkpoint = frozen_rollback.prestate_checkpoint;
        let wrong_prestate = execute_upgrade_v2_instruction(
            harness.controller,
            wrong_prestate_accounts,
            rollback_execute_data.clone(),
        )
        .expect("wrong prestate negative instruction");
        let before = snapshot_accounts(&mut context, &rollback_writable_accounts).await;
        let [limit, price] = failure_prefix();
        let failure = submit_expected_failure_with_evidence(
            &mut context,
            &[limit, price, wrong_prestate],
            &[],
        )
        .await;
        assert!(!failure.error.is_empty());
        assert_accounts_unchanged(
            &mut context,
            &rollback_writable_accounts,
            &before,
            "wrong rollback prestate rejection",
        )
        .await;

        let (repaired_before, repaired_after) =
            inject_programdata_payload_fault(&mut context, harness.target_programdata, 0).await;
        assert_eq!(repaired_before, primary_artifact[0] ^ 1);
        assert_eq!(repaired_after, primary_artifact[0]);
        let repaired_mismatch = execute_upgrade_v2_instruction(
            harness.controller,
            rollback_execute_accounts,
            rollback_execute_data.clone(),
        )
        .expect("repaired mismatch negative instruction");
        let before = snapshot_accounts(&mut context, &rollback_writable_accounts).await;
        let [limit, price] = failure_prefix();
        let failure = submit_expected_failure_with_evidence(
            &mut context,
            &[limit, price, repaired_mismatch],
            &[],
        )
        .await;
        assert!(!failure.error.is_empty());
        assert_accounts_unchanged(
            &mut context,
            &rollback_writable_accounts,
            &before,
            "repaired rollback mismatch rejection",
        )
        .await;
        let (fault_restored_before, fault_restored_after) =
            inject_programdata_payload_fault(&mut context, harness.target_programdata, 0).await;
        assert_eq!(fault_restored_before, primary_artifact[0]);
        assert_eq!(fault_restored_after, primary_artifact[0] ^ 1);

        let rollback_execute = execute_upgrade_v2_instruction(
            harness.controller,
            rollback_execute_accounts,
            rollback_execute_data,
        )
        .expect("execute witness-authorized rollback instruction");
        let [limit, price] = envelope_prefix();
        submit(&mut context, &[limit, price, rollback_execute], &[])
            .await
            .expect("controller SBF must execute one typed real Loader-v3 rollback CPI");
        let rollback_executed: UpgradeProposalV3 = state(&mut context, rollback_key).await;
        assert_eq!(rollback_executed.state, ProposalStateV2::UpgradeExecuted);
        let rolled_back_bytes = bytes(&mut context, harness.target_programdata).await;
        assert_eq!(
            &rolled_back_bytes[LOADER_PROGRAMDATA_METADATA_LEN
                ..LOADER_PROGRAMDATA_METADATA_LEN + rollback_artifact.len()],
            rollback_artifact
        );
        let rolled_back_header = parse_upgradeable_programdata(&rolled_back_bytes)
            .expect("parse rolled-back ProgramData header");
        assert_eq!(
            rolled_back_header.deployed_slot,
            rollback_executed.upgrade_executed_slot
        );

        advance_to_slot(&mut context, rolled_back_header.deployed_slot + 1)
            .await
            .expect("rollback verification must start after its Loader slot");
        let (rollback_observation_key, rollback_observation) = observe_programdata(
            &mut context,
            &harness,
            harness.target,
            harness.target_programdata,
            rollback_key,
            ProgramDataObservationPurposeV1::Rollback,
            1,
            rollback_artifact,
            Some(harness.authority),
        )
        .await;
        let (rollback_verified, rollback_programdata_verification) =
            bind_and_finalize_programdata_v3(
                &mut context,
                &harness,
                rollback_key,
                rollback_observation_key,
                &rollback_observation,
            )
            .await;
        assert_eq!(
            rollback_verified.state,
            ProposalStateV2::ProgramDataVerified
        );
        let rollback_poststate = attest_and_finalize_checkpoint_v3(
            &mut context,
            &harness,
            rollback_key,
            StateCheckpointPhaseV1::Poststate,
            rollback_observation_key,
            &rollback_observation,
            "ceremony-protected-state",
        )
        .await;
        assert_eq!(
            rollback_poststate.hard_combined_root,
            prestate.hard_combined_root
        );
        let (completed_rollback, final_deployment, final_gate) = approve_and_execute_unfreeze_v3(
            &mut context,
            &harness,
            rollback_key,
            primary_key,
            &rollback_poststate,
            &rollback_programdata_verification,
        )
        .await;
        assert_eq!(completed_rollback.state, ProposalStateV2::Completed);
        assert_eq!(final_gate.status, GateStatusV1::Active);
        assert_eq!(final_gate.epoch, rollback_gate.epoch + 1);
        assert_eq!(final_gate.active_proposal, Pubkey::default());
        assert_eq!(final_gate.last_completed_proposal, rollback_key);
        assert_eq!(final_deployment.completed_proposal.value, rollback_key);
        assert_eq!(
            final_deployment.artifact_sha256,
            completed_rollback.artifact_sha256
        );
        let superseded_primary: UpgradeProposalV3 = state(&mut context, primary_key).await;
        assert_eq!(
            superseded_primary.state,
            ProposalStateV2::SupersededByRollback
        );
        assert!(superseded_primary.rollback_proposal.present);
        assert_eq!(superseded_primary.rollback_proposal.value, rollback_key);
        assert!(completed_rollback.primary_proposal.present);
        assert_eq!(completed_rollback.primary_proposal.value, primary_key);

        if target_is_spread {
            let permitted_mutation = spread_init_user_collateral_instruction(
                harness.target,
                spread_user,
                spread_user_collateral,
                harness.gate,
                final_gate.epoch,
            );
            submit(&mut context, &[permitted_mutation], &[])
                .await
                .expect(
                    "Spread must resume only after rollback poststate and separate unfreeze quorum",
                );
            assert!(maybe_account(&mut context, spread_user_collateral)
                .await
                .is_some());
        }
        println!(
            "AMOEBA_V3_ROLLBACK_EVIDENCE={{\"sbpf_target\":\"{}\",\"target_kind\":\"{}\",\"controller_program\":\"{}\",\"controller_elf_length\":{},\"controller_elf_sha256\":\"{}\",\"target_elf_length\":{},\"target_elf_sha256\":\"{}\",\"natural_zero_tail_failure\":false,\"fault_trigger\":\"programtest-one-byte-payload-corruption\",\"failure_witness_actual_controller_sbf\":true,\"rollback_activation_actual_controller_sbf\":true,\"rollback_loader_cpi_actual_controller_sbf\":true,\"rollback_programdata_verified\":true,\"rollback_poststate_accepted\":true,\"separate_unfreeze_quorum\":true,\"first_spread_mutation\":{},\"live_rpc_write\":false}}",
            std::env::var("AMOEBA_SBPF_TARGET").unwrap_or_else(|_| "unspecified".into()),
            if target_is_spread { "spread" } else { "generic-sacrificial" },
            controller,
            controller_artifact.len(),
            lower_hex(hashv(&[&controller_artifact]).as_ref()),
            spread_artifact.len(),
            lower_hex(hashv(&[&spread_artifact]).as_ref()),
            target_is_spread,
        );
        return;
    }

    advance_to_slot(&mut context, deployed_header.deployed_slot + 1)
        .await
        .expect("post-upgrade observation must begin after deployment slot");
    let (postupgrade_observation_key, postupgrade_observation) = observe_programdata(
        &mut context,
        &harness,
        harness.target,
        harness.target_programdata,
        primary_key,
        ProgramDataObservationPurposeV1::PostUpgrade,
        1,
        &spread_artifact,
        Some(harness.authority),
    )
    .await;
    let (programdata_verified, verification) = bind_and_finalize_programdata_v3(
        &mut context,
        &harness,
        primary_key,
        postupgrade_observation_key,
        &postupgrade_observation,
    )
    .await;
    assert_eq!(
        programdata_verified.state,
        ProposalStateV2::ProgramDataVerified
    );
    assert_eq!(
        verification.status,
        ProgramDataVerificationStatusV2::Verified
    );
    assert!(verification.zero_tail_verified);

    let poststate = attest_and_finalize_checkpoint_v3(
        &mut context,
        &harness,
        primary_key,
        StateCheckpointPhaseV1::Poststate,
        postupgrade_observation_key,
        &postupgrade_observation,
        "ceremony-protected-state",
    )
    .await;
    assert_eq!(poststate.phase, StateCheckpointPhaseV1::Poststate);
    assert_eq!(poststate.hard_combined_root, prestate.hard_combined_root);
    let (completed, final_deployment, final_gate) = approve_and_execute_unfreeze_v3(
        &mut context,
        &harness,
        primary_key,
        rollback_key,
        &poststate,
        &verification,
    )
    .await;
    let retired_rollback: UpgradeProposalV3 = state(&mut context, rollback_key).await;
    assert_eq!(completed.state, ProposalStateV2::Completed);
    assert_eq!(retired_rollback.state, ProposalStateV2::Retired);
    assert_eq!(final_gate.status, GateStatusV1::Active);
    assert_eq!(final_gate.epoch, frozen_gate.epoch + 1);
    assert_eq!(final_gate.active_proposal, Pubkey::default());
    assert_eq!(final_gate.last_completed_proposal, primary_key);
    assert_eq!(
        final_deployment.deployment_generation,
        deployment.deployment_generation + 1
    );
    assert_eq!(final_deployment.completed_proposal.value, primary_key);
    assert_eq!(final_deployment.artifact_sha256, executed.artifact_sha256);

    if target_is_spread {
        let permitted_mutation = spread_init_user_collateral_instruction(
            harness.target,
            spread_user,
            spread_user_collateral,
            harness.gate,
            final_gate.epoch,
        );
        submit(&mut context, &[permitted_mutation], &[])
            .await
            .expect("first post-ceremony Spread mutation through active canonical gate");
        let mutated = maybe_account(&mut context, spread_user_collateral)
            .await
            .expect("Spread mutation created canonical user collateral");
        assert_eq!(mutated.owner, harness.target);
        assert!(!mutated.data.is_empty());
    }

    if let Some(validator) = _standalone_validator.as_ref() {
        let receipt_accounts = ceremony_account_snapshots
            .iter()
            .map(|snapshot| {
                assert!(!snapshot.account.executable);
                json!({
                    "role": snapshot.role,
                    "pubkey": snapshot.pubkey.to_string(),
                    "owner": snapshot.account.owner.to_string(),
                    "executable": false,
                    "dataLength": snapshot.account.data.len(),
                    "dataSha256": lower_hex(hashv(&[&snapshot.account.data]).as_ref()),
                    "dataBase64": base64::engine::general_purpose::STANDARD.encode(&snapshot.account.data),
                    "finalizedSlot": snapshot.finalized_slot.to_string(),
                })
            })
            .collect::<Vec<_>>();
        let raw_root_before =
            lower_hex(hashv(&[&programdata_before_old_authority_attempt]).as_ref());
        let raw_root_after = lower_hex(hashv(&[&programdata_after_old_authority_attempt]).as_ref());
        let receipt_material = json!({
            "schema": "amoeba-release1-ceremony-receipt-v4",
            "version": 4,
            "production": false,
            "identityKind": "synthetic-local",
            "clusterDomainHex": lower_hex(&context.cluster_domain),
            "controllerProgram": harness.controller.to_string(),
            "controllerProgramdata": harness.controller_programdata.to_string(),
            "controllerConfig": harness.config.to_string(),
            "controllerAuthority": harness.authority.to_string(),
            "targetProgram": harness.target.to_string(),
            "targetProgramdata": harness.target_programdata.to_string(),
            "legacyAuthority": harness.legacy_authority.pubkey().to_string(),
            "upgradeableLoader": UPGRADEABLE_LOADER_ID.to_string(),
            "accounts": receipt_accounts,
            "controllerImmutabilityTransition": "loader-set-authority-to-none",
            "checkedHandoff": {
                "performed": true,
                "simulated": false,
                "bankPatched": false,
                "controllerInstructionProgram": harness.controller.to_string(),
                "topLevelInstructionCount": 1,
                "loaderCpiKind": "set-authority-checked",
                "loaderProgram": UPGRADEABLE_LOADER_ID.to_string(),
                "loaderDataHex": "07000000",
                "loaderAccounts": [
                    { "pubkey": harness.target_programdata.to_string(), "isSigner": false, "isWritable": true },
                    { "pubkey": harness.legacy_authority.pubkey().to_string(), "isSigner": true, "isWritable": false },
                    { "pubkey": harness.authority.to_string(), "isSigner": true, "isWritable": false }
                ],
                "authorityBefore": harness.legacy_authority.pubkey().to_string(),
                "authorityAfter": harness.authority.to_string(),
                "acceptedSlot": handoff.accepted_slot.to_string(),
            },
            "formerAuthorityRejection": {
                "attemptedSlot": former_authority_failure.observed_slot.to_string(),
                "signatureHex": former_authority_failure.signature_hex,
                "status": "failed",
                "errorSha256": lower_hex(hashv(&[former_authority_failure.error.as_bytes()]).as_ref()),
                "authorityAfter": harness.authority.to_string(),
                "targetRawRootBefore": raw_root_before,
                "targetRawRootAfter": raw_root_after,
            },
            "bootstrapActivation": {
                "transitionKind": "controller-bootstrap-activation-v1",
                "bankPatched": false,
                "controllerInstructionProgram": harness.controller.to_string(),
                "topLevelInstructionCount": 1,
                "innerCpiCount": 0,
                "executedSlot": activation.finalized_slot.to_string(),
                "gateStatusBefore": "emergency-frozen",
                "gateEpochBefore": bootstrap_gate.epoch.to_string(),
                "freezeReasonBefore": bootstrap_gate.freeze_reason_code,
                "gateStatusAfter": "active",
                "gateEpochAfter": active_gate.epoch.to_string(),
                "targetNonceBefore": activation.target_nonce.to_string(),
                "targetNonceAfter": activation.target_nonce.to_string(),
            },
            "postHandoffUpgradeEvents": [
                {
                    "slot": former_authority_failure.observed_slot.to_string(),
                    "authorityKind": "external-key",
                    "authority": harness.legacy_authority.pubkey().to_string(),
                    "status": "failed",
                    "artifactSha256": lower_hex(hashv(&[&spread_artifact]).as_ref()),
                    "programdataObservationDigest": lower_hex(&handoff_observation.observation_digest),
                },
                {
                    "slot": executed.upgrade_executed_slot.to_string(),
                    "authorityKind": "controller-pda",
                    "authority": harness.authority.to_string(),
                    "status": "succeeded",
                    "artifactSha256": lower_hex(&executed.artifact_sha256),
                    "programdataObservationDigest": lower_hex(&postupgrade_observation.observation_digest),
                }
            ]
        });
        fs::write(
            validator.evidence_dir.join("receipt-v4-material.json"),
            serde_json::to_vec_pretty(&receipt_material)
                .expect("serialize standalone receipt v4 material"),
        )
        .expect("write standalone receipt v4 material");
        let lookup_table = match &context.backend {
            CeremonyBackend::Rpc(backend) => backend
                .lookup_table
                .as_ref()
                .map(|table| table.key.to_string()),
            CeremonyBackend::ProgramTest(_) => None,
        };
        let transaction_evidence = json!({
            "messageVersion": 0,
            "addressLookupTable": lookup_table,
            "durableNonceUsed": true,
            "durableNonceAuthority": durable_nonce_authority.to_string(),
            "upgradeSignatureHex": upgrade_signature_hex,
            "upgradeExecutedSlot": executed.upgrade_executed_slot.to_string(),
            "finalGateEpoch": final_gate.epoch.to_string(),
            "liveRpcWrite": false,
        });
        fs::write(
            validator
                .evidence_dir
                .join("standalone-transaction-evidence.json"),
            serde_json::to_vec_pretty(&transaction_evidence)
                .expect("serialize standalone transaction evidence"),
        )
        .expect("write standalone transaction evidence");
        run_local_ceremony_cli(validator, context.cluster_domain);
    }

    if maximum_geometry {
        let raw_chunk_count = programdata_observation_chunk_count(
            deployed_bytes.len() as u64,
            PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB,
        )
        .expect("maximum ProgramData raw chunk count");
        let artifact_chunk_count =
            artifact_chunk_count(spread_artifact.len() as u64, ARTIFACT_BINDING_CHUNK_SIZE_V1)
                .expect("maximum artifact chunk count");
        assert_eq!(spread_artifact.len() as u64, MAX_ARTIFACT_BYTES_V1);
        assert_eq!(
            deployed_bytes.len() as u64,
            MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
        );
        assert_eq!(raw_chunk_count, 640);
        assert_eq!(artifact_chunk_count, 96);
        let maximum_evidence = json!({
            "schema": "amoeba-release1-maximum-geometry-actual-sbf-v1",
            "sbpfTarget": std::env::var("AMOEBA_SBPF_TARGET").unwrap_or_else(|_| "unspecified".into()),
            "targetKind": if target_is_spread { "spread" } else { "generic-sacrificial" },
            "controllerElfLength": controller_artifact.len(),
            "controllerElfSha256": lower_hex(hashv(&[&controller_artifact]).as_ref()),
            "artifactLength": spread_artifact.len(),
            "artifactSha256": lower_hex(hashv(&[&spread_artifact]).as_ref()),
            "programdataPayloadCapacity": target_programdata_capacity,
            "rawProgramdataLength": deployed_bytes.len(),
            "rawObservationChunkSize": PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB,
            "rawObservationChunkCount": raw_chunk_count,
            "artifactChunkSize": ARTIFACT_BINDING_CHUNK_SIZE_V1,
            "artifactChunkCount": artifact_chunk_count,
            "fullV3LoaderLifecycle": true,
            "rollbackPreparedAndRetired": true,
            "firstSpreadMutation": target_is_spread,
            "finalGateEpoch": final_gate.epoch,
            "liveRpcWrite": false,
        });
        if let Some(evidence_dir) = std::env::var_os("AMOEBA_MAXIMUM_EVIDENCE_DIR") {
            let evidence_dir = PathBuf::from(evidence_dir);
            fs::create_dir_all(&evidence_dir).expect("create maximum-geometry evidence directory");
            fs::write(
                evidence_dir.join("maximum-geometry-actual-sbf.json"),
                serde_json::to_vec_pretty(&maximum_evidence)
                    .expect("serialize maximum-geometry evidence"),
            )
            .expect("write maximum-geometry evidence");
        }
        println!("AMOEBA_MAXIMUM_GEOMETRY_EVIDENCE={maximum_evidence}");
    }

    println!(
        "AMOEBA_CEREMONY_AUTHORITY_EVIDENCE={{\"sbpf_target\":\"{}\",\"target_kind\":\"{}\",\"controller_elf_length\":{},\"controller_elf_sha256\":\"{}\",\"target_elf_length\":{},\"target_elf_sha256\":\"{}\",\"controller_immutable\":true,\"checked_handoff\":true,\"former_authority_rejected\":true,\"bootstrap_activation\":true,\"full_v3_loader_lifecycle\":true,\"rollback_prepared_and_retired\":true,\"first_spread_mutation\":{},\"activated_epoch\":{},\"final_epoch\":{},\"live_rpc_write\":false}}",
        std::env::var("AMOEBA_SBPF_TARGET").unwrap_or_else(|_| "unspecified".into()),
        if target_is_spread { "spread" } else { "generic-sacrificial" },
        controller_artifact.len(),
        hashv(&[&controller_artifact]),
        spread_artifact.len(),
        hashv(&[&spread_artifact]),
        target_is_spread,
        active_gate.epoch,
        final_gate.epoch,
    );
}
