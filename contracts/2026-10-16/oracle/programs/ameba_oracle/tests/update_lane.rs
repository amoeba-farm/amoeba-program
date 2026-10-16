//! Game-mode cash updates on a Live period: commit (166), reveal (198), challenge (168),
//! finalize (199), expired commitments (167) and stale claims (201).
mod common;

use ameba_oracle::{
    constants::ORACLE_UPDATE_REVEAL_WINDOW_SLOTS,
    error::OracleError,
    instruction::{
        self, ChallengeOracleUpdateClaimParams, FinalizeOracleUpdateClaimV2Params,
        UpdateFinalization,
    },
    state::{
        derive_oracle_bucket_median_pda, derive_user_collateral_pda, OracleBucketMedianState,
        OracleBucketMedianStatus, OracleChallengeStatus, OracleClaimStatus,
        OracleEscrowDisposition, OracleEscrowKind, OracleMonthState, OraclePhase,
        OracleSourceState, OracleUpdateChallenge, OracleUpdateChallengeGuard,
        OracleUpdateClaimOutcome, OracleUpdateClaimV2, UserCollateral,
    },
};
use ameba_oracle_interface::PeriodStatus;
use common::{cohort::*, updates::*, *};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Signer,
};

const DAY: u64 = 86_400;

/// A Live cohort of two SKUs, one Active source each, with the clock at the listing.
async fn live(h: &mut Harness) -> (Cohort, Formed) {
    let cohort = h.open_cohort(2).await;
    let formed = h.form_to_opening(&cohort).await;
    h.at(cohort.listing).await;
    let cranker = h.payer();
    h.send(
        instruction::finalize_oracle_opening_phase(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
        ),
        &[],
    )
    .await
    .unwrap();
    (cohort, formed)
}

/// The futures index read: phase at 86, delta at 125, record tag at 142.
fn readable_index(data: &[u8]) -> (u8, i64, u8) {
    assert_eq!(data.len(), OracleMonthState::LEN);
    (
        data[86],
        i64::from_le_bytes(data[125..133].try_into().unwrap()),
        data[142],
    )
}

impl Harness {
    async fn available(&mut self, owner: &Pubkey) -> u64 {
        self.collateral(owner).await.available_balance
    }

    async fn assert_live_and_readable(&mut self, cohort: &Cohort, at: u64) {
        let period = self.period(cohort.product, cohort.expiry).await;
        assert_eq!(period.status, PeriodStatus::Live);
        assert!(period.is_tradable_at(at));
        let raw = self.account(cohort.month).await.unwrap();
        let (phase, _delta, record_tag) = readable_index(&raw.data);
        assert_eq!(phase, OraclePhase::Game as u8);
        assert_eq!(record_tag, 0);
    }
}

fn challenge_bond(stake: u64) -> u64 {
    (stake * u64::from(CHALLENGE_BOND_BPS) / 10_000).clamp(CHALLENGE_MIN_BOND, CHALLENGE_MAX_BOND)
}

