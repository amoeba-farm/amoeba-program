use super::*;

pub use crate::constants::{
    WRITER_MAX_FUNDED_BIDS as WRITER_BID_STORAGE_CAPACITY,
    WRITER_MAX_LIVE_SERIES as WRITER_LIVE_SERIES_LIMIT, WRITER_SERIES_STORAGE_CAPACITY,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WriterSecurityMode {
    #[default]
    GrossExternalMaxPayout,
    ExactExternalEnvelope,
}

stable_borsh_enum!(WriterSecurityMode {
    GrossExternalMaxPayout = 0,
    ExactExternalEnvelope = 1,
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WriterReserveRoundingMode {
    #[default]
    AggregateBookCeiling,
}

stable_borsh_enum!(WriterReserveRoundingMode {
    AggregateBookCeiling = 0,
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WriterAuctionPriorityRule {
    #[default]
    PayAsBidPriceThenSeriesProRata,
}

stable_borsh_enum!(WriterAuctionPriorityRule {
    PayAsBidPriceThenSeriesProRata = 0,
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WriterSettlementGroupStatus {
    #[default]
    Anchored,
    Active,
    Settled,
    Closed,
}

stable_borsh_enum!(WriterSettlementGroupStatus {
    Anchored = 0,
    Active = 1,
    Settled = 2,
    Closed = 3,
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WriterSleeveStatus {
    #[default]
    Draft,
    PolicyFrozen,
    Funding,
    Active,
    CloseStaging,
    Expired,
    SettlementFinalized,
    Closed,
}

stable_borsh_enum!(WriterSleeveStatus {
    Draft = 0,
    PolicyFrozen = 1,
    Funding = 2,
    Active = 3,
    CloseStaging = 4,
    Expired = 5,
    SettlementFinalized = 6,
    Closed = 7,
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WriterSeriesCustodyStatus {
    #[default]
    Absent,
    Open,
    Closed,
}

stable_borsh_enum!(WriterSeriesCustodyStatus {
    Absent = 0,
    Open = 1,
    Closed = 2,
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WriterSeriesSettlementStatus {
    #[default]
    Open,
    Frozen,
    Exhausted,
}

stable_borsh_enum!(WriterSeriesSettlementStatus {
    Open = 0,
    Frozen = 1,
    Exhausted = 2,
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WriterAuctionStatus {
    #[default]
    Committed,
    Bidding,
    Revealed,
    Planning,
    Executing,
    Finalized,
    Refundable,
    Closed,
}

stable_borsh_enum!(WriterAuctionStatus {
    Committed = 0,
    Bidding = 1,
    Revealed = 2,
    Planning = 3,
    Executing = 4,
    Finalized = 5,
    Refundable = 6,
    Closed = 7,
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WriterBidStatus {
    #[default]
    Empty,
    Funded,
    Cancelled,
    Planned,
    Executed,
    Refundable,
    Refunded,
}

stable_borsh_enum!(WriterBidStatus {
    Empty = 0,
    Funded = 1,
    Cancelled = 2,
    Planned = 3,
    Executed = 4,
    Refundable = 5,
    Refunded = 6,
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WriterBidDeliveryMode {
    #[default]
    LightToken,
    ClassicSpl,
}

stable_borsh_enum!(WriterBidDeliveryMode {
    LightToken = 0,
    ClassicSpl = 1,
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WriterCloseRequestStatus {
    #[default]
    Collecting,
    Complete,
    Finalized,
    Cancelling,
    Cancelled,
}

stable_borsh_enum!(WriterCloseRequestStatus {
    Collecting = 0,
    Complete = 1,
    Finalized = 2,
    Cancelling = 3,
    Cancelled = 4,
});

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterPolicyRegistryV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub vault_config: Pubkey,
    pub policy_authority: Pubkey,
    pub pending_policy_authority: Pubkey,
    pub protocol_fee_vault: Pubkey,
    pub pending_activation_slot: u64,
    pub rotation_delay_slots: u64,
    pub latest_policy_version: u64,
    pub last_updated_slot: u64,
    pub reserved: [u8; 32],
}

#[allow(clippy::derivable_impls)]
impl Default for WriterPolicyRegistryV1 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            vault_config: Pubkey::default(),
            policy_authority: Pubkey::default(),
            pending_policy_authority: Pubkey::default(),
            protocol_fee_vault: Pubkey::default(),
            pending_activation_slot: 0,
            rotation_delay_slots: 0,
            latest_policy_version: 0,
            last_updated_slot: 0,
            reserved: [0; 32],
        }
    }
}

impl WriterPolicyRegistryV1 {
    pub const LEN: usize = 198;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"WPR";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && self.reserved == [0; 32]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterPolicySnapshotV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub sleeve: Pubkey,
    pub registry: Pubkey,
    pub policy_version: u64,
    pub regime_input_version: u64,
    pub policy_hash: [u8; 32],
    pub scenario_set_hash: [u8; 32],
    pub risk_limit_hash: [u8; 32],
    pub series_family_hash: [u8; 32],
    pub security_mode: WriterSecurityMode,
    pub reserve_rounding_mode: WriterReserveRoundingMode,
    pub auction_priority_rule: WriterAuctionPriorityRule,
    pub v2_feature_flags: u8,
    pub max_series: u8,
    pub reserved_0: u8,
    pub primary_fee_bps: u16,
    pub reserved_1: [u8; 2],
    pub drawdown_scale: u64,
    pub worst_drawdown_limit: u64,
    pub upper_drawdown_limit: u64,
    pub lower_drawdown_limit: u64,
    pub lower_tail_max_settlement_atomic: u64,
    pub upper_tail_min_settlement_atomic: u64,
    pub operational_buffer_atoms: u64,
    pub max_auction_issue_atoms: u64,
    pub max_close_flat_atoms: u64,
    pub created_slot: u64,
    pub sealed_slot: u64,
    pub reserved: [u8; 32],
}

impl Default for WriterPolicySnapshotV1 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            sleeve: Pubkey::default(),
            registry: Pubkey::default(),
            policy_version: 0,
            regime_input_version: 0,
            policy_hash: [0; 32],
            scenario_set_hash: [0; 32],
            risk_limit_hash: [0; 32],
            series_family_hash: [0; 32],
            security_mode: WriterSecurityMode::default(),
            reserve_rounding_mode: WriterReserveRoundingMode::default(),
            auction_priority_rule: WriterAuctionPriorityRule::default(),
            v2_feature_flags: 0,
            max_series: WRITER_LIVE_SERIES_LIMIT as u8,
            reserved_0: 0,
            primary_fee_bps: 0,
            reserved_1: [0; 2],
            drawdown_scale: 0,
            worst_drawdown_limit: 0,
            upper_drawdown_limit: 0,
            lower_drawdown_limit: 0,
            lower_tail_max_settlement_atomic: 0,
            upper_tail_min_settlement_atomic: 0,
            operational_buffer_atoms: 0,
            max_auction_issue_atoms: 0,
            max_close_flat_atoms: 0,
            created_slot: 0,
            sealed_slot: 0,
            reserved: [0; 32],
        }
    }
}

impl WriterPolicySnapshotV1 {
    pub const LEN: usize = 344;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"WPS";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && self.reserved_0 == 0
            && self.reserved_1 == [0; 2]
            && self.reserved == [0; 32]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterSettlementGroupV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub underlying_id: [u8; 32],
    pub expiry_ts: u64,
    pub settlement_mint: Pubkey,
    pub anchor_market: Pubkey,
    pub anchor_oracle_month: Pubkey,
    pub oracle_methodology_version: u64,
    pub product_manifest_root: [u8; 32],
    pub coverage_manifest_hash: [u8; 32],
    pub recipe_hash: [u8; 32],
    pub settlement_source_digest: [u8; 32],
    pub active_weight_manifest_hash: [u8; 32],
    pub security_cap_atoms: u64,
    pub signer_registry: Pubkey,
    pub signer_set: Pubkey,
    pub signer_set_version: u64,
    pub signer_set_hash: [u8; 32],
    pub settlement_ts: u64,
    pub settlement_price_atomic: u64,
    pub final_settlement_commitment: [u8; 32],
    pub submitted_by: Pubkey,
    pub finalized_slot: u64,
    pub sleeve: Pubkey,
    pub status: WriterSettlementGroupStatus,
    pub series_count: u8,
    pub reserved: [u8; 6],
    pub last_updated_slot: u64,
}

#[allow(clippy::derivable_impls)]
impl Default for WriterSettlementGroupV1 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            underlying_id: [0; 32],
            expiry_ts: 0,
            settlement_mint: Pubkey::default(),
            anchor_market: Pubkey::default(),
            anchor_oracle_month: Pubkey::default(),
            oracle_methodology_version: 0,
            product_manifest_root: [0; 32],
            coverage_manifest_hash: [0; 32],
            recipe_hash: [0; 32],
            settlement_source_digest: [0; 32],
            active_weight_manifest_hash: [0; 32],
            security_cap_atoms: 0,
            signer_registry: Pubkey::default(),
            signer_set: Pubkey::default(),
            signer_set_version: 0,
            signer_set_hash: [0; 32],
            settlement_ts: 0,
            settlement_price_atomic: 0,
            final_settlement_commitment: [0; 32],
            submitted_by: Pubkey::default(),
            finalized_slot: 0,
            sleeve: Pubkey::default(),
            status: WriterSettlementGroupStatus::default(),
            series_count: 0,
            reserved: [0; 6],
            last_updated_slot: 0,
        }
    }
}

impl WriterSettlementGroupV1 {
    pub const LEN: usize = 558;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"WSG";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && self.reserved == [0; 6]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterSleeveV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub vault_config: Pubkey,
    pub underlying_id: [u8; 32],
    pub expiry_ts: u64,
    pub settlement_mint: Pubkey,
    pub settlement_group: Pubkey,
    pub series_book: Pubkey,
    pub usdc_vault: Pubkey,
    pub flat_mint: Pubkey,
    pub flat_spl_interface: Pubkey,
    pub flat_staging: Pubkey,
    pub flat_burn_custody: Pubkey,
    pub policy_registry: Pubkey,
    pub policy_snapshot: Pubkey,
    pub policy_version: u64,
    pub policy_hash: [u8; 32],
    pub scenario_set_hash: [u8; 32],
    pub risk_limit_hash: [u8; 32],
    pub writer_principal_atoms: u64,
    pub locked_primary_premium_atoms: u64,
    pub accounted_asset_atoms: u64,
    pub exact_reserve_atoms: u64,
    pub upper_tail_reserve_atoms: u64,
    pub lower_tail_reserve_atoms: u64,
    pub flat_par_supply_atoms: u64,
    pub security_exposure_atoms: u64,
    pub long_liability_initial_atoms: u64,
    pub long_liability_remaining_atoms: u64,
    pub flat_residual_initial_atoms: u64,
    pub flat_residual_remaining_atoms: u64,
    pub flat_supply_snapshot_atoms: u64,
    pub flat_claim_supply_remaining_atoms: u64,
    pub stranded_surplus_atoms: u64,
    pub operational_buffer_atoms: u64,
    pub auction_nonce: u64,
    pub close_nonce: u64,
    pub series_count: u8,
    pub status: WriterSleeveStatus,
    pub security_mode: WriterSecurityMode,
    pub v2_feature_flags: u8,
    pub active_auction: Option<Pubkey>,
    pub active_close_request: Option<Pubkey>,
    pub settlement_finalized_slot: u64,
    pub last_updated_slot: u64,
    pub reserved: [u8; 32],
}

#[allow(clippy::derivable_impls)]
impl Default for WriterSleeveV1 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            vault_config: Pubkey::default(),
            underlying_id: [0; 32],
            expiry_ts: 0,
            settlement_mint: Pubkey::default(),
            settlement_group: Pubkey::default(),
            series_book: Pubkey::default(),
            usdc_vault: Pubkey::default(),
            flat_mint: Pubkey::default(),
            flat_spl_interface: Pubkey::default(),
            flat_staging: Pubkey::default(),
            flat_burn_custody: Pubkey::default(),
            policy_registry: Pubkey::default(),
            policy_snapshot: Pubkey::default(),
            policy_version: 0,
            policy_hash: [0; 32],
            scenario_set_hash: [0; 32],
            risk_limit_hash: [0; 32],
            writer_principal_atoms: 0,
            locked_primary_premium_atoms: 0,
            accounted_asset_atoms: 0,
            exact_reserve_atoms: 0,
            upper_tail_reserve_atoms: 0,
            lower_tail_reserve_atoms: 0,
            flat_par_supply_atoms: 0,
            security_exposure_atoms: 0,
            long_liability_initial_atoms: 0,
            long_liability_remaining_atoms: 0,
            flat_residual_initial_atoms: 0,
            flat_residual_remaining_atoms: 0,
            flat_supply_snapshot_atoms: 0,
            flat_claim_supply_remaining_atoms: 0,
            stranded_surplus_atoms: 0,
            operational_buffer_atoms: 0,
            auction_nonce: 0,
            close_nonce: 0,
            series_count: 0,
            status: WriterSleeveStatus::default(),
            security_mode: WriterSecurityMode::default(),
            v2_feature_flags: 0,
            active_auction: None,
            active_close_request: None,
            settlement_finalized_slot: 0,
            last_updated_slot: 0,
            reserved: [0; 32],
        }
    }
}

impl WriterSleeveV1 {
    pub const LEN: usize = 764;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"WSL";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && self.reserved == [0; 32]
    }
}

#[derive(BorshDeserialize, BorshSerialize, Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriterSeriesRecordV1 {
    pub active: bool,
    pub option_kind: OptionKind,
    pub custody_status: WriterSeriesCustodyStatus,
    pub settlement_status: WriterSeriesSettlementStatus,
    pub reserved: [u8; 4],
    pub series_id: [u8; 32],
    pub market: Pubkey,
    pub contract_mint: Pubkey,
    pub retirement_custody: Pubkey,
    pub strike_price_atomic: u64,
    pub cap_or_floor_price_atomic: u64,
    pub contract_size_atoms: u64,
    pub max_payout_per_contract_atoms: u64,
    pub total_physical_supply_atoms: u64,
    pub issuer_controlled_atoms: u64,
    pub external_open_interest_atoms: u64,
    pub primary_premium_collected_atoms: u64,
    pub settlement_external_oi_snapshot_atoms: u64,
    pub settlement_liability_initial_atoms: u64,
    pub settlement_liability_remaining_atoms: u64,
    pub payoff_digest: [u8; 32],
}

impl WriterSeriesRecordV1 {
    pub const LEN: usize = 256;
    pub const EMPTY: Self = Self {
        active: false,
        option_kind: OptionKind::CallSpread,
        custody_status: WriterSeriesCustodyStatus::Absent,
        settlement_status: WriterSeriesSettlementStatus::Open,
        reserved: [0; 4],
        series_id: [0; 32],
        market: Pubkey::new_from_array([0; 32]),
        contract_mint: Pubkey::new_from_array([0; 32]),
        retirement_custody: Pubkey::new_from_array([0; 32]),
        strike_price_atomic: 0,
        cap_or_floor_price_atomic: 0,
        contract_size_atoms: 0,
        max_payout_per_contract_atoms: 0,
        total_physical_supply_atoms: 0,
        issuer_controlled_atoms: 0,
        external_open_interest_atoms: 0,
        primary_premium_collected_atoms: 0,
        settlement_external_oi_snapshot_atoms: 0,
        settlement_liability_initial_atoms: 0,
        settlement_liability_remaining_atoms: 0,
        payoff_digest: [0; 32],
    };

    pub fn is_canonical_empty(&self) -> bool {
        self == &Self::EMPTY
    }
}

impl Default for WriterSeriesRecordV1 {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterSeriesBookV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub sleeve: Pubkey,
    pub settlement_group: Pubkey,
    pub series_count: u8,
    pub max_series: u8,
    pub frozen: bool,
    pub reserved: [u8; 7],
    pub book_digest: [u8; 32],
    pub last_updated_slot: u64,
    pub records: Box<[WriterSeriesRecordV1; WRITER_SERIES_STORAGE_CAPACITY]>,
}

impl Default for WriterSeriesBookV1 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            sleeve: Pubkey::default(),
            settlement_group: Pubkey::default(),
            series_count: 0,
            max_series: WRITER_LIVE_SERIES_LIMIT as u8,
            frozen: false,
            reserved: [0; 7],
            book_digest: [0; 32],
            last_updated_slot: 0,
            records: vec![WriterSeriesRecordV1::EMPTY; WRITER_SERIES_STORAGE_CAPACITY]
                .into_boxed_slice()
                .try_into()
                .expect("fixed writer series capacity"),
        }
    }
}

impl WriterSeriesBookV1 {
    pub const LEN: usize = 8_312;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"WSB";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && self.max_series == WRITER_LIVE_SERIES_LIMIT as u8
            && usize::from(self.series_count) <= WRITER_LIVE_SERIES_LIMIT
            && self.reserved == [0; 7]
            && self.records[usize::from(self.series_count)..]
                .iter()
                .all(WriterSeriesRecordV1::is_canonical_empty)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterAuctionV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub sleeve: Pubkey,
    pub series_book: Pubkey,
    pub policy_snapshot: Pubkey,
    pub auction_nonce: u64,
    pub escrow: Pubkey,
    pub bid_index: Pubkey,
    pub fee_vault: Pubkey,
    pub policy_version: u64,
    pub scenario_set_hash: [u8; 32],
    pub risk_limit_hash: [u8; 32],
    /// For auctions created by this implementation, the actual-slot-bound reserve
    /// commitment stored by the program at commit time.
    pub reserve_vector_commitment: [u8; 32],
    pub reveal_hash: [u8; 32],
    pub revealed_nonce: [u8; 32],
    pub commit_slot: u64,
    pub bid_deadline_ts: u64,
    pub reveal_deadline_ts: u64,
    pub execute_deadline_ts: u64,
    pub bid_count: u16,
    pub planned_bid_count: u16,
    pub executed_bid_count: u16,
    pub refunded_bid_count: u16,
    pub planning_cursor: u16,
    pub reserved_0: [u8; 6],
    pub total_escrow_atoms: u64,
    pub accepted_premium_atoms: u64,
    pub accepted_fee_atoms: u64,
    pub accepted_contract_atoms: u64,
    pub refundable_atoms: u64,
    pub plan_digest: [u8; 32],
    pub status: WriterAuctionStatus,
    pub reserved_1: [u8; 7],
    pub last_updated_slot: u64,
    pub reserve_prices_atoms: [u64; WRITER_SERIES_STORAGE_CAPACITY],
    pub issue_caps_atoms: [u64; WRITER_SERIES_STORAGE_CAPACITY],
    pub planned_issue_atoms: [u64; WRITER_SERIES_STORAGE_CAPACITY],
    pub executed_issue_atoms: [u64; WRITER_SERIES_STORAGE_CAPACITY],
}

impl Default for WriterAuctionV1 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            sleeve: Pubkey::default(),
            series_book: Pubkey::default(),
            policy_snapshot: Pubkey::default(),
            auction_nonce: 0,
            escrow: Pubkey::default(),
            bid_index: Pubkey::default(),
            fee_vault: Pubkey::default(),
            policy_version: 0,
            scenario_set_hash: [0; 32],
            risk_limit_hash: [0; 32],
            reserve_vector_commitment: [0; 32],
            reveal_hash: [0; 32],
            revealed_nonce: [0; 32],
            commit_slot: 0,
            bid_deadline_ts: 0,
            reveal_deadline_ts: 0,
            execute_deadline_ts: 0,
            bid_count: 0,
            planned_bid_count: 0,
            executed_bid_count: 0,
            refunded_bid_count: 0,
            planning_cursor: 0,
            reserved_0: [0; 6],
            total_escrow_atoms: 0,
            accepted_premium_atoms: 0,
            accepted_fee_atoms: 0,
            accepted_contract_atoms: 0,
            refundable_atoms: 0,
            plan_digest: [0; 32],
            status: WriterAuctionStatus::default(),
            reserved_1: [0; 7],
            last_updated_slot: 0,
            reserve_prices_atoms: [0; WRITER_SERIES_STORAGE_CAPACITY],
            issue_caps_atoms: [0; WRITER_SERIES_STORAGE_CAPACITY],
            planned_issue_atoms: [0; WRITER_SERIES_STORAGE_CAPACITY],
            executed_issue_atoms: [0; WRITER_SERIES_STORAGE_CAPACITY],
        }
    }
}

