use super::*;
use crate::compact_error::CompactAccountInfo;
use solana_program::program_pack::Pack;
use spl_token::state::{
    Account as SplTokenAccount, AccountState as SplAccountState, Mint as SplMint,
};

fn validate_test_creation_target(
    program_id: &Pubkey,
    owner: &Pubkey,
    is_writable: bool,
    executable: bool,
    data_len: usize,
) -> ProgramResult {
    let key = Pubkey::new_unique();
    let mut lamports = 1;
    let mut data = vec![0; data_len];
    let account = AccountInfo::new(
        &key,
        false,
        is_writable,
        &mut lamports,
        &mut data,
        owner,
        executable,
        0,
    );
    validate_create_only_program_account_target(program_id, &account)
}

#[test]
fn generic_program_state_loader_requires_exact_length_and_zero_padding() {
    let program_id = Pubkey::new_unique();
    let key = Pubkey::new_unique();
    let mut canonical = 42_u64.try_to_vec().unwrap();
    canonical.extend_from_slice(&[0; 8]);
    with_test_account_info(&key, &program_id, canonical, |info| {
        assert_eq!(load_state::<u64>(info, &program_id), Ok(42));
    });

    let mut noncanonical = 42_u64.try_to_vec().unwrap();
    noncanonical.extend_from_slice(&[0, 0, 1]);
    with_test_account_info(&key, &program_id, noncanonical, |info| {
        assert_eq!(
            load_state::<u64>(info, &program_id),
            Err(ProgramError::Custom(
                OracleError::InvalidConfigAccount as u32
            ))
        );
    });

    let mut overlong_zero_padded = 42_u64.try_to_vec().unwrap();
    overlong_zero_padded.extend_from_slice(&[0; 9]);
    with_test_account_info(&key, &program_id, overlong_zero_padded, |info| {
        assert_eq!(
            load_state::<u64>(info, &program_id),
            Err(ProgramError::Custom(
                OracleError::InvalidConfigAccount as u32
            ))
        );
    });
}

#[test]
fn oracle_month_loader_accepts_only_exact_current_layout() {
    let program_id = Pubkey::new_unique();
    let key = Pubkey::new_unique();
    let current = OracleMonthState {
        is_initialized: true,
        bump: 7,
        account_discriminator: OracleMonthState::ACCOUNT_DISCRIMINATOR,
        account_version: OracleMonthState::ACCOUNT_VERSION,
        period: Pubkey::new_unique(),
        accepted_cash_update_count: 3,
        ..OracleMonthState::default()
    };
    let mut current_data = current.try_to_vec().expect("serialize month");
    current_data.resize(OracleMonthState::LEN, 0);
    with_test_account_info(&key, &program_id, current_data, |info| {
        let mut loaded = load_oracle_month_state(info, &program_id).expect("load month");
        assert_eq!(loaded, current);
        loaded.last_updated_slot = 10;
        store_oracle_month_state(info, &loaded).expect("store month");
        assert_eq!(info.data_len(), OracleMonthState::LEN);
    });

    for invalid_len in [OracleMonthState::LEN - 1, OracleMonthState::LEN + 1] {
        with_test_account_info(&key, &program_id, vec![0; invalid_len], |info| {
            assert_eq!(
                load_oracle_month_state(info, &program_id),
                Err(ProgramError::Custom(
                    OracleError::InvalidOracleMonthAccount as u32
                ))
            );
        });
    }
    with_test_account_info(
        &key,
        &Pubkey::new_unique(),
        exact_test_state_data(&current, OracleMonthState::LEN),
        |info| {
            assert_eq!(
                load_oracle_month_state(info, &program_id),
                Err(ProgramError::Custom(
                    OracleError::InvalidOracleMonthAccount as u32
                ))
            );
        },
    );
}

