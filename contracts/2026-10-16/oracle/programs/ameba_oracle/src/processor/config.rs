use super::*;

/// `UpgradeableLoaderState::ProgramData` metadata: u32 tag 3, u64 slot,
/// then `Option<Pubkey>` as one presence byte and 32 key bytes.
const PROGRAM_DATA_METADATA_LEN: usize = 45;

/// The config's create-once initializer is the authority recorded in this program's loader-v3
/// ProgramData account. An immutable program (no recorded authority) can never initialize.
pub(super) fn require_upgrade_authority(
    program_id: &Pubkey,
    program_data: &AccountInfo,
    signer: &Pubkey,
) -> ProgramResult {
    let (expected, _) =
        Pubkey::find_program_address(&[program_id.as_ref()], &bpf_loader_upgradeable::ID);
    if program_data.key != &expected || program_data.owner != &bpf_loader_upgradeable::ID {
        return Err(OracleError::InvalidUpgradeAuthority.into());
    }
    let data = program_data.try_borrow_data()?;
    if data.len() < PROGRAM_DATA_METADATA_LEN
        || data[0..4] != 3u32.to_le_bytes()
        || data[12] != 1
        || &data[13..45] != signer.as_ref()
    {
        return Err(OracleError::InvalidUpgradeAuthority.into());
    }
    Ok(())
}

#[cfg(all(
    not(feature = "localnet-dev-authority"),
    not(feature = "synthetic-governance-controller")
))]
pub(super) fn require_bootstrap_council(accounts: &[AccountInfo]) -> ProgramResult {
    let (config, approvals) = accounts
        .split_first()
        .ok_or(OracleError::InvalidUpgradeAuthority)?;
    ameba_governance_interface::validate_council_authorization(config, approvals)
        .map_err(|_| OracleError::InvalidUpgradeAuthority)?;
    Ok(())
}

// Only the synthetic fixture profile substitutes its already pinned test identity. This branch
// cannot exist in a real controller build, and preserves the exact Config and quorum checks.
#[cfg(feature = "synthetic-governance-controller")]
pub(super) fn require_bootstrap_council(accounts: &[AccountInfo]) -> ProgramResult {
    let [config, first, second, third] = accounts else {
        return Err(OracleError::InvalidUpgradeAuthority.into());
    };
    if config.is_signer || config.is_writable || config.executable {
        return Err(OracleError::InvalidUpgradeAuthority.into());
    }
    let roster = super::council::decode_council(
        &crate::governance_gate::PINNED_CONTROLLER_PROGRAM_ID,
        config.key,
        config.owner,
        &config.try_borrow_data()?,
    )
    .map_err(|_| OracleError::InvalidUpgradeAuthority)?;
    let approvals = [first, second, third];
    for (index, seat) in approvals.iter().enumerate() {
        if !seat.is_signer
            || !roster.seats.contains(seat.key)
            || approvals[..index].iter().any(|prior| prior.key == seat.key)
        {
            return Err(OracleError::InvalidUpgradeAuthority.into());
        }
    }
    Ok(())
}

pub(super) fn validate_vault_config_initialization_target(
    program_id: &Pubkey,
    config_info: &AccountInfo,
) -> ProgramResult {
    if config_info.owner == program_id {
        return Err(OracleError::AlreadyInitialized.into());
    }
    if !config_info.is_writable
        || !crate::is_system_program(config_info.owner)
        || config_info.executable
        || !config_info.data_is_empty()
    {
        return Err(OracleError::InvalidPda.into());
    }
    Ok(())
}

pub(super) fn initial_vault_config(
    bump: u8,
    admin: Pubkey,
    usdc_mint: Pubkey,
    bond_vault: Pubkey,
) -> OracleConfig {
    OracleConfig {
        is_initialized: true,
        bump,
        admin,
        oracle_authority: admin,
        usdc_mint,
        bond_vault,
        paused: true,
        account_discriminator: OracleConfig::ACCOUNT_DISCRIMINATOR,
        account_version: OracleConfig::ACCOUNT_VERSION,
    }
}

