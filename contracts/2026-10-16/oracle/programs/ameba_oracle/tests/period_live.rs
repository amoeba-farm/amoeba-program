//! Openings, active weights and tag 80: the month enters Game and its period becomes Live at the
//! month's listing.
mod common;

use ameba_oracle::{
    error::OracleError,
    instruction::{self, ChallengeOracleOpeningClaimParams},
    state::{
        derive_oracle_active_weight_manifest_pda, derive_oracle_bucket_median_pda,
        derive_user_collateral_pda, OracleActiveWeightManifest, OracleBucketMedianState,
        OracleBucketMedianStatus, OracleChallengeStatus, OracleEscrowDisposition, OracleEscrowKind,
        OracleMonthState, OracleOpeningChallengeOutcome, OracleOpeningClaim,
        OracleOpeningClaimStatus, OraclePhase, OracleRecipeWeightPhase, OracleSourceState,
        OracleSourceStatus,
    },
};
use ameba_oracle_interface::{find_period_address, Period, PeriodStatus};
use common::{cohort::*, *};
use solana_sdk::{instruction::AccountMeta, signature::Signer};

fn go_live(h: &Harness, cohort: &Cohort) -> solana_sdk::instruction::Instruction {
    instruction::finalize_oracle_opening_phase(
        &program(),
        &h.ctx.payer.pubkey(),
        &cohort.period,
        &cohort.month,
    )
}

async fn month(h: &mut Harness, cohort: &Cohort) -> OracleMonthState {
    h.read_state(cohort.month).await
}

/// Overwrite the cohort's period with an explicit, layout-valid state.
async fn install_period(h: &mut Harness, period: Period) {
    let (key, bump) = find_period_address(&program(), &period.product_id, period.expiry_ts);
    let period = Period { bump, ..period };
    period.validate().unwrap();
    let rent = h.ctx.banks_client.get_rent().await.unwrap();
    let mut account = solana_sdk::account::AccountSharedData::new(
        rent.minimum_balance(Period::LEN),
        Period::LEN,
        &program(),
    );
    account.set_data_from_slice(&period.encode());
    h.set_account_preserving_capitalization(key, account).await;
}

#[tokio::test]
async fn formation_freezes_the_recipe_opens_every_source_and_builds_active_weights() {
    let mut h = start().await;
    let cohort = h.open_cohort(2).await;
    let formed = h.form_to_opening(&cohort).await;
    let opened = month(&mut h, &cohort).await;
    assert_eq!(opened.phase, OraclePhase::Opening);
    assert_eq!(opened.weight_scheme_version, 1);
    assert_eq!(opened.effective_weight_total_bps, 10_000);
    assert_eq!(opened.frozen_source_count, 2);
    assert_eq!(opened.opened_source_count, 2);
    assert_eq!(opened.opening_resolved_source_count, 2);
    assert_eq!(opened.pending_resolution_count, 0);
    assert_eq!(
        opened.active_weight_scheme_version,
        OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION
    );
    assert_eq!(opened.active_weight_group_count, 2);
    assert_ne!(opened.active_weight_manifest_hash, [0; 32]);
    assert_eq!(
        opened.weight_manifest_hash,
        *cohort.recipe_chain(&formed).last().unwrap()
    );

    for (index, (source, _)) in formed.sources.iter().enumerate() {
        let state: OracleSourceState = h.read_state(*source).await;
        assert_eq!(state.status, OracleSourceStatus::Active);
        assert_eq!(state.baseline_state, OPENING_STATE + index as u64);
        assert_eq!(state.observation_count, 1);
        assert_eq!(state.bucket_weight_bps, cohort.bucket_weight(index));
        // The first checkpoint and its journal exist.
        let claim =
            instruction::find_oracle_opening_claim_address(&program(), &cohort.month, source);
        assert!(h
            .account(instruction::find_checkpoint_address(
                &program(),
                source,
                &claim
            ))
            .await
            .is_some());
        assert!(h
            .account(instruction::find_source_journal_address(&program(), source))
            .await
            .is_some());
        let bucket: OracleBucketMedianState = h
            .read_state(
                derive_oracle_bucket_median_pda(&program(), &cohort.month, &cohort.sku_ids[index])
                    .0,
            )
            .await;
        assert_eq!(bucket.status, OracleBucketMedianStatus::Live);
        assert_eq!(bucket.active_source_count, 1);
        assert_eq!(bucket.bucket_delta_bps, 0);
    }
    let manifest: OracleActiveWeightManifest = h
        .read_state(derive_oracle_active_weight_manifest_pda(&program(), &cohort.month).0)
        .await;
    assert_eq!(manifest.phase, OracleRecipeWeightPhase::Finalized);
    assert_eq!(manifest.processed_bucket_weight_bps, 10_000);
    // Opening bonds are refundable once accepted.
    let period = h.period(cohort.product, cohort.expiry).await;
    assert_eq!(period.status, PeriodStatus::Forming);
}