#[test]
fn oracle_month_loader_accepts_only_the_current_schedule_generation() {
    let program = Pubkey::new_unique();
    let key = Pubkey::new_unique();
    for schedule in [2, 3, 4, 5] {
        for phase in [OraclePhase::Opening, OraclePhase::Game] {
            let month = OracleMonthState {
                is_initialized: true,
                account_discriminator: OracleMonthState::ACCOUNT_DISCRIMINATOR,
                account_version: OracleMonthState::ACCOUNT_VERSION,
                schedule_version: schedule,
                phase,
                ..OracleMonthState::default()
            };
            let data = exact_test_state_data(&month, OracleMonthState::LEN);
            // Wire decoding stays available even when business loading rejects it.
            assert_eq!(
                crate::fixed_codec::decode_oracle_month(
                    &data,
                    OracleError::InvalidOracleMonthAccount,
                )
                .unwrap(),
                month
            );
            let accepted = schedule == 2;
            assert_eq!(month.has_current_layout(), accepted);
            with_test_account_info(&key, &program, data.clone(), |info| {
                let result = load_oracle_month_state(info, &program);
                if accepted {
                    assert_eq!(result, Ok(month));
                } else {
                    assert_eq!(result, Err(OracleError::InvalidOracleMonthAccount.into()));
                }
                assert_eq!(&**info.try_data().unwrap(), data.as_slice());
            });
        }
    }
}

#[test]
fn fresh_initialize_is_paused_and_accepts_only_safe_prefunded_target() {
    let program_id = Pubkey::new_unique();
    let admin = Pubkey::new_unique();
    let (config_key, bump) =
        Pubkey::find_program_address(&[CURRENT_STATE_NAMESPACE_SEED, VAULT_PDA_SEED], &program_id);
    let config = initial_vault_config(bump, admin, Pubkey::new_unique(), Pubkey::new_unique());
    assert!(config.paused, "fresh initialization must fail closed");
    assert_eq!(config.admin, admin);
    assert_eq!(config.oracle_authority, admin);
    assert!(config.has_current_layout());

    let system_owner = system_program::id();
    let mut prefunded_lamports = 1;
    let mut empty_data = [];
    let prefunded = AccountInfo::new(
        &config_key,
        false,
        true,
        &mut prefunded_lamports,
        &mut empty_data,
        &system_owner,
        false,
        0,
    );
    assert_eq!(
        validate_vault_config_initialization_target(&program_id, &prefunded),
        Ok(())
    );
    drop(prefunded);
    assert_eq!(prefunded_lamports, 1);

    let foreign_owner = Pubkey::new_unique();
    let mut foreign_lamports = 1;
    let mut foreign_data = [];
    let foreign = AccountInfo::new(
        &config_key,
        false,
        true,
        &mut foreign_lamports,
        &mut foreign_data,
        &foreign_owner,
        false,
        0,
    );
    assert_eq!(
        validate_vault_config_initialization_target(&program_id, &foreign),
        Err(ProgramError::Custom(OracleError::InvalidPda as u32))
    );

    let mut data_lamports = 1;
    let mut data = [1_u8];
    let data_bearing = AccountInfo::new(
        &config_key,
        false,
        true,
        &mut data_lamports,
        &mut data,
        &system_owner,
        false,
        0,
    );
    assert_eq!(
        validate_vault_config_initialization_target(&program_id, &data_bearing),
        Err(ProgramError::Custom(OracleError::InvalidPda as u32))
    );

    let mut initialized_lamports = 1;
    let mut initialized_data = [];
    let initialized = AccountInfo::new(
        &config_key,
        false,
        true,
        &mut initialized_lamports,
        &mut initialized_data,
        &program_id,
        false,
        0,
    );
    assert_eq!(
        validate_vault_config_initialization_target(&program_id, &initialized),
        Err(ProgramError::Custom(OracleError::AlreadyInitialized as u32))
    );
}

