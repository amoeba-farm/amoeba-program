//! BATX Dec 1 (`BATX-1796083200`) ended on the production artifact, from every state the
//! program deployed on mainnet before the heads upgrade lets anyone reach (the formation build
//! here) while the month waits in source submission with complete coverage:
//!
//! - **source submission**, as on mainnet on 2026-10-30 (more sources and supports may come);
//! - **Scramble**: tag 184 is permissionless and coverage is complete, so anyone can finalize it;
//!   the calendar then starts from 184 (placement already over, two kill days, the freeze day);
//! - **Scramble with open source challenges**: 161 in the kill window, anyone with the bond;
//! - **after the Scramble**: 169 dismisses an unruled challenge (its bond to the proposer), 192
//!   calls one off (back to source submission, the bond still held), possibly with others still
//!   open;
//! - **failed by 79 at expiry** with its month left in Scramble (the upgrade or the admin late).
//!
//! From each, after the upgrade: 30/25 kind 0 (the admin), the failure lanes 194, 191, 194, 195
//! (anyone, each escrow to its own owner), then the admin's closes (kinds 1-12) until no account
//! of the month is left: the recipient receives exactly their rent, the period stays Failed, and
//! the bond vault's USDC never moves (the refunds are ledger credits). Last, the calendar
//! fallback without kind 0, from Scramble after the submission deadline (2026-11-27 for BATX).
//!
//! After the upgrade these tests move the clock only (`clock`), and each ends by checking that
//! the production artifact still answers (`still_production`): this harness runs the code the
//! program started with again after a warp unless it reinstalls the upgrade
//! (`Harness::upgraded`; seen 2026-10-30).
#![cfg(all(
    feature = "test-sbf",
    not(feature = "localnet-dev-authority"),
    not(feature = "localnet-governance")
))]

mod common;

