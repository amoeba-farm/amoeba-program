use super::*;

#[test]
fn unverified_weight_scheme_cannot_open_update_or_settle() {
    let month = OracleMonthState {
        recipe_hash: [7; 32],
        source_count: 1,
        frozen_source_count: 1,
        opened_source_count: 1,
        opening_resolved_source_count: 1,
        settlement_status: OracleSettlementStatus::Final,
        ..OracleMonthState::default()
    };
    let expected = Err(ProgramError::Custom(
        OracleError::OracleWeightSchemeUnverified as u32,
    ));
    assert_eq!(ensure_oracle_opening_resolution_complete(&month), expected);
    assert_eq!(ensure_oracle_month_ready_for_settlement(&month), expected);
}

#[test]
fn oracle_opening_resolution_allows_inactive_sources_but_counts_only_accepted_openings() {
    let month = OracleMonthState {
        recipe_hash: [7; 32],
        weight_scheme_version: 1,
        effective_weight_total_bps: 10_000,
        weight_manifest_hash: [8; 32],
        source_count: 3,
        frozen_source_count: 3,
        opened_source_count: 2,
        opening_resolved_source_count: 3,
        pending_resolution_count: 0,
        active_weight_scheme_version: OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION,
        active_weight_group_count: 1,
        active_weight_manifest_hash: [9; 32],
        ..OracleMonthState::default()
    };

    ensure_oracle_opening_resolution_complete(&month)
        .expect("two accepted sources plus one inactive source should be terminal");
    assert_eq!(month.opened_source_count, 2);
}

#[test]
fn oracle_opening_challenge_rejects_only_a_true_noop() {
    let archive_url = "https://web.archive.org/web/20260701000000/https://example.com/source/3";
    let claim = OracleOpeningClaim {
        opening_state: 100_000_000,
        source_time: 1_788_220_000,
        canonical_locator_hash: [1; 32],
        source_definition_hash: [2; 32],
        evidence_hash: [3; 32],
        archive_url_hash: derive_oracle_opening_archive_url_hash(archive_url),
        ..OracleOpeningClaim::default()
    };
    let identical = ChallengeOracleOpeningClaimParams {
        challenge_id: [4; 32],
        alternative_opening_state: claim.opening_state,
        alternative_source_time: claim.source_time,
        bond: 100,
        canonical_locator_hash: claim.canonical_locator_hash,
        source_definition_hash: claim.source_definition_hash,
        archive_url: archive_url.to_string(),
    };
    assert!(oracle_opening_challenge_is_true_noop(&claim, &identical));

    let evidence_only = ChallengeOracleOpeningClaimParams {
        archive_url: "https://web.archive.org/web/20260701000001/https://example.com/source/3"
            .to_string(),
        ..identical.clone()
    };
    assert!(!oracle_opening_challenge_is_true_noop(
        &claim,
        &evidence_only
    ));

    let time_only = ChallengeOracleOpeningClaimParams {
        alternative_source_time: claim.source_time + 1,
        ..identical
    };
    assert!(!oracle_opening_challenge_is_true_noop(&claim, &time_only));
}

#[test]
fn oracle_opening_evidence_hash_matches_cross_language_vector() {
    let canonical_locator_hash = [
        0x03, 0x4e, 0xd8, 0x0a, 0xf5, 0x6b, 0x51, 0x9d, 0xa3, 0x5d, 0xd7, 0x89, 0xef, 0x20, 0xca,
        0x25, 0xa0, 0x52, 0x96, 0xb7, 0x6d, 0xb9, 0xa6, 0xfd, 0xcc, 0xfc, 0xbb, 0xc0, 0x2e, 0x03,
        0x72, 0x06,
    ];
    let actual = derive_oracle_opening_evidence_hash(
        &Pubkey::new_from_array([0x0a; 32]),
        &Pubkey::new_from_array([0x0b; 32]),
        100_000_000,
        1_788_220_000,
        &canonical_locator_hash,
        &[0x02; 32],
        "https://web.archive.org/web/20260831234640/https://example.com/source/3",
    );
    assert_eq!(
        actual,
        [
            0x53, 0xcd, 0x8a, 0x6a, 0x7d, 0xd9, 0xfa, 0x9a, 0xe3, 0x07, 0x0a, 0x7f, 0x46, 0xbb,
            0xc7, 0xe0, 0xcb, 0x75, 0x69, 0xcb, 0x21, 0x52, 0x5f, 0x14, 0x95, 0x90, 0x03, 0xcb,
            0x92, 0x64, 0xac, 0x56,
        ]
    );
    assert_eq!(
        parse_oracle_opening_archive_url(
            "https://web.archive.org/web/20260831234640/https://example.com/source/3"
        )
        .expect("parse cross-language Wayback URL")
        .0,
        1_788_220_000
    );
}

