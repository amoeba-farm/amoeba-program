//! CFM (China Flash Market) launch: the pinned parent registries, the council-approved launch
//! plan and the launch receipt (tag 130, `processor/cfm_launch.rs`).
//!
//! The user decided on 2026-10-15 that the first mainnet launch month forms this way "only for
//! the first mainnet launch month, like the old Sep/Oct one": three of the five current council
//! seats approve a plan of dated CFM prints, and the launch writes the month and its sources into
//! ordinary Game state. The receipt is created once per product and oracle deployment, so the
//! path is usable once for RAMX and once for NANDX; every later month forms on the ordinary path.
//!
//! The registries are Spread's `governance/cfm_parent_proxy_registry.v1.json` compiled to the
//! same bytes, so their pinned hashes are Spread's reviewed values. The plan is new: one per
//! product, sealed as a kind-2 evidence object whose commitment is the plan digest.

use crate::error::OracleError;
use crate::ProgramError;
use solana_program::{hash::hashv, pubkey::Pubkey};

/// Registry preimage domain: `sha256(REGISTRY_DOMAIN ‖ registry bytes)` is the pinned hash.
pub const REGISTRY_DOMAIN: &[u8] = b"amoeba-cfm-registry-v1";
pub const REGISTRY_HEADER_LEN: usize = 72;
pub const REGISTRY_ROW_LEN: usize = 107;
/// Plan preimage domain: the plan digest is `sha256(PLAN_DOMAIN ‖ program id ‖ plan bytes)`.
pub const PLAN_DOMAIN: &[u8] = b"ameba-oracle-cfm-launch-plan-v1";
pub const PLAN_MAGIC: [u8; 4] = *b"ACL1";
pub const PLAN_VERSION: u8 = 1;
pub const PLAN_HEADER_LEN: usize = 138;
pub const PLAN_ROW_LEN: usize = 16;
/// Receipt PDA: `[ameba-oracle-v1, RECEIPT_SEED, product_id]`.
pub const RECEIPT_SEED: &[u8] = b"cfm-launch-v1";
pub const RECEIPT_DISCRIMINATOR: [u8; 4] = *b"OCLR";
pub const RECEIPT_VERSION: u8 = 1;
/// Source type of a CFM parent row: `sha256(SOURCE_TYPE_DOMAIN)`.
pub const SOURCE_TYPE_DOMAIN: &[u8] = b"amoeba-cfm-parent-source-v1";
/// Archive commitment of a launch observation, a council-approved document print (not a
/// fabricated archive URL): `sha256(domain ‖ plan digest ‖ source id ‖ value ‖ print time)`.
pub const SNAPSHOT_DOMAIN: &[u8] = b"amoeba-council-document-snapshot-v1";

pub const CFM_PUBLISHER: [u8; 32] = [
    0xbb, 0x30, 0x29, 0x59, 0xd2, 0xc3, 0xa5, 0x5a, 0xdc, 0x97, 0xdf, 0x1c, 0x1a, 0xdf, 0xef, 0xff,
    0x31, 0x0e, 0xda, 0x20, 0x63, 0x9e, 0x24, 0x5c, 0x3a, 0x68, 0x85, 0x4b, 0x52, 0x75, 0xed, 0xd7,
];

/// One launchable product: its period product id, its registry and its terminal catalog (the
/// governed product SKU manifest in `governance/`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CfmProduct {
    pub code: u8,
    pub symbol: &'static str,
    /// The period product id: the symbol, zero-padded to 32 bytes.
    pub product_id: [u8; 32],
    pub parent_count: u8,
    pub terminal_count: u16,
    pub terminal_root: [u8; 32],
    pub registry_hash: [u8; 32],
    /// The launch month's coverage root: the index-bound SKU Merkle root over the registry's
    /// parent source ids (one bucket per parent row), as tag 190 computes roots.
    pub coverage_root: [u8; 32],
    /// Parent rows assessed directly (no terminal children) and their total weight.
    pub direct_count: u8,
    pub direct_weight_bps: u16,
}

const fn padded(symbol: &str) -> [u8; 32] {
    let bytes = symbol.as_bytes();
    let mut id = [0u8; 32];
    let mut index = 0;
    while index < bytes.len() {
        id[index] = bytes[index];
        index += 1;
    }
    id
}

