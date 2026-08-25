use super::*;

pub(super) fn process_instruction_with_context(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
    compressed_state_transport: bool,
) -> ProgramResult {
    if instruction_data.len() > MAX_INSTRUCTION_DATA_BYTES {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let (tag_bytes, payload) = instruction_data
        .split_first()
        .ok_or(VaultError::InvalidInstructionData)?;
    if !compressed_state_transport {
        if let Some(tag) =
            crate::ameba_dlmm_instruction::AmoebaDlmmInstructionTag::from_byte(*tag_bytes)
        {
            if matches!(
                tag,
                crate::ameba_dlmm_instruction::AmoebaDlmmInstructionTag::InitializeLightConfig
                    | crate::ameba_dlmm_instruction::AmoebaDlmmInstructionTag::UpdateLightConfig
                    | crate::ameba_dlmm_instruction::AmoebaDlmmInstructionTag::CompressLightState
                    | crate::ameba_dlmm_instruction::AmoebaDlmmInstructionTag::DecompressLightState
            ) {
                return ameba_dlmm_light::process_lifecycle_instruction(
                    program_id, accounts, tag, payload,
                );
            }
            return ameba_dlmm::process_instruction(program_id, accounts, tag, payload);
        }
    }
    #[cfg(feature = "devnet-solo-backfill-2026")]
    if devnet_solo_backfill_2026::is_instruction_tag(*tag_bytes) {
        if compressed_state_transport {
            return Err(VaultError::InvalidInstructionData.into());
        }
        return devnet_solo_backfill_2026::process_instruction(
            program_id, accounts, *tag_bytes, payload,
        );
    }
    let tag =
        VaultInstructionTag::from_byte(*tag_bytes).ok_or(VaultError::InvalidInstructionData)?;

    match tag as u8 {
        0..=63 => dispatch_0_63(
            program_id,
            accounts,
            tag,
            payload,
            compressed_state_transport,
        ),
        64..=124 => dispatch_64_124(
            program_id,
            accounts,
            tag,
            payload,
            compressed_state_transport,
        ),
        128..=158 => dispatch_128_158(
            program_id,
            accounts,
            tag,
            payload,
            compressed_state_transport,
        ),
        161..=205 => dispatch_161_205(
            program_id,
            accounts,
            tag,
            payload,
            compressed_state_transport,
        ),
        220..=248 => writer_sleeve::process_instruction(
            program_id,
            accounts,
            tag,
            payload,
            compressed_state_transport,
        ),
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

#[inline(never)]
pub(super) fn dispatch_0_63(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    tag: VaultInstructionTag,
    payload: &[u8],
    compressed_state_transport: bool,
) -> ProgramResult {
    let _ = compressed_state_transport;
    let handler: fn(&Pubkey, &[AccountInfo], &[u8]) -> ProgramResult = match tag {
        VaultInstructionTag::Initialize => process_initialize_instruction,
        VaultInstructionTag::UpdateConfig => process_update_config_instruction,
        VaultInstructionTag::DepositUsdc => process_deposit_instruction,
        VaultInstructionTag::InitUserCollateral => process_init_user_collateral_instruction,
        VaultInstructionTag::DepositCollateral => process_deposit_collateral_instruction,
        VaultInstructionTag::WithdrawCollateral => process_withdraw_collateral_instruction,
        VaultInstructionTag::ConfigureOracleMajorToken => {
            process_configure_oracle_major_token_instruction
        }
        VaultInstructionTag::CloseOracleMonth => process_close_oracle_month_instruction,
        _ => return Err(VaultError::InvalidInstructionData.into()),
    };
    handler(program_id, accounts, payload)
}

#[inline(never)]
pub(super) fn dispatch_64_124(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    tag: VaultInstructionTag,
    payload: &[u8],
    compressed_state_transport: bool,
) -> ProgramResult {
    let _ = compressed_state_transport;
    match tag {
        VaultInstructionTag::RotateVaultAuthoritiesV2 => {
            let params: RotateVaultAuthoritiesV2Params = decode_instruction_payload(payload)?;
            process_rotate_vault_authorities_v2(program_id, accounts, params)
        }
        VaultInstructionTag::InitMarketV2 => {
            process_init_market_v2_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::SetMarketPaused => {
            process_set_market_paused_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::InitializeSettlementSignerRegistry => {
            process_initialize_settlement_signer_registry_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::ProposeSettlementSignerRotation => {
            process_propose_settlement_signer_rotation_instruction(
                program_id, accounts, payload, false,
            )
        }
        VaultInstructionTag::ActivateSettlementSignerRotation => {
            expect_empty_payload(payload)?;
            process_activate_settlement_signer_rotation(program_id, accounts)
        }
        VaultInstructionTag::CancelSettlementSignerRotation => {
            expect_empty_payload(payload)?;
            process_cancel_settlement_signer_rotation(program_id, accounts)
        }
        VaultInstructionTag::ProposeEmergencySettlementSignerRecovery => {
            process_propose_settlement_signer_rotation_instruction(
                program_id, accounts, payload, true,
            )
        }
        VaultInstructionTag::UpsertSettlementV3 => {
            process_upsert_settlement_v3_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::UpsertMarketPageV2 => {
            process_upsert_market_page_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::ConfigureOracleEconomicsTemplateV2 => {
            process_configure_oracle_economics_template_v2_instruction(
                program_id, accounts, payload,
            )
        }
        VaultInstructionTag::DepositOracleMajorTokens => {
            process_oracle_major_token_movement_instruction(program_id, accounts, payload, true)
        }
        VaultInstructionTag::WithdrawOracleMajorTokens => {
            process_oracle_major_token_movement_instruction(program_id, accounts, payload, false)
        }
        VaultInstructionTag::AccumulateOracleRecipeBucketV2 => {
            require_compressed_state_transport(compressed_state_transport)?;
            process_accumulate_oracle_recipe_bucket_v2_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::FinalizeOracleRecipeWeightsV2 => {
            process_finalize_oracle_recipe_weights_v2_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::AccumulateOracleSettlementSourceBucket => {
            process_accumulate_oracle_settlement_source_bucket_instruction(
                program_id, accounts, payload,
            )
        }
        VaultInstructionTag::BeginOracleActiveWeights => {
            process_begin_oracle_active_weights_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::AccumulateOracleActiveWeightGroup => {
            require_compressed_state_transport(compressed_state_transport)?;
            process_accumulate_oracle_active_weight_group_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::FinalizeOracleActiveWeights => {
            process_finalize_oracle_active_weights_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::FinalizeOracleOpeningPhase => {
            process_finalize_oracle_opening_phase_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::ExpireOracleOpeningSource => {
            require_compressed_state_transport(compressed_state_transport)?;
            process_expire_oracle_opening_source_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::FinalizeOracleMonth => {
            process_finalize_oracle_month_instruction(program_id, accounts, payload)
        }
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

#[inline(never)]
pub(super) fn dispatch_128_158(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    tag: VaultInstructionTag,
    payload: &[u8],
    compressed_state_transport: bool,
) -> ProgramResult {
    let _ = compressed_state_transport;
    match tag {
        VaultInstructionTag::BootstrapVaultGovernanceV2 => {
            let params = BootstrapVaultGovernanceV2Params {
                new_oracle_authority: Pubkey::new_from_array(decode_bytes32_payload(payload)?),
            };
            process_bootstrap_vault_governance_v2(program_id, accounts, params)
        }
        VaultInstructionTag::ActivateVaultV2 => {
            let params = ActivateVaultV2Params {
                expected_collateral_freeze_authority: decode_optional_bytes32_payload(payload)?
                    .map(Pubkey::new_from_array),
            };
            process_activate_vault_v2(program_id, accounts, params)
        }
        VaultInstructionTag::AdminAssistedWithdrawCollateral => {
            process_admin_assisted_withdraw_collateral_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::InitializeOracleUsdcRewardVault => {
            expect_empty_payload(payload)?;
            oracle_usdc::process_initialize_oracle_usdc_reward_vault(program_id, accounts)
        }
        VaultInstructionTag::DepositOracleUsdcRewards => {
            let params = DepositOracleUsdcRewardsParams {
                amount: decode_u64_payload(payload)?,
            };
            oracle_usdc::process_deposit_oracle_usdc_rewards(program_id, accounts, params)
        }
        VaultInstructionTag::BeginOracleUsdcRewardSchedule => {
            expect_empty_payload(payload)?;
            oracle_usdc::process_begin_oracle_usdc_reward_schedule(program_id, accounts)
        }
        VaultInstructionTag::AddOracleUsdcSkuBudget => {
            let params: AddOracleUsdcSkuBudgetParams = decode_instruction_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_add_oracle_usdc_sku_budget(program_id, accounts, params)
        }
        VaultInstructionTag::FinalizeOracleUsdcRewardSchedule => {
            expect_empty_payload(payload)?;
            oracle_usdc::process_finalize_oracle_usdc_reward_schedule(program_id, accounts)
        }
        VaultInstructionTag::InitializeOracleSambaPool => {
            expect_empty_payload(payload)?;
            process_initialize_oracle_samba_pool(program_id, accounts)
        }
        VaultInstructionTag::InitializeOracleRewardFunnel => {
            expect_empty_payload(payload)?;
            process_initialize_oracle_reward_funnel(program_id, accounts)
        }
        VaultInstructionTag::SweepOracleRewardFunnel => {
            expect_empty_payload(payload)?;
            process_sweep_oracle_reward_funnel(program_id, accounts)
        }
        VaultInstructionTag::QueueStakeAmbaForSamba => {
            process_queue_stake_amba_for_samba_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::ActivateQueuedStakeAmbaForSamba => {
            process_activate_queued_stake_amba_for_samba_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::CancelQueuedStakeAmba => {
            expect_empty_payload(payload)?;
            process_cancel_queued_stake_amba(program_id, accounts)
        }
        VaultInstructionTag::RequestUnstakeSamba => {
            process_request_unstake_samba_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::CompleteUnstakeSamba => {
            expect_empty_payload(payload)?;
            process_complete_unstake_samba(program_id, accounts)
        }
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

#[inline(never)]
pub(super) fn dispatch_161_205(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    tag: VaultInstructionTag,
    payload: &[u8],
    compressed_state_transport: bool,
) -> ProgramResult {
    let _ = compressed_state_transport;
    match tag {
        VaultInstructionTag::CreateMarketContractMintV3 => {
            process_create_market_contract_mint_v3_instruction(program_id, accounts, payload)
        }
        VaultInstructionTag::InitializeOracleMonthV5 => {
            let params: InitializeOracleMonthV5Params = decode_instruction_payload(payload)?;
            process_initialize_oracle_month_v5(program_id, accounts, params)
        }
        VaultInstructionTag::ConfigureOracleProductSkuManifest => {
            let params: ConfigureOracleProductSkuManifestParams =
                decode_instruction_payload(payload)?;
            process_configure_oracle_product_sku_manifest(program_id, accounts, params)
        }
        VaultInstructionTag::ExecuteCompressedStateV1 => {
            let params: ExecuteCompressedStateParams = decode_instruction_payload(payload)?;
            compressed_state::process_execute_compressed_state_v1(program_id, accounts, params)
        }
        VaultInstructionTag::ExpireUnlistableOracleSourceV2 => {
            expect_empty_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            process_expire_unlistable_oracle_source_v2(program_id, accounts)
        }
        VaultInstructionTag::CancelStaleOracleSourceChallengeV2 => {
            expect_empty_payload(payload)?;
            process_cancel_stale_oracle_source_challenge_v2(program_id, accounts)
        }
        VaultInstructionTag::ResolveOracleEmergencyDisputeV4 => {
            expect_empty_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_samba_pot::process_resolve_oracle_emergency_dispute_v4(program_id, accounts)
        }
        VaultInstructionTag::SettleFailedOracleMonthEscrowV2 => {
            let params = SettleOracleEscrowParams {
                kind: match decode_u8_payload(payload)? {
                    0 => OracleEscrowKind::ListingBond,
                    1 => OracleEscrowKind::SupportStake,
                    2 => OracleEscrowKind::SourceChallenge,
                    3 => OracleEscrowKind::OpeningClaim,
                    4 => OracleEscrowKind::OpeningChallenge,
                    5 => OracleEscrowKind::UpdateClaim,
                    6 => OracleEscrowKind::UpdateChallenge,
                    _ => return Err(VaultError::InvalidInstructionData.into()),
                },
            };
            if matches!(
                params.kind,
                OracleEscrowKind::ListingBond
                    | OracleEscrowKind::SupportStake
                    | OracleEscrowKind::SourceChallenge
            ) {
                require_compressed_state_transport(compressed_state_transport)?;
            }
            oracle_usdc_rewards::process_settle_failed_oracle_month_escrow_v2(
                program_id, accounts, params,
            )
        }
        VaultInstructionTag::AbortOracleUsdcRewardScheduleV2 => {
            expect_empty_payload(payload)?;
            oracle_usdc_rewards::process_abort_oracle_usdc_reward_schedule_v2(program_id, accounts)
        }
        VaultInstructionTag::TimeoutUnsupportedOracleSourceV2 => {
            expect_empty_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_timeout_unsupported_oracle_source_v2(program_id, accounts)
        }
        VaultInstructionTag::RecomputeOracleBucketMedianV1 => {
            let params: RecomputeOracleBucketMedianV1Params = decode_instruction_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            process_recompute_oracle_bucket_median_v1(program_id, accounts, params)
        }
        VaultInstructionTag::RevealOracleUpdateClaimV3 => {
            let params: RevealOracleUpdateClaimV3Params = decode_instruction_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_reveal_oracle_update_claim_v3(program_id, accounts, params)
        }
        VaultInstructionTag::FinalizeOracleUpdateClaimV2 => {
            let (outcome, current_step) = decode_u8_u64_payload(payload)?;
            let params = FinalizeOracleUpdateClaimV2Params {
                outcome: match outcome {
                    0 => OracleUpdateClaimOutcome::AcceptClaim,
                    1 => OracleUpdateClaimOutcome::RejectClaim,
                    2 => OracleUpdateClaimOutcome::RuleReviewUnresolved,
                    _ => return Err(VaultError::InvalidInstructionData.into()),
                },
                current_step,
            };
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_finalize_oracle_update_claim_v2(program_id, accounts, params)
        }
        VaultInstructionTag::ProposeOracleSourceV3 => {
            let params: ProposeOracleSourceV3Params = decode_instruction_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_propose_oracle_source_v3(program_id, accounts, params)
        }
        VaultInstructionTag::SupportOracleSourceV3 => {
            let params: SupportOracleSourceV3Params = decode_instruction_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_support_oracle_source_v3(program_id, accounts, params)
        }
        VaultInstructionTag::FinalizeOracleSkuCoverage => {
            expect_empty_payload(payload)?;
            process_finalize_oracle_sku_coverage(program_id, accounts)
        }
        VaultInstructionTag::ChallengeOracleSourceV2 => {
            let params: ChallengeOracleSourceParams = decode_instruction_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_challenge_oracle_source_v2(program_id, accounts, params)
        }
        VaultInstructionTag::ResolveOracleSourceChallengeV2 => {
            let params = ResolveOracleSourceChallengeParams {
                outcome: match decode_u8_payload(payload)? {
                    0 => OracleSourceChallengeOutcome::KeepSource,
                    1 => OracleSourceChallengeOutcome::RejectSource,
                    2 => OracleSourceChallengeOutcome::RuleReviewUnresolved,
                    3 => OracleSourceChallengeOutcome::MergeSource,
                    _ => return Err(VaultError::InvalidInstructionData.into()),
                },
            };
            require_compressed_state_transport(compressed_state_transport)?;
            process_resolve_oracle_source_challenge(program_id, accounts, params)
        }
        VaultInstructionTag::CancelStaleOracleUpdateClaimV2 => {
            expect_empty_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_cancel_stale_oracle_update_claim_v2(program_id, accounts)
        }
        VaultInstructionTag::AbortStaleOracleUpdateEmergencyDisputeV2 => {
            expect_empty_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_samba_pot::process_abort_stale_oracle_update_emergency_dispute_v2(
                program_id, accounts,
            )
        }
        VaultInstructionTag::ReopenOracleSkuCoverage => {
            expect_empty_payload(payload)?;
            process_reopen_oracle_sku_coverage(program_id, accounts)
        }
        VaultInstructionTag::BeginOracleRecipeWeightsV3 => {
            let (expected_source_count, expected_bucket_count) = decode_u16_pair_payload(payload)?;
            let params = BeginOracleRecipeWeightsV2Params {
                expected_source_count,
                expected_bucket_count,
            };
            process_begin_oracle_recipe_weights_v2(program_id, accounts, params)
        }
        VaultInstructionTag::SubmitOracleOpeningClaimV2 => {
            let params: SubmitOracleOpeningClaimParams = decode_instruction_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_submit_oracle_opening_claim_v2(program_id, accounts, params)
        }
        VaultInstructionTag::ChallengeOracleOpeningClaimV2 => {
            let params: ChallengeOracleOpeningClaimParams = decode_instruction_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_challenge_oracle_opening_claim_v2(program_id, accounts, params)
        }
        VaultInstructionTag::ResolveOracleOpeningClaimChallengeV2 => {
            let params = ResolveOracleOpeningClaimChallengeParams {
                outcome: match decode_u8_payload(payload)? {
                    0 => OracleOpeningChallengeOutcome::KeepOpening,
                    1 => OracleOpeningChallengeOutcome::AcceptAlternativeOpening,
                    2 => OracleOpeningChallengeOutcome::SourceInactiveForMonth,
                    3 => OracleOpeningChallengeOutcome::RuleReviewUnresolved,
                    4 => OracleOpeningChallengeOutcome::RejectOpeningForRetry,
                    _ => return Err(VaultError::InvalidInstructionData.into()),
                },
            };
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_resolve_oracle_opening_claim_challenge_v2(
                program_id, accounts, params,
            )
        }
        VaultInstructionTag::FinalizeOracleOpeningClaimV2 => {
            expect_empty_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_finalize_oracle_opening_claim_v2(program_id, accounts)
        }
        VaultInstructionTag::CommitOracleUpdateClaimV3 => {
            let params: CommitOracleUpdateClaimV2Params = decode_instruction_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_commit_oracle_update_claim_v3(program_id, accounts, params)
        }
        VaultInstructionTag::SettleExpiredOracleUpdateCommitmentV3 => {
            expect_empty_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_settle_expired_oracle_update_commitment_v3(program_id, accounts)
        }
        VaultInstructionTag::ChallengeOracleUpdateClaimV2 => {
            let params: ChallengeOracleUpdateClaimParams = decode_instruction_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc::process_challenge_oracle_update_claim_v2(program_id, accounts, params)
        }
        VaultInstructionTag::TryOpenOracleEmergencyDisputeV2 => {
            let (kind, target_id, expected_case_hash) = decode_emergency_dispute_payload(payload)?;
            let params = TryOpenOracleEmergencyDisputeParams {
                kind: match kind {
                    0 => OracleEmergencyDisputeKind::Source,
                    1 => OracleEmergencyDisputeKind::Update,
                    2 => OracleEmergencyDisputeKind::Opening,
                    3 => OracleEmergencyDisputeKind::BucketMedian,
                    _ => return Err(VaultError::InvalidInstructionData.into()),
                },
                target_id,
                expected_case_hash,
            };
            if params.kind != OracleEmergencyDisputeKind::BucketMedian {
                require_compressed_state_transport(compressed_state_transport)?;
            }
            oracle_samba_pot::process_try_open_oracle_emergency_dispute_v2(
                program_id, accounts, params,
            )
        }
        VaultInstructionTag::CommitOracleEmergencyVoteV3 => {
            let params: CommitOracleEmergencyVoteV2Params = decode_instruction_payload(payload)?;
            oracle_samba_pot::process_commit_oracle_emergency_vote_v3(program_id, accounts, params)
        }
        VaultInstructionTag::RevealOracleEmergencyVoteV2 => {
            let params: RevealOracleEmergencyVoteParams = decode_instruction_payload(payload)?;
            oracle_samba_pot::process_reveal_oracle_emergency_vote_v2(program_id, accounts, params)
        }
        VaultInstructionTag::ResolveOracleEmergencyDisputeV2 => {
            expect_empty_payload(payload)?;
            if accounts.len() != 8 {
                require_compressed_state_transport(compressed_state_transport)?;
            }
            oracle_samba_pot::process_resolve_oracle_emergency_dispute_v2(program_id, accounts)
        }
        VaultInstructionTag::RegisterOracleSambaWinningVote => {
            expect_empty_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_samba_pot::process_register_oracle_samba_winning_vote(program_id, accounts)
        }
        VaultInstructionTag::SettleOracleSambaEmergencyVoteV2 => {
            expect_empty_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_samba_pot::process_settle_oracle_samba_emergency_vote_v2(program_id, accounts)
        }
        VaultInstructionTag::SettleOracleUsdcEscrow => {
            let params = SettleOracleEscrowParams {
                kind: match decode_u8_payload(payload)? {
                    0 => OracleEscrowKind::ListingBond,
                    1 => OracleEscrowKind::SupportStake,
                    2 => OracleEscrowKind::SourceChallenge,
                    3 => OracleEscrowKind::OpeningClaim,
                    4 => OracleEscrowKind::OpeningChallenge,
                    5 => OracleEscrowKind::UpdateClaim,
                    6 => OracleEscrowKind::UpdateChallenge,
                    _ => return Err(VaultError::InvalidInstructionData.into()),
                },
            };
            if !matches!(params.kind, OracleEscrowKind::UpdateChallenge) {
                require_compressed_state_transport(compressed_state_transport)?;
            }
            oracle_usdc_rewards::process_settle_oracle_usdc_escrow(program_id, accounts, params)
        }
        VaultInstructionTag::RegisterOracleUsdcRewardSource => {
            expect_empty_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc_rewards::process_register_oracle_usdc_reward_source(program_id, accounts)
        }
        VaultInstructionTag::RegisterOracleUsdcRewardUpdate => {
            expect_empty_payload(payload)?;
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc_rewards::process_register_oracle_usdc_reward_update(program_id, accounts)
        }
        VaultInstructionTag::FinalizeOracleUsdcRewardEntitlements => {
            expect_empty_payload(payload)?;
            oracle_usdc_rewards::process_finalize_oracle_usdc_reward_entitlements(
                program_id, accounts,
            )
        }
        VaultInstructionTag::ClaimOracleUsdcReward => {
            let params = ClaimOracleUsdcRewardParams {
                kind: match decode_u8_payload(payload)? {
                    0 => OracleUsdcRewardKind::SourceProposer,
                    1 => OracleUsdcRewardKind::SourceSupport,
                    2 => OracleUsdcRewardKind::Opening,
                    3 => OracleUsdcRewardKind::Update,
                    _ => return Err(VaultError::InvalidInstructionData.into()),
                },
            };
            require_compressed_state_transport(compressed_state_transport)?;
            oracle_usdc_rewards::process_claim_oracle_usdc_reward(program_id, accounts, params)
        }
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

pub(super) fn require_compressed_state_transport(enabled: bool) -> ProgramResult {
    if enabled {
        Ok(())
    } else {
        Err(VaultError::CompressedStateTransportRequired.into())
    }
}
