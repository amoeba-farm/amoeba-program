//! A fixed header and separately reclaimable records. Queue work is bounded by
//! supplied witnesses, while the number of records has no per-market constant cap.
use super::*;
use crate::dlmm_order_state::{
    derive_order_record, DlmmOrderRecord, MAX_ORDER_WITNESSES, ORDER_RECORD_SEED,
};

pub(super) fn witness_end(a: &[AccountInfo]) -> Result<usize, ProgramError> {
    if a.len() < ORDER_SWAP_FIXED_ACCOUNTS {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut end = ORDER_SWAP_FIXED_ACCOUNTS;
    while let Some(info) = a.get(end) {
        if info.data_len() != DlmmOrderRecord::LEN
            && !(info.owner == &system_program::id() && info.data_is_empty())
        {
            break;
        }
        if end - ORDER_SWAP_FIXED_ACCOUNTS == MAX_ORDER_WITNESSES {
            return Err(VaultError::InvalidAccountList.into());
        }
        end += 1;
    }
    if a.len() - end > usize::from(MAX_AMOEBA_DLMM_PAGE_HOPS_PER_SWAP) {
        return Err(VaultError::InvalidAccountList.into());
    }
    Ok(end)
}

pub(super) fn load(
    program: &Pubkey,
    a: &[AccountInfo],
    book: &mut DlmmOrderBook,
    pool: &AmoebaDlmmPoolV1,
) -> ProgramResult {
    let end = witness_end(a)?;
    for info in &a[ORDER_SWAP_FIXED_ACCOUNTS..end] {
        if !info.is_writable
            || info.is_signer
            || info.executable
            || a.iter().filter(|other| other.key == info.key).count() != 1
        {
            return Err(VaultError::InvalidAccountList.into());
        }
        if info.owner == &system_program::id() {
            if *info.key != derive_order_record(program, a[31].key, book.header.next_sequence).0 {
                return Err(VaultError::InvalidPda.into());
            }
            continue;
        }
        let record = load_exact_zero_padded_state::<DlmmOrderRecord>(
            info,
            program,
            DlmmOrderRecord::LEN,
            VaultError::InvalidAmoebaDlmmPool,
        )?;
        let order = &record.order;
        let (key, bump) = derive_order_record(program, a[31].key, order.sequence);
        if *info.key != key
            || !record.initialized
            || record.bump != bump
            || record.discriminator != *b"DOR"
            || record.version != 3
            || record.book != *a[31].key
            || order.sequence == 0
            || order.sequence >= book.header.next_sequence
            || order.side > 1
            || order.limit_bin == 0
            || order.limit_bin > pool.maximum_bin_id
            || order.original_quantity == 0
            || order.remaining_quantity > order.original_quantity
            || crate::pubkey_is_default(&order.owner)
            || order.previous == order.sequence
            || order.next == order.sequence
            || order.previous >= book.header.next_sequence
            || order.next >= book.header.next_sequence
            || (order.side == 1 && order.remaining_input != order.remaining_quantity)
            || (order.remaining_quantity == 0 && order.remaining_input != 0)
        {
            return Err(VaultError::InvalidAmoebaDlmmPool.into());
        }
        let (option, quote) = order
            .balance(pool.tick_size_quote_atomic)
            .map_err(order_error)?
            .obligations()
            .map_err(order_error)?;
        book.unloaded_option = book
            .unloaded_option
            .checked_sub(option)
            .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
        book.unloaded_quote = book
            .unloaded_quote
            .checked_sub(quote)
            .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
        book.orders.push(record.order);
    }
    Ok(())
}

pub(super) fn require_heads(book: &DlmmOrderBook) -> ProgramResult {
    for head in [book.header.bid_head, book.header.ask_head] {
        if head != 0
            && !book
                .orders
                .iter()
                .any(|order| order.sequence == head && order.previous == 0)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    Ok(())
}

fn before(a: &DlmmOrder, b: &DlmmOrder) -> bool {
    (if a.side == 0 {
        a.limit_bin > b.limit_bin
    } else {
        a.limit_bin < b.limit_bin
    }) || (a.limit_bin == b.limit_bin && a.sequence < b.sequence)
}

pub(super) fn insert(book: &mut DlmmOrderBook, mut order: DlmmOrder) -> ProgramResult {
    let mut peers: Vec<usize> = book
        .orders
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            candidate.side == order.side
                && candidate.remaining_quantity > 0
                && candidate.remaining_input > 0
        })
        .map(|(index, _)| index)
        .collect();
    peers.sort_unstable_by(|a, b| {
        if before(&book.orders[*a], &book.orders[*b]) {
            core::cmp::Ordering::Less
        } else {
            core::cmp::Ordering::Greater
        }
    });
    let previous = peers
        .iter()
        .copied()
        .filter(|i| before(&book.orders[*i], &order))
        .next_back();
    let next = peers
        .iter()
        .copied()
        .find(|i| before(&order, &book.orders[*i]));
    let head = if order.side == 0 {
        book.header.bid_head
    } else {
        book.header.ask_head
    };
    let prev_sequence = previous.map_or(0, |i| book.orders[i].sequence);
    let next_sequence = next.map_or(0, |i| book.orders[i].sequence);
    if previous.map_or(head != next_sequence, |i| {
        book.orders[i].next != next_sequence
    }) || next.is_some_and(|i| book.orders[i].previous != prev_sequence)
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    order.previous = prev_sequence;
    order.next = next_sequence;
    if let Some(i) = previous {
        book.orders[i].next = order.sequence;
    } else if order.side == 0 {
        book.header.bid_head = order.sequence;
    } else {
        book.header.ask_head = order.sequence;
    }
    if let Some(i) = next {
        book.orders[i].previous = order.sequence;
    }
    book.header.record_count = book
        .header
        .record_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    book.orders.push(order);
    Ok(())
}