#[tokio::test]
async fn accepted_update_moves_the_source_and_the_live_index_stays_readable_throughout_game() {
    let mut h = start().await;
    let (cohort, formed) = live(&mut h).await;
    let (source, source_id) = formed.sources[0];
    let claimant = h.participant(PARTICIPANT_USDC).await;
    h.assert_live_and_readable(&cohort, cohort.listing).await;

    let now = cohort.listing + 3_600;
    h.at(now).await;
    let print = h
        .print(
            &cohort,
            &claimant,
            &source,
            &source_id,
            105_000_000,
            now - 60,
        )
        .await;
    let before = h.available(&claimant.pubkey()).await;
    let claim = h
        .commit(&cohort, &claimant, &source, &source_id, 0, [1; 32], print)
        .await
        .unwrap();
    assert_eq!(
        h.available(&claimant.pubkey()).await,
        before - UPDATE_MIN_BOND
    );
    let committed: OracleUpdateClaimV2 = h.read_state(claim.key).await;
    assert_eq!(committed.claim.status, OracleClaimStatus::Committed);
    assert_eq!(committed.claim.new_state, 0);
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.pending_resolution_count, 1);

    // The same slot is before the earliest reveal.
    assert_oracle_error(
        h.reveal(&cohort, &claimant, &source, &claim).await,
        OracleError::InvalidOracleUpdateRevealTiming,
    );
    h.advance_slot().await;
    h.reveal(&cohort, &claimant, &source, &claim).await.unwrap();
    let revealed: OracleUpdateClaimV2 = h.read_state(claim.key).await;
    assert_eq!(revealed.claim.status, OracleClaimStatus::Revealed);
    assert_eq!(revealed.claim.prior_state, OPENING_STATE);
    assert_eq!(revealed.claim.new_state, 105_000_000);
    assert_eq!(revealed.revealed_at_ts, now);
    assert_eq!(revealed.freshness_reward_multiplier, 1);
    h.assert_live_and_readable(&cohort, now).await;

    // A full day passes before anyone can finalize.
    h.at(now + ameba_oracle::constants::ORACLE_UPDATE_CHALLENGE_SECONDS - 1)
        .await;
    assert_oracle_error(
        h.finalize(
            &cohort,
            &source,
            0,
            &claim.key,
            OracleUpdateClaimOutcome::AcceptClaim,
            1,
            UpdateFinalization::Unchallenged,
        )
        .await,
        OracleError::OracleTimingWindowClosed,
    );
    h.at(now + DAY).await;
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

    let updated: OracleSourceState = h.read_state(source).await;
    assert_eq!(updated.current_state, 105_000_000);
    assert_eq!(updated.observation_count, 2);
    assert_eq!(updated.last_finalized_step, 1);
    assert!(h
        .account(instruction::find_checkpoint_address(
            &program(),
            &source,
            &claim.key
        ))
        .await
        .is_some());
    let finalized: OracleUpdateClaimV2 = h.read_state(claim.key).await;
    assert_eq!(finalized.claim.status, OracleClaimStatus::Finalized);
    let bucket: OracleBucketMedianState = h
        .read_state(
            derive_oracle_bucket_median_pda(&program(), &cohort.month, &cohort.sku_ids[0]).0,
        )
        .await;
    assert_eq!(bucket.status, OracleBucketMedianStatus::Dirty);
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.pending_resolution_count, 0);
    assert_eq!(month.accepted_cash_update_count, 1);
    h.assert_live_and_readable(&cohort, now + DAY).await;

    // Exactly once.
    h.advance_slot().await;
    assert!(h
        .finalize(
            &cohort,
            &source,
            0,
            &claim.key,
            OracleUpdateClaimOutcome::AcceptClaim,
            2,
            UpdateFinalization::Unchallenged,
        )
        .await
        .is_err());

    // The accepted claimant's stake settles back through tag 169.
    let cranker = h.payer();
    h.send(
        instruction::settle_oracle_usdc_escrow(
            &program(),
            OracleEscrowKind::UpdateClaim,
            vec![
                AccountMeta::new_readonly(cranker.pubkey(), true),
                AccountMeta::new_readonly(cohort.period, false),
                AccountMeta::new_readonly(cohort.month, false),
                AccountMeta::new_readonly(source, false),
                AccountMeta::new(claim.key, false),
                AccountMeta::new(
                    derive_user_collateral_pda(&program(), &claimant.pubkey()).0,
                    false,
                ),
                AccountMeta::new_readonly(
                    instruction::find_oracle_update_guard_address(
                        &program(),
                        &cohort.month,
                        &claim.key,
                    ),
                    false,
                ),
            ],
        ),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(h.available(&claimant.pubkey()).await, before);

    // Late in Game, a second print still updates the same source while the period trades.
    let late = cohort.expiry - 2 * DAY;
    h.at(late).await;
    let print = h
        .print(
            &cohort,
            &claimant,
            &source,
            &source_id,
            104_000_000,
            late - 60,
        )
        .await;
    let second = h
        .commit(&cohort, &claimant, &source, &source_id, 0, [2; 32], print)
        .await
        .unwrap();
    h.advance_slot().await;
    h.reveal(&cohort, &claimant, &source, &second)
        .await
        .unwrap();
    let fresh: OracleUpdateClaimV2 = h.read_state(second.key).await;
    assert_eq!(
        fresh.freshness_reward_multiplier,
        ameba_oracle::constants::ORACLE_FRESH_UPDATE_REWARD_MULTIPLIER
    );
    h.assert_live_and_readable(&cohort, late).await;
}

