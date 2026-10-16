#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleInstructionTag {
    InitializeConfig = 0,
    CreatePeriod = 1,
    UpdateConfig = 2,
    InitUserCollateral = 9,
    DepositCollateral = 10,
    WithdrawCollateral = 11,
    OracleCarryForwardV1 = 30,
    CloseOracleMonth = 59,
    FinalizeOracleMonth = 75,
    FailPeriod = 79,
    FinalizeOracleOpeningPhase = 80,
    ExpireOracleOpeningSource = 81,
    AccumulateOracleRecipeBucketV2 = 84,
    FinalizeOracleRecipeWeightsV2 = 86,
    InitializeSettlementSignerRegistry = 90,
    ProposeSettlementSignerRotation = 91,
    ActivateSettlementSignerRotation = 92,
    CancelSettlementSignerRotation = 93,
    ProposeEmergencySettlementSignerRecovery = 94,
    RotateVaultAuthoritiesV2 = 109,
    ConfigureOracleEconomicsTemplateV2 = 110,
    AccumulateOracleSettlementSourceBucket = 113,
    UpsertSettlementV3 = 116,
    BeginOracleActiveWeights = 121,
    AccumulateOracleActiveWeightGroup = 122,
    FinalizeOracleActiveWeights = 124,
    BootstrapVaultGovernanceV2 = 128,
    ActivateVaultV2 = 129,
    CfmLaunchV1 = 130,
    AdminAssistedWithdrawCollateral = 140,
    InitializeOracleUsdcRewardVault = 154,
    DepositOracleUsdcRewards = 155,
    BeginOracleUsdcRewardSchedule = 156,
    AddOracleUsdcSkuBudget = 157,
    FinalizeOracleUsdcRewardSchedule = 158,
    ChallengeOracleSourceV2 = 161,
    SubmitOracleOpeningClaimV2 = 162,
    ChallengeOracleOpeningClaimV2 = 163,
    ResolveOracleOpeningClaimChallengeV2 = 164,
    FinalizeOracleOpeningClaimV2 = 165,
    CommitOracleUpdateClaimV3 = 166,
    SettleExpiredOracleUpdateCommitmentV3 = 167,
    ChallengeOracleUpdateClaimV2 = 168,
    SettleOracleUsdcEscrow = 169,
    RegisterOracleUsdcRewardSource = 170,
    RegisterOracleUsdcRewardUpdate = 171,
    FinalizeOracleUsdcRewardEntitlements = 173,
    ClaimOracleUsdcReward = 174,
    InitializeOracleMonthV5 = 181,
    ProposeOracleSourceV3 = 182,
    SupportOracleSourceV3 = 183,
    FinalizeOracleSkuCoverage = 184,
    ResolveOracleSourceChallengeV2 = 185,
    ReopenOracleSkuCoverage = 186,
    BeginOracleRecipeWeightsV3 = 187,
    ConfigureOracleProductSkuManifest = 190,
    ExpireUnlistableOracleSourceV2 = 191,
    CancelStaleOracleSourceChallengeV2 = 192,
    SettleFailedOracleMonthEscrowV2 = 194,
    AbortOracleUsdcRewardScheduleV2 = 195,
    TimeoutUnsupportedOracleSourceV2 = 196,
    RecomputeOracleBucketMedianV1 = 197,
    RevealOracleUpdateClaimV3 = 198,
    FinalizeOracleUpdateClaimV2 = 199,
    IndexOracleRecipeSourceV1 = 200,
    CancelStaleOracleUpdateClaimV2 = 201,
}

