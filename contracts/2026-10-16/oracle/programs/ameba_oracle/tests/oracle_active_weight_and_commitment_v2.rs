use ameba_oracle::{
    constants::{
        ORACLE_ACTIVE_WEIGHT_MANIFEST_PDA_SEED, ORACLE_UPDATE_CLAIM_V2_PDA_SEED,
        ORACLE_UPDATE_COMMITMENT_V2_DOMAIN,
    },
    error::OracleError,
    instruction::{
        OracleInstruction, OracleInstructionTag, RecomputeOracleBucketMedianV1Params,
        RevealOracleUpdateClaimV3Params,
    },
    processor::{
        deterministic_bucket_median, oracle_update_claim_required_bond,
        oracle_update_claim_v2_commitment_hash, validate_active_group_collection_completion,
        validate_active_group_source_identity, validate_oracle_active_manifest_begin_membership,
        validate_oracle_active_manifest_completion, validate_oracle_update_claim_v2_reveal,
    },
    state::{
        derive_oracle_active_weight_manifest_pda, derive_oracle_update_claim_v2_pda,
        OracleActiveWeightManifest, OracleClaimStatus, OracleEscrowDisposition, OracleMonthState,
        OraclePhase, OracleRecipeWeightManifest, OracleRecipeWeightPhase, OracleSourceState,
        OracleSourceStatus, OracleUpdateClaimV2,
    },
    ProgramError,
};
use borsh::BorshSerialize;
use solana_program::pubkey::Pubkey;

#[test]
fn current_median_and_council_claim_protocol_versions_are_exact() {
    assert_eq!(OracleInstructionTag::BeginOracleActiveWeights as u8, 121);
    assert_eq!(
        OracleInstructionTag::RecomputeOracleBucketMedianV1 as u8,
        197
    );
    assert_eq!(
        ORACLE_ACTIVE_WEIGHT_MANIFEST_PDA_SEED,
        b"oracle-active-weights-v1"
    );
    assert_eq!(ORACLE_UPDATE_CLAIM_V2_PDA_SEED, b"oracle-update-claim-v2");
    assert_eq!(
        ORACLE_UPDATE_COMMITMENT_V2_DOMAIN,
        b"ameba-oracle-update-commit-v2"
    );
    assert_eq!(OracleActiveWeightManifest::ACCOUNT_VERSION, 3);
    assert_eq!(OracleUpdateClaimV2::ACCOUNT_VERSION, 4);
    assert!(
        OracleActiveWeightManifest::default()
            .try_to_vec()
            .unwrap()
            .len()
            <= OracleActiveWeightManifest::LEN
    );
    assert!(OracleUpdateClaimV2::default().try_to_vec().unwrap().len() <= OracleUpdateClaimV2::LEN);

    let encoded = OracleInstruction::RecomputeOracleBucketMedianV1 {
        params: RecomputeOracleBucketMedianV1Params {
            bucket_id: [7; 32],
            mode: 1,
        },
    }
    .try_to_vec()
    .unwrap();
    assert_eq!(encoded[0], 197);
    assert_eq!(encoded.len(), 34);
}

#[test]
fn deterministic_median_ignores_cash_and_rounds_even_negative_toward_zero() {
    let mut values = vec![-3, 10, -2, 10];
    assert_eq!(deterministic_bucket_median(&mut values).unwrap(), 4);
    let mut negative = vec![-3, -2];
    assert_eq!(deterministic_bucket_median(&mut negative).unwrap(), -2);
}

#[test]
fn active_manifest_requires_complete_groups_not_a_legacy_security_cap() {
    let recipe_hash = [1; 32];
    let frozen_hash = [2; 32];
    let month = OracleMonthState {
        phase: OraclePhase::Opening,
        recipe_hash,
        weight_manifest_hash: frozen_hash,
        frozen_source_count: 2,
        opened_source_count: 1,
        active_weight_group_count: 1,
        ..OracleMonthState::default()
    };
    let recipe = OracleRecipeWeightManifest {
        phase: OracleRecipeWeightPhase::Finalized,
        recipe_hash,
        rolling_manifest_hash: frozen_hash,
        expected_source_count: 2,
        expected_bucket_count: 1,
        ..OracleRecipeWeightManifest::default()
    };
    validate_oracle_active_manifest_begin_membership(&month, &recipe, 2, 1).unwrap();
    let complete = OracleActiveWeightManifest {
        phase: OracleRecipeWeightPhase::ReadyToFinalize,
        expected_source_count: 2,
        expected_group_count: 1,
        processed_source_count: 2,
        processed_group_count: 1,
        processed_bucket_weight_bps: 10_000,
        rolling_manifest_hash: [3; 32],
        max_open_interest_payout: 1,
        ..OracleActiveWeightManifest::default()
    };
    validate_oracle_active_manifest_completion(&month, &recipe, &complete).unwrap();
    let mut missing_cap = complete;
    missing_cap.max_open_interest_payout = 0;
    validate_oracle_active_manifest_completion(&month, &recipe, &missing_cap).unwrap();
}

#[test]
fn source_identity_and_group_completion_are_ordered_and_complete() {
    let group = [7; 32];
    let source = OracleSourceState {
        source_id: [10; 32],
        bucket_id: group,
        bucket_weight_bps: 10_000,
        ..OracleSourceState::default()
    };
    validate_active_group_source_identity(&group, 10_000, 0, &[0; 32], &source).unwrap();
    assert!(validate_active_group_source_identity(&group, 10_000, 1, &[11; 32], &source).is_err());
    let manifest = OracleActiveWeightManifest {
        current_group_source_count: 3,
        current_group_active_count: 1,
        ..OracleActiveWeightManifest::default()
    };
    validate_active_group_collection_completion(&manifest).unwrap();
}