#[tokio::test]
async fn challenged_update_settles_to_the_alternative_and_the_guard_admits_one_challenge() {
    let mut h = start().await;
    let (cohort, formed) = live(&mut h).await;
    let (source, source_id) = formed.sources[1];
    let claimant = h.participant(PARTICIPANT_USDC).await;
    let challenger = h.participant(PARTICIPANT_USDC).await;
    let now = cohort.listing + 7_200;
    h.at(now).await;
    let print = h
        .print(
            &cohort,
            &claimant,
            &source,
            &source_id,
            110_000_000,
            now - 120,
        )
        .await;
    let claim = h
        .commit(&cohort, &claimant, &source, &source_id, 1, [3; 32], print)
        .await
        .unwrap();
    h.advance_slot().await;
    h.reveal(&cohort, &claimant, &source, &claim).await.unwrap();

    let alternative = h
        .print(
            &cohort,
            &challenger,
            &source,
            &source_id,
            99_000_000,
            now - 60,
        )
        .await;
    let bond = challenge_bond(UPDATE_MIN_BOND);
    let challenge_ix = |challenge_id: [u8; 32], bond: u64, alternative: &Print| {
        instruction::challenge_oracle_update_claim_v2(
            &program(),
            &challenger.pubkey(),
            &challenger.pubkey(),
            &cohort.period,
            &cohort.month,
            &source,
            &cohort.sku_ids[1],
            &claim.key,
            ChallengeOracleUpdateClaimParams {
                challenge_id,
                alternative_state: alternative.state,
                alternative_source_time: alternative.source_time,
                bond,
                evidence_hash: alternative.evidence,
                archive_url: String::new(),
            },
            &alternative.url,
        )
    };
    assert_oracle_error(
        h.send(
            challenge_ix([4; 32], bond + 1, &alternative),
            &[&challenger],
        )
        .await,
        OracleError::InvalidOracleUsdcBond,
    );
    let before = h.available(&challenger.pubkey()).await;
    h.send(challenge_ix([4; 32], bond, &alternative), &[&challenger])
        .await
        .unwrap();
    assert_eq!(h.available(&challenger.pubkey()).await, before - bond);
    let challenge_key = instruction::find_oracle_update_challenge_address(
        &program(),
        &cohort.month,
        &claim.key,
        &[4; 32],
    );
    let guard_key =
        instruction::find_oracle_update_guard_address(&program(), &cohort.month, &claim.key);
    let guard: OracleUpdateChallengeGuard = h.read_state(guard_key).await;
    assert_eq!(guard.challenge, challenge_key);
    // The guard admits exactly one challenge per claim.
    h.advance_slot().await;
    assert!(h
        .send(challenge_ix([5; 32], bond, &alternative), &[&challenger])
        .await
        .is_err());

    // An unchallenged-shape finalize no longer applies.
    h.at(now + DAY).await;
    assert!(h
        .finalize(
            &cohort,
            &source,
            1,
            &claim.key,
            OracleUpdateClaimOutcome::AcceptClaim,
            1,
            UpdateFinalization::Unchallenged,
        )
        .await
        .is_err());
    h.finalize(
        &cohort,
        &source,
        1,
        &claim.key,
        OracleUpdateClaimOutcome::RejectClaim,
        1,
        UpdateFinalization::Challenged {
            challenge: challenge_key,
        },
    )
    .await
    .unwrap();
    let updated: OracleSourceState = h.read_state(source).await;
    assert_eq!(updated.current_state, 99_000_000);
    assert_eq!(updated.observation_count, 2);
    let rejected: OracleUpdateClaimV2 = h.read_state(claim.key).await;
    assert_eq!(rejected.claim.status, OracleClaimStatus::Rejected);
    let accepted: OracleUpdateChallenge = h.read_state(challenge_key).await;
    assert_eq!(accepted.status, OracleChallengeStatus::Accepted);
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.pending_resolution_count, 0);
    h.assert_live_and_readable(&cohort, now + DAY).await;
}

