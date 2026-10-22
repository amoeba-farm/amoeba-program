//! Deterministic routing (spec `routing`): every series and observation gets one status and
//! the reasons for it. Nothing is guessed: an unset or unknown field is missing information,
//! never a default (`no_default_assumptions`). Routing refuses nothing; it only says what
//! counts, so open submission stays open.

use serde::{Deserialize, Serialize};

use crate::buckets::{Bucket, Product};
use crate::series::{price_atoms, Observation, ScopeValue, SeriesDefinition};
use crate::time::{days, unix_seconds};

/// The spec's routing statuses.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    /// Fits a bucket; it counts once a month commits it (its freeze).
    ClassifiedCandidate,
    /// In the month's committed series set: it counts toward settlement.
    EligibleForContract,
    /// Repeats a series or a release already counted: no extra weight.
    DuplicateNoExtraWeight,
    /// Possibly relevant, but a required identity or pricing field is unresolved.
    NeedsMetadata,
    /// Not the bucket's product.
    WrongProduct,
    /// Not an agreed contract price (spot, listing, offer, forecast).
    WrongPriceBasis,
    /// Useful evidence that doesn't match the settlement method.
    ResearchOnly,
    /// Relevant, but not in the month's committed series set.
    OutsideCommittedSeries,
    /// The product, terms, unit, statistic or method changed against the series' baseline.
    ScopeChange,
    /// Not the period the series is at, or older than what it already reported.
    StaleOrWrongPeriod,
}

impl Status {
    pub const ALL: [Status; 10] = [
        Status::ClassifiedCandidate,
        Status::EligibleForContract,
        Status::DuplicateNoExtraWeight,
        Status::NeedsMetadata,
        Status::WrongProduct,
        Status::WrongPriceBasis,
        Status::ResearchOnly,
        Status::OutsideCommittedSeries,
        Status::ScopeChange,
        Status::StaleOrWrongPeriod,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Status::ClassifiedCandidate => "CLASSIFIED_CANDIDATE",
            Status::EligibleForContract => "ELIGIBLE_FOR_CONTRACT",
            Status::DuplicateNoExtraWeight => "DUPLICATE_NO_EXTRA_WEIGHT",
            Status::NeedsMetadata => "NEEDS_METADATA",
            Status::WrongProduct => "WRONG_PRODUCT",
            Status::WrongPriceBasis => "WRONG_PRICE_BASIS",
            Status::ResearchOnly => "RESEARCH_ONLY",
            Status::OutsideCommittedSeries => "OUTSIDE_COMMITTED_SERIES",
            Status::ScopeChange => "SCOPE_CHANGE",
            Status::StaleOrWrongPeriod => "STALE_OR_WRONG_PERIOD",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Status::ALL
            .into_iter()
            .find(|status| status.as_str() == text)
    }

    /// Plain words for people.
    pub fn summary(self) -> &'static str {
        match self {
            Status::ClassifiedCandidate => "Fits the bucket",
            Status::EligibleForContract => "Counts this month",
            Status::DuplicateNoExtraWeight => "Already counted",
            Status::NeedsMetadata => "Missing details",
            Status::WrongProduct => "Wrong product",
            Status::WrongPriceBasis => "Not a contract price",
            Status::ResearchOnly => "Research only",
            Status::OutsideCommittedSeries => "Not in this month",
            Status::ScopeChange => "Definition changed",
            Status::StaleOrWrongPeriod => "Wrong period",
        }
    }

    /// Whether it counts toward this month's settlement.
    pub fn counts(self) -> bool {
        self == Status::EligibleForContract
    }

    /// Whether it is worth a bond: it fits a bucket and nothing disqualifies it.
    pub fn admissible(self) -> bool {
        matches!(
            self,
            Status::ClassifiedCandidate | Status::EligibleForContract
        )
    }

    /// Which status wins when several reasons apply: disqualifying ones first.
    fn precedence(self) -> u8 {
        match self {
            Status::WrongProduct => 0,
            Status::WrongPriceBasis => 1,
            Status::ResearchOnly => 2,
            Status::ScopeChange => 3,
            Status::NeedsMetadata => 4,
            Status::OutsideCommittedSeries => 5,
            Status::DuplicateNoExtraWeight => 6,
            Status::StaleOrWrongPeriod => 7,
            Status::ClassifiedCandidate => 8,
            Status::EligibleForContract => 9,
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One reason a routing isn't the base status.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reason {
    pub status: Status,
    /// The field it concerns, as the JSON names it (`dram.dieCount`).
    pub field: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Routing {
    pub status: Status,
    pub reasons: Vec<Reason>,
}

impl Routing {
    fn decide(base: Status, reasons: Vec<Reason>) -> Self {
        let status = reasons
            .iter()
            .map(|reason| reason.status)
            .min_by_key(|status| status.precedence())
            .unwrap_or(base);
        Routing { status, reasons }
    }

    /// The reasons that decided the status.
    pub fn deciding(&self) -> impl Iterator<Item = &Reason> {
        self.reasons
            .iter()
            .filter(move |reason| reason.status == self.status)
    }

    /// One routing reason that something outside the definition supplies (the bucket a
    /// source is registered under on chain, the month a submission is for). Its status
    /// becomes the routing's when it outranks the current one, as in routing itself.
    pub fn push(&mut self, status: Status, field: &str, message: impl Into<String>) {
        if status.precedence() < self.status.precedence() {
            self.status = status;
        }
        self.reasons.push(Reason {
            status,
            field: field.to_string(),
            message: message.into(),
        });
    }

    /// A routing of one reason, for something routing can't read at all (a source whose
    /// definition isn't a comparison series).
    pub fn only(status: Status, field: &str, message: impl Into<String>) -> Self {
        Routing {
            status,
            reasons: vec![Reason {
                status,
                field: field.to_string(),
                message: message.into(),
            }],
        }
    }
}

struct Reasons(Vec<Reason>);

impl Reasons {
    fn push(&mut self, status: Status, field: &str, message: impl Into<String>) {
        self.0.push(Reason {
            status,
            field: field.to_string(),
            message: message.into(),
        });
    }

    fn missing(&mut self, field: &str, what: &str) {
        self.push(Status::NeedsMetadata, field, format!("{what} is not set"));
    }

    /// A text field that must be set.
    fn text<'a>(&mut self, field: &str, what: &str, value: &'a Option<String>) -> Option<&'a str> {
        match value.as_deref().map(str::trim) {
            Some(text) if !text.is_empty() => Some(text),
            _ => {
                self.missing(field, what);
                None
            }
        }
    }

    /// A scope dimension. `exact` and `documented_mix` need a value; `strict` admits only
    /// those two (the series must fix the dimension, or define it as a documented mix).
    fn scope(&mut self, field: &str, what: &str, scope: &Option<ScopeValue>, strict: bool) {
        let Some(scope) = scope else {
            return self.missing(field, what);
        };
        let state = scope.state.as_deref().map(str::trim).unwrap_or("");
        let has_value = scope
            .value
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        match state {
            "exact" | "documented_mix" if has_value => {}
            "exact" | "documented_mix" => self.push(
                Status::NeedsMetadata,
                field,
                format!("{what} is {state} but names no value"),
            ),
            "not_disclosed" | "not_applicable" if !strict => {}
            "not_disclosed" | "not_applicable" => self.push(
                Status::NeedsMetadata,
                field,
                format!("{what} must be fixed (exact) or a documented mix, not {state}"),
            ),
            "" => self.missing(field, what),
            other => self.push(
                Status::NeedsMetadata,
                field,
                format!("{what} state {other:?} is not exact, documented_mix, not_disclosed or not_applicable"),
            ),
        }
    }

    fn exact(&mut self, field: &str, what: &str, scope: &Option<ScopeValue>) {
        let exact = scope.as_ref().is_some_and(|scope| {
            scope.state.as_deref() == Some("exact")
                && scope
                    .value
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
        });
        if !exact {
            self.push(
                Status::NeedsMetadata,
                field,
                format!("a direct component price needs its exact {what}"),
            );
        }
    }
}