impl WriterAuctionV1 {
    pub const LEN: usize = 1_534;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"WAU";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && self.reserved_0 == [0; 6]
            && self.reserved_1 == [0; 7]
            && usize::from(self.bid_count) <= WRITER_BID_STORAGE_CAPACITY
            && self.planned_bid_count <= self.bid_count
            && self.executed_bid_count <= self.planned_bid_count
            && self.refunded_bid_count <= self.bid_count
            && self.planning_cursor <= self.bid_count
    }
}

#[derive(BorshDeserialize, BorshSerialize, Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriterBidIndexRecordV1 {
    pub occupied: bool,
    pub status: WriterBidStatus,
    pub series_index: u8,
    pub reserved: u8,
    pub bid_price_per_contract_atoms: u64,
    pub requested_contract_atoms: u64,
    pub accepted_contract_atoms: u64,
    pub executed_contract_atoms: u64,
    pub escrowed_atoms: u64,
    pub bid: Pubkey,
    pub bidder: Pubkey,
    pub order_id: u64,
}

impl WriterBidIndexRecordV1 {
    pub const LEN: usize = 116;
    pub const EMPTY: Self = Self {
        occupied: false,
        status: WriterBidStatus::Empty,
        series_index: 0,
        reserved: 0,
        bid_price_per_contract_atoms: 0,
        requested_contract_atoms: 0,
        accepted_contract_atoms: 0,
        executed_contract_atoms: 0,
        escrowed_atoms: 0,
        bid: Pubkey::new_from_array([0; 32]),
        bidder: Pubkey::new_from_array([0; 32]),
        order_id: 0,
    };

