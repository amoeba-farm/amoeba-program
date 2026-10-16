//! Off-chain instruction builders with the canonical account lists.

use super::*;
use crate::state::{
    derive_oracle_active_weight_manifest_pda, derive_oracle_bucket_median_pda,
    derive_oracle_bucket_source_index_pda, derive_oracle_maturity_ladder_registry_pda,
    derive_oracle_product_sku_draft_pda, derive_oracle_product_sku_manifest_pda,
    derive_oracle_recipe_source_index_pda, derive_oracle_recipe_weight_manifest_pda,
    derive_oracle_settlement_source_manifest_pda, derive_oracle_sku_coverage_manifest_pda,
    derive_oracle_sku_coverage_record_pda, derive_oracle_source_challenge_guard_pda,
    derive_oracle_usdc_reward_schedule_pda, derive_oracle_usdc_reward_vault_pda,
    derive_oracle_usdc_sku_pool_pda, derive_oracle_usdc_source_reward_pda,
    derive_settlement_signer_registry_pda, derive_user_collateral_pda, find_bond_vault_address,
    find_config_address,
};
use ameba_oracle_interface::find_period_address;
use solana_program::instruction::{AccountMeta, Instruction};
use solana_sdk_ids::{bpf_loader_upgradeable, system_program};

fn instruction(
    program_id: &Pubkey,
    data: OracleInstruction,
    accounts: Vec<AccountMeta>,
) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts,
        data: data
            .try_to_vec()
            .expect("instruction serialization is infallible"),
    }
}

pub fn find_program_data_address(program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[program_id.as_ref()], &bpf_loader_upgradeable::ID).0
}

pub fn find_oracle_economics_config_address(program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_ECONOMICS_CONFIG_PDA_SEED,
        ],
        program_id,
    )
    .0
}

pub fn find_oracle_month_address(program_id: &Pubkey, period: &Pubkey, expiry_ts: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::ORACLE_MONTH_PDA_SEED,
            period.as_ref(),
            &expiry_ts.to_le_bytes(),
        ],
        program_id,
    )
    .0
}

pub fn initialize_config(
    program_id: &Pubkey,
    payer: &Pubkey,
    upgrade_authority: &Pubkey,
    admin: Pubkey,
    usdc_mint: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::InitializeConfig { admin },
        vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(find_program_data_address(program_id), false),
            AccountMeta::new_readonly(*upgrade_authority, true),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(*usdc_mint, false),
            AccountMeta::new(find_bond_vault_address(program_id, usdc_mint), false),
            AccountMeta::new_readonly(crate::token_instruction::id(), false),
            AccountMeta::new_readonly(crate::associated_token::id(), false),
        ],
    )
}

/// Create the oracle config after controller registration has taken loader authority.
/// Three distinct current council seats authorize the initial admin. The gate envelope
/// is appended separately by the sender, as for every governed instruction.
pub fn initialize_config_with_council(
    program_id: &Pubkey,
    payer: &Pubkey,
    controller: &Pubkey,
    seats: &[Pubkey; 3],
    admin: Pubkey,
    usdc_mint: &Pubkey,
) -> Instruction {
    let authority = Pubkey::find_program_address(
        &[
            b"ameba-governance-v3",
            b"target-authority",
            program_id.as_ref(),
        ],
        controller,
    )
    .0;
    let mut ix = initialize_config(program_id, payer, &authority, admin, usdc_mint);
    ix.accounts[3].is_signer = false;
    ix.accounts.push(AccountMeta::new_readonly(
        crate::governance_gate::derive_controller_config_pda(controller, program_id).0,
        false,
    ));
    ix.accounts.extend(
        seats
            .iter()
            .map(|seat| AccountMeta::new_readonly(*seat, true)),
    );
    ix
}

pub fn create_period(
    program_id: &Pubkey,
    payer: &Pubkey,
    authority: &Pubkey,
    product_id: [u8; 32],
    planned_listing_ts: u64,
    expiry_ts: u64,
    value_decimals: u8,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::CreatePeriod {
            params: CreatePeriodParams {
                product_id,
                planned_listing_ts,
                expiry_ts,
                value_decimals,
            },
        },
        vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new(
                find_period_address(program_id, &product_id, expiry_ts).0,
                false,
            ),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn update_config(
    program_id: &Pubkey,
    admin: &Pubkey,
    usdc_mint: &Pubkey,
    bond_vault: &Pubkey,
    new_admin: Option<Pubkey>,
    new_oracle_authority: Option<Pubkey>,
    new_usdc_mint: Option<Pubkey>,
    new_bond_vault: Option<Pubkey>,
    paused: Option<bool>,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::UpdateConfig {
            new_admin,
            new_oracle_authority,
            new_usdc_mint,
            new_bond_vault,
            paused,
        },
        vec![
            AccountMeta::new_readonly(*admin, true),
            AccountMeta::new(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(*usdc_mint, false),
            AccountMeta::new_readonly(*bond_vault, false),
            AccountMeta::new_readonly(crate::token_instruction::id(), false),
        ],
    )
}

pub fn rotate_vault_authorities_v2(
    program_id: &Pubkey,
    current_admin: &Pubkey,
    current_oracle_authority: &Pubkey,
    params: RotateVaultAuthoritiesV2Params,
    current_signer_set: &Pubkey,
) -> Instruction {
    let accounts = vec![
        AccountMeta::new_readonly(*current_admin, true),
        AccountMeta::new_readonly(*current_oracle_authority, true),
        AccountMeta::new_readonly(params.new_admin, true),
        AccountMeta::new_readonly(params.new_oracle_authority, true),
        AccountMeta::new(find_config_address(program_id).0, false),
        AccountMeta::new_readonly(derive_settlement_signer_registry_pda(program_id).0, false),
        AccountMeta::new_readonly(*current_signer_set, false),
    ];
    instruction(
        program_id,
        OracleInstruction::RotateVaultAuthoritiesV2 { params },
        accounts,
    )
}

pub fn bootstrap_vault_governance_v2(
    program_id: &Pubkey,
    bootstrap_admin: &Pubkey,
    new_oracle_authority: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::BootstrapVaultGovernanceV2 {
            params: BootstrapVaultGovernanceV2Params {
                new_oracle_authority: *new_oracle_authority,
            },
        },
        vec![
            AccountMeta::new_readonly(*bootstrap_admin, true),
            AccountMeta::new_readonly(*new_oracle_authority, true),
            AccountMeta::new(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(derive_settlement_signer_registry_pda(program_id).0, false),
        ],
    )
}

pub fn activate_vault_v2(
    program_id: &Pubkey,
    admin: &Pubkey,
    current_signer_set: &Pubkey,
    usdc_mint: &Pubkey,
    expected_collateral_freeze_authority: Option<Pubkey>,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::ActivateVaultV2 {
            params: ActivateVaultV2Params {
                expected_collateral_freeze_authority,
            },
        },
        vec![
            AccountMeta::new_readonly(*admin, true),
            AccountMeta::new(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(derive_settlement_signer_registry_pda(program_id).0, false),
            AccountMeta::new_readonly(*current_signer_set, false),
            AccountMeta::new_readonly(*usdc_mint, false),
            AccountMeta::new_readonly(find_bond_vault_address(program_id, usdc_mint), false),
            AccountMeta::new_readonly(crate::token_instruction::id(), false),
        ],
    )
}

pub fn init_user_collateral(program_id: &Pubkey, owner: &Pubkey) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::InitUserCollateral,
        vec![
            AccountMeta::new(*owner, true),
            AccountMeta::new(derive_user_collateral_pda(program_id, owner).0, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

pub fn deposit_collateral(
    program_id: &Pubkey,
    owner: &Pubkey,
    owner_token: &Pubkey,
    usdc_mint: &Pubkey,
    amount: u64,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::DepositCollateral { amount },
        vec![
            AccountMeta::new_readonly(*owner, true),
            AccountMeta::new(*owner_token, false),
            AccountMeta::new(find_bond_vault_address(program_id, usdc_mint), false),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new(derive_user_collateral_pda(program_id, owner).0, false),
            AccountMeta::new_readonly(*usdc_mint, false),
            AccountMeta::new_readonly(crate::token_instruction::id(), false),
        ],
    )
}

pub fn withdraw_collateral(
    program_id: &Pubkey,
    owner: &Pubkey,
    destination_token: &Pubkey,
    usdc_mint: &Pubkey,
    amount: u64,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::WithdrawCollateral { amount },
        vec![
            AccountMeta::new_readonly(*owner, true),
            AccountMeta::new(find_bond_vault_address(program_id, usdc_mint), false),
            AccountMeta::new(*destination_token, false),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new(derive_user_collateral_pda(program_id, owner).0, false),
            AccountMeta::new_readonly(*usdc_mint, false),
            AccountMeta::new_readonly(crate::token_instruction::id(), false),
        ],
    )
}

pub fn admin_assisted_withdraw_collateral(
    program_id: &Pubkey,
    admin: &Pubkey,
    owner: &Pubkey,
    destination_token: &Pubkey,
    usdc_mint: &Pubkey,
    amount: u64,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::AdminAssistedWithdrawCollateral { amount },
        vec![
            AccountMeta::new_readonly(*admin, true),
            AccountMeta::new_readonly(*owner, true),
            AccountMeta::new(find_bond_vault_address(program_id, usdc_mint), false),
            AccountMeta::new(*destination_token, false),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new(derive_user_collateral_pda(program_id, owner).0, false),
            AccountMeta::new_readonly(*usdc_mint, false),
            AccountMeta::new_readonly(crate::token_instruction::id(), false),
        ],
    )
}

