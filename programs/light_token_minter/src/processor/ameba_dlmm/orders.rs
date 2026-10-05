use super::*;
mod compressed;
pub(in crate::processor) mod expired;
mod recovery;
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
const COMPRESSED_ORDER_EXITS_READY: bool = false;

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
    sidecar: Option<&AccountInfo>,
) -> Result<OrderSwapState, ProgramError> {
    if a.len() < ORDER_SWAP_FIXED_ACCOUNTS {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut book = load_book(program, &a[31], &a[7], pool)?;
    storage::load(program, a, &mut book, pool)?;
    let before_option = custody(&a[32], a[31].key, &pool.option_mint)?;
    let before_quote = custody(&a[33], a[31].key, &pool.quote_mint)?;
    let book_custody = crate::compressed_custody::load(
        program,
        sidecar,
        crate::compressed_custody::CustodyKind::OrderBook,
        a[31].key,
        &pool.option_mint,
        &pool.quote_mint,
    )?;
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

pub(super) struct CompressedOrderEffects {
    pub book_after: DlmmOrderBook,
    pub before_option_obligations: u64,
    pub before_quote_obligations: u64,
    pub after_option_obligations: u64,
    pub after_quote_obligations: u64,
    pub maker_input: u64,
    pub maker_output: u64,
    pub taker_input: u64,
    pub taker_output: u64,
    pub direct_bid_option_atoms: u64,
}

/// Preview the exact FIFO order mutation before a collective Light CPI. The
/// caller settles the aggregate custody delta, then persists book_after.
pub(super) fn preview_compressed_finish(
    state: &OrderSwapState,
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
    pool: &AmoebaDlmmPoolV1,
) -> Result<CompressedOrderEffects, ProgramError> {
    let (maker_input, maker_output) = maker_amounts(route, direction)?;
    let mut book_after = state.book.clone();
    for fill in &route.order_fills {
        let order = book_after
            .orders
            .iter_mut()
            .find(|order| order.sequence == fill.sequence)
            .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
        if !order.has_compressed_escrow_funding() {
            return Err(VaultError::InvalidAmoebaDlmmRoute.into());
        }
        order.set_balance(fill.balance_after);
    }
    let mut taker_input = 0u64;
    let mut taker_output = 0u64;
    let mut direct_bid_option_atoms = 0u64;
    if let Some(sequence) = state.taker_sequence {
        let order = book_after
            .orders
            .iter_mut()
            .find(|order| order.sequence == sequence)
            .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
        if !order.has_compressed_escrow_funding() {
            return Err(VaultError::InvalidAmoebaDlmmRoute.into());
        }
        let mut balance = order
            .balance(pool.tick_size_quote_atomic)
            .map_err(order_error)?;
        taker_input = route.quote.amount_in;
        taker_output = route.quote.amount_out;
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
            if state.direct_bid_delivery {
                direct_bid_option_atoms = taker_output;
            } else {
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
        before_option_obligations: state.book.header.option_obligations,
        before_quote_obligations: state.book.header.quote_obligations,
        after_option_obligations: book_after.header.option_obligations,
        after_quote_obligations: book_after.header.quote_obligations,
        book_after,
        maker_input,
        maker_output,
        taker_input,
        taker_output,
        direct_bid_option_atoms,
    })
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
        DlmmOrderAction::ExpireClassic {
            sequence,
            record_count,
        } => return expired::process(program, a, sequence, record_count),
        DlmmOrderAction::Cancel { .. }
        | DlmmOrderAction::Claim { .. }
        | DlmmOrderAction::Close { .. }
        | DlmmOrderAction::CloseBook => return recovery::process(program, a, action),
        DlmmOrderAction::PlaceCompressedEscrow { .. } => {
            return compressed::place(program, a, action)
        }
        DlmmOrderAction::CancelCompressedEscrow { .. }
        | DlmmOrderAction::ClaimCompressedEscrow { .. } => {
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
    store_state(&a[31], &book)?;
    store_light_state(&a[7], &pool)
}
