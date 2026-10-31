//! Ending a month that never listed at once (tag 30 selector 25, `processor::failed_month`; the
//! user, 2026-10-30, for mainnet: "dont hide stuff, just annihilate. no dec 1st expiry
//! markets!"): the admin fails the Forming period before its month's submission deadline, the
//! failure lanes (191, 194, 195) refund every bond and support to its own owner at once, and
//! the admin closes the month's accounts into a recipient.
mod common;

use ameba_oracle::instruction::InitializeOracleMonthV5Params;
use ameba_oracle::{
    error::OracleError,
    instruction::{self, PrelistingEscrow},
    state::{
        derive_oracle_sku_coverage_manifest_pda, derive_oracle_sku_coverage_record_pda,
        derive_oracle_usdc_reward_schedule_pda, derive_oracle_usdc_sku_pool_pda,
        derive_oracle_usdc_source_reward_pda, OracleUsdcRewardSchedule,
        OracleUsdcRewardSchedulePhase,
    },
};
use ameba_oracle_interface::{find_period_address, PeriodStatus};
use common::{cohort::*, settle::*, *};
use solana_sdk::{
    account::AccountSharedData,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

fn source_id(n: u8) -> [u8; 32] {
    [n; 32]
}

/// Tag 30 selector 25 signed by `admin`.
fn failed_month(
    cohort: &Cohort,
    admin: &Pubkey,
    recipient: &Pubkey,
    kind: u8,
    target: Option<&Pubkey>,
    parent: Option<&Pubkey>,
) -> solana_sdk::instruction::Instruction {
    instruction::failed_month(
        &program(),
        admin,
        &cohort.product,
        &cohort.period,
        &cohort.month,
        recipient,
        kind,
        target,
        parent,
    )
}

#[tokio::test]
async fn the_admin_ends_a_month_that_never_listed() {
    let mut h = start().await;
    let cohort = h.open_cohort(2).await;
    h.fund_schedule(&cohort).await;
    h.at(cohort.scramble).await;
    let admin = h.admin.insecure_clone();
    let recipient = Keypair::new().pubkey();
    let proposer = h.participant(PARTICIPANT_USDC).await;
    let supporters = [
        h.participant(PARTICIPANT_USDC).await,
        h.participant(PARTICIPANT_USDC).await,
    ];
    let mut sources = Vec::new();
    for (n, supporter) in supporters.iter().enumerate() {
        let id = source_id(n as u8 + 1);
        let propose = h.propose_ix(&cohort, &proposer.pubkey(), id, n as u16);
        let links = [propose.accounts[13].pubkey, propose.accounts[15].pubkey];
        let source = h
            .propose_source(&cohort, &proposer, id, n as u16)
            .await
            .unwrap();
        h.support_source(&cohort, supporter, &source, n as u16)
            .await
            .unwrap();
        sources.push((source, links, n));
    }
    let late = h.participant(PARTICIPANT_USDC).await;
    let vault = h.bond_vault();
    let vault_before = h.token_balance(vault).await;
    let owners: Vec<Pubkey> = std::iter::once(proposer.pubkey())
        .chain(supporters.iter().map(Keypair::pubkey))
        .collect();
    let mut held = 0;
    for owner in &owners {
        held += PARTICIPANT_USDC - h.collateral(owner).await.available_balance;
    }
    assert_eq!(held, 2 * LISTING_BOND + 2 * SUPPORT_BOND);

    // Only the admin fails it early; the ordinary crank still waits for the deadline.
    let cranker = h.payer();
    let ix = instruction::fail_period(
        &program(),
        &cranker.pubkey(),
        &cohort.product,
        cohort.expiry,
    );
    assert_oracle_error(h.send(ix, &[]).await, OracleError::PeriodNotFailable);
    let stranger = Keypair::new();
    h.fund(&stranger.pubkey(), 1_000_000_000).await;
    let ix = failed_month(&cohort, &stranger.pubkey(), &recipient, 0, None, None);
    assert_oracle_error(h.send(ix, &[&stranger]).await, OracleError::Unauthorized);
    // A Live period, and a listed one, are refused.
    let period_key = cohort.period;
    let original = h.account(period_key).await.unwrap();
    let period = h.period(cohort.product, cohort.expiry).await;
    for status in [PeriodStatus::Live, PeriodStatus::Forming] {
        let listed = ameba_oracle_interface::Period {
            status,
            effective_listing_ts: cohort.listing,
            ..period.clone()
        };
        let mut account = original.clone();
        account.data.copy_from_slice(&listed.encode());
        h.ctx
            .set_account(&period_key, &AccountSharedData::from(account));
        let ix = failed_month(&cohort, &admin.pubkey(), &recipient, 0, None, None);
        assert!(h.send(ix, &[&admin]).await.is_err(), "{status:?} listed");
        h.advance_slot().await;
    }
    h.ctx
        .set_account(&period_key, &AccountSharedData::from(original));
    // Cleanup waits for the failure.
    let (source, _, _) = sources[0];
    let ix = failed_month(&cohort, &admin.pubkey(), &recipient, 5, Some(&source), None);
    assert_oracle_error(
        h.send(ix, &[&admin]).await,
        OracleError::InvalidPeriodTransition,
    );

    let ix = failed_month(&cohort, &admin.pubkey(), &recipient, 0, None, None);
    h.send(ix, &[&admin]).await.unwrap();
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Failed
    );
    // A failed period takes no new source.
    assert!(h
        .propose_source(&cohort, &late, source_id(9), 0)
        .await
        .is_err());
    // Nothing closes while escrow is outstanding.
    let ix = failed_month(&cohort, &admin.pubkey(), &recipient, 5, Some(&source), None);
    assert_oracle_error(
        h.send(ix, &[&admin]).await,
        OracleError::InvalidOracleUsdcRewardSchedule,
    );

    // The failure lanes refund at once, each escrow to its own owner.
    for (n, (source, _, sku)) in sources.iter().enumerate() {
        h.send(
            instruction::expire_unlistable_oracle_source_v2(
                &program(),
                &cranker.pubkey(),
                &cohort.period,
                &cohort.month,
                source,
                Some(&cohort.sku_ids[*sku]),
            ),
            &[],
        )
        .await
        .unwrap();
        for escrow in [
            PrelistingEscrow::ListingBond {
                source: *source,
                proposer: proposer.pubkey(),
            },
            PrelistingEscrow::SupportStake {
                source: *source,
                supporter: supporters[n].pubkey(),
            },
        ] {
            h.send(
                instruction::settle_failed_oracle_month_escrow_v2(
                    &program(),
                    &cranker.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    &escrow,
                ),
                &[],
            )
            .await
            .unwrap();
        }
    }
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
    for owner in &owners {
        assert_eq!(
            h.collateral(owner).await.available_balance,
            PARTICIPANT_USDC
        );
    }
    // USDC never left the bond vault: the refunds are ledger credits.
    assert_eq!(h.token_balance(vault).await, vault_before);
    let schedule_key = derive_oracle_usdc_reward_schedule_pda(&program(), &cohort.month).0;
    let schedule: OracleUsdcRewardSchedule = h.read_state(schedule_key).await;
    assert_eq!(
        (schedule.phase, schedule.outstanding_prelisting_escrow_count),
        (OracleUsdcRewardSchedulePhase::Aborted, 0)
    );

    // The admin closes everything the month made, into the recipient.
    let manifest = derive_oracle_sku_coverage_manifest_pda(&program(), &cohort.month).0;
    let mut steps: Vec<(u8, Pubkey, Option<Pubkey>)> = Vec::new();
    for (n, (source, links, _)) in sources.iter().enumerate() {
        let support = instruction::support_oracle_source_v3(
            &program(),
            &supporters[n].pubkey(),
            &cohort.period,
            &cohort.month,
            source,
            &cohort.sku_ids[n],
            instruction::SupportOracleSourceV3Params {
                stake: SUPPORT_BOND,
                sku_index: n as u16,
                sku_proof: cohort.sku_proof(n as u16),
            },
        )
        .accounts[8]
            .pubkey;
        steps.push((1, support, Some(*source)));
        steps.push((
            2,
            derive_oracle_usdc_source_reward_pda(&program(), &schedule_key, source).0,
            Some(*source),
        ));
        steps.push((
            3,
            instruction::find_oracle_source_observations_address(&program(), source),
            Some(*source),
        ));
        for link in links {
            steps.push((4, *link, None));
        }
        steps.push((5, *source, None));
    }
    for sku in &cohort.sku_ids {
        steps.push((
            6,
            derive_oracle_sku_coverage_record_pda(&program(), &cohort.month, sku).0,
            None,
        ));
        steps.push((
            7,
            derive_oracle_usdc_sku_pool_pda(&program(), &schedule_key, sku).0,
            None,
        ));
    }
    // The manifest waits for the schedule, the month for both.
    let ix = failed_month(
        &cohort,
        &admin.pubkey(),
        &recipient,
        9,
        Some(&manifest),
        None,
    );
    assert!(h.send(ix, &[&admin]).await.is_err());
    steps.push((8, schedule_key, None));
    steps.push((9, manifest, None));
    steps.push((10, cohort.month, Some(manifest)));
    let mut rent = 0;
    for (kind, target, parent) in steps {
        rent += h.account(target).await.expect("open").lamports;
        let ix = failed_month(
            &cohort,
            &admin.pubkey(),
            &recipient,
            kind,
            Some(&target),
            parent.as_ref(),
        );
        h.send(ix, &[&admin])
            .await
            .unwrap_or_else(|e| panic!("kind {kind}: {e}"));
        assert!(h.account(target).await.is_none(), "kind {kind} closed");
    }
    assert_eq!(h.account(recipient).await.unwrap().lamports, rent);
    // The period stays, Failed; the refunds stay paid.
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Failed
    );
    for owner in &owners {
        assert_eq!(
            h.collateral(owner).await.available_balance,
            PARTICIPANT_USDC
        );
    }
}

