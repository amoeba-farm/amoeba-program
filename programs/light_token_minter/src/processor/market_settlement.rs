use super::*;

pub(super) fn process_upsert_market_page(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    page: CompressedMarketPageLeaf,
    proof: light_sdk::proof::borsh_compat::ValidityProof,
    new_page_output: Option<crate::instruction::CompressionOutput>,
    existing_page: Option<CompressedMarketPageWitness>,
) -> ProgramResult {
    if accounts.len() < 3 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let system_program_info = &accounts[2];

    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }

    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.paused {
        return Err(VaultError::ContractPaused.into());
    }
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    validate_market_page(&page)?;

    if let Some(existing) = existing_page.as_ref() {
        if existing.page.item_id != page.item_id
            || existing.page.underlying_id != page.underlying_id
        {
            return Err(VaultError::MarketPageIdMismatch.into());
        }
        let expected_commitment = existing
            .page
            .compute_commitment()
            .map_err(|_| VaultError::MarketPageHashMismatch)?;
        if existing.expected_commitment != expected_commitment {
            return Err(VaultError::MarketPageHashMismatch.into());
        }
        let updates = [MarketPageLeafUpdate {
            meta: &existing.meta,
            page: &page,
        }];
        apply_market_page_leaf_mutations(
            program_id,
            admin_info,
            &accounts[3..],
            &proof,
            &updates,
            &[],
        )
    } else {
        let output = new_page_output.ok_or(VaultError::InvalidCompressionWitness)?;
        let creates = [MarketPageLeafCreate {
            output: &output,
            page: &page,
        }];
        apply_market_page_leaf_mutations(
            program_id,
            admin_info,
            &accounts[3..],
            &proof,
            &[],
            &creates,
        )
    }
}