pub const RAMX: CfmProduct = CfmProduct {
    code: 0,
    symbol: "RAMX",
    product_id: padded("RAMX"),
    parent_count: 13,
    terminal_count: 52,
    terminal_root: [
        0x52, 0xa5, 0x74, 0xe7, 0xfe, 0xe1, 0x2f, 0x99, 0x21, 0xb3, 0xb2, 0x20, 0xb7, 0x09, 0xbe,
        0x16, 0xc1, 0x3a, 0x6f, 0x29, 0x0a, 0xbb, 0xdc, 0x2c, 0xc2, 0x03, 0xed, 0x11, 0x91, 0xec,
        0xf5, 0x78,
    ],
    registry_hash: [
        0x35, 0xb2, 0xa6, 0xde, 0x57, 0x30, 0x6c, 0xa2, 0x66, 0xc6, 0x9a, 0xbb, 0x34, 0xcc, 0x9c,
        0x28, 0x3e, 0xfc, 0x30, 0xc5, 0xc2, 0xc2, 0x40, 0x3f, 0xc0, 0xe4, 0xdc, 0xa1, 0xfe, 0x73,
        0xeb, 0x8a,
    ],
    coverage_root: [
        0xcb, 0xd4, 0x9e, 0xa9, 0x8d, 0x47, 0x5f, 0x51, 0xc5, 0x55, 0xbf, 0xa0, 0xad, 0x78, 0x37,
        0x80, 0x6e, 0x85, 0xce, 0x3f, 0x83, 0x6e, 0xe9, 0x9e, 0xf1, 0xf3, 0x8d, 0x12, 0x19, 0x9c,
        0x3d, 0x4e,
    ],
    direct_count: 0,
    direct_weight_bps: 0,
};

pub const NANDX: CfmProduct = CfmProduct {
    code: 1,
    symbol: "NANDX",
    product_id: padded("NANDX"),
    parent_count: 22,
    terminal_count: 48,
    terminal_root: [
        0x41, 0xbb, 0x5d, 0xc7, 0x9b, 0xac, 0xab, 0xb3, 0xe0, 0x87, 0x6b, 0x55, 0xe6, 0x47, 0xb0,
        0x10, 0x84, 0xc7, 0xfe, 0x8b, 0x39, 0x84, 0x6f, 0xd7, 0x07, 0xee, 0x0f, 0xd9, 0x73, 0xd2,
        0x8c, 0x3d,
    ],
    registry_hash: [
        0x93, 0xfc, 0x23, 0x8f, 0xc2, 0x71, 0xa4, 0x5e, 0xdf, 0x93, 0xe0, 0x5a, 0xdf, 0x1a, 0x52,
        0x1f, 0x60, 0xf2, 0x12, 0xce, 0x8b, 0xaa, 0xca, 0x04, 0xc6, 0x43, 0x0d, 0xfb, 0xdf, 0x96,
        0xb3, 0x56,
    ],
    coverage_root: [
        0x21, 0x12, 0x90, 0xb0, 0x15, 0x1e, 0xdb, 0x2e, 0xb8, 0x8a, 0x1e, 0x88, 0x88, 0xcd, 0x00,
        0xc8, 0xe5, 0x2b, 0xe3, 0xcb, 0x6b, 0x82, 0x4d, 0xb9, 0x58, 0x00, 0xcc, 0xe5, 0x5b, 0x8d,
        0x46, 0x2f,
    ],
    direct_count: 4,
    direct_weight_bps: 2_500,
};

impl CfmProduct {
    pub fn by_code(code: u8) -> Result<Self, ProgramError> {
        match code {
            0 => Ok(RAMX),
            1 => Ok(NANDX),
            _ => Err(OracleError::InvalidCfmLaunchPlan.into()),
        }
    }

    pub fn plan_len(&self) -> usize {
        PLAN_HEADER_LEN + usize::from(self.parent_count) * PLAN_ROW_LEN
    }

    pub fn registry_len(&self) -> usize {
        REGISTRY_HEADER_LEN + usize::from(self.parent_count) * REGISTRY_ROW_LEN
    }
}

