use super::*;

fn median(mut values: Vec<i64>) -> i64 {
    deterministic_bucket_median(&mut values).expect("deterministic median")
}

fn load_observation_account_for_test(
    program_id: &Pubkey,
    expected_month: &Pubkey,
    expected_source: &Pubkey,
    account_key: &Pubkey,
    account_owner: &Pubkey,
    observations: &OracleSourceObservations,
) -> Result<Box<OracleSourceObservations>, ProgramError> {
    let mut lamports = 1;
    let mut data = borsh::to_vec(observations).unwrap();
    data.resize(OracleSourceObservations::LEN, 0);
    let account = AccountInfo::new(
        account_key,
        false,
        false,
        &mut lamports,
        &mut data,
        account_owner,
        false,
        0,
    );
    load_valid_oracle_source_observations(program_id, expected_month, expected_source, &account)
}

fn blank_account_info(is_signer: bool, is_writable: bool) -> AccountInfo<'static> {
    let key = Box::leak(Box::new(Pubkey::new_unique()));
    let owner = Box::leak(Box::new(Pubkey::new_unique()));
    let lamports = Box::leak(Box::new(0u64));
    let data = Box::leak(Vec::new().into_boxed_slice());
    AccountInfo::new(key, is_signer, is_writable, lamports, data, owner, false, 0)
}

fn assert_only_required_signer_writable_promotion_is_accepted(
    accounts: &[AccountInfo],
    initial_guard_count: usize,
    mut invoke: impl FnMut(&[AccountInfo]) -> ProgramResult,
) {
    assert_ne!(
        invoke(accounts).unwrap_err(),
        OracleError::InvalidAccountList.into()
    );
    // These blank accounts exercise the initial privilege guard. State-dependent
    // checkpoint and membership tails are validated only after typed state loads.
    for index in 0..initial_guard_count {
        let mut signer_flip = accounts.to_vec();
        signer_flip[index].is_signer = !signer_flip[index].is_signer;
        assert_eq!(
            invoke(&signer_flip).unwrap_err(),
            OracleError::InvalidAccountList.into(),
            "signer flip at account {index}"
        );
        let mut writable_flip = accounts.to_vec();
        writable_flip[index].is_writable = !writable_flip[index].is_writable;
        let result = invoke(&writable_flip).unwrap_err();
        if accounts[index].is_signer && !accounts[index].is_writable {
            assert_ne!(
                result,
                OracleError::InvalidAccountList.into(),
                "required-signer fee-payer promotion at account {index}"
            );
        } else {
            assert_eq!(
                result,
                OracleError::InvalidAccountList.into(),
                "writable flip at account {index}"
            );
        }
    }
}

#[test]
fn sybil_split_gains_nothing_from_support_allocation() {
    // Five honest sources outnumber three admitted attacker identities. Moving support between
    // those identities never enters the consensus input and cannot move the median beyond the
    // honest range; obtaining another observation requires another admitted source and bond.
    let admitted = (0u8..5)
        .map(i64::from)
        .chain((0u8..3).map(|_| 1_000_000))
        .collect::<Vec<_>>();
    let support_split_between_attacker_sources = median(admitted.clone());
    let support_concentrated_on_one_attacker_source = median(admitted);
    assert_eq!(support_split_between_attacker_sources, 3);
    assert_eq!(
        support_concentrated_on_one_attacker_source,
        support_split_between_attacker_sources
    );
}

#[test]
fn median_majority_cliff() {
    let below_majority = median(vec![0, 100, 200, 1_000_000, 1_000_000]);
    assert!((0..=200).contains(&below_majority));

    let majority = median(vec![0, 100, 1_000_000, 1_000_000, 1_000_000]);
    assert_eq!(majority, 1_000_000);
}

