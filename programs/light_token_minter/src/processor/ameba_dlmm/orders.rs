use super::*;
mod compressed;
pub(in crate::processor) mod expired;
mod recovery;
mod storage;
pub(super) mod trading;
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

pub(in crate::processor) struct OrderSwapState {
    pub book: DlmmOrderBook,
    pub taker_sequence: Option<u64>,
    pub direct_bid_delivery: bool,
    pub post_only: bool,
    pub maximum_fills: usize,
    pub before_option: u64,
    pub before_quote: u64,
    pub book_custody: Option<crate::compressed_custody::CompressedCustodyV1>,
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
    validate_book_identity(program, info, pool_info.key, pool, &book.header)?;
    Ok(book)
}

pub(super) fn validate_book_identity(
    program: &Pubkey,
    info: &AccountInfo,
    pool_key: &Pubkey,
    pool: &AmoebaDlmmPoolV1,
    h: &DlmmOrderBookHeader,
) -> ProgramResult {
    let (key, bump) = derive_order_book(program, pool_key);
    if *info.key != key
        || info.executable
        || info.is_signer
        || !info.is_writable
        || !h.initialized
        || h.bump != bump
        || h.discriminator != *b"DOB"
        || h.version != 3
        || h.pool != *pool_key
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
    Ok(())
}

pub(super) fn custody(
    info: &AccountInfo,
    owner: &Pubkey,
    mint: &Pubkey,
) -> Result<u64, ProgramError> {
    Ok(load_canonical_light_token_account(info, owner, mint)?.amount)
}