/// `sha256(SOURCE_TYPE_DOMAIN)`, pinned (unit-tested) so the program does not hash it.
pub const SOURCE_TYPE_HASH: [u8; 32] = [
    0x48, 0xde, 0x48, 0xc2, 0x67, 0xc7, 0xb0, 0x8c, 0xb2, 0xb5, 0x37, 0x6c, 0xad, 0x83, 0xef, 0x31,
    0x25, 0x0c, 0xa7, 0xfd, 0x32, 0x47, 0xc9, 0x8c, 0xc0, 0x67, 0xf2, 0x76, 0x42, 0x49, 0xfc, 0x8f,
];

pub fn snapshot_hash(
    plan_hash: &[u8; 32],
    source_id: &[u8; 32],
    value: u64,
    time: u64,
) -> [u8; 32] {
    hashv(&[
        SNAPSHOT_DOMAIN,
        plan_hash,
        source_id,
        &value.to_le_bytes(),
        &time.to_le_bytes(),
    ])
    .to_bytes()
}

pub fn plan_digest(program: &Pubkey, plan: &[u8]) -> [u8; 32] {
    hashv(&[PLAN_DOMAIN, program.as_ref(), plan]).to_bytes()
}

pub fn registry_digest(registry: &[u8]) -> [u8; 32] {
    hashv(&[REGISTRY_DOMAIN, registry]).to_bytes()
}

pub fn find_receipt_address(program: &Pubkey, product_id: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            RECEIPT_SEED,
            product_id,
        ],
        program,
    )
}

fn invalid<T>() -> Result<T, ProgramError> {
    Err(OracleError::InvalidCfmLaunchPlan.into())
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    let mut value = [0u8; 8];
    value.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(value)
}

fn bytes32_at(bytes: &[u8], at: usize) -> [u8; 32] {
    let mut value = [0u8; 32];
    value.copy_from_slice(&bytes[at..at + 32]);
    value
}

/// One parent row of a registry, as a source of the launch month: bucket id = source id.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RegistryRow {
    pub source_id: [u8; 32],
    pub locator_hash: [u8; 32],
    pub definition_hash: [u8; 32],
    pub weight_bps: u16,
}

/// A product registry's bytes (without the domain).
///
/// Wire: version u8 (1), product u8, terminal count u16, terminal root, publisher, row count
/// u32, then per row source id, locator hash, definition hash, weight u16, terminal mask u64
/// and a direct-assessment byte, in strictly ascending source id.
///
/// On chain the registry is read from a sealed evidence object whose commitment is the pinned
/// hash, which fixes every byte; [`CfmRegistry::authenticated`] then checks only its shape.
/// The pinned registries' content rules are checked off chain ([`CfmRegistry::decode`] in the
/// compiler and the tests).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CfmRegistry {
    product: CfmProduct,
    bytes: Vec<u8>,
}

impl CfmRegistry {
    /// `bytes` already hash to `product.registry_hash`.
    pub fn authenticated(product: CfmProduct, bytes: Vec<u8>) -> Result<Self, ProgramError> {
        if bytes.len() != product.registry_len() || bytes[1] != product.code {
            return invalid();
        }
        Ok(Self { product, bytes })
    }