/// N9: kind 0 fails only a month its refund lanes can unwind. A month that froze its weights
/// and opened (here through Opening) is refused. One in Scramble after coverage (tag 184 is
/// permissionless before the heads upgrade) fails, its month back in source submission with its
/// coverage unfinalized, as 186 reopens a stalled one; it names its coverage manifest, written.
#[tokio::test]
async fn a_month_past_source_submission_cannot_fail_early() {
    use ameba_oracle::state::{OracleMonthState, OraclePhase, OracleSkuCoverageManifest};
    let mut h = start().await;
    let cohort = h.open_cohort(2).await;
    h.form_to_opening(&cohort).await;
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.phase, OraclePhase::Opening);
    assert_ne!(month.weight_scheme_version, 0);
    let admin = h.admin.insecure_clone();
    let manifest = derive_oracle_sku_coverage_manifest_pda(&program(), &cohort.month).0;
    for parent in [None, Some(&manifest)] {
        let ix = failed_month(&cohort, &admin.pubkey(), &admin.pubkey(), 0, None, parent);
        assert_oracle_error(h.send(ix, &[&admin]).await, OracleError::InvalidOraclePhase);
        h.advance_slot().await;
    }
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Forming
    );

    let mut h = start().await;
    let wide = wide_placed(&mut h, None).await;
    h.wide_coverage(&wide).await;
    let admin = h.admin.insecure_clone();
    let manifest = derive_oracle_sku_coverage_manifest_pda(&program(), &wide.cohort.month).0;
    // In Scramble it must name the coverage manifest it reopens.
    let ix = failed_month(
        &wide.cohort,
        &admin.pubkey(),
        &admin.pubkey(),
        0,
        None,
        None,
    );
    assert_oracle_error(h.send(ix, &[&admin]).await, OracleError::InvalidAccountList);
    let ix = failed_month(
        &wide.cohort,
        &admin.pubkey(),
        &admin.pubkey(),
        0,
        None,
        Some(&manifest),
    );
    h.send(ix, &[&admin]).await.unwrap();
    assert_eq!(
        h.period(wide.cohort.product, wide.cohort.expiry)
            .await
            .status,
        PeriodStatus::Failed
    );
    let month: OracleMonthState = h.read_state(wide.cohort.month).await;
    assert_eq!(month.phase, OraclePhase::SourceSubmission);
    let coverage: OracleSkuCoverageManifest = h.read_state(manifest).await;
    assert!(!coverage.coverage_finalized);
    assert_eq!(coverage.coverage_complete_ts, 0);
    // A Failed period whose month is already in source submission has nothing left for kind 0.
    h.advance_slot().await;
    let ix = failed_month(
        &wide.cohort,
        &admin.pubkey(),
        &admin.pubkey(),
        0,
        None,
        Some(&manifest),
    );
    assert_oracle_error(h.send(ix, &[&admin]).await, OracleError::InvalidOraclePhase);
}