const REFERENCE_PERIODS: [&str; 6] = ["month", "half_month", "week", "day", "quarter", "session"];
const CADENCES: [&str; 5] = ["monthly", "semi_monthly", "weekly", "daily", "quarterly"];

/// Routes a series on its own (spec `ordered_checks` 1-5): provenance, hard fields, the bucket's
/// identity, the evidence class and the measurement. The best it can say is
/// `CLASSIFIED_CANDIDATE`; a month decides the rest ([`route_series_in_month`]).
pub fn route_series(definition: &SeriesDefinition, product: Product) -> Routing {
    let mut reasons = Reasons(Vec::new());
    series_reasons(definition, product, &mut reasons);
    Routing::decide(Status::ClassifiedCandidate, reasons.0)
}

fn series_reasons(definition: &SeriesDefinition, product: Product, reasons: &mut Reasons) {
    // 1. Provenance: who publishes it and where.
    match definition.series_id.as_deref().map(str::trim) {
        Some(id) if valid_series_id(id) => {}
        Some(id) if !id.is_empty() => reasons.push(
            Status::NeedsMetadata,
            "seriesId",
            "the series id must be 1-64 letters, digits, '.', '_', ':' or '-'",
        ),
        _ => reasons.missing("seriesId", "the series id"),
    }
    if definition.definition_version.unwrap_or(0) == 0 {
        reasons.missing("definitionVersion", "the definition version (1 or more)");
    }
    reasons.text(
        "origin.publisher",
        "the publisher",
        &definition.origin.publisher,
    );
    if let Some(locator) = reasons.text(
        "origin.locator",
        "the page that publishes it",
        &definition.origin.locator,
    ) {
        if !public_https(locator) {
            reasons.push(
                Status::NeedsMetadata,
                "origin.locator",
                "the page must be a public https:// address of at most 384 bytes",
            );
        }
    }
    reasons.text(
        "origin.row",
        "the exact row or series label on that page",
        &definition.origin.row,
    );

    // 2-3. The bucket's hard identity.
    let bucket = match definition.bucket.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => match product.bucket(id) {
            Some(bucket) => Some(bucket),
            None if product.research_candidate(id).is_some() => {
                let candidate = product.research_candidate(id).expect("checked");
                reasons.push(
                    Status::ResearchOnly,
                    "bucket",
                    format!(
                        "{} is a research candidate of {}, not one of its buckets: a series of it is evidence for a later decision, not a settlement input",
                        candidate.name,
                        product.symbol()
                    ),
                );
                Some(candidate)
            }
            None => {
                let names: Vec<&str> = product.buckets().iter().map(|bucket| bucket.id).collect();
                reasons.push(
                    Status::WrongProduct,
                    "bucket",
                    format!(
                        "{id} is not a {} bucket ({})",
                        product.symbol(),
                        names.join(", ")
                    ),
                );
                None
            }
        },
        _ => {
            reasons.missing("bucket", "the commodity bucket");
            None
        }
    };
    match product {
        Product::Ramx => dram_reasons(definition, bucket, reasons),
        Product::Nandx => nand_reasons(definition, bucket, reasons),
        Product::Batx => cell_reasons(definition, bucket, reasons),
    }

    // 4. Evidence class and price basis.
    match reasons.text("evidenceClass", "the evidence class", &definition.evidence_class) {
        Some("published_assessment") => {}
        Some("direct_price") if product == Product::Nandx => reasons.push(
            Status::ResearchOnly,
            "evidenceClass",
            "NANDX buckets take published wafer contract assessments only; a direct quote is research evidence",
        ),
        Some("direct_price") => direct_reasons(definition, reasons),
        Some(other) => reasons.push(
            Status::NeedsMetadata,
            "evidenceClass",
            format!("evidence class {other:?} is not published_assessment or direct_price"),
        ),
        None => {}
    }
    match reasons.text("priceBasis", "the price basis", &definition.price_basis) {
        Some("contract") => {}
        // BATX records spot or contract and compares like with like (brief §2); the series
        // fixes its basis, and a change is a scope change. Retail listings and forecasts
        // aren't industrial prices (brief §1, §3).
        Some("spot" | "firm_offer") if product == Product::Batx => {}
        Some(basis @ ("listing" | "forecast")) if product == Product::Batx => reasons.push(
            Status::WrongPriceBasis,
            "priceBasis",
            format!("a {basis} price is not a China industrial cell price"),
        ),
        Some(basis @ ("spot" | "listing" | "firm_offer" | "forecast")) => reasons.push(
            Status::WrongPriceBasis,
            "priceBasis",
            format!("a {basis} price is not an agreed contract price"),
        ),
        Some(other) => reasons.push(
            Status::NeedsMetadata,
            "priceBasis",
            format!("price basis {other:?} is not contract, spot, listing, firm_offer or forecast"),
        ),
        None => {}
    }

    // 5. The measurement, fixed before any observation.
    reasons.text(
        "nativeUnit",
        "the native unit (an unknown denomination is not USD per chip)",
        &definition.native_unit,
    );
    reasons.text("statistic", "the price statistic", &definition.statistic);
    if let Some(currency) = reasons.text("currency", "the currency", &definition.currency) {
        if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
            reasons.push(
                Status::NeedsMetadata,
                "currency",
                "the currency must be a 3-letter code like USD",
            );
        } else if product == Product::Batx && currency != "CNY" {
            reasons.push(
                Status::ResearchOnly,
                "currency",
                format!("BATX measures native-CNY prices; a {currency} series would add a currency return"),
            );
        }
    }
    if let Some(period) = reasons.text(
        "referencePeriod",
        "the reference period",
        &definition.reference_period,
    ) {
        if !REFERENCE_PERIODS.contains(&period) {
            reasons.push(
                Status::NeedsMetadata,
                "referencePeriod",
                format!(
                    "reference period {period:?} is not one of {}",
                    REFERENCE_PERIODS.join(", ")
                ),
            );
        }
    }
    if let Some(cadence) = reasons.text("cadence", "the publication cadence", &definition.cadence) {
        if !CADENCES.contains(&cadence) {
            reasons.push(
                Status::NeedsMetadata,
                "cadence",
                format!("cadence {cadence:?} is not one of {}", CADENCES.join(", ")),
            );
        }
    }
    reasons.text(
        "revisionPolicy",
        "the revision and correction policy",
        &definition.revision_policy,
    );
    reasons.text(
        "missingDataPolicy",
        "the missing-data and continuation policy",
        &definition.missing_data_policy,
    );
    let direct = definition.is_direct();
    if direct {
        reasons.exact(
            "procurementSegment",
            "procurement segment",
            &definition.procurement_segment,
        );
        reasons.exact("geography", "geographic market", &definition.geography);
    } else {
        reasons.scope(
            "procurementSegment",
            "the procurement segment",
            &definition.procurement_segment,
            false,
        );
        reasons.scope(
            "geography",
            "the geographic scope",
            &definition.geography,
            false,
        );
    }
    if product == Product::Batx {
        china_reasons(definition, reasons);
    }
}