#[tokio::test]
async fn period_goes_live_exactly_at_the_month_listing() {
    let mut h = start().await;
    let cohort = h.open_cohort(2).await;
    h.form_to_opening(&cohort).await;

    // One second early: the opening window is still open.
    h.at(cohort.listing - 1).await;
    assert_oracle_error(
        h.send(go_live(&h, &cohort), &[]).await,
        OracleError::OracleTimingWindowClosed,
    );
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Forming
    );

    h.at(cohort.listing).await;
    h.send(go_live(&h, &cohort), &[]).await.unwrap();
    let period = h.period(cohort.product, cohort.expiry).await;
    assert_eq!(period.status, PeriodStatus::Live);
    assert_eq!(period.effective_listing_ts, cohort.listing);
    assert_eq!(period.planned_listing_ts, cohort.listing);
    assert_eq!(period.settlement(), None);
    assert!(!period.is_tradable_at(cohort.listing - 1));
    assert!(period.is_tradable_at(cohort.listing));
    assert!(period.is_tradable_at(cohort.expiry - 1));
    assert!(!period.is_tradable_at(cohort.expiry));
    assert_eq!(month(&mut h, &cohort).await.phase, OraclePhase::Game);

    // Exactly once.
    h.advance_slot().await;
    assert_oracle_error(
        h.send(go_live(&h, &cohort), &[]).await,
        OracleError::InvalidOraclePhase,
    );
}

#[tokio::test]
async fn period_listed_after_the_listing_starts_at_the_month_listing() {
    let mut h = start().await;
    let cohort = h.open_cohort(1).await;
    h.form_to_opening(&cohort).await;
    h.at(cohort.listing + 1).await;
    h.send(go_live(&h, &cohort), &[]).await.unwrap();
    let period = h.period(cohort.product, cohort.expiry).await;
    assert_eq!(period.status, PeriodStatus::Live);
    assert_eq!(period.effective_listing_ts, cohort.listing);
}

#[tokio::test]
async fn late_coverage_lists_the_period_at_the_shifted_listing() {
    let mut h = start().await;
    let planned = h.open_cohort(2).await;
    h.fund_schedule(&planned).await;
    h.at(planned.scramble).await;
    let proposer = h.participant(PARTICIPANT_USDC).await;
    let supporter = h.participant(PARTICIPANT_USDC).await;
    let mut sources = Vec::new();
    for index in 0..2u16 {
        if index == 1 {
            // The second SKU is covered only after the planned placement end.
            h.at(planned.placement_end() + 1_234).await;
        }
        let source_id = formed_source_id(index);
        let source = h
            .propose_source(&planned, &proposer, source_id, index)
            .await
            .unwrap();
        h.support_source(&planned, &supporter, &source, index)
            .await
            .unwrap();
        sources.push((source, source_id));
    }
    h.finalize_coverage(&planned).await;
    let shifted_month = month(&mut h, &planned).await;
    assert!(shifted_month.listing_ts > planned.listing);
    let cohort = Cohort {
        scramble: shifted_month.scramble_start_ts,
        listing: shifted_month.listing_ts,
        product: planned.product,
        sku_ids: planned.sku_ids.clone(),
        root: planned.root,
        period: planned.period,
        month: planned.month,
        expiry: planned.expiry,
    };
    let formed = Formed {
        proposer,
        supporter,
        sources,
    };
    h.at(cohort.kill_end()).await;
    h.freeze_recipe(&cohort, &formed).await;
    h.index_recipe(&cohort, &formed).await;
    h.at(cohort.scramble_end()).await;
    h.open_sources(&cohort, &formed, &[]).await;
    h.build_active_weights(&cohort, &formed).await;

    // The planned listing has passed but the shifted one has not.
    h.at(cohort.listing - 1).await;
    assert!(cohort.listing - 1 >= planned.listing);
    assert_oracle_error(
        h.send(go_live(&h, &cohort), &[]).await,
        OracleError::OracleTimingWindowClosed,
    );
    h.at(cohort.listing).await;
    h.send(go_live(&h, &cohort), &[]).await.unwrap();
    let period = h.period(cohort.product, cohort.expiry).await;
    assert_eq!(period.status, PeriodStatus::Live);
    assert_eq!(period.planned_listing_ts, planned.listing);
    assert_eq!(period.effective_listing_ts, cohort.listing);
    assert!(period.effective_listing_ts > period.planned_listing_ts);
}

