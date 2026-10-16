use super::*;
use borsh::BorshSerialize;

mod account_loading;
mod config_governance;
mod opening_settlement;
mod product_sku;
mod schedule;
mod settlement;
mod sku_coverage;
mod source_weights;
mod stale_cleanup;

fn with_test_account_info<R>(
    key: &Pubkey,
    owner: &Pubkey,
    mut data: Vec<u8>,
    f: impl FnOnce(&AccountInfo) -> R,
) -> R {
    let mut lamports = 0;
    let account = AccountInfo::new(key, false, false, &mut lamports, &mut data, owner, false, 0);
    f(&account)
}

fn exact_test_state_data<T: BorshSerialize>(value: &T, len: usize) -> Vec<u8> {
    let serialized = value.try_to_vec().unwrap();
    assert!(serialized.len() <= len);
    let mut data = vec![0; len];
    data[..serialized.len()].copy_from_slice(&serialized);
    data
}

fn maturity_ladder_fixture(
    planned_listing_ts: u64,
    planned_expiry_ts: u64,
) -> OracleMaturityLadderRegistry {
    OracleMaturityLadderRegistry {
        is_initialized: true,
        account_discriminator: OracleMaturityLadderRegistry::ACCOUNT_DISCRIMINATOR,
        account_version: OracleMaturityLadderRegistry::ACCOUNT_VERSION,
        underlying_id: [7; 32],
        planned_listing_ts,
        planned_expiry_ts,
        ..OracleMaturityLadderRegistry::default()
    }
}

fn synthetic_ram_product_sku_draft() -> OracleProductSkuDraft {
    let mut underlying_id = [0u8; 32];
    underlying_id[..4].copy_from_slice(b"RAMX");
    OracleProductSkuDraft {
        is_initialized: true,
        account_discriminator: OracleProductSkuDraft::ACCOUNT_DISCRIMINATOR,
        account_version: OracleProductSkuDraft::ACCOUNT_VERSION,
        underlying_id,
        expected_sku_count: 52,
        expected_sku_root: [0xab; 32],
        ..OracleProductSkuDraft::default()
    }
}

/// A Forming period anchor with the given expiry, for helpers that read only the anchor.
pub(in crate::processor) fn sample_anchor(expiry_ts: u64) -> PeriodAnchor {
    let mut product_id = [0u8; 32];
    product_id[..4].copy_from_slice(b"RAMX");
    let planned_listing_ts = expiry_ts.saturating_sub(90 * 86_400);
    PeriodAnchor {
        underlying_id: product_id,
        expiry_ts,
        period: Period::new_forming(255, product_id, planned_listing_ts, expiry_ts, 6),
    }
}
