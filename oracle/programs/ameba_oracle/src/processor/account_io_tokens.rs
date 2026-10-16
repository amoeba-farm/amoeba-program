use super::*;
use crate::compact_error::CompactAccountInfo;

#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(super) fn invoke_token_transfer_checked<'a>(
    token_program_info: &AccountInfo<'a>,
    source_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    destination_info: &AccountInfo<'a>,
    authority_info: &AccountInfo<'a>,
    amount: u64,
    decimals: u8,
    signer_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let transfer = token_instruction::transfer_checked(
        token_program_info.key,
        source_info.key,
        mint_info.key,
        destination_info.key,
        authority_info.key,
        &[],
        amount,
        decimals,
    )?;
    invoke_signed(
        &transfer,
        &[
            source_info.clone(),
            mint_info.clone(),
            destination_info.clone(),
            authority_info.clone(),
            token_program_info.clone(),
        ],
        signer_seeds,
    )
}

#[inline(never)]
pub(super) fn invoke_create_associated_token_account_idempotent<'a>(
    payer_info: &AccountInfo<'a>,
    account_info: &AccountInfo<'a>,
    owner_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    token_program_info: &AccountInfo<'a>,
    associated_token_program_info: &AccountInfo<'a>,
) -> ProgramResult {
    let instruction = crate::associated_token::create_associated_token_account_idempotent(
        payer_info.key,
        owner_info.key,
        mint_info.key,
        token_program_info.key,
    );
    invoke(
        &instruction,
        &[
            payer_info.clone(),
            account_info.clone(),
            owner_info.clone(),
            mint_info.clone(),
            system_program_info.clone(),
            token_program_info.clone(),
            associated_token_program_info.clone(),
        ],
    )
}

#[inline(never)]
pub(super) fn store_state(
    account_info: &AccountInfo,
    value: &dyn crate::fixed_codec::FixedStateEncode,
) -> ProgramResult {
    let mut data = account_info.try_data_mut()?;
    if value.maximum_encoded_len() > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    // `slice::fill` compiles to a byte loop on SBF (~4 CU per byte); the memset syscall zeroes
    // the same bytes for a small fixed cost.
    let len = data.len();
    solana_program::program_memory::sol_memset(&mut data, 0, len);
    value.encode_fixed(&mut data);
    Ok(())
}

pub(super) fn validate_mint_account(
    mint_info: &AccountInfo,
    token_program: &Pubkey,
) -> Result<Mint, ProgramError> {
    if mint_info.owner != token_program {
        return Err(OracleError::InvalidMint.into());
    }
    let mint_data = mint_info.try_data()?;
    Mint::unpack(&mint_data).map_err(|_| OracleError::InvalidMint.into())
}

pub(super) fn validate_collateral_mint_account(
    mint_info: &AccountInfo,
    token_program: &Pubkey,
) -> Result<Mint, ProgramError> {
    let mint = validate_mint_account(mint_info, token_program)?;
    if !mint.is_initialized || mint.decimals != COLLATERAL_MINT_DECIMALS {
        return Err(OracleError::InvalidMint.into());
    }
    Ok(mint)
}

pub(super) fn validate_expected_collateral_freeze_authority(
    mint: &Mint,
    expected: Option<Pubkey>,
) -> ProgramResult {
    let matches = match (mint.freeze_authority, expected) {
        (COption::None, None) => true,
        (COption::Some(actual), Some(expected)) => actual == expected,
        _ => false,
    };
    if !matches {
        return Err(OracleError::InvalidMint.into());
    }
    Ok(())
}

#[inline(always)]
pub(super) fn validate_vault_token_account(
    token_info: &AccountInfo,
    expected_mint: &Pubkey,
    expected_owner: &Pubkey,
) -> Result<TokenAccount, ProgramError> {
    validate_strict_token_account(
        token_info,
        expected_mint,
        expected_owner,
        OracleError::InvalidTokenAccount,
    )
}

#[inline(never)]
pub(super) fn validate_strict_token_account(
    token_info: &AccountInfo,
    expected_mint: &Pubkey,
    expected_owner: &Pubkey,
    mismatch_error: OracleError,
) -> Result<TokenAccount, ProgramError> {
    let token = validate_token_account(token_info)?;
    if token.state != AccountState::Initialized
        || token.mint != *expected_mint
        || token.owner != *expected_owner
        || token.delegate != COption::None
        || token.delegated_amount != 0
        || token.is_native != COption::None
        || token.close_authority != COption::None
    {
        return Err(mismatch_error.into());
    }
    Ok(token)
}

pub(super) fn validate_token_account(
    token_info: &AccountInfo,
) -> Result<TokenAccount, ProgramError> {
    if !crate::token_instruction::check_id(token_info.owner) {
        return Err(OracleError::InvalidTokenAccount.into());
    }
    let token_data = token_info.try_data()?;
    TokenAccount::unpack(&token_data).map_err(|_| OracleError::InvalidTokenAccount.into())
}
