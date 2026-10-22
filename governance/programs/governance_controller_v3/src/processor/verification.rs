use super::*;
use crate::verification::{
    matches_existing_record, metadata_pda, record, INITIALIZE, UPDATE, VERIFY_PROGRAM,
};
use solana_program::program::invoke;

/// Timelocked, quorum-approved metadata only; never alters code, authorities or gates.
pub(super) fn publish(
    program: &Pubkey,
    accounts: &[AccountInfo<'_>],
    digest: [u8; 32],
    data: &[u8],
) -> Result<()> {
    let (base, gate) = match accounts.len() {
        10 => (accounts, None),
        11 => (&accounts[..10], Some(&accounts[10])),
        _ => return Err(ProgramError::NotEnoughAccountKeys),
    };
    let [payer, config_info, proposal_info, target, programdata, authority, metadata, verifier, system, ixs] =
        base
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    privileges(payer, true, true, false)?;
    readonly(config_info)?;
    writable(proposal_info)?;
    privileges(target, false, false, true)?;
    readonly(programdata)?;
    writable(authority)?;
    writable(metadata)?;
    executable(verifier, &VERIFY_PROGRAM)?;
    executable(system, &system_program::ID)?;
    envelope(program, accounts, ixs, data)?;
    let cfg = config(program, config_info)?;
    let mut p = proposal(program, proposal_info, Some(digest))?;
    let slot = clock()?;
    p.executable(&cfg, slot)?;
    let policy = p.action.verification_policy()?;
    require(policy.target == *target.key, Error::InvalidAccount)?;
    let own = *target.key == *program;
    require(own == gate.is_none(), Error::InvalidAccount)?;
    if let Some(info) = gate {
        readonly(info)?;
        load_gate(program, info, target.key)?;
    }
    let (authority_key, bump) = if own {
        authority_pda(program)
    } else {
        target_authority_pda(program, target.key)
    };
    let header = validate_program_programdata_linkage(target, programdata, &LOADER)?;
    require(
        *programdata.key == Pubkey::find_program_address(&[target.key.as_ref()], &LOADER).0
            && *authority.key == authority_key
            && header.upgrade_authority == Some(authority_key)
            && header.deployed_slot == policy.deployed_slot,
        Error::StaleProposal,
    )?;
    require(
        *authority.owner == system_program::ID && authority.data_is_empty(),
        Error::InvalidAccount,
    )?;
    let (metadata_key, metadata_bump) = metadata_pda(authority.key, target.key);
    require(*metadata.key == metadata_key, Error::InvalidAccount)?;
    let params = policy.params(program)?;
    let expected = record(target.key, authority.key, &params, metadata_bump);
    let fresh = *metadata.owner == system_program::ID && metadata.data_is_empty();
    let previous_size = metadata.data_len();
    if !fresh {
        let bytes = metadata.try_borrow_data()?;
        let current_recipe = matches_existing_record(&bytes, &expected, own);
        let previous_recipe = !own
            && !current_recipe
            && matches_existing_record(
                &bytes,
                &record(
                    target.key,
                    authority.key,
                    &if policy.target == crate::verification::MAINNET_FUTURES {
                        policy.previous_futures_params(program)?
                    } else {
                        policy.previous_spread_params(program)?
                    },
                    metadata_bump,
                ),
                false,
            );
        require(
            *metadata.owner == VERIFY_PROGRAM
                && !metadata.executable
                && (current_recipe || previous_recipe),
            Error::InvalidAccount,
        )?;
    }
    let original_metadata_lamports = metadata.lamports();
    let original_authority_lamports = authority.lamports();
    let original_payer_lamports = payer.lamports();
    let funding = Rent::get()?
        .minimum_balance(expected.len())
        .saturating_sub(original_metadata_lamports);
    require(
        funding <= 10_000_000 && original_payer_lamports >= funding,
        Error::InvalidAccount,
    )?;
    if funding > 0 {
        invoke(
            &solana_system_interface::instruction::transfer(payer.key, authority.key, funding),
            &[payer.clone(), authority.clone(), system.clone()],
        )?;
    }
    let mut inner_data = if fresh { INITIALIZE } else { UPDATE }.to_vec();
    inner_data.extend_from_slice(&params);
    let inner = SolanaInstruction {
        program_id: VERIFY_PROGRAM,
        accounts: vec![
            AccountMeta::new(metadata_key, false),
            AccountMeta::new(authority_key, true),
            AccountMeta::new_readonly(*target.key, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
        data: inner_data,
    };
    let bump_bytes = [bump];
    let seeds = upgrade_authority_seeds(target.key, &bump_bytes, !own);
    invoke_signed(
        &inner,
        &[
            metadata.clone(),
            authority.clone(),
            target.clone(),
            system.clone(),
            verifier.clone(),
        ],
        &[&seeds],
    )?;
    // Otter's Anchor reallocation returns the size reduction and all excess
    // lamports to its payer. Return that exact amount to the same metadata PDA
    // so rent and donations remain there and the authority balance is unchanged.
    if previous_size > expected.len() {
        let returned = original_metadata_lamports
            .checked_sub(Rent::get()?.minimum_balance(expected.len()))
            .ok_or(Error::InvalidAccount)?;
        require(
            metadata.lamports() == original_metadata_lamports - returned
                && authority.lamports() == add(original_authority_lamports, returned)?
                && funding == 0,
            Error::InvalidAccount,
        )?;
        if returned > 0 {
            invoke_signed(
                &solana_system_interface::instruction::transfer(
                    authority.key,
                    metadata.key,
                    returned,
                ),
                &[authority.clone(), metadata.clone(), system.clone()],
                &[&seeds],
            )?;
        }
    }
    require(
        *metadata.owner == VERIFY_PROGRAM
            && !metadata.executable
            && metadata.try_borrow_data()?.as_ref() == expected.as_slice()
            && metadata.lamports() == add(original_metadata_lamports, funding)?
            && authority.lamports() == original_authority_lamports
            && payer.lamports() == original_payer_lamports - funding,
        Error::InvalidAccount,
    )?;
    p.state = EXECUTED;
    p.executed_slot = slot;
    save_proposal(program, proposal_info, &p)
}
