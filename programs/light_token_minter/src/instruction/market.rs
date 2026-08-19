use super::*;

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct InitMarketV2Params {
    pub market_id: [u8; 32],
    pub instrument: InstrumentDefinition,
    pub params: MarketParameters,
    pub collateral_mint: Pubkey,
}

crate::fixed_codec::fixed_instruction_deserialize!(InitMarketV2Params, 177, {
    market_id: [u8; 32],
    instrument: InstrumentDefinition,
    params: MarketParameters,
    collateral_mint: Pubkey,
});

#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, Eq, PartialEq)]
pub struct SetMarketPausedParams {
    pub paused: bool,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct UpsertMarketPageParams {
    pub page: CompressedMarketPageLeaf,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct UpsertSettlementParams {
    pub settlement: CompressedSettlementLeaf,
}

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct InitializeSettlementSignerRegistryParams {
    pub signer_set_version: u64,
    pub threshold: u8,
    pub signer_count: u8,
    pub signers: [Pubkey; MAX_SETTLEMENT_SIGNER_COUNT],
    pub rotation_delay_slots: u64,
    pub recovery_authority: Pubkey,
}

crate::fixed_codec::fixed_instruction_deserialize!(InitializeSettlementSignerRegistryParams, 306, {
    signer_set_version: u64,
    threshold: u8,
    signer_count: u8,
    signers: [Pubkey; MAX_SETTLEMENT_SIGNER_COUNT],
    rotation_delay_slots: u64,
    recovery_authority: Pubkey,
});

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct ProposeSettlementSignerRotationParams {
    pub target_version: u64,
    pub threshold: u8,
    pub signer_count: u8,
    pub signers: [Pubkey; MAX_SETTLEMENT_SIGNER_COUNT],
    pub rotation_delay_slots: u64,
    /// Exact activation slot approved by governance/current signers. The processor requires this
    /// to remain at least the governed delay beyond the proposal's execution slot.
    pub activate_after_slot: u64,
}

crate::fixed_codec::fixed_instruction_deserialize!(ProposeSettlementSignerRotationParams, 282, {
    target_version: u64,
    threshold: u8,
    signer_count: u8,
    signers: [Pubkey; MAX_SETTLEMENT_SIGNER_COUNT],
    rotation_delay_slots: u64,
    activate_after_slot: u64,
});

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct RotateVaultAuthoritiesV2Params {
    pub new_admin: Pubkey,
    pub new_oracle_authority: Pubkey,
}

crate::fixed_codec::fixed_instruction_deserialize!(RotateVaultAuthoritiesV2Params, 64, {
    new_admin: Pubkey,
    new_oracle_authority: Pubkey,
});

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct BootstrapVaultGovernanceV2Params {
    pub new_oracle_authority: Pubkey,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct ActivateVaultV2Params {
    pub expected_collateral_freeze_authority: Option<Pubkey>,
}

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
            sku_id_chunk: deserialize_bounded_vec(
                reader,
                MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS,
                "sku_id_chunk",
            )?,
        })
    }
}
