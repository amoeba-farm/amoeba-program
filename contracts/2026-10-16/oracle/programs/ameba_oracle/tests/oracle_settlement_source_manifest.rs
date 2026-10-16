//! Tag 113: the ordered settlement-source digest over a finalized month's bucket medians.
mod common;

use ameba_oracle::{
    instruction::{self, AccumulateOracleSettlementSourceBucketParams},
    processor::{
        advance_oracle_settlement_source_digest, canonical_recipe_digest,
        initial_oracle_settlement_source_digest,
    },
    state::{
        derive_oracle_bucket_median_pda, derive_oracle_settlement_source_manifest_pda,
        OracleBucketMedianState, OracleBucketMedianStatus, OracleMonthState, OraclePhase,
        OracleRecipeWeightPhase, OracleSettlementSourceManifest, OracleSettlementStatus,
    },
};
use borsh::BorshDeserialize;
use common::*;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signature::Signer};

const BUCKET_A: [u8; 32] = [0x44; 32];
const BUCKET_B: [u8; 32] = [0x55; 32];

struct ManifestFixture {
    h: Harness,
    period: Pubkey,
    month: Pubkey,
    source_manifest: Pubkey,
    buckets: [(Pubkey, OracleBucketMedianState); 2],
    canonical_source_digest: [u8; 32],
}

fn bucket_state(
    month: Pubkey,
    bucket_id: [u8; 32],
    group_index: u16,
    bucket_weight_start_bps: u16,
    bucket_weight_bps: u16,
    source_count: u16,
) -> (Pubkey, OracleBucketMedianState) {
    let (address, bump) = derive_oracle_bucket_median_pda(&program(), &month, &bucket_id);
    (
        address,
        OracleBucketMedianState {
            is_initialized: true,
            bump,
            account_discriminator: OracleBucketMedianState::ACCOUNT_DISCRIMINATOR,
            account_version: OracleBucketMedianState::ACCOUNT_VERSION,
            month,
            bucket_id,
            group_index,
            bucket_weight_start_bps,
            bucket_weight_bps,
            frozen_source_count: source_count,
            active_source_count: source_count,
            eligible_source_count: source_count,
            status: OracleBucketMedianStatus::SettlementReady,
            bucket_delta_bps: i64::from(group_index) + 7,
            last_recomputed_ts: 3,
            source_snapshot_hash: [bucket_id[0].wrapping_add(1); 32],
            ..OracleBucketMedianState::default()
        },
    )
}

