//! Tag 190 product-SKU registration through the fixed Merkle frontier: exact final chunk, draft
//! refund, declared count and root binding, and rejection of drafts in any other layout.
mod common;

use ameba_oracle::{
    constants::MAX_ORACLE_REQUIRED_SKUS,
    state::{
        derive_oracle_product_sku_draft_pda, derive_oracle_product_sku_manifest_pda,
        OracleProductSkuDraft, OracleProductSkuManifest,
    },
};
use borsh::BorshDeserialize;
use common::*;
use solana_program_test::BanksClientError;
use solana_sdk::{account::AccountSharedData, pubkey::Pubkey, signature::Signer};

const DRAFT_NONCE: u64 = 0x51_4b_55;

/// Strictly ascending, nonzero synthetic identifiers.
fn sku_ids(count: u16, salt: u8) -> Vec<[u8; 32]> {
    (1..=count)
        .map(|index| {
            let mut sku_id = [0u8; 32];
            sku_id[0] = salt;
            sku_id[30..].copy_from_slice(&index.to_be_bytes());
            sku_id
        })
        .collect()
}

struct Fixture {
    h: Harness,
    product: [u8; 32],
}

impl Fixture {
    async fn start(label: &str) -> Self {
        let mut h = start().await;
        h.initialize().await.unwrap();
        h.split_oracle_authority().await;
        Self {
            h,
            product: product(label),
        }
    }

    async fn chunk(
        &mut self,
        nonce: u64,
        expected_start_index: u16,
        sku_id_chunk: &[[u8; 32]],
        sku_count: u16,
        expected_root: [u8; 32],
    ) -> Result<(), BanksClientError> {
        self.h.advance_slot().await;
        let product = self.product;
        self.h
            .send_product_sku_chunk(
                product,
                nonce,
                expected_start_index,
                sku_count,
                expected_root,
                sku_id_chunk,
            )
            .await
    }

    /// Registers `ids` with the declared count and root in packet-safe chunks of 16.
    async fn register(
        &mut self,
        nonce: u64,
        ids: &[[u8; 32]],
        sku_count: u16,
        expected_root: [u8; 32],
    ) -> Result<(), BanksClientError> {
        for (index, chunk) in ids.chunks(16).enumerate() {
            self.chunk(nonce, (index * 16) as u16, chunk, sku_count, expected_root)
                .await?;
        }
        Ok(())
    }

    fn draft_key(&self, nonce: u64) -> Pubkey {
        derive_oracle_product_sku_draft_pda(&program(), &self.product, nonce).0
    }

    async fn draft(&mut self, nonce: u64) -> Option<OracleProductSkuDraft> {
        let key = self.draft_key(nonce);
        self.h.account(key).await.map(|account| {
            assert_eq!(account.data.len(), OracleProductSkuDraft::LEN);
            OracleProductSkuDraft::deserialize(&mut account.data.as_slice()).unwrap()
        })
    }

    async fn manifest(&mut self) -> Option<OracleProductSkuManifest> {
        let key = derive_oracle_product_sku_manifest_pda(&program(), &self.product).0;
        self.h.account(key).await.map(|account| {
            assert_eq!(account.data.len(), OracleProductSkuManifest::LEN);
            OracleProductSkuManifest::deserialize(&mut account.data.as_slice()).unwrap()
        })
    }
}

