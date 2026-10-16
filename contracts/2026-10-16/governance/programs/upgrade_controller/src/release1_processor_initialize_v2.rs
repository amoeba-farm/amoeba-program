//! Capacity-safe one-time Release 1 controller initialization.
//!
//! This path replaces the capacity-fragile V1 initializer. It validates the
//! complete Loader-v3 graph and pre-encodes every fixed account before its
//! first System Program CPI. The target's existing upgrade authority is only
//! observed here; it is not transferred or granted any controller capability.

use solana_program::{
    account_info::AccountInfo, clock::Clock, entrypoint::ProgramResult,
    program_error::ProgramError, pubkey::Pubkey, rent::Rent, sysvar::Sysvar,
};
use solana_sdk_ids::system_program;

use crate::{
    artifact_merkle::{ARTIFACT_MERKLE_SCHEME_ID, MAX_ARTIFACT_BYTES_V1},
    council::{
        compute_council_set_hash, validate_council_guardian_separation, validate_council_set,
    },
    pda::{
        derive_authority_pda, derive_capacity_policy_pda, derive_controller_config_pda,
        derive_controller_release_commitment_pda, derive_council_pda, derive_gate_pda,
        derive_policy_pda, derive_upgradeable_programdata_address, CAPACITY_POLICY_SEED,
        CONTROLLER_RELEASE_SEED, COUNCIL_SEED, GATE_SEED, POLICY_SEED, TARGET_SEED,
        UPGRADEABLE_LOADER_ID, UPGRADE_SEED_DOMAIN_V1,
    },
    policy::{compute_policy_hash, validate_policy_against_config},
    programdata_observation_merkle::{
        programdata_observation_chunk_count, MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
        PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB, PROGRAMDATA_OBSERVATION_FRONTIER_SLOTS_V1,
        PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
    },
    release1_account_io::{
        create_fixed_pda_account, encode_fixed_account, require_distinct_accounts,
        validate_exact_privileges,
    },
    release1_ceremony_digest::{
        compute_capacity_policy_digest_v1, compute_controller_release_digest_v1,
        validate_capacity_policy_digest_v1, validate_controller_release_digest_v1,
    },
    release1_ceremony_state::{
        ControllerReleaseCommitmentV1, ProgramDataCapacityPolicyV1, ARTIFACT_BINDING_CHUNK_SIZE_V1,
        CEREMONY_ACCOUNT_VERSION_V1, CONTROLLER_RELEASE_COMMITMENT_V1_DISCRIMINATOR,
        CONTROLLER_RELEASE_COMMITMENT_V1_RESERVED_LEN, EXTEND_PROGRAM_CHECKED_FEATURE_ID_V1,
        MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1, PROGRAMDATA_CAPACITY_POLICY_V1_DISCRIMINATOR,
        PROGRAMDATA_CAPACITY_POLICY_V1_RESERVED_LEN, SET_AUTHORITY_CHECKED_FEATURE_ID_V1,
    },
    release1_loader_accounts::{
        validate_program_programdata_linkage, LOADER_PROGRAMDATA_METADATA_LEN,
    },
    release1_state::BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
    release1_v3_instruction::InitializeControllerV2,
    state::{
        ControllerConfigV1, CouncilSeatV1, GateStatusV1, GovernanceCouncilSetV1, GovernanceModeV1,
        GovernancePolicyV1, OptionalPubkeyV1, ProtocolGateV1, ACCOUNT_VERSION_V1,
        CONTROLLER_CONFIG_DISCRIMINATOR, CONTROLLER_CONFIG_RESERVED_LEN, COUNCIL_SEAT_RESERVED_LEN,
        GOVERNANCE_COUNCIL_DISCRIMINATOR, GOVERNANCE_COUNCIL_RESERVED_LEN,
        GOVERNANCE_POLICY_DISCRIMINATOR, GOVERNANCE_POLICY_RESERVED_LEN,
        PROTOCOL_GATE_DISCRIMINATOR, PROTOCOL_GATE_RESERVED_LEN,
    },
    GovernanceError,
};

const INITIAL_POLICY_VERSION_V2: u64 = 1;
const INITIAL_COUNCIL_VERSION_V2: u64 = 1;
const INITIAL_NEXT_PROPOSAL_ID_V2: u64 = 1;
const INITIAL_TARGET_NONCE_V2: u64 = 1;
const INITIAL_GATE_EPOCH_V2: u64 = 1;
const COUNCIL_SIZE_V2: u8 = 5;
const ROUTINE_THRESHOLD_V2: u8 = 3;
const TERMINAL_THRESHOLD_V2: u8 = 4;
pub const INITIALIZE_CONTROLLER_ACCOUNT_COUNT_V2: usize = 22;

struct InitializeAccounts<'a, 'info> {
    payer: &'a AccountInfo<'info>,
    initializer: &'a AccountInfo<'info>,
    controller_program: &'a AccountInfo<'info>,
    controller_programdata: &'a AccountInfo<'info>,
    target_program: &'a AccountInfo<'info>,
    target_programdata: &'a AccountInfo<'info>,
    upgradeable_loader: &'a AccountInfo<'info>,
    controller_config: &'a AccountInfo<'info>,
    authority_pda: &'a AccountInfo<'info>,
    protocol_gate: &'a AccountInfo<'info>,
    policy: &'a AccountInfo<'info>,
    council: &'a AccountInfo<'info>,
    capacity_policy: &'a AccountInfo<'info>,
    controller_release: &'a AccountInfo<'info>,
    canonical_spill_treasury: &'a AccountInfo<'info>,
    guardian: &'a AccountInfo<'info>,
    seat_authorities: [&'a AccountInfo<'info>; 5],
    system_program: &'a AccountInfo<'info>,
}

struct PreparedInitialization {
    target_program: Pubkey,
    target_legacy_authority: Pubkey,
    policy_version: u64,
    council_version: u64,
    config_bump: u8,
    gate_bump: u8,
    policy_bump: u8,
    council_bump: u8,
    capacity_policy_bump: u8,
    controller_release_bump: u8,
    config_bytes: Vec<u8>,
    gate_bytes: Vec<u8>,
    policy_bytes: Vec<u8>,
    council_bytes: Vec<u8>,
    capacity_policy_bytes: Vec<u8>,
    controller_release_bytes: Vec<u8>,
}

