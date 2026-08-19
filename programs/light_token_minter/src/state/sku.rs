use super::*;

/// Number of fixed nodes needed to stream a Merkle tree containing at most 256 leaves. Levels
/// zero through seven hold incomplete subtrees; level eight holds a completed 256-leaf root.
pub const ORACLE_PRODUCT_SKU_FRONTIER_NODE_COUNT: usize =
    crate::constants::MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH + 1;

/// Nonce-scoped, abandonable staging account for one governed product-SKU manifest attempt.
///
/// Only the incremental Merkle frontier and the final identifier needed for strict ordering are
/// retained. The immutable manifest is created on the exact final append, after which this scratch
/// account is closed atomically and its rent is returned to the config admin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleProductSkuDraft {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub underlying_id: [u8; 32],
    pub draft_nonce: u64,
    pub expected_sku_count: u16,
    pub appended_sku_count: u16,
    /// Occupancy bits for `merkle_frontier`. Canonically equal to `appended_sku_count` because the
    /// occupied subtree levels are exactly the set bits in the binary leaf count.
    pub frontier_mask: u16,
    pub last_sku_id: [u8; 32],
    pub merkle_frontier: [[u8; 32]; ORACLE_PRODUCT_SKU_FRONTIER_NODE_COUNT],
    pub last_updated_slot: u64,
}

impl Default for OracleProductSkuDraft {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            underlying_id: [0; 32],
            draft_nonce: 0,
            expected_sku_count: 0,
            appended_sku_count: 0,
            frontier_mask: 0,
            last_sku_id: [0; 32],
            merkle_frontier: [[0; 32]; ORACLE_PRODUCT_SKU_FRONTIER_NODE_COUNT],
            last_updated_slot: 0,
        }
    }
}

impl OracleProductSkuDraft {
    pub const LEN: usize = 380;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OPD";
    pub const ACCOUNT_VERSION: u8 = 2;

    pub fn has_canonical_layout(&self) -> bool {
        if self.account_discriminator != Self::ACCOUNT_DISCRIMINATOR
            || self.account_version != Self::ACCOUNT_VERSION
            || self.expected_sku_count == 0
            || self.expected_sku_count > crate::constants::MAX_ORACLE_REQUIRED_SKUS
            || self.appended_sku_count > self.expected_sku_count
            || self.frontier_mask != self.appended_sku_count
            || (self.appended_sku_count == 0) != (crate::bytes32_is_zero(&self.last_sku_id))
        {
            return false;
        }
        self.merkle_frontier
            .iter()
            .enumerate()
            .all(|(level, node)| {
                self.frontier_mask & (1u16 << level) != 0 || crate::bytes32_is_zero(&*node)
            })
    }
}

pub fn derive_oracle_product_sku_draft_pda(
    program_id: &Pubkey,
    underlying_id: &[u8; 32],
    draft_nonce: u64,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_PRODUCT_SKU_DRAFT_PDA_SEED,
            underlying_id,
            &draft_nonce.to_le_bytes(),
        ],
        program_id,
    )
}

/// Create-once product-level commitment to the complete canonical terminal-SKU set.
///
/// A month may copy a root/count only from this exact PDA. Governance supplies the full ordered
/// identifier set through bounded chunks in a nonce-scoped OPD. The exact final append checks the
/// known product count, rejects zero, duplicate, and out-of-order identifiers, and computes this
/// root on chain before creating the OPM atomically.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleProductSkuManifest {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub underlying_id: [u8; 32],
    pub required_sku_root: [u8; 32],
    pub required_sku_count: u16,
    pub reserved: [u8; 6],
    pub last_updated_slot: u64,
}

impl Default for OracleProductSkuManifest {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            underlying_id: [0; 32],
            required_sku_root: [0; 32],
            required_sku_count: 0,
            reserved: [0; 6],
            last_updated_slot: 0,
        }
    }
}

impl OracleProductSkuManifest {
    pub const LEN: usize = 96;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OPM";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_canonical_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && self.reserved == [0; 6]
    }
}

pub fn derive_oracle_product_sku_manifest_pda(
    program_id: &Pubkey,
    underlying_id: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_PRODUCT_SKU_MANIFEST_PDA_SEED,
            underlying_id,
        ],
        program_id,
    )
}

/// Immutable required-SKU commitment plus the live coverage latch for one oracle month.
///
/// `required_sku_root` commits to ordered, index-bound SKU leaves. `covered_sku_count` changes only
/// when a canonical per-SKU record crosses between zero and one supported candidate. The manifest
/// can be finalized only at exact coverage, and is reopened if the last supported source for any
/// SKU is rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleSkuCoverageManifest {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub required_sku_root: [u8; 32],
    pub required_sku_count: u16,
    pub covered_sku_count: u16,
    pub planned_scramble_start_ts: u64,
    pub planned_listing_ts: u64,
    pub coverage_finalized: bool,
    pub coverage_complete_ts: u64,
    pub last_updated_slot: u64,
}

impl Default for OracleSkuCoverageManifest {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            month: Pubkey::default(),
            required_sku_root: [0; 32],
            required_sku_count: 0,
            covered_sku_count: 0,
            planned_scramble_start_ts: 0,
            planned_listing_ts: 0,
            coverage_finalized: false,
            coverage_complete_ts: 0,
            last_updated_slot: 0,
        }
    }
}

impl OracleSkuCoverageManifest {
    pub const LEN: usize = 128;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OSC";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_canonical_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
    }
}

pub fn derive_oracle_sku_coverage_manifest_pda(
    program_id: &Pubkey,
    month: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_SKU_COVERAGE_MANIFEST_PDA_SEED,
            month.as_ref(),
        ],
        program_id,
    )
}

/// Live supported-candidate count for one committed terminal SKU.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleSkuCoverageRecord {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub sku_id: [u8; 32],
    pub sku_index: u16,
    pub active_supported_source_count: u16,
    pub last_updated_slot: u64,
}

impl Default for OracleSkuCoverageRecord {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            month: Pubkey::default(),
            sku_id: [0; 32],
            sku_index: 0,
            active_supported_source_count: 0,
            last_updated_slot: 0,
        }
    }
}

impl OracleSkuCoverageRecord {
    pub const LEN: usize = 96;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OSK";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_canonical_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
    }
}

pub fn derive_oracle_sku_coverage_record_pda(
    program_id: &Pubkey,
    month: &Pubkey,
    sku_id: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_SKU_COVERAGE_RECORD_PDA_SEED,
            month.as_ref(),
            sku_id,
        ],
        program_id,
    )
}
