//! Config initialization and period creation, end to end through the processor.

mod common;

use ameba_oracle::{
    error::OracleError,
    instruction,
    state::{find_config_address, OracleConfig},
};
use ameba_oracle_interface::{find_period_address, Period, PeriodStatus};
use common::*;
use solana_sdk::{
    account::Account,
    instruction::{AccountMeta, Instruction, InstructionError},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};
use solana_sdk_ids::system_program;

#[tokio::test]
async fn config_requires_upgrade_authority_and_is_create_only() {
    let mut h = start().await;
    let (config_key, bump) = find_config_address(&program());

    let impostor = Keypair::new();
    let ix = instruction::initialize_config(
        &program(),
        &h.ctx.payer.pubkey(),
        &impostor.pubkey(),
        impostor.pubkey(),
        &h.usdc_mint,
    );
    assert_oracle_error(
        h.send(ix, &[&impostor]).await,
        OracleError::InvalidUpgradeAuthority,
    );
    assert!(h.account(config_key).await.is_none());

    h.initialize().await.unwrap();
    let stored = h.account(config_key).await.unwrap();
    assert_eq!(stored.owner, program());
    let expected = OracleConfig {
        is_initialized: true,
        bump,
        admin: h.admin.pubkey(),
        oracle_authority: h.admin.pubkey(),
        usdc_mint: h.usdc_mint,
        bond_vault: h.bond_vault(),
        paused: true,
        account_discriminator: OracleConfig::ACCOUNT_DISCRIMINATOR,
        account_version: OracleConfig::ACCOUNT_VERSION,
    };
    assert_eq!(h.config().await, expected);
    // The bond vault is the config PDA's empty associated token account for the mint.
    let vault = h.account(h.bond_vault()).await.expect("bond vault exists");
    assert_eq!(vault.owner, spl_token::id());
    assert_eq!(h.token_balance(h.bond_vault()).await, 0);

    // A retry with a different admin neither succeeds nor resets state.
    h.admin = Keypair::new();
    assert_oracle_error(h.initialize().await, OracleError::AlreadyInitialized);
    assert_eq!(h.account(config_key).await.unwrap(), stored);
}

#[tokio::test]
async fn config_rejects_immutable_program() {
    let mut h = start_with(false, vec![]).await;
    assert_oracle_error(h.initialize().await, OracleError::InvalidUpgradeAuthority);
}

#[tokio::test]
async fn config_requires_a_six_decimal_mint_and_the_canonical_bond_vault() {
    let mut h = start().await;
    let (config_key, _) = find_config_address(&program());
    let mint_authority = h.usdc_mint_authority.pubkey();
    let nine_decimals = h.create_mint(&mint_authority, 9, None).await;
    assert_oracle_error(
        h.initialize_with_mint(nine_decimals).await,
        OracleError::InvalidMint,
    );

    let mut wrong_vault = instruction::initialize_config(
        &program(),
        &h.ctx.payer.pubkey(),
        &h.upgrade_authority.pubkey(),
        h.admin.pubkey(),
        &h.usdc_mint,
    );
    let payer = h.ctx.payer.pubkey();
    let mint = h.usdc_mint;
    let other_vault = h.create_token_account(&mint, &config_key).await;
    wrong_vault.accounts[6] = AccountMeta::new(other_vault, false);
    let upgrade_authority = h.upgrade_authority.insecure_clone();
    assert_oracle_error(
        h.send(wrong_vault, &[&upgrade_authority]).await,
        OracleError::InvalidTokenAccount,
    );

    let zero_admin = instruction::initialize_config(
        &program(),
        &payer,
        &h.upgrade_authority.pubkey(),
        Pubkey::default(),
        &h.usdc_mint,
    );
    assert_oracle_error(
        h.send(zero_admin, &[&upgrade_authority]).await,
        OracleError::InvalidConfigAccount,
    );
    assert!(h.account(config_key).await.is_none());
    h.initialize().await.unwrap();
}

#[tokio::test]
async fn period_creation_is_authority_gated_validated_and_create_only() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    let authority = h.oracle_authority.insecure_clone();
    let ramx = product("RAMX");

    let outsider = Keypair::new();
    assert_oracle_error(
        h.create_period(&outsider, ramx, PLANNED, EXPIRY, 6).await,
        OracleError::Unauthorized,
    );
    for (product_id, planned, expiry, decimals) in [
        ([0; 32], PLANNED, EXPIRY, 6),
        (ramx, EXPIRY, EXPIRY, 6),
        (ramx, OCT_01_2026 - 86_400, OCT_01_2026, 6),
        (ramx, PLANNED, EXPIRY, 19),
        (ramx, PLANNED, EXPIRY, 9),
        (ramx, PLANNED, EXPIRY, 0),
    ] {
        assert_oracle_error(
            h.create_period(&authority, product_id, planned, expiry, decimals)
                .await,
            OracleError::InvalidPeriodSchedule,
        );
    }

    h.create_period(&authority, ramx, PLANNED, EXPIRY, 6)
        .await
        .unwrap();
    let period = h.period(ramx, EXPIRY).await;
    assert_eq!(period.status, PeriodStatus::Forming);
    assert_eq!(
        period,
        Period::new_forming(
            find_period_address(&program(), &ramx, EXPIRY).1,
            ramx,
            PLANNED,
            EXPIRY,
            6
        )
    );
    assert!(!period.is_tradable_at(PLANNED));
    assert_eq!(period.settlement(), None);

    // A late period may still open after its planned listing, before expiry.
    h.create_period(&authority, product("NANDX"), OCT_01_2026, JAN_01_2027, 6)
        .await
        .unwrap();

    assert_oracle_error(
        h.create_period(&authority, ramx, PLANNED, EXPIRY, 6).await,
        OracleError::AlreadyInitialized,
    );
    assert_eq!(h.period(ramx, EXPIRY).await, period);
}

