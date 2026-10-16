use ameba_oracle::{
    constants::{CURRENT_STATE_NAMESPACE_SEED, ORACLE_RECIPE_WEIGHT_MANIFEST_PDA_SEED},
    error::OracleError,
    instruction::{
        AccumulateOracleRecipeBucketV2Params, BeginOracleRecipeWeightsV2Params, OracleInstruction,
        OracleInstructionTag, SettlementComputation, SettlementLeaf,
    },
    processor::{
        advance_oracle_settlement_source_digest, advance_oracle_weight_manifest_hash,
        canonical_recipe_digest, initial_oracle_settlement_source_digest,
        initial_oracle_weight_manifest_hash, validate_canonical_settlement_provenance,
    },
    state::{
        derive_oracle_recipe_weight_manifest_pda, OracleActiveWeightManifest,
        OracleBucketMedianState, OracleMonthState, OracleRecipeWeightManifest,
        OracleRecipeWeightPhase, OracleSettlementSourceManifest, OracleSourceState,
    },
};
use borsh::BorshSerialize;
use solana_program::pubkey::Pubkey;

#[test]
fn canonical_weight_instructions_have_stable_append_only_wire_bytes() {
    let begin = OracleInstruction::BeginOracleRecipeWeightsV3 {
        params: BeginOracleRecipeWeightsV2Params {
            expected_source_count: 52,
            expected_bucket_count: 2,
        },
    }
    .try_to_vec()
    .unwrap();
    assert_eq!(begin[0], 187);
    assert_eq!(&begin[1..3], &52u16.to_le_bytes());
    assert_eq!(&begin[3..5], &2u16.to_le_bytes());
    assert_eq!(begin.len(), 5);

    let accumulate = OracleInstruction::AccumulateOracleRecipeBucketV2 {
        params: AccumulateOracleRecipeBucketV2Params {
            bucket_id: [3; 32],
            bucket_weight_bps: 5_000,
            finalize_collection: true,
        },
    }
    .try_to_vec()
    .unwrap();
    assert_eq!(accumulate[0], 84);
    assert_eq!(accumulate.len(), 36);

    assert_eq!(
        OracleInstruction::FinalizeOracleRecipeWeightsV2
            .try_to_vec()
            .unwrap(),
        vec![86]
    );
    for tag in [84u8, 86, 187] {
        assert!(OracleInstructionTag::from_byte(tag).is_some());
    }
    assert!(OracleInstructionTag::from_byte(85).is_none());
}

#[test]
fn current_weight_error_codes_are_stable() {
    assert_eq!(OracleError::InvalidOracleWeightManifest as u32, 6122);
    assert_eq!(OracleError::OracleWeightManifestFinalized as u32, 6123);
    assert_eq!(OracleError::InvalidOracleWeightOrder as u32, 6124);
    assert_eq!(OracleError::OracleWeightSchemeUnverified as u32, 6125);
    assert_eq!(OracleError::OracleWeightManifestHashMismatch as u32, 6126);
    assert_eq!(OracleError::OracleWeightManifestIncomplete as u32, 6127);
    assert_eq!(OracleError::InvalidOracleWeightSource as u32, 6128);
}

#[test]
fn current_month_and_manifest_fit_their_exact_allocations() {
    let current = OracleMonthState::default();
    assert_eq!(current.weight_scheme_version, 0);
    assert_eq!(current.weight_manifest_hash, [0; 32]);
    assert_eq!(current.active_weight_group_count, 0);
    assert_eq!(current.active_weight_manifest_hash, [0; 32]);
    assert_eq!(current.accepted_cash_update_count, 0);
    assert!(!current.has_rulebook_schedule());
    let encoded = borsh::to_vec(&current).unwrap();
    assert_eq!(encoded.len(), 267);
    assert_eq!(OracleMonthState::LEN, 299);
    assert!(encoded.len() <= OracleMonthState::LEN);
    let mut maximal = current.clone();
    maximal.settlement_record = Some(Pubkey::new_unique());
    assert_eq!(
        borsh::to_vec(&maximal).unwrap().len(),
        OracleMonthState::LEN
    );

    let manifest = OracleRecipeWeightManifest::default();
    let manifest_len = borsh::to_vec(&manifest).unwrap().len();
    assert_eq!(manifest_len, 181);
    assert!(manifest_len <= OracleRecipeWeightManifest::LEN);
    assert_eq!(OracleRecipeWeightManifest::LEN, 192);
    assert_eq!(OracleRecipeWeightManifest::LEN - manifest_len, 11);
}