/// N10: the product's latest registered carry month is neither failed nor closed here: the next
/// rung could register against neither it nor its parent.
#[tokio::test]
async fn the_latest_carry_month_cannot_fail_early() {
    let mut h = start().await;
    let parent = h.open_cohort(2).await;
    h.form_to_opening(&parent).await;
    h.at(parent.listing).await;
    let cranker = h.payer();
    h.send(
        instruction::finalize_oracle_opening_phase(
            &program(),
            &cranker.pubkey(),
            &parent.period,
            &parent.month,
        ),
        &[],
    )
    .await
    .unwrap();
    let oracle = h.oracle_authority.insecure_clone();
    let root = instruction::register_carry_root(
        &program(),
        &oracle.pubkey(),
        &parent.product,
        &parent.period,
        &parent.month,
    );
    h.send(root, &[&oracle]).await.unwrap();
    // B, the next rung, registers as the successor while in source submission.
    let (listing, expiry) = (JAN_01_2027, APR_01_2027);
    h.create_period(&oracle, parent.product, listing, expiry, 6)
        .await
        .unwrap();
    let scramble = listing - PROGRAM_PRE_LISTING_SECONDS;
    h.send(
        instruction::initialize_oracle_month_v5(
            &program(),
            &oracle.pubkey(),
            &h.ctx.payer.pubkey(),
            &parent.product,
            expiry,
            InitializeOracleMonthV5Params {
                scramble_start_ts: scramble,
                listing_ts: listing,
                settlement_base_oracle_atomic: BASE,
                required_sku_root: parent.root,
                required_sku_count: parent.sku_ids.len() as u16,
            },
        ),
        &[&oracle],
    )
    .await
    .unwrap();
    let period = find_period_address(&program(), &parent.product, expiry).0;
    let child = Cohort {
        product: parent.product,
        sku_ids: parent.sku_ids.clone(),
        root: parent.root,
        period,
        month: instruction::find_oracle_month_address(&program(), &period, expiry),
        listing,
        scramble,
        expiry,
    };
    h.send(
        instruction::register_carry_successor(
            &program(),
            &oracle.pubkey(),
            &parent.product,
            &child.period,
            &child.month,
            &parent.period,
            &parent.month,
        ),
        &[&oracle],
    )
    .await
    .unwrap();
    let admin = h.admin.insecure_clone();
    let ix = failed_month(&child, &admin.pubkey(), &admin.pubkey(), 0, None, None);
    assert_oracle_error(h.send(ix, &[&admin]).await, OracleError::InvalidOraclePhase);
    assert_eq!(
        h.period(child.product, child.expiry).await.status,
        PeriodStatus::Forming
    );
}

