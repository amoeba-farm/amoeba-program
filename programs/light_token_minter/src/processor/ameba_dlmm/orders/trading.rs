//! Current compressed Book execution. Quote and FIFO rights are admitted before
//! one aggregate Transfer2; a failure rolls back funding, fills and records.
use super::*;
use crate::compact_error::CompactAccountInfo;
use crate::state::{WriterSettlementGroupStatus, WriterSleeveStatus};
use crate::{
    ameba_dlmm_instruction::{
        CompressedSwapLeafWitnessV1 as Witness, SwapCollectiveCompressedExactInV1Params as Wire,
    },
    compressed_custody::{self as custody, derive_compressed_custody, CustodyKind},
    regular_compressed_transfer::{self as transfer, HotCompression, InputLeaf, OutputLeaf},
};
use solana_program::instruction::AccountMeta;
const TAIL: usize = 11;

pub(in crate::processor::ameba_dlmm) struct SettlementContext<'a, 'info> {
    pub after_balances: core::cell::Cell<Option<(u64, u64, u64, u64)>>,
    pub prefix: &'a [AccountInfo<'info>],
    pub tail: &'a [AccountInfo<'info>],
    pub merkle: &'a [AccountInfo<'info>],
    pub wire: Wire,
    pub book_option: Option<Witness>,
    pub book_quote: Option<Witness>,
    pub funding_mode: u8,
    pub funding: u64,
    pub fee_input: Option<Witness>,
    pub fee_amount: u64,
    pub old_option_obligations: u64,
    pub old_quote_obligations: u64,
    pub placement: bool,
    pub funded_taker: bool,
    pub trading_owner: Option<Pubkey>,
}

fn valid(w: &Witness, n: usize) -> bool {
    usize::from(w.tree_index) < n && usize::from(w.queue_index) < n && w.tree_index != w.queue_index
}
fn dummy(tree: u8, queue: u8) -> Witness {
    Witness {
        leaf_index: 0,
        root_index: 0,
        prove_by_index: true,
        tree_index: tree,
        queue_index: queue,
    }
}