#[test]
fn oracle_opening_archive_url_is_bounded_wayback_only_and_delimiter_safe() {
    let valid = "https://web.archive.org/web/20260701000000/https://example.com/source/3";
    parse_oracle_opening_archive_url(valid).expect("valid Wayback URL");

    for invalid in [
        "".to_string(),
        ORACLE_OPENING_ARCHIVE_URL_PREFIX.to_string(),
        "https://example.com/source/3".to_string(),
        format!("{}{}", ORACLE_OPENING_ARCHIVE_URL_PREFIX, "x".repeat(385)),
        format!(
            "{}snapshot\nhttps://evil.example",
            ORACLE_OPENING_ARCHIVE_URL_PREFIX
        ),
        format!("{}snapshot\u{0007}", ORACLE_OPENING_ARCHIVE_URL_PREFIX),
        "https://web.archive.org/web/20261301000000/https://example.com/source/3".to_string(),
        "https://web.archive.org/web/20260230000000/https://example.com/source/3".to_string(),
        "https://web.archive.org/web/2026070100000/https://example.com/source/3".to_string(),
        "https://web.archive.org/web/20260701000000id_/https://example.com/source/3".to_string(),
        "https://web.archive.org/web/20260701000000/example.com/source/3".to_string(),
        "https://web.archive.org/web/20260701000000/https:///source/3".to_string(),
        "https://web.archive.org/web/20260701000000/http://".to_string(),
        "https://web.archive.org/web/20260701000000/https://:443/source/3".to_string(),
    ] {
        assert_eq!(
            parse_oracle_opening_archive_url(&invalid).map(|_| ()),
            Err(ProgramError::Custom(
                OracleError::InvalidOracleOpeningEvidence as u32
            )),
            "unexpectedly accepted {invalid:?}"
        );
    }
}

#[test]
fn oracle_opening_evidence_time_never_exceeds_period_expiry() {
    let expiry_ts = 1_788_220_800;
    let capture_start = expiry_ts - ORACLE_OPENING_WINDOW_SECONDS;
    let anchor = sample_anchor(expiry_ts);
    let original_url = "https://example.com/source/3";
    let canonical_locator_hash = hashv(&[b"locator", original_url.as_bytes()]).to_bytes();
    let source = OracleSourceState {
        canonical_locator_hash,
        source_definition_hash: [2; 32],
        ..OracleSourceState::default()
    };
    let archive_url = "https://web.archive.org/web/20260901000000/https://example.com/source/3";

    validate_oracle_opening_evidence_fields_at_time(
        &anchor,
        capture_start,
        &source,
        100_000_000,
        expiry_ts,
        &canonical_locator_hash,
        &[2; 32],
        archive_url,
        expiry_ts + 100,
    )
    .expect("an observation at the period-expiry bound remains structurally valid");
    assert_eq!(
        validate_oracle_opening_evidence_fields_at_time(
            &anchor,
            capture_start,
            &source,
            100_000_000,
            expiry_ts + 1,
            &canonical_locator_hash,
            &[2; 32],
            "https://web.archive.org/web/20260901000001/https://example.com/source/3",
            expiry_ts + 100,
        ),
        Err(ProgramError::Custom(
            OracleError::InvalidOracleOpeningEvidence as u32
        ))
    );
    for invalid_archive_url in [
        "https://web.archive.org/web/20260831235959/https://example.com/source/3",
        "https://web.archive.org/web/20260901000000/https://example.com/wrong-page",
    ] {
        assert_eq!(
            validate_oracle_opening_evidence_fields_at_time(
                &anchor,
                capture_start,
                &source,
                100_000_000,
                expiry_ts,
                &canonical_locator_hash,
                &[2; 32],
                invalid_archive_url,
                expiry_ts + 100,
            ),
            Err(ProgramError::Custom(
                OracleError::InvalidOracleOpeningEvidence as u32
            ))
        );
    }
}

const RECORD_DIGEST: &str = "abababababababababababababababababababababababababababababababab";

fn evidence_record(time: u64, locator: &str) -> String {
    format!("{ORACLE_EVIDENCE_RECORD_PREFIX}m=web;t={time};d={RECORD_DIGEST};u={locator}")
}

