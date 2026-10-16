//! Evidence objects (tag 30, selectors 14-15): chunked writes, resume, sealing against the
//! committed hash, overwrite rejection and draft refunds.
mod common;

use ameba_oracle::instruction;
use common::*;
use solana_sdk::{hash::hashv, pubkey::Pubkey, signature::Signer};

async fn snapshot(h: &mut Harness, keys: &[Pubkey]) -> Vec<Option<(u64, Vec<u8>)>> {
    let mut out = Vec::new();
    for key in keys {
        out.push(h.account(*key).await.map(|a| (a.lamports, a.data)));
    }
    out
}

fn write(
    payer: &Pubkey,
    kind: u8,
    hash: [u8; 32],
    total: usize,
    offset: usize,
    bytes: &[u8],
) -> solana_sdk::instruction::Instruction {
    instruction::write_evidence(
        &program(),
        payer,
        kind,
        hash,
        total as u16,
        offset as u16,
        bytes.to_vec(),
    )
}

#[tokio::test]
async fn evidence_chunk_resume_seal_and_reject_overwrite() {
    let mut h = start().await;
    let payer = h.payer();
    let bytes = vec![65; 4096];
    let digest = hashv(&[&bytes]).to_bytes();
    let key = instruction::find_evidence_object_address(&program(), &payer.pubkey(), 2, &digest);
    h.send(
        write(&payer.pubkey(), 2, digest, 4096, 0, &bytes[..192]),
        &[],
    )
    .await
    .unwrap();
    let before = snapshot(&mut h, &[key]).await;
    // A chunk beyond the written prefix is rejected and changes nothing.
    assert!(h
        .send(
            write(&payer.pubkey(), 2, digest, 4096, 384, &bytes[384..576]),
            &[]
        )
        .await
        .is_err());
    assert_eq!(before, snapshot(&mut h, &[key]).await);
    // An exact replay of a written chunk succeeds without changing the object.
    h.advance_slot().await;
    h.send(
        write(&payer.pubkey(), 2, digest, 4096, 0, &bytes[..192]),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(before, snapshot(&mut h, &[key]).await);
    for offset in (192..4096).step_by(192) {
        h.send(
            write(
                &payer.pubkey(),
                2,
                digest,
                4096,
                offset,
                &bytes[offset..(offset + 192).min(4096)],
            ),
            &[],
        )
        .await
        .unwrap();
    }
    let sealed = snapshot(&mut h, &[key]).await;
    let object = h.account(key).await.unwrap();
    assert_eq!(object.owner, program());
    assert_eq!(object.data.len(), 76 + 4096);
    assert_eq!(object.data[75], 1, "sealed");
    assert_eq!(&object.data[76..], &bytes[..]);
    // A conflicting rewrite of a sealed object is rejected.
    assert!(h
        .send(write(&payer.pubkey(), 2, digest, 4096, 0, &[66; 192]), &[])
        .await
        .is_err());
    assert_eq!(sealed, snapshot(&mut h, &[key]).await);
    // Sealed objects are permanent.
    assert!(h
        .send(
            instruction::close_evidence_draft(&program(), &payer.pubkey(), &key),
            &[]
        )
        .await
        .is_err());
    assert_eq!(sealed, snapshot(&mut h, &[key]).await);

    // A draft whose bytes do not hash to its commitment never seals and can be refunded.
    let bad = [77; 32];
    let draft = instruction::find_evidence_object_address(&program(), &payer.pubkey(), 2, &bad);
    h.send(write(&payer.pubkey(), 2, bad, 200, 0, &bytes[..192]), &[])
        .await
        .unwrap();
    let draft_before = snapshot(&mut h, &[draft]).await;
    assert!(h
        .send(
            write(&payer.pubkey(), 2, bad, 200, 192, &bytes[192..200]),
            &[]
        )
        .await
        .is_err());
    assert_eq!(draft_before, snapshot(&mut h, &[draft]).await);
    h.send(
        instruction::close_evidence_draft(&program(), &payer.pubkey(), &draft),
        &[],
    )
    .await
    .unwrap();
    assert!(h.account(draft).await.is_none());
}

#[tokio::test]
async fn evidence_locator_rejects_credentials_and_binds_its_domain() {
    let mut h = start().await;
    let payer = h.payer();
    let locator = b"https://example.com/source";
    let digest = hashv(&[b"locator", locator]).to_bytes();
    h.send(
        write(&payer.pubkey(), 1, digest, locator.len(), 0, locator),
        &[],
    )
    .await
    .unwrap();
    let key = instruction::find_evidence_object_address(&program(), &payer.pubkey(), 1, &digest);
    assert_eq!(h.account(key).await.unwrap().data[75], 1);

    // The definition domain does not seal a locator preimage.
    let wrong_domain =
        instruction::find_evidence_object_address(&program(), &payer.pubkey(), 2, &digest);
    assert!(h
        .send(
            write(&payer.pubkey(), 2, digest, locator.len(), 0, locator),
            &[]
        )
        .await
        .is_err());
    assert!(h.account(wrong_domain).await.is_none());

    // A locator carrying credentials never seals.
    let secret = b"https://example.com/?api_key=secret";
    let secret_digest = hashv(&[b"locator", secret]).to_bytes();
    assert!(h
        .send(
            write(&payer.pubkey(), 1, secret_digest, secret.len(), 0, secret),
            &[]
        )
        .await
        .is_err());
}

mod canonical_month {
    use super::*;
    use ameba_oracle::{
        constants::{CURRENT_STATE_NAMESPACE_SEED, ORACLE_SOURCE_PDA_SEED},
        state::{OracleSourceState, OracleSourceStatus},
    };
    use common::cohort::*;
    use solana_sdk::signature::Keypair;

    fn backfill(
        cohort: &Cohort,
        payer: &Pubkey,
        object_payer: &Pubkey,
        source: &Pubkey,
        source_id: &[u8; 32],
    ) -> solana_sdk::instruction::Instruction {
        instruction::backfill_source_evidence(
            &program(),
            payer,
            object_payer,
            &cohort.period,
            &cohort.month,
            source,
            &locator_hash(&locator(source_id)),
            &definition_hash(&definition(source_id)),
        )
    }

    /// The month of a period is the PDA of the period key and the period's expiry. Backfill must
    /// accept that canonical month and publish the exact links a proposal would have published.
    #[tokio::test]
    async fn source_evidence_backfills_on_the_canonical_month() {
        let mut h = start().await;
        let cohort = h.open_cohort(1).await;
        let source_id = [0x51; 32];
        let (source, bump) = Pubkey::find_program_address(
            &[
                CURRENT_STATE_NAMESPACE_SEED,
                ORACLE_SOURCE_PDA_SEED,
                cohort.month.as_ref(),
                &source_id,
            ],
            &program(),
        );
        let proposer = Keypair::new();
        // A source whose publication links were never written.
        h.set_program_state(
            source,
            &OracleSourceState {
                is_initialized: true,
                bump,
                month: cohort.month,
                source_id,
                bucket_id: cohort.sku_ids[0],
                source_type_hash: [0x5e; 32],
                canonical_locator_hash: locator_hash(&locator(&source_id)),
                source_definition_hash: definition_hash(&definition(&source_id)),
                proposer: proposer.pubkey(),
                status: OracleSourceStatus::Candidate,
                ..OracleSourceState::default()
            },
            OracleSourceState::LEN,
        )
        .await;
        let payer = h.payer();
        let lh = locator_hash(&locator(&source_id));
        let dh = definition_hash(&definition(&source_id));
        let links = [
            instruction::find_evidence_link_address(&program(), &source, 0, &lh),
            instruction::find_evidence_link_address(&program(), &source, 1, &dh),
        ];
        assert_eq!(snapshot(&mut h, &links).await, vec![None, None]);

        // Without both sealed preimages nothing is published.
        h.upload_evidence(&payer, 1, &locator(&source_id)).await;
        assert!(h
            .send(
                backfill(
                    &cohort,
                    &payer.pubkey(),
                    &payer.pubkey(),
                    &source,
                    &source_id
                ),
                &[]
            )
            .await
            .is_err());
        assert_eq!(snapshot(&mut h, &links).await, vec![None, None]);
        h.upload_evidence(&payer, 2, &definition(&source_id)).await;

        h.send(
            backfill(
                &cohort,
                &payer.pubkey(),
                &payer.pubkey(),
                &source,
                &source_id,
            ),
            &[],
        )
        .await
        .unwrap();
        let published = snapshot(&mut h, &links).await;
        for (role, link) in links.iter().enumerate() {
            let account = h.account(*link).await.expect("link published");
            assert_eq!(account.owner, program());
            assert_eq!(account.data.len(), 251);
            assert_eq!(&account.data[..5], b"OEL\x01\x01");
            assert_eq!(&account.data[6..38], cohort.month.as_ref());
            assert_eq!(&account.data[38..70], source.as_ref());
            assert_eq!(account.data[166], role as u8);
        }

        // Exact replay changes nothing.
        h.advance_slot().await;
        h.send(
            backfill(
                &cohort,
                &payer.pubkey(),
                &payer.pubkey(),
                &source,
                &source_id,
            ),
            &[],
        )
        .await
        .unwrap();
        assert_eq!(snapshot(&mut h, &links).await, published);

        // Another payer's sealed copy of the same commitment keeps the original pointer.
        let sponsor = Keypair::new();
        h.fund(&sponsor.pubkey(), 1_000_000_000).await;
        h.upload_evidence(&sponsor, 1, &locator(&source_id)).await;
        h.upload_evidence(&sponsor, 2, &definition(&source_id))
            .await;
        h.send(
            backfill(
                &cohort,
                &sponsor.pubkey(),
                &sponsor.pubkey(),
                &source,
                &source_id,
            ),
            &[&sponsor],
        )
        .await
        .unwrap();
        assert_eq!(snapshot(&mut h, &links).await, published);

        // Only the month's own period authenticates it.
        let oracle = h.oracle_authority.insecure_clone();
        h.create_period(&oracle, cohort.product, JAN_01_2027, APR_01_2027, 6)
            .await
            .unwrap();
        let other =
            ameba_oracle_interface::find_period_address(&program(), &cohort.product, APR_01_2027).0;
        let mut wrong_period = backfill(
            &cohort,
            &payer.pubkey(),
            &payer.pubkey(),
            &source,
            &source_id,
        );
        wrong_period.accounts[8].pubkey = other;
        assert_oracle_error(
            h.send(wrong_period, &[]).await,
            ameba_oracle::error::OracleError::InvalidOracleOpeningEvidence,
        );
        let mut no_period = backfill(
            &cohort,
            &payer.pubkey(),
            &payer.pubkey(),
            &source,
            &source_id,
        );
        no_period.accounts.pop();
        assert_oracle_error(
            h.send(no_period, &[]).await,
            ameba_oracle::error::OracleError::InvalidOracleOpeningEvidence,
        );
    }

    /// A proposal publishes both links itself; a later backfill is a no-op on the same month.
    #[tokio::test]
    async fn proposal_links_survive_a_backfill_replay() {
        let mut h = start().await;
        let cohort = h.open_cohort(1).await;
        h.fund_schedule(&cohort).await;
        h.at(cohort.scramble).await;
        let proposer = h.participant(PARTICIPANT_USDC).await;
        let source_id = [0x52; 32];
        let source = h
            .propose_source(&cohort, &proposer, source_id, 0)
            .await
            .unwrap();
        let lh = locator_hash(&locator(&source_id));
        let dh = definition_hash(&definition(&source_id));
        let links = [
            instruction::find_evidence_link_address(&program(), &source, 0, &lh),
            instruction::find_evidence_link_address(&program(), &source, 1, &dh),
        ];
        let published = snapshot(&mut h, &links).await;
        assert!(published.iter().all(Option::is_some));
        h.send(
            backfill(
                &cohort,
                &proposer.pubkey(),
                &proposer.pubkey(),
                &source,
                &source_id,
            ),
            &[&proposer],
        )
        .await
        .unwrap();
        assert_eq!(snapshot(&mut h, &links).await, published);
    }
}
