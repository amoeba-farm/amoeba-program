//! Shared ProgramTest harness. It runs the native processor by default; with
//! `--features test-sbf` and `SBF_OUT_DIR` set, the same cases run against the SBF artifact
//! built with the same feature (`target/deploy-test`), so both sides use the shortened
//! `test-sbf` calendar.
#![allow(dead_code)]

pub mod cfm;
pub mod cohort;
pub mod settle;
pub mod updates;

use ameba_oracle::{
    error::OracleError,
    instruction,
    state::{find_bond_vault_address, find_config_address, OracleConfig, UserCollateral},
};
use ameba_oracle_interface::{find_period_address, load_period_for, Period};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{account_info::AccountInfo, program_option::COption, program_pack::Pack};
use solana_program_test::{BanksClientError, ProgramTest, ProgramTestContext};
use solana_sdk::{
    account::{Account, AccountSharedData},
    clock::Clock,
    instruction::{Instruction, InstructionError},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::{Transaction, TransactionError},
};
use solana_sdk_ids::bpf_loader_upgradeable;

pub const PROGRAM_DAY_SECONDS: u64 = ameba_oracle::constants::ORACLE_CALENDAR_DAY_SECONDS;
pub const PROGRAM_PRE_LISTING_SECONDS: u64 =
    ameba_oracle::constants::ORACLE_PRE_LISTING_WINDOW_SECONDS;

pub const NOW: u64 = 1_792_195_200; // 2026-10-17T00:00:00Z
pub const OCT_01_2026: u64 = 1_790_812_800;
pub const NOV_01_2026: u64 = 1_793_491_200;
pub const DEC_01_2026: u64 = 1_796_083_200;
pub const JAN_01_2027: u64 = 1_798_761_600;
pub const FEB_01_2027: u64 = 1_801_440_000;
pub const MAR_01_2027: u64 = 1_803_859_200;
pub const APR_01_2027: u64 = 1_806_537_600;

/// Planned listing and expiry of the first rung used by most tests.
pub const PLANNED: u64 = DEC_01_2026;
pub const EXPIRY: u64 = MAR_01_2027;

pub fn product(name: &str) -> [u8; 32] {
    let mut id = [0; 32];
    id[..name.len()].copy_from_slice(name.as_bytes());
    id
}

pub fn program() -> Pubkey {
    ameba_oracle::id()
}

pub fn program_test() -> ProgramTest {
    #[cfg(feature = "test-sbf")]
    let mut test = ProgramTest::new("ameba_oracle", ameba_oracle::id(), None);
    #[cfg(not(feature = "test-sbf"))]
    let mut test = ProgramTest::new(
        "ameba_oracle",
        ameba_oracle::id(),
        solana_program_test::processor!(ameba_oracle::process_instruction),
    );
    #[cfg(not(feature = "localnet-dev-authority"))]
    {
        use ameba_oracle::governance_gate::*;
        let controller = PINNED_CONTROLLER_PROGRAM_ID;
        let (key, bump) = derive_protocol_gate_pda(&controller, &program());
        let mut data = vec![0u8; PROTOCOL_GATE_LEN];
        data[..8].copy_from_slice(&PROTOCOL_GATE_DISCRIMINATOR);
        data[8] = PROTOCOL_GATE_VERSION_V1;
        data[9] = bump;
        data[10] = 1;
        data[12..44].copy_from_slice(
            derive_controller_config_pda(&controller, &program())
                .0
                .as_ref(),
        );
        data[44..76].copy_from_slice(program().as_ref());
        data[76..108].copy_from_slice(derive_target_programdata_pda(&program()).as_ref());
        data[108..116].copy_from_slice(&1u64.to_le_bytes());
        test.add_account(
            key,
            Account {
                lamports: 1_000_000,
                data,
                owner: controller,
                executable: false,
                rent_epoch: 0,
            },
        );
    }
    test.prefer_bpf(cfg!(feature = "test-sbf"));
    test
}

