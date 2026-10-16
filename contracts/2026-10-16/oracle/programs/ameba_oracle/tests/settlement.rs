//! Settlement on real instructions: a three-source bucket forms, lists and settles through the
//! post-expiry median walk (197), month finalization (75), the settlement-source manifest (113)
//! and the attested publication (116) that writes the period Settled; then the month closes (59).
//! And the failure crank (79): every condition under which a Forming period can no longer list.
mod common;

use ameba_oracle::{
    constants::{
        ORACLE_KILL_WINDOW_SECONDS, ORACLE_OPENING_WINDOW_SECONDS,
        ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS, ORACLE_SETTLEMENT_GRACE_SECONDS,
    },
    error::OracleError,
    instruction,
    state::{
        derive_settlement_v2_pda, OracleMonthState, OraclePhase, OracleSettlementStatus,
        SettlementRecordV2,
    },
};
use ameba_oracle_interface::{find_period_address, Period, PeriodStatus};
use common::{settle::*, *};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

#[tokio::test]
async fn attested_publication_settles_the_period_once_and_the_month_closes() {
    let mut h = start().await;
    let wide = wide_live(&mut h).await;
    let cohort = &wide.cohort;
    let payer = h.payer();

    // Tag 75 waits for expiry plus grace.
    let finalize = instruction::finalize_oracle_month(
        &program(),
        &payer.pubkey(),
        &cohort.period,
        &cohort.month,
        &[cohort.sku_ids[0]],
    );
    h.at(cohort.expiry + ORACLE_SETTLEMENT_GRACE_SECONDS - 1)
        .await;
    assert!(h.send(finalize.clone(), &[]).await.is_err());
    h.at(cohort.expiry + ORACLE_SETTLEMENT_GRACE_SECONDS).await;
    h.finalize_wide(&wide).await;
    let finalized: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(finalized.settlement_status, OracleSettlementStatus::Final);
    assert_eq!(finalized.phase, OraclePhase::Game);
    h.advance_slot().await;
    assert_oracle_error(h.send(finalize, &[]).await, OracleError::InvalidOracleState);
    // The period is still Live: only the attested publication settles it.
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Live
    );

    let leaf = h.leaf_for(cohort).await;
    let signers: Vec<Keypair> = h
        .settlement_signers
        .iter()
        .map(|key| key.insecure_clone())
        .collect();
    let (a, b, c) = (&signers[0], &signers[1], &signers[2]);
    let stranger = Keypair::new();

    // Fewer than the threshold, a repeated signer, a signer outside the set, a different message.
    assert_oracle_error(
        h.publish(cohort, &[a, b], &leaf, &leaf).await,
        OracleError::MissingSettlementOracleSignatures,
    );
    h.advance_slot().await;
    assert_oracle_error(
        h.publish(cohort, &[a, a, b], &leaf, &leaf).await,
        OracleError::DuplicateSettlementOracleSigner,
    );
    h.advance_slot().await;
    assert_oracle_error(
        h.publish(cohort, &[a, b, &stranger], &leaf, &leaf).await,
        OracleError::InvalidSettlementOracleSigner,
    );
    let mut other = leaf.clone();
    other.source_uri = "https://oracle.example/ramx/other".into();
    h.advance_slot().await;
    assert_oracle_error(
        h.publish(cohort, &[a, b, c], &other, &leaf).await,
        OracleError::SettlementOraclePayloadMismatch,
    );
    // A properly attested leaf whose value is not the month's.
    let mut inflated = leaf.clone();
    inflated.settlement_price_atomic += 1;
    h.advance_slot().await;
    assert_oracle_error(
        h.publish(cohort, &[a, b, c], &inflated, &inflated).await,
        OracleError::SettlementRecordHashMismatch,
    );
    // A negative index factor is invalid even with genuine attestations.
    let mut zero_month = finalized.clone();
    zero_month.index_delta_bps = -10_001;
    h.set_program_state(cohort.month, &zero_month, OracleMonthState::LEN)
        .await;
    let mut zero = leaf.clone();
    zero.index_delta_bps = -10_001;
    zero.settlement_price_atomic = 0;
    h.advance_slot().await;
    assert_oracle_error(
        h.publish(cohort, &[a, b, c], &zero, &zero).await,
        OracleError::InvalidSettlementRecord,
    );
    h.set_program_state(cohort.month, &finalized, OracleMonthState::LEN)
        .await;
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Live
    );

    // Three of five current signers: the month and the period settle.
    let settled_at = cohort.expiry + ORACLE_SETTLEMENT_GRACE_SECONDS + 60;
    h.at(settled_at).await;
    h.publish(cohort, &[a, c, &signers[4]], &leaf, &leaf)
        .await
        .unwrap();
    let digest = record_commitment(&leaf);
    let period = h.period(cohort.product, cohort.expiry).await;
    let settlement = period.settlement().expect("Settled");
    assert_eq!(settlement.value, leaf.settlement_price_atomic);
    assert_eq!(settlement.decimals, 6);
    assert_eq!(settlement.settled_ts, settled_at);
    assert_eq!(settlement.digest, digest);
    assert!(!period.is_tradable_at(settled_at));
    // The bytes the oracle wrote decode through the consumer contract unchanged.
    let raw = h.account(cohort.period).await.unwrap();
    assert_eq!(raw.data.len(), Period::LEN);
    assert_eq!(Period::decode(&raw.data).unwrap(), period);
    assert_eq!(period.encode().to_vec(), raw.data);

    let record_key = derive_settlement_v2_pda(&program(), &cohort.period, &cohort.month).0;
    let record: SettlementRecordV2 = h.read_state(record_key).await;
    assert_eq!(record.period, cohort.period);
    assert_eq!(record.oracle_month, cohort.month);
    assert_eq!(record.settlement_ts, cohort.expiry);
    assert_eq!(record.settlement_price_atomic, leaf.settlement_price_atomic);
    assert_eq!(record.signed_leaf_commitment, digest);
    assert_eq!(record.submitted_by, h.oracle_authority.pubkey());
    assert_eq!(record.signer_set_version, 1);
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.phase, OraclePhase::Settled);
    assert_eq!(month.settlement_record, Some(record_key));

    // A replay of the same publication changes nothing; a different one is refused.
    let before = (
        h.account(cohort.period).await.unwrap().data,
        h.account(cohort.month).await.unwrap().data,
        h.account(record_key).await.unwrap().data,
    );
    h.at(settled_at + 3_600).await;
    h.publish(cohort, &[a, b, c], &leaf, &leaf).await.unwrap();
    assert_eq!(
        before,
        (
            h.account(cohort.period).await.unwrap().data,
            h.account(cohort.month).await.unwrap().data,
            h.account(record_key).await.unwrap().data,
        )
    );
    h.advance_slot().await;
    assert_oracle_error(
        h.publish(cohort, &[a, b, c], &other, &other).await,
        OracleError::SettlementRecordImmutable,
    );

    // Tag 59 closes the Settled month once.
    let oracle = h.oracle_authority.insecure_clone();
    let close = instruction::close_oracle_month(
        &program(),
        &oracle.pubkey(),
        &cohort.period,
        &cohort.month,
    );
    let stranger_close = instruction::close_oracle_month(
        &program(),
        &stranger.pubkey(),
        &cohort.period,
        &cohort.month,
    );
    assert_oracle_error(
        h.send(stranger_close, &[&stranger]).await,
        OracleError::Unauthorized,
    );
    h.send(close.clone(), &[&oracle]).await.unwrap();
    let closed: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(closed.phase, OraclePhase::Closed);
    h.advance_slot().await;
    assert_oracle_error(
        h.send(close, &[&oracle]).await,
        OracleError::InvalidOraclePhase,
    );
    // The period stays Settled.
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Settled
    );
}