/// N8: a sponsored source closes only once its sponsorship reconciled, which pays the refunded
/// bond on to its funder (reconciliation reads the source). The reviewer's order: fail, refund
/// (191, 194), release (195), close: the close is refused until 30/22 reconciles.
#[tokio::test]
async fn a_sponsored_source_closes_only_once_its_funder_is_paid() {
    let mut h = start().await;
    let researcher = Keypair::new();
    let terms = ameba_oracle::sponsorship::SponsorTerms {
        agent_id: [0xa1; 32],
        researcher: researcher.pubkey(),
        candidate_hash: WIDE_SOURCES[1],
        funding: 0,
        researcher_bps: ameba_oracle::sponsorship::DEFAULT_RESEARCHER_BPS,
        retention_bps: ameba_oracle::sponsorship::DEFAULT_RETENTION_BPS,
        maximum_bond: LISTING_BOND,
        maximum_outstanding: 2 * LISTING_BOND,
    };
    let wide = wide_placed(&mut h, Some(terms.clone())).await;
    let cohort = wide.cohort.clone();
    let sponsor = wide.sponsor.as_ref().unwrap().insecure_clone();
    let source = wide.sources[1];
    let sponsored =
        instruction::SponsoredAccounts::new(&program(), &terms, &source, Some(&sponsor.pubkey()));
    let admin = h.admin.insecure_clone();
    let recipient = Keypair::new().pubkey();
    let ix = failed_month(&cohort, &admin.pubkey(), &recipient, 0, None, None);
    h.send(ix, &[&admin]).await.unwrap();
    let cranker = h.payer();
    for (n, source) in wide.sources.iter().enumerate() {
        h.send(
            instruction::expire_unlistable_oracle_source_v2(
                &program(),
                &cranker.pubkey(),
                &cohort.period,
                &cohort.month,
                source,
                Some(&cohort.sku_ids[0]),
            ),
            &[],
        )
        .await
        .unwrap();
        for escrow in [
            PrelistingEscrow::ListingBond {
                source: *source,
                proposer: wide.proposers[n],
            },
            PrelistingEscrow::SupportStake {
                source: *source,
                supporter: wide.supporter.pubkey(),
            },
        ] {
            h.send(
                instruction::settle_failed_oracle_month_escrow_v2(
                    &program(),
                    &cranker.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    &escrow,
                ),
                &[],
            )
            .await
            .unwrap();
        }
    }
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
    // The bond sits in the agreement's ledger, not yet the funder's.
    assert_eq!(
        h.collateral(&sponsor.pubkey()).await.available_balance,
        PARTICIPANT_USDC - LISTING_BOND
    );
    for parent in [None, Some(&sponsored.agreement)] {
        let ix = failed_month(
            &cohort,
            &admin.pubkey(),
            &recipient,
            5,
            Some(&source),
            parent,
        );
        assert!(
            h.send(ix, &[&admin]).await.is_err(),
            "closed before reconciling"
        );
        h.advance_slot().await;
    }
    h.send(
        instruction::sponsored_reconcile(
            &program(),
            &cranker.pubkey(),
            &cohort.month,
            &source,
            &sponsored,
        ),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        h.collateral(&sponsor.pubkey()).await.available_balance,
        PARTICIPANT_USDC
    );
    let ix = failed_month(
        &cohort,
        &admin.pubkey(),
        &recipient,
        5,
        Some(&source),
        Some(&sponsored.agreement),
    );
    h.send(ix, &[&admin]).await.unwrap();
    assert!(h.account(source).await.is_none());
}

