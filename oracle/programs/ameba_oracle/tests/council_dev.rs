//! The council (tag 30 selector 18) on the localnet dev profile's council Config (tag 243): a
//! thin bucket that cannot reach the three-source floor after expiry is decided by three of five
//! seats, after which the month finalizes, settles and cleans up its council records. Seat
//! rotation moves the epoch: old messages, removed seats and empty rounds behave as specified.
#![cfg(feature = "localnet-dev-authority")]

mod common;

use ameba_oracle::{
    constants::ORACLE_SETTLEMENT_GRACE_SECONDS,
    error::OracleError,
    instruction::{
        self, OracleCouncilActionV1, RecomputeOracleBucketMedianV1Params, TerminalCleanupParams,
    },
    processor::{decode_council, CouncilCase},
    state::{
        derive_oracle_bucket_median_pda, OracleBucketMedianState, OracleBucketMedianStatus,
        OracleEmergencyDisputeKind, OracleMonthState, OracleSettlementStatus,
    },
};
use ameba_oracle_interface::PeriodStatus;
use borsh::BorshDeserialize;
use common::{cohort::*, settle::*, *};
use solana_program::hash::hashv;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

struct Council {
    config: Pubkey,
    seats: Vec<Keypair>,
}

impl Harness {
    async fn dev_council(&mut self, seats: Vec<Keypair>) -> Council {
        let admin = self.admin.insecure_clone();
        let keys: [Pubkey; 5] = core::array::from_fn(|i| seats[i].pubkey());
        self.send(
            instruction::dev_council_config(
                &program(),
                &admin.pubkey(),
                &self.ctx.payer.pubkey(),
                keys,
            ),
            &[&admin],
        )
        .await
        .unwrap();
        Council {
            config: instruction::find_dev_council_config_address(&program()).0,
            seats,
        }
    }

    async fn view(&mut self, council: &Council) -> (u64, [u8; 32]) {
        let account = self.account(council.config).await.unwrap();
        let view =
            decode_council(&program(), &council.config, &account.owner, &account.data).unwrap();
        (view.epoch, view.digest)
    }

    async fn case(&mut self, cohort: &Cohort, target: &Pubkey) -> Option<CouncilCase> {
        let key = instruction::find_council_case_address(&program(), &cohort.month, target);
        self.account(key)
            .await
            .map(|account| CouncilCase::try_from_slice(&account.data).unwrap())
    }

    #[allow(clippy::too_many_arguments)]
    async fn council_ix(
        &mut self,
        council: &Council,
        cohort: &Cohort,
        bucket_id: [u8; 32],
        operation: u8,
        choice: u8,
        voter: Option<&Pubkey>,
    ) -> solana_sdk::instruction::Instruction {
        let (epoch, digest) = self.view(council).await;
        let target = derive_oracle_bucket_median_pda(&program(), &cohort.month, &bucket_id).0;
        let case_hash = match operation {
            0 => [0; 32],
            _ => self.case(cohort, &target).await.unwrap().case_hash,
        };
        instruction::council_action(
            &program(),
            &self.ctx.payer.pubkey(),
            &cohort.period,
            &cohort.month,
            &target,
            &council.config,
            OracleCouncilActionV1 {
                operation,
                kind: OracleEmergencyDisputeKind::BucketMedian,
                target_id: bucket_id,
                expected_case_hash: case_hash,
                expected_epoch: epoch,
                expected_seats_hash: digest,
                choice,
            },
            voter,
            vec![],
        )
    }
}

fn seats() -> Vec<Keypair> {
    (0..5).map(|_| Keypair::new()).collect()
}

/// A Live cohort whose one bucket froze three sources but only one opened (the other two expire
/// unclaimed at the listing), walked past expiry and grace into EmergencyRequired: one eligible
/// source is below the floor of every profile.
async fn thin_bucket_needing_the_council(h: &mut Harness) -> (Cohort, [u8; 32]) {
    thin_bucket_with_update(h, None).await
}

