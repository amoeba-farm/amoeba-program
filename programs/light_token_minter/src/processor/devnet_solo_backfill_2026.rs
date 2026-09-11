//! Devnet-only, one-shot accelerated backfill for the exact 202609/202610 RAMX/NANDX cohorts.
//!
//! This module is absent from the ordinary artifact. It owns one sidecar PDA and never reads or
//! writes a maturity-ladder registry. Every market and authority is derived or pinned here.

use super::*;
use crate::governance_manifest::{
    is_devnet_solo_backfill_2026_instruction_tag,
    DEVNET_SOLO_BACKFILL_ACCUMULATE_ACTIVE_WEIGHT_TAG as ACCUMULATE_ACTIVE_WEIGHT_TAG,
    DEVNET_SOLO_BACKFILL_ACCUMULATE_RECIPE_TAG as ACCUMULATE_RECIPE_TAG,
    DEVNET_SOLO_BACKFILL_BEGIN_ACTIVE_WEIGHTS_TAG as BEGIN_ACTIVE_WEIGHTS_TAG,
    DEVNET_SOLO_BACKFILL_CREATE_SUPPORTED_SOURCE_TAG as CREATE_SUPPORTED_SOURCE_TAG,
    DEVNET_SOLO_BACKFILL_FINALIZE_ACTIVE_WEIGHTS_TAG as FINALIZE_ACTIVE_WEIGHTS_TAG,
    DEVNET_SOLO_BACKFILL_FINALIZE_COVERAGE_TAG as FINALIZE_COVERAGE_TAG,
    DEVNET_SOLO_BACKFILL_FINALIZE_GAME_TAG as FINALIZE_GAME_TAG,
    DEVNET_SOLO_BACKFILL_FINALIZE_OPENING_TAG as FINALIZE_OPENING_TAG,
    DEVNET_SOLO_BACKFILL_FINALIZE_RECIPE_TAG as FINALIZE_RECIPE_TAG,
    DEVNET_SOLO_BACKFILL_INITIALIZE_COHORT_TAG as INITIALIZE_COHORT_TAG,
    DEVNET_SOLO_BACKFILL_INITIALIZE_SIDECAR_TAG as INITIALIZE_SIDECAR_TAG,
    DEVNET_SOLO_BACKFILL_SUBMIT_OPENING_TAG as SUBMIT_OPENING_TAG,
};
use crate::state::{
    derive_devnet_solo_backfill_2026_v1_pda, derive_oracle_active_weight_manifest_pda,
    derive_oracle_bucket_median_pda, derive_oracle_product_sku_manifest_pda,
    derive_oracle_recipe_weight_manifest_pda, derive_oracle_sku_coverage_manifest_pda,
    derive_oracle_sku_coverage_record_pda, derive_oracle_usdc_reward_schedule_pda,
    derive_oracle_usdc_reward_vault_pda, derive_oracle_usdc_sku_pool_pda,
    derive_oracle_usdc_source_reward_pda, DevnetSoloBackfill2026V1, InstrumentDefinition,
    MarketParameters, OptionKind, OracleActiveWeightManifest, OracleBucketMedianState,
    OracleBucketMedianStatus, OracleOpeningClaim, OracleOpeningClaimStatus,
    OracleRecipeWeightManifest, OracleRecipeWeightPhase, OracleSkuCoverageManifest,
    OracleSkuCoverageRecord, OracleSourceObservations, OracleSourceState, OracleSourceStatus,
    OracleSupportPosition, OracleUsdcRewardSchedule, OracleUsdcRewardSchedulePhase,
    OracleUsdcSkuPool, OracleUsdcSourceReward, SettlementStyle,
    DEVNET_SOLO_BACKFILL_2026_V1_PDA_SEED,
};
use borsh::BorshDeserialize;

const EXPECTED_PROGRAM_ID: Pubkey =
    solana_program::pubkey!("2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw");
const EXPECTED_ADMIN: Pubkey =
    solana_program::pubkey!("99riHvpFvwfz2tbrWbanEMPz5iyhHHThBbM7eY35vMwL");
const EXPECTED_ORACLE_AUTHORITY: Pubkey =
    solana_program::pubkey!("C8gpKjnPss4SKpjhBpFGNH7cbD26XCzSWhcjj6gF4dx7");
const EXPECTED_COLLATERAL_MINT: Pubkey =
    solana_program::pubkey!("21Ft8EZpugvFofW9713vLnYDRfSqyVUGUo9wvvUGhTsZ");

/// V3 Devnet preparation ends at the first cohort expiry, 2026-10-01T00:00:00Z.
/// Consumed bits prevent replay before this fixed terminal bound.
pub(super) const SUNSET_TS: u64 = 1_790_812_800;
pub(super) const PLACEMENT_SECONDS: u64 = 4 * 60 * 60;
pub(super) const CHALLENGE_SECONDS: u64 = 2 * 60 * 60;
pub(super) const RESOLUTION_SECONDS: u64 = 60 * 60;
pub(super) const OPENING_SECONDS: u64 = 60 * 60;
const ACCELERATED_TOTAL_SECONDS: u64 =
    PLACEMENT_SECONDS + CHALLENGE_SECONDS + RESOLUTION_SECONDS + OPENING_SECONDS;