#[test]
fn manifest_seed_phase_and_pda_are_stable() {
    assert_eq!(
        ORACLE_RECIPE_WEIGHT_MANIFEST_PDA_SEED,
        b"oracle-recipe-weights-v2"
    );
    assert_eq!(
        borsh::to_vec(&OracleRecipeWeightPhase::Collecting).unwrap(),
        vec![0]
    );
    assert_eq!(
        borsh::to_vec(&OracleRecipeWeightPhase::ReadyToFinalize).unwrap(),
        vec![2]
    );
    assert_eq!(
        borsh::to_vec(&OracleRecipeWeightPhase::Finalized).unwrap(),
        vec![3]
    );

    let program = Pubkey::new_unique();
    let month = Pubkey::new_unique();
    let expected = Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_RECIPE_WEIGHT_MANIFEST_PDA_SEED,
            month.as_ref(),
        ],
        &program,
    );
    assert_eq!(
        derive_oracle_recipe_weight_manifest_pda(&program, &month),
        expected
    );
}

#[test]
fn settlement_source_digest_binds_canonical_bucket_medians_and_weights() {
    let month = Pubkey::new_from_array([0x41; 32]);
    let recipe = [0x42; 32];
    let active_manifest_hash = [0x49; 32];
    let initial =
        initial_oracle_settlement_source_digest(&month, &recipe, &active_manifest_hash, 1, 1);
    let bucket = OracleBucketMedianState {
        bucket_id: [0x44; 32],
        bucket_weight_bps: 10_000,
        frozen_source_count: 5,
        active_source_count: 4,
        eligible_source_count: 3,
        bucket_delta_bps: 1_250,
        source_snapshot_hash: [0x48; 32],
        ..OracleBucketMedianState::default()
    };
    let canonical = advance_oracle_settlement_source_digest(&initial, &bucket);

    let mut forged_median = bucket.clone();
    forged_median.bucket_delta_bps += 1;
    assert_ne!(
        canonical,
        advance_oracle_settlement_source_digest(&initial, &forged_median)
    );
    let mut forged_weight = bucket.clone();
    forged_weight.bucket_weight_bps -= 1;
    assert_ne!(
        canonical,
        advance_oracle_settlement_source_digest(&initial, &forged_weight)
    );
    let mut substituted_snapshot = bucket;
    substituted_snapshot.source_snapshot_hash = [0xee; 32];
    assert_ne!(
        canonical,
        advance_oracle_settlement_source_digest(&initial, &substituted_snapshot)
    );
}

#[test]
fn canonical_manifest_hash_is_independent_of_support_cash() {
    let month = Pubkey::new_from_array([1; 32]);
    let initial = initial_oracle_weight_manifest_hash(&month, 2, 1);
    assert_eq!(
        initial,
        [
            134, 169, 220, 190, 35, 155, 243, 175, 116, 38, 225, 35, 57, 182, 172, 168, 133, 93,
            247, 177, 154, 224, 24, 171, 210, 124, 6, 103, 230, 129, 221, 120,
        ]
    );
    let source = OracleSourceState {
        source_id: [3; 32],
        support_stake_total: 1_234,
        canonical_locator_hash: [4; 32],
        source_definition_hash: [5; 32],
        ..OracleSourceState::default()
    };
    let final_hash = advance_oracle_weight_manifest_hash(&initial, &[2; 32], &source, 5_678);
    let richer_support = OracleSourceState {
        support_stake_total: u64::MAX,
        ..source
    };
    assert_eq!(
        final_hash,
        advance_oracle_weight_manifest_hash(&initial, &[2; 32], &richer_support, 5_678)
    );
}