#[test]
fn upgrade_authority_gate_reads_only_canonical_program_data() {
    let program_id = Pubkey::new_unique();
    let authority = Pubkey::new_unique();
    let (program_data_key, _) =
        Pubkey::find_program_address(&[program_id.as_ref()], &bpf_loader_upgradeable::ID);
    let metadata = |tag: u32, recorded: Option<Pubkey>| {
        let mut data = vec![0u8; PROGRAM_DATA_TEST_LEN];
        data[0..4].copy_from_slice(&tag.to_le_bytes());
        data[4..12].copy_from_slice(&9u64.to_le_bytes());
        if let Some(recorded) = recorded {
            data[12] = 1;
            data[13..45].copy_from_slice(recorded.as_ref());
        }
        data
    };
    const PROGRAM_DATA_TEST_LEN: usize = 45 + 16;
    let loader = bpf_loader_upgradeable::ID;
    with_test_account_info(
        &program_data_key,
        &loader,
        metadata(3, Some(authority)),
        |info| {
            assert_eq!(
                require_upgrade_authority(&program_id, info, &authority),
                Ok(())
            )
        },
    );
    let invalid = Err(ProgramError::Custom(
        OracleError::InvalidUpgradeAuthority as u32,
    ));
    for (key, owner, data, signer) in [
        (
            program_data_key,
            loader,
            metadata(3, Some(authority)),
            Pubkey::new_unique(),
        ),
        (program_data_key, loader, metadata(3, None), authority),
        (
            program_data_key,
            loader,
            metadata(2, Some(authority)),
            authority,
        ),
        (
            Pubkey::new_unique(),
            loader,
            metadata(3, Some(authority)),
            authority,
        ),
        (
            program_data_key,
            Pubkey::new_unique(),
            metadata(3, Some(authority)),
            authority,
        ),
        (program_data_key, loader, vec![3, 0, 0, 0], authority),
    ] {
        with_test_account_info(&key, &owner, data, |info| {
            assert_eq!(
                require_upgrade_authority(&program_id, info, &signer),
                invalid
            );
        });
    }
}

#[test]
fn paused_bootstrap_setup_accepts_only_current_oracle_authority() {
    let program_id = Pubkey::new_unique();
    let admin = Pubkey::new_unique();
    let oracle = Pubkey::new_unique();
    let outsider = Pubkey::new_unique();
    let (config_key, bump) =
        Pubkey::find_program_address(&[CURRENT_STATE_NAMESPACE_SEED, VAULT_PDA_SEED], &program_id);
    let config = OracleConfig {
        is_initialized: true,
        bump,
        admin,
        oracle_authority: oracle,
        usdc_mint: Pubkey::new_unique(),
        bond_vault: Pubkey::new_unique(),
        paused: true,
        account_discriminator: OracleConfig::ACCOUNT_DISCRIMINATOR,
        account_version: OracleConfig::ACCOUNT_VERSION,
    };
    let system_owner = system_program::id();
    let mut admin_lamports = 0;
    let mut admin_data = [];
    let mut oracle_lamports = 0;
    let mut oracle_data = [];
    let mut outsider_lamports = 0;
    let mut outsider_data = [];
    let mut unsigned_lamports = 0;
    let mut unsigned_data = [];
    let mut config_lamports = 1;
    let mut config_data = exact_test_state_data(&config, OracleConfig::LEN);
    let admin_info = AccountInfo::new(
        &admin,
        true,
        false,
        &mut admin_lamports,
        &mut admin_data,
        &system_owner,
        false,
        0,
    );
    let oracle_info = AccountInfo::new(
        &oracle,
        true,
        false,
        &mut oracle_lamports,
        &mut oracle_data,
        &system_owner,
        false,
        0,
    );
    let outsider_info = AccountInfo::new(
        &outsider,
        true,
        false,
        &mut outsider_lamports,
        &mut outsider_data,
        &system_owner,
        false,
        0,
    );
    let unsigned_oracle_info = AccountInfo::new(
        &oracle,
        false,
        false,
        &mut unsigned_lamports,
        &mut unsigned_data,
        &system_owner,
        false,
        0,
    );
    let config_info = AccountInfo::new(
        &config_key,
        false,
        false,
        &mut config_lamports,
        &mut config_data,
        &program_id,
        false,
        0,
    );

    assert_eq!(
        validate_current_oracle_authority_allow_paused(&program_id, &oracle_info, &config_info),
        Ok(config.clone())
    );
    assert_eq!(validate_current_account_creation_payer(&admin_info), Ok(()));
    assert_eq!(
        validate_current_account_creation_payer(&oracle_info),
        Ok(())
    );
    assert_eq!(
        validate_current_account_creation_payer(&outsider_info),
        Ok(())
    );
    assert_eq!(
        validate_current_account_creation_payer(&unsigned_oracle_info),
        Err(ProgramError::MissingRequiredSignature)
    );
    assert_eq!(
        validate_current_oracle_authority_allow_paused(&program_id, &admin_info, &config_info),
        Err(ProgramError::Custom(OracleError::Unauthorized as u32))
    );
    assert_eq!(
        validate_current_oracle_authority_allow_paused(&program_id, &outsider_info, &config_info),
        Err(ProgramError::Custom(OracleError::Unauthorized as u32))
    );
    assert_eq!(
        validate_current_oracle_authority_allow_paused(
            &program_id,
            &unsigned_oracle_info,
            &config_info
        ),
        Err(ProgramError::MissingRequiredSignature)
    );
    assert_eq!(
        validate_oracle_authority(&program_id, &oracle_info, &config_info),
        Err(ProgramError::Custom(OracleError::ContractPaused as u32))
    );
}

