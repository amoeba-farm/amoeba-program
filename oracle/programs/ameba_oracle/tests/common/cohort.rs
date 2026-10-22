//! One cohort through the formation instructions: an active config, a registered product, a
//! Forming period with its month, a funded reward schedule and bonded participants.
use super::*;
use ameba_oracle::{
    constants::{
        ORACLE_KILL_WINDOW_SECONDS, ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS,
        ORACLE_PLACEMENT_WINDOW_SECONDS, ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS,
        ORACLE_SKU_EMPTY_HASH_DOMAIN, ORACLE_SKU_LEAF_HASH_DOMAIN, ORACLE_SKU_NODE_HASH_DOMAIN,
    },
    instruction::{
        AccumulateOracleActiveWeightGroupParams, AccumulateOracleRecipeBucketV2Params,
        AddOracleUsdcSkuBudgetParams, IndexOracleRecipeSourceV1Params,
        InitializeOracleMonthV5Params, ProposeOracleSourceV3Params, SubmitOracleOpeningClaimParams,
        SupportOracleSourceV3Params,
    },
    processor::{advance_oracle_weight_manifest_hash, initial_oracle_weight_manifest_hash},
    state::{derive_oracle_usdc_reward_schedule_pda, OracleSourceState},
};
use solana_program::hash::hashv;

pub const BASE: u64 = 100_000_000;
pub const LISTING_BOND: u64 = 10_000_000;
pub const SUPPORT_BOND: u64 = 4_000_000;
pub const OPENING_BOND: u64 = 3_000_000;
pub const UPDATE_MIN_BOND: u64 = 2_000_000;
pub const CHALLENGE_MIN_BOND: u64 = 1_000_000;
pub const CHALLENGE_MAX_BOND: u64 = 50_000_000;
pub const CHALLENGE_BOND_BPS: u16 = 2_000;
pub const SOURCE_REWARD: u64 = 6_000_000;
pub const OPENING_REWARD: u64 = 2_000_000;
pub const UPDATE_REWARD: u64 = 1_000_000;
pub const PARTICIPANT_USDC: u64 = 200_000_000;
pub const SOURCE_TYPE_HASH: [u8; 32] = [0x5e; 32];
pub const OPENING_STATE: u64 = 100_000_000;

/// Index-bound Merkle siblings of `sku_ids[index]`, from leaf to root.
pub fn sku_merkle_proof(sku_ids: &[[u8; 32]], index: u16) -> Vec<[u8; 32]> {
    let mut width = 1usize;
    while width < sku_ids.len() {
        width *= 2;
    }
    let mut level: Vec<[u8; 32]> = (0..width)
        .map(|i| {
            let i = i as u16;
            match sku_ids.get(usize::from(i)) {
                Some(id) => hashv(&[ORACLE_SKU_LEAF_HASH_DOMAIN, &i.to_le_bytes(), id]).to_bytes(),
                None => hashv(&[ORACLE_SKU_EMPTY_HASH_DOMAIN, &i.to_le_bytes()]).to_bytes(),
            }
        })
        .collect();
    let mut position = usize::from(index);
    let mut proof = Vec::new();
    while level.len() > 1 {
        proof.push(level[position ^ 1]);
        level = level
            .chunks_exact(2)
            .map(|pair| hashv(&[ORACLE_SKU_NODE_HASH_DOMAIN, &pair[0], &pair[1]]).to_bytes())
            .collect();
        position /= 2;
    }
    proof
}

#[derive(Clone)]
pub struct Cohort {
    pub product: [u8; 32],
    pub sku_ids: Vec<[u8; 32]>,
    pub root: [u8; 32],
    pub period: Pubkey,
    pub month: Pubkey,
    pub listing: u64,
    pub scramble: u64,
    pub expiry: u64,
}

impl Cohort {
    /// Index-bound Merkle siblings from leaf to root.
    pub fn sku_proof(&self, index: u16) -> Vec<[u8; 32]> {
        sku_merkle_proof(&self.sku_ids, index)
    }

