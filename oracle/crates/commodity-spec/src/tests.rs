use serde_json::Value;
use sha2::{Digest, Sha256};

use super::*;

fn spec() -> Value {
    serde_json::from_str(SPEC_JSON).expect("the spec is JSON")
}

pub(crate) fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

/// The program's SKU root (`processor/sku_manifest.rs`, tag 190).
pub(crate) fn sku_root(ids: &[[u8; 32]]) -> [u8; 32] {
    let mut width = 1;
    while width < ids.len() {
        width *= 2;
    }
    let mut nodes: Vec<[u8; 32]> = (0..width)
        .map(|index| {
            let index_bytes = (index as u16).to_le_bytes();
            match ids.get(index) {
                Some(id) => sha256(&[b"amoeba-oracle-sku-leaf-v1", &index_bytes, id]),
                None => sha256(&[b"amoeba-oracle-sku-empty-v1", &index_bytes]),
            }
        })
        .collect();
    while nodes.len() > 1 {
        nodes = nodes
            .chunks_exact(2)
            .map(|pair| sha256(&[b"amoeba-oracle-sku-node-v1", &pair[0], &pair[1]]))
            .collect();
    }
    nodes[0]
}

#[test]
fn the_spec_is_the_pinned_document() {
    let digest = sha256(&[SPEC_JSON.as_bytes()]);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(hex, SPEC_SHA256_HEX);
    assert_eq!(spec()["version"], SPEC_VERSION);
}