#[test]
fn unchanged_source_carries_at_the_window_boundary() {
    let mut source = OracleSourceState {
        is_initialized: true,
        status: OracleSourceStatus::Active,
        baseline_state: 100,
        current_state: 100,
        month: Pubkey::new_unique(),
        source_id: [7; 32],
        ..OracleSourceState::default()
    };
    let mut observations = OracleSourceObservations {
        account_version: OracleSourceObservations::ACCOUNT_VERSION,
        ..OracleSourceObservations::default()
    };
    append_oracle_source_observation(
        &mut source,
        &mut observations,
        100,
        1_000,
        &[1; 32],
        &[2; 32],
    )
    .unwrap();
    let source_before = source;
    let observations_before = observations.clone();

    assert_eq!(
        oracle_temporal_median_state(&source, &observations, 2_000, 3_000).unwrap(),
        Some(100)
    );
    assert_eq!(source, source_before);
    assert_eq!(observations, observations_before);
}

#[test]
fn changed_source_window_preserves_existing_sample_median_and_boundaries() {
    let mut source = OracleSourceState {
        is_initialized: true,
        status: OracleSourceStatus::Active,
        baseline_state: 100,
        current_state: 100,
        month: Pubkey::new_unique(),
        source_id: [7; 32],
        ..OracleSourceState::default()
    };
    let mut observations = OracleSourceObservations {
        account_version: OracleSourceObservations::ACCOUNT_VERSION,
        ..OracleSourceObservations::default()
    };
    append_oracle_source_observation(
        &mut source,
        &mut observations,
        100,
        1_000,
        &[1; 32],
        &[2; 32],
    )
    .unwrap();
    append_oracle_source_observation(
        &mut source,
        &mut observations,
        120,
        2_000,
        &[3; 32],
        &[4; 32],
    )
    .unwrap();
    append_oracle_source_observation(
        &mut source,
        &mut observations,
        140,
        3_000,
        &[5; 32],
        &[6; 32],
    )
    .unwrap();
    source.current_state = 140;
    let source_before = source;
    let observations_before = observations.clone();

    assert_eq!(
        oracle_temporal_median_state(&source, &observations, 1_500, 3_000).unwrap(),
        Some(130)
    );
    // Both window boundaries remain inclusive, matching the pre-fix settlement statistic.
    assert_eq!(
        oracle_temporal_median_state(&source, &observations, 2_000, 3_000).unwrap(),
        Some(130)
    );
    assert_eq!(
        oracle_temporal_median_state(&source, &observations, 2_000, 2_999).unwrap(),
        Some(120)
    );
    assert_eq!(
        oracle_temporal_median_state(&source, &observations, 2_001, 3_000).unwrap(),
        Some(140)
    );
    assert_eq!(
        oracle_temporal_median_state(&source, &observations, 3_001, 4_000).unwrap(),
        Some(140)
    );
    assert_eq!(source, source_before);
    assert_eq!(observations, observations_before);
}

#[test]
fn final_second_change_is_not_mixed_with_the_empty_window_carry() {
    let mut source = OracleSourceState {
        is_initialized: true,
        status: OracleSourceStatus::Active,
        baseline_state: 100,
        current_state: 100,
        month: Pubkey::new_unique(),
        source_id: [7; 32],
        ..OracleSourceState::default()
    };
    let mut observations = OracleSourceObservations {
        account_version: OracleSourceObservations::ACCOUNT_VERSION,
        ..OracleSourceObservations::default()
    };
    append_oracle_source_observation(&mut source, &mut observations, 100, 900, &[1; 32], &[2; 32])
        .unwrap();
    append_oracle_source_observation(
        &mut source,
        &mut observations,
        1_001,
        1_100,
        &[3; 32],
        &[4; 32],
    )
    .unwrap();
    source.current_state = 1_001;
    let source_before = source;
    let observations_before = observations.clone();

    // HEAD's inclusive-window sample set is [1_001], so the carry fallback must not turn this
    // into the new and economically different midpoint floor((100 + 1_001) / 2) = 550.
    assert_eq!(
        oracle_temporal_median_state(&source, &observations, 1_000, 1_100).unwrap(),
        Some(1_001)
    );
    assert_eq!(source, source_before);
    assert_eq!(observations, observations_before);
}

