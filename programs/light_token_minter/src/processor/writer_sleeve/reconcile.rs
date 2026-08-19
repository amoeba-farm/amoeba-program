use super::*;
use crate::instruction::{CleanupWriterCustodyV1Params, ReconcileWriterSupplyV1Params};

const RECONCILE_WRITER_SUPPLY_ACCOUNT_COUNT: usize = 15;
const CLEANUP_WRITER_CUSTODY_ACCOUNT_COUNT: usize = 8;
const RECONCILE_TARGET_SERIES: u8 = 0;
const RECONCILE_TARGET_FLAT: u8 = 1;

fn optional_canonical_token_amount(
    info: &AccountInfo,
    expected_key: &Pubkey,
    expected_mint: &Pubkey,
    expected_owner: &Pubkey,
) -> Result<u64, ProgramError> {
    if info.key != expected_key {
        return Err(VaultError::InvalidPda.into());
    }
    if info.owner == &system_program::id() && !info.executable && info.data_len() == 0 {
        return Ok(0);
    }
    validate_vault_token_account(info, expected_mint, expected_owner)?;
    Ok(validate_token_account(info)?.amount)
}

fn cumulative_allocation_delta(
    initial_supply: u64,
    remaining_before: u64,
    consumed_now: u64,
    initial_liability: u64,
) -> Result<u64, ProgramError> {
    crate::writer_sleeve_math::cumulative_allocation_delta(
        initial_supply,
        remaining_before,
        consumed_now,
        initial_liability,
    )
    .map_err(|error| match error {
        WriterMathError::InvalidCloseAmount => VaultError::WriterSupplyMismatch.into(),
        _ => VaultError::ArithmeticOverflow.into(),
    })
}

fn apply_long_forfeiture(
    sleeve: &mut WriterSleeveV1,
    record: &mut WriterSeriesRecordV1,
    burned_external_atoms: u64,
) -> ProgramResult {
    if burned_external_atoms == 0 || sleeve.status != WriterSleeveStatus::SettlementFinalized {
        return Ok(());
    }
    let allocation = cumulative_allocation_delta(
        record.settlement_external_oi_snapshot_atoms,
        record.external_open_interest_atoms,
        burned_external_atoms,
        record.settlement_liability_initial_atoms,
    )?;
    if allocation > record.settlement_liability_remaining_atoms
        || allocation > sleeve.long_liability_remaining_atoms
        || allocation > sleeve.accounted_asset_atoms
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    record.settlement_liability_remaining_atoms = record
        .settlement_liability_remaining_atoms
        .checked_sub(allocation)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.long_liability_remaining_atoms = sleeve
        .long_liability_remaining_atoms
        .checked_sub(allocation)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.accounted_asset_atoms = sleeve
        .accounted_asset_atoms
        .checked_sub(allocation)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.stranded_surplus_atoms = sleeve
        .stranded_surplus_atoms
        .checked_add(allocation)
        .ok_or(VaultError::ArithmeticOverflow)?;
    Ok(())
}

