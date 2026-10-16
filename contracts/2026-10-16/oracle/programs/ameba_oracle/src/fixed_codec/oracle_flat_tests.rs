//! Reference-byte and I/O coverage for the explicitly represented rigid account codecs.
use crate::fixed_codec::{FixedStateDecode, FixedStateEncode, ReferenceBorsh};
use borsh::{BorshDeserialize, BorshSerialize};
use std::fmt::Debug;
use std::io::{self, Read, Write};

struct ChunkedWriter {
    bytes: Vec<u8>,
    remaining: Option<usize>,
}
impl Write for ChunkedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = bytes.len().min(7).min(self.remaining.unwrap_or(usize::MAX));
        self.bytes.extend_from_slice(&bytes[..count]);
        if let Some(left) = &mut self.remaining {
            *left -= count;
        }
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct ChunkedReader<'a>(&'a [u8]);
impl Read for ChunkedReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = output.len().min(self.0.len()).min(3);
        output[..count].copy_from_slice(&self.0[..count]);
        self.0 = &self.0[count..];
        Ok(count)
    }
}

// Offsets and valid tags below are explicit Borsh wire positions/domains,
// independent of the flat descriptors and native struct offsets.
fn check<T>(bool_offsets: &[usize], enums: &[(usize, &[u8])])
where
    T: Default
        + Eq
        + Debug
        + ReferenceBorsh
        + FixedStateDecode
        + FixedStateEncode
        + BorshDeserialize
        + BorshSerialize,
{
    let default = T::default();
    let baseline = default.reference_borsh_bytes();
    assert_eq!(default.maximum_encoded_len(), baseline.len());
    assert_eq!(T::REQUIRED_DATA_LEN, baseline.len());
    exercise::<T>(&baseline);
    for seed in [0u64, 1, u64::MAX] {
        let mut expected = baseline.clone();
        let mut random = seed ^ 0x9e37_79b9_7f4a_7c15;
        for byte in &mut expected {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            *byte = random as u8;
        }
        for &offset in bool_offsets {
            expected[offset] = (seed & 1) as u8;
        }
        for &(offset, tags) in enums {
            expected[offset] = tags[0];
        }
        exercise::<T>(&expected);
        for &(offset, tags) in enums {
            for tag in 0u8..=255 {
                let mut wire = expected.clone();
                wire[offset] = tag;
                if tags.contains(&tag) {
                    // Every valid variant is independently checked against the
                    // field-by-field ReferenceBorsh serializer and full I/O.
                    exercise::<T>(&wire);
                } else {
                    rejects::<T>(&wire);
                }
            }
        }
        for &offset in bool_offsets {
            for invalid in [2, 255] {
                let mut wire = expected.clone();
                wire[offset] = invalid;
                rejects::<T>(&wire);
            }
        }
    }
}

fn rejects<T: BorshDeserialize + FixedStateDecode>(wire: &[u8]) {
    assert!(T::try_from_slice(wire).is_err());
    assert!(T::deserialize_reader(&mut ChunkedReader(wire)).is_err());
    // All callers supply the complete fixed allocation required by this API.
    assert!(unsafe { T::decode_fixed(wire) }.is_err());
}

fn exercise<T>(expected: &[u8])
where
    T: Eq
        + Debug
        + ReferenceBorsh
        + FixedStateDecode
        + FixedStateEncode
        + BorshDeserialize
        + BorshSerialize,
{
    let value = T::try_from_slice(expected).unwrap();
    assert_eq!(value.reference_borsh_bytes(), expected);
    assert_eq!(value.try_to_vec().unwrap(), expected);
    assert_eq!(unsafe { T::decode_fixed(expected) }.unwrap(), value);
    let length = expected.len();
    let mut fixed = vec![0x6d; length + 16];
    value.encode_fixed(&mut fixed[..length]);
    assert_eq!(&fixed[..length], expected);
    assert_eq!(&fixed[length..], &[0x6d; 16]);
    let mut padded = expected.to_vec();
    padded.extend([0; 16]);
    assert_eq!(unsafe { T::decode_fixed(&padded) }.unwrap(), value);
    padded[length + 7] = 1;
    assert!(unsafe { T::decode_fixed(&padded) }.is_err());
    let mut prefixed = expected.to_vec();
    prefixed.extend([0xab; 16]);
    let mut remaining = prefixed.as_slice();
    assert_eq!(T::deserialize(&mut remaining).unwrap(), value);
    assert_eq!(remaining, &[0xab; 16]);
    assert!(T::try_from_slice(&prefixed).is_err());
    let mut reader = ChunkedReader(expected);
    assert_eq!(T::deserialize_reader(&mut reader).unwrap(), value);
    assert!(reader.0.is_empty());
    let mut writer = ChunkedWriter {
        bytes: vec![],
        remaining: None,
    };
    value.serialize(&mut writer).unwrap();
    assert_eq!(writer.bytes, expected);
    for limit in [0, 1, length / 2, length - 1] {
        let mut writer = ChunkedWriter {
            bytes: vec![],
            remaining: Some(limit),
        };
        assert_eq!(
            value.serialize(&mut writer).unwrap_err().kind(),
            io::ErrorKind::WriteZero
        );
        assert_eq!(writer.bytes, expected[..limit]);
    }
    for short in 0..length {
        assert!(T::try_from_slice(&expected[..short]).is_err());
        assert!(T::deserialize_reader(&mut ChunkedReader(&expected[..short])).is_err());
    }
}

