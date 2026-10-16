//! Actual-SBF rehearsals for the 2 MiB ceiling and ProgramData growth. Every
//! test runs this worktree's controller ELF (`AMOEBA_GOV_V3_SBF`) under the
//! real Loader-v3 in ProgramTest.
//!
//! - 128 chunks (2,097,152 B) verify, approve and install; per-instruction
//!   compute is printed and bounded.
//! - ProgramData grows only through the permissionless top-level Loader-v3
//!   `ExtendProgram`, at any time before installation. The retired
//!   checked-extension tags 10 and 13 fail like unknown tags.
use borsh::{BorshDeserialize, BorshSerialize};
use governance_controller_v3::{artifact, instruction::Instruction as Gov, state::*};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    rent::Rent,
};
use solana_program_test::{ProgramTest, ProgramTestContext};
use solana_sdk::{
    account::Account,
    compute_budget::ComputeBudgetInstruction,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use solana_sdk_ids::{bpf_loader_upgradeable::ID as LOADER, system_program, sysvar};
use upgrade_controller::{
    release1_loader_accounts::parse_upgradeable_programdata,
    state::{GateStatusV1, ProtocolGateV1},
};

type Sent = std::result::Result<u64, String>;
/// `enable_extend_program_checked`, as known to the pinned 2.3.13 runtime.
/// Upstream cancelled it (Agave master replaced its key with the placeholder
/// `ExtendProgCheckedWi11BeDe1eted11111111111111`), so on real clusters legacy
/// `ExtendProgram` stays available. ProgramTest activates every known feature
/// by default, so each bank deactivates this one to match.
const EXTEND_CHECKED_FEATURE: Pubkey =
    solana_program::pubkey!("2oMRZEDWT2tqtYMofhmmfQ8SsjqUFzT6sYXppQDavxwz");
/// Conservative per-instruction ceilings for the measured SBF paths.
const MAX_VERIFY_CHUNK_CU: u64 = 200_000;
const MAX_APPROVE_CU: u64 = 100_000;
const MAX_EXECUTE_CU: u64 = 400_000;

fn elf() -> Vec<u8> {
    let file = std::env::var("AMOEBA_GOV_V3_SBF")
        .expect("AMOEBA_GOV_V3_SBF must select this worktree's final ELF");
    let bytes = std::fs::read(file).unwrap();
    assert!(bytes.starts_with(b"\x7fELF"));
    bytes
}
/// The ELF followed by a nonzero pattern, so installation is observable.
fn artifact_of(elf: &[u8], len: usize) -> Vec<u8> {
    assert!(len >= elf.len());
    let mut bytes = elf.to_vec();
    bytes.extend((elf.len()..len).map(|i| (i % 251) as u8 | 1));
    bytes
}
fn account(owner: Pubkey, data: Vec<u8>, executable: bool) -> Account {
    Account {
        lamports: Rent::default().minimum_balance(data.len()).max(1),
        data,
        owner,
        executable,
        rent_epoch: 0,
    }
}
fn programdata_bytes(authority: &Pubkey, payload: &[u8]) -> Vec<u8> {
    let mut pd = 3u32.to_le_bytes().to_vec();
    pd.extend_from_slice(&0u64.to_le_bytes());
    pd.push(1);
    pd.extend_from_slice(authority.as_ref());
    pd.extend_from_slice(payload);
    pd
}
fn ro(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, false)
}
fn rw(key: Pubkey) -> AccountMeta {
    AccountMeta::new(key, false)
}
fn sig(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, true)
}
fn ix(program: Pubkey, accounts: Vec<AccountMeta>, value: Gov) -> Instruction {
    Instruction {
        program_id: program,
        accounts,
        data: value.try_to_vec().unwrap(),
    }
}