#[tokio::test]
async fn update_challenge_rejects_an_initialized_pda_without_debit_or_overwrite() {
    let mut h = start().await;
    let (cohort, formed) = live(&mut h).await;
    let (source, source_id) = formed.sources[0];
    let claimant = h.participant(PARTICIPANT_USDC).await;
    let challenger = h.participant(PARTICIPANT_USDC).await;
    let now = cohort.listing + 600;
    h.at(now).await;
    let print = h
        .print(
            &cohort,
            &claimant,
            &source,
            &source_id,
            101_000_000,
            now - 60,
        )
        .await;
    let claim = h
        .commit(&cohort, &claimant, &source, &source_id, 0, [6; 32], print)
        .await
        .unwrap();
    h.advance_slot().await;
    h.reveal(&cohort, &claimant, &source, &claim).await.unwrap();

    let challenge_id = [153u8; 32];
    let challenge_key = instruction::find_oracle_update_challenge_address(
        &program(),
        &cohort.month,
        &claim.key,
        &challenge_id,
    );
    let bump = Pubkey::find_program_address(
        &[
            ameba_oracle::constants::CURRENT_STATE_NAMESPACE_SEED,
            ameba_oracle::constants::ORACLE_UPDATE_CHALLENGE_PDA_SEED,
            cohort.month.as_ref(),
            claim.key.as_ref(),
            &challenge_id,
        ],
        &program(),
    )
    .1;
    let original = OracleUpdateChallenge {
        is_initialized: true,
        bump,
        month: cohort.month,
        challenge_id,
        claim: claim.key,
        claim_id: claim.claim_id,
        challenger: claimant.pubkey(),
        alternative_state: 99_000_000,
        bond: 250,
        required_bond: 250,
        status: OracleChallengeStatus::RuleReviewUnresolved,
        evidence_hash: [154u8; 32],
        rule_review_slot: 7,
        escrow_disposition: OracleEscrowDisposition::Unsettled,
        account_discriminator: OracleUpdateChallenge::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUpdateChallenge::ACCOUNT_VERSION,
        ..OracleUpdateChallenge::default()
    };
    h.set_program_state(challenge_key, &original, OracleUpdateChallenge::LEN)
        .await;
    let alternative = h
        .print(
            &cohort,
            &challenger,
            &source,
            &source_id,
            90_000_000,
            now - 30,
        )
        .await;
    let collateral_before: UserCollateral = h.collateral(&challenger.pubkey()).await;
    assert_oracle_error(
        h.send(
            instruction::challenge_oracle_update_claim_v2(
                &program(),
                &challenger.pubkey(),
                &challenger.pubkey(),
                &cohort.period,
                &cohort.month,
                &source,
                &cohort.sku_ids[0],
                &claim.key,
                ChallengeOracleUpdateClaimParams {
                    challenge_id,
                    alternative_state: alternative.state,
                    alternative_source_time: alternative.source_time,
                    bond: challenge_bond(UPDATE_MIN_BOND),
                    evidence_hash: alternative.evidence,
                    archive_url: String::new(),
                },
                &alternative.url,
            ),
            &[&challenger],
        )
        .await,
        OracleError::AlreadyInitialized,
    );
    let after: OracleUpdateChallenge = h.read_state(challenge_key).await;
    assert_eq!(after, original);
    assert_eq!(h.collateral(&challenger.pubkey()).await, collateral_before);
    assert!(h
        .account(instruction::find_oracle_update_guard_address(
            &program(),
            &cohort.month,
            &claim.key
        ))
        .await
        .is_none());
}