    /// Decodes registry bytes and checks the pinned hash and every content rule: the header,
    /// strictly ascending nonzero rows, weights summing to 10,000 bps, a terminal mask that
    /// covers the catalog exactly once, and the product's direct-assessment rows.
    #[cfg(not(target_os = "solana"))]
    pub fn decode(bytes: &[u8]) -> Result<Self, ProgramError> {
        if bytes.len() < REGISTRY_HEADER_LEN {
            return invalid();
        }
        let product = CfmProduct::by_code(bytes[1])?;
        if registry_digest(bytes) != product.registry_hash
            || bytes[0] != 1
            || bytes[2..4] != product.terminal_count.to_le_bytes()
            || bytes[4..36] != product.terminal_root
            || bytes[36..68] != CFM_PUBLISHER
            || bytes[68..72] != u32::from(product.parent_count).to_le_bytes()
        {
            return invalid();
        }
        let registry = Self::authenticated(product, bytes.to_vec())?;
        let complete_mask = (1u64 << product.terminal_count) - 1;
        let (mut mask, mut weight, mut direct, mut direct_weight) = (0u64, 0u32, 0u8, 0u16);
        let mut previous = [0u8; 32];
        for index in 0..product.parent_count {
            let row = registry.row(index)?;
            let at = REGISTRY_HEADER_LEN + usize::from(index) * REGISTRY_ROW_LEN;
            let terminal_mask = u64_at(bytes, at + 98);
            let is_direct = bytes[at + 106];
            if row.source_id <= previous
                || row.locator_hash == [0; 32]
                || row.definition_hash == [0; 32]
                || row.weight_bps == 0
                || is_direct > 1
                || (is_direct == 1) != (terminal_mask == 0)
                || terminal_mask & !complete_mask != 0
                || terminal_mask & mask != 0
            {
                return invalid();
            }
            previous = row.source_id;
            mask |= terminal_mask;
            weight += u32::from(row.weight_bps);
            if is_direct == 1 {
                direct += 1;
                direct_weight += row.weight_bps;
            }
        }
        if mask != complete_mask
            || weight != 10_000
            || direct != product.direct_count
            || direct_weight != product.direct_weight_bps
        {
            return invalid();
        }
        Ok(registry)
    }

    pub fn product(&self) -> CfmProduct {
        self.product
    }

    pub fn len(&self) -> u8 {
        self.product.parent_count
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn row(&self, index: u8) -> Result<RegistryRow, ProgramError> {
        if index >= self.product.parent_count {
            return invalid();
        }
        let at = REGISTRY_HEADER_LEN + usize::from(index) * REGISTRY_ROW_LEN;
        let bytes = &self.bytes;
        Ok(RegistryRow {
            source_id: bytes32_at(bytes, at),
            locator_hash: bytes32_at(bytes, at + 32),
            definition_hash: bytes32_at(bytes, at + 64),
            weight_bps: u16::from_le_bytes([bytes[at + 96], bytes[at + 97]]),
        })
    }
}

/// The SKU pool terms of every bucket of the launch month, as tag 157 takes them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PoolTerms {
    pub proposer_reward_bps: u16,
    pub challenge_bond_bps: u16,
    pub listing_bond: u64,
    pub support_bond: u64,
    pub opening_bond: u64,
    pub update_min_bond: u64,
    pub challenge_min_bond: u64,
    pub challenge_max_bond: u64,
}

impl PoolTerms {
    /// The validity rules of tag 157.
    pub fn validate(&self) -> Result<(), ProgramError> {
        if self.proposer_reward_bps == 0
            || self.proposer_reward_bps > 10_000
            || self.listing_bond == 0
            || self.support_bond == 0
            || self.opening_bond == 0
            || self.update_min_bond == 0
            || self.challenge_min_bond == 0
            || self.challenge_max_bond == 0
            || self.challenge_min_bond > self.challenge_max_bond
            || self.challenge_bond_bps > 10_000
        {
            return Err(OracleError::InvalidOracleUsdcSkuPool.into());
        }
        Ok(())
    }
}

/// A council-approved launch plan for one product, validated.
///
/// Wire (little-endian): magic `ACL1`, version 1, product code, the SHA-256 of the handoff
/// document the council approved, the period expiry, the product's registry hash, the
/// settlement base (six decimals), the pool terms (proposer reward bps, challenge bond bps, then
/// the listing, support, opening, minimum update, minimum and maximum challenge bonds), then per
/// registry row in registry order the printed value (six decimals) and its publisher print
/// time (unix seconds).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CfmPlanView {
    product: CfmProduct,
    bytes: Vec<u8>,
}

impl CfmPlanView {
    /// Validates plan bytes (without the domain and program id). Print times must not be after
    /// `now`.
    pub fn decode(bytes: Vec<u8>, now: u64) -> Result<Self, ProgramError> {
        if bytes.len() < PLAN_HEADER_LEN || bytes[..4] != PLAN_MAGIC || bytes[4] != PLAN_VERSION {
            return invalid();
        }
        let plan = Self {
            product: CfmProduct::by_code(bytes[5])?,
            bytes,
        };
        if plan.bytes.len() != plan.product.plan_len()
            || plan.document_sha256() == [0; 32]
            || plan.bytes[46..78] != plan.product.registry_hash
            || plan.settlement_base_oracle_atomic() == 0
        {
            return invalid();
        }
        for index in 0..plan.product.parent_count {
            let (value, time) = plan.row(index)?;
            if value == 0 || time == 0 || time > now {
                return invalid();
            }
        }
        plan.terms().validate()?;
        Ok(plan)
    }

