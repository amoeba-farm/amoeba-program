//! The channel molds of the long-term agreement (LTA) layer, from
//! `governance/channel_molds.v1.json` (`oracle/docs/lta.md`; the brief is
//! `oracle/docs/lta-spec.md`).
//!
//! A mold is what Amoeba fixes for a product at listing: its items (the commodity buckets,
//! with their basket weights), the three channel components of each item with their target
//! sentences, every bracketed term filled, each item's channel mix with its basis and
//! reasoning, the settlement target, and the disclosure. The sentences carry only the terms
//! that define their channel (currency and unit, product, the channel definition, the
//! observation period; LTA's agreement, price in effect, next reset and band; BATX's
//! index-linked formula): the user, 2026-10-29, "agreed just have challenges deal w some of
//! that stuff". Delegators choose sources freely
//! inside it. The math that uses it (origins, the reference, what settles) is
//! `ameba_oracle_heads::channels`; the codes here are the same bytes.
//!
//! Hashes: a component's `sentence_sha256` is SHA-256 of the exact sentence text, which is
//! also the commitment of a sealed kind-2 evidence object holding exactly those bytes, so each
//! sentence can sit on chain under its own hash. The listing terms document
//! ([`listing_terms_document`]) names every sentence by that hash and fits one kind-2 object.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::{Bucket, Product};

/// The governed molds, compiled in byte for byte.
pub const CHANNEL_MOLDS_JSON: &str = include_str!("../../../governance/channel_molds.v1.json");
/// SHA-256 of [`CHANNEL_MOLDS_JSON`].
pub const CHANNEL_MOLDS_SHA256_HEX: &str =
    "d8311ecaf0678196ede01d3cc10d9e7bbacaeafdcacd62b4a6bfe6625c694d8c";
/// The molds' `version`.
pub const CHANNEL_MOLDS_VERSION: &str = "channel-molds-v1-placeholder";
/// The molds' `schema`.
pub const CHANNEL_MOLDS_SCHEMA: &str = "amoeba.oracle-channel-molds";
/// The brief the molds implement, compiled in (`oracle/docs/lta-spec.md`).
pub const LTA_BRIEF_MD: &str = include_str!("../../../docs/lta-spec.md");

/// The first bytes of a listing terms document ([`listing_terms_document`]).
pub const LISTING_TERMS_DOMAIN: &str = "amoeba-oracle-listing-terms-v1\n";
/// The listing terms document's `schema`.
pub const LISTING_TERMS_SCHEMA: &str = "amoeba.oracle-listing-terms.v1";
/// The first bytes of a listing's mix commitment ([`listing_mix_commitment`]; the program's
/// `HEADS_LISTING_MIX_DOMAIN`).
pub const LISTING_MIX_DOMAIN: &str = "amoeba-oracle-listing-mix-v1\n";
/// The first bytes of an origin identity ([`origin_id`]).
pub const ORIGIN_DOMAIN: &str = "amoeba-oracle-origin-v1\n";
/// The most bytes a sealed kind-2 evidence object holds (the program's `DEFINITION_MAX`).
pub const SEALED_OBJECT_MAX_BYTES: usize = 4_096;

/// Channels per item.
pub const CHANNELS: usize = 3;

/// The channel a component observes (`ameba_oracle_heads::channels::Channel`, same codes).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Channel {
    Spot,
    Contract,
    Lta,
}

impl Channel {
    /// In code order.
    pub const ALL: [Channel; CHANNELS] = [Channel::Spot, Channel::Contract, Channel::Lta];

    /// The on-chain byte: 0 Spot, 1 Contract, 2 LTA.
    pub const fn code(self) -> u8 {
        match self {
            Channel::Spot => 0,
            Channel::Contract => 1,
            Channel::Lta => 2,
        }
    }

    pub const fn index(self) -> usize {
        self.code() as usize
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Channel::ALL.into_iter().find(|c| c.code() == code)
    }

    /// The mold's key: `spot`, `contract`, `lta`.
    pub const fn id(self) -> &'static str {
        match self {
            Channel::Spot => "spot",
            Channel::Contract => "contract",
            Channel::Lta => "lta",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Channel::ALL.into_iter().find(|c| c.id() == id)
    }

    /// What people read: `Spot`, `Contract`, `LTA`.
    pub const fn name(self) -> &'static str {
        match self {
            Channel::Spot => "Spot",
            Channel::Contract => "Contract",
            Channel::Lta => "LTA",
        }
    }
}

/// What backs an item's mix (`ameba_oracle_heads::channels::MixBasis`, same codes).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum MixBasis {
    None,
    Assumed,
    Matched,
}

impl MixBasis {
    pub const ALL: [MixBasis; 3] = [MixBasis::None, MixBasis::Assumed, MixBasis::Matched];

