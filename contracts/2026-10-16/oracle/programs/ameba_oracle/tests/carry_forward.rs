//! Carry-forward (tag 30 selectors 0-7): a Live month registers as the product's root, the next
//! rung registers as its successor, imports the parent's sources before any fresh proposal, and
//! opens them at the parent's last eligible checkpoint.
mod common;

use ameba_oracle::{
    error::OracleError,
    instruction::{self, CarryParent, InitializeOracleMonthV5Params},
    state::{OracleMonthState, OraclePhase, OracleSourceState, OracleSourceStatus},
};
use ameba_oracle_interface::{find_period_address, PeriodStatus};
use common::{cohort::*, *};
use solana_sdk::{pubkey::Pubkey, signature::Signer};

async fn go_live(h: &mut Harness, cohort: &Cohort) {
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
}

/// The next rung of the same product: a Forming period and its month in source submission.
async fn next_rung(h: &mut Harness, parent: &Cohort, listing: u64, expiry: u64) -> Cohort {
    let oracle = h.oracle_authority.insecure_clone();
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
    Cohort {
        product: parent.product,
        sku_ids: parent.sku_ids.clone(),
        root: parent.root,
        period,
        month: instruction::find_oracle_month_address(&program(), &period, expiry),
        listing,
        scramble,
        expiry,
    }
}

/// `head` of a source journal and `previous` of a checkpoint, read from their fixed layouts.
fn journal_head(data: &[u8]) -> Pubkey {
    Pubkey::new_from_array(data[38..70].try_into().unwrap())
}

fn checkpoint_previous(data: &[u8]) -> Pubkey {
    Pubkey::new_from_array(data[102..134].try_into().unwrap())
}

