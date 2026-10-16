//! Compare shared instruction decoding with the former typed reader and the
//! unchanged field-derived Borsh serializer, including every byte domain.
use crate::fixed_codec::ReferenceInstructionDeserialize;
use crate::instruction::*;
use borsh::{BorshDeserialize, BorshSerialize};
use std::fmt::Debug;
use std::io::{self, Read};

struct ChunkedReader<'a>(&'a [u8]);
impl Read for ChunkedReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = output.len().min(self.0.len()).min(7);
        output[..count].copy_from_slice(&self.0[..count]);
        self.0 = &self.0[count..];
        Ok(count)
    }
}

fn compare_prefix<T>(data: &[u8]) -> bool
where
    T: ReferenceInstructionDeserialize + BorshDeserialize + BorshSerialize + Eq + Debug,
{
    let mut actual_rest = data;
    let mut reference_rest = data;
    let actual = T::deserialize(&mut actual_rest);
    let reference = T::reference_deserialize(&mut reference_rest);
    assert_eq!(
        actual_rest,
        reference_rest,
        "{} prefix",
        std::any::type_name::<T>()
    );
    match (actual, reference) {
        (Ok(actual), Ok(reference)) => {
            assert_eq!(actual, reference);
            assert_eq!(actual.try_to_vec().unwrap(), data[..T::WIRE_LEN]);
            assert_eq!(actual_rest, &data[T::WIRE_LEN..]);
            true
        }
        (Err(actual), Err(reference)) => {
            assert_eq!(actual.kind(), reference.kind());
            assert_eq!(actual_rest, data, "failed decoding must not consume input");
            false
        }
        (actual, reference) => panic!("decoder mismatch: {actual:?} / {reference:?}"),
    }
}

fn check<T>(domains: &[(usize, u16)])
where
    T: ReferenceInstructionDeserialize + BorshDeserialize + BorshSerialize + Eq + Debug,
{
    for seed in [0u64, 1, 0xa75e_8421, u64::MAX] {
        let mut bytes = vec![0u8; T::WIRE_LEN];
        let mut random = seed ^ 0x9e37_79b9_7f4a_7c15;
        for byte in &mut bytes {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            *byte = random as u8;
        }
        for &(offset, mask) in domains {
            let valid: Vec<u8> = (0..16).filter(|bit| mask & (1u16 << bit) != 0).collect();
            bytes[offset] = valid[seed as usize % valid.len()];
        }
        assert!(compare_prefix::<T>(&bytes));
        let value = T::try_from_slice(&bytes).unwrap();
        let mut reader = ChunkedReader(&bytes);
        assert_eq!(T::deserialize_reader(&mut reader).unwrap(), value);
        assert!(reader.0.is_empty());

        for length in 0..bytes.len() {
            assert!(!compare_prefix::<T>(&bytes[..length]));
            let mut reader = ChunkedReader(&bytes[..length]);
            assert_eq!(
                T::deserialize_reader(&mut reader).unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof
            );
        }
        let mut extended = bytes.clone();
        extended.extend([0xb5; 17]);
        assert!(compare_prefix::<T>(&extended));
        assert!(T::try_from_slice(&extended).is_err());
        let mut reader = ChunkedReader(&extended);
        assert_eq!(T::deserialize_reader(&mut reader).unwrap(), value);
        assert_eq!(reader.0, &[0xb5; 17]);

        for offset in 0..bytes.len() {
            for byte in [0u8, 1, 127, 255] {
                let mut changed = bytes.clone();
                changed[offset] = byte;
                let accepted = compare_prefix::<T>(&changed);
                let expected = domains
                    .iter()
                    .find(|(index, _)| *index == offset)
                    .is_none_or(|(_, mask)| byte < 16 && mask & (1u16 << byte) != 0);
                assert_eq!(accepted, expected);
            }
        }
        for &(offset, mask) in domains {
            for byte in 0u8..=u8::MAX {
                let mut changed = bytes.clone();
                changed[offset] = byte;
                changed.extend([0x63; 11]);
                assert_eq!(
                    compare_prefix::<T>(&changed),
                    byte < 16 && mask & (1u16 << byte) != 0
                );
            }
        }
    }
}

#[test]
fn all_config_and_period_payload_fields_and_domains_match_the_typed_reader() {
    check::<RotateVaultAuthoritiesV2Params>(&[]);
    check::<CreatePeriodParams>(&[]);
    check::<InitializeOracleMonthV5Params>(&[]);
}

#[test]
fn all_oracle_payload_fields_and_domains_match_the_typed_reader() {
    check::<AddOracleUsdcSkuBudgetParams>(&[]);
    check::<ConfigureOracleEconomicsTemplateV2Params>(&[]);
    check::<ChallengeOracleSourceParams>(&[]);
    check::<AccumulateOracleRecipeBucketV2Params>(&[(34, 0b11)]);
    check::<AccumulateOracleSettlementSourceBucketParams>(&[(32, 0b11)]);
    check::<IndexOracleRecipeSourceV1Params>(&[]);
    check::<AccumulateOracleActiveWeightGroupParams>(&[(32, 0b11)]);
    check::<CommitOracleUpdateClaimV2Params>(&[]);
    // mode remains an unrestricted u8 in the wire codec.
    check::<RecomputeOracleBucketMedianV1Params>(&[]);
}