#[allow(clippy::too_many_arguments)]
fn commitment(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    source_id: &[u8; 32],
    claimant: &Pubkey,
    claim_id: &[u8; 32],
    prior_state: u64,
    new_state: u64,
    source_time: u64,
    evidence: &[u8; 32],
    archive_url_hash: &[u8; 32],
    salt: &[u8; 32],
) -> [u8; 32] {
    oracle_update_claim_v2_commitment_hash(
        program_id,
        month,
        source,
        source_id,
        claimant,
        claim_id,
        prior_state,
        new_state,
        source_time,
        evidence,
        archive_url_hash,
        salt,
    )
}

#[test]
fn commitment_hash_binds_freshness_and_archive_evidence() {
    let program_id = Pubkey::new_unique();
    let month = Pubkey::new_unique();
    let source = Pubkey::new_unique();
    let claimant = Pubkey::new_unique();
    let base = commitment(
        &program_id,
        &month,
        &source,
        &[1; 32],
        &claimant,
        &[2; 32],
        100,
        110,
        1_000,
        &[3; 32],
        &[4; 32],
        &[5; 32],
    );
    assert_ne!(
        base,
        commitment(
            &program_id,
            &month,
            &source,
            &[1; 32],
            &claimant,
            &[2; 32],
            100,
            110,
            1_001,
            &[3; 32],
            &[4; 32],
            &[5; 32]
        )
    );
    assert_ne!(
        base,
        commitment(
            &program_id,
            &month,
            &source,
            &[1; 32],
            &claimant,
            &[2; 32],
            100,
            110,
            1_000,
            &[3; 32],
            &[9; 32],
            &[5; 32]
        )
    );
}

#[test]
fn reveal_guard_checks_the_extended_commitment() {
    let program_id = Pubkey::new_unique();
    let month = Pubkey::new_unique();
    let source_key = Pubkey::new_unique();
    let claimant = Pubkey::new_unique();
    let claim_id = [2; 32];
    let source = OracleSourceState {
        source_id: [1; 32],
        current_state: 100,
        status: OracleSourceStatus::Active,
        ..OracleSourceState::default()
    };
    let params = RevealOracleUpdateClaimV3Params {
        claim_id,
        prior_state: 100,
        new_state: 110,
        source_time: 1_000,
        evidence_hash: [3; 32],
        archive_url: "https://web.archive.org/web/19700101001640/https://example.com/feed".into(),
        secret_salt: [4; 32],
    };
    let archive_hash = solana_program::hash::hashv(&[
        ameba_oracle::constants::ORACLE_OPENING_ARCHIVE_URL_HASH_DOMAIN,
        params.archive_url.as_bytes(),
    ])
    .to_bytes();
    let commit_hash = commitment(
        &program_id,
        &month,
        &source_key,
        &source.source_id,
        &claimant,
        &claim_id,
        params.prior_state,
        params.new_state,
        params.source_time,
        &params.evidence_hash,
        &archive_hash,
        &params.secret_salt,
    );
    let (claim_key, bump) =
        derive_oracle_update_claim_v2_pda(&program_id, &month, &source_key, &claimant, &claim_id);
    let mut claim = OracleUpdateClaimV2::default();
    claim.claim.bump = bump;
    claim.claim.month = month;
    claim.claim.source = source_key;
    claim.claim.source_id = source.source_id;
    claim.claim.claimant = claimant;
    claim.claim.claim_id = claim_id;
    claim.claim.status = OracleClaimStatus::Committed;
    claim.claim.escrow_disposition = OracleEscrowDisposition::Unsettled;
    claim.commit_hash = commit_hash;
    claim.commit_slot = 9;
    claim.earliest_reveal_slot = 10;
    claim.reveal_deadline_slot = 20;
    validate_oracle_update_claim_v2_reveal(
        &program_id,
        &month,
        &source_key,
        &claimant,
        &claim_key,
        &source,
        &claim,
        &params,
        10,
    )
    .unwrap();
    let mut mismatched = claim;
    mismatched.commit_hash = [9; 32];
    assert_eq!(
        validate_oracle_update_claim_v2_reveal(
            &program_id,
            &month,
            &source_key,
            &claimant,
            &claim_key,
            &source,
            &mismatched,
            &params,
            10,
        ),
        Err(ProgramError::Custom(
            OracleError::OracleUpdateCommitmentMismatch as u32
        ))
    );
}

#[test]
fn active_manifest_pda_is_current_namespace_only() {
    let program = Pubkey::new_unique();
    let month = Pubkey::new_unique();
    let (derived, _) = derive_oracle_active_weight_manifest_pda(&program, &month);
    assert_ne!(derived, Pubkey::default());
    let verified = OracleMonthState {
        weight_scheme_version: 1,
        effective_weight_total_bps: 10_000,
        weight_manifest_hash: [1; 32],
        active_weight_scheme_version: OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION,
        active_weight_group_count: 1,
        active_weight_manifest_hash: [2; 32],
        ..OracleMonthState::default()
    };
    let source = OracleSourceState {
        ..OracleSourceState::default()
    };
    assert!(oracle_update_claim_required_bond(&verified, &source).unwrap() > 0);
}
