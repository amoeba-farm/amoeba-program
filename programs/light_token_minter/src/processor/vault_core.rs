use super::*;

pub(super) fn process_close_oracle_month(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 5 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let config_info = &accounts[1];
    let market_info = &accounts[2];
    let month_info = &accounts[3];
    let settlement_record_info = &accounts[4];
    validate_oracle_authority(program_id, authority_info, config_info)?;
    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_month_ready_to_close(&month, settlement_record_info.key)?;
    let settlement = load_valid_settlement_record_v2(
        program_id,
        market_info.key,
        &market,
        settlement_record_info,
    )?;
    // The immutable compressed settlement leaf owns the detailed recipe/source
    // provenance.  This rent-minimized native anchor is bound to the same month
    // by its canonical PDA and the commitment checked when it was created.
    if settlement.oracle_month != *month_info.key {
        return Err(VaultError::InvalidSettlementRecord.into());
    }
    month.phase = OraclePhase::Closed;
    month.last_updated_slot = Clock::get()?.slot;
    store_oracle_month_state(month_info, &month)
}

pub(super) fn process_finalize_oracle_month(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() < 4 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    if !cranker_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_opening_resolution_complete(&month)?;
    if month.phase != OraclePhase::Game {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    if month.settlement_status == OracleSettlementStatus::Final || month.finalized_at_ts != 0 {
        return Err(VaultError::InvalidOracleState.into());
    }
    let (slot, now) = current_slot_and_unix_timestamp()?;
    ensure_settlement_finalization_ready_at(market.instrument.expiry_ts, now)?;
    if month.pending_resolution_count != 0 {
        return Err(VaultError::InvalidOracleState.into());
    }
    if accounts.len() != 3 + usize::from(month.active_weight_group_count) {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut last_bucket_id = [0u8; 32];
    let mut weight_total = 0u16;
    let mut index_total = 0i128;
    for (group_index, bucket_info) in accounts[3..].iter().enumerate() {
        let bucket = load_valid_oracle_bucket_median(program_id, month_info.key, bucket_info)?;
        if usize::from(bucket.group_index) != group_index
            || bucket.bucket_id <= last_bucket_id
            || !matches!(
                bucket.status,
                OracleBucketMedianStatus::SettlementReady
                    | OracleBucketMedianStatus::EmergencyDefaulted
            )
            || bucket.eligible_source_count
                < minimum_oracle_bucket_eligible_sources(bucket.frozen_source_count)
                && bucket.status != OracleBucketMedianStatus::EmergencyDefaulted
            || bucket.last_recomputed_ts < market.instrument.expiry_ts
        {
            return Err(VaultError::InvalidOracleMedian.into());
        }
        weight_total = weight_total
            .checked_add(bucket.bucket_weight_bps)
            .ok_or(VaultError::ArithmeticOverflow)?;
        index_total = index_total
            .checked_add(i128::from(bucket_index_contribution_bps(
                bucket.bucket_weight_bps,
                bucket.bucket_delta_bps,
            )?))
            .ok_or(VaultError::ArithmeticOverflow)?;
        last_bucket_id = bucket.bucket_id;
    }
    let canonical_index = i64::try_from(index_total).map_err(|_| VaultError::ArithmeticOverflow)?;
    if weight_total != 10_000 || canonical_index != month.index_delta_bps {
        return Err(VaultError::InvalidOracleMedian.into());
    }
    month.c_raw_bps = 10_000;
    month.g_camo_bps = 0;
    month.g_thin_bps = 0;
    month.c_settle_bps = 10_000;
    month.settlement_status = OracleSettlementStatus::Final;
    month.finalized_at_ts = now;
    month.last_updated_slot = slot;
    store_oracle_month_state(month_info, &month)
}

pub(super) fn is_authorized_vault_initializer(authority: &Pubkey) -> bool {
    *authority == VAULT_CONFIG_BOOTSTRAP_AUTHORITY
}

pub(super) fn validate_vault_config_initialization_target(
    program_id: &Pubkey,
    config_info: &AccountInfo,
) -> ProgramResult {
    if config_info.owner == program_id {
        return Err(VaultError::AlreadyInitialized.into());
    }
    if !config_info.is_writable
        || config_info.owner != &system_program::id()
        || config_info.executable
        || !config_info.data_is_empty()
    {
        return Err(VaultError::InvalidPda.into());
    }
    Ok(())
}

pub(super) fn initial_vault_config(
    bump: u8,
    admin: Pubkey,
    usdc_mint: Pubkey,
    vault_token_account: Pubkey,
) -> VaultConfig {
    VaultConfig {
        is_initialized: true,
        bump,
        admin,
        oracle_authority: admin,
        usdc_mint,
        vault_token_account,
        paused: true,
        account_discriminator: VaultConfig::ACCOUNT_DISCRIMINATOR,
        account_version: VaultConfig::ACCOUNT_VERSION,
    }
}

pub(super) fn process_initialize(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    if accounts.len() < 6 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let usdc_mint_info = &accounts[2];
    let vault_token_info = &accounts[3];
    let token_program_info = &accounts[4];
    let system_program_info = &accounts[5];

    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !is_authorized_vault_initializer(admin_info.key) {
        return Err(VaultError::Unauthorized.into());
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }

    let (expected_config_pda, bump) = derive_vault_config_pda(program_id);
    if *config_info.key != expected_config_pda {
        return Err(VaultError::InvalidPda.into());
    }
    validate_vault_config_initialization_target(program_id, config_info)?;

    validate_collateral_mint_account(usdc_mint_info, token_program_info.key)?;
    validate_vault_token_account(vault_token_info, usdc_mint_info.key, config_info.key)?;

    create_program_account(
        admin_info,
        config_info,
        system_program_info,
        program_id,
        VaultConfig::LEN,
        &[VAULT_PDA_SEED, &[bump]],
    )?;

    let config = initial_vault_config(
        bump,
        *admin_info.key,
        *usdc_mint_info.key,
        *vault_token_info.key,
    );
    store_state(config_info, &config)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn process_update_config(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    new_admin: Option<Pubkey>,
    new_oracle_authority: Option<Pubkey>,
    new_usdc_mint: Option<Pubkey>,
    new_vault_token_account: Option<Pubkey>,
    paused: Option<bool>,
) -> ProgramResult {
    if accounts.len() != 5 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let mint_info = &accounts[2];
    let vault_token_info = &accounts[3];
    let token_program_info = &accounts[4];

    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }

    let mut config = load_canonical_vault_config(program_id, config_info)?;
    if *admin_info.key != config.admin {
        return Err(VaultError::Unauthorized.into());
    }
    if !config.has_current_layout() {
        return Err(VaultError::InvalidConfigAccount.into());
    }
    if new_admin == Some(Pubkey::default()) || new_oracle_authority == Some(Pubkey::default()) {
        return Err(VaultError::InvalidConfigAccount.into());
    }
    if new_admin.is_some_and(|authority| authority != config.admin)
        || new_oracle_authority.is_some_and(|authority| authority != config.oracle_authority)
    {
        return Err(VaultError::SettlementSignerGovernanceRequired.into());
    }
    if paused == Some(false) {
        return Err(VaultError::SettlementSignerGovernanceRequired.into());
    }

    if new_usdc_mint.is_some_and(|mint| mint != config.usdc_mint)
        || new_vault_token_account
            .is_some_and(|vault_token| vault_token != config.vault_token_account)
    {
        return Err(VaultError::CollateralIdentityImmutable.into());
    }
    if *mint_info.key != config.usdc_mint {
        return Err(VaultError::AccountMismatch.into());
    }
    if *vault_token_info.key != config.vault_token_account {
        return Err(VaultError::AccountMismatch.into());
    }

    // Emergency pause must remain live even when custody is frozen, closed, or otherwise
    // undecodable. The immutable canonical account keys are still bound above; every non-pause
    // use continues through the full custody validation.
    if paused != Some(true) {
        validate_collateral_mint_account(mint_info, token_program_info.key)?;
        validate_vault_token_account(vault_token_info, &config.usdc_mint, config_info.key)?;
    }

    if let Some(is_paused) = paused {
        config.paused = is_paused;
    }
    store_state(config_info, &config)
}

pub(super) fn process_rotate_vault_authorities_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: RotateVaultAuthoritiesV2Params,
) -> ProgramResult {
    if accounts.len() < 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let current_admin_info = &accounts[0];
    let current_oracle_info = &accounts[1];
    let new_admin_info = &accounts[2];
    let new_oracle_info = &accounts[3];
    let config_info = &accounts[4];
    let registry_info = &accounts[5];
    let current_set_info = &accounts[6];
    if accounts.len() != 7
        || !current_admin_info.is_signer
        || !current_oracle_info.is_signer
        || !new_admin_info.is_signer
        || !new_oracle_info.is_signer
    {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let mut config = load_canonical_vault_config(program_id, config_info)?;
    if !config.has_current_layout()
        || !config.paused
        || config.admin != *current_admin_info.key
        || config.oracle_authority != *current_oracle_info.key
        || config.admin == config.oracle_authority
        || params.new_admin != *new_admin_info.key
        || params.new_oracle_authority != *new_oracle_info.key
        || crate::pubkey_is_default(&params.new_admin)
        || crate::pubkey_is_default(&params.new_oracle_authority)
        || params.new_admin == params.new_oracle_authority
    {
        return Err(VaultError::SettlementSignerGovernanceRequired.into());
    }
    let registry = load_canonical_settlement_signer_registry(program_id, registry_info)?;
    let current_set =
        load_canonical_settlement_signer_set(program_id, registry_info.key, current_set_info)?;
    if registry.current_set != *current_set_info.key
        || registry.current_version != current_set.version
        || registry.recovery_authority == config.admin
        || registry.recovery_authority == config.oracle_authority
        || registry.recovery_authority == params.new_admin
        || registry.recovery_authority == params.new_oracle_authority
    {
        return Err(VaultError::SettlementSignerGovernanceRequired.into());
    }
    settlement_signer_set_excludes_governance(
        &current_set,
        &[
            config.admin,
            config.oracle_authority,
            params.new_admin,
            params.new_oracle_authority,
            registry.recovery_authority,
        ],
    )?;
    config.admin = params.new_admin;
    config.oracle_authority = params.new_oracle_authority;
    store_state(config_info, &config)
}

pub(super) fn process_bootstrap_vault_governance_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: BootstrapVaultGovernanceV2Params,
) -> ProgramResult {
    if accounts.len() != 4 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let bootstrap_admin_info = &accounts[0];
    let new_oracle_info = &accounts[1];
    let config_info = &accounts[2];
    let registry_info = &accounts[3];
    if !bootstrap_admin_info.is_signer || !new_oracle_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !config_info.is_writable {
        return Err(VaultError::InvalidAccountList.into());
    }

    let mut config = load_canonical_vault_config(program_id, config_info)?;
    if !config.has_current_layout()
        || !config.paused
        || *bootstrap_admin_info.key != VAULT_CONFIG_BOOTSTRAP_AUTHORITY
        || config.admin != VAULT_CONFIG_BOOTSTRAP_AUTHORITY
        || config.oracle_authority != VAULT_CONFIG_BOOTSTRAP_AUTHORITY
        || params.new_oracle_authority != *new_oracle_info.key
        || crate::pubkey_is_default(&params.new_oracle_authority)
        || params.new_oracle_authority == VAULT_CONFIG_BOOTSTRAP_AUTHORITY
    {
        return Err(VaultError::SettlementSignerGovernanceRequired.into());
    }

    let (expected_registry, _) = derive_settlement_signer_registry_pda(program_id);
    if *registry_info.key != expected_registry {
        return Err(VaultError::InvalidPda.into());
    }
    if registry_info.owner == program_id {
        return Err(VaultError::AlreadyInitialized.into());
    }
    if registry_info.owner != &system_program::id()
        || registry_info.executable
        || !registry_info.data_is_empty()
    {
        return Err(VaultError::InvalidSettlementSignerRegistry.into());
    }

    config.oracle_authority = params.new_oracle_authority;
    store_state(config_info, &config)
}

pub(super) fn process_activate_vault_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ActivateVaultV2Params,
) -> ProgramResult {
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let registry_info = &accounts[2];
    let current_set_info = &accounts[3];
    let collateral_mint_info = &accounts[4];
    let vault_token_info = &accounts[5];
    let token_program_info = &accounts[6];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !config_info.is_writable {
        return Err(VaultError::InvalidAccountList.into());
    }

    let mut config = load_canonical_vault_config(program_id, config_info)?;
    if !config.has_current_layout()
        || !config.paused
        || config.admin != *admin_info.key
        || config.admin == config.oracle_authority
    {
        return Err(VaultError::SettlementSignerGovernanceRequired.into());
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    if *collateral_mint_info.key != config.usdc_mint
        || *vault_token_info.key != config.vault_token_account
    {
        return Err(VaultError::AccountMismatch.into());
    }
    let collateral_mint =
        validate_collateral_mint_account(collateral_mint_info, token_program_info.key)?;
    validate_expected_collateral_freeze_authority(
        &collateral_mint,
        params.expected_collateral_freeze_authority,
    )?;
    validate_vault_token_account(vault_token_info, &config.usdc_mint, config_info.key)?;
    let registry = load_canonical_settlement_signer_registry(program_id, registry_info)?;
    let current_set =
        load_canonical_settlement_signer_set(program_id, registry_info.key, current_set_info)?;
    if registry.current_set != *current_set_info.key
        || registry.current_version != current_set.version
        || registry.recovery_authority == config.admin
        || registry.recovery_authority == config.oracle_authority
    {
        return Err(VaultError::SettlementSignerGovernanceRequired.into());
    }
    settlement_signer_set_excludes_governance(
        &current_set,
        &[
            config.admin,
            config.oracle_authority,
            registry.recovery_authority,
        ],
    )?;

    config.paused = false;
    store_state(config_info, &config)
}

pub(super) fn process_deposit(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    amount: u64,
    require_current_admin: bool,
) -> ProgramResult {
    if amount == 0 {
        return Err(VaultError::AmountMustBePositive.into());
    }

    if accounts.len() < 6 {
        return Err(VaultError::InvalidAccountList.into());
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
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }

    let config = load_active_vault_config(program_id, config_info, mint_info, vault_token_info)?;
    if require_current_admin && *user_info.key != config.admin {
        return Err(VaultError::Unauthorized.into());
    }

    let mint = validate_mint_account(mint_info, token_program_info.key)?;
    let user_token = validate_token_account(user_token_info)?;
    let vault_token = validate_token_account(vault_token_info)?;

    if user_token.owner != *user_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    if user_token.mint != config.usdc_mint || vault_token.mint != config.usdc_mint {
        return Err(VaultError::InvalidMint.into());
    }
    if vault_token.owner != *config_info.key {
        return Err(VaultError::InvalidTokenAccount.into());
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
        return Err(VaultError::AmountMustBePositive.into());
    }

    if accounts.len() != 7 + usize::from(assisted) {
        return Err(VaultError::InvalidAccountList.into());
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

    if (assisted && (!admin_info.is_signer || !owner_info.is_signer))
        || (!assisted && !owner_info.is_signer)
    {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }

    let config = load_active_vault_config(program_id, config_info, mint_info, vault_token_info)?;
    if assisted && *admin_info.key != config.admin {
        return Err(VaultError::Unauthorized.into());
    }

    let mint = validate_mint_account(mint_info, token_program_info.key)?;
    let vault_token = validate_token_account(vault_token_info)?;
    let destination_token = validate_token_account(destination_token_info)?;
    let destination_is_canonical = !assisted
        || *destination_token_info.key
            == crate::associated_token::get_associated_token_address_with_program_id(
                owner_info.key,
                &config.usdc_mint,
                &spl_token_program_id(),
            );

    if vault_token.owner != *config_info.key
        || vault_token.mint != config.usdc_mint
        || !destination_is_canonical
        || destination_token.owner != *owner_info.key
        || destination_token.mint != config.usdc_mint
    {
        return Err(VaultError::InvalidTokenAccount.into());
    }

    let mut collateral =
        load_canonical_user_collateral(program_id, collateral_info, owner_info.key)?;
    if collateral.available_balance < amount {
        return Err(VaultError::InsufficientAvailableCollateral.into());
    }
    collateral.available_balance = collateral
        .available_balance
        .checked_sub(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
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