    pub fn placement_end(&self) -> u64 {
        self.scramble + ORACLE_PLACEMENT_WINDOW_SECONDS
    }

    pub fn kill_end(&self) -> u64 {
        self.placement_end() + ORACLE_KILL_WINDOW_SECONDS
    }

    pub fn scramble_end(&self) -> u64 {
        self.kill_end() + ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
    }

    pub fn schedule(&self) -> Pubkey {
        derive_oracle_usdc_reward_schedule_pda(&program(), &self.month).0
    }

    pub fn source(&self, source_id: &[u8; 32]) -> Pubkey {
        instruction::find_oracle_source_address(&program(), &self.month, source_id)
    }
}

pub fn sku_budget(bucket_id: [u8; 32]) -> AddOracleUsdcSkuBudgetParams {
    AddOracleUsdcSkuBudgetParams {
        bucket_id,
        source_reward_budget: SOURCE_REWARD,
        opening_reward_budget: OPENING_REWARD,
        update_reward_budget: UPDATE_REWARD,
        proposer_reward_bps: 2_500,
        listing_bond: LISTING_BOND,
        support_bond: SUPPORT_BOND,
        opening_bond: OPENING_BOND,
        update_min_bond: UPDATE_MIN_BOND,
        challenge_min_bond: CHALLENGE_MIN_BOND,
        challenge_max_bond: CHALLENGE_MAX_BOND,
        challenge_bond_bps: CHALLENGE_BOND_BPS,
    }
}

pub fn locator(source_id: &[u8; 32]) -> Vec<u8> {
    format!(
        "https://example.com/source/{:02x}{:02x}",
        source_id[0], source_id[1]
    )
    .into_bytes()
}

pub fn definition(source_id: &[u8; 32]) -> Vec<u8> {
    let mut bytes = b"source-definition-v1\0USD\0spot\0".to_vec();
    bytes.extend_from_slice(source_id);
    bytes
}

pub fn locator_hash(bytes: &[u8]) -> [u8; 32] {
    hashv(&[b"locator", bytes]).to_bytes()
}

pub fn definition_hash(bytes: &[u8]) -> [u8; 32] {
    hashv(&[bytes]).to_bytes()
}

impl Harness {
    /// Active config, economics, a registered product of `sku_count` SKUs, the first rung's
    /// Forming period, its month in source submission and the reward vault.
    pub async fn open_cohort(&mut self, sku_count: u16) -> Cohort {
        self.initialize().await.unwrap();
        self.activate().await;
        // The month authority pays for the schedule and its SKU pools.
        let oracle_key = self.oracle_authority.pubkey();
        self.fund(&oracle_key, 5_000_000_000).await;
        self.configure_economics(0).await.unwrap();
        let product = product("RAMX");
        let sku_ids = synthetic_sku_ids(sku_count);
        let root = self.register_product(product, &sku_ids).await;
        let oracle = self.oracle_authority.insecure_clone();
        self.create_period(&oracle, product, PLANNED, EXPIRY, 6)
            .await
            .unwrap();
        let scramble = PLANNED - PROGRAM_PRE_LISTING_SECONDS;
        let ix = instruction::initialize_oracle_month_v5(
            &program(),
            &oracle.pubkey(),
            &self.ctx.payer.pubkey(),
            &product,
            EXPIRY,
            InitializeOracleMonthV5Params {
                scramble_start_ts: scramble,
                listing_ts: PLANNED,
                settlement_base_oracle_atomic: BASE,
                required_sku_root: root,
                required_sku_count: sku_count,
            },
        );
        self.send(ix, &[&oracle]).await.unwrap();
        let payer = self.ctx.payer.pubkey();
        let mint = self.usdc_mint;
        self.send(
            instruction::initialize_oracle_usdc_reward_vault(&program(), &payer, &mint),
            &[],
        )
        .await
        .unwrap();
        let period = find_period_address(&program(), &product, EXPIRY).0;
        Cohort {
            product,
            sku_ids,
            root,
            period,
            month: instruction::find_oracle_month_address(&program(), &period, EXPIRY),
            listing: PLANNED,
            scramble,
            expiry: EXPIRY,
        }
    }

