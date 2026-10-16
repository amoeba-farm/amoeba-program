use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    hash::hashv,
    instruction::{AccountMeta, Instruction, InstructionError},
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
};
use solana_program_test::{processor, BanksClientError, ProgramTest, ProgramTestContext};
use solana_sdk::{
    account::{Account, AccountSharedData},
    clock::Clock,
    signature::{Keypair, Signer},
    transaction::{Transaction, TransactionError},
};
use solana_sdk_ids::{native_loader, system_program};
use std::{cell::RefCell, rc::Rc};
use upgrade_controller::{
    artifact_merkle::{
        artifact_chunk_count, artifact_merkle_root, ARTIFACT_MERKLE_SCHEME_ID,
        RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
    },
    council::compute_council_set_hash,
    instruction::*,
    pda::*,
    policy::compute_policy_hash,
    processor::process_instruction,
    release1_digest::{
        compute_checkpoint_attestation_digest_v1, compute_council_rotation_digest_v1,
        compute_emergency_freeze_observation_digest_v1, compute_emergency_resolution_digest_v1,
        compute_proposal_digest_v2, compute_state_checkpoint_digest_v1,
        compute_state_checkpoint_hard_combined_root_v1,
    },
    release1_loader_accounts::{
        loader_account_data_hash, LOADER_BUFFER_METADATA_LEN, LOADER_PROGRAMDATA_METADATA_LEN,
        LOADER_PROGRAM_ACCOUNT_LEN, LOADER_STATE_TAG_BUFFER, LOADER_STATE_TAG_PROGRAM,
        LOADER_STATE_TAG_PROGRAMDATA,
    },
    release1_model::{
        ModelDelays, ModelHardStateObservation, ModelIdentityGraph, ModelInitialization,
        ModelProgramDataObservation, ModelProposalRequest, ModelSeatTerm, Release1Model,
        Release1ModelAction, Release1ModelError, Release1ModelOutcome,
    },
    release1_state::*,
    state::{
        CheckpointPhaseV1, ControllerConfigV1, CouncilSeatV1, GateStatusV1, GovernanceCouncilSetV1,
        GovernanceModeV1, GovernancePolicyV1, OptionalPubkeyV1, ProposalClassV1, ProtocolGateV1,
        VoteRequirementV1, ACCOUNT_VERSION_V1, COUNCIL_SEAT_RESERVED_LEN,
        GOVERNANCE_COUNCIL_DISCRIMINATOR, GOVERNANCE_COUNCIL_RESERVED_LEN,
        GOVERNANCE_POLICY_DISCRIMINATOR, GOVERNANCE_POLICY_RESERVED_LEN,
    },
    GovernanceError,
};

const INITIAL_SLOT: u64 = 40;
const ACTIVE_SLOT: u64 = 50;
const TEST_CONTROLLER_ID: Pubkey = Pubkey::new_from_array([0xa1; 32]);
const TEST_TARGET_ID: Pubkey = Pubkey::new_from_array([0xb1; 32]);
const EXECUTABLE_PRIVILEGE_PROBE_ID: Pubkey = Pubkey::new_from_array([0xc1; 32]);
const PROXY_SEAT_SEED: &[u8] = b"release1-seat";
const CURRENT_PAYLOAD: &[u8] = b"release-1-current-programdata";
const CANDIDATE_ARTIFACT: &[u8] = b"release-1-candidate";
const ROLLBACK_ARTIFACT: &[u8] = b"release-1-rollback";

struct Harness {
    controller: Pubkey,
    target: Pubkey,
    target_programdata: Pubkey,
    guardian: Keypair,
    seats: [Keypair; 5],
    config: Pubkey,
    authority: Pubkey,
    gate: Pubkey,
    policy: Pubkey,
    council: Pubkey,
    spill: Pubkey,
    buffer: Pubkey,
    uploader: Pubkey,
    rollback_buffer: Pubkey,
    rollback_uploader: Pubkey,
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
    data[45..].copy_from_slice(payload);
    data
}

fn buffer_bytes(authority: Pubkey, payload: &[u8]) -> Vec<u8> {
    let mut data = vec![0; LOADER_BUFFER_METADATA_LEN + payload.len()];
    data[..4].copy_from_slice(&LOADER_STATE_TAG_BUFFER.to_le_bytes());
    data[4] = 1;
    data[5..37].copy_from_slice(authority.as_ref());
    data[37..].copy_from_slice(payload);
    data
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

fn system_account() -> Account {
    account(system_program::ID, Vec::new(), false)
}

fn state_account<T: BorshSerialize>(owner: Pubkey, value: &T) -> Account {
    account(owner, value.try_to_vec().unwrap(), false)
}

fn native_controller_adapter(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    data: &[u8],
) -> ProgramResult {
    // A native ProgramTest builtin must be native-loader-owned, while controller
    // initialization deliberately requires that same public identity to expose
    // Upgradeable Loader Program metadata. Adapt only those read-only Program
    // AccountInfos; every dispatcher and processor remains the production code.
    let mut adapted = accounts.to_vec();
    for account in &mut adapted {
        if *account.key == TEST_CONTROLLER_ID || *account.key == TEST_TARGET_ID {
            let mut program_account = account.clone();
            program_account.owner = &UPGRADEABLE_LOADER_ID;
            program_account.executable = true;
            let programdata = derive_upgradeable_programdata_address(account.key).0;
            let program_data = Box::leak(program_bytes(programdata).into_boxed_slice());
            program_account.data = Rc::new(RefCell::new(program_data));
            *account = program_account;
        }
    }
    process_instruction(program_id, &adapted, data)
}

fn executable_privilege_probe(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    data: &[u8],
) -> ProgramResult {
    if *program_id != EXECUTABLE_PRIVILEGE_PROBE_ID || data.len() < 3 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mode = data[0];
    let account_index = usize::from(u16::from_le_bytes([data[1], data[2]]));
    if account_index >= accounts.len() {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let mut adapted = accounts.to_vec();
    let instruction_data = match mode {
        0 => {
            adapted[account_index].executable = true;
            &data[3..]
        }
        1 if data.len() >= 35 => {
            let replacement = Box::leak(Box::new(Pubkey::new_from_array(
                data[3..35].try_into().unwrap(),
            )));
            adapted[account_index].key = replacement;
            &data[35..]
        }
        _ => return Err(ProgramError::InvalidInstructionData),
    };
    native_controller_adapter(&TEST_CONTROLLER_ID, &adapted, instruction_data)
}

fn set_account(context: &mut ProgramTestContext, key: Pubkey, value: Account) {
    context.set_account(&key, &AccountSharedData::from(value));
}

async fn bytes(context: &mut ProgramTestContext, key: Pubkey) -> Vec<u8> {
    context
        .banks_client
        .get_account(key)
        .await
        .unwrap()
        .unwrap()
        .data
}

async fn state<T: BorshDeserialize>(context: &mut ProgramTestContext, key: Pubkey) -> T {
    T::try_from_slice(&bytes(context, key).await).unwrap()
}

async fn current_slot(context: &mut ProgramTestContext) -> u64 {
    context
        .banks_client
        .get_sysvar::<Clock>()
        .await
        .unwrap()
        .slot
}

async fn set_clock_slot(context: &mut ProgramTestContext, slot: u64) {
    let mut clock = context.banks_client.get_sysvar::<Clock>().await.unwrap();
    assert!(slot >= clock.slot);
    clock.slot = slot;
    context.set_sysvar(&clock);
}

async fn submit(
    context: &mut ProgramTestContext,
    instructions: &[Instruction],
    extra_signers: &[&Keypair],
) -> Result<(), BanksClientError> {
    context.get_new_latest_blockhash().await.unwrap();
    let mut signers: Vec<&dyn Signer> = vec![&context.payer];
    signers.extend(extra_signers.iter().map(|signer| *signer as &dyn Signer));
    let transaction = Transaction::new_signed_with_payer(
        instructions,
        Some(&context.payer.pubkey()),
        &signers,
        context.last_blockhash,
    );
    context.banks_client.process_transaction(transaction).await
}

async fn assert_atomic_failure(
    context: &mut ProgramTestContext,
    instruction: Instruction,
    signers: &[&Keypair],
    writable: &[Pubkey],
) {
    // Include the instruction as mutated by the negative case, rather than
    // relying only on the valid instruction's writable set.  This is
    // important for alternate-account and duplicate-alias cases: a
    // substituted writable must be proven byte-identical too.
    let mut writable_keys = writable.to_vec();
    for meta in &instruction.accounts {
        if meta.is_writable && !writable_keys.contains(&meta.pubkey) {
            writable_keys.push(meta.pubkey);
        }
    }
    let mut before = Vec::with_capacity(writable_keys.len());
    for key in &writable_keys {
        before.push(
            context
                .banks_client
                .get_account(*key)
                .await
                .expect("pre-failure account read")
                .map(|account| account.data),
        );
    }
    assert!(submit(context, &[instruction], signers).await.is_err());
    for (key, expected) in writable_keys.iter().zip(before) {
        let actual = context
            .banks_client
            .get_account(*key)
            .await
            .expect("post-failure account read")
            .map(|account| account.data);
        assert_eq!(actual, expected, "mutated writable account {key}");
    }
}

async fn assert_direct_executable_rejected_unchanged(
    context: &mut ProgramTestContext,
    instruction: &Instruction,
    executable_index: usize,
    signers: &[&Keypair],
    writable: &[Pubkey],
) {
    // BanksServer attempts to load every executable transaction account into
    // ProgramCache before invoking the controller.  Marking a controller-owned
    // state account executable in the bank therefore tests Agave's ELF loader,
    // not this controller, and malformed fixture bytes can leave BanksServer in
    // repeated tarpc timeouts.  A native test-only probe runs inside the real
    // ProgramTest invoke context, flips only `AccountInfo::executable`, and
    // calls the same production dispatcher. The bank never sees malformed
    // executable state and therefore never tries to ELF-load it.
    let executable_index = u16::try_from(executable_index).unwrap();
    let mut data = vec![0];
    data.extend_from_slice(&executable_index.to_le_bytes());
    data.extend_from_slice(&instruction.data);
    let probe = Instruction {
        program_id: EXECUTABLE_PRIVILEGE_PROBE_ID,
        accounts: instruction.accounts.clone(),
        data,
    };
    let mut before = Vec::with_capacity(writable.len());
    for key in writable {
        before.push(
            context
                .banks_client
                .get_account(*key)
                .await
                .expect("executable-case pre-read")
                .map(|account| account.data),
        );
    }
    match submit(context, &[probe], signers).await {
        Err(BanksClientError::TransactionError(TransactionError::InstructionError(
            0,
            InstructionError::Custom(actual),
        ))) => assert_eq!(
            actual,
            GovernanceError::InvalidAccountPrivileges as u32,
            "executable-account case returned the wrong controller error",
        ),
        other => panic!("unexpected executable-account result: {other:?}"),
    }
    let mut after = Vec::with_capacity(writable.len());
    for key in writable {
        after.push(
            context
                .banks_client
                .get_account(*key)
                .await
                .expect("executable-case post-read")
                .map(|account| account.data),
        );
    }
    assert_eq!(after, before, "executable-account failure mutated state");
}

async fn assert_direct_identity_rejected_unchanged(
    context: &mut ProgramTestContext,
    instruction: &Instruction,
    account_index: usize,
    replacement: Pubkey,
    signers: &[&Keypair],
    writable: &[Pubkey],
) {
    let account_index = u16::try_from(account_index).unwrap();
    let mut data = vec![1];
    data.extend_from_slice(&account_index.to_le_bytes());
    data.extend_from_slice(replacement.as_ref());
    data.extend_from_slice(&instruction.data);
    let probe = Instruction {
        program_id: EXECUTABLE_PRIVILEGE_PROBE_ID,
        accounts: instruction.accounts.clone(),
        data,
    };
    let mut before = Vec::with_capacity(writable.len());
    for key in writable {
        before.push(
            context
                .banks_client
                .get_account(*key)
                .await
                .expect("identity-case pre-read")
                .map(|account| account.data),
        );
    }
    assert!(matches!(
        submit(context, &[probe], signers).await,
        Err(BanksClientError::TransactionError(
            TransactionError::InstructionError(0, _)
        ))
    ));
    let mut after = Vec::with_capacity(writable.len());
    for key in writable {
        after.push(
            context
                .banks_client
                .get_account(*key)
                .await
                .expect("identity-case post-read")
                .map(|account| account.data),
        );
    }
    assert_eq!(after, before, "identity failure mutated state");
}

fn typed_account_header_corruptions(canonical: &Account) -> Vec<(&'static str, Account)> {
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
    vec![
        ("owner", wrong_owner),
        ("size", wrong_size),
        ("discriminator", wrong_discriminator),
        ("version", wrong_version),
        ("bump", wrong_bump),
    ]
}

fn account_contract_candidate() -> CheckpointCandidateV1 {
    CheckpointCandidateV1 {
        phase: StateCheckpointPhaseV1::Prestate,
        expected_subject_state: CheckpointSubjectStateV1::ProposalFrozen,
        expected_subject_digest: [1; 32],
        expected_gate_status: GateStatusV1::FrozenForUpgrade,
        expected_gate_epoch: 1,
        finalized_observation_slot: 1,
        target_programdata_slot: 1,
        target_payload_commitment: [2; 32],
        target_raw_programdata_commitment: [3; 32],
        target_capacity: 1,
        program_owned_state_root: [4; 32],
        program_owned_state_count: 1,
        logical_compressed_state_root: [5; 32],
        logical_compressed_state_count: 1,
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

fn account_contract_rotation_expectation() -> CouncilRotationExpectationV1 {
    CouncilRotationExpectationV1 {
        expected_rotation_digest: [12; 32],
        expected_current_council_version: 1,
        expected_current_council_hash: [13; 32],
        expected_candidate_council_version: 2,
        expected_candidate_council_hash: [14; 32],
        expected_gate_status: GateStatusV1::Active,
        expected_gate_epoch: 2,
        expected_target_nonce: 1,
        expected_state: CouncilRotationStateV1::Draft,
        expected_not_before_slot: 60,
        expected_expiry_slot: 100,
    }
}

fn account_contract_emergency_expectation() -> EmergencyResolutionExpectationV1 {
    EmergencyResolutionExpectationV1 {
        expected_resolution_digest: [15; 32],
        expected_policy_version: 1,
        expected_policy_hash: [16; 32],
        expected_council_version: 1,
        expected_council_hash: [17; 32],
        expected_gate_status: GateStatusV1::EmergencyFrozen,
        expected_gate_epoch: 1,
        expected_freeze_slot: INITIAL_SLOT,
        expected_freeze_reason_code: 1,
        expected_target_nonce: 1,
        expected_state: EmergencyFreezeResolutionStateV1::Draft,
        expected_not_before_slot: 60,
        expected_expiry_slot: 100,
    }
}

struct AccountContractFixture {
    initializer: Keypair,
    proposal: Pubkey,
    subject: Pubkey,
    checkpoint: Pubkey,
    checkpoint_attestation: Pubkey,
    emergency_observation: Pubkey,
    phase_evidence: Pubkey,
    baseline: Pubkey,
    checkpoint_attestations: [Pubkey; 3],
    candidate_council: Pubkey,
    rotation: Pubkey,
    emergency_resolution: Pubkey,
    alternate_config: Pubkey,
}

fn install_account_contract_fixture(
    context: &mut ProgramTestContext,
    harness: &Harness,
    config: &ControllerConfigV1,
) -> AccountContractFixture {
    let initializer = Keypair::new();
    let fixture = AccountContractFixture {
        initializer,
        proposal: derive_proposal_pda(
            &harness.controller,
            &harness.target,
            config.next_proposal_id,
        )
        .0,
        subject: Pubkey::new_unique(),
        checkpoint: Pubkey::new_unique(),
        checkpoint_attestation: Pubkey::new_unique(),
        emergency_observation: Pubkey::new_unique(),
        phase_evidence: Pubkey::new_unique(),
        baseline: Pubkey::new_unique(),
        checkpoint_attestations: std::array::from_fn(|_| Pubkey::new_unique()),
        candidate_council: derive_council_pda(&harness.controller, &harness.target, 2).0,
        rotation: derive_council_rotation_pda(&harness.controller, &harness.target, 2).0,
        emergency_resolution: derive_emergency_resolution_pda(
            &harness.controller,
            &harness.target,
            1,
        )
        .0,
        alternate_config: Pubkey::new_unique(),
    };
    for key in [
        fixture.subject,
        fixture.proposal,
        fixture.checkpoint,
        fixture.checkpoint_attestation,
        fixture.emergency_observation,
        fixture.phase_evidence,
        fixture.baseline,
        fixture.checkpoint_attestations[0],
        fixture.checkpoint_attestations[1],
        fixture.checkpoint_attestations[2],
        fixture.candidate_council,
        fixture.rotation,
        fixture.emergency_resolution,
        fixture.initializer.pubkey(),
    ] {
        set_account(context, key, system_account());
    }
    // This is deliberately valid controller-owned config data at the wrong
    // address. It separates an alternate-identity failure from owner/size and
    // header failures in the matrix below.
    set_account(
        context,
        fixture.alternate_config,
        state_account(harness.controller, config),
    );
    fixture
}

fn release1_state_account_contract_instructions<'a>(
    context: &ProgramTestContext,
    harness: &'a Harness,
    fixture: &'a AccountContractFixture,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
    council: &GovernanceCouncilSetV1,
    gate: &ProtocolGateV1,
) -> Vec<(&'static str, Instruction, Vec<&'a Keypair>)> {
    let payer = context.payer.pubkey();
    let controller_programdata = derive_upgradeable_programdata_address(&harness.controller).0;
    let target_raw = programdata_bytes(7, harness.authority, CURRENT_PAYLOAD);
    let target_raw_hash = loader_account_data_hash(&target_raw);
    let rollback_hash = hashv(&[ROLLBACK_ARTIFACT]).to_bytes();
    let rollback_root =
        artifact_merkle_root(ROLLBACK_ARTIFACT, RELEASE1_ARTIFACT_CHUNK_SIZE_V1).unwrap();
    let proposal = proposal_template(
        harness,
        config,
        policy,
        council,
        gate,
        ProposalTemplateInput {
            proposal_id: config.next_proposal_id,
            creation_slot: ACTIVE_SLOT,
            buffer: harness.buffer,
            uploader: harness.uploader,
            artifact: CANDIDATE_ARTIFACT,
            class: ProposalClassV1::RoutineUpgrade,
            primary: OptionalPubkeyV1::none(),
            rollback: OptionalPubkeyV1::some(fixture.baseline).unwrap(),
            rollback_buffer: OptionalPubkeyV1::some(harness.rollback_buffer).unwrap(),
            rollback_hash,
            rollback_root,
        },
    );
    assert_eq!(
        fixture.proposal,
        derive_proposal_pda(&harness.controller, &harness.target, proposal.proposal_id,).0
    );
    let proposal_expected = expectation(&proposal, config, gate);
    let emergency_expected = account_contract_emergency_expectation();
    let observed_programdata =
        OptionalInstructionPubkeyV1::some(harness.target_programdata).unwrap();
    let observed_authority = OptionalInstructionPubkeyV1::some(harness.authority).unwrap();
    let initialization = InitializeControllerV1 {
        cluster_domain: config.cluster_domain,
        initial_policy_version: policy.version,
        initial_council_version: council.version,
        next_proposal_id: 1,
        target_nonce: 1,
        initial_gate_epoch: 1,
        policy_activation_slot: INITIAL_SLOT,
        routine_delay_slots: config.routine_delay_slots,
        major_delay_slots: config.major_delay_slots,
        rollback_delay_slots: config.rollback_delay_slots,
        terminal_delay_slots: config.terminal_delay_slots,
        vote_review_slots: config.vote_review_slots,
        proposal_expiry_slots: config.proposal_expiry_slots,
        expected_policy_hash: policy.policy_hash,
        expected_council_hash: council.set_hash,
        seat_terms: [CouncilSeatTermV1 {
            term_start_slot: 1,
            term_end_slot: 10_000,
        }; 5],
    };
    let freeze_observation_digest = [22; 32];
    let checkpoint_digest = [23; 32];
    vec![
        (
            "tag1-initialize-controller",
            initialize_controller_v1_instruction(
                harness.controller,
                InitializeControllerV1Accounts {
                    payer,
                    initializer: fixture.initializer.pubkey(),
                    controller_program: harness.controller,
                    controller_programdata,
                    target_program: harness.target,
                    target_programdata: harness.target_programdata,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                    controller_config: harness.config,
                    authority_pda: harness.authority,
                    protocol_gate: harness.gate,
                    policy: harness.policy,
                    council: harness.council,
                    canonical_spill_treasury: harness.spill,
                    guardian: harness.guardian.pubkey(),
                    seat_authorities: harness.seats.each_ref().map(Signer::pubkey),
                    system_program: system_program::ID,
                },
                initialization,
            ),
            vec![&fixture.initializer],
        ),
        (
            "tag2-create-proposal",
            create_primary_instruction(context, harness, &proposal),
            vec![&harness.seats[1]],
        ),
        (
            "tag3-approve-proposal",
            approve_proposal_v2_instruction(
                harness.controller,
                ApproveProposalV2Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    council: harness.council,
                    protocol_gate: harness.gate,
                    proposal: fixture.proposal,
                    seat_authority: harness.seats[0].pubkey(),
                },
                ApproveProposalV2 {
                    expected: proposal_expected,
                    expected_approval_bitset: 0,
                    expected_approval_count: 0,
                },
            ),
            vec![&harness.seats[0]],
        ),
        (
            "tag4-finalize-governance",
            finalize_governance_v2_instruction(
                harness.controller,
                FinalizeGovernanceV2Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    council: harness.council,
                    protocol_gate: harness.gate,
                    proposal: fixture.proposal,
                },
                FinalizeGovernanceV2 {
                    expected: proposal_expected,
                    expected_approval_bitset: 7,
                    expected_approval_count: 3,
                },
            ),
            vec![],
        ),
        (
            "tag5-queue-proposal",
            queue_proposal_v2_instruction(
                harness.controller,
                QueueProposalV2Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    protocol_gate: harness.gate,
                    proposal: fixture.proposal,
                },
                QueueProposalV2 {
                    expected: proposal_expected,
                },
            ),
            vec![],
        ),
        (
            "tag6-freeze-proposal",
            freeze_proposal_v2_instruction(
                harness.controller,
                FreezeProposalV2Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    council: harness.council,
                    protocol_gate: harness.gate,
                    proposal: fixture.proposal,
                    target_program: harness.target,
                    target_programdata: harness.target_programdata,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                    authority_pda: harness.authority,
                    rollback_proposal: fixture.baseline,
                    rollback_buffer_verification: fixture.phase_evidence,
                    rollback_buffer: harness.rollback_buffer,
                },
                FreezeProposalV2 {
                    expected: proposal_expected,
                    expected_next_gate_epoch: gate.epoch + 1,
                },
            ),
            vec![],
        ),
        (
            "tag7-cancel-proposal",
            cancel_proposal_v2_instruction(
                harness.controller,
                CancelProposalV2Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    council: harness.council,
                    protocol_gate: harness.gate,
                    proposal: fixture.proposal,
                    seat_authority: harness.seats[0].pubkey(),
                },
                CancelProposalV2 {
                    expected: proposal_expected,
                    expected_cancellation_approval_bitset: 0,
                    expected_cancellation_approval_count: 0,
                    cancellation_reason_code: 1,
                },
            ),
            vec![&harness.seats[0]],
        ),
        (
            "tag8-expire-proposal",
            expire_proposal_v2_instruction(
                harness.controller,
                ExpireProposalV2Accounts {
                    controller_config: harness.config,
                    protocol_gate: harness.gate,
                    proposal: fixture.proposal,
                },
                ExpireProposalV2 {
                    expected: proposal_expected,
                },
            ),
            vec![],
        ),
        (
            "tag9-guardian-freeze",
            guardian_freeze_v1_instruction(
                harness.controller,
                GuardianFreezeV1Accounts {
                    payer,
                    controller_config: harness.config,
                    protocol_gate: harness.gate,
                    target_program: harness.target,
                    target_programdata: harness.target_programdata,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                    authority_pda: harness.authority,
                    guardian: harness.guardian.pubkey(),
                    emergency_freeze_observation: fixture.emergency_observation,
                    system_program: system_program::ID,
                },
                GuardianFreezeV1 {
                    expected_gate_status: GateStatusV1::Active,
                    expected_gate_epoch: gate.epoch,
                    expected_next_gate_epoch: gate.epoch + 1,
                    expected_target_nonce: config.target_nonce,
                    expected_program_owner: UPGRADEABLE_LOADER_ID,
                    expected_program_executable: true,
                    expected_program_data_length: LOADER_PROGRAM_ACCOUNT_LEN as u64,
                    expected_program_header_present: true,
                    expected_linked_programdata: observed_programdata,
                    expected_programdata_owner: UPGRADEABLE_LOADER_ID,
                    expected_programdata_executable: false,
                    expected_programdata_data_length: target_raw.len() as u64,
                    expected_programdata_header_present: true,
                    expected_programdata_slot: 7,
                    expected_raw_hash_complete: true,
                    expected_raw_programdata_hash: target_raw_hash,
                    expected_capacity: CURRENT_PAYLOAD.len() as u64,
                    expected_programdata_authority: observed_authority,
                    freeze_reason_code: 1,
                    expected_observation_digest: freeze_observation_digest,
                },
            ),
            vec![&harness.guardian],
        ),
        (
            "tag10-create-emergency-resolution",
            create_emergency_resolution_v1_instruction(
                harness.controller,
                CreateEmergencyResolutionV1Accounts {
                    payer,
                    controller_config: harness.config,
                    policy: harness.policy,
                    council: harness.council,
                    protocol_gate: harness.gate,
                    emergency_freeze_observation: fixture.emergency_observation,
                    emergency_resolution: fixture.emergency_resolution,
                    system_program: system_program::ID,
                },
                CreateEmergencyResolutionV1 {
                    resolution_kind: EmergencyFreezeResolutionKindV1::ResumeWithoutUpgrade,
                    creation_slot: ACTIVE_SLOT,
                    not_before_slot: ACTIVE_SLOT + config.routine_delay_slots,
                    expiry_slot: ACTIVE_SLOT + config.proposal_expiry_slots,
                    expected_policy_version: policy.version,
                    expected_policy_hash: policy.policy_hash,
                    expected_council_version: council.version,
                    expected_council_hash: council.set_hash,
                    expected_gate_epoch: gate.epoch,
                    expected_freeze_slot: gate.freeze_slot,
                    expected_freeze_reason_code: gate.freeze_reason_code,
                    expected_target_nonce: config.target_nonce,
                    expected_freeze_observation_digest: freeze_observation_digest,
                    observed_program_owner: UPGRADEABLE_LOADER_ID,
                    observed_program_executable: true,
                    observed_program_data_length: LOADER_PROGRAM_ACCOUNT_LEN as u64,
                    observed_program_header_present: true,
                    observed_linked_programdata: observed_programdata,
                    observed_programdata_owner: UPGRADEABLE_LOADER_ID,
                    observed_programdata_executable: false,
                    observed_programdata_data_length: target_raw.len() as u64,
                    observed_programdata_header_present: true,
                    observed_programdata_slot: 7,
                    observed_raw_hash_complete: true,
                    observed_raw_programdata_hash: target_raw_hash,
                    observed_capacity: CURRENT_PAYLOAD.len() as u64,
                    observed_programdata_authority: observed_authority,
                    expected_resolution_digest: [24; 32],
                },
            ),
            vec![],
        ),
        (
            "tag11-approve-emergency-resolution",
            approve_emergency_resolution_v1_instruction(
                harness.controller,
                ApproveEmergencyResolutionV1Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    council: harness.council,
                    protocol_gate: harness.gate,
                    emergency_resolution: fixture.emergency_resolution,
                    seat_authority: harness.seats[0].pubkey(),
                },
                ApproveEmergencyResolutionV1 {
                    expected: emergency_expected,
                    expected_approval_bitset: 0,
                    expected_approval_count: 0,
                },
            ),
            vec![&harness.seats[0]],
        ),
        (
            "tag12-queue-emergency-resolution",
            queue_emergency_resolution_v1_instruction(
                harness.controller,
                QueueEmergencyResolutionV1Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    council: harness.council,
                    protocol_gate: harness.gate,
                    emergency_resolution: fixture.emergency_resolution,
                },
                QueueEmergencyResolutionV1 {
                    expected: emergency_expected,
                    expected_approval_bitset: 7,
                    expected_approval_count: 3,
                },
            ),
            vec![],
        ),
        (
            "tag13-execute-emergency-resolution",
            execute_emergency_resolution_v1_instruction(
                harness.controller,
                ExecuteEmergencyResolutionV1Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    council: harness.council,
                    protocol_gate: harness.gate,
                    emergency_resolution: fixture.emergency_resolution,
                    emergency_freeze_observation: fixture.emergency_observation,
                    emergency_checkpoint: fixture.baseline,
                    target_program: harness.target,
                    target_programdata: harness.target_programdata,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                    authority_pda: harness.authority,
                    instructions_sysvar: solana_program::sysvar::instructions::ID,
                },
                ExecuteEmergencyResolutionV1 {
                    expected: emergency_expected,
                    expected_freeze_observation_digest: freeze_observation_digest,
                    expected_checkpoint_digest: checkpoint_digest,
                    expected_program_owner: UPGRADEABLE_LOADER_ID,
                    expected_program_executable: true,
                    expected_program_data_length: LOADER_PROGRAM_ACCOUNT_LEN as u64,
                    expected_program_header_present: true,
                    expected_linked_programdata: observed_programdata,
                    expected_programdata_owner: UPGRADEABLE_LOADER_ID,
                    expected_programdata_executable: false,
                    expected_programdata_data_length: target_raw.len() as u64,
                    expected_programdata_header_present: true,
                    expected_programdata_slot: 7,
                    expected_raw_hash_complete: true,
                    expected_raw_programdata_hash: target_raw_hash,
                    expected_capacity: CURRENT_PAYLOAD.len() as u64,
                    expected_programdata_authority: observed_authority,
                },
            ),
            vec![],
        ),
        (
            "tag14-convert-emergency-freeze",
            convert_emergency_freeze_v2_instruction(
                harness.controller,
                ConvertEmergencyFreezeV2Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    council: harness.council,
                    protocol_gate: harness.gate,
                    proposal: fixture.proposal,
                    emergency_freeze_observation: fixture.emergency_observation,
                    target_program: harness.target,
                    target_programdata: harness.target_programdata,
                    upgradeable_loader: UPGRADEABLE_LOADER_ID,
                    authority_pda: harness.authority,
                    rollback_proposal: fixture.baseline,
                    rollback_buffer_verification: fixture.phase_evidence,
                    rollback_buffer: harness.rollback_buffer,
                },
                ConvertEmergencyFreezeV2 {
                    expected: proposal_expected,
                    expected_next_gate_epoch: gate.epoch + 1,
                    expected_freeze_observation_digest: freeze_observation_digest,
                },
            ),
            vec![],
        ),
    ]
}

