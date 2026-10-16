use super::*;
use crate::{
    governance_gate::{self, GateValidated},
    governance_manifest::{classify_active_instruction_tag, InstructionGovernanceClass},
};

#[cfg(all(test, feature = "synthetic-governance-controller"))]
#[derive(Clone, Debug, Eq, PartialEq)]
struct HandlerBoundaryAccount {
    account_info_address: usize,
    key: Pubkey,
    owner: Pubkey,
    is_signer: bool,
    is_writable: bool,
    executable: bool,
    rent_epoch: u64,
}

#[cfg(all(test, feature = "synthetic-governance-controller"))]
#[derive(Clone, Debug, Eq, PartialEq)]
struct HandlerBoundaryCapture {
    tag: u8,
    payload: Vec<u8>,
    accounts: Vec<HandlerBoundaryAccount>,
}

#[cfg(all(test, feature = "synthetic-governance-controller"))]
#[derive(Default)]
struct HandlerBoundaryProbe {
    capture: core::cell::RefCell<Option<HandlerBoundaryCapture>>,
}

#[cfg(all(test, feature = "synthetic-governance-controller"))]
impl HandlerBoundaryProbe {
    fn record(&self, tag: u8, payload: &[u8], accounts: &[AccountInfo]) {
        let capture = HandlerBoundaryCapture {
            tag,
            payload: payload.to_vec(),
            accounts: accounts
                .iter()
                .map(|account| HandlerBoundaryAccount {
                    account_info_address: account as *const AccountInfo as usize,
                    key: *account.key,
                    owner: *account.owner,
                    is_signer: account.is_signer,
                    is_writable: account.is_writable,
                    executable: account.executable,
                    rent_epoch: account.rent_epoch,
                })
                .collect(),
        };
        assert!(
            self.capture.replace(Some(capture)).is_none(),
            "a boundary probe must capture exactly one handler handoff"
        );
    }

    fn take(&self) -> HandlerBoundaryCapture {
        self.capture
            .borrow_mut()
            .take()
            .expect("handler boundary was not reached")
    }
}

/// Internal routing context. In a governance-enabled artifact there is no business-routing call
/// site that can construct this value without first receiving the private gate capability.
pub(super) struct ExecutionContext<'a> {
    gate: &'a GateValidated,
    #[cfg(all(test, feature = "synthetic-governance-controller"))]
    boundary_probe: Option<&'a HandlerBoundaryProbe>,
}

impl<'a> ExecutionContext<'a> {
    fn top_level(gate: &'a GateValidated) -> Self {
        Self {
            gate,
            #[cfg(all(test, feature = "synthetic-governance-controller"))]
            boundary_probe: None,
        }
    }

    #[cfg(all(test, feature = "synthetic-governance-controller"))]
    fn with_boundary_probe(
        gate: &'a GateValidated,
        boundary_probe: &'a HandlerBoundaryProbe,
    ) -> Self {
        Self {
            gate,
            boundary_probe: Some(boundary_probe),
        }
    }

    pub(super) fn gate(&self) -> &GateValidated {
        self.gate
    }

    #[cfg(all(test, feature = "synthetic-governance-controller"))]
    fn capture_handler_boundary(&self, tag: u8, payload: &[u8], accounts: &[AccountInfo]) -> bool {
        let Some(probe) = self.boundary_probe else {
            return false;
        };
        probe.record(tag, payload, accounts);
        true
    }
}

#[cfg(not(feature = "localnet-dev-authority"))]
#[inline(always)]
fn require_transaction_level_stack_height(stack_height: usize) -> ProgramResult {
    if stack_height != solana_program::instruction::TRANSACTION_LEVEL_STACK_HEIGHT {
        return Err(OracleError::InvalidInstructionData.into());
    }
    Ok(())
}

#[cfg(all(test, not(feature = "localnet-dev-authority")))]
std::thread_local! {
    static TEST_STACK_HEIGHT_READS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

#[cfg(all(test, not(feature = "localnet-dev-authority")))]
#[inline(always)]
fn current_invocation_stack_height() -> usize {
    TEST_STACK_HEIGHT_READS.with(|reads| reads.set(reads.get().saturating_add(1)));
    // Direct native unit tests do not execute inside ProgramTest's runtime syscall stubs. Model
    // their direct-entry behavior as transaction-level while separately testing the exact height
    // predicate and exercising the real syscall in SBF ProgramTest coverage.
    solana_program::instruction::TRANSACTION_LEVEL_STACK_HEIGHT
}

#[cfg(all(not(feature = "localnet-dev-authority"), not(test)))]
#[inline(always)]
fn current_invocation_stack_height() -> usize {
    solana_program::instruction::get_stack_height()
}

/// Governed instructions are accepted only at transaction level; a CPI cannot borrow a signed
/// governance envelope.
#[cfg(not(feature = "localnet-dev-authority"))]
#[inline(always)]
fn require_governed_invocation() -> ProgramResult {
    require_transaction_level_stack_height(current_invocation_stack_height())
}

/// Size cap, then classification, then (in gated builds) the governance envelope, then decode.
pub(super) fn process_top_level_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    if instruction_data.len() > MAX_INSTRUCTION_DATA_BYTES {
        return Err(OracleError::InstructionTooLarge.into());
    }
    let tag = *instruction_data
        .first()
        .ok_or(OracleError::InvalidInstructionData)?;
    match classify_active_instruction_tag(tag) {
        InstructionGovernanceClass::Unknown | InstructionGovernanceClass::Reserved => {
            return Err(OracleError::InvalidInstructionData.into());
        }
        // The exhaustive manifest has no ordinary read-only entries. Keeping this arm explicit
        // ensures a future classification cannot silently acquire a mutating route.
        InstructionGovernanceClass::RecognizedReadOnly => {
            return Err(OracleError::InvalidInstructionData.into());
        }
        InstructionGovernanceClass::RecognizedMutating
        | InstructionGovernanceClass::FeatureGatedMutating => {}
    }

    #[cfg(not(feature = "localnet-dev-authority"))]
    {
        require_governed_invocation()?;
        let (legacy_accounts, legacy_data, gate) =
            governance_gate::validate_top_level_envelope(program_id, accounts, instruction_data)?;
        let context = ExecutionContext::top_level(&gate);
        process_instruction_with_context(program_id, legacy_accounts, legacy_data, &context)
    }

    #[cfg(feature = "localnet-dev-authority")]
    {
        let gate = governance_gate::disabled_build_capability();
        let context = ExecutionContext::top_level(&gate);
        process_instruction_with_context(program_id, accounts, instruction_data, &context)
    }
}

