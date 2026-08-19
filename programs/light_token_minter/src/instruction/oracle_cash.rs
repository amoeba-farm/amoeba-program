use super::*;

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct DepositOracleUsdcRewardsParams {
    pub amount: u64,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct BeginOracleUsdcRewardScheduleParams;

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct AddOracleUsdcSkuBudgetParams {
    pub bucket_id: [u8; 32],
    pub source_reward_budget: u64,
    pub opening_reward_budget: u64,
    pub update_reward_budget: u64,
    pub proposer_reward_bps: u16,
    pub listing_bond: u64,
    pub support_bond: u64,
    pub opening_bond: u64,
    pub update_min_bond: u64,
    pub challenge_min_bond: u64,
    pub challenge_max_bond: u64,
    pub challenge_bond_bps: u16,
}

crate::fixed_codec::fixed_instruction_deserialize!(AddOracleUsdcSkuBudgetParams, 108, {
    bucket_id: [u8; 32],
    source_reward_budget: u64,
    opening_reward_budget: u64,
    update_reward_budget: u64,
    proposer_reward_bps: u16,
    listing_bond: u64,
    support_bond: u64,
    opening_bond: u64,
    update_min_bond: u64,
    challenge_min_bond: u64,
    challenge_max_bond: u64,
    challenge_bond_bps: u16,
});

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct ClaimOracleUsdcRewardParams {
    pub kind: OracleUsdcRewardKind,
}

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct ConfigureOracleEconomicsTemplateV2Params {
    pub expected_config_version: u64,
    pub economics: OracleEconomicParams,
}

crate::fixed_codec::fixed_instruction_deserialize!(ConfigureOracleEconomicsTemplateV2Params, 26, {
    expected_config_version: u64,
    economics: OracleEconomicParams,
});

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct ConfigureOracleMajorTokenParams {}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct DepositOracleMajorTokensParams {
    pub amount: u64,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct WithdrawOracleMajorTokensParams {
    pub amount: u64,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct InitializeOracleSambaPoolParams {}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct InitializeOracleRewardFunnelParams {}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct SweepOracleRewardFunnelParams {}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct QueueStakeAmbaForSambaParams {
    pub amba_amount: u64,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct ActivateQueuedStakeAmbaForSambaParams {
    pub min_samba_out: u64,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct CancelQueuedStakeAmbaParams {}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct RequestUnstakeSambaParams {
    pub samba_amount: u64,
    pub min_amba_out: u64,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct CompleteUnstakeSambaParams {}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct SettleOracleEscrowParams {
    pub kind: OracleEscrowKind,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct ProposeOracleSourceParams {
    pub source_id: [u8; 32],
    pub bucket_id: [u8; 32],
    pub source_type_hash: [u8; 32],
    pub canonical_locator_hash: [u8; 32],
    pub source_definition_hash: [u8; 32],
    pub listing_bond: u64,
}

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct ProposeOracleSourceV3Params {
    pub source_id: [u8; 32],
    pub bucket_id: [u8; 32],
    pub source_type_hash: [u8; 32],
    pub canonical_locator_hash: [u8; 32],
    pub source_definition_hash: [u8; 32],
    pub listing_bond: u64,
    pub sku_index: u16,
    pub sku_proof: Vec<[u8; 32]>,
}

impl BorshDeserialize for ProposeOracleSourceV3Params {
    fn deserialize(data: &mut &[u8]) -> io::Result<Self> {
        let mut cursor = CheckedCursor::new(data);
        let value = Self {
            source_id: cursor.bytes(),
            bucket_id: cursor.bytes(),
            source_type_hash: cursor.bytes(),
            canonical_locator_hash: cursor.bytes(),
            source_definition_hash: cursor.bytes(),
            listing_bond: cursor.u64(),
            sku_index: cursor.u16(),
            sku_proof: deserialize_bounded_bytes32_vec_cursor(
                &mut cursor,
                MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH,
            ),
        };
        *data = cursor.finish()?;
        Ok(value)
    }

    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        Ok(Self {
            source_id: <[u8; 32]>::deserialize_reader(reader)?,
            bucket_id: <[u8; 32]>::deserialize_reader(reader)?,
            source_type_hash: <[u8; 32]>::deserialize_reader(reader)?,
            canonical_locator_hash: <[u8; 32]>::deserialize_reader(reader)?,
            source_definition_hash: <[u8; 32]>::deserialize_reader(reader)?,
            listing_bond: u64::deserialize_reader(reader)?,
            sku_index: u16::deserialize_reader(reader)?,
            sku_proof: deserialize_bounded_vec(
                reader,
                MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH,
                "sku_proof",
            )?,
        })
    }
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct SupportOracleSourceParams {
    pub stake: u64,
}

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct SupportOracleSourceV3Params {
    pub stake: u64,
    pub sku_index: u16,
    pub sku_proof: Vec<[u8; 32]>,
}

impl BorshDeserialize for SupportOracleSourceV3Params {
    fn deserialize(data: &mut &[u8]) -> io::Result<Self> {
        let mut cursor = CheckedCursor::new(data);
        let value = Self {
            stake: cursor.u64(),
            sku_index: cursor.u16(),
            sku_proof: deserialize_bounded_bytes32_vec_cursor(
                &mut cursor,
                MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH,
            ),
        };
        *data = cursor.finish()?;
        Ok(value)
    }

    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        Ok(Self {
            stake: u64::deserialize_reader(reader)?,
            sku_index: u16::deserialize_reader(reader)?,
            sku_proof: deserialize_bounded_vec(
                reader,
                MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH,
                "sku_proof",
            )?,
        })
    }
}