fn review_seconds() -> u64 {
    ORACLE_KILL_WINDOW_SECONDS
        + ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
        + ORACLE_OPENING_WINDOW_SECONDS
}

#[tokio::test]
async fn a_period_without_a_month_fails_once_no_month_can_fit_before_expiry() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    let oracle = h.oracle_authority.insecure_clone();
    let product_id = product("RAMX");
    h.create_period(&oracle, product_id, PLANNED, EXPIRY, 6)
        .await
        .unwrap();
    let payer = h.payer();
    let fail = instruction::fail_period(&program(), &payer.pubkey(), &product_id, EXPIRY);
    h.at(EXPIRY - review_seconds() - 1).await;
    assert_oracle_error(
        h.send(fail.clone(), &[]).await,
        OracleError::PeriodNotFailable,
    );
    h.at(EXPIRY - review_seconds()).await;
    h.send(fail.clone(), &[]).await.unwrap();
    let failed = h.period(product_id, EXPIRY).await;
    assert_eq!(failed.status, PeriodStatus::Failed);
    assert_eq!(failed.effective_listing_ts, 0);
    assert_eq!(failed.settlement(), None);
    assert!(!failed.is_tradable_at(PLANNED));
    let raw = h
        .account(find_period_address(&program(), &product_id, EXPIRY).0)
        .await
        .unwrap();
    assert_eq!(Period::decode(&raw.data).unwrap(), failed);
    // Exactly once.
    h.advance_slot().await;
    assert_oracle_error(
        h.send(fail.clone(), &[]).await,
        OracleError::InvalidPeriodTransition,
    );

    // The month slot must be the period's canonical month PDA.
    h.create_period(&oracle, product("NANDX"), PLANNED, EXPIRY, 6)
        .await
        .unwrap();
    let mut wrong =
        instruction::fail_period(&program(), &payer.pubkey(), &product("NANDX"), EXPIRY);
    wrong.accounts[2].pubkey = Pubkey::new_unique();
    assert_oracle_error(
        h.send(wrong, &[]).await,
        OracleError::InvalidOracleMonthAccount,
    );
}

