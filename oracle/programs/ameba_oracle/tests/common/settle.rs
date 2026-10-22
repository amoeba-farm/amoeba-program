//! A Live one-SKU cohort with three sources in its bucket (the settlement floor), and its
//! post-expiry finalization and attested publication.
use super::{cohort::*, *};
use ameba_oracle::{
    constants::ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS,
    instruction::{
        self, AccumulateOracleActiveWeightGroupParams, AccumulateOracleRecipeBucketV2Params,
        AccumulateOracleSettlementSourceBucketParams, IndexOracleRecipeSourceV1Params,
        RecomputeOracleBucketMedianV1Params, SettlementComputation, SettlementLeaf,
        SubmitOracleOpeningClaimParams, TerminalCleanupParams,
    },
    processor::{advance_oracle_weight_manifest_hash, initial_oracle_weight_manifest_hash},
    state::{
        derive_oracle_bucket_median_pda, derive_oracle_settlement_source_manifest_pda,
        derive_settlement_signer_set_pda, OracleBucketMedianState, OracleBucketMedianStatus,
        OracleMonthState, OracleSettlementSourceManifest, OracleSourceState, OracleUsdcRewardKind,
        SettlementSignerSet,
    },
};
use solana_program::hash::hashv;
use solana_sdk::instruction::AccountMeta;

pub const WIDE_SOURCES: [[u8; 32]; 3] = [[10; 32], [11; 32], [12; 32]];
pub const WIDE_OPENINGS: [u64; 3] = [100_000_000, 102_000_000, 104_000_000];

/// A Live one-SKU cohort whose bucket holds three sources, the settlement floor.
pub struct Wide {
    pub cohort: Cohort,
    pub sources: Vec<Pubkey>,
    /// The proposer recorded on each source: the participant, or a sponsorship agreement.
    pub proposers: Vec<Pubkey>,
    pub proposer: Keypair,
    pub supporter: Keypair,
    pub claimant: Keypair,
    /// The funded sponsor of the middle source, when it is underwritten.
    pub sponsor: Option<Keypair>,
}

pub async fn wide_live(h: &mut Harness) -> Wide {
    wide_live_with(h, None).await
}

/// With `sponsored` terms (sponsor cash), a funded sponsor underwrites the middle source.
pub async fn wide_live_with(
    h: &mut Harness,
    sponsored: Option<ameba_oracle::sponsorship::SponsorTerms>,
) -> Wide {
    wide_live_paid(h, sponsored, false).await
}

/// As `wide_live_with`; with `sponsor_pays` the sponsor's own wallet pays its activation, as
/// petri sends it.
pub async fn wide_live_paid(
    h: &mut Harness,
    sponsored: Option<ameba_oracle::sponsorship::SponsorTerms>,
    sponsor_pays: bool,
) -> Wide {
    let wide = wide_placed_paid(h, sponsored, sponsor_pays).await;
    h.wide_coverage(&wide).await;
    h.wide_recipe(&wide).await;
    h.wide_openings(&wide).await;
    h.wide_active_weights(&wide).await;
    h.wide_list(&wide).await;
    wide
}

/// Tag 1 and 181 for the first rung, its funded reward schedule (156-158), then placement: three
/// sources proposed (182) and supported (183) in the single bucket.
pub async fn wide_placed(
    h: &mut Harness,
    sponsored: Option<ameba_oracle::sponsorship::SponsorTerms>,
) -> Wide {
    wide_placed_paid(h, sponsored, false).await
}

