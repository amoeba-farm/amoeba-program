use super::*;
use crate::{
    artifact_merkle::ARTIFACT_MERKLE_SCHEME_ID,
    release1_ceremony_state::{
        CEREMONY_ACCOUNT_VERSION_V1, CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR,
        CURRENT_DEPLOYMENT_STATE_V1_RESERVED_LEN,
    },
};

fn key(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}

fn unfreeze_guard() -> UnfreezeGuardV2 {
    UnfreezeGuardV2 {
        expected_proposal_digest: [1; 32],
        expected_checkpoint_digest: [2; 32],
        expected_checkpoint_generation: 1,
        expected_verification_digest: [3; 32],
        expected_verification_generation: 1,
        expected_original_council_version: 1,
        expected_original_council_hash: [4; 32],
        expected_current_council_version: 1,
        expected_current_council_hash: [5; 32],
        expected_gate_epoch: 1,
        expected_target_nonce: 1,
        expected_current_deployment_digest: [6; 32],
        expected_current_deployment_generation: 1,
        expected_artifact_sha256: [7; 32],
        expected_artifact_merkle_root: [8; 32],
        expected_actual_capacity: 1,
        expected_approval_bitset: 0,
        expected_approval_count: 0,
    }
}

fn deployment_fixture(generation: u64) -> CurrentDeploymentStateV1 {
    let mut deployment = CurrentDeploymentStateV1 {
        discriminator: CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: 1,
        initialized: true,
        controller_program: key(1),
        controller_config: key(2),
        capacity_policy: key(3),
        capacity_policy_digest: [4; 32],
        target_program: key(5),
        target_programdata: key(6),
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        controller_authority: key(7),
        artifact_length: 16_384,
        artifact_sha256: [8; 32],
        artifact_merkle_root: [9; 32],
        artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
        actual_programdata_capacity: 32_768,
        programdata_observation: key(10),
        observation_generation: 1,
        observation_root: [11; 32],
        observation_digest: [12; 32],
        deployed_slot: 13,
        installed_authority: key(7),
        source_commitment: [14; 32],
        build_inputs_commitment: [15; 32],
        package_commitment: [16; 32],
        release_manifest_commitment: [17; 32],
        release_commitment: key(18),
        release_commitment_digest: [19; 32],
        activation_receipt: OptionalPubkeyV1::some(key(20)).unwrap(),
        completed_proposal: OptionalPubkeyV1::none(),
        gate_epoch_at_activation: 2,
        deployment_generation: generation,
        deployment_digest: [0; 32],
        last_updated_slot: 22,
        reserved: [0; CURRENT_DEPLOYMENT_STATE_V1_RESERVED_LEN],
    };
    deployment.deployment_digest = compute_current_deployment_digest_v1(&deployment).unwrap();
    validate_current_deployment_digest_v1(&deployment).unwrap();
    deployment
}

fn activated_evidence() -> ActivatedDeploymentEvidenceV2 {
    ActivatedDeploymentEvidenceV2 {
        artifact_length: 24_576,
        artifact_sha256: [21; 32],
        artifact_merkle_root: [22; 32],
        artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
        actual_capacity: 32_768,
        programdata_observation: key(23),
        observation_generation: 2,
        observation_root: [24; 32],
        observation_digest: [25; 32],
        deployed_slot: 26,
        installed_authority: key(7),
        source_commitment: [27; 32],
        build_inputs_commitment: [28; 32],
        package_commitment: [29; 32],
        release_manifest_commitment: [30; 32],
        release_commitment: key(31),
        release_commitment_digest: [32; 32],
    }
}

#[test]
fn monotonic_increment_never_enters_terminal_sentinel() {
    assert_eq!(checked_nonterminal_increment(1), Ok(2));
    assert_eq!(
        checked_nonterminal_increment(u64::MAX - 2),
        Ok(u64::MAX - 1)
    );
    assert_eq!(
        checked_nonterminal_increment(u64::MAX - 1),
        Err(GovernanceError::ArithmeticOverflow)
    );
    assert_eq!(
        checked_nonterminal_increment(u64::MAX),
        Err(GovernanceError::ArithmeticOverflow)
    );
}

