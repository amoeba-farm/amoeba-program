use super::*;
use crate::instruction::{SettlementObservation, SignedSettlementPayload};
use borsh::BorshDeserialize;

const SETTLEMENT_SOURCE: &str = concat!(
    include_str!("../settlement.rs"),
    include_str!("../month_terminal.rs"),
    include_str!("../settlement_validation.rs"),
);

fn function_source<'a>(source: &'a str, name: &str) -> &'a str {
    let start = source
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("missing function {name}"));
    let rest = &source[start..];
    let end = rest
        .get(1..)
        .and_then(|tail| {
            ["\nfn ", "\npub fn ", "\npub(super) fn ", "\npub(crate) fn "]
                .iter()
                .filter_map(|next| tail.find(next))
                .min()
        })
        .map(|offset| offset + 1)
        .unwrap_or(rest.len());
    &rest[..end]
}

fn sample_leaf(oracle_month: Pubkey, submitter: Pubkey) -> SettlementLeaf {
    SettlementLeaf {
        schema_version: SettlementLeaf::CURRENT_SCHEMA_VERSION,
        oracle_month,
        recipe_hash: [4; 32],
        underlying_id: [5; 32],
        item_id: "ramx".to_string(),
        expiry_id: "RAMX-202612".to_string(),
        settlement_ts: 1_796_083_200,
        price_display_decimals: 2,
        computation: SettlementComputation::OracleIndexDelta,
        trailing_window_days: 0,
        observations: vec![],
        settlement_price_atomic: 10_500,
        source_uri: "https://oracle.example/ramx/RAMX-202612".to_string(),
        source_digest: [6; 32],
        base_oracle_atomic: 10_000,
        index_delta_bps: 500,
        submitted_by: submitter,
        submitted_slot: 77,
        signer_set_version: 1,
        signer_set_hash: [7; 32],
    }
}

#[test]
fn settlement_v2_identity_is_scoped_to_period_and_oracle_month() {
    let program_id = Pubkey::new_unique();
    let period_a = Pubkey::new_unique();
    let period_b = Pubkey::new_unique();
    let oracle_month_a = Pubkey::new_unique();
    let oracle_month_b = Pubkey::new_unique();

    let (record_a, _) = derive_settlement_v2_pda(&program_id, &period_a, &oracle_month_a);
    let (record_b, _) = derive_settlement_v2_pda(&program_id, &period_b, &oracle_month_b);
    let (record_c, _) = derive_settlement_v2_pda(&program_id, &period_a, &oracle_month_b);

    assert_ne!(record_a, record_b);
    assert_ne!(record_a, record_c);
}

#[test]
fn settlement_v2_rewrite_requires_identical_signed_semantics_and_provenance() {
    let period = Pubkey::new_unique();
    let oracle_month = Pubkey::new_unique();
    let submitter = Pubkey::new_unique();
    let leaf = sample_leaf(oracle_month, submitter);
    let commitment = settlement_signed_leaf_commitment(&leaf).expect("commitment");
    let record =
        settlement_record_v2_from_leaf(1, period, oracle_month, &leaf, commitment, submitter);

    assert!(settlement_record_v2_matches_submission(
        &record,
        &period,
        &oracle_month,
        &leaf,
        &commitment,
        &submitter,
    ));

    let mut conflicting = leaf.clone();
    conflicting.settlement_price_atomic += 1;
    let conflicting_commitment =
        settlement_signed_leaf_commitment(&conflicting).expect("conflicting commitment");
    assert!(!settlement_record_v2_matches_submission(
        &record,
        &period,
        &oracle_month,
        &conflicting,
        &conflicting_commitment,
        &submitter,
    ));
    assert!(!settlement_record_v2_matches_submission(
        &record,
        &Pubkey::new_unique(),
        &oracle_month,
        &leaf,
        &commitment,
        &submitter,
    ));
    assert!(!settlement_record_v2_matches_submission(
        &record,
        &period,
        &oracle_month,
        &leaf,
        &commitment,
        &Pubkey::new_unique(),
    ));
    let mut forged_timestamp = leaf.clone();
    forged_timestamp.settlement_ts += 1;
    let forged_timestamp_commitment =
        settlement_signed_leaf_commitment(&forged_timestamp).expect("forged commitment");
    assert!(!settlement_record_v2_matches_submission(
        &record,
        &period,
        &oracle_month,
        &forged_timestamp,
        &forged_timestamp_commitment,
        &submitter,
    ));
    // The submitter and slot are outside the signed payload; the commitment ignores them.
    let mut resubmitted = leaf.clone();
    resubmitted.submitted_by = Pubkey::new_unique();
    resubmitted.submitted_slot += 1;
    assert_eq!(
        settlement_signed_leaf_commitment(&resubmitted).unwrap(),
        commitment
    );
}

