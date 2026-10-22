//! Typed permanent target closure and reclamation of its obsolete records.
use super::*;
use solana_loader_v3_interface::instruction::close_any;
use solana_program::hash::hash;

pub(super) fn retire(
    program: &Pubkey,
    accounts: &[AccountInfo<'_>],
    digest: [u8; 32],
    data: &[u8],
) -> Result<()> {
    let [config_info, proposal_info, target, programdata, gate_info, treasury, authority, loader, ixs] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    readonly(config_info)?;
    for info in [proposal_info, programdata, gate_info, treasury] {
        writable(info)?;
    }
    privileges(target, true, false, true)?;
    readonly(authority)?;
    executable(loader, &LOADER)?;
    envelope(program, accounts, ixs, data)?;
    let cfg = config(program, config_info)?;
    let mut p = proposal(program, proposal_info, Some(digest))?;
    let slot = clock()?;
    p.executable(&cfg, slot)?;
    let policy = p.action.retirement_policy()?;
    let mut gate = load_gate(program, gate_info, target.key)?;
    let (authority_key, bump) = target_authority_pda(program, target.key);
    let header = validate_program_programdata_linkage(target, programdata, &LOADER)?;
    require(
        policy.target == *target.key
            && *treasury.key == cfg.treasury
            && *authority.key == authority_key
            && *programdata.key == gate.target_programdata
            && header.upgrade_authority == Some(authority_key)
            && header.deployed_slot == policy.deployed_slot
            && gate.epoch == policy.expected_epoch
            && gate.status == GateStatusV1::EmergencyFrozen,
        Error::StaleProposal,
    )?;
    // Bind the entire executable allocation, including Loader zero padding.
    require(
        hash(&programdata.try_borrow_data()?[45..]).to_bytes() == policy.payload_sha256,
        Error::InvalidArtifact,
    )?;
    let amount = programdata.lamports();
    let expected_treasury = add(treasury.lamports(), amount)?;
    invoke_signed(
        &close_any(
            programdata.key,
            treasury.key,
            Some(authority.key),
            Some(target.key),
        ),
        &[
            programdata.clone(),
            treasury.clone(),
            authority.clone(),
            target.clone(),
            loader.clone(),
        ],
        &[&[DOMAIN, b"target-authority", target.key.as_ref(), &[bump]]],
    )?;
    require(
        programdata.lamports() == 0 && treasury.lamports() == expected_treasury,
        Error::InvalidAccount,
    )?;
    gate.epoch = add(gate.epoch, 1)?;
    gate.freeze_slot = slot;
    gate.freeze_reason_code = 4;
    gate.last_completed_proposal = *proposal_info.key;
    save_gate(program, gate_info, &gate)?;
    p.state = EXECUTED;
    p.executed_slot = slot;
    save_proposal(program, proposal_info, &p)
}

/// No fresh governance decision: Loader has permanently removed this target's
/// executable. Only its canonical gate and target-specific proposal records
/// can be reclaimed, and every lamport goes to the configured treasury.
pub(super) fn reclaim_records(
    program: &Pubkey,
    accounts: &[AccountInfo<'_>],
    target_key: Pubkey,
) -> Result<()> {
    let [config_info, target, programdata, treasury, records @ ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    retired_target(
        program,
        config_info,
        target,
        programdata,
        treasury,
        target_key,
    )?;
    require(!records.is_empty(), Error::InvalidAccount)?;
    let mut total = treasury.lamports();
    for info in records {
        writable(info)?;
        if *info.key == gate_pda(program, target.key).0 {
            let gate = load_gate(program, info, target.key)?;
            require(
                gate.status == GateStatusV1::EmergencyFrozen,
                Error::WrongState,
            )?;
        } else {
            let p = proposal(program, info, None)?;
            require(
                p.action.explicit_target()? == *target.key,
                Error::InvalidAccount,
            )?;
        }
        total = add(total, info.lamports())?;
    }
    for info in records {
        **info.try_borrow_mut_lamports()? = 0;
        info.resize(0)?;
        info.assign(&system_program::ID);
    }
    **treasury.try_borrow_mut_lamports()? = total;
    Ok(())
}

fn retired_target(
    program: &Pubkey,
    config_info: &AccountInfo<'_>,
    target: &AccountInfo<'_>,
    programdata: &AccountInfo<'_>,
    treasury: &AccountInfo<'_>,
    target_key: Pubkey,
) -> Result<()> {
    readonly(config_info)?;
    privileges(target, false, false, true)?;
    readonly(programdata)?;
    writable(treasury)?;
    let cfg = config(program, config_info)?;
    require(
        target_key != *program
            && target_key == *target.key
            && *treasury.key == cfg.treasury
            && *target.owner == LOADER
            && *programdata.key == Pubkey::find_program_address(&[target.key.as_ref()], &LOADER).0
            && programdata.lamports() == 0
            && (programdata.data_is_empty()
                || (*programdata.owner == LOADER
                    && programdata.try_borrow_data()?.as_ref() == [0u8; 4])),
        Error::InvalidAccount,
    )?;
    let bytes = target.try_borrow_data()?;
    require(
        bytes.len() == 36
            && bytes[..4] == 2u32.to_le_bytes()
            && bytes[4..] == programdata.key.to_bytes(),
        Error::InvalidAccount,
    )
}

/// Otter returns closed-record rent to its signer PDA. Forward that refund and
/// any remaining target-authority balance to the same configured treasury.
pub(super) fn reclaim_verification(
    program: &Pubkey,
    accounts: &[AccountInfo<'_>],
    target_key: Pubkey,
) -> Result<()> {
    use crate::verification::{metadata_pda, VERIFY_PROGRAM};
    let [config_info, target, programdata, treasury, authority, metadata, verifier, system] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    retired_target(
        program,
        config_info,
        target,
        programdata,
        treasury,
        target_key,
    )?;
    writable(authority)?;
    writable(metadata)?;
    executable(verifier, &VERIFY_PROGRAM)?;
    executable(system, &system_program::ID)?;
    let (authority_key, bump) = target_authority_pda(program, target.key);
    require(
        *authority.key == authority_key
            && *authority.owner == system_program::ID
            && authority.data_is_empty()
            && *metadata.key == metadata_pda(&authority_key, target.key).0
            && *metadata.owner == VERIFY_PROGRAM,
        Error::InvalidAccount,
    )?;
    let amount = add(authority.lamports(), metadata.lamports())?;
    let expected_treasury = add(treasury.lamports(), amount)?;
    let seeds: &[&[u8]] = &[DOMAIN, b"target-authority", target.key.as_ref(), &[bump]];
    invoke_signed(
        &SolanaInstruction {
            program_id: VERIFY_PROGRAM,
            accounts: vec![
                AccountMeta::new(*metadata.key, false),
                AccountMeta::new(authority_key, true),
                AccountMeta::new_readonly(*target.key, false),
            ],
            data: hash(b"global:close").to_bytes()[..8].to_vec(),
        },
        &[
            metadata.clone(),
            authority.clone(),
            target.clone(),
            verifier.clone(),
        ],
        &[seeds],
    )?;
    require(
        metadata.lamports() == 0 && authority.lamports() == amount,
        Error::InvalidAccount,
    )?;
    invoke_signed(
        &solana_system_interface::instruction::transfer(authority.key, treasury.key, amount),
        &[authority.clone(), treasury.clone(), system.clone()],
        &[seeds],
    )?;
    require(
        authority.lamports() == 0 && treasury.lamports() == expected_treasury,
        Error::InvalidAccount,
    )
}
