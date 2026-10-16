use super::*;

/// Immutable period/month-scoped settlement provenance, created once when the month settles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettlementRecordV2 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub period: Pubkey,
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
            period: Pubkey::default(),
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

/// One record per period and month: `[namespace, "settlement_v2", period, month]`.
pub fn derive_settlement_v2_pda(
    program_id: &Pubkey,
    period: &Pubkey,
    oracle_month: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::SETTLEMENT_V2_PDA_SEED,
            period.as_ref(),
            oracle_month.as_ref(),
        ],
        program_id,
    )
}
