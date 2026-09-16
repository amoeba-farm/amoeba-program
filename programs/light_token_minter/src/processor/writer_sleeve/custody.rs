use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn create_classic_token_pda<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    token_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    authority: &Pubkey,
    token_program_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    signer_seeds: &[&[u8]],
) -> ProgramResult {
    validate_create_only_program_account_target(program_id, token_info)?;
    create_program_account(
        payer_info,
        token_info,
        system_program_info,
        token_program_info.key,
        TokenAccount::LEN,
        signer_seeds,
    )?;
    invoke_token_initialize_account3(token_program_info, token_info, mint_info, authority)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn load_or_create_writer_retirement_custody<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    sleeve_info: &AccountInfo<'a>,
    market_info: &AccountInfo<'a>,
    custody_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    token_program_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
) -> Result<TokenAccount, ProgramError> {
    let (expected, bump) =
        derive_writer_retirement_custody_pda(program_id, sleeve_info.key, market_info.key);
    if *custody_info.key != expected {
        return Err(VaultError::InvalidPda.into());
    }
    if custody_info.owner == token_program_info.key {
        validate_vault_token_account(custody_info, mint_info.key, sleeve_info.key)?;
        return validate_token_account(custody_info);
    }
    validate_create_only_program_account_target(program_id, custody_info)?;
    create_program_account(
        payer_info,
        custody_info,
        system_program_info,
        token_program_info.key,
        TokenAccount::LEN,
        &[
            crate::constants::WRITER_RETIREMENT_CUSTODY_PDA_SEED,
            sleeve_info.key.as_ref(),
            market_info.key.as_ref(),
            &[bump],
        ],
    )?;
    invoke_token_initialize_account3(token_program_info, custody_info, mint_info, sleeve_info.key)?;
    validate_vault_token_account(custody_info, mint_info.key, sleeve_info.key)?;
    validate_token_account(custody_info)
}

pub(super) fn close_sleeve_token_custody<'a>(
    sleeve: &WriterSleeveV1,
    sleeve_info: &AccountInfo<'a>,
    custody_info: &AccountInfo<'a>,
    recipient_info: &AccountInfo<'a>,
    token_program_info: &AccountInfo<'a>,
) -> ProgramResult {
    if validate_token_account(custody_info)?.amount != 0 {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let bump = [sleeve.bump];
    let signer_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::WRITER_SLEEVE_PDA_SEED,
        sleeve.settlement_group.as_ref(),
        &bump,
    ];
    invoke_token_close_account(
        token_program_info,
        custody_info,
        recipient_info,
        sleeve_info,
        &[signer_seeds],
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::processor::writer_sleeve) fn load_or_create_market_staging<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    market_info: &AccountInfo<'a>,
    market: &Market,
    staging_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    token_program_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
) -> Result<TokenAccount, ProgramError> {
    let (expected, bump) = derive_contract_mint_staging_pda(program_id, market_info.key);
    if *staging_info.key != expected {
        return Err(VaultError::InvalidPda.into());
    }
    if staging_info.owner == token_program_info.key {
        validate_vault_token_account(staging_info, mint_info.key, market_info.key)?;
        return validate_token_account(staging_info);
    }
    validate_create_only_program_account_target(program_id, staging_info)?;
    create_program_account(
        payer_info,
        staging_info,
        system_program_info,
        token_program_info.key,
        TokenAccount::LEN,
        &[
            CONTRACT_MINT_STAGING_PDA_SEED,
            market_info.key.as_ref(),
            &[bump],
        ],
    )?;
    invoke_token_initialize_account3(token_program_info, staging_info, mint_info, market_info.key)?;
    validate_vault_token_account(staging_info, mint_info.key, market_info.key)?;
    let _ = market;
    validate_token_account(staging_info)
}

pub(in crate::processor::writer_sleeve) fn observe_market_staging_amount(
    program_id: &Pubkey,
    market_info: &AccountInfo,
    staging_info: &AccountInfo,
    mint_info: &AccountInfo,
    token_program_info: &AccountInfo,
) -> Result<u64, ProgramError> {
    let expected = derive_contract_mint_staging_pda(program_id, market_info.key).0;
    if *staging_info.key != expected {
        return Err(VaultError::InvalidPda.into());
    }
    if staging_info.owner == token_program_info.key {
        validate_vault_token_account(staging_info, mint_info.key, market_info.key)?;
        return Ok(validate_token_account(staging_info)?.amount);
    }
    validate_create_only_program_account_target(program_id, staging_info)?;
    Ok(0)
}

pub(in crate::processor::writer_sleeve) fn observe_writer_retirement_custody_amount(
    program_id: &Pubkey,
    sleeve_info: &AccountInfo,
    market_info: &AccountInfo,
    custody_info: &AccountInfo,
    mint_info: &AccountInfo,
    token_program_info: &AccountInfo,
) -> Result<u64, ProgramError> {
    let expected =
        derive_writer_retirement_custody_pda(program_id, sleeve_info.key, market_info.key).0;
    if *custody_info.key != expected {
        return Err(VaultError::InvalidPda.into());
    }
    if custody_info.owner == token_program_info.key {
        validate_vault_token_account(custody_info, mint_info.key, sleeve_info.key)?;
        return Ok(validate_token_account(custody_info)?.amount);
    }
    validate_create_only_program_account_target(program_id, custody_info)?;
    Ok(0)
}

pub(in crate::processor::writer_sleeve) fn market_signer_seeds<'a>(
    market: &'a Market,
    bump: &'a [u8; 1],
) -> [&'a [u8]; 4] {
    [
        CURRENT_STATE_NAMESPACE_SEED,
        MARKET_PDA_SEED,
        &market.market_id,
        bump,
    ]
}
