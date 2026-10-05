use super::*;
use crate::regular_compressed_transfer::{self as transfer, InputLeaf, OutputLeaf};
use crate::trading_session::{Funding, SEED, SPONSOR_FEE_ATOMS};

pub(super) fn validate_accounts(
    program: &Pubkey,
    a: &[AccountInfo],
    sponsor: &Pubkey,
    mint: &Pubkey,
) -> ProgramResult {
    if a.len() != 15
        || !a[0].is_signer
        || !a[1].is_writable
        || a[1].is_signer
        || !a[4].is_signer
        || !a[4].is_writable
        || a[4].key != sponsor
        || a[3].key != mint
        || *a[5].key != light_token_program_id()
        || *a[6].key != cpi_authority()
        || *a[7].key != Pubkey::new_from_array(light_sdk::constants::LIGHT_SYSTEM_PROGRAM_ID)
        || *a[8].key != Pubkey::new_from_array(light_sdk::constants::REGISTERED_PROGRAM_PDA)
        || *a[9].key
            != Pubkey::new_from_array(light_sdk::constants::ACCOUNT_COMPRESSION_AUTHORITY_PDA)
        || *a[10].key
            != Pubkey::new_from_array(light_sdk::constants::ACCOUNT_COMPRESSION_PROGRAM_ID)
        || *a[11].key != system_program::id()
        || a[12..]
            .iter()
            .any(|i| !i.is_writable || i.is_signer || i.executable)
        || a[12].key == a[13].key
        || a.iter()
            .enumerate()
            .any(|(n, i)| !matches!(n, 0 | 4) && i.is_signer)
        || a[2..12]
            .iter()
            .enumerate()
            .any(|(n, i)| n != 2 && i.is_writable)
        || *a[1].key != session::derive(program, a[0].key).0
    {
        return Err(invalid());
    }
    let config = load_canonical_vault_config(program, &a[2])?;
    if config.usdc_mint != *mint {
        return Err(invalid());
    }
    validate_collateral_mint_account(&a[3], &spl_token_program_id())?;
    Ok(())
}

/// Funding can create dormant custody before a trading key is enabled.
pub(super) fn owner_record(
    program: &Pubkey,
    a: &[AccountInfo],
    allow_create: bool,
) -> Result<TradingSessionV2, ProgramError> {
    if a.len() != 15 {
        return Err(invalid());
    }
    if a[1].owner == program {
        let record = load(program, &a[1], a[0].key)?;
        validate_accounts(program, a, &record.sponsor, &record.quote_mint)?;
        return Ok(record);
    }
    if !allow_create
        || a[1].owner != &system_program::id()
        || !a[1].data_is_empty()
        || a[1].executable
    {
        return Err(invalid());
    }
    validate_accounts(program, a, a[4].key, a[3].key)?;
    let bump = session::derive(program, a[0].key).1;
    let record =
        TradingSessionV2::dormant(*a[0].key, *a[3].key, *a[4].key, bump).ok_or_else(invalid)?;
    if record.sponsor == *a[1].key {
        return Err(invalid());
    }
    create_program_account(
        &a[4],
        &a[1],
        &a[11],
        program,
        session::ACCOUNT_SIZE,
        &[SEED, a[0].key.as_ref(), &[bump]],
    )?;
    store(&a[1], &record)?;
    Ok(record)
}

pub(super) fn owner_move(
    program: &Pubkey,
    a: &[AccountInfo],
    movement: Funding,
    withdraw: bool,
) -> ProgramResult {
    if a.len() != 15 || !a[0].is_signer {
        return Err(invalid());
    }
    let record = owner_record(program, a, !withdraw)?;
    // Wallet authorization is required even after expiration/revocation.
    move_leaf(a, movement, withdraw, Some(&record))
}

fn move_leaf(
    a: &[AccountInfo],
    movement: Funding,
    withdraw: bool,
    record: Option<&TradingSessionV2>,
) -> ProgramResult {
    let fee = if withdraw { 0 } else { SPONSOR_FEE_ATOMS };
    let debit = movement
        .amount
        .checked_add(fee)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if movement.amount == 0
        || movement.input_amount < debit
        || (movement.prove_by_index && movement.root_index != 0)
        || (!movement.prove_by_index && movement.proof.is_none())
    {
        return Err(invalid());
    }
    let indices = [7, 4, 6, 8, 9, 10, 11, 12, 13, 14, 0, 1, 3, 4];
    let metas = indices
        .iter()
        .enumerate()
        .map(|(n, &i)| solana_program::instruction::AccountMeta {
            pubkey: *a[i].key,
            is_signer: matches!(n, 1 | 13) || n == if withdraw { 11 } else { 10 },
            is_writable: matches!(n, 1 | 7 | 8 | 9),
        })
        .collect();
    let source = if withdraw { 4 } else { 3 };
    let destination = if withdraw { 3 } else { 4 };
    let input = InputLeaf {
        owner: source,
        amount: movement.input_amount,
        has_delegate: false,
        delegate: 0,
        mint: 5,
        tree: 0,
        queue: 1,
        leaf_index: movement.leaf_index,
        root_index: movement.root_index,
        prove_by_index: movement.prove_by_index,
    };
    let mut outputs = vec![OutputLeaf {
        owner: destination,
        amount: movement.amount,
        has_delegate: false,
        delegate: 0,
        mint: 5,
    }];
    if fee > 0 {
        outputs.push(OutputLeaf {
            owner: 6,
            amount: fee,
            has_delegate: false,
            delegate: 0,
            mint: 5,
        });
    }
    if movement.input_amount > debit {
        outputs.push(OutputLeaf {
            owner: source,
            amount: movement.input_amount - debit,
            has_delegate: false,
            delegate: 0,
            mint: 5,
        });
    }
    let ix = transfer::instruction(*a[5].key, metas, 2, movement.proof, &[input], &outputs)?;
    let mut infos: Vec<_> = indices.iter().map(|&i| a[i].clone()).collect();
    infos.push(a[5].clone());
    if withdraw {
        let bump = [record.ok_or_else(invalid)?.bump];
        let seeds: &[&[u8]] = &[CURRENT_STATE_NAMESPACE_SEED, SEED, a[0].key.as_ref(), &bump];
        invoke_signed(&ix, &infos, &[seeds])
    } else {
        invoke(&ix, &infos)
    }
}