#[test]
fn program_account_creation_targets_are_strictly_create_only() {
    let program_id = Pubkey::new_unique();
    let system_owner = system_program::id();
    let foreign_owner = Pubkey::new_unique();

    assert_eq!(
        validate_test_creation_target(&program_id, &system_owner, true, false, 0),
        Ok(())
    );
    assert_eq!(
        validate_test_creation_target(&program_id, &program_id, true, false, 0),
        Err(ProgramError::Custom(OracleError::AlreadyInitialized as u32))
    );
    for invalid in [
        validate_test_creation_target(&program_id, &foreign_owner, true, false, 0),
        validate_test_creation_target(&program_id, &system_owner, true, false, 1),
        validate_test_creation_target(&program_id, &system_owner, true, true, 0),
        validate_test_creation_target(&program_id, &system_owner, false, false, 0),
    ] {
        assert_eq!(
            invalid,
            Err(ProgramError::Custom(OracleError::InvalidPda as u32))
        );
    }
}

#[test]
fn bootstrap_vault_governance_succeeds_once_and_replay_fails_closed() {
    let program_id = Pubkey::new_unique();
    let (config_key, bump) =
        Pubkey::find_program_address(&[CURRENT_STATE_NAMESPACE_SEED, VAULT_PDA_SEED], &program_id);
    let (registry_key, _) = derive_settlement_signer_registry_pda(&program_id);
    let bootstrap_admin = Pubkey::new_unique();
    let new_oracle = Pubkey::new_unique();
    let config = initial_vault_config(
        bump,
        bootstrap_admin,
        Pubkey::new_unique(),
        Pubkey::new_unique(),
    );
    let system_owner = system_program::id();
    let mut admin_lamports = 0;
    let mut admin_data = [];
    let mut oracle_lamports = 0;
    let mut oracle_data = [];
    let mut config_lamports = 1;
    let mut config_data = exact_test_state_data(&config, OracleConfig::LEN);
    let mut registry_lamports = 0;
    let mut registry_data = [];
    let accounts = vec![
        AccountInfo::new(
            &bootstrap_admin,
            true,
            false,
            &mut admin_lamports,
            &mut admin_data,
            &system_owner,
            false,
            0,
        ),
        AccountInfo::new(
            &new_oracle,
            true,
            false,
            &mut oracle_lamports,
            &mut oracle_data,
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
            &registry_key,
            false,
            false,
            &mut registry_lamports,
            &mut registry_data,
            &system_owner,
            false,
            0,
        ),
    ];
    let params = BootstrapVaultGovernanceV2Params {
        new_oracle_authority: new_oracle,
    };
    // The duplicated authority cannot hand the oracle role to itself.
    assert_eq!(
        process_bootstrap_vault_governance_v2(
            &program_id,
            &accounts,
            BootstrapVaultGovernanceV2Params {
                new_oracle_authority: bootstrap_admin,
            },
        ),
        Err(ProgramError::Custom(
            OracleError::SettlementSignerGovernanceRequired as u32
        ))
    );
    assert_eq!(
        process_bootstrap_vault_governance_v2(&program_id, &accounts, params.clone()),
        Ok(())
    );
    assert_eq!(
        process_bootstrap_vault_governance_v2(&program_id, &accounts, params),
        Err(ProgramError::Custom(
            OracleError::SettlementSignerGovernanceRequired as u32
        ))
    );
    drop(accounts);
    let updated = <OracleConfig as borsh::BorshDeserialize>::try_from_slice(&config_data).unwrap();
    assert_eq!(updated.admin, bootstrap_admin);
    assert_eq!(updated.oracle_authority, new_oracle);
    assert!(updated.paused);
}

