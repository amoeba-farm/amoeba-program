//! Prospective, create-once CFM policy registration through governed tag 30.
//! Registration reserves a typed empty month without starting its launch clock.
use super::*;
use crate::oracle_parent_proxy::{
    policy::{CfmMonthPolicy, CFM_MONTH_POLICY_SEED},
    CfmRegistry, NANDX_REGISTRY_HASH, NANDX_TERMINAL_ROOT, RAMX_REGISTRY_HASH, RAMX_TERMINAL_ROOT,
    REGISTRY_DOMAIN,
};

#[inline(never)]
pub(super) fn initialize_policy(
    program: &Pubkey,
    accounts: &[AccountInfo],
    product: u8,
) -> ProgramResult {
    if accounts.len() != 10 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin = &accounts[0];
    let oracle = &accounts[1];
    let payer = &accounts[2];
    let market_info = &accounts[3];
    let month_info = &accounts[4];
    let policy_info = &accounts[5];
    let config_info = &accounts[6];
    let product_manifest_info = &accounts[7];
    let registry_object = &accounts[8];
    let system = &accounts[9];
    if !admin.is_signer || !oracle.is_signer || admin.key == oracle.key || !payer.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_current_account_creation_payer(payer)?;
    validate_system_program(system)?;
    let config = load_current_canonical_vault_config(program, config_info)?;
    if *admin.key != config.admin || *oracle.key != config.oracle_authority {
        return Err(VaultError::Unauthorized.into());
    }
    let market = load_valid_market(program, market_info)?;
    if !market.paused
        || market.total_position_collateral_locked != 0
        || market.mint_accounting.total_issued != 0
        || market.mint_accounting.total_consumed != 0
        || market.mint_accounting.total_burned != 0
    {
        return Err(VaultError::InvalidOracleState.into());
    }
    let expected_month =
        derive_oracle_month_pda(program, market_info.key, market.instrument.expiry_ts).0;
    if *month_info.key != expected_month
        || month_info.owner != &solana_sdk_ids::system_program::ID
        || month_info.data_len() != 0
        || month_info.executable
        || month_info.is_signer
        || !month_info.is_writable
    {
        return Err(VaultError::InvalidOracleMonthAccount.into());
    }
    let product_manifest = load_valid_oracle_product_sku_manifest(
        program,
        &market.instrument.underlying_id,
        product_manifest_info,
    )?;
    let (count, root, registry_hash) = match product {
        0 => (52, RAMX_TERMINAL_ROOT, RAMX_REGISTRY_HASH),
        1 => (48, NANDX_TERMINAL_ROOT, NANDX_REGISTRY_HASH),
        _ => return Err(VaultError::InvalidOracleState.into()),
    };
    if product_manifest.required_sku_count != count || product_manifest.required_sku_root != root {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    let preimage =
        oracle_evidence::sealed_definition_preimage(program, registry_object, &registry_hash)?;
    let registry_bytes = preimage
        .strip_prefix(REGISTRY_DOMAIN)
        .ok_or(VaultError::InvalidOracleState)?;
    let registry = CfmRegistry::decode(registry_bytes)?;
    if registry.product() != product || registry.digest() != registry_hash {
        return Err(VaultError::InvalidOracleState.into());
    }
    let bytes = CfmMonthPolicy::initial_bytes(
        program,
        market_info.key,
        month_info.key,
        product,
        &market.instrument.underlying_id,
        market.instrument.expiry_ts,
    )?;
    let (address, bump) = CfmMonthPolicy::address(program, month_info.key);
    if policy_info.key != &address || policy_info.is_signer || !policy_info.is_writable {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program, policy_info)?;
    let (_, month_bump) =
        derive_oracle_month_pda(program, market_info.key, market.instrument.expiry_ts);
    let pending = pending_month(*market_info.key, *oracle.key, month_bump);
    validate_create_only_program_account_target(program, month_info)?;
    create_program_account(
        payer,
        month_info,
        system,
        program,
        OracleMonthState::LEN,
        &[
            ORACLE_MONTH_PDA_SEED,
            market_info.key.as_ref(),
            &market.instrument.expiry_ts.to_le_bytes(),
            &[month_bump],
        ],
    )?;
    store_oracle_month_state(month_info, &pending)?;
    create_program_account(
        payer,
        policy_info,
        system,
        program,
        CfmMonthPolicy::LEN,
        &[CFM_MONTH_POLICY_SEED, month_info.key.as_ref(), &[bump]],
    )?;
    if policy_info.owner != program || policy_info.data_len() != bytes.len() {
        return Err(VaultError::InvalidOracleState.into());
    }
    policy_info.try_borrow_mut_data()?.copy_from_slice(&bytes);
    Ok(())
}

fn pending_month(market: Pubkey, authority: Pubkey, bump: u8) -> OracleMonthState {
    OracleMonthState {
        is_initialized: true,
        bump,
        market,
        authority,
        account_discriminator: OracleMonthState::CFM_DISCRIMINATOR,
        account_version: OracleMonthState::CFM_VERSION,
        schedule_version: LAUNCH_SCHEDULE_VERSION,
        ..OracleMonthState::default()
    }
}

pub(super) fn require_pending_month(
    program: &Pubkey,
    market: &Pubkey,
    month_info: &AccountInfo,
) -> ProgramResult {
    let (address, bump) = derive_oracle_month_pda(
        program,
        market,
        crate::oracle_parent_proxy::policy::SEPTEMBER_EXPIRY_TS,
    );
    let current = load_oracle_month_state(month_info, program)?;
    if month_info.key != &address
        || current.authority == Pubkey::default()
        || current != pending_month(*market, current.authority, bump)
    {
        return Err(VaultError::InvalidOracleState.into());
    }
    Ok(())
}

pub(super) fn initialize_month(
    program: &Pubkey,
    accounts: &[AccountInfo],
    product: u8,
    base: u64,
) -> ProgramResult {
    if accounts.len() != 12 || base == 0 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let market = load_valid_market(program, &accounts[2])?;
    let registry_hash = match product {
        0 => RAMX_REGISTRY_HASH,
        1 => NANDX_REGISTRY_HASH,
        _ => return Err(VaultError::InvalidOracleState.into()),
    };
    let preimage =
        oracle_evidence::sealed_definition_preimage(program, &accounts[11], &registry_hash)?;
    let registry = CfmRegistry::decode(
        preimage
            .strip_prefix(REGISTRY_DOMAIN)
            .ok_or(VaultError::InvalidOracleState)?,
    )?;
    CfmMonthPolicy::load(
        program,
        &accounts[10],
        accounts[2].key,
        accounts[3].key,
        market.instrument.expiry_ts,
        &registry,
    )?;
    require_pending_month(program, accounts[2].key, &accounts[3])?;
    let (required_sku_count, required_sku_root) = match product {
        0 => (52, RAMX_TERMINAL_ROOT),
        1 => (48, NANDX_TERMINAL_ROOT),
        _ => return Err(VaultError::InvalidOracleState.into()),
    };
    process_initialize_oracle_month_with_registry(
        program,
        &accounts[..10],
        InitializeOracleMonthV5Params {
            scramble_start_ts: 0,
            listing_ts: 0,
            settlement_base_oracle_atomic: base,
            required_sku_count,
            required_sku_root,
        },
        Some(&registry),
    )
}

/// Legacy entrypoints cannot operate on CFM month/coverage records. This runs
/// after known-tag classification and before any business handler or CPI.
pub(super) fn reject_cfm_accounts(program: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    for info in accounts {
        if info.owner != program
            || ![OracleMonthState::LEN, OracleSkuCoverageManifest::LEN].contains(&info.data_len())
        {
            continue;
        }
        let data = info.try_borrow_data()?;
        if (data.len() == OracleMonthState::LEN
            && data[2..5] == OracleMonthState::CFM_DISCRIMINATOR)
            || (data.len() == OracleSkuCoverageManifest::LEN
                && data[2..5] == OracleSkuCoverageManifest::CFM_DISCRIMINATOR)
        {
            return Err(VaultError::InvalidOracleState.into());
        }
    }
    Ok(())
}