#[tokio::test]
async fn a_month_still_in_source_submission_fails_its_period_at_the_submission_deadline() {
    let mut h = start().await;
    let cohort = h.open_cohort(1).await;
    let payer = h.payer();
    let fail =
        instruction::fail_period(&program(), &payer.pubkey(), &cohort.product, cohort.expiry);
    // Not while submission can still complete with the full review calendar.
    h.at(cohort.scramble).await;
    assert_oracle_error(
        h.send(fail.clone(), &[]).await,
        OracleError::PeriodNotFailable,
    );
    h.at(cohort.expiry - review_seconds() - 1).await;
    assert_oracle_error(
        h.send(fail.clone(), &[]).await,
        OracleError::PeriodNotFailable,
    );
    h.at(cohort.expiry - review_seconds()).await;
    h.send(fail, &[]).await.unwrap();
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Failed
    );
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.phase, OraclePhase::SourceSubmission);
}

#[tokio::test]
async fn a_month_that_never_reached_game_fails_its_period_at_expiry_and_a_live_one_never_does() {
    let mut h = start().await;
    let cohort = h.open_cohort(1).await;
    h.form_to_opening(&cohort).await;
    let payer = h.payer();
    let fail =
        instruction::fail_period(&program(), &payer.pubkey(), &cohort.product, cohort.expiry);
    // Opening, past the listing but before expiry: tag 80 can still list it.
    h.at(cohort.listing + 1).await;
    assert_oracle_error(
        h.send(fail.clone(), &[]).await,
        OracleError::PeriodNotFailable,
    );
    h.at(cohort.expiry - 1).await;
    assert_oracle_error(
        h.send(fail.clone(), &[]).await,
        OracleError::PeriodNotFailable,
    );
    h.at(cohort.expiry).await;
    h.send(fail, &[]).await.unwrap();
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Failed
    );

    // A Live period is never failed.
    let mut h = start().await;
    let cohort = h.open_cohort(1).await;
    h.form_to_opening(&cohort).await;
    h.at(cohort.listing).await;
    let payer = h.payer();
    h.send(
        instruction::finalize_oracle_opening_phase(
            &program(),
            &payer.pubkey(),
            &cohort.period,
            &cohort.month,
        ),
        &[],
    )
    .await
    .unwrap();
    h.at(cohort.expiry + 1).await;
    assert_oracle_error(
        h.send(
            instruction::fail_period(&program(), &payer.pubkey(), &cohort.product, cohort.expiry),
            &[],
        )
        .await,
        OracleError::InvalidPeriodTransition,
    );
}

