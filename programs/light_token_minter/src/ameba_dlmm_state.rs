//! Typed, namespaced state for the in-program Amoeba DLMM.

use borsh::{BorshDeserialize, BorshSerialize};
use light_sdk_types::interface::account::compression_info::{CompressionInfo, CompressionState};
use solana_program::pubkey::Pubkey;

use crate::constants::{
    AMOEBA_DLMM_AUTHORITY_PDA_SEED, AMOEBA_DLMM_BIN_PAGE_PDA_SEED, AMOEBA_DLMM_POOL_PDA_SEED,
    AMOEBA_DLMM_POSITION_PDA_SEED, AMOEBA_DLMM_SHARE_PAGE_PDA_SEED, AMOEBA_DLMM_VAULT_PDA_SEED,
    CURRENT_STATE_NAMESPACE_SEED,
};
use crate::fixed_codec::{
    fixed_state_deserialize, invalid_fixed_borsh, FixedCursor, FixedField, FixedStateDecode,
    FixedStateEncode, FixedWriter,
};

pub const AMOEBA_DLMM_POOL_LIGHT_DISCRIMINATOR: [u8; 8] = *b"ADPOOLV1";
pub const AMOEBA_DLMM_BIN_PAGE_LIGHT_DISCRIMINATOR: [u8; 8] = *b"ADPAGEV1";
pub const AMOEBA_DLMM_SHARE_PAGE_LIGHT_DISCRIMINATOR: [u8; 8] = *b"ADSHARE1";
pub const AMOEBA_DLMM_POSITION_LIGHT_DISCRIMINATOR: [u8; 8] = *b"ADPOSIV1";

pub const AMOEBA_DLMM_POOL_ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"ADP";
pub const AMOEBA_DLMM_BIN_PAGE_ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"ABP";
pub const AMOEBA_DLMM_SHARE_PAGE_ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"ASP";
pub const AMOEBA_DLMM_POSITION_ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"ALP";
pub const AMOEBA_DLMM_ACCOUNT_VERSION: u8 = 1;
pub const AMOEBA_DLMM_EMPTY_BIN_ID: u16 = 0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
#[repr(u8)]
pub enum AmoebaDlmmPoolStatus {
    #[default]
    Pending = 0,
    Active = 1,
    Paused = 2,
    Settled = 3,
    Closed = 4,
}

impl AmoebaDlmmPoolStatus {
    pub fn allows_liquidity_add(self) -> bool {
        matches!(self, Self::Pending | Self::Active | Self::Paused)
    }

    pub fn allows_liquidity_remove(self) -> bool {
        matches!(
            self,
            Self::Pending | Self::Active | Self::Paused | Self::Settled
        )
    }

    pub fn allows_protocol_fee_collection(self) -> bool {
        matches!(self, Self::Paused | Self::Settled)
    }

    pub fn can_admin_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Pending, Self::Active)
                | (Self::Pending, Self::Paused)
                | (Self::Active, Self::Paused)
                | (Self::Paused, Self::Active)
        )
    }
}

/// One `u64` covers 64 logical pages. A pool's immutable `maximum_bin_id`
/// bounds its real addressable grid; the current 12-USDC pools use only pages
/// 0 through 7 and create them lazily.
#[derive(Clone, Debug, PartialEq)]
pub struct AmoebaDlmmPoolV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub market: Pubkey,
    pub oracle_month: Pubkey,
    pub liquidity_manager: Pubkey,
    pub option_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub option_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub expiry_ts: u64,
    pub tick_size_quote_atomic: u64,
    pub maximum_price_quote_atomic: u64,
    pub maximum_bin_id: u16,
    pub best_bid_bin_id: u16,
    pub best_ask_bin_id: u16,
    pub last_trade_bin_id: u16,
    pub initialized_page_bitmap: u64,
    pub bid_page_bitmap: u64,
    pub ask_page_bitmap: u64,
    pub swap_fee_bps: u16,
    pub protocol_fee_share_bps: u16,
    pub maximum_bins_per_swap: u8,
    pub accounted_option_reserve: u64,
    pub accounted_quote_reserve: u64,
    pub protocol_fee_option: u64,
    pub protocol_fee_quote: u64,
    pub position_count: u32,
    pub status: AmoebaDlmmPoolStatus,
    pub settlement_price_atomic: u64,
    pub settled_slot: u64,
    pub last_updated_slot: u64,
    pub compression_info: CompressionInfo,
}