impl OracleInstructionTag {
    /// Every assigned instruction byte, in ascending order.
    pub const ALL: [Self; 66] = [
        Self::InitializeConfig,
        Self::CreatePeriod,
        Self::UpdateConfig,
        Self::InitUserCollateral,
        Self::DepositCollateral,
        Self::WithdrawCollateral,
        Self::OracleCarryForwardV1,
        Self::CloseOracleMonth,
        Self::FinalizeOracleMonth,
        Self::FailPeriod,
        Self::FinalizeOracleOpeningPhase,
        Self::ExpireOracleOpeningSource,
        Self::AccumulateOracleRecipeBucketV2,
        Self::FinalizeOracleRecipeWeightsV2,
        Self::InitializeSettlementSignerRegistry,
        Self::ProposeSettlementSignerRotation,
        Self::ActivateSettlementSignerRotation,
        Self::CancelSettlementSignerRotation,
        Self::ProposeEmergencySettlementSignerRecovery,
        Self::RotateVaultAuthoritiesV2,
        Self::ConfigureOracleEconomicsTemplateV2,
        Self::AccumulateOracleSettlementSourceBucket,
        Self::UpsertSettlementV3,
        Self::BeginOracleActiveWeights,
        Self::AccumulateOracleActiveWeightGroup,
        Self::FinalizeOracleActiveWeights,
        Self::BootstrapVaultGovernanceV2,
        Self::ActivateVaultV2,
        Self::CfmLaunchV1,
        Self::AdminAssistedWithdrawCollateral,
        Self::InitializeOracleUsdcRewardVault,
        Self::DepositOracleUsdcRewards,
        Self::BeginOracleUsdcRewardSchedule,
        Self::AddOracleUsdcSkuBudget,
        Self::FinalizeOracleUsdcRewardSchedule,
        Self::ChallengeOracleSourceV2,
        Self::SubmitOracleOpeningClaimV2,
        Self::ChallengeOracleOpeningClaimV2,
        Self::ResolveOracleOpeningClaimChallengeV2,
        Self::FinalizeOracleOpeningClaimV2,
        Self::CommitOracleUpdateClaimV3,
        Self::SettleExpiredOracleUpdateCommitmentV3,
        Self::ChallengeOracleUpdateClaimV2,
        Self::SettleOracleUsdcEscrow,
        Self::RegisterOracleUsdcRewardSource,
        Self::RegisterOracleUsdcRewardUpdate,
        Self::FinalizeOracleUsdcRewardEntitlements,
        Self::ClaimOracleUsdcReward,
        Self::InitializeOracleMonthV5,
        Self::ProposeOracleSourceV3,
        Self::SupportOracleSourceV3,
        Self::FinalizeOracleSkuCoverage,
        Self::ResolveOracleSourceChallengeV2,
        Self::ReopenOracleSkuCoverage,
        Self::BeginOracleRecipeWeightsV3,
        Self::ConfigureOracleProductSkuManifest,
        Self::ExpireUnlistableOracleSourceV2,
        Self::CancelStaleOracleSourceChallengeV2,
        Self::SettleFailedOracleMonthEscrowV2,
        Self::AbortOracleUsdcRewardScheduleV2,
        Self::TimeoutUnsupportedOracleSourceV2,
        Self::RecomputeOracleBucketMedianV1,
        Self::RevealOracleUpdateClaimV3,
        Self::FinalizeOracleUpdateClaimV2,
        Self::IndexOracleRecipeSourceV1,
        Self::CancelStaleOracleUpdateClaimV2,
    ];

    pub fn from_byte(tag: u8) -> Option<Self> {
        const VALID_TAGS: [u64; 4] = [
            0x800_0000_4000_0e07,
            0x1612_6000_7c53_8800,
            0xcfe0_6ffe_7c00_1007,
            0x3fd,
        ];
        let bit = 1u64 << (tag & 63);
        if VALID_TAGS[usize::from(tag >> 6)] & bit == 0 {
            None
        } else {
            // SAFETY: the bitmap contains exactly the declared repr(u8) discriminants above.
            Some(unsafe { core::mem::transmute::<u8, Self>(tag) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OracleInstructionTag;

    #[test]
    fn bitmap_contains_exactly_the_declared_discriminants() {
        let mut expected = [0u64; 4];
        for tag in OracleInstructionTag::ALL {
            let byte = tag as u8;
            expected[usize::from(byte >> 6)] |= 1u64 << (byte & 63);
            assert_eq!(OracleInstructionTag::from_byte(byte), Some(tag));
        }
        assert!(OracleInstructionTag::ALL
            .windows(2)
            .all(|pair| (pair[0] as u8) < (pair[1] as u8)));
        for byte in 0u8..=u8::MAX {
            let assigned = expected[usize::from(byte >> 6)] & (1u64 << (byte & 63)) != 0;
            assert_eq!(
                OracleInstructionTag::from_byte(byte).is_some(),
                assigned,
                "byte {byte}"
            );
        }
    }

    #[test]
    fn unknown_bytes_remain_unassigned() {
        for byte in [
            3u8, 12, 29, 31, 32, 34, 35, 69, 87, 89, 96, 97, 189, 203, 204, 211, 212, 214,
        ] {
            assert!(
                OracleInstructionTag::from_byte(byte).is_none(),
                "byte {byte}"
            );
        }
        for byte in 220u8..=255 {
            assert!(
                OracleInstructionTag::from_byte(byte).is_none(),
                "byte {byte}"
            );
        }
    }
}