#[test]
fn temporal_median_and_bucket_contribution_round_toward_zero() {
    let mut even_states = [100, 101];
    assert_eq!(
        super::super::median::deterministic_temporal_median(&mut even_states).unwrap(),
        100
    );
    assert_eq!(bucket_index_contribution_bps(5_000, -3).unwrap(), -1);
    assert_eq!(bucket_index_contribution_bps(5_000, 3).unwrap(), 1);
    assert_eq!(
        super::super::median::deterministic_temporal_median(&mut []),
        Err(OracleError::InvalidOracleMedian.into())
    );
    assert_eq!(
        bucket_index_contribution_bps(0, 1),
        Err(OracleError::InvalidOracleMedian.into())
    );
    assert_eq!(
        bucket_index_contribution_bps(10_001, 1),
        Err(OracleError::InvalidOracleMedian.into())
    );
}

#[test]
fn freshness_window_counts_utc_business_days_across_weekends() {
    // Monday 1970-01-12 00:00 UTC minus five business days is Monday 1970-01-05.
    let monday_january_12 = 11 * 86_400;
    assert_eq!(
        oracle_settlement_window_start(monday_january_12, 5).unwrap(),
        4 * 86_400
    );
    assert_eq!(
        oracle_settlement_window_start(0, 5),
        Err(OracleError::InvalidOracleMedian.into())
    );
    assert_eq!(
        oracle_settlement_window_start(monday_january_12, 0),
        Err(OracleError::InvalidOracleMedian.into())
    );
}

#[test]
fn observation_shape_rejects_noncanonical_history() {
    let mut source = OracleSourceState {
        is_initialized: true,
        status: OracleSourceStatus::Active,
        baseline_state: 100,
        current_state: 100,
        month: Pubkey::new_unique(),
        source_id: [7; 32],
        ..OracleSourceState::default()
    };
    let mut observations = OracleSourceObservations {
        account_version: OracleSourceObservations::ACCOUNT_VERSION,
        ..OracleSourceObservations::default()
    };
    append_oracle_source_observation(
        &mut source,
        &mut observations,
        100,
        1_000,
        &[1; 32],
        &[2; 32],
    )
    .unwrap();
    validate_oracle_observation_shape(&source, &observations).unwrap();

    let mut nonzero_tail = observations.clone();
    nonzero_tail.states[1] = 101;
    assert!(validate_oracle_observation_shape(&source, &nonzero_tail).is_err());

    let mut duplicate_source = source;
    let mut duplicate_time = observations;
    assert!(append_oracle_source_observation(
        &mut duplicate_source,
        &mut duplicate_time,
        101,
        1_000,
        &[3; 32],
        &[4; 32],
    )
    .is_err());

    let mut mismatched_current_state = source;
    mismatched_current_state.current_state = 101;
    assert!(validate_oracle_observation_shape(&mismatched_current_state, &duplicate_time).is_err());
}

#[test]
fn month_bucket_contribution_replaces_prior_value_exactly() {
    let mut month = OracleMonthState {
        index_delta_bps: 250,
        ..OracleMonthState::default()
    };
    let mut bucket = OracleBucketMedianState {
        bucket_weight_bps: 5_000,
        bucket_delta_bps: 500,
        ..OracleBucketMedianState::default()
    };
    update_month_bucket_contribution(&mut month, &mut bucket, -300).unwrap();
    assert_eq!(bucket.bucket_delta_bps, -300);
    assert_eq!(month.index_delta_bps, -150);
}

