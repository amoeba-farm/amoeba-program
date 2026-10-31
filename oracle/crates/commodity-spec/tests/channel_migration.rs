//! Holds `governance/channel_migration.v1.json`, the LTA brief's migration step (§8 G), to its
//! inputs: every existing source appears exactly once, each origin id is the SHA-256 of its
//! declared identity, duplicates point at a source of the same origin, item and channel, and
//! the published counts are the ones the rows give.

use std::collections::{BTreeMap, BTreeSet};

use ameba_commodity_spec::Product;
use serde_json::Value;
use sha2::{Digest, Sha256};

const MIGRATION: &str = include_str!("../../../governance/channel_migration.v1.json");
const CFM_REGISTRY: &str = include_str!("../../../governance/cfm_parent_proxy_registry.v1.json");
const BATX_SOURCES: &str = include_str!("../../../governance/batx_source_manifest.v1.json");
const DEV_SOURCES: &str =
    include_str!("../../../programs/ameba_oracle/tests/fixtures/dev-oracle-sources.json");
const DEV_CHIP_SOURCES: &str =
    include_str!("../../../programs/ameba_oracle/tests/fixtures/dev-oracle-chip-sources.json");

const ORIGIN_TAG: &str = "amoeba-oracle-origin-v1";
const CHANNELS: [&str; 3] = ["spot", "contract", "lta"];
const BOARDS: [&str; 3] = ["mainnet", "dev", "catalog"];

fn json(text: &str) -> Value {
    serde_json::from_str(text).expect("valid JSON")
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} is not a string in {value}"))
}

fn rows(migration: &Value) -> &Vec<Value> {
    migration["sources"].as_array().expect("sources")
}