/// Creates the capacity-safe Bootstrap V1 trust root for exactly one target.
///
/// The only CPIs are fixed System Program account creations. This instruction
/// does not transfer target authority, initialize the target bridge, make the
/// controller immutable, activate the target gate, or invoke Loader-v3.
pub fn process_initialize_controller_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: Box<InitializeControllerV2>,
) -> ProgramResult {
    let accounts = parse_accounts(accounts)?;
    let clock_slot = Clock::get()?.slot;
    let prepared = prepare_initialization(program_id, &accounts, &instruction, clock_slot)?;
    let rent = Rent::get()?;

    create_initial_accounts(program_id, &accounts, &prepared, &rent)?;
    store_preencoded_accounts(program_id, &accounts, &prepared)
}

#[inline(never)]
fn parse_accounts<'a, 'info>(
    accounts: &'a [AccountInfo<'info>],
) -> Result<InitializeAccounts<'a, 'info>, ProgramError> {
    let [payer, initializer, controller_program, controller_programdata, target_program, target_programdata, upgradeable_loader, controller_config, authority_pda, protocol_gate, policy, council, capacity_policy, controller_release, canonical_spill_treasury, guardian, seat_0, seat_1, seat_2, seat_3, seat_4, system_program] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    debug_assert_eq!(accounts.len(), INITIALIZE_CONTROLLER_ACCOUNT_COUNT_V2);
    Ok(InitializeAccounts {
        payer,
        initializer,
        controller_program,
        controller_programdata,
        target_program,
        target_programdata,
        upgradeable_loader,
        controller_config,
        authority_pda,
        protocol_gate,
        policy,
        council,
        capacity_policy,
        controller_release,
        canonical_spill_treasury,
        guardian,
        seat_authorities: [seat_0, seat_1, seat_2, seat_3, seat_4],
        system_program,
    })
}

#[inline(never)]
fn prepare_initialization(
    program_id: &Pubkey,
    accounts: &InitializeAccounts<'_, '_>,
    instruction: &InitializeControllerV2,
    clock_slot: u64,
) -> Result<PreparedInitialization, ProgramError> {
    validate_account_contract(program_id, accounts)?;
    validate_initial_constants(instruction, clock_slot)?;

    let target_program = *accounts.target_program.key;
    let (expected_config, config_bump) = derive_controller_config_pda(program_id, &target_program);
    let (expected_authority, _) = derive_authority_pda(program_id, &target_program);
    let (expected_gate, gate_bump) = derive_gate_pda(program_id, &target_program);
    let (expected_policy, policy_bump) =
        derive_policy_pda(program_id, &target_program, INITIAL_POLICY_VERSION_V2);
    let (expected_council, council_bump) =
        derive_council_pda(program_id, &target_program, INITIAL_COUNCIL_VERSION_V2);
    let (expected_capacity_policy, capacity_policy_bump) =
        derive_capacity_policy_pda(program_id, &target_program);
    let (expected_controller_release, controller_release_bump) =
        derive_controller_release_commitment_pda(program_id, &target_program);

    if *accounts.controller_config.key != expected_config
        || *accounts.authority_pda.key != expected_authority
        || *accounts.protocol_gate.key != expected_gate
        || *accounts.policy.key != expected_policy
        || *accounts.council.key != expected_council
        || *accounts.capacity_policy.key != expected_capacity_policy
        || *accounts.controller_release.key != expected_controller_release
    {
        return Err(GovernanceError::InvalidPda.into());
    }

    for account in [
        accounts.controller_config,
        accounts.protocol_gate,
        accounts.policy,
        accounts.council,
        accounts.capacity_policy,
        accounts.controller_release,
    ] {
        validate_uninitialized_pda(account)?;
    }
    validate_vacant_authority_pda(accounts.authority_pda)?;

    let loader_graph = validate_loader_graph(program_id, accounts, &expected_authority)?;

    let config = Box::new(build_config(
        accounts,
        instruction,
        config_bump,
        expected_authority,
        expected_gate,
    ));
    config.validate_static()?;

    let policy = Box::new(build_policy(
        accounts,
        instruction,
        policy_bump,
        *accounts.controller_config.key,
    ));
    if policy.policy_hash != instruction.expected_policy_hash {
        return Err(GovernanceError::PolicyHashMismatch.into());
    }
    validate_policy_against_config(&policy, &config)?;

    let council = Box::new(build_council(
        accounts,
        instruction,
        council_bump,
        *accounts.controller_config.key,
    ));
    if council.set_hash != instruction.expected_council_hash {
        return Err(GovernanceError::CouncilHashMismatch.into());
    }
    validate_council_set(&council, &policy)?;
    validate_council_guardian_separation(&council, accounts.guardian.key)?;
    if !council.active_at(clock_slot)
        || council.seats.iter().any(|seat| {
            !seat.term_covers(clock_slot)
                || seat.term_end_slot
                    != crate::release1_v3_instruction::BOOTSTRAP_INITIAL_SEAT_TERM_END_V2
        })
    {
        return Err(GovernanceError::InactiveCouncilSeat.into());
    }

    let gate = Box::new(build_gate(accounts, gate_bump, clock_slot));
    gate.validate_static()?;

    let capacity_policy = Box::new(build_capacity_policy(
        program_id,
        accounts,
        instruction,
        capacity_policy_bump,
        clock_slot,
    )?);
    validate_capacity_policy_digest_v1(&capacity_policy)?;

    let controller_release = Box::new(build_controller_release(
        program_id,
        accounts,
        instruction,
        controller_release_bump,
        &capacity_policy,
        loader_graph.controller_capacity,
        clock_slot,
    )?);
    validate_controller_release_digest_v1(&controller_release)?;

    Ok(PreparedInitialization {
        target_program,
        target_legacy_authority: loader_graph.target_legacy_authority,
        policy_version: INITIAL_POLICY_VERSION_V2,
        council_version: INITIAL_COUNCIL_VERSION_V2,
        config_bump,
        gate_bump,
        policy_bump,
        council_bump,
        capacity_policy_bump,
        controller_release_bump,
        config_bytes: encode_fixed_account(&config, ControllerConfigV1::LEN)?,
        gate_bytes: encode_fixed_account(&gate, ProtocolGateV1::LEN)?,
        policy_bytes: encode_fixed_account(&policy, GovernancePolicyV1::LEN)?,
        council_bytes: encode_fixed_account(&council, GovernanceCouncilSetV1::LEN)?,
        capacity_policy_bytes: encode_fixed_account(
            &capacity_policy,
            ProgramDataCapacityPolicyV1::LEN,
        )?,
        controller_release_bytes: encode_fixed_account(
            &controller_release,
            ControllerReleaseCommitmentV1::LEN,
        )?,
    })
}

