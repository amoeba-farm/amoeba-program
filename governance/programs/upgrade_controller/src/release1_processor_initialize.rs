//! Release 1 one-time controller initialization.
//!
//! The processor validates the complete controller/target Loader-v3 graph and
//! every immutable bootstrap byte before creating any account.  The four
//! retained V1 accounts are then created as fixed-size PDAs and populated only
//! after every creation succeeds.

use solana_program::{
    account_info::AccountInfo, clock::Clock, entrypoint::ProgramResult,
    program_error::ProgramError, pubkey::Pubkey, rent::Rent, sysvar::Sysvar,
};
use solana_sdk_ids::system_program;

use crate::{
    council::{compute_council_set_hash, validate_council_set},
    instruction::InitializeControllerV1,
    pda::{
        derive_authority_pda, derive_controller_config_pda, derive_council_pda, derive_gate_pda,
        derive_policy_pda, derive_upgradeable_programdata_address, COUNCIL_SEED, GATE_SEED,
        POLICY_SEED, TARGET_SEED, UPGRADEABLE_LOADER_ID, UPGRADE_SEED_DOMAIN_V1,
    },
    policy::{compute_policy_hash, validate_policy_against_config},
    release1_account_io::{
        create_fixed_pda_account, encode_fixed_account, require_distinct_accounts,
        validate_exact_privileges,
    },
    release1_loader_accounts::validate_program_programdata_linkage,
    release1_state::{
        BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1, MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
    },
    state::{
        ControllerConfigV1, CouncilSeatV1, GateStatusV1, GovernanceCouncilSetV1, GovernanceModeV1,
        GovernancePolicyV1, ProtocolGateV1, ACCOUNT_VERSION_V1, CONTROLLER_CONFIG_DISCRIMINATOR,
        CONTROLLER_CONFIG_RESERVED_LEN, COUNCIL_SEAT_RESERVED_LEN,
        GOVERNANCE_COUNCIL_DISCRIMINATOR, GOVERNANCE_COUNCIL_RESERVED_LEN,
        GOVERNANCE_POLICY_DISCRIMINATOR, GOVERNANCE_POLICY_RESERVED_LEN,
        PROTOCOL_GATE_DISCRIMINATOR, PROTOCOL_GATE_RESERVED_LEN,
    },
    GovernanceError,
};

const INITIAL_POLICY_VERSION_V1: u64 = 1;
const INITIAL_COUNCIL_VERSION_V1: u64 = 1;
const INITIAL_NEXT_PROPOSAL_ID_V1: u64 = 1;
const INITIAL_TARGET_NONCE_V1: u64 = 1;
const INITIAL_GATE_EPOCH_V1: u64 = 1;
const COUNCIL_SIZE_V1: u8 = 5;
const ROUTINE_THRESHOLD_V1: u8 = 3;
const TERMINAL_THRESHOLD_V1: u8 = 4;
const INITIALIZE_CONTROLLER_ACCOUNT_COUNT_V1: usize = 20;

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
    canonical_spill_treasury: &'a AccountInfo<'info>,
    guardian: &'a AccountInfo<'info>,
    seat_authorities: [&'a AccountInfo<'info>; 5],
    system_program: &'a AccountInfo<'info>,
}

struct PreparedInitialization {
    target_program: Pubkey,
    policy_version: u64,
    council_version: u64,
    config_bump: u8,
    gate_bump: u8,
    policy_bump: u8,
    council_bump: u8,
    config_bytes: Vec<u8>,
    gate_bytes: Vec<u8>,
    policy_bytes: Vec<u8>,
    council_bytes: Vec<u8>,
}

/// Creates the exact Bootstrap V1 trust-root accounts for one target.
///
/// This instruction does not deploy a controller, transfer target authority,
/// initialize the target bridge, or activate the target gate.
pub fn process_initialize_controller_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: InitializeControllerV1,
) -> ProgramResult {
    let accounts = parse_accounts(accounts)?;
    let clock_slot = Clock::get()?.slot;
    let prepared = prepare_initialization(program_id, &accounts, &instruction, clock_slot)?;
    let rent = Rent::get()?;

    create_initial_accounts(program_id, &accounts, &prepared, &rent)?;
    store_preencoded_accounts(program_id, &accounts, &prepared)
}