fn unlink(book: &mut DlmmOrderBook, index: usize) -> ProgramResult {
    let order = book.orders[index].clone();
    let head = if order.side == 0 {
        book.header.bid_head
    } else {
        book.header.ask_head
    };
    if order.previous == 0 && order.next == 0 && head != order.sequence {
        return Ok(());
    }
    if order.previous != 0 {
        let prev = book
            .orders
            .iter_mut()
            .find(|other| other.sequence == order.previous)
            .ok_or(VaultError::InvalidAccountList)?;
        if prev.next != order.sequence || prev.side != order.side {
            return Err(VaultError::AmoebaDlmmInvariantViolation.into());
        }
        prev.next = order.next;
    } else {
        if head != order.sequence {
            return Err(VaultError::AmoebaDlmmInvariantViolation.into());
        }
        if order.side == 0 {
            book.header.bid_head = order.next;
        } else {
            book.header.ask_head = order.next;
        }
    }
    if order.next != 0 {
        let next = book
            .orders
            .iter_mut()
            .find(|other| other.sequence == order.next)
            .ok_or(VaultError::InvalidAccountList)?;
        if next.previous != order.sequence || next.side != order.side {
            return Err(VaultError::AmoebaDlmmInvariantViolation.into());
        }
        next.previous = order.previous;
    }
    book.orders[index].previous = 0;
    book.orders[index].next = 0;
    Ok(())
}

pub(super) fn persist(
    program: &Pubkey,
    a: &[AccountInfo],
    book: &mut DlmmOrderBook,
) -> ProgramResult {
    for index in 0..book.orders.len() {
        if book.orders[index].remaining_quantity == 0 || book.orders[index].remaining_input == 0 {
            unlink(book, index)?;
        }
    }
    let end = witness_end(a)?;
    for info in &a[ORDER_SWAP_FIXED_ACCOUNTS..end] {
        let candidate = book
            .orders
            .iter()
            .find(|order| derive_order_record(program, a[31].key, order.sequence).0 == *info.key);
        if let Some(order) = candidate {
            let (_, bump) = derive_order_record(program, a[31].key, order.sequence);
            if info.owner == &system_program::id() {
                if order.owner != *a[0].key {
                    return Err(VaultError::Unauthorized.into());
                }
                validate_create_only_program_account_target(program, info)?;
                create_program_account(
                    &a[0],
                    info,
                    &a[20],
                    program,
                    DlmmOrderRecord::LEN,
                    &[
                        ORDER_RECORD_SEED,
                        a[31].key.as_ref(),
                        &order.sequence.to_le_bytes(),
                        &[bump],
                    ],
                )?;
            }
            store_state(
                info,
                &DlmmOrderRecord {
                    initialized: true,
                    bump,
                    discriminator: *b"DOR",
                    version: 3,
                    book: *a[31].key,
                    order: order.clone(),
                },
            )?;
        } else if info.owner == program {
            let old = load_exact_zero_padded_state::<DlmmOrderRecord>(
                info,
                program,
                DlmmOrderRecord::LEN,
                VaultError::InvalidAmoebaDlmmPool,
            )?;
            if old.order.owner != *a[0].key
                || old.order.remaining_input != 0
                || old.order.remaining_quantity != 0
                || old.order.claimable_option != 0
                || old.order.claimable_quote != 0
                || old.order.previous != 0
                || old.order.next != 0
            {
                return Err(VaultError::AmoebaDlmmNotEmpty.into());
            }
            book.header.record_count = book
                .header
                .record_count
                .checked_sub(1)
                .ok_or(VaultError::ArithmeticOverflow)?;
            close_program_account(program, info, &a[0])?;
        }
    }
    // Every new record must have a supplied, create-only account; no phantom liabilities.
    for order in &book.orders {
        if !a[ORDER_SWAP_FIXED_ACCOUNTS..end]
            .iter()
            .any(|info| *info.key == derive_order_record(program, a[31].key, order.sequence).0)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    store_state(&a[31], book)
}
