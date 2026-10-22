//! Underwritten sources (tag 30 selector 22): a sponsor funds a researcher's listing bond, the
//! outcome reconciles back to the sponsor, the proposer reward splits between sponsor, researcher
//! and the researcher's reserve, and the researcher withdraws the reserve.
mod common;

use ameba_oracle::{
    constants::ORACLE_SETTLEMENT_GRACE_SECONDS,
    instruction::{self, ProposeOracleSourceV3Params, SponsoredAccounts},
    sponsorship::{
        exposure_address, reserve_address, split_bounty, AgentReserve, SourceSponsorship,
        SponsorExposure, SponsorTerms, DEFAULT_RESEARCHER_BPS, DEFAULT_RETENTION_BPS,
        LEGACY_RESEARCHER_BPS,
    },
    state::{
        derive_oracle_usdc_reward_schedule_pda, derive_oracle_usdc_sku_pool_pda,
        OracleUsdcRewardSchedule, OracleUsdcSkuPool,
    },
};
use common::{cohort::*, settle::*, *};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

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
    sponsored_to_reward(false).await;
}

/// An agreement made before the 80/20 split keeps the researcher's half to the end.
#[tokio::test]
async fn an_agreement_from_before_the_80_20_split_settles_with_its_half() {
    sponsored_to_reward(true).await;
}