/// BATX v1 measures China domestic industrial pricing and blends in no other market
/// (brief §3): the geography must be exactly `CN`.
fn china_reasons(definition: &SeriesDefinition, reasons: &mut Reasons) {
    let Some(geography) = &definition.geography else {
        return; // already missing
    };
    let value = geography.value.as_deref().map(str::trim).unwrap_or("");
    match geography.state.as_deref().map(str::trim) {
        Some("exact") if value == "CN" => {}
        Some("exact") if !value.is_empty() => reasons.push(
            Status::ResearchOnly,
            "geography",
            format!("BATX version 1 measures China domestic prices (CN), not {value}"),
        ),
        Some("documented_mix") if !value.is_empty() => reasons.push(
            Status::ResearchOnly,
            "geography",
            "BATX version 1 measures China domestic prices (CN) alone, not a blend of markets",
        ),
        Some("not_disclosed" | "not_applicable") => reasons.push(
            Status::NeedsMetadata,
            "geography",
            "BATX needs the market the price is for (CN)",
        ),
        _ => {} // a malformed scope is already missing details
    }
}

fn dram_reasons(definition: &SeriesDefinition, bucket: Option<&Bucket>, reasons: &mut Reasons) {
    match reasons.text(
        "physicalScope",
        "the physical scope",
        &definition.physical_scope,
    ) {
        Some("packaged_ic") => {}
        Some(
            scope @ ("module" | "bare_die" | "known_good_die" | "fabricated_wafer" | "blank_wafer"
            | "ssd" | "emmc" | "ufs"),
        ) => reasons.push(
            Status::WrongProduct,
            "physicalScope",
            format!(
                "RAMX buckets are packaged DRAM chips, not {}",
                scope_words(scope)
            ),
        ),
        Some(other) => reasons.push(
            Status::NeedsMetadata,
            "physicalScope",
            format!("physical scope {other:?} is not recognized"),
        ),
        None => {}
    }
    let Some(dram) = &definition.dram else {
        return reasons.missing("dram", "the DRAM scope");
    };
    match reasons.text(
        "dram.memoryFamily",
        "the memory family",
        &dram.memory_family,
    ) {
        Some("ddr_sdram") => {}
        Some(family @ ("lpddr" | "gddr" | "hbm")) => reasons.push(
            Status::WrongProduct,
            "dram.memoryFamily",
            format!(
                "RAMX covers conventional DDR SDRAM, not {}",
                family.to_uppercase()
            ),
        ),
        Some(other) => reasons.push(
            Status::NeedsMetadata,
            "dram.memoryFamily",
            format!("memory family {other:?} is not ddr_sdram, lpddr, gddr or hbm"),
        ),
        None => {}
    }
    if let (Some(generation), Some(bucket)) = (
        reasons.text("dram.generation", "the DRAM generation", &dram.generation),
        bucket,
    ) {
        if Some(generation) != bucket.generation {
            reasons.push(
                Status::WrongProduct,
                "dram.generation",
                format!(
                    "{generation} is not {}'s generation, {}",
                    bucket.id,
                    bucket.generation.unwrap_or("")
                ),
            );
        }
    }
    if let (Some(density), Some(bucket)) = (
        reasons.text("dram.dieDensity", "the die density", &dram.die_density),
        bucket,
    ) {
        if density != bucket.die_density {
            reasons.push(
                Status::WrongProduct,
                "dram.dieDensity",
                format!(
                    "a {density} die is not {}'s {} die",
                    bucket.id, bucket.die_density
                ),
            );
        }
    }
    match dram.die_count {
        Some(1) => {}
        Some(0) | None => reasons.missing("dram.dieCount", "the number of dies in the package"),
        Some(count) => reasons.push(
            Status::WrongProduct,
            "dram.dieCount",
            format!("a {count}-die package is not a monolithic chip; total package density doesn't make it one"),
        ),
    }
    match dram.condition.as_deref().map(str::trim) {
        Some("new_unused") => {}
        Some(condition @ ("reclaimed" | "desoldered" | "refurbished" | "engineering_sample")) => {
            reasons.push(
                Status::WrongProduct,
                "dram.condition",
                format!(
                    "{} material is outside the standard-production exposure",
                    condition.replace('_', " ")
                ),
            )
        }
        Some("unknown") | Some("") | None => reasons.missing(
            "dram.condition",
            "the condition (an unknown condition is not new)",
        ),
        Some(other) => reasons.push(
            Status::NeedsMetadata,
            "dram.condition",
            format!("condition {other:?} is not recognized"),
        ),
    }
    match dram.grade.as_deref().map(str::trim) {
        Some("standard_production") => {}
        Some(grade @ ("ett" | "utt" | "downgrade" | "untested")) => reasons.push(
            Status::WrongProduct,
            "dram.grade",
            format!("{} material can't stand in for standard production", grade_words(grade)),
        ),
        Some("specialty") => reasons.push(
            Status::ResearchOnly,
            "dram.grade",
            "a specialty qualification or temperature grade needs its own series profile, which no contract has yet",
        ),
        Some("unknown") | Some("") | None => {
            reasons.missing("dram.grade", "the grade (an unknown grade is not standard grade)")
        }
        Some(other) => reasons.push(
            Status::NeedsMetadata,
            "dram.grade",
            format!("grade {other:?} is not recognized"),
        ),
    }
    reasons.scope(
        "dram.organization",
        "the organization (x4/x8/x16)",
        &dram.organization,
        true,
    );
    reasons.scope("dram.speed", "the speed (MT/s)", &dram.speed, true);
    if definition.is_direct() {
        reasons.exact("dram.organization", "organization", &dram.organization);
        reasons.exact("dram.speed", "speed", &dram.speed);
        reasons.exact(
            "dram.packageConstruction",
            "package construction",
            &dram.package_construction,
        );
    }
}