fn normalized(part: &str) -> String {
    part.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn boards(row: &Value) -> Vec<&str> {
    row["boards"]
        .as_array()
        .expect("boards")
        .iter()
        .map(|board| board.as_str().expect("board"))
        .collect()
}

/// The CFM launch rows' labels by source id (`cfm_parent_proxy_registry.v1.json`).
fn launch_labels() -> BTreeMap<String, (String, String)> {
    let registry = json(CFM_REGISTRY);
    registry["parentRows"]
        .as_array()
        .expect("parentRows")
        .iter()
        .map(|row| {
            let transcript = text(row, "definitionTranscript");
            let (_, definition) = transcript.split_once('\0').expect("transcript separator");
            let definition = json(definition);
            (
                text(row, "parentSourceId").to_string(),
                (
                    text(row, "product").to_string(),
                    text(&definition, "exact_row_label").to_string(),
                ),
            )
        })
        .collect()
}

/// Every source the repository lists: the dev fixtures' sources, the CFM launch rows and the
/// BATX manifest's candidate series and leads, with the product and item each must carry.
fn expected_sources() -> BTreeMap<String, (String, String, Option<String>)> {
    let mut expected = BTreeMap::new();
    let launch = launch_labels();
    for (id, (product, label)) in &launch {
        expected.insert(id.clone(), (product.clone(), label.clone(), None));
    }
    for fixture in [DEV_SOURCES, DEV_CHIP_SOURCES] {
        let fixture = json(fixture);
        for product in fixture["products"].as_array().expect("products") {
            let symbol = text(product, "product");
            for source in product["sources"].as_array().expect("sources") {
                let id = text(source, "sourceId").to_string();
                let item = match source["bucketLabel"].as_str() {
                    Some(label) => label.to_string(),
                    None => launch[&id].1.clone(),
                };
                let url = Some(text(source, "url").to_string());
                expected.insert(id, (symbol.to_string(), item, url));
            }
        }
    }
    let batx = json(BATX_SOURCES);
    for (category, entries) in batx["categories"].as_object().expect("categories") {
        for entry in entries.as_array().expect("entries") {
            let id = format!("batx-manifest:{category}:{}", text(entry, "publisher"));
            let url = entry["url"].as_str().map(str::to_string);
            expected.insert(id, ("BATX".into(), category.clone(), url));
        }
    }
    for category in ["TERNARY_PRISM_EV", "LFP_EV"] {
        let id = format!("batx-manifest:{category}:iccsino");
        expected.insert(id, ("BATX".into(), category.into(), None));
    }
    expected
}

#[test]
fn every_existing_source_appears_exactly_once() {
    let migration = json(MIGRATION);
    let mut seen = BTreeMap::new();
    for row in rows(&migration) {
        let id = text(row, "id");
        assert!(
            seen.insert(id.to_string(), row).is_none(),
            "{id} appears twice"
        );
    }
    let expected = expected_sources();
    assert_eq!(expected.len(), 35 + 12 + 17);
    for (id, (product, item, url)) in &expected {
        let row = seen
            .get(id)
            .unwrap_or_else(|| panic!("{id} ({product} {item}) is missing"));
        assert_eq!(text(row, "product"), product, "{id}");
        assert_eq!(text(row, "item"), item, "{id}");
        if let Some(url) = url {
            assert_eq!(text(row, "url"), url, "{id}");
        }
    }
    // Rows the repository does not list come from the APIs' reads: mainnet's BATX Nov 26
    // proposals (on chain) and the off-chain candidate stores.
    for (id, row) in &seen {
        if expected.contains_key(id) {
            continue;
        }
        let kind = text(row, "kind");
        match kind {
            "candidate" => {
                let hash = id.strip_prefix("candidate:").expect("candidate id");
                assert!(hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
            }
            "source" => {
                assert_eq!(boards(row), ["mainnet"], "{id}");
                assert_eq!(text(row, "product"), "BATX", "{id}");
                assert!(id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()));
            }
            _ => panic!("{id}: a {kind} row the repository does not list"),
        }
    }
}

#[test]
fn origin_ids_are_the_hashes_of_their_identities() {
    let migration = json(MIGRATION);
    let mut origins = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for origin in migration["origins"].as_array().expect("origins") {
        let identity = format!(
            "{ORIGIN_TAG}\n{}\n{}",
            normalized(text(origin, "publisher")),
            normalized(text(origin, "report"))
        );
        assert_eq!(text(origin, "identity"), identity);
        let id = sha256_hex(identity.as_bytes());
        assert_eq!(text(origin, "originId"), id, "{identity:?}");
        assert!(ids.insert(id.clone()), "two origins share {identity:?}");
        let label = text(origin, "label");
        assert!(origins.insert(label.to_string(), id).is_none(), "{label}");
    }
    for row in rows(&migration) {
        let after = &row["after"];
        let label = text(after, "origin");
        let id = origins
            .get(label)
            .unwrap_or_else(|| panic!("{}: unknown origin {label}", text(row, "id")));
        assert_eq!(text(after, "originId"), id, "{}", text(row, "id"));
    }
}

#[test]
fn channels_items_and_duplicates_are_consistent() {
    let migration = json(MIGRATION);
    let launch: BTreeSet<_> = launch_labels().into_values().collect();
    let by_id: BTreeMap<_, _> = rows(&migration)
        .iter()
        .map(|row| (text(row, "id"), row))
        .collect();
    for row in rows(&migration) {
        let id = text(row, "id");
        let product = text(row, "product");
        let item = text(row, "item");
        let symbol = Product::from_symbol(product).unwrap_or_else(|| panic!("{id}: {product}"));
        match text(row, "itemKind") {
            "launch-category" => assert!(
                launch.contains(&(product.to_string(), item.to_string())),
                "{id}: {product} {item} is no launch category"
            ),
            "chip-bucket" | "cell-category" => {
                assert!(symbol.bucket(item).is_some(), "{id}: {product} {item}")
            }
            other => panic!("{id}: item kind {other}"),
        }
        for board in boards(row) {
            assert!(BOARDS.contains(&board), "{id}: board {board}");
        }
        let after = &row["after"];
        let channel = text(after, "channel");
        assert!(
            channel == "none" || CHANNELS.contains(&channel),
            "{id}: {channel}"
        );
        assert!(!text(after, "reason").is_empty(), "{id}");
        match after["duplicateOf"].as_str() {
            None => assert!(after["collapse"].is_null(), "{id}"),
            Some(original) => {
                assert_ne!(channel, "none", "{id}: a none row collapses into nothing");
                let collapse = text(after, "collapse");
                assert!(["republication", "sibling-row"].contains(&collapse), "{id}");
                let first = by_id
                    .get(original)
                    .unwrap_or_else(|| panic!("{id}: duplicate of unknown {original}"));
                assert!(
                    first["after"]["duplicateOf"].is_null(),
                    "{id}: {original} is itself a duplicate"
                );
                for key in ["product", "item"] {
                    assert_eq!(first[key], row[key], "{id} vs {original}: {key}");
                }
                for key in ["channel", "originId"] {
                    assert_eq!(first["after"][key], after[key], "{id} vs {original}: {key}");
                }
                let shared = boards(row).into_iter().any(|b| boards(first).contains(&b));
                assert!(shared, "{id} and {original} share no board");
            }
        }
    }
}

/// Distinct origins per channel among the rows on `board` ("all" for every board).
fn counted(migration: &Value, board: &str, product: &str, item: &str) -> Value {
    let mut out = serde_json::Map::new();
    for channel in CHANNELS {
        let (mut all, mut allowed) = (BTreeSet::new(), BTreeSet::new());
        for row in rows(migration) {
            let after = &row["after"];
            let on_board = board == "all" || boards(row).contains(&board);
            if on_board
                && text(row, "product") == product
                && text(row, "item") == item
                && text(after, "channel") == channel
            {
                all.insert(text(after, "originId"));
                let banned = after["blockers"]
                    .as_array()
                    .expect("blockers")
                    .iter()
                    .any(|code| code == "terms_ban");
                if !banned {
                    allowed.insert(text(after, "originId"));
                }
            }
        }
        out.insert(
            channel.into(),
            serde_json::json!({ "origins": all.len(), "withoutTermsBan": allowed.len() }),
        );
    }
    Value::Object(out)
}

#[test]
fn published_counts_are_the_rows_counts() {
    let migration = json(MIGRATION);
    let summary = &migration["summary"];
    let mut contract_three = BTreeSet::new();
    for (board, products) in summary["currentItems"].as_object().expect("currentItems") {
        for (product, items) in products.as_object().expect("products") {
            let symbol = Product::from_symbol(product).expect("product");
            let listed: Vec<_> = items.as_object().expect("items").keys().cloned().collect();
            let buckets: Vec<_> = symbol.buckets().iter().map(|b| b.id.to_string()).collect();
            let (mut listed_sorted, mut buckets_sorted) = (listed.clone(), buckets);
            listed_sorted.sort();
            buckets_sorted.sort();
            assert_eq!(listed_sorted, buckets_sorted, "{board} {product}");
            for (item, counts) in items.as_object().expect("items") {
                assert_eq!(
                    counts,
                    &counted(&migration, board, product, item),
                    "{board} {product} {item}"
                );
                if counts["contract"]["origins"].as_u64().expect("count") >= 3 {
                    contract_three.insert(format!("{product}/{item}"));
                }
            }
        }
    }
    for (product, items) in summary["launchItems"].as_object().expect("launchItems") {
        let launch = launch_labels();
        let expected = launch.values().filter(|(p, _)| p == product).count();
        assert_eq!(
            items.as_object().expect("items").len(),
            expected,
            "{product}"
        );
        for (item, counts) in items.as_object().expect("items") {
            assert_eq!(
                counts,
                &counted(&migration, "mainnet", product, item),
                "{product} {item}"
            );
            if counts["contract"]["origins"].as_u64().expect("count") >= 3 {
                contract_three.insert(format!("{product}/{item}"));
            }
        }
    }
    let published: BTreeSet<String> = summary["contractChannelsWithThreeOrigins"]
        .as_array()
        .expect("list")
        .iter()
        .map(|item| item.as_str().expect("item").to_string())
        .collect();
    assert_eq!(published, contract_three);
}
