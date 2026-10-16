//! Tag 130, the council-approved CFM launch of a product's first month, in every profile the
//! integration tests run (production with the controller's Config, the localnet dev profile with
//! its own Config, and the production SBF artifact): three seats begin, the rows append and the
//! last one lists the period, and the ordinary 200 indexes the recipe; the receipt allows one
//! launch per product; and a launched month's council bucket settlement applies as soon as three
//! seats agree, then settles through 75, 113 and 116.

mod common;

use ameba_oracle::{
    cfm::{self, CfmLaunchReceipt, CfmRegistry, NANDX, RAMX},
    constants::{MAX_ORACLE_STATE_RECORDS_PER_STEP, ORACLE_SETTLEMENT_GRACE_SECONDS},
    error::OracleError,
    instruction::{
        self, AccumulateOracleSettlementSourceBucketParams, OracleCouncilActionV1,
        RecomputeOracleBucketMedianV1Params,
    },
    processor::{canonical_recipe_digest, decode_council, CouncilCase},
    state::{
        OracleActiveWeightManifest, OracleBucketMedianState, OracleBucketMedianStatus,
        OracleEmergencyDisputeKind, OracleMonthState, OraclePhase, OracleRecipeWeightManifest,
        OracleRecipeWeightPhase, OracleSettlementStatus, OracleSkuCoverageManifest,
        OracleSourceObservations, OracleSourceState, OracleSourceStatus, OracleUsdcRewardSchedule,
        OracleUsdcRewardSchedulePhase, OracleUsdcSkuPool,
    },
};
use ameba_oracle_interface::PeriodStatus;
use borsh::BorshDeserialize;
use common::{cfm::*, *};
use solana_sdk::{
    instruction::AccountMeta,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

/// A late rung: planned listing Oct 1, expiry Jan 1. The launch lists it at once.
const LATE_LISTING: u64 = OCT_01_2026;
const LATE_EXPIRY: u64 = JAN_01_2027;

async fn harness() -> (Harness, Council) {
    let mut h = start().await;
    h.initialize().await.unwrap();
    h.configure_economics(0).await.unwrap();
    let council = h.launch_council().await;
    (h, council)
}

async fn period(h: &mut Harness, product: &cfm::CfmProduct, planned: u64, expiry: u64) {
    let oracle = h.oracle_authority.insecure_clone();
    h.create_period(&oracle, product.product_id, planned, expiry, 6)
        .await
        .unwrap();
}

async fn receipt(h: &mut Harness, launch: &Launch) -> Option<CfmLaunchReceipt> {
    h.account(launch.accounts.receipt)
        .await
        .map(|a| CfmLaunchReceipt::try_from_slice(&a.data).unwrap())
}

async fn ramx_launch(h: &mut Harness) -> Launch {
    period(h, &RAMX, LATE_LISTING, LATE_EXPIRY).await;
    let plan = plan(&RAMX, LATE_EXPIRY, NOW);
    h.launch_objects(&RAMX, plan).await
}

#[test]
fn the_pinned_registries_compile_from_the_governed_documents() {
    for product in [RAMX, NANDX] {
        let compiled = registry(&product);
        assert_eq!(compiled.digest, product.registry_hash);
        assert_eq!(compiled.parents.len(), usize::from(product.parent_count));
        assert_eq!(compiled.bytes.len(), product.registry_len());
        let decoded = CfmRegistry::decode(&compiled.bytes).unwrap();
        let rows: Vec<_> = (0..decoded.len())
            .map(|i| decoded.row(i).unwrap())
            .collect();
        assert_eq!(rows.len(), usize::from(product.parent_count));
        assert_eq!(
            rows.iter().map(|r| u32::from(r.weight_bps)).sum::<u32>(),
            10_000
        );
        assert!(rows.windows(2).all(|w| w[0].source_id < w[1].source_id));
        assert!(decoded.row(product.parent_count).is_err());
        // The pinned hash rejects any other registry, however valid its shape.
        for at in [0, 1, 5, 40, 72, 72 + 96, compiled.bytes.len() - 1] {
            let mut tampered = compiled.bytes.clone();
            tampered[at] ^= 1;
            assert!(CfmRegistry::decode(&tampered).is_err(), "byte {at}");
        }
        // The registry's terminal root is the governed product manifest's.
        assert_eq!(
            sku_merkle_root(&{
                let mut ids: Vec<[u8; 32]> = terminal_labels(&product)
                    .iter()
                    .map(|l| cfm::compile::sku_id(l).unwrap())
                    .collect();
                ids.sort_unstable();
                ids
            }),
            product.terminal_root
        );
    }
    // The dev chain's dev months use the same 35 parent rows.
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dev-oracle-sources.json")).unwrap();
    for product in [RAMX, NANDX] {
        let compiled = registry(&product);
        let dev = fixture["products"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["product"] == product.symbol)
            .unwrap();
        let mut dev_ids: Vec<String> = dev["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["sourceId"].as_str().unwrap().to_string())
            .collect();
        dev_ids.sort();
        let ids: Vec<String> = compiled
            .parents
            .iter()
            .map(|p| {
                p.parent_source_id
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect()
            })
            .collect();
        assert_eq!(ids, dev_ids);
    }
}

#[tokio::test]
async fn a_launch_writes_an_ordinary_game_month_and_lists_the_period() {
    let (mut h, council) = harness().await;
    let launch = ramx_launch(&mut h).await;
    let a = launch.accounts;
    let count = u16::from(RAMX.parent_count);

    h.begin_launch(&launch, &council).await.unwrap();
    let r = receipt(&mut h, &launch).await.unwrap();
    assert_eq!(
        (
            r.product,
            r.row_count,
            r.cursor,
            r.finished,
            r.approving_seats
        ),
        (0, 13, 0, false, 0b111)
    );
    assert_eq!(
        (r.period, r.month, r.plan_hash),
        (a.period, a.month, launch.plan_hash)
    );
    assert_eq!(r.document_sha256, launch.plan.document_sha256);
    assert_eq!(r.proposer, h.ctx.payer.pubkey());
    let account = h.account(council.config).await.unwrap();
    let view = decode_council(
        &if cfg!(feature = "localnet-dev-authority") {
            program()
        } else {
            ameba_oracle::governance_gate::PINNED_CONTROLLER_PROGRAM_ID
        },
        &council.config,
        &account.owner,
        &account.data,
    )
    .unwrap();
    assert_eq!((r.council_epoch, r.seats_hash), (view.epoch, view.digest));
    let month: OracleMonthState = h.read_state(a.month).await;
    assert_eq!(month.phase, OraclePhase::Opening);
    assert_eq!(
        month.listing_ts, NOW,
        "the later of the planned listing and now"
    );
    assert_eq!(month.settlement_base_oracle_atomic, LAUNCH_BASE);
    assert_eq!(month.authority, h.oracle_authority.pubkey());
    let coverage: OracleSkuCoverageManifest = h.read_state(a.coverage).await;
    assert!(coverage.coverage_finalized);
    assert_eq!(coverage.required_sku_count, count);
    assert_eq!(
        coverage.required_sku_root,
        sku_merkle_root(&launch.source_ids())
    );
    assert_eq!(
        h.period(RAMX.product_id, LATE_EXPIRY).await.status,
        PeriodStatus::Forming
    );

    // Nothing ordinary advances a month mid-launch.
    let payer = h.payer();
    let ix = instruction::finalize_oracle_opening_phase(
        &program(),
        &payer.pubkey(),
        &a.period,
        &a.month,
    );
    assert!(h.send(ix, &[]).await.is_err());
    let ix = instruction::fail_period(&program(), &payer.pubkey(), &RAMX.product_id, LATE_EXPIRY);
    assert_oracle_error(h.send(ix, &[]).await, OracleError::PeriodNotFailable);

    // Every row but the last: the month is still forming, and the ordinary active-weight
    // finalization (124) cannot run on the launch's collecting manifest.
    for row in 0..RAMX.parent_count - 1 {
        let ix = h.append_ix(&launch, row);
        h.send(ix, &[]).await.unwrap();
    }
    let ix = instruction::finalize_oracle_active_weights(
        &program(),
        &payer.pubkey(),
        &a.period,
        &a.month,
    );
    assert!(h.send(ix, &[]).await.is_err());
    assert_eq!(
        h.period(RAMX.product_id, LATE_EXPIRY).await.status,
        PeriodStatus::Forming
    );
    let ix = h.append_ix(&launch, RAMX.parent_count - 1);
    h.send(ix, &[]).await.unwrap();
    let recipe: OracleRecipeWeightManifest = h.read_state(a.recipe).await;
    assert_eq!(recipe.phase, OracleRecipeWeightPhase::Finalized);
    let month: OracleMonthState = h.read_state(a.month).await;
    assert_eq!(
        month.recipe_hash,
        canonical_recipe_digest(&a.month, &month.weight_manifest_hash)
    );
    assert_eq!(
        month.weight_manifest_hash,
        launch.recipe_chain()[usize::from(count)]
    );
    assert_eq!(
        (
            month.source_count,
            month.frozen_source_count,
            month.opened_source_count
        ),
        (count, count, count)
    );
    // The ordinary recipe index (200) runs on the listed month, as settlement needs.
    h.index_launch(&launch).await;

    let p = h.period(RAMX.product_id, LATE_EXPIRY).await;
    assert_eq!(
        (p.status, p.effective_listing_ts),
        (PeriodStatus::Live, NOW)
    );
    assert!(p.is_tradable_at(NOW));
    let month: OracleMonthState = h.read_state(a.month).await;
    assert_eq!(month.phase, OraclePhase::Game);
    assert_eq!(month.active_weight_group_count, count);
    assert_eq!(month.index_delta_bps, 0);
    let active: OracleActiveWeightManifest = h.read_state(a.active).await;
    assert_eq!(active.phase, OracleRecipeWeightPhase::Finalized);
    assert_eq!(
        active.rolling_manifest_hash,
        month.active_weight_manifest_hash
    );
    let schedule: OracleUsdcRewardSchedule = h.read_state(a.schedule).await;
    assert_eq!(
        (schedule.phase, schedule.sku_pool_count),
        (OracleUsdcRewardSchedulePhase::Funded, count)
    );
    let r = receipt(&mut h, &launch).await.unwrap();
    assert!(r.finished && r.cursor == 13 && r.finished_at == NOW);
    for (row, parent) in launch.registry.parents.iter().enumerate() {
        let rows = a.row(&program(), &parent.parent_source_id);
        let source: OracleSourceState = h.read_state(rows.source).await;
        let (value, time) = launch.plan.rows[row];
        assert_eq!(source.status, OracleSourceStatus::Active);
        assert_eq!(
            (source.baseline_state, source.current_state),
            (value, value)
        );
        assert_eq!(
            (source.observation_count, source.latest_source_time),
            (1, time)
        );
        assert_eq!(source.bucket_weight_bps, parent.weight_bps);
        assert_eq!(source.canonical_locator_hash, parent.locator_hash);
        assert_eq!(source.source_definition_hash, parent.definition_hash);
        assert_eq!(source.opening_evidence_hash, launch.plan_hash);
        let observations: OracleSourceObservations = h.read_state(rows.observations).await;
        assert_eq!(
            (observations.states[0], observations.source_times[0]),
            (value, time)
        );
        let bucket: OracleBucketMedianState = h.read_state(rows.bucket).await;
        assert_eq!(bucket.status, OracleBucketMedianStatus::Live);
        assert_eq!(
            (bucket.group_index, bucket.frozen_source_count),
            (row as u16, 1)
        );
        let pool: OracleUsdcSkuPool = h.read_state(rows.sku_pool).await;
        assert_eq!(pool.update_min_bond, LAUNCH_TERMS.update_min_bond);
        assert!(h.account(rows.checkpoint).await.is_some());
    }

    // Finished: no step repeats.
    let ix = h.append_ix(&launch, 12);
    assert_oracle_error(h.send(ix, &[]).await, OracleError::InvalidCfmLaunchReceipt);
}

#[tokio::test]
async fn each_product_launches_once() {
    let (mut h, council) = harness().await;
    let first = ramx_launch(&mut h).await;
    h.begin_launch(&first, &council).await.unwrap();

    // Another RAMX period, with its own valid plan: the receipt already exists.
    period(&mut h, &RAMX, PLANNED, EXPIRY).await;
    let again = h.launch_objects(&RAMX, plan(&RAMX, EXPIRY, NOW)).await;
    assert_ne!(again.accounts.period, first.accounts.period);
    assert_eq!(again.accounts.receipt, first.accounts.receipt);
    h.advance_slot().await;
    assert_oracle_error(
        h.begin_launch(&again, &council).await,
        OracleError::AlreadyInitialized,
    );
    assert!(h.account(again.accounts.month).await.is_none());

    // NANDX has its own receipt and launches once too.
    period(&mut h, &NANDX, LATE_LISTING, LATE_EXPIRY).await;
    let nandx = h
        .launch_objects(&NANDX, plan(&NANDX, LATE_EXPIRY, NOW))
        .await;
    h.begin_launch(&nandx, &council).await.unwrap();
    let r = receipt(&mut h, &nandx).await.unwrap();
    assert_eq!((r.product, r.row_count), (1, 22));
    h.advance_slot().await;
    assert_oracle_error(
        h.begin_launch(&nandx, &council).await,
        OracleError::AlreadyInitialized,
    );
}

#[tokio::test]
async fn three_distinct_current_seats_must_sign() {
    let (mut h, council) = harness().await;
    let launch = ramx_launch(&mut h).await;
    let outsider = Keypair::new();
    let seats: Vec<Keypair> = council.seats.iter().map(|s| s.insecure_clone()).collect();

    // Two seats and an outsider.
    let ix = h.begin_ix(
        &launch,
        &council,
        [seats[0].pubkey(), seats[1].pubkey(), outsider.pubkey()],
    );
    assert_oracle_error(
        h.send(ix, &[&seats[0], &seats[1], &outsider]).await,
        OracleError::CfmLaunchCouncilRequired,
    );
    // Three seats listed, one of them not signing.
    let mut ix = h.begin_ix(&launch, &council, council.three());
    ix.accounts[15].is_signer = false;
    assert_oracle_error(
        h.send(ix, &[&seats[0], &seats[1]]).await,
        OracleError::CfmLaunchCouncilRequired,
    );
    // One seat twice.
    let ix = h.begin_ix(
        &launch,
        &council,
        [seats[0].pubkey(), seats[1].pubkey(), seats[1].pubkey()],
    );
    assert_oracle_error(
        h.send(ix, &[&seats[0], &seats[1]]).await,
        OracleError::InvalidAccountList,
    );
    // Seats of another Config.
    let mut ix = h.begin_ix(&launch, &council, council.three());
    ix.accounts[12] = AccountMeta::new_readonly(h.oracle_authority.pubkey(), false);
    assert_oracle_error(
        h.send(ix, &[&seats[0], &seats[1], &seats[2]]).await,
        OracleError::InvalidGovernanceGateData,
    );
    assert!(receipt(&mut h, &launch).await.is_none());
    assert!(h.account(launch.accounts.month).await.is_none());

    // Any three of the five.
    let ix = h.begin_ix(
        &launch,
        &council,
        [seats[4].pubkey(), seats[2].pubkey(), seats[3].pubkey()],
    );
    h.send(ix, &[&seats[2], &seats[3], &seats[4]])
        .await
        .unwrap();
    assert_eq!(
        receipt(&mut h, &launch).await.unwrap().approving_seats,
        0b11100
    );
}

#[tokio::test]
async fn the_plan_digest_binds_the_approved_plan() {
    let (mut h, council) = harness().await;
    let launch = ramx_launch(&mut h).await;
    let seats: Vec<Keypair> = council.seats[..3]
        .iter()
        .map(|s| s.insecure_clone())
        .collect();
    let signers: Vec<&Keypair> = seats.iter().collect();

    // The payload's digest is not the sealed object's commitment.
    let mut other = launch.plan.clone();
    other.rows[0].0 += 1;
    let other_hash = cfm::plan_digest(&program(), &other.encode());
    let mut ix = h.begin_ix(&launch, &council, council.three());
    ix.data[4..36].copy_from_slice(&other_hash);
    assert_oracle_error(
        h.send(ix, &signers).await,
        OracleError::InvalidCfmLaunchPlan,
    );

    // A sealed, self-consistent plan for another expiry, or for the other product.
    let mut stale = launch.plan.clone();
    stale.expiry_ts = EXPIRY;
    let wrong_expiry = h.launch_objects(&RAMX, stale).await;
    let ix = h.begin_ix(&wrong_expiry, &council, council.three());
    let mut ix = ix;
    ix.accounts[1] = AccountMeta::new(launch.accounts.period, false);
    for (index, key) in [
        (2, launch.accounts.month),
        (3, launch.accounts.coverage),
        (5, launch.accounts.recipe),
        (6, launch.accounts.active),
        (7, launch.accounts.schedule),
    ] {
        ix.accounts[index] = AccountMeta::new(key, false);
    }
    assert_oracle_error(
        h.send(ix, &signers).await,
        OracleError::InvalidCfmLaunchPlan,
    );
    let nandx_plan = h
        .launch_objects(&NANDX, plan(&NANDX, LATE_EXPIRY, NOW))
        .await;
    let mut ix = h.begin_ix(&launch, &council, council.three());
    ix.data[4..36].copy_from_slice(&nandx_plan.plan_hash);
    ix.accounts[10] = AccountMeta::new_readonly(nandx_plan.plan_object, false);
    assert_oracle_error(
        h.send(ix, &signers).await,
        OracleError::InvalidCfmLaunchPlan,
    );
    // The other product's registry.
    let mut ix = h.begin_ix(&launch, &council, council.three());
    ix.accounts[11] = AccountMeta::new_readonly(nandx_plan.registry_object, false);
    assert_oracle_error(
        h.send(ix, &signers).await,
        OracleError::InvalidCfmLaunchPlan,
    );
    // A print after now.
    let mut future = launch.plan.clone();
    future.rows[3].1 = NOW + 1;
    let future = h.launch_objects(&RAMX, future).await;
    let ix = h.begin_ix(&future, &council, council.three());
    assert_oracle_error(
        h.send(ix, &signers).await,
        OracleError::InvalidCfmLaunchPlan,
    );
    assert!(receipt(&mut h, &launch).await.is_none());

    // Begun under the approved digest, rows append only under it, in order and once.
    h.begin_launch(&launch, &council).await.unwrap();
    let mut ix = h.append_ix(&launch, 0);
    ix.data[4..36].copy_from_slice(&other_hash);
    assert_oracle_error(h.send(ix, &[]).await, OracleError::InvalidCfmLaunchReceipt);
    let ix = h.append_ix(&launch, 1);
    assert_oracle_error(h.send(ix, &[]).await, OracleError::InvalidCfmLaunchReceipt);
    let ix = h.append_ix(&launch, 0);
    h.send(ix, &[]).await.unwrap();
    h.advance_slot().await;
    let ix = h.append_ix(&launch, 0);
    assert_oracle_error(h.send(ix, &[]).await, OracleError::InvalidCfmLaunchReceipt);
}

impl Harness {
    async fn council_case(&mut self, month: &Pubkey, target: &Pubkey) -> CouncilCase {
        let key = instruction::find_council_case_address(&program(), month, target);
        CouncilCase::try_from_slice(&self.account(key).await.unwrap().data).unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    async fn bucket_council(
        &mut self,
        council: &Council,
        launch: &Launch,
        bucket_id: [u8; 32],
        operation: u8,
        choice: u8,
        voter: Option<&Pubkey>,
        remaining: Vec<AccountMeta>,
    ) -> solana_sdk::instruction::Instruction {
        let account = self.account(council.config).await.unwrap();
        let controller = if cfg!(feature = "localnet-dev-authority") {
            program()
        } else {
            ameba_oracle::governance_gate::PINNED_CONTROLLER_PROGRAM_ID
        };
        let view =
            decode_council(&controller, &council.config, &account.owner, &account.data).unwrap();
        let target = ameba_oracle::state::derive_oracle_bucket_median_pda(
            &program(),
            &launch.accounts.month,
            &bucket_id,
        )
        .0;
        let expected_case_hash = if operation == 0 {
            [0; 32]
        } else {
            self.council_case(&launch.accounts.month, &target)
                .await
                .case_hash
        };
        instruction::council_action(
            &program(),
            &self.ctx.payer.pubkey(),
            &launch.accounts.period,
            &launch.accounts.month,
            &target,
            &council.config,
            OracleCouncilActionV1 {
                operation,
                kind: OracleEmergencyDisputeKind::BucketMedian,
                target_id: bucket_id,
                expected_case_hash,
                expected_epoch: view.epoch,
                expected_seats_hash: view.digest,
                choice,
            },
            voter,
            remaining,
        )
    }
}

#[tokio::test]
async fn the_council_settles_a_launched_month_as_soon_as_three_seats_agree() {
    let (mut h, council) = harness().await;
    h.activate().await;
    // The oracle authority publishes (116) and pays the settlement record.
    let oracle_key = h.oracle_authority.pubkey();
    h.fund(&oracle_key, 5_000_000_000).await;
    let launch = ramx_launch(&mut h).await;
    h.launch(&launch, &council).await;
    let a = launch.accounts;
    let ids = launch.source_ids();
    let payer = h.payer();

    // One accepted Game update: the first row's source prints 10% above its opening.
    let opening = launch.plan.rows[0].0;
    h.at(NOW + 3_600).await;
    h.launch_update(&launch, 0, opening + opening / 10, NOW + 3_540)
        .await;

    // After expiry and grace every one-source bucket is below the three-source floor, in every
    // profile: 197 sends it to grace, then to the council, measuring each bucket's median.
    h.at(LATE_EXPIRY + ORACLE_SETTLEMENT_GRACE_SECONDS).await;
    for mode in [0, 1] {
        for (row, id) in ids.iter().enumerate() {
            let source = launch.source(row);
            h.send(
                instruction::recompute_oracle_bucket_median_v1(
                    &program(),
                    &payer.pubkey(),
                    &a.period,
                    &a.month,
                    RecomputeOracleBucketMedianV1Params {
                        bucket_id: *id,
                        mode,
                    },
                    &source,
                    &instruction::find_oracle_source_observations_address(&program(), &source),
                    0,
                    &solana_sdk::system_program::id(),
                ),
                &[],
            )
            .await
            .unwrap();
        }
        h.advance_slot().await;
    }
    for id in &ids {
        let bucket: OracleBucketMedianState = h
            .read_state(
                ameba_oracle::state::derive_oracle_bucket_median_pda(&program(), &a.month, id).0,
            )
            .await;
        assert_eq!(bucket.status, OracleBucketMedianStatus::EmergencyRequired);
        assert_eq!(
            (bucket.frozen_source_count, bucket.eligible_source_count),
            (1, 1)
        );
        // The measured median the council may confirm: the updated source moved +10%.
        let measured = if *id == ids[0] { 1_000 } else { 0 };
        assert_eq!(bucket.opening_source_deltas_bps[0], measured);
        assert_eq!(
            bucket.bucket_delta_bps, 0,
            "the last accepted value is the opening"
        );
    }

    let receipt_meta = AccountMeta::new_readonly(a.receipt, false);
    let seats: Vec<Keypair> = council.seats.iter().map(|s| s.insecure_clone()).collect();
    for (index, id) in ids.iter().enumerate() {
        let open = h
            .bucket_council(&council, &launch, *id, 0, 0, None, vec![])
            .await;
        h.send(open, &[]).await.unwrap();
        let target =
            ameba_oracle::state::derive_oracle_bucket_median_pda(&program(), &a.month, id).0;
        assert_eq!(h.council_case(&a.month, &target).await.choices, 3);
        // Bucket 1 is defaulted (0, keep the last accepted value); every other bucket's measured
        // median is confirmed (2).
        let choice = if index == 1 { 0 } else { 2 };
        let votes = if index == 1 { 2 } else { 3 };
        for seat in &seats[..votes] {
            let vote = h
                .bucket_council(
                    &council,
                    &launch,
                    *id,
                    1,
                    choice,
                    Some(&seat.pubkey()),
                    vec![],
                )
                .await;
            h.send(vote, &[seat]).await.unwrap();
        }
        if index == 0 {
            // Without the receipt the case waits for its deadline; a wrong receipt is rejected.
            let apply = h
                .bucket_council(&council, &launch, *id, 2, 0, None, vec![])
                .await;
            assert_oracle_error(
                h.send(apply, &[]).await,
                OracleError::OracleTimingWindowClosed,
            );
            let wrong = AccountMeta::new_readonly(a.month, false);
            let apply = h
                .bucket_council(&council, &launch, *id, 2, 0, None, vec![wrong])
                .await;
            assert!(h.send(apply, &[]).await.is_err());
            let other = AccountMeta::new_readonly(
                cfm::find_receipt_address(&program(), &NANDX.product_id).0,
                false,
            );
            let apply = h
                .bucket_council(&council, &launch, *id, 2, 0, None, vec![other])
                .await;
            assert_oracle_error(
                h.send(apply, &[]).await,
                OracleError::InvalidCfmLaunchReceipt,
            );
        }
        if index == 1 {
            // Two votes are not a decision, receipt or not.
            let apply = h
                .bucket_council(
                    &council,
                    &launch,
                    *id,
                    2,
                    0,
                    None,
                    vec![receipt_meta.clone()],
                )
                .await;
            assert_oracle_error(
                h.send(apply, &[]).await,
                OracleError::OracleTimingWindowClosed,
            );
            let vote = h
                .bucket_council(
                    &council,
                    &launch,
                    *id,
                    1,
                    0,
                    Some(&seats[4].pubkey()),
                    vec![],
                )
                .await;
            h.send(vote, &[&seats[4]]).await.unwrap();
        }
        // Three matching current votes and the finished receipt: it applies in the same slot.
        let apply = h
            .bucket_council(
                &council,
                &launch,
                *id,
                2,
                0,
                None,
                vec![receipt_meta.clone()],
            )
            .await;
        h.send(apply, &[]).await.unwrap();
        let case = h.council_case(&a.month, &target).await;
        assert!(case.finalized);
        assert!(
            case.resolved_slot <= case.deadline,
            "applied before the deadline"
        );
        assert_eq!((case.outcome, case.reason), (choice, 0));
        let bucket: OracleBucketMedianState = h.read_state(target).await;
        if index == 1 {
            assert_eq!(bucket.status, OracleBucketMedianStatus::EmergencyDefaulted);
        } else {
            assert_eq!(bucket.status, OracleBucketMedianStatus::EmergencyConfirmed);
        }
        assert_eq!(
            bucket.bucket_delta_bps,
            if index == 0 { 1_000 } else { 0 },
            "a confirmed bucket settles on its measured median"
        );
    }
    let month: OracleMonthState = h.read_state(a.month).await;
    let weight = launch.registry.parents[0].weight_bps;
    assert_eq!(
        month.index_delta_bps,
        ameba_oracle::processor::bucket_index_contribution_bps(weight, 1_000).unwrap()
    );

    // The month then finalizes and settles on the ordinary path.
    h.send(
        instruction::finalize_oracle_month(&program(), &payer.pubkey(), &a.period, &a.month, &ids),
        &[],
    )
    .await
    .unwrap();
    let month: OracleMonthState = h.read_state(a.month).await;
    assert_eq!(month.settlement_status, OracleSettlementStatus::Final);
    for (index, chunk) in ids.chunks(MAX_ORACLE_STATE_RECORDS_PER_STEP).enumerate() {
        let last = (index + 1) * MAX_ORACLE_STATE_RECORDS_PER_STEP >= ids.len();
        h.send(
            instruction::accumulate_oracle_settlement_source_bucket(
                &program(),
                &payer.pubkey(),
                &a.period,
                &a.month,
                AccumulateOracleSettlementSourceBucketParams {
                    bucket_id: chunk[0],
                    finalize_collection: last,
                },
                chunk,
            ),
            &[],
        )
        .await
        .unwrap();
    }
    let cohort = launch.cohort(NOW);
    let leaf = h.leaf_for(&cohort).await;
    let signers: Vec<Keypair> = h
        .settlement_signers
        .iter()
        .map(|k| k.insecure_clone())
        .collect();
    let signers: Vec<&Keypair> = signers[..3].iter().collect();
    h.publish(&cohort, &signers, &leaf, &leaf).await.unwrap();
    let settled = h.period(RAMX.product_id, LATE_EXPIRY).await;
    assert_eq!(settled.status, PeriodStatus::Settled);
    // The confirmed bucket moves the month: round(base × (10000 + Δ) / 10000), above the base.
    let delta = i128::from(month.index_delta_bps);
    let expected = (i128::from(LAUNCH_BASE) * (10_000 + delta) + 5_000) / 10_000;
    assert_eq!(settled.settlement_value, expected as u64);
    assert!(settled.settlement_value > LAUNCH_BASE);
}