#[inline(never)]
fn validate_account_contract(
    program_id: &Pubkey,
    accounts: &InitializeAccounts<'_, '_>,
) -> ProgramResult {
    validate_exact_privileges(accounts.payer, true, true, false)?;
    validate_exact_privileges(accounts.initializer, false, true, false)?;
    validate_exact_privileges(accounts.controller_program, false, false, true)?;
    validate_exact_privileges(accounts.controller_programdata, false, false, false)?;
    validate_exact_privileges(accounts.target_program, false, false, true)?;
    validate_exact_privileges(accounts.target_programdata, false, false, false)?;
    validate_exact_privileges(accounts.upgradeable_loader, false, false, true)?;
    validate_exact_privileges(accounts.controller_config, true, false, false)?;
    validate_exact_privileges(accounts.authority_pda, false, false, false)?;
    validate_exact_privileges(accounts.protocol_gate, true, false, false)?;
    validate_exact_privileges(accounts.policy, true, false, false)?;
    validate_exact_privileges(accounts.council, true, false, false)?;
    validate_exact_privileges(accounts.capacity_policy, true, false, false)?;
    validate_exact_privileges(accounts.controller_release, true, false, false)?;
    validate_exact_privileges(accounts.canonical_spill_treasury, false, false, false)?;
    validate_exact_privileges(accounts.guardian, false, false, false)?;
    for seat in accounts.seat_authorities {
        validate_exact_privileges(seat, false, false, false)?;
    }
    validate_exact_privileges(accounts.system_program, false, false, true)?;

    let all_accounts = [
        accounts.payer,
        accounts.initializer,
        accounts.controller_program,
        accounts.controller_programdata,
        accounts.target_program,
        accounts.target_programdata,
        accounts.upgradeable_loader,
        accounts.controller_config,
        accounts.authority_pda,
        accounts.protocol_gate,
        accounts.policy,
        accounts.council,
        accounts.capacity_policy,
        accounts.controller_release,
        accounts.canonical_spill_treasury,
        accounts.guardian,
        accounts.seat_authorities[0],
        accounts.seat_authorities[1],
        accounts.seat_authorities[2],
        accounts.seat_authorities[3],
        accounts.seat_authorities[4],
        accounts.system_program,
    ];
    require_distinct_accounts(&all_accounts)?;
    // The System Program is the sole canonical default-key account.
    if all_accounts[..INITIALIZE_CONTROLLER_ACCOUNT_COUNT_V2 - 1]
        .iter()
        .any(|account| *account.key == Pubkey::default())
    {
        return Err(GovernanceError::DefaultPubkey.into());
    }
    if *accounts.controller_program.key != *program_id
        || *accounts.upgradeable_loader.key != UPGRADEABLE_LOADER_ID
        || *accounts.system_program.key != system_program::ID
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

#[inline(never)]
fn validate_initial_constants(
    instruction: &InitializeControllerV2,
    clock_slot: u64,
) -> ProgramResult {
    if clock_slot == 0
        || instruction.cluster_domain == [0; 32]
        || instruction.initial_policy_version != INITIAL_POLICY_VERSION_V2
        || instruction.initial_council_version != INITIAL_COUNCIL_VERSION_V2
        || instruction.next_proposal_id != INITIAL_NEXT_PROPOSAL_ID_V2
        || instruction.target_nonce != INITIAL_TARGET_NONCE_V2
        || instruction.initial_gate_epoch != INITIAL_GATE_EPOCH_V2
        || instruction.policy_activation_slot == 0
        || instruction.policy_activation_slot > clock_slot
    {
        return Err(GovernanceError::InvalidControllerConfig.into());
    }
    Ok(())
}

fn validate_uninitialized_pda(account: &AccountInfo<'_>) -> ProgramResult {
    if account.owner != &system_program::ID || account.data_len() != 0 {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    Ok(())
}

fn validate_vacant_authority_pda(account: &AccountInfo<'_>) -> ProgramResult {
    if account.owner != &system_program::ID || account.data_len() != 0 {
        return Err(GovernanceError::InvalidRelease1Account.into());
    }
    Ok(())
}

struct LoaderGraph {
    controller_capacity: u64,
    target_legacy_authority: Pubkey,
}

#[inline(never)]
fn validate_loader_graph(
    program_id: &Pubkey,
    accounts: &InitializeAccounts<'_, '_>,
    authority_pda: &Pubkey,
) -> Result<LoaderGraph, ProgramError> {
    let canonical_controller_programdata = derive_upgradeable_programdata_address(program_id).0;
    let canonical_target_programdata =
        derive_upgradeable_programdata_address(accounts.target_program.key).0;
    if *accounts.controller_programdata.key != canonical_controller_programdata
        || *accounts.target_programdata.key != canonical_target_programdata
    {
        return Err(GovernanceError::InvalidPda.into());
    }

    let controller = validate_program_programdata_linkage(
        accounts.controller_program,
        accounts.controller_programdata,
        &UPGRADEABLE_LOADER_ID,
    )?;
    let controller_capacity =
        u64::try_from(controller.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let controller_raw_length = u64::try_from(accounts.controller_programdata.data_len())
        .map_err(|_| GovernanceError::ArithmeticOverflow)?;
    if controller.deployed_slot == 0
        || controller_capacity == 0
        || controller_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
        || controller_raw_length > MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
        || controller.upgrade_authority != Some(*accounts.initializer.key)
    {
        return Err(GovernanceError::InvalidRelease1Account.into());
    }

    let target = validate_program_programdata_linkage(
        accounts.target_program,
        accounts.target_programdata,
        &UPGRADEABLE_LOADER_ID,
    )?;
    let target_capacity =
        u64::try_from(target.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let target_raw_length = u64::try_from(accounts.target_programdata.data_len())
        .map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let Some(target_legacy_authority) = target.upgrade_authority else {
        return Err(GovernanceError::InvalidRelease1Account.into());
    };
    if target.deployed_slot == 0
        || target_capacity == 0
        || target_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
        || target_raw_length > MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
        || target_legacy_authority == *authority_pda
        || target_legacy_authority == *accounts.guardian.key
    {
        return Err(GovernanceError::InvalidRelease1Account.into());
    }

    Ok(LoaderGraph {
        controller_capacity,
        target_legacy_authority,
    })
}

fn build_config(
    accounts: &InitializeAccounts<'_, '_>,
    instruction: &InitializeControllerV2,
    bump: u8,
    authority_pda: Pubkey,
    gate_pda: Pubkey,
) -> ControllerConfigV1 {
    ControllerConfigV1 {
        discriminator: CONTROLLER_CONFIG_DISCRIMINATOR,
        version: ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        cluster_domain: instruction.cluster_domain,
        target_program: *accounts.target_program.key,
        target_programdata: *accounts.target_programdata.key,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        authority_pda,
        gate_pda,
        canonical_spill_treasury: *accounts.canonical_spill_treasury.key,
        current_council_version: INITIAL_COUNCIL_VERSION_V2,
        current_policy_version: INITIAL_POLICY_VERSION_V2,
        next_proposal_id: INITIAL_NEXT_PROPOSAL_ID_V2,
        target_nonce: INITIAL_TARGET_NONCE_V2,
        guardian: *accounts.guardian.key,
        vote_program: Pubkey::default(),
        vote_programdata: Pubkey::default(),
        vote_config: Pubkey::default(),
        vote_mint: Pubkey::default(),
        token_governance_enabled: false,
        routine_delay_slots: instruction.routine_delay_slots,
        major_delay_slots: instruction.major_delay_slots,
        rollback_delay_slots: instruction.rollback_delay_slots,
        terminal_delay_slots: instruction.terminal_delay_slots,
        vote_review_slots: instruction.vote_review_slots,
        proposal_expiry_slots: instruction.proposal_expiry_slots,
        policy_flags: 0,
        reserved: [0; CONTROLLER_CONFIG_RESERVED_LEN],
    }
}

fn build_policy(
    accounts: &InitializeAccounts<'_, '_>,
    instruction: &InitializeControllerV2,
    bump: u8,
    controller_config: Pubkey,
) -> GovernancePolicyV1 {
    let mut policy = GovernancePolicyV1 {
        discriminator: GOVERNANCE_POLICY_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        controller_config,
        version: INITIAL_POLICY_VERSION_V2,
        target_program: *accounts.target_program.key,
        activation_slot: instruction.policy_activation_slot,
        council_size: COUNCIL_SIZE_V2,
        routine_threshold: ROUTINE_THRESHOLD_V2,
        terminal_threshold: TERMINAL_THRESHOLD_V2,
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

fn build_council(
    accounts: &InitializeAccounts<'_, '_>,
    instruction: &InitializeControllerV2,
    bump: u8,
    controller_config: Pubkey,
) -> GovernanceCouncilSetV1 {
    let seats = std::array::from_fn(|index| CouncilSeatV1 {
        seat_authority: *accounts.seat_authorities[index].key,
        term_start_slot: instruction.seat_terms[index].term_start_slot,
        term_end_slot: instruction.seat_terms[index].term_end_slot,
        active: true,
        reserved: [0; COUNCIL_SEAT_RESERVED_LEN],
    });
    let mut council = GovernanceCouncilSetV1 {
        discriminator: GOVERNANCE_COUNCIL_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        controller_config,
        version: INITIAL_COUNCIL_VERSION_V2,
        target_program: *accounts.target_program.key,
        activation_slot: instruction.policy_activation_slot,
        deactivation_slot: 0,
        seats,
        routine_threshold: ROUTINE_THRESHOLD_V2,
        terminal_threshold: TERMINAL_THRESHOLD_V2,
        policy_flags: 0,
        set_hash: [0; 32],
        reserved: [0; GOVERNANCE_COUNCIL_RESERVED_LEN],
    };
    council.set_hash = compute_council_set_hash(&council);
    council
}

fn build_gate(accounts: &InitializeAccounts<'_, '_>, bump: u8, clock_slot: u64) -> ProtocolGateV1 {
    ProtocolGateV1 {
        discriminator: PROTOCOL_GATE_DISCRIMINATOR,
        version: ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        status: GateStatusV1::EmergencyFrozen,
        controller_config: *accounts.controller_config.key,
        target_program: *accounts.target_program.key,
        target_programdata: *accounts.target_programdata.key,
        epoch: INITIAL_GATE_EPOCH_V2,
        active_proposal: Pubkey::default(),
        freeze_slot: clock_slot,
        freeze_reason_code: BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
        last_completed_proposal: Pubkey::default(),
        reserved: [0; PROTOCOL_GATE_RESERVED_LEN],
    }
}

fn build_capacity_policy(
    program_id: &Pubkey,
    accounts: &InitializeAccounts<'_, '_>,
    instruction: &InitializeControllerV2,
    bump: u8,
    clock_slot: u64,
) -> Result<ProgramDataCapacityPolicyV1, ProgramError> {
    let chunk_size = PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB;
    let count =
        programdata_observation_chunk_count(MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1, chunk_size)?;
    let padded = count
        .checked_next_power_of_two()
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let depth = u8::try_from(padded.ilog2()).map_err(|_| GovernanceError::ArithmeticOverflow)?;

    let mut policy = ProgramDataCapacityPolicyV1 {
        discriminator: PROGRAMDATA_CAPACITY_POLICY_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        controller_program: *program_id,
        controller_config: *accounts.controller_config.key,
        target_program: *accounts.target_program.key,
        target_programdata: *accounts.target_programdata.key,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        loader_programdata_metadata_len: LOADER_PROGRAMDATA_METADATA_LEN as u64,
        maximum_raw_programdata_length: MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
        maximum_payload_capacity: MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1,
        maximum_artifact_length: MAX_ARTIFACT_BYTES_V1,
        observation_scheme_id: PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
        observation_chunk_size: chunk_size,
        observation_max_chunk_count: count,
        observation_padded_leaf_count: padded,
        observation_tree_depth: depth,
        observation_frontier_hash_count: PROGRAMDATA_OBSERVATION_FRONTIER_SLOTS_V1 as u8,
        artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
        artifact_chunk_size: ARTIFACT_BINDING_CHUNK_SIZE_V1,
        zero_tail_required: true,
        extend_program_checked_feature: EXTEND_PROGRAM_CHECKED_FEATURE_ID_V1,
        set_authority_checked_feature: SET_AUTHORITY_CHECKED_FEATURE_ID_V1,
        policy_digest: [0; 32],
        creation_slot: clock_slot,
        reserved: [0; PROGRAMDATA_CAPACITY_POLICY_V1_RESERVED_LEN],
    };
    policy.policy_digest = compute_capacity_policy_digest_v1(&policy)?;
    if policy.policy_digest != instruction.capacity_policy.expected_policy_digest {
        return Err(GovernanceError::CapacityPolicyMismatch.into());
    }
    Ok(policy)
}

#[allow(clippy::too_many_arguments)]
fn build_controller_release(
    program_id: &Pubkey,
    accounts: &InitializeAccounts<'_, '_>,
    instruction: &InitializeControllerV2,
    bump: u8,
    capacity_policy: &ProgramDataCapacityPolicyV1,
    controller_capacity: u64,
    clock_slot: u64,
) -> Result<ControllerReleaseCommitmentV1, ProgramError> {
    let manifest = &instruction.controller_release;
    let mut release = ControllerReleaseCommitmentV1 {
        discriminator: CONTROLLER_RELEASE_COMMITMENT_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        controller_program: *program_id,
        controller_programdata: *accounts.controller_programdata.key,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        capacity_policy: *accounts.capacity_policy.key,
        capacity_policy_digest: capacity_policy.policy_digest,
        artifact_length: manifest.artifact_length,
        artifact_sha256: manifest.artifact_sha256,
        artifact_merkle_root: manifest.artifact_merkle_root,
        artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
        source_commitment: manifest.source_commitment,
        source_tree_commitment: manifest.source_tree_commitment,
        build_inputs_commitment: manifest.build_inputs_commitment,
        toolchain_commitment: manifest.toolchain_commitment,
        package_commitment: manifest.package_commitment,
        release_manifest_commitment: manifest.release_manifest_commitment,
        abi_commitment: manifest.abi_commitment,
        pre_immutability_authority: OptionalPubkeyV1::some(*accounts.initializer.key)?,
        minimum_programdata_capacity: controller_capacity,
        release_digest: [0; 32],
        creation_slot: clock_slot,
        finalized: true,
        reserved: [0; CONTROLLER_RELEASE_COMMITMENT_V1_RESERVED_LEN],
    };
    release.release_digest = compute_controller_release_digest_v1(&release)?;
    if release.release_digest != manifest.expected_release_digest {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    Ok(release)
}

#[inline(never)]
fn create_initial_accounts<'info>(
    program_id: &Pubkey,
    accounts: &InitializeAccounts<'_, 'info>,
    prepared: &PreparedInitialization,
    rent: &Rent,
) -> ProgramResult {
    let config_bump = [prepared.config_bump];
    let gate_bump = [prepared.gate_bump];
    let policy_bump = [prepared.policy_bump];
    let council_bump = [prepared.council_bump];
    let capacity_policy_bump = [prepared.capacity_policy_bump];
    let controller_release_bump = [prepared.controller_release_bump];
    let policy_version = prepared.policy_version.to_le_bytes();
    let council_version = prepared.council_version.to_le_bytes();

    // This read makes the preservation boundary explicit: initialization has
    // proven a live legacy authority but never uses it as a signer or CPI seed.
    if prepared.target_legacy_authority == Pubkey::default() {
        return Err(GovernanceError::InvalidRelease1Account.into());
    }

    create_fixed_pda_account(
        program_id,
        accounts.payer,
        accounts.controller_config,
        accounts.system_program,
        rent,
        ControllerConfigV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            TARGET_SEED,
            prepared.target_program.as_ref(),
            &config_bump,
        ],
    )?;
    create_fixed_pda_account(
        program_id,
        accounts.payer,
        accounts.protocol_gate,
        accounts.system_program,
        rent,
        ProtocolGateV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            GATE_SEED,
            prepared.target_program.as_ref(),
            &gate_bump,
        ],
    )?;
    create_fixed_pda_account(
        program_id,
        accounts.payer,
        accounts.policy,
        accounts.system_program,
        rent,
        GovernancePolicyV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            POLICY_SEED,
            prepared.target_program.as_ref(),
            &policy_version,
            &policy_bump,
        ],
    )?;
    create_fixed_pda_account(
        program_id,
        accounts.payer,
        accounts.council,
        accounts.system_program,
        rent,
        GovernanceCouncilSetV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            COUNCIL_SEED,
            prepared.target_program.as_ref(),
            &council_version,
            &council_bump,
        ],
    )?;
    create_fixed_pda_account(
        program_id,
        accounts.payer,
        accounts.capacity_policy,
        accounts.system_program,
        rent,
        ProgramDataCapacityPolicyV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            CAPACITY_POLICY_SEED,
            prepared.target_program.as_ref(),
            &capacity_policy_bump,
        ],
    )?;
    create_fixed_pda_account(
        program_id,
        accounts.payer,
        accounts.controller_release,
        accounts.system_program,
        rent,
        ControllerReleaseCommitmentV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            CONTROLLER_RELEASE_SEED,
            prepared.target_program.as_ref(),
            &controller_release_bump,
        ],
    )
}

