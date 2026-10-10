//! Regular compressed owner and order-book escrow. A single Transfer2 consumes
//! the whole witnessed leaves, returns owner change, pays the fixed sponsor fee,
//! and updates the authenticated book sidecar in the same transaction.
use super::*;
use crate::{
    ameba_dlmm_instruction::CompressedSwapLeafWitnessV1,
    compressed_custody::{derive_compressed_custody, CustodyKind},
    regular_compressed_transfer::{self, InputLeaf, OutputLeaf},
};
use solana_program::instruction::AccountMeta;

const TAIL: usize = 7;
const SPONSOR_FEE: u64 = 10_000;

fn valid_witness(w: &CompressedSwapLeafWitnessV1, count: usize) -> bool {
    usize::from(w.tree_index) < count
        && usize::from(w.queue_index) < count
        && w.tree_index != w.queue_index
}

fn input(
    w: CompressedSwapLeafWitnessV1,
    owner: u8,
    amount: u64,
    mint: u8,
    delegate: Option<u8>,
) -> InputLeaf {
    InputLeaf {
        owner,
        amount,
        has_delegate: delegate.is_some(),
        delegate: delegate.unwrap_or(0),
        mint,
        tree: w.tree_index,
        queue: w.queue_index,
        leaf_index: w.leaf_index,
        prove_by_index: w.prove_by_index,
        root_index: w.root_index,
    }
}

fn output(owner: u8, amount: u64, mint: u8, delegate: Option<u8>) -> OutputLeaf {
    OutputLeaf {
        owner,
        amount,
        has_delegate: delegate.is_some(),
        delegate: delegate.unwrap_or(0),
        mint,
    }
}

