//! BATX (`governance/batx_basket_spec.v1.json`, the battery-cell basket brief of 2026-10-20):
//! its SKU manifest's root is what tag 190 computes, and the brief's acceptance tests (§5) hold
//! through the program's own index arithmetic, with where it differs from the brief's
//! fixed-base reference written down as tests.

use ameba_oracle::constants::{
    MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS, ORACLE_SKU_EMPTY_HASH_DOMAIN, ORACLE_SKU_LEAF_HASH_DOMAIN,
    ORACLE_SKU_NODE_HASH_DOMAIN,
};
use ameba_oracle::live_index::game_delta_bps;
use ameba_oracle::state::{OracleMonthState, OraclePhase, OracleSourceState, OracleSourceStatus};
use serde_json::Value;
use solana_program::{hash::hashv, pubkey::Pubkey};

const MANIFEST: &str = include_str!("../../../governance/batx_product_sku_manifest.v1.json");
const SPEC: &str = include_str!("../../../governance/batx_basket_spec.v1.json");

fn encode(label: &str) -> [u8; 32] {
    assert!(!label.is_empty() && label.len() <= 32);
    assert!(label.bytes().all(|byte| (0x20..=0x7e).contains(&byte)));
    let mut id = [0u8; 32];
    id[..label.len()].copy_from_slice(label.as_bytes());
    id
}

fn root(ids: &[[u8; 32]]) -> [u8; 32] {
    let width = ids.len().next_power_of_two();
    let mut nodes: Vec<[u8; 32]> = (0..width)
        .map(|index| {
            let index_bytes = (index as u16).to_le_bytes();
            match ids.get(index) {
                Some(id) => hashv(&[ORACLE_SKU_LEAF_HASH_DOMAIN, &index_bytes, id]).to_bytes(),
                None => hashv(&[ORACLE_SKU_EMPTY_HASH_DOMAIN, &index_bytes]).to_bytes(),
            }
        })
        .collect();
    while nodes.len() > 1 {
        nodes = nodes
            .chunks_exact(2)
            .map(|pair| hashv(&[ORACLE_SKU_NODE_HASH_DOMAIN, &pair[0], &pair[1]]).to_bytes())
            .collect();
    }
    nodes[0]
}

#[test]
fn batx_manifest_pins_the_root_tag_190_computes() {
    let manifest: Value = serde_json::from_str(MANIFEST).expect("manifest JSON");
    assert_eq!(manifest["product"]["symbol"], "BATX");
    let mut labels: Vec<&str> = manifest["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["subgroups"].as_array().unwrap())
        .flat_map(|subgroup| subgroup["skuLabels"].as_array().unwrap())
        .map(|label| label.as_str().unwrap())
        .collect();
    labels.sort_by_key(|label| encode(label));
    assert_eq!(
        labels,
        [
            "LFP_EV",
            "LFP_STORAGE",
            "TERNARY_POUCH_EV",
            "TERNARY_PRISM_EV"
        ]
    );
    let ids: Vec<[u8; 32]> = labels.iter().map(|label| encode(label)).collect();
    assert!(
        ids.len() <= MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS,
        "one tag-190 transaction"
    );
    assert_eq!(manifest["requiredSkuCount"], ids.len());
    let hex: String = root(&ids)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(manifest["requiredSkuRootHex"], hex.as_str());
}

/// The spec's buckets in its order: (label, weight bps).
fn buckets() -> Vec<(String, u16)> {
    let spec: Value = serde_json::from_str(SPEC).expect("spec JSON");
    spec["basket"]["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["id"].as_str().unwrap().to_string(),
                row["weight_bps"].as_u64().unwrap() as u16,
            )
        })
        .collect()
}

/// One Active source with an accepted opening (`baseline`) and current value.
fn source(
    month: Pubkey,
    index: u8,
    bucket: &str,
    weight: u16,
    baseline: u64,
    now: u64,
) -> OracleSourceState {
    OracleSourceState {
        is_initialized: true,
        month,
        source_id: [index + 1; 32],
        bucket_id: encode(bucket),
        bucket_weight_bps: weight,
        status: OracleSourceStatus::Active,
        opening_submitted: true,
        baseline_state: baseline,
        current_state: now,
        observation_count: 2,
        ..Default::default()
    }
}

fn game_month(sources: usize) -> OracleMonthState {
    OracleMonthState {
        phase: OraclePhase::Game,
        source_count: sources as u16,
        ..Default::default()
    }
}

