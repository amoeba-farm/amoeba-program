use super::*;

#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(super) fn invoke_token_transfer_checked<'a>(
    token_program_info: &AccountInfo<'a>,
    source_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    destination_info: &AccountInfo<'a>,
    authority_info: &AccountInfo<'a>,
    amount: u64,
    decimals: u8,
    signer_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let transfer = token_instruction::transfer_checked(
        token_program_info.key,
        source_info.key,
        mint_info.key,
        destination_info.key,
        authority_info.key,
        &[],
        amount,
        decimals,
    )?;
    invoke_signed(
        &transfer,
        &[
            source_info.clone(),
            mint_info.clone(),
            destination_info.clone(),
            authority_info.clone(),
            token_program_info.clone(),
        ],
        signer_seeds,
    )
}

#[inline(never)]
pub(super) fn invoke_token_mint_to_checked<'a>(
    token_program_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    destination_info: &AccountInfo<'a>,
    authority_info: &AccountInfo<'a>,
    amount: u64,
    decimals: u8,
    signer_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let instruction = token_instruction::mint_to_checked(
        token_program_info.key,
        mint_info.key,
        destination_info.key,
        authority_info.key,
        &[],
        amount,
        decimals,
    )?;
    invoke_signed(
        &instruction,
        &[
            mint_info.clone(),
            destination_info.clone(),
            authority_info.clone(),
            token_program_info.clone(),
        ],
        signer_seeds,
    )
}

#[inline(never)]
pub(super) fn invoke_token_burn_checked<'a>(
    token_program_info: &AccountInfo<'a>,
    source_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    authority_info: &AccountInfo<'a>,
    amount: u64,
    decimals: u8,
    signer_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let instruction = token_instruction::burn_checked(
        token_program_info.key,
        source_info.key,
        mint_info.key,
        authority_info.key,
        &[],
        amount,
        decimals,
    )?;
    invoke_signed(
        &instruction,
        &[
            source_info.clone(),
            mint_info.clone(),
            authority_info.clone(),
            token_program_info.clone(),
        ],
        signer_seeds,
    )
}

#[inline(never)]
pub(super) fn invoke_token_close_account<'a>(
    token_program_info: &AccountInfo<'a>,
    account_info: &AccountInfo<'a>,
    destination_info: &AccountInfo<'a>,
    authority_info: &AccountInfo<'a>,
    signer_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let instruction = token_instruction::close_account(
        token_program_info.key,
        account_info.key,
        destination_info.key,
        authority_info.key,
        &[],
    )?;
    invoke_signed(
        &instruction,
        &[
            account_info.clone(),
            destination_info.clone(),
            authority_info.clone(),
            token_program_info.clone(),
        ],
        signer_seeds,
    )
}

#[inline(never)]
pub(super) fn invoke_token_initialize_account3<'a>(
    token_program_info: &AccountInfo<'a>,
    account_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    owner: &Pubkey,
) -> ProgramResult {
    let instruction = token_instruction::initialize_account3(
        token_program_info.key,
        account_info.key,
        mint_info.key,
        owner,
    )?;
    invoke(
        &instruction,
        &[
            account_info.clone(),
            mint_info.clone(),
            token_program_info.clone(),
        ],
    )
}

#[inline(never)]
pub(super) fn invoke_token_initialize_mint2<'a>(
    token_program_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    mint_authority: &Pubkey,
    freeze_authority: Option<&Pubkey>,
    decimals: u8,
) -> ProgramResult {
    let instruction = token_instruction::initialize_mint2(
        token_program_info.key,
        mint_info.key,
        mint_authority,
        freeze_authority,
        decimals,
    )?;
    invoke(
        &instruction,
        &[mint_info.clone(), token_program_info.clone()],
    )
}

#[inline(never)]
pub(super) fn invoke_create_associated_token_account_idempotent<'a>(
    payer_info: &AccountInfo<'a>,
    account_info: &AccountInfo<'a>,
    owner_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    token_program_info: &AccountInfo<'a>,
    associated_token_program_info: &AccountInfo<'a>,
) -> ProgramResult {
    let instruction = crate::associated_token::create_associated_token_account_idempotent(
        payer_info.key,
        owner_info.key,
        mint_info.key,
        token_program_info.key,
    );
    invoke(
        &instruction,
        &[
            payer_info.clone(),
            account_info.clone(),
            owner_info.clone(),
            mint_info.clone(),
            system_program_info.clone(),
            token_program_info.clone(),
            associated_token_program_info.clone(),
        ],
    )
}

