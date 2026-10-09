//! Canonical resident projections used by the atomic options executor.
use super::*;
use crate::processor::ameba_dlmm::orders::{
    preview_resident_finish, CompressedOrderEffects, OrderSwapState,
};
use crate::{
    ameba_dlmm_math::{
        bin_to_page, page_first_bin, AmoebaDlmmBinLiquidity, AmoebaDlmmSwapDirection,
    },
    ameba_dlmm_state::AMOEBA_DLMM_EMPTY_BIN_ID,
    dlmm_order_state::{DlmmOrder, DlmmOrderBook, MAX_ORDER_WITNESSES},
    market_router::ResidentRouterState,
    writer_dlmm_quote::WriterDlmmRouteQuote,
};

pub(super) fn lp_sources(
    state: &ResidentRouterState,
    buy: bool,
) -> Result<(Vec<AmoebaDlmmBinLiquidity>, Option<u16>), ProgramError> {
    let mut bitmap = if buy {
        state.pool.ask_page_bitmap
    } else {
        state.pool.bid_page_bitmap
    };
    let mut bins = Vec::new();
    while bitmap != 0 {
        let index = if buy {
            bitmap.trailing_zeros()
        } else {
            63 - bitmap.leading_zeros()
        } as u16;
        let Some(page) = state.page(index) else {
            let first = page_first_bin(index).map_err(|_| invalid())?;
            let boundary = if bins.is_empty() {
                if buy {
                    state.pool.best_ask_bin_id
                } else {
                    state.pool.best_bid_bin_id
                }
            } else if buy {
                first
            } else {
                first.saturating_add(31).min(state.pool.maximum_bin_id)
            };
            return Ok((
                bins,
                (boundary != AMOEBA_DLMM_EMPTY_BIN_ID).then_some(boundary),
            ));
        };
        let mut local_bitmap = if buy {
            page.ask_bitmap
        } else {
            page.bid_bitmap
        };
        while local_bitmap != 0 {
            let local = if buy {
                local_bitmap.trailing_zeros()
            } else {
                31 - local_bitmap.leading_zeros()
            } as usize;
            bins.push(AmoebaDlmmBinLiquidity {
                bin_id: page.first_bin_id + local as u16,
                option_reserve: page.option_reserve[local],
                quote_reserve: page.quote_reserve[local],
            });
            local_bitmap &= !(1u32 << local);
        }
        bitmap &= !(1u64 << index);
    }
    Ok((bins, None))
}

/// The quote window is the existing bounded canonical queue prefix. Rebuilding
/// it after a chunk permits any number of cached records to participate without
/// changing FIFO identity or requiring all records to be instruction accounts.
// Keep this fixed authenticated result tuple explicit.
#[allow(clippy::type_complexity)]
pub(super) fn order_sources(
    state: &ResidentRouterState,
    buy: bool,
) -> Result<(Option<OrderSwapState>, Vec<DlmmOrder>, Option<u16>), ProgramError> {
    let Some(header) = state.book.as_ref() else {
        return Ok((None, vec![], None));
    };
    let mut sequence = if buy {
        header.ask_head
    } else {
        header.bid_head
    };
    let mut rows: Vec<DlmmOrder> = Vec::new();
    let mut seen = Vec::new();
    let mut boundary = None;
    while sequence != 0 {
        if seen.contains(&sequence) {
            return Err(invalid());
        }
        seen.push(sequence);
        let Some(record) = state.record(sequence) else {
            // Without even the current head, an unseen better FIFO offer may
            // precede every other source. Import that canonical row first.
            if rows.is_empty() {
                return Err(VaultError::InvalidAmoebaDlmmRoute.into());
            }
            boundary = rows.last().map(|r| r.limit_bin);
            break;
        };
        if record.order.order_side() != u8::from(buy) {
            return Err(invalid());
        }
        if record.order.remaining_quantity != 0 && record.order.remaining_input != 0 {
            if rows.len() == MAX_ORDER_WITNESSES {
                boundary = Some(record.order.limit_bin);
                break;
            }
            rows.push(record.order);
        }
        sequence = record.order.next;
    }
    let mut book = DlmmOrderBook {
        header: *header,
        orders: rows.clone(),
        unloaded_option: header.option_obligations,
        unloaded_quote: header.quote_obligations,
    };
    for row in &book.orders {
        let (option, quote) = row
            .balance(state.pool.tick_size_quote_atomic)
            .and_then(|v| v.obligations())
            .map_err(|_| invalid())?;
        book.unloaded_option = book
            .unloaded_option
            .checked_sub(option)
            .ok_or_else(invalid)?;
        book.unloaded_quote = book.unloaded_quote.checked_sub(quote).ok_or_else(invalid)?;
    }
    Ok((
        Some(OrderSwapState {
            book,
            taker_sequence: None,
            direct_bid_delivery: false,
            post_only: false,
            maximum_fills: crate::dlmm_order_math::MAX_ORDER_FILLS,
            before_option: 0,
            before_quote: 0,
            book_custody: None,
        }),
        rows,
        boundary,
    ))
}

