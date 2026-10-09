//! Ordinary actions keep their logical Pool/page/record PDAs after residency.
//! Reads follow authenticated forwarding markers. Mutations commit correlated
//! parent totals and selected rows together; stale physical copies are not written.
use super::*;
use crate::compact_error::CompactAccountInfo;
use crate::dlmm_order_state::{
    derive_order_book, derive_order_record, DlmmOrderBook, DlmmOrderRecord,
};
use crate::market_router::{
    self, ResidentRouterState, FORWARDED_PAGE_VERSION, FORWARDED_POOL_VERSION,
};

pub(super) fn load_book_with_accounts<'info>(
    program: &Pubkey,
    info: &AccountInfo<'info>,
    pool_info: &AccountInfo<'info>,
    pool: &AmoebaDlmmPoolV1,
    accounts: &[AccountInfo<'info>],
) -> Result<DlmmOrderBook, ProgramError> {
    let external = load_exact_zero_padded_state::<DlmmOrderBook>(
        info,
        program,
        DlmmOrderBook::LEN,
        VaultError::InvalidAmoebaDlmmPool,
    )?;
    if external.header.version != market_router::FORWARDED_BOOK_VERSION {
        return orders::load_book(program, info, pool_info, pool);
    }
    let (_, state) = resident_for_pool(program, pool_info, accounts)?
        .ok_or(VaultError::InvalidAmoebaDlmmPool)?;
    let h = state.book.ok_or(VaultError::InvalidAmoebaDlmmPool)?;
    let (key, bump) = derive_order_book(program, pool_info.key);
    if *info.key != key
        || !external.header.initialized
        || external.header.bump != bump
        || external.header.discriminator != *b"DOB"
        || external.header.pool != *pool_info.key
    {
        return Err(VaultError::InvalidAmoebaDlmmPool.into());
    }
    orders::validate_book_identity(program, info, pool_info.key, pool, &h)?;
    Ok(DlmmOrderBook {
        unloaded_option: h.option_obligations,
        unloaded_quote: h.quote_obligations,
        header: h,
        orders: Vec::new(),
    })
}

pub(super) fn load_record_with_accounts<'info>(
    program: &Pubkey,
    info: &AccountInfo<'info>,
    accounts: &[AccountInfo<'info>],
) -> Result<DlmmOrderRecord, ProgramError> {
    let external = load_exact_zero_padded_state::<DlmmOrderRecord>(
        info,
        program,
        DlmmOrderRecord::LEN,
        VaultError::InvalidAmoebaDlmmPool,
    )?;
    if external.version != market_router::FORWARDED_RECORD_VERSION {
        return Ok(external);
    }
    let pool_info = accounts.get(7).ok_or(VaultError::InvalidAccountList)?;
    let (_, state) = resident_for_pool(program, pool_info, accounts)?
        .ok_or(VaultError::InvalidAmoebaDlmmPool)?;
    let record = state
        .record(external.order.sequence)
        .ok_or(VaultError::InvalidAmoebaDlmmPool)?;
    let (key, bump) = derive_order_record(program, &external.book, external.order.sequence);
    if *info.key != key
        || !external.initialized
        || external.bump != bump
        || external.discriminator != *b"DOR"
        || state.book_key(program) != external.book
        || record.book != external.book
        || record.bump != bump
    {
        return Err(VaultError::InvalidAmoebaDlmmPool.into());
    }
    Ok(*record)
}

/// Only witnessed resident records change. Unloaded FIFO obligations and rows
/// stay in place, and newly created records may remain ordinary external PDAs.
pub(super) fn merge_resident_book<'info>(
    program: &Pubkey,
    state: &mut ResidentRouterState,
    accounts: &[AccountInfo<'info>],
    book: &DlmmOrderBook,
) -> ProgramResult {
    let end = orders::witness_end(accounts)?;
    let witnesses = &accounts[orders::ORDER_SWAP_FIXED_ACCOUNTS..end];
    state.book = Some(book.header);
    state.records.retain_mut(|record| {
        let key = derive_order_record(program, &record.book, record.order.sequence).0;
        if !witnesses.iter().any(|info| *info.key == key) {
            return true;
        }
        if let Some(order) = book
            .orders
            .iter()
            .find(|order| order.sequence == record.order.sequence)
        {
            record.order = *order;
            true
        } else {
            false
        }
    });
    Ok(())
}

