use super::*;

#[test]
fn collective_activation_coverage_gate_accepts_only_complete_current_recipe() {
    let month = OracleMonthState {
        scramble_start_ts: 100,
        listing_ts: 100 + ORACLE_PRE_LISTING_WINDOW_SECONDS,
        ..OracleMonthState::default()
    };
    let coverage = OracleSkuCoverageManifest {
        required_sku_count: 2,
        covered_sku_count: 2,
        coverage_finalized: true,
        coverage_complete_ts: 101,
        planned_scramble_start_ts: month.scramble_start_ts,
        planned_listing_ts: month.listing_ts,
        ..OracleSkuCoverageManifest::default()
    };
    let active_manifest = OracleActiveWeightManifest {
        expected_group_count: 2,
        ..OracleActiveWeightManifest::default()
    };
    ensure_finalized_oracle_issue_sku_coverage(&month, &coverage, &active_manifest)
        .expect("complete fresh V5 coverage must pass");

    for invalid_coverage in [
        OracleSkuCoverageManifest {
            coverage_finalized: false,
            ..coverage.clone()
        },
        OracleSkuCoverageManifest {
            coverage_complete_ts: 0,
            ..coverage.clone()
        },
        OracleSkuCoverageManifest {
            covered_sku_count: 1,
            ..coverage.clone()
        },
        OracleSkuCoverageManifest {
            planned_listing_ts: coverage.planned_listing_ts + 1,
            ..coverage.clone()
        },
        OracleSkuCoverageManifest {
            coverage_complete_ts: month.listing_ts,
            ..coverage.clone()
        },
    ] {
        assert!(ensure_finalized_oracle_issue_sku_coverage(
            &month,
            &invalid_coverage,
            &active_manifest
        )
        .is_err());
    }

    let invalid_active = OracleActiveWeightManifest {
        expected_group_count: 1,
        ..active_manifest
    };
    assert!(
        ensure_finalized_oracle_issue_sku_coverage(&month, &coverage, &invalid_active).is_err()
    );
}

#[test]
fn terminal_sku_membership_is_index_bound_and_exact_depth() {
    let sku_ids = [[1u8; 32], [2u8; 32], [3u8; 32]];
    let leaf = |index: u16, sku_id: &[u8; 32]| {
        hashv(&[ORACLE_SKU_LEAF_HASH_DOMAIN, &index.to_le_bytes(), sku_id]).to_bytes()
    };
    let leaves = [
        leaf(0, &sku_ids[0]),
        leaf(1, &sku_ids[1]),
        leaf(2, &sku_ids[2]),
        hashv(&[
            crate::constants::ORACLE_SKU_EMPTY_HASH_DOMAIN,
            &3u16.to_le_bytes(),
        ])
        .to_bytes(),
    ];
    let left = hashv(&[ORACLE_SKU_NODE_HASH_DOMAIN, &leaves[0], &leaves[1]]).to_bytes();
    let right = hashv(&[ORACLE_SKU_NODE_HASH_DOMAIN, &leaves[2], &leaves[3]]).to_bytes();
    let root = hashv(&[ORACLE_SKU_NODE_HASH_DOMAIN, &left, &right]).to_bytes();
    assert_eq!(oracle_sku_merkle_root(&sku_ids), Ok(root));
    let coverage = OracleSkuCoverageManifest {
        required_sku_root: root,
        required_sku_count: 3,
        ..OracleSkuCoverageManifest::default()
    };

    assert_eq!(
        verify_oracle_sku_membership(&coverage, &sku_ids[0], 0, &[leaves[1], right]),
        Ok(())
    );
    assert_eq!(
        verify_oracle_sku_membership(&coverage, &sku_ids[1], 1, &[leaves[0], right]),
        Ok(())
    );
    assert_eq!(
        verify_oracle_sku_membership(&coverage, &sku_ids[2], 2, &[leaves[3], left]),
        Ok(())
    );
    assert_eq!(
        verify_oracle_sku_membership(&coverage, &sku_ids[0], 1, &[leaves[1], right]),
        Err(ProgramError::Custom(
            OracleError::InvalidOracleSkuMembershipProof as u32
        ))
    );
    assert_eq!(
        verify_oracle_sku_membership(&coverage, &sku_ids[0], 0, &[leaves[1]]),
        Err(ProgramError::Custom(
            OracleError::InvalidOracleSkuMembershipProof as u32
        ))
    );
}