/// Cross-check with the off-chain reference parser, `amoeba_evidence::parse_onchain_record`
/// (`evidence/crates/evidence/src/lib.rs`): its own test vectors (`onchain_record_round_trip`)
/// first, then hard-coded strings with the result both sides give. The program adds the
/// oracle's locator rules on `u`, the archive URL's byte rules and a 512-byte bound, and refuses
/// `t=0`, whose form the reference accepts (no claim's source time is zero).
#[test]
fn evidence_record_parser_matches_the_off_chain_reference_vectors() {
    let text =
        format!("amoeba-evidence-v1;m=web;t=1760900000;d={RECORD_DIGEST};u=https://a.com/x?y=1");
    assert_eq!(
        parse_oracle_evidence_record(&text),
        Ok((1_760_900_000, "https://a.com/x?y=1"))
    );
    assert!(parse_oracle_evidence_record(&text.replace("t=1", "t=01")).is_err());
    assert!(parse_oracle_evidence_record(&text.replace("m=web", "m=wayback")).is_err());

    let d = RECORD_DIGEST;
    let p = ORACLE_EVIDENCE_RECORD_PREFIX;
    let accepted: [(String, u64, &str); 6] = [
        (
            format!("{p}m=email;t=1760900000;d={d};u=https://a.com/x"),
            1_760_900_000,
            "https://a.com/x",
        ),
        (
            format!("{p}m=web;t=1;d={d};u=http://a.com/x"),
            1,
            "http://a.com/x",
        ),
        (
            format!("{p}m=web;t=18446744073709551615;d={d};u=https://a.com/x"),
            u64::MAX,
            "https://a.com/x",
        ),
        // `u` is the rest of the text, delimiters included.
        (
            format!("{p}m=web;t=5;d={d};u=https://a.com/x;u=https://b.com/y"),
            5,
            "https://a.com/x;u=https://b.com/y",
        ),
        (
            format!(
                "{p}m=web;t=5;d={};u=https://a.com/x",
                "0123456789abcdef".repeat(4)
            ),
            5,
            "https://a.com/x",
        ),
        (
            format!("{p}m=web;t=5;d={d};u=https://a.com/caf\u{e9}"),
            5,
            "https://a.com/caf\u{e9}",
        ),
    ];
    for (text, time, locator) in &accepted {
        assert_eq!(
            parse_oracle_evidence_record(text),
            Ok((*time, *locator)),
            "{text}"
        );
    }
    let invalid = Err(ProgramError::Custom(
        OracleError::InvalidOracleOpeningEvidence as u32,
    ));
    for text in [
        // The one addition to the reference: t=0.
        format!("{p}m=web;t=0;d={d};u=https://a.com/x"),
        format!("{p}m=web;t=00;d={d};u=https://a.com/x"),
        format!("{p}m=web;t=;d={d};u=https://a.com/x"),
        format!("{p}m=web;t=18446744073709551616;d={d};u=https://a.com/x"),
        format!("{p}m=web;t=100000000000000000000;d={d};u=https://a.com/x"),
        format!("{p}m=web;t=+1;d={d};u=https://a.com/x"),
        format!("{p}m=web;t=1;d={};u=https://a.com/x", d.to_uppercase()),
        format!("{p}m=web;t=1;d={};u=https://a.com/x", &d[1..]),
        format!("{p}m=web;t=1;d={d}a;u=https://a.com/x"),
        format!(
            "{p}m=web;t=1;d={};u=https://a.com/x",
            d.replacen('a', "g", 1)
        ),
        format!("{p}m=web;t=1;d={d}"),
        format!("{p}m=;t=1;d={d};u=https://a.com/x"),
        format!("{p}m=WEB;t=1;d={d};u=https://a.com/x"),
        format!("{p}t=1;d={d};u=https://a.com/x"),
        format!("amoeba-evidence-v2;m=web;t=1;d={d};u=https://a.com/x"),
        format!("{p}m=web;t=1;d={d};u="),
        format!("{p}m=web;t=1;d={d};u=ftp://a.com/x"),
        format!("{p}m=web;t=1;d={d};u=https://a.com/x?token=1"),
        format!("{p}m=web;t=1;d={d};u=https://a.com/x?Api_Key=1"),
        format!("{p}m=web;t=1;d={d};u=https://u@a.com/x"),
        format!("{p}m=web;t=1;d={d};u=https://a.com/a b"),
        format!("{p}m=web;t=1;d={d};u=https://a.com/\t"),
        format!("{p}m=web;t=1;d={d};u=https://a.com/\u{7f}"),
        format!("{p}m=web;t=1;d={d};u=https://a.com/{}", "a".repeat(400)),
        format!("{p}m=web;t=1;d={d};u=https://a.com/{}", "a".repeat(500)),
        "https://web.archive.org/web/20260701000000/https://a.com/x".to_string(),
    ] {
        assert_eq!(parse_oracle_evidence_record(&text), invalid, "{text}");
    }
}

