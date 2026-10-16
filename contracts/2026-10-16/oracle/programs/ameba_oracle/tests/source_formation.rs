//! Formation on ordinary accounts: reward schedule (154-158), proposals and support (182, 183),
//! coverage (184, 186), challenges and their resolution (161, 185), unsupported timeouts (196),
//! counted escrow settlement (169) and the failed-month lanes (191, 192, 194, 195).
mod common;

use ameba_oracle::{
    error::OracleError,
    instruction::{self, ChallengeOracleSourceParams, PrelistingEscrow},
    state::{
        derive_oracle_sku_coverage_manifest_pda, derive_oracle_sku_coverage_record_pda,
        derive_oracle_source_challenge_guard_pda, derive_oracle_usdc_reward_vault_pda,
        derive_oracle_usdc_source_reward_pda, OracleChallengeStatus, OracleEscrowDisposition,
        OracleMonthState, OraclePhase, OracleSkuCoverageManifest, OracleSkuCoverageRecord,
        OracleSourceChallenge, OracleSourceChallengeGuard, OracleSourceChallengeOutcome,
        OracleSourceState, OracleSourceStatus, OracleSupportPosition, OracleUsdcRewardSchedule,
        OracleUsdcRewardSchedulePhase, OracleUsdcRewardVault, OracleUsdcSourceReward,
    },
};
use common::{cohort::*, *};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

const NON_INDEPENDENT: u8 = 8;

fn source_id(n: u8) -> [u8; 32] {
    [n; 32]
}

async fn month(h: &mut Harness, cohort: &Cohort) -> OracleMonthState {
    h.read_state(cohort.month).await
}

async fn coverage(h: &mut Harness, cohort: &Cohort) -> OracleSkuCoverageManifest {
    h.read_state(derive_oracle_sku_coverage_manifest_pda(&program(), &cohort.month).0)
        .await
}

async fn schedule(h: &mut Harness, cohort: &Cohort) -> OracleUsdcRewardSchedule {
    h.read_state(cohort.schedule()).await
}

async fn assert_bond_vault_covers_ledgers(h: &mut Harness, owners: &[&Keypair]) {
    let mut total = 0;
    for owner in owners {
        let ledger = h.collateral(&owner.pubkey()).await;
        total += ledger.available_balance;
    }
    let vault = h.bond_vault();
    assert!(h.token_balance(vault).await >= total);
}

/// Every SKU gets one proposed and one supported source; returns (proposer, supporter, sources).
async fn cover_every_sku(h: &mut Harness, cohort: &Cohort) -> (Keypair, Keypair, Vec<Pubkey>) {
    let proposer = h.participant(PARTICIPANT_USDC).await;
    let supporter = h.participant(PARTICIPANT_USDC).await;
    let mut sources = Vec::new();
    for index in 0..cohort.sku_ids.len() as u16 {
        let source = h
            .propose_source(cohort, &proposer, source_id(10 + index as u8), index)
            .await
            .unwrap();
        h.support_source(cohort, &supporter, &source, index)
            .await
            .unwrap();
        sources.push(source);
    }
    (proposer, supporter, sources)
}

