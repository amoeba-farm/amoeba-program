use super::*;

impl BorshDeserialize for VaultInstruction {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let tag_byte = u8::deserialize_reader(reader)?;
        let tag = VaultInstructionTag::from_byte(tag_byte)
            .ok_or_else(|| invalid_instruction_tag_error(tag_byte))?;
        match tag {
            VaultInstructionTag::Initialize => Ok(Self::Initialize),
            VaultInstructionTag::UpdateConfig => Ok(Self::UpdateConfig {
                new_admin: Option::<Pubkey>::deserialize_reader(reader)?,
                new_oracle_authority: Option::<Pubkey>::deserialize_reader(reader)?,
                new_usdc_mint: Option::<Pubkey>::deserialize_reader(reader)?,
                new_vault_token_account: Option::<Pubkey>::deserialize_reader(reader)?,
                paused: Option::<bool>::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::RotateVaultAuthoritiesV2 => Ok(Self::RotateVaultAuthoritiesV2 {
                params: RotateVaultAuthoritiesV2Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::BootstrapVaultGovernanceV2 => {
                Ok(Self::BootstrapVaultGovernanceV2 {
                    params: BootstrapVaultGovernanceV2Params::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::ActivateVaultV2 => Ok(Self::ActivateVaultV2 {
                params: ActivateVaultV2Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::DepositUsdc => Ok(Self::DepositUsdc {
                amount: u64::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::InitMarketV2 => Ok(Self::InitMarketV2 {
                params: InitMarketV2Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::CreateMarketContractMintV3 => Ok(Self::CreateMarketContractMintV3),
            VaultInstructionTag::SetMarketPaused => Ok(Self::SetMarketPaused {
                params: SetMarketPausedParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::InitializeSettlementSignerRegistry => {
                Ok(Self::InitializeSettlementSignerRegistry {
                    params: InitializeSettlementSignerRegistryParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::ProposeSettlementSignerRotation => {
                Ok(Self::ProposeSettlementSignerRotation {
                    params: ProposeSettlementSignerRotationParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::ActivateSettlementSignerRotation => {
                Ok(Self::ActivateSettlementSignerRotation)
            }
            VaultInstructionTag::CancelSettlementSignerRotation => {
                Ok(Self::CancelSettlementSignerRotation)
            }
            VaultInstructionTag::ProposeEmergencySettlementSignerRecovery => {
                Ok(Self::ProposeEmergencySettlementSignerRecovery {
                    params: ProposeSettlementSignerRotationParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::UpsertSettlementV3 => Ok(Self::UpsertSettlementV3 {
                params: UpsertSettlementParams::deserialize_reader(reader)?,
                proof: Box::<ValidityProof>::deserialize_reader(reader)?,
                new_settlement_output: Option::<Box<CompressionOutput>>::deserialize_reader(
                    reader,
                )?,
                existing_settlement:
                    Option::<Box<CompressedSettlementWitness>>::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::UpsertMarketPageV2 => Ok(Self::UpsertMarketPageV2 {
                params: UpsertMarketPageParams::deserialize_reader(reader)?,
                proof: Box::<ValidityProof>::deserialize_reader(reader)?,
                new_page_output: Option::<Box<CompressionOutput>>::deserialize_reader(reader)?,
                existing_page: Option::<Box<CompressedMarketPageWitness>>::deserialize_reader(
                    reader,
                )?,
            }),
            VaultInstructionTag::InitUserCollateral => Ok(Self::InitUserCollateral),
            VaultInstructionTag::DepositCollateral => Ok(Self::DepositCollateral {
                amount: u64::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::WithdrawCollateral => Ok(Self::WithdrawCollateral {
                amount: u64::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::ConfigureOracleEconomicsTemplateV2 => {
                Ok(Self::ConfigureOracleEconomicsTemplateV2 {
                    params: ConfigureOracleEconomicsTemplateV2Params::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::InitializeOracleMonthV5 => Ok(Self::InitializeOracleMonthV5 {
                params: InitializeOracleMonthV5Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::ConfigureOracleProductSkuManifest => {
                Ok(Self::ConfigureOracleProductSkuManifest {
                    params: ConfigureOracleProductSkuManifestParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::ExpireUnlistableOracleSourceV2 => {
                Ok(Self::ExpireUnlistableOracleSourceV2)
            }
            VaultInstructionTag::CancelStaleOracleSourceChallengeV2 => {
                Ok(Self::CancelStaleOracleSourceChallengeV2)
            }
            VaultInstructionTag::ResolveOracleEmergencyDisputeV4 => {
                Ok(Self::ResolveOracleEmergencyDisputeV4 {
                    params: ResolveOracleEmergencyDisputeParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::SettleFailedOracleMonthEscrowV2 => {
                Ok(Self::SettleFailedOracleMonthEscrowV2 {
                    params: SettleOracleEscrowParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::AbortOracleUsdcRewardScheduleV2 => {
                Ok(Self::AbortOracleUsdcRewardScheduleV2)
            }
            VaultInstructionTag::TimeoutUnsupportedOracleSourceV2 => {
                Ok(Self::TimeoutUnsupportedOracleSourceV2)
            }
            VaultInstructionTag::InitializeOracleUsdcRewardVault => {
                Ok(Self::InitializeOracleUsdcRewardVault)
            }
            VaultInstructionTag::DepositOracleUsdcRewards => Ok(Self::DepositOracleUsdcRewards {
                params: DepositOracleUsdcRewardsParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::BeginOracleUsdcRewardSchedule => {
                Ok(Self::BeginOracleUsdcRewardSchedule {
                    params: BeginOracleUsdcRewardScheduleParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::AddOracleUsdcSkuBudget => Ok(Self::AddOracleUsdcSkuBudget {
                params: AddOracleUsdcSkuBudgetParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::FinalizeOracleUsdcRewardSchedule => {
                Ok(Self::FinalizeOracleUsdcRewardSchedule)
            }
            VaultInstructionTag::ChallengeOracleSourceV2 => Ok(Self::ChallengeOracleSourceV2 {
                params: ChallengeOracleSourceParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::SubmitOracleOpeningClaimV2 => {
                Ok(Self::SubmitOracleOpeningClaimV2 {
                    params: SubmitOracleOpeningClaimParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::ChallengeOracleOpeningClaimV2 => {
                Ok(Self::ChallengeOracleOpeningClaimV2 {
                    params: ChallengeOracleOpeningClaimParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::ResolveOracleOpeningClaimChallengeV2 => {
                Ok(Self::ResolveOracleOpeningClaimChallengeV2 {
                    params: ResolveOracleOpeningClaimChallengeParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::FinalizeOracleOpeningClaimV2 => {
                Ok(Self::FinalizeOracleOpeningClaimV2)
            }
            VaultInstructionTag::CommitOracleUpdateClaimV3 => Ok(Self::CommitOracleUpdateClaimV3 {
                params: CommitOracleUpdateClaimV2Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::RevealOracleUpdateClaimV3 => Ok(Self::RevealOracleUpdateClaimV3 {
                params: RevealOracleUpdateClaimV3Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::SettleExpiredOracleUpdateCommitmentV3 => {
                Ok(Self::SettleExpiredOracleUpdateCommitmentV3)
            }
            VaultInstructionTag::ChallengeOracleUpdateClaimV2 => {
                Ok(Self::ChallengeOracleUpdateClaimV2 {
                    params: ChallengeOracleUpdateClaimParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::FinalizeOracleUpdateClaimV2 => {
                Ok(Self::FinalizeOracleUpdateClaimV2 {
                    params: FinalizeOracleUpdateClaimV2Params::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::SettleOracleUsdcEscrow => Ok(Self::SettleOracleUsdcEscrow {
                params: SettleOracleEscrowParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::RegisterOracleUsdcRewardSource => {
                Ok(Self::RegisterOracleUsdcRewardSource)
            }
            VaultInstructionTag::RegisterOracleUsdcRewardUpdate => {
                Ok(Self::RegisterOracleUsdcRewardUpdate)
            }
            VaultInstructionTag::FinalizeOracleUsdcRewardEntitlements => {
                Ok(Self::FinalizeOracleUsdcRewardEntitlements)
            }
            VaultInstructionTag::ClaimOracleUsdcReward => Ok(Self::ClaimOracleUsdcReward {
                params: ClaimOracleUsdcRewardParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::ConfigureOracleMajorToken => Ok(Self::ConfigureOracleMajorToken {
                params: ConfigureOracleMajorTokenParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::DepositOracleMajorTokens => Ok(Self::DepositOracleMajorTokens {
                params: DepositOracleMajorTokensParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::WithdrawOracleMajorTokens => Ok(Self::WithdrawOracleMajorTokens {
                params: WithdrawOracleMajorTokensParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::InitializeOracleSambaPool => Ok(Self::InitializeOracleSambaPool {
                params: InitializeOracleSambaPoolParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::InitializeOracleRewardFunnel => {
                Ok(Self::InitializeOracleRewardFunnel {
                    params: InitializeOracleRewardFunnelParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::SweepOracleRewardFunnel => Ok(Self::SweepOracleRewardFunnel {
                params: SweepOracleRewardFunnelParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::QueueStakeAmbaForSamba => Ok(Self::QueueStakeAmbaForSamba {
                params: QueueStakeAmbaForSambaParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::ActivateQueuedStakeAmbaForSamba => {
                Ok(Self::ActivateQueuedStakeAmbaForSamba {
                    params: ActivateQueuedStakeAmbaForSambaParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::CancelQueuedStakeAmba => Ok(Self::CancelQueuedStakeAmba {
                params: CancelQueuedStakeAmbaParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::RequestUnstakeSamba => Ok(Self::RequestUnstakeSamba {
                params: RequestUnstakeSambaParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::CompleteUnstakeSamba => Ok(Self::CompleteUnstakeSamba {
                params: CompleteUnstakeSambaParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::AdminAssistedWithdrawCollateral => {
                Ok(Self::AdminAssistedWithdrawCollateral {
                    amount: u64::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::ProposeOracleSourceV3 => Ok(Self::ProposeOracleSourceV3 {
                params: ProposeOracleSourceV3Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::SupportOracleSourceV3 => Ok(Self::SupportOracleSourceV3 {
                params: SupportOracleSourceV3Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::FinalizeOracleSkuCoverage => Ok(Self::FinalizeOracleSkuCoverage),
            VaultInstructionTag::ResolveOracleSourceChallengeV2 => {
                Ok(Self::ResolveOracleSourceChallengeV2 {
                    params: ResolveOracleSourceChallengeParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::CancelStaleOracleUpdateClaimV2 => {
                Ok(Self::CancelStaleOracleUpdateClaimV2)
            }
            VaultInstructionTag::AbortStaleOracleUpdateEmergencyDisputeV2 => {
                Ok(Self::AbortStaleOracleUpdateEmergencyDisputeV2)
            }
            VaultInstructionTag::ReopenOracleSkuCoverage => Ok(Self::ReopenOracleSkuCoverage),
            VaultInstructionTag::BeginOracleRecipeWeightsV3 => {
                Ok(Self::BeginOracleRecipeWeightsV3 {
                    params: BeginOracleRecipeWeightsV2Params::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::AccumulateOracleRecipeBucketV2 => {
                Ok(Self::AccumulateOracleRecipeBucketV2 {
                    params: AccumulateOracleRecipeBucketV2Params::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::FinalizeOracleRecipeWeightsV2 => {
                Ok(Self::FinalizeOracleRecipeWeightsV2)
            }
            VaultInstructionTag::AccumulateOracleSettlementSourceBucket => {
                Ok(Self::AccumulateOracleSettlementSourceBucket {
                    params: AccumulateOracleSettlementSourceBucketParams::deserialize_reader(
                        reader,
                    )?,
                })
            }
            VaultInstructionTag::BeginOracleActiveWeights => Ok(Self::BeginOracleActiveWeights {
                params: BeginOracleActiveWeightsParams::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::AccumulateOracleActiveWeightGroup => {
                Ok(Self::AccumulateOracleActiveWeightGroup {
                    params: AccumulateOracleActiveWeightGroupParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::FinalizeOracleActiveWeights => {
                Ok(Self::FinalizeOracleActiveWeights)
            }
            VaultInstructionTag::RecomputeOracleBucketMedianV1 => {
                Ok(Self::RecomputeOracleBucketMedianV1 {
                    params: RecomputeOracleBucketMedianV1Params::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::FinalizeOracleOpeningPhase => Ok(Self::FinalizeOracleOpeningPhase),
            VaultInstructionTag::ExpireOracleOpeningSource => Ok(Self::ExpireOracleOpeningSource),
            VaultInstructionTag::TryOpenOracleEmergencyDisputeV2 => {
                Ok(Self::TryOpenOracleEmergencyDisputeV2 {
                    params: TryOpenOracleEmergencyDisputeParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::CommitOracleEmergencyVoteV3 => {
                Ok(Self::CommitOracleEmergencyVoteV3 {
                    params: CommitOracleEmergencyVoteV2Params::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::RevealOracleEmergencyVoteV2 => {
                Ok(Self::RevealOracleEmergencyVoteV2 {
                    params: RevealOracleEmergencyVoteParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::ResolveOracleEmergencyDisputeV2 => {
                Ok(Self::ResolveOracleEmergencyDisputeV2 {
                    params: ResolveOracleEmergencyDisputeParams::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::RegisterOracleSambaWinningVote => {
                Ok(Self::RegisterOracleSambaWinningVote)
            }
            VaultInstructionTag::SettleOracleSambaEmergencyVoteV2 => {
                Ok(Self::SettleOracleSambaEmergencyVoteV2)
            }
            VaultInstructionTag::CloseOracleMonth => Ok(Self::CloseOracleMonth),
            VaultInstructionTag::FinalizeOracleMonth => Ok(Self::FinalizeOracleMonth),
            VaultInstructionTag::InitializeWriterPolicyRegistryV1 => {
                Ok(Self::InitializeWriterPolicyRegistryV1 {
                    params: InitializeWriterPolicyRegistryV1Params::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::ManageWriterPolicyAuthorityV1 => {
                Ok(Self::ManageWriterPolicyAuthorityV1 {
                    params: ManageWriterPolicyAuthorityV1Params::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::InitializeWriterSettlementGroupV1 => {
                Ok(Self::InitializeWriterSettlementGroupV1)
            }
            VaultInstructionTag::InitializeWriterSleeveV1 => Ok(Self::InitializeWriterSleeveV1),
            VaultInstructionTag::RegisterWriterSeriesV1 => Ok(Self::RegisterWriterSeriesV1),
            VaultInstructionTag::SealWriterPolicyV1 => Ok(Self::SealWriterPolicyV1 {
                params: SealWriterPolicyV1Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::OpenWriterFundingV1 => Ok(Self::OpenWriterFundingV1),
            VaultInstructionTag::DepositWriterPrincipalV1 => Ok(Self::DepositWriterPrincipalV1 {
                params: WriterAmountV1Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::WithdrawWriterPrincipalV1 => Ok(Self::WithdrawWriterPrincipalV1 {
                params: WriterAmountV1Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::ActivateWriterSleeveV1 => Ok(Self::ActivateWriterSleeveV1),
            VaultInstructionTag::SetCollectiveMarketPausedV1 => {
                Ok(Self::SetCollectiveMarketPausedV1 {
                    params: SetCollectiveMarketPausedV1Params::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::ReconcileWriterSupplyV1 => Ok(Self::ReconcileWriterSupplyV1 {
                params: ReconcileWriterSupplyV1Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::CleanupWriterCustodyV1 => Ok(Self::CleanupWriterCustodyV1 {
                params: CleanupWriterCustodyV1Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::CommitWriterAuctionV1 => Ok(Self::CommitWriterAuctionV1 {
                params: CommitWriterAuctionV1Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::PlaceWriterBidV1 => Ok(Self::PlaceWriterBidV1 {
                params: PlaceWriterBidV1Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::CancelOrRefundWriterBidV1 => Ok(Self::CancelOrRefundWriterBidV1),
            VaultInstructionTag::RevealWriterAuctionV1 => Ok(Self::RevealWriterAuctionV1 {
                params: Box::new(RevealWriterAuctionV1Params::deserialize_reader(reader)?),
            }),
            VaultInstructionTag::PlanWriterAuctionChunkV1 => Ok(Self::PlanWriterAuctionChunkV1 {
                params: PlanWriterAuctionChunkV1Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::ExecuteWriterAuctionFillV1 => Ok(Self::ExecuteWriterAuctionFillV1),
            VaultInstructionTag::FinalizeOrAbortWriterAuctionV1 => {
                Ok(Self::FinalizeOrAbortWriterAuctionV1 {
                    params: FinalizeOrAbortWriterAuctionV1Params::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::BeginWriterCloseV1 => Ok(Self::BeginWriterCloseV1 {
                params: BeginWriterCloseV1Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::DepositWriterCloseBasketV1 => {
                Ok(Self::DepositWriterCloseBasketV1 {
                    params: WriterSeriesIndexV1Params::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::FinalizeWriterCloseV1 => Ok(Self::FinalizeWriterCloseV1),
            VaultInstructionTag::ProcessWriterCloseCancellationV1 => {
                Ok(Self::ProcessWriterCloseCancellationV1 {
                    params: ProcessWriterCloseCancellationV1Params::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::PublishWriterGroupSettlementV1 => {
                Ok(Self::PublishWriterGroupSettlementV1)
            }
            VaultInstructionTag::FinalizeWriterSleeveSettlementV1 => {
                Ok(Self::FinalizeWriterSleeveSettlementV1)
            }
            VaultInstructionTag::ClaimCollectiveLongV1 => Ok(Self::ClaimCollectiveLongV1 {
                params: ClaimCollectiveLongV1Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::ClaimWriterFlatResidualV1 => Ok(Self::ClaimWriterFlatResidualV1 {
                params: ClaimWriterFlatResidualV1Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::CloseWriterSleeveV1 => Ok(Self::CloseWriterSleeveV1),
            VaultInstructionTag::ScopedCollectiveSettlementV1 => {
                Ok(Self::ScopedCollectiveSettlementV1 {
                    action: ScopedSettlementActionV1::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::ScopedPositionSettlementV1 => {
                Ok(Self::ScopedPositionSettlementV1 {
                    action: ScopedSettlementActionV1::deserialize_reader(reader)?,
                })
            }
            VaultInstructionTag::PrepareWriterBidIndexV1 => Ok(Self::PrepareWriterBidIndexV1 {
                params: PrepareWriterBidIndexV1Params::deserialize_reader(reader)?,
            }),
            VaultInstructionTag::ExecuteCompressedStateV1 => Ok(Self::ExecuteCompressedStateV1 {
                params: ExecuteCompressedStateParams::deserialize_reader(reader)?,
            }),
        }
    }
}