    pub fn has_current_layout(&self) -> bool {
        self.occupied
            && self.status != WriterBidStatus::Empty
            && usize::from(self.series_index) < WRITER_LIVE_SERIES_LIMIT
            && self.reserved == 0
            && self.bid_price_per_contract_atoms != 0
            && self.requested_contract_atoms != 0
            && self.accepted_contract_atoms <= self.requested_contract_atoms
            && self.executed_contract_atoms <= self.accepted_contract_atoms
            && self.escrowed_atoms != 0
            && !crate::pubkey_is_default(&self.bid)
            && !crate::pubkey_is_default(&self.bidder)
    }
}

impl Default for WriterBidIndexRecordV1 {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterBidIndexV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub auction: Pubkey,
    pub bid_count: u16,
    pub planned_bid_count: u16,
    pub executed_bid_count: u16,
    pub refunded_bid_count: u16,
    pub planning_cursor: u16,
    pub rolling_digest: [u8; 32],
    pub last_updated_slot: u64,
    pub records: Box<[WriterBidIndexRecordV1; WRITER_BID_STORAGE_CAPACITY]>,
}

impl Default for WriterBidIndexV1 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            auction: Pubkey::default(),
            bid_count: 0,
            planned_bid_count: 0,
            executed_bid_count: 0,
            refunded_bid_count: 0,
            planning_cursor: 0,
            rolling_digest: [0; 32],
            last_updated_slot: 0,
            records: vec![WriterBidIndexRecordV1::EMPTY; WRITER_BID_STORAGE_CAPACITY]
                .into_boxed_slice()
                .try_into()
                .expect("fixed writer bid capacity"),
        }
    }
}