impl Default for AmoebaDlmmPoolV1 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: AMOEBA_DLMM_POOL_ACCOUNT_DISCRIMINATOR,
            account_version: AMOEBA_DLMM_ACCOUNT_VERSION,
            market: Pubkey::default(),
            oracle_month: Pubkey::default(),
            liquidity_manager: Pubkey::default(),
            option_mint: Pubkey::default(),
            quote_mint: Pubkey::default(),
            option_vault: Pubkey::default(),
            quote_vault: Pubkey::default(),
            expiry_ts: 0,
            tick_size_quote_atomic: 0,
            maximum_price_quote_atomic: 0,
            maximum_bin_id: 0,
            best_bid_bin_id: AMOEBA_DLMM_EMPTY_BIN_ID,
            best_ask_bin_id: AMOEBA_DLMM_EMPTY_BIN_ID,
            last_trade_bin_id: AMOEBA_DLMM_EMPTY_BIN_ID,
            initialized_page_bitmap: 0,
            bid_page_bitmap: 0,
            ask_page_bitmap: 0,
            swap_fee_bps: 0,
            protocol_fee_share_bps: 0,
            maximum_bins_per_swap: 0,
            accounted_option_reserve: 0,
            accounted_quote_reserve: 0,
            protocol_fee_option: 0,
            protocol_fee_quote: 0,
            position_count: 0,
            status: AmoebaDlmmPoolStatus::Pending,
            settlement_price_atomic: 0,
            settled_slot: 0,
            last_updated_slot: 0,
            compression_info: CompressionInfo::default(),
        }
    }
}

impl AmoebaDlmmPoolV1 {
    pub const LEN: usize = 384;
    pub const BODY_LEN: usize = Self::LEN - 8;

    pub fn has_current_layout(&self) -> bool {
        self.is_initialized
            && self.account_discriminator == AMOEBA_DLMM_POOL_ACCOUNT_DISCRIMINATOR
            && self.account_version == AMOEBA_DLMM_ACCOUNT_VERSION
            && self.compression_info.state == CompressionState::Decompressed
    }
}

/// Reserve-only hot path page.  Shares are split into the companion page below
/// because the literal combined layout is larger than Light 0.23's 800-byte cap.
#[derive(Clone, Debug, PartialEq)]
pub struct AmoebaDlmmBinPageV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub pool: Pubkey,
    pub page_index: u16,
    pub first_bin_id: u16,
    pub bid_bitmap: u32,
    pub ask_bitmap: u32,
    pub option_reserve: [u64; 32],
    pub quote_reserve: [u64; 32],
    pub last_updated_slot: u64,
    pub compression_info: CompressionInfo,
}

impl Default for AmoebaDlmmBinPageV1 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: AMOEBA_DLMM_BIN_PAGE_ACCOUNT_DISCRIMINATOR,
            account_version: AMOEBA_DLMM_ACCOUNT_VERSION,
            pool: Pubkey::default(),
            page_index: 0,
            first_bin_id: 0,
            bid_bitmap: 0,
            ask_bitmap: 0,
            option_reserve: [0; 32],
            quote_reserve: [0; 32],
            last_updated_slot: 0,
            compression_info: CompressionInfo::default(),
        }
    }
}

impl AmoebaDlmmBinPageV1 {
    pub const LEN: usize = 602;
    pub const BODY_LEN: usize = Self::LEN - 8;

    pub fn has_current_layout(&self) -> bool {
        self.is_initialized
            && self.account_discriminator == AMOEBA_DLMM_BIN_PAGE_ACCOUNT_DISCRIMINATOR
            && self.account_version == AMOEBA_DLMM_ACCOUNT_VERSION
            && self.compression_info.state == CompressionState::Decompressed
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AmoebaDlmmSharePageV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub pool: Pubkey,
    pub page_index: u16,
    pub first_bin_id: u16,
    pub total_liquidity_shares: [u128; 32],
    pub last_updated_slot: u64,
    pub compression_info: CompressionInfo,
}

impl Default for AmoebaDlmmSharePageV1 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: AMOEBA_DLMM_SHARE_PAGE_ACCOUNT_DISCRIMINATOR,
            account_version: AMOEBA_DLMM_ACCOUNT_VERSION,
            pool: Pubkey::default(),
            page_index: 0,
            first_bin_id: 0,
            total_liquidity_shares: [0; 32],
            last_updated_slot: 0,
            compression_info: CompressionInfo::default(),
        }
    }
}

