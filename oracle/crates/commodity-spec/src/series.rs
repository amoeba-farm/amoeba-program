//! The comparison series (spec `comparison_series_manifest`) and the observation
//! (`observation_fields`) formats.
//!
//! A comparison series is one oracle source: its definition object carries
//! [`SeriesDefinition::definition_text`], compact JSON with unset fields left out. An observation
//! is one opening or update print of that source, with the metadata the chain doesn't hold.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::buckets::{Bucket, Product};

pub const SERIES_SCHEMA: &str = "amoeba.comparison-series.v1";
pub const OBSERVATION_SCHEMA: &str = "amoeba.series-observation.v1";

/// The longest definition text a source can carry: the oracle's 4,096-byte definition object
/// less petri's `ameba-oracle-source-definition-v1\0` domain and 4-byte length.
pub const MAX_DEFINITION_TEXT_BYTES: usize = 4_096 - 34 - 4;

/// A scope dimension (spec `scope_value_states`).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopeValue {
    /// `exact`, `documented_mix`, `not_disclosed` or `not_applicable`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// The value (`exact`) or the documented mix (`documented_mix`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

impl ScopeValue {
    pub fn exact(value: &str) -> Self {
        ScopeValue {
            state: Some("exact".into()),
            value: Some(value.into()),
        }
    }

    pub fn mix(value: &str) -> Self {
        ScopeValue {
            state: Some("documented_mix".into()),
            value: Some(value.into()),
        }
    }

    pub fn state(state: &str) -> Self {
        ScopeValue {
            state: Some(state.into()),
            value: None,
        }
    }
}

/// Where the series comes from: the publisher, the page and the exact row.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Origin {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    /// The public page where the price is published (the source's canonical locator).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
    /// The exact row, series or table label on that page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<String>,
}

/// DRAM technical scope (`hard_fields` and `dram_scope` of `RAMX_MONOLITHIC_CONTRACT`).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DramScope {
    /// `ddr_sdram` (conventional), `lpddr`, `gddr`, `hbm`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_family: Option<String>,
    /// `DDR5`, `DDR4`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<String>,
    /// Nominal monolithic die density: `8Gb`, `16Gb`, `24Gb`, `32Gb`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub die_density: Option<String>,
    /// Dies in the package; the initial profile admits 1 only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub die_count: Option<u8>,
    /// `new_unused`, `reclaimed`, `desoldered`, `refurbished`, `engineering_sample`, `unknown`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// `standard_production`, `ett`, `utt`, `downgrade`, `untested`, `specialty`, `unknown`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade: Option<String>,
    /// x4/x8/x16, or a documented organization mix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<ScopeValue>,
    /// Speed in MT/s, or a documented speed basket.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<ScopeValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_construction: Option<ScopeValue>,
    /// Qualification or temperature grade scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualification: Option<ScopeValue>,
}

/// NAND technical scope (`hard_fields` and `nand_scope` of `NANDX_PACKAGED_CONTRACT`): a packaged
/// NAND chip's identity, as [`DramScope`] is a DRAM chip's.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NandScope {
    /// `3D` or `planar`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
    /// `SLC`, `MLC`, `TLC`, `QLC`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_type: Option<String>,
    /// Nominal monolithic die density: `256Gb`, `512Gb`, `1Tb`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub die_density: Option<String>,
    /// Dies in the package; the initial profile admits 1 only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub die_count: Option<u8>,
    /// `new_unused`, `reclaimed`, `desoldered`, `refurbished`, `engineering_sample`, `unknown`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// `standard_production`, `downgrade`, `ink_die`, `white_label`, `untested`, `specialty`,
    /// `unknown`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade: Option<String>,
    /// Interface and I/O speed (ONFI or Toggle, MT/s), or a documented interface mix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface: Option<ScopeValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_construction: Option<ScopeValue>,
    /// Qualification or temperature grade scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualification: Option<ScopeValue>,
    /// Layer or process-generation scope; a documented mix is fine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_scope: Option<ScopeValue>,
    /// A wafer assessment's quality, testing and yield treatment: NANDX's definition before
    /// 2026-10-22, still read so such a series routes (as the wrong product). A chip series
    /// states its `grade` instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub testing_scope: Option<ScopeValue>,
}