    /// Begin the reward schedule, add one pool per SKU, fund the reward vault and freeze it.
    pub async fn fund_schedule(&mut self, cohort: &Cohort) {
        let oracle = self.oracle_authority.insecure_clone();
        self.send(
            instruction::begin_oracle_usdc_reward_schedule(
                &program(),
                &oracle.pubkey(),
                &cohort.period,
                &cohort.month,
            ),
            &[&oracle],
        )
        .await
        .unwrap();
        for sku in &cohort.sku_ids {
            self.send(
                instruction::add_oracle_usdc_sku_budget(
                    &program(),
                    &oracle.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    sku_budget(*sku),
                ),
                &[&oracle],
            )
            .await
            .unwrap();
        }
        let total = (SOURCE_REWARD + OPENING_REWARD + UPDATE_REWARD) * cohort.sku_ids.len() as u64;
        self.deposit_rewards(total).await;
        let mint = self.usdc_mint;
        self.send(
            instruction::finalize_oracle_usdc_reward_schedule(
                &program(),
                &oracle.pubkey(),
                &cohort.period,
                &cohort.month,
                &mint,
            ),
            &[&oracle],
        )
        .await
        .unwrap();
    }

    pub async fn deposit_rewards(&mut self, amount: u64) {
        let funder = Keypair::new();
        self.fund(&funder.pubkey(), 1_000_000_000).await;
        let token = self.create_ata(&funder.pubkey()).await;
        self.mint_usdc(&token, amount).await;
        let mint = self.usdc_mint;
        self.send(
            instruction::deposit_oracle_usdc_rewards(
                &program(),
                &funder.pubkey(),
                &token,
                &mint,
                amount,
            ),
            &[&funder],
        )
        .await
        .unwrap();
    }

    /// A funded wallet with an initialized ledger holding `usdc` available collateral.
    pub async fn participant(&mut self, usdc: u64) -> Keypair {
        let owner = Keypair::new();
        self.fund(&owner.pubkey(), 2_000_000_000).await;
        self.send(
            instruction::init_user_collateral(&program(), &owner.pubkey()),
            &[&owner],
        )
        .await
        .unwrap();
        let token = self.create_ata(&owner.pubkey()).await;
        self.mint_usdc(&token, usdc).await;
        let mint = self.usdc_mint;
        self.send(
            instruction::deposit_collateral(&program(), &owner.pubkey(), &token, &mint, usdc),
            &[&owner],
        )
        .await
        .unwrap();
        owner
    }

    /// Write and seal one evidence object in 192-byte chunks.
    pub async fn upload_evidence(&mut self, payer: &Keypair, kind: u8, bytes: &[u8]) -> [u8; 32] {
        let hash = match kind {
            1 => locator_hash(bytes),
            2 => definition_hash(bytes),
            3 | 4 => instruction::archive_url_hash(std::str::from_utf8(bytes).unwrap()),
            _ => panic!("unsupported evidence kind"),
        };
        for (index, chunk) in bytes.chunks(192).enumerate() {
            self.send(
                instruction::write_evidence(
                    &program(),
                    &payer.pubkey(),
                    kind,
                    hash,
                    bytes.len() as u16,
                    (index * 192) as u16,
                    chunk.to_vec(),
                ),
                &[payer],
            )
            .await
            .unwrap();
        }
        hash
    }

