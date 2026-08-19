use super::*;

#[inline(always)]
pub(super) fn process_initialize_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    expect_empty_payload(payload)?;
    process_initialize(program_id, accounts)
}

#[inline(always)]
pub(super) fn process_update_config_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let args = decode_update_config_payload(payload)?;
    process_update_config(
        program_id,
        accounts,
        args.new_admin,
        args.new_oracle_authority,
        args.new_usdc_mint,
        args.new_vault_token_account,
        args.paused,
    )
}

#[inline(always)]
pub(super) fn process_deposit_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let amount = decode_u64_payload(payload)?;
    process_deposit(program_id, accounts, amount, true)
}

pub(super) fn process_init_market_v2_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params: InitMarketV2Params = decode_instruction_payload(payload)?;
    process_init_market_v2(program_id, accounts, params)
}

#[inline(always)]
pub(super) fn process_create_market_contract_mint_v3_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    expect_empty_payload(payload)?;
    process_create_market_contract_mint_v3(program_id, accounts)
}

pub(super) fn process_set_market_paused_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params = SetMarketPausedParams {
        paused: decode_bool_payload(payload)?,
    };
    process_set_market_paused(program_id, accounts, params)
}

pub(super) fn process_initialize_settlement_signer_registry_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params: InitializeSettlementSignerRegistryParams = decode_instruction_payload(payload)?;
    process_initialize_settlement_signer_registry(program_id, accounts, params)
}

pub(super) fn process_propose_settlement_signer_rotation_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
    emergency: bool,
) -> ProgramResult {
    let params: ProposeSettlementSignerRotationParams = decode_instruction_payload(payload)?;
    process_propose_settlement_signer_rotation(program_id, accounts, params, emergency)
}

#[inline(always)]
pub(super) fn process_upsert_market_page_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let args = deserialize_upsert_market_page_args(payload)
        .map_err(|_| ProgramError::from(VaultError::InvalidInstructionData))?;
    process_upsert_market_page(
        program_id,
        accounts,
        args.params.page,
        *args.proof,
        args.new_page_output.map(|output| *output),
        args.existing_page.map(|page| *page),
    )
}

#[inline(always)]
pub(super) fn process_upsert_settlement_v3_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let args = deserialize_upsert_settlement_args(payload)
        .map_err(|_| ProgramError::from(VaultError::InvalidInstructionData))?;
    process_upsert_settlement(
        program_id,
        accounts,
        Box::new(args.params.settlement),
        args.proof,
        args.new_settlement_output,
        args.existing_settlement,
    )
}

#[inline(always)]
pub(super) fn process_init_user_collateral_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    expect_empty_payload(payload)?;
    process_init_user_collateral(program_id, accounts)
}

#[inline(always)]
pub(super) fn process_deposit_collateral_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let amount = decode_u64_payload(payload)?;
    process_deposit_collateral(program_id, accounts, amount)
}

#[inline(always)]
pub(super) fn process_withdraw_collateral_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let amount = decode_u64_payload(payload)?;
    process_collateral_withdrawal(program_id, accounts, amount, false)
}

#[inline(always)]
pub(super) fn process_admin_assisted_withdraw_collateral_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let amount = decode_u64_payload(payload)?;
    process_collateral_withdrawal(program_id, accounts, amount, true)
}

#[inline(always)]
pub(super) fn process_configure_oracle_economics_template_v2_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params: ConfigureOracleEconomicsTemplateV2Params = decode_instruction_payload(payload)?;
    process_configure_oracle_economics_template_v2(program_id, accounts, params)
}

#[inline(always)]
pub(super) fn process_configure_oracle_major_token_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    expect_empty_payload(payload)?;
    process_configure_oracle_major_token(program_id, accounts)
}

#[inline(always)]
pub(super) fn process_oracle_major_token_movement_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
    deposit: bool,
) -> ProgramResult {
    let amount = decode_u64_payload(payload)?;
    let movement = if deposit {
        OracleMajorTokenMovement::Deposit(DepositOracleMajorTokensParams { amount })
    } else {
        OracleMajorTokenMovement::Withdraw(WithdrawOracleMajorTokensParams { amount })
    };
    process_oracle_major_token_movement(program_id, accounts, movement)
}

#[inline(always)]
pub(super) fn process_queue_stake_amba_for_samba_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params = QueueStakeAmbaForSambaParams {
        amba_amount: decode_u64_payload(payload)?,
    };
    process_queue_stake_amba_for_samba(program_id, accounts, params)
}

#[inline(always)]
pub(super) fn process_activate_queued_stake_amba_for_samba_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params = ActivateQueuedStakeAmbaForSambaParams {
        min_samba_out: decode_u64_payload(payload)?,
    };
    process_activate_queued_stake_amba_for_samba(program_id, accounts, params)
}

#[inline(always)]
pub(super) fn process_request_unstake_samba_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let (samba_amount, min_amba_out) = decode_u64_pair_payload(payload)?;
    let params = RequestUnstakeSambaParams {
        samba_amount,
        min_amba_out,
    };
    process_request_unstake_samba(program_id, accounts, params)
}

#[inline(always)]
pub(super) fn process_accumulate_oracle_recipe_bucket_v2_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params: AccumulateOracleRecipeBucketV2Params = decode_instruction_payload(payload)?;
    process_accumulate_oracle_recipe_bucket_v2(program_id, accounts, params)
}

#[inline(always)]
pub(super) fn process_finalize_oracle_recipe_weights_v2_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    expect_empty_payload(payload)?;
    process_finalize_oracle_recipe_weights_v2(program_id, accounts)
}

#[inline(always)]
pub(super) fn process_accumulate_oracle_settlement_source_bucket_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params: AccumulateOracleSettlementSourceBucketParams = decode_instruction_payload(payload)?;
    process_accumulate_oracle_settlement_source_bucket(program_id, accounts, params)
}

#[inline(always)]
pub(super) fn process_begin_oracle_active_weights_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let (expected_source_count, expected_group_count) = decode_u16_pair_payload(payload)?;
    let params = BeginOracleActiveWeightsParams {
        expected_source_count,
        expected_group_count,
    };
    process_begin_oracle_active_weights(program_id, accounts, params)
}

#[inline(always)]
pub(super) fn process_accumulate_oracle_active_weight_group_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params: AccumulateOracleActiveWeightGroupParams = decode_instruction_payload(payload)?;
    process_accumulate_oracle_active_weight_group(program_id, accounts, params)
}

#[inline(always)]
pub(super) fn process_finalize_oracle_active_weights_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    expect_empty_payload(payload)?;
    process_finalize_oracle_active_weights(program_id, accounts)
}

#[inline(always)]
pub(super) fn process_finalize_oracle_opening_phase_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    expect_empty_payload(payload)?;
    process_finalize_oracle_opening_phase(program_id, accounts)
}

#[inline(always)]
pub(super) fn process_expire_oracle_opening_source_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    expect_empty_payload(payload)?;
    process_expire_oracle_opening_source(program_id, accounts)
}

#[inline(always)]
pub(super) fn process_close_oracle_month_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    expect_empty_payload(payload)?;
    process_close_oracle_month(program_id, accounts)
}

#[inline(always)]
pub(super) fn process_finalize_oracle_month_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    expect_empty_payload(payload)?;
    process_finalize_oracle_month(program_id, accounts)
}