/// A battery cell's identity and commercial basis (BATX brief §2, "Product identity" and
/// "Commercial basis"). The model or assessment code is `origin.row` for an assessment and
/// `direct.mpn` for a direct price.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CellScope {
    /// `LFP`, `NCM`, `NCA`, `NCMA`, `nickel_ternary`; also `LMFP`, `LMO`, `LCO`,
    /// `sodium_ion`, which no bucket covers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chemistry: Option<String>,
    /// `prismatic`, `pouch`, `cylindrical`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub casing: Option<String>,
    /// `stationary_storage`, `electric_vehicle`; also `consumer`, `power_tool`,
    /// `two_wheeler`, which no bucket covers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application: Option<String>,
    /// `new_unused`, `used`, `second_life`, `refurbished`, `unknown`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// Rated capacity (`314Ah`), or a documented mix (`280Ah and 314Ah`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity: Option<ScopeValue>,
    /// Nominal voltage (`3.2V`), as documented; never guessed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nominal_voltage: Option<ScopeValue>,
    /// The exact cell's documented rated energy in Wh, when a per-cell price is converted
    /// to CNY/Wh.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rated_energy_wh: Option<String>,
    /// Performance class: cycle-life, energy-density or power grade, or a documented mix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance_class: Option<ScopeValue>,
    /// VAT-inclusive or VAT-exclusive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax_basis: Option<ScopeValue>,
    /// Ex-works or delivered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_terms: Option<ScopeValue>,
}

/// A direct component price's identity and terms (`direct_price_scope`).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DirectTerms {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    /// Exact MPN and specification revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mpn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specification_reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_terms: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing_period: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax_freight: Option<String>,
    /// Payment, rebate and bundling treatment of the net unit price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_terms: Option<String>,
}

/// One comparison series: a repeatable priced object or published assessment, with its
/// commercial terms and chosen statistic fixed before any observation.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SeriesDefinition {
    /// Always [`SERIES_SCHEMA`].
    pub schema: String,
    /// The submitter's stable name for the series.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition_version: Option<u32>,
    /// The commodity bucket id (`DDR5_16GBIT`, `tlc_1tb`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
    /// `published_assessment` or `direct_price`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_class: Option<String>,
    /// `contract`, `spot`, `listing`, `firm_offer`, `forecast`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_basis: Option<String>,
    #[serde(default)]
    pub origin: Origin,
    /// Where the technical and commercial scope is documented (a methodology page, a
    /// datasheet), if not on the origin page itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_reference: Option<String>,
    /// `packaged_ic`, `module`, `bare_die`, `known_good_die`, `fabricated_wafer`,
    /// `blank_wafer`, `ssd`, `emmc`, `ufs`; for BATX `cell`, `battery_module`, `pack`,
    /// `storage_system`, `cathode_material`, `lithium_carbonate`, `retail_bundle`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_scope: Option<String>,
    /// The documented native unit, e.g. "USD per chip", "USD per GB".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_unit: Option<String>,
    /// The fixed price statistic: "average", "mainstream", "high", "low", "latest", …
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statistic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversion_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procurement_segment: Option<ScopeValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geography: Option<ScopeValue>,
    /// `month`, `half_month`, `week`, `day`, `quarter`, `session`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_period: Option<String>,
    /// `monthly`, `semi_monthly`, `weekly`, `daily`, `quarterly`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cadence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_data_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dram: Option<DramScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nand: Option<NandScope>,
    /// BATX only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell: Option<CellScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct: Option<DirectTerms>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl SeriesDefinition {
    /// Parses a series from JSON text (a file, or a source's definition text). Fails on any
    /// field the schema doesn't name.
    pub fn parse(text: &str) -> Result<Self, String> {
        let definition: SeriesDefinition = serde_json::from_str(text)
            .map_err(|error| format!("not a series definition: {error}"))?;
        if definition.schema != SERIES_SCHEMA {
            return Err(format!(
                "schema is {:?}, not {SERIES_SCHEMA:?}",
                definition.schema
            ));
        }
        Ok(definition)
    }

    /// Whether this text is a series definition at all (its `schema` names one), so a reader
    /// can tell a malformed series from an ordinary free-text definition.
    pub fn claims_schema(text: &str) -> bool {
        serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|value| {
                value
                    .get("schema")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .is_some_and(|schema| schema == SERIES_SCHEMA)
    }

    /// The canonical text a source's definition object carries: compact JSON in the schema's
    /// field order, unset fields left out.
    pub fn definition_text(&self) -> String {
        serde_json::to_string(self).expect("a series definition always serializes")
    }

    /// The bucket this series names, in the given product.
    pub fn bucket_in(&self, product: Product) -> Option<&'static Bucket> {
        product.bucket(self.bucket.as_deref()?)
    }

    pub fn is_direct(&self) -> bool {
        self.evidence_class.as_deref() == Some("direct_price")
    }

    /// The source type a series' source carries (the oracle stores only its hash):
    /// `comparison-series/v1:<evidenceClass>`.
    pub fn source_type(&self) -> String {
        format!(
            "{SERIES_SOURCE_TYPE_PREFIX}{}",
            self.evidence_class.as_deref().unwrap_or("unset")
        )
    }
}

