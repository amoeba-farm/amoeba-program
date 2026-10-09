//! Owner-approved classic USDC ATA compression as a separate reusable deposit.
//! No session signer, caller destination, pool choice, fee or withdrawal mode.
use super::*;
use crate::regular_compressed_transfer::{self as transfer, HotCompression, OutputLeaf};
use crate::trading_session::SPONSOR_FEE_ATOMS;

pub(super) fn deposit(program: &Pubkey, a: &[AccountInfo], amount: u64) -> ProgramResult {
    if a.len() != 18
        || !a[0].is_signer
        || a[1].is_signer
        || a[15].is_signer
        || !a[15].is_writable
        || a[16].is_signer
        || !a[16].is_writable
        || a[17].is_signer
        || a[17].is_writable
        || !crate::token_instruction::check_id(a[17].key)
    {
        return Err(invalid());
    }
    let _record = funding::owner_record(program, &a[..15], true)?;
    let canonical = crate::associated_token::get_associated_token_address_with_program_id(
        a[0].key,
        a[3].key,
        &spl_token_program_id(),
    );
    if *a[15].key != canonical || !crate::token_instruction::check_id(a[15].owner) {
        return Err(invalid());
    }
    validate_vault_token_account(&a[15], a[3].key, a[0].key)?;
    validate_spl_interface_account(a[3].key, &a[16])?;
    let debit = amount
        .checked_add(SPONSOR_FEE_ATOMS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if amount == 0 || validate_token_account(&a[15])?.amount < debit {
        return Err(ProgramError::InsufficientFunds);
    }
    // Seven fixed Light CPI accounts followed by authenticated packed accounts.
    let indices = [7, 4, 6, 8, 9, 10, 11, 12, 13, 14, 0, 1, 3, 4, 15, 16, 17];
    let metas = indices
        .iter()
        .enumerate()
        .map(|(n, &i)| solana_program::instruction::AccountMeta {
            pubkey: *a[i].key,
            is_signer: matches!(n, 1 | 10 | 13),
            is_writable: matches!(n, 1 | 7 | 8 | 9 | 14 | 15),
        })
        .collect();
    let compression = HotCompression {
        amount: debit,
        mint: 5,
        source: 7,
        authority: 3,
        pool_account_index: 8,
        pool_index: 0,
        bump: crate::light_token_instruction::get_spl_interface_pda_and_bump(a[3].key).1,
        decimals: 6,
    };
    let outputs = [
        OutputLeaf {
            owner: 4,
            amount,
            has_delegate: false,
            delegate: 0,
            mint: 5,
        },
        OutputLeaf {
            owner: 6,
            amount: SPONSOR_FEE_ATOMS,
            has_delegate: false,
            delegate: 0,
            mint: 5,
        },
    ];
    let ix = transfer::instruction_with_compressions(
        *a[5].key,
        metas,
        2,
        None,
        &[],
        &[compression],
        &outputs,
    )?;
    let mut infos: Vec<_> = indices.iter().map(|&i| a[i].clone()).collect();
    infos.push(a[5].clone());
    invoke(&ix, &infos)?;
    Ok(())
}