#[tokio::test]
async fn cohort_forms_at_exact_coverage_and_keeps_the_planned_calendar() {
    let mut h = start().await;
    let cohort = h.open_cohort(3).await;
    h.fund_schedule(&cohort).await;
    let frozen = schedule(&mut h, &cohort).await;
    assert_eq!(frozen.phase, OracleUsdcRewardSchedulePhase::Funded);
    assert_eq!(frozen.sku_pool_count, 3);
    assert_eq!(
        frozen.total_reward_budget,
        3 * (SOURCE_REWARD + OPENING_REWARD + UPDATE_REWARD)
    );
    let vault: OracleUsdcRewardVault = h
        .read_state(derive_oracle_usdc_reward_vault_pda(&program()).0)
        .await;
    assert_eq!(vault.total_reserved, frozen.total_reward_budget);

    // Submission opens at the planned scramble start, not before.
    let early = h.participant(PARTICIPANT_USDC).await;
    h.at(cohort.scramble - 1).await;
    assert_oracle_error(
        h.propose_source(&cohort, &early, source_id(1), 0)
            .await
            .map(|_| ()),
        OracleError::OracleSkuCoverageExtensionTooLate,
    );

    h.at(cohort.scramble).await;
    let (proposer, supporter, sources) = cover_every_sku(&mut h, &cohort).await;
    // An extra source nobody supports.
    let unsupported = h
        .propose_source(&cohort, &proposer, source_id(99), 0)
        .await
        .unwrap();

    let after = h.collateral(&proposer.pubkey()).await;
    assert_eq!(after.available_balance, PARTICIPANT_USDC - 4 * LISTING_BOND);
    let after = h.collateral(&supporter.pubkey()).await;
    assert_eq!(after.available_balance, PARTICIPANT_USDC - 3 * SUPPORT_BOND);
    assert_bond_vault_covers_ledgers(&mut h, &[&proposer, &supporter, &early]).await;

    let source: OracleSourceState = h.read_state(sources[0]).await;
    assert_eq!(source.status, OracleSourceStatus::Candidate);
    assert_eq!(source.listing_bond_locked, LISTING_BOND);
    assert_eq!(source.support_stake_total, SUPPORT_BOND);
    let record: OracleSkuCoverageRecord = h
        .read_state(
            derive_oracle_sku_coverage_record_pda(&program(), &cohort.month, &cohort.sku_ids[0]).0,
        )
        .await;
    assert_eq!(record.active_supported_source_count, 1);
    assert_eq!(record.sku_index, 0);
    let covered = coverage(&mut h, &cohort).await;
    assert_eq!(covered.covered_sku_count, 3);
    assert!(!covered.coverage_finalized);
    // Four listings and three supports are counted pre-listing escrows.
    assert_eq!(
        schedule(&mut h, &cohort)
            .await
            .outstanding_prelisting_escrow_count,
        7
    );

    // A proof for another index does not prove membership.
    let wrong = h.participant(PARTICIPANT_USDC).await;
    h.upload_evidence(&wrong, 1, &locator(&source_id(98))).await;
    h.upload_evidence(&wrong, 2, &definition(&source_id(98)))
        .await;
    let mut bad = h.propose_ix(&cohort, &wrong.pubkey(), source_id(98), 1);
    let good = h.propose_ix(&cohort, &wrong.pubkey(), source_id(98), 0);
    bad.data = good.data.clone();
    bad.data[1 + 32..1 + 64].copy_from_slice(&cohort.sku_ids[1]);
    assert!(h.send(bad, &[&wrong]).await.is_err());

    // Exact coverage at the planned placement end keeps the planned calendar.
    h.at(cohort.placement_end()).await;
    let cranker = Keypair::new();
    h.fund(&cranker.pubkey(), 1_000_000_000).await;
    h.send(
        instruction::finalize_oracle_sku_coverage(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
        ),
        &[&cranker],
    )
    .await
    .unwrap();
    let formed = month(&mut h, &cohort).await;
    assert_eq!(formed.phase, OraclePhase::Scramble);
    assert_eq!(formed.scramble_start_ts, cohort.scramble);
    assert_eq!(formed.listing_ts, cohort.listing);
    let finalized = coverage(&mut h, &cohort).await;
    assert!(finalized.coverage_finalized);
    assert_eq!(finalized.coverage_complete_ts, cohort.placement_end());

    // Submission is closed once coverage is final and placement has ended.
    assert!(h
        .propose_source(&cohort, &wrong, source_id(98), 0)
        .await
        .is_err());

    // The zero-support source times out without disturbing coverage and refunds its listing.
    h.send(
        instruction::timeout_unsupported_oracle_source_v2(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
            &unsupported,
        ),
        &[&cranker],
    )
    .await
    .unwrap();
    let timed_out: OracleSourceState = h.read_state(unsupported).await;
    assert_eq!(timed_out.status, OracleSourceStatus::TimedOut);
    assert_eq!(coverage(&mut h, &cohort).await, finalized);
    h.send(
        instruction::settle_oracle_usdc_prelisting_escrow(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
            &PrelistingEscrow::ListingBond {
                source: unsupported,
                proposer: proposer.pubkey(),
            },
        ),
        &[&cranker],
    )
    .await
    .unwrap();
    assert_eq!(
        h.collateral(&proposer.pubkey()).await.available_balance,
        PARTICIPANT_USDC - 3 * LISTING_BOND
    );
    let reward: OracleUsdcSourceReward = h
        .read_state(
            derive_oracle_usdc_source_reward_pda(&program(), &cohort.schedule(), &unsupported).0,
        )
        .await;
    assert!(reward.listing_escrow_counted);
    assert_eq!(
        schedule(&mut h, &cohort)
            .await
            .outstanding_prelisting_escrow_count,
        6
    );
    // Counted exactly once.
    h.advance_slot().await;
    assert!(h
        .send(
            instruction::settle_oracle_usdc_prelisting_escrow(
                &program(),
                &cranker.pubkey(),
                &cohort.period,
                &cohort.month,
                &PrelistingEscrow::ListingBond {
                    source: unsupported,
                    proposer: proposer.pubkey(),
                },
            ),
            &[&cranker],
        )
        .await
        .is_err());
}