pub(super) fn process_instruction_with_context(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
    context: &ExecutionContext<'_>,
) -> ProgramResult {
    let _validated_epoch = context.gate().expected_epoch();
    if instruction_data.len() > MAX_INSTRUCTION_DATA_BYTES {
        return Err(OracleError::InstructionTooLarge.into());
    }
    let (tag_bytes, payload) = instruction_data
        .split_first()
        .ok_or(OracleError::InvalidInstructionData)?;
    let class = classify_active_instruction_tag(*tag_bytes);
    if matches!(
        class,
        InstructionGovernanceClass::Unknown
            | InstructionGovernanceClass::Reserved
            | InstructionGovernanceClass::RecognizedReadOnly
    ) {
        return Err(OracleError::InvalidInstructionData.into());
    }
    #[cfg(any(feature = "localnet-dev-authority", feature = "localnet-governance"))]
    if class == InstructionGovernanceClass::FeatureGatedMutating {
        return dev_bootstrap::process(program_id, accounts, *tag_bytes, payload);
    }
    let tag =
        OracleInstructionTag::from_byte(*tag_bytes).ok_or(OracleError::InvalidInstructionData)?;
    #[cfg(all(test, feature = "synthetic-governance-controller"))]
    if context.capture_handler_boundary(*tag_bytes, payload, accounts) {
        return Ok(());
    }

    match tag as u8 {
        0..=63 => dispatch_0_63(program_id, accounts, tag, payload),
        64..=127 => dispatch_64_127(program_id, accounts, tag, payload),
        128..=160 => dispatch_128_160(program_id, accounts, tag, payload),
        161..=255 => dispatch_161_255(program_id, accounts, tag, payload),
    }
}

#[inline(never)]
fn dispatch_0_63(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    tag: OracleInstructionTag,
    payload: &[u8],
) -> ProgramResult {
    if tag == OracleInstructionTag::OracleCarryForwardV1 {
        return carry::process(program_id, accounts, payload);
    }
    if tag == OracleInstructionTag::CloseOracleMonth {
        expect_empty_payload(payload)?;
        return process_close_oracle_month(program_id, accounts);
    }
    let handler: fn(&Pubkey, &[AccountInfo], &[u8]) -> ProgramResult = match tag {
        OracleInstructionTag::InitializeConfig => process_initialize_config_instruction,
        OracleInstructionTag::CreatePeriod => process_create_period_instruction,
        OracleInstructionTag::UpdateConfig => process_update_config_instruction,
        OracleInstructionTag::InitUserCollateral => process_init_user_collateral_instruction,
        OracleInstructionTag::DepositCollateral => process_deposit_collateral_instruction,
        OracleInstructionTag::WithdrawCollateral => process_withdraw_collateral_instruction,
        _ => return Err(OracleError::InvalidInstructionData.into()),
    };
    handler(program_id, accounts, payload)
}