#[inline(never)]
pub(super) fn invoke_create_spl_interface_pda<'a>(
    payer_info: &AccountInfo<'a>,
    interface_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    token_program_info: &AccountInfo<'a>,
    cpi_authority_info: &AccountInfo<'a>,
    light_token_program_info: &AccountInfo<'a>,
) -> ProgramResult {
    let instruction = light_token_instruction::create_spl_interface_pda(
        payer_info.key,
        mint_info.key,
        token_program_info.key,
    );
    invoke(
        &instruction,
        &[
            payer_info.clone(),
            interface_info.clone(),
            system_program_info.clone(),
            mint_info.clone(),
            token_program_info.clone(),
            cpi_authority_info.clone(),
            light_token_program_info.clone(),
        ],
    )
}

#[inline(never)]
pub(super) fn store_state(
    account_info: &AccountInfo,
    value: &dyn crate::fixed_codec::FixedStateEncode,
) -> ProgramResult {
    let mut data = account_info.try_borrow_mut_data()?;
    if value.maximum_encoded_len() > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    data.fill(0);
    value.encode_fixed(&mut data);
    Ok(())
}

pub(super) fn validate_mint_account(
    mint_info: &AccountInfo,
    token_program: &Pubkey,
) -> Result<Mint, ProgramError> {
    if mint_info.owner != token_program {
        return Err(VaultError::InvalidMint.into());
    }
    let mint_data = mint_info.try_borrow_data()?;
    Mint::unpack(&mint_data).map_err(|_| VaultError::InvalidMint.into())
}

pub(super) fn validate_collateral_mint_account(
    mint_info: &AccountInfo,
    token_program: &Pubkey,
) -> Result<Mint, ProgramError> {
    let mint = validate_mint_account(mint_info, token_program)?;
    if !mint.is_initialized || mint.decimals != MarketMintAccounting::CANONICAL_DECIMALS {
        return Err(VaultError::InvalidMint.into());
    }
    Ok(mint)
}

pub(super) fn validate_expected_collateral_freeze_authority(
    mint: &Mint,
    expected: Option<Pubkey>,
) -> ProgramResult {
    let matches = match (mint.freeze_authority, expected) {
        (COption::None, None) => true,
        (COption::Some(actual), Some(expected)) => actual == expected,
        _ => false,
    };
    if !matches {
        return Err(VaultError::InvalidMint.into());
    }
    Ok(())
}

pub(super) fn market_outstanding_contract_amount(market: &Market) -> Result<u64, ProgramError> {
    if !market.mint_accounting.has_canonical_layout()
        || market.mint_accounting.total_burned > market.mint_accounting.total_consumed
        || market.mint_accounting.total_consumed > market.mint_accounting.total_issued
    {
        return Err(VaultError::InvalidMarketMintAccounting.into());
    }
    market
        .mint_accounting
        .total_issued
        .checked_sub(market.mint_accounting.total_consumed)
        .ok_or_else(|| VaultError::InvalidMarketMintAccounting.into())
}

/// Validate the one canonical classic-SPL mint. Supply mode 0 checks policy;
/// nonzero mode consumes any downward external-burn delta before the caller
/// stores the Market. Supply increases are never admitted.
#[inline(never)]
pub(super) fn validate_canonical_market_mint(
    market_info: &AccountInfo,
    market: &mut Market,
    mint_info: &AccountInfo,
    supply_policy: u8,
) -> Result<Mint, ProgramError> {
    let (expected_mint, _) = derive_contract_mint_pda(market_info.owner, market_info.key);
    if *mint_info.key != expected_mint || market.long_contract_mint != Some(expected_mint) {
        return Err(VaultError::InvalidContractMint.into());
    }
    if mint_info.owner != &spl_token_program_id() {
        return Err(VaultError::InvalidMarketMintPolicy.into());
    }
    let mint = validate_mint_account(mint_info, &spl_token_program_id())?;
    if !mint.is_initialized
        || mint.decimals != MarketMintAccounting::CANONICAL_DECIMALS
        || mint.mint_authority != COption::Some(*market_info.key)
        || mint.freeze_authority != COption::None
    {
        return Err(VaultError::InvalidMarketMintPolicy.into());
    }
    if supply_policy == 0 {
        return Ok(mint);
    }
    if !market.mint_accounting.has_canonical_layout()
        || market.mint_accounting.total_burned > market.mint_accounting.total_consumed
        || market.mint_accounting.total_consumed > market.mint_accounting.total_issued
    {
        return Err(VaultError::InvalidMarketMintAccounting.into());
    }
    let expected_supply = market
        .mint_accounting
        .total_issued
        .checked_sub(market.mint_accounting.total_burned)
        .ok_or(VaultError::InvalidMarketMintAccounting)?;
    let externally_burned = expected_supply
        .checked_sub(mint.supply)
        .ok_or(VaultError::MarketMintSupplyMismatch)?;
    if externally_burned != 0 {
        consume_market_contracts(market, externally_burned, true)?;
    }
    Ok(mint)
}