pub fn configure_oracle_economics_template_v2(
    program_id: &Pubkey,
    admin: &Pubkey,
    params: ConfigureOracleEconomicsTemplateV2Params,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::ConfigureOracleEconomicsTemplateV2 { params },
        vec![
            AccountMeta::new(*admin, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new(find_oracle_economics_config_address(program_id), false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

pub fn configure_oracle_product_sku_manifest(
    program_id: &Pubkey,
    admin: &Pubkey,
    oracle_authority: &Pubkey,
    params: ConfigureOracleProductSkuManifestParams,
) -> Instruction {
    let draft =
        derive_oracle_product_sku_draft_pda(program_id, &params.underlying_id, params.draft_nonce)
            .0;
    let manifest = derive_oracle_product_sku_manifest_pda(program_id, &params.underlying_id).0;
    instruction(
        program_id,
        OracleInstruction::ConfigureOracleProductSkuManifest { params },
        vec![
            AccountMeta::new(*admin, true),
            AccountMeta::new_readonly(*oracle_authority, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new(draft, false),
            AccountMeta::new(manifest, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

pub fn initialize_oracle_month_v5(
    program_id: &Pubkey,
    oracle_authority: &Pubkey,
    payer: &Pubkey,
    product_id: &[u8; 32],
    expiry_ts: u64,
    params: InitializeOracleMonthV5Params,
) -> Instruction {
    let period = find_period_address(program_id, product_id, expiry_ts).0;
    let month = find_oracle_month_address(program_id, &period, expiry_ts);
    instruction(
        program_id,
        OracleInstruction::InitializeOracleMonthV5 { params },
        vec![
            AccountMeta::new_readonly(*oracle_authority, true),
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(period, false),
            AccountMeta::new(month, false),
            AccountMeta::new(
                derive_oracle_sku_coverage_manifest_pda(program_id, &month).0,
                false,
            ),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(find_oracle_economics_config_address(program_id), false),
            AccountMeta::new_readonly(
                derive_oracle_product_sku_manifest_pda(program_id, product_id).0,
                false,
            ),
            AccountMeta::new(
                derive_oracle_maturity_ladder_registry_pda(program_id, product_id).0,
                false,
            ),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

/// Evidence object `[ns, "g3-evidence-bytes-v1", payer, kind, hash]`: kind 1 is a public
/// locator, 2 a source definition, 3 a Wayback archive URL.
pub fn find_evidence_object_address(
    program_id: &Pubkey,
    payer: &Pubkey,
    kind: u8,
    hash: &[u8; 32],
) -> Pubkey {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            b"g3-evidence-bytes-v1",
            payer.as_ref(),
            &[kind],
            hash,
        ],
        program_id,
    )
    .0
}

/// Publication link `[ns, "g3-evidence-link-v1", event, role, hash]`.
pub fn find_evidence_link_address(
    program_id: &Pubkey,
    event: &Pubkey,
    role: u8,
    hash: &[u8; 32],
) -> Pubkey {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            b"g3-evidence-link-v1",
            event.as_ref(),
            &[role],
            hash,
        ],
        program_id,
    )
    .0
}

fn oracle_carry_forward(
    program_id: &Pubkey,
    action: OracleCarryForwardActionV1,
    accounts: Vec<AccountMeta>,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::OracleCarryForwardV1 { action },
        accounts,
    )
}

/// Append one chunk of at most 192 bytes to the payer's evidence object; the first chunk creates
/// it and the exact final chunk seals it against its committed hash.
pub fn write_evidence(
    program_id: &Pubkey,
    payer: &Pubkey,
    kind: u8,
    hash: [u8; 32],
    total: u16,
    offset: u16,
    bytes: Vec<u8>,
) -> Instruction {
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::WriteEvidence {
            kind,
            hash,
            total,
            offset,
            bytes,
        },
        vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(
                find_evidence_object_address(program_id, payer, kind, &hash),
                false,
            ),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

/// Close an unsealed evidence draft and refund its rent to the payer that wrote it.
pub fn close_evidence_draft(program_id: &Pubkey, payer: &Pubkey, object: &Pubkey) -> Instruction {
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::CloseEvidenceDraft,
        vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(*object, false),
        ],
    )
}

/// Publish the locator and definition links of an existing source from sealed objects written by
/// `object_payer`.
#[allow(clippy::too_many_arguments)]
pub fn backfill_source_evidence(
    program_id: &Pubkey,
    payer: &Pubkey,
    object_payer: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    canonical_locator_hash: &[u8; 32],
    source_definition_hash: &[u8; 32],
) -> Instruction {
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::BackfillSourceEvidence,
        vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new(*source, false),
            AccountMeta::new_readonly(
                find_evidence_object_address(program_id, object_payer, 1, canonical_locator_hash),
                false,
            ),
            AccountMeta::new(
                find_evidence_link_address(program_id, source, 0, canonical_locator_hash),
                false,
            ),
            AccountMeta::new_readonly(
                find_evidence_object_address(program_id, object_payer, 2, source_definition_hash),
                false,
            ),
            AccountMeta::new(
                find_evidence_link_address(program_id, source, 1, source_definition_hash),
                false,
            ),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(*period, false),
        ],
    )
}

/// Publish the archive-URL link of an opening claim (role 2), opening challenge (3), update
/// claim (4) or update challenge (5). `subject` is the claim or challenge and `parent` the claim
/// it belongs to (the claim itself for roles 2 and 4).
#[allow(clippy::too_many_arguments)]
pub fn backfill_claim_evidence(
    program_id: &Pubkey,
    payer: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    subject: &Pubkey,
    parent: &Pubkey,
    archive_object: &Pubkey,
    evidence_hash: &[u8; 32],
    role: u8,
) -> Instruction {
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::BackfillClaimEvidence { role },
        vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new_readonly(*subject, false),
            AccountMeta::new_readonly(*parent, false),
            AccountMeta::new_readonly(*archive_object, false),
            AccountMeta::new(
                find_evidence_link_address(program_id, subject, role, evidence_hash),
                false,
            ),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(*period, false),
        ],
    )
}

fn namespaced(program_id: &Pubkey, seeds: &[&[u8]]) -> Pubkey {
    let mut all: Vec<&[u8]> = vec![crate::constants::CURRENT_STATE_NAMESPACE_SEED];
    all.extend_from_slice(seeds);
    Pubkey::find_program_address(&all, program_id).0
}

pub fn find_oracle_source_address(
    program_id: &Pubkey,
    month: &Pubkey,
    source_id: &[u8; 32],
) -> Pubkey {
    namespaced(
        program_id,
        &[
            crate::constants::ORACLE_SOURCE_PDA_SEED,
            month.as_ref(),
            source_id,
        ],
    )
}

pub fn find_oracle_source_observations_address(program_id: &Pubkey, source: &Pubkey) -> Pubkey {
    namespaced(
        program_id,
        &[
            crate::constants::ORACLE_SOURCE_OBSERVATIONS_PDA_SEED,
            source.as_ref(),
        ],
    )
}

pub fn find_oracle_support_address(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    supporter: &Pubkey,
) -> Pubkey {
    namespaced(
        program_id,
        &[
            crate::constants::ORACLE_SUPPORT_POSITION_PDA_SEED,
            month.as_ref(),
            source.as_ref(),
            supporter.as_ref(),
        ],
    )
}

pub fn find_oracle_source_challenge_address(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    challenge_id: &[u8; 32],
) -> Pubkey {
    namespaced(
        program_id,
        &[
            crate::constants::ORACLE_SOURCE_CHALLENGE_PDA_SEED,
            month.as_ref(),
            source.as_ref(),
            challenge_id,
        ],
    )
}

/// The carry-period PDA of a month. Proposals pass it as an absent canonical proof until the
/// month registers a carry period.
pub fn find_carry_period_address(program_id: &Pubkey, month: &Pubkey) -> Pubkey {
    namespaced(program_id, &[b"g3-oracle-carry-period", month.as_ref()])
}

pub fn find_reward_vault_token_address(program_id: &Pubkey, usdc_mint: &Pubkey) -> Pubkey {
    crate::associated_token::get_associated_token_address_with_program_id(
        &derive_oracle_usdc_reward_vault_pda(program_id).0,
        usdc_mint,
        &crate::token_instruction::id(),
    )
}

pub fn initialize_oracle_usdc_reward_vault(
    program_id: &Pubkey,
    payer: &Pubkey,
    usdc_mint: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::InitializeOracleUsdcRewardVault,
        vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(*usdc_mint, false),
            AccountMeta::new(derive_oracle_usdc_reward_vault_pda(program_id).0, false),
            AccountMeta::new(
                find_reward_vault_token_address(program_id, usdc_mint),
                false,
            ),
            AccountMeta::new_readonly(crate::token_instruction::id(), false),
            AccountMeta::new_readonly(crate::associated_token::id(), false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

pub fn deposit_oracle_usdc_rewards(
    program_id: &Pubkey,
    funder: &Pubkey,
    funder_token: &Pubkey,
    usdc_mint: &Pubkey,
    amount: u64,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::DepositOracleUsdcRewards {
            params: DepositOracleUsdcRewardsParams { amount },
        },
        vec![
            AccountMeta::new_readonly(*funder, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(*usdc_mint, false),
            AccountMeta::new(derive_oracle_usdc_reward_vault_pda(program_id).0, false),
            AccountMeta::new(*funder_token, false),
            AccountMeta::new(
                find_reward_vault_token_address(program_id, usdc_mint),
                false,
            ),
            AccountMeta::new_readonly(crate::token_instruction::id(), false),
        ],
    )
}

pub fn begin_oracle_usdc_reward_schedule(
    program_id: &Pubkey,
    authority: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::BeginOracleUsdcRewardSchedule {
            params: BeginOracleUsdcRewardScheduleParams,
        },
        vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new_readonly(derive_oracle_usdc_reward_vault_pda(program_id).0, false),
            AccountMeta::new(
                derive_oracle_usdc_reward_schedule_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

pub fn add_oracle_usdc_sku_budget(
    program_id: &Pubkey,
    authority: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    params: AddOracleUsdcSkuBudgetParams,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let sku = derive_oracle_usdc_sku_pool_pda(program_id, &schedule, &params.bucket_id).0;
    instruction(
        program_id,
        OracleInstruction::AddOracleUsdcSkuBudget { params },
        vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new(schedule, false),
            AccountMeta::new(sku, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

pub fn finalize_oracle_usdc_reward_schedule(
    program_id: &Pubkey,
    authority: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    usdc_mint: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::FinalizeOracleUsdcRewardSchedule,
        vec![
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new_readonly(*usdc_mint, false),
            AccountMeta::new(derive_oracle_usdc_reward_vault_pda(program_id).0, false),
            AccountMeta::new_readonly(
                find_reward_vault_token_address(program_id, usdc_mint),
                false,
            ),
            AccountMeta::new(
                derive_oracle_usdc_reward_schedule_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new_readonly(crate::token_instruction::id(), false),
        ],
    )
}

/// Propose a source with sealed locator and definition objects written by `object_payer`.
pub fn propose_oracle_source_v3(
    program_id: &Pubkey,
    proposer: &Pubkey,
    object_payer: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    params: ProposeOracleSourceV3Params,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let sku = derive_oracle_usdc_sku_pool_pda(program_id, &schedule, &params.bucket_id).0;
    let source = find_oracle_source_address(program_id, month, &params.source_id);
    let locator = params.canonical_locator_hash;
    let definition = params.source_definition_hash;
    instruction(
        program_id,
        OracleInstruction::ProposeOracleSourceV3 { params },
        vec![
            AccountMeta::new(*proposer, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new(schedule, false),
            AccountMeta::new_readonly(sku, false),
            AccountMeta::new(source, false),
            AccountMeta::new(
                find_oracle_source_observations_address(program_id, &source),
                false,
            ),
            AccountMeta::new(
                derive_oracle_usdc_source_reward_pda(program_id, &schedule, &source).0,
                false,
            ),
            AccountMeta::new(derive_user_collateral_pda(program_id, proposer).0, false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(
                derive_oracle_sku_coverage_manifest_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new_readonly(find_carry_period_address(program_id, month), false),
            AccountMeta::new_readonly(
                find_evidence_object_address(program_id, object_payer, 1, &locator),
                false,
            ),
            AccountMeta::new(
                find_evidence_link_address(program_id, &source, 0, &locator),
                false,
            ),
            AccountMeta::new_readonly(
                find_evidence_object_address(program_id, object_payer, 2, &definition),
                false,
            ),
            AccountMeta::new(
                find_evidence_link_address(program_id, &source, 1, &definition),
                false,
            ),
        ],
    )
}

pub fn support_oracle_source_v3(
    program_id: &Pubkey,
    supporter: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    bucket_id: &[u8; 32],
    params: SupportOracleSourceV3Params,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    instruction(
        program_id,
        OracleInstruction::SupportOracleSourceV3 { params },
        vec![
            AccountMeta::new(*supporter, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new(schedule, false),
            AccountMeta::new_readonly(
                derive_oracle_usdc_sku_pool_pda(program_id, &schedule, bucket_id).0,
                false,
            ),
            AccountMeta::new(*source, false),
            AccountMeta::new(
                derive_oracle_usdc_source_reward_pda(program_id, &schedule, source).0,
                false,
            ),
            AccountMeta::new(derive_user_collateral_pda(program_id, supporter).0, false),
            AccountMeta::new(
                find_oracle_support_address(program_id, month, source, supporter),
                false,
            ),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new(
                derive_oracle_sku_coverage_manifest_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new(
                derive_oracle_sku_coverage_record_pda(program_id, month, bucket_id).0,
                false,
            ),
        ],
    )
}

fn coverage_crank(
    program_id: &Pubkey,
    data: OracleInstruction,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        data,
        vec![
            AccountMeta::new_readonly(*cranker, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new(
                derive_oracle_sku_coverage_manifest_pda(program_id, month).0,
                false,
            ),
        ],
    )
}

pub fn finalize_oracle_sku_coverage(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
) -> Instruction {
    coverage_crank(
        program_id,
        OracleInstruction::FinalizeOracleSkuCoverage,
        cranker,
        period,
        month,
    )
}

pub fn reopen_oracle_sku_coverage(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
) -> Instruction {
    coverage_crank(
        program_id,
        OracleInstruction::ReopenOracleSkuCoverage,
        cranker,
        period,
        month,
    )
}

pub fn timeout_unsupported_oracle_source_v2(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::TimeoutUnsupportedOracleSourceV2,
        vec![
            AccountMeta::new_readonly(*cranker, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new_readonly(
                derive_oracle_sku_coverage_manifest_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new(*source, false),
        ],
    )
}

/// `coverage_record` is `Some(bucket_id)` exactly when the source has nonzero support.
pub fn expire_unlistable_oracle_source_v2(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    coverage_record: Option<&[u8; 32]>,
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new_readonly(*cranker, true),
        AccountMeta::new_readonly(*period, false),
        AccountMeta::new(*month, false),
        AccountMeta::new(
            derive_oracle_sku_coverage_manifest_pda(program_id, month).0,
            false,
        ),
        AccountMeta::new(*source, false),
    ];
    if let Some(bucket_id) = coverage_record {
        accounts.push(AccountMeta::new(
            derive_oracle_sku_coverage_record_pda(program_id, month, bucket_id).0,
            false,
        ));
    }
    instruction(
        program_id,
        OracleInstruction::ExpireUnlistableOracleSourceV2,
        accounts,
    )
}

/// Challenge a Candidate source. `comparison_source` is the other source of a
/// non-independence challenge.
#[allow(clippy::too_many_arguments)]
pub fn challenge_oracle_source_v2(
    program_id: &Pubkey,
    challenger: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    bucket_id: &[u8; 32],
    comparison_source: Option<&Pubkey>,
    params: ChallengeOracleSourceParams,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let challenge =
        find_oracle_source_challenge_address(program_id, month, source, &params.challenge_id);
    let mut accounts = vec![
        AccountMeta::new(*challenger, true),
        AccountMeta::new_readonly(*period, false),
        AccountMeta::new(*month, false),
        AccountMeta::new_readonly(
            derive_oracle_usdc_sku_pool_pda(program_id, &schedule, bucket_id).0,
            false,
        ),
        AccountMeta::new_readonly(*source, false),
        AccountMeta::new(derive_user_collateral_pda(program_id, challenger).0, false),
        AccountMeta::new(challenge, false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new(
            derive_oracle_source_challenge_guard_pda(program_id, month, source).0,
            false,
        ),
    ];
    if let Some(comparison) = comparison_source {
        accounts.push(AccountMeta::new_readonly(*comparison, false));
        accounts.push(AccountMeta::new(
            derive_oracle_source_challenge_guard_pda(program_id, month, comparison).0,
            false,
        ));
    }
    accounts.push(AccountMeta::new(schedule, false));
    instruction(
        program_id,
        OracleInstruction::ChallengeOracleSourceV2 { params },
        accounts,
    )
}

/// Resolve a source challenge in RuleReview. For `MergeSource`, `merge` carries the comparison
/// source and the challenged source's bucket.
#[allow(clippy::too_many_arguments)]
pub fn resolve_oracle_source_challenge_v2(
    program_id: &Pubkey,
    authority: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    challenge: &Pubkey,
    bucket_id: &[u8; 32],
    comparison_source: Option<&Pubkey>,
    outcome: crate::state::OracleSourceChallengeOutcome,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let mut accounts = vec![
        AccountMeta::new_readonly(*authority, true),
        AccountMeta::new_readonly(find_config_address(program_id).0, false),
        AccountMeta::new(*month, false),
        AccountMeta::new(*source, false),
        AccountMeta::new(*challenge, false),
    ];
    if outcome == crate::state::OracleSourceChallengeOutcome::MergeSource {
        let comparison = comparison_source.expect("a merge names its comparison source");
        accounts.extend([
            AccountMeta::new(*comparison, false),
            AccountMeta::new_readonly(
                derive_oracle_usdc_sku_pool_pda(program_id, &schedule, bucket_id).0,
                false,
            ),
            AccountMeta::new(
                derive_oracle_usdc_source_reward_pda(program_id, &schedule, source).0,
                false,
            ),
            AccountMeta::new(
                derive_oracle_usdc_source_reward_pda(program_id, &schedule, comparison).0,
                false,
            ),
        ]);
    }
    accounts.push(AccountMeta::new(
        derive_oracle_source_challenge_guard_pda(program_id, month, source).0,
        false,
    ));
    if let Some(comparison) = comparison_source {
        accounts.push(AccountMeta::new(
            derive_oracle_source_challenge_guard_pda(program_id, month, comparison).0,
            false,
        ));
    }
    accounts.push(AccountMeta::new(
        derive_oracle_sku_coverage_manifest_pda(program_id, month).0,
        false,
    ));
    accounts.push(AccountMeta::new(
        derive_oracle_sku_coverage_record_pda(program_id, month, bucket_id).0,
        false,
    ));
    instruction(
        program_id,
        OracleInstruction::ResolveOracleSourceChallengeV2 {
            params: ResolveOracleSourceChallengeParams { outcome },
        },
        accounts,
    )
}

pub fn cancel_stale_oracle_source_challenge_v2(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    challenge: &Pubkey,
    comparison_source: Option<&Pubkey>,
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new_readonly(*cranker, true),
        AccountMeta::new_readonly(*period, false),
        AccountMeta::new(*month, false),
        AccountMeta::new(
            derive_oracle_sku_coverage_manifest_pda(program_id, month).0,
            false,
        ),
        AccountMeta::new(*challenge, false),
        AccountMeta::new(
            derive_oracle_source_challenge_guard_pda(program_id, month, source).0,
            false,
        ),
    ];
    if let Some(comparison) = comparison_source {
        accounts.push(AccountMeta::new(
            derive_oracle_source_challenge_guard_pda(program_id, month, comparison).0,
            false,
        ));
    }
    instruction(
        program_id,
        OracleInstruction::CancelStaleOracleSourceChallengeV2,
        accounts,
    )
}

/// The exact counted-escrow account tail shared by tags 169 and 194 for the three pre-listing
/// kinds.
#[derive(Clone, Debug)]
pub enum PrelistingEscrow {
    ListingBond {
        source: Pubkey,
        proposer: Pubkey,
    },
    SupportStake {
        source: Pubkey,
        supporter: Pubkey,
    },
    SourceChallenge {
        source: Pubkey,
        proposer: Pubkey,
        challenge: Pubkey,
        challenger: Pubkey,
        comparison_source: Option<Pubkey>,
    },
}

fn prelisting_escrow_accounts(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    escrow: &PrelistingEscrow,
) -> (OracleEscrowKind, Vec<AccountMeta>) {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let mut accounts = vec![
        AccountMeta::new_readonly(*cranker, true),
        AccountMeta::new_readonly(*period, false),
    ];
    let kind = match escrow {
        PrelistingEscrow::ListingBond { source, proposer } => {
            accounts.extend([
                AccountMeta::new_readonly(*month, false),
                AccountMeta::new(schedule, false),
                AccountMeta::new(*source, false),
                AccountMeta::new(
                    derive_oracle_usdc_source_reward_pda(program_id, &schedule, source).0,
                    false,
                ),
                AccountMeta::new(derive_user_collateral_pda(program_id, proposer).0, false),
            ]);
            OracleEscrowKind::ListingBond
        }
        PrelistingEscrow::SupportStake { source, supporter } => {
            accounts.extend([
                AccountMeta::new_readonly(*month, false),
                AccountMeta::new(schedule, false),
                AccountMeta::new_readonly(*source, false),
                AccountMeta::new(
                    find_oracle_support_address(program_id, month, source, supporter),
                    false,
                ),
                AccountMeta::new(derive_user_collateral_pda(program_id, supporter).0, false),
            ]);
            OracleEscrowKind::SupportStake
        }
        PrelistingEscrow::SourceChallenge {
            source,
            proposer,
            challenge,
            challenger,
            comparison_source,
        } => {
            accounts.extend([
                AccountMeta::new(*month, false),
                AccountMeta::new(schedule, false),
                AccountMeta::new(*source, false),
                AccountMeta::new(
                    derive_oracle_usdc_source_reward_pda(program_id, &schedule, source).0,
                    false,
                ),
                AccountMeta::new(*challenge, false),
                AccountMeta::new(derive_user_collateral_pda(program_id, proposer).0, false),
                AccountMeta::new(derive_user_collateral_pda(program_id, challenger).0, false),
                AccountMeta::new(
                    derive_oracle_source_challenge_guard_pda(program_id, month, source).0,
                    false,
                ),
            ]);
            if let Some(comparison) = comparison_source {
                accounts.push(AccountMeta::new(
                    derive_oracle_source_challenge_guard_pda(program_id, month, comparison).0,
                    false,
                ));
            }
            OracleEscrowKind::SourceChallenge
        }
    };
    (kind, accounts)
}

/// Tag 169 for one of the counted pre-listing kinds.
pub fn settle_oracle_usdc_prelisting_escrow(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    escrow: &PrelistingEscrow,
) -> Instruction {
    let (kind, accounts) = prelisting_escrow_accounts(program_id, cranker, period, month, escrow);
    settle_oracle_usdc_escrow(program_id, kind, accounts)
}

/// Tag 169 with an explicit account list (opening and update kinds).
pub fn settle_oracle_usdc_escrow(
    program_id: &Pubkey,
    kind: OracleEscrowKind,
    accounts: Vec<AccountMeta>,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::SettleOracleUsdcEscrow {
            params: SettleOracleEscrowParams { kind },
        },
        accounts,
    )
}

/// Tag 194: the same counted pre-listing tails, only once the month has failed to list.
pub fn settle_failed_oracle_month_escrow_v2(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    escrow: &PrelistingEscrow,
) -> Instruction {
    let (kind, accounts) = prelisting_escrow_accounts(program_id, cranker, period, month, escrow);
    instruction(
        program_id,
        OracleInstruction::SettleFailedOracleMonthEscrowV2 {
            params: SettleOracleEscrowParams { kind },
        },
        accounts,
    )
}

pub fn abort_oracle_usdc_reward_schedule_v2(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::AbortOracleUsdcRewardScheduleV2,
        vec![
            AccountMeta::new_readonly(*cranker, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new_readonly(
                derive_oracle_sku_coverage_manifest_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new(derive_oracle_usdc_reward_vault_pda(program_id).0, false),
            AccountMeta::new(
                derive_oracle_usdc_reward_schedule_pda(program_id, month).0,
                false,
            ),
        ],
    )
}

pub fn find_oracle_opening_claim_address(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
) -> Pubkey {
    namespaced(
        program_id,
        &[
            crate::constants::ORACLE_OPENING_CLAIM_PDA_SEED,
            month.as_ref(),
            source.as_ref(),
        ],
    )
}

pub fn find_oracle_opening_challenge_address(
    program_id: &Pubkey,
    month: &Pubkey,
    claim: &Pubkey,
    challenge_id: &[u8; 32],
) -> Pubkey {
    namespaced(
        program_id,
        &[
            crate::constants::ORACLE_OPENING_CLAIM_CHALLENGE_PDA_SEED,
            month.as_ref(),
            claim.as_ref(),
            challenge_id,
        ],
    )
}

/// The member page `page` of a bucket source index.
pub fn find_member_page_address(program_id: &Pubkey, bucket_index: &Pubkey, page: u16) -> Pubkey {
    namespaced(
        program_id,
        &[
            b"g3-oracle-members-page",
            bucket_index.as_ref(),
            &page.to_le_bytes(),
        ],
    )
}

/// A source's checkpoint journal and the checkpoint of one accepted event.
pub fn find_source_journal_address(program_id: &Pubkey, source: &Pubkey) -> Pubkey {
    namespaced(program_id, &[b"g3-oracle-knowledge", source.as_ref()])
}

pub fn find_checkpoint_address(program_id: &Pubkey, source: &Pubkey, event: &Pubkey) -> Pubkey {
    namespaced(
        program_id,
        &[b"g3-oracle-checkpoint", source.as_ref(), event.as_ref()],
    )
}

/// The carry-source companion PDA of a source; absent unless the source was imported.
pub fn find_carry_source_address(program_id: &Pubkey, source: &Pubkey) -> Pubkey {
    namespaced(program_id, &[b"g3-oracle-carry-source", source.as_ref()])
}

/// A bucket-rank certificate proposed by `payer` for one bucket source index.
pub fn find_bucket_rank_address(
    program_id: &Pubkey,
    bucket_index: &Pubkey,
    payer: &Pubkey,
    nonce: &[u8; 32],
) -> Pubkey {
    namespaced(
        program_id,
        &[
            b"g3-bucket-rank-v1",
            bucket_index.as_ref(),
            payer.as_ref(),
            nonce,
        ],
    )
}

/// Commitment of a sealed Wayback archive URL (evidence kind 3).
pub fn archive_url_hash(archive_url: &str) -> [u8; 32] {
    solana_program::hash::hashv(&[
        crate::constants::ORACLE_OPENING_ARCHIVE_URL_HASH_DOMAIN,
        archive_url.as_bytes(),
    ])
    .to_bytes()
}

/// The opening (or alternative opening) evidence commitment a claim stores.
pub fn opening_evidence_hash(
    month: &Pubkey,
    source: &Pubkey,
    opening_state: u64,
    source_time: u64,
    canonical_locator_hash: &[u8; 32],
    source_definition_hash: &[u8; 32],
    archive_url: &str,
) -> [u8; 32] {
    solana_program::hash::hashv(&[
        crate::constants::ORACLE_OPENING_EVIDENCE_HASH_DOMAIN,
        month.as_ref(),
        source.as_ref(),
        &opening_state.to_le_bytes(),
        &source_time.to_le_bytes(),
        canonical_locator_hash,
        source_definition_hash,
        archive_url.as_bytes(),
    ])
    .to_bytes()
}

fn recipe_actor_accounts(
    program_id: &Pubkey,
    authority: AccountMeta,
    period: &Pubkey,
    month: AccountMeta,
) -> Vec<AccountMeta> {
    vec![
        authority,
        AccountMeta::new_readonly(*period, false),
        AccountMeta::new_readonly(find_config_address(program_id).0, false),
        month,
    ]
}

pub fn begin_oracle_recipe_weights_v3(
    program_id: &Pubkey,
    authority: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    expected_source_count: u16,
    expected_bucket_count: u16,
) -> Instruction {
    let mut accounts = recipe_actor_accounts(
        program_id,
        AccountMeta::new(*authority, true),
        period,
        AccountMeta::new(*month, false),
    );
    accounts.extend([
        AccountMeta::new(
            derive_oracle_recipe_weight_manifest_pda(program_id, month).0,
            false,
        ),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(
            derive_oracle_sku_coverage_manifest_pda(program_id, month).0,
            false,
        ),
        AccountMeta::new_readonly(find_carry_period_address(program_id, month), false),
    ]);
    instruction(
        program_id,
        OracleInstruction::BeginOracleRecipeWeightsV3 {
            params: BeginOracleRecipeWeightsV2Params {
                expected_source_count,
                expected_bucket_count,
            },
        },
        accounts,
    )
}

pub fn accumulate_oracle_recipe_bucket_v2(
    program_id: &Pubkey,
    authority: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    params: AccumulateOracleRecipeBucketV2Params,
    sources: &[Pubkey],
) -> Instruction {
    let mut accounts = recipe_actor_accounts(
        program_id,
        AccountMeta::new_readonly(*authority, true),
        period,
        AccountMeta::new_readonly(*month, false),
    );
    accounts.push(AccountMeta::new(
        derive_oracle_recipe_weight_manifest_pda(program_id, month).0,
        false,
    ));
    accounts.extend(
        sources
            .iter()
            .map(|source| AccountMeta::new(*source, false)),
    );
    instruction(
        program_id,
        OracleInstruction::AccumulateOracleRecipeBucketV2 { params },
        accounts,
    )
}

pub fn finalize_oracle_recipe_weights_v2(
    program_id: &Pubkey,
    authority: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
) -> Instruction {
    let mut accounts = recipe_actor_accounts(
        program_id,
        AccountMeta::new_readonly(*authority, true),
        period,
        AccountMeta::new(*month, false),
    );
    accounts.push(AccountMeta::new(
        derive_oracle_recipe_weight_manifest_pda(program_id, month).0,
        false,
    ));
    instruction(
        program_id,
        OracleInstruction::FinalizeOracleRecipeWeightsV2,
        accounts,
    )
}

/// Index one frozen source. `member_page` is the page the bucket's next member lands on.
pub fn index_oracle_recipe_source_v1(
    program_id: &Pubkey,
    payer: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    params: IndexOracleRecipeSourceV1Params,
    member_page: u16,
) -> Instruction {
    let bucket_index =
        derive_oracle_bucket_source_index_pda(program_id, month, &params.bucket_id).0;
    instruction(
        program_id,
        OracleInstruction::IndexOracleRecipeSourceV1 { params },
        vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new_readonly(
                derive_oracle_recipe_weight_manifest_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new(
                derive_oracle_recipe_source_index_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new(bucket_index, false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new(
                find_member_page_address(program_id, &bucket_index, member_page),
                false,
            ),
        ],
    )
}

pub fn accumulate_oracle_settlement_source_bucket(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    params: AccumulateOracleSettlementSourceBucketParams,
    buckets: &[[u8; 32]],
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new(*cranker, true),
        AccountMeta::new_readonly(*period, false),
        AccountMeta::new_readonly(*month, false),
        AccountMeta::new(
            derive_oracle_settlement_source_manifest_pda(program_id, month).0,
            false,
        ),
        AccountMeta::new_readonly(system_program::ID, false),
    ];
    accounts.extend(buckets.iter().map(|bucket| {
        AccountMeta::new_readonly(
            derive_oracle_bucket_median_pda(program_id, month, bucket).0,
            false,
        )
    }));
    instruction(
        program_id,
        OracleInstruction::AccumulateOracleSettlementSourceBucket { params },
        accounts,
    )
}

/// Submit an opening print. The archive URL travels only through the sealed kind-3 object that
/// `object_payer` wrote; `params.archive_url` must be empty.
#[allow(clippy::too_many_arguments)]
pub fn submit_oracle_opening_claim_v2(
    program_id: &Pubkey,
    claimant: &Pubkey,
    object_payer: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    bucket_id: &[u8; 32],
    params: SubmitOracleOpeningClaimParams,
    archive_url: &str,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let claim = find_oracle_opening_claim_address(program_id, month, source);
    let evidence = opening_evidence_hash(
        month,
        source,
        params.opening_state,
        params.source_time,
        &params.canonical_locator_hash,
        &params.source_definition_hash,
        archive_url,
    );
    instruction(
        program_id,
        OracleInstruction::SubmitOracleOpeningClaimV2 { params },
        vec![
            AccountMeta::new(*claimant, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new_readonly(
                derive_oracle_usdc_sku_pool_pda(program_id, &schedule, bucket_id).0,
                false,
            ),
            AccountMeta::new(*source, false),
            AccountMeta::new(derive_user_collateral_pda(program_id, claimant).0, false),
            AccountMeta::new(claim, false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(
                find_evidence_object_address(
                    program_id,
                    object_payer,
                    3,
                    &archive_url_hash(archive_url),
                ),
                false,
            ),
            AccountMeta::new(
                find_evidence_link_address(program_id, &claim, 2, &evidence),
                false,
            ),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn challenge_oracle_opening_claim_v2(
    program_id: &Pubkey,
    challenger: &Pubkey,
    object_payer: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    bucket_id: &[u8; 32],
    params: ChallengeOracleOpeningClaimParams,
    archive_url: &str,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let claim = find_oracle_opening_claim_address(program_id, month, source);
    let challenge =
        find_oracle_opening_challenge_address(program_id, month, &claim, &params.challenge_id);
    let evidence = opening_evidence_hash(
        month,
        source,
        params.alternative_opening_state,
        params.alternative_source_time,
        &params.canonical_locator_hash,
        &params.source_definition_hash,
        archive_url,
    );
    instruction(
        program_id,
        OracleInstruction::ChallengeOracleOpeningClaimV2 { params },
        vec![
            AccountMeta::new(*challenger, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new_readonly(
                derive_oracle_usdc_sku_pool_pda(program_id, &schedule, bucket_id).0,
                false,
            ),
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new(claim, false),
            AccountMeta::new(derive_user_collateral_pda(program_id, challenger).0, false),
            AccountMeta::new(challenge, false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(
                find_evidence_object_address(
                    program_id,
                    object_payer,
                    3,
                    &archive_url_hash(archive_url),
                ),
                false,
            ),
            AccountMeta::new(
                find_evidence_link_address(program_id, &challenge, 3, &evidence),
                false,
            ),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn resolve_oracle_opening_claim_challenge_v2(
    program_id: &Pubkey,
    authority: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    bucket_id: &[u8; 32],
    claimant: &Pubkey,
    challenger: &Pubkey,
    challenge: &Pubkey,
    outcome: crate::state::OracleOpeningChallengeOutcome,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    instruction(
        program_id,
        OracleInstruction::ResolveOracleOpeningClaimChallengeV2 {
            params: ResolveOracleOpeningClaimChallengeParams { outcome },
        },
        vec![
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new(*month, false),
            AccountMeta::new_readonly(
                derive_oracle_usdc_sku_pool_pda(program_id, &schedule, bucket_id).0,
                false,
            ),
            AccountMeta::new(*source, false),
            AccountMeta::new(
                find_oracle_opening_claim_address(program_id, month, source),
                false,
            ),
            AccountMeta::new_readonly(derive_user_collateral_pda(program_id, claimant).0, false),
            AccountMeta::new_readonly(derive_user_collateral_pda(program_id, challenger).0, false),
            AccountMeta::new(*challenge, false),
        ],
    )
}

pub fn finalize_oracle_opening_claim_v2(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    claimant: &Pubkey,
) -> Instruction {
    let claim = find_oracle_opening_claim_address(program_id, month, source);
    instruction(
        program_id,
        OracleInstruction::FinalizeOracleOpeningClaimV2,
        vec![
            AccountMeta::new(*cranker, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new(*source, false),
            AccountMeta::new(
                find_oracle_source_observations_address(program_id, source),
                false,
            ),
            AccountMeta::new(claim, false),
            AccountMeta::new_readonly(derive_user_collateral_pda(program_id, claimant).0, false),
            AccountMeta::new(find_source_journal_address(program_id, source), false),
            AccountMeta::new(find_checkpoint_address(program_id, source, &claim), false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

pub fn expire_oracle_opening_source(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::ExpireOracleOpeningSource,
        vec![
            AccountMeta::new_readonly(*cranker, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new(*source, false),
            AccountMeta::new_readonly(find_carry_source_address(program_id, source), false),
        ],
    )
}

pub fn begin_oracle_active_weights(
    program_id: &Pubkey,
    actor: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    expected_source_count: u16,
    expected_group_count: u16,
) -> Instruction {
    let mut accounts = recipe_actor_accounts(
        program_id,
        AccountMeta::new(*actor, true),
        period,
        AccountMeta::new(*month, false),
    );
    accounts.extend([
        AccountMeta::new_readonly(
            derive_oracle_recipe_weight_manifest_pda(program_id, month).0,
            false,
        ),
        AccountMeta::new(
            derive_oracle_active_weight_manifest_pda(program_id, month).0,
            false,
        ),
        AccountMeta::new_readonly(system_program::ID, false),
    ]);
    instruction(
        program_id,
        OracleInstruction::BeginOracleActiveWeights {
            params: BeginOracleActiveWeightsParams {
                expected_source_count,
                expected_group_count,
            },
        },
        accounts,
    )
}

/// Collect one source of `group_id`. `rank` is a completed bucket-rank certificate when the group
/// has more active sources than the inline median holds; otherwise any read-only account.
#[allow(clippy::too_many_arguments)]
pub fn accumulate_oracle_active_weight_group(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    params: AccumulateOracleActiveWeightGroupParams,
    source: &Pubkey,
    member_page: u16,
    rank: &Pubkey,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let bucket_index = derive_oracle_bucket_source_index_pda(program_id, month, &params.group_id).0;
    let sku = derive_oracle_usdc_sku_pool_pda(program_id, &schedule, &params.group_id).0;
    let bucket = derive_oracle_bucket_median_pda(program_id, month, &params.group_id).0;
    instruction(
        program_id,
        OracleInstruction::AccumulateOracleActiveWeightGroup { params },
        vec![
            AccountMeta::new(*cranker, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new(
                derive_oracle_active_weight_manifest_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new(sku, false),
            AccountMeta::new(bucket, false),
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new_readonly(
                derive_oracle_recipe_source_index_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new_readonly(bucket_index, false),
            AccountMeta::new_readonly(
                find_member_page_address(program_id, &bucket_index, member_page),
                false,
            ),
            AccountMeta::new_readonly(*rank, false),
        ],
    )
}

pub fn finalize_oracle_active_weights(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::FinalizeOracleActiveWeights,
        vec![
            AccountMeta::new_readonly(*cranker, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new_readonly(
                derive_oracle_recipe_weight_manifest_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new(
                derive_oracle_active_weight_manifest_pda(program_id, month).0,
                false,
            ),
        ],
    )
}

/// Tag 80: the month enters Game and the period becomes Live.
pub fn finalize_oracle_opening_phase(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::FinalizeOracleOpeningPhase,
        vec![
            AccountMeta::new_readonly(*cranker, true),
            AccountMeta::new(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new_readonly(
                derive_oracle_active_weight_manifest_pda(program_id, month).0,
                false,
            ),
        ],
    )
}

/// Tag 197 for one source of a bucket's recompute walk.
#[allow(clippy::too_many_arguments)]
pub fn recompute_oracle_bucket_median_v1(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    params: RecomputeOracleBucketMedianV1Params,
    source: &Pubkey,
    observations: &Pubkey,
    member_page: u16,
    rank: &Pubkey,
) -> Instruction {
    let bucket_index =
        derive_oracle_bucket_source_index_pda(program_id, month, &params.bucket_id).0;
    let bucket = derive_oracle_bucket_median_pda(program_id, month, &params.bucket_id).0;
    instruction(
        program_id,
        OracleInstruction::RecomputeOracleBucketMedianV1 { params },
        vec![
            AccountMeta::new_readonly(*cranker, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new(bucket, false),
            AccountMeta::new(*source, false),
            AccountMeta::new_readonly(*observations, false),
            AccountMeta::new_readonly(
                derive_oracle_recipe_source_index_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new_readonly(bucket_index, false),
            AccountMeta::new_readonly(
                find_member_page_address(program_id, &bucket_index, member_page),
                false,
            ),
            AccountMeta::new_readonly(*rank, false),
        ],
    )
}

/// The update (or alternative update) evidence commitment a claim or challenge stores.
#[allow(clippy::too_many_arguments)]
pub fn update_evidence_hash(
    month: &Pubkey,
    source: &Pubkey,
    source_id: &[u8; 32],
    state: u64,
    source_time: u64,
    canonical_locator_hash: &[u8; 32],
    source_definition_hash: &[u8; 32],
    archive_url: &str,
) -> [u8; 32] {
    solana_program::hash::hashv(&[
        crate::constants::ORACLE_UPDATE_EVIDENCE_HASH_DOMAIN,
        month.as_ref(),
        source.as_ref(),
        source_id,
        &state.to_le_bytes(),
        &source_time.to_le_bytes(),
        canonical_locator_hash,
        source_definition_hash,
        archive_url.as_bytes(),
    ])
    .to_bytes()
}

pub fn find_oracle_update_claim_address(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    claimant: &Pubkey,
    claim_id: &[u8; 32],
) -> Pubkey {
    crate::state::derive_oracle_update_claim_v2_pda(program_id, month, source, claimant, claim_id).0
}

pub fn find_oracle_update_challenge_address(
    program_id: &Pubkey,
    month: &Pubkey,
    claim: &Pubkey,
    challenge_id: &[u8; 32],
) -> Pubkey {
    namespaced(
        program_id,
        &[
            crate::constants::ORACLE_UPDATE_CHALLENGE_PDA_SEED,
            month.as_ref(),
            claim.as_ref(),
            challenge_id,
        ],
    )
}

pub fn find_oracle_update_guard_address(
    program_id: &Pubkey,
    month: &Pubkey,
    claim: &Pubkey,
) -> Pubkey {
    crate::state::derive_oracle_update_challenge_guard_pda(program_id, month, claim).0
}

/// Accounts: 0 [signer, writable] claimant and payer; 1 period; 2 [writable] month; 3 source;
/// 4 finalized active-weight manifest; 5 SKU pool; 6 [writable] update claim PDA;
/// 7 [writable] claimant collateral; 8 system program.
#[allow(clippy::too_many_arguments)]
pub fn commit_oracle_update_claim_v3(
    program_id: &Pubkey,
    claimant: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    bucket_id: &[u8; 32],
    params: CommitOracleUpdateClaimV2Params,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let claim =
        find_oracle_update_claim_address(program_id, month, source, claimant, &params.claim_id);
    instruction(
        program_id,
        OracleInstruction::CommitOracleUpdateClaimV3 { params },
        vec![
            AccountMeta::new(*claimant, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new_readonly(
                derive_oracle_active_weight_manifest_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new_readonly(
                derive_oracle_usdc_sku_pool_pda(program_id, &schedule, bucket_id).0,
                false,
            ),
            AccountMeta::new(claim, false),
            AccountMeta::new(derive_user_collateral_pda(program_id, claimant).0, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

/// Accounts: 0 [signer, writable] claimant and payer; 1 period; 2 month; 3 source;
/// 4 finalized active-weight manifest; 5 [writable] update claim; 6 system program; 7 sealed
/// archive-URL object written by `object_payer`; 8 [writable] publication link.
#[allow(clippy::too_many_arguments)]
pub fn reveal_oracle_update_claim_v3(
    program_id: &Pubkey,
    claimant: &Pubkey,
    object_payer: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    params: RevealOracleUpdateClaimV3Params,
    archive_url: &str,
) -> Instruction {
    let claim =
        find_oracle_update_claim_address(program_id, month, source, claimant, &params.claim_id);
    let evidence = params.evidence_hash;
    instruction(
        program_id,
        OracleInstruction::RevealOracleUpdateClaimV3 { params },
        vec![
            AccountMeta::new(*claimant, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new_readonly(
                derive_oracle_active_weight_manifest_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new(claim, false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(
                find_evidence_object_address(
                    program_id,
                    object_payer,
                    3,
                    &archive_url_hash(archive_url),
                ),
                false,
            ),
            AccountMeta::new(
                find_evidence_link_address(program_id, &claim, 4, &evidence),
                false,
            ),
        ],
    )
}

/// Accounts: 0 [signer, writable] challenger and payer; 1 period; 2 month; 3 SKU pool; 4 claim;
/// 5 source; 6 [writable] challenger collateral; 7 [writable] challenge PDA; 8 system program;
/// 9 [writable] update-challenge guard PDA; 10 sealed archive-URL object written by
/// `object_payer`; 11 [writable] publication link.
#[allow(clippy::too_many_arguments)]
pub fn challenge_oracle_update_claim_v2(
    program_id: &Pubkey,
    challenger: &Pubkey,
    object_payer: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    bucket_id: &[u8; 32],
    claim: &Pubkey,
    params: ChallengeOracleUpdateClaimParams,
    archive_url: &str,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let challenge =
        find_oracle_update_challenge_address(program_id, month, claim, &params.challenge_id);
    let evidence = params.evidence_hash;
    instruction(
        program_id,
        OracleInstruction::ChallengeOracleUpdateClaimV2 { params },
        vec![
            AccountMeta::new(*challenger, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new_readonly(
                derive_oracle_usdc_sku_pool_pda(program_id, &schedule, bucket_id).0,
                false,
            ),
            AccountMeta::new_readonly(*claim, false),
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new(derive_user_collateral_pda(program_id, challenger).0, false),
            AccountMeta::new(challenge, false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new(
                find_oracle_update_guard_address(program_id, month, claim),
                false,
            ),
            AccountMeta::new_readonly(
                find_evidence_object_address(
                    program_id,
                    object_payer,
                    3,
                    &archive_url_hash(archive_url),
                ),
                false,
            ),
            AccountMeta::new(
                find_evidence_link_address(program_id, &challenge, 5, &evidence),
                false,
            ),
        ],
    )
}

/// How a finalized update claim's trailing accounts are shaped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateFinalization {
    /// Unchallenged acceptance: the absent guard proof, then the checkpoint tail.
    Unchallenged,
    /// A challenged claim accepted, or rejected for a changed alternative: the challenge and
    /// guard, then the checkpoint tail (its event is the claim).
    Challenged { challenge: Pubkey },
    /// A challenged claim rejected for an alternative equal to the standing state: the
    /// challenge and guard, no checkpoint.
    ChallengedUnchanged { challenge: Pubkey },
    /// Rule review left unresolved: the writable challenge and writable guard.
    Unresolved { challenge: Pubkey },
}

/// Fixed accounts: 0 [signer, writable] admin or oracle authority (payer of the checkpoint);
/// 1 period; 2 config; 3 [writable] month; 4 [writable] source; 5 [writable] observations;
/// 6 finalized active-weight manifest; 7 [writable] claim; 8 [writable] bucket median. The tail
/// follows [`UpdateFinalization`]; a checkpoint tail is the source journal, the checkpoint PDA
/// keyed by the claim, and the system program.
#[allow(clippy::too_many_arguments)]
pub fn finalize_oracle_update_claim_v2(
    program_id: &Pubkey,
    authority: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    bucket_id: &[u8; 32],
    claim: &Pubkey,
    params: FinalizeOracleUpdateClaimV2Params,
    shape: UpdateFinalization,
) -> Instruction {
    let guard = find_oracle_update_guard_address(program_id, month, claim);
    let mut accounts = vec![
        AccountMeta::new(*authority, true),
        AccountMeta::new_readonly(*period, false),
        AccountMeta::new_readonly(find_config_address(program_id).0, false),
        AccountMeta::new(*month, false),
        AccountMeta::new(*source, false),
        AccountMeta::new(
            find_oracle_source_observations_address(program_id, source),
            false,
        ),
        AccountMeta::new_readonly(
            derive_oracle_active_weight_manifest_pda(program_id, month).0,
            false,
        ),
        AccountMeta::new(*claim, false),
        AccountMeta::new(
            derive_oracle_bucket_median_pda(program_id, month, bucket_id).0,
            false,
        ),
    ];
    let checkpoint_tail = [
        AccountMeta::new(find_source_journal_address(program_id, source), false),
        AccountMeta::new(find_checkpoint_address(program_id, source, claim), false),
        AccountMeta::new_readonly(system_program::ID, false),
    ];
    match shape {
        UpdateFinalization::Unchallenged => {
            accounts.push(AccountMeta::new_readonly(guard, false));
            accounts.extend(checkpoint_tail);
        }
        UpdateFinalization::Challenged { challenge } => {
            accounts.push(AccountMeta::new(challenge, false));
            accounts.push(AccountMeta::new_readonly(guard, false));
            accounts.extend(checkpoint_tail);
        }
        UpdateFinalization::ChallengedUnchanged { challenge } => {
            accounts.push(AccountMeta::new(challenge, false));
            accounts.push(AccountMeta::new_readonly(guard, false));
        }
        UpdateFinalization::Unresolved { challenge } => {
            accounts.push(AccountMeta::new(challenge, false));
            accounts.push(AccountMeta::new(guard, false));
        }
    }
    instruction(
        program_id,
        OracleInstruction::FinalizeOracleUpdateClaimV2 { params },
        accounts,
    )
}

/// Accounts: 0 [signer] cranker; 1 period; 2 [writable] month; 3 source; 4 [writable] claim;
/// 5 [writable] claimant collateral.
pub fn settle_expired_oracle_update_commitment_v3(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    claim: &Pubkey,
    claimant: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::SettleExpiredOracleUpdateCommitmentV3,
        vec![
            AccountMeta::new_readonly(*cranker, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new(*claim, false),
            AccountMeta::new(derive_user_collateral_pda(program_id, claimant).0, false),
        ],
    )
}

/// Accounts: 0 [signer] cranker; 1 period; 2 [writable] month; 3 source; 4 [writable] claim;
/// 5 the absent update-guard PDA, or [writable] the guard; 6 [writable] the guarded challenge
/// when the guard exists.
pub fn cancel_stale_oracle_update_claim_v2(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    claim: &Pubkey,
    challenge: Option<&Pubkey>,
) -> Instruction {
    let guard = find_oracle_update_guard_address(program_id, month, claim);
    let mut accounts = vec![
        AccountMeta::new_readonly(*cranker, true),
        AccountMeta::new_readonly(*period, false),
        AccountMeta::new(*month, false),
        AccountMeta::new_readonly(*source, false),
        AccountMeta::new(*claim, false),
    ];
    match challenge {
        Some(challenge) => {
            accounts.push(AccountMeta::new(guard, false));
            accounts.push(AccountMeta::new(*challenge, false));
        }
        None => accounts.push(AccountMeta::new_readonly(guard, false)),
    }
    instruction(
        program_id,
        OracleInstruction::CancelStaleOracleUpdateClaimV2,
        accounts,
    )
}

pub fn find_carry_registry_address(program_id: &Pubkey, underlying_id: &[u8; 32]) -> Pubkey {
    namespaced(program_id, &[b"g3-oracle-carry-registry", underlying_id])
}

/// Tag 30 selector 0: register the first carry period of a product on a month in Game or Settled.
///
/// Accounts: 0 [signer, writable] oracle authority and payer; 1 config; 2 period; 3 month;
/// 4 [writable] carry registry PDA; 5 [writable] carry period PDA; 6 system program.
pub fn register_carry_root(
    program_id: &Pubkey,
    authority: &Pubkey,
    underlying_id: &[u8; 32],
    period: &Pubkey,
    month: &Pubkey,
) -> Instruction {
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::RegisterRoot,
        vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new(
                find_carry_registry_address(program_id, underlying_id),
                false,
            ),
            AccountMeta::new(find_carry_period_address(program_id, month), false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

/// Tag 30 selector 1: register a month in source submission as the successor of the product's
/// latest carry period, before its scramble and before its reward schedule exists.
///
/// Accounts: 0 [signer, writable] oracle authority and payer; 1 config; 2 period; 3 month;
/// 4 [writable] carry registry; 5 [writable] carry period PDA; 6 system program; 7 parent
/// period; 8 parent month; 9 the absent reward-schedule PDA of the month.
#[allow(clippy::too_many_arguments)]
pub fn register_carry_successor(
    program_id: &Pubkey,
    authority: &Pubkey,
    underlying_id: &[u8; 32],
    period: &Pubkey,
    month: &Pubkey,
    parent_period: &Pubkey,
    parent_month: &Pubkey,
) -> Instruction {
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::RegisterSuccessor,
        vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new(
                find_carry_registry_address(program_id, underlying_id),
                false,
            ),
            AccountMeta::new(find_carry_period_address(program_id, month), false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(*parent_period, false),
            AccountMeta::new_readonly(*parent_month, false),
            AccountMeta::new_readonly(
                derive_oracle_usdc_reward_schedule_pda(program_id, month).0,
                false,
            ),
        ],
    )
}

/// Tag 30 selector 2: enroll an already accepted opening (kind 0), update (1) or accepted
/// alternative (2) of a source that has no journal yet. `event` is the accepted claim or
/// challenge; kind 2 appends the rejected claim.
///
/// Accounts: 0 [signer, writable] payer; 1 period; 2 month; 3 source; 4 observations; 5 event;
/// 6 [writable] the absent source journal; 7 [writable] checkpoint PDA; 8 system program;
/// 9 the rejected claim (kind 2 only).
#[allow(clippy::too_many_arguments)]
pub fn capture_current_checkpoint(
    program_id: &Pubkey,
    payer: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    event: &Pubkey,
    rejected_claim: Option<&Pubkey>,
    kind: u8,
    previous_hash: [u8; 32],
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new(*payer, true),
        AccountMeta::new_readonly(*period, false),
        AccountMeta::new_readonly(*month, false),
        AccountMeta::new_readonly(*source, false),
        AccountMeta::new_readonly(
            find_oracle_source_observations_address(program_id, source),
            false,
        ),
        AccountMeta::new_readonly(*event, false),
        AccountMeta::new(find_source_journal_address(program_id, source), false),
        AccountMeta::new(find_checkpoint_address(program_id, source, event), false),
        AccountMeta::new_readonly(system_program::ID, false),
    ];
    if let Some(claim) = rejected_claim {
        accounts.push(AccountMeta::new_readonly(*claim, false));
    }
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::CaptureCurrent {
            kind,
            previous_hash,
        },
        accounts,
    )
}

/// The parent side of an import or skip: the parent month's source at the carry period's next
/// import index, with its recipe and bucket membership.
pub struct CarryParent {
    pub period: Pubkey,
    pub month: Pubkey,
    pub source: Pubkey,
    pub bucket_id: [u8; 32],
    pub member_page: u16,
}

/// Tag 30 selector 3: import the parent's next Active source into the month with a new listing
/// bond from `importer`. The locator and definition objects are the sealed preimages written by
/// `object_payer`.
///
/// Accounts: 0-10 exactly as ProposeOracleSourceV3's first eleven; 11 parent source; 12 parent
/// period; 13 parent month; 14 parent recipe source index; 15 parent bucket source index;
/// 16 [writable] carry period; 17 [writable] carry source PDA; 18 member page; 19 locator object;
/// 20 [writable] locator link; 21 definition object; 22 [writable] definition link.
#[allow(clippy::too_many_arguments)]
pub fn import_carry_source(
    program_id: &Pubkey,
    importer: &Pubkey,
    object_payer: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source_id: &[u8; 32],
    canonical_locator_hash: &[u8; 32],
    source_definition_hash: &[u8; 32],
    parent: &CarryParent,
    sku_index: u16,
    proof: Vec<[u8; 32]>,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let sku = derive_oracle_usdc_sku_pool_pda(program_id, &schedule, &parent.bucket_id).0;
    let source = find_oracle_source_address(program_id, month, source_id);
    let bucket_index =
        derive_oracle_bucket_source_index_pda(program_id, &parent.month, &parent.bucket_id).0;
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::Import { sku_index, proof },
        vec![
            AccountMeta::new(*importer, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new(schedule, false),
            AccountMeta::new_readonly(sku, false),
            AccountMeta::new(source, false),
            AccountMeta::new(
                find_oracle_source_observations_address(program_id, &source),
                false,
            ),
            AccountMeta::new(
                derive_oracle_usdc_source_reward_pda(program_id, &schedule, &source).0,
                false,
            ),
            AccountMeta::new(derive_user_collateral_pda(program_id, importer).0, false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(
                derive_oracle_sku_coverage_manifest_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new_readonly(parent.source, false),
            AccountMeta::new_readonly(parent.period, false),
            AccountMeta::new_readonly(parent.month, false),
            AccountMeta::new_readonly(
                derive_oracle_recipe_source_index_pda(program_id, &parent.month).0,
                false,
            ),
            AccountMeta::new_readonly(bucket_index, false),
            AccountMeta::new(find_carry_period_address(program_id, month), false),
            AccountMeta::new(find_carry_source_address(program_id, &source), false),
            AccountMeta::new_readonly(
                find_member_page_address(program_id, &bucket_index, parent.member_page),
                false,
            ),
            AccountMeta::new_readonly(
                find_evidence_object_address(program_id, object_payer, 1, canonical_locator_hash),
                false,
            ),
            AccountMeta::new(
                find_evidence_link_address(program_id, &source, 0, canonical_locator_hash),
                false,
            ),
            AccountMeta::new_readonly(
                find_evidence_object_address(program_id, object_payer, 2, source_definition_hash),
                false,
            ),
            AccountMeta::new(
                find_evidence_link_address(program_id, &source, 1, source_definition_hash),
                false,
            ),
        ],
    )
}

/// Tag 30 selector 4: skip the parent's next source when it is Inactive.
///
/// Accounts: 0 period; 1 month; 2 [writable] carry period; 3 parent period; 4 parent month;
/// 5 parent source; 6 parent recipe source index; 7 parent bucket source index; 8 cranker;
/// 9 member page.
pub fn skip_inactive_carry_source(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    parent: &CarryParent,
) -> Instruction {
    let bucket_index =
        derive_oracle_bucket_source_index_pda(program_id, &parent.month, &parent.bucket_id).0;
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::SkipInactive,
        vec![
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new(find_carry_period_address(program_id, month), false),
            AccountMeta::new_readonly(parent.period, false),
            AccountMeta::new_readonly(parent.month, false),
            AccountMeta::new_readonly(parent.source, false),
            AccountMeta::new_readonly(
                derive_oracle_recipe_source_index_pda(program_id, &parent.month).0,
                false,
            ),
            AccountMeta::new_readonly(bucket_index, false),
            AccountMeta::new_readonly(*cranker, true),
            AccountMeta::new_readonly(
                find_member_page_address(program_id, &bucket_index, parent.member_page),
                false,
            ),
        ],
    )
}

/// Tag 30 selector 5: after the listing, mark an imported source's opening fresh (accepted
/// opening claim) or start scanning its parent's checkpoints from the parent journal head.
///
/// Accounts: 0 period; 1 month; 2 source; 3 [writable] carry source; 4 carry period; 5 the
/// source's opening-claim PDA; 6 parent source; 7 parent journal; 8 parent month; 9 cranker.
#[allow(clippy::too_many_arguments)]
pub fn begin_carry_selection(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    parent_month: &Pubkey,
    parent_source: &Pubkey,
) -> Instruction {
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::BeginSelection,
        vec![
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new(find_carry_source_address(program_id, source), false),
            AccountMeta::new_readonly(find_carry_period_address(program_id, month), false),
            AccountMeta::new_readonly(
                find_oracle_opening_claim_address(program_id, month, source),
                false,
            ),
            AccountMeta::new_readonly(*parent_source, false),
            AccountMeta::new_readonly(
                find_source_journal_address(program_id, parent_source),
                false,
            ),
            AccountMeta::new_readonly(*parent_month, false),
            AccountMeta::new_readonly(*cranker, true),
        ],
    )
}

/// Tag 30 selector 6: one backwards step over the parent checkpoint at the carry cursor.
///
/// Accounts: 0 source; 1 [writable] carry source; 2 the checkpoint at the cursor.
pub fn scan_carry_checkpoint(
    program_id: &Pubkey,
    source: &Pubkey,
    checkpoint: &Pubkey,
) -> Instruction {
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::ScanCheckpoint,
        vec![
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new(find_carry_source_address(program_id, source), false),
            AccountMeta::new_readonly(*checkpoint, false),
        ],
    )
}

/// Tag 30 selector 7: open an imported Frozen source at its selected parent checkpoint.
///
/// Accounts: 0 [signer, writable] payer; 1 period; 2 [writable] month; 3 [writable] source;
/// 4 [writable] carry source; 5 carry period; 6 the source's opening-claim PDA; 7 [writable]
/// observations; 8 SKU pool; 9 reward schedule; 10 [writable] source journal; 11 [writable]
/// checkpoint PDA keyed by the carry source; 12 system program.
pub fn freeze_carry_opening(
    program_id: &Pubkey,
    payer: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    bucket_id: &[u8; 32],
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let carry = find_carry_source_address(program_id, source);
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::FreezeOpening,
        vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new(*source, false),
            AccountMeta::new(carry, false),
            AccountMeta::new_readonly(find_carry_period_address(program_id, month), false),
            AccountMeta::new_readonly(
                find_oracle_opening_claim_address(program_id, month, source),
                false,
            ),
            AccountMeta::new(
                find_oracle_source_observations_address(program_id, source),
                false,
            ),
            AccountMeta::new_readonly(
                derive_oracle_usdc_sku_pool_pda(program_id, &schedule, bucket_id).0,
                false,
            ),
            AccountMeta::new_readonly(schedule, false),
            AccountMeta::new(find_source_journal_address(program_id, source), false),
            AccountMeta::new(find_checkpoint_address(program_id, source, &carry), false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

/// Tag 59: close a Settled month.
pub fn close_oracle_month(
    program_id: &Pubkey,
    authority: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::CloseOracleMonth,
        vec![
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new(*month, false),
            AccountMeta::new_readonly(
                crate::state::derive_settlement_v2_pda(program_id, period, month).0,
                false,
            ),
        ],
    )
}

/// Tag 75: finalize a month from its bucket medians, given every bucket of the month in group
/// order.
pub fn finalize_oracle_month(
    program_id: &Pubkey,
    cranker: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    bucket_ids: &[[u8; 32]],
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new_readonly(*cranker, true),
        AccountMeta::new_readonly(*period, false),
        AccountMeta::new(*month, false),
    ];
    for bucket_id in bucket_ids {
        accounts.push(AccountMeta::new_readonly(
            derive_oracle_bucket_median_pda(program_id, month, bucket_id).0,
            false,
        ));
    }
    instruction(program_id, OracleInstruction::FinalizeOracleMonth, accounts)
}

/// Tag 79: fail a Forming period that can no longer list.
pub fn fail_period(
    program_id: &Pubkey,
    cranker: &Pubkey,
    product_id: &[u8; 32],
    expiry_ts: u64,
) -> Instruction {
    let period = find_period_address(program_id, product_id, expiry_ts).0;
    instruction(
        program_id,
        OracleInstruction::FailPeriod,
        vec![
            AccountMeta::new_readonly(*cranker, true),
            AccountMeta::new(period, false),
            AccountMeta::new_readonly(
                find_oracle_month_address(program_id, &period, expiry_ts),
                false,
            ),
        ],
    )
}

/// Tag 90: the signer registry and its version-1 set.
pub fn initialize_settlement_signer_registry(
    program_id: &Pubkey,
    admin: &Pubkey,
    oracle_authority: &Pubkey,
    recovery_authority: &Pubkey,
    params: crate::instruction::InitializeSettlementSignerRegistryParams,
) -> Instruction {
    let set =
        crate::state::derive_settlement_signer_set_pda(program_id, params.signer_set_version).0;
    instruction(
        program_id,
        OracleInstruction::InitializeSettlementSignerRegistry { params },
        vec![
            AccountMeta::new(*admin, true),
            AccountMeta::new_readonly(*oracle_authority, true),
            AccountMeta::new_readonly(*recovery_authority, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new(derive_settlement_signer_registry_pda(program_id).0, false),
            AccountMeta::new(set, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

/// Tag 91 (`recovery_authority` = None) or 94 (Some): schedule the next signer set.
pub fn propose_settlement_signer_rotation(
    program_id: &Pubkey,
    admin: &Pubkey,
    oracle_authority: &Pubkey,
    recovery_authority: Option<&Pubkey>,
    current_version: u64,
    params: crate::instruction::ProposeSettlementSignerRotationParams,
) -> Instruction {
    let registry = derive_settlement_signer_registry_pda(program_id).0;
    let current = crate::state::derive_settlement_signer_set_pda(program_id, current_version).0;
    let pending =
        crate::state::derive_settlement_signer_set_pda(program_id, params.target_version).0;
    let mut accounts = vec![
        AccountMeta::new(*admin, true),
        AccountMeta::new_readonly(*oracle_authority, true),
    ];
    if let Some(recovery) = recovery_authority {
        accounts.push(AccountMeta::new_readonly(*recovery, true));
    }
    accounts.extend([
        AccountMeta::new_readonly(find_config_address(program_id).0, false),
        AccountMeta::new(registry, false),
        AccountMeta::new_readonly(current, false),
        AccountMeta::new(pending, false),
    ]);
    if recovery_authority.is_none() {
        accounts.push(AccountMeta::new_readonly(
            solana_program::sysvar::instructions::ID,
            false,
        ));
    }
    accounts.push(AccountMeta::new_readonly(system_program::ID, false));
    let data = if recovery_authority.is_some() {
        OracleInstruction::ProposeEmergencySettlementSignerRecovery { params }
    } else {
        OracleInstruction::ProposeSettlementSignerRotation { params }
    };
    instruction(program_id, data, accounts)
}

/// Tag 92: activate the pending signer set after its timelock.
pub fn activate_settlement_signer_rotation(
    program_id: &Pubkey,
    pending_version: u64,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::ActivateSettlementSignerRotation,
        vec![
            AccountMeta::new(derive_settlement_signer_registry_pda(program_id).0, false),
            AccountMeta::new_readonly(
                crate::state::derive_settlement_signer_set_pda(program_id, pending_version).0,
                false,
            ),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
        ],
    )
}

/// Tag 93: cancel the pending rotation.
pub fn cancel_settlement_signer_rotation(
    program_id: &Pubkey,
    admin: &Pubkey,
    oracle_authority: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::CancelSettlementSignerRotation,
        vec![
            AccountMeta::new_readonly(*admin, true),
            AccountMeta::new_readonly(*oracle_authority, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new(derive_settlement_signer_registry_pda(program_id).0, false),
        ],
    )
}

/// Tag 116: publish `settlement` for the month of `period` at `expiry_ts`, signed by the
/// current signer set of version `signer_set_version`. The Ed25519 attestations must precede it.
pub fn upsert_settlement_v3(
    program_id: &Pubkey,
    authority: &Pubkey,
    product_id: &[u8; 32],
    expiry_ts: u64,
    signer_set_version: u64,
    settlement: crate::instruction::SettlementLeaf,
) -> Instruction {
    let period = find_period_address(program_id, product_id, expiry_ts).0;
    let month = find_oracle_month_address(program_id, &period, expiry_ts);
    instruction(
        program_id,
        OracleInstruction::UpsertSettlementV3 {
            params: crate::instruction::UpsertSettlementParams { settlement },
        },
        vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new(period, false),
            AccountMeta::new(month, false),
            AccountMeta::new(
                crate::state::derive_settlement_v2_pda(program_id, &period, &month).0,
                false,
            ),
            AccountMeta::new_readonly(
                derive_oracle_recipe_weight_manifest_pda(program_id, &month).0,
                false,
            ),
            AccountMeta::new_readonly(
                derive_oracle_active_weight_manifest_pda(program_id, &month).0,
                false,
            ),
            AccountMeta::new_readonly(
                derive_oracle_settlement_source_manifest_pda(program_id, &month).0,
                false,
            ),
            AccountMeta::new_readonly(derive_settlement_signer_registry_pda(program_id).0, false),
            AccountMeta::new_readonly(
                crate::state::derive_settlement_signer_set_pda(program_id, signer_set_version).0,
                false,
            ),
            AccountMeta::new_readonly(solana_program::sysvar::instructions::ID, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

/// The message settlement signers sign for `settlement` under the given signer set: the
/// attestation digest tag 116 verifies.
pub fn settlement_attestation_message(
    program_id: &Pubkey,
    signer_set_version: u64,
    signer_set_hash: &[u8; 32],
    settlement: &crate::instruction::SettlementLeaf,
) -> Vec<u8> {
    let registry = derive_settlement_signer_registry_pda(program_id).0;
    let set = crate::state::derive_settlement_signer_set_pda(program_id, signer_set_version).0;
    let payload = settlement
        .signed_payload_bytes()
        .expect("settlement payload serializes");
    let payload_hash = solana_program::hash::hashv(&[payload.as_slice()]).to_bytes();
    solana_program::hash::hashv(&[
        b"ameba_settlement_attestation_v2",
        program_id.as_ref(),
        registry.as_ref(),
        set.as_ref(),
        &signer_set_version.to_le_bytes(),
        signer_set_hash,
        &payload_hash,
    ])
    .to_bytes()
    .to_vec()
}

/// The council case of `target` in `month` and its ballot round for `epoch`.
pub fn find_council_case_address(program_id: &Pubkey, month: &Pubkey, target: &Pubkey) -> Pubkey {
    namespaced(
        program_id,
        &[b"g3-council-case-v1", month.as_ref(), target.as_ref()],
    )
}

pub fn find_council_round_address(program_id: &Pubkey, case: &Pubkey, epoch: u64) -> Pubkey {
    namespaced(
        program_id,
        &[b"g3-council-round-v1", case.as_ref(), &epoch.to_le_bytes()],
    )
}

/// Tag 30 selector 18: open (0), vote (1, `voter` signs), apply (2) or abort a stale update (3).
///
/// Accounts: 0 [signer, writable] payer; 1 period; 2 [writable] month; 3 [writable] case;
/// 4 [writable] target; 5 the controller Config; 6 [writable] current-epoch round; 7 system
/// program; 8 [signer] voter (vote only); then the kind's target accounts.
#[allow(clippy::too_many_arguments)]
pub fn council_action(
    program_id: &Pubkey,
    payer: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    target: &Pubkey,
    controller_config: &Pubkey,
    action: OracleCouncilActionV1,
    voter: Option<&Pubkey>,
    remaining: Vec<AccountMeta>,
) -> Instruction {
    let case = find_council_case_address(program_id, month, target);
    let round = find_council_round_address(program_id, &case, action.expected_epoch);
    let mut accounts = vec![
        AccountMeta::new(*payer, true),
        AccountMeta::new_readonly(*period, false),
        AccountMeta::new(*month, false),
        AccountMeta::new(case, false),
        AccountMeta::new(*target, false),
        AccountMeta::new_readonly(*controller_config, false),
        AccountMeta::new(round, false),
        AccountMeta::new_readonly(system_program::ID, false),
    ];
    if let Some(voter) = voter {
        accounts.push(AccountMeta::new_readonly(*voter, true));
    }
    accounts.extend(remaining);
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::Council(action),
        accounts,
    )
}

/// Tag 170: register a terminal source for cash rewards. `opening_claim` is the accepted opening
/// claim of an Active source opened fresh.
pub fn register_oracle_usdc_reward_source(
    program_id: &Pubkey,
    payer: &Pubkey,
    month: &Pubkey,
    bucket_id: &[u8; 32],
    source: &Pubkey,
    opening_claim: Option<&Pubkey>,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let mut accounts = vec![
        AccountMeta::new_readonly(*payer, true),
        AccountMeta::new_readonly(*month, false),
        AccountMeta::new(schedule, false),
        AccountMeta::new(
            derive_oracle_usdc_sku_pool_pda(program_id, &schedule, bucket_id).0,
            false,
        ),
        AccountMeta::new_readonly(
            derive_oracle_active_weight_manifest_pda(program_id, month).0,
            false,
        ),
        AccountMeta::new_readonly(*source, false),
        AccountMeta::new(
            derive_oracle_usdc_source_reward_pda(program_id, &schedule, source).0,
            false,
        ),
    ];
    if let Some(claim) = opening_claim {
        accounts.push(AccountMeta::new_readonly(*claim, false));
    }
    accounts.push(AccountMeta::new_readonly(
        find_carry_source_address(program_id, source),
        false,
    ));
    instruction(
        program_id,
        OracleInstruction::RegisterOracleUsdcRewardSource,
        accounts,
    )
}

/// Tag 171: register an accepted update for cash rewards.
pub fn register_oracle_usdc_reward_update(
    program_id: &Pubkey,
    payer: &Pubkey,
    month: &Pubkey,
    bucket_id: &[u8; 32],
    source: &Pubkey,
    claim: &Pubkey,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    instruction(
        program_id,
        OracleInstruction::RegisterOracleUsdcRewardUpdate,
        vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new(schedule, false),
            AccountMeta::new(
                derive_oracle_usdc_sku_pool_pda(program_id, &schedule, bucket_id).0,
                false,
            ),
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new_readonly(*claim, false),
            AccountMeta::new(
                crate::state::derive_oracle_usdc_reward_registration_pda(
                    program_id,
                    &schedule,
                    crate::state::OracleUsdcRewardKind::Update,
                    claim,
                )
                .0,
                false,
            ),
            AccountMeta::new_readonly(
                derive_oracle_bucket_median_pda(program_id, month, bucket_id).0,
                false,
            ),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
    )
}

/// Tag 173: freeze the month's cash entitlements.
pub fn finalize_oracle_usdc_reward_entitlements(
    program_id: &Pubkey,
    authority: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
) -> Instruction {
    instruction(
        program_id,
        OracleInstruction::FinalizeOracleUsdcRewardEntitlements,
        vec![
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new_readonly(derive_oracle_usdc_reward_vault_pda(program_id).0, false),
            AccountMeta::new(
                derive_oracle_usdc_reward_schedule_pda(program_id, month).0,
                false,
            ),
        ],
    )
}

/// Tag 174: claim one reward. `subject` keys the receipt (source for the proposer, support
/// position for a supporter, claim for an opening or update); `trailing` is the kind's account
/// tail, starting with the writable SKU pool.
#[allow(clippy::too_many_arguments)]
pub fn claim_oracle_usdc_reward(
    program_id: &Pubkey,
    recipient: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    usdc_mint: &Pubkey,
    kind: crate::state::OracleUsdcRewardKind,
    subject: &Pubkey,
    trailing: Vec<AccountMeta>,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let mut accounts = vec![
        AccountMeta::new(*recipient, true),
        AccountMeta::new_readonly(find_config_address(program_id).0, false),
        AccountMeta::new_readonly(*period, false),
        AccountMeta::new_readonly(*month, false),
        AccountMeta::new(schedule, false),
        AccountMeta::new(derive_oracle_usdc_reward_vault_pda(program_id).0, false),
        AccountMeta::new(
            find_reward_vault_token_address(program_id, usdc_mint),
            false,
        ),
        AccountMeta::new(find_bond_vault_address(program_id, usdc_mint), false),
        AccountMeta::new_readonly(*usdc_mint, false),
        AccountMeta::new(derive_user_collateral_pda(program_id, recipient).0, false),
        AccountMeta::new(
            crate::state::derive_oracle_usdc_reward_receipt_pda(
                program_id, &schedule, kind, subject, recipient,
            )
            .0,
            false,
        ),
        AccountMeta::new_readonly(crate::token_instruction::id(), false),
        AccountMeta::new_readonly(system_program::ID, false),
    ];
    accounts.extend(trailing);
    instruction(
        program_id,
        OracleInstruction::ClaimOracleUsdcReward {
            params: ClaimOracleUsdcRewardParams { kind },
        },
        accounts,
    )
}

/// The receipt of a month's terminal cleanup.
pub fn find_cleanup_receipt_address(program_id: &Pubkey, month: &Pubkey) -> Pubkey {
    namespaced(program_id, &[b"g3-terminal-cleanup-v1", month.as_ref()])
}

/// Tag 30 selector 24: start the cleanup of a Settled month (kind 0) or reclaim one target.
///
/// Accounts: 0 [signer, writable] admin; 1 config; 2 period; 3 [writable] month; 4 settlement
/// record; 5 [writable] rent recipient; 6 [writable] cleanup receipt; 7 system program; 8 carry
/// registry of the product (or its absent PDA); 9 the registry's latest carry period (or the
/// system program); 10 [writable] target (kinds 1-6); 11 the kind's extra account (the reward
/// schedule for kind 1, the recipe source index for kind 2, the council case for kind 5).
#[allow(clippy::too_many_arguments)]
pub fn terminal_cleanup(
    program_id: &Pubkey,
    admin: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    recipient: &Pubkey,
    underlying_id: &[u8; 32],
    latest_carry_period: Option<&Pubkey>,
    params: TerminalCleanupParams,
    target: Option<&Pubkey>,
    extra: Option<&Pubkey>,
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new(*admin, true),
        AccountMeta::new_readonly(find_config_address(program_id).0, false),
        AccountMeta::new_readonly(*period, false),
        AccountMeta::new(*month, false),
        AccountMeta::new_readonly(
            crate::state::derive_settlement_v2_pda(program_id, period, month).0,
            false,
        ),
        AccountMeta::new(*recipient, recipient == admin),
        AccountMeta::new(find_cleanup_receipt_address(program_id, month), false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(
            find_carry_registry_address(program_id, underlying_id),
            false,
        ),
        AccountMeta::new_readonly(
            latest_carry_period.copied().unwrap_or(system_program::ID),
            false,
        ),
    ];
    if let Some(target) = target {
        accounts.push(AccountMeta::new(*target, false));
    }
    if let Some(extra) = extra {
        accounts.push(AccountMeta::new_readonly(*extra, false));
    }
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::TerminalCleanup(params),
        accounts,
    )
}

/// The accounts of an underwritten source (tag 30 selector 22): its agreement, the researcher's
/// agent reserve, and the funder's exposure to that reserve.
pub struct SponsoredAccounts {
    pub agreement: Pubkey,
    pub reserve: Pubkey,
    pub funder: Pubkey,
    pub exposure: Pubkey,
}

impl SponsoredAccounts {
    /// `funder` is the sponsor's cash owner, or `None` for a researcher-funded (reserve) bond.
    pub fn new(
        program_id: &Pubkey,
        terms: &crate::sponsorship::SponsorTerms,
        source: &Pubkey,
        funder: Option<&Pubkey>,
    ) -> Self {
        let reserve =
            crate::sponsorship::reserve_address(program_id, &terms.agent_id, &terms.researcher).0;
        let funder = funder.copied().unwrap_or(reserve);
        Self {
            agreement: crate::sponsorship::agreement_address(program_id, source).0,
            reserve,
            funder,
            exposure: crate::sponsorship::exposure_address(program_id, &reserve, &funder).0,
        }
    }
}

/// Selector 22 / Activate: propose `params` as an underwritten source. The listing bond moves
/// from the funder's ledger to the agreement's, which becomes the source's proposer. `signer`
/// is the sponsor (cash) or the researcher (reserve). The locator and definition objects are the
/// preimages written by `object_payer`.
#[allow(clippy::too_many_arguments)]
pub fn sponsored_activate(
    program_id: &Pubkey,
    payer: &Pubkey,
    signer: &Pubkey,
    object_payer: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    terms: crate::sponsorship::SponsorTerms,
    params: ProposeOracleSourceV3Params,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let source = find_oracle_source_address(program_id, month, &params.source_id);
    let funder = if terms.funding == 1 {
        None
    } else {
        Some(signer)
    };
    let sponsored = SponsoredAccounts::new(program_id, &terms, &source, funder);
    let locator = params.canonical_locator_hash;
    let definition = params.source_definition_hash;
    let accounts = vec![
        AccountMeta::new(*payer, true),
        AccountMeta::new_readonly(*period, false),
        AccountMeta::new(*month, false),
        AccountMeta::new(schedule, false),
        AccountMeta::new_readonly(
            derive_oracle_usdc_sku_pool_pda(program_id, &schedule, &params.bucket_id).0,
            false,
        ),
        AccountMeta::new(source, false),
        AccountMeta::new(
            find_oracle_source_observations_address(program_id, &source),
            false,
        ),
        AccountMeta::new(
            derive_oracle_usdc_source_reward_pda(program_id, &schedule, &source).0,
            false,
        ),
        AccountMeta::new(
            derive_user_collateral_pda(program_id, &sponsored.agreement).0,
            false,
        ),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(
            derive_oracle_sku_coverage_manifest_pda(program_id, month).0,
            false,
        ),
        AccountMeta::new_readonly(find_carry_period_address(program_id, month), false),
        AccountMeta::new_readonly(
            find_evidence_object_address(program_id, object_payer, 1, &locator),
            false,
        ),
        AccountMeta::new(
            find_evidence_link_address(program_id, &source, 0, &locator),
            false,
        ),
        AccountMeta::new_readonly(
            find_evidence_object_address(program_id, object_payer, 2, &definition),
            false,
        ),
        AccountMeta::new(
            find_evidence_link_address(program_id, &source, 1, &definition),
            false,
        ),
        AccountMeta::new(sponsored.agreement, false),
        AccountMeta::new(sponsored.reserve, false),
        AccountMeta::new_readonly(*signer, true),
        AccountMeta::new(
            derive_user_collateral_pda(program_id, &sponsored.funder).0,
            false,
        ),
        AccountMeta::new(
            derive_user_collateral_pda(program_id, &sponsored.reserve).0,
            false,
        ),
        AccountMeta::new(sponsored.exposure, false),
    ];
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::SponsoredSource(OracleSponsoredActionV1::Activate {
            terms,
            source: params,
        }),
        accounts,
    )
}

/// Selector 22 / Reconcile: after the source's listing bond settles, record its outcome and
/// sweep the agreement's cash to the funder. Permissionless.
pub fn sponsored_reconcile(
    program_id: &Pubkey,
    cranker: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    sponsored: &SponsoredAccounts,
) -> Instruction {
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::SponsoredSource(OracleSponsoredActionV1::Reconcile),
        vec![
            AccountMeta::new_readonly(*cranker, true),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new(sponsored.agreement, false),
            AccountMeta::new(sponsored.reserve, false),
            AccountMeta::new(
                derive_user_collateral_pda(program_id, &sponsored.agreement).0,
                false,
            ),
            AccountMeta::new(
                derive_user_collateral_pda(program_id, &sponsored.funder).0,
                false,
            ),
            AccountMeta::new(sponsored.exposure, false),
        ],
    )
}

/// Selector 22 / ClaimBounty: claim the source's proposer reward for the agreement and split it
/// between the funder, the researcher and the researcher's reserve. Permissionless.
#[allow(clippy::too_many_arguments)]
pub fn sponsored_claim_bounty(
    program_id: &Pubkey,
    payer: &Pubkey,
    period: &Pubkey,
    month: &Pubkey,
    usdc_mint: &Pubkey,
    bucket_id: &[u8; 32],
    source: &Pubkey,
    researcher: &Pubkey,
    sponsored: &SponsoredAccounts,
) -> Instruction {
    let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
    let mut accounts = claim_oracle_usdc_reward(
        program_id,
        payer,
        period,
        month,
        usdc_mint,
        crate::state::OracleUsdcRewardKind::SourceProposer,
        source,
        vec![
            AccountMeta::new(
                derive_oracle_usdc_sku_pool_pda(program_id, &schedule, bucket_id).0,
                false,
            ),
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new_readonly(
                derive_oracle_usdc_source_reward_pda(program_id, &schedule, source).0,
                false,
            ),
            AccountMeta::new_readonly(find_carry_source_address(program_id, source), false),
        ],
    )
    .accounts;
    // The agreement is the reward recipient: its ledger and receipt replace the payer's.
    accounts[9] = AccountMeta::new(
        derive_user_collateral_pda(program_id, &sponsored.agreement).0,
        false,
    );
    accounts[10] = AccountMeta::new(
        crate::state::derive_oracle_usdc_reward_receipt_pda(
            program_id,
            &schedule,
            crate::state::OracleUsdcRewardKind::SourceProposer,
            source,
            &sponsored.agreement,
        )
        .0,
        false,
    );
    accounts.extend([
        AccountMeta::new(sponsored.agreement, false),
        AccountMeta::new(sponsored.reserve, false),
        AccountMeta::new(
            derive_user_collateral_pda(program_id, &sponsored.funder).0,
            false,
        ),
        AccountMeta::new(derive_user_collateral_pda(program_id, researcher).0, false),
        AccountMeta::new(
            derive_user_collateral_pda(program_id, &sponsored.reserve).0,
            false,
        ),
    ]);
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::SponsoredSource(OracleSponsoredActionV1::ClaimBounty),
        accounts,
    )
}

/// Selector 22 / WithdrawReserve: the researcher moves reserve cash to their own ledger.
pub fn sponsored_withdraw_reserve(
    program_id: &Pubkey,
    researcher: &Pubkey,
    reserve: &Pubkey,
    amount: u64,
) -> Instruction {
    oracle_carry_forward(
        program_id,
        OracleCarryForwardActionV1::SponsoredSource(OracleSponsoredActionV1::WithdrawReserve {
            amount,
        }),
        vec![
            AccountMeta::new_readonly(*researcher, true),
            AccountMeta::new_readonly(*reserve, false),
            AccountMeta::new(derive_user_collateral_pda(program_id, reserve).0, false),
            AccountMeta::new(derive_user_collateral_pda(program_id, researcher).0, false),
        ],
    )
}
