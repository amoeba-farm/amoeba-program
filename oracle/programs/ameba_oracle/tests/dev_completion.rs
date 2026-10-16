//! Localnet dev profile: a month made by tags 240 and 241 (one source per bucket, as on the dev
//! chain) is completed in place by tag 244 and the ordinary indexing and active-weight
//! instructions, then takes Game updates (166, 198, 199) and settles through the ordinary path
//! (197, 75, 113, 116) at the value its updated sources give.
#![cfg(any(feature = "localnet-dev-authority", feature = "localnet-governance"))]

mod common;

use ameba_oracle::{
    constants::{ORACLE_SETTLEMENT_FRESHNESS_BUSINESS_DAYS, ORACLE_SETTLEMENT_GRACE_SECONDS},
    error::OracleError,
    instruction::{
        self, AccumulateOracleActiveWeightGroupParams,
        AccumulateOracleSettlementSourceBucketParams, DevBootstrapMonthParams,
        DevBootstrapSourceParams, DevFormationBucketParams, IndexOracleRecipeSourceV1Params,
        RecomputeOracleBucketMedianV1Params, UpdateFinalization,
    },
    processor::{
        advance_oracle_weight_manifest_hash, canonical_recipe_digest,
        initial_oracle_weight_manifest_hash, oracle_settlement_window_start,
    },
    state::{
        derive_oracle_bucket_median_pda, derive_oracle_usdc_reward_schedule_pda,
        OracleBucketMedianState, OracleBucketMedianStatus, OracleMonthState, OraclePhase,
        OracleSettlementStatus, OracleSourceState, OracleUpdateClaimOutcome,
        OracleUsdcRewardSchedule, OracleUsdcRewardSchedulePhase,
    },
};
use ameba_oracle_interface::{find_period_address, PeriodStatus};
use common::{cohort::*, settle::*, *};
use solana_sdk::{pubkey::Pubkey, signature::Keypair, signature::Signer};

const DAY: u64 = 86_400;
const DEV_BASE: u64 = 100_000_000;
/// The first dev expiry (2026-11-01).
const DEV_EXPIRY: u64 = NOV_01_2026;
/// Four buckets, one source each: (weight, opening, updated state or none).
const SOURCES: [(u16, u64, Option<u64>); 4] = [
    (4_000, 100_000_000, Some(110_000_000)),
    (3_000, 50_000_000, Some(47_500_000)),
    (2_000, 200_000_000, Some(205_000_000)),
    (1_000, 80_000_000, None),
];

fn bucket_id(index: usize) -> [u8; 32] {
    [0x11 * (index as u8 + 1); 32]
}

fn source_id(index: usize) -> [u8; 32] {
    [0xa1 + index as u8; 32]
}

fn bonds(bucket: [u8; 32]) -> DevFormationBucketParams {
    let budget = sku_budget(bucket);
    DevFormationBucketParams {
        bucket_id: bucket,
        bucket_weight_bps: 0,
        finalize_collection: true,
        proposer_reward_bps: budget.proposer_reward_bps,
        listing_bond: budget.listing_bond,
        support_bond: budget.support_bond,
        opening_bond: budget.opening_bond,
        update_min_bond: budget.update_min_bond,
        challenge_min_bond: budget.challenge_min_bond,
        challenge_max_bond: budget.challenge_max_bond,
        challenge_bond_bps: budget.challenge_bond_bps,
    }
}

/// The frozen recipe chain of the dev sources: `chain[i]` precedes source `i`.
fn recipe_chain(month: &Pubkey) -> Vec<[u8; 32]> {
    let mut hash = initial_oracle_weight_manifest_hash(month, 4, 4);
    let mut chain = vec![hash];
    for (index, (weight, _, _)) in SOURCES.iter().enumerate() {
        hash =
            advance_oracle_weight_manifest_hash(&hash, &bucket_id(index), &frozen(index), *weight);
        chain.push(hash);
    }
    chain
}

fn frozen(index: usize) -> OracleSourceState {
    let id = source_id(index);
    OracleSourceState {
        source_id: id,
        source_type_hash: SOURCE_TYPE_HASH,
        canonical_locator_hash: locator_hash(&locator(&id)),
        source_definition_hash: definition_hash(&definition(&id)),
        ..OracleSourceState::default()
    }
}