fn apply_flat_direct_burn(sleeve: &mut WriterSleeveV1, burned_atoms: u64) -> ProgramResult {
    if burned_atoms == 0 {
        return Ok(());
    }
    if burned_atoms > sleeve.flat_par_supply_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    if sleeve.status == WriterSleeveStatus::SettlementFinalized {
        let allocation = cumulative_allocation_delta(
            sleeve.flat_supply_snapshot_atoms,
            sleeve.flat_claim_supply_remaining_atoms,
            burned_atoms,
            sleeve.flat_residual_initial_atoms,
        )?;
        if allocation > sleeve.flat_residual_remaining_atoms
            || allocation > sleeve.accounted_asset_atoms
        {
            return Err(VaultError::WriterSolvencyViolation.into());
        }
        sleeve.flat_claim_supply_remaining_atoms = sleeve
            .flat_claim_supply_remaining_atoms
            .checked_sub(burned_atoms)
            .ok_or(VaultError::ArithmeticOverflow)?;
        sleeve.flat_residual_remaining_atoms = sleeve
            .flat_residual_remaining_atoms
            .checked_sub(allocation)
            .ok_or(VaultError::ArithmeticOverflow)?;
        sleeve.accounted_asset_atoms = sleeve
            .accounted_asset_atoms
            .checked_sub(allocation)
            .ok_or(VaultError::ArithmeticOverflow)?;
        sleeve.stranded_surplus_atoms = sleeve
            .stranded_surplus_atoms
            .checked_add(allocation)
            .ok_or(VaultError::ArithmeticOverflow)?;
    } else {
        if burned_atoms > sleeve.writer_principal_atoms {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        sleeve.writer_principal_atoms = sleeve
            .writer_principal_atoms
            .checked_sub(burned_atoms)
            .ok_or(VaultError::ArithmeticOverflow)?;
    }
    sleeve.flat_par_supply_atoms = sleeve
        .flat_par_supply_atoms
        .checked_sub(burned_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    Ok(())
}

pub(super) fn process_reconcile_writer_supply(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ReconcileWriterSupplyV1Params,
) -> ProgramResult {
    if accounts.len() != RECONCILE_WRITER_SUPPLY_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let snapshot_info = &accounts[5];
    let target_authority_info = &accounts[6];
    let target_mint_info = &accounts[7];
    let staging_info = &accounts[8];
    let custody_info = &accounts[9];
    let interface_info = &accounts[10];
    let light_program_info = &accounts[11];
    let cpi_authority_info = &accounts[12];
    let token_program_info = &accounts[13];
    let system_program_info = &accounts[14];
    if !cranker_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_writer_program_accounts(
        light_program_info,
        cpi_authority_info,
        token_program_info,
        system_program_info,
    )?;
    let config = load_canonical_vault_config(program_id, config_info)?;
    let WriterPolicyContext {
        group,
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
        || sleeve.policy_snapshot != *snapshot_info.key
        || sleeve.active_auction.is_some()
        || sleeve.active_close_request.is_some()
        || matches!(
            sleeve.status,
            WriterSleeveStatus::Draft
                | WriterSleeveStatus::PolicyFrozen
                | WriterSleeveStatus::Funding
                | WriterSleeveStatus::CloseStaging
                | WriterSleeveStatus::Closed
        )
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }

    match params.target_kind {
        RECONCILE_TARGET_SERIES => {
            let index = usize::from(params.series_index);
            if index >= usize::from(book.series_count) {
                return Err(VaultError::InvalidWriterSeriesBook.into());
            }
            let record = &mut book.records[index];
            if record.market != *target_authority_info.key
                || record.contract_mint != *target_mint_info.key
            {
                return Err(VaultError::InvalidWriterSeriesBook.into());
            }
            let mut market = load_valid_market(program_id, target_authority_info)?;
            if market.market_id != record.series_id
                || market.instrument.underlying_id != group.underlying_id
                || market.instrument.expiry_ts != group.expiry_ts
                || market.long_contract_mint != Some(*target_mint_info.key)
            {
                return Err(VaultError::InvalidWriterSeriesBook.into());
            }
            let mint = validate_canonical_market_mint(
                target_authority_info,
                &mut market,
                target_mint_info,
                0,
            )?;
            validate_spl_interface_account(target_mint_info.key, interface_info)?;
            let expected_staging =
                derive_contract_mint_staging_pda(program_id, target_authority_info.key).0;
            let staging_atoms = optional_canonical_token_amount(
                staging_info,
                &expected_staging,
                target_mint_info.key,
                target_authority_info.key,
            )?;
            let retirement_atoms = optional_canonical_token_amount(
                custody_info,
                &record.retirement_custody,
                target_mint_info.key,
                sleeve_info.key,
            )?;
            let observed_issuer = staging_atoms
                .checked_add(retirement_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            if mint.supply > record.total_physical_supply_atoms
                || observed_issuer < record.issuer_controlled_atoms
                || observed_issuer > mint.supply
            {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            let new_external = mint
                .supply
                .checked_sub(observed_issuer)
                .ok_or(VaultError::ArithmeticOverflow)?;
            if new_external > record.external_open_interest_atoms {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            let external_decrease = record
                .external_open_interest_atoms
                .checked_sub(new_external)
                .ok_or(VaultError::ArithmeticOverflow)?;
            let physical_decrease = record
                .total_physical_supply_atoms
                .checked_sub(mint.supply)
                .ok_or(VaultError::ArithmeticOverflow)?;
            let custody_increase = observed_issuer
                .checked_sub(record.issuer_controlled_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            if external_decrease
                != physical_decrease
                    .checked_add(custody_increase)
                    .ok_or(VaultError::ArithmeticOverflow)?
                || market_outstanding_contract_amount(&market)?
                    != record.external_open_interest_atoms
                || market
                    .mint_accounting
                    .total_issued
                    .checked_sub(market.mint_accounting.total_burned)
                    != Some(record.total_physical_supply_atoms)
            {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            // Both an external direct burn and an unsolicited transition into irrevocable
            // issuer custody consume a post-settlement holder allocation without paying anyone.
            apply_long_forfeiture(&mut sleeve, record, external_decrease)?;
            market.mint_accounting.total_consumed = market
                .mint_accounting
                .total_consumed
                .checked_add(external_decrease)
                .ok_or(VaultError::ArithmeticOverflow)?;
            market.mint_accounting.total_burned = market
                .mint_accounting
                .total_burned
                .checked_add(physical_decrease)
                .ok_or(VaultError::ArithmeticOverflow)?;
            record.total_physical_supply_atoms = mint.supply;
            record.issuer_controlled_atoms = observed_issuer;
            record.external_open_interest_atoms = new_external;
            record.custody_status = if observed_issuer == 0 {
                WriterSeriesCustodyStatus::Absent
            } else {
                WriterSeriesCustodyStatus::Open
            };
            let _ = validate_canonical_market_mint(
                target_authority_info,
                &mut market,
                target_mint_info,
                0,
            )?;
            store_state(target_authority_info, &market)?;
        }
        RECONCILE_TARGET_FLAT => {
            if params.series_index != 0
                || *target_authority_info.key != *sleeve_info.key
                || *target_mint_info.key != sleeve.flat_mint
                || *staging_info.key != sleeve.flat_staging
                || *custody_info.key != sleeve.flat_burn_custody
                || *interface_info.key != sleeve.flat_spl_interface
            {
                return Err(VaultError::InvalidAccountList.into());
            }
            let mint = validate_mint_account(target_mint_info, token_program_info.key)?;
            if !mint.is_initialized
                || mint.decimals != MarketMintAccounting::CANONICAL_DECIMALS
                || mint.mint_authority != COption::Some(*sleeve_info.key)
                || mint.freeze_authority != COption::None
                || mint.supply > sleeve.flat_par_supply_atoms
            {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            validate_spl_interface_account(target_mint_info.key, interface_info)?;
            let staging_atoms = optional_canonical_token_amount(
                staging_info,
                &sleeve.flat_staging,
                target_mint_info.key,
                sleeve_info.key,
            )?;
            let burn_atoms = optional_canonical_token_amount(
                custody_info,
                &sleeve.flat_burn_custody,
                target_mint_info.key,
                sleeve_info.key,
            )?;
            if staging_atoms != 0 || burn_atoms != 0 {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            let direct_burn = sleeve
                .flat_par_supply_atoms
                .checked_sub(mint.supply)
                .ok_or(VaultError::ArithmeticOverflow)?;
            apply_flat_direct_burn(&mut sleeve, direct_burn)?;
        }
        _ => return Err(VaultError::InvalidInstructionData.into()),
    }

    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = Clock::get()?.slot;
    if sleeve.status == WriterSleeveStatus::SettlementFinalized {
        let partition_remaining = sleeve
            .long_liability_remaining_atoms
            .checked_add(sleeve.flat_residual_remaining_atoms)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if sleeve.accounted_asset_atoms != partition_remaining {
            return Err(VaultError::WriterSolvencyViolation.into());
        }
        // Settlement removes price uncertainty. The live reserve is now the frozen long-class
        // ledger; tail and oracle-security measures no longer have an unsettled exposure to
        // recompute. Re-running the pre-settlement envelope here would overwrite the partition.
        sleeve.exact_reserve_atoms = sleeve.long_liability_remaining_atoms;
        sleeve.lower_tail_reserve_atoms = 0;
        sleeve.upper_tail_reserve_atoms = 0;
        sleeve.security_exposure_atoms = 0;
    } else {
        recompute_writer_metrics(
            &mut sleeve,
            &book,
            &snapshot,
            Some(group.security_cap_atoms),
            true,
        )?;
    }
    sleeve.last_updated_slot = book.last_updated_slot;
    if config.usdc_mint != sleeve.settlement_mint {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    store_state(book_info, &book)?;
    store_state(sleeve_info, &sleeve)
}

pub(super) fn process_cleanup_writer_custody(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: CleanupWriterCustodyV1Params,
) -> ProgramResult {
    if accounts.len() != CLEANUP_WRITER_CUSTODY_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let sleeve_info = &accounts[1];
    let book_info = &accounts[2];
    let market_info = &accounts[3];
    let mint_info = &accounts[4];
    let custody_info = &accounts[5];
    let token_program_info = &accounts[6];
    let system_program_info = &accounts[7];
    if !cranker_info.is_signer || !cranker_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id()
        || *system_program_info.key != system_program::id()
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut sleeve = load_writer_sleeve_without_group_meta(program_id, sleeve_info)?;
    let mut book = load_writer_series_book(
        program_id,
        book_info,
        sleeve_info.key,
        &sleeve.settlement_group,
    )?;
    if sleeve.series_book != *book_info.key
        || sleeve.active_close_request.is_some()
        || sleeve.active_auction.is_some()
        || matches!(
            sleeve.status,
            WriterSleeveStatus::Draft | WriterSleeveStatus::Closed
        )
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let index = usize::from(params.series_index);
    if index >= usize::from(book.series_count) {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let record = &mut book.records[index];
    if record.market != *market_info.key
        || record.contract_mint != *mint_info.key
        || record.retirement_custody != *custody_info.key
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mut market = load_valid_market(program_id, market_info)?;
    let mint_before = validate_canonical_market_mint(market_info, &mut market, mint_info, 0)?;
    validate_vault_token_account(custody_info, mint_info.key, sleeve_info.key)?;
    let amount = validate_token_account(custody_info)?.amount;
    if amount == 0 || amount > record.issuer_controlled_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let sleeve_bump = [sleeve.bump];
    let sleeve_signer_seeds = writer_sleeve_signer_seeds(&sleeve.settlement_group, &sleeve_bump);
    invoke_token_burn_checked(
        token_program_info,
        custody_info,
        mint_info,
        sleeve_info,
        amount,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &[&sleeve_signer_seeds],
    )?;
    market.mint_accounting.total_burned = market
        .mint_accounting
        .total_burned
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    record.total_physical_supply_atoms = record
        .total_physical_supply_atoms
        .checked_sub(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    record.issuer_controlled_atoms = record
        .issuer_controlled_atoms
        .checked_sub(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if record.issuer_controlled_atoms == 0 {
        record.custody_status = WriterSeriesCustodyStatus::Closed;
    }
    let mint_after = validate_mint_account(mint_info, token_program_info.key)?;
    if mint_before.supply.checked_sub(mint_after.supply) != Some(amount)
        || market_outstanding_contract_amount(&market)? != record.external_open_interest_atoms
        || market
            .mint_accounting
            .total_issued
            .checked_sub(market.mint_accounting.total_burned)
            != Some(record.total_physical_supply_atoms)
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    funding::close_sleeve_token_custody(
        &sleeve,
        sleeve_info,
        custody_info,
        cranker_info,
        token_program_info,
    )?;
    let slot = Clock::get()?.slot;
    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = slot;
    sleeve.last_updated_slot = slot;
    store_state(market_info, &market)?;
    store_state(book_info, &book)?;
    store_state(sleeve_info, &sleeve)
}