#[tokio::test]
async fn unrevealed_commitment_refunds_only_after_its_reveal_deadline() {
    let mut h = start().await;
    let (cohort, formed) = live(&mut h).await;
    let (source, source_id) = formed.sources[0];
    let claimant = h.participant(PARTICIPANT_USDC).await;
    let now = cohort.listing + 60;
    h.at(now).await;
    let print = h
        .print(
            &cohort,
            &claimant,
            &source,
            &source_id,
            102_000_000,
            now - 30,
        )
        .await;
    let before = h.available(&claimant.pubkey()).await;
    let claim = h
        .commit(&cohort, &claimant, &source, &source_id, 0, [7; 32], print)
        .await
        .unwrap();
    let settle = |h: &Harness| -> Instruction {
        instruction::settle_expired_oracle_update_commitment_v3(
            &program(),
            &h.ctx.payer.pubkey(),
            &cohort.period,
            &cohort.month,
            &source,
            &claim.key,
            &claimant.pubkey(),
        )
    };
    assert_oracle_error(
        h.send(settle(&h), &[]).await,
        OracleError::OracleEscrowNotSettleable,
    );
    h.advance_slots(ORACLE_UPDATE_REVEAL_WINDOW_SLOTS + 2).await;
    h.send(settle(&h), &[]).await.unwrap();
    assert_eq!(h.available(&claimant.pubkey()).await, before);
    let timed_out: OracleUpdateClaimV2 = h.read_state(claim.key).await;
    assert_eq!(timed_out.claim.status, OracleClaimStatus::TimedOut);
    assert_eq!(
        timed_out.claim.escrow_disposition,
        OracleEscrowDisposition::Refunded
    );
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.pending_resolution_count, 0);
    // A late reveal finds nothing to reveal.
    assert!(h.reveal(&cohort, &claimant, &source, &claim).await.is_err());
    h.advance_slot().await;
    assert!(h.send(settle(&h), &[]).await.is_err());
}

#[tokio::test]
async fn tag_201_absent_guard_is_fail_closed_until_the_exact_cleanup_boundary() {
    let mut h = start().await;
    let (cohort, formed) = live(&mut h).await;
    let (source, source_id) = formed.sources[0];
    let claimant = h.participant(PARTICIPANT_USDC).await;
    let now = cohort.listing + 60;
    h.at(now).await;
    let print = h
        .print(
            &cohort,
            &claimant,
            &source,
            &source_id,
            103_000_000,
            now - 30,
        )
        .await;
    let claim = h
        .commit(&cohort, &claimant, &source, &source_id, 0, [8; 32], print)
        .await
        .unwrap();
    h.advance_slot().await;
    h.reveal(&cohort, &claimant, &source, &claim).await.unwrap();
    let cancel = |h: &Harness| {
        instruction::cancel_stale_oracle_update_claim_v2(
            &program(),
            &h.ctx.payer.pubkey(),
            &cohort.period,
            &cohort.month,
            &source,
            &claim.key,
            None,
        )
    };

    // A wrong claim layout is rejected before any timing check.
    let valid: OracleUpdateClaimV2 = h.read_state(claim.key).await;
    let mut wrong_layout = valid.clone();
    wrong_layout.claim.account_version = 0;
    h.set_program_state(claim.key, &wrong_layout, OracleUpdateClaimV2::LEN)
        .await;
    assert_oracle_error(
        h.send(cancel(&h), &[]).await,
        OracleError::InvalidOracleUpdateAccount,
    );
    h.set_program_state(claim.key, &valid, OracleUpdateClaimV2::LEN)
        .await;

    let boundary = cohort.expiry + ameba_oracle::constants::ORACLE_SETTLEMENT_GRACE_SECONDS;
    h.at(boundary - 1).await;
    assert_oracle_error(
        h.send(cancel(&h), &[]).await,
        OracleError::OracleTimingWindowClosed,
    );
    h.at(boundary).await;
    h.send(cancel(&h), &[]).await.unwrap();
    let terminal: OracleUpdateClaimV2 = h.read_state(claim.key).await;
    assert_eq!(terminal.claim.status, OracleClaimStatus::TimedOut);
    assert_eq!(
        terminal.claim.escrow_disposition,
        OracleEscrowDisposition::Unsettled
    );
    assert!(!terminal.council_review_pending);
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.pending_resolution_count, 0);

    let before = h.available(&claimant.pubkey()).await;
    let cranker = h.payer();
    h.send(
        instruction::settle_oracle_usdc_escrow(
            &program(),
            OracleEscrowKind::UpdateClaim,
            vec![
                AccountMeta::new_readonly(cranker.pubkey(), true),
                AccountMeta::new_readonly(cohort.period, false),
                AccountMeta::new_readonly(cohort.month, false),
                AccountMeta::new_readonly(source, false),
                AccountMeta::new(claim.key, false),
                AccountMeta::new(
                    derive_user_collateral_pda(&program(), &claimant.pubkey()).0,
                    false,
                ),
                AccountMeta::new_readonly(
                    instruction::find_oracle_update_guard_address(
                        &program(),
                        &cohort.month,
                        &claim.key,
                    ),
                    false,
                ),
            ],
        ),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        h.available(&claimant.pubkey()).await,
        before + terminal.claim.stake
    );
    let refunded: OracleUpdateClaimV2 = h.read_state(claim.key).await;
    assert_eq!(
        refunded.claim.escrow_disposition,
        OracleEscrowDisposition::Refunded
    );
    h.advance_slot().await;
    assert_oracle_error(
        h.send(cancel(&h), &[]).await,
        OracleError::InvalidOracleUpdateAccount,
    );
}