/// Loader-v3 ProgramData metadata; the test program itself loads through the non-upgradeable
/// loader, so this address is otherwise empty.
pub fn program_data(authority: Option<Pubkey>) -> Account {
    let mut data = vec![0; 45];
    data[0..4].copy_from_slice(&3u32.to_le_bytes());
    data[4..12].copy_from_slice(&1u64.to_le_bytes());
    if let Some(authority) = authority {
        data[12] = 1;
        data[13..45].copy_from_slice(authority.as_ref());
    }
    Account {
        lamports: 1_000_000_000,
        data,
        owner: bpf_loader_upgradeable::id(),
        executable: false,
        rent_epoch: 0,
    }
}

pub struct Harness {
    pub ctx: ProgramTestContext,
    pub upgrade_authority: Keypair,
    /// Config admin. Until tag 128 runs it is also the oracle authority.
    pub admin: Keypair,
    pub oracle_authority: Keypair,
    pub usdc_mint: Pubkey,
    pub usdc_mint_authority: Keypair,
    /// The version-1 settlement signers (sorted by key) once `activate` created the registry.
    pub settlement_signers: Vec<Keypair>,
    pub recovery_authority: Keypair,
}

pub async fn start_with(
    upgrade_authority_recorded: bool,
    extra: Vec<(Pubkey, Account)>,
) -> Harness {
    let upgrade_authority = Keypair::new();
    let mut test = program_test();
    let (program_data_key, _) =
        Pubkey::find_program_address(&[program().as_ref()], &bpf_loader_upgradeable::id());
    test.add_account(
        program_data_key,
        program_data(upgrade_authority_recorded.then(|| upgrade_authority.pubkey())),
    );
    for (key, account) in extra {
        test.add_account(key, account);
    }
    let ctx = test.start_with_context().await;
    set_unix_timestamp(&ctx, NOW).await;
    let admin = Keypair::new();
    let mut harness = Harness {
        ctx,
        upgrade_authority,
        oracle_authority: admin.insecure_clone(),
        admin,
        usdc_mint: Pubkey::default(),
        usdc_mint_authority: Keypair::new(),
        settlement_signers: Vec::new(),
        recovery_authority: Keypair::new(),
    };
    let mint_authority = harness.usdc_mint_authority.pubkey();
    harness.usdc_mint = harness.create_mint(&mint_authority, 6, None).await;
    let admin_key = harness.admin.pubkey();
    harness.fund(&admin_key, 10_000_000_000).await;
    harness
}

pub async fn start() -> Harness {
    start_with(true, vec![]).await
}

pub async fn set_unix_timestamp(ctx: &ProgramTestContext, unix_timestamp: u64) {
    let mut clock: Clock = ctx.banks_client.get_sysvar().await.unwrap();
    clock.unix_timestamp = unix_timestamp as i64;
    ctx.set_sysvar(&clock);
}

pub fn instruction_error(result: Result<(), BanksClientError>) -> InstructionError {
    match result.expect_err("transaction should fail").unwrap() {
        TransactionError::InstructionError(_, error) => error,
        other => panic!("unexpected transaction error: {other:?}"),
    }
}

pub fn custom(result: Result<(), BanksClientError>) -> u32 {
    match instruction_error(result) {
        InstructionError::Custom(code) => code,
        other => panic!("expected custom error, got {other:?}"),
    }
}

pub fn assert_oracle_error(result: Result<(), BanksClientError>, expected: OracleError) {
    assert_eq!(custom(result), expected as u32, "expected {expected:?}");
}

impl Harness {
    pub fn payer(&self) -> Keypair {
        self.ctx.payer.insecure_clone()
    }

    pub async fn send(
        &mut self,
        ix: Instruction,
        signers: &[&Keypair],
    ) -> Result<(), BanksClientError> {
        self.send_many(vec![ix], signers).await
    }