#[tokio::test]
async fn late_coverage_shifts_only_the_listing() {
    let mut h = start().await;
    let cohort = h.open_cohort(2).await;
    h.fund_schedule(&cohort).await;
    h.at(cohort.scramble).await;
    let proposer = h.participant(PARTICIPANT_USDC).await;
    let supporter = h.participant(PARTICIPANT_USDC).await;
    let first = h
        .propose_source(&cohort, &proposer, source_id(1), 0)
        .await
        .unwrap();
    h.support_source(&cohort, &supporter, &first, 0)
        .await
        .unwrap();
    let cranker = h.payer();
    // Incomplete coverage cannot finalize.
    assert_oracle_error(
        h.send(
            instruction::finalize_oracle_sku_coverage(
                &program(),
                &cranker.pubkey(),
                &cohort.period,
                &cohort.month,
            ),
            &[],
        )
        .await,
        OracleError::OracleSkuCoverageIncomplete,
    );
    // The month stays in submission past the planned placement end.
    let late = cohort.placement_end() + 1_234;
    h.at(late).await;
    let second = h
        .propose_source(&cohort, &proposer, source_id(2), 1)
        .await
        .unwrap();
    h.support_source(&cohort, &supporter, &second, 1)
        .await
        .unwrap();
    h.send(
        instruction::finalize_oracle_sku_coverage(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
        ),
        &[],
    )
    .await
    .unwrap();
    let shifted = month(&mut h, &cohort).await;
    use ameba_oracle::constants::{
        ORACLE_KILL_WINDOW_SECONDS, ORACLE_OPENING_WINDOW_SECONDS, ORACLE_PLACEMENT_WINDOW_SECONDS,
        ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS,
    };
    assert_eq!(shifted.phase, OraclePhase::Scramble);
    assert_eq!(
        shifted.scramble_start_ts,
        late - ORACLE_PLACEMENT_WINDOW_SECONDS
    );
    assert_eq!(
        shifted.listing_ts,
        late + ORACLE_KILL_WINDOW_SECONDS
            + ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
            + ORACLE_OPENING_WINDOW_SECONDS
    );
    assert!(shifted.listing_ts > cohort.listing);
    let covered = coverage(&mut h, &cohort).await;
    assert_eq!(covered.planned_listing_ts, cohort.listing);
    assert_eq!(covered.coverage_complete_ts, late);
    // The period's planned listing is untouched: only the month moved.
    let period = h.period(cohort.product, cohort.expiry).await;
    assert_eq!(period.planned_listing_ts, cohort.listing);
    assert_eq!(period.effective_listing_ts, 0);
}

