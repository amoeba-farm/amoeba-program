//! The channel molds (`governance/channel_molds.v1.json`) against the brief
//! (`oracle/docs/lta-spec.md`), the bucket tables and `ameba_oracle_heads::channels`.

use ameba_oracle_heads::channels as heads;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::molds::*;
use super::{Product, BATX_FIRST_EXPIRY};

/// The listing terms documents pinned below are for the months expiring 2026-12-01, the first
/// all three products list.
const EXPIRY: i64 = BATX_FIRST_EXPIRY;

/// `(product, listing terms document SHA-256 at EXPIRY, SHA-256 over its sentence hashes in
/// component order)`. Computed independently of this crate (Python's hashlib and json).
const PINNED: [(Product, &str, &str); 3] = [
    (
        Product::Ramx,
        "952068c17a4bdcc77f9270f25c6065292ec4166e4b90c61f77e8dc300698da7b",
        "2208a7313fa72575e5445ae682763d3fdf88b992d57be9b78f41a0fc525dfcab",
    ),
    (
        Product::Nandx,
        "25863149d199114a9254b6d77910540998b89dbee128fb799b800e99ba6363a8",
        "2630a981b3301505f474822cef18c75ea6bfd02f88a369447167c9d4abdf1f26",
    ),
    (
        Product::Batx,
        "1abb31710f559120a102fe663d5540b5b0cf6ffc53492c0e3b99fc795c1cb18f",
        "e88b760334ce454d73ba39a35e21ce20467605323d6446b79f8b453e50a3c0b0",
    ),
];

fn reference_sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// The clauses of the brief's templates that do not define a channel, dropped from the molds.
const DROPPED_CLAUSES: [&str; 5] = [
    ", lot size ≥ [N]",
    ", delivery within [D days]",
    ", volume tier [X]",
    ", [delivery basis]",
    ", [payment terms]",
];

/// A brief template without [`DROPPED_CLAUSES`].
fn slimmed(template: &str) -> String {
    DROPPED_CLAUSES
        .iter()
        .fold(template.to_string(), |t, clause| t.replace(clause, ""))
}

fn raw() -> Value {
    serde_json::from_str(CHANNEL_MOLDS_JSON).expect("the molds are JSON")
}

#[test]
fn the_molds_are_the_pinned_document() {
    assert_eq!(
        hex32(&reference_sha256(CHANNEL_MOLDS_JSON.as_bytes())),
        CHANNEL_MOLDS_SHA256_HEX
    );
    assert_eq!(raw()["version"], CHANNEL_MOLDS_VERSION);
    assert_eq!(molds().version, CHANNEL_MOLDS_VERSION);
    // It implements the brief, which is in the repository byte for byte.
    assert_eq!(
        raw()["brief"]["sha256"],
        hex32(&reference_sha256(LTA_BRIEF_MD.as_bytes()))
    );
}