/// The prefix of a series source's source type ([`SeriesDefinition::source_type`]).
pub const SERIES_SOURCE_TYPE_PREFIX: &str = "comparison-series/v1:";

/// A blank series for a bucket, every field present (unset ones as `null`) so a submitter
/// sees the whole schema. Only the bucket's own identity is filled in.
pub fn series_template(bucket: &Bucket, direct: bool) -> Value {
    let scope = || json!({ "state": null, "value": null });
    let mut template = json!({
        "schema": SERIES_SCHEMA,
        "seriesId": null,
        "definitionVersion": 1,
        "bucket": bucket.id,
        "evidenceClass": if direct { "direct_price" } else { "published_assessment" },
        "priceBasis": "contract",
        "origin": { "publisher": null, "locator": null, "row": null },
        "scopeReference": null,
        "physicalScope": null,
        "nativeUnit": null,
        "statistic": null,
        "currency": "USD",
        "conversionPolicy": null,
        "procurementSegment": scope(),
        "geography": scope(),
        "referencePeriod": null,
        "cadence": null,
        "revisionPolicy": null,
        "missingDataPolicy": null,
        "notes": null,
    });
    let object = template.as_object_mut().expect("object");
    match bucket.product {
        Product::Ramx => {
            object.insert("physicalScope".into(), json!("packaged_ic"));
            object.insert(
                "dram".into(),
                json!({
                    "memoryFamily": "ddr_sdram",
                    "generation": bucket.generation,
                    "dieDensity": bucket.die_density,
                    "dieCount": null,
                    "condition": null,
                    "grade": null,
                    "organization": scope(),
                    "speed": scope(),
                    "packageConstruction": scope(),
                    "qualification": scope(),
                }),
            );
        }
        Product::Batx => {
            // BATX prices may be spot or contract, CNY, China domestic (brief §3); the
            // submitter says which basis the series fixes.
            object.insert("priceBasis".into(), Value::Null);
            object.insert("currency".into(), json!("CNY"));
            object.insert(
                "geography".into(),
                json!({ "state": "exact", "value": "CN" }),
            );
            object.insert("physicalScope".into(), json!("cell"));
            object.insert(
                "cell".into(),
                json!({
                    "chemistry": if bucket.chemistry == Some("LFP") { Some("LFP") } else { None },
                    "casing": bucket.casing,
                    "application": bucket.application,
                    "condition": null,
                    "capacity": scope(),
                    "nominalVoltage": scope(),
                    "ratedEnergyWh": null,
                    "performanceClass": scope(),
                    "taxBasis": scope(),
                    "deliveryTerms": scope(),
                }),
            );
        }
        Product::Nandx => {
            object.insert("physicalScope".into(), json!("packaged_ic"));
            object.insert(
                "nand".into(),
                json!({
                    "architecture": "3D",
                    "cellType": bucket.cell_type,
                    "dieDensity": bucket.die_density,
                    "dieCount": null,
                    "condition": null,
                    "grade": null,
                    "interface": scope(),
                    "packageConstruction": scope(),
                    "qualification": scope(),
                    "layerScope": scope(),
                }),
            );
        }
    }
    if direct {
        object.insert(
            "direct".into(),
            json!({
                "manufacturer": null,
                "mpn": null,
                "specificationReference": null,
                "quantityTier": null,
                "deliveryTerms": null,
                "pricingPeriod": null,
                "market": null,
                "taxFreight": null,
                "paymentTerms": null,
            }),
        );
    }
    template
}

