use super::*;

pub(super) fn process_init_user_collateral(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 3 {
        return Err(OracleError::InvalidAccountList.into());
    }
    let user_info = &accounts[0];
    let collateral_info = &accounts[1];
    let system_program_info = &accounts[2];

    if !user_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !crate::is_system_program(system_program_info.key) {
        return Err(OracleError::InvalidSystemProgram.into());
    }

    let (expected_collateral, bump) = derive_user_collateral_pda(program_id, user_info.key);
    if *collateral_info.key != expected_collateral {
        return Err(OracleError::InvalidPda.into());
    }
    if collateral_info.owner == program_id {
        load_canonical_user_collateral(program_id, collateral_info, user_info.key)?;
        return Err(OracleError::AlreadyInitialized.into());
    }

    create_program_account(
        user_info,
        collateral_info,
        system_program_info,
        program_id,
        UserCollateral::LEN,
        &[USER_COLLATERAL_PDA_SEED, user_info.key.as_ref(), &[bump]],
    )?;

    let collateral = UserCollateral {
        is_initialized: true,
        bump,
        owner: *user_info.key,
        ..UserCollateral::default()
    };
    store_state(collateral_info, &collateral)
}

pub(super) fn process_deposit_collateral(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    amount: u64,
) -> ProgramResult {
    if amount == 0 {
        return Err(OracleError::AmountMustBePositive.into());
    }
    if accounts.len() != 7 {
        return Err(OracleError::InvalidAccountList.into());
    }
    let user_info = &accounts[0];
    let user_token_info = &accounts[1];
    let vault_token_info = &accounts[2];
    let config_info = &accounts[3];
    let collateral_info = &accounts[4];
    let mint_info = &accounts[5];
    let token_program_info = &accounts[6];

    let mut collateral =
        load_canonical_user_collateral(program_id, collateral_info, user_info.key)?;
    process_deposit(
        program_id,
        &[
            user_info.clone(),
            user_token_info.clone(),
            vault_token_info.clone(),
            config_info.clone(),
            mint_info.clone(),
            token_program_info.clone(),
        ],
        amount,
    )?;

    collateral.available_balance = collateral
        .available_balance
        .checked_add(amount)
        .ok_or(OracleError::ArithmeticOverflow)?;
    store_state(collateral_info, &collateral)
}

/// Owner collateral movement is bound to current custody and balances. It does not depend on the
/// pause flag used by protocol-admin value flows.
fn load_user_collateral_vault_config(
    program_id: &Pubkey,
    config_info: &AccountInfo,
    mint_info: &AccountInfo,
    vault_token_info: &AccountInfo,
) -> Result<OracleConfig, ProgramError> {
    let config = load_current_canonical_vault_config(program_id, config_info)?;
    if *mint_info.key != config.usdc_mint {
        return Err(OracleError::InvalidMint.into());
    }
    if *vault_token_info.key != config.bond_vault {
        return Err(OracleError::InvalidTokenAccount.into());
    }
    Ok(config)
}

/// Moves `amount` from the signer's token account into the bond vault. Ledger accounting is the
/// caller's responsibility.
pub(super) fn process_deposit(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    amount: u64,
) -> ProgramResult {
    if amount == 0 {
        return Err(OracleError::AmountMustBePositive.into());
    }

    if accounts.len() < 6 {
        return Err(OracleError::InvalidAccountList.into());
    }
    let user_info = &accounts[0];
    let user_token_info = &accounts[1];
    let vault_token_info = &accounts[2];
    let config_info = &accounts[3];
    let mint_info = &accounts[4];
    let token_program_info = &accounts[5];

    if !user_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !crate::token_instruction::check_id(token_program_info.key) {
        return Err(OracleError::InvalidTokenProgram.into());
    }

    let config =
        load_user_collateral_vault_config(program_id, config_info, mint_info, vault_token_info)?;

    let mint = validate_mint_account(mint_info, token_program_info.key)?;
    let user_token = validate_token_account(user_token_info)?;
    let vault_token = validate_token_account(vault_token_info)?;

    if user_token.owner != *user_info.key {
        return Err(OracleError::Unauthorized.into());
    }
    if user_token.mint != config.usdc_mint || vault_token.mint != config.usdc_mint {
        return Err(OracleError::InvalidMint.into());
    }
    if vault_token.owner != *config_info.key {
        return Err(OracleError::InvalidTokenAccount.into());
    }

    invoke_token_transfer_checked(
        token_program_info,
        user_token_info,
        mint_info,
        vault_token_info,
        user_info,
        amount,
        mint.decimals,
        &[],
    )
}

#[inline(never)]
pub(super) fn process_collateral_withdrawal(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    amount: u64,
    assisted: bool,
) -> ProgramResult {
    if amount == 0 {
        return Err(OracleError::AmountMustBePositive.into());
    }

    if accounts.len() != 7 + usize::from(assisted) {
        return Err(OracleError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let offset = usize::from(assisted);
    let owner_info = &accounts[offset];
    let vault_token_info = &accounts[1 + offset];
    let destination_token_info = &accounts[2 + offset];
    let config_info = &accounts[3 + offset];
    let collateral_info = &accounts[4 + offset];
    let mint_info = &accounts[5 + offset];
    let token_program_info = &accounts[6 + offset];

    if !owner_info.is_signer || (assisted && !admin_info.is_signer) {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !crate::token_instruction::check_id(token_program_info.key) {
        return Err(OracleError::InvalidTokenProgram.into());
    }

    let config = if assisted {
        load_active_vault_config(program_id, config_info, mint_info, vault_token_info)?
    } else {
        load_user_collateral_vault_config(program_id, config_info, mint_info, vault_token_info)?
    };
    if assisted && *admin_info.key != config.admin {
        return Err(OracleError::Unauthorized.into());
    }

    let mint = validate_mint_account(mint_info, token_program_info.key)?;
    let vault_token = validate_token_account(vault_token_info)?;
    let destination_token = validate_token_account(destination_token_info)?;
    let destination_is_canonical = !assisted
        || *destination_token_info.key
            == crate::associated_token::get_associated_token_address_with_program_id(
                owner_info.key,
                &config.usdc_mint,
                &token_instruction::id(),
            );

    if vault_token.owner != *config_info.key
        || vault_token.mint != config.usdc_mint
        || !destination_is_canonical
        || destination_token.owner != *owner_info.key
        || destination_token.mint != config.usdc_mint
    {
        return Err(OracleError::InvalidTokenAccount.into());
    }

    let mut collateral =
        load_canonical_user_collateral(program_id, collateral_info, owner_info.key)?;
    if collateral.available_balance < amount {
        return Err(OracleError::InsufficientAvailableCollateral.into());
    }
    collateral.available_balance = collateral
        .available_balance
        .checked_sub(amount)
        .ok_or(OracleError::ArithmeticOverflow)?;
    store_state(collateral_info, &collateral)?;

    invoke_token_transfer_checked(
        token_program_info,
        vault_token_info,
        mint_info,
        destination_token_info,
        config_info,
        amount,
        mint.decimals,
        &[&[CURRENT_STATE_NAMESPACE_SEED, VAULT_PDA_SEED, &[config.bump]]],
    )
}