#[tokio::test]
async fn successor_imports_the_parent_recipe_and_opens_at_its_last_eligible_checkpoint() {
    let mut h = start().await;
    let parent = h.open_cohort(2).await;
    let parent_formed = h.form_to_opening(&parent).await;
    h.at(parent.listing).await;
    go_live(&mut h, &parent).await;
    let oracle = h.oracle_authority.insecure_clone();

    // Root registration on the Live parent, exactly once.
    let root = instruction::register_carry_root(
        &program(),
        &oracle.pubkey(),
        &parent.product,
        &parent.period,
        &parent.month,
    );
    let admin = h.admin.insecure_clone();
    let by_admin = instruction::register_carry_root(
        &program(),
        &admin.pubkey(),
        &parent.product,
        &parent.period,
        &parent.month,
    );
    assert_oracle_error(h.send(by_admin, &[&admin]).await, OracleError::Unauthorized);
    h.send(root.clone(), &[&oracle]).await.unwrap();
    h.advance_slot().await;
    assert!(h.send(root, &[&oracle]).await.is_err());

    // The next rung registers as the successor before its scramble and its reward schedule.
    let child = next_rung(&mut h, &parent, JAN_01_2027, APR_01_2027).await;
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
    h.fund_schedule(&child).await;
    h.at(child.scramble).await;

    // A fresh proposal waits for the import walk.
    let fresh_proposer = h.participant(PARTICIPANT_USDC).await;
    let fresh_id = [0x77; 32];
    h.upload_evidence(&fresh_proposer, 1, &locator(&fresh_id))
        .await;
    h.upload_evidence(&fresh_proposer, 2, &definition(&fresh_id))
        .await;
    let fresh = h.propose_ix(&child, &fresh_proposer.pubkey(), fresh_id, 0);
    assert_oracle_error(
        h.send(fresh.clone(), &[&fresh_proposer]).await,
        OracleError::InvalidOracleState,
    );

    // Import both parent sources in recipe order, with a new listing bond each.
    let importer = parent_formed.proposer.insecure_clone();
    let bonded_before = h.collateral(&importer.pubkey()).await.available_balance;
    let mut child_sources = Vec::new();
    for (index, (parent_source, source_id)) in parent_formed.sources.iter().enumerate() {
        let parent_side = CarryParent {
            period: parent.period,
            month: parent.month,
            source: *parent_source,
            bucket_id: parent.sku_ids[index],
            member_page: 0,
        };
        let import = instruction::import_carry_source(
            &program(),
            &importer.pubkey(),
            &importer.pubkey(),
            &child.period,
            &child.month,
            source_id,
            &locator_hash(&locator(source_id)),
            &definition_hash(&definition(source_id)),
            &parent_side,
            index as u16,
            child.sku_proof(index as u16),
        );
        if index == 1 {
            // The walk is ordered: the second source cannot be imported twice.
            h.send(import.clone(), &[&importer]).await.unwrap();
            h.advance_slot().await;
            assert!(h.send(import, &[&importer]).await.is_err());
        } else {
            h.send(import, &[&importer]).await.unwrap();
        }
        let source = child.source(source_id);
        let imported: OracleSourceState = h.read_state(source).await;
        assert_eq!(imported.status, OracleSourceStatus::Candidate);
        assert_eq!(imported.listing_bond_locked, LISTING_BOND);
        assert!(h
            .account(instruction::find_carry_source_address(&program(), &source))
            .await
            .is_some());
        child_sources.push((source, *source_id));
    }
    assert_eq!(
        h.collateral(&importer.pubkey()).await.available_balance,
        bonded_before - 2 * LISTING_BOND
    );
    // With the walk complete the import guard passes: the same proposal now reaches the SKU
    // membership check (made to fail here so the fresh source stays out of the recipe).
    let mut past_guard = fresh;
    let mut data = past_guard.data.clone();
    let proof_at = data.len() - 32;
    data[proof_at..].copy_from_slice(&[0xee; 32]);
    past_guard.data = data;
    h.advance_slot().await;
    assert_oracle_error(
        h.send(past_guard, &[&fresh_proposer]).await,
        OracleError::InvalidOracleSkuMembershipProof,
    );

    // Support the imported sources, freeze the child recipe over them and skip fresh openings.
    let supporter = h.participant(PARTICIPANT_USDC).await;
    for (index, (source, _)) in child_sources.iter().enumerate() {
        h.support_source(&child, &supporter, source, index as u16)
            .await
            .unwrap();
    }
    let child_formed = Formed {
        proposer: importer,
        supporter,
        sources: child_sources.clone(),
    };
    h.at(child.placement_end()).await;
    h.finalize_coverage(&child).await;
    let cranker = h.payer();
    h.at(child.kill_end()).await;
    h.freeze_recipe(&child, &child_formed).await;
    h.index_recipe(&child, &child_formed).await;

    // After the listing: select each source's opening from its parent's checkpoint journal.
    h.at(child.listing).await;
    for (index, (source, _)) in child_sources.iter().enumerate() {
        let parent_source = parent_formed.sources[index].0;
        let begin = instruction::begin_carry_selection(
            &program(),
            &cranker.pubkey(),
            &child.period,
            &child.month,
            source,
            &parent.month,
            &parent_source,
        );
        h.send(begin, &[]).await.unwrap();
        let journal = h
            .account(instruction::find_source_journal_address(
                &program(),
                &parent_source,
            ))
            .await
            .unwrap();
        let mut cursor = journal_head(&journal.data);
        while cursor != Pubkey::default() {
            let checkpoint = h.account(cursor).await.unwrap();
            h.send(
                instruction::scan_carry_checkpoint(&program(), source, &cursor),
                &[],
            )
            .await
            .unwrap();
            cursor = checkpoint_previous(&checkpoint.data);
        }
        h.send(
            instruction::freeze_carry_opening(
                &program(),
                &cranker.pubkey(),
                &child.period,
                &child.month,
                source,
                &child.sku_ids[index],
            ),
            &[],
        )
        .await
        .unwrap();
        let opened: OracleSourceState = h.read_state(*source).await;
        let parent_state: OracleSourceState = h.read_state(parent_source).await;
        assert_eq!(opened.status, OracleSourceStatus::Active);
        assert!(opened.opening_submitted);
        assert_eq!(opened.baseline_state, parent_state.baseline_state);
        assert_eq!(opened.baseline_state, OPENING_STATE + index as u64);
        assert_eq!(opened.observation_count, 1);
        assert!(h
            .account(instruction::find_checkpoint_address(
                &program(),
                source,
                &instruction::find_carry_source_address(&program(), source),
            ))
            .await
            .is_some());
    }
    let month: OracleMonthState = h.read_state(child.month).await;
    assert_eq!(month.phase, OraclePhase::Opening);
    assert_eq!(month.opened_source_count, 2);
    assert_eq!(month.opening_resolved_source_count, 2);

    // The carried month builds its active weights and lists like any other.
    h.build_active_weights(&child, &child_formed).await;
    go_live(&mut h, &child).await;
    let period = h.period(child.product, child.expiry).await;
    assert_eq!(period.status, PeriodStatus::Live);
    assert_eq!(period.effective_listing_ts, child.listing);
}
