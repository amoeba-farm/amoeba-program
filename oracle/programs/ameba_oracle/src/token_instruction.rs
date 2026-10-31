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

/// Token-2022's program id. The heads family's AMBA may live under it (`docs/heads.md`); the
/// USDC custody stays classic.
pub(crate) const TOKEN_2022_PROGRAM_ID: Pubkey =
    solana_program::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// Classic SPL Token or Token-2022.
#[inline(never)]
pub(crate) fn is_token_program(key: &Pubkey) -> bool {
    check_id(key) || crate::pubkey_eq(key, &TOKEN_2022_PROGRAM_ID)
}

fn check_any_token_program(program_id: &Pubkey) -> Result<(), ProgramError> {
    if is_token_program(program_id) {
        Ok(())
    } else {
        Err(ProgramError::IncorrectProgramId)
    }
}

/// `TransferChecked` (12) under either token program: the same bytes and accounts in both. A
/// Token-2022 mint's transfer fee is withheld in the destination; the caller measures what
/// arrived.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn transfer_checked_any(
    token_program_id: &Pubkey,
    source: &Pubkey,
    mint: &Pubkey,
    destination: &Pubkey,
    authority: &Pubkey,
    amount: u64,
    decimals: u8,
) -> Result<Instruction, ProgramError> {
    check_any_token_program(token_program_id)?;
    Ok(Instruction {
        program_id: *token_program_id,
        accounts: authority_accounts(
            vec![
                AccountMeta::new(*source, false),
                AccountMeta::new_readonly(*mint, false),
                AccountMeta::new(*destination, false),
            ],
            authority,
            &[],
        ),
        data: checked_amount_data(12, amount, decimals),
    })
}

/// `BurnChecked` (15) under either token program: burns `amount` from `account` (the authority
/// signs; for a PDA, by `invoke_signed`), reducing the mint's supply. The heads family burns
/// slashed AMBA from its vault this way, and burned USDC from the classic bond vault.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn burn_checked(
    token_program_id: &Pubkey,
    account: &Pubkey,
    mint: &Pubkey,
    authority: &Pubkey,
    amount: u64,
    decimals: u8,
) -> Result<Instruction, ProgramError> {
    check_any_token_program(token_program_id)?;
    Ok(Instruction {
        program_id: *token_program_id,
        accounts: authority_accounts(
            vec![
                AccountMeta::new(*account, false),
                AccountMeta::new(*mint, false),
            ],
            authority,
            &[],
        ),
        data: checked_amount_data(15, amount, decimals),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heads_encoders_match_spl_token_abi_under_both_programs() {
        let source = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let destination = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        for program in [TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID] {
            assert!(is_token_program(&program));
            // The classic client builds the same bytes and accounts for any program id it is
            // given; Token-2022 shares instructions 12 and 15 byte for byte.
            let mut classic = spl_token::instruction::transfer_checked(
                &spl_token::id(),
                &source,
                &mint,
                &destination,
                &authority,
                &[],
                0x0102_0304_0506_0708,
                9,
            )
            .unwrap();
            classic.program_id = program;
            assert_eq!(
                transfer_checked_any(
                    &program,
                    &source,
                    &mint,
                    &destination,
                    &authority,
                    0x0102_0304_0506_0708,
                    9
                )
                .unwrap(),
                classic
            );
            let mut classic = spl_token::instruction::burn_checked(
                &spl_token::id(),
                &source,
                &mint,
                &authority,
                &[],
                77,
                6,
            )
            .unwrap();
            classic.program_id = program;
            assert_eq!(
                burn_checked(&program, &source, &mint, &authority, 77, 6).unwrap(),
                classic
            );
        }
        let wrong = Pubkey::new_unique();
        assert!(!is_token_program(&wrong));
        assert!(
            transfer_checked_any(&wrong, &source, &mint, &destination, &authority, 1, 9).is_err()
        );
        assert!(burn_checked(&wrong, &source, &mint, &authority, 1, 9).is_err());
        assert_eq!(
            TOKEN_2022_PROGRAM_ID.to_string(),
            "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
        );
    }

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
