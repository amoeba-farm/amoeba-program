//! Differential checks for instruction payloads decoded through `CheckedCursor` readers. Each
//! compact decoder must accept exactly the byte strings the reference reader accepts, consume the
//! same bytes and produce the same value: thousands of generated buffers, then every truncation,
//! a one-byte extension and single-byte mutations of valid encodings.

use super::equivalence::check_with_samples;
use borsh::BorshSerialize;

fn bytes<T: BorshSerialize>(values: &[T]) -> Vec<Vec<u8>> {
    values.iter().map(|v| v.try_to_vec().unwrap()).collect()
}

/// Tag 190 registers a product as data: every chunk carries the declared count and root. The
/// cursor decoder must match the reference reader exactly, including the chunk bound.
#[test]
fn product_sku_manifest_chunks_match_their_reader() {
    use crate::instruction::ConfigureOracleProductSkuManifestParams;
    let params = |chunk: u8, count: u16| ConfigureOracleProductSkuManifestParams {
        underlying_id: [chunk; 32],
        draft_nonce: 9,
        expected_start_index: 16,
        sku_count: count,
        expected_root: [0x5a; 32],
        sku_id_chunk: (1..=chunk).map(|i| [i; 32]).collect(),
    };
    let samples = bytes(&[params(0, 1), params(1, 52), params(5, 48), params(16, 256)]);
    check_with_samples::<ConfigureOracleProductSkuManifestParams>(0x5770, 700, &samples);
}