#[test]
fn governed_activation_validates_registry_custody_and_is_the_only_unpause() {
    let program_id = Pubkey::new_unique();
    let admin = Pubkey::new_unique();
    let oracle = Pubkey::new_unique();
    let recovery = Pubkey::new_unique();
    let collateral_mint = Pubkey::new_unique();
    let collateral_vault = Pubkey::new_unique();
    let (config_key, config_bump) =
        Pubkey::find_program_address(&[CURRENT_STATE_NAMESPACE_SEED, VAULT_PDA_SEED], &program_id);
    let (registry_key, registry_bump) = derive_settlement_signer_registry_pda(&program_id);
    let (set_key, set_bump) = derive_settlement_signer_set_pda(&program_id, 1);
    let config = OracleConfig {
        is_initialized: true,
        bump: config_bump,
        admin,
        oracle_authority: oracle,
        usdc_mint: collateral_mint,
        bond_vault: collateral_vault,
        paused: true,
        account_discriminator: OracleConfig::ACCOUNT_DISCRIMINATOR,
        account_version: OracleConfig::ACCOUNT_VERSION,
    };
    let registry = SettlementSignerRegistry {
        is_initialized: true,
        bump: registry_bump,
        account_discriminator: SettlementSignerRegistry::ACCOUNT_DISCRIMINATOR,
        account_version: SettlementSignerRegistry::ACCOUNT_VERSION,
        current_set: set_key,
        current_version: 1,
        pending_set: Pubkey::default(),
        pending_version: 0,
        recovery_authority: recovery,
        proposal_nonce: 0,
    };
    let mut signer_keys = (0..5).map(|_| Pubkey::new_unique()).collect::<Vec<_>>();
    signer_keys.sort_unstable();
    let mut signers = [Pubkey::default(); MAX_SETTLEMENT_SIGNER_COUNT];
    signers[..5].copy_from_slice(&signer_keys);
    let mut set = SettlementSignerSet {
        is_initialized: true,
        bump: set_bump,
        account_discriminator: SettlementSignerSet::ACCOUNT_DISCRIMINATOR,
        account_version: SettlementSignerSet::ACCOUNT_VERSION,
        registry: registry_key,
        version: 1,
        threshold: 3,
        signer_count: 5,
        signers,
        set_hash: [0; 32],
        rotation_delay_slots: MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS,
        proposed_slot: 0,
        activate_after_slot: 0,
        emergency: false,
        proposer: admin,
    };
    set.set_hash = set.compute_set_hash();

    let system_owner = system_program::id();
    let token_program = token_instruction::id();
    let mut admin_lamports = 0;
    let mut admin_data = [];
    let mut config_lamports = 1;
    let mut config_data = exact_test_state_data(&config, OracleConfig::LEN);
    let mut registry_lamports = 1;
    let mut registry_data = exact_test_state_data(&registry, SettlementSignerRegistry::LEN);
    let mut set_lamports = 1;
    let mut set_data = exact_test_state_data(&set, SettlementSignerSet::LEN);
    let mut mint_lamports = 1;
    let mut mint_data = vec![0; SplMint::LEN];
    SplMint::pack(
        SplMint {
            mint_authority: COption::None,
            supply: 0,
            decimals: COLLATERAL_MINT_DECIMALS,
            is_initialized: true,
            freeze_authority: COption::None,
        },
        &mut mint_data,
    )
    .unwrap();
    let mut vault_lamports = 1;
    let mut vault_data = vec![0; SplTokenAccount::LEN];
    SplTokenAccount::pack(
        SplTokenAccount {
            mint: collateral_mint,
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
            &admin,
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
            &registry_key,
            false,
            false,
            &mut registry_lamports,
            &mut registry_data,
            &program_id,
            false,
            0,
        ),
        AccountInfo::new(
            &set_key,
            false,
            false,
            &mut set_lamports,
            &mut set_data,
            &program_id,
            false,
            0,
        ),
        AccountInfo::new(
            &collateral_mint,
            false,
            false,
            &mut mint_lamports,
            &mut mint_data,
            &token_program,
            false,
            0,
        ),
        AccountInfo::new(
            &collateral_vault,
            false,
            false,
            &mut vault_lamports,
            &mut vault_data,
            &token_program,
            false,
            0,
        ),
        AccountInfo::new(
            &token_program,
            false,
            false,
            &mut token_program_lamports,
            &mut token_program_data,
            &system_owner,
            true,
            0,
        ),
    ];
    let params = ActivateVaultV2Params {
        expected_collateral_freeze_authority: None,
    };
    assert_eq!(
        process_activate_vault_v2(&program_id, &accounts, params.clone()),
        Ok(())
    );
    assert_eq!(
        process_activate_vault_v2(&program_id, &accounts, params),
        Err(ProgramError::Custom(
            OracleError::SettlementSignerGovernanceRequired as u32
        ))
    );
    drop(accounts);
    let activated =
        <OracleConfig as borsh::BorshDeserialize>::try_from_slice(&config_data).unwrap();
    assert!(!activated.paused);

    let no_freeze_mint = Mint::unpack(&mint_data).unwrap();
    assert_eq!(
        validate_expected_collateral_freeze_authority(&no_freeze_mint, Some(Pubkey::new_unique()),),
        Err(ProgramError::Custom(OracleError::InvalidMint as u32))
    );
}