#[test]
fn median_persistent_hashes_bind_the_exact_canonical_fields() {
    let month = Pubkey::new_unique();
    let source_id = [3; 32];
    let bucket_id = [4; 32];
    let evidence_hash = [5; 32];
    let archive_hash = [6; 32];
    let mut source = OracleSourceState {
        month,
        source_id,
        bucket_id,
        status: OracleSourceStatus::Active,
        baseline_state: 100,
        ..OracleSourceState::default()
    };
    let mut observations = OracleSourceObservations {
        account_version: OracleSourceObservations::ACCOUNT_VERSION,
        ..OracleSourceObservations::default()
    };
    append_oracle_source_observation(
        &mut source,
        &mut observations,
        100,
        1_000,
        &evidence_hash,
        &archive_hash,
    )
    .unwrap();
    let expected_observation_hash = hashv(&[
        crate::constants::ORACLE_UPDATE_EVIDENCE_HASH_DOMAIN,
        &[0; 32],
        month.as_ref(),
        &source_id,
        &1u32.to_le_bytes(),
        &100u64.to_le_bytes(),
        &1_000u64.to_le_bytes(),
        &evidence_hash,
        &archive_hash,
    ])
    .to_bytes();
    assert_eq!(source.rolling_observation_hash, expected_observation_hash);

    let expected_initial_snapshot = initial_oracle_bucket_source_snapshot(&month, &bucket_id, 0);
    let expected_advanced_snapshot = hashv(&[
        crate::constants::ORACLE_BUCKET_MEDIAN_HASH_DOMAIN,
        &expected_initial_snapshot,
        &source_id,
        &bucket_id,
        &100u64.to_le_bytes(),
        &source.observation_count.to_le_bytes(),
        &expected_observation_hash,
    ])
    .to_bytes();
    assert_eq!(
        advance_oracle_bucket_source_snapshot(&expected_initial_snapshot, &source),
        expected_advanced_snapshot
    );
}

#[test]
fn eligible_source_ceiling_handles_the_full_u16_domain() {
    assert_eq!(minimum_oracle_bucket_eligible_sources(u16::MAX), 32_768);
}

#[test]
fn even_n_median_deterministic_with_negative_values_and_ties() {
    // The ordered source walk already supplies equal deltas in source-id order. Stable insertion
    // sorting preserves that order while the consensus output remains the numeric median.
    let values = vec![-3, -2, -2, 9];
    assert_eq!(median(values.clone()), -2);
    assert_eq!(median(values.into_iter().rev().collect()), -2);
    assert_eq!(median(vec![-3, -2]), -2);
    assert_eq!(median(vec![i64::MIN, i64::MAX]), 0);
    assert_eq!(median(vec![i64::MIN, i64::MIN]), i64::MIN);
    assert_eq!(median(vec![i64::MAX, i64::MAX]), i64::MAX);

    let mut temporal_extremes = [0, u64::MAX];
    assert_eq!(
        super::super::median::deterministic_temporal_median(&mut temporal_extremes).unwrap(),
        u64::MAX / 2
    );
}

#[test]
fn the_bucket_settlement_floor_is_the_ordinary_floor_except_for_dev_buckets_under_three() {
    for frozen in [3u16, 4, 5, 8, 9, 32, 1_000, u16::MAX] {
        assert_eq!(
            bucket_settlement_floor(frozen),
            minimum_oracle_bucket_eligible_sources(frozen)
        );
    }
    #[cfg(not(any(feature = "localnet-dev-authority", feature = "localnet-governance")))]
    for frozen in [0u16, 1, 2] {
        assert_eq!(bucket_settlement_floor(frozen), 3);
    }
    #[cfg(any(feature = "localnet-dev-authority", feature = "localnet-governance"))]
    for (frozen, floor) in [(0u16, 1u16), (1, 1), (2, 2)] {
        assert_eq!(bucket_settlement_floor(frozen), floor);
    }
}