#[tokio::test]
async fn live_transition_rejects_a_wrong_failed_or_read_only_period() {
    let mut h = start().await;
    let cohort = h.open_cohort(1).await;
    h.form_to_opening(&cohort).await;
    // Another Forming period of the same product on the next rung.
    let oracle = h.oracle_authority.insecure_clone();
    h.create_period(&oracle, cohort.product, JAN_01_2027, APR_01_2027, 6)
        .await
        .unwrap();
    h.at(cohort.listing).await;

    let other = find_period_address(&program(), &cohort.product, APR_01_2027).0;
    let mut wrong = go_live(&h, &cohort);
    wrong.accounts[1].pubkey = other;
    assert_oracle_error(
        h.send(wrong, &[]).await,
        OracleError::InvalidOracleMonthAccount,
    );

    let mut read_only = go_live(&h, &cohort);
    read_only.accounts[1] = AccountMeta::new_readonly(cohort.period, false);
    assert_oracle_error(
        h.send(read_only, &[]).await,
        OracleError::InvalidPeriodAccount,
    );

    let forming = h.period(cohort.product, cohort.expiry).await;
    install_period(
        &mut h,
        Period {
            status: PeriodStatus::Failed,
            ..forming.clone()
        },
    )
    .await;
    assert_oracle_error(
        h.send(go_live(&h, &cohort), &[]).await,
        OracleError::InvalidPeriodTransition,
    );
    assert_eq!(month(&mut h, &cohort).await.phase, OraclePhase::Opening);

    install_period(&mut h, forming).await;
    h.advance_slot().await;
    h.send(go_live(&h, &cohort), &[]).await.unwrap();
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Live
    );
}

#[tokio::test]
async fn unclaimed_source_expires_only_once_the_listing_arrives() {
    let mut h = start().await;
    let cohort = h.open_cohort(2).await;
    h.fund_schedule(&cohort).await;
    h.at(cohort.scramble).await;
    let formed = h.cover_skus(&cohort).await;
    h.at(cohort.placement_end()).await;
    h.finalize_coverage(&cohort).await;
    h.at(cohort.kill_end()).await;
    h.freeze_recipe(&cohort, &formed).await;
    h.index_recipe(&cohort, &formed).await;
    h.at(cohort.scramble_end()).await;
    h.open_sources(&cohort, &formed, &[1]).await;

    let unclaimed = formed.sources[1].0;
    let expire = instruction::expire_oracle_opening_source(
        &program(),
        &h.ctx.payer.pubkey(),
        &cohort.period,
        &cohort.month,
        &unclaimed,
    );
    assert_oracle_error(
        h.send(expire.clone(), &[]).await,
        OracleError::OracleTimingWindowClosed,
    );
    // Active weights cannot begin while a frozen source is unresolved.
    let oracle = h.oracle_authority.insecure_clone();
    assert!(h
        .send(
            instruction::begin_oracle_active_weights(
                &program(),
                &oracle.pubkey(),
                &cohort.period,
                &cohort.month,
                2,
                2,
            ),
            &[&oracle],
        )
        .await
        .is_err());
    h.at(cohort.listing).await;
    h.send(expire.clone(), &[]).await.unwrap();
    let source: OracleSourceState = h.read_state(unclaimed).await;
    assert_eq!(source.status, OracleSourceStatus::Inactive);
    let resolved = month(&mut h, &cohort).await;
    assert_eq!(resolved.opening_resolved_source_count, 2);
    assert_eq!(resolved.opened_source_count, 1);
    h.advance_slot().await;
    assert!(h.send(expire, &[]).await.is_err());
}