fn nand_reasons(definition: &SeriesDefinition, bucket: Option<&Bucket>, reasons: &mut Reasons) {
    match reasons.text(
        "physicalScope",
        "the physical scope",
        &definition.physical_scope,
    ) {
        Some("fabricated_wafer") => {}
        Some(scope @ ("packaged_ic" | "bare_die" | "known_good_die")) => reasons.push(
            Status::ResearchOnly,
            "physicalScope",
            format!(
                "a {} quote doesn't become a wafer assessment observation",
                scope_words(scope)
            ),
        ),
        Some(scope @ ("ssd" | "emmc" | "ufs" | "module" | "blank_wafer")) => reasons.push(
            Status::WrongProduct,
            "physicalScope",
            format!(
                "NANDX buckets are fabricated 3D NAND wafers, not {}",
                scope_words(scope)
            ),
        ),
        Some(other) => reasons.push(
            Status::NeedsMetadata,
            "physicalScope",
            format!("physical scope {other:?} is not recognized"),
        ),
        None => {}
    }
    let Some(nand) = &definition.nand else {
        return reasons.missing("nand", "the NAND scope");
    };
    match reasons.text(
        "nand.architecture",
        "the NAND architecture",
        &nand.architecture,
    ) {
        Some("3D") => {}
        Some("planar") => reasons.push(
            Status::WrongProduct,
            "nand.architecture",
            "planar NAND is excluded",
        ),
        Some(other) => reasons.push(
            Status::NeedsMetadata,
            "nand.architecture",
            format!("architecture {other:?} is not 3D or planar"),
        ),
        None => {}
    }
    if let (Some(cell), Some(bucket)) = (
        reasons.text("nand.cellType", "the cell type", &nand.cell_type),
        bucket,
    ) {
        if Some(cell) != bucket.cell_type {
            reasons.push(
                Status::WrongProduct,
                "nand.cellType",
                format!(
                    "{cell} is not {}'s cell type, {}",
                    bucket.id,
                    bucket.cell_type.unwrap_or("")
                ),
            );
        }
    }
    if let (Some(density), Some(bucket)) = (
        reasons.text("nand.dieDensity", "the per-die density", &nand.die_density),
        bucket,
    ) {
        if density != bucket.die_density {
            reasons.push(
                Status::WrongProduct,
                "nand.dieDensity",
                format!(
                    "{density} per die is not {}'s {}",
                    bucket.id, bucket.die_density
                ),
            );
        }
    }
    reasons.scope(
        "nand.testingScope",
        "the assessment's quality and testing scope",
        &nand.testing_scope,
        true,
    );
    // No universal layer requirement: any stated scope is fine, and it may be left out.
}