    pub fn product(&self) -> CfmProduct {
        self.product
    }

    pub fn document_sha256(&self) -> [u8; 32] {
        bytes32_at(&self.bytes, 6)
    }

    pub fn expiry_ts(&self) -> u64 {
        u64_at(&self.bytes, 38)
    }

    pub fn settlement_base_oracle_atomic(&self) -> u64 {
        u64_at(&self.bytes, 78)
    }

    pub fn terms(&self) -> PoolTerms {
        let b = &self.bytes;
        PoolTerms {
            proposer_reward_bps: u16::from_le_bytes([b[86], b[87]]),
            challenge_bond_bps: u16::from_le_bytes([b[88], b[89]]),
            listing_bond: u64_at(b, 90),
            support_bond: u64_at(b, 98),
            opening_bond: u64_at(b, 106),
            update_min_bond: u64_at(b, 114),
            challenge_min_bond: u64_at(b, 122),
            challenge_max_bond: u64_at(b, 130),
        }
    }

    /// `(value, print time)` of a registry row.
    pub fn row(&self, index: u8) -> Result<(u64, u64), ProgramError> {
        if index >= self.product.parent_count {
            return invalid();
        }
        let at = PLAN_HEADER_LEN + usize::from(index) * PLAN_ROW_LEN;
        Ok((u64_at(&self.bytes, at), u64_at(&self.bytes, at + 8)))
    }
}

/// A launch plan as tooling builds it ([`CfmPlan::encode`] gives the wire of [`CfmPlanView`]).
#[cfg(not(target_os = "solana"))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CfmPlan {
    pub product: CfmProduct,
    pub document_sha256: [u8; 32],
    pub expiry_ts: u64,
    pub settlement_base_oracle_atomic: u64,
    pub terms: PoolTerms,
    /// `(value, print time)` per registry row.
    pub rows: Vec<(u64, u64)>,
}

#[cfg(not(target_os = "solana"))]
impl CfmPlan {
    pub fn decode(bytes: &[u8], now: u64) -> Result<Self, ProgramError> {
        let view = CfmPlanView::decode(bytes.to_vec(), now)?;
        Ok(Self {
            product: view.product,
            document_sha256: view.document_sha256(),
            expiry_ts: view.expiry_ts(),
            settlement_base_oracle_atomic: view.settlement_base_oracle_atomic(),
            terms: view.terms(),
            rows: (0..view.product.parent_count)
                .map(|index| view.row(index))
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.product.plan_len());
        out.extend_from_slice(&PLAN_MAGIC);
        out.push(PLAN_VERSION);
        out.push(self.product.code);
        out.extend_from_slice(&self.document_sha256);
        out.extend_from_slice(&self.expiry_ts.to_le_bytes());
        out.extend_from_slice(&self.product.registry_hash);
        out.extend_from_slice(&self.settlement_base_oracle_atomic.to_le_bytes());
        let t = &self.terms;
        out.extend_from_slice(&t.proposer_reward_bps.to_le_bytes());
        out.extend_from_slice(&t.challenge_bond_bps.to_le_bytes());
        for bond in [
            t.listing_bond,
            t.support_bond,
            t.opening_bond,
            t.update_min_bond,
            t.challenge_min_bond,
            t.challenge_max_bond,
        ] {
            out.extend_from_slice(&bond.to_le_bytes());
        }
        for (value, time) in &self.rows {
            out.extend_from_slice(&value.to_le_bytes());
            out.extend_from_slice(&time.to_le_bytes());
        }
        out
    }

    pub fn row(&self, index: u8) -> Result<(u64, u64), ProgramError> {
        self.rows
            .get(usize::from(index))
            .copied()
            .ok_or_else(|| OracleError::InvalidCfmLaunchPlan.into())
    }
}

/// Created once per product by the launch's begin step; the create-only PDA is the one-shot
/// rule. `cursor` counts the appended rows; `finished` is set once the period is Live.
///
/// Layout (259 bytes, little-endian): discriminator `OCLR`, version, bump, product, row count,
/// cursor, finished (0/1), period, month, plan hash, registry hash, document SHA-256, proposer,
/// council epoch u64, seats hash, approving seat mask (bit `i` = seat `i` of the Config), begun
/// at u64, finished at u64.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CfmLaunchReceipt {
    pub discriminator: [u8; 4],
    pub version: u8,
    pub bump: u8,
    pub product: u8,
    pub row_count: u8,
    pub cursor: u8,
    pub finished: bool,
    pub period: Pubkey,
    pub month: Pubkey,
    pub plan_hash: [u8; 32],
    pub registry_hash: [u8; 32],
    pub document_sha256: [u8; 32],
    pub proposer: Pubkey,
    pub council_epoch: u64,
    pub seats_hash: [u8; 32],
    pub approving_seats: u8,
    pub begun_at: u64,
    pub finished_at: u64,
}

