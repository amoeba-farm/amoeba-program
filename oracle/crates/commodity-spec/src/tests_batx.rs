//! BATX: its tables against `batx_basket_spec.v1.json` and its manifest, its routing, and
//! the brief's acceptance tests (§5) through the fixed-base index.

use serde_json::Value;

use super::index::*;
use super::tests::{scope, sha256, sku_root};
use super::*;

const BRIEF_PDF: &[u8] =
    include_bytes!("../../../governance/batx_battery_cell_basket_brief.v1.pdf");

fn spec() -> Value {
    serde_json::from_str(BATX_SPEC_JSON).expect("the BATX spec is JSON")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn the_spec_and_the_brief_are_the_pinned_documents() {
    assert_eq!(
        hex(&sha256(&[BATX_SPEC_JSON.as_bytes()])),
        BATX_SPEC_SHA256_HEX
    );
    assert_eq!(hex(&sha256(&[BRIEF_PDF])), BATX_BRIEF_SHA256_HEX);
    let spec = spec();
    assert_eq!(spec["version"], BATX_SPEC_VERSION);
    assert_eq!(spec["source_document"]["sha256"], BATX_BRIEF_SHA256_HEX);
}

#[test]
fn the_bucket_table_is_the_briefs() {
    let spec = spec();
    let rows = spec["basket"]["buckets"].as_array().expect("buckets");
    assert_eq!(rows.len(), BATX_BUCKETS.len());
    let mut total = 0u32;
    for (row, bucket) in rows.iter().zip(Product::Batx.buckets()) {
        assert_eq!(row["id"], bucket.id);
        assert_eq!(row["name"], bucket.name);
        assert_eq!(row["weight_bps"], bucket.weight_bps);
        assert_eq!(row["chemistry_family"].as_str(), bucket.chemistry);
        assert_eq!(row["casing"].as_str(), bucket.casing);
        assert_eq!(row["application"].as_str(), bucket.application);
        assert_eq!(
            spec["calculation"]["category_weights_bps"][bucket.id],
            bucket.weight_bps
        );
        assert_eq!(bucket.product, Product::Batx);
        assert_eq!(bucket.profile, Profile::BatxChinaIndustrialCell);
        assert_eq!(Bucket::from_id32(&bucket.id32()), Some(bucket));
        total += u32::from(bucket.weight_bps);
    }
    assert_eq!(total, 10_000);
    assert_eq!(
        weights(Product::Batx),
        vec![
            ("LFP_STORAGE", 4_000),
            ("LFP_EV", 3_000),
            ("TERNARY_PRISM_EV", 2_000),
            ("TERNARY_POUCH_EV", 1_000)
        ]
    );
    assert_eq!(Product::from_symbol("batx"), Some(Product::Batx));
    assert_eq!(Product::from_symbol("BATX"), Some(Product::Batx));
}

#[test]
fn the_manifest_lists_the_four_categories_under_its_pinned_root() {
    let cells = Product::Batx.manifest();
    let manifest: Value = serde_json::from_str(cells.json).expect("manifest JSON");
    assert_eq!(manifest["schema"], "amoeba.oracle-product-sku-manifest");
    assert_eq!(manifest["manifestVersion"], 1);
    assert_eq!(manifest["product"]["symbol"], "BATX");
    assert_eq!(manifest["product"]["marketId"], "batx");
    assert_eq!(
        manifest["provenance"]["sourceSha256"],
        BATX_BRIEF_SHA256_HEX
    );
    assert_eq!(manifest["requiredSkuCount"], cells.required_sku_count);
    assert_eq!(manifest["requiredSkuRootHex"], cells.required_sku_root_hex);
    let mut labels: Vec<String> = manifest["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["subgroups"].as_array().unwrap().iter())
        .flat_map(|subgroup| subgroup["skuLabels"].as_array().unwrap().iter())
        .map(|label| label.as_str().unwrap().to_string())
        .collect();
    labels.sort_by_key(|label| label_id32(label).unwrap());
    assert_eq!(
        cells.sorted_labels(),
        [
            "LFP_EV",
            "LFP_STORAGE",
            "TERNARY_POUCH_EV",
            "TERNARY_PRISM_EV"
        ]
    );
    assert_eq!(labels, cells.sorted_labels());
    assert_eq!(sku_root(&cells.sorted_ids()), cells.root());
}

#[test]
fn the_source_manifest_maps_every_category_and_names_its_publishers() {
    let manifest: Value = serde_json::from_str(include_str!(
        "../../../governance/batx_source_manifest.v1.json"
    ))
    .expect("source manifest JSON");
    let categories = manifest["categories"].as_object().unwrap();
    let mut ids: Vec<&str> = categories.keys().map(String::as_str).collect();
    let mut buckets: Vec<&str> = Product::Batx.buckets().iter().map(|b| b.id).collect();
    ids.sort_unstable();
    buckets.sort_unstable();
    assert_eq!(ids, buckets);
    for (bucket, rows) in categories {
        let rows = rows.as_array().unwrap();
        assert!(!rows.is_empty(), "{bucket}");
        for row in rows {
            let publisher = row["publisher"].as_str().unwrap();
            assert!(
                manifest["publishers"][publisher].is_object(),
                "{bucket}: {publisher}"
            );
            assert!(row["row"].as_str().is_some_and(|row| !row.is_empty()));
        }
        let counted = manifest["coverage"][bucket]["publishers"]
            .as_array()
            .unwrap();
        assert_eq!(manifest["coverage"][bucket]["count"], counted.len());
    }
}

#[test]
fn batx_starts_with_the_months_expiring_december_first() {
    assert_eq!(time::unix_seconds("2026-12-01"), Some(BATX_FIRST_EXPIRY));
    assert!(!Product::Batx.uses_buckets(1_793_491_200)); // Oct 26 months
    assert!(Product::Batx.uses_buckets(BATX_FIRST_EXPIRY)); // Nov 26 months
    assert!(Product::Batx.uses_buckets(1_798_761_600)); // Dec 26 months
    assert!(Product::Ramx.uses_buckets(BATX_FIRST_EXPIRY));
}

fn lfp_storage_assessment() -> SeriesDefinition {
    SeriesDefinition {
        schema: SERIES_SCHEMA.into(),
        series_id: Some("example.cell.lfp-ess-prismatic".into()),
        definition_version: Some(1),
        bucket: Some("LFP_STORAGE".into()),
        evidence_class: Some("published_assessment".into()),
        price_basis: Some("spot".into()),
        origin: Origin {
            publisher: Some("Example Research".into()),
            locator: Some("https://example.com/price/battery-cell".into()),
            row: Some("LFP prismatic energy-storage cell".into()),
        },
        scope_reference: Some("https://example.com/methodology/battery-cell".into()),
        physical_scope: Some("cell".into()),
        native_unit: Some("CNY per Wh".into()),
        statistic: Some("average".into()),
        currency: Some("CNY".into()),
        conversion_policy: None,
        procurement_segment: scope("documented_mix", "domestic industrial buyers"),
        geography: scope("exact", "CN"),
        reference_period: Some("month".into()),
        cadence: Some("monthly".into()),
        revision_policy: Some("a correction replaces the release it corrects".into()),
        missing_data_policy: Some("flag the month; no carry-forward".into()),
        dram: None,
        nand: None,
        cell: Some(CellScope {
            chemistry: Some("LFP".into()),
            casing: Some("prismatic".into()),
            application: Some("stationary_storage".into()),
            condition: Some("new_unused".into()),
            capacity: scope("documented_mix", "280Ah and 314Ah"),
            nominal_voltage: scope("exact", "3.2V"),
            rated_energy_wh: None,
            performance_class: scope("not_disclosed", ""),
            tax_basis: scope("exact", "VAT-inclusive"),
            delivery_terms: scope("not_disclosed", ""),
        }),
        direct: None,
        notes: None,
    }
}

fn ncm_pouch_assessment() -> SeriesDefinition {
    let mut series = lfp_storage_assessment();
    series.series_id = Some("example.cell.ncm-ev-pouch".into());
    series.bucket = Some("TERNARY_POUCH_EV".into());
    series.origin.row = Some("NCM pouch EV cell".into());
    let cell = series.cell.as_mut().unwrap();
    cell.chemistry = Some("NCM".into());
    cell.casing = Some("pouch".into());
    cell.application = Some("electric_vehicle".into());
    cell.capacity = scope("documented_mix", "mainstream EV pouch capacities");
    cell.nominal_voltage = scope("not_disclosed", "");
    series
}

fn cell_quote() -> SeriesDefinition {
    let mut series = lfp_storage_assessment();
    series.series_id = Some("example.quote.lfp-314ah".into());
    series.evidence_class = Some("direct_price".into());
    series.price_basis = Some("firm_offer".into());
    series.origin.row = Some("LFP 314Ah prismatic storage cell".into());
    series.native_unit = Some("CNY per cell".into());
    series.statistic = Some("latest".into());
    series.procurement_segment = scope("exact", "industrial buyers, 1 MWh lots");
    let cell = series.cell.as_mut().unwrap();
    cell.capacity = scope("exact", "314Ah");
    cell.rated_energy_wh = Some("1004.8".into());
    cell.performance_class = scope("exact", "8000 cycles");
    series.direct = Some(DirectTerms {
        manufacturer: Some("Example Cells".into()),
        mpn: Some("EX-LFP-314".into()),
        specification_reference: Some("https://example.com/datasheet/ex-lfp-314".into()),
        quantity_tier: Some("1 MWh".into()),
        delivery_terms: Some("ex-works".into()),
        pricing_period: Some("month".into()),
        market: Some("China domestic".into()),
        tax_freight: Some("VAT-inclusive, freight excluded".into()),
        payment_terms: Some("net 30, no rebates".into()),
    });
    series
}

fn committed<'a>(sources: &'a [MonthSeries<'a>]) -> MonthContext<'a> {
    MonthContext {
        product: Product::Batx,
        stage: Stage::Committed,
        sources,
    }
}

fn status(series: &SeriesDefinition) -> Status {
    route_series(series, Product::Batx).status
}

#[test]
fn complete_cell_series_fit_their_bucket_and_count_once_committed() {
    for series in [
        lfp_storage_assessment(),
        ncm_pouch_assessment(),
        cell_quote(),
    ] {
        let routing = route_series(&series, Product::Batx);
        assert_eq!(routing.status, Status::ClassifiedCandidate, "{:?}", routing);
        assert!(routing.reasons.is_empty(), "{:?}", routing.reasons);
    }
    let series = lfp_storage_assessment();
    let sources = [MonthSeries {
        source_id: "s1",
        definition: Some(&series),
        committed: true,
    }];
    assert_eq!(
        route_series_in_month(&series, &committed(&sources), Some("s1")).status,
        Status::EligibleForContract
    );
}

#[test]
fn excluded_products_are_the_wrong_product() {
    for scope_name in [
        "pack",
        "storage_system",
        "battery_module",
        "cathode_material",
        "lithium_carbonate",
        "retail_bundle",
    ] {
        let mut series = lfp_storage_assessment();
        series.physical_scope = Some(scope_name.into());
        assert_eq!(status(&series), Status::WrongProduct, "{scope_name}");
    }
    let mut cylindrical = ncm_pouch_assessment();
    cylindrical.cell.as_mut().unwrap().casing = Some("cylindrical".into());
    let routing = route_series(&cylindrical, Product::Batx);
    assert_eq!(routing.status, Status::WrongProduct);
    assert!(routing
        .deciding()
        .any(|reason| reason.field == "cell.casing"));
    for condition in ["used", "second_life", "refurbished"] {
        let mut series = lfp_storage_assessment();
        series.cell.as_mut().unwrap().condition = Some(condition.into());
        assert_eq!(status(&series), Status::WrongProduct, "{condition}");
    }
    let mut sodium = lfp_storage_assessment();
    sodium.cell.as_mut().unwrap().chemistry = Some("sodium_ion".into());
    assert_eq!(status(&sodium), Status::WrongProduct);
    // A chip bucket is not a BATX bucket.
    let mut chip = lfp_storage_assessment();
    chip.bucket = Some("DDR5_16GBIT".into());
    assert_eq!(status(&chip), Status::WrongProduct);
}

#[test]
fn each_observation_lands_in_exactly_one_category() {
    // An LFP cell can't inform a ternary bucket, a pouch cell a prismatic one, or a vehicle
    // cell the storage bucket.
    let mut lfp_as_ternary = lfp_storage_assessment();
    lfp_as_ternary.bucket = Some("TERNARY_PRISM_EV".into());
    assert_eq!(status(&lfp_as_ternary), Status::WrongProduct);
    let mut pouch_as_prismatic = ncm_pouch_assessment();
    pouch_as_prismatic.bucket = Some("TERNARY_PRISM_EV".into());
    let routing = route_series(&pouch_as_prismatic, Product::Batx);
    assert_eq!(routing.status, Status::WrongProduct);
    assert!(routing
        .deciding()
        .any(|reason| reason.field == "cell.casing"));
    let mut ev_as_storage = lfp_storage_assessment();
    ev_as_storage.cell.as_mut().unwrap().application = Some("electric_vehicle".into());
    assert_eq!(status(&ev_as_storage), Status::WrongProduct);
    let mut storage_as_ev = lfp_storage_assessment();
    storage_as_ev.bucket = Some("LFP_EV".into());
    assert_eq!(status(&storage_as_ev), Status::WrongProduct);
}

#[test]
fn other_markets_and_currencies_are_not_blended_in() {
    let mut usd = lfp_storage_assessment();
    usd.currency = Some("USD".into());
    assert_eq!(status(&usd), Status::ResearchOnly);
    let mut us = lfp_storage_assessment();
    us.geography = scope("exact", "US");
    assert_eq!(status(&us), Status::ResearchOnly);
    let mut blend = lfp_storage_assessment();
    blend.geography = scope("documented_mix", "China and Europe");
    assert_eq!(status(&blend), Status::ResearchOnly);
    let mut unstated = lfp_storage_assessment();
    unstated.geography = scope("not_disclosed", "");
    assert_eq!(status(&unstated), Status::NeedsMetadata);
    let mut retail = lfp_storage_assessment();
    retail.price_basis = Some("listing".into());
    assert_eq!(status(&retail), Status::WrongPriceBasis);
    let mut forecast = lfp_storage_assessment();
    forecast.price_basis = Some("forecast".into());
    assert_eq!(status(&forecast), Status::WrongPriceBasis);
    let mut contract = lfp_storage_assessment();
    contract.price_basis = Some("contract".into());
    assert_eq!(status(&contract), Status::ClassifiedCandidate);
}

#[test]
fn unknowns_are_never_assumed() {
    let mut series = lfp_storage_assessment();
    series.cell.as_mut().unwrap().condition = Some("unknown".into());
    series.cell.as_mut().unwrap().capacity = scope("not_disclosed", "");
    let routing = route_series(&series, Product::Batx);
    assert_eq!(routing.status, Status::NeedsMetadata);
    let fields: Vec<&str> = routing.deciding().map(|r| r.field.as_str()).collect();
    assert!(fields.contains(&"cell.condition"), "{fields:?}");
    assert!(fields.contains(&"cell.capacity"), "{fields:?}");
    let mut no_cell = lfp_storage_assessment();
    no_cell.cell = None;
    assert_eq!(status(&no_cell), Status::NeedsMetadata);
    // A direct quote is one exact cell: a capacity range or an unstated voltage won't do.
    let mut quote = cell_quote();
    quote.cell.as_mut().unwrap().capacity = scope("documented_mix", "280Ah and 314Ah");
    quote.cell.as_mut().unwrap().nominal_voltage = scope("not_disclosed", "");
    let routing = route_series(&quote, Product::Batx);
    assert_eq!(routing.status, Status::NeedsMetadata);
    let fields: Vec<&str> = routing.deciding().map(|r| r.field.as_str()).collect();
    assert!(fields.contains(&"cell.capacity"), "{fields:?}");
    assert!(fields.contains(&"cell.nominalVoltage"), "{fields:?}");
}

#[test]
fn a_changed_basis_or_capacity_is_a_scope_change() {
    let original = lfp_storage_assessment();
    for change in ["priceBasis", "cell.capacity", "geography", "cell.taxBasis"] {
        let mut next = original.clone();
        next.definition_version = Some(2);
        match change {
            "priceBasis" => next.price_basis = Some("contract".into()),
            "cell.capacity" => next.cell.as_mut().unwrap().capacity = scope("exact", "314Ah"),
            "geography" => next.geography = scope("exact", "US"),
            _ => next.cell.as_mut().unwrap().tax_basis = scope("exact", "VAT-exclusive"),
        }
        assert!(comparability_changes(&original, &next).contains(&change));
        let sources = [
            MonthSeries {
                source_id: "s1",
                definition: Some(&original),
                committed: true,
            },
            MonthSeries {
                source_id: "s2",
                definition: Some(&next),
                committed: false,
            },
        ];
        assert_eq!(
            route_series_in_month(&next, &committed(&sources), Some("s2")).status,
            Status::ScopeChange,
            "{change}"
        );
    }
}

#[test]
fn one_publisher_on_more_websites_gains_no_weight() {
    let original = lfp_storage_assessment();
    let mut repost = original.clone();
    repost.series_id = Some("someone-else.reposted-lfp-ess".into());
    repost.origin.locator = Some("https://news.example.org/battery-prices".into());
    let sources = [MonthSeries {
        source_id: "s1",
        definition: Some(&original),
        committed: true,
    }];
    let month = MonthContext {
        product: Product::Batx,
        stage: Stage::Open,
        sources: &sources,
    };
    assert_eq!(
        route_series_in_month(&repost, &month, None).status,
        Status::DuplicateNoExtraWeight
    );
}

#[test]
fn a_cheaper_unapproved_model_does_not_join_a_committed_month() {
    let approved = lfp_storage_assessment();
    let mut cheaper = cell_quote();
    cheaper.series_id = Some("example.quote.lfp-280ah-cheap".into());
    cheaper.origin.row = Some("LFP 280Ah prismatic storage cell".into());
    cheaper.direct.as_mut().unwrap().mpn = Some("EX-LFP-280".into());
    cheaper.cell.as_mut().unwrap().capacity = scope("exact", "280Ah");
    let sources = [
        MonthSeries {
            source_id: "s1",
            definition: Some(&approved),
            committed: true,
        },
        MonthSeries {
            source_id: "s2",
            definition: Some(&cheaper),
            committed: false,
        },
    ];
    assert_eq!(
        route_series_in_month(&cheaper, &committed(&sources), Some("s2")).status,
        Status::OutsideCommittedSeries
    );
}

#[test]
fn templates_and_definitions_fit_a_source() {
    for bucket in Product::Batx.buckets() {
        let template = series_template(bucket, false);
        assert_eq!(template["currency"], "CNY");
        assert_eq!(template["geography"]["value"], "CN");
        assert_eq!(template["cell"]["casing"].as_str(), bucket.casing);
    }
    for series in [
        lfp_storage_assessment(),
        ncm_pouch_assessment(),
        cell_quote(),
    ] {
        let text = series.definition_text();
        assert!(
            text.len() <= MAX_DEFINITION_TEXT_BYTES,
            "{} bytes",
            text.len()
        );
        assert_eq!(SeriesDefinition::parse(&text).unwrap(), series);
    }
}

// The fixed-base index (brief §4) and its acceptance tests (§5).

/// 0.300000 CNY/Wh, a base for each category's one series.
const BASE_ATOMS: u64 = 300_000;

fn point(atoms: u64, id: &'static str) -> PricePoint<'static> {
    PricePoint {
        atoms,
        observation_id: id,
        evidence_reference: "https://web.archive.org/web/2026/https://example.com/",
        reference_period_end: "2026-11-30",
    }
}

fn one_series_panel() -> Vec<PanelSeries<'static>> {
    Product::Batx
        .buckets()
        .iter()
        .map(|bucket| PanelSeries {
            series_id: bucket.id,
            bucket: bucket.id,
            coefficient_bps: 10_000,
            base: Some(point(BASE_ATOMS, "base")),
        })
        .collect()
}

fn moved(changes_bps: &Value) -> Vec<ObservedPrice<'static>> {
    Product::Batx
        .buckets()
        .iter()
        .map(|bucket| {
            let change = changes_bps[bucket.id]
                .as_i64()
                .expect("a change per bucket");
            let atoms = (BASE_ATOMS as i64 * (10_000 + change) / 10_000) as u64;
            ObservedPrice {
                series_id: bucket.id,
                point: point(atoms, "now"),
            }
        })
        .collect()
}

#[test]
fn the_briefs_numeric_acceptance_tests_pass() {
    let spec = spec();
    let mut numeric = 0;
    for test in spec["acceptance_tests"].as_array().unwrap() {
        let Some(expected) = test["expected_index"].as_str() else {
            continue;
        };
        let report = fixed_base_index(
            Product::Batx,
            &one_series_panel(),
            &moved(&test["changes_bps"]),
            IndexOptions::default(),
        );
        assert!(
            report.flags.is_empty(),
            "{}: {:?}",
            test["id"],
            report.flags
        );
        let expected: f64 = expected.parse().unwrap();
        assert_eq!(
            report.index.as_deref(),
            Some(format!("{expected:.6}").as_str()),
            "{}",
            test["id"]
        );
        numeric += 1;
    }
    assert_eq!(numeric, 4);
}

#[test]
fn contributions_explain_the_total() {
    let report = fixed_base_index(
        Product::Batx,
        &one_series_panel(),
        &moved(&serde_json::json!({
            "LFP_STORAGE": 1000, "LFP_EV": -500, "TERNARY_PRISM_EV": 0, "TERNARY_POUCH_EV": 0
        })),
        IndexOptions::default(),
    );
    assert_eq!(report.index.as_deref(), Some("102.500000"));
    let storage = &report.categories[0];
    assert_eq!(storage.bucket, "LFP_STORAGE");
    assert_eq!(storage.relative.as_deref(), Some("1.100000"));
    assert_eq!(storage.index.as_deref(), Some("110.000000"));
    assert_eq!(storage.contribution.as_deref(), Some("44.000000"));
    assert_eq!(storage.change.as_deref(), Some("4.000000"));
    let lfp_ev = &report.categories[1];
    assert_eq!(lfp_ev.change.as_deref(), Some("-1.500000"));
    assert_eq!(lfp_ev.contribution.as_deref(), Some("28.500000"));
    assert_eq!(report.categories[2].change.as_deref(), Some("0.000000"));
    assert_eq!(storage.series[0].base_atoms, Some(300_000));
    assert_eq!(storage.series[0].current_atoms, Some(330_000));
    assert_eq!(
        storage.series[0].current_observation.as_deref(),
        Some("now")
    );
    assert!(storage.series[0]
        .current_evidence
        .as_deref()
        .is_some_and(|evidence| evidence.starts_with("https://web.archive.org/")));
}

#[test]
fn a_cheaper_unapproved_model_changes_nothing() {
    let unchanged = serde_json::json!({
        "LFP_STORAGE": 0, "LFP_EV": 0, "TERNARY_PRISM_EV": 0, "TERNARY_POUCH_EV": 0
    });
    let mut observations = moved(&unchanged);
    observations.push(ObservedPrice {
        series_id: "a-cheaper-280ah-model",
        point: point(150_000, "cheap"),
    });
    let report = fixed_base_index(
        Product::Batx,
        &one_series_panel(),
        &observations,
        IndexOptions::default(),
    );
    assert_eq!(report.index.as_deref(), Some("100.000000"));
    assert_eq!(
        report.flags,
        vec![Flag::OutsidePanel {
            series_id: "a-cheaper-280ah-model".into()
        }]
    );
}

#[test]
fn a_missing_series_is_flagged_and_its_weight_goes_nowhere() {
    let changes = serde_json::json!({
        "LFP_STORAGE": 1000, "LFP_EV": 0, "TERNARY_PRISM_EV": 0, "TERNARY_POUCH_EV": 0
    });
    let observations: Vec<ObservedPrice<'_>> = moved(&changes)
        .into_iter()
        .filter(|observation| observation.series_id != "TERNARY_POUCH_EV")
        .collect();
    let report = fixed_base_index(
        Product::Batx,
        &one_series_panel(),
        &observations,
        IndexOptions::default(),
    );
    assert_eq!(report.index, None);
    assert_eq!(
        report.flags,
        vec![Flag::MissingData {
            series_id: "TERNARY_POUCH_EV".into(),
            bucket: "TERNARY_POUCH_EV".into()
        }]
    );
    // The others keep their own weight: storage still contributes 40% of its 110.
    assert_eq!(
        report.categories[0].contribution.as_deref(),
        Some("44.000000")
    );
    assert_eq!(report.categories[3].relative, None);
    // A missing base blocks the same way.
    let mut panel = one_series_panel();
    panel[1].base = None;
    let report = fixed_base_index(
        Product::Batx,
        &panel,
        &moved(&changes),
        IndexOptions::default(),
    );
    assert_eq!(report.index, None);
    assert!(matches!(report.flags[0], Flag::MissingBase { .. }));
}

#[test]
fn coefficients_weight_series_within_a_bucket() {
    let mut panel = one_series_panel();
    panel[0].series_id = "storage-a";
    panel[0].coefficient_bps = 6_000;
    panel.insert(
        1,
        PanelSeries {
            series_id: "storage-b",
            bucket: "LFP_STORAGE",
            coefficient_bps: 4_000,
            base: Some(point(1_000_000, "base-b")),
        },
    );
    let unchanged = serde_json::json!({
        "LFP_STORAGE": 0, "LFP_EV": 0, "TERNARY_PRISM_EV": 0, "TERNARY_POUCH_EV": 0
    });
    let mut observations = moved(&unchanged);
    observations[0] = ObservedPrice {
        series_id: "storage-a",
        point: point(330_000, "now-a"),
    };
    observations.push(ObservedPrice {
        series_id: "storage-b",
        point: point(1_000_000, "now-b"),
    });
    // Storage = 0.6 * 1.10 + 0.4 * 1.00 = 1.06; BATX = 100 + 40% * 6 = 102.4. Each series is
    // relative to its own base, so their different price levels don't matter.
    let report = fixed_base_index(
        Product::Batx,
        &panel,
        &observations,
        IndexOptions::default(),
    );
    assert_eq!(report.categories[0].relative.as_deref(), Some("1.060000"));
    assert_eq!(report.index.as_deref(), Some("102.400000"));
    // Coefficients that don't make a whole bucket block the index.
    panel[1].coefficient_bps = 3_000;
    let report = fixed_base_index(
        Product::Batx,
        &panel,
        &observations,
        IndexOptions::default(),
    );
    assert_eq!(report.index, None);
    assert!(report.flags.contains(&Flag::CoefficientsNotWhole {
        bucket: "LFP_STORAGE".into(),
        sum_bps: 9_000
    }));
}

#[test]
fn precision_is_kept_until_the_output() {
    // Storage moves by 0.00004%: rounding its relative to 6 places first would lose it.
    let mut panel = one_series_panel();
    panel[0].base = Some(point(10_000_000, "base"));
    let unchanged = serde_json::json!({
        "LFP_STORAGE": 0, "LFP_EV": 0, "TERNARY_PRISM_EV": 0, "TERNARY_POUCH_EV": 0
    });
    let mut observations = moved(&unchanged);
    observations[0].point = point(10_000_004, "now");
    let report = fixed_base_index(
        Product::Batx,
        &panel,
        &observations,
        IndexOptions::default(),
    );
    assert_eq!(report.categories[0].relative.as_deref(), Some("1.000000"));
    assert_eq!(report.index.as_deref(), Some("100.000016"));
}

#[test]
fn freshness_is_reported_and_limits_are_the_contracts() {
    let unchanged = serde_json::json!({
        "LFP_STORAGE": 0, "LFP_EV": 0, "TERNARY_PRISM_EV": 0, "TERNARY_POUCH_EV": 0
    });
    let mut observations = moved(&unchanged);
    observations[0].point.observation_id = "base";
    let options = IndexOptions {
        as_of: Some("2026-12-20"),
        stale_after_days: None,
    };
    let report = fixed_base_index(Product::Batx, &one_series_panel(), &observations, options);
    assert_eq!(report.index.as_deref(), Some("100.000000"));
    assert_eq!(report.categories[0].series[0].age_days, Some(20));
    assert_eq!(
        report.flags,
        vec![Flag::NoNewObservation {
            series_id: "LFP_STORAGE".into()
        }]
    );
    let report = fixed_base_index(
        Product::Batx,
        &one_series_panel(),
        &observations,
        IndexOptions {
            stale_after_days: Some(14),
            ..options
        },
    );
    assert_eq!(report.flags.len(), 5);
    assert!(report.flags.contains(&Flag::Stale {
        series_id: "TERNARY_POUCH_EV".into(),
        age_days: 20
    }));
    // Staleness is reported, never repaired: the index still stands on what was observed.
    assert_eq!(report.index.as_deref(), Some("100.000000"));
}

#[test]
fn reports_serialize_for_the_api() {
    let report = fixed_base_index(
        Product::Batx,
        &one_series_panel(),
        &[],
        IndexOptions::default(),
    );
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["product"], "BATX");
    assert_eq!(json["index"], Value::Null);
    assert_eq!(json["categories"][0]["weightBps"], 4_000);
    assert_eq!(json["flags"][0]["flag"], "MISSING_DATA");
    assert_eq!(json["flags"][0]["seriesId"], "LFP_STORAGE");
}