#[test]
fn settlement_grace_is_checked_by_upsert_finalize_and_close() {
    let upsert = function_source(SETTLEMENT_SOURCE, "process_upsert_settlement");
    let binding = upsert
        .find("validate_settlement_period_binding")
        .expect("upsert binds the settlement to the period expiry");
    let grace = upsert
        .find("ensure_settlement_finalization_ready")
        .expect("upsert enforces the settlement grace");
    let signatures = upsert
        .find("verify_settlement_oracle_attestations")
        .expect("upsert verifies the signer quorum");
    let create = upsert
        .find("load_or_create_settlement_record_v2")
        .expect("upsert creates or loads the record");
    let settled = upsert
        .find("write_period_settled")
        .expect("upsert writes the period Settled");
    assert!(binding < grace && grace < signatures && signatures < create && create < settled);

    let finalize = function_source(SETTLEMENT_SOURCE, "process_finalize_oracle_month");
    assert!(finalize.contains("ensure_settlement_finalization_ready_at"));
    let close = function_source(SETTLEMENT_SOURCE, "process_close_oracle_month");
    assert!(
        close.contains("load_valid_settlement_record_v2"),
        "close must use the grace-enforcing canonical settlement loader"
    );
}

#[test]
fn settlement_leaf_slice_codec_matches_borsh_for_all_single_byte_changes() {
    fn decode(mut data: &[u8]) -> Result<SettlementLeaf, ()> {
        let decoded = SettlementLeaf::deserialize_slice(&mut data).map_err(|_| ())?;
        if data.is_empty() {
            Ok(decoded)
        } else {
            Err(())
        }
    }
    let mut settlement = sample_leaf(
        Pubkey::new_from_array([2; 32]),
        Pubkey::new_from_array([6; 32]),
    );
    settlement.trailing_window_days = 5;
    settlement.observations = vec![
        SettlementObservation {
            observed_at_ts: 1_777_478_400,
            price_atomic: 10_500,
        },
        SettlementObservation {
            observed_at_ts: 1_777_564_800,
            price_atomic: 10_625,
        },
    ];
    let bytes = settlement.try_to_vec().unwrap();
    assert_eq!(decode(&bytes), Ok(settlement));
    for end in 0..bytes.len() {
        assert_eq!(
            decode(&bytes[..end]),
            SettlementLeaf::try_from_slice(&bytes[..end]).map_err(|_| ())
        );
    }
    for offset in 0..bytes.len() {
        for replacement in [0, 1, 0x7f, 0xff] {
            let mut changed = bytes.clone();
            changed[offset] = replacement;
            assert_eq!(
                decode(&changed),
                SettlementLeaf::try_from_slice(&changed).map_err(|_| ())
            );
        }
    }
}

#[test]
fn settlement_signing_message_is_the_reference_signed_payload() {
    let mut underlying_id = [0u8; 32];
    underlying_id[..24].copy_from_slice(b"ram-standardized-baskets");
    let settlement = SettlementLeaf {
        schema_version: SettlementLeaf::CURRENT_SCHEMA_VERSION,
        oracle_month: Pubkey::new_from_array([0x42; 32]),
        recipe_hash: [0x24; 32],
        underlying_id,
        item_id: "ramx".to_string(),
        expiry_id: "APR26".to_string(),
        settlement_ts: 1_777_564_800,
        price_display_decimals: 2,
        computation: SettlementComputation::OracleIndexDelta,
        trailing_window_days: 0,
        observations: vec![],
        settlement_price_atomic: 10_625,
        source_uri: "https://oracle.example/oracle/markets/ramx/APR26".to_string(),
        source_digest: [0x9a; 32],
        base_oracle_atomic: 10_000,
        index_delta_bps: 625,
        submitted_by: Pubkey::default(),
        submitted_slot: 0,
        signer_set_version: 1,
        signer_set_hash: [0x55; 32],
    };
    let message = settlement.signing_message().expect("message");
    assert_eq!(
        message,
        settlement
            .to_signed_payload()
            .try_to_vec()
            .expect("reference signed payload")
    );
    // Domain (4 + 26), schema, month, recipe, underlying, item (4 + 4), expiry (4 + 5), ts,
    // decimals, computation, window, observations (4), price, uri (4 + 48), digest, base, delta,
    // signer-set version and hash.
    assert_eq!(message.len(), 307);
    let payload = SignedSettlementPayload::try_from_slice(&message).expect("decodes");
    assert_eq!(
        payload.signing_domain,
        SignedSettlementPayload::SIGNING_DOMAIN
    );
    assert_eq!(payload.settlement_price_atomic, 10_625);
}