    pub async fn send_many(
        &mut self,
        ixs: Vec<Instruction>,
        signers: &[&Keypair],
    ) -> Result<(), BanksClientError> {
        #[cfg(not(feature = "localnet-dev-authority"))]
        let ixs = ixs
            .into_iter()
            .map(|mut ix| {
                if ix.program_id == program() {
                    use ameba_oracle::governance_gate::*;
                    ix.data
                        .extend_from_slice(&GovernanceInstructionTailV1::for_epoch(1).encode());
                    ix.accounts
                        .push(solana_sdk::instruction::AccountMeta::new_readonly(
                            derive_protocol_gate_pda(&PINNED_CONTROLLER_PROGRAM_ID, &program()).0,
                            false,
                        ));
                }
                ix
            })
            .collect::<Vec<_>>();
        let payer = self.payer();
        let blockhash = self.ctx.get_new_latest_blockhash().await.unwrap();
        let mut all = vec![&payer];
        for signer in signers {
            if signer.pubkey() != payer.pubkey()
                && !all.iter().any(|s| s.pubkey() == signer.pubkey())
            {
                all.push(signer);
            }
        }
        let tx = Transaction::new_signed_with_payer(&ixs, Some(&payer.pubkey()), &all, blockhash);
        self.ctx.banks_client.process_transaction(tx).await
    }

    pub async fn fund(&mut self, recipient: &Pubkey, lamports: u64) {
        let ix = solana_system_interface::instruction::transfer(
            &self.ctx.payer.pubkey(),
            recipient,
            lamports,
        );
        self.send(ix, &[]).await.unwrap();
    }

    pub async fn create_mint(
        &mut self,
        mint_authority: &Pubkey,
        decimals: u8,
        freeze_authority: Option<&Pubkey>,
    ) -> Pubkey {
        let mint = Keypair::new();
        let rent = self.ctx.banks_client.get_rent().await.unwrap();
        let create = solana_system_interface::instruction::create_account(
            &self.ctx.payer.pubkey(),
            &mint.pubkey(),
            rent.minimum_balance(spl_token::state::Mint::LEN),
            spl_token::state::Mint::LEN as u64,
            &spl_token::id(),
        );
        let init = spl_token::instruction::initialize_mint2(
            &spl_token::id(),
            &mint.pubkey(),
            mint_authority,
            freeze_authority,
            decimals,
        )
        .unwrap();
        self.send_many(vec![create, init], &[&mint]).await.unwrap();
        mint.pubkey()
    }

    pub async fn create_token_account(&mut self, mint: &Pubkey, owner: &Pubkey) -> Pubkey {
        let account = Keypair::new();
        let rent = self.ctx.banks_client.get_rent().await.unwrap();
        let create = solana_system_interface::instruction::create_account(
            &self.ctx.payer.pubkey(),
            &account.pubkey(),
            rent.minimum_balance(spl_token::state::Account::LEN),
            spl_token::state::Account::LEN as u64,
            &spl_token::id(),
        );
        let init = spl_token::instruction::initialize_account3(
            &spl_token::id(),
            &account.pubkey(),
            mint,
            owner,
        )
        .unwrap();
        self.send_many(vec![create, init], &[&account])
            .await
            .unwrap();
        account.pubkey()
    }

