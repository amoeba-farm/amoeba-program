use std::collections::HashSet;

use ameba_oracle::constants::{
    MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS, MAX_ORACLE_REQUIRED_SKUS, ORACLE_SKU_EMPTY_HASH_DOMAIN,
    ORACLE_SKU_LEAF_HASH_DOMAIN, ORACLE_SKU_NODE_HASH_DOMAIN,
};
use serde::Deserialize;
use solana_program::hash::{hash, hashv};

const MANIFEST_SOURCE: &str =
    include_str!("../../../governance/nandx_product_sku_manifest.v1.json");
const BASKET_SOURCE: &str = include_str!("../../../governance/nandx_benchmark_basket.v1.json");
const PINNED_SOURCE_SHA256: &str =
    "14e31bd83e0c92c5b3894df5d68aec1091373c0ae48b53775d12640b92cb1be7";
const PINNED_REQUIRED_SKU_ROOT_HEX: &str =
    "41bb5dc79bacabb3e0876b55e647b01084c7fe8b39846fd707ee0fd973d28c3d";
const PINNED_MANIFEST_LAYOUT_SHA256: &str =
    "b4da8563f5e5e773cadfe64ee93f056f1f57ab4439a6ea7b8b7a9554c1f08405";
const PINNED_BASKET_LAYOUT_SHA256: &str =
    "fea5a56a2ed2d470a797639056e4a6b1aebe1f0174984683aeac26bf8d620fa7";

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

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProductIdentity {
    market_id: String,
    symbol: String,
    underlying_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
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

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GovernedBenchmarkBasket {
    schema: String,
    schema_version: u16,
    basket_version: u16,
    product: ProductIdentity,
    provenance: Provenance,
    weight_scale: WeightScale,
    aggregation_policy: AggregationPolicy,
    groups: Vec<BenchmarkGroup>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WeightScale {
    unit: String,
    decimal_places: u8,
    total: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AggregationPolicy {
    terminal_identity_weight_field: String,
    terminal_identity_weight_value: serde_json::Value,
    observation_aggregation: String,
    monthly_source_weights: String,
    monthly_active_source_set: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BenchmarkGroup {
    name: String,
    row_count: u16,
    terminal_mpn_count: u16,
    weight_pct: String,
    rows: Vec<BenchmarkRow>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BenchmarkRow {
    name: String,
    terminal_mpn_count: u16,
    weight_pct: String,
    source_mode: String,
}

#[derive(Debug)]
struct ValidatedManifest {
    sku_ids: Vec<[u8; 32]>,
    required_sku_root: [u8; 32],
}

fn parse_manifest() -> GovernedProductManifest {
    serde_json::from_str(MANIFEST_SOURCE)
        .expect("governed NANDX manifest must use its exact schema")
}

fn parse_basket() -> GovernedBenchmarkBasket {
    serde_json::from_str(BASKET_SOURCE).expect("governed NANDX basket must use its exact schema")
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
        return Err("digest must be exactly 32 lowercase hexadecimal bytes".to_string());
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

fn push_u32(target: &mut Vec<u8>, value: usize) -> Result<(), String> {
    target.extend_from_slice(
        &u32::try_from(value)
            .map_err(|_| "layout length exceeds u32".to_string())?
            .to_le_bytes(),
    );
    Ok(())
}

fn push_string(target: &mut Vec<u8>, value: &str) -> Result<(), String> {
    push_u32(target, value.len())?;
    target.extend_from_slice(value.as_bytes());
    Ok(())
}

fn derive_manifest_layout_sha256(groups: &[ProductGroup]) -> Result<[u8; 32], String> {
    let mut encoded = b"amoeba-nandx-product-sku-layout-v1".to_vec();
    push_u32(&mut encoded, groups.len())?;
    for group in groups {
        push_string(&mut encoded, &group.name)?;
        push_u32(&mut encoded, usize::from(group.sku_count))?;
        push_u32(&mut encoded, group.subgroups.len())?;
        for subgroup in &group.subgroups {
            push_string(&mut encoded, &subgroup.name)?;
            push_u32(&mut encoded, usize::from(subgroup.sku_count))?;
            push_u32(&mut encoded, subgroup.sku_labels.len())?;
            for label in &subgroup.sku_labels {
                push_string(&mut encoded, label)?;
            }
        }
    }
    Ok(hash(&encoded).to_bytes())
}

fn derive_basket_layout_sha256(groups: &[BenchmarkGroup]) -> Result<[u8; 32], String> {
    let mut encoded = b"amoeba-nandx-benchmark-basket-layout-v1".to_vec();
    push_u32(&mut encoded, groups.len())?;
    for group in groups {
        push_string(&mut encoded, &group.name)?;
        push_u32(&mut encoded, usize::from(group.row_count))?;
        push_u32(&mut encoded, usize::from(group.terminal_mpn_count))?;
        push_string(&mut encoded, &group.weight_pct)?;
        push_u32(&mut encoded, group.rows.len())?;
        for row in &group.rows {
            push_string(&mut encoded, &row.name)?;
            push_u32(&mut encoded, usize::from(row.terminal_mpn_count))?;
            push_string(&mut encoded, &row.weight_pct)?;
            push_string(&mut encoded, &row.source_mode)?;
        }
    }
    Ok(hash(&encoded).to_bytes())
}

fn parse_percent_hundredths(value: &str) -> Result<u32, String> {
    let (whole, fractional) = value
        .split_once('.')
        .ok_or_else(|| "percentage must have exactly two decimals".to_string())?;
    if whole.is_empty()
        || (whole.len() > 1 && whole.starts_with('0'))
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fractional.len() != 2
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("percentage must use canonical two-decimal percent scale".to_string());
    }
    whole
        .parse::<u32>()
        .map_err(|_| "percentage whole part is invalid".to_string())?
        .checked_mul(100)
        .and_then(|amount| fractional.parse::<u32>().ok()?.checked_add(amount))
        .ok_or_else(|| "percentage overflows".to_string())
}

fn validate_manifest(manifest: &GovernedProductManifest) -> Result<ValidatedManifest, String> {
    if manifest.schema != "amoeba.oracle-product-sku-manifest"
        || manifest.schema_version != 1
        || manifest.manifest_version != 1
    {
        return Err("manifest schema/version mismatch".to_string());
    }
    if manifest.product
        != (ProductIdentity {
            market_id: "nandx".to_string(),
            symbol: "NANDX".to_string(),
            underlying_id: "nand-standardized-baskets".to_string(),
        })
    {
        return Err("NANDX product identity mismatch".to_string());
    }
    if manifest.provenance
        != (Provenance {
            source_description: "NANDX Manufacturer Item SKU Registry v0.1 (1 August 2026)"
                .to_string(),
            source_sha256: PINNED_SOURCE_SHA256.to_string(),
        })
    {
        return Err("NANDX manifest provenance mismatch".to_string());
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
        || manifest.tag190_plan.chunk_sku_counts != [16, 16, 16]
    {
        return Err("tag-190 packet plan mismatch".to_string());
    }
    if manifest.required_sku_count != NANDX_DOCUMENT_SKU_COUNT {
        return Err("NANDX canonical MPN count mismatch".to_string());
    }
    if manifest.required_sku_root_hex != PINNED_REQUIRED_SKU_ROOT_HEX {
        return Err("NANDX pinned SKU root mismatch".to_string());
    }
    if manifest.groups.len() != 5
        || manifest
            .groups
            .iter()
            .map(|group| group.subgroups.len())
            .sum::<usize>()
            != 22
    {
        return Err("NANDX must preserve five sleeves and 22 benchmark rows".to_string());
    }
    if derive_manifest_layout_sha256(&manifest.groups)?
        != parse_lower_hex_32(PINNED_MANIFEST_LAYOUT_SHA256)?
    {
        return Err("NANDX manifest group, row, membership, or ordering drift".to_string());
    }

    let mut sku_ids = Vec::new();
    let mut seen = HashSet::new();
    for group in &manifest.groups {
        let mut group_count = 0u16;
        for subgroup in &group.subgroups {
            if subgroup.sku_labels.len() != usize::from(subgroup.sku_count) {
                return Err(format!("subgroup count mismatch for {}", subgroup.name));
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
                    return Err(format!("duplicate terminal MPN label: {label}"));
                }
                sku_ids.push(encoded);
            }
        }
        if group_count != group.sku_count {
            return Err(format!("group count mismatch for {}", group.name));
        }
    }
    if sku_ids.len() != usize::from(manifest.required_sku_count) {
        return Err("flattened terminal-MPN count mismatch".to_string());
    }
    sku_ids.sort_unstable();
    if sku_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("encoded terminal-MPN set is not strictly ascending".to_string());
    }
    let actual_chunk_counts = sku_ids
        .chunks(MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS)
        .map(|chunk| chunk.len())
        .collect::<Vec<_>>();
    if actual_chunk_counts != manifest.tag190_plan.chunk_sku_counts {
        return Err("flattened terminal-MPN chunk plan mismatch".to_string());
    }
    let required_sku_root = derive_on_chain_compatible_root(&sku_ids)?;
    if required_sku_root != parse_lower_hex_32(&manifest.required_sku_root_hex)? {
        return Err("derived terminal-MPN root mismatch".to_string());
    }
    Ok(ValidatedManifest {
        sku_ids,
        required_sku_root,
    })
}

fn validate_basket(
    basket: &GovernedBenchmarkBasket,
    manifest: &GovernedProductManifest,
) -> Result<(), String> {
    if basket.schema != "amoeba.nandx-benchmark-basket"
        || basket.schema_version != 1
        || basket.basket_version != 1
        || basket.product != manifest.product
        || basket.provenance != manifest.provenance
    {
        return Err("NANDX benchmark basket identity or provenance mismatch".to_string());
    }
    if basket.weight_scale.unit != "percent"
        || basket.weight_scale.decimal_places != 2
        || basket.weight_scale.total != "100.00"
    {
        return Err("NANDX benchmark basket weight scale mismatch".to_string());
    }
    if basket.aggregation_policy.terminal_identity_weight_field != "weightPct"
        || !basket
            .aggregation_policy
            .terminal_identity_weight_value
            .is_null()
        || basket.aggregation_policy.observation_aggregation
            != "within-parent-row-before-basket-weight"
        || basket.aggregation_policy.monthly_source_weights != "separate-governed-oracle-recipe"
        || basket.aggregation_policy.monthly_active_source_set
            != "validated-subset-of-governed-terminal-identities"
    {
        return Err("NANDX benchmark basket aggregation policy mismatch".to_string());
    }
    if basket.groups.len() != manifest.groups.len()
        || derive_basket_layout_sha256(&basket.groups)?
            != parse_lower_hex_32(PINNED_BASKET_LAYOUT_SHA256)?
    {
        return Err("NANDX benchmark basket layout or weights drift".to_string());
    }

    let mut total_weight = 0u32;
    let mut total_rows = 0usize;
    let mut item_rows = 0usize;
    let mut assessment_rows = 0usize;
    let mut terminal_mpn_count = 0u16;
    for (group, manifest_group) in basket.groups.iter().zip(&manifest.groups) {
        if group.name != manifest_group.name
            || group.terminal_mpn_count != manifest_group.sku_count
            || group.rows.len() != usize::from(group.row_count)
            || group.rows.len() != manifest_group.subgroups.len()
        {
            return Err(format!(
                "NANDX basket group mapping mismatch for {}",
                group.name
            ));
        }
        let mut group_weight = 0u32;
        let mut group_mpn_count = 0u16;
        for (row, manifest_row) in group.rows.iter().zip(&manifest_group.subgroups) {
            if row.name != manifest_row.name
                || row.terminal_mpn_count != manifest_row.sku_count
                || (row.source_mode == "assessment-only" && !manifest_row.sku_labels.is_empty())
            {
                return Err(format!(
                    "NANDX basket row mapping mismatch for {}",
                    row.name
                ));
            }
            group_weight = group_weight
                .checked_add(parse_percent_hundredths(&row.weight_pct)?)
                .ok_or_else(|| "group weight overflow".to_string())?;
            group_mpn_count = group_mpn_count
                .checked_add(row.terminal_mpn_count)
                .ok_or_else(|| "group MPN count overflow".to_string())?;
            match row.source_mode.as_str() {
                "candidate-mpn" => item_rows += 1,
                "assessment-only" => assessment_rows += 1,
                _ => return Err("unknown NANDX benchmark source mode".to_string()),
            }
        }
        if group_weight != parse_percent_hundredths(&group.weight_pct)?
            || group_mpn_count != group.terminal_mpn_count
        {
            return Err(format!(
                "NANDX basket aggregate mismatch for {}",
                group.name
            ));
        }
        total_weight = total_weight
            .checked_add(group_weight)
            .ok_or_else(|| "total weight overflow".to_string())?;
        total_rows += group.rows.len();
        terminal_mpn_count = terminal_mpn_count
            .checked_add(group.terminal_mpn_count)
            .ok_or_else(|| "terminal MPN count overflow".to_string())?;
    }
    if total_weight != 10_000
        || total_rows != 22
        || item_rows != 18
        || assessment_rows != 4
        || terminal_mpn_count != NANDX_DOCUMENT_SKU_COUNT
    {
        return Err("NANDX basket must preserve 100.00%, 22/18/4 rows, and 48 MPNs".to_string());
    }
    Ok(())
}

/// The governed NANDX document registers 48 terminal MPNs. The program has no product table: this
/// count and the pinned root are data, registered once through tag 190.
const NANDX_DOCUMENT_SKU_COUNT: u16 = 48;

#[test]
fn governed_nandx_manifest_matches_rust_counts_codec_layout_merkle_and_weight_contracts() {
    assert_eq!(NANDX_DOCUMENT_SKU_COUNT, 48);
    let manifest = parse_manifest();
    let validated = validate_manifest(&manifest).expect("governed NANDX manifest is valid");
    validate_basket(&parse_basket(), &manifest).expect("governed NANDX basket is valid");
    assert_eq!(validated.sku_ids.len(), 48);
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
        [16, 16, 16]
    );
    assert_eq!(
        (0..validated.sku_ids.len())
            .step_by(MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS)
            .collect::<Vec<_>>(),
        [0, 16, 32]
    );
    assert_ne!(
        derive_on_chain_compatible_root(&validated.sku_ids[..47]).unwrap(),
        validated.required_sku_root
    );
}

#[test]
fn governed_nandx_manifest_rejects_set_layout_encoding_and_weight_mutations() {
    let manifest = parse_manifest();

    let mut duplicate = manifest.clone();
    duplicate.groups[0].subgroups[0].sku_labels[1] =
        duplicate.groups[0].subgroups[0].sku_labels[0].clone();
    assert!(validate_manifest(&duplicate).is_err());

    let mut missing = manifest.clone();
    missing.groups[0].subgroups[0].sku_labels.pop();
    assert!(validate_manifest(&missing).is_err());

    let mut extra = manifest.clone();
    extra.groups[0].subgroups[0]
        .sku_labels
        .push("EXTRA-MPN".to_string());
    assert!(validate_manifest(&extra).is_err());

    let mut reordered = manifest.clone();
    reordered.groups[0].subgroups[0].sku_labels.swap(0, 1);
    assert!(validate_manifest(&reordered).is_err());

    let mut empty = manifest.clone();
    empty.groups[0].subgroups[0].sku_labels[0] = String::new();
    assert!(validate_manifest(&empty).is_err());

    let mut non_ascii = manifest.clone();
    non_ascii.groups[0].subgroups[0].sku_labels[0] = "mémoire".to_string();
    assert!(validate_manifest(&non_ascii).is_err());

    let mut overlength = manifest.clone();
    overlength.groups[0].subgroups[0].sku_labels[0] = "X".repeat(33);
    assert!(validate_manifest(&overlength).is_err());

    let mut wrong_count = manifest.clone();
    wrong_count.required_sku_count = 24;
    assert!(validate_manifest(&wrong_count).is_err());

    assert!(encode_canonical_ascii_label("").is_err());
    assert!(encode_canonical_ascii_label("line\nbreak").is_err());
    let mut malformed_padding = encode_canonical_ascii_label("ABC").unwrap();
    malformed_padding[4] = b'X';
    assert!(decode_canonical_ascii_label(&malformed_padding).is_err());

    let mut basket = parse_basket();
    basket.groups[0].rows[0].weight_pct = "3.01".to_string();
    assert!(validate_basket(&basket, &manifest).is_err());

    let mut basket = parse_basket();
    basket.aggregation_policy.terminal_identity_weight_value = serde_json::json!("0.00");
    assert!(validate_basket(&basket, &manifest).is_err());

    let mut basket = parse_basket();
    basket.groups[4].rows[0].terminal_mpn_count = 1;
    assert!(validate_basket(&basket, &manifest).is_err());
}