#[test]
fn canonical_collateral_loader_rejects_wrong_size_and_bump() {
    let program_id = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    let (collateral_key, collateral_bump) = derive_user_collateral_pda(&program_id, &owner);
    let collateral = UserCollateral {
        is_initialized: true,
        bump: collateral_bump,
        owner,
        available_balance: 41,
        ..UserCollateral::default()
    };
    let collateral_data = exact_test_state_data(&collateral, UserCollateral::LEN);

    assert!(with_test_account_info(
        &collateral_key,
        &program_id,
        collateral_data.clone(),
        |info| load_canonical_user_collateral(&program_id, info, &owner)
    )
    .is_ok());

    assert_eq!(
        with_test_account_info(
            &collateral_key,
            &program_id,
            exact_test_state_data(&collateral, UserCollateral::LEN + 1),
            |info| load_canonical_user_collateral(&program_id, info, &owner).unwrap_err()
        ),
        ProgramError::Custom(OracleError::InvalidUserCollateralAccount as u32)
    );

    let mut wrong_collateral_bump = collateral.clone();
    wrong_collateral_bump.bump = wrong_collateral_bump.bump.wrapping_add(1);
    assert_eq!(
        with_test_account_info(
            &collateral_key,
            &program_id,
            exact_test_state_data(&wrong_collateral_bump, UserCollateral::LEN),
            |info| load_canonical_user_collateral(&program_id, info, &owner).unwrap_err()
        ),
        ProgramError::Custom(OracleError::InvalidUserCollateralAccount as u32)
    );

    // Another owner's ledger never loads under this owner, even at its own canonical address.
    let other = Pubkey::new_unique();
    assert_eq!(
        with_test_account_info(&collateral_key, &program_id, collateral_data, |info| {
            load_canonical_user_collateral(&program_id, info, &other).unwrap_err()
        }),
        ProgramError::Custom(OracleError::InvalidUserCollateralAccount as u32)
    );
}

#[test]
fn period_anchor_loader_binds_this_program_and_maps_product_and_expiry() {
    let program_id = Pubkey::new_unique();
    let product = [5u8; 32];
    let expiry = 1_803_859_200;
    let (key, bump) = ameba_oracle_interface::find_period_address(&program_id, &product, expiry);
    let period = Period::new_forming(bump, product, 1_796_083_200, expiry, PERIOD_VALUE_DECIMALS);
    let data = period.encode().to_vec();
    with_test_account_info(&key, &program_id, data.clone(), |info| {
        let anchor = load_valid_period_anchor(&program_id, info).unwrap();
        assert_eq!(anchor.underlying_id, product);
        assert_eq!(anchor.expiry_ts, expiry);
        assert_eq!(anchor.period, period);
    });
    let invalid = ProgramError::Custom(OracleError::InvalidPeriodAccount as u32);
    with_test_account_info(&key, &Pubkey::new_unique(), data.clone(), |info| {
        assert_eq!(
            load_valid_period_anchor(&program_id, info).unwrap_err(),
            invalid
        );
    });
    with_test_account_info(&Pubkey::new_unique(), &program_id, data.clone(), |info| {
        assert_eq!(
            load_valid_period_anchor(&program_id, info).unwrap_err(),
            invalid
        );
    });
    let mut corrupt = data;
    corrupt[0] ^= 1;
    with_test_account_info(&key, &program_id, corrupt, |info| {
        assert_eq!(
            load_valid_period_anchor(&program_id, info).unwrap_err(),
            invalid
        );
    });
}
