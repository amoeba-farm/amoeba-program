use super::*;

pub(in crate::processor) fn process_initialize_settlement_group(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let group_info = &accounts[2];
    let market_info = &accounts[3];
    let month_info = &accounts[4];
    let settlement_mint_info = &accounts[5];
    let signer_registry_info = &accounts[6];
    let system_program_info = &accounts[7];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key || config.usdc_mint != *settlement_mint_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    validate_collateral_mint_account(settlement_mint_info, &spl_token_program_id())?;
    let (market, month) = load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let signer_registry =
        load_canonical_settlement_signer_registry(program_id, signer_registry_info)?;
    if !market.paused
        || market.total_position_collateral_locked != 0
        || market.mint_accounting.total_issued != 0
        || market.mint_accounting.total_consumed != 0
        || market.mint_accounting.total_burned != 0
        || market.collateral_mint != *settlement_mint_info.key
        || month.settlement_record.is_some()
        || matches!(month.phase, OraclePhase::Settled | OraclePhase::Closed)
    {
        return Err(VaultError::InvalidWriterSettlementGroup.into());
    }
    let (expected_group, bump) = derive_writer_settlement_group_pda(
        program_id,
        &market.instrument.underlying_id,
        market.instrument.expiry_ts,
        settlement_mint_info.key,
    );
    if *group_info.key != expected_group {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, group_info)?;
    create_program_account(
        admin_info,
        group_info,
        system_program_info,
        program_id,
        WriterSettlementGroupV1::LEN,
        &[
            crate::constants::WRITER_SETTLEMENT_GROUP_PDA_SEED,
            &market.instrument.underlying_id,
            &market.instrument.expiry_ts.to_le_bytes(),
            settlement_mint_info.key.as_ref(),
            &[bump],
        ],
    )?;
    let sleeve = derive_writer_sleeve_pda(program_id, group_info.key).0;
    let group = WriterSettlementGroupV1 {
        is_initialized: true,
        bump,
        account_discriminator: WriterSettlementGroupV1::ACCOUNT_DISCRIMINATOR,
        account_version: WriterSettlementGroupV1::ACCOUNT_VERSION,
        underlying_id: market.instrument.underlying_id,
        expiry_ts: market.instrument.expiry_ts,
        settlement_mint: *settlement_mint_info.key,
        anchor_market: *market_info.key,
        anchor_oracle_month: *month_info.key,
        oracle_methodology_version: crate::constants::WRITER_ORACLE_METHODOLOGY_VERSION,
        product_manifest_root: [0; 32],
        coverage_manifest_hash: [0; 32],
        recipe_hash: [0; 32],
        settlement_source_digest: [0; 32],
        active_weight_manifest_hash: [0; 32],
        security_cap_atoms: 0,
        signer_registry: *signer_registry_info.key,
        signer_set: Pubkey::default(),
        signer_set_version: 0,
        signer_set_hash: [0; 32],
        settlement_ts: market.instrument.expiry_ts,
        settlement_price_atomic: 0,
        final_settlement_commitment: [0; 32],
        submitted_by: Pubkey::default(),
        finalized_slot: 0,
        sleeve,
        status: WriterSettlementGroupStatus::Anchored,
        series_count: 0,
        full_collateral_capacity: false,
        shared_reserve: false,
        reserved: [0; 4],
        last_updated_slot: Clock::get()?.slot,
    };
    let _ = signer_registry;
    store_state(group_info, &group)
}