    pub async fn create_ata(&mut self, owner: &Pubkey) -> Pubkey {
        let ix =
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &self.ctx.payer.pubkey(),
                owner,
                &self.usdc_mint,
                &spl_token::id(),
            );
        self.send(ix, &[]).await.unwrap();
        spl_associated_token_account::get_associated_token_address_with_program_id(
            owner,
            &self.usdc_mint,
            &spl_token::id(),
        )
    }

    pub async fn mint_usdc(&mut self, destination: &Pubkey, amount: u64) {
        let ix = spl_token::instruction::mint_to(
            &spl_token::id(),
            &self.usdc_mint,
            destination,
            &self.usdc_mint_authority.pubkey(),
            &[],
            amount,
        )
        .unwrap();
        let authority = self.usdc_mint_authority.insecure_clone();
        self.send(ix, &[&authority]).await.unwrap();
    }

    pub async fn token_balance(&mut self, token_account: Pubkey) -> u64 {
        let account = self
            .account(token_account)
            .await
            .expect("token account exists");
        spl_token::state::Account::unpack(&account.data[..spl_token::state::Account::LEN])
            .unwrap()
            .amount
    }

    pub fn bond_vault(&self) -> Pubkey {
        find_bond_vault_address(&program(), &self.usdc_mint)
    }

    pub async fn initialize_with_mint(&mut self, mint: Pubkey) -> Result<(), BanksClientError> {
        let ix = instruction::initialize_config(
            &program(),
            &self.ctx.payer.pubkey(),
            &self.upgrade_authority.pubkey(),
            self.admin.pubkey(),
            &mint,
        );
        let upgrade_authority = self.upgrade_authority.insecure_clone();
        self.send(ix, &[&upgrade_authority]).await
    }

    pub async fn initialize(&mut self) -> Result<(), BanksClientError> {
        let mint = self.usdc_mint;
        self.initialize_with_mint(mint).await
    }

    /// Split the oracle authority off the bootstrap admin (tag 128).
    pub async fn split_oracle_authority(&mut self) {
        let oracle = Keypair::new();
        let ix = instruction::bootstrap_vault_governance_v2(
            &program(),
            &self.admin.pubkey(),
            &oracle.pubkey(),
        );
        let admin = self.admin.insecure_clone();
        self.send(ix, &[&admin, &oracle]).await.unwrap();
        self.oracle_authority = oracle;
    }

    pub async fn create_period(
        &mut self,
        signer: &Keypair,
        product_id: [u8; 32],
        planned: u64,
        expiry: u64,
        decimals: u8,
    ) -> Result<(), BanksClientError> {
        let ix = instruction::create_period(
            &program(),
            &self.ctx.payer.pubkey(),
            &signer.pubkey(),
            product_id,
            planned,
            expiry,
            decimals,
        );
        self.send(ix, &[signer]).await
    }

    pub async fn account(&mut self, key: Pubkey) -> Option<Account> {
        self.ctx.banks_client.get_account(key).await.unwrap()
    }

    pub async fn period(&mut self, product_id: [u8; 32], expiry: u64) -> Period {
        let (key, _) = find_period_address(&program(), &product_id, expiry);
        let mut account = self.account(key).await.expect("period exists");
        let info = AccountInfo::new(
            &key,
            false,
            false,
            &mut account.lamports,
            &mut account.data,
            &account.owner,
            false,
            0,
        );
        load_period_for(&program(), &info, &product_id, expiry).unwrap()
    }

    pub async fn config(&mut self) -> OracleConfig {
        self.read_state(find_config_address(&program()).0).await
    }

    pub async fn read_state<T: BorshDeserialize>(&mut self, key: Pubkey) -> T {
        let account = self.account(key).await.expect("program account exists");
        assert_eq!(account.owner, program());
        T::deserialize(&mut &account.data[..]).expect("deserialize state")
    }

    pub async fn collateral(&mut self, owner: &Pubkey) -> UserCollateral {
        self.read_state(ameba_oracle::state::derive_user_collateral_pda(&program(), owner).0)
            .await
    }

    /// Install program-owned state directly, for accounts whose creating instruction is outside
    /// the tested surface.
    pub async fn set_program_state<T: BorshSerialize>(
        &mut self,
        address: Pubkey,
        value: &T,
        len: usize,
    ) {
        let rent = self.ctx.banks_client.get_rent().await.unwrap();
        let mut data = vec![0; len];
        let serialized = value.try_to_vec().expect("serialize state");
        data[..serialized.len()].copy_from_slice(&serialized);
        let mut account = AccountSharedData::new(rent.minimum_balance(len), len, &program());
        account.set_data_from_slice(&data);
        self.set_account_preserving_capitalization(address, account)
            .await;
    }

    /// Install an account, moving any lamport difference from or to the fee payer so the bank's
    /// capitalization is unchanged (epoch boundaries verify it).
    pub async fn set_account_preserving_capitalization(
        &mut self,
        address: Pubkey,
        account: AccountSharedData,
    ) {
        use solana_sdk::account::{ReadableAccount, WritableAccount};
        let previous = self.account(address).await.map_or(0, |a| a.lamports);
        let payer_key = self.ctx.payer.pubkey();
        let payer = self.account(payer_key).await.expect("payer exists");
        let lamports = payer.lamports + previous - account.lamports();
        let mut payer = AccountSharedData::from(payer);
        payer.set_lamports(lamports);
        self.ctx.set_account(&payer_key, &payer);
        self.ctx.set_account(&address, &account);
    }

    pub async fn set_system_account_with_lamports(&mut self, address: Pubkey, lamports: u64) {
        let account = AccountSharedData::new(lamports, 0, &solana_sdk::system_program::id());
        self.set_account_preserving_capitalization(address, account)
            .await;
    }

    pub async fn advance_slot(&mut self) {
        let clock: Clock = self.ctx.banks_client.get_sysvar().await.unwrap();
        self.ctx.warp_to_slot(clock.slot.saturating_add(1)).unwrap();
        // Warping resets the clock sysvar; keep the harness' wall clock.
        set_unix_timestamp(&self.ctx, clock.unix_timestamp as u64).await;
    }
}