impl CfmLaunchReceipt {
    pub const LEN: usize = 259;
}

/// Host-side compilation of a product registry from the parsed registry document
/// (`governance/cfm_parent_proxy_registry.v1.json`) and the product's terminal SKU labels
/// (`governance/<product>_product_sku_manifest.v1.json`). Callers parse the JSON; the bytes,
/// hashes and the pinned-registry check live here.
#[cfg(not(target_os = "solana"))]
pub mod compile {
    use super::*;

    /// One `parentRows` entry of the registry document.
    #[derive(Clone, Debug)]
    pub struct ParentRowInput {
        pub id: String,
        pub product: String,
        pub parent_source_key: String,
        pub parent_source_id: [u8; 32],
        pub canonical_locator: String,
        pub locator_hash: [u8; 32],
        pub definition_transcript: String,
        pub definition_hash: [u8; 32],
        pub weight_bps: u16,
        pub direct_assessment: bool,
        pub children: Vec<String>,
    }

    /// A compiled product registry and what the launch uploads beside it.
    #[derive(Clone, Debug)]
    pub struct CompiledRegistry {
        pub product: CfmProduct,
        /// Registry bytes without the domain; [`registry_preimage`] adds it.
        pub bytes: Vec<u8>,
        pub digest: [u8; 32],
        /// The product's parent rows in registry (ascending source id) order.
        pub parents: Vec<ParentRowInput>,
    }

    impl CompiledRegistry {
        /// The sealed kind-2 evidence preimage whose commitment is the pinned registry hash.
        pub fn preimage(&self) -> Vec<u8> {
            [REGISTRY_DOMAIN, &self.bytes].concat()
        }
    }

    fn err(message: &str) -> String {
        format!("CFM registry: {message}")
    }

    /// The 32-byte terminal SKU id of a label: printable ASCII, right zero-padded.
    pub fn sku_id(label: &str) -> Result<[u8; 32], String> {
        if label.is_empty()
            || label.len() > 32
            || !label.bytes().all(|b| (0x20..=0x7e).contains(&b))
        {
            return Err(err(&format!("invalid terminal label {label:?}")));
        }
        let mut id = [0u8; 32];
        id[..label.len()].copy_from_slice(label.as_bytes());
        Ok(id)
    }

