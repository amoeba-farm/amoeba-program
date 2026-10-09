//! Authoritative option-router residency in the canonical live Market account.
//!
//! Logical Pool, page, record and writer-position addresses keep their identity.
//! The regular Market allocation stores selected mutable source rows; untouched
//! rows retain their external PDA and unloaded obligations. There is no compressed
//! leaf-size limit on this regular account. Physical compressed reserves use the
//! Market PDA, with separate LP and public-order ledgers preserving claim rights.
use crate::compact_error::CompactAccountInfo;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{account_info::AccountInfo, pubkey::Pubkey};

use crate::{
    ameba_dlmm_math::{page_first_bin, refresh_local_liquidity_bits},
    ameba_dlmm_state::{derive_ameba_dlmm_pool_pda, AmoebaDlmmBinPageV1, AmoebaDlmmPoolV1},
    constants::{AMOEBA_DLMM_POOL_PDA_SEED, CURRENT_STATE_NAMESPACE_SEED, MARKET_PDA_SEED},
    dlmm_order_state::{
        derive_order_book, derive_order_record, DlmmOrderBookHeader, DlmmOrderRecord,
    },
    market_router_account,
    state::{
        derive_writer_dlmm_policy_pda, derive_writer_dlmm_position_pda, Market,
        WriterDlmmPositionV1,
    },
    ProgramError,
};

mod codec;

pub const FORWARDED_POOL_VERSION: u8 = 5;
pub const FORWARDED_BOOK_VERSION: u8 = 4;
pub const FORWARDED_RECORD_VERSION: u8 = 4;
pub const FORWARDED_PAGE_VERSION: u8 = 2;
pub const FORWARDED_WRITER_VERSION: u8 = 2;

pub const PREPARE_COMMON: usize = 25;
pub const IMPORT_COOL: u8 = 0;
pub const THAW: u8 = 1;
pub const CONSOLIDATE: u8 = 2;
pub const PREPARE_STAGING: u8 = 3;
pub const fn valid_prepare_direction(direction: u8) -> bool {
    direction <= PREPARE_STAGING
}
crate::fixed_codec::compact_borsh_struct! {
    /// Permissionless, claim-preserving import/cool, thaw, or same-owner leaf consolidation.
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Prepare {
        pub direction: u8,
        pub allocation_bytes: u32,
        pub page_count: u16,
        pub record_count: u16,
        pub include_book: bool,
        pub include_writer: bool,
        /// Hashes of canonical source roles 1,2,3,4,5,6,7,9,10,23,
        /// followed by each supplied page and record, in account order.
        pub expected_hashes: Vec<[u8;32]>,
        pub merkle_accounts: u8,
        /// Asset 0 quote / 1 option. Party 0 resident Market, 1 Pool
        /// custody and 2 Book custody. Exact amounts may use fragmented leaves.
        pub batch: crate::multi_order::Batch,
    }
}

/// Field order is the version-one resident payload wire layout. Nested states
/// use their canonical Borsh body (Pool/page Light discriminators stay external).
#[derive(Clone, Debug, PartialEq, BorshSerialize)]
pub struct ResidentRouterState {
    pub pool: AmoebaDlmmPoolV1,
    pub book: Option<DlmmOrderBookHeader>,
    pub records: Vec<DlmmOrderRecord>,
    pub bin_pages: Vec<AmoebaDlmmBinPageV1>,
    pub writer_position: Option<WriterDlmmPositionV1>,
    pub pool_option: u64,
    pub pool_quote: u64,
    pub book_option: u64,
    pub book_quote: u64,
    pub pool_hot_option: u64,
    pub pool_hot_quote: u64,
    pub book_hot_option: u64,
    pub book_hot_quote: u64,
}

fn invalid() -> ProgramError {
    ProgramError::InvalidAccountData
}

