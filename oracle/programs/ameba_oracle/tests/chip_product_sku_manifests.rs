//! The chip product SKU manifests (`governance/{ramx,nandx}_product_sku_manifest.v2.json`,
//! the commodity buckets of `governance/commodity_submission_spec.v1.json`): their pinned roots
//! are what tag 190 computes over their labels.

use ameba_oracle::constants::{
    MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS, MAX_ORACLE_REQUIRED_SKUS, ORACLE_SKU_EMPTY_HASH_DOMAIN,
    ORACLE_SKU_LEAF_HASH_DOMAIN, ORACLE_SKU_NODE_HASH_DOMAIN,
};
use serde_json::Value;
use solana_program::hash::hashv;

const MANIFESTS: [(&str, &str, &[&str]); 2] = [
    (
        "RAMX",
        include_str!("../../../governance/ramx_product_sku_manifest.v2.json"),
        &["DDR4_16GBIT", "DDR4_8GBIT", "DDR5_16GBIT", "DDR5_24GBIT"],
    ),
    (
        "NANDX",
        include_str!("../../../governance/nandx_product_sku_manifest.v2.json"),
        &["qlc_1tb", "tlc_1tb", "tlc_256gb", "tlc_512gb"],
    ),
];

fn encode(label: &str) -> [u8; 32] {
    assert!(!label.is_empty() && label.len() <= 32);
    assert!(label.bytes().all(|byte| (0x20..=0x7e).contains(&byte)));
    let mut id = [0u8; 32];
    id[..label.len()].copy_from_slice(label.as_bytes());
    id
}

fn root(ids: &[[u8; 32]]) -> [u8; 32] {
    let width = ids.len().next_power_of_two();
    let mut nodes: Vec<[u8; 32]> = (0..width)
        .map(|index| {
            let index = (index as u16).to_le_bytes();
            match ids.get(usize::from(u16::from_le_bytes(index))) {
                Some(id) => hashv(&[ORACLE_SKU_LEAF_HASH_DOMAIN, &index, id]).to_bytes(),
                None => hashv(&[ORACLE_SKU_EMPTY_HASH_DOMAIN, &index]).to_bytes(),
            }
        })
        .collect();
    while nodes.len() > 1 {
        nodes = nodes
            .chunks_exact(2)
            .map(|pair| hashv(&[ORACLE_SKU_NODE_HASH_DOMAIN, &pair[0], &pair[1]]).to_bytes())
            .collect();
    }
    nodes[0]
}

#[test]
fn chip_manifests_pin_the_roots_tag_190_computes() {
    for (symbol, source, sorted) in MANIFESTS {
        let manifest: Value = serde_json::from_str(source).expect("manifest JSON");
        assert_eq!(manifest["product"]["symbol"], symbol);
        assert_eq!(manifest["manifestVersion"], 2);
        let mut labels: Vec<&str> = manifest["groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|group| group["subgroups"].as_array().unwrap())
            .flat_map(|subgroup| subgroup["skuLabels"].as_array().unwrap())
            .map(|label| label.as_str().unwrap())
            .collect();
        labels.sort_by_key(|label| encode(label));
        assert_eq!(labels, sorted);
        let ids: Vec<[u8; 32]> = labels.iter().map(|label| encode(label)).collect();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(ids.len() <= usize::from(MAX_ORACLE_REQUIRED_SKUS));
        assert!(
            ids.len() <= MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS,
            "one tag-190 transaction"
        );
        assert_eq!(manifest["requiredSkuCount"], ids.len());
        let hex: String = root(&ids)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(manifest["requiredSkuRootHex"], hex.as_str());
    }
}