async fn wide_placed_paid(
    h: &mut Harness,
    sponsored: Option<ameba_oracle::sponsorship::SponsorTerms>,
    sponsor_pays: bool,
) -> Wide {
    let cohort = h.open_cohort(1).await;
    h.fund_schedule(&cohort).await;
    h.at(cohort.scramble).await;
    let sponsor = match sponsored {
        Some(_) => Some(h.participant(PARTICIPANT_USDC).await),
        None => None,
    };
    let proposer = h.participant(PARTICIPANT_USDC).await;
    let supporter = h.participant(PARTICIPANT_USDC).await;
    let claimant = h.participant(PARTICIPANT_USDC).await;
    let mut sources = Vec::new();
    let mut proposers = Vec::new();
    for (index, source_id) in WIDE_SOURCES.into_iter().enumerate() {
        let source = match (&sponsored, &sponsor, index) {
            (Some(terms), Some(sponsor), 1) => {
                let signer = sponsor.insecure_clone();
                h.upload_evidence(&signer, 1, &locator(&source_id)).await;
                h.upload_evidence(&signer, 2, &definition(&source_id)).await;
                let params = instruction::ProposeOracleSourceV3Params {
                    source_id,
                    bucket_id: cohort.sku_ids[0],
                    source_type_hash: SOURCE_TYPE_HASH,
                    canonical_locator_hash: locator_hash(&locator(&source_id)),
                    source_definition_hash: definition_hash(&definition(&source_id)),
                    listing_bond: LISTING_BOND,
                    sku_index: 0,
                    sku_proof: cohort.sku_proof(0),
                };
                let payer = if sponsor_pays {
                    signer.insecure_clone()
                } else {
                    h.payer()
                };
                h.send(
                    instruction::sponsored_activate(
                        &program(),
                        &payer.pubkey(),
                        &signer.pubkey(),
                        &signer.pubkey(),
                        &cohort.period,
                        &cohort.month,
                        terms.clone(),
                        params,
                    ),
                    &[&signer],
                )
                .await
                .unwrap();
                let source = cohort.source(&source_id);
                proposers.push(ameba_oracle::sponsorship::agreement_address(&program(), &source).0);
                source
            }
            _ => {
                proposers.push(proposer.pubkey());
                h.propose_source(&cohort, &proposer, source_id, 0)
                    .await
                    .unwrap()
            }
        };
        h.support_source(&cohort, &supporter, &source, 0)
            .await
            .unwrap();
        sources.push(source);
    }
    Wide {
        cohort,
        sources,
        proposers,
        proposer,
        supporter,
        claimant,
        sponsor,
    }
}

impl Harness {
    /// Coverage finalizes at the end of placement (184).
    pub async fn wide_coverage(&mut self, wide: &Wide) {
        self.at(wide.cohort.placement_end()).await;
        self.finalize_coverage(&wide.cohort).await;
    }

    /// The recipe: one bucket of three sources at the full weight (187, 84, 86), indexed in
    /// reverse order (200).
    pub async fn wide_recipe(&mut self, wide: &Wide) {
        let h = self;
        let cohort = &wide.cohort;
        let sources = &wide.sources;
        h.at(cohort.kill_end()).await;
        let oracle = h.oracle_authority.insecure_clone();
        let bucket = cohort.sku_ids[0];
        h.send(
            instruction::begin_oracle_recipe_weights_v3(
                &program(),
                &oracle.pubkey(),
                &cohort.period,
                &cohort.month,
                3,
                1,
            ),
            &[&oracle],
        )
        .await
        .unwrap();
        h.send(
            instruction::accumulate_oracle_recipe_bucket_v2(
                &program(),
                &oracle.pubkey(),
                &cohort.period,
                &cohort.month,
                AccumulateOracleRecipeBucketV2Params {
                    bucket_id: bucket,
                    bucket_weight_bps: 10_000,
                    finalize_collection: true,
                },
                sources,
            ),
            &[&oracle],
        )
        .await
        .unwrap();
        h.send(
            instruction::finalize_oracle_recipe_weights_v2(
                &program(),
                &oracle.pubkey(),
                &cohort.period,
                &cohort.month,
            ),
            &[&oracle],
        )
        .await
        .unwrap();
        let mut chain = vec![initial_oracle_weight_manifest_hash(&cohort.month, 3, 1)];
        for source_id in WIDE_SOURCES {
            let source = OracleSourceState {
                source_id,
                source_type_hash: SOURCE_TYPE_HASH,
                canonical_locator_hash: locator_hash(&locator(&source_id)),
                source_definition_hash: definition_hash(&definition(&source_id)),
                ..OracleSourceState::default()
            };
            let next = advance_oracle_weight_manifest_hash(
                chain.last().unwrap(),
                &bucket,
                &source,
                10_000,
            );
            chain.push(next);
        }
        let payer = h.payer();
        for index in (0..3).rev() {
            let source_id = WIDE_SOURCES[index];
            h.send(
                instruction::index_oracle_recipe_source_v1(
                    &program(),
                    &payer.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    IndexOracleRecipeSourceV1Params {
                        previous_hash: chain[index],
                        bucket_id: bucket,
                        source_id,
                        source_type_hash: SOURCE_TYPE_HASH,
                        canonical_locator_hash: locator_hash(&locator(&source_id)),
                        source_definition_hash: definition_hash(&definition(&source_id)),
                        bucket_weight_bps: 10_000,
                    },
                    0,
                ),
                &[],
            )
            .await
            .unwrap();
        }
    }

