use super::*;

pub(in crate::processor) fn process_open_funding(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 9 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let snapshot_info = &accounts[5];
    let sleeve_vault_info = &accounts[6];
    let flat_mint_info = &accounts[7];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let WriterPolicyContext {
        group,
        mut sleeve,
        book,
        snapshot,
    } = load_writer_policy_context(
        program_id,
        sleeve_info,
        group_info,
        book_info,
        snapshot_info,
        None,
    )?;
    validate_vault_token_account(sleeve_vault_info, &sleeve.settlement_mint, sleeve_info.key)?;
    validate_writer_flat_mint(sleeve_info, &sleeve, flat_mint_info)?;
    let _writer_liquidity_policy =
        dlmm::load_funding_policy(program_id, &accounts[8], sleeve_info, &sleeve, &snapshot)?;
    if group.sleeve != *sleeve_info.key
        || sleeve.usdc_vault != *sleeve_vault_info.key
        || sleeve.flat_mint != *flat_mint_info.key
        || sleeve.policy_snapshot != *snapshot_info.key
        || sleeve.status != WriterSleeveStatus::PolicyFrozen
        || group.status != WriterSettlementGroupStatus::Anchored
        || !book.frozen
        || book.series_count == 0
        || validate_token_account(sleeve_vault_info)?.amount != sleeve.accounted_asset_atoms
        || snapshot.policy_hash != sleeve.policy_hash
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    sleeve.status = WriterSleeveStatus::Funding;
    sleeve.last_updated_slot = Clock::get()?.slot;
    store_state(sleeve_info, &sleeve)
}