impl WriterBidIndexV1 {
    pub const LEN: usize = 14_936;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"WBI";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && usize::from(self.bid_count) <= WRITER_BID_STORAGE_CAPACITY
            && self.planned_bid_count <= self.bid_count
            && self.executed_bid_count <= self.planned_bid_count
            && self.refunded_bid_count <= self.bid_count
            && self.planning_cursor <= self.bid_count
            && self.records[..usize::from(self.bid_count)]
                .iter()
                .all(WriterBidIndexRecordV1::has_current_layout)
            && self.records[usize::from(self.bid_count)..]
                .iter()
                .all(|record| record == &WriterBidIndexRecordV1::EMPTY)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterBidV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub auction: Pubkey,
    pub bidder: Pubkey,
    pub refund_token_account: Pubkey,
    pub claim_destination: Pubkey,
    pub order_id: u64,
    pub series_index: u8,
    pub status: WriterBidStatus,
    pub delivery_mode: WriterBidDeliveryMode,
    pub reserved: [u8; 5],
    pub bid_price_per_contract_atoms: u64,
    pub requested_contract_atoms: u64,
    pub accepted_contract_atoms: u64,
    pub executed_contract_atoms: u64,
    pub escrowed_atoms: u64,
    pub premium_charged_atoms: u64,
    pub fee_charged_atoms: u64,
    pub refunded_atoms: u64,
    pub placed_slot: u64,
    pub last_updated_slot: u64,
}