#[inline(never)]
fn dispatch_64_127(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    tag: OracleInstructionTag,
    payload: &[u8],
) -> ProgramResult {
    match tag {
        OracleInstructionTag::RotateVaultAuthoritiesV2 => {
            let params: RotateVaultAuthoritiesV2Params = decode_instruction_payload(payload)?;
            process_rotate_vault_authorities_v2(program_id, accounts, params)
        }
        OracleInstructionTag::ConfigureOracleEconomicsTemplateV2 => {
            process_configure_oracle_economics_template_v2_instruction(
                program_id, accounts, payload,
            )
        }
        OracleInstructionTag::FinalizeOracleOpeningPhase => {
            process_finalize_oracle_opening_phase_instruction(program_id, accounts, payload)
        }
        OracleInstructionTag::ExpireOracleOpeningSource => {
            process_expire_oracle_opening_source_instruction(program_id, accounts, payload)
        }
        OracleInstructionTag::AccumulateOracleRecipeBucketV2 => {
            process_accumulate_oracle_recipe_bucket_v2_instruction(program_id, accounts, payload)
        }
        OracleInstructionTag::FinalizeOracleRecipeWeightsV2 => {
            process_finalize_oracle_recipe_weights_v2_instruction(program_id, accounts, payload)
        }
        OracleInstructionTag::AccumulateOracleSettlementSourceBucket => {
            process_accumulate_oracle_settlement_source_bucket_instruction(
                program_id, accounts, payload,
            )
        }
        OracleInstructionTag::BeginOracleActiveWeights => {
            process_begin_oracle_active_weights_instruction(program_id, accounts, payload)
        }
        OracleInstructionTag::AccumulateOracleActiveWeightGroup => {
            process_accumulate_oracle_active_weight_group_instruction(program_id, accounts, payload)
        }
        OracleInstructionTag::FinalizeOracleActiveWeights => {
            process_finalize_oracle_active_weights_instruction(program_id, accounts, payload)
        }
        OracleInstructionTag::FinalizeOracleMonth => {
            expect_empty_payload(payload)?;
            process_finalize_oracle_month(program_id, accounts)
        }
        OracleInstructionTag::FailPeriod => {
            expect_empty_payload(payload)?;
            process_fail_period(program_id, accounts)
        }
        OracleInstructionTag::InitializeSettlementSignerRegistry => {
            let params: InitializeSettlementSignerRegistryParams =
                decode_instruction_payload(payload)?;
            process_initialize_settlement_signer_registry(program_id, accounts, params)
        }
        OracleInstructionTag::ProposeSettlementSignerRotation => {
            let params: ProposeSettlementSignerRotationParams =
                decode_instruction_payload(payload)?;
            process_propose_settlement_signer_rotation(program_id, accounts, params, false)
        }
        OracleInstructionTag::ActivateSettlementSignerRotation => {
            expect_empty_payload(payload)?;
            process_activate_settlement_signer_rotation(program_id, accounts)
        }
        OracleInstructionTag::CancelSettlementSignerRotation => {
            expect_empty_payload(payload)?;
            process_cancel_settlement_signer_rotation(program_id, accounts)
        }
        OracleInstructionTag::ProposeEmergencySettlementSignerRecovery => {
            let params: ProposeSettlementSignerRotationParams =
                decode_instruction_payload(payload)?;
            process_propose_settlement_signer_rotation(program_id, accounts, params, true)
        }
        OracleInstructionTag::UpsertSettlementV3 => {
            let params: UpsertSettlementParams = decode_instruction_payload(payload)?;
            settlement::process_upsert_settlement(program_id, accounts, Box::new(params.settlement))
        }
        _ => Err(OracleError::InvalidInstructionData.into()),
    }
}

#[inline(never)]
fn dispatch_128_160(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    tag: OracleInstructionTag,
    payload: &[u8],
) -> ProgramResult {
    match tag {
        OracleInstructionTag::BootstrapVaultGovernanceV2 => {
            let params = BootstrapVaultGovernanceV2Params {
                new_oracle_authority: Pubkey::new_from_array(decode_bytes32_payload(payload)?),
            };
            process_bootstrap_vault_governance_v2(program_id, accounts, params)
        }
        OracleInstructionTag::ActivateVaultV2 => {
            let params = ActivateVaultV2Params {
                expected_collateral_freeze_authority: decode_optional_bytes32_payload(payload)?
                    .map(Pubkey::new_from_array),
            };
            process_activate_vault_v2(program_id, accounts, params)
        }
        OracleInstructionTag::AdminAssistedWithdrawCollateral => {
            process_admin_assisted_withdraw_collateral_instruction(program_id, accounts, payload)
        }
        OracleInstructionTag::CfmLaunchV1 => {
            let params: crate::instruction::CfmLaunchV1Params =
                decode_instruction_payload(payload)?;
            super::cfm_launch::process(program_id, accounts, params)
        }
        OracleInstructionTag::InitializeOracleUsdcRewardVault => {
            expect_empty_payload(payload)?;
            usdc::process_initialize_oracle_usdc_reward_vault(program_id, accounts)
        }
        OracleInstructionTag::DepositOracleUsdcRewards => {
            let params = DepositOracleUsdcRewardsParams {
                amount: decode_u64_payload(payload)?,
            };
            usdc::process_deposit_oracle_usdc_rewards(program_id, accounts, params)
        }
        OracleInstructionTag::BeginOracleUsdcRewardSchedule => {
            expect_empty_payload(payload)?;
            usdc::process_begin_oracle_usdc_reward_schedule(program_id, accounts)
        }
        OracleInstructionTag::AddOracleUsdcSkuBudget => {
            let params: AddOracleUsdcSkuBudgetParams = decode_instruction_payload(payload)?;
            usdc::process_add_oracle_usdc_sku_budget(program_id, accounts, params)
        }
        OracleInstructionTag::FinalizeOracleUsdcRewardSchedule => {
            expect_empty_payload(payload)?;
            usdc::process_finalize_oracle_usdc_reward_schedule(program_id, accounts)
        }
        _ => Err(OracleError::InvalidInstructionData.into()),
    }
}