fn checkpoint_rotation_account_contract_instructions<'a>(
    context: &ProgramTestContext,
    harness: &'a Harness,
    fixture: &'a AccountContractFixture,
) -> Vec<(&'static str, Instruction, Vec<&'a Keypair>)> {
    let candidate = account_contract_candidate();
    let rotation = account_contract_rotation_expectation();
    let emergency = account_contract_emergency_expectation();
    let payer = context.payer.pubkey();
    let seat = harness.seats[0].pubkey();
    let candidate_authorities = harness.seats.each_ref().map(Signer::pubkey);
    vec![
        (
            "tag15-create-checkpoint-attestation",
            create_checkpoint_attestation_v1_instruction(
                harness.controller,
                CreateCheckpointAttestationV1Accounts {
                    payer,
                    controller_config: harness.config,
                    policy: harness.policy,
                    council: harness.council,
                    protocol_gate: harness.gate,
                    subject: fixture.subject,
                    checkpoint: fixture.checkpoint,
                    checkpoint_attestation: fixture.checkpoint_attestation,
                    seat_authority: seat,
                    system_program: system_program::ID,
                },
                CreateCheckpointAttestationV1 {
                    candidate,
                    expected_council_version: 1,
                    expected_council_hash: [13; 32],
                    seat_index: 0,
                },
            ),
            vec![&harness.seats[0]],
        ),
        (
            "tag16-recast-checkpoint-attestation",
            recast_checkpoint_attestation_v1_instruction(
                harness.controller,
                RecastCheckpointAttestationV1Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    council: harness.council,
                    protocol_gate: harness.gate,
                    subject: fixture.subject,
                    checkpoint: fixture.checkpoint,
                    checkpoint_attestation: fixture.checkpoint_attestation,
                    seat_authority: seat,
                },
                RecastCheckpointAttestationV1 {
                    candidate,
                    expected_council_version: 1,
                    expected_council_hash: [13; 32],
                    seat_index: 0,
                    expected_previous_attestation_digest: [18; 32],
                },
            ),
            vec![&harness.seats[0]],
        ),
        (
            "tag17-finalize-checkpoint",
            finalize_checkpoint_v1_instruction(
                harness.controller,
                FinalizeCheckpointV1Accounts {
                    payer,
                    controller_config: harness.config,
                    policy: harness.policy,
                    council: harness.council,
                    protocol_gate: harness.gate,
                    subject: fixture.subject,
                    target_program: harness.target,
                    target_programdata: harness.target_programdata,
                    phase_evidence: fixture.phase_evidence,
                    baseline_checkpoint: None,
                    checkpoint: fixture.checkpoint,
                    checkpoint_attestations: fixture.checkpoint_attestations,
                    system_program: system_program::ID,
                },
                FinalizeCheckpointV1 {
                    candidate,
                    expected_council_version: 1,
                    expected_council_hash: [13; 32],
                },
            ),
            vec![],
        ),
        (
            "tag18-create-candidate-council",
            create_candidate_council_set_v1_instruction(
                harness.controller,
                CreateCandidateCouncilSetV1Accounts {
                    payer,
                    creator_seat_authority: seat,
                    controller_config: harness.config,
                    policy: harness.policy,
                    current_council: harness.council,
                    protocol_gate: harness.gate,
                    candidate_council: fixture.candidate_council,
                    candidate_seat_authorities: candidate_authorities,
                    system_program: system_program::ID,
                },
                CreateCandidateCouncilSetV1 {
                    expected_current_council_version: 1,
                    expected_current_council_hash: [13; 32],
                    candidate_council_version: 2,
                    activation_slot: 60,
                    expected_target_nonce: 1,
                    expected_gate_status: GateStatusV1::Active,
                    expected_gate_epoch: 2,
                    expected_candidate_council_hash: [14; 32],
                    seat_terms: [CouncilSeatTermV1 {
                        term_start_slot: 1,
                        term_end_slot: 10_000,
                    }; 5],
                },
            ),
            vec![&harness.seats[0]],
        ),
        (
            "tag19-create-council-rotation",
            create_council_rotation_v1_instruction(
                harness.controller,
                CreateCouncilRotationV1Accounts {
                    payer,
                    creator_seat_authority: seat,
                    controller_config: harness.config,
                    policy: harness.policy,
                    current_council: harness.council,
                    candidate_council: fixture.candidate_council,
                    protocol_gate: harness.gate,
                    rotation: fixture.rotation,
                    system_program: system_program::ID,
                },
                CreateCouncilRotationV1 {
                    creation_slot: ACTIVE_SLOT,
                    not_before_slot: 60,
                    expiry_slot: 100,
                    expected_current_council_version: 1,
                    expected_current_council_hash: [13; 32],
                    expected_candidate_council_version: 2,
                    expected_candidate_council_hash: [14; 32],
                    expected_gate_status: GateStatusV1::Active,
                    expected_gate_epoch: 2,
                    expected_target_nonce: 1,
                    expected_rotation_digest: [12; 32],
                },
            ),
            vec![&harness.seats[0]],
        ),
        (
            "tag20-approve-council-rotation",
            approve_council_rotation_v1_instruction(
                harness.controller,
                ApproveCouncilRotationV1Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    current_council: harness.council,
                    candidate_council: fixture.candidate_council,
                    protocol_gate: harness.gate,
                    rotation: fixture.rotation,
                    seat_authority: seat,
                },
                ApproveCouncilRotationV1 {
                    expected: rotation,
                    expected_approval_bitset: 0,
                    expected_approval_count: 0,
                },
            ),
            vec![&harness.seats[0]],
        ),
        (
            "tag21-activate-council-rotation",
            activate_council_rotation_v1_instruction(
                harness.controller,
                ActivateCouncilRotationV1Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    current_council: harness.council,
                    candidate_council: fixture.candidate_council,
                    protocol_gate: harness.gate,
                    rotation: fixture.rotation,
                },
                ActivateCouncilRotationV1 {
                    expected: rotation,
                    expected_approval_bitset: 7,
                    expected_approval_count: 3,
                },
            ),
            vec![],
        ),
        (
            "tag22-queue-council-rotation",
            queue_council_rotation_v1_instruction(
                harness.controller,
                QueueCouncilRotationV1Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    current_council: harness.council,
                    candidate_council: fixture.candidate_council,
                    protocol_gate: harness.gate,
                    rotation: fixture.rotation,
                },
                QueueCouncilRotationV1 {
                    expected: rotation,
                    expected_approval_bitset: 7,
                    expected_approval_count: 3,
                },
            ),
            vec![],
        ),
        (
            "tag23-expire-emergency-resolution",
            expire_emergency_resolution_v1_instruction(
                harness.controller,
                ExpireEmergencyResolutionV1Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    protocol_gate: harness.gate,
                    emergency_resolution: fixture.emergency_resolution,
                },
                ExpireEmergencyResolutionV1 {
                    expected: emergency,
                },
            ),
            vec![],
        ),
        (
            "tag24-cancel-council-rotation",
            cancel_council_rotation_v1_instruction(
                harness.controller,
                CancelCouncilRotationV1Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    current_council: harness.council,
                    candidate_council: fixture.candidate_council,
                    protocol_gate: harness.gate,
                    rotation: fixture.rotation,
                    seat_authority: seat,
                },
                CancelCouncilRotationV1 {
                    expected: rotation,
                    expected_cancellation_approval_bitset: 0,
                    expected_cancellation_approval_count: 0,
                    cancellation_reason_code: 1,
                },
            ),
            vec![&harness.seats[0]],
        ),
        (
            "tag25-expire-council-rotation",
            expire_council_rotation_v1_instruction(
                harness.controller,
                ExpireCouncilRotationV1Accounts {
                    controller_config: harness.config,
                    protocol_gate: harness.gate,
                    rotation: fixture.rotation,
                },
                ExpireCouncilRotationV1 { expected: rotation },
            ),
            vec![],
        ),
    ]
}