fn delta(balance: u64, before: u64, after: u64) -> Result<u64, ProgramError> {
    if after >= before {
        checked(balance.checked_add(after - before))
    } else {
        checked(balance.checked_sub(before - after))
    }
}
fn best(state: &ResidentRouterState, buy: bool) -> Result<u16, ProgramError> {
    let bitmap = if buy {
        state.pool.ask_page_bitmap
    } else {
        state.pool.bid_page_bitmap
    };
    if bitmap == 0 {
        return Ok(AMOEBA_DLMM_EMPTY_BIN_ID);
    }
    let index = if buy {
        bitmap.trailing_zeros()
    } else {
        63 - bitmap.leading_zeros()
    } as u16;
    if let Some(page) = state.page(index) {
        let local = if buy {
            page.ask_bitmap.trailing_zeros()
        } else {
            31 - page.bid_bitmap.leading_zeros()
        };
        if local >= 32 {
            return Err(invalid());
        }
        return Ok(page.first_bin_id + local as u16);
    }
    let previous = if buy {
        state.pool.best_ask_bin_id
    } else {
        state.pool.best_bid_bin_id
    };
    if previous != AMOEBA_DLMM_EMPTY_BIN_ID
        && bin_to_page(previous).map_err(|_| invalid())?.0 == index
    {
        Ok(previous)
    } else {
        Err(VaultError::InvalidAmoebaDlmmRoute.into())
    }
}

/// Apply the ordinary router's existing LP/FIFO reserve equations. Writer
/// projection runs afterward, before the caller commits the correlated custody
/// ledger delta, so its source sweeps/retirement are included exactly once.
pub(super) fn project_normal(
    state: &mut ResidentRouterState,
    orders: Option<&OrderSwapState>,
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
    slot: u64,
) -> Result<Option<CompressedOrderEffects>, ProgramError> {
    let buy = direction == AmoebaDlmmSwapDirection::QuoteForOption;
    let effects = orders
        .map(|orders| preview_resident_finish(state, orders, route, direction, &state.pool))
        .transpose()?;
    for fill in &route.ordinary_fills {
        let (page_index, local) = bin_to_page(fill.bin_id).map_err(|_| invalid())?;
        let page = state.page_mut(page_index).ok_or_else(invalid)?;
        page.option_reserve[usize::from(local)] = fill.option_reserve_after;
        page.quote_reserve[usize::from(local)] = fill.quote_reserve_after;
        let bit = 1u32 << local;
        page.ask_bitmap = (page.ask_bitmap & !bit)
            | if fill.option_reserve_after != 0 {
                bit
            } else {
                0
            };
        page.bid_bitmap = (page.bid_bitmap & !bit)
            | if fill.quote_reserve_after != 0 {
                bit
            } else {
                0
            };
        page.last_updated_slot = slot;
        let ask = page.ask_bitmap != 0;
        let bid = page.bid_bitmap != 0;
        let bit = 1u64 << page_index;
        state.pool.ask_page_bitmap =
            (state.pool.ask_page_bitmap & !bit) | if ask { bit } else { 0 };
        state.pool.bid_page_bitmap =
            (state.pool.bid_page_bitmap & !bit) | if bid { bit } else { 0 };
    }
    let maker_input = effects.as_ref().map_or(0, |e| e.maker_input);
    let maker_output = effects.as_ref().map_or(0, |e| e.maker_output);
    let (input, output) = if buy {
        (
            &mut state.pool.accounted_quote_reserve,
            &mut state.pool.accounted_option_reserve,
        )
    } else {
        (
            &mut state.pool.accounted_option_reserve,
            &mut state.pool.accounted_quote_reserve,
        )
    };
    *input = checked(
        input
            .checked_add(
                route
                    .quote
                    .amount_in
                    .checked_sub(route.quote.protocol_fee)
                    .ok_or_else(invalid)?,
            )
            .and_then(|v| v.checked_sub(maker_input)),
    )?;
    *output = checked(
        output
            .checked_add(maker_output)
            .and_then(|v| v.checked_sub(route.quote.amount_out)),
    )?;
    state.pool.best_ask_bin_id = best(state, true)?;
    state.pool.best_bid_bin_id = best(state, false)?;
    state.pool.last_trade_bin_id = route.quote.last_bin_id;
    state.pool.last_updated_slot = slot;
    Ok(effects)
}
pub(super) fn commit_normal(
    state: &mut ResidentRouterState,
    before_pool_option: u64,
    before_pool_quote: u64,
    effects: Option<CompressedOrderEffects>,
    buy: bool,
) -> ProgramResult {
    state.pool_option = delta(
        state.pool_option,
        before_pool_option,
        state.pool.accounted_option_reserve,
    )?;
    state.pool_quote = delta(
        state.pool_quote,
        before_pool_quote,
        state.pool.accounted_quote_reserve,
    )?;
    if let Some(e) = effects {
        if buy {
            state.book_quote = checked(state.book_quote.checked_add(e.maker_input))?;
            state.book_option = checked(state.book_option.checked_sub(e.maker_output))?;
        } else {
            state.book_option = checked(state.book_option.checked_add(e.maker_input))?;
            state.book_quote = checked(state.book_quote.checked_sub(e.maker_output))?;
        }
        state.book = Some(e.book_after.header);
        for order in e.book_after.orders {
            let sequence = order.sequence;
            state.record_mut(sequence).ok_or_else(invalid)?.order = order;
        }
    }
    Ok(())
}
