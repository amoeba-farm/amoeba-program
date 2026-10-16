use super::*;

fn serialize_tag<W: io::Write>(writer: &mut W, tag: OracleInstructionTag) -> io::Result<()> {
    (tag as u8).serialize(writer)
}

fn serialize_tagged_payload<W, T>(
    writer: &mut W,
    tag: OracleInstructionTag,
    payload: &T,
) -> io::Result<()>
where
    W: io::Write,
    T: BorshSerialize,
{
    serialize_tag(writer, tag)?;
    payload.serialize(writer)
}

impl BorshSerialize for OracleInstruction {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        match self {
            Self::InitializeConfig { admin } => {
                serialize_tagged_payload(writer, OracleInstructionTag::InitializeConfig, admin)
            }
            Self::CreatePeriod { params } => {
                serialize_tagged_payload(writer, OracleInstructionTag::CreatePeriod, params)
            }
            Self::UpdateConfig {
                new_admin,
                new_oracle_authority,
                new_usdc_mint,
                new_bond_vault,
                paused,
            } => {
                serialize_tag(writer, OracleInstructionTag::UpdateConfig)?;
                new_admin.serialize(writer)?;
                new_oracle_authority.serialize(writer)?;
                new_usdc_mint.serialize(writer)?;
                new_bond_vault.serialize(writer)?;
                paused.serialize(writer)
            }
            Self::RotateVaultAuthoritiesV2 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::RotateVaultAuthoritiesV2,
                params,
            ),
            Self::BootstrapVaultGovernanceV2 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::BootstrapVaultGovernanceV2,
                params,
            ),
            Self::ActivateVaultV2 { params } => {
                serialize_tagged_payload(writer, OracleInstructionTag::ActivateVaultV2, params)
            }
            Self::CfmLaunchV1 { params } => {
                serialize_tagged_payload(writer, OracleInstructionTag::CfmLaunchV1, params)
            }
            Self::InitUserCollateral => {
                serialize_tag(writer, OracleInstructionTag::InitUserCollateral)
            }
            Self::DepositCollateral { amount } => {
                serialize_tagged_payload(writer, OracleInstructionTag::DepositCollateral, amount)
            }
            Self::WithdrawCollateral { amount } => {
                serialize_tagged_payload(writer, OracleInstructionTag::WithdrawCollateral, amount)
            }
            Self::OracleCarryForwardV1 { action } => {
                serialize_tagged_payload(writer, OracleInstructionTag::OracleCarryForwardV1, action)
            }
            Self::AdminAssistedWithdrawCollateral { amount } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::AdminAssistedWithdrawCollateral,
                amount,
            ),
            Self::ConfigureOracleEconomicsTemplateV2 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::ConfigureOracleEconomicsTemplateV2,
                params,
            ),
            Self::InitializeOracleMonthV5 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::InitializeOracleMonthV5,
                params,
            ),
            Self::InitializeOracleUsdcRewardVault => serialize_tag(
                writer,
                OracleInstructionTag::InitializeOracleUsdcRewardVault,
            ),
            Self::DepositOracleUsdcRewards { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::DepositOracleUsdcRewards,
                params,
            ),
            Self::BeginOracleUsdcRewardSchedule { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::BeginOracleUsdcRewardSchedule,
                params,
            ),
            Self::AddOracleUsdcSkuBudget { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::AddOracleUsdcSkuBudget,
                params,
            ),
            Self::FinalizeOracleUsdcRewardSchedule => serialize_tag(
                writer,
                OracleInstructionTag::FinalizeOracleUsdcRewardSchedule,
            ),
            Self::ChallengeOracleSourceV2 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::ChallengeOracleSourceV2,
                params,
            ),
            Self::SettleOracleUsdcEscrow { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::SettleOracleUsdcEscrow,
                params,
            ),
            Self::ProposeOracleSourceV3 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::ProposeOracleSourceV3,
                params,
            ),
            Self::SupportOracleSourceV3 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::SupportOracleSourceV3,
                params,
            ),
            Self::FinalizeOracleSkuCoverage => {
                serialize_tag(writer, OracleInstructionTag::FinalizeOracleSkuCoverage)
            }
            Self::ResolveOracleSourceChallengeV2 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::ResolveOracleSourceChallengeV2,
                params,
            ),
            Self::ReopenOracleSkuCoverage => {
                serialize_tag(writer, OracleInstructionTag::ReopenOracleSkuCoverage)
            }
            Self::ExpireUnlistableOracleSourceV2 => {
                serialize_tag(writer, OracleInstructionTag::ExpireUnlistableOracleSourceV2)
            }
            Self::CancelStaleOracleSourceChallengeV2 => serialize_tag(
                writer,
                OracleInstructionTag::CancelStaleOracleSourceChallengeV2,
            ),
            Self::SettleFailedOracleMonthEscrowV2 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::SettleFailedOracleMonthEscrowV2,
                params,
            ),
            Self::AbortOracleUsdcRewardScheduleV2 => serialize_tag(
                writer,
                OracleInstructionTag::AbortOracleUsdcRewardScheduleV2,
            ),
            Self::TimeoutUnsupportedOracleSourceV2 => serialize_tag(
                writer,
                OracleInstructionTag::TimeoutUnsupportedOracleSourceV2,
            ),
            Self::FinalizeOracleOpeningPhase => {
                serialize_tag(writer, OracleInstructionTag::FinalizeOracleOpeningPhase)
            }
            Self::ExpireOracleOpeningSource => {
                serialize_tag(writer, OracleInstructionTag::ExpireOracleOpeningSource)
            }
            Self::AccumulateOracleRecipeBucketV2 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::AccumulateOracleRecipeBucketV2,
                params,
            ),
            Self::FinalizeOracleRecipeWeightsV2 => {
                serialize_tag(writer, OracleInstructionTag::FinalizeOracleRecipeWeightsV2)
            }
            Self::AccumulateOracleSettlementSourceBucket { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::AccumulateOracleSettlementSourceBucket,
                params,
            ),
            Self::BeginOracleActiveWeights { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::BeginOracleActiveWeights,
                params,
            ),
            Self::AccumulateOracleActiveWeightGroup { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::AccumulateOracleActiveWeightGroup,
                params,
            ),
            Self::FinalizeOracleActiveWeights => {
                serialize_tag(writer, OracleInstructionTag::FinalizeOracleActiveWeights)
            }
            Self::SubmitOracleOpeningClaimV2 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::SubmitOracleOpeningClaimV2,
                params,
            ),
            Self::ChallengeOracleOpeningClaimV2 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::ChallengeOracleOpeningClaimV2,
                params,
            ),
            Self::ResolveOracleOpeningClaimChallengeV2 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::ResolveOracleOpeningClaimChallengeV2,
                params,
            ),
            Self::FinalizeOracleOpeningClaimV2 => {
                serialize_tag(writer, OracleInstructionTag::FinalizeOracleOpeningClaimV2)
            }
            Self::BeginOracleRecipeWeightsV3 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::BeginOracleRecipeWeightsV3,
                params,
            ),
            Self::RecomputeOracleBucketMedianV1 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::RecomputeOracleBucketMedianV1,
                params,
            ),
            Self::IndexOracleRecipeSourceV1 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::IndexOracleRecipeSourceV1,
                params,
            ),
            Self::ConfigureOracleProductSkuManifest { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::ConfigureOracleProductSkuManifest,
                params,
            ),
            Self::CommitOracleUpdateClaimV3 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::CommitOracleUpdateClaimV3,
                params,
            ),
            Self::SettleExpiredOracleUpdateCommitmentV3 => serialize_tag(
                writer,
                OracleInstructionTag::SettleExpiredOracleUpdateCommitmentV3,
            ),
            Self::ChallengeOracleUpdateClaimV2 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::ChallengeOracleUpdateClaimV2,
                params,
            ),
            Self::RevealOracleUpdateClaimV3 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::RevealOracleUpdateClaimV3,
                params,
            ),
            Self::FinalizeOracleUpdateClaimV2 { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::FinalizeOracleUpdateClaimV2,
                params,
            ),
            Self::CancelStaleOracleUpdateClaimV2 => {
                serialize_tag(writer, OracleInstructionTag::CancelStaleOracleUpdateClaimV2)
            }
            Self::CloseOracleMonth => serialize_tag(writer, OracleInstructionTag::CloseOracleMonth),
            Self::FinalizeOracleMonth => {
                serialize_tag(writer, OracleInstructionTag::FinalizeOracleMonth)
            }
            Self::FailPeriod => serialize_tag(writer, OracleInstructionTag::FailPeriod),
            Self::InitializeSettlementSignerRegistry { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::InitializeSettlementSignerRegistry,
                params,
            ),
            Self::ProposeSettlementSignerRotation { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::ProposeSettlementSignerRotation,
                params,
            ),
            Self::ActivateSettlementSignerRotation => serialize_tag(
                writer,
                OracleInstructionTag::ActivateSettlementSignerRotation,
            ),
            Self::CancelSettlementSignerRotation => {
                serialize_tag(writer, OracleInstructionTag::CancelSettlementSignerRotation)
            }
            Self::ProposeEmergencySettlementSignerRecovery { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::ProposeEmergencySettlementSignerRecovery,
                params,
            ),
            Self::UpsertSettlementV3 { params } => {
                serialize_tagged_payload(writer, OracleInstructionTag::UpsertSettlementV3, params)
            }
            Self::RegisterOracleUsdcRewardSource => {
                serialize_tag(writer, OracleInstructionTag::RegisterOracleUsdcRewardSource)
            }
            Self::RegisterOracleUsdcRewardUpdate => {
                serialize_tag(writer, OracleInstructionTag::RegisterOracleUsdcRewardUpdate)
            }
            Self::FinalizeOracleUsdcRewardEntitlements => serialize_tag(
                writer,
                OracleInstructionTag::FinalizeOracleUsdcRewardEntitlements,
            ),
            Self::ClaimOracleUsdcReward { params } => serialize_tagged_payload(
                writer,
                OracleInstructionTag::ClaimOracleUsdcReward,
                params,
            ),
        }
    }
}