#[tokio::test]
async fn kept_opening_survives_a_challenge_and_its_escrow_settles_back_to_the_claim() {
    let mut h = start().await;
    let cohort = h.open_cohort(1).await;
    h.fund_schedule(&cohort).await;
    h.at(cohort.scramble).await;
    let formed = h.cover_skus(&cohort).await;
    h.at(cohort.placement_end()).await;
    h.finalize_coverage(&cohort).await;
    h.at(cohort.kill_end()).await;
    h.freeze_recipe(&cohort, &formed).await;
    h.index_recipe(&cohort, &formed).await;
    h.at(cohort.scramble_end()).await;

    let (source, source_id) = formed.sources[0];
    let claimant = h.participant(PARTICIPANT_USDC).await;
    let capture = cohort.scramble_end();
    let url = archive_url(&source_id, capture);
    h.upload_evidence(&claimant, 3, url.as_bytes()).await;
    h.send(
        instruction::submit_oracle_opening_claim_v2(
            &program(),
            &claimant.pubkey(),
            &claimant.pubkey(),
            &cohort.period,
            &cohort.month,
            &source,
            &cohort.sku_ids[0],
            ameba_oracle::instruction::SubmitOracleOpeningClaimParams {
                opening_state: OPENING_STATE,
                source_time: capture,
                stake: OPENING_BOND,
                canonical_locator_hash: locator_hash(&locator(&source_id)),
                source_definition_hash: definition_hash(&definition(&source_id)),
                archive_url: String::new(),
            },
            &url,
        ),
        &[&claimant],
    )
    .await
    .unwrap();
    assert_eq!(
        h.collateral(&claimant.pubkey()).await.available_balance,
        PARTICIPANT_USDC - OPENING_BOND
    );

    let challenger = h.participant(PARTICIPANT_USDC).await;
    h.upload_evidence(&challenger, 3, url.as_bytes()).await;
    let challenge_id = [0xa1; 32];
    let alternative = ChallengeOracleOpeningClaimParams {
        challenge_id,
        alternative_opening_state: OPENING_STATE + 7,
        alternative_source_time: capture,
        bond: CHALLENGE_MIN_BOND,
        canonical_locator_hash: locator_hash(&locator(&source_id)),
        source_definition_hash: definition_hash(&definition(&source_id)),
        archive_url: String::new(),
    };
    // Repeating the claim exactly is not a challenge.
    let noop = ChallengeOracleOpeningClaimParams {
        alternative_opening_state: OPENING_STATE,
        ..alternative.clone()
    };
    assert_oracle_error(
        h.send(
            instruction::challenge_oracle_opening_claim_v2(
                &program(),
                &challenger.pubkey(),
                &challenger.pubkey(),
                &cohort.period,
                &cohort.month,
                &source,
                &cohort.sku_ids[0],
                noop,
                &url,
            ),
            &[&challenger],
        )
        .await,
        OracleError::InvalidOracleChallengeAccount,
    );
    h.send(
        instruction::challenge_oracle_opening_claim_v2(
            &program(),
            &challenger.pubkey(),
            &challenger.pubkey(),
            &cohort.period,
            &cohort.month,
            &source,
            &cohort.sku_ids[0],
            alternative,
            &url,
        ),
        &[&challenger],
    )
    .await
    .unwrap();
    let claim_key =
        instruction::find_oracle_opening_claim_address(&program(), &cohort.month, &source);
    let challenge_key = instruction::find_oracle_opening_challenge_address(
        &program(),
        &cohort.month,
        &claim_key,
        &challenge_id,
    );
    assert_eq!(month(&mut h, &cohort).await.pending_resolution_count, 2);

    let oracle = h.oracle_authority.insecure_clone();
    h.send(
        instruction::resolve_oracle_opening_claim_challenge_v2(
            &program(),
            &oracle.pubkey(),
            &cohort.month,
            &source,
            &cohort.sku_ids[0],
            &claimant.pubkey(),
            &challenger.pubkey(),
            &challenge_key,
            OracleOpeningChallengeOutcome::KeepOpening,
        ),
        &[&oracle],
    )
    .await
    .unwrap();
    assert_eq!(month(&mut h, &cohort).await.pending_resolution_count, 1);

    // The rejected challenge's bond goes to the claimant and the claim reopens for acceptance.
    h.send(
        instruction::settle_oracle_usdc_escrow(
            &program(),
            OracleEscrowKind::OpeningChallenge,
            vec![
                AccountMeta::new_readonly(h.ctx.payer.pubkey(), true),
                AccountMeta::new_readonly(cohort.period, false),
                AccountMeta::new_readonly(cohort.month, false),
                AccountMeta::new(source, false),
                AccountMeta::new(claim_key, false),
                AccountMeta::new(challenge_key, false),
                AccountMeta::new(
                    derive_user_collateral_pda(&program(), &claimant.pubkey()).0,
                    false,
                ),
                AccountMeta::new(
                    derive_user_collateral_pda(&program(), &challenger.pubkey()).0,
                    false,
                ),
            ],
        ),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        h.collateral(&claimant.pubkey()).await.available_balance,
        PARTICIPANT_USDC - OPENING_BOND + CHALLENGE_MIN_BOND
    );
    let reopened: OracleOpeningClaim = h.read_state(claim_key).await;
    assert_eq!(reopened.status, OracleOpeningClaimStatus::Pending);
    let settled: ameba_oracle::state::OracleOpeningClaimChallenge =
        h.read_state(challenge_key).await;
    assert_eq!(settled.status, OracleChallengeStatus::Rejected);
    assert_eq!(settled.escrow_disposition, OracleEscrowDisposition::Slashed);

    // The claim is accepted after its fresh challenge window.
    h.advance_slots(ameba_oracle::constants::ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS + 1)
        .await;
    h.send(
        instruction::finalize_oracle_opening_claim_v2(
            &program(),
            &h.ctx.payer.pubkey(),
            &cohort.period,
            &cohort.month,
            &source,
            &claimant.pubkey(),
        ),
        &[],
    )
    .await
    .unwrap();
    let state: OracleSourceState = h.read_state(source).await;
    assert_eq!(state.status, OracleSourceStatus::Active);
    assert_eq!(state.baseline_state, OPENING_STATE);
    assert_eq!(month(&mut h, &cohort).await.pending_resolution_count, 0);

    // The accepted claim's bond is refunded.
    h.send(
        instruction::settle_oracle_usdc_escrow(
            &program(),
            OracleEscrowKind::OpeningClaim,
            vec![
                AccountMeta::new_readonly(h.ctx.payer.pubkey(), true),
                AccountMeta::new_readonly(cohort.period, false),
                AccountMeta::new_readonly(cohort.month, false),
                AccountMeta::new_readonly(source, false),
                AccountMeta::new(claim_key, false),
                AccountMeta::new(
                    derive_user_collateral_pda(&program(), &claimant.pubkey()).0,
                    false,
                ),
            ],
        ),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        h.collateral(&claimant.pubkey()).await.available_balance,
        PARTICIPANT_USDC + CHALLENGE_MIN_BOND
    );
}

