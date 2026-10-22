use crate::{
    artifact::{self, CHUNK, MAX_CHUNKS},
    instruction::Instruction,
    state::*,
};
mod retirement;
mod verification;
use borsh::BorshSerialize;
use solana_loader_v3_interface::instruction::{
    close, set_buffer_authority_checked, set_upgrade_authority_checked, upgrade,
};
use solana_program::{
    account_info::AccountInfo,
    clock::Clock,
    instruction::{AccountMeta, Instruction as SolanaInstruction},
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    sysvar::{instructions, Sysvar},
};
use solana_sdk_ids::{
    bpf_loader_upgradeable::ID as LOADER, compute_budget, system_program, sysvar,
};
use upgrade_controller::{
    release1_account_io::{
        create_fixed_pda_account, load_fixed_controller_account, require_distinct_accounts,
        store_fixed_controller_account, validate_exact_privileges,
    },
    release1_loader_accounts::{validate_buffer_account, validate_program_programdata_linkage},
    state::{GateStatusV1, ProtocolGateV1},
};

fn privileges(account: &AccountInfo<'_>, write: bool, sign: bool, executable: bool) -> Result<()> {
    validate_exact_privileges(account, write, sign, executable)
}
fn readonly(account: &AccountInfo<'_>) -> Result<()> {
    privileges(account, false, false, false)
}
fn writable(account: &AccountInfo<'_>) -> Result<()> {
    privileges(account, true, false, false)
}
fn signer(account: &AccountInfo<'_>) -> Result<()> {
    privileges(account, false, true, false)
}
fn executable(account: &AccountInfo<'_>, key: &Pubkey) -> Result<()> {
    privileges(account, false, false, true)?;
    require(account.key == key, Error::InvalidAccount)
}
fn config(program: &Pubkey, info: &AccountInfo<'_>) -> Result<Box<Config>> {
    let value: Box<Config> = load_fixed_controller_account(program, info, CONFIG_LEN)?;
    value.validate(program, info.key)?;
    Ok(value)
}
fn proposal(
    program: &Pubkey,
    info: &AccountInfo<'_>,
    digest: Option<[u8; 32]>,
) -> Result<Box<Proposal>> {
    let value: Box<Proposal> = load_fixed_controller_account(program, info, PROPOSAL_LEN)?;
    value.validate(program, info.key)?;
    require(
        digest.is_none_or(|hash| hash == value.digest),
        Error::StaleProposal,
    )?;
    Ok(value)
}
fn seat(config: &Config, info: &AccountInfo<'_>) -> Result<usize> {
    signer(info)?;
    config
        .seats
        .iter()
        .position(|key| key == info.key)
        .ok_or(Error::Unauthorized.into())
}
fn save_proposal(program: &Pubkey, info: &AccountInfo<'_>, value: &Proposal) -> Result<()> {
    value.validate(program, info.key)?;
    store_fixed_controller_account(program, info, value, PROPOSAL_LEN)
}
fn clock() -> Result<u64> {
    let slot = Clock::get()?.slot;
    require(slot > 0, Error::InvalidTiming)?;
    Ok(slot)
}
fn sealed(
    program: &Pubkey,
    proposal_key: &Pubkey,
    buffer: &AccountInfo<'_>,
    action: &Artifact,
) -> Result<()> {
    let header = validate_buffer_account(buffer, &LOADER)?;
    require(
        *buffer.key == action.buffer
            && header.payload_length as u64 == action.artifact_length
            && header.authority == Some(buffer_authority_pda(program, proposal_key).0),
        Error::InvalidArtifact,
    )
}
/// Every real chunk, and no bit beyond them, across the full 128-bit bitmap.
fn complete(proposal: &Proposal) -> Result<()> {
    let action = proposal.action.artifact()?;
    let chunks = artifact::chunk_count(action.artifact_length)?;
    require(proposal.verified_count == chunks, Error::InvalidArtifact)?;
    let bitmap = proposal.verified_bitmap();
    for index in 0..MAX_CHUNKS {
        let set = bitmap[index / 8] & (1 << (index % 8)) != 0;
        require(set == (index < chunks as usize), Error::InvalidArtifact)?;
    }
    Ok(())
}