fn cell_reasons(definition: &SeriesDefinition, bucket: Option<&Bucket>, reasons: &mut Reasons) {
    match reasons.text(
        "physicalScope",
        "the physical scope",
        &definition.physical_scope,
    ) {
        Some("cell") => {}
        Some(
            scope @ ("battery_module" | "pack" | "storage_system" | "cathode_material"
            | "lithium_carbonate" | "retail_bundle"),
        ) => reasons.push(
            Status::WrongProduct,
            "physicalScope",
            format!(
                "BATX buckets are manufactured cells, not {}",
                cell_scope_words(scope)
            ),
        ),
        Some(other) => reasons.push(
            Status::NeedsMetadata,
            "physicalScope",
            format!("physical scope {other:?} is not recognized"),
        ),
        None => {}
    }
    let Some(cell) = &definition.cell else {
        return reasons.missing("cell", "the cell scope");
    };
    if let Some(chemistry) = reasons.text("cell.chemistry", "the cell chemistry", &cell.chemistry) {
        let family = match chemistry {
            "LFP" => Some("LFP"),
            "NCM" | "NCA" | "NCMA" | "nickel_ternary" => Some("nickel_ternary"),
            "LMFP" | "LMO" | "LCO" | "sodium_ion" => {
                reasons.push(
                    Status::WrongProduct,
                    "cell.chemistry",
                    format!("BATX covers LFP and nickel-based ternary cells, not {chemistry}"),
                );
                None
            }
            other => {
                reasons.push(
                    Status::NeedsMetadata,
                    "cell.chemistry",
                    format!("chemistry {other:?} is not LFP, NCM, NCA, NCMA, LMFP, LMO, LCO or sodium_ion"),
                );
                None
            }
        };
        if let (Some(family), Some(bucket)) = (family, bucket) {
            if Some(family) != bucket.chemistry {
                reasons.push(
                    Status::WrongProduct,
                    "cell.chemistry",
                    format!("a {chemistry} cell is not a {} cell", bucket.name),
                );
            }
        }
    }
    match reasons.text("cell.casing", "the cell casing", &cell.casing) {
        Some("cylindrical") => reasons.push(
            Status::WrongProduct,
            "cell.casing",
            "cylindrical cells are excluded from BATX version 1",
        ),
        Some(casing @ ("prismatic" | "pouch")) => {
            if let Some(bucket) = bucket.filter(|bucket| bucket.casing != Some(casing)) {
                reasons.push(
                    Status::WrongProduct,
                    "cell.casing",
                    format!("a {casing} cell is not a {} cell", bucket.name),
                );
            }
        }
        Some(other) => reasons.push(
            Status::NeedsMetadata,
            "cell.casing",
            format!("casing {other:?} is not prismatic, pouch or cylindrical"),
        ),
        None => {}
    }
    match reasons.text("cell.application", "the application", &cell.application) {
        Some(
            application @ ("stationary_storage" | "electric_vehicle" | "consumer" | "power_tool"
            | "two_wheeler"),
        ) => {
            if let Some(bucket) = bucket.filter(|bucket| bucket.application != Some(application)) {
                reasons.push(
                    Status::WrongProduct,
                    "cell.application",
                    format!(
                        "a cell for {} is not a {} cell",
                        application.replace('_', " "),
                        bucket.name
                    ),
                );
            }
        }
        Some(other) => reasons.push(
            Status::NeedsMetadata,
            "cell.application",
            format!("application {other:?} is not recognized"),
        ),
        None => {}
    }
    match cell.condition.as_deref().map(str::trim) {
        Some("new_unused") => {}
        Some(condition @ ("used" | "second_life" | "refurbished")) => reasons.push(
            Status::WrongProduct,
            "cell.condition",
            format!(
                "{} cells are excluded; BATX measures new manufactured cells",
                condition.replace('_', "-")
            ),
        ),
        Some("unknown") | Some("") | None => reasons.missing(
            "cell.condition",
            "the condition (an unknown condition is not new)",
        ),
        Some(other) => reasons.push(
            Status::NeedsMetadata,
            "cell.condition",
            format!("condition {other:?} is not recognized"),
        ),
    }
    reasons.scope("cell.capacity", "the capacity (Ah)", &cell.capacity, true);
    reasons.scope(
        "cell.performanceClass",
        "the performance class",
        &cell.performance_class,
        false,
    );
    reasons.scope(
        "cell.nominalVoltage",
        "the nominal voltage",
        &cell.nominal_voltage,
        false,
    );
    if definition.is_direct() {
        // A direct quote is one exact cell: its capacity, voltage and class are known. Its
        // tax and delivery terms are `direct.taxFreight` and `direct.deliveryTerms`.
        reasons.exact("cell.capacity", "capacity", &cell.capacity);
        reasons.exact(
            "cell.nominalVoltage",
            "nominal voltage",
            &cell.nominal_voltage,
        );
        reasons.exact(
            "cell.performanceClass",
            "performance class",
            &cell.performance_class,
        );
    } else {
        reasons.scope("cell.taxBasis", "the tax basis", &cell.tax_basis, false);
        reasons.scope(
            "cell.deliveryTerms",
            "the delivery terms",
            &cell.delivery_terms,
            false,
        );
    }
}

fn cell_scope_words(scope: &str) -> &'static str {
    match scope {
        "battery_module" => "battery modules",
        "pack" => "assembled packs",
        "storage_system" => "complete storage systems",
        "cathode_material" => "cathode powders",
        "lithium_carbonate" => "lithium carbonate",
        "retail_bundle" => "retail bundles",
        _ => "that",
    }
}