#[inline(never)]
pub(super) fn process_upsert_settlement(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    mut settlement: Box<CompressedSettlementLeaf>,
    proof: Box<light_sdk::proof::borsh_compat::ValidityProof>,
    new_settlement_output: Option<Box<crate::instruction::CompressionOutput>>,
    existing_settlement: Option<Box<CompressedSettlementWitness>>,
) -> ProgramResult {
    if accounts.len() < 12 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let config_info = &accounts[1];
    let market_info = &accounts[2];
    let oracle_month_info = &accounts[3];
    let settlement_record_info = &accounts[4];
    let recipe_manifest_info = &accounts[5];
    let active_manifest_info = &accounts[6];
    let source_manifest_info = &accounts[7];
    let signer_registry_info = &accounts[8];
    let signer_set_info = &accounts[9];
    let instructions_sysvar_info = &accounts[10];
    let system_program_info = &accounts[11];
    let light_accounts = &accounts[12..];

    if !authority_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    if *instructions_sysvar_info.key != instructions::id() {
        return Err(VaultError::InvalidInstructionsSysvar.into());
    }

    let config = Box::new(load_canonical_vault_config(program_id, config_info)?);
    if !config.has_current_layout() {
        return Err(VaultError::InvalidConfigAccount.into());
    }
    let market = Box::new(load_valid_market(program_id, market_info)?);
    ensure_market_value_flow_unpaused(&config, &market)?;
    let _ = market_outstanding_contract_amount(&market)?;
    let mut oracle_month = Box::new(load_valid_oracle_month(
        program_id,
        market_info,
        oracle_month_info,
        &market,
    )?);
    let recipe_manifest = Box::new(load_valid_oracle_recipe_weight_manifest(
        program_id,
        oracle_month_info.key,
        recipe_manifest_info,
    )?);
    let source_manifest = Box::new(load_valid_oracle_settlement_source_manifest(
        program_id,
        oracle_month_info.key,
        source_manifest_info,
    )?);
    let active_manifest = Box::new(load_valid_oracle_active_weight_manifest(
        program_id,
        oracle_month_info.key,
        active_manifest_info,
    )?);
    validate_canonical_settlement_provenance(
        oracle_month_info.key,
        &oracle_month,
        &recipe_manifest,
        &active_manifest,
        &source_manifest,
        &settlement,
    )?;
    validate_settlement_market_binding(
        program_id,
        market_info,
        oracle_month_info,
        &market,
        &oracle_month,
        &settlement,
    )?;
    ensure_settlement_finalization_ready(&market)?;
    let registry = Box::new(load_canonical_settlement_signer_registry(
        program_id,
        signer_registry_info,
    )?);
    let signer_set = Box::new(load_canonical_settlement_signer_set(
        program_id,
        signer_registry_info.key,
        signer_set_info,
    )?);
    if registry.recovery_authority == config.admin
        || registry.recovery_authority == config.oracle_authority
        || registry.current_set != *signer_set_info.key
    {
        return Err(VaultError::SettlementSignerGovernanceRequired.into());
    }
    settlement_signer_set_excludes_governance(
        &signer_set,
        &[
            config.admin,
            config.oracle_authority,
            registry.recovery_authority,
        ],
    )?;
    let _signer_bitmap = verify_settlement_oracle_attestations(
        program_id,
        &settlement,
        signer_registry_info,
        signer_set_info,
        instructions_sysvar_info,
    )?;
    let (submitted_slot, now) = current_slot_and_unix_timestamp()?;
    if now < settlement.settlement_ts {
        return Err(VaultError::MarketNotExpired.into());
    }
    validate_settlement_leaf(&settlement, &oracle_month)?;
    let signed_leaf_commitment = settlement_signed_leaf_commitment(&settlement)?;

    let (existing_record, record_bump) = load_or_create_settlement_record_v2(
        program_id,
        authority_info,
        market_info,
        oracle_month_info,
        settlement_record_info,
        system_program_info,
    )?;
    let existing_record = existing_record.map(Box::new);

    if let Some(record) = existing_record.as_ref() {
        if !settlement_record_v2_matches_submission(
            record,
            market_info.key,
            oracle_month_info.key,
            &settlement,
            &signed_leaf_commitment,
            authority_info.key,
        ) || oracle_month.phase != OraclePhase::Settled
            || oracle_month.settlement_record != Some(*settlement_record_info.key)
        {
            return Err(VaultError::SettlementRecordImmutable.into());
        }
        if let Some(existing_leaf) = existing_settlement.as_ref() {
            let expected_commitment = existing_leaf
                .settlement
                .compute_commitment()
                .map_err(|_| VaultError::SettlementRecordHashMismatch)?;
            let existing_signed_commitment =
                settlement_signed_leaf_commitment(&existing_leaf.settlement)?;
            if existing_leaf.expected_commitment != expected_commitment
                || existing_signed_commitment != signed_leaf_commitment
                || existing_leaf.settlement.submitted_by != record.submitted_by
                || existing_leaf.settlement.submitted_slot != oracle_month.last_updated_slot
            {
                return Err(VaultError::SettlementRecordImmutable.into());
            }
        }
        return Ok(());
    }

    if oracle_month.phase != OraclePhase::Game || existing_settlement.is_some() {
        return Err(VaultError::SettlementRecordImmutable.into());
    }
    ensure_oracle_month_ready_for_settlement(&oracle_month)?;
    settlement.submitted_by = *authority_info.key;
    settlement.submitted_slot = submitted_slot;
    let settlement_record = settlement_record_v2_from_leaf(
        record_bump,
        *market_info.key,
        *oracle_month_info.key,
        &settlement,
        signed_leaf_commitment,
        *authority_info.key,
    );
    store_state(settlement_record_info, &settlement_record)?;
    oracle_month.phase = OraclePhase::Settled;
    oracle_month.settlement_record = Some(*settlement_record_info.key);
    oracle_month.last_updated_slot = submitted_slot;
    store_state(oracle_month_info, oracle_month.as_ref())?;

    let output = new_settlement_output.ok_or(VaultError::InvalidCompressionWitness)?;
    let creates = [SettlementLeafCreate {
        output: output.as_ref(),
        settlement: settlement.as_ref(),
    }];

    apply_settlement_leaf_mutations(
        program_id,
        authority_info,
        light_accounts,
        proof.as_ref(),
        &[],
        &creates,
    )
}