#[tokio::test]
async fn final_chunk_preserves_root_and_refunds_the_fixed_frontier_draft() {
    let mut fixture = Fixture::start("RAMX").await;
    let ids = sku_ids(52, 0);
    let root = sku_merkle_root(&ids);
    let admin = fixture.h.admin.pubkey();
    let initial_admin_lamports = fixture.h.account(admin).await.unwrap().lamports;

    for (start, end) in [(0usize, 16usize), (16, 32), (32, 48)] {
        fixture
            .chunk(DRAFT_NONCE, start as u16, &ids[start..end], 52, root)
            .await
            .unwrap();
    }
    let draft = fixture.draft(DRAFT_NONCE).await.expect("partial draft");
    assert_eq!(draft.appended_sku_count, 48);
    assert_eq!(draft.frontier_mask, 48);
    assert_eq!(draft.expected_sku_count, 52);
    assert_eq!(draft.expected_sku_root, root);
    assert!(fixture.manifest().await.is_none());

    fixture
        .chunk(DRAFT_NONCE, 48, &ids[48..], 52, root)
        .await
        .unwrap();

    assert!(fixture.draft(DRAFT_NONCE).await.is_none());
    let manifest = fixture.manifest().await.expect("immutable SKU manifest");
    assert_eq!(manifest.required_sku_count, 52);
    assert_eq!(manifest.required_sku_root, root);

    let final_admin_lamports = fixture.h.account(admin).await.unwrap().lamports;
    let rent = fixture.h.ctx.banks_client.get_rent().await.unwrap();
    assert_eq!(
        initial_admin_lamports - final_admin_lamports,
        rent.minimum_balance(OracleProductSkuManifest::LEN),
        "the temporary draft rent must be fully returned on finalization"
    );

    // Create-once: the finalized manifest can never be rewritten, under any nonce.
    assert!(fixture
        .chunk(DRAFT_NONCE + 1, 0, &ids[..16], 52, root)
        .await
        .is_err());
    assert_eq!(fixture.manifest().await.unwrap(), manifest);
}

/// Any product is registered as data: an identifier no build knows, an arbitrary count, and the
/// declared root.
#[tokio::test]
async fn registers_a_new_product_with_its_declared_count_and_root() {
    for count in [1u16, 37, 129, MAX_ORACLE_REQUIRED_SKUS] {
        let mut fixture = Fixture::start("HBMX").await;
        let ids = sku_ids(count, 7);
        let root = sku_merkle_root(&ids);
        fixture
            .register(DRAFT_NONCE, &ids, count, root)
            .await
            .unwrap();
        assert!(fixture.draft(DRAFT_NONCE).await.is_none());
        let manifest = fixture.manifest().await.expect("registered manifest");
        assert_eq!(manifest.underlying_id, fixture.product);
        assert_eq!(manifest.required_sku_count, count);
        assert_eq!(manifest.required_sku_root, root);
    }
}

#[tokio::test]
async fn wrong_declared_root_is_rejected_at_finalize() {
    let mut fixture = Fixture::start("HBMX").await;
    let ids = sku_ids(20, 7);
    let wrong_root = sku_merkle_root(&sku_ids(20, 8));
    fixture
        .chunk(DRAFT_NONCE, 0, &ids[..16], 20, wrong_root)
        .await
        .unwrap();
    assert!(fixture
        .chunk(DRAFT_NONCE, 16, &ids[16..], 20, wrong_root)
        .await
        .is_err());
    let draft = fixture.draft(DRAFT_NONCE).await.expect("draft survives");
    assert_eq!(draft.appended_sku_count, 16);
    assert!(fixture.manifest().await.is_none());

    // A one-chunk registration with the wrong root creates nothing at all.
    let mut single = Fixture::start("HBMX").await;
    assert!(single
        .chunk(DRAFT_NONCE, 0, &ids[..4], 4, wrong_root)
        .await
        .is_err());
    assert!(single.draft(DRAFT_NONCE).await.is_none());
    assert!(single.manifest().await.is_none());

    // The abandoned draft cannot be repaired; a fresh nonce with the right root registers.
    let root = sku_merkle_root(&ids);
    assert!(fixture
        .chunk(DRAFT_NONCE, 16, &ids[16..], 20, root)
        .await
        .is_err());
    fixture
        .register(DRAFT_NONCE + 1, &ids, 20, root)
        .await
        .unwrap();
    assert_eq!(fixture.manifest().await.unwrap().required_sku_root, root);
}