#[test]
fn eligibility_fallback_requires_grace_instead_of_silent_settlement() {
    assert_eq!(minimum_oracle_bucket_eligible_sources(1), 3);
    assert_eq!(minimum_oracle_bucket_eligible_sources(5), 3);
    assert_eq!(minimum_oracle_bucket_eligible_sources(8), 4);
    assert!(2 < minimum_oracle_bucket_eligible_sources(5));
}

#[test]
fn oracle_security_budget_cap_is_exact() {
    let (threshold, cap) = oracle_bucket_security_cap(5, 1_000, 1_000, 5_000).unwrap();
    assert_eq!(threshold, 3);
    assert_eq!(cap, 3_000);
    assert!(oracle_bucket_security_cap(5, 1_000, 1_000, 5_001).is_err());
    let minimum_fresh = minimum_oracle_bucket_eligible_sources(32);
    let (worst_case_threshold, _) =
        oracle_bucket_security_cap(minimum_fresh, 1_000, 1_000, 5_000).unwrap();
    assert_eq!((minimum_fresh, worst_case_threshold), (16, 9));
}

#[test]
fn temporal_median_boundaries_are_permutation_and_duplicate_stable() {
    assert!(super::super::median::deterministic_temporal_median(&mut []).is_err());
    assert_eq!(
        super::super::median::deterministic_temporal_median(&mut [7]).unwrap(),
        7
    );
    assert_eq!(
        super::super::median::deterministic_temporal_median(&mut [0, u64::MAX]).unwrap(),
        u64::MAX / 2
    );
    let mut ordered = [1, 5, 5, 9];
    let mut permuted = [9, 5, 1, 5];
    assert_eq!(
        super::super::median::deterministic_temporal_median(&mut ordered).unwrap(),
        5
    );
    assert_eq!(
        super::super::median::deterministic_temporal_median(&mut permuted).unwrap(),
        5
    );
}

#[test]
fn oracle_security_budget_rejects_invalid_and_overflow_boundaries() {
    for active in [1, 2, 3, 4, 5, u16::MAX] {
        let (threshold, _) = oracle_bucket_security_cap(active, 1, 1, 5_000).unwrap();
        assert_eq!(threshold, active / 2 + 1);
    }
    assert!(oracle_bucket_security_cap(0, 1, 1, 5_000).is_err());
    assert!(oracle_bucket_security_cap(1, 0, 1, 5_000).is_err());
    assert!(oracle_bucket_security_cap(1, 1, 0, 5_000).is_err());
    assert!(oracle_bucket_security_cap(1, 1, 1, 0).is_err());
    assert!(oracle_bucket_security_cap(1, 1, 1, 5_001).is_err());
    assert!(oracle_bucket_security_cap(1, u64::MAX, 1, 5_000).is_err());
    assert!(oracle_bucket_security_cap(3, u64::MAX / 2, 1, 5_000).is_err());
}

#[test]
fn minimum_security_cap_across_economically_active_buckets_is_order_independent() {
    let bucket_cap = |frozen_source_count: u16,
                      active_source_count: u16,
                      listing_bond: u64,
                      support_bond: u64| {
        let eligible =
            active_source_count.min(minimum_oracle_bucket_eligible_sources(frozen_source_count));
        oracle_bucket_security_cap(eligible, listing_bond, support_bond, 5_000)
            .unwrap()
            .1
    };
    let strong = bucket_cap(9, 9, 1_000, 1_000);
    let weak_at_one = bucket_cap(5, 1, 100, 100);
    let weak_at_threshold = bucket_cap(5, 3, 100, 100);
    assert_eq!((strong, weak_at_one, weak_at_threshold), (3_000, 100, 200));

    let fold = |buckets: &[(u16, u64)]| {
        buckets.iter().fold(u64::MAX, |current, (weight, cap)| {
            fold_economically_active_bucket_security_cap(current, *weight, *cap)
        })
    };
    for weak in [weak_at_one, weak_at_threshold] {
        assert_eq!(fold(&[(6_000, strong), (4_000, weak)]), weak);
        assert_eq!(fold(&[(4_000, weak), (6_000, strong)]), weak);
        assert_eq!(
            fold(&[(0, 1), (6_000, strong), (0, u64::MAX), (4_000, weak)]),
            weak
        );
    }
}