struct Harness {
    ctx: ProgramTestContext,
    slot: u64,
    program: Pubkey,
    programdata: Pubkey,
    target: Pubkey,
    target_pd: Pubkey,
    deployer: Keypair,
    uploader: Keypair,
    seats: [Keypair; 5],
    treasury: Pubkey,
    /// Unrelated funded signer standing in for any third party.
    griefer: Keypair,
    /// The buffer used by the proposal currently being driven.
    buffer: Pubkey,
    buffers: Vec<Pubkey>,
}
impl Harness {
    /// Controller and target ProgramData hold the given payloads; one
    /// uploader-owned buffer per artifact.
    async fn start(controller_payload: &[u8], target_payload: &[u8], artifact: &[u8]) -> Self {
        Self::start_with(controller_payload, target_payload, &[artifact]).await
    }
    async fn start_with(
        controller_payload: &[u8],
        target_payload: &[u8],
        artifacts: &[&[u8]],
    ) -> Self {
        let program = Pubkey::new_unique();
        let programdata = Pubkey::find_program_address(&[program.as_ref()], &LOADER).0;
        let target = Pubkey::new_unique();
        let target_pd = Pubkey::find_program_address(&[target.as_ref()], &LOADER).0;
        let deployer = Keypair::new();
        let uploader = Keypair::new();
        let seats: [Keypair; 5] = std::array::from_fn(|_| Keypair::new());
        let treasury = Pubkey::new_unique();
        let griefer = Keypair::new();
        let buffers: Vec<Pubkey> = artifacts.iter().map(|_| Pubkey::new_unique()).collect();
        let buffer = buffers[0];
        let mut test = ProgramTest::default();
        test.prefer_bpf(true);
        test.deactivate_feature(EXTEND_CHECKED_FEATURE);
        for (key, pd_key, payload) in [
            (program, programdata, controller_payload),
            (target, target_pd, target_payload),
        ] {
            let mut program_bytes = 2u32.to_le_bytes().to_vec();
            program_bytes.extend_from_slice(pd_key.as_ref());
            test.add_genesis_account(key, account(LOADER, program_bytes, true));
            test.add_genesis_account(
                pd_key,
                account(
                    LOADER,
                    programdata_bytes(&deployer.pubkey(), payload),
                    false,
                ),
            );
        }
        for (key, artifact) in buffers.iter().zip(artifacts) {
            let mut buffer_bytes = 1u32.to_le_bytes().to_vec();
            buffer_bytes.push(1);
            buffer_bytes.extend_from_slice(uploader.pubkey().as_ref());
            buffer_bytes.extend_from_slice(artifact);
            test.add_account(*key, account(LOADER, buffer_bytes, false));
        }
        for key in [
            deployer.pubkey(),
            uploader.pubkey(),
            treasury,
            authority_pda(&program).0,
            target_authority_pda(&program, &target).0,
        ]
        .into_iter()
        .chain(seats.iter().map(Signer::pubkey))
        {
            test.add_account(key, account(system_program::ID, vec![], false));
        }
        test.add_account(
            griefer.pubkey(),
            Account {
                lamports: 100_000_000_000,
                ..account(system_program::ID, vec![], false)
            },
        );
        let mut ctx = test.start_with_context().await;
        ctx.warp_to_slot(100).unwrap();
        let mut h = Self {
            ctx,
            slot: 100,
            program,
            programdata,
            target,
            target_pd,
            deployer,
            uploader,
            seats,
            treasury,
            griefer,
            buffer,
            buffers,
        };
        let initialize = ix(
            program,
            vec![
                AccountMeta::new(h.ctx.payer.pubkey(), true),
                sig(h.deployer.pubkey()),
                ro(program),
                rw(programdata),
                rw(config_pda(&program).0),
                ro(authority_pda(&program).0),
                ro(LOADER),
                ro(system_program::ID),
                sig(h.seats[0].pubkey()),
                sig(h.seats[1].pubkey()),
                sig(h.seats[2].pubkey()),
            ],
            Gov::Initialize {
                seats: std::array::from_fn(|i| h.seats[i].pubkey()),
                treasury,
            },
        );
        let signers = [
            h.deployer.insecure_clone(),
            h.seats[0].insecure_clone(),
            h.seats[1].insecure_clone(),
            h.seats[2].insecure_clone(),
        ];
        h.ok(
            initialize.clone(),
            &signers.iter().collect::<Vec<_>>(),
            "bootstrap",
        )
        .await;
        let register = ix(
            program,
            vec![
                AccountMeta::new(h.ctx.payer.pubkey(), true),
                sig(h.deployer.pubkey()),
                ro(config_pda(&program).0),
                ro(target),
                rw(target_pd),
                rw(gate_pda(&program, &target).0),
                ro(target_authority_pda(&program, &target).0),
                ro(LOADER),
                ro(system_program::ID),
                sig(h.seats[0].pubkey()),
                sig(h.seats[1].pubkey()),
                sig(h.seats[2].pubkey()),
                ro(sysvar::instructions::ID),
            ],
            Gov::RegisterTarget,
        );
        h.ok(
            register,
            &signers.iter().collect::<Vec<_>>(),
            "register target",
        )
        .await;
        h.warp(1).await;
        h
    }
    /// Continue on a fresh ProgramTest bank holding this harness's accounts.
    /// ProgramTest keeps executing the genesis ELF of a program upgraded by
    /// CPI once a later warp roots the bank (ProgramData already holds the
    /// new bytes), so code installed in-test must be reloaded this way before
    /// it is exercised. Every account carries over; only the ProgramData
    /// deployment slots restart at genesis (0), which no proposal check reads.
    async fn restart(mut self) -> Self {
        let next_id = {
            let config = self
                .ctx
                .banks_client
                .get_account(config_pda(&self.program).0)
                .await
                .unwrap()
                .unwrap();
            Config::try_from_slice(&config.data).unwrap().next_id
        };
        let mut keys = vec![
            config_pda(&self.program).0,
            gate_pda(&self.program, &self.target).0,
            authority_pda(&self.program).0,
            target_authority_pda(&self.program, &self.target).0,
            self.treasury,
            self.griefer.pubkey(),
            self.deployer.pubkey(),
            self.uploader.pubkey(),
        ];
        keys.extend(self.seats.iter().map(Signer::pubkey));
        keys.extend(self.buffers.iter().copied());
        keys.extend((1..next_id).map(|id| proposal_pda(&self.program, id).0));
        let mut test = ProgramTest::default();
        test.prefer_bpf(true);
        test.deactivate_feature(EXTEND_CHECKED_FEATURE);
        for key in [self.program, self.programdata, self.target, self.target_pd] {
            let mut account = self
                .ctx
                .banks_client
                .get_account(key)
                .await
                .unwrap()
                .unwrap();
            if key == self.programdata || key == self.target_pd {
                account.data[4..12].fill(0);
            }
            test.add_genesis_account(key, account);
        }
        for key in keys {
            if let Some(account) = self.ctx.banks_client.get_account(key).await.unwrap() {
                test.add_account(key, account);
            }
        }
        self.ctx = test.start_with_context().await;
        self.slot += 1;
        self.ctx.warp_to_slot(self.slot).unwrap();
        self
    }
    /// Permissionless legacy top-level ExtendProgram paid by the third party.
    async fn grief(&mut self, program: Pubkey, bytes: u32) {
        let griefer = self.griefer.insecure_clone();
        let extend = solana_loader_v3_interface::instruction::extend_program(
            &program,
            Some(&griefer.pubkey()),
            bytes,
        );
        self.ok(extend, &[&griefer], "third-party ExtendProgram")
            .await;
        self.warp(1).await;
    }
    async fn warp(&mut self, slots: u64) {
        self.slot += slots;
        self.ctx.warp_to_slot(self.slot).unwrap();
    }
    async fn send(&mut self, instruction: Instruction, signers: &[&Keypair]) -> Sent {
        let blockhash = self.ctx.banks_client.get_latest_blockhash().await.unwrap();
        let mut keys = vec![&self.ctx.payer];
        keys.extend_from_slice(signers);
        let transaction = Transaction::new_signed_with_payer(
            &[
                ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
                instruction,
            ],
            Some(&self.ctx.payer.pubkey()),
            &keys,
            blockhash,
        );
        let outcome = self
            .ctx
            .banks_client
            .process_transaction_with_metadata(transaction)
            .await
            .map_err(|e| e.to_string())?;
        let units = outcome
            .metadata
            .as_ref()
            .map(|m| m.compute_units_consumed)
            .unwrap_or_default();
        match outcome.result {
            Ok(()) => Ok(units),
            Err(error) => {
                let logs = outcome.metadata.map(|m| m.log_messages).unwrap_or_default();
                Err(format!("{error:?} {logs:#?}"))
            }
        }
    }
    async fn ok(&mut self, instruction: Instruction, signers: &[&Keypair], what: &str) -> u64 {
        match self.send(instruction, signers).await {
            Ok(units) => units,
            Err(error) => panic!("{what}: {error}"),
        }
    }
    async fn fails(&mut self, instruction: Instruction, signers: &[&Keypair], what: &str) {
        assert!(
            self.send(instruction, signers).await.is_err(),
            "{what} unexpectedly succeeded"
        );
    }
    async fn header(&mut self, key: Pubkey) -> (u64, u64, Vec<u8>) {
        let account = self
            .ctx
            .banks_client
            .get_account(key)
            .await
            .unwrap()
            .unwrap();
        let header = parse_upgradeable_programdata(&account.data).unwrap();
        (
            header.deployed_slot,
            header.capacity as u64,
            account.data[45..].to_vec(),
        )
    }
    async fn proposal(&mut self, id: u64) -> Proposal {
        let key = proposal_pda(&self.program, id).0;
        let account = self
            .ctx
            .banks_client
            .get_account(key)
            .await
            .unwrap()
            .unwrap();
        Proposal::try_from_slice(&account.data).unwrap()
    }
    async fn create(&mut self, id: u64, action: Action) -> Sent {
        let seat = self.seats[0].insecure_clone();
        let create = ix(
            self.program,
            vec![
                AccountMeta::new(self.ctx.payer.pubkey(), true),
                sig(seat.pubkey()),
                rw(config_pda(&self.program).0),
                rw(proposal_pda(&self.program, id).0),
                ro(system_program::ID),
            ],
            Gov::Create {
                expected_id: id,
                action,
            },
        );
        self.send(create, &[&seat]).await
    }
    async fn target_action(&mut self, artifact: &[u8]) -> Action {
        let (deployed_slot, capacity, _) = self.header(self.target_pd).await;
        let body = TargetUpgrade {
            buffer: self.buffer,
            artifact_length: artifact.len() as u64,
            merkle_root: artifact::merkle_root(artifact).unwrap(),
            deployed_slot,
            capacity,
            source_commitment: [1; 32],
            build_commitment: [2; 32],
            target: self.target,
            gate_epoch: 1,
        };
        let mut data = [0; ACTION_LEN];
        data.copy_from_slice(&body.try_to_vec().unwrap());
        Action {
            kind: UPGRADE_TARGET,
            data,
        }
    }
    async fn controller_action(&mut self, artifact: &[u8], release: u64) -> Action {
        let (deployed_slot, capacity, _) = self.header(self.programdata).await;
        let body = Upgrade {
            buffer: self.buffer,
            artifact_length: artifact.len() as u64,
            artifact_sha256: solana_program::hash::hashv(&[artifact]).to_bytes(),
            merkle_root: artifact::merkle_root(artifact).unwrap(),
            deployed_slot,
            capacity,
            source_commitment: [1; 32],
            build_commitment: [2; 32],
            controller_release: release,
        };
        let mut data = [0; ACTION_LEN];
        data.copy_from_slice(&body.try_to_vec().unwrap());
        Action {
            kind: UPGRADE_CONTROLLER,
            data,
        }
    }
    /// Seal, verify every chunk (returning the maximum compute), and check
    /// that approval is impossible until the last chunk is verified.
    async fn seal_and_verify(&mut self, id: u64, artifact: &[u8]) -> u64 {
        self.seal(id).await;
        let count = artifact::chunk_count(artifact.len() as u64).unwrap();
        let units = self.verify_range(id, artifact, 0..count).await;
        assert_eq!(self.proposal(id).await.verified_count, count);
        units
    }
    async fn seal(&mut self, id: u64) {
        let p = self.proposal(id).await;
        let key = proposal_pda(&self.program, id).0;
        let config = config_pda(&self.program).0;
        let uploader = self.uploader.insecure_clone();
        let seal = ix(
            self.program,
            vec![
                ro(config),
                ro(key),
                rw(self.buffer),
                sig(uploader.pubkey()),
                ro(buffer_authority_pda(&self.program, &key).0),
                ro(LOADER),
            ],
            Gov::SealBuffer { digest: p.digest },
        );
        self.ok(seal, &[&uploader], "seal").await;
    }
    async fn verify_range(&mut self, id: u64, artifact: &[u8], range: std::ops::Range<u32>) -> u64 {
        let p = self.proposal(id).await;
        let key = proposal_pda(&self.program, id).0;
        let config = config_pda(&self.program).0;
        let count = artifact::chunk_count(artifact.len() as u64).unwrap();
        let mut max_units = 0;
        for index in range {
            if index + 1 == count {
                let seat = self.seats[0].insecure_clone();
                let early = ix(
                    self.program,
                    vec![ro(config), rw(key), sig(seat.pubkey())],
                    Gov::Approve { digest: p.digest },
                );
                self.fails(early, &[&seat], "approval before the last chunk")
                    .await;
            }
            let branch = artifact::merkle_proof(artifact, index).unwrap();
            let mut proof = [[0; 32]; 7];
            proof[..branch.len()].copy_from_slice(&branch);
            let verify = ix(
                self.program,
                vec![ro(config), rw(key), ro(self.buffer)],
                Gov::VerifyChunk {
                    digest: p.digest,
                    index,
                    proof_len: branch.len() as u8,
                    proof,
                },
            );
            let units = self.ok(verify.clone(), &[], "verify chunk").await;
            max_units = max_units.max(units);
            if index == 0 {
                self.warp(1).await;
                self.fails(verify, &[], "duplicate chunk").await;
            }
        }
        max_units
    }
    /// Three approvals after the delay; returns the maximum approval compute.
    async fn approve(&mut self, id: u64) -> u64 {
        let p = self.proposal(id).await;
        if self.slot <= p.not_before {
            self.warp(p.not_before + 1 - self.slot).await;
        }
        let key = proposal_pda(&self.program, id).0;
        let config = config_pda(&self.program).0;
        let mut max_units = 0;
        for i in 0..3 {
            let seat = self.seats[i].insecure_clone();
            let approve = ix(
                self.program,
                vec![ro(config), rw(key), sig(seat.pubkey())],
                Gov::Approve { digest: p.digest },
            );
            max_units = max_units.max(self.ok(approve, &[&seat], "approve").await);
        }
        max_units
    }
    /// A retired checked-extension instruction (tag 10 or 13) with a
    /// well-formed digest and the account list it once took.
    fn retired_extension(&self, tag: u8, id: u64, digest: [u8; 32]) -> Instruction {
        let (program, programdata, authority) = if tag == 13 {
            (
                self.target,
                self.target_pd,
                target_authority_pda(&self.program, &self.target).0,
            )
        } else {
            (
                self.program,
                self.programdata,
                authority_pda(&self.program).0,
            )
        };
        let mut accounts = vec![
            ro(config_pda(&self.program).0),
            ro(proposal_pda(&self.program, id).0),
            rw(program),
            rw(programdata),
            rw(authority),
            ro(LOADER),
            ro(system_program::ID),
            AccountMeta::new(self.ctx.payer.pubkey(), true),
            ro(sysvar::instructions::ID),
        ];
        if tag == 13 {
            accounts.push(rw(gate_pda(&self.program, &self.target).0));
        }
        let mut data = vec![tag];
        data.extend_from_slice(&digest);
        Instruction {
            program_id: self.program,
            accounts,
            data,
        }
    }
    fn execute_target(&self, id: u64, digest: [u8; 32]) -> Instruction {
        let key = proposal_pda(&self.program, id).0;
        ix(
            self.program,
            vec![
                ro(config_pda(&self.program).0),
                rw(key),
                rw(self.target),
                rw(self.target_pd),
                rw(self.buffer),
                rw(self.treasury),
                ro(target_authority_pda(&self.program, &self.target).0),
                ro(buffer_authority_pda(&self.program, &key).0),
                ro(LOADER),
                ro(sysvar::rent::ID),
                ro(sysvar::clock::ID),
                ro(sysvar::instructions::ID),
                rw(gate_pda(&self.program, &self.target).0),
            ],
            Gov::ExecuteTargetUpgrade { digest },
        )
    }
    fn execute_controller(&self, id: u64, digest: [u8; 32]) -> Instruction {
        let key = proposal_pda(&self.program, id).0;
        ix(
            self.program,
            vec![
                ro(config_pda(&self.program).0),
                rw(key),
                rw(self.program),
                rw(self.programdata),
                rw(self.buffer),
                rw(self.treasury),
                ro(authority_pda(&self.program).0),
                ro(buffer_authority_pda(&self.program, &key).0),
                ro(LOADER),
                ro(sysvar::rent::ID),
                ro(sysvar::clock::ID),
                ro(sysvar::instructions::ID),
            ],
            Gov::ExecuteControllerUpgrade { digest },
        )
    }
    async fn gate(&mut self) -> ProtocolGateV1 {
        let key = gate_pda(&self.program, &self.target).0;
        let account = self
            .ctx
            .banks_client
            .get_account(key)
            .await
            .unwrap()
            .unwrap();
        ProtocolGateV1::try_from_slice(&account.data).unwrap()
    }
}