/// Early review B3, the reviewer's sequence: a sponsored source with a rejected source challenge
/// whose bond is still unpaid (the ruling keeps it Candidate and clears the pending count); the
/// stalled coverage reopens, the month fails, the source times out, its listing bond comes back
/// and the sponsorship reconciles; then the rejected challenge settles, its bond into the
/// agreement's ledger; the escrows finish and the schedule aborts. The source cannot close while
/// that ledger holds the bond: reconciling again sweeps it on to the funder, then it closes.
#[tokio::test]
async fn proceeds_after_reconciling_reach_the_funder_before_the_source_closes() {
    use ameba_oracle::instruction::ChallengeOracleSourceParams;
    use ameba_oracle::state::OracleSourceChallengeOutcome;
    let mut h = start().await;
    let researcher = Keypair::new();
    let terms = ameba_oracle::sponsorship::SponsorTerms {
        agent_id: [0xa1; 32],
        researcher: researcher.pubkey(),
        candidate_hash: WIDE_SOURCES[1],
        funding: 0,
        researcher_bps: ameba_oracle::sponsorship::DEFAULT_RESEARCHER_BPS,
        retention_bps: ameba_oracle::sponsorship::DEFAULT_RETENTION_BPS,
        maximum_bond: LISTING_BOND,
        maximum_outstanding: 2 * LISTING_BOND,
    };
    let wide = wide_placed(&mut h, Some(terms.clone())).await;
    let cohort = wide.cohort.clone();
    let sponsor = wide.sponsor.as_ref().unwrap().insecure_clone();
    let source = wide.sources[1];
    let sponsored =
        instruction::SponsoredAccounts::new(&program(), &terms, &source, Some(&sponsor.pubkey()));
    let cranker = h.payer();
    let oracle = h.oracle_authority.insecure_clone();
    // Coverage finalizes; a challenge of the sponsored source is rejected (it stays Candidate).
    h.at(cohort.placement_end()).await;
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
    let challenge_id = [0xb3; 32];
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
    h.at(cohort.kill_end()).await;
    h.send(
        instruction::resolve_oracle_source_challenge_v2(
            &program(),
            &oracle.pubkey(),
            &cohort.month,
            &source,
            &challenge,
            &cohort.sku_ids[0],
            None,
            OracleSourceChallengeOutcome::KeepSource,
        ),
        &[&oracle],
    )
    .await
    .unwrap();
    // The stalled scramble reopens; the admin fails the month.
    h.at(cohort.scramble_end()).await;
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
    let admin = h.admin.insecure_clone();
    let recipient = Keypair::new().pubkey();
    let ix = failed_month(&cohort, &admin.pubkey(), &recipient, 0, None, None);
    h.send(ix, &[&admin]).await.unwrap();
    // The sponsored source times out, its listing bond comes back, the sponsorship reconciles.
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
    let settle = |escrow: PrelistingEscrow| {
        instruction::settle_failed_oracle_month_escrow_v2(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
            &escrow,
        )
    };
    h.send(
        settle(PrelistingEscrow::ListingBond {
            source,
            proposer: sponsored.agreement,
        }),
        &[],
    )
    .await
    .unwrap();
    let reconcile = instruction::sponsored_reconcile(
        &program(),
        &cranker.pubkey(),
        &cohort.month,
        &source,
        &sponsored,
    );
    h.send(reconcile.clone(), &[]).await.unwrap();
    assert_eq!(
        h.collateral(&sponsor.pubkey()).await.available_balance,
        PARTICIPANT_USDC
    );
    // The rejected challenge settles afterwards: its bond lands in the agreement's ledger.
    h.send(
        settle(PrelistingEscrow::SourceChallenge {
            source,
            proposer: sponsored.agreement,
            challenge,
            challenger: challenger.pubkey(),
            comparison_source: None,
        }),
        &[],
    )
    .await
    .unwrap();
    let late = h.collateral(&sponsored.agreement).await.available_balance;
    assert!(
        late > 0,
        "the rejected challenge's bond went to the agreement"
    );
    // The other escrows finish; the reward reservation is released.
    for (n, other) in wide.sources.iter().enumerate() {
        if *other != source {
            h.send(
                instruction::expire_unlistable_oracle_source_v2(
                    &program(),
                    &cranker.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    other,
                    Some(&cohort.sku_ids[0]),
                ),
                &[],
            )
            .await
            .unwrap();
            h.send(
                settle(PrelistingEscrow::ListingBond {
                    source: *other,
                    proposer: wide.proposers[n],
                }),
                &[],
            )
            .await
            .unwrap();
        }
        h.send(
            settle(PrelistingEscrow::SupportStake {
                source: *other,
                supporter: wide.supporter.pubkey(),
            }),
            &[],
        )
        .await
        .unwrap();
    }
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
    // Not while the agreement's ledger holds the late bond.
    let close = failed_month(
        &cohort,
        &admin.pubkey(),
        &recipient,
        5,
        Some(&source),
        Some(&sponsored.agreement),
    );
    assert_oracle_error(
        h.send(close.clone(), &[&admin]).await,
        OracleError::OracleEscrowNotSettleable,
    );
    h.advance_slot().await;
    h.send(reconcile, &[]).await.unwrap();
    assert_eq!(
        h.collateral(&sponsor.pubkey()).await.available_balance,
        PARTICIPANT_USDC + late
    );
    assert_eq!(
        h.collateral(&sponsored.agreement).await.available_balance,
        0
    );
    h.send(close, &[&admin]).await.unwrap();
    assert!(h.account(source).await.is_none());
}