#[tokio::test]
async fn close_rejects_a_month_with_incomplete_openings() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    h.activate().await;
    let oracle = h.oracle_authority.insecure_clone();
    let product_id = product("RAMX");
    h.create_period(&oracle, product_id, PLANNED, EXPIRY, 6)
        .await
        .unwrap();
    let period = find_period_address(&program(), &product_id, EXPIRY).0;
    let (month_key, month_bump) = Pubkey::find_program_address(
        &[
            ameba_oracle::constants::CURRENT_STATE_NAMESPACE_SEED,
            ameba_oracle::constants::ORACLE_MONTH_PDA_SEED,
            period.as_ref(),
            &EXPIRY.to_le_bytes(),
        ],
        &program(),
    );
    let (record_key, record_bump) = derive_settlement_v2_pda(&program(), &period, &month_key);
    let month = OracleMonthState {
        is_initialized: true,
        bump: month_bump,
        account_discriminator: OracleMonthState::ACCOUNT_DISCRIMINATOR,
        account_version: OracleMonthState::ACCOUNT_VERSION,
        period,
        authority: oracle.pubkey(),
        scramble_start_ts: PLANNED - PROGRAM_PRE_LISTING_SECONDS,
        listing_ts: PLANNED,
        phase: OraclePhase::Game,
        source_count: 1,
        frozen_source_count: 1,
        opened_source_count: 0,
        opening_resolved_source_count: 0,
        recipe_hash: [84; 32],
        settlement_base_oracle_atomic: 100,
        index_delta_bps: 1_000,
        settlement_status: OracleSettlementStatus::Final,
        weight_scheme_version: 1,
        effective_weight_total_bps: 10_000,
        weight_manifest_hash: [85; 32],
        ..OracleMonthState::default()
    };
    h.set_program_state(month_key, &month, OracleMonthState::LEN)
        .await;
    h.set_program_state(
        record_key,
        &SettlementRecordV2 {
            is_initialized: true,
            bump: record_bump,
            account_discriminator: SettlementRecordV2::ACCOUNT_DISCRIMINATOR,
            account_version: SettlementRecordV2::ACCOUNT_VERSION,
            period,
            oracle_month: month_key,
            settlement_ts: EXPIRY,
            settlement_price_atomic: 110,
            signed_leaf_commitment: [85; 32],
            submitted_by: oracle.pubkey(),
            signer_set_version: 1,
        },
        SettlementRecordV2::LEN,
    )
    .await;
    assert_oracle_error(
        h.send(
            instruction::close_oracle_month(&program(), &oracle.pubkey(), &period, &month_key),
            &[&oracle],
        )
        .await,
        OracleError::InvalidOracleState,
    );
    let after: OracleMonthState = h.read_state(month_key).await;
    assert_eq!(after.phase, OraclePhase::Game);
    assert_eq!(after.settlement_record, None);
}