impl Harness {
    /// Tags 0, 128, 110, 1, 240 and 241: the dev chain's shape, paused and without signers.
    async fn bootstrapped_month(&mut self) -> Cohort {
        self.initialize().await.unwrap();
        self.split_oracle_authority().await;
        let oracle_key = self.oracle_authority.pubkey();
        self.fund(&oracle_key, 5_000_000_000).await;
        self.configure_economics(0).await.unwrap();
        let product = product("RAMX");
        let oracle = self.oracle_authority.insecure_clone();
        self.create_period(&oracle, product, OCT_01_2026, DEV_EXPIRY, 6)
            .await
            .unwrap();
        let buckets: Vec<[u8; 32]> = (0..SOURCES.len()).map(bucket_id).collect();
        let root = sku_merkle_root(&buckets);
        let payer = self.payer();
        self.send(
            instruction::dev_bootstrap_month(
                &program(),
                &oracle.pubkey(),
                &payer.pubkey(),
                &product,
                DEV_EXPIRY,
                DevBootstrapMonthParams {
                    settlement_base_oracle_atomic: DEV_BASE,
                    required_sku_root: root,
                    required_sku_count: buckets.len() as u16,
                },
            ),
            &[&oracle],
        )
        .await
        .unwrap();
        let period = find_period_address(&program(), &product, DEV_EXPIRY).0;
        let month = instruction::find_oracle_month_address(&program(), &period, DEV_EXPIRY);
        for (index, (weight, opening, _)) in SOURCES.iter().enumerate() {
            let id = source_id(index);
            self.upload_evidence(&payer, 1, &locator(&id)).await;
            self.advance_slot().await;
            self.send(
                instruction::dev_bootstrap_source(
                    &program(),
                    &oracle.pubkey(),
                    &payer.pubkey(),
                    &payer.pubkey(),
                    &period,
                    &month,
                    DevBootstrapSourceParams {
                        source_id: id,
                        bucket_id: bucket_id(index),
                        source_type_hash: SOURCE_TYPE_HASH,
                        canonical_locator_hash: locator_hash(&locator(&id)),
                        source_definition_hash: definition_hash(&definition(&id)),
                        bucket_weight_bps: *weight,
                        opening_state: *opening,
                        sku_index: index as u16,
                        sku_proof: sku_merkle_proof(&buckets, index as u16),
                    },
                ),
                &[&oracle],
            )
            .await
            .unwrap();
        }
        let state: OracleMonthState = self.read_state(month).await;
        Cohort {
            product,
            sku_ids: buckets,
            root,
            period,
            month,
            listing: state.listing_ts,
            scramble: state.scramble_start_ts,
            expiry: DEV_EXPIRY,
        }
    }

    /// Tag 244 begin, one bucket step per bucket, finalize.
    async fn complete_recipe(&mut self, cohort: &Cohort) {
        let oracle = self.oracle_authority.insecure_clone();
        let payer = self.payer();
        self.send(
            instruction::dev_begin_formation(
                &program(),
                &oracle.pubkey(),
                &payer.pubkey(),
                &cohort.period,
                &cohort.month,
                4,
                4,
            ),
            &[&oracle],
        )
        .await
        .unwrap();
        for (index, (weight, _, _)) in SOURCES.iter().enumerate() {
            self.send(
                instruction::dev_formation_bucket(
                    &program(),
                    &oracle.pubkey(),
                    &payer.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    DevFormationBucketParams {
                        bucket_weight_bps: *weight,
                        ..bonds(bucket_id(index))
                    },
                    &[cohort.source(&source_id(index))],
                ),
                &[&oracle],
            )
            .await
            .unwrap();
        }
        self.send(
            instruction::dev_finalize_formation(
                &program(),
                &oracle.pubkey(),
                &cohort.period,
                &cohort.month,
            ),
            &[&oracle],
        )
        .await
        .unwrap();
    }

