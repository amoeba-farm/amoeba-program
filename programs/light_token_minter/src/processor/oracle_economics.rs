use super::*;

pub(super) fn process_configure_oracle_economics_template_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ConfigureOracleEconomicsTemplateV2Params,
) -> ProgramResult {
    if accounts.len() != 4 || !accounts[0].is_signer {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let economics_info = &accounts[2];
    let system_program_info = &accounts[3];
    // Economics are configuration state, not value flow.  Bootstrap and later
    // governed versioning must remain possible while the global vault is
    // paused; each month snapshots the selected version at create time.
    let config = load_current_canonical_vault_config(program_id, config_info)?;
    if *admin_info.key != config.admin {
        return Err(VaultError::Unauthorized.into());
    }
    validate_system_program(system_program_info)?;
    validate_oracle_economics(&params.economics)?;
    let (expected, bump) = derive_oracle_economics_config_pda(program_id);
    if *economics_info.key != expected {
        return Err(VaultError::InvalidOracleState.into());
    }
    let next_version = if economics_info.owner == program_id {
        let existing = load_canonical_oracle_economics_config(program_id, economics_info)?;
        if existing.config_version != params.expected_config_version {
            return Err(VaultError::InvalidOracleState.into());
        }
        existing
            .config_version
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?
    } else {
        if params.expected_config_version != 0 {
            return Err(VaultError::InvalidOracleState.into());
        }
        create_program_account(
            admin_info,
            economics_info,
            system_program_info,
            program_id,
            OracleEconomicsConfig::LEN,
            &[ORACLE_ECONOMICS_CONFIG_PDA_SEED, &[bump]],
        )?;
        1
    };
    let economics = OracleEconomicsConfig {
        is_initialized: true,
        bump,
        account_discriminator: OracleEconomicsConfig::ACCOUNT_DISCRIMINATOR,
        account_version: OracleEconomicsConfig::ACCOUNT_VERSION,
        config_version: next_version,
        economics: params.economics,
        last_updated_slot: Clock::get()?.slot,
    };
    store_state(economics_info, &economics)
}

pub(super) fn validate_oracle_economics(economics: &OracleEconomicParams) -> ProgramResult {
    if economics.emergency_supermajority_bps == 0
        || economics.emergency_supermajority_bps > 10_000
        || economics.emergency_commit_window_slots == 0
        || economics.emergency_reveal_window_slots == 0
    {
        return Err(VaultError::InvalidOracleState.into());
    }
    Ok(())
}

pub(super) fn process_configure_oracle_major_token(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let payer_info = &accounts[1];
    let config_info = &accounts[2];
    let major_config_info = &accounts[3];
    let mint_info = &accounts[4];
    let vault_token_info = &accounts[5];
    let token_program_info = &accounts[6];
    let system_program_info = &accounts[7];

    let config =
        validate_current_oracle_authority_allow_paused(program_id, authority_info, config_info)?;
    validate_current_account_creation_payer(payer_info)?;
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    let (expected, bump) = derive_oracle_major_token_config_pda(program_id);
    if *major_config_info.key != expected {
        return Err(VaultError::InvalidOracleMajorTokenConfig.into());
    }

    if *mint_info.key == config.usdc_mint || *vault_token_info.key == config.vault_token_account {
        return Err(VaultError::InvalidOracleMajorTokenConfig.into());
    }
    validate_mint_account(mint_info, token_program_info.key)?;
    validate_vault_token_account(vault_token_info, mint_info.key, config_info.key)?;

    if major_config_info.owner == program_id {
        let existing: OracleMajorTokenConfig = load_exact_zero_padded_state(
            major_config_info,
            program_id,
            OracleMajorTokenConfig::LEN,
            VaultError::InvalidOracleMajorTokenConfig,
        )?;
        if existing.is_initialized
            && (existing.mint != *mint_info.key
                || existing.vault_token_account != *vault_token_info.key)
        {
            return Err(VaultError::AlreadyInitialized.into());
        }
    } else {
        create_program_account(
            payer_info,
            major_config_info,
            system_program_info,
            program_id,
            OracleMajorTokenConfig::LEN,
            &[ORACLE_MAJOR_TOKEN_CONFIG_PDA_SEED, &[bump]],
        )?;
    }

    let major_config = OracleMajorTokenConfig {
        is_initialized: true,
        bump,
        mint: *mint_info.key,
        vault_token_account: *vault_token_info.key,
    };
    store_state(major_config_info, &major_config)
}