#[tokio::test]
async fn thin_bucket_recompute_after_grace_requires_grace_then_council() {
    use ameba_oracle::constants::ORACLE_SETTLEMENT_GRACE_SECONDS;
    use ameba_oracle::instruction::RecomputeOracleBucketMedianV1Params;
    let mut h = start().await;
    let cohort = h.open_cohort(1).await;
    let formed = h.form_to_opening(&cohort).await;
    h.at(cohort.listing).await;
    h.send(go_live(&h, &cohort), &[]).await.unwrap();
    let (source, _) = formed.sources[0];
    let observations = instruction::find_oracle_source_observations_address(&program(), &source);
    let recompute = |h: &Harness, mode: u8| {
        instruction::recompute_oracle_bucket_median_v1(
            &program(),
            &h.ctx.payer.pubkey(),
            &cohort.period,
            &cohort.month,
            RecomputeOracleBucketMedianV1Params {
                bucket_id: cohort.sku_ids[0],
                mode,
            },
            &source,
            &observations,
            0,
            &solana_sdk::system_program::id(),
        )
    };
    // Not before expiry plus the settlement grace.
    h.at(cohort.expiry + ORACLE_SETTLEMENT_GRACE_SECONDS - 1)
        .await;
    assert_oracle_error(
        h.send(recompute(&h, 0), &[]).await,
        OracleError::SettlementGracePeriodActive,
    );
    h.at(cohort.expiry + ORACLE_SETTLEMENT_GRACE_SECONDS).await;
    // Grace mode needs a primary walk that fell short first.
    assert_oracle_error(
        h.send(recompute(&h, 1), &[]).await,
        OracleError::InvalidOracleMedian,
    );
    h.send(recompute(&h, 0), &[]).await.unwrap();
    let bucket_key =
        derive_oracle_bucket_median_pda(&program(), &cohort.month, &cohort.sku_ids[0]).0;
    let bucket: OracleBucketMedianState = h.read_state(bucket_key).await;
    assert_eq!(bucket.eligible_source_count, 1);
    // One eligible source is below the three-source floor, in every profile: the localnet
    // profiles relax it only for the dev months' sources (tag 241), never for a formed month.
    assert_eq!(bucket.status, OracleBucketMedianStatus::GraceRequired);
    h.advance_slot().await;
    h.send(recompute(&h, 1), &[]).await.unwrap();
    let bucket: OracleBucketMedianState = h.read_state(bucket_key).await;
    assert_eq!(bucket.status, OracleBucketMedianStatus::EmergencyRequired);
    assert_eq!(bucket.council_authority_version, 1);
    // The live index is unchanged by a bucket that could not settle.
    assert_eq!(month(&mut h, &cohort).await.index_delta_bps, 0);
}
