use super::*;

pub(super) fn process_init_market_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: InitMarketV2Params,
) -> ProgramResult {
    if accounts.len() != 4 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let market_info = &accounts[1];
    let config_info = &accounts[2];
    let system_program_info = &accounts[3];

    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    if config.usdc_mint != params.collateral_mint {
        return Err(VaultError::InvalidMint.into());
    }
    validate_instrument_definition(&params.instrument)?;
    validate_market_parameters(&params.params, params.instrument.max_payout_per_contract)?;

    let (expected_market, bump) = derive_market_pda(program_id, &params.market_id);
    if *market_info.key != expected_market {
        return Err(VaultError::InvalidPda.into());
    }
    if market_info.owner == program_id {
        return Err(VaultError::AlreadyInitialized.into());
    }
    create_program_account(
        admin_info,
        market_info,
        system_program_info,
        program_id,
        Market::LEN,
        &[MARKET_PDA_SEED, &params.market_id, &[bump]],
    )?;

    let market = Market {
        is_initialized: true,
        bump,
        created_by: *admin_info.key,
        market_id: params.market_id,
        collateral_mint: params.collateral_mint,
        long_contract_mint: None,
        instrument: params.instrument,
        params: params.params,
        total_position_collateral_locked: 0,
        paused: true,
        mint_accounting: MarketMintAccounting::canonical_empty(),
    };
    store_state(market_info, &market)
}

pub(super) fn process_create_market_contract_mint_v3(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 9 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let payer_info = &accounts[0];
    let market_info = &accounts[1];
    let config_info = &accounts[2];
    let mint_info = &accounts[3];
    let spl_interface_info = &accounts[4];
    let light_token_program_info = &accounts[5];
    let cpi_authority_info = &accounts[6];
    let token_program_info = &accounts[7];
    let system_program_info = &accounts[8];

    if !payer_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *light_token_program_info.key != light_token_program_id() {
        return Err(VaultError::InvalidLightTokenProgram.into());
    }
    if *cpi_authority_info.key != cpi_authority() {
        return Err(VaultError::InvalidCompressedTokenAuthority.into());
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }

    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *payer_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let mut market = load_valid_market(program_id, market_info)?;
    if !market.is_initialized
        || market.long_contract_mint.is_some()
        || market.mint_accounting != MarketMintAccounting::canonical_empty()
    {
        return Err(VaultError::InvalidMarketMintAccounting.into());
    }

    let (expected_mint, mint_bump) = derive_contract_mint_pda(program_id, market_info.key);
    if *mint_info.key != expected_mint {
        return Err(VaultError::InvalidPda.into());
    }
    if mint_info.owner == &spl_token_program_id() {
        return Err(VaultError::AlreadyInitialized.into());
    }
    create_program_account(
        payer_info,
        mint_info,
        system_program_info,
        &spl_token_program_id(),
        Mint::LEN,
        &[
            CONTRACT_MINT_PDA_SEED,
            market_info.key.as_ref(),
            &[mint_bump],
        ],
    )?;
    invoke_token_initialize_mint2(
        token_program_info,
        mint_info,
        market_info.key,
        None,
        MarketMintAccounting::CANONICAL_DECIMALS,
    )?;

    let (expected_spl_interface, _) =
        light_token_instruction::get_spl_interface_pda_and_bump(mint_info.key);
    if *spl_interface_info.key != expected_spl_interface {
        return Err(VaultError::InvalidSplInterfaceAccount.into());
    }
    if spl_interface_info.owner != &system_program::id()
        || spl_interface_info.executable
        || spl_interface_info.data_len() != 0
    {
        return Err(VaultError::InvalidSplInterfaceAccount.into());
    }
    invoke_create_spl_interface_pda(
        payer_info,
        spl_interface_info,
        system_program_info,
        mint_info,
        token_program_info,
        cpi_authority_info,
        light_token_program_info,
    )?;

    let mint = validate_mint_account(mint_info, token_program_info.key)?;
    if !mint.is_initialized
        || mint.supply != 0
        || mint.decimals != MarketMintAccounting::CANONICAL_DECIMALS
        || mint.mint_authority != COption::Some(*market_info.key)
        || mint.freeze_authority != COption::None
    {
        return Err(VaultError::InvalidMarketMintPolicy.into());
    }
    if validate_spl_interface_account(mint_info.key, spl_interface_info)?.amount != 0 {
        return Err(VaultError::InvalidSplInterfaceAccount.into());
    }

    market.long_contract_mint = Some(*mint_info.key);
    store_state(market_info, &market)
}

pub(super) fn process_set_market_paused(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SetMarketPausedParams,
) -> ProgramResult {
    let expected_len = if params.paused { 3 } else { 4 };
    if accounts.len() != expected_len {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let market_info = &accounts[2];
    let month_info = if params.paused {
        None
    } else {
        Some(&accounts[3])
    };
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    if !params.paused && config.paused {
        return Err(VaultError::ContractPaused.into());
    }
    let mut market = load_valid_market(program_id, market_info)?;
    let _ = market_outstanding_contract_amount(&market)?;
    if let Some(month_info) = month_info {
        let month = load_valid_oracle_month(program_id, market_info, month_info, &market)?;
        ensure_oracle_game_window(&market, &month)?;
    }
    market.paused = params.paused;
    store_state(market_info, &market)
}
