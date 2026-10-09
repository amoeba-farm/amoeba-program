//! Expired classic obligations enter the ordinary compressed expiry payout rails.
//! The keeper cannot select a recipient, quantity, settlement price or fee.
use super::*;

pub(super) const FIXED: usize = 27;

pub(super) fn process(
    program: &Pubkey,
    a: &[AccountInfo],
    sequence: u64,
    count: u8,
    admin_refund: bool,
) -> ProgramResult {
    if !(1..=3).contains(&count) || a.len() != FIXED + usize::from(count) {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_is_owner = admin_refund && a[0].key == a[1].key;
    for (i, info) in a.iter().enumerate() {
        let owner_alias = i == 1 && admin_is_owner;
        let writable = matches!(i, 0 | 3 | 4 | 5 | 6 | 7 | 10 | 11 | 12 | 14 | 15 | 25)
            || i >= FIXED
            || owner_alias;
        if info.is_signer != (i == 0 || owner_alias)
            || info.is_writable != writable
            || !owner_alias && a[..i].iter().any(|prior| prior.key == info.key)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    if a[1].executable
        || crate::pubkey_is_default(a[1].key)
        || !crate::light_token_instruction::is_program(a[17].key)
        || !crate::light_token_instruction::is_cpi_authority(a[18].key)
        || !crate::token_instruction::check_id(a[19].key)
        || !crate::is_system_program(a[20].key)
        || !crate::light_token_instruction::is_light_system_program(a[21].key)
        || !crate::light_token_instruction::is_registered_program(a[22].key)
        || !crate::light_token_instruction::is_compression_authority(a[23].key)
        || !crate::light_token_instruction::is_compression_program(a[24].key)
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let config = load_canonical_vault_config(program, &a[2])?;
    if admin_refund && config.admin != *a[0].key {
        return Err(VaultError::Unauthorized.into());
    }
    let pool = load_pool_with_accounts(program, &a[3], a)?;
    if !admin_refund && current_unix_timestamp()? < pool.expiry_ts
        || pool.market != *a[7].key
        || pool.option_mint != *a[8].key
        || pool.quote_mint != *a[9].key
        || config.usdc_mint != *a[9].key
    {
        return Err(VaultError::InvalidAmoebaDlmmRoute.into());
    }
    validate_collateral_mint_account(&a[8], a[19].key)?;
    validate_collateral_mint_account(&a[9], a[19].key)?;
    validate_spl_interface_account(a[8].key, &a[10])?;
    validate_spl_interface_account(a[9].key, &a[11])?;
    if *a[5].key != light_token_instruction::get_associated_token_address(a[4].key, a[8].key)
        || *a[6].key != light_token_instruction::get_associated_token_address(a[4].key, a[9].key)
    {
        return Err(VaultError::InvalidPda.into());
    }
    let mut book = load_book_with_accounts(program, &a[4], &a[3], &pool, a)?;
    // The storage engine authenticates the unchanged DORv3 bytes and neighbors.
    // This internal view supplies only its book/pool/payer/system/record roles;
    // it never goes through a swap or owner-recovery privilege validator.
    let mut storage_accounts = vec![a[0].clone(); ORDER_SWAP_FIXED_ACCOUNTS];
    storage_accounts[7] = a[3].clone();
    storage_accounts[2] = a[7].clone();
    storage_accounts[20] = a[20].clone();
    storage_accounts[31] = a[4].clone();
    storage_accounts.extend_from_slice(&a[FIXED..]);
    if a[FIXED..].iter().any(|info| info.owner != program) {
        return Err(VaultError::InvalidAccountList.into());
    }
    storage::load_classic_recovery(program, &storage_accounts, &mut book, &pool)?;
    let index = book
        .orders
        .iter()
        .position(|o| o.sequence == sequence)
        .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
    if book.orders[index].owner != *a[1].key {
        return Err(VaultError::Unauthorized.into());
    }
    let option_before = custody(&a[5], a[4].key, a[8].key)?;
    let quote_before = custody(&a[6], a[4].key, a[9].key)?;
    let resident = resident_for_pool(program, &a[3], a)?;
    let cold = resident
        .as_ref()
        .map(|(_, s)| book_ledger(program, a[4].key, &pool, s.book_option, s.book_quote));
    if !crate::compressed_custody::backs(
        cold.as_ref(),
        option_before,
        quote_before,
        book.header.option_obligations,
        book.header.quote_obligations,
    ) {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    let mut balance = book.orders[index]
        .balance(pool.tick_size_quote_atomic)
        .map_err(order_error)?;
    balance.cancel().map_err(order_error)?;
    let (options, quote) = balance.claim();
    let bump = [book.header.bump];
    let seeds: [&[u8]; 4] = [
        CURRENT_STATE_NAMESPACE_SEED,
        ORDER_BOOK_SEED,
        a[3].key.as_ref(),
        &bump,
    ];
    if options != 0 {
        if admin_refund {
            compress(a, 5, 8, 10, 4, 1, options, &seeds)?;
        } else {
            crate::processor::writer_sleeve::settle_classic_order(program, a, options, &seeds)?;
        }
    }
    compress(a, 6, 9, 11, 4, 1, quote, &seeds)?;
    if option_before.checked_sub(custody(&a[5], a[4].key, a[8].key)?) != Some(options)
        || quote_before.checked_sub(custody(&a[6], a[4].key, a[9].key)?) != Some(quote)
    {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    book.orders[index].set_balance(balance);
    book.recompute_obligations(pool.tick_size_quote_atomic)
        .map_err(order_error)?;
    if !crate::compressed_custody::backs(
        cold.as_ref(),
        custody(&a[5], a[4].key, a[8].key)?,
        custody(&a[6], a[4].key, a[9].key)?,
        book.header.option_obligations,
        book.header.quote_obligations,
    ) {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    // Empty records remain replay-safe, with rent recovery still owner signed.
    storage::persist_with_payer_staged(program, &storage_accounts, &mut book, &a[0])?;
    if !commit_resident(program, &a[3], a, |resident| {
        merge_resident_book(program, resident, &storage_accounts, &book)?;
        resident.book_hot_option = custody(&a[5], a[4].key, a[8].key)?;
        resident.book_hot_quote = custody(&a[6], a[4].key, a[9].key)?;
        Ok(())
    })? {
        store_state(&a[4], &book)?;
    }
    Ok(())
}

// Keep the existing accounting interface and its explicit inputs.
#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn compress(
    a: &[AccountInfo],
    source: usize,
    mint: usize,
    interface: usize,
    authority: usize,
    recipient: usize,
    amount: u64,
    seeds: &[&[u8]],
) -> ProgramResult {
    if amount == 0 {
        return Ok(());
    }
    let balance = || {
        if crate::token_instruction::check_id(a[source].owner) {
            validate_vault_token_account(&a[source], a[mint].key, a[authority].key)
        } else {
            load_canonical_light_token_account(&a[source], a[authority].key, a[mint].key)
        }
    };
    let before = balance()?.amount;
    let ix = light_token_instruction::compress_to_wallet(
        amount,
        MarketMintAccounting::CANONICAL_DECIMALS,
        a[source].key,
        a[source].owner,
        a[authority].key,
        a[0].key,
        a[mint].key,
        a[recipient].key,
        a[interface].key,
        [a[21].key, a[22].key, a[23].key, a[24].key, a[25].key],
    )?;
    let indices = [
        21, 0, 18, 22, 23, 24, 20, 25, mint, source, authority, recipient, interface, 19, 17,
    ];
    let infos: Vec<_> = indices.iter().map(|&i| a[i].clone()).collect();
    invoke_signed(&ix, &infos, &[seeds])?;
    if before.checked_sub(balance()?.amount) != Some(amount) {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    Ok(())
}
