use super::*;

pub const ORACLE_RECIPE_SOURCE_INDEX_SEED: &[u8] = b"oracle-recipe-source-index";
pub const ORACLE_BUCKET_SOURCE_INDEX_SEED: &[u8] = b"oracle-bucket-source-index";

/// Permissionless reverse verification of the existing frozen recipe hash chain.
/// Completion makes this record and every derived bucket index immutable.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleRecipeSourceIndex {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub recipe_hash: [u8; 32],
    pub manifest_hash: [u8; 32],
    pub remaining_hash: [u8; 32],
    pub expected_source_count: u16,
    pub expected_bucket_count: u16,
    pub remaining_source_count: u16,
    pub indexed_bucket_count: u16,
    pub indexed_bucket_weight_bps: u16,
    pub last_bucket_id: [u8; 32],
    pub last_source_id: [u8; 32],
    pub complete: bool,
}

impl OracleRecipeSourceIndex {
    pub const LEN: usize = 209;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"ORI";
    pub const ACCOUNT_VERSION: u8 = 1;
}

/// Exact ascending membership, authenticated against the unchanged frozen recipe.
/// Consumers require the matching recipe source index to be complete.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleBucketSourceIndex {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub recipe_hash: [u8; 32],
    pub bucket_id: [u8; 32],
    pub group_index: u16,
    pub first_source_index: u16,
    pub bucket_weight_bps: u16,
    pub source_count: u16,
    pub source_ids: [[u8; 32]; crate::constants::MAX_ORACLE_BUCKET_SOURCES],
}

impl OracleBucketSourceIndex {
    pub const LEN: usize = 110 + 32 * crate::constants::MAX_ORACLE_BUCKET_SOURCES;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OBI";
    pub const ACCOUNT_VERSION: u8 = 1;
}

pub fn derive_oracle_recipe_source_index_pda(program_id: &Pubkey, month: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_RECIPE_SOURCE_INDEX_SEED,
            month.as_ref(),
        ],
        program_id,
    )
}

pub fn derive_oracle_bucket_source_index_pda(
    program_id: &Pubkey,
    month: &Pubkey,
    bucket_id: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_BUCKET_SOURCE_INDEX_SEED,
            month.as_ref(),
            bucket_id,
        ],
        program_id,
    )
}
