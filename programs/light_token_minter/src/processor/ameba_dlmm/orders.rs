use super::*;
mod storage;
use crate::ameba_dlmm_state::AMOEBA_DLMM_ACCOUNT_VERSION;
use crate::dlmm_order_math::{OrderBalance, OrderSide, MAX_ORDER_FILLS};
use crate::dlmm_order_state::{
    derive_order_book, DlmmOrder, DlmmOrderAction, DlmmOrderBook, DlmmOrderBookHeader,
    ORDER_BOOK_SEED, ORDER_POOL_VERSION,
};
use crate::writer_dlmm_quote::{PublicOrderRouteLimits, WriterDlmmRouteQuote};

pub(super) fn witness_end(accounts: &[AccountInfo]) -> Result<usize, ProgramError> {
    storage::witness_end(accounts)
}

pub(super) const ORDER_SWAP_FIXED_ACCOUNTS: usize = 34;

/// Commit the header together with every loaded or newly created owner record.
pub(super) fn persist_book(
    program: &Pubkey,
    accounts: &[AccountInfo],
    book: &mut DlmmOrderBook,
) -> ProgramResult {
    storage::persist(program, accounts, book)
}

fn admit_book_initialization(pool: &AmoebaDlmmPoolV1, now: u64) -> ProgramResult {
    if pool.account_version != AMOEBA_DLMM_ACCOUNT_VERSION
        || pool.status == AmoebaDlmmPoolStatus::Closed
        || now >= pool.expiry_ts
    {
        return Err(VaultError::InvalidAmoebaDlmmStatusTransition.into());
    }
    Ok(())
}

pub(super) struct OrderSwapState {
    pub book: DlmmOrderBook,
    pub taker_sequence: Option<u64>,
    pub post_only: bool,
    pub maximum_fills: usize,
    pub before_option: u64,
    pub before_quote: u64,
}

fn order_error(_: crate::dlmm_order_math::OrderError) -> ProgramError {
    VaultError::InvalidAmoebaDlmmRoute.into()
}

pub(super) fn load_book(
    program: &Pubkey,
    info: &AccountInfo,
    pool_info: &AccountInfo,
    pool: &AmoebaDlmmPoolV1,
) -> Result<DlmmOrderBook, ProgramError> {
    let book = load_exact_zero_padded_state::<DlmmOrderBook>(
        info,
        program,
        DlmmOrderBook::LEN,
        VaultError::InvalidAmoebaDlmmPool,
    )?;
    let (key, bump) = derive_order_book(program, pool_info.key);
    let h = &book.header;
    if *info.key != key
        || info.executable
        || info.is_signer
        || !info.is_writable
        || !h.initialized
        || h.bump != bump
        || h.discriminator != *b"DOB"
        || h.version != 3
        || h.pool != *pool_info.key
        || h.market != pool.market
        || h.option_mint != pool.option_mint
        || h.quote_mint != pool.quote_mint
        || h.expiry_ts != pool.expiry_ts
        || h.next_sequence == 0
        || h.bid_head >= h.next_sequence
        || h.ask_head >= h.next_sequence
        || h.continuation_sequence >= h.next_sequence
        || h.record_count >= h.next_sequence
        || (h.record_count == 0 && (h.bid_head != 0 || h.ask_head != 0))
        || pool.account_version != ORDER_POOL_VERSION
        || crate::pubkey_is_default(&h.rent_payer)
    {
        return Err(VaultError::InvalidAmoebaDlmmPool.into());
    }
    Ok(book)
}

fn custody(info: &AccountInfo, owner: &Pubkey, mint: &Pubkey) -> Result<u64, ProgramError> {
    Ok(load_canonical_light_token_account(info, owner, mint)?.amount)
}

