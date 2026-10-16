//! The settlement signer registry (tags 90-94): creation, an attested 3-of-5 rotation, its
//! cancellation and replay protection, timelocked activation, and emergency recovery that only
//! activates while the config is paused.
mod common;

use ameba_oracle::{
    constants::{
        EMERGENCY_SETTLEMENT_SIGNER_DELAY_MULTIPLIER, MAX_SETTLEMENT_SIGNER_COUNT,
        MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS,
    },
    error::OracleError,
    instruction::{self, ProposeSettlementSignerRotationParams},
    state::{
        derive_settlement_signer_registry_pda, derive_settlement_signer_set_pda,
        SettlementSignerRegistry, SettlementSignerSet,
    },
};
use common::*;
use solana_program::hash::hashv;
use solana_sdk::{
    clock::Clock,
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

fn sorted_signers() -> Vec<Keypair> {
    let mut signers: Vec<Keypair> = (0..5).map(|_| Keypair::new()).collect();
    signers.sort_by_key(Signer::pubkey);
    signers
}

fn signer_array(signers: &[Keypair]) -> [Pubkey; MAX_SETTLEMENT_SIGNER_COUNT] {
    let mut result = [Pubkey::default(); MAX_SETTLEMENT_SIGNER_COUNT];
    for (destination, signer) in result.iter_mut().zip(signers) {
        *destination = signer.pubkey();
    }
    result
}

/// The pending set's hashed identity as the rotation digest commits to it.
fn pending_set(
    version: u64,
    signers: [Pubkey; MAX_SETTLEMENT_SIGNER_COUNT],
    activate_after_slot: u64,
    emergency: bool,
) -> SettlementSignerSet {
    let mut set = SettlementSignerSet {
        registry: derive_settlement_signer_registry_pda(&program()).0,
        version,
        threshold: 3,
        signer_count: 5,
        signers,
        rotation_delay_slots: MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS,
        activate_after_slot,
        emergency,
        ..SettlementSignerSet::default()
    };
    set.set_hash = set.compute_set_hash();
    set
}

fn rotation_digest(
    current: &SettlementSignerSet,
    nonce: u64,
    pending: &SettlementSignerSet,
) -> Vec<u8> {
    hashv(&[
        b"ameba_settlement_signer_rotation_v1",
        program().as_ref(),
        derive_settlement_signer_registry_pda(&program()).0.as_ref(),
        derive_settlement_signer_set_pda(&program(), current.version)
            .0
            .as_ref(),
        &current.version.to_le_bytes(),
        &current.set_hash,
        &nonce.to_le_bytes(),
        derive_settlement_signer_set_pda(&program(), pending.version)
            .0
            .as_ref(),
        &pending.version.to_le_bytes(),
        &pending.set_hash,
        &pending.activate_after_slot.to_le_bytes(),
        &pending.rotation_delay_slots.to_le_bytes(),
        &[u8::from(pending.emergency)],
    ])
    .to_bytes()
    .to_vec()
}

fn rotation_params(set: &SettlementSignerSet) -> ProposeSettlementSignerRotationParams {
    ProposeSettlementSignerRotationParams {
        target_version: set.version,
        threshold: set.threshold,
        signer_count: set.signer_count,
        signers: set.signers,
        rotation_delay_slots: set.rotation_delay_slots,
        activate_after_slot: set.activate_after_slot,
    }
}

impl Harness {
    async fn slot(&mut self) -> u64 {
        let clock: Clock = self.ctx.banks_client.get_sysvar().await.unwrap();
        clock.slot
    }

    fn propose(
        &self,
        attesters: &[&Keypair],
        digest: &[u8],
        current_version: u64,
        pending: &SettlementSignerSet,
    ) -> Vec<Instruction> {
        let mut ixs: Vec<Instruction> = attesters
            .iter()
            .map(|signer| ed25519_verify_instruction(signer, digest))
            .collect();
        ixs.push(instruction::propose_settlement_signer_rotation(
            &program(),
            &self.admin.pubkey(),
            &self.oracle_authority.pubkey(),
            None,
            current_version,
            rotation_params(pending),
        ));
        ixs
    }
}

#[tokio::test]
async fn attested_rotation_cancellation_activation_and_emergency_recovery_are_enforced() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    h.split_oracle_authority().await;
    let admin = h.admin.insecure_clone();
    let oracle = h.oracle_authority.insecure_clone();
    let recovery = h.recovery_authority.insecure_clone();
    let registry_key = derive_settlement_signer_registry_pda(&program()).0;
    let set_v1_key = derive_settlement_signer_set_pda(&program(), 1).0;
    let set_v2_key = derive_settlement_signer_set_pda(&program(), 2).0;
    let set_v3_key = derive_settlement_signer_set_pda(&program(), 3).0;

    h.initialize_signer_registry().await;
    let registry: SettlementSignerRegistry = h.read_state(registry_key).await;
    let set_v1: SettlementSignerSet = h.read_state(set_v1_key).await;
    assert_eq!(registry.current_set, set_v1_key);
    assert_eq!(registry.current_version, 1);
    assert_eq!(registry.pending_set, Pubkey::default());
    assert_eq!(registry.proposal_nonce, 0);
    assert_eq!(registry.recovery_authority, recovery.pubkey());
    assert_eq!((set_v1.threshold, set_v1.signer_count), (3, 5));
    assert_eq!(set_v1.set_hash, set_v1.compute_set_hash());
    let current: Vec<Keypair> = h
        .settlement_signers
        .iter()
        .map(|key| key.insecure_clone())
        .collect();
    let attesters: Vec<&Keypair> = current.iter().take(3).collect();
    let mint = h.usdc_mint;
    let vault = h.bond_vault();

    // The registry is created once.
    h.advance_slot().await;
    let again = instruction::initialize_settlement_signer_registry(
        &program(),
        &admin.pubkey(),
        &oracle.pubkey(),
        &recovery.pubkey(),
        ameba_oracle::instruction::InitializeSettlementSignerRegistryParams {
            signer_set_version: 1,
            threshold: 3,
            signer_count: 5,
            signers: signer_array(&sorted_signers()),
            rotation_delay_slots: MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS,
            recovery_authority: recovery.pubkey(),
        },
    );
    assert!(h.send(again, &[&admin, &oracle, &recovery]).await.is_err());

    // An ordinary rotation needs the threshold of current signers over its exact digest.
    let slot = h.slot().await;
    let replacement_a = signer_array(&sorted_signers());
    let pending_a = pending_set(
        2,
        replacement_a,
        slot + MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS + 10,
        false,
    );
    let digest_a = rotation_digest(&set_v1, 1, &pending_a);
    assert_oracle_error(
        h.send_many(
            h.propose(&attesters[..2], &digest_a, 1, &pending_a),
            &[&admin, &oracle],
        )
        .await,
        OracleError::MissingSettlementOracleSignatures,
    );
    h.send_many(
        h.propose(&attesters, &digest_a, 1, &pending_a),
        &[&admin, &oracle],
    )
    .await
    .unwrap();
    let proposed: SettlementSignerRegistry = h.read_state(registry_key).await;
    assert_eq!(proposed.pending_set, set_v2_key);
    assert_eq!(proposed.pending_version, 2);
    assert_eq!(proposed.proposal_nonce, 1);
    // One pending rotation at a time, and none activates before its timelock.
    h.advance_slot().await;
    assert_oracle_error(
        h.send_many(
            h.propose(&attesters, &digest_a, 1, &pending_a),
            &[&admin, &oracle],
        )
        .await,
        OracleError::SettlementSignerRotationPending,
    );
    assert_oracle_error(
        h.send(
            instruction::activate_settlement_signer_rotation(&program(), 2),
            &[],
        )
        .await,
        OracleError::SettlementSignerRotationNotReady,
    );

    // Cancellation clears the pointer; the cancelled signatures cannot be replayed.
    h.send(
        instruction::cancel_settlement_signer_rotation(
            &program(),
            &admin.pubkey(),
            &oracle.pubkey(),
        ),
        &[&admin, &oracle],
    )
    .await
    .unwrap();
    let cancelled: SettlementSignerRegistry = h.read_state(registry_key).await;
    assert_eq!(cancelled.current_set, set_v1_key);
    assert_eq!(cancelled.pending_set, Pubkey::default());
    assert_eq!(cancelled.pending_version, 0);
    assert_eq!(cancelled.proposal_nonce, 1);
    h.advance_slot().await;
    assert_oracle_error(
        h.send_many(
            h.propose(&attesters, &digest_a, 1, &pending_a),
            &[&admin, &oracle],
        )
        .await,
        OracleError::SettlementOraclePayloadMismatch,
    );

    // A fresh proposal may overwrite only the never-active cancelled set of the same version.
    let slot = h.slot().await;
    let replacement_b = signer_array(&sorted_signers());
    let pending_b = pending_set(
        2,
        replacement_b,
        slot + MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS + 10,
        false,
    );
    let digest_b = rotation_digest(&set_v1, 2, &pending_b);
    h.send_many(
        h.propose(&attesters, &digest_b, 1, &pending_b),
        &[&admin, &oracle],
    )
    .await
    .unwrap();
    let rewritten: SettlementSignerSet = h.read_state(set_v2_key).await;
    assert_eq!(rewritten.signers, replacement_b);
    assert_ne!(rewritten.signers, replacement_a);
    let reproposed: SettlementSignerRegistry = h.read_state(registry_key).await;
    assert_eq!(reproposed.proposal_nonce, 2);

    // After the timelock anyone activates it.
    h.advance_slots(MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS + 11)
        .await;
    h.send(
        instruction::activate_settlement_signer_rotation(&program(), 2),
        &[],
    )
    .await
    .unwrap();
    let active_v2: SettlementSignerRegistry = h.read_state(registry_key).await;
    assert_eq!(active_v2.current_set, set_v2_key);
    assert_eq!(active_v2.current_version, 2);
    assert_eq!(active_v2.pending_set, Pubkey::default());

    // Emergency recovery: no attestations, the recovery authority, a doubled timelock.
    let slot = h.slot().await;
    let emergency_after = slot
        + MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS * EMERGENCY_SETTLEMENT_SIGNER_DELAY_MULTIPLIER
        + 10;
    let pending_v3 = pending_set(3, signer_array(&sorted_signers()), emergency_after, true);
    let stranger = Keypair::new();
    assert_oracle_error(
        h.send(
            instruction::propose_settlement_signer_rotation(
                &program(),
                &admin.pubkey(),
                &oracle.pubkey(),
                Some(&stranger.pubkey()),
                2,
                rotation_params(&pending_v3),
            ),
            &[&admin, &oracle, &stranger],
        )
        .await,
        OracleError::SettlementSignerGovernanceRequired,
    );
    let too_soon = pending_set(
        3,
        pending_v3.signers,
        slot + MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS,
        true,
    );
    assert_oracle_error(
        h.send(
            instruction::propose_settlement_signer_rotation(
                &program(),
                &admin.pubkey(),
                &oracle.pubkey(),
                Some(&recovery.pubkey()),
                2,
                rotation_params(&too_soon),
            ),
            &[&admin, &oracle, &recovery],
        )
        .await,
        OracleError::SettlementSignerTimelockTooShort,
    );
    h.send(
        instruction::propose_settlement_signer_rotation(
            &program(),
            &admin.pubkey(),
            &oracle.pubkey(),
            Some(&recovery.pubkey()),
            2,
            rotation_params(&pending_v3),
        ),
        &[&admin, &oracle, &recovery],
    )
    .await
    .unwrap();
    let emergency: SettlementSignerSet = h.read_state(set_v3_key).await;
    assert!(emergency.emergency);
    assert_eq!(emergency.activate_after_slot, emergency_after);
    let registry: SettlementSignerRegistry = h.read_state(registry_key).await;
    assert_eq!(registry.proposal_nonce, 3);

    // Unpausing still needs the governed activation; an emergency set then waits for a pause.
    let unpause = instruction::update_config(
        &program(),
        &admin.pubkey(),
        &mint,
        &vault,
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
    h.send(
        instruction::activate_vault_v2(&program(), &admin.pubkey(), &set_v2_key, &mint, None),
        &[&admin],
    )
    .await
    .unwrap();
    assert!(!h.config().await.paused);
    h.advance_slots(
        MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS * EMERGENCY_SETTLEMENT_SIGNER_DELAY_MULTIPLIER
            + 11,
    )
    .await;
    assert_oracle_error(
        h.send(
            instruction::activate_settlement_signer_rotation(&program(), 3),
            &[],
        )
        .await,
        OracleError::SettlementSignerRecoveryRequiresPause,
    );
    h.send(
        instruction::update_config(
            &program(),
            &admin.pubkey(),
            &mint,
            &vault,
            None,
            None,
            None,
            None,
            Some(true),
        ),
        &[&admin],
    )
    .await
    .unwrap();
    h.advance_slot().await;
    h.send(
        instruction::activate_settlement_signer_rotation(&program(), 3),
        &[],
    )
    .await
    .unwrap();
    let active_v3: SettlementSignerRegistry = h.read_state(registry_key).await;
    assert_eq!(active_v3.current_set, set_v3_key);
    assert_eq!(active_v3.current_version, 3);
    assert_eq!(active_v3.pending_set, Pubkey::default());
    assert_eq!(active_v3.pending_version, 0);
}
