use super::*;

/// Canonical program authority for the separate classic-SPL USDC reward ATA.
///
/// This custody is never mixed with the AMBA treasury or sAMBA backing. `total_reserved`
/// is the outstanding external liability across frozen cash schedules; claims decrease it
/// while moving the same amount into the primary collateral vault and player ledger.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleUsdcRewardVault {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub mint: Pubkey,
    pub token_account: Pubkey,
    pub total_reserved: u64,
    pub total_paid: u64,
    pub last_updated_slot: u64,
}

impl OracleUsdcRewardVault {
    pub const LEN: usize = 192;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"URV";
    pub const ACCOUNT_VERSION: u8 = 1;
}

pub fn derive_oracle_usdc_reward_vault_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_USDC_REWARD_VAULT_PDA_SEED,
        ],
        program_id,
    )
}

/// Month-scoped finite USDC reservation. SKU promises are frozen before Scramble; entitlement
/// counts freeze only after canonical month finality.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleUsdcRewardSchedule {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub authority: Pubkey,
    pub reward_vault: Pubkey,
    pub phase: OracleUsdcRewardSchedulePhase,
    pub sku_pool_count: u16,
    pub registered_source_count: u32,
    pub registered_opening_count: u32,
    pub registered_update_count: u32,
    pub registered_update_reward_units: u32,
    pub total_reward_budget: u64,
    pub remaining_reward_budget: u64,
    pub last_updated_slot: u64,
    /// Exact count of V5 listing, support, and source-challenge escrows which have been created
    /// but not yet reconciled through the schedule-aware settlement lane.
    pub outstanding_prelisting_escrow_count: u32,
    /// Trading-fee-derived additions to SKU update-bounty pools.
    pub trading_fee_bounty_total: u64,
    /// Set exactly once by the settled DLMM bounty transfer, including a zero-fee month.
    pub bounty_fee_sweep_finalized: bool,
}

impl OracleUsdcRewardSchedule {
    pub const LEN: usize = 160;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"URS";
    pub const ACCOUNT_VERSION: u8 = 2;
}

pub fn derive_oracle_usdc_reward_schedule_pda(program_id: &Pubkey, month: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_USDC_REWARD_SCHEDULE_PDA_SEED,
            month.as_ref(),
        ],
        program_id,
    )
}

/// One SKU/bucket's exact finite rewards and exact ordinary-oracle bond policy.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleUsdcSkuPool {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub schedule: Pubkey,
    pub month: Pubkey,
    pub bucket_id: [u8; 32],
    pub source_reward_budget: u64,
    pub remaining_source_reward_budget: u64,
    pub opening_reward_budget: u64,
    pub remaining_opening_reward_budget: u64,
    pub update_reward_budget: u64,
    pub remaining_update_reward_budget: u64,
    pub proposer_reward_bps: u16,
    pub listing_bond: u64,
    pub support_bond: u64,
    pub opening_bond: u64,
    pub update_min_bond: u64,
    pub challenge_min_bond: u64,
    pub challenge_max_bond: u64,
    pub challenge_bond_bps: u16,
    pub registered_source_count: u32,
    pub registered_opening_count: u32,
    pub registered_update_count: u32,
    pub registered_update_reward_units: u32,
    pub last_updated_slot: u64,
}

impl OracleUsdcSkuPool {
    pub const LEN: usize = 240;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"URP";
    pub const ACCOUNT_VERSION: u8 = 2;

    pub fn total_reward_budget(&self) -> Option<u64> {
        self.source_reward_budget
            .checked_add(self.opening_reward_budget)?
            .checked_add(self.update_reward_budget)
    }
}

pub fn derive_oracle_usdc_sku_pool_pda(
    program_id: &Pubkey,
    schedule: &Pubkey,
    bucket_id: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_USDC_SKU_POOL_PDA_SEED,
            schedule.as_ref(),
            bucket_id,
        ],
        program_id,
    )
}