    pub fn propose_ix(
        &self,
        cohort: &Cohort,
        proposer: &Pubkey,
        source_id: [u8; 32],
        sku_index: u16,
    ) -> Instruction {
        let source_def = definition(&source_id);
        let source_loc = locator(&source_id);
        instruction::propose_oracle_source_v3(
            &program(),
            proposer,
            proposer,
            &cohort.period,
            &cohort.month,
            ProposeOracleSourceV3Params {
                source_id,
                bucket_id: cohort.sku_ids[usize::from(sku_index)],
                source_type_hash: SOURCE_TYPE_HASH,
                canonical_locator_hash: locator_hash(&source_loc),
                source_definition_hash: definition_hash(&source_def),
                listing_bond: LISTING_BOND,
                sku_index,
                sku_proof: cohort.sku_proof(sku_index),
            },
        )
    }

    /// Upload both preimages and propose `source_id` for SKU `sku_index`.
    pub async fn propose_source(
        &mut self,
        cohort: &Cohort,
        proposer: &Keypair,
        source_id: [u8; 32],
        sku_index: u16,
    ) -> Result<Pubkey, BanksClientError> {
        self.upload_evidence(proposer, 1, &locator(&source_id))
            .await;
        self.upload_evidence(proposer, 2, &definition(&source_id))
            .await;
        let ix = self.propose_ix(cohort, &proposer.pubkey(), source_id, sku_index);
        self.send(ix, &[proposer]).await?;
        Ok(cohort.source(&source_id))
    }

    pub async fn support_source(
        &mut self,
        cohort: &Cohort,
        supporter: &Keypair,
        source: &Pubkey,
        sku_index: u16,
    ) -> Result<(), BanksClientError> {
        let ix = instruction::support_oracle_source_v3(
            &program(),
            &supporter.pubkey(),
            &cohort.period,
            &cohort.month,
            source,
            &cohort.sku_ids[usize::from(sku_index)],
            SupportOracleSourceV3Params {
                stake: SUPPORT_BOND,
                sku_index,
                sku_proof: cohort.sku_proof(sku_index),
            },
        );
        self.send(ix, &[supporter]).await
    }

    /// Move the cluster clock; a fresh slot keeps otherwise identical transactions distinct.
    pub async fn at(&mut self, unix_timestamp: u64) {
        self.advance_slot().await;
        set_unix_timestamp(&self.ctx, unix_timestamp).await;
    }
}

/// One proposed and supported source per SKU, in SKU order.
pub struct Formed {
    pub proposer: Keypair,
    pub supporter: Keypair,
    pub sources: Vec<(Pubkey, [u8; 32])>,
}

pub fn formed_source_id(sku_index: u16) -> [u8; 32] {
    [10 + sku_index as u8; 32]
}

/// `YYYYMMDDhhmmss` in UTC, the Wayback capture timestamp of `unix`.
pub fn wayback_timestamp(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        y,
        m,
        d,
        secs / 3_600,
        (secs / 60) % 60,
        secs % 60
    )
}

pub fn archive_url(source_id: &[u8; 32], capture: u64) -> String {
    format!(
        "{}{}/{}",
        ameba_oracle::constants::ORACLE_OPENING_ARCHIVE_URL_PREFIX,
        wayback_timestamp(capture),
        String::from_utf8(locator(source_id)).unwrap()
    )
}

/// The bundle digest the test records name (the program checks only its form).
pub const RECORD_DIGEST: &str = "abababababababababababababababababababababababababababababababab";

/// An authenticated evidence record (evidence kind 4) of the source's locator at `capture`.
pub fn evidence_record(source_id: &[u8; 32], capture: u64) -> String {
    evidence_record_of(&String::from_utf8(locator(source_id)).unwrap(), capture)
}

/// An authenticated evidence record of `locator` at `capture`.
pub fn evidence_record_of(locator: &str, capture: u64) -> String {
    format!(
        "{}m=web;t={capture};d={RECORD_DIGEST};u={locator}",
        ameba_oracle::constants::ORACLE_EVIDENCE_RECORD_PREFIX
    )
}