#[test]
fn index_delta_price_uses_the_base_and_the_signed_delta() {
    let mut settlement = sample_leaf(Pubkey::new_unique(), Pubkey::default());
    settlement.index_delta_bps = 625;
    assert_eq!(
        settlement.computed_index_delta_price_atomic().unwrap(),
        10_625
    );
    settlement.index_delta_bps = -625;
    assert_eq!(
        settlement.computed_index_delta_price_atomic().unwrap(),
        9_375
    );
    settlement.index_delta_bps = -10_000;
    assert_eq!(settlement.computed_index_delta_price_atomic().unwrap(), 0);
    settlement.index_delta_bps = -10_001;
    assert!(settlement.computed_index_delta_price_atomic().is_err());
    settlement.base_oracle_atomic = 0;
    assert!(settlement.computed_index_delta_price_atomic().is_err());
    // The period's value is the same function and is never zero.
    assert_eq!(index_delta_settlement_value(10_000, 625), Ok(10_625));
    assert_eq!(
        index_delta_settlement_value(100_000_000, 2_345),
        Ok(123_450_000)
    );
    assert_eq!(index_delta_settlement_value(3, -1_667), Ok(2));
    assert_eq!(index_delta_settlement_value(10_000, -10_000), Ok(0));
    assert_eq!(index_delta_settlement_value(1, -9_999), Ok(0));
    assert_eq!(index_delta_settlement_value(1, -5_000), Ok(1));
    assert!(index_delta_settlement_value(1, -10_001).is_err());
    assert!(index_delta_settlement_value(0, 0).is_err());
}

#[test]
fn failure_predicates_follow_month_creation_submission_and_expiry() {
    let expiry = 1_800_000_000u64;
    let review = ORACLE_KILL_WINDOW_SECONDS
        + ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
        + ORACLE_OPENING_WINDOW_SECONDS;
    // (a) No month: failable once a month could no longer hold the review calendar.
    assert_eq!(
        period_failure_predicate_holds(None, expiry, expiry - review - 1),
        Ok(false)
    );
    assert_eq!(
        period_failure_predicate_holds(None, expiry, expiry - review),
        Ok(true)
    );
    let month = |phase| OracleMonthState {
        scramble_start_ts: expiry - 30 * 86_400,
        listing_ts: expiry - 30 * 86_400 + ORACLE_PRE_LISTING_WINDOW_SECONDS,
        phase,
        ..OracleMonthState::default()
    };
    // (b) Source submission: failable at the submission deadline.
    let submitting = month(OraclePhase::SourceSubmission);
    let deadline = source_submission_deadline(&submitting, expiry).unwrap();
    assert_eq!(deadline, expiry - review);
    assert_eq!(
        period_failure_predicate_holds(Some(&submitting), expiry, deadline - 1),
        Ok(false)
    );
    assert_eq!(
        period_failure_predicate_holds(Some(&submitting), expiry, deadline),
        Ok(true)
    );
    // (c) Any other pre-Game phase: failable at expiry.
    for phase in [OraclePhase::Scramble, OraclePhase::Opening] {
        let forming = month(phase);
        assert_eq!(
            period_failure_predicate_holds(Some(&forming), expiry, deadline),
            Ok(false)
        );
        assert_eq!(
            period_failure_predicate_holds(Some(&forming), expiry, expiry - 1),
            Ok(false)
        );
        assert_eq!(
            period_failure_predicate_holds(Some(&forming), expiry, expiry),
            Ok(true)
        );
    }
    // A month that reached Game or later never fails its period.
    for phase in [OraclePhase::Game, OraclePhase::Settled, OraclePhase::Closed] {
        let listed = month(phase);
        assert_eq!(
            period_failure_predicate_holds(Some(&listed), expiry, u64::MAX),
            Ok(false)
        );
    }
}