#[test]
fn exact_coverage_keeps_on_time_calendar_and_late_coverage_preserves_review() {
    let planned_scramble = 1_000_000u64;
    let planned_listing = planned_scramble + ORACLE_PRE_LISTING_WINDOW_SECONDS;
    let expiry = planned_listing + 90 * crate::constants::ORACLE_CALENDAR_DAY_SECONDS;
    let coverage = OracleSkuCoverageManifest {
        planned_scramble_start_ts: planned_scramble,
        planned_listing_ts: planned_listing,
        ..OracleSkuCoverageManifest::default()
    };
    let planned_placement_end = planned_scramble + ORACLE_PLACEMENT_WINDOW_SECONDS;

    assert_eq!(
        finalized_oracle_sku_coverage_schedule(&coverage, expiry, planned_placement_end),
        Ok((planned_scramble, planned_listing))
    );

    let late_completion = planned_placement_end + 123;
    let (shifted_scramble, shifted_listing) =
        finalized_oracle_sku_coverage_schedule(&coverage, expiry, late_completion).unwrap();
    assert_eq!(
        shifted_scramble + ORACLE_PLACEMENT_WINDOW_SECONDS,
        late_completion
    );
    assert_eq!(
        shifted_listing - late_completion,
        ORACLE_KILL_WINDOW_SECONDS
            + ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
            + ORACLE_OPENING_WINDOW_SECONDS
    );
    let shifted_month = OracleMonthState {
        scramble_start_ts: shifted_scramble,
        listing_ts: shifted_listing,
        ..OracleMonthState::default()
    };
    assert!(rulebook_schedule_boundaries(&shifted_month).is_ok());

    let latest_unsafe_completion = expiry
        - ORACLE_KILL_WINDOW_SECONDS
        - ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
        - ORACLE_OPENING_WINDOW_SECONDS;
    assert_eq!(
        finalized_oracle_sku_coverage_schedule(&coverage, expiry, latest_unsafe_completion),
        Err(ProgramError::Custom(
            OracleError::OracleSkuCoverageExtensionTooLate as u32
        ))
    );
}

#[test]
fn removing_the_last_supported_source_invalidates_finalized_coverage() {
    let mut coverage = OracleSkuCoverageManifest {
        required_sku_count: 3,
        covered_sku_count: 2,
        coverage_finalized: true,
        coverage_complete_ts: 1_234,
        ..OracleSkuCoverageManifest::default()
    };
    let mut record = OracleSkuCoverageRecord {
        active_supported_source_count: 2,
        ..OracleSkuCoverageRecord::default()
    };

    remove_supported_source_from_sku_coverage(&mut coverage, &mut record).unwrap();
    assert_eq!(record.active_supported_source_count, 1);
    assert_eq!(coverage.covered_sku_count, 2);
    assert!(coverage.coverage_finalized);

    remove_supported_source_from_sku_coverage(&mut coverage, &mut record).unwrap();
    assert_eq!(record.active_supported_source_count, 0);
    assert_eq!(coverage.covered_sku_count, 1);
    assert!(!coverage.coverage_finalized);
    assert_eq!(coverage.coverage_complete_ts, 0);
    assert_eq!(
        remove_supported_source_from_sku_coverage(&mut coverage, &mut record),
        Err(ProgramError::Custom(
            OracleError::InvalidOracleSkuCoverageRecord as u32
        ))
    );
}

#[test]
fn months_are_cash_only() {
    let source_lifecycle = include_str!("../usdc/sources.rs");
    assert!(source_lifecycle.contains("load_canonical_user_collateral"));
    assert!(source_lifecycle.contains("debit_oracle_usdc_available"));
    assert!(!source_lifecycle.contains("load_oracle_major_token"));
}
