//! The compact Oracle claim omits five identity fields from its 144-byte wire.
//! Keep the original typed reader and field-by-field Borsh writer as references.
use super::*;
use std::io::{self, Read, Write};

fn legacy(bytes: &[u8]) -> io::Result<OracleOpeningClaim> {
    let mut cursor = FixedCursor::new(bytes);
    let value = <OracleOpeningClaim as FixedField>::read(&mut cursor);
    cursor.finish_borsh()?;
    Ok(value)
}

fn dense(seed: u64) -> Vec<u8> {
    let mut random = seed ^ 0x9e37_79b9_7f4a_7c15;
    let mut bytes: Vec<_> = (0..144)
        .map(|_| {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            random as u8
        })
        .collect();
    // Original wire domains: initialized, opening status, escrow disposition.
    bytes[0] &= 1;
    bytes[142] %= 6;
    bytes[143] %= 4;
    bytes
}

fn assert_omitted_defaults(value: &OracleOpeningClaim) {
    assert_eq!(value.month, Pubkey::default());
    assert_eq!(value.source, Pubkey::default());
    assert_eq!(value.source_id, [0; 32]);
    assert_eq!(value.canonical_locator_hash, [0; 32]);
    assert_eq!(value.source_definition_hash, [0; 32]);
}

#[test]
fn compact_claim_flat_fields_preserve_bytes_and_omitted_defaults() {
    for seed in [0, 1, 71, u64::MAX] {
        let bytes = dense(seed);
        let reference = legacy(&bytes).unwrap();
        assert_omitted_defaults(&reference);
        assert_eq!(reference.reference_borsh_bytes(), bytes);
        assert_eq!(reference.try_to_vec().unwrap(), bytes);
        assert_eq!(
            OracleOpeningClaim::try_from_slice(&bytes).unwrap(),
            reference
        );
        assert_eq!(
            unsafe { OracleOpeningClaim::decode_fixed(&bytes) }.unwrap(),
            reference
        );

        let mut with_identities = reference.clone();
        with_identities.month = Pubkey::new_from_array([19; 32]);
        with_identities.source = Pubkey::new_from_array([27; 32]);
        with_identities.source_id = [43; 32];
        with_identities.canonical_locator_hash = [89; 32];
        with_identities.source_definition_hash = [131; 32];
        assert_eq!(with_identities.reference_borsh_bytes(), bytes);
        assert_eq!(with_identities.try_to_vec().unwrap(), bytes);
        let mut out = vec![0xa7; 161];
        with_identities.encode_fixed(&mut out);
        assert_eq!(&out[..144], bytes);
        assert_eq!(&out[144..], &[0xa7; 17]);

        for end in 0..144 {
            let mut input = &bytes[..end];
            assert!(OracleOpeningClaim::deserialize(&mut input).is_err());
            assert_eq!(input, &bytes[..end]);
        }
        let mut padded = bytes.clone();
        padded.extend_from_slice(&[0; 17]);
        assert_eq!(
            unsafe { OracleOpeningClaim::decode_fixed(&padded) }.unwrap(),
            reference
        );
        for offset in 144..padded.len() {
            padded[offset] = 0x80;
            assert!(unsafe { OracleOpeningClaim::decode_fixed(&padded) }.is_err());
            padded[offset] = 0;
        }
        padded[144] = 0x7f;
        let mut input = padded.as_slice();
        assert_eq!(
            OracleOpeningClaim::deserialize(&mut input).unwrap(),
            reference
        );
        assert_eq!(input, &padded[144..]);
    }
}

#[test]
fn compact_claim_flat_domains_match_the_original_typed_reader() {
    let baseline = dense(59);
    for (offset, maximum) in [(0, 1), (142, 5), (143, 3)] {
        for byte in 0..=u8::MAX {
            let mut bytes = baseline.clone();
            bytes[offset] = byte;
            let reference = legacy(&bytes);
            let actual = unsafe { OracleOpeningClaim::decode_fixed(&bytes) };
            assert_eq!(reference.is_ok(), byte <= maximum);
            assert_eq!(actual.is_ok(), reference.is_ok());
            match reference {
                Ok(expected) => {
                    assert_eq!(actual.unwrap(), expected);
                    assert_omitted_defaults(&expected);
                    assert_eq!(expected.reference_borsh_bytes(), bytes);
                }
                Err(_) => {
                    let mut input = bytes.as_slice();
                    assert!(OracleOpeningClaim::deserialize(&mut input).is_err());
                    assert_eq!(input, bytes);
                }
            }
        }
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

struct ChunkedWriter {
    bytes: Vec<u8>,
    remaining: usize,
}
impl Write for ChunkedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = bytes.len().min(7).min(self.remaining);
        self.bytes.extend_from_slice(&bytes[..count]);
        self.remaining -= count;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn compact_claim_streaming_io_preserves_prefix_and_short_write_errors() {
    let bytes = dense(137);
    let value = legacy(&bytes).unwrap();
    let mut trailing = bytes.clone();
    trailing.extend_from_slice(&[9, 8, 7]);
    let mut reader = ChunkedReader(&trailing);
    assert_eq!(
        OracleOpeningClaim::deserialize_reader(&mut reader).unwrap(),
        value
    );
    assert_eq!(reader.0, &[9, 8, 7]);
    for end in 0..144 {
        let mut reader = ChunkedReader(&bytes[..end]);
        assert_eq!(
            OracleOpeningClaim::deserialize_reader(&mut reader)
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
    let mut writer = ChunkedWriter {
        bytes: vec![],
        remaining: usize::MAX,
    };
    value.serialize(&mut writer).unwrap();
    assert_eq!(writer.bytes, bytes);
    let mut partial = ChunkedWriter {
        bytes: vec![],
        remaining: 53,
    };
    assert_eq!(
        value.serialize(&mut partial).unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
    assert_eq!(partial.bytes, bytes[..53]);
}