pub(super) fn book_ledger(
    program: &Pubkey,
    key: &Pubkey,
    pool: &AmoebaDlmmPoolV1,
    option: u64,
    quote: u64,
) -> crate::compressed_custody::CompressedCustodyV1 {
    let mut value = crate::compressed_custody::CompressedCustodyV1::new(
        crate::compressed_custody::CustodyKind::OrderBook,
        *key,
        pool.option_mint,
        pool.quote_mint,
        crate::compressed_custody::derive_compressed_custody(
            program,
            crate::compressed_custody::CustodyKind::OrderBook,
            key,
        )
        .1,
    );
    value.option_atoms = option;
    value.quote_atoms = quote;
    value
}

pub(in crate::processor) fn resident_for_pool<'a, 'info>(
    program: &Pubkey,
    pool_info: &AccountInfo<'info>,
    accounts: &'a [AccountInfo<'info>],
) -> Result<Option<(&'a AccountInfo<'info>, ResidentRouterState)>, ProgramError> {
    let forwarded: AmoebaDlmmPoolV1 = load_light_state(
        pool_info,
        program,
        &AMOEBA_DLMM_POOL_LIGHT_DISCRIMINATOR,
        VaultError::InvalidAmoebaDlmmPool,
    )?;
    if forwarded.account_version != FORWARDED_POOL_VERSION {
        return Ok(None);
    }
    let (key, bump) = derive_ameba_dlmm_pool_pda(program, &forwarded.market);
    if key != *pool_info.key
        || forwarded.bump != bump
        || !forwarded.is_initialized
        || forwarded.account_discriminator
            != crate::ameba_dlmm_state::AMOEBA_DLMM_POOL_ACCOUNT_DISCRIMINATOR
    {
        return Err(VaultError::InvalidAmoebaDlmmPool.into());
    }
    let mut matches = accounts.iter().filter(|info| *info.key == forwarded.market);
    let market = matches.next().ok_or(VaultError::InvalidAccountList)?;
    if matches.next().is_some() {
        return Err(VaultError::InvalidAccountList.into());
    }
    let state = market_router::load(program, market)?.ok_or(VaultError::InvalidAmoebaDlmmPool)?;
    if state.pool_key(program) != *pool_info.key {
        return Err(VaultError::InvalidPda.into());
    }
    validate_pool_identity(program, pool_info.key, &state.pool)?;
    Ok(Some((market, state)))
}

pub(in crate::processor) fn load_pool_with_accounts<'info>(
    program: &Pubkey,
    pool: &AccountInfo<'info>,
    accounts: &[AccountInfo<'info>],
) -> Result<AmoebaDlmmPoolV1, ProgramError> {
    if let Some((_, state)) = resident_for_pool(program, pool, accounts)? {
        Ok(state.pool)
    } else {
        load_pool(program, pool)
    }
}

pub(super) fn load_page_with_accounts<'info>(
    program: &Pubkey,
    pool: &AccountInfo<'info>,
    page_info: &AccountInfo<'info>,
    accounts: &[AccountInfo<'info>],
) -> Result<AmoebaDlmmBinPageV1, ProgramError> {
    let external: AmoebaDlmmBinPageV1 = load_light_state(
        page_info,
        program,
        &AMOEBA_DLMM_BIN_PAGE_LIGHT_DISCRIMINATOR,
        VaultError::InvalidAmoebaDlmmBinPage,
    )?;
    if external.account_version != FORWARDED_PAGE_VERSION {
        return load_bin_page(program, pool.key, page_info);
    }
    let (_, state) =
        resident_for_pool(program, pool, accounts)?.ok_or(VaultError::InvalidAmoebaDlmmPool)?;
    let page = state
        .page(external.page_index)
        .ok_or(VaultError::InvalidAmoebaDlmmBinPage)?;
    let (key, bump) = derive_ameba_dlmm_bin_page_pda(program, pool.key, page.page_index);
    if *page_info.key != key
        || external.pool != *pool.key
        || external.bump != bump
        || !external.is_initialized
        || external.account_discriminator
            != crate::ameba_dlmm_state::AMOEBA_DLMM_BIN_PAGE_ACCOUNT_DISCRIMINATOR
        || page.bump != bump
    {
        return Err(VaultError::InvalidAmoebaDlmmBinPage.into());
    }
    Ok(*page)
}

/// Read the newest complete resident payload, apply all correlated changes,
/// then validate and write once. The caller has already checked custody CPIs.
pub(in crate::processor) fn commit_resident<'info>(
    program: &Pubkey,
    pool: &AccountInfo<'info>,
    accounts: &[AccountInfo<'info>],
    apply: impl FnOnce(&mut ResidentRouterState) -> ProgramResult,
) -> Result<bool, ProgramError> {
    let Some((market, mut state)) = resident_for_pool(program, pool, accounts)? else {
        return Ok(false);
    };
    apply(&mut state)?;
    market_router::store(market, &state)?;
    Ok(true)
}