use ameba_oracle::{
    error::OracleError,
    instruction::{self, ChallengeOracleSourceParams, PrelistingEscrow},
    state::{
        derive_oracle_sku_coverage_manifest_pda, derive_oracle_sku_coverage_record_pda,
        derive_oracle_source_challenge_guard_pda, derive_oracle_usdc_reward_schedule_pda,
        derive_oracle_usdc_reward_vault_pda, derive_oracle_usdc_sku_pool_pda,
        derive_oracle_usdc_source_reward_pda, OracleChallengeStatus, OracleEscrowDisposition,
        OracleMonthState, OraclePhase, OracleSkuCoverageManifest, OracleSourceChallenge,
        OracleSourceState, OracleSourceStatus, OracleSupportPosition, OracleUsdcRewardSchedule,
        OracleUsdcRewardSchedulePhase, OracleUsdcRewardVault,
    },
};
use ameba_oracle_interface::PeriodStatus;
use common::{cohort::*, production::*, *};
use solana_sdk::{
    account::AccountSharedData,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

const DAY: u64 = 86_400;
/// The non-independence reason (`processor::ORACLE_SOURCE_REASON_NON_INDEPENDENT`).
const NON_INDEPENDENT: u8 = 8;

/// One proposed source: its key, id, SKU, evidence links, proposer and supporter (indices into
/// `Batx::wallets`, none for an unsupported one).
struct Placed {
    key: Pubkey,
    id: [u8; 32],
    sku: usize,
    links: [Pubkey; 2],
    proposer: usize,
    supporter: Option<usize>,
}

/// A month placed as BATX Dec 1 is: house wallets that each propose one source and support
/// another wallet's (two SKUs here, all covered), and an outsider's unsupported source.
struct Batx {
    cohort: Cohort,
    wallets: Vec<Keypair>,
    sources: Vec<Placed>,
    challenger: Keypair,
    /// Each challenge with its source and comparison source.
    challenges: Vec<(Pubkey, Pubkey, Option<Pubkey>)>,
}

fn source_id(n: u8) -> [u8; 32] {
    [0x40 + n; 32]
}

impl Batx {
    fn owners(&self) -> Vec<Pubkey> {
        self.wallets
            .iter()
            .chain(std::iter::once(&self.challenger))
            .map(Keypair::pubkey)
            .collect()
    }

    fn proposer(&self, source: &Pubkey) -> Pubkey {
        let placed = self.sources.iter().find(|p| p.key == *source).unwrap();
        self.wallets[placed.proposer].pubkey()
    }

    fn source(&self, n: usize) -> &Placed {
        &self.sources[n]
    }
}

/// The formation build forms the month to source submission with complete coverage, as
/// mainnet's BATX Dec 1 stands (`None`: no production artifact, skip).
async fn batx() -> Option<(Harness, Vec<u8>, Batx)> {
    let (mut h, production) = start_formation().await?;
    let cohort = h.open_cohort(2).await;
    h.fund_schedule(&cohort).await;
    h.at(cohort.scramble).await;
    let mut wallets = Vec::new();
    for _ in 0..4 {
        wallets.push(h.participant(PARTICIPANT_USDC).await);
    }
    // Wallet n proposes source n and wallet n+1 supports it; the fourth (an outsider) proposes
    // one nobody supports.
    let mut sources = Vec::new();
    for (n, sku, supporter) in [
        (0, 0, Some(1)),
        (1, 1, Some(2)),
        (2, 0, Some(0)),
        (3, 1, None),
    ] {
        let id = source_id(n as u8);
        let propose = h.propose_ix(&cohort, &wallets[n].pubkey(), id, sku as u16);
        let links = [propose.accounts[13].pubkey, propose.accounts[15].pubkey];
        let proposer = wallets[n].insecure_clone();
        let key = h
            .propose_source(&cohort, &proposer, id, sku as u16)
            .await
            .unwrap();
        if let Some(s) = supporter {
            let supporter_key = wallets[s].insecure_clone();
            h.support_source(&cohort, &supporter_key, &key, sku as u16)
                .await
                .unwrap();
        }
        sources.push(Placed {
            key,
            id,
            sku,
            links,
            proposer: n,
            supporter,
        });
    }
    let challenger = h.participant(PARTICIPANT_USDC).await;
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(
        (month.phase, month.pending_resolution_count),
        (OraclePhase::SourceSubmission, 0)
    );
    Some((
        h,
        production,
        Batx {
            cohort,
            wallets,
            sources,
            challenger,
            challenges: Vec::new(),
        },
    ))
}

/// Anyone: tag 184 at `at` (the formation build).
async fn close_sources(h: &mut Harness, b: &Batx, at: u64) {
    h.at(at).await;
    h.finalize_coverage(&b.cohort).await;
    let month: OracleMonthState = h.read_state(b.cohort.month).await;
    assert_eq!(month.phase, OraclePhase::Scramble);
}

/// Anyone with the bond: 161 on source `n` (with `comparison`, a non-independence challenge).
async fn challenge(h: &mut Harness, b: &mut Batx, n: usize, comparison: Option<usize>, tag: u8) {
    let source = b.source(n).key;
    let sku = b.cohort.sku_ids[b.source(n).sku];
    let comparison = comparison.map(|c| (b.source(c).key, b.source(c).id));
    let challenge_id = [0xc0 + tag; 32];
    let ix = instruction::challenge_oracle_source_v2(
        &program(),
        &b.challenger.pubkey(),
        &b.cohort.period,
        &b.cohort.month,
        &source,
        &sku,
        comparison.as_ref().map(|(key, _)| key),
        ChallengeOracleSourceParams {
            challenge_id,
            reason: if comparison.is_some() {
                NON_INDEPENDENT
            } else {
                1
            },
            bond: CHALLENGE_MIN_BOND,
            evidence_hash: [0xe0 + tag; 32],
            comparison_source_id: comparison.map_or([0; 32], |(_, id)| id),
        },
    );
    let challenger = b.challenger.insecure_clone();
    h.send(ix, &[&challenger]).await.unwrap();
    let key = instruction::find_oracle_source_challenge_address(
        &program(),
        &b.cohort.month,
        &source,
        &challenge_id,
    );
    b.challenges
        .push((key, source, comparison.map(|(key, _)| key)));
}

fn challenge_escrow(b: &Batx, index: usize) -> PrelistingEscrow {
    let (challenge, source, comparison_source) = b.challenges[index];
    PrelistingEscrow::SourceChallenge {
        source,
        proposer: b.proposer(&source),
        challenge,
        challenger: b.challenger.pubkey(),
        comparison_source,
    }
}

fn kind0(b: &Batx, admin: &Pubkey, recipient: &Pubkey) -> solana_sdk::instruction::Instruction {
    let manifest = derive_oracle_sku_coverage_manifest_pda(&program(), &b.cohort.month).0;
    instruction::failed_month(
        &program(),
        admin,
        &b.cohort.product,
        &b.cohort.period,
        &b.cohort.month,
        recipient,
        0,
        None,
        Some(&manifest),
    )
}

/// After the upgrade: the clock moves, the slot doesn't (see the module note).
async fn clock(h: &mut Harness, unix_timestamp: u64) {
    set_unix_timestamp(&h.ctx, unix_timestamp).await;
}

/// The production artifact still runs: a v1 formation byte (186) is refused as unassigned.
async fn still_production(h: &mut Harness, b: &Batx) {
    let cranker = h.payer();
    assert_oracle_error(
        h.send(
            instruction::reopen_oracle_sku_coverage(
                &program(),
                &cranker.pubkey(),
                &b.cohort.period,
                &b.cohort.month,
            ),
            &[],
        )
        .await,
        OracleError::InvalidInstructionData,
    );
}

/// Each owner's available collateral.
async fn balances(h: &mut Harness, owners: &[Pubkey]) -> Vec<u64> {
    let mut out = Vec::new();
    for owner in owners {
        out.push(h.collateral(owner).await.available_balance);
    }
    out
}

/// Stage 2, permissionless, read from the chain as `futures-ops end-month` plans it: 194 for
/// every source challenge whose bond is still held, then 191 for every candidate, 194 for every
/// listing bond and support stake, 195. The period must be Failed and its month in source
/// submission.
async fn refunds(h: &mut Harness, b: &Batx) {
    let cranker = h.payer();
    let (period, month) = (b.cohort.period, b.cohort.month);
    let settle = |escrow: &PrelistingEscrow| {
        instruction::settle_failed_oracle_month_escrow_v2(
            &program(),
            &cranker.pubkey(),
            &period,
            &month,
            escrow,
        )
    };
    for index in 0..b.challenges.len() {
        let state: OracleSourceChallenge = h.read_state(b.challenges[index].0).await;
        if state.escrow_disposition == OracleEscrowDisposition::Unsettled {
            h.send(settle(&challenge_escrow(b, index)), &[])
                .await
                .unwrap_or_else(|e| panic!("194 challenge {index}: {e}"));
        }
    }
    let state: OracleMonthState = h.read_state(month).await;
    assert_eq!(state.pending_resolution_count, 0);
    for placed in &b.sources {
        let source: OracleSourceState = h.read_state(placed.key).await;
        if source.status == OracleSourceStatus::Candidate {
            let sku = b.cohort.sku_ids[placed.sku];
            h.send(
                instruction::expire_unlistable_oracle_source_v2(
                    &program(),
                    &cranker.pubkey(),
                    &period,
                    &month,
                    &placed.key,
                    placed.supporter.map(|_| &sku),
                ),
                &[],
            )
            .await
            .unwrap_or_else(|e| panic!("191 {:?}: {e}", placed.id[0]));
        }
        let source: OracleSourceState = h.read_state(placed.key).await;
        if source.listing_bond_locked > 0 {
            let escrow = PrelistingEscrow::ListingBond {
                source: placed.key,
                proposer: source.proposer,
            };
            h.send(settle(&escrow), &[]).await.unwrap();
        }
        if let Some(s) = placed.supporter {
            let supporter = b.wallets[s].pubkey();
            let support: OracleSupportPosition = h
                .read_state(instruction::find_oracle_support_address(
                    &program(),
                    &month,
                    &placed.key,
                    &supporter,
                ))
                .await;
            if !support.released {
                let escrow = PrelistingEscrow::SupportStake {
                    source: placed.key,
                    supporter,
                };
                h.send(settle(&escrow), &[]).await.unwrap();
            }
        }
    }
    h.send(
        instruction::abort_oracle_usdc_reward_schedule_v2(
            &program(),
            &cranker.pubkey(),
            &period,
            &month,
        ),
        &[],
    )
    .await
    .unwrap();
    let schedule: OracleUsdcRewardSchedule = h.read_state(b.cohort.schedule()).await;
    assert_eq!(
        (schedule.phase, schedule.outstanding_prelisting_escrow_count),
        (OracleUsdcRewardSchedulePhase::Aborted, 0)
    );
}

/// Every account the month made, with the 30/25 kind and parent that close it, children first.
fn closes(b: &Batx) -> Vec<(u8, Pubkey, Option<Pubkey>)> {
    let month = b.cohort.month;
    let schedule = derive_oracle_usdc_reward_schedule_pda(&program(), &month).0;
    let manifest = derive_oracle_sku_coverage_manifest_pda(&program(), &month).0;
    let mut steps = Vec::new();
    for placed in &b.sources {
        if let Some(s) = placed.supporter {
            let support = instruction::find_oracle_support_address(
                &program(),
                &month,
                &placed.key,
                &b.wallets[s].pubkey(),
            );
            steps.push((1, support, Some(placed.key)));
        }
        steps.push((
            2,
            derive_oracle_usdc_source_reward_pda(&program(), &schedule, &placed.key).0,
            Some(placed.key),
        ));
        steps.push((
            3,
            instruction::find_oracle_source_observations_address(&program(), &placed.key),
            Some(placed.key),
        ));
        for link in placed.links {
            steps.push((4, link, None));
        }
    }
    let mut guarded = Vec::new();
    for (challenge, source, comparison) in &b.challenges {
        steps.push((11, *challenge, None));
        guarded.push(*source);
        guarded.extend(*comparison);
    }
    guarded.sort();
    guarded.dedup();
    for source in guarded {
        steps.push((
            12,
            derive_oracle_source_challenge_guard_pda(&program(), &month, &source).0,
            None,
        ));
    }
    for placed in &b.sources {
        steps.push((5, placed.key, None));
    }
    for sku in &b.cohort.sku_ids {
        steps.push((
            6,
            derive_oracle_sku_coverage_record_pda(&program(), &month, sku).0,
            None,
        ));
        steps.push((
            7,
            derive_oracle_usdc_sku_pool_pda(&program(), &schedule, sku).0,
            None,
        ));
    }
    steps.push((8, schedule, None));
    steps.push((9, manifest, None));
    steps.push((10, month, Some(manifest)));
    steps
}

/// Stage 3, the admin: every close, each into `recipient`. Returns the rent they freed, after
/// checking that each account is gone and the recipient holds exactly that rent.
async fn close_everything(h: &mut Harness, b: &Batx, recipient: &Pubkey) -> u64 {
    let admin = h.admin.insecure_clone();
    let mut rent = 0;
    for (kind, target, parent) in closes(b) {
        rent += h
            .account(target)
            .await
            .unwrap_or_else(|| panic!("kind {kind}: {target} open"))
            .lamports;
        let ix = instruction::failed_month(
            &program(),
            &admin.pubkey(),
            &b.cohort.product,
            &b.cohort.period,
            &b.cohort.month,
            recipient,
            kind,
            Some(&target),
            parent.as_ref(),
        );
        h.send(ix, &[&admin])
            .await
            .unwrap_or_else(|e| panic!("kind {kind} {target}: {e}"));
        assert!(h.account(target).await.is_none(), "kind {kind} closed");
    }
    // Nothing of the month is left: no rent stranded.
    for (kind, target, _) in closes(b) {
        assert!(h.account(target).await.is_none(), "kind {kind} {target}");
    }
    assert_eq!(h.account(*recipient).await.unwrap().lamports, rent);
    assert_eq!(
        h.period(b.cohort.product, b.cohort.expiry).await.status,
        PeriodStatus::Failed
    );
    rent
}

/// What `end_month` compares with after the refunds: the recipient and the reward reservation.
struct Before {
    recipient: Pubkey,
    reserved: u64,
    budget: u64,
}

/// After the upgrade: kind 0, the refunds, the closes. Every owner's ledger ends at `expected`
/// (in `Batx::owners` order), the bond vault's USDC never moves and the reward reservation is
/// released.
async fn end_month(h: &mut Harness, b: &Batx, expected: &[u64]) {
    let before = fail_month(h, b).await;
    finish_month(h, b, expected, before).await;
}

/// Kind 0, the admin only: then the period is Failed and its month in source submission.
async fn fail_month(h: &mut Harness, b: &Batx) -> Before {
    let reward_vault_key = derive_oracle_usdc_reward_vault_pda(&program()).0;
    let reserved: OracleUsdcRewardVault = h.read_state(reward_vault_key).await;
    let budget = h
        .read_state::<OracleUsdcRewardSchedule>(b.cohort.schedule())
        .await
        .total_reward_budget;
    let admin = h.admin.insecure_clone();
    let recipient = Keypair::new().pubkey();
    let stranger = Keypair::new();
    h.fund(&stranger.pubkey(), 1_000_000_000).await;
    assert_oracle_error(
        h.send(kind0(b, &stranger.pubkey(), &recipient), &[&stranger])
            .await,
        OracleError::Unauthorized,
    );
    h.send(kind0(b, &admin.pubkey(), &recipient), &[&admin])
        .await
        .unwrap();
    assert_eq!(
        h.period(b.cohort.product, b.cohort.expiry).await.status,
        PeriodStatus::Failed
    );
    let month: OracleMonthState = h.read_state(b.cohort.month).await;
    assert_eq!(month.phase, OraclePhase::SourceSubmission);
    let manifest = derive_oracle_sku_coverage_manifest_pda(&program(), &b.cohort.month).0;
    let coverage: OracleSkuCoverageManifest = h.read_state(manifest).await;
    assert!(!coverage.coverage_finalized);
    Before {
        recipient,
        reserved: reserved.total_reserved,
        budget,
    }
}

/// The refunds and the closes after kind 0.
async fn finish_month(h: &mut Harness, b: &Batx, expected: &[u64], before: Before) {
    let vault = h.bond_vault();
    // Nothing forms after it.
    let late = h.participant(PARTICIPANT_USDC).await;
    assert!(h
        .propose_source(&b.cohort, &late, source_id(9), 0)
        .await
        .is_err());
    let vault_before = h.token_balance(vault).await;
    refunds(h, b).await;
    assert_eq!(balances(h, &b.owners()).await, expected);
    assert_eq!(h.token_balance(vault).await, vault_before);
    let released: OracleUsdcRewardVault = h
        .read_state(derive_oracle_usdc_reward_vault_pda(&program()).0)
        .await;
    assert_eq!(released.total_reserved, before.reserved - before.budget);
    close_everything(h, b, &before.recipient).await;
    assert_eq!(balances(h, &b.owners()).await, expected);
    still_production(h, b).await;
}

/// Mainnet now (2026-10-30): source submission, coverage complete, nothing pending.
#[tokio::test]
async fn batx_ends_from_source_submission_on_the_production_artifact() {
    let Some((mut h, production, b)) = batx().await else {
        return;
    };
    h.upgrade_oracle(&production).await;
    let everyone = vec![PARTICIPANT_USDC; b.owners().len()];
    end_month(&mut h, &b, &everyone).await;
}

/// Anyone ran 184 before the upgrade: Scramble, coverage final, in the kill window. Kind 0
/// needs the coverage manifest it reopens.
#[tokio::test]
async fn batx_ends_from_scramble_on_the_production_artifact() {
    let Some((mut h, production, b)) = batx().await else {
        return;
    };
    close_sources(&mut h, &b, b.cohort.placement_end()).await;
    h.upgrade_oracle(&production).await;
    // 184 and 186 are gone from the production build; kind 0 without the manifest is refused.
    let cranker = h.payer();
    assert_oracle_error(
        h.send(
            instruction::reopen_oracle_sku_coverage(
                &program(),
                &cranker.pubkey(),
                &b.cohort.period,
                &b.cohort.month,
            ),
            &[],
        )
        .await,
        OracleError::InvalidInstructionData,
    );
    let admin = h.admin.insecure_clone();
    let bare = instruction::failed_month(
        &program(),
        &admin.pubkey(),
        &b.cohort.product,
        &b.cohort.period,
        &b.cohort.month,
        &admin.pubkey(),
        0,
        None,
        None,
    );
    assert_oracle_error(
        h.send(bare, &[&admin]).await,
        OracleError::InvalidAccountList,
    );
    let everyone = vec![PARTICIPANT_USDC; b.owners().len()];
    end_month(&mut h, &b, &everyone).await;
}

/// Scramble with challenges open at the upgrade (161 in the kill window, nobody ruled): an
/// ordinary one and a non-independence one holding two guards. In the Failed period 194 returns
/// each bond to the challenger, releases the guards and clears the pending count; the closes
/// take the challenges and the guards too.
#[tokio::test]
async fn batx_ends_from_scramble_with_open_challenges_on_the_production_artifact() {
    let Some((mut h, production, mut b)) = batx().await else {
        return;
    };
    close_sources(&mut h, &b, b.cohort.placement_end()).await;
    challenge(&mut h, &mut b, 1, None, 1).await;
    challenge(&mut h, &mut b, 2, Some(0), 2).await;
    let challenger = b.challenger.pubkey();
    assert_eq!(
        h.collateral(&challenger).await.available_balance,
        PARTICIPANT_USDC - 2 * CHALLENGE_MIN_BOND
    );
    let month: OracleMonthState = h.read_state(b.cohort.month).await;
    assert_eq!(month.pending_resolution_count, 2);
    h.upgrade_oracle(&production).await;
    // 185 and 192 are gone; 194 waits for the failure.
    let cranker = h.payer();
    let early = instruction::settle_failed_oracle_month_escrow_v2(
        &program(),
        &cranker.pubkey(),
        &b.cohort.period,
        &b.cohort.month,
        &challenge_escrow(&b, 0),
    );
    assert_oracle_error(h.send(early, &[]).await, OracleError::InvalidOraclePhase);
    let everyone = vec![PARTICIPANT_USDC; b.owners().len()];
    end_month(&mut h, &b, &everyone).await;
    for (key, _, _) in &b.challenges {
        assert!(h.account(*key).await.is_none());
    }
}

/// After the Scramble, before the upgrade: three challenges, one dismissed by 169 when nobody
/// ruled by the Opening's start (its bond to the proposer: the deployed program's rule), one
/// called off by 192 (back to source submission, its bond still held), one still open. Source
/// submission with a challenge pending: kind 0 takes it; 194 returns the called-off and the open
/// one to the challenger.
#[tokio::test]
async fn batx_ends_after_the_scramble_with_settled_and_open_challenges_on_the_production_artifact()
{
    let Some((mut h, production, mut b)) = batx().await else {
        return;
    };
    close_sources(&mut h, &b, b.cohort.placement_end()).await;
    challenge(&mut h, &mut b, 0, None, 1).await;
    challenge(&mut h, &mut b, 1, None, 2).await;
    challenge(&mut h, &mut b, 2, None, 3).await;
    h.at(b.cohort.scramble_end()).await;
    let cranker = h.payer();
    h.send(
        instruction::settle_oracle_usdc_prelisting_escrow(
            &program(),
            &cranker.pubkey(),
            &b.cohort.period,
            &b.cohort.month,
            &challenge_escrow(&b, 0),
        ),
        &[],
    )
    .await
    .unwrap();
    let (key1, source1, _) = b.challenges[1];
    h.send(
        instruction::cancel_stale_oracle_source_challenge_v2(
            &program(),
            &cranker.pubkey(),
            &b.cohort.period,
            &b.cohort.month,
            &source1,
            &key1,
            None,
        ),
        &[],
    )
    .await
    .unwrap();
    let month: OracleMonthState = h.read_state(b.cohort.month).await;
    assert_eq!(
        (month.phase, month.pending_resolution_count),
        (OraclePhase::SourceSubmission, 1)
    );
    let dismissed: OracleSourceChallenge = h.read_state(b.challenges[0].0).await;
    assert_eq!(
        (dismissed.status, dismissed.escrow_disposition),
        (
            OracleChallengeStatus::Rejected,
            OracleEscrowDisposition::Slashed
        )
    );
    h.upgrade_oracle(&production).await;
    // The dismissed challenge's bond went to source 0's proposer, wallet 0, before the upgrade.
    let mut expected = vec![PARTICIPANT_USDC; b.owners().len()];
    expected[0] += CHALLENGE_MIN_BOND;
    *expected.last_mut().unwrap() -= CHALLENGE_MIN_BOND;
    end_month(&mut h, &b, &expected).await;
}

/// Nobody ended it before expiry: 79 failed the period with its month left in Scramble and a
/// challenge open. The production build has no 186; kind 0 on the Failed period only reopens.
#[tokio::test]
async fn batx_failed_at_expiry_in_scramble_still_ends_on_the_production_artifact() {
    let Some((mut h, production, mut b)) = batx().await else {
        return;
    };
    close_sources(&mut h, &b, b.cohort.placement_end()).await;
    challenge(&mut h, &mut b, 1, None, 1).await;
    h.upgrade_oracle(&production).await;
    clock(&mut h, b.cohort.expiry).await;
    let cranker = h.payer();
    h.send(
        instruction::fail_period(
            &program(),
            &cranker.pubkey(),
            &b.cohort.product,
            b.cohort.expiry,
        ),
        &[],
    )
    .await
    .unwrap();
    let month: OracleMonthState = h.read_state(b.cohort.month).await;
    assert_eq!(month.phase, OraclePhase::Scramble);
    // 195 cannot release from Scramble.
    assert_oracle_error(
        h.send(
            instruction::abort_oracle_usdc_reward_schedule_v2(
                &program(),
                &cranker.pubkey(),
                &b.cohort.period,
                &b.cohort.month,
            ),
            &[],
        )
        .await,
        OracleError::InvalidOraclePhase,
    );
    let everyone = vec![PARTICIPANT_USDC; b.owners().len()];
    end_month(&mut h, &b, &everyone).await;
}

/// The calendar fallback, no kind 0: anyone ran 184 and challenged before the upgrade. On the
/// production build, from the submission deadline (expiry less four days; 2026-11-27 00:00 UTC
/// for BATX Dec 1): 79 refuses a month in Scramble until expiry, so 169 first dismisses the
/// unruled challenge (its bond to the proposer, the deployed rule), 191 times one candidate out,
/// which puts the month back in source submission, then 79, the other refunds, 195 and the
/// closes.
#[tokio::test]
async fn batx_calendar_fallback_from_scramble_after_the_deadline_on_the_production_artifact() {
    let Some((mut h, production, mut b)) = batx().await else {
        return;
    };
    close_sources(&mut h, &b, b.cohort.placement_end()).await;
    challenge(&mut h, &mut b, 0, None, 1).await;
    h.upgrade_oracle(&production).await;
    // The formation build's calendar runs on 15-second test days; mainnet's on real ones. Give
    // the month the calendar mainnet's 184 writes: from four days before it to four after.
    // (`scramble_start_ts` and `listing_ts` sit at bytes 70 and 78, the phase at 86.)
    let deadline = b.cohort.expiry - 4 * DAY;
    let at_184 = deadline - 20 * DAY;
    let mut account = h.account(b.cohort.month).await.unwrap();
    account.data[70..78].copy_from_slice(&(at_184 - 4 * DAY).to_le_bytes());
    account.data[78..86].copy_from_slice(&(at_184 + 4 * DAY).to_le_bytes());
    h.ctx
        .set_account(&b.cohort.month, &AccountSharedData::from(account));
    let month: OracleMonthState = h.read_state(b.cohort.month).await;
    assert_eq!(
        (month.scramble_start_ts, month.listing_ts, month.phase),
        (at_184 - 4 * DAY, at_184 + 4 * DAY, OraclePhase::Scramble)
    );
    let cranker = h.payer();
    let crank =
        |ix: fn(&Pubkey, &Pubkey, &[u8; 32], u64) -> solana_sdk::instruction::Instruction| {
            ix(
                &program(),
                &cranker.pubkey(),
                &b.cohort.product,
                b.cohort.expiry,
            )
        };
    let expire = |n: usize| {
        let placed = b.source(n);
        instruction::expire_unlistable_oracle_source_v2(
            &program(),
            &cranker.pubkey(),
            &b.cohort.period,
            &b.cohort.month,
            &placed.key,
            Some(&b.cohort.sku_ids[placed.sku]),
        )
    };
    // Just before the deadline nothing fails: 79 waits, 191 waits (and the challenge holds it).
    clock(&mut h, deadline - 1).await;
    assert_oracle_error(
        h.send(crank(instruction::fail_period), &[]).await,
        OracleError::PeriodNotFailable,
    );
    assert!(h.send(expire(1), &[]).await.is_err());
    // From the deadline 79 still refuses Scramble; the challenge is dismissed (169), then 191.
    clock(&mut h, deadline).await;
    assert_oracle_error(
        h.send(crank(instruction::fail_period), &[]).await,
        OracleError::PeriodNotFailable,
    );
    h.send(
        instruction::settle_oracle_usdc_prelisting_escrow(
            &program(),
            &cranker.pubkey(),
            &b.cohort.period,
            &b.cohort.month,
            &challenge_escrow(&b, 0),
        ),
        &[],
    )
    .await
    .unwrap();
    h.send(expire(1), &[]).await.unwrap();
    let month: OracleMonthState = h.read_state(b.cohort.month).await;
    assert_eq!(month.phase, OraclePhase::SourceSubmission);
    h.send(crank(instruction::fail_period), &[]).await.unwrap();
    assert_eq!(
        h.period(b.cohort.product, b.cohort.expiry).await.status,
        PeriodStatus::Failed
    );
    let vault = h.bond_vault();
    let vault_before = h.token_balance(vault).await;
    refunds(&mut h, &b).await;
    let mut expected = vec![PARTICIPANT_USDC; b.owners().len()];
    expected[0] += CHALLENGE_MIN_BOND;
    *expected.last_mut().unwrap() -= CHALLENGE_MIN_BOND;
    assert_eq!(balances(&mut h, &b.owners()).await, expected);
    assert_eq!(h.token_balance(vault).await, vault_before);
    let recipient = Keypair::new().pubkey();
    close_everything(&mut h, &b, &recipient).await;
    still_production(&mut h, &b).await;
}

/// A challenge the oracle authority sent to the council before the upgrade (185, then 30/18 opens
/// its case, one seat even votes to keep the source): the review of bdecd849, finding 1. After
/// kind 0 nothing rules on it: 194 cannot settle it while its guards name the case, the council
/// takes no vote or application in a Failed period (past the case's deadline too, where the no-vote
/// fallback would keep the source and pay the bond to the proposer), and 30/18 operation 3 ends
/// each case unapplied, cancels its challenge and releases its guards; 194 then returns each bond
/// to the challenger, and the rest ends as ever. The cases and their rounds stay (no close).
#[tokio::test]
async fn batx_ends_with_challenges_before_the_council_on_the_production_artifact() {
    use ameba_oracle::{
        governance_gate::PINNED_CONTROLLER_PROGRAM_ID,
        instruction::OracleCouncilActionV1,
        processor::{decode_council, CouncilCase},
        state::{OracleEmergencyDisputeKind, OracleSourceChallengeOutcome},
    };
    use solana_sdk::instruction::AccountMeta;
    let Some((mut h, production, mut b)) = batx().await else {
        return;
    };
    close_sources(&mut h, &b, b.cohort.placement_end()).await;
    challenge(&mut h, &mut b, 1, None, 1).await;
    challenge(&mut h, &mut b, 2, Some(0), 2).await;
    // Day 7: the oracle authority sends both to the council (185, RuleReviewUnresolved).
    h.at(b.cohort.kill_end()).await;
    let oracle = h.oracle_authority.insecure_clone();
    for index in 0..2 {
        let (key, source, comparison) = b.challenges[index];
        let sku = b.cohort.sku_ids[b.sources.iter().find(|p| p.key == source).unwrap().sku];
        h.send(
            instruction::resolve_oracle_source_challenge_v2(
                &program(),
                &oracle.pubkey(),
                &b.cohort.month,
                &source,
                &key,
                &sku,
                comparison.as_ref(),
                OracleSourceChallengeOutcome::RuleReviewUnresolved,
            ),
            &[&oracle],
        )
        .await
        .unwrap();
    }
    let council = h.launch_council().await;
    // launch_council installs the controller's Config with 10,000,000 lamports outside the bank's
    // capitalization; the payer gives them back so that a long warp's accounts check holds.
    {
        use solana_sdk::account::{ReadableAccount, WritableAccount};
        let payer_key = h.ctx.payer.pubkey();
        let mut payer = AccountSharedData::from(h.account(payer_key).await.unwrap());
        payer.set_lamports(payer.lamports() - 10_000_000);
        h.ctx.set_account(&payer_key, &payer);
    }
    let (epoch, digest) = {
        let account = h.account(council.config).await.unwrap();
        let v = decode_council(
            &PINNED_CONTROLLER_PROGRAM_ID,
            &council.config,
            &account.owner,
            &account.data,
        )
        .unwrap();
        (v.epoch, v.digest)
    };
    let remaining = |b: &Batx, index: usize| {
        let (_, source, comparison) = b.challenges[index];
        let guard =
            |s: &Pubkey| derive_oracle_source_challenge_guard_pda(&program(), &b.cohort.month, s).0;
        let mut accounts = vec![
            AccountMeta::new_readonly(source, false),
            AccountMeta::new(guard(&source), false),
        ];
        if let Some(c) = comparison {
            accounts.push(AccountMeta::new_readonly(c, false));
            accounts.push(AccountMeta::new(guard(&c), false));
        }
        accounts
    };
    let action = |b: &Batx, index: usize, operation: u8, case_hash: [u8; 32], choice: u8| {
        let c = b.challenges[index].0;
        let challenge_id = [0xc0 + 1 + index as u8; 32];
        assert_eq!(
            instruction::find_oracle_source_challenge_address(
                &program(),
                &b.cohort.month,
                &b.challenges[index].1,
                &challenge_id
            ),
            c
        );
        OracleCouncilActionV1 {
            operation,
            kind: OracleEmergencyDisputeKind::Source,
            target_id: challenge_id,
            expected_case_hash: case_hash,
            expected_epoch: epoch,
            expected_seats_hash: digest,
            choice,
        }
    };
    let payer = h.payer();
    let council_ix =
        |b: &Batx, index: usize, act: OracleCouncilActionV1, voter: Option<&Pubkey>| {
            instruction::council_action(
                &program(),
                &payer.pubkey(),
                &b.cohort.period,
                &b.cohort.month,
                &b.challenges[index].0,
                &council.config,
                act,
                voter,
                remaining(b, index),
            )
        };
    let case_key = |b: &Batx, index: usize| {
        instruction::find_council_case_address(&program(), &b.cohort.month, &b.challenges[index].0)
    };
    // Anyone opens the cases; a seat votes to keep the first source (the fallback too).
    let mut hashes = Vec::new();
    for index in 0..2 {
        h.send(
            council_ix(&b, index, action(&b, index, 0, [0; 32], 0), None),
            &[],
        )
        .await
        .unwrap();
        let case: CouncilCase = h.read_state(case_key(&b, index)).await;
        hashes.push(case.case_hash);
    }
    let seat = council.seats[0].insecure_clone();
    h.send(
        council_ix(&b, 0, action(&b, 0, 1, hashes[0], 0), Some(&seat.pubkey())),
        &[&seat],
    )
    .await
    .unwrap();
    let challenger = b.challenger.pubkey();
    assert_eq!(
        h.collateral(&challenger).await.available_balance,
        PARTICIPANT_USDC - 2 * CHALLENGE_MIN_BOND
    );

    h.upgrade_oracle(&production).await;
    // Before the failure, operation 3 ends no source case.
    assert!(h
        .send(council_ix(&b, 0, action(&b, 0, 3, hashes[0], 0), None), &[])
        .await
        .is_err());
    let before = fail_month(&mut h, &b).await;
    // 194 cannot settle a challenge whose guards name its case.
    let cranker = h.payer();
    let early = instruction::settle_failed_oracle_month_escrow_v2(
        &program(),
        &cranker.pubkey(),
        &b.cohort.period,
        &b.cohort.month,
        &challenge_escrow(&b, 0),
    );
    assert_oracle_error(
        h.send(early, &[]).await,
        OracleError::InvalidOracleChallengeAccount,
    );
    // Past the case's deadline the council neither votes nor applies in a Failed period: the
    // no-vote fallback would keep the source and pay the bond to the proposer.
    let case: CouncilCase = h.read_state(case_key(&b, 0)).await;
    let clock: solana_sdk::clock::Clock = h.ctx.banks_client.get_sysvar().await.unwrap();
    h.advance_slots(case.deadline + 1 - clock.slot).await;
    for (act, voter) in [
        (action(&b, 0, 2, hashes[0], 0), None),
        (
            action(&b, 0, 1, hashes[0], 1),
            Some(council.seats[1].insecure_clone()),
        ),
    ] {
        let signers: Vec<&Keypair> = voter.iter().collect();
        let voter_key = voter.as_ref().map(|v| v.pubkey());
        assert_oracle_error(
            h.send(council_ix(&b, 0, act, voter_key.as_ref()), &signers)
                .await,
            OracleError::InvalidOraclePhase,
        );
    }
    // Operation 3 ends each case: the challenge Cancelled, its guards free, nothing pending.
    for index in 0..2 {
        h.send(
            council_ix(&b, index, action(&b, index, 3, hashes[index], 0), None),
            &[],
        )
        .await
        .unwrap();
        let case: CouncilCase = h.read_state(case_key(&b, index)).await;
        assert!(case.finalized && case.reason == 2 && case.seat_mask == 0);
        let c: OracleSourceChallenge = h.read_state(b.challenges[index].0).await;
        assert_eq!(
            (c.status, c.escrow_disposition),
            (
                OracleChallengeStatus::Cancelled,
                OracleEscrowDisposition::Unsettled
            )
        );
    }
    let month: OracleMonthState = h.read_state(b.cohort.month).await;
    assert_eq!(month.pending_resolution_count, 0);
    // A case that ended takes nothing more.
    assert!(h
        .send(council_ix(&b, 0, action(&b, 0, 3, hashes[0], 0), None), &[])
        .await
        .is_err());
    // 194 returns both bonds to the challenger; everyone ends whole.
    let everyone = vec![PARTICIPANT_USDC; b.owners().len()];
    finish_month(&mut h, &b, &everyone, before).await;
    // The cases and their rounds stay: 30/25 has no close for them.
    for index in 0..2 {
        assert!(h.account(case_key(&b, index)).await.is_some());
    }
}

/// The controller's council Config (`common::cfm`), its 10,000,000 lamports given back by the
/// payer so that a long warp's accounts check holds, with its epoch and seats hash.
async fn council_with_view(h: &mut Harness) -> (common::cfm::Council, u64, [u8; 32]) {
    use ameba_oracle::{governance_gate::PINNED_CONTROLLER_PROGRAM_ID, processor::decode_council};
    use solana_sdk::account::{ReadableAccount, WritableAccount};
    let council = h.launch_council().await;
    let payer_key = h.ctx.payer.pubkey();
    let mut payer = AccountSharedData::from(h.account(payer_key).await.unwrap());
    payer.set_lamports(payer.lamports() - 10_000_000);
    h.ctx.set_account(&payer_key, &payer);
    let account = h.account(council.config).await.unwrap();
    let view = decode_council(
        &PINNED_CONTROLLER_PROGRAM_ID,
        &council.config,
        &account.owner,
        &account.data,
    )
    .unwrap();
    (council, view.epoch, view.digest)
}

/// Re-review of f067295f: the Failed-period refusal is the source cases' alone. An Opening
/// council case of a month that never listed (its opening challenged, 164 sent it to the council,
/// the case opened) resolves as ever after 79 fails the period at expiry: past the case's deadline
/// its no-vote fallback applies (the challenge upheld, the claim rejected, both pending
/// resolutions cleared), and 169 pays the claim's stake and the challenge's bond to the
/// challenger, as before f067295f.
#[tokio::test]
async fn an_opening_case_still_settles_after_79_fails_the_period_on_the_production_artifact() {
    use ameba_oracle::{
        instruction::{
            ChallengeOracleOpeningClaimParams, OracleCouncilActionV1,
            SubmitOracleOpeningClaimParams,
        },
        processor::CouncilCase,
        state::{
            derive_user_collateral_pda, OracleEmergencyDisputeKind, OracleEscrowKind,
            OracleOpeningChallengeOutcome, OracleOpeningClaim, OracleOpeningClaimStatus,
        },
    };
    use solana_sdk::instruction::AccountMeta;
    let Some((mut h, production)) = start_formation().await else {
        return;
    };
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
    // The opening day: a claim, a challenge with another value, and 164 sends it to the council.
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
            SubmitOracleOpeningClaimParams {
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
    let challenger = h.participant(PARTICIPANT_USDC).await;
    h.upload_evidence(&challenger, 3, url.as_bytes()).await;
    let challenge_id = [0xa1; 32];
    h.send(
        instruction::challenge_oracle_opening_claim_v2(
            &program(),
            &challenger.pubkey(),
            &challenger.pubkey(),
            &cohort.period,
            &cohort.month,
            &source,
            &cohort.sku_ids[0],
            ChallengeOracleOpeningClaimParams {
                challenge_id,
                alternative_opening_state: OPENING_STATE + 7,
                alternative_source_time: capture,
                bond: CHALLENGE_MIN_BOND,
                canonical_locator_hash: locator_hash(&locator(&source_id)),
                source_definition_hash: definition_hash(&definition(&source_id)),
                archive_url: String::new(),
            },
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
            OracleOpeningChallengeOutcome::RuleReviewUnresolved,
        ),
        &[&oracle],
    )
    .await
    .unwrap();
    let (council, epoch, digest) = council_with_view(&mut h).await;
    let payer = h.payer();
    let council_ix = |operation: u8, case_hash: [u8; 32]| {
        instruction::council_action(
            &program(),
            &payer.pubkey(),
            &cohort.period,
            &cohort.month,
            &challenge_key,
            &council.config,
            OracleCouncilActionV1 {
                operation,
                kind: OracleEmergencyDisputeKind::Opening,
                target_id: challenge_id,
                expected_case_hash: case_hash,
                expected_epoch: epoch,
                expected_seats_hash: digest,
                choice: 0,
            },
            None,
            vec![
                AccountMeta::new(claim_key, false),
                AccountMeta::new(source, false),
            ],
        )
    };
    h.send(council_ix(0, [0; 32]), &[]).await.unwrap();
    let case_key =
        instruction::find_council_case_address(&program(), &cohort.month, &challenge_key);
    let case: CouncilCase = h.read_state(case_key).await;
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(
        (month.phase, month.pending_resolution_count),
        (OraclePhase::Opening, 2)
    );

    // The upgrade; the month never lists, and at expiry 79 fails its period.
    h.upgrade_oracle(&production).await;
    h.at(cohort.expiry).await;
    let cranker = h.payer();
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
    // Past the case's deadline its no-vote fallback applies, as before.
    let clock: solana_sdk::clock::Clock = h.ctx.banks_client.get_sysvar().await.unwrap();
    h.advance_slots(case.deadline + 1 - clock.slot).await;
    h.send(council_ix(2, case.case_hash), &[]).await.unwrap();
    let decided: CouncilCase = h.read_state(case_key).await;
    assert!(decided.finalized && decided.outcome == decided.fallback);
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.pending_resolution_count, 0);
    let claim: OracleOpeningClaim = h.read_state(claim_key).await;
    assert_eq!(claim.status, OracleOpeningClaimStatus::Rejected);
    // 169 pays the claim's stake and the challenge's bond to the challenger.
    h.send(
        instruction::settle_oracle_usdc_escrow(
            &program(),
            OracleEscrowKind::OpeningChallenge,
            vec![
                AccountMeta::new_readonly(cranker.pubkey(), true),
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
        h.collateral(&challenger.pubkey()).await.available_balance,
        PARTICIPANT_USDC + OPENING_BOND
    );
    assert_eq!(
        h.collateral(&claimant.pubkey()).await.available_balance,
        PARTICIPANT_USDC - OPENING_BOND
    );
    // All of it on the production artifact: a formation byte (186) is refused as unassigned.
    assert_oracle_error(
        h.send(
            instruction::reopen_oracle_sku_coverage(
                &program(),
                &cranker.pubkey(),
                &cohort.period,
                &cohort.month,
            ),
            &[],
        )
        .await,
        OracleError::InvalidInstructionData,
    );
}