fn parse_accounts<'a, 'info>(
    accounts: &'a [AccountInfo<'info>],
) -> Result<InitializeAccounts<'a, 'info>, ProgramError> {
    let [payer, initializer, controller_program, controller_programdata, target_program, target_programdata, upgradeable_loader, controller_config, authority_pda, protocol_gate, policy, council, canonical_spill_treasury, guardian, seat_0, seat_1, seat_2, seat_3, seat_4, system_program] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    debug_assert_eq!(accounts.len(), INITIALIZE_CONTROLLER_ACCOUNT_COUNT_V1);
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
        canonical_spill_treasury,
        guardian,
        seat_authorities: [seat_0, seat_1, seat_2, seat_3, seat_4],
        system_program,
    })
}

fn prepare_initialization(
    program_id: &Pubkey,
    accounts: &InitializeAccounts<'_, '_>,
    instruction: &InitializeControllerV1,
    clock_slot: u64,
) -> Result<PreparedInitialization, ProgramError> {
    validate_account_contract(program_id, accounts)?;
    validate_initial_constants(instruction, clock_slot)?;

    let target_program = *accounts.target_program.key;
    let (expected_config, config_bump) = derive_controller_config_pda(program_id, &target_program);
    let (expected_authority, _) = derive_authority_pda(program_id, &target_program);
    let (expected_gate, gate_bump) = derive_gate_pda(program_id, &target_program);
    let (expected_policy, policy_bump) =
        derive_policy_pda(program_id, &target_program, INITIAL_POLICY_VERSION_V1);
    let (expected_council, council_bump) =
        derive_council_pda(program_id, &target_program, INITIAL_COUNCIL_VERSION_V1);
    if *accounts.controller_config.key != expected_config
        || *accounts.authority_pda.key != expected_authority
        || *accounts.protocol_gate.key != expected_gate
        || *accounts.policy.key != expected_policy
        || *accounts.council.key != expected_council
    {
        return Err(GovernanceError::InvalidPda.into());
    }

    validate_uninitialized_pda(accounts.controller_config)?;
    validate_uninitialized_pda(accounts.protocol_gate)?;
    validate_uninitialized_pda(accounts.policy)?;
    validate_uninitialized_pda(accounts.council)?;
    validate_vacant_authority_pda(accounts.authority_pda)?;

    validate_loader_graph(program_id, accounts, &expected_authority)?;

    let config = build_config(
        accounts,
        instruction,
        config_bump,
        expected_authority,
        expected_gate,
    );
    config.validate_static()?;

    let policy = build_policy(
        accounts,
        instruction,
        policy_bump,
        *accounts.controller_config.key,
    );
    if policy.policy_hash != instruction.expected_policy_hash {
        return Err(GovernanceError::PolicyHashMismatch.into());
    }
    validate_policy_against_config(&policy, &config)?;

    let council = build_council(
        accounts,
        instruction,
        council_bump,
        *accounts.controller_config.key,
    );
    if council.set_hash != instruction.expected_council_hash {
        return Err(GovernanceError::CouncilHashMismatch.into());
    }
    validate_council_set(&council, &policy)?;
    if !council.active_at(clock_slot)
        || council
            .seats
            .iter()
            .any(|seat| !seat.term_covers(clock_slot))
    {
        return Err(GovernanceError::InactiveCouncilSeat.into());
    }

    let gate = build_gate(accounts, gate_bump, clock_slot);
    gate.validate_static()?;

    Ok(PreparedInitialization {
        target_program,
        policy_version: INITIAL_POLICY_VERSION_V1,
        council_version: INITIAL_COUNCIL_VERSION_V1,
        config_bump,
        gate_bump,
        policy_bump,
        council_bump,
        config_bytes: encode_fixed_account(&config, ControllerConfigV1::LEN)?,
        gate_bytes: encode_fixed_account(&gate, ProtocolGateV1::LEN)?,
        policy_bytes: encode_fixed_account(&policy, GovernancePolicyV1::LEN)?,
        council_bytes: encode_fixed_account(&council, GovernanceCouncilSetV1::LEN)?,
    })
}

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
    // The canonical System Program ID is the all-zero public key.  It is the
    // one required account for which the default-key representation is valid.
    if all_accounts[..INITIALIZE_CONTROLLER_ACCOUNT_COUNT_V1 - 1]
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

