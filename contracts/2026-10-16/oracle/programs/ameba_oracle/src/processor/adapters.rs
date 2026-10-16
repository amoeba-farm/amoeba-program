use super::*;

#[inline(always)]
pub(super) fn process_initialize_config_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let admin = Pubkey::new_from_array(decode_bytes32_payload(payload)?);
    process_initialize_config(program_id, accounts, admin)
}

#[inline(always)]
pub(super) fn process_create_period_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params: CreatePeriodParams = decode_instruction_payload(payload)?;
    process_create_period(program_id, accounts, params)
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
        args.new_bond_vault,
        args.paused,
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