#[tokio::test]
#[ignore = "requires AMOEBA_GOV_V3_SBF; explicitly run by the SBF CI job"]
async fn actual_sbf_two_mib_target_verifies_128_chunks_and_installs() {
    let elf = elf();
    let max = artifact::MAX_ARTIFACT_BYTES as usize;
    let artifact = artifact_of(&elf, max);
    let mut padded = elf.clone();
    padded.resize(max, 0);
    let mut h = Harness::start(&elf, &padded, &artifact).await;
    assert_eq!(h.header(h.target_pd).await.1, 2_097_152);

    // One byte above the ceiling is rejected at creation.
    let mut over = h.target_action(&artifact).await;
    over.data[32..40].copy_from_slice(&2_097_153u64.to_le_bytes());
    assert!(h.create(1, over).await.is_err(), "2,097,153 B");
    let action = h.target_action(&artifact).await;
    let create_units = h.create(1, action).await.expect("create 2 MiB proposal");
    let verify_units = h.seal_and_verify(1, &artifact).await;
    let p = h.proposal(1).await;
    assert_eq!(p.verified_count, 128);
    assert_eq!(p.verified_chunks, [0xff; 12]);
    assert_eq!(p.verified_chunks_high, [0xff; 4]);
    assert_eq!(p.reserved, [0; 3]);
    let approve_units = h.approve(1).await;
    h.warp(1).await;
    let execute = h.execute_target(1, p.digest);
    let execute_units = h.ok(execute, &[], "install 2 MiB target").await;
    let (slot, capacity, installed) = h.header(h.target_pd).await;
    assert_eq!(slot, h.slot);
    assert_eq!(capacity, 2_097_152);
    assert_eq!(installed, artifact);
    let gate = h.gate().await;
    assert_eq!(gate.epoch, 2);
    assert_eq!(gate.status, GateStatusV1::EmergencyFrozen);
    assert_eq!(h.proposal(1).await.state, EXECUTED);
    println!(
        "AMOEBA_GOV_V3_2MIB_CU={{\"artifact_bytes\":{},\"chunks\":128,\"create\":{create_units},\"verify_chunk_max\":{verify_units},\"approve_max\":{approve_units},\"execute_target_upgrade\":{execute_units}}}",
        artifact.len()
    );
    assert!(verify_units <= MAX_VERIFY_CHUNK_CU, "{verify_units}");
    assert!(approve_units <= MAX_APPROVE_CU, "{approve_units}");
    assert!(execute_units <= MAX_EXECUTE_CU, "{execute_units}");
}

