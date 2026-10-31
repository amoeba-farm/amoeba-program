//! Oracle v2, heads on pikes (tag 210), part B end to end: claims (fills and updates through
//! commit and reveal), challenges ruled by the dev council (tag 243), source challenges, listing
//! a contract on a Forming period, collecting and allocating its settlement weights, settling
//! every component and the contract, and the fetchers' prizes, on a Token-2022 AMBA with a
//! 25 bp transfer fee and the RAMX molds. Plus the briefs' focused cases and the security edges
//! of every part B instruction.
#![cfg(feature = "localnet-dev-authority")]

mod common;

use ameba_commodity_spec::{molds, Product};
use ameba_oracle::{
    error::OracleError,
    governance_gate::{
        derive_protocol_gate_pda, GovernanceInstructionTailV1, PINNED_CONTROLLER_PROGRAM_ID,
    },
    heads_view,
    instruction::{
        self,
        heads::{self as hx, heads_claim_commitment, heads_reason_hash},
    },
    processor::decode_council,
    state::heads::{
        find_head_address, find_head_challenge_address, find_head_claim_address,
        find_head_delegation_address, find_heads_amba_vault_address, find_heads_authority_address,
        find_heads_board_address, find_heads_config_address, find_heads_contract_address,
        find_heads_epoch_address, find_heads_stake_address, find_heads_weights_address,
        heads_listing_mix_commitment, heads_mix_commitment_field, heads_settlement_digest,
        Delegation, Head, HeadAccessClass, HeadChallengeReason, HeadClaim, HeadClaimKind,
        HeadClaimStatus, HeadStatus, HeadsAccount, HeadsBoard, HeadsChannel, HeadsConfig,
        HeadsContract, HeadsContractStatus, HeadsEpoch, HeadsMixBasis, HeadsParams,
        HeadsSettlementTarget, HeadsStake, HeadsWeightSource, HeadsWeightsHeader,
        COMPONENT_LAST_VALID, COMPONENT_LIVE, COMPONENT_SETTLED, DEGRADED_FREEZE_CHECK_FAILED,
        DEGRADED_TOO_FEW_ORIGINS, DEGRADED_TOO_LITTLE_WEIGHT, HEADS_SETTLE_MAX_BATCH,
        HEAD_PRINT_CAPACITY, REFERENCE_ASSUMED, REFERENCE_AVAILABLE, REFERENCE_COMPUTED,
        RING_FREEZE_CAPTURED, SOURCE_CASE_FELL, SOURCE_CASE_STOOD, WEIGHT_SOURCE_DUPLICATE,
        WEIGHT_SOURCE_ELIGIBLE, WEIGHT_SOURCE_FREEZE_LEVEL, WEIGHT_SOURCE_LEVEL,
        WEIGHT_SOURCE_RETIRED,
    },
};
use ameba_oracle_heads::{channels, liability, median, rewards, settle, time};
use ameba_oracle_interface::{find_period_address, PeriodStatus};
use common::*;
use solana_program_test::BanksClientError;
use solana_sdk::{
    account::AccountSharedData,
    compute_budget::ComputeBudgetInstruction,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use spl_token_2022::extension::ExtensionType;

const AMBA: u64 = 1_000_000_000;
const USDC: u64 = 1_000_000;
const DAY: u64 = 86_400;
const FEE_BPS: u16 = 25;

fn t22() -> Pubkey {
    spl_token_2022::id()
}

/// The test boards' parameters: the spec's, with thresholds that do not grow with the board (so
/// heads can be posted in a batch).
fn test_params() -> HeadsParams {
    let mut p = HeadsParams::spec_defaults();
    p.alpha_bps = 0;
    p.beta_bps = 0;
    // Three independent origins per settling component (a board parameter; the default is 1).
    p.min_live_sources = 3;
    p
}

fn origin(publisher: &str) -> [u8; 32] {
    molds::origin_id(publisher, "monthly contract price table")
}

#[derive(Clone, Copy)]
struct Pending {
    id: u32,
    seq: u32,
}

struct Env {
    h: Harness,
    mint: Pubkey,
    mint_authority: Keypair,
    product: [u8; 32],
    board: Pubkey,
    now: u64,
    seats: Vec<Keypair>,
    council: Pubkey,
    params: HeadsParams,
    /// The board's items: bucket ids and default mixes.
    buckets: Vec<[u8; 32]>,
    mixes: Vec<hx::HeadsListedMix>,
    /// On the production artifact (`production_on`): every instruction carries the governance
    /// envelope.
    governed: bool,
    /// The full-size benchmarks' production artifact and test build (`bench_elfs`).
    production: Option<(Vec<u8>, Vec<u8>)>,
    /// A non-RAMX listing's terms document is the largest kind-2 object (4,096 bytes) of near
    /// matches of its commitment field, the field itself last (round 10 review, CU).
    adverse_terms: bool,
    /// Heads a `SettleComponent` call records (`settle`): 8, the benchmarks the program's maximum.
    settle_chunk: usize,
}

impl Env {
    /// Sends with a full heap frame and compute limit, as clients of the big accounts do.
    async fn send(
        &mut self,
        ix: Instruction,
        signers: &[&Keypair],
    ) -> Result<(), BanksClientError> {
        let ix = self.admit(ix);
        self.h
            .send_many(
                vec![
                    ComputeBudgetInstruction::request_heap_frame(256 * 1024),
                    ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
                    ix,
                ],
                signers,
            )
            .await
    }

    /// `ix` as the program now admits it: on the production artifact, with the governance tail
    /// and the controller's gate (`dispatch::process_top_level_instruction`).
    fn admit(&self, mut ix: Instruction) -> Instruction {
        if self.governed && ix.program_id == program() {
            ix.data
                .extend_from_slice(&GovernanceInstructionTailV1::for_epoch(1).encode());
            ix.accounts.push(AccountMeta::new_readonly(
                derive_protocol_gate_pda(&PINNED_CONTROLLER_PROGRAM_ID, &program()).0,
                false,
            ));
        }
        ix
    }

    /// Sends and reports the compute units (the SBF artifact meters them; natively zero).
    async fn measure(&mut self, label: &str, ix: Instruction, signers: &[&Keypair]) -> u64 {
        let ix = self.admit(ix);
        let payer = self.h.payer();
        let mut all = vec![&payer];
        for s in signers {
            if s.pubkey() != payer.pubkey() && !all.iter().any(|a| a.pubkey() == s.pubkey()) {
                all.push(s);
            }
        }
        let blockhash = self.h.ctx.get_new_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[
                ComputeBudgetInstruction::request_heap_frame(256 * 1024),
                ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
                ix,
            ],
            Some(&payer.pubkey()),
            &all,
            blockhash,
        );
        let size = bincode_len(&tx);
        // The harness's one path (`common::process_now`), as `send` takes.
        let (result, units) = process_now(&self.h.ctx.banks_client, tx).await;
        result.expect(label);
        assert!(size <= 1232, "{label}: {size} bytes");
        eprintln!("CU {label}: {units} ({size} bytes)");
        units
    }

    async fn warp(&mut self, to: u64) {
        assert!(to >= self.now);
        self.now = to;
        set_unix_timestamp(&self.h.ctx, to).await;
        self.h.advance_slot().await;
    }

    async fn read<T: HeadsAccount>(&mut self, key: Pubkey) -> Box<T> {
        let account = self.h.account(key).await.expect("account exists");
        assert_eq!(account.owner, program());
        T::unpack(&account.data).expect("valid heads account")
    }

    fn head_key(&self, id: u32) -> Pubkey {
        find_head_address(&program(), &self.board, id).0
    }

    fn claim_key(&self, id: u32, seq: u32) -> Pubkey {
        find_head_claim_address(&program(), &self.head_key(id), seq).0
    }

    async fn head(&mut self, id: u32) -> Box<Head> {
        let key = self.head_key(id);
        self.read(key).await
    }

    async fn claim(&mut self, p: Pending) -> Box<HeadClaim> {
        let key = self.claim_key(p.id, p.seq);
        self.read(key).await
    }

    async fn board(&mut self) -> Box<HeadsBoard> {
        let key = self.board;
        self.read(key).await
    }

    async fn config(&mut self) -> Box<HeadsConfig> {
        self.read(find_heads_config_address(&program()).0).await
    }

    async fn stake_of(&mut self, owner: &Pubkey) -> Box<HeadsStake> {
        self.read(find_heads_stake_address(&program(), owner).0)
            .await
    }

    async fn usdc(&mut self, owner: &Pubkey) -> u64 {
        self.h.collateral(owner).await.available_balance
    }

    async fn amba_account(&mut self, owner: &Pubkey, amount: u64) -> Pubkey {
        let create =
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &self.h.ctx.payer.pubkey(),
                owner,
                &self.mint,
                &t22(),
            );
        self.h.send(create, &[]).await.unwrap();
        let ata = spl_associated_token_account::get_associated_token_address_with_program_id(
            owner,
            &self.mint,
            &t22(),
        );
        if amount > 0 {
            let mint = spl_token_2022::instruction::mint_to(
                &t22(),
                &self.mint,
                &ata,
                &self.mint_authority.pubkey(),
                &[],
                amount,
            )
            .unwrap();
            let authority = self.mint_authority.insecure_clone();
            self.h.send(mint, &[&authority]).await.unwrap();
        }
        ata
    }

    /// A key with SOL, `amba` AMBA staked into the family (what arrives, the fee off) and `usdc`
    /// USDC in its `UserCollateral`.
    async fn user(&mut self, amba: u64, usdc: u64) -> Keypair {
        let key = Keypair::new();
        self.h.fund(&key.pubkey(), 5_000_000_000).await;
        if amba > 0 {
            let ata = self.amba_account(&key.pubkey(), amba).await;
            let ix = hx::stake(
                &program(),
                &key.pubkey(),
                &key.pubkey(),
                &ata,
                &self.mint,
                &t22(),
                amba,
            );
            self.send(ix, &[&key]).await.unwrap();
        }
        let token = self.h.create_ata(&key.pubkey()).await;
        self.h
            .send(
                instruction::init_user_collateral(&program(), &key.pubkey()),
                &[&key],
            )
            .await
            .unwrap();
        if usdc > 0 {
            self.h.mint_usdc(&token, usdc).await;
            let ix = instruction::deposit_collateral(
                &program(),
                &key.pubkey(),
                &token,
                &self.h.usdc_mint,
                usdc,
            );
            self.h.send(ix, &[&key]).await.unwrap();
        }
        key
    }

    /// Writes and seals a kind-2 evidence object; returns its commitment and address.
    async fn object(&mut self, payer: &Keypair, bytes: &[u8]) -> ([u8; 32], Pubkey) {
        let hash = solana_program::hash::hashv(&[bytes]).to_bytes();
        for (index, chunk) in bytes.chunks(192).enumerate() {
            let ix = instruction::write_evidence(
                &program(),
                &payer.pubkey(),
                2,
                hash,
                bytes.len() as u16,
                (index * 192) as u16,
                chunk.to_vec(),
            );
            self.h.send(ix, &[payer]).await.unwrap();
        }
        (
            hash,
            instruction::find_evidence_object_address(&program(), &payer.pubkey(), 2, &hash),
        )
    }

    /// Posts a head of `amount` on `component` and returns its id.
    async fn post(&mut self, proposer: &Keypair, component: u8, amount: u64) -> u32 {
        let id = self.board().await.head_count;
        let spec = format!("head {id} on component {component}: a distributor's monthly table");
        let (hash, object) = self.object(proposer, spec.as_bytes()).await;
        let ix = hx::post_head(
            &program(),
            &proposer.pubkey(),
            &proposer.pubkey(),
            &self.product,
            id,
            component,
            HeadAccessClass::PublicPage,
            amount,
            &format!("source {id}"),
            hash,
            &object,
        )
        .unwrap();
        self.send(ix, &[proposer]).await.unwrap();
        id
    }

    /// `post`, measured (on the production artifact when the benchmark has one).
    async fn post_measured(&mut self, proposer: &Keypair, component: u8, amount: u64) -> u64 {
        let id = self.board().await.head_count;
        let spec = format!("head {id} on component {component}: a distributor's monthly table");
        let (hash, object) = self.object(proposer, spec.as_bytes()).await;
        let ix = hx::post_head(
            &program(),
            &proposer.pubkey(),
            &proposer.pubkey(),
            &self.product,
            id,
            component,
            HeadAccessClass::PublicPage,
            amount,
            &format!("source {id}"),
            hash,
            &object,
        )
        .unwrap();
        self.production_on().await;
        let units = self.measure("PostHead", ix, &[proposer]).await;
        self.production_off().await;
        units
    }

    async fn delegate(&mut self, owner: &Keypair, id: u32, amount: u64) {
        let ix = hx::delegate(
            &program(),
            &owner.pubkey(),
            &owner.pubkey(),
            &self.product,
            id,
            amount,
        );
        self.send(ix, &[owner]).await.unwrap();
    }

    /// Posts (A) and backs (B, the majority) a head until it is Active.
    async fn active_head(&mut self, a: &Keypair, b: &Keypair, component: u8) -> u32 {
        let id = self.post(a, component, 1_000 * AMBA).await;
        self.delegate(b, id, 1_500 * AMBA).await;
        assert_eq!(self.head(id).await.status, HeadStatus::Active);
        id
    }

    fn reveal_params(
        value: u64,
        source_ts: u64,
        origin: [u8; 32],
        lta: hx::HeadsLtaTerms,
        seq: u32,
    ) -> hx::RevealClaimParams {
        hx::RevealClaimParams {
            value,
            source_ts,
            origin_id: origin,
            lta,
            salt: [seq as u8 ^ 0x5a; 32],
            evidence_kind: 0,
            evidence_commitment: [0; 32],
        }
    }

    fn commit_ix(
        &self,
        fetcher: &Pubkey,
        id: u32,
        seq: u32,
        kind: HeadClaimKind,
        reveal: &hx::RevealClaimParams,
    ) -> Instruction {
        let hash = heads_claim_commitment(&self.head_key(id), seq, fetcher, reveal);
        hx::commit_claim(
            &program(),
            fetcher,
            fetcher,
            &self.product,
            id,
            seq,
            kind,
            hash,
            self.params.claim_bond,
        )
    }

    /// Commits a claim and reveals it in the next slot; returns where it is.
    #[allow(clippy::too_many_arguments)]
    async fn claim_with(
        &mut self,
        fetcher: &Keypair,
        id: u32,
        kind: HeadClaimKind,
        value: u64,
        source_ts: u64,
        origin: [u8; 32],
        lta: hx::HeadsLtaTerms,
    ) -> Pending {
        let seq = self.head(id).await.claim_seq;
        let reveal = Self::reveal_params(value, source_ts, origin, lta, seq);
        let ix = self.commit_ix(&fetcher.pubkey(), id, seq, kind, &reveal);
        self.measure("CommitClaim", ix, &[fetcher]).await;
        self.h.advance_slot().await;
        let ix = hx::reveal_claim(
            &program(),
            &fetcher.pubkey(),
            &self.product,
            id,
            seq,
            reveal,
            None,
        )
        .unwrap();
        self.measure("RevealClaim", ix, &[fetcher]).await;
        Pending { id, seq }
    }

    async fn fill(
        &mut self,
        fetcher: &Keypair,
        id: u32,
        value: u64,
        source_ts: u64,
        origin: [u8; 32],
    ) -> Pending {
        self.claim_with(
            fetcher,
            id,
            HeadClaimKind::Fill,
            value,
            source_ts,
            origin,
            Default::default(),
        )
        .await
    }

    async fn update(
        &mut self,
        fetcher: &Keypair,
        id: u32,
        value: u64,
        source_ts: u64,
        origin: [u8; 32],
    ) -> Pending {
        self.claim_with(
            fetcher,
            id,
            HeadClaimKind::Update,
            value,
            source_ts,
            origin,
            Default::default(),
        )
        .await
    }

    fn finalize_ix(&self, p: Pending, fetcher: &Pubkey) -> Instruction {
        hx::finalize_claim(
            &program(),
            &self.h.ctx.payer.pubkey(),
            &self.product,
            p.id,
            p.seq,
            fetcher,
            time::month_id(self.now),
        )
    }

    async fn finalize(&mut self, p: Pending, fetcher: &Pubkey) -> u64 {
        let ix = self.finalize_ix(p, fetcher);
        self.measure("FinalizeClaim", ix, &[]).await
    }

    fn challenge_params(bond: u64, reason: HeadChallengeReason) -> hx::ChallengeClaimParams {
        hx::ChallengeClaimParams {
            bond,
            reason,
            alternative_value: 0,
            alternative_source_ts: 0,
            evidence_kind: 0,
            evidence_commitment: [0; 32],
            reason_hash: heads_reason_hash(b"the table shows another number"),
        }
    }

    async fn challenge(
        &mut self,
        challenger: &Keypair,
        p: Pending,
        bond: u64,
    ) -> Result<(), BanksClientError> {
        let ix = hx::challenge_claim(
            &program(),
            &challenger.pubkey(),
            &challenger.pubkey(),
            &self.product,
            p.id,
            p.seq,
            Self::challenge_params(bond, HeadChallengeReason::ValueOrOther),
            None,
        )
        .unwrap();
        self.send(ix, &[challenger]).await
    }

    async fn epoch(&mut self) -> u64 {
        let account = self.h.account(self.council).await.unwrap();
        decode_council(&program(), &self.council, &account.owner, &account.data)
            .unwrap()
            .epoch
    }

    fn seats3(&self, which: [usize; 3]) -> ([Pubkey; 3], Vec<Keypair>) {
        (
            which.map(|i| self.seats[i].pubkey()),
            which
                .iter()
                .map(|&i| self.seats[i].insecure_clone())
                .collect(),
        )
    }

    async fn rule(&mut self, p: Pending, challenger: &Pubkey, won: bool) {
        let epoch = self.epoch().await;
        let (keys, signers) = self.seats3([0, 2, 4]);
        let ix = hx::rule_claim_challenge(
            &program(),
            &self.council,
            &keys,
            &self.product,
            p.id,
            p.seq,
            challenger,
            won,
            epoch,
        );
        let refs: Vec<&Keypair> = signers.iter().collect();
        self.measure("RuleChallenge", ix, &refs).await;
    }

    async fn rule_source(&mut self, id: u32, challenger: &Pubkey, won: bool) {
        let epoch = self.epoch().await;
        let (keys, signers) = self.seats3([1, 2, 3]);
        let ix = hx::rule_source_challenge(
            &program(),
            &self.council,
            &keys,
            &self.product,
            id,
            challenger,
            won,
            epoch,
        );
        let refs: Vec<&Keypair> = signers.iter().collect();
        self.measure("RuleSourceChallenge", ix, &refs).await;
    }

    async fn settle_claim_challenge(
        &mut self,
        p: Pending,
        challenger: &Pubkey,
    ) -> Result<(), BanksClientError> {
        let ix = hx::settle_claim_challenge(
            &program(),
            &self.product,
            p.id,
            p.seq,
            challenger,
            challenger,
        );
        self.send(ix, &[]).await
    }

    async fn sync(&mut self, id: u32, owner: &Pubkey) {
        let ix = hx::sync_delegation(&program(), &self.product, id, owner, owner);
        self.send(ix, &[]).await.unwrap();
    }

    async fn create_period(&mut self, planned: u64, expiry: u64) -> Pubkey {
        let oracle = self.h.oracle_authority.insecure_clone();
        let product = self.product;
        self.h
            .create_period(&oracle, product, planned, expiry, 6)
            .await
            .unwrap();
        find_period_address(&program(), &self.product, expiry).0
    }

    /// The items' mixes a listing at `expiry` carries: RAMX's quarter mixes from the molds,
    /// else the board's own.
    fn listing_mixes(&self, expiry: u64) -> Vec<hx::HeadsListedMix> {
        if self.product != product("RAMX") {
            return self.mixes.clone();
        }
        molds::listing_mixes(Product::Ramx, expiry as i64)
            .into_iter()
            .map(|(mix_bps, basis)| hx::HeadsListedMix {
                mix_bps,
                mix_basis: HeadsMixBasis::from_byte(basis.code()).unwrap(),
            })
            .collect()
    }

    /// The sealed listing terms document of a listing at `expiry`: RAMX's from the molds, else
    /// one that carries only the board's mixes' commitment.
    async fn terms(&mut self, expiry: u64) -> ([u8; 32], Pubkey) {
        let admin = self.h.admin.insecure_clone();
        let document = if self.product == product("RAMX") {
            molds::listing_terms_document(Product::Ramx, expiry as i64)
        } else {
            let (year, quarter) = time::contract_quarter(expiry);
            let mixes: Vec<_> = self.mixes.iter().map(|m| m.as_pair()).collect();
            let commitment = heads_listing_mix_commitment(
                &self.product,
                time::quarter_start(year, quarter),
                &self.buckets,
                &mixes,
            );
            let field = heads_mix_commitment_field(&commitment);
            let mut document = format!("listing terms of {expiry} ").into_bytes();
            if self.adverse_terms {
                // Near matches, each differing from the field in its last byte only.
                let mut decoy = field;
                decoy[field.len() - 1] = b'x';
                while document.len() + 2 * field.len() <= 4_096 {
                    document.extend_from_slice(&decoy);
                }
                document.resize(4_096 - field.len(), b' ');
            }
            document.extend_from_slice(&field);
            document
        };
        self.object(&admin, &document).await
    }

    /// A quarterly Forming period (selector 28), signed by the oracle authority.
    async fn create_quarter_period(
        &mut self,
        planned: u64,
        expiry: u64,
    ) -> Result<(), BanksClientError> {
        let oracle = self.h.oracle_authority.insecure_clone();
        let ix = hx::create_quarter_period(
            &program(),
            &self.h.ctx.payer.pubkey(),
            &oracle.pubkey(),
            &self.product,
            planned,
            expiry,
        )
        .unwrap();
        self.send(ix, &[&oracle]).await
    }

    fn list_ix(
        &self,
        expiry: u64,
        target: HeadsSettlementTarget,
        terms: ([u8; 32], Pubkey),
        base: u64,
    ) -> Instruction {
        hx::list_contract(
            &program(),
            &self.h.ctx.payer.pubkey(),
            &self.h.oracle_authority.pubkey(),
            &self.product,
            expiry,
            base,
            target,
            &self.listing_mixes(expiry),
            terms.0,
            &terms.1,
        )
        .unwrap()
    }

    async fn list(&mut self, expiry: u64, base: u64) {
        let terms = self.terms(expiry).await;
        let ix = self.list_ix(expiry, HeadsSettlementTarget::ContractChannel, terms, base);
        let oracle = self.h.oracle_authority.insecure_clone();
        self.measure("ListContract", ix, &[&oracle]).await;
    }

    /// The start of contract `expiry`'s freeze-level window (its first five sample instants
    /// come after it).
    async fn freeze_window(&mut self, expiry: u64) -> u64 {
        let slot = usize::from(self.contract(expiry).await.ring_slot);
        self.board().await.contract_freeze_window_ts[slot]
    }

    async fn contract(&mut self, expiry: u64) -> Box<HeadsContract> {
        let period = find_period_address(&program(), &self.product, expiry).0;
        self.read(find_heads_contract_address(&program(), &period).0)
            .await
    }

    fn weights_key(&self, expiry: u64, component: u8) -> Pubkey {
        let period = find_period_address(&program(), &self.product, expiry).0;
        let contract = find_heads_contract_address(&program(), &period).0;
        find_heads_weights_address(&program(), &contract, component).0
    }

    /// The weights header and its source records, if the account exists.
    async fn weights(
        &mut self,
        expiry: u64,
        component: u8,
    ) -> Option<(HeadsWeightsHeader, Vec<HeadsWeightSource>)> {
        let account = self.h.account(self.weights_key(expiry, component)).await?;
        let (header, layout) = HeadsWeightsHeader::unpack_account(&account.data).unwrap();
        let sources = (0..usize::from(header.source_count))
            .map(|i| HeadsWeightSource::unpack(&account.data[layout.source_range(i)]).unwrap())
            .collect();
        Some((header, sources))
    }

    /// The heads `SettleComponent` records for `component`: the contract's listed sources of it,
    /// then the board's live sources of it that went live before the freeze, then `extra`.
    async fn component_sources(&mut self, expiry: u64, component: u8, extra: &[u32]) -> Vec<u32> {
        let contract = self.contract(expiry).await;
        let board = self.board().await;
        let mut ids: Vec<u32> = (0..usize::from(contract.listed_count))
            .filter(|&i| contract.listed_components[i] == component)
            .map(|i| contract.listed_head_ids[i])
            .collect();
        for i in 0..usize::from(board.live_count) {
            let id = board.live_head_ids[i];
            if board.live_components[i] == component
                && board.live_since_ts[i] < contract.freeze_ts
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
        for &id in extra {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    }

    /// Settles one component: records every source's level and weight, `settle_chunk` heads a call (plus
    /// `extra` heads to append); the call that completes the record settles it. Returns the CU of
    /// each call.
    async fn settle(&mut self, expiry: u64, component: u8, extra: &[u32]) -> Vec<(String, u64)> {
        // Records open only after the cutoff K (expiry plus the challenge window).
        let cutoff = expiry + self.params.challenge_window_secs;
        if self.now <= cutoff {
            self.warp(cutoff + 1).await;
        }
        let ids = self.component_sources(expiry, component, extra).await;
        let payer = self.h.ctx.payer.pubkey();
        let mut cu = Vec::new();
        let mut first = 0usize;
        self.production_on().await;
        loop {
            let chunk = &ids[first..(first + self.settle_chunk).min(ids.len())];
            let ix = hx::settle_component(
                &program(),
                &payer,
                &self.product,
                expiry,
                component,
                first as u16,
                chunk,
            )
            .unwrap();
            cu.push((
                format!(
                    "SettleComponent c{component} ({} sources from {first})",
                    chunk.len()
                ),
                self.measure("SettleComponent", ix, &[]).await,
            ));
            first += chunk.len();
            if first >= ids.len() {
                break;
            }
        }
        self.production_off().await;
        cu
    }
}

/// An active oracle, the dev council, a Token-2022 AMBA with a 25 bp fee and its vault, the heads
/// config and a board of `product` with `items` and every component's sentence sealed.
async fn env_on(
    mut h: Harness,
    product_name: &str,
    params: HeadsParams,
    items: Vec<hx::HeadsItemParams>,
    sentences: Vec<(u8, u8, u8, String)>,
) -> Env {
    h.initialize().await.unwrap();
    h.activate().await;
    let mint_authority = Keypair::new();
    let mint = Keypair::new();
    let space = ExtensionType::try_calculate_account_len::<spl_token_2022::state::Mint>(&[
        ExtensionType::TransferFeeConfig,
    ])
    .unwrap();
    let rent = h.ctx.banks_client.get_rent().await.unwrap();
    let ixs = vec![
        solana_system_interface::instruction::create_account(
            &h.ctx.payer.pubkey(),
            &mint.pubkey(),
            rent.minimum_balance(space),
            space as u64,
            &t22(),
        ),
        spl_token_2022::extension::transfer_fee::instruction::initialize_transfer_fee_config(
            &t22(),
            &mint.pubkey(),
            Some(&mint_authority.pubkey()),
            Some(&mint_authority.pubkey()),
            FEE_BPS,
            u64::MAX,
        )
        .unwrap(),
        spl_token_2022::instruction::initialize_mint2(
            &t22(),
            &mint.pubkey(),
            &mint_authority.pubkey(),
            None,
            9,
        )
        .unwrap(),
    ];
    h.send_many(ixs, &[&mint]).await.unwrap();
    let seats: Vec<Keypair> = (0..5).map(|_| Keypair::new()).collect();
    let admin = h.admin.insecure_clone();
    let payer = h.payer();
    h.send(
        instruction::dev_council_config(
            &program(),
            &admin.pubkey(),
            &payer.pubkey(),
            core::array::from_fn(|i| seats[i].pubkey()),
        ),
        &[&admin],
    )
    .await
    .unwrap();
    let product_id = product(product_name);
    let mut env = Env {
        h,
        mint: mint.pubkey(),
        mint_authority,
        product: product_id,
        board: find_heads_board_address(&program(), &product_id).0,
        now: NOW,
        seats,
        council: instruction::find_dev_council_config_address(&program()).0,
        params,
        buckets: items.iter().map(|i| i.bucket_id).collect(),
        mixes: items
            .iter()
            .map(|i| hx::HeadsListedMix {
                mix_bps: i.mix_bps,
                mix_basis: i.mix_basis,
            })
            .collect(),
        governed: false,
        production: None,
        adverse_terms: false,
        settle_chunk: 8,
    };
    let authority = find_heads_authority_address(&program()).0;
    let vault = env.amba_account(&authority, 0).await;
    assert_eq!(
        vault,
        find_heads_amba_vault_address(&program(), &env.mint, &t22())
    );
    let ix = hx::init_heads_config(
        &program(),
        &payer.pubkey(),
        &admin.pubkey(),
        &env.mint,
        &t22(),
    );
    env.send(ix, &[&admin]).await.unwrap();
    let ix = hx::init_board(
        &program(),
        &payer.pubkey(),
        &admin.pubkey(),
        product_id,
        params,
        items,
    )
    .unwrap();
    env.send(ix, &[&admin]).await.unwrap();
    for (index, item, channel, sentence) in sentences {
        let (hash, object) = env.object(&admin, sentence.as_bytes()).await;
        let ix = hx::set_component(
            &program(),
            &admin.pubkey(),
            &product_id,
            index,
            item,
            HeadsChannel::from_byte(channel).unwrap(),
            hash,
            &object,
        )
        .unwrap();
        env.send(ix, &[&admin]).await.unwrap();
    }
    env
}

/// The RAMX board from the channel molds: four items, twelve components, every sentence sealed.
async fn ramx_env() -> Env {
    ramx_env_with(test_params()).await
}

async fn ramx_env_with(params: HeadsParams) -> Env {
    ramx_env_on(start().await, params).await
}

async fn ramx_env_on(h: Harness, params: HeadsParams) -> Env {
    let mold = Product::Ramx.mold();
    let items = mold
        .items
        .iter()
        .map(|item| hx::HeadsItemParams {
            bucket_id: item.bucket.id32(),
            basket_weight_bps: item.basket_weight_bps,
            mix_bps: item.mix_bps,
            mix_basis: HeadsMixBasis::Assumed,
        })
        .collect();
    let sentences = mold
        .components()
        .map(|c| (c.index, c.item, c.channel.code(), c.sentence.clone()))
        .collect();
    let env = env_on(h, "RAMX", params, items, sentences).await;
    let board = env_board(&env).await;
    for c in mold.components() {
        assert_eq!(
            board.component_sentence_hashes[usize::from(c.index)],
            c.sentence_sha256
        );
    }
    env
}

async fn env_board(env: &Env) -> Box<HeadsBoard> {
    let account = env
        .h
        .ctx
        .banks_client
        .get_account(env.board)
        .await
        .unwrap()
        .unwrap();
    HeadsBoard::unpack(&account.data).unwrap()
}

/// A one-item board ("TESTX", the whole basket, mix `[20, 50, 30]` on `basis`) with three
/// placeholder sentences.
async fn small_env(params: HeadsParams, basis: HeadsMixBasis) -> Env {
    small_env_on(start().await, params, basis).await
}

async fn small_env_on(h: Harness, params: HeadsParams, basis: HeadsMixBasis) -> Env {
    let items = vec![hx::HeadsItemParams {
        bucket_id: product("TESTX-ITEM"),
        basket_weight_bps: 10_000,
        mix_bps: [2_000, 5_000, 3_000],
        mix_basis: basis,
    }];
    let sentences = (0..3u8)
        .map(|ch| {
            (
                ch,
                0,
                ch,
                format!("USD per chip, TESTX item, channel {ch}, as of the observation date"),
            )
        })
        .collect();
    env_on(h, "TESTX", params, items, sentences).await
}

fn bincode_len(tx: &Transaction) -> usize {
    // The wire size of a legacy transaction: signatures, message header, keys, blockhash and
    // instructions, each compact-length prefixed.
    fn compact(n: usize) -> usize {
        if n < 0x80 {
            1
        } else if n < 0x4000 {
            2
        } else {
            3
        }
    }
    let m = &tx.message;
    let mut size = compact(tx.signatures.len()) + 64 * tx.signatures.len() + 3;
    size += compact(m.account_keys.len()) + 32 * m.account_keys.len() + 32;
    size += compact(m.instructions.len());
    for ix in &m.instructions {
        size += 1
            + compact(ix.accounts.len())
            + ix.accounts.len()
            + compact(ix.data.len())
            + ix.data.len();
    }
    size
}

/// The whole life of a RAMX contract on the board, on the fixed-mix reference (the user,
/// 2026-10-29: "SPOT LTA AND CONTRACT NEED TO BE WEIGHTED AGAINST EACHOTHER"): three origins on
/// every channel of every item, the molds' assumed mixes, each item settling on its three
/// channels weighted by its mix.
#[tokio::test]
async fn a_ramx_contract_from_fills_to_settled() {
    let mut env = ramx_env().await;
    let a = env.user(100_000 * AMBA, 0).await;
    let b = env.user(100_000 * AMBA, 0).await;
    let filler = env.user(0, 1_000 * USDC).await;
    let updater = env.user(10_000 * AMBA, 1_000 * USDC).await;
    let challenger = env.user(0, 100 * USDC).await;
    let funder = env.user(0, 2_000 * USDC).await;
    let mold = Product::Ramx.mold();
    let items = mold.items.len();
    let components = 3 * items;

    // Three heads with three independent origins on every component: spot, contract and LTA of
    // every item.
    let mut heads: Vec<Vec<u32>> = Vec::new();
    for component in 0..components {
        let mut ids = Vec::new();
        for _ in 0..3 {
            ids.push(env.active_head(&a, &b, component as u8).await);
        }
        heads.push(ids);
    }

    // Fills, revealed on 2026-10-17; the first one challenged and ruled by the dev council.
    let opening = |c: usize, j: usize| 1_000_000 * (c as u64 + 2) + 10_000 * j as u64;
    let publisher = |c: usize, j: usize| origin(&format!("publisher-{c}-{j}"));
    let mut fills = Vec::new();
    for (c, ids) in heads.iter().enumerate() {
        for (j, &id) in ids.iter().enumerate() {
            let p = env
                .fill(&filler, id, opening(c, j), env.now, publisher(c, j))
                .await;
            fills.push(p);
        }
    }
    let disputed = fills[0];
    // A second fill on a head whose fill is open is refused (competing fills are challenges).
    let seq = env.head(disputed.id).await.claim_seq;
    let reveal = Env::reveal_params(1, env.now, origin("x"), Default::default(), seq);
    let ix = env.commit_ix(
        &challenger.pubkey(),
        disputed.id,
        seq,
        HeadClaimKind::Fill,
        &reveal,
    );
    assert_oracle_error(
        env.send(ix, &[&challenger]).await,
        OracleError::InvalidHeadsState,
    );
    env.challenge(&challenger, disputed, 5 * USDC)
        .await
        .unwrap();
    // One challenge per challenger and claim; a bond below the board's is refused.
    assert_oracle_error(
        env.challenge(&challenger, disputed, 5 * USDC).await,
        OracleError::AlreadyInitialized,
    );
    let other = env.user(0, 10 * USDC).await;
    assert_oracle_error(
        env.challenge(&other, disputed, 0).await,
        OracleError::InvalidHeadsState,
    );
    // A claim challenge opens no case on the head: it never strikes the source.
    assert_eq!(env.head(disputed.id).await.unresolved_cases, 0);
    // Finalizing before the window ends, or with the challenge unruled, is refused.
    let ix = env.finalize_ix(disputed, &filler.pubkey());
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    env.warp(env.now + DAY).await;
    let ix = env.finalize_ix(disputed, &filler.pubkey());
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    env.rule(disputed, &challenger.pubkey(), false).await;
    let usdc_before = env.usdc(&filler.pubkey()).await;
    let mut finalize_cu = Vec::new();
    for &p in &fills {
        finalize_cu.push(env.finalize(p, &filler.pubkey()).await);
    }
    // The disputed fill stood: the filler got the challenger's bond on top of its own.
    let bond = env.params.claim_bond;
    assert_eq!(
        env.usdc(&filler.pubkey()).await,
        usdc_before + fills.len() as u64 * bond + 5 * USDC
    );
    assert_eq!(env.head(disputed.id).await.unresolved_cases, 0);
    env.settle_claim_challenge(disputed, &challenger.pubkey())
        .await
        .unwrap();
    assert!(env
        .h
        .account(
            find_head_challenge_address(
                &program(),
                &env.claim_key(disputed.id, disputed.seq),
                &challenger.pubkey()
            )
            .0
        )
        .await
        .is_none());
    let board = env.board().await;
    assert_eq!(usize::from(board.live_count), 3 * components);
    assert!(board.component_live_origins[..components]
        .iter()
        .all(|&o| o == 3));
    // The fill prizes accrue in October's epoch; fund it.
    let october = time::month_id(env.now);
    let ix = hx::fund_board(
        &program(),
        &funder.pubkey(),
        &funder.pubkey(),
        &env.product,
        october,
        100 * USDC,
    );
    env.send(ix, &[&funder]).await.unwrap();

    // Anyone updates a Live head: no seats, votes or fetcher stakes.
    // RAMX Q4 2026: a quarterly period expiring 2027-01-01 (selector 28), listed at once, since
    // its quarter is the current one.
    let expiry = JAN_01_2027;
    let planned = env.now;
    env.create_quarter_period(planned, expiry).await.unwrap();
    let period = find_period_address(&program(), &env.product, expiry).0;
    // Only the oracle authority lists.
    let terms = env.terms(expiry).await;
    let target = HeadsSettlementTarget::FixedMixReference;
    let mut wrong = env.list_ix(expiry, target, terms, 100 * USDC);
    let stranger = Keypair::new();
    wrong.accounts[2].pubkey = stranger.pubkey();
    assert_oracle_error(
        env.send(wrong, &[&stranger]).await,
        OracleError::Unauthorized,
    );
    // The molds' target on their assumed mixes: every channel has three origins, so it lists.
    let base = 100 * USDC;
    let ix = env.list_ix(expiry, target, terms, base);
    let oracle = env.h.oracle_authority.insecure_clone();
    env.measure("ListContract", ix, &[&oracle]).await;
    let p = env.h.period(env.product, expiry).await;
    assert_eq!(
        (p.status, p.effective_listing_ts),
        (PeriodStatus::Live, planned)
    );
    let contract = env.contract(expiry).await;
    assert_eq!(contract.settlement_target, target);
    let freeze = time::settlement_window_start(expiry, env.params.settlement_business_days);
    assert_eq!((contract.listing_ts, contract.freeze_ts), (planned, freeze));
    assert_eq!(
        contract.terms_hash,
        molds::listing_terms_sha256(Product::Ramx, expiry as i64)
    );
    for c in 0..components {
        assert_eq!(contract.component_listed_origins[c], 3);
    }
    // The quarter's mixes (Q4 2026's are the board's defaults).
    let quarter_mixes = molds::listing_mixes(Product::Ramx, expiry as i64);
    for (i, item) in mold.items.iter().enumerate() {
        assert_eq!(contract.item_mix_bps[i * 3..i * 3 + 3], quarter_mixes[i].0);
        assert_eq!(contract.item_mix_bps[i * 3..i * 3 + 3], item.mix_bps);
        assert_eq!(contract.item_mix_basis[i], HeadsMixBasis::Assumed as u8);
    }
    assert_eq!(usize::from(contract.listed_count), 3 * components);
    // The period is Live: it cannot list again.
    let ix = env.list_ix(expiry, target, terms, base);
    assert_oracle_error(
        env.send(ix, &[&oracle]).await,
        OracleError::InvalidPeriodTransition,
    );
    let _ = period;

    // An update inside the window, before the freeze: it moves the head's value, not the
    // contract's opening.
    env.warp(NOV_01_2026 + 9 * DAY).await;
    let mut updates = Vec::new();
    for (c, ids) in heads.iter().enumerate() {
        for (j, &id) in ids.iter().enumerate() {
            let value = opening(c, j) + 1_000;
            updates.push(
                env.update(&updater, id, value, env.now, publisher(c, j))
                    .await,
            );
        }
    }
    // The last one is challenged and stands.
    let disputed_update = *updates.last().unwrap();
    env.challenge(&challenger, disputed_update, 2 * USDC)
        .await
        .unwrap();
    env.warp(env.now + DAY).await;
    env.rule(disputed_update, &challenger.pubkey(), false).await;
    for &p in &updates {
        finalize_cu.push(env.finalize(p, &updater.pubkey()).await);
    }
    env.settle_claim_challenge(disputed_update, &challenger.pubkey())
        .await
        .unwrap();
    let november = time::month_id(env.now);
    let ix = hx::fund_board(
        &program(),
        &funder.pubkey(),
        &funder.pubkey(),
        &env.product,
        november,
        1_000 * USDC,
    );
    env.send(ix, &[&funder]).await.unwrap();

    // The settlement window's prints: each source moves by its own amount, in percent.
    let moves = |c: usize, j: usize| (c as i64 * 7 + j as i64 * 5) % 23 - 11;
    let level = |c: usize, j: usize| {
        (opening(c, j) as i64 + opening(c, j) as i64 * moves(c, j) / 100) as u64
    };
    env.warp(freeze + DAY).await;
    let mut late = Vec::new();
    for (c, ids) in heads.iter().enumerate() {
        for (j, &id) in ids.iter().enumerate() {
            late.push(
                env.update(&updater, id, level(c, j), env.now, publisher(c, j))
                    .await,
            );
        }
    }
    env.warp(env.now + DAY).await;
    for &p in &late {
        env.finalize(p, &updater.pubkey()).await;
    }

    // Nothing settles before expiry plus the challenge window.
    let payer = env.h.ctx.payer.pubkey();
    let ix =
        hx::settle_component(&program(), &payer, &env.product, expiry, 1, 0, &heads[1]).unwrap();
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    env.warp(expiry + env.params.challenge_window_secs).await;
    // Finalizing the contract before its components settle is refused.
    let ix = hx::finalize_contract(&program(), &env.product, expiry);
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    // Levels go in source order from the next one to record.
    let ix = hx::settle_component(
        &program(),
        &payer,
        &env.product,
        expiry,
        1,
        1,
        &heads[1][1..],
    )
    .unwrap();
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    let mut settle_cu = Vec::new();
    for component in 0..components as u8 {
        settle_cu.extend(env.settle(expiry, component, &[]).await);
    }
    // Each source weighs its head's time-weighted delegation over the window, uncapped.
    let (header, sources) = env.weights(expiry, 1).await.unwrap();
    assert_eq!(header.source_count, 3);
    assert!(sources.iter().all(|s| s.weight > 0 && s.integral > 0));

    // The expected value, from the shared crate: every component's weighted median (equal
    // capped weights), each item's three channels weighted by its mix, the basket over items.
    let contract = env.contract(expiry).await;
    let mut expected_items = Vec::new();
    for (item, mold_item) in mold.items.iter().enumerate() {
        let mut readings = [channels::ChannelReading::default(); 3];
        for (ch, reading) in readings.iter_mut().enumerate() {
            let c = item * 3 + ch;
            let mut pairs: Vec<(i64, u64)> = (0..3)
                .map(|j| (settle::change_ppm(opening(c, j), level(c, j)).unwrap(), 1))
                .collect();
            let change = median::weighted_median(&mut pairs).unwrap();
            assert_eq!(
                contract.component_flags[c] & (COMPONENT_SETTLED | COMPONENT_LIVE),
                COMPONENT_SETTLED | COMPONENT_LIVE,
                "component {c}"
            );
            assert_eq!(contract.component_change_ppm[c], change, "component {c}");
            assert_eq!(contract.component_eligible[c], 3);
            *reading = channels::ChannelReading {
                change_ppm: change,
                degraded: false,
            };
        }
        let reference =
            channels::reference(&mold_item.mix_bps, channels::MixBasis::Assumed, &readings);
        expected_items.push(channels::ItemSettlement {
            basket_weight_bps: mold_item.basket_weight_bps,
            contract_change_ppm: readings[1].change_ppm,
            reference,
        });
        assert_eq!(contract.item_change_ppm[item], 0, "not yet");
    }
    let index = channels::stage2_change_ppm(&expected_items).unwrap();
    // Not the Contract channel alone.
    assert_ne!(index, channels::stage1_change_ppm(&expected_items).unwrap());
    let value = settle::settlement_value(base, index).unwrap();
    let ix = hx::finalize_contract(&program(), &env.product, expiry);
    let finalize_contract_cu = env.measure("FinalizeContract", ix, &[]).await;
    let contract = env.contract(expiry).await;
    assert_eq!(contract.status, HeadsContractStatus::Settled);
    assert_eq!((contract.index_change_ppm, contract.value), (index, value));
    assert_eq!(contract.digest, heads_settlement_digest(&contract));
    for (item, expected) in expected_items.iter().enumerate() {
        let blend = expected.reference.change_ppm().unwrap();
        // Each item settles on its blend, recorded and flagged as an assumed scenario, with no
        // channel held at a last valid value.
        assert_eq!(contract.item_change_ppm[item], blend);
        assert_eq!(contract.item_reference_ppm[item], blend);
        assert_eq!(
            contract.item_reference_flags[item],
            REFERENCE_COMPUTED | REFERENCE_AVAILABLE | REFERENCE_ASSUMED
        );
        assert_eq!(contract.item_reference_reason[item], 0);
        assert_eq!(contract.item_degraded_channels[item], 0);
    }
    let settled = env.h.period(env.product, expiry).await;
    assert_eq!(settled.status, PeriodStatus::Settled);
    assert_eq!(settled.settlement_value, value);
    assert_eq!(settled.settlement_digest, contract.digest);
    assert_eq!(
        env.board().await.contract_status[usize::from(contract.ring_slot)],
        3
    );
    eprintln!("index {index} ppm, value {value}");

    // Prizes: October's fills share its 20 USDC pro rata; November's updates are paid in full.
    let fill_prize = rewards::fill_prize(&env.params.to_params(), 2_500 * AMBA);
    let epoch: Box<HeadsEpoch> = env
        .read(find_heads_epoch_address(&program(), &env.board, october).0)
        .await;
    assert_eq!(epoch.prize_points, fill_prize * fills.len() as u64);
    assert_eq!(epoch.prize_pool, 20 * USDC);
    let before = env.usdc(&filler.pubkey()).await;
    let first = fills[1];
    let ix = hx::claim_prize(
        &program(),
        &filler.pubkey(),
        &env.product,
        first.id,
        first.seq,
        october,
    );
    env.send(ix.clone(), &[&filler]).await.unwrap();
    let paid = rewards::prize_payout(fill_prize, epoch.prize_points, epoch.prize_pool);
    assert_eq!(env.usdc(&filler.pubkey()).await, before + paid);
    assert!(paid < fill_prize);
    assert_oracle_error(
        env.send(ix, &[&filler]).await,
        OracleError::InvalidHeadsState,
    );
    // Only the claim's fetcher claims it.
    let second = fills[2];
    let ix = hx::claim_prize(
        &program(),
        &challenger.pubkey(),
        &env.product,
        second.id,
        second.seq,
        october,
    );
    assert_oracle_error(
        env.send(ix, &[&challenger]).await,
        OracleError::InvalidHeadsState,
    );
    let update_prize = rewards::update_prize(&env.params.to_params(), 2_500 * AMBA);
    let p = updates[0];
    let before = env.usdc(&updater.pubkey()).await;
    let ix = hx::claim_prize(
        &program(),
        &updater.pubkey(),
        &env.product,
        p.id,
        p.seq,
        november,
    );
    env.send(ix, &[&updater]).await.unwrap();
    assert_eq!(env.usdc(&updater.pubkey()).await, before + update_prize);

    for (label, units) in settle_cu.iter().filter(|(_, u)| *u > 0) {
        eprintln!("CU {label}: {units}");
    }
    eprintln!(
        "CU FinalizeClaim max {} / FinalizeContract {}",
        finalize_cu.iter().max().unwrap(),
        finalize_contract_cu
    );
    assert!(settle_cu.iter().all(|(_, u)| *u < 1_400_000));
}

/// Quarterly contracts (the user, 2026-10-29: "after the november 1st expiry market i want it to
/// work in Quarters. so Q1 Q2 etc."; "LTA %ages of weight need to adjust dynamically depending on
/// the Q."): a period expiring at a quarter boundary, created apart from v1's monthly ladder,
/// lists with its quarter's mixes from the molds, which its sealed listing terms document
/// commits to; the board's own mixes are only the default.
#[tokio::test]
async fn a_quarter_period_lists_with_its_quarters_mix() {
    let mut params = test_params();
    params.min_live_sources = 1;
    let mut env = ramx_env_with(params).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let mut fills = Vec::new();
    for item in 0..4u8 {
        let id = env.active_head(&a, &b, item * 3 + 1).await;
        fills.push(
            env.fill(&f, id, 1_000_000, env.now, origin(&format!("q{item}")))
                .await,
        );
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    // RAMX Q1 2027 expires 2027-04-01; its listing is planned for the quarter's start.
    let (expiry, start) = (APR_01_2027, JAN_01_2027);
    assert!(hx::create_quarter_period(
        &program(),
        &env.h.ctx.payer.pubkey(),
        &env.h.oracle_authority.pubkey(),
        &env.product,
        start,
        expiry + DAY,
    )
    .is_err());
    // A listing planned after both the quarter's start and now is refused, as is another signer.
    assert_oracle_error(
        env.create_quarter_period(start + DAY, expiry).await,
        OracleError::InvalidPeriodSchedule,
    );
    let stranger = Keypair::new();
    let ix = hx::create_quarter_period(
        &program(),
        &env.h.ctx.payer.pubkey(),
        &stranger.pubkey(),
        &env.product,
        start,
        expiry,
    )
    .unwrap();
    assert_oracle_error(env.send(ix, &[&stranger]).await, OracleError::Unauthorized);
    env.create_quarter_period(start, expiry).await.unwrap();
    let period = env.h.period(env.product, expiry).await;
    assert_eq!(
        (period.status, period.planned_listing_ts, period.expiry_ts),
        (PeriodStatus::Forming, start, expiry)
    );
    // Once is enough: the period exists.
    assert!(env.create_quarter_period(start, expiry).await.is_err());
    env.warp(start).await;
    let terms = env.terms(expiry).await;
    let oracle = env.h.oracle_authority.insecure_clone();
    // The board's default mixes (Q4 2026's) are not what the Q1 2027 document commits to.
    let ix = hx::list_contract(
        &program(),
        &env.h.ctx.payer.pubkey(),
        &oracle.pubkey(),
        &env.product,
        expiry,
        10 * USDC,
        HeadsSettlementTarget::ContractChannel,
        &env.mixes.clone(),
        terms.0,
        &terms.1,
    )
    .unwrap();
    assert_oracle_error(
        env.send(ix, &[&oracle]).await,
        OracleError::InvalidHeadsEvidence,
    );
    // The quarter's own mixes list, and the contract freezes them.
    let ix = env.list_ix(
        expiry,
        HeadsSettlementTarget::ContractChannel,
        terms,
        10 * USDC,
    );
    env.send(ix, &[&oracle]).await.unwrap();
    let contract = env.contract(expiry).await;
    let board = env.board().await;
    let quarter = molds::listing_mixes(Product::Ramx, expiry as i64);
    for (i, (mix, _)) in quarter.iter().enumerate() {
        assert_eq!(contract.item_mix_bps[i * 3..i * 3 + 3], *mix);
        assert_ne!(
            contract.item_mix_bps[i * 3..i * 3 + 3],
            board.item_mix_bps[i * 3..i * 3 + 3]
        );
    }
    assert_eq!(
        contract.terms_hash,
        molds::listing_terms_sha256(Product::Ramx, expiry as i64)
    );
    let p = env.h.period(env.product, expiry).await;
    assert_eq!(
        (p.status, p.effective_listing_ts),
        (PeriodStatus::Live, start)
    );
}

/// A self-challenge never pays, and a falling update costs only its bond: anyone may update a
/// Live head (no seats, votes or fetcher stake: the user, 2026-10-29, "agreed just have
/// challenges deal w some of that stuff"); a claimant who challenges its own update and wins gets
/// its own claim bond back, and nothing else happens: no strike, no slash of the delegators.
#[tokio::test]
async fn a_self_challenge_never_pays() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(10_000 * AMBA, 0).await;
    let b = env.user(10_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let usdc = env.usdc(&f.pubkey()).await;
    let burn = env.config().await.pending_amba_burn;

    // A stranger with no stake at all files an update.
    let p = env.update(&f, id, 2_000_000, env.now, origin("one")).await;
    env.challenge(&f, p, 3 * USDC).await.unwrap();
    env.warp(env.now + DAY).await;
    env.rule(p, &f.pubkey(), true).await;
    env.finalize(p, &f.pubkey()).await;
    env.settle_claim_challenge(p, &f.pubkey()).await.unwrap();
    // USDC: the claim bond went from its fetcher to its challenger, the same wallet.
    assert_eq!(env.usdc(&f.pubkey()).await, usdc);
    assert_eq!(env.claim(p).await.status, HeadClaimStatus::Fell);
    // No strike, no AMBA burned, the delegators untouched, the head still Live.
    let head = env.head(id).await;
    assert_eq!((head.strike_count, head.status), (0, HeadStatus::Live));
    assert_eq!(env.config().await.pending_amba_burn, burn);
    env.sync(id, &b.pubkey()).await;
    let d: Box<Delegation> = env
        .read(find_head_delegation_address(&program(), &env.head_key(id), &b.pubkey()).0)
        .await;
    assert_eq!(d.active, 1_500 * AMBA);
    // The same wallet files the next update.
    let p = env.update(&f, id, 1_010_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    assert_eq!(env.head(id).await.value, 1_010_000);
}

/// A fill attempt that falls costs only its bond: no strike, no slash, the head stays Active
/// for the next fill. With several winners the claim bond splits pro rata, the last winner
/// after every other.
#[tokio::test]
async fn a_falling_fill_costs_only_its_bond() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(10_000 * AMBA, 0).await;
    let b = env.user(10_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(0, 100 * USDC).await;
    let y = env.user(0, 100 * USDC).await;
    let z = env.user(0, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let total = env.board().await.total_delegation;
    let f_usdc = env.usdc(&f.pubkey()).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.challenge(&x, p, 2 * USDC).await.unwrap();
    env.challenge(&y, p, 3 * USDC).await.unwrap();
    // A late challenge extends the window by the extension.
    let window_end = env.claim(p).await.window_end_ts;
    env.warp(window_end - 60).await;
    env.challenge(&z, p, 5 * USDC).await.unwrap();
    assert_eq!(
        env.claim(p).await.window_end_ts,
        env.now + env.params.challenge_extension_secs
    );
    env.warp(window_end).await;
    // Two of three seats are not a ruling; a repeated seat is not three.
    let epoch = env.epoch().await;
    let keys = [
        env.seats[0].pubkey(),
        env.seats[0].pubkey(),
        env.seats[1].pubkey(),
    ];
    let ix = hx::rule_claim_challenge(
        &program(),
        &env.council,
        &keys,
        &env.product,
        id,
        p.seq,
        &x.pubkey(),
        true,
        epoch,
    );
    let (s0, s1) = (env.seats[0].insecure_clone(), env.seats[1].insecure_clone());
    assert_oracle_error(
        env.send(ix, &[&s0, &s1]).await,
        OracleError::HeadsCouncilRequired,
    );
    // A stranger is not a seat; a stale epoch is not current.
    let stranger = Keypair::new();
    let keys = [
        env.seats[0].pubkey(),
        env.seats[1].pubkey(),
        stranger.pubkey(),
    ];
    let ix = hx::rule_claim_challenge(
        &program(),
        &env.council,
        &keys,
        &env.product,
        id,
        p.seq,
        &x.pubkey(),
        true,
        epoch,
    );
    assert_oracle_error(
        env.send(ix, &[&s0, &s1, &stranger]).await,
        OracleError::HeadsCouncilRequired,
    );
    let (keys, signers) = env.seats3([0, 1, 2]);
    let refs: Vec<&Keypair> = signers.iter().collect();
    let ix = hx::rule_claim_challenge(
        &program(),
        &env.council,
        &keys,
        &env.product,
        id,
        p.seq,
        &x.pubkey(),
        true,
        epoch + 1,
    );
    assert_oracle_error(env.send(ix, &refs).await, OracleError::HeadsCouncilRequired);
    env.rule(p, &x.pubkey(), true).await;
    // A ruling is final.
    let ix = hx::rule_claim_challenge(
        &program(),
        &env.council,
        &keys,
        &env.product,
        id,
        p.seq,
        &x.pubkey(),
        false,
        epoch,
    );
    assert_oracle_error(env.send(ix, &refs).await, OracleError::InvalidHeadsState);
    env.rule(p, &y.pubkey(), false).await;
    let ix = env.finalize_ix(p, &f.pubkey());
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    env.warp(env.now + env.params.challenge_extension_secs)
        .await;
    env.rule(p, &z.pubkey(), true).await;
    // Paying a challenge before its claim resolves is refused.
    assert_oracle_error(
        env.settle_claim_challenge(p, &x.pubkey()).await,
        OracleError::InvalidHeadsState,
    );
    let burn = env.board().await.pending_usdc_burn;
    env.finalize(p, &f.pubkey()).await;
    let head = env.head(id).await;
    assert_eq!(
        (head.status, head.strike_count, head.open_fill),
        (HeadStatus::Active, 0, false)
    );
    assert_eq!(env.board().await.total_delegation, total);
    assert_eq!(env.usdc(&f.pubkey()).await, f_usdc - env.params.claim_bond);
    // The loser's bond burns.
    assert_eq!(env.board().await.pending_usdc_burn, burn + 3 * USDC);
    // X and Z share the claim bond 2:5; Z, the last winner, is paid after X.
    let (x0, z0) = (env.usdc(&x.pubkey()).await, env.usdc(&z.pubkey()).await);
    assert_oracle_error(
        env.settle_claim_challenge(p, &z.pubkey()).await,
        OracleError::InvalidHeadsState,
    );
    env.settle_claim_challenge(p, &y.pubkey()).await.unwrap();
    env.settle_claim_challenge(p, &x.pubkey()).await.unwrap();
    env.settle_claim_challenge(p, &z.pubkey()).await.unwrap();
    let shares =
        ameba_oracle_heads::outcome::pro_rata(env.params.claim_bond, &[2 * USDC, 0, 5 * USDC]);
    assert_eq!(env.usdc(&x.pubkey()).await, x0 + 2 * USDC + shares[0]);
    assert_eq!(env.usdc(&z.pubkey()).await, z0 + 5 * USDC + shares[2]);
    let claim = env.claim(p).await;
    assert_eq!(
        (claim.settled_count, claim.shares_paid),
        (3, env.params.claim_bond)
    );
    // Paid challenges are closed: a second settlement finds nothing.
    assert!(env.settle_claim_challenge(p, &x.pubkey()).await.is_err());
    // The head takes its next fill.
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    assert_eq!(env.head(id).await.status, HeadStatus::Live);
    // A finalized claim does not finalize again.
    let ix = env.finalize_ix(p, &f.pubkey());
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
}

/// A falling source case pays the proposer's delegation to the winners pro rata (the last winner
/// takes the remainder), burns the loser's bond and strikes the source; the other delegators pay
/// their slash on their next sync.
#[tokio::test]
async fn a_source_challenge_pays_the_proposers_delegation_to_the_winners() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(10_000 * AMBA, 0).await;
    let b = env.user(10_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let y = env.user(AMBA, 100 * USDC).await;
    let z = env.user(0, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    // An Open head cannot be source challenged.
    let open = env.post(&a, 1, 1_000 * AMBA).await;
    let source = |bond| hx::ChallengeSourceParams {
        bond,
        reason: HeadChallengeReason::OffChannel,
        evidence_kind: 0,
        evidence_commitment: [0; 32],
        reason_hash: heads_reason_hash(b"the distributor publishes no such table"),
    };
    let ix = hx::challenge_source(
        &program(),
        &x.pubkey(),
        &x.pubkey(),
        &env.product,
        open,
        source(USDC),
        None,
    )
    .unwrap();
    assert_oracle_error(env.send(ix, &[&x]).await, OracleError::InvalidHeadsState);
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    for (who, bond) in [(&x, 2 * USDC), (&z, 5 * USDC), (&y, 3 * USDC)] {
        let ix = hx::challenge_source(
            &program(),
            &who.pubkey(),
            &who.pubkey(),
            &env.product,
            id,
            source(bond),
            None,
        )
        .unwrap();
        env.measure("ChallengeSource", ix, &[who]).await;
    }
    assert_eq!(env.head(id).await.unresolved_cases, 1);
    let finalize = hx::finalize_source_challenges(&program(), &env.product, id, &a.pubkey());
    assert_oracle_error(
        env.send(finalize.clone(), &[]).await,
        OracleError::InvalidHeadsState,
    );
    env.warp(env.now + DAY).await;
    // After the window no challenge joins the case.
    let late = env.user(0, 10 * USDC).await;
    let ix = hx::challenge_source(
        &program(),
        &late.pubkey(),
        &late.pubkey(),
        &env.product,
        id,
        source(USDC),
        None,
    )
    .unwrap();
    assert_oracle_error(env.send(ix, &[&late]).await, OracleError::InvalidHeadsState);
    env.rule_source(id, &x.pubkey(), true).await;
    env.rule_source(id, &y.pubkey(), true).await;
    assert_oracle_error(
        env.send(finalize.clone(), &[]).await,
        OracleError::InvalidHeadsState,
    );
    env.rule_source(id, &z.pubkey(), false).await;
    let proposer_amount = env
        .read::<Delegation>(
            find_head_delegation_address(&program(), &env.head_key(id), &a.pubkey()).0,
        )
        .await
        .active;
    let burn = env.board().await.pending_usdc_burn;
    let accounted = env.config().await.amba_accounted;
    env.measure("FinalizeSourceChallenges", finalize, &[]).await;
    let head = env.head(id).await;
    assert_eq!(
        (head.source_case_status, head.strike_count),
        (SOURCE_CASE_FELL, 1)
    );
    assert_eq!(head.source_case_proposer_slashed, proposer_amount);
    assert_eq!(head.delegation, 1_500 * AMBA);
    assert_eq!(env.board().await.pending_usdc_burn, burn + 5 * USDC);
    // Nothing burned of the proposer's: it is owed to the winners.
    assert_eq!(env.config().await.amba_accounted, accounted);
    let d: Box<Delegation> = env
        .read(find_head_delegation_address(&program(), &env.head_key(id), &a.pubkey()).0)
        .await;
    assert_eq!((d.active, d.proposer_slashed), (0, proposer_amount));
    let parts = ameba_oracle_heads::outcome::pro_rata(proposer_amount, &[2 * USDC, 0, 3 * USDC]);
    let (x0, y0) = (
        env.stake_of(&x.pubkey()).await.free,
        env.stake_of(&y.pubkey()).await.free,
    );
    let (xu, zu) = (env.usdc(&x.pubkey()).await, env.usdc(&z.pubkey()).await);
    // Y, the last winner by index, is paid after X.
    let settle = |who: &Keypair| {
        hx::settle_source_challenge(&program(), &env.product, id, &who.pubkey(), &who.pubkey())
    };
    let (sx, sy, sz) = (settle(&x), settle(&y), settle(&z));
    assert_oracle_error(
        env.send(sy.clone(), &[]).await,
        OracleError::InvalidHeadsState,
    );
    env.measure("SettleChallenge (source)", sx, &[]).await;
    env.send(sz, &[]).await.unwrap();
    env.send(sy, &[]).await.unwrap();
    assert_eq!(env.stake_of(&x.pubkey()).await.free, x0 + parts[0]);
    assert_eq!(env.stake_of(&y.pubkey()).await.free, y0 + parts[2]);
    assert_eq!(env.usdc(&x.pubkey()).await, xu + 2 * USDC);
    assert_eq!(env.usdc(&z.pubkey()).await, zu);
    assert_eq!(env.head(id).await.source_case_amba_paid, proposer_amount);
    assert_eq!(env.config().await.amba_accounted, accounted);
    // B pays the slash rate on its next sync.
    env.sync(id, &b.pubkey()).await;
    let d: Box<Delegation> = env
        .read(find_head_delegation_address(&program(), &env.head_key(id), &b.pubkey()).0)
        .await;
    assert_eq!(
        d.active,
        1_500 * AMBA - liability::slash(1_500 * AMBA, env.params.slash_rate_bps)
    );
}

/// At `max_strikes` a source retires: it leaves the live list, its delegations cool, and it
/// takes no more claims. A commit nobody reveals is refunded after its deadline.
#[tokio::test]
async fn max_strikes_retire_a_source() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(10_000 * AMBA, 0).await;
    let b = env.user(10_000 * AMBA, 0).await;
    let f = env.user(1_000 * AMBA, 100 * USDC).await;
    // A stake to receive the proposer's slash.
    let x = env.user(AMBA, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    // An unrevealed commit: the refund waits for the deadline, then the bond comes back.
    let seq = env.head(id).await.claim_seq;
    let reveal = Env::reveal_params(5, env.now, origin("one"), Default::default(), seq);
    let ix = env.commit_ix(&f.pubkey(), id, seq, HeadClaimKind::Update, &reveal);
    env.send(ix, &[&f]).await.unwrap();
    let usdc = env.usdc(&f.pubkey()).await;
    let refund = hx::refund_expired_commit(&program(), &env.product, id, seq, &f.pubkey());
    assert_oracle_error(
        env.send(refund.clone(), &[]).await,
        OracleError::InvalidHeadsState,
    );
    // Revealing in the commit's own slot is refused.
    let ix = hx::reveal_claim(
        &program(),
        &f.pubkey(),
        &env.product,
        id,
        seq,
        reveal.clone(),
        None,
    )
    .unwrap();
    assert_oracle_error(env.send(ix, &[&f]).await, OracleError::InvalidHeadsState);
    let deadline = env.claim(Pending { id, seq }).await.reveal_deadline_slot;
    env.h.ctx.warp_to_slot(deadline + 1).unwrap();
    set_unix_timestamp(&env.h.ctx, env.now).await;
    let ix =
        hx::reveal_claim(&program(), &f.pubkey(), &env.product, id, seq, reveal, None).unwrap();
    assert_oracle_error(env.send(ix, &[&f]).await, OracleError::InvalidHeadsState);
    env.send(refund.clone(), &[]).await.unwrap();
    assert_eq!(env.usdc(&f.pubkey()).await, usdc + env.params.claim_bond);
    assert_eq!(env.head(id).await.open_claims, 0);
    assert_oracle_error(env.send(refund, &[]).await, OracleError::InvalidHeadsState);
    // A falling update strikes nothing.
    let p = env.update(&f, id, 2_000_000, env.now, origin("one")).await;
    env.challenge(&x, p, USDC).await.unwrap();
    env.warp(env.now + DAY).await;
    env.rule(p, &x.pubkey(), true).await;
    env.finalize(p, &f.pubkey()).await;
    env.settle_claim_challenge(p, &x.pubkey()).await.unwrap();
    assert_eq!(env.head(id).await.strike_count, 0);
    // Strikes come from lost source challenges only: two retire it.
    for strike in 1..=2u8 {
        let ix = hx::challenge_source(
            &program(),
            &x.pubkey(),
            &x.pubkey(),
            &env.product,
            id,
            hx::ChallengeSourceParams {
                bond: USDC,
                reason: HeadChallengeReason::ValueOrOther,
                evidence_kind: 0,
                evidence_commitment: [0; 32],
                reason_hash: heads_reason_hash(b"fabricated"),
            },
            None,
        )
        .unwrap();
        env.send(ix, &[&x]).await.unwrap();
        env.warp(env.now + DAY).await;
        env.rule_source(id, &x.pubkey(), true).await;
        env.send(
            hx::finalize_source_challenges(&program(), &env.product, id, &a.pubkey()),
            &[],
        )
        .await
        .unwrap();
        env.send(
            hx::settle_source_challenge(&program(), &env.product, id, &x.pubkey(), &x.pubkey()),
            &[],
        )
        .await
        .unwrap();
        assert_eq!(env.head(id).await.strike_count, strike);
    }
    let head = env.head(id).await;
    assert_eq!(head.status, HeadStatus::Retired);
    let board = env.board().await;
    assert_eq!(
        (
            board.live_count,
            board.component_live_origins[1],
            board.total_delegation
        ),
        (0, 0, 0)
    );
    // Its delegations cool on their next sync; it takes no claims.
    env.sync(id, &b.pubkey()).await;
    let d: Box<Delegation> = env
        .read(find_head_delegation_address(&program(), &env.head_key(id), &b.pubkey()).0)
        .await;
    assert_eq!(d.active, 0);
    assert!(d.held > 0);
    let seq = env.head(id).await.claim_seq;
    let reveal = Env::reveal_params(5, env.now, origin("one"), Default::default(), seq);
    let ix = env.commit_ix(&f.pubkey(), id, seq, HeadClaimKind::Update, &reveal);
    assert_oracle_error(env.send(ix, &[&f]).await, OracleError::InvalidHeadsState);
}

/// Three live sources with one origin are one origin: the Contract component cannot list.
#[tokio::test]
async fn three_sources_one_origin_cannot_list() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let mut fills = Vec::new();
    for _ in 0..3 {
        let id = env.active_head(&a, &b, 1).await;
        fills.push(
            env.fill(&f, id, 1_000_000, env.now, origin("the same report"))
                .await,
        );
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let board = env.board().await;
    assert_eq!(
        (
            board.component_live_sources[1],
            board.component_live_origins[1]
        ),
        (3, 1)
    );
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    let terms = env.terms(expiry).await;
    let ix = env.list_ix(
        expiry,
        HeadsSettlementTarget::ContractChannel,
        terms,
        1_000_000,
    );
    let oracle = env.h.oracle_authority.insecure_clone();
    assert_oracle_error(
        env.send(ix, &[&oracle]).await,
        OracleError::InvalidHeadsState,
    );
}

/// The fixed mix lists on an assumed or a matched mix, never on a mix basis of none; and only with
/// `min_live_sources` independent origins on every channel the mix weights, where the Contract
/// channel alone needs them only on Contract.
#[tokio::test]
async fn a_fixed_mix_lists_on_assumed_or_matched_mixes_with_every_channel_populated() {
    for (basis, populated, fixed_lists) in [
        (HeadsMixBasis::Assumed, [0u8, 1, 2].as_slice(), true),
        (HeadsMixBasis::Matched, [0u8, 1, 2].as_slice(), true),
        (HeadsMixBasis::None, [0u8, 1, 2].as_slice(), false),
        // Spot (20% of the mix) has no sources.
        (HeadsMixBasis::Assumed, [1u8, 2].as_slice(), false),
    ] {
        let mut env = small_env(test_params(), basis).await;
        let a = env.user(40_000 * AMBA, 0).await;
        let b = env.user(40_000 * AMBA, 0).await;
        let f = env.user(0, 100 * USDC).await;
        let mut fills = Vec::new();
        for &component in populated {
            for j in 0..3 {
                let id = env.active_head(&a, &b, component).await;
                fills.push(
                    env.fill(
                        &f,
                        id,
                        1_000_000,
                        env.now,
                        origin(&format!("{component}-{j}")),
                    )
                    .await,
                );
            }
        }
        env.warp(env.now + DAY).await;
        for p in fills {
            env.finalize(p, &f.pubkey()).await;
        }
        let expiry = DEC_01_2026;
        env.create_period(env.now + DAY, expiry).await;
        env.warp(env.now + DAY).await;
        let terms = env.terms(expiry).await;
        let ix = env.list_ix(
            expiry,
            HeadsSettlementTarget::FixedMixReference,
            terms,
            1_000_000,
        );
        let oracle = env.h.oracle_authority.insecure_clone();
        let result = env.send(ix, &[&oracle]).await;
        if fixed_lists {
            result.unwrap();
            let contract = env.contract(expiry).await;
            assert_eq!(
                contract.settlement_target,
                HeadsSettlementTarget::FixedMixReference
            );
            assert_eq!(contract.component_listed_origins[..3], [3, 3, 3]);
            continue;
        }
        assert_oracle_error(result, OracleError::InvalidHeadsState);
        // The Contract channel alone needs origins on Contract only, on any basis.
        let ix = env.list_ix(
            expiry,
            HeadsSettlementTarget::ContractChannel,
            terms,
            1_000_000,
        );
        env.send(ix, &[&oracle]).await.unwrap();
        let contract = env.contract(expiry).await;
        assert_eq!(
            contract.settlement_target,
            HeadsSettlementTarget::ContractChannel
        );
    }
}

/// The brief's rule 6, as the user decided it (2026-10-29): on the fixed mix, a channel that
/// degrades at settlement keeps its fixed share at its last valid value. LTA loses a source just
/// after the freeze, so it settles at its value as of the freeze; its weight is not
/// redistributed onto spot and contract, and its survivors' later prints do not count.
#[tokio::test]
async fn a_channel_degrading_at_settlement_keeps_its_share_at_its_last_valid_value() {
    let mut params = test_params();
    params.max_strikes = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(40_000 * AMBA, 0).await;
    let b = env.user(40_000 * AMBA, 0).await;
    let f = env.user(2_000 * AMBA, 100 * USDC).await;
    let x = env.user(0, 100 * USDC).await;
    let mut ids = [[0u32; 3]; 3];
    let mut fills = Vec::new();
    for channel in 0..3 {
        for j in 0..3 {
            let id = env.active_head(&a, &b, channel as u8).await;
            ids[channel][j] = id;
            fills.push(
                env.fill(
                    &f,
                    id,
                    1_000_000,
                    env.now,
                    origin(&format!("{channel}-{j}")),
                )
                .await,
            );
        }
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    let terms = env.terms(expiry).await;
    let base = 50 * USDC;
    let ix = env.list_ix(
        expiry,
        HeadsSettlementTarget::FixedMixReference,
        terms,
        base,
    );
    let oracle = env.h.oracle_authority.insecure_clone();
    env.send(ix, &[&oracle]).await.unwrap();
    let freeze = env.contract(expiry).await.freeze_ts;
    // Prints in the freeze's own level window: spot +1/+2/+3%, contract +4/+5/+6%, LTA
    // +2/+4/+8%. Spot and contract print nothing later, so these are their levels at settlement.
    // At the freeze level's window's start: every freeze-window sample takes them.
    let window = env.freeze_window(expiry).await;
    env.warp(window + 60).await;
    let values = [
        [1_010_000, 1_020_000, 1_030_000],
        [1_040_000, 1_050_000, 1_060_000],
        [1_020_000, 1_040_000, 1_080_000],
    ];
    let mut updates = Vec::new();
    for channel in 0..3 {
        for j in 0..3 {
            let id = ids[channel][j];
            let o = env.head(id).await.origin_id;
            updates.push(env.update(&f, id, values[channel][j], env.now, o).await);
        }
    }
    env.warp(freeze - DAY).await;
    for p in updates {
        env.finalize(p, &f.pubkey()).await;
    }
    // A source case retires LTA's third source just after the freeze.
    let ix = hx::challenge_source(
        &program(),
        &x.pubkey(),
        &x.pubkey(),
        &env.product,
        ids[2][2],
        hx::ChallengeSourceParams {
            bond: USDC,
            reason: HeadChallengeReason::DuplicateOrigin,
            evidence_kind: 0,
            evidence_commitment: [0; 32],
            reason_hash: heads_reason_hash(b"a republished table"),
        },
        None,
    )
    .unwrap();
    env.send(ix, &[&x]).await.unwrap();
    env.warp(freeze + 1).await;
    env.rule_source(ids[2][2], &x.pubkey(), true).await;
    env.send(
        hx::finalize_source_challenges(&program(), &env.product, ids[2][2], &a.pubkey()),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(env.head(ids[2][2]).await.status, HeadStatus::Retired);
    // LTA's survivors move far after the freeze: those levels must not settle.
    env.warp(freeze + DAY).await;
    let mut late = Vec::new();
    for &id in &ids[2][..2] {
        let o = env.head(id).await.origin_id;
        late.push(env.update(&f, id, 2_000_000, env.now, o).await);
    }
    env.warp(env.now + DAY).await;
    for p in late {
        env.finalize(p, &f.pubkey()).await;
    }
    env.warp(expiry + DAY).await;
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    let contract = env.contract(expiry).await;
    // Spot and contract settle live on their medians, +2% and +5%; LTA on its freeze value, +4%.
    for (c, change) in [(0, 20_000), (1, 50_000)] {
        assert_ne!(contract.component_flags[c] & COMPONENT_LIVE, 0);
        assert_eq!(contract.component_change_ppm[c], change);
    }
    assert_ne!(contract.component_flags[2] & COMPONENT_LAST_VALID, 0);
    assert_eq!(contract.component_degraded[2], DEGRADED_TOO_FEW_ORIGINS);
    assert_eq!(contract.component_change_ppm[2], 40_000);
    env.send(hx::finalize_contract(&program(), &env.product, expiry), &[])
        .await
        .unwrap();
    let contract = env.contract(expiry).await;
    // 0.2 x 2% + 0.5 x 5% + 0.3 x 4% = 4.1%, LTA flagged as held at its last valid value.
    // Redistributing LTA's 30% onto the others would give 4.14%; its survivors' +100%, 34.1%.
    assert_eq!(contract.item_reference_ppm[0], 41_000);
    assert_eq!(contract.item_change_ppm[0], 41_000);
    assert_eq!(contract.item_degraded_channels[0], 0b100);
    assert_eq!(
        contract.item_reference_flags[0],
        REFERENCE_COMPUTED | REFERENCE_AVAILABLE | REFERENCE_ASSUMED
    );
    assert_eq!(contract.index_change_ppm, 41_000);
    assert_eq!(contract.value, 52_050_000);
    assert_eq!(
        env.h.period(env.product, expiry).await.settlement_value,
        52_050_000
    );
}

/// Degraded rule 2 after the freeze: three origins at listing and at freeze, one retired after
/// the freeze, so two are eligible at settlement. The component settles at its value as of the
/// freeze (the levels then, the same raw weights, the sources live then), not on the two left.
/// A new listing with two origins is refused outright.
#[tokio::test]
async fn sources_lost_after_the_freeze_settle_on_the_freeze_value() {
    let mut params = test_params();
    params.max_strikes = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(1_000 * AMBA, 100 * USDC).await;
    let x = env.user(0, 100 * USDC).await;
    let mut ids = Vec::new();
    let mut fills = Vec::new();
    for j in 0..3 {
        let id = env.active_head(&a, &b, 1).await;
        ids.push(id);
        fills.push(
            env.fill(&f, id, 1_000_000, env.now, origin(&format!("o{j}")))
                .await,
        );
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    assert_eq!(env.contract(expiry).await.component_listed_origins[1], 3);
    let freeze = env.contract(expiry).await.freeze_ts;
    // Prints in the freeze's own level window: +2%, +4%, +8%.
    // At the freeze level's window's start: every freeze-window sample takes them.
    let window = env.freeze_window(expiry).await;
    env.warp(window + 60).await;
    let mut updates = Vec::new();
    for (&id, value) in ids.iter().zip([1_020_000, 1_040_000, 1_080_000]) {
        let o = env.head(id).await.origin_id;
        updates.push(env.update(&f, id, value, env.now, o).await);
    }
    env.warp(freeze - DAY).await;
    for p in updates {
        env.finalize(p, &f.pubkey()).await;
    }
    // A source case retires the third source just after the freeze.
    let ix = hx::challenge_source(
        &program(),
        &x.pubkey(),
        &x.pubkey(),
        &env.product,
        ids[2],
        hx::ChallengeSourceParams {
            bond: USDC,
            reason: HeadChallengeReason::DuplicateOrigin,
            evidence_kind: 0,
            evidence_commitment: [0; 32],
            reason_hash: heads_reason_hash(b"a republished table"),
        },
        None,
    )
    .unwrap();
    env.send(ix, &[&x]).await.unwrap();
    env.warp(freeze + 1).await;
    env.rule_source(ids[2], &x.pubkey(), true).await;
    env.send(
        hx::finalize_source_challenges(&program(), &env.product, ids[2], &a.pubkey()),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(env.head(ids[2]).await.status, HeadStatus::Retired);
    let board = env.board().await;
    let slot = usize::from(env.contract(expiry).await.ring_slot);
    // The board captured the origins at the freeze before the retirement changed them.
    assert_eq!(board.contract_freeze_origins[slot * 24 + 1], 3);
    assert_eq!(board.component_live_origins[1], 2);
    // The survivors move far after the freeze: those levels must not settle.
    env.warp(freeze + DAY).await;
    let mut late = Vec::new();
    for &id in &ids[..2] {
        let o = env.head(id).await.origin_id;
        late.push(env.update(&f, id, 2_000_000, env.now, o).await);
    }
    env.warp(env.now + DAY).await;
    for p in late {
        env.finalize(p, &f.pubkey()).await;
    }
    env.warp(expiry + DAY).await;
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    let contract = env.contract(expiry).await;
    assert_eq!(contract.component_freeze_origins[1], 3);
    assert_ne!(contract.component_flags[1] & COMPONENT_LAST_VALID, 0);
    assert_eq!(contract.component_degraded[1], DEGRADED_TOO_FEW_ORIGINS);
    // The median of +2%, +4%, +8% under equal (capped) weights.
    assert_eq!(contract.component_change_ppm[1], 40_000);
    assert_eq!(contract.component_freeze_change_ppm[1], 40_000);
    let (_, sources) = env.weights(expiry, 1).await.unwrap();
    assert_eq!(
        sources
            .iter()
            .filter(|s| s.flags & WEIGHT_SOURCE_ELIGIBLE != 0)
            .count(),
        2
    );
    env.send(hx::finalize_contract(&program(), &env.product, expiry), &[])
        .await
        .unwrap();
    assert_eq!(env.contract(expiry).await.value, 52 * USDC);

    let expiry2 = FEB_01_2027;
    env.create_period(env.now + DAY, expiry2).await;
    env.warp(env.now + DAY).await;
    let terms = env.terms(expiry2).await;
    let ix = env.list_ix(
        expiry2,
        HeadsSettlementTarget::ContractChannel,
        terms,
        50 * USDC,
    );
    let oracle = env.h.oracle_authority.insecure_clone();
    assert_oracle_error(
        env.send(ix, &[&oracle]).await,
        OracleError::InvalidHeadsState,
    );
}

/// Listing passes, freeze fails: listed with three origins, one retired before the freeze, so
/// the component fails its freeze check and settles on its degraded path (the listing value),
/// never on the sources it has later; the reference holds it, and the display channels without
/// sources, at the listing value.
#[tokio::test]
async fn a_component_failing_its_freeze_check_settles_at_the_listing_value() {
    let mut params = test_params();
    params.max_strikes = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(0, 100 * USDC).await;
    let mut ids = Vec::new();
    let mut fills = Vec::new();
    for j in 0..3 {
        let id = env.active_head(&a, &b, 1).await;
        ids.push(id);
        fills.push(
            env.fill(&f, id, 1_000_000, env.now, origin(&format!("o{j}")))
                .await,
        );
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    let ix = hx::challenge_source(
        &program(),
        &x.pubkey(),
        &x.pubkey(),
        &env.product,
        ids[2],
        hx::ChallengeSourceParams {
            bond: USDC,
            reason: HeadChallengeReason::ValueOrOther,
            evidence_kind: 0,
            evidence_commitment: [0; 32],
            reason_hash: heads_reason_hash(b"fabricated"),
        },
        None,
    )
    .unwrap();
    env.send(ix, &[&x]).await.unwrap();
    env.warp(env.now + DAY).await;
    env.rule_source(ids[2], &x.pubkey(), true).await;
    env.send(
        hx::finalize_source_challenges(&program(), &env.product, ids[2], &a.pubkey()),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(env.head(ids[2]).await.status, HeadStatus::Retired);
    // A fourth source with a new origin goes live just after the freeze: three origins again,
    // too late (it carries no window weight, and the freeze check stands).
    let freeze = env.contract(expiry).await.freeze_ts;
    env.warp(freeze + 1).await;
    let late = env.active_head(&a, &b, 1).await;
    let p = env
        .fill(&f, late, 1_000_000, env.now, origin("o-late"))
        .await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    env.warp(expiry + DAY).await;
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    let contract = env.contract(expiry).await;
    assert_eq!(contract.component_freeze_origins[1], 2);
    assert_eq!(contract.component_degraded[1], DEGRADED_FREEZE_CHECK_FAILED);
    assert_eq!(contract.component_change_ppm[1], 0);
    env.send(hx::finalize_contract(&program(), &env.product, expiry), &[])
        .await
        .unwrap();
    let contract = env.contract(expiry).await;
    assert_eq!(contract.value, 50 * USDC);
    // Every channel is on its degraded path at the listing value: the reference is 0, each
    // channel flagged.
    assert_eq!(
        contract.item_reference_flags[0],
        REFERENCE_COMPUTED | REFERENCE_AVAILABLE | REFERENCE_ASSUMED
    );
    assert_eq!(contract.item_reference_ppm[0], 0);
    assert_eq!(contract.item_degraded_channels[0], 0b111);
}

/// An LTA print past its own next reset is no level: the source is not eligible; a print still
/// in effect is, however old. Duplicates carry no weight: a later source repeating an origin is
/// dropped from the median whatever it prints.
#[tokio::test]
async fn lta_prints_past_their_reset_and_duplicates_carry_no_weight() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(60_000 * AMBA, 0).await;
    let b = env.user(60_000 * AMBA, 0).await;
    let f = env.user(0, 200 * USDC).await;
    let expiry = DEC_01_2026;
    // Contract: three origins plus a duplicate of the first (posted later, so it is the one
    // dropped); LTA: four origins.
    let mut contract_ids = Vec::new();
    let mut lta_ids = Vec::new();
    let mut fills = Vec::new();
    for j in 0..3 {
        let id = env.active_head(&a, &b, 1).await;
        contract_ids.push(id);
        fills.push(
            env.fill(&f, id, 1_000_000, env.now, origin(&format!("c{j}")))
                .await,
        );
    }
    // Terms: in effect from October; three reset after expiry, one resets on 2026-11-20, inside
    // the window's month but before the window's end.
    let resets = [
        JAN_01_2027,
        JAN_01_2027,
        JAN_01_2027,
        NOV_01_2026 + 19 * DAY,
    ];
    for (j, reset) in resets.iter().enumerate() {
        let id = env.active_head(&a, &b, 2).await;
        lta_ids.push(id);
        let lta = hx::HeadsLtaTerms {
            effective_from_ts: OCT_01_2026,
            next_reset_ts: *reset,
            floor: 0,
            ceiling: 0,
        };
        let p = env
            .claim_with(
                &f,
                id,
                HeadClaimKind::Fill,
                1_000_000 + j as u64,
                env.now,
                origin(&format!("l{j}")),
                lta,
            )
            .await;
        fills.push(p);
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let duplicate = env.active_head(&a, &b, 1).await;
    let p = env
        .fill(&f, duplicate, 1_000_000, env.now, origin("c0"))
        .await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let board = env.board().await;
    assert_eq!(
        (
            board.component_live_sources[1],
            board.component_live_origins[1]
        ),
        (4, 3)
    );
    assert!(env.head(duplicate).await.duplicate);

    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 10 * USDC).await;
    let contract = env.contract(expiry).await;
    assert_eq!(
        (
            contract.component_listed_origins[1],
            contract.component_listed_origins[2]
        ),
        (3, 4)
    );
    // An updater moves the levels inside the window: the duplicate prints far away.
    let updater = env.user(0, 200 * USDC).await;
    let freeze = contract.freeze_ts;
    env.warp(freeze + DAY).await;
    let targets = [
        (contract_ids[0], 1_050_000),
        (contract_ids[1], 1_100_000),
        (contract_ids[2], 1_200_000),
        (duplicate, 9_000_000),
    ];
    let mut updates = Vec::new();
    for &(id, value) in &targets {
        let origin = if id == duplicate {
            origin("c0")
        } else {
            env.head(id).await.origin_id
        };
        updates.push(env.update(&updater, id, value, env.now, origin).await);
    }
    env.warp(env.now + DAY).await;
    for p in updates {
        env.finalize(p, &updater.pubkey()).await;
    }
    env.warp(expiry + DAY).await;
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    let contract = env.contract(expiry).await;
    // Contract: the duplicate is dropped; the median of +5%, +10%, +20% is +10%.
    assert_eq!(contract.component_duplicates[1], 1);
    assert_eq!(contract.component_eligible[1], 3);
    assert_eq!(contract.component_change_ppm[1], 100_000);
    let (_, sources) = env.weights(expiry, 1).await.unwrap();
    let dup = sources.iter().find(|s| s.head_id == duplicate).unwrap();
    assert_ne!(dup.flags & WEIGHT_SOURCE_DUPLICATE, 0);
    assert_eq!(dup.flags & WEIGHT_SOURCE_ELIGIBLE, 0);
    // LTA: the print past its reset is no level; three in effect remain (unchanged levels).
    let (_, sources) = env.weights(expiry, 2).await.unwrap();
    let expired = sources.iter().find(|s| s.head_id == lta_ids[3]).unwrap();
    assert_eq!(
        expired.flags & (WEIGHT_SOURCE_LEVEL | WEIGHT_SOURCE_ELIGIBLE),
        0
    );
    assert_eq!(
        sources
            .iter()
            .filter(|s| s.flags & WEIGHT_SOURCE_ELIGIBLE != 0)
            .count(),
        3
    );
    assert_ne!(contract.component_flags[2] & COMPONENT_LIVE, 0);
    assert_eq!(contract.component_change_ppm[2], 0);
    env.send(hx::finalize_contract(&program(), &env.product, expiry), &[])
        .await
        .unwrap();
    let contract = env.contract(expiry).await;
    assert_eq!(contract.index_change_ppm, 100_000);
    assert_eq!(contract.value, 11 * USDC);
    // The reference (displayed under the Contract channel alone): spot had no origins, so it
    // holds its 20% at the listing value; 0.5 x 10% + 0.3 x 0% = 5%.
    assert_eq!(contract.item_reference_ppm[0], 50_000);
    assert_ne!(contract.item_reference_flags[0] & REFERENCE_ASSUMED, 0);
    assert_eq!(contract.item_degraded_channels[0], 0b001);
}

/// The security edges of the part B instructions not covered above: identities, signers,
/// payloads bound to their accounts, ordering, the settlement's completeness checks.
#[tokio::test]
async fn security_edges() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(40_000 * AMBA, 0).await;
    let b = env.user(40_000 * AMBA, 0).await;
    let f = env.user(1_000 * AMBA, 100 * USDC).await;
    let poor = env.user(0, 0).await;
    let id = env.active_head(&a, &b, 1).await;
    let product = env.product;

    // LTA terms off the LTA channel: the commit binds them, the reveal refuses them, and the
    // unrevealed commit is refunded after its deadline.
    let lta = hx::HeadsLtaTerms {
        effective_from_ts: 1,
        next_reset_ts: 2,
        floor: 0,
        ceiling: 0,
    };
    let bad = Env::reveal_params(1_000_000, env.now, origin("one"), lta, 0);
    let ix = env.commit_ix(&f.pubkey(), id, 0, HeadClaimKind::Fill, &bad);
    env.send(ix, &[&f]).await.unwrap();
    env.h.advance_slot().await;
    let ix = hx::reveal_claim(&program(), &f.pubkey(), &product, id, 0, bad, None).unwrap();
    assert_oracle_error(env.send(ix, &[&f]).await, OracleError::InvalidHeadsState);
    let deadline = env.claim(Pending { id, seq: 0 }).await.reveal_deadline_slot;
    env.h.ctx.warp_to_slot(deadline + 1).unwrap();
    set_unix_timestamp(&env.h.ctx, env.now).await;
    env.send(
        hx::refund_expired_commit(&program(), &product, id, 0, &f.pubkey()),
        &[],
    )
    .await
    .unwrap();
    assert!(!env.head(id).await.open_fill);

    // CommitClaim: the expected seq, the board's bond, the canonical claim, the fetcher's
    // collateral, an update only of a Live head.
    let reveal = Env::reveal_params(1_000_000, env.now, origin("one"), Default::default(), 1);
    let hash = heads_claim_commitment(&env.head_key(id), 1, &f.pubkey(), &reveal);
    let ix = hx::commit_claim(
        &program(),
        &f.pubkey(),
        &f.pubkey(),
        &product,
        id,
        2,
        HeadClaimKind::Fill,
        hash,
        env.params.claim_bond,
    );
    assert!(env.send(ix, &[&f]).await.is_err());
    let ix = hx::commit_claim(
        &program(),
        &f.pubkey(),
        &f.pubkey(),
        &product,
        id,
        1,
        HeadClaimKind::Fill,
        hash,
        env.params.claim_bond + 1,
    );
    assert_oracle_error(env.send(ix, &[&f]).await, OracleError::InvalidHeadsState);
    let mut ix = hx::commit_claim(
        &program(),
        &f.pubkey(),
        &f.pubkey(),
        &product,
        id,
        1,
        HeadClaimKind::Fill,
        hash,
        env.params.claim_bond,
    );
    ix.accounts[5].pubkey = Pubkey::new_unique();
    assert_oracle_error(env.send(ix, &[&f]).await, OracleError::InvalidHeadsAccount);
    let ix = hx::commit_claim(
        &program(),
        &poor.pubkey(),
        &poor.pubkey(),
        &product,
        id,
        1,
        HeadClaimKind::Fill,
        hash,
        env.params.claim_bond,
    );
    assert_oracle_error(
        env.send(ix, &[&poor]).await,
        OracleError::InsufficientAvailableCollateral,
    );
    let ix = hx::commit_claim(
        &program(),
        &f.pubkey(),
        &f.pubkey(),
        &product,
        id,
        1,
        HeadClaimKind::Update,
        hash,
        env.params.claim_bond,
    );
    assert_oracle_error(env.send(ix, &[&f]).await, OracleError::InvalidHeadsState);
    let mut ix = hx::commit_claim(
        &program(),
        &f.pubkey(),
        &f.pubkey(),
        &product,
        id,
        1,
        HeadClaimKind::Fill,
        hash,
        env.params.claim_bond,
    );
    // Neither the payer nor the fetcher (the same key here) signs.
    ix.accounts[0].is_signer = false;
    ix.accounts[1].is_signer = false;
    assert!(env.send(ix, &[]).await.is_err());
    let ix = hx::commit_claim(
        &program(),
        &f.pubkey(),
        &f.pubkey(),
        &product,
        id,
        1,
        HeadClaimKind::Fill,
        hash,
        env.params.claim_bond,
    );
    env.send(ix, &[&f]).await.unwrap();
    env.h.advance_slot().await;

    // RevealClaim: the committed payload, by its fetcher, an evidence object matching its kind.
    let mut wrong = reveal.clone();
    wrong.value += 1;
    let ix = hx::reveal_claim(&program(), &f.pubkey(), &product, id, 1, wrong, None).unwrap();
    assert_oracle_error(env.send(ix, &[&f]).await, OracleError::InvalidHeadsState);
    let ix = hx::reveal_claim(
        &program(),
        &a.pubkey(),
        &product,
        id,
        1,
        reveal.clone(),
        None,
    )
    .unwrap();
    assert_oracle_error(env.send(ix, &[&a]).await, OracleError::InvalidHeadsState);
    let mut evidence = reveal.clone();
    evidence.evidence_kind = 2;
    evidence.evidence_commitment = [7; 32];
    let ix = hx::reveal_claim(&program(), &f.pubkey(), &product, id, 1, evidence, None).unwrap();
    assert!(env.send(ix, &[&f]).await.is_err());
    let ix = hx::reveal_claim(
        &program(),
        &f.pubkey(),
        &product,
        id,
        1,
        reveal.clone(),
        None,
    )
    .unwrap();
    env.send(ix, &[&f]).await.unwrap();
    let ix = hx::reveal_claim(&program(), &f.pubkey(), &product, id, 1, reveal, None).unwrap();
    assert_oracle_error(env.send(ix, &[&f]).await, OracleError::InvalidHeadsState);
    let fill = Pending { id, seq: 1 };

    // ChallengeClaim on another head's claim account is refused.
    let other = env.active_head(&a, &b, 1).await;
    let mut ix = hx::challenge_claim(
        &program(),
        &poor.pubkey(),
        &poor.pubkey(),
        &product,
        other,
        0,
        Env::challenge_params(USDC, HeadChallengeReason::ValueOrOther),
        None,
    )
    .unwrap();
    ix.accounts[5].pubkey = env.claim_key(id, 1);
    assert!(env.send(ix, &[&poor]).await.is_err());
    // A poor challenger cannot bond.
    assert_oracle_error(
        env.challenge(&poor, fill, USDC).await,
        OracleError::InsufficientAvailableCollateral,
    );

    // RuleChallenge with the wrong council Config.
    let x = env.user(0, 10 * USDC).await;
    env.challenge(&x, fill, USDC).await.unwrap();
    let epoch = env.epoch().await;
    let (keys, signers) = env.seats3([0, 1, 2]);
    let refs: Vec<&Keypair> = signers.iter().collect();
    let mut ix = hx::rule_claim_challenge(
        &program(),
        &env.council,
        &keys,
        &product,
        id,
        1,
        &x.pubkey(),
        true,
        epoch,
    );
    ix.accounts[0].pubkey = env.board;
    assert!(env.send(ix, &refs).await.is_err());
    // A source ruling of a claim challenge is refused.
    let ix = hx::rule_source_challenge(
        &program(),
        &env.council,
        &keys,
        &product,
        id,
        &x.pubkey(),
        true,
        epoch,
    );
    assert!(env.send(ix, &refs).await.is_err());
    env.warp(env.now + DAY).await;
    env.rule(fill, &x.pubkey(), false).await;
    // FinalizeClaim with another fetcher's collateral is refused.
    let ix = env.finalize_ix(fill, &x.pubkey());
    assert!(env.send(ix, &[]).await.is_err());
    env.finalize(fill, &f.pubkey()).await;
    // SettleChallenge pays the challenger only.
    let mut ix = hx::settle_claim_challenge(&program(), &product, id, 1, &x.pubkey(), &x.pubkey());
    ix.accounts[7].pubkey = f.pubkey();
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsAccount);
    env.settle_claim_challenge(fill, &x.pubkey()).await.unwrap();

    // The second head live, a third with one origin: a contract lists (three origins needed:
    // add one more).
    let mut fills = Vec::new();
    let third = env.active_head(&a, &b, 1).await;
    for (head, o) in [(other, "two"), (third, "three")] {
        fills.push(env.fill(&f, head, 1_000_000, env.now, origin(o)).await);
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let expiry = DEC_01_2026;
    let period = env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    let terms = env.terms(expiry).await;
    let oracle = env.h.oracle_authority.insecure_clone();
    // ListContract: the terms object must be the committed one, the month PDA absent, a nonzero
    // base.
    let mut ix = env.list_ix(
        expiry,
        HeadsSettlementTarget::ContractChannel,
        ([9; 32], terms.1),
        1_000_000,
    );
    assert_oracle_error(
        env.send(ix.clone(), &[&oracle]).await,
        OracleError::InvalidHeadsEvidence,
    );
    ix = env.list_ix(expiry, HeadsSettlementTarget::ContractChannel, terms, 0);
    assert_oracle_error(
        env.send(ix, &[&oracle]).await,
        OracleError::InvalidHeadsState,
    );
    let month = instruction::find_oracle_month_address(&program(), &period, expiry);
    let mut occupied = AccountSharedData::new(1_000_000, 8, &program());
    occupied.set_data_from_slice(&[1; 8]);
    env.h
        .set_account_preserving_capitalization(month, occupied)
        .await;
    let ix = env.list_ix(
        expiry,
        HeadsSettlementTarget::ContractChannel,
        terms,
        1_000_000,
    );
    assert_oracle_error(
        env.send(ix, &[&oracle]).await,
        OracleError::InvalidOracleMonthAccount,
    );
    env.h.set_system_account_with_lamports(month, 0).await;
    env.list(expiry, 1_000_000).await;

    // A claim committed before expiry and still open holds its source's level; one committed
    // after expiry never holds settlement up (its print would not count either).
    env.warp(expiry - DAY).await;
    let seq = env.head(id).await.claim_seq;
    let reveal = Env::reveal_params(1_100_000, env.now, origin("one"), Default::default(), seq);
    let ix = env.commit_ix(&f.pubkey(), id, seq, HeadClaimKind::Update, &reveal);
    env.send(ix, &[&f]).await.unwrap();
    env.warp(expiry + DAY).await;
    let late_seq = env.head(other).await.claim_seq;
    let late = Env::reveal_params(
        9_000_000,
        env.now,
        origin("two"),
        Default::default(),
        late_seq,
    );
    let ix = env.commit_ix(&f.pubkey(), other, late_seq, HeadClaimKind::Update, &late);
    env.send(ix, &[&f]).await.unwrap();
    let payer = env.h.ctx.payer.pubkey();
    let ix = hx::settle_component(
        &program(),
        &payer,
        &product,
        expiry,
        1,
        0,
        &[id, other, third],
    )
    .unwrap();
    assert_oracle_error(
        env.send(ix.clone(), &[]).await,
        OracleError::InvalidHeadsState,
    );
    let deadline = env.claim(Pending { id, seq }).await.reveal_deadline_slot;
    env.h.ctx.warp_to_slot(deadline + 1).unwrap();
    set_unix_timestamp(&env.h.ctx, env.now).await;
    env.send(
        hx::refund_expired_commit(&program(), &product, id, seq, &f.pubkey()),
        &[],
    )
    .await
    .unwrap();
    // A head passed twice is refused (the record holds each source once).
    let ix2 =
        hx::settle_component(&program(), &payer, &product, expiry, 1, 0, &[id, id, third]).unwrap();
    assert_oracle_error(env.send(ix2, &[]).await, OracleError::InvalidHeadsState);
    // The late claim on `other` is still open, and the component settles anyway.
    assert_eq!(env.head(other).await.open_claims, 1);
    env.settle(expiry, 1, &[]).await;
    // A settled component does not settle again.
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    for component in [0u8, 2] {
        env.settle(expiry, component, &[]).await;
    }
    env.send(hx::finalize_contract(&program(), &product, expiry), &[])
        .await
        .unwrap();
    assert_oracle_error(
        env.send(hx::finalize_contract(&program(), &product, expiry), &[])
            .await,
        OracleError::InvalidHeadsAccount,
    );
    // ClaimPrize before its month ends is refused (December's claims).
    let p = env.update(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let month = env.claim(p).await.epoch;
    let ix = hx::claim_prize(&program(), &f.pubkey(), &product, id, p.seq, month);
    assert_oracle_error(env.send(ix, &[&f]).await, OracleError::InvalidHeadsState);
    let epoch: Box<HeadsEpoch> = env
        .read(find_heads_epoch_address(&program(), &env.board, month).0)
        .await;
    assert_eq!(
        epoch.prize_points,
        rewards::update_prize(&env.params.to_params(), env.head(id).await.delegation)
    );
}

/// A wide component: twelve sources settle over two `SettleComponent` calls (eight heads a call)
/// while the weights account grows. No cap (the user, 2026-10-29: "i dont want a hard source cap
/// lol"): a whale's source carrying more than half the delegation sets the median. The settlement
/// is exactly `settle::settle_component` on the account's own records.
#[tokio::test]
async fn a_wide_component_settles_over_several_calls() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let whale = env.user(240_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let mut ids = Vec::new();
    let mut fills = Vec::new();
    for j in 0..12u32 {
        let id = env.post(&a, 1, 1_000 * AMBA).await;
        let big = if j == 0 {
            200_000
        } else {
            1_000 + 100 * u64::from(j)
        };
        env.delegate(&whale, id, big * AMBA).await;
        assert_eq!(env.head(id).await.status, HeadStatus::Active);
        fills.push(
            env.fill(&f, id, 1_000_000, env.now, origin(&format!("w{j}")))
                .await,
        );
        ids.push(id);
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 10 * USDC).await;
    let freeze = env.contract(expiry).await.freeze_ts;
    env.warp(freeze + DAY).await;
    let mut updates = Vec::new();
    for (j, &id) in ids.iter().enumerate() {
        let value = 1_000_000 + 37_000 * (j as u64 * 3 % 7 + 1);
        updates.push(
            env.update(&f, id, value, env.now, origin(&format!("w{j}")))
                .await,
        );
    }
    env.warp(env.now + DAY).await;
    for p in updates {
        env.finalize(p, &f.pubkey()).await;
    }
    env.warp(expiry + DAY).await;
    let cu = env.settle(expiry, 1, &[]).await;
    for (label, units) in &cu {
        eprintln!("CU wide {label}: {units}");
    }
    assert_eq!(cu.len(), 2, "two calls: {cu:?}");
    let (header, sources) = env.weights(expiry, 1).await.unwrap();
    assert_eq!((header.source_count, header.levels_recorded), (12, 12));
    assert!(header.source_capacity >= 12);

    // The crate on the account's own records.
    let inputs: Vec<settle::SourceInput> = sources
        .iter()
        .map(|s| settle::SourceInput {
            live: true,
            change_ppm: Some(s.change_ppm),
            weight: s.weight,
            origin: s.origin_id,
            live_at: s.live_at,
            head_id: s.head_id,
        })
        .collect();
    let expected = settle::settle_component(&env.params.to_params(), &inputs, 0, true);
    let contract = env.contract(expiry).await;
    let settle::ComponentSettlement::Live {
        change_ppm, weight, ..
    } = expected
    else {
        panic!("{expected:?}")
    };
    assert_eq!(contract.component_change_ppm[1], change_ppm);
    assert_eq!(contract.component_weight[1], weight);
    assert_eq!(header.weight_total, weight);
    // The whale's source holds most of the weight, so its change is the median: +3.7%.
    let whale_source = sources.iter().find(|s| s.head_id == ids[0]).unwrap();
    assert!(whale_source.weight * 2 > weight);
    assert_eq!(change_ppm, whale_source.change_ppm);
    assert_eq!(change_ppm, 37_000);
}

// Security review 2026-10-30
// ------------------------------------------------------------------------------------------

impl Env {
    /// Commits a claim and tries its reveal in the next slot; returns where it is and the
    /// reveal's result.
    async fn try_claim(
        &mut self,
        fetcher: &Keypair,
        id: u32,
        kind: HeadClaimKind,
        value: u64,
        source_ts: u64,
        origin: [u8; 32],
    ) -> (Pending, Result<(), BanksClientError>) {
        let seq = self.head(id).await.claim_seq;
        let reveal = Self::reveal_params(value, source_ts, origin, Default::default(), seq);
        let ix = self.commit_ix(&fetcher.pubkey(), id, seq, kind, &reveal);
        self.send(ix, &[fetcher]).await.unwrap();
        self.h.advance_slot().await;
        let ix = hx::reveal_claim(
            &program(),
            &fetcher.pubkey(),
            &self.product,
            id,
            seq,
            reveal,
            None,
        )
        .unwrap();
        let result = self.send(ix, &[fetcher]).await;
        (Pending { id, seq }, result)
    }

    async fn prize_points(&mut self) -> u64 {
        let key = find_heads_epoch_address(&program(), &self.board, time::month_id(self.now)).0;
        match self.h.account(key).await {
            Some(_) => self.read::<HeadsEpoch>(key).await.prize_points,
            None => 0,
        }
    }
}

/// Review #3, as round 3 R1 has it: a replay of an accepted observation (its source time again)
/// can neither move a level nor earn another prize. No reveal shuts out another, so it reveals
/// and stands, but a sample takes the latest observation at or before its instant (the same
/// one, or a later one) and its day already earned the head's prize; so for a second claim of one
/// new observation: both stand, one prize.
#[tokio::test]
async fn an_observation_prints_once_per_head() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(10_000 * AMBA, 0).await;
    let b = env.user(10_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    // Three observations: 1.00, 2.00, 3.00.
    let mut seen = Vec::new();
    for value in [1_000_000u64, 2_000_000, 3_000_000] {
        env.warp(env.now + 3_600).await;
        let ts = env.now;
        let p = env.update(&f, id, value, ts, origin("one")).await;
        env.warp(env.now + DAY).await;
        env.finalize(p, &f.pubkey()).await;
        seen.push(ts);
    }
    let prints = env.head(id).await.print_count;
    let points = env.prize_points().await;
    // Replays of the first observation and of the last: each reveals.
    let mut claims = Vec::new();
    for (ts, value) in [(seen[0], 1_000_000), (seen[2], 3_000_000)] {
        let (p, result) = env
            .try_claim(&f, id, HeadClaimKind::Update, value, ts, origin("one"))
            .await;
        result.unwrap();
        claims.push(p);
    }
    // Two claims of one new observation.
    let ts = env.now;
    claims.push(env.update(&f, id, 1_000_000, ts, origin("one")).await);
    claims.push(env.update(&f, id, 1_000_000, ts, origin("one")).await);
    env.warp(env.now + DAY).await;
    for p in claims {
        env.finalize(p, &f.pubkey()).await;
    }
    let head = env.head(id).await;
    assert_eq!(head.print_count, prints + 4);
    assert_eq!((head.value, head.value_source_ts), (1_000_000, ts));
    let prize = rewards::update_prize(&env.params.to_params(), head.delegation);
    assert_eq!(
        env.prize_points().await,
        points + prize,
        "one prize, the new day's"
    );
}

/// Review #5: claims committed after a contract's expiry cannot erase the prints its levels read.
/// A sole source prints +20% in the level window; after expiry 32 updates fill the head's ring
/// before anyone records the source, yet the contract settles at +20%: the head fixed its levels
/// for the contract before the first of them printed, and the contract admits none of them, in
/// whatever order they finalize.
#[tokio::test]
async fn post_expiry_updates_keep_the_contracts_levels() {
    let mut params = test_params();
    params.min_live_sources = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(10_000 * AMBA, 0).await;
    let b = env.user(10_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    let freeze = env.contract(expiry).await.freeze_ts;
    env.warp(freeze + DAY).await;
    let p = env.update(&f, id, 1_200_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    // A claim committed just before the expiry, still in its window.
    env.warp(expiry - 60).await;
    let before = env.update(&f, id, 1_200_000, env.now, origin("one")).await;
    // After the expiry, 32 updates: as many as the ring holds.
    env.warp(expiry + 60).await;
    let mut after = Vec::new();
    for _ in 0..HEAD_PRINT_CAPACITY {
        env.warp(env.now + 1).await;
        after.push(env.update(&f, id, 900_000, env.now, origin("one")).await);
    }
    env.warp(env.now + DAY).await;
    // Any order (round 3 R1): the 32 first, then the claim committed before the expiry.
    for p in after {
        env.finalize(p, &f.pubkey()).await;
    }
    env.finalize(before, &f.pubkey()).await;
    let head = env.head(id).await;
    assert_eq!(head.value, 900_000);
    assert!(head.print_count as usize >= HEAD_PRINT_CAPACITY + 2);
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    let contract = env.contract(expiry).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_LIVE, 0);
    assert_eq!(contract.component_change_ppm[1], 200_000);
    let (_, sources) = env.weights(expiry, 1).await.unwrap();
    assert_eq!(sources[0].level, 1_200_000);
}

/// Review #6: an origin declared after the expiry cannot rewrite the contract's duplicates. A
/// (4,000 AMBA, -10%) and B (6,000, +20%) carry distinct origins, so the median is +20%; after
/// the expiry an update gives A, the earlier source, B's origin. Its price is left out of the
/// contract, and so is its origin: the contract settles at +20% with both sources eligible.
#[tokio::test]
async fn a_post_expiry_origin_cannot_rewrite_the_contracts_duplicates() {
    let mut params = test_params();
    params.min_live_sources = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let mut ids = Vec::new();
    for (backing, name) in [(3_000, "a"), (5_000, "b")] {
        let id = env.post(&a, 1, 1_000 * AMBA).await;
        env.delegate(&b, id, backing * AMBA).await;
        let p = env.fill(&f, id, 1_000_000, env.now, origin(name)).await;
        env.warp(env.now + DAY).await;
        env.finalize(p, &f.pubkey()).await;
        ids.push(id);
    }
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    let freeze = env.contract(expiry).await.freeze_ts;
    env.warp(freeze + DAY).await;
    let mut updates = Vec::new();
    for (&id, (value, name)) in ids.iter().zip([(900_000, "a"), (1_200_000, "b")]) {
        updates.push(env.update(&f, id, value, env.now, origin(name)).await);
    }
    env.warp(env.now + DAY).await;
    for p in updates {
        env.finalize(p, &f.pubkey()).await;
    }
    env.warp(expiry + 60).await;
    let p = env.update(&f, ids[0], 950_000, env.now, origin("b")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    assert_eq!(env.head(ids[0]).await.origin_id, origin("b"));
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    let contract = env.contract(expiry).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_LIVE, 0);
    assert_eq!(contract.component_change_ppm[1], 200_000);
    assert_eq!(contract.component_eligible[1], 2);
    let (_, sources) = env.weights(expiry, 1).await.unwrap();
    let first = sources.iter().find(|s| s.head_id == ids[0]).unwrap();
    assert_eq!(first.origin_id, origin("a"));
}

impl Env {
    /// A source case against `id` that the council upholds: it falls, strikes the source and
    /// pays the challenger (`x`).
    async fn lose_source_case(&mut self, id: u32, x: &Keypair) {
        let proposer = self.head(id).await.proposer;
        let ix = hx::challenge_source(
            &program(),
            &x.pubkey(),
            &x.pubkey(),
            &self.product,
            id,
            hx::ChallengeSourceParams {
                bond: USDC,
                reason: HeadChallengeReason::ValueOrOther,
                evidence_kind: 0,
                evidence_commitment: [0; 32],
                reason_hash: heads_reason_hash(b"fabricated"),
            },
            None,
        )
        .unwrap();
        self.send(ix, &[x]).await.unwrap();
        self.warp(self.now + self.params.challenge_window_secs)
            .await;
        self.rule_source(id, &x.pubkey(), true).await;
        let ix = hx::finalize_source_challenges(&program(), &self.product, id, &proposer);
        self.send(ix, &[]).await.unwrap();
        let ix =
            hx::settle_source_challenge(&program(), &self.product, id, &x.pubkey(), &x.pubkey());
        self.send(ix, &[]).await.unwrap();
    }

    fn settle_ix(&self, expiry: u64, component: u8, first: u16, heads: &[u32]) -> Instruction {
        hx::settle_component(
            &program(),
            &self.h.ctx.payer.pubkey(),
            &self.product,
            expiry,
            component,
            first,
            heads,
        )
        .unwrap()
    }
}

/// Review #2: a contract's membership is fixed, so nobody chooses its fallback by leaving a source
/// out. A was live at listing; B joined during the accumulation window. Both were live at the
/// freeze and retired after it, so the component settles on its freeze value: A (light, -10%)
/// and B (heavy, +20%) give +20%. Recording A alone does not settle it: B, though retired and off
/// the board's live list, must be recorded too.
#[tokio::test]
async fn a_source_that_joined_the_window_cannot_be_left_out() {
    let mut params = test_params();
    params.min_live_sources = 1;
    params.max_strikes = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let first = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, first, 1_000_000, env.now, origin("a")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    // B joins inside the window, with far more backing.
    let second = env.post(&a, 1, 1_000 * AMBA).await;
    env.delegate(&b, second, 9_000 * AMBA).await;
    let p = env.fill(&f, second, 1_000_000, env.now, origin("b")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let freeze = env.contract(expiry).await.freeze_ts;
    // At the freeze level's window's start: every freeze-window sample takes them.
    let window = env.freeze_window(expiry).await;
    env.warp(window + 60).await;
    let mut updates = Vec::new();
    for (id, value, name) in [(first, 900_000, "a"), (second, 1_200_000, "b")] {
        updates.push(env.update(&f, id, value, env.now, origin(name)).await);
    }
    env.warp(freeze - DAY).await;
    for p in updates {
        env.finalize(p, &f.pubkey()).await;
    }
    // Both retire after the freeze.
    env.warp(freeze + 3_600).await;
    env.lose_source_case(first, &x).await;
    env.lose_source_case(second, &x).await;
    assert_eq!(env.board().await.live_count, 0);
    env.warp(expiry + DAY).await;
    for component in [0u8, 2] {
        env.settle(expiry, component, &[]).await;
    }
    let ix = env.settle_ix(expiry, 1, 0, &[first]);
    env.send(ix, &[]).await.unwrap();
    assert_eq!(
        env.contract(expiry).await.component_flags[1] & COMPONENT_SETTLED,
        0,
        "settled without B"
    );
    let ix = env.settle_ix(expiry, 1, 1, &[second]);
    env.send(ix, &[]).await.unwrap();
    let contract = env.contract(expiry).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_SETTLED, 0);
    assert_ne!(contract.component_flags[1] & COMPONENT_LAST_VALID, 0);
    assert_eq!(contract.component_change_ppm[1], 200_000);
}

/// Review #4: eligibility has one cutoff, so the order sources are recorded in cannot change it.
/// A (6,000 AMBA, +100%) and B (4,000, 0%) were live when settlement opened (expiry plus the
/// challenge window). B is recorded first; then a later source case retires A; then A is
/// recorded. A retired after the cutoff, so it stays eligible exactly as if it had been recorded
/// first: +100%.
#[tokio::test]
async fn a_retirement_between_settlement_chunks_changes_nothing() {
    let mut params = test_params();
    params.min_live_sources = 1;
    params.max_strikes = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let mut ids = Vec::new();
    for (backing, name) in [(5_000, "a"), (3_000, "b")] {
        let id = env.post(&a, 1, 1_000 * AMBA).await;
        env.delegate(&b, id, backing * AMBA).await;
        let p = env.fill(&f, id, 1_000_000, env.now, origin(name)).await;
        env.warp(env.now + DAY).await;
        env.finalize(p, &f.pubkey()).await;
        ids.push(id);
    }
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    let freeze = env.contract(expiry).await.freeze_ts;
    env.warp(freeze + DAY).await;
    let mut updates = Vec::new();
    for (&id, (value, name)) in ids.iter().zip([(2_000_000, "a"), (1_000_000, "b")]) {
        updates.push(env.update(&f, id, value, env.now, origin(name)).await);
    }
    env.warp(env.now + DAY).await;
    for p in updates {
        env.finalize(p, &f.pubkey()).await;
    }
    env.warp(expiry + env.params.challenge_window_secs).await;
    for component in [0u8, 2] {
        env.settle(expiry, component, &[]).await;
    }
    let ix = env.settle_ix(expiry, 1, 0, &[ids[1]]);
    env.send(ix, &[]).await.unwrap();
    env.lose_source_case(ids[0], &x).await;
    assert_eq!(env.head(ids[0]).await.status, HeadStatus::Retired);
    let ix = env.settle_ix(expiry, 1, 1, &[ids[0]]);
    env.send(ix, &[]).await.unwrap();
    let contract = env.contract(expiry).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_LIVE, 0);
    assert_eq!(contract.component_eligible[1], 2);
    assert_eq!(contract.component_change_ppm[1], 1_000_000);
}

impl Env {
    async fn delegation_of(&mut self, id: u32, owner: &Pubkey) -> Box<Delegation> {
        let key = find_head_delegation_address(&program(), &self.head_key(id), owner).0;
        self.read(key).await
    }
}

/// Review #1: every AMBA a source case awards is AMBA taken from the proposer. With a 50%
/// proposer slash and two strikes, an exposed proposer undelegates 10,000: the escrow keeps what
/// both remaining cases can take (7,500), and the two falling cases award exactly what they
/// remove (5,000, then 2,500). What the proposer got back, what the winners got and what is still
/// held add up to the 10,000.
#[tokio::test]
async fn partial_proposer_slashes_award_only_what_they_take() {
    let mut params = test_params();
    params.min_live_sources = 1;
    params.proposer_slash_bps = 5_000;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let y = env.user(AMBA, 100 * USDC).await;
    let id = env.post(&a, 1, 10_000 * AMBA).await;
    env.delegate(&b, id, 1_000 * AMBA).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    // The proposer contributed to the contract, then leaves.
    env.warp(env.now + DAY).await;
    let ix = hx::undelegate(&program(), &a.pubkey(), &env.product, id, 10_000 * AMBA);
    env.send(ix, &[&a]).await.unwrap();
    env.warp(env.now + env.params.cooldown_secs).await;
    let free = env.stake_of(&a.pubkey()).await.free;
    env.sync(id, &a.pubkey()).await;
    let released = env.stake_of(&a.pubkey()).await.free - free;
    let (x0, y0) = (
        env.stake_of(&x.pubkey()).await.free,
        env.stake_of(&y.pubkey()).await.free,
    );
    env.lose_source_case(id, &x).await;
    env.lose_source_case(id, &y).await;
    assert_eq!(env.head(id).await.status, HeadStatus::Retired);
    let awarded =
        env.stake_of(&x.pubkey()).await.free - x0 + env.stake_of(&y.pubkey()).await.free - y0;
    let d = env.delegation_of(id, &a.pubkey()).await;
    assert_eq!(released + awarded + d.held, 10_000 * AMBA);
    assert_eq!(awarded, 7_500 * AMBA);
    assert_eq!(d.proposer_slashed, awarded);
}

/// Review #9: escrow kept for a contract is slashable for the cases that open while it is kept
/// for it, exactly as if still delegated, and only those. D backs the source through the
/// listing, undelegates and, after the cooldown, keeps 190 of 1,000 in escrow for the
/// unsettled contract. A case opened after D left falls: D's escrow pays its 10% (100). Once the
/// contract settled, a later case no longer touches what D still holds, and it all comes back.
#[tokio::test]
async fn contract_escrow_answers_for_cases_opened_while_it_is_held() {
    let mut params = test_params();
    params.min_live_sources = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let d = env.user(1_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let staked = env.stake_of(&d.pubkey()).await.free;
    env.delegate(&d, id, staked).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    env.warp(env.now + DAY).await;
    let ix = hx::undelegate(&program(), &d.pubkey(), &env.product, id, staked);
    env.send(ix, &[&d]).await.unwrap();
    env.warp(env.now + env.params.cooldown_secs).await;
    env.sync(id, &d.pubkey()).await;
    let escrow = liability::escrow_required(staked, env.params.slash_rate_bps, 2);
    assert_eq!(env.delegation_of(id, &d.pubkey()).await.held, escrow);
    // A case opened after D left, while its escrow is held for the contract.
    env.lose_source_case(id, &x).await;
    env.sync(id, &d.pubkey()).await;
    let slash = liability::slash(staked, env.params.slash_rate_bps);
    let held = env.delegation_of(id, &d.pubkey()).await;
    assert_eq!((held.slashed_total, held.held), (slash, escrow - slash));
    // The contract settles; a case opened afterwards is not D's.
    env.warp(expiry + DAY).await;
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    env.send(hx::finalize_contract(&program(), &env.product, expiry), &[])
        .await
        .unwrap();
    env.warp(env.now + 3_600).await;
    env.lose_source_case(id, &x).await;
    let free = env.stake_of(&d.pubkey()).await.free;
    env.sync(id, &d.pubkey()).await;
    assert_eq!(env.stake_of(&d.pubkey()).await.free, free + escrow - slash);
}

/// Review #10: undelegating never exposes stake to a case it was never exposed to. A case opens;
/// N delegates a moment later (fresh: that case cannot strike it) and undelegates a moment after
/// that. When the case falls, N's held stake is not slashed for it.
#[tokio::test]
async fn fresh_stake_stays_exempt_once_it_cools() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let n = env.user(100 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let ix = hx::challenge_source(
        &program(),
        &x.pubkey(),
        &x.pubkey(),
        &env.product,
        id,
        hx::ChallengeSourceParams {
            bond: USDC,
            reason: HeadChallengeReason::ValueOrOther,
            evidence_kind: 0,
            evidence_commitment: [0; 32],
            reason_hash: heads_reason_hash(b"fabricated"),
        },
        None,
    )
    .unwrap();
    env.send(ix, &[&x]).await.unwrap();
    env.warp(env.now + 1).await;
    let staked = env.stake_of(&n.pubkey()).await.free;
    env.delegate(&n, id, staked).await;
    assert_eq!(env.delegation_of(id, &n.pubkey()).await.fresh, staked);
    env.warp(env.now + 1).await;
    let ix = hx::undelegate(&program(), &n.pubkey(), &env.product, id, staked);
    env.send(ix, &[&n]).await.unwrap();
    env.warp(env.now + DAY).await;
    env.rule_source(id, &x.pubkey(), true).await;
    let ix = hx::finalize_source_challenges(&program(), &env.product, id, &a.pubkey());
    env.send(ix, &[]).await.unwrap();
    assert_eq!(env.head(id).await.strike_count, 1);
    env.sync(id, &n.pubkey()).await;
    let d = env.delegation_of(id, &n.pubkey()).await;
    assert_eq!((d.slashed_total, d.held), (0, staked));
}

/// Review #8: the proposer's delegation is the proposer's however it was made. A proposer exits
/// before any exposure, lets the cooldown pass and the delegation close, then delegates again:
/// that delegation is still the proposer's, so a falling source case takes it for the winners.
#[tokio::test]
async fn a_recreated_proposer_delegation_is_still_the_proposers() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let ix = hx::undelegate(&program(), &a.pubkey(), &env.product, id, 1_000 * AMBA);
    env.send(ix, &[&a]).await.unwrap();
    env.warp(env.now + env.params.cooldown_secs).await;
    env.sync(id, &a.pubkey()).await;
    let key = find_head_delegation_address(&program(), &env.head_key(id), &a.pubkey()).0;
    assert!(env.h.account(key).await.is_none(), "closed");
    env.delegate(&a, id, 1_000 * AMBA).await;
    assert!(env.delegation_of(id, &a.pubkey()).await.is_proposer);
    let x0 = env.stake_of(&x.pubkey()).await.free;
    env.lose_source_case(id, &x).await;
    assert_eq!(env.delegation_of(id, &a.pubkey()).await.active, 0);
    assert_eq!(env.stake_of(&x.pubkey()).await.free, x0 + 1_000 * AMBA);
}

impl Env {
    /// A key with SOL and `amba` AMBA staked into the family, and no `UserCollateral`.
    async fn staker_without_ledger(&mut self, amba: u64) -> Keypair {
        let key = Keypair::new();
        self.h.fund(&key.pubkey(), 5_000_000_000).await;
        let ata = self.amba_account(&key.pubkey(), amba).await;
        let ix = hx::stake(
            &program(),
            &key.pubkey(),
            &key.pubkey(),
            &ata,
            &self.mint,
            &t22(),
            amba,
        );
        self.send(ix, &[&key]).await.unwrap();
        key
    }

    async fn source_challenge(&mut self, id: u32, x: &Keypair) {
        let ix = hx::challenge_source(
            &program(),
            &x.pubkey(),
            &x.pubkey(),
            &self.product,
            id,
            hx::ChallengeSourceParams {
                bond: USDC,
                reason: HeadChallengeReason::ValueOrOther,
                evidence_kind: 0,
                evidence_commitment: [0; 32],
                reason_hash: heads_reason_hash(b"fabricated"),
            },
            None,
        )
        .unwrap();
        self.send(ix, &[x]).await.unwrap();
    }
}

/// Review #7: nobody blocks a source case by lacking a payout account. A proposer who never made
/// a `UserCollateral` gets one when posting, so a standing case pays them; a challenger with no
/// AMBA stake gets a `HeadsStake` when challenging, so a falling case can pay them.
#[tokio::test]
async fn payout_ledgers_exist_before_a_case_needs_them() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.staker_without_ledger(20_000 * AMBA).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let z = env.user(0, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    // A standing case: the challenger's bond goes to the proposer.
    env.source_challenge(id, &z).await;
    env.warp(env.now + DAY).await;
    env.rule_source(id, &z.pubkey(), false).await;
    let ix = hx::finalize_source_challenges(&program(), &env.product, id, &a.pubkey());
    env.send(ix, &[]).await.unwrap();
    assert_eq!(env.usdc(&a.pubkey()).await, USDC);
    let ix = hx::settle_source_challenge(&program(), &env.product, id, &z.pubkey(), &z.pubkey());
    env.send(ix, &[]).await.unwrap();
    // A falling case: the proposer's delegation goes to the challenger's stake.
    env.source_challenge(id, &z).await;
    env.warp(env.now + DAY).await;
    env.rule_source(id, &z.pubkey(), true).await;
    let ix = hx::finalize_source_challenges(&program(), &env.product, id, &a.pubkey());
    env.send(ix, &[]).await.unwrap();
    let ix = hx::settle_source_challenge(&program(), &env.product, id, &z.pubkey(), &z.pubkey());
    env.send(ix, &[]).await.unwrap();
    assert_eq!(env.stake_of(&z.pubkey()).await.free, 1_000 * AMBA);
}

/// Review #11: a full live-source list never locks a fill's bond. With the board's list full, a
/// fill of an Active head that stands cannot make it Live: it stands for its bond and the
/// challengers' (no print, no prize), the head stays Active for a later fill, and nothing waits.
#[tokio::test]
async fn a_fill_on_a_full_board_stands_for_its_bond() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    // The list is full (installed: 96 sources would take hundreds of transactions to make).
    let mut account = env.h.account(env.board).await.unwrap();
    let mut board = HeadsBoard::unpack(&account.data).unwrap();
    let full = board.live_head_ids.len();
    for i in 0..full {
        board.live_head_ids[i] = 10_000 + i as u32;
        board.live_since_ts[i] = env.now;
    }
    board.live_count = full as u16;
    board.pack(&mut account.data).unwrap();
    let key = env.board;
    env.h
        .ctx
        .set_account(&key, &AccountSharedData::from(account));
    let usdc = env.usdc(&f.pubkey()).await;
    let points = env.prize_points().await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let claim = env.claim(p).await;
    assert_eq!(claim.status, HeadClaimStatus::Stood);
    let head = env.head(id).await;
    assert_eq!(
        (head.status, head.open_fill, head.print_count),
        (HeadStatus::Active, false, 0)
    );
    assert_eq!(env.usdc(&f.pubkey()).await, usdc);
    assert_eq!(env.prize_points().await, points);
}

/// Review #12: a month's unclaimable prize money rolls over, once, and every prize earned stays
/// payable in full. October is funded 100 USDC (20 to prizes) and earns one update prize (1
/// USDC): after October, the 19 nobody can claim move to November's pool; the 1 is still paid.
#[tokio::test]
async fn unclaimable_prize_money_rolls_over_once() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 200 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let october = time::month_id(env.now);
    let ix = hx::fund_board(
        &program(),
        &f.pubkey(),
        &f.pubkey(),
        &env.product,
        october,
        100 * USDC,
    );
    env.send(ix, &[&f]).await.unwrap();
    let epoch: Box<HeadsEpoch> = env
        .read(find_heads_epoch_address(&program(), &env.board, october).0)
        .await;
    let (pool, points) = (epoch.prize_pool, epoch.prize_points);
    assert_eq!(pool, 20 * USDC);
    assert!(points < pool);
    // Not before October ends.
    let roll = hx::roll_over_prizes(&program(), &f.pubkey(), &env.product, october, october + 1);
    assert_oracle_error(env.send(roll, &[&f]).await, OracleError::InvalidHeadsState);
    env.warp(time::month_start(october + 1) + DAY).await;
    let november = time::month_id(env.now);
    let ix = hx::roll_over_prizes(&program(), &f.pubkey(), &env.product, october, november);
    env.send(ix, &[&f]).await.unwrap();
    let old: Box<HeadsEpoch> = env
        .read(find_heads_epoch_address(&program(), &env.board, october).0)
        .await;
    let new: Box<HeadsEpoch> = env
        .read(find_heads_epoch_address(&program(), &env.board, november).0)
        .await;
    assert_eq!((old.prize_pool, old.rolled_out), (points, pool - points));
    assert_eq!(
        (new.prize_pool, new.rolled_in),
        (pool - points, pool - points)
    );
    // Once only.
    let ix = hx::roll_over_prizes(&program(), &f.pubkey(), &env.product, october, november);
    assert_oracle_error(env.send(ix, &[&f]).await, OracleError::InvalidHeadsState);
    // October's prize is still paid in full.
    let before = env.usdc(&f.pubkey()).await;
    let ix = hx::claim_prize(&program(), &f.pubkey(), &env.product, id, p.seq, october);
    env.send(ix, &[&f]).await.unwrap();
    assert_eq!(env.usdc(&f.pubkey()).await, before + points);
}

/// Review #13: an unruled challenge cannot hold a contract's settlement. An update committed just
/// before the expiry is challenged and never ruled; settlement opens on time and leaves the
/// unresolved claim out (it still resolves later, for its bonds), and a claim committed after
/// the expiry finalizes before it (no queue, round 3 R1); the later observation stays the head's.
#[tokio::test]
async fn an_unruled_challenge_cannot_hold_settlement() {
    let mut params = test_params();
    params.min_live_sources = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let z = env.user(0, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    let freeze = env.contract(expiry).await.freeze_ts;
    env.warp(freeze + DAY).await;
    let p = env.update(&f, id, 1_100_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    env.warp(expiry - 60).await;
    let held = env.update(&f, id, 2_000_000, env.now, origin("one")).await;
    env.challenge(&z, held, USDC).await.unwrap();
    env.warp(expiry + 60).await;
    let after = env.update(&f, id, 1_050_000, env.now, origin("one")).await;
    env.warp(expiry + env.params.challenge_window_secs + 3_600)
        .await;
    // The claim committed after the expiry finalizes at once; the contract admits neither, so
    // settlement waits for neither.
    env.finalize(after, &f.pubkey()).await;
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    let contract = env.contract(expiry).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_LIVE, 0);
    assert_eq!(contract.component_change_ppm[1], 100_000);
    // It resolves later, for its bonds; its observation is older than the head's print.
    env.rule(held, &z.pubkey(), false).await;
    env.finalize(held, &f.pubkey()).await;
    assert_eq!(env.claim(held).await.status, HeadClaimStatus::Stood);
    assert_eq!(env.head(id).await.value, 1_050_000);
}

/// Review #14 (not a bug): `max_strikes` above the head's strike capacity is refused by both
/// `InitBoard` and `SetBoardParams` (`HeadsParams::validate`), and the capacity itself is
/// accepted.
#[tokio::test]
async fn max_strikes_beyond_the_strike_capacity_are_refused() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let admin = env.h.admin.insecure_clone();
    let payer = env.h.payer();
    let capacity = ameba_oracle::state::heads::HEAD_STRIKE_CAPACITY as u8;
    let mut too_many = test_params();
    too_many.max_strikes = capacity + 1;
    let item = hx::HeadsItemParams {
        bucket_id: product("OTHER-ITEM"),
        basket_weight_bps: 10_000,
        mix_bps: [2_000, 5_000, 3_000],
        mix_basis: HeadsMixBasis::Assumed,
    };
    // The decoder or the handler refuses it (`instruction_error` panics on success).
    let refused = |result: Result<(), BanksClientError>| {
        instruction_error(result);
    };
    let init = |params| {
        hx::init_board(
            &program(),
            &payer.pubkey(),
            &admin.pubkey(),
            product("OTHERX"),
            params,
            vec![item.clone()],
        )
    };
    if let Ok(ix) = init(too_many) {
        refused(env.send(ix, &[&admin]).await);
    }
    assert!(env
        .h
        .account(find_heads_board_address(&program(), &product("OTHERX")).0)
        .await
        .is_none());
    let version = env.board().await.params_version;
    let ix = hx::set_board_params(&program(), &admin.pubkey(), &env.product, version, too_many);
    refused(env.send(ix, &[&admin]).await);
    assert_eq!(
        env.board().await.params.max_strikes,
        test_params().max_strikes
    );
    // The capacity itself is a valid value.
    let mut most = test_params();
    most.max_strikes = capacity;
    let ix = hx::set_board_params(&program(), &admin.pubkey(), &env.product, version, most);
    env.send(ix, &[&admin]).await.unwrap();
    assert_eq!(env.board().await.params.max_strikes, capacity);
    env.send(init(most).unwrap(), &[&admin]).await.unwrap();
}

// The 2,688-byte Head layout of before 2026-10-30, grown in place (selector 30)
// ------------------------------------------------------------------------------------------

const OLD_HEAD_LEN: usize = 2_688;

impl Env {
    /// Rewrites head `id` as the old layout held it: none of the new fields, no `CAPTURE_JOINED`,
    /// and in the 24 bytes the samples took over, an old expiry capture of slot `slot` (claim seq
    /// `seq`, `open` open; old flag bit 4), 2,688 bytes; its open claims lose their new fields.
    async fn to_old_layout(&mut self, id: u32, slot: usize, seq: u32, open: u16) {
        let key = self.head_key(id);
        let mut account = self.h.account(key).await.unwrap();
        let mut head = Head::unpack(&account.data).unwrap();
        // No source case on these heads: the old program's fields past its wire are all new.
        assert_eq!((head.source_case, head.strike_count), (0, 0));
        for flags in head.capture_flags.iter_mut() {
            *flags &= !ameba_oracle::state::heads::CAPTURE_JOINED;
        }
        // `capture_expiry_seq` and `capture_expiry_open` sat where these are now.
        let mut seqs = [0u32; 4];
        seqs[slot] = seq;
        head.reveals = seqs[0];
        head.finalized_reveals = seqs[1];
        head.last_prize_day = seqs[2];
        head.samples_reserved = seqs[3].to_le_bytes();
        head.adm_open = [0; 4];
        head.adm_open[slot] = open;
        head.restart_reveals = [0; 4];
        head.capture_flags[slot] |= 4;
        head.pack(&mut account.data).unwrap();
        // The new fields sit past the old wire, in what was its zero reserved tail.
        account.data[2_575..].fill(0);
        account.data.truncate(OLD_HEAD_LEN);
        let rent = self.h.ctx.banks_client.get_rent().await.unwrap();
        account.lamports = rent.minimum_balance(OLD_HEAD_LEN);
        self.h
            .ctx
            .set_account(&key, &AccountSharedData::from(account));
    }

    fn migrate_ix(&self, id: u32, contracts: &[Pubkey], claims: &[Pubkey]) -> Instruction {
        hx::migrate_head(
            &program(),
            &self.h.ctx.payer.pubkey(),
            &self.product,
            id,
            contracts,
            claims,
        )
        .unwrap()
    }
}

/// Round 8 size plan: dev ran the round-7 migration, so selector 30 no longer grows the layouts
/// it left behind. A 2,688-byte Head, a version-1 Head and a version-2 Delegation are refused
/// (`InvalidHeadsAccount`).
#[tokio::test]
async fn the_retired_migration_paths_refuse_their_layouts() {
    let (mut env, _, id, _) = one_listed_source(one_source_params()).await;
    let d = env.user(2_000 * AMBA, 0).await;
    env.delegate(&d, id, 1_000 * AMBA).await;
    let key = env.head_key(id);
    let current = env.h.account(key).await.unwrap();
    let mut v1 = current.clone();
    v1.data[8] = 1;
    env.h.ctx.set_account(&key, &AccountSharedData::from(v1));
    let ix = env.migrate_ix(id, &[], &[]);
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsAccount);
    env.h
        .ctx
        .set_account(&key, &AccountSharedData::from(current));
    env.to_old_layout(id, 0, 0, 0).await;
    env.h.advance_slot().await;
    let ix = env.migrate_ix(id, &[], &[]);
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsAccount);
    let delegation = find_head_delegation_address(&program(), &key, &d.pubkey()).0;
    let mut account = env.h.account(delegation).await.unwrap();
    account.data[8] = 2;
    account.data.truncate(896);
    env.h
        .ctx
        .set_account(&delegation, &AccountSharedData::from(account));
    let mut ix = env.migrate_ix(id, &[], &[]);
    ix.accounts[2].pubkey = delegation;
    ix.accounts
        .push(solana_sdk::instruction::AccountMeta::new_readonly(
            key, false,
        ));
    env.h.advance_slot().await;
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsAccount);
}

impl Env {
    /// Every component of the contract at `expiry`, then the contract itself.
    async fn settle_contract(&mut self, expiry: u64) {
        for component in 0..3u8 {
            self.settle(expiry, component, &[]).await;
        }
        let ix = hx::finalize_contract(&program(), &self.product, expiry);
        self.send(ix, &[]).await.unwrap();
    }

    fn source_challenge_ix(&self, id: u32, x: &Keypair) -> Instruction {
        hx::challenge_source(
            &program(),
            &x.pubkey(),
            &x.pubkey(),
            &self.product,
            id,
            hx::ChallengeSourceParams {
                bond: USDC,
                reason: HeadChallengeReason::ValueOrOther,
                evidence_kind: 0,
                evidence_commitment: [0; 32],
                reason_hash: heads_reason_hash(b"fabricated"),
            },
            None,
        )
        .unwrap()
    }

    /// Rules the head's open case for `x` and finalizes it (the strike), paying `x`.
    async fn uphold_source_case(&mut self, id: u32, x: &Keypair) {
        let proposer = self.head(id).await.proposer;
        let end = self.head(id).await.source_case_window_end_ts;
        if self.now < end {
            self.warp(end).await;
        }
        self.rule_source(id, &x.pubkey(), true).await;
        let ix = hx::finalize_source_challenges(&program(), &self.product, id, &proposer);
        self.send(ix, &[]).await.unwrap();
        let ix =
            hx::settle_source_challenge(&program(), &self.product, id, &x.pubkey(), &x.pubkey());
        self.send(ix, &[]).await.unwrap();
    }

    /// A one-source board, D's 1,000 AMBA behind it through C1's listing, then out: after the
    /// cooldown D keeps C1's escrow (190 of every 1,000). Returns (head, D, D's stake, C1).
    async fn escrow_for_c1(
        &mut self,
        a: &Keypair,
        b: &Keypair,
        f: &Keypair,
    ) -> (u32, Keypair, u64, u64) {
        let d = self.user(1_000 * AMBA, 0).await;
        let id = self.active_head(a, b, 1).await;
        let staked = self.stake_of(&d.pubkey()).await.free;
        self.delegate(&d, id, staked).await;
        let p = self.fill(f, id, 1_000_000, self.now, origin("one")).await;
        self.warp(self.now + DAY).await;
        self.finalize(p, &f.pubkey()).await;
        let c1 = DEC_01_2026;
        self.create_period(self.now + DAY, c1).await;
        self.warp(self.now + DAY).await;
        self.list(c1, 50 * USDC).await;
        self.warp(self.now + DAY).await;
        let ix = hx::undelegate(&program(), &d.pubkey(), &self.product, id, staked);
        self.send(ix, &[&d]).await.unwrap();
        self.warp(self.now + self.params.cooldown_secs).await;
        self.sync(id, &d.pubkey()).await;
        let escrow = liability::escrow_required(staked, self.params.slash_rate_bps, 2);
        assert_eq!(self.delegation_of(id, &d.pubkey()).await.held, escrow);
        (id, d, staked, c1)
    }
}

/// Re-review N5: escrow kept for a contract answers for a case opened while it was unsettled,
/// however many ring generations pass before the owner syncs. The case opens while C1 is
/// unsettled and is upheld; D does not sync; C1 settles, C2 reuses its ring slot and settles too;
/// only then does D sync, and pays the strike.
#[tokio::test]
async fn escrow_answers_across_ring_generations() {
    let mut params = test_params();
    params.min_live_sources = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let (id, d, staked, c1) = env.escrow_for_c1(&a, &b, &f).await;
    let ix = env.source_challenge_ix(id, &x);
    env.send(ix, &[&x]).await.unwrap();
    env.uphold_source_case(id, &x).await;
    // The lost case took the proposer's delegation for the winners: a new delegator backs the
    // source again, so it is an origin for C2 (round 6 review, finding 1).
    let c = env.user(2_000 * AMBA, 0).await;
    env.delegate(&c, id, 1_000 * AMBA).await;
    env.warp(c1 + DAY).await;
    env.settle_contract(c1).await;
    let c2 = FEB_01_2027;
    env.create_period(env.now + DAY, c2).await;
    env.warp(env.now + DAY).await;
    env.list(c2, 50 * USDC).await;
    assert_eq!(
        env.contract(c2).await.ring_slot,
        env.contract(c1).await.ring_slot
    );
    env.warp(c2 + DAY).await;
    env.settle_contract(c2).await;
    // D syncs at last: it pays the strike, and the rest of its escrow comes back.
    let free = env.stake_of(&d.pubkey()).await.free;
    env.sync(id, &d.pubkey()).await;
    let escrow = liability::escrow_required(staked, env.params.slash_rate_bps, 2);
    let slash = liability::slash(staked, env.params.slash_rate_bps);
    assert_eq!(env.stake_of(&d.pubkey()).await.free, free + escrow - slash);
}

/// Re-review N5, the short variant: a case opens and C1 settles at the same timestamp, the case
/// first. D's escrow stays held for the unresolved case, and pays when it is upheld.
#[tokio::test]
async fn a_case_opened_before_the_settlement_at_one_timestamp_holds_the_escrow() {
    let mut params = test_params();
    params.min_live_sources = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let (id, d, staked, c1) = env.escrow_for_c1(&a, &b, &f).await;
    env.warp(c1 + DAY).await;
    for component in 0..3u8 {
        env.settle(c1, component, &[]).await;
    }
    // One transaction, one timestamp: the case opens, then C1 settles.
    let open = env.source_challenge_ix(id, &x);
    let finalize = hx::finalize_contract(&program(), &env.product, c1);
    env.h
        .send_many(
            vec![
                ComputeBudgetInstruction::request_heap_frame(256 * 1024),
                ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
                open,
                finalize,
            ],
            &[&x],
        )
        .await
        .unwrap();
    let escrow = liability::escrow_required(staked, env.params.slash_rate_bps, 2);
    env.sync(id, &d.pubkey()).await;
    assert_eq!(env.delegation_of(id, &d.pubkey()).await.held, escrow);
    env.uphold_source_case(id, &x).await;
    let free = env.stake_of(&d.pubkey()).await.free;
    env.sync(id, &d.pubkey()).await;
    let slash = liability::slash(staked, env.params.slash_rate_bps);
    assert_eq!(env.stake_of(&d.pubkey()).await.free, free + escrow - slash);
}

/// Re-review N6: a later ring generation never makes old escrow answer for an unrelated case. C1
/// settles; then a case opens; then C2 reuses the slot (D contributes nothing to it). D's escrow
/// is free of the case: it all comes back, and the upheld case burns none of it.
#[tokio::test]
async fn a_later_ring_generation_exposes_no_old_escrow() {
    let mut params = test_params();
    params.min_live_sources = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let (id, d, staked, c1) = env.escrow_for_c1(&a, &b, &f).await;
    env.warp(c1 + DAY).await;
    env.settle_contract(c1).await;
    env.warp(env.now + 3_600).await;
    let ix = env.source_challenge_ix(id, &x);
    env.send(ix, &[&x]).await.unwrap();
    let c2 = FEB_01_2027;
    env.create_period(env.now + DAY, c2).await;
    env.warp(env.now + 3_600).await;
    env.list(c2, 50 * USDC).await;
    let free = env.stake_of(&d.pubkey()).await.free;
    env.sync(id, &d.pubkey()).await;
    let escrow = liability::escrow_required(staked, env.params.slash_rate_bps, 2);
    assert_eq!(env.stake_of(&d.pubkey()).await.free, free + escrow);
    env.uphold_source_case(id, &x).await;
    let ledger = find_head_delegation_address(&program(), &env.head_key(id), &d.pubkey()).0;
    assert!(
        env.h.account(ledger).await.is_none() || {
            env.sync(id, &d.pubkey()).await;
            env.delegation_of(id, &d.pubkey()).await.slashed_total == 0
        }
    );
}

/// Re-review N7: in one transaction at one timestamp a case opens, N delegates, N undelegates.
/// N entered after the case opened, so the upheld case slashes none of N's stake.
#[tokio::test]
async fn fresh_stake_in_the_cases_own_timestamp_stays_exempt() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let n = env.user(100 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let staked = env.stake_of(&n.pubkey()).await.free;
    let open = env.source_challenge_ix(id, &x);
    let delegate = hx::delegate(
        &program(),
        &n.pubkey(),
        &n.pubkey(),
        &env.product,
        id,
        staked,
    );
    let undelegate = hx::undelegate(&program(), &n.pubkey(), &env.product, id, staked);
    env.h
        .send_many(
            vec![
                ComputeBudgetInstruction::request_heap_frame(256 * 1024),
                ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
                open,
                delegate,
                undelegate,
            ],
            &[&x, &n],
        )
        .await
        .unwrap();
    env.uphold_source_case(id, &x).await;
    env.sync(id, &n.pubkey()).await;
    let d = env.delegation_of(id, &n.pubkey()).await;
    assert_eq!((d.slashed_total, d.held), (0, staked));
}

/// Re-review N4: one rule for eligibility and waiting, whatever the order. A source with an old
/// level of 1.00, a ready pre-expiry update to 2.00, and an upheld source case whose strike
/// retires it after the cutoff K. Retiring first must not let the settlement skip the update:
/// it waits for it, and the update still prints for the contract it is eligible in.
#[tokio::test]
async fn retirement_after_the_cutoff_neither_skips_nor_drops_a_ready_update() {
    let mut params = test_params();
    params.min_live_sources = 1;
    params.max_strikes = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    // At the settlement window's start: every settlement sample takes it.
    let freeze = env.contract(expiry).await.freeze_ts;
    env.warp(freeze + 60).await;
    let ready = env.update(&f, id, 2_000_000, env.now, origin("one")).await;
    let ix = env.source_challenge_ix(id, &x);
    env.send(ix, &[&x]).await.unwrap();
    env.rule_source(id, &x.pubkey(), true).await;
    let cutoff = expiry + env.params.challenge_window_secs;
    // At K exactly nothing records: the interval is closed first.
    env.warp(cutoff).await;
    let ix = env.settle_ix(expiry, 1, 0, &[id]);
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    // After K: the case retires the source first, then settlement must wait for the update.
    env.warp(cutoff + 1).await;
    let ix = hx::finalize_source_challenges(&program(), &env.product, id, &a.pubkey());
    env.send(ix, &[]).await.unwrap();
    assert_eq!(env.head(id).await.status, HeadStatus::Retired);
    let ix = env.settle_ix(expiry, 1, 0, &[id]);
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    env.finalize(ready, &f.pubkey()).await;
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    let contract = env.contract(expiry).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_LIVE, 0);
    assert_eq!(contract.component_change_ppm[1], 1_000_000);
}

/// Re-review #13 remainder: a claim or source case nobody rules resolves at its ruling deadline
/// (its first window's end plus the board's ruling deadline) on no ruling: every challenge times
/// out (its bond back), the claim stands for its bond but prints nothing, the case stands with no
/// strike, and the escrow it held is free.
#[tokio::test]
async fn an_unruled_case_is_void_after_the_ruling_deadline() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let z = env.user(AMBA, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let deadline = env.board().await.ruling_deadline_secs;
    let deadline = if deadline == 0 {
        ameba_oracle::state::heads::DEFAULT_RULING_DEADLINE_SECS
    } else {
        deadline
    };
    // A claim challenged and never ruled.
    let (f0, z0) = (env.usdc(&f.pubkey()).await, env.usdc(&z.pubkey()).await);
    let held = env.update(&f, id, 2_000_000, env.now, origin("one")).await;
    env.challenge(&z, held, USDC).await.unwrap();
    let end = env.claim(held).await.window_end_ts;
    env.warp(end + deadline - 1).await;
    let ix = env.finalize_ix(held, &f.pubkey());
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    env.warp(end + deadline).await;
    env.finalize(held, &f.pubkey()).await;
    env.settle_claim_challenge(held, &z.pubkey()).await.unwrap();
    assert_eq!(
        (env.usdc(&f.pubkey()).await, env.usdc(&z.pubkey()).await),
        (f0, z0)
    );
    assert_eq!(env.head(id).await.value, 1_000_000);
    // A source case nobody rules.
    let z1 = env.usdc(&z.pubkey()).await;
    let ix = env.source_challenge_ix(id, &z);
    env.send(ix, &[&z]).await.unwrap();
    let end = env.head(id).await.source_case_window_end_ts;
    env.warp(end + deadline).await;
    let ix = hx::finalize_source_challenges(&program(), &env.product, id, &a.pubkey());
    env.send(ix, &[]).await.unwrap();
    let head = env.head(id).await;
    assert_eq!((head.strike_count, head.unresolved_cases), (0, 0));
    let ix = hx::settle_source_challenge(&program(), &env.product, id, &z.pubkey(), &z.pubkey());
    env.send(ix, &[]).await.unwrap();
    assert_eq!(env.usdc(&z.pubkey()).await, z1);
}

// Re-review 2026-10-30: a level is the median of fixed samples
// ------------------------------------------------------------------------------------------

impl Env {
    /// Contract `expiry`'s settlement-window sample instants: five, evenly spaced from the
    /// freeze, the last at the expiry.
    async fn settlement_instants(&mut self, expiry: u64) -> [u64; 5] {
        let k = self.contract(expiry).await;
        let (freeze, end) = (k.freeze_ts, k.expiry_ts);
        core::array::from_fn(|j| freeze + (end - freeze) * (j as u64 + 1) / 5)
    }
}

/// One listed source of component 1 opening at 1.00 (`origin("one")`) on a board that settles
/// on one source: `(env, fetcher, its id, the contract's expiry)`.
async fn one_listed_source(params: HeadsParams) -> (Env, Keypair, u32, u64) {
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 1_000 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    (env, f, id, expiry)
}

fn one_source_params() -> HeadsParams {
    let mut params = test_params();
    params.min_live_sources = 1;
    params.max_strikes = 1;
    params
}

/// Re-review N1, the reviewer's sequence: three distinct observations inside the settlement
/// window, 1.00 at t1, 2.00 at t2, 3.00 at t3, all revealed before any finalizes, and finalized
/// t3 first, then t2, then t1. Each print decides the samples at and after its own source time
/// (round 3 R1: no queue), so the level is the median of (1.00, 2.00, 2.00, 3.00, 3.00), 2.00:
/// +100%, as in source-time order.
#[tokio::test]
async fn finalization_order_cannot_choose_the_level() {
    let (mut env, f, id, expiry) = one_listed_source(one_source_params()).await;
    let at = env.settlement_instants(expiry).await;
    let freeze = env.contract(expiry).await.freeze_ts;
    let mut claims = Vec::new();
    for (ts, value) in [
        (freeze + 60, 1_000_000),
        (at[0] + 60, 2_000_000),
        (at[2] + 60, 3_000_000),
    ] {
        env.warp(ts).await;
        claims.push(env.update(&f, id, value, ts, origin("one")).await);
    }
    env.warp(env.now + DAY + 60).await;
    for p in claims.into_iter().rev() {
        env.finalize(p, &f.pubkey()).await;
    }
    assert_eq!(
        env.head(id).await.value,
        3_000_000,
        "the latest observation"
    );
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    let contract = env.contract(expiry).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_LIVE, 0);
    assert_eq!(contract.component_change_ppm[1], 1_000_000);
}

/// Re-review N2, the reviewer's sequence: an old print of 1.00 and a pre-expiry claim of 2.00
/// in the window whose challenge was ruled Lost and whose window ended, not finalized at K+1.
/// SettleComponent cannot settle at 1.00 first: it waits for the claim (the contract admits
/// it); finalized, the contract settles at 2.00 (+100%).
#[tokio::test]
async fn a_claim_ruled_by_the_cutoff_is_waited_for() {
    let (mut env, f, id, expiry) = one_listed_source(one_source_params()).await;
    let z = env.user(0, 100 * USDC).await;
    let freeze = env.contract(expiry).await.freeze_ts;
    env.warp(freeze + 60).await;
    let p = env.update(&f, id, 2_000_000, env.now, origin("one")).await;
    env.challenge(&z, p, USDC).await.unwrap();
    env.rule(p, &z.pubkey(), false).await;
    env.warp(expiry + env.params.challenge_window_secs + 1)
        .await;
    let ix = env.settle_ix(expiry, 1, 0, &[id]);
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    env.finalize(p, &f.pubkey()).await;
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    assert_eq!(
        env.contract(expiry).await.component_change_ppm[1],
        1_000_000
    );
}

/// Re-review N2, the other side: a claim still unruled at the cutoff K is never the contract's.
/// Ruled Lost after K and finalized before anyone records the source, it prints, but the
/// contract settles on what it admitted by K: 1.00, 0%.
#[tokio::test]
async fn a_claim_ruled_after_the_cutoff_never_counts() {
    let (mut env, f, id, expiry) = one_listed_source(one_source_params()).await;
    let z = env.user(0, 100 * USDC).await;
    let freeze = env.contract(expiry).await.freeze_ts;
    env.warp(freeze + 60).await;
    let p = env.update(&f, id, 2_000_000, env.now, origin("one")).await;
    env.challenge(&z, p, USDC).await.unwrap();
    env.warp(expiry + env.params.challenge_window_secs + 1)
        .await;
    env.rule(p, &z.pubkey(), false).await;
    env.finalize(p, &f.pubkey()).await;
    assert_eq!(env.head(id).await.value, 2_000_000);
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    let contract = env.contract(expiry).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_LIVE, 0);
    assert_eq!(contract.component_change_ppm[1], 0);
}

/// Re-review limit (a), the reviewer's sequence: observations 1.00, 2.00, 3.00 in the window,
/// then the unchanged page showing 3.00 captured 17 more times. The level stays the samples'
/// median, 2.00 (+100%), and the 17 captures, made the same day as the 3.00, earn no prize
/// points: one update prize per head per day.
#[tokio::test]
async fn repeat_captures_move_neither_the_level_nor_the_prizes() {
    let (mut env, f, id, expiry) = one_listed_source(one_source_params()).await;
    let at = env.settlement_instants(expiry).await;
    let freeze = env.contract(expiry).await.freeze_ts;
    let mut claims = Vec::new();
    for (ts, value) in [
        (freeze + 60, 1_000_000),
        (at[0] + 60, 2_000_000),
        (at[2] + 60, 3_000_000),
    ] {
        env.warp(ts).await;
        claims.push(env.update(&f, id, value, ts, origin("one")).await);
    }
    for _ in 0..17 {
        env.warp(env.now + 60).await;
        claims.push(env.update(&f, id, 3_000_000, env.now, origin("one")).await);
    }
    assert_eq!(
        env.now / DAY,
        (at[2] + 60) / DAY,
        "the captures share the 3.00's day"
    );
    env.warp(env.now + DAY + 60).await;
    let mut points = Vec::new();
    for p in claims {
        env.finalize(p, &f.pubkey()).await;
        points.push(env.claim(p).await.prize_points);
    }
    let prize = rewards::update_prize(&env.params.to_params(), env.head(id).await.delegation);
    assert_eq!(points[..3], [prize; 3]);
    assert_eq!(points[3..], [0; 17]);
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    assert_eq!(
        env.contract(expiry).await.component_change_ppm[1],
        1_000_000
    );
}

/// Re-review limit (b), the reviewer's sequence: three sources open at 1.00 and print 1.20 at the
/// freeze window's start (valid freeze levels of +20%); after the freeze two of them print 33
/// times each before the expiry, past the 32-print ring, and a source case retires the third,
/// so the component settles on its fallback. The freeze levels are fixed samples, not ring
/// entries: the fallback is still +20%.
#[tokio::test]
async fn freeze_levels_outlive_the_print_ring() {
    let mut params = test_params();
    params.max_strikes = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(1_000 * AMBA, 1_000 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let mut ids = Vec::new();
    let mut fills = Vec::new();
    for j in 0..3 {
        let id = env.active_head(&a, &b, 1).await;
        ids.push(id);
        fills.push(
            env.fill(&f, id, 1_000_000, env.now, origin(&format!("o{j}")))
                .await,
        );
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    let window = env.freeze_window(expiry).await;
    let freeze = env.contract(expiry).await.freeze_ts;
    env.warp(window + 60).await;
    let mut claims = Vec::new();
    for &id in &ids {
        let o = env.head(id).await.origin_id;
        claims.push(env.update(&f, id, 1_200_000, env.now, o).await);
    }
    env.warp(env.now + DAY + 60).await;
    for p in claims {
        env.finalize(p, &f.pubkey()).await;
    }
    env.warp(freeze + 3_600).await;
    env.lose_source_case(ids[2], &x).await;
    assert_eq!(env.head(ids[2]).await.status, HeadStatus::Retired);
    let mut late = Vec::new();
    for _ in 0..33 {
        env.warp(env.now + 600).await;
        for &id in &ids[..2] {
            let o = env.head(id).await.origin_id;
            late.push(env.update(&f, id, 2_000_000, env.now, o).await);
        }
    }
    env.warp(env.now + DAY + 60).await;
    for p in late {
        env.finalize(p, &f.pubkey()).await;
    }
    assert!(env.head(ids[0]).await.print_count as usize > HEAD_PRINT_CAPACITY + 1);
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    let contract = env.contract(expiry).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_LAST_VALID, 0);
    assert_eq!(contract.component_degraded[1], DEGRADED_TOO_FEW_ORIGINS);
    assert_eq!(contract.component_freeze_change_ppm[1], 200_000);
    assert_eq!(contract.component_change_ppm[1], 200_000);
}

/// Early review of round 2, B1, the reviewer's sequence: the fallback's freeze level is the same
/// whoever calls what when. A, the only source (`min_live_sources` 1), printed 1.00 before the
/// freeze window; an update to 2.00 dated inside that window was committed and revealed before
/// the expiry, unchallenged, and is not finalized; a source case retires A after the freeze and
/// before K. At K+1 SettleComponent waits for the update (A was live at the freeze, so the
/// fallback reads A's freeze level); finalized, it prints, and the fallback settles at +100%.
#[tokio::test]
async fn the_fallback_waits_for_a_retired_sources_admitted_claim() {
    let (mut env, f, id, expiry) = one_listed_source(one_source_params()).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let window = env.freeze_window(expiry).await;
    let freeze = env.contract(expiry).await.freeze_ts;
    env.warp(freeze + 60).await;
    let p = env
        .update(&f, id, 2_000_000, window + 60, origin("one"))
        .await;
    env.warp(freeze + 3_600).await;
    env.lose_source_case(id, &x).await;
    assert_eq!(env.head(id).await.status, HeadStatus::Retired);
    env.warp(expiry + env.params.challenge_window_secs + 1)
        .await;
    let ix = env.settle_ix(expiry, 1, 0, &[id]);
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    env.finalize(p, &f.pubkey()).await;
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    let contract = env.contract(expiry).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_LAST_VALID, 0);
    assert_eq!(contract.component_freeze_change_ppm[1], 1_000_000);
    assert_eq!(contract.component_change_ppm[1], 1_000_000);
}

// Early review of round 2 (2026-10-30): unruled challenges time out alone; fixed deadlines
// ------------------------------------------------------------------------------------------

impl Env {
    /// A ruling on head `id`'s source challenge by `challenger`: its result.
    async fn try_rule_source(
        &mut self,
        id: u32,
        challenger: &Pubkey,
        won: bool,
    ) -> Result<(), BanksClientError> {
        let epoch = self.epoch().await;
        let (keys, signers) = self.seats3([1, 2, 3]);
        let ix = hx::rule_source_challenge(
            &program(),
            &self.council,
            &keys,
            &self.product,
            id,
            challenger,
            won,
            epoch,
        );
        let refs: Vec<&Keypair> = signers.iter().collect();
        self.send(ix, &refs).await
    }

    /// A ruling on claim `p`'s challenge by `challenger`: its result.
    async fn try_rule(
        &mut self,
        p: Pending,
        challenger: &Pubkey,
        won: bool,
    ) -> Result<(), BanksClientError> {
        let epoch = self.epoch().await;
        let (keys, signers) = self.seats3([0, 2, 4]);
        let ix = hx::rule_claim_challenge(
            &program(),
            &self.council,
            &keys,
            &self.product,
            p.id,
            p.seq,
            challenger,
            won,
            epoch,
        );
        let refs: Vec<&Keypair> = signers.iter().collect();
        self.send(ix, &refs).await
    }

    async fn finalize_source(&mut self, id: u32) -> Result<(), BanksClientError> {
        let proposer = self.head(id).await.proposer;
        let ix = hx::finalize_source_challenges(&program(), &self.product, id, &proposer);
        self.send(ix, &[]).await
    }

    async fn settle_source(&mut self, id: u32, x: &Pubkey) -> Result<(), BanksClientError> {
        let ix = hx::settle_source_challenge(&program(), &self.product, id, x, x);
        self.send(ix, &[]).await
    }
}

/// A live source of component 1 at 1.00: `(env, its proposer, fetcher, id)`.
async fn one_live_source() -> (Env, Keypair, Keypair, u32) {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    (env, a, f, id)
}

/// Early review B2, the reviewer's sequence, on a source case: a genuine challenge ruled Won,
/// then the proposer's own challenger joins inside the window and stays Pending. At the case's
/// deadline it resolves on its ruling: it falls, the source takes its strike and the proposer's
/// slash goes to the winner; the pending challenge times out alone, its bond back.
#[tokio::test]
async fn a_pending_sibling_cannot_erase_a_won_source_ruling() {
    let (mut env, _a, _f, id) = one_live_source().await;
    let x = env.user(AMBA, 100 * USDC).await;
    let y = env.user(AMBA, 100 * USDC).await;
    let (x0, y0) = (env.usdc(&x.pubkey()).await, env.usdc(&y.pubkey()).await);
    let ix = env.source_challenge_ix(id, &x);
    env.send(ix, &[&x]).await.unwrap();
    let free = env.stake_of(&x.pubkey()).await.free;
    env.rule_source(id, &x.pubkey(), true).await;
    let ix = env.source_challenge_ix(id, &y);
    env.send(ix, &[&y]).await.unwrap();
    let deadline = env.head(id).await.source_case_deadline_ts;
    env.warp(deadline - 1).await;
    assert_oracle_error(
        env.finalize_source(id).await,
        OracleError::InvalidHeadsState,
    );
    env.warp(deadline).await;
    env.finalize_source(id).await.unwrap();
    let head = env.head(id).await;
    assert_eq!(head.source_case_status, SOURCE_CASE_FELL);
    assert_eq!(head.strike_count, 1);
    assert!(head.source_case_proposer_slashed > 0);
    env.settle_source(id, &y.pubkey()).await.unwrap();
    env.settle_source(id, &x.pubkey()).await.unwrap();
    assert_eq!(env.usdc(&y.pubkey()).await, y0, "timed out: its bond back");
    assert_eq!(env.usdc(&x.pubkey()).await, x0, "won: its bond back");
    assert_eq!(
        env.stake_of(&x.pubkey()).await.free,
        free + head.source_case_proposer_slashed
    );
}

/// Early review B2, the claim's sequence: a false claim's challenge ruled Won, a second challenge
/// Pending at the claim's deadline. The claim falls on its ruling: the winner takes the fetcher's
/// bond, the pending challenge gets its bond back, nothing prints.
#[tokio::test]
async fn a_pending_sibling_cannot_erase_a_won_claim_ruling() {
    let (mut env, _a, f, id) = one_live_source().await;
    let z = env.user(0, 100 * USDC).await;
    let w = env.user(0, 100 * USDC).await;
    let (f0, z0, w0) = (
        env.usdc(&f.pubkey()).await,
        env.usdc(&z.pubkey()).await,
        env.usdc(&w.pubkey()).await,
    );
    env.warp(env.now + 3_600).await;
    let held = env.update(&f, id, 2_000_000, env.now, origin("one")).await;
    env.challenge(&z, held, USDC).await.unwrap();
    env.rule(held, &z.pubkey(), true).await;
    env.challenge(&w, held, USDC).await.unwrap();
    let deadline = env.claim(held).await.ruling_deadline_ts;
    env.warp(deadline - 1).await;
    let ix = env.finalize_ix(held, &f.pubkey());
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    env.warp(deadline).await;
    env.finalize(held, &f.pubkey()).await;
    assert_eq!(env.claim(held).await.status, HeadClaimStatus::Fell);
    assert_eq!(env.head(id).await.value, 1_000_000);
    env.settle_claim_challenge(held, &w.pubkey()).await.unwrap();
    env.settle_claim_challenge(held, &z.pubkey()).await.unwrap();
    let bond = env.params.claim_bond;
    assert_eq!(env.usdc(&f.pubkey()).await, f0 - bond);
    assert_eq!(env.usdc(&z.pubkey()).await, z0 + bond);
    assert_eq!(env.usdc(&w.pubkey()).await, w0);
}

/// Early review B5, the reviewer's sequence: past the deadline a case with a Pending challenge
/// takes no ruling, so `RuleChallenge(Won)` cannot race `FinalizeSourceChallenges`: the case
/// resolves on what was ruled before (here nothing: it stands, no strike, the bond back).
/// Likewise a claim's.
#[tokio::test]
async fn no_ruling_is_taken_from_the_deadline() {
    let (mut env, _a, f, id) = one_live_source().await;
    let x = env.user(AMBA, 100 * USDC).await;
    let z = env.user(0, 100 * USDC).await;
    let (x0, z0) = (env.usdc(&x.pubkey()).await, env.usdc(&z.pubkey()).await);
    let ix = env.source_challenge_ix(id, &x);
    env.send(ix, &[&x]).await.unwrap();
    let deadline = env.head(id).await.source_case_deadline_ts;
    env.warp(deadline + 1).await;
    let result = env.try_rule_source(id, &x.pubkey(), true).await;
    assert_oracle_error(result, OracleError::InvalidHeadsState);
    env.finalize_source(id).await.unwrap();
    let head = env.head(id).await;
    assert_eq!(
        (head.source_case_status, head.strike_count),
        (SOURCE_CASE_STOOD, 0)
    );
    env.settle_source(id, &x.pubkey()).await.unwrap();
    assert_eq!(env.usdc(&x.pubkey()).await, x0);
    // A claim's case.
    let held = env.update(&f, id, 2_000_000, env.now, origin("one")).await;
    env.challenge(&z, held, USDC).await.unwrap();
    let deadline = env.claim(held).await.ruling_deadline_ts;
    env.warp(deadline + 1).await;
    let result = env.try_rule(held, &z.pubkey(), true).await;
    assert_oracle_error(result, OracleError::InvalidHeadsState);
    env.finalize(held, &f.pubkey()).await;
    assert_eq!(env.claim(held).await.status, HeadClaimStatus::Stood);
    assert_eq!(
        env.head(id).await.value,
        1_000_000,
        "never vetted: nothing prints"
    );
    env.settle_claim_challenge(held, &z.pubkey()).await.unwrap();
    assert_eq!(env.usdc(&z.pubkey()).await, z0);
}

/// Early review B6, the reviewer's sequence: fresh keys file 1-USDC challenges just before each
/// window end, each extending it by almost the extension. The window never passes the case's
/// deadline, fixed when it opened, so at the deadline no challenge joins and the case resolves on
/// its winning ruling: it falls and strikes.
#[tokio::test]
async fn late_challenges_cannot_postpone_the_deadline() {
    let (mut env, _a, _f, id) = one_live_source().await;
    let x = env.user(AMBA, 100 * USDC).await;
    let ix = env.source_challenge_ix(id, &x);
    env.send(ix, &[&x]).await.unwrap();
    env.rule_source(id, &x.pubkey(), true).await;
    let deadline = env.head(id).await.source_case_deadline_ts;
    let mut late = 0;
    loop {
        let end = env.head(id).await.source_case_window_end_ts;
        assert!(end <= deadline);
        if end == deadline {
            break;
        }
        env.warp(end - 1).await;
        let y = env.user(0, 10 * USDC).await;
        let ix = env.source_challenge_ix(id, &y);
        env.send(ix, &[&y]).await.unwrap();
        late += 1;
    }
    assert!(late > 20, "{late} late challenges");
    env.warp(deadline).await;
    let y = env.user(0, 10 * USDC).await;
    let ix = env.source_challenge_ix(id, &y);
    assert_oracle_error(env.send(ix, &[&y]).await, OracleError::InvalidHeadsState);
    env.finalize_source(id).await.unwrap();
    let head = env.head(id).await;
    assert_eq!(
        (head.source_case_status, head.strike_count),
        (SOURCE_CASE_FELL, 1)
    );
}

// Round 3 review, 2026-10-30
// ------------------------------------------------------------------------------------------

/// Round 3 R2, the reviewer's sequence: a Live head printed 1.00; before the listing an
/// authentic 2.00 update is committed and revealed, its window ending long before K, and left
/// unfinalized; the contract lists (opening 1.00) and nothing touches the head until K+1, so its
/// ring slot still names no contract. SettleComponent must not take "no slot" for "nothing
/// pending": it waits for the update; finalized, the source settles at +100% (as finalizing first
/// always gave).
#[tokio::test]
async fn an_untouched_head_waits_for_its_claim_from_before_the_listing() {
    let mut env = small_env(one_source_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 1_000 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    env.warp(env.now + 3_600).await;
    let update = env.update(&f, id, 2_000_000, env.now, origin("one")).await;
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    let contract = env.contract(expiry).await;
    let slot = usize::from(contract.ring_slot);
    assert_eq!(contract.listed_openings[0], 1_000_000);
    env.warp(expiry + env.params.challenge_window_secs + 1)
        .await;
    assert_ne!(env.head(id).await.capture_periods[slot], contract.period);
    let ix = env.settle_ix(expiry, 1, 0, &[id]);
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    env.finalize(update, &f.pubkey()).await;
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    let contract = env.contract(expiry).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_LIVE, 0);
    assert_eq!(contract.component_change_ppm[1], 1_000_000);
}

/// Round 3 R8, the reviewer's sequence: a head printed 1.00 at the freeze window's first three
/// sample instants, then 2.00 before a late listing. Listed then, its slot would start from 2.00
/// and date it back into the three samples already past (freeze median 2.00, not 1.00). A
/// contract lists no later than its first sample instant: after it, refused; at it, listed.
#[tokio::test]
async fn a_contract_lists_no_later_than_its_first_sample() {
    let mut env = small_env(one_source_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 1_000 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let expiry = DEC_01_2026;
    let days = env.params.settlement_business_days;
    let freeze = time::settlement_window_start(expiry, days);
    let window = time::settlement_window_start(freeze, days);
    let at: [u64; 5] = core::array::from_fn(|j| window + (freeze - window) * (j as u64 + 1) / 5);
    env.create_period(env.now + DAY, expiry).await;
    env.warp(at[2] + 60).await;
    let p = env.update(&f, id, 2_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    assert!(env.now < freeze);
    let terms = env.terms(expiry).await;
    let ix = env.list_ix(
        expiry,
        HeadsSettlementTarget::ContractChannel,
        terms,
        50 * USDC,
    );
    let oracle = env.h.oracle_authority.insecure_clone();
    assert_oracle_error(
        env.send(ix, &[&oracle]).await,
        OracleError::InvalidHeadsState,
    );

    // The next contract, listed at its first sample instant exactly.
    let expiry = FEB_01_2027;
    let freeze = time::settlement_window_start(expiry, days);
    let window = time::settlement_window_start(freeze, days);
    let first = window + (freeze - window) / 5;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(first).await;
    env.list(expiry, 50 * USDC).await;
    assert_eq!(env.contract(expiry).await.listing_ts, first);
}

fn lta_terms(from: u64, reset: u64) -> hx::HeadsLtaTerms {
    hx::HeadsLtaTerms {
        effective_from_ts: from,
        next_reset_ts: reset,
        floor: 0,
        ceiling: 0,
    }
}

/// Contract `expiry`'s settlement-window sample instants, before it lists.
fn settlement_instants_of(expiry: u64, days: u8) -> [u64; 5] {
    let freeze = time::settlement_window_start(expiry, days);
    core::array::from_fn(|j| freeze + (expiry - freeze) * (j as u64 + 1) / 5)
}

impl Env {
    /// Commits an LTA-channel claim and tries its reveal in the next slot.
    async fn try_lta_update(
        &mut self,
        fetcher: &Keypair,
        id: u32,
        value: u64,
        source_ts: u64,
        lta: hx::HeadsLtaTerms,
    ) -> (Pending, Result<(), BanksClientError>) {
        let seq = self.head(id).await.claim_seq;
        let reveal = Self::reveal_params(value, source_ts, origin("lta"), lta, seq);
        let ix = self.commit_ix(&fetcher.pubkey(), id, seq, HeadClaimKind::Update, &reveal);
        self.send(ix, &[fetcher]).await.unwrap();
        self.h.advance_slot().await;
        let ix = hx::reveal_claim(
            &program(),
            &fetcher.pubkey(),
            &self.product,
            id,
            seq,
            reveal,
            None,
        )
        .unwrap();
        let result = self.send(ix, &[fetcher]).await;
        (Pending { id, seq }, result)
    }
}

/// A board settling on one source per component: a Contract source at 1.00 (so the contract
/// lists) and an LTA source of component 2 at 1.00, in effect from its fill and resetting at
/// `reset` (0: none), origin `origin("lta")`. Returns `(env, fetcher, the LTA head)`; the
/// contract at `DEC_01_2026` is not listed yet.
async fn an_lta_source(reset: u64) -> (Env, Keypair, u32) {
    let mut env = small_env(one_source_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 1_000 * USDC).await;
    let c = env.active_head(&a, &b, 1).await;
    let l = env.active_head(&a, &b, 2).await;
    let p = env
        .fill(&f, c, 1_000_000, env.now, origin("contract"))
        .await;
    let q = env
        .claim_with(
            &f,
            l,
            HeadClaimKind::Fill,
            1_000_000,
            env.now,
            origin("lta"),
            lta_terms(env.now, reset),
        )
        .await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    env.finalize(q, &f.pubkey()).await;
    env.create_period(env.now + DAY, DEC_01_2026).await;
    (env, f, l)
}

impl Env {
    /// Lists `DEC_01_2026`, finalizes `claims` once their windows end, settles every component
    /// and returns the LTA component's change.
    async fn lta_change(&mut self, f: &Keypair, claims: Vec<Pending>) -> i64 {
        self.warp(self.now + DAY + 60).await;
        for p in claims {
            self.finalize(p, &f.pubkey()).await;
        }
        for component in 0..3u8 {
            self.settle(DEC_01_2026, component, &[]).await;
        }
        let contract = self.contract(DEC_01_2026).await;
        assert_ne!(contract.component_flags[2] & COMPONENT_LIVE, 0);
        contract.component_change_ppm[2]
    }
}

/// Round 3 R3, the reviewer's sequence on settlement instants 22, 24, 26, 28 and 30: the LTA price
/// stands at 1.00 with no reset; observed at 21, 4.00 is announced effective at 23 and resetting
/// at 27. An LTA price counts only from its effective date: the announcement is refused. Captured
/// once in effect (at 23), the samples are 1.00, 4.00, 4.00 and none past the reset: +300%; an
/// identical recapture at 24 changes nothing.
#[tokio::test]
async fn an_identical_lta_recapture_moves_nothing() {
    for recapture in [false, true] {
        let (mut env, f, l) = an_lta_source(0).await;
        let at = settlement_instants_of(DEC_01_2026, env.params.settlement_business_days);
        let (t23, t27) = ((at[0] + at[1]) / 2, (at[2] + at[3]) / 2);
        env.warp(env.now + DAY).await;
        env.list(DEC_01_2026, 50 * USDC).await;
        env.warp(at[0] - 3_600).await;
        let ahead = lta_terms(t23, t27);
        let (_, refused) = env.try_lta_update(&f, l, 4_000_000, env.now, ahead).await;
        assert_oracle_error(refused, OracleError::InvalidHeadsState);
        env.warp(t23).await;
        let (p, ok) = env.try_lta_update(&f, l, 4_000_000, t23, ahead).await;
        ok.unwrap();
        let mut claims = vec![p];
        if recapture {
            env.warp(at[1]).await;
            let (p, ok) = env.try_lta_update(&f, l, 4_000_000, at[1], ahead).await;
            ok.unwrap();
            claims.push(p);
        }
        assert_eq!(env.lta_change(&f, claims).await, 3_000_000);
    }
}

/// Round 3 R4, the reviewer's sequence: the price of 1.00 resets at 25; observed at 21, 4.00 is
/// announced effective 25 resetting at 29; observed at 22, 5.00 effective 29 resetting at 40. Both
/// announcements are refused (neither erases the other). Captured as each takes effect, the
/// samples follow the schedule, 1.00, 1.00, 4.00, 4.00, 5.00: median 4.00, +300%.
#[tokio::test]
async fn a_later_lta_announcement_erases_no_earlier_interval() {
    let expiry = DEC_01_2026;
    let days = HeadsParams::spec_defaults().settlement_business_days;
    let at = settlement_instants_of(expiry, days);
    // The reviewer's time units: instants 22, 24, ... two units apart.
    let unit = (at[1] - at[0]) / 2;
    let t = |n: u64| at[0] - 22 * unit + n * unit;
    let (mut env, f, l) = an_lta_source(t(25)).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    env.warp(t(21)).await;
    let (_, refused) = env
        .try_lta_update(&f, l, 4_000_000, t(21), lta_terms(t(25), t(29)))
        .await;
    assert_oracle_error(refused, OracleError::InvalidHeadsState);
    env.warp(t(22) - 60).await;
    let (_, refused) = env
        .try_lta_update(&f, l, 5_000_000, env.now, lta_terms(t(29), t(40)))
        .await;
    assert_oracle_error(refused, OracleError::InvalidHeadsState);
    let mut claims = Vec::new();
    for (value, from, reset) in [(4_000_000, t(25), t(29)), (5_000_000, t(29), t(40))] {
        env.warp(from).await;
        let (p, ok) = env
            .try_lta_update(&f, l, value, from, lta_terms(from, reset))
            .await;
        ok.unwrap();
        claims.push(p);
    }
    assert_eq!(env.lta_change(&f, claims).await, 3_000_000);
}

/// Round 3 R6, the reviewer's sequence: the LTA price in effect is 1.00; before the listing an
/// authentic announcement of 2.00 effective after the contract's expiry is refused, so the
/// listing opens at the price in effect, 1.00, and the source settles at 0% (not -50% against
/// an opening that was never in effect).
#[tokio::test]
async fn a_listing_never_opens_on_an_announced_lta_price() {
    let (mut env, f, l) = an_lta_source(0).await;
    let (_, refused) = env
        .try_lta_update(&f, l, 2_000_000, env.now, lta_terms(DEC_01_2026 + DAY, 0))
        .await;
    assert_oracle_error(refused, OracleError::InvalidHeadsState);
    assert_eq!(env.head(l).await.value, 1_000_000);
    env.warp(env.now + DAY).await;
    env.list(DEC_01_2026, 50 * USDC).await;
    let contract = env.contract(DEC_01_2026).await;
    let listed = (0..usize::from(contract.listed_count))
        .find(|&i| contract.listed_head_ids[i] == l)
        .unwrap();
    assert_eq!(contract.listed_openings[listed], 1_000_000);
    assert_eq!(env.lta_change(&f, Vec::new()).await, 0);
}

/// Round 3 R1, the reviewer's sequence: the source stands at 1.00; at the third settlement
/// instant τ3 an authentic observation becomes 2.00 and its fetcher commits it. Before its
/// still-timely reveal, an attacker commits and reveals 9.99 dated τ3 + 1 (no evidence). No
/// reveal shuts out another: the authentic one reveals in its window. The council rejects the
/// attacker's claim, which prints nothing; an honest recapture at τ3 + 2. The samples are 1.00,
/// 1.00, 2.00, 2.00, 2.00 (not 1.00 at τ3): median 2.00, +100%.
#[tokio::test]
async fn a_later_reveal_cannot_shut_out_an_earlier_observation() {
    let (mut env, f, id, expiry) = one_listed_source(one_source_params()).await;
    let x = env.user(0, 1_000 * USDC).await;
    let z = env.user(0, 100 * USDC).await;
    let at = env.settlement_instants(expiry).await;
    env.warp(at[2]).await;
    let seq = env.head(id).await.claim_seq;
    let honest = Env::reveal_params(2_000_000, at[2], origin("one"), Default::default(), seq);
    let ix = env.commit_ix(&f.pubkey(), id, seq, HeadClaimKind::Update, &honest);
    env.send(ix, &[&f]).await.unwrap();
    env.warp(at[2] + 1).await;
    let attack = env
        .update(&x, id, 9_990_000, at[2] + 1, origin("one"))
        .await;
    let ix =
        hx::reveal_claim(&program(), &f.pubkey(), &env.product, id, seq, honest, None).unwrap();
    env.send(ix, &[&f]).await.unwrap();
    env.challenge(&z, attack, USDC).await.unwrap();
    env.rule(attack, &z.pubkey(), true).await;
    env.warp(at[2] + 2).await;
    let recapture = env
        .update(&f, id, 2_000_000, at[2] + 2, origin("one"))
        .await;
    env.warp(env.now + DAY + 60).await;
    env.finalize(attack, &x.pubkey()).await;
    assert_eq!(env.claim(attack).await.status, HeadClaimStatus::Fell);
    for p in [Pending { id, seq }, recapture] {
        env.finalize(p, &f.pubkey()).await;
    }
    for component in 0..3u8 {
        env.settle(expiry, component, &[]).await;
    }
    let contract = env.contract(expiry).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_LIVE, 0);
    assert_eq!(contract.component_change_ppm[1], 1_000_000);
}

/// Round 3 R7, the reviewer's sequence: D contributed 1,000 AMBA to C1 and left; after the
/// cooldown it holds C1's escrow (190 of the 1,000). C1 settles; an unrelated source case opens
/// before D runs the releasing sync; during it D delegates 1 AMBA and undelegates it, which pools
/// the new withdrawal with the old escrow. The upheld case strikes neither: the old escrow left
/// before it opened, the 1 AMBA came after. Once the new cooldown ends, everything comes back.
#[tokio::test]
async fn a_new_withdrawal_exposes_no_old_escrow() {
    let mut params = test_params();
    params.min_live_sources = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let (id, d, staked, c1) = env.escrow_for_c1(&a, &b, &f).await;
    let escrow = liability::escrow_required(staked, env.params.slash_rate_bps, 2);
    env.warp(c1 + DAY).await;
    env.settle_contract(c1).await;
    env.warp(env.now + 3_600).await;
    let ix = env.source_challenge_ix(id, &x);
    env.send(ix, &[&x]).await.unwrap();
    env.delegate(&d, id, AMBA).await;
    let ix = hx::undelegate(&program(), &d.pubkey(), &env.product, id, AMBA);
    env.send(ix, &[&d]).await.unwrap();
    env.uphold_source_case(id, &x).await;
    env.warp(env.now + env.params.cooldown_secs).await;
    let free = env.stake_of(&d.pubkey()).await.free;
    let ledger = find_head_delegation_address(&program(), &env.head_key(id), &d.pubkey()).0;
    env.sync(id, &d.pubkey()).await;
    assert_eq!(env.stake_of(&d.pubkey()).await.free, free + escrow + AMBA);
    if env.h.account(ledger).await.is_some() {
        assert_eq!(env.delegation_of(id, &d.pubkey()).await.slashed_total, 0);
    }
}

/// Round 3 R5, the reviewer's sequence: a component whose window saw 2,000 sources join and
/// retire (never more than the live list at once) beside its one listed source. Every one of
/// them is a member the record must hold, but none can count at settlement: the call that
/// completes the record reads them by their flags alone and settles on the live one. (The 2,000
/// records are written straight into the weights account; read whole, they alone would need
/// 282,000 bytes of the 262,144-byte heap. Membership now stops at `HEADS_MAX_JOINED`, round 4
/// N4: this is the completing call at about eight times that bound.)
#[tokio::test]
async fn two_thousand_past_members_settle_within_the_heap() {
    use ameba_oracle::state::heads::{
        HeadsWeightsLayout, WEIGHT_SOURCE_JOINED, WEIGHT_SOURCE_RETIRED,
    };
    const PAST: usize = 2_000;
    let (mut env, _f, id, expiry) = one_listed_source(one_source_params()).await;
    env.warp(expiry + env.params.challenge_window_secs + 1)
        .await;
    let slot = usize::from(env.contract(expiry).await.ring_slot);
    let mut account = env.h.account(env.board).await.unwrap();
    let mut board = HeadsBoard::unpack(&account.data).unwrap();
    board.contract_joined[slot * 24 + 1] = PAST as u16;
    board.pack(&mut account.data).unwrap();
    let key = env.board;
    env.h
        .ctx
        .set_account(&key, &AccountSharedData::from(account));
    // The listed source, recorded as usual: the record is not complete.
    let ix = env.settle_ix(expiry, 1, 0, &[id]);
    env.send(ix, &[]).await.unwrap();
    assert_eq!(
        env.contract(expiry).await.component_flags[1] & COMPONENT_SETTLED,
        0
    );
    // The 2,000 that joined and retired, recorded.
    let key = env.weights_key(expiry, 1);
    let mut account = env.h.account(key).await.unwrap();
    let (mut header, _) = HeadsWeightsHeader::unpack_account(&account.data).unwrap();
    let layout = HeadsWeightsLayout {
        source_capacity: 1 + PAST,
    };
    account.data.resize(layout.len(), 0);
    for i in 1..=PAST {
        let past = HeadsWeightSource {
            head_id: id + 1_000 + i as u32,
            flags: WEIGHT_SOURCE_JOINED | WEIGHT_SOURCE_RETIRED,
            origin_id: [(i % 251) as u8; 32],
            live_at: env.now - 30 * DAY,
            opening: 1_000_000,
            level: 3_000_000,
            change_ppm: 2_000_000,
            ..HeadsWeightSource::default()
        };
        past.pack(&mut account.data[layout.source_range(i)])
            .unwrap();
    }
    header.source_capacity = (1 + PAST) as u16;
    header.source_count = (1 + PAST) as u16;
    header.levels_recorded = (1 + PAST) as u16;
    header.joined_recorded = PAST as u16;
    header.pack_account(&mut account.data).unwrap();
    let rent = env.h.ctx.banks_client.get_rent().await.unwrap();
    account.lamports = rent.minimum_balance(layout.len());
    env.h
        .ctx
        .set_account(&key, &AccountSharedData::from(account));
    // Complete: this call settles, on the listed source alone.
    let ix = env.settle_ix(expiry, 1, (1 + PAST) as u16, &[]);
    env.measure("SettleComponent (2,001 records)", ix, &[])
        .await;
    let contract = env.contract(expiry).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_LIVE, 0);
    assert_eq!(contract.component_eligible[1], 1);
    assert_eq!(contract.component_change_ppm[1], 0);
}

// Round 4 review, 2026-10-30
// ------------------------------------------------------------------------------------------

/// Round 4 N1, the reviewer's sequence: a listed source reveals two updates its contract admits
/// (`adm_open` 2); a source case retires it before the freeze, so the settlement records it
/// without waiting for them, and the contract settles. Finalizing the updates then left the count
/// at 2 while the open claims went to 1, and no instruction could load the head again (the other
/// bond and the delegations locked). A settled slot's count goes as the head syncs: both
/// finalize, and the head loads.
#[tokio::test]
async fn claims_finalized_after_their_contract_settles_leave_the_head_loadable() {
    let (mut env, f, id, expiry) = one_listed_source(one_source_params()).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let freeze = env.contract(expiry).await.freeze_ts;
    let window = env.freeze_window(expiry).await;
    env.warp(window - DAY).await;
    let mut claims = Vec::new();
    for value in [1_100_000, 1_200_000] {
        env.warp(env.now + 60).await;
        claims.push(env.update(&f, id, value, env.now, origin("one")).await);
    }
    let slot = usize::from(env.contract(expiry).await.ring_slot);
    assert_eq!(env.head(id).await.adm_open[slot], 2);
    env.lose_source_case(id, &x).await;
    assert_eq!(env.head(id).await.status, HeadStatus::Retired);
    assert!(env.now < freeze);
    env.warp(expiry + DAY).await;
    env.settle_contract(expiry).await;
    for p in claims {
        env.finalize(p, &f.pubkey()).await;
    }
    let head = env.head(id).await;
    assert_eq!((head.open_claims, head.adm_open[slot]), (0, 0));
}

/// Round 4 N2, the reviewer's sequence: a listed source printed 1.00 at t1 and 2.00 at t2; then
/// 32 replays of the authentic (t1, 1.00) stand and fill the 32-print ring. The program's value
/// stays 2.00, and so does the source's current level for every reader (`heads_view::level_now`,
/// which the indicative index and the maker's quotes read): it is the latest print, never the
/// ring.
#[tokio::test]
async fn replays_filling_the_print_ring_move_no_current_level() {
    let (mut env, f, id, _) = one_listed_source(one_source_params()).await;
    let t1 = env.head(id).await.value_source_ts;
    env.warp(env.now + 3_600).await;
    let p = env.update(&f, id, 2_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let mut replays = Vec::new();
    for _ in 0..HEAD_PRINT_CAPACITY {
        env.warp(env.now + 60).await;
        replays.push(env.update(&f, id, 1_000_000, t1, origin("one")).await);
    }
    env.warp(env.now + DAY).await;
    for p in replays {
        env.finalize(p, &f.pubkey()).await;
    }
    let head = env.head(id).await;
    assert_eq!(head.value, 2_000_000);
    assert!(
        head.prints().iter().all(|&(_, v)| v == 1_000_000),
        "the ring: replays only"
    );
    assert_eq!(
        ameba_oracle::heads_view::level_now(channels::Channel::Contract, &head, env.now),
        Some(2_000_000)
    );
}

/// Round 4 N3, the reviewer's sequence: two authentic captures within the second of the third
/// settlement instant τ3 show 1.50, then 2.00 (claims a < b), and stand. Then the 1.50 capture is
/// committed again (claim c > b) and stands. A tie of one second goes by the observation's rank,
/// from what it says, not by its claim: the replay ranks as the original, so the contract settles
/// the same with it as without it.
#[tokio::test]
async fn a_replayed_capture_cannot_take_its_second() {
    let mut changes = Vec::new();
    for replay in [false, true] {
        let (mut env, f, id, expiry) = one_listed_source(one_source_params()).await;
        let at = env.settlement_instants(expiry).await;
        env.warp(at[2]).await;
        let mut claims = Vec::new();
        for value in [1_500_000, 2_000_000] {
            let (p, ok) = env
                .try_claim(&f, id, HeadClaimKind::Update, value, at[2], origin("one"))
                .await;
            ok.unwrap();
            claims.push(p);
        }
        if replay {
            env.warp(at[2] + 600).await;
            let (p, ok) = env
                .try_claim(
                    &f,
                    id,
                    HeadClaimKind::Update,
                    1_500_000,
                    at[2],
                    origin("one"),
                )
                .await;
            ok.unwrap();
            claims.push(p);
        }
        env.warp(env.now + DAY + 60).await;
        for p in claims {
            env.finalize(p, &f.pubkey()).await;
        }
        for component in 0..3u8 {
            env.settle(expiry, component, &[]).await;
        }
        changes.push(env.contract(expiry).await.component_change_ppm[1]);
    }
    assert_eq!(changes[0], changes[1]);
}

/// Round 5 item 2 (round 4 N4), the reviewer's sequence: junk sources join a contract's window
/// and retire before its freeze, using up its joins (the board's count starts at 255 to stand in
/// for them); then an honest replacement goes live. A joiner retired before the freeze can never
/// count (not eligible at the cutoff, not live at the freeze), so it leaves the membership it
/// took, and the replacement joins.
#[tokio::test]
async fn a_joiner_retired_before_the_freeze_frees_its_membership() {
    use ameba_oracle::state::heads::CAPTURE_JOINED;
    let (mut env, f, _, expiry) = one_listed_source(one_source_params()).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let slot = usize::from(env.contract(expiry).await.ring_slot);
    let at = slot * 24 + 1;
    let mut account = env.h.account(env.board).await.unwrap();
    let mut board = HeadsBoard::unpack(&account.data).unwrap();
    board.contract_joined[at] = 255;
    board.pack(&mut account.data).unwrap();
    let key = env.board;
    env.h
        .ctx
        .set_account(&key, &AccountSharedData::from(account));
    let mut joiners = Vec::new();
    for name in ["junk", "honest"] {
        let id = env.active_head(&a, &b, 1).await;
        let p = env.fill(&f, id, 1_000_000, env.now, origin(name)).await;
        env.warp(env.now + DAY).await;
        env.finalize(p, &f.pubkey()).await;
        assert_ne!(env.head(id).await.capture_flags[slot] & CAPTURE_JOINED, 0);
        assert_eq!(env.board().await.contract_joined[at], 256);
        joiners.push(id);
        if name == "junk" {
            env.lose_source_case(id, &x).await;
            let junk = env.head(id).await;
            assert_eq!(junk.status, HeadStatus::Retired);
            assert_eq!(junk.capture_flags[slot] & CAPTURE_JOINED, 0);
            assert_eq!(env.board().await.contract_joined[at], 255);
        }
    }
    assert!(env.now < env.contract(expiry).await.freeze_ts);
}

/// Round 4 N5, the reviewer's sequence on one source per channel: the LTA source's price of
/// 1.00 was in effect when captured and reset on day 8; the listing comes after it. A price past
/// its reset is no opening and no origin: a target settling on the LTA channel cannot list on
/// it, and a contract that lists does not take it (nor open it at the expired price).
#[tokio::test]
async fn an_expired_lta_price_neither_lists_nor_opens() {
    let mut env = small_env(one_source_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(30_000 * AMBA, 0).await;
    let b = env.user(30_000 * AMBA, 0).await;
    let f = env.user(0, 1_000 * USDC).await;
    let reset = env.now + 8 * DAY;
    let mut ids = Vec::new();
    let mut fills = Vec::new();
    for component in 0..3u8 {
        let id = env.active_head(&a, &b, component).await;
        let lta = if component == 2 {
            lta_terms(env.now, reset)
        } else {
            Default::default()
        };
        fills.push(
            env.claim_with(
                &f,
                id,
                HeadClaimKind::Fill,
                1_000_000,
                env.now,
                origin(&format!("c{component}")),
                lta,
            )
            .await,
        );
        ids.push(id);
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    env.warp(reset + DAY).await;
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    let terms = env.terms(expiry).await;
    let oracle = env.h.oracle_authority.insecure_clone();
    let ix = env.list_ix(
        expiry,
        HeadsSettlementTarget::FixedMixReference,
        terms,
        50 * USDC,
    );
    assert_oracle_error(
        env.send(ix, &[&oracle]).await,
        OracleError::InvalidHeadsState,
    );
    env.list(expiry, 50 * USDC).await;
    let contract = env.contract(expiry).await;
    let listed = &contract.listed_head_ids[..usize::from(contract.listed_count)];
    assert!(listed.contains(&ids[1]) && !listed.contains(&ids[2]));
    assert_eq!(
        (
            contract.component_listed_sources[2],
            contract.component_listed_origins[2]
        ),
        (0, 0)
    );
}

/// Round 4 N6, the reviewer's sequence: D contributed 1,000 AMBA to C1 and left, keeping C1's
/// escrow (190 of the 1,000); C1 settles and D runs no releasing sync. C2 lists; with no case
/// open, D delegates 1 AMBA, contributes to C2 and withdraws it. A case opens while C2 is
/// unsettled and is upheld: only the 1 AMBA that weighed in C2 answers for it (10% of it), not
/// the old escrow, which nothing could strike any more when the new withdrawal joined it.
#[tokio::test]
async fn a_withdrawal_before_a_later_case_exposes_no_old_escrow() {
    let mut params = test_params();
    params.min_live_sources = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let (id, d, staked, c1) = env.escrow_for_c1(&a, &b, &f).await;
    let escrow = liability::escrow_required(staked, env.params.slash_rate_bps, 2);
    env.warp(c1 + DAY).await;
    env.settle_contract(c1).await;
    let c2 = FEB_01_2027;
    env.create_period(env.now + DAY, c2).await;
    env.warp(env.now + DAY).await;
    env.list(c2, 50 * USDC).await;
    env.delegate(&d, id, AMBA).await;
    env.warp(env.now + DAY).await;
    let ix = hx::undelegate(&program(), &d.pubkey(), &env.product, id, AMBA);
    env.send(ix, &[&d]).await.unwrap();
    // The old escrow (C1 settled, so nothing can strike it) and the new 1 AMBA (weighed in C2),
    // each answering for its own exposure (round 6 review, findings 3 and 4).
    let held = env.delegation_of(id, &d.pubkey()).await;
    assert_eq!(
        (held.held, held.held_principal),
        (escrow + AMBA, staked + AMBA)
    );
    assert_eq!(held.held_exposed[0], staked);
    assert_eq!(held.held_exposed.iter().filter(|h| **h > 0).count(), 2);
    env.warp(env.now + 3_600).await;
    let ix = env.source_challenge_ix(id, &x);
    env.send(ix, &[&x]).await.unwrap();
    env.uphold_source_case(id, &x).await;
    env.sync(id, &d.pubkey()).await;
    let slash = liability::slash(AMBA, env.params.slash_rate_bps);
    assert_eq!(
        env.delegation_of(id, &d.pubkey()).await.slashed_total,
        slash
    );
}

/// Round 4 N7, the reviewer's sequence: a fill captured on day D stands and earns the fill prize.
/// A later update replaying that same observation stands, and earns nothing: the fill's day has
/// its prize.
#[tokio::test]
async fn a_replayed_fill_earns_no_update_prize() {
    let (mut env, f, id, _) = one_listed_source(one_source_params()).await;
    let filled = env.head(id).await.value_source_ts;
    env.warp(env.now + 3_600).await;
    let p = env.update(&f, id, 1_000_000, filled, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let claim = env.claim(p).await;
    assert_eq!(claim.status, HeadClaimStatus::Stood);
    assert_eq!(claim.prize_points, 0);
}

// Round 5 review, 2026-10-30
// ------------------------------------------------------------------------------------------

/// Round 5 item 1, the reviewer's collision: two standing observations of the third settlement
/// instant's second, 1,000,081 and 1,129,816 (origin [1; 32], no LTA terms), whose round-4
/// 32-bit ranks are equal (4,230,379,087), so the first to finalize kept the instant. One
/// second's observations now order by what they say: the contract settles the same in both
/// finalization orders.
#[tokio::test]
async fn colliding_observations_of_one_second_settle_the_same_in_any_order() {
    let mut changes = Vec::new();
    for reverse in [false, true] {
        let (mut env, f, id, expiry) = one_listed_source(one_source_params()).await;
        let at = env.settlement_instants(expiry).await;
        env.warp(at[2]).await;
        let mut claims = Vec::new();
        for value in [1_000_081, 1_129_816] {
            let (p, ok) = env
                .try_claim(&f, id, HeadClaimKind::Update, value, at[2], [1; 32])
                .await;
            ok.unwrap();
            claims.push(p);
        }
        if reverse {
            claims.reverse();
        }
        env.warp(env.now + DAY + 60).await;
        for p in claims {
            env.finalize(p, &f.pubkey()).await;
        }
        for component in 0..3u8 {
            env.settle(expiry, component, &[]).await;
        }
        changes.push(env.contract(expiry).await.component_change_ppm[1]);
    }
    assert_eq!(changes[0], changes[1]);
    assert_eq!(changes[0], 129_816);
}

impl Env {
    /// D's slashed total after `x`'s source case on `id` is opened, upheld and applied.
    async fn uphold_new_case(&mut self, id: u32, d: &Keypair, x: &Keypair) -> u64 {
        let before = self.delegation_of(id, &d.pubkey()).await.slashed_total;
        self.warp(self.now + 3_600).await;
        let ix = self.source_challenge_ix(id, x);
        self.send(ix, &[x]).await.unwrap();
        self.uphold_source_case(id, x).await;
        self.sync(id, &d.pubkey()).await;
        self.delegation_of(id, &d.pubkey()).await.slashed_total - before
    }
}

/// Round 5 item 3 (round 4 N6), the reviewer's numbers at 10% a strike: D's 1,000 AMBA weighed in
/// C1 and left, 190 kept as C1's escrow. While C1 is unsettled, D delegates 1 AMBA through C2's
/// window and withdraws it. C1 settles; then two C2-era source cases are upheld. Each withdrawal
/// keeps its own exposure, answering for its own contracts: the cases burn 0.1 and then 0.09 AMBA of
/// the 1 AMBA, not 100.1 and 90.09 of a pool that made the old escrow answer for C2.
#[tokio::test]
async fn a_withdrawal_during_a_later_contract_never_extends_an_older_escrow() {
    let mut params = test_params();
    params.min_live_sources = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let (id, d, _, c1) = env.escrow_for_c1(&a, &b, &f).await;
    let c2 = FEB_01_2027;
    env.create_period(env.now + DAY, c2).await;
    env.warp(env.now + DAY).await;
    env.list(c2, 50 * USDC).await;
    env.delegate(&d, id, AMBA).await;
    env.warp(env.now + DAY).await;
    let ix = hx::undelegate(&program(), &d.pubkey(), &env.product, id, AMBA);
    env.send(ix, &[&d]).await.unwrap();
    assert!(env.now < c1);
    env.warp(c1 + DAY).await;
    for component in 0..3u8 {
        env.settle(c1, component, &[]).await;
    }
    let ix = hx::finalize_contract(&program(), &env.product, c1);
    env.send(ix, &[]).await.unwrap();
    let rate = env.params.slash_rate_bps;
    let first = liability::slash(AMBA, rate);
    assert_eq!(env.uphold_new_case(id, &d, &x).await, first);
    assert_eq!(
        env.uphold_new_case(id, &d, &x).await,
        liability::slash(AMBA - first, rate)
    );
}

/// Round 5 item 3, the reviewer's close variant: C1 settles while a source case opened during
/// it is still unresolved; C2 lists, and during the case D delegates 1 AMBA through C2's window
/// and withdraws it. The old case is upheld: it strikes the old escrow (it opened while C1 was
/// unsettled) and not the 1 AMBA (added during it). A later C2-era case strikes the 1 AMBA alone.
#[tokio::test]
async fn an_unresolved_old_case_lends_no_new_exposure_to_old_escrow() {
    let mut params = test_params();
    params.min_live_sources = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let (id, d, staked, c1) = env.escrow_for_c1(&a, &b, &f).await;
    env.warp(c1 - DAY).await;
    let before = env.delegation_of(id, &d.pubkey()).await.slashed_total;
    let ix = env.source_challenge_ix(id, &x);
    env.send(ix, &[&x]).await.unwrap();
    env.warp(c1 + DAY).await;
    for component in 0..3u8 {
        env.settle(c1, component, &[]).await;
    }
    let ix = hx::finalize_contract(&program(), &env.product, c1);
    env.send(ix, &[]).await.unwrap();
    let c2 = FEB_01_2027;
    env.create_period(env.now + 3_600, c2).await;
    env.warp(env.now + 3_600).await;
    env.list(c2, 50 * USDC).await;
    env.delegate(&d, id, AMBA).await;
    env.warp(env.now + 3_600).await;
    let ix = hx::undelegate(&program(), &d.pubkey(), &env.product, id, AMBA);
    env.send(ix, &[&d]).await.unwrap();
    env.uphold_source_case(id, &x).await;
    env.sync(id, &d.pubkey()).await;
    let rate = env.params.slash_rate_bps;
    assert_eq!(
        env.delegation_of(id, &d.pubkey()).await.slashed_total - before,
        liability::slash(staked, rate)
    );
    assert_eq!(
        env.uphold_new_case(id, &d, &x).await,
        liability::slash(AMBA, rate)
    );
}

/// Round 5 item 5: the fill's day is not used up by the fill. An update of another observation
/// that day earns the day's update prize; a replay of the fill's own observation still earns
/// nothing (round 4 N7).
#[tokio::test]
async fn another_observation_of_the_fills_day_earns_its_prize() {
    let (mut env, f, id, _) = one_listed_source(one_source_params()).await;
    let head = env.head(id).await;
    let (filled, delegation) = (head.value_source_ts, head.delegation);
    let mut claims = Vec::new();
    for (value, ts) in [(1_000_000, filled), (1_100_000, filled + 3_600)] {
        env.warp(env.now + 3_600).await;
        claims.push(env.update(&f, id, value, ts, origin("one")).await);
    }
    assert_eq!((filled + 3_600) / DAY, filled / DAY);
    env.warp(env.now + DAY).await;
    let mut points = Vec::new();
    for p in claims {
        env.finalize(p, &f.pubkey()).await;
        points.push(env.claim(p).await.prize_points);
    }
    let prize = rewards::update_prize(&env.params.to_params(), delegation);
    assert_eq!(points, vec![0, prize]);
}

// Round 6 review, 2026-10-30
// ------------------------------------------------------------------------------------------

impl Env {
    /// Three sources of component 1 with distinct origins, posted by `a` (1,000 AMBA) and backed
    /// by `b` (1,500), filled and Live. Returns their ids.
    async fn three_live_sources(&mut self, a: &Keypair, b: &Keypair, f: &Keypair) -> Vec<u32> {
        let mut ids = Vec::new();
        let mut fills = Vec::new();
        for j in 0..3 {
            let id = self.active_head(a, b, 1).await;
            ids.push(id);
            fills.push(
                self.fill(f, id, 1_000_000, self.now, origin(&format!("o{j}")))
                    .await,
            );
        }
        self.warp(self.now + DAY).await;
        for p in fills {
            self.finalize(p, &f.pubkey()).await;
        }
        ids
    }

    async fn undelegate(&mut self, owner: &Keypair, id: u32, amount: u64) {
        let ix = hx::undelegate(&program(), &owner.pubkey(), &self.product, id, amount);
        self.send(ix, &[owner]).await.unwrap();
    }
}

/// Round 6 review, finding 1, the reviewer's order: three sources of distinct origins go live
/// backed; then B withdraws its 1,500 AMBA from two of them, leaving each 1,000 AMBA and one
/// delegator (below `floor_activate`, 2,000, and `min_delegators`, 2). They stay live, but are no
/// origins toward `min_live_sources` (3): the contract cannot list. B backing one again makes two
/// origins, still too few; backing the other, three, and it lists. Raising `floor_activate`
/// above every source's delegation is refused once heads exist (round 10 review, N2): the
/// component keeps its three origins.
#[tokio::test]
async fn an_unbacked_live_source_is_no_origin_at_the_listing() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let ids = env.three_live_sources(&a, &b, &f).await;
    assert_eq!(env.board().await.component_live_origins[1], 3);
    for &id in &ids[1..] {
        env.undelegate(&b, id, 1_500 * AMBA).await;
    }
    let board = env.board().await;
    assert_eq!(
        (
            board.component_live_sources[1],
            board.component_live_origins[1]
        ),
        (3, 1)
    );
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    let oracle = env.h.oracle_authority.insecure_clone();
    let terms = env.terms(expiry).await;
    let ix = env.list_ix(
        expiry,
        HeadsSettlementTarget::ContractChannel,
        terms,
        50 * USDC,
    );
    assert_oracle_error(
        env.send(ix, &[&oracle]).await,
        OracleError::InvalidHeadsState,
    );
    env.delegate(&b, ids[1], 1_500 * AMBA).await;
    assert_eq!(env.board().await.component_live_origins[1], 2);
    env.h.advance_slot().await;
    let ix = env.list_ix(
        expiry,
        HeadsSettlementTarget::ContractChannel,
        terms,
        50 * USDC,
    );
    assert_oracle_error(
        env.send(ix, &[&oracle]).await,
        OracleError::InvalidHeadsState,
    );
    env.delegate(&b, ids[2], 1_500 * AMBA).await;
    env.list(expiry, 50 * USDC).await;
    let contract = env.contract(expiry).await;
    assert_eq!(
        (
            contract.component_listed_sources[1],
            contract.component_listed_origins[1]
        ),
        (3, 3)
    );
    let board = env.board().await;
    let mut params = board.params;
    params.floor_activate = 3_000 * AMBA;
    let admin = env.h.admin.insecure_clone();
    let ix = hx::set_board_params(
        &program(),
        &admin.pubkey(),
        &env.product,
        board.params_version,
        params,
    );
    assert_oracle_error(
        env.send(ix, &[&admin]).await,
        OracleError::InvalidHeadsState,
    );
    assert_eq!(env.board().await.component_live_origins[1], 3);
}

/// Round 6 review, finding 1, at the freeze: a contract lists on three backed origins, then B
/// withdraws from one source. Before the freeze, the freeze counts two backed origins and the
/// component fails its freeze check (settling at the listing value); just after it, the freeze's
/// three were captured first and the check passes.
#[tokio::test]
async fn backing_lost_before_the_freeze_fails_its_check_and_after_it_does_not() {
    for before in [true, false] {
        let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
        let a = env.user(20_000 * AMBA, 0).await;
        let b = env.user(20_000 * AMBA, 0).await;
        let f = env.user(0, 100 * USDC).await;
        let ids = env.three_live_sources(&a, &b, &f).await;
        let expiry = DEC_01_2026;
        env.create_period(env.now + DAY, expiry).await;
        env.warp(env.now + DAY).await;
        env.list(expiry, 50 * USDC).await;
        let freeze = env.contract(expiry).await.freeze_ts;
        assert!(env.now < freeze - DAY);
        env.warp(if before { freeze - DAY } else { freeze + 1 })
            .await;
        env.undelegate(&b, ids[2], 1_500 * AMBA).await;
        env.warp(expiry + DAY).await;
        for component in 0..3u8 {
            env.settle(expiry, component, &[]).await;
        }
        let contract = env.contract(expiry).await;
        assert_eq!(
            contract.component_freeze_origins[1],
            if before { 2 } else { 3 }
        );
        assert_eq!(
            contract.component_degraded[1] == DEGRADED_FREEZE_CHECK_FAILED,
            before
        );
    }
}

/// Round 6 review, finding 1, dev migration: a board of version 1 (its live list without the
/// delegators) loads nowhere until selector 30 takes it with its live heads: each entry gets its
/// head's delegators, and the component's origins count only the backed ones (two of three here,
/// one source left with a single delegator below the floor).
#[tokio::test]
async fn a_version_1_board_takes_its_live_delegators() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let ids = env.three_live_sources(&a, &b, &f).await;
    env.undelegate(&b, ids[2], 1_500 * AMBA).await;
    let fresh = env.board().await;
    assert_eq!(fresh.live_delegators[..3], [2, 2, 1]);
    assert_eq!(fresh.component_live_origins[1], 2);
    // Back to version 1 as the round-6 program left it: no delegators, every live origin counted.
    let board_key = env.board;
    let mut account = env.h.account(board_key).await.unwrap();
    let mut old = HeadsBoard::unpack(&account.data).unwrap();
    old.live_delegators = [0; 96];
    old.component_live_origins[1] = 3;
    old.pack(&mut account.data).unwrap();
    account.data[8] = 1;
    env.h
        .ctx
        .set_account(&board_key, &AccountSharedData::from(account));
    let heads: Vec<Pubkey> = ids.iter().map(|id| env.head_key(*id)).collect();
    let payer = env.h.ctx.payer.pubkey();
    // Every live head, once each.
    let short = hx::migrate_board(&program(), &payer, &env.product, &heads[..2], &[]).unwrap();
    assert_oracle_error(env.send(short, &[]).await, OracleError::InvalidAccountList);
    let ix = hx::migrate_board(&program(), &payer, &env.product, &heads, &[]).unwrap();
    env.send(ix, &[]).await.unwrap();
    let board = env.board().await;
    assert_eq!(board.version, HeadsBoard::VERSION);
    assert_eq!(board.live_delegators, fresh.live_delegators);
    assert_eq!(board.component_live_origins[1], 2);
    // Migrated once only.
    env.h.advance_slot().await;
    let ix = hx::migrate_board(&program(), &payer, &env.product, &heads, &[]).unwrap();
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsAccount);
}

impl Env {
    /// A source of component 1 posted by `a` and backed by `b`, filled and Live.
    async fn live_source(&mut self, a: &Keypair, b: &Keypair, f: &Keypair) -> u32 {
        let id = self.active_head(a, b, 1).await;
        let p = self.fill(f, id, 1_000_000, self.now, origin("one")).await;
        self.warp(self.now + DAY).await;
        self.finalize(p, &f.pubkey()).await;
        id
    }

    /// Lists the contract of `expiry` at `at` (its period made the day before).
    async fn list_at(&mut self, expiry: u64, at: u64) -> usize {
        self.warp(at - DAY).await;
        self.create_period(self.now + DAY, expiry).await;
        self.warp(at).await;
        self.list(expiry, 50 * USDC).await;
        usize::from(self.contract(expiry).await.ring_slot)
    }

    /// D delegates `amount` at `from` and withdraws it all at `to`.
    async fn cycle(&mut self, d: &Keypair, id: u32, amount: u64, from: u64, to: u64) {
        self.warp(from).await;
        self.delegate(d, id, amount).await;
        self.warp(to).await;
        self.undelegate(d, id, amount).await;
    }
}

fn one_source_two_strikes() -> HeadsParams {
    let mut params = test_params();
    params.min_live_sources = 1;
    params
}

/// Round 6 review, finding 3, the reviewer's sequence at 10% a strike: D keeps 1 AMBA delegated
/// through C1's window; after C1's freeze, with no case open, D adds 999 AMBA and withdraws all
/// 1,000. Only the 1 AMBA weighed in C1: its escrow is 0.19 AMBA (two strikes), not 190, and a
/// C1-era case burns 0.1 AMBA of it, not 100.
#[tokio::test]
async fn stake_added_after_a_freeze_takes_none_of_its_exposure() {
    let mut env = small_env(one_source_two_strikes(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let d = env.user(2_000 * AMBA, 0).await;
    let id = env.live_source(&a, &b, &f).await;
    env.delegate(&d, id, AMBA).await;
    let c1 = DEC_01_2026;
    let slot = env.list_at(c1, env.now + 2 * DAY).await;
    let freeze = env.contract(c1).await.freeze_ts;
    env.warp(freeze + DAY).await;
    env.delegate(&d, id, 999 * AMBA).await;
    env.undelegate(&d, id, 1_000 * AMBA).await;
    let held = env.delegation_of(id, &d.pubkey()).await;
    assert_eq!(
        (held.held_exposed[1 << slot], held.held_exposed[0]),
        (AMBA, 999 * AMBA)
    );
    env.warp(env.now + env.params.cooldown_secs).await;
    env.sync(id, &d.pubkey()).await;
    let rate = env.params.slash_rate_bps;
    assert_eq!(
        env.delegation_of(id, &d.pubkey()).await.held,
        liability::escrow_required(AMBA, rate, 2)
    );
    assert_eq!(
        env.uphold_new_case(id, &d, &x).await,
        liability::slash(AMBA, rate)
    );
}

/// Round 6 review, finding 4, the reviewer's six exposures across four unsettled contracts
/// (monthly here): withdrawals weighed in {C1}, {C1, C2}, {C2}, {C2, C3}, {C3} (1,000 AMBA each)
/// and {C3, C4} (1 AMBA). Each keeps its own principal; once C1 to C3 settle, a C4-era case
/// strikes the 1 AMBA alone: 0.1 AMBA, not 100.1.
#[tokio::test]
async fn six_exposures_never_join_unrelated_principal() {
    let mut env = small_env(one_source_two_strikes(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let d = env.user(6_000 * AMBA, 0).await;
    let id = env.live_source(&a, &b, &f).await;
    let (e1, e2, e3, e4) = (DEC_01_2026, JAN_01_2027, FEB_01_2027, MAR_01_2027);
    let s1 = env.list_at(e1, env.now + 2 * DAY).await;
    let k = 1_000 * AMBA;
    env.cycle(&d, id, k, NOV_01_2026 - 6 * DAY, NOV_01_2026)
        .await;
    let s2 = env.list_at(e2, NOV_01_2026 + 9 * DAY).await;
    env.cycle(&d, id, k, NOV_01_2026 + 11 * DAY, NOV_01_2026 + 14 * DAY)
        .await;
    let f1 = env.contract(e1).await.freeze_ts;
    env.cycle(&d, id, k, f1 + DAY, DEC_01_2026).await;
    let s3 = env.list_at(e3, DEC_01_2026 + 9 * DAY).await;
    env.cycle(&d, id, k, DEC_01_2026 + 10 * DAY, DEC_01_2026 + 14 * DAY)
        .await;
    let f2 = env.contract(e2).await.freeze_ts;
    env.cycle(&d, id, k, f2 + DAY, JAN_01_2027 + DAY).await;
    let s4 = env.list_at(e4, JAN_01_2027 + 9 * DAY).await;
    env.cycle(&d, id, AMBA, JAN_01_2027 + 10 * DAY, JAN_01_2027 + 14 * DAY)
        .await;
    let held = env.delegation_of(id, &d.pubkey()).await;
    let m = |slots: &[usize]| slots.iter().fold(0usize, |m, s| m | 1 << s);
    let masks = [
        m(&[s1]),
        m(&[s1, s2]),
        m(&[s2]),
        m(&[s2, s3]),
        m(&[s3]),
        m(&[s3, s4]),
    ];
    for (i, mask) in masks.iter().enumerate() {
        assert_eq!(
            held.held_exposed[*mask],
            if i < 5 { k } else { AMBA },
            "{i}"
        );
    }
    for e in [e1, e2, e3] {
        env.settle_contract(e).await;
    }
    let rate = env.params.slash_rate_bps;
    assert_eq!(
        env.uphold_new_case(id, &d, &x).await,
        liability::slash(AMBA, rate)
    );
}

/// Round 6 review, finding 6, the reviewer's atoms at 10% and two strikes: 10 atoms leave weighed
/// in C1 and C2, 10 more weighed in C2 alone. Their escrow is sized on the 20 atoms together: 3
/// (two strikes take 2, then 1), not 1 for each 10. After C1 settles their exposures are one, and
/// two C2-era cases burn 2 and then 1 atom, all of it held.
#[tokio::test]
async fn escrow_holds_every_atom_of_exposures_that_become_one() {
    let mut env = small_env(one_source_two_strikes(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let d = env.user(AMBA, 0).await;
    let id = env.live_source(&a, &b, &f).await;
    let (e1, e2) = (DEC_01_2026, JAN_01_2027);
    let s1 = env.list_at(e1, env.now + 2 * DAY).await;
    let s2 = env.list_at(e2, NOV_01_2026 + 9 * DAY).await;
    env.cycle(&d, id, 10, NOV_01_2026 + 10 * DAY, NOV_01_2026 + 11 * DAY)
        .await;
    let f1 = env.contract(e1).await.freeze_ts;
    env.cycle(&d, id, 10, f1 + DAY, f1 + 2 * DAY).await;
    let held = env.delegation_of(id, &d.pubkey()).await;
    assert_eq!(
        (
            held.held_exposed[1 << s1 | 1 << s2],
            held.held_exposed[1 << s2]
        ),
        (10, 10)
    );
    env.warp(env.now + env.params.cooldown_secs).await;
    env.sync(id, &d.pubkey()).await;
    let rate = env.params.slash_rate_bps;
    assert_eq!(liability::escrow_required(20, rate, 2), 3);
    assert_eq!(env.delegation_of(id, &d.pubkey()).await.held, 3);
    env.settle_contract(e1).await;
    env.sync(id, &d.pubkey()).await;
    let held = env.delegation_of(id, &d.pubkey()).await;
    assert_eq!((held.held, held.held_exposed[1 << s2]), (3, 20));
    assert_eq!(env.uphold_new_case(id, &d, &x).await, 2);
    assert_eq!(env.uphold_new_case(id, &d, &x).await, 1);
}

// Round 7 review, 2026-10-30
// ------------------------------------------------------------------------------------------

/// Round 7 review, B3, the reviewer's sequence: a head posted now is filled with an observation of
/// 46 days ago; once it is Live, observations of the days after that one, finalized in ascending
/// order, each cleared the prize watermark. Prizes are for days from the posting day on as well:
/// they earn nothing, and one of a day since the posting earns the day's prize.
#[tokio::test]
async fn a_backdated_fill_opens_no_prizes_before_the_posting() {
    let mut env = small_env(one_source_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 1_000 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let old = env.now - 46 * DAY;
    let p = env.fill(&f, id, 1_000_000, old, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let head = env.head(id).await;
    assert_eq!(head.status, HeadStatus::Live);
    let today = env.now;
    let mut claims = Vec::new();
    for (k, ts) in [old + DAY, old + 2 * DAY, old + 3 * DAY, today]
        .into_iter()
        .enumerate()
    {
        env.warp(env.now + 600).await;
        claims.push(
            env.update(&f, id, 1_000_000 + k as u64, ts, origin("one"))
                .await,
        );
    }
    env.warp(env.now + DAY).await;
    let mut points = Vec::new();
    for p in claims {
        env.finalize(p, &f.pubkey()).await;
        points.push(env.claim(p).await.prize_points);
    }
    let prize = rewards::update_prize(&env.params.to_params(), head.delegation);
    assert_eq!(points, vec![0, 0, 0, prize]);
}

impl Env {
    /// An LTA source of component 2 posted by `a` and backed by `b`, filled at 1.00 with origin
    /// `name`, in effect from now and resetting at `reset` (0: none). Returns its fill.
    async fn lta_source(
        &mut self,
        a: &Keypair,
        b: &Keypair,
        f: &Keypair,
        name: &str,
        reset: u64,
    ) -> (u32, Pending) {
        let id = self.active_head(a, b, 2).await;
        let terms = lta_terms(self.now, reset);
        let p = self
            .claim_with(
                f,
                id,
                HeadClaimKind::Fill,
                1_000_000,
                self.now,
                origin(name),
                terms,
            )
            .await;
        (id, p)
    }
}

/// Round 7 review, B1, the reviewer's sequence: A, B, C and D are backed LTA sources of distinct
/// origins; D's price expired before the listing, so the contract lists A, B and C. Before the
/// freeze C falls below activation backing. The freeze counts the contract's own members still
/// backed with a price in effect then: A and B, two of the three needed, so the component settles
/// on its failed-freeze fallback. D, excluded and expired, supplies no origin.
#[tokio::test]
async fn a_source_the_contract_left_out_never_passes_its_freeze() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(40_000 * AMBA, 0).await;
    let b = env.user(40_000 * AMBA, 0).await;
    let f = env.user(0, 1_000 * USDC).await;
    env.three_live_sources(&a, &b, &f).await;
    let mut lta = Vec::new();
    let mut fills = Vec::new();
    for (name, reset) in [
        ("lta-a", 0),
        ("lta-b", 0),
        ("lta-c", 0),
        ("lta-d", env.now + 3 * DAY),
    ] {
        let (id, p) = env.lta_source(&a, &b, &f, name, reset).await;
        lta.push(id);
        fills.push(p);
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + 3 * DAY).await;
    env.list(expiry, 50 * USDC).await;
    let contract = env.contract(expiry).await;
    let listed = &contract.listed_head_ids[..usize::from(contract.listed_count)];
    assert!(
        !listed.contains(&lta[3]),
        "expired at the listing: left out"
    );
    assert!(env.now < contract.freeze_ts - DAY);
    env.undelegate(&b, lta[2], 1_500 * AMBA).await;
    for component in 0..2u8 {
        env.settle(expiry, component, &[]).await;
    }
    let payer = env.h.ctx.payer.pubkey();
    let ix =
        hx::settle_component(&program(), &payer, &env.product, expiry, 2, 0, &lta[..3]).unwrap();
    env.send(ix, &[]).await.unwrap();
    let contract = env.contract(expiry).await;
    assert_eq!(contract.component_freeze_origins[2], 2);
    assert_eq!(contract.component_degraded[2], DEGRADED_FREEZE_CHECK_FAILED);
}

/// Round 7 review, B2, the reviewer's sequence: the contract lists without an LTA source whose
/// price had expired. D keeps 1,000 AMBA delegated to that source through the contract's window,
/// then withdraws it with no case open. The source is no member of the contract, so D's stake
/// weighed in nothing: after the cooldown all of it comes back, and a case upheld later on the
/// source strikes nothing of it (it burned 100 AMBA of a 190 AMBA escrow).
#[tokio::test]
async fn delegators_of_a_left_out_source_answer_for_nothing_of_the_contract() {
    let (mut env, _f, l) = an_lta_source(NOW + 8 * DAY).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let d = env.user(2_000 * AMBA, 0).await;
    env.warp(NOW + 9 * DAY).await;
    env.list(DEC_01_2026, 50 * USDC).await;
    let contract = env.contract(DEC_01_2026).await;
    let listed = &contract.listed_head_ids[..usize::from(contract.listed_count)];
    assert!(!listed.contains(&l), "expired at the listing: left out");
    let free = env.stake_of(&d.pubkey()).await.free;
    env.delegate(&d, l, 1_000 * AMBA).await;
    env.warp(env.now + 2 * DAY).await;
    env.undelegate(&d, l, 1_000 * AMBA).await;
    env.warp(env.now + env.params.cooldown_secs).await;
    assert!(env.now < contract.freeze_ts);
    // Round 8 review, finding 2: nothing ties it to the contract, so the empty delegation closes
    // and its rent goes back to its payer (D).
    let key = find_head_delegation_address(&program(), &env.head_key(l), &d.pubkey()).0;
    let rent = env.h.account(key).await.unwrap().lamports;
    let sol = env.h.account(d.pubkey()).await.unwrap().lamports;
    env.sync(l, &d.pubkey()).await;
    assert_eq!(env.stake_of(&d.pubkey()).await.free, free);
    assert!(env.h.account(key).await.is_none(), "closed");
    assert_eq!(
        env.h.account(d.pubkey()).await.unwrap().lamports,
        sol + rent
    );
    env.warp(env.now + 3_600).await;
    let ix = env.source_challenge_ix(l, &x);
    env.send(ix, &[&x]).await.unwrap();
    env.uphold_source_case(l, &x).await;
    assert_eq!(env.stake_of(&d.pubkey()).await.free, free);
}

// Round 8 review, 2026-10-30
// ------------------------------------------------------------------------------------------

impl Env {
    fn expire_ix(&self, id: u32) -> Instruction {
        hx::expire_head(&program(), &self.product, id)
    }
}

/// Round 8 review, finding 1, the reviewer's sequence: 96 heads repeating one real source fill
/// every live slot, then lose their backing (B withdraws). Honest fills stand for their bonds but
/// cannot go Live, and the board cannot list. On a full live list `ExpireHead` retires an
/// unbacked Live head (never a backed one, nor on a list with room): three honest sources then go
/// Live and the contract lists on their three origins.
#[tokio::test]
async fn unbacked_live_sources_give_up_their_slots_on_a_full_board() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(300_000 * AMBA, 0).await;
    let b = env.user(300_000 * AMBA, 0).await;
    let f = env.user(0, 1_000 * USDC).await;
    let mut copies = Vec::new();
    let mut fills = Vec::new();
    for _ in 0..96 {
        let id = env.active_head(&a, &b, 1).await;
        copies.push(id);
        fills.push(
            env.fill(&f, id, 1_000_000, env.now, origin("the real report"))
                .await,
        );
    }
    let honest: Vec<u32> = {
        let mut ids = Vec::new();
        for _ in 0..3 {
            ids.push(env.active_head(&a, &b, 1).await);
        }
        ids
    };
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    assert_eq!(env.board().await.live_count, 96);
    // A backed entry keeps its slot.
    let ix = env.expire_ix(copies[95]);
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    for &id in &copies[..95] {
        env.undelegate(&b, id, 1_500 * AMBA).await;
    }
    // An honest fill stands for its bond, but there is no room.
    let p = env
        .fill(&f, honest[0], 1_000_000, env.now, origin("honest 0"))
        .await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    assert_eq!(env.head(honest[0]).await.status, HeadStatus::Active);
    // On a full list an unbacked entry gives up its slot, and an honest source takes it; with
    // room on the list, none is retired.
    for (k, &id) in honest.iter().enumerate() {
        let ix = env.expire_ix(copies[k]);
        env.send(ix, &[]).await.unwrap();
        assert_eq!(env.head(copies[k]).await.status, HeadStatus::Retired);
        assert_eq!(env.board().await.live_count, 95);
        let ix = env.expire_ix(copies[k + 3]);
        assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
        let p = env
            .fill(&f, id, 1_000_000, env.now, origin(&format!("honest {k}")))
            .await;
        env.warp(env.now + DAY).await;
        env.finalize(p, &f.pubkey()).await;
        assert_eq!(env.head(id).await.status, HeadStatus::Live);
        assert_eq!(env.board().await.live_count, 96);
    }
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    // The three honest origins, and the copies' one through the copy still backed.
    let contract = env.contract(expiry).await;
    assert_eq!(contract.component_listed_origins[1], 4);
}

impl Env {
    /// Opens a source case on `id` and upholds it, measuring the resolution
    /// (`FinalizeSourceChallenges`) as `label`.
    async fn uphold_measured(&mut self, id: u32, x: &Keypair, label: &str) -> u64 {
        let ix = self.source_challenge_ix(id, x);
        self.send(ix, &[x]).await.unwrap();
        let proposer = self.head(id).await.proposer;
        let end = self.head(id).await.source_case_window_end_ts;
        self.warp(end).await;
        self.rule_source(id, &x.pubkey(), true).await;
        let ix = hx::finalize_source_challenges(&program(), &self.product, id, &proposer);
        self.production_on().await;
        let units = self.measure(label, ix, &[]).await;
        self.production_off().await;
        let ix =
            hx::settle_source_challenge(&program(), &self.product, id, &x.pubkey(), &x.pubkey());
        self.send(ix, &[]).await.unwrap();
        units
    }

    /// Lists `expiry` at `at` (its period made the day before), measured as `label`.
    async fn list_measured(&mut self, expiry: u64, at: u64, label: &str) -> u64 {
        self.warp(at - DAY).await;
        self.create_period(self.now + DAY, expiry).await;
        self.warp(at).await;
        let terms = self.terms(expiry).await;
        let ix = self.list_ix(
            expiry,
            HeadsSettlementTarget::ContractChannel,
            terms,
            50 * USDC,
        );
        let oracle = self.h.oracle_authority.insecure_clone();
        self.production_on().await;
        let units = self.measure(label, ix, &[&oracle]).await;
        self.production_off().await;
        units
    }

    async fn set_params_measured(&mut self, params: HeadsParams, label: &str) -> u64 {
        let version = self.board().await.params_version;
        let admin = self.h.admin.insecure_clone();
        let ix = hx::set_board_params(&program(), &admin.pubkey(), &self.product, version, params);
        self.production_on().await;
        let units = self.measure(label, ix, &[&admin]).await;
        self.production_off().await;
        units
    }
}

/// 96 live sources in component 1 for the full-size benchmarks, posted by `a`, of distinct
/// origins that agree in their first 28 bytes (every pair compares every word), the first two
/// (X and Y) with their own backing, the rest with `a`'s 1,000 and `b`'s 1,500 AMBA. Returns
/// their ids and the last fill's finalization CU (the 96th going live).
impl Env {
    async fn full_board(
        &mut self,
        a: &Keypair,
        b: &Keypair,
        f: &Keypair,
        x_backing: &[(&Keypair, u64)],
        y_backing: &[(&Keypair, u64)],
    ) -> (Vec<u32>, u64) {
        let mut ids = Vec::new();
        for backing in [x_backing, y_backing] {
            let id = self.post(a, 1, 1_000 * AMBA).await;
            for (owner, amount) in backing {
                self.delegate(owner, id, *amount).await;
            }
            ids.push(id);
        }
        while ids.len() < 96 {
            ids.push(self.active_head(a, b, 1).await);
        }
        let mut fills = Vec::new();
        for (k, &id) in ids.iter().enumerate() {
            fills.push(self.fill(f, id, 1_000_000, self.now, adverse(k)).await);
        }
        self.warp(self.now + DAY).await;
        let last = fills.pop().unwrap();
        for p in fills {
            self.finalize(p, &f.pubkey()).await;
        }
        self.production_on().await;
        let last = self.finalize(last, &f.pubkey()).await;
        self.production_off().await;
        assert_eq!(self.board().await.component_live_origins[1], 96);
        (ids, last)
    }
}

/// An origin of the full-size benchmarks: 28 bytes shared, the last four `k`.
fn adverse(k: usize) -> [u8; 32] {
    let mut o = [0x5a; 32];
    o[28..].copy_from_slice(&(k as u32).to_le_bytes());
    o
}

/// The production artifact and the SBF test build, when `HEADS_BENCH_PRODUCTION_SO` names the
/// first (round 9 and 10 reviews: the production dispatch's admission, `dispatch.rs`, runs before
/// every heads instruction there). It runs only at the production address, so this test and its
/// SBF build are then made with `AMEBA_ORACLE_PROGRAM_ID` set to that address.
fn bench_elfs() -> Option<(Vec<u8>, Vec<u8>)> {
    let path = std::env::var_os("HEADS_BENCH_PRODUCTION_SO")?;
    let production = std::fs::read(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let dir = std::env::var_os("SBF_OUT_DIR").expect("SBF_OUT_DIR names the SBF test build");
    let test = std::fs::read(std::path::Path::new(&dir).join("ameba_oracle.so")).unwrap();
    Some((production, test))
}

/// The full-size benchmarks' harness: with a production artifact named, the test build as an
/// upgradeable program, so that each measured instruction can run on the production artifact
/// (`Env::production_on`); else the usual one.
async fn bench_harness() -> (Harness, Option<(Vec<u8>, Vec<u8>)>) {
    match bench_elfs() {
        None => (start().await, None),
        Some((production, test)) => {
            let h = start_upgradeable(&test, test.len().max(production.len())).await;
            (h, Some((production, test)))
        }
    }
}

/// A benchmark's TESTX board (`small_env`, mixes assumed).
async fn bench_env(params: HeadsParams) -> Env {
    let (h, elfs) = bench_harness().await;
    let mut env = small_env_on(h, params, HeadsMixBasis::Assumed).await;
    env.production = elfs;
    env.settle_chunk = HEADS_SETTLE_MAX_BATCH;
    env
}

/// A benchmark's RAMX board (`ramx_env`).
async fn bench_ramx_env() -> Env {
    let (h, elfs) = bench_harness().await;
    let mut env = ramx_env_on(h, test_params()).await;
    env.production = elfs;
    env.settle_chunk = HEADS_SETTLE_MAX_BATCH;
    env
}

impl Env {
    /// With a production artifact (`bench_elfs`), upgrades the program to it: the instructions
    /// until `production_off` carry the governance envelope and pass the production dispatch's
    /// admission. The benchmarks measure each reported instruction so, and run everything else
    /// (the dev council's rulings, the harness' setup) on the test build.
    async fn production_on(&mut self) {
        if let Some((production, _)) = self.production.clone() {
            self.h.upgrade_oracle(&production).await;
            self.governed = true;
        }
    }

    /// Back to the test build (the code the program started with, which a warp runs again).
    async fn production_off(&mut self) {
        if let (true, Some((_, test))) = (self.governed, self.production.clone()) {
            self.h.upgrade_oracle(&test).await;
            self.h.upgraded = None;
            self.governed = false;
        }
    }
}

/// `$e`, a measurement, on the production artifact when the benchmark has one.
macro_rules! on_production {
    ($env:ident, $e:expr) => {{
        $env.production_on().await;
        let units = $e;
        $env.production_off().await;
        units
    }};
}

fn report(cu: &[(&str, u64)]) {
    for (label, units) in cu {
        eprintln!("BENCH {label}: {units}");
        assert!(*units < 1_400_000, "{label}: {units}");
    }
}

/// Round 7 and 8 reviews, CU at full size (a merge condition since 2026-10-18), first board: 96
/// live sources of adverse origins and four contracts listed on them. Measured at their worst
/// branches: the 96th fill going live and an update declaring a new origin; each listing; a
/// parameter change; an undelegation losing backing, and one with four contracts' exposures
/// populated; a delegation whose own held stake carries four contracts' exposures, whose pending
/// strike unbacks its source and whose top-up backs it again, while four freezes are captured
/// and all 96 sources are live; a lost case slashing a proposer below backing and retiring its
/// source; a strike applied over populated exposures; and the completing settlement call on its
/// fallback. Each stays under 1,400,000 CU (the SBF artifact meters them; natively zero).
#[tokio::test]
async fn every_heavy_instruction_fits_at_full_size() {
    let mut env = bench_env(test_params()).await;
    let a = env.user(300_000 * AMBA, 0).await;
    let b = env.user(400_000 * AMBA, 0).await;
    let c = env.user(10_000 * AMBA, 0).await;
    let d = env.user(10_000 * AMBA, 0).await;
    let f = env.user(0, 10_000 * USDC).await;
    let x = env.user(AMBA, 1_000 * USDC).await;
    let mut cu: Vec<(&str, u64)> = Vec::new();
    // X: its proposer's 1,000, B's 1,040 (40 to withdraw in four parts), C's 1,000 (each at its
    // share of the floor at least, so each is a delegator). Y: 2,000.
    let (ids, last) = env
        .full_board(
            &a,
            &b,
            &f,
            &[(&b, 1_040 * AMBA), (&c, 1_000 * AMBA)],
            &[(&b, 1_000 * AMBA)],
        )
        .await;
    let (xh, yh) = (ids[0], ids[1]);
    cu.push(("FinalizeClaim: the 96th source goes live", last));
    let p = env
        .update(&f, ids[10], 1_010_000, env.now, adverse(500))
        .await;
    env.warp(env.now + DAY).await;
    cu.push((
        "FinalizeClaim: an update declares a new origin",
        on_production!(env, env.finalize(p, &f.pubkey()).await),
    ));
    let normal = env.board().await.params;
    let mut no_weight = normal;
    no_weight.min_settle_weight = u64::MAX;
    cu.push((
        "SetBoardParams",
        env.set_params_measured(no_weight, "SetBoardParams").await,
    ));
    let (dec, jan, feb, mar) = (DEC_01_2026, JAN_01_2027, FEB_01_2027, MAR_01_2027);
    cu.push((
        "ListContract (1st of 4)",
        env.list_measured(dec, env.now + 2 * DAY, "ListContract")
            .await,
    ));
    env.set_params_measured(normal, "SetBoardParams").await;
    // Y's first case unbacks it (1,000 left); its proposer backs it again.
    env.uphold_measured(yh, &x, "FinalizeSourceChallenges")
        .await;
    env.delegate(&a, yh, 1_000 * AMBA).await;
    let ix = hx::undelegate(&program(), &b.pubkey(), &env.product, ids[5], 1_500 * AMBA);
    cu.push((
        "Undelegate: backing lost",
        on_production!(env, env.measure("Undelegate", ix, &[&b]).await),
    ));
    // D's withdrawals from Z, and B's from X, weigh in one, two, three and then four contracts.
    let z = ids[6];
    let take = |env: &Env, owner: &Keypair, id: u32, amount: u64| {
        hx::undelegate(&program(), &owner.pubkey(), &env.product, id, amount)
    };
    assert!(env.now < NOV_01_2026 - 6 * DAY);
    env.cycle(&d, z, 1_000 * AMBA, NOV_01_2026 - 6 * DAY, NOV_01_2026)
        .await;
    let ix = take(&env, &b, xh, 10 * AMBA);
    env.send(ix, &[&b]).await.unwrap();
    cu.push((
        "ListContract (2nd of 4)",
        env.list_measured(jan, NOV_01_2026 + 9 * DAY, "ListContract")
            .await,
    ));
    env.cycle(
        &d,
        z,
        1_000 * AMBA,
        NOV_01_2026 + 10 * DAY,
        NOV_01_2026 + 12 * DAY,
    )
    .await;
    let ix = take(&env, &b, xh, 10 * AMBA);
    env.send(ix, &[&b]).await.unwrap();
    cu.push((
        "ListContract (3rd of 4)",
        env.list_measured(feb, NOV_01_2026 + 14 * DAY, "ListContract")
            .await,
    ));
    env.cycle(
        &d,
        z,
        1_000 * AMBA,
        NOV_01_2026 + 15 * DAY,
        NOV_01_2026 + 16 * DAY,
    )
    .await;
    let ix = take(&env, &b, xh, 10 * AMBA);
    env.send(ix, &[&b]).await.unwrap();
    cu.push((
        "ListContract (4th of 4)",
        env.list_measured(mar, NOV_01_2026 + 18 * DAY, "ListContract")
            .await,
    ));
    env.warp(NOV_01_2026 + 19 * DAY).await;
    env.delegate(&d, z, 1_000 * AMBA).await;
    let ix = take(&env, &b, xh, 10 * AMBA);
    env.send(ix, &[&b]).await.unwrap();
    env.warp(NOV_01_2026 + 20 * DAY).await;
    let ix = take(&env, &d, z, 1_000 * AMBA);
    cu.push((
        "Undelegate: four contracts' exposures",
        on_production!(env, env.measure("Undelegate", ix, &[&d]).await),
    ));
    // X's case, with every contract unsettled: its proposer's 1,000 go to the winner and X stays
    // backed (2,000: B's 1,000 and C's 1,000); B's strike waits.
    env.uphold_measured(xh, &x, "FinalizeSourceChallenges")
        .await;
    let held = env.delegation_of(xh, &b.pubkey()).await;
    assert_eq!(held.held_exposed.iter().filter(|h| **h > 0).count(), 4);
    assert!(env.now < env.contract(dec).await.freeze_ts);
    // Past every freeze with nothing changed since: B's top-up captures all four, applies the
    // strike to its stake and its four exposures (B's 900 below its share: X unbacked), then
    // backs X again (B's 1,400); 96 are live.
    let freeze = env.contract(mar).await.freeze_ts;
    env.warp(freeze + DAY).await;
    let board = env.board().await;
    assert_eq!(board.live_count, 96);
    assert!(board.live_backed(0) && board.live_head_ids[0] == xh);
    let ix = hx::delegate(
        &program(),
        &b.pubkey(),
        &b.pubkey(),
        &env.product,
        xh,
        500 * AMBA,
    );
    cu.push((
        "Delegate: four freezes captured, a strike over four exposures unbacks, the top-up backs",
        on_production!(env, env.measure("Delegate", ix, &[&b]).await),
    ));
    let board = env.board().await;
    assert!((0..4).all(|i| board.contract_flags[i] & RING_FREEZE_CAPTURED != 0));
    assert!(board.live_backed(0));
    // B was struck below its share and topped up past it.
    let struck = env.delegation_of(xh, &b.pubkey()).await;
    assert!(!struck.dust && struck.active + struck.fresh < 1_500 * AMBA);
    // Y's second case: the proposer slash unbacks it, the strike retires it.
    cu.push((
        "FinalizeSourceChallenges: slash unbacks, then retirement",
        env.uphold_measured(yh, &x, "FinalizeSourceChallenges")
            .await,
    ));
    assert_eq!(env.head(yh).await.status, HeadStatus::Retired);
    cu.push((
        "FinalizeSourceChallenges: slash unbacks",
        env.uphold_measured(z, &x, "FinalizeSourceChallenges").await,
    ));
    let ix = hx::sync_delegation(&program(), &env.product, z, &d.pubkey(), &d.pubkey());
    cu.push((
        "SyncDelegation: a strike over four contracts' exposures",
        on_production!(env, env.measure("SyncDelegation", ix, &[]).await),
    ));
    let chunks = env.settle(dec, 1, &[]).await;
    let completing = chunks.last().map_or(0, |(_, units)| *units);
    let most = chunks.iter().map(|(_, units)| *units).max().unwrap_or(0);
    cu.push((
        "SettleComponent: the completing call, on its fallback",
        completing,
    ));
    cu.push(("SettleComponent: the heaviest call", most));
    let contract = env.contract(dec).await;
    assert_eq!(contract.component_degraded[1], DEGRADED_TOO_LITTLE_WEIGHT);
    report(&cu);
}

/// Round 8 review, CU at full size, second board: 96 live sources of adverse origins with equal
/// backing (2,500 AMBA each) and four contracts listed. Updates of 96 descending values,
/// observed before the first contract's freeze window, finalize after its freeze: each takes
/// four sample slots, the first a due freeze capture. A parameter change captures the second
/// contract's freeze, and a lost case slashing a proposer below backing and retiring its source
/// the third's. One source retires after the first freeze and before its cutoff, and its
/// settlement weight threshold lies between 95 and 96 sources' weights: the settlement on live
/// data fails, and the fallback sorts all 96 freeze changes for its weighted median.
#[tokio::test]
async fn the_fallback_median_and_due_captures_fit_at_full_size() {
    let mut params = test_params();
    params.max_strikes = 1;
    let mut env = bench_env(params).await;
    let a = env.user(300_000 * AMBA, 0).await;
    let b = env.user(400_000 * AMBA, 0).await;
    let f = env.user(0, 10_000 * USDC).await;
    let x = env.user(AMBA, 1_000 * USDC).await;
    let mut cu: Vec<(&str, u64)> = Vec::new();
    let even = [(&b, 1_500 * AMBA)];
    let (ids, _) = env.full_board(&a, &b, &f, &even, &even).await;
    let normal = env.board().await.params;
    let mut threshold = normal;
    threshold.min_settle_weight = 95 * 2_500 * AMBA + 1_250 * AMBA;
    env.set_params_measured(threshold, "SetBoardParams").await;
    let (dec, jan, feb, mar) = (DEC_01_2026, JAN_01_2027, FEB_01_2027, MAR_01_2027);
    env.list_measured(dec, env.now + 2 * DAY, "ListContract")
        .await;
    env.set_params_measured(normal, "SetBoardParams").await;
    env.list_measured(jan, NOV_01_2026 - 2 * DAY, "ListContract")
        .await;
    env.list_measured(feb, NOV_01_2026, "ListContract").await;
    env.list_measured(mar, NOV_01_2026 + 2 * DAY, "ListContract")
        .await;
    // 96 descending observations of one second, before the freeze window of the first contract.
    let seen = env.now;
    let mut updates = Vec::new();
    for (k, &id) in ids.iter().enumerate() {
        let value = 2_000_000 - 10_000 * k as u64;
        updates.push(env.update(&f, id, value, seen, adverse(k)).await);
    }
    let dec_freeze = env.contract(dec).await.freeze_ts;
    assert!(seen < env.freeze_window(dec).await);
    env.warp(dec_freeze + 3_600).await;
    let mut first = None;
    let mut most = 0;
    env.production_on().await;
    for p in updates {
        let units = env.finalize(p, &f.pubkey()).await;
        first.get_or_insert(units);
        most = most.max(units);
    }
    env.production_off().await;
    cu.push((
        "FinalizeClaim: an update, four sample slots and a due freeze capture",
        first.unwrap_or(0),
    ));
    cu.push(("FinalizeClaim: the heaviest of 96 updates", most));
    // One source retires after the first freeze and before its cutoff.
    env.uphold_measured(ids[95], &x, "FinalizeSourceChallenges")
        .await;
    assert_eq!(env.head(ids[95]).await.status, HeadStatus::Retired);
    assert!(env.now < dec + env.params.challenge_window_secs);
    let jan_freeze = env.contract(jan).await.freeze_ts;
    env.warp(jan_freeze + 3_600).await;
    cu.push((
        "SetBoardParams: a due freeze capture",
        env.set_params_measured(normal, "SetBoardParams").await,
    ));
    let feb_freeze = env.contract(feb).await.freeze_ts;
    env.warp(feb_freeze + 3_600).await;
    cu.push((
        "FinalizeSourceChallenges: a due freeze capture, slash unbacks, then retirement",
        env.uphold_measured(ids[94], &x, "FinalizeSourceChallenges")
            .await,
    ));
    assert_eq!(env.head(ids[94]).await.status, HeadStatus::Retired);
    let chunks = env.settle(dec, 1, &[]).await;
    let completing = chunks.last().map_or(0, |(_, units)| *units);
    cu.push((
        "SettleComponent: the completing call, the fallback's 96-source median",
        completing,
    ));
    let contract = env.contract(dec).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_LAST_VALID, 0);
    assert_eq!(contract.component_degraded[1], DEGRADED_TOO_LITTLE_WEIGHT);
    assert_ne!(contract.component_change_ppm[1], 0);
    let _ = mar;
    report(&cu);
}

/// Round 7 review, B1 and B2, dev migration: a board of the round-7 program (version 2) kept no
/// contract memberships. Selector 30 takes it with its live heads and its ring contracts: each
/// source the contract listed is its member again (and so counts at its freeze, and its
/// delegators' stake weighs in it).
#[tokio::test]
async fn a_version_2_board_takes_its_memberships() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let ids = env.three_live_sources(&a, &b, &f).await;
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    let fresh = env.board().await;
    assert!(
        fresh.live_flags[..3].iter().all(|f| *f > 1),
        "listed: members"
    );
    let board_key = env.board;
    let mut account = env.h.account(board_key).await.unwrap();
    let mut old = HeadsBoard::unpack(&account.data).unwrap();
    for f in &mut old.live_flags[..3] {
        *f &= 1;
    }
    old.pack(&mut account.data).unwrap();
    account.data[8] = 2;
    env.h
        .ctx
        .set_account(&board_key, &AccountSharedData::from(account));
    let heads: Vec<Pubkey> = ids.iter().map(|id| env.head_key(*id)).collect();
    let period = find_period_address(&program(), &env.product, expiry).0;
    let contract = find_heads_contract_address(&program(), &period).0;
    let payer = env.h.ctx.payer.pubkey();
    let ix = hx::migrate_board(&program(), &payer, &env.product, &heads, &[contract]).unwrap();
    env.send(ix, &[]).await.unwrap();
    let board = env.board().await;
    assert_eq!(board.version, HeadsBoard::VERSION);
    assert_eq!(board.live_flags, fresh.live_flags);
}

/// Round 6 review, finding 5: prizes are for source days from the fill's on. Updates observed one,
/// two and three days before the fill, captured after it, earn nothing (each was a new day of
/// prize before); one of the fill's own day still earns the day's prize, and a replay of the fill
/// still earns nothing.
#[tokio::test]
async fn an_observation_from_before_the_fill_earns_no_prize() {
    let (mut env, f, id, _) = one_listed_source(one_source_params()).await;
    let head = env.head(id).await;
    let (filled, delegation) = (head.value_source_ts, head.delegation);
    assert!(
        filled % DAY < DAY - 3_600,
        "the fill's day has room after it"
    );
    let mut claims = Vec::new();
    for (value, ts) in [
        (900_000, filled - 3 * DAY),
        (950_000, filled - 2 * DAY),
        (980_000, filled - DAY),
        (1_000_000, filled),
        (1_100_000, filled + 3_600),
    ] {
        env.warp(env.now + 600).await;
        claims.push(env.update(&f, id, value, ts, origin("one")).await);
    }
    env.warp(env.now + DAY).await;
    let mut points = Vec::new();
    for p in claims {
        env.finalize(p, &f.pubkey()).await;
        points.push(env.claim(p).await.prize_points);
    }
    let prize = rewards::update_prize(&env.params.to_params(), delegation);
    assert_eq!(points, vec![0, 0, 0, 0, prize]);
}

/// Round 6 review, finding 2 (round 5 item 6 removed): an LTA source left out of a listing because
/// its price had expired stays out of that contract. Two renewals print before the freeze; in
/// either finalization order neither joins it nor opens it, so the contract has no member of that
/// component and settles the same both ways. The source counts again from the next listing.
#[tokio::test]
async fn a_source_left_out_at_the_listing_stays_out_whatever_its_renewals() {
    use ameba_oracle::state::heads::CAPTURE_JOINED;
    let mut outcomes = Vec::new();
    for reversed in [false, true] {
        let (mut env, f, l) = an_lta_source(NOW + 8 * DAY).await;
        env.warp(NOW + 9 * DAY).await;
        env.list(DEC_01_2026, 50 * USDC).await;
        let contract = env.contract(DEC_01_2026).await;
        let listed = &contract.listed_head_ids[..usize::from(contract.listed_count)];
        assert!(!listed.contains(&l), "expired at the listing: left out");
        let (p, ok) = env
            .try_lta_update(&f, l, 1_200_000, env.now, lta_terms(env.now, 0))
            .await;
        ok.unwrap();
        env.warp(env.now + 600).await;
        let (q, ok) = env
            .try_lta_update(&f, l, 1_300_000, env.now, lta_terms(env.now, 0))
            .await;
        ok.unwrap();
        env.warp(env.now + DAY).await;
        let order = if reversed { [q, p] } else { [p, q] };
        for claim in order {
            env.finalize(claim, &f.pubkey()).await;
        }
        assert!(env.now < contract.freeze_ts);
        let slot = usize::from(contract.ring_slot);
        assert_eq!(env.head(l).await.capture_flags[slot] & CAPTURE_JOINED, 0);
        assert_eq!(env.board().await.contract_joined[slot * 24 + 2], 0);
        for component in 0..2u8 {
            env.settle(DEC_01_2026, component, &[]).await;
        }
        // Component 2 has no member: it settles on none (the source is not one to record).
        let payer = env.h.ctx.payer.pubkey();
        let record = |chunk: &[u32]| {
            hx::settle_component(&program(), &payer, &env.product, DEC_01_2026, 2, 0, chunk)
                .unwrap()
        };
        let (with_it, without) = (record(&[l]), record(&[]));
        assert_oracle_error(env.send(with_it, &[]).await, OracleError::InvalidHeadsState);
        env.send(without, &[]).await.unwrap();
        let settled = env.contract(DEC_01_2026).await;
        assert_eq!(settled.component_flags[2] & COMPONENT_LIVE, 0);
        outcomes.push((settled.component_flags, settled.component_change_ppm));
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

/// Round 6 review, findings 2 and 5, dev migration: a head of the round-6 program that a renewal
/// joined (its `join_opening`, the 32 bytes past the current wire, not zero) loads nowhere;
/// selector 30 with no contracts clears it and the left-out mark, and starts its prize day at
/// the day before its fill's. A head with none of these is refused, as a migrated account is.
#[tokio::test]
async fn a_round_6_head_sheds_its_renewal_join() {
    let (mut env, _, id, _) = one_listed_source(one_source_params()).await;
    let key = env.head_key(id);
    let head = env.head(id).await;
    let floor = (head.fill_source_ts / DAY) as u32 - 1;
    assert_eq!(head.last_prize_day, floor);
    let ix = env.migrate_ix(id, &[], &[]);
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsAccount);
    let mut account = env.h.account(key).await.unwrap();
    let mut old = Head::unpack(&account.data).unwrap();
    old.last_prize_day = 0;
    old.capture_flags[0] |= 64;
    old.pack(&mut account.data).unwrap();
    account.data[Head::WIRE_LEN..Head::WIRE_LEN + 8].copy_from_slice(&1_200_000u64.to_le_bytes());
    env.h
        .ctx
        .set_account(&key, &AccountSharedData::from(account));
    assert!(Head::unpack(&env.h.account(key).await.unwrap().data).is_err());
    let ix = env.migrate_ix(id, &[], &[]);
    env.send(ix, &[]).await.unwrap();
    let head = env.head(id).await;
    assert_eq!(
        (head.capture_flags[0] & 64, head.last_prize_day),
        (0, floor)
    );
    let data = env.h.account(key).await.unwrap().data;
    assert!(data[Head::WIRE_LEN..].iter().all(|b| *b == 0));
}

/// A fill makes a head Live only while it is still backed at the activation floor
/// (`floor_activate`, `min_delegators`). The sequence: the head activates on two delegators; the
/// second undelegates before any fill, cools and is released; a fill then stands for its bonds
/// and the head stays Active. It is not on the live list, so it is no origin toward
/// `min_live_sources`: a contract that needs it cannot list, and the freeze check would not count
/// it either.
#[tokio::test]
async fn a_fill_on_a_head_no_longer_backed_leaves_it_active() {
    let mut env = small_env(one_source_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 1_000 * USDC).await;
    let id = env.active_head(&a, &b, 1).await;
    let backing = env.delegation_of(id, &b.pubkey()).await.active;
    let ix = hx::undelegate(&program(), &b.pubkey(), &env.product, id, backing);
    env.send(ix, &[&b]).await.unwrap();
    env.warp(env.now + env.params.cooldown_secs).await;
    env.sync(id, &b.pubkey()).await;
    assert_eq!(env.head(id).await.delegators, 1);
    let usdc = env.usdc(&f.pubkey()).await;
    let p = env.fill(&f, id, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    assert_eq!(env.claim(p).await.status, HeadClaimStatus::Stood);
    assert_eq!(env.usdc(&f.pubkey()).await, usdc, "its bond back");
    let head = env.head(id).await;
    assert_eq!((head.status, head.open_fill), (HeadStatus::Active, false));
    let board = env.board().await;
    assert_eq!((board.live_count, board.component_live_origins[1]), (0, 0));
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    let terms = env.terms(expiry).await;
    let ix = env.list_ix(
        expiry,
        HeadsSettlementTarget::ContractChannel,
        terms,
        50 * USDC,
    );
    let oracle = env.h.oracle_authority.insecure_clone();
    assert_oracle_error(
        env.send(ix, &[&oracle]).await,
        OracleError::InvalidHeadsState,
    );
}

// Round 9 review, 2026-10-30
// ------------------------------------------------------------------------------------------

/// Round 9 review, findings 1 and 2, the reviewer's sequences on a full list. X is backed by its
/// proposer and E, the second delegator it needs; after the freeze E withdraws. At K+1 Y's
/// proposer withdraws below backing. Neither can then evict its source, a member of the unsettled
/// contract: X stays eligible, and a source case still opens on Y. An entry unbacked at the
/// listing is a member of it too, though no origin (round 10 review, N1); unbacked by K+1 for
/// longer than the cooldown, it gives up its slot (round 11 review, finding 3) and stays Live for
/// its contract (round 12 review), while X and Y, unbacked for days, do not.
#[tokio::test]
async fn eviction_spares_a_contracts_members() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(300_000 * AMBA, 0).await;
    let b = env.user(400_000 * AMBA, 0).await;
    let e = env.user(2_000 * AMBA, 0).await;
    let f = env.user(0, 10_000 * USDC).await;
    let x = env.user(AMBA, 1_000 * USDC).await;
    let (ids, _) = env
        .full_board(&a, &b, &f, &[(&e, 1_000 * AMBA)], &[(&b, 1_500 * AMBA)])
        .await;
    let (xh, yh, junk) = (ids[0], ids[1], ids[95]);
    env.undelegate(&b, junk, 1_500 * AMBA).await;
    let dec = DEC_01_2026;
    env.create_period(env.now + DAY, dec).await;
    env.warp(env.now + DAY).await;
    env.list(dec, 50 * USDC).await;
    let contract = env.contract(dec).await;
    let listed = &contract.listed_head_ids[..usize::from(contract.listed_count)];
    assert!(listed.contains(&xh) && listed.contains(&yh) && listed.contains(&junk));
    assert_eq!(
        (
            contract.component_listed_sources[1],
            contract.component_listed_origins[1]
        ),
        (96, 95)
    );
    env.warp(contract.freeze_ts + 3_600).await;
    env.undelegate(&e, xh, 1_000 * AMBA).await;
    assert_eq!(env.board().await.live_count, 96);
    let ix = env.expire_ix(xh);
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    env.warp(dec + env.params.challenge_window_secs + 1).await;
    env.undelegate(&a, yh, 1_000 * AMBA).await;
    for id in [xh, yh] {
        let ix = env.expire_ix(id);
        assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
        assert_eq!(env.head(id).await.status, HeadStatus::Live);
    }
    let ix = env.source_challenge_ix(yh, &x);
    env.send(ix, &[&x]).await.unwrap();
    let ix = env.expire_ix(junk);
    env.send(ix, &[]).await.unwrap();
    let head = env.head(junk).await;
    assert_eq!((head.status, head.evicted_ts), (HeadStatus::Live, env.now));
    assert_eq!(env.board().await.live_count, 95);
}

/// Round 9 review, dust delegators: a delegation counts toward `min_delegators` only while it
/// holds its share of the activation floor, `floor_activate / min_delegators` (1,000 AMBA here).
/// The reviewer's numbers: the proposer's whole floor and one atom leave the head Open with one
/// delegator. B counts from the delegation that brings it to its share, and stops at the
/// withdrawal that takes it below; the atom's withdrawal changes nothing.
#[tokio::test]
async fn a_dust_delegation_is_no_delegator() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(5_000 * AMBA, 0).await;
    let b = env.user(5_000 * AMBA, 0).await;
    let dust = env.user(AMBA, 0).await;
    let id = env.post(&a, 1, 2_000 * AMBA).await;
    env.delegate(&dust, id, 1).await;
    let head = env.head(id).await;
    assert_eq!(head.delegation, 2_000 * AMBA + 1);
    assert_eq!((head.status, head.delegators), (HeadStatus::Open, 1));
    env.delegate(&b, id, 1_000 * AMBA - 1).await;
    let head = env.head(id).await;
    assert_eq!((head.status, head.delegators), (HeadStatus::Open, 1));
    env.delegate(&b, id, 1).await;
    let head = env.head(id).await;
    assert_eq!((head.status, head.delegators), (HeadStatus::Active, 2));
    env.undelegate(&dust, id, 1).await;
    assert_eq!(env.head(id).await.delegators, 2);
    env.undelegate(&b, id, 1).await;
    assert_eq!(env.head(id).await.delegators, 1);
}

/// Eight items of one eighth of the basket each, mix `[20, 50, 30]`: 24 components, the most a
/// board holds.
async fn wide_env() -> Env {
    let items = (0..8)
        .map(|i| hx::HeadsItemParams {
            bucket_id: product(&format!("WIDEX-ITEM-{i}")),
            basket_weight_bps: 1_250,
            mix_bps: [2_000, 5_000, 3_000],
            mix_basis: HeadsMixBasis::Assumed,
        })
        .collect();
    let sentences = (0..24u8)
        .map(|c| {
            (
                c,
                c / 3,
                c % 3,
                format!(
                    "USD per chip, WIDEX item {}, channel {}, as of the observation date",
                    c / 3,
                    c % 3
                ),
            )
        })
        .collect();
    let (h, elfs) = bench_harness().await;
    let mut env = env_on(h, "WIDEX", test_params(), items, sentences).await;
    env.production = elfs;
    env
}

/// Round 9 review, heap: a board of 24 components with four sources of distinct origins in each
/// (96 live) lists a contract, and its freeze passes with nothing captured. `SetBoardParams`
/// then captures the freeze and recomputes every component's origins within the 256 KiB heap:
/// each component's grouping reserves its own population, not the whole list. (The SBF artifact
/// bounds the heap; natively it is not.)
#[tokio::test]
async fn twenty_four_components_capture_a_freeze_within_the_heap() {
    let mut env = wide_env().await;
    let a = env.user(300_000 * AMBA, 0).await;
    let b = env.user(400_000 * AMBA, 0).await;
    let f = env.user(0, 10_000 * USDC).await;
    let mut fills = Vec::new();
    for c in 0..24u8 {
        for k in 0..4 {
            let id = env.active_head(&a, &b, c).await;
            let lta = if c % 3 == HeadsChannel::Lta as u8 {
                lta_terms(env.now, 0)
            } else {
                Default::default()
            };
            let origin = origin(&format!("component {c}, source {k}"));
            fills.push(
                env.claim_with(&f, id, HeadClaimKind::Fill, 1_000_000, env.now, origin, lta)
                    .await,
            );
        }
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let board = env.board().await;
    assert_eq!(board.live_count, 96);
    assert!((0..24).all(|c| board.component_live_origins[c] == 4));
    let dec = DEC_01_2026;
    env.create_period(env.now + DAY, dec).await;
    env.warp(env.now + DAY).await;
    env.list(dec, 50 * USDC).await;
    let contract = env.contract(dec).await;
    env.warp(contract.freeze_ts + 3_600).await;
    let normal = env.board().await.params;
    let units = env.set_params_measured(normal, "SetBoardParams").await;
    let board = env.board().await;
    let slot = usize::from(contract.ring_slot);
    assert_ne!(board.contract_flags[slot] & RING_FREEZE_CAPTURED, 0);
    report(&[("SetBoardParams: 24 components, a due freeze capture", units)]);
}

/// Round 9 review, CU at full size, the combined paths: 96 live sources whose delegations descend
/// in list order (the activation median's insertion sort at its worst), and four contracts listed
/// on them. Measured: posting a head (round 11 review: the activation median of the 96 and the new
/// accounts) and a delegation to it, Open (the median again); each
/// listing, its terms document the largest kind-2 object (4,096 bytes) of near matches of the
/// commitment field it scans for, the field last (round 10 review); and an update declaring a new
/// origin, observed before the first contract's freeze window and finalized past all four freezes
/// in a month with no prize epoch yet: four sample slots, four due captures, the regrouping and
/// the epoch's creation in one `FinalizeClaim`.
#[tokio::test]
async fn the_combined_paths_fit_at_full_size() {
    let mut env = bench_env(test_params()).await;
    let a = env.user(300_000 * AMBA, 0).await;
    let b = env.user(400_000 * AMBA, 0).await;
    let f = env.user(0, 10_000 * USDC).await;
    let mut cu: Vec<(&str, u64)> = Vec::new();
    let even = [(&b, 1_500 * AMBA)];
    let (ids, _) = env.full_board(&a, &b, &f, &even, &even).await;
    for (k, &id) in ids.iter().enumerate() {
        env.delegate(&b, id, (96 - k as u64) * AMBA).await;
    }
    let board = env.board().await;
    assert!((1..96).all(|e| board.live_delegations[e - 1] > board.live_delegations[e]));
    let open = board.head_count;
    cu.push((
        "PostHead: the 96 descending live delegations' median, the head's and its delegation's accounts",
        env.post_measured(&a, 1, 1_000 * AMBA).await,
    ));
    let ix = hx::delegate(
        &program(),
        &b.pubkey(),
        &b.pubkey(),
        &env.product,
        open,
        1_500 * AMBA,
    );
    cu.push((
        "Delegate: an Open head, the activation median of 96 descending delegations",
        on_production!(env, env.measure("Delegate", ix, &[&b]).await),
    ));
    assert_eq!(env.head(open).await.status, HeadStatus::Active);
    let (dec, jan, feb, mar) = (DEC_01_2026, JAN_01_2027, FEB_01_2027, MAR_01_2027);
    env.adverse_terms = true;
    for (expiry, at) in [
        (dec, env.now + 2 * DAY),
        (jan, NOV_01_2026 - 2 * DAY),
        (feb, NOV_01_2026),
        (mar, NOV_01_2026 + 2 * DAY),
    ] {
        cu.push((
            "ListContract: 96 live, a 4,096-byte terms document of near matches",
            env.list_measured(expiry, at, "ListContract").await,
        ));
    }
    let p = env
        .update(&f, ids[10], 1_010_000, env.now, adverse(500))
        .await;
    assert!(env.now < env.freeze_window(dec).await);
    let mar_freeze = env.contract(mar).await.freeze_ts;
    env.warp(mar_freeze + 3_600).await;
    let epoch = find_heads_epoch_address(&program(), &env.board, time::month_id(env.now)).0;
    assert!(env.h.account(epoch).await.is_none());
    cu.push((
        "FinalizeClaim: a new origin, four sample slots, four due captures, a new prize epoch",
        on_production!(env, env.finalize(p, &f.pubkey()).await),
    ));
    let board = env.board().await;
    assert!((0..4).all(|i| board.contract_flags[i] & RING_FREEZE_CAPTURED != 0));
    assert_eq!(board.component_live_origins[1], 96);
    assert!(env.h.account(epoch).await.is_some());
    report(&cu);
}

/// Round 9 review, CU at full size, several pending strikes: `max_strikes` 8 (the strike
/// capacity), 96 live sources and four contracts listed. Y is backed by its proposer, G (2,000
/// AMBA, of which G withdraws 10 after each listing: four exposures) and H (1,000). Seven upheld
/// cases strike Y before the first freeze (its proposer pays the first in full). Past all four
/// freezes, G's top-up captures them, applies the seven strikes to its stake and its four
/// exposures, which takes G below its share of the floor (Y unbacked), and backs Y again. Then
/// (round 10 review) an eighth case retires Y; H (1,040 AMBA, of which it also withdrew 10 after
/// each listing), never synced since, has eight strikes unapplied: its sync applies them to its
/// stake and its four held exposures and cools what is left, and after the cooldown a sync
/// releases it (round 11 review).
#[tokio::test]
async fn seven_pending_strikes_fit_at_full_size() {
    let mut params = test_params();
    params.max_strikes = 8;
    let mut env = bench_env(params).await;
    let a = env.user(300_000 * AMBA, 0).await;
    let b = env.user(400_000 * AMBA, 0).await;
    let g = env.user(10_000 * AMBA, 0).await;
    let o = env.user(5_000 * AMBA, 0).await;
    let f = env.user(0, 10_000 * USDC).await;
    let x = env.user(AMBA, 1_000 * USDC).await;
    let mut cu: Vec<(&str, u64)> = Vec::new();
    let (ids, _) = env
        .full_board(
            &a,
            &b,
            &f,
            &[(&b, 1_500 * AMBA)],
            &[(&g, 2_000 * AMBA), (&o, 1_040 * AMBA)],
        )
        .await;
    let yh = ids[1];
    let (dec, jan, feb, mar) = (DEC_01_2026, JAN_01_2027, FEB_01_2027, MAR_01_2027);
    for (expiry, at) in [
        (dec, env.now + 2 * DAY),
        (jan, NOV_01_2026 - 2 * DAY),
        (feb, NOV_01_2026),
        (mar, NOV_01_2026 + 2 * DAY),
    ] {
        env.list_measured(expiry, at, "ListContract").await;
        env.undelegate(&g, yh, 10 * AMBA).await;
        env.undelegate(&o, yh, 10 * AMBA).await;
    }
    for _ in 0..7 {
        env.uphold_measured(yh, &x, "FinalizeSourceChallenges")
            .await;
    }
    let head = env.head(yh).await;
    assert_eq!((head.status, head.strike_count), (HeadStatus::Live, 7));
    let held = env.delegation_of(yh, &g.pubkey()).await;
    assert_eq!(held.held_exposed.iter().filter(|h| **h > 0).count(), 4);
    assert!(env.now < env.contract(dec).await.freeze_ts);
    let mar_freeze = env.contract(mar).await.freeze_ts;
    env.warp(mar_freeze + 3_600).await;
    let ix = hx::delegate(
        &program(),
        &g.pubkey(),
        &g.pubkey(),
        &env.product,
        yh,
        500 * AMBA,
    );
    cu.push((
        "Delegate: seven pending strikes over four exposures unback, four captures, the top-up backs",
        on_production!(env, env.measure("Delegate", ix, &[&g]).await),
    ));
    let board = env.board().await;
    assert!((0..4).all(|i| board.contract_flags[i] & RING_FREEZE_CAPTURED != 0));
    let entry = (0..96).find(|&e| board.live_head_ids[e] == yh).unwrap();
    assert!(board.live_backed(entry));
    let d = env.delegation_of(yh, &g.pubkey()).await;
    assert_eq!(d.applied_strikes, 7);
    cu.push((
        "FinalizeSourceChallenges: the eighth strike retires the source",
        env.uphold_measured(yh, &x, "FinalizeSourceChallenges")
            .await,
    ));
    assert_eq!(env.head(yh).await.status, HeadStatus::Retired);
    let ix = hx::sync_delegation(&program(), &env.product, yh, &o.pubkey(), &o.pubkey());
    cu.push((
        "SyncDelegation: a retired source, eight unapplied strikes over four withdrawals' exposures, automatic cooling",
        on_production!(env, env.measure("SyncDelegation", ix, &[]).await),
    ));
    let cooled = env.delegation_of(yh, &o.pubkey()).await;
    assert_eq!(cooled.applied_strikes, 8);
    assert_eq!(cooled.active + cooled.fresh, 0);
    assert!(cooled.held > 0);
    env.warp(env.now + env.params.cooldown_secs + 1).await;
    let ix = hx::sync_delegation(&program(), &env.product, yh, &o.pubkey(), &o.pubkey());
    cu.push((
        "SyncDelegation: the release after the cooldown",
        on_production!(env, env.measure("SyncDelegation", ix, &[]).await),
    ));
    let key = find_head_delegation_address(&program(), &env.head_key(yh), &o.pubkey()).0;
    if let Some(account) = env.h.account(key).await {
        assert_eq!(Delegation::unpack(&account.data).unwrap().held, 0);
    }
    report(&cu);
}

/// Round 9 review, CU at full size, maximum membership turnover: the contract lists 96 sources,
/// and every one retires (one strike each) before its freeze; 96 others then go live and join it
/// (one with 200,000 AMBA behind it), and update to 96 descending values observed before its
/// freeze window and finalized after its freeze. The large joiner retires after the freeze, and
/// the settlement weight threshold (95 regular joiners' full-window weight) is out of the live
/// data's reach but not the freeze's: settlement records all 192 sources, and the completing call
/// falls back to the weighted median of the joiners' 96 freeze changes.
#[tokio::test]
async fn a_turned_over_membership_settles_at_full_size() {
    let mut params = test_params();
    params.max_strikes = 1;
    let mut env = bench_env(params).await;
    let a = env.user(600_000 * AMBA, 0).await;
    let b = env.user(900_000 * AMBA, 0).await;
    let f = env.user(0, 10_000 * USDC).await;
    let x = env.user(AMBA, 1_000 * USDC).await;
    // Rent for 192 heads and their proposer delegations.
    env.h.fund(&a.pubkey(), 10_000_000_000).await;
    let mut cu: Vec<(&str, u64)> = Vec::new();
    let even = [(&b, 1_500 * AMBA)];
    let (listed, _) = env.full_board(&a, &b, &f, &even, &even).await;
    let normal = env.board().await.params;
    let mut threshold = normal;
    threshold.min_settle_weight = 95 * 2_500 * AMBA;
    env.set_params_measured(threshold, "SetBoardParams").await;
    let dec = DEC_01_2026;
    env.list_measured(dec, env.now + 2 * DAY, "ListContract")
        .await;
    env.set_params_measured(normal, "SetBoardParams").await;
    for &id in &listed {
        let ix = env.source_challenge_ix(id, &x);
        env.send(ix, &[&x]).await.unwrap();
    }
    let end = env.head(listed[95]).await.source_case_window_end_ts;
    env.warp(end).await;
    for &id in &listed {
        env.rule_source(id, &x.pubkey(), true).await;
        let ix = hx::finalize_source_challenges(&program(), &env.product, id, &a.pubkey());
        env.send(ix, &[]).await.unwrap();
        let ix =
            hx::settle_source_challenge(&program(), &env.product, id, &x.pubkey(), &x.pubkey());
        env.send(ix, &[]).await.unwrap();
    }
    assert_eq!(env.board().await.live_count, 0);
    let mut joiners = Vec::new();
    let mut fills = Vec::new();
    for k in 0..96 {
        let id = env.active_head(&a, &b, 1).await;
        if k == 95 {
            env.delegate(&b, id, 200_000 * AMBA).await;
        }
        fills.push(
            env.fill(&f, id, 1_000_000, env.now, adverse(1_000 + k))
                .await,
        );
        joiners.push(id);
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    assert_eq!(env.board().await.live_count, 96);
    let seen = env.now;
    let mut updates = Vec::new();
    for (k, &id) in joiners.iter().enumerate() {
        let value = 2_000_000 - 10_000 * k as u64;
        updates.push(env.update(&f, id, value, seen, adverse(1_000 + k)).await);
    }
    assert!(seen < env.freeze_window(dec).await);
    let freeze = env.contract(dec).await.freeze_ts;
    env.warp(freeze + 3_600).await;
    for p in updates {
        env.finalize(p, &f.pubkey()).await;
    }
    let large = joiners[95];
    env.uphold_measured(large, &x, "FinalizeSourceChallenges")
        .await;
    assert_eq!(env.head(large).await.status, HeadStatus::Retired);
    assert!(env.now < dec + env.params.challenge_window_secs);
    let chunks = env.settle(dec, 1, &[large]).await;
    assert_eq!(chunks.len(), 192_usize.div_ceil(env.settle_chunk));
    let completing = chunks.last().map_or(0, |(_, units)| *units);
    let most = chunks.iter().map(|(_, units)| *units).max().unwrap_or(0);
    cu.push((
        "SettleComponent: the completing call of 192 records, the fallback's 96-source median",
        completing,
    ));
    cu.push(("SettleComponent: the heaviest call", most));
    let contract = env.contract(dec).await;
    assert_eq!(contract.component_degraded[1], DEGRADED_TOO_LITTLE_WEIGHT);
    assert_ne!(contract.component_flags[1] & COMPONENT_LAST_VALID, 0);
    assert_ne!(contract.component_change_ppm[1], 0);
    report(&cu);
}

/// Round 9 review, CU at RAMX's dimensions (four items, twelve components): 96 live sources, three
/// of distinct origins in each component, a fourth in an LTA component and the other 62 in one, so
/// every component lists. The fourth LTA source's price lapses (past its reset) before the
/// listings and it loses its backing: no member of any contract (round 10 review, N1: a listing
/// takes every live source with a price in effect, backed or not); one of the 62 loses its backing
/// too, a member of all four, and a source case opens on it. Measured (round 12 review): past all
/// four freezes with nothing captured, the member leaving the list (its case open, unbacked past
/// the cooldown; four overdue captures), then, the list full again, the non-member retiring.
/// Also: the 96th going live; a parameter change; four listings; an update declaring a new
/// origin finalized then (four sample slots, a new prize epoch); and the completing settlement
/// call of the large component, the evicted member recorded.
#[tokio::test]
async fn ramx_dimensions_fit_at_full_size() {
    let mut env = bench_ramx_env().await;
    let a = env.user(300_000 * AMBA, 0).await;
    let b = env.user(400_000 * AMBA, 0).await;
    let f = env.user(0, 10_000 * USDC).await;
    let x = env.user(AMBA, 1_000 * USDC).await;
    let mut cu: Vec<(&str, u64)> = Vec::new();
    let board = env.board().await;
    let components = usize::from(board.component_count);
    assert_eq!(components, 12);
    let lta = |c: usize| board.component_channels[c] == HeadsChannel::Lta as u8;
    let large = (0..components).find(|&c| !lta(c)).unwrap();
    let lapsing = (0..components).find(|&c| lta(c)).unwrap();
    let order: Vec<usize> = (0..components)
        .filter(|&c| c != large)
        .chain([large])
        .collect();
    let mut fills = Vec::new();
    let mut heads = Vec::new();
    let mut junk = 0;
    for c in order {
        let n = match c {
            _ if c == large => 62,
            _ if c == lapsing => 4,
            _ => 3,
        };
        for j in 0..n {
            let id = env.active_head(&a, &b, c as u8).await;
            let terms = if c == lapsing && j == 3 {
                junk = id;
                lta_terms(env.now, env.now + 2 * DAY)
            } else if lta(c) {
                lta_terms(env.now, 0)
            } else {
                Default::default()
            };
            let k = fills.len();
            fills.push(
                env.claim_with(
                    &f,
                    id,
                    HeadClaimKind::Fill,
                    1_000_000,
                    env.now,
                    adverse(k),
                    terms,
                )
                .await,
            );
            if c == large {
                heads.push(id);
            }
        }
    }
    env.warp(env.now + DAY).await;
    let last = fills.pop().unwrap();
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let last = on_production!(env, env.finalize(last, &f.pubkey()).await);
    assert_eq!(env.board().await.live_count, 96);
    cu.push(("FinalizeClaim: the 96th source goes live", last));
    env.undelegate(&b, junk, 1_500 * AMBA).await;
    let squatter = heads[1];
    env.undelegate(&b, squatter, 1_500 * AMBA).await;
    let normal = env.board().await.params;
    cu.push((
        "SetBoardParams",
        env.set_params_measured(normal, "SetBoardParams").await,
    ));
    let (dec, jan, feb, mar) = (DEC_01_2026, JAN_01_2027, FEB_01_2027, MAR_01_2027);
    for (expiry, at) in [
        (dec, env.now + 2 * DAY),
        (jan, NOV_01_2026 - 2 * DAY),
        (feb, NOV_01_2026),
        (mar, NOV_01_2026 + 2 * DAY),
    ] {
        cu.push((
            "ListContract",
            env.list_measured(expiry, at, "ListContract").await,
        ));
        let contract = env.contract(expiry).await;
        let listed = &contract.listed_head_ids[..usize::from(contract.listed_count)];
        assert_eq!(listed.len(), 95);
        assert!(!listed.contains(&junk));
    }
    let p = env
        .update(&f, heads[5], 1_010_000, env.now, adverse(500))
        .await;
    assert!(env.now < env.freeze_window(dec).await);
    let ix = env.source_challenge_ix(squatter, &x);
    env.send(ix, &[&x]).await.unwrap();
    let standby = env.active_head(&a, &b, large as u8).await;
    let fill = env
        .fill(&f, standby, 1_000_000, env.now, adverse(600))
        .await;
    let mar_freeze = env.contract(mar).await.freeze_ts;
    env.warp(mar_freeze + 3_600).await;
    let ix = env.expire_ix(squatter);
    cu.push((
        "ExpireHead: a member with a case open, unbacked past the cooldown, leaves the list, four overdue captures",
        on_production!(env, env.measure("ExpireHead", ix, &[]).await),
    ));
    let board = env.board().await;
    assert!((0..4).all(|i| board.contract_flags[i] & RING_FREEZE_CAPTURED != 0));
    assert_eq!(board.live_count, 95);
    assert_ne!(env.head(squatter).await.evicted_ts, 0);
    env.finalize(fill, &f.pubkey()).await;
    assert_eq!(env.board().await.live_count, 96);
    let ix = env.expire_ix(junk);
    cu.push((
        "ExpireHead: a full list's unbacked non-member retires",
        on_production!(env, env.measure("ExpireHead", ix, &[]).await),
    ));
    assert_eq!(env.head(junk).await.status, HeadStatus::Retired);
    cu.push((
        "FinalizeClaim: a new origin, four sample slots, a new prize epoch",
        on_production!(env, env.finalize(p, &f.pubkey()).await),
    ));
    let chunks = env.settle(dec, large as u8, &[]).await;
    let (_, sources) = env.weights(dec, large as u8).await.unwrap();
    assert!(sources.iter().any(|s| s.head_id == squatter));
    let completing = chunks.last().map_or(0, |(_, units)| *units);
    let most = chunks.iter().map(|(_, units)| *units).max().unwrap_or(0);
    cu.push(("SettleComponent: the completing call", completing));
    cu.push(("SettleComponent: the heaviest call", most));
    report(&cu);
}

// Round 10 review, 2026-10-30
// ------------------------------------------------------------------------------------------

/// Round 10 review, N1, the reviewer's sequence: H's proposer holds the activation floor and D
/// exactly its share of it (`floor_activate / min_delegators`). Just before the listing D withdraws
/// one atom: H, one delegator short, is unbacked. The contract lists on the three other origins
/// and takes H as a member all the same, though no origin. D delegates the atom back: at the
/// freeze H is one of four origins, and the settlement records it as eligible.
#[tokio::test]
async fn a_dip_in_backing_at_the_listing_keeps_a_source_in_the_contract() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let d = env.user(2_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    env.three_live_sources(&a, &b, &f).await;
    let h = env.post(&a, 1, 2_000 * AMBA).await;
    env.delegate(&d, h, 1_000 * AMBA).await;
    let p = env.fill(&f, h, 1_000_000, env.now, origin("o3")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    assert_eq!(env.head(h).await.status, HeadStatus::Live);
    assert_eq!(env.board().await.component_live_origins[1], 4);
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.undelegate(&d, h, 1).await;
    assert_eq!(env.head(h).await.delegators, 1);
    env.list(expiry, 50 * USDC).await;
    let contract = env.contract(expiry).await;
    let listed = &contract.listed_head_ids[..usize::from(contract.listed_count)];
    assert!(listed.contains(&h));
    assert_eq!(
        (
            contract.component_listed_sources[1],
            contract.component_listed_origins[1]
        ),
        (4, 3)
    );
    env.delegate(&d, h, 1).await;
    assert_eq!(env.board().await.component_live_origins[1], 4);
    env.warp(contract.freeze_ts + 3_600).await;
    let normal = env.board().await.params;
    env.set_params_measured(normal, "SetBoardParams").await;
    let slot = usize::from(contract.ring_slot);
    assert_eq!(env.board().await.contract_freeze_origins[slot * 24 + 1], 4);
    env.settle(expiry, 1, &[]).await;
    let (_, sources) = env.weights(expiry, 1).await.unwrap();
    let record = sources.iter().find(|s| s.head_id == h).unwrap();
    assert_ne!(record.flags & WEIGHT_SOURCE_ELIGIBLE, 0);
    let contract = env.contract(expiry).await;
    assert_ne!(contract.component_flags[1] & COMPONENT_LIVE, 0);
    assert_eq!(contract.component_eligible[1], 4);
}

impl Env {
    async fn set_params(&mut self, params: HeadsParams) -> Result<(), BanksClientError> {
        let version = self.board().await.params_version;
        let admin = self.h.admin.insecure_clone();
        let ix = hx::set_board_params(&program(), &admin.pubkey(), &self.product, version, params);
        self.send(ix, &[&admin]).await
    }
}

/// Round 10 review N2, round 11 review findings 1 and 2, round 12 review N3: once a head is
/// posted, `SetBoardParams` refuses to change any parameter recorded state is read with later (the
/// share of the floor, the strike policy, the cooldown, the prizes, the dispute terms: lowering
/// `max_strikes`, changing a slash rate or lengthening an extension is refused) and still changes
/// each of the others, which apply only to what comes after; the ruling deadline is fixed too
/// (round 13 review, finding 4). Before
/// the first head every one changes. The destructuring names every field, so a new one must be
/// placed in one list or the other (heads.md, "Board parameters").
#[tokio::test]
async fn parameters_recorded_state_depends_on_are_fixed_once_a_head_is_posted() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let normal = env.board().await.params;
    let HeadsParams {
        floor_activate: _,
        min_delegators: _,
        slash_rate_bps: _,
        max_strikes: _,
        proposer_slash_bps: _,
        cooldown_secs: _,
        base_fill_prize: _,
        fill_prize_slope_ppb: _,
        base_update_prize: _,
        update_prize_slope_ppb: _,
        prize_cut_bps: _,
        claim_bond: _,
        challenge_bond: _,
        challenge_window_secs: _,
        challenge_extension_secs: _,
        head_ttl_secs: _,
        min_live_sources: _,
        min_settle_weight: _,
        alpha_bps: _,
        floor_post: _,
        beta_bps: _,
        settlement_business_days: _,
    } = normal;
    type Change = fn(&mut HeadsParams);
    let fixed: [Change; 13] = [
        |p| p.challenge_window_secs += 1,
        |p| p.challenge_extension_secs += 1,
        |p| p.challenge_bond += 1,
        |p| p.floor_activate = 3_000 * AMBA,
        |p| p.min_delegators = 3,
        |p| p.slash_rate_bps = 5_000,
        |p| p.max_strikes = 1,
        |p| p.proposer_slash_bps = 5_000,
        |p| p.cooldown_secs = DAY,
        |p| p.base_fill_prize += 1,
        |p| p.fill_prize_slope_ppb += 1,
        |p| p.base_update_prize += 1,
        |p| p.update_prize_slope_ppb += 1,
    ];
    let changeable: [Change; 9] = [
        |p| p.prize_cut_bps = 1_000,
        |p| p.claim_bond += 1,
        |p| p.head_ttl_secs += 1,
        |p| p.min_live_sources = 2,
        |p| p.min_settle_weight += 1,
        |p| p.alpha_bps = 1_000,
        |p| p.floor_post += 1,
        |p| p.beta_bps = 4_000,
        |p| p.settlement_business_days = 4,
    ];
    let with = |change: Change| {
        let mut p = normal;
        change(&mut p);
        assert_ne!(p, normal);
        p
    };
    for change in fixed.iter().chain(&changeable) {
        env.set_params(with(*change)).await.unwrap();
        env.set_params(normal).await.unwrap();
    }
    // The ruling deadline (its own instruction) too (round 13 review, finding 4).
    let admin = env.h.admin.insecure_clone();
    let ix = hx::set_ruling_deadline(&program(), &admin.pubkey(), &env.product, 14 * DAY);
    env.send(ix, &[&admin]).await.unwrap();
    let a = env.user(5_000 * AMBA, 0).await;
    let b = env.user(5_000 * AMBA, 0).await;
    let id = env.post(&a, 1, 2_000 * AMBA).await;
    env.delegate(&b, id, 1_000 * AMBA).await;
    let ix = hx::set_ruling_deadline(&program(), &admin.pubkey(), &env.product, 7 * DAY);
    assert_oracle_error(
        env.send(ix, &[&admin]).await,
        OracleError::InvalidHeadsState,
    );
    for change in fixed {
        assert_oracle_error(
            env.set_params(with(change)).await,
            OracleError::InvalidHeadsState,
        );
    }
    for change in changeable {
        let p = with(change);
        env.set_params(p).await.unwrap();
        assert_eq!(env.board().await.params, p);
        env.set_params(normal).await.unwrap();
    }
    assert_eq!(env.head(id).await.delegators, 2);
}

// Round 11 review, 2026-10-30
// ------------------------------------------------------------------------------------------

/// Round 11 review, finding 3: on a full list, 60 junk sources withdraw their backing before the
/// listing and become its members (no origins). Unbacked for less than `cooldown_secs`, a member
/// keeps its slot; once each has been unbacked that long it gives it up all the same, and an
/// honest source takes it, counted at the next listing. One that backed again for a while within
/// the cooldown starts over from its next dip. One with no time recorded (a dev head of before)
/// gets it from its first `ExpireHead`, and its slot a cooldown later. Round 12 review, N1 and N2:
/// an evicted member only leaves the list. It stays Live and the contract's member (recorded at
/// its settlement), its delegations do not cool; a case open on it when it is evicted resolves and
/// strikes C's delegation, and a case opened on it afterwards, which Y joins, strikes G's. Once
/// the contract settled, `ExpireHead` retires it.
#[tokio::test]
async fn long_unbacked_members_give_up_their_slots() {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(500_000 * AMBA, 0).await;
    let b = env.user(500_000 * AMBA, 0).await;
    let c = env.user(2_000 * AMBA, 0).await;
    let g = env.user(2_000 * AMBA, 0).await;
    let f = env.user(0, 10_000 * USDC).await;
    let x = env.user(AMBA, 1_000 * USDC).await;
    let y = env.user(AMBA, 1_000 * USDC).await;
    env.h.fund(&a.pubkey(), 10_000_000_000).await;
    let even = [(&b, 1_500 * AMBA)];
    let (ids, _) = env.full_board(&a, &b, &f, &even, &even).await;
    let junk = ids[36..].to_vec();
    let (dipper, accused, legacy, later) = (junk[0], junk[1], junk[3], junk[4]);
    env.delegate(&c, accused, 500 * AMBA).await;
    env.delegate(&g, later, 500 * AMBA).await;
    let start = env.now;
    for &id in &junk {
        env.undelegate(&b, id, 1_500 * AMBA).await;
    }
    let dec = DEC_01_2026;
    env.create_period(env.now + DAY, dec).await;
    env.warp(env.now + DAY).await;
    env.list(dec, 50 * USDC).await;
    let contract = env.contract(dec).await;
    assert_eq!(
        (
            contract.component_listed_sources[1],
            contract.component_listed_origins[1]
        ),
        (96, 36)
    );
    let ix = env.expire_ix(junk[2]);
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    env.warp(start + 10 * DAY).await;
    env.delegate(&b, dipper, 1_500 * AMBA).await;
    env.warp(start + 11 * DAY).await;
    env.undelegate(&b, dipper, 1_500 * AMBA).await;
    env.warp(start + 13 * DAY).await;
    let ix = env.source_challenge_ix(accused, &x);
    env.send(ix, &[&x]).await.unwrap();
    let key = env.head_key(legacy);
    let mut account = env.h.account(key).await.unwrap();
    let mut head = Head::unpack(&account.data).unwrap();
    assert_eq!(head.unbacked_since_ts, start);
    head.unbacked_since_ts = 0;
    head.pack(&mut account.data).unwrap();
    env.h
        .ctx
        .set_account(&key, &AccountSharedData::from(account));
    let mut standby = Vec::new();
    for k in 0..60 {
        let id = env.active_head(&a, &b, 1).await;
        let p = env
            .fill(&f, id, 1_000_000, env.now, adverse(1_000 + k))
            .await;
        standby.push((id, p));
    }
    env.warp(start + env.params.cooldown_secs).await;
    let ix = env.expire_ix(dipper);
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    let ix = env.expire_ix(legacy);
    env.send(ix, &[]).await.unwrap();
    let head = env.head(legacy).await;
    assert_eq!(
        (head.status, head.unbacked_since_ts),
        (HeadStatus::Live, env.now)
    );
    let mut honest = Vec::new();
    for &id in junk.iter().filter(|&&id| id != dipper && id != legacy) {
        let ix = env.expire_ix(id);
        env.send(ix, &[]).await.unwrap();
        let head = env.head(id).await;
        assert_eq!((head.status, head.evicted_ts), (HeadStatus::Live, env.now));
        let (h, p) = standby.pop().unwrap();
        env.finalize(p, &f.pubkey()).await;
        assert_eq!(env.head(h).await.status, HeadStatus::Live);
        honest.push(h);
    }
    // The case on the evicted source still resolves: a strike, which C's delegation takes.
    env.rule_source(accused, &x.pubkey(), true).await;
    let ix = hx::finalize_source_challenges(&program(), &env.product, accused, &a.pubkey());
    env.send(ix, &[]).await.unwrap();
    assert_eq!(env.head(accused).await.strike_count, 1);
    env.sync(accused, &c.pubkey()).await;
    let struck = env.delegation_of(accused, &c.pubkey()).await;
    assert_eq!(
        (struck.applied_strikes, struck.active, struck.held),
        (1, 450 * AMBA, 0)
    );
    // A case on an evicted member opens, Y joins it, and it strikes G's delegation.
    let ix = env.expire_ix(later);
    assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidHeadsState);
    let ix = env.source_challenge_ix(later, &x);
    env.send(ix, &[&x]).await.unwrap();
    let ix = env.source_challenge_ix(later, &y);
    env.send(ix, &[&y]).await.unwrap();
    assert_eq!(env.head(later).await.source_case_challenges, 2);
    let end = env.head(later).await.source_case_window_end_ts;
    env.warp(end).await;
    env.rule_source(later, &x.pubkey(), true).await;
    env.rule_source(later, &y.pubkey(), true).await;
    let ix = hx::finalize_source_challenges(&program(), &env.product, later, &a.pubkey());
    env.send(ix, &[]).await.unwrap();
    env.sync(later, &g.pubkey()).await;
    let struck = env.delegation_of(later, &g.pubkey()).await;
    assert_eq!((struck.applied_strikes, struck.active), (1, 450 * AMBA));
    // The one that backed again gives up its slot a cooldown after its second dip.
    env.warp(start + 11 * DAY + env.params.cooldown_secs).await;
    let ix = env.expire_ix(dipper);
    env.send(ix, &[]).await.unwrap();
    let (h, p) = standby.pop().unwrap();
    env.finalize(p, &f.pubkey()).await;
    honest.push(h);
    env.warp(start + 2 * env.params.cooldown_secs).await;
    let ix = env.expire_ix(legacy);
    env.send(ix, &[]).await.unwrap();
    let (h, p) = standby.pop().unwrap();
    env.finalize(p, &f.pubkey()).await;
    honest.push(h);
    assert_eq!(env.board().await.live_count, 96);
    let jan = JAN_01_2027;
    env.create_period(env.now + DAY, jan).await;
    env.warp(env.now + DAY).await;
    env.list(jan, 50 * USDC).await;
    let contract = env.contract(jan).await;
    let listed = &contract.listed_head_ids[..usize::from(contract.listed_count)];
    assert!(honest.iter().all(|h| listed.contains(h)));
    assert!(junk.iter().all(|j| !listed.contains(j)));
    assert_eq!(
        (
            contract.component_listed_sources[1],
            contract.component_listed_origins[1]
        ),
        (96, 96)
    );
    // The evicted members are recorded at their contract's settlement; once it settled, they
    // retire, and their delegations cool.
    for component in 0..3u8 {
        env.settle(dec, component, &[]).await;
    }
    let (_, sources) = env.weights(dec, 1).await.unwrap();
    assert!(junk.iter().all(|j| sources.iter().any(|s| s.head_id == *j)));
    env.send(hx::finalize_contract(&program(), &env.product, dec), &[])
        .await
        .unwrap();
    for id in [accused, later, junk[5]] {
        let ix = env.expire_ix(id);
        env.send(ix, &[]).await.unwrap();
        assert_eq!(env.head(id).await.status, HeadStatus::Retired);
    }
    env.sync(accused, &c.pubkey()).await;
    let cooled = env.delegation_of(accused, &c.pubkey()).await;
    assert_eq!((cooled.active, cooled.held), (0, 450 * AMBA));
}

// Round 12 review, 2026-10-30
// ------------------------------------------------------------------------------------------

impl Env {
    /// The off-chain view of the board now (`heads_view`), with every head and `expiry`'s
    /// contract.
    async fn view_of(&mut self, expiry: u64) -> heads_view::BoardView {
        let board = self.board().await;
        let mut heads = Vec::new();
        for id in 0..board.head_count {
            heads.push((self.head_key(id), self.head(id).await));
        }
        let period = find_period_address(&program(), &self.product, expiry).0;
        let key = find_heads_contract_address(&program(), &period).0;
        let contract = self.contract(expiry).await;
        let accounts = heads_view::BoardAccounts {
            address: self.board,
            board,
            heads,
            delegations: Vec::new(),
            contracts: vec![(key, contract)],
            weights: Vec::new(),
        };
        heads_view::view(&accounts, self.now)
    }
}

/// Round 12 review, N1, the reviewer's sequence on a full list: H has 250,000 AMBA of its
/// proposer's and E's 1,000, the second delegator it needs; the other 95 back B's report (one
/// origin, B's weight) but C and D. The contract lists, and H updates to +10% (its weight
/// decides the median). Thirteen days before the freeze E leaves (H unbacked, no freeze origin);
/// a day after the freeze H is evicted or not (`evict`), then D retires or not (`d_retires`), all
/// before the cutoff K. The off-chain indication then matches the settlement (round 13 review,
/// finding 3). Returns the component's settlement: change, eligible sources, weight and flags.
async fn evicted_member_settlement(evict: bool, d_retires: bool) -> (i64, u16, u64, u8) {
    let mut params = test_params();
    params.max_strikes = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(500_000 * AMBA, 0).await;
    let b = env.user(400_000 * AMBA, 0).await;
    let e = env.user(2_000 * AMBA, 0).await;
    let f = env.user(0, 10_000 * USDC).await;
    let x = env.user(AMBA, 1_000 * USDC).await;
    let h = env.post(&a, 1, 250_000 * AMBA).await;
    env.delegate(&e, h, 1_000 * AMBA).await;
    let mut ids = vec![h];
    while ids.len() < 96 {
        ids.push(env.active_head(&a, &b, 1).await);
    }
    let d = ids[3];
    let mut fills = Vec::new();
    for (k, &id) in ids.iter().enumerate() {
        let report = match k {
            0 => origin("H"),
            2 => origin("C"),
            3 => origin("D"),
            _ => origin("B"),
        };
        fills.push(env.fill(&f, id, 1_000_000, env.now, report).await);
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let dec = DEC_01_2026;
    env.create_period(env.now + DAY, dec).await;
    env.warp(env.now + DAY).await;
    env.list(dec, 50 * USDC).await;
    let p = env.update(&f, h, 1_100_000, env.now, origin("H")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let freeze = env.contract(dec).await.freeze_ts;
    env.warp(freeze - 13 * DAY).await;
    env.undelegate(&e, h, 1_000 * AMBA).await;
    env.warp(freeze + DAY).await;
    if evict {
        let ix = env.expire_ix(h);
        env.send(ix, &[]).await.unwrap();
        assert_eq!(env.board().await.live_count, 95);
    }
    if d_retires {
        env.uphold_measured(d, &x, "FinalizeSourceChallenges").await;
        assert_eq!(env.head(d).await.status, HeadStatus::Retired);
    }
    assert!(env.now < dec + env.params.challenge_window_secs);
    env.warp(dec + env.params.challenge_window_secs + 1).await;
    if !evict && !d_retires {
        // A call records at most `HEADS_SETTLE_MAX_BATCH` heads, and at least one while a member
        // is left to record (round 13 review, finding 2).
        let all = env.component_sources(dec, 1, &[]).await;
        let payer = env.h.ctx.payer.pubkey();
        for batch in [&all[..HEADS_SETTLE_MAX_BATCH + 1], &[]] {
            let ix =
                hx::settle_component(&program(), &payer, &env.product, dec, 1, 0, batch).unwrap();
            assert_oracle_error(env.send(ix, &[]).await, OracleError::InvalidAccountList);
        }
    }
    // The indication prices the contract's members, as settlement does (round 13 review,
    // finding 3).
    let indicated = match &env.view_of(dec).await.contracts[0].components[1].reading {
        heads_view::Reading::Live {
            change_ppm,
            eligible,
            ..
        } => (*change_ppm, *eligible),
        other => panic!("{other:?}"),
    };
    env.settle(dec, 1, &[]).await;
    let k = env.contract(dec).await;
    assert_eq!(
        indicated,
        (k.component_change_ppm[1], k.component_eligible[1])
    );
    (
        k.component_change_ppm[1],
        k.component_eligible[1],
        k.component_weight[1],
        k.component_flags[1] & (COMPONENT_LIVE | COMPONENT_LAST_VALID),
    )
}

/// Round 12 review, N1: evicting an honest member before K changes nothing of its contract's
/// settlement. Unevicted, H's weight puts the median at its +10%; evicted the day after the
/// freeze, the same. The reviewer's variant: D retires after the freeze, so H, B and C are the
/// three eligible origins; evicting H keeps them three, and the component settles live.
#[tokio::test]
async fn an_evicted_member_settles_as_if_still_listed() {
    for d_retires in [false, true] {
        let kept = evicted_member_settlement(false, d_retires).await;
        assert_eq!(evicted_member_settlement(true, d_retires).await, kept);
        assert_eq!(kept.3, COMPONENT_LIVE);
        if d_retires {
            assert_eq!(kept.1, 3);
        } else {
            assert_eq!(kept.0, 100_000);
        }
    }
}

// Round 13 review, 2026-10-30
// ------------------------------------------------------------------------------------------

/// Round 13 review, finding 1, the reviewer's churn sequence: 36 honest sources list a contract;
/// 60 junk sources join its window (truthful duplicates of one of them), lose their backing, and a
/// cooldown later, before the freeze, give up their slots one by one to honest replacements of
/// their own origins. A joiner evicted before the freeze leaves the contract and frees its join:
/// every replacement joins, the joins end at the replacements' 60, all 96 count at the freeze, and
/// settlement records the 36 listed and the 60 replacements, none of the junk.
#[tokio::test]
async fn evicted_joiners_free_their_joins_for_honest_replacements() {
    use ameba_oracle::state::heads::CAPTURE_MEMBER;
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(400_000 * AMBA, 0).await;
    let b = env.user(500_000 * AMBA, 0).await;
    let f = env.user(0, 10_000 * USDC).await;
    env.h.fund(&a.pubkey(), 10_000_000_000).await;
    let mut honest = Vec::new();
    let mut fills = Vec::new();
    for k in 0..36 {
        let id = env.active_head(&a, &b, 1).await;
        fills.push(env.fill(&f, id, 1_000_000, env.now, adverse(k)).await);
        honest.push(id);
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let dec = DEC_01_2026;
    env.create_period(env.now + DAY, dec).await;
    env.warp(env.now + DAY).await;
    env.list(dec, 50 * USDC).await;
    let contract = env.contract(dec).await;
    let (slot, at) = (
        usize::from(contract.ring_slot),
        usize::from(contract.ring_slot) * 24 + 1,
    );
    let mut junk = Vec::new();
    let mut fills = Vec::new();
    for _ in 0..60 {
        let id = env.active_head(&a, &b, 1).await;
        fills.push(env.fill(&f, id, 1_000_000, env.now, adverse(0)).await);
        junk.push(id);
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    assert_eq!(env.board().await.contract_joined[at], 60);
    let start = env.now;
    for &id in &junk {
        env.undelegate(&b, id, 1_500 * AMBA).await;
    }
    env.warp(start + env.params.cooldown_secs - DAY).await;
    let mut standby = Vec::new();
    for k in 0..60 {
        let id = env.active_head(&a, &b, 1).await;
        let p = env.fill(&f, id, 1_000_000, env.now, adverse(100 + k)).await;
        standby.push((id, p));
    }
    env.warp(start + env.params.cooldown_secs).await;
    let mut replacements = Vec::new();
    for (&id, (h, p)) in junk.iter().zip(standby) {
        let ix = env.expire_ix(id);
        env.send(ix, &[]).await.unwrap();
        let evicted = env.head(id).await;
        assert_ne!(evicted.evicted_ts, 0);
        assert_eq!(evicted.capture_flags[slot] & CAPTURE_MEMBER, 0);
        env.finalize(p, &f.pubkey()).await;
        let joined = env.head(h).await;
        assert_eq!(joined.status, HeadStatus::Live);
        assert_ne!(joined.capture_flags[slot] & CAPTURE_MEMBER, 0);
        replacements.push(h);
    }
    assert_eq!(env.board().await.contract_joined[at], 60);
    let freeze = env.contract(dec).await.freeze_ts;
    assert!(env.now < freeze);
    env.warp(freeze + 3_600).await;
    let normal = env.board().await.params;
    env.set_params_measured(normal, "SetBoardParams").await;
    assert_eq!(env.board().await.contract_freeze_origins[at], 96);
    env.settle(dec, 1, &[]).await;
    let (_, sources) = env.weights(dec, 1).await.unwrap();
    assert_eq!(sources.len(), 96);
    assert!(honest
        .iter()
        .chain(&replacements)
        .all(|h| sources.iter().any(|s| s.head_id == *h)));
    assert_eq!(env.contract(dec).await.component_eligible[1], 96);
}

/// Round 13 review, CU at full size, the largest population that stays eligible at once: the
/// contract lists 96 sources, which update to descending values, lose their backing and a cooldown
/// later give up their slots (still members, eligible, weighing as listed); 96 joiners take them
/// (one with 2,000,000 AMBA behind it) and update to descending values too, all before the freeze
/// window. The large joiner retires after the freeze; the settlement weight threshold (all 192 at
/// full backing) is out of the live data's reach but not the freeze's: the completing call sorts
/// 191 live changes, falls back, and sorts the 192 freeze changes.
#[tokio::test]
async fn a_retained_population_settles_at_full_size() {
    let mut params = test_params();
    params.max_strikes = 1;
    let mut env = bench_env(params).await;
    let a = env.user(600_000 * AMBA, 0).await;
    let b = env.user(3_000_000 * AMBA, 0).await;
    let f = env.user(0, 20_000 * USDC).await;
    let x = env.user(AMBA, 1_000 * USDC).await;
    env.h.fund(&a.pubkey(), 10_000_000_000).await;
    let mut cu: Vec<(&str, u64)> = Vec::new();
    let even = [(&b, 1_500 * AMBA)];
    let (listed, _) = env.full_board(&a, &b, &f, &even, &even).await;
    let normal = env.board().await.params;
    let mut threshold = normal;
    threshold.min_settle_weight = 192 * 2_500 * AMBA;
    env.set_params_measured(threshold, "SetBoardParams").await;
    let dec = DEC_01_2026;
    env.list_measured(dec, env.now + 2 * DAY, "ListContract")
        .await;
    env.set_params_measured(normal, "SetBoardParams").await;
    let mut updates = Vec::new();
    for (k, &id) in listed.iter().enumerate() {
        let value = 2_000_000 - 10_000 * k as u64;
        updates.push(env.update(&f, id, value, env.now, adverse(k)).await);
    }
    env.warp(env.now + DAY).await;
    for p in updates {
        env.finalize(p, &f.pubkey()).await;
    }
    let start = env.now;
    for &id in &listed {
        env.undelegate(&b, id, 1_500 * AMBA).await;
    }
    env.warp(start + env.params.cooldown_secs - DAY).await;
    let mut standby = Vec::new();
    for k in 0..96 {
        let id = env.active_head(&a, &b, 1).await;
        if k == 95 {
            env.delegate(&b, id, 2_000_000 * AMBA).await;
        }
        let p = env
            .fill(&f, id, 1_000_000, env.now, adverse(1_000 + k))
            .await;
        standby.push((id, p));
    }
    env.warp(start + env.params.cooldown_secs).await;
    let mut joiners = Vec::new();
    for (&id, (h, p)) in listed.iter().zip(standby) {
        let ix = env.expire_ix(id);
        env.send(ix, &[]).await.unwrap();
        env.finalize(p, &f.pubkey()).await;
        joiners.push(h);
    }
    assert_eq!(env.board().await.live_count, 96);
    let seen = env.now;
    let mut updates = Vec::new();
    for (k, &id) in joiners.iter().enumerate() {
        let value = 1_500_000 - 5_000 * k as u64;
        updates.push(env.update(&f, id, value, seen, adverse(1_000 + k)).await);
    }
    assert!(seen < env.freeze_window(dec).await);
    let freeze = env.contract(dec).await.freeze_ts;
    env.warp(freeze + 3_600).await;
    for p in updates {
        env.finalize(p, &f.pubkey()).await;
    }
    let large = joiners[95];
    env.uphold_measured(large, &x, "FinalizeSourceChallenges")
        .await;
    assert!(env.now < dec + env.params.challenge_window_secs);
    let chunks = env.settle(dec, 1, &[large]).await;
    let completing = chunks.last().map_or(0, |(_, units)| *units);
    let most = chunks.iter().map(|(_, units)| *units).max().unwrap_or(0);
    cu.push((
        "SettleComponent: the completing call of 192 eligible members (96 evicted), the fallback",
        completing,
    ));
    cu.push(("SettleComponent: the heaviest call", most));
    let (_, sources) = env.weights(dec, 1).await.unwrap();
    assert_eq!(sources.len(), 192);
    let contract = env.contract(dec).await;
    assert_eq!(contract.component_degraded[1], DEGRADED_TOO_LITTLE_WEIGHT);
    assert_ne!(contract.component_flags[1] & COMPONENT_LAST_VALID, 0);
    assert_eq!(contract.component_eligible[1], 191);
    assert_ne!(contract.component_change_ppm[1], 0);
    report(&cu);
}

// Round 14 review, 2026-10-30
// ------------------------------------------------------------------------------------------

/// Round 14 review, N1, the reviewer's sequence: on a list of 95 listed sources, H joins the
/// contract's window backed by D's 1,500 AMBA (and its proposer's 1,000); D undelegates, H is
/// unbacked, and a cooldown later, before the freeze, H is evicted: it leaves the contract (a
/// joiner whose freeze is ahead) and stays Live. D syncs then (`eager`) or not; a source case on H
/// opens after the eviction, or before it (`case_first`), and is upheld. Returns what D lost of
/// its stake once everything cooled and came back (its delegation closed).
async fn departed_joiner_case(eager: bool, case_first: bool) -> u64 {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(300_000 * AMBA, 0).await;
    let b = env.user(300_000 * AMBA, 0).await;
    let d = env.user(2_000 * AMBA, 0).await;
    let f = env.user(0, 10_000 * USDC).await;
    let x = env.user(AMBA, 1_000 * USDC).await;
    env.h.fund(&a.pubkey(), 10_000_000_000).await;
    let staked = env.stake_of(&d.pubkey()).await.free;
    let mut fills = Vec::new();
    for k in 0..95 {
        let id = env.active_head(&a, &b, 1).await;
        fills.push(env.fill(&f, id, 1_000_000, env.now, adverse(k)).await);
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let dec = DEC_01_2026;
    env.create_period(env.now + DAY, dec).await;
    env.warp(env.now + DAY).await;
    env.list(dec, 50 * USDC).await;
    let h = env.post(&a, 1, 1_000 * AMBA).await;
    env.delegate(&d, h, 1_500 * AMBA).await;
    let p = env.fill(&f, h, 1_000_000, env.now, adverse(95)).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    let k = env.contract(dec).await;
    assert!(heads_view::joined(&k, &*env.head(h).await));
    assert_eq!(env.board().await.live_count, 96);
    env.warp(env.now + DAY).await;
    env.undelegate(&d, h, 1_500 * AMBA).await;
    let start = env.now;
    if case_first {
        env.warp(start + env.params.cooldown_secs - 3_600).await;
        let ix = env.source_challenge_ix(h, &x);
        env.send(ix, &[&x]).await.unwrap();
    }
    env.warp(start + env.params.cooldown_secs).await;
    let ix = env.expire_ix(h);
    env.send(ix, &[]).await.unwrap();
    let evicted = env.head(h).await;
    assert_eq!(evicted.status, HeadStatus::Live);
    assert_eq!(env.board().await.live_count, 95);
    assert!(!heads_view::member(&k, &evicted));
    assert!(env.now < k.freeze_ts);
    if eager {
        env.sync(h, &d.pubkey()).await;
    }
    if !case_first {
        env.warp(env.now + 3_600).await;
        let ix = env.source_challenge_ix(h, &x);
        env.send(ix, &[&x]).await.unwrap();
    }
    env.uphold_source_case(h, &x).await;
    assert_eq!(env.head(h).await.strike_count, 1);
    // D syncs until its delegation closes, empty (escrow comes back as what it answers settles).
    let key = find_head_delegation_address(&program(), &env.head_key(h), &d.pubkey()).0;
    for _ in 0..4 {
        if env
            .h
            .ctx
            .banks_client
            .get_account(key)
            .await
            .unwrap()
            .is_none()
        {
            break;
        }
        env.sync(h, &d.pubkey()).await;
        env.warp(env.now + env.params.cooldown_secs).await;
    }
    assert!(env
        .h
        .ctx
        .banks_client
        .get_account(key)
        .await
        .unwrap()
        .is_none());
    staked - env.stake_of(&d.pubkey()).await.free
}

/// Round 14 review, N1: a case opened after a source left a contract answers for nothing it left,
/// so its delegators' stake ends the same whenever they sync. A case opened after the eviction
/// strikes none of D's stake, synced at the eviction or not (before the fix, unsynced, D's stake
/// still held for the contract H left paid 10%); one open at the departure keeps its exposure:
/// D's stake held for that contract pays it, synced or not.
#[tokio::test]
async fn a_case_after_a_source_left_a_contract_strikes_no_stake_held_for_it() {
    for case_first in [false, true] {
        let eager = departed_joiner_case(true, case_first).await;
        assert_eq!(departed_joiner_case(false, case_first).await, eager);
        let slash = liability::slash(1_500 * AMBA, test_params().slash_rate_bps);
        assert_eq!(eager, if case_first { slash } else { 0 });
    }
}

/// Round 15 review, the proposer's variant of N1: the reviewer's sequence with H's own proposer P
/// (1,000 AMBA) and D (1,500) behind it, both leaving it entirely; a cooldown later, before the
/// freeze, H is evicted and leaves the contract. P and D sync never (`when` 0), at the eviction
/// (1), or just after the case opens (2: P's release then precedes the case's finalization). The
/// case opens after the eviction, or before it (`case_first`), and is upheld. Returns what P and
/// D lost of their stake once everything came back (their delegations closed).
async fn departed_proposer_case(when: u8, case_first: bool) -> (u64, u64) {
    let mut env = small_env(test_params(), HeadsMixBasis::Assumed).await;
    let a = env.user(300_000 * AMBA, 0).await;
    let b = env.user(300_000 * AMBA, 0).await;
    let p = env.user(2_000 * AMBA, 0).await;
    let d = env.user(2_000 * AMBA, 0).await;
    let f = env.user(0, 10_000 * USDC).await;
    let x = env.user(AMBA, 1_000 * USDC).await;
    env.h.fund(&a.pubkey(), 10_000_000_000).await;
    env.h.fund(&p.pubkey(), 1_000_000_000).await;
    let staked = [
        env.stake_of(&p.pubkey()).await.free,
        env.stake_of(&d.pubkey()).await.free,
    ];
    let mut fills = Vec::new();
    for k in 0..95 {
        let id = env.active_head(&a, &b, 1).await;
        fills.push(env.fill(&f, id, 1_000_000, env.now, adverse(k)).await);
    }
    env.warp(env.now + DAY).await;
    for q in fills {
        env.finalize(q, &f.pubkey()).await;
    }
    let dec = DEC_01_2026;
    env.create_period(env.now + DAY, dec).await;
    env.warp(env.now + DAY).await;
    env.list(dec, 50 * USDC).await;
    let h = env.post(&p, 1, 1_000 * AMBA).await;
    env.delegate(&d, h, 1_500 * AMBA).await;
    let q = env.fill(&f, h, 1_000_000, env.now, adverse(95)).await;
    env.warp(env.now + DAY).await;
    env.finalize(q, &f.pubkey()).await;
    let k = env.contract(dec).await;
    assert!(heads_view::joined(&k, &*env.head(h).await));
    env.warp(env.now + DAY).await;
    env.undelegate(&p, h, 1_000 * AMBA).await;
    env.undelegate(&d, h, 1_500 * AMBA).await;
    assert!(env.delegation_of(h, &p.pubkey()).await.is_proposer);
    let start = env.now;
    let owners = [p.pubkey(), d.pubkey()];
    let keys = owners.map(|o| find_head_delegation_address(&program(), &env.head_key(h), &o).0);
    let open_case = |env: &mut Env| env.source_challenge_ix(h, &x);
    if case_first {
        env.warp(start + env.params.cooldown_secs - 3_600).await;
        let ix = open_case(&mut env);
        env.send(ix, &[&x]).await.unwrap();
        if when == 2 {
            for o in &owners {
                env.sync(h, o).await;
            }
        }
    }
    env.warp(start + env.params.cooldown_secs).await;
    let ix = env.expire_ix(h);
    env.send(ix, &[]).await.unwrap();
    let evicted = env.head(h).await;
    assert_eq!(evicted.status, HeadStatus::Live);
    assert!(!heads_view::member(&k, &evicted));
    assert!(env.now < k.freeze_ts);
    if when == 1 {
        for o in &owners {
            env.sync(h, o).await;
        }
    }
    if !case_first {
        env.warp(env.now + 3_600).await;
        let ix = open_case(&mut env);
        env.send(ix, &[&x]).await.unwrap();
        if when == 2 {
            for o in &owners {
                env.sync(h, o).await;
            }
        }
    }
    env.uphold_source_case(h, &x).await;
    assert_eq!(env.head(h).await.strike_count, 1);
    // Each syncs until its delegation closes, empty.
    for (o, key) in owners.iter().zip(keys) {
        for _ in 0..4 {
            if env
                .h
                .ctx
                .banks_client
                .get_account(key)
                .await
                .unwrap()
                .is_none()
            {
                break;
            }
            env.sync(h, o).await;
            env.warp(env.now + env.params.cooldown_secs).await;
        }
        assert!(env
            .h
            .ctx
            .banks_client
            .get_account(key)
            .await
            .unwrap()
            .is_none());
    }
    (
        staked[0] - env.stake_of(&p.pubkey()).await.free,
        staked[1] - env.stake_of(&d.pubkey()).await.free,
    )
}

/// Round 15 review: a falling case takes of the proposer's held stake only what answers it (the
/// membership rule every delegator's held stake follows), so the proposer's loss does not hang on
/// when it syncs either. A case opened after H left the contract takes nothing from P, synced
/// never, at the eviction, or after the case opened (before the fix the lazy P's 1,000 went to the
/// winners at the 100% proposer rate, where the synced one had it back); one open at the
/// departure takes all of P's (its rate, 100%) and D's 10%, whenever they sync.
#[tokio::test]
async fn a_case_after_a_source_left_a_contract_takes_no_proposer_stake_held_for_it() {
    let rate = test_params().slash_rate_bps;
    for case_first in [false, true] {
        let expected = if case_first {
            (1_000 * AMBA, liability::slash(1_500 * AMBA, rate))
        } else {
            (0, 0)
        };
        for when in 0..3 {
            assert_eq!(
                departed_proposer_case(when, case_first).await,
                expected,
                "sync {when}, case first {case_first}"
            );
        }
    }
}

/// Round 16 review, N1: H's proposer P has 10,000 AMBA delegated and D 1,500 when a case on H
/// opens; during it each tops up (P 5,000, D 1,000), and keeps the top-up delegated or withdraws
/// it (`withdraw`) before the case falls. Before the freeze (`after_freeze` false) the top-up
/// weighs in the contract the case snapshotted, so the withdrawn part's mask meets the case's;
/// after it, it weighs in none. Returns what the case took from P, what it slashed of D, and
/// P's escrow right after its withdrawal.
async fn top_up_during_a_case(after_freeze: bool, withdraw: bool) -> (u64, u64, u64) {
    let mut params = test_params();
    params.min_live_sources = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let p = env.user(20_000 * AMBA, 0).await;
    let d = env.user(20_000 * AMBA, 0).await;
    let f = env.user(0, 100 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let h = env.post(&p, 1, 10_000 * AMBA).await;
    env.delegate(&d, h, 1_500 * AMBA).await;
    let q = env.fill(&f, h, 1_000_000, env.now, origin("one")).await;
    env.warp(env.now + DAY).await;
    env.finalize(q, &f.pubkey()).await;
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    let k = env.contract(expiry).await;
    let opens = if after_freeze {
        k.freeze_ts + 3_600
    } else {
        env.now + DAY
    };
    env.warp(opens).await;
    let ix = env.source_challenge_ix(h, &x);
    env.send(ix, &[&x]).await.unwrap();
    let slot = usize::from(k.ring_slot);
    assert_eq!(
        env.head(h).await.source_case_open_gens[slot],
        env.board().await.contract_gens[slot]
    );
    env.warp(opens + 3_600).await;
    env.delegate(&p, h, 5_000 * AMBA).await;
    env.delegate(&d, h, 1_000 * AMBA).await;
    assert_eq!(env.delegation_of(h, &p.pubkey()).await.fresh, 5_000 * AMBA);
    let mut escrow = 0;
    if withdraw {
        env.warp(opens + 7_200).await;
        env.undelegate(&p, h, 5_000 * AMBA).await;
        env.undelegate(&d, h, 1_000 * AMBA).await;
        let board = env.board().await;
        let head = env.head(h).await;
        let held = env.delegation_of(h, &p.pubkey()).await;
        assert_eq!(held.held, 5_000 * AMBA);
        escrow = held.held_escrow(&board.params.to_params(), &board, &head);
    }
    env.uphold_source_case(h, &x).await;
    let taken = env.delegation_of(h, &p.pubkey()).await.proposer_slashed;
    env.sync(h, &d.pubkey()).await;
    (
        taken,
        env.delegation_of(h, &d.pubkey()).await.slashed_total,
        escrow,
    )
}

/// Round 16 review, N1: the proposer answers for everything it delegated during its case, at its
/// rate, whether it keeps a mid-case top-up delegated or withdraws it before the case falls, and
/// whether the withdrawn part's contracts meet the case's or not; its escrow keeps the
/// withdrawn top-up meanwhile. An ordinary delegator's top-up stays exempt either way (before the
/// fix the withdrawn top-up was exempt for the proposer too: the case took 10,000, not 15,000).
#[tokio::test]
async fn a_proposer_answers_for_a_top_up_withdrawn_during_its_case() {
    let slash = liability::slash(1_500 * AMBA, test_params().slash_rate_bps);
    for after_freeze in [false, true] {
        let kept = top_up_during_a_case(after_freeze, false).await;
        assert_eq!(kept, (15_000 * AMBA, slash, 0));
        let withdrawn = top_up_during_a_case(after_freeze, true).await;
        assert_eq!(
            withdrawn,
            (15_000 * AMBA, slash, 5_000 * AMBA),
            "after the freeze: {after_freeze}"
        );
    }
}

/// Three listed sources of distinct origins at 1.00 (`max_strikes` 1): returns the env, the
/// fetcher, a challenger, the heads and the contract's expiry.
async fn three_listed_sources() -> (Env, Keypair, Keypair, Vec<u32>, u64) {
    let mut params = test_params();
    params.max_strikes = 1;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let a = env.user(20_000 * AMBA, 0).await;
    let b = env.user(20_000 * AMBA, 0).await;
    let f = env.user(1_000 * AMBA, 1_000 * USDC).await;
    let x = env.user(AMBA, 100 * USDC).await;
    let mut ids = Vec::new();
    let mut fills = Vec::new();
    for j in 0..3 {
        let id = env.active_head(&a, &b, 1).await;
        ids.push(id);
        fills.push(
            env.fill(&f, id, 1_000_000, env.now, origin(&format!("o{j}")))
                .await,
        );
    }
    env.warp(env.now + DAY).await;
    for p in fills {
        env.finalize(p, &f.pubkey()).await;
    }
    let expiry = DEC_01_2026;
    env.create_period(env.now + DAY, expiry).await;
    env.warp(env.now + DAY).await;
    env.list(expiry, 50 * USDC).await;
    (env, f, x, ids, expiry)
}

impl Env {
    /// The contract indication now: its index change, its item's reference change and its
    /// Contract component's reading.
    async fn indication(&mut self, expiry: u64) -> (i64, i64, heads_view::Reading) {
        let view = self.view_of(expiry).await;
        let k = &view.contracts[0];
        let index = match &k.index {
            heads_view::IndexReading::Indicative { change_ppm, .. } => *change_ppm,
            other => panic!("{other:?}"),
        };
        let reference = match &k.items[0].reference {
            heads_view::ReferenceView::Available { change_ppm, .. } => *change_ppm,
            other => panic!("{other:?}"),
        };
        (index, reference, k.components[1].reading.clone())
    }

    /// Settles every component past the cutoff K and finalizes the contract: its index change,
    /// its item's reference change and its Contract component's.
    async fn settle_all(&mut self, expiry: u64) -> (i64, i64, i64) {
        let cutoff = expiry + self.params.challenge_window_secs;
        if self.now <= cutoff {
            self.warp(cutoff + 1).await;
        }
        for component in 0..3u8 {
            self.settle(expiry, component, &[]).await;
        }
        self.send(
            hx::finalize_contract(&program(), &self.product, expiry),
            &[],
        )
        .await
        .unwrap();
        let k = self.contract(expiry).await;
        (
            k.index_change_ppm,
            k.item_reference_ppm[0],
            k.component_change_ppm[1],
        )
    }
}

/// Round 14 review, N2, scenario A: the three members print 1.10 over the freeze level's window
/// (freeze levels of +10% on openings of 1.00); after the freeze a source case retires one, two
/// eligible origins are left, and the Contract component is on its degraded path, at its freeze
/// fallback, +10%. Before the expiry the indication already counts it there, as settlement then
/// does (before the fix: at zero).
#[tokio::test]
async fn the_indication_counts_a_degraded_component_at_its_freeze_fallback() {
    let (mut env, f, x, ids, expiry) = three_listed_sources().await;
    let window = env.freeze_window(expiry).await;
    let freeze = env.contract(expiry).await.freeze_ts;
    env.warp(window + 60).await;
    let mut claims = Vec::new();
    for &id in &ids {
        let o = env.head(id).await.origin_id;
        claims.push(env.update(&f, id, 1_100_000, env.now, o).await);
    }
    env.warp(env.now + DAY + 60).await;
    for p in claims {
        env.finalize(p, &f.pubkey()).await;
    }
    env.warp(freeze + 3_600).await;
    env.lose_source_case(ids[2], &x).await;
    assert_eq!(env.head(ids[2]).await.status, HeadStatus::Retired);
    assert!(env.now < expiry);
    let (index, reference, reading) = env.indication(expiry).await;
    assert_eq!(
        reading,
        heads_view::Reading::Degraded {
            reason: settle::Degraded::TooFewOrigins
        }
    );
    let settled = env.settle_all(expiry).await;
    assert_eq!(settled.2, 100_000);
    assert_eq!((index, reference), (settled.0, settled.1));
    // The Contract-channel target: the index is the Contract component's.
    assert_eq!(index, 100_000);
}

/// Round 14 review, N2, scenario B: the three members print 2.00 between the third and fourth
/// settlement-window instants, so each samples [1.00, 1.00, 1.00, 2.00, 2.00] there: a level of
/// 1.00, 0% on its opening, though its latest print is 2.00. Past the cutoff K, with no claim
/// open, the indication reads 0% as settlement does (before the fix: +100%, the latest prints).
#[tokio::test]
async fn the_indication_past_the_freeze_reads_the_sampled_levels() {
    let (mut env, f, _, ids, expiry) = three_listed_sources().await;
    let freeze = env.contract(expiry).await.freeze_ts;
    // The settlement window's third instant (`samples::instants`).
    let third = freeze + (expiry - freeze) * 3 / 5;
    env.warp(third + 60).await;
    let mut claims = Vec::new();
    for &id in &ids {
        let o = env.head(id).await.origin_id;
        claims.push(env.update(&f, id, 2_000_000, env.now, o).await);
    }
    env.warp(env.now + DAY + 60).await;
    for p in claims {
        env.finalize(p, &f.pubkey()).await;
    }
    env.warp(expiry + env.params.challenge_window_secs + 1)
        .await;
    for &id in &ids {
        let head = env.head(id).await;
        assert_eq!(head.value, 2_000_000);
        assert_eq!(head.finalized_reveals, head.reveals);
    }
    let (index, reference, reading) = env.indication(expiry).await;
    let settled = env.settle_all(expiry).await;
    assert_eq!(settled.2, 0);
    match reading {
        heads_view::Reading::Live {
            change_ppm,
            eligible,
            ..
        } => assert_eq!((change_ppm, eligible), (0, 3)),
        other => panic!("{other:?}"),
    }
    assert_eq!((index, reference), (settled.0, settled.1));
}

/// The order of `0..n` that makes `median::sorted_positions` compare the most: every merge of two
/// runs interleaves them to its last slot (top down, the largest keys alternate between the two
/// runs, then each run's share is spread over its own runs the same way).
fn interleaved_ranks(n: usize) -> Vec<usize> {
    fn assign(ranks: &mut [usize], s: usize, e: usize, w: usize, keys: &[usize]) {
        if e - s == 1 {
            ranks[s] = keys[0];
            return;
        }
        let mut h = w / 2;
        while s + h >= e {
            h /= 2;
        }
        let mid = s + h;
        let (a, b) = (mid - s, e - mid);
        let (mut left, mut right) = (Vec::new(), Vec::new());
        let mut to_right = true;
        for &k in keys.iter().rev() {
            if to_right && right.len() < b {
                right.push(k);
                to_right = false;
            } else if !to_right && left.len() < a {
                left.push(k);
                to_right = true;
            } else if right.len() < b {
                right.push(k);
            } else {
                left.push(k);
            }
        }
        left.reverse();
        right.reverse();
        assign(ranks, s, mid, h, &left);
        assign(ranks, mid, e, h, &right);
    }
    let mut ranks = vec![0; n];
    let keys: Vec<usize> = (0..n).collect();
    assign(&mut ranks, 0, n, n.next_power_of_two(), &keys);
    ranks
}

impl Env {
    /// The smallest heap frame `ix` runs in, in KiB (the runtime's unit), simulated on the
    /// artifact in place, with what fails a KiB below it. The allocator is a bump allocator that
    /// never frees or reuses, so the frame bounds everything allocated and written to up to the
    /// last write: the instruction's heap high-water mark, within a KiB (an allocation never
    /// written to is free).
    async fn heap_floor(&mut self, ix: Instruction) -> (u32, String) {
        let ix = self.admit(ix);
        let payer = self.h.payer();
        let blockhash = self.h.ctx.get_new_latest_blockhash().await.unwrap();
        let run = |kib: u32| {
            let tx = Transaction::new_signed_with_payer(
                &[
                    ComputeBudgetInstruction::request_heap_frame(kib * 1024),
                    ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
                    ix.clone(),
                ],
                Some(&payer.pubkey()),
                &[&payer],
                blockhash,
            );
            self.h.ctx.banks_client.simulate_transaction(tx)
        };
        let full = run(256).await.unwrap();
        assert_eq!(full.result, Some(Ok(())), "{:?}", full.simulation_details);
        let (mut lo, mut hi, mut below) = (32u32, 256u32, String::new());
        while lo < hi {
            let mid = (lo + hi) / 2;
            let sim = run(mid).await.unwrap();
            if sim.result == Some(Ok(())) {
                hi = mid;
            } else {
                lo = mid + 1;
                let last = sim.simulation_details.and_then(|d| {
                    d.logs
                        .iter()
                        .rev()
                        .find(|l| !l.contains("consumed"))
                        .cloned()
                });
                below = format!("{:?}: {}", sim.result, last.unwrap_or_default());
            }
        }
        (hi, below)
    }
}

/// Round 14 review, CU and heap: the retained population (`a_retained_population_settles_at_full_size`)
/// with every ring slot holding a contract (January, February and March list on the joiners), and
/// keys that make every merge interleave to its end (`interleaved_ranks`): the 192 records' levels
/// and origins in settlement order, so both eligibility passes' origin sorts and the fallback's
/// value sort compare the most they can (1,281 comparisons at 192, against 704 for the descending
/// values before). The completing call records the last eight heads, sorts the 191 live origins,
/// falls back on weight, sorts the 192 freeze origins and the 192 freeze changes. Measures its CU,
/// and on the production artifact the smallest heap frame it runs in (`heap_floor`).
#[tokio::test]
async fn a_retained_population_settles_with_four_contracts_and_interleaved_keys() {
    let mut params = test_params();
    params.max_strikes = 1;
    let mut env = bench_env(params).await;
    let a = env.user(600_000 * AMBA, 0).await;
    let b = env.user(3_000_000 * AMBA, 0).await;
    let f = env.user(0, 20_000 * USDC).await;
    let x = env.user(AMBA, 1_000 * USDC).await;
    env.h.fund(&a.pubkey(), 10_000_000_000).await;
    let rank = interleaved_ranks(192);
    let compares = std::cell::Cell::new(0);
    median::sorted_positions(192, &|i, j| {
        compares.set(compares.get() + 1);
        rank[i] < rank[j]
    });
    assert_eq!(compares.get(), 1_281);
    // Record `p`'s level and origin.
    let key = |p: usize| (1_010_000 + 5_000 * rank[p] as u64, adverse(2_000 + rank[p]));
    let mut cu: Vec<(&str, u64)> = Vec::new();
    let even = [(&b, 1_500 * AMBA)];
    let (listed, _) = env.full_board(&a, &b, &f, &even, &even).await;
    let normal = env.board().await.params;
    let mut threshold = normal;
    threshold.min_settle_weight = 192 * 2_500 * AMBA;
    env.set_params_measured(threshold, "SetBoardParams").await;
    let dec = DEC_01_2026;
    env.list_measured(dec, env.now + 2 * DAY, "ListContract")
        .await;
    env.set_params_measured(normal, "SetBoardParams").await;
    // Settlement records the listed sources first, in listing order.
    let first = env.component_sources(dec, 1, &[]).await;
    assert_eq!(first.len(), 96);
    let mut updates = Vec::new();
    for (p, &id) in first.iter().enumerate() {
        let (value, o) = key(p);
        updates.push(env.update(&f, id, value, env.now, o).await);
    }
    env.warp(env.now + DAY).await;
    for p in updates {
        env.finalize(p, &f.pubkey()).await;
    }
    let start = env.now;
    for &id in &listed {
        env.undelegate(&b, id, 1_500 * AMBA).await;
    }
    env.warp(start + env.params.cooldown_secs - DAY).await;
    let mut standby = Vec::new();
    for k in 0..96 {
        let id = env.active_head(&a, &b, 1).await;
        if k == 95 {
            env.delegate(&b, id, 2_000_000 * AMBA).await;
        }
        let p = env
            .fill(&f, id, 1_000_000, env.now, adverse(1_000 + k))
            .await;
        standby.push((id, p));
    }
    env.warp(start + env.params.cooldown_secs).await;
    let mut joiners = Vec::new();
    for (&id, (h, p)) in listed.iter().zip(standby) {
        let ix = env.expire_ix(id);
        env.send(ix, &[]).await.unwrap();
        env.finalize(p, &f.pubkey()).await;
        joiners.push(h);
    }
    assert_eq!(env.board().await.live_count, 96);
    // Then the joiners, in live-list order, the large one (retired by then) last.
    let large = joiners[95];
    let mut order = env.component_sources(dec, 1, &[]).await;
    order.retain(|&id| id != large);
    order.push(large);
    assert_eq!(order.len(), 192);
    assert_eq!(order[..96], first[..]);
    let seen = env.now;
    let mut updates = Vec::new();
    for (p, &id) in order.iter().enumerate().skip(96) {
        let (value, o) = key(p);
        updates.push(env.update(&f, id, value, seen, o).await);
    }
    assert!(seen < env.freeze_window(dec).await);
    // Every ring slot holds a contract.
    for (expiry, at) in [
        (JAN_01_2027, NOV_01_2026 + 9 * DAY),
        (FEB_01_2027, NOV_01_2026 + 14 * DAY),
        (MAR_01_2027, NOV_01_2026 + 18 * DAY),
    ] {
        assert!(env.now < at - DAY);
        env.list_measured(expiry, at, "ListContract").await;
    }
    let board = env.board().await;
    assert!((0..4).all(|i| board.unsettled(i, board.contract_gens[i])));
    let freeze = env.contract(dec).await.freeze_ts;
    env.warp(freeze + 3_600).await;
    for p in updates {
        env.finalize(p, &f.pubkey()).await;
    }
    env.uphold_measured(large, &x, "FinalizeSourceChallenges")
        .await;
    assert!(env.now < dec + env.params.challenge_window_secs);
    let ids = env.component_sources(dec, 1, &[large]).await;
    assert_eq!(ids, order);
    env.warp(dec + env.params.challenge_window_secs + 1).await;
    let payer = env.h.ctx.payer.pubkey();
    let (mut most, mut completing, mut heap) = (0, 0, (0, String::new()));
    env.production_on().await;
    for (n, chunk) in ids.chunks(HEADS_SETTLE_MAX_BATCH).enumerate() {
        let first = (n * HEADS_SETTLE_MAX_BATCH) as u16;
        let ix =
            hx::settle_component(&program(), &payer, &env.product, dec, 1, first, chunk).unwrap();
        if (n + 1) * HEADS_SETTLE_MAX_BATCH >= ids.len() {
            heap = env.heap_floor(ix.clone()).await;
        }
        completing = env.measure("SettleComponent", ix, &[]).await;
        most = most.max(completing);
    }
    env.production_off().await;
    cu.push((
        "SettleComponent: the completing call, 192 members, four contracts, interleaved keys",
        completing,
    ));
    cu.push(("SettleComponent: the heaviest call", most));
    eprintln!(
        "HEAP SettleComponent, the completing call: runs in a {} KiB frame (at {} KiB: {})",
        heap.0,
        heap.0 - 1,
        heap.1
    );
    let (_, sources) = env.weights(dec, 1).await.unwrap();
    assert_eq!(sources.len(), 192);
    let contract = env.contract(dec).await;
    assert_eq!(contract.component_degraded[1], DEGRADED_TOO_LITTLE_WEIGHT);
    assert_ne!(contract.component_flags[1] & COMPONENT_LAST_VALID, 0);
    assert_eq!(contract.component_eligible[1], 191);
    report(&cu);
}

// Round 15 review, 2026-10-31
// ------------------------------------------------------------------------------------------

/// A benchmark's board whose one item settles on its LTA channel alone (mix 0 / 0 / 100%, the
/// fixed-mix reference gating only that channel).
async fn bench_lta_env(params: HeadsParams) -> Env {
    let (h, elfs) = bench_harness().await;
    let items = vec![hx::HeadsItemParams {
        bucket_id: product("TESTX-ITEM"),
        basket_weight_bps: 10_000,
        mix_bps: [0, 0, 10_000],
        mix_basis: HeadsMixBasis::Assumed,
    }];
    let sentences = (0..3u8)
        .map(|ch| {
            (
                ch,
                0,
                ch,
                format!("USD per chip, TESTX item, channel {ch}, as of the observation date"),
            )
        })
        .collect();
    let mut env = env_on(h, "TESTX", params, items, sentences).await;
    env.production = elfs;
    env.settle_chunk = HEADS_SETTLE_MAX_BATCH;
    env
}

impl Env {
    /// Lists `expiry` at `at` (its period made the day before) on the fixed-mix reference.
    async fn list_reference(&mut self, expiry: u64, at: u64) {
        self.warp(at - DAY).await;
        self.create_period(self.now + DAY, expiry).await;
        self.warp(at).await;
        let terms = self.terms(expiry).await;
        let ix = self.list_ix(
            expiry,
            HeadsSettlementTarget::FixedMixReference,
            terms,
            50 * USDC,
        );
        let oracle = self.h.oracle_authority.insecure_clone();
        self.measure("ListContract", ix, &[&oracle]).await;
    }
}

/// Round 15 review, CU: the retained population (`a_retained_population_settles_with_four_
/// contracts_and_interleaved_keys`) on the LTA channel, with what that fixture missed. The large
/// joiner stays unretired but its LTA price resets just after the freeze, so it has a freeze level
/// and no settlement level: all 192 members are live inputs (one more than before), the live
/// weight falls short, and the fallback weighs the 192 freeze changes. Keys interleave every
/// merge as before. `dormant_last`: the listed sources, evicted and untouched since before
/// January, February and March list, are recorded last, so each of the completing call's eight
/// starts three ring slots and takes its due captures there; else the joiners are. `interleaved`:
/// the completing call's heads but the large one print every five-sample window of the contract
/// in the order that compares the most (`interleaved_ranks(5)`: nine comparisons a window,
/// against eight). `late` (round 16 review, N2): the three later contracts list only after the
/// first's windows (so its dormant heads can print them), and the component completes after all
/// three later freezes, every contract unsettled, so each of those heads also takes the three
/// later listings' and freezes' captures. Returns the completing call's CU, the heaviest call's
/// and the completing call's heap floor.
async fn lta_retained_population(
    dormant_last: bool,
    interleaved: bool,
    late: bool,
) -> (u64, u64, (u32, String)) {
    let mut params = test_params();
    params.max_strikes = 1;
    let mut env = bench_lta_env(params).await;
    let a = env.user(600_000 * AMBA, 0).await;
    let b = env.user(3_000_000 * AMBA, 0).await;
    let f = env.user(0, 20_000 * USDC).await;
    env.h.fund(&a.pubkey(), 10_000_000_000).await;
    let rank = interleaved_ranks(192);
    let key = |p: usize| (1_010_000 + 5_000 * rank[p] as u64, adverse(2_000 + rank[p]));
    let far = lta_terms(OCT_01_2026, JAN_01_2027 + 365 * DAY);
    let (fill, update) = (HeadClaimKind::Fill, HeadClaimKind::Update);
    let mut listed = Vec::new();
    let mut fills = Vec::new();
    for k in 0..96 {
        let id = env.active_head(&a, &b, 2).await;
        fills.push(
            env.claim_with(&f, id, fill, 1_000_000, env.now, adverse(k), far)
                .await,
        );
        listed.push(id);
    }
    env.warp(env.now + DAY).await;
    for q in fills {
        env.finalize(q, &f.pubkey()).await;
    }
    assert_eq!(env.board().await.component_live_origins[2], 96);
    let normal = env.board().await.params;
    let mut threshold = normal;
    threshold.min_settle_weight = 192 * 2_500 * AMBA;
    env.set_params_measured(threshold, "SetBoardParams").await;
    let dec = DEC_01_2026;
    env.list_reference(dec, env.now + 2 * DAY).await;
    env.set_params_measured(normal, "SetBoardParams").await;
    let freeze = env.contract(dec).await.freeze_ts;
    // Settlement order: the listed sources, in listing order, then the joiners, or the other
    // way round.
    let first = env.component_sources(dec, 2, &[]).await;
    assert_eq!(first.len(), 96);
    let listed_at = |i: usize| if dormant_last { 96 + i } else { i };
    let mut updates = Vec::new();
    for (i, &id) in first.iter().enumerate() {
        let (value, o) = key(listed_at(i));
        updates.push(env.claim_with(&f, id, update, value, env.now, o, far).await);
    }
    env.warp(env.now + DAY).await;
    for q in updates {
        env.finalize(q, &f.pubkey()).await;
    }
    let start = env.now;
    for &id in &listed {
        env.undelegate(&b, id, 1_500 * AMBA).await;
    }
    env.warp(start + env.params.cooldown_secs - DAY).await;
    let mut standby = Vec::new();
    for k in 0..96 {
        let id = env.active_head(&a, &b, 2).await;
        if k == 95 {
            env.delegate(&b, id, 2_000_000 * AMBA).await;
        }
        let q = env
            .claim_with(&f, id, fill, 1_000_000, env.now, adverse(1_000 + k), far)
            .await;
        standby.push((id, q));
    }
    env.warp(start + env.params.cooldown_secs).await;
    let mut large = 0;
    for (&id, (h, q)) in listed.iter().zip(standby) {
        let ix = env.expire_ix(id);
        env.send(ix, &[]).await.unwrap();
        env.finalize(q, &f.pubkey()).await;
        large = h;
    }
    assert_eq!(env.board().await.live_count, 96);
    let mut joiners = env.component_sources(dec, 2, &[]).await.split_off(96);
    joiners.retain(|&id| id != large);
    joiners.push(large);
    assert_eq!(joiners.len(), 96);
    let joiner_at = |j: usize| if dormant_last { j } else { 96 + j };
    let order: Vec<u32> = if dormant_last {
        joiners.iter().chain(&first).copied().collect()
    } else {
        first.iter().chain(&joiners).copied().collect()
    };
    let seen = env.now;
    let mut updates = Vec::new();
    for (j, &id) in joiners.iter().enumerate() {
        let (value, o) = key(joiner_at(j));
        // The large one's price resets just after the freeze.
        let terms = if id == large {
            lta_terms(OCT_01_2026, freeze + 1)
        } else {
            far
        };
        updates.push(env.claim_with(&f, id, update, value, seen, o, terms).await);
    }
    assert!(seen < env.freeze_window(dec).await);
    // Every ring slot holds a contract.
    let later = [JAN_01_2027, FEB_01_2027, MAR_01_2027];
    if !late {
        for (n, expiry) in later.into_iter().enumerate() {
            env.list_reference(expiry, seen + 2 * (n as u64 + 1) * DAY)
                .await;
        }
    }
    if interleaved {
        // The completing call's heads but the large one print each window's five samples in the
        // order that compares the most, about their key (its median).
        let fw = env.freeze_window(dec).await;
        let at = |j: usize| {
            let (s, e, k) = if j < 5 {
                (fw, freeze, j)
            } else {
                (freeze, dec, j - 5)
            };
            s + (e - s) * (k as u64 + 1) / 5
        };
        let r5 = interleaved_ranks(5);
        for j in 0..10 {
            env.warp(at(j) - 600).await;
            for p in (184..192).filter(|&p| order[p] != large) {
                let (value, o) = key(p);
                let v = value + r5[j % 5] as u64 - 2;
                updates.push(
                    env.claim_with(&f, order[p], update, v, env.now, o, far)
                        .await,
                );
            }
        }
    }
    env.warp(env.now.max(freeze + 3_600) + DAY + 60).await;
    for q in updates {
        env.finalize(q, &f.pubkey()).await;
    }
    if late {
        let base = env.now;
        for (n, expiry) in later.into_iter().enumerate() {
            env.list_reference(expiry, base + 2 * (n as u64 + 1) * DAY)
                .await;
        }
    }
    let board = env.board().await;
    assert!((0..4).all(|i| board.unsettled(i, board.contract_gens[i])));
    let k = env.contract(dec).await;
    let record = heads_view::level_record(&k, &*env.head(large).await);
    assert!(record.eligible && record.level.is_none() && record.freeze_level.is_some());
    let ids = env.component_sources(dec, 2, &[]).await;
    assert_eq!(ids.len(), 192);
    let complete = if late {
        env.contract(MAR_01_2027).await.freeze_ts + DAY
    } else {
        dec + env.params.challenge_window_secs + 1
    };
    env.warp(complete).await;
    let board = env.board().await;
    assert!((0..4).all(|i| board.unsettled(i, board.contract_gens[i])));
    let tail = &order[184..];
    if dormant_last {
        // Untouched since before January listed: each starts three slots when recorded.
        let board = env.board().await;
        for &id in tail {
            let h = env.head(id).await;
            assert!(h.evicted_ts != 0);
            assert_eq!(
                (1..4)
                    .filter(|&i| h.capture_periods[i] != board.contract_periods[i])
                    .count(),
                3
            );
        }
    }
    let payer = env.h.ctx.payer.pubkey();
    let (mut completing, mut most, mut heap) = (0, 0, (0, String::new()));
    env.production_on().await;
    for (n, chunk) in order.chunks(HEADS_SETTLE_MAX_BATCH).enumerate() {
        let first = (n * HEADS_SETTLE_MAX_BATCH) as u16;
        let ix =
            hx::settle_component(&program(), &payer, &env.product, dec, 2, first, chunk).unwrap();
        if (n + 1) * HEADS_SETTLE_MAX_BATCH >= order.len() {
            heap = env.heap_floor(ix.clone()).await;
        }
        completing = env.measure("SettleComponent", ix, &[]).await;
        most = most.max(completing);
    }
    env.production_off().await;
    let (_, sources) = env.weights(dec, 2).await.unwrap();
    assert_eq!(sources.len(), 192);
    // All 192 are live inputs (none retired by K); the large one has no settlement level.
    assert!(
        sources
            .iter()
            .all(|s| s.flags & WEIGHT_SOURCE_RETIRED == 0
                && s.flags & WEIGHT_SOURCE_FREEZE_LEVEL != 0)
    );
    assert_eq!(
        sources
            .iter()
            .filter(|s| s.flags & WEIGHT_SOURCE_LEVEL == 0)
            .count(),
        1
    );
    let contract = env.contract(dec).await;
    assert_eq!(contract.component_degraded[2], DEGRADED_TOO_LITTLE_WEIGHT);
    assert_ne!(contract.component_flags[2] & COMPONENT_LAST_VALID, 0);
    assert_eq!(contract.component_eligible[2], 191);
    (completing, most, heap)
}

/// The completing call's CU and heap for `lta_retained_population` in `label`'s variant.
async fn lta_population_report(label: &str, dormant_last: bool, interleaved: bool, late: bool) {
    let (units, most, heap) = lta_retained_population(dormant_last, interleaved, late).await;
    eprintln!(
        "HEAP SettleComponent, 192 live inputs, {label}: runs in a {} KiB frame (at {} KiB: {})",
        heap.0,
        heap.0 - 1,
        heap.1
    );
    let completing = format!("SettleComponent: the completing call, 192 live inputs, {label}");
    let heaviest = format!("SettleComponent: the heaviest call, {label}");
    report(&[(completing.as_str(), units), (heaviest.as_str(), most)]);
}

/// Round 15 review, CU and heap: the completing call over 192 live inputs, its eight heads the
/// dormant listed sources (three slot starts and the due captures each).
#[tokio::test]
async fn lta_population_settles_with_dormant_members_last() {
    lta_population_report("dormant members last", true, false, false).await;
}

/// Round 15 review, CU and heap: the completing call over 192 live inputs, its heads joiners
/// with interleaved five-sample windows.
#[tokio::test]
async fn lta_population_settles_with_interleaved_windows_last() {
    lta_population_report("interleaved windows last", false, true, false).await;
}

/// Round 16 review, N2: the two combined and completed late. The completing call's eight heads
/// are listed sources with interleaved windows in the first contract, untouched since before the
/// other three listed; it completes after all three later freezes with every contract unsettled,
/// so each head starts three slots and takes six captures there.
#[tokio::test]
async fn lta_population_settles_dormant_and_interleaved_after_every_freeze() {
    lta_population_report("dormant, interleaved, after every freeze", true, true, true).await;
}

// The user's decision, 2026-10-31: pooled stake activates a source
// ------------------------------------------------------------------------------------------

/// A board needing one delegator, its activation floor 3,000 AMBA, and a head that three wallets
/// back with 1,000 AMBA each (the first posts it): returns the env, the head and the wallets.
async fn pooled_source() -> (Env, u32, Vec<Keypair>) {
    let mut params = test_params();
    params.min_delegators = 1;
    params.floor_activate = 3_000 * AMBA;
    let mut env = small_env(params, HeadsMixBasis::Assumed).await;
    let mut wallets = Vec::new();
    for _ in 0..3 {
        wallets.push(env.user(2_000 * AMBA, 0).await);
    }
    let h = env.post(&wallets[0], 1, 1_000 * AMBA).await;
    env.delegate(&wallets[1], h, 1_000 * AMBA).await;
    assert_eq!(env.head(h).await.status, HeadStatus::Open, "2,000 of 3,000");
    env.delegate(&wallets[2], h, 1_000 * AMBA).await;
    (env, h, wallets)
}

/// The user's decision (2026-10-31): with `min_delegators` 1 every positive delegation counts as a
/// delegator, so stake pooled over three wallets, each with a third of the activation floor,
/// activates a source (before, each was below its share of the floor, all of it, so none counted
/// and the head stayed Open).
#[tokio::test]
async fn pooled_stake_activates_a_source_needing_one_delegator() {
    let (mut env, h, _) = pooled_source().await;
    let head = env.head(h).await;
    assert_eq!(
        (head.status, head.delegators, head.delegation),
        (HeadStatus::Active, 3, 3_000 * AMBA)
    );
}

/// With `min_delegators` 1 a dust delegation is a delegator too, and neither its arrival nor its
/// withdrawal switches a pooled source off: it stays backed, an origin of its component.
#[tokio::test]
async fn a_dust_delegator_cannot_switch_off_a_source_needing_one_delegator() {
    let (mut env, h, _) = pooled_source().await;
    let f = env.user(0, 100 * USDC).await;
    let p = env.fill(&f, h, 1_000_000, env.now, origin("pooled")).await;
    env.warp(env.now + DAY).await;
    env.finalize(p, &f.pubkey()).await;
    assert_eq!(env.head(h).await.status, HeadStatus::Live);
    assert_eq!(env.board().await.component_live_origins[1], 1);
    let dust = env.user(AMBA, 0).await;
    env.delegate(&dust, h, 1).await;
    assert_eq!(env.head(h).await.delegators, 4);
    assert_eq!(env.board().await.component_live_origins[1], 1);
    env.warp(env.now + DAY).await;
    env.undelegate(&dust, h, 1).await;
    let head = env.head(h).await;
    assert_eq!((head.delegators, head.unbacked_since_ts), (3, 0));
    assert_eq!(env.board().await.component_live_origins[1], 1);
}