#[tokio::test]
async fn source_rejection_invalidates_coverage_and_reopens_submission() {
    let mut h = start().await;
    let cohort = h.open_cohort(1).await;
    h.fund_schedule(&cohort).await;
    h.at(cohort.scramble).await;
    let (proposer, supporter, sources) = cover_every_sku(&mut h, &cohort).await;
    let source = sources[0];
    h.at(cohort.placement_end()).await;
    let cranker = h.payer();
    h.send(
        instruction::finalize_oracle_sku_coverage(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
        ),
        &[],
    )
    .await
    .unwrap();

    // The challenge bond is the clamped share of the source's support.
    let challenger = h.participant(PARTICIPANT_USDC).await;
    let challenge_id = [0xc1; 32];
    let challenge_ix = |bond: u64| {
        instruction::challenge_oracle_source_v2(
            &program(),
            &challenger.pubkey(),
            &cohort.period,
            &cohort.month,
            &source,
            &cohort.sku_ids[0],
            None,
            ChallengeOracleSourceParams {
                challenge_id,
                reason: 1,
                bond,
                evidence_hash: [0xe1; 32],
                comparison_source_id: [0; 32],
            },
        )
    };
    // Not before the kill window opens.
    h.at(cohort.placement_end() - 1).await;
    assert!(h
        .send(challenge_ix(CHALLENGE_MIN_BOND), &[&challenger])
        .await
        .is_err());
    h.at(cohort.placement_end()).await;
    assert_oracle_error(
        h.send(challenge_ix(CHALLENGE_MIN_BOND + 1), &[&challenger])
            .await,
        OracleError::InvalidOracleUsdcBond,
    );
    h.send(challenge_ix(CHALLENGE_MIN_BOND), &[&challenger])
        .await
        .unwrap();
    let challenge = instruction::find_oracle_source_challenge_address(
        &program(),
        &cohort.month,
        &source,
        &challenge_id,
    );
    let guard_key = derive_oracle_source_challenge_guard_pda(&program(), &cohort.month, &source).0;
    let guard: OracleSourceChallengeGuard = h.read_state(guard_key).await;
    assert_eq!(guard.active_challenge, challenge);
    assert_eq!(month(&mut h, &cohort).await.pending_resolution_count, 1);
    assert_eq!(
        h.collateral(&challenger.pubkey()).await.available_balance,
        PARTICIPANT_USDC - CHALLENGE_MIN_BOND
    );
    // One live challenge per source.
    h.advance_slot().await;
    let mut second = challenge_ix(CHALLENGE_MIN_BOND);
    second.data[1] ^= 1;
    second.accounts[6].pubkey =
        instruction::find_oracle_source_challenge_address(&program(), &cohort.month, &source, &{
            let mut id = challenge_id;
            id[0] ^= 1;
            id
        });
    assert!(h.send(second, &[&challenger]).await.is_err());

    // Resolution only in the resolution-freeze window, by the oracle authority.
    let oracle = h.oracle_authority.insecure_clone();
    let resolve = |outcome| {
        instruction::resolve_oracle_source_challenge_v2(
            &program(),
            &oracle.pubkey(),
            &cohort.month,
            &source,
            &challenge,
            &cohort.sku_ids[0],
            None,
            outcome,
        )
    };
    assert_oracle_error(
        h.send(
            resolve(OracleSourceChallengeOutcome::RejectSource),
            &[&oracle],
        )
        .await,
        OracleError::OracleTimingWindowClosed,
    );
    h.at(cohort.kill_end()).await;
    let outsider = Keypair::new();
    let mut by_outsider = resolve(OracleSourceChallengeOutcome::RejectSource);
    by_outsider.accounts[0].pubkey = outsider.pubkey();
    assert_oracle_error(
        h.send(by_outsider, &[&outsider]).await,
        OracleError::Unauthorized,
    );
    h.send(
        resolve(OracleSourceChallengeOutcome::RejectSource),
        &[&oracle],
    )
    .await
    .unwrap();

    let source_after: OracleSourceState = h.read_state(source).await;
    let challenge_after: OracleSourceChallenge = h.read_state(challenge).await;
    let guard_after: OracleSourceChallengeGuard = h.read_state(guard_key).await;
    let coverage_after = coverage(&mut h, &cohort).await;
    let record_after: OracleSkuCoverageRecord = h
        .read_state(
            derive_oracle_sku_coverage_record_pda(&program(), &cohort.month, &cohort.sku_ids[0]).0,
        )
        .await;
    let month_after = month(&mut h, &cohort).await;
    assert_eq!(source_after.status, OracleSourceStatus::Rejected);
    assert_eq!(challenge_after.status, OracleChallengeStatus::Accepted);
    assert_eq!(guard_after.active_challenge, Pubkey::default());
    assert_eq!(coverage_after.covered_sku_count, 0);
    assert!(!coverage_after.coverage_finalized);
    assert_eq!(coverage_after.coverage_complete_ts, 0);
    assert_eq!(record_after.active_supported_source_count, 0);
    assert_eq!(month_after.pending_resolution_count, 0);
    assert_eq!(month_after.source_count, 0);
    assert_eq!(month_after.phase, OraclePhase::Scramble);

    h.send(
        instruction::reopen_oracle_sku_coverage(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
        ),
        &[],
    )
    .await
    .unwrap();
    let reopened = month(&mut h, &cohort).await;
    let reopened_coverage = coverage(&mut h, &cohort).await;
    assert_eq!(reopened.phase, OraclePhase::SourceSubmission);
    assert!(!reopened_coverage.coverage_finalized);
    assert_eq!(reopened_coverage.covered_sku_count, 0);

    // The accepted challenge wins the rejected listing bond; the supporter is refunded.
    let before = schedule(&mut h, &cohort)
        .await
        .outstanding_prelisting_escrow_count;
    h.send(
        instruction::settle_oracle_usdc_prelisting_escrow(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
            &PrelistingEscrow::SourceChallenge {
                source,
                proposer: proposer.pubkey(),
                challenge,
                challenger: challenger.pubkey(),
                comparison_source: None,
            },
        ),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        h.collateral(&challenger.pubkey()).await.available_balance,
        PARTICIPANT_USDC + LISTING_BOND
    );
    let settled: OracleSourceChallenge = h.read_state(challenge).await;
    assert_eq!(
        settled.escrow_disposition,
        OracleEscrowDisposition::Refunded
    );
    assert!(settled.failed_schedule_escrow_counted);
    // Challenge and listing both counted.
    assert_eq!(
        schedule(&mut h, &cohort)
            .await
            .outstanding_prelisting_escrow_count,
        before - 2
    );
    h.send(
        instruction::settle_oracle_usdc_prelisting_escrow(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
            &PrelistingEscrow::SupportStake {
                source,
                supporter: supporter.pubkey(),
            },
        ),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        h.collateral(&supporter.pubkey()).await.available_balance,
        PARTICIPANT_USDC
    );
    let support: OracleSupportPosition = h
        .read_state(instruction::find_oracle_support_address(
            &program(),
            &cohort.month,
            &source,
            &supporter.pubkey(),
        ))
        .await;
    assert!(support.released);
    assert_eq!(
        schedule(&mut h, &cohort)
            .await
            .outstanding_prelisting_escrow_count,
        0
    );
    assert_bond_vault_covers_ledgers(&mut h, &[&proposer, &supporter, &challenger]).await;
}