#[inline(never)]
fn dispatch_161_255(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    tag: OracleInstructionTag,
    payload: &[u8],
) -> ProgramResult {
    match tag {
        OracleInstructionTag::InitializeOracleMonthV5 => {
            let params: InitializeOracleMonthV5Params = decode_instruction_payload(payload)?;
            process_initialize_oracle_month_v5(program_id, accounts, params)
        }
        OracleInstructionTag::ConfigureOracleProductSkuManifest => {
            let params: ConfigureOracleProductSkuManifestParams =
                decode_instruction_payload(payload)?;
            process_configure_oracle_product_sku_manifest(program_id, accounts, params)
        }
        OracleInstructionTag::ChallengeOracleSourceV2 => {
            let params: ChallengeOracleSourceParams = decode_instruction_payload(payload)?;
            usdc::process_challenge_oracle_source_v2(program_id, accounts, params)
        }
        OracleInstructionTag::SettleOracleUsdcEscrow => {
            let params = SettleOracleEscrowParams {
                kind: decode_escrow_kind_payload(payload)?,
            };
            rewards::process_settle_oracle_usdc_escrow(program_id, accounts, params)
        }
        OracleInstructionTag::ProposeOracleSourceV3 => {
            let params: ProposeOracleSourceV3Params = decode_instruction_payload(payload)?;
            usdc::process_propose_oracle_source_v3(program_id, accounts, params)
        }
        OracleInstructionTag::SupportOracleSourceV3 => {
            let params: SupportOracleSourceV3Params = decode_instruction_payload(payload)?;
            usdc::process_support_oracle_source_v3(program_id, accounts, params)
        }
        OracleInstructionTag::FinalizeOracleSkuCoverage => {
            expect_empty_payload(payload)?;
            process_finalize_oracle_sku_coverage(program_id, accounts)
        }
        OracleInstructionTag::ResolveOracleSourceChallengeV2 => {
            let params = ResolveOracleSourceChallengeParams {
                outcome: match decode_u8_payload(payload)? {
                    0 => OracleSourceChallengeOutcome::KeepSource,
                    1 => OracleSourceChallengeOutcome::RejectSource,
                    2 => OracleSourceChallengeOutcome::RuleReviewUnresolved,
                    3 => OracleSourceChallengeOutcome::MergeSource,
                    _ => return Err(OracleError::InvalidInstructionData.into()),
                },
            };
            process_resolve_oracle_source_challenge(program_id, accounts, params)
        }
        OracleInstructionTag::ReopenOracleSkuCoverage => {
            expect_empty_payload(payload)?;
            process_reopen_oracle_sku_coverage(program_id, accounts)
        }
        OracleInstructionTag::ExpireUnlistableOracleSourceV2 => {
            expect_empty_payload(payload)?;
            process_expire_unlistable_oracle_source_v2(program_id, accounts)
        }
        OracleInstructionTag::CancelStaleOracleSourceChallengeV2 => {
            expect_empty_payload(payload)?;
            process_cancel_stale_oracle_source_challenge_v2(program_id, accounts)
        }
        OracleInstructionTag::SettleFailedOracleMonthEscrowV2 => {
            let params = SettleOracleEscrowParams {
                kind: decode_escrow_kind_payload(payload)?,
            };
            rewards::process_settle_failed_oracle_month_escrow_v2(program_id, accounts, params)
        }
        OracleInstructionTag::AbortOracleUsdcRewardScheduleV2 => {
            expect_empty_payload(payload)?;
            rewards::process_abort_oracle_usdc_reward_schedule_v2(program_id, accounts)
        }
        OracleInstructionTag::TimeoutUnsupportedOracleSourceV2 => {
            expect_empty_payload(payload)?;
            usdc::process_timeout_unsupported_oracle_source_v2(program_id, accounts)
        }
        OracleInstructionTag::SubmitOracleOpeningClaimV2 => {
            let params: SubmitOracleOpeningClaimParams = decode_instruction_payload(payload)?;
            usdc::process_submit_oracle_opening_claim_v2(program_id, accounts, params)
        }
        OracleInstructionTag::ChallengeOracleOpeningClaimV2 => {
            let params: ChallengeOracleOpeningClaimParams = decode_instruction_payload(payload)?;
            usdc::process_challenge_oracle_opening_claim_v2(program_id, accounts, params)
        }
        OracleInstructionTag::ResolveOracleOpeningClaimChallengeV2 => {
            let params = ResolveOracleOpeningClaimChallengeParams {
                outcome: match decode_u8_payload(payload)? {
                    0 => OracleOpeningChallengeOutcome::KeepOpening,
                    1 => OracleOpeningChallengeOutcome::AcceptAlternativeOpening,
                    2 => OracleOpeningChallengeOutcome::SourceInactiveForMonth,
                    3 => OracleOpeningChallengeOutcome::RuleReviewUnresolved,
                    4 => OracleOpeningChallengeOutcome::RejectOpeningForRetry,
                    _ => return Err(OracleError::InvalidInstructionData.into()),
                },
            };
            usdc::process_resolve_oracle_opening_claim_challenge_v2(program_id, accounts, params)
        }
        OracleInstructionTag::FinalizeOracleOpeningClaimV2 => {
            expect_empty_payload(payload)?;
            usdc::process_finalize_oracle_opening_claim_v2(program_id, accounts)
        }
        OracleInstructionTag::BeginOracleRecipeWeightsV3 => {
            let (expected_source_count, expected_bucket_count) = decode_u16_pair_payload(payload)?;
            let params = BeginOracleRecipeWeightsV2Params {
                expected_source_count,
                expected_bucket_count,
            };
            process_begin_oracle_recipe_weights_v2(program_id, accounts, params)
        }
        OracleInstructionTag::RecomputeOracleBucketMedianV1 => {
            let params: RecomputeOracleBucketMedianV1Params = decode_instruction_payload(payload)?;
            process_recompute_oracle_bucket_median_v1(program_id, accounts, params)
        }
        OracleInstructionTag::IndexOracleRecipeSourceV1 => {
            let params: IndexOracleRecipeSourceV1Params = decode_instruction_payload(payload)?;
            oracle_membership::process_index_oracle_recipe_source(program_id, accounts, params)
        }
        OracleInstructionTag::CommitOracleUpdateClaimV3 => {
            let params: CommitOracleUpdateClaimV2Params = decode_instruction_payload(payload)?;
            usdc::process_commit_oracle_update_claim_v3(program_id, accounts, params)
        }
        OracleInstructionTag::SettleExpiredOracleUpdateCommitmentV3 => {
            expect_empty_payload(payload)?;
            usdc::process_settle_expired_oracle_update_commitment_v3(program_id, accounts)
        }
        OracleInstructionTag::ChallengeOracleUpdateClaimV2 => {
            let params: ChallengeOracleUpdateClaimParams = decode_instruction_payload(payload)?;
            usdc::process_challenge_oracle_update_claim_v2(program_id, accounts, params)
        }
        OracleInstructionTag::RevealOracleUpdateClaimV3 => {
            let params: RevealOracleUpdateClaimV3Params = decode_instruction_payload(payload)?;
            usdc::process_reveal_oracle_update_claim_v3(program_id, accounts, params)
        }
        OracleInstructionTag::FinalizeOracleUpdateClaimV2 => {
            let (outcome, current_step) = decode_u8_u64_payload(payload)?;
            let params = FinalizeOracleUpdateClaimV2Params {
                outcome: match outcome {
                    0 => OracleUpdateClaimOutcome::AcceptClaim,
                    1 => OracleUpdateClaimOutcome::RejectClaim,
                    2 => OracleUpdateClaimOutcome::RuleReviewUnresolved,
                    _ => return Err(OracleError::InvalidInstructionData.into()),
                },
                current_step,
            };
            usdc::process_finalize_oracle_update_claim_v2(program_id, accounts, params)
        }
        OracleInstructionTag::CancelStaleOracleUpdateClaimV2 => {
            expect_empty_payload(payload)?;
            usdc::process_cancel_stale_oracle_update_claim_v2(program_id, accounts)
        }
        OracleInstructionTag::RegisterOracleUsdcRewardSource => {
            expect_empty_payload(payload)?;
            rewards::process_register_oracle_usdc_reward_source(program_id, accounts)
        }
        OracleInstructionTag::RegisterOracleUsdcRewardUpdate => {
            expect_empty_payload(payload)?;
            rewards::process_register_oracle_usdc_reward_update(program_id, accounts)
        }
        OracleInstructionTag::FinalizeOracleUsdcRewardEntitlements => {
            expect_empty_payload(payload)?;
            rewards::process_finalize_oracle_usdc_reward_entitlements(program_id, accounts)
        }
        OracleInstructionTag::ClaimOracleUsdcReward => {
            let params = ClaimOracleUsdcRewardParams {
                kind: match decode_u8_payload(payload)? {
                    0 => OracleUsdcRewardKind::SourceProposer,
                    1 => OracleUsdcRewardKind::SourceSupport,
                    2 => OracleUsdcRewardKind::Opening,
                    3 => OracleUsdcRewardKind::Update,
                    _ => return Err(OracleError::InvalidInstructionData.into()),
                },
            };
            rewards::process_claim_oracle_usdc_reward(program_id, accounts, params)
        }
        _ => Err(OracleError::InvalidInstructionData.into()),
    }
}