#[allow(clippy::derivable_impls)]
impl Default for WriterBidV1 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            auction: Pubkey::default(),
            bidder: Pubkey::default(),
            refund_token_account: Pubkey::default(),
            claim_destination: Pubkey::default(),
            order_id: 0,
            series_index: 0,
            status: WriterBidStatus::default(),
            delivery_mode: WriterBidDeliveryMode::default(),
            reserved: [0; 5],
            bid_price_per_contract_atoms: 0,
            requested_contract_atoms: 0,
            accepted_contract_atoms: 0,
            executed_contract_atoms: 0,
            escrowed_atoms: 0,
            premium_charged_atoms: 0,
            fee_charged_atoms: 0,
            refunded_atoms: 0,
            placed_slot: 0,
            last_updated_slot: 0,
        }
    }
}

impl WriterBidV1 {
    pub const LEN: usize = 230;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"WBD";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && self.reserved == [0; 5]
            && self
                .premium_charged_atoms
                .checked_add(self.fee_charged_atoms)
                .is_some_and(|charged| charged <= self.escrowed_atoms)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterCloseRequestV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub sleeve: Pubkey,
    pub owner: Pubkey,
    pub flat_escrow: Pubkey,
    pub flat_mint: Pubkey,
    pub request_nonce: u64,
    pub flat_amount_atoms: u64,
    pub minimum_withdrawal_atoms: u64,
    pub snapshot_asset_atoms: u64,
    pub snapshot_reserve_atoms: u64,
    pub snapshot_writer_principal_atoms: u64,
    pub snapshot_locked_primary_premium_atoms: u64,
    pub snapshot_flat_supply_atoms: u64,
    pub snapshot_security_exposure_atoms: u64,
    pub snapshot_policy_version: u64,
    pub snapshot_group_commitment: [u8; 32],
    pub snapshot_book_digest: [u8; 32],
    pub deadline_ts: u64,
    pub status: WriterCloseRequestStatus,
    pub series_count: u8,
    pub next_deposit_index: u8,
    pub next_cancel_index: u8,
    pub reserved: [u8; 6],
    pub required_claim_atoms: [u64; WRITER_SERIES_STORAGE_CAPACITY],
    pub deposited_claim_atoms: [u64; WRITER_SERIES_STORAGE_CAPACITY],
    pub snapshot_external_oi_atoms: [u64; WRITER_SERIES_STORAGE_CAPACITY],
    pub final_withdrawal_atoms: u64,
    pub finalized_slot: u64,
    pub last_updated_slot: u64,
}

