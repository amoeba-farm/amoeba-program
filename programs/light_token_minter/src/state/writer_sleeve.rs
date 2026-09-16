use super::*;

pub use crate::constants::{
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
pub enum WriterSettlementGroupStatus {
    #[default]
    Anchored,
    Active,
    Settled,
    Closed,
    FundingExpired,
}

stable_borsh_enum!(WriterSettlementGroupStatus {
    Anchored = 0,
    Active = 1,
    Settled = 2,
    Closed = 3,
    FundingExpired = 4,
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WriterSleeveStatus {
    #[default]
    Draft,
    PolicyFrozen,
    Funding,
    Active,
    Expired,
    SettlementFinalized,
    Closed,
    FundingRefunds,
}

stable_borsh_enum!(WriterSleeveStatus {
    Draft = 0,
    PolicyFrozen = 1,
    Funding = 2,
    Active = 3,
    Expired = 5,
    SettlementFinalized = 6,
    Closed = 7,
    FundingRefunds = 8,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterPolicyRegistryV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub vault_config: Pubkey,
    pub policy_authority: Pubkey,
    pub pending_policy_authority: Pubkey,

    pub pending_activation_slot: u64,
    pub rotation_delay_slots: u64,
    pub latest_policy_version: u64,
    pub last_updated_slot: u64,
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

            pending_activation_slot: 0,
            rotation_delay_slots: 0,
            latest_policy_version: 0,
            last_updated_slot: 0,
        }
    }
}

impl WriterPolicyRegistryV1 {
    pub const LEN: usize = 134;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"WPR";
    pub const ACCOUNT_VERSION: u8 = 3;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
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

    pub v2_feature_flags: u8,
    pub max_series: u8,

    pub drawdown_scale: u64,
    pub worst_drawdown_limit: u64,
    pub upper_drawdown_limit: u64,
    pub lower_drawdown_limit: u64,
    pub lower_tail_max_settlement_atomic: u64,
    pub upper_tail_min_settlement_atomic: u64,
    pub operational_buffer_atoms: u64,
    pub max_issue_atoms: u64,

    pub created_slot: u64,
    pub sealed_slot: u64,
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

            v2_feature_flags: 0,
            max_series: WRITER_LIVE_SERIES_LIMIT as u8,

            drawdown_scale: 0,
            worst_drawdown_limit: 0,
            upper_drawdown_limit: 0,
            lower_drawdown_limit: 0,
            lower_tail_max_settlement_atomic: 0,
            upper_tail_min_settlement_atomic: 0,
            operational_buffer_atoms: 0,
            max_issue_atoms: 0,

            created_slot: 0,
            sealed_slot: 0,
        }
    }
}

impl WriterPolicySnapshotV1 {
    pub const LEN: usize = 298;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"WPS";
    pub const ACCOUNT_VERSION: u8 = 3;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
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
    pub security_exposure_atoms: u64,
    pub long_liability_initial_atoms: u64,
    pub long_liability_remaining_atoms: u64,
    pub writer_residual_initial_atoms: u64,
    pub writer_residual_remaining_atoms: u64,
    pub settlement_principal_atoms: u64,
    pub unclaimed_principal_atoms: u64,
    pub stranded_surplus_atoms: u64,
    pub operational_buffer_atoms: u64,
    pub series_count: u8,
    pub status: WriterSleeveStatus,
    pub security_mode: WriterSecurityMode,
    pub settlement_finalized_slot: u64,
    pub last_updated_slot: u64,
    pub capital_seconds: u128,
    pub maximum_contribution_duration: u64,
    pub participation_start_ts: u64,
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
            security_exposure_atoms: 0,
            long_liability_initial_atoms: 0,
            long_liability_remaining_atoms: 0,
            writer_residual_initial_atoms: 0,
            writer_residual_remaining_atoms: 0,
            settlement_principal_atoms: 0,
            unclaimed_principal_atoms: 0,
            stranded_surplus_atoms: 0,
            operational_buffer_atoms: 0,
            series_count: 0,
            status: WriterSleeveStatus::default(),
            security_mode: WriterSecurityMode::default(),
            settlement_finalized_slot: 0,
            last_updated_slot: 0,
            capital_seconds: 0,
            maximum_contribution_duration: 0,
            participation_start_ts: 0,
        }
    }
}

impl WriterSleeveV1 {
    pub const LEN: usize = 545;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"WSL";
    pub const ACCOUNT_VERSION: u8 = 3;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && self.participation_layout_valid()
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
