//! The commodity buckets, their weights and the products' SKU manifests.
//!
//! RAMX's and NANDX's tables mirror `governance/commodity_submission_spec.v1.json` (`baskets`)
//! and `governance/{ramx,nandx}_product_sku_manifest.v2.json`. BATX's mirror
//! `governance/batx_basket_spec.v1.json` (`basket`) and
//! `governance/batx_product_sku_manifest.v1.json`. The tests hold each table to its files.

/// The governed submission spec, compiled in byte for byte.
pub const SPEC_JSON: &str = include_str!("../../../governance/commodity_submission_spec.v1.json");
/// SHA-256 of [`SPEC_JSON`], also the manifests' `provenance.sourceSha256`.
pub const SPEC_SHA256_HEX: &str =
    "69ad4a0c427f2eada2d18353e0ec5c58e8e326a1d447f9bbee77b9e5423a4309";
/// The spec's `version`.
pub const SPEC_VERSION: &str = "commodity-submission-spec-v1-proposed";

/// The chip product SKU manifests: what tag 190 registers for each product.
pub const RAMX_MANIFEST_JSON: &str =
    include_str!("../../../governance/ramx_product_sku_manifest.v2.json");
pub const NANDX_MANIFEST_JSON: &str =
    include_str!("../../../governance/nandx_product_sku_manifest.v2.json");

/// BATX's specification: the user's battery-cell basket brief of 2026-10-20
/// (`governance/batx_battery_cell_basket_brief.v1.pdf`), restated as JSON. Where they differ,
/// the brief governs.
pub const BATX_SPEC_JSON: &str = include_str!("../../../governance/batx_basket_spec.v1.json");
/// SHA-256 of [`BATX_SPEC_JSON`].
pub const BATX_SPEC_SHA256_HEX: &str =
    "48c7a9722ccb397f9c46814f7dae0cb8d996645d120fb11b63978e23bdfdbb38";
/// The BATX spec's `version`.
pub const BATX_SPEC_VERSION: &str = "batx-basket-spec-v1-proposed";
/// SHA-256 of the brief (the PDF), also BATX's manifest `provenance.sourceSha256`.
pub const BATX_BRIEF_SHA256_HEX: &str =
    "9761097947cf590562184ebb59ae09c53451b01b5629e3f44df5547eb0d3317c";
/// BATX's cell-category SKU manifest: what tag 190 registers for BATX.
pub const BATX_MANIFEST_JSON: &str =
    include_str!("../../../governance/batx_product_sku_manifest.v1.json");
/// BATX's first month expires 2026-12-01 00:00 UTC (the Nov 26 months); it has no earlier ones.
pub const BATX_FIRST_EXPIRY: i64 = 1_796_083_200;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Product {
    Ramx,
    Nandx,
    /// The battery-cell basket, from its first month (expiry 2026-12-01) on.
    Batx,
}

impl Product {
    pub const ALL: [Product; 3] = [Product::Ramx, Product::Nandx, Product::Batx];
    /// The products of `commodity_submission_spec.v1.json`.
    pub const CHIPS: [Product; 2] = [Product::Ramx, Product::Nandx];