impl ManifestFixture {
    /// A finalized month of a real period, installed directly: month finalization (75) is not
    /// part of this surface yet.
    async fn start() -> Self {
        let mut h = start().await;
        h.initialize().await.unwrap();
        let product = product("RAMX");
        let oracle = h.oracle_authority.insecure_clone();
        h.create_period(&oracle, product, PLANNED, EXPIRY, 6)
            .await
            .unwrap();
        let period = ameba_oracle_interface::find_period_address(&program(), &product, EXPIRY).0;
        let (month, month_bump) = Pubkey::find_program_address(
            &[
                ameba_oracle::constants::CURRENT_STATE_NAMESPACE_SEED,
                ameba_oracle::constants::ORACLE_MONTH_PDA_SEED,
                period.as_ref(),
                &EXPIRY.to_le_bytes(),
            ],
            &program(),
        );
        let (source_manifest, _) = derive_oracle_settlement_source_manifest_pda(&program(), &month);
        let frozen_manifest_hash = [0x55; 32];
        let recipe_hash = canonical_recipe_digest(&month, &frozen_manifest_hash);
        let active_manifest_hash = [0x66; 32];
        let month_state = OracleMonthState {
            is_initialized: true,
            bump: month_bump,
            account_discriminator: OracleMonthState::ACCOUNT_DISCRIMINATOR,
            account_version: OracleMonthState::ACCOUNT_VERSION,
            period,
            authority: Pubkey::new_unique(),
            scramble_start_ts: 1,
            listing_ts: 2,
            phase: OraclePhase::Game,
            source_count: 3,
            frozen_source_count: 3,
            opened_source_count: 2,
            recipe_hash,
            settlement_status: OracleSettlementStatus::Final,
            pending_resolution_count: 0,
            finalized_at_ts: 1,
            opening_resolved_source_count: 3,
            weight_scheme_version: 1,
            effective_weight_total_bps: 10_000,
            weight_manifest_hash: frozen_manifest_hash,
            active_weight_scheme_version: OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION,
            active_weight_group_count: 2,
            active_weight_manifest_hash: active_manifest_hash,
            ..OracleMonthState::default()
        };
        let buckets = [
            bucket_state(month, BUCKET_A, 0, 0, 4_000, 1),
            bucket_state(month, BUCKET_B, 1, 4_000, 6_000, 2),
        ];
        let mut canonical_source_digest = initial_oracle_settlement_source_digest(
            &month,
            &recipe_hash,
            &active_manifest_hash,
            3,
            2,
        );
        for (_, bucket) in &buckets {
            canonical_source_digest =
                advance_oracle_settlement_source_digest(&canonical_source_digest, bucket);
        }
        h.set_program_state(month, &month_state, OracleMonthState::LEN)
            .await;
        for (address, bucket) in &buckets {
            h.set_program_state(*address, bucket, OracleBucketMedianState::LEN)
                .await;
        }
        Self {
            h,
            period,
            month,
            source_manifest,
            buckets,
            canonical_source_digest,
        }
    }

    fn accumulate_ix(&self, bucket_indexes: &[usize], finalize_collection: bool) -> Instruction {
        let first_bucket = self.buckets[bucket_indexes[0]].1.bucket_id;
        let buckets: Vec<[u8; 32]> = bucket_indexes
            .iter()
            .map(|index| self.buckets[*index].1.bucket_id)
            .collect();
        instruction::accumulate_oracle_settlement_source_bucket(
            &program(),
            &self.h.ctx.payer.pubkey(),
            &self.period,
            &self.month,
            AccumulateOracleSettlementSourceBucketParams {
                bucket_id: first_bucket,
                finalize_collection,
            },
            &buckets,
        )
    }

    async fn manifest(&mut self) -> Option<OracleSettlementSourceManifest> {
        let key = self.source_manifest;
        self.h.account(key).await.map(|account| {
            OracleSettlementSourceManifest::deserialize(&mut &account.data[..]).unwrap()
        })
    }
}

#[tokio::test]
async fn canonical_bucket_walk_creates_and_finalizes_the_source_manifest() {
    let mut fixture = ManifestFixture::start().await;
    let ix = fixture.accumulate_ix(&[0, 1], true);
    fixture.h.send(ix, &[]).await.unwrap();
    let finalized = fixture.manifest().await.unwrap();
    assert_eq!(finalized.phase, OracleRecipeWeightPhase::Finalized);
    assert_eq!(finalized.processed_source_count, 3);
    assert_eq!(finalized.processed_bucket_count, 2);
    assert_eq!(finalized.declared_weight_total_bps, 10_000);
    assert_eq!(
        finalized.rolling_source_digest,
        fixture.canonical_source_digest
    );
}

#[tokio::test]
async fn source_manifest_rejects_reordered_or_incomplete_bucket_walks_atomically() {
    let mut reordered = ManifestFixture::start().await;
    let ix = reordered.accumulate_ix(&[1, 0], true);
    assert!(reordered.h.send(ix, &[]).await.is_err());
    assert!(reordered.manifest().await.is_none());

    let mut incomplete = ManifestFixture::start().await;
    let ix = incomplete.accumulate_ix(&[0], true);
    assert!(incomplete.h.send(ix, &[]).await.is_err());
    assert!(incomplete.manifest().await.is_none());
}
