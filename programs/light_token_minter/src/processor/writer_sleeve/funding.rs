use super::*;
use crate::instruction::WriterAmountV1Params;

const DEPOSIT_WRITER_PRINCIPAL_ACCOUNT_COUNT: usize = 17;
const WITHDRAW_WRITER_PRINCIPAL_ACCOUNT_COUNT: usize = 14;

#[allow(clippy::too_many_arguments)]
pub(super) fn load_or_create_sleeve_token_custody<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    sleeve_info: &AccountInfo<'a>,
    custody_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    token_program_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    seed: &'static [u8],
    expected: Pubkey,
    bump: u8,
) -> Result<TokenAccount, ProgramError> {
    if *custody_info.key != expected {
        return Err(VaultError::InvalidPda.into());
    }
    if custody_info.owner == token_program_info.key {
        validate_vault_token_account(custody_info, mint_info.key, sleeve_info.key)?;
        return validate_token_account(custody_info);
    }
    validate_create_only_program_account_target(program_id, custody_info)?;
    create_program_account(
        payer_info,
        custody_info,
        system_program_info,
        token_program_info.key,
        TokenAccount::LEN,
        &[seed, sleeve_info.key.as_ref(), &[bump]],
    )?;
    invoke_token_initialize_account3(token_program_info, custody_info, mint_info, sleeve_info.key)?;
    validate_vault_token_account(custody_info, mint_info.key, sleeve_info.key)?;
    validate_token_account(custody_info)
}

pub(super) fn close_sleeve_token_custody<'a>(
    sleeve: &WriterSleeveV1,
    sleeve_info: &AccountInfo<'a>,
    custody_info: &AccountInfo<'a>,
    recipient_info: &AccountInfo<'a>,
    token_program_info: &AccountInfo<'a>,
) -> ProgramResult {
    if validate_token_account(custody_info)?.amount != 0 {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let bump = [sleeve.bump];
    let signer_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::WRITER_SLEEVE_PDA_SEED,
        sleeve.settlement_group.as_ref(),
        &bump,
    ];
    invoke_token_close_account(
        token_program_info,
        custody_info,
        recipient_info,
        sleeve_info,
        &[signer_seeds],
    )
}