pub(super) fn ensure_backed_contract_amount(market: &Market, amount: u64) -> ProgramResult {
    if amount == 0 || market_outstanding_contract_amount(market)? < amount {
        return Err(VaultError::InsufficientBackedContractSupply.into());
    }
    Ok(())
}

pub(super) fn consume_market_contracts(
    market: &mut Market,
    amount: u64,
    burned: bool,
) -> ProgramResult {
    ensure_backed_contract_amount(market, amount)?;
    market.mint_accounting.total_consumed = market
        .mint_accounting
        .total_consumed
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if burned {
        market.mint_accounting.total_burned = market
            .mint_accounting
            .total_burned
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
    }
    let _ = market_outstanding_contract_amount(market)?;
    Ok(())
}

pub(super) fn ensure_market_value_flow_unpaused(
    config: &VaultConfig,
    market: &Market,
) -> ProgramResult {
    if config.paused {
        return Err(VaultError::ContractPaused.into());
    }
    if market.paused {
        return Err(VaultError::MarketPaused.into());
    }
    Ok(())
}

pub(super) fn load_canonical_light_token_account(
    account_info: &AccountInfo,
    expected_owner: &Pubkey,
    expected_mint: &Pubkey,
) -> Result<TokenAccount, ProgramError> {
    validate_light_token_account(account_info)?;
    let data = account_info.try_borrow_data()?;
    if !has_canonical_compressible_token_layout(&data) {
        return Err(VaultError::InvalidLightTokenAccount.into());
    }
    let token = TokenAccount::unpack(&data[..TokenAccount::LEN])
        .map_err(|_| ProgramError::from(VaultError::InvalidLightTokenAccount))?;
    if token.owner != *expected_owner
        || token.mint != *expected_mint
        || token.state != AccountState::Initialized
        || token.delegate != COption::None
        || token.delegated_amount != 0
        || token.is_native != COption::None
        || token.close_authority != COption::None
    {
        return Err(VaultError::InvalidLightTokenAccount.into());
    }
    Ok(token)
}

#[inline(always)]
pub(super) fn validate_vault_token_account(
    token_info: &AccountInfo,
    expected_mint: &Pubkey,
    expected_owner: &Pubkey,
) -> Result<TokenAccount, ProgramError> {
    validate_strict_token_account(
        token_info,
        expected_mint,
        expected_owner,
        VaultError::InvalidTokenAccount,
    )
}

#[inline(never)]
pub(super) fn validate_strict_token_account(
    token_info: &AccountInfo,
    expected_mint: &Pubkey,
    expected_owner: &Pubkey,
    mismatch_error: VaultError,
) -> Result<TokenAccount, ProgramError> {
    let token = validate_token_account(token_info)?;
    if token.state != AccountState::Initialized
        || token.mint != *expected_mint
        || token.owner != *expected_owner
        || token.delegate != COption::None
        || token.delegated_amount != 0
        || token.is_native != COption::None
        || token.close_authority != COption::None
    {
        return Err(mismatch_error.into());
    }
    Ok(token)
}

pub(super) fn validate_token_account(
    token_info: &AccountInfo,
) -> Result<TokenAccount, ProgramError> {
    if token_info.owner != &spl_token_program_id() {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    let token_data = token_info.try_borrow_data()?;
    TokenAccount::unpack(&token_data).map_err(|_| VaultError::InvalidTokenAccount.into())
}

#[inline(never)]
pub(super) fn validate_spl_interface_account(
    mint: &Pubkey,
    interface_info: &AccountInfo,
) -> Result<TokenAccount, ProgramError> {
    let expected = light_token_instruction::get_spl_interface_pda_and_bump(mint).0;
    if *interface_info.key != expected {
        return Err(VaultError::InvalidSplInterfaceAccount.into());
    }
    let token = validate_token_account(interface_info)?;
    if token.mint != *mint
        || token.owner != cpi_authority()
        || token.state != AccountState::Initialized
        || token.delegate != COption::None
        || token.delegated_amount != 0
        || token.is_native != COption::None
        || token.close_authority != COption::None
    {
        return Err(VaultError::InvalidSplInterfaceAccount.into());
    }
    Ok(token)
}