impl ResidentRouterState {
    pub fn pool_key(&self, program: &Pubkey) -> Pubkey {
        derive_ameba_dlmm_pool_pda(program, &self.pool.market).0
    }
    pub fn book_key(&self, program: &Pubkey) -> Pubkey {
        derive_order_book(program, &self.pool_key(program)).0
    }
    pub fn total_option(&self) -> Result<u64, ProgramError> {
        self.pool_option
            .checked_add(self.book_option)
            .ok_or_else(invalid)
    }
    pub fn total_quote(&self) -> Result<u64, ProgramError> {
        self.pool_quote
            .checked_add(self.book_quote)
            .ok_or_else(invalid)
    }
    pub fn pool_backing(&self) -> Result<(u64, u64), ProgramError> {
        Ok((
            self.pool_option
                .checked_add(self.pool_hot_option)
                .ok_or_else(invalid)?,
            self.pool_quote
                .checked_add(self.pool_hot_quote)
                .ok_or_else(invalid)?,
        ))
    }
    pub fn book_backing(&self) -> Result<(u64, u64), ProgramError> {
        Ok((
            self.book_option
                .checked_add(self.book_hot_option)
                .ok_or_else(invalid)?,
            self.book_quote
                .checked_add(self.book_hot_quote)
                .ok_or_else(invalid)?,
        ))
    }
    pub fn has_hot_sources(&self) -> bool {
        self.pool_hot_option != 0
            || self.pool_hot_quote != 0
            || self.book_hot_option != 0
            || self.book_hot_quote != 0
    }
    pub fn record(&self, sequence: u64) -> Option<&DlmmOrderRecord> {
        self.records.iter().find(|r| r.order.sequence == sequence)
    }
    pub fn record_mut(&mut self, sequence: u64) -> Option<&mut DlmmOrderRecord> {
        self.records
            .iter_mut()
            .find(|r| r.order.sequence == sequence)
    }
    pub fn page(&self, index: u16) -> Option<&AmoebaDlmmBinPageV1> {
        self.bin_pages.iter().find(|p| p.page_index == index)
    }
    pub fn page_mut(&mut self, index: u16) -> Option<&mut AmoebaDlmmBinPageV1> {
        self.bin_pages.iter_mut().find(|p| p.page_index == index)
    }

    /// Full identities remain authenticated even when their AccountInfos are
    /// omitted from a transaction. Cached rows may be incomplete; their reserves
    /// and obligations cannot exceed the canonical parent's complete totals.
    pub fn validate(
        &self,
        program: &Pubkey,
        key: &Pubkey,
        market: &Market,
    ) -> Result<(), ProgramError> {
        self.validate_inner(program, key, market, false, None)
    }

    /// Only for the financial executor after loading a program-owned Market.
    /// Imports and ordinary commits strictly authenticate immutable child PDA
    /// identities. The executor preserves those fields and still checks all
    /// mutable financial, linkage, bitmap and backing invariants here.
    pub(crate) fn validate_financial_route(
        &self,
        program: &Pubkey,
        key: &Pubkey,
        market: &Market,
        policy: Option<&Pubkey>,
    ) -> Result<(), ProgramError> {
        self.validate_inner(program, key, market, true, policy)
    }

