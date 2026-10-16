//! Tag 190 on chain: the governed RAMX and NANDX terminal-SKU sets register through
//! packet-safe chunks and reproduce their pinned roots.

mod common;

use ameba_oracle::{
    error::OracleError,
    state::{
        derive_oracle_product_sku_draft_pda, derive_oracle_product_sku_manifest_pda,
        OracleProductSkuDraft, OracleProductSkuManifest,
    },
};
use common::*;
use solana_sdk::signature::{Keypair, Signer};

const RAMX_SOURCE: &str = include_str!("../../../governance/ramx_product_sku_manifest.v1.json");
const NANDX_SOURCE: &str = include_str!("../../../governance/nandx_product_sku_manifest.v1.json");
const RAMX_ROOT_HEX: &str = "52a574e7fee12f9921b3b220b709be16c13a6f290abbdc2cc203ed1191ecf578";
const NANDX_ROOT_HEX: &str = "41bb5dc79bacabb3e0876b55e647b01084c7fe8b39846fd707ee0fd973d28c3d";

/// Every governed label, right-zero-padded to 32 bytes and sorted as the manifest requires.
fn governed_sku_ids(source: &str) -> Vec<[u8; 32]> {
    let manifest: serde_json::Value = serde_json::from_str(source).unwrap();
    let mut ids = manifest["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["subgroups"].as_array().unwrap().iter())
        .flat_map(|subgroup| subgroup["skuLabels"].as_array().unwrap().iter())
        .map(|label| {
            let label = label.as_str().unwrap().as_bytes();
            let mut id = [0u8; 32];
            id[..label.len()].copy_from_slice(label);
            id
        })
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn governed_harness() -> Harness {
    let mut h = start().await;
    h.initialize().await.unwrap();
    h.split_oracle_authority().await;
    h
}

async fn register_and_check(
    h: &mut Harness,
    product_name: &str,
    source: &str,
    root_hex: &str,
    chunks: &[usize],
) {
    let product_id = product(product_name);
    let ids = governed_sku_ids(source);
    assert_eq!(
        ids.chunks(MAX_CHUNK_IDS)
            .map(<[_]>::len)
            .collect::<Vec<_>>(),
        chunks
    );
    let root = sku_merkle_root(&ids);
    assert_eq!(hex(&root), root_hex);
    let admin_before = h.account(h.admin.pubkey()).await.unwrap().lamports;
    let draft_key = derive_oracle_product_sku_draft_pda(&program(), &product_id, 7).0;
    let manifest_key = derive_oracle_product_sku_manifest_pda(&program(), &product_id).0;
    for (index, chunk) in ids.chunks(MAX_CHUNK_IDS).enumerate() {
        h.send_product_sku_chunk(
            product_id,
            7,
            (index * MAX_CHUNK_IDS) as u16,
            ids.len() as u16,
            root,
            chunk,
        )
        .await
        .unwrap();
        let last = (index + 1) * MAX_CHUNK_IDS >= ids.len();
        if !last {
            let draft: OracleProductSkuDraft = h.read_state(draft_key).await;
            assert_eq!(
                usize::from(draft.appended_sku_count),
                (index + 1) * MAX_CHUNK_IDS
            );
            assert!(draft.has_canonical_layout());
            assert!(h.account(manifest_key).await.is_none());
        }
    }
    // The final chunk creates the immutable manifest and closes the draft to the admin.
    assert!(h.account(draft_key).await.is_none());
    let manifest: OracleProductSkuManifest = h.read_state(manifest_key).await;
    assert!(manifest.is_initialized);
    assert!(manifest.has_canonical_layout());
    assert_eq!(manifest.underlying_id, product_id);
    assert_eq!(hex(&manifest.required_sku_root), root_hex);
    assert_eq!(usize::from(manifest.required_sku_count), ids.len());
    let admin_after = h.account(h.admin.pubkey()).await.unwrap().lamports;
    let rent = h.ctx.banks_client.get_rent().await.unwrap();
    assert_eq!(
        admin_before - admin_after,
        rent.minimum_balance(OracleProductSkuManifest::LEN),
        "the admin funds only the manifest once the draft rent is returned"
    );
}

#[tokio::test]
async fn governed_ramx_manifest_registers_its_pinned_root_on_chain() {
    let mut h = governed_harness().await;
    register_and_check(&mut h, "RAMX", RAMX_SOURCE, RAMX_ROOT_HEX, &[16, 16, 16, 4]).await;
}

#[tokio::test]
async fn governed_nandx_manifest_registers_its_pinned_root_on_chain() {
    let mut h = governed_harness().await;
    register_and_check(&mut h, "NANDX", NANDX_SOURCE, NANDX_ROOT_HEX, &[16, 16, 16]).await;
}

#[tokio::test]
async fn registration_is_create_only_dual_signed_and_root_bound() {
    let mut h = governed_harness().await;
    let ramx = product("RAMX");
    let ids = synthetic_sku_ids(20);
    let root = sku_merkle_root(&ids);

    // A declared root the identifiers do not reproduce never creates a manifest.
    h.send_product_sku_chunk(ramx, 1, 0, 20, [9; 32], &ids[..16])
        .await
        .unwrap();
    assert_oracle_error(
        h.send_product_sku_chunk(ramx, 1, 16, 20, [9; 32], &ids[16..])
            .await,
        OracleError::InvalidOracleSkuCoverageManifest,
    );
    // A chunk that does not continue the stored draft is rejected.
    assert_oracle_error(
        h.send_product_sku_chunk(ramx, 1, 0, 20, [9; 32], &ids[..16])
            .await,
        OracleError::InvalidOracleProductSkuDraft,
    );
    // A different declared count or root cannot reuse that draft.
    assert_oracle_error(
        h.send_product_sku_chunk(ramx, 1, 16, 21, [9; 32], &ids[16..])
            .await,
        OracleError::InvalidOracleProductSkuDraft,
    );

    // The admin and the oracle authority must be two distinct signers.
    let admin = h.admin.insecure_clone();
    let previous_oracle = std::mem::replace(&mut h.oracle_authority, admin);
    assert_oracle_error(
        h.send_product_sku_chunk(ramx, 2, 0, 20, root, &ids[..16])
            .await,
        OracleError::Unauthorized,
    );
    h.oracle_authority = Keypair::new();
    assert_oracle_error(
        h.send_product_sku_chunk(ramx, 2, 0, 20, root, &ids[..16])
            .await,
        OracleError::Unauthorized,
    );
    h.oracle_authority = previous_oracle;

    // A fresh nonce registers the product exactly once.
    h.send_product_sku_chunk(ramx, 3, 0, 20, root, &ids[..16])
        .await
        .unwrap();
    h.send_product_sku_chunk(ramx, 3, 16, 20, root, &ids[16..])
        .await
        .unwrap();
    let manifest: OracleProductSkuManifest = h
        .read_state(derive_oracle_product_sku_manifest_pda(&program(), &ramx).0)
        .await;
    assert_eq!(manifest.required_sku_root, root);
    assert_eq!(manifest.required_sku_count, 20);
    assert_oracle_error(
        h.send_product_sku_chunk(ramx, 4, 0, 20, root, &ids[..16])
            .await,
        OracleError::AlreadyInitialized,
    );
    let _ = h.oracle_authority.pubkey();
}
