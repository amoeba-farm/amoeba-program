use super::*;
use crate::{
    instruction::{ClaimCollectiveLongV1Params, ClaimWriterFlatResidualV1Params},
    writer_sleeve_math::{cumulative_allocation_delta, settlement_series_liabilities},
};

const PUBLISH_WRITER_GROUP_SETTLEMENT_ACCOUNT_COUNT: usize = 13;
const FINALIZE_WRITER_SETTLEMENT_FIXED_ACCOUNT_COUNT: usize = 8;
const CLAIM_COLLECTIVE_LONG_ACCOUNT_COUNT: usize = 20;
const CLAIM_WRITER_FLAT_ACCOUNT_COUNT: usize = 17;
const CLOSE_WRITER_SLEEVE_ACCOUNT_COUNT: usize = 10;
const WRITER_GROUP_FINAL_SETTLEMENT_DOMAIN: &[u8] = b"ameba-writer-final-settlement-v1";

fn final_settlement_commitment(
    program_id: &Pubkey,
    group_info: &AccountInfo,
    group: &WriterSettlementGroupV1,
    settlement_info: &AccountInfo,
    settlement: &SettlementRecordV2,
) -> [u8; 32] {
    let group_commitment = writer_group_commitment(group_info.key, group);
    hashv(&[
        WRITER_GROUP_FINAL_SETTLEMENT_DOMAIN,
        program_id.as_ref(),
        group_info.key.as_ref(),
        &group_commitment,
        settlement_info.key.as_ref(),
        &settlement.signed_leaf_commitment,
        &settlement.settlement_ts.to_le_bytes(),
        &settlement.settlement_price_atomic.to_le_bytes(),
        &settlement.signer_set_version.to_le_bytes(),
        settlement.submitted_by.as_ref(),
    ])
    .to_bytes()
}

pub(super) fn process_publish_writer_group_settlement(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != PUBLISH_WRITER_GROUP_SETTLEMENT_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let submitter_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let anchor_market_info = &accounts[4];
    let month_info = &accounts[5];
    let settlement_info = &accounts[6];
    let coverage_info = &accounts[7];
    let recipe_info = &accounts[8];
    let settlement_source_info = &accounts[9];
    let active_weight_info = &accounts[10];
    let signer_registry_info = &accounts[11];
    let signer_set_info = &accounts[12];
    if !submitter_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    let mut group = load_writer_settlement_group(program_id, group_info)?;
    let mut sleeve = load_writer_sleeve(program_id, sleeve_info, group_info.key)?;
    if config.paused
        || sleeve.vault_config != *config_info.key
        || group.sleeve != *sleeve_info.key
        || group.status != WriterSettlementGroupStatus::Active
        || sleeve.status != WriterSleeveStatus::Active
        || sleeve.active_auction.is_some()
        || sleeve.active_close_request.is_some()
        || group.anchor_market != *anchor_market_info.key
        || group.anchor_oracle_month != *month_info.key
        || group.signer_registry != *signer_registry_info.key
        || group.signer_set != *signer_set_info.key
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let market = load_valid_market(program_id, anchor_market_info)?;
    if market.instrument.underlying_id != group.underlying_id
        || market.instrument.expiry_ts != group.expiry_ts
        || market.collateral_mint != group.settlement_mint
    {
        return Err(VaultError::InvalidWriterSettlementGroup.into());
    }
    let month = load_valid_oracle_month(program_id, anchor_market_info, month_info, &market)?;
    let settlement = load_valid_settlement_record_v2(
        program_id,
        anchor_market_info.key,
        &market,
        settlement_info,
    )?;
    let coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    let recipe = load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, recipe_info)?;
    let settlement_sources = load_valid_oracle_settlement_source_manifest(
        program_id,
        month_info.key,
        settlement_source_info,
    )?;
    let active =
        load_valid_oracle_active_weight_manifest(program_id, month_info.key, active_weight_info)?;
    let _registry = load_canonical_settlement_signer_registry(program_id, signer_registry_info)?;
    let signer_set = load_canonical_settlement_signer_set(
        program_id,
        signer_registry_info.key,
        signer_set_info,
    )?;
    if month.phase != OraclePhase::Settled
        || month.settlement_record != Some(*settlement_info.key)
        || settlement.settlement_ts != group.settlement_ts
        || settlement.signer_set_version != group.signer_set_version
        || signer_set.version != group.signer_set_version
        || signer_set.set_hash != group.signer_set_hash
        || writer_coverage_manifest_hash(&coverage) != group.coverage_manifest_hash
        || recipe.phase != OracleRecipeWeightPhase::Finalized
        || recipe.recipe_hash != group.recipe_hash
        || settlement_sources.phase != OracleRecipeWeightPhase::Finalized
        || settlement_sources.rolling_source_digest != group.settlement_source_digest
        || active.rolling_manifest_hash != group.active_weight_manifest_hash
        || active.max_open_interest_payout != group.security_cap_atoms
    {
        return Err(VaultError::InvalidWriterSettlementGroup.into());
    }
    let slot = Clock::get()?.slot;
    group.settlement_price_atomic = settlement.settlement_price_atomic;
    group.final_settlement_commitment =
        final_settlement_commitment(program_id, group_info, &group, settlement_info, &settlement);
    group.submitted_by = settlement.submitted_by;
    group.finalized_slot = slot;
    group.status = WriterSettlementGroupStatus::Settled;
    group.last_updated_slot = slot;
    sleeve.status = WriterSleeveStatus::Expired;
    sleeve.last_updated_slot = slot;
    store_state(group_info, &group)?;
    store_state(sleeve_info, &sleeve)
}