/// One observation of a series: one published price, its period and its evidence
/// (spec `observation_fields`).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Observation {
    /// Always [`OBSERVATION_SCHEMA`].
    pub schema: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_definition_version: Option<u32>,
    /// The product label exactly as the source prints it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_product_label: Option<String>,
    /// The price exactly as published, a decimal number in the native unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_statistic: Option<String>,
    /// The economic period the price is for, `YYYY-MM-DD` (inclusive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_period_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_period_end: Option<String>,
    /// When the source published it (`YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SSZ`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    /// When it was retrieved or proved. Never replaces `publishedAt`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieved_at: Option<String>,
    /// The publication's edition or vintage label ("2026-10 H2", "first release").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_vintage: Option<String>,
    /// The page the price was read from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_evidence_locator: Option<String>,
    /// The underlying release or event: every capture of one release shares it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_id: Option<String>,
    /// A web archive capture or a content hash of the evidence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_reference: Option<String>,
    // Direct prices only (`conditional_direct_price`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mpn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_specification_reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terms_reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl Observation {
    pub fn parse(text: &str) -> Result<Self, String> {
        let observation: Observation =
            serde_json::from_str(text).map_err(|error| format!("not an observation: {error}"))?;
        if observation.schema != OBSERVATION_SCHEMA {
            return Err(format!(
                "schema is {:?}, not {OBSERVATION_SCHEMA:?}",
                observation.schema
            ));
        }
        Ok(observation)
    }

    pub fn text(&self) -> String {
        serde_json::to_string(self).expect("an observation always serializes")
    }
}

/// The domain of an observation record's signed bytes ([`observation_record_message`]).
pub const OBSERVATION_RECORD_DOMAIN: &str = "amoeba-observation-record-v1\n";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationRecord<'a> {
    market_id: &'a str,
    expiry_id: &'a str,
    source_id: &'a str,
    claim: &'a str,
    observation: &'a Observation,
}

/// The bytes the claimant signs (ed25519, as a wallet signs a message) to record an observation
/// of its own claim with the API (`POST /oracle/observations`): [`OBSERVATION_RECORD_DOMAIN`]
/// followed by the record's compact JSON, `{"marketId","expiryId","sourceId","claim",
/// "observation"}` in that order, the observation in its own field order with unset fields left
/// out ([`Observation::text`]). The user's decision (2026-10-21): only the claimant may record what
/// its claim observed. Petri signs these bytes and the API verifies them.
pub fn observation_record_message(
    market_id: &str,
    expiry_id: &str,
    source_id: &str,
    claim: &str,
    observation: &Observation,
) -> Vec<u8> {
    let record = ObservationRecord {
        market_id,
        expiry_id,
        source_id,
        claim,
        observation,
    };
    let mut bytes = OBSERVATION_RECORD_DOMAIN.as_bytes().to_vec();
    bytes.extend(serde_json::to_vec(&record).expect("an observation record always serializes"));
    bytes
}

/// A blank observation of a series, every field present.
pub fn observation_template(series: &SeriesDefinition) -> Value {
    let mut template = json!({
        "schema": OBSERVATION_SCHEMA,
        "observationId": null,
        "seriesId": series.series_id,
        "seriesDefinitionVersion": series.definition_version,
        "rawProductLabel": series.origin.row,
        "rawPrice": null,
        "rawCurrency": series.currency,
        "rawUnit": series.native_unit,
        "selectedStatistic": series.statistic,
        "referencePeriodStart": null,
        "referencePeriodEnd": null,
        "publishedAt": null,
        "retrievedAt": null,
        "publicationVintage": null,
        "originalEvidenceLocator": series.origin.locator,
        "releaseId": null,
        "evidenceReference": null,
        "notes": null,
    });
    if series.is_direct() {
        let direct = series.direct.clone().unwrap_or_default();
        let object = template.as_object_mut().expect("object");
        object.insert("manufacturer".into(), json!(direct.manufacturer));
        object.insert("mpn".into(), json!(direct.mpn));
        object.insert(
            "productSpecificationReference".into(),
            json!(direct.specification_reference),
        );
        object.insert("quantityTier".into(), json!(direct.quantity_tier));
        object.insert("termsReference".into(), Value::Null);
    }
    template
}

/// A decimal price as 6-decimal atoms, the oracle's value scale: `"12.5"` → `12_500_000`.
/// `None` unless it is a positive decimal with at most six places.
pub fn price_atoms(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    let (whole, fraction) = raw.split_once('.').unwrap_or((raw, ""));
    if whole.is_empty()
        || fraction.len() > 6
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let whole: u64 = whole.parse().ok()?;
    let mut scaled = fraction.to_string();
    while scaled.len() < 6 {
        scaled.push('0');
    }
    let fraction: u64 = scaled.parse().ok()?;
    let atoms = whole.checked_mul(1_000_000)?.checked_add(fraction)?;
    (atoms > 0).then_some(atoms)
}