#[test]
fn evidence_record_binds_time_and_locator_like_a_wayback_capture() {
    let expiry_ts = 1_788_220_800;
    let capture_start = expiry_ts - ORACLE_OPENING_WINDOW_SECONDS;
    let anchor = sample_anchor(expiry_ts);
    let original_url = "https://example.com/source/3";
    let canonical_locator_hash = hashv(&[b"locator", original_url.as_bytes()]).to_bytes();
    let source = OracleSourceState {
        canonical_locator_hash,
        source_definition_hash: [2; 32],
        ..OracleSourceState::default()
    };
    let check = |source_time: u64, evidence: &str| {
        validate_oracle_opening_evidence_fields_at_time(
            &anchor,
            capture_start,
            &source,
            100_000_000,
            source_time,
            &canonical_locator_hash,
            &[2; 32],
            evidence,
            expiry_ts + 100,
        )
    };
    let invalid = Err(ProgramError::Custom(
        OracleError::InvalidOracleOpeningEvidence as u32,
    ));
    // A record and a Wayback capture of the same page at the same second bind alike.
    let record = evidence_record(expiry_ts, original_url);
    check(expiry_ts, &record).expect("a record at the source time on the registered locator");
    check(
        expiry_ts,
        "https://web.archive.org/web/20260901000000/https://example.com/source/3",
    )
    .expect("the Wayback form keeps binding");
    assert_eq!(
        parse_oracle_archive_evidence(&record),
        Ok((expiry_ts, original_url))
    );
    // The record's time must be the claim's source time.
    assert_eq!(check(expiry_ts - 1, &record), invalid);
    assert_eq!(
        check(expiry_ts, &evidence_record(expiry_ts - 1, original_url)),
        invalid
    );
    // The record's locator must be the source's registered one, byte for byte.
    for other in [
        "https://example.com/source/4",
        "https://example.com/source/3/",
        "http://example.com/source/3",
        "https://EXAMPLE.com/source/3",
    ] {
        assert_eq!(
            check(expiry_ts, &evidence_record(expiry_ts, other)),
            invalid
        );
    }
    // A credential-looking locator is refused even when it is the registered one.
    let secret = "https://example.com/source/3?token=1";
    assert_eq!(
        validate_oracle_opening_archive_binding(
            &evidence_record(expiry_ts, secret),
            expiry_ts,
            &hashv(&[b"locator", secret.as_bytes()]).to_bytes(),
        ),
        invalid
    );
    // Malformed records never bind.
    for malformed in [
        record.replace(RECORD_DIGEST, &RECORD_DIGEST.to_uppercase()),
        record.replace("m=web", "m=wayback"),
        record.replace(";t=", ";t=0"),
    ] {
        assert_eq!(check(expiry_ts, &malformed), invalid, "{malformed}");
    }
}

#[test]
fn oracle_opening_challenge_with_a_record_for_the_same_capture_is_not_a_noop() {
    let wayback = "https://web.archive.org/web/20260701000000/https://example.com/source/3";
    let record = evidence_record(1_782_864_000, "https://example.com/source/3");
    let claim = OracleOpeningClaim {
        opening_state: 100_000_000,
        source_time: 1_782_864_000,
        canonical_locator_hash: [1; 32],
        source_definition_hash: [2; 32],
        evidence_hash: [3; 32],
        archive_url_hash: derive_oracle_opening_archive_url_hash(wayback),
        ..OracleOpeningClaim::default()
    };
    let params = ChallengeOracleOpeningClaimParams {
        challenge_id: [4; 32],
        alternative_opening_state: claim.opening_state,
        alternative_source_time: claim.source_time,
        bond: 100,
        canonical_locator_hash: claim.canonical_locator_hash,
        source_definition_hash: claim.source_definition_hash,
        archive_url: record.clone(),
    };
    assert!(!oracle_opening_challenge_is_true_noop(&claim, &params));
    let recorded = OracleOpeningClaim {
        archive_url_hash: derive_oracle_opening_archive_url_hash(&record),
        ..claim
    };
    assert!(oracle_opening_challenge_is_true_noop(&recorded, &params));
}