pub(super) fn exit(program: &Pubkey, a: &[AccountInfo], action: DlmmOrderAction) -> ProgramResult {
    exit_with_authority(program, a, action, None)
}
pub(super) fn exit_with_authority(
    program: &Pubkey,
    a: &[AccountInfo],
    action: DlmmOrderAction,
    trading_owner: Option<Pubkey>,
) -> ProgramResult {
    let (cancel, admin_refund, params) = match action {
        DlmmOrderAction::CancelCompressedEscrow { params } => (true, false, params),
        DlmmOrderAction::ClaimCompressedEscrow { params } => (false, false, params),
        DlmmOrderAction::AdminRefundCompressedEscrow { params } => (true, true, params),
        _ => return Err(VaultError::InvalidAmoebaDlmmRoute.into()),
    };
    if a.len() < ORDER_SWAP_FIXED_ACCOUNTS {
        return Err(VaultError::InvalidAccountList.into());
    }
    // A claim (never a cancel) may be submitted by anyone: a keeper or the
    // sponsor signs and pays as tail[1]. It moves only the order's claimable
    // balances to the order owner, account zero, which must equal the record's
    // owner below. It charges the owner no sponsor fee and pays the caller nothing.
    let keeper = !cancel && !a[0].is_signer;
    let fee = if admin_refund
        || keeper
        || a.get(ORDER_SWAP_FIXED_ACCOUNTS + usize::from(params.record_count) + 1)
            .is_some_and(|s| s.key == a[0].key)
    {
        0
    } else {
        SPONSOR_FEE
    };
    if fee == 0 && (params.fee_input.is_some() || params.fee_input_amount != 0) {
        return Err(VaultError::InvalidAccountList.into());
    }
    let count = usize::from(params.merkle_account_count);
    let records = usize::from(params.record_count);
    let prefix_end = ORDER_SWAP_FIXED_ACCOUNTS + records;
    if records == 0
        || records > crate::dlmm_order_state::MAX_ORDER_WITNESSES
        || count < 2
        || a.len() != prefix_end + TAIL + count
        || !admin_refund && !keeper && !a[0].is_signer
        || !a[0].is_writable
        || usize::from(params.output_tree_index) >= count
        || usize::from(params.output_queue_index) >= count
        || params.output_tree_index == params.output_queue_index
        || params
            .book_option_input
            .is_some_and(|w| !valid_witness(&w, count))
        || params
            .book_quote_input
            .is_some_and(|w| !valid_witness(&w, count))
        || params.fee_input.is_some_and(|w| !valid_witness(&w, count))
        || (params.proof.is_none()
            && [
                params.book_option_input,
                params.book_quote_input,
                params.fee_input,
            ]
            .into_iter()
            .flatten()
            .any(|w| !w.prove_by_index))
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let prefix = &a[..prefix_end];
    let tail = &a[prefix_end..prefix_end + TAIL];
    let merkle = &a[prefix_end + TAIL..];
    if storage::witness_end(prefix)? != prefix_end
        || *tail[0].key != derive_compressed_custody(program, CustodyKind::OrderBook, a[31].key).0
        || !tail[0].is_writable
        || tail[0].is_signer
        || crate::pubkey_is_default(tail[1].key)
        || !tail[1].is_signer
        || !tail[1].is_writable
        || !(crate::is_system_program(tail[2].key) || *tail[2].key == *a[23].key)
        || tail[2].is_signer
        || tail[2].is_writable
        || !crate::light_token_instruction::is_light_system_program(tail[3].key)
        || !crate::light_token_instruction::is_registered_program(tail[4].key)
        || !crate::light_token_instruction::is_compression_authority(tail[5].key)
        || !crate::light_token_instruction::is_compression_program(tail[6].key)
        || tail[3..7]
            .iter()
            .any(|info| info.is_signer || info.is_writable)
        || merkle
            .iter()
            .any(|info| !info.is_writable || info.is_signer || info.executable)
        || merkle.iter().enumerate().any(|(i, info)| {
            merkle[..i]
                .iter()
                .any(|prior| crate::pubkey_eq(prior.key, info.key))
        })
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let config = load_canonical_vault_config(program, &a[1])?;
    if admin_refund && (trading_owner.is_some() || config.admin != *tail[1].key) {
        return Err(VaultError::Unauthorized.into());
    }
    let mut base: Vec<_> = prefix[..31].to_vec();
    // Only the privilege validator receives this view. The authenticated admin
    // authorizes returning Book custody, and a keeper claim only returns claimable
    // custody to the owner; no CPI requests the owner's signature in either case.
    if admin_refund || keeper {
        base[0].is_signer = true;
    }
    validate_collective_swap_base_privileges(program, &base)?;
    for index in 31..34 {
        if !prefix[index].is_writable
            || prefix[index].is_signer
            || prefix[index].executable
            || prefix.iter().enumerate().any(|(other, info)| {
                other != index && crate::pubkey_eq(info.key, prefix[index].key)
            })
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    assert_program_accounts(&a[15], &a[16], &a[19], &a[20])?;
    let pool = load_pool_with_accounts(program, &a[7], a)?;
    let resident = resident_for_pool(program, &a[7], a)?;
    if pool.quote_mint != config.usdc_mint
        || *a[9].key != pool.option_mint
        || *a[10].key != pool.quote_mint
        || *a[2].key != pool.market
        || *a[3].key != pool.oracle_month
        || *a[21].key != light_token_instruction::compressible_config()
        || *a[22].key != light_token_instruction::rent_sponsor()
        || *a[23].key
            != crate::scoped_settlement::derive_collective_settlement_delegate(
                program, a[0].key, a[9].key,
            )
            .0
    {
        return Err(VaultError::InvalidAmoebaDlmmPool.into());
    }
    validate_collateral_mint_account(&a[9], a[19].key)?;
    validate_collateral_mint_account(&a[10], a[19].key)?;
    validate_spl_interface_account(a[9].key, &a[17])?;
    validate_spl_interface_account(a[10].key, &a[18])?;
    let mut state = load_swap_state(program, prefix, &pool, Some(&tail[0]))?;
    let index = state
        .book
        .orders
        .iter()
        .position(|order| order.sequence == params.sequence)
        .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
    let order = &state.book.orders[index];
    if order.owner != *a[0].key || !order.has_compressed_escrow_funding() {
        return Err(VaultError::Unauthorized.into());
    }
    if (params.expected_option_atoms > 0 && order.has_compressed_delegate_consent())
        != (*tail[2].key == *a[23].key)
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut balance = order
        .balance(pool.tick_size_quote_atomic)
        .map_err(order_error)?;
    if cancel {
        balance.cancel().map_err(order_error)?;
    }
    let (option_return, quote_return) = balance.claim();
    if option_return != params.expected_option_atoms
        || quote_return != params.expected_quote_atoms
        || option_return == 0 && quote_return == 0
    {
        return Err(VaultError::InvalidAmoebaDlmmRoute.into());
    }
    let fee_from_return = quote_return >= fee;
    if fee_from_return != params.fee_input.is_none()
        || (fee_from_return && params.fee_input_amount != 0)
        || (!fee_from_return && params.fee_input_amount < fee)
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut custody = state
        .book_custody
        .take()
        .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
    if custody.option_atoms < option_return
        || custody.quote_atoms < quote_return
        || params.book_option_input.is_some()
            != (resident
                .as_ref()
                .map(|(_, s)| s.total_option())
                .transpose()?
                .unwrap_or(custody.option_atoms)
                > 0)
        || params.book_quote_input.is_some()
            != (resident
                .as_ref()
                .map(|(_, s)| s.total_quote())
                .transpose()?
                .unwrap_or(custody.quote_atoms)
                > 0)
    {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    let option_after = custody.option_atoms - option_return;
    let quote_after = custody.quote_atoms - quote_return;
    let pool_cold = resident
        .as_ref()
        .map(|(_, s)| (s.pool_option, s.pool_quote))
        .unwrap_or((0, 0));
    let aggregate_before = (
        custody
            .option_atoms
            .checked_add(pool_cold.0)
            .ok_or(VaultError::ArithmeticOverflow)?,
        custody
            .quote_atoms
            .checked_add(pool_cold.1)
            .ok_or(VaultError::ArithmeticOverflow)?,
    );
    let aggregate_after = (
        option_after
            .checked_add(pool_cold.0)
            .ok_or(VaultError::ArithmeticOverflow)?,
        quote_after
            .checked_add(pool_cold.1)
            .ok_or(VaultError::ArithmeticOverflow)?,
    );
    let custody_info = resident
        .as_ref()
        .map(|(market, _)| *market)
        .unwrap_or(&tail[0]);
    let count = params.merkle_account_count;
    let wallet = count;
    let option = count.checked_add(1).ok_or(VaultError::InvalidAccountList)?;
    let quote = count.checked_add(2).ok_or(VaultError::InvalidAccountList)?;
    let book = count.checked_add(3).ok_or(VaultError::InvalidAccountList)?;
    let sponsor = count.checked_add(4).ok_or(VaultError::InvalidAccountList)?;
    let delegate = count.checked_add(5).ok_or(VaultError::InvalidAccountList)?;
    let mut inputs = Vec::with_capacity(3);
    if let Some(w) = params.book_option_input {
        inputs.push(input(w, book, aggregate_before.0, option, None));
    }
    if let Some(w) = params.book_quote_input {
        inputs.push(input(w, book, aggregate_before.1, quote, None));
    }
    if let Some(w) = params.fee_input {
        inputs.push(input(w, wallet, params.fee_input_amount, quote, None));
    }
    let mut outputs = Vec::with_capacity(6);
    if aggregate_after.0 > 0 {
        outputs.push(output(book, aggregate_after.0, option, None));
    }
    if aggregate_after.1 > 0 {
        outputs.push(output(book, aggregate_after.1, quote, None));
    }
    if option_return > 0 {
        outputs.push(output(
            wallet,
            option_return,
            option,
            order.has_compressed_delegate_consent().then_some(delegate),
        ));
    }
    let quote_net = quote_return - if fee_from_return { fee } else { 0 };
    if quote_net > 0 {
        outputs.push(output(wallet, quote_net, quote, None));
    }
    if !fee_from_return && params.fee_input_amount > fee {
        outputs.push(output(wallet, params.fee_input_amount - fee, quote, None));
    }
    if fee > 0 {
        outputs.push(output(sponsor, fee, quote, None));
    }
    let mut metas = vec![
        AccountMeta::new_readonly(*tail[3].key, false),
        AccountMeta::new(*tail[1].key, true),
        AccountMeta::new_readonly(*a[16].key, false),
        AccountMeta::new_readonly(*tail[4].key, false),
        AccountMeta::new_readonly(*tail[5].key, false),
        AccountMeta::new_readonly(*tail[6].key, false),
        AccountMeta::new_readonly(*a[20].key, false),
    ];
    metas.extend(merkle.iter().map(|info| AccountMeta::new(*info.key, false)));
    metas.extend([
        AccountMeta::new_readonly(*a[0].key, !admin_refund && !keeper),
        AccountMeta::new_readonly(*a[9].key, false),
        AccountMeta::new_readonly(*a[10].key, false),
        AccountMeta::new(*custody_info.key, true),
        AccountMeta::new(*tail[1].key, true),
        AccountMeta::new_readonly(*tail[2].key, false),
    ]);
    let ix = regular_compressed_transfer::instruction(
        *a[15].key,
        metas,
        params.output_queue_index,
        params.proof,
        &inputs,
        &outputs,
    )?;
    let mut infos = vec![
        tail[3].clone(),
        tail[1].clone(),
        a[16].clone(),
        tail[4].clone(),
        tail[5].clone(),
        tail[6].clone(),
        a[20].clone(),
    ];
    infos.extend(merkle.iter().cloned());
    infos.extend([
        a[0].clone(),
        a[9].clone(),
        a[10].clone(),
        custody_info.clone(),
        tail[1].clone(),
        tail[2].clone(),
        a[15].clone(),
    ]);
    let kind = [CustodyKind::OrderBook as u8];
    let bump = [custody.bump];
    let seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::compressed_custody::COMPRESSED_CUSTODY_SEED,
        &kind,
        a[31].key.as_ref(),
        &bump,
    ];
    let market = if resident.is_some() {
        Some(load_valid_market(program, custody_info)?)
    } else {
        None
    };
    let market_bump = market.as_ref().map(|m| [m.bump]).unwrap_or([0]);
    let market_id = market.as_ref().map(|m| m.market_id).unwrap_or([0; 32]);
    let market_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::MARKET_PDA_SEED,
        &market_id,
        &market_bump,
    ];
    let seeds = if resident.is_some() {
        market_seeds
    } else {
        seeds
    };
    if let Some(owner) = trading_owner {
        let (key, bump) = crate::trading_session::derive(program, &owner);
        if key != *a[0].key {
            return Err(VaultError::InvalidAccountList.into());
        }
        let bump = [bump];
        let trading_seeds: &[&[u8]] = &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::trading_session::SEED,
            owner.as_ref(),
            &bump,
        ];
        invoke_signed(&ix, &infos, &[seeds, trading_seeds])?;
    } else {
        invoke_signed(&ix, &infos, &[seeds])?;
    }
    custody.option_atoms = option_after;
    custody.quote_atoms = quote_after;
    if resident.is_none() {
        crate::compressed_custody::store(&tail[0], &custody)?;
    }
    state.book.orders[index].set_balance(balance);
    state
        .book
        .recompute_obligations(pool.tick_size_quote_atomic)
        .map_err(order_error)?;
    if !crate::compressed_custody::backs(
        Some(&custody),
        state.before_option,
        state.before_quote,
        state.book.header.option_obligations,
        state.book.header.quote_obligations,
    ) {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    storage::persist_with_payer_staged(
        program,
        prefix,
        &mut state.book,
        if admin_refund || keeper {
            &tail[1]
        } else {
            &a[0]
        },
    )?;
    if !commit_resident(program, &a[7], prefix, |resident| {
        merge_resident_book(program, resident, prefix, &state.book)?;
        resident.book_option = option_after;
        resident.book_quote = quote_after;
        Ok(())
    })? {
        store_state(&a[31], &state.book)?;
    }
    Ok(())
}