/// A proposal may be created with a snapshot below the artifact length;
/// installation waits until anyone grows ProgramData top-level. The retired
/// checked-extension tags fail like unknown tags and change nothing.
#[tokio::test]
#[ignore = "requires AMOEBA_GOV_V3_SBF; explicitly run by the SBF CI job"]
async fn actual_sbf_target_installs_after_legacy_growth_and_retired_tags_fail() {
    let elf = elf();
    let artifact = artifact_of(&elf, elf.len() + 25_000);
    let mut h = Harness::start(&elf, &elf, &artifact).await;
    let (_, start_capacity, _) = h.header(h.target_pd).await;
    assert_eq!(start_capacity, elf.len() as u64);
    let action = h.target_action(&artifact).await;
    h.create(1, action)
        .await
        .expect("create with smaller snapshot");
    h.seal_and_verify(1, &artifact).await;
    h.approve(1).await;
    let p = h.proposal(1).await;
    h.warp(1).await;
    let before = h.header(h.target_pd).await;
    for tag in [10, 13] {
        h.fails(
            h.retired_extension(tag, 1, p.digest),
            &[],
            "retired checked-extension tag",
        )
        .await;
    }
    assert_eq!(h.header(h.target_pd).await, before);
    let stored = h.proposal(1).await;
    assert_eq!((stored.extension_slot, stored.extended_capacity), (0, 0));
    // Installation cannot run before ProgramData covers the artifact.
    h.fails(h.execute_target(1, p.digest), &[], "install before growth")
        .await;
    // Legacy top-level growth by anyone completes the path, and it does not
    // stale the approved proposal.
    let target = h.target;
    h.grief(target, 25_000).await;
    let units = h
        .ok(
            h.execute_target(1, p.digest),
            &[],
            "install after legacy growth",
        )
        .await;
    let (_, capacity, installed) = h.header(h.target_pd).await;
    assert_eq!(capacity, artifact.len() as u64);
    assert_eq!(installed, artifact);
    assert_eq!(h.gate().await.epoch, 2);
    assert!(units <= MAX_EXECUTE_CU, "{units}");
}