#[tokio::test]
async fn non_independent_sources_merge_their_support_into_the_comparison_source() {
    let mut h = start().await;
    let cohort = h.open_cohort(1).await;
    h.fund_schedule(&cohort).await;
    h.at(cohort.scramble).await;
    let proposer = h.participant(PARTICIPANT_USDC).await;
    let other = h.participant(PARTICIPANT_USDC).await;
    let supporter = h.participant(PARTICIPANT_USDC).await;
    let duplicate = h
        .propose_source(&cohort, &proposer, source_id(1), 0)
        .await
        .unwrap();
    let canonical = h
        .propose_source(&cohort, &other, source_id(2), 0)
        .await
        .unwrap();
    h.support_source(&cohort, &supporter, &duplicate, 0)
        .await
        .unwrap();
    h.support_source(&cohort, &supporter, &canonical, 0)
        .await
        .unwrap();
    h.at(cohort.placement_end()).await;
    let cranker = h.payer();
    h.send(
        instruction::finalize_oracle_sku_coverage(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
        ),
        &[],
    )
    .await
    .unwrap();

    let challenger = h.participant(PARTICIPANT_USDC).await;
    let challenge_id = [0xc2; 32];
    h.send(
        instruction::challenge_oracle_source_v2(
            &program(),
            &challenger.pubkey(),
            &cohort.period,
            &cohort.month,
            &duplicate,
            &cohort.sku_ids[0],
            Some(&canonical),
            ChallengeOracleSourceParams {
                challenge_id,
                reason: NON_INDEPENDENT,
                bond: CHALLENGE_MIN_BOND,
                evidence_hash: [0xe2; 32],
                comparison_source_id: source_id(2),
            },
        ),
        &[&challenger],
    )
    .await
    .unwrap();
    // Both guards are held by the one challenge.
    let challenge = instruction::find_oracle_source_challenge_address(
        &program(),
        &cohort.month,
        &duplicate,
        &challenge_id,
    );
    for source in [duplicate, canonical] {
        let guard: OracleSourceChallengeGuard = h
            .read_state(
                derive_oracle_source_challenge_guard_pda(&program(), &cohort.month, &source).0,
            )
            .await;
        assert_eq!(guard.active_challenge, challenge);
    }

    h.at(cohort.kill_end()).await;
    let oracle = h.oracle_authority.insecure_clone();
    h.send(
        instruction::resolve_oracle_source_challenge_v2(
            &program(),
            &oracle.pubkey(),
            &cohort.month,
            &duplicate,
            &challenge,
            &cohort.sku_ids[0],
            Some(&canonical),
            OracleSourceChallengeOutcome::MergeSource,
        ),
        &[&oracle],
    )
    .await
    .unwrap();
    let merged: OracleSourceState = h.read_state(duplicate).await;
    let root: OracleSourceState = h.read_state(canonical).await;
    assert_eq!(merged.status, OracleSourceStatus::Merged);
    assert_eq!(merged.support_stake_total, 0);
    assert_eq!(root.status, OracleSourceStatus::Candidate);
    assert_eq!(root.support_stake_total, 2 * SUPPORT_BOND);
    let merged_reward: OracleUsdcSourceReward = h
        .read_state(
            derive_oracle_usdc_source_reward_pda(&program(), &cohort.schedule(), &duplicate).0,
        )
        .await;
    let root_reward: OracleUsdcSourceReward = h
        .read_state(
            derive_oracle_usdc_source_reward_pda(&program(), &cohort.schedule(), &canonical).0,
        )
        .await;
    assert_eq!(merged_reward.merged_into_source, canonical);
    assert_eq!(root_reward.supporter_count, 2);
    assert_eq!(root_reward.max_merge_depth, 1);
    // The SKU stays covered by the surviving source.
    let covered = coverage(&mut h, &cohort).await;
    assert_eq!(covered.covered_sku_count, 1);
    for source in [duplicate, canonical] {
        let guard: OracleSourceChallengeGuard = h
            .read_state(
                derive_oracle_source_challenge_guard_pda(&program(), &cohort.month, &source).0,
            )
            .await;
        assert_eq!(guard.active_challenge, Pubkey::default());
    }
}