impl AmoebaDlmmSharePageV1 {
    pub const LEN: usize = 594;
    pub const BODY_LEN: usize = Self::LEN - 8;

    pub fn has_current_layout(&self) -> bool {
        self.is_initialized
            && self.account_discriminator == AMOEBA_DLMM_SHARE_PAGE_ACCOUNT_DISCRIMINATOR
            && self.account_version == AMOEBA_DLMM_ACCOUNT_VERSION
            && self.compression_info.state == CompressionState::Decompressed
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AmoebaDlmmPositionV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub pool: Pubkey,
    pub owner: Pubkey,
    pub position_nonce: u64,
    pub lower_bin_id: u16,
    pub bin_count: u8,
    pub initialized_bitmap: u32,
    pub liquidity_shares: [u128; 32],
    pub last_updated_slot: u64,
    pub compression_info: CompressionInfo,
}

impl Default for AmoebaDlmmPositionV1 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: AMOEBA_DLMM_POSITION_ACCOUNT_DISCRIMINATOR,
            account_version: AMOEBA_DLMM_ACCOUNT_VERSION,
            pool: Pubkey::default(),
            owner: Pubkey::default(),
            position_nonce: 0,
            lower_bin_id: 0,
            bin_count: 0,
            initialized_bitmap: 0,
            liquidity_shares: [0; 32],
            last_updated_slot: 0,
            compression_info: CompressionInfo::default(),
        }
    }
}

impl AmoebaDlmmPositionV1 {
    pub const LEN: usize = 637;
    pub const BODY_LEN: usize = Self::LEN - 8;

    pub fn has_current_layout(&self) -> bool {
        self.is_initialized
            && self.account_discriminator == AMOEBA_DLMM_POSITION_ACCOUNT_DISCRIMINATOR
            && self.account_version == AMOEBA_DLMM_ACCOUNT_VERSION
            && self.compression_info.state == CompressionState::Decompressed
    }

    pub fn is_empty(&self) -> bool {
        self.initialized_bitmap == 0 && self.liquidity_shares.iter().all(|shares| *shares == 0)
    }
}

fixed_state_deserialize!(AmoebaDlmmPoolV1, AmoebaDlmmPoolV1::BODY_LEN, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    market: Pubkey,
    oracle_month: Pubkey,
    liquidity_manager: Pubkey,
    option_mint: Pubkey,
    quote_mint: Pubkey,
    option_vault: Pubkey,
    quote_vault: Pubkey,
    expiry_ts: u64,
    tick_size_quote_atomic: u64,
    maximum_price_quote_atomic: u64,
    maximum_bin_id: u16,
    best_bid_bin_id: u16,
    best_ask_bin_id: u16,
    last_trade_bin_id: u16,
    initialized_page_bitmap: u64,
    bid_page_bitmap: u64,
    ask_page_bitmap: u64,
    swap_fee_bps: u16,
    protocol_fee_share_bps: u16,
    maximum_bins_per_swap: u8,
    accounted_option_reserve: u64,
    accounted_quote_reserve: u64,
    protocol_fee_option: u64,
    protocol_fee_quote: u64,
    position_count: u32,
    status: AmoebaDlmmPoolStatus,
    settlement_price_atomic: u64,
    settled_slot: u64,
    last_updated_slot: u64,
    compression_info: CompressionInfo,
});

fixed_state_deserialize!(AmoebaDlmmBinPageV1, AmoebaDlmmBinPageV1::BODY_LEN, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    pool: Pubkey,
    page_index: u16,
    first_bin_id: u16,
    bid_bitmap: u32,
    ask_bitmap: u32,
    option_reserve: [u64; 32],
    quote_reserve: [u64; 32],
    last_updated_slot: u64,
    compression_info: CompressionInfo,
});

fixed_state_deserialize!(AmoebaDlmmSharePageV1, AmoebaDlmmSharePageV1::BODY_LEN, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    pool: Pubkey,
    page_index: u16,
    first_bin_id: u16,
    total_liquidity_shares: [u128; 32],
    last_updated_slot: u64,
    compression_info: CompressionInfo,
});