impl Cohort {
    /// Equal recipe weights in basis points; the last bucket takes the remainder.
    pub fn bucket_weight(&self, sku_index: usize) -> u16 {
        let count = self.sku_ids.len() as u16;
        let base = 10_000 / count;
        if sku_index + 1 == self.sku_ids.len() {
            10_000 - base * (count - 1)
        } else {
            base
        }
    }

    /// The frozen recipe chain: `chain[i]` is the hash before source `i` was collected.
    pub fn recipe_chain(&self, formed: &Formed) -> Vec<[u8; 32]> {
        let count = formed.sources.len() as u16;
        let mut hash =
            initial_oracle_weight_manifest_hash(&self.month, count, self.sku_ids.len() as u16);
        let mut chain = Vec::new();
        for (index, (_, source_id)) in formed.sources.iter().enumerate() {
            chain.push(hash);
            let source = OracleSourceState {
                source_id: *source_id,
                source_type_hash: SOURCE_TYPE_HASH,
                canonical_locator_hash: locator_hash(&locator(source_id)),
                source_definition_hash: definition_hash(&definition(source_id)),
                ..OracleSourceState::default()
            };
            hash = advance_oracle_weight_manifest_hash(
                &hash,
                &self.sku_ids[index],
                &source,
                self.bucket_weight(index),
            );
        }
        chain.push(hash);
        chain
    }
}

impl Harness {
    pub async fn cover_skus(&mut self, cohort: &Cohort) -> Formed {
        let proposer = self.participant(PARTICIPANT_USDC).await;
        let supporter = self.participant(PARTICIPANT_USDC).await;
        let mut sources = Vec::new();
        for index in 0..cohort.sku_ids.len() as u16 {
            let source_id = formed_source_id(index);
            let source = self
                .propose_source(cohort, &proposer, source_id, index)
                .await
                .unwrap();
            self.support_source(cohort, &supporter, &source, index)
                .await
                .unwrap();
            sources.push((source, source_id));
        }
        Formed {
            proposer,
            supporter,
            sources,
        }
    }