fn direct_reasons(definition: &SeriesDefinition, reasons: &mut Reasons) {
    let Some(direct) = &definition.direct else {
        return reasons.missing("direct", "a direct price's product identity and terms");
    };
    reasons.text(
        "direct.manufacturer",
        "the manufacturer",
        &direct.manufacturer,
    );
    reasons.text(
        "direct.mpn",
        "the exact MPN and specification revision",
        &direct.mpn,
    );
    reasons.text(
        "direct.quantityTier",
        "the quantity or quantity tier",
        &direct.quantity_tier,
    );
    reasons.text(
        "direct.deliveryTerms",
        "the delivery terms",
        &direct.delivery_terms,
    );
    reasons.text(
        "direct.pricingPeriod",
        "the pricing period",
        &direct.pricing_period,
    );
    reasons.text(
        "direct.taxFreight",
        "the tax and freight treatment",
        &direct.tax_freight,
    );
    reasons.text(
        "direct.paymentTerms",
        "the payment, rebate and bundling treatment",
        &direct.payment_terms,
    );
}

fn scope_words(scope: &str) -> &'static str {
    match scope {
        "module" => "a module",
        "bare_die" => "a bare die",
        "known_good_die" => "a known-good die",
        "fabricated_wafer" => "a wafer",
        "blank_wafer" => "a blank silicon wafer",
        "ssd" => "an SSD",
        "emmc" => "eMMC",
        "ufs" => "UFS",
        "packaged_ic" => "a packaged-chip",
        _ => "that",
    }
}

fn grade_words(grade: &str) -> &'static str {
    match grade {
        "ett" => "eTT",
        "utt" => "uTT",
        "downgrade" => "Downgrade",
        _ => "Untested",
    }
}

fn valid_series_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}

fn public_https(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    url.len() <= 384
        && !url
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
        && host.contains('.')
        && !host.contains('@')
        && !host.starts_with("localhost")
        && !host.starts_with("127.")
}

/// A month's stage for routing: whether its series set is still open or already committed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    /// Taking sources (SourceSubmission, Scramble): candidates join at the freeze.
    Open,
    /// From the freeze on: the recipe's sources are the committed series set.
    Committed,
}

/// One source of a month, in proposal order.
#[derive(Clone, Copy, Debug)]
pub struct MonthSeries<'a> {
    pub source_id: &'a str,
    /// Its series, when its definition is one.
    pub definition: Option<&'a SeriesDefinition>,
    /// In the month's recipe (Frozen, Active or Inactive).
    pub committed: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct MonthContext<'a> {
    pub product: Product,
    pub stage: Stage,
    pub sources: &'a [MonthSeries<'a>],
}

/// Routes a series against a month (spec `ordered_checks` 4-7): committed membership,
/// continuity with an earlier definition of the same series, and duplicates. `this_source` is
/// the series' own source when it is already on chain; `None` for a new proposal.
pub fn route_series_in_month(
    definition: &SeriesDefinition,
    month: &MonthContext<'_>,
    this_source: Option<&str>,
) -> Routing {
    let mut reasons = Reasons(Vec::new());
    series_reasons(definition, month.product, &mut reasons);
    let position = this_source.and_then(|id| month.sources.iter().position(|s| s.source_id == id));
    let earlier = match position {
        Some(index) => &month.sources[..index],
        None => month.sources,
    };
    let mut scope_change = false;
    let predecessor = earlier.iter().find(|source| {
        source.definition.is_some_and(|other| {
            other.series_id.is_some() && other.series_id == definition.series_id
        })
    });
    if let Some(predecessor) = predecessor {
        let previous = predecessor.definition.expect("found by its definition");
        let changes = comparability_changes(previous, definition);
        if changes.is_empty() {
            reasons.push(
                Status::DuplicateNoExtraWeight,
                "seriesId",
                format!(
                    "source {} already carries this series",
                    predecessor.source_id
                ),
            );
        } else {
            scope_change = true;
            for field in changes {
                reasons.push(
                    Status::ScopeChange,
                    field,
                    format!(
                        "{field} differs from series {} as source {} defines it; a changed series can't silently continue",
                        definition.series_id.as_deref().unwrap_or(""),
                        predecessor.source_id
                    ),
                );
            }
        }
    } else if let Some(original) = earlier.iter().find(|source| {
        source
            .definition
            .is_some_and(|other| same_underlying(other, definition))
    }) {
        reasons.push(
            Status::DuplicateNoExtraWeight,
            "origin.row",
            format!(
                "source {} already reports this row of this publisher; one assessment is one series",
                original.source_id
            ),
        );
    }
    let base = match month.stage {
        Stage::Open => Status::ClassifiedCandidate,
        Stage::Committed => {
            let committed = position.is_some_and(|index| month.sources[index].committed);
            if committed {
                Status::EligibleForContract
            } else {
                reasons.push(
                    Status::OutsideCommittedSeries,
                    "source",
                    "the month's series set is committed; a new series can join a later month",
                );
                Status::ClassifiedCandidate
            }
        }
    };
    let mut routing = Routing::decide(base, reasons.0);
    if scope_change {
        routing.status = Status::ScopeChange;
    }
    routing
}