    pub const fn code(self) -> u8 {
        match self {
            MixBasis::None => 0,
            MixBasis::Assumed => 1,
            MixBasis::Matched => 2,
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            MixBasis::None => "none",
            MixBasis::Assumed => "assumed",
            MixBasis::Matched => "matched",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        MixBasis::ALL.into_iter().find(|b| b.id() == id)
    }
}

// ------------------------------------------------------------------------------------------
// Quarters
// ------------------------------------------------------------------------------------------

/// A calendar quarter (UTC). Heads contracts are quarterly after the 2026-11-01 expiry (the
/// user, 2026-10-29: "after the november 1st expiry market i want it to work in Quarters. so Q1
/// Q2 etc."): a contract expiring at a quarter boundary covers the quarter before it, and is
/// named by it ("RAMX Q1 2027" expires 2027-04-01).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Quarter {
    pub year: i32,
    /// 1 to 4.
    pub quarter: u8,
}

impl Quarter {
    pub fn new(year: i32, quarter: u8) -> Option<Self> {
        (1..=4).contains(&quarter).then_some(Self { year, quarter })
    }

    /// The quarter a contract expiring at `expiry_ts` covers: the one holding its last second
    /// (`ameba_oracle_heads::time::contract_quarter`).
    pub fn of_expiry(expiry_ts: i64) -> Self {
        let days = expiry_ts.saturating_sub(1).div_euclid(86_400);
        let (year, month, _) = crate::time::civil(days);
        Self {
            year: year.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            quarter: ((month - 1) / 3 + 1) as u8,
        }
    }

    /// Its first second, UTC.
    pub fn start_ts(self) -> i64 {
        let month = u32::from(self.quarter - 1) * 3 + 1;
        crate::time::days_from_civil(i64::from(self.year), month, 1).saturating_mul(86_400)
    }

    /// The next quarter.
    pub fn next(self) -> Self {
        if self.quarter == 4 {
            Self {
                year: self.year + 1,
                quarter: 1,
            }
        } else {
            Self {
                quarter: self.quarter + 1,
                ..self
            }
        }
    }

    /// `2027-Q1`.
    pub fn id(self) -> String {
        format!("{}-Q{}", self.year, self.quarter)
    }

    pub fn from_id(id: &str) -> Option<Self> {
        let (year, quarter) = id.split_once("-Q")?;
        if year.len() != 4 || quarter.len() != 1 {
            return None;
        }
        Self::new(year.parse().ok()?, quarter.parse().ok()?)
    }

    /// `Q1 2027`, as contracts are named ("RAMX Q1 2027").
    pub fn label(self) -> String {
        format!("Q{} {}", self.quarter, self.year)
    }
}

/// The first quarter of every item's mix schedule, and how many quarters it covers at least.
pub const MIX_SCHEDULE_FIRST: Quarter = Quarter {
    year: 2026,
    quarter: 4,
};
pub const MIX_SCHEDULE_QUARTERS: usize = 5;

/// What a listed contract settles on (`ameba_oracle_heads::channels::SettlementTarget`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SettlementTarget {
    /// Every item's Contract channel alone (the brief's Stage 1); no mold names it since
    /// 2026-10-29.
    ContractChannel,
    /// Every item's fixed-mix reference: its spot, contract and LTA channels weighted against
    /// each other by its mix, assumed or matched (every mold, since 2026-10-29).
    FixedMixReference,
}

impl SettlementTarget {
    pub const ALL: [SettlementTarget; 2] = [
        SettlementTarget::ContractChannel,
        SettlementTarget::FixedMixReference,
    ];

    pub const fn code(self) -> u8 {
        match self {
            SettlementTarget::ContractChannel => 0,
            SettlementTarget::FixedMixReference => 1,
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            SettlementTarget::ContractChannel => "contract_channel",
            SettlementTarget::FixedMixReference => "fixed_mix_reference",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        SettlementTarget::ALL.into_iter().find(|t| t.id() == id)
    }
}

/// One channel of a product's mold.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChannelMold {
    pub channel: Channel,
    /// Which purchases belong to the channel (the brief's hierarchy, section 3.1).
    pub classification: String,
    /// The target sentence with its bracketed terms, `[Currency]`, `[unit]`, ….
    pub template: String,
    /// What a claim's evidence must show to survive challenge (the brief, section 5), shown to
    /// fetchers at fill time.
    pub evidence: Vec<String>,
}

/// One component: an item observed in one channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Component {
    pub product: Product,
    pub bucket: &'static Bucket,
    /// The item's index in the basket.
    pub item: u8,
    pub channel: Channel,
    /// The mold's component order: `item * 3 + channel code`.
    pub index: u8,
    /// The bracketed terms of the channel's template in the order they appear, `(name,
    /// value)`.
    pub terms: Vec<(String, String)>,
    /// The frozen target sentence: the template with every term filled.
    pub sentence: String,
    /// SHA-256 of [`Component::sentence`]'s bytes: the component's `sentence_hash`, and the
    /// commitment of the sealed kind-2 object holding the sentence.
    pub sentence_sha256: [u8; 32],
}