/// The index the program reports, as 100 + delta: one source per bucket, each at 0.300000
/// CNY/Wh at its opening and moved by `changes_bps`.
fn one_source_index(changes_bps: &Value) -> Option<String> {
    let month = Pubkey::new_unique();
    let sources: Vec<OracleSourceState> = buckets()
        .iter()
        .enumerate()
        .map(|(index, (label, weight))| {
            let change = changes_bps[label.as_str()].as_i64().unwrap();
            let now = (300_000 * (10_000 + change) / 10_000) as u64;
            source(month, index as u8, label, *weight, 300_000, now)
        })
        .collect();
    let delta = game_delta_bps(&month, &game_month(sources.len()), &sources)?;
    let points = 10_000 + delta;
    Some(format!("{}.{:02}", points / 100, points % 100))
}

#[test]
fn the_briefs_acceptance_tests_hold_on_chain() {
    let spec: Value = serde_json::from_str(SPEC).unwrap();
    let mut numeric = 0;
    for test in spec["acceptance_tests"].as_array().unwrap() {
        let Some(expected) = test["expected_index"].as_str() else {
            continue;
        };
        let expected: f64 = expected.parse().unwrap();
        assert_eq!(
            one_source_index(&test["changes_bps"]),
            Some(format!("{expected:.2}")),
            "{}",
            test["id"]
        );
        numeric += 1;
    }
    assert_eq!(numeric, 4);
}

#[test]
fn a_missing_series_leaves_the_index_unset_and_moves_no_weight() {
    let month = Pubkey::new_unique();
    let mut sources: Vec<OracleSourceState> = buckets()
        .iter()
        .enumerate()
        .map(|(index, (label, weight))| {
            source(month, index as u8, label, *weight, 300_000, 330_000)
        })
        .collect();
    assert_eq!(
        game_delta_bps(&month, &game_month(4), &sources),
        Some(1_000)
    );
    // The pouch bucket's only source goes inactive: no median, no index. The program never
    // spreads its 10% over the other buckets.
    sources[3].status = OracleSourceStatus::Inactive;
    assert_eq!(game_delta_bps(&month, &game_month(4), &sources), None);
    // Leaving it out of the inventory doesn't help either: the inventory must be complete.
    assert_eq!(game_delta_bps(&month, &game_month(4), &sources[..3]), None);
}

#[test]
fn within_a_bucket_the_program_takes_the_median_not_coefficients() {
    // The brief's reference weights a bucket's series by precommitted coefficients a[c,s];
    // the program settles a bucket on the median of its sources' changes. Three storage
    // series at +10%, 0, 0: equal coefficients would give storage +3.33% (BATX 101.33); the
    // median gives storage 0 (BATX 100). With one series per bucket the two agree.
    let month = Pubkey::new_unique();
    let mut sources = Vec::new();
    for (index, now) in [330_000u64, 300_000, 300_000].into_iter().enumerate() {
        sources.push(source(
            month,
            index as u8,
            "LFP_STORAGE",
            4_000,
            300_000,
            now,
        ));
    }
    for (index, (label, weight)) in buckets().iter().enumerate().skip(1) {
        sources.push(source(
            month,
            10 + index as u8,
            label,
            *weight,
            300_000,
            300_000,
        ));
    }
    assert_eq!(
        game_delta_bps(&month, &game_month(sources.len()), &sources),
        Some(0)
    );
    // Two storage series: the median of +10% and 0 is +5%, as their equal-coefficient mean.
    sources.remove(2);
    assert_eq!(
        game_delta_bps(&month, &game_month(sources.len()), &sources),
        Some(200)
    );
}

#[test]
fn the_program_counts_changes_in_whole_bps() {
    // The brief keeps every decimal until the output; the program truncates each source's
    // change to whole bps. A +0.00004% storage move is BATX 100.000016 in the reference and
    // 100.00 on chain.
    let month = Pubkey::new_unique();
    let sources: Vec<OracleSourceState> = buckets()
        .iter()
        .enumerate()
        .map(|(index, (label, weight))| {
            let now = if index == 0 { 10_000_004 } else { 10_000_000 };
            source(month, index as u8, label, *weight, 10_000_000, now)
        })
        .collect();
    assert_eq!(game_delta_bps(&month, &game_month(4), &sources), Some(0));
}