#[tokio::test]
async fn failed_month_refunds_every_counted_escrow_and_aborts_the_schedule() {
    let mut h = start().await;
    let cohort = h.open_cohort(2).await;
    h.fund_schedule(&cohort).await;
    h.at(cohort.scramble).await;
    let proposer = h.participant(PARTICIPANT_USDC).await;
    let supporter = h.participant(PARTICIPANT_USDC).await;
    // Only one of two SKUs is ever covered.
    let source = h
        .propose_source(&cohort, &proposer, source_id(1), 0)
        .await
        .unwrap();
    h.support_source(&cohort, &supporter, &source, 0)
        .await
        .unwrap();
    let cranker = h.payer();
    let escrows = [
        PrelistingEscrow::ListingBond {
            source,
            proposer: proposer.pubkey(),
        },
        PrelistingEscrow::SupportStake {
            source,
            supporter: supporter.pubkey(),
        },
    ];

    // Before the latest safe boundary nothing in the failure lane runs.
    use ameba_oracle::constants::{
        ORACLE_KILL_WINDOW_SECONDS, ORACLE_OPENING_WINDOW_SECONDS,
        ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS,
    };
    let latest_safe = cohort.expiry
        - ORACLE_KILL_WINDOW_SECONDS
        - ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
        - ORACLE_OPENING_WINDOW_SECONDS;
    h.at(latest_safe - 1).await;
    for escrow in &escrows {
        assert!(h
            .send(
                instruction::settle_failed_oracle_month_escrow_v2(
                    &program(),
                    &cranker.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    escrow,
                ),
                &[],
            )
            .await
            .is_err());
    }
    assert_oracle_error(
        h.send(
            instruction::expire_unlistable_oracle_source_v2(
                &program(),
                &cranker.pubkey(),
                &cohort.period,
                &cohort.month,
                &source,
                Some(&cohort.sku_ids[0]),
            ),
            &[],
        )
        .await,
        OracleError::OracleTimingWindowClosed,
    );

    h.at(latest_safe).await;
    // The reservation cannot be released while escrows are outstanding.
    assert_oracle_error(
        h.send(
            instruction::abort_oracle_usdc_reward_schedule_v2(
                &program(),
                &cranker.pubkey(),
                &cohort.period,
                &cohort.month,
            ),
            &[],
        )
        .await,
        OracleError::InvalidOracleUsdcRewardSchedule,
    );
    h.send(
        instruction::expire_unlistable_oracle_source_v2(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
            &source,
            Some(&cohort.sku_ids[0]),
        ),
        &[],
    )
    .await
    .unwrap();
    let expired: OracleSourceState = h.read_state(source).await;
    assert_eq!(expired.status, OracleSourceStatus::TimedOut);
    assert_eq!(coverage(&mut h, &cohort).await.covered_sku_count, 0);
    for escrow in &escrows {
        h.send(
            instruction::settle_failed_oracle_month_escrow_v2(
                &program(),
                &cranker.pubkey(),
                &cohort.period,
                &cohort.month,
                escrow,
            ),
            &[],
        )
        .await
        .unwrap();
    }
    assert_eq!(
        h.collateral(&proposer.pubkey()).await.available_balance,
        PARTICIPANT_USDC
    );
    assert_eq!(
        h.collateral(&supporter.pubkey()).await.available_balance,
        PARTICIPANT_USDC
    );
    assert_eq!(
        schedule(&mut h, &cohort)
            .await
            .outstanding_prelisting_escrow_count,
        0
    );

    let reserved_before: OracleUsdcRewardVault = h
        .read_state(derive_oracle_usdc_reward_vault_pda(&program()).0)
        .await;
    h.send(
        instruction::abort_oracle_usdc_reward_schedule_v2(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
        ),
        &[],
    )
    .await
    .unwrap();
    let aborted = schedule(&mut h, &cohort).await;
    assert_eq!(aborted.phase, OracleUsdcRewardSchedulePhase::Aborted);
    assert_eq!(aborted.remaining_reward_budget, 0);
    let reserved_after: OracleUsdcRewardVault = h
        .read_state(derive_oracle_usdc_reward_vault_pda(&program()).0)
        .await;
    assert_eq!(
        reserved_before.total_reserved - reserved_after.total_reserved,
        aborted.total_reward_budget
    );
    // Aborted once only.
    h.advance_slot().await;
    assert!(h
        .send(
            instruction::abort_oracle_usdc_reward_schedule_v2(
                &program(),
                &cranker.pubkey(),
                &cohort.period,
                &cohort.month,
            ),
            &[],
        )
        .await
        .is_err());
}