fn decode_escrow_kind_payload(payload: &[u8]) -> Result<OracleEscrowKind, ProgramError> {
    Ok(match decode_u8_payload(payload)? {
        0 => OracleEscrowKind::ListingBond,
        1 => OracleEscrowKind::SupportStake,
        2 => OracleEscrowKind::SourceChallenge,
        3 => OracleEscrowKind::OpeningClaim,
        4 => OracleEscrowKind::OpeningChallenge,
        5 => OracleEscrowKind::UpdateClaim,
        6 => OracleEscrowKind::UpdateChallenge,
        _ => return Err(OracleError::InvalidInstructionData.into()),
    })
}

#[cfg(all(test, feature = "localnet-dev-authority"))]
mod ungated_dispatch_tests {
    use crate::error::OracleError;
    use crate::governance_manifest::RESERVED_INSTRUCTION_TAGS;
    use crate::instruction::OracleInstructionTag;
    use crate::processor::process_instruction_compact as process_instruction;
    use crate::ProgramError;
    use solana_program::account_info::AccountInfo;
    use solana_program::pubkey::Pubkey;

    #[test]
    fn unknown_and_reserved_instruction_bytes_use_the_generic_invalid_instruction_path() {
        let program_id = Pubkey::new_unique();
        for tag in 0u8..=u8::MAX {
            if OracleInstructionTag::from_byte(tag).is_some() {
                continue;
            }
            assert_eq!(
                process_instruction(&program_id, &[], &[tag, 0xff, 0xff]),
                Err(ProgramError::Custom(
                    OracleError::InvalidInstructionData as u32
                )),
                "unassigned tag {tag}"
            );
        }
        for tag in RESERVED_INSTRUCTION_TAGS {
            assert!(OracleInstructionTag::from_byte(tag).is_none());
        }
    }

