//! Check nested descriptor paths against the unchanged typed field reader and
//! independently generated Borsh bytes, including exact rejection domains.
use super::*;
use std::fmt::Debug;

fn typed_read<T: FixedField>(bytes: &[u8]) -> std::io::Result<T> {
    let mut cursor = FixedCursor::new(bytes);
    let value = T::read(&mut cursor);
    cursor.finish_borsh()?;
    Ok(value)
}

fn check<T>(len: usize, domains: &[(usize, u8)])
where
    T: FixedField
        + FixedStateEncode
        + FixedStateDecode
        + ReferenceBorsh
        + BorshSerialize
        + BorshDeserialize
        + PartialEq
        + Debug,
{
    for seed in [0u64, 1, 71, u64::MAX] {
        let mut random = seed ^ 0x9e37_79b9_7f4a_7c15;
        let mut bytes: Vec<_> = (0..len)
            .map(|_| {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                random as u8
            })
            .collect();
        for &(offset, maximum) in domains {
            bytes[offset] %= maximum + 1;
        }
        let reference = typed_read::<T>(&bytes).unwrap();
        assert_eq!(reference.reference_borsh_bytes(), bytes);
        assert_eq!(reference.try_to_vec().unwrap(), bytes);
        assert_eq!(unsafe { T::decode_fixed(&bytes) }.unwrap(), reference);
        assert_eq!(T::try_from_slice(&bytes).unwrap(), reference);
        let mut out = vec![0xa7; len + 17];
        reference.encode_fixed(&mut out);
        assert_eq!(&out[..len], bytes);
        assert_eq!(&out[len..], &[0xa7; 17]);
        for &(offset, maximum) in domains {
            for byte in 0..=u8::MAX {
                let mut changed = bytes.clone();
                changed[offset] = byte;
                let legacy = typed_read::<T>(&changed);
                let decoded = unsafe { T::decode_fixed(&changed) };
                assert_eq!(legacy.is_ok(), byte <= maximum);
                assert_eq!(decoded.is_ok(), legacy.is_ok());
                if let Ok(expected) = legacy {
                    assert_eq!(decoded.unwrap(), expected);
                    assert_eq!(expected.reference_borsh_bytes(), changed);
                } else {
                    let mut input = changed.as_slice();
                    assert!(T::deserialize(&mut input).is_err());
                    assert_eq!(input, changed);
                }
            }
        }
        for end in 0..len {
            let mut input = &bytes[..end];
            assert!(T::deserialize(&mut input).is_err());
            assert_eq!(input, &bytes[..end]);
        }
        let mut padded = bytes.clone();
        padded.extend_from_slice(&[0; 19]);
        assert_eq!(unsafe { T::decode_fixed(&padded) }.unwrap(), reference);
        for offset in len..padded.len() {
            padded[offset] = 0x80;
            assert!(unsafe { T::decode_fixed(&padded) }.is_err());
            padded[offset] = 0;
        }
        padded[len] = 0x7f;
        let mut input = padded.as_slice();
        assert_eq!(T::deserialize(&mut input).unwrap(), reference);
        assert_eq!(input, &padded[len..]);
    }
}

#[test]
fn economics_nested_fields_preserve_full_u64_windows() {
    check::<OracleEconomicsConfig>(40, &[(0, 1)]);
}

#[test]
fn claim_v2_nested_fields_preserve_status_escrow_and_review_domains() {
    // ClaimData occupies 264 bytes; its status and escrow bytes are 194/259.
    // The outer review bool follows 32 bytes and four u64s, at 328.
    check::<OracleUpdateClaimV2>(346, &[(0, 1), (194, 5), (259, 3), (328, 1)]);
}