fn load_gate(
    program: &Pubkey,
    info: &AccountInfo<'_>,
    target: &Pubkey,
) -> Result<Box<ProtocolGateV1>> {
    let gate: Box<ProtocolGateV1> =
        load_fixed_controller_account(program, info, ProtocolGateV1::LEN)?;
    gate.validate_static()?;
    require(
        *target != *program
            && gate_pda(program, target) == (*info.key, gate.bump)
            && gate.controller_config == config_pda(program).0
            && gate.target_program == *target
            && gate.target_programdata
                == Pubkey::find_program_address(&[target.as_ref()], &LOADER).0,
        Error::InvalidAccount,
    )?;
    Ok(gate)
}
fn save_gate(program: &Pubkey, info: &AccountInfo<'_>, gate: &ProtocolGateV1) -> Result<()> {
    gate.validate_static()?;
    store_fixed_controller_account(program, info, gate, ProtocolGateV1::LEN)
}
fn split_target_accounts<'a, 'b>(
    accounts: &'a [AccountInfo<'b>],
    target_mode: bool,
) -> Result<(&'a [AccountInfo<'b>], Option<&'a AccountInfo<'b>>)> {
    if target_mode {
        let (gate, remaining) = accounts
            .split_last()
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        writable(gate)?;
        Ok((remaining, Some(gate)))
    } else {
        Ok((accounts, None))
    }
}
fn upgrade_authority_seeds<'a>(
    target: &'a Pubkey,
    bump: &'a [u8; 1],
    target_mode: bool,
) -> Vec<&'a [u8]> {
    if target_mode {
        vec![DOMAIN, b"target-authority", target.as_ref(), bump]
    } else {
        vec![DOMAIN, b"authority", bump]
    }
}
fn upgrade_scope(
    program: &Pubkey,
    p: &Proposal,
    target: &AccountInfo<'_>,
    programdata: &AccountInfo<'_>,
    gate_info: Option<&AccountInfo<'_>>,
) -> Result<(Pubkey, u8)> {
    require(
        *programdata.key == Pubkey::find_program_address(&[target.key.as_ref()], &LOADER).0,
        Error::InvalidAccount,
    )?;
    if let Some(info) = gate_info {
        let action = p.action.target_upgrade()?;
        require(action.target == *target.key, Error::InvalidAccount)?;
        let gate = load_gate(program, info, target.key)?;
        // A separate quorum action must freeze an active target first. Code
        // installation always leaves it frozen; activation is never implicit.
        require(
            gate.epoch == action.gate_epoch && gate.status == GateStatusV1::EmergencyFrozen,
            Error::StaleProposal,
        )?;
        Ok(target_authority_pda(program, target.key))
    } else {
        require(
            p.action.kind == UPGRADE_CONTROLLER && *target.key == *program,
            Error::InvalidAccount,
        )?;
        // Installing any other controller release retires this proposal.
        require(
            p.action.upgrade()?.controller_release == CONTROLLER_RELEASE,
            Error::StaleProposal,
        )?;
        Ok(authority_pda(program))
    }
}