/// Early review B4, the reviewer's sequence: the admin fails a month in source submission with
/// complete but unfinalized coverage, no weights, nothing pending and a wholly unspent funded
/// schedule. Before the refunds, coverage cannot finalize into Scramble, and no budget can be
/// added: formation, funding and weights stop at the failure, so only the unwind remains, and it
/// runs.
#[tokio::test]
async fn a_failed_month_forms_no_further() {
    let mut h = start().await;
    let wide = wide_placed(&mut h, None).await;
    let cohort = wide.cohort.clone();
    let admin = h.admin.insecure_clone();
    let recipient = Keypair::new().pubkey();
    let ix = failed_month(&cohort, &admin.pubkey(), &recipient, 0, None, None);
    h.send(ix, &[&admin]).await.unwrap();
    let cranker = h.payer();
    h.at(cohort.placement_end()).await;
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
        OracleError::InvalidOraclePhase,
    );
    // The unwind still runs: a source times out (191).
    h.send(
        instruction::expire_unlistable_oracle_source_v2(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
            &wide.sources[0],
            Some(&cohort.sku_ids[0]),
        ),
        &[],
    )
    .await
    .unwrap();
}

/// Independent review of e25cd610, finding 1 (a v1 regression from B4): a month whose sources
/// were all rejected sits in Scramble with nothing left to form, and at expiry tag 79 fails its
/// period. Its stalled coverage still reopens (186), the one way back to source submission, so
/// the escrows settle on the failure lanes and 195 releases the reward reservation; no source,
/// support or coverage forms after.
#[tokio::test]
async fn a_month_failed_after_rejections_still_releases_its_rewards() {
    use ameba_oracle::instruction::ChallengeOracleSourceParams;
    use ameba_oracle::state::{
        derive_oracle_usdc_reward_vault_pda, OracleMonthState, OraclePhase,
        OracleSourceChallengeOutcome, OracleUsdcRewardVault,
    };
    let mut h = start().await;
    let cohort = h.open_cohort(2).await;
    h.fund_schedule(&cohort).await;
    let vault_key = derive_oracle_usdc_reward_vault_pda(&program()).0;
    let reserved: OracleUsdcRewardVault = h.read_state(vault_key).await;
    let schedule_key = derive_oracle_usdc_reward_schedule_pda(&program(), &cohort.month).0;
    let budget = h
        .read_state::<OracleUsdcRewardSchedule>(schedule_key)
        .await
        .total_reward_budget;
    assert!(budget > 0);
    h.at(cohort.scramble).await;
    let proposer = h.participant(PARTICIPANT_USDC).await;
    let supporter = h.participant(PARTICIPANT_USDC).await;
    let mut sources = Vec::new();
    for sku in 0..2u16 {
        let source = h
            .propose_source(&cohort, &proposer, source_id(sku as u8 + 1), sku)
            .await
            .unwrap();
        h.support_source(&cohort, &supporter, &source, sku)
            .await
            .unwrap();
        sources.push(source);
    }
    let cranker = h.payer();
    let crank = |ix: fn(&Pubkey, &Pubkey, &Pubkey, &Pubkey) -> _| {
        ix(&program(), &cranker.pubkey(), &cohort.period, &cohort.month)
    };
    h.at(cohort.placement_end()).await;
    h.send(crank(instruction::finalize_oracle_sku_coverage), &[])
        .await
        .unwrap();
    // Both sources are challenged and rejected: Scramble, no weights, nothing left to form.
    let challenger = h.participant(PARTICIPANT_USDC).await;
    let mut challenges = Vec::new();
    for (n, source) in sources.iter().enumerate() {
        let challenge_id = [0xc0 + n as u8; 32];
        h.send(
            instruction::challenge_oracle_source_v2(
                &program(),
                &challenger.pubkey(),
                &cohort.period,
                &cohort.month,
                source,
                &cohort.sku_ids[n],
                None,
                ChallengeOracleSourceParams {
                    challenge_id,
                    reason: 1,
                    bond: CHALLENGE_MIN_BOND,
                    evidence_hash: [0xe0 + n as u8; 32],
                    comparison_source_id: [0; 32],
                },
            ),
            &[&challenger],
        )
        .await
        .unwrap();
        challenges.push(instruction::find_oracle_source_challenge_address(
            &program(),
            &cohort.month,
            source,
            &challenge_id,
        ));
    }
    h.at(cohort.kill_end()).await;
    let oracle = h.oracle_authority.insecure_clone();
    for (n, source) in sources.iter().enumerate() {
        h.send(
            instruction::resolve_oracle_source_challenge_v2(
                &program(),
                &oracle.pubkey(),
                &cohort.month,
                source,
                &challenges[n],
                &cohort.sku_ids[n],
                None,
                OracleSourceChallengeOutcome::RejectSource,
            ),
            &[&oracle],
        )
        .await
        .unwrap();
    }
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(
        (month.phase, month.source_count, month.weight_scheme_version),
        (OraclePhase::Scramble, 0, 0)
    );
    // At expiry the period fails on its own (79); the schedule cannot abort from Scramble.
    h.at(cohort.expiry).await;
    h.send(
        instruction::fail_period(
            &program(),
            &cranker.pubkey(),
            &cohort.product,
            cohort.expiry,
        ),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Failed
    );
    let abort = crank(instruction::abort_oracle_usdc_reward_schedule_v2);
    assert_oracle_error(
        h.send(abort.clone(), &[]).await,
        OracleError::InvalidOraclePhase,
    );
    // The stalled coverage reopens: the terminal way back to source submission.
    h.send(crank(instruction::reopen_oracle_sku_coverage), &[])
        .await
        .unwrap();
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.phase, OraclePhase::SourceSubmission);
    // Nothing forms after: no source, no support, no coverage.
    let late = h.participant(PARTICIPANT_USDC).await;
    assert!(h
        .propose_source(&cohort, &late, source_id(9), 0)
        .await
        .is_err());
    assert!(h
        .support_source(&cohort, &late, &sources[0], 0)
        .await
        .is_err());
    assert_oracle_error(
        h.send(crank(instruction::finalize_oracle_sku_coverage), &[])
            .await,
        OracleError::InvalidOraclePhase,
    );
    // The escrows settle on the failure lanes, then the reservation is released.
    for (n, source) in sources.iter().enumerate() {
        for escrow in [
            PrelistingEscrow::SourceChallenge {
                source: *source,
                proposer: proposer.pubkey(),
                challenge: challenges[n],
                challenger: challenger.pubkey(),
                comparison_source: None,
            },
            PrelistingEscrow::SupportStake {
                source: *source,
                supporter: supporter.pubkey(),
            },
        ] {
            h.send(
                instruction::settle_failed_oracle_month_escrow_v2(
                    &program(),
                    &cranker.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    &escrow,
                ),
                &[],
            )
            .await
            .unwrap();
        }
    }
    h.send(abort, &[]).await.unwrap();
    let schedule: OracleUsdcRewardSchedule = h.read_state(schedule_key).await;
    assert_eq!(schedule.phase, OracleUsdcRewardSchedulePhase::Aborted);
    let released: OracleUsdcRewardVault = h.read_state(vault_key).await;
    assert_eq!(released.total_reserved, reserved.total_reserved - budget);
    assert_eq!(
        h.collateral(&supporter.pubkey()).await.available_balance,
        PARTICIPANT_USDC
    );
}