    /// The ordinary tags: 200 per source in reverse recipe order, 121, 122 per bucket, 124.
    async fn index_and_weigh(&mut self, cohort: &Cohort) {
        let payer = self.payer();
        let oracle = self.oracle_authority.insecure_clone();
        let chain = recipe_chain(&cohort.month);
        for index in (0..SOURCES.len()).rev() {
            let source = frozen(index);
            self.send(
                instruction::index_oracle_recipe_source_v1(
                    &program(),
                    &payer.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    IndexOracleRecipeSourceV1Params {
                        previous_hash: chain[index],
                        bucket_id: bucket_id(index),
                        source_id: source.source_id,
                        source_type_hash: source.source_type_hash,
                        canonical_locator_hash: source.canonical_locator_hash,
                        source_definition_hash: source.source_definition_hash,
                        bucket_weight_bps: SOURCES[index].0,
                    },
                    0,
                ),
                &[],
            )
            .await
            .unwrap();
        }
        self.send(
            instruction::begin_oracle_active_weights(
                &program(),
                &oracle.pubkey(),
                &cohort.period,
                &cohort.month,
                4,
                4,
            ),
            &[&oracle],
        )
        .await
        .unwrap();
        for index in 0..SOURCES.len() {
            self.send(
                instruction::accumulate_oracle_active_weight_group(
                    &program(),
                    &payer.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    AccumulateOracleActiveWeightGroupParams {
                        group_id: bucket_id(index),
                        finalize_collection: true,
                    },
                    &cohort.source(&source_id(index)),
                    0,
                    &solana_sdk::system_program::id(),
                ),
                &[],
            )
            .await
            .unwrap();
        }
        self.send(
            instruction::finalize_oracle_active_weights(
                &program(),
                &payer.pubkey(),
                &cohort.period,
                &cohort.month,
            ),
            &[],
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn a_bootstrapped_month_completes_in_place_and_settles_on_its_updates() {
    let mut h = start().await;
    let cohort = h.bootstrapped_month().await;
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Live
    );

    // Signers and unpause (90, 129), as the dev chain needs before 121 and 116.
    h.activate().await;

    // Updates cannot start before the month has an active-weight manifest.
    let updater = h.participant(PARTICIPANT_USDC).await;
    let window =
        oracle_settlement_window_start(cohort.expiry, ORACLE_SETTLEMENT_FRESHNESS_BUSINESS_DAYS)
            .unwrap();
    let now = window + DAY;
    h.at(now).await;
    let source0 = cohort.source(&source_id(0));
    let early = h
        .print(
            &cohort,
            &updater,
            &source0,
            &source_id(0),
            101_000_000,
            now - 120,
        )
        .await;
    assert_oracle_error(
        h.commit(
            &cohort,
            &updater,
            &source0,
            &source_id(0),
            0,
            [9; 32],
            early,
        )
        .await
        .map(|_| ()),
        OracleError::InvalidOracleActiveWeightManifest,
    );

    h.complete_recipe(&cohort).await;
    let month: OracleMonthState = h.read_state(cohort.month).await;
    let chain = recipe_chain(&cohort.month);
    assert_eq!(month.weight_manifest_hash, chain[SOURCES.len()]);
    assert_eq!(
        month.recipe_hash,
        canonical_recipe_digest(&cohort.month, &chain[SOURCES.len()])
    );
    assert_eq!(month.phase, OraclePhase::Game);
    let schedule: OracleUsdcRewardSchedule = h
        .read_state(derive_oracle_usdc_reward_schedule_pda(&program(), &cohort.month).0)
        .await;
    assert_eq!(schedule.phase, OracleUsdcRewardSchedulePhase::Funded);
    assert_eq!(
        (schedule.sku_pool_count, schedule.total_reward_budget),
        (4, 0)
    );

    h.index_and_weigh(&cohort).await;
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.active_weight_group_count, 4);
    assert_eq!(month.index_delta_bps, 0);

    // The completed month is an ordinary month now: the dev source and settlement tags refuse it,
    // and tag 244 cannot start again.
    let oracle = h.oracle_authority.insecure_clone();
    let payer = h.payer();
    assert_oracle_error(
        h.send(
            instruction::dev_begin_formation(
                &program(),
                &oracle.pubkey(),
                &payer.pubkey(),
                &cohort.period,
                &cohort.month,
                4,
                4,
            ),
            &[&oracle],
        )
        .await,
        OracleError::InvalidOraclePhase,
    );

    // Game: three of the four sources move by +1000, -500 and +250 bps inside the settlement
    // window.
    let mut claims = Vec::new();
    for (index, (_, _, update)) in SOURCES.iter().enumerate() {
        let Some(state) = update else { continue };
        let source = cohort.source(&source_id(index));
        let print = h
            .print(
                &cohort,
                &updater,
                &source,
                &source_id(index),
                *state,
                now - 60,
            )
            .await;
        let claim = h
            .commit(
                &cohort,
                &updater,
                &source,
                &source_id(index),
                index,
                [index as u8 + 1; 32],
                print,
            )
            .await
            .unwrap();
        h.advance_slot().await;
        h.reveal(&cohort, &updater, &source, &claim).await.unwrap();
        claims.push((index, source, claim));
    }
    h.at(now + DAY).await;
    for (index, source, claim) in &claims {
        h.finalize(
            &cohort,
            source,
            *index,
            &claim.key,
            OracleUpdateClaimOutcome::AcceptClaim,
            1,
            UpdateFinalization::Unchallenged,
        )
        .await
        .unwrap();
    }

    // At expiry the dev settlement refuses the completed month.
    h.at(cohort.expiry).await;
    assert_oracle_error(
        h.send(
            instruction::dev_settle_month(
                &program(),
                &oracle.pubkey(),
                &payer.pubkey(),
                &cohort.product,
                cohort.expiry,
            ),
            &[&oracle],
        )
        .await,
        OracleError::InvalidOraclePhase,
    );

    // After expiry and grace: one median step per bucket settles each from its own source.
    h.at(cohort.expiry + ORACLE_SETTLEMENT_GRACE_SECONDS).await;
    for index in 0..SOURCES.len() {
        let source = cohort.source(&source_id(index));
        h.send(
            instruction::recompute_oracle_bucket_median_v1(
                &program(),
                &payer.pubkey(),
                &cohort.period,
                &cohort.month,
                RecomputeOracleBucketMedianV1Params {
                    bucket_id: bucket_id(index),
                    mode: 0,
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
        let median: OracleBucketMedianState = h
            .read_state(
                derive_oracle_bucket_median_pda(&program(), &cohort.month, &bucket_id(index)).0,
            )
            .await;
        assert_eq!(median.status, OracleBucketMedianStatus::SettlementReady);
        assert_eq!(median.eligible_source_count, 1);
    }
    let buckets: Vec<[u8; 32]> = (0..SOURCES.len()).map(bucket_id).collect();
    h.send(
        instruction::finalize_oracle_month(
            &program(),
            &payer.pubkey(),
            &cohort.period,
            &cohort.month,
            &buckets,
        ),
        &[],
    )
    .await
    .unwrap();
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.settlement_status, OracleSettlementStatus::Final);
    // 0.4 × 1000 + 0.3 × (−500) + 0.2 × 250 + 0.1 × 0 = 300 bps.
    assert_eq!(month.index_delta_bps, 300);
    h.send(
        instruction::accumulate_oracle_settlement_source_bucket(
            &program(),
            &payer.pubkey(),
            &cohort.period,
            &cohort.month,
            AccumulateOracleSettlementSourceBucketParams {
                bucket_id: buckets[0],
                finalize_collection: true,
            },
            &buckets,
        ),
        &[],
    )
    .await
    .unwrap();

    // 116 with three of five attestations settles the period at 103.000000.
    let leaf = h.leaf_for(&cohort).await;
    assert_eq!(leaf.settlement_price_atomic, 103_000_000);
    let signers: Vec<Keypair> = h
        .settlement_signers
        .iter()
        .map(|key| key.insecure_clone())
        .collect();
    // Three attestations and the publication exceed one packet as a legacy transaction; as a v0
    // transaction with 116's eleven non-signer accounts in a lookup table they fit.
    let attesters = [&signers[0], &signers[1], &signers[2]];
    let publication = h.publication(&cohort, &attesters, &leaf, &leaf);
    let (legacy, versioned) = publication_sizes(&publication, &h.oracle_authority.pubkey());
    println!("publication: legacy {legacy} bytes, v0 with a lookup table {versioned} bytes");
    assert!(legacy > 1_232, "legacy {legacy}");
    assert!(versioned <= 1_232, "v0 {versioned}");
    h.publish(&cohort, &attesters, &leaf, &leaf).await.unwrap();
    let settled = h.period(cohort.product, cohort.expiry).await;
    assert_eq!(settled.status, PeriodStatus::Settled);
    let settlement = settled.settlement().unwrap();
    assert_eq!(settlement.value, 103_000_000);
    assert_eq!(settlement.digest, record_commitment(&leaf));
}

/// Serialized sizes of the publication signed by its publisher alone: as a legacy transaction,
/// and as a v0 transaction whose lookup table holds 116's eleven non-signer accounts.
fn publication_sizes(
    ixs: &[solana_sdk::instruction::Instruction],
    publisher: &Pubkey,
) -> (usize, usize) {
    use solana_sdk::{
        address_lookup_table::AddressLookupTableAccount,
        hash::Hash,
        message::{v0, Message, VersionedMessage},
    };
    let signed = |message: Vec<u8>| 1 + 64 + message.len();
    let legacy = signed(Message::new(ixs, Some(publisher)).serialize());
    let upsert = ixs.last().unwrap();
    let table = AddressLookupTableAccount {
        key: Pubkey::new_unique(),
        addresses: upsert.accounts[1..]
            .iter()
            .map(|meta| meta.pubkey)
            .collect(),
    };
    let message = v0::Message::try_compile(publisher, ixs, &[table], Hash::default()).unwrap();
    (legacy, signed(VersionedMessage::V0(message).serialize()))
}

#[tokio::test]
async fn completion_refuses_ordinary_months_wrong_counts_and_out_of_order_buckets() {
    let mut h = start().await;
    let cohort = h.bootstrapped_month().await;
    let oracle = h.oracle_authority.insecure_clone();
    let payer = h.payer();
    let begin = |sources: u16, buckets: u16| {
        instruction::dev_begin_formation(
            &program(),
            &oracle.pubkey(),
            &payer.pubkey(),
            &cohort.period,
            &cohort.month,
            sources,
            buckets,
        )
    };
    // The counts must be the month's sources and its required SKUs.
    assert_oracle_error(
        h.send(begin(3, 3), &[&oracle]).await,
        OracleError::InvalidOracleWeightManifest,
    );
    assert_oracle_error(
        h.send(begin(4, 3), &[&oracle]).await,
        OracleError::OracleSkuCoverageIncomplete,
    );
    // Only the oracle authority.
    let stranger = Keypair::new();
    h.fund(&stranger.pubkey(), 1_000_000_000).await;
    let mut foreign = begin(4, 4);
    foreign.accounts[0].pubkey = stranger.pubkey();
    assert!(h.send(foreign, &[&stranger]).await.is_err());
    h.send(begin(4, 4), &[&oracle]).await.unwrap();

    // Buckets go in ascending order with the weight their sources carry.
    let step = |index: usize, weight: u16| {
        instruction::dev_formation_bucket(
            &program(),
            &oracle.pubkey(),
            &payer.pubkey(),
            &cohort.period,
            &cohort.month,
            DevFormationBucketParams {
                bucket_weight_bps: weight,
                ..bonds(bucket_id(index))
            },
            &[cohort.source(&source_id(index))],
        )
    };
    h.send(step(1, SOURCES[1].0), &[&oracle]).await.unwrap();
    h.advance_slot().await;
    assert_oracle_error(
        h.send(step(0, SOURCES[0].0), &[&oracle]).await,
        OracleError::InvalidOracleWeightOrder,
    );
    assert_oracle_error(
        h.send(step(2, SOURCES[2].0 + 1), &[&oracle]).await,
        OracleError::InvalidOracleWeightSource,
    );
    // Finalize needs every bucket.
    assert_oracle_error(
        h.send(
            instruction::dev_finalize_formation(
                &program(),
                &oracle.pubkey(),
                &cohort.period,
                &cohort.month,
            ),
            &[&oracle],
        )
        .await,
        OracleError::OracleWeightManifestIncomplete,
    );
    // Tag 241 cannot add a source while the recipe builds.
    let buckets: Vec<[u8; 32]> = (0..SOURCES.len()).map(bucket_id).collect();
    let extra = [0xee; 32];
    h.upload_evidence(&payer, 1, &locator(&extra)).await;
    assert!(h
        .send(
            instruction::dev_bootstrap_source(
                &program(),
                &oracle.pubkey(),
                &payer.pubkey(),
                &payer.pubkey(),
                &cohort.period,
                &cohort.month,
                DevBootstrapSourceParams {
                    source_id: extra,
                    bucket_id: buckets[3],
                    source_type_hash: SOURCE_TYPE_HASH,
                    canonical_locator_hash: locator_hash(&locator(&extra)),
                    source_definition_hash: definition_hash(&definition(&extra)),
                    bucket_weight_bps: 1,
                    opening_state: 1,
                    sku_index: 3,
                    sku_proof: sku_merkle_proof(&buckets, 3),
                },
            ),
            &[&oracle],
        )
        .await
        .is_err());

    // An ordinary month (tag 181) is not completed by tag 244.
    let mut h = start().await;
    let ordinary = h.open_cohort(1).await;
    let oracle = h.oracle_authority.insecure_clone();
    let payer = h.payer();
    assert_oracle_error(
        h.send(
            instruction::dev_begin_formation(
                &program(),
                &oracle.pubkey(),
                &payer.pubkey(),
                &ordinary.period,
                &ordinary.month,
                1,
                1,
            ),
            &[&oracle],
        )
        .await,
        OracleError::InvalidOraclePhase,
    );
}