fn install_typed_account_contract_states(
    context: &mut ProgramTestContext,
    harness: &Harness,
    fixture: &AccountContractFixture,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
    council: &GovernanceCouncilSetV1,
    gate: &ProtocolGateV1,
) -> Vec<Pubkey> {
    let rollback_hash = hashv(&[ROLLBACK_ARTIFACT]).to_bytes();
    let rollback_root =
        artifact_merkle_root(ROLLBACK_ARTIFACT, RELEASE1_ARTIFACT_CHUNK_SIZE_V1).unwrap();
    let proposal = proposal_template(
        harness,
        config,
        policy,
        council,
        gate,
        ProposalTemplateInput {
            proposal_id: config.next_proposal_id,
            creation_slot: ACTIVE_SLOT,
            buffer: harness.buffer,
            uploader: harness.uploader,
            artifact: CANDIDATE_ARTIFACT,
            class: ProposalClassV1::RoutineUpgrade,
            primary: OptionalPubkeyV1::none(),
            rollback: OptionalPubkeyV1::some(fixture.baseline).unwrap(),
            rollback_buffer: OptionalPubkeyV1::some(harness.rollback_buffer).unwrap(),
            rollback_hash,
            rollback_root,
        },
    );
    set_account(
        context,
        fixture.proposal,
        state_account(harness.controller, &proposal),
    );
    set_account(
        context,
        fixture.subject,
        state_account(harness.controller, &proposal),
    );
    set_account(
        context,
        fixture.baseline,
        state_account(harness.controller, &proposal),
    );

    let candidate = prestate_candidate(
        harness,
        fixture.proposal,
        &proposal,
        council,
        gate,
        ACTIVE_SLOT,
    );
    let checkpoint = StateCheckpointV1 {
        discriminator: STATE_CHECKPOINT_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_checkpoint_pda(
            &harness.controller,
            &fixture.proposal,
            CheckpointPhaseV1::Prestate,
        )
        .1,
        initialized: true,
        phase: candidate.phase,
        controller_config: harness.config,
        proposal: fixture.proposal,
        emergency_resolution: Pubkey::default(),
        subject_digest: candidate.expected_subject_digest,
        target_program: harness.target,
        target_programdata: harness.target_programdata,
        finalized_observation_slot: candidate.finalized_observation_slot,
        gate_epoch: candidate.expected_gate_epoch,
        target_programdata_slot: candidate.target_programdata_slot,
        target_payload_commitment: candidate.target_payload_commitment,
        target_raw_programdata_commitment: candidate.target_raw_programdata_commitment,
        target_capacity: candidate.target_capacity,
        program_owned_state_root: candidate.program_owned_state_root,
        program_owned_state_count: candidate.program_owned_state_count,
        logical_compressed_state_root: candidate.logical_compressed_state_root,
        logical_compressed_state_count: candidate.logical_compressed_state_count,
        semantic_custody_accounting_root: candidate.semantic_custody_accounting_root,
        hard_combined_root: candidate.hard_combined_root,
        external_metadata_observation_root: candidate.external_metadata_observation_root,
        external_raw_balance_observation_root: candidate.external_raw_balance_observation_root,
        schema_identifier: candidate.schema_identifier,
        admitted_positive_donation_root: candidate.admitted_positive_donation_root,
        admitted_positive_donation_count: candidate.admitted_positive_donation_count,
        forbidden_drift_count: candidate.forbidden_drift_count,
        approval_council_version: council.version,
        approval_council_hash: council.set_hash,
        checkpoint_digest: candidate.expected_checkpoint_digest,
        approval_bitset: 0b00111,
        approval_count: 3,
        accepted: true,
        finalized_slot: ACTIVE_SLOT,
        reserved: [0; STATE_CHECKPOINT_V1_RESERVED_LEN],
    };
    checkpoint
        .validate_schema()
        .expect("typed checkpoint fixture");
    set_account(
        context,
        fixture.checkpoint,
        state_account(harness.controller, &checkpoint),
    );

    let mut attestation_keys = fixture.checkpoint_attestations.to_vec();
    attestation_keys.push(fixture.checkpoint_attestation);
    for (ordinal, key) in attestation_keys.iter().enumerate() {
        let seat_index = (ordinal % 3) as u8;
        let mut attestation = CheckpointAttestationV1 {
            discriminator: CHECKPOINT_ATTESTATION_V1_DISCRIMINATOR,
            account_version: RELEASE1_ACCOUNT_VERSION_V1,
            bump: derive_checkpoint_attestation_pda(
                &harness.controller,
                &fixture.checkpoint,
                council.version,
                seat_index,
            )
            .1,
            initialized: true,
            controller_program: harness.controller,
            controller_config: harness.config,
            checkpoint: fixture.checkpoint,
            subject: fixture.proposal,
            subject_digest: proposal.proposal_digest,
            phase: StateCheckpointPhaseV1::Prestate,
            checkpoint_digest: checkpoint.checkpoint_digest,
            council: harness.council,
            council_version: council.version,
            council_hash: council.set_hash,
            gate_epoch: gate.epoch,
            seat_index,
            seat_authority: harness.seats[seat_index as usize].pubkey(),
            attested_slot: ACTIVE_SLOT,
            attestation_digest: [0; 32],
            reserved: [0; CHECKPOINT_ATTESTATION_V1_RESERVED_LEN],
        };
        attestation.attestation_digest =
            compute_checkpoint_attestation_digest_v1(&attestation).unwrap();
        attestation
            .validate_schema()
            .expect("typed checkpoint attestation fixture");
        set_account(
            context,
            *key,
            state_account(harness.controller, &attestation),
        );
    }

    let frozen_epoch = gate.epoch;
    let mut freeze_observation = EmergencyFreezeObservationV1 {
        discriminator: EMERGENCY_FREEZE_OBSERVATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_emergency_freeze_observation_pda(
            &harness.controller,
            &harness.target,
            frozen_epoch,
        )
        .1,
        initialized: true,
        finalized: true,
        controller_program: harness.controller,
        controller_config: harness.config,
        protocol_gate: harness.gate,
        target_program: harness.target,
        target_programdata: harness.target_programdata,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        controller_authority: harness.authority,
        frozen_epoch,
        freeze_slot: INITIAL_SLOT,
        freeze_reason_code: 77,
        actual_program_owner: UPGRADEABLE_LOADER_ID,
        actual_program_executable: true,
        actual_program_data_length: LOADER_PROGRAM_ACCOUNT_LEN as u64,
        program_header_present: true,
        actual_linked_programdata: OptionalPubkeyV1::some(harness.target_programdata).unwrap(),
        actual_programdata_owner: UPGRADEABLE_LOADER_ID,
        actual_programdata_executable: false,
        actual_programdata_data_length: (LOADER_PROGRAMDATA_METADATA_LEN + CURRENT_PAYLOAD.len())
            as u64,
        programdata_header_present: true,
        deployed_programdata_slot: 7,
        raw_hash_complete: true,
        raw_programdata_sha256: loader_account_data_hash(&programdata_bytes(
            7,
            harness.authority,
            CURRENT_PAYLOAD,
        )),
        capacity: CURRENT_PAYLOAD.len() as u64,
        observed_authority: OptionalPubkeyV1::some(harness.authority).unwrap(),
        observation_digest: [0; 32],
        finalized_slot: INITIAL_SLOT,
        reserved: [0; EMERGENCY_FREEZE_OBSERVATION_V1_RESERVED_LEN],
    };
    freeze_observation.observation_digest =
        compute_emergency_freeze_observation_digest_v1(&freeze_observation).unwrap();
    freeze_observation
        .validate_schema()
        .expect("typed emergency observation fixture");
    set_account(
        context,
        fixture.emergency_observation,
        state_account(harness.controller, &freeze_observation),
    );

    let raw_programdata = programdata_bytes(7, harness.authority, CURRENT_PAYLOAD);
    let mut resolution = EmergencyFreezeResolutionV1 {
        discriminator: EMERGENCY_FREEZE_RESOLUTION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_emergency_resolution_pda(&harness.controller, &harness.target, frozen_epoch).1,
        initialized: true,
        state: EmergencyFreezeResolutionStateV1::Timelocked,
        controller_config: harness.config,
        protocol_gate: harness.gate,
        target_program: harness.target,
        target_programdata: harness.target_programdata,
        emergency_freeze_observation: fixture.emergency_observation,
        frozen_epoch,
        freeze_slot: INITIAL_SLOT,
        freeze_reason_code: 77,
        resolution_kind: EmergencyFreezeResolutionKindV1::ResumeWithoutUpgrade,
        creation_slot: INITIAL_SLOT + 1,
        not_before_slot: ACTIVE_SLOT,
        expiry_slot: ACTIVE_SLOT + config.proposal_expiry_slots,
        target_nonce: config.target_nonce,
        observed_program_owner: UPGRADEABLE_LOADER_ID,
        observed_program_executable: true,
        observed_program_data_length: LOADER_PROGRAM_ACCOUNT_LEN as u64,
        observed_program_header_present: true,
        observed_linked_programdata: OptionalPubkeyV1::some(harness.target_programdata).unwrap(),
        observed_programdata_owner: UPGRADEABLE_LOADER_ID,
        observed_programdata_executable: false,
        observed_programdata_data_length: raw_programdata.len() as u64,
        observed_programdata_header_present: true,
        observed_programdata_slot: 7,
        observed_raw_hash_complete: true,
        observed_raw_programdata_hash: loader_account_data_hash(&raw_programdata),
        observed_capacity: CURRENT_PAYLOAD.len() as u64,
        observed_authority: OptionalPubkeyV1::some(harness.authority).unwrap(),
        emergency_checkpoint: fixture.checkpoint,
        approval_council_version: council.version,
        approval_council_hash: council.set_hash,
        approval_bitset: 0b00111,
        approval_count: 3,
        resolution_digest: [0; 32],
        executed_slot: 0,
        cancellation_reason_code: 0,
        terminal_reason_code: 0,
        reserved: [0; EMERGENCY_FREEZE_RESOLUTION_V1_RESERVED_LEN],
    };
    resolution.resolution_digest = compute_emergency_resolution_digest_v1(&resolution).unwrap();
    resolution
        .validate_schema()
        .expect("typed emergency resolution fixture");
    set_account(
        context,
        fixture.emergency_resolution,
        state_account(harness.controller, &resolution),
    );

    let candidate_council =
        candidate_council_fixture(context, harness, config, council, gate, 2, ACTIVE_SLOT);
    assert_eq!(candidate_council.key, fixture.candidate_council);
    set_account(
        context,
        fixture.candidate_council,
        state_account(harness.controller, &candidate_council.value),
    );
    let rotation = rotation_fixture(
        context,
        harness,
        config,
        council,
        &candidate_council,
        gate,
        ACTIVE_SLOT,
    );
    assert_eq!(rotation.key, fixture.rotation);
    set_account(
        context,
        fixture.rotation,
        state_account(harness.controller, &rotation.value),
    );

    let buffer_verification = verification(harness, fixture.proposal, &proposal, ACTIVE_SLOT);
    set_account(
        context,
        fixture.phase_evidence,
        state_account(harness.controller, &buffer_verification),
    );

    let mut keys = vec![
        harness.config,
        harness.policy,
        harness.council,
        harness.gate,
        fixture.proposal,
        fixture.subject,
        fixture.baseline,
        fixture.checkpoint,
        fixture.checkpoint_attestation,
        fixture.emergency_observation,
        fixture.emergency_resolution,
        fixture.candidate_council,
        fixture.rotation,
        fixture.phase_evidence,
    ];
    keys.extend(fixture.checkpoint_attestations);
    keys
}