#[tokio::test]
async fn tag_201_handles_guarded_and_unresolved_stale_claims_exactly_once() {
    let mut h = start().await;
    let (cohort, formed) = live(&mut h).await;
    let (source, source_id) = formed.sources[0];
    let claimant = h.participant(PARTICIPANT_USDC).await;
    let challenger = h.participant(PARTICIPANT_USDC).await;
    let now = cohort.listing + 600;
    h.at(now).await;

    // Claim A is challenged and its rule review is left unresolved at step 1.
    let print = h
        .print(
            &cohort,
            &claimant,
            &source,
            &source_id,
            106_000_000,
            now - 60,
        )
        .await;
    let a = h
        .commit(&cohort, &claimant, &source, &source_id, 0, [9; 32], print)
        .await
        .unwrap();
    h.advance_slot().await;
    h.reveal(&cohort, &claimant, &source, &a).await.unwrap();
    let alternative = h
        .print(
            &cohort,
            &challenger,
            &source,
            &source_id,
            98_000_000,
            now - 30,
        )
        .await;
    h.send(
        instruction::challenge_oracle_update_claim_v2(
            &program(),
            &challenger.pubkey(),
            &challenger.pubkey(),
            &cohort.period,
            &cohort.month,
            &source,
            &cohort.sku_ids[0],
            &a.key,
            ChallengeOracleUpdateClaimParams {
                challenge_id: [10; 32],
                alternative_state: alternative.state,
                alternative_source_time: alternative.source_time,
                bond: challenge_bond(UPDATE_MIN_BOND),
                evidence_hash: alternative.evidence,
                archive_url: String::new(),
            },
            &alternative.url,
        ),
        &[&challenger],
    )
    .await
    .unwrap();
    let a_challenge = instruction::find_oracle_update_challenge_address(
        &program(),
        &cohort.month,
        &a.key,
        &[10; 32],
    );
    let a_guard = instruction::find_oracle_update_guard_address(&program(), &cohort.month, &a.key);
    h.finalize(
        &cohort,
        &source,
        0,
        &a.key,
        OracleUpdateClaimOutcome::RuleReviewUnresolved,
        1,
        UpdateFinalization::Unresolved {
            challenge: a_challenge,
        },
    )
    .await
    .unwrap();
    let pending: OracleUpdateClaimV2 = h.read_state(a.key).await;
    assert!(pending.council_review_pending);
    let cancel_a = |h: &Harness| {
        instruction::cancel_stale_oracle_update_claim_v2(
            &program(),
            &h.ctx.payer.pubkey(),
            &cohort.period,
            &cohort.month,
            &source,
            &a.key,
            Some(&a_challenge),
        )
    };
    // While the source still stands at A's prior and no later step finalized, A is not stale.
    assert_oracle_error(
        h.send(cancel_a(&h), &[]).await,
        OracleError::OracleTimingWindowClosed,
    );

    // Claim B finalizes at step 2, so A's unresolved step is now behind the source.
    let print = h
        .print(
            &cohort,
            &claimant,
            &source,
            &source_id,
            107_000_000,
            now - 10,
        )
        .await;
    let b = h
        .commit(&cohort, &claimant, &source, &source_id, 0, [11; 32], print)
        .await
        .unwrap();
    h.advance_slot().await;
    h.reveal(&cohort, &claimant, &source, &b).await.unwrap();
    h.at(now + DAY).await;
    h.finalize(
        &cohort,
        &source,
        0,
        &b.key,
        OracleUpdateClaimOutcome::AcceptClaim,
        2,
        UpdateFinalization::Unchallenged,
    )
    .await
    .unwrap();

    // An active dispute on the guard keeps A open; clearing it lets the crank close A once.
    let mut guard: OracleUpdateChallengeGuard = h.read_state(a_guard).await;
    let cleared = guard.clone();
    guard.active_dispute = Pubkey::new_unique();
    h.set_program_state(a_guard, &guard, OracleUpdateChallengeGuard::LEN)
        .await;
    assert_oracle_error(
        h.send(cancel_a(&h), &[]).await,
        OracleError::InvalidOracleUpdateAccount,
    );
    h.set_program_state(a_guard, &cleared, OracleUpdateChallengeGuard::LEN)
        .await;
    h.advance_slot().await;
    h.send(cancel_a(&h), &[]).await.unwrap();
    let closed: OracleUpdateClaimV2 = h.read_state(a.key).await;
    assert_eq!(closed.claim.status, OracleClaimStatus::TimedOut);
    assert!(!closed.council_review_pending);
    let cancelled: OracleUpdateChallenge = h.read_state(a_challenge).await;
    assert_eq!(cancelled.status, OracleChallengeStatus::Cancelled);
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.pending_resolution_count, 0);
    h.advance_slot().await;
    assert!(h.send(cancel_a(&h), &[]).await.is_err());
}