/// Independent review of e25cd610, finding 2: no order of the admin's closes strands rent. The
/// month and a source close before their children here (the schedule, the coverage manifest and
/// the month first, then the source before its support, reward and observations), and every
/// child still closes into the recipient; the schedule still closes before the coverage
/// manifest, the manifest before the month, and a child names its own parent, closed or not.
#[tokio::test]
async fn the_cleanup_strands_nothing_whatever_its_order() {
    let mut h = start().await;
    let cohort = h.open_cohort(1).await;
    h.fund_schedule(&cohort).await;
    h.at(cohort.scramble).await;
    let admin = h.admin.insecure_clone();
    let recipient = Keypair::new().pubkey();
    let proposer = h.participant(PARTICIPANT_USDC).await;
    let supporter = h.participant(PARTICIPANT_USDC).await;
    let id = source_id(1);
    let links: Vec<Pubkey> = {
        let propose = h.propose_ix(&cohort, &proposer.pubkey(), id, 0);
        vec![propose.accounts[13].pubkey, propose.accounts[15].pubkey]
    };
    let source = h.propose_source(&cohort, &proposer, id, 0).await.unwrap();
    h.support_source(&cohort, &supporter, &source, 0)
        .await
        .unwrap();
    let ix = failed_month(&cohort, &admin.pubkey(), &recipient, 0, None, None);
    h.send(ix, &[&admin]).await.unwrap();
    let cranker = h.payer();
    let settle = |escrow: PrelistingEscrow| {
        instruction::settle_failed_oracle_month_escrow_v2(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
            &escrow,
        )
    };
    for ix in [
        instruction::expire_unlistable_oracle_source_v2(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
            &source,
            Some(&cohort.sku_ids[0]),
        ),
        settle(PrelistingEscrow::ListingBond {
            source,
            proposer: proposer.pubkey(),
        }),
        settle(PrelistingEscrow::SupportStake {
            source,
            supporter: supporter.pubkey(),
        }),
        instruction::abort_oracle_usdc_reward_schedule_v2(
            &program(),
            &cranker.pubkey(),
            &cohort.period,
            &cohort.month,
        ),
    ] {
        h.send(ix, &[]).await.unwrap();
    }

    let schedule = derive_oracle_usdc_reward_schedule_pda(&program(), &cohort.month).0;
    let manifest = derive_oracle_sku_coverage_manifest_pda(&program(), &cohort.month).0;
    let support = instruction::find_oracle_support_address(
        &program(),
        &cohort.month,
        &source,
        &supporter.pubkey(),
    );
    let reward = derive_oracle_usdc_source_reward_pda(&program(), &schedule, &source).0;
    let observations = instruction::find_oracle_source_observations_address(&program(), &source);
    let record =
        derive_oracle_sku_coverage_record_pda(&program(), &cohort.month, &cohort.sku_ids[0]).0;
    let pool = derive_oracle_usdc_sku_pool_pda(&program(), &schedule, &cohort.sku_ids[0]).0;
    let close = |kind: u8, target: &Pubkey, parent: Option<&Pubkey>| {
        failed_month(
            &cohort,
            &admin.pubkey(),
            &recipient,
            kind,
            Some(target),
            parent,
        )
    };
    // What still orders: the manifest after the schedule, the month after both.
    assert!(h
        .send(close(10, &cohort.month, Some(&manifest)), &[&admin])
        .await
        .is_err());
    assert!(h.send(close(9, &manifest, None), &[&admin]).await.is_err());
    // Parents first: the schedule, the coverage manifest, the month, then the source.
    let mut rent = close_all(
        &mut h,
        &cohort,
        &admin,
        &recipient,
        vec![
            (8, schedule, None),
            (9, manifest, None),
            (10, cohort.month, Some(manifest)),
            (5, source, None),
        ],
    )
    .await;
    // A child closes only under its own parent, gone or not.
    let stranger = Keypair::new().pubkey();
    for (kind, target) in [(1, support), (2, reward), (3, observations)] {
        h.advance_slot().await;
        let wrong = close(kind, &target, Some(&stranger));
        assert!(
            h.send(wrong, &[&admin]).await.is_err(),
            "kind {kind} under a stranger"
        );
    }
    let mut children: Vec<(u8, Pubkey, Option<Pubkey>)> = vec![
        (1, support, Some(source)),
        (2, reward, Some(source)),
        (3, observations, Some(source)),
    ];
    children.extend(links.iter().map(|link| (4, *link, None)));
    children.push((6, record, None));
    children.push((7, pool, None));
    rent += close_all(&mut h, &cohort, &admin, &recipient, children).await;
    assert_eq!(h.account(recipient).await.unwrap().lamports, rent);
    for owner in [proposer.pubkey(), supporter.pubkey()] {
        assert_eq!(
            h.collateral(&owner).await.available_balance,
            PARTICIPANT_USDC
        );
    }
}

/// The admin's closes (30/25 kinds 1-12), in order, each into `recipient`: the rent they free.
async fn close_all(
    h: &mut Harness,
    cohort: &Cohort,
    admin: &Keypair,
    recipient: &Pubkey,
    steps: Vec<(u8, Pubkey, Option<Pubkey>)>,
) -> u64 {
    let mut rent = 0;
    for (kind, target, parent) in steps {
        rent += h.account(target).await.expect("open").lamports;
        h.advance_slot().await;
        let ix = failed_month(
            cohort,
            &admin.pubkey(),
            recipient,
            kind,
            Some(&target),
            parent.as_ref(),
        );
        h.send(ix, &[admin])
            .await
            .unwrap_or_else(|e| panic!("kind {kind}: {e}"));
        assert!(h.account(target).await.is_none(), "kind {kind} closed");
    }
    rent
}