    #[test]
    fn unknown_tags_reject_before_borrowing_any_account_data() {
        let key = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mut lamports = 1;
        let mut data = [0u8; 1];
        let account = AccountInfo::new(
            &key,
            false,
            false,
            &mut lamports,
            &mut data,
            &owner,
            false,
            0,
        );
        let poison_borrow = account.try_borrow_mut_data().unwrap();
        for tag in 0u8..=u8::MAX {
            if OracleInstructionTag::from_byte(tag).is_none() {
                let result = process_instruction(&crate::id(), &[account.clone()], &[tag]);
                if crate::governance_manifest::classify_active_instruction_tag(tag)
                    == crate::governance_manifest::InstructionGovernanceClass::FeatureGatedMutating
                {
                    // A dev byte reaches its handler, which rejects this account list unread.
                    assert!(
                        matches!(result, Err(error) if error != ProgramError::AccountBorrowFailed),
                        "byte {tag} borrowed account data before rejection"
                    );
                    continue;
                }
                assert_eq!(
                    result,
                    Err(ProgramError::Custom(
                        OracleError::InvalidInstructionData as u32
                    )),
                    "byte {tag} borrowed account data before rejection"
                );
            }
        }
        drop(poison_borrow);
    }

    #[test]
    fn empty_and_oversized_instruction_data_reject_before_classification() {
        assert_eq!(
            process_instruction(&crate::id(), &[], &[]),
            Err(ProgramError::Custom(
                OracleError::InvalidInstructionData as u32
            ))
        );
        // A recognized tag over the cap is rejected by size, before its payload is examined.
        let mut data = vec![0u8; crate::constants::MAX_INSTRUCTION_DATA_BYTES + 1];
        data[0] = OracleInstructionTag::CreatePeriod as u8;
        assert_eq!(
            process_instruction(&crate::id(), &[], &data),
            Err(ProgramError::Custom(
                OracleError::InstructionTooLarge as u32
            ))
        );
        data.truncate(crate::constants::MAX_INSTRUCTION_DATA_BYTES);
        data[0] = u8::MAX;
        assert_eq!(
            process_instruction(&crate::id(), &[], &data),
            Err(ProgramError::Custom(
                OracleError::InvalidInstructionData as u32
            ))
        );
    }

    #[test]
    fn zero_byte_and_fixed_payloads_reject_trailing_data_before_account_access() {
        let program_id = Pubkey::new_unique();
        let invalid = Err(ProgramError::Custom(
            OracleError::InvalidInstructionData as u32,
        ));
        for tag in [
            OracleInstructionTag::InitUserCollateral,
            OracleInstructionTag::InitializeOracleUsdcRewardVault,
            OracleInstructionTag::BeginOracleUsdcRewardSchedule,
            OracleInstructionTag::FinalizeOracleUsdcRewardSchedule,
            OracleInstructionTag::FinalizeOracleSkuCoverage,
            OracleInstructionTag::ReopenOracleSkuCoverage,
            OracleInstructionTag::ExpireUnlistableOracleSourceV2,
            OracleInstructionTag::CancelStaleOracleSourceChallengeV2,
            OracleInstructionTag::AbortOracleUsdcRewardScheduleV2,
            OracleInstructionTag::TimeoutUnsupportedOracleSourceV2,
            OracleInstructionTag::SettleExpiredOracleUpdateCommitmentV3,
            OracleInstructionTag::CancelStaleOracleUpdateClaimV2,
            OracleInstructionTag::CloseOracleMonth,
            OracleInstructionTag::FinalizeOracleMonth,
            OracleInstructionTag::FailPeriod,
            OracleInstructionTag::ActivateSettlementSignerRotation,
            OracleInstructionTag::CancelSettlementSignerRotation,
            OracleInstructionTag::RegisterOracleUsdcRewardSource,
            OracleInstructionTag::RegisterOracleUsdcRewardUpdate,
            OracleInstructionTag::FinalizeOracleUsdcRewardEntitlements,
        ] {
            assert_eq!(
                process_instruction(&program_id, &[], &[tag as u8, 0]),
                invalid,
                "tag {} accepted a nonempty zero-byte payload",
                tag as u8
            );
        }
        // Enumerated one-byte payloads reject unknown values and any other length.
        for (tag, values) in [
            (OracleInstructionTag::SettleOracleUsdcEscrow, 7u8),
            (OracleInstructionTag::SettleFailedOracleMonthEscrowV2, 7),
            (OracleInstructionTag::ResolveOracleSourceChallengeV2, 4),
            (OracleInstructionTag::ClaimOracleUsdcReward, 4),
        ] {
            for payload in [&[][..], &[values][..], &[0, 0][..], &[u8::MAX][..]] {
                let data = [&[tag as u8][..], payload].concat();
                assert_eq!(
                    process_instruction(&program_id, &[], &data),
                    invalid,
                    "tag {} payload {payload:?}",
                    tag as u8
                );
            }
        }
        for (tag, len) in [
            (OracleInstructionTag::InitializeConfig, 32usize),
            (OracleInstructionTag::CreatePeriod, 49),
            (OracleInstructionTag::DepositCollateral, 8),
            (OracleInstructionTag::WithdrawCollateral, 8),
            (OracleInstructionTag::AdminAssistedWithdrawCollateral, 8),
            (OracleInstructionTag::RotateVaultAuthoritiesV2, 64),
            (OracleInstructionTag::ConfigureOracleEconomicsTemplateV2, 26),
            (OracleInstructionTag::BootstrapVaultGovernanceV2, 32),
            (OracleInstructionTag::InitializeOracleMonthV5, 58),
            (OracleInstructionTag::DepositOracleUsdcRewards, 8),
            (OracleInstructionTag::AddOracleUsdcSkuBudget, 108),
            (OracleInstructionTag::ChallengeOracleSourceV2, 105),
            (OracleInstructionTag::CommitOracleUpdateClaimV3, 72),
            (OracleInstructionTag::FinalizeOracleUpdateClaimV2, 9),
            (
                OracleInstructionTag::InitializeSettlementSignerRegistry,
                306,
            ),
            (OracleInstructionTag::ProposeSettlementSignerRotation, 282),
            (
                OracleInstructionTag::ProposeEmergencySettlementSignerRecovery,
                282,
            ),
        ] {
            for payload_len in [len - 1, len + 1] {
                let mut data = vec![0u8; payload_len + 1];
                data[0] = tag as u8;
                assert_eq!(
                    process_instruction(&program_id, &[], &data),
                    invalid,
                    "tag {} payload {payload_len}",
                    tag as u8
                );
            }
        }
        for payload in [&[][..], &[0, 0][..], &[2][..], &[1; 32][..]] {
            let data = [&[OracleInstructionTag::ActivateVaultV2 as u8][..], payload].concat();
            assert_eq!(process_instruction(&program_id, &[], &data), invalid);
        }
    }
}