#[test]
fn sha256_is_sha256() {
    assert_eq!(
        hex32(&sha256(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hex32(&sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // Every padding case (lengths around the 55/56/64-byte boundaries and several blocks).
    let data: Vec<u8> = (0..1_000u32).map(|i| (i * 7 + i / 13) as u8).collect();
    for length in 0..=300 {
        assert_eq!(
            sha256(&data[..length]),
            reference_sha256(&data[..length]),
            "{length}"
        );
    }
    assert_eq!(sha256(&data), reference_sha256(&data));
}

#[test]
fn every_product_has_its_buckets_in_three_channels() {
    let m = molds();
    assert_eq!(
        m.products.iter().map(|p| p.product).collect::<Vec<_>>(),
        Product::ALL
    );
    for product in Product::ALL {
        let mold = product.mold();
        assert_eq!(mold.items.len(), product.buckets().len());
        for (index, (item, bucket)) in mold.items.iter().zip(product.buckets()).enumerate() {
            assert_eq!(item.bucket, bucket);
            assert_eq!(item.basket_weight_bps, bucket.weight_bps);
            assert_eq!(
                item.mix_bps.iter().map(|&m| u32::from(m)).sum::<u32>(),
                10_000
            );
            assert!(heads::mix_is_valid(&item.mix_bps));
            // Placeholders for dev: every mix is an assumed scenario.
            assert_eq!(item.mix_basis, MixBasis::Assumed, "{}", bucket.id);
            for (c, channel) in item.components.iter().zip(Channel::ALL) {
                assert_eq!(c.channel, channel);
                assert_eq!(usize::from(c.item), index);
                assert_eq!(usize::from(c.index), index * 3 + channel.index());
                assert_eq!(c.product, product);
                assert!(!c.sentence.contains(['[', ']']), "{}", c.sentence);
                assert_eq!(c.sentence_sha256, reference_sha256(c.sentence.as_bytes()));
                assert!(c.sentence.len() <= SEALED_OBJECT_MAX_BYTES);
                assert_eq!(
                    fill_template(
                        &mold.channel(channel).template,
                        &c.terms.iter().cloned().collect()
                    )
                    .unwrap(),
                    c.sentence
                );
                assert_eq!(mold.component(c.index), Some(c));
            }
        }
        // Every product settles on the fixed-mix reference (the user, 2026-10-29: "SPOT LTA
        // AND CONTRACT NEED TO BE WEIGHTED AGAINST EACHOTHER").
        assert_eq!(mold.settlement_target, SettlementTarget::FixedMixReference);
        // The core lists it on these mixes: valid, on an assumed basis.
        let mixes: Vec<(heads::Mix, heads::MixBasis)> = mold
            .items
            .iter()
            .map(|i| {
                let basis = heads::MixBasis::from_code(i.mix_basis.code()).unwrap();
                (i.mix_bps, basis)
            })
            .collect();
        let target = heads::SettlementTarget::from_code(mold.settlement_target.code()).unwrap();
        assert!(heads::listing_allowed(target, &mixes));
        // Every channel of every item settles: each mix gives every channel a share.
        assert!(mold.items.iter().all(|i| heads::Channel::ALL
            .iter()
            .all(|&c| heads::settles(target, c, &i.mix_bps))));
        for channel in Channel::ALL {
            let cm = mold.channel(channel);
            assert!(!cm.evidence.is_empty() && !cm.classification.is_empty());
        }
    }
}

#[test]
fn memory_molds_use_the_briefs_templates_slimmed() {
    // The brief's section 3.2 templates, quoted ("> ") in the brief itself, without the terms
    // that do not define a channel (the user, 2026-10-29, on "bracketed terms per product are
    // too much listing work": "agreed just have challenges deal w some of that stuff").
    let quoted: Vec<&str> = LTA_BRIEF_MD
        .lines()
        .filter_map(|line| line.strip_prefix("> "))
        .collect();
    for product in [Product::Ramx, Product::Nandx] {
        for channel in Channel::ALL {
            let template = &product.mold().channel(channel).template;
            assert!(
                quoted.iter().any(|q| slimmed(q) == *template),
                "{} {}",
                product.symbol(),
                channel.id()
            );
            for clause in DROPPED_CLAUSES {
                assert!(!template.contains(&clause[2..]), "{template}");
            }
        }
        assert_eq!(
            product.mold().disclosure,
            "This contract settles on a fixed-mix procurement reference: each item's spot, \
             periodic-contract and long-term-agreement prices, weighted by the mix published \
             with this listing (an assumed scenario until matched evidence backs it)."
        );
    }
    // The evidence table of section 5, row for row, less the lot size the sentence dropped.
    let memory = Product::Ramx.mold();
    assert_eq!(
        memory.channel(Channel::Spot).evidence,
        ["price", "availability and delivery", "as-of date", "venue"]
    );
    assert!(memory
        .channel(Channel::Lta)
        .evidence
        .iter()
        .any(|row| row == "next reset"));
    assert_eq!(
        Product::Nandx.mold().channels,
        memory.channels,
        "RAMX and NANDX share the brief's mold"
    );
}

#[test]
fn every_term_is_filled_and_none_is_left_standard() {
    for product in Product::ALL {
        for c in product.mold().components() {
            let names: Vec<&str> = c.terms.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(
                names,
                template_placeholders(&product.mold().channel(c.channel).template)
                    .unwrap()
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            );
            for (name, value) in &c.terms {
                assert!(!value.trim().is_empty(), "{name}");
                assert!(!value.eq_ignore_ascii_case("standard"), "{name}");
            }
        }
    }
    // The terms the memory sentences carry.
    let ddr5 = Product::Ramx.mold().item("DDR5_16GBIT").unwrap();
    let contract = ddr5.component(Channel::Contract);
    let term = |name: &str| {
        contract
            .terms
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(term("Currency"), Some("USD"));
    assert_eq!(term("unit"), Some("chip"));
    assert_eq!(term("period"), Some("month"));
    assert_eq!(
        contract.sentence,
        "USD per chip, new standard-production single-die packaged DDR5 16Gb SDRAM, supplier → \
         OEM/ODM purchase not under an LTA, delivered during the observation month, price fixed \
         during this negotiation period."
    );
    // Only the terms that define the channel: no lot size, delivery days, delivery basis,
    // volume tier or payment terms anywhere.
    for product in Product::ALL {
        for c in product.mold().components() {
            for (name, _) in &c.terms {
                assert!(
                    ["Currency", "unit", "product", "period", "price adjustment"]
                        .contains(&name.as_str()),
                    "{} {name}",
                    product.symbol()
                );
            }
        }
    }
    assert_eq!(
        contract.sentence_sha256_hex(),
        raw()["products"][0]["items"][0]["sentences"]["contract"]["sha256"]
    );
}

#[test]
fn observation_periods_are_monthly_except_where_prices_fix_quarterly() {
    let period = |product: Product, bucket: &str| {
        product
            .mold()
            .item(bucket)
            .unwrap()
            .component(Channel::Contract)
            .terms
            .iter()
            .find(|(n, _)| n == "period")
            .map(|(_, v)| v.clone())
            .unwrap()
    };
    let quarterly = [
        (Product::Ramx, "DDR5_24GBIT"),
        (Product::Nandx, "qlc_1tb"),
        (Product::Batx, "LFP_EV"),
        (Product::Batx, "TERNARY_PRISM_EV"),
        (Product::Batx, "TERNARY_POUCH_EV"),
    ];
    for product in Product::ALL {
        for bucket in product.buckets() {
            let expected = if quarterly.contains(&(product, bucket.id)) {
                "quarter"
            } else {
                "month"
            };
            assert_eq!(period(product, bucket.id), expected, "{}", bucket.id);
        }
    }
}

#[test]
fn batx_has_its_own_battery_mold() {
    let mold = Product::Batx.mold();
    assert_ne!(mold.channels, Product::Ramx.mold().channels);
    assert_eq!(
        mold.disclosure,
        "This contract settles on a fixed-mix procurement reference for battery cells: each \
         item's spot, periodic-contract and long-term-agreement prices, weighted by the mix \
         published with this listing (an assumed scenario until matched evidence backs it)."
    );
    for item in &mold.items {
        let [spot, contract, lta] = &item.components;
        assert!(spot
            .sentence
            .starts_with("CNY per Wh of rated cell energy, new "));
        assert!(spot.sentence.contains("China domestic market"));
        assert!(contract
            .sentence
            .contains("cell maker → vehicle OEM or energy-storage integrator"));
        // The LTA sentence names the agreements' price adjustment: the metal index, the reset
        // cadence and the band.
        assert!(
            lta.sentence.contains("lithium carbonate"),
            "{}",
            lta.sentence
        );
        assert!(lta.sentence.contains("reset monthly or quarterly"));
        assert!(lta.sentence.contains("within any agreed band"));
        assert!(lta
            .sentence
            .contains("the price formula with the index readings"));
        let ternary = item.bucket.chemistry == Some("nickel_ternary");
        assert_eq!(lta.sentence.contains("nickel sulfate"), ternary);
        assert_eq!(lta.sentence.contains("cobalt sulfate"), ternary);
    }
    assert!(mold
        .channel(Channel::Lta)
        .evidence
        .iter()
        .any(|row| row.contains("price formula")));
}

#[test]
fn the_placeholder_mixes_are_near_the_users_shape() {
    // Basket-weighted, in percent: RAMX ~10/50/40, NANDX ~15/55/30, BATX ~10/30/60.
    let targets = [
        (Product::Ramx, [10, 50, 40]),
        (Product::Nandx, [15, 55, 30]),
        (Product::Batx, [10, 30, 60]),
    ];
    for (product, target) in targets {
        let mut blended = [0u64; 3];
        for item in &product.mold().items {
            for c in 0..3 {
                blended[c] += u64::from(item.basket_weight_bps) * u64::from(item.mix_bps[c]);
            }
        }
        for c in 0..3 {
            let percent = blended[c] as f64 / 1e6;
            assert!(
                (percent - target[c] as f64).abs() <= 3.0,
                "{} {c}: {percent}",
                product.symbol()
            );
        }
    }
}

#[test]
fn the_classification_examples_classify_as_documented() {
    let examples = &molds().classification_examples;
    // The brief's five (section 8.H) are among them.
    let channels: Vec<Option<Channel>> = examples.iter().map(|e| e.channel).collect();
    for expected in [
        Some(Channel::Lta),
        Some(Channel::Contract),
        Some(Channel::Spot),
        None,
    ] {
        assert!(channels.contains(&expected));
    }
    assert!(examples.iter().any(|e| e.case.contains("newly signed")));
    assert!(examples
        .iter()
        .any(|e| e.case.contains("reset under an LTA")));
    assert!(examples.iter().any(|e| e.case.contains("unterminated")));
    for e in examples {
        let facts = heads::PurchaseFacts {
            agreed: e.facts.agreed,
            agreement_months: e.facts.agreement_months,
            committed_volume: e.facts.committed_volume,
            negotiated_wholesale: e.facts.negotiated_wholesale,
            delivered_in_period: e.facts.delivered_in_period,
            immediate_inventory: e.facts.immediate_inventory,
        };
        let got = heads::classify(&facts).map(|c| c.code());
        assert_eq!(got, e.channel.map(Channel::code), "{}", e.case);
    }
}

#[test]
fn codes_match_the_heads_crate() {
    for channel in Channel::ALL {
        let h = heads::Channel::from_code(channel.code()).unwrap();
        assert_eq!(h.index(), channel.index());
        assert_eq!(Channel::from_code(channel.code()), Some(channel));
        assert_eq!(Channel::from_id(channel.id()), Some(channel));
    }
    assert_eq!(Channel::Spot.name(), "Spot");
    assert_eq!(Channel::Lta.name(), "LTA");
    for basis in MixBasis::ALL {
        assert_eq!(
            heads::MixBasis::from_code(basis.code()).map(heads::MixBasis::code),
            Some(basis.code())
        );
    }
    assert_eq!(
        heads::SettlementTarget::ContractChannel.code(),
        SettlementTarget::ContractChannel.code()
    );
    assert_eq!(
        heads::SettlementTarget::FixedMixReference.code(),
        SettlementTarget::FixedMixReference.code()
    );
    let reasons = &molds().challenge_reasons;
    assert_eq!(reasons.len(), 4);
    for r in reasons {
        assert_eq!(
            heads::ChallengeReason::from_code(r.code).unwrap().code(),
            r.code
        );
    }
}

#[test]
fn the_listing_terms_document_is_pinned_and_fits_a_sealed_object() {
    let mut sentence_hashes = std::collections::BTreeSet::new();
    for (product, document_hex, sentences_hex) in PINNED {
        let document = listing_terms_document(product, EXPIRY);
        assert_eq!(document, listing_terms_document(product, EXPIRY));
        assert_eq!(hex32(&reference_sha256(&document)), document_hex);
        assert_eq!(hex32(&listing_terms_sha256(product, EXPIRY)), document_hex);
        let mut hasher = Sha256::new();
        for c in product.mold().components() {
            hasher.update(c.sentence_sha256);
            assert!(
                sentence_hashes.insert(c.sentence_sha256),
                "sentences are distinct"
            );
        }
        assert_eq!(hex32(&hasher.finalize().into()), sentences_hex);
        // One kind-2 object holds it, at any expiry.
        assert!(listing_terms_document(product, i64::MIN).len() <= SEALED_OBJECT_MAX_BYTES);
        // Its content: the domain, then compact JSON naming everything the brief lists.
        let json = document
            .strip_prefix(LISTING_TERMS_DOMAIN.as_bytes())
            .expect("the domain comes first");
        let value: Value = serde_json::from_slice(json).unwrap();
        assert!(!json.contains(&b'\n'), "compact JSON on one line");
        assert_eq!(value["schema"], LISTING_TERMS_SCHEMA);
        assert_eq!(value["moldsSha256"], CHANNEL_MOLDS_SHA256_HEX);
        assert_eq!(value["product"], product.symbol());
        assert_eq!(value["expiryTs"], EXPIRY);
        assert_eq!(value["quarter"], "2026-Q4");
        assert_eq!(
            value["mixCommitment"],
            hex32(&listing_mix_commitment(product, EXPIRY))
        );
        assert_eq!(value["settlementTarget"], "fixed_mix_reference");
        assert_eq!(value["disclosure"], product.mold().disclosure.as_str());
        assert_eq!(value["mixCitation"], product.mold().mix_citation.as_str());
        let items = value["items"].as_array().unwrap();
        for (item, mold_item) in items.iter().zip(&product.mold().items) {
            assert_eq!(item["bucket"], mold_item.bucket.id);
            assert_eq!(item["basketWeightBps"], mold_item.basket_weight_bps);
            assert_eq!(item["mixBasis"], "assumed");
            for (c, component) in item["components"]
                .as_array()
                .unwrap()
                .iter()
                .zip(&mold_item.components)
            {
                assert_eq!(c["sentenceSha256"], component.sentence_sha256_hex());
                assert_eq!(c["index"], component.index);
            }
        }
        assert_ne!(
            listing_terms_sha256(product, EXPIRY),
            listing_terms_sha256(product, EXPIRY + 1)
        );
    }
}

#[test]
fn origins_are_normalized_report_identities() {
    let a = origin_id("TrendForce", "DRAM contract price");
    assert_eq!(
        hex32(&a),
        "5f262f0674294e72cd0d39f2ffe50453e7ddc28ff6ba22793f1d771dc1c9c82d"
    );
    assert_eq!(origin_id("  trendforce ", "DRAM   Contract\tPrice"), a);
    assert_ne!(origin_id("TrendForce", "DRAM spot price"), a);
    assert_ne!(origin_id("TrendForceDRAM", "contract price"), a);
    assert_eq!(
        a,
        reference_sha256(b"amoeba-oracle-origin-v1\ntrendforce\ndram contract price")
    );
}

fn mutated(edit: impl FnOnce(&mut Value)) -> Result<Molds, String> {
    let mut value = raw();
    edit(&mut value);
    parse_molds(&serde_json::to_string(&value).unwrap())
}

#[test]
fn validation_refuses_a_broken_mold() {
    assert!(mutated(|_| {}).is_ok());
    fn item(v: &mut Value) -> &mut Value {
        &mut v["products"][0]["items"][0]
    }
    // An unfilled term.
    let err = mutated(|v| {
        item(v)["terms"]["all"]
            .as_object_mut()
            .unwrap()
            .remove("period");
    })
    .unwrap_err();
    assert!(err.contains("[period] is not filled"), "{err}");
    // A term left as "standard".
    let err = mutated(|v| item(v)["terms"]["all"]["period"] = "Standard".into()).unwrap_err();
    assert!(err.contains("left as"), "{err}");
    // A bracket inside a term.
    let err = mutated(|v| item(v)["terms"]["all"]["period"] = "[period]".into()).unwrap_err();
    assert!(err.contains("bracket"), "{err}");
    // A literal sentence that is not its template filled.
    let err = mutated(|v| {
        item(v)["sentences"]["spot"]["text"] = "USD per chip, anything.".into();
    })
    .unwrap_err();
    assert!(err.contains("not its template filled"), "{err}");
    // A wrong hash.
    let err = mutated(|v| {
        item(v)["sentences"]["lta"]["sha256"] = "00".repeat(32).into();
    })
    .unwrap_err();
    assert!(err.contains("sha256"), "{err}");
    // A mix that does not sum to 10,000.
    let err = mutated(|v| item(v)["mixBps"]["spot"] = 999.into()).unwrap_err();
    assert!(err.contains("10,000"), "{err}");
    // An unknown basis.
    let err = mutated(|v| item(v)["mixBasis"] = "guessed".into()).unwrap_err();
    assert!(err.contains("mixBasis"), "{err}");
    // A basket weight that is not the bucket table's.
    let err = mutated(|v| item(v)["basketWeightBps"] = 3_999.into()).unwrap_err();
    assert!(err.contains("basket weight"), "{err}");
    // A term no template uses.
    let err =
        mutated(|v| item(v)["terms"]["spot"] = serde_json::json!({"colour": "green"})).unwrap_err();
    assert!(err.contains("never used"), "{err}");
    // Two channels only.
    let err = mutated(|v| {
        v["products"][2]["channels"].as_array_mut().unwrap().pop();
    })
    .unwrap_err();
    assert!(err.contains("three channels"), "{err}");
    // An unclosed bracket in a template.
    let err = mutated(|v| {
        v["products"][1]["channels"][0]["template"] = "[Currency per [unit]".into();
    })
    .unwrap_err();
    assert!(err.contains("unclosed"), "{err}");
    // A product out of place.
    let err = mutated(|v| v["products"][0]["product"] = "DRAMX".into()).unwrap_err();
    assert!(err.contains("RAMX"), "{err}");
    // Unknown fields are refused.
    let err = mutated(|v| item(v)["note"] = "x".into()).unwrap_err();
    assert!(err.contains("unknown field"), "{err}");
}

/// The quarterly mix (the user, 2026-10-29: "LTA %ages of weight need to adjust dynamically
/// depending on the Q."): every item has a mix per quarter from Q4 2026, starting at its default
/// mix; LTA's basket-weighted share rises quarter by quarter, taken from the contract channel,
/// spot flat; a contract's quarter is the one holding its expiry's last second.
#[test]
fn every_item_has_a_mix_per_quarter_with_lta_rising() {
    let q = |year, quarter| Quarter::new(year, quarter).unwrap();
    assert_eq!(Quarter::of_expiry(1_798_761_600), q(2026, 4)); // 2027-01-01
    assert_eq!(Quarter::of_expiry(1_806_537_600), q(2027, 1)); // 2027-04-01
    assert_eq!(Quarter::of_expiry(EXPIRY), q(2026, 4)); // 2026-12-01
    assert_eq!(q(2027, 1).start_ts(), 1_798_761_600);
    assert_eq!(q(2026, 4).next(), q(2027, 1));
    assert_eq!(Quarter::from_id("2027-Q1"), Some(q(2027, 1)));
    assert_eq!(Quarter::from_id("2027-Q5"), None);
    assert_eq!(q(2027, 1).label(), "Q1 2027");
    // Basket-weighted LTA share per quarter, in basis points of a basis point.
    let expected = [
        (Product::Ramx, [3_725, 4_125, 4_525, 4_825, 5_125]),
        (Product::Nandx, [3_300, 3_600, 3_900, 4_200, 4_500]),
        (Product::Batx, [6_100, 6_300, 6_500, 6_700, 6_900]),
    ];
    for (product, lta) in expected {
        let mold = product.mold();
        assert!(!mold.mix_schedule_reason.is_empty());
        let mut quarter = MIX_SCHEDULE_FIRST;
        for want in lta {
            let mut got = 0u64;
            for item in &mold.items {
                let mix = item.mix_for(quarter);
                assert!(heads::mix_is_valid(&mix));
                // Spot stays flat.
                assert_eq!(mix[0], item.mix_bps[0]);
                got += u64::from(mix[2]) * u64::from(item.basket_weight_bps);
            }
            assert_eq!(got / 10_000, want, "{} {}", product.symbol(), quarter.id());
            quarter = quarter.next();
        }
        // Outside the schedule, its ends hold.
        for item in &mold.items {
            assert_eq!(item.mix_for(q(2026, 1)), item.mix_bps);
            assert_eq!(
                item.mix_for(q(2030, 2)),
                item.mix_schedule.last().unwrap().1
            );
        }
    }
    // The listing carries the quarter's mixes, and the documents differ by quarter.
    let q1 = 1_806_537_600;
    let mixes = listing_mixes(Product::Ramx, q1);
    assert_eq!(mixes[0].0, [1_000, 4_600, 4_400]);
    assert_ne!(
        listing_mix_commitment(Product::Ramx, q1),
        listing_mix_commitment(Product::Ramx, EXPIRY)
    );
    // Pinned (computed independently with Python).
    assert_eq!(
        hex32(&listing_mix_commitment(Product::Ramx, EXPIRY)),
        "6c472f01096b01979176b39cc44e903b8aaead399f854ac9c93bcd316df71129"
    );
    assert_eq!(
        hex32(&listing_mix_commitment(Product::Ramx, q1)),
        "528731dba738cd5ca468daf218f98726b92b19f0e2afbcd87a87cc10a5239abc"
    );
    assert_eq!(
        hex32(&listing_terms_sha256(Product::Ramx, q1)),
        "2a4e700755a7f778ba1225b0a7f27d08a824066588d5eb0bae793b5de719d4b6"
    );
    for product in [Product::Ramx, Product::Nandx, Product::Batx] {
        assert!(listing_terms_document(product, q1).len() <= SEALED_OBJECT_MAX_BYTES);
    }
}
