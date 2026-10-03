//! Recovery for deployed classic owner-funded records. No placement or trading.
//! The action body retains deployed 3cb40d6 semantics and canonical Light CPI
//! checks. Classic record admission is isolated from the compressed trade loader.
use super::*;
use crate::scoped_settlement::derive_collective_settlement_delegate;

pub(super) fn process(
    program: &Pubkey,
    a: &[AccountInfo],
    action: DlmmOrderAction,
) -> ProgramResult {
    if a.len() < ORDER_SWAP_FIXED_ACCOUNTS
        || a.len() > ORDER_SWAP_FIXED_ACCOUNTS + crate::dlmm_order_state::MAX_ORDER_WITNESSES
        || !a[0].is_signer
        || !a[0].is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let witness_end = storage::witness_end(a)?;
    // Recovery admits no reserve pages, compressed sidecar or trading suffix.
    if a.len() != witness_end {
        return Err(VaultError::InvalidAccountList.into());
    }
    validate_pack_dlmm_account_privileges(
        AmoebaDlmmInstructionTag::SwapCollectiveDlmmExactInV1,
        &a[..31],
    )?;
    for index in 31..34 {
        if !a[index].is_writable
            || a[index].is_signer
            || a[index].executable
            || a.iter()
                .enumerate()
                .any(|(other, info)| other != index && crate::pubkey_eq(info.key, a[index].key))
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    assert_program_accounts(&a[15], &a[16], &a[19], &a[20])?;
    let mut pool = load_pool(program, &a[7])?;
    let config = load_canonical_vault_config(program, &a[1])?;
    if pool.quote_mint != config.usdc_mint
        || *a[9].key != pool.option_mint
        || *a[10].key != pool.quote_mint
        || *a[2].key != pool.market
        || *a[3].key != pool.oracle_month
        || *a[21].key != light_token_instruction::compressible_config()
        || *a[22].key != light_token_instruction::rent_sponsor()
    {
        return Err(VaultError::InvalidAmoebaDlmmPool.into());
    }
    validate_collateral_mint_account(&a[9], a[19].key)?;
    validate_collateral_mint_account(&a[10], a[19].key)?;
    validate_spl_interface_account(a[9].key, &a[17])?;
    validate_spl_interface_account(a[10].key, &a[18])?;
    // Pause/expiry do not extinguish deployed owner recovery rights.
    let mut book = load_book(program, &a[31], &a[7], &pool)?;
    storage::load_classic_recovery(program, a, &mut book, &pool)?;
    let before_option = custody(&a[32], a[31].key, &pool.option_mint)?;
    let before_quote = custody(&a[33], a[31].key, &pool.quote_mint)?;
    if before_option < book.header.option_obligations
        || before_quote < book.header.quote_obligations
    {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    let mut state = OrderSwapState {
        book,
        taker_sequence: None,
        direct_bid_delivery: false,
        post_only: false,
        maximum_fills: MAX_ORDER_FILLS,
        before_option,
        before_quote,
        book_custody: None,
    };
    match action {
        DlmmOrderAction::Cancel { sequence }
        | DlmmOrderAction::Claim { sequence }
        | DlmmOrderAction::Close { sequence } => {
            if a.len() != witness_end {
                return Err(VaultError::InvalidAccountList.into());
            }
            let index = state
                .book
                .orders
                .iter()
                .position(|order| order.sequence == sequence)
                .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
            if state.book.orders[index].owner != *a[0].key {
                return Err(VaultError::Unauthorized.into());
            }
            let mut balance = state.book.orders[index]
                .balance(pool.tick_size_quote_atomic)
                .map_err(order_error)?;
            match action {
                DlmmOrderAction::Cancel { .. } => balance.cancel().map_err(order_error)?,
                DlmmOrderAction::Claim { .. } => {
                    let (options, quote) = balance.claim();
                    let bump = [state.book.header.bump];
                    let seeds: &[&[u8]] = &[
                        CURRENT_STATE_NAMESPACE_SEED,
                        ORDER_BOOK_SEED,
                        a[7].key.as_ref(),
                        &bump,
                    ];
                    for (amount, source, destination, mint, interface) in [
                        (options, &a[32], &a[13], &a[9], &a[17]),
                        (quote, &a[33], &a[14], &a[10], &a[18]),
                    ] {
                        if amount == 0 {
                            continue;
                        }
                        if destination.owner == &system_program::id() {
                            load_or_create_light_associated_token_account(
                                &a[0],
                                &a[0],
                                mint,
                                destination,
                                &a[15],
                                &a[21],
                                &a[22],
                                &a[20],
                            )?;
                        }
                        let before =
                            load_recovery_destination(program, destination, a[0].key, mint.key)?
                                .amount;
                        let custody_before = custody(source, a[31].key, mint.key)?;
                        invoke_light_token_account_transfer_with_signer_seeds(
                            amount,
                            MarketMintAccounting::CANONICAL_DECIMALS,
                            &a[15],
                            &a[16],
                            &a[0],
                            source,
                            destination,
                            &a[31],
                            mint,
                            interface,
                            &a[19],
                            &a[20],
                            &[seeds],
                        )?;
                        if custody_before.checked_sub(custody(source, a[31].key, mint.key)?)
                            != Some(amount)
                            || load_recovery_destination(program, destination, a[0].key, mint.key)?
                                .amount
                                .checked_sub(before)
                                != Some(amount)
                        {
                            return Err(VaultError::AmoebaDlmmInvariantViolation.into());
                        }
                    }
                    if options > 0 {
                        authorize_collective_settlement(
                            program,
                            &[
                                a[0].clone(),
                                a[2].clone(),
                                a[9].clone(),
                                a[13].clone(),
                                a[23].clone(),
                                a[15].clone(),
                                a[21].clone(),
                                a[22].clone(),
                                a[20].clone(),
                            ],
                            false,
                        )?;
                    }
                }
                DlmmOrderAction::Close { .. } => {
                    if balance.remaining_quantity != 0
                        || balance.obligations().map_err(order_error)? != (0, 0)
                    {
                        return Err(VaultError::AmoebaDlmmNotEmpty.into());
                    }
                    state.book.orders.remove(index);
                    state
                        .book
                        .recompute_obligations(pool.tick_size_quote_atomic)
                        .map_err(order_error)?;
                    return persist_book(program, a, &mut state.book);
                }
                _ => unreachable!(),
            }
            state.book.orders[index].set_balance(balance);
            state
                .book
                .recompute_obligations(pool.tick_size_quote_atomic)
                .map_err(order_error)?;
            if custody(&a[32], a[31].key, &pool.option_mint)? < state.book.header.option_obligations
                || custody(&a[33], a[31].key, &pool.quote_mint)?
                    < state.book.header.quote_obligations
            {
                return Err(VaultError::AmoebaDlmmInvariantViolation.into());
            }
            persist_book(program, a, &mut state.book)
        }
        DlmmOrderAction::CloseBook => {
            if a.len() != ORDER_SWAP_FIXED_ACCOUNTS
                || state.book.header.record_count != 0
                || state.book.header.option_obligations != 0
                || state.book.header.quote_obligations != 0
                || state.book.header.bid_head != 0
                || state.book.header.ask_head != 0
                || state.book.header.continuation_sequence != 0
                || pool.status != AmoebaDlmmPoolStatus::Settled
                || state.book.header.rent_payer != *a[0].key
            {
                return Err(VaultError::AmoebaDlmmNotEmpty.into());
            }
            pool.position_count = pool
                .position_count
                .checked_sub(1)
                .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
            pool.account_version = AMOEBA_DLMM_ACCOUNT_VERSION;
            store_light_state(&a[7], &pool)?;
            close_program_account(program, &a[31], &a[0])
        }
        _ => Err(VaultError::InvalidAmoebaDlmmRoute.into()),
    }
}

fn load_recovery_destination(
    program: &Pubkey,
    info: &AccountInfo,
    owner: &Pubkey,
    mint: &Pubkey,
) -> Result<TokenAccount, ProgramError> {
    if info.owner == &spl_token_program_id() {
        validate_vault_token_account(info, mint, owner)
    } else {
        load_scoped_holder_token_account(program, info, owner, mint)
    }
}
/// User token accounts may carry only this program's exact wallet/mint capability.
/// Vaults and other protocol custody retain the ordinary no-delegate validator.
fn load_scoped_holder_token_account(
    program_id: &Pubkey,
    account: &AccountInfo,
    owner: &Pubkey,
    mint: &Pubkey,
) -> Result<TokenAccount, ProgramError> {
    validate_light_token_account(account)?;
    validate_light_associated_token_address(owner, mint, account)?;
    let data = account.try_borrow_data()?;
    if !has_canonical_compressible_token_layout(&data) {
        return Err(VaultError::InvalidLightTokenAccount.into());
    }
    let token = TokenAccount::unpack(&data[..TokenAccount::LEN])
        .map_err(|_| VaultError::InvalidLightTokenAccount)?;
    let delegation_ok = match token.delegate {
        COption::None => token.delegated_amount == 0,
        COption::Some(delegate) => {
            delegate == derive_collective_settlement_delegate(program_id, owner, mint).0
        }
    };
    if token.owner != *owner
        || token.mint != *mint
        || token.state != AccountState::Initialized
        || !delegation_ok
        || token.is_native != COption::None
        || token.close_authority != COption::None
    {
        return Err(VaultError::InvalidLightTokenAccount.into());
    }
    Ok(token)
}

/// Retained only inside owner-signed classic Claim for delivered option proceeds.
fn authorize_collective_settlement(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    revoke: bool,
) -> ProgramResult {
    if accounts.len() != 9 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let owner = &accounts[0];
    let market = load_valid_market(program_id, &accounts[1])?;
    let mint = &accounts[2];
    let source = &accounts[3];
    let delegate = &accounts[4];
    let light = &accounts[5];
    if !owner.is_signer || !owner.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if market.long_contract_mint != Some(*mint.key)
        || *delegate.key != derive_collective_settlement_delegate(program_id, owner.key, mint.key).0
        || *light.key != light_token_program_id()
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    validate_collateral_mint_account(mint, &spl_token_program_id())?;
    if !revoke && source.owner == &system_program::id() {
        load_or_create_light_associated_token_account(
            owner,
            owner,
            mint,
            source,
            light,
            &accounts[6],
            &accounts[7],
            &accounts[8],
        )?;
    }
    let _ = load_scoped_holder_token_account(program_id, source, owner.key, mint.key)?;
    let instruction = if revoke {
        recovery_revoke(source.key, owner.key)
    } else {
        recovery_approve(source.key, delegate.key, owner.key, u64::MAX)
    };
    invoke(
        &instruction,
        &[
            source.clone(),
            delegate.clone(),
            owner.clone(),
            light.clone(),
        ],
    )?;
    let token = load_scoped_holder_token_account(program_id, source, owner.key, mint.key)?;
    if (revoke && token.delegate != COption::None)
        || (!revoke
            && (token.delegate != COption::Some(*delegate.key)
                || token.delegated_amount != u64::MAX))
    {
        return Err(VaultError::InvalidLightTokenAccount.into());
    }
    Ok(())
}

// Deployed Light Token 0.23 wire constructors, kept private to recovery.
fn recovery_approve(
    source: &Pubkey,
    delegate: &Pubkey,
    owner: &Pubkey,
    amount: u64,
) -> solana_program::instruction::Instruction {
    use solana_program::instruction::{AccountMeta, Instruction};
    let mut data = vec![4];
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: light_token_program_id(),
        data,
        accounts: vec![
            AccountMeta::new(*source, false),
            AccountMeta::new_readonly(*delegate, false),
            AccountMeta::new_readonly(*owner, true),
        ],
    }
}
fn recovery_revoke(source: &Pubkey, owner: &Pubkey) -> solana_program::instruction::Instruction {
    use solana_program::instruction::{AccountMeta, Instruction};
    Instruction {
        program_id: light_token_program_id(),
        data: vec![5],
        accounts: vec![
            AccountMeta::new(*source, false),
            AccountMeta::new_readonly(*owner, true),
        ],
    }
}