pub(in crate::processor) fn process_activate_sleeve(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 16 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let snapshot_info = &accounts[5];
    let anchor_market_info = &accounts[6];
    let month_info = &accounts[7];
    let coverage_info = &accounts[8];
    let active_weight_info = &accounts[9];
    let recipe_info = &accounts[10];
    let settlement_source_info = &accounts[11];
    let signer_registry_info = &accounts[12];
    let signer_set_info = &accounts[13];
    let sleeve_vault_info = &accounts[14];
    let flat_mint_info = &accounts[15];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key || config.paused {
        return Err(VaultError::Unauthorized.into());
    }
    let WriterPolicyContext {
        mut group,
        mut sleeve,
        book,
        snapshot,
    } = load_writer_policy_context(
        program_id,
        sleeve_info,
        group_info,
        book_info,
        snapshot_info,
        None,
    )?;
    let (anchor_market, month) =
        load_valid_market_and_oracle_month(program_id, anchor_market_info, month_info)?;
    ensure_oracle_game_window(&anchor_market, &month)?;
    let coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    let active =
        load_valid_oracle_active_weight_manifest(program_id, month_info.key, active_weight_info)?;
    ensure_finalized_oracle_active_weight_manifest(&month, &active)?;
    ensure_finalized_oracle_issue_sku_coverage(&month, &coverage, &active)?;
    let recipe = load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, recipe_info)?;
    // Terminal evidence does not exist until expiry + the settlement grace period.
    // Retain its canonical future address in the ABI without creating a placeholder.
    if *settlement_source_info.key
        != crate::state::derive_oracle_settlement_source_manifest_pda(program_id, month_info.key).0
        || settlement_source_info.owner != &system_program::id()
        || !settlement_source_info.data_is_empty()
        || settlement_source_info.executable
        || settlement_source_info.is_signer
        || settlement_source_info.is_writable
    {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    let signer_registry =
        load_canonical_settlement_signer_registry(program_id, signer_registry_info)?;
    let signer_set = load_canonical_settlement_signer_set(
        program_id,
        signer_registry_info.key,
        signer_set_info,
    )?;
    validate_vault_token_account(sleeve_vault_info, &sleeve.settlement_mint, sleeve_info.key)?;
    validate_writer_flat_mint(sleeve_info, &sleeve, flat_mint_info)?;
    let physical_assets = validate_token_account(sleeve_vault_info)?.amount;
    let anchor_registered = book
        .records
        .iter()
        .take(usize::from(book.series_count))
        .any(|record| record.market == *anchor_market_info.key);
    recompute_writer_metrics(
        &mut sleeve,
        &book,
        &snapshot,
        Some(active.max_open_interest_payout),
        true,
    )?;
    let required_assets = sleeve
        .exact_reserve_atoms
        .checked_add(snapshot.operational_buffer_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let activation_assets_sufficient = writer_activation_assets_are_sufficient(&sleeve)?;
    if group.sleeve != *sleeve_info.key
        || group.anchor_market != *anchor_market_info.key
        || group.anchor_oracle_month != *month_info.key
        || group.signer_registry != *signer_registry_info.key
        || signer_registry.current_set != *signer_set_info.key
        || signer_registry.current_version != signer_set.version
        || sleeve.status != WriterSleeveStatus::Funding
        || group.status != WriterSettlementGroupStatus::Anchored
        || !book.frozen
        || sleeve.policy_snapshot != *snapshot_info.key
        || snapshot.policy_hash != sleeve.policy_hash
        || snapshot.series_family_hash != writer_series_family_hash(&book)
        || !anchor_registered
        || sleeve.writer_principal_atoms == 0
        || sleeve.writer_principal_atoms != sleeve.flat_par_supply_atoms
        || !activation_assets_sufficient
        || sleeve.accounted_asset_atoms < required_assets
        || physical_assets < sleeve.accounted_asset_atoms
        || recipe.phase != OracleRecipeWeightPhase::Finalized
        || recipe.recipe_hash != month.recipe_hash
        || recipe.rolling_manifest_hash != month.weight_manifest_hash
        || canonical_recipe_digest(month_info.key, &recipe.rolling_manifest_hash)
            != month.recipe_hash
        || recipe.expected_source_count != month.frozen_source_count
        || recipe.expected_bucket_count != month.active_weight_group_count
        || recipe.processed_source_count != recipe.expected_source_count
        || recipe.processed_bucket_count != recipe.expected_bucket_count
        || recipe.declared_weight_total_bps != 10_000
        || !crate::bytes32_is_zero(&group.settlement_source_digest)
        || !crate::bytes32_is_zero(&group.final_settlement_commitment)
        || group.finalized_slot != 0
        || active.max_open_interest_payout == 0
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let slot = Clock::get()?.slot;
    group.product_manifest_root = coverage.required_sku_root;
    group.coverage_manifest_hash = writer_coverage_manifest_hash(&coverage);
    group.recipe_hash = recipe.recipe_hash;
    // Source identities are frozen by recipe/coverage/active hashes. The actual
    // terminal digest remains zero until normal settlement publication binds it.
    group.active_weight_manifest_hash = active.rolling_manifest_hash;
    group.security_cap_atoms = active.max_open_interest_payout;
    group.signer_set = *signer_set_info.key;
    group.signer_set_version = signer_set.version;
    group.signer_set_hash = signer_set.set_hash;
    group.status = WriterSettlementGroupStatus::Active;
    group.last_updated_slot = slot;
    sleeve.status = WriterSleeveStatus::Active;
    sleeve.last_updated_slot = slot;
    store_state(group_info, &group)?;
    store_state(sleeve_info, &sleeve)
}

pub(in crate::processor) fn process_set_collective_market_paused(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SetCollectiveMarketPausedV1Params,
) -> ProgramResult {
    if accounts.len() != 9 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let market_info = &accounts[5];
    let month_info = &accounts[6];
    let coverage_info = &accounts[7];
    let active_weight_info = &accounts[8];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let WriterBookContext {
        group,
        sleeve,
        book,
    } = load_writer_book_context(program_id, sleeve_info, group_info, book_info)?;
    let record = book
        .records
        .iter()
        .take(usize::from(book.series_count))
        .find(|record| record.market == *market_info.key)
        .ok_or(VaultError::InvalidWriterSeriesBook)?;
    if group.sleeve != *sleeve_info.key {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mut market = load_valid_market(program_id, market_info)?;
    if record.series_id != market.market_id
        || Some(record.contract_mint) != market.long_contract_mint
        || market.instrument.underlying_id != group.underlying_id
        || market.instrument.expiry_ts != group.expiry_ts
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    if !params.paused {
        if config.paused
            || sleeve.status != WriterSleeveStatus::Active
            || group.status != WriterSettlementGroupStatus::Active
        {
            return Err(VaultError::InvalidWriterLifecycle.into());
        }
        let month = load_oracle_month_state(month_info, program_id)?;
        let (expected_month, expected_bump) =
            derive_oracle_month_pda(program_id, &group.anchor_market, group.expiry_ts);
        if *month_info.key != group.anchor_oracle_month
            || *month_info.key != expected_month
            || !month.is_initialized
            || month.bump != expected_bump
            || month.market != group.anchor_market
        {
            return Err(VaultError::InvalidOracleMonthAccount.into());
        }
        // All registered Markets share the group's expiry, so the canonical anchor month's
        // Game/timing window is safely checked against this target Market without inventing a
        // second settlement world or requiring an extra, undocumented account meta.
        ensure_oracle_game_window(&market, &month)?;
        let coverage =
            load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
        let active = load_valid_oracle_active_weight_manifest(
            program_id,
            month_info.key,
            active_weight_info,
        )?;
        ensure_finalized_oracle_active_weight_manifest(&month, &active)?;
        ensure_finalized_oracle_issue_sku_coverage(&month, &coverage, &active)?;
        if active.rolling_manifest_hash != group.active_weight_manifest_hash
            || writer_coverage_manifest_hash(&coverage) != group.coverage_manifest_hash
        {
            return Err(VaultError::InvalidWriterSettlementGroup.into());
        }
    }
    market.paused = params.paused;
    store_state(market_info, &market)
}