pub(in crate::processor) fn process_initialize_sleeve(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    participation_start_ts: u64,
) -> ProgramResult {
    if accounts.len() != 10 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let registry_info = &accounts[2];
    let group_info = &accounts[3];
    let sleeve_info = &accounts[4];
    let book_info = &accounts[5];
    let sleeve_vault_info = &accounts[6];
    let settlement_mint_info = &accounts[7];
    let token_program_info = &accounts[8];
    let system_program_info = &accounts[9];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id()
        || *system_program_info.key != system_program::id()
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key || config.usdc_mint != *settlement_mint_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let registry = load_writer_policy_registry(program_id, registry_info, config_info.key)?;
    let mut group = load_writer_settlement_group(program_id, group_info)?;
    if participation_start_ts == 0
        || participation_start_ts >= group.expiry_ts
        || current_unix_timestamp()? >= group.expiry_ts
        || group.status != WriterSettlementGroupStatus::Anchored
        || group.settlement_mint != *settlement_mint_info.key
        || group.sleeve != *sleeve_info.key
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let (expected_sleeve, sleeve_bump) = derive_writer_sleeve_pda(program_id, group_info.key);
    let (expected_book, book_bump) = derive_writer_series_book_pda(program_id, sleeve_info.key);
    let (expected_vault, vault_bump) =
        derive_writer_sleeve_usdc_vault_pda(program_id, sleeve_info.key);
    if *sleeve_info.key != expected_sleeve
        || *book_info.key != expected_book
        || *sleeve_vault_info.key != expected_vault
    {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, sleeve_info)?;
    validate_create_only_program_account_target(program_id, book_info)?;
    create_program_account(
        admin_info,
        sleeve_info,
        system_program_info,
        program_id,
        WriterSleeveV1::LEN,
        &[
            crate::constants::WRITER_SLEEVE_PDA_SEED,
            group_info.key.as_ref(),
            &[sleeve_bump],
        ],
    )?;
    create_program_account(
        admin_info,
        book_info,
        system_program_info,
        program_id,
        WriterSeriesBookV1::LEN,
        &[
            crate::constants::WRITER_SERIES_BOOK_PDA_SEED,
            sleeve_info.key.as_ref(),
            &[book_bump],
        ],
    )?;
    create_classic_token_pda(
        program_id,
        admin_info,
        sleeve_vault_info,
        settlement_mint_info,
        sleeve_info.key,
        token_program_info,
        system_program_info,
        &[
            crate::constants::WRITER_SLEEVE_USDC_VAULT_PDA_SEED,
            sleeve_info.key.as_ref(),
            &[vault_bump],
        ],
    )?;
    validate_vault_token_account(sleeve_vault_info, settlement_mint_info.key, sleeve_info.key)?;
    let slot = Clock::get()?.slot;
    let mut book = WriterSeriesBookV1 {
        is_initialized: true,
        bump: book_bump,
        account_discriminator: WriterSeriesBookV1::ACCOUNT_DISCRIMINATOR,
        account_version: WriterSeriesBookV1::ACCOUNT_VERSION,
        sleeve: *sleeve_info.key,
        settlement_group: *group_info.key,
        series_count: 0,
        max_series: crate::constants::WRITER_MAX_LIVE_SERIES as u8,
        frozen: false,
        reserved: [0; 7],
        book_digest: [0; 32],
        last_updated_slot: slot,
        records: vec![WriterSeriesRecordV1::EMPTY; crate::constants::WRITER_MAX_LIVE_SERIES]
            .into_boxed_slice()
            .try_into()
            .map_err(|_| VaultError::ArithmeticOverflow)?,
        individual: Default::default(),
    };
    book.book_digest = writer_book_digest(&book);
    let sleeve = WriterSleeveV1 {
        is_initialized: true,
        bump: sleeve_bump,
        account_discriminator: WriterSleeveV1::ACCOUNT_DISCRIMINATOR,
        account_version: WriterSleeveV1::ACCOUNT_VERSION,
        vault_config: *config_info.key,
        underlying_id: group.underlying_id,
        expiry_ts: group.expiry_ts,
        settlement_mint: *settlement_mint_info.key,
        settlement_group: *group_info.key,
        series_book: *book_info.key,
        usdc_vault: *sleeve_vault_info.key,
        policy_registry: *registry_info.key,
        policy_snapshot: Pubkey::default(),
        policy_version: 0,
        policy_hash: [0; 32],
        scenario_set_hash: [0; 32],
        risk_limit_hash: [0; 32],
        writer_principal_atoms: 0,
        locked_primary_premium_atoms: 0,
        accounted_asset_atoms: 0,
        exact_reserve_atoms: 0,
        upper_tail_reserve_atoms: 0,
        lower_tail_reserve_atoms: 0,
        security_exposure_atoms: 0,
        long_liability_initial_atoms: 0,
        long_liability_remaining_atoms: 0,
        writer_residual_initial_atoms: 0,
        writer_residual_remaining_atoms: 0,
        settlement_principal_atoms: 0,
        unclaimed_principal_atoms: 0,
        stranded_surplus_atoms: 0,
        operational_buffer_atoms: 0,
        series_count: 0,
        status: WriterSleeveStatus::Draft,
        security_mode: WriterSecurityMode::GrossExternalMaxPayout,
        settlement_finalized_slot: 0,
        last_updated_slot: slot,
        capital_seconds: 0,
        maximum_contribution_duration: 0,
        participation_start_ts,
    };
    group.last_updated_slot = slot;
    let _ = registry;
    store_state(book_info, &book)?;
    store_state(sleeve_info, &sleeve)?;
    store_state(group_info, &group)
}