    pub async fn finalize_coverage(&mut self, cohort: &Cohort) {
        let cranker = self.payer();
        self.send(
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
    }

    /// Recipe weights (187, 84, 86) inside the resolution-freeze window.
    pub async fn freeze_recipe(&mut self, cohort: &Cohort, formed: &Formed) {
        let oracle = self.oracle_authority.insecure_clone();
        self.send(
            instruction::begin_oracle_recipe_weights_v3(
                &program(),
                &oracle.pubkey(),
                &cohort.period,
                &cohort.month,
                formed.sources.len() as u16,
                cohort.sku_ids.len() as u16,
            ),
            &[&oracle],
        )
        .await
        .unwrap();
        for (index, (source, _)) in formed.sources.iter().enumerate() {
            self.send(
                instruction::accumulate_oracle_recipe_bucket_v2(
                    &program(),
                    &oracle.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    AccumulateOracleRecipeBucketV2Params {
                        bucket_id: cohort.sku_ids[index],
                        bucket_weight_bps: cohort.bucket_weight(index),
                        finalize_collection: true,
                    },
                    &[*source],
                ),
                &[&oracle],
            )
            .await
            .unwrap();
        }
        self.send(
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
    }

    /// Authenticate every frozen source in reverse recipe order (200).
    pub async fn index_recipe(&mut self, cohort: &Cohort, formed: &Formed) {
        let chain = cohort.recipe_chain(formed);
        let payer = self.payer();
        for index in (0..formed.sources.len()).rev() {
            let source_id = formed.sources[index].1;
            self.send(
                instruction::index_oracle_recipe_source_v1(
                    &program(),
                    &payer.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    IndexOracleRecipeSourceV1Params {
                        previous_hash: chain[index],
                        bucket_id: cohort.sku_ids[index],
                        source_id,
                        source_type_hash: SOURCE_TYPE_HASH,
                        canonical_locator_hash: locator_hash(&locator(&source_id)),
                        source_definition_hash: definition_hash(&definition(&source_id)),
                        bucket_weight_bps: cohort.bucket_weight(index),
                    },
                    0,
                ),
                &[],
            )
            .await
            .unwrap();
        }
    }

    /// Submit an opening for every source except those listed in `skip` (162), then accept them
    /// once their challenge windows pass (165). Returns the claimant.
    pub async fn open_sources(
        &mut self,
        cohort: &Cohort,
        formed: &Formed,
        skip: &[usize],
    ) -> Keypair {
        let claimant = self.participant(PARTICIPANT_USDC).await;
        let capture = cohort.scramble_end();
        for (index, (source, source_id)) in formed.sources.iter().enumerate() {
            if skip.contains(&index) {
                continue;
            }
            let url = archive_url(source_id, capture);
            self.upload_evidence(&claimant, 3, url.as_bytes()).await;
            self.send(
                instruction::submit_oracle_opening_claim_v2(
                    &program(),
                    &claimant.pubkey(),
                    &claimant.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    source,
                    &cohort.sku_ids[index],
                    SubmitOracleOpeningClaimParams {
                        opening_state: OPENING_STATE + index as u64,
                        source_time: capture,
                        stake: OPENING_BOND,
                        canonical_locator_hash: locator_hash(&locator(source_id)),
                        source_definition_hash: definition_hash(&definition(source_id)),
                        archive_url: String::new(),
                    },
                    &url,
                ),
                &[&claimant],
            )
            .await
            .unwrap();
        }
        self.advance_slots(ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS + 1)
            .await;
        let cranker = self.payer();
        for (index, (source, _)) in formed.sources.iter().enumerate() {
            if skip.contains(&index) {
                continue;
            }
            self.send(
                instruction::finalize_oracle_opening_claim_v2(
                    &program(),
                    &cranker.pubkey(),
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
        claimant
    }

    /// Active weights (121, 122 per source, 124).
    pub async fn build_active_weights(&mut self, cohort: &Cohort, formed: &Formed) {
        let oracle = self.oracle_authority.insecure_clone();
        self.send(
            instruction::begin_oracle_active_weights(
                &program(),
                &oracle.pubkey(),
                &cohort.period,
                &cohort.month,
                formed.sources.len() as u16,
                cohort.sku_ids.len() as u16,
            ),
            &[&oracle],
        )
        .await
        .unwrap();
        let cranker = self.payer();
        for (index, (source, _)) in formed.sources.iter().enumerate() {
            self.send(
                instruction::accumulate_oracle_active_weight_group(
                    &program(),
                    &cranker.pubkey(),
                    &cohort.period,
                    &cohort.month,
                    AccumulateOracleActiveWeightGroupParams {
                        group_id: cohort.sku_ids[index],
                        finalize_collection: true,
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
        self.send(
            instruction::finalize_oracle_active_weights(
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

    /// Everything from an opened cohort to a month ready for tag 80: coverage, recipe, index,
    /// openings and active weights. The clock is left inside the opening window.
    pub async fn form_to_opening(&mut self, cohort: &Cohort) -> Formed {
        self.fund_schedule(cohort).await;
        self.at(cohort.scramble).await;
        let formed = self.cover_skus(cohort).await;
        self.at(cohort.placement_end()).await;
        self.finalize_coverage(cohort).await;
        self.at(cohort.kill_end()).await;
        self.freeze_recipe(cohort, &formed).await;
        self.index_recipe(cohort, &formed).await;
        self.at(cohort.scramble_end()).await;
        self.open_sources(cohort, &formed, &[]).await;
        self.build_active_weights(cohort, &formed).await;
        formed
    }

    pub async fn advance_slots(&mut self, slots: u64) {
        let clock: solana_sdk::clock::Clock = self.ctx.banks_client.get_sysvar().await.unwrap();
        self.ctx.warp_to_slot(clock.slot + slots).unwrap();
        set_unix_timestamp(&self.ctx, clock.unix_timestamp as u64).await;
    }
}