#[tokio::test]
async fn stale_source_challenge_is_cancelled_once_the_month_can_no_longer_list() {
    let mut h = start().await;
    let cohort = h.open_cohort(1).await;
    h.fund_schedule(&cohort).await;
    h.at(cohort.scramble).await;
    let (proposer, _supporter, sources) = cover_every_sku(&mut h, &cohort).await;
    let source = sources[0];
    h.at(cohort.placement_end()).await;
    let cranker = h.payer();
    h.send(
        instruction::finalize_oracle_sku_coverage(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
        ),
        &[],
    )
    .await
    .unwrap();
    let challenger = h.participant(PARTICIPANT_USDC).await;
    let challenge_id = [0xc3; 32];
    h.send(
        instruction::challenge_oracle_source_v2(
            &program(),
            &challenger.pubkey(),
            &cohort.period,
            &cohort.month,
            &source,
            &cohort.sku_ids[0],
            None,
            ChallengeOracleSourceParams {
                challenge_id,
                reason: 1,
                bond: CHALLENGE_MIN_BOND,
                evidence_hash: [0xe3; 32],
                comparison_source_id: [0; 32],
            },
        ),
        &[&challenger],
    )
    .await
    .unwrap();
    let challenge = instruction::find_oracle_source_challenge_address(
        &program(),
        &cohort.month,
        &source,
        &challenge_id,
    );
    let cancel = instruction::cancel_stale_oracle_source_challenge_v2(
        &program(),
        &cranker.pubkey(),
        &cohort.period,
        &cohort.month,
        &source,
        &challenge,
        None,
    );
    // Not while the resolution window can still decide it.
    h.at(cohort.scramble_end() - 1).await;
    assert!(h.send(cancel.clone(), &[]).await.is_err());
    h.at(cohort.scramble_end()).await;
    h.send(cancel.clone(), &[]).await.unwrap();
    let cancelled: OracleSourceChallenge = h.read_state(challenge).await;
    assert_eq!(cancelled.status, OracleChallengeStatus::Cancelled);
    let reopened = month(&mut h, &cohort).await;
    assert_eq!(reopened.phase, OraclePhase::SourceSubmission);
    assert_eq!(reopened.pending_resolution_count, 0);
    assert!(!coverage(&mut h, &cohort).await.coverage_finalized);
    h.advance_slot().await;
    assert!(h.send(cancel, &[]).await.is_err());

    // The cancelled challenge refunds its bond through the counted lane.
    h.send(
        instruction::settle_oracle_usdc_prelisting_escrow(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
            &PrelistingEscrow::SourceChallenge {
                source,
                proposer: proposer.pubkey(),
                challenge,
                challenger: challenger.pubkey(),
                comparison_source: None,
            },
        ),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        h.collateral(&challenger.pubkey()).await.available_balance,
        PARTICIPANT_USDC
    );
}