    fn validate_inner(
        &self,
        program: &Pubkey,
        key: &Pubkey,
        market: &Market,
        authenticated_children: bool,
        policy: Option<&Pubkey>,
    ) -> Result<(), ProgramError> {
        let pool = &self.pool;
        let (pool_key, pool_bump) = if authenticated_children {
            let bump = [pool.bump];
            (
                Pubkey::create_program_address(
                    &[
                        CURRENT_STATE_NAMESPACE_SEED,
                        AMOEBA_DLMM_POOL_PDA_SEED,
                        key.as_ref(),
                        &bump,
                    ],
                    program,
                )
                .map_err(|_| invalid())?,
                pool.bump,
            )
        } else {
            derive_ameba_dlmm_pool_pda(program, key)
        };
        let pool_backing = self.pool_backing()?;
        let book_backing = self.book_backing()?;
        if !pool.has_current_layout()
            || pool.market != *key
            || pool.bump != pool_bump
            || market.long_contract_mint != Some(pool.option_mint)
            || pool.quote_mint != market.collateral_mint
            || pool.expiry_ts != market.instrument.expiry_ts
            || pool.tick_size_quote_atomic != market.params.tick_size
            || pool.tick_size_quote_atomic == 0
            || pool.maximum_bin_id == 0
            || pool
                .tick_size_quote_atomic
                .checked_mul(u64::from(pool.maximum_bin_id))
                != Some(pool.maximum_price_quote_atomic)
            || pool_backing.0 < pool.accounted_option_reserve
            || pool_backing.1 < pool.accounted_quote_reserve
        {
            return Err(invalid());
        }
        self.total_option()?;
        self.total_quote()?;
        let (book_key, book_bump) = if let Some(book) = self.book.as_ref() {
            if authenticated_children {
                let bump = [book.bump];
                (
                    Pubkey::create_program_address(
                        &[
                            CURRENT_STATE_NAMESPACE_SEED,
                            crate::dlmm_order_state::ORDER_BOOK_SEED,
                            pool_key.as_ref(),
                            &bump,
                        ],
                        program,
                    )
                    .map_err(|_| invalid())?,
                    book.bump,
                )
            } else {
                derive_order_book(program, &pool_key)
            }
        } else {
            (Pubkey::default(), 0)
        };
        if let Some(book) = &self.book {
            if !book.initialized
                || book.discriminator != *b"DOB"
                || book.version != 3
                || book.bump != book_bump
                || book.pool != pool_key
                || book.market != *key
                || book.option_mint != pool.option_mint
                || book.quote_mint != pool.quote_mint
                || book.expiry_ts != pool.expiry_ts
                || book.next_sequence == 0
                || book.bid_head >= book.next_sequence
                || book.ask_head >= book.next_sequence
                || book.continuation_sequence >= book.next_sequence
                || book_backing.0 < book.option_obligations
                || book_backing.1 < book.quote_obligations
            {
                return Err(invalid());
            }
        } else if !self.records.is_empty() {
            // Terminal Book donations keep their source ledger and canonical
            // custody after the last liability/header is closed. They create
            // no LP or maker claim and need no live Book header to be preserved.
            return Err(invalid());
        }
        let mut option_obligations = 0u64;
        let mut quote_obligations = 0u64;
        let mut previous = 0;
        for record in &self.records {
            let book = self.book.as_ref().ok_or_else(invalid)?;
            let order = &record.order;
            let bump = if authenticated_children {
                record.bump
            } else {
                derive_order_record(program, &book_key, order.sequence).1
            };
            if !record.initialized
                || record.discriminator != *b"DOR"
                || record.version != 3
                || record.bump != bump
                || record.book != book_key
                || order.sequence <= previous
                || order.sequence >= book.next_sequence
                || order.previous >= book.next_sequence
                || order.next >= book.next_sequence
                || order.previous == order.sequence
                || order.next == order.sequence
                || crate::pubkey_is_default(&order.owner)
                || !(order.side <= 1 || order.valid_side_flags())
                || order.limit_bin == 0
                || order.limit_bin > pool.maximum_bin_id
                || order.original_quantity == 0
                || order.remaining_quantity > order.original_quantity
                || (order.order_side() == 1 && order.remaining_input != order.remaining_quantity)
                || (order.remaining_quantity == 0 && order.remaining_input != 0)
            {
                return Err(invalid());
            }
            let (option, quote) = order
                .balance(pool.tick_size_quote_atomic)
                .and_then(|b| b.obligations())
                .map_err(|_| invalid())?;
            option_obligations = option_obligations.checked_add(option).ok_or_else(invalid)?;
            quote_obligations = quote_obligations.checked_add(quote).ok_or_else(invalid)?;
            previous = order.sequence;
        }
        if self.book.as_ref().is_some_and(|b| {
            option_obligations > b.option_obligations
                || quote_obligations > b.quote_obligations
                || self.records.len() as u64 > b.record_count
        }) {
            return Err(invalid());
        }
        let mut options = 0u64;
        let mut quote = 0u64;
        let mut previous = None;
        for page in &self.bin_pages {
            let bump = if authenticated_children {
                page.bump
            } else {
                crate::ameba_dlmm_state::derive_ameba_dlmm_bin_page_pda(
                    program,
                    &pool_key,
                    page.page_index,
                )
                .1
            };
            if !page.has_current_layout()
                || page.pool != pool_key
                || page.bump != bump
                || previous.is_some_and(|v| v >= page.page_index)
                || page.page_index >= 64
                || pool.initialized_page_bitmap & (1u64 << page.page_index) == 0
                || page_first_bin(page.page_index).map_err(|_| invalid())? != page.first_bin_id
                || refresh_local_liquidity_bits(&page.option_reserve, &page.quote_reserve)
                    != (page.bid_bitmap, page.ask_bitmap)
            {
                return Err(invalid());
            }
            for &v in &page.option_reserve {
                options = options.checked_add(v).ok_or_else(invalid)?;
            }
            for &v in &page.quote_reserve {
                quote = quote.checked_add(v).ok_or_else(invalid)?;
            }
            previous = Some(page.page_index);
        }
        if options > pool.accounted_option_reserve || quote > pool.accounted_quote_reserve {
            return Err(invalid());
        }
        if let Some(position) = &self.writer_position {
            let bump = if authenticated_children {
                position.bump
            } else {
                derive_writer_dlmm_position_pda(program, &pool_key, &position.sleeve).1
            };
            let expected_policy = policy
                .copied()
                .unwrap_or_else(|| derive_writer_dlmm_policy_pda(program, &position.sleeve).0);
            if !position.is_initialized
                || !position.has_current_layout()
                || position.bump != bump
                || position.pool != pool_key
                || position.market != *key
                || position.policy != expected_policy
                || options
                    .checked_add(position.option_inventory_atoms)
                    .is_none_or(|v| v > pool.accounted_option_reserve)
                || quote
                    .checked_add(position.allocated_quote_atoms)
                    .and_then(|v| v.checked_add(position.uncommitted_quote_atoms))
                    .is_none_or(|v| v > pool.accounted_quote_reserve)
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

/// Read live Market and typed resident state together; this never uses a cached
/// pause or expiry projection. The ordinary Market prefix remains the authority.
pub fn load(
    program: &Pubkey,
    info: &AccountInfo,
) -> Result<Option<ResidentRouterState>, ProgramError> {
    if info.owner != program || info.executable {
        return Err(invalid());
    }
    let data = info.try_data()?;
    let prefix = market_router_account::prefix(&data)?;
    let market = codec::market(prefix)?;
    let (key, bump) = Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            MARKET_PDA_SEED,
            &market.market_id,
        ],
        program,
    );
    if !market.is_initialized || *info.key != key || market.bump != bump {
        return Err(invalid());
    }
    let Some(range) = market_router_account::payload_range(&data)? else {
        return Ok(None);
    };
    let state = ResidentRouterState::try_from_slice(&data[range]).map_err(|_| invalid())?;
    state.validate(program, info.key, &market)?;
    Ok(Some(state))
}

pub fn store(info: &AccountInfo, state: &ResidentRouterState) -> Result<(), ProgramError> {
    if !info.is_writable || info.executable {
        return Err(invalid());
    }
    let data = info.try_data()?;
    let market = codec::market(market_router_account::prefix(&data)?)?;
    state.validate(info.owner, info.key, &market)?;
    drop(data);
    let payload = borsh::to_vec(state).map_err(|_| invalid())?;
    market_router_account::write_payload(&mut info.try_data_mut()?, &payload)
}
