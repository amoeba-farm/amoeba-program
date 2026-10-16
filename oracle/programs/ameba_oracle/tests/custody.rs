//! Bond vault custody and `UserCollateral` ledgers (tags 9, 10, 11 and 140).

mod common;

use ameba_oracle::{
    constants::{
        CURRENT_STATE_NAMESPACE_SEED, MAX_SETTLEMENT_SIGNER_COUNT,
        PRODUCTION_MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS, USER_COLLATERAL_PDA_SEED,
    },
    error::OracleError,
    instruction,
    state::{
        derive_settlement_signer_registry_pda, derive_settlement_signer_set_pda,
        derive_user_collateral_pda, SettlementSignerRegistry, SettlementSignerSet, UserCollateral,
    },
};
use common::*;
use solana_sdk::{
    instruction::{Instruction, InstructionError},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

/// Install a governed signer registry and its current 3-of-5 set. Their creating instruction
/// belongs to the settlement signer family; activation only reads them.
async fn install_signer_registry(h: &mut Harness) -> (Pubkey, Pubkey) {
    let (registry_key, registry_bump) = derive_settlement_signer_registry_pda(&program());
    let (set_key, set_bump) = derive_settlement_signer_set_pda(&program(), 1);
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
        rotation_delay_slots: PRODUCTION_MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS,
        proposed_slot: 0,
        activate_after_slot: 0,
        emergency: false,
        proposer: h.admin.pubkey(),
    };
    set.set_hash = set.compute_set_hash();
    let registry = SettlementSignerRegistry {
        is_initialized: true,
        bump: registry_bump,
        account_discriminator: SettlementSignerRegistry::ACCOUNT_DISCRIMINATOR,
        account_version: SettlementSignerRegistry::ACCOUNT_VERSION,
        current_set: set_key,
        current_version: 1,
        pending_set: Pubkey::default(),
        pending_version: 0,
        recovery_authority: Pubkey::new_unique(),
        proposal_nonce: 0,
    };
    h.set_program_state(registry_key, &registry, SettlementSignerRegistry::LEN)
        .await;
    h.set_program_state(set_key, &set, SettlementSignerSet::LEN)
        .await;
    (registry_key, set_key)
}

/// Initialize, split the oracle authority, install signers and activate (0 -> 128 -> 129).
async fn active_harness() -> Harness {
    let mut h = start().await;
    h.initialize().await.unwrap();
    h.split_oracle_authority().await;
    let (_, set_key) = install_signer_registry(&mut h).await;
    let ix =
        instruction::activate_vault_v2(&program(), &h.admin.pubkey(), &set_key, &h.usdc_mint, None);
    let admin = h.admin.insecure_clone();
    h.send(ix, &[&admin]).await.unwrap();
    assert!(!h.config().await.paused);
    h
}

struct Owner {
    key: Keypair,
    token: Pubkey,
}

async fn funded_owner(h: &mut Harness, usdc: u64) -> Owner {
    let key = Keypair::new();
    h.fund(&key.pubkey(), 1_000_000_000).await;
    let token = h.create_ata(&key.pubkey()).await;
    if usdc > 0 {
        h.mint_usdc(&token, usdc).await;
    }
    let ix = instruction::init_user_collateral(&program(), &key.pubkey());
    let signer = key.insecure_clone();
    h.send(ix, &[&signer]).await.unwrap();
    Owner { key, token }
}

async fn deposit(
    h: &mut Harness,
    owner: &Owner,
    amount: u64,
) -> Result<(), solana_program_test::BanksClientError> {
    let ix = instruction::deposit_collateral(
        &program(),
        &owner.key.pubkey(),
        &owner.token,
        &h.usdc_mint,
        amount,
    );
    let signer = owner.key.insecure_clone();
    h.send(ix, &[&signer]).await
}

async fn withdraw(
    h: &mut Harness,
    owner: &Owner,
    amount: u64,
) -> Result<(), solana_program_test::BanksClientError> {
    let ix = instruction::withdraw_collateral(
        &program(),
        &owner.key.pubkey(),
        &owner.token,
        &h.usdc_mint,
        amount,
    );
    let signer = owner.key.insecure_clone();
    h.send(ix, &[&signer]).await
}