#[test]
fn oracle_opening_challenge_pda_validation_rejects_noncanonical_accounts() {
    let program_id = Pubkey::new_unique();
    let month = Pubkey::new_unique();
    let claim = Pubkey::new_unique();
    let challenge_id = [9; 32];
    let (canonical, _) =
        derive_oracle_opening_claim_challenge_pda(&program_id, &month, &claim, &challenge_id);
    validate_oracle_opening_claim_challenge_pda(
        &program_id,
        &canonical,
        &month,
        &claim,
        &challenge_id,
    )
    .expect("canonical opening challenge PDA");
    assert_eq!(
        validate_oracle_opening_claim_challenge_pda(
            &program_id,
            &Pubkey::new_unique(),
            &month,
            &claim,
            &challenge_id,
        ),
        Err(ProgramError::Custom(
            OracleError::InvalidOracleChallengeAccount as u32
        ))
    );
}

#[test]
fn canonical_absence_proof_accepts_prefunding_and_rejects_wrong_owner_key_or_data() {
    let expected = Pubkey::new_unique();
    let system_owner = system_program::id();
    let mut prefunded_lamports = 42;
    let mut empty_data = [];
    let prefunded_system_zero = AccountInfo::new(
        &expected,
        false,
        false,
        &mut prefunded_lamports,
        &mut empty_data,
        &system_owner,
        false,
        0,
    );
    validate_canonical_system_zero_pda_proof(&expected, &prefunded_system_zero)
        .expect("prefunded system-owned zero-data canonical PDA is a valid absence proof");

    let wrong_key = Pubkey::new_unique();
    let mut wrong_key_lamports = 0;
    let mut wrong_key_data = [];
    let wrong_key_info = AccountInfo::new(
        &wrong_key,
        false,
        false,
        &mut wrong_key_lamports,
        &mut wrong_key_data,
        &system_owner,
        false,
        0,
    );
    assert!(validate_canonical_system_zero_pda_proof(&expected, &wrong_key_info).is_err());

    let foreign_owner = Pubkey::new_unique();
    let mut foreign_lamports = 1;
    let mut foreign_data = [];
    let foreign_info = AccountInfo::new(
        &expected,
        false,
        false,
        &mut foreign_lamports,
        &mut foreign_data,
        &foreign_owner,
        false,
        0,
    );
    assert!(validate_canonical_system_zero_pda_proof(&expected, &foreign_info).is_err());

    let mut data_lamports = 1;
    let mut nonempty_data = [1];
    let data_info = AccountInfo::new(
        &expected,
        false,
        false,
        &mut data_lamports,
        &mut nonempty_data,
        &system_owner,
        false,
        0,
    );
    assert!(validate_canonical_system_zero_pda_proof(&expected, &data_info).is_err());
}

#[test]
fn oracle_source_challenge_pending_count_tracks_terminal_outcomes_once() {
    for outcome in [
        OracleSourceChallengeOutcome::KeepSource,
        OracleSourceChallengeOutcome::RejectSource,
        OracleSourceChallengeOutcome::MergeSource,
    ] {
        let mut month = OracleMonthState {
            pending_resolution_count: 1,
            ..OracleMonthState::default()
        };
        decrement_pending_oracle_source_challenge_if_terminal(&mut month, outcome)
            .expect("terminal source challenge should decrement once");
        assert_eq!(month.pending_resolution_count, 0);
        assert_eq!(
            decrement_pending_oracle_source_challenge_if_terminal(&mut month, outcome),
            Err(ProgramError::Custom(OracleError::InvalidOracleState as u32))
        );
    }

    let mut unresolved = OracleMonthState {
        pending_resolution_count: 1,
        ..OracleMonthState::default()
    };
    decrement_pending_oracle_source_challenge_if_terminal(
        &mut unresolved,
        OracleSourceChallengeOutcome::RuleReviewUnresolved,
    )
    .expect("unresolved source challenge must remain pending");
    assert_eq!(unresolved.pending_resolution_count, 1);
}

