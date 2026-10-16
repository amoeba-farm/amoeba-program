use super::*;
use solana_program::program_pack::Pack;
use spl_token::state::{
    Account as SplTokenAccount, AccountState as SplAccountState, Mint as SplMint,
};

const PROCESSOR_SOURCE: &str = concat!(
    include_str!("../mod.rs"),
    include_str!("../account_addresses.rs"),
    include_str!("../account_io.rs"),
    include_str!("../account_io_tokens.rs"),
    include_str!("../adapters.rs"),
    include_str!("../collateral_accounts.rs"),
    include_str!("../config.rs"),
    include_str!("../dispatch.rs"),
    include_str!("../oracle_core.rs"),
    include_str!("../oracle_economics.rs"),
    include_str!("../oracle_schedule_validation.rs"),
    include_str!("../oracle_validation.rs"),
    include_str!("../period.rs"),
    include_str!("../schedule.rs"),
    include_str!("../settlement_signers.rs"),
    include_str!("../settlement_validation.rs"),
    include_str!("../sku_coverage.rs"),
    include_str!("../sku_manifest.rs"),
);

fn top_level_function_source<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("fn {name}(");
    let start = source
        .find(&marker)
        .or_else(|| source.find(&format!("fn {name}<")))
        .unwrap_or_else(|| panic!("missing function {name}"));
    let rest = &source[start..];
    let end = rest
        .get(1..)
        .and_then(|tail| {
            ["\nfn ", "\npub fn ", "\npub(super) fn ", "\npub(crate) fn "]
                .iter()
                .filter_map(|next| tail.find(next))
                .min()
        })
        .map(|offset| offset + 1)
        .unwrap_or(rest.len());
    &rest[..end]
}

#[allow(clippy::too_many_arguments)]
fn run_test_update_config(
    mut config: OracleConfig,
    stored_bump_delta: u8,
    is_initialized: bool,
    new_admin: Option<Pubkey>,
    new_oracle_authority: Option<Pubkey>,
    new_usdc_mint: Option<Pubkey>,
    new_bond_vault: Option<Pubkey>,
    paused: Option<bool>,
) -> (ProgramResult, OracleConfig, OracleConfig) {
    let program_id = Pubkey::new_unique();
    let (config_key, config_bump) =
        Pubkey::find_program_address(&[CURRENT_STATE_NAMESPACE_SEED, VAULT_PDA_SEED], &program_id);
    config.bump = config_bump.wrapping_add(stored_bump_delta);
    config.is_initialized = is_initialized;
    let original = config.clone();
    let admin_key = config.admin;
    let mint_key = config.usdc_mint;
    let vault_key = config.bond_vault;
    let token_program_key = token_instruction::id();
    let system_owner = system_program::id();
    let mut admin_lamports = 0;
    let mut admin_data = [];
    let mut config_lamports = 0;
    let mut config_data = exact_test_state_data(&config, OracleConfig::LEN);
    let mut mint_lamports = 0;
    let mut mint_data = vec![0; SplMint::LEN];
    SplMint::pack(
        SplMint {
            mint_authority: COption::None,
            supply: 0,
            decimals: 6,
            is_initialized: true,
            freeze_authority: COption::None,
        },
        &mut mint_data,
    )
    .unwrap();
    let mut vault_lamports = 0;
    let mut vault_data = vec![0; SplTokenAccount::LEN];
    SplTokenAccount::pack(
        SplTokenAccount {
            mint: mint_key,
            owner: config_key,
            amount: 0,
            delegate: COption::None,
            state: SplAccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 0,
            close_authority: COption::None,
        },
        &mut vault_data,
    )
    .unwrap();
    let mut token_program_lamports = 0;
    let mut token_program_data = [];
    let accounts = vec![
        AccountInfo::new(
            &admin_key,
            true,
            false,
            &mut admin_lamports,
            &mut admin_data,
            &system_owner,
            false,
            0,
        ),
        AccountInfo::new(
            &config_key,
            false,
            true,
            &mut config_lamports,
            &mut config_data,
            &program_id,
            false,
            0,
        ),
        AccountInfo::new(
            &mint_key,
            false,
            false,
            &mut mint_lamports,
            mint_data.as_mut_slice(),
            &token_program_key,
            false,
            0,
        ),
        AccountInfo::new(
            &vault_key,
            false,
            false,
            &mut vault_lamports,
            vault_data.as_mut_slice(),
            &token_program_key,
            false,
            0,
        ),
        AccountInfo::new(
            &token_program_key,
            false,
            false,
            &mut token_program_lamports,
            &mut token_program_data,
            &system_owner,
            true,
            0,
        ),
    ];
    let result = process_update_config(
        &program_id,
        &accounts,
        new_admin,
        new_oracle_authority,
        new_usdc_mint,
        new_bond_vault,
        paused,
    );
    drop(accounts);
    let stored = <OracleConfig as borsh::BorshDeserialize>::try_from_slice(&config_data).unwrap();
    (result, stored, original)
}

