//! Config governance lifecycle: bootstrap split (128), activation (129), pause (2) and rotation
//! (109).

mod common;

use ameba_oracle::{
    error::OracleError,
    instruction::{self, RotateVaultAuthoritiesV2Params},
    state::derive_settlement_signer_set_pda,
};
use common::*;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

fn five_members() -> Vec<Pubkey> {
    (0..5).map(|_| Pubkey::new_unique()).collect()
}

#[tokio::test]
async fn bootstrap_split_is_admin_only_once_and_before_the_signer_registry() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    let admin = h.admin.insecure_clone();
    let outsider = Keypair::new();
    let new_oracle = Keypair::new();

    let by_outsider = instruction::bootstrap_vault_governance_v2(
        &program(),
        &outsider.pubkey(),
        &new_oracle.pubkey(),
    );
    assert_oracle_error(
        h.send(by_outsider, &[&outsider, &new_oracle]).await,
        OracleError::SettlementSignerGovernanceRequired,
    );
    let to_self =
        instruction::bootstrap_vault_governance_v2(&program(), &admin.pubkey(), &admin.pubkey());
    assert_oracle_error(
        h.send(to_self, &[&admin]).await,
        OracleError::SettlementSignerGovernanceRequired,
    );

    let split = instruction::bootstrap_vault_governance_v2(
        &program(),
        &admin.pubkey(),
        &new_oracle.pubkey(),
    );
    h.send(split, &[&admin, &new_oracle]).await.unwrap();
    let config = h.config().await;
    assert_eq!(config.admin, admin.pubkey());
    assert_eq!(config.oracle_authority, new_oracle.pubkey());
    assert!(config.paused);

    let again = Keypair::new();
    let replay =
        instruction::bootstrap_vault_governance_v2(&program(), &admin.pubkey(), &again.pubkey());
    assert_oracle_error(
        h.send(replay, &[&admin, &again]).await,
        OracleError::SettlementSignerGovernanceRequired,
    );
    assert_eq!(h.config().await, config);
}

#[tokio::test]
async fn bootstrap_split_rejects_an_existing_signer_registry() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    install_signer_registry(&mut h, five_members()).await;
    let admin = h.admin.insecure_clone();
    let new_oracle = Keypair::new();
    let split = instruction::bootstrap_vault_governance_v2(
        &program(),
        &admin.pubkey(),
        &new_oracle.pubkey(),
    );
    assert_oracle_error(
        h.send(split, &[&admin, &new_oracle]).await,
        OracleError::AlreadyInitialized,
    );
}

#[tokio::test]
async fn activation_requires_split_roles_registry_and_exact_custody_and_is_the_only_unpause() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    let admin = h.admin.insecure_clone();
    let signer_set = derive_settlement_signer_set_pda(&program(), 1).0;
    let activate = |h: &Harness, freeze: Option<Pubkey>| {
        instruction::activate_vault_v2(
            &program(),
            &h.admin.pubkey(),
            &signer_set,
            &h.usdc_mint,
            freeze,
        )
    };

    // The fresh config still duplicates its authority, so it cannot activate even before the
    // signer registry is examined.
    assert_oracle_error(
        h.send(activate(&h, None), &[&admin]).await,
        OracleError::SettlementSignerGovernanceRequired,
    );
    h.split_oracle_authority().await;
    // Split roles alone are not enough: the governed signer registry must exist.
    assert_oracle_error(
        h.send(activate(&h, None), &[&admin]).await,
        OracleError::InvalidSettlementSignerRegistry,
    );
    let signers = install_signer_registry(&mut h, five_members()).await;
    assert_eq!(signers.set, signer_set);

    // Tag 2 can never unpause.
    let unpause = instruction::update_config(
        &program(),
        &admin.pubkey(),
        &h.usdc_mint,
        &h.bond_vault(),
        None,
        None,
        None,
        None,
        Some(false),
    );
    assert_oracle_error(
        h.send(unpause, &[&admin]).await,
        OracleError::SettlementSignerGovernanceRequired,
    );

    // The mint's freeze authority is bound exactly.
    assert_oracle_error(
        h.send(activate(&h, Some(Pubkey::new_unique())), &[&admin])
            .await,
        OracleError::InvalidMint,
    );
    let outsider = Keypair::new();
    let by_outsider = instruction::activate_vault_v2(
        &program(),
        &outsider.pubkey(),
        &signers.set,
        &h.usdc_mint,
        None,
    );
    assert_oracle_error(
        h.send(by_outsider, &[&outsider]).await,
        OracleError::SettlementSignerGovernanceRequired,
    );

    h.send(activate(&h, None), &[&admin]).await.unwrap();
    assert!(!h.config().await.paused);
    h.advance_slot().await;
    assert_oracle_error(
        h.send(activate(&h, None), &[&admin]).await,
        OracleError::SettlementSignerGovernanceRequired,
    );

    // The admin may always pause again.
    let pause = instruction::update_config(
        &program(),
        &admin.pubkey(),
        &h.usdc_mint,
        &h.bond_vault(),
        None,
        None,
        None,
        None,
        Some(true),
    );
    h.send(pause, &[&admin]).await.unwrap();
    assert!(h.config().await.paused);
    let _ = signers.recovery;
}

#[tokio::test]
async fn activation_rejects_a_signer_set_that_includes_governance() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    h.split_oracle_authority().await;
    let mut members = five_members();
    members[2] = h.oracle_authority.pubkey();
    let signers = install_signer_registry(&mut h, members).await;
    let admin = h.admin.insecure_clone();
    let ix = instruction::activate_vault_v2(
        &program(),
        &admin.pubkey(),
        &signers.set,
        &h.usdc_mint,
        None,
    );
    assert_oracle_error(
        h.send(ix, &[&admin]).await,
        OracleError::InvalidSettlementSignerConfiguration,
    );
    assert!(h.config().await.paused);
}