impl Component {
    pub fn sentence_sha256_hex(&self) -> String {
        hex32(&self.sentence_sha256)
    }
}

/// One item (basket member) of a product's mold.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoldItem {
    pub bucket: &'static Bucket,
    pub basket_weight_bps: u16,
    /// `[spot, contract, lta]` in basis points, summing to 10,000: the board's default and
    /// display mix, the first scheduled quarter's.
    pub mix_bps: [u16; CHANNELS],
    pub mix_basis: MixBasis,
    /// One mix per calendar quarter, consecutive from the first; a contract freezes its
    /// quarter's ([`MoldItem::mix_for`]).
    pub mix_schedule: Vec<(Quarter, [u16; CHANNELS])>,
    /// One sentence of reasoning for this item's mix.
    pub mix_reason: String,
    /// Why the observation period is what it is.
    pub period_reason: String,
    /// Spot, Contract, LTA.
    pub components: [Component; CHANNELS],
}

impl MoldItem {
    pub fn component(&self, channel: Channel) -> &Component {
        &self.components[channel.index()]
    }

    /// The item's mix for `quarter` from its schedule: before the first scheduled quarter the
    /// first, after the last the last.
    pub fn mix_for(&self, quarter: Quarter) -> [u16; CHANNELS] {
        self.mix_schedule
            .iter()
            .rev()
            .find(|(q, _)| *q <= quarter)
            .or(self.mix_schedule.first())
            .map_or(self.mix_bps, |(_, mix)| *mix)
    }
}

/// A product's mold.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mold {
    pub product: Product,
    pub settlement_target: SettlementTarget,
    /// The one-sentence disclosure the listing carries: what the contract settles on ("This
    /// contract settles on a fixed-mix procurement reference: ...").
    pub disclosure: String,
    /// The mix's citation and reasoning, one line.
    pub mix_citation: String,
    /// Why the mix moves from quarter to quarter, one line.
    pub mix_schedule_reason: String,
    /// Spot, Contract, LTA.
    pub channels: [ChannelMold; CHANNELS],
    /// The basket's items in its order.
    pub items: Vec<MoldItem>,
}

impl Mold {
    pub fn channel(&self, channel: Channel) -> &ChannelMold {
        &self.channels[channel.index()]
    }

    pub fn item(&self, bucket_id: &str) -> Option<&MoldItem> {
        self.items.iter().find(|item| item.bucket.id == bucket_id)
    }

    /// Every component in the mold's order (`index` ascending).
    pub fn components(&self) -> impl Iterator<Item = &Component> {
        self.items.iter().flat_map(|item| item.components.iter())
    }

    pub fn component(&self, index: u8) -> Option<&Component> {
        self.components().find(|c| c.index == index)
    }
}

/// What a purchase's evidence declares, as the mold's classification examples write it
/// (`ameba_oracle_heads::channels::PurchaseFacts`, field for field).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PurchaseFacts {
    pub agreed: bool,
    pub agreement_months: u16,
    pub committed_volume: bool,
    pub negotiated_wholesale: bool,
    pub delivered_in_period: bool,
    pub immediate_inventory: bool,
}

/// One of the mold's documented classification examples.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassificationExample {
    pub case: String,
    pub facts: PurchaseFacts,
    /// `None`: eligible nowhere.
    pub channel: Option<Channel>,
}

/// One challenge reason code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChallengeReason {
    pub code: u8,
    pub id: String,
    pub meaning: String,
}

/// The parsed, validated molds file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Molds {
    pub version: String,
    /// The products' molds, in [`Product::ALL`] order.
    pub products: Vec<Mold>,
    pub evidence_rules: Vec<String>,
    pub mechanism_claim: String,
    pub origin_rule: String,
    pub lta_terms_rule: String,
    pub mix_rule: String,
    pub challenge_reasons: Vec<ChallengeReason>,
    pub classification_examples: Vec<ClassificationExample>,
}

impl Molds {
    pub fn mold(&self, product: Product) -> &Mold {
        self.products
            .iter()
            .find(|m| m.product == product)
            .expect("the molds cover every product")
    }
}

/// The compiled-in molds, parsed and validated once. The tests hold the file valid.
pub fn molds() -> &'static Molds {
    static MOLDS: OnceLock<Molds> = OnceLock::new();
    MOLDS.get_or_init(|| parse_molds(CHANNEL_MOLDS_JSON).expect("the compiled-in molds are valid"))
}