/// A fresh target's deployer and three current council seats consent to the
/// exact target in one transaction. No existing target can be reinitialized.
fn register_target(program: &Pubkey, accounts: &[AccountInfo<'_>], data: &[u8]) -> Result<()> {
    let [payer, deployer, config_info, target, programdata, gate_info, authority, loader, system, a, b, c, ixs] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    privileges(payer, true, true, false)?;
    signer(deployer)?;
    readonly(config_info)?;
    privileges(target, false, false, true)?;
    writable(programdata)?;
    writable(gate_info)?;
    readonly(authority)?;
    executable(loader, &LOADER)?;
    executable(system, &system_program::ID)?;
    envelope(program, accounts, ixs, data)?;
    let config = config(program, config_info)?;
    for consent in [a, b, c] {
        seat(&config, consent)?;
    }
    let (gate_key, bump) = gate_pda(program, target.key);
    let (authority_key, authority_bump) = target_authority_pda(program, target.key);
    require(
        *target.key != *program
            && *gate_info.key == gate_key
            && *authority.key == authority_key
            && *programdata.key == Pubkey::find_program_address(&[target.key.as_ref()], &LOADER).0,
        Error::InvalidAccount,
    )?;
    let before = validate_program_programdata_linkage(target, programdata, &LOADER)?;
    require(
        before.upgrade_authority == Some(*deployer.key),
        Error::Unauthorized,
    )?;
    let gate = ProtocolGateV1 {
        discriminator: *b"AGVGAT01",
        version: 1,
        bump,
        initialized: true,
        status: GateStatusV1::EmergencyFrozen,
        controller_config: *config_info.key,
        target_program: *target.key,
        target_programdata: *programdata.key,
        epoch: 1,
        active_proposal: Pubkey::default(),
        freeze_slot: clock()?,
        freeze_reason_code: 1,
        last_completed_proposal: Pubkey::default(),
        reserved: [0; 2],
    };
    create_fixed_pda_account(
        program,
        payer,
        gate_info,
        system,
        &Rent::get()?,
        ProtocolGateV1::LEN,
        &[DOMAIN, b"gate", target.key.as_ref(), &[bump]],
    )?;
    save_gate(program, gate_info, &gate)?;
    invoke_signed(
        &set_upgrade_authority_checked(target.key, deployer.key, authority.key),
        &[
            programdata.clone(),
            deployer.clone(),
            authority.clone(),
            loader.clone(),
        ],
        &[&[
            DOMAIN,
            b"target-authority",
            target.key.as_ref(),
            &[authority_bump],
        ]],
    )?;
    let after = validate_program_programdata_linkage(target, programdata, &LOADER)?;
    require(
        after.upgrade_authority == Some(authority_key)
            && after.capacity == before.capacity
            && after.deployed_slot == before.deployed_slot,
        Error::InvalidAccount,
    )
}

fn execute_target_gate(
    program: &Pubkey,
    accounts: &[AccountInfo<'_>],
    digest: [u8; 32],
) -> Result<()> {
    let [config_info, proposal_info, gate_info] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    readonly(config_info)?;
    writable(proposal_info)?;
    writable(gate_info)?;
    let config = config(program, config_info)?;
    let mut p = proposal(program, proposal_info, Some(digest))?;
    let slot = clock()?;
    p.executable(&config, slot)?;
    let action = p.action.gate_policy()?;
    let mut gate = load_gate(program, gate_info, &action.target)?;
    require(
        gate.epoch == action.expected_epoch
            && matches!(
                gate.status,
                GateStatusV1::Active | GateStatusV1::EmergencyFrozen
            ),
        Error::StaleProposal,
    )?;
    gate.epoch = add(gate.epoch, 1)?;
    gate.status = if action.active {
        GateStatusV1::Active
    } else {
        GateStatusV1::EmergencyFrozen
    };
    gate.freeze_slot = if action.active { 0 } else { slot };
    gate.freeze_reason_code = if action.active { 0 } else { 3 };
    gate.active_proposal = Pubkey::default();
    gate.last_completed_proposal = *proposal_info.key;
    save_gate(program, gate_info, &gate)?;
    p.state = EXECUTED;
    p.executed_slot = slot;
    save_proposal(program, proposal_info, &p)
}

/// A typed, create-once council operation. There is no caller-selected CPI tag or account list.
fn execute_spread_light_config(
    program: &Pubkey,
    accounts: &[AccountInfo<'_>],
    digest: [u8; 32],
    data: &[u8],
) -> Result<()> {
    let [payer, config_info, proposal_info, target, programdata, gate_info, authority, light_config, system, ixs] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    privileges(payer, true, true, false)?;
    readonly(config_info)?;
    writable(proposal_info)?;
    executable(target, &DEVNET_SPREAD)?;
    readonly(programdata)?;
    readonly(gate_info)?;
    readonly(authority)?;
    writable(light_config)?;
    executable(system, &system_program::ID)?;
    envelope(program, accounts, ixs, data)?;
    let config = config(program, config_info)?;
    let mut p = proposal(program, proposal_info, Some(digest))?;
    let slot = clock()?;
    p.executable(&config, slot)?;
    let policy = p.action.light_config_policy()?;
    let gate = load_gate(program, gate_info, target.key)?;
    let (authority_key, authority_bump) = target_authority_pda(program, target.key);
    let header = validate_program_programdata_linkage(target, programdata, &LOADER)?;
    require(
        gate.status == GateStatusV1::Active
            && gate.epoch == policy.expected_epoch
            && policy.target == *target.key
            && policy.deployed_slot == header.deployed_slot
            && header.upgrade_authority == Some(authority_key)
            && *authority.key == authority_key,
        Error::StaleProposal,
    )?;
    let (config_key, config_bump) =
        Pubkey::find_program_address(&[b"compressible_config", &[0, 0]], target.key);
    let (rent_sponsor, sponsor_bump) = Pubkey::find_program_address(&[b"rent_sponsor"], target.key);
    require(
        *light_config.key == config_key
            && *light_config.owner == system_program::ID
            && light_config.data_is_empty(),
        Error::InvalidAccount,
    )?;
    let config_lamports = light_config.lamports();
    let payer_lamports = payer.lamports();
    let funding = Rent::get()?
        .minimum_balance(156)
        .saturating_sub(config_lamports);
    require(payer_lamports >= funding, Error::InvalidAccount)?;
    let rent_bytes = policy
        .rent
        .try_to_vec()
        .map_err(|_| Error::InvalidAccount)?;
    let mut inner_data = Vec::with_capacity(130);
    inner_data.push(216);
    inner_data.extend_from_slice(rent_sponsor.as_ref());
    inner_data.extend_from_slice(policy.compression_authority.as_ref());
    inner_data.extend_from_slice(&rent_bytes);
    inner_data.extend_from_slice(&policy.write_top_up.to_le_bytes());
    inner_data.extend_from_slice(&1u32.to_le_bytes());
    inner_data.extend_from_slice(policy.address_tree.as_ref());
    inner_data.push(0);
    inner_data.extend_from_slice(b"AGV1\x01\0\0\0");
    inner_data.extend_from_slice(&policy.expected_epoch.to_le_bytes());
    require(inner_data.len() == 130, Error::InvalidEnvelope)?;
    let inner = SolanaInstruction {
        program_id: *target.key,
        accounts: vec![
            AccountMeta::new(*payer.key, true),
            AccountMeta::new(*light_config.key, false),
            AccountMeta::new_readonly(*programdata.key, false),
            AccountMeta::new_readonly(authority_key, true),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(*gate_info.key, false),
        ],
        data: inner_data,
    };
    invoke_signed(
        &inner,
        &[
            payer.clone(),
            light_config.clone(),
            programdata.clone(),
            authority.clone(),
            system.clone(),
            gate_info.clone(),
            target.clone(),
        ],
        &[&[
            DOMAIN,
            b"target-authority",
            target.key.as_ref(),
            &[authority_bump],
        ]],
    )?;
    let mut expected = Vec::with_capacity(156);
    expected.extend_from_slice(b"LightCfg\x01");
    expected.extend_from_slice(&policy.write_top_up.to_le_bytes());
    expected.extend_from_slice(authority_key.as_ref());
    expected.extend_from_slice(rent_sponsor.as_ref());
    expected.extend_from_slice(policy.compression_authority.as_ref());
    expected.extend_from_slice(&rent_bytes);
    expected.extend_from_slice(&[0, config_bump, sponsor_bump]);
    expected.extend_from_slice(&1u32.to_le_bytes());
    expected.extend_from_slice(policy.address_tree.as_ref());
    require(
        expected.len() == 156
            && *light_config.owner == *target.key
            && !light_config.executable
            && light_config.try_borrow_data()?.as_ref() == expected.as_slice()
            && light_config.lamports() == add(config_lamports, funding)?
            && payer.lamports() == payer_lamports - funding,
        Error::InvalidAccount,
    )?;
    p.state = EXECUTED;
    p.executed_slot = slot;
    save_proposal(program, proposal_info, &p)
}

pub fn process(program: &Pubkey, accounts: &[AccountInfo<'_>], data: &[u8]) -> Result<()> {
    let instruction = Instruction::unpack(data)?;
    require_distinct_accounts(&accounts.iter().collect::<Vec<_>>())?;
    match instruction {
        Instruction::Initialize { seats, treasury } => {
            initialize(program, accounts, seats, treasury)
        }
        Instruction::Create {
            expected_id,
            action,
        } => create(program, accounts, expected_id, action),
        Instruction::SealBuffer { digest } => seal(program, accounts, digest),
        Instruction::VerifyChunk {
            digest,
            index,
            proof_len,
            proof,
        } => verify(program, accounts, digest, index, proof_len, proof),
        Instruction::Approve { digest } => approve(program, accounts, digest, false),
        Instruction::Cancel { digest } => approve(program, accounts, digest, true),
        Instruction::Expire => expire(program, accounts),
        Instruction::ExecutePolicy { digest } => execute_policy(program, accounts, digest),
        Instruction::ExecuteControllerUpgrade { digest } => {
            execute_upgrade(program, accounts, digest, data, false)
        }
        Instruction::CloseBuffer { digest } => close_buffer(program, accounts, digest),
        Instruction::RegisterTarget => register_target(program, accounts, data),
        Instruction::ExecuteTargetUpgrade { digest } => {
            execute_upgrade(program, accounts, digest, data, true)
        }
        Instruction::ExecuteTargetGate { digest } => execute_target_gate(program, accounts, digest),
        Instruction::ExecuteSpreadLightConfig { digest } => {
            execute_spread_light_config(program, accounts, digest, data)
        }
        Instruction::PublishVerification { digest } => {
            verification::publish(program, accounts, digest, data)
        }
        Instruction::RetireTarget { digest } => retirement::retire(program, accounts, digest, data),
        Instruction::ReclaimRetiredTargetRecords { target } => {
            retirement::reclaim_records(program, accounts, target)
        }
        Instruction::ReclaimRetiredTargetVerification { target } => {
            retirement::reclaim_verification(program, accounts, target)
        }
        // Retired tags: `Instruction::unpack` rejects them before this match.
        Instruction::ExtendTarget { .. } | Instruction::ExtendController { .. } => {
            Err(ProgramError::InvalidInstructionData)
        }
    }
}

/// Initial deployment authority and any three chosen seats consent atomically.
/// The bootstrap transaction leaves ProgramData controlled by this council's PDA.
fn initialize(
    program: &Pubkey,
    accounts: &[AccountInfo<'_>],
    seats: [Pubkey; 5],
    treasury: Pubkey,
) -> Result<()> {
    let [payer, deployer, controller, programdata, config_info, authority, loader, system, a, b, c] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    privileges(payer, true, true, false)?;
    signer(deployer)?;
    executable(controller, program)?;
    writable(programdata)?;
    writable(config_info)?;
    readonly(authority)?;
    executable(loader, &LOADER)?;
    executable(system, &system_program::ID)?;
    let (config_key, bump) = config_pda(program);
    let (authority_key, authority_bump) = authority_pda(program);
    require(
        *config_info.key == config_key
            && *authority.key == authority_key
            && *programdata.key == Pubkey::find_program_address(&[program.as_ref()], &LOADER).0,
        Error::InvalidAccount,
    )?;
    let header = validate_program_programdata_linkage(controller, programdata, &LOADER)?;
    require(
        header.upgrade_authority == Some(*deployer.key),
        Error::Unauthorized,
    )?;
    let value = Config {
        discriminator: *b"AG3CFG01",
        version: 1,
        bump,
        initialized: true,
        controller: *program,
        programdata: *programdata.key,
        authority: authority_key,
        treasury,
        seats,
        council_epoch: 1,
        timing_version: 1,
        timing: Timing::default(),
        next_id: 1,
        reserved: [0; 37],
    };
    value.validate(program, config_info.key)?;
    require(
        treasury != *config_info.key && treasury != *programdata.key && treasury != *controller.key,
        Error::InvalidAccount,
    )?;
    for signer_info in [a, b, c] {
        seat(&value, signer_info)?;
    }
    create_fixed_pda_account(
        program,
        payer,
        config_info,
        system,
        &Rent::get()?,
        CONFIG_LEN,
        &[DOMAIN, b"council", &[bump]],
    )?;
    store_fixed_controller_account(program, config_info, &value, CONFIG_LEN)?;
    invoke_signed(
        &set_upgrade_authority_checked(program, deployer.key, authority.key),
        &[
            programdata.clone(),
            deployer.clone(),
            authority.clone(),
            loader.clone(),
        ],
        &[&[DOMAIN, b"authority", &[authority_bump]]],
    )?;
    let after = validate_program_programdata_linkage(controller, programdata, &LOADER)?;
    require(
        after.upgrade_authority == Some(authority_key)
            && after.capacity == header.capacity
            && after.deployed_slot == header.deployed_slot,
        Error::InvalidAccount,
    )
}

fn create(
    program: &Pubkey,
    accounts: &[AccountInfo<'_>],
    expected_id: u64,
    action: Action,
) -> Result<()> {
    let [payer, creator, config_info, proposal_info, system] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    privileges(payer, true, true, false)?;
    writable(config_info)?;
    writable(proposal_info)?;
    executable(system, &system_program::ID)?;
    let mut config = config(program, config_info)?;
    seat(&config, creator)?;
    action.validate()?;
    // The ProgramData slot/capacity in an upgrade action are an informational
    // snapshot. Anyone may grow ProgramData with the permissionless top-level
    // Loader-v3 ExtendProgram at any time, so neither value gates execution:
    // ordering comes from the target gate epoch or the controller release, and
    // installation needs live capacity >= artifact length.
    if action.kind == UPGRADE_CONTROLLER {
        require(
            action.upgrade()?.controller_release == CONTROLLER_RELEASE,
            Error::StaleProposal,
        )?;
    }
    if action.kind == ROTATE_COUNCIL {
        require(
            !action.seats()?.contains(&config.authority),
            Error::InvalidCouncil,
        )?;
    }
    require(config.next_id == expected_id, Error::StaleProposal)?;
    let (key, bump) = proposal_pda(program, expected_id);
    require(*proposal_info.key == key, Error::InvalidAccount)?;
    let next = add(expected_id, 1)?;
    let created = clock()?;
    let (review_end, not_before, expires) = config.timing.boundaries(created)?;
    let mut value = Proposal {
        discriminator: *b"AG3PRP01",
        version: 1,
        bump,
        initialized: true,
        config: *config_info.key,
        id: expected_id,
        council_epoch: config.council_epoch,
        timing_version: config.timing_version,
        timing: config.timing,
        created,
        review_end,
        not_before,
        expires,
        action,
        digest: [0; 32],
        approvals: 0,
        approval_count: 0,
        cancellations: 0,
        cancellation_count: 0,
        state: PENDING,
        verified_chunks: [0; 12],
        verified_count: 0,
        extension_slot: 0,
        extended_capacity: 0,
        executed_slot: 0,
        verified_chunks_high: [0; 4],
        reserved: [0; 3],
    };
    value.digest = value.compute_digest(program)?;
    value.validate(program, proposal_info.key)?;
    create_fixed_pda_account(
        program,
        payer,
        proposal_info,
        system,
        &Rent::get()?,
        PROPOSAL_LEN,
        &[DOMAIN, b"proposal", &expected_id.to_le_bytes(), &[bump]],
    )?;
    save_proposal(program, proposal_info, &value)?;
    config.next_id = next;
    store_fixed_controller_account(program, config_info, &*config, CONFIG_LEN)
}

fn seal(program: &Pubkey, accounts: &[AccountInfo<'_>], digest: [u8; 32]) -> Result<()> {
    let [config_info, proposal_info, buffer, uploader, buffer_authority, loader] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    readonly(config_info)?;
    readonly(proposal_info)?;
    writable(buffer)?;
    signer(uploader)?;
    readonly(buffer_authority)?;
    executable(loader, &LOADER)?;
    let config = config(program, config_info)?;
    let p = proposal(program, proposal_info, Some(digest))?;
    p.current(&config, clock()?)?;
    let action = p.action.artifact()?;
    let (authority, bump) = buffer_authority_pda(program, proposal_info.key);
    let header = validate_buffer_account(buffer, &LOADER)?;
    require(
        *buffer_authority.key == authority
            && *buffer.key == action.buffer
            && header.authority == Some(*uploader.key)
            && header.payload_length as u64 == action.artifact_length
            && p.approvals == 0
            && p.verified_count == 0,
        Error::InvalidArtifact,
    )?;
    invoke_signed(
        &set_buffer_authority_checked(buffer.key, uploader.key, buffer_authority.key),
        &[
            buffer.clone(),
            uploader.clone(),
            buffer_authority.clone(),
            loader.clone(),
        ],
        &[&[DOMAIN, b"buffer", proposal_info.key.as_ref(), &[bump]]],
    )?;
    sealed(program, proposal_info.key, buffer, &action)
}

fn verify(
    program: &Pubkey,
    accounts: &[AccountInfo<'_>],
    digest: [u8; 32],
    index: u32,
    proof_len: u8,
    proof: [[u8; 32]; 7],
) -> Result<()> {
    let [config_info, proposal_info, buffer] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    readonly(config_info)?;
    writable(proposal_info)?;
    readonly(buffer)?;
    let config = config(program, config_info)?;
    let mut p = proposal(program, proposal_info, Some(digest))?;
    p.current(&config, clock()?)?;
    let action = p.action.artifact()?;
    sealed(program, proposal_info.key, buffer, &action)?;
    let count = artifact::chunk_count(action.artifact_length)?;
    require(
        index < count
            && proof_len <= 7
            && proof[usize::from(proof_len)..]
                .iter()
                .all(|v| *v == [0; 32]),
        Error::InvalidArtifact,
    )?;
    // Duplicate chunks fail before hashing; nothing persists unless the proof verifies.
    p.mark_chunk_verified(index)?;
    let start = 37 + index as usize * CHUNK as usize;
    let end = (start + CHUNK as usize).min(37 + action.artifact_length as usize);
    {
        let bytes = buffer.try_borrow_data()?;
        artifact::verify_chunk(
            &action.merkle_root,
            action.artifact_length,
            index,
            &bytes[start..end],
            &proof[..usize::from(proof_len)],
        )?;
    }
    save_proposal(program, proposal_info, &p)
}

fn approve(
    program: &Pubkey,
    accounts: &[AccountInfo<'_>],
    digest: [u8; 32],
    cancel: bool,
) -> Result<()> {
    let [config_info, proposal_info, seat_info] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    readonly(config_info)?;
    writable(proposal_info)?;
    let config = config(program, config_info)?;
    let mut p = proposal(program, proposal_info, Some(digest))?;
    let slot = clock()?;
    p.current(&config, slot)?;
    let index = seat(&config, seat_info)?;
    if !cancel && p.action.is_upgrade() {
        complete(&p)?;
    }
    p.approve(index, slot, cancel)?;
    save_proposal(program, proposal_info, &p)
}

fn expire(program: &Pubkey, accounts: &[AccountInfo<'_>]) -> Result<()> {
    let [proposal_info] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    writable(proposal_info)?;
    let mut p = proposal(program, proposal_info, None)?;
    require(
        p.state == PENDING && clock()? >= p.expires,
        Error::WrongState,
    )?;
    p.state = EXPIRED;
    save_proposal(program, proposal_info, &p)
}

fn execute_policy(program: &Pubkey, accounts: &[AccountInfo<'_>], digest: [u8; 32]) -> Result<()> {
    let [config_info, proposal_info] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    writable(config_info)?;
    writable(proposal_info)?;
    let mut config = config(program, config_info)?;
    let mut p = proposal(program, proposal_info, Some(digest))?;
    let slot = clock()?;
    p.executable(&config, slot)?;
    match p.action.kind {
        SET_TIMING => {
            // Concurrent proposals cannot silently overwrite a policy selected later.
            require(
                p.timing_version == config.timing_version,
                Error::StaleProposal,
            )?;
            config.timing = p.action.timing()?;
            config.timing_version = add(config.timing_version, 1)?;
        }
        ROTATE_COUNCIL => {
            config.seats = p.action.seats()?;
            config.council_epoch = add(config.council_epoch, 1)?;
        }
        _ => return Err(ProgramError::InvalidInstructionData),
    }
    config.validate(program, config_info.key)?;
    p.state = EXECUTED;
    p.executed_slot = slot;
    save_proposal(program, proposal_info, &p)?;
    store_fixed_controller_account(program, config_info, &*config, CONFIG_LEN)
}

/// Only bounded ComputeBudget prefixes and this exact top-level instruction.
fn envelope(
    program: &Pubkey,
    accounts: &[AccountInfo<'_>],
    info: &AccountInfo<'_>,
    data: &[u8],
) -> Result<()> {
    readonly(info)?;
    require(*info.key == instructions::ID, Error::InvalidEnvelope)?;
    let index = usize::from(instructions::load_current_index_checked(info)?);
    let count = {
        let bytes = info.try_borrow_data()?;
        let first: [u8; 2] = bytes
            .get(..2)
            .ok_or(Error::InvalidEnvelope)?
            .try_into()
            .map_err(|_| Error::InvalidEnvelope)?;
        usize::from(u16::from_le_bytes(first))
    };
    require(count == index + 1 && index <= 2, Error::InvalidEnvelope)?;
    let current = instructions::load_instruction_at_checked(index, info)?;
    require(
        current.program_id == *program
            && current.data == data
            && current.accounts.len() == accounts.len(),
        Error::InvalidEnvelope,
    )?;
    for (meta, account) in current.accounts.iter().zip(accounts) {
        require(
            meta.pubkey == *account.key
                && meta.is_signer == account.is_signer
                && meta.is_writable == account.is_writable,
            Error::InvalidEnvelope,
        )?;
    }
    let mut seen = 0u8;
    for n in 0..index {
        let ix = instructions::load_instruction_at_checked(n, info)?;
        require(
            ix.program_id == compute_budget::ID && ix.accounts.is_empty(),
            Error::InvalidEnvelope,
        )?;
        let bit = match ix.data.as_slice() {
            [2, a, b, c, d] if u32::from_le_bytes([*a, *b, *c, *d]) <= 1_400_000 => 1,
            [3, a, b, c, d, e, f, g, h]
                if u64::from_le_bytes([*a, *b, *c, *d, *e, *f, *g, *h]) <= 100_000 =>
            {
                2
            }
            _ => return Err(Error::InvalidEnvelope.into()),
        };
        require(seen & bit == 0, Error::InvalidEnvelope)?;
        seen |= bit;
    }
    Ok(())
}

fn execute_upgrade(
    program: &Pubkey,
    accounts: &[AccountInfo<'_>],
    digest: [u8; 32],
    data: &[u8],
    target_mode: bool,
) -> Result<()> {
    let all_accounts = accounts;
    let (accounts, gate_info) = split_target_accounts(accounts, target_mode)?;
    let [config_info, proposal_info, controller, programdata, buffer, treasury, authority, buffer_authority, loader, rent, clock_info, ixs] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    readonly(config_info)?;
    writable(proposal_info)?;
    privileges(controller, true, false, true)?;
    for info in [programdata, buffer, treasury] {
        writable(info)?;
    }
    for info in [authority, buffer_authority, rent, clock_info] {
        readonly(info)?;
    }
    executable(loader, &LOADER)?;
    envelope(program, all_accounts, ixs, data)?;
    let config = config(program, config_info)?;
    let mut p = proposal(program, proposal_info, Some(digest))?;
    let slot = clock()?;
    p.executable(&config, slot)?;
    complete(&p)?;
    let action = p.action.artifact()?;
    let (authority_key, authority_bump) =
        upgrade_scope(program, &p, controller, programdata, gate_info)?;
    // ProgramData length and slot are deliberately not compared with the
    // snapshot: third-party top-level growth must not stale a proposal.
    let before = validate_program_programdata_linkage(controller, programdata, &LOADER)?;
    let capacity = before.capacity as u64;
    require(
        before.upgrade_authority == Some(authority_key) && capacity >= action.artifact_length,
        Error::StaleProposal,
    )?;
    let (buffer_key, buffer_bump) = buffer_authority_pda(program, proposal_info.key);
    require(
        *treasury.key == config.treasury
            && *authority.key == authority_key
            && *buffer_authority.key == buffer_key
            && *rent.key == sysvar::rent::ID
            && *clock_info.key == sysvar::clock::ID,
        Error::InvalidAccount,
    )?;
    sealed(program, proposal_info.key, buffer, &action)?;
    let authority_bump_bytes = [authority_bump];
    let authority_seeds =
        upgrade_authority_seeds(controller.key, &authority_bump_bytes, target_mode);
    let buffer_seeds: &[&[u8]] = &[
        DOMAIN,
        b"buffer",
        proposal_info.key.as_ref(),
        &[buffer_bump],
    ];
    // Both CPIs are fixed Loader-v3 instructions. The buffer-authority change
    // and code installation roll back together if either fails. Loader-v3
    // Upgrade copies the sealed, fully verified buffer and zero-fills every
    // ProgramData byte beyond it, so whatever length third parties added, the
    // installed payload is exactly the artifact followed by zeros.
    invoke_signed(
        &set_buffer_authority_checked(buffer.key, buffer_authority.key, authority.key),
        &[
            buffer.clone(),
            buffer_authority.clone(),
            authority.clone(),
            loader.clone(),
        ],
        &[buffer_seeds, &authority_seeds],
    )?;
    invoke_signed(
        &upgrade(controller.key, buffer.key, authority.key, treasury.key),
        &[
            programdata.clone(),
            controller.clone(),
            buffer.clone(),
            treasury.clone(),
            rent.clone(),
            clock_info.clone(),
            authority.clone(),
            loader.clone(),
        ],
        &[&authority_seeds],
    )?;
    let after = validate_program_programdata_linkage(controller, programdata, &LOADER)?;
    require(
        after.upgrade_authority == Some(authority_key)
            && after.deployed_slot == slot
            && after.capacity as u64 == capacity,
        Error::InvalidArtifact,
    )?;
    p.state = EXECUTED;
    p.executed_slot = slot;
    if let Some(info) = gate_info {
        let mut gate = load_gate(program, info, controller.key)?;
        gate.epoch = add(gate.epoch, 1)?;
        gate.freeze_slot = slot;
        gate.freeze_reason_code = 2;
        gate.last_completed_proposal = *proposal_info.key;
        save_gate(program, info, &gate)?;
    }
    save_proposal(program, proposal_info, &p)
}

fn close_buffer(program: &Pubkey, accounts: &[AccountInfo<'_>], digest: [u8; 32]) -> Result<()> {
    let [config_info, proposal_info, buffer, treasury, buffer_authority, loader] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    readonly(config_info)?;
    readonly(proposal_info)?;
    writable(buffer)?;
    writable(treasury)?;
    readonly(buffer_authority)?;
    executable(loader, &LOADER)?;
    let config = config(program, config_info)?;
    let p = proposal(program, proposal_info, Some(digest))?;
    require(matches!(p.state, CANCELLED | EXPIRED), Error::WrongState)?;
    let action = p.action.artifact()?;
    sealed(program, proposal_info.key, buffer, &action)?;
    let (key, bump) = buffer_authority_pda(program, proposal_info.key);
    require(
        *buffer_authority.key == key && *treasury.key == config.treasury,
        Error::InvalidAccount,
    )?;
    invoke_signed(
        &close(buffer.key, treasury.key, buffer_authority.key),
        &[
            buffer.clone(),
            treasury.clone(),
            buffer_authority.clone(),
            loader.clone(),
        ],
        &[&[DOMAIN, b"buffer", proposal_info.key.as_ref(), &[bump]]],
    )?;
    require(buffer.lamports() == 0, Error::InvalidArtifact)
}
