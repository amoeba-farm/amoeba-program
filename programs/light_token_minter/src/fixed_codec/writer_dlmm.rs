use super::*;
use crate::state::{
    WriterDlmmBinV1, WriterDlmmPolicyV1, WriterDlmmPositionV1, WriterDlmmSeriesPolicyV1,
    WRITER_DLMM_POSITION_BINS,
};

impl FixedField for [WriterDlmmSeriesPolicyV1; WRITER_SERIES_STORAGE_CAPACITY] {
    #[inline(always)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        core::array::from_fn(|_| WriterDlmmSeriesPolicyV1::read(input))
    }
    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        for entry in self {
            entry.write(output);
        }
    }
}

impl FixedField for [WriterDlmmBinV1; WRITER_DLMM_POSITION_BINS] {
    #[inline(always)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        core::array::from_fn(|_| WriterDlmmBinV1::read(input))
    }
    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        for bin in self {
            bin.write(output);
        }
    }
}

fixed_state_deserialize!(WriterDlmmSeriesPolicyV1, WriterDlmmSeriesPolicyV1::LEN, {
    conservative_claim_value_atoms: u64,
    seller_floor_quote_atoms: u64,
    monthly_buyback_cap_atoms: u64,
    transaction_buyback_cap_atoms: u64,
});

fixed_state_deserialize!(WriterDlmmPolicyV1, WriterDlmmPolicyV1::LEN, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    sleeve: Pubkey,
    policy_snapshot: Pubkey,
    management_authority: Pubkey,
    committing_policy_authority: Pubkey,
    expected_policy_hash: [u8; 32],
    rolling_policy_hash: [u8; 32],
    monthly_buyback_cap_atoms: u64,
    transaction_buyback_cap_atoms: u64,
    reserve_release_spend_ratio_ppm: u64,
    price_separation_ticks: u16,
    series_count: u8,
    appended_series_count: u8,
    sealed: bool,
    created_slot: u64,
    sealed_slot: u64,
    spending_month_start_ts: u64,
    monthly_spent_atoms: u64,
    total_pool_quote_atoms: u64,
    total_uncommitted_quote_atoms: u64,
    series: [WriterDlmmSeriesPolicyV1; WRITER_SERIES_STORAGE_CAPACITY],
    series_monthly_spent_atoms: [u64; WRITER_SERIES_STORAGE_CAPACITY],
    series_pool_inventory_atoms: [u64; WRITER_SERIES_STORAGE_CAPACITY],
    reserved: [u8; 32],
});

fixed_state_deserialize!(WriterDlmmBinV1, WriterDlmmBinV1::LEN, {
    bin_id: u16,
    option_atoms: u64,
    quote_atoms: u64,
});

fixed_state_deserialize!(WriterDlmmPositionV1, WriterDlmmPositionV1::LEN, {
    is_initialized: bool,
    bump: u8,
    account_discriminator: [u8; 3],
    account_version: u8,
    pool: Pubkey,
    sleeve: Pubkey,
    policy: Pubkey,
    market: Pubkey,
    series_index: u8,
    bin_count: u8,
    uncommitted_quote_atoms: u64,
    option_inventory_atoms: u64,
    allocated_quote_atoms: u64,
    last_updated_slot: u64,
    bins: [WriterDlmmBinV1; WRITER_DLMM_POSITION_BINS],
    reserved: [u8; 32],
});