pub(in crate::processor) fn process_register_series(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let market_info = &accounts[5];
    let contract_mint_info = &accounts[6];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let WriterBookContext {
        mut group,
        mut sleeve,
        mut book,
    } = load_writer_book_context(program_id, sleeve_info, group_info, book_info)?;
    if group.sleeve != *sleeve_info.key
        || sleeve.series_book != *book_info.key
        || sleeve.status != WriterSleeveStatus::Draft
        || group.status != WriterSettlementGroupStatus::Anchored
        || book.frozen
        || usize::from(book.series_count) >= crate::constants::WRITER_MAX_LIVE_SERIES
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let mut market = load_valid_market(program_id, market_info)?;
    let mint = validate_canonical_market_mint(market_info, &mut market, contract_mint_info, 0)?;
    let expected_max_payout = match market.instrument.kind {
        crate::state::OptionKind::CallSpread => market
            .instrument
            .cap_price
            .checked_sub(market.instrument.strike_price),
        crate::state::OptionKind::PutSpread => market
            .instrument
            .strike_price
            .checked_sub(market.instrument.cap_price),
    }
    .ok_or(VaultError::InvalidMarketConfig)?;
    if !market.paused
        || market.market_id == [0; 32]
        || market.instrument.underlying_id != group.underlying_id
        || market.instrument.expiry_ts != group.expiry_ts
        || market.collateral_mint != group.settlement_mint
        || market.instrument.contract_size != MarketMintAccounting::CANONICAL_ATOMIC_SCALE
        || market.instrument.max_payout_per_contract != expected_max_payout
        || market.total_position_collateral_locked != 0
        || market.mint_accounting.total_issued != 0
        || market.mint_accounting.total_consumed != 0
        || market.mint_accounting.total_burned != 0
        || mint.supply != 0
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let index = usize::from(book.series_count);
    if index != 0 && book.records[index - 1].series_id >= market.market_id {
        return Err(VaultError::InvalidWriterSeriesOrder.into());
    }
    let retirement =
        derive_writer_retirement_custody_pda(program_id, sleeve_info.key, market_info.key).0;
    let record = WriterSeriesRecordV1 {
        active: true,
        option_kind: market.instrument.kind,
        custody_status: WriterSeriesCustodyStatus::Absent,
        settlement_status: WriterSeriesSettlementStatus::Open,
        reserved: [0; 4],
        series_id: market.market_id,
        market: *market_info.key,
        contract_mint: *contract_mint_info.key,
        retirement_custody: retirement,
        strike_price_atomic: market.instrument.strike_price,
        cap_or_floor_price_atomic: market.instrument.cap_price,
        contract_size_atoms: market.instrument.contract_size,
        max_payout_per_contract_atoms: market.instrument.max_payout_per_contract,
        total_physical_supply_atoms: 0,
        issuer_controlled_atoms: 0,
        external_open_interest_atoms: 0,
        primary_premium_collected_atoms: 0,
        settlement_external_oi_snapshot_atoms: 0,
        settlement_liability_initial_atoms: 0,
        settlement_liability_remaining_atoms: 0,
        payoff_digest: writer_payoff_digest(
            group_info.key,
            &market,
            market_info.key,
            contract_mint_info.key,
        ),
    };
    let candidate_series = WriterSeries {
        kind: record.option_kind,
        strike_price_atomic: record.strike_price_atomic,
        cap_price_atomic: record.cap_or_floor_price_atomic,
        contract_size_atoms: record.contract_size_atoms,
        max_payout_per_contract_atoms: record.max_payout_per_contract_atoms,
        external_oi_atoms: 0,
    };
    for existing in &book.records[..index] {
        let existing_series = WriterSeries {
            kind: existing.option_kind,
            strike_price_atomic: existing.strike_price_atomic,
            cap_price_atomic: existing.cap_or_floor_price_atomic,
            contract_size_atoms: existing.contract_size_atoms,
            max_payout_per_contract_atoms: existing.max_payout_per_contract_atoms,
            external_oi_atoms: 0,
        };
        if candidate_series.same_instrument(&existing_series) {
            return Err(VaultError::InvalidWriterSeriesBook.into());
        }
    }
    book.records[index] = record;
    book.series_count = book
        .series_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.series_count = book.series_count;
    group.series_count = book.series_count;
    let slot = Clock::get()?.slot;
    book.last_updated_slot = slot;
    book.book_digest = writer_book_digest(&book);
    sleeve.last_updated_slot = slot;
    group.last_updated_slot = slot;
    store_state(book_info, &book)?;
    store_state(sleeve_info, &sleeve)?;
    store_state(group_info, &group)
}