async fn release1_tags_1_through_25_account_and_privilege_matrix_is_failure_atomic() {
    let (mut context, harness) = start_harness(None).await;
    activate_for_test(&mut context, &harness).await;
    let canonical_config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let policy: GovernancePolicyV1 = state(&mut context, harness.policy).await;
    let council: GovernanceCouncilSetV1 = state(&mut context, harness.council).await;
    let gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let fixture = install_account_contract_fixture(&mut context, &harness, &canonical_config);
    let typed_keys = install_typed_account_contract_states(
        &mut context,
        &harness,
        &fixture,
        &canonical_config,
        &policy,
        &council,
        &gate,
    );

    let alternate_target = Pubkey::new_unique();
    let alternate_programdata = Pubkey::new_unique();
    let alternate_loader = Pubkey::new_unique();
    let alternate_authority = Pubkey::new_unique();
    let alternate_spill = Pubkey::new_unique();
    let alternate_buffer = Pubkey::new_unique();
    let alternate_gate = Pubkey::new_unique();
    let alternate_policy = Pubkey::new_unique();
    let alternate_council = Pubkey::new_unique();
    let alternate_proposal = Pubkey::new_unique();
    let alternate_checkpoint = Pubkey::new_unique();
    let alternate_sysvar = Pubkey::new_unique();
    set_account(
        &mut context,
        alternate_target,
        account(
            UPGRADEABLE_LOADER_ID,
            program_bytes(alternate_programdata),
            false,
        ),
    );
    set_account(
        &mut context,
        alternate_programdata,
        account(
            UPGRADEABLE_LOADER_ID,
            programdata_bytes(7, alternate_authority, CURRENT_PAYLOAD),
            false,
        ),
    );
    set_account(
        &mut context,
        alternate_loader,
        account(native_loader::ID, Vec::new(), false),
    );
    for key in [alternate_authority, alternate_spill, alternate_sysvar] {
        set_account(&mut context, key, system_account());
    }
    set_account(
        &mut context,
        alternate_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(harness.uploader, CANDIDATE_ARTIFACT),
            false,
        ),
    );
    set_account(
        &mut context,
        alternate_gate,
        state_account(harness.controller, &gate),
    );
    set_account(
        &mut context,
        alternate_policy,
        state_account(harness.controller, &policy),
    );
    set_account(
        &mut context,
        alternate_council,
        state_account(harness.controller, &council),
    );
    let proposal_account = context
        .banks_client
        .get_account(fixture.proposal)
        .await
        .unwrap()
        .unwrap();
    let checkpoint_account = context
        .banks_client
        .get_account(fixture.checkpoint)
        .await
        .unwrap()
        .unwrap();
    set_account(&mut context, alternate_proposal, proposal_account);
    set_account(&mut context, alternate_checkpoint, checkpoint_account);

    let identity_alternates = [
        (harness.config, fixture.alternate_config),
        (harness.target, alternate_target),
        (harness.target_programdata, alternate_programdata),
        (UPGRADEABLE_LOADER_ID, alternate_loader),
        (harness.authority, alternate_authority),
        (harness.spill, alternate_spill),
        (harness.buffer, alternate_buffer),
        (harness.rollback_buffer, alternate_buffer),
        (harness.gate, alternate_gate),
        (harness.policy, alternate_policy),
        (harness.council, alternate_council),
        (fixture.proposal, alternate_proposal),
        (fixture.subject, alternate_proposal),
        (fixture.baseline, alternate_checkpoint),
        (fixture.checkpoint, alternate_checkpoint),
        (solana_program::sysvar::instructions::ID, alternate_sysvar),
    ];

    let mut contracts = release1_state_account_contract_instructions(
        &context,
        &harness,
        &fixture,
        &canonical_config,
        &policy,
        &council,
        &gate,
    );
    contracts.extend(checkpoint_rotation_account_contract_instructions(
        &context, &harness, &fixture,
    ));
    let payer_key = context.payer.pubkey();
    for (name, valid_shape, signers) in contracts {
        eprintln!("account-contract matrix: {name}");
        let writable = valid_shape
            .accounts
            .iter()
            .filter(|meta| meta.is_writable)
            .map(|meta| meta.pubkey)
            .collect::<Vec<_>>();

        // Wrong count while retaining every required signer.
        let mut wrong_count = valid_shape.clone();
        let removable = wrong_count
            .accounts
            .iter()
            .position(|meta| !meta.is_signer && !meta.is_writable)
            .expect("every Release 1 account contract has a readonly role");
        wrong_count.accounts.remove(removable);
        assert_atomic_failure(&mut context, wrong_count, &signers, &writable).await;

        let mut extra_account = valid_shape.clone();
        extra_account
            .accounts
            .push(AccountMeta::new_readonly(alternate_authority, false));
        assert_atomic_failure(&mut context, extra_account, &signers, &writable).await;

        // Wrong order is made unambiguous by exchanging roles with distinct
        // privilege contracts.
        let mut wrong_order = valid_shape.clone();
        let writable_index = wrong_order
            .accounts
            .iter()
            .position(|meta| meta.is_writable)
            .expect("writable state role");
        let readonly_index = wrong_order
            .accounts
            .iter()
            .position(|meta| {
                !meta.is_writable
                    && !meta.is_signer
                    && ![
                        harness.controller,
                        harness.target,
                        UPGRADEABLE_LOADER_ID,
                        system_program::ID,
                    ]
                    .contains(&meta.pubkey)
            })
            .expect("readonly state role");
        wrong_order.accounts.swap(writable_index, readonly_index);
        assert_atomic_failure(&mut context, wrong_order, &signers, &writable).await;

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
            assert_atomic_failure(&mut context, wrong_writable, &signers, &writable).await;
        }

        // A non-payer signer, where present, must retain runtime signer
        // privilege. Payer-only and permissionless contracts have no such
        // relevant seat-signer case.
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
            assert_atomic_failure(&mut context, wrong_signer, &remaining_signers, &writable).await;
        }

        // Exercise the executable bit at the canonical typed-account identity
        // through the production dispatcher.  This is deliberately direct:
        // BanksServer otherwise tries to ELF-load malformed state bytes before
        // the controller can reject them.
        let executable_index = valid_shape
            .accounts
            .iter()
            .position(|meta| typed_keys.contains(&meta.pubkey))
            .unwrap_or_else(|| panic!("{name} lacks a typed non-executable role"));
        assert_direct_executable_rejected_unchanged(
            &mut context,
            &valid_shape,
            executable_index,
            &signers,
            &writable,
        )
        .await;

        let readonly_indices = valid_shape
            .accounts
            .iter()
            .enumerate()
            .filter_map(|(index, meta)| {
                (!meta.is_writable
                    && !meta.is_signer
                    && ![
                        harness.controller,
                        harness.target,
                        UPGRADEABLE_LOADER_ID,
                        system_program::ID,
                    ]
                    .contains(&meta.pubkey))
                .then_some(index)
            })
            .collect::<Vec<_>>();
        assert!(readonly_indices.len() >= 2, "{name} lacks duplicate roles");
        let mut duplicate = valid_shape.clone();
        duplicate.accounts[readonly_indices[1]].pubkey =
            duplicate.accounts[readonly_indices[0]].pubkey;
        assert_atomic_failure(&mut context, duplicate, &signers, &writable).await;

        // Exercise every applicable canonical identity role, not only config.
        // Alternate accounts retain relevant owner/header data. Alternate
        // Program and Loader identities remain non-executable at the bank
        // boundary because Agave tries to ELF-load any malformed executable
        // fixture before controller code runs; exact executable rejection is
        // covered at every instruction through the canonical probe above.
        let mut exercised_identity = false;
        for (index, meta) in valid_shape.accounts.iter().enumerate() {
            let Some((_, alternate)) = identity_alternates
                .iter()
                .find(|(canonical, _)| *canonical == meta.pubkey)
            else {
                continue;
            };
            exercised_identity = true;
            assert_direct_identity_rejected_unchanged(
                &mut context,
                &valid_shape,
                index,
                *alternate,
                &signers,
                &writable,
            )
            .await;
        }
        assert!(exercised_identity, "{name} lacks an identity-bound role");
    }

    // Every controller-owned typed account participating in the matrix is
    // independently corrupted at every fixed header boundary.  Cases are
    // filtered by account membership so each assertion proves the loader that
    // actually consumes that type rejects owner/size/discriminator/version/
    // bump drift without mutating any writable account. Executable drift is
    // covered for every instruction above without asking ProgramCache to load
    // malformed governance-state bytes as an ELF.
    let mut contracts = release1_state_account_contract_instructions(
        &context,
        &harness,
        &fixture,
        &canonical_config,
        &policy,
        &council,
        &gate,
    );
    contracts.extend(checkpoint_rotation_account_contract_instructions(
        &context, &harness, &fixture,
    ));
    for typed_key in typed_keys {
        let canonical_account = context
            .banks_client
            .get_account(typed_key)
            .await
            .unwrap()
            .unwrap();
        for (corruption, corrupted_account) in typed_account_header_corruptions(&canonical_account)
        {
            set_account(&mut context, typed_key, corrupted_account);
            let (name, instruction, signers) = contracts
                .iter()
                .find(|(_, instruction, _)| {
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
            assert_atomic_failure(&mut context, instruction.clone(), signers, &writable).await;
            let _ = (corruption, name);
            set_account(&mut context, typed_key, canonical_account.clone());
        }
    }
}

fn expected_policy(controller: Pubkey, config: Pubkey, target: Pubkey) -> GovernancePolicyV1 {
    let mut policy = GovernancePolicyV1 {
        discriminator: GOVERNANCE_POLICY_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump: derive_policy_pda(&controller, &target, 1).1,
        initialized: true,
        controller_config: config,
        version: 1,
        target_program: target,
        activation_slot: INITIAL_SLOT,
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

fn expected_council(
    controller: Pubkey,
    config: Pubkey,
    target: Pubkey,
    authorities: [Pubkey; 5],
) -> GovernanceCouncilSetV1 {
    let mut council = GovernanceCouncilSetV1 {
        discriminator: GOVERNANCE_COUNCIL_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump: derive_council_pda(&controller, &target, 1).1,
        initialized: true,
        controller_config: config,
        version: 1,
        target_program: target,
        activation_slot: INITIAL_SLOT,
        deactivation_slot: 0,
        seats: authorities.map(|seat_authority| CouncilSeatV1 {
            seat_authority,
            term_start_slot: 1,
            term_end_slot: 10_000,
            active: true,
            reserved: [0; COUNCIL_SEAT_RESERVED_LEN],
        }),
        routine_threshold: 3,
        terminal_threshold: 4,
        policy_flags: 0,
        set_hash: [0; 32],
        reserved: [0; GOVERNANCE_COUNCIL_RESERVED_LEN],
    };
    council.set_hash = compute_council_set_hash(&council);
    council
}

async fn start_harness(proxy: Option<(Pubkey, Pubkey)>) -> (ProgramTestContext, Harness) {
    let controller = TEST_CONTROLLER_ID;
    let target = TEST_TARGET_ID;
    let controller_programdata = derive_upgradeable_programdata_address(&controller).0;
    let target_programdata = derive_upgradeable_programdata_address(&target).0;
    let old_target_authority = Pubkey::new_unique();
    let initializer = Keypair::new();
    let guardian = Keypair::new();
    let seats = std::array::from_fn(|_| Keypair::new());
    let mut authorities = seats.each_ref().map(Signer::pubkey);
    if let Some((proxy_seat, _)) = proxy {
        authorities[0] = proxy_seat;
    }
    let config = derive_controller_config_pda(&controller, &target).0;
    let authority = derive_authority_pda(&controller, &target).0;
    let gate = derive_gate_pda(&controller, &target).0;
    let policy = derive_policy_pda(&controller, &target, 1).0;
    let council = derive_council_pda(&controller, &target, 1).0;
    let spill = Pubkey::new_unique();
    let buffer = Pubkey::new_unique();
    let uploader = Pubkey::new_unique();
    let rollback_buffer = Pubkey::new_unique();
    let rollback_uploader = Pubkey::new_unique();

    let mut test = ProgramTest::default();
    test.prefer_bpf(false);
    test.add_program(
        "upgrade_controller",
        controller,
        processor!(native_controller_adapter),
    );
    test.add_program(
        "executable_privilege_probe",
        EXECUTABLE_PRIVILEGE_PROBE_ID,
        processor!(executable_privilege_probe),
    );
    if let Some((_, proxy_program)) = proxy {
        test.add_program(
            "smart_account_proxy",
            proxy_program,
            processor!(proxy_process_instruction),
        );
    }
    test.add_account(
        controller_programdata,
        account(
            UPGRADEABLE_LOADER_ID,
            programdata_bytes(2, initializer.pubkey(), b"controller"),
            false,
        ),
    );
    test.add_account(target, system_account());
    test.add_account(
        target_programdata,
        account(
            UPGRADEABLE_LOADER_ID,
            programdata_bytes(7, old_target_authority, CURRENT_PAYLOAD),
            false,
        ),
    );
    for key in [
        initializer.pubkey(),
        guardian.pubkey(),
        old_target_authority,
        spill,
        uploader,
        rollback_uploader,
    ] {
        test.add_account(key, system_account());
    }
    for authority_key in authorities {
        let owner = proxy
            .filter(|(proxy_seat, _)| *proxy_seat == authority_key)
            .map_or(system_program::ID, |(_, proxy_program)| proxy_program);
        test.add_account(authority_key, account(owner, Vec::new(), false));
    }
    test.add_account(
        buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(uploader, CANDIDATE_ARTIFACT),
            false,
        ),
    );
    test.add_account(
        rollback_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(rollback_uploader, ROLLBACK_ARTIFACT),
            false,
        ),
    );
    let mut context = test.start_with_context().await;
    set_clock_slot(&mut context, INITIAL_SLOT).await;

    let expected_policy = expected_policy(controller, config, target);
    let expected_council = expected_council(controller, config, target, authorities);
    let init = InitializeControllerV1 {
        cluster_domain: [1; 32],
        initial_policy_version: 1,
        initial_council_version: 1,
        next_proposal_id: 1,
        target_nonce: 1,
        initial_gate_epoch: 1,
        policy_activation_slot: INITIAL_SLOT,
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
    };
    let init_ix = initialize_controller_v1_instruction(
        controller,
        InitializeControllerV1Accounts {
            payer: context.payer.pubkey(),
            initializer: initializer.pubkey(),
            controller_program: controller,
            controller_programdata,
            target_program: target,
            target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            controller_config: config,
            authority_pda: authority,
            protocol_gate: gate,
            policy,
            council,
            canonical_spill_treasury: spill,
            guardian: guardian.pubkey(),
            seat_authorities: authorities,
            system_program: system_program::ID,
        },
        init,
    );
    submit(
        &mut context,
        std::slice::from_ref(&init_ix),
        &[&initializer],
    )
    .await
    .unwrap();
    let cfg: ControllerConfigV1 = state(&mut context, config).await;
    let bootstrap_gate: ProtocolGateV1 = state(&mut context, gate).await;
    assert_eq!((cfg.next_proposal_id, cfg.target_nonce), (1, 1));
    assert!(!cfg.token_governance_enabled);
    assert_eq!(bootstrap_gate.status, GateStatusV1::EmergencyFrozen);

    let before = [
        bytes(&mut context, config).await,
        bytes(&mut context, gate).await,
        bytes(&mut context, policy).await,
        bytes(&mut context, council).await,
    ];
    assert!(submit(&mut context, &[init_ix], &[&initializer])
        .await
        .is_err());
    assert_eq!(
        before,
        [
            bytes(&mut context, config).await,
            bytes(&mut context, gate).await,
            bytes(&mut context, policy).await,
            bytes(&mut context, council).await,
        ]
    );

    (
        context,
        Harness {
            controller,
            target,
            target_programdata,
            guardian,
            seats,
            config,
            authority,
            gate,
            policy,
            council,
            spill,
            buffer,
            uploader,
            rollback_buffer,
            rollback_uploader,
        },
    )
}

async fn activate_for_test(context: &mut ProgramTestContext, harness: &Harness) {
    set_clock_slot(context, ACTIVE_SLOT).await;
    let mut gate: ProtocolGateV1 = state(context, harness.gate).await;
    gate.status = GateStatusV1::Active;
    gate.epoch = 2;
    gate.active_proposal = Pubkey::default();
    gate.freeze_slot = 0;
    gate.freeze_reason_code = 0;
    set_account(
        context,
        harness.gate,
        state_account(harness.controller, &gate),
    );
    set_account(
        context,
        harness.target_programdata,
        account(
            UPGRADEABLE_LOADER_ID,
            programdata_bytes(7, harness.authority, CURRENT_PAYLOAD),
            false,
        ),
    );
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

struct ProposalTemplateInput<'a> {
    proposal_id: u64,
    creation_slot: u64,
    buffer: Pubkey,
    uploader: Pubkey,
    artifact: &'a [u8],
    class: ProposalClassV1,
    primary: OptionalPubkeyV1,
    rollback: OptionalPubkeyV1,
    rollback_buffer: OptionalPubkeyV1,
    rollback_hash: [u8; 32],
    rollback_root: [u8; 32],
}

fn proposal_template(
    harness: &Harness,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
    council: &GovernanceCouncilSetV1,
    gate: &ProtocolGateV1,
    input: ProposalTemplateInput<'_>,
) -> UpgradeProposalV2 {
    let ProposalTemplateInput {
        proposal_id,
        creation_slot,
        buffer,
        uploader,
        artifact,
        class,
        primary,
        rollback,
        rollback_buffer,
        rollback_hash,
        rollback_root,
    } = input;
    let (proposal_key, bump) =
        derive_proposal_pda(&harness.controller, &harness.target, proposal_id);
    let artifact_sha256 = hashv(&[artifact]).to_bytes();
    let artifact_root = artifact_merkle_root(artifact, RELEASE1_ARTIFACT_CHUNK_SIZE_V1).unwrap();
    let rollback_class = class == ProposalClassV1::EmergencyRollback;
    let review_start_slot = creation_slot + 1;
    let review_end_slot = review_start_slot + config.vote_review_slots;
    let delay = if rollback_class {
        config.rollback_delay_slots
    } else {
        config.routine_delay_slots
    };
    let mut value = UpgradeProposalV2 {
        discriminator: UPGRADE_PROPOSAL_V2_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V2,
        bump,
        initialized: true,
        proposal_class: class,
        state: ProposalStateV2::Draft,
        creation_gate_status: gate.status,
        zero_tail_required: true,
        proposal_flags: 0,
        proposal_id,
        target_nonce: config.target_nonce,
        creation_slot,
        cluster_domain: config.cluster_domain,
        controller_program: harness.controller,
        controller_config: harness.config,
        protocol_gate: harness.gate,
        policy_version: policy.version,
        policy_hash: policy.policy_hash,
        creation_council_version: council.version,
        creation_council_hash: council.set_hash,
        creation_gate_epoch: gate.epoch,
        freeze_gate_epoch: 0,
        target_program: harness.target,
        target_programdata: harness.target_programdata,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        authority_pda: harness.authority,
        canonical_spill_treasury: harness.spill,
        buffer_pubkey: buffer,
        buffer_loader_owner: UPGRADEABLE_LOADER_ID,
        buffer_uploader_authority: uploader,
        buffer_final_authority: harness.authority,
        buffer_verification: derive_buffer_check_pda(&harness.controller, &proposal_key).0,
        programdata_verification: derive_programdata_check_pda(&harness.controller, &proposal_key)
            .0,
        artifact_length: artifact.len() as u64,
        artifact_sha256,
        artifact_chunk_merkle_root: artifact_root,
        chunk_hash_domain: ARTIFACT_MERKLE_SCHEME_ID,
        chunk_size: RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        chunk_count: artifact_chunk_count(artifact.len() as u64, RELEASE1_ARTIFACT_CHUNK_SIZE_V1)
            .unwrap(),
        source_commit_hash: [10; 32],
        source_tree_hash: [11; 32],
        build_input_inventory_hash: [12; 32],
        reproducible_build_receipt_hash: [13; 32],
        package_receipt_hash: [14; 32],
        release_intent_hash: [15; 32],
        expected_execution_pre_payload_hash: if rollback_class {
            hashv(&[CANDIDATE_ARTIFACT]).to_bytes()
        } else {
            hashv(&[CURRENT_PAYLOAD]).to_bytes()
        },
        expected_execution_pre_chunk_root: if rollback_class {
            artifact_merkle_root(CANDIDATE_ARTIFACT, RELEASE1_ARTIFACT_CHUNK_SIZE_V1).unwrap()
        } else {
            artifact_merkle_root(CURRENT_PAYLOAD, RELEASE1_ARTIFACT_CHUNK_SIZE_V1).unwrap()
        },
        current_raw_programdata_hash: if rollback_class {
            [0; 32]
        } else {
            loader_account_data_hash(&programdata_bytes(7, harness.authority, CURRENT_PAYLOAD))
        },
        deployed_slot: if rollback_class { 0 } else { 7 },
        current_capacity: CURRENT_PAYLOAD.len() as u64,
        extension_delta: 0,
        expected_post_capacity: CURRENT_PAYLOAD.len() as u64,
        prestate_checkpoint: derive_checkpoint_pda(
            &harness.controller,
            &proposal_key,
            CheckpointPhaseV1::Prestate,
        )
        .0,
        required_poststate_checkpoint: derive_checkpoint_pda(
            &harness.controller,
            &proposal_key,
            CheckpointPhaseV1::Poststate,
        )
        .0,
        checkpoint_schema_id: [20; 32],
        checkpoint_policy_hash: [21; 32],
        primary_proposal: primary,
        rollback_proposal: rollback,
        rollback_buffer,
        rollback_artifact_sha256: rollback_hash,
        rollback_artifact_chunk_root: rollback_root,
        vote_requirement: VoteRequirementV1::None,
        vote_program: Pubkey::default(),
        vote_result_pda: Pubkey::default(),
        review_start_slot,
        review_end_slot,
        not_before_slot: review_end_slot + delay,
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
        proposal_digest: [1; 32],
        cancellation_reason_code: 0,
        terminal_reason_code: 0,
        reserved: [0; UPGRADE_PROPOSAL_V2_RESERVED_LEN],
    };
    value.proposal_digest = compute_proposal_digest_v2(&value).unwrap();
    value
}

fn create_primary_instruction(
    context: &ProgramTestContext,
    harness: &Harness,
    proposal: &UpgradeProposalV2,
) -> Instruction {
    create_proposal_v2_instruction(
        harness.controller,
        CreateProposalV2Accounts {
            payer: context.payer.pubkey(),
            creator_seat_authority: harness.seats[1].pubkey(),
            controller_config: harness.config,
            policy: harness.policy,
            council: harness.council,
            protocol_gate: harness.gate,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            authority_pda: harness.authority,
            canonical_spill_treasury: harness.spill,
            buffer: proposal.buffer_pubkey,
            buffer_uploader_authority: proposal.buffer_uploader_authority,
            proposal: derive_proposal_pda(
                &harness.controller,
                &harness.target,
                proposal.proposal_id,
            )
            .0,
            system_program: system_program::ID,
        },
        CreateProposalV2 {
            proposal_class: proposal.proposal_class,
            creation_gate_status: proposal.creation_gate_status,
            expected_proposal_id: proposal.proposal_id,
            expected_target_nonce: proposal.target_nonce,
            creation_slot: proposal.creation_slot,
            expected_policy_version: proposal.policy_version,
            expected_policy_hash: proposal.policy_hash,
            expected_creation_council_version: proposal.creation_council_version,
            expected_creation_council_hash: proposal.creation_council_hash,
            expected_creation_gate_epoch: proposal.creation_gate_epoch,
            expected_freeze_gate_epoch: 0,
            artifact_length: proposal.artifact_length,
            artifact_sha256: proposal.artifact_sha256,
            artifact_chunk_merkle_root: proposal.artifact_chunk_merkle_root,
            source_commit_hash: proposal.source_commit_hash,
            source_tree_hash: proposal.source_tree_hash,
            build_input_inventory_hash: proposal.build_input_inventory_hash,
            reproducible_build_receipt_hash: proposal.reproducible_build_receipt_hash,
            package_receipt_hash: proposal.package_receipt_hash,
            release_intent_hash: proposal.release_intent_hash,
            expected_execution_pre_payload_hash: proposal.expected_execution_pre_payload_hash,
            expected_execution_pre_chunk_root: proposal.expected_execution_pre_chunk_root,
            current_raw_programdata_hash: proposal.current_raw_programdata_hash,
            deployed_slot: proposal.deployed_slot,
            current_capacity: proposal.current_capacity,
            extension_delta: proposal.extension_delta,
            expected_post_capacity: proposal.expected_post_capacity,
            checkpoint_schema_id: proposal.checkpoint_schema_id,
            checkpoint_policy_hash: proposal.checkpoint_policy_hash,
            primary_proposal: OptionalInstructionPubkeyV1::none(),
            rollback_proposal: OptionalInstructionPubkeyV1::some(proposal.rollback_proposal.value)
                .unwrap(),
            rollback_buffer: OptionalInstructionPubkeyV1::some(proposal.rollback_buffer.value)
                .unwrap(),
            rollback_artifact_sha256: proposal.rollback_artifact_sha256,
            rollback_artifact_chunk_root: proposal.rollback_artifact_chunk_root,
            review_start_slot: proposal.review_start_slot,
            review_end_slot: proposal.review_end_slot,
            not_before_slot: proposal.not_before_slot,
            expiry_slot: proposal.expiry_slot,
            expected_proposal_digest: proposal.proposal_digest,
        },
    )
}

async fn create_primary(
    context: &mut ProgramTestContext,
    harness: &Harness,
) -> (Pubkey, UpgradeProposalV2) {
    let config: ControllerConfigV1 = state(context, harness.config).await;
    let policy: GovernancePolicyV1 = state(context, harness.policy).await;
    let council: GovernanceCouncilSetV1 = state(context, harness.council).await;
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let rollback_key = derive_proposal_pda(&harness.controller, &harness.target, 2).0;
    let proposal = proposal_template(
        harness,
        &config,
        &policy,
        &council,
        &gate,
        ProposalTemplateInput {
            proposal_id: 1,
            creation_slot: current_slot(context).await,
            buffer: harness.buffer,
            uploader: harness.uploader,
            artifact: CANDIDATE_ARTIFACT,
            class: ProposalClassV1::RoutineUpgrade,
            primary: OptionalPubkeyV1::none(),
            rollback: OptionalPubkeyV1::some(rollback_key).unwrap(),
            rollback_buffer: OptionalPubkeyV1::some(harness.rollback_buffer).unwrap(),
            rollback_hash: hashv(&[ROLLBACK_ARTIFACT]).to_bytes(),
            rollback_root: artifact_merkle_root(ROLLBACK_ARTIFACT, RELEASE1_ARTIFACT_CHUNK_SIZE_V1)
                .unwrap(),
        },
    );
    let key = derive_proposal_pda(&harness.controller, &harness.target, 1).0;
    submit(
        context,
        &[create_primary_instruction(context, harness, &proposal)],
        &[&harness.seats[1]],
    )
    .await
    .unwrap();
    assert_eq!(state::<UpgradeProposalV2>(context, key).await, proposal);
    (key, proposal)
}

fn verification(
    harness: &Harness,
    proposal_key: Pubkey,
    proposal: &UpgradeProposalV2,
    finalized_slot: u64,
) -> BufferVerificationV1 {
    let mut bitmap = [0; VERIFICATION_BITMAP_BYTES_V1];
    bitmap[0] = 1;
    let header = buffer_bytes(
        harness.authority,
        &vec![0; proposal.artifact_length as usize],
    );
    BufferVerificationV1 {
        discriminator: BUFFER_VERIFICATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_buffer_check_pda(&harness.controller, &proposal_key).1,
        initialized: true,
        status: BufferVerificationStatusV1::Verified,
        controller_config: harness.config,
        proposal: proposal_key,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        buffer: proposal.buffer_pubkey,
        expected_uploader_authority: proposal.buffer_uploader_authority,
        controller_authority: harness.authority,
        artifact_length: proposal.artifact_length,
        artifact_sha256: proposal.artifact_sha256,
        artifact_chunk_merkle_root: proposal.artifact_chunk_merkle_root,
        chunk_hash_domain: proposal.chunk_hash_domain,
        chunk_size: proposal.chunk_size,
        chunk_count: proposal.chunk_count,
        verified_chunk_bitmap: bitmap,
        verified_chunk_count: proposal.chunk_count,
        adopted_slot: finalized_slot.saturating_sub(1),
        finalized_slot,
        sealed_buffer_header_hash: hashv(&[&header[..LOADER_BUFFER_METADATA_LEN]]).to_bytes(),
        terminal_slot: 0,
        reserved: [0; BUFFER_VERIFICATION_V1_RESERVED_LEN],
    }
}

/// Tags 27-29 are deliberately closed at Gate C. This injects only the state
/// they will have to produce, allowing the non-loader state machine to be
/// exercised without pretending that buffer adoption already passed Gate D.
async fn seed_sealed_buffers(
    context: &mut ProgramTestContext,
    harness: &Harness,
    primary_key: Pubkey,
) -> UpgradeProposalV2 {
    let mut primary: UpgradeProposalV2 = state(context, primary_key).await;
    primary.state = ProposalStateV2::BufferVerified;
    set_account(
        context,
        primary_key,
        state_account(harness.controller, &primary),
    );
    let now = current_slot(context).await;
    set_account(
        context,
        primary.buffer_verification,
        state_account(
            harness.controller,
            &verification(harness, primary_key, &primary, now),
        ),
    );

    let config: ControllerConfigV1 = state(context, harness.config).await;
    let policy: GovernancePolicyV1 = state(context, harness.policy).await;
    let council: GovernanceCouncilSetV1 = state(context, harness.council).await;
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    let rollback_key = primary.rollback_proposal.value;
    let mut rollback = proposal_template(
        harness,
        &config,
        &policy,
        &council,
        &gate,
        ProposalTemplateInput {
            proposal_id: 2,
            creation_slot: primary.creation_slot + config.rollback_delay_slots + 1,
            buffer: harness.rollback_buffer,
            uploader: harness.rollback_uploader,
            artifact: ROLLBACK_ARTIFACT,
            class: ProposalClassV1::EmergencyRollback,
            primary: OptionalPubkeyV1::some(primary_key).unwrap(),
            rollback: OptionalPubkeyV1::none(),
            rollback_buffer: OptionalPubkeyV1::none(),
            rollback_hash: [0; 32],
            rollback_root: [0; 32],
        },
    );
    rollback.state = ProposalStateV2::Timelocked;
    rollback.first_approval_slot = rollback.review_start_slot;
    rollback.council_approved_slot = rollback.review_start_slot;
    rollback.governance_satisfied_slot = rollback.review_start_slot;
    rollback.queued_slot = rollback.review_start_slot;
    rollback.council_approval_bitset = 0b00111;
    rollback.council_approval_count = 3;
    set_account(
        context,
        rollback_key,
        state_account(harness.controller, &rollback),
    );
    set_account(
        context,
        harness.rollback_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(harness.authority, ROLLBACK_ARTIFACT),
            false,
        ),
    );
    set_account(
        context,
        rollback.buffer_verification,
        state_account(
            harness.controller,
            &verification(harness, rollback_key, &rollback, now),
        ),
    );
    let mut config = config;
    config.next_proposal_id = 3;
    set_account(
        context,
        harness.config,
        state_account(harness.controller, &config),
    );
    primary
}