#[tokio::test]
async fn chunks_beyond_the_declared_count_are_rejected() {
    let mut fixture = Fixture::start("HBMX").await;
    let ids = sku_ids(16, 7);
    let root = sku_merkle_root(&ids[..10]);
    // A first chunk longer than the declared count.
    assert!(fixture.chunk(DRAFT_NONCE, 0, &ids, 10, root).await.is_err());
    assert!(fixture.draft(DRAFT_NONCE).await.is_none());
    // A later chunk that would pass the declared count.
    fixture
        .chunk(DRAFT_NONCE, 0, &ids[..8], 10, root)
        .await
        .unwrap();
    assert!(fixture
        .chunk(DRAFT_NONCE, 8, &ids[8..11], 10, root)
        .await
        .is_err());
    assert_eq!(
        fixture.draft(DRAFT_NONCE).await.unwrap().appended_sku_count,
        8
    );
    fixture
        .chunk(DRAFT_NONCE, 8, &ids[8..10], 10, root)
        .await
        .unwrap();
    assert_eq!(fixture.manifest().await.unwrap().required_sku_count, 10);
}

#[tokio::test]
async fn later_chunks_must_repeat_the_declared_count_and_root() {
    let mut fixture = Fixture::start("HBMX").await;
    let ids = sku_ids(20, 7);
    let root = sku_merkle_root(&ids);
    fixture
        .chunk(DRAFT_NONCE, 0, &ids[..16], 20, root)
        .await
        .unwrap();
    let before = fixture.draft(DRAFT_NONCE).await.unwrap();
    let mut other_root = root;
    other_root[0] ^= 1;
    for (count, declared_root) in [(19, root), (21, root), (20, other_root)] {
        assert!(fixture
            .chunk(DRAFT_NONCE, 16, &ids[16..], count, declared_root)
            .await
            .is_err());
        assert_eq!(fixture.draft(DRAFT_NONCE).await.unwrap(), before);
    }
    fixture
        .chunk(DRAFT_NONCE, 16, &ids[16..], 20, root)
        .await
        .unwrap();
    assert_eq!(fixture.manifest().await.unwrap().required_sku_root, root);
}

#[tokio::test]
async fn declarations_outside_the_supported_tree_are_rejected() {
    let mut fixture = Fixture::start("HBMX").await;
    let ids = sku_ids(4, 7);
    let root = sku_merkle_root(&ids);
    for (count, declared_root) in [
        (0, root),
        (MAX_ORACLE_REQUIRED_SKUS + 1, root),
        (u16::MAX, root),
        (4, [0; 32]),
    ] {
        assert!(fixture
            .chunk(DRAFT_NONCE, 0, &ids, count, declared_root)
            .await
            .is_err());
        assert!(fixture.draft(DRAFT_NONCE).await.is_none());
        assert!(fixture.manifest().await.is_none());
    }
}

/// A draft in the 380-byte version-2 layout is never read under the current layout: every chunk
/// against it fails and leaves it untouched.
#[tokio::test]
async fn version_two_draft_layout_fails_cleanly() {
    let mut fixture = Fixture::start("HBMX").await;
    let (stale_key, stale_bump) =
        derive_oracle_product_sku_draft_pda(&program(), &fixture.product, DRAFT_NONCE);
    // Version-2 layout: header, underlying, nonce, count 20, appended 0, mask 0, last id,
    // nine frontier nodes and the slot, 380 bytes in all.
    let mut stale = vec![1u8, stale_bump, b'O', b'P', b'D', 2];
    stale.extend_from_slice(&fixture.product);
    stale.extend_from_slice(&DRAFT_NONCE.to_le_bytes());
    stale.extend_from_slice(&20u16.to_le_bytes());
    stale.resize(380, 0);
    let mut account = AccountSharedData::new(10_000_000, stale.len(), &program());
    account.set_data_from_slice(&stale);
    fixture
        .h
        .set_account_preserving_capitalization(stale_key, account)
        .await;

    let ids = sku_ids(20, 7);
    let root = sku_merkle_root(&ids);
    for start in [0u16, 16] {
        let chunk = ids[usize::from(start)..usize::from(start) + 4].to_vec();
        assert!(fixture
            .chunk(DRAFT_NONCE, start, &chunk, 20, root)
            .await
            .is_err());
        assert_eq!(fixture.h.account(stale_key).await.unwrap().data, stale);
    }
    assert!(fixture.manifest().await.is_none());
    fixture
        .register(DRAFT_NONCE + 1, &ids, 20, root)
        .await
        .unwrap();
    assert_eq!(fixture.manifest().await.unwrap().required_sku_count, 20);
}