#[test]
fn the_bucket_tables_are_the_specs() {
    let spec = spec();
    for product in Product::CHIPS {
        let basket = &spec["baskets"][product.symbol()];
        // The product's buckets are the spec's less its research candidates (RAMX's DDR5 32Gb,
        // the user's 2026-10-21), and those candidates are the spec's too.
        let rows: Vec<&Value> = basket["buckets"]
            .as_array()
            .expect("buckets")
            .iter()
            .filter(|row| {
                product
                    .research_candidate(row["id"].as_str().unwrap())
                    .is_none()
            })
            .collect();
        assert_eq!(rows.len(), product.buckets().len());
        for candidate in product.research_candidates() {
            let row = basket["buckets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == candidate.id)
                .expect("a spec bucket");
            assert_eq!(row["nominal_die_density"], candidate.die_density);
            assert_eq!(candidate.weight_bps, 0);
        }
        let mut total = 0u32;
        for (row, bucket) in rows.iter().zip(product.buckets()) {
            assert_eq!(row["id"], bucket.id);
            // RAMX weighs as the user decided on 2026-10-21; NANDX as the spec.
            let weight = match product {
                Product::Ramx => [4_000, 2_000, 2_500, 1_500]
                    [product.buckets().iter().position(|b| b == bucket).unwrap()],
                _ => row["weight_bps"].as_u64().unwrap() as u16,
            };
            assert_eq!(weight, bucket.weight_bps);
            assert_eq!(row["nominal_die_density"], bucket.die_density);
            assert_eq!(row["profile"], bucket.profile.as_str());
            assert_eq!(bucket.product, product);
            match product {
                Product::Ramx => assert_eq!(row["generation"].as_str(), bucket.generation),
                Product::Nandx => assert_eq!(row["cell_type"].as_str(), bucket.cell_type),
                Product::Batx => unreachable!("BATX is in batx_basket_spec.v1.json"),
            }
            total += u32::from(bucket.weight_bps);
            assert_eq!(Bucket::from_id32(&bucket.id32()), Some(bucket));
            assert_eq!(decode_label32(&bucket.id32()).as_deref(), Some(bucket.id));
        }
        assert_eq!(total, 10_000);
        assert_eq!(
            weights(product)
                .iter()
                .map(|(_, bps)| u32::from(*bps))
                .sum::<u32>(),
            10_000
        );
    }
}

#[test]
fn the_chip_manifests_list_the_buckets_under_their_pinned_roots() {
    for product in Product::CHIPS {
        let chips = product.manifest();
        let manifest: Value = serde_json::from_str(chips.json).expect("manifest JSON");
        assert_eq!(manifest["schema"], "amoeba.oracle-product-sku-manifest");
        assert_eq!(manifest["manifestVersion"], 2);
        assert_eq!(manifest["product"]["symbol"], product.symbol());
        assert_eq!(manifest["provenance"]["sourceSha256"], SPEC_SHA256_HEX);
        assert_eq!(manifest["requiredSkuCount"], chips.required_sku_count);
        assert_eq!(manifest["requiredSkuRootHex"], chips.required_sku_root_hex);
        assert_eq!(
            manifest["tag190Plan"]["chunkSkuCounts"],
            serde_json::json!([chips.required_sku_count])
        );
        let mut labels: Vec<String> = manifest["groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|group| group["subgroups"].as_array().unwrap().iter())
            .flat_map(|subgroup| subgroup["skuLabels"].as_array().unwrap().iter())
            .map(|label| label.as_str().unwrap().to_string())
            .collect();
        labels.sort_by_key(|label| label_id32(label).unwrap());
        assert_eq!(labels, chips.sorted_labels());
        assert_eq!(usize::from(chips.required_sku_count), labels.len());
        assert_eq!(sku_root(&chips.sorted_ids()), chips.root());
    }
}

#[test]
fn chip_months_are_those_expiring_after_november_first() {
    assert!(!uses_chip_buckets(1_793_491_200)); // Oct 26, expiry 2026-11-01
    assert!(uses_chip_buckets(1_796_083_200)); // Nov 26
    assert_eq!(
        time::unix_seconds("2026-11-01"),
        Some(CHIP_BUCKETS_AFTER_EXPIRY)
    );
}

pub(crate) fn scope(state: &str, value: &str) -> Option<ScopeValue> {
    Some(ScopeValue {
        state: Some(state.into()),
        value: Some(value.into()),
    })
}

/// A complete published assessment of DDR5 16Gb x8 chips.
fn ddr5_assessment() -> SeriesDefinition {
    SeriesDefinition {
        schema: SERIES_SCHEMA.into(),
        series_id: Some("example-ddr5-16gb-x8-contract-avg".into()),
        definition_version: Some(1),
        bucket: Some("DDR5_16GBIT".into()),
        evidence_class: Some("published_assessment".into()),
        price_basis: Some("contract".into()),
        origin: Origin {
            publisher: Some("Example Research".into()),
            locator: Some("https://example.com/price/dram/contract".into()),
            row: Some("DDR5 16Gb (2Gx8) 4800/5600".into()),
        },
        scope_reference: Some("https://example.com/methodology".into()),
        physical_scope: Some("packaged_ic".into()),
        native_unit: Some("USD per chip".into()),
        statistic: Some("average".into()),
        currency: Some("USD".into()),
        conversion_policy: None,
        procurement_segment: scope("documented_mix", "PC and server OEM contract buyers"),
        geography: scope("documented_mix", "global"),
        reference_period: Some("month".into()),
        cadence: Some("monthly".into()),
        revision_policy: Some(
            "the publisher's corrections replace the release they correct".into(),
        ),
        missing_data_policy: Some("carry the last accepted value forward".into()),
        dram: Some(DramScope {
            memory_family: Some("ddr_sdram".into()),
            generation: Some("DDR5".into()),
            die_density: Some("16Gb".into()),
            die_count: Some(1),
            condition: Some("new_unused".into()),
            grade: Some("standard_production".into()),
            organization: scope("exact", "x8"),
            speed: scope("documented_mix", "4800 and 5600 MT/s"),
            package_construction: scope("exact", "single-die FBGA"),
            qualification: None,
        }),
        nand: None,
        cell: None,
        direct: None,
        notes: None,
    }
}

/// A complete direct contract price of one DDR5 16Gb x8 part.
fn ddr5_direct() -> SeriesDefinition {
    let mut series = ddr5_assessment();
    series.series_id = Some("example-oem-ddr5-16gb-x8-5600-1k".into());
    series.evidence_class = Some("direct_price".into());
    series.origin.row = Some("MT60B2G8HB-56B:A, 1k-5k units".into());
    series.procurement_segment = scope("exact", "distribution contract, 1k-5k units");
    series.geography = scope("exact", "US");
    if let Some(dram) = series.dram.as_mut() {
        dram.speed = scope("exact", "5600 MT/s");
    }
    series.direct = Some(DirectTerms {
        manufacturer: Some("Micron".into()),
        mpn: Some("MT60B2G8HB-56B:A".into()),
        specification_reference: Some("https://example.com/datasheet".into()),
        quantity_tier: Some("1k-5k".into()),
        delivery_terms: Some("FOB, 4-week lead".into()),
        pricing_period: Some("calendar month".into()),
        market: Some("US distribution".into()),
        tax_freight: Some("ex tax, ex freight".into()),
        payment_terms: Some("net 30, no rebate".into()),
    });
    series
}

/// A complete 1Tb TLC fabricated-wafer contract assessment.
fn tlc_wafer_assessment() -> SeriesDefinition {
    SeriesDefinition {
        schema: SERIES_SCHEMA.into(),
        series_id: Some("example-wafer-1tb-tlc-avg".into()),
        definition_version: Some(1),
        bucket: Some("tlc_1tb".into()),
        evidence_class: Some("published_assessment".into()),
        price_basis: Some("contract".into()),
        origin: Origin {
            publisher: Some("Example Research".into()),
            locator: Some("https://example.com/price/flash/wafer_contract".into()),
            row: Some("1Tb TLC".into()),
        },
        scope_reference: None,
        physical_scope: Some("fabricated_wafer".into()),
        native_unit: Some("USD per unit as the assessment quotes it".into()),
        statistic: Some("average".into()),
        currency: Some("USD".into()),
        conversion_policy: None,
        procurement_segment: scope("documented_mix", "wafer buyers surveyed by the publisher"),
        geography: Some(ScopeValue::state("not_disclosed")),
        reference_period: Some("month".into()),
        cadence: Some("monthly".into()),
        revision_policy: Some("corrections replace the release".into()),
        missing_data_policy: Some("carry forward".into()),
        dram: None,
        nand: Some(NandScope {
            architecture: Some("3D".into()),
            cell_type: Some("TLC".into()),
            die_density: Some("1Tb".into()),
            testing_scope: scope(
                "documented_mix",
                "as the assessment defines its wafer quality",
            ),
            layer_scope: None,
        }),
        cell: None,
        direct: None,
        notes: None,
    }
}

fn committed_month<'a>(product: Product, sources: &'a [MonthSeries<'a>]) -> MonthContext<'a> {
    MonthContext {
        product,
        stage: Stage::Committed,
        sources,
    }
}

fn observation(series: &SeriesDefinition) -> Observation {
    Observation {
        schema: OBSERVATION_SCHEMA.into(),
        observation_id: Some("obs-2026-11".into()),
        series_id: series.series_id.clone(),
        series_definition_version: series.definition_version,
        raw_product_label: series.origin.row.clone(),
        raw_price: Some("5.25".into()),
        raw_currency: series.currency.clone(),
        raw_unit: series.native_unit.clone(),
        selected_statistic: series.statistic.clone(),
        reference_period_start: Some("2026-11-01".into()),
        reference_period_end: Some("2026-11-30".into()),
        published_at: Some("2026-11-30".into()),
        retrieved_at: Some("2026-12-01T09:00:00Z".into()),
        publication_vintage: Some("first release".into()),
        original_evidence_locator: series.origin.locator.clone(),
        release_id: Some("2026-11".into()),
        evidence_reference: Some(
            "https://web.archive.org/web/20261201090000/https://example.com/".into(),
        ),
        ..Observation::default()
    }
}

fn observe(
    series: &SeriesDefinition,
    observation: &Observation,
    prior: &[PriorObservation<'_>],
) -> Routing {
    route_observation(
        series,
        observation,
        &ObservationContext {
            product: Product::from_symbol(if series.nand.is_some() {
                "NANDX"
            } else {
                "RAMX"
            })
            .unwrap(),
            committed: true,
            prior,
            expiry: Some("2027-01-01"),
        },
    )
}

// The spec's `illustrative_cases`, one test each.

#[test]
fn ramx_direct_match() {
    let series = ddr5_direct();
    assert_eq!(
        route_series(&series, Product::Ramx).status,
        Status::ClassifiedCandidate,
        "{:?}",
        route_series(&series, Product::Ramx)
    );
    let sources = [MonthSeries {
        source_id: "S1",
        definition: Some(&series),
        committed: true,
    }];
    let routing = route_series_in_month(
        &series,
        &committed_month(Product::Ramx, &sources),
        Some("S1"),
    );
    assert_eq!(routing.status, Status::EligibleForContract, "{routing:?}");
}

#[test]
fn ramx_other_speed() {
    let admitted = ddr5_direct();
    let mut other = ddr5_direct();
    other.series_id = Some("example-oem-ddr5-16gb-x8-4800-1k".into());
    other.origin.row = Some("MT60B2G8HB-48B:A, 1k-5k units".into());
    other.direct.as_mut().unwrap().mpn = Some("MT60B2G8HB-48B:A".into());
    other.dram.as_mut().unwrap().speed = scope("exact", "4800 MT/s");
    let sources = [MonthSeries {
        source_id: "S1",
        definition: Some(&admitted),
        committed: false,
    }];
    let month = MonthContext {
        product: Product::Ramx,
        stage: Stage::Open,
        sources: &sources,
    };
    let routing = route_series_in_month(&other, &month, None);
    assert_eq!(routing.status, Status::ClassifiedCandidate, "{routing:?}");
}

#[test]
fn ramx_stacked_package() {
    let mut series = ddr5_assessment();
    series.bucket = Some("DDR5_32GBIT".into());
    series.dram.as_mut().unwrap().die_density = Some("16Gb".into());
    series.dram.as_mut().unwrap().die_count = Some(2);
    let routing = route_series(&series, Product::Ramx);
    assert_eq!(routing.status, Status::WrongProduct);
    assert!(routing
        .deciding()
        .any(|reason| reason.field == "dram.dieCount"));
}

#[test]
fn ramx_module() {
    let mut series = ddr5_assessment();
    series.physical_scope = Some("module".into());
    series.origin.row = Some("DDR5 16GB UDIMM".into());
    let routing = route_series(&series, Product::Ramx);
    assert_eq!(routing.status, Status::WrongProduct);
    assert!(routing
        .deciding()
        .any(|reason| reason.field == "physicalScope"));
}

#[test]
fn ramx_grade_change() {
    let standard = ddr5_direct();
    let mut ett = ddr5_direct();
    ett.definition_version = Some(2);
    ett.dram.as_mut().unwrap().grade = Some("ett".into());
    let sources = [MonthSeries {
        source_id: "S1",
        definition: Some(&standard),
        committed: true,
    }];
    let routing = route_series_in_month(&ett, &committed_month(Product::Ramx, &sources), None);
    assert_eq!(routing.status, Status::ScopeChange, "{routing:?}");
    // On its own, eTT material is simply not the product.
    assert_eq!(
        route_series(&ett, Product::Ramx).status,
        Status::WrongProduct
    );
}

#[test]
fn nand_assessment_match() {
    let series = tlc_wafer_assessment();
    let sources = [MonthSeries {
        source_id: "N1",
        definition: Some(&series),
        committed: true,
    }];
    let routing = route_series_in_month(
        &series,
        &committed_month(Product::Nandx, &sources),
        Some("N1"),
    );
    assert_eq!(routing.status, Status::EligibleForContract, "{routing:?}");
}

#[test]
fn nand_mixed_layers() {
    let mut series = tlc_wafer_assessment();
    series.nand.as_mut().unwrap().layer_scope = scope("documented_mix", "176, 218 and 232 layers");
    assert_eq!(
        route_series(&series, Product::Nandx).status,
        Status::ClassifiedCandidate
    );
}

#[test]
fn nand_packaged_quote() {
    let mut series = tlc_wafer_assessment();
    series.physical_scope = Some("packaged_ic".into());
    assert_eq!(
        route_series(&series, Product::Nandx).status,
        Status::ResearchOnly
    );
    let mut direct = tlc_wafer_assessment();
    direct.evidence_class = Some("direct_price".into());
    assert_eq!(
        route_series(&direct, Product::Nandx).status,
        Status::ResearchOnly
    );
}

#[test]
fn spot_as_contract() {
    let mut series = tlc_wafer_assessment();
    series.price_basis = Some("spot".into());
    assert_eq!(
        route_series(&series, Product::Nandx).status,
        Status::WrongPriceBasis
    );
    let mut listing = ddr5_direct();
    listing.price_basis = Some("listing".into());
    assert_eq!(
        route_series(&listing, Product::Ramx).status,
        Status::WrongPriceBasis
    );
}

#[test]
fn unknown_unit() {
    let mut series = ddr5_assessment();
    series.native_unit = None;
    let routing = route_series(&series, Product::Ramx);
    assert_eq!(routing.status, Status::NeedsMetadata);
    assert!(routing
        .deciding()
        .any(|reason| reason.field == "nativeUnit"));
    let series = ddr5_assessment();
    let mut observed = observation(&series);
    observed.raw_unit = None;
    assert_eq!(
        observe(&series, &observed, &[]).status,
        Status::NeedsMetadata
    );
}

#[test]
fn high_low_switch() {
    let series = ddr5_assessment();
    let mut observed = observation(&series);
    observed.selected_statistic = Some("high".into());
    let routing = observe(&series, &observed, &[]);
    assert_eq!(routing.status, Status::ScopeChange, "{routing:?}");
}

#[test]
fn duplicate_parent() {
    let original = ddr5_assessment();
    let mut repost = ddr5_assessment();
    repost.series_id = Some("someone-elses-screenshot".into());
    repost.origin.locator = Some("https://www.example.com/price/dram/contract/".into());
    let sources = [MonthSeries {
        source_id: "S1",
        definition: Some(&original),
        committed: false,
    }];
    let month = MonthContext {
        product: Product::Ramx,
        stage: Stage::Open,
        sources: &sources,
    };
    assert_eq!(
        route_series_in_month(&repost, &month, None).status,
        Status::DuplicateNoExtraWeight
    );
    // The same series proposed twice is a duplicate too.
    assert_eq!(
        route_series_in_month(&original, &month, None).status,
        Status::DuplicateNoExtraWeight
    );
    // The original itself stays a candidate.
    assert_eq!(
        route_series_in_month(&original, &month, Some("S1")).status,
        Status::ClassifiedCandidate
    );
}

#[test]
fn new_source_current_contract() {
    let committed = ddr5_assessment();
    let mut newcomer = ddr5_assessment();
    newcomer.series_id = Some("another-publisher-ddr5-16gb".into());
    newcomer.origin.publisher = Some("Another Research".into());
    newcomer.origin.locator = Some("https://another.example/dram".into());
    let sources = [MonthSeries {
        source_id: "S1",
        definition: Some(&committed),
        committed: true,
    }];
    let routing = route_series_in_month(&newcomer, &committed_month(Product::Ramx, &sources), None);
    assert_eq!(
        routing.status,
        Status::OutsideCommittedSeries,
        "{routing:?}"
    );
}

#[test]
fn monthly_recapture() {
    let series = ddr5_assessment();
    let mut recapture = observation(&series);
    recapture.observation_id = Some("obs-2026-11-again".into());
    recapture.retrieved_at = Some("2026-12-02T09:00:00Z".into());
    let prior = [PriorObservation {
        release_id: "2026-11",
        reference_period_end: "2026-11-30",
    }];
    assert_eq!(
        observe(&series, &recapture, &prior).status,
        Status::DuplicateNoExtraWeight
    );
}

// Beyond the spec's cases.

#[test]
fn complete_observations_count_and_old_ones_are_stale() {
    let series = ddr5_assessment();
    let observed = observation(&series);
    assert_eq!(
        observe(&series, &observed, &[]).status,
        Status::EligibleForContract,
        "{:?}",
        observe(&series, &observed, &[])
    );
    let prior = [PriorObservation {
        release_id: "2026-12",
        reference_period_end: "2026-12-31",
    }];
    assert_eq!(
        observe(&series, &observed, &prior).status,
        Status::StaleOrWrongPeriod
    );
    let mut early = observation(&series);
    early.retrieved_at = Some("2026-11-29".into());
    assert_eq!(
        observe(&series, &early, &[]).status,
        Status::StaleOrWrongPeriod
    );
    let mut late = observation(&series);
    late.reference_period_start = Some("2027-01-01".into());
    late.reference_period_end = Some("2027-01-31".into());
    assert_eq!(
        observe(&series, &late, &[]).status,
        Status::StaleOrWrongPeriod
    );
}

#[test]
fn unknown_condition_and_grade_are_not_assumed() {
    let mut series = ddr5_assessment();
    series.dram.as_mut().unwrap().condition = Some("unknown".into());
    series.dram.as_mut().unwrap().grade = None;
    let routing = route_series(&series, Product::Ramx);
    assert_eq!(routing.status, Status::NeedsMetadata);
    let fields: Vec<&str> = routing
        .deciding()
        .map(|reason| reason.field.as_str())
        .collect();
    assert_eq!(fields, ["dram.condition", "dram.grade"]);
}

#[test]
fn a_wrong_bucket_or_product_is_named() {
    let series = ddr5_assessment();
    let routing = route_series(&series, Product::Nandx);
    assert_eq!(routing.status, Status::WrongProduct);
    let mut lpddr = ddr5_assessment();
    lpddr.dram.as_mut().unwrap().memory_family = Some("lpddr".into());
    assert_eq!(
        route_series(&lpddr, Product::Ramx).status,
        Status::WrongProduct
    );
    let mut ddr4 = ddr5_assessment();
    ddr4.dram.as_mut().unwrap().generation = Some("DDR4".into());
    assert_eq!(
        route_series(&ddr4, Product::Ramx).status,
        Status::WrongProduct
    );
    let mut qlc = tlc_wafer_assessment();
    qlc.nand.as_mut().unwrap().cell_type = Some("QLC".into());
    assert_eq!(
        route_series(&qlc, Product::Nandx).status,
        Status::WrongProduct
    );
    let mut ssd = tlc_wafer_assessment();
    ssd.physical_scope = Some("ssd".into());
    assert_eq!(
        route_series(&ssd, Product::Nandx).status,
        Status::WrongProduct
    );
}

#[test]
fn direct_prices_need_their_identity_and_assessments_do_not() {
    let mut direct = ddr5_direct();
    direct.direct.as_mut().unwrap().mpn = None;
    direct.dram.as_mut().unwrap().speed = scope("documented_mix", "4800 and 5600 MT/s");
    let routing = route_series(&direct, Product::Ramx);
    assert_eq!(routing.status, Status::NeedsMetadata);
    let fields: Vec<&str> = routing
        .deciding()
        .map(|reason| reason.field.as_str())
        .collect();
    assert!(
        fields.contains(&"direct.mpn") && fields.contains(&"dram.speed"),
        "{fields:?}"
    );
    // An assessment inherits its scope: no MPN, a documented mix, an undisclosed geography.
    let mut assessment = ddr5_assessment();
    assessment.geography = Some(ScopeValue::state("not_disclosed"));
    assert_eq!(
        route_series(&assessment, Product::Ramx).status,
        Status::ClassifiedCandidate
    );
    // A documented mix is affirmative; missing is not a mix.
    assessment.dram.as_mut().unwrap().organization = Some(ScopeValue::state("not_disclosed"));
    assert_eq!(
        route_series(&assessment, Product::Ramx).status,
        Status::NeedsMetadata
    );
}

#[test]
fn templates_name_every_field_and_route_as_missing_details() {
    for product in Product::ALL {
        for bucket in product.buckets() {
            for direct in [false, true] {
                let template = series_template(bucket, direct);
                let text = template.to_string();
                let series = SeriesDefinition::parse(&text).expect("a template parses");
                assert_eq!(series.bucket_in(product), Some(bucket));
                let routing = route_series(&series, product);
                let expected = if direct && product == Product::Nandx {
                    Status::ResearchOnly
                } else {
                    Status::NeedsMetadata
                };
                assert_eq!(routing.status, expected, "{} {direct}", bucket.id);
            }
        }
    }
    let template = observation_template(&ddr5_direct()).to_string();
    let observation = Observation::parse(&template).expect("an observation template parses");
    assert_eq!(observation.mpn.as_deref(), Some("MT60B2G8HB-56B:A"));
}

#[test]
fn definitions_round_trip_compactly_and_fit_a_source() {
    for series in [ddr5_assessment(), ddr5_direct(), tlc_wafer_assessment()] {
        let text = series.definition_text();
        assert!(
            text.len() <= MAX_DEFINITION_TEXT_BYTES,
            "{} bytes",
            text.len()
        );
        assert!(!text.contains("null"));
        assert_eq!(SeriesDefinition::parse(&text).unwrap(), series);
        assert!(SeriesDefinition::claims_schema(&text));
    }
    assert!(!SeriesDefinition::claims_schema(
        "Product: a module; Price field: latest"
    ));
    assert!(
        SeriesDefinition::parse(r#"{"schema":"amoeba.comparison-series.v1","colour":"red"}"#)
            .is_err()
    );
}

#[test]
fn series_sources_name_their_evidence_class() {
    assert_eq!(
        ddr5_assessment().source_type(),
        "comparison-series/v1:published_assessment"
    );
    assert_eq!(
        ddr5_direct().source_type(),
        "comparison-series/v1:direct_price"
    );
}

#[test]
fn an_outside_reason_decides_only_when_it_outranks() {
    // A source registered under another bucket than its series names: wrong product.
    let mut routing = route_series(&ddr5_assessment(), Product::Ramx);
    assert_eq!(routing.status, Status::ClassifiedCandidate);
    routing.push(
        Status::WrongProduct,
        "bucket",
        "registered under DDR4_8GBIT",
    );
    assert_eq!(routing.status, Status::WrongProduct);
    assert_eq!(routing.deciding().count(), 1);
    // A weaker reason is kept but doesn't decide.
    routing.push(
        Status::OutsideCommittedSeries,
        "expiryId",
        "a part-number month",
    );
    assert_eq!(routing.status, Status::WrongProduct);
    assert_eq!(routing.reasons.len(), 2);
    let only = Routing::only(
        Status::NeedsMetadata,
        "definition",
        "not a comparison series",
    );
    assert_eq!(only.status, Status::NeedsMetadata);
    assert_eq!(only.deciding().count(), 1);
}

#[test]
fn an_observation_record_signs_fixed_bytes() {
    let series = ddr5_assessment();
    let observed = observation(&series);
    let bytes = observation_record_message(
        "ramx",
        "RAMX-202612",
        "ab".repeat(32).as_str(),
        "Claim1111",
        &observed,
    );
    let text = String::from_utf8(bytes.clone()).unwrap();
    let (domain, json) = text.split_at(OBSERVATION_RECORD_DOMAIN.len());
    assert_eq!(domain, "amoeba-observation-record-v1\n");
    assert!(
        json.starts_with(r#"{"marketId":"ramx","expiryId":"RAMX-202612","sourceId":"abab"#),
        "{json}"
    );
    assert!(
        json.contains(
            r#","claim":"Claim1111","observation":{"schema":"amoeba.series-observation.v1""#
        ),
        "{json}"
    );
    assert!(!json.contains("null"));
    // The same record gives the same bytes; any field changes them.
    assert_eq!(
        bytes,
        observation_record_message(
            "ramx",
            "RAMX-202612",
            &"ab".repeat(32),
            "Claim1111",
            &observed
        )
    );
    let mut other = observed.clone();
    other.raw_price = Some("5.26".into());
    assert_ne!(
        bytes,
        observation_record_message("ramx", "RAMX-202612", &"ab".repeat(32), "Claim1111", &other)
    );
}

#[test]
fn prices_scale_to_six_decimals() {
    assert_eq!(price_atoms("12.5"), Some(12_500_000));
    assert_eq!(price_atoms("0.000001"), Some(1));
    assert_eq!(price_atoms("3"), Some(3_000_000));
    for bad in ["0", "0.0", "-1", "1.2345678", "1e3", "", ".5", "1,000"] {
        assert_eq!(price_atoms(bad), None, "{bad}");
    }
}

#[test]
fn statuses_use_the_specs_names() {
    let spec = spec();
    let names: Vec<&str> = spec["routing"]["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|status| status.as_str().unwrap())
        .collect();
    let mut ours: Vec<&str> = Status::ALL.iter().map(|status| status.as_str()).collect();
    let mut theirs = names.clone();
    ours.sort();
    theirs.sort();
    assert_eq!(ours, theirs);
    for status in Status::ALL {
        assert_eq!(Status::parse(status.as_str()), Some(status));
        assert_eq!(serde_json::to_value(status).unwrap(), status.as_str());
    }
}

#[test]
fn ddr5_32gb_is_a_research_candidate_of_ramx() {
    let mut series = ddr5_assessment();
    series.bucket = Some("DDR5_32GBIT".into());
    series.dram.as_mut().unwrap().die_density = Some("32Gb".into());
    let routing = route_series(&series, Product::Ramx);
    assert_eq!(routing.status, Status::ResearchOnly, "{routing:?}");
    assert!(routing
        .deciding()
        .any(|reason| reason.field == "bucket" && reason.message.contains("research candidate")));
    // Still checked against the 32Gb identity: a 16Gb die is the wrong product.
    series.dram.as_mut().unwrap().die_density = Some("16Gb".into());
    assert_eq!(
        route_series(&series, Product::Ramx).status,
        Status::WrongProduct
    );
    assert!(Product::Ramx.bucket("DDR5_32GBIT").is_none());
    assert_eq!(
        Product::Ramx.manifest().sorted_labels(),
        ["DDR4_16GBIT", "DDR4_8GBIT", "DDR5_16GBIT", "DDR5_24GBIT"]
    );
}