async fn sponsored_to_reward(legacy: bool) {
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
    assert_eq!(agreement.researcher_bps, DEFAULT_RESEARCHER_BPS);
    if legacy {
        // As the program before the split stored it: researcher_bps follows the discriminator,
        // bump, source, reserve, funder, candidate hash and bond.
        let mut account = h.account(sponsored.agreement).await.unwrap();
        account.data[145..147].copy_from_slice(&LEGACY_RESEARCHER_BPS.to_le_bytes());
        h.set_account_preserving_capitalization(sponsored.agreement, account.into())
            .await;
    }
    let agreement: SourceSponsorship = h.read_state(sponsored.agreement).await;
    let researcher_bps = agreement.researcher_bps;
    assert_eq!(
        researcher_bps,
        if legacy {
            LEGACY_RESEARCHER_BPS
        } else {
            DEFAULT_RESEARCHER_BPS
        }
    );
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
    let (to_sponsor, to_researcher, retained) =
        split_bounty(paid, researcher_bps, DEFAULT_RETENTION_BPS).unwrap();
    // 80/20, or the legacy half.
    assert_eq!(
        to_researcher + retained,
        paid * u64::from(researcher_bps) / 10_000
    );
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

/// The terms `petri oracle sponsor` (and the TUI's Sponsor it) sets: sponsor cash, the governed
/// split, the SKU pool's bond, and an exposure cap of what the sponsor's collateral can post.
fn petri_terms(
    researcher: &Pubkey,
    candidate: [u8; 32],
    bond: u64,
    available: u64,
) -> SponsorTerms {
    SponsorTerms {
        agent_id: [0xa2; 32],
        researcher: *researcher,
        candidate_hash: candidate,
        funding: 0,
        researcher_bps: DEFAULT_RESEARCHER_BPS,
        retention_bps: DEFAULT_RETENTION_BPS,
        maximum_bond: bond,
        maximum_outstanding: available.max(bond),
    }
}

/// A sponsorship as petri sends it, through every step: the sponsor's wallet pays its activation,
/// petri finds the agreement by its funder, the sponsor settles its own bond and reconciles in
/// one transaction, claims the bounty, and the researcher withdraws the reserve.
#[tokio::test]
async fn a_wallet_sponsors_a_find_through_every_step_as_petri_sends_it() {
    let mut h = start().await;
    let researcher = Keypair::new();
    // A new sponsor's collateral is its deposit.
    let terms = petri_terms(
        &researcher.pubkey(),
        WIDE_SOURCES[1],
        LISTING_BOND,
        PARTICIPANT_USDC,
    );
    let wide = wide_live_paid(&mut h, Some(terms.clone()), true).await;
    let sponsor = wide.sponsor.as_ref().unwrap().insecure_clone();
    let cohort = &wide.cohort;
    let source = wide.sources[1];
    let bucket = cohort.sku_ids[0];
    let schedule = derive_oracle_usdc_reward_schedule_pda(&program(), &cohort.month).0;
    let pool: OracleUsdcSkuPool = h
        .read_state(derive_oracle_usdc_sku_pool_pda(&program(), &schedule, &bucket).0)
        .await;
    assert_eq!(pool.listing_bond, LISTING_BOND);

    // `petri oracle sponsorships`: agreements by marker, with the funder at byte 73; petri
    // rebuilds the accounts from the agreement.
    let sponsored = SponsoredAccounts::new(&program(), &terms, &source, Some(&sponsor.pubkey()));
    let raw = h.account(sponsored.agreement).await.unwrap().data;
    assert_eq!(raw.len(), SourceSponsorship::LEN);
    assert_eq!(raw[..8], SourceSponsorship::MAGIC);
    assert_eq!(raw[73..105], sponsor.pubkey().to_bytes());
    let agreement: SourceSponsorship = h.read_state(sponsored.agreement).await;
    assert_eq!(
        (
            agreement.outcome,
            agreement.bond,
            agreement.month,
            agreement.bucket_id
        ),
        (0, LISTING_BOND, cohort.month, bucket)
    );
    assert_eq!(agreement.funder, sponsor.pubkey());
    assert_eq!(
        exposure_address(&program(), &agreement.reserve, &agreement.funder).0,
        sponsored.exposure
    );
    assert_eq!(
        h.collateral(&sponsor.pubkey()).await.available_balance,
        PARTICIPANT_USDC - LISTING_BOND
    );

    // `petri oracle sponsorships settle`: after the month publishes, the sponsor settles its
    // bond to the agreement and records the outcome in one transaction.
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
    h.settle_wide_escrows_except(&wide, Some(&source)).await;
    h.send_many(
        vec![
            instruction::settle_oracle_usdc_prelisting_escrow(
                &program(),
                &sponsor.pubkey(),
                &cohort.period,
                &cohort.month,
                &instruction::PrelistingEscrow::ListingBond {
                    source,
                    proposer: sponsored.agreement,
                },
            ),
            instruction::sponsored_reconcile(
                &program(),
                &sponsor.pubkey(),
                &cohort.month,
                &source,
                &sponsored,
            ),
        ],
        &[&sponsor],
    )
    .await
    .unwrap();
    let reconciled: SourceSponsorship = h.read_state(sponsored.agreement).await;
    assert_eq!((reconciled.outcome, reconciled.reward_settled), (1, false));
    assert_eq!(
        h.collateral(&sponsor.pubkey()).await.available_balance,
        PARTICIPANT_USDC
    );

    // The month's rewards register and freeze (protocol cranks, not petri's).
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

    // `petri oracle sponsorships claim`: the sponsor pays; the researcher is the reserve's
    // authority and the product the agreement's.
    let reserve: AgentReserve = h.read_state(agreement.reserve).await;
    assert_eq!(reserve.authority, researcher.pubkey());
    let before_budget = h
        .read_state::<OracleUsdcRewardSchedule>(schedule)
        .await
        .remaining_reward_budget;
    let mint = h.usdc_mint;
    h.send(
        instruction::sponsored_claim_bounty(
            &program(),
            &sponsor.pubkey(),
            &cohort.period,
            &cohort.month,
            &mint,
            &agreement.bucket_id,
            &source,
            &reserve.authority,
            &sponsored,
        ),
        &[&sponsor],
    )
    .await
    .unwrap();
    let paid = before_budget
        - h.read_state::<OracleUsdcRewardSchedule>(schedule)
            .await
            .remaining_reward_budget;
    assert!(paid > 0);
    let (to_sponsor, to_researcher, retained) =
        split_bounty(paid, DEFAULT_RESEARCHER_BPS, DEFAULT_RETENTION_BPS).unwrap();
    assert_eq!(
        h.collateral(&sponsor.pubkey()).await.available_balance,
        PARTICIPANT_USDC + to_sponsor
    );
    let settled: SourceSponsorship = h.read_state(sponsored.agreement).await;
    assert_eq!((settled.reward_settled, settled.reward_paid), (true, paid));

    // `petri oracle reserve withdraw`: the researcher's reserve is found by agent and wallet.
    assert_eq!(
        reserve_address(&program(), &terms.agent_id, &researcher.pubkey()).0,
        sponsored.reserve
    );
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
    // New sponsorships take the 80/20 split, not the legacy half.
    let mut legacy = terms(&researcher);
    legacy.researcher_bps = LEGACY_RESEARCHER_BPS;
    cases.push(legacy);
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