#[tokio::test]
async fn formation_rejects_a_month_of_another_period() {
    let mut h = start().await;
    let cohort = h.open_cohort(1).await;
    h.fund_schedule(&cohort).await;
    h.at(cohort.scramble).await;
    let proposer = h.participant(PARTICIPANT_USDC).await;
    h.upload_evidence(&proposer, 1, &locator(&source_id(1)))
        .await;
    h.upload_evidence(&proposer, 2, &definition(&source_id(1)))
        .await;
    // A second Forming period of the same product on the next rung.
    let oracle = h.oracle_authority.insecure_clone();
    h.create_period(&oracle, cohort.product, JAN_01_2027, APR_01_2027, 6)
        .await
        .unwrap();
    let other_period =
        ameba_oracle_interface::find_period_address(&program(), &cohort.product, APR_01_2027).0;
    let mut ix = h.propose_ix(&cohort, &proposer.pubkey(), source_id(1), 0);
    ix.accounts[1].pubkey = other_period;
    assert_oracle_error(
        h.send(ix, &[&proposer]).await,
        OracleError::InvalidOracleMonthAccount,
    );
    let mut ix = h.propose_ix(&cohort, &proposer.pubkey(), source_id(1), 0);
    ix.accounts[1].pubkey = Pubkey::new_unique();
    assert_oracle_error(
        h.send(ix, &[&proposer]).await,
        OracleError::InvalidPeriodAccount,
    );
    // A carry period proof at any other address is refused.
    let mut ix = h.propose_ix(&cohort, &proposer.pubkey(), source_id(1), 0);
    ix.accounts[11].pubkey = Pubkey::new_unique();
    assert_oracle_error(h.send(ix, &[&proposer]).await, OracleError::InvalidPda);
    let ix = h.propose_ix(&cohort, &proposer.pubkey(), source_id(1), 0);
    h.send(ix, &[&proposer]).await.unwrap();
}