/// The localnet dev profile relaxes this (tests/dev_bootstrap.rs); every other build enforces it.
#[cfg(not(any(feature = "localnet-dev-authority", feature = "localnet-governance")))]
#[tokio::test]
async fn period_creation_enforces_the_rolling_three_month_ladder() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    let authority = h.oracle_authority.insecure_clone();
    let ramx = product("RAMX");
    for (planned, expiry) in [
        // One, two and four months.
        (DEC_01_2026, JAN_01_2027),
        (DEC_01_2026, FEB_01_2027),
        (DEC_01_2026, APR_01_2027),
        // Not the protocol's midnight-UTC roll boundary.
        (DEC_01_2026 + 1, MAR_01_2027 + 1),
        (DEC_01_2026 + 3_600, MAR_01_2027),
        // Not the first day of the month.
        (DEC_01_2026 + 86_400, MAR_01_2027 + 86_400),
    ] {
        assert_oracle_error(
            h.create_period(&authority, ramx, planned, expiry, 6).await,
            OracleError::InvalidOracleMaturityLadder,
        );
    }
    for (planned, expiry) in [
        (NOV_01_2026, FEB_01_2027),
        (DEC_01_2026, MAR_01_2027),
        (JAN_01_2027, APR_01_2027),
    ] {
        h.create_period(&authority, ramx, planned, expiry, 6)
            .await
            .unwrap();
        assert_eq!(h.period(ramx, expiry).await.planned_listing_ts, planned);
    }
}

#[tokio::test]
async fn period_creation_follows_the_split_oracle_authority_and_works_while_paused() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    h.split_oracle_authority().await;
    assert!(h.config().await.paused);
    let admin = h.admin.insecure_clone();
    let oracle = h.oracle_authority.insecure_clone();
    let ramx = product("RAMX");
    assert_oracle_error(
        h.create_period(&admin, ramx, PLANNED, EXPIRY, 6).await,
        OracleError::Unauthorized,
    );
    h.create_period(&oracle, ramx, PLANNED, EXPIRY, 6)
        .await
        .unwrap();
}

#[tokio::test]
async fn prefunded_period_address_is_accepted_and_squats_reject() {
    let prefunded = product("PREFUNDED");
    let foreign = product("FOREIGN");
    let data_bearing = product("DATA");
    let account = |owner: Pubkey, data: Vec<u8>| Account {
        lamports: 1_000,
        data,
        owner,
        executable: false,
        rent_epoch: 0,
    };
    let mut h = start_with(
        true,
        vec![
            (
                find_period_address(&program(), &prefunded, EXPIRY).0,
                account(system_program::id(), vec![]),
            ),
            (
                find_period_address(&program(), &foreign, EXPIRY).0,
                account(Pubkey::new_unique(), vec![]),
            ),
            (
                find_period_address(&program(), &data_bearing, EXPIRY).0,
                account(system_program::id(), vec![0; 8]),
            ),
        ],
    )
    .await;
    h.initialize().await.unwrap();
    let authority = h.oracle_authority.insecure_clone();

    h.create_period(&authority, prefunded, PLANNED, EXPIRY, 6)
        .await
        .unwrap();
    assert_eq!(
        h.period(prefunded, EXPIRY).await.status,
        PeriodStatus::Forming
    );
    let rent = h.ctx.banks_client.get_rent().await.unwrap();
    let key = find_period_address(&program(), &prefunded, EXPIRY).0;
    assert_eq!(
        h.account(key).await.unwrap().lamports,
        rent.minimum_balance(Period::LEN)
    );

    for squatted in [foreign, data_bearing] {
        assert_oracle_error(
            h.create_period(&authority, squatted, PLANNED, EXPIRY, 6)
                .await,
            OracleError::AddressOccupied,
        );
    }
}

#[tokio::test]
async fn unknown_tags_and_wrong_account_shapes_reject() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    let authority = h.oracle_authority.insecure_clone();

    for tag in [3u8, 30, 87, 205, 255] {
        let unknown = Instruction::new_with_bytes(program(), &[tag], vec![]);
        assert_eq!(
            custom(h.send(unknown, &[]).await),
            OracleError::InvalidInstructionData as u32,
            "tag {tag}"
        );
    }

    let ramx = product("RAMX");
    let canonical = instruction::create_period(
        &program(),
        &h.ctx.payer.pubkey(),
        &authority.pubkey(),
        ramx,
        PLANNED,
        EXPIRY,
        6,
    );
    let with = |metas: Vec<AccountMeta>| {
        let mut ix = canonical.clone();
        ix.accounts = metas;
        ix
    };

    let mut writable_config = canonical.accounts.clone();
    writable_config[1].is_writable = true;
    let mut short = canonical.accounts.clone();
    short.pop();
    let mut other_period = canonical.accounts.clone();
    other_period[3].pubkey = find_period_address(&program(), &ramx, APR_01_2027).0;

    assert_oracle_error(
        h.send(with(writable_config), &[&authority]).await,
        OracleError::InvalidAccountList,
    );
    assert_oracle_error(
        h.send(with(short), &[&authority]).await,
        OracleError::InvalidAccountList,
    );
    assert_eq!(
        instruction_error(h.send(with(other_period), &[&authority]).await),
        InstructionError::InvalidSeeds
    );
}
