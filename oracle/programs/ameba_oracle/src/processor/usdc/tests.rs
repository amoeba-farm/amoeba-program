use super::*;

#[test]
fn current_update_commitment_matches_bundled_client_vector() {
    let actual = oracle_update_claim_v2_commitment_hash(
        &Pubkey::new_from_array([90; 32]),
        &Pubkey::new_from_array([91; 32]),
        &Pubkey::new_from_array([92; 32]),
        &[4; 32],
        &Pubkey::new_from_array([9; 32]),
        &[1; 32],
        100_000,
        100_500,
        1_000_000,
        &[2; 32],
        &derive_oracle_opening_archive_url_hash(
            "https://web.archive.org/web/19700112134640/https://example.test/oracle",
        ),
        &[8; 32],
    );
    let expected = "92941331d43e1501e77aad00627fad9bf8a0224176e053b8c1fab4a174d6b53d";
    let hex: String = actual.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(hex, expected);
}

#[test]
fn public_reveal_gets_the_profile_window_independent_of_commit_slots() {
    let claim = OracleUpdateClaimV2 {
        revealed_at_ts: 100,
        ..Default::default()
    };
    let ready = 100 + crate::constants::ORACLE_UPDATE_CHALLENGE_SECONDS;
    for now in [100, ready - 1] {
        assert_eq!(
            ensure_update_challenge_elapsed_at(&claim, now),
            Err(OracleError::OracleTimingWindowClosed.into())
        );
    }
    assert_eq!(ensure_update_challenge_elapsed_at(&claim, ready), Ok(()));
    assert_eq!(
        ensure_update_challenge_elapsed_at(&claim, ready + 1),
        Ok(())
    );
    assert!(ensure_update_challenge_elapsed_at(&OracleUpdateClaimV2::default(), u64::MAX).is_err());
    assert!(ensure_update_challenge_elapsed_at(
        &OracleUpdateClaimV2 {
            revealed_at_ts: u64::MAX,
            ..claim
        },
        u64::MAX
    )
    .is_err());
}

#[test]
fn grace_cannot_timeout_a_current_challenge_or_an_open_vote() {
    let anchor = crate::processor::tests::sample_anchor(100);
    let source = OracleSourceState {
        status: OracleSourceStatus::Active,
        opening_submitted: true,
        current_state: 100,
        last_finalized_step: 7,
        ..Default::default()
    };
    let claim = OracleUpdateClaimV2 {
        claim: OracleUpdateClaimData {
            prior_state: 100,
            ..Default::default()
        },
        prior_finalized_step: 7,
        ..Default::default()
    };
    let guard = OracleUpdateChallengeGuard::default();
    assert!(ensure_oracle_update_cleanup_ready_at(
        &anchor,
        &source,
        &claim,
        Some(&guard),
        1_000_000
    )
    .is_err());
    assert!(ensure_oracle_update_cleanup_ready_at(&anchor, &source, &claim, None, 86_500).is_ok());
    let stale = OracleSourceState {
        last_finalized_step: 8,
        ..source
    };
    assert!(ensure_oracle_update_cleanup_ready_at(
        &anchor,
        &stale,
        &claim,
        Some(&guard),
        1_000_000
    )
    .is_ok());
}

fn coverage_schedule_month(phase: OraclePhase) -> OracleMonthState {
    let scramble_start_ts = 1_800_000_000;
    OracleMonthState {
        scramble_start_ts,
        listing_ts: scramble_start_ts + ORACLE_PRE_LISTING_WINDOW_SECONDS,
        phase,
        ..OracleMonthState::default()
    }
}