#[test]
fn oracle_opening_resolution_rejects_missing_or_incomplete_recipe() {
    let missing_recipe = OracleMonthState {
        weight_scheme_version: 1,
        effective_weight_total_bps: 10_000,
        weight_manifest_hash: [8; 32],
        source_count: 1,
        frozen_source_count: 1,
        opened_source_count: 1,
        opening_resolved_source_count: 1,
        ..OracleMonthState::default()
    };
    assert_eq!(
        ensure_oracle_opening_resolution_complete(&missing_recipe),
        Err(ProgramError::Custom(OracleError::InvalidOracleState as u32))
    );

    let incomplete_opening = OracleMonthState {
        recipe_hash: [7; 32],
        weight_scheme_version: 1,
        effective_weight_total_bps: 10_000,
        weight_manifest_hash: [8; 32],
        source_count: 2,
        frozen_source_count: 2,
        opened_source_count: 1,
        opening_resolved_source_count: 1,
        ..OracleMonthState::default()
    };
    assert_eq!(
        ensure_oracle_opening_resolution_complete(&incomplete_opening),
        Err(ProgramError::Custom(OracleError::InvalidOracleState as u32))
    );

    let mismatched_frozen_count = OracleMonthState {
        recipe_hash: [7; 32],
        weight_scheme_version: 1,
        effective_weight_total_bps: 10_000,
        weight_manifest_hash: [8; 32],
        source_count: 2,
        frozen_source_count: 1,
        opened_source_count: 2,
        opening_resolved_source_count: 2,
        ..OracleMonthState::default()
    };
    assert_eq!(
        ensure_oracle_opening_resolution_complete(&mismatched_frozen_count),
        Err(ProgramError::Custom(OracleError::InvalidOracleState as u32))
    );
}

#[test]
fn oracle_month_ready_for_settlement_requires_final_status() {
    let ready = OracleMonthState {
        recipe_hash: [7; 32],
        weight_scheme_version: 1,
        effective_weight_total_bps: 10_000,
        weight_manifest_hash: [8; 32],
        source_count: 1,
        frozen_source_count: 1,
        opened_source_count: 1,
        opening_resolved_source_count: 1,
        settlement_status: OracleSettlementStatus::Final,
        active_weight_scheme_version: OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION,
        active_weight_group_count: 1,
        active_weight_manifest_hash: [9; 32],
        ..OracleMonthState::default()
    };
    ensure_oracle_month_ready_for_settlement(&ready)
        .expect("final opened month should be settlement-ready");

    let provisional = OracleMonthState {
        settlement_status: OracleSettlementStatus::Provisional,
        ..ready.clone()
    };
    assert_eq!(
        ensure_oracle_month_ready_for_settlement(&provisional),
        Err(ProgramError::Custom(OracleError::InvalidOracleState as u32))
    );

    let frozen_pending = OracleMonthState {
        settlement_status: OracleSettlementStatus::FrozenPendingEvidence,
        ..ready
    };
    assert_eq!(
        ensure_oracle_month_ready_for_settlement(&frozen_pending),
        Err(ProgramError::Custom(OracleError::InvalidOracleState as u32))
    );
}

#[test]
fn oracle_month_ready_to_close_requires_settled_matching_record() {
    let settlement_record = Pubkey::new_unique();
    let ready = OracleMonthState {
        recipe_hash: [7; 32],
        weight_scheme_version: 1,
        effective_weight_total_bps: 10_000,
        weight_manifest_hash: [8; 32],
        phase: OraclePhase::Settled,
        source_count: 1,
        frozen_source_count: 1,
        opened_source_count: 1,
        opening_resolved_source_count: 1,
        settlement_record: Some(settlement_record),
        active_weight_scheme_version: OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION,
        active_weight_group_count: 1,
        active_weight_manifest_hash: [9; 32],
        ..OracleMonthState::default()
    };
    ensure_oracle_month_ready_to_close(&ready, &settlement_record)
        .expect("settled month with matching record should close");

    let incomplete = OracleMonthState {
        opening_resolved_source_count: 0,
        ..ready.clone()
    };
    assert_eq!(
        ensure_oracle_month_ready_to_close(&incomplete, &settlement_record),
        Err(ProgramError::Custom(OracleError::InvalidOracleState as u32))
    );

    let unsettled = OracleMonthState {
        phase: OraclePhase::Game,
        ..ready.clone()
    };
    assert_eq!(
        ensure_oracle_month_ready_to_close(&unsettled, &settlement_record),
        Err(ProgramError::Custom(OracleError::InvalidOraclePhase as u32))
    );

    let mismatched_record = Pubkey::new_unique();
    assert_eq!(
        ensure_oracle_month_ready_to_close(&ready, &mismatched_record),
        Err(ProgramError::Custom(OracleError::InvalidOracleState as u32))
    );
}