fn approve_instruction(
    harness: &Harness,
    key: Pubkey,
    proposal: &UpgradeProposalV2,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    seat: Pubkey,
) -> Instruction {
    approve_proposal_v2_instruction(
        harness.controller,
        ApproveProposalV2Accounts {
            controller_config: harness.config,
            policy: harness.policy,
            council: harness.council,
            protocol_gate: harness.gate,
            proposal: key,
            seat_authority: seat,
        },
        ApproveProposalV2 {
            expected: expectation(proposal, config, gate),
            expected_approval_bitset: proposal.council_approval_bitset,
            expected_approval_count: proposal.council_approval_count,
        },
    )
}

fn prestate_candidate(
    harness: &Harness,
    proposal_key: Pubkey,
    proposal: &UpgradeProposalV2,
    council: &GovernanceCouncilSetV1,
    gate: &ProtocolGateV1,
    observation_slot: u64,
) -> CheckpointCandidateV1 {
    let raw = loader_account_data_hash(&programdata_bytes(7, harness.authority, CURRENT_PAYLOAD));
    let payload = hashv(&[CURRENT_PAYLOAD]).to_bytes();
    let mut prototype = StateCheckpointV1 {
        discriminator: STATE_CHECKPOINT_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: derive_checkpoint_pda(
            &harness.controller,
            &proposal_key,
            CheckpointPhaseV1::Prestate,
        )
        .1,
        initialized: true,
        phase: StateCheckpointPhaseV1::Prestate,
        controller_config: harness.config,
        proposal: proposal_key,
        emergency_resolution: Pubkey::default(),
        subject_digest: proposal.proposal_digest,
        target_program: harness.target,
        target_programdata: harness.target_programdata,
        finalized_observation_slot: observation_slot,
        gate_epoch: gate.epoch,
        target_programdata_slot: 7,
        target_payload_commitment: payload,
        target_raw_programdata_commitment: raw,
        target_capacity: CURRENT_PAYLOAD.len() as u64,
        program_owned_state_root: [31; 32],
        program_owned_state_count: 5,
        logical_compressed_state_root: [32; 32],
        logical_compressed_state_count: 6,
        semantic_custody_accounting_root: [33; 32],
        hard_combined_root: [0; 32],
        external_metadata_observation_root: [34; 32],
        external_raw_balance_observation_root: [35; 32],
        schema_identifier: proposal.checkpoint_schema_id,
        admitted_positive_donation_root: [0; 32],
        admitted_positive_donation_count: 0,
        forbidden_drift_count: 0,
        approval_council_version: council.version,
        approval_council_hash: council.set_hash,
        checkpoint_digest: [1; 32],
        approval_bitset: 0b00111,
        approval_count: 3,
        accepted: true,
        finalized_slot: observation_slot,
        reserved: [0; STATE_CHECKPOINT_V1_RESERVED_LEN],
    };
    prototype.hard_combined_root =
        compute_state_checkpoint_hard_combined_root_v1(&prototype).unwrap();
    prototype.checkpoint_digest = compute_state_checkpoint_digest_v1(&prototype).unwrap();
    CheckpointCandidateV1 {
        phase: StateCheckpointPhaseV1::Prestate,
        expected_subject_state: CheckpointSubjectStateV1::ProposalFrozen,
        expected_subject_digest: proposal.proposal_digest,
        expected_gate_status: GateStatusV1::FrozenForUpgrade,
        expected_gate_epoch: gate.epoch,
        finalized_observation_slot: observation_slot,
        target_programdata_slot: 7,
        target_payload_commitment: payload,
        target_raw_programdata_commitment: raw,
        target_capacity: CURRENT_PAYLOAD.len() as u64,
        program_owned_state_root: prototype.program_owned_state_root,
        program_owned_state_count: prototype.program_owned_state_count,
        logical_compressed_state_root: prototype.logical_compressed_state_root,
        logical_compressed_state_count: prototype.logical_compressed_state_count,
        semantic_custody_accounting_root: prototype.semantic_custody_accounting_root,
        hard_combined_root: prototype.hard_combined_root,
        external_metadata_observation_root: prototype.external_metadata_observation_root,
        external_raw_balance_observation_root: prototype.external_raw_balance_observation_root,
        schema_identifier: prototype.schema_identifier,
        admitted_positive_donation_root: [0; 32],
        admitted_positive_donation_count: 0,
        forbidden_drift_count: 0,
        expected_checkpoint_digest: prototype.checkpoint_digest,
    }
}

fn model_programdata_observation(authority: Pubkey) -> ModelProgramDataObservation {
    ModelProgramDataObservation {
        slot: 7,
        payload_hash: hashv(&[CURRENT_PAYLOAD]).to_bytes(),
        raw_hash: loader_account_data_hash(&programdata_bytes(7, authority, CURRENT_PAYLOAD)),
        capacity: CURRENT_PAYLOAD.len() as u64,
        authority,
    }
}

fn model_initialization(harness: &Harness) -> ModelInitialization {
    ModelInitialization {
        slot: INITIAL_SLOT,
        graph: ModelIdentityGraph {
            controller_program: harness.controller,
            controller_programdata: derive_upgradeable_programdata_address(&harness.controller).0,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            authority_pda: harness.authority,
            gate_pda: harness.gate,
            policy_pda: harness.policy,
            council_pda: harness.council,
            canonical_spill_treasury: harness.spill,
            guardian: harness.guardian.pubkey(),
        },
        seats: harness.seats.each_ref().map(Signer::pubkey),
        seat_terms: [ModelSeatTerm {
            start_slot: 1,
            end_slot: 10_000,
        }; 5],
        delays: ModelDelays {
            review_slots: 8,
            rollback_slots: 2,
            routine_slots: 5,
            major_slots: 8,
            terminal_slots: 10,
            proposal_expiry_slots: 100,
        },
        programdata: model_programdata_observation(Pubkey::new_from_array([0xd1; 32])),
        controller_programdata_linked: true,
        initializer_is_controller_upgrade_authority: true,
        target_programdata_linked: true,
        canonical_pdas_verified: true,
        seat_accounts_readonly: true,
        seat_accounts_nonexecutable: true,
    }
}

fn model_apply_ok(model: &mut Release1Model, action: Release1ModelAction) {
    assert_eq!(
        model.apply(action).expect("model action must succeed"),
        Release1ModelOutcome::Applied
    );
}

fn model_create_proposal(model: &mut Release1Model, request: ModelProposalRequest) -> u64 {
    match model
        .apply(Release1ModelAction::CreateProposal(request))
        .expect("model proposal creation must succeed")
    {
        Release1ModelOutcome::ProposalCreated(proposal_id) => proposal_id,
        outcome => panic!("unexpected model proposal outcome: {outcome:?}"),
    }
}

fn assert_model_atomic_error(
    model: &mut Release1Model,
    action: Release1ModelAction,
    expected: Release1ModelError,
) {
    let before = model.clone();
    let before_bytes = model.try_to_vec().unwrap();
    assert_eq!(model.apply(action).unwrap_err(), expected);
    assert_eq!(*model, before);
    assert_eq!(model.try_to_vec().unwrap(), before_bytes);
}

async fn assert_model_processor_projection(
    context: &mut ProgramTestContext,
    harness: &Harness,
    model: &Release1Model,
    proposal_keys: &[(u64, Pubkey)],
) {
    let config: ControllerConfigV1 = state(context, harness.config).await;
    let gate: ProtocolGateV1 = state(context, harness.gate).await;
    assert_eq!(config.next_proposal_id, model.next_proposal_id);
    assert_eq!(config.target_nonce, model.target_nonce);
    assert_eq!(config.current_council_version, model.council.version);
    assert_eq!(
        config.token_governance_enabled,
        model.token_governance_enabled
    );
    assert_eq!(gate.status, model.gate.status);
    assert_eq!(gate.epoch, model.gate.epoch);
    assert_eq!(gate.freeze_slot, model.gate.freeze_slot);
    assert_eq!(gate.freeze_reason_code, model.gate.freeze_reason);
    let proposal_key = |proposal_id: u64| {
        proposal_keys
            .iter()
            .find_map(|(id, key)| (*id == proposal_id).then_some(*key))
            .expect("model proposal must have a processor PDA projection")
    };
    assert_eq!(
        gate.active_proposal,
        model
            .gate
            .active_proposal
            .map(proposal_key)
            .unwrap_or_default()
    );
    assert_eq!(
        gate.last_completed_proposal,
        model
            .gate
            .last_completed_proposal
            .map(proposal_key)
            .unwrap_or_default()
    );
    for (proposal_id, key) in proposal_keys {
        let expected = model
            .proposals
            .get(proposal_id)
            .expect("processor proposal must exist in model");
        let actual: UpgradeProposalV2 = state(context, *key).await;
        assert_eq!(actual.proposal_id, expected.id);
        assert_eq!(actual.proposal_class, expected.class);
        assert_eq!(actual.state, expected.state);
        assert_eq!(actual.target_nonce, expected.target_nonce);
        assert_eq!(actual.creation_gate_status, expected.creation_gate_status);
        assert_eq!(actual.creation_gate_epoch, expected.creation_gate_epoch);
        assert_eq!(actual.freeze_gate_epoch, expected.freeze_gate_epoch);
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
        assert_eq!(actual.frozen_slot, expected.frozen_slot);
        assert_eq!(actual.terminal_slot, expected.terminal_slot);
        assert_eq!(actual.terminal_reason_code, expected.terminal_reason);
    }
}

async fn assert_model_rotation_projection(
    context: &mut ProgramTestContext,
    harness: &Harness,
    model: &Release1Model,
    candidate_key: Pubkey,
    rotation_key: Pubkey,
    rotation_id: u64,
) {
    let config: ControllerConfigV1 = state(context, harness.config).await;
    let candidate: GovernanceCouncilSetV1 = state(context, candidate_key).await;
    let rotation: CouncilRotationProposalV1 = state(context, rotation_key).await;
    let expected = model
        .rotations
        .get(&rotation_id)
        .expect("processor rotation must exist in model");
    let expected_candidate = model
        .council_history
        .get(&candidate.version)
        .expect("processor candidate council must exist in model");

    assert_eq!(config.current_council_version, model.council.version);
    assert_eq!(candidate.version, expected_candidate.version);
    assert_eq!(
        candidate.activation_slot,
        expected_candidate.activation_slot
    );
    for (actual, (authority, term)) in candidate.seats.iter().zip(
        expected_candidate
            .seats
            .iter()
            .zip(expected_candidate.seat_terms.iter()),
    ) {
        assert_eq!(actual.seat_authority, *authority);
        assert_eq!(actual.term_start_slot, term.start_slot);
        assert_eq!(actual.term_end_slot, term.end_slot);
        assert!(actual.active);
    }
    assert_eq!(candidate.routine_threshold, 3);
    assert_eq!(candidate.terminal_threshold, 4);
    assert_eq!(candidate.policy_flags, 0);

    assert_eq!(rotation.state, expected.state);
    assert_eq!(
        rotation.current_council_version,
        expected.current_council_version
    );
    assert_eq!(rotation.candidate_council, candidate_key);
    assert_eq!(
        rotation.candidate_council_version,
        expected.candidate.version
    );
    assert_eq!(rotation.target_nonce, expected.target_nonce);
    assert_eq!(rotation.creation_slot, expected.creation_slot);
    assert_eq!(rotation.not_before_slot, expected.not_before_slot);
    assert_eq!(rotation.expiry_slot, expected.expiry_slot);
    assert_eq!(rotation.approval_bitset, expected.approvals.bitset);
    assert_eq!(rotation.approval_count, expected.approvals.count);
    assert_eq!(
        rotation.cancellation_approval_bitset,
        expected.cancellation_approvals.bitset
    );
    assert_eq!(
        rotation.cancellation_approval_count,
        expected.cancellation_approvals.count
    );
    assert_eq!(
        rotation.cancellation_reason_code,
        expected.cancellation_reason
    );
    assert_eq!(rotation.activated_slot, expected.terminal_slot);
    assert_eq!(rotation.terminal_reason_code, expected.terminal_reason);
}

fn model_proposal_request(
    class: ProposalClassV1,
    primary_proposal: Option<u64>,
    rollback_proposal: Option<u64>,
    proposer_seat: u8,
    slot: u64,
) -> ModelProposalRequest {
    ModelProposalRequest {
        class,
        extension_required: false,
        primary_proposal,
        rollback_proposal,
        proposer_seat,
        proposer_signed: true,
        payer_is_separate: true,
        slot,
    }
}

fn model_mark_buffer_verified(model: &mut Release1Model, proposal_id: u64) {
    model_apply_ok(model, Release1ModelAction::AdoptBuffer { proposal_id });
    model_apply_ok(model, Release1ModelAction::VerifyBuffer { proposal_id });
}