/// Older prefixes without a Market slot append it after their existing tail.
/// Keep it outside page/Light parsers while retaining it for resident commits.
pub(super) fn without_market_tail<'a, 'info>(
    program: &Pubkey,
    pool: &AccountInfo<'info>,
    accounts: &'a [AccountInfo<'info>],
) -> Result<&'a [AccountInfo<'info>], ProgramError> {
    let raw: AmoebaDlmmPoolV1 = load_light_state(
        pool,
        program,
        &AMOEBA_DLMM_POOL_LIGHT_DISCRIMINATOR,
        VaultError::InvalidAmoebaDlmmPool,
    )?;
    if raw.account_version != FORWARDED_POOL_VERSION {
        return Ok(accounts);
    }
    let Some(last) = accounts.last() else {
        return Err(VaultError::InvalidAccountList.into());
    };
    if *last.key != raw.market {
        return Err(VaultError::InvalidAccountList.into());
    }
    resident_for_pool(program, pool, accounts)?.ok_or(VaultError::InvalidAmoebaDlmmPool)?;
    Ok(&accounts[..accounts.len() - 1])
}

pub(super) fn persist_pool<'info>(
    program: &Pubkey,
    pool_info: &AccountInfo<'info>,
    accounts: &[AccountInfo<'info>],
    pool: &AmoebaDlmmPoolV1,
) -> ProgramResult {
    if !commit_resident(program, pool_info, accounts, |state| {
        state.pool = *pool;
        Ok(())
    })? {
        store_light_state(pool_info, pool)?;
    }
    Ok(())
}

pub(super) fn forwarded_pool(program: &Pubkey, info: &AccountInfo) -> Result<bool, ProgramError> {
    if info.owner != program || info.data_len() != AmoebaDlmmPoolV1::ACCOUNT_LEN {
        return Ok(false);
    }
    let data = info.try_data()?;
    Ok(
        data.get(..8) == Some(AMOEBA_DLMM_POOL_LIGHT_DISCRIMINATOR.as_slice())
            && data.get(13) == Some(&FORWARDED_POOL_VERSION),
    )
}

pub(super) fn pool_ledger(
    program: &Pubkey,
    pool_key: &Pubkey,
    pool: &AmoebaDlmmPoolV1,
    option: u64,
    quote: u64,
) -> crate::compressed_custody::CompressedCustodyV1 {
    let mut ledger = crate::compressed_custody::CompressedCustodyV1::new(
        crate::compressed_custody::CustodyKind::Pool,
        *pool_key,
        pool.option_mint,
        pool.quote_mint,
        crate::compressed_custody::derive_compressed_custody(
            program,
            crate::compressed_custody::CustodyKind::Pool,
            pool_key,
        )
        .1,
    );
    ledger.option_atoms = option;
    ledger.quote_atoms = quote;
    ledger
}

pub(super) fn with_resident_market<'info>(
    program: &Pubkey,
    pool: &AccountInfo<'info>,
    supplied: &[AccountInfo<'info>],
    mut base: Vec<AccountInfo<'info>>,
) -> Result<Vec<AccountInfo<'info>>, ProgramError> {
    if let Some((market, _)) = resident_for_pool(program, pool, supplied)? {
        if !base.iter().any(|info| info.key == market.key) {
            base.push(market.clone());
        }
    }
    Ok(base)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn resident_pool_hot_vault_amounts<'info>(
    program: &Pubkey,
    pool_info: &AccountInfo<'info>,
    pool: &AmoebaDlmmPoolV1,
    authority: &AccountInfo<'info>,
    option_vault: &AccountInfo<'info>,
    quote_vault: &AccountInfo<'info>,
    accounts: &[AccountInfo<'info>],
) -> Result<(u64, u64), ProgramError> {
    let vaults = validate_pool_vaults(
        program,
        pool_info,
        pool,
        authority,
        option_vault,
        quote_vault,
    )?;
    let cold = resident_for_pool(program, pool_info, accounts)?
        .map(|(_, state)| (state.pool_option, state.pool_quote))
        .unwrap_or((0, 0));
    if vaults
        .0
        .amount
        .checked_add(cold.0)
        .ok_or(VaultError::ArithmeticOverflow)?
        < pool.accounted_option_reserve
        || vaults
            .1
            .amount
            .checked_add(cold.1)
            .ok_or(VaultError::ArithmeticOverflow)?
            < pool.accounted_quote_reserve
    {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    // Return actual hot balances: writer CPIs must never treat cold reserves as SPL tokens.
    Ok((vaults.0.amount, vaults.1.amount))
}