/// The bond vault always holds at least every ledger's balance.
async fn assert_conserved(h: &mut Harness, owners: &[&Owner]) -> (u64, u64) {
    let vault = h.token_balance(h.bond_vault()).await;
    let mut ledgers = 0u64;
    for owner in owners {
        let collateral = h.collateral(&owner.key.pubkey()).await;
        ledgers += collateral.available_balance + collateral.position_locked_balance;
    }
    assert!(
        vault >= ledgers,
        "bond vault {vault} holds less than its ledgers {ledgers}"
    );
    (vault, ledgers)
}

#[tokio::test]
async fn user_collateral_init_is_create_only_and_preserves_prefunded_pdas() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    let owner = funded_owner(&mut h, 2_000_000).await;
    let collateral = derive_user_collateral_pda(&program(), &owner.key.pubkey()).0;
    deposit(&mut h, &owner, 444_000).await.unwrap();
    let before = h.account(collateral).await.unwrap();
    assert_eq!(
        h.collateral(&owner.key.pubkey()).await.available_balance,
        444_000
    );

    // A retry with a fresh transaction signature reaches the create-only guard.
    h.advance_slot().await;
    let signer = owner.key.insecure_clone();
    assert_oracle_error(
        h.send(
            instruction::init_user_collateral(&program(), &owner.key.pubkey()),
            &[&signer],
        )
        .await,
        OracleError::AlreadyInitialized,
    );
    assert_eq!(h.account(collateral).await.unwrap(), before);

    let prefunded_owner = Keypair::new();
    h.fund(&prefunded_owner.pubkey(), 1_000_000_000).await;
    let (prefunded, _) = Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            USER_COLLATERAL_PDA_SEED,
            prefunded_owner.pubkey().as_ref(),
        ],
        &program(),
    );
    h.set_system_account_with_lamports(prefunded, 1).await;
    h.send(
        instruction::init_user_collateral(&program(), &prefunded_owner.pubkey()),
        &[&prefunded_owner],
    )
    .await
    .unwrap();
    let state: UserCollateral = h.read_state(prefunded).await;
    assert!(state.is_initialized);
    assert_eq!(state.owner, prefunded_owner.pubkey());
    assert_eq!(state.available_balance, 0);
}

#[tokio::test]
async fn deposits_reject_zero_amounts_wrong_mints_and_foreign_ledgers() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    let owner = funded_owner(&mut h, 1_000_000).await;
    assert_oracle_error(
        deposit(&mut h, &owner, 0).await,
        OracleError::AmountMustBePositive,
    );

    let other_authority = Keypair::new();
    let other_mint = h.create_mint(&other_authority.pubkey(), 6, None).await;
    let other_token = h
        .create_token_account(&other_mint, &owner.key.pubkey())
        .await;
    let mut wrong_mint = instruction::deposit_collateral(
        &program(),
        &owner.key.pubkey(),
        &other_token,
        &h.usdc_mint,
        1,
    );
    wrong_mint.accounts[5].pubkey = other_mint;
    let signer = owner.key.insecure_clone();
    assert_oracle_error(
        h.send(wrong_mint, &[&signer]).await,
        OracleError::InvalidMint,
    );

    // Another owner's ledger never takes this owner's deposit.
    let other = funded_owner(&mut h, 0).await;
    let mut foreign = instruction::deposit_collateral(
        &program(),
        &owner.key.pubkey(),
        &owner.token,
        &h.usdc_mint,
        1,
    );
    foreign.accounts[4].pubkey = derive_user_collateral_pda(&program(), &other.key.pubkey()).0;
    assert_oracle_error(
        h.send(foreign, &[&signer]).await,
        OracleError::InvalidUserCollateralAccount,
    );
    assert_eq!(h.token_balance(h.bond_vault()).await, 0);
}

#[tokio::test]
async fn owner_movement_ignores_pause_but_assisted_withdrawal_requires_activation() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    assert!(h.config().await.paused);
    let owner = funded_owner(&mut h, 1_000_000).await;
    deposit(&mut h, &owner, 700_000).await.unwrap();
    withdraw(&mut h, &owner, 200_000).await.unwrap();
    assert_eq!(
        h.collateral(&owner.key.pubkey()).await.available_balance,
        500_000
    );

    let ix = instruction::admin_assisted_withdraw_collateral(
        &program(),
        &h.admin.pubkey(),
        &owner.key.pubkey(),
        &owner.token,
        &h.usdc_mint,
        1,
    );
    let admin = h.admin.insecure_clone();
    let signer = owner.key.insecure_clone();
    assert_oracle_error(
        h.send(ix, &[&admin, &signer]).await,
        OracleError::ContractPaused,
    );
    assert_conserved(&mut h, &[&owner]).await;
}