pub fn spl_mint_bytes(decimals: u8, freeze_authority: Option<Pubkey>) -> Vec<u8> {
    let mut data = vec![0; spl_token::state::Mint::LEN];
    spl_token::state::Mint::pack(
        spl_token::state::Mint {
            mint_authority: COption::None,
            supply: 0,
            decimals,
            is_initialized: true,
            freeze_authority: freeze_authority.map_or(COption::None, COption::Some),
        },
        &mut data,
    )
    .unwrap();
    data
}

pub const MAX_CHUNK_IDS: usize = ameba_oracle::constants::MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS;

/// Index-bound terminal-SKU Merkle root, computed independently of the program.
pub fn sku_merkle_root(sku_ids: &[[u8; 32]]) -> [u8; 32] {
    use ameba_oracle::constants::{
        ORACLE_SKU_EMPTY_HASH_DOMAIN, ORACLE_SKU_LEAF_HASH_DOMAIN, ORACLE_SKU_NODE_HASH_DOMAIN,
    };
    use solana_program::hash::hashv;
    let mut width = 1usize;
    while width < sku_ids.len() {
        width *= 2;
    }
    let mut nodes = (0..width)
        .map(|index| {
            let index = index as u16;
            match sku_ids.get(usize::from(index)) {
                Some(sku_id) => {
                    hashv(&[ORACLE_SKU_LEAF_HASH_DOMAIN, &index.to_le_bytes(), sku_id]).to_bytes()
                }
                None => hashv(&[ORACLE_SKU_EMPTY_HASH_DOMAIN, &index.to_le_bytes()]).to_bytes(),
            }
        })
        .collect::<Vec<_>>();
    while nodes.len() > 1 {
        nodes = nodes
            .chunks_exact(2)
            .map(|pair| hashv(&[ORACLE_SKU_NODE_HASH_DOMAIN, &pair[0], &pair[1]]).to_bytes())
            .collect();
    }
    nodes[0]
}

impl Harness {
    /// Send one tag-190 chunk signed by the config admin and the oracle authority.
    pub async fn send_product_sku_chunk(
        &mut self,
        product_id: [u8; 32],
        draft_nonce: u64,
        expected_start_index: u16,
        sku_count: u16,
        expected_root: [u8; 32],
        chunk: &[[u8; 32]],
    ) -> Result<(), BanksClientError> {
        let ix = instruction::configure_oracle_product_sku_manifest(
            &program(),
            &self.admin.pubkey(),
            &self.oracle_authority.pubkey(),
            instruction::ConfigureOracleProductSkuManifestParams {
                underlying_id: product_id,
                draft_nonce,
                expected_start_index,
                sku_count,
                expected_root,
                sku_id_chunk: chunk.to_vec(),
            },
        );
        let admin = self.admin.insecure_clone();
        let oracle = self.oracle_authority.insecure_clone();
        self.send(ix, &[&admin, &oracle]).await
    }