/// Validate and borrow every destination before copying any account image.
#[inline(never)]
fn store_preencoded_accounts(
    program_id: &Pubkey,
    accounts: &InitializeAccounts<'_, '_>,
    prepared: &PreparedInitialization,
) -> ProgramResult {
    let destinations = [
        (
            accounts.controller_config,
            prepared.config_bytes.as_slice(),
            ControllerConfigV1::LEN,
        ),
        (
            accounts.protocol_gate,
            prepared.gate_bytes.as_slice(),
            ProtocolGateV1::LEN,
        ),
        (
            accounts.policy,
            prepared.policy_bytes.as_slice(),
            GovernancePolicyV1::LEN,
        ),
        (
            accounts.council,
            prepared.council_bytes.as_slice(),
            GovernanceCouncilSetV1::LEN,
        ),
        (
            accounts.capacity_policy,
            prepared.capacity_policy_bytes.as_slice(),
            ProgramDataCapacityPolicyV1::LEN,
        ),
        (
            accounts.controller_release,
            prepared.controller_release_bytes.as_slice(),
            ControllerReleaseCommitmentV1::LEN,
        ),
    ];
    for (account, bytes, expected_len) in &destinations {
        if account.owner != program_id {
            return Err(GovernanceError::IncorrectAccountOwner.into());
        }
        if account.data_len() != *expected_len || bytes.len() != *expected_len {
            return Err(GovernanceError::InvalidAccountSize.into());
        }
    }

    let mut config_data = accounts.controller_config.try_borrow_mut_data()?;
    let mut gate_data = accounts.protocol_gate.try_borrow_mut_data()?;
    let mut policy_data = accounts.policy.try_borrow_mut_data()?;
    let mut council_data = accounts.council.try_borrow_mut_data()?;
    let mut capacity_data = accounts.capacity_policy.try_borrow_mut_data()?;
    let mut release_data = accounts.controller_release.try_borrow_mut_data()?;
    config_data.copy_from_slice(&prepared.config_bytes);
    gate_data.copy_from_slice(&prepared.gate_bytes);
    policy_data.copy_from_slice(&prepared.policy_bytes);
    council_data.copy_from_slice(&prepared.council_bytes);
    capacity_data.copy_from_slice(&prepared.capacity_policy_bytes);
    release_data.copy_from_slice(&prepared.controller_release_bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use borsh::BorshDeserialize;
    use solana_program::account_info::AccountInfo;

    use super::*;
    use crate::{
        release1_loader_accounts::{
            LOADER_PROGRAM_ACCOUNT_LEN, LOADER_STATE_TAG_PROGRAM, LOADER_STATE_TAG_PROGRAMDATA,
        },
        release1_v3_instruction::{CapacityPolicyInputV1, ControllerReleaseInputV1, SeatTermV2},
    };

    const TEST_CLOCK_SLOT: u64 = 100;

    fn key(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    fn test_account(
        key: Pubkey,
        signer: bool,
        writable: bool,
        owner: Pubkey,
        executable: bool,
        data: Vec<u8>,
    ) -> AccountInfo<'static> {
        AccountInfo::new(
            Box::leak(Box::new(key)),
            signer,
            writable,
            Box::leak(Box::new(10_000_000_000)),
            Box::leak(data.into_boxed_slice()),
            Box::leak(Box::new(owner)),
            executable,
            0,
        )
    }

    fn program_bytes(programdata: Pubkey) -> Vec<u8> {
        let mut data = vec![0; LOADER_PROGRAM_ACCOUNT_LEN];
        data[..4].copy_from_slice(&LOADER_STATE_TAG_PROGRAM.to_le_bytes());
        data[4..36].copy_from_slice(programdata.as_ref());
        data
    }

    fn programdata_bytes(slot: u64, authority: Pubkey, payload_len: usize) -> Vec<u8> {
        let mut data = vec![0; LOADER_PROGRAMDATA_METADATA_LEN + payload_len];
        data[..4].copy_from_slice(&LOADER_STATE_TAG_PROGRAMDATA.to_le_bytes());
        data[4..12].copy_from_slice(&slot.to_le_bytes());
        data[12] = 1;
        data[13..45].copy_from_slice(authority.as_ref());
        data[45..].fill(7);
        data
    }

    struct Fixture {
        program_id: Pubkey,
        target_legacy_authority: Pubkey,
        accounts: Vec<AccountInfo<'static>>,
        instruction: InitializeControllerV2,
    }

    fn fixture() -> Fixture {
        let program_id = key(200);
        let payer = key(1);
        let initializer = key(2);
        let target_program = key(201);
        let target_legacy_authority = key(3);
        let controller_programdata = derive_upgradeable_programdata_address(&program_id).0;
        let target_programdata = derive_upgradeable_programdata_address(&target_program).0;
        let controller_config = derive_controller_config_pda(&program_id, &target_program).0;
        let authority_pda = derive_authority_pda(&program_id, &target_program).0;
        let protocol_gate = derive_gate_pda(&program_id, &target_program).0;
        let policy = derive_policy_pda(&program_id, &target_program, 1).0;
        let council = derive_council_pda(&program_id, &target_program, 1).0;
        let capacity_policy = derive_capacity_policy_pda(&program_id, &target_program).0;
        let controller_release =
            derive_controller_release_commitment_pda(&program_id, &target_program).0;
        let native_owner = key(250);

        let accounts = vec![
            test_account(payer, true, true, system_program::ID, false, vec![]),
            test_account(initializer, true, false, system_program::ID, false, vec![]),
            test_account(
                program_id,
                false,
                false,
                UPGRADEABLE_LOADER_ID,
                true,
                program_bytes(controller_programdata),
            ),
            test_account(
                controller_programdata,
                false,
                false,
                UPGRADEABLE_LOADER_ID,
                false,
                programdata_bytes(80, initializer, 96),
            ),
            test_account(
                target_program,
                false,
                false,
                UPGRADEABLE_LOADER_ID,
                true,
                program_bytes(target_programdata),
            ),
            test_account(
                target_programdata,
                false,
                false,
                UPGRADEABLE_LOADER_ID,
                false,
                programdata_bytes(81, target_legacy_authority, 128),
            ),
            test_account(
                UPGRADEABLE_LOADER_ID,
                false,
                false,
                native_owner,
                true,
                vec![],
            ),
            test_account(
                controller_config,
                false,
                true,
                system_program::ID,
                false,
                vec![],
            ),
            test_account(
                authority_pda,
                false,
                false,
                system_program::ID,
                false,
                vec![],
            ),
            test_account(
                protocol_gate,
                false,
                true,
                system_program::ID,
                false,
                vec![],
            ),
            test_account(policy, false, true, system_program::ID, false, vec![]),
            test_account(council, false, true, system_program::ID, false, vec![]),
            test_account(
                capacity_policy,
                false,
                true,
                system_program::ID,
                false,
                vec![],
            ),
            test_account(
                controller_release,
                false,
                true,
                system_program::ID,
                false,
                vec![],
            ),
            test_account(key(12), false, false, system_program::ID, false, vec![]),
            test_account(key(13), false, false, system_program::ID, false, vec![]),
            test_account(key(20), false, false, system_program::ID, false, vec![]),
            test_account(key(21), false, false, system_program::ID, false, vec![]),
            test_account(key(22), false, false, system_program::ID, false, vec![]),
            test_account(key(23), false, false, system_program::ID, false, vec![]),
            test_account(key(24), false, false, system_program::ID, false, vec![]),
            test_account(system_program::ID, false, false, native_owner, true, vec![]),
        ];

        let capacity_candidate = capacity_input(
            program_id,
            controller_config,
            target_program,
            target_programdata,
            capacity_policy,
        );
        let release_candidate = release_input(
            program_id,
            controller_programdata,
            target_program,
            capacity_policy,
            capacity_candidate.expected_policy_digest,
            initializer,
        );
        let mut instruction = InitializeControllerV2 {
            cluster_domain: [9; 32],
            initial_policy_version: 1,
            initial_council_version: 1,
            next_proposal_id: 1,
            target_nonce: 1,
            initial_gate_epoch: 1,
            policy_activation_slot: 90,
            routine_delay_slots: 10,
            major_delay_slots: 20,
            rollback_delay_slots: 5,
            terminal_delay_slots: 30,
            vote_review_slots: 7,
            proposal_expiry_slots: 50,
            expected_policy_hash: [1; 32],
            expected_council_hash: [1; 32],
            seat_terms: std::array::from_fn(|_| SeatTermV2 {
                term_start_slot: 80,
                term_end_slot: u64::MAX,
            }),
            capacity_policy: capacity_candidate,
            controller_release: release_candidate,
        };
        set_expected_hashes(&program_id, &accounts, &mut instruction);
        Fixture {
            program_id,
            target_legacy_authority,
            accounts,
            instruction,
        }
    }

    fn capacity_input(
        controller_program: Pubkey,
        controller_config: Pubkey,
        target_program: Pubkey,
        target_programdata: Pubkey,
        capacity_policy: Pubkey,
    ) -> CapacityPolicyInputV1 {
        let count =
            programdata_observation_chunk_count(MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1, 16 * 1024)
                .unwrap();
        let padded = count.next_power_of_two();
        let mut input = CapacityPolicyInputV1 {
            expected_policy_digest: [0; 32],
        };
        let mut full = ProgramDataCapacityPolicyV1 {
            discriminator: PROGRAMDATA_CAPACITY_POLICY_V1_DISCRIMINATOR,
            version: CEREMONY_ACCOUNT_VERSION_V1,
            bump: derive_capacity_policy_pda(&controller_program, &target_program).1,
            initialized: true,
            controller_program,
            controller_config,
            target_program,
            target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            loader_programdata_metadata_len: LOADER_PROGRAMDATA_METADATA_LEN as u64,
            maximum_raw_programdata_length: MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
            maximum_payload_capacity: MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1,
            maximum_artifact_length: MAX_ARTIFACT_BYTES_V1,
            observation_scheme_id: PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
            observation_chunk_size: PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB,
            observation_max_chunk_count: count,
            observation_padded_leaf_count: padded,
            observation_tree_depth: padded.ilog2() as u8,
            observation_frontier_hash_count: PROGRAMDATA_OBSERVATION_FRONTIER_SLOTS_V1 as u8,
            artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            artifact_chunk_size: ARTIFACT_BINDING_CHUNK_SIZE_V1,
            zero_tail_required: true,
            extend_program_checked_feature: EXTEND_PROGRAM_CHECKED_FEATURE_ID_V1,
            set_authority_checked_feature: SET_AUTHORITY_CHECKED_FEATURE_ID_V1,
            policy_digest: [0; 32],
            creation_slot: 90,
            reserved: [0; PROGRAMDATA_CAPACITY_POLICY_V1_RESERVED_LEN],
        };
        full.policy_digest = compute_capacity_policy_digest_v1(&full).unwrap();
        input.expected_policy_digest = full.policy_digest;
        assert_eq!(
            derive_capacity_policy_pda(&controller_program, &target_program).0,
            capacity_policy
        );
        input
    }

    fn release_input(
        controller_program: Pubkey,
        controller_programdata: Pubkey,
        target_program: Pubkey,
        capacity_policy: Pubkey,
        capacity_policy_digest: [u8; 32],
        initializer: Pubkey,
    ) -> ControllerReleaseInputV1 {
        let mut input = ControllerReleaseInputV1 {
            artifact_length: 64,
            artifact_sha256: [40; 32],
            artifact_merkle_root: [41; 32],
            source_commitment: [42; 32],
            source_tree_commitment: [43; 32],
            build_inputs_commitment: [44; 32],
            toolchain_commitment: [45; 32],
            package_commitment: [46; 32],
            release_manifest_commitment: [47; 32],
            abi_commitment: [48; 32],
            expected_release_digest: [0; 32],
        };
        let mut full = ControllerReleaseCommitmentV1 {
            discriminator: CONTROLLER_RELEASE_COMMITMENT_V1_DISCRIMINATOR,
            version: CEREMONY_ACCOUNT_VERSION_V1,
            bump: derive_controller_release_commitment_pda(&controller_program, &target_program).1,
            initialized: true,
            controller_program,
            controller_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            capacity_policy,
            capacity_policy_digest,
            artifact_length: input.artifact_length,
            artifact_sha256: input.artifact_sha256,
            artifact_merkle_root: input.artifact_merkle_root,
            artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            source_commitment: input.source_commitment,
            source_tree_commitment: input.source_tree_commitment,
            build_inputs_commitment: input.build_inputs_commitment,
            toolchain_commitment: input.toolchain_commitment,
            package_commitment: input.package_commitment,
            release_manifest_commitment: input.release_manifest_commitment,
            abi_commitment: input.abi_commitment,
            pre_immutability_authority: OptionalPubkeyV1::some(initializer).unwrap(),
            minimum_programdata_capacity: 96,
            release_digest: [0; 32],
            creation_slot: 90,
            finalized: true,
            reserved: [0; CONTROLLER_RELEASE_COMMITMENT_V1_RESERVED_LEN],
        };
        full.release_digest = compute_controller_release_digest_v1(&full).unwrap();
        input.expected_release_digest = full.release_digest;
        input
    }

    fn set_expected_hashes(
        program_id: &Pubkey,
        raw_accounts: &[AccountInfo<'_>],
        instruction: &mut InitializeControllerV2,
    ) {
        let accounts = parse_accounts(raw_accounts).unwrap();
        let (_, policy_bump) = derive_policy_pda(program_id, accounts.target_program.key, 1);
        let (_, council_bump) = derive_council_pda(program_id, accounts.target_program.key, 1);
        let policy = build_policy(
            &accounts,
            instruction,
            policy_bump,
            *accounts.controller_config.key,
        );
        let council = build_council(
            &accounts,
            instruction,
            council_bump,
            *accounts.controller_config.key,
        );
        instruction.expected_policy_hash = policy.policy_hash;
        instruction.expected_council_hash = council.set_hash;
    }

    fn snapshot(accounts: &[AccountInfo<'_>]) -> Vec<(u64, Vec<u8>)> {
        accounts
            .iter()
            .map(|account| {
                (
                    account.lamports(),
                    account.try_borrow_data().unwrap().to_vec(),
                )
            })
            .collect()
    }

    fn assert_preflight_rejected_unchanged(fixture: &Fixture) {
        let before = snapshot(&fixture.accounts);
        let accounts = parse_accounts(&fixture.accounts).unwrap();
        assert!(prepare_initialization(
            &fixture.program_id,
            &accounts,
            &fixture.instruction,
            TEST_CLOCK_SLOT,
        )
        .is_err());
        assert_eq!(snapshot(&fixture.accounts), before);
    }

    #[test]
    fn exact_twenty_two_account_contract_prepares_six_canonical_accounts() {
        let fixture = fixture();
        let accounts = parse_accounts(&fixture.accounts).unwrap();
        let prepared = prepare_initialization(
            &fixture.program_id,
            &accounts,
            &fixture.instruction,
            TEST_CLOCK_SLOT,
        )
        .unwrap();

        assert_eq!(
            prepared.target_legacy_authority,
            fixture.target_legacy_authority
        );
        let config = ControllerConfigV1::try_from_slice(&prepared.config_bytes).unwrap();
        let gate = ProtocolGateV1::try_from_slice(&prepared.gate_bytes).unwrap();
        let policy = GovernancePolicyV1::try_from_slice(&prepared.policy_bytes).unwrap();
        let council = GovernanceCouncilSetV1::try_from_slice(&prepared.council_bytes).unwrap();
        let capacity =
            ProgramDataCapacityPolicyV1::try_from_slice(&prepared.capacity_policy_bytes).unwrap();
        let release =
            ControllerReleaseCommitmentV1::try_from_slice(&prepared.controller_release_bytes)
                .unwrap();

        assert_eq!(config.next_proposal_id, 1);
        assert_eq!(config.target_nonce, 1);
        assert!(!config.token_governance_enabled);
        assert_eq!(gate.status, GateStatusV1::EmergencyFrozen);
        assert_eq!(gate.epoch, 1);
        assert_eq!(
            gate.freeze_reason_code,
            BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        );
        assert_eq!(policy.routine_threshold, 3);
        assert_eq!(policy.terminal_threshold, 4);
        assert_eq!(council.seats.len(), 5);
        assert_eq!(capacity.creation_slot, TEST_CLOCK_SLOT);
        assert_eq!(release.creation_slot, TEST_CLOCK_SLOT);
        assert_eq!(release.minimum_programdata_capacity, 96);
        assert_eq!(
            capacity.policy_digest,
            fixture.instruction.capacity_policy.expected_policy_digest
        );
        assert_eq!(
            release.release_digest,
            fixture
                .instruction
                .controller_release
                .expected_release_digest
        );
        assert_eq!(
            release.pre_immutability_authority.value,
            *fixture.accounts[1].key
        );
        validate_capacity_policy_digest_v1(&capacity).unwrap();
        validate_controller_release_digest_v1(&release).unwrap();
    }

    #[test]
    fn signer_visible_candidates_must_equal_created_account_images() {
        let mut capacity_mismatch = fixture();
        capacity_mismatch
            .instruction
            .capacity_policy
            .expected_policy_digest[0] ^= 1;
        assert_preflight_rejected_unchanged(&capacity_mismatch);

        let mut release_mismatch = fixture();
        release_mismatch
            .instruction
            .controller_release
            .expected_release_digest[0] ^= 1;
        assert_preflight_rejected_unchanged(&release_mismatch);
    }

    #[test]
    fn malformed_graph_timing_seats_and_reinitialization_fail_without_mutation() {
        let mut wrong_initializer = fixture();
        wrong_initializer.accounts[1] =
            test_account(key(99), true, false, system_program::ID, false, vec![]);
        assert_preflight_rejected_unchanged(&wrong_initializer);

        let mut weak_timing = fixture();
        weak_timing.instruction.proposal_expiry_slots = 30;
        assert_preflight_rejected_unchanged(&weak_timing);

        for weaken in [
            |instruction: &mut InitializeControllerV2| instruction.rollback_delay_slots = 4,
            |instruction: &mut InitializeControllerV2| instruction.routine_delay_slots = 9,
            |instruction: &mut InitializeControllerV2| instruction.major_delay_slots = 19,
            |instruction: &mut InitializeControllerV2| instruction.terminal_delay_slots = 29,
            |instruction: &mut InitializeControllerV2| instruction.vote_review_slots = 6,
            |instruction: &mut InitializeControllerV2| instruction.proposal_expiry_slots = 49,
        ] {
            let mut weak = fixture();
            weaken(&mut weak.instruction);
            assert!(weak.instruction.pack().is_err());
            assert_preflight_rejected_unchanged(&weak);
        }

        let minimum_timing = fixture();
        minimum_timing.instruction.pack().unwrap();
        let accounts = parse_accounts(&minimum_timing.accounts).unwrap();
        prepare_initialization(
            &minimum_timing.program_id,
            &accounts,
            &minimum_timing.instruction,
            TEST_CLOCK_SLOT,
        )
        .unwrap();

        let mut duplicate_seat = fixture();
        duplicate_seat.accounts[17] = duplicate_seat.accounts[16].clone();
        assert_preflight_rejected_unchanged(&duplicate_seat);

        let mut guardian_seat = fixture();
        guardian_seat.accounts[16] = guardian_seat.accounts[15].clone();
        assert_preflight_rejected_unchanged(&guardian_seat);

        let mut writable_seat = fixture();
        let seat_key = *writable_seat.accounts[16].key;
        writable_seat.accounts[16] =
            test_account(seat_key, false, true, system_program::ID, false, vec![]);
        assert_preflight_rejected_unchanged(&writable_seat);

        let mut expiring_seat = fixture();
        expiring_seat.instruction.seat_terms[0].term_end_slot = u64::MAX - 1;
        set_expected_hashes(
            &expiring_seat.program_id,
            &expiring_seat.accounts,
            &mut expiring_seat.instruction,
        );
        assert_preflight_rejected_unchanged(&expiring_seat);

        let mut reinitialize = fixture();
        let config_key = *reinitialize.accounts[7].key;
        reinitialize.accounts[7] = test_account(
            config_key,
            false,
            true,
            reinitialize.program_id,
            false,
            vec![0; ControllerConfigV1::LEN],
        );
        assert_preflight_rejected_unchanged(&reinitialize);
    }

    #[test]
    fn wrong_count_default_identity_and_controller_owned_target_fail_closed() {
        let baseline = fixture();
        assert!(parse_accounts(&baseline.accounts[..21]).is_err());
        assert!(parse_accounts(
            &baseline
                .accounts
                .iter()
                .cloned()
                .chain(std::iter::once(baseline.accounts[21].clone()))
                .collect::<Vec<_>>()
        )
        .is_err());

        let mut default_guardian = fixture();
        default_guardian.accounts[15] = test_account(
            Pubkey::default(),
            false,
            false,
            system_program::ID,
            false,
            vec![],
        );
        assert_preflight_rejected_unchanged(&default_guardian);

        let mut controller_owned_target = fixture();
        let authority = *controller_owned_target.accounts[8].key;
        let target_programdata = *controller_owned_target.accounts[5].key;
        controller_owned_target.accounts[5] = test_account(
            target_programdata,
            false,
            false,
            UPGRADEABLE_LOADER_ID,
            false,
            programdata_bytes(81, authority, 128),
        );
        assert_preflight_rejected_unchanged(&controller_owned_target);

        let mut guardian_owned_target = fixture();
        let guardian = *guardian_owned_target.accounts[15].key;
        let target_programdata = *guardian_owned_target.accounts[5].key;
        guardian_owned_target.accounts[5] = test_account(
            target_programdata,
            false,
            false,
            UPGRADEABLE_LOADER_ID,
            false,
            programdata_bytes(81, guardian, 128),
        );
        assert_preflight_rejected_unchanged(&guardian_owned_target);

        let mut default_target_authority = fixture();
        let target_programdata = *default_target_authority.accounts[5].key;
        default_target_authority.accounts[5] = test_account(
            target_programdata,
            false,
            false,
            UPGRADEABLE_LOADER_ID,
            false,
            programdata_bytes(81, Pubkey::default(), 128),
        );
        assert_preflight_rejected_unchanged(&default_target_authority);
    }
}