fn model_prepare_rollback(model: &mut Release1Model, proposal_id: u64) {
    model_mark_buffer_verified(model, proposal_id);
    let timing = model.proposals[&proposal_id].timing;
    for seat in 0..3 {
        model_apply_ok(
            model,
            Release1ModelAction::ApproveProposal {
                proposal_id,
                seat,
                slot: timing.review_start_slot,
            },
        );
    }
    model_apply_ok(
        model,
        Release1ModelAction::SatisfyGovernance {
            proposal_id,
            slot: timing.review_start_slot,
        },
    );
    model_apply_ok(
        model,
        Release1ModelAction::QueueProposal {
            proposal_id,
            slot: timing.review_start_slot,
        },
    );
}

fn differential_hard_state() -> ModelHardStateObservation {
    let mut observation = ModelHardStateObservation {
        schema_identifier: [20; 32],
        program_owned_root: [31; 32],
        program_owned_count: 5,
        logical_compressed_root: [32; 32],
        logical_compressed_count: 6,
        semantic_custody_root: [33; 32],
        custody_identity_root: [34; 32],
        hard_combined_root: [0; 32],
    };
    observation.hard_combined_root = observation.recompute_hard_combined_root();
    observation
}

async fn gate_c_model_processor_differential_through_accepted_prestate() {
    let (mut context, harness) = start_harness(None).await;
    let initialization = model_initialization(&harness);
    let mut model = Release1Model::default();
    model_apply_ok(
        &mut model,
        Release1ModelAction::Initialize(Box::new(initialization.clone())),
    );
    assert_model_processor_projection(&mut context, &harness, &model, &[]).await;
    assert_model_atomic_error(
        &mut model,
        Release1ModelAction::Initialize(Box::new(initialization)),
        Release1ModelError::AlreadyInitialized,
    );
    assert_model_processor_projection(&mut context, &harness, &model, &[]).await;

    activate_for_test(&mut context, &harness).await;
    model_apply_ok(
        &mut model,
        Release1ModelAction::SetProgramDataObservation(model_programdata_observation(
            harness.authority,
        )),
    );
    model_apply_ok(
        &mut model,
        Release1ModelAction::SimulatePostHandoffActivation {
            slot: ACTIVE_SLOT,
            bridge_and_authority_graph_verified: true,
        },
    );
    assert_model_processor_projection(&mut context, &harness, &model, &[]).await;

    let (primary_key, _) = create_primary(&mut context, &harness).await;
    let primary_id = model_create_proposal(
        &mut model,
        model_proposal_request(
            ProposalClassV1::RoutineUpgrade,
            None,
            Some(2),
            1,
            ACTIVE_SLOT,
        ),
    );
    assert_eq!(primary_id, 1);
    let rollback_creation_slot = ACTIVE_SLOT + model.delays.rollback_slots + 1;
    let rollback_id = model_create_proposal(
        &mut model,
        model_proposal_request(
            ProposalClassV1::EmergencyRollback,
            Some(primary_id),
            None,
            1,
            rollback_creation_slot,
        ),
    );
    assert_eq!(rollback_id, 2);
    let primary = seed_sealed_buffers(&mut context, &harness, primary_key).await;
    model_mark_buffer_verified(&mut model, primary_id);
    model_prepare_rollback(&mut model, rollback_id);
    let rollback_key = primary.rollback_proposal.value;
    let mut proposal_keys = vec![(primary_id, primary_key), (rollback_id, rollback_key)];
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;

    let legacy = Instruction {
        program_id: harness.controller,
        accounts: vec![AccountMeta::new(primary_key, false)],
        data: RecordProposalApprovalV1 {
            expected_proposal_digest: primary.proposal_digest,
            expected_council_version: primary.creation_council_version,
        }
        .pack()
        .to_vec(),
    };
    let before = bytes(&mut context, primary_key).await;
    let error = submit(&mut context, &[legacy], &[]).await.unwrap_err();
    assert_eq!(
        error.unwrap(),
        TransactionError::InstructionError(0, InstructionError::InvalidInstructionData)
    );
    assert_eq!(bytes(&mut context, primary_key).await, before);
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;

    let config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let valid = || {
        approve_instruction(
            &harness,
            primary_key,
            &primary,
            &config,
            &gate,
            harness.seats[0].pubkey(),
        )
    };

    // Timing is checked before any approval byte is touched.
    assert_atomic_failure(&mut context, valid(), &[&harness.seats[0]], &[primary_key]).await;
    assert_model_atomic_error(
        &mut model,
        Release1ModelAction::ApproveProposal {
            proposal_id: primary_id,
            seat: 0,
            slot: ACTIVE_SLOT,
        },
        Release1ModelError::TimingViolation,
    );
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;
    set_clock_slot(&mut context, primary.review_start_slot).await;

    let mut wrong_count = valid();
    wrong_count.accounts.remove(0);
    assert_atomic_failure(
        &mut context,
        wrong_count,
        &[&harness.seats[0]],
        &[primary_key],
    )
    .await;
    let mut wrong_order = valid();
    wrong_order.accounts.swap(1, 2);
    assert_atomic_failure(
        &mut context,
        wrong_order,
        &[&harness.seats[0]],
        &[primary_key],
    )
    .await;
    let mut wrong_privilege = valid();
    wrong_privilege.accounts[0].is_writable = true;
    assert_atomic_failure(
        &mut context,
        wrong_privilege,
        &[&harness.seats[0]],
        &[primary_key],
    )
    .await;
    let mut alias = valid();
    alias.accounts[3].pubkey = harness.policy;
    assert_atomic_failure(&mut context, alias, &[&harness.seats[0]], &[primary_key]).await;

    let policy: GovernancePolicyV1 = state(&mut context, harness.policy).await;
    set_account(
        &mut context,
        harness.policy,
        state_account(system_program::ID, &policy),
    );
    assert_atomic_failure(&mut context, valid(), &[&harness.seats[0]], &[primary_key]).await;
    set_account(
        &mut context,
        harness.policy,
        state_account(harness.controller, &policy),
    );

    let mut stale_nonce = ApproveProposalV2::unpack(&valid().data).unwrap();
    stale_nonce.expected.expected_target_nonce += 1;
    let mut stale_nonce_ix = valid();
    stale_nonce_ix.data = stale_nonce.pack().to_vec();
    assert_atomic_failure(
        &mut context,
        stale_nonce_ix,
        &[&harness.seats[0]],
        &[primary_key],
    )
    .await;
    let mut stale_epoch = ApproveProposalV2::unpack(&valid().data).unwrap();
    stale_epoch.expected.expected_gate_epoch += 1;
    let mut stale_epoch_ix = valid();
    stale_epoch_ix.data = stale_epoch.pack().to_vec();
    assert_atomic_failure(
        &mut context,
        stale_epoch_ix,
        &[&harness.seats[0]],
        &[primary_key],
    )
    .await;
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;

    for index in 0..3 {
        let proposal: UpgradeProposalV2 = state(&mut context, primary_key).await;
        let config: ControllerConfigV1 = state(&mut context, harness.config).await;
        let gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
        let instruction = approve_instruction(
            &harness,
            primary_key,
            &proposal,
            &config,
            &gate,
            harness.seats[index].pubkey(),
        );
        submit(
            &mut context,
            std::slice::from_ref(&instruction),
            &[&harness.seats[index]],
        )
        .await
        .unwrap();
        model_apply_ok(
            &mut model,
            Release1ModelAction::ApproveProposal {
                proposal_id: primary_id,
                seat: index as u8,
                slot: primary.review_start_slot,
            },
        );
        assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;
        if index == 0 {
            let once: UpgradeProposalV2 = state(&mut context, primary_key).await;
            let duplicate = approve_instruction(
                &harness,
                primary_key,
                &once,
                &config,
                &gate,
                harness.seats[index].pubkey(),
            );
            assert_atomic_failure(
                &mut context,
                duplicate,
                &[&harness.seats[index]],
                &[primary_key],
            )
            .await;
            assert_model_atomic_error(
                &mut model,
                Release1ModelAction::ApproveProposal {
                    proposal_id: primary_id,
                    seat: index as u8,
                    slot: primary.review_start_slot,
                },
                Release1ModelError::DuplicateApproval,
            );
            assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;
        }
    }
    let approved: UpgradeProposalV2 = state(&mut context, primary_key).await;
    assert_eq!(approved.state, ProposalStateV2::CouncilApproved);
    assert_eq!(approved.council_approval_count, 3);

    let config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    submit(
        &mut context,
        &[finalize_governance_v2_instruction(
            harness.controller,
            FinalizeGovernanceV2Accounts {
                controller_config: harness.config,
                policy: harness.policy,
                council: harness.council,
                protocol_gate: harness.gate,
                proposal: primary_key,
            },
            FinalizeGovernanceV2 {
                expected: expectation(&approved, &config, &gate),
                expected_approval_bitset: approved.council_approval_bitset,
                expected_approval_count: approved.council_approval_count,
            },
        )],
        &[],
    )
    .await
    .unwrap();
    model_apply_ok(
        &mut model,
        Release1ModelAction::SatisfyGovernance {
            proposal_id: primary_id,
            slot: primary.review_start_slot,
        },
    );
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;
    let satisfied: UpgradeProposalV2 = state(&mut context, primary_key).await;
    submit(
        &mut context,
        &[queue_proposal_v2_instruction(
            harness.controller,
            QueueProposalV2Accounts {
                controller_config: harness.config,
                policy: harness.policy,
                protocol_gate: harness.gate,
                proposal: primary_key,
            },
            QueueProposalV2 {
                expected: expectation(&satisfied, &config, &gate),
            },
        )],
        &[],
    )
    .await
    .unwrap();
    model_apply_ok(
        &mut model,
        Release1ModelAction::QueueProposal {
            proposal_id: primary_id,
            slot: primary.review_start_slot,
        },
    );
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;
    let timelocked: UpgradeProposalV2 = state(&mut context, primary_key).await;
    assert_eq!(timelocked.state, ProposalStateV2::Timelocked);

    let freeze = freeze_proposal_v2_instruction(
        harness.controller,
        FreezeProposalV2Accounts {
            controller_config: harness.config,
            policy: harness.policy,
            council: harness.council,
            protocol_gate: harness.gate,
            proposal: primary_key,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            authority_pda: harness.authority,
            rollback_proposal: timelocked.rollback_proposal.value,
            rollback_buffer_verification: derive_buffer_check_pda(
                &harness.controller,
                &timelocked.rollback_proposal.value,
            )
            .0,
            rollback_buffer: timelocked.rollback_buffer.value,
        },
        FreezeProposalV2 {
            expected: expectation(&timelocked, &config, &gate),
            expected_next_gate_epoch: gate.epoch + 1,
        },
    );
    assert_atomic_failure(
        &mut context,
        freeze.clone(),
        &[],
        &[harness.config, harness.gate, primary_key],
    )
    .await;
    assert_model_atomic_error(
        &mut model,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary_id,
            slot: primary.review_start_slot,
        },
        Release1ModelError::TimingViolation,
    );
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;

    // Create a second nonce-1 candidate late enough that its review window is
    // still open after the primary consumes the nonce. That distinguishes a
    // mechanically stale candidate from a merely out-of-window candidate.
    let competing_creation_slot = timelocked.not_before_slot - 2;
    set_clock_slot(&mut context, competing_creation_slot).await;
    let competing_buffer = Pubkey::new_unique();
    let competing_uploader = Pubkey::new_unique();
    let competing_rollback_buffer = Pubkey::new_unique();
    set_account(&mut context, competing_uploader, system_account());
    set_account(
        &mut context,
        competing_buffer,
        account(
            UPGRADEABLE_LOADER_ID,
            buffer_bytes(competing_uploader, b"release-1-competing"),
            false,
        ),
    );
    let competing_config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let competing_policy: GovernancePolicyV1 = state(&mut context, harness.policy).await;
    let competing_council: GovernanceCouncilSetV1 = state(&mut context, harness.council).await;
    let competing_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let competing_rollback = derive_proposal_pda(&harness.controller, &harness.target, 4).0;
    let mut competing = proposal_template(
        &harness,
        &competing_config,
        &competing_policy,
        &competing_council,
        &competing_gate,
        ProposalTemplateInput {
            proposal_id: 3,
            creation_slot: competing_creation_slot,
            buffer: competing_buffer,
            uploader: competing_uploader,
            artifact: b"release-1-competing",
            class: ProposalClassV1::RoutineUpgrade,
            primary: OptionalPubkeyV1::none(),
            rollback: OptionalPubkeyV1::some(competing_rollback).unwrap(),
            rollback_buffer: OptionalPubkeyV1::some(competing_rollback_buffer).unwrap(),
            rollback_hash: hashv(&[ROLLBACK_ARTIFACT]).to_bytes(),
            rollback_root: artifact_merkle_root(ROLLBACK_ARTIFACT, RELEASE1_ARTIFACT_CHUNK_SIZE_V1)
                .unwrap(),
        },
    );
    let competing_key = derive_proposal_pda(&harness.controller, &harness.target, 3).0;
    let create_competing = create_primary_instruction(&context, &harness, &competing);
    submit(&mut context, &[create_competing], &[&harness.seats[1]])
        .await
        .unwrap();
    let competing_id = model_create_proposal(
        &mut model,
        model_proposal_request(
            ProposalClassV1::RoutineUpgrade,
            None,
            Some(4),
            1,
            competing_creation_slot,
        ),
    );
    assert_eq!(competing_id, 3);
    competing.state = ProposalStateV2::BufferVerified;
    set_account(
        &mut context,
        competing_key,
        state_account(harness.controller, &competing),
    );
    model_mark_buffer_verified(&mut model, competing_id);
    proposal_keys.push((competing_id, competing_key));
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;

    set_clock_slot(&mut context, timelocked.not_before_slot).await;
    submit(&mut context, &[freeze], &[]).await.unwrap();
    model_apply_ok(
        &mut model,
        Release1ModelAction::FreezeProposal {
            proposal_id: primary_id,
            slot: timelocked.not_before_slot,
        },
    );
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;

    let frozen: UpgradeProposalV2 = state(&mut context, primary_key).await;
    let frozen_config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let frozen_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    assert_eq!(frozen.state, ProposalStateV2::Frozen);
    assert_eq!(frozen_config.target_nonce, 2);
    assert_eq!(frozen_gate.epoch, 3);
    assert_eq!(frozen_gate.active_proposal, primary_key);

    let stale_candidate: UpgradeProposalV2 = state(&mut context, competing_key).await;
    let stale_config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let stale_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    assert_atomic_failure(
        &mut context,
        approve_instruction(
            &harness,
            competing_key,
            &stale_candidate,
            &stale_config,
            &stale_gate,
            harness.seats[0].pubkey(),
        ),
        &[&harness.seats[0]],
        &[competing_key],
    )
    .await;
    assert_model_atomic_error(
        &mut model,
        Release1ModelAction::ApproveProposal {
            proposal_id: competing_id,
            seat: 0,
            slot: timelocked.not_before_slot,
        },
        Release1ModelError::StaleBinding,
    );
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;

    let council: GovernanceCouncilSetV1 = state(&mut context, harness.council).await;
    let candidate = prestate_candidate(
        &harness,
        primary_key,
        &frozen,
        &council,
        &frozen_gate,
        current_slot(&mut context).await,
    );
    let checkpoint = frozen.prestate_checkpoint;
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
        let payer = context.payer.pubkey();
        submit(
            &mut context,
            &[create_checkpoint_attestation_v1_instruction(
                harness.controller,
                CreateCheckpointAttestationV1Accounts {
                    payer,
                    controller_config: harness.config,
                    policy: harness.policy,
                    council: harness.council,
                    protocol_gate: harness.gate,
                    subject: primary_key,
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
            )],
            &[&harness.seats[index]],
        )
        .await
        .unwrap();
        model_apply_ok(
            &mut model,
            Release1ModelAction::AttestCheckpoint {
                proposal_id: primary_id,
                phase: StateCheckpointPhaseV1::Prestate,
                seat: index as u8,
                hard_state: differential_hard_state(),
                forbidden_drift_count: 0,
                slot: timelocked.not_before_slot,
            },
        );
        assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;
    }
    let payer = context.payer.pubkey();
    submit(
        &mut context,
        &[finalize_checkpoint_v1_instruction(
            harness.controller,
            FinalizeCheckpointV1Accounts {
                payer,
                controller_config: harness.config,
                policy: harness.policy,
                council: harness.council,
                protocol_gate: harness.gate,
                subject: primary_key,
                target_program: harness.target,
                target_programdata: harness.target_programdata,
                phase_evidence: frozen.buffer_verification,
                baseline_checkpoint: None,
                checkpoint,
                checkpoint_attestations: attestations,
                system_program: system_program::ID,
            },
            FinalizeCheckpointV1 {
                candidate,
                expected_council_version: council.version,
                expected_council_hash: council.set_hash,
            },
        )],
        &[],
    )
    .await
    .unwrap();
    model_apply_ok(
        &mut model,
        Release1ModelAction::FinalizeCheckpoint {
            proposal_id: primary_id,
            phase: StateCheckpointPhaseV1::Prestate,
            attesting_seats: [0, 1, 2],
            slot: timelocked.not_before_slot,
        },
    );
    let finalized: StateCheckpointV1 = state(&mut context, checkpoint).await;
    assert!(finalized.accepted);
    assert_eq!(finalized.approval_bitset, 0b00111);
    assert!(model.proposals[&primary_id]
        .prestate
        .as_ref()
        .is_some_and(|checkpoint| checkpoint.accepted));
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;

    set_clock_slot(&mut context, competing.expiry_slot).await;
    let expiring: UpgradeProposalV2 = state(&mut context, competing_key).await;
    let expiry_config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let expiry_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let expire = expire_proposal_v2_instruction(
        harness.controller,
        ExpireProposalV2Accounts {
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: competing_key,
        },
        ExpireProposalV2 {
            expected: expectation(&expiring, &expiry_config, &expiry_gate),
        },
    );
    submit(&mut context, &[expire], &[]).await.unwrap();
    model_apply_ok(
        &mut model,
        Release1ModelAction::ExpireProposal {
            proposal_id: competing_id,
            slot: competing.expiry_slot,
        },
    );
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;

    let expired: UpgradeProposalV2 = state(&mut context, competing_key).await;
    let duplicate_expiry = expire_proposal_v2_instruction(
        harness.controller,
        ExpireProposalV2Accounts {
            controller_config: harness.config,
            protocol_gate: harness.gate,
            proposal: competing_key,
        },
        ExpireProposalV2 {
            expected: expectation(&expired, &expiry_config, &expiry_gate),
        },
    );
    assert_atomic_failure(&mut context, duplicate_expiry, &[], &[competing_key]).await;
    assert_model_atomic_error(
        &mut model,
        Release1ModelAction::ExpireProposal {
            proposal_id: competing_id,
            slot: competing.expiry_slot,
        },
        Release1ModelError::InvalidStateTransition,
    );
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;
}

