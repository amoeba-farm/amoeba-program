//! After settlement: cash reward registration (170, 171), entitlements (173) and claims (174)
//! into collateral ledgers, then the terminal cleanup (30/24) of the month's bookkeeping.
mod common;

use ameba_oracle::{
    constants::ORACLE_SETTLEMENT_GRACE_SECONDS,
    error::OracleError,
    instruction,
    state::{
        derive_oracle_bucket_median_pda, derive_oracle_bucket_source_index_pda,
        derive_oracle_recipe_source_index_pda, derive_oracle_usdc_reward_schedule_pda,
        derive_oracle_usdc_sku_pool_pda, derive_oracle_usdc_source_reward_pda, OracleMonthState,
        OraclePhase, OracleUpdateClaimOutcome, OracleUsdcRewardKind, OracleUsdcRewardReceipt,
        OracleUsdcRewardSchedule, OracleUsdcRewardSchedulePhase,
    },
};
use ameba_oracle_interface::PeriodStatus;
use common::{cohort::*, settle::*, *};
use solana_sdk::{
    instruction::AccountMeta,
    signature::{Keypair, Signer},
};

const DAY: u64 = 86_400;

#[tokio::test]
async fn settled_month_pays_every_reward_once_and_cleans_up_without_touching_claims() {
    let mut h = start().await;
    let wide = wide_live(&mut h).await;
    let cohort = &wide.cohort;
    let bucket = cohort.sku_ids[0];
    let source0 = wide.sources[0];

    // One accepted update during Game.
    let updater = h.participant(PARTICIPANT_USDC).await;
    let now = cohort.listing + 3_600;
    h.at(now).await;
    let print = h
        .print(
            cohort,
            &updater,
            &source0,
            &WIDE_SOURCES[0],
            101_000_000,
            now - 60,
        )
        .await;
    let update = h
        .commit(
            cohort,
            &updater,
            &source0,
            &WIDE_SOURCES[0],
            0,
            [21; 32],
            print,
        )
        .await
        .unwrap();
    h.advance_slot().await;
    h.reveal(cohort, &updater, &source0, &update).await.unwrap();
    h.at(now + DAY).await;
    h.finalize(
        cohort,
        &source0,
        0,
        &update.key,
        OracleUpdateClaimOutcome::AcceptClaim,
        1,
        instruction::UpdateFinalization::Unchallenged,
    )
    .await
    .unwrap();

    // Settle the month.
    h.at(cohort.expiry + ORACLE_SETTLEMENT_GRACE_SECONDS).await;
    h.finalize_wide(&wide).await;
    let leaf = h.leaf_for(cohort).await;
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
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Settled
    );

    // Entitlements cannot freeze before every pre-listing escrow settles and every source and
    // update is registered.
    h.settle_wide_escrows(&wide).await;
    let oracle = h.oracle_authority.insecure_clone();
    let finalize = instruction::finalize_oracle_usdc_reward_entitlements(
        &program(),
        &oracle.pubkey(),
        &cohort.period,
        &cohort.month,
    );
    assert_oracle_error(
        h.send(finalize.clone(), &[&oracle]).await,
        OracleError::OracleUsdcRewardRegistrationIncomplete,
    );
    let payer = h.payer();
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
    h.send(
        instruction::register_oracle_usdc_reward_update(
            &program(),
            &payer.pubkey(),
            &cohort.month,
            &bucket,
            &source0,
            &update.key,
        ),
        &[],
    )
    .await
    .unwrap();
    // A claim before the entitlements freeze is refused.
    let schedule = derive_oracle_usdc_reward_schedule_pda(&program(), &cohort.month).0;
    let sku = derive_oracle_usdc_sku_pool_pda(&program(), &schedule, &bucket).0;
    let reward0 = derive_oracle_usdc_source_reward_pda(&program(), &schedule, &source0).0;
    let proposer_tail = || {
        vec![
            AccountMeta::new(sku, false),
            AccountMeta::new_readonly(source0, false),
            AccountMeta::new_readonly(reward0, false),
            AccountMeta::new_readonly(
                instruction::find_carry_source_address(&program(), &source0),
                false,
            ),
        ]
    };
    let proposer = wide.proposer.insecure_clone();
    assert_oracle_error(
        h.claim_reward(
            cohort,
            &proposer,
            OracleUsdcRewardKind::SourceProposer,
            &source0,
            proposer_tail(),
        )
        .await
        .map(|_| ()),
        OracleError::OracleUsdcRewardEntitlementsNotFinalized,
    );
    h.advance_slot().await;
    h.send(finalize, &[&oracle]).await.unwrap();
    let frozen: OracleUsdcRewardSchedule = h.read_state(schedule).await;
    assert_eq!(
        frozen.phase,
        OracleUsdcRewardSchedulePhase::EntitlementsFinalized
    );
    assert_eq!(frozen.registered_source_count, 3);
    assert_eq!(frozen.registered_update_count, 1);

    // Every kind pays once into the recipient's collateral ledger.
    let proposer_paid = h
        .claim_reward(
            cohort,
            &proposer,
            OracleUsdcRewardKind::SourceProposer,
            &source0,
            proposer_tail(),
        )
        .await
        .unwrap();
    assert!(proposer_paid > 0);
    let supporter = wide.supporter.insecure_clone();
    let support = instruction::find_oracle_support_address(
        &program(),
        &cohort.month,
        &source0,
        &supporter.pubkey(),
    );
    let supporter_paid = h
        .claim_reward(
            cohort,
            &supporter,
            OracleUsdcRewardKind::SourceSupport,
            &support,
            vec![
                AccountMeta::new(sku, false),
                AccountMeta::new_readonly(source0, false),
                AccountMeta::new_readonly(reward0, false),
                AccountMeta::new_readonly(support, false),
            ],
        )
        .await
        .unwrap();
    assert!(supporter_paid > 0);
    let claimant = wide.claimant.insecure_clone();
    let opening =
        instruction::find_oracle_opening_claim_address(&program(), &cohort.month, &source0);
    let opening_paid = h
        .claim_reward(
            cohort,
            &claimant,
            OracleUsdcRewardKind::Opening,
            &opening,
            vec![
                AccountMeta::new(sku, false),
                AccountMeta::new_readonly(source0, false),
                AccountMeta::new_readonly(reward0, false),
                AccountMeta::new_readonly(opening, false),
            ],
        )
        .await
        .unwrap();
    assert_eq!(opening_paid, OPENING_REWARD / 3);
    let registration = ameba_oracle::state::derive_oracle_usdc_reward_registration_pda(
        &program(),
        &schedule,
        OracleUsdcRewardKind::Update,
        &update.key,
    )
    .0;
    let update_paid = h
        .claim_reward(
            cohort,
            &updater,
            OracleUsdcRewardKind::Update,
            &update.key,
            vec![
                AccountMeta::new(sku, false),
                AccountMeta::new_readonly(registration, false),
                AccountMeta::new_readonly(update.key, false),
            ],
        )
        .await
        .unwrap();
    assert_eq!(update_paid, UPDATE_REWARD);
    let receipt: OracleUsdcRewardReceipt = h
        .read_state(
            ameba_oracle::state::derive_oracle_usdc_reward_receipt_pda(
                &program(),
                &schedule,
                OracleUsdcRewardKind::Update,
                &update.key,
                &updater.pubkey(),
            )
            .0,
        )
        .await;
    assert_eq!(receipt.amount, update_paid);
    // Exactly once.
    h.advance_slot().await;
    assert!(h
        .claim_reward(
            cohort,
            &proposer,
            OracleUsdcRewardKind::SourceProposer,
            &source0,
            proposer_tail(),
        )
        .await
        .is_err());
    let paid = h.read_state::<OracleUsdcRewardSchedule>(schedule).await;
    assert_eq!(
        paid.total_reward_budget - paid.remaining_reward_budget,
        proposer_paid + supporter_paid + opening_paid + update_paid
    );

    // Cleanup starts on the Settled period and closes the month.
    let receipt_key = instruction::find_cleanup_receipt_address(&program(), &cohort.month);
    h.cleanup(cohort, 0, [0; 32], None, None).await.unwrap();
    let closed: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(closed.phase, OraclePhase::Closed);
    assert!(h.account(receipt_key).await.is_some());
    h.advance_slot().await;
    assert!(h.cleanup(cohort, 0, [0; 32], None, None).await.is_err());

    // Reclaim the bucket median (rewards are frozen), the bucket index, a member page and a
    // locator link; each refund goes to the recipient and the receipt counts it.
    let admin_key = h.admin.pubkey();
    let median = derive_oracle_bucket_median_pda(&program(), &cohort.month, &bucket).0;
    let median_lamports = h.account(median).await.unwrap().lamports;
    let admin_before = h.account(admin_key).await.unwrap().lamports;
    h.cleanup(cohort, 1, [0; 32], Some(median), Some(schedule))
        .await
        .unwrap();
    assert!(h.account(median).await.is_none());
    let admin_after = h.account(admin_key).await.unwrap().lamports;
    assert!(admin_after + 10_000 >= admin_before + median_lamports);
    let recipe_index = derive_oracle_recipe_source_index_pda(&program(), &cohort.month).0;
    let bucket_index = derive_oracle_bucket_source_index_pda(&program(), &cohort.month, &bucket).0;
    let page = instruction::find_member_page_address(&program(), &bucket_index, 0);
    h.cleanup(cohort, 3, bucket, Some(page), None)
        .await
        .unwrap();
    h.cleanup(cohort, 2, [0; 32], Some(bucket_index), Some(recipe_index))
        .await
        .unwrap();
    let link = instruction::find_evidence_link_address(
        &program(),
        &source0,
        0,
        &locator_hash(&locator(&WIDE_SOURCES[0])),
    );
    h.cleanup(cohort, 6, [0; 32], Some(link), None)
        .await
        .unwrap();
    for key in [page, bucket_index, link] {
        assert!(h.account(key).await.is_none());
    }
    let receipt_account = h.account(receipt_key).await.unwrap();
    assert_eq!(
        u32::from_le_bytes(receipt_account.data[134..138].try_into().unwrap()),
        4
    );
    // Financial accounts are never targets.
    assert!(h
        .cleanup(cohort, 1, [0; 32], Some(reward0), Some(schedule))
        .await
        .is_err());
    // Claims and the settled period are untouched.
    assert!(h.account(reward0).await.is_some());
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Settled
    );
}

#[tokio::test]
async fn cleanup_requires_a_settled_period() {
    let mut h = start().await;
    let wide = wide_live(&mut h).await;
    let cohort = &wide.cohort;
    h.at(cohort.expiry + ORACLE_SETTLEMENT_GRACE_SECONDS).await;
    // A Live period cannot start cleanup.
    assert!(h.cleanup(cohort, 0, [0; 32], None, None).await.is_err());
    h.finalize_wide(&wide).await;
    assert!(h.cleanup(cohort, 0, [0; 32], None, None).await.is_err());
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Live
    );
}