/// Source-local contributor count and post-OAW eligibility marker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleUsdcSourceReward {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub schedule: Pubkey,
    pub sku_pool: Pubkey,
    pub source: Pubkey,
    pub source_id: [u8; 32],
    pub proposer: Pubkey,
    pub supporter_count: u32,
    pub registered: bool,
    pub terminal_status: OracleSourceStatus,
    pub opening_claim: Pubkey,
    pub last_updated_slot: u64,
    /// Direct parent in the bounded merge lineage. Zero means this source is the current root.
    pub merged_into_source: Pubkey,
    /// Longest supporter-origin path currently absorbed by this source.
    pub max_merge_depth: u8,
    /// Makes schedule accounting for this source's listing bond exactly once.
    pub listing_escrow_counted: bool,
}

impl Default for OracleUsdcSourceReward {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            month: Pubkey::default(),
            schedule: Pubkey::default(),
            sku_pool: Pubkey::default(),
            source: Pubkey::default(),
            source_id: [0; 32],
            proposer: Pubkey::default(),
            supporter_count: 0,
            registered: false,
            terminal_status: OracleSourceStatus::Candidate,
            opening_claim: Pubkey::default(),
            last_updated_slot: 0,
            merged_into_source: Pubkey::default(),
            max_merge_depth: 0,
            listing_escrow_counted: false,
        }
    }
}

impl OracleUsdcSourceReward {
    pub const LEN: usize = 288;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"URC";
    pub const ACCOUNT_VERSION: u8 = 1;
}

pub fn derive_oracle_usdc_source_reward_pda(
    program_id: &Pubkey,
    schedule: &Pubkey,
    source: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_USDC_SOURCE_REWARD_PDA_SEED,
            schedule.as_ref(),
            source.as_ref(),
        ],
        program_id,
    )
}

/// Unique counted entitlement subject used for update and emergency completeness.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleUsdcRewardRegistration {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub schedule: Pubkey,
    pub sku_pool: Pubkey,
    pub kind: OracleUsdcRewardKind,
    pub subject: Pubkey,
    pub recipient: Pubkey,
    pub reward_units: u8,
    pub last_updated_slot: u64,
}

impl OracleUsdcRewardRegistration {
    pub const LEN: usize = 224;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"URG";
    pub const ACCOUNT_VERSION: u8 = 2;
}

pub fn derive_oracle_usdc_reward_registration_pda(
    program_id: &Pubkey,
    schedule: &Pubkey,
    kind: OracleUsdcRewardKind,
    subject: &Pubkey,
) -> (Pubkey, u8) {
    let kind_seed = [kind as u8];
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_USDC_REWARD_REGISTRATION_PDA_SEED,
            crate::constants::ORACLE_USDC_REWARD_REGISTRATION_VERSION_SEED,
            schedule.as_ref(),
            &kind_seed,
            subject.as_ref(),
        ],
        program_id,
    )
}

/// Unique paid receipt in a cash-only replay domain.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleUsdcRewardReceipt {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub schedule: Pubkey,
    pub recipient: Pubkey,
    pub kind: OracleUsdcRewardKind,
    pub subject: Pubkey,
    pub amount: u64,
    pub claimed_slot: u64,
}

impl OracleUsdcRewardReceipt {
    pub const LEN: usize = 224;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"URR";
    pub const ACCOUNT_VERSION: u8 = 1;
}

pub fn derive_oracle_usdc_reward_receipt_pda(
    program_id: &Pubkey,
    schedule: &Pubkey,
    kind: OracleUsdcRewardKind,
    subject: &Pubkey,
    recipient: &Pubkey,
) -> (Pubkey, u8) {
    let kind_seed = [kind as u8];
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_USDC_REWARD_RECEIPT_PDA_SEED,
            schedule.as_ref(),
            &kind_seed,
            subject.as_ref(),
            recipient.as_ref(),
        ],
        program_id,
    )
}
