use super::*;
use crate::{
    writer_portfolio::{portfolio_liability_numerator, portfolio_settlement_partition},
    writer_settlement_handoff::{
        authorized_handoff_version, derive_writer_settlement_handoff, WriterSettlementHandoffV3,
        HANDOFF_PAYLOAD, HANDOFF_SEED,
    },
    writer_sleeve_math::settlement_series_liabilities,
};

const PUBLISH_WRITER_GROUP_SETTLEMENT_ACCOUNT_COUNT: usize = 13;
const FINALIZE_WRITER_SETTLEMENT_FIXED_ACCOUNT_COUNT: usize = 8;
const CLOSE_WRITER_SLEEVE_ACCOUNT_COUNT: usize = 9;
const WRITER_GROUP_FINAL_SETTLEMENT_DOMAIN: &[u8] = b"ameba-writer-final-settlement-g3";

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
    payload: &[u8],
) -> ProgramResult {
    let handoff = match payload {
        [] => false,
        bytes if bytes == HANDOFF_PAYLOAD => true,
        _ => return Err(ProgramError::InvalidInstructionData),
    };
    if accounts.len() != PUBLISH_WRITER_GROUP_SETTLEMENT_ACCOUNT_COUNT + if handoff { 2 } else { 0 }
    {
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
        || group.anchor_market != *anchor_market_info.key
        || group.anchor_oracle_month != *month_info.key
        || group.signer_registry != *signer_registry_info.key
        || (!handoff && group.signer_set != *signer_set_info.key)
        || !crate::bytes32_is_zero(&group.settlement_source_digest)
        || !crate::bytes32_is_zero(&group.final_settlement_commitment)
        || group.finalized_slot != 0
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
    ensure_settlement_finalization_ready(&market)?;
    ensure_oracle_month_ready_for_settlement(&month)?;
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
    let registry = load_canonical_settlement_signer_registry(program_id, signer_registry_info)?;
    let signer_set = load_canonical_settlement_signer_set(
        program_id,
        signer_registry_info.key,
        signer_set_info,
    )?;
    ensure_finalized_oracle_active_weight_manifest(&month, &active)?;
    ensure_finalized_oracle_issue_sku_coverage(&month, &coverage, &active)?;
    // The canonical record can only be created by a then-current signer quorum.
    // Never use this handoff to authenticate signatures or rewrite the original
    // group binding. A later rotation cannot invalidate a completed attestation.
    let signer_binding_valid = if handoff {
        authorized_handoff_version(
            group.signer_set_version,
            signer_set.version,
            registry.current_version,
        ) && group.signer_set
            == derive_settlement_signer_set_pda(program_id, group.signer_set_version).0
            && !crate::bytes32_is_zero(&group.signer_set_hash)
    } else {
        signer_set.version == group.signer_set_version
            && signer_set.set_hash == group.signer_set_hash
    };
    if month.phase != OraclePhase::Settled
        || month.finalized_at_ts == 0
        || month.settlement_record != Some(*settlement_info.key)
        || settlement.settlement_ts != group.settlement_ts
        || settlement.signer_set_version != signer_set.version
        || !signer_binding_valid
        || writer_coverage_manifest_hash(&coverage) != group.coverage_manifest_hash
        || recipe.phase != OracleRecipeWeightPhase::Finalized
        || recipe.recipe_hash != group.recipe_hash
        || recipe.recipe_hash != month.recipe_hash
        || recipe.rolling_manifest_hash != month.weight_manifest_hash
        || canonical_recipe_digest(month_info.key, &recipe.rolling_manifest_hash)
            != month.recipe_hash
        || recipe.expected_source_count != month.frozen_source_count
        || recipe.expected_bucket_count != month.active_weight_group_count
        || recipe.processed_source_count != recipe.expected_source_count
        || recipe.processed_bucket_count != recipe.expected_bucket_count
        || recipe.declared_weight_total_bps != 10_000
        || settlement_sources.phase != OracleRecipeWeightPhase::Finalized
        || settlement_sources.expected_source_count != month.frozen_source_count
        || settlement_sources.expected_bucket_count != month.active_weight_group_count
        || settlement_sources.processed_source_count != settlement_sources.expected_source_count
        || settlement_sources.processed_bucket_count != settlement_sources.expected_bucket_count
        || settlement_sources.declared_weight_total_bps != 10_000
        || crate::bytes32_is_zero(&settlement_sources.rolling_source_digest)
        || active.rolling_manifest_hash != group.active_weight_manifest_hash
        || active.max_open_interest_payout != group.security_cap_atoms
    {
        return Err(VaultError::InvalidWriterSettlementGroup.into());
    }
    let slot = crate::compact_error::slot()?;
    if handoff {
        let handoff_info = &accounts[13];
        let system_info = &accounts[14];
        let (key, bump) = derive_writer_settlement_handoff(program_id, group_info.key);
        if key != *handoff_info.key || !crate::is_system_program(system_info.key) {
            return Err(VaultError::InvalidAccountList.into());
        }
        validate_create_only_program_account_target(program_id, handoff_info)?;
        create_program_account(
            submitter_info,
            handoff_info,
            system_info,
            program_id,
            WriterSettlementHandoffV3::LEN,
            &[HANDOFF_SEED, group_info.key.as_ref(), &[bump]],
        )?;
        store_state(
            handoff_info,
            &WriterSettlementHandoffV3 {
                initialized: true,
                bump,
                discriminator: WriterSettlementHandoffV3::DISCRIMINATOR,
                version: WriterSettlementHandoffV3::VERSION,
                group: *group_info.key,
                group_commitment_before_settlement: writer_group_commitment(group_info.key, &group),
                original_signer_set: group.signer_set,
                original_version: group.signer_set_version,
                replacement_signer_set: *signer_set_info.key,
                replacement_version: signer_set.version,
                replacement_set_hash: signer_set.set_hash,
                settlement_record: *settlement_info.key,
                recorded_slot: slot,
            },
        )?;
    }
    // Bind terminal evidence exactly once, before hashing the final group state.
    group.settlement_source_digest = settlement_sources.rolling_source_digest;
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
    let lp_policy_info = &accounts[7];
    if !cranker_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    let WriterBookContext {
        group,
        mut sleeve,
        mut book,
    } = load_writer_book_context(program_id, sleeve_info, group_info, book_info)?;
    let mint_end = FINALIZE_WRITER_SETTLEMENT_FIXED_ACCOUNT_COUNT + usize::from(book.series_count);
    if accounts.len() != mint_end && accounts.len() != mint_end + 1 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let snapshot = load_writer_policy_snapshot(
        program_id,
        snapshot_info,
        sleeve_info.key,
        &sleeve.policy_registry,
        sleeve.policy_version,
    )?;
    let lp_policy = dlmm::load_optional_policy(program_id, lp_policy_info, sleeve_info, &sleeve)?;
    dlmm::require_unwound_policy(lp_policy.as_deref())?;
    if config.paused
        || sleeve.vault_config != *config_info.key
        || sleeve.status != WriterSleeveStatus::Expired
        || group.status != WriterSettlementGroupStatus::Settled
        || crate::bytes32_is_zero(&group.final_settlement_commitment)
        || sleeve.policy_snapshot != *snapshot_info.key
        || sleeve.policy_hash != snapshot.policy_hash
        || sleeve.usdc_vault != *sleeve_vault_info.key
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    validate_vault_token_account(sleeve_vault_info, &sleeve.settlement_mint, sleeve_info.key)?;
    let compressed_cash = crate::compressed_custody::load(
        program_id,
        accounts.get(mint_end),
        crate::compressed_custody::CustodyKind::WriterCash,
        sleeve_vault_info.key,
        &Pubkey::default(),
        &sleeve.settlement_mint,
    )?;
    let individual_funding = book.individual.funded_long_liability;
    let owner_credits = book.individual.total_portfolio_credit;
    let has_portfolio_exposure = book
        .individual
        .series
        .iter()
        .any(|series| series.issued > 0)
        || book
            .individual
            .active_locked
            .iter()
            .any(|quantity| *quantity != 0)
        || book.individual.funding_base_initialized;
    if (has_portfolio_exposure
        && (!book.individual.funded || !book.individual.funding_base_initialized))
        || book.individual.pending_portfolio_funding != 0
        || book.individual.active_locked != book.individual.hedge_retired
        || book.individual.remaining_portfolio_credit != owner_credits
        || book.individual.funded_stranded != 0
        || !crate::compressed_custody::backs(
            compressed_cash.as_ref(),
            0,
            validate_token_account(sleeve_vault_info)?.amount,
            0,
            sleeve
                .accounted_asset_atoms
                .checked_add(individual_funding)
                .ok_or(VaultError::ArithmeticOverflow)?,
        )
        || compressed_cash
            .as_ref()
            .is_some_and(|cash| cash.option_atoms != 0)
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let count = usize::from(book.series_count);
    // Account count and book size were authenticated above. Iterate over exactly
    // the mint accounts so the optional WriterCash account is never included.
    for (index, mint_info) in accounts
        .iter()
        .skip(FINALIZE_WRITER_SETTLEMENT_FIXED_ACCOUNT_COUNT)
        .take(count)
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
            || Some(record.total_physical_supply_atoms)
                != book
                    .external_total(index)
                    .and_then(|n| n.checked_add(book.individual.compressed_retired_atoms[index]))
                    .and_then(|n| n.checked_add(book.individual.forfeited_atoms[index]))
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
    let issued = core::array::from_fn(|index| book.individual.series[index].issued);
    let owner_net = portfolio_liability_numerator(
        &series,
        &issued,
        &book.individual.hedge_retired,
        group.settlement_price_atomic,
    )
    .map_err(writer_math_error)?;
    let managed_numerator = if book.individual.funding_base_initialized {
        if i128::from_le_bytes(book.individual.funding_prefix_numerator_le) != owner_net {
            return Err(VaultError::WriterSolvencyViolation.into());
        }
        i128::from_le_bytes(book.individual.funding_managed_numerator_le)
    } else {
        if owner_net != 0 || individual_funding != 0 || owner_credits != 0 {
            return Err(VaultError::WriterSolvencyViolation.into());
        }
        i128::try_from(
            crate::writer_sleeve_math::aggregate_liability_numerator(
                &series,
                group.settlement_price_atomic,
            )
            .map_err(writer_math_error)?,
        )
        .map_err(|_| VaultError::ArithmeticOverflow)?
    };
    if !book.individual.funding_base_initialized {
        book.individual.funding_base_initialized = true;
        book.individual.funding_managed_numerator_le = managed_numerator.to_le_bytes();
        book.individual.funding_prefix_numerator_le = owner_net.to_le_bytes();
        book.individual.funded = true;
    }
    // The first hedge retirement may have already consolidated these quantities.
    // Managed P&L always uses the immutable pre-consolidation numerator above.
    if !book.individual.hedges_consolidated {
        for index in 0..count {
            book.records[index].external_open_interest_atoms = book
                .external_total(index)
                .ok_or(VaultError::ArithmeticOverflow)?;
            book.individual.series[index].outstanding = 0;
        }
        book.individual.hedges_consolidated = true;
    }
    let (liabilities, long_total) = settlement_series_liabilities(
        &writer_book_math_series(&book)?,
        group.settlement_price_atomic,
    )
    .map_err(writer_math_error)?;
    let partition = portfolio_settlement_partition(
        sleeve.accounted_asset_atoms,
        managed_numerator,
        owner_net,
        individual_funding,
        owner_credits,
        long_total,
    )
    .map_err(writer_math_error)?;
    let mut writer_residual = partition.writer_residual;
    participation::admit_time_participation(
        &sleeve,
        sleeve.accounted_asset_atoms,
        partition.managed_liability,
    )?;
    sleeve.accounted_asset_atoms = partition.accounted_assets;
    sleeve.stranded_surplus_atoms = sleeve
        .stranded_surplus_atoms
        .checked_add(partition.stranded)
        .ok_or(VaultError::ArithmeticOverflow)?;
    book.individual.funded_stranded = partition.stranded;
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
    let entitlement_principal = sleeve.writer_principal_atoms;
    sleeve.settlement_principal_atoms = entitlement_principal;
    sleeve.unclaimed_principal_atoms = entitlement_principal;
    if entitlement_principal == 0 {
        sleeve.accounted_asset_atoms = partition.protected_reserve;
        sleeve.stranded_surplus_atoms = sleeve
            .stranded_surplus_atoms
            .checked_add(writer_residual)
            .ok_or(VaultError::ArithmeticOverflow)?;
        writer_residual = 0;
    }
    sleeve.writer_residual_initial_atoms = writer_residual;
    sleeve.writer_residual_remaining_atoms = writer_residual;
    sleeve.exact_reserve_atoms = partition.protected_reserve;
    sleeve.lower_tail_reserve_atoms = 0;
    sleeve.upper_tail_reserve_atoms = 0;
    sleeve.security_exposure_atoms = 0;
    let slot = crate::compact_error::slot()?;
    sleeve.status = WriterSleeveStatus::SettlementFinalized;
    book.individual.settlement_finalized_ts = current_unix_timestamp()?;
    sleeve.settlement_finalized_slot = slot;
    sleeve.last_updated_slot = slot;
    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = slot;
    store_state(book_info, &book)?;
    store_state(sleeve_info, &sleeve)
}

pub(super) fn process_close_writer_sleeve(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    // An optional tenth account is the sleeve vault's canonical WriterCash custody.
    if accounts.len() != CLOSE_WRITER_SLEEVE_ACCOUNT_COUNT
        && accounts.len() != CLOSE_WRITER_SLEEVE_ACCOUNT_COUNT + 1
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let snapshot_info = &accounts[5];
    let sleeve_vault_info = &accounts[6];
    let token_program_info = &accounts[7];
    let system_program_info = &accounts[8];
    if !cranker_info.is_signer || !cranker_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !crate::token_instruction::check_id(token_program_info.key)
        || !crate::is_system_program(system_program_info.key)
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
        || !matches!(
            (sleeve.status, group.status),
            (
                WriterSleeveStatus::SettlementFinalized,
                WriterSettlementGroupStatus::Settled
            ) | (
                WriterSleeveStatus::FundingRefunds,
                WriterSettlementGroupStatus::FundingExpired
            )
        )
        || sleeve.policy_snapshot != *snapshot_info.key
        || sleeve.policy_hash != snapshot.policy_hash
        || book.individual.cash_obligations != 0
        || book.individual.open_positions != 0
        || book.individual.pending_portfolio_funding != 0
        || book.individual.remaining_portfolio_credit != 0
        || sleeve.accounted_asset_atoms != 0
        || sleeve.exact_reserve_atoms != 0
        || sleeve.long_liability_remaining_atoms != 0
        || sleeve.writer_residual_remaining_atoms != 0
        || sleeve.unclaimed_principal_atoms != 0
        || sleeve.usdc_vault != *sleeve_vault_info.key
        || book.records[..usize::from(book.series_count)]
            .iter()
            .enumerate()
            .any(|(index, record)| {
                record.external_open_interest_atoms != 0
                    || record.issuer_controlled_atoms != 0
                    || Some(record.total_physical_supply_atoms)
                        != book.individual.compressed_retired_atoms[index]
                            .checked_add(book.individual.forfeited_atoms[index])
                    || record.settlement_liability_remaining_atoms != 0
                    || record.settlement_status != WriterSeriesSettlementStatus::Exhausted
                    || record.custody_status == WriterSeriesCustodyStatus::Open
            })
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    validate_vault_token_account(sleeve_vault_info, &sleeve.settlement_mint, sleeve_info.key)?;
    let physical = validate_token_account(sleeve_vault_info)?.amount;
    // Every claim is paid (accounted assets are zero), so compressed WriterCash is
    // surplus exactly like hot cash: it backs stranded surplus and is never moved.
    let cash = crate::compressed_custody::writer_cash(
        program_id,
        accounts.get(CLOSE_WRITER_SLEEVE_ACCOUNT_COUNT),
        sleeve_vault_info.key,
        &sleeve.settlement_mint,
    )?;
    if physical
        .checked_add(cash)
        .is_none_or(|cash| cash < sleeve.stranded_surplus_atoms)
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
    let slot = crate::compact_error::slot()?;
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
