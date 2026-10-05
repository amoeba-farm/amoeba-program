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
    let (cancel, params) = match action {
        DlmmOrderAction::CancelCompressedEscrow { params } => (true, params),
        DlmmOrderAction::ClaimCompressedEscrow { params } => (false, params),
        _ => return Err(VaultError::InvalidAmoebaDlmmRoute.into()),
    };
    let fee = if a
        .get(ORDER_SWAP_FIXED_ACCOUNTS + usize::from(params.record_count) + 1)
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
        || !a[0].is_signer
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
        || *tail[1].key == Pubkey::default()
        || !tail[1].is_signer
        || !tail[1].is_writable
        || !(*tail[2].key == system_program::id() || *tail[2].key == *a[23].key)
        || tail[2].is_signer
        || tail[2].is_writable
        || *tail[3].key != Pubkey::new_from_array(light_sdk::constants::LIGHT_SYSTEM_PROGRAM_ID)
        || *tail[4].key != Pubkey::new_from_array(light_sdk::constants::REGISTERED_PROGRAM_PDA)
        || *tail[5].key
            != Pubkey::new_from_array(light_sdk::constants::ACCOUNT_COMPRESSION_AUTHORITY_PDA)
        || *tail[6].key
            != Pubkey::new_from_array(light_sdk::constants::ACCOUNT_COMPRESSION_PROGRAM_ID)
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
    let base: Vec<_> = prefix[..31].iter().cloned().collect();
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
    let pool = load_pool(program, &a[7])?;
    let config = load_canonical_vault_config(program, &a[1])?;
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
        || params.book_option_input.is_some() != (custody.option_atoms > 0)
        || params.book_quote_input.is_some() != (custody.quote_atoms > 0)
    {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    let option_after = custody.option_atoms - option_return;
    let quote_after = custody.quote_atoms - quote_return;
    let count = params.merkle_account_count;
    let wallet = count;
    let option = count.checked_add(1).ok_or(VaultError::InvalidAccountList)?;
    let quote = count.checked_add(2).ok_or(VaultError::InvalidAccountList)?;
    let book = count.checked_add(3).ok_or(VaultError::InvalidAccountList)?;
    let sponsor = count.checked_add(4).ok_or(VaultError::InvalidAccountList)?;
    let delegate = count.checked_add(5).ok_or(VaultError::InvalidAccountList)?;
    let mut inputs = Vec::with_capacity(3);
    if let Some(w) = params.book_option_input {
        inputs.push(input(w, book, custody.option_atoms, option, None));
    }
    if let Some(w) = params.book_quote_input {
        inputs.push(input(w, book, custody.quote_atoms, quote, None));
    }
    if let Some(w) = params.fee_input {
        inputs.push(input(w, wallet, params.fee_input_amount, quote, None));
    }
    let mut outputs = Vec::with_capacity(6);
    if option_after > 0 {
        outputs.push(output(book, option_after, option, None));
    }
    if quote_after > 0 {
        outputs.push(output(book, quote_after, quote, None));
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
        AccountMeta::new_readonly(*a[0].key, true),
        AccountMeta::new_readonly(*a[9].key, false),
        AccountMeta::new_readonly(*a[10].key, false),
        AccountMeta::new(*tail[0].key, true),
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
        tail[0].clone(),
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
    crate::compressed_custody::store(&tail[0], &custody)?;
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
    persist_book(program, prefix, &mut state.book)
}