#[cfg(all(test, feature = "synthetic-governance-controller"))]
mod governance_precedence_tests {
    use super::{
        process_instruction_with_context, process_top_level_instruction, ExecutionContext,
        HandlerBoundaryAccount, HandlerBoundaryCapture, HandlerBoundaryProbe,
        TEST_STACK_HEIGHT_READS,
    };
    use crate::ProgramError;
    use crate::{
        error::OracleError,
        governance_gate::{
            derive_controller_config_pda, derive_protocol_gate_pda, derive_target_programdata_pda,
            validate_top_level_envelope, GovernanceInstructionTailV1, GOVERNANCE_TAIL_LEN,
            PINNED_CONTROLLER_PROGRAM_ID, PROTOCOL_GATE_DISCRIMINATOR, PROTOCOL_GATE_LEN,
            PROTOCOL_GATE_VERSION_V1,
        },
        governance_manifest::{classify_active_instruction_tag, InstructionGovernanceClass},
        instruction::OracleInstructionTag,
    };
    use solana_program::{account_info::AccountInfo, pubkey::Pubkey};

    const TEST_EPOCH: u64 = 41;

    fn active_gate_bytes(epoch: u64) -> [u8; PROTOCOL_GATE_LEN] {
        let target = crate::id();
        let (_, bump) = derive_protocol_gate_pda(&PINNED_CONTROLLER_PROGRAM_ID, &target);
        let controller_config =
            derive_controller_config_pda(&PINNED_CONTROLLER_PROGRAM_ID, &target).0;
        let mut bytes = [0u8; PROTOCOL_GATE_LEN];
        bytes[0..8].copy_from_slice(&PROTOCOL_GATE_DISCRIMINATOR);
        bytes[8] = PROTOCOL_GATE_VERSION_V1;
        bytes[9] = bump;
        bytes[10] = 1;
        bytes[11] = 0;
        bytes[12..44].copy_from_slice(controller_config.as_ref());
        bytes[44..76].copy_from_slice(target.as_ref());
        bytes[76..108].copy_from_slice(derive_target_programdata_pda(&target).as_ref());
        bytes[108..116].copy_from_slice(&epoch.to_le_bytes());
        bytes
    }

    fn account_boundary(accounts: &[AccountInfo]) -> Vec<HandlerBoundaryAccount> {
        accounts
            .iter()
            .map(|account| HandlerBoundaryAccount {
                account_info_address: account as *const AccountInfo as usize,
                key: *account.key,
                owner: *account.owner,
                is_signer: account.is_signer,
                is_writable: account.is_writable,
                executable: account.executable,
                rent_epoch: account.rent_epoch,
            })
            .collect()
    }

    fn capture_top_level_handler_boundary(
        accounts: &[AccountInfo],
        legacy_data: &[u8],
    ) -> HandlerBoundaryCapture {
        let mut governed_data = legacy_data.to_vec();
        governed_data
            .extend_from_slice(&GovernanceInstructionTailV1::for_epoch(TEST_EPOCH).encode());
        let (legacy_accounts, stripped_data, gate) =
            validate_top_level_envelope(&crate::id(), accounts, &governed_data)
                .expect("representative governance envelope validates");
        let probe = HandlerBoundaryProbe::default();
        let context = ExecutionContext::with_boundary_probe(&gate, &probe);
        assert_eq!(
            process_instruction_with_context(
                &crate::id(),
                legacy_accounts,
                stripped_data,
                &context,
            ),
            Ok(())
        );
        probe.take()
    }