#[tokio::test]
async fn admin_assisted_withdraw_requires_exact_owner_ledger_and_ata() {
    let mut h = active_harness().await;
    let admin = h.admin.pubkey();
    let owner = funded_owner(&mut h, 1_700).await;
    deposit(&mut h, &owner, 1_700).await.unwrap();
    let attacker = funded_owner(&mut h, 0).await;
    let owner_key = owner.key.pubkey();

    let mut missing_owner_signature = instruction::admin_assisted_withdraw_collateral(
        &program(),
        &admin,
        &owner_key,
        &owner.token,
        &h.usdc_mint,
        1,
    );
    missing_owner_signature.accounts[1].is_signer = false;
    let admin_signer = h.admin.insecure_clone();
    assert_eq!(
        instruction_error(h.send(missing_owner_signature, &[&admin_signer]).await),
        InstructionError::MissingRequiredSignature
    );

    let owner_signer = owner.key.insecure_clone();
    let attacker_signer = attacker.key.insecure_clone();
    let wrong_admin = instruction::admin_assisted_withdraw_collateral(
        &program(),
        &attacker.key.pubkey(),
        &owner_key,
        &owner.token,
        &h.usdc_mint,
        1,
    );
    assert_oracle_error(
        h.send(wrong_admin, &[&attacker_signer, &owner_signer])
            .await,
        OracleError::Unauthorized,
    );

    let mut generic_user = instruction::admin_assisted_withdraw_collateral(
        &program(),
        &admin,
        &attacker.key.pubkey(),
        &attacker.token,
        &h.usdc_mint,
        1,
    );
    generic_user.accounts[5].pubkey = derive_user_collateral_pda(&program(), &owner_key).0;
    assert_oracle_error(
        h.send(generic_user, &[&admin_signer, &attacker_signer])
            .await,
        OracleError::InvalidUserCollateralAccount,
    );

    let mint = h.usdc_mint;
    let noncanonical = h.create_token_account(&mint, &owner_key).await;
    let noncanonical_destination = instruction::admin_assisted_withdraw_collateral(
        &program(),
        &admin,
        &owner_key,
        &noncanonical,
        &h.usdc_mint,
        1,
    );
    assert_oracle_error(
        h.send(noncanonical_destination, &[&admin_signer, &owner_signer])
            .await,
        OracleError::InvalidTokenAccount,
    );

    let encumbered = instruction::admin_assisted_withdraw_collateral(
        &program(),
        &admin,
        &owner_key,
        &owner.token,
        &h.usdc_mint,
        1_701,
    );
    assert_oracle_error(
        h.send(encumbered, &[&admin_signer, &owner_signer]).await,
        OracleError::InsufficientAvailableCollateral,
    );

    let assisted = instruction::admin_assisted_withdraw_collateral(
        &program(),
        &admin,
        &owner_key,
        &owner.token,
        &h.usdc_mint,
        600,
    );
    h.send(assisted, &[&admin_signer, &owner_signer])
        .await
        .unwrap();
    assert_eq!(h.collateral(&owner_key).await.available_balance, 1_100);
    assert_eq!(h.token_balance(owner.token).await, 600);
    assert_eq!(h.token_balance(h.bond_vault()).await, 1_100);
}

