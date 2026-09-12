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