    /// `RAMX`/`NANDX`/`BATX`, any case, or the market ids `ramx`/`nandx`/`batx`.
    pub fn from_symbol(symbol: &str) -> Option<Self> {
        Product::ALL
            .into_iter()
            .find(|product| symbol.eq_ignore_ascii_case(product.symbol()))
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Product::Ramx => "RAMX",
            Product::Nandx => "NANDX",
            Product::Batx => "BATX",
        }
    }

    /// The buckets in the spec's order.
    pub fn buckets(self) -> &'static [Bucket] {
        match self {
            Product::Ramx => &RAMX_BUCKETS,
            Product::Nandx => &NANDX_BUCKETS,
            Product::Batx => &BATX_BUCKETS,
        }
    }

    pub fn bucket(self, id: &str) -> Option<&'static Bucket> {
        self.buckets().iter().find(|bucket| bucket.id == id)
    }

    /// Buckets the spec defines that this product doesn't register ([`RAMX_RESEARCH_CANDIDATES`]).
    pub fn research_candidates(self) -> &'static [Bucket] {
        match self {
            Product::Ramx => &RAMX_RESEARCH_CANDIDATES,
            Product::Nandx | Product::Batx => &[],
        }
    }

    pub fn research_candidate(self, id: &str) -> Option<&'static Bucket> {
        self.research_candidates()
            .iter()
            .find(|bucket| bucket.id == id)
    }

    pub fn manifest(self) -> &'static ChipManifest {
        match self {
            Product::Ramx => &RAMX_CHIPS,
            Product::Nandx => &NANDX_CHIPS,
            Product::Batx => &BATX_CELLS,
        }
    }

    pub fn profile(self) -> Profile {
        match self {
            Product::Ramx => Profile::RamxMonolithicContract,
            Product::Nandx => Profile::NandxWaferContractAssessment,
            Product::Batx => Profile::BatxChinaIndustrialCell,
        }
    }

    /// Whether this product's buckets are the ones a month with this expiry (unix seconds)
    /// uses: RAMX and NANDX after 2026-11-01 ([`crate::uses_chip_buckets`]), BATX from its
    /// first month on.
    pub fn uses_buckets(self, expiry_ts: i64) -> bool {
        match self {
            Product::Ramx | Product::Nandx => crate::uses_chip_buckets(expiry_ts),
            Product::Batx => expiry_ts >= BATX_FIRST_EXPIRY,
        }
    }
}

/// The spec's product profiles (`product_profiles`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Profile {
    /// New, standard-production, single-die packaged DDR4/DDR5 SDRAM on a contract basis.
    RamxMonolithicContract,
    /// Published contract assessments of fabricated 3D NAND wafers (assessment only).
    NandxWaferContractAssessment,
    /// New manufactured lithium-ion cells, China domestic industrial pricing in native CNY
    /// (BATX brief, "Collection basis").
    BatxChinaIndustrialCell,
}

impl Profile {
    pub fn as_str(self) -> &'static str {
        match self {
            Profile::RamxMonolithicContract => "RAMX_MONOLITHIC_CONTRACT",
            Profile::NandxWaferContractAssessment => "NANDX_WAFER_CONTRACT_ASSESSMENT",
            Profile::BatxChinaIndustrialCell => "BATX_CHINA_INDUSTRIAL_CELL",
        }
    }
}

/// One commodity bucket: an on-chain SKU of the chip manifests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bucket {
    pub product: Product,
    /// The spec's bucket id, which is also the on-chain SKU label.
    pub id: &'static str,
    /// What people read: "DDR5 16Gb", "1Tb TLC", "LFP storage".
    pub name: &'static str,
    pub weight_bps: u16,
    pub profile: Profile,
    /// DRAM generation (`DDR5`, `DDR4`); `None` for NAND.
    pub generation: Option<&'static str>,
    /// NAND cell type (`TLC`, `QLC`); `None` for DRAM.
    pub cell_type: Option<&'static str>,
    /// Nominal die density, as the spec writes it (`16Gb`, `1Tb`); empty for BATX.
    pub die_density: &'static str,
    /// BATX cell chemistry family (`LFP`, `nickel_ternary`); `None` for RAMX and NANDX.
    pub chemistry: Option<&'static str>,
    /// BATX cell casing (`prismatic`, `pouch`); `None` for RAMX and NANDX.
    pub casing: Option<&'static str>,
    /// BATX cell application (`stationary_storage`, `electric_vehicle`); `None` otherwise.
    pub application: Option<&'static str>,
}

impl Bucket {
    /// The on-chain bucket id: the label, right-zero-padded to 32 bytes.
    pub fn id32(&self) -> [u8; 32] {
        label_id32(self.id).expect("bucket ids are canonical labels")
    }