#[test]
fn temporal_median_observation_account_rejects_version_and_domain_substitution() {
    let program_id = Pubkey::new_unique();
    let month = Pubkey::new_unique();
    let source = Pubkey::new_unique();
    let (account_key, bump) = derive_oracle_source_observations_pda(&program_id, &source);
    let canonical = OracleSourceObservations {
        is_initialized: true,
        bump,
        account_discriminator: OracleSourceObservations::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSourceObservations::ACCOUNT_VERSION,
        month,
        source,
        ..OracleSourceObservations::default()
    };
    load_observation_account_for_test(
        &program_id,
        &month,
        &source,
        &account_key,
        &program_id,
        &canonical,
    )
    .unwrap();

    let mut mutated = canonical.clone();
    mutated.account_version = OracleSourceObservations::INHERITED_ANCHOR_VERSION;
    load_observation_account_for_test(
        &program_id,
        &month,
        &source,
        &account_key,
        &program_id,
        &mutated,
    )
    .unwrap();
    mutated.account_version = OracleSourceObservations::INHERITED_ANCHOR_VERSION + 1;
    assert!(load_observation_account_for_test(
        &program_id,
        &month,
        &source,
        &account_key,
        &program_id,
        &mutated,
    )
    .is_err());
    mutated = canonical.clone();
    mutated.account_discriminator[0] ^= 1;
    assert!(load_observation_account_for_test(
        &program_id,
        &month,
        &source,
        &account_key,
        &program_id,
        &mutated,
    )
    .is_err());
    mutated = canonical.clone();
    mutated.month = Pubkey::new_unique();
    assert!(load_observation_account_for_test(
        &program_id,
        &month,
        &source,
        &account_key,
        &program_id,
        &mutated,
    )
    .is_err());
    mutated = canonical.clone();
    mutated.source = Pubkey::new_unique();
    assert!(load_observation_account_for_test(
        &program_id,
        &month,
        &source,
        &account_key,
        &program_id,
        &mutated,
    )
    .is_err());
    assert!(load_observation_account_for_test(
        &program_id,
        &month,
        &source,
        &account_key,
        &Pubkey::new_unique(),
        &canonical,
    )
    .is_err());
    assert!(load_observation_account_for_test(
        &Pubkey::new_unique(),
        &month,
        &source,
        &account_key,
        &program_id,
        &canonical,
    )
    .is_err());
}

#[test]

fn incompatible_raw_source_levels_are_normalized_before_bucket_median() {
    let mut normalized = [
        source_delta_bps(100, 110).unwrap(),
        source_delta_bps(1_000, 1_050).unwrap(),
        source_delta_bps(10, 12).unwrap(),
    ];

    assert_eq!(deterministic_bucket_median(&mut normalized).unwrap(), 1_000);

    // A median over the raw vendor levels would be 110 and has no shared unit.

    let mut raw_levels = [110, 1_050, 12];

    assert_eq!(
        super::super::median::deterministic_temporal_median(&mut raw_levels).unwrap(),
        110
    );
}

#[test]

fn source_local_delta_rejects_zero_and_covers_rounding_and_overflow_boundaries() {
    assert!(source_delta_bps(0, 1).is_err());

    assert!(source_delta_bps(1, 0).is_err());

    assert_eq!(source_delta_bps(3, 4).unwrap(), 3_333);

    assert_eq!(source_delta_bps(3, 2).unwrap(), -3_333);

    assert_eq!(source_delta_bps(10_000, 10_001).unwrap(), 1);

    assert_eq!(source_delta_bps(10_000, 9_999).unwrap(), -1);

    assert_eq!(source_delta_bps(u64::MAX, u64::MAX - 1).unwrap(), 0);

    assert!(source_delta_bps(1, u64::MAX).is_err());
}