#[test]
fn settlement_rejects_forged_incomplete_or_substituted_canonical_provenance() {
    let month_key = Pubkey::new_unique();
    let canonical_manifest_hash = [7; 32];
    let canonical_recipe_hash = canonical_recipe_digest(&month_key, &canonical_manifest_hash);
    let canonical_source_digest = [9; 32];
    let active_manifest_hash = [10; 32];
    let month = OracleMonthState {
        is_initialized: true,
        account_discriminator: OracleMonthState::ACCOUNT_DISCRIMINATOR,
        account_version: OracleMonthState::ACCOUNT_VERSION,
        source_count: 2,
        frozen_source_count: 2,
        opened_source_count: 2,
        opening_resolved_source_count: 2,
        recipe_hash: canonical_recipe_hash,
        weight_manifest_hash: canonical_manifest_hash,
        weight_scheme_version: 1,
        effective_weight_total_bps: 10_000,
        active_weight_scheme_version: OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION,
        active_weight_group_count: 1,
        active_weight_manifest_hash: active_manifest_hash,
        ..OracleMonthState::default()
    };
    let recipe_manifest = OracleRecipeWeightManifest {
        is_initialized: true,
        account_discriminator: OracleRecipeWeightManifest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleRecipeWeightManifest::ACCOUNT_VERSION,
        month: month_key,
        phase: OracleRecipeWeightPhase::Finalized,
        expected_source_count: 2,
        expected_bucket_count: 1,
        processed_source_count: 2,
        processed_bucket_count: 1,
        declared_weight_total_bps: 10_000,
        recipe_hash: canonical_recipe_hash,
        rolling_manifest_hash: canonical_manifest_hash,
        ..OracleRecipeWeightManifest::default()
    };
    let active_manifest = OracleActiveWeightManifest {
        is_initialized: true,
        account_discriminator: OracleActiveWeightManifest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleActiveWeightManifest::ACCOUNT_VERSION,
        month: month_key,
        phase: OracleRecipeWeightPhase::Finalized,
        expected_source_count: 2,
        expected_group_count: 1,
        processed_source_count: 2,
        processed_group_count: 1,
        processed_bucket_weight_bps: 10_000,
        rolling_manifest_hash: active_manifest_hash,
        max_open_interest_payout: 1,
        ..OracleActiveWeightManifest::default()
    };
    let source_manifest = OracleSettlementSourceManifest {
        is_initialized: true,
        account_discriminator: OracleSettlementSourceManifest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSettlementSourceManifest::ACCOUNT_VERSION,
        month: month_key,
        phase: OracleRecipeWeightPhase::Finalized,
        expected_source_count: 2,
        expected_bucket_count: 1,
        processed_source_count: 2,
        processed_bucket_count: 1,
        declared_weight_total_bps: 10_000,
        rolling_source_digest: canonical_source_digest,
        ..OracleSettlementSourceManifest::default()
    };
    let settlement = SettlementLeaf {
        schema_version: SettlementLeaf::CURRENT_SCHEMA_VERSION,
        oracle_month: month_key,
        recipe_hash: canonical_recipe_hash,
        underlying_id: [1; 32],
        item_id: "ramx".to_string(),
        expiry_id: "APR26".to_string(),
        settlement_ts: 1,
        price_display_decimals: 2,
        computation: SettlementComputation::OracleIndexDelta,
        trailing_window_days: 0,
        observations: vec![],
        settlement_price_atomic: 10_000,
        source_uri: "https://oracle.example/canonical".to_string(),
        source_digest: canonical_source_digest,
        base_oracle_atomic: 10_000,
        index_delta_bps: 0,
        submitted_by: Pubkey::new_unique(),
        submitted_slot: 1,
        signer_set_version: 1,
        signer_set_hash: [2; 32],
    };

    validate_canonical_settlement_provenance(
        &month_key,
        &month,
        &recipe_manifest,
        &active_manifest,
        &source_manifest,
        &settlement,
    )
    .unwrap();

    let mut forged = settlement.clone();
    forged.source_digest = [8; 32];
    assert_eq!(
        validate_canonical_settlement_provenance(
            &month_key,
            &month,
            &recipe_manifest,
            &active_manifest,
            &source_manifest,
            &forged,
        )
        .unwrap_err(),
        OracleError::InvalidOracleWeightManifest.into()
    );

    let mut omitted = source_manifest.clone();
    omitted.processed_source_count = 1;
    assert_eq!(
        validate_canonical_settlement_provenance(
            &month_key,
            &month,
            &recipe_manifest,
            &active_manifest,
            &omitted,
            &settlement,
        )
        .unwrap_err(),
        OracleError::InvalidOracleWeightManifest.into()
    );

    let mut substituted_recipe = recipe_manifest;
    substituted_recipe.rolling_manifest_hash = [6; 32];
    assert_eq!(
        validate_canonical_settlement_provenance(
            &month_key,
            &month,
            &substituted_recipe,
            &active_manifest,
            &source_manifest,
            &settlement,
        )
        .unwrap_err(),
        OracleError::InvalidOracleWeightManifest.into()
    );
}
