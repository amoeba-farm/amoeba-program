use super::*;

impl BorshDeserialize for OracleInstruction {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let tag_byte = u8::deserialize_reader(reader)?;
        let tag = OracleInstructionTag::from_byte(tag_byte)
            .ok_or_else(|| invalid_instruction_tag_error(tag_byte))?;
        match tag {
            OracleInstructionTag::InitializeConfig => Ok(Self::InitializeConfig {
                admin: Pubkey::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::CreatePeriod => Ok(Self::CreatePeriod {
                params: CreatePeriodParams::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::UpdateConfig => Ok(Self::UpdateConfig {
                new_admin: Option::<Pubkey>::deserialize_reader(reader)?,
                new_oracle_authority: Option::<Pubkey>::deserialize_reader(reader)?,
                new_usdc_mint: Option::<Pubkey>::deserialize_reader(reader)?,
                new_bond_vault: Option::<Pubkey>::deserialize_reader(reader)?,
                paused: Option::<bool>::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::RotateVaultAuthoritiesV2 => Ok(Self::RotateVaultAuthoritiesV2 {
                params: RotateVaultAuthoritiesV2Params::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::BootstrapVaultGovernanceV2 => {
                Ok(Self::BootstrapVaultGovernanceV2 {
                    params: BootstrapVaultGovernanceV2Params::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::ActivateVaultV2 => Ok(Self::ActivateVaultV2 {
                params: ActivateVaultV2Params::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::CfmLaunchV1 => Ok(Self::CfmLaunchV1 {
                params: CfmLaunchV1Params::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::InitUserCollateral => Ok(Self::InitUserCollateral),
            OracleInstructionTag::DepositCollateral => Ok(Self::DepositCollateral {
                amount: u64::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::WithdrawCollateral => Ok(Self::WithdrawCollateral {
                amount: u64::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::OracleCarryForwardV1 => Ok(Self::OracleCarryForwardV1 {
                action: OracleCarryForwardActionV1::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::AdminAssistedWithdrawCollateral => {
                Ok(Self::AdminAssistedWithdrawCollateral {
                    amount: u64::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::ConfigureOracleEconomicsTemplateV2 => {
                Ok(Self::ConfigureOracleEconomicsTemplateV2 {
                    params: ConfigureOracleEconomicsTemplateV2Params::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::InitializeOracleMonthV5 => Ok(Self::InitializeOracleMonthV5 {
                params: InitializeOracleMonthV5Params::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::InitializeOracleUsdcRewardVault => {
                Ok(Self::InitializeOracleUsdcRewardVault)
            }
            OracleInstructionTag::DepositOracleUsdcRewards => Ok(Self::DepositOracleUsdcRewards {
                params: DepositOracleUsdcRewardsParams::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::BeginOracleUsdcRewardSchedule => {
                Ok(Self::BeginOracleUsdcRewardSchedule {
                    params: BeginOracleUsdcRewardScheduleParams::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::AddOracleUsdcSkuBudget => Ok(Self::AddOracleUsdcSkuBudget {
                params: AddOracleUsdcSkuBudgetParams::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::FinalizeOracleUsdcRewardSchedule => {
                Ok(Self::FinalizeOracleUsdcRewardSchedule)
            }
            OracleInstructionTag::ChallengeOracleSourceV2 => Ok(Self::ChallengeOracleSourceV2 {
                params: ChallengeOracleSourceParams::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::SettleOracleUsdcEscrow => Ok(Self::SettleOracleUsdcEscrow {
                params: SettleOracleEscrowParams::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::ProposeOracleSourceV3 => Ok(Self::ProposeOracleSourceV3 {
                params: ProposeOracleSourceV3Params::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::SupportOracleSourceV3 => Ok(Self::SupportOracleSourceV3 {
                params: SupportOracleSourceV3Params::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::FinalizeOracleSkuCoverage => Ok(Self::FinalizeOracleSkuCoverage),
            OracleInstructionTag::ResolveOracleSourceChallengeV2 => {
                Ok(Self::ResolveOracleSourceChallengeV2 {
                    params: ResolveOracleSourceChallengeParams::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::ReopenOracleSkuCoverage => Ok(Self::ReopenOracleSkuCoverage),
            OracleInstructionTag::ExpireUnlistableOracleSourceV2 => {
                Ok(Self::ExpireUnlistableOracleSourceV2)
            }
            OracleInstructionTag::CancelStaleOracleSourceChallengeV2 => {
                Ok(Self::CancelStaleOracleSourceChallengeV2)
            }
            OracleInstructionTag::SettleFailedOracleMonthEscrowV2 => {
                Ok(Self::SettleFailedOracleMonthEscrowV2 {
                    params: SettleOracleEscrowParams::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::AbortOracleUsdcRewardScheduleV2 => {
                Ok(Self::AbortOracleUsdcRewardScheduleV2)
            }
            OracleInstructionTag::TimeoutUnsupportedOracleSourceV2 => {
                Ok(Self::TimeoutUnsupportedOracleSourceV2)
            }
            OracleInstructionTag::FinalizeOracleOpeningPhase => {
                Ok(Self::FinalizeOracleOpeningPhase)
            }
            OracleInstructionTag::ExpireOracleOpeningSource => Ok(Self::ExpireOracleOpeningSource),
            OracleInstructionTag::AccumulateOracleRecipeBucketV2 => {
                Ok(Self::AccumulateOracleRecipeBucketV2 {
                    params: AccumulateOracleRecipeBucketV2Params::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::FinalizeOracleRecipeWeightsV2 => {
                Ok(Self::FinalizeOracleRecipeWeightsV2)
            }
            OracleInstructionTag::AccumulateOracleSettlementSourceBucket => {
                Ok(Self::AccumulateOracleSettlementSourceBucket {
                    params: AccumulateOracleSettlementSourceBucketParams::deserialize_reader(
                        reader,
                    )?,
                })
            }
            OracleInstructionTag::BeginOracleActiveWeights => Ok(Self::BeginOracleActiveWeights {
                params: BeginOracleActiveWeightsParams::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::AccumulateOracleActiveWeightGroup => {
                Ok(Self::AccumulateOracleActiveWeightGroup {
                    params: AccumulateOracleActiveWeightGroupParams::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::FinalizeOracleActiveWeights => {
                Ok(Self::FinalizeOracleActiveWeights)
            }
            OracleInstructionTag::SubmitOracleOpeningClaimV2 => {
                Ok(Self::SubmitOracleOpeningClaimV2 {
                    params: SubmitOracleOpeningClaimParams::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::ChallengeOracleOpeningClaimV2 => {
                Ok(Self::ChallengeOracleOpeningClaimV2 {
                    params: ChallengeOracleOpeningClaimParams::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::ResolveOracleOpeningClaimChallengeV2 => {
                Ok(Self::ResolveOracleOpeningClaimChallengeV2 {
                    params: ResolveOracleOpeningClaimChallengeParams::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::FinalizeOracleOpeningClaimV2 => {
                Ok(Self::FinalizeOracleOpeningClaimV2)
            }
            OracleInstructionTag::BeginOracleRecipeWeightsV3 => {
                Ok(Self::BeginOracleRecipeWeightsV3 {
                    params: BeginOracleRecipeWeightsV2Params::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::RecomputeOracleBucketMedianV1 => {
                Ok(Self::RecomputeOracleBucketMedianV1 {
                    params: RecomputeOracleBucketMedianV1Params::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::IndexOracleRecipeSourceV1 => {
                Ok(Self::IndexOracleRecipeSourceV1 {
                    params: IndexOracleRecipeSourceV1Params::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::ConfigureOracleProductSkuManifest => {
                Ok(Self::ConfigureOracleProductSkuManifest {
                    params: ConfigureOracleProductSkuManifestParams::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::CommitOracleUpdateClaimV3 => {
                Ok(Self::CommitOracleUpdateClaimV3 {
                    params: CommitOracleUpdateClaimV2Params::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::SettleExpiredOracleUpdateCommitmentV3 => {
                Ok(Self::SettleExpiredOracleUpdateCommitmentV3)
            }
            OracleInstructionTag::ChallengeOracleUpdateClaimV2 => {
                Ok(Self::ChallengeOracleUpdateClaimV2 {
                    params: ChallengeOracleUpdateClaimParams::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::RevealOracleUpdateClaimV3 => {
                Ok(Self::RevealOracleUpdateClaimV3 {
                    params: RevealOracleUpdateClaimV3Params::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::FinalizeOracleUpdateClaimV2 => {
                Ok(Self::FinalizeOracleUpdateClaimV2 {
                    params: FinalizeOracleUpdateClaimV2Params::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::CancelStaleOracleUpdateClaimV2 => {
                Ok(Self::CancelStaleOracleUpdateClaimV2)
            }
            OracleInstructionTag::CloseOracleMonth => Ok(Self::CloseOracleMonth),
            OracleInstructionTag::FinalizeOracleMonth => Ok(Self::FinalizeOracleMonth),
            OracleInstructionTag::FailPeriod => Ok(Self::FailPeriod),
            OracleInstructionTag::InitializeSettlementSignerRegistry => {
                Ok(Self::InitializeSettlementSignerRegistry {
                    params: InitializeSettlementSignerRegistryParams::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::ProposeSettlementSignerRotation => {
                Ok(Self::ProposeSettlementSignerRotation {
                    params: ProposeSettlementSignerRotationParams::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::ActivateSettlementSignerRotation => {
                Ok(Self::ActivateSettlementSignerRotation)
            }
            OracleInstructionTag::CancelSettlementSignerRotation => {
                Ok(Self::CancelSettlementSignerRotation)
            }
            OracleInstructionTag::ProposeEmergencySettlementSignerRecovery => {
                Ok(Self::ProposeEmergencySettlementSignerRecovery {
                    params: ProposeSettlementSignerRotationParams::deserialize_reader(reader)?,
                })
            }
            OracleInstructionTag::UpsertSettlementV3 => Ok(Self::UpsertSettlementV3 {
                params: UpsertSettlementParams::deserialize_reader(reader)?,
            }),
            OracleInstructionTag::RegisterOracleUsdcRewardSource => {
                Ok(Self::RegisterOracleUsdcRewardSource)
            }
            OracleInstructionTag::RegisterOracleUsdcRewardUpdate => {
                Ok(Self::RegisterOracleUsdcRewardUpdate)
            }
            OracleInstructionTag::FinalizeOracleUsdcRewardEntitlements => {
                Ok(Self::FinalizeOracleUsdcRewardEntitlements)
            }
            OracleInstructionTag::ClaimOracleUsdcReward => Ok(Self::ClaimOracleUsdcReward {
                params: ClaimOracleUsdcRewardParams::deserialize_reader(reader)?,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<OracleInstruction> {
        vec![
            OracleInstruction::InitializeConfig {
                admin: Pubkey::new_unique(),
            },
            OracleInstruction::CreatePeriod {
                params: CreatePeriodParams {
                    product_id: [3; 32],
                    planned_listing_ts: 10,
                    expiry_ts: u64::MAX,
                    value_decimals: 6,
                },
            },
            OracleInstruction::UpdateConfig {
                new_admin: Some(Pubkey::new_unique()),
                new_oracle_authority: None,
                new_usdc_mint: Some(Pubkey::new_unique()),
                new_bond_vault: None,
                paused: Some(true),
            },
            OracleInstruction::RotateVaultAuthoritiesV2 {
                params: RotateVaultAuthoritiesV2Params {
                    new_admin: Pubkey::new_unique(),
                    new_oracle_authority: Pubkey::new_unique(),
                },
            },
            OracleInstruction::BootstrapVaultGovernanceV2 {
                params: BootstrapVaultGovernanceV2Params {
                    new_oracle_authority: Pubkey::new_unique(),
                },
            },
            OracleInstruction::ActivateVaultV2 {
                params: ActivateVaultV2Params {
                    expected_collateral_freeze_authority: Some(Pubkey::new_unique()),
                },
            },
            OracleInstruction::CfmLaunchV1 {
                params: CfmLaunchV1Params {
                    operation: 1,
                    product: 1,
                    row: 21,
                    plan_hash: [9; 32],
                },
            },
            OracleInstruction::InitUserCollateral,
            OracleInstruction::DepositCollateral { amount: 7 },
            OracleInstruction::WithdrawCollateral { amount: 8 },
            OracleInstruction::OracleCarryForwardV1 {
                action: OracleCarryForwardActionV1::BackfillClaimEvidence { role: 4 },
            },
            OracleInstruction::AdminAssistedWithdrawCollateral { amount: 9 },
            OracleInstruction::ConfigureOracleEconomicsTemplateV2 {
                params: ConfigureOracleEconomicsTemplateV2Params {
                    expected_config_version: 4,
                    economics: OracleEconomicParams::default(),
                },
            },
            OracleInstruction::FinalizeOracleOpeningPhase,
            OracleInstruction::ExpireOracleOpeningSource,
            OracleInstruction::AccumulateOracleRecipeBucketV2 {
                params: AccumulateOracleRecipeBucketV2Params {
                    bucket_id: [30; 32],
                    bucket_weight_bps: 31,
                    finalize_collection: true,
                },
            },
            OracleInstruction::FinalizeOracleRecipeWeightsV2,
            OracleInstruction::AccumulateOracleSettlementSourceBucket {
                params: AccumulateOracleSettlementSourceBucketParams {
                    bucket_id: [32; 32],
                    finalize_collection: false,
                },
            },
            OracleInstruction::BeginOracleActiveWeights {
                params: BeginOracleActiveWeightsParams {
                    expected_source_count: 33,
                    expected_group_count: 34,
                },
            },
            OracleInstruction::AccumulateOracleActiveWeightGroup {
                params: AccumulateOracleActiveWeightGroupParams {
                    group_id: [35; 32],
                    finalize_collection: true,
                },
            },
            OracleInstruction::FinalizeOracleActiveWeights,
            OracleInstruction::SubmitOracleOpeningClaimV2 {
                params: SubmitOracleOpeningClaimParams {
                    opening_state: 36,
                    source_time: 37,
                    stake: 38,
                    canonical_locator_hash: [39; 32],
                    source_definition_hash: [40; 32],
                    archive_url: String::new(),
                },
            },
            OracleInstruction::ChallengeOracleOpeningClaimV2 {
                params: ChallengeOracleOpeningClaimParams {
                    challenge_id: [41; 32],
                    alternative_opening_state: 42,
                    alternative_source_time: 43,
                    bond: 44,
                    canonical_locator_hash: [45; 32],
                    source_definition_hash: [46; 32],
                    archive_url: String::new(),
                },
            },
            OracleInstruction::ResolveOracleOpeningClaimChallengeV2 {
                params: ResolveOracleOpeningClaimChallengeParams {
                    outcome: OracleOpeningChallengeOutcome::RejectOpeningForRetry,
                },
            },
            OracleInstruction::FinalizeOracleOpeningClaimV2,
            OracleInstruction::BeginOracleRecipeWeightsV3 {
                params: BeginOracleRecipeWeightsV2Params {
                    expected_source_count: 47,
                    expected_bucket_count: 48,
                },
            },
            OracleInstruction::RecomputeOracleBucketMedianV1 {
                params: RecomputeOracleBucketMedianV1Params {
                    bucket_id: [49; 32],
                    mode: 1,
                },
            },
            OracleInstruction::IndexOracleRecipeSourceV1 {
                params: IndexOracleRecipeSourceV1Params {
                    previous_hash: [50; 32],
                    bucket_id: [51; 32],
                    source_id: [52; 32],
                    source_type_hash: [53; 32],
                    canonical_locator_hash: [54; 32],
                    source_definition_hash: [55; 32],
                    bucket_weight_bps: 56,
                },
            },
            OracleInstruction::InitializeOracleUsdcRewardVault,
            OracleInstruction::DepositOracleUsdcRewards {
                params: DepositOracleUsdcRewardsParams { amount: 11 },
            },
            OracleInstruction::BeginOracleUsdcRewardSchedule {
                params: BeginOracleUsdcRewardScheduleParams,
            },
            OracleInstruction::AddOracleUsdcSkuBudget {
                params: AddOracleUsdcSkuBudgetParams {
                    bucket_id: [12; 32],
                    source_reward_budget: 1,
                    opening_reward_budget: 2,
                    update_reward_budget: 3,
                    proposer_reward_bps: 4,
                    listing_bond: 5,
                    support_bond: 6,
                    opening_bond: 7,
                    update_min_bond: 8,
                    challenge_min_bond: 9,
                    challenge_max_bond: 10,
                    challenge_bond_bps: 11,
                },
            },
            OracleInstruction::FinalizeOracleUsdcRewardSchedule,
            OracleInstruction::ChallengeOracleSourceV2 {
                params: ChallengeOracleSourceParams {
                    challenge_id: [13; 32],
                    reason: 8,
                    bond: 14,
                    evidence_hash: [15; 32],
                    comparison_source_id: [16; 32],
                },
            },
            OracleInstruction::SettleOracleUsdcEscrow {
                params: SettleOracleEscrowParams {
                    kind: OracleEscrowKind::UpdateChallenge,
                },
            },
            OracleInstruction::ProposeOracleSourceV3 {
                params: ProposeOracleSourceV3Params {
                    source_id: [17; 32],
                    bucket_id: [18; 32],
                    source_type_hash: [19; 32],
                    canonical_locator_hash: [20; 32],
                    source_definition_hash: [21; 32],
                    listing_bond: 22,
                    sku_index: 23,
                    sku_proof: vec![[24; 32]; 3],
                },
            },
            OracleInstruction::SupportOracleSourceV3 {
                params: SupportOracleSourceV3Params {
                    stake: 25,
                    sku_index: 26,
                    sku_proof: vec![[27; 32]],
                },
            },
            OracleInstruction::FinalizeOracleSkuCoverage,
            OracleInstruction::ResolveOracleSourceChallengeV2 {
                params: ResolveOracleSourceChallengeParams {
                    outcome: OracleSourceChallengeOutcome::MergeSource,
                },
            },
            OracleInstruction::ReopenOracleSkuCoverage,
            OracleInstruction::ExpireUnlistableOracleSourceV2,
            OracleInstruction::CancelStaleOracleSourceChallengeV2,
            OracleInstruction::SettleFailedOracleMonthEscrowV2 {
                params: SettleOracleEscrowParams {
                    kind: OracleEscrowKind::SourceChallenge,
                },
            },
            OracleInstruction::AbortOracleUsdcRewardScheduleV2,
            OracleInstruction::TimeoutUnsupportedOracleSourceV2,
            OracleInstruction::InitializeOracleMonthV5 {
                params: InitializeOracleMonthV5Params {
                    scramble_start_ts: 1,
                    listing_ts: 2,
                    settlement_base_oracle_atomic: 3,
                    required_sku_root: [4; 32],
                    required_sku_count: 52,
                },
            },
            OracleInstruction::ConfigureOracleProductSkuManifest {
                params: ConfigureOracleProductSkuManifestParams {
                    underlying_id: [5; 32],
                    draft_nonce: 6,
                    expected_start_index: 0,
                    sku_count: 52,
                    expected_root: [7; 32],
                    sku_id_chunk: vec![[8; 32], [9; 32]],
                },
            },
            OracleInstruction::CommitOracleUpdateClaimV3 {
                params: CommitOracleUpdateClaimV2Params {
                    claim_id: [60; 32],
                    commit_hash: [61; 32],
                    stake: 62,
                },
            },
            OracleInstruction::SettleExpiredOracleUpdateCommitmentV3,
            OracleInstruction::ChallengeOracleUpdateClaimV2 {
                params: ChallengeOracleUpdateClaimParams {
                    challenge_id: [63; 32],
                    alternative_state: 64,
                    alternative_source_time: 65,
                    bond: 66,
                    evidence_hash: [67; 32],
                    archive_url: String::new(),
                },
            },
            OracleInstruction::RevealOracleUpdateClaimV3 {
                params: RevealOracleUpdateClaimV3Params {
                    claim_id: [68; 32],
                    prior_state: 69,
                    new_state: 70,
                    source_time: 71,
                    evidence_hash: [72; 32],
                    archive_url: String::new(),
                    secret_salt: [73; 32],
                },
            },
            OracleInstruction::FinalizeOracleUpdateClaimV2 {
                params: FinalizeOracleUpdateClaimV2Params {
                    outcome: OracleUpdateClaimOutcome::RejectClaim,
                    current_step: 74,
                },
            },
            OracleInstruction::CancelStaleOracleUpdateClaimV2,
            OracleInstruction::CloseOracleMonth,
            OracleInstruction::FinalizeOracleMonth,
            OracleInstruction::FailPeriod,
            OracleInstruction::InitializeSettlementSignerRegistry {
                params: InitializeSettlementSignerRegistryParams {
                    signer_set_version: 1,
                    threshold: 2,
                    signer_count: 3,
                    signers: [Pubkey::new_from_array([75; 32]);
                        crate::constants::MAX_SETTLEMENT_SIGNER_COUNT],
                    rotation_delay_slots: 76,
                    recovery_authority: Pubkey::new_from_array([77; 32]),
                },
            },
            OracleInstruction::ProposeSettlementSignerRotation {
                params: ProposeSettlementSignerRotationParams {
                    target_version: 2,
                    threshold: 2,
                    signer_count: 3,
                    signers: [Pubkey::new_from_array([78; 32]);
                        crate::constants::MAX_SETTLEMENT_SIGNER_COUNT],
                    rotation_delay_slots: 79,
                    activate_after_slot: 80,
                },
            },
            OracleInstruction::ActivateSettlementSignerRotation,
            OracleInstruction::CancelSettlementSignerRotation,
            OracleInstruction::ProposeEmergencySettlementSignerRecovery {
                params: ProposeSettlementSignerRotationParams {
                    target_version: 3,
                    threshold: 1,
                    signer_count: 1,
                    signers: [Pubkey::new_from_array([81; 32]);
                        crate::constants::MAX_SETTLEMENT_SIGNER_COUNT],
                    rotation_delay_slots: 82,
                    activate_after_slot: 83,
                },
            },
            OracleInstruction::UpsertSettlementV3 {
                params: UpsertSettlementParams {
                    settlement: SettlementLeaf {
                        schema_version: SettlementLeaf::CURRENT_SCHEMA_VERSION,
                        oracle_month: Pubkey::new_from_array([84; 32]),
                        item_id: "ramx".into(),
                        expiry_id: "RAMX-202703".into(),
                        observations: vec![SettlementObservation {
                            observed_at_ts: 85,
                            price_atomic: 86,
                        }],
                        source_uri: "https://oracle.example/ramx/RAMX-202703".into(),
                        index_delta_bps: -87,
                        ..SettlementLeaf::default()
                    },
                },
            },
            OracleInstruction::RegisterOracleUsdcRewardSource,
            OracleInstruction::RegisterOracleUsdcRewardUpdate,
            OracleInstruction::FinalizeOracleUsdcRewardEntitlements,
            OracleInstruction::ClaimOracleUsdcReward {
                params: ClaimOracleUsdcRewardParams {
                    kind: OracleUsdcRewardKind::SourceSupport,
                },
            },
        ]
    }

    #[test]
    fn every_instruction_roundtrips_and_leads_with_its_tag() {
        let mut samples = samples();
        samples.sort_by_key(|instruction| instruction.try_to_vec().unwrap()[0]);
        assert_eq!(samples.len(), OracleInstructionTag::ALL.len());
        for (instruction, tag) in samples.into_iter().zip(OracleInstructionTag::ALL) {
            let encoded = instruction.try_to_vec().unwrap();
            assert_eq!(encoded[0], tag as u8);
            assert_eq!(
                OracleInstruction::try_from_slice(&encoded).unwrap(),
                instruction
            );
            assert!(OracleInstruction::try_from_slice(&encoded[..encoded.len() - 1]).is_err());
            let mut trailing = encoded.clone();
            trailing.push(0);
            assert!(OracleInstruction::try_from_slice(&trailing).is_err());
        }
    }

    #[test]
    fn scaffold_wire_for_tags_zero_and_one_is_unchanged() {
        let admin = Pubkey::new_unique();
        assert_eq!(
            OracleInstruction::InitializeConfig { admin }
                .try_to_vec()
                .unwrap(),
            [&[0u8][..], admin.as_ref()].concat()
        );
        let params = CreatePeriodParams {
            product_id: [3; 32],
            planned_listing_ts: 11,
            expiry_ts: 12,
            value_decimals: 6,
        };
        assert_eq!(
            OracleInstruction::CreatePeriod {
                params: params.clone()
            }
            .try_to_vec()
            .unwrap(),
            [
                &[1u8][..],
                &params.product_id,
                &11u64.to_le_bytes(),
                &12u64.to_le_bytes(),
                &[6],
            ]
            .concat()
        );
    }

    #[test]
    fn unknown_tags_reject_before_payload_decoding() {
        for tag in 0u8..=u8::MAX {
            if OracleInstructionTag::from_byte(tag).is_none() {
                assert!(OracleInstruction::try_from_slice(&[tag]).is_err());
                assert!(OracleInstruction::try_from_slice(&[tag, 0, 0, 0]).is_err());
            }
        }
        assert!(OracleInstruction::try_from_slice(&[]).is_err());
    }
}
