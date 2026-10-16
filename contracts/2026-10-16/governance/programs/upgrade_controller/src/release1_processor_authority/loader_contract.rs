use super::*;

pub(super) fn copy_program_header(program: &AccountInfo<'_>) -> Result<[u8; 36], ProgramError> {
    let data = program.try_borrow_data()?;
    data.get(..36)
        .ok_or(GovernanceError::InvalidRelease1Account)?
        .try_into()
        .map_err(|_| GovernanceError::InvalidRelease1Account.into())
}

pub(super) fn copy_programdata_header(
    programdata: &AccountInfo<'_>,
) -> Result<[u8; LOADER_PROGRAMDATA_METADATA_LEN], ProgramError> {
    let data = programdata.try_borrow_data()?;
    data.get(..LOADER_PROGRAMDATA_METADATA_LEN)
        .ok_or(GovernanceError::InvalidRelease1Account)?
        .try_into()
        .map_err(|_| GovernanceError::InvalidRelease1Account.into())
}

pub(super) fn optional_matches(value: &OptionalPubkeyV1, expected: Option<Pubkey>) -> bool {
    match expected {
        Some(key) => value.present && value.value == key,
        None => !value.present && value.value == Pubkey::default(),
    }
}

pub(super) fn validate_some_to_none_header_delta(
    pre: &[u8; LOADER_PROGRAMDATA_METADATA_LEN],
    post: &[u8; LOADER_PROGRAMDATA_METADATA_LEN],
    authority: Pubkey,
) -> ProgramResult {
    let pre_parsed = parse_upgradeable_programdata(pre)?;
    let post_parsed = parse_upgradeable_programdata(post)?;
    if pre_parsed.deployed_slot > post_parsed.deployed_slot
        || pre_parsed.upgrade_authority != Some(authority)
        || post_parsed.upgrade_authority.is_some()
        || pre[..4] != post[..4]
        || pre[12] != 1
        || post[12] != 0
        || pre[13..45] != authority.to_bytes()
        // Loader-v3 serializes `None` into the existing fixed ProgramData
        // account without clearing the now-inactive authority payload bytes.
        // Require that tail to remain byte-identical to the prestate so the
        // verifier admits only the exact real Loader transition.
        || post[13..45] != pre[13..45]
    {
        return Err(GovernanceError::InvalidAuthorityTransition.into());
    }
    Ok(())
}

pub(super) fn expected_handoff_post_header(
    pre: &[u8; LOADER_PROGRAMDATA_METADATA_LEN],
    legacy: Pubkey,
    controller: Pubkey,
) -> Result<[u8; LOADER_PROGRAMDATA_METADATA_LEN], ProgramError> {
    if parse_upgradeable_programdata(pre)?.upgrade_authority != Some(legacy) {
        return Err(GovernanceError::InvalidAuthorityTransition.into());
    }
    let mut post = *pre;
    post[12] = 1;
    post[13..45].copy_from_slice(controller.as_ref());
    Ok(post)
}

pub(super) fn validate_some_to_some_header_delta(
    pre: &[u8; LOADER_PROGRAMDATA_METADATA_LEN],
    post: &[u8; LOADER_PROGRAMDATA_METADATA_LEN],
    legacy: Pubkey,
    controller: Pubkey,
) -> ProgramResult {
    if parse_upgradeable_programdata(pre)?.upgrade_authority != Some(legacy)
        || parse_upgradeable_programdata(post)?.upgrade_authority != Some(controller)
        || pre[..12] != post[..12]
        || pre[12] != 1
        || post[12] != 1
        || post[13..45] != controller.to_bytes()
        || pre[13..45] != legacy.to_bytes()
    {
        return Err(GovernanceError::InvalidAuthorityTransition.into());
    }
    Ok(())
}

pub(super) fn validate_checked_handoff_cpi_shape(
    instruction: &Instruction,
    programdata: &Pubkey,
    legacy: &Pubkey,
    controller: &Pubkey,
) -> ProgramResult {
    if instruction.program_id != UPGRADEABLE_LOADER_ID
        || instruction.accounts.len() != 3
        || instruction.accounts[0].pubkey != *programdata
        || !instruction.accounts[0].is_writable
        || instruction.accounts[0].is_signer
        || instruction.accounts[1].pubkey != *legacy
        || instruction.accounts[1].is_writable
        || !instruction.accounts[1].is_signer
        || instruction.accounts[2].pubkey != *controller
        || instruction.accounts[2].is_writable
        || !instruction.accounts[2].is_signer
        || instruction.data != 7u32.to_le_bytes()
    {
        return Err(GovernanceError::InvalidAuthorityTransition.into());
    }
    Ok(())
}

pub(super) fn validate_canonical_envelope(
    program_id: &Pubkey,
    current_accounts: &[AccountInfo<'_>],
    instructions_info: &AccountInfo<'_>,
    expected_current_data: &[u8],
    envelope: &CeremonyEnvelopeV1,
) -> ProgramResult {
    envelope.validate()?;
    if envelope.compute_unit_limit == 0
        || envelope.compute_unit_limit > MAX_CEREMONY_COMPUTE_UNIT_LIMIT_V1
        || envelope.compute_unit_price_micro_lamports
            > MAX_CEREMONY_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1
        || *instructions_info.key != sysvar_ids::instructions::ID
    {
        return Err(ProgramError::InvalidInstructionData);
    }
    let nonce = if envelope.durable_nonce_account.present {
        Some(envelope.durable_nonce_account.value)
    } else {
        None
    };
    let nonce_authority = if envelope.durable_nonce_authority.present {
        Some(envelope.durable_nonce_authority.value)
    } else {
        None
    };
    if nonce.is_some() != nonce_authority.is_some() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let current_index = if nonce.is_some() { 3usize } else { 2usize };
    if usize::from(instructions::load_current_index_checked(instructions_info)?) != current_index {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let mut index = 0usize;
    if let (Some(nonce), Some(authority)) = (nonce, nonce_authority) {
        if instructions::load_instruction_at_checked(index, instructions_info)?
            != system_instruction::advance_nonce_account(&nonce, &authority)
        {
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