pub(super) fn load_swap_state<'info>(
    program: &Pubkey,
    a: &[AccountInfo<'info>],
    pool: &AmoebaDlmmPoolV1,
    sidecar: Option<&AccountInfo<'info>>,
) -> Result<OrderSwapState, ProgramError> {
    if a.len() < ORDER_SWAP_FIXED_ACCOUNTS {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut book = load_book_with_accounts(program, &a[31], &a[7], pool, a)?;
    storage::load(program, a, &mut book, pool)?;
    let before_option = custody(&a[32], a[31].key, &pool.option_mint)?;
    let before_quote = custody(&a[33], a[31].key, &pool.quote_mint)?;
    let resident = resident_for_pool(program, &a[7], a)?;
    let book_custody = if let Some((_, resident)) = &resident {
        Some(book_ledger(
            program,
            a[31].key,
            pool,
            resident.book_option,
            resident.book_quote,
        ))
    } else {
        crate::compressed_custody::load(
            program,
            sidecar,
            crate::compressed_custody::CustodyKind::OrderBook,
            a[31].key,
            &pool.option_mint,
            &pool.quote_mint,
        )?
    };
    if !a[32].is_writable
        || !a[33].is_writable
        || book
            .orders
            .iter()
            .any(DlmmOrder::has_compressed_escrow_funding)
            && book_custody.is_none()
        || !crate::compressed_custody::backs(
            book_custody.as_ref(),
            before_option,
            before_quote,
            book.header.option_obligations,
            book.header.quote_obligations,
        )
    {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    Ok(OrderSwapState {
        book,
        taker_sequence: None,
        direct_bid_delivery: false,
        post_only: false,
        maximum_fills: MAX_ORDER_FILLS,
        before_option,
        before_quote,
        book_custody,
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
            .map(|index| self.book.orders[index])
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
            if order.order_side() == 0 {
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
            direction: if order.order_side() == 0 {
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

pub(in crate::processor) struct CompressedOrderEffects {
    pub book_after: DlmmOrderBook,
    pub after_option_obligations: u64,
    pub after_quote_obligations: u64,
    pub maker_input: u64,
    pub maker_output: u64,
}

/// Preview the exact FIFO order mutation before a collective Light CPI. The
/// caller settles the aggregate custody delta, then persists book_after.
pub(in crate::processor) fn preview_compressed_finish(
    state: &OrderSwapState,
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
    pool: &AmoebaDlmmPoolV1,
) -> Result<CompressedOrderEffects, ProgramError> {
    preview_finish(state, route, direction, pool, false)
}

/// Resident inventory authenticates physical funding through the Market ledger.
/// Historical classic/compressed order flags retain their original exit rights.
/// The caller passes only the canonical FIFO rows used by this route; unrelated
/// cached rows do not consume a quote or witness limit.
pub(in crate::processor) fn preview_resident_finish(
    resident: &crate::market_router::ResidentRouterState,
    state: &OrderSwapState,
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
    pool: &AmoebaDlmmPoolV1,
) -> Result<CompressedOrderEffects, ProgramError> {
    if resident.book.as_ref() != Some(&state.book.header)
        || &resident.pool != pool
        || state.book.orders.iter().any(|order| {
            resident
                .record(order.sequence)
                .is_none_or(|record| &record.order != order)
        })
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    preview_finish(state, route, direction, pool, true)
}

fn preview_finish(
    state: &OrderSwapState,
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
    pool: &AmoebaDlmmPoolV1,
    resident_funding: bool,
) -> Result<CompressedOrderEffects, ProgramError> {
    let (maker_input, maker_output) = maker_amounts(route, direction)?;
    let mut book_after = state.book.clone();
    if resident_funding {
        // Selected FIFO rows may be only part of the resident cache or Book.
        // Derive the unselected claims from the authenticated parent totals;
        // callers cannot accidentally discard them with a zero remainder.
        let mut loaded_option = 0u64;
        let mut loaded_quote = 0u64;
        for (index, order) in book_after.orders.iter().enumerate() {
            if book_after.orders[..index]
                .iter()
                .any(|prior| prior.sequence == order.sequence)
            {
                return Err(VaultError::InvalidAccountList.into());
            }
            let (option, quote) = order
                .balance(pool.tick_size_quote_atomic)
                .and_then(|balance| balance.obligations())
                .map_err(order_error)?;
            loaded_option = loaded_option
                .checked_add(option)
                .ok_or(VaultError::ArithmeticOverflow)?;
            loaded_quote = loaded_quote
                .checked_add(quote)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
        book_after.unloaded_option = book_after
            .header
            .option_obligations
            .checked_sub(loaded_option)
            .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
        book_after.unloaded_quote = book_after
            .header
            .quote_obligations
            .checked_sub(loaded_quote)
            .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
    }
    for fill in &route.order_fills {
        let order = book_after
            .orders
            .iter_mut()
            .find(|order| order.sequence == fill.sequence)
            .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
        if !resident_funding && !order.has_compressed_escrow_funding() {
            return Err(VaultError::InvalidAmoebaDlmmRoute.into());
        }
        order.set_balance(fill.balance_after);
    }
    if let Some(sequence) = state.taker_sequence {
        let order = book_after
            .orders
            .iter_mut()
            .find(|order| order.sequence == sequence)
            .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
        if !resident_funding && !order.has_compressed_escrow_funding() {
            return Err(VaultError::InvalidAmoebaDlmmRoute.into());
        }
        let mut balance = order
            .balance(pool.tick_size_quote_atomic)
            .map_err(order_error)?;
        let taker_input = route.quote.amount_in;
        let taker_output = route.quote.amount_out;
        let quantity = if order.order_side() == 0 {
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
        if order.order_side() == 0 {
            if !state.direct_bid_delivery {
                balance.claimable_option = balance
                    .claimable_option
                    .checked_add(taker_output)
                    .ok_or(VaultError::ArithmeticOverflow)?;
            }
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
        let side = order.order_side();
        book_after.header.continuation_sequence = book_after.next_match_sequence(side);
    }
    book_after
        .recompute_obligations(pool.tick_size_quote_atomic)
        .map_err(order_error)?;
    Ok(CompressedOrderEffects {
        after_option_obligations: book_after.header.option_obligations,
        after_quote_obligations: book_after.header.quote_obligations,
        book_after,
        maker_input,
        maker_output,
    })
}

pub(in crate::processor) fn process_with_authority(
    program: &Pubkey,
    a: &[AccountInfo],
    action: DlmmOrderAction,
    owner: Pubkey,
) -> ProgramResult {
    if a.first().map(|v| *v.key) != Some(crate::trading_session::derive(program, &owner).0) {
        return Err(VaultError::InvalidAccountList.into());
    }
    match action {
        DlmmOrderAction::CancelCompressedEscrow { .. }
        | DlmmOrderAction::ClaimCompressedEscrow { .. } => {
            compressed::exit_with_authority(program, a, action, Some(owner))
        }
        DlmmOrderAction::PlaceCompressedEscrow { .. }
        | DlmmOrderAction::SwapCompressedOrders { .. } => {
            trading::process_with_authority(program, a, action, Some(owner))
        }
        _ => Err(VaultError::InvalidAmoebaDlmmRoute.into()),
    }
}

/// Move maker output into the existing pool before the taker's output transfer.
/// The future Book admits only current compressed custody actions. Deployed
/// classic records retain their separate owner recovery path.
pub(in crate::processor) fn process(
    program: &Pubkey,
    a: &[AccountInfo],
    action: DlmmOrderAction,
) -> ProgramResult {
    match action {
        DlmmOrderAction::MultiOrder(action) => {
            return crate::processor::multi_order::process(program, a, action)
        }
        DlmmOrderAction::ExpireClassic {
            sequence,
            record_count,
        } => return expired::process(program, a, sequence, record_count, false),
        DlmmOrderAction::AdminRefundClassic {
            sequence,
            record_count,
        } => return expired::process(program, a, sequence, record_count, true),
        DlmmOrderAction::Cancel { .. }
        | DlmmOrderAction::Claim { .. }
        | DlmmOrderAction::Close { .. }
        | DlmmOrderAction::CloseBook => return recovery::process(program, a, action),
        DlmmOrderAction::PlaceCompressedEscrow { .. }
        | DlmmOrderAction::SwapCompressedOrders { .. }
        | DlmmOrderAction::MatchCompressedBid { .. } => {
            return trading::process(program, a, action)
        }
        DlmmOrderAction::CancelCompressedEscrow { .. }
        | DlmmOrderAction::ClaimCompressedEscrow { .. } => {
            return compressed::exit(program, a, action)
        }
        DlmmOrderAction::AdminRefundCompressedEscrow { .. } => {
            return compressed::exit(program, a, action)
        }
        DlmmOrderAction::Initialize => {}
    }
    if a.len() != ORDER_SWAP_FIXED_ACCOUNTS || !a[0].is_signer || !a[0].is_writable {
        return Err(VaultError::InvalidAccountList.into());
    }
    // Initialization has the swap's core roles and three book custody roles,
    // but it does not deliver compressed tokens or carry a delivery tail.
    validate_collective_swap_base_privileges(program, a)?;
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
    let mut pool = load_pool_with_accounts(program, &a[7], a)?;
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
    if config.admin != *a[0].key || config.paused {
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
    pool.position_count = pool
        .position_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let mut physical = book.clone();
    if resident_for_pool(program, &a[7], a)?.is_some() {
        physical.header.version = crate::market_router::FORWARDED_BOOK_VERSION;
    }
    store_state(&a[31], &physical)?;
    if !commit_resident(program, &a[7], a, |resident| {
        resident.pool = pool;
        resident.book = Some(book.header);
        Ok(())
    })? {
        store_light_state(&a[7], &pool)?;
    }
    Ok(())
}
