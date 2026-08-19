//! Minimal, byte-exact classic SPL Token instruction constructors used by the runtime.
//!
//! Keeping these five canonical layouts and the program id local avoids linking the
//! general-purpose token client into the SBF artifact.

use solana_program::{
    instruction::{AccountMeta, Instruction},
    program_error::ProgramError,
    pubkey::Pubkey,
};

pub(crate) const TOKEN_PROGRAM_ID: Pubkey =
    solana_program::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

pub(crate) const fn id() -> Pubkey {
    TOKEN_PROGRAM_ID
}

fn check_program_account(program_id: &Pubkey) -> Result<(), ProgramError> {
    if *program_id == id() {
        Ok(())
    } else {
        Err(ProgramError::IncorrectProgramId)
    }
}

fn checked_amount_data(tag: u8, amount: u64, decimals: u8) -> Vec<u8> {
    let mut data = Vec::with_capacity(10);
    data.push(tag);
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(decimals);
    data
}

fn authority_accounts(
    mut accounts: Vec<AccountMeta>,
    authority: &Pubkey,
    signer_pubkeys: &[&Pubkey],
) -> Vec<AccountMeta> {
    accounts.push(AccountMeta::new_readonly(
        *authority,
        signer_pubkeys.is_empty(),
    ));
    for signer in signer_pubkeys {
        accounts.push(AccountMeta::new_readonly(**signer, true));
    }
    accounts
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn transfer_checked(
    token_program_id: &Pubkey,
    source: &Pubkey,
    mint: &Pubkey,
    destination: &Pubkey,
    authority: &Pubkey,
    signer_pubkeys: &[&Pubkey],
    amount: u64,
    decimals: u8,
) -> Result<Instruction, ProgramError> {
    check_program_account(token_program_id)?;
    Ok(Instruction {
        program_id: *token_program_id,
        accounts: authority_accounts(
            vec![
                AccountMeta::new(*source, false),
                AccountMeta::new_readonly(*mint, false),
                AccountMeta::new(*destination, false),
            ],
            authority,
            signer_pubkeys,
        ),
        data: checked_amount_data(12, amount, decimals),
    })
}

pub(crate) fn mint_to_checked(
    token_program_id: &Pubkey,
    mint: &Pubkey,
    destination: &Pubkey,
    authority: &Pubkey,
    signer_pubkeys: &[&Pubkey],
    amount: u64,
    decimals: u8,
) -> Result<Instruction, ProgramError> {
    check_program_account(token_program_id)?;
    Ok(Instruction {
        program_id: *token_program_id,
        accounts: authority_accounts(
            vec![
                AccountMeta::new(*mint, false),
                AccountMeta::new(*destination, false),
            ],
            authority,
            signer_pubkeys,
        ),
        data: checked_amount_data(14, amount, decimals),
    })
}

pub(crate) fn burn_checked(
    token_program_id: &Pubkey,
    source: &Pubkey,
    mint: &Pubkey,
    authority: &Pubkey,
    signer_pubkeys: &[&Pubkey],
    amount: u64,
    decimals: u8,
) -> Result<Instruction, ProgramError> {
    check_program_account(token_program_id)?;
    Ok(Instruction {
        program_id: *token_program_id,
        accounts: authority_accounts(
            vec![
                AccountMeta::new(*source, false),
                AccountMeta::new(*mint, false),
            ],
            authority,
            signer_pubkeys,
        ),
        data: checked_amount_data(15, amount, decimals),
    })
}

pub(crate) fn close_account(
    token_program_id: &Pubkey,
    account: &Pubkey,
    destination: &Pubkey,
    authority: &Pubkey,
    signer_pubkeys: &[&Pubkey],
) -> Result<Instruction, ProgramError> {
    check_program_account(token_program_id)?;
    Ok(Instruction {
        program_id: *token_program_id,
        accounts: authority_accounts(
            vec![
                AccountMeta::new(*account, false),
                AccountMeta::new(*destination, false),
            ],
            authority,
            signer_pubkeys,
        ),
        data: vec![9],
    })
}

pub(crate) fn initialize_account3(
    token_program_id: &Pubkey,
    account: &Pubkey,
    mint: &Pubkey,
    owner: &Pubkey,
) -> Result<Instruction, ProgramError> {
    check_program_account(token_program_id)?;
    let mut data = Vec::with_capacity(33);
    data.push(18);
    data.extend_from_slice(owner.as_ref());
    Ok(Instruction {
        program_id: *token_program_id,
        accounts: vec![
            AccountMeta::new(*account, false),
            AccountMeta::new_readonly(*mint, false),
        ],
        data,
    })
}

pub(crate) fn initialize_mint2(
    token_program_id: &Pubkey,
    mint: &Pubkey,
    mint_authority: &Pubkey,
    freeze_authority: Option<&Pubkey>,
    decimals: u8,
) -> Result<Instruction, ProgramError> {
    check_program_account(token_program_id)?;
    let mut data = Vec::with_capacity(if freeze_authority.is_some() { 67 } else { 35 });
    data.push(20);
    data.push(decimals);
    data.extend_from_slice(mint_authority.as_ref());
    match freeze_authority {
        Some(authority) => {
            data.push(1);
            data.extend_from_slice(authority.as_ref());
        }
        None => data.push(0),
    }
    Ok(Instruction {
        program_id: *token_program_id,
        accounts: vec![AccountMeta::new(*mint, false)],
        data,
    })
}
