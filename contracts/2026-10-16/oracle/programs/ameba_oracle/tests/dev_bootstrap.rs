//! Localnet dev bootstrap (`localnet-dev-authority`): Game-phase months and Active sources for
//! the dev expiries, loaded from the dev source fixture (`tests/fixtures/dev-oracle-sources.json`,
//! a byte-identical copy of the futures ops fixture), their settlement at expiry, and the
//! profile's off-ladder periods.
#![cfg(any(feature = "localnet-dev-authority", feature = "localnet-governance"))]

mod common;

use ameba_oracle::{
    error::OracleError,
    instruction::{
        self, DevBootstrapMonthParams, DevBootstrapSourceParams, InitializeOracleMonthV5Params,
    },
    state::{
        derive_oracle_sku_coverage_manifest_pda, derive_settlement_v2_pda, OracleMonthState,
        OraclePhase, OracleSettlementStatus, OracleSkuCoverageManifest, OracleSourceObservations,
        OracleSourceState, OracleSourceStatus, SettlementRecordV2,
    },
};
use ameba_oracle_interface::{find_period_address, PeriodStatus};
use common::{cohort::*, *};
use serde::Deserialize;
use solana_program::hash::hashv;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::Signer, transaction::Transaction,
};

const DEV_BASE: u64 = 100_000_000;
const NOV_EXPIRY: u64 = 1_793_491_200;
const DEC_EXPIRY: u64 = 1_796_083_200;
const AUG_01_2026: u64 = 1_785_542_400;
const SEP_01_2026: u64 = 1_788_220_800;
const MARKER: &str = "Program log: AMEBA_ORACLE_LOCALNET_DEV_AUTHORITY";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    products: Vec<FixtureProduct>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureProduct {
    product: String,
    sources: Vec<FixtureSource>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FixtureSource {
    source_id: String,
    bucket_id: String,
    url: String,
    locator_hash: String,
    status: String,
    bucket_weight_bps: u16,
    baseline_state_atomic: String,
}

fn hex32(value: &str) -> [u8; 32] {
    assert_eq!(value.len(), 64);
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    out
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/dev-oracle-sources.json")).unwrap()
}

/// The product's bucket set in ascending order, as its coverage root commits to it.
fn bucket_set(product: &FixtureProduct) -> Vec<[u8; 32]> {
    let mut ids: Vec<[u8; 32]> = product
        .sources
        .iter()
        .map(|s| hex32(&s.bucket_id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn definition_hash_for(source_id: &[u8; 32]) -> [u8; 32] {
    hashv(&[b"ameba-oracle-localnet-dev-definition-v1", source_id]).to_bytes()
}

const SOURCE_TYPE: [u8; 32] = [0xd5; 32];

impl Harness {
    /// Send and return the program's log lines.
    async fn send_logged(
        &mut self,
        ix: Instruction,
        signers: &[&solana_sdk::signature::Keypair],
    ) -> Result<Vec<String>, solana_program_test::BanksClientError> {
        #[cfg(feature = "localnet-governance")]
        let ix = {
            use ameba_oracle::governance_gate::*;
            let mut ix = ix;
            ix.data
                .extend_from_slice(&GovernanceInstructionTailV1::for_epoch(1).encode());
            ix.accounts
                .push(solana_sdk::instruction::AccountMeta::new_readonly(
                    derive_protocol_gate_pda(&PINNED_CONTROLLER_PROGRAM_ID, &program()).0,
                    false,
                ));
            ix
        };
        let payer = self.payer();
        let blockhash = self.ctx.get_new_latest_blockhash().await.unwrap();
        let mut all = vec![&payer];
        for signer in signers {
            if signer.pubkey() != payer.pubkey() {
                all.push(signer);
            }
        }
        let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &all, blockhash);
        let result = self
            .ctx
            .banks_client
            .process_transaction_with_metadata(tx)
            .await?;
        result.result?;
        Ok(result.metadata.map(|m| m.log_messages).unwrap_or_default())
    }

    async fn dev_ready(&mut self) {
        self.initialize().await.unwrap();
        self.split_oracle_authority().await;
        let oracle_key = self.oracle_authority.pubkey();
        self.fund(&oracle_key, 5_000_000_000).await;
        self.configure_economics(0).await.unwrap();
    }

    async fn bootstrap_month(
        &mut self,
        product: &[u8; 32],
        expiry: u64,
        buckets: &[[u8; 32]],
    ) -> Result<Vec<String>, solana_program_test::BanksClientError> {
        let oracle = self.oracle_authority.insecure_clone();
        let ix = instruction::dev_bootstrap_month(
            &program(),
            &oracle.pubkey(),
            &self.ctx.payer.pubkey(),
            product,
            expiry,
            DevBootstrapMonthParams {
                settlement_base_oracle_atomic: DEV_BASE,
                required_sku_root: sku_merkle_root(buckets),
                required_sku_count: buckets.len() as u16,
            },
        );
        self.send_logged(ix, &[&oracle]).await
    }

    fn source_ix(
        &self,
        period: &Pubkey,
        month: &Pubkey,
        buckets: &[[u8; 32]],
        source: &FixtureSource,
    ) -> Instruction {
        let source_id = hex32(&source.source_id);
        let bucket_id = hex32(&source.bucket_id);
        let sku_index = buckets.iter().position(|b| *b == bucket_id).unwrap() as u16;
        instruction::dev_bootstrap_source(
            &program(),
            &self.oracle_authority.pubkey(),
            &self.ctx.payer.pubkey(),
            &self.ctx.payer.pubkey(),
            period,
            month,
            DevBootstrapSourceParams {
                source_id,
                bucket_id,
                source_type_hash: SOURCE_TYPE,
                canonical_locator_hash: hex32(&source.locator_hash),
                source_definition_hash: definition_hash_for(&source_id),
                bucket_weight_bps: source.bucket_weight_bps,
                opening_state: source.baseline_state_atomic.parse().unwrap(),
                sku_index,
                sku_proof: sku_merkle_proof(buckets, sku_index),
            },
        )
    }

    /// Seal the locator object through tag 30 selector 14 and bootstrap the source.
    async fn bootstrap_source(
        &mut self,
        period: &Pubkey,
        month: &Pubkey,
        buckets: &[[u8; 32]],
        source: &FixtureSource,
    ) -> Result<Vec<String>, solana_program_test::BanksClientError> {
        let payer = self.payer();
        let hash = self.upload_evidence(&payer, 1, source.url.as_bytes()).await;
        assert_eq!(hash, hex32(&source.locator_hash), "fixture locatorHash");
        self.advance_slot().await;
        let ix = self.source_ix(period, month, buckets, source);
        let oracle = self.oracle_authority.insecure_clone();
        self.send_logged(ix, &[&oracle]).await
    }

    /// A Live dev month for `product` at `expiry` with every fixture source of the product.
    async fn live_dev_month(
        &mut self,
        product: &FixtureProduct,
        planned: u64,
        expiry: u64,
    ) -> (Pubkey, Pubkey) {
        let product_id = common::product(&product.product);
        let buckets = bucket_set(product);
        let oracle = self.oracle_authority.insecure_clone();
        self.create_period(&oracle, product_id, planned, expiry, 6)
            .await
            .unwrap();
        self.bootstrap_month(&product_id, expiry, &buckets)
            .await
            .unwrap();
        let period = find_period_address(&program(), &product_id, expiry).0;
        let month = instruction::find_oracle_month_address(&program(), &period, expiry);
        for source in &product.sources {
            self.bootstrap_source(&period, &month, &buckets, source)
                .await
                .unwrap();
        }
        (period, month)
    }

    async fn settle_month(
        &mut self,
        signer: &solana_sdk::signature::Keypair,
        product_id: &[u8; 32],
        expiry: u64,
    ) -> Result<Vec<String>, solana_program_test::BanksClientError> {
        let ix = instruction::dev_settle_month(
            &program(),
            &signer.pubkey(),
            &self.ctx.payer.pubkey(),
            product_id,
            expiry,
        );
        self.send_logged(ix, &[signer]).await
    }
}

/// The digest tag 242 writes: `sha256(domain ‖ period ‖ month ‖ expiry ‖ base ‖ Δ ‖ value ‖
/// settled_ts)`, integers little-endian.
fn dev_digest(
    period: &Pubkey,
    month: &Pubkey,
    expiry: u64,
    base: u64,
    delta: i64,
    value: u64,
    settled_ts: u64,
) -> [u8; 32] {
    hashv(&[
        b"ameba-oracle-localnet-dev-settlement-v1",
        period.as_ref(),
        month.as_ref(),
        &expiry.to_le_bytes(),
        &base.to_le_bytes(),
        &delta.to_le_bytes(),
        &value.to_le_bytes(),
        &settled_ts.to_le_bytes(),
    ])
    .to_bytes()
}

/// The index reader the futures API uses: phase at 86, delta at 125, the settlement-record tag at
/// 142, base at 169 (tag 0) or 201 (tag 1); value `(base * (10_000 + delta) + 5_000) / 10_000`.
fn api_index(data: &[u8]) -> Option<(u8, i64, u64)> {
    if data.len() != 299 || data[0] != 1 || &data[2..5] != b"OMS" {
        return None;
    }
    let base_at = match data[142] {
        0 => 169,
        1 => 201,
        _ => return None,
    };
    let delta = i64::from_le_bytes(data[125..133].try_into().unwrap());
    let base = u64::from_le_bytes(data[base_at..base_at + 8].try_into().unwrap());
    let factor = 10_000i128 + i128::from(delta);
    let value = (i128::from(base) * factor + 5_000) / 10_000;
    Some((data[86], delta, u64::try_from(value).ok()?))
}

/// The SBF program logs through the runtime; the native builtin prints to stdout instead.
fn assert_dev_marker(logs: &[String]) {
    if cfg!(feature = "test-sbf") {
        assert!(
            logs.iter().any(|line| line == MARKER),
            "dev marker logged: {logs:?}"
        );
    }
}

#[test]
fn fixture_locator_hashes_are_the_canonical_locator_hash_of_their_url() {
    let fixture = fixture();
    let mut total = 0;
    for product in &fixture.products {
        let weights: u32 = product
            .sources
            .iter()
            .map(|s| u32::from(s.bucket_weight_bps))
            .sum();
        assert_eq!(weights, 10_000, "{} weights", product.product);
        for source in &product.sources {
            assert_eq!(source.status, "Active");
            assert_eq!(
                locator_hash(source.url.as_bytes()),
                hex32(&source.locator_hash),
                "{}",
                source.url
            );
            total += 1;
        }
    }
    assert_eq!(total, 35);
}

#[tokio::test]
async fn dev_months_go_live_and_load_every_fixture_source() {
    let mut h = start().await;
    h.dev_ready().await;
    let fixture = fixture();
    let oracle = h.oracle_authority.insecure_clone();
    for product in &fixture.products {
        let product_id = common::product(&product.product);
        let buckets = bucket_set(product);
        for (planned, expiry) in [(AUG_01_2026, NOV_EXPIRY), (SEP_01_2026, DEC_EXPIRY)] {
            h.create_period(&oracle, product_id, planned, expiry, 6)
                .await
                .unwrap();
            let period = find_period_address(&program(), &product_id, expiry).0;
            let month = instruction::find_oracle_month_address(&program(), &period, expiry);
            let logs = h
                .bootstrap_month(&product_id, expiry, &buckets)
                .await
                .unwrap();
            assert_dev_marker(&logs);

            // Live at the later of the planned listing and now.
            let live = h.period(product_id, expiry).await;
            assert_eq!(live.status, PeriodStatus::Live);
            assert_eq!(live.effective_listing_ts, planned.max(NOW));
            assert!(live.is_tradable_at(NOW));
            assert!(!live.is_tradable_at(expiry));
            assert_eq!(live.value_decimals, 6);

            for source in &product.sources {
                let logs = h
                    .bootstrap_source(&period, &month, &buckets, source)
                    .await
                    .unwrap();
                assert_dev_marker(&logs);
            }

            let state: OracleMonthState = h.read_state(month).await;
            assert_eq!(state.phase, OraclePhase::Game);
            assert_eq!(state.period, period);
            assert_eq!(state.listing_ts, live.effective_listing_ts);
            assert_eq!(state.settlement_base_oracle_atomic, DEV_BASE);
            assert_eq!(state.index_delta_bps, 0);
            assert_eq!(state.source_count as usize, product.sources.len());
            assert_eq!(state.opened_source_count, state.source_count);
            assert_eq!(state.effective_weight_total_bps, 10_000);
            let coverage: OracleSkuCoverageManifest = h
                .read_state(derive_oracle_sku_coverage_manifest_pda(&program(), &month).0)
                .await;
            assert!(coverage.coverage_finalized);
            assert_eq!(coverage.covered_sku_count as usize, buckets.len());

            // The index read the futures API performs.
            let raw = h.account(month).await.unwrap();
            assert_eq!(raw.owner, program());
            let (phase, delta, value) = api_index(&raw.data).expect("API decodes the month");
            assert_eq!(phase, 2, "Game");
            assert_eq!(delta, 0);
            assert_eq!(value, 100_000_000, "100.000000");
            assert_eq!(
                format!("{}.{:06}", value / 1_000_000, value % 1_000_000),
                "100.000000"
            );

            // Every source decodes, Active, with its URL readable through its locator link.
            for source in &product.sources {
                let source_id = hex32(&source.source_id);
                let key = instruction::find_oracle_source_address(&program(), &month, &source_id);
                let state: OracleSourceState = h.read_state(key).await;
                assert_eq!(state.status, OracleSourceStatus::Active);
                assert_eq!(state.bucket_id, hex32(&source.bucket_id));
                assert_eq!(state.bucket_weight_bps, source.bucket_weight_bps);
                assert_eq!(state.canonical_locator_hash, hex32(&source.locator_hash));
                let opening: u64 = source.baseline_state_atomic.parse().unwrap();
                assert_eq!(state.baseline_state, opening);
                assert_eq!(state.current_state, opening);
                assert_eq!(state.observation_count, 1);
                let observations: OracleSourceObservations = h
                    .read_state(instruction::find_oracle_source_observations_address(
                        &program(),
                        &key,
                    ))
                    .await;
                assert_eq!(observations.states[0], opening);
                let link = h
                    .account(instruction::find_evidence_link_address(
                        &program(),
                        &key,
                        0,
                        &state.canonical_locator_hash,
                    ))
                    .await
                    .expect("locator link");
                assert_eq!(&link.data[38..70], key.as_ref());
                let object_key = Pubkey::new_from_array(link.data[102..134].try_into().unwrap());
                let object = h.account(object_key).await.unwrap();
                assert_eq!(&object.data[76..], source.url.as_bytes());
                assert!(h
                    .account(instruction::find_checkpoint_address(&program(), &key, &key))
                    .await
                    .is_some());
            }
        }
    }
}

#[tokio::test]
async fn dev_bootstrap_is_create_only_and_refuses_ordinary_or_mismatched_state() {
    let mut h = start().await;
    h.dev_ready().await;
    let fixture = fixture();
    let ramx = fixture
        .products
        .iter()
        .find(|p| p.product == "RAMX")
        .unwrap();
    let product_id = common::product("RAMX");
    let buckets = bucket_set(ramx);
    let oracle = h.oracle_authority.insecure_clone();
    h.create_period(&oracle, product_id, AUG_01_2026, NOV_EXPIRY, 6)
        .await
        .unwrap();
    let period = find_period_address(&program(), &product_id, NOV_EXPIRY).0;
    let month = instruction::find_oracle_month_address(&program(), &period, NOV_EXPIRY);

    // Only the oracle authority bootstraps.
    let by_admin = instruction::dev_bootstrap_month(
        &program(),
        &h.admin.pubkey(),
        &h.ctx.payer.pubkey(),
        &product_id,
        NOV_EXPIRY,
        DevBootstrapMonthParams {
            settlement_base_oracle_atomic: DEV_BASE,
            required_sku_root: sku_merkle_root(&buckets),
            required_sku_count: buckets.len() as u16,
        },
    );
    let admin = h.admin.insecure_clone();
    assert_oracle_error(h.send(by_admin, &[&admin]).await, OracleError::Unauthorized);

    h.bootstrap_month(&product_id, NOV_EXPIRY, &buckets)
        .await
        .unwrap();
    let month_before = h.account(month).await.unwrap();
    let period_before = h.account(period).await.unwrap();
    // A retry fails without mutation: the period is no longer Forming.
    h.advance_slot().await;
    assert_oracle_error(
        h.bootstrap_month(&product_id, NOV_EXPIRY, &buckets)
            .await
            .map(|_| ()),
        OracleError::InvalidPeriodTransition,
    );
    assert_eq!(h.account(month).await.unwrap().data, month_before.data);
    assert_eq!(h.account(period).await.unwrap().data, period_before.data);

    let first = &ramx.sources[0];
    h.bootstrap_source(&period, &month, &buckets, first)
        .await
        .unwrap();
    let source_key =
        instruction::find_oracle_source_address(&program(), &month, &hex32(&first.source_id));
    let source_before = h.account(source_key).await.unwrap();
    h.advance_slot().await;
    let retry = h.source_ix(&period, &month, &buckets, first);
    assert_oracle_error(
        h.send(retry, &[&oracle]).await,
        OracleError::AlreadyInitialized,
    );
    assert_eq!(
        h.account(source_key).await.unwrap().data,
        source_before.data
    );

    // A bucket outside the month's set does not prove membership.
    let mut stranger = ramx.sources[1].clone();
    stranger.bucket_id = "ff".repeat(32);
    stranger.source_id = "ee".repeat(32);
    let mut ix = h.source_ix(&period, &month, &buckets, &ramx.sources[1]);
    let mut data = ix.data.clone();
    data[1..33].copy_from_slice(&hex32(&stranger.source_id));
    data[33..65].copy_from_slice(&hex32(&stranger.bucket_id));
    ix.data = data;
    ix.accounts[6].pubkey =
        instruction::find_oracle_source_address(&program(), &month, &hex32(&stranger.source_id));
    assert_oracle_error(
        h.send(ix, &[&oracle]).await,
        OracleError::InvalidOracleSkuMembershipProof,
    );

    // A locator hash that the sealed object does not commit to publishes nothing.
    let mut wrong_url = ramx.sources[1].clone();
    wrong_url.locator_hash = hex::encode_like(&locator_hash(b"https://example.com/other"));
    let ix = h.source_ix(&period, &month, &buckets, &wrong_url);
    assert!(h.send(ix, &[&oracle]).await.is_err());

    // An ordinary month (tag 181, still in source submission) is never a dev month.
    let ordinary_ids = synthetic_sku_ids(1);
    let ordinary_root = h
        .register_product(common::product("NANDX"), &ordinary_ids)
        .await;
    h.create_period(&oracle, common::product("NANDX"), PLANNED, EXPIRY, 6)
        .await
        .unwrap();
    h.send(
        instruction::initialize_oracle_month_v5(
            &program(),
            &oracle.pubkey(),
            &h.ctx.payer.pubkey(),
            &common::product("NANDX"),
            EXPIRY,
            InitializeOracleMonthV5Params {
                scramble_start_ts: PLANNED - PROGRAM_PRE_LISTING_SECONDS,
                listing_ts: PLANNED,
                settlement_base_oracle_atomic: DEV_BASE,
                required_sku_root: ordinary_root,
                required_sku_count: 1,
            },
        ),
        &[&oracle],
    )
    .await
    .unwrap();
    let ordinary_period = find_period_address(&program(), &common::product("NANDX"), EXPIRY).0;
    let ordinary_month =
        instruction::find_oracle_month_address(&program(), &ordinary_period, EXPIRY);
    let ix = h.source_ix(
        &ordinary_period,
        &ordinary_month,
        &buckets,
        &ramx.sources[2],
    );
    assert_oracle_error(
        h.send(ix, &[&oracle]).await,
        OracleError::InvalidOraclePhase,
    );
}

#[tokio::test]
async fn dev_settlement_settles_a_bootstrapped_month_at_expiry_and_replays_without_change() {
    let mut h = start().await;
    h.dev_ready().await;
    let fixture = fixture();
    let ramx = fixture
        .products
        .iter()
        .find(|p| p.product == "RAMX")
        .unwrap();
    let product_id = common::product("RAMX");
    let (period, month) = h.live_dev_month(ramx, AUG_01_2026, NOV_EXPIRY).await;
    let oracle = h.oracle_authority.insecure_clone();
    let record_key = derive_settlement_v2_pda(&program(), &period, &month).0;

    // Not before expiry, and only by the oracle authority.
    h.at(NOV_EXPIRY - 1).await;
    assert_oracle_error(
        h.settle_month(&oracle, &product_id, NOV_EXPIRY)
            .await
            .map(|_| ()),
        OracleError::PeriodNotExpired,
    );
    h.at(NOV_EXPIRY).await;
    let admin = h.admin.insecure_clone();
    assert_oracle_error(
        h.settle_month(&admin, &product_id, NOV_EXPIRY)
            .await
            .map(|_| ()),
        OracleError::Unauthorized,
    );
    assert_eq!(
        h.period(product_id, NOV_EXPIRY).await.status,
        PeriodStatus::Live
    );

    // At expiry exactly, with no grace.
    let logs = h
        .settle_month(&oracle, &product_id, NOV_EXPIRY)
        .await
        .unwrap();
    assert_dev_marker(&logs);
    let expected_digest = dev_digest(
        &period,
        &month,
        NOV_EXPIRY,
        DEV_BASE,
        0,
        100_000_000,
        NOV_EXPIRY,
    );
    // `Harness::period` reads through `load_period_for`.
    let settled = h.period(product_id, NOV_EXPIRY).await;
    assert_eq!(settled.status, PeriodStatus::Settled);
    let settlement = settled
        .settlement()
        .expect("Settled period has a settlement");
    assert_eq!(settlement.value, 100_000_000);
    assert_eq!(settlement.decimals, 6);
    assert_eq!(settlement.settled_ts, NOV_EXPIRY);
    assert_eq!(settlement.digest, expected_digest);
    assert!(!settled.is_tradable_at(NOV_EXPIRY));

    let state: OracleMonthState = h.read_state(month).await;
    assert_eq!(state.phase, OraclePhase::Settled);
    assert_eq!(state.settlement_record, Some(record_key));
    assert_eq!(state.settlement_status, OracleSettlementStatus::Final);
    assert_eq!(state.finalized_at_ts, NOV_EXPIRY);
    assert_eq!(state.c_settle_bps, 10_000);
    let record: SettlementRecordV2 = h.read_state(record_key).await;
    assert_eq!(record.account_discriminator, *b"STL");
    assert_eq!(record.period, period);
    assert_eq!(record.oracle_month, month);
    assert_eq!(record.settlement_ts, NOV_EXPIRY);
    assert_eq!(record.settlement_price_atomic, 100_000_000);
    assert_eq!(record.signed_leaf_commitment, expected_digest);
    assert_eq!(record.submitted_by, oracle.pubkey());
    assert_eq!(record.signer_set_version, 0);

    // The futures index read sees the record tag at 142 and the base at 201.
    let raw = h.account(month).await.unwrap();
    assert_eq!(raw.data[142], 1);
    assert_eq!(&raw.data[143..175], record_key.as_ref());
    let (phase, delta, value) = api_index(&raw.data).expect("API decodes the settled month");
    assert_eq!((phase, delta, value), (3, 0, 100_000_000));

    // A replay later changes nothing.
    let before = (
        h.account(period).await.unwrap().data,
        raw.data.clone(),
        h.account(record_key).await.unwrap().data,
    );
    h.at(NOV_EXPIRY + 3_600).await;
    h.settle_month(&oracle, &product_id, NOV_EXPIRY)
        .await
        .unwrap();
    assert_eq!(
        before,
        (
            h.account(period).await.unwrap().data,
            h.account(month).await.unwrap().data,
            h.account(record_key).await.unwrap().data,
        )
    );
}

#[tokio::test]
async fn dev_settlement_uses_the_stored_delta_and_refuses_negative_factors_and_ordinary_months() {
    let mut h = start().await;
    h.dev_ready().await;
    let fixture = fixture();
    let nandx = fixture
        .products
        .iter()
        .find(|p| p.product == "NANDX")
        .unwrap();
    let product_id = common::product("NANDX");
    let (period, month) = h.live_dev_month(nandx, SEP_01_2026, DEC_EXPIRY).await;
    let oracle = h.oracle_authority.insecure_clone();
    h.at(DEC_EXPIRY + 60).await;

    // A factor below zero is invalid; zero-valued settlements are valid in Period v2.
    let mut state: OracleMonthState = h.read_state(month).await;
    state.index_delta_bps = -10_001;
    h.set_program_state(month, &state, OracleMonthState::LEN)
        .await;
    assert_oracle_error(
        h.settle_month(&oracle, &product_id, DEC_EXPIRY)
            .await
            .map(|_| ()),
        OracleError::InvalidSettlementRecord,
    );
    // Δ as stored, rounded half up: 100 × 1.2345 = 123.450000.
    state.index_delta_bps = 2_345;
    h.set_program_state(month, &state, OracleMonthState::LEN)
        .await;
    h.advance_slot().await;
    h.settle_month(&oracle, &product_id, DEC_EXPIRY)
        .await
        .unwrap();
    let settled = h.period(product_id, DEC_EXPIRY).await;
    assert_eq!(settled.settlement_value, 123_450_000);
    assert_eq!(settled.settled_ts, DEC_EXPIRY + 60);
    assert_eq!(
        settled.settlement_digest,
        dev_digest(
            &period,
            &month,
            DEC_EXPIRY,
            DEV_BASE,
            2_345,
            123_450_000,
            DEC_EXPIRY + 60
        )
    );
    let raw = h.account(month).await.unwrap();
    assert_eq!(api_index(&raw.data), Some((3, 2_345, 123_450_000)));

    // A month formed by the ordinary instructions is never a dev month, even after expiry.
    let ordinary_ids = synthetic_sku_ids(1);
    let ordinary_root = h
        .register_product(common::product("RAMX"), &ordinary_ids)
        .await;
    h.at(NOW).await;
    h.create_period(&oracle, common::product("RAMX"), PLANNED, EXPIRY, 6)
        .await
        .unwrap();
    h.send(
        instruction::initialize_oracle_month_v5(
            &program(),
            &oracle.pubkey(),
            &h.ctx.payer.pubkey(),
            &common::product("RAMX"),
            EXPIRY,
            InitializeOracleMonthV5Params {
                scramble_start_ts: PLANNED - PROGRAM_PRE_LISTING_SECONDS,
                listing_ts: PLANNED,
                settlement_base_oracle_atomic: DEV_BASE,
                required_sku_root: ordinary_root,
                required_sku_count: 1,
            },
        ),
        &[&oracle],
    )
    .await
    .unwrap();
    h.at(EXPIRY + 1).await;
    assert_oracle_error(
        h.settle_month(&oracle, &common::product("RAMX"), EXPIRY)
            .await
            .map(|_| ()),
        OracleError::InvalidOraclePhase,
    );
    let ordinary_period = find_period_address(&program(), &common::product("RAMX"), EXPIRY).0;
    let ordinary_month =
        instruction::find_oracle_month_address(&program(), &ordinary_period, EXPIRY);
    assert!(h
        .account(derive_settlement_v2_pda(&program(), &ordinary_period, &ordinary_month).0)
        .await
        .is_none());
    assert_eq!(
        h.period(common::product("RAMX"), EXPIRY).await.status,
        PeriodStatus::Forming
    );
}

#[tokio::test]
async fn dev_periods_take_any_future_expiry_and_keep_every_other_check() {
    let mut h = start().await;
    h.dev_ready().await;
    let oracle = h.oracle_authority.insecure_clone();
    let fixture = fixture();
    let ramx = fixture
        .products
        .iter()
        .find(|p| p.product == "RAMX")
        .unwrap();
    let product_id = common::product("RAMX");
    let short_expiry = NOW + 3_600;

    for (planned, expiry, decimals, product) in [
        // Expiry not after now.
        (NOW - 60, NOW, 6, product_id),
        // Planned listing not before expiry.
        (short_expiry, short_expiry, 6, product_id),
        // Not six decimals.
        (NOW - 60, short_expiry, 7, product_id),
        // No product.
        (NOW - 60, short_expiry, 6, [0; 32]),
    ] {
        assert_oracle_error(
            h.create_period(&oracle, product, planned, expiry, decimals)
                .await,
            OracleError::InvalidPeriodSchedule,
        );
    }
    let admin = h.admin.insecure_clone();
    assert_oracle_error(
        h.create_period(&admin, product_id, NOW - 60, short_expiry, 6)
            .await,
        OracleError::Unauthorized,
    );

    // An hour-long period off the ladder runs the whole dev lifecycle.
    let (_period, _month) = h.live_dev_month(ramx, NOW - 60, short_expiry).await;
    let live = h.period(product_id, short_expiry).await;
    assert_eq!(live.status, PeriodStatus::Live);
    assert_eq!(live.effective_listing_ts, NOW);
    assert!(live.is_tradable_at(NOW + 1_800));
    h.at(short_expiry).await;
    h.settle_month(&oracle, &product_id, short_expiry)
        .await
        .unwrap();
    let settled = h.period(product_id, short_expiry).await;
    assert_eq!(settled.status, PeriodStatus::Settled);
    assert_eq!(settled.settlement_value, 100_000_000);
    assert_eq!(settled.settled_ts, short_expiry);
}

/// Hex helper for the fixture's lowercase strings.
mod hex {
    pub fn encode_like(bytes: &[u8; 32]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