    /// Openings for all three (162), accepted after their challenge windows (165).
    pub async fn wide_openings(&mut self, wide: &Wide) {
        self.wide_openings_of(wide, &[0, 1, 2]).await;
    }

    /// Openings for the sources at `opened` only; the others stay unclaimed.
    pub async fn wide_openings_of(&mut self, wide: &Wide, opened: &[usize]) {
        let h = self;
        let cohort = &wide.cohort;
        let sources = &wide.sources;
        let bucket = cohort.sku_ids[0];
        let payer = h.payer();
        let claimant = wide.claimant.insecure_clone();
        h.at(cohort.scramble_end()).await;
        let capture = cohort.scramble_end();
        for (index, source) in sources.iter().enumerate() {
            if !opened.contains(&index) {
                continue;
            }
            let source_id = WIDE_SOURCES[index];
            let url = archive_url(&source_id, capture);
            h.upload_evidence(&claimant, 3, url.as_bytes()).await;
            h.send(
                instruction::submit_oracle_opening_claim_v2(
                    &program(),
                    &claimant.pubkey(),
                    &claimant.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    source,
                    &bucket,
                    SubmitOracleOpeningClaimParams {
                        opening_state: WIDE_OPENINGS[index],
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
        }
        h.advance_slots(ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS + 1)
            .await;
        for &index in opened {
            let source = &sources[index];
            h.send(
                instruction::finalize_oracle_opening_claim_v2(
                    &program(),
                    &payer.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    source,
                    &claimant.pubkey(),
                ),
                &[],
            )
            .await
            .unwrap();
        }
    }

    /// Active weights: the three sources collect into the bucket median (121, 122, 124).
    pub async fn wide_active_weights(&mut self, wide: &Wide) {
        let h = self;
        let cohort = &wide.cohort;
        let sources = &wide.sources;
        let bucket = cohort.sku_ids[0];
        let payer = h.payer();
        let oracle = h.oracle_authority.insecure_clone();
        h.send(
            instruction::begin_oracle_active_weights(
                &program(),
                &oracle.pubkey(),
                &cohort.period,
                &cohort.month,
                3,
                1,
            ),
            &[&oracle],
        )
        .await
        .unwrap();
        for (index, source) in sources.iter().enumerate() {
            h.send(
                instruction::accumulate_oracle_active_weight_group(
                    &program(),
                    &payer.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    AccumulateOracleActiveWeightGroupParams {
                        group_id: bucket,
                        finalize_collection: index == 2,
                    },
                    source,
                    0,
                    &solana_sdk::system_program::id(),
                ),
                &[],
            )
            .await
            .unwrap();
        }
        h.send(
            instruction::finalize_oracle_active_weights(
                &program(),
                &payer.pubkey(),
                &cohort.period,
                &cohort.month,
            ),
            &[],
        )
        .await
        .unwrap();
    }

    /// Tag 80 at the listing: the month enters Game and the period becomes Live.
    pub async fn wide_list(&mut self, wide: &Wide) {
        let cohort = &wide.cohort;
        let payer = self.payer();
        self.at(cohort.listing).await;
        self.send(
            instruction::finalize_oracle_opening_phase(
                &program(),
                &payer.pubkey(),
                &cohort.period,
                &cohort.month,
            ),
            &[],
        )
        .await
        .unwrap();
    }
}

impl Harness {
    /// After expiry and grace: walk the bucket median (197) source by source, then finalize the
    /// month (75) and its settlement-source manifest (113).
    pub async fn finalize_wide(&mut self, wide: &Wide) {
        let cohort = &wide.cohort;
        let payer = self.payer();
        let bucket = cohort.sku_ids[0];
        for source in &wide.sources {
            self.send(
                instruction::recompute_oracle_bucket_median_v1(
                    &program(),
                    &payer.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    RecomputeOracleBucketMedianV1Params {
                        bucket_id: bucket,
                        mode: 0,
                    },
                    source,
                    &instruction::find_oracle_source_observations_address(&program(), source),
                    0,
                    &solana_sdk::system_program::id(),
                ),
                &[],
            )
            .await
            .unwrap();
        }
        let median: OracleBucketMedianState = self
            .read_state(derive_oracle_bucket_median_pda(&program(), &cohort.month, &bucket).0)
            .await;
        assert_eq!(median.status, OracleBucketMedianStatus::SettlementReady);
        assert_eq!(median.eligible_source_count, 3);
        self.send(
            instruction::finalize_oracle_month(
                &program(),
                &payer.pubkey(),
                &cohort.period,
                &cohort.month,
                &[bucket],
            ),
            &[],
        )
        .await
        .unwrap();
        self.send(
            instruction::accumulate_oracle_settlement_source_bucket(
                &program(),
                &payer.pubkey(),
                &cohort.period,
                &cohort.month,
                AccumulateOracleSettlementSourceBucketParams {
                    bucket_id: bucket,
                    finalize_collection: true,
                },
                &[bucket],
            ),
            &[],
        )
        .await
        .unwrap();
    }

    /// The settlement leaf of a finalized month, as a publisher builds it from chain state.
    pub async fn leaf_for(&mut self, cohort: &Cohort) -> SettlementLeaf {
        let month: OracleMonthState = self.read_state(cohort.month).await;
        let manifest: OracleSettlementSourceManifest = self
            .read_state(derive_oracle_settlement_source_manifest_pda(&program(), &cohort.month).0)
            .await;
        let set: SettlementSignerSet = self
            .read_state(derive_settlement_signer_set_pda(&program(), 1).0)
            .await;
        let factor = 10_000i128 + i128::from(month.index_delta_bps);
        let value = (i128::from(month.settlement_base_oracle_atomic) * factor + 5_000) / 10_000;
        SettlementLeaf {
            schema_version: SettlementLeaf::CURRENT_SCHEMA_VERSION,
            oracle_month: cohort.month,
            recipe_hash: month.recipe_hash,
            underlying_id: cohort.product,
            item_id: "ramx".into(),
            expiry_id: "RAMX-202703".into(),
            settlement_ts: cohort.expiry,
            price_display_decimals: 6,
            computation: SettlementComputation::OracleIndexDelta,
            trailing_window_days: 0,
            observations: vec![],
            settlement_price_atomic: value as u64,
            source_uri: "https://oracle.example/ramx/RAMX-202703".into(),
            source_digest: manifest.rolling_source_digest,
            base_oracle_atomic: month.settlement_base_oracle_atomic,
            index_delta_bps: month.index_delta_bps,
            submitted_by: Pubkey::default(),
            submitted_slot: 0,
            signer_set_version: 1,
            signer_set_hash: set.set_hash,
        }
    }

    /// Attestations by `signers` over `attested`, then tag 116 publishing `leaf`.
    pub fn publication(
        &self,
        cohort: &Cohort,
        signers: &[&Keypair],
        attested: &SettlementLeaf,
        leaf: &SettlementLeaf,
    ) -> Vec<Instruction> {
        let set_hash = attested.signer_set_hash;
        let message =
            instruction::settlement_attestation_message(&program(), 1, &set_hash, attested);
        let mut ixs: Vec<Instruction> = signers
            .iter()
            .map(|signer| ed25519_verify_instruction(signer, &message))
            .collect();
        ixs.push(instruction::upsert_settlement_v3(
            &program(),
            &self.oracle_authority.pubkey(),
            &cohort.product,
            cohort.expiry,
            1,
            leaf.clone(),
        ));
        ixs
    }

    pub async fn publish(
        &mut self,
        cohort: &Cohort,
        signers: &[&Keypair],
        attested: &SettlementLeaf,
        leaf: &SettlementLeaf,
    ) -> Result<(), solana_program_test::BanksClientError> {
        let ixs = self.publication(cohort, signers, attested, leaf);
        let oracle = self.oracle_authority.insecure_clone();
        self.send_many(ixs, &[&oracle]).await
    }
}

pub fn record_commitment(leaf: &SettlementLeaf) -> [u8; 32] {
    hashv(&[
        b"ameba-settlement-record-v2",
        leaf.signed_payload_bytes().unwrap().as_slice(),
    ])
    .to_bytes()
}

impl Harness {
    /// Settle every counted pre-listing escrow of a wide cohort: each source's listing bond and
    /// support stake.
    pub async fn settle_wide_escrows(&mut self, wide: &Wide) {
        self.settle_wide_escrows_except(wide, None).await
    }

    /// As `settle_wide_escrows`, leaving `skip`'s listing bond for its owner to settle.
    pub async fn settle_wide_escrows_except(&mut self, wide: &Wide, skip: Option<&Pubkey>) {
        let cranker = self.payer();
        for (source, proposer) in wide.sources.iter().zip(&wide.proposers) {
            for escrow in [
                instruction::PrelistingEscrow::ListingBond {
                    source: *source,
                    proposer: *proposer,
                },
                instruction::PrelistingEscrow::SupportStake {
                    source: *source,
                    supporter: wide.supporter.pubkey(),
                },
            ] {
                if matches!(escrow, instruction::PrelistingEscrow::ListingBond { .. })
                    && skip == Some(source)
                {
                    continue;
                }
                self.send(
                    instruction::settle_oracle_usdc_prelisting_escrow(
                        &program(),
                        &cranker.pubkey(),
                        &wide.cohort.period,
                        &wide.cohort.month,
                        &escrow,
                    ),
                    &[],
                )
                .await
                .unwrap();
            }
        }
    }
}

impl Harness {
    /// Tag 174 for `recipient`; returns the amount credited to its ledger.
    pub async fn claim_reward(
        &mut self,
        cohort: &Cohort,
        recipient: &Keypair,
        kind: OracleUsdcRewardKind,
        subject: &Pubkey,
        trailing: Vec<AccountMeta>,
    ) -> Result<u64, solana_program_test::BanksClientError> {
        let mint = self.usdc_mint;
        let before = self.collateral(&recipient.pubkey()).await.available_balance;
        let ix = instruction::claim_oracle_usdc_reward(
            &program(),
            &recipient.pubkey(),
            &cohort.period,
            &cohort.month,
            &mint,
            kind,
            subject,
            trailing,
        );
        self.send(ix, &[recipient]).await?;
        let after = self.collateral(&recipient.pubkey()).await.available_balance;
        Ok(after - before)
    }

    /// Tag 30 selector 24 by the admin, refunding to the admin; `target` is hashed as found.
    pub async fn cleanup(
        &mut self,
        cohort: &Cohort,
        kind: u8,
        bucket_id: [u8; 32],
        target: Option<Pubkey>,
        extra: Option<Pubkey>,
    ) -> Result<(), solana_program_test::BanksClientError> {
        let (expected_data_hash, expected_lamports) = match target {
            Some(key) => {
                let account = self.account(key).await.expect("cleanup target exists");
                (hashv(&[&account.data]).to_bytes(), account.lamports)
            }
            None => ([0; 32], 0),
        };
        let admin = self.admin.insecure_clone();
        let ix = instruction::terminal_cleanup(
            &program(),
            &admin.pubkey(),
            &cohort.period,
            &cohort.month,
            &admin.pubkey(),
            &cohort.product,
            None,
            TerminalCleanupParams {
                kind,
                bucket_id,
                expected_data_hash,
                expected_lamports,
            },
            target.as_ref(),
            extra.as_ref(),
        );
        self.send(ix, &[&admin]).await
    }
}