pub(super) fn process_deposit_writer_principal(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: WriterAmountV1Params,
) -> ProgramResult {
    if accounts.len() != DEPOSIT_WRITER_PRINCIPAL_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    if params.amount_atoms == 0 {
        return Err(VaultError::AmountMustBePositive.into());
    }
    let depositor_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let snapshot_info = &accounts[3];
    let sleeve_vault_info = &accounts[4];
    let source_usdc_info = &accounts[5];
    let settlement_mint_info = &accounts[6];
    let flat_mint_info = &accounts[7];
    let flat_staging_info = &accounts[8];
    let flat_destination_info = &accounts[9];
    let light_program_info = &accounts[10];
    let cpi_authority_info = &accounts[11];
    let flat_interface_info = &accounts[12];
    let token_program_info = &accounts[13];
    let system_program_info = &accounts[14];
    let compressible_config_info = &accounts[15];
    let rent_sponsor_info = &accounts[16];
    if !depositor_info.is_signer || !depositor_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_writer_program_accounts(
        light_program_info,
        cpi_authority_info,
        token_program_info,
        system_program_info,
    )?;
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.paused || config.usdc_mint != *settlement_mint_info.key {
        return Err(VaultError::ContractPaused.into());
    }
    let mut sleeve = load_writer_sleeve_without_group_meta(program_id, sleeve_info)?;
    let snapshot = load_writer_policy_snapshot(
        program_id,
        snapshot_info,
        sleeve_info.key,
        &sleeve.policy_registry,
        sleeve.policy_version,
    )?;
    if sleeve.status != WriterSleeveStatus::Funding
        || sleeve.policy_snapshot != *snapshot_info.key
        || sleeve.usdc_vault != *sleeve_vault_info.key
        || sleeve.settlement_mint != *settlement_mint_info.key
        || sleeve.flat_mint != *flat_mint_info.key
        || sleeve.flat_spl_interface != *flat_interface_info.key
        || snapshot.policy_hash != sleeve.policy_hash
        || sleeve.locked_primary_premium_atoms != 0
        || sleeve.active_auction.is_some()
        || sleeve.active_close_request.is_some()
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    validate_collateral_mint_account(settlement_mint_info, token_program_info.key)?;
    validate_vault_token_account(sleeve_vault_info, settlement_mint_info.key, sleeve_info.key)?;
    let source = validate_token_account(source_usdc_info)?;
    if source.owner != *depositor_info.key
        || source.mint != *settlement_mint_info.key
        || source.state != AccountState::Initialized
        || source.amount < params.amount_atoms
    {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    let flat_mint = validate_writer_flat_mint(sleeve_info, &sleeve, flat_mint_info)?;
    if flat_mint.supply != sleeve.flat_par_supply_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let destination_before = load_or_create_light_associated_token_account(
        depositor_info,
        depositor_info,
        flat_mint_info,
        flat_destination_info,
        light_program_info,
        compressible_config_info,
        rent_sponsor_info,
        system_program_info,
    )?;
    let interface_before = validate_token_account(flat_interface_info)?;
    if interface_before.mint != *flat_mint_info.key || interface_before.owner != cpi_authority() {
        return Err(VaultError::InvalidSplInterfaceAccount.into());
    }
    let (expected_staging, staging_bump) =
        derive_writer_flat_staging_pda(program_id, sleeve_info.key);
    let staging_before = load_or_create_sleeve_token_custody(
        program_id,
        depositor_info,
        sleeve_info,
        flat_staging_info,
        flat_mint_info,
        token_program_info,
        system_program_info,
        crate::constants::WRITER_FLAT_STAGING_PDA_SEED,
        expected_staging,
        staging_bump,
    )?;
    if staging_before.amount != 0 {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let vault_before = validate_token_account(sleeve_vault_info)?.amount;
    if vault_before < sleeve.accounted_asset_atoms {
        return Err(VaultError::WriterSolvencyViolation.into());
    }

    invoke_token_transfer_checked(
        token_program_info,
        source_usdc_info,
        settlement_mint_info,
        sleeve_vault_info,
        depositor_info,
        params.amount_atoms,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &[],
    )?;
    let sleeve_bump = [sleeve.bump];
    let sleeve_signer_seeds = writer_sleeve_signer_seeds(&sleeve.settlement_group, &sleeve_bump);
    invoke_token_mint_to_checked(
        token_program_info,
        flat_mint_info,
        flat_staging_info,
        sleeve_info,
        params.amount_atoms,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &[&sleeve_signer_seeds],
    )?;
    invoke_light_token_account_transfer_with_signer_seeds(
        params.amount_atoms,
        MarketMintAccounting::CANONICAL_DECIMALS,
        light_program_info,
        cpi_authority_info,
        depositor_info,
        flat_staging_info,
        flat_destination_info,
        sleeve_info,
        flat_mint_info,
        flat_interface_info,
        token_program_info,
        system_program_info,
        &[&sleeve_signer_seeds],
    )?;

    let vault_after = validate_token_account(sleeve_vault_info)?.amount;
    let mint_after = validate_mint_account(flat_mint_info, token_program_info.key)?;
    let destination_after = load_canonical_light_token_account(
        flat_destination_info,
        depositor_info.key,
        flat_mint_info.key,
    )?;
    let interface_after = validate_token_account(flat_interface_info)?;
    if vault_after.checked_sub(vault_before) != Some(params.amount_atoms)
        || mint_after.supply.checked_sub(flat_mint.supply) != Some(params.amount_atoms)
        || destination_after
            .amount
            .checked_sub(destination_before.amount)
            != Some(params.amount_atoms)
        || interface_after.amount.checked_sub(interface_before.amount) != Some(params.amount_atoms)
        || validate_token_account(flat_staging_info)?.amount != 0
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    close_sleeve_token_custody(
        &sleeve,
        sleeve_info,
        flat_staging_info,
        depositor_info,
        token_program_info,
    )?;
    sleeve.writer_principal_atoms = sleeve
        .writer_principal_atoms
        .checked_add(params.amount_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.flat_par_supply_atoms = sleeve
        .flat_par_supply_atoms
        .checked_add(params.amount_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.accounted_asset_atoms = sleeve
        .accounted_asset_atoms
        .checked_add(params.amount_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.last_updated_slot = Clock::get()?.slot;
    store_state(sleeve_info, &sleeve)
}

pub(super) fn process_withdraw_writer_principal(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: WriterAmountV1Params,
) -> ProgramResult {
    if accounts.len() != WITHDRAW_WRITER_PRINCIPAL_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    if params.amount_atoms == 0 {
        return Err(VaultError::AmountMustBePositive.into());
    }
    let depositor_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let sleeve_vault_info = &accounts[3];
    let destination_usdc_info = &accounts[4];
    let settlement_mint_info = &accounts[5];
    let flat_mint_info = &accounts[6];
    let flat_source_info = &accounts[7];
    let flat_burn_info = &accounts[8];
    let flat_interface_info = &accounts[9];
    let light_program_info = &accounts[10];
    let cpi_authority_info = &accounts[11];
    let token_program_info = &accounts[12];
    let system_program_info = &accounts[13];
    if !depositor_info.is_signer || !depositor_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_writer_program_accounts(
        light_program_info,
        cpi_authority_info,
        token_program_info,
        system_program_info,
    )?;
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.paused || config.usdc_mint != *settlement_mint_info.key {
        return Err(VaultError::ContractPaused.into());
    }
    let mut sleeve = load_writer_sleeve_without_group_meta(program_id, sleeve_info)?;
    if sleeve.status != WriterSleeveStatus::Funding
        || sleeve.usdc_vault != *sleeve_vault_info.key
        || sleeve.settlement_mint != *settlement_mint_info.key
        || sleeve.flat_mint != *flat_mint_info.key
        || sleeve.flat_spl_interface != *flat_interface_info.key
        || sleeve.locked_primary_premium_atoms != 0
        || sleeve.exact_reserve_atoms != 0
        || sleeve.security_exposure_atoms != 0
        || sleeve.active_auction.is_some()
        || sleeve.active_close_request.is_some()
        || params.amount_atoms > sleeve.writer_principal_atoms
        || params.amount_atoms > sleeve.flat_par_supply_atoms
        || params.amount_atoms > sleeve.accounted_asset_atoms
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    validate_collateral_mint_account(settlement_mint_info, token_program_info.key)?;
    validate_vault_token_account(sleeve_vault_info, settlement_mint_info.key, sleeve_info.key)?;
    let destination = validate_token_account(destination_usdc_info)?;
    if destination.owner != *depositor_info.key
        || destination.mint != *settlement_mint_info.key
        || destination.state != AccountState::Initialized
    {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    validate_light_associated_token_account(
        depositor_info.key,
        flat_mint_info.key,
        flat_source_info,
    )?;
    let source_before = load_canonical_light_token_account(
        flat_source_info,
        depositor_info.key,
        flat_mint_info.key,
    )?;
    if source_before.amount < params.amount_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let flat_mint = validate_writer_flat_mint(sleeve_info, &sleeve, flat_mint_info)?;
    if flat_mint.supply != sleeve.flat_par_supply_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let (expected_burn, burn_bump) =
        derive_writer_flat_burn_custody_pda(program_id, sleeve_info.key);
    let burn_before = load_or_create_sleeve_token_custody(
        program_id,
        depositor_info,
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
    let vault_before = validate_token_account(sleeve_vault_info)?.amount;
    let destination_before = destination.amount;
    if vault_before < sleeve.accounted_asset_atoms || vault_before < params.amount_atoms {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    invoke_light_token_account_transfer(
        params.amount_atoms,
        MarketMintAccounting::CANONICAL_DECIMALS,
        light_program_info,
        cpi_authority_info,
        depositor_info,
        flat_source_info,
        flat_burn_info,
        depositor_info,
        flat_mint_info,
        flat_interface_info,
        token_program_info,
        system_program_info,
    )?;
    let sleeve_bump = [sleeve.bump];
    let sleeve_signer_seeds = writer_sleeve_signer_seeds(&sleeve.settlement_group, &sleeve_bump);
    invoke_token_burn_checked(
        token_program_info,
        flat_burn_info,
        flat_mint_info,
        sleeve_info,
        params.amount_atoms,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &[&sleeve_signer_seeds],
    )?;
    invoke_token_transfer_checked(
        token_program_info,
        sleeve_vault_info,
        settlement_mint_info,
        destination_usdc_info,
        sleeve_info,
        params.amount_atoms,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &[&sleeve_signer_seeds],
    )?;
    if flat_mint
        .supply
        .checked_sub(validate_mint_account(flat_mint_info, token_program_info.key)?.supply)
        != Some(params.amount_atoms)
        || vault_before.checked_sub(validate_token_account(sleeve_vault_info)?.amount)
            != Some(params.amount_atoms)
        || validate_token_account(destination_usdc_info)?
            .amount
            .checked_sub(destination_before)
            != Some(params.amount_atoms)
        || validate_token_account(flat_burn_info)?.amount != 0
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    close_sleeve_token_custody(
        &sleeve,
        sleeve_info,
        flat_burn_info,
        depositor_info,
        token_program_info,
    )?;
    sleeve.writer_principal_atoms = sleeve
        .writer_principal_atoms
        .checked_sub(params.amount_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.flat_par_supply_atoms = sleeve
        .flat_par_supply_atoms
        .checked_sub(params.amount_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.accounted_asset_atoms = sleeve
        .accounted_asset_atoms
        .checked_sub(params.amount_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.last_updated_slot = Clock::get()?.slot;
    store_state(sleeve_info, &sleeve)
}