fn validate_initial_constants(
    instruction: &InitializeControllerV1,
    clock_slot: u64,
) -> ProgramResult {
    if clock_slot == 0
        || instruction.cluster_domain == [0; 32]
        || instruction.initial_policy_version != INITIAL_POLICY_VERSION_V1
        || instruction.initial_council_version != INITIAL_COUNCIL_VERSION_V1
        || instruction.next_proposal_id != INITIAL_NEXT_PROPOSAL_ID_V1
        || instruction.target_nonce != INITIAL_TARGET_NONCE_V1
        || instruction.initial_gate_epoch != INITIAL_GATE_EPOCH_V1
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

fn validate_loader_graph(
    program_id: &Pubkey,
    accounts: &InitializeAccounts<'_, '_>,
    authority_pda: &Pubkey,
) -> ProgramResult {
    let canonical_controller_programdata = derive_upgradeable_programdata_address(program_id).0;
    let canonical_target_programdata =
        derive_upgradeable_programdata_address(accounts.target_program.key).0;
    if *accounts.controller_programdata.key != canonical_controller_programdata
        || *accounts.target_programdata.key != canonical_target_programdata
    {
        return Err(GovernanceError::InvalidPda.into());
    }

    let controller_programdata = validate_program_programdata_linkage(
        accounts.controller_program,
        accounts.controller_programdata,
        &UPGRADEABLE_LOADER_ID,
    )?;
    if controller_programdata.deployed_slot == 0
        || controller_programdata.capacity == 0
        || controller_programdata.upgrade_authority != Some(*accounts.initializer.key)
    {
        return Err(GovernanceError::InvalidRelease1Account.into());
    }

    let target_programdata = validate_program_programdata_linkage(
        accounts.target_program,
        accounts.target_programdata,
        &UPGRADEABLE_LOADER_ID,
    )?;
    let target_programdata_data_length = u64::try_from(accounts.target_programdata.data_len())
        .map_err(|_| GovernanceError::ArithmeticOverflow)?;
    // The controller ProgramData only establishes initializer authority and is
    // intentionally not capacity-capped here. The protected target must fit
    // the fixed one-pass raw-hash ceiling, or both proposal creation and a
    // governed emergency resume would be impossible immediately after init.
    if target_programdata.deployed_slot == 0
        || target_programdata.capacity == 0
        || target_programdata_data_length > MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
    {
        return Err(GovernanceError::InvalidRelease1Account.into());
    }
    let Some(target_upgrade_authority) = target_programdata.upgrade_authority else {
        return Err(GovernanceError::InvalidRelease1Account.into());
    };
    if target_upgrade_authority == *authority_pda {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    Ok(())
}

fn build_config(
    accounts: &InitializeAccounts<'_, '_>,
    instruction: &InitializeControllerV1,
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
        current_council_version: INITIAL_COUNCIL_VERSION_V1,
        current_policy_version: INITIAL_POLICY_VERSION_V1,
        next_proposal_id: INITIAL_NEXT_PROPOSAL_ID_V1,
        target_nonce: INITIAL_TARGET_NONCE_V1,
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
    instruction: &InitializeControllerV1,
    bump: u8,
    controller_config: Pubkey,
) -> GovernancePolicyV1 {
    let mut policy = GovernancePolicyV1 {
        discriminator: GOVERNANCE_POLICY_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump,
        initialized: true,
        controller_config,
        version: INITIAL_POLICY_VERSION_V1,
        target_program: *accounts.target_program.key,
        activation_slot: instruction.policy_activation_slot,
        council_size: COUNCIL_SIZE_V1,
        routine_threshold: ROUTINE_THRESHOLD_V1,
        terminal_threshold: TERMINAL_THRESHOLD_V1,
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
    instruction: &InitializeControllerV1,
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
        version: INITIAL_COUNCIL_VERSION_V1,
        target_program: *accounts.target_program.key,
        activation_slot: instruction.policy_activation_slot,
        deactivation_slot: 0,
        seats,
        routine_threshold: ROUTINE_THRESHOLD_V1,
        terminal_threshold: TERMINAL_THRESHOLD_V1,
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
        epoch: INITIAL_GATE_EPOCH_V1,
        active_proposal: Pubkey::default(),
        freeze_slot: clock_slot,
        freeze_reason_code: BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
        last_completed_proposal: Pubkey::default(),
        reserved: [0; PROTOCOL_GATE_RESERVED_LEN],
    }
}

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
    let policy_version = prepared.policy_version.to_le_bytes();
    let council_version = prepared.council_version.to_le_bytes();

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
    )
}