#[test]
fn late_source_submission_can_build_rewards_only_while_full_review_still_fits() {
    let source_submission = coverage_schedule_month(OraclePhase::SourceSubmission);
    let (placement_end, _, _) = rulebook_schedule_boundaries(&source_submission).unwrap();
    let expiry_ts = source_submission.listing_ts + 7_948_800;
    let latest_safe = oracle_source_submission_latest_safe_ts(expiry_ts).unwrap();

    assert!(ensure_oracle_usdc_schedule_build_window_at(
        expiry_ts,
        &source_submission,
        placement_end - 1,
    )
    .is_ok());
    assert!(ensure_oracle_usdc_schedule_build_window_at(
        expiry_ts,
        &source_submission,
        placement_end,
    )
    .is_ok());
    assert!(ensure_oracle_usdc_schedule_build_window_at(
        expiry_ts,
        &source_submission,
        latest_safe - 1,
    )
    .is_ok());
    assert_eq!(
        ensure_oracle_usdc_schedule_build_window_at(expiry_ts, &source_submission, latest_safe,),
        Err(OracleError::OracleTimingWindowClosed.into())
    );

    let preserved_review = ORACLE_KILL_WINDOW_SECONDS
        + ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
        + ORACLE_OPENING_WINDOW_SECONDS;
    let tight_expiry_ts = placement_end + preserved_review - 1;
    let tight_latest_safe = oracle_source_submission_latest_safe_ts(tight_expiry_ts).unwrap();
    assert!(tight_latest_safe < placement_end);
    assert!(ensure_oracle_usdc_schedule_build_window_at(
        tight_expiry_ts,
        &source_submission,
        tight_latest_safe - 1,
    )
    .is_ok());
    assert_eq!(
        ensure_oracle_usdc_schedule_build_window_at(
            tight_expiry_ts,
            &source_submission,
            tight_latest_safe,
        ),
        Err(OracleError::OracleTimingWindowClosed.into())
    );

    let scramble = coverage_schedule_month(OraclePhase::Scramble);
    assert!(
        ensure_oracle_usdc_schedule_build_window_at(expiry_ts, &scramble, placement_end - 1,)
            .is_ok()
    );
    assert_eq!(
        ensure_oracle_usdc_schedule_build_window_at(expiry_ts, &scramble, placement_end),
        Err(OracleError::OracleTimingWindowClosed.into())
    );

    let opening = coverage_schedule_month(OraclePhase::Opening);
    assert_eq!(
        ensure_oracle_usdc_schedule_build_window_at(expiry_ts, &opening, placement_end - 1,),
        Err(OracleError::InvalidOraclePhase.into())
    );
}

#[test]
fn equal_floor_never_exceeds_the_budget() {
    for budget in 0..100_u64 {
        for count in 1..20_u32 {
            let total = (0..count)
                .map(|_| calculate_oracle_usdc_equal_share(budget, count).unwrap())
                .sum::<u64>();
            assert!(total <= budget);
            assert!(budget - total < u64::from(count));
        }
    }
}

#[test]
fn source_count_saturates_each_source_cash_share() {
    assert_eq!(calculate_oracle_usdc_equal_share(100, 2).unwrap(), 50);
    assert_eq!(calculate_oracle_usdc_equal_share(100, 100).unwrap(), 1);
}

#[test]
fn proposer_is_protected_and_lone_proposer_receives_all() {
    assert_eq!(
        calculate_oracle_usdc_source_reward_allocation(100, 1, 2_000, 4).unwrap(),
        OracleUsdcSourceRewardAllocation {
            source_reward_budget: 100,
            proposer_reward: 20,
            supporter_reward_budget: 80,
        }
    );
    assert_eq!(
        calculate_oracle_usdc_source_reward_allocation(100, 1, 2_000, 0).unwrap(),
        OracleUsdcSourceRewardAllocation {
            source_reward_budget: 100,
            proposer_reward: 100,
            supporter_reward_budget: 0,
        }
    );
}