#[test]
fn emergency_checkpoint_separates_checkpoint_and_observation_subjects() {
    let resolution = key(1);
    let freeze_observation = key(2);
    let resolution_digest = [3; 32];
    let gate_bound_observation_subject_digest = [4; 32];
    let subjects = emergency_checkpoint_subjects(
        resolution,
        resolution_digest,
        freeze_observation,
        gate_bound_observation_subject_digest,
    );
    assert_eq!(subjects.0, resolution);
    assert_eq!(subjects.1, resolution_digest);
    assert_eq!(subjects.2, freeze_observation);
    assert_eq!(subjects.3, gate_bound_observation_subject_digest);
    assert_ne!(subjects.0, subjects.2);
    assert_ne!(subjects.1, subjects.3);
}

#[test]
fn rollback_prestate_expectation_uses_failed_primary_candidate() {
    let candidate =
        candidate_observation_expectation(24_576, [1; 32], [2; 32], ARTIFACT_MERKLE_SCHEME_ID, 91);
    assert_eq!(candidate.artifact_length, 24_576);
    assert_eq!(candidate.artifact_sha256, [1; 32]);
    assert_eq!(candidate.artifact_merkle_root, [2; 32]);
    assert_eq!(candidate.artifact_scheme_id, ARTIFACT_MERKLE_SCHEME_ID);
    assert_eq!(candidate.deployed_slot, 91);
    assert_eq!(candidate.exact_capacity, None);
}

#[test]
fn unfreeze_activation_advances_deployment_for_next_proposal() {
    let current = deployment_fixture(7);
    let evidence = activated_evidence();
    let next = apply_activated_deployment_v2(&current, &evidence, 10, 50).unwrap();
    assert_eq!(current.deployment_generation, 7);
    assert_eq!(next.deployment_generation, 8);
    assert_eq!(next.gate_epoch_at_activation, 10);
    assert_eq!(next.last_updated_slot, 50);
    assert_eq!(next.release_commitment, evidence.release_commitment);
    assert_eq!(
        next.release_commitment_digest,
        evidence.release_commitment_digest
    );
    assert_eq!(next.completed_proposal.value, evidence.release_commitment);
    assert!(next.completed_proposal.present);
    assert!(!next.activation_receipt.present);
    assert_ne!(next.deployment_digest, current.deployment_digest);
    // These are the exact two fields CreateProposalV3 requires a fresh
    // operator plan to bind, so the next proposal cannot accidentally use
    // the pre-unfreeze deployment generation.
    let next_plan_binding = (next.deployment_digest, next.deployment_generation);
    assert_eq!(next_plan_binding, (next.deployment_digest, 8));
    validate_current_deployment_digest_v1(&next).unwrap();
}

#[test]
fn unfreeze_activation_rejects_terminal_deployment_generation() {
    let current = deployment_fixture(u64::MAX - 1);
    assert_eq!(
        apply_activated_deployment_v2(&current, &activated_evidence(), 10, 50),
        Err(GovernanceError::ArithmeticOverflow)
    );
}

#[test]
fn freeze_runway_reserves_distinct_extension_and_execution_slots() {
    assert!(require_freeze_execution_runway(false, 13, 10, 1).is_ok());
    assert!(require_freeze_execution_runway(false, 12, 10, 1).is_err());
    assert!(require_freeze_execution_runway(true, 14, 10, 1).is_ok());
    assert!(require_freeze_execution_runway(true, 13, 10, 1).is_err());
}

#[test]
fn unfreeze_accumulator_rotation_resets_but_same_council_duplicate_fails() {
    let old_hash = [1; 32];
    let current_hash = [2; 32];
    assert_eq!(
        classify_unfreeze_accumulator_v2(
            ProposalStateV2::UnfreezeApproved,
            1,
            &old_hash,
            RELEASE1_APPROVAL_THRESHOLD,
            2,
            &current_hash,
        ),
        Ok(UnfreezeAccumulatorActionV2::ResetForCurrentCouncil)
    );
    assert_eq!(
        classify_unfreeze_accumulator_v2(
            ProposalStateV2::UnfreezeApproved,
            2,
            &current_hash,
            RELEASE1_APPROVAL_THRESHOLD,
            2,
            &current_hash,
        ),
        Err(GovernanceError::DuplicateApproval)
    );
    assert_eq!(
        classify_unfreeze_accumulator_v2(
            ProposalStateV2::PoststateAccepted,
            2,
            &current_hash,
            2,
            2,
            &current_hash,
        ),
        Ok(UnfreezeAccumulatorActionV2::Continue)
    );
}