impl Default for WriterCloseRequestV1 {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            sleeve: Pubkey::default(),
            owner: Pubkey::default(),
            flat_escrow: Pubkey::default(),
            flat_mint: Pubkey::default(),
            request_nonce: 0,
            flat_amount_atoms: 0,
            minimum_withdrawal_atoms: 0,
            snapshot_asset_atoms: 0,
            snapshot_reserve_atoms: 0,
            snapshot_writer_principal_atoms: 0,
            snapshot_locked_primary_premium_atoms: 0,
            snapshot_flat_supply_atoms: 0,
            snapshot_security_exposure_atoms: 0,
            snapshot_policy_version: 0,
            snapshot_group_commitment: [0; 32],
            snapshot_book_digest: [0; 32],
            deadline_ts: 0,
            status: WriterCloseRequestStatus::default(),
            series_count: 0,
            next_deposit_index: 0,
            next_cancel_index: 0,
            reserved: [0; 6],
            required_claim_atoms: [0; WRITER_SERIES_STORAGE_CAPACITY],
            deposited_claim_atoms: [0; WRITER_SERIES_STORAGE_CAPACITY],
            snapshot_external_oi_atoms: [0; WRITER_SERIES_STORAGE_CAPACITY],
            final_withdrawal_atoms: 0,
            finalized_slot: 0,
            last_updated_slot: 0,
        }
    }
}

impl WriterCloseRequestV1 {
    pub const LEN: usize = 1_088;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"WCR";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && usize::from(self.series_count) <= WRITER_LIVE_SERIES_LIMIT
            && self.next_deposit_index <= self.series_count
            && self.next_cancel_index <= self.series_count
            && self.reserved == [0; 6]
            && self.required_claim_atoms[usize::from(self.series_count)..]
                .iter()
                .all(|amount| *amount == 0)
            && self.deposited_claim_atoms[usize::from(self.series_count)..]
                .iter()
                .all(|amount| *amount == 0)
            && self.snapshot_external_oi_atoms[usize::from(self.series_count)..]
                .iter()
                .all(|amount| *amount == 0)
    }
}