    /// Compiles `product`'s registry from every parent row of the document and the product's
    /// terminal labels, and refuses anything but the pinned registry.
    pub fn compile_registry(
        product: CfmProduct,
        rows: &[ParentRowInput],
        terminal_labels: &[String],
    ) -> Result<CompiledRegistry, String> {
        let mut terminals: Vec<[u8; 32]> = terminal_labels
            .iter()
            .map(|l| sku_id(l))
            .collect::<Result<_, _>>()?;
        terminals.sort_unstable();
        terminals.dedup();
        if terminals.len() != usize::from(product.terminal_count) {
            return Err(err("terminal catalog size"));
        }
        let mut parents: Vec<ParentRowInput> = rows
            .iter()
            .filter(|r| r.product == product.symbol)
            .cloned()
            .collect();
        if parents.len() != usize::from(product.parent_count) {
            return Err(err(&format!(
                "{} parent rows for {}",
                parents.len(),
                product.symbol
            )));
        }
        parents.sort_by(|a, b| a.parent_source_id.cmp(&b.parent_source_id));
        let mut bytes = Vec::with_capacity(product.registry_len());
        bytes.push(1);
        bytes.push(product.code);
        bytes.extend_from_slice(&product.terminal_count.to_le_bytes());
        bytes.extend_from_slice(&product.terminal_root);
        bytes.extend_from_slice(&CFM_PUBLISHER);
        bytes.extend_from_slice(&u32::from(product.parent_count).to_le_bytes());
        for row in &parents {
            let key_id = hashv(&[
                b"amoeba-cfm-parent-proxy-v1\0",
                row.parent_source_key.as_bytes(),
            ]);
            if key_id.to_bytes() != row.parent_source_id
                || hashv(&[b"locator", row.canonical_locator.as_bytes()]).to_bytes()
                    != row.locator_hash
                || hashv(&[row.definition_transcript.as_bytes()]).to_bytes() != row.definition_hash
                || row.direct_assessment != row.children.is_empty()
            {
                return Err(err(&format!("row {} does not match its hashes", row.id)));
            }
            let mut mask = 0u64;
            for child in &row.children {
                let id = sku_id(child)?;
                let index = terminals
                    .binary_search(&id)
                    .map_err(|_| err(&format!("row {} child {child} is not a terminal", row.id)))?;
                mask |= 1u64 << index;
            }
            bytes.extend_from_slice(&row.parent_source_id);
            bytes.extend_from_slice(&row.locator_hash);
            bytes.extend_from_slice(&row.definition_hash);
            bytes.extend_from_slice(&row.weight_bps.to_le_bytes());
            bytes.extend_from_slice(&mask.to_le_bytes());
            bytes.push(u8::from(row.direct_assessment));
        }
        let digest = registry_digest(&bytes);
        if digest != product.registry_hash {
            return Err(err(&format!(
                "{} does not compile to the pinned registry",
                product.symbol
            )));
        }
        CfmRegistry::decode(&bytes).map_err(|_| err("the pinned registry fails its own checks"))?;
        Ok(CompiledRegistry {
            product,
            bytes,
            digest,
            parents,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_bytes(product: CfmProduct) -> Vec<u8> {
        CfmPlan {
            product,
            document_sha256: [7; 32],
            expiry_ts: 1_796_083_200,
            settlement_base_oracle_atomic: 100_000_000,
            terms: PoolTerms {
                proposer_reward_bps: 2_500,
                challenge_bond_bps: 2_000,
                listing_bond: 1,
                support_bond: 1,
                opening_bond: 1,
                update_min_bond: 1,
                challenge_min_bond: 1,
                challenge_max_bond: 10,
            },
            rows: (0..u64::from(product.parent_count))
                .map(|i| (1_000_000 + i, 1_790_000_000 + i))
                .collect(),
        }
        .encode()
    }

    #[test]
    fn plan_roundtrips_at_its_exact_length() {
        for product in [RAMX, NANDX] {
            let bytes = plan_bytes(product);
            assert_eq!(bytes.len(), product.plan_len());
            let plan = CfmPlan::decode(&bytes, 1_800_000_000).unwrap();
            assert_eq!(plan.encode(), bytes);
            assert_eq!(plan.row(0).unwrap(), (1_000_000, 1_790_000_000));
            assert!(plan.row(product.parent_count).is_err());
            assert!(CfmPlan::decode(&bytes[..bytes.len() - 1], 1_800_000_000).is_err());
            let mut long = bytes.clone();
            long.push(0);
            assert!(CfmPlan::decode(&long, 1_800_000_000).is_err());
        }
        assert_eq!(plan_bytes(RAMX).len(), 346);
        assert_eq!(plan_bytes(NANDX).len(), 490);
    }

    #[test]
    fn plan_rejects_every_invalid_header_field_and_row() {
        let good = plan_bytes(RAMX);
        let now = 1_800_000_000;
        let mutate = |at: usize, value: &[u8]| {
            let mut bytes = good.clone();
            bytes[at..at + value.len()].copy_from_slice(value);
            CfmPlan::decode(&bytes, now)
        };
        assert!(mutate(0, b"SCB1").is_err(), "magic");
        assert!(mutate(4, &[2]).is_err(), "version");
        assert!(
            mutate(5, &[1]).is_err(),
            "product code with a RAMX-sized body"
        );
        assert!(mutate(5, &[2]).is_err(), "unknown product");
        assert!(mutate(6, &[0; 32]).is_err(), "document digest");
        assert!(mutate(46, &NANDX.registry_hash).is_err(), "registry hash");
        assert!(mutate(78, &0u64.to_le_bytes()).is_err(), "base");
        assert!(mutate(86, &0u16.to_le_bytes()).is_err(), "proposer reward");
        assert!(
            mutate(88, &10_001u16.to_le_bytes()).is_err(),
            "challenge bps"
        );
        assert!(mutate(90, &0u64.to_le_bytes()).is_err(), "listing bond");
        assert!(
            mutate(122, &11u64.to_le_bytes()).is_err(),
            "challenge min above max"
        );
        assert!(
            mutate(PLAN_HEADER_LEN, &0u64.to_le_bytes()).is_err(),
            "zero value"
        );
        assert!(
            mutate(PLAN_HEADER_LEN + 8, &0u64.to_le_bytes()).is_err(),
            "zero time"
        );
        assert!(
            mutate(PLAN_HEADER_LEN + 8, &(now + 1).to_le_bytes()).is_err(),
            "a print after now"
        );
        assert!(CfmPlan::decode(&good, now).is_ok());
    }

    #[test]
    fn plan_digest_binds_the_program_and_every_byte() {
        let bytes = plan_bytes(NANDX);
        let program = Pubkey::new_unique();
        let digest = plan_digest(&program, &bytes);
        assert_eq!(
            digest,
            hashv(&[&[PLAN_DOMAIN, program.as_ref(), &bytes].concat()]).to_bytes()
        );
        assert_ne!(digest, plan_digest(&Pubkey::new_unique(), &bytes));
        let mut other = bytes.clone();
        *other.last_mut().unwrap() ^= 1;
        assert_ne!(digest, plan_digest(&program, &other));
    }

    #[test]
    fn receipt_roundtrips_at_its_fixed_length() {
        let receipt = CfmLaunchReceipt {
            discriminator: RECEIPT_DISCRIMINATOR,
            version: 1,
            bump: 254,
            product: 1,
            row_count: 22,
            cursor: 7,
            finished: true,
            period: Pubkey::new_unique(),
            month: Pubkey::new_unique(),
            plan_hash: [1; 32],
            registry_hash: [2; 32],
            document_sha256: [3; 32],
            proposer: Pubkey::new_unique(),
            council_epoch: u64::MAX - 1,
            seats_hash: [4; 32],
            approving_seats: 0b10101,
            begun_at: 11,
            finished_at: u64::MAX,
        };
        use borsh::{BorshDeserialize, BorshSerialize};
        let bytes = receipt.try_to_vec().unwrap();
        assert_eq!(bytes.len(), CfmLaunchReceipt::LEN);
        assert_eq!(bytes[9], 1, "finished");
        assert_eq!(&bytes[243..251], &11u64.to_le_bytes(), "begun at");
        assert_eq!(CfmLaunchReceipt::try_from_slice(&bytes).unwrap(), receipt);
        assert!(CfmLaunchReceipt::try_from_slice(&bytes[..258]).is_err());
        let mut bad = bytes;
        bad[9] = 2;
        assert!(CfmLaunchReceipt::try_from_slice(&bad).is_err());
    }

    #[test]
    fn the_source_type_is_the_hash_of_its_domain() {
        assert_eq!(SOURCE_TYPE_HASH, hashv(&[SOURCE_TYPE_DOMAIN]).to_bytes());
    }

    #[test]
    fn products_are_bound_to_their_period_ids() {
        assert_eq!(&RAMX.product_id[..4], b"RAMX");
        assert_eq!(&NANDX.product_id[..5], b"NANDX");
        assert!(RAMX.product_id[4..].iter().all(|b| *b == 0));
        assert_eq!(CfmProduct::by_code(0).unwrap(), RAMX);
        assert_eq!(CfmProduct::by_code(1).unwrap(), NANDX);
        assert!(CfmProduct::by_code(2).is_err());
    }
}
