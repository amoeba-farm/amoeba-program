//! Underwritten sources (tag 30 selector 22): a sponsor funds a researcher's listing bond, the
//! outcome reconciles back to the sponsor, the proposer reward splits between sponsor, researcher
//! and the researcher's reserve, and the researcher withdraws the reserve.
mod common;

use ameba_oracle::{
    constants::ORACLE_SETTLEMENT_GRACE_SECONDS,
    instruction::{self, ProposeOracleSourceV3Params, SponsoredAccounts},
    sponsorship::{split_bounty, AgentReserve, SourceSponsorship, SponsorExposure, SponsorTerms},
    state::derive_oracle_usdc_reward_schedule_pda,
};
use common::{cohort::*, settle::*, *};
use solana_sdk::signature::{Keypair, Signer};

fn terms(researcher: &Keypair) -> SponsorTerms {
    SponsorTerms {
        agent_id: [0xa1; 32],
        researcher: researcher.pubkey(),
        candidate_hash: WIDE_SOURCES[1],
        funding: 0,
        researcher_bps: ameba_oracle::sponsorship::DEFAULT_RESEARCHER_BPS,
        retention_bps: ameba_oracle::sponsorship::DEFAULT_RETENTION_BPS,
        maximum_bond: LISTING_BOND,
        maximum_outstanding: 2 * LISTING_BOND,
    }
}

#[tokio::test]
async fn sponsored_bond_reconciles_and_its_reward_splits_three_ways() {
    let mut h = start().await;
    let researcher = Keypair::new();
    let wide = wide_live_with(&mut h, Some(terms(&researcher))).await;
    let sponsor = wide.sponsor.as_ref().unwrap().insecure_clone();
    let cohort = &wide.cohort;
    let source = wide.sources[1];
    let sponsored = SponsoredAccounts::new(
        &program(),
        &terms(&researcher),
        &source,
        Some(&sponsor.pubkey()),
    );
    let agreement: SourceSponsorship = h.read_state(sponsored.agreement).await;
    assert_eq!(agreement.bond, LISTING_BOND);
    assert_eq!(agreement.outcome, 0);
    assert_eq!(agreement.funder, sponsor.pubkey());
    let exposure: SponsorExposure = h.read_state(sponsored.exposure).await;
    assert_eq!(exposure.outstanding, LISTING_BOND);
    let sponsor_after_bond = h.collateral(&sponsor.pubkey()).await.available_balance;
    assert_eq!(sponsor_after_bond, PARTICIPANT_USDC - LISTING_BOND);

    // Settle the month and its escrows; the refunded bond reconciles back to the sponsor.
    h.at(cohort.expiry + ORACLE_SETTLEMENT_GRACE_SECONDS).await;
    h.finalize_wide(&wide).await;
    let leaf = h.leaf_for(cohort).await;
    let signers: Vec<Keypair> = h
        .settlement_signers
        .iter()
        .map(|k| k.insecure_clone())
        .collect();
    h.publish(
        cohort,
        &[&signers[0], &signers[1], &signers[2]],
        &leaf,
        &leaf,
    )
    .await
    .unwrap();
    h.settle_wide_escrows(&wide).await;
    let payer = h.payer();
    h.send(
        instruction::sponsored_reconcile(
            &program(),
            &payer.pubkey(),
            &cohort.month,
            &source,
            &sponsored,
        ),
        &[],
    )
    .await
    .unwrap();
    let reconciled: SourceSponsorship = h.read_state(sponsored.agreement).await;
    assert_eq!(reconciled.outcome, 1);
    let agent: AgentReserve = h.read_state(sponsored.reserve).await;
    assert_eq!((agent.accepted, agent.pending, agent.rejected), (1, 0, 0));
    let exposure: SponsorExposure = h.read_state(sponsored.exposure).await;
    assert_eq!(exposure.outstanding, 0);
    assert_eq!(
        h.collateral(&sponsor.pubkey()).await.available_balance,
        PARTICIPANT_USDC
    );

    // Rewards: register, freeze, then the agreement's proposer reward splits three ways.
    let bucket = cohort.sku_ids[0];
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
    let oracle = h.oracle_authority.insecure_clone();
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
    let before_budget = h
        .read_state::<ameba_oracle::state::OracleUsdcRewardSchedule>(schedule)
        .await
        .remaining_reward_budget;
    let mint = h.usdc_mint;
    let claim = instruction::sponsored_claim_bounty(
        &program(),
        &payer.pubkey(),
        &cohort.period,
        &cohort.month,
        &mint,
        &bucket,
        &source,
        &researcher.pubkey(),
        &sponsored,
    );
    h.send(claim.clone(), &[]).await.unwrap();
    let paid = before_budget
        - h.read_state::<ameba_oracle::state::OracleUsdcRewardSchedule>(schedule)
            .await
            .remaining_reward_budget;
    assert!(paid > 0);
    let (to_sponsor, to_researcher, retained) = split_bounty(paid, 5_000, 2_000).unwrap();
    assert_eq!(
        h.collateral(&sponsor.pubkey()).await.available_balance,
        PARTICIPANT_USDC + to_sponsor
    );
    assert_eq!(
        h.collateral(&researcher.pubkey()).await.available_balance,
        to_researcher
    );
    assert_eq!(
        h.collateral(&sponsored.reserve).await.available_balance,
        retained
    );
    let settled: SourceSponsorship = h.read_state(sponsored.agreement).await;
    assert!(settled.reward_settled);
    assert_eq!(settled.reward_paid, paid);
    h.advance_slot().await;
    assert!(h.send(claim, &[]).await.is_err());

    // The researcher withdraws the reserve; nobody else can.
    let stranger = Keypair::new();
    assert!(h
        .send(
            instruction::sponsored_withdraw_reserve(
                &program(),
                &stranger.pubkey(),
                &sponsored.reserve,
                1
            ),
            &[&stranger],
        )
        .await
        .is_err());
    h.send(
        instruction::sponsored_withdraw_reserve(
            &program(),
            &researcher.pubkey(),
            &sponsored.reserve,
            retained,
        ),
        &[&researcher],
    )
    .await
    .unwrap();
    assert_eq!(
        h.collateral(&researcher.pubkey()).await.available_balance,
        to_researcher + retained
    );
    assert_eq!(h.collateral(&sponsored.reserve).await.available_balance, 0);
}