#[tokio::test]
async fn acceptance_without_its_exact_checkpoint_rolls_back_entirely() {
    let mut h = start().await;
    let (cohort, formed) = live(&mut h).await;
    let (source, source_id) = formed.sources[0];
    let claimant = h.participant(PARTICIPANT_USDC).await;
    let now = cohort.listing + 300;
    h.at(now).await;
    let print = h
        .print(
            &cohort,
            &claimant,
            &source,
            &source_id,
            108_000_000,
            now - 30,
        )
        .await;
    let claim = h
        .commit(&cohort, &claimant, &source, &source_id, 0, [12; 32], print)
        .await
        .unwrap();
    h.advance_slot().await;
    h.reveal(&cohort, &claimant, &source, &claim).await.unwrap();
    h.at(now + DAY).await;

    let oracle = h.oracle_authority.insecure_clone();
    let build = |checkpoint: Pubkey| {
        let mut ix = instruction::finalize_oracle_update_claim_v2(
            &program(),
            &oracle.pubkey(),
            &cohort.period,
            &cohort.month,
            &source,
            &cohort.sku_ids[0],
            &claim.key,
            FinalizeOracleUpdateClaimV2Params {
                outcome: OracleUpdateClaimOutcome::AcceptClaim,
                current_step: 1,
            },
            UpdateFinalization::Unchallenged,
        );
        let at = ix.accounts.len() - 2;
        ix.accounts[at].pubkey = checkpoint;
        ix
    };
    let before = (
        h.account(source).await.unwrap().data,
        h.account(claim.key).await.unwrap().data,
        h.account(cohort.month).await.unwrap().data,
    );
    // A checkpoint keyed to another event, or no fresh account at all, fails the acceptance.
    for wrong in [
        instruction::find_checkpoint_address(&program(), &source, &Pubkey::new_unique()),
        Pubkey::new_unique(),
    ] {
        assert!(h.send(build(wrong), &[&oracle]).await.is_err());
        assert_eq!(
            before,
            (
                h.account(source).await.unwrap().data,
                h.account(claim.key).await.unwrap().data,
                h.account(cohort.month).await.unwrap().data,
            )
        );
        h.advance_slot().await;
    }
    let exact = instruction::find_checkpoint_address(&program(), &source, &claim.key);
    h.send(build(exact), &[&oracle]).await.unwrap();
    let updated: OracleSourceState = h.read_state(source).await;
    assert_eq!(updated.current_state, 108_000_000);
}
