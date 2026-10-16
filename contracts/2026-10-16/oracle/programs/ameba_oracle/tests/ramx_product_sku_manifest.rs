use std::collections::HashSet;

use ameba_oracle::constants::{
    MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS, MAX_ORACLE_REQUIRED_SKUS, ORACLE_SKU_EMPTY_HASH_DOMAIN,
    ORACLE_SKU_LEAF_HASH_DOMAIN, ORACLE_SKU_NODE_HASH_DOMAIN,
};
use serde::Deserialize;
use solana_program::hash::hashv;

const MANIFEST_SOURCE: &str = include_str!("../../../governance/ramx_product_sku_manifest.v1.json");
const PINNED_SOURCE_SHA256: &str =
    "c89628e1656c82572b4f1b089388674aff0fae2aa3c3db1167da602a25b1cd3f";
const PINNED_REQUIRED_SKU_ROOT_HEX: &str =
    "52a574e7fee12f9921b3b220b709be16c13a6f290abbdc2cc203ed1191ecf578";

const DDR5_RDIMM_SUBGROUPS: &[(&str, u16)] = &[("32GB", 5), ("64GB", 5), ("96GB", 4)];
const DDR4_RDIMM_SUBGROUPS: &[(&str, u16)] = &[("16GB", 4), ("32GB", 4), ("64GB", 4)];
const DDR5_UDIMM_SUBGROUPS: &[(&str, u16)] = &[("16GB 5600", 3), ("32GB 5600", 3)];
const DDR4_UDIMM_SUBGROUPS: &[(&str, u16)] = &[("8GB 3200", 4), ("16GB 3200", 4), ("32GB 3200", 4)];
const DDR4_SODIMM_SUBGROUPS: &[(&str, u16)] = &[("8GB 3200", 4), ("16GB 3200", 4)];
type ExpectedGroupLayoutEntry = (&'static str, u16, &'static [(&'static str, u16)]);

const EXPECTED_GROUP_LAYOUT: &[ExpectedGroupLayoutEntry] = &[
    ("DDR5 RDIMM", 14, DDR5_RDIMM_SUBGROUPS),
    ("DDR4 RDIMM", 12, DDR4_RDIMM_SUBGROUPS),
    ("DDR5 UDIMM", 6, DDR5_UDIMM_SUBGROUPS),
    ("DDR4 UDIMM", 12, DDR4_UDIMM_SUBGROUPS),
    ("DDR4 SODIMM", 8, DDR4_SODIMM_SUBGROUPS),
];

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GovernedProductManifest {
    schema: String,
    schema_version: u16,
    manifest_version: u16,
    product: ProductIdentity,
    provenance: Provenance,
    canonical_label_encoding: CanonicalLabelEncoding,
    tag190_plan: Tag190Plan,
    required_sku_count: u16,
    required_sku_root_hex: String,
    groups: Vec<ProductGroup>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProductIdentity {
    market_id: String,
    symbol: String,
    underlying_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Provenance {
    source_description: String,
    source_sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CanonicalLabelEncoding {
    name: String,
    minimum_bytes: usize,
    maximum_bytes: usize,
    sort: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Tag190Plan {
    instruction_tag: u16,
    maximum_chunk_sku_ids: usize,
    chunk_sku_counts: Vec<usize>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProductGroup {
    name: String,
    sku_count: u16,
    subgroups: Vec<ProductSubgroup>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProductSubgroup {
    name: String,
    sku_count: u16,
    sku_labels: Vec<String>,
}

#[derive(Debug)]
struct ValidatedManifest {
    sku_ids: Vec<[u8; 32]>,
    required_sku_root: [u8; 32],
}

fn parse_manifest() -> GovernedProductManifest {
    serde_json::from_str(MANIFEST_SOURCE).expect("governed RAMX manifest must use its exact schema")
}

fn encode_canonical_ascii_label(label: &str) -> Result<[u8; 32], String> {
    let bytes = label.as_bytes();
    if bytes.is_empty() || bytes.len() > 32 {
        return Err("label must contain 1 through 32 UTF-8 bytes".to_string());
    }
    if bytes.iter().any(|byte| !(0x20..=0x7e).contains(byte)) {
        return Err("label must contain printable ASCII only".to_string());
    }
    let mut encoded = [0u8; 32];
    encoded[..bytes.len()].copy_from_slice(bytes);
    Ok(encoded)
}

fn decode_canonical_ascii_label(encoded: &[u8; 32]) -> Result<String, String> {
    let length = encoded.iter().position(|byte| *byte == 0).unwrap_or(32);
    if length == 0 {
        return Err("encoded label must be nonempty".to_string());
    }
    if encoded[length..].iter().any(|byte| *byte != 0) {
        return Err("encoded label must use right-zero padding only".to_string());
    }
    if encoded[..length]
        .iter()
        .any(|byte| !(0x20..=0x7e).contains(byte))
    {
        return Err("encoded label must contain printable ASCII only".to_string());
    }
    let decoded = std::str::from_utf8(&encoded[..length])
        .map_err(|_| "encoded label must contain valid UTF-8".to_string())?
        .to_string();
    if encode_canonical_ascii_label(&decoded)? != *encoded {
        return Err("encoded label is not canonical".to_string());
    }
    Ok(decoded)
}

fn parse_lower_hex_32(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err("root must be exactly 32 lowercase hexadecimal bytes".to_string());
    }
    let nibble = |byte: u8| -> u8 {
        match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => unreachable!("validated lowercase hexadecimal nibble"),
        }
    };
    let source = value.as_bytes();
    let mut decoded = [0u8; 32];
    for (index, byte) in decoded.iter_mut().enumerate() {
        *byte = (nibble(source[index * 2]) << 4) | nibble(source[index * 2 + 1]);
    }
    Ok(decoded)
}

fn derive_on_chain_compatible_root(sku_ids: &[[u8; 32]]) -> Result<[u8; 32], String> {
    if sku_ids.is_empty() || sku_ids.len() > usize::from(MAX_ORACLE_REQUIRED_SKUS) {
        return Err("terminal-SKU set is outside the on-chain count bound".to_string());
    }
    let mut width = 1usize;
    while width < sku_ids.len() {
        width = width
            .checked_mul(2)
            .ok_or_else(|| "Merkle width overflow".to_string())?;
    }
    let mut nodes = Vec::with_capacity(width);
    for index in 0..width {
        let index = u16::try_from(index).map_err(|_| "Merkle index overflow".to_string())?;
        nodes.push(if let Some(sku_id) = sku_ids.get(usize::from(index)) {
            hashv(&[ORACLE_SKU_LEAF_HASH_DOMAIN, &index.to_le_bytes(), sku_id]).to_bytes()
        } else {
            hashv(&[ORACLE_SKU_EMPTY_HASH_DOMAIN, &index.to_le_bytes()]).to_bytes()
        });
    }
    while nodes.len() > 1 {
        let mut next = Vec::with_capacity(nodes.len() / 2);
        for pair in nodes.chunks_exact(2) {
            next.push(hashv(&[ORACLE_SKU_NODE_HASH_DOMAIN, &pair[0], &pair[1]]).to_bytes());
        }
        nodes = next;
    }
    nodes
        .first()
        .copied()
        .ok_or_else(|| "Merkle root is missing".to_string())
}

fn validate_manifest(manifest: &GovernedProductManifest) -> Result<ValidatedManifest, String> {
    if manifest.schema != "amoeba.oracle-product-sku-manifest"
        || manifest.schema_version != 1
        || manifest.manifest_version != 1
    {
        return Err("manifest schema/version mismatch".to_string());
    }
    if manifest.product.market_id != "ramx"
        || manifest.product.symbol != "RAMX"
        || manifest.product.underlying_id != "ram-standardized-baskets"
    {
        return Err("RAMX product identity mismatch".to_string());
    }
    if manifest.provenance.source_description != "matching thumb-drive/maintained RAMX module JSON"
        || manifest.provenance.source_sha256 != PINNED_SOURCE_SHA256
    {
        return Err("RAMX manifest provenance mismatch".to_string());
    }
    if manifest.canonical_label_encoding.name != "printable-ascii-utf8-right-zero-padded-bytes32-v1"
        || manifest.canonical_label_encoding.minimum_bytes != 1
        || manifest.canonical_label_encoding.maximum_bytes != 32
        || manifest.canonical_label_encoding.sort != "ascending-encoded-bytes"
    {
        return Err("canonical label encoding contract mismatch".to_string());
    }
    if manifest.tag190_plan.instruction_tag != 190
        || manifest.tag190_plan.maximum_chunk_sku_ids != MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS
        || manifest.tag190_plan.chunk_sku_counts != [16, 16, 16, 4]
    {
        return Err("tag-190 packet plan mismatch".to_string());
    }
    if manifest.required_sku_count != RAMX_DOCUMENT_SKU_COUNT {
        return Err("RAMX canonical SKU count mismatch".to_string());
    }
    if manifest.required_sku_root_hex != PINNED_REQUIRED_SKU_ROOT_HEX {
        return Err("RAMX pinned SKU root mismatch".to_string());
    }
    if manifest.groups.len() != EXPECTED_GROUP_LAYOUT.len() {
        return Err("RAMX group count mismatch".to_string());
    }

    let mut sku_ids = Vec::new();
    let mut seen = HashSet::new();
    for (group, (expected_name, expected_count, expected_subgroups)) in
        manifest.groups.iter().zip(EXPECTED_GROUP_LAYOUT.iter())
    {
        if group.name != *expected_name
            || group.sku_count != *expected_count
            || group.subgroups.len() != expected_subgroups.len()
        {
            return Err(format!("group layout mismatch for {}", group.name));
        }
        let mut group_count = 0u16;
        for (subgroup, (expected_name, expected_count)) in
            group.subgroups.iter().zip(expected_subgroups.iter())
        {
            if subgroup.name != *expected_name
                || subgroup.sku_count != *expected_count
                || subgroup.sku_labels.len() != usize::from(*expected_count)
            {
                return Err(format!("subgroup layout mismatch for {}", subgroup.name));
            }
            group_count = group_count
                .checked_add(subgroup.sku_count)
                .ok_or_else(|| "group count overflow".to_string())?;
            for label in &subgroup.sku_labels {
                let encoded = encode_canonical_ascii_label(label)?;
                if decode_canonical_ascii_label(&encoded)? != *label {
                    return Err(format!("label does not round-trip exactly: {label}"));
                }
                if !seen.insert(encoded) {
                    return Err(format!("duplicate terminal-SKU label: {label}"));
                }
                sku_ids.push(encoded);
            }
        }
        if group_count != group.sku_count {
            return Err(format!(
                "group count does not match subgroups: {}",
                group.name
            ));
        }
    }
    if sku_ids.len() != usize::from(manifest.required_sku_count) {
        return Err("flattened terminal-SKU count mismatch".to_string());
    }
    sku_ids.sort_unstable();
    if sku_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("encoded terminal-SKU set is not strictly ascending".to_string());
    }
    let actual_chunk_counts = sku_ids
        .chunks(MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS)
        .map(|chunk| chunk.len())
        .collect::<Vec<_>>();
    if actual_chunk_counts != manifest.tag190_plan.chunk_sku_counts {
        return Err("flattened terminal-SKU chunk plan mismatch".to_string());
    }
    let required_sku_root = derive_on_chain_compatible_root(&sku_ids)?;
    if required_sku_root != parse_lower_hex_32(&manifest.required_sku_root_hex)? {
        return Err("derived terminal-SKU root mismatch".to_string());
    }
    Ok(ValidatedManifest {
        sku_ids,
        required_sku_root,
    })
}

/// The governed RAMX document registers 52 terminal SKUs. The program has no product table: this
/// count and the pinned root are data, registered once through tag 190.
const RAMX_DOCUMENT_SKU_COUNT: u16 = 52;

#[test]
fn governed_ramx_manifest_matches_rust_counts_codec_layout_and_merkle_contract() {
    let validated = validate_manifest(&parse_manifest()).expect("governed RAMX manifest is valid");
    assert_eq!(validated.sku_ids.len(), 52);
    assert_eq!(
        validated.required_sku_root,
        parse_lower_hex_32(PINNED_REQUIRED_SKU_ROOT_HEX).unwrap()
    );
    assert_eq!(
        validated
            .sku_ids
            .chunks(MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS)
            .map(|chunk| chunk.len())
            .collect::<Vec<_>>(),
        [16, 16, 16, 4]
    );
    assert_eq!(
        (0..validated.sku_ids.len())
            .step_by(MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS)
            .collect::<Vec<_>>(),
        [0, 16, 32, 48]
    );
}

#[test]
fn governed_ramx_manifest_rejects_duplicates_mutations_invalid_labels_padding_and_count_60() {
    let manifest = parse_manifest();

    let mut duplicate = manifest.clone();
    duplicate.groups[0].subgroups[0].sku_labels[1] =
        duplicate.groups[0].subgroups[0].sku_labels[0].clone();
    assert!(validate_manifest(&duplicate).is_err());

    let mut mutation = manifest.clone();
    mutation.groups[0].subgroups[0].sku_labels[0] = "MTC20F2085S1RC48Br".to_string();
    assert!(validate_manifest(&mutation).is_err());

    let mut overlength = manifest.clone();
    overlength.groups[0].subgroups[0].sku_labels[0] = "X".repeat(33);
    assert!(validate_manifest(&overlength).is_err());

    let mut non_ascii = manifest.clone();
    non_ascii.groups[0].subgroups[0].sku_labels[0] = "MÉMORY".to_string();
    assert!(validate_manifest(&non_ascii).is_err());

    let mut reverted_count = manifest;
    reverted_count.required_sku_count = 60;
    assert!(validate_manifest(&reverted_count).is_err());

    assert!(encode_canonical_ascii_label("").is_err());
    assert!(encode_canonical_ascii_label("line\nbreak").is_err());
    let mut malformed_padding = encode_canonical_ascii_label("ABC").unwrap();
    malformed_padding[4] = b'X';
    assert!(decode_canonical_ascii_label(&malformed_padding).is_err());
}
