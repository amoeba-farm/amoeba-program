use super::*;

pub(super) fn validate_target_account_keys(
    config: &ControllerConfigV1,
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    authority_info: &AccountInfo<'_>,
) -> ProgramResult {
    if *target_program.key != config.target_program
        || *target_programdata.key != config.target_programdata
        || *authority_info.key != config.authority_pda
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn validate_program_and_sysvar_ids(
    loader: &AccountInfo<'_>,
    system: &AccountInfo<'_>,
    rent: Option<&AccountInfo<'_>>,
    clock: Option<&AccountInfo<'_>>,
    instructions_info: &AccountInfo<'_>,
) -> ProgramResult {
    if *loader.key != UPGRADEABLE_LOADER_ID
        || *system.key != system_program::ID
        || rent.is_some_and(|account| *account.key != sysvar_ids::rent::ID)
        || clock.is_some_and(|account| *account.key != sysvar_ids::clock::ID)
        || *instructions_info.key != sysvar_ids::instructions::ID
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn validate_canonical_envelope(
    program_id: &Pubkey,
    current_accounts: &[AccountInfo<'_>],
    instructions_info: &AccountInfo<'_>,
    expected_current_data: &[u8],
    envelope: &EnvelopeExpectationV1,
) -> ProgramResult {
    envelope.validate()?;
    if envelope.compute_unit_limit == 0
        || envelope.compute_unit_limit > MAX_ENVELOPE_COMPUTE_UNIT_LIMIT_V1
        || envelope.compute_unit_price_micro_lamports
            > MAX_ENVELOPE_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1
    {
        return Err(ProgramError::InvalidInstructionData);
    }
    let nonce = envelope.durable_nonce_account.value();
    let nonce_authority = envelope.durable_nonce_authority.value();
    if nonce.is_some() != nonce_authority.is_some() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let current_index = if nonce.is_some() { 3usize } else { 2usize };
    if usize::from(instructions::load_current_index_checked(instructions_info)?) != current_index {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let mut index = 0usize;
    if let (Some(nonce), Some(authority)) = (nonce, nonce_authority) {
        let actual = instructions::load_instruction_at_checked(index, instructions_info)?;
        if actual != system_instruction::advance_nonce_account(&nonce, &authority) {
            return Err(GovernanceError::CrossAccountMismatch.into());
        }
        index += 1;
    }
    let limit = instructions::load_instruction_at_checked(index, instructions_info)?;
    let mut limit_data = [0u8; 5];
    limit_data[0] = 2;
    limit_data[1..].copy_from_slice(&envelope.compute_unit_limit.to_le_bytes());
    if limit.program_id != compute_budget::ID
        || !limit.accounts.is_empty()
        || limit.data != limit_data
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    index += 1;
    let price = instructions::load_instruction_at_checked(index, instructions_info)?;
    let mut price_data = [0u8; 9];
    price_data[0] = 3;
    price_data[1..].copy_from_slice(&envelope.compute_unit_price_micro_lamports.to_le_bytes());
    if price.program_id != compute_budget::ID
        || !price.accounts.is_empty()
        || price.data != price_data
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    index += 1;
    let current = instructions::load_instruction_at_checked(index, instructions_info)?;
    if current.program_id != *program_id
        || current.data != expected_current_data
        || current.accounts.len() != current_accounts.len()
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    for (meta, info) in current.accounts.iter().zip(current_accounts) {
        if meta.pubkey != *info.key
            || meta.is_signer != info.is_signer
            || meta.is_writable != info.is_writable
        {
            return Err(GovernanceError::InvalidAccountPrivileges.into());
        }
    }
    if instructions::load_instruction_at_checked(index + 1, instructions_info).is_ok() {
        return Err(GovernanceError::InvalidAccountCount.into());
    }
    Ok(())
}

pub(super) fn validate_extend_cpi_shape(
    instruction: &Instruction,
    programdata: &Pubkey,
    program: &Pubkey,
    authority: &Pubkey,
    system: &Pubkey,
    payer: &Pubkey,
) -> ProgramResult {
    if instruction.program_id != UPGRADEABLE_LOADER_ID
        || instruction.accounts.len() != 5
        || instruction.accounts[0].pubkey != *programdata
        || !instruction.accounts[0].is_writable
        || instruction.accounts[0].is_signer
        || instruction.accounts[1].pubkey != *program
        || !instruction.accounts[1].is_writable
        || instruction.accounts[1].is_signer
        || instruction.accounts[2].pubkey != *authority
        || !instruction.accounts[2].is_writable
        || !instruction.accounts[2].is_signer
        || instruction.accounts[3].pubkey != *system
        || instruction.accounts[3].is_writable
        || instruction.accounts[3].is_signer
        || instruction.accounts[4].pubkey != *payer
        || !instruction.accounts[4].is_writable
        || !instruction.accounts[4].is_signer
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_upgrade_cpi_shape(
    instruction: &Instruction,
    programdata: &Pubkey,
    program: &Pubkey,
    buffer: &Pubkey,
    spill: &Pubkey,
    rent: &Pubkey,
    clock: &Pubkey,
    authority: &Pubkey,
) -> ProgramResult {
    let expected = [
        (*programdata, false, true),
        (*program, false, true),
        (*buffer, false, true),
        (*spill, false, true),
        (*rent, false, false),
        (*clock, false, false),
        (*authority, true, false),
    ];
    if instruction.program_id != UPGRADEABLE_LOADER_ID
        || instruction.accounts.len() != expected.len()
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    for (meta, (key, signer, writable)) in instruction.accounts.iter().zip(expected) {
        if meta.pubkey != key || meta.is_signer != signer || meta.is_writable != writable {
            return Err(GovernanceError::CrossAccountMismatch.into());
        }
    }
    Ok(())
}
