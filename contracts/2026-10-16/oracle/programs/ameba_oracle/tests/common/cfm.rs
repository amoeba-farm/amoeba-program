//! The CFM launch (tag 130) in the harness: the pinned registries compiled from the governed
//! documents, a plan of dated prints, the council of the build's profile and every launch step.
use super::{cohort::Cohort, *};
use ameba_oracle::{
    cfm::{
        self,
        compile::{compile_registry, CompiledRegistry, ParentRowInput},
        CfmPlan, CfmProduct, PoolTerms,
    },
    instruction::{self, CfmLaunchAccounts, IndexOracleRecipeSourceV1Params},
    processor::{advance_oracle_weight_manifest_hash, initial_oracle_weight_manifest_hash},
    state::OracleSourceState,
};

const REGISTRY_SOURCE: &str =
    include_str!("../../../../governance/cfm_parent_proxy_registry.v1.json");
const RAMX_MANIFEST: &str =
    include_str!("../../../../governance/ramx_product_sku_manifest.v1.json");
const NANDX_MANIFEST: &str =
    include_str!("../../../../governance/nandx_product_sku_manifest.v1.json");

pub const LAUNCH_BASE: u64 = 100_000_000;

/// The SKU pool terms of the launch plans: 1 USDC bonds, a 2 USDC update stake (the common update
/// helpers' stake), challenges 1-10 USDC.
pub const LAUNCH_TERMS: PoolTerms = PoolTerms {
    proposer_reward_bps: 2_500,
    challenge_bond_bps: 2_000,
    listing_bond: 1_000_000,
    support_bond: 1_000_000,
    opening_bond: 1_000_000,
    update_min_bond: super::cohort::UPDATE_MIN_BOND,
    challenge_min_bond: 1_000_000,
    challenge_max_bond: 10_000_000,
};

fn hex32(value: &serde_json::Value) -> [u8; 32] {
    let text = value.as_str().unwrap();
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap();
    }
    out
}

pub fn parent_rows() -> Vec<ParentRowInput> {
    let document: serde_json::Value = serde_json::from_str(REGISTRY_SOURCE).unwrap();
    document["parentRows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| ParentRowInput {
            id: row["id"].as_str().unwrap().into(),
            product: row["product"].as_str().unwrap().into(),
            parent_source_key: row["parentSourceKey"].as_str().unwrap().into(),
            parent_source_id: hex32(&row["parentSourceId"]),
            canonical_locator: row["canonicalLocator"].as_str().unwrap().into(),
            locator_hash: hex32(&row["locatorHash"]),
            definition_transcript: row["definitionTranscript"].as_str().unwrap().into(),
            definition_hash: hex32(&row["definitionHash"]),
            weight_bps: row["weightBps"].as_u64().unwrap() as u16,
            direct_assessment: row["directAssessment"].as_bool().unwrap(),
            children: row["children"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c.as_str().unwrap().into())
                .collect(),
        })
        .collect()
}

