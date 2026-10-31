//! Oracle v2, heads on pikes (tag 210): the part A handlers end to end on a Token-2022 AMBA with
//! a 25 bp transfer fee, a RAMX board from the channel molds, stake, heads, delegation, escrow,
//! expiry, rewards and burns, plus the security edges. States that need part B's
//! claims (a Live head, an open case) are installed directly.
#![cfg(feature = "localnet-dev-authority")]

mod common;

use ameba_commodity_spec::Product;
use ameba_oracle::{
    error::OracleError,
    instruction::{self, heads as hx},
    state::heads::{
        find_head_address, find_head_delegation_address, find_heads_amba_vault_address,
        find_heads_authority_address, find_heads_board_address, find_heads_config_address,
        find_heads_epoch_address, find_heads_stake_address, Delegation, Head, HeadAccessClass,
        HeadClaimKind, HeadStatus, HeadsAccount, HeadsBoard, HeadsConfig, HeadsEpoch,
        HeadsMixBasis, HeadsParams, HeadsStake, LIVE_DUPLICATE,
    },
};
use ameba_oracle_heads::liability;
use common::*;
use solana_sdk::{
    account::AccountSharedData,
    compute_budget::ComputeBudgetInstruction,
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use spl_token_2022::extension::ExtensionType;

const AMBA: u64 = 1_000_000_000;
const FEE_BPS: u16 = 25;

fn t22() -> Pubkey {
    spl_token_2022::id()
}

fn ramx() -> [u8; 32] {
    product("RAMX")
}

struct Env {
    h: Harness,
    mint: Pubkey,
    mint_authority: Keypair,
    board: Pubkey,
    now: u64,
}

impl Env {
    /// Sends with a full heap frame and compute limit, as clients of the big accounts do.
    async fn send(
        &mut self,
        ix: Instruction,
        signers: &[&Keypair],
    ) -> Result<(), solana_program_test::BanksClientError> {
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

    /// Sends and returns the compute units consumed (the SBF artifact meters them; the native
    /// processor reports none).
    async fn measure(&mut self, label: &str, ix: Instruction, signers: &[&Keypair]) -> u64 {
        let payer = self.h.payer();
        let blockhash = self.h.ctx.get_new_latest_blockhash().await.unwrap();
        let mut all = vec![&payer];
        for s in signers {
            if s.pubkey() != payer.pubkey() && !all.iter().any(|a| a.pubkey() == s.pubkey()) {
                all.push(s);
            }
        }
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
        let result = self
            .h
            .ctx
            .banks_client
            .process_transaction_with_metadata(tx)
            .await
            .unwrap();
        result.result.unwrap();
        let units = result.metadata.map_or(0, |m| m.compute_units_consumed);
        eprintln!("CU {label}: {units}");
        units
    }

    async fn warp(&mut self, to: u64) {
        self.now = to;
        set_unix_timestamp(&self.h.ctx, to).await;
        self.h.advance_slot().await;
    }

    async fn read<T: HeadsAccount>(&mut self, key: Pubkey) -> Box<T> {
        let account = self.h.account(key).await.expect("account exists");
        assert_eq!(account.owner, program());
        T::unpack(&account.data).expect("valid heads account")
    }

    /// Installs a state change part B's instructions would make.
    async fn edit<T: HeadsAccount>(&mut self, key: Pubkey, f: impl FnOnce(&mut T)) {
        let mut account = self.h.account(key).await.expect("account exists");
        let mut value = T::unpack(&account.data).unwrap();
        f(&mut value);
        value.pack(&mut account.data).unwrap();
        self.h
            .ctx
            .set_account(&key, &AccountSharedData::from(account));
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

    async fn amount(&mut self, token_account: Pubkey) -> u64 {
        let account = self.h.account(token_account).await.unwrap();
        u64::from_le_bytes(account.data[64..72].try_into().unwrap())
    }

    async fn staker(&mut self, amba: u64) -> (Keypair, Pubkey) {
        let key = Keypair::new();
        self.h.fund(&key.pubkey(), 2_000_000_000).await;
        let ata = self.amba_account(&key.pubkey(), amba).await;
        (key, ata)
    }

    async fn stake(&mut self, owner: &Keypair, ata: &Pubkey, amount: u64) {
        let ix = hx::stake(
            &program(),
            &owner.pubkey(),
            &owner.pubkey(),
            ata,
            &self.mint,
            &t22(),
            amount,
        );
        self.measure("Stake", ix, &[owner]).await;
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

    async fn post(
        &mut self,
        proposer: &Keypair,
        component: u8,
        amount: u64,
    ) -> Result<u32, solana_program_test::BanksClientError> {
        let id = self.read::<HeadsBoard>(self.board).await.head_count;
        let spec = format!(
            "head {id} ({amount} atoms, component {component}): Mouser DDR5-4800 16Gb, the component's sentence restated"
        );
        let (hash, object) = self.object(proposer, spec.as_bytes()).await;
        let ix = hx::post_head(
            &program(),
            &proposer.pubkey(),
            &proposer.pubkey(),
            &ramx(),
            id,
            component,
            HeadAccessClass::PublicPage,
            amount,
            "Mouser DDR5 16Gb",
            hash,
            &object,
        )
        .unwrap();
        if amount >= 2_000 * AMBA && component == 1 && id == 0 {
            // The first head of each scenario: its cost is the one worth tracking.
            self.measure("PostHead", ix, &[proposer]).await;
        } else {
            self.send(ix, &[proposer]).await?;
        }
        Ok(id)
    }

    fn head_key(&self, id: u32) -> Pubkey {
        find_head_address(&program(), &self.board, id).0
    }

    fn delegation_key(&self, id: u32, owner: &Pubkey) -> Pubkey {
        find_head_delegation_address(&program(), &self.head_key(id), owner).0
    }

    async fn stake_of(&mut self, owner: &Pubkey) -> Box<HeadsStake> {
        self.read(find_heads_stake_address(&program(), owner).0)
            .await
    }

    async fn sync(&mut self, id: u32, owner: &Pubkey) {
        let ix = hx::sync_delegation(&program(), &ramx(), id, owner, owner);
        self.measure("SyncDelegation", ix, &[]).await;
    }
}

fn params() -> HeadsParams {
    HeadsParams::spec_defaults()
}

/// An active oracle, a Token-2022 AMBA with a 25 bp fee, its vault, the heads config and a RAMX
/// board from the molds with its sentences sealed.
async fn env() -> Env {
    let mut h = start().await;
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
    let mut env = Env {
        h,
        mint: mint.pubkey(),
        mint_authority,
        board: find_heads_board_address(&program(), &ramx()).0,
        now: NOW,
    };
    let authority = find_heads_authority_address(&program()).0;
    let vault = env.amba_account(&authority, 0).await;
    assert_eq!(
        vault,
        find_heads_amba_vault_address(&program(), &env.mint, &t22())
    );
    let admin = env.h.admin.insecure_clone();
    let payer = env.h.payer();
    let ix = hx::init_heads_config(
        &program(),
        &payer.pubkey(),
        &admin.pubkey(),
        &env.mint,
        &t22(),
    );
    env.send(ix, &[&admin]).await.unwrap();
    let config: Box<HeadsConfig> = env.read(find_heads_config_address(&program()).0).await;
    assert_eq!((config.amba_decimals, config.amba_vault), (9, vault));

    let mold = Product::Ramx.mold();
    let items: Vec<hx::HeadsItemParams> = mold
        .items
        .iter()
        .map(|item| hx::HeadsItemParams {
            bucket_id: item.bucket.id32(),
            basket_weight_bps: item.basket_weight_bps,
            mix_bps: item.mix_bps,
            mix_basis: HeadsMixBasis::Assumed,
        })
        .collect();
    let ix = hx::init_board(
        &program(),
        &payer.pubkey(),
        &admin.pubkey(),
        ramx(),
        params(),
        items,
    )
    .unwrap();
    env.measure("InitBoard", ix, &[&admin]).await;
    // The first item's spot and contract sentences, from the mold.
    for channel in [0u8, 1] {
        let component = mold.component(channel).unwrap().clone();
        let (hash, object) = env.object(&admin, component.sentence.as_bytes()).await;
        assert_eq!(hash, component.sentence_sha256);
        let ix = hx::set_component(
            &program(),
            &admin.pubkey(),
            &ramx(),
            component.index,
            component.item,
            ameba_oracle::state::heads::HeadsChannel::from_byte(channel).unwrap(),
            hash,
            &object,
        )
        .unwrap();
        env.send(ix, &[&admin]).await.unwrap();
    }
    let board: Box<HeadsBoard> = env.read(env.board).await;
    assert_eq!(board.item_count as usize, mold.items.len());
    assert_eq!(board.component_count as usize, 3 * mold.items.len());
    assert_ne!(board.component_sentence_hashes[1], [0; 32]);
    assert_eq!(board.component_sentence_hashes[2], [0; 32]);
    env
}

fn fee(amount: u64) -> u64 {
    (u128::from(amount) * u128::from(FEE_BPS)).div_ceil(10_000) as u64
}

#[tokio::test]
async fn stake_post_delegate_activate_and_release_with_escrow() {
    let mut env = env().await;
    let (a, a_ata) = env.staker(100_000 * AMBA).await;
    let (b, b_ata) = env.staker(100_000 * AMBA).await;
    env.stake(&a, &a_ata, 50_000 * AMBA).await;
    env.stake(&b, &b_ata, 10_000 * AMBA).await;
    let received_a = 50_000 * AMBA - fee(50_000 * AMBA);
    let received_b = 10_000 * AMBA - fee(10_000 * AMBA);
    assert_eq!(env.stake_of(&a.pubkey()).await.free, received_a);
    let config: Box<HeadsConfig> = env.read(find_heads_config_address(&program()).0).await;
    assert_eq!(config.amba_accounted, received_a + received_b);
    assert_eq!(env.amount(config.amba_vault).await, received_a + received_b);

    // A component with no sentence takes no head; then a post of min_post on item 0's contract.
    assert_oracle_error(
        env.post(&a, 2, 2_000 * AMBA).await.map(|_| ()),
        OracleError::InvalidHeadsState,
    );
    assert_oracle_error(
        env.post(&a, 1, 999 * AMBA).await.map(|_| ()),
        OracleError::InsufficientHeadsStake,
    );
    let id = env.post(&a, 1, 2_000 * AMBA).await.unwrap();
    let head: Box<Head> = env.read(env.head_key(id)).await;
    assert_eq!(
        (head.status, head.delegation, head.delegators),
        (HeadStatus::Open, 2_000 * AMBA, 1)
    );
    assert_eq!(head.channel as u8, 1);
    assert_eq!(head.expires_ts, env.now + params().head_ttl_secs);
    // The proposer alone cannot activate it; a second delegator past the threshold does.
    let ix = hx::delegate(
        &program(),
        &b.pubkey(),
        &b.pubkey(),
        &ramx(),
        id,
        1_000 * AMBA,
    );
    env.measure("Delegate", ix, &[&b]).await;
    let head: Box<Head> = env.read(env.head_key(id)).await;
    assert_eq!(
        (head.status, head.delegation, head.delegators),
        (HeadStatus::Active, 3_000 * AMBA, 2)
    );
    let board: Box<HeadsBoard> = env.read(env.board).await;
    assert_eq!(board.total_delegation, 3_000 * AMBA);
    assert_eq!(
        env.stake_of(&b.pubkey()).await.free,
        received_b - 1_000 * AMBA
    );

    // A case opened before B leaves: B's escrow (19% at two strikes left) stays.
    let opened = env.now;
    env.edit::<Head>(env.head_key(id), |h| {
        h.source_case = 1;
        h.unresolved_cases = 1;
        h.unresolved_since_ts = opened;
    })
    .await;
    env.warp(env.now + 10).await;
    let ix = hx::undelegate(&program(), &b.pubkey(), &ramx(), id, 400 * AMBA);
    env.measure("Undelegate", ix, &[&b]).await;
    let d: Box<Delegation> = env.read(env.delegation_key(id, &b.pubkey())).await;
    assert_eq!(
        (d.active, d.held, d.held_principal),
        (600 * AMBA, 400 * AMBA, 400 * AMBA)
    );
    assert_eq!(d.cooldown_end_ts, env.now + params().cooldown_secs);
    let free_before = env.stake_of(&b.pubkey()).await.free;
    // Nothing before the cooldown ends.
    env.sync(id, &b.pubkey()).await;
    assert_eq!(env.stake_of(&b.pubkey()).await.free, free_before);
    env.warp(env.now + params().cooldown_secs).await;
    env.sync(id, &b.pubkey()).await;
    let escrow =
        liability::escrow_required(400 * AMBA, params().slash_rate_bps, params().max_strikes);
    assert_eq!(escrow, 76 * AMBA);
    assert_eq!(
        env.stake_of(&b.pubkey()).await.free,
        free_before + 400 * AMBA - escrow
    );
    // The proposer leaving under the open case keeps all of it held.
    let ix = hx::undelegate(&program(), &a.pubkey(), &ramx(), id, 2_000 * AMBA);
    env.send(ix, &[&a]).await.unwrap();
    env.warp(env.now + params().cooldown_secs).await;
    let free_a = env.stake_of(&a.pubkey()).await.free;
    env.sync(id, &a.pubkey()).await;
    assert_eq!(env.stake_of(&a.pubkey()).await.free, free_a);
    // The case resolves without a strike: everything held comes back, and the proposer's
    // delegation, now empty, closes into its owner.
    env.edit::<Head>(env.head_key(id), |h| {
        h.unresolved_cases = 0;
        h.unresolved_since_ts = 0;
    })
    .await;
    env.sync(id, &b.pubkey()).await;
    env.sync(id, &a.pubkey()).await;
    assert_eq!(
        env.stake_of(&b.pubkey()).await.free,
        free_before + 400 * AMBA
    );
    assert_eq!(env.stake_of(&a.pubkey()).await.free, free_a + 2_000 * AMBA);
    assert!(env
        .h
        .account(env.delegation_key(id, &a.pubkey()))
        .await
        .is_none());
    assert_eq!(env.stake_of(&a.pubkey()).await.delegation_count, 0);
    let d: Box<Delegation> = env.read(env.delegation_key(id, &b.pubkey())).await;
    assert_eq!((d.active, d.held), (600 * AMBA, 0));

    // Unstake what is free; the vault keeps exactly what is still owed.
    let free = env.stake_of(&b.pubkey()).await.free;
    let ix = hx::unstake(&program(), &b.pubkey(), &b_ata, &env.mint, &t22(), free);
    env.measure("Unstake", ix, &[&b]).await;
    assert_eq!(env.stake_of(&b.pubkey()).await.free, 0);
    let config: Box<HeadsConfig> = env.read(find_heads_config_address(&program()).0).await;
    assert_eq!(config.amba_accounted, received_a + received_b - free);
    assert_eq!(env.amount(config.amba_vault).await, config.amba_accounted);
    let ix = hx::unstake(&program(), &b.pubkey(), &b_ata, &env.mint, &t22(), 1);
    assert_oracle_error(
        env.send(ix, &[&b]).await,
        OracleError::InsufficientHeadsStake,
    );
}

#[tokio::test]
async fn an_open_head_expires_and_its_delegations_cool() {
    let mut env = env().await;
    let (a, ata) = env.staker(20_000 * AMBA).await;
    env.stake(&a, &ata, 10_000 * AMBA).await;
    let id = env.post(&a, 1, 1_000 * AMBA).await.unwrap();
    let ix = hx::expire_head(&program(), &ramx(), id);
    assert_oracle_error(
        env.send(ix.clone(), &[]).await,
        OracleError::InvalidHeadsState,
    );
    env.warp(env.now + params().head_ttl_secs).await;
    // Past its time to live it takes no delegation, and anyone expires it.
    let more = hx::delegate(&program(), &a.pubkey(), &a.pubkey(), &ramx(), id, AMBA);
    assert_oracle_error(env.send(more, &[&a]).await, OracleError::InvalidHeadsState);
    env.send(ix, &[]).await.unwrap();
    let head: Box<Head> = env.read(env.head_key(id)).await;
    assert_eq!(head.status, HeadStatus::Expired);
    assert_eq!(env.read::<HeadsBoard>(env.board).await.total_delegation, 0);
    let free = env.stake_of(&a.pubkey()).await.free;
    env.sync(id, &a.pubkey()).await;
    let d: Box<Delegation> = env.read(env.delegation_key(id, &a.pubkey())).await;
    assert_eq!((d.active, d.held), (0, 1_000 * AMBA));
    assert_eq!(d.cooldown_end_ts, head.retired_ts + params().cooldown_secs);
    env.warp(head.retired_ts + params().cooldown_secs).await;
    env.sync(id, &a.pubkey()).await;
    assert_eq!(env.stake_of(&a.pubkey()).await.free, free + 1_000 * AMBA);
    assert!(env
        .h
        .account(env.delegation_key(id, &a.pubkey()))
        .await
        .is_none());
}

/// Installs a Live head on the board's live list, as part B's standing fill does.
async fn make_live(env: &mut Env, id: u32, origin: u8, duplicate: bool) {
    let head_key = env.head_key(id);
    let now = env.now;
    let delegation = env.read::<Head>(head_key).await.delegation;
    env.edit::<Head>(head_key, |h| {
        h.status = HeadStatus::Live;
        h.live_ts = now;
        h.value = 100_000_000;
        h.origin_id = [origin; 32];
        h.duplicate = duplicate;
    })
    .await;
    env.edit::<HeadsBoard>(env.board, |b| {
        let e = usize::from(b.live_count);
        b.live_head_ids[e] = id;
        b.live_components[e] = 1;
        b.live_since_ts[e] = now;
        b.live_delegations[e] = delegation;
        b.live_values[e] = 100_000_000;
        b.live_origin_ids[e] = [origin; 32];
        if duplicate {
            b.live_flags[e] = LIVE_DUPLICATE;
            b.live_reward_offsets[e] = 0;
        } else {
            b.live_reward_offsets[e] = b.reward_index;
            b.live_delegation += delegation;
        }
        b.live_count += 1;
        b.component_live_sources[1] += 1;
        b.component_live_origins[1] = 1;
    })
    .await;
}

async fn usdc_owner(env: &mut Env, usdc: u64) -> Keypair {
    let key = Keypair::new();
    env.h.fund(&key.pubkey(), 1_000_000_000).await;
    let token = env.h.create_ata(&key.pubkey()).await;
    env.h.mint_usdc(&token, usdc).await;
    env.h
        .send(
            instruction::init_user_collateral(&program(), &key.pubkey()),
            &[&key],
        )
        .await
        .unwrap();
    if usdc > 0 {
        let ix = instruction::deposit_collateral(
            &program(),
            &key.pubkey(),
            &token,
            &env.h.usdc_mint,
            usdc,
        );
        env.h.send(ix, &[&key]).await.unwrap();
    }
    key
}

#[tokio::test]
async fn rewards_reach_live_heads_but_not_open_or_duplicate_ones() {
    let mut env = env().await;
    let (a, a_ata) = env.staker(20_000 * AMBA).await;
    let (b, b_ata) = env.staker(20_000 * AMBA).await;
    env.stake(&a, &a_ata, 10_000 * AMBA).await;
    env.stake(&b, &b_ata, 10_000 * AMBA).await;
    let live = env.post(&a, 1, 3_000 * AMBA).await.unwrap();
    env.send(
        hx::delegate(
            &program(),
            &b.pubkey(),
            &b.pubkey(),
            &ramx(),
            live,
            1_000 * AMBA,
        ),
        &[&b],
    )
    .await
    .unwrap();
    let copy = env.post(&b, 1, 2_000 * AMBA).await.unwrap();
    let open = env.post(&a, 1, 2_000 * AMBA).await.unwrap();
    let funder = usdc_owner(&mut env, 10_000_000).await;
    let month = ameba_oracle_heads::time::month_id(env.now);
    // Nothing live yet: the delegator share waits in the reserve.
    let fund = |amount| {
        hx::fund_board(
            &program(),
            &funder.pubkey(),
            &funder.pubkey(),
            &ramx(),
            month,
            amount,
        )
    };
    env.measure("FundBoard", fund(1_000_000), &[&funder]).await;
    let board: Box<HeadsBoard> = env.read(env.board).await;
    assert_eq!((board.reserve, board.reward_index), (800_000, 0));
    make_live(&mut env, live, 7, false).await;
    make_live(&mut env, copy, 7, true).await;
    env.send(fund(1_000_000), &[&funder]).await.unwrap();
    let board: Box<HeadsBoard> = env.read(env.board).await;
    assert_eq!(board.reserve, 0);
    assert_eq!(board.prize_funded_total, 400_000);
    let epoch: Box<HeadsEpoch> = env
        .read(find_heads_epoch_address(&program(), &env.board, month).0)
        .await;
    assert_eq!(epoch.prize_pool, 400_000);
    assert_eq!(
        env.h.collateral(&funder.pubkey()).await.available_balance,
        8_000_000
    );
    // 1.6 USDC over the live head's 4,000 AMBA: A holds 3/4 of it, B 1/4.
    for (who, expected) in [(&a, 1_200_000u64), (&b, 400_000)] {
        let collateral =
            ameba_oracle::state::derive_user_collateral_pda(&program(), &who.pubkey()).0;
        if env.h.account(collateral).await.is_none() {
            env.h
                .send(
                    instruction::init_user_collateral(&program(), &who.pubkey()),
                    &[who],
                )
                .await
                .unwrap();
        }
        let ix = hx::claim_rewards(&program(), &who.pubkey(), &ramx(), live);
        env.measure("ClaimRewards", ix, &[who]).await;
        let paid = env.h.collateral(&who.pubkey()).await.available_balance;
        assert!(expected - paid <= 1, "{paid} of {expected}");
    }
    // The duplicate and the open head earned nothing.
    for (who, id) in [(&b, copy), (&a, open)] {
        let ix = hx::claim_rewards(&program(), &who.pubkey(), &ramx(), id);
        assert_oracle_error(
            env.send(ix, &[who]).await,
            OracleError::AmountMustBePositive,
        );
    }
    let board: Box<HeadsBoard> = env.read(env.board).await;
    assert!(board.delegator_paid_total <= board.delegator_funded_total);
}

#[tokio::test]
async fn burn_pending_burns_slashed_amba_and_burned_usdc() {
    let mut env = env().await;
    let (a, ata) = env.staker(20_000 * AMBA).await;
    env.stake(&a, &ata, 10_000 * AMBA).await;
    let _ = usdc_owner(&mut env, 5_000_000).await;
    let config_key = find_heads_config_address(&program()).0;
    env.edit::<HeadsConfig>(config_key, |c| {
        c.amba_accounted -= 70 * AMBA;
        c.pending_amba_burn = 70 * AMBA;
    })
    .await;
    env.edit::<HeadsBoard>(env.board, |b| b.pending_usdc_burn = 2_000_000)
        .await;
    let vault = env.read::<HeadsConfig>(config_key).await.amba_vault;
    let (amba_before, usdc_before) = (
        env.amount(vault).await,
        env.h.token_balance(env.h.bond_vault()).await,
    );
    let ix = hx::burn_pending(&program(), &ramx(), &env.mint, &t22(), &env.h.usdc_mint);
    env.measure("BurnPending", ix.clone(), &[]).await;
    assert_eq!(env.amount(vault).await, amba_before - 70 * AMBA);
    assert_eq!(
        env.h.token_balance(env.h.bond_vault()).await,
        usdc_before - 2_000_000
    );
    let config: Box<HeadsConfig> = env.read(config_key).await;
    assert_eq!(
        (config.pending_amba_burn, config.amba_burned_total),
        (0, 70 * AMBA)
    );
    assert_eq!(env.amount(vault).await, config.amba_accounted);
    let board: Box<HeadsBoard> = env.read(env.board).await;
    assert_eq!(
        (board.pending_usdc_burn, board.usdc_burned_total),
        (0, 2_000_000)
    );
    // Nothing pending: nothing to do.
    assert_oracle_error(env.send(ix, &[]).await, OracleError::AmountMustBePositive);
}

#[tokio::test]
async fn security_edges() {
    let mut env = env().await;
    let admin = env.h.admin.insecure_clone();
    let payer = env.h.payer();
    // The config is create-only.
    let again = hx::init_heads_config(
        &program(),
        &payer.pubkey(),
        &admin.pubkey(),
        &env.mint,
        &t22(),
    );
    assert_oracle_error(
        env.send(again, &[&admin]).await,
        OracleError::AlreadyInitialized,
    );
    // Only the admin sets up boards.
    let stranger = Keypair::new();
    env.h.fund(&stranger.pubkey(), 1_000_000_000).await;
    let ix = hx::set_board_params(&program(), &stranger.pubkey(), &ramx(), 1, params());
    assert_oracle_error(env.send(ix, &[&stranger]).await, OracleError::Unauthorized);
    let ix = hx::set_board_params(&program(), &admin.pubkey(), &ramx(), 7, params());
    assert_oracle_error(
        env.send(ix, &[&admin]).await,
        OracleError::InvalidHeadsState,
    );
    let ix = hx::set_board_params(&program(), &admin.pubkey(), &ramx(), 1, params());
    env.send(ix, &[&admin]).await.unwrap();
    assert_eq!(env.read::<HeadsBoard>(env.board).await.params_version, 2);
    // An item's bucket never changes; appending past the end is refused.
    let mut item = hx::HeadsItemParams {
        bucket_id: [9; 32],
        basket_weight_bps: 1_000,
        mix_bps: [0, 10_000, 0],
        mix_basis: HeadsMixBasis::None,
    };
    let ix = hx::set_item(&program(), &admin.pubkey(), &ramx(), 0, item.clone()).unwrap();
    assert_oracle_error(
        env.send(ix, &[&admin]).await,
        OracleError::InvalidHeadsState,
    );
    item.bucket_id = env.read::<HeadsBoard>(env.board).await.item_bucket_ids[0];
    let ix = hx::set_item(&program(), &admin.pubkey(), &ramx(), 0, item).unwrap();
    env.send(ix, &[&admin]).await.unwrap();

    let (a, ata) = env.staker(20_000 * AMBA).await;
    // A vault that is not the authority's ATA is refused.
    let other = env.amba_account(&a.pubkey(), 0).await;
    let mut ix = hx::stake(
        &program(),
        &a.pubkey(),
        &a.pubkey(),
        &ata,
        &env.mint,
        &t22(),
        AMBA,
    );
    ix.accounts[5].pubkey = other;
    assert_oracle_error(
        env.send(ix, &[&a]).await,
        OracleError::InvalidHeadsTokenAccount,
    );
    // A squatted stake address (foreign-owned data) fails without mutation.
    let stake_key = find_heads_stake_address(&program(), &a.pubkey()).0;
    let mut squat = AccountSharedData::new(10_000_000, 8, &Pubkey::new_unique());
    squat.set_data_from_slice(&[1; 8]);
    env.h
        .set_account_preserving_capitalization(stake_key, squat)
        .await;
    let ix = hx::stake(
        &program(),
        &a.pubkey(),
        &a.pubkey(),
        &ata,
        &env.mint,
        &t22(),
        AMBA,
    );
    assert_oracle_error(
        env.send(ix.clone(), &[&a]).await,
        OracleError::AddressOccupied,
    );
    // A prefunded, empty system account at the address is fine.
    env.h
        .set_system_account_with_lamports(stake_key, 5_000_000)
        .await;
    env.send(ix, &[&a]).await.unwrap();
    // Somebody else's stake is not the signer's.
    let (b, b_ata) = env.staker(20_000 * AMBA).await;
    env.stake(&b, &b_ata, 5_000 * AMBA).await;
    let mut ix = hx::unstake(&program(), &a.pubkey(), &ata, &env.mint, &t22(), 1);
    ix.accounts[2].pubkey = find_heads_stake_address(&program(), &b.pubkey()).0;
    assert_oracle_error(env.send(ix, &[&a]).await, OracleError::InvalidHeadsAccount);
    // A heads account owned by another program never loads.
    let id = env.post(&b, 1, 1_000 * AMBA).await.unwrap();
    let delegation_key = env.delegation_key(id, &b.pubkey());
    let mut moved = env.h.account(delegation_key).await.unwrap();
    moved.owner = Pubkey::new_unique();
    env.h
        .ctx
        .set_account(&delegation_key, &AccountSharedData::from(moved));
    let ix = hx::undelegate(&program(), &b.pubkey(), &ramx(), id, AMBA);
    assert_oracle_error(env.send(ix, &[&b]).await, OracleError::InvalidHeadsAccount);
    // Payloads are exact length.
    let mut ix = hx::delegate(&program(), &a.pubkey(), &a.pubkey(), &ramx(), id, AMBA);
    ix.data.push(0);
    assert_oracle_error(
        env.send(ix, &[&a]).await,
        OracleError::InvalidInstructionData,
    );
    // A delegation over the free stake, and a commit whose bond is not the board's.
    let ix = hx::delegate(
        &program(),
        &a.pubkey(),
        &a.pubkey(),
        &ramx(),
        id,
        1_000_000 * AMBA,
    );
    assert_oracle_error(
        env.send(ix, &[&a]).await,
        OracleError::InsufficientHeadsStake,
    );
    let ix = hx::commit_claim(
        &program(),
        &a.pubkey(),
        &a.pubkey(),
        &ramx(),
        id,
        0,
        HeadClaimKind::Fill,
        [1; 32],
        1,
    );
    assert_oracle_error(env.send(ix, &[&a]).await, OracleError::InvalidHeadsState);
}

#[tokio::test]
async fn a_sponsored_delegation_returns_its_rent_to_the_sponsor() {
    let mut env = env().await;
    let (a, a_ata) = env.staker(20_000 * AMBA).await;
    env.stake(&a, &a_ata, 10_000 * AMBA).await;
    let id = env.post(&a, 1, 1_000 * AMBA).await.unwrap();
    // A wallet without SOL (a Google or Magic sign-in) and the sponsor that pays its rent.
    let owner = Keypair::new();
    let owner_ata = env.amba_account(&owner.pubkey(), 1_000 * AMBA).await;
    let sponsor = Keypair::new();
    env.h.fund(&sponsor.pubkey(), 1_000_000_000).await;
    let ix = hx::stake(
        &program(),
        &sponsor.pubkey(),
        &owner.pubkey(),
        &owner_ata,
        &env.mint,
        &t22(),
        100 * AMBA,
    );
    env.send(ix, &[&sponsor, &owner]).await.unwrap();
    let ix = hx::delegate(
        &program(),
        &sponsor.pubkey(),
        &owner.pubkey(),
        &ramx(),
        id,
        1,
    );
    env.send(ix, &[&sponsor, &owner]).await.unwrap();
    let delegation_key = env.delegation_key(id, &owner.pubkey());
    let d: Box<Delegation> = env.read(delegation_key).await;
    assert_eq!(d.rent_payer, sponsor.pubkey());
    let rent = env.h.account(delegation_key).await.unwrap().lamports;
    // Delegate an atom, take it back, sync: the rent cannot be farmed.
    let ix = hx::undelegate(&program(), &owner.pubkey(), &ramx(), id, 1);
    env.send(ix, &[&owner]).await.unwrap();
    env.warp(env.now + params().cooldown_secs).await;
    let to_owner = hx::sync_delegation(&program(), &ramx(), id, &owner.pubkey(), &owner.pubkey());
    assert_oracle_error(
        env.send(to_owner, &[]).await,
        OracleError::InvalidHeadsAccount,
    );
    let sponsor_before = env.h.account(sponsor.pubkey()).await.unwrap().lamports;
    let ix = hx::sync_delegation(&program(), &ramx(), id, &owner.pubkey(), &sponsor.pubkey());
    env.send(ix, &[]).await.unwrap();
    assert!(env.h.account(delegation_key).await.is_none());
    assert_eq!(
        env.h.account(sponsor.pubkey()).await.unwrap().lamports,
        sponsor_before + rent
    );
    assert_eq!(
        env.h
            .account(owner.pubkey())
            .await
            .map_or(0, |a| a.lamports),
        0
    );
    // The owner's stake came back whole; its stake account stays (a stake never closes).
    assert_eq!(
        env.stake_of(&owner.pubkey()).await.free,
        100 * AMBA - fee(100 * AMBA)
    );
}