#[tokio::test]
async fn activation_refuses_terms_outside_the_governed_split_and_bond() {
    let mut h = start().await;
    let cohort = h.open_cohort(1).await;
    h.fund_schedule(&cohort).await;
    h.at(cohort.scramble).await;
    let sponsor = h.participant(PARTICIPANT_USDC).await;
    let researcher = Keypair::new();
    h.upload_evidence(&sponsor, 1, &locator(&WIDE_SOURCES[1]))
        .await;
    h.upload_evidence(&sponsor, 2, &definition(&WIDE_SOURCES[1]))
        .await;
    let params = ProposeOracleSourceV3Params {
        source_id: WIDE_SOURCES[1],
        bucket_id: cohort.sku_ids[0],
        source_type_hash: SOURCE_TYPE_HASH,
        canonical_locator_hash: locator_hash(&locator(&WIDE_SOURCES[1])),
        source_definition_hash: definition_hash(&definition(&WIDE_SOURCES[1])),
        listing_bond: LISTING_BOND,
        sku_index: 0,
        sku_proof: cohort.sku_proof(0),
    };
    let payer = h.payer();
    let mut cases = Vec::new();
    let mut split = terms(&researcher);
    split.researcher_bps = 6_000;
    cases.push(split);
    let mut small = terms(&researcher);
    small.maximum_bond = LISTING_BOND - 1;
    cases.push(small);
    let mut other = terms(&researcher);
    other.candidate_hash = [0xee; 32];
    cases.push(other);
    // A researcher-funded bond needs the researcher's signature.
    let mut reserve_funded = terms(&researcher);
    reserve_funded.funding = 1;
    reserve_funded.researcher_bps = 10_000;
    cases.push(reserve_funded);
    for case in cases {
        h.advance_slot().await;
        let ix = instruction::sponsored_activate(
            &program(),
            &payer.pubkey(),
            &sponsor.pubkey(),
            &sponsor.pubkey(),
            &cohort.period,
            &cohort.month,
            case,
            params.clone(),
        );
        assert!(h.send(ix, &[&sponsor]).await.is_err());
    }
    assert!(h.account(cohort.source(&WIDE_SOURCES[1])).await.is_none());
}