/// The comparability fields that differ between two definitions of one series.
pub fn comparability_changes(
    previous: &SeriesDefinition,
    next: &SeriesDefinition,
) -> Vec<&'static str> {
    let mut changes = Vec::new();
    let mut check = |field: &'static str, same: bool| {
        if !same {
            changes.push(field);
        }
    };
    check("bucket", previous.bucket == next.bucket);
    check(
        "evidenceClass",
        previous.evidence_class == next.evidence_class,
    );
    check("priceBasis", previous.price_basis == next.price_basis);
    check(
        "origin.locator",
        same_text(&previous.origin.locator, &next.origin.locator),
    );
    check(
        "origin.row",
        same_text(&previous.origin.row, &next.origin.row),
    );
    check(
        "physicalScope",
        previous.physical_scope == next.physical_scope,
    );
    check(
        "nativeUnit",
        same_text(&previous.native_unit, &next.native_unit),
    );
    check("statistic", same_text(&previous.statistic, &next.statistic));
    check("currency", previous.currency == next.currency);
    check(
        "referencePeriod",
        previous.reference_period == next.reference_period,
    );
    check(
        "procurementSegment",
        previous.procurement_segment == next.procurement_segment,
    );
    let (old_dram, new_dram) = (
        previous.dram.clone().unwrap_or_default(),
        next.dram.clone().unwrap_or_default(),
    );
    check("dram.dieCount", old_dram.die_count == new_dram.die_count);
    check("dram.condition", old_dram.condition == new_dram.condition);
    check("dram.grade", old_dram.grade == new_dram.grade);
    check(
        "dram.organization",
        old_dram.organization == new_dram.organization,
    );
    check("dram.speed", old_dram.speed == new_dram.speed);
    check(
        "dram.packageConstruction",
        old_dram.package_construction == new_dram.package_construction,
    );
    check(
        "dram.qualification",
        old_dram.qualification == new_dram.qualification,
    );
    let (old_nand, new_nand) = (
        previous.nand.clone().unwrap_or_default(),
        next.nand.clone().unwrap_or_default(),
    );
    check(
        "nand.testingScope",
        old_nand.testing_scope == new_nand.testing_scope,
    );
    let (old_cell, new_cell) = (
        previous.cell.clone().unwrap_or_default(),
        next.cell.clone().unwrap_or_default(),
    );
    check("cell.chemistry", old_cell.chemistry == new_cell.chemistry);
    check("cell.casing", old_cell.casing == new_cell.casing);
    check(
        "cell.application",
        old_cell.application == new_cell.application,
    );
    check("cell.condition", old_cell.condition == new_cell.condition);
    check("cell.capacity", old_cell.capacity == new_cell.capacity);
    check(
        "cell.nominalVoltage",
        old_cell.nominal_voltage == new_cell.nominal_voltage,
    );
    check(
        "cell.performanceClass",
        old_cell.performance_class == new_cell.performance_class,
    );
    check("cell.taxBasis", old_cell.tax_basis == new_cell.tax_basis);
    check(
        "cell.deliveryTerms",
        old_cell.delivery_terms == new_cell.delivery_terms,
    );
    if previous.cell.is_some() || next.cell.is_some() {
        // BATX's market is part of its definition (brief §3).
        check("geography", previous.geography == next.geography);
    }
    let (old_direct, new_direct) = (
        previous.direct.clone().unwrap_or_default(),
        next.direct.clone().unwrap_or_default(),
    );
    check(
        "direct.manufacturer",
        same_text(&old_direct.manufacturer, &new_direct.manufacturer),
    );
    check("direct.mpn", same_text(&old_direct.mpn, &new_direct.mpn));
    check(
        "direct.quantityTier",
        same_text(&old_direct.quantity_tier, &new_direct.quantity_tier),
    );
    check(
        "direct.deliveryTerms",
        same_text(&old_direct.delivery_terms, &new_direct.delivery_terms),
    );
    check(
        "direct.paymentTerms",
        same_text(&old_direct.payment_terms, &new_direct.payment_terms),
    );
    changes
}

/// Two series that report one underlying assessment row (or one direct product): the same
/// bucket and row, from the same page or the same publisher.
pub fn same_underlying(left: &SeriesDefinition, right: &SeriesDefinition) -> bool {
    let row = |definition: &SeriesDefinition| normalized(&definition.origin.row);
    if left.bucket != right.bucket || row(left).is_none() || row(left) != row(right) {
        return false;
    }
    let mpn =
        |definition: &SeriesDefinition| definition.direct.as_ref().and_then(|d| normalized(&d.mpn));
    if mpn(left) != mpn(right) {
        return false;
    }
    let locator = |definition: &SeriesDefinition| normalized_url(&definition.origin.locator);
    let publisher = |definition: &SeriesDefinition| normalized(&definition.origin.publisher);
    (locator(left).is_some() && locator(left) == locator(right))
        || (publisher(left).is_some() && publisher(left) == publisher(right))
}

fn same_text(left: &Option<String>, right: &Option<String>) -> bool {
    normalized(left) == normalized(right)
}

fn normalized(text: &Option<String>) -> Option<String> {
    let words: Vec<String> = text
        .as_deref()?
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

fn normalized_url(url: &Option<String>) -> Option<String> {
    let url = url.as_deref()?.trim();
    let url = url.split('#').next().unwrap_or(url).trim_end_matches('/');
    let (scheme, rest) = url.split_once("://")?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    Some(format!("{}://{host}/{path}", scheme.to_lowercase()))
}

/// An earlier observation of the same series.
#[derive(Clone, Copy, Debug)]
pub struct PriorObservation<'a> {
    pub release_id: &'a str,
    pub reference_period_end: &'a str,
}

#[derive(Clone, Copy, Debug)]
pub struct ObservationContext<'a> {
    pub product: Product,
    /// The series is in the month's committed set.
    pub committed: bool,
    /// Its earlier observations.
    pub prior: &'a [PriorObservation<'a>],
    /// The month's expiry date (`YYYY-MM-DD`): a reference period starting after it is the
    /// wrong period.
    pub expiry: Option<&'a str>,
}