#[test]
fn oracle_config_reference_and_full_io() {
    check::<crate::state::OracleConfig>(&[0, 130], &[]);
}

#[test]
fn user_collateral_reference_and_full_io() {
    check::<crate::state::UserCollateral>(&[0], &[]);
}

#[test]
fn settlement_signer_registry_reference_and_full_io() {
    check::<crate::state::SettlementSignerRegistry>(&[0], &[]);
}

#[test]
fn settlement_signer_set_reference_and_full_io() {
    check::<crate::state::SettlementSignerSet>(&[0, 360], &[]);
}

#[test]
fn oracle_economics_reference_and_full_io() {
    check::<crate::state::OracleEconomicParams>(&[], &[]);
    check::<crate::state::OracleEconomicsConfig>(&[0], &[]);
}

#[test]
fn oracle_maturity_ladder_reference_and_full_io() {
    check::<crate::state::OracleMaturityLadderRegistry>(&[0], &[]);
}

#[test]
fn oracle_product_sku_accounts_reference_and_full_io() {
    check::<crate::state::OracleProductSkuDraft>(&[0], &[]);
    check::<crate::state::OracleProductSkuManifest>(&[0], &[]);
}

#[test]
fn oracle_sku_coverage_accounts_reference_and_full_io() {
    check::<crate::state::OracleSkuCoverageManifest>(&[0, 90], &[]);
    check::<crate::state::OracleSkuCoverageRecord>(&[0], &[]);
}

const FOUR: &[u8] = &[0, 1, 2, 3];
const SIX: &[u8] = &[0, 1, 2, 3, 4, 5];
const EIGHT: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 7];

#[test]
fn oracle_support_position_reference_and_full_io() {
    check::<crate::state::OracleSupportPosition>(&[0, 138, 140], &[(139, FOUR)]);
}

#[test]
fn oracle_opening_claim_challenge_reference_and_full_io() {
    check::<crate::state::OracleOpeningClaimChallenge>(&[0], &[(230, SIX), (367, FOUR)]);
}

#[test]
fn oracle_update_challenge_reference_and_full_io() {
    check::<crate::state::OracleUpdateChallenge>(&[0], &[(194, SIX), (267, FOUR)]);
}

#[test]
fn oracle_usdc_reward_registration_reference_and_full_io() {
    check::<crate::state::OracleUsdcRewardRegistration>(&[0], &[(102, FOUR)]);
}

#[test]
fn oracle_usdc_reward_receipt_reference_and_full_io() {
    check::<crate::state::OracleUsdcRewardReceipt>(&[0], &[(102, FOUR)]);
}

#[test]
fn settlement_record_reference_and_full_io() {
    check::<crate::state::SettlementRecordV2>(&[0], &[]);
}

#[test]
fn oracle_source_state_reference_and_full_io() {
    check::<crate::state::OracleSourceState>(&[0, 261], &[(260, EIGHT)]);
}

#[test]
fn oracle_source_challenge_reference_and_full_io() {
    check::<crate::state::OracleSourceChallenge>(&[0, 294], &[(243, SIX), (284, FOUR)]);
}

#[test]
fn oracle_usdc_source_reward_reference_and_full_io() {
    check::<crate::state::OracleUsdcSourceReward>(&[0, 202, 277], &[(203, EIGHT)]);
}

#[test]
fn oracle_usdc_reward_schedule_reference_and_full_io() {
    check::<crate::state::OracleUsdcRewardSchedule>(&[0, 157], &[(102, FOUR)]);
}
const BUCKET_STATUS: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 7];
const RECIPE: &[u8] = &[0, 2, 3]; // Tag 1 is deliberately invalid.

#[test]
fn oracle_bucket_median_state_reference_and_full_io() {
    check::<crate::state::OracleBucketMedianState>(&[0], &[(82, BUCKET_STATUS)]);
}

#[test]
fn oracle_active_weight_manifest_reference_and_full_io() {
    check::<crate::state::OracleActiveWeightManifest>(&[0], &[(38, RECIPE)]);
}

#[test]
fn oracle_settlement_source_manifest_reference_and_full_io() {
    check::<crate::state::OracleSettlementSourceManifest>(&[0], &[(38, RECIPE)]);
}

#[test]
fn oracle_recipe_weight_manifest_reference_and_full_io() {
    check::<crate::state::OracleRecipeWeightManifest>(&[0], &[(38, RECIPE)]);
}
