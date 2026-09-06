use super::*;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InstrumentDefinition {
    pub underlying_id: [u8; 32],
    pub expiry_ts: u64,
    pub strike_price: u64,
    pub cap_price: u64,
    pub contract_size: u64,
    pub max_payout_per_contract: u64,
    pub kind: OptionKind,
    pub settlement: SettlementStyle,
}

impl InstrumentDefinition {
    pub const LEN: usize = 32 + 8 + 8 + 8 + 8 + 8 + 1 + 1;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MarketParameters {
    pub tick_size: u64,
    pub lot_size: u64,
    pub min_order_qty: u64,
    pub maker_fee_bps: u16,
    pub taker_fee_bps: u16,
    pub cancel_fee_bps: u16,
    pub min_cancel_slots: u64,
    pub max_fills_per_instruction: u8,
}

impl MarketParameters {
    pub const LEN: usize = 8 + 8 + 8 + 2 + 2 + 2 + 8 + 1;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MarketMintAccounting {
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub decimals: u8,
    pub reserved: [u8; 3],
    pub total_issued: u64,
    pub total_consumed: u64,
    pub total_burned: u64,
}

impl MarketMintAccounting {
    pub const LEN: usize = 32;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"MNT";
    pub const ACCOUNT_VERSION: u8 = 1;
    pub const CANONICAL_DECIMALS: u8 = 6;
    /// Atomic units per whole current contract token. Instrument prices and
    /// collateral also use six-decimal atomic values, so spread-width payout
    /// multiplication divides by this scale exactly once.
    pub const CANONICAL_ATOMIC_SCALE: u64 = 1_000_000;

    pub const fn canonical_empty() -> Self {
        Self {
            account_discriminator: Self::ACCOUNT_DISCRIMINATOR,
            account_version: Self::ACCOUNT_VERSION,
            decimals: Self::CANONICAL_DECIMALS,
            reserved: [0; 3],
            total_issued: 0,
            total_consumed: 0,
            total_burned: 0,
        }
    }

    pub fn has_canonical_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && self.decimals == Self::CANONICAL_DECIMALS
            && self.reserved == [0; 3]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Market {
    pub is_initialized: bool,
    pub bump: u8,
    /// Immutable creation provenance. Authorization always comes from the current vault config.
    pub created_by: Pubkey,
    pub market_id: [u8; 32],
    pub collateral_mint: Pubkey,
    pub long_contract_mint: Option<Pubkey>,
    pub instrument: InstrumentDefinition,
    pub params: MarketParameters,
    pub total_position_collateral_locked: u64,
    pub paused: bool,
    pub mint_accounting: MarketMintAccounting,
}

impl Default for Market {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            created_by: Pubkey::default(),
            market_id: [0; 32],
            collateral_mint: Pubkey::default(),
            long_contract_mint: None,
            instrument: InstrumentDefinition::default(),
            params: MarketParameters::default(),
            total_position_collateral_locked: 0,
            paused: true,
            mint_accounting: MarketMintAccounting::default(),
        }
    }
}

impl Market {
    pub const LEN: usize = 1
        + 1
        + 32
        + 32
        + 32
        + (1 + 32)
        + InstrumentDefinition::LEN
        + MarketParameters::LEN
        + 8
        + 1
        + MarketMintAccounting::LEN;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserCollateral {
    pub is_initialized: bool,
    pub bump: u8,
    pub owner: Pubkey,
    pub available_balance: u64,
    pub position_locked_balance: u64,
    pub last_action_slot: u64,
}

#[allow(clippy::derivable_impls)]
impl Default for UserCollateral {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            owner: Pubkey::default(),
            available_balance: 0,
            position_locked_balance: 0,
            last_action_slot: 0,
        }
    }
}

impl UserCollateral {
    pub const LEN: usize = 1 + 1 + 32 + 8 + 8 + 8;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PositionRecord {
    pub is_initialized: bool,
    pub bump: u8,
    pub market: Pubkey,
    pub owner: Pubkey,
    pub long_qty: u64,
    pub short_qty: u64,
    pub short_collateral_locked: u64,
    pub long_tokens_minted: u64,
    pub settlement_claimed: bool,
    pub settlement_claimed_slot: u64,
    pub last_updated_slot: u64,
}

#[allow(clippy::derivable_impls)]
impl Default for PositionRecord {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            market: Pubkey::default(),
            owner: Pubkey::default(),
            long_qty: 0,
            short_qty: 0,
            short_collateral_locked: 0,
            long_tokens_minted: 0,
            settlement_claimed: false,
            settlement_claimed_slot: 0,
            last_updated_slot: 0,
        }
    }
}

impl PositionRecord {
    pub const LEN: usize = 1 + 1 + 32 + 32 + 8 + 8 + 8 + 8 + 1 + 8 + 8;
}

/// Immutable market/month-scoped settlement provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettlementRecordV2 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub market: Pubkey,
    pub oracle_month: Pubkey,
    pub settlement_ts: u64,
    pub settlement_price_atomic: u64,
    pub signed_leaf_commitment: [u8; 32],
    pub submitted_by: Pubkey,
    /// Immutable active signer-set version.
    pub signer_set_version: u64,
}

#[allow(clippy::derivable_impls)]
impl Default for SettlementRecordV2 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            market: Pubkey::default(),
            oracle_month: Pubkey::default(),
            settlement_ts: 0,
            settlement_price_atomic: 0,
            signed_leaf_commitment: [0; 32],
            submitted_by: Pubkey::default(),
            signer_set_version: 0,
        }
    }
}

impl SettlementRecordV2 {
    pub const LEN: usize = 160;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"STL";
    pub const ACCOUNT_VERSION: u8 = 4;
}