#[test]
fn protected_split_and_equal_supporters_never_exceed_source_budget() {
    for budget in 0..250_u64 {
        for source_count in 1..20_u32 {
            for supporter_count in 1..20_u32 {
                let allocation = calculate_oracle_usdc_source_reward_allocation(
                    budget,
                    source_count,
                    2_500,
                    supporter_count,
                )
                .unwrap();
                let supporter_share = calculate_oracle_usdc_supporter_reward(
                    allocation.supporter_reward_budget,
                    supporter_count,
                )
                .unwrap();
                let paid = allocation
                    .proposer_reward
                    .checked_add(
                        supporter_share
                            .checked_mul(u64::from(supporter_count))
                            .unwrap(),
                    )
                    .unwrap();
                assert!(paid <= allocation.source_reward_budget);
                assert!(allocation.source_reward_budget <= budget);
            }
        }
    }
}

#[test]
fn empty_recipient_set_and_invalid_proposer_bps_fail_closed() {
    assert!(calculate_oracle_usdc_equal_share(100, 0).is_err());
    assert!(calculate_oracle_usdc_source_reward_allocation(100, 1, 0, 1).is_err());
    assert!(calculate_oracle_usdc_source_reward_allocation(100, 1, 10_001, 1).is_err());
}

#[test]
fn supporter_reward_is_independent_of_bond_size() {
    let shares = (0..3)
        .map(|_| calculate_oracle_usdc_supporter_reward(100, 3).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(shares, vec![33, 33, 33]);
}

#[test]
fn transitive_merge_carries_every_supporter_to_the_final_root() {
    let source_a = Pubkey::new_unique();
    let source_b = Pubkey::new_unique();
    let source_c = Pubkey::new_unique();
    let mut reward_a = OracleUsdcSourceReward {
        supporter_count: 2,
        ..OracleUsdcSourceReward::default()
    };
    let mut reward_b = OracleUsdcSourceReward {
        supporter_count: 3,
        ..OracleUsdcSourceReward::default()
    };
    let mut reward_c = OracleUsdcSourceReward {
        supporter_count: 5,
        ..OracleUsdcSourceReward::default()
    };

    carry_merged_supporter_rewards(&source_a, &mut reward_a, &source_b, &mut reward_b).unwrap();
    carry_merged_supporter_rewards(&source_b, &mut reward_b, &source_c, &mut reward_c).unwrap();

    assert_eq!(reward_a.merged_into_source, source_b);
    assert_eq!(reward_b.merged_into_source, source_c);
    assert_eq!(reward_b.supporter_count, 5);
    assert_eq!(reward_c.supporter_count, 10);
    assert_eq!(reward_c.max_merge_depth, 2);
}

#[test]
fn merge_depth_bound_and_relink_are_fail_closed() {
    let source_a = Pubkey::new_unique();
    let source_b = Pubkey::new_unique();
    let mut depth_bound = OracleUsdcSourceReward {
        max_merge_depth: MAX_ORACLE_USDC_MERGE_DEPTH,
        ..OracleUsdcSourceReward::default()
    };
    let mut root = OracleUsdcSourceReward::default();
    assert!(
        carry_merged_supporter_rewards(&source_a, &mut depth_bound, &source_b, &mut root).is_err()
    );

    let mut already_linked = OracleUsdcSourceReward {
        merged_into_source: Pubkey::new_unique(),
        ..OracleUsdcSourceReward::default()
    };
    assert!(
        carry_merged_supporter_rewards(&source_a, &mut already_linked, &source_b, &mut root)
            .is_err()
    );
}

#[test]
fn zero_support_timeout_preserves_complete_coverage() {
    let scramble_start = 1_000_000;
    let mut month = OracleMonthState {
        scramble_start_ts: scramble_start,
        listing_ts: scramble_start + ORACLE_PRE_LISTING_WINDOW_SECONDS,
        phase: OraclePhase::Scramble,
        ..OracleMonthState::default()
    };
    let coverage = OracleSkuCoverageManifest {
        coverage_finalized: true,
        required_sku_count: 1,
        covered_sku_count: 1,
        ..OracleSkuCoverageManifest::default()
    };
    let original_coverage = coverage.clone();
    let mut source = OracleSourceState {
        status: OracleSourceStatus::Candidate,
        listing_bond_locked: 10,
        support_stake_total: 0,
        ..OracleSourceState::default()
    };
    timeout_unsupported_oracle_source_state(
        &mut month,
        &coverage,
        &mut source,
        scramble_start + ORACLE_PLACEMENT_WINDOW_SECONDS,
        99,
    )
    .unwrap();
    assert_eq!(source.status, OracleSourceStatus::TimedOut);
    assert_eq!(month.phase, OraclePhase::Scramble);
    assert_eq!(coverage, original_coverage);
}

#[test]
fn challenge_bond_is_clamped() {
    assert_eq!(
        calculate_oracle_usdc_challenge_bond(10, 1_500, 5, 40).unwrap(),
        5
    );
    assert_eq!(
        calculate_oracle_usdc_challenge_bond(100, 1_500, 5, 40).unwrap(),
        15
    );
    assert_eq!(
        calculate_oracle_usdc_challenge_bond(1_000, 1_500, 5, 40).unwrap(),
        40
    );
}

#[test]
fn challenge_bond_covers_floor_clamp_and_u64_boundaries() {
    assert_eq!(calculate_oracle_usdc_challenge_bond(1, 1, 1, 9).unwrap(), 1);
    assert_eq!(
        calculate_oracle_usdc_challenge_bond(10_000, 1, 1, u64::MAX).unwrap(),
        1
    );
    assert_eq!(
        calculate_oracle_usdc_challenge_bond(10_001, 1, 1, u64::MAX).unwrap(),
        1
    );
    assert_eq!(
        calculate_oracle_usdc_challenge_bond(u64::MAX, 10_000, 1, u64::MAX).unwrap(),
        u64::MAX
    );
    assert!(calculate_oracle_usdc_challenge_bond(0, 1, 1, 1).is_err());
    assert!(calculate_oracle_usdc_challenge_bond(1, 10_001, 1, 1).is_err());
    assert!(calculate_oracle_usdc_challenge_bond(1, 1, 0, 1).is_err());
    assert!(calculate_oracle_usdc_challenge_bond(1, 1, 2, 1).is_err());
}

#[test]
fn differentiation_bond_uses_the_larger_local_source_backing() {
    assert_eq!(oracle_source_challenge_bond_backing(10, None), 10);
    assert_eq!(oracle_source_challenge_bond_backing(10, Some(7)), 10);
    assert_eq!(oracle_source_challenge_bond_backing(10, Some(25)), 25);
    assert_eq!(
        calculate_oracle_usdc_challenge_bond(
            oracle_source_challenge_bond_backing(10, Some(25)),
            2_000,
            1,
            100,
        )
        .unwrap(),
        5
    );
}

#[test]
fn kill_challenge_threshold_is_stable_for_backing_permutation_and_duplicates() {
    let required = |primary, comparison| {
        calculate_oracle_usdc_challenge_bond(
            oracle_source_challenge_bond_backing(primary, Some(comparison)),
            2_000,
            3,
            40,
        )
        .unwrap()
    };
    assert_eq!(required(25, 10), 5);
    assert_eq!(required(10, 25), 5);
    assert_eq!(required(25, 25), 5);

    // K_min/K_max are an ordered clamp, not an unordered pair. Reversal is rejected instead of
    // silently normalizing a malformed public input into a different threshold.
    assert!(calculate_oracle_usdc_challenge_bond(25, 2_000, 40, 3).is_err());
}

#[test]
fn ordinary_challenge_is_zero_sum_and_loser_funded() {
    assert_eq!(
        calculate_oracle_usdc_challenge_settlement(10, 4, true).unwrap(),
        OracleUsdcChallengeSettlement {
            defender_credit: 0,
            challenger_credit: 14,
        }
    );
    assert_eq!(
        calculate_oracle_usdc_challenge_settlement(10, 4, false).unwrap(),
        OracleUsdcChallengeSettlement {
            defender_credit: 14,
            challenger_credit: 0,
        }
    );
    assert!(
        calculate_oracle_usdc_challenge_settlement(u64::MAX, 1, true).is_err(),
        "the winner credit must never wrap"
    );
}