#[tokio::test]
async fn rotation_requires_pause_and_all_four_signatures() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    h.split_oracle_authority().await;
    let signers = install_signer_registry(&mut h, five_members()).await;
    let admin = h.admin.insecure_clone();
    let oracle = h.oracle_authority.insecure_clone();
    let new_admin = Keypair::new();
    let new_oracle = Keypair::new();
    let params = RotateVaultAuthoritiesV2Params {
        new_admin: new_admin.pubkey(),
        new_oracle_authority: new_oracle.pubkey(),
    };
    let rotate = |params: RotateVaultAuthoritiesV2Params| {
        instruction::rotate_vault_authorities_v2(
            &program(),
            &admin.pubkey(),
            &oracle.pubkey(),
            params,
            &signers.set,
        )
    };

    // The incoming pair must be distinct and must not include a signer-set member.
    let same = RotateVaultAuthoritiesV2Params {
        new_admin: new_admin.pubkey(),
        new_oracle_authority: new_admin.pubkey(),
    };
    assert_oracle_error(
        h.send(rotate(same), &[&admin, &oracle, &new_admin]).await,
        OracleError::SettlementSignerGovernanceRequired,
    );

    h.send(
        rotate(params.clone()),
        &[&admin, &oracle, &new_admin, &new_oracle],
    )
    .await
    .unwrap();
    let config = h.config().await;
    assert_eq!(config.admin, new_admin.pubkey());
    assert_eq!(config.oracle_authority, new_oracle.pubkey());
    assert!(config.paused);

    // The rotated-out pair can no longer act.
    h.advance_slot().await;
    assert_oracle_error(
        h.send(rotate(params), &[&admin, &oracle, &new_admin, &new_oracle])
            .await,
        OracleError::SettlementSignerGovernanceRequired,
    );
    let _ = signers.members;
}

#[tokio::test]
#[cfg(not(feature = "localnet-dev-authority"))]
async fn controller_handoff_initialization_requires_three_current_council_signers() {
    use borsh::BorshSerialize;
    use governance_controller_v3::state::{
        authority_pda, config_pda, target_authority_pda, Config, Timing,
    };
    use solana_sdk::account::AccountSharedData;
    let mut h = start().await;
    let controller = ameba_oracle::governance_gate::PINNED_CONTROLLER_PROGRAM_ID;
    let authority = authority_pda(&controller).0;
    let keys: [Keypair; 5] = core::array::from_fn(|_| Keypair::new());
    let seats = keys.each_ref().map(Signer::pubkey);
    let (config_key, bump) = config_pda(&controller);
    let controller_config = Config {
        discriminator: *b"AG3CFG01",
        version: 1,
        bump,
        initialized: true,
        controller,
        programdata: Pubkey::find_program_address(
            &[controller.as_ref()],
            &solana_sdk_ids::bpf_loader_upgradeable::ID,
        )
        .0,
        authority,
        treasury: Pubkey::new_unique(),
        seats,
        council_epoch: 1,
        timing_version: 1,
        timing: Timing::default(),
        next_id: 1,
        reserved: [0; 37],
    };
    controller_config
        .validate(&controller, &config_key)
        .unwrap();
    h.ctx.set_account(
        &config_key,
        &AccountSharedData::from(solana_sdk::account::Account {
            lamports: 10_000_000,
            data: controller_config.try_to_vec().unwrap(),
            owner: controller,
            executable: false,
            rent_epoch: 0,
        }),
    );
    let program_data_key = Pubkey::find_program_address(
        &[program().as_ref()],
        &solana_sdk_ids::bpf_loader_upgradeable::ID,
    )
    .0;
    h.ctx.set_account(
        &program_data_key,
        // Registration moves the loader authority to the per-target PDA, not the
        // controller's own authority.
        &AccountSharedData::from(program_data(Some(
            target_authority_pda(&controller, &program()).0,
        ))),
    );
    for key in &keys {
        h.fund(&key.pubkey(), 10_000_000).await;
    }
    let payer = h.payer();
    let admin = h.admin.pubkey();
    let mint = h.usdc_mint;
    let make = |approvals| {
        instruction::initialize_config_with_council(
            &program(),
            &payer.pubkey(),
            &controller,
            &approvals,
            admin,
            &mint,
        )
    };
    let mut missing = make([seats[0], seats[1], seats[2]]);
    missing.accounts[12].is_signer = false;
    assert_oracle_error(
        h.send(missing, &[&keys[0], &keys[1]]).await,
        OracleError::InvalidUpgradeAuthority,
    );
    let duplicate = make([seats[0], seats[0], seats[2]]);
    assert_oracle_error(
        h.send(duplicate, &[&keys[0], &keys[2]]).await,
        OracleError::InvalidUpgradeAuthority,
    );
    let good = make([seats[0], seats[1], seats[2]]);
    h.send(good.clone(), &[&keys[0], &keys[1], &keys[2]])
        .await
        .unwrap();
    let config: ameba_oracle::state::OracleConfig = h
        .read_state(ameba_oracle::state::find_config_address(&program()).0)
        .await;
    assert_eq!(config.admin, h.admin.pubkey());
    assert!(config.paused);
    h.advance_slot().await;
    assert_oracle_error(
        h.send(good, &[&keys[0], &keys[1], &keys[2]]).await,
        OracleError::AlreadyInitialized,
    );
}