fn proxy_process_instruction(
    proxy_program: &Pubkey,
    accounts: &[AccountInfo<'_>],
    data: &[u8],
) -> ProgramResult {
    let [config, policy, council, gate, proposal, seat, controller] = accounts else {
        return Err(solana_program::program_error::ProgramError::NotEnoughAccountKeys);
    };
    let approval = ApproveProposalV2::unpack(data)?;
    let (expected_seat, bump) = Pubkey::find_program_address(&[PROXY_SEAT_SEED], proxy_program);
    if expected_seat != *seat.key {
        return Err(solana_program::program_error::ProgramError::InvalidSeeds);
    }
    let inner = approve_proposal_v2_instruction(
        *controller.key,
        ApproveProposalV2Accounts {
            controller_config: *config.key,
            policy: *policy.key,
            council: *council.key,
            protocol_gate: *gate.key,
            proposal: *proposal.key,
            seat_authority: *seat.key,
        },
        approval,
    );
    let bump_seed = [bump];
    invoke_signed(
        &inner,
        &[
            config.clone(),
            policy.clone(),
            council.clone(),
            gate.clone(),
            proposal.clone(),
            seat.clone(),
            controller.clone(),
        ],
        &[&[PROXY_SEAT_SEED, &bump_seed]],
    )
}

async fn smart_account_pda_seat_requires_cpi_invoke_signed() {
    let proxy_program = Pubkey::new_unique();
    let proxy_seat = Pubkey::find_program_address(&[PROXY_SEAT_SEED], &proxy_program).0;
    let (mut context, harness) = start_harness(Some((proxy_seat, proxy_program))).await;
    activate_for_test(&mut context, &harness).await;
    let (primary_key, _) = create_primary(&mut context, &harness).await;
    let primary = seed_sealed_buffers(&mut context, &harness, primary_key).await;
    set_clock_slot(&mut context, primary.review_start_slot).await;
    let config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let approval = ApproveProposalV2 {
        expected: expectation(&primary, &config, &gate),
        expected_approval_bitset: 0,
        expected_approval_count: 0,
    };
    let outer = Instruction {
        program_id: proxy_program,
        accounts: vec![
            AccountMeta::new_readonly(harness.config, false),
            AccountMeta::new_readonly(harness.policy, false),
            AccountMeta::new_readonly(harness.council, false),
            AccountMeta::new_readonly(harness.gate, false),
            AccountMeta::new(primary_key, false),
            AccountMeta::new_readonly(proxy_seat, false),
            AccountMeta::new_readonly(harness.controller, false),
        ],
        data: approval.pack().to_vec(),
    };
    submit(&mut context, &[outer], &[]).await.unwrap();
    let approved: UpgradeProposalV2 = state(&mut context, primary_key).await;
    assert_eq!(
        (
            approved.council_approval_bitset,
            approved.council_approval_count
        ),
        (1, 1)
    );

    let mut direct =
        approve_instruction(&harness, primary_key, &approved, &config, &gate, proxy_seat);
    direct.accounts[5].is_signer = false;
    assert_atomic_failure(&mut context, direct, &[], &[primary_key]).await;
}

fn rotation_expectation(
    rotation: &CouncilRotationProposalV1,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
) -> CouncilRotationExpectationV1 {
    CouncilRotationExpectationV1 {
        expected_rotation_digest: rotation.rotation_digest,
        expected_current_council_version: rotation.current_council_version,
        expected_current_council_hash: rotation.current_council_hash,
        expected_candidate_council_version: rotation.candidate_council_version,
        expected_candidate_council_hash: rotation.candidate_council_hash,
        expected_gate_status: gate.status,
        expected_gate_epoch: gate.epoch,
        expected_target_nonce: config.target_nonce,
        expected_state: rotation.state,
        expected_not_before_slot: rotation.not_before_slot,
        expected_expiry_slot: rotation.expiry_slot,
    }
}

struct CandidateCouncilFixture {
    key: Pubkey,
    value: GovernanceCouncilSetV1,
    create_instruction: Instruction,
}

fn candidate_council_fixture(
    context: &ProgramTestContext,
    harness: &Harness,
    config: &ControllerConfigV1,
    current: &GovernanceCouncilSetV1,
    gate: &ProtocolGateV1,
    version: u64,
    creation_slot: u64,
) -> CandidateCouncilFixture {
    let candidate_authorities = harness.seats.each_ref().map(Signer::pubkey);
    let activation_slot = creation_slot + config.major_delay_slots;
    let (key, bump) = derive_council_pda(&harness.controller, &harness.target, version);
    let mut value = GovernanceCouncilSetV1 {
        discriminator: GOVERNANCE_COUNCIL_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        controller_config: harness.config,
        version,
        target_program: harness.target,
        activation_slot,
        deactivation_slot: 0,
        seats: candidate_authorities.map(|seat_authority| CouncilSeatV1 {
            seat_authority,
            term_start_slot: 1,
            term_end_slot: 10_000,
            active: true,
            reserved: [0; COUNCIL_SEAT_RESERVED_LEN],
        }),
        routine_threshold: 3,
        terminal_threshold: 4,
        policy_flags: 0,
        set_hash: [0; 32],
        reserved: [0; GOVERNANCE_COUNCIL_RESERVED_LEN],
    };
    value.set_hash = compute_council_set_hash(&value);
    let create_instruction = create_candidate_council_set_v1_instruction(
        harness.controller,
        CreateCandidateCouncilSetV1Accounts {
            payer: context.payer.pubkey(),
            creator_seat_authority: harness.seats[0].pubkey(),
            controller_config: harness.config,
            policy: harness.policy,
            current_council: harness.council,
            protocol_gate: harness.gate,
            candidate_council: key,
            candidate_seat_authorities: candidate_authorities,
            system_program: system_program::ID,
        },
        CreateCandidateCouncilSetV1 {
            expected_current_council_version: current.version,
            expected_current_council_hash: current.set_hash,
            candidate_council_version: version,
            activation_slot,
            expected_target_nonce: config.target_nonce,
            expected_gate_status: gate.status,
            expected_gate_epoch: gate.epoch,
            expected_candidate_council_hash: value.set_hash,
            seat_terms: [CouncilSeatTermV1 {
                term_start_slot: 1,
                term_end_slot: 10_000,
            }; 5],
        },
    );
    CandidateCouncilFixture {
        key,
        value,
        create_instruction,
    }
}

struct RotationFixture {
    key: Pubkey,
    value: CouncilRotationProposalV1,
    create_instruction: Instruction,
}

fn rotation_fixture(
    context: &ProgramTestContext,
    harness: &Harness,
    config: &ControllerConfigV1,
    current: &GovernanceCouncilSetV1,
    candidate: &CandidateCouncilFixture,
    gate: &ProtocolGateV1,
    creation_slot: u64,
) -> RotationFixture {
    let (key, bump) = derive_council_rotation_pda(
        &harness.controller,
        &harness.target,
        candidate.value.version,
    );
    let mut value = CouncilRotationProposalV1 {
        discriminator: COUNCIL_ROTATION_PROPOSAL_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        state: CouncilRotationStateV1::Draft,
        controller_config: harness.config,
        target_program: harness.target,
        current_council: harness.council,
        current_council_version: current.version,
        current_council_hash: current.set_hash,
        candidate_council: candidate.key,
        candidate_council_version: candidate.value.version,
        candidate_council_hash: candidate.value.set_hash,
        creation_slot,
        not_before_slot: candidate.value.activation_slot,
        expiry_slot: creation_slot + config.proposal_expiry_slots,
        target_nonce: config.target_nonce,
        approval_bitset: 0,
        approval_count: 0,
        cancellation_approval_bitset: 0,
        cancellation_approval_count: 0,
        rotation_digest: [0; 32],
        activated_slot: 0,
        cancellation_reason_code: 0,
        terminal_reason_code: 0,
        reserved: [0; COUNCIL_ROTATION_PROPOSAL_V1_RESERVED_LEN],
    };
    value.rotation_digest = compute_council_rotation_digest_v1(&value).unwrap();
    let create_instruction = create_council_rotation_v1_instruction(
        harness.controller,
        CreateCouncilRotationV1Accounts {
            payer: context.payer.pubkey(),
            creator_seat_authority: harness.seats[0].pubkey(),
            controller_config: harness.config,
            policy: harness.policy,
            current_council: harness.council,
            candidate_council: candidate.key,
            protocol_gate: harness.gate,
            rotation: key,
            system_program: system_program::ID,
        },
        CreateCouncilRotationV1 {
            creation_slot,
            not_before_slot: value.not_before_slot,
            expiry_slot: value.expiry_slot,
            expected_current_council_version: current.version,
            expected_current_council_hash: current.set_hash,
            expected_candidate_council_version: candidate.value.version,
            expected_candidate_council_hash: candidate.value.set_hash,
            expected_gate_status: gate.status,
            expected_gate_epoch: gate.epoch,
            expected_target_nonce: config.target_nonce,
            expected_rotation_digest: value.rotation_digest,
        },
    );
    RotationFixture {
        key,
        value,
        create_instruction,
    }
}

async fn create_candidate_and_rotation(
    context: &mut ProgramTestContext,
    harness: &Harness,
    config: &ControllerConfigV1,
    current: &GovernanceCouncilSetV1,
    gate: &ProtocolGateV1,
    version: u64,
) -> (CandidateCouncilFixture, RotationFixture) {
    let creation_slot = current_slot(context).await;
    let candidate = candidate_council_fixture(
        context,
        harness,
        config,
        current,
        gate,
        version,
        creation_slot,
    );
    submit(
        context,
        std::slice::from_ref(&candidate.create_instruction),
        &[&harness.seats[0]],
    )
    .await
    .unwrap();
    let rotation = rotation_fixture(
        context,
        harness,
        config,
        current,
        &candidate,
        gate,
        creation_slot,
    );
    submit(
        context,
        std::slice::from_ref(&rotation.create_instruction),
        &[&harness.seats[0]],
    )
    .await
    .unwrap();
    (candidate, rotation)
}

async fn approve_queue_and_activate_rotation(
    context: &mut ProgramTestContext,
    harness: &Harness,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
    candidate: &CandidateCouncilFixture,
    rotation: &RotationFixture,
) {
    for index in 0..3 {
        let current_rotation: CouncilRotationProposalV1 = state(context, rotation.key).await;
        submit(
            context,
            &[approve_council_rotation_v1_instruction(
                harness.controller,
                ApproveCouncilRotationV1Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    current_council: harness.council,
                    candidate_council: candidate.key,
                    protocol_gate: harness.gate,
                    rotation: rotation.key,
                    seat_authority: harness.seats[index].pubkey(),
                },
                ApproveCouncilRotationV1 {
                    expected: rotation_expectation(&current_rotation, config, gate),
                    expected_approval_bitset: current_rotation.approval_bitset,
                    expected_approval_count: current_rotation.approval_count,
                },
            )],
            &[&harness.seats[index]],
        )
        .await
        .unwrap();
    }
    let approved: CouncilRotationProposalV1 = state(context, rotation.key).await;
    submit(
        context,
        &[queue_council_rotation_v1_instruction(
            harness.controller,
            QueueCouncilRotationV1Accounts {
                controller_config: harness.config,
                policy: harness.policy,
                current_council: harness.council,
                candidate_council: candidate.key,
                protocol_gate: harness.gate,
                rotation: rotation.key,
            },
            QueueCouncilRotationV1 {
                expected: rotation_expectation(&approved, config, gate),
                expected_approval_bitset: approved.approval_bitset,
                expected_approval_count: approved.approval_count,
            },
        )],
        &[],
    )
    .await
    .unwrap();
    let timelocked: CouncilRotationProposalV1 = state(context, rotation.key).await;
    set_clock_slot(context, timelocked.not_before_slot).await;
    submit(
        context,
        &[activate_council_rotation_v1_instruction(
            harness.controller,
            ActivateCouncilRotationV1Accounts {
                controller_config: harness.config,
                policy: harness.policy,
                current_council: harness.council,
                candidate_council: candidate.key,
                protocol_gate: harness.gate,
                rotation: rotation.key,
            },
            ActivateCouncilRotationV1 {
                expected: rotation_expectation(&timelocked, config, gate),
                expected_approval_bitset: timelocked.approval_bitset,
                expected_approval_count: timelocked.approval_count,
            },
        )],
        &[],
    )
    .await
    .unwrap();
    let updated: ControllerConfigV1 = state(context, harness.config).await;
    let activated: CouncilRotationProposalV1 = state(context, rotation.key).await;
    assert_eq!(updated.current_council_version, candidate.value.version);
    assert_eq!(activated.state, CouncilRotationStateV1::Activated);
}

#[derive(Clone, Copy)]
enum LosingRotationTerminal {
    Cancelled,
    Expired,
}

async fn terminal_v2_candidate_does_not_squat_v3(terminal: LosingRotationTerminal) {
    let (mut context, harness) = start_harness(None).await;
    activate_for_test(&mut context, &harness).await;
    let config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let current: GovernanceCouncilSetV1 = state(&mut context, harness.council).await;
    let gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let (candidate_v2, rotation_v2) =
        create_candidate_and_rotation(&mut context, &harness, &config, &current, &gate, 2).await;

    match terminal {
        LosingRotationTerminal::Cancelled => {
            for index in 0..3 {
                let rotation: CouncilRotationProposalV1 =
                    state(&mut context, rotation_v2.key).await;
                submit(
                    &mut context,
                    &[cancel_council_rotation_v1_instruction(
                        harness.controller,
                        CancelCouncilRotationV1Accounts {
                            controller_config: harness.config,
                            policy: harness.policy,
                            current_council: harness.council,
                            candidate_council: candidate_v2.key,
                            protocol_gate: harness.gate,
                            rotation: rotation_v2.key,
                            seat_authority: harness.seats[index].pubkey(),
                        },
                        CancelCouncilRotationV1 {
                            expected: rotation_expectation(&rotation, &config, &gate),
                            expected_cancellation_approval_bitset: rotation
                                .cancellation_approval_bitset,
                            expected_cancellation_approval_count: rotation
                                .cancellation_approval_count,
                            cancellation_reason_code: 41,
                        },
                    )],
                    &[&harness.seats[index]],
                )
                .await
                .unwrap();
            }
            let cancelled: CouncilRotationProposalV1 = state(&mut context, rotation_v2.key).await;
            assert_eq!(cancelled.state, CouncilRotationStateV1::Cancelled);
        }
        LosingRotationTerminal::Expired => {
            set_clock_slot(&mut context, rotation_v2.value.expiry_slot).await;
            submit(
                &mut context,
                &[expire_council_rotation_v1_instruction(
                    harness.controller,
                    ExpireCouncilRotationV1Accounts {
                        controller_config: harness.config,
                        protocol_gate: harness.gate,
                        rotation: rotation_v2.key,
                    },
                    ExpireCouncilRotationV1 {
                        expected: rotation_expectation(&rotation_v2.value, &config, &gate),
                    },
                )],
                &[],
            )
            .await
            .unwrap();
            let expired: CouncilRotationProposalV1 = state(&mut context, rotation_v2.key).await;
            assert_eq!(expired.state, CouncilRotationStateV1::Expired);
        }
    }

    // Candidate accounts are immutable and create-once. A losing v2 rotation
    // therefore cannot recycle or overwrite the v2 candidate PDA.
    assert_atomic_failure(
        &mut context,
        candidate_v2.create_instruction,
        &[&harness.seats[0]],
        &[candidate_v2.key],
    )
    .await;
    let unchanged: ControllerConfigV1 = state(&mut context, harness.config).await;
    assert_eq!(unchanged.current_council_version, 1);

    // The monotonic rule is strictly greater than the current council version,
    // not current+1, so an independently hashed v3 can proceed normally.
    let (candidate_v3, rotation_v3) =
        create_candidate_and_rotation(&mut context, &harness, &config, &current, &gate, 3).await;
    approve_queue_and_activate_rotation(
        &mut context,
        &harness,
        &config,
        &gate,
        &candidate_v3,
        &rotation_v3,
    )
    .await;
}