const RAMX_ROOT: [u8; 32] = [
    0x52, 0xa5, 0x74, 0xe7, 0xfe, 0xe1, 0x2f, 0x99, 0x21, 0xb3, 0xb2, 0x20, 0xb7, 0x09, 0xbe, 0x16,
    0xc1, 0x3a, 0x6f, 0x29, 0x0a, 0xbb, 0xdc, 0x2c, 0xc2, 0x03, 0xed, 0x11, 0x91, 0xec, 0xf5, 0x78,
];
const NANDX_ROOT: [u8; 32] = [
    0x41, 0xbb, 0x5d, 0xc7, 0x9b, 0xac, 0xab, 0xb3, 0xe0, 0x87, 0x6b, 0x55, 0xe6, 0x47, 0xb0, 0x10,
    0x84, 0xc7, 0xfe, 0x8b, 0x39, 0x84, 0x6f, 0xd7, 0x07, 0xee, 0x0f, 0xd9, 0x73, 0xd2, 0x8c, 0x3d,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Product {
    Ramx,
    Nandx,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Cohort {
    index: u8,
    product: Product,
    month: u32,
    expiry_ts: u64,
    required_sku_count: u16,
    required_sku_root: [u8; 32],
}

impl Cohort {
    fn from_index(index: u8) -> Result<Self, ProgramError> {
        match index {
            0 => Ok(Self {
                index,
                product: Product::Ramx,
                month: 202609,
                expiry_ts: 1_790_812_800,
                required_sku_count: RAMX_ORACLE_PRODUCT_SKU_COUNT,
                required_sku_root: RAMX_ROOT,
            }),
            1 => Ok(Self {
                index,
                product: Product::Nandx,
                month: 202609,
                expiry_ts: 1_790_812_800,
                required_sku_count: NANDX_ORACLE_PRODUCT_SKU_COUNT,
                required_sku_root: NANDX_ROOT,
            }),
            2 => Ok(Self {
                index,
                product: Product::Ramx,
                month: 202610,
                expiry_ts: 1_793_491_200,
                required_sku_count: RAMX_ORACLE_PRODUCT_SKU_COUNT,
                required_sku_root: RAMX_ROOT,
            }),
            3 => Ok(Self {
                index,
                product: Product::Nandx,
                month: 202610,
                expiry_ts: 1_793_491_200,
                required_sku_count: NANDX_ORACLE_PRODUCT_SKU_COUNT,
                required_sku_root: NANDX_ROOT,
            }),
            _ => Err(VaultError::InvalidInstructionData.into()),
        }
    }

    fn bit(self) -> u8 {
        1 << self.index
    }

    fn underlying_id(self) -> [u8; 32] {
        match self.product {
            Product::Ramx => fixed_text_32(b"ram-standardized-baskets"),
            Product::Nandx => fixed_text_32(b"nand-standardized-baskets"),
        }
    }

    fn product_symbol(self) -> &'static [u8] {
        match self.product {
            Product::Ramx => b"RAMX",
            Product::Nandx => b"NANDX",
        }
    }

    fn market_id(self, kind: OptionKind) -> [u8; 32] {
        let mut value = [0u8; 32];
        let side = match kind {
            OptionKind::CallSpread => b"CALL".as_slice(),
            OptionKind::PutSpread => b"PUT".as_slice(),
        };
        let month = match self.month {
            202609 => b"202609".as_slice(),
            202610 => b"202610".as_slice(),
            _ => unreachable!(),
        };
        let mut offset = 0usize;
        for part in [self.product_symbol(), b"-", month, b"-", side, b"-01"] {
            value[offset..offset + part.len()].copy_from_slice(part);
            offset += part.len();
        }
        value
    }
}

#[inline(always)]
const fn fixed_text_32(value: &[u8]) -> [u8; 32] {
    let mut output = [0u8; 32];
    let mut index = 0usize;
    while index < value.len() {
        output[index] = value[index];
        index += 1;
    }
    output
}

pub(super) fn is_instruction_tag(tag: u8) -> bool {
    is_devnet_solo_backfill_2026_instruction_tag(tag)
}

pub(super) fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    tag: u8,
    payload: &[u8],
) -> ProgramResult {
    if *program_id != EXPECTED_PROGRAM_ID || *program_id != crate::id() {
        return Err(VaultError::Unauthorized.into());
    }
    match tag {
        INITIALIZE_SIDECAR_TAG => {
            expect_payload_len(payload, 0)?;
            process_initialize_sidecar(program_id, accounts)
        }
        INITIALIZE_COHORT_TAG => {
            expect_payload_len(payload, 1)?;
            process_initialize_cohort(program_id, accounts, Cohort::from_index(payload[0])?)
        }
        CREATE_SUPPORTED_SOURCE_TAG => {
            let parsed = parse_source_payload(payload)?;
            process_create_supported_source(program_id, accounts, parsed)
        }
        FINALIZE_COVERAGE_TAG => {
            let (cohort, kind) = parse_cohort_kind_payload(payload)?;
            process_finalize_coverage(program_id, accounts, cohort, kind)
        }
        ACCUMULATE_RECIPE_TAG => {
            let parsed = parse_item_payload(payload)?;
            process_accumulate_recipe(program_id, accounts, parsed)
        }
        FINALIZE_RECIPE_TAG => {
            let (cohort, kind) = parse_cohort_kind_payload(payload)?;
            process_finalize_recipe(program_id, accounts, cohort, kind)
        }
        SUBMIT_OPENING_TAG => {
            let parsed = parse_item_payload(payload)?;
            process_submit_opening(program_id, accounts, parsed)
        }
        FINALIZE_OPENING_TAG => {
            let parsed = parse_item_payload(payload)?;
            process_finalize_opening(program_id, accounts, parsed)
        }
        BEGIN_ACTIVE_WEIGHTS_TAG => {
            let (cohort, kind) = parse_cohort_kind_payload(payload)?;
            process_begin_active_weights(program_id, accounts, cohort, kind)
        }
        ACCUMULATE_ACTIVE_WEIGHT_TAG => {
            let parsed = parse_item_payload(payload)?;
            process_accumulate_active_weight(program_id, accounts, parsed)
        }
        FINALIZE_ACTIVE_WEIGHTS_TAG => {
            let (cohort, kind) = parse_cohort_kind_payload(payload)?;
            process_finalize_active_weights(program_id, accounts, cohort, kind)
        }
        FINALIZE_GAME_TAG => {
            let (cohort, kind) = parse_cohort_kind_payload(payload)?;
            process_finalize_game(program_id, accounts, cohort, kind)
        }
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

fn expect_payload_len(payload: &[u8], expected: usize) -> ProgramResult {
    if payload.len() != expected {
        return Err(VaultError::InvalidInstructionData.into());
    }
    Ok(())
}

fn option_kind(value: u8) -> Result<OptionKind, ProgramError> {
    match value {
        0 => Ok(OptionKind::CallSpread),
        1 => Ok(OptionKind::PutSpread),
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

fn parse_cohort_kind_payload(payload: &[u8]) -> Result<(Cohort, OptionKind), ProgramError> {
    expect_payload_len(payload, 2)?;
    Ok((Cohort::from_index(payload[0])?, option_kind(payload[1])?))
}

struct SourcePayload {
    cohort: Cohort,
    kind: OptionKind,
    sku_index: u16,
    sku_id: [u8; 32],
    sku_proof: [[u8; 32]; 6],
}

#[derive(Clone, Copy)]
struct ItemPayload {
    cohort: Cohort,
    kind: OptionKind,
    sku_index: u16,
    sku_id: [u8; 32],
}

fn parse_item_payload(payload: &[u8]) -> Result<ItemPayload, ProgramError> {
    expect_payload_len(payload, 36)?;
    let cohort = Cohort::from_index(payload[0])?;
    let kind = option_kind(payload[1])?;
    let sku_index = u16::from_le_bytes([payload[2], payload[3]]);
    let mut sku_id = [0u8; 32];
    sku_id.copy_from_slice(&payload[4..]);
    if sku_index >= cohort.required_sku_count || crate::bytes32_is_zero(&sku_id) {
        return Err(VaultError::InvalidOracleSkuMembershipProof.into());
    }
    Ok(ItemPayload {
        cohort,
        kind,
        sku_index,
        sku_id,
    })
}

fn parse_source_payload(payload: &[u8]) -> Result<SourcePayload, ProgramError> {
    expect_payload_len(payload, 228)?;
    let cohort = Cohort::from_index(payload[0])?;
    let kind = option_kind(payload[1])?;
    let sku_index = u16::from_le_bytes([payload[2], payload[3]]);
    if sku_index >= cohort.required_sku_count {
        return Err(VaultError::InvalidOracleSkuMembershipProof.into());
    }
    let mut sku_id = [0u8; 32];
    sku_id.copy_from_slice(&payload[4..36]);
    let mut sku_proof = [[0u8; 32]; 6];
    for (index, node) in sku_proof.iter_mut().enumerate() {
        node.copy_from_slice(&payload[36 + index * 32..36 + (index + 1) * 32]);
    }
    Ok(SourcePayload {
        cohort,
        kind,
        sku_index,
        sku_id,
        sku_proof,
    })
}

fn load_sidecar(
    program_id: &Pubkey,
    sidecar_info: &AccountInfo,
) -> Result<DevnetSoloBackfill2026V1, ProgramError> {
    let (expected, bump) = derive_devnet_solo_backfill_2026_v1_pda(program_id);
    if *sidecar_info.key != expected
        || sidecar_info.owner != program_id
        || sidecar_info.data_len() != DevnetSoloBackfill2026V1::LEN
    {
        return Err(VaultError::InvalidPda.into());
    }
    let data = sidecar_info.try_borrow_data()?;
    let sidecar = DevnetSoloBackfill2026V1::try_from_slice(data.as_ref())
        .map_err(|_| ProgramError::from(VaultError::InvalidOracleState))?;
    if !sidecar.is_initialized
        || sidecar.bump != bump
        || !sidecar.has_canonical_layout()
        || sidecar.vault_config != derive_vault_config_pda(program_id).0
        || sidecar.admin != EXPECTED_ADMIN
        || sidecar.oracle_authority != EXPECTED_ORACLE_AUTHORITY
        || sidecar.sunset_ts != SUNSET_TS
    {
        return Err(VaultError::InvalidOracleState.into());
    }
    Ok(sidecar)
}

fn store_sidecar(sidecar_info: &AccountInfo, sidecar: &DevnetSoloBackfill2026V1) -> ProgramResult {
    let encoded = borsh::to_vec(sidecar).map_err(|_| ProgramError::InvalidAccountData)?;
    if encoded.len() != DevnetSoloBackfill2026V1::LEN
        || sidecar_info.data_len() != DevnetSoloBackfill2026V1::LEN
    {
        return Err(ProgramError::AccountDataTooSmall);
    }
    sidecar_info
        .try_borrow_mut_data()?
        .copy_from_slice(&encoded);
    Ok(())
}

fn exact_config(
    program_id: &Pubkey,
    config_info: &AccountInfo,
) -> Result<VaultConfig, ProgramError> {
    if *config_info.key != derive_vault_config_pda(program_id).0 {
        return Err(VaultError::InvalidConfigAccount.into());
    }
    let config = load_current_canonical_vault_config(program_id, config_info)?;
    if config.admin != EXPECTED_ADMIN
        || config.oracle_authority != EXPECTED_ORACLE_AUTHORITY
        || config.usdc_mint != EXPECTED_COLLATERAL_MINT
    {
        return Err(VaultError::Unauthorized.into());
    }
    Ok(config)
}

fn current_clock() -> Result<(u64, u64), ProgramError> {
    let clock = Clock::get()?;
    let timestamp = u64::try_from(clock.unix_timestamp)
        .map_err(|_| ProgramError::from(VaultError::InvalidOracleState))?;
    Ok((clock.slot, timestamp))
}

fn validate_operator_and_sidecar(
    program_id: &Pubkey,
    operator_info: &AccountInfo,
    config_info: &AccountInfo,
    sidecar_info: &AccountInfo,
) -> Result<(VaultConfig, DevnetSoloBackfill2026V1, u64, u64), ProgramError> {
    if !operator_info.is_signer || *operator_info.key != EXPECTED_ORACLE_AUTHORITY {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = exact_config(program_id, config_info)?;
    if config.oracle_authority != *operator_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let sidecar = load_sidecar(program_id, sidecar_info)?;
    let (slot, now) = current_clock()?;
    if now >= sidecar.sunset_ts {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    Ok((config, sidecar, slot, now))
}

fn process_initialize_sidecar(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    if accounts.len() != 5 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let oracle_info = &accounts[1];
    let config_info = &accounts[2];
    let sidecar_info = &accounts[3];
    let system_program_info = &accounts[4];
    if !admin_info.is_signer
        || !oracle_info.is_signer
        || *admin_info.key != EXPECTED_ADMIN
        || *oracle_info.key != EXPECTED_ORACLE_AUTHORITY
    {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_system_program(system_program_info)?;
    let config = exact_config(program_id, config_info)?;
    if config.admin != *admin_info.key || config.oracle_authority != *oracle_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let (expected, bump) = derive_devnet_solo_backfill_2026_v1_pda(program_id);
    if *sidecar_info.key != expected {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, sidecar_info)?;
    let (slot, now) = current_clock()?;
    if now >= SUNSET_TS
        || now
            .checked_add(ACCELERATED_TOTAL_SECONDS)
            .ok_or(VaultError::ArithmeticOverflow)?
            >= SUNSET_TS
    {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    create_program_account(
        admin_info,
        sidecar_info,
        system_program_info,
        program_id,
        DevnetSoloBackfill2026V1::LEN,
        &[DEVNET_SOLO_BACKFILL_2026_V1_PDA_SEED, &[bump]],
    )?;
    store_sidecar(
        sidecar_info,
        &DevnetSoloBackfill2026V1 {
            is_initialized: true,
            bump,
            account_discriminator: DevnetSoloBackfill2026V1::ACCOUNT_DISCRIMINATOR,
            account_version: DevnetSoloBackfill2026V1::ACCOUNT_VERSION,
            vault_config: *config_info.key,
            admin: *admin_info.key,
            oracle_authority: *oracle_info.key,
            initialized_slot: slot,
            initialized_at_ts: now,
            sunset_ts: SUNSET_TS,
            consumed_cohort_mask: 0,
            completed_cohort_mask: 0,
            game_market_mask: 0,
            reserved: 0,
            cohort_start_ts: [0; 4],
            last_updated_slot: slot,
        },
    )
}

fn validate_exact_market(
    program_id: &Pubkey,
    market_info: &AccountInfo,
    cohort: Cohort,
    kind: OptionKind,
) -> Result<Market, ProgramError> {
    let market_id = cohort.market_id(kind);
    let (expected, _) = derive_market_pda(program_id, &market_id);
    if *market_info.key != expected {
        return Err(VaultError::InvalidPda.into());
    }
    let market = load_valid_market(program_id, market_info)?;
    let expected_cap = match kind {
        OptionKind::CallSpread => 112_000_000,
        OptionKind::PutSpread => 88_000_000,
    };
    let exact_instrument = InstrumentDefinition {
        underlying_id: cohort.underlying_id(),
        expiry_ts: cohort.expiry_ts,
        strike_price: 100_000_000,
        cap_price: expected_cap,
        contract_size: 1_000_000,
        max_payout_per_contract: 12_000_000,
        kind,
        settlement: SettlementStyle::CashSettledMonthly,
    };
    let exact_params = MarketParameters {
        tick_size: 50_000,
        lot_size: 1,
        min_order_qty: 1,
        maker_fee_bps: 0,
        taker_fee_bps: 20,
        cancel_fee_bps: 0,
        min_cancel_slots: 32,
        max_fills_per_instruction: 8,
    };
    if !market.is_initialized
        || market.created_by != EXPECTED_ADMIN
        || market.market_id != market_id
        || market.collateral_mint != EXPECTED_COLLATERAL_MINT
        || market.long_contract_mint.is_none()
        || market.instrument != exact_instrument
        || market.params != exact_params
        || market.total_position_collateral_locked != 0
        || !market.paused
        || market.mint_accounting != MarketMintAccounting::canonical_empty()
    {
        return Err(VaultError::InvalidOracleState.into());
    }
    Ok(market)
}

#[allow(clippy::too_many_arguments)]
fn create_month_bundle<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    market_info: &AccountInfo<'a>,
    month_info: &AccountInfo<'a>,
    coverage_info: &AccountInfo<'a>,
    schedule_info: &AccountInfo<'a>,
    reward_vault_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    economics: &OracleEconomicParams,
    cohort: Cohort,
    start_ts: u64,
    slot: u64,
) -> ProgramResult {
    let market = load_valid_market(program_id, market_info)?;
    let (expected_month, month_bump) =
        derive_oracle_month_pda(program_id, market_info.key, market.instrument.expiry_ts);
    if *month_info.key != expected_month {
        return Err(VaultError::InvalidOracleMonthAccount.into());
    }
    validate_create_only_program_account_target(program_id, month_info)?;
    let (expected_coverage, coverage_bump) =
        derive_oracle_sku_coverage_manifest_pda(program_id, month_info.key);
    if *coverage_info.key != expected_coverage {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    validate_create_only_program_account_target(program_id, coverage_info)?;
    let (expected_schedule, schedule_bump) =
        derive_oracle_usdc_reward_schedule_pda(program_id, month_info.key);
    if *schedule_info.key != expected_schedule {
        return Err(VaultError::InvalidOracleUsdcRewardSchedule.into());
    }
    validate_create_only_program_account_target(program_id, schedule_info)?;
    let expiry_seed = market.instrument.expiry_ts.to_le_bytes();
    create_program_account(
        payer_info,
        month_info,
        system_program_info,
        program_id,
        OracleMonthState::LEN,
        &[
            ORACLE_MONTH_PDA_SEED,
            market_info.key.as_ref(),
            &expiry_seed,
            &[month_bump],
        ],
    )?;
    create_program_account(
        payer_info,
        coverage_info,
        system_program_info,
        program_id,
        OracleSkuCoverageManifest::LEN,
        &[
            crate::constants::ORACLE_SKU_COVERAGE_MANIFEST_PDA_SEED,
            month_info.key.as_ref(),
            &[coverage_bump],
        ],
    )?;
    create_program_account(
        payer_info,
        schedule_info,
        system_program_info,
        program_id,
        OracleUsdcRewardSchedule::LEN,
        &[
            crate::constants::ORACLE_USDC_REWARD_SCHEDULE_PDA_SEED,
            month_info.key.as_ref(),
            &[schedule_bump],
        ],
    )?;
    let listing_ts = start_ts
        .checked_add(ACCELERATED_TOTAL_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let month = OracleMonthState {
        is_initialized: true,
        bump: month_bump,
        account_discriminator: OracleMonthState::ACCOUNT_DISCRIMINATOR,
        account_version: OracleMonthState::ACCOUNT_VERSION,
        market: *market_info.key,
        authority: EXPECTED_ORACLE_AUTHORITY,
        scramble_start_ts: start_ts,
        listing_ts,
        phase: OraclePhase::SourceSubmission,
        economics: economics.clone(),
        last_updated_slot: slot,
        settlement_base_oracle_atomic: 100_000_000,
        ..OracleMonthState::default()
    };
    let coverage = OracleSkuCoverageManifest {
        is_initialized: true,
        bump: coverage_bump,
        account_discriminator: OracleSkuCoverageManifest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSkuCoverageManifest::ACCOUNT_VERSION,
        month: *month_info.key,
        required_sku_root: cohort.required_sku_root,
        required_sku_count: cohort.required_sku_count,
        covered_sku_count: 0,
        planned_scramble_start_ts: start_ts,
        planned_listing_ts: listing_ts,
        coverage_finalized: false,
        coverage_complete_ts: 0,
        last_updated_slot: slot,
    };
    let schedule = OracleUsdcRewardSchedule {
        is_initialized: true,
        bump: schedule_bump,
        account_discriminator: OracleUsdcRewardSchedule::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUsdcRewardSchedule::ACCOUNT_VERSION,
        month: *month_info.key,
        authority: EXPECTED_ORACLE_AUTHORITY,
        reward_vault: *reward_vault_info.key,
        phase: OracleUsdcRewardSchedulePhase::Funded,
        sku_pool_count: 0,
        registered_source_count: 0,
        registered_opening_count: 0,
        registered_update_count: 0,
        registered_update_reward_units: 0,
        total_reward_budget: 0,
        remaining_reward_budget: 0,
        last_updated_slot: slot,
        outstanding_prelisting_escrow_count: 0,
        trading_fee_bounty_total: 0,
        bounty_fee_sweep_finalized: false,
    };
    store_state(month_info, &month)?;
    store_state(coverage_info, &coverage)?;
    store_state(schedule_info, &schedule)
}

fn process_initialize_cohort(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    cohort: Cohort,
) -> ProgramResult {
    if accounts.len() != 15 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let operator_info = &accounts[0];
    let sidecar_info = &accounts[1];
    let config_info = &accounts[2];
    let economics_info = &accounts[3];
    let product_manifest_info = &accounts[4];
    let reward_vault_info = &accounts[5];
    let call_market_info = &accounts[6];
    let call_month_info = &accounts[7];
    let call_coverage_info = &accounts[8];
    let call_schedule_info = &accounts[9];
    let put_market_info = &accounts[10];
    let put_month_info = &accounts[11];
    let put_coverage_info = &accounts[12];
    let put_schedule_info = &accounts[13];
    let system_program_info = &accounts[14];
    validate_system_program(system_program_info)?;
    let (_config, mut sidecar, slot, now) =
        validate_operator_and_sidecar(program_id, operator_info, config_info, sidecar_info)?;
    if sidecar.consumed_cohort_mask & cohort.bit() != 0
        || sidecar.cohort_start_ts[usize::from(cohort.index)] != 0
        || now
            .checked_add(ACCELERATED_TOTAL_SECONDS)
            .ok_or(VaultError::ArithmeticOverflow)?
            >= cohort.expiry_ts
        || now + ACCELERATED_TOTAL_SECONDS >= sidecar.sunset_ts
    {
        return Err(VaultError::AlreadyInitialized.into());
    }
    validate_exact_market(program_id, call_market_info, cohort, OptionKind::CallSpread)?;
    validate_exact_market(program_id, put_market_info, cohort, OptionKind::PutSpread)?;
    let expected_manifest =
        derive_oracle_product_sku_manifest_pda(program_id, &cohort.underlying_id()).0;
    if *product_manifest_info.key != expected_manifest {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    let product_manifest = load_valid_oracle_product_sku_manifest(
        program_id,
        &cohort.underlying_id(),
        product_manifest_info,
    )?;
    if product_manifest.required_sku_root != cohort.required_sku_root
        || product_manifest.required_sku_count != cohort.required_sku_count
    {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    let economics = load_canonical_oracle_economics_config(program_id, economics_info)?;
    let expected_reward_vault = derive_oracle_usdc_reward_vault_pda(program_id).0;
    if *reward_vault_info.key != expected_reward_vault {
        return Err(VaultError::InvalidOracleUsdcRewardVault.into());
    }
    let reward_vault = oracle_usdc::load_oracle_usdc_reward_vault(program_id, reward_vault_info)?;
    if reward_vault.mint != EXPECTED_COLLATERAL_MINT {
        return Err(VaultError::InvalidOracleUsdcRewardVault.into());
    }
    create_month_bundle(
        program_id,
        operator_info,
        call_market_info,
        call_month_info,
        call_coverage_info,
        call_schedule_info,
        reward_vault_info,
        system_program_info,
        &economics.economics,
        cohort,
        now,
        slot,
    )?;
    create_month_bundle(
        program_id,
        operator_info,
        put_market_info,
        put_month_info,
        put_coverage_info,
        put_schedule_info,
        reward_vault_info,
        system_program_info,
        &economics.economics,
        cohort,
        now,
        slot,
    )?;
    sidecar.consumed_cohort_mask |= cohort.bit();
    sidecar.cohort_start_ts[usize::from(cohort.index)] = now;
    sidecar.last_updated_slot = slot;
    store_sidecar(sidecar_info, &sidecar)
}

const LISTING_BOND: u64 = 12_000_000;
const SUPPORT_BOND: u64 = 12_000_000;
const OPENING_BOND: u64 = 1_000_000;

// Preserve the observed, already-repaired old Devnet demo profile. Its writer
// groups anchor to the call month (one call plus one put, $24 gross exposure).
// The put-month manifests retained their original $1 cap. These fixed fixture
// values apply only to this exact Devnet cohort allowlist, before finalization;
// ordinary oracle security-budget calculation and writer risk rules are unchanged.
fn demo_exposure_cap(kind: OptionKind) -> u64 {
    match kind {
        OptionKind::CallSpread => 24_000_000,
        OptionKind::PutSpread => 1_000_000,
    }
}

#[derive(Clone, Copy)]
struct AcceleratedBoundaries {
    start: u64,
    placement_end: u64,
    challenge_end: u64,
    resolution_end: u64,
    opening_end: u64,
}

fn accelerated_boundaries(
    sidecar: &DevnetSoloBackfill2026V1,
    cohort: Cohort,
) -> Result<AcceleratedBoundaries, ProgramError> {
    if sidecar.consumed_cohort_mask & cohort.bit() == 0 {
        return Err(VaultError::InvalidOracleState.into());
    }
    let start = sidecar.cohort_start_ts[usize::from(cohort.index)];
    let placement_end = start
        .checked_add(PLACEMENT_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let challenge_end = placement_end
        .checked_add(CHALLENGE_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let resolution_end = challenge_end
        .checked_add(RESOLUTION_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let opening_end = resolution_end
        .checked_add(OPENING_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if start == 0 || opening_end >= cohort.expiry_ts || opening_end >= sidecar.sunset_ts {
        return Err(VaultError::InvalidOracleState.into());
    }
    Ok(AcceleratedBoundaries {
        start,
        placement_end,
        challenge_end,
        resolution_end,
        opening_end,
    })
}

fn validate_special_month(
    program_id: &Pubkey,
    market_info: &AccountInfo,
    month_info: &AccountInfo,
    sidecar: &DevnetSoloBackfill2026V1,
    cohort: Cohort,
    kind: OptionKind,
) -> Result<(Market, OracleMonthState, AcceleratedBoundaries), ProgramError> {
    let market = validate_exact_market(program_id, market_info, cohort, kind)?;
    let month = load_valid_oracle_month(program_id, market_info, month_info, &market)?;
    let boundaries = accelerated_boundaries(sidecar, cohort)?;
    if month.authority != EXPECTED_ORACLE_AUTHORITY
        || month.scramble_start_ts != boundaries.start
        || month.listing_ts != boundaries.opening_end
        || month.settlement_base_oracle_atomic != 100_000_000
        || !month.has_current_layout()
    {
        return Err(VaultError::InvalidOracleState.into());
    }
    Ok((market, month, boundaries))
}

fn deterministic_source_hash(domain: &'static [u8], month: &Pubkey, sku_id: &[u8; 32]) -> [u8; 32] {
    hashv(&[
        b"amoeba-devnet-solo-backfill-2026-v1",
        domain,
        month.as_ref(),
        sku_id,
    ])
    .to_bytes()
}

fn expected_source_id(month: &Pubkey, sku_id: &[u8; 32]) -> [u8; 32] {
    deterministic_source_hash(b"source-id", month, sku_id)
}

fn process_create_supported_source(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SourcePayload,
) -> ProgramResult {
    if accounts.len() != 15 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let operator_info = &accounts[0];
    let sidecar_info = &accounts[1];
    let config_info = &accounts[2];
    let market_info = &accounts[3];
    let month_info = &accounts[4];
    let coverage_info = &accounts[5];
    let schedule_info = &accounts[6];
    let sku_info = &accounts[7];
    let source_info = &accounts[8];
    let observations_info = &accounts[9];
    let source_reward_info = &accounts[10];
    let collateral_info = &accounts[11];
    let support_info = &accounts[12];
    let coverage_record_info = &accounts[13];
    let system_program_info = &accounts[14];
    validate_system_program(system_program_info)?;
    let (_config, sidecar, slot, now) =
        validate_operator_and_sidecar(program_id, operator_info, config_info, sidecar_info)?;
    let (_market, mut month, boundaries) = validate_special_month(
        program_id,
        market_info,
        month_info,
        &sidecar,
        params.cohort,
        params.kind,
    )?;
    if month.phase != OraclePhase::SourceSubmission
        || now < boundaries.start
        || now >= boundaries.placement_end
        || month.pending_resolution_count != 0
        || month.weight_scheme_version != 0
    {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    let mut coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    if coverage.coverage_finalized
        || coverage.required_sku_count != params.cohort.required_sku_count
        || coverage.required_sku_root != params.cohort.required_sku_root
        || coverage.planned_scramble_start_ts != boundaries.start
        || coverage.planned_listing_ts != boundaries.opening_end
    {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    verify_oracle_sku_membership(
        &coverage,
        &params.sku_id,
        params.sku_index,
        &params.sku_proof,
    )?;
    let mut schedule =
        oracle_usdc::load_oracle_usdc_reward_schedule(program_id, month_info.key, schedule_info)?;
    if schedule.authority != EXPECTED_ORACLE_AUTHORITY
        || schedule.phase != OracleUsdcRewardSchedulePhase::Funded
        || schedule.total_reward_budget != 0
        || schedule.remaining_reward_budget != 0
    {
        return Err(VaultError::InvalidOracleUsdcRewardSchedule.into());
    }

    let (expected_sku, sku_bump) =
        derive_oracle_usdc_sku_pool_pda(program_id, schedule_info.key, &params.sku_id);
    let source_id = expected_source_id(month_info.key, &params.sku_id);
    let (expected_source, source_bump) =
        derive_oracle_source_pda(program_id, month_info.key, &source_id);
    let (expected_observations, observations_bump) =
        derive_oracle_source_observations_pda(program_id, &expected_source);
    let (expected_source_reward, source_reward_bump) =
        derive_oracle_usdc_source_reward_pda(program_id, schedule_info.key, &expected_source);
    let (expected_support, support_bump) = derive_oracle_support_pda(
        program_id,
        month_info.key,
        &expected_source,
        operator_info.key,
    );
    let (expected_record, record_bump) =
        derive_oracle_sku_coverage_record_pda(program_id, month_info.key, &params.sku_id);
    if *sku_info.key != expected_sku
        || *source_info.key != expected_source
        || *observations_info.key != expected_observations
        || *source_reward_info.key != expected_source_reward
        || *support_info.key != expected_support
        || *coverage_record_info.key != expected_record
    {
        return Err(VaultError::InvalidPda.into());
    }
    for target in [
        sku_info,
        source_info,
        observations_info,
        source_reward_info,
        support_info,
        coverage_record_info,
    ] {
        validate_create_only_program_account_target(program_id, target)?;
    }
    let mut collateral =
        load_canonical_user_collateral(program_id, collateral_info, operator_info.key)?;
    oracle_usdc::debit_oracle_usdc_available(&mut collateral, LISTING_BOND)?;
    oracle_usdc::debit_oracle_usdc_available(&mut collateral, SUPPORT_BOND)?;

    create_program_account(
        operator_info,
        sku_info,
        system_program_info,
        program_id,
        OracleUsdcSkuPool::LEN,
        &[
            crate::constants::ORACLE_USDC_SKU_POOL_PDA_SEED,
            schedule_info.key.as_ref(),
            &params.sku_id,
            &[sku_bump],
        ],
    )?;
    create_program_account(
        operator_info,
        source_info,
        system_program_info,
        program_id,
        OracleSourceState::LEN,
        &[
            ORACLE_SOURCE_PDA_SEED,
            month_info.key.as_ref(),
            &source_id,
            &[source_bump],
        ],
    )?;
    create_program_account(
        operator_info,
        observations_info,
        system_program_info,
        program_id,
        OracleSourceObservations::LEN,
        &[
            ORACLE_SOURCE_OBSERVATIONS_PDA_SEED,
            source_info.key.as_ref(),
            &[observations_bump],
        ],
    )?;
    create_program_account(
        operator_info,
        source_reward_info,
        system_program_info,
        program_id,
        OracleUsdcSourceReward::LEN,
        &[
            crate::constants::ORACLE_USDC_SOURCE_REWARD_PDA_SEED,
            schedule_info.key.as_ref(),
            source_info.key.as_ref(),
            &[source_reward_bump],
        ],
    )?;
    create_program_account(
        operator_info,
        support_info,
        system_program_info,
        program_id,
        OracleSupportPosition::LEN,
        &[
            ORACLE_SUPPORT_POSITION_PDA_SEED,
            month_info.key.as_ref(),
            source_info.key.as_ref(),
            operator_info.key.as_ref(),
            &[support_bump],
        ],
    )?;
    create_program_account(
        operator_info,
        coverage_record_info,
        system_program_info,
        program_id,
        OracleSkuCoverageRecord::LEN,
        &[
            crate::constants::ORACLE_SKU_COVERAGE_RECORD_PDA_SEED,
            month_info.key.as_ref(),
            &params.sku_id,
            &[record_bump],
        ],
    )?;

    let sku = OracleUsdcSkuPool {
        is_initialized: true,
        bump: sku_bump,
        account_discriminator: OracleUsdcSkuPool::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUsdcSkuPool::ACCOUNT_VERSION,
        schedule: *schedule_info.key,
        month: *month_info.key,
        bucket_id: params.sku_id,
        source_reward_budget: 0,
        remaining_source_reward_budget: 0,
        opening_reward_budget: 0,
        remaining_opening_reward_budget: 0,
        update_reward_budget: 0,
        remaining_update_reward_budget: 0,
        proposer_reward_bps: 5_000,
        listing_bond: LISTING_BOND,
        support_bond: SUPPORT_BOND,
        opening_bond: OPENING_BOND,
        update_min_bond: OPENING_BOND,
        challenge_min_bond: OPENING_BOND,
        challenge_max_bond: LISTING_BOND,
        challenge_bond_bps: 5_000,
        registered_source_count: 0,
        registered_opening_count: 0,
        registered_update_count: 0,
        registered_update_reward_units: 0,
        last_updated_slot: slot,
    };
    let source = OracleSourceState {
        is_initialized: true,
        bump: source_bump,
        month: *month_info.key,
        source_id,
        bucket_id: params.sku_id,
        source_type_hash: deterministic_source_hash(b"source-type", month_info.key, &params.sku_id),
        canonical_locator_hash: deterministic_source_hash(
            b"canonical-locator",
            month_info.key,
            &params.sku_id,
        ),
        source_definition_hash: deterministic_source_hash(
            b"source-definition",
            month_info.key,
            &params.sku_id,
        ),
        proposer: *operator_info.key,
        listing_bond_locked: LISTING_BOND,
        support_stake_total: SUPPORT_BOND,
        status: OracleSourceStatus::Candidate,
        ..OracleSourceState::default()
    };
    let observations = OracleSourceObservations {
        is_initialized: true,
        bump: observations_bump,
        account_discriminator: OracleSourceObservations::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSourceObservations::ACCOUNT_VERSION,
        month: *month_info.key,
        source: *source_info.key,
        states: [0; crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS],
        source_times: [0; crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS],
    };
    let source_reward = OracleUsdcSourceReward {
        is_initialized: true,
        bump: source_reward_bump,
        account_discriminator: OracleUsdcSourceReward::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUsdcSourceReward::ACCOUNT_VERSION,
        month: *month_info.key,
        schedule: *schedule_info.key,
        sku_pool: *sku_info.key,
        source: *source_info.key,
        source_id,
        proposer: *operator_info.key,
        supporter_count: 1,
        registered: false,
        terminal_status: OracleSourceStatus::Candidate,
        opening_claim: Pubkey::default(),
        last_updated_slot: slot,
        merged_into_source: Pubkey::default(),
        max_merge_depth: 0,
        listing_escrow_counted: false,
    };
    let support = OracleSupportPosition {
        is_initialized: true,
        bump: support_bump,
        month: *month_info.key,
        supporter: *operator_info.key,
        source: *source_info.key,
        source_id,
        support_stake: SUPPORT_BOND,
        released: false,
        escrow_disposition: OracleEscrowDisposition::Unsettled,
        failed_schedule_escrow_counted: false,
    };
    let coverage_record = OracleSkuCoverageRecord {
        is_initialized: true,
        bump: record_bump,
        account_discriminator: OracleSkuCoverageRecord::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSkuCoverageRecord::ACCOUNT_VERSION,
        month: *month_info.key,
        sku_id: params.sku_id,
        sku_index: params.sku_index,
        active_supported_source_count: 1,
        last_updated_slot: slot,
    };
    month.source_count = month
        .source_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    coverage.covered_sku_count = coverage
        .covered_sku_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if month.source_count > params.cohort.required_sku_count
        || coverage.covered_sku_count > coverage.required_sku_count
    {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    schedule.sku_pool_count = schedule
        .sku_pool_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    schedule.outstanding_prelisting_escrow_count = schedule
        .outstanding_prelisting_escrow_count
        .checked_add(2)
        .ok_or(VaultError::ArithmeticOverflow)?;
    schedule.last_updated_slot = slot;
    month.last_updated_slot = slot;
    coverage.last_updated_slot = slot;
    store_state(sku_info, &sku)?;
    store_state(source_info, &source)?;
    store_state(observations_info, &observations)?;
    store_state(source_reward_info, &source_reward)?;
    store_state(support_info, &support)?;
    store_state(coverage_record_info, &coverage_record)?;
    store_state(collateral_info, &collateral)?;
    store_state(schedule_info, &schedule)?;
    store_state(coverage_info, &coverage)?;
    store_oracle_month_state(month_info, &month)
}

fn process_finalize_coverage(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    cohort: Cohort,
    kind: OptionKind,
) -> ProgramResult {
    if accounts.len() != 6 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let operator_info = &accounts[0];
    let sidecar_info = &accounts[1];
    let config_info = &accounts[2];
    let market_info = &accounts[3];
    let month_info = &accounts[4];
    let coverage_info = &accounts[5];
    let (_config, sidecar, slot, now) =
        validate_operator_and_sidecar(program_id, operator_info, config_info, sidecar_info)?;
    let (_market, mut month, boundaries) =
        validate_special_month(program_id, market_info, month_info, &sidecar, cohort, kind)?;
    let mut coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    if month.phase != OraclePhase::SourceSubmission
        || now < boundaries.start
        || now >= boundaries.placement_end
        || coverage.coverage_finalized
        || coverage.covered_sku_count != cohort.required_sku_count
        || coverage.required_sku_count != cohort.required_sku_count
        || month.source_count != cohort.required_sku_count
        || month.pending_resolution_count != 0
        || month.weight_scheme_version != 0
    {
        return Err(VaultError::OracleSkuCoverageIncomplete.into());
    }
    coverage.coverage_finalized = true;
    coverage.coverage_complete_ts = now;
    coverage.last_updated_slot = slot;
    month.phase = OraclePhase::Scramble;
    month.last_updated_slot = slot;
    store_state(coverage_info, &coverage)?;
    store_oracle_month_state(month_info, &month)
}

fn program_computed_bucket_weight(sku_index: u16, sku_count: u16) -> Result<u16, ProgramError> {
    if sku_count == 0 || sku_index >= sku_count {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    let base = 10_000u16 / sku_count;
    let remainder = 10_000u16 % sku_count;
    base.checked_add(u16::from(sku_index < remainder))
        .ok_or_else(|| VaultError::ArithmeticOverflow.into())
}

fn process_accumulate_recipe(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ItemPayload,
) -> ProgramResult {
    if accounts.len() != 10 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let operator_info = &accounts[0];
    let sidecar_info = &accounts[1];
    let config_info = &accounts[2];
    let market_info = &accounts[3];
    let month_info = &accounts[4];
    let coverage_info = &accounts[5];
    let coverage_record_info = &accounts[6];
    let source_info = &accounts[7];
    let manifest_info = &accounts[8];
    let system_program_info = &accounts[9];
    validate_system_program(system_program_info)?;
    let (_config, sidecar, slot, now) =
        validate_operator_and_sidecar(program_id, operator_info, config_info, sidecar_info)?;
    let (_market, mut month, boundaries) = validate_special_month(
        program_id,
        market_info,
        month_info,
        &sidecar,
        params.cohort,
        params.kind,
    )?;
    if month.phase != OraclePhase::Scramble
        || now < boundaries.challenge_end
        || now >= boundaries.resolution_end
        || month.pending_resolution_count != 0
        || month.source_count != params.cohort.required_sku_count
        || !matches!(month.weight_scheme_version, 0 | 255)
    {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    let coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    if !coverage.coverage_finalized
        || coverage.coverage_complete_ts < boundaries.start
        || coverage.coverage_complete_ts >= boundaries.placement_end
        || coverage.covered_sku_count != params.cohort.required_sku_count
        || coverage.required_sku_count != params.cohort.required_sku_count
    {
        return Err(VaultError::OracleSkuCoverageIncomplete.into());
    }
    let record = load_valid_oracle_sku_coverage_record(
        program_id,
        month_info.key,
        &params.sku_id,
        coverage_record_info,
    )?;
    if record.sku_index != params.sku_index || record.active_supported_source_count != 1 {
        return Err(VaultError::InvalidOracleSkuCoverageRecord.into());
    }
    let source_id = expected_source_id(month_info.key, &params.sku_id);
    let expected_source = derive_oracle_source_pda(program_id, month_info.key, &source_id).0;
    if *source_info.key != expected_source || !source_info.is_writable {
        return Err(VaultError::InvalidOracleSourceAccount.into());
    }
    let mut source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    if source.source_id != source_id
        || source.bucket_id != params.sku_id
        || source.proposer != EXPECTED_ORACLE_AUTHORITY
        || source.status != OracleSourceStatus::Candidate
        || source.listing_bond_locked != LISTING_BOND
        || source.support_stake_total != SUPPORT_BOND
        || source.bucket_weight_bps != 0
    {
        return Err(VaultError::InvalidOracleWeightSource.into());
    }
    let (expected_manifest, manifest_bump) =
        derive_oracle_recipe_weight_manifest_pda(program_id, month_info.key);
    if *manifest_info.key != expected_manifest {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    let mut manifest = if params.sku_index == 0 {
        if month.weight_scheme_version != 0 {
            return Err(VaultError::InvalidOracleWeightManifest.into());
        }
        validate_create_only_program_account_target(program_id, manifest_info)?;
        create_oracle_manifest_account(
            operator_info,
            manifest_info,
            system_program_info,
            program_id,
            month_info.key.as_ref(),
            OracleRecipeWeightManifest::LEN,
            ORACLE_RECIPE_WEIGHT_MANIFEST_PDA_SEED,
            manifest_bump,
        )?;
        month.weight_scheme_version = 255;
        month.effective_weight_total_bps = 0;
        month.weight_manifest_hash = [0; 32];
        OracleRecipeWeightManifest {
            is_initialized: true,
            bump: manifest_bump,
            account_discriminator: OracleRecipeWeightManifest::ACCOUNT_DISCRIMINATOR,
            account_version: OracleRecipeWeightManifest::ACCOUNT_VERSION,
            month: *month_info.key,
            phase: OracleRecipeWeightPhase::Collecting,
            expected_source_count: params.cohort.required_sku_count,
            expected_bucket_count: params.cohort.required_sku_count,
            rolling_manifest_hash: initial_oracle_weight_manifest_hash(
                month_info.key,
                params.cohort.required_sku_count,
                params.cohort.required_sku_count,
            ),
            ..OracleRecipeWeightManifest::default()
        }
    } else {
        load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, manifest_info)?
    };
    if manifest.phase != OracleRecipeWeightPhase::Collecting
        || manifest.processed_source_count != params.sku_index
        || manifest.processed_bucket_count != params.sku_index
        || manifest.expected_source_count != params.cohort.required_sku_count
        || manifest.expected_bucket_count != params.cohort.required_sku_count
        || manifest.current_bucket_source_count != 0
        || manifest.current_bucket_weight_bps != 0
        || !crate::bytes32_is_zero(&manifest.last_collected_source_id)
    {
        return Err(VaultError::InvalidOracleWeightOrder.into());
    }
    let weight =
        program_computed_bucket_weight(params.sku_index, params.cohort.required_sku_count)?;
    manifest.current_bucket_id = params.sku_id;
    manifest.current_bucket_weight_bps = weight;
    manifest.current_bucket_source_count = 1;
    manifest.declared_weight_total_bps = manifest
        .declared_weight_total_bps
        .checked_add(weight)
        .ok_or(VaultError::ArithmeticOverflow)?;
    manifest.rolling_manifest_hash = advance_oracle_weight_manifest_hash(
        &manifest.rolling_manifest_hash,
        &params.sku_id,
        &source,
        weight,
    );
    manifest.last_collected_source_id = source.source_id;
    source.bucket_weight_bps = weight;
    source.status = OracleSourceStatus::Frozen;
    source.baseline_state = 0;
    source.current_state = 0;
    manifest.processed_source_count = manifest
        .processed_source_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    manifest.processed_bucket_count = manifest
        .processed_bucket_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    manifest.phase = if manifest.processed_bucket_count == manifest.expected_bucket_count {
        OracleRecipeWeightPhase::ReadyToFinalize
    } else {
        OracleRecipeWeightPhase::Collecting
    };
    manifest.current_bucket_weight_bps = 0;
    manifest.current_bucket_source_count = 0;
    manifest.last_collected_source_id = [0; 32];
    month.last_updated_slot = slot;
    store_state(source_info, &source)?;
    store_state(manifest_info, &manifest)?;
    store_oracle_month_state(month_info, &month)
}

fn process_finalize_recipe(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    cohort: Cohort,
    kind: OptionKind,
) -> ProgramResult {
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let operator_info = &accounts[0];
    let sidecar_info = &accounts[1];
    let config_info = &accounts[2];
    let market_info = &accounts[3];
    let month_info = &accounts[4];
    let coverage_info = &accounts[5];
    let manifest_info = &accounts[6];
    let (_config, sidecar, slot, now) =
        validate_operator_and_sidecar(program_id, operator_info, config_info, sidecar_info)?;
    let (_market, mut month, boundaries) =
        validate_special_month(program_id, market_info, month_info, &sidecar, cohort, kind)?;
    if month.phase != OraclePhase::Scramble
        || now < boundaries.challenge_end
        || now >= boundaries.resolution_end
        || month.weight_scheme_version != 255
        || month.pending_resolution_count != 0
    {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    let coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    if !coverage.coverage_finalized
        || coverage.covered_sku_count != cohort.required_sku_count
        || coverage.coverage_complete_ts == 0
        || coverage.coverage_complete_ts >= boundaries.placement_end
    {
        return Err(VaultError::OracleSkuCoverageIncomplete.into());
    }
    let mut manifest =
        load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, manifest_info)?;
    if manifest.phase != OracleRecipeWeightPhase::ReadyToFinalize
        || manifest.expected_source_count != cohort.required_sku_count
        || manifest.expected_bucket_count != cohort.required_sku_count
        || manifest.processed_source_count != manifest.expected_source_count
        || manifest.processed_bucket_count != manifest.expected_bucket_count
        || manifest.declared_weight_total_bps != 10_000
        || crate::bytes32_is_zero(&manifest.rolling_manifest_hash)
    {
        return Err(VaultError::OracleWeightManifestIncomplete.into());
    }
    let recipe_hash = canonical_recipe_digest(month_info.key, &manifest.rolling_manifest_hash);
    manifest.recipe_hash = recipe_hash;
    manifest.phase = OracleRecipeWeightPhase::Finalized;
    month.recipe_hash = recipe_hash;
    month.source_count = manifest.expected_source_count;
    month.frozen_source_count = manifest.expected_source_count;
    month.opened_source_count = 0;
    month.opening_resolved_source_count = 0;
    month.pending_resolution_count = 0;
    month.index_delta_bps = 0;
    month.phase = OraclePhase::Opening;
    month.weight_scheme_version = 1;
    month.effective_weight_total_bps = manifest.declared_weight_total_bps;
    month.weight_manifest_hash = manifest.rolling_manifest_hash;
    month.last_updated_slot = slot;
    store_state(manifest_info, &manifest)?;
    store_oracle_month_state(month_info, &month)
}

fn deterministic_opening_state(source_id: &[u8; 32]) -> u64 {
    let signed_offset = i64::from(source_id[0]) - 128;
    u64::try_from(100_000_000i64 + signed_offset * 10_000).unwrap_or(100_000_000)
}

fn process_submit_opening(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ItemPayload,
) -> ProgramResult {
    if accounts.len() != 11 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let operator_info = &accounts[0];
    let sidecar_info = &accounts[1];
    let config_info = &accounts[2];
    let market_info = &accounts[3];
    let month_info = &accounts[4];
    let coverage_record_info = &accounts[5];
    let sku_info = &accounts[6];
    let source_info = &accounts[7];
    let collateral_info = &accounts[8];
    let claim_info = &accounts[9];
    let system_program_info = &accounts[10];
    validate_system_program(system_program_info)?;
    let (_config, sidecar, slot, now) =
        validate_operator_and_sidecar(program_id, operator_info, config_info, sidecar_info)?;
    let (_market, mut month, boundaries) = validate_special_month(
        program_id,
        market_info,
        month_info,
        &sidecar,
        params.cohort,
        params.kind,
    )?;
    if month.phase != OraclePhase::Opening
        || now < boundaries.resolution_end
        || now >= boundaries.opening_end
        || month.weight_scheme_version != 1
        || month.effective_weight_total_bps != 10_000
        || crate::bytes32_is_zero(&month.recipe_hash)
        || crate::bytes32_is_zero(&month.weight_manifest_hash)
    {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    let record = load_valid_oracle_sku_coverage_record(
        program_id,
        month_info.key,
        &params.sku_id,
        coverage_record_info,
    )?;
    if record.sku_index != params.sku_index || record.active_supported_source_count != 1 {
        return Err(VaultError::InvalidOracleSkuCoverageRecord.into());
    }
    let source_id = expected_source_id(month_info.key, &params.sku_id);
    let expected_source = derive_oracle_source_pda(program_id, month_info.key, &source_id).0;
    if *source_info.key != expected_source {
        return Err(VaultError::InvalidOracleSourceAccount.into());
    }
    let mut source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let expected_weight =
        program_computed_bucket_weight(params.sku_index, params.cohort.required_sku_count)?;
    if source.source_id != source_id
        || source.bucket_id != params.sku_id
        || source.status != OracleSourceStatus::Frozen
        || source.opening_submitted
        || source.bucket_weight_bps != expected_weight
        || source.support_stake_total != SUPPORT_BOND
        || source.listing_bond_locked != LISTING_BOND
    {
        return Err(VaultError::InvalidOracleWeightSource.into());
    }
    let sku = oracle_usdc::load_oracle_usdc_sku_for_month(program_id, month_info.key, sku_info)?;
    if sku.bucket_id != params.sku_id
        || sku.listing_bond != LISTING_BOND
        || sku.support_bond != SUPPORT_BOND
        || sku.opening_bond != OPENING_BOND
    {
        return Err(VaultError::InvalidOracleUsdcSkuPool.into());
    }
    let (expected_claim, claim_bump) =
        derive_oracle_opening_claim_pda(program_id, month_info.key, source_info.key);
    if *claim_info.key != expected_claim {
        return Err(VaultError::InvalidOracleOpeningClaim.into());
    }
    validate_create_only_program_account_target(program_id, claim_info)?;
    let mut collateral =
        load_canonical_user_collateral(program_id, collateral_info, operator_info.key)?;
    oracle_usdc::debit_oracle_usdc_available(&mut collateral, OPENING_BOND)?;
    create_program_account(
        operator_info,
        claim_info,
        system_program_info,
        program_id,
        OracleOpeningClaim::LEN,
        &[
            ORACLE_OPENING_CLAIM_PDA_SEED,
            month_info.key.as_ref(),
            source_info.key.as_ref(),
            &[claim_bump],
        ],
    )?;
    let opening_state = deterministic_opening_state(&source_id);
    let evidence_hash = hashv(&[
        b"amoeba-devnet-solo-backfill-2026-v1/opening-evidence",
        month_info.key.as_ref(),
        source_info.key.as_ref(),
        &source_id,
        &opening_state.to_le_bytes(),
        &now.to_le_bytes(),
        &source.canonical_locator_hash,
        &source.source_definition_hash,
    ])
    .to_bytes();
    let archive_url_hash = hashv(&[
        b"amoeba-devnet-solo-backfill-2026-v1/evidence-receipt",
        month_info.key.as_ref(),
        &source_id,
        &now.to_le_bytes(),
    ])
    .to_bytes();
    let claim = OracleOpeningClaim {
        is_initialized: true,
        bump: claim_bump,
        month: *month_info.key,
        source: *source_info.key,
        source_id,
        attempt: 1,
        claimant: *operator_info.key,
        opening_state,
        source_time: now,
        stake: OPENING_BOND,
        canonical_locator_hash: source.canonical_locator_hash,
        source_definition_hash: source.source_definition_hash,
        evidence_hash,
        archive_url_hash,
        submitted_slot: slot,
        challenge_deadline_slot: slot
            .checked_add(ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS)
            .ok_or(VaultError::ArithmeticOverflow)?,
        status: OracleOpeningClaimStatus::Pending,
        escrow_disposition: OracleEscrowDisposition::Unsettled,
    };
    source.status = OracleSourceStatus::OpeningPending;
    month.pending_resolution_count = month
        .pending_resolution_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    month.last_updated_slot = slot;
    store_state(source_info, &source)?;
    store_state(claim_info, &claim)?;
    store_state(collateral_info, &collateral)?;
    store_oracle_month_state(month_info, &month)
}

fn process_finalize_opening(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ItemPayload,
) -> ProgramResult {
    if accounts.len() != 13 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let operator_info = &accounts[0];
    let sidecar_info = &accounts[1];
    let config_info = &accounts[2];
    let market_info = &accounts[3];
    let month_info = &accounts[4];
    let coverage_record_info = &accounts[5];
    let source_info = &accounts[6];
    let observations_info = &accounts[7];
    let claim_info = &accounts[8];
    let collateral_info = &accounts[9];
    let (_config, sidecar, slot, now) =
        validate_operator_and_sidecar(program_id, operator_info, config_info, sidecar_info)?;
    let (_market, mut month, boundaries) = validate_special_month(
        program_id,
        market_info,
        month_info,
        &sidecar,
        params.cohort,
        params.kind,
    )?;
    if month.phase != OraclePhase::Opening
        || now < boundaries.resolution_end
        || now >= boundaries.opening_end
    {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    let record = load_valid_oracle_sku_coverage_record(
        program_id,
        month_info.key,
        &params.sku_id,
        coverage_record_info,
    )?;
    if record.sku_index != params.sku_index || record.active_supported_source_count != 1 {
        return Err(VaultError::InvalidOracleSkuCoverageRecord.into());
    }
    let source_id = expected_source_id(month_info.key, &params.sku_id);
    if *source_info.key != derive_oracle_source_pda(program_id, month_info.key, &source_id).0 {
        return Err(VaultError::InvalidOracleSourceAccount.into());
    }
    let mut source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let mut observations = load_valid_oracle_source_observations(
        program_id,
        month_info.key,
        source_info.key,
        observations_info,
    )?;
    let mut claim = load_valid_oracle_opening_claim(
        program_id,
        month_info.key,
        source_info.key,
        claim_info,
        &source,
    )?;
    let _collateral =
        load_canonical_user_collateral(program_id, collateral_info, operator_info.key)?;
    if source.source_id != source_id
        || source.status != OracleSourceStatus::OpeningPending
        || source.opening_submitted
        || claim.claimant != EXPECTED_ORACLE_AUTHORITY
        || claim.status != OracleOpeningClaimStatus::Pending
        || claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || claim.challenge_deadline_slot == 0
        || slot < claim.challenge_deadline_slot
        || claim.opening_state != deterministic_opening_state(&source_id)
        || claim.source_time < boundaries.resolution_end
        || claim.source_time > now
        || crate::bytes32_is_zero(&claim.evidence_hash)
        || crate::bytes32_is_zero(&claim.archive_url_hash)
        || claim.canonical_locator_hash != source.canonical_locator_hash
        || claim.source_definition_hash != source.source_definition_hash
    {
        return Err(VaultError::OracleOpeningClaimNotFinalizable.into());
    }
    let source_before = source.clone();
    append_oracle_source_observation(
        &mut source,
        &mut observations,
        claim.opening_state,
        claim.source_time,
        &claim.evidence_hash,
        &claim.archive_url_hash,
    )?;
    source.baseline_state = claim.opening_state;
    source.current_state = claim.opening_state;
    source.opening_submitted = true;
    source.opening_evidence_hash = claim.evidence_hash;
    source.status = OracleSourceStatus::Active;
    claim.status = OracleOpeningClaimStatus::Accepted;
    month.opened_source_count = month
        .opened_source_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    month.opening_resolved_source_count = month
        .opening_resolved_source_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    decrement_pending_oracle_resolution(&mut month)?;
    month.last_updated_slot = slot;
    crate::processor::oracle_carry::record_fresh_accept(
        program_id,
        operator_info,
        source_info.key,
        &source_before,
        &source,
        &observations,
        crate::processor::oracle_carry::AcceptedEvent {
            event: *claim_info.key,
            value: claim.opening_state,
            observed_at: claim.source_time,
            evidence_hash: claim.evidence_hash,
            archive_hash: claim.archive_url_hash,
            contributor: claim.claimant,
        },
        &accounts[10..],
    )?;
    store_state(source_info, &source)?;
    store_state(observations_info, &observations)?;
    store_state(claim_info, &claim)?;
    store_oracle_month_state(month_info, &month)
}

fn process_begin_active_weights(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    cohort: Cohort,
    kind: OptionKind,
) -> ProgramResult {
    if accounts.len() != 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let operator_info = &accounts[0];
    let sidecar_info = &accounts[1];
    let config_info = &accounts[2];
    let market_info = &accounts[3];
    let month_info = &accounts[4];
    let recipe_info = &accounts[5];
    let active_info = &accounts[6];
    let system_program_info = &accounts[7];
    validate_system_program(system_program_info)?;
    let (_config, sidecar, slot, now) =
        validate_operator_and_sidecar(program_id, operator_info, config_info, sidecar_info)?;
    let (_market, mut month, boundaries) =
        validate_special_month(program_id, market_info, month_info, &sidecar, cohort, kind)?;
    if month.phase != OraclePhase::Opening
        || now < boundaries.resolution_end
        || now >= boundaries.opening_end
        || month.pending_resolution_count != 0
        || month.opened_source_count != cohort.required_sku_count
        || month.opening_resolved_source_count != cohort.required_sku_count
        || month.active_weight_scheme_version != 0
        || month.active_weight_group_count != 0
        || !crate::bytes32_is_zero(&month.active_weight_manifest_hash)
    {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    ensure_oracle_opening_sources_terminal(&month)?;
    let recipe = load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, recipe_info)?;
    validate_oracle_active_manifest_begin_membership(
        &month,
        &recipe,
        cohort.required_sku_count,
        cohort.required_sku_count,
    )?;
    let (expected_active, active_bump) =
        derive_oracle_active_weight_manifest_pda(program_id, month_info.key);
    if *active_info.key != expected_active {
        return Err(VaultError::InvalidOracleActiveWeightManifest.into());
    }
    validate_create_only_program_account_target(program_id, active_info)?;
    create_oracle_manifest_account(
        operator_info,
        active_info,
        system_program_info,
        program_id,
        month_info.key.as_ref(),
        OracleActiveWeightManifest::LEN,
        ORACLE_ACTIVE_WEIGHT_MANIFEST_PDA_SEED,
        active_bump,
    )?;
    let manifest = OracleActiveWeightManifest {
        is_initialized: true,
        bump: active_bump,
        account_discriminator: OracleActiveWeightManifest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleActiveWeightManifest::ACCOUNT_VERSION,
        month: *month_info.key,
        phase: OracleRecipeWeightPhase::Collecting,
        expected_source_count: cohort.required_sku_count,
        expected_group_count: cohort.required_sku_count,
        rolling_manifest_hash: initial_oracle_active_weight_hash(
            month_info.key,
            &month.recipe_hash,
            &month.weight_manifest_hash,
            cohort.required_sku_count,
            cohort.required_sku_count,
        ),
        max_open_interest_payout: u64::MAX,
        ..OracleActiveWeightManifest::default()
    };
    month.active_weight_scheme_version = 255;
    month.active_weight_group_count = 0;
    month.active_weight_manifest_hash = [0; 32];
    month.last_updated_slot = slot;
    store_state(active_info, &manifest)?;
    store_oracle_month_state(month_info, &month)
}

fn process_accumulate_active_weight(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ItemPayload,
) -> ProgramResult {
    if accounts.len() != 12 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let operator_info = &accounts[0];
    let sidecar_info = &accounts[1];
    let config_info = &accounts[2];
    let market_info = &accounts[3];
    let month_info = &accounts[4];
    let active_info = &accounts[5];
    let coverage_record_info = &accounts[6];
    let source_info = &accounts[7];
    let observations_info = &accounts[8];
    let sku_info = &accounts[9];
    let bucket_info = &accounts[10];
    let system_program_info = &accounts[11];
    validate_system_program(system_program_info)?;
    let (_config, sidecar, slot, now) =
        validate_operator_and_sidecar(program_id, operator_info, config_info, sidecar_info)?;
    let (_market, mut month, boundaries) = validate_special_month(
        program_id,
        market_info,
        month_info,
        &sidecar,
        params.cohort,
        params.kind,
    )?;
    if month.phase != OraclePhase::Opening
        || now < boundaries.resolution_end
        || now >= boundaries.opening_end
        || month.active_weight_scheme_version != 255
        || month.pending_resolution_count != 0
    {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    let record = load_valid_oracle_sku_coverage_record(
        program_id,
        month_info.key,
        &params.sku_id,
        coverage_record_info,
    )?;
    if record.sku_index != params.sku_index || record.active_supported_source_count != 1 {
        return Err(VaultError::InvalidOracleSkuCoverageRecord.into());
    }
    let source_id = expected_source_id(month_info.key, &params.sku_id);
    if *source_info.key != derive_oracle_source_pda(program_id, month_info.key, &source_id).0 {
        return Err(VaultError::InvalidOracleSourceAccount.into());
    }
    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let observations = load_valid_oracle_source_observations(
        program_id,
        month_info.key,
        source_info.key,
        observations_info,
    )?;
    validate_oracle_observation_shape(&source, &observations)?;
    let expected_weight =
        program_computed_bucket_weight(params.sku_index, params.cohort.required_sku_count)?;
    if source.source_id != source_id
        || source.bucket_id != params.sku_id
        || source.status != OracleSourceStatus::Active
        || !source.opening_submitted
        || source.observation_count != 1
        || source.bucket_weight_bps != expected_weight
        || source.listing_bond_locked != LISTING_BOND
        || source.support_stake_total != SUPPORT_BOND
    {
        return Err(VaultError::InvalidOracleWeightSource.into());
    }
    let sku = oracle_usdc::load_oracle_usdc_sku_for_month(program_id, month_info.key, sku_info)?;
    if sku.bucket_id != params.sku_id
        || sku.listing_bond != LISTING_BOND
        || sku.support_bond != SUPPORT_BOND
    {
        return Err(VaultError::InvalidOracleUsdcSkuPool.into());
    }
    let mut manifest =
        load_valid_oracle_active_weight_manifest(program_id, month_info.key, active_info)?;
    if manifest.phase != OracleRecipeWeightPhase::Collecting
        || manifest.processed_source_count != params.sku_index
        || manifest.processed_group_count != params.sku_index
        || manifest.expected_source_count != params.cohort.required_sku_count
        || manifest.expected_group_count != params.cohort.required_sku_count
        || manifest.current_group_source_count != 0
        || manifest.current_group_active_count != 0
        || manifest.current_group_bucket_weight_bps != 0
    {
        return Err(VaultError::InvalidOracleWeightOrder.into());
    }
    let (expected_bucket, bucket_bump) =
        derive_oracle_bucket_median_pda(program_id, month_info.key, &params.sku_id);
    if *bucket_info.key != expected_bucket {
        return Err(VaultError::InvalidOracleMedian.into());
    }
    validate_create_only_program_account_target(program_id, bucket_info)?;
    create_program_account(
        operator_info,
        bucket_info,
        system_program_info,
        program_id,
        OracleBucketMedianState::LEN,
        &[
            ORACLE_BUCKET_MEDIAN_PDA_SEED,
            month_info.key.as_ref(),
            &params.sku_id,
            &[bucket_bump],
        ],
    )?;
    let delta_bps = source_delta_bps(source.baseline_state, source.current_state)?;
    let (_, funded_security_cap) = oracle_bucket_security_cap(
        1,
        LISTING_BOND,
        SUPPORT_BOND,
        crate::constants::ORACLE_OI_CAP_KAPPA_BPS,
    )?;
    if funded_security_cap != LISTING_BOND {
        return Err(VaultError::InvalidOracleSecurityBudget.into());
    }
    let security_cap = demo_exposure_cap(params.kind);
    let bucket = OracleBucketMedianState {
        is_initialized: true,
        bump: bucket_bump,
        account_discriminator: OracleBucketMedianState::ACCOUNT_DISCRIMINATOR,
        account_version: OracleBucketMedianState::ACCOUNT_VERSION,
        month: *month_info.key,
        bucket_id: params.sku_id,
        group_index: params.sku_index,
        bucket_weight_start_bps: manifest.processed_bucket_weight_bps,
        bucket_weight_bps: expected_weight,
        frozen_source_count: 1,
        active_source_count: 1,
        eligible_source_count: 1,
        status: OracleBucketMedianStatus::Live,
        bucket_delta_bps: delta_bps,
        last_recomputed_ts: now,
        source_snapshot_hash: [0; 32],
        recompute_processed_source_count: 0,
        last_recompute_source_id: [0; 32],
        emergency_snapshot_slot: 0,
        emergency_snapshot_total_samba: 0,
        opening_source_deltas_bps: {
            let mut values = [0; crate::constants::MAX_ORACLE_BUCKET_SOURCES];
            values[0] = delta_bps;
            values
        },
    };
    manifest.current_group_id = params.sku_id;
    manifest.current_group_source_count = 1;
    manifest.current_group_active_count = 1;
    manifest.current_group_bucket_weight_bps = expected_weight;
    manifest.last_collected_source_id = source.source_id;
    manifest.rolling_manifest_hash =
        advance_oracle_active_manifest_hash(&manifest.rolling_manifest_hash, &source);
    manifest.max_open_interest_payout = manifest.max_open_interest_payout.min(security_cap);
    manifest.processed_source_count = manifest
        .processed_source_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    manifest.processed_group_count = manifest
        .processed_group_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    manifest.processed_bucket_weight_bps = manifest
        .processed_bucket_weight_bps
        .checked_add(expected_weight)
        .ok_or(VaultError::ArithmeticOverflow)?;
    manifest.phase = if manifest.processed_group_count == manifest.expected_group_count {
        OracleRecipeWeightPhase::ReadyToFinalize
    } else {
        OracleRecipeWeightPhase::Collecting
    };
    manifest.current_group_source_count = 0;
    manifest.current_group_active_count = 0;
    manifest.current_group_bucket_weight_bps = 0;
    manifest.last_collected_source_id = [0; 32];
    month.index_delta_bps = month
        .index_delta_bps
        .checked_add(bucket_index_contribution_bps(expected_weight, delta_bps)?)
        .ok_or(VaultError::ArithmeticOverflow)?;
    month.active_weight_group_count = month
        .active_weight_group_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    month.last_updated_slot = slot;
    store_state(bucket_info, &bucket)?;
    store_state(active_info, &manifest)?;
    store_oracle_month_state(month_info, &month)
}

fn process_finalize_active_weights(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    cohort: Cohort,
    kind: OptionKind,
) -> ProgramResult {
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let operator_info = &accounts[0];
    let sidecar_info = &accounts[1];
    let config_info = &accounts[2];
    let market_info = &accounts[3];
    let month_info = &accounts[4];
    let recipe_info = &accounts[5];
    let active_info = &accounts[6];
    let (_config, sidecar, slot, now) =
        validate_operator_and_sidecar(program_id, operator_info, config_info, sidecar_info)?;
    let (_market, mut month, boundaries) =
        validate_special_month(program_id, market_info, month_info, &sidecar, cohort, kind)?;
    if month.phase != OraclePhase::Opening
        || now < boundaries.resolution_end
        || now >= boundaries.opening_end
        || month.active_weight_scheme_version != 255
    {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    let recipe = load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, recipe_info)?;
    let mut manifest =
        load_valid_oracle_active_weight_manifest(program_id, month_info.key, active_info)?;
    validate_oracle_active_manifest_completion(&month, &recipe, &manifest)?;
    if manifest.expected_source_count != cohort.required_sku_count
        || manifest.expected_group_count != cohort.required_sku_count
        || manifest.max_open_interest_payout != demo_exposure_cap(kind)
    {
        return Err(VaultError::OracleWeightManifestIncomplete.into());
    }
    manifest.phase = OracleRecipeWeightPhase::Finalized;
    month.active_weight_scheme_version = OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION;
    month.active_weight_manifest_hash = manifest.rolling_manifest_hash;
    month.last_updated_slot = slot;
    store_state(active_info, &manifest)?;
    store_oracle_month_state(month_info, &month)
}

fn game_market_bit(cohort: Cohort, kind: OptionKind) -> u8 {
    let offset = cohort.index * 2 + u8::from(kind == OptionKind::PutSpread);
    1 << offset
}

fn process_finalize_game(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    cohort: Cohort,
    kind: OptionKind,
) -> ProgramResult {
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let operator_info = &accounts[0];
    let sidecar_info = &accounts[1];
    let config_info = &accounts[2];
    let market_info = &accounts[3];
    let month_info = &accounts[4];
    let coverage_info = &accounts[5];
    let active_info = &accounts[6];
    let (config, mut sidecar, slot, now) =
        validate_operator_and_sidecar(program_id, operator_info, config_info, sidecar_info)?;
    if config.paused {
        return Err(VaultError::ContractPaused.into());
    }
    let (mut market, mut month, boundaries) =
        validate_special_month(program_id, market_info, month_info, &sidecar, cohort, kind)?;
    let bit = game_market_bit(cohort, kind);
    if sidecar.game_market_mask & bit != 0
        || month.phase != OraclePhase::Opening
        || now < boundaries.opening_end
        || now >= cohort.expiry_ts
    {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    let active = load_valid_oracle_active_weight_manifest(program_id, month_info.key, active_info)?;
    ensure_finalized_oracle_active_weight_manifest(&month, &active)?;
    let mut coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    if !coverage.coverage_finalized
        || coverage.covered_sku_count != cohort.required_sku_count
        || coverage.required_sku_count != cohort.required_sku_count
        || coverage.coverage_complete_ts < boundaries.start
        || coverage.coverage_complete_ts >= boundaries.placement_end
        || active.expected_group_count != cohort.required_sku_count
    {
        return Err(VaultError::OracleSkuCoverageIncomplete.into());
    }
    let canonical_scramble_start = boundaries
        .opening_end
        .checked_sub(ORACLE_PRE_LISTING_WINDOW_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    month.scramble_start_ts = canonical_scramble_start;
    month.listing_ts = boundaries.opening_end;
    coverage.planned_scramble_start_ts = canonical_scramble_start;
    coverage.planned_listing_ts = boundaries.opening_end;
    rulebook_schedule_boundaries(&month)?;
    ensure_finalized_oracle_issue_sku_coverage(&month, &coverage, &active)?;
    ensure_oracle_game_transition_window(&market, &month)?;
    if market_outstanding_contract_amount(&market)? != 0
        || !market.paused
        || market.mint_accounting != MarketMintAccounting::canonical_empty()
    {
        return Err(VaultError::InvalidMarketMintAccounting.into());
    }
    month.phase = OraclePhase::Game;
    month.last_updated_slot = slot;
    coverage.last_updated_slot = slot;
    market.paused = false;
    sidecar.game_market_mask |= bit;
    let cohort_market_bits = 0b11u8 << (cohort.index * 2);
    if sidecar.game_market_mask & cohort_market_bits == cohort_market_bits {
        sidecar.completed_cohort_mask |= cohort.bit();
    }
    sidecar.last_updated_slot = slot;
    store_state(market_info, &market)?;
    store_state(coverage_info, &coverage)?;
    store_oracle_month_state(month_info, &month)?;
    store_sidecar(sidecar_info, &sidecar)
}