#[tokio::test]
async fn a_source_that_never_opened_is_walked_with_its_empty_observations() {
    use ameba_oracle::{
        instruction::RecomputeOracleBucketMedianV1Params,
        state::{
            derive_oracle_bucket_median_pda, OracleBucketMedianState, OracleBucketMedianStatus,
        },
    };
    let mut h = start().await;
    let wide = wide_placed(&mut h, None).await;
    h.wide_coverage(&wide).await;
    h.wide_recipe(&wide).await;
    h.wide_openings_of(&wide, &[0, 1]).await;
    let cohort = wide.cohort.clone();
    h.at(cohort.listing).await;
    let payer = h.payer();
    let unopened = wide.sources[2];
    h.send(
        instruction::expire_oracle_opening_source(
            &program(),
            &payer.pubkey(),
            &cohort.period,
            &cohort.month,
            &unopened,
        ),
        &[],
    )
    .await
    .unwrap();
    h.wide_active_weights(&wide).await;
    h.wide_list(&wide).await;
    // The unopened source keeps the empty observations account its proposal created.
    let observations = instruction::find_oracle_source_observations_address(&program(), &unopened);
    assert_eq!(h.account(observations).await.unwrap().owner, program());

    h.at(cohort.expiry + ORACLE_SETTLEMENT_GRACE_SECONDS).await;
    let bucket_id = cohort.sku_ids[0];
    for source in &wide.sources {
        h.send(
            instruction::recompute_oracle_bucket_median_v1(
                &program(),
                &payer.pubkey(),
                &cohort.period,
                &cohort.month,
                RecomputeOracleBucketMedianV1Params { bucket_id, mode: 0 },
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
    // Two of three frozen sources are eligible: below the three-source floor, so grace is next.
    let bucket: OracleBucketMedianState = h
        .read_state(derive_oracle_bucket_median_pda(&program(), &cohort.month, &bucket_id).0)
        .await;
    assert_eq!(bucket.status, OracleBucketMedianStatus::GraceRequired);
    assert_eq!(
        (bucket.frozen_source_count, bucket.active_source_count),
        (3, 2)
    );
}

#[tokio::test]
async fn tiny_positive_base_can_publish_a_rounded_zero_settlement() {
    let mut h = start().await;
    let wide = wide_live(&mut h).await;
    let cohort = &wide.cohort;
    h.at(cohort.expiry + ORACLE_SETTLEMENT_GRACE_SECONDS).await;
    h.finalize_wide(&wide).await;
    // Exercise attested publication against an otherwise finalized month whose valid
    // positive index base rounds below one atomic unit; no minimum base is introduced.
    let mut month: OracleMonthState = h.read_state(cohort.month).await;
    month.settlement_base_oracle_atomic = 1;
    month.index_delta_bps = -9_999;
    h.set_program_state(cohort.month, &month, OracleMonthState::LEN)
        .await;
    let leaf = h.leaf_for(cohort).await;
    assert_eq!(leaf.settlement_price_atomic, 0);
    let signers: Vec<Keypair> = h
        .settlement_signers
        .iter()
        .map(Keypair::insecure_clone)
        .collect();
    h.publish(
        cohort,
        &[&signers[0], &signers[1], &signers[2]],
        &leaf,
        &leaf,
    )
    .await
    .unwrap();
    let period = h.period(cohort.product, cohort.expiry).await;
    assert_eq!(period.status, PeriodStatus::Settled);
    assert_eq!(period.settlement().unwrap().value, 0);
    assert_eq!(period.settlement_digest, record_commitment(&leaf));
    assert!(!period.is_tradable_at(cohort.expiry));
    let bytes = h.account(cohort.period).await.unwrap().data;
    assert_eq!(bytes[8], Period::VERSION);
    assert_eq!(Period::decode(&bytes).unwrap(), period);
    let record: SettlementRecordV2 = h
        .read_state(derive_settlement_v2_pda(&program(), &cohort.period, &cohort.month).0)
        .await;
    assert_eq!(record.settlement_price_atomic, 0);
    assert_eq!(record.signed_leaf_commitment, period.settlement_digest);
}