/// Deterministic tag 116 wire for publisher clients: the leaf, its signed payload, the
/// attestation digest and the instruction bytes. Values are wire samples, not chain evidence.
#[test]
fn settlement_publication_wire_fixture() {
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
    let program = Pubkey::new_from_array([1; 32]);
    let registry = Pubkey::new_from_array([2; 32]);
    let signer_set_key = Pubkey::new_from_array([3; 32]);
    let signer_set = SettlementSignerSet {
        version: 7,
        set_hash: [4; 32],
        ..Default::default()
    };
    let mut underlying = [0u8; 32];
    underlying[..24].copy_from_slice(b"ram-standardized-baskets");
    let leaf = SettlementLeaf {
        schema_version: SettlementLeaf::CURRENT_SCHEMA_VERSION,
        oracle_month: Pubkey::new_from_array([5; 32]),
        recipe_hash: [6; 32],
        underlying_id: underlying,
        item_id: "ramx".into(),
        expiry_id: "RAMX-202612".into(),
        settlement_ts: 1_796_083_200,
        price_display_decimals: 6,
        computation: SettlementComputation::OracleIndexDelta,
        trailing_window_days: 0,
        observations: vec![],
        settlement_price_atomic: 106_250_000,
        source_uri: "https://example.invalid/fixture/RAMX-202612".into(),
        source_digest: [7; 32],
        base_oracle_atomic: 100_000_000,
        index_delta_bps: 625,
        submitted_by: Pubkey::new_from_array([8; 32]),
        submitted_slot: 123,
        signer_set_version: signer_set.version,
        signer_set_hash: signer_set.set_hash,
    };
    assert_eq!(
        leaf.computed_index_delta_price_atomic().unwrap(),
        leaf.settlement_price_atomic
    );
    let instruction = crate::instruction::OracleInstruction::UpsertSettlementV3 {
        params: UpsertSettlementParams {
            settlement: leaf.clone(),
        },
    };
    let leaf_bytes = borsh::to_vec(&leaf).unwrap();
    let signed = leaf.signed_payload_bytes().unwrap();
    assert_eq!(signed, borsh::to_vec(&leaf.to_signed_payload()).unwrap());
    let digest =
        settlement_attestation_digest(&program, &registry, &signer_set_key, &signer_set, &leaf)
            .unwrap();
    let wire = borsh::to_vec(&instruction).unwrap();
    assert_eq!(wire[0], 116);
    assert_eq!(&wire[1..], leaf_bytes.as_slice());
    assert_eq!(
        crate::instruction::OracleInstruction::try_from_slice(&wire).unwrap(),
        instruction
    );
    let corpus = serde_json::json!({
        "kind": "deterministic-wire-sample-not-settlement-evidence",
        "program": program.to_string(),
        "registry": registry.to_string(),
        "signerSet": signer_set_key.to_string(),
        "inputs": {
            "schemaVersion": leaf.schema_version,
            "oracleMonth": leaf.oracle_month.to_string(),
            "recipeHash": hex(&leaf.recipe_hash),
            "underlyingId": hex(&leaf.underlying_id),
            "itemId": leaf.item_id,
            "expiryId": leaf.expiry_id,
            "settlementTs": leaf.settlement_ts.to_string(),
            "priceDisplayDecimals": leaf.price_display_decimals,
            "computation": 1,
            "trailingWindowDays": 0,
            "observations": [],
            "settlementPriceAtomic": leaf.settlement_price_atomic.to_string(),
            "sourceUri": leaf.source_uri,
            "sourceDigest": hex(&leaf.source_digest),
            "baseOracleAtomic": leaf.base_oracle_atomic.to_string(),
            "indexDeltaBps": leaf.index_delta_bps.to_string(),
            "submittedBy": leaf.submitted_by.to_string(),
            "submittedSlot": leaf.submitted_slot.to_string(),
            "signerSetVersion": signer_set.version.to_string(),
            "signerSetHash": hex(&signer_set.set_hash)
        },
        "expected": {
            "leafHex": hex(&leaf_bytes),
            "signedPayloadHex": hex(&signed),
            "attestationDigestHex": hex(&digest),
            "tag116Hex": hex(&wire)
        }
    });
    if let Ok(path) = std::env::var("AMEBA_SETTLEMENT_WIRE_FIXTURE_OUT") {
        std::fs::write(path, serde_json::to_string_pretty(&corpus).unwrap() + "\n").unwrap();
    }
}