#[test]
fn update_config_rejects_uninitialized_wrong_bump_and_zero_authorities_without_mutation() {
    let base = OracleConfig {
        is_initialized: true,
        bump: 0,
        admin: Pubkey::new_unique(),
        oracle_authority: Pubkey::new_unique(),
        usdc_mint: Pubkey::new_unique(),
        bond_vault: Pubkey::new_unique(),
        paused: true,
        account_discriminator: OracleConfig::ACCOUNT_DISCRIMINATOR,
        account_version: OracleConfig::ACCOUNT_VERSION,
    };
    for (bump_delta, initialized) in [(1, true), (0, false)] {
        let (result, stored, original) = run_test_update_config(
            base.clone(),
            bump_delta,
            initialized,
            None,
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            result,
            Err(ProgramError::Custom(
                OracleError::InvalidConfigAccount as u32
            ))
        );
        assert_eq!(stored, original);
    }

    for (new_admin, new_oracle_authority) in [
        (Some(Pubkey::default()), None),
        (None, Some(Pubkey::default())),
    ] {
        let (result, stored, original) = run_test_update_config(
            base.clone(),
            0,
            true,
            new_admin,
            new_oracle_authority,
            None,
            None,
            None,
        );
        assert_eq!(
            result,
            Err(ProgramError::Custom(
                OracleError::InvalidConfigAccount as u32
            ))
        );
        assert_eq!(stored, original);
    }

    for new_vault in [Some(Pubkey::default()), Some(Pubkey::new_unique())] {
        let (result, stored, original) =
            run_test_update_config(base.clone(), 0, true, None, None, None, new_vault, None);
        assert_eq!(
            result,
            Err(ProgramError::Custom(
                OracleError::CollateralIdentityImmutable as u32
            ))
        );
        assert_eq!(stored, original);
    }

    let (result, stored, original) = run_test_update_config(
        base.clone(),
        0,
        true,
        None,
        None,
        Some(Pubkey::new_unique()),
        None,
        None,
    );
    assert_eq!(
        result,
        Err(ProgramError::Custom(
            OracleError::CollateralIdentityImmutable as u32
        ))
    );
    assert_eq!(stored, original);

    let (result, stored, original) = run_test_update_config(
        base.clone(),
        0,
        true,
        Some(base.admin),
        Some(base.oracle_authority),
        Some(base.usdc_mint),
        Some(base.bond_vault),
        None,
    );
    assert_eq!(result, Ok(()));
    assert_eq!(stored.admin, original.admin);
    assert_eq!(stored.oracle_authority, original.oracle_authority);
    assert_eq!(stored.usdc_mint, original.usdc_mint);
    assert_eq!(stored.bond_vault, original.bond_vault);
    assert!(stored.has_current_layout());

    let (result, stored, original) = run_test_update_config(
        base.clone(),
        0,
        true,
        Some(Pubkey::new_unique()),
        None,
        None,
        None,
        None,
    );
    assert_eq!(
        result,
        Err(OracleError::SettlementSignerGovernanceRequired.into())
    );
    assert_eq!(stored, original);

    let (result, stored, original) = run_test_update_config(
        base.clone(),
        0,
        true,
        Some(base.oracle_authority),
        None,
        None,
        None,
        None,
    );
    assert_eq!(
        result,
        Err(OracleError::SettlementSignerGovernanceRequired.into())
    );
    assert_eq!(stored, original);

    let (result, stored, original) =
        run_test_update_config(base, 0, true, None, None, None, None, Some(false));
    assert_eq!(
        result,
        Err(OracleError::SettlementSignerGovernanceRequired.into())
    );
    assert_eq!(stored, original, "tag 2 must never unpause the config");
}