async fn council_rotation_uses_old_council_quorum_and_major_timelock() {
    let (mut context, harness) = start_harness(None).await;
    let mut model = Release1Model::default();
    model_apply_ok(
        &mut model,
        Release1ModelAction::Initialize(Box::new(model_initialization(&harness))),
    );
    activate_for_test(&mut context, &harness).await;
    model_apply_ok(
        &mut model,
        Release1ModelAction::SetProgramDataObservation(model_programdata_observation(
            harness.authority,
        )),
    );
    model_apply_ok(
        &mut model,
        Release1ModelAction::SimulatePostHandoffActivation {
            slot: ACTIVE_SLOT,
            bridge_and_authority_graph_verified: true,
        },
    );
    assert_model_processor_projection(&mut context, &harness, &model, &[]).await;
    let config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let current: GovernanceCouncilSetV1 = state(&mut context, harness.council).await;
    let gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    // Renewing the same five authorities with a new immutable council version
    // is a valid rotation. The creator is therefore intentionally present in
    // both the signer role and candidate-seat role; runtime privilege
    // coalescing makes both AccountInfos read-only signers.
    let candidate_authorities = harness.seats.each_ref().map(Signer::pubkey);
    let creation_slot = current_slot(&mut context).await;
    let activation_slot = creation_slot + config.major_delay_slots;
    let (candidate_key, candidate_bump) =
        derive_council_pda(&harness.controller, &harness.target, 2);
    let mut candidate = GovernanceCouncilSetV1 {
        discriminator: GOVERNANCE_COUNCIL_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump: candidate_bump,
        initialized: true,
        controller_config: harness.config,
        version: 2,
        target_program: harness.target,
        activation_slot,
        deactivation_slot: 0,
        seats: candidate_authorities.map(|seat_authority| CouncilSeatV1 {
            seat_authority,
            term_start_slot: 1,
            term_end_slot: 10_000,
            active: true,
            reserved: [0; COUNCIL_SEAT_RESERVED_LEN],
        }),
        routine_threshold: 3,
        terminal_threshold: 4,
        policy_flags: 0,
        set_hash: [0; 32],
        reserved: [0; GOVERNANCE_COUNCIL_RESERVED_LEN],
    };
    candidate.set_hash = compute_council_set_hash(&candidate);
    let payer = context.payer.pubkey();
    submit(
        &mut context,
        &[create_candidate_council_set_v1_instruction(
            harness.controller,
            CreateCandidateCouncilSetV1Accounts {
                payer,
                creator_seat_authority: harness.seats[0].pubkey(),
                controller_config: harness.config,
                policy: harness.policy,
                current_council: harness.council,
                protocol_gate: harness.gate,
                candidate_council: candidate_key,
                candidate_seat_authorities: candidate_authorities,
                system_program: system_program::ID,
            },
            CreateCandidateCouncilSetV1 {
                expected_current_council_version: current.version,
                expected_current_council_hash: current.set_hash,
                candidate_council_version: 2,
                activation_slot,
                expected_target_nonce: config.target_nonce,
                expected_gate_status: gate.status,
                expected_gate_epoch: gate.epoch,
                expected_candidate_council_hash: candidate.set_hash,
                seat_terms: [CouncilSeatTermV1 {
                    term_start_slot: 1,
                    term_end_slot: 10_000,
                }; 5],
            },
        )],
        &[&harness.seats[0]],
    )
    .await
    .unwrap();
    assert_eq!(
        model
            .apply(Release1ModelAction::CreateCandidateCouncilSet {
                candidate_version: 2,
                candidate_seats: candidate_authorities,
                candidate_seat_terms: [ModelSeatTerm {
                    start_slot: 1,
                    end_slot: 10_000,
                }; 5],
                activation_slot,
                slot: creation_slot,
            })
            .unwrap(),
        Release1ModelOutcome::CandidateCouncilCreated(2)
    );

    let (rotation_key, rotation_bump) =
        derive_council_rotation_pda(&harness.controller, &harness.target, 2);
    let mut rotation = CouncilRotationProposalV1 {
        discriminator: COUNCIL_ROTATION_PROPOSAL_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: rotation_bump,
        initialized: true,
        state: CouncilRotationStateV1::Draft,
        controller_config: harness.config,
        target_program: harness.target,
        current_council: harness.council,
        current_council_version: current.version,
        current_council_hash: current.set_hash,
        candidate_council: candidate_key,
        candidate_council_version: candidate.version,
        candidate_council_hash: candidate.set_hash,
        creation_slot,
        not_before_slot: activation_slot,
        expiry_slot: creation_slot + config.proposal_expiry_slots,
        target_nonce: config.target_nonce,
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
    rotation.rotation_digest = compute_council_rotation_digest_v1(&rotation).unwrap();
    submit(
        &mut context,
        &[create_council_rotation_v1_instruction(
            harness.controller,
            CreateCouncilRotationV1Accounts {
                payer,
                creator_seat_authority: harness.seats[0].pubkey(),
                controller_config: harness.config,
                policy: harness.policy,
                current_council: harness.council,
                candidate_council: candidate_key,
                protocol_gate: harness.gate,
                rotation: rotation_key,
                system_program: system_program::ID,
            },
            CreateCouncilRotationV1 {
                creation_slot,
                not_before_slot: activation_slot,
                expiry_slot: rotation.expiry_slot,
                expected_current_council_version: current.version,
                expected_current_council_hash: current.set_hash,
                expected_candidate_council_version: candidate.version,
                expected_candidate_council_hash: candidate.set_hash,
                expected_gate_status: gate.status,
                expected_gate_epoch: gate.epoch,
                expected_target_nonce: config.target_nonce,
                expected_rotation_digest: rotation.rotation_digest,
            },
        )],
        &[&harness.seats[0]],
    )
    .await
    .unwrap();
    assert_eq!(
        model
            .apply(Release1ModelAction::CreateCouncilRotation {
                candidate_version: 2,
                slot: creation_slot,
            })
            .unwrap(),
        Release1ModelOutcome::CouncilRotationCreated(2)
    );
    assert_model_rotation_projection(
        &mut context,
        &harness,
        &model,
        candidate_key,
        rotation_key,
        2,
    )
    .await;

    for index in 0..3 {
        let current_rotation: CouncilRotationProposalV1 = state(&mut context, rotation_key).await;
        submit(
            &mut context,
            &[approve_council_rotation_v1_instruction(
                harness.controller,
                ApproveCouncilRotationV1Accounts {
                    controller_config: harness.config,
                    policy: harness.policy,
                    current_council: harness.council,
                    candidate_council: candidate_key,
                    protocol_gate: harness.gate,
                    rotation: rotation_key,
                    seat_authority: harness.seats[index].pubkey(),
                },
                ApproveCouncilRotationV1 {
                    expected: rotation_expectation(&current_rotation, &config, &gate),
                    expected_approval_bitset: current_rotation.approval_bitset,
                    expected_approval_count: current_rotation.approval_count,
                },
            )],
            &[&harness.seats[index]],
        )
        .await
        .unwrap();
        model_apply_ok(
            &mut model,
            Release1ModelAction::ApproveCouncilRotation {
                rotation_id: 2,
                seat: index as u8,
                slot: creation_slot,
            },
        );
        assert_model_rotation_projection(
            &mut context,
            &harness,
            &model,
            candidate_key,
            rotation_key,
            2,
        )
        .await;
    }
    let approved: CouncilRotationProposalV1 = state(&mut context, rotation_key).await;
    assert_eq!(approved.state, CouncilRotationStateV1::CouncilApproved);
    submit(
        &mut context,
        &[queue_council_rotation_v1_instruction(
            harness.controller,
            QueueCouncilRotationV1Accounts {
                controller_config: harness.config,
                policy: harness.policy,
                current_council: harness.council,
                candidate_council: candidate_key,
                protocol_gate: harness.gate,
                rotation: rotation_key,
            },
            QueueCouncilRotationV1 {
                expected: rotation_expectation(&approved, &config, &gate),
                expected_approval_bitset: approved.approval_bitset,
                expected_approval_count: approved.approval_count,
            },
        )],
        &[],
    )
    .await
    .unwrap();
    model_apply_ok(
        &mut model,
        Release1ModelAction::QueueCouncilRotation {
            rotation_id: 2,
            slot: creation_slot,
        },
    );
    assert_model_rotation_projection(
        &mut context,
        &harness,
        &model,
        candidate_key,
        rotation_key,
        2,
    )
    .await;
    let timelocked: CouncilRotationProposalV1 = state(&mut context, rotation_key).await;
    set_clock_slot(&mut context, activation_slot).await;
    submit(
        &mut context,
        &[activate_council_rotation_v1_instruction(
            harness.controller,
            ActivateCouncilRotationV1Accounts {
                controller_config: harness.config,
                policy: harness.policy,
                current_council: harness.council,
                candidate_council: candidate_key,
                protocol_gate: harness.gate,
                rotation: rotation_key,
            },
            ActivateCouncilRotationV1 {
                expected: rotation_expectation(&timelocked, &config, &gate),
                expected_approval_bitset: timelocked.approval_bitset,
                expected_approval_count: timelocked.approval_count,
            },
        )],
        &[],
    )
    .await
    .unwrap();
    model_apply_ok(
        &mut model,
        Release1ModelAction::ActivateCouncilRotation {
            rotation_id: 2,
            slot: activation_slot,
        },
    );
    assert_model_rotation_projection(
        &mut context,
        &harness,
        &model,
        candidate_key,
        rotation_key,
        2,
    )
    .await;
    let updated: ControllerConfigV1 = state(&mut context, harness.config).await;
    let activated: CouncilRotationProposalV1 = state(&mut context, rotation_key).await;
    assert_eq!(updated.current_council_version, 2);
    assert_eq!(activated.state, CouncilRotationStateV1::Activated);
}

async fn guardian_freeze_grants_no_vote_and_conversion_stays_continuously_frozen() {
    let (mut context, harness) = start_harness(None).await;
    let mut model = Release1Model::default();
    model_apply_ok(
        &mut model,
        Release1ModelAction::Initialize(Box::new(model_initialization(&harness))),
    );
    activate_for_test(&mut context, &harness).await;
    model_apply_ok(
        &mut model,
        Release1ModelAction::SetProgramDataObservation(model_programdata_observation(
            harness.authority,
        )),
    );
    model_apply_ok(
        &mut model,
        Release1ModelAction::SimulatePostHandoffActivation {
            slot: ACTIVE_SLOT,
            bridge_and_authority_graph_verified: true,
        },
    );
    assert_model_processor_projection(&mut context, &harness, &model, &[]).await;
    let config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let active_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let freeze_slot = current_slot(&mut context).await;
    let frozen_epoch = active_gate.epoch + 1;
    let reason = 77;
    let (observation_key, observation_bump) =
        derive_emergency_freeze_observation_pda(&harness.controller, &harness.target, frozen_epoch);
    let raw_programdata = programdata_bytes(7, harness.authority, CURRENT_PAYLOAD);
    let raw_hash = loader_account_data_hash(&raw_programdata);
    let mut observation = EmergencyFreezeObservationV1 {
        discriminator: EMERGENCY_FREEZE_OBSERVATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: observation_bump,
        initialized: true,
        finalized: true,
        controller_program: harness.controller,
        controller_config: harness.config,
        protocol_gate: harness.gate,
        target_program: harness.target,
        target_programdata: harness.target_programdata,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        controller_authority: harness.authority,
        frozen_epoch,
        freeze_slot,
        freeze_reason_code: reason,
        actual_program_owner: UPGRADEABLE_LOADER_ID,
        actual_program_executable: true,
        actual_program_data_length: LOADER_PROGRAM_ACCOUNT_LEN as u64,
        program_header_present: true,
        actual_linked_programdata: OptionalPubkeyV1::some(harness.target_programdata).unwrap(),
        actual_programdata_owner: UPGRADEABLE_LOADER_ID,
        actual_programdata_executable: false,
        actual_programdata_data_length: raw_programdata.len() as u64,
        programdata_header_present: true,
        deployed_programdata_slot: 7,
        raw_hash_complete: true,
        raw_programdata_sha256: raw_hash,
        capacity: CURRENT_PAYLOAD.len() as u64,
        observed_authority: OptionalPubkeyV1::some(harness.authority).unwrap(),
        observation_digest: [1; 32],
        finalized_slot: freeze_slot,
        reserved: [0; EMERGENCY_FREEZE_OBSERVATION_V1_RESERVED_LEN],
    };
    observation.observation_digest =
        compute_emergency_freeze_observation_digest_v1(&observation).unwrap();
    let payer = context.payer.pubkey();
    let freeze = guardian_freeze_v1_instruction(
        harness.controller,
        GuardianFreezeV1Accounts {
            payer,
            controller_config: harness.config,
            protocol_gate: harness.gate,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            authority_pda: harness.authority,
            guardian: harness.guardian.pubkey(),
            emergency_freeze_observation: observation_key,
            system_program: system_program::ID,
        },
        GuardianFreezeV1 {
            expected_gate_status: GateStatusV1::Active,
            expected_gate_epoch: active_gate.epoch,
            expected_next_gate_epoch: frozen_epoch,
            expected_target_nonce: config.target_nonce,
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
            expected_programdata_slot: 7,
            expected_raw_hash_complete: true,
            expected_raw_programdata_hash: raw_hash,
            expected_capacity: CURRENT_PAYLOAD.len() as u64,
            expected_programdata_authority: OptionalInstructionPubkeyV1::some(harness.authority)
                .unwrap(),
            freeze_reason_code: reason,
            expected_observation_digest: observation.observation_digest,
        },
    );
    submit(
        &mut context,
        std::slice::from_ref(&freeze),
        &[&harness.guardian],
    )
    .await
    .unwrap();
    model_apply_ok(
        &mut model,
        Release1ModelAction::GuardianFreeze {
            guardian: harness.guardian.pubkey(),
            slot: freeze_slot,
            reason,
        },
    );
    assert_model_processor_projection(&mut context, &harness, &model, &[]).await;
    let emergency_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let unchanged_nonce: ControllerConfigV1 = state(&mut context, harness.config).await;
    assert_eq!(emergency_gate.status, GateStatusV1::EmergencyFrozen);
    assert_eq!(emergency_gate.epoch, frozen_epoch);
    assert_eq!(unchanged_nonce.target_nonce, config.target_nonce);
    assert_atomic_failure(
        &mut context,
        freeze,
        &[&harness.guardian],
        &[harness.gate, observation_key],
    )
    .await;
    assert_model_atomic_error(
        &mut model,
        Release1ModelAction::GuardianFreeze {
            guardian: harness.guardian.pubkey(),
            slot: freeze_slot,
            reason,
        },
        Release1ModelError::InvalidGate,
    );
    assert_model_processor_projection(&mut context, &harness, &model, &[]).await;

    let (primary_key, _) = create_primary(&mut context, &harness).await;
    let primary_id = model_create_proposal(
        &mut model,
        model_proposal_request(
            ProposalClassV1::RoutineUpgrade,
            None,
            Some(2),
            1,
            freeze_slot,
        ),
    );
    assert_eq!(primary_id, 1);
    let rollback_creation_slot = freeze_slot + model.delays.rollback_slots + 1;
    let rollback_id = model_create_proposal(
        &mut model,
        model_proposal_request(
            ProposalClassV1::EmergencyRollback,
            Some(primary_id),
            None,
            1,
            rollback_creation_slot,
        ),
    );
    assert_eq!(rollback_id, 2);
    let primary = seed_sealed_buffers(&mut context, &harness, primary_key).await;
    model_mark_buffer_verified(&mut model, primary_id);
    model_prepare_rollback(&mut model, rollback_id);
    let rollback_key = primary.rollback_proposal.value;
    let proposal_keys = vec![(primary_id, primary_key), (rollback_id, rollback_key)];
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;
    set_clock_slot(&mut context, primary.review_start_slot).await;
    let current_config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let current_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let guardian_vote = approve_instruction(
        &harness,
        primary_key,
        &primary,
        &current_config,
        &current_gate,
        harness.guardian.pubkey(),
    );
    assert_atomic_failure(
        &mut context,
        guardian_vote,
        &[&harness.guardian],
        &[primary_key],
    )
    .await;
    assert_model_atomic_error(
        &mut model,
        Release1ModelAction::ApproveProposal {
            proposal_id: primary_id,
            seat: 5,
            slot: primary.review_start_slot,
        },
        Release1ModelError::InvalidApproval,
    );
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;

    for index in 0..3 {
        let proposal: UpgradeProposalV2 = state(&mut context, primary_key).await;
        let current_config: ControllerConfigV1 = state(&mut context, harness.config).await;
        let current_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
        submit(
            &mut context,
            &[approve_instruction(
                &harness,
                primary_key,
                &proposal,
                &current_config,
                &current_gate,
                harness.seats[index].pubkey(),
            )],
            &[&harness.seats[index]],
        )
        .await
        .unwrap();
        model_apply_ok(
            &mut model,
            Release1ModelAction::ApproveProposal {
                proposal_id: primary_id,
                seat: index as u8,
                slot: primary.review_start_slot,
            },
        );
        assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;
    }
    let approved: UpgradeProposalV2 = state(&mut context, primary_key).await;
    let current_config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let current_gate: ProtocolGateV1 = state(&mut context, harness.gate).await;
    submit(
        &mut context,
        &[finalize_governance_v2_instruction(
            harness.controller,
            FinalizeGovernanceV2Accounts {
                controller_config: harness.config,
                policy: harness.policy,
                council: harness.council,
                protocol_gate: harness.gate,
                proposal: primary_key,
            },
            FinalizeGovernanceV2 {
                expected: expectation(&approved, &current_config, &current_gate),
                expected_approval_bitset: approved.council_approval_bitset,
                expected_approval_count: approved.council_approval_count,
            },
        )],
        &[],
    )
    .await
    .unwrap();
    model_apply_ok(
        &mut model,
        Release1ModelAction::SatisfyGovernance {
            proposal_id: primary_id,
            slot: primary.review_start_slot,
        },
    );
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;
    let satisfied: UpgradeProposalV2 = state(&mut context, primary_key).await;
    submit(
        &mut context,
        &[queue_proposal_v2_instruction(
            harness.controller,
            QueueProposalV2Accounts {
                controller_config: harness.config,
                policy: harness.policy,
                protocol_gate: harness.gate,
                proposal: primary_key,
            },
            QueueProposalV2 {
                expected: expectation(&satisfied, &current_config, &current_gate),
            },
        )],
        &[],
    )
    .await
    .unwrap();
    model_apply_ok(
        &mut model,
        Release1ModelAction::QueueProposal {
            proposal_id: primary_id,
            slot: primary.review_start_slot,
        },
    );
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;
    let timelocked: UpgradeProposalV2 = state(&mut context, primary_key).await;
    set_clock_slot(&mut context, timelocked.not_before_slot).await;
    let convert = convert_emergency_freeze_v2_instruction(
        harness.controller,
        ConvertEmergencyFreezeV2Accounts {
            controller_config: harness.config,
            policy: harness.policy,
            council: harness.council,
            protocol_gate: harness.gate,
            proposal: primary_key,
            emergency_freeze_observation: observation_key,
            target_program: harness.target,
            target_programdata: harness.target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            authority_pda: harness.authority,
            rollback_proposal: timelocked.rollback_proposal.value,
            rollback_buffer_verification: derive_buffer_check_pda(
                &harness.controller,
                &timelocked.rollback_proposal.value,
            )
            .0,
            rollback_buffer: timelocked.rollback_buffer.value,
        },
        ConvertEmergencyFreezeV2 {
            expected: expectation(&timelocked, &current_config, &current_gate),
            expected_next_gate_epoch: current_gate.epoch + 1,
            expected_freeze_observation_digest: observation.observation_digest,
        },
    );
    submit(&mut context, &[convert], &[]).await.unwrap();
    model_apply_ok(
        &mut model,
        Release1ModelAction::ConvertEmergencyFreeze {
            proposal_id: primary_id,
            slot: timelocked.not_before_slot,
        },
    );
    assert_model_processor_projection(&mut context, &harness, &model, &proposal_keys).await;
    let converted: ProtocolGateV1 = state(&mut context, harness.gate).await;
    let converted_config: ControllerConfigV1 = state(&mut context, harness.config).await;
    let converted_proposal: UpgradeProposalV2 = state(&mut context, primary_key).await;
    assert_eq!(converted.status, GateStatusV1::FrozenForUpgrade);
    assert_eq!(converted.epoch, frozen_epoch + 1);
    assert_eq!(converted_config.target_nonce, config.target_nonce + 1);
    assert_eq!(converted_proposal.state, ProposalStateV2::Frozen);
}

#[tokio::test]
#[ignore = "historical V1/V2 executable lifecycle is intentionally closed; Release 1 V3 ceremony ProgramTest supersedes this matrix"]
async fn gate_c_release1_programtest_matrix() {
    // Agave's ProgramTest spins up a large accounts-db worker pool per context.
    // Running independent contexts concurrently can exhaust the test host's file
    // descriptor limit before controller code executes, so keep the four isolated
    // scenarios deterministic and sequential within one integration test.
    eprintln!("gate_c stage: account-and-privilege matrix");
    release1_tags_1_through_25_account_and_privilege_matrix_is_failure_atomic().await;
    eprintln!("gate_c stage: model/processor differential");
    gate_c_model_processor_differential_through_accepted_prestate().await;
    eprintln!("gate_c stage: smart-account PDA seat");
    smart_account_pda_seat_requires_cpi_invoke_signed().await;
    eprintln!("gate_c stage: council rotation");
    council_rotation_uses_old_council_quorum_and_major_timelock().await;
    eprintln!("gate_c stage: cancelled rotation candidate");
    terminal_v2_candidate_does_not_squat_v3(LosingRotationTerminal::Cancelled).await;
    eprintln!("gate_c stage: expired rotation candidate");
    terminal_v2_candidate_does_not_squat_v3(LosingRotationTerminal::Expired).await;
    eprintln!("gate_c stage: guardian conversion");
    guardian_freeze_grants_no_vote_and_conversion_stays_continuously_frozen().await;
}