#[tokio::test]
async fn bond_vault_balance_always_covers_every_ledger() {
    let mut h = active_harness().await;
    let alice = funded_owner(&mut h, 5_000_000).await;
    let bob = funded_owner(&mut h, 3_000_000).await;
    let carol = funded_owner(&mut h, 0).await;
    let owners = [&alice, &bob, &carol];
    assert_eq!(assert_conserved(&mut h, &owners).await, (0, 0));

    deposit(&mut h, &alice, 4_000_000).await.unwrap();
    deposit(&mut h, &bob, 3_000_000).await.unwrap();
    assert_eq!(
        assert_conserved(&mut h, &owners).await,
        (7_000_000, 7_000_000)
    );

    withdraw(&mut h, &alice, 1_500_000).await.unwrap();
    assert_eq!(
        assert_conserved(&mut h, &owners).await,
        (5_500_000, 5_500_000)
    );

    // Over-withdrawal and cross-ledger withdrawal fail without moving tokens.
    assert_oracle_error(
        withdraw(&mut h, &bob, 3_000_001).await,
        OracleError::InsufficientAvailableCollateral,
    );
    assert_oracle_error(
        withdraw(&mut h, &carol, 1).await,
        OracleError::InsufficientAvailableCollateral,
    );
    let mut cross = instruction::withdraw_collateral(
        &program(),
        &carol.key.pubkey(),
        &carol.token,
        &h.usdc_mint,
        1,
    );
    cross.accounts[4].pubkey = derive_user_collateral_pda(&program(), &bob.key.pubkey()).0;
    let carol_signer = carol.key.insecure_clone();
    assert_oracle_error(
        h.send(cross, &[&carol_signer]).await,
        OracleError::InvalidUserCollateralAccount,
    );
    // A destination owned by someone else is rejected.
    let mut redirected = instruction::withdraw_collateral(
        &program(),
        &bob.key.pubkey(),
        &carol.token,
        &h.usdc_mint,
        1,
    );
    redirected.accounts[2].pubkey = carol.token;
    let bob_signer = bob.key.insecure_clone();
    assert_oracle_error(
        h.send(redirected, &[&bob_signer]).await,
        OracleError::InvalidTokenAccount,
    );
    assert_eq!(
        assert_conserved(&mut h, &owners).await,
        (5_500_000, 5_500_000)
    );

    let assisted = instruction::admin_assisted_withdraw_collateral(
        &program(),
        &h.admin.pubkey(),
        &bob.key.pubkey(),
        &bob.token,
        &h.usdc_mint,
        1_000_000,
    );
    let admin = h.admin.insecure_clone();
    h.send(assisted, &[&admin, &bob_signer]).await.unwrap();
    assert_eq!(
        assert_conserved(&mut h, &owners).await,
        (4_500_000, 4_500_000)
    );

    // Tokens sent to the vault outside the ledgers only widen the margin.
    let vault = h.bond_vault();
    h.mint_usdc(&vault, 123).await;
    assert_eq!(
        assert_conserved(&mut h, &owners).await,
        (4_500_123, 4_500_000)
    );

    withdraw(&mut h, &alice, 2_500_000).await.unwrap();
    withdraw(&mut h, &bob, 2_000_000).await.unwrap();
    assert_eq!(assert_conserved(&mut h, &owners).await, (123, 0));
    assert_eq!(h.token_balance(alice.token).await, 5_000_000);
    assert_eq!(h.token_balance(bob.token).await, 3_000_000);
}

#[tokio::test]
async fn malformed_and_amplified_instruction_data_rejects_before_account_validation() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    for data in [vec![255u8, 0, 1], vec![3, 0, 0, 0, 0, 0, 0, 0, 1]] {
        let malformed = Instruction {
            program_id: program(),
            accounts: vec![],
            data,
        };
        assert_oracle_error(
            h.send(malformed, &[]).await,
            OracleError::InvalidInstructionData,
        );
    }

    // A product-SKU chunk whose length prefix exceeds the chunk bound fails as instruction data.
    let mut fixed_prefix = vec![190u8];
    fixed_prefix.extend_from_slice(&[7; 32]);
    fixed_prefix.extend_from_slice(&11u64.to_le_bytes());
    fixed_prefix.extend_from_slice(&0u16.to_le_bytes());
    fixed_prefix.extend_from_slice(&52u16.to_le_bytes());
    fixed_prefix.extend_from_slice(&[3; 32]);
    for malformed_length in [
        u32::MAX.to_le_bytes().to_vec(),
        17u32.to_le_bytes().to_vec(),
        vec![1, 0, 0],
    ] {
        let mut data = fixed_prefix.clone();
        data.extend_from_slice(&malformed_length);
        let ix = Instruction {
            program_id: program(),
            accounts: vec![],
            data,
        };
        assert_oracle_error(h.send(ix, &[]).await, OracleError::InvalidInstructionData);
    }
}