/// The controller's own ProgramData grows the same way: a self-upgrade whose
/// artifact exceeds the allocation installs once anyone extends it top-level.
#[tokio::test]
#[ignore = "requires AMOEBA_GOV_V3_SBF; explicitly run by the SBF CI job"]
async fn actual_sbf_controller_self_upgrades_after_legacy_growth() {
    let elf = elf();
    let artifact = artifact_of(&elf, elf.len() + 12_000);
    let mut h = Harness::start(&elf, &elf, &artifact).await;
    let action = h.controller_action(&artifact, CONTROLLER_RELEASE).await;
    h.create(1, action).await.expect("create");
    h.seal_and_verify(1, &artifact).await;
    h.approve(1).await;
    let p = h.proposal(1).await;
    h.warp(1).await;
    h.fails(
        h.execute_controller(1, p.digest),
        &[],
        "self-upgrade before growth",
    )
    .await;
    let program = h.program;
    h.grief(program, 12_000).await;
    assert_eq!(h.header(h.programdata).await.1, artifact.len() as u64);
    h.ok(h.execute_controller(1, p.digest), &[], "self-upgrade")
        .await;
    assert_eq!(h.header(h.programdata).await.2, artifact);
    // The upgraded controller still executes and opens its next proposal.
    h.warp(1).await;
    let mut timing = [0; ACTION_LEN];
    timing[..24].copy_from_slice(&Timing::default().try_to_vec().unwrap());
    h.create(
        2,
        Action {
            kind: SET_TIMING,
            data: timing,
        },
    )
    .await
    .expect("upgraded controller remains usable");
}