pub(super) fn load_swap_state(
    program: &Pubkey,
    a: &[AccountInfo],
    pool: &AmoebaDlmmPoolV1,
) -> Result<OrderSwapState, ProgramError> {
    if a.len() < ORDER_SWAP_FIXED_ACCOUNTS {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut book = load_book(program, &a[31], &a[7], pool)?;
    storage::load(program, a, &mut book, pool)?;
    let before_option = custody(&a[32], a[31].key, &pool.option_mint)?;
    let before_quote = custody(&a[33], a[31].key, &pool.quote_mint)?;
    if !a[32].is_writable
        || !a[33].is_writable
        || before_option < book.header.option_obligations
        || before_quote < book.header.quote_obligations
    {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    Ok(OrderSwapState {
        book,
        taker_sequence: None,
        post_only: false,
        maximum_fills: MAX_ORDER_FILLS,
        before_option,
        before_quote,
    })
}

impl OrderSwapState {
    pub(super) fn makers(&self, direction: AmoebaDlmmSwapDirection) -> Vec<DlmmOrder> {
        let side = if direction == AmoebaDlmmSwapDirection::QuoteForOption {
            1
        } else {
            0
        };
        self.book
            .priority(side)
            .into_iter()
            .map(|index| self.book.orders[index].clone())
            .collect()
    }
    pub(super) fn limits(&self) -> Result<PublicOrderRouteLimits, ProgramError> {
        let maximum_option_output = if let Some(sequence) = self.taker_sequence {
            let order = self
                .book
                .orders
                .iter()
                .find(|order| order.sequence == sequence)
                .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
            if order.side == 0 {
                order.remaining_quantity
            } else {
                u64::MAX
            }
        } else {
            u64::MAX
        };
        Ok(PublicOrderRouteLimits {
            allow_partial: self.taker_sequence.is_some(),
            maximum_option_output,
            maximum_order_fills: self.maximum_fills,
        })
    }
    pub(super) fn taker_params(
        &self,
        expiry: u64,
    ) -> Result<SwapAmoebaDlmmExactInV1Params, ProgramError> {
        let order = self
            .book
            .orders
            .iter()
            .find(|order| Some(order.sequence) == self.taker_sequence)
            .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
        Ok(SwapAmoebaDlmmExactInV1Params {
            direction: if order.side == 0 {
                WireSwapDirection::QuoteForOption
            } else {
                WireSwapDirection::OptionForQuote
            },
            amount_in: order.remaining_input,
            minimum_amount_out: 0,
            limit_bin_id: order.limit_bin,
            deadline_ts: expiry,
        })
    }
}

pub(super) fn maker_amounts(
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
) -> Result<(u64, u64), ProgramError> {
    let mut options = 0u64;
    let mut quote = 0u64;
    for fill in &route.order_fills {
        options = options
            .checked_add(fill.quantity)
            .ok_or(VaultError::ArithmeticOverflow)?;
        quote = quote
            .checked_add(fill.quote)
            .ok_or(VaultError::ArithmeticOverflow)?;
    }
    Ok(if direction == AmoebaDlmmSwapDirection::QuoteForOption {
        (quote, options)
    } else {
        (options, quote)
    })
}

/// Move maker output into the existing pool before the taker's output transfer.
pub(super) fn seed_output(
    a: &[AccountInfo],
    state: &OrderSwapState,
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
) -> ProgramResult {
    let (_, amount) = maker_amounts(route, direction)?;
    if amount == 0 {
        return Ok(());
    }
    let (source, destination, mint, interface) =
        if direction == AmoebaDlmmSwapDirection::QuoteForOption {
            (&a[32], &a[11], &a[9], &a[17])
        } else {
            (&a[33], &a[12], &a[10], &a[18])
        };
    let bump = [state.book.header.bump];
    let seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        ORDER_BOOK_SEED,
        a[7].key.as_ref(),
        &bump,
    ];
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
    )
}

