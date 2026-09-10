use super::*;

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct ChallengeOracleSourceParams {
    pub challenge_id: [u8; 32],
    pub reason: u8,
    pub bond: u64,
    pub evidence_hash: [u8; 32],
    pub comparison_source_id: [u8; 32],
}

crate::fixed_codec::fixed_instruction_deserialize!(ChallengeOracleSourceParams, 105, {
    challenge_id: [u8; 32],
    reason: u8,
    bond: u64,
    evidence_hash: [u8; 32],
    comparison_source_id: [u8; 32],
});

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct ResolveOracleSourceChallengeParams {
    pub outcome: OracleSourceChallengeOutcome,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct BeginOracleRecipeWeightsV2Params {
    pub expected_source_count: u16,
    pub expected_bucket_count: u16,
}

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct AccumulateOracleRecipeBucketV2Params {
    pub bucket_id: [u8; 32],
    pub bucket_weight_bps: u16,
    /// Finishes this bucket after its ordered source records are committed.
    pub finalize_collection: bool,
}

crate::fixed_codec::fixed_instruction_deserialize!(AccumulateOracleRecipeBucketV2Params, 35, {
    bucket_id: [u8; 32],
    bucket_weight_bps: u16,
    finalize_collection: bool,
});

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct AccumulateOracleSettlementSourceBucketParams {
    pub bucket_id: [u8; 32],
    pub finalize_collection: bool,
}

crate::fixed_codec::fixed_instruction_deserialize!(AccumulateOracleSettlementSourceBucketParams, 33, {
    bucket_id: [u8; 32],
    finalize_collection: bool,
});

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct SubmitOracleOpeningClaimParams {
    pub opening_state: u64,
    pub source_time: u64,
    pub stake: u64,
    pub canonical_locator_hash: [u8; 32],
    pub source_definition_hash: [u8; 32],
    pub archive_url: String,
}

impl BorshDeserialize for SubmitOracleOpeningClaimParams {
    fn deserialize(data: &mut &[u8]) -> io::Result<Self> {
        let mut cursor = CheckedCursor::new(data);
        let value = Self {
            opening_state: cursor.u64(),
            source_time: cursor.u64(),
            stake: cursor.u64(),
            canonical_locator_hash: cursor.bytes(),
            source_definition_hash: cursor.bytes(),
            archive_url: cursor.bounded_string(ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES),
        };
        *data = cursor.finish()?;
        Ok(value)
    }

    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        Ok(Self {
            opening_state: u64::deserialize_reader(reader)?,
            source_time: u64::deserialize_reader(reader)?,
            stake: u64::deserialize_reader(reader)?,
            canonical_locator_hash: <[u8; 32]>::deserialize_reader(reader)?,
            source_definition_hash: <[u8; 32]>::deserialize_reader(reader)?,
            archive_url: deserialize_bounded_string(
                reader,
                ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES,
                "archive_url",
            )?,
        })
    }
}

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct ChallengeOracleOpeningClaimParams {
    pub challenge_id: [u8; 32],
    pub alternative_opening_state: u64,
    pub alternative_source_time: u64,
    pub bond: u64,
    pub canonical_locator_hash: [u8; 32],
    pub source_definition_hash: [u8; 32],
    pub archive_url: String,
}

impl BorshDeserialize for ChallengeOracleOpeningClaimParams {
    fn deserialize(data: &mut &[u8]) -> io::Result<Self> {
        let mut cursor = CheckedCursor::new(data);
        let value = Self {
            challenge_id: cursor.bytes(),
            alternative_opening_state: cursor.u64(),
            alternative_source_time: cursor.u64(),
            bond: cursor.u64(),
            canonical_locator_hash: cursor.bytes(),
            source_definition_hash: cursor.bytes(),
            archive_url: cursor.bounded_string(ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES),
        };
        *data = cursor.finish()?;
        Ok(value)
    }

    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        Ok(Self {
            challenge_id: <[u8; 32]>::deserialize_reader(reader)?,
            alternative_opening_state: u64::deserialize_reader(reader)?,
            alternative_source_time: u64::deserialize_reader(reader)?,
            bond: u64::deserialize_reader(reader)?,
            canonical_locator_hash: <[u8; 32]>::deserialize_reader(reader)?,
            source_definition_hash: <[u8; 32]>::deserialize_reader(reader)?,
            archive_url: deserialize_bounded_string(
                reader,
                ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES,
                "archive_url",
            )?,
        })
    }
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct ResolveOracleOpeningClaimChallengeParams {
    pub outcome: OracleOpeningChallengeOutcome,
}

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct ChallengeOracleUpdateClaimParams {
    pub challenge_id: [u8; 32],
    pub alternative_state: u64,
    pub alternative_source_time: u64,
    pub bond: u64,
    pub evidence_hash: [u8; 32],
    pub archive_url: String,
}