impl Product {
    /// This product's mold.
    pub fn mold(self) -> &'static Mold {
        molds().mold(self)
    }
}

// ------------------------------------------------------------------------------------------
// Parsing and validation
// ------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code)]
struct RawBrief {
    file: String,
    title: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawReason {
    code: u8,
    id: String,
    meaning: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawExample {
    case: String,
    facts: PurchaseFacts,
    channel: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawChannel {
    channel: String,
    code: u8,
    classification: String,
    template: String,
    evidence: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawSentence {
    text: String,
    sha256: String,
}

type RawTerms = BTreeMap<String, BTreeMap<String, String>>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawItem {
    bucket: String,
    basket_weight_bps: u16,
    mix_bps: BTreeMap<String, u16>,
    mix_basis: String,
    mix_schedule: Vec<RawQuarterMix>,
    mix_reason: String,
    terms: RawTerms,
    period_reason: String,
    sentences: BTreeMap<String, RawSentence>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawQuarterMix {
    quarter: String,
    spot: u16,
    contract: u16,
    lta: u16,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawProduct {
    product: String,
    settlement_target: String,
    disclosure: String,
    mix_citation: String,
    mix_schedule_reason: String,
    channels: Vec<RawChannel>,
    terms: RawTerms,
    items: Vec<RawItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code)]
struct RawMolds {
    schema: String,
    schema_version: u32,
    version: String,
    date: String,
    status: String,
    brief: RawBrief,
    decisions: Vec<String>,
    channel_codes: BTreeMap<String, u8>,
    classification_order: Vec<String>,
    classification_none: String,
    component_order: String,
    mix_basis_codes: BTreeMap<String, u8>,
    mix_rule: String,
    settlement_target_codes: BTreeMap<String, u8>,
    challenge_reasons: Vec<RawReason>,
    challenge_rule: String,
    evidence_rules: Vec<String>,
    mechanism_claim: String,
    origin_rule: String,
    lta_terms_rule: String,
    classification_examples: Vec<RawExample>,
    products: Vec<RawProduct>,
}

/// The challenge reasons, `(code, id)`, as `ameba_oracle_heads::channels::ChallengeReason`.
const CHALLENGE_REASONS: [(u8, &str); 4] = [
    (0, "value_or_other"),
    (1, "off_channel"),
    (2, "duplicate_origin"),
    (3, "expired_observation"),
];

const TERM_LAYERS: [&str; 4] = ["all", "spot", "contract", "lta"];

fn nonempty(text: &str, what: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err(format!("{what} is empty"));
    }
    Ok(())
}

/// The bracketed names of a template, in order of first appearance; fails on an unclosed or
/// nested bracket.
pub fn template_placeholders(template: &str) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find(['[', ']']) {
        if rest.as_bytes()[open] == b']' {
            return Err(format!("unmatched ']' in {template:?}"));
        }
        let after = &rest[open + 1..];
        let close = after
            .find(['[', ']'])
            .filter(|&i| after.as_bytes()[i] == b']')
            .ok_or_else(|| format!("unclosed '[' in {template:?}"))?;
        let name = &after[..close];
        if name.trim().is_empty() {
            return Err(format!("empty term name in {template:?}"));
        }
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
        rest = &after[close + 1..];
    }
    Ok(names)
}

/// The template with every `[name]` replaced by its term.
pub fn fill_template(template: &str, terms: &BTreeMap<String, String>) -> Result<String, String> {
    let mut out = String::with_capacity(template.len() * 2);
    let mut rest = template;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after
            .find(']')
            .ok_or_else(|| format!("unclosed '[' in {template:?}"))?;
        let name = &after[..close];
        let value = terms
            .get(name)
            .ok_or_else(|| format!("term [{name}] is not filled"))?;
        out.push_str(value);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn check_term_value(name: &str, value: &str, at: &str) -> Result<(), String> {
    nonempty(value, &format!("{at}: term [{name}]"))?;
    if value.contains(['[', ']']) {
        return Err(format!("{at}: term [{name}] has a bracket"));
    }
    // The brief, section 3.2: bracketed terms are fixed at listing, never left as "standard".
    if ["standard", "standard terms", "tbd", "n/a"]
        .iter()
        .any(|lazy| value.trim().eq_ignore_ascii_case(lazy))
    {
        return Err(format!("{at}: term [{name}] is left as {value:?}"));
    }
    Ok(())
}

fn check_layers(terms: &RawTerms, at: &str) -> Result<(), String> {
    for (layer, values) in terms {
        if !TERM_LAYERS.contains(&layer.as_str()) {
            return Err(format!("{at}: unknown term layer {layer:?}"));
        }
        for (name, value) in values {
            check_term_value(name, value, at)?;
        }
    }
    Ok(())
}

/// Parses and validates a molds file: the products are exactly RAMX, NANDX and BATX with their
/// buckets and basket weights, each with three channels; mixes sum to 10,000; every bracketed
/// term of every sentence is filled; each literal sentence is its template filled with its
/// terms and carries its SHA-256; no term goes unused.
pub fn parse_molds(json: &str) -> Result<Molds, String> {
    let raw: RawMolds =
        serde_json::from_str(json).map_err(|error| format!("not a molds file: {error}"))?;
    if raw.schema != CHANNEL_MOLDS_SCHEMA || raw.schema_version != 1 {
        return Err(format!(
            "schema {:?} v{}, not {CHANNEL_MOLDS_SCHEMA:?} v1",
            raw.schema, raw.schema_version
        ));
    }
    nonempty(&raw.version, "version")?;
    let channel_codes: BTreeMap<String, u8> = Channel::ALL
        .iter()
        .map(|c| (c.id().to_string(), c.code()))
        .collect();
    if raw.channel_codes != channel_codes {
        return Err("channelCodes are not spot 0, contract 1, lta 2".into());
    }
    if raw.classification_order != ["lta", "contract", "spot"] {
        return Err("classificationOrder is not lta, contract, spot".into());
    }
    let basis_codes: BTreeMap<String, u8> = MixBasis::ALL
        .iter()
        .map(|b| (b.id().to_string(), b.code()))
        .collect();
    if raw.mix_basis_codes != basis_codes {
        return Err("mixBasisCodes are not none 0, assumed 1, matched 2".into());
    }
    let target_codes: BTreeMap<String, u8> = SettlementTarget::ALL
        .iter()
        .map(|t| (t.id().to_string(), t.code()))
        .collect();
    if raw.settlement_target_codes != target_codes {
        return Err(
            "settlementTargetCodes are not contract_channel 0, fixed_mix_reference 1".into(),
        );
    }
    let reasons: Vec<(u8, &str)> = raw
        .challenge_reasons
        .iter()
        .map(|r| (r.code, r.id.as_str()))
        .collect();
    if reasons != CHALLENGE_REASONS {
        return Err("challengeReasons are not the four codes".into());
    }
    for r in &raw.challenge_reasons {
        nonempty(&r.meaning, &format!("challenge reason {}", r.id))?;
    }
    if raw.evidence_rules.len() != 3 {
        return Err("evidenceRules are not the brief's three".into());
    }
    for (what, text) in [
        ("mechanismClaim", &raw.mechanism_claim),
        ("originRule", &raw.origin_rule),
        ("ltaTermsRule", &raw.lta_terms_rule),
        ("mixRule", &raw.mix_rule),
        ("challengeRule", &raw.challenge_rule),
        ("classificationNone", &raw.classification_none),
    ] {
        nonempty(text, what)?;
    }
    let mut examples = Vec::new();
    for e in raw.classification_examples {
        nonempty(&e.case, "classification example")?;
        let channel = match e.channel.as_deref() {
            None => None,
            Some(id) => Some(
                Channel::from_id(id)
                    .ok_or_else(|| format!("{}: unknown channel {id:?}", e.case))?,
            ),
        };
        examples.push(ClassificationExample {
            case: e.case,
            facts: e.facts,
            channel,
        });
    }
    if raw.products.len() != Product::ALL.len() {
        return Err("the molds are not exactly RAMX, NANDX and BATX".into());
    }
    let mut products = Vec::new();
    for (raw_product, product) in raw.products.into_iter().zip(Product::ALL) {
        products.push(parse_product(raw_product, product)?);
    }
    Ok(Molds {
        version: raw.version,
        products,
        evidence_rules: raw.evidence_rules,
        mechanism_claim: raw.mechanism_claim,
        origin_rule: raw.origin_rule,
        lta_terms_rule: raw.lta_terms_rule,
        mix_rule: raw.mix_rule,
        challenge_reasons: raw
            .challenge_reasons
            .into_iter()
            .map(|r| ChallengeReason {
                code: r.code,
                id: r.id,
                meaning: r.meaning,
            })
            .collect(),
        classification_examples: examples,
    })
}

fn parse_product(raw: RawProduct, product: Product) -> Result<Mold, String> {
    let at = product.symbol();
    if raw.product != at {
        return Err(format!("expected {at}'s mold, found {:?}", raw.product));
    }
    let settlement_target = SettlementTarget::from_id(&raw.settlement_target)
        .ok_or_else(|| format!("{at}: unknown settlementTarget {:?}", raw.settlement_target))?;
    nonempty(&raw.disclosure, &format!("{at}: disclosure"))?;
    nonempty(&raw.mix_citation, &format!("{at}: mixCitation"))?;
    if raw.channels.len() != CHANNELS {
        return Err(format!("{at}: not three channels"));
    }
    let mut channels = Vec::new();
    let mut placeholders = Vec::new();
    for (c, channel) in raw.channels.into_iter().zip(Channel::ALL) {
        if c.channel != channel.id() || c.code != channel.code() {
            return Err(format!(
                "{at}: channel {} is not {}",
                c.channel,
                channel.id()
            ));
        }
        let where_ = format!("{at} {}", channel.id());
        nonempty(&c.classification, &format!("{where_}: classification"))?;
        nonempty(&c.template, &format!("{where_}: template"))?;
        if c.evidence.is_empty() {
            return Err(format!("{where_}: no evidence table"));
        }
        for row in &c.evidence {
            nonempty(row, &format!("{where_}: evidence"))?;
        }
        let names = template_placeholders(&c.template)?;
        if names.is_empty() {
            return Err(format!("{where_}: the template has no terms"));
        }
        placeholders.push(names);
        channels.push(ChannelMold {
            channel,
            classification: c.classification,
            template: c.template,
            evidence: c.evidence,
        });
    }
    check_layers(&raw.terms, at)?;
    let buckets = product.buckets();
    if raw.items.len() != buckets.len() {
        return Err(format!(
            "{at}: {} items, the basket has {}",
            raw.items.len(),
            buckets.len()
        ));
    }
    // Which term names each layer defines and whether a template used them.
    let mut used: BTreeMap<(String, String, String), bool> = BTreeMap::new();
    let mut note = |scope: &str, layers: &RawTerms| {
        for (layer, values) in layers {
            for name in values.keys() {
                used.entry((scope.to_string(), layer.clone(), name.clone()))
                    .or_insert(false);
            }
        }
    };
    note("product", &raw.terms);
    for item in &raw.items {
        note(&item.bucket, &item.terms);
    }
    let mut items = Vec::new();
    for (index, (item, bucket)) in raw.items.into_iter().zip(buckets).enumerate() {
        let where_ = format!("{at} {}", bucket.id);
        if item.bucket != bucket.id {
            return Err(format!(
                "{at}: item {index} is {:?}, not {}",
                item.bucket, bucket.id
            ));
        }
        if item.basket_weight_bps != bucket.weight_bps {
            return Err(format!(
                "{where_}: basket weight {} is not the bucket table's {}",
                item.basket_weight_bps, bucket.weight_bps
            ));
        }
        check_layers(&item.terms, &where_)?;
        let mut mix = [0u16; CHANNELS];
        if item.mix_bps.len() != CHANNELS {
            return Err(format!("{where_}: the mix is not spot, contract, lta"));
        }
        for channel in Channel::ALL {
            mix[channel.index()] = *item
                .mix_bps
                .get(channel.id())
                .ok_or_else(|| format!("{where_}: no {} share", channel.id()))?;
        }
        if mix.iter().map(|&m| u32::from(m)).sum::<u32>() != 10_000 {
            return Err(format!("{where_}: the mix does not sum to 10,000"));
        }
        let mix_basis = MixBasis::from_id(&item.mix_basis)
            .ok_or_else(|| format!("{where_}: unknown mixBasis {:?}", item.mix_basis))?;
        let mut mix_schedule: Vec<(Quarter, [u16; CHANNELS])> = Vec::new();
        for entry in &item.mix_schedule {
            let quarter = Quarter::from_id(&entry.quarter)
                .ok_or_else(|| format!("{where_}: unknown quarter {:?}", entry.quarter))?;
            let expected = mix_schedule
                .last()
                .map_or(MIX_SCHEDULE_FIRST, |(q, _)| q.next());
            if quarter != expected {
                return Err(format!(
                    "{where_}: the mix schedule's {} should be {}",
                    quarter.id(),
                    expected.id()
                ));
            }
            let shares = [entry.spot, entry.contract, entry.lta];
            if shares.iter().map(|&m| u32::from(m)).sum::<u32>() != 10_000 {
                return Err(format!(
                    "{where_}: the {} mix does not sum to 10,000",
                    quarter.id()
                ));
            }
            mix_schedule.push((quarter, shares));
        }
        if mix_schedule.len() < MIX_SCHEDULE_QUARTERS || mix_schedule[0].1 != mix {
            return Err(format!(
                "{where_}: the mix schedule must cover {MIX_SCHEDULE_QUARTERS} quarters from {} and \
                 start at mixBps",
                MIX_SCHEDULE_FIRST.id()
            ));
        }
        nonempty(&item.mix_reason, &format!("{where_}: mixReason"))?;
        nonempty(&item.period_reason, &format!("{where_}: periodReason"))?;
        if item.sentences.len() != CHANNELS {
            return Err(format!("{where_}: not three sentences"));
        }
        let mut components = Vec::new();
        for channel in Channel::ALL {
            let mut resolved: BTreeMap<String, String> = BTreeMap::new();
            let mut origin: BTreeMap<String, (String, String, String)> = BTreeMap::new();
            for (scope, layers) in [("product", &raw.terms), (bucket.id, &item.terms)] {
                for layer in ["all", channel.id()] {
                    for (name, value) in layers.get(layer).into_iter().flatten() {
                        resolved.insert(name.clone(), value.clone());
                        origin.insert(
                            name.clone(),
                            (scope.to_string(), layer.to_string(), name.clone()),
                        );
                    }
                }
            }
            let names = &placeholders[channel.index()];
            let mut terms = Vec::new();
            for name in names {
                let value = resolved.get(name).ok_or_else(|| {
                    format!("{where_} {}: term [{name}] is not filled", channel.id())
                })?;
                used.insert(origin[name].clone(), true);
                terms.push((name.clone(), value.clone()));
            }
            let sentence = fill_template(&channels[channel.index()].template, &resolved)?;
            if sentence.contains(['[', ']']) {
                return Err(format!("{where_} {}: unfilled bracket", channel.id()));
            }
            if sentence.len() > SEALED_OBJECT_MAX_BYTES {
                return Err(format!("{where_} {}: sentence too long", channel.id()));
            }
            let literal = item
                .sentences
                .get(channel.id())
                .ok_or_else(|| format!("{where_}: no {} sentence", channel.id()))?;
            if literal.text != sentence {
                return Err(format!(
                    "{where_} {}: the sentence is not its template filled with its terms",
                    channel.id()
                ));
            }
            let sentence_sha256 = sha256(sentence.as_bytes());
            if literal.sha256 != hex32(&sentence_sha256) {
                return Err(format!("{where_} {}: wrong sentence sha256", channel.id()));
            }
            components.push(Component {
                product,
                bucket,
                item: index as u8,
                channel,
                index: (index * CHANNELS + channel.index()) as u8,
                terms,
                sentence,
                sentence_sha256,
            });
        }
        let components: [Component; CHANNELS] = components.try_into().expect("three channels");
        items.push(MoldItem {
            bucket,
            basket_weight_bps: item.basket_weight_bps,
            mix_bps: mix,
            mix_basis,
            mix_schedule,
            mix_reason: item.mix_reason,
            period_reason: item.period_reason,
            components,
        });
    }
    if let Some(((scope, layer, name), _)) = used.iter().find(|(_, &was_used)| !was_used) {
        return Err(format!(
            "{at}: term [{name}] ({scope}, {layer}) is never used"
        ));
    }
    let channels: [ChannelMold; CHANNELS] = channels.try_into().expect("three channels");
    Ok(Mold {
        product,
        settlement_target,
        disclosure: raw.disclosure,
        mix_citation: raw.mix_citation,
        mix_schedule_reason: raw.mix_schedule_reason,
        channels,
        items,
    })
}

// ------------------------------------------------------------------------------------------
// The listing terms document
// ------------------------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TermsComponent<'a> {
    index: u8,
    channel: &'a str,
    sentence_sha256: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TermsItem<'a> {
    bucket: &'a str,
    name: &'a str,
    basket_weight_bps: u16,
    mix_bps: [u16; CHANNELS],
    mix_basis: &'a str,
    mix_reason: &'a str,
    components: Vec<TermsComponent<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TermsDocument<'a> {
    schema: &'a str,
    molds: &'a str,
    molds_sha256: &'a str,
    product: &'a str,
    expiry_ts: i64,
    quarter: String,
    settlement_target: &'a str,
    disclosure: &'a str,
    mix_citation: &'a str,
    mix_commitment: String,
    items: Vec<TermsItem<'a>>,
}

/// The listing terms document of one listed contract (the brief, section 8.E): the product,
/// the contract's quarter ([`Quarter::of_expiry`]), its items with their basket weights, each
/// item's mix for that quarter (`[spot, contract, lta]` bps, [`MoldItem::mix_for`]) with its
/// basis and reasoning, the mixes' commitment ([`listing_mix_commitment`], which the program
/// checks against the mixes `ListContract` carries), the product's mix citation, every
/// component's sentence by its SHA-256 (the text is the mold's, sealed under that hash), the
/// settlement target and the disclosure.
/// The bytes are [`LISTING_TERMS_DOMAIN`] followed by compact JSON in the field order above;
/// they fit one sealed kind-2 object, whose commitment is [`listing_terms_sha256`]. Every
/// listing of `product` at `expiry_ts` under these molds produces the same bytes.
pub fn listing_terms_document(product: Product, expiry_ts: i64) -> Vec<u8> {
    let mold = product.mold();
    let quarter = Quarter::of_expiry(expiry_ts);
    let document = TermsDocument {
        schema: LISTING_TERMS_SCHEMA,
        molds: CHANNEL_MOLDS_VERSION,
        molds_sha256: CHANNEL_MOLDS_SHA256_HEX,
        product: product.symbol(),
        expiry_ts,
        quarter: quarter.id(),
        settlement_target: mold.settlement_target.id(),
        disclosure: &mold.disclosure,
        mix_citation: &mold.mix_citation,
        mix_commitment: hex32(&listing_mix_commitment(product, expiry_ts)),
        items: mold
            .items
            .iter()
            .map(|item| TermsItem {
                bucket: item.bucket.id,
                name: item.bucket.name,
                basket_weight_bps: item.basket_weight_bps,
                mix_bps: item.mix_for(quarter),
                mix_basis: item.mix_basis.id(),
                mix_reason: &item.mix_reason,
                components: item
                    .components
                    .iter()
                    .map(|c| TermsComponent {
                        index: c.index,
                        channel: c.channel.id(),
                        sentence_sha256: c.sentence_sha256_hex(),
                    })
                    .collect(),
            })
            .collect(),
    };
    let mut bytes = LISTING_TERMS_DOMAIN.as_bytes().to_vec();
    bytes.extend(serde_json::to_vec(&document).expect("the terms document always serializes"));
    bytes
}

/// SHA-256 of [`listing_terms_document`]: the contract's `terms_hash`.
pub fn listing_terms_sha256(product: Product, expiry_ts: i64) -> [u8; 32] {
    sha256(&listing_terms_document(product, expiry_ts))
}

/// Each item's `(mix, basis)` for the contract expiring at `expiry_ts`: its quarter's
/// ([`MoldItem::mix_for`]), what `ListContract` carries.
pub fn listing_mixes(product: Product, expiry_ts: i64) -> Vec<([u16; CHANNELS], MixBasis)> {
    let quarter = Quarter::of_expiry(expiry_ts);
    product
        .mold()
        .items
        .iter()
        .map(|item| (item.mix_for(quarter), item.mix_basis))
        .collect()
}

/// The commitment of the contract's quarter mixes, which its listing terms document carries as
/// `mixCommitment` and the program recomputes from `ListContract`'s mixes
/// (`state::heads::heads_listing_mix_commitment`): SHA-256 of [`LISTING_MIX_DOMAIN`] ‖ product
/// id (32 bytes) ‖ the quarter's first second (u64 LE) ‖ item count (u8) ‖ per item: bucket id
/// (32 bytes) ‖ spot, contract, LTA shares (u16 LE each) ‖ basis code (u8).
pub fn listing_mix_commitment(product: Product, expiry_ts: i64) -> [u8; 32] {
    let quarter = Quarter::of_expiry(expiry_ts);
    let mold = product.mold();
    let mut bytes = LISTING_MIX_DOMAIN.as_bytes().to_vec();
    bytes.extend_from_slice(
        &crate::label_id32(product.symbol()).expect("product labels are canonical"),
    );
    bytes.extend_from_slice(&(quarter.start_ts() as u64).to_le_bytes());
    bytes.push(mold.items.len() as u8);
    for (item, (mix, basis)) in mold.items.iter().zip(listing_mixes(product, expiry_ts)) {
        bytes.extend_from_slice(&item.bucket.id32());
        for share in mix {
            bytes.extend_from_slice(&share.to_le_bytes());
        }
        bytes.push(basis.code());
    }
    sha256(&bytes)
}

// ------------------------------------------------------------------------------------------
// Origins
// ------------------------------------------------------------------------------------------

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

/// The canonical `origin_id` of a report (the molds' `originRule`): SHA-256 of
/// [`ORIGIN_DOMAIN`] ‖ publisher ‖ `\n` ‖ report, each trimmed, ASCII-lowercased and with runs
/// of whitespace made one space. The publisher is whoever first published the price, not a page
/// repeating it; the report is the publication series (a table, an assessment, a price list),
/// not one of its rows. Fetchers declare it; a false one is challengeable (`DuplicateOrigin`).
pub fn origin_id(publisher: &str, report: &str) -> [u8; 32] {
    let text = format!(
        "{ORIGIN_DOMAIN}{}\n{}",
        normalize(publisher),
        normalize(report)
    );
    sha256(text.as_bytes())
}

// ------------------------------------------------------------------------------------------
// SHA-256
// ------------------------------------------------------------------------------------------

/// Lowercase hex of 32 bytes.
pub fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 (FIPS 180-4). Implemented here so the crate's runtime dependencies stay serde and
/// serde_json: its consumers (the API, petri, the futures ops) build with `--locked` against
/// their own lockfiles. The tests check it against the `sha2` crate.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend(((data.len() as u64).wrapping_mul(8)).to_be_bytes());
    for block in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let [a, b, c, d, e, f, g, hh] = v;
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
        }
        for (x, y) in h.iter_mut().zip(v) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (chunk, word) in out.chunks_exact_mut(4).zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}
