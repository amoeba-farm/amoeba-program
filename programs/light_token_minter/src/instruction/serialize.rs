use super::*;

fn serialize_tag<W: io::Write>(writer: &mut W, tag: VaultInstructionTag) -> io::Result<()> {
    (tag as u8).serialize(writer)
}

fn serialize_tagged_payload<W, T>(
    writer: &mut W,
    tag: VaultInstructionTag,
    payload: &T,
) -> io::Result<()>
where
    W: io::Write,
    T: BorshSerialize,
{
    serialize_tag(writer, tag)?;
    payload.serialize(writer)
}

impl BorshSerialize for VaultInstruction {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        match self {
            Self::ManageWriterDlmmV1 { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::ManageWriterDlmmV1, params)
            }
            Self::Initialize => serialize_tag(writer, VaultInstructionTag::Initialize),
            Self::UpdateConfig {
                new_admin,
                new_oracle_authority,
                new_usdc_mint,
                new_vault_token_account,
                paused,
            } => {
                serialize_tag(writer, VaultInstructionTag::UpdateConfig)?;
                new_admin.serialize(writer)?;
                new_oracle_authority.serialize(writer)?;
                new_usdc_mint.serialize(writer)?;
                new_vault_token_account.serialize(writer)?;
                paused.serialize(writer)
            }
            Self::RotateVaultAuthoritiesV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::RotateVaultAuthoritiesV2,
                params,
            ),
            Self::BootstrapVaultGovernanceV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::BootstrapVaultGovernanceV2,
                params,
            ),
            Self::ActivateVaultV2 { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::ActivateVaultV2, params)
            }
            Self::DepositUsdc { amount } => {
                serialize_tagged_payload(writer, VaultInstructionTag::DepositUsdc, amount)
            }
            Self::InitMarketV2 { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::InitMarketV2, params)
            }
            Self::CreateMarketContractMintV3 => {
                serialize_tag(writer, VaultInstructionTag::CreateMarketContractMintV3)
            }
            Self::SetMarketPaused { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::SetMarketPaused, params)
            }
            Self::InitializeSettlementSignerRegistry { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::InitializeSettlementSignerRegistry,
                params,
            ),
            Self::ProposeSettlementSignerRotation { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ProposeSettlementSignerRotation,
                params,
            ),
            Self::ActivateSettlementSignerRotation => serialize_tag(
                writer,
                VaultInstructionTag::ActivateSettlementSignerRotation,
            ),
            Self::CancelSettlementSignerRotation => {
                serialize_tag(writer, VaultInstructionTag::CancelSettlementSignerRotation)
            }
            Self::ProposeEmergencySettlementSignerRecovery { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ProposeEmergencySettlementSignerRecovery,
                params,
            ),
            Self::UpsertSettlementV3 {
                params,
                proof,
                new_settlement_output,
                existing_settlement,
            } => {
                serialize_tag(writer, VaultInstructionTag::UpsertSettlementV3)?;
                params.serialize(writer)?;
                proof.serialize(writer)?;
                new_settlement_output.serialize(writer)?;
                existing_settlement.serialize(writer)
            }
            Self::UpsertMarketPageV2 {
                params,
                proof,
                new_page_output,
                existing_page,
            } => {
                serialize_tag(writer, VaultInstructionTag::UpsertMarketPageV2)?;
                params.serialize(writer)?;
                proof.serialize(writer)?;
                new_page_output.serialize(writer)?;
                existing_page.serialize(writer)
            }
            Self::InitUserCollateral => {
                serialize_tag(writer, VaultInstructionTag::InitUserCollateral)
            }
            Self::DepositCollateral { amount } => {
                serialize_tagged_payload(writer, VaultInstructionTag::DepositCollateral, amount)
            }
            Self::WithdrawCollateral { amount } => {
                serialize_tagged_payload(writer, VaultInstructionTag::WithdrawCollateral, amount)
            }
            Self::AdminAssistedWithdrawCollateral { amount } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::AdminAssistedWithdrawCollateral,
                amount,
            ),
            Self::ConfigureOracleEconomicsTemplateV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ConfigureOracleEconomicsTemplateV2,
                params,
            ),
            Self::InitializeOracleMonthV5 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::InitializeOracleMonthV5,
                params,
            ),
            Self::ConfigureOracleProductSkuManifest { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ConfigureOracleProductSkuManifest,
                params,
            ),
            Self::ExpireUnlistableOracleSourceV2 => {
                serialize_tag(writer, VaultInstructionTag::ExpireUnlistableOracleSourceV2)
            }
            Self::CancelStaleOracleSourceChallengeV2 => serialize_tag(
                writer,
                VaultInstructionTag::CancelStaleOracleSourceChallengeV2,
            ),
            Self::ResolveOracleEmergencyDisputeV4 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ResolveOracleEmergencyDisputeV4,
                params,
            ),
            Self::SettleFailedOracleMonthEscrowV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::SettleFailedOracleMonthEscrowV2,
                params,
            ),
            Self::AbortOracleUsdcRewardScheduleV2 => {
                serialize_tag(writer, VaultInstructionTag::AbortOracleUsdcRewardScheduleV2)
            }
            Self::TimeoutUnsupportedOracleSourceV2 => serialize_tag(
                writer,
                VaultInstructionTag::TimeoutUnsupportedOracleSourceV2,
            ),
            Self::InitializeOracleUsdcRewardVault => {
                serialize_tag(writer, VaultInstructionTag::InitializeOracleUsdcRewardVault)
            }
            Self::DepositOracleUsdcRewards { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::DepositOracleUsdcRewards,
                params,
            ),
            Self::BeginOracleUsdcRewardSchedule { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::BeginOracleUsdcRewardSchedule,
                params,
            ),
            Self::AddOracleUsdcSkuBudget { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::AddOracleUsdcSkuBudget,
                params,
            ),
            Self::FinalizeOracleUsdcRewardSchedule => serialize_tag(
                writer,
                VaultInstructionTag::FinalizeOracleUsdcRewardSchedule,
            ),
            Self::ChallengeOracleSourceV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ChallengeOracleSourceV2,
                params,
            ),
            Self::SubmitOracleOpeningClaimV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::SubmitOracleOpeningClaimV2,
                params,
            ),
            Self::ChallengeOracleOpeningClaimV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ChallengeOracleOpeningClaimV2,
                params,
            ),
            Self::ResolveOracleOpeningClaimChallengeV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ResolveOracleOpeningClaimChallengeV2,
                params,
            ),
            Self::FinalizeOracleOpeningClaimV2 => {
                serialize_tag(writer, VaultInstructionTag::FinalizeOracleOpeningClaimV2)
            }
            Self::CommitOracleUpdateClaimV3 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::CommitOracleUpdateClaimV3,
                params,
            ),
            Self::RevealOracleUpdateClaimV3 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::RevealOracleUpdateClaimV3,
                params,
            ),
            Self::SettleExpiredOracleUpdateCommitmentV3 => serialize_tag(
                writer,
                VaultInstructionTag::SettleExpiredOracleUpdateCommitmentV3,
            ),
            Self::ChallengeOracleUpdateClaimV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ChallengeOracleUpdateClaimV2,
                params,
            ),
            Self::FinalizeOracleUpdateClaimV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::FinalizeOracleUpdateClaimV2,
                params,
            ),
            Self::SettleOracleUsdcEscrow { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::SettleOracleUsdcEscrow,
                params,
            ),
            Self::RegisterOracleUsdcRewardSource => {
                serialize_tag(writer, VaultInstructionTag::RegisterOracleUsdcRewardSource)
            }
            Self::RegisterOracleUsdcRewardUpdate => {
                serialize_tag(writer, VaultInstructionTag::RegisterOracleUsdcRewardUpdate)
            }
            Self::FinalizeOracleUsdcRewardEntitlements => serialize_tag(
                writer,
                VaultInstructionTag::FinalizeOracleUsdcRewardEntitlements,
            ),
            Self::ClaimOracleUsdcReward { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::ClaimOracleUsdcReward, params)
            }
            Self::ConfigureOracleMajorToken { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ConfigureOracleMajorToken,
                params,
            ),
            Self::DepositOracleMajorTokens { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::DepositOracleMajorTokens,
                params,
            ),
            Self::WithdrawOracleMajorTokens { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::WithdrawOracleMajorTokens,
                params,
            ),
            Self::InitializeOracleSambaPool { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::InitializeOracleSambaPool,
                params,
            ),
            Self::InitializeOracleRewardFunnel { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::InitializeOracleRewardFunnel,
                params,
            ),
            Self::SweepOracleRewardFunnel { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::SweepOracleRewardFunnel,
                params,
            ),
            Self::QueueStakeAmbaForSamba { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::QueueStakeAmbaForSamba,
                params,
            ),
            Self::ActivateQueuedStakeAmbaForSamba { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ActivateQueuedStakeAmbaForSamba,
                params,
            ),
            Self::CancelQueuedStakeAmba { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::CancelQueuedStakeAmba, params)
            }
            Self::RequestUnstakeSamba { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::RequestUnstakeSamba, params)
            }
            Self::CompleteUnstakeSamba { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::CompleteUnstakeSamba, params)
            }
            Self::ProposeOracleSourceV3 { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::ProposeOracleSourceV3, params)
            }
            Self::SupportOracleSourceV3 { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::SupportOracleSourceV3, params)
            }
            Self::FinalizeOracleSkuCoverage => {
                serialize_tag(writer, VaultInstructionTag::FinalizeOracleSkuCoverage)
            }
            Self::ResolveOracleSourceChallengeV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ResolveOracleSourceChallengeV2,
                params,
            ),
            Self::CancelStaleOracleUpdateClaimV2 => {
                serialize_tag(writer, VaultInstructionTag::CancelStaleOracleUpdateClaimV2)
            }
            Self::AbortStaleOracleUpdateEmergencyDisputeV2 => serialize_tag(
                writer,
                VaultInstructionTag::AbortStaleOracleUpdateEmergencyDisputeV2,
            ),
            Self::ReopenOracleSkuCoverage => {
                serialize_tag(writer, VaultInstructionTag::ReopenOracleSkuCoverage)
            }
            Self::BeginOracleRecipeWeightsV3 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::BeginOracleRecipeWeightsV3,
                params,
            ),
            Self::AccumulateOracleRecipeBucketV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::AccumulateOracleRecipeBucketV2,
                params,
            ),
            Self::FinalizeOracleRecipeWeightsV2 => {
                serialize_tag(writer, VaultInstructionTag::FinalizeOracleRecipeWeightsV2)
            }
            Self::AccumulateOracleSettlementSourceBucket { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::AccumulateOracleSettlementSourceBucket,
                params,
            ),
            Self::BeginOracleActiveWeights { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::BeginOracleActiveWeights,
                params,
            ),
            Self::AccumulateOracleActiveWeightGroup { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::AccumulateOracleActiveWeightGroup,
                params,
            ),
            Self::FinalizeOracleActiveWeights => {
                serialize_tag(writer, VaultInstructionTag::FinalizeOracleActiveWeights)
            }
            Self::RecomputeOracleBucketMedianV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::RecomputeOracleBucketMedianV1,
                params,
            ),
            Self::IndexOracleRecipeSourceV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::IndexOracleRecipeSourceV1,
                params,
            ),
            Self::FinalizeOracleOpeningPhase => {
                serialize_tag(writer, VaultInstructionTag::FinalizeOracleOpeningPhase)
            }
            Self::ExpireOracleOpeningSource => {
                serialize_tag(writer, VaultInstructionTag::ExpireOracleOpeningSource)
            }
            Self::TryOpenOracleEmergencyDisputeV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::TryOpenOracleEmergencyDisputeV2,
                params,
            ),
            Self::CommitOracleEmergencyVoteV3 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::CommitOracleEmergencyVoteV3,
                params,
            ),
            Self::RevealOracleEmergencyVoteV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::RevealOracleEmergencyVoteV2,
                params,
            ),
            Self::ResolveOracleEmergencyDisputeV2 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ResolveOracleEmergencyDisputeV2,
                params,
            ),
            Self::RegisterOracleSambaWinningVote => {
                serialize_tag(writer, VaultInstructionTag::RegisterOracleSambaWinningVote)
            }
            Self::SettleOracleSambaEmergencyVoteV2 => serialize_tag(
                writer,
                VaultInstructionTag::SettleOracleSambaEmergencyVoteV2,
            ),
            Self::CloseOracleMonth => serialize_tag(writer, VaultInstructionTag::CloseOracleMonth),
            Self::FinalizeOracleMonth => {
                serialize_tag(writer, VaultInstructionTag::FinalizeOracleMonth)
            }
            Self::InitializeWriterPolicyRegistryV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::InitializeWriterPolicyRegistryV1,
                params,
            ),
            Self::ManageWriterPolicyAuthorityV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ManageWriterPolicyAuthorityV1,
                params,
            ),
            Self::InitializeWriterSettlementGroupV1 => serialize_tag(
                writer,
                VaultInstructionTag::InitializeWriterSettlementGroupV1,
            ),
            Self::InitializeWriterSleeveV1 => {
                serialize_tag(writer, VaultInstructionTag::InitializeWriterSleeveV1)
            }
            Self::RegisterWriterSeriesV1 => {
                serialize_tag(writer, VaultInstructionTag::RegisterWriterSeriesV1)
            }
            Self::SealWriterPolicyV1 { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::SealWriterPolicyV1, params)
            }
            Self::OpenWriterFundingV1 => {
                serialize_tag(writer, VaultInstructionTag::OpenWriterFundingV1)
            }
            Self::DepositWriterPrincipalV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::DepositWriterPrincipalV1,
                params,
            ),
            Self::WithdrawWriterPrincipalV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::WithdrawWriterPrincipalV1,
                params,
            ),
            Self::ActivateWriterSleeveV1 => {
                serialize_tag(writer, VaultInstructionTag::ActivateWriterSleeveV1)
            }
            Self::SetCollectiveMarketPausedV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::SetCollectiveMarketPausedV1,
                params,
            ),
            Self::ReconcileWriterSupplyV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ReconcileWriterSupplyV1,
                params,
            ),
            Self::CleanupWriterCustodyV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::CleanupWriterCustodyV1,
                params,
            ),
            Self::CommitWriterAuctionV1 { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::CommitWriterAuctionV1, params)
            }
            Self::PlaceWriterBidV1 { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::PlaceWriterBidV1, params)
            }
            Self::CancelOrRefundWriterBidV1 => {
                serialize_tag(writer, VaultInstructionTag::CancelOrRefundWriterBidV1)
            }
            Self::RevealWriterAuctionV1 { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::RevealWriterAuctionV1, params)
            }
            Self::PlanWriterAuctionChunkV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::PlanWriterAuctionChunkV1,
                params,
            ),
            Self::ExecuteWriterAuctionFillV1 => {
                serialize_tag(writer, VaultInstructionTag::ExecuteWriterAuctionFillV1)
            }
            Self::FinalizeOrAbortWriterAuctionV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::FinalizeOrAbortWriterAuctionV1,
                params,
            ),
            Self::BeginWriterCloseV1 { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::BeginWriterCloseV1, params)
            }
            Self::DepositWriterCloseBasketV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::DepositWriterCloseBasketV1,
                params,
            ),
            Self::FinalizeWriterCloseV1 => {
                serialize_tag(writer, VaultInstructionTag::FinalizeWriterCloseV1)
            }
            Self::ProcessWriterCloseCancellationV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ProcessWriterCloseCancellationV1,
                params,
            ),
            Self::PublishWriterGroupSettlementV1 => {
                serialize_tag(writer, VaultInstructionTag::PublishWriterGroupSettlementV1)
            }
            Self::FinalizeWriterSleeveSettlementV1 => serialize_tag(
                writer,
                VaultInstructionTag::FinalizeWriterSleeveSettlementV1,
            ),
            Self::ClaimCollectiveLongV1 { params } => {
                serialize_tagged_payload(writer, VaultInstructionTag::ClaimCollectiveLongV1, params)
            }
            Self::ClaimWriterFlatResidualV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ClaimWriterFlatResidualV1,
                params,
            ),
            Self::PrepareWriterBidIndexV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::PrepareWriterBidIndexV1,
                params,
            ),
            Self::CloseWriterSleeveV1 => {
                serialize_tag(writer, VaultInstructionTag::CloseWriterSleeveV1)
            }
            Self::ScopedCollectiveSettlementV1 { action } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ScopedCollectiveSettlementV1,
                action,
            ),
            Self::ScopedPositionSettlementV1 { action } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ScopedPositionSettlementV1,
                action,
            ),
            Self::ExecuteCompressedStateV1 { params } => serialize_tagged_payload(
                writer,
                VaultInstructionTag::ExecuteCompressedStateV1,
                params,
            ),
        }
    }
}