    /// Register a complete product SKU manifest through packet-safe chunks.
    pub async fn register_product(
        &mut self,
        product_id: [u8; 32],
        sku_ids: &[[u8; 32]],
    ) -> [u8; 32] {
        let root = sku_merkle_root(sku_ids);
        let count = sku_ids.len() as u16;
        for (index, chunk) in sku_ids.chunks(MAX_CHUNK_IDS).enumerate() {
            self.send_product_sku_chunk(
                product_id,
                7,
                (index * MAX_CHUNK_IDS) as u16,
                count,
                root,
                chunk,
            )
            .await
            .unwrap();
        }
        root
    }

    pub async fn configure_economics(
        &mut self,
        expected_config_version: u64,
    ) -> Result<(), BanksClientError> {
        let ix = instruction::configure_oracle_economics_template_v2(
            &program(),
            &self.admin.pubkey(),
            instruction::ConfigureOracleEconomicsTemplateV2Params {
                expected_config_version,
                economics: ameba_oracle::state::OracleEconomicParams::default(),
            },
        );
        let admin = self.admin.insecure_clone();
        self.send(ix, &[&admin]).await
    }
}

/// Synthetic ascending SKU identifiers; they are not catalog identities.
pub fn synthetic_sku_ids(count: u16) -> Vec<[u8; 32]> {
    (1..=count)
        .map(|value| {
            let mut id = [0u8; 32];
            id[..2].copy_from_slice(&value.to_be_bytes());
            id
        })
        .collect()
}

pub struct Signers {
    pub set: Pubkey,
    pub recovery: Pubkey,
    pub members: Vec<Pubkey>,
}

/// Install the governed settlement signer registry and its first 3-of-N set directly, for tests
/// that need a registry without its creating instruction (tag 90).
pub async fn install_signer_registry(h: &mut Harness, members: Vec<Pubkey>) -> Signers {
    use ameba_oracle::{
        constants::{
            MAX_SETTLEMENT_SIGNER_COUNT, PRODUCTION_MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS,
        },
        state::{
            derive_settlement_signer_registry_pda, derive_settlement_signer_set_pda,
            SettlementSignerRegistry, SettlementSignerSet,
        },
    };
    let (registry_key, registry_bump) = derive_settlement_signer_registry_pda(&program());
    let (set_key, set_bump) = derive_settlement_signer_set_pda(&program(), 1);
    let mut sorted = members.clone();
    sorted.sort_unstable();
    let mut signers = [Pubkey::default(); MAX_SETTLEMENT_SIGNER_COUNT];
    signers[..sorted.len()].copy_from_slice(&sorted);
    let mut set = SettlementSignerSet {
        is_initialized: true,
        bump: set_bump,
        account_discriminator: SettlementSignerSet::ACCOUNT_DISCRIMINATOR,
        account_version: SettlementSignerSet::ACCOUNT_VERSION,
        registry: registry_key,
        version: 1,
        threshold: 3,
        signer_count: sorted.len() as u8,
        signers,
        set_hash: [0; 32],
        rotation_delay_slots: PRODUCTION_MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS,
        proposed_slot: 0,
        activate_after_slot: 0,
        emergency: false,
        proposer: h.admin.pubkey(),
    };
    set.set_hash = set.compute_set_hash();
    let recovery = Pubkey::new_unique();
    let registry = SettlementSignerRegistry {
        is_initialized: true,
        bump: registry_bump,
        account_discriminator: SettlementSignerRegistry::ACCOUNT_DISCRIMINATOR,
        account_version: SettlementSignerRegistry::ACCOUNT_VERSION,
        current_set: set_key,
        current_version: 1,
        pending_set: Pubkey::default(),
        pending_version: 0,
        recovery_authority: recovery,
        proposal_nonce: 0,
    };
    h.set_program_state(registry_key, &registry, SettlementSignerRegistry::LEN)
        .await;
    h.set_program_state(set_key, &set, SettlementSignerSet::LEN)
        .await;
    Signers {
        set: set_key,
        recovery,
        members: sorted,
    }
}

