use super::*;

pub(super) fn process_init_user_collateral(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 3 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let user_info = &accounts[0];
    let collateral_info = &accounts[1];
    let system_program_info = &accounts[2];

    if !user_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }

    let (expected_collateral, bump) = derive_user_collateral_pda(program_id, user_info.key);
    if *collateral_info.key != expected_collateral {
        return Err(VaultError::InvalidPda.into());
    }
    if collateral_info.owner == program_id {
        load_canonical_user_collateral(program_id, collateral_info, user_info.key)?;
        return Err(VaultError::AlreadyInitialized.into());
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
        return Err(VaultError::AmountMustBePositive.into());
    }
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
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
        false,
    )?;

    collateral.available_balance = collateral
        .available_balance
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    store_state(collateral_info, &collateral)
}