    #[test]
    fn every_assigned_top_level_tag_requires_the_envelope() {
        for tag in 0u8..=u8::MAX {
            TEST_STACK_HEIGHT_READS.with(|reads| reads.set(0));
            let result = process_top_level_instruction(&crate::id(), &[], &[tag]);
            let class = classify_active_instruction_tag(tag);
            match class {
                InstructionGovernanceClass::RecognizedMutating
                | InstructionGovernanceClass::FeatureGatedMutating => assert_eq!(
                    result,
                    Err(ProgramError::Custom(
                        OracleError::MissingGovernanceTail as u32
                    )),
                    "assigned byte {tag}"
                ),
                InstructionGovernanceClass::Unknown
                | InstructionGovernanceClass::Reserved
                | InstructionGovernanceClass::RecognizedReadOnly => assert_eq!(
                    result,
                    Err(ProgramError::Custom(
                        OracleError::InvalidInstructionData as u32
                    )),
                    "unassigned byte {tag}"
                ),
            }
            let stack_height_reads = TEST_STACK_HEIGHT_READS.with(core::cell::Cell::get);
            assert_eq!(
                stack_height_reads,
                usize::from(matches!(
                    class,
                    InstructionGovernanceClass::RecognizedMutating
                        | InstructionGovernanceClass::FeatureGatedMutating
                )),
                "byte {tag} stack-height precedence"
            );
        }
    }

    #[test]
    fn governance_gate_accepts_only_transaction_level_stack_height() {
        let transaction_level = solana_program::instruction::TRANSACTION_LEVEL_STACK_HEIGHT;
        assert_eq!(
            super::require_transaction_level_stack_height(transaction_level),
            Ok(())
        );
        for rejected_height in [0, transaction_level + 1, usize::MAX] {
            assert_eq!(
                super::require_transaction_level_stack_height(rejected_height),
                Err(ProgramError::Custom(
                    OracleError::InvalidInstructionData as u32
                )),
                "stack height {rejected_height}"
            );
        }
    }

    #[test]
    fn unknown_and_reserved_tags_reject_before_borrowing_any_account_data() {
        let key = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mut lamports = 1;
        let mut data = [0u8; 1];
        let account = AccountInfo::new(
            &key,
            false,
            false,
            &mut lamports,
            &mut data,
            &owner,
            false,
            0,
        );
        let poison_borrow = account.try_borrow_mut_data().unwrap();
        for tag in 0u8..=u8::MAX {
            if matches!(
                classify_active_instruction_tag(tag),
                InstructionGovernanceClass::Unknown | InstructionGovernanceClass::Reserved
            ) {
                assert_eq!(
                    process_top_level_instruction(&crate::id(), &[account.clone()], &[tag]),
                    Err(ProgramError::Custom(
                        OracleError::InvalidInstructionData as u32
                    )),
                    "byte {tag} borrowed account data before rejection"
                );
            }
        }
        drop(poison_borrow);
    }

    #[test]
    fn valid_tail_without_a_gate_reports_the_missing_gate() {
        let mut data = vec![OracleInstructionTag::UpdateConfig as u8];
        data.extend_from_slice(&GovernanceInstructionTailV1::for_epoch(41).encode());
        assert_eq!(data.len(), GOVERNANCE_TAIL_LEN + 1);
        assert_eq!(
            process_top_level_instruction(&crate::id(), &[], &data),
            Err(ProgramError::Custom(
                OracleError::MissingGovernanceGate as u32
            ))
        );
    }

    #[test]
    fn governed_top_level_routes_preserve_exact_handler_boundary_inputs() {
        let legacy_key_a = Pubkey::new_unique();
        let legacy_owner_a = Pubkey::new_unique();
        let mut legacy_lamports_a = 11;
        let mut legacy_data_a = [0xabu8; 3];
        let legacy_key_b = Pubkey::new_unique();
        let legacy_owner_b = Pubkey::new_unique();
        let mut legacy_lamports_b = 13;
        let mut legacy_data_b = [0xcdu8; 5];
        let gate_key = derive_protocol_gate_pda(&PINNED_CONTROLLER_PROGRAM_ID, &crate::id()).0;
        let gate_owner = PINNED_CONTROLLER_PROGRAM_ID;
        let mut gate_lamports = 17;
        let mut gate_data = active_gate_bytes(TEST_EPOCH);
        let accounts = vec![
            AccountInfo::new(
                &legacy_key_a,
                true,
                true,
                &mut legacy_lamports_a,
                &mut legacy_data_a,
                &legacy_owner_a,
                false,
                7,
            ),
            AccountInfo::new(
                &legacy_key_b,
                false,
                false,
                &mut legacy_lamports_b,
                &mut legacy_data_b,
                &legacy_owner_b,
                true,
                9,
            ),
            AccountInfo::new(
                &gate_key,
                false,
                false,
                &mut gate_lamports,
                &mut gate_data,
                &gate_owner,
                false,
                0,
            ),
        ];

        for (index, tag) in OracleInstructionTag::ALL.into_iter().enumerate() {
            let legacy_data = vec![tag as u8, 0x10 + index as u8, 0x20, 0x30];
            let capture = capture_top_level_handler_boundary(&accounts, &legacy_data);
            assert_eq!(capture.tag, legacy_data[0]);
            assert_eq!(capture.payload, legacy_data[1..]);
            assert_eq!(
                capture.accounts,
                account_boundary(&accounts[..accounts.len() - 1])
            );
        }
    }
}
