//! End to end on real instructions from an empty program with a warped clock. One cohort runs
//! from CreatePeriod through formation, listing, Game updates, finalization, attested settlement,
//! cash rewards and terminal cleanup, while the product's next rung fails. The consumer contract
//! is read back at every Period transition, and every USDC unit is accounted for at the end.
mod common;

use ameba_oracle::{
    constants::{
        ORACLE_KILL_WINDOW_SECONDS, ORACLE_OPENING_WINDOW_SECONDS,
        ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS, ORACLE_SETTLEMENT_FRESHNESS_BUSINESS_DAYS,
        ORACLE_SETTLEMENT_GRACE_SECONDS,
    },
    error::OracleError,
    instruction::{self, InitializeOracleMonthV5Params, PrelistingEscrow, UpdateFinalization},
    processor::oracle_settlement_window_start,
    state::{
        derive_oracle_bucket_median_pda, derive_oracle_usdc_reward_registration_pda,
        derive_oracle_usdc_reward_schedule_pda, derive_oracle_usdc_reward_vault_pda,
        derive_oracle_usdc_sku_pool_pda, derive_oracle_usdc_source_reward_pda,
        derive_user_collateral_pda, OracleEscrowKind, OracleMonthState, OraclePhase,
        OracleSettlementStatus, OracleSourceState, OracleUpdateClaimOutcome, OracleUsdcRewardKind,
        OracleUsdcRewardSchedule, OracleUsdcRewardSchedulePhase, OracleUsdcRewardVault,
    },
};
use ameba_oracle_interface::{find_period_address, Period, PeriodStatus};
use common::{cohort::*, settle::*, *};
use solana_sdk::{
    instruction::AccountMeta,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

const DAY: u64 = 86_400;

impl Harness {
    /// The consumer contract's view of a period, checked against its raw bytes.
    async fn consumer_view(&mut self, cohort: &Cohort) -> Period {
        let period = self.period(cohort.product, cohort.expiry).await;
        let raw = self
            .account(find_period_address(&program(), &cohort.product, cohort.expiry).0)
            .await
            .unwrap();
        assert_eq!(Period::decode(&raw.data).unwrap(), period);
        period
    }

    /// Tag 181 for the product's next rung (its period already exists) and its funded schedule.
    async fn open_next_rung(&mut self, first: &Cohort, planned: u64, expiry: u64) -> Cohort {
        let oracle = self.oracle_authority.insecure_clone();
        let payer = self.payer();
        let scramble = planned - PROGRAM_PRE_LISTING_SECONDS;
        self.send(
            instruction::initialize_oracle_month_v5(
                &program(),
                &oracle.pubkey(),
                &payer.pubkey(),
                &first.product,
                expiry,
                InitializeOracleMonthV5Params {
                    scramble_start_ts: scramble,
                    listing_ts: planned,
                    settlement_base_oracle_atomic: BASE,
                    required_sku_root: first.root,
                    required_sku_count: first.sku_ids.len() as u16,
                },
            ),
            &[&oracle],
        )
        .await
        .unwrap();
        let period = find_period_address(&program(), &first.product, expiry).0;
        let next = Cohort {
            product: first.product,
            sku_ids: first.sku_ids.clone(),
            root: first.root,
            period,
            month: instruction::find_oracle_month_address(&program(), &period, expiry),
            listing: planned,
            scramble,
            expiry,
        };
        self.fund_schedule(&next).await;
        next
    }

    /// Tag 169 for an accepted opening claim's or update claim's stake.
    async fn refund_claim_stake(
        &mut self,
        cohort: &Cohort,
        kind: OracleEscrowKind,
        source: &Pubkey,
        claim: &Pubkey,
        claimant: &Pubkey,
    ) {
        let cranker = self.payer();
        let mut accounts = vec![
            AccountMeta::new_readonly(cranker.pubkey(), true),
            AccountMeta::new_readonly(cohort.period, false),
            AccountMeta::new_readonly(cohort.month, false),
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new(*claim, false),
            AccountMeta::new(derive_user_collateral_pda(&program(), claimant).0, false),
        ];
        if kind == OracleEscrowKind::UpdateClaim {
            accounts.push(AccountMeta::new_readonly(
                instruction::find_oracle_update_guard_address(&program(), &cohort.month, claim),
                false,
            ));
        }
        self.send(
            instruction::settle_oracle_usdc_escrow(&program(), kind, accounts),
            &[],
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn a_cohort_settles_end_to_end_while_the_next_rung_fails() {
    let mut h = start().await;

    // Tags 0, 128 split, 90, 129, 110, 190, 1, 181, 154-158, then placement (182, 183).
    let wide = wide_placed(&mut h, None).await;
    let cohort = &wide.cohort;
    let bucket = cohort.sku_ids[0];
    let forming = h.consumer_view(cohort).await;
    assert_eq!(forming.status, PeriodStatus::Forming);
    assert_eq!(forming.planned_listing_ts, cohort.listing);
    assert_eq!(forming.effective_listing_ts, 0);
    assert_eq!(forming.settlement(), None);
    assert!(!forming.is_tradable_at(cohort.listing));

    // The product's next rung: its period exists now, its month waits for this listing.
    let oracle = h.oracle_authority.insecure_clone();
    h.create_period(&oracle, cohort.product, JAN_01_2027, APR_01_2027, 6)
        .await
        .unwrap();

    // Coverage (184), recipe (187, 84, 86, 200), openings (162, 165) and active weights
    // (121, 122, 124): the period stays Forming throughout.
    h.wide_coverage(&wide).await;
    assert_eq!(h.consumer_view(cohort).await, forming);
    h.wide_recipe(&wide).await;
    h.wide_openings(&wide).await;
    h.wide_active_weights(&wide).await;
    assert_eq!(h.consumer_view(cohort).await, forming);

    // Tag 80 is refused one second before the listing and lists the period at it.
    let payer = h.payer();
    h.at(cohort.listing - 1).await;
    assert_oracle_error(
        h.send(
            instruction::finalize_oracle_opening_phase(
                &program(),
                &payer.pubkey(),
                &cohort.period,
                &cohort.month,
            ),
            &[],
        )
        .await,
        OracleError::OracleTimingWindowClosed,
    );
    assert_eq!(h.consumer_view(cohort).await, forming);
    h.wide_list(&wide).await;
    let live = h.consumer_view(cohort).await;
    assert_eq!(live.status, PeriodStatus::Live);
    assert_eq!(live.effective_listing_ts, cohort.listing);
    assert_eq!(live.settlement(), None);
    assert!(live.is_tradable_at(cohort.listing));
    assert!(live.is_tradable_at(cohort.expiry - 1));
    assert!(!live.is_tradable_at(cohort.expiry));

    // The next rung's month opens now that this listing has arrived; one source is proposed
    // in its placement and nobody supports it.
    let next = h.open_next_rung(cohort, JAN_01_2027, APR_01_2027).await;
    let late = h.participant(PARTICIPANT_USDC).await;
    h.at(next.scramble).await;
    let orphan = h.propose_source(&next, &late, [40; 32], 0).await.unwrap();
    assert_eq!(
        h.collateral(&late.pubkey()).await.available_balance,
        PARTICIPANT_USDC - LISTING_BOND
    );
    assert_eq!(h.consumer_view(&next).await.status, PeriodStatus::Forming);

    // Game: two accepted updates inside the settlement window move two of the three sources,
    // by +100 and +200 bps.
    let window =
        oracle_settlement_window_start(cohort.expiry, ORACLE_SETTLEMENT_FRESHNESS_BUSINESS_DAYS)
            .unwrap();
    let updater = h.participant(PARTICIPANT_USDC).await;
    let now = window + DAY;
    h.at(now).await;
    let mut updates = Vec::new();
    for (index, state, claim_id) in [
        (0usize, 101_000_000u64, [21; 32]),
        (1, 104_040_000, [22; 32]),
    ] {
        let source = wide.sources[index];
        let print = h
            .print(
                cohort,
                &updater,
                &source,
                &WIDE_SOURCES[index],
                state,
                now - 60,
            )
            .await;
        let claim = h
            .commit(
                cohort,
                &updater,
                &source,
                &WIDE_SOURCES[index],
                0,
                claim_id,
                print,
            )
            .await
            .unwrap();
        h.advance_slot().await;
        h.reveal(cohort, &updater, &source, &claim).await.unwrap();
        updates.push((source, claim));
    }
    h.at(now + DAY).await;
    for (source, claim) in &updates {
        h.finalize(
            cohort,
            source,
            0,
            &claim.key,
            OracleUpdateClaimOutcome::AcceptClaim,
            1,
            UpdateFinalization::Unchallenged,
        )
        .await
        .unwrap();
        let moved: OracleSourceState = h.read_state(*source).await;
        assert_eq!(moved.current_state, claim.print.state);
    }
    // The month's index moves only in the post-expiry median walk; the period is unchanged.
    let game: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(game.phase, OraclePhase::Game);
    assert_eq!(game.index_delta_bps, 0);
    assert_eq!(h.consumer_view(cohort).await, live);

    // After expiry and grace: 197 per source, 75 and 113. The bucket median of the source moves
    // (+100, +200, 0 bps) is +100 bps at the full bucket weight.
    h.at(cohort.expiry).await;
    assert_eq!(h.consumer_view(cohort).await, live);
    let settled_at = cohort.expiry + ORACLE_SETTLEMENT_GRACE_SECONDS;
    h.at(settled_at).await;
    h.finalize_wide(&wide).await;
    let finalized: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(finalized.settlement_status, OracleSettlementStatus::Final);
    assert_eq!(finalized.index_delta_bps, 100);
    assert_eq!(h.consumer_view(cohort).await, live);

    // 116 with three of five signer attestations: the period is Settled at
    // round(100.000000 × 1.01) = 101.000000.
    let leaf = h.leaf_for(cohort).await;
    assert_eq!(leaf.settlement_price_atomic, 101_000_000);
    let signers: Vec<Keypair> = h
        .settlement_signers
        .iter()
        .map(|key| key.insecure_clone())
        .collect();
    h.publish(
        cohort,
        &[&signers[0], &signers[1], &signers[2]],
        &leaf,
        &leaf,
    )
    .await
    .unwrap();
    let settled = h.consumer_view(cohort).await;
    assert_eq!(settled.status, PeriodStatus::Settled);
    assert_eq!(settled.effective_listing_ts, cohort.listing);
    let settlement = settled.settlement().expect("Settled");
    assert_eq!(settlement.value, 101_000_000);
    assert_eq!(settlement.decimals, 6);
    assert_eq!(settlement.settled_ts, settled_at);
    assert_eq!(settlement.digest, record_commitment(&leaf));

    // Every bond and stake returns: listing bonds and support stakes, the opening claims' and
    // the update claims' stakes.
    h.settle_wide_escrows(&wide).await;
    for source in &wide.sources {
        let claim =
            instruction::find_oracle_opening_claim_address(&program(), &cohort.month, source);
        h.refund_claim_stake(
            cohort,
            OracleEscrowKind::OpeningClaim,
            source,
            &claim,
            &wide.claimant.pubkey(),
        )
        .await;
    }
    for (source, claim) in &updates {
        h.refund_claim_stake(
            cohort,
            OracleEscrowKind::UpdateClaim,
            source,
            &claim.key,
            &updater.pubkey(),
        )
        .await;
    }

    // Rewards: register every source and update (170, 171), freeze (173), claim every kind (174).
    for source in &wide.sources {
        let opening =
            instruction::find_oracle_opening_claim_address(&program(), &cohort.month, source);
        h.send(
            instruction::register_oracle_usdc_reward_source(
                &program(),
                &payer.pubkey(),
                &cohort.month,
                &bucket,
                source,
                Some(&opening),
            ),
            &[],
        )
        .await
        .unwrap();
    }
    for (source, claim) in &updates {
        h.send(
            instruction::register_oracle_usdc_reward_update(
                &program(),
                &payer.pubkey(),
                &cohort.month,
                &bucket,
                source,
                &claim.key,
            ),
            &[],
        )
        .await
        .unwrap();
    }
    h.send(
        instruction::finalize_oracle_usdc_reward_entitlements(
            &program(),
            &oracle.pubkey(),
            &cohort.period,
            &cohort.month,
        ),
        &[&oracle],
    )
    .await
    .unwrap();
    let schedule = derive_oracle_usdc_reward_schedule_pda(&program(), &cohort.month).0;
    let sku = derive_oracle_usdc_sku_pool_pda(&program(), &schedule, &bucket).0;
    let proposer = wide.proposer.insecure_clone();
    let supporter = wide.supporter.insecure_clone();
    let claimant = wide.claimant.insecure_clone();
    let (mut proposer_paid, mut supporter_paid, mut opening_paid, mut update_paid) = (0, 0, 0, 0);
    for source in &wide.sources {
        let reward = derive_oracle_usdc_source_reward_pda(&program(), &schedule, source).0;
        proposer_paid += h
            .claim_reward(
                cohort,
                &proposer,
                OracleUsdcRewardKind::SourceProposer,
                source,
                vec![
                    AccountMeta::new(sku, false),
                    AccountMeta::new_readonly(*source, false),
                    AccountMeta::new_readonly(reward, false),
                    AccountMeta::new_readonly(
                        instruction::find_carry_source_address(&program(), source),
                        false,
                    ),
                ],
            )
            .await
            .unwrap();
        let support = instruction::find_oracle_support_address(
            &program(),
            &cohort.month,
            source,
            &supporter.pubkey(),
        );
        supporter_paid += h
            .claim_reward(
                cohort,
                &supporter,
                OracleUsdcRewardKind::SourceSupport,
                &support,
                vec![
                    AccountMeta::new(sku, false),
                    AccountMeta::new_readonly(*source, false),
                    AccountMeta::new_readonly(reward, false),
                    AccountMeta::new_readonly(support, false),
                ],
            )
            .await
            .unwrap();
        let opening =
            instruction::find_oracle_opening_claim_address(&program(), &cohort.month, source);
        opening_paid += h
            .claim_reward(
                cohort,
                &claimant,
                OracleUsdcRewardKind::Opening,
                &opening,
                vec![
                    AccountMeta::new(sku, false),
                    AccountMeta::new_readonly(*source, false),
                    AccountMeta::new_readonly(reward, false),
                    AccountMeta::new_readonly(opening, false),
                ],
            )
            .await
            .unwrap();
    }
    for (_, claim) in &updates {
        let registration = derive_oracle_usdc_reward_registration_pda(
            &program(),
            &schedule,
            OracleUsdcRewardKind::Update,
            &claim.key,
        )
        .0;
        update_paid += h
            .claim_reward(
                cohort,
                &updater,
                OracleUsdcRewardKind::Update,
                &claim.key,
                vec![
                    AccountMeta::new(sku, false),
                    AccountMeta::new_readonly(registration, false),
                    AccountMeta::new_readonly(claim.key, false),
                ],
            )
            .await
            .unwrap();
    }
    assert!(proposer_paid > 0 && supporter_paid > 0);
    assert_eq!(opening_paid, OPENING_REWARD / 3 * 3);
    // Both prints are fresh (three reward units each) and share the update budget.
    assert!(update_paid <= UPDATE_REWARD && update_paid + 1 >= UPDATE_REWARD);
    let paid = proposer_paid + supporter_paid + opening_paid + update_paid;
    let first_schedule: OracleUsdcRewardSchedule = h.read_state(schedule).await;
    assert_eq!(
        first_schedule.total_reward_budget - first_schedule.remaining_reward_budget,
        paid
    );

    // Terminal cleanup: the month closes and its bucket median is reclaimed; the period stays
    // Settled.
    h.cleanup(cohort, 0, [0; 32], None, None).await.unwrap();
    let median = derive_oracle_bucket_median_pda(&program(), &cohort.month, &bucket).0;
    h.cleanup(cohort, 1, [0; 32], Some(median), Some(schedule))
        .await
        .unwrap();
    assert!(h.account(median).await.is_none());
    let closed: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(closed.phase, OraclePhase::Closed);
    assert_eq!(h.consumer_view(cohort).await, settled);

    // The next rung never covered its SKU. Tag 79 fails it exactly at the submission deadline.
    let deadline = next.expiry
        - ORACLE_KILL_WINDOW_SECONDS
        - ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
        - ORACLE_OPENING_WINDOW_SECONDS;
    let fail = instruction::fail_period(&program(), &payer.pubkey(), &next.product, next.expiry);
    h.at(deadline - 1).await;
    assert_oracle_error(
        h.send(fail.clone(), &[]).await,
        OracleError::PeriodNotFailable,
    );
    h.at(deadline).await;
    h.send(fail, &[]).await.unwrap();
    let failed = h.consumer_view(&next).await;
    assert_eq!(failed.status, PeriodStatus::Failed);
    assert_eq!(failed.effective_listing_ts, 0);
    assert_eq!(failed.settlement(), None);
    assert!(!failed.is_tradable_at(next.listing));

    // Its lanes: the unlistable source (which never counted toward coverage) expires (191), its
    // listing bond returns (194) and the reward reservation is released (195).
    h.send(
        instruction::expire_unlistable_oracle_source_v2(
            &program(),
            &payer.pubkey(),
            &next.period,
            &next.month,
            &orphan,
            None,
        ),
        &[],
    )
    .await
    .unwrap();
    h.send(
        instruction::settle_failed_oracle_month_escrow_v2(
            &program(),
            &payer.pubkey(),
            &next.period,
            &next.month,
            &PrelistingEscrow::ListingBond {
                source: orphan,
                proposer: late.pubkey(),
            },
        ),
        &[],
    )
    .await
    .unwrap();
    h.send(
        instruction::abort_oracle_usdc_reward_schedule_v2(
            &program(),
            &payer.pubkey(),
            &next.period,
            &next.month,
        ),
        &[],
    )
    .await
    .unwrap();
    let aborted: OracleUsdcRewardSchedule = h.read_state(next.schedule()).await;
    assert_eq!(aborted.phase, OracleUsdcRewardSchedulePhase::Aborted);
    assert_eq!(h.consumer_view(&next).await, failed);
    assert_eq!(h.consumer_view(cohort).await, settled);

    // Every USDC unit is accounted for: each ledger holds its deposit plus its rewards, the bond
    // vault holds exactly the ledgers, and the reward vault holds the deposits less the claims,
    // reserving only the settled month's unclaimed remainder.
    let expected = [
        (&wide.proposer, PARTICIPANT_USDC + proposer_paid),
        (&wide.supporter, PARTICIPANT_USDC + supporter_paid),
        (&wide.claimant, PARTICIPANT_USDC + opening_paid),
        (&updater, PARTICIPANT_USDC + update_paid),
        (&late, PARTICIPANT_USDC),
    ];
    let mut ledgers = 0;
    for (owner, balance) in expected {
        let ledger = h.collateral(&owner.pubkey()).await;
        assert_eq!(ledger.available_balance, balance);
        assert_eq!(ledger.position_locked_balance, 0);
        ledgers += balance;
    }
    let bond_vault = h.bond_vault();
    assert_eq!(h.token_balance(bond_vault).await, ledgers);
    let deposits = 2 * (SOURCE_REWARD + OPENING_REWARD + UPDATE_REWARD);
    let mint = h.usdc_mint;
    let reward_token = instruction::find_reward_vault_token_address(&program(), &mint);
    assert_eq!(h.token_balance(reward_token).await, deposits - paid);
    let vault: OracleUsdcRewardVault = h
        .read_state(derive_oracle_usdc_reward_vault_pda(&program()).0)
        .await;
    assert_eq!(vault.total_paid, paid);
    assert_eq!(vault.total_reserved, first_schedule.remaining_reward_budget);
}