#[test]
fn unfreeze_wrong_account_count_fails_before_writable_bytes_change() {
    let program_id = key(90);
    let account_key = key(91);
    let owner = key(92);
    let mut lamports = 1;
    let mut data = [9u8; 16];
    let info = AccountInfo::new(
        &account_key,
        false,
        true,
        &mut lamports,
        &mut data,
        &owner,
        false,
        0,
    );
    let accounts = [info];
    let before = accounts[0].try_borrow_data().unwrap().to_vec();
    assert_eq!(
        process_approve_unfreeze_v2(
            &program_id,
            &accounts,
            ApproveUnfreezeV2 {
                expected: unfreeze_guard(),
            },
        ),
        Err(GovernanceError::InvalidAccountCount.into())
    );
    assert_eq!(&**accounts[0].try_borrow_data().unwrap(), before.as_slice());
    assert_eq!(
        process_execute_unfreeze_v2(
            &program_id,
            &accounts,
            ExecuteUnfreezeV2 {
                expected: unfreeze_guard(),
                linked_proposal: key(93),
                envelope: CeremonyEnvelopeV1 {
                    compute_unit_limit: 1,
                    compute_unit_price_micro_lamports: 0,
                    durable_nonce_account: OptionalPubkeyV1::none(),
                    durable_nonce_authority: OptionalPubkeyV1::none(),
                },
            },
        ),
        Err(GovernanceError::InvalidAccountCount.into())
    );
    assert_eq!(&**accounts[0].try_borrow_data().unwrap(), before.as_slice());
}

#[test]
fn four_account_commit_is_failure_atomic_on_late_size_error() {
    let program_id = key(100);
    let keys = [key(101), key(102), key(103), key(104)];
    let mut first_lamports = 1u64;
    let mut second_lamports = 1u64;
    let mut third_lamports = 1u64;
    let mut fourth_lamports = 1u64;
    let mut first_data = [1u8; 4];
    let mut second_data = [2u8; 4];
    let mut third_data = [3u8; 4];
    let mut fourth_data = [4u8; 3];
    let first = AccountInfo::new(
        &keys[0],
        false,
        true,
        &mut first_lamports,
        &mut first_data,
        &program_id,
        false,
        0,
    );
    let second = AccountInfo::new(
        &keys[1],
        false,
        true,
        &mut second_lamports,
        &mut second_data,
        &program_id,
        false,
        0,
    );
    let third = AccountInfo::new(
        &keys[2],
        false,
        true,
        &mut third_lamports,
        &mut third_data,
        &program_id,
        false,
        0,
    );
    let fourth = AccountInfo::new(
        &keys[3],
        false,
        true,
        &mut fourth_lamports,
        &mut fourth_data,
        &program_id,
        false,
        0,
    );
    let before = [
        first.try_borrow_data().unwrap().to_vec(),
        second.try_borrow_data().unwrap().to_vec(),
        third.try_borrow_data().unwrap().to_vec(),
        fourth.try_borrow_data().unwrap().to_vec(),
    ];
    assert_eq!(
        commit_four_fixed_accounts(
            &program_id,
            &first,
            &[9; 4],
            4,
            &second,
            &[9; 4],
            4,
            &third,
            &[9; 4],
            4,
            &fourth,
            &[9; 4],
            4,
        ),
        Err(GovernanceError::InvalidAccountSize.into())
    );
    for (account, expected) in [&first, &second, &third, &fourth].into_iter().zip(before) {
        assert_eq!(&**account.try_borrow_data().unwrap(), expected.as_slice());
    }
}