/// The shared fixed-account writer accepts typed values one at a time.  This
/// private commit helper instead validates and borrows all four already-created
/// destinations before copying any prevalidated byte, preserving one explicit
/// all-checks-before-account-write boundary.
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
    config_data.copy_from_slice(&prepared.config_bytes);
    gate_data.copy_from_slice(&prepared.gate_bytes);
    policy_data.copy_from_slice(&prepared.policy_bytes);
    council_data.copy_from_slice(&prepared.council_bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use borsh::BorshDeserialize;
    use solana_program::account_info::AccountInfo;

    use super::*;
    use crate::{
        instruction::CouncilSeatTermV1,
        release1_loader_accounts::{
            LOADER_PROGRAMDATA_METADATA_LEN, LOADER_PROGRAM_ACCOUNT_LEN, LOADER_STATE_TAG_PROGRAM,
            LOADER_STATE_TAG_PROGRAMDATA,
        },
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
        accounts: Vec<AccountInfo<'static>>,
        instruction: InitializeControllerV1,
    }

    fn fixture() -> Fixture {
        let program_id = key(200);
        let payer = key(1);
        let initializer = key(2);
        let target_program = key(201);
        let target_authority = key(3);
        let controller_programdata = derive_upgradeable_programdata_address(&program_id).0;
        let target_programdata = derive_upgradeable_programdata_address(&target_program).0;
        let controller_config = derive_controller_config_pda(&program_id, &target_program).0;
        let authority_pda = derive_authority_pda(&program_id, &target_program).0;
        let protocol_gate = derive_gate_pda(&program_id, &target_program).0;
        let policy = derive_policy_pda(&program_id, &target_program, 1).0;
        let council = derive_council_pda(&program_id, &target_program, 1).0;
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
                programdata_bytes(80, initializer, 64),
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
                programdata_bytes(81, target_authority, 96),
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
            test_account(key(12), false, false, system_program::ID, false, vec![]),
            test_account(key(13), false, false, system_program::ID, false, vec![]),
            test_account(key(20), false, false, system_program::ID, false, vec![]),
            test_account(key(21), false, false, system_program::ID, false, vec![]),
            test_account(key(22), false, false, system_program::ID, false, vec![]),
            test_account(key(23), false, false, system_program::ID, false, vec![]),
            test_account(key(24), false, false, system_program::ID, false, vec![]),
            test_account(system_program::ID, false, false, native_owner, true, vec![]),
        ];
        let mut instruction = InitializeControllerV1 {
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
            expected_policy_hash: [0; 32],
            expected_council_hash: [0; 32],
            seat_terms: [
                CouncilSeatTermV1 {
                    term_start_slot: 80,
                    term_end_slot: 1_000,
                },
                CouncilSeatTermV1 {
                    term_start_slot: 80,
                    term_end_slot: 1_001,
                },
                CouncilSeatTermV1 {
                    term_start_slot: 80,
                    term_end_slot: 1_002,
                },
                CouncilSeatTermV1 {
                    term_start_slot: 80,
                    term_end_slot: 1_003,
                },
                CouncilSeatTermV1 {
                    term_start_slot: 80,
                    term_end_slot: 1_004,
                },
            ],
        };
        set_expected_hashes(&program_id, &accounts, &mut instruction);
        Fixture {
            program_id,
            accounts,
            instruction,
        }
    }

    fn set_expected_hashes(
        program_id: &Pubkey,
        raw_accounts: &[AccountInfo<'_>],
        instruction: &mut InitializeControllerV1,
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

    fn materialize_controller_destinations(fixture: &mut Fixture) {
        for (index, length) in [
            (7, ControllerConfigV1::LEN),
            (9, ProtocolGateV1::LEN),
            (10, GovernancePolicyV1::LEN),
            (11, GovernanceCouncilSetV1::LEN),
        ] {
            let address = *fixture.accounts[index].key;
            fixture.accounts[index] = test_account(
                address,
                false,
                true,
                fixture.program_id,
                false,
                vec![0; length],
            );
        }
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
    fn preflight_builds_exact_canonical_frozen_bootstrap() {
        let fixture = fixture();
        let accounts = parse_accounts(&fixture.accounts).unwrap();
        let prepared = prepare_initialization(
            &fixture.program_id,
            &accounts,
            &fixture.instruction,
            TEST_CLOCK_SLOT,
        )
        .unwrap();

        let config = ControllerConfigV1::try_from_slice(&prepared.config_bytes).unwrap();
        let policy = GovernancePolicyV1::try_from_slice(&prepared.policy_bytes).unwrap();
        let council = GovernanceCouncilSetV1::try_from_slice(&prepared.council_bytes).unwrap();
        let gate = ProtocolGateV1::try_from_slice(&prepared.gate_bytes).unwrap();
        assert_eq!(config.current_policy_version, 1);
        assert_eq!(config.current_council_version, 1);
        assert_eq!(config.next_proposal_id, 1);
        assert_eq!(config.target_nonce, 1);
        assert!(!config.token_governance_enabled);
        assert_eq!(config.vote_program, Pubkey::default());
        assert_eq!(policy.policy_hash, fixture.instruction.expected_policy_hash);
        assert_eq!(council.set_hash, fixture.instruction.expected_council_hash);
        assert_eq!(council.seats.len(), 5);
        assert!(council.seats.iter().all(|seat| seat.active));
        assert_eq!(gate.status, GateStatusV1::EmergencyFrozen);
        assert_eq!(gate.epoch, 1);
        assert_eq!(gate.freeze_slot, TEST_CLOCK_SLOT);
        assert_eq!(
            gate.freeze_reason_code,
            BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        );
        assert_eq!(gate.active_proposal, Pubkey::default());
        assert_eq!(gate.last_completed_proposal, Pubkey::default());
    }

    #[test]
    fn preencoded_commit_writes_all_four_accounts_or_none() {
        let mut success = fixture();
        let prepared = {
            let accounts = parse_accounts(&success.accounts).unwrap();
            prepare_initialization(
                &success.program_id,
                &accounts,
                &success.instruction,
                TEST_CLOCK_SLOT,
            )
            .unwrap()
        };
        materialize_controller_destinations(&mut success);
        let accounts = parse_accounts(&success.accounts).unwrap();
        store_preencoded_accounts(&success.program_id, &accounts, &prepared).unwrap();
        assert_eq!(
            &**success.accounts[7].try_borrow_data().unwrap(),
            prepared.config_bytes.as_slice()
        );
        assert_eq!(
            &**success.accounts[9].try_borrow_data().unwrap(),
            prepared.gate_bytes.as_slice()
        );
        assert_eq!(
            &**success.accounts[10].try_borrow_data().unwrap(),
            prepared.policy_bytes.as_slice()
        );
        assert_eq!(
            &**success.accounts[11].try_borrow_data().unwrap(),
            prepared.council_bytes.as_slice()
        );

        let mut wrong_final_size = fixture();
        let prepared = {
            let accounts = parse_accounts(&wrong_final_size.accounts).unwrap();
            prepare_initialization(
                &wrong_final_size.program_id,
                &accounts,
                &wrong_final_size.instruction,
                TEST_CLOCK_SLOT,
            )
            .unwrap()
        };
        materialize_controller_destinations(&mut wrong_final_size);
        let council_address = *wrong_final_size.accounts[11].key;
        wrong_final_size.accounts[11] = test_account(
            council_address,
            false,
            true,
            wrong_final_size.program_id,
            false,
            vec![0; GovernanceCouncilSetV1::LEN - 1],
        );
        let before = snapshot(&wrong_final_size.accounts);
        let accounts = parse_accounts(&wrong_final_size.accounts).unwrap();
        assert!(
            store_preencoded_accounts(&wrong_final_size.program_id, &accounts, &prepared).is_err()
        );
        assert_eq!(snapshot(&wrong_final_size.accounts), before);
    }

    #[test]
    fn malformed_account_graph_and_reinitialization_are_failure_atomic() {
        let mut duplicate_seat = fixture();
        duplicate_seat.accounts[15] = duplicate_seat.accounts[14].clone();
        assert_preflight_rejected_unchanged(&duplicate_seat);

        let mut writable_seat = fixture();
        writable_seat.accounts[14].is_writable = true;
        assert_preflight_rejected_unchanged(&writable_seat);

        let mut executable_seat = fixture();
        executable_seat.accounts[14].executable = true;
        assert_preflight_rejected_unchanged(&executable_seat);

        let wrong_controller_authority = fixture();
        wrong_controller_authority.accounts[3]
            .try_borrow_mut_data()
            .unwrap()[13..45]
            .copy_from_slice(key(99).as_ref());
        assert_preflight_rejected_unchanged(&wrong_controller_authority);

        let wrong_target_link = fixture();
        wrong_target_link.accounts[4].try_borrow_mut_data().unwrap()[4..36]
            .copy_from_slice(key(98).as_ref());
        assert_preflight_rejected_unchanged(&wrong_target_link);

        let mut oversized_target = fixture();
        let target_programdata_key = *oversized_target.accounts[5].key;
        let oversized_payload_length = usize::try_from(
            MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 - LOADER_PROGRAMDATA_METADATA_LEN as u64
                + 1,
        )
        .unwrap();
        oversized_target.accounts[5] = test_account(
            target_programdata_key,
            false,
            false,
            UPGRADEABLE_LOADER_ID,
            false,
            programdata_bytes(81, key(3), oversized_payload_length),
        );
        assert_preflight_rejected_unchanged(&oversized_target);

        let mut reinitialization = fixture();
        let config_key = *reinitialization.accounts[7].key;
        reinitialization.accounts[7] = test_account(
            config_key,
            false,
            true,
            reinitialization.program_id,
            false,
            vec![11; ControllerConfigV1::LEN],
        );
        assert_preflight_rejected_unchanged(&reinitialization);
    }

    #[test]
    fn constants_timing_hashes_and_seat_terms_fail_closed_without_writes() {
        for mutate in [
            |instruction: &mut InitializeControllerV1| instruction.initial_policy_version = 2,
            |instruction: &mut InitializeControllerV1| instruction.initial_council_version = 2,
            |instruction: &mut InitializeControllerV1| instruction.next_proposal_id = 2,
            |instruction: &mut InitializeControllerV1| instruction.target_nonce = 2,
            |instruction: &mut InitializeControllerV1| instruction.initial_gate_epoch = 2,
            |instruction: &mut InitializeControllerV1| instruction.routine_delay_slots = 0,
            |instruction: &mut InitializeControllerV1| instruction.expected_policy_hash[0] ^= 1,
            |instruction: &mut InitializeControllerV1| instruction.expected_council_hash[0] ^= 1,
        ] {
            let mut invalid = fixture();
            mutate(&mut invalid.instruction);
            assert_preflight_rejected_unchanged(&invalid);
        }

        let mut expired_seat = fixture();
        expired_seat.instruction.seat_terms[0].term_end_slot = TEST_CLOCK_SLOT;
        set_expected_hashes(
            &expired_seat.program_id,
            &expired_seat.accounts,
            &mut expired_seat.instruction,
        );
        assert_preflight_rejected_unchanged(&expired_seat);

        let mut future_policy = fixture();
        future_policy.instruction.policy_activation_slot = TEST_CLOCK_SLOT + 1;
        set_expected_hashes(
            &future_policy.program_id,
            &future_policy.accounts,
            &mut future_policy.instruction,
        );
        assert_preflight_rejected_unchanged(&future_policy);
    }

    #[test]
    fn wrong_count_default_or_nonvacant_authority_fails_closed() {
        let baseline = fixture();
        let before = snapshot(&baseline.accounts);
        assert!(parse_accounts(&baseline.accounts[..19]).is_err());
        assert_eq!(snapshot(&baseline.accounts), before);

        let mut default_seat = fixture();
        default_seat.accounts[14] = test_account(
            Pubkey::default(),
            false,
            false,
            system_program::ID,
            false,
            vec![],
        );
        assert_preflight_rejected_unchanged(&default_seat);

        let mut occupied_authority = fixture();
        let authority = *occupied_authority.accounts[8].key;
        occupied_authority.accounts[8] =
            test_account(authority, false, false, key(77), false, vec![1]);
        assert_preflight_rejected_unchanged(&occupied_authority);
    }
}