/// As [`thin_bucket_needing_the_council`], with an accepted Game update of the opened source to
/// `update` an hour after the listing.
async fn thin_bucket_with_update(h: &mut Harness, update: Option<u64>) -> (Cohort, [u8; 32]) {
    let wide = wide_placed(h, None).await;
    h.wide_coverage(&wide).await;
    h.wide_recipe(&wide).await;
    h.wide_openings_of(&wide, &[0]).await;
    let cohort = wide.cohort.clone();
    h.at(cohort.listing).await;
    let payer = h.payer();
    for source in &wide.sources[1..] {
        h.send(
            instruction::expire_oracle_opening_source(
                &program(),
                &payer.pubkey(),
                &cohort.period,
                &cohort.month,
                source,
            ),
            &[],
        )
        .await
        .unwrap();
    }
    h.wide_active_weights(&wide).await;
    h.wide_list(&wide).await;
    if let Some(state) = update {
        use ameba_oracle::{instruction::UpdateFinalization, state::OracleUpdateClaimOutcome};
        let claimant = h.participant(PARTICIPANT_USDC).await;
        let now = cohort.listing + 3_600;
        h.at(now).await;
        let (source, source_id) = (wide.sources[0], WIDE_SOURCES[0]);
        let print = h
            .print(&cohort, &claimant, &source, &source_id, state, now - 60)
            .await;
        let claim = h
            .commit(&cohort, &claimant, &source, &source_id, 0, [3; 32], print)
            .await
            .unwrap();
        h.advance_slot().await;
        h.reveal(&cohort, &claimant, &source, &claim).await.unwrap();
        h.at(now + ameba_oracle::constants::ORACLE_UPDATE_CHALLENGE_SECONDS)
            .await;
        h.finalize(
            &cohort,
            &source,
            0,
            &claim.key,
            OracleUpdateClaimOutcome::AcceptClaim,
            1,
            UpdateFinalization::Unchallenged,
        )
        .await
        .unwrap();
    }
    let bucket_id = cohort.sku_ids[0];
    h.at(cohort.expiry + ORACLE_SETTLEMENT_GRACE_SECONDS).await;
    for mode in [0, 1] {
        for source in &wide.sources {
            h.send(
                instruction::recompute_oracle_bucket_median_v1(
                    &program(),
                    &payer.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    RecomputeOracleBucketMedianV1Params { bucket_id, mode },
                    source,
                    &instruction::find_oracle_source_observations_address(&program(), source),
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
    let bucket: OracleBucketMedianState = h
        .read_state(derive_oracle_bucket_median_pda(&program(), &cohort.month, &bucket_id).0)
        .await;
    assert_eq!(bucket.status, OracleBucketMedianStatus::EmergencyRequired);
    assert_eq!(
        (bucket.frozen_source_count, bucket.eligible_source_count),
        (3, 1)
    );
    (cohort, bucket_id)
}

#[tokio::test]
async fn three_seats_decide_a_thin_bucket_and_the_month_then_settles_and_cleans_up() {
    let mut h = start().await;
    let (cohort, bucket_id) = thin_bucket_needing_the_council(&mut h).await;
    let council = h.dev_council(seats()).await;
    assert_eq!(h.view(&council).await.0, 1);
    let target = derive_oracle_bucket_median_pda(&program(), &cohort.month, &bucket_id).0;

    // Open: the case records the snapshot and a deadline of both economics windows.
    let open = h.council_ix(&council, &cohort, bucket_id, 0, 0, None).await;
    h.send(open, &[]).await.unwrap();
    let case = h.case(&cohort, &target).await.unwrap();
    assert!(!case.finalized);
    assert_eq!(
        case.deadline - case.opened_slot,
        ameba_oracle::constants::DEV_COUNCIL_WINDOW_SLOTS
    );

    // Only seats vote, once each.
    let stranger = Keypair::new();
    let ix = h
        .council_ix(&council, &cohort, bucket_id, 1, 1, Some(&stranger.pubkey()))
        .await;
    assert_oracle_error(h.send(ix, &[&stranger]).await, OracleError::Unauthorized);
    for seat in &council.seats[..3] {
        let ix = h
            .council_ix(&council, &cohort, bucket_id, 1, 1, Some(&seat.pubkey()))
            .await;
        h.send(ix, &[seat]).await.unwrap();
    }
    let first = council.seats[0].insecure_clone();
    let ix = h
        .council_ix(&council, &cohort, bucket_id, 1, 0, Some(&first.pubkey()))
        .await;
    assert_oracle_error(
        h.send(ix, &[&first]).await,
        OracleError::InvalidOracleEmergencyDispute,
    );

    // The decision applies only after the deadline.
    let apply = h.council_ix(&council, &cohort, bucket_id, 2, 0, None).await;
    assert_oracle_error(
        h.send(apply.clone(), &[]).await,
        OracleError::OracleTimingWindowClosed,
    );
    h.advance_slots(case.deadline - case.opened_slot + 1).await;
    h.send(apply, &[]).await.unwrap();
    let decided = h.case(&cohort, &target).await.unwrap();
    assert!(decided.finalized);
    assert_eq!(
        (decided.outcome, decided.reason, decided.seat_mask),
        (1, 0, 0b111)
    );
    let bucket: OracleBucketMedianState = h.read_state(target).await;
    assert_eq!(bucket.status, OracleBucketMedianStatus::EmergencyRejected);

    // The month now finalizes and settles through the ordinary path.
    let payer = h.payer();
    h.send(
        instruction::finalize_oracle_month(
            &program(),
            &payer.pubkey(),
            &cohort.period,
            &cohort.month,
            &[bucket_id],
        ),
        &[],
    )
    .await
    .unwrap();
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.settlement_status, OracleSettlementStatus::Final);
    h.send(
        instruction::accumulate_oracle_settlement_source_bucket(
            &program(),
            &payer.pubkey(),
            &cohort.period,
            &cohort.month,
            ameba_oracle::instruction::AccumulateOracleSettlementSourceBucketParams {
                bucket_id,
                finalize_collection: true,
            },
            &[bucket_id],
        ),
        &[],
    )
    .await
    .unwrap();
    let leaf = settle_leaf(&mut h, &cohort).await;
    let signers: Vec<Keypair> = h
        .settlement_signers
        .iter()
        .map(|key| key.insecure_clone())
        .collect();
    let message =
        instruction::settlement_attestation_message(&program(), 1, &leaf.signer_set_hash, &leaf);
    let mut ixs: Vec<_> = signers[..3]
        .iter()
        .map(|signer| ed25519_verify_instruction(signer, &message))
        .collect();
    let oracle = h.oracle_authority.insecure_clone();
    ixs.push(instruction::upsert_settlement_v3(
        &program(),
        &oracle.pubkey(),
        &cohort.product,
        cohort.expiry,
        1,
        leaf.clone(),
    ));
    h.send_many(ixs, &[&oracle]).await.unwrap();
    let settled = h.period(cohort.product, cohort.expiry).await;
    assert_eq!(settled.status, PeriodStatus::Settled);
    assert_eq!(settled.settlement_value, leaf.settlement_price_atomic);

    // Cleanup reclaims the finalized case and its round.
    let admin = h.admin.insecure_clone();
    let cleanup = |kind: u8,
                   target: Option<&Pubkey>,
                   extra: Option<&Pubkey>,
                   hash: [u8; 32],
                   lamports: u64| {
        instruction::terminal_cleanup(
            &program(),
            &admin.pubkey(),
            &cohort.period,
            &cohort.month,
            &admin.pubkey(),
            &cohort.product,
            None,
            TerminalCleanupParams {
                kind,
                bucket_id: [0; 32],
                expected_data_hash: hash,
                expected_lamports: lamports,
            },
            target,
            extra,
        )
    };
    h.send(cleanup(0, None, None, [0; 32], 0), &[&admin])
        .await
        .unwrap();
    let case_key = instruction::find_council_case_address(&program(), &cohort.month, &target);
    let round_key = instruction::find_council_round_address(&program(), &case_key, 1);
    let round = h.account(round_key).await.unwrap();
    h.send(
        cleanup(
            5,
            Some(&round_key),
            Some(&case_key),
            hashv(&[&round.data]).to_bytes(),
            round.lamports,
        ),
        &[&admin],
    )
    .await
    .unwrap();
    let case_account = h.account(case_key).await.unwrap();
    h.send(
        cleanup(
            4,
            Some(&case_key),
            None,
            hashv(&[&case_account.data]).to_bytes(),
            case_account.lamports,
        ),
        &[&admin],
    )
    .await
    .unwrap();
    assert!(h.account(round_key).await.is_none());
    assert!(h.account(case_key).await.is_none());
}

async fn settle_leaf(h: &mut Harness, cohort: &Cohort) -> instruction::SettlementLeaf {
    let month: OracleMonthState = h.read_state(cohort.month).await;
    let manifest: ameba_oracle::state::OracleSettlementSourceManifest = h
        .read_state(
            ameba_oracle::state::derive_oracle_settlement_source_manifest_pda(
                &program(),
                &cohort.month,
            )
            .0,
        )
        .await;
    let set: ameba_oracle::state::SettlementSignerSet = h
        .read_state(ameba_oracle::state::derive_settlement_signer_set_pda(&program(), 1).0)
        .await;
    let factor = 10_000i128 + i128::from(month.index_delta_bps);
    let value = (i128::from(month.settlement_base_oracle_atomic) * factor + 5_000) / 10_000;
    instruction::SettlementLeaf {
        schema_version: instruction::SettlementLeaf::CURRENT_SCHEMA_VERSION,
        oracle_month: cohort.month,
        recipe_hash: month.recipe_hash,
        underlying_id: cohort.product,
        item_id: "ramx".into(),
        expiry_id: "RAMX-202703".into(),
        settlement_ts: cohort.expiry,
        price_display_decimals: 6,
        computation: instruction::SettlementComputation::OracleIndexDelta,
        trailing_window_days: 0,
        observations: vec![],
        settlement_price_atomic: value as u64,
        source_uri: "https://oracle.example/ramx/RAMX-202703".into(),
        source_digest: manifest.rolling_source_digest,
        base_oracle_atomic: month.settlement_base_oracle_atomic,
        index_delta_bps: month.index_delta_bps,
        submitted_by: Pubkey::default(),
        submitted_slot: 0,
        signer_set_version: 1,
        signer_set_hash: set.set_hash,
    }
}

#[tokio::test]
async fn rotation_moves_the_epoch_and_an_empty_round_falls_back() {
    let mut h = start().await;
    let (cohort, bucket_id) = thin_bucket_needing_the_council(&mut h).await;
    let council = h.dev_council(seats()).await;
    let target = derive_oracle_bucket_median_pda(&program(), &cohort.month, &bucket_id).0;
    let open = h.council_ix(&council, &cohort, bucket_id, 0, 0, None).await;
    h.send(open, &[]).await.unwrap();
    for seat in &council.seats[..3] {
        let ix = h
            .council_ix(&council, &cohort, bucket_id, 1, 1, Some(&seat.pubkey()))
            .await;
        h.send(ix, &[seat]).await.unwrap();
    }
    let opened = h.case(&cohort, &target).await.unwrap();
    let prepared = h.council_ix(&council, &cohort, bucket_id, 2, 0, None).await;
    let old_round = instruction::find_council_round_address(
        &program(),
        &instruction::find_council_case_address(&program(), &cohort.month, &target),
        1,
    );
    let old_round_bytes = h.account(old_round).await.unwrap().data;

    // Rotate the seats: epoch 2.
    let next = h.dev_council(seats()).await;
    assert_eq!(h.view(&next).await.0, 2);
    h.advance_slots(opened.deadline - opened.opened_slot + 1)
        .await;
    assert_oracle_error(
        h.send(prepared, &[]).await,
        OracleError::GovernanceGateEpochMismatch,
    );
    let removed = council.seats[0].insecure_clone();
    let ix = h
        .council_ix(&next, &cohort, bucket_id, 1, 1, Some(&removed.pubkey()))
        .await;
    assert!(h.send(ix, &[&removed]).await.is_err());
    // The current epoch's round is empty: the case falls back without extending its deadline.
    let apply = h.council_ix(&next, &cohort, bucket_id, 2, 0, None).await;
    h.send(apply, &[]).await.unwrap();
    let decided = h.case(&cohort, &target).await.unwrap();
    assert_eq!(
        (
            decided.outcome,
            decided.reason,
            decided.decision_epoch,
            decided.seat_mask
        ),
        (0, 1, 2, 0)
    );
    assert_eq!(decided.deadline, opened.deadline);
    assert_eq!(h.account(old_round).await.unwrap().data, old_round_bytes);
    let bucket: OracleBucketMedianState = h.read_state(target).await;
    assert_eq!(bucket.status, OracleBucketMedianStatus::EmergencyDefaulted);

    // Only the admin configures the dev council.
    let stranger = Keypair::new();
    h.fund(&stranger.pubkey(), 1_000_000_000).await;
    let keys: [Pubkey; 5] = core::array::from_fn(|_| Pubkey::new_unique());
    assert_oracle_error(
        h.send(
            instruction::dev_council_config(
                &program(),
                &stranger.pubkey(),
                &stranger.pubkey(),
                keys,
            ),
            &[&stranger],
        )
        .await,
        OracleError::Unauthorized,
    );
}

#[tokio::test]
async fn three_seats_confirm_a_thin_buckets_measured_median_and_the_month_settles_on_it() {
    let mut h = start().await;
    // The opened source prints +5% during Game; its bucket stays below the three-source floor.
    let (cohort, bucket_id) = thin_bucket_with_update(&mut h, Some(105_000_000)).await;
    let target = derive_oracle_bucket_median_pda(&program(), &cohort.month, &bucket_id).0;
    let bucket: OracleBucketMedianState = h.read_state(target).await;
    assert_eq!(bucket.status, OracleBucketMedianStatus::EmergencyRequired);
    assert_eq!(
        bucket.bucket_delta_bps, 0,
        "the last accepted value is the opening"
    );
    assert_eq!(
        bucket.opening_source_deltas_bps[0], 500,
        "the measured median"
    );
    let council = h.dev_council(seats()).await;
    let open = h.council_ix(&council, &cohort, bucket_id, 0, 0, None).await;
    h.send(open, &[]).await.unwrap();
    let case = h.case(&cohort, &target).await.unwrap();
    assert_eq!(
        case.choices, 3,
        "default, reject and confirm the measured median"
    );
    for seat in &council.seats[..3] {
        let ix = h
            .council_ix(&council, &cohort, bucket_id, 1, 2, Some(&seat.pubkey()))
            .await;
        h.send(ix, &[seat]).await.unwrap();
    }
    // An ordinary month's case applies after its deadline.
    let apply = h.council_ix(&council, &cohort, bucket_id, 2, 0, None).await;
    assert_oracle_error(
        h.send(apply.clone(), &[]).await,
        OracleError::OracleTimingWindowClosed,
    );
    h.advance_slots(case.deadline - case.opened_slot + 1).await;
    h.send(apply, &[]).await.unwrap();
    let decided = h.case(&cohort, &target).await.unwrap();
    assert_eq!(
        (decided.outcome, decided.reason, decided.seat_mask),
        (2, 0, 0b111)
    );
    let bucket: OracleBucketMedianState = h.read_state(target).await;
    assert_eq!(bucket.status, OracleBucketMedianStatus::EmergencyConfirmed);
    assert_eq!(bucket.bucket_delta_bps, 500);

    let payer = h.payer();
    h.send(
        instruction::finalize_oracle_month(
            &program(),
            &payer.pubkey(),
            &cohort.period,
            &cohort.month,
            &[bucket_id],
        ),
        &[],
    )
    .await
    .unwrap();
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.settlement_status, OracleSettlementStatus::Final);
    assert_eq!(
        month.index_delta_bps, 500,
        "the one bucket carries the whole weight"
    );
}

#[tokio::test]
async fn a_thin_bucket_without_updates_measures_its_opening() {
    let mut h = start().await;
    let (cohort, bucket_id) = thin_bucket_needing_the_council(&mut h).await;
    let target = derive_oracle_bucket_median_pda(&program(), &cohort.month, &bucket_id).0;
    let bucket: OracleBucketMedianState = h.read_state(target).await;
    // One opened source with only its opening print: eligible, measured median zero (the
    // council may still confirm it).
    assert_eq!(bucket.eligible_source_count, 1);
    assert_eq!(bucket.opening_source_deltas_bps, [0; 8]);
    let council = h.dev_council(seats()).await;
    let open = h.council_ix(&council, &cohort, bucket_id, 0, 0, None).await;
    h.send(open, &[]).await.unwrap();
    assert_eq!(h.case(&cohort, &target).await.unwrap().choices, 3);
}
