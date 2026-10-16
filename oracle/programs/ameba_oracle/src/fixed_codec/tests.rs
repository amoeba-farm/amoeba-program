use borsh::BorshSerialize;
use solana_program::pubkey::Pubkey;

use super::decode_oracle_month as decode_month_inner;
use crate::error::OracleError;
use crate::state::{OracleMonthState, OraclePhase, OracleSettlementStatus};

fn encoded_month(mut month: OracleMonthState) -> Vec<u8> {
    let mut data = month.try_to_vec().unwrap();
    data.resize(OracleMonthState::LEN, 0);
    // Keep ownership local so callers cannot accidentally compare against a later mutation.
    month.accepted_cash_update_count = month.accepted_cash_update_count.wrapping_add(1);
    data
}

fn decode_oracle_month(data: &[u8]) -> Result<OracleMonthState, OracleError> {
    decode_month_inner(data, OracleError::InvalidOracleMonthAccount)
}

#[test]
fn current_month_wire_layout_preserves_the_live_299_byte_shape() {
    let month = OracleMonthState {
        is_initialized: true,
        account_discriminator: OracleMonthState::ACCOUNT_DISCRIMINATOR,
        account_version: OracleMonthState::ACCOUNT_VERSION,
        phase: OraclePhase::SourceSubmission,
        effective_weight_total_bps: 0x1234,
        weight_manifest_hash: [0x51; 32],
        candidate_count_tracking_version: OracleMonthState::CANDIDATE_COUNT_TRACKING_VERSION,
        active_weight_initialization_version:
            OracleMonthState::ACTIVE_WEIGHT_INITIALIZATION_VERSION,
        active_weight_scheme_version: OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION,
        active_weight_group_count: 0x5678,
        active_weight_manifest_hash: [0x91; 32],
        schedule_version: OracleMonthState::SKU_COVERAGE_SCHEDULE_VERSION,
        work_reward_currency_version: OracleMonthState::WORK_REWARD_CURRENCY_USDC_V1,
        accepted_cash_update_count: 0xa1b2_c3d4,
        ..OracleMonthState::default()
    };
    let serialized = month.try_to_vec().unwrap();

    // A None settlement record serializes to 267 bytes and occupies the canonical 299-byte
    // account with 32 bytes of zero padding. These offsets are the already-live current ABI.
    assert_eq!(serialized.len(), 267);
    assert_eq!(&serialized[190..192], &0x1234u16.to_le_bytes());
    assert_eq!(&serialized[192..224], &[0x51; 32]);
    assert_eq!(serialized[224], 1);
    assert_eq!(serialized[225], 1);
    assert_eq!(serialized[226], 2);
    assert_eq!(&serialized[227..229], &0x5678u16.to_le_bytes());
    assert_eq!(&serialized[229..261], &[0x91; 32]);
    assert_eq!(serialized[261], 2);
    assert_eq!(serialized[262], 1);
    assert_eq!(&serialized[263..267], &0xa1b2_c3d4u32.to_le_bytes());

    let account_data = encoded_month(month.clone());
    assert_eq!(account_data.len(), OracleMonthState::LEN);
    assert!(account_data[267..].iter().all(|byte| *byte == 0));
    let decoded = decode_oracle_month(&account_data).unwrap();
    assert_eq!(decoded, month);
    assert!(decoded.has_current_layout());
}

#[test]
fn month_decoder_matches_borsh_for_maximum_and_padded_options() {
    for settlement_record in [None, Some(Pubkey::new_unique())] {
        let month = OracleMonthState {
            is_initialized: true,
            bump: 9,
            phase: OraclePhase::SourceSubmission,
            settlement_status: OracleSettlementStatus::Provisional,
            settlement_record,
            recipe_hash: [3; 32],
            weight_manifest_hash: [4; 32],
            active_weight_manifest_hash: [5; 32],
            accepted_cash_update_count: 77,
            ..OracleMonthState::default()
        };
        assert_eq!(
            decode_oracle_month(&encoded_month(month.clone())),
            Ok(month)
        );
    }
}

#[test]
fn month_decoder_rejects_invalid_borsh_tags_and_nonzero_padding() {
    let mut invalid_bool = encoded_month(OracleMonthState::default());
    invalid_bool[0] = 2;
    assert!(decode_oracle_month(&invalid_bool).is_err());

    let mut invalid_enum = encoded_month(OracleMonthState::default());
    invalid_enum[86] = 7;
    assert!(decode_oracle_month(&invalid_enum).is_err());

    let mut nonzero_padding = encoded_month(OracleMonthState::default());
    *nonzero_padding.last_mut().unwrap() = 1;
    assert!(decode_oracle_month(&nonzero_padding).is_err());
}