fixed_state_deserialize!(AmoebaDlmmPositionV1, AmoebaDlmmPositionV1::BODY_LEN, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    pool: Pubkey,
    owner: Pubkey,
    position_nonce: u64,
    lower_bin_id: u16,
    bin_count: u8,
    initialized_bitmap: u32,
    liquidity_shares: [u128; 32],
    last_updated_slot: u64,
    compression_info: CompressionInfo,
});

pub(crate) trait AmoebaDlmmLightState: Clone + FixedStateDecode + FixedStateEncode {
    const ACCOUNT_LEN: usize;
    const LIGHT_DISCRIMINATOR: [u8; 8];

    fn compression_info_mut(&mut self) -> &mut CompressionInfo;
}

macro_rules! impl_ameba_dlmm_light_state {
    ($state:ty, $light_discriminator:expr, $account_discriminator:expr, $account_len:expr) => {
        impl AmoebaDlmmLightState for $state {
            const ACCOUNT_LEN: usize = $account_len;
            const LIGHT_DISCRIMINATOR: [u8; 8] = $light_discriminator;

            fn compression_info_mut(&mut self) -> &mut CompressionInfo {
                &mut self.compression_info
            }
        }
    };
}

impl_ameba_dlmm_light_state!(
    AmoebaDlmmPoolV1,
    AMOEBA_DLMM_POOL_LIGHT_DISCRIMINATOR,
    AMOEBA_DLMM_POOL_ACCOUNT_DISCRIMINATOR,
    AmoebaDlmmPoolV1::LEN
);
impl_ameba_dlmm_light_state!(
    AmoebaDlmmBinPageV1,
    AMOEBA_DLMM_BIN_PAGE_LIGHT_DISCRIMINATOR,
    AMOEBA_DLMM_BIN_PAGE_ACCOUNT_DISCRIMINATOR,
    AmoebaDlmmBinPageV1::LEN
);
impl_ameba_dlmm_light_state!(
    AmoebaDlmmSharePageV1,
    AMOEBA_DLMM_SHARE_PAGE_LIGHT_DISCRIMINATOR,
    AMOEBA_DLMM_SHARE_PAGE_ACCOUNT_DISCRIMINATOR,
    AmoebaDlmmSharePageV1::LEN
);
impl_ameba_dlmm_light_state!(
    AmoebaDlmmPositionV1,
    AMOEBA_DLMM_POSITION_LIGHT_DISCRIMINATOR,
    AMOEBA_DLMM_POSITION_ACCOUNT_DISCRIMINATOR,
    AmoebaDlmmPositionV1::LEN
);

pub fn derive_ameba_dlmm_pool_pda(program_id: &Pubkey, market: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            AMOEBA_DLMM_POOL_PDA_SEED,
            market.as_ref(),
        ],
        program_id,
    )
}

pub fn derive_ameba_dlmm_authority_pda(program_id: &Pubkey, pool: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            AMOEBA_DLMM_AUTHORITY_PDA_SEED,
            pool.as_ref(),
        ],
        program_id,
    )
}

pub fn derive_ameba_dlmm_bin_page_pda(
    program_id: &Pubkey,
    pool: &Pubkey,
    page_index: u16,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            AMOEBA_DLMM_BIN_PAGE_PDA_SEED,
            pool.as_ref(),
            &page_index.to_le_bytes(),
        ],
        program_id,
    )
}

pub fn derive_ameba_dlmm_share_page_pda(
    program_id: &Pubkey,
    pool: &Pubkey,
    page_index: u16,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            AMOEBA_DLMM_SHARE_PAGE_PDA_SEED,
            pool.as_ref(),
            &page_index.to_le_bytes(),
        ],
        program_id,
    )
}

pub fn derive_ameba_dlmm_position_pda(
    program_id: &Pubkey,
    pool: &Pubkey,
    owner: &Pubkey,
    position_nonce: u64,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            AMOEBA_DLMM_POSITION_PDA_SEED,
            pool.as_ref(),
            owner.as_ref(),
            &position_nonce.to_le_bytes(),
        ],
        program_id,
    )
}

pub fn derive_ameba_dlmm_vault_pda(
    program_id: &Pubkey,
    pool: &Pubkey,
    mint: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            AMOEBA_DLMM_VAULT_PDA_SEED,
            pool.as_ref(),
            mint.as_ref(),
        ],
        program_id,
    )
}