impl BorshDeserialize for ChallengeOracleUpdateClaimParams {
    fn deserialize(data: &mut &[u8]) -> io::Result<Self> {
        let mut cursor = CheckedCursor::new(data);
        let value = Self {
            challenge_id: cursor.bytes(),
            alternative_state: cursor.u64(),
            alternative_source_time: cursor.u64(),
            bond: cursor.u64(),
            evidence_hash: cursor.bytes(),
            archive_url: cursor.bounded_string(ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES),
        };
        *data = cursor.finish()?;
        Ok(value)
    }

    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        Ok(Self {
            challenge_id: <[u8; 32]>::deserialize_reader(reader)?,
            alternative_state: u64::deserialize_reader(reader)?,
            alternative_source_time: u64::deserialize_reader(reader)?,
            bond: u64::deserialize_reader(reader)?,
            evidence_hash: <[u8; 32]>::deserialize_reader(reader)?,
            archive_url: deserialize_bounded_string(
                reader,
                ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES,
                "archive_url",
            )?,
        })
    }
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct BeginOracleActiveWeightsParams {
    pub expected_source_count: u16,
    pub expected_group_count: u16,
}

/// One reverse hash-chain step from the finalized recipe. `previous_hash` is authenticated
/// by hashing it with these frozen fields and comparing with the on-chain remaining hash.
#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct IndexOracleRecipeSourceV1Params {
    pub previous_hash: [u8; 32],
    pub bucket_id: [u8; 32],
    pub source_id: [u8; 32],
    pub source_type_hash: [u8; 32],
    pub canonical_locator_hash: [u8; 32],
    pub source_definition_hash: [u8; 32],
    pub bucket_weight_bps: u16,
}

crate::fixed_codec::fixed_instruction_deserialize!(IndexOracleRecipeSourceV1Params, 194, {
    previous_hash: [u8; 32],
    bucket_id: [u8; 32],
    source_id: [u8; 32],
    source_type_hash: [u8; 32],
    canonical_locator_hash: [u8; 32],
    source_definition_hash: [u8; 32],
    bucket_weight_bps: u16,
});

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct AccumulateOracleActiveWeightGroupParams {
    pub group_id: [u8; 32],
    pub finalize_collection: bool,
}

crate::fixed_codec::fixed_instruction_deserialize!(AccumulateOracleActiveWeightGroupParams, 33, {
    group_id: [u8; 32],
    finalize_collection: bool,
});

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct CommitOracleUpdateClaimV2Params {
    pub claim_id: [u8; 32],
    pub commit_hash: [u8; 32],
    pub stake: u64,
}

crate::fixed_codec::fixed_instruction_deserialize!(CommitOracleUpdateClaimV2Params, 72, {
    claim_id: [u8; 32],
    commit_hash: [u8; 32],
    stake: u64,
});

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct RevealOracleUpdateClaimV3Params {
    pub claim_id: [u8; 32],
    pub prior_state: u64,
    pub new_state: u64,
    pub source_time: u64,
    pub evidence_hash: [u8; 32],
    pub archive_url: String,
    pub secret_salt: [u8; 32],
}

impl BorshDeserialize for RevealOracleUpdateClaimV3Params {
    fn deserialize(data: &mut &[u8]) -> io::Result<Self> {
        let mut cursor = CheckedCursor::new(data);
        let value = Self {
            claim_id: cursor.bytes(),
            prior_state: cursor.u64(),
            new_state: cursor.u64(),
            source_time: cursor.u64(),
            evidence_hash: cursor.bytes(),
            archive_url: cursor.bounded_string(ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES),
            secret_salt: cursor.bytes(),
        };
        *data = cursor.finish()?;
        Ok(value)
    }

    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        Ok(Self {
            claim_id: <[u8; 32]>::deserialize_reader(reader)?,
            prior_state: u64::deserialize_reader(reader)?,
            new_state: u64::deserialize_reader(reader)?,
            source_time: u64::deserialize_reader(reader)?,
            evidence_hash: <[u8; 32]>::deserialize_reader(reader)?,
            archive_url: deserialize_bounded_string(
                reader,
                ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES,
                "archive_url",
            )?,
            secret_salt: <[u8; 32]>::deserialize_reader(reader)?,
        })
    }
}

/// mode: 0 = primary settlement window, 1 = one-time grace window.
#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct RecomputeOracleBucketMedianV1Params {
    pub bucket_id: [u8; 32],
    pub mode: u8,
}

crate::fixed_codec::fixed_instruction_deserialize!(RecomputeOracleBucketMedianV1Params, 33, {
    bucket_id: [u8; 32],
    mode: u8,
});

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct FinalizeOracleUpdateClaimV2Params {
    pub outcome: OracleUpdateClaimOutcome,
    pub current_step: u64,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct TryOpenOracleEmergencyDisputeParams {
    pub kind: OracleEmergencyDisputeKind,
    pub target_id: [u8; 32],
    pub expected_case_hash: Option<[u8; 32]>,
}

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct CommitOracleEmergencyVoteV2Params {
    pub commit_hash: [u8; 32],
    pub samba_amount: u64,
}

crate::fixed_codec::fixed_instruction_deserialize!(CommitOracleEmergencyVoteV2Params, 40, {
    commit_hash: [u8; 32],
    samba_amount: u64,
});

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct RevealOracleEmergencyVoteParams {
    pub choice: u8,
    pub salt: [u8; 32],
}

crate::fixed_codec::fixed_instruction_deserialize!(RevealOracleEmergencyVoteParams, 33, {
    choice: u8,
    salt: [u8; 32],
});

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct ResolveOracleEmergencyDisputeParams {}
