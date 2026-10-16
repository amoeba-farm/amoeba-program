//! Minimal, byte-exact classic SPL Token instruction constructors used by the runtime.
//!
//! Keeping the canonical layouts and the program id local avoids linking the general-purpose
//! token client into the SBF artifact.

use crate::ProgramError;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

pub(crate) const TOKEN_PROGRAM_ID: Pubkey =
    solana_program::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

#[inline(never)]
pub(crate) const fn id() -> Pubkey {
    TOKEN_PROGRAM_ID
}

#[inline(never)]
pub(crate) fn check_id(key: &Pubkey) -> bool {
    crate::pubkey_eq(key, &TOKEN_PROGRAM_ID)
}

fn check_program_account(program_id: &Pubkey) -> Result<(), ProgramError> {
    if check_id(program_id) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_constructors_match_spl_token_abi() {
        let program = spl_token::id();
        assert_eq!(id(), program);
        let source = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let destination = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let signer = Pubkey::new_unique();
        let signers = [&signer];

        for signer_pubkeys in [&signers[..], &[]] {
            assert_eq!(
                transfer_checked(
                    &program,
                    &source,
                    &mint,
                    &destination,
                    &authority,
                    signer_pubkeys,
                    0x0102_0304_0506_0708,
                    9,
                )
                .unwrap(),
                spl_token::instruction::transfer_checked(
                    &program,
                    &source,
                    &mint,
                    &destination,
                    &authority,
                    signer_pubkeys,
                    0x0102_0304_0506_0708,
                    9,
                )
                .unwrap(),
            );
        }
    }

    #[test]
    fn local_constructors_match_invalid_program_error() {
        let wrong = Pubkey::new_unique();
        let key = Pubkey::new_unique();
        assert_eq!(
            transfer_checked(&wrong, &key, &key, &key, &key, &[], 1, 6)
                .map_err(solana_program::program_error::ProgramError::from),
            spl_token::instruction::transfer_checked(&wrong, &key, &key, &key, &key, &[], 1, 6),
        );
    }
}