pub fn terminal_labels(product: &CfmProduct) -> Vec<String> {
    let source = if product.code == 0 {
        RAMX_MANIFEST
    } else {
        NANDX_MANIFEST
    };
    let manifest: serde_json::Value = serde_json::from_str(source).unwrap();
    manifest["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["subgroups"].as_array().unwrap().iter())
        .flat_map(|subgroup| subgroup["skuLabels"].as_array().unwrap().iter())
        .map(|label| label.as_str().unwrap().to_string())
        .collect()
}

pub fn registry(product: &CfmProduct) -> CompiledRegistry {
    compile_registry(*product, &parent_rows(), &terminal_labels(product)).unwrap()
}

/// A plan of one print per registry row, printed `age` seconds before `now`.
pub fn plan(product: &CfmProduct, expiry: u64, now: u64) -> CfmPlan {
    CfmPlan {
        product: *product,
        document_sha256: [0xd0; 32],
        expiry_ts: expiry,
        settlement_base_oracle_atomic: LAUNCH_BASE,
        terms: LAUNCH_TERMS,
        rows: (0..u64::from(product.parent_count))
            .map(|row| (50_000_000 + row * 1_000_000, now - 3_600 + row))
            .collect(),
    }
}

pub fn plan_preimage(plan: &CfmPlan) -> Vec<u8> {
    [cfm::PLAN_DOMAIN, program().as_ref(), &plan.encode()].concat()
}

/// The five seats of the council Config the build's council reads, and that Config.
pub struct Council {
    pub config: Pubkey,
    pub seats: Vec<Keypair>,
}

impl Council {
    pub fn three(&self) -> [Pubkey; 3] {
        core::array::from_fn(|i| self.seats[i].pubkey())
    }
}

/// One product's launch: its plan, sealed plan and registry objects and derived accounts.
pub struct Launch {
    pub product: CfmProduct,
    pub expiry: u64,
    pub plan: CfmPlan,
    pub plan_hash: [u8; 32],
    pub plan_object: Pubkey,
    pub registry: CompiledRegistry,
    pub registry_object: Pubkey,
    pub accounts: CfmLaunchAccounts,
}

impl Launch {
    pub fn source(&self, row: usize) -> Pubkey {
        self.accounts
            .row(&program(), &self.registry.parents[row].parent_source_id)
            .source
    }

    pub fn source_ids(&self) -> Vec<[u8; 32]> {
        self.registry
            .parents
            .iter()
            .map(|p| p.parent_source_id)
            .collect()
    }

    /// The month as a cohort, for the settlement helpers.
    pub fn cohort(&self, listing: u64) -> Cohort {
        let ids = self.source_ids();
        Cohort {
            product: self.product.product_id,
            root: sku_merkle_root(&ids),
            sku_ids: ids,
            period: self.accounts.period,
            month: self.accounts.month,
            listing,
            scramble: listing.saturating_sub(PROGRAM_PRE_LISTING_SECONDS),
            expiry: self.expiry,
        }
    }

    /// The frozen recipe chain over the registry rows: `chain[i]` precedes row `i`.
    pub fn recipe_chain(&self) -> Vec<[u8; 32]> {
        let count = self.product.parent_count as u16;
        let mut hash = initial_oracle_weight_manifest_hash(&self.accounts.month, count, count);
        let mut chain = vec![hash];
        for parent in &self.registry.parents {
            let frozen = OracleSourceState {
                source_id: parent.parent_source_id,
                source_type_hash: cfm::SOURCE_TYPE_HASH,
                canonical_locator_hash: parent.locator_hash,
                source_definition_hash: parent.definition_hash,
                ..OracleSourceState::default()
            };
            hash = advance_oracle_weight_manifest_hash(
                &hash,
                &parent.parent_source_id,
                &frozen,
                parent.weight_bps,
            );
            chain.push(hash);
        }
        chain
    }
}

impl Harness {
    /// The council the build reads: the controller's Config at its pinned address, or the dev
    /// profile's own Config (tag 243).
    pub async fn launch_council(&mut self) -> Council {
        let seats: Vec<Keypair> = (0..5).map(|_| Keypair::new()).collect();
        let keys: [Pubkey; 5] = core::array::from_fn(|i| seats[i].pubkey());
        #[cfg(feature = "localnet-dev-authority")]
        let config = {
            let admin = self.admin.insecure_clone();
            self.send(
                instruction::dev_council_config(
                    &program(),
                    &admin.pubkey(),
                    &self.ctx.payer.pubkey(),
                    keys,
                ),
                &[&admin],
            )
            .await
            .unwrap();
            instruction::find_dev_council_config_address(&program()).0
        };
        #[cfg(not(feature = "localnet-dev-authority"))]
        let config = {
            use borsh::BorshSerialize;
            use governance_controller_v3::state::{authority_pda, config_pda, Config, Timing};
            let controller = ameba_oracle::governance_gate::PINNED_CONTROLLER_PROGRAM_ID;
            let (config_key, bump) = config_pda(&controller);
            let controller_config = Config {
                discriminator: *b"AG3CFG01",
                version: 1,
                bump,
                initialized: true,
                controller,
                programdata: Pubkey::find_program_address(
                    &[controller.as_ref()],
                    &solana_sdk_ids::bpf_loader_upgradeable::ID,
                )
                .0,
                authority: authority_pda(&controller).0,
                treasury: Pubkey::new_unique(),
                seats: keys,
                council_epoch: 1,
                timing_version: 1,
                timing: Timing::default(),
                next_id: 1,
                reserved: [0; 37],
            };
            controller_config
                .validate(&controller, &config_key)
                .unwrap();
            self.ctx.set_account(
                &config_key,
                &AccountSharedData::from(Account {
                    lamports: 10_000_000,
                    data: controller_config.try_to_vec().unwrap(),
                    owner: controller,
                    executable: false,
                    rent_epoch: 0,
                }),
            );
            config_key
        };
        Council { config, seats }
    }

    /// Seal the product's registry and `plan` as kind-2 evidence objects, paid by the payer.
    pub async fn launch_objects(&mut self, product: &CfmProduct, plan: CfmPlan) -> Launch {
        let payer = self.payer();
        let registry = registry(product);
        let registry_hash = self.upload_evidence(&payer, 2, &registry.preimage()).await;
        assert_eq!(registry_hash, product.registry_hash);
        let plan_hash = self.upload_evidence(&payer, 2, &plan_preimage(&plan)).await;
        assert_eq!(plan_hash, cfm::plan_digest(&program(), &plan.encode()));
        Launch {
            product: *product,
            expiry: plan.expiry_ts,
            plan_hash,
            plan_object: instruction::find_evidence_object_address(
                &program(),
                &payer.pubkey(),
                2,
                &plan_hash,
            ),
            registry_object: instruction::find_evidence_object_address(
                &program(),
                &payer.pubkey(),
                2,
                &registry_hash,
            ),
            accounts: CfmLaunchAccounts::derive(&program(), product, plan.expiry_ts),
            registry,
            plan,
        }
    }

    pub fn begin_ix(&self, launch: &Launch, council: &Council, seats: [Pubkey; 3]) -> Instruction {
        instruction::cfm_launch_begin(
            &program(),
            &self.ctx.payer.pubkey(),
            &launch.product,
            launch.expiry,
            launch.plan_hash,
            &launch.plan_object,
            &launch.registry_object,
            &council.config,
            &seats,
        )
    }

    pub async fn begin_launch(
        &mut self,
        launch: &Launch,
        council: &Council,
    ) -> Result<(), BanksClientError> {
        let ix = self.begin_ix(launch, council, council.three());
        let seats: Vec<Keypair> = council.seats[..3]
            .iter()
            .map(|s| s.insecure_clone())
            .collect();
        let signers: Vec<&Keypair> = seats.iter().collect();
        self.send(ix, &signers).await
    }

    pub fn append_ix(&self, launch: &Launch, row: u8) -> Instruction {
        instruction::cfm_launch_append(
            &program(),
            &self.ctx.payer.pubkey(),
            &launch.product,
            launch.expiry,
            launch.plan_hash,
            &launch.plan_object,
            &launch.registry_object,
            row,
            &launch.registry.parents[usize::from(row)].parent_source_id,
        )
    }

    pub async fn append_rows(&mut self, launch: &Launch) {
        for row in 0..launch.product.parent_count {
            let ix = self.append_ix(launch, row);
            self.send(ix, &[]).await.unwrap();
        }
    }

    /// Tag 200 over the frozen recipe, in reverse order.
    pub async fn index_launch(&mut self, launch: &Launch) {
        let chain = launch.recipe_chain();
        let payer = self.ctx.payer.pubkey();
        for (at, parent) in launch.registry.parents.iter().enumerate().rev() {
            self.send(
                instruction::index_oracle_recipe_source_v1(
                    &program(),
                    &payer,
                    &launch.accounts.period,
                    &launch.accounts.month,
                    IndexOracleRecipeSourceV1Params {
                        previous_hash: chain[at],
                        bucket_id: parent.parent_source_id,
                        source_id: parent.parent_source_id,
                        source_type_hash: cfm::SOURCE_TYPE_HASH,
                        canonical_locator_hash: parent.locator_hash,
                        source_definition_hash: parent.definition_hash,
                        bucket_weight_bps: parent.weight_bps,
                    },
                    0,
                ),
                &[],
            )
            .await
            .unwrap();
        }
    }

    /// One accepted Game update of a launch row's source to `state`, printed `source_time`
    /// (166, 198, then 199 once the challenge window has passed). Leaves the clock a day later.
    pub async fn launch_update(
        &mut self,
        launch: &Launch,
        row: usize,
        state: u64,
        source_time: u64,
    ) {
        use super::updates::Print;
        use ameba_oracle::{instruction::UpdateFinalization, state::OracleUpdateClaimOutcome};
        let parent = &launch.registry.parents[row];
        let source = launch.source(row);
        let cohort = launch.cohort(0);
        let claimant = self.participant(super::cohort::PARTICIPANT_USDC).await;
        let url = format!(
            "{}{}/{}",
            ameba_oracle::constants::ORACLE_OPENING_ARCHIVE_URL_PREFIX,
            super::cohort::wayback_timestamp(source_time),
            parent.canonical_locator
        );
        self.upload_evidence(&claimant, 3, url.as_bytes()).await;
        let print = Print {
            state,
            source_time,
            evidence: instruction::update_evidence_hash(
                &launch.accounts.month,
                &source,
                &parent.parent_source_id,
                state,
                source_time,
                &parent.locator_hash,
                &parent.definition_hash,
                &url,
            ),
            url,
        };
        let claim = self
            .commit(
                &cohort,
                &claimant,
                &source,
                &parent.parent_source_id,
                row,
                [7; 32],
                print,
            )
            .await
            .unwrap();
        self.advance_slot().await;
        self.reveal(&cohort, &claimant, &source, &claim)
            .await
            .unwrap();
        let clock: solana_sdk::clock::Clock = self.ctx.banks_client.get_sysvar().await.unwrap();
        self.at(
            clock.unix_timestamp as u64 + ameba_oracle::constants::ORACLE_UPDATE_CHALLENGE_SECONDS
        )
        .await;
        self.finalize(
            &cohort,
            &source,
            row,
            &claim.key,
            OracleUpdateClaimOutcome::AcceptClaim,
            1,
            UpdateFinalization::Unchallenged,
        )
        .await
        .unwrap();
    }

    /// Begin, every row (the last lists the period) and the recipe index.
    pub async fn launch(&mut self, launch: &Launch, council: &Council) {
        self.begin_launch(launch, council).await.unwrap();
        self.append_rows(launch).await;
        self.index_launch(launch).await;
    }
}