#[test]
fn governed_three_of_five_signer_set_is_version_bound_and_member_order_independent() {
    let program_id = Pubkey::new_unique();
    let (registry, _) = derive_settlement_signer_registry_pda(&program_id);
    let (_, bump) = derive_settlement_signer_set_pda(&program_id, 1);
    let mut signers = [Pubkey::default(); MAX_SETTLEMENT_SIGNER_COUNT];
    for (index, slot) in signers.iter_mut().take(5).enumerate() {
        *slot = Pubkey::new_from_array([(index + 1) as u8; 32]);
    }
    let set = build_settlement_signer_set(
        registry,
        bump,
        2,
        3,
        5,
        signers,
        MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS,
        10,
        10,
        false,
        Pubkey::new_unique(),
    )
    .expect("3-of-5 governed signer set");
    assert_eq!(set.threshold, 3);
    assert_eq!(set.signer_count, 5);
    assert_eq!(set.compute_set_hash(), set.set_hash);
    // A two-of-five threshold is not a strict majority and never forms a governed set.
    assert!(build_settlement_signer_set(
        registry,
        bump,
        2,
        2,
        5,
        signers,
        MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS,
        11,
        12,
        false,
        Pubkey::new_unique(),
    )
    .is_err());
    let mut unsorted = signers;
    unsorted.swap(0, 1);
    assert!(build_settlement_signer_set(
        registry,
        bump,
        2,
        3,
        5,
        unsorted,
        MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS,
        10,
        10,
        false,
        Pubkey::new_unique(),
    )
    .is_err());
    assert_eq!(
        settlement_signer_set_excludes_governance(&set, &[signers[2]]),
        Err(OracleError::InvalidSettlementSignerConfiguration.into())
    );
    assert_eq!(
        settlement_signer_set_excludes_governance(&set, &[Pubkey::new_unique()]),
        Ok(())
    );
}

#[test]
fn paused_bootstrap_setup_and_value_paths_use_their_exact_pause_guards() {
    let source = PROCESSOR_SOURCE;
    for function_name in [
        "process_configure_oracle_economics_template_v2",
        "process_configure_oracle_product_sku_manifest",
        "process_initialize_oracle_month_v5",
        "initialize_oracle_month",
        "process_create_period",
    ] {
        let function_source = top_level_function_source(source, function_name);
        assert!(
            !function_source.contains("load_active_vault_config")
                && !function_source.contains("load_active_oracle_vault_config")
                && !function_source.contains("validate_oracle_authority(")
                && !function_source.contains("if config.paused"),
            "{function_name} is create-only bootstrap setup and must remain possible while the config is paused"
        );
    }

    let bootstrap_governance =
        top_level_function_source(source, "process_bootstrap_vault_governance_v2");
    assert!(bootstrap_governance.contains("!config.paused"));
    let activation = top_level_function_source(source, "process_activate_vault_v2");
    assert!(activation.contains("!config.paused"));
    assert!(activation.contains("config.paused = false"));
    assert_eq!(
        source.matches("config.paused = false").count(),
        1,
        "only governed activation unpauses"
    );

    assert!(
        top_level_function_source(source, "process_initialize_oracle_month_v5")
            .contains("initialize_oracle_month(program_id, accounts, params)")
    );
    for function_name in ["initialize_oracle_month", "process_create_period"] {
        assert!(
            top_level_function_source(source, function_name)
                .contains("validate_current_oracle_authority_allow_paused"),
            "{function_name} must preserve the oracle/admin role split during paused bootstrap"
        );
    }

    let withdrawal = top_level_function_source(source, "process_collateral_withdrawal");
    assert!(
        withdrawal.contains("load_active_vault_config"),
        "admin-assisted withdrawal must load the exact canonical active config"
    );
    let deposit_collateral_source = top_level_function_source(source, "process_deposit_collateral");
    assert!(deposit_collateral_source.contains("process_deposit("));
    let initialize = top_level_function_source(source, "process_initialize_config");
    let authority = initialize
        .find("require_upgrade_authority")
        .expect("tag 0 checks the upgrade authority");
    let creation = initialize
        .find("create_program_account")
        .expect("tag 0 creates the config");
    assert!(authority < creation);
}