    /// The bucket whose on-chain id this is, in any product (labels are unique across them).
    pub fn from_id32(id: &[u8; 32]) -> Option<&'static Bucket> {
        Product::ALL
            .iter()
            .flat_map(|product| product.buckets())
            .find(|bucket| bucket.id32() == *id)
    }

    /// `"<product> <name>"`, e.g. "RAMX DDR5 16Gb".
    pub fn title(&self) -> String {
        format!("{} {}", self.product.symbol(), self.name)
    }
}

const fn dram(
    id: &'static str,
    name: &'static str,
    generation: &'static str,
    die_density: &'static str,
    weight_bps: u16,
) -> Bucket {
    Bucket {
        product: Product::Ramx,
        id,
        name,
        weight_bps,
        profile: Profile::RamxMonolithicContract,
        generation: Some(generation),
        cell_type: None,
        die_density,
        chemistry: None,
        casing: None,
        application: None,
    }
}

const fn nand(
    id: &'static str,
    name: &'static str,
    cell_type: &'static str,
    die_density: &'static str,
    weight_bps: u16,
) -> Bucket {
    Bucket {
        product: Product::Nandx,
        id,
        name,
        weight_bps,
        profile: Profile::NandxWaferContractAssessment,
        generation: None,
        cell_type: Some(cell_type),
        die_density,
        chemistry: None,
        casing: None,
        application: None,
    }
}

const fn cell(
    id: &'static str,
    name: &'static str,
    chemistry: &'static str,
    casing: &'static str,
    application: &'static str,
    weight_bps: u16,
) -> Bucket {
    Bucket {
        product: Product::Batx,
        id,
        name,
        weight_bps,
        profile: Profile::BatxChinaIndustrialCell,
        generation: None,
        cell_type: None,
        die_density: "",
        chemistry: Some(chemistry),
        casing: Some(casing),
        application: Some(application),
    }
}

/// RAMX's buckets and weights: the user's of 2026-10-21 (DDR5 16Gb 40%, 24Gb 20%, DDR4 16Gb
/// 25%, 8Gb 15%), which replace the spec's 35/20/5/25/15. DDR5 32Gb is a research candidate
/// ([`RAMX_RESEARCH_CANDIDATES`]).
pub const RAMX_BUCKETS: [Bucket; 4] = [
    dram("DDR5_16GBIT", "DDR5 16Gb", "DDR5", "16Gb", 4_000),
    dram("DDR5_24GBIT", "DDR5 24Gb", "DDR5", "24Gb", 2_000),
    dram("DDR4_16GBIT", "DDR4 16Gb", "DDR4", "16Gb", 2_500),
    dram("DDR4_8GBIT", "DDR4 8Gb", "DDR4", "8Gb", 1_500),
];

/// A product's research candidates: a bucket the spec defines that the product doesn't
/// register, so a series of it is evidence for a later decision, never a settlement input. The
/// user, 2026-10-21: "DDR5 32Gb | 0% — research candidate", left out of RAMX's list (a
/// registered SKU needs a weight of at least 1 bps and a supported source every month, and tag
/// 190 registers a product's list once, so adding it later takes an oracle upgrade).
pub const RAMX_RESEARCH_CANDIDATES: [Bucket; 1] =
    [dram("DDR5_32GBIT", "DDR5 32Gb", "DDR5", "32Gb", 0)];

pub const NANDX_BUCKETS: [Bucket; 4] = [
    nand("tlc_1tb", "1Tb TLC", "TLC", "1Tb", 4_000),
    nand("qlc_1tb", "1Tb QLC", "QLC", "1Tb", 3_000),
    nand("tlc_512gb", "512Gb TLC", "TLC", "512Gb", 2_000),
    nand("tlc_256gb", "256Gb TLC", "TLC", "256Gb", 1_000),
];