pub(super) fn process<'info>(
    program: &Pubkey,
    a: &[AccountInfo<'info>],
    action: DlmmOrderAction,
) -> ProgramResult {
    process_with_authority(program, a, action, None)
}
pub(super) fn process_with_authority<'info>(
    program: &Pubkey,
    a: &[AccountInfo<'info>],
    action: DlmmOrderAction,
    trading_owner: Option<Pubkey>,
) -> ProgramResult {
    let (
        mut wire,
        records,
        book_option,
        book_quote,
        funding_mode,
        fee_input,
        fee_amount,
        placement,
        sequence,
        side,
        quantity,
        post_only,
    ) = match action {
        DlmmOrderAction::PlaceCompressedEscrow {
            expected_sequence,
            side,
            limit_bin,
            quantity,
            post_only,
            record_count,
            page_count,
            merkle_account_count,
            output_tree_index,
            output_queue_index,
            funding_mode,
            user_input_amount,
            user_input_has_delegate,
            user_input,
            book_option_input,
            book_quote_input,
            pool_option_input,
            pool_quote_input,
            writer_quote_input,
            sponsor_fee_atoms,
            fee_input_mode,
            fee_input_amount,
            fee_input,
            proof,
        } => {
            if side > 1
                || quantity == 0
                || funding_mode > 1
                || fee_input_mode != 0
                || (funding_mode == 0 && user_input.is_none())
                || (funding_mode == 1
                    && (side != 0 || user_input.is_some() || user_input_has_delegate))
            {
                return Err(VaultError::InvalidAccountList.into());
            }
            (
                Wire {
                    swap: SwapAmoebaDlmmExactInV1Params {
                        direction: if side == 0 {
                            WireSwapDirection::QuoteForOption
                        } else {
                            WireSwapDirection::OptionForQuote
                        },
                        amount_in: 0,
                        minimum_amount_out: 0,
                        limit_bin_id: limit_bin,
                        deadline_ts: u64::MAX,
                    },
                    page_count,
                    merkle_account_count,
                    output_tree_index,
                    output_queue_index,
                    user_input_amount,
                    user_input_has_delegate,
                    user_input: user_input
                        .unwrap_or_else(|| dummy(output_tree_index, output_queue_index)),
                    pool_option_input,
                    pool_quote_input,
                    writer_quote_input,
                    sponsor_fee_atoms,
                    proof,
                },
                record_count,
                book_option_input,
                book_quote_input,
                funding_mode,
                fee_input,
                fee_input_amount,
                true,
                Some(expected_sequence),
                side,
                quantity,
                post_only,
            )
        }
        DlmmOrderAction::SwapCompressedOrders { params } => (
            params.params,
            params.record_count,
            params.book_option_input,
            params.book_quote_input,
            0,
            None,
            0,
            false,
            None,
            0,
            0,
            false,
        ),
        DlmmOrderAction::MatchCompressedBid { sequence, params } => (
            params.params,
            params.record_count,
            params.book_option_input,
            params.book_quote_input,
            2,
            None,
            0,
            false,
            Some(sequence),
            0,
            0,
            false,
        ),
        _ => return Err(VaultError::InvalidAmoebaDlmmRoute.into()),
    };
    let records = usize::from(records);
    let pages = usize::from(wire.page_count);
    let n = usize::from(wire.merkle_account_count);
    let end = ORDER_SWAP_FIXED_ACCOUNTS + records + pages;
    if a.len() != end + TAIL + n
        || (records == 0 && (placement || sequence.is_some()))
        || records > crate::dlmm_order_state::MAX_ORDER_WITNESSES
        || pages > usize::from(MAX_AMOEBA_DLMM_PAGE_HOPS_PER_SWAP)
        || n < 2
        || a.len() > 255
        || !a[0].is_signer
        || !a[0].is_writable
        || !valid(&dummy(wire.output_tree_index, wire.output_queue_index), n)
        || [
            book_option,
            book_quote,
            wire.pool_option_input,
            wire.pool_quote_input,
            wire.writer_quote_input,
            fee_input,
        ]
        .iter()
        .flatten()
        .any(|w| !valid(w, n))
        || (funding_mode == 0 && (!valid(&wire.user_input, n) || wire.user_input_amount == 0))
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let prefix = &a[..end];
    let tail = &a[end..end + TAIL];
    let merkle = &a[end + TAIL..];
    let fee = if a[0].key == tail[1].key { 0 } else { 10_000 };
    if wire.sponsor_fee_atoms != fee
        || funding_mode == 2
            && (fee != 0
                || wire.user_input_amount != 0
                || wire.user_input_has_delegate
                || wire.user_input != dummy(wire.output_tree_index, wire.output_queue_index))
        || (!placement && fee_input.is_some())
        || placement
            && side == 0
            && (fee_input.is_some() || fee_amount != 0 || wire.user_input_has_delegate)
        || placement
            && side == 1
            && ((fee == 0 && (fee_input.is_some() || fee_amount != 0))
                || (fee > 0 && (fee_input.is_none() || fee_amount < fee)))
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    if storage::witness_end(prefix)? != ORDER_SWAP_FIXED_ACCOUNTS + records
        || *tail[0].key != derive_compressed_custody(program, CustodyKind::OrderBook, a[31].key).0
        || *tail[7].key != derive_compressed_custody(program, CustodyKind::Pool, a[7].key).0
        || *tail[8].key != derive_compressed_custody(program, CustodyKind::WriterCash, a[27].key).0
        || [0, 7, 8]
            .iter()
            .any(|&i| !tail[i].is_writable || tail[i].is_signer || tail[i].executable)
        || !tail[1].is_signer
        || !tail[1].is_writable
        || crate::pubkey_is_default(tail[1].key)
        || (!wire.user_input_has_delegate && !crate::is_system_program(tail[2].key))
        || (wire.user_input_has_delegate
            && *tail[2].key != *a[23].key
            && !(tail[2].owner == program
                && !tail[2].executable
                && <crate::multi_order::Order as borsh::BorshDeserialize>::try_from_slice(
                    &tail[2].try_data()?,
                )
                .ok()
                .and_then(|order| {
                    order.settlement_scope(
                        program,
                        tail[2].key,
                        a[0].key,
                        a[2].key,
                        a[9].key,
                        wire.user_input_amount,
                    )
                })
                .is_some()))
        || wire.user_input_has_delegate && wire.swap.direction != WireSwapDirection::OptionForQuote
        || !crate::light_token_instruction::is_light_system_program(tail[3].key)
        || !crate::light_token_instruction::is_registered_program(tail[4].key)
        || !crate::light_token_instruction::is_compression_authority(tail[5].key)
        || !crate::light_token_instruction::is_compression_program(tail[6].key)
        || tail[2..7].iter().any(|i| i.is_writable || i.is_signer)
        || *tail[9].key
            != crate::compressed_option_settlement::retirement_owner(program, a[4].key, a[2].key)
        || tail[9].is_writable
        || tail[9].is_signer
        || funding_mode != 1 && !crate::is_system_program(tail[10].key)
        || merkle
            .iter()
            .any(|i| !i.is_writable || i.is_signer || i.executable)
        || merkle
            .iter()
            .enumerate()
            .any(|(i, v)| merkle[..i].iter().any(|p| p.key == v.key))
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let base: Vec<_> = prefix[..31]
        .iter()
        .chain(prefix[ORDER_SWAP_FIXED_ACCOUNTS + records..].iter())
        .cloned()
        .collect();
    validate_collective_swap_base_privileges(program, &base)?;
    for index in 31..34 {
        if !prefix[index].is_writable
            || prefix[index].is_signer
            || prefix[index].executable
            || prefix
                .iter()
                .enumerate()
                .any(|(j, v)| j != index && v.key == prefix[index].key)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    let (collective, book_context) =
        super::super::super::writer_sleeve::load_collective_dlmm_context_with_book(
            program, &base[4], &base[5], &base[6], &base[2], &base[3],
        )?;
    let config = super::super::swap::load_swap_config(program, &base[1])?;
    let pool = super::super::swap::load_swap_pool(program, &base[7], &base)?;
    super::super::collective::validate_collective_pool_binding(
        &config,
        &base[2],
        &base[3],
        &collective,
        &pool,
    )?;
    if collective.group_status != WriterSettlementGroupStatus::Active
        || !matches!(collective.sleeve_status, WriterSleeveStatus::Active)
        || collective.anchor_month_settled
    {
        return Err(VaultError::AmoebaDlmmMarketNotTradable.into());
    }
    let scope = crate::scoped_settlement::derive_collective_settlement_delegate(
        program, a[0].key, a[9].key,
    )
    .0;
    if *a[23].key != scope {
        return Err(VaultError::InvalidAccountList.into());
    }
    if funding_mode == 1 {
        let ata = crate::associated_token::get_associated_token_address_with_program_id(
            a[0].key,
            a[10].key,
            &spl_token_program_id(),
        );
        if *tail[10].key != ata
            || !tail[10].is_writable
            || tail[10].is_signer
            || !crate::token_instruction::check_id(tail[10].owner)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
        validate_vault_token_account(&tail[10], a[10].key, a[0].key)?;
        if validate_token_account(&tail[10])?.amount < wire.user_input_amount {
            return Err(ProgramError::InsufficientFunds);
        }
    }
    let mut state = load_swap_state(
        program,
        prefix,
        &pool,
        super::super::compressed_swap::existing_sidecar(program, &tail[0])?,
    )?;
    storage::require_heads(&state.book)?;
    let old_option = state.book.header.option_obligations;
    let old_quote = state.book.header.quote_obligations;
    let mut funding = 0;
    if placement {
        let sequence = sequence.ok_or(VaultError::InvalidAccountList)?;
        if sequence != state.book.header.next_sequence {
            return Err(VaultError::InvalidAmoebaDlmmRoute.into());
        }
        let price = price_from_bin(
            pool.tick_size_quote_atomic,
            pool.maximum_bin_id,
            wire.swap.limit_bin_id,
        )
        .map_err(math_error)?;
        let balance = OrderBalance::funded(
            if side == 0 {
                OrderSide::Bid
            } else {
                OrderSide::Ask
            },
            quantity,
            price,
        )
        .map_err(order_error)?;
        funding = balance.remaining_input;
        let required = funding
            .checked_add(if side == 0 { fee } else { 0 })
            .ok_or(VaultError::ArithmeticOverflow)?;
        if wire.user_input_amount < required
            || funding_mode == 1 && wire.user_input_amount != required
        {
            return Err(VaultError::InvalidAccountList.into());
        }
        let mut order = DlmmOrder {
            owner: *a[0].key,
            sequence,
            side: side
                | DlmmOrder::COMPRESSED_ESCROW_FUNDING
                | if side == 0 {
                    DlmmOrder::COMPRESSED_DELEGATE_CONSENT
                } else {
                    0
                },
            limit_bin: wire.swap.limit_bin_id,
            original_quantity: quantity,
            ..DlmmOrder::default()
        };
        order.set_balance(balance);
        storage::unlink_spent(&mut state.book)?;
        storage::insert(&mut state.book, order)?;
        state.book.header.next_sequence = sequence
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        state
            .book
            .recompute_obligations(pool.tick_size_quote_atomic)
            .map_err(order_error)?;
        state.taker_sequence = Some(sequence);
        state.post_only = post_only;
        wire.swap.amount_in = funding;
        wire.swap.deadline_ts = pool.expiry_ts;
    } else if let Some(sequence) = sequence {
        let order = state
            .book
            .orders
            .iter()
            .find(|o| o.sequence == sequence)
            .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
        if order.order_side() != 0
            || !order.has_compressed_escrow_funding()
            || order.remaining_quantity == 0
            || state
                .book
                .priority(0)
                .first()
                .map(|&i| state.book.orders[i].sequence)
                != Some(sequence)
        {
            return Err(VaultError::InvalidAmoebaDlmmRoute.into());
        }
        state.taker_sequence = Some(sequence);
        let derived = state.taker_params(pool.expiry_ts)?;
        if wire.swap.direction != derived.direction
            || wire.swap.amount_in != derived.amount_in
            || wire.swap.minimum_amount_out != 0
            || wire.swap.limit_bin_id != derived.limit_bin_id
            || wire.swap.deadline_ts > pool.expiry_ts
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    let resident_mode = forwarded_pool(program, &base[7])?;
    let ordinary_only = [2, 4, 6, 24, 26, 27, 28, 29]
        .iter()
        .all(|&i| (i == 2 && resident_mode) || !base[i].is_writable);
    let mut writer = if ordinary_only {
        None
    } else {
        super::super::super::writer_sleeve::dlmm::load_swap_state_with_cash(
            program,
            &base,
            &pool,
            book_context,
            super::super::compressed_swap::existing_sidecar(program, &tail[8])?,
            &collective.market,
        )?
    };
    let compressed = super::super::compressed_swap::CompressedSwapAccounts {
        after_pool: core::cell::Cell::new(None),
        trading_owner,
        base: &base,
        pool_custody: &tail[7],
        writer_cash_custody: &tail[8],
        user_delegate: &tail[2],
        retirement: &tail[9],
        light_system: &tail[3],
        registered: &tail[4],
        compression_authority: &tail[5],
        compression_program: &tail[6],
        sponsor: &tail[1],
        merkle,
    };
    let settlement = SettlementContext {
        after_balances: core::cell::Cell::new(None),
        prefix,
        tail,
        merkle,
        wire,
        book_option,
        book_quote,
        funding_mode,
        funding,
        fee_input,
        fee_amount,
        old_option_obligations: old_option,
        old_quote_obligations: old_quote,
        placement,
        funded_taker: sequence.is_some(),
        trading_owner,
    };
    let normalized: Vec<_> = base[..4]
        .iter()
        .chain(base[7..23].iter())
        .chain(base[31..].iter())
        .cloned()
        .collect();
    super::super::swap::process_collective_swap_with_orders(
        program,
        &normalized,
        wire.swap,
        &mut writer,
        &base,
        &[],
        Some((&compressed, &wire)),
        super::super::swap::LoadedSwapState {
            config,
            pool,
            market: collective.market,
            month: collective.anchor_month,
        },
        Some((&mut state, &settlement)),
    )
}

pub(in crate::processor::ameba_dlmm) fn persist(
    program: &Pubkey,
    context: &SettlementContext,
    state: &mut OrderSwapState,
) -> ProgramResult {
    storage::persist_with_payer_staged(
        program,
        context.prefix,
        &mut state.book,
        if context.placement {
            &context.tail[1]
        } else {
            &context.prefix[0]
        },
    )
}

fn leaf(w: Witness, owner: u8, amount: u64, mint: u8, delegate: Option<u8>) -> InputLeaf {
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
fn out(owner: u8, amount: u64, mint: u8, delegate: Option<u8>) -> OutputLeaf {
    OutputLeaf {
        owner,
        amount,
        mint,
        has_delegate: delegate.is_some(),
        delegate: delegate.unwrap_or(0),
    }
}
fn delta(before: u64, old: u64, new: u64) -> Result<u64, ProgramError> {
    u64::try_from(
        u128::from(before)
            .checked_add(u128::from(new))
            .and_then(|v| v.checked_sub(u128::from(old)))
            .ok_or(VaultError::AmoebaDlmmInvariantViolation)?,
    )
    .map_err(|_| VaultError::ArithmeticOverflow.into())
}

#[allow(clippy::too_many_arguments)]
pub(in crate::processor::ameba_dlmm) fn settle<'info>(
    program: &Pubkey,
    c: &SettlementContext<'_, 'info>,
    state: &OrderSwapState,
    effects: &CompressedOrderEffects,
    pool: &AmoebaDlmmPoolV1,
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
    authority_seeds: &[&[u8]],
) -> ProgramResult {
    let a = c.prefix;
    let t = c.tail;
    let p = &c.wire;
    let buy = direction == AmoebaDlmmSwapDirection::QuoteForOption;
    let book_before = state.book_custody.clone().unwrap_or_else(|| {
        custody::CompressedCustodyV1::new(
            CustodyKind::OrderBook,
            *a[31].key,
            pool.option_mint,
            pool.quote_mint,
            derive_compressed_custody(program, CustodyKind::OrderBook, a[31].key).1,
        )
    });
    let resident = resident_for_pool(program, &a[7], a)?;
    let pool_before = if let Some((_, state)) = &resident {
        Some(pool_ledger(
            program,
            a[7].key,
            pool,
            state.pool_option,
            state.pool_quote,
        ))
    } else {
        custody::load(
            program,
            super::super::compressed_swap::existing_sidecar(program, &t[7])?,
            CustodyKind::Pool,
            a[7].key,
            &pool.option_mint,
            &pool.quote_mint,
        )?
    }
    .unwrap_or_else(|| {
        custody::CompressedCustodyV1::new(
            CustodyKind::Pool,
            *a[7].key,
            pool.option_mint,
            pool.quote_mint,
            derive_compressed_custody(program, CustodyKind::Pool, a[7].key).1,
        )
    });
    let cash_before = custody::load(
        program,
        super::super::compressed_swap::existing_sidecar(program, &t[8])?,
        CustodyKind::WriterCash,
        a[27].key,
        &Pubkey::default(),
        &pool.quote_mint,
    )?
    .unwrap_or_else(|| {
        custody::CompressedCustodyV1::new(
            CustodyKind::WriterCash,
            *a[27].key,
            Pubkey::default(),
            pool.quote_mint,
            derive_compressed_custody(program, CustodyKind::WriterCash, a[27].key).1,
        )
    });
    let book_option_after = delta(
        book_before.option_atoms,
        c.old_option_obligations,
        effects.after_option_obligations,
    )?;
    let book_quote_after = delta(
        book_before.quote_atoms,
        c.old_quote_obligations,
        effects.after_quote_obligations,
    )?;
    let premium = if buy {
        route
            .writer
            .gross_premium_atoms
            .checked_add(route.writer.lp_fee_atoms)
            .ok_or(VaultError::ArithmeticOverflow)?
    } else {
        0
    };
    let retirement = if buy {
        0
    } else {
        route.writer.retired_option_atoms
    };
    let cash_after = cash_before
        .quote_atoms
        .checked_add(premium)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let user_input = if c.funding_mode == 2 {
        0
    } else {
        p.user_input_amount
    };
    let used = if c.placement {
        c.funding
    } else if c.funded_taker {
        0
    } else {
        route.quote.amount_in
    };
    let change = user_input
        .checked_sub(used)
        .and_then(|x| x.checked_sub(if buy { p.sponsor_fee_atoms } else { 0 }))
        .ok_or(VaultError::InvalidAccountList)?;
    let user_output = if c.funded_taker {
        0
    } else {
        route
            .quote
            .amount_out
            .checked_sub(if buy { 0 } else { p.sponsor_fee_atoms })
            .ok_or(VaultError::InvalidAccountList)?
    };
    if !c.funded_taker && user_output < p.swap.minimum_amount_out {
        return Err(VaultError::InvalidAmoebaDlmmRoute.into());
    }
    let fee_quote = if c.placement && !buy { c.fee_amount } else { 0 };
    let fee_change = fee_quote
        .checked_sub(if c.placement && !buy {
            p.sponsor_fee_atoms
        } else {
            0
        })
        .ok_or(VaultError::InvalidAccountList)?;
    let hot_output = route
        .quote
        .amount_out
        .saturating_sub(effects.maker_output)
        .min(
            load_pool_vault(
                program,
                a[7].key,
                a[8].key,
                &a[8],
                if buy { a[9].key } else { a[10].key },
                if buy { a[11].key } else { a[12].key },
                if buy { &a[11] } else { &a[12] },
            )?
            .amount,
        );
    let solve = |option: bool| {
        crate::compressed_swap_plan::pool_sidecar_target(
            if c.funding_mode == 0 && (option != buy) {
                user_input
            } else {
                0
            },
            if c.funding_mode == 1 && !option {
                user_input
            } else {
                0
            },
            if option == buy { hot_output } else { 0 },
            if option {
                book_before.option_atoms
            } else {
                book_before.quote_atoms
            },
            if option {
                book_option_after
            } else {
                book_quote_after
            },
            if option {
                pool_before.option_atoms
            } else {
                pool_before.quote_atoms
            },
            if !option && premium > 0 {
                cash_before.quote_atoms + fee_quote
            } else if !option {
                fee_quote
            } else {
                0
            },
            if !option && premium > 0 {
                cash_after
            } else {
                0
            },
            (if option != buy { change } else { 0 }) + if !option { fee_change } else { 0 },
            if !option { p.sponsor_fee_atoms } else { 0 },
            if option == buy { user_output } else { 0 },
            if option { retirement } else { 0 },
        )
    };
    let pool_option_after = solve(true)?;
    let pool_quote_after = solve(false)?;
    let aggregate_before = (
        pool_before
            .option_atoms
            .checked_add(book_before.option_atoms)
            .ok_or(VaultError::ArithmeticOverflow)?,
        pool_before
            .quote_atoms
            .checked_add(book_before.quote_atoms)
            .ok_or(VaultError::ArithmeticOverflow)?,
    );
    let aggregate_after = (
        pool_option_after
            .checked_add(book_option_after)
            .ok_or(VaultError::ArithmeticOverflow)?,
        pool_quote_after
            .checked_add(book_quote_after)
            .ok_or(VaultError::ArithmeticOverflow)?,
    );
    if c.book_option.is_some() != (resident.is_none() && book_before.option_atoms > 0)
        || c.book_quote.is_some() != (resident.is_none() && book_before.quote_atoms > 0)
        || p.pool_option_input.is_some()
            != (if resident.is_some() {
                aggregate_before.0
            } else {
                pool_before.option_atoms
            } > 0)
        || p.pool_quote_input.is_some()
            != (if resident.is_some() {
                aggregate_before.1
            } else {
                pool_before.quote_atoms
            } > 0)
        || p.writer_quote_input.is_some() != (premium > 0 && cash_before.quote_atoms > 0)
        || cash_before.option_atoms != 0
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut hot = Vec::new();
    let n = u8::try_from(c.merkle.len()).map_err(|_| VaultError::InvalidAccountList)?;
    let idx = |offset: u8| n.checked_add(offset).ok_or(VaultError::InvalidAccountList);
    let wallet = idx(0)?;
    let option = idx(1)?;
    let quote = idx(2)?;
    let book = idx(3)?;
    let owner_pool = idx(4)?;
    let cash = idx(5)?;
    let sponsor = idx(6)?;
    let delegate = idx(7)?;
    let scope = idx(8)?;
    let retired = idx(9)?;
    let optvault = idx(10)?;
    let qvault = idx(11)?;
    let authority = idx(12)?;
    let optinterface = idx(13)?;
    let qinterface = idx(14)?;
    let ata = idx(15)?;
    let _spl = idx(16)?;
    if c.funding_mode == 0 {
        inputs.push(leaf(
            p.user_input,
            wallet,
            user_input,
            if buy { quote } else { option },
            p.user_input_has_delegate.then_some(delegate),
        ));
    }
    for (w, o, amount, mint) in [
        (c.book_option, book, book_before.option_atoms, option),
        (c.book_quote, book, book_before.quote_atoms, quote),
        (
            p.pool_option_input,
            owner_pool,
            if resident.is_some() {
                aggregate_before.0
            } else {
                pool_before.option_atoms
            },
            option,
        ),
        (
            p.pool_quote_input,
            owner_pool,
            if resident.is_some() {
                aggregate_before.1
            } else {
                pool_before.quote_atoms
            },
            quote,
        ),
        (p.writer_quote_input, cash, cash_before.quote_atoms, quote),
        (c.fee_input, wallet, fee_quote, quote),
    ] {
        if let Some(w) = w {
            inputs.push(leaf(w, o, amount, mint, None));
        }
    }
    if inputs.len() > 7
        || inputs.iter().enumerate().any(|(i, w)| {
            inputs[..i]
                .iter()
                .any(|v| v.tree == w.tree && v.queue == w.queue && v.leaf_index == w.leaf_index)
        })
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    for (owner, amount, mint, d) in [
        (
            book,
            if resident.is_some() {
                0
            } else {
                book_option_after
            },
            option,
            None,
        ),
        (
            book,
            if resident.is_some() {
                0
            } else {
                book_quote_after
            },
            quote,
            None,
        ),
        (
            owner_pool,
            if resident.is_some() {
                aggregate_after.0
            } else {
                pool_option_after
            },
            option,
            None,
        ),
        (
            owner_pool,
            if resident.is_some() {
                aggregate_after.1
            } else {
                pool_quote_after
            },
            quote,
            None,
        ),
        (
            wallet,
            change,
            if buy { quote } else { option },
            p.user_input_has_delegate.then_some(delegate),
        ),
        (wallet, fee_change, quote, None),
        (
            wallet,
            user_output,
            if buy { option } else { quote },
            buy.then_some(scope),
        ),
        (sponsor, p.sponsor_fee_atoms, quote, None),
        (retired, retirement, option, None),
    ] {
        if amount > 0 {
            outputs.push(out(owner, amount, mint, d));
        }
    }
    if premium > 0 && cash_after > 0 {
        outputs.push(out(cash, cash_after, quote, None));
    }
    if hot_output > 0 {
        hot.push(HotCompression {
            amount: hot_output,
            mint: if buy { option } else { quote },
            source: if buy { optvault } else { qvault },
            authority,
            pool_account_index: if buy { optinterface } else { qinterface },
            pool_index: 0,
            bump: crate::light_token_instruction::get_spl_interface_pda_and_bump(if buy {
                a[9].key
            } else {
                a[10].key
            })
            .1,
            decimals: 6,
        });
    }
    if c.funding_mode == 1 {
        hot.push(HotCompression {
            amount: user_input,
            mint: quote,
            source: ata,
            authority: wallet,
            pool_account_index: qinterface,
            pool_index: 0,
            bump: crate::light_token_instruction::get_spl_interface_pda_and_bump(a[10].key).1,
            decimals: 6,
        });
    }
    let mut new_book = book_before.clone();
    new_book.option_atoms = book_option_after;
    new_book.quote_atoms = book_quote_after;
    if !custody::backs(
        Some(&new_book),
        state.before_option,
        state.before_quote,
        effects.after_option_obligations,
        effects.after_quote_obligations,
    ) {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    if resident.is_none() {
        load_or_create_custody(
            program,
            &t[1],
            &t[0],
            &a[20],
            CustodyKind::OrderBook,
            a[31].key,
            &pool.option_mint,
            &pool.quote_mint,
        )?;
    }
    if resident.is_none()
        && (pool_option_after != pool_before.option_atoms
            || pool_quote_after != pool_before.quote_atoms)
    {
        load_or_create_custody(
            program,
            &t[1],
            &t[7],
            &a[20],
            CustodyKind::Pool,
            a[7].key,
            &pool.option_mint,
            &pool.quote_mint,
        )?;
    }
    if premium > 0 {
        load_or_create_custody(
            program,
            &t[1],
            &t[8],
            &a[20],
            CustodyKind::WriterCash,
            a[27].key,
            &Pubkey::default(),
            &pool.quote_mint,
        )?;
    }
    let mut infos = vec![
        t[3].clone(),
        t[1].clone(),
        a[16].clone(),
        t[4].clone(),
        t[5].clone(),
        t[6].clone(),
        a[20].clone(),
    ];
    let mut metas = vec![
        AccountMeta::new_readonly(*t[3].key, false),
        AccountMeta::new(*t[1].key, true),
        AccountMeta::new_readonly(*a[16].key, false),
        AccountMeta::new_readonly(*t[4].key, false),
        AccountMeta::new_readonly(*t[5].key, false),
        AccountMeta::new_readonly(*t[6].key, false),
        AccountMeta::new_readonly(*a[20].key, false),
    ];
    for info in c.merkle {
        infos.push(info.clone());
        metas.push(AccountMeta::new(*info.key, false));
    }
    for (info, writable, signer) in [
        (&a[0], false, true),
        (&a[9], false, false),
        (&a[10], false, false),
        (&t[0], true, resident.is_none()),
        (
            resident
                .as_ref()
                .map(|(market, _)| *market)
                .unwrap_or(&t[7]),
            true,
            true,
        ),
        (&t[8], true, true),
        (&t[1], true, true),
        (&t[2], false, false),
        (&a[23], false, false),
        (&t[9], false, false),
        (&a[11], true, false),
        (&a[12], true, false),
        (&a[8], false, true),
        (&a[17], true, false),
        (&a[18], true, false),
        (&t[10], c.funding_mode == 1, false),
        (&a[19], false, false),
    ] {
        infos.push(info.clone());
        metas.push(AccountMeta {
            pubkey: *info.key,
            is_writable: writable,
            is_signer: signer,
        });
    }
    let ix = transfer::instruction_with_compressions(
        *a[15].key,
        metas,
        p.output_queue_index,
        p.proof,
        &inputs,
        &hot,
        &outputs,
    )?;
    infos.push(a[15].clone());
    let book_kind = [CustodyKind::OrderBook as u8];
    let pool_kind = [CustodyKind::Pool as u8];
    let cash_kind = [CustodyKind::WriterCash as u8];
    let book_bump = [book_before.bump];
    let pool_bump = [pool_before.bump];
    let cash_bump = [cash_before.bump];
    let book_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        custody::COMPRESSED_CUSTODY_SEED,
        &book_kind,
        a[31].key.as_ref(),
        &book_bump,
    ];
    let pool_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        custody::COMPRESSED_CUSTODY_SEED,
        &pool_kind,
        a[7].key.as_ref(),
        &pool_bump,
    ];
    let market = if let Some((market, _)) = &resident {
        Some(load_valid_market(program, market)?)
    } else {
        None
    };
    let market_id = market.as_ref().map(|m| m.market_id).unwrap_or([0; 32]);
    let market_bump = market.as_ref().map(|m| [m.bump]).unwrap_or([0]);
    let market_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::MARKET_PDA_SEED,
        &market_id,
        &market_bump,
    ];
    let pool_seeds = if resident.is_some() {
        market_seeds
    } else {
        pool_seeds
    };
    let cash_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        custody::COMPRESSED_CUSTODY_SEED,
        &cash_kind,
        a[27].key.as_ref(),
        &cash_bump,
    ];
    if let Some(owner) = c.trading_owner {
        let (key, bump) = crate::trading_session::derive(program, &owner);
        if key != *a[0].key {
            return Err(VaultError::InvalidAccountList.into());
        }
        let bump = [bump];
        let seeds: &[&[u8]] = &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::trading_session::SEED,
            owner.as_ref(),
            &bump,
        ];
        invoke_signed(
            &ix,
            &infos,
            &[book_seeds, pool_seeds, cash_seeds, authority_seeds, seeds],
        )?;
    } else {
        invoke_signed(
            &ix,
            &infos,
            &[book_seeds, pool_seeds, cash_seeds, authority_seeds],
        )?;
    }
    if resident.is_none() {
        custody::store(&t[0], &new_book)?;
    }
    if resident.is_none() && t[7].owner == program {
        let mut v = pool_before;
        v.option_atoms = pool_option_after;
        v.quote_atoms = pool_quote_after;
        custody::store(&t[7], &v)?;
    }
    if resident.is_some() {
        c.after_balances.set(Some((
            pool_option_after,
            pool_quote_after,
            book_option_after,
            book_quote_after,
        )));
    }
    if premium > 0 {
        let mut v = cash_before;
        v.quote_atoms = cash_after;
        custody::store(&t[8], &v)?;
    }
    Ok(())
}