pub(super) fn process_finalize_writer_sleeve_settlement(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() < FINALIZE_WRITER_SETTLEMENT_FIXED_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let snapshot_info = &accounts[5];
    let sleeve_vault_info = &accounts[6];
    let flat_mint_info = &accounts[7];
    if !cranker_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    let WriterBookContext {
        group,
        mut sleeve,
        mut book,
    } = load_writer_book_context(program_id, sleeve_info, group_info, book_info)?;
    if accounts.len()
        != FINALIZE_WRITER_SETTLEMENT_FIXED_ACCOUNT_COUNT + usize::from(book.series_count)
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let snapshot = load_writer_policy_snapshot(
        program_id,
        snapshot_info,
        sleeve_info.key,
        &sleeve.policy_registry,
        sleeve.policy_version,
    )?;
    if config.paused
        || sleeve.vault_config != *config_info.key
        || sleeve.status != WriterSleeveStatus::Expired
        || group.status != WriterSettlementGroupStatus::Settled
        || crate::bytes32_is_zero(&group.final_settlement_commitment)
        || sleeve.active_auction.is_some()
        || sleeve.active_close_request.is_some()
        || sleeve.policy_snapshot != *snapshot_info.key
        || sleeve.policy_hash != snapshot.policy_hash
        || sleeve.usdc_vault != *sleeve_vault_info.key
        || sleeve.flat_mint != *flat_mint_info.key
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    validate_vault_token_account(sleeve_vault_info, &sleeve.settlement_mint, sleeve_info.key)?;
    if validate_token_account(sleeve_vault_info)?.amount < sleeve.accounted_asset_atoms {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let flat_mint = validate_writer_flat_mint(sleeve_info, &sleeve, flat_mint_info)?;
    if flat_mint.supply != sleeve.flat_par_supply_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let count = usize::from(book.series_count);
    for (index, mint_info) in accounts[FINALIZE_WRITER_SETTLEMENT_FIXED_ACCOUNT_COUNT..]
        .iter()
        .enumerate()
    {
        let record = &book.records[index];
        let expected_mint = derive_contract_mint_pda(program_id, &record.market).0;
        let mint = validate_mint_account(mint_info, &spl_token_program_id())?;
        if *mint_info.key != record.contract_mint
            || *mint_info.key != expected_mint
            || mint.decimals != MarketMintAccounting::CANONICAL_DECIMALS
            || mint.mint_authority != COption::Some(record.market)
            || mint.freeze_authority != COption::None
            || mint.supply != record.total_physical_supply_atoms
            || record.total_physical_supply_atoms != record.external_open_interest_atoms
            || record.issuer_controlled_atoms != 0
            || record.custody_status == WriterSeriesCustodyStatus::Open
            || record.settlement_status != WriterSeriesSettlementStatus::Open
            || record.settlement_external_oi_snapshot_atoms != 0
            || record.settlement_liability_initial_atoms != 0
            || record.settlement_liability_remaining_atoms != 0
        {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
    }
    let series = writer_book_math_series(&book)?;
    let (liabilities, long_total) =
        settlement_series_liabilities(&series, group.settlement_price_atomic)
            .map_err(writer_math_error)?;
    if long_total > sleeve.accounted_asset_atoms {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let mut flat_residual = sleeve
        .accounted_asset_atoms
        .checked_sub(long_total)
        .ok_or(VaultError::ArithmeticOverflow)?;
    for index in 0..count {
        let record = &mut book.records[index];
        record.settlement_external_oi_snapshot_atoms = record.external_open_interest_atoms;
        record.settlement_liability_initial_atoms = liabilities.values[index];
        record.settlement_liability_remaining_atoms = liabilities.values[index];
        record.settlement_status = if record.external_open_interest_atoms == 0 {
            WriterSeriesSettlementStatus::Exhausted
        } else {
            WriterSeriesSettlementStatus::Frozen
        };
    }
    sleeve.long_liability_initial_atoms = long_total;
    sleeve.long_liability_remaining_atoms = long_total;
    sleeve.flat_supply_snapshot_atoms = sleeve.flat_par_supply_atoms;
    sleeve.flat_claim_supply_remaining_atoms = sleeve.flat_par_supply_atoms;
    if sleeve.flat_par_supply_atoms == 0 {
        sleeve.accounted_asset_atoms = long_total;
        sleeve.stranded_surplus_atoms = sleeve
            .stranded_surplus_atoms
            .checked_add(flat_residual)
            .ok_or(VaultError::ArithmeticOverflow)?;
        flat_residual = 0;
    }
    sleeve.flat_residual_initial_atoms = flat_residual;
    sleeve.flat_residual_remaining_atoms = flat_residual;
    sleeve.exact_reserve_atoms = long_total;
    sleeve.lower_tail_reserve_atoms = 0;
    sleeve.upper_tail_reserve_atoms = 0;
    sleeve.security_exposure_atoms = 0;
    let slot = Clock::get()?.slot;
    sleeve.status = WriterSleeveStatus::SettlementFinalized;
    sleeve.settlement_finalized_slot = slot;
    sleeve.last_updated_slot = slot;
    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = slot;
    store_state(book_info, &book)?;
    store_state(sleeve_info, &sleeve)
}

pub(super) fn process_claim_collective_long(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ClaimCollectiveLongV1Params,
) -> ProgramResult {
    if accounts.len() != CLAIM_COLLECTIVE_LONG_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    if params.claim_atoms == 0 {
        return Err(VaultError::AmountMustBePositive.into());
    }
    let holder_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let market_info = &accounts[5];
    let contract_mint_info = &accounts[6];
    let claim_source_info = &accounts[7];
    let retirement_info = &accounts[8];
    let contract_interface_info = &accounts[9];
    let sleeve_vault_info = &accounts[10];
    let payout_destination_info = &accounts[11];
    let settlement_mint_info = &accounts[12];
    let usdc_interface_info = &accounts[13];
    let light_program_info = &accounts[14];
    let cpi_authority_info = &accounts[15];
    let token_program_info = &accounts[16];
    let system_program_info = &accounts[17];
    let compressible_config_info = &accounts[18];
    let rent_sponsor_info = &accounts[19];
    if !holder_info.is_signer || !holder_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_writer_compression_accounts(
        light_program_info,
        cpi_authority_info,
        token_program_info,
        system_program_info,
        compressible_config_info,
        rent_sponsor_info,
    )?;
    let config = load_canonical_vault_config(program_id, config_info)?;
    let WriterBookContext {
        group,
        mut sleeve,
        mut book,
    } = load_writer_book_context(program_id, sleeve_info, group_info, book_info)?;
    if config.paused
        || sleeve.vault_config != *config_info.key
        || sleeve.status != WriterSleeveStatus::SettlementFinalized
        || group.status != WriterSettlementGroupStatus::Settled
        || sleeve.active_auction.is_some()
        || sleeve.active_close_request.is_some()
        || sleeve.settlement_mint != *settlement_mint_info.key
        || sleeve.usdc_vault != *sleeve_vault_info.key
        || config.usdc_mint != *settlement_mint_info.key
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let index = book.records[..usize::from(book.series_count)]
        .iter()
        .position(|record| record.market == *market_info.key)
        .ok_or(VaultError::InvalidWriterSeriesBook)?;
    let stored = book.records[index];
    if stored.contract_mint != *contract_mint_info.key
        || stored.retirement_custody != *retirement_info.key
        || stored.settlement_status != WriterSeriesSettlementStatus::Frozen
        || params.claim_atoms > stored.external_open_interest_atoms
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mut market = load_valid_market(program_id, market_info)?;
    if market.market_id != stored.series_id
        || market.long_contract_mint != Some(*contract_mint_info.key)
        || market_outstanding_contract_amount(&market)? != stored.external_open_interest_atoms
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mint = validate_canonical_market_mint(market_info, &mut market, contract_mint_info, 0)?;
    if mint.supply != stored.total_physical_supply_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    validate_light_associated_token_account(
        holder_info.key,
        contract_mint_info.key,
        claim_source_info,
    )?;
    let source_before = load_canonical_light_token_account(
        claim_source_info,
        holder_info.key,
        contract_mint_info.key,
    )?;
    if source_before.amount < params.claim_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    validate_spl_interface_account(contract_mint_info.key, contract_interface_info)?;
    let retirement_before = load_or_create_writer_retirement_custody(
        program_id,
        holder_info,
        sleeve_info,
        market_info,
        retirement_info,
        contract_mint_info,
        token_program_info,
        system_program_info,
    )?
    .amount;
    let payout = cumulative_allocation_delta(
        stored.settlement_external_oi_snapshot_atoms,
        stored.external_open_interest_atoms,
        params.claim_atoms,
        stored.settlement_liability_initial_atoms,
    )
    .map_err(writer_math_error)?;
    if payout > stored.settlement_liability_remaining_atoms
        || payout > sleeve.long_liability_remaining_atoms
        || payout > sleeve.accounted_asset_atoms
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    validate_vault_token_account(sleeve_vault_info, settlement_mint_info.key, sleeve_info.key)?;
    let vault_before = validate_token_account(sleeve_vault_info)?.amount;
    if vault_before < sleeve.accounted_asset_atoms || vault_before < payout {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    validate_collateral_mint_account(settlement_mint_info, token_program_info.key)?;
    validate_spl_interface_account(settlement_mint_info.key, usdc_interface_info)?;
    let destination_before = load_or_create_light_associated_token_account(
        holder_info,
        holder_info,
        settlement_mint_info,
        payout_destination_info,
        light_program_info,
        compressible_config_info,
        rent_sponsor_info,
        system_program_info,
    )?;

    invoke_light_token_account_transfer(
        params.claim_atoms,
        MarketMintAccounting::CANONICAL_DECIMALS,
        light_program_info,
        cpi_authority_info,
        holder_info,
        claim_source_info,
        retirement_info,
        holder_info,
        contract_mint_info,
        contract_interface_info,
        token_program_info,
        system_program_info,
    )?;
    if payout != 0 {
        let bump = [sleeve.bump];
        let signer = writer_sleeve_signer_seeds(&sleeve.settlement_group, &bump);
        invoke_light_token_account_transfer_with_signer_seeds(
            payout,
            MarketMintAccounting::CANONICAL_DECIMALS,
            light_program_info,
            cpi_authority_info,
            holder_info,
            sleeve_vault_info,
            payout_destination_info,
            sleeve_info,
            settlement_mint_info,
            usdc_interface_info,
            token_program_info,
            system_program_info,
            &[&signer],
        )?;
    }
    let source_after = load_canonical_light_token_account(
        claim_source_info,
        holder_info.key,
        contract_mint_info.key,
    )?;
    let destination_after = load_canonical_light_token_account(
        payout_destination_info,
        holder_info.key,
        settlement_mint_info.key,
    )?;
    if source_before.amount.checked_sub(source_after.amount) != Some(params.claim_atoms)
        || validate_token_account(retirement_info)?
            .amount
            .checked_sub(retirement_before)
            != Some(params.claim_atoms)
        || destination_after
            .amount
            .checked_sub(destination_before.amount)
            != Some(payout)
        || vault_before.checked_sub(validate_token_account(sleeve_vault_info)?.amount)
            != Some(payout)
        || validate_mint_account(contract_mint_info, token_program_info.key)?.supply != mint.supply
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }

    let record = &mut book.records[index];
    record.external_open_interest_atoms = record
        .external_open_interest_atoms
        .checked_sub(params.claim_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    record.issuer_controlled_atoms = record
        .issuer_controlled_atoms
        .checked_add(params.claim_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    record.custody_status = WriterSeriesCustodyStatus::Open;
    record.settlement_liability_remaining_atoms = record
        .settlement_liability_remaining_atoms
        .checked_sub(payout)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if record.external_open_interest_atoms == 0 {
        record.settlement_status = WriterSeriesSettlementStatus::Exhausted;
    }
    market.mint_accounting.total_consumed = market
        .mint_accounting
        .total_consumed
        .checked_add(params.claim_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if market_outstanding_contract_amount(&market)? != record.external_open_interest_atoms
        || record
            .issuer_controlled_atoms
            .checked_add(record.external_open_interest_atoms)
            != Some(record.total_physical_supply_atoms)
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    sleeve.long_liability_remaining_atoms = sleeve
        .long_liability_remaining_atoms
        .checked_sub(payout)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.accounted_asset_atoms = sleeve
        .accounted_asset_atoms
        .checked_sub(payout)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.exact_reserve_atoms = sleeve.long_liability_remaining_atoms;
    let slot = Clock::get()?.slot;
    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = slot;
    sleeve.last_updated_slot = slot;
    store_state(market_info, &market)?;
    store_state(book_info, &book)?;
    store_state(sleeve_info, &sleeve)
}

pub(super) fn process_claim_writer_flat_residual(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ClaimWriterFlatResidualV1Params,
) -> ProgramResult {
    if accounts.len() != CLAIM_WRITER_FLAT_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    if params.flat_atoms == 0 {
        return Err(VaultError::AmountMustBePositive.into());
    }
    let holder_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let flat_mint_info = &accounts[3];
    let flat_source_info = &accounts[4];
    let flat_burn_info = &accounts[5];
    let flat_interface_info = &accounts[6];
    let sleeve_vault_info = &accounts[7];
    let payout_destination_info = &accounts[8];
    let settlement_mint_info = &accounts[9];
    let usdc_interface_info = &accounts[10];
    let light_program_info = &accounts[11];
    let cpi_authority_info = &accounts[12];
    let token_program_info = &accounts[13];
    let system_program_info = &accounts[14];
    let compressible_config_info = &accounts[15];
    let rent_sponsor_info = &accounts[16];
    if !holder_info.is_signer || !holder_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_writer_compression_accounts(
        light_program_info,
        cpi_authority_info,
        token_program_info,
        system_program_info,
        compressible_config_info,
        rent_sponsor_info,
    )?;
    let config = load_canonical_vault_config(program_id, config_info)?;
    let mut sleeve = load_writer_sleeve_without_group_meta(program_id, sleeve_info)?;
    if config.paused
        || sleeve.vault_config != *config_info.key
        || sleeve.status != WriterSleeveStatus::SettlementFinalized
        || sleeve.active_auction.is_some()
        || sleeve.active_close_request.is_some()
        || sleeve.flat_mint != *flat_mint_info.key
        || sleeve.usdc_vault != *sleeve_vault_info.key
        || sleeve.settlement_mint != *settlement_mint_info.key
        || config.usdc_mint != *settlement_mint_info.key
        || params.flat_atoms > sleeve.flat_claim_supply_remaining_atoms
        || params.flat_atoms > sleeve.flat_par_supply_atoms
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let payout = cumulative_allocation_delta(
        sleeve.flat_supply_snapshot_atoms,
        sleeve.flat_claim_supply_remaining_atoms,
        params.flat_atoms,
        sleeve.flat_residual_initial_atoms,
    )
    .map_err(writer_math_error)?;
    if payout > sleeve.flat_residual_remaining_atoms || payout > sleeve.accounted_asset_atoms {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let flat_mint = validate_writer_flat_mint(sleeve_info, &sleeve, flat_mint_info)?;
    validate_light_associated_token_account(holder_info.key, flat_mint_info.key, flat_source_info)?;
    let source_before =
        load_canonical_light_token_account(flat_source_info, holder_info.key, flat_mint_info.key)?;
    if source_before.amount < params.flat_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    validate_spl_interface_account(flat_mint_info.key, flat_interface_info)?;
    let (expected_burn, burn_bump) =
        derive_writer_flat_burn_custody_pda(program_id, sleeve_info.key);
    let burn_before = funding::load_or_create_sleeve_token_custody(
        program_id,
        holder_info,
        sleeve_info,
        flat_burn_info,
        flat_mint_info,
        token_program_info,
        system_program_info,
        crate::constants::WRITER_FLAT_BURN_CUSTODY_PDA_SEED,
        expected_burn,
        burn_bump,
    )?;
    if burn_before.amount != 0 {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    validate_vault_token_account(sleeve_vault_info, settlement_mint_info.key, sleeve_info.key)?;
    let vault_before = validate_token_account(sleeve_vault_info)?.amount;
    if vault_before < sleeve.accounted_asset_atoms || vault_before < payout {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    validate_collateral_mint_account(settlement_mint_info, token_program_info.key)?;
    validate_spl_interface_account(settlement_mint_info.key, usdc_interface_info)?;
    let destination_before = load_or_create_light_associated_token_account(
        holder_info,
        holder_info,
        settlement_mint_info,
        payout_destination_info,
        light_program_info,
        compressible_config_info,
        rent_sponsor_info,
        system_program_info,
    )?;

    invoke_light_token_account_transfer(
        params.flat_atoms,
        MarketMintAccounting::CANONICAL_DECIMALS,
        light_program_info,
        cpi_authority_info,
        holder_info,
        flat_source_info,
        flat_burn_info,
        holder_info,
        flat_mint_info,
        flat_interface_info,
        token_program_info,
        system_program_info,
    )?;
    let bump = [sleeve.bump];
    let signer = writer_sleeve_signer_seeds(&sleeve.settlement_group, &bump);
    invoke_token_burn_checked(
        token_program_info,
        flat_burn_info,
        flat_mint_info,
        sleeve_info,
        params.flat_atoms,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &[&signer],
    )?;
    if payout != 0 {
        invoke_light_token_account_transfer_with_signer_seeds(
            payout,
            MarketMintAccounting::CANONICAL_DECIMALS,
            light_program_info,
            cpi_authority_info,
            holder_info,
            sleeve_vault_info,
            payout_destination_info,
            sleeve_info,
            settlement_mint_info,
            usdc_interface_info,
            token_program_info,
            system_program_info,
            &[&signer],
        )?;
    }
    let source_after =
        load_canonical_light_token_account(flat_source_info, holder_info.key, flat_mint_info.key)?;
    let destination_after = load_canonical_light_token_account(
        payout_destination_info,
        holder_info.key,
        settlement_mint_info.key,
    )?;
    if source_before.amount.checked_sub(source_after.amount) != Some(params.flat_atoms)
        || validate_token_account(flat_burn_info)?.amount != 0
        || flat_mint
            .supply
            .checked_sub(validate_mint_account(flat_mint_info, token_program_info.key)?.supply)
            != Some(params.flat_atoms)
        || destination_after
            .amount
            .checked_sub(destination_before.amount)
            != Some(payout)
        || vault_before.checked_sub(validate_token_account(sleeve_vault_info)?.amount)
            != Some(payout)
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    funding::close_sleeve_token_custody(
        &sleeve,
        sleeve_info,
        flat_burn_info,
        holder_info,
        token_program_info,
    )?;
    sleeve.flat_claim_supply_remaining_atoms = sleeve
        .flat_claim_supply_remaining_atoms
        .checked_sub(params.flat_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.flat_par_supply_atoms = sleeve
        .flat_par_supply_atoms
        .checked_sub(params.flat_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.flat_residual_remaining_atoms = sleeve
        .flat_residual_remaining_atoms
        .checked_sub(payout)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.accounted_asset_atoms = sleeve
        .accounted_asset_atoms
        .checked_sub(payout)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.last_updated_slot = Clock::get()?.slot;
    store_state(sleeve_info, &sleeve)
}

pub(super) fn process_close_writer_sleeve(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != CLOSE_WRITER_SLEEVE_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let snapshot_info = &accounts[5];
    let sleeve_vault_info = &accounts[6];
    let flat_mint_info = &accounts[7];
    let token_program_info = &accounts[8];
    let system_program_info = &accounts[9];
    if !cranker_info.is_signer || !cranker_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id()
        || *system_program_info.key != system_program::id()
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    let WriterPolicyContext {
        mut group,
        mut sleeve,
        mut book,
        snapshot,
    } = load_writer_policy_context(
        program_id,
        sleeve_info,
        group_info,
        book_info,
        snapshot_info,
        None,
    )?;
    if sleeve.vault_config != *config_info.key
        || sleeve.status != WriterSleeveStatus::SettlementFinalized
        || group.status != WriterSettlementGroupStatus::Settled
        || sleeve.policy_snapshot != *snapshot_info.key
        || sleeve.policy_hash != snapshot.policy_hash
        || sleeve.active_auction.is_some()
        || sleeve.active_close_request.is_some()
        || sleeve.accounted_asset_atoms != 0
        || sleeve.exact_reserve_atoms != 0
        || sleeve.long_liability_remaining_atoms != 0
        || sleeve.flat_residual_remaining_atoms != 0
        || sleeve.flat_claim_supply_remaining_atoms != 0
        || sleeve.flat_par_supply_atoms != 0
        || sleeve.usdc_vault != *sleeve_vault_info.key
        || sleeve.flat_mint != *flat_mint_info.key
        || book.records[..usize::from(book.series_count)]
            .iter()
            .any(|record| {
                record.external_open_interest_atoms != 0
                    || record.issuer_controlled_atoms != 0
                    || record.total_physical_supply_atoms != 0
                    || record.settlement_liability_remaining_atoms != 0
                    || record.settlement_status != WriterSeriesSettlementStatus::Exhausted
                    || record.custody_status == WriterSeriesCustodyStatus::Open
            })
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    validate_vault_token_account(sleeve_vault_info, &sleeve.settlement_mint, sleeve_info.key)?;
    let physical = validate_token_account(sleeve_vault_info)?.amount;
    if physical < sleeve.stranded_surplus_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let flat_mint = validate_mint_account(flat_mint_info, token_program_info.key)?;
    if flat_mint.supply != 0
        || flat_mint.decimals != MarketMintAccounting::CANONICAL_DECIMALS
        || flat_mint.mint_authority != COption::Some(*sleeve_info.key)
        || flat_mint.freeze_authority != COption::None
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    if physical == 0 {
        let bump = [sleeve.bump];
        let signer = writer_sleeve_signer_seeds(&sleeve.settlement_group, &bump);
        invoke_token_close_account(
            token_program_info,
            sleeve_vault_info,
            cranker_info,
            sleeve_info,
            &[&signer],
        )?;
    }
    let slot = Clock::get()?.slot;
    group.status = WriterSettlementGroupStatus::Closed;
    group.last_updated_slot = slot;
    sleeve.status = WriterSleeveStatus::Closed;
    sleeve.last_updated_slot = slot;
    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = slot;
    let _ = config;
    store_state(group_info, &group)?;
    store_state(book_info, &book)?;
    store_state(sleeve_info, &sleeve)
}
