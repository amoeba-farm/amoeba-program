use super::*;

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct InitializeOracleMonthV3Params {
    pub scramble_start_ts: u64,
    pub listing_ts: u64,
    pub settlement_base_oracle_atomic: u64,
}

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct InitializeOracleMonthV5Params {
    pub scramble_start_ts: u64,
    pub listing_ts: u64,
    pub settlement_base_oracle_atomic: u64,
    /// Root of the index-bound Merkle tree containing every terminal SKU required by this month.
    pub required_sku_root: [u8; 32],
    pub required_sku_count: u16,
}

crate::fixed_codec::fixed_instruction_deserialize!(InitializeOracleMonthV5Params, 58, {
    scramble_start_ts: u64,
    listing_ts: u64,
    settlement_base_oracle_atomic: u64,
    required_sku_root: [u8; 32],
    required_sku_count: u16,
});

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct ConfigureOracleProductSkuManifestParams {
    pub underlying_id: [u8; 32],
    /// Governance-selected nonce for this abandonable draft attempt.
    pub draft_nonce: u64,
    /// Must equal the number of identifiers already stored in the canonical draft.
    pub expected_start_index: u16,
    /// Declared number of terminal SKUs in this product (1..=MAX_ORACLE_REQUIRED_SKUS). The first
    /// chunk stores it in the draft; every later chunk must repeat the same value.
    pub sku_count: u16,
    /// Declared SKU Merkle root. The draft finalizes only when the program's own root over all
    /// `sku_count` identifiers equals it. Every chunk repeats it, like `sku_count`.
    pub expected_root: [u8; 32],
    /// Nonempty, strictly ascending continuation of the canonical terminal-SKU identifiers.
    pub sku_id_chunk: Vec<[u8; 32]>,
}

impl BorshDeserialize for ConfigureOracleProductSkuManifestParams {
    fn deserialize(data: &mut &[u8]) -> io::Result<Self> {
        let mut cursor = CheckedCursor::new(data);
        let value = Self {
            underlying_id: cursor.bytes(),
            draft_nonce: cursor.u64(),
            expected_start_index: cursor.u16(),
            sku_count: cursor.u16(),
            expected_root: cursor.bytes(),
            sku_id_chunk: deserialize_bounded_bytes32_vec_cursor(
                &mut cursor,
                MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS,
            ),
        };
        *data = cursor.finish()?;
        Ok(value)
    }

    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        Ok(Self {
            underlying_id: <[u8; 32]>::deserialize_reader(reader)?,
            draft_nonce: u64::deserialize_reader(reader)?,
            expected_start_index: u16::deserialize_reader(reader)?,
            sku_count: u16::deserialize_reader(reader)?,
            expected_root: <[u8; 32]>::deserialize_reader(reader)?,
            sku_id_chunk: deserialize_bounded_vec(
                reader,
                MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS,
                "sku_id_chunk",
            )?,
        })
    }
}