/// Routes an observation of a series (spec `ordered_checks` 6-7 and `observation_fields`).
pub fn route_observation(
    series: &SeriesDefinition,
    observation: &Observation,
    context: &ObservationContext<'_>,
) -> Routing {
    let mut reasons = Reasons(Vec::new());
    let series_routing = route_series(series, context.product);
    if !series_routing.status.admissible() {
        reasons.push(
            series_routing.status,
            "series",
            format!(
                "its series is {}: {}",
                series_routing.status,
                series_routing
                    .deciding()
                    .map(|reason| reason.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        );
    }
    let o = observation;
    reasons.text("observationId", "the observation id", &o.observation_id);
    if let Some(id) = reasons.text("seriesId", "the series id", &o.series_id) {
        if Some(id) != series.series_id.as_deref() {
            reasons.push(
                Status::ScopeChange,
                "seriesId",
                format!(
                    "it observes series {id}, not {}",
                    series.series_id.as_deref().unwrap_or("")
                ),
            );
        }
    }
    match o.series_definition_version {
        None => reasons.missing("seriesDefinitionVersion", "the series definition version"),
        Some(version) if Some(version) != series.definition_version => reasons.push(
            Status::ScopeChange,
            "seriesDefinitionVersion",
            format!(
                "it uses definition version {version}; the series is at {}",
                series.definition_version.unwrap_or(0)
            ),
        ),
        Some(_) => {}
    }
    reasons.text(
        "rawProductLabel",
        "the product label as printed",
        &o.raw_product_label,
    );
    if let Some(price) = reasons.text("rawPrice", "the price", &o.raw_price) {
        if price_atoms(price).is_none() {
            reasons.push(
                Status::NeedsMetadata,
                "rawPrice",
                "the price must be a positive decimal with at most 6 places",
            );
        }
    }
    let mut same = |field: &str, what: &str, value: &Option<String>, fixed: &Option<String>| {
        if let Some(value) = reasons.text(field, what, value) {
            if !same_text(&Some(value.to_string()), fixed) {
                reasons.push(
                    Status::ScopeChange,
                    field,
                    format!(
                        "{what} {value:?} isn't the series' {:?}",
                        fixed.as_deref().unwrap_or("")
                    ),
                );
            }
        }
    };
    same(
        "rawCurrency",
        "the currency",
        &o.raw_currency,
        &series.currency,
    );
    same("rawUnit", "the unit", &o.raw_unit, &series.native_unit);
    same(
        "selectedStatistic",
        "the statistic",
        &o.selected_statistic,
        &series.statistic,
    );
    if series.is_direct() {
        let direct = series.direct.clone().unwrap_or_default();
        same(
            "manufacturer",
            "the manufacturer",
            &o.manufacturer,
            &direct.manufacturer,
        );
        same("mpn", "the MPN", &o.mpn, &direct.mpn);
        same(
            "quantityTier",
            "the quantity tier",
            &o.quantity_tier,
            &direct.quantity_tier,
        );
        reasons.text(
            "productSpecificationReference",
            "the product specification reference",
            &o.product_specification_reference,
        );
        reasons.text("termsReference", "the terms reference", &o.terms_reference);
    }
    reasons.text(
        "publicationVintage",
        "the publication vintage",
        &o.publication_vintage,
    );
    if let Some(locator) = reasons.text(
        "originalEvidenceLocator",
        "the page it was read from",
        &o.original_evidence_locator,
    ) {
        if !locator.starts_with("https://") {
            reasons.push(
                Status::NeedsMetadata,
                "originalEvidenceLocator",
                "the page must be an https:// address",
            );
        }
    }
    let release = reasons.text("releaseId", "the release or event id", &o.release_id);
    reasons.text(
        "evidenceReference",
        "the archive capture or evidence hash",
        &o.evidence_reference,
    );

    // Time: the economic period, the publication and the retrieval stay separate.
    let start = date_field(
        &mut reasons,
        "referencePeriodStart",
        "the reference period start",
        &o.reference_period_start,
    );
    let end = date_field(
        &mut reasons,
        "referencePeriodEnd",
        "the reference period end",
        &o.reference_period_end,
    );
    let published = time_field(
        &mut reasons,
        "publishedAt",
        "the publication time",
        &o.published_at,
    );
    let retrieved = time_field(
        &mut reasons,
        "retrievedAt",
        "the retrieval time",
        &o.retrieved_at,
    );
    if let (Some(start), Some(end)) = (start, end) {
        if end < start {
            reasons.push(
                Status::NeedsMetadata,
                "referencePeriodEnd",
                "the period ends before it starts",
            );
        }
    }
    if let (Some(published), Some(retrieved)) = (published, retrieved) {
        if retrieved < published {
            reasons.push(
                Status::StaleOrWrongPeriod,
                "retrievedAt",
                "it was retrieved before it was published",
            );
        }
    }
    if let (Some(start), Some(expiry)) = (start, context.expiry.and_then(days)) {
        if start >= expiry {
            reasons.push(
                Status::StaleOrWrongPeriod,
                "referencePeriodStart",
                "its period starts after the month expires",
            );
        }
    }
    if let Some(release) = release {
        if context
            .prior
            .iter()
            .any(|prior| prior.release_id.trim() == release)
        {
            reasons.push(
                Status::DuplicateNoExtraWeight,
                "releaseId",
                format!(
                    "release {release} is already reported; a new capture of it adds no weight"
                ),
            );
        } else if let Some(end) = end {
            let latest = context
                .prior
                .iter()
                .filter_map(|prior| days(prior.reference_period_end.trim()))
                .max();
            if latest.is_some_and(|latest| end < latest) {
                reasons.push(
                    Status::StaleOrWrongPeriod,
                    "referencePeriodEnd",
                    "its period is older than the series' last observation",
                );
            }
        }
    }
    let base = if context.committed {
        Status::EligibleForContract
    } else {
        Status::ClassifiedCandidate
    };
    Routing::decide(base, reasons.0)
}

fn date_field(
    reasons: &mut Reasons,
    field: &str,
    what: &str,
    value: &Option<String>,
) -> Option<i64> {
    let text = reasons.text(field, what, value)?;
    let parsed = days(text);
    if parsed.is_none() {
        reasons.push(
            Status::NeedsMetadata,
            field,
            format!("{what} must be a date, YYYY-MM-DD"),
        );
    }
    parsed
}

fn time_field(
    reasons: &mut Reasons,
    field: &str,
    what: &str,
    value: &Option<String>,
) -> Option<i64> {
    let text = reasons.text(field, what, value)?;
    let parsed = unix_seconds(text);
    if parsed.is_none() {
        reasons.push(
            Status::NeedsMetadata,
            field,
            format!("{what} must be YYYY-MM-DD or YYYY-MM-DDTHH:MM:SSZ"),
        );
    }
    parsed
}