pub(super) fn finish(
    program: &Pubkey,
    a: &[AccountInfo],
    state: &mut OrderSwapState,
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
    pool: &AmoebaDlmmPoolV1,
) -> ProgramResult {
    let (maker_input, maker_output) = maker_amounts(route, direction)?;
    let (source, destination, mint, interface) =
        if direction == AmoebaDlmmSwapDirection::QuoteForOption {
            (&a[12], &a[33], &a[10], &a[18])
        } else {
            (&a[11], &a[32], &a[9], &a[17])
        };
    if maker_input > 0 {
        let (_, bump) = derive_ameba_dlmm_authority_pda(program, a[7].key);
        let bump_bytes = [bump];
        let seeds: &[&[u8]] = &[
            CURRENT_STATE_NAMESPACE_SEED,
            AMOEBA_DLMM_AUTHORITY_PDA_SEED,
            a[7].key.as_ref(),
            &bump_bytes,
        ];
        invoke_light_token_account_transfer_with_signer_seeds(
            maker_input,
            MarketMintAccounting::CANONICAL_DECIMALS,
            &a[15],
            &a[16],
            &a[0],
            source,
            destination,
            &a[8],
            mint,
            interface,
            &a[19],
            &a[20],
            &[seeds],
        )?;
    }
    for fill in &route.order_fills {
        let order = state
            .book
            .orders
            .iter_mut()
            .find(|order| order.sequence == fill.sequence)
            .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
        order.set_balance(fill.balance_after);
    }
    let mut taker_input = 0u64;
    let mut taker_output = 0u64;
    if let Some(sequence) = state.taker_sequence {
        let order = state
            .book
            .orders
            .iter_mut()
            .find(|order| order.sequence == sequence)
            .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
        let mut balance = order
            .balance(pool.tick_size_quote_atomic)
            .map_err(order_error)?;
        taker_input = route.quote.amount_in;
        taker_output = route.quote.amount_out;
        let quantity = if order.side == 0 {
            taker_output
        } else {
            taker_input
        };
        balance.remaining_quantity = balance
            .remaining_quantity
            .checked_sub(quantity)
            .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
        balance.remaining_input = balance
            .remaining_input
            .checked_sub(taker_input)
            .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
        if order.side == 0 {
            balance.claimable_option = balance
                .claimable_option
                .checked_add(taker_output)
                .ok_or(VaultError::ArithmeticOverflow)?;
            balance
                .release_unspendable_bid_remainder()
                .map_err(order_error)?;
        } else {
            balance.claimable_quote = balance
                .claimable_quote
                .checked_add(taker_output)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
        order.set_balance(balance);
        let side = order.side;
        state.book.header.continuation_sequence = state.book.next_match_sequence(side);
    }
    state
        .book
        .recompute_obligations(pool.tick_size_quote_atomic)
        .map_err(order_error)?;
    let after_option = custody(&a[32], a[31].key, &pool.option_mint)?;
    let after_quote = custody(&a[33], a[31].key, &pool.quote_mint)?;
    let (before_input, before_output, after_input, after_output) =
        if direction == AmoebaDlmmSwapDirection::QuoteForOption {
            (
                state.before_quote,
                state.before_option,
                after_quote,
                after_option,
            )
        } else {
            (
                state.before_option,
                state.before_quote,
                after_option,
                after_quote,
            )
        };
    if before_input
        .checked_sub(taker_input)
        .and_then(|value| value.checked_add(maker_input))
        != Some(after_input)
        || before_output
            .checked_sub(maker_output)
            .and_then(|value| value.checked_add(taker_output))
            != Some(after_output)
        || after_option < state.book.header.option_obligations
        || after_quote < state.book.header.quote_obligations
    {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    persist_book(program, a, &mut state.book)
}

/// Non-trading actions use the same 35-account prefix, without reserve pages.
/// This keeps custody identities identical for placement, fills, and claims.
pub(in crate::processor) fn process(
    program: &Pubkey,
    a: &[AccountInfo],
    action: DlmmOrderAction,
) -> ProgramResult {
    if a.len() < ORDER_SWAP_FIXED_ACCOUNTS
        || a.len()
            > ORDER_SWAP_FIXED_ACCOUNTS
                + crate::dlmm_order_state::MAX_ORDER_WITNESSES
                + usize::from(MAX_AMOEBA_DLMM_PAGE_HOPS_PER_SWAP)
        || !a[0].is_signer
        || !a[0].is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let witness_end = storage::witness_end(a)?;
    let base: Vec<_> = a[..31]
        .iter()
        .chain(a[witness_end..].iter())
        .cloned()
        .collect();
    validate_pack_dlmm_account_privileges(
        AmoebaDlmmInstructionTag::SwapCollectiveDlmmExactInV1,
        &base,
    )?;
    for index in 31..34 {
        if !a[index].is_writable
            || a[index].is_signer
            || a[index].executable
            || a.iter()
                .enumerate()
                .any(|(other, info)| other != index && info.key == a[index].key)
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
    if let DlmmOrderAction::Initialize = action {
        if a.len() != ORDER_SWAP_FIXED_ACCOUNTS || config.admin != *a[0].key || config.paused {
            return Err(VaultError::InvalidAmoebaDlmmStatusTransition.into());
        }
        admit_book_initialization(&pool, current_unix_timestamp()?)?;
        let (key, bump) = derive_order_book(program, a[7].key);
        if *a[31].key != key || !a[31].is_writable {
            return Err(VaultError::InvalidPda.into());
        }
        validate_create_only_program_account_target(program, &a[31])?;
        create_program_account(
            &a[0],
            &a[31],
            &a[20],
            program,
            DlmmOrderBook::LEN,
            &[ORDER_BOOK_SEED, a[7].key.as_ref(), &[bump]],
        )?;
        let book = DlmmOrderBook {
            header: DlmmOrderBookHeader {
                initialized: true,
                bump,
                discriminator: *b"DOB",
                version: 3,
                pool: *a[7].key,
                market: pool.market,
                option_mint: pool.option_mint,
                quote_mint: pool.quote_mint,
                rent_payer: *a[0].key,
                next_sequence: 1,
                expiry_ts: pool.expiry_ts,
                ..DlmmOrderBookHeader::default()
            },
            orders: Vec::new(),
            unloaded_option: 0,
            unloaded_quote: 0,
        };
        for (mint, vault) in [(&a[9], &a[32]), (&a[10], &a[33])] {
            load_or_create_light_associated_token_account(
                &a[0], &a[31], mint, vault, &a[15], &a[21], &a[22], &a[20],
            )?;
        }
        pool.account_version = ORDER_POOL_VERSION;
        // This retained position prevents terminal pool closure while ANY order
        // escrow or proceeds remain, including after expiry.
        pool.position_count = pool
            .position_count
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        store_state(&a[31], &book)?;
        return store_light_state(&a[7], &pool);
    }
    let mut state = load_swap_state(program, a, &pool)?;
    if matches!(
        action,
        DlmmOrderAction::Place { .. }
            | DlmmOrderAction::Match { .. }
            | DlmmOrderAction::Swap { .. }
    ) {
        storage::require_heads(&state.book)?;
    }
    // A crossing continuation is completed against the older resting head's
    // price before newly submitted same-side interest can take priority.
    if matches!(
        action,
        DlmmOrderAction::Place {
            post_only: false,
            ..
        }
    ) {
        let bids = state.book.priority(0);
        let asks = state.book.priority(1);
        if let (Some(bid), Some(ask)) = (bids.first(), asks.first()) {
            if state.book.orders[*bid].limit_bin >= state.book.orders[*ask].limit_bin {
                return Err(VaultError::InvalidAmoebaDlmmRoute.into());
            }
        }
    }
    match action {
        DlmmOrderAction::Swap { params } => {
            collective::process_collective_order_swap(program, a, params, &mut state)
        }
        DlmmOrderAction::Place {
            expected_sequence,
            side,
            limit_bin,
            quantity,
            post_only,
        } => {
            if config.paused
                || pool.status != AmoebaDlmmPoolStatus::Active
                || current_unix_timestamp()? >= pool.expiry_ts
                || expected_sequence != state.book.header.next_sequence
                || side > 1
                || quantity == 0
            {
                return Err(VaultError::InvalidAmoebaDlmmRoute.into());
            }
            let price = price_from_bin(pool.tick_size_quote_atomic, pool.maximum_bin_id, limit_bin)
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
            let (source, destination, mint, interface) = if side == 0 {
                (&a[14], &a[33], &a[10], &a[18])
            } else {
                (&a[13], &a[32], &a[9], &a[17])
            };
            let source_before =
                load_user_transfer_account(program, source, a[0].key, mint.key)?.amount;
            let destination_before = custody(destination, a[31].key, mint.key)?;
            if source_before < balance.remaining_input {
                return Err(VaultError::InvalidTokenAccount.into());
            }
            invoke_light_token_account_transfer(
                balance.remaining_input,
                MarketMintAccounting::CANONICAL_DECIMALS,
                &a[15],
                &a[16],
                &a[0],
                source,
                destination,
                &a[0],
                mint,
                interface,
                &a[19],
                &a[20],
            )?;
            if source_before.checked_sub(
                load_user_transfer_account(program, source, a[0].key, mint.key)?.amount,
            ) != Some(balance.remaining_input)
                || custody(destination, a[31].key, mint.key)?.checked_sub(destination_before)
                    != Some(balance.remaining_input)
            {
                return Err(VaultError::AmoebaDlmmInvariantViolation.into());
            }
            let mut order = DlmmOrder {
                owner: *a[0].key,
                sequence: expected_sequence,
                side,
                limit_bin,
                original_quantity: quantity,
                ..DlmmOrder::default()
            };
            order.set_balance(balance);
            storage::insert(&mut state.book, order)?;
            state.book.header.next_sequence = expected_sequence
                .checked_add(1)
                .ok_or(VaultError::ArithmeticOverflow)?;
            state
                .book
                .recompute_obligations(pool.tick_size_quote_atomic)
                .map_err(order_error)?;
            state.before_option = custody(&a[32], a[31].key, &pool.option_mint)?;
            state.before_quote = custody(&a[33], a[31].key, &pool.quote_mint)?;
            state.taker_sequence = Some(if post_only {
                expected_sequence
            } else {
                state.book.orders[*state
                    .book
                    .priority(side)
                    .first()
                    .ok_or(VaultError::InvalidAmoebaDlmmRoute)?]
                .sequence
            });
            state.post_only = post_only;
            state.book.header.continuation_sequence = state.book.next_match_sequence(side);
            let params = state.taker_params(pool.expiry_ts)?;
            collective::process_collective_order_swap(program, a, params, &mut state)
        }
        DlmmOrderAction::Match {
            side,
            maximum_fills,
        } => {
            if side > 1 || maximum_fills == 0 || usize::from(maximum_fills) > MAX_ORDER_FILLS {
                return Err(VaultError::InvalidAmoebaDlmmRoute.into());
            }
            let index = *state
                .book
                .priority(side)
                .first()
                .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
            if let Some(other_index) = state.book.priority(1 - side).first() {
                let this = &state.book.orders[index];
                let other = &state.book.orders[*other_index];
                let crossed = if side == 0 {
                    this.limit_bin >= other.limit_bin
                } else {
                    this.limit_bin <= other.limit_bin
                };
                if crossed && this.sequence < other.sequence {
                    return Err(VaultError::InvalidAmoebaDlmmRoute.into());
                }
            }
            state.taker_sequence = Some(state.book.orders[index].sequence);
            state.maximum_fills = usize::from(maximum_fills);
            let params = state.taker_params(pool.expiry_ts)?;
            collective::process_collective_order_swap(program, a, params, &mut state)
        }
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
                            load_user_transfer_account(program, destination, a[0].key, mint.key)?
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
                            || load_user_transfer_account(program, destination, a[0].key, mint.key)?
                                .amount
                                .checked_sub(before)
                                != Some(amount)
                        {
                            return Err(VaultError::AmoebaDlmmInvariantViolation.into());
                        }
                    }
                    if options > 0 {
                        super::super::scoped_settlement::authorize_collective_settlement(
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
        DlmmOrderAction::Initialize => unreachable!(),
    }
}
