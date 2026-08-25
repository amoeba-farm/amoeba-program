use super::*;

/// Separate one-shot authorization account. No field is embedded in an ordinary program account.
#[derive(BorshDeserialize, BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct DevnetSoloBackfill2026V1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub vault_config: Pubkey,
    pub admin: Pubkey,
    pub oracle_authority: Pubkey,
    pub initialized_slot: u64,
    pub initialized_at_ts: u64,
    pub sunset_ts: u64,
    /// RAMX-202609, NANDX-202609, RAMX-202610, NANDX-202610.
    pub consumed_cohort_mask: u8,
    pub completed_cohort_mask: u8,
    /// Two bits per cohort, call then put.
    pub game_market_mask: u8,
    pub reserved: u8,
    /// Chain-clock placement starts, indexed by the exact cohort order above.
    pub cohort_start_ts: [u64; 4],
    pub last_updated_slot: u64,
}

impl DevnetSoloBackfill2026V1 {
    pub const LEN: usize = 170;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"DSB";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_canonical_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && self.reserved == 0
    }
}

pub const DEVNET_SOLO_BACKFILL_2026_V1_PDA_SEED: &[u8] = b"devnet-solo-backfill-2026-v1";

pub fn derive_devnet_solo_backfill_2026_v1_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            DEVNET_SOLO_BACKFILL_2026_V1_PDA_SEED,
        ],
        program_id,
    )
}