impl Harness {
    /// Create the signer registry through tag 90 with five signer keys (threshold 3).
    pub async fn initialize_signer_registry(&mut self) -> Signers {
        use ameba_oracle::{
            constants::{MAX_SETTLEMENT_SIGNER_COUNT, MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS},
            instruction::InitializeSettlementSignerRegistryParams,
            state::derive_settlement_signer_set_pda,
        };
        let mut keys: Vec<Keypair> = (0..5).map(|_| Keypair::new()).collect();
        keys.sort_by_key(Signer::pubkey);
        let mut signers = [Pubkey::default(); MAX_SETTLEMENT_SIGNER_COUNT];
        for (slot, key) in signers.iter_mut().zip(&keys) {
            *slot = key.pubkey();
        }
        let recovery = self.recovery_authority.insecure_clone();
        let ix = instruction::initialize_settlement_signer_registry(
            &program(),
            &self.admin.pubkey(),
            &self.oracle_authority.pubkey(),
            &recovery.pubkey(),
            InitializeSettlementSignerRegistryParams {
                signer_set_version: 1,
                threshold: 3,
                signer_count: 5,
                signers,
                rotation_delay_slots: MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS,
                recovery_authority: recovery.pubkey(),
            },
        );
        let admin = self.admin.insecure_clone();
        let oracle = self.oracle_authority.insecure_clone();
        self.send(ix, &[&admin, &oracle, &recovery]).await.unwrap();
        let members = keys.iter().map(Signer::pubkey).collect();
        self.settlement_signers = keys;
        Signers {
            set: derive_settlement_signer_set_pda(&program(), 1).0,
            recovery: recovery.pubkey(),
            members,
        }
    }

    /// Split the authorities, create the signer registry (tag 90) and unpause through tag 129.
    pub async fn activate(&mut self) -> Signers {
        if self.oracle_authority.pubkey() == self.admin.pubkey() {
            self.split_oracle_authority().await;
        }
        let signers = self.initialize_signer_registry().await;
        let ix = instruction::activate_vault_v2(
            &program(),
            &self.admin.pubkey(),
            &signers.set,
            &self.usdc_mint,
            None,
        );
        let admin = self.admin.insecure_clone();
        self.send(ix, &[&admin]).await.unwrap();
        assert!(!self.config().await.paused);
        signers
    }
}

/// An Ed25519 precompile instruction carrying `signer`'s signature over `message` inline.
pub fn ed25519_verify_instruction(signer: &Keypair, message: &[u8]) -> Instruction {
    const DATA_START: usize = 16;
    const PUBKEY_SIZE: usize = 32;
    const SIGNATURE_SIZE: usize = 64;
    let public_key_offset = DATA_START as u16;
    let signature_offset = (DATA_START + PUBKEY_SIZE) as u16;
    let message_offset = (DATA_START + PUBKEY_SIZE + SIGNATURE_SIZE) as u16;
    let signature = signer.sign_message(message);
    let mut data = Vec::with_capacity(usize::from(message_offset) + message.len());
    data.extend_from_slice(&[1, 0]);
    data.extend_from_slice(&signature_offset.to_le_bytes());
    data.extend_from_slice(&u16::MAX.to_le_bytes());
    data.extend_from_slice(&public_key_offset.to_le_bytes());
    data.extend_from_slice(&u16::MAX.to_le_bytes());
    data.extend_from_slice(&message_offset.to_le_bytes());
    data.extend_from_slice(&(message.len() as u16).to_le_bytes());
    data.extend_from_slice(&u16::MAX.to_le_bytes());
    data.extend_from_slice(signer.pubkey().as_ref());
    data.extend_from_slice(signature.as_ref());
    data.extend_from_slice(message);
    Instruction {
        program_id: solana_sdk_ids::ed25519_program::id(),
        accounts: vec![],
        data,
    }
}