#[test]
fn bucket_median_recompute_allows_only_required_signer_writable_promotion() {
    let program_id = Pubkey::new_unique();
    let mut recompute_accounts = vec![
        blank_account_info(true, false),
        blank_account_info(false, false),
        blank_account_info(false, true),
        blank_account_info(false, true),
        blank_account_info(false, true),
        blank_account_info(false, false),
        blank_account_info(false, false),
        blank_account_info(false, false),
    ];
    recompute_accounts.extend([
        blank_account_info(false, false),
        blank_account_info(false, false),
    ]);
    assert_only_required_signer_writable_promotion_is_accepted(
        &recompute_accounts,
        6,
        |accounts| {
            super::super::bucket_medians::process_recompute_oracle_bucket_median_v1(
                &program_id,
                accounts,
                RecomputeOracleBucketMedianV1Params {
                    bucket_id: [4; 32],
                    mode: 0,
                },
            )
        },
    );
    let grace_recompute_accounts = recompute_accounts.clone();
    assert_only_required_signer_writable_promotion_is_accepted(
        &grace_recompute_accounts,
        6,
        |accounts| {
            super::super::bucket_medians::process_recompute_oracle_bucket_median_v1(
                &program_id,
                accounts,
                RecomputeOracleBucketMedianV1Params {
                    bucket_id: [4; 32],
                    mode: 1,
                },
            )
        },
    );
}

#[test]
fn equal_state_update_remains_rejected_when_carry_supplies_freshness() {
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
    let mut params = RevealOracleUpdateClaimV3Params {
        claim_id,
        prior_state: 100,
        new_state: 101,
        source_time: 1_000,
        evidence_hash: [3; 32],
        archive_url: "https://web.archive.org/web/19700101001640/https://example.com/feed".into(),
        secret_salt: [4; 32],
    };
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
    claim.commit_slot = 9;
    claim.earliest_reveal_slot = 10;
    claim.reveal_deadline_slot = 20;
    claim.commit_hash = oracle_update_claim_v2_commitment_hash(
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
        &derive_oracle_opening_archive_url_hash(&params.archive_url),
        &params.secret_salt,
    );
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

    params.new_state = params.prior_state;
    claim.commit_hash = oracle_update_claim_v2_commitment_hash(
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
        &derive_oracle_opening_archive_url_hash(&params.archive_url),
        &params.secret_salt,
    );
    assert_eq!(
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
        ),
        Err(OracleError::InvalidOracleUpdateRevealTiming.into())
    );
}

