use super::*;

pub const WRITER_DLMM_POLICY_SEED: &[u8] = b"writer-dlmm-policy";
pub const WRITER_DLMM_POSITION_SEED: &[u8] = b"writer-dlmm-position";
pub const WRITER_DLMM_POSITION_BINS: usize = 32;
pub const WRITER_DLMM_ACTION_ENTRIES: usize = 8;
pub const WRITER_DLMM_POLICY_HASH_DOMAIN: &[u8] = b"ameba-writer-dlmm-policy-v1";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WriterDlmmSeriesPolicyV1 {
    pub conservative_claim_value_atoms: u64,
    pub seller_floor_quote_atoms: u64,
    pub monthly_buyback_cap_atoms: u64,
    pub transaction_buyback_cap_atoms: u64,
}

impl WriterDlmmSeriesPolicyV1 {
    pub const LEN: usize = 32;
}

/// Immutable signed terms and separately maintained spending/custody counters.
/// `sealed` is a create-once lifecycle state, never an operational feature switch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WriterDlmmPolicyV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub sleeve: Pubkey,
    pub policy_snapshot: Pubkey,
    pub management_authority: Pubkey,
    pub committing_policy_authority: Pubkey,
    pub expected_policy_hash: [u8; 32],
    pub rolling_policy_hash: [u8; 32],
    pub monthly_buyback_cap_atoms: u64,
    pub transaction_buyback_cap_atoms: u64,
    pub reserve_release_spend_ratio_ppm: u64,
    pub price_separation_ticks: u16,
    pub series_count: u8,
    pub appended_series_count: u8,
    pub sealed: bool,
    pub created_slot: u64,
    pub sealed_slot: u64,
    /// First UTC calendar day, 00:00:00, of the accounted spending month.
    pub spending_month_start_ts: u64,
    pub monthly_spent_atoms: u64,
    /// Sum of writer-owned committed and uncommitted quote in all canonical pools.
    pub total_pool_quote_atoms: u64,
    pub total_uncommitted_quote_atoms: u64,
    pub series: [WriterDlmmSeriesPolicyV1; WRITER_SERIES_STORAGE_CAPACITY],
    pub series_monthly_spent_atoms: [u64; WRITER_SERIES_STORAGE_CAPACITY],
    pub series_pool_inventory_atoms: [u64; WRITER_SERIES_STORAGE_CAPACITY],
    pub reserved: [u8; 32],
}

impl WriterDlmmPolicyV1 {
    pub const LEN: usize = 6
        + 6 * 32
        + 3 * 8
        + 2
        + 3
        + 6 * 8
        + WRITER_SERIES_STORAGE_CAPACITY * (WriterDlmmSeriesPolicyV1::LEN + 2 * 8)
        + 32;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"WLP";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_current_layout(&self) -> bool {
        let count = usize::from(self.series_count);
        let appended = usize::from(self.appended_series_count);
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
            && count > 0
            && count <= WRITER_LIVE_SERIES_LIMIT
            && appended <= count
            && (!self.sealed
                || (appended == count && self.rolling_policy_hash == self.expected_policy_hash))
            && self.total_uncommitted_quote_atoms <= self.total_pool_quote_atoms
            && self
                .series_monthly_spent_atoms
                .iter()
                .try_fold(0u64, |sum, value| sum.checked_add(*value))
                == Some(self.monthly_spent_atoms)
            && self.monthly_spent_atoms <= self.monthly_buyback_cap_atoms
            && self
                .series_monthly_spent_atoms
                .iter()
                .zip(self.series.iter())
                .all(|(spent, terms)| *spent <= terms.monthly_buyback_cap_atoms)
            && self.series[appended..]
                .iter()
                .all(|entry| *entry == WriterDlmmSeriesPolicyV1::default())
            && self.series_monthly_spent_atoms[count..]
                .iter()
                .all(|value| *value == 0)
            && self.series_pool_inventory_atoms[count..]
                .iter()
                .all(|value| *value == 0)
            && self.reserved == [0; 32]
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WriterDlmmBinV1 {
    pub bin_id: u16,
    pub option_atoms: u64,
    pub quote_atoms: u64,
}

impl WriterDlmmBinV1 {
    pub const LEN: usize = 18;
}

/// Sole writer owner of its segregated lane in one existing canonical pool.
/// Sale proceeds are uncommitted until a permitted placement below the sale floor.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WriterDlmmPositionV1 {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub pool: Pubkey,
    pub sleeve: Pubkey,
    pub policy: Pubkey,
    pub market: Pubkey,
    pub series_index: u8,
    pub bin_count: u8,
    pub uncommitted_quote_atoms: u64,
    pub option_inventory_atoms: u64,
    pub allocated_quote_atoms: u64,
    pub last_updated_slot: u64,
    pub bins: [WriterDlmmBinV1; WRITER_DLMM_POSITION_BINS],
    pub reserved: [u8; 32],
}

impl WriterDlmmPositionV1 {
    pub const LEN: usize =
        6 + 4 * 32 + 2 + 4 * 8 + WRITER_DLMM_POSITION_BINS * WriterDlmmBinV1::LEN + 32;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"WDP";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_current_layout(&self) -> bool {
        let count = usize::from(self.bin_count);
        if self.account_discriminator != Self::ACCOUNT_DISCRIMINATOR
            || self.account_version != Self::ACCOUNT_VERSION
            || usize::from(self.series_index) >= WRITER_LIVE_SERIES_LIMIT
            || count > WRITER_DLMM_POSITION_BINS
            || self.reserved != [0; 32]
            || self.bins[count..]
                .iter()
                .any(|bin| *bin != WriterDlmmBinV1::default())
        {
            return false;
        }
        let mut options = 0u64;
        let mut quote = 0u64;
        let mut previous = None;
        for bin in &self.bins[..count] {
            if bin.bin_id == 0
                || bin.bin_id > crate::constants::MAX_AMOEBA_DLMM_BIN_COUNT
                || previous.is_some_and(|value| value >= bin.bin_id)
                || (bin.option_atoms == 0 && bin.quote_atoms == 0)
            {
                return false;
            }
            previous = Some(bin.bin_id);
            let Some(next_options) = options.checked_add(bin.option_atoms) else {
                return false;
            };
            let Some(next_quote) = quote.checked_add(bin.quote_atoms) else {
                return false;
            };
            options = next_options;
            quote = next_quote;
        }
        options == self.option_inventory_atoms && quote == self.allocated_quote_atoms
    }
}

pub fn derive_writer_dlmm_policy_pda(program_id: &Pubkey, sleeve: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            WRITER_DLMM_POLICY_SEED,
            sleeve.as_ref(),
        ],
        program_id,
    )
}

pub fn derive_writer_dlmm_position_pda(
    program_id: &Pubkey,
    pool: &Pubkey,
    sleeve: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            WRITER_DLMM_POSITION_SEED,
            pool.as_ref(),
            sleeve.as_ref(),
        ],
        program_id,
    )
}
