use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleRecipeWeightPhase {
    #[default]
    Collecting = 0,
    ReadyToFinalize = 2,
    Finalized = 3,
}
stable_borsh_enum!(OracleRecipeWeightPhase { Collecting = 0, ReadyToFinalize = 2, Finalized = 3 });

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleRecipeWeightManifest {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub phase: OracleRecipeWeightPhase,
    pub expected_source_count: u16,
    pub expected_bucket_count: u16,
    pub recipe_hash: [u8; 32],
    pub rolling_manifest_hash: [u8; 32],
    /// Current bucket during collection; after completion it is the last completed bucket.
    pub current_bucket_id: [u8; 32],
    pub current_bucket_weight_bps: u16,
    pub current_bucket_source_count: u16,
    pub processed_source_count: u16,
    pub processed_bucket_count: u16,
    pub declared_weight_total_bps: u16,
    pub last_collected_source_id: [u8; 32],
}

impl Default for OracleRecipeWeightManifest {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            month: Pubkey::default(),
            phase: OracleRecipeWeightPhase::Collecting,
            expected_source_count: 0,
            expected_bucket_count: 0,
            recipe_hash: [0; 32],
            rolling_manifest_hash: [0; 32],
            current_bucket_id: [0; 32],
            current_bucket_weight_bps: 0,
            current_bucket_source_count: 0,
            processed_source_count: 0,
            processed_bucket_count: 0,
            declared_weight_total_bps: 0,
            last_collected_source_id: [0; 32],
        }
    }
}

impl OracleRecipeWeightManifest {
    pub const LEN: usize = 192;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"ORW";
    pub const ACCOUNT_VERSION: u8 = 2;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
    }
}

pub fn derive_oracle_recipe_weight_manifest_pda(
    program_id: &Pubkey,
    month: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_RECIPE_WEIGHT_MANIFEST_PDA_SEED,
            month.as_ref(),
        ],
        program_id,
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleSettlementSourceManifest {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub phase: OracleRecipeWeightPhase,
    pub expected_source_count: u16,
    pub expected_bucket_count: u16,
    pub processed_source_count: u16,
    pub processed_bucket_count: u16,
    pub declared_weight_total_bps: u16,
    pub current_bucket_id: [u8; 32],
    pub rolling_source_digest: [u8; 32],
}

impl Default for OracleSettlementSourceManifest {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            month: Pubkey::default(),
            phase: OracleRecipeWeightPhase::Collecting,
            expected_source_count: 0,
            expected_bucket_count: 0,
            processed_source_count: 0,
            processed_bucket_count: 0,
            declared_weight_total_bps: 0,
            current_bucket_id: [0; 32],
            rolling_source_digest: [0; 32],
        }
    }
}

impl OracleSettlementSourceManifest {
    pub const LEN: usize = 128;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OSD";
    pub const ACCOUNT_VERSION: u8 = 4;
}

pub fn derive_oracle_settlement_source_manifest_pda(
    program_id: &Pubkey,
    month: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_SETTLEMENT_SOURCE_MANIFEST_PDA_SEED,
            month.as_ref(),
        ],
        program_id,
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleActiveWeightManifest {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub phase: OracleRecipeWeightPhase,
    pub expected_source_count: u16,
    pub expected_group_count: u16,
    pub processed_source_count: u16,
    pub processed_group_count: u16,
    pub current_group_id: [u8; 32],
    pub current_group_source_count: u16,
    pub current_group_active_count: u16,
    pub current_group_bucket_weight_bps: u16,
    pub processed_bucket_weight_bps: u16,
    pub last_collected_source_id: [u8; 32],
    pub rolling_manifest_hash: [u8; 32],
    pub max_open_interest_payout: u64,
}

impl Default for OracleActiveWeightManifest {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            month: Pubkey::default(),
            phase: OracleRecipeWeightPhase::Collecting,
            expected_source_count: 0,
            expected_group_count: 0,
            processed_source_count: 0,
            processed_group_count: 0,
            current_group_id: [0; 32],
            current_group_source_count: 0,
            current_group_active_count: 0,
            current_group_bucket_weight_bps: 0,
            processed_bucket_weight_bps: 0,
            last_collected_source_id: [0; 32],
            rolling_manifest_hash: [0; 32],
            max_open_interest_payout: 0,
        }
    }
}

impl OracleActiveWeightManifest {
    pub const LEN: usize = 160;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OAW";
    pub const ACCOUNT_VERSION: u8 = 3;
}

pub fn derive_oracle_active_weight_manifest_pda(
    program_id: &Pubkey,
    month: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_ACTIVE_WEIGHT_MANIFEST_PDA_SEED,
            month.as_ref(),
        ],
        program_id,
    )
}