#[test]
fn update_claim_handlers_allow_only_required_signer_writable_promotion() {
    let program_id = Pubkey::new_unique();
    let reveal_accounts = vec![
        blank_account_info(true, true),
        blank_account_info(false, false),
        blank_account_info(false, false),
        blank_account_info(false, false),
        blank_account_info(false, false),
        blank_account_info(false, true),
        blank_account_info(false, false),
        blank_account_info(false, false),
        blank_account_info(false, true),
    ];
    assert_only_required_signer_writable_promotion_is_accepted(&reveal_accounts, 9, |accounts| {
        super::super::usdc::process_reveal_oracle_update_claim_v3(
            &program_id,
            accounts,
            RevealOracleUpdateClaimV3Params {
                claim_id: [1; 32],
                prior_state: 1,
                new_state: 2,
                source_time: 1,
                evidence_hash: [2; 32],
                archive_url: "https://web.archive.org/web/19700101000001/https://example.com"
                    .into(),
                secret_salt: [3; 32],
            },
        )
    });

    let mut finalize_accounts = vec![
        blank_account_info(true, false),
        blank_account_info(false, false),
        blank_account_info(false, false),
        blank_account_info(false, true),
        blank_account_info(false, true),
        blank_account_info(false, true),
        blank_account_info(false, false),
        blank_account_info(false, true),
        blank_account_info(false, true),
        blank_account_info(false, false),
    ];
    let checkpoint_tail = [
        blank_account_info(false, true),
        blank_account_info(false, true),
        blank_account_info(false, false),
    ];
    finalize_accounts.extend(checkpoint_tail.clone());
    assert_only_required_signer_writable_promotion_is_accepted(
        &finalize_accounts,
        10,
        |accounts| {
            super::super::usdc::process_finalize_oracle_update_claim_v2(
                &program_id,
                accounts,
                FinalizeOracleUpdateClaimV2Params {
                    outcome: OracleUpdateClaimOutcome::AcceptClaim,
                    current_step: 1,
                },
            )
        },
    );
    let mut challenged_finalize_accounts = finalize_accounts[..9].to_vec();
    challenged_finalize_accounts.extend([
        blank_account_info(false, true),
        blank_account_info(false, false),
    ]);
    challenged_finalize_accounts.extend(checkpoint_tail);
    assert_only_required_signer_writable_promotion_is_accepted(
        &challenged_finalize_accounts,
        11,
        |accounts| {
            super::super::usdc::process_finalize_oracle_update_claim_v2(
                &program_id,
                accounts,
                FinalizeOracleUpdateClaimV2Params {
                    outcome: OracleUpdateClaimOutcome::RejectClaim,
                    current_step: 1,
                },
            )
        },
    );
    let mut unresolved_finalize_accounts = finalize_accounts[..9].to_vec();
    unresolved_finalize_accounts.extend([
        blank_account_info(false, true),
        blank_account_info(false, true),
    ]);
    assert_only_required_signer_writable_promotion_is_accepted(
        &unresolved_finalize_accounts,
        11,
        |accounts| {
            super::super::usdc::process_finalize_oracle_update_claim_v2(
                &program_id,
                accounts,
                FinalizeOracleUpdateClaimV2Params {
                    outcome: OracleUpdateClaimOutcome::RuleReviewUnresolved,
                    current_step: 1,
                },
            )
        },
    );
}

#[test]
fn current_update_bond_ignores_informational_bucket_weight() {
    let month = OracleMonthState {
        weight_scheme_version: 1,
        effective_weight_total_bps: 10_000,
        weight_manifest_hash: [8; 32],
        active_weight_scheme_version: OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION,
        active_weight_group_count: 1,
        active_weight_manifest_hash: [9; 32],
        ..OracleMonthState::default()
    };
    let source = OracleSourceState {
        listing_bond_locked: 1_000,
        support_stake_total: 1_000_000,
        bucket_weight_bps: 5_000,
        ..OracleSourceState::default()
    };
    let expected = oracle_update_claim_required_bond(&month, &source).unwrap();
    let informational_only = OracleSourceState {
        bucket_weight_bps: 10_000,
        ..source
    };
    assert_eq!(
        oracle_update_claim_required_bond(&month, &informational_only).unwrap(),
        expected
    );
}

#[test]
fn oracle_source_merge_folds_support_without_buying_weight() {
    let month = Pubkey::new_unique();
    let bucket_id = [9; 32];
    let mut duplicate = OracleSourceState {
        month,
        bucket_id,
        source_id: [1; 32],
        support_stake_total: 400,
        status: OracleSourceStatus::Candidate,
        ..OracleSourceState::default()
    };
    let mut canonical = OracleSourceState {
        month,
        bucket_id,
        source_id: [2; 32],
        support_stake_total: 600,
        status: OracleSourceStatus::Candidate,
        ..OracleSourceState::default()
    };
    apply_oracle_source_merge(&mut duplicate, &mut canonical).unwrap();
    assert_eq!(duplicate.status, OracleSourceStatus::Merged);
    assert_eq!(canonical.support_stake_total, 1_000);
    assert_eq!(canonical.bucket_weight_bps, 0);
}