/// The brief's four market categories, in its order (40 / 30 / 20 / 10).
pub const BATX_BUCKETS: [Bucket; 4] = [
    cell(
        "LFP_STORAGE",
        "LFP storage",
        "LFP",
        "prismatic",
        "stationary_storage",
        4_000,
    ),
    cell(
        "LFP_EV",
        "LFP EV",
        "LFP",
        "prismatic",
        "electric_vehicle",
        3_000,
    ),
    cell(
        "TERNARY_PRISM_EV",
        "Ternary prismatic EV",
        "nickel_ternary",
        "prismatic",
        "electric_vehicle",
        2_000,
    ),
    cell(
        "TERNARY_POUCH_EV",
        "Ternary pouch EV",
        "nickel_ternary",
        "pouch",
        "electric_vehicle",
        1_000,
    ),
];

/// A product SKU manifest: the label set tag 190 registers and the root 181 must match (the
/// chip lists, and BATX's cell categories).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChipManifest {
    pub product: Product,
    pub json: &'static str,
    pub required_sku_count: u16,
    /// The index-bound SKU Merkle root over the sorted labels, as tag 190 computes it.
    pub required_sku_root_hex: &'static str,
}

pub const RAMX_CHIPS: ChipManifest = ChipManifest {
    product: Product::Ramx,
    json: RAMX_MANIFEST_JSON,
    required_sku_count: 4,
    required_sku_root_hex: "bdd8895912b378796860682716c5e6139026b10389254360b7aa7ffa628542eb",
};

pub const NANDX_CHIPS: ChipManifest = ChipManifest {
    product: Product::Nandx,
    json: NANDX_MANIFEST_JSON,
    required_sku_count: 4,
    required_sku_root_hex: "918d65590251611af7e03014ed3f2ede200d644006d7d71f30a41fa702829cbc",
};

pub const BATX_CELLS: ChipManifest = ChipManifest {
    product: Product::Batx,
    json: BATX_MANIFEST_JSON,
    required_sku_count: 4,
    required_sku_root_hex: "5303c0507ecb9d42bd10367d7a5b13a32bb4f84ba3ad9dc8069fd6b8a9a341b9",
};

impl ChipManifest {
    /// The labels in on-chain order (ascending encoded bytes): leaf index order.
    pub fn sorted_labels(&self) -> Vec<&'static str> {
        let mut labels: Vec<&'static str> = self
            .product
            .buckets()
            .iter()
            .map(|bucket| bucket.id)
            .collect();
        labels.sort_by_key(|label| label_id32(label).expect("canonical label"));
        labels
    }

    pub fn sorted_ids(&self) -> Vec<[u8; 32]> {
        self.sorted_labels()
            .into_iter()
            .map(|label| label_id32(label).expect("canonical label"))
            .collect()
    }

    pub fn root(&self) -> [u8; 32] {
        let mut root = [0u8; 32];
        for (index, byte) in root.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&self.required_sku_root_hex[index * 2..index * 2 + 2], 16)
                .expect("pinned root is hex");
        }
        root
    }
}

/// The canonical label encoding (`printable-ascii-utf8-right-zero-padded-bytes32-v1`).
pub fn label_id32(label: &str) -> Option<[u8; 32]> {
    let bytes = label.as_bytes();
    if bytes.is_empty() || bytes.len() > 32 || bytes.iter().any(|b| !(0x20..=0x7e).contains(b)) {
        return None;
    }
    let mut id = [0u8; 32];
    id[..bytes.len()].copy_from_slice(bytes);
    Some(id)
}

/// The label a canonical id encodes, if it is one.
pub fn decode_label32(id: &[u8; 32]) -> Option<String> {
    let length = id.iter().position(|byte| *byte == 0).unwrap_or(32);
    let label = std::str::from_utf8(&id[..length]).ok()?;
    (label_id32(label)? == *id).then(|| label.to_string())
}

/// The bucket weights a month of this product freezes with, label → bps (sum 10,000).
pub fn weights(product: Product) -> Vec<(&'static str, u16)> {
    product
        .buckets()
        .iter()
        .map(|bucket| (bucket.id, bucket.weight_bps))
        .collect()
}