/// Tag 0. Creates the paused config and its bond vault. Before controller handoff the recorded
/// upgrade authority signs; after handoff three current council seats authorize initialization.
/// `admin` becomes both the config admin and the oracle authority until tag 128 splits them.
#[inline(never)]
pub(super) fn process_initialize_config(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    admin: Pubkey,
) -> ProgramResult {
    let (initialization_accounts, council_accounts) = match accounts.len() {
        9 => (accounts, &accounts[9..]),
        13 => accounts.split_at(9),
        _ => return Err(OracleError::InvalidAccountList.into()),
    };
    let [payer_info, config_info, program_data_info, upgrade_authority_info, system_program_info, usdc_mint_info, bond_vault_info, token_program_info, associated_token_program_info] =
        initialization_accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    if !payer_info.is_signer
        || !payer_info.is_writable
        || config_info.is_signer
        || !config_info.is_writable
        || program_data_info.is_signer
        || program_data_info.is_writable
        || !bond_vault_info.is_writable
    {
        return Err(OracleError::InvalidAccountList.into());
    }
    if crate::pubkey_is_default(&admin) {
        return Err(OracleError::InvalidConfigAccount.into());
    }
    if council_accounts.is_empty() {
        if !upgrade_authority_info.is_signer {
            return Err(OracleError::InvalidUpgradeAuthority.into());
        }
    } else {
        #[cfg(feature = "localnet-dev-authority")]
        return Err(OracleError::InvalidAccountList.into());
        #[cfg(not(feature = "localnet-dev-authority"))]
        {
            let controller = crate::governance_gate::PINNED_CONTROLLER_PROGRAM_ID;
            // Registration hands the target's loader authority to the controller's
            // per-target PDA (`[domain, "target-authority", target]`).
            let authority = Pubkey::find_program_address(
                &[
                    b"ameba-governance-v3",
                    b"target-authority",
                    program_id.as_ref(),
                ],
                &controller,
            )
            .0;
            if *upgrade_authority_info.key != authority
                || upgrade_authority_info.is_signer
                || upgrade_authority_info.is_writable
            {
                return Err(OracleError::InvalidUpgradeAuthority.into());
            }
            require_bootstrap_council(council_accounts)?;
        }
    }
    require_upgrade_authority(program_id, program_data_info, upgrade_authority_info.key)?;
    if !crate::token_instruction::check_id(token_program_info.key)
        || !crate::pubkey_eq(
            associated_token_program_info.key,
            &crate::associated_token::id(),
        )
    {
        return Err(OracleError::InvalidTokenProgram.into());
    }
    if !crate::is_system_program(system_program_info.key) {
        return Err(OracleError::InvalidSystemProgram.into());
    }

    let (expected_config_pda, bump) = derive_vault_config_pda(program_id);
    if *config_info.key != expected_config_pda {
        return Err(OracleError::InvalidPda.into());
    }
    validate_vault_config_initialization_target(program_id, config_info)?;
    validate_collateral_mint_account(usdc_mint_info, token_program_info.key)?;
    let expected_bond_vault = crate::associated_token::get_associated_token_address_with_program_id(
        config_info.key,
        usdc_mint_info.key,
        token_program_info.key,
    );
    if *bond_vault_info.key != expected_bond_vault {
        return Err(OracleError::InvalidTokenAccount.into());
    }

    invoke_create_associated_token_account_idempotent(
        payer_info,
        bond_vault_info,
        config_info,
        usdc_mint_info,
        system_program_info,
        token_program_info,
        associated_token_program_info,
    )?;
    validate_vault_token_account(bond_vault_info, usdc_mint_info.key, config_info.key)?;

    create_program_account(
        payer_info,
        config_info,
        system_program_info,
        program_id,
        OracleConfig::LEN,
        &[VAULT_PDA_SEED, &[bump]],
    )?;

    let config = initial_vault_config(bump, admin, *usdc_mint_info.key, *bond_vault_info.key);
    store_state(config_info, &config)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn process_update_config(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    new_admin: Option<Pubkey>,
    new_oracle_authority: Option<Pubkey>,
    new_usdc_mint: Option<Pubkey>,
    new_bond_vault: Option<Pubkey>,
    paused: Option<bool>,
) -> ProgramResult {
    if accounts.len() != 5 {
        return Err(OracleError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let mint_info = &accounts[2];
    let vault_token_info = &accounts[3];
    let token_program_info = &accounts[4];

    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !crate::token_instruction::check_id(token_program_info.key) {
        return Err(OracleError::InvalidTokenProgram.into());
    }

    let mut config = load_canonical_vault_config(program_id, config_info)?;
    if *admin_info.key != config.admin {
        return Err(OracleError::Unauthorized.into());
    }
    if !config.has_current_layout() {
        return Err(OracleError::InvalidConfigAccount.into());
    }
    if new_admin == Some(Pubkey::default()) || new_oracle_authority == Some(Pubkey::default()) {
        return Err(OracleError::InvalidConfigAccount.into());
    }
    if new_admin.is_some_and(|authority| authority != config.admin)
        || new_oracle_authority.is_some_and(|authority| authority != config.oracle_authority)
    {
        return Err(OracleError::SettlementSignerGovernanceRequired.into());
    }
    if paused == Some(false) {
        return Err(OracleError::SettlementSignerGovernanceRequired.into());
    }

    if new_usdc_mint.is_some_and(|mint| mint != config.usdc_mint)
        || new_bond_vault.is_some_and(|vault_token| vault_token != config.bond_vault)
    {
        return Err(OracleError::CollateralIdentityImmutable.into());
    }
    if *mint_info.key != config.usdc_mint {
        return Err(OracleError::AccountMismatch.into());
    }
    if *vault_token_info.key != config.bond_vault {
        return Err(OracleError::AccountMismatch.into());
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
        return Err(OracleError::InvalidAccountList.into());
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
        return Err(OracleError::SettlementSignerGovernanceRequired.into());
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
        return Err(OracleError::SettlementSignerGovernanceRequired.into());
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

/// Tag 128. The fresh config carries one duplicated authority; its holder hands the oracle role
/// to a distinct signer exactly once, while paused and before the signer registry exists.
pub(super) fn process_bootstrap_vault_governance_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: BootstrapVaultGovernanceV2Params,
) -> ProgramResult {
    if accounts.len() != 4 {
        return Err(OracleError::InvalidAccountList.into());
    }
    let bootstrap_admin_info = &accounts[0];
    let new_oracle_info = &accounts[1];
    let config_info = &accounts[2];
    let registry_info = &accounts[3];
    if !bootstrap_admin_info.is_signer || !new_oracle_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !config_info.is_writable {
        return Err(OracleError::InvalidAccountList.into());
    }

    let mut config = load_canonical_vault_config(program_id, config_info)?;
    if !config.has_current_layout()
        || !config.paused
        || *bootstrap_admin_info.key != config.admin
        || config.admin != config.oracle_authority
        || params.new_oracle_authority != *new_oracle_info.key
        || crate::pubkey_is_default(&params.new_oracle_authority)
        || params.new_oracle_authority == config.admin
    {
        return Err(OracleError::SettlementSignerGovernanceRequired.into());
    }

    let (expected_registry, _) = derive_settlement_signer_registry_pda(program_id);
    if *registry_info.key != expected_registry {
        return Err(OracleError::InvalidPda.into());
    }
    if registry_info.owner == program_id {
        return Err(OracleError::AlreadyInitialized.into());
    }
    if !crate::is_system_program(registry_info.owner)
        || registry_info.executable
        || !registry_info.data_is_empty()
    {
        return Err(OracleError::InvalidSettlementSignerRegistry.into());
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
        return Err(OracleError::InvalidAccountList.into());
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
        return Err(OracleError::InvalidAccountList.into());
    }

    let mut config = load_canonical_vault_config(program_id, config_info)?;
    if !config.has_current_layout()
        || !config.paused
        || config.admin != *admin_info.key
        || config.admin == config.oracle_authority
    {
        return Err(OracleError::SettlementSignerGovernanceRequired.into());
    }
    if !crate::token_instruction::check_id(token_program_info.key) {
        return Err(OracleError::InvalidTokenProgram.into());
    }
    if *collateral_mint_info.key != config.usdc_mint || *vault_token_info.key != config.bond_vault {
        return Err(OracleError::AccountMismatch.into());
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
        return Err(OracleError::SettlementSignerGovernanceRequired.into());
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