fn sha256_hex(bytes: &[u8]) -> String {
    solana_program::hash::hash(bytes)
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The deployed controller (Mainnet artifact e06bf76a, 219,584 B in a
/// 223,096 B allocation) installs this controller through its own council
/// path after a legacy top-level extension, and a proposal it created and
/// partly verified completes under the new binary.
#[tokio::test]
#[ignore = "requires AMOEBA_GOV_V3_SBF and AMOEBA_GOV_V3_DEPLOYED_SBF"]
async fn actual_sbf_deployed_controller_installs_this_controller_and_its_proposals_survive() {
    const ALLOCATED: usize = 223_096;
    let new = elf();
    let deployed = std::fs::read(
        std::env::var("AMOEBA_GOV_V3_DEPLOYED_SBF")
            .expect("AMOEBA_GOV_V3_DEPLOYED_SBF must select the deployed controller ELF"),
    )
    .unwrap();
    assert_eq!(
        sha256_hex(&deployed),
        "e06bf76acfacbf7e58f9bcdd0637051ea1ca629951e41a7b47738e082e51a8eb"
    );
    let mut controller_pd = deployed.clone();
    controller_pd.resize(ALLOCATED, 0);
    // An ordinary target upgrade within the old 1.5 MiB bound, pre-sized.
    let target_artifact = artifact_of(&deployed, deployed.len() + 40_000);
    let mut target_pd = deployed.clone();
    target_pd.resize(target_artifact.len(), 0);
    let mut h =
        Harness::start_with(&controller_pd, &target_pd, &[&target_artifact, &new, &new]).await;
    let target_buffer = h.buffers[0];
    let controller_buffer = h.buffers[1];
    let rival_buffer = h.buffers[2];

    // Proposal 1 is created and half verified by the deployed binary.
    h.buffer = target_buffer;
    let action = h.target_action(&target_artifact).await;
    h.create(1, action)
        .await
        .expect("deployed controller creates");
    h.seal(1).await;
    let count = artifact::chunk_count(target_artifact.len() as u64).unwrap();
    h.verify_range(1, &target_artifact, 0..count / 2).await;
    let old_bytes = h
        .ctx
        .banks_client
        .get_account(proposal_pda(&h.program, 1).0)
        .await
        .unwrap()
        .unwrap()
        .data;
    assert_eq!(&old_bytes[393..], &[0; 7]);

    // The deployed controller only accepts a self-upgrade whose snapshot
    // already covers the artifact, so a larger binary needs legacy growth.
    h.buffer = controller_buffer;
    if new.len() > ALLOCATED {
        let too_small = h.controller_action(&new, 0).await;
        assert!(h.create(2, too_small).await.is_err());
        let legacy = solana_loader_v3_interface::instruction::extend_program(
            &h.program,
            Some(&h.ctx.payer.pubkey()),
            (new.len() - ALLOCATED) as u32,
        );
        h.ok(legacy, &[], "legacy top-level controller extension")
            .await;
        h.warp(1).await;
    }
    // The deployed binary requires the eight release bytes to be zero.
    let action = h.controller_action(&new, CONTROLLER_RELEASE).await;
    assert!(h.create(2, action).await.is_err());
    let action = h.controller_action(&new, 0).await;
    h.create(2, action).await.expect("self-upgrade proposal");
    h.seal_and_verify(2, &new).await;
    // A rival self-upgrade, also approved under the deployed binary.
    h.buffer = rival_buffer;
    let action = h.controller_action(&new, 0).await;
    h.create(3, action)
        .await
        .expect("rival self-upgrade proposal");
    h.seal_and_verify(3, &new).await;
    h.approve(3).await;
    h.buffer = controller_buffer;
    h.approve(2).await;
    h.warp(1).await;
    let p2 = h.proposal(2).await;
    let p3 = h.proposal(3).await;
    h.ok(
        h.execute_controller(2, p2.digest),
        &[],
        "deployed controller installs this controller",
    )
    .await;
    let installed = h.header(h.programdata).await.2;
    assert_eq!(&installed[..new.len()], new.as_slice());
    assert!(installed[new.len()..].iter().all(|b| *b == 0));
    // From here on the installed binary (this controller) must execute.
    let mut h = h.restart().await;

    // Release binding retires every self-upgrade of the deployed binary,
    // even though ProgramData length and slot no longer gate execution.
    h.buffer = rival_buffer;
    h.fails(
        h.execute_controller(3, p3.digest),
        &[],
        "rival release-0 self-upgrade after installation",
    )
    .await;
    for release in [0, CONTROLLER_RELEASE + 1] {
        let action = h.controller_action(&new, release).await;
        assert!(h.create(4, action).await.is_err(), "release {release}");
    }
    let action = h.controller_action(&new, CONTROLLER_RELEASE).await;
    h.create(4, action).await.expect("current-release proposal");

    // The new binary reads and finishes the old proposal unchanged, and a
    // third party's one-byte growth of the target no longer stales it.
    h.buffer = target_buffer;
    let target = h.target;
    h.grief(target, 1).await;
    h.verify_range(1, &target_artifact, count / 2..count).await;
    let p1 = h.proposal(1).await;
    assert_eq!(p1.verified_count, count);
    assert_eq!(p1.verified_chunks_high, [0; 4]);
    h.approve(1).await;
    h.warp(1).await;
    h.ok(
        h.execute_target(1, p1.digest),
        &[],
        "new controller installs the old proposal",
    )
    .await;
    let installed = h.header(h.target_pd).await.2;
    assert_eq!(installed.len(), target_artifact.len() + 1);
    assert_eq!(
        &installed[..target_artifact.len()],
        target_artifact.as_slice()
    );
    assert_eq!(installed[target_artifact.len()], 0);
    println!(
        "AMOEBA_GOV_V3_DEPLOYED_TO_NEW={{\"deployed_bytes\":{},\"allocated\":{ALLOCATED},\"new_bytes\":{},\"legacy_extension_bytes\":{}}}",
        deployed.len(),
        new.len(),
        new.len().saturating_sub(ALLOCATED)
    );
}

/// The production release-1 binary must authorize the release-2 upgrade, and
/// the installed binary must reject a second self-upgrade approved by release 1.
#[tokio::test]
#[ignore = "requires current candidate and exact release-1 controller SBF"]
async fn actual_sbf_release_one_installs_release_two_and_retires_old_proposals() {
    const ALLOCATED: usize = 223_096;
    let new = elf();
    let deployed = std::fs::read(
        std::env::var("AMOEBA_GOV_V3_RELEASE_ONE_SBF")
            .expect("AMOEBA_GOV_V3_RELEASE_ONE_SBF must select the production release-1 ELF"),
    )
    .unwrap();
    assert_eq!(
        sha256_hex(&deployed),
        "1442c507f6f40f666e4191d767f8a892d61ddf73c82ab80da593c611c7312780"
    );
    assert_eq!(CONTROLLER_RELEASE, 2);
    let mut current = deployed.clone();
    current.resize(ALLOCATED, 0);
    let mut h = Harness::start_with(&current, &deployed, &[&new, &new]).await;
    let first_buffer = h.buffers[0];
    let rival_buffer = h.buffers[1];
    if new.len() > ALLOCATED {
        let program = h.program;
        h.grief(program, (new.len() - ALLOCATED) as u32).await;
    }
    // The payload names the release making the authorization, not its successor.
    let premature = h.controller_action(&new, 2).await;
    assert!(h.create(1, premature).await.is_err());
    let first = h.controller_action(&new, 1).await;
    h.create(1, first).await.expect("release-1 proposal");
    h.seal_and_verify(1, &new).await;
    h.buffer = rival_buffer;
    let rival = h.controller_action(&new, 1).await;
    h.create(2, rival).await.expect("rival release-1 proposal");
    h.seal_and_verify(2, &new).await;
    h.approve(2).await;
    h.buffer = first_buffer;
    h.approve(1).await;
    h.warp(1).await;
    let config_before = h
        .ctx
        .banks_client
        .get_account(config_pda(&h.program).0)
        .await
        .unwrap()
        .unwrap();
    let gate_before = h
        .ctx
        .banks_client
        .get_account(gate_pda(&h.program, &h.target).0)
        .await
        .unwrap()
        .unwrap();
    let first = h.proposal(1).await;
    let rival = h.proposal(2).await;
    let units = h
        .ok(
            h.execute_controller(1, first.digest),
            &[],
            "release 1 installs release 2",
        )
        .await;
    assert!(units < MAX_EXECUTE_CU);
    let installed = h.header(h.programdata).await.2;
    assert_eq!(&installed[..new.len()], new.as_slice());
    assert!(installed[new.len()..].iter().all(|b| *b == 0));
    assert_eq!(
        h.ctx
            .banks_client
            .get_account(config_pda(&h.program).0)
            .await
            .unwrap()
            .unwrap()
            .data,
        config_before.data
    );
    assert_eq!(
        h.ctx
            .banks_client
            .get_account(gate_pda(&h.program, &h.target).0)
            .await
            .unwrap()
            .unwrap()
            .data,
        gate_before.data
    );
    let mut h = h.restart().await;
    h.buffer = rival_buffer;
    let before = h
        .ctx
        .banks_client
        .get_account(h.programdata)
        .await
        .unwrap()
        .unwrap();
    let rival_before = h
        .ctx
        .banks_client
        .get_account(proposal_pda(&h.program, 2).0)
        .await
        .unwrap()
        .unwrap();
    h.fails(
        h.execute_controller(2, rival.digest),
        &[],
        "retired release-1 authorization",
    )
    .await;
    assert_eq!(
        h.ctx
            .banks_client
            .get_account(h.programdata)
            .await
            .unwrap()
            .unwrap()
            .data,
        before.data
    );
    assert_eq!(
        h.ctx
            .banks_client
            .get_account(proposal_pda(&h.program, 2).0)
            .await
            .unwrap()
            .unwrap()
            .data,
        rival_before.data
    );
    for release in [0, 1, 3] {
        let action = h.controller_action(&new, release).await;
        assert!(
            h.create(3, action).await.is_err(),
            "retired or future release {release}"
        );
    }
    let action = h.controller_action(&new, 2).await;
    h.create(3, action)
        .await
        .expect("release-2 proposal accepted");
    println!("AMOEBA_GOV_V3_RELEASE_ONE_TO_TWO old_bytes={} new_bytes={} allocated={} execute_cu={units}", deployed.len(), new.len(), installed.len());
}

/// Anyone may grow ProgramData top-level at any time. Neither one-byte
/// growth before or after creation nor growth past both the old (1.5 MiB) and
/// new (2 MiB) ceilings may stale or brick a proposal, and installation still
/// writes exactly the verified artifact followed by zeros.
#[tokio::test]
#[ignore = "requires AMOEBA_GOV_V3_SBF; explicitly run by the SBF CI job"]
async fn actual_sbf_third_party_growth_cannot_stale_or_brick_proposals() {
    let elf = elf();
    let target_artifact = artifact_of(&elf, elf.len() + 25_000);
    let controller_artifact = artifact_of(&elf, elf.len() + 100);
    let mut h = Harness::start_with(&elf, &elf, &[&target_artifact, &controller_artifact]).await;
    let (target, controller) = (h.target, h.program);

    // Target: growth before creation, including past both ceilings.
    h.grief(target, 1).await;
    let past = 2_097_153 - h.header(h.target_pd).await.1;
    h.grief(target, past as u32).await;
    assert_eq!(h.header(h.target_pd).await.1, 2_097_153);
    let action = h.target_action(&target_artifact).await;
    h.create(1, action)
        .await
        .expect("creation is not bricked past the ceiling");
    h.grief(target, 1).await;
    h.seal_and_verify(1, &target_artifact).await;
    h.grief(target, 1).await;
    h.approve(1).await;
    h.grief(target, 1).await;
    let p1 = h.proposal(1).await;
    h.ok(h.execute_target(1, p1.digest), &[], "install after growth")
        .await;
    let (_, capacity, installed) = h.header(h.target_pd).await;
    assert_eq!(capacity, 2_097_156);
    assert_eq!(
        &installed[..target_artifact.len()],
        target_artifact.as_slice()
    );
    assert!(installed[target_artifact.len()..].iter().all(|b| *b == 0));
    assert_eq!(h.gate().await.epoch, 2);

    // Controller: only the current release may propose; growth by anyone
    // after creation and after approval does not stale the self-upgrade.
    h.buffer = h.buffers[1];
    for release in [0, CONTROLLER_RELEASE + 1] {
        let action = h.controller_action(&controller_artifact, release).await;
        assert!(h.create(2, action).await.is_err(), "release {release}");
    }
    let action = h
        .controller_action(&controller_artifact, CONTROLLER_RELEASE)
        .await;
    h.create(2, action).await.expect("self-upgrade proposal");
    h.grief(controller, 1).await;
    h.seal_and_verify(2, &controller_artifact).await;
    h.approve(2).await;
    h.grief(controller, 99).await;
    h.grief(controller, 1).await;
    let p2 = h.proposal(2).await;
    h.ok(
        h.execute_controller(2, p2.digest),
        &[],
        "self-upgrade after growth",
    )
    .await;
    let installed = h.header(h.programdata).await.2;
    assert_eq!(installed.len(), controller_artifact.len() + 1);
    assert_eq!(
        &installed[..controller_artifact.len()],
        controller_artifact.as_slice()
    );
    assert_eq!(installed[controller_artifact.len()], 0);
    h.warp(1).await;
    let mut timing = [0; ACTION_LEN];
    timing[..24].copy_from_slice(&Timing::default().try_to_vec().unwrap());
    h.create(
        3,
        Action {
            kind: SET_TIMING,
            data: timing,
        },
    )
    .await
    .expect("upgraded controller remains usable");
}
