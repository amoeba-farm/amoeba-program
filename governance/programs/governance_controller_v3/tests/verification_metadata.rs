//! Actual controller SBF -> actual Mainnet Otter Verify SBF, on an isolated test bank.
use borsh::{BorshDeserialize, BorshSerialize};
use governance_controller_v3::{
    instruction::Instruction as GovInstruction, state::*, verification::*,
};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    rent::Rent,
};
use solana_program_test::{ProgramTest, ProgramTestContext};
use solana_sdk::{
    account::{Account, AccountSharedData},
    compute_budget::ComputeBudgetInstruction,
    signature::Signer,
    transaction::Transaction,
};
use solana_sdk_ids::{bpf_loader_upgradeable::ID as LOADER, system_program, sysvar};
use upgrade_controller::state::{GateStatusV1, ProtocolGateV1};

fn account(owner: Pubkey, data: Vec<u8>, executable: bool) -> Account {
    Account {
        lamports: Rent::default().minimum_balance(data.len()).max(1),
        data,
        owner,
        executable,
        rent_epoch: 0,
    }
}
fn ro(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, false)
}
fn unchanged(actual: Account, expected: &Account) {
    // The bank normalizes exempt accounts' rent_epoch; it is not program state.
    assert_eq!(actual.lamports, expected.lamports);
    assert_eq!(actual.data, expected.data);
    assert_eq!(actual.owner, expected.owner);
    assert_eq!(actual.executable, expected.executable);
}
fn rw(key: Pubkey) -> AccountMeta {
    AccountMeta::new(key, false)
}
// Literal previous on-chain recipe, independent of the implementation's
// compatibility constructor. Only a stored record may use these old arguments.
fn previous_spread_params(commit: &str, slot: u64) -> Vec<u8> {
    (
        "0.5.1".to_string(),
        "https://github.com/amoeba-farm/amoeba-program".to_string(),
        commit.to_string(),
        vec![
            "--mount-path", "programs/light_token_minter",
            "--workspace-path", "programs/light_token_minter",
            "--library-name", "light_token_minter", "--base-image",
            "solanafoundation/solana-verifiable-build@sha256:0b4e3716fad9ca4b4aac3e3f977f43aad93a18c22296c0c0f44fc22e644bdd68",
            "--arch", "v1", "--", "--no-default-features", "--features",
            "mainnet-v3,devnet-solo-backfill-2026",
        ].into_iter().map(str::to_string).collect::<Vec<_>>(),
        slot,
    ).try_to_vec().unwrap()
}
// Independent literal release-5 Futures metadata, not the compatibility constructor.
fn previous_futures_params(commit: &str, slot: u64) -> Vec<u8> {
    (
        "0.5.2".to_string(),
        "https://github.com/amoeba-farm/amoeba-program".to_string(),
        commit.to_string(),
        vec!["--workspace-path", "futures", "--library-name", "ameba_futures",
            "--base-image", "us-central1-docker.pkg.dev/amoeba-hm0q2k/solana-verifiable-builds/gate-final@sha256:7ad1e567f19831e658f5b892a7269d23d7f1826438e025b5d9d2e385240a4d67",
            "--arch", "v1", "--cargo-build-sbf-args", "--tools-version v1.53"]
            .into_iter().map(str::to_string).collect::<Vec<_>>(),
        slot,
    ).try_to_vec().unwrap()
}
fn install(test: &mut ProgramTest, program: Pubkey, authority: Pubkey, bytes: &[u8]) {
    let pd = Pubkey::find_program_address(&[program.as_ref()], &LOADER).0;
    let mut p = 2u32.to_le_bytes().to_vec();
    p.extend_from_slice(pd.as_ref());
    let mut d = 3u32.to_le_bytes().to_vec();
    d.extend_from_slice(&0u64.to_le_bytes());
    d.push(1);
    d.extend_from_slice(authority.as_ref());
    d.extend_from_slice(bytes);
    test.add_genesis_account(program, account(LOADER, p, true));
    test.add_genesis_account(pd, account(LOADER, d, false));
}
async fn send(ctx: &mut ProgramTestContext, instructions: &[Instruction]) -> bool {
    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        instructions,
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    let result = ctx.banks_client.process_transaction(tx).await;
    if let Err(e) = &result {
        eprintln!("expected-or-investigate: {e:?}");
    }
    result.is_ok()
}
async fn run(target_override: Option<Pubkey>) {
    let elf =
        std::fs::read(std::env::var("AMOEBA_GOV_V3_SBF").expect("exact controller ELF required"))
            .unwrap();
    let verifier = std::fs::read(
        std::env::var("AMOEBA_OTTER_VERIFY_SBF").expect("actual Otter Verify ELF required"),
    )
    .unwrap();
    let controller = Pubkey::new_unique();
    let own = target_override.is_none();
    let target = target_override.unwrap_or(controller);
    let cfg_key = config_pda(&controller).0;
    let auth = if own {
        authority_pda(&controller).0
    } else {
        target_authority_pda(&controller, &target).0
    };
    let pd = Pubkey::find_program_address(&[target.as_ref()], &LOADER).0;
    let config = Config {
        discriminator: *b"AG3CFG01",
        version: 1,
        bump: config_pda(&controller).1,
        initialized: true,
        controller,
        programdata: Pubkey::find_program_address(&[controller.as_ref()], &LOADER).0,
        authority: authority_pda(&controller).0,
        treasury: Pubkey::new_unique(),
        seats: std::array::from_fn(|_| Pubkey::new_unique()),
        council_epoch: 1,
        timing_version: 1,
        timing: Timing::default(),
        next_id: 3,
        reserved: [0; 37],
    };
    let policy = VerificationPolicy {
        target,
        deployed_slot: 0,
        commit: [0xab; 20],
    };
    let mut action = Action {
        kind: PUBLISH_VERIFICATION,
        data: [0; ACTION_LEN],
    };
    action.data[..60].copy_from_slice(&policy.try_to_vec().unwrap());
    let (review_end, not_before, expires) = config.timing.boundaries(100).unwrap();
    let mut proposal = Proposal {
        discriminator: *b"AG3PRP01",
        version: 1,
        bump: proposal_pda(&controller, 1).1,
        initialized: true,
        config: cfg_key,
        id: 1,
        council_epoch: 1,
        timing_version: 1,
        timing: config.timing,
        created: 100,
        review_end,
        not_before,
        expires,
        action,
        digest: [0; 32],
        approvals: 7,
        approval_count: 3,
        cancellations: 0,
        cancellation_count: 0,
        state: PENDING,
        verified_chunks: [0; 12],
        verified_count: 0,
        extension_slot: 0,
        extended_capacity: 0,
        executed_slot: 0,
        verified_chunks_high: [0; 4],
        reserved: [0; 3],
    };
    proposal.digest = proposal.compute_digest(&controller).unwrap();
    let pkey = proposal_pda(&controller, 1).0;
    let (metadata, bump) = metadata_pda(&auth, &target);
    let mut test = ProgramTest::default();
    test.prefer_bpf(true);
    install(&mut test, controller, config.authority, &elf);
    if !own {
        install(&mut test, target, auth, &elf);
    }
    install(&mut test, VERIFY_PROGRAM, Pubkey::new_unique(), &verifier);
    test.add_account(
        cfg_key,
        account(controller, config.try_to_vec().unwrap(), false),
    );
    test.add_account(
        pkey,
        account(controller, proposal.try_to_vec().unwrap(), false),
    );
    let authority_before = account(system_program::ID, vec![], false);
    test.add_account(auth, authority_before.clone());
    let mut prefund = account(system_program::ID, vec![], false);
    prefund.lamports = 123_456;
    test.add_account(metadata, prefund.clone());
    let (gate_key, gate_bump) = gate_pda(&controller, &target);
    let gate = ProtocolGateV1 {
        discriminator: *b"AGVGAT01",
        version: 1,
        bump: gate_bump,
        initialized: true,
        status: GateStatusV1::EmergencyFrozen,
        controller_config: cfg_key,
        target_program: target,
        target_programdata: pd,
        epoch: 1,
        active_proposal: Pubkey::default(),
        freeze_slot: 1,
        freeze_reason_code: 1,
        last_completed_proposal: Pubkey::default(),
        reserved: [0; 2],
    };
    if !own {
        test.add_account(
            gate_key,
            account(controller, gate.try_to_vec().unwrap(), false),
        );
    }
    let mut ctx = test.start_with_context().await;
    ctx.warp_to_slot(100).unwrap();
    let instruction = |payer: Pubkey, p: &Proposal| {
        let mut keys = vec![
            AccountMeta::new(payer, true),
            ro(cfg_key),
            rw(pkey),
            ro(target),
            ro(pd),
            rw(auth),
            rw(metadata),
            ro(VERIFY_PROGRAM),
            ro(system_program::ID),
            ro(sysvar::instructions::ID),
        ];
        if !own {
            keys.push(ro(gate_key));
        }
        Instruction {
            program_id: controller,
            accounts: keys,
            data: GovInstruction::PublishVerification { digest: p.digest }
                .try_to_vec()
                .unwrap(),
        }
    };
    let ix = instruction(ctx.payer.pubkey(), &proposal);
    assert!(
        !send(&mut ctx, std::slice::from_ref(&ix)).await,
        "early execution"
    );
    ctx.warp_to_slot(4600).unwrap();
    let reset = |ctx: &mut ProgramTestContext, p: &Proposal| {
        ctx.set_account(
            &pkey,
            &AccountSharedData::from(account(controller, p.try_to_vec().unwrap(), false)),
        )
    };
    for case in 0..5 {
        let mut bad = proposal.clone();
        match case {
            0 => {
                bad.approvals = 3;
                bad.approval_count = 2;
            }
            1 => {
                bad.council_epoch = 2;
            }
            2 => {
                bad.action.data[32] = 1;
            }
            3 => {
                bad.state = CANCELLED;
                bad.cancellations = 7;
                bad.cancellation_count = 3;
            }
            _ => {
                bad.action.data[..32].copy_from_slice(Pubkey::new_unique().as_ref());
            }
        }
        bad.digest = bad.compute_digest(&controller).unwrap();
        reset(&mut ctx, &bad);
        let bad_ix = instruction(ctx.payer.pubkey(), &bad);
        assert!(!send(&mut ctx, &[bad_ix]).await, "case {case}");
        assert_eq!(
            ctx.banks_client
                .get_account(metadata)
                .await
                .unwrap()
                .unwrap(),
            prefund
        );
        unchanged(
            ctx.banks_client.get_account(auth).await.unwrap().unwrap(),
            &authority_before,
        );
    }
    reset(&mut ctx, &proposal);
    for position in [4, 5, 6, 7] {
        let mut wrong = ix.clone();
        wrong.accounts[position].pubkey = Pubkey::new_unique();
        assert!(
            !send(&mut ctx, &[wrong]).await,
            "substituted account {position}"
        );
    }
    let mut duplicate = ix.clone();
    duplicate.accounts[6] = duplicate.accounts[5].clone();
    assert!(!send(&mut ctx, &[duplicate]).await);
    let sibling = solana_system_interface::instruction::transfer(&ctx.payer.pubkey(), &auth, 1);
    assert!(!send(&mut ctx, &[sibling, ix.clone()]).await);
    // Foreign metadata ownership is rejected without mutation.
    ctx.set_account(
        &metadata,
        &AccountSharedData::from(account(Pubkey::new_unique(), vec![0; 1], false)),
    );
    assert!(!send(&mut ctx, std::slice::from_ref(&ix)).await);
    ctx.set_account(&metadata, &AccountSharedData::from(prefund));
    // Refresh the bank/blockhash so an earlier failed signature is not replayed.
    ctx.warp_to_slot(4700).unwrap();
    assert!(send(&mut ctx, std::slice::from_ref(&ix)).await);
    let result = ctx
        .banks_client
        .get_account(metadata)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.owner, VERIFY_PROGRAM);
    assert_eq!(
        result.data,
        record(&target, &auth, &policy.params(&controller).unwrap(), bump)
    );
    unchanged(
        ctx.banks_client.get_account(auth).await.unwrap().unwrap(),
        &authority_before,
    );
    let finished = ctx.banks_client.get_account(pkey).await.unwrap().unwrap();
    assert_eq!(
        Proposal::try_from_slice(&finished.data).unwrap().state,
        EXECUTED
    );
    // A different approved digest exercises Update; it cannot close or redirect the PDA.
    let mut changed = proposal.clone();
    changed.action.data[40..60].fill(0xcd);
    changed.digest = changed.compute_digest(&controller).unwrap();
    reset(&mut ctx, &changed);
    let update = instruction(ctx.payer.pubkey(), &changed);
    assert!(send(&mut ctx, &[update]).await);
    let updated = ctx
        .banks_client
        .get_account(metadata)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.lamports, result.lamports);
    let new_policy = changed.action.verification_policy().unwrap();
    assert_eq!(
        updated.data,
        record(
            &target,
            &auth,
            &new_policy.params(&controller).unwrap(),
            bump
        )
    );
    unchanged(
        ctx.banks_client.get_account(auth).await.unwrap().unwrap(),
        &authority_before,
    );
    if target == MAINNET_FUTURES {
        // Real Mainnet shape: exact old minimum rent, zero authority, +2-byte growth.
        let minimum = account(
            VERIFY_PROGRAM,
            record(
                &target,
                &auth,
                &previous_futures_params(&"cd".repeat(20), 19),
                bump,
            ),
            false,
        );
        let mut zero_authority = authority_before.clone();
        zero_authority.lamports = 0;
        let mut payer = ctx
            .banks_client
            .get_account(ctx.payer.pubkey())
            .await
            .unwrap()
            .unwrap();
        payer.lamports += updated.lamports - minimum.lamports + authority_before.lamports;
        ctx.set_account(&ctx.payer.pubkey(), &AccountSharedData::from(payer));
        ctx.set_account(&metadata, &AccountSharedData::from(minimum.clone()));
        ctx.set_account(&auth, &AccountSharedData::from(zero_authority));
        let mut minimum_proposal = proposal.clone();
        minimum_proposal.action.data[40..60].fill(0xa1);
        minimum_proposal.digest = minimum_proposal.compute_digest(&controller).unwrap();
        reset(&mut ctx, &minimum_proposal);
        let expected = record(
            &target,
            &auth,
            &minimum_proposal
                .action
                .verification_policy()
                .unwrap()
                .params(&controller)
                .unwrap(),
            bump,
        );
        assert_eq!(expected.len(), minimum.data.len() + 2);
        let topup = Rent::default().minimum_balance(expected.len()) - minimum.lamports;
        let payer_before = ctx
            .banks_client
            .get_balance(ctx.payer.pubkey())
            .await
            .unwrap();
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[instruction(ctx.payer.pubkey(), &minimum_proposal)],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        let fee = ctx
            .banks_client
            .get_fee_for_message(tx.message.clone())
            .await
            .unwrap()
            .unwrap();
        ctx.banks_client.process_transaction(tx).await.unwrap();
        let migrated = ctx
            .banks_client
            .get_account(metadata)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(migrated.data, expected);
        assert_eq!(migrated.lamports, minimum.lamports + topup);
        assert_eq!(
            ctx.banks_client
                .get_balance(ctx.payer.pubkey())
                .await
                .unwrap(),
            payer_before - topup - fee
        );
        assert_eq!(
            ctx.banks_client
                .get_account(auth)
                .await
                .unwrap()
                .map_or(0, |a| a.lamports),
            0
        );
        let finalized = ctx.banks_client.get_account(pkey).await.unwrap().unwrap();
        assert_eq!(
            Proposal::try_from_slice(&finalized.data).unwrap().state,
            EXECUTED
        );
        println!("AMOEBA_FUTURES_MINIMUM_RENT_MIGRATION old_bytes={} new_bytes={} topup={} fee={} authority_lamports=0", minimum.data.len(), migrated.data.len(), topup, fee);
        // Restore the original authority fixture using a real transfer; later donation cases stay unchanged.
        let payer_key = ctx.payer.pubkey();
        assert!(
            send(
                &mut ctx,
                &[solana_system_interface::instruction::transfer(
                    &payer_key,
                    &auth,
                    authority_before.lamports
                )]
            )
            .await
        );
    }
    if target == DEVNET_SPREAD || target == MAINNET_FUTURES {
        // Transition an authentic old record using actual Otter SBF. Include
        // donations to both accounts; Anchor refunds all excess on shrink.
        let legacy_bytes = record(
            &target,
            &auth,
            &if target == MAINNET_FUTURES {
                previous_futures_params(&"cd".repeat(20), 19)
            } else {
                previous_spread_params(&"cd".repeat(20), 19)
            },
            bump,
        );
        let mut legacy = account(VERIFY_PROGRAM, legacy_bytes, false);
        legacy.lamports += 765_432;
        let mut donated_authority = authority_before.clone();
        donated_authority.lamports += 234_567;
        // Fund donations through transfers so the bank's capitalization stays
        // valid when the fixture record is replaced between root/warp steps.
        let donor = ctx.payer.pubkey();
        assert!(
            send(
                &mut ctx,
                &[
                    solana_system_interface::instruction::transfer(&donor, &auth, 234_567),
                    solana_system_interface::instruction::transfer(
                        &donor,
                        &metadata,
                        legacy.lamports - updated.lamports
                    ),
                ]
            )
            .await
        );
        let mut transition = proposal.clone();
        transition.action.data[40..60].fill(0xef);
        transition.digest = transition.compute_digest(&controller).unwrap();
        let transition_ix = instruction(ctx.payer.pubkey(), &transition);
        let transition_account = account(controller, transition.try_to_vec().unwrap(), false);
        // Byte offsets are independently derived from the literal Borsh recipe.
        let commit_start = 72 + 4 + 5 + 4 + 45 + 4;
        assert_eq!(
            &legacy.data[commit_start..commit_start + 40],
            "cd".repeat(20).as_bytes()
        );
        for case in 0..9 {
            let mut bad = legacy.clone();
            match case {
                0 => bad.owner = Pubkey::new_unique(),
                1 => bad.data[8] ^= 1,                   // target
                2 => bad.data[40] ^= 1,                  // authority
                3 => *bad.data.last_mut().unwrap() ^= 1, // PDA bump
                4 => bad.data[commit_start] = b'G',
                5 => bad.data[commit_start..commit_start + 40].fill(b'0'),
                6 => bad.data[commit_start + 44] ^= 1, // first argument length
                7 => bad.data.push(0),
                _ => {
                    bad.data.pop();
                }
            }
            ctx.set_account(&metadata, &AccountSharedData::from(bad.clone()));
            reset(&mut ctx, &transition);
            ctx.warp_to_slot(4800 + case * 100).unwrap();
            assert!(
                !send(&mut ctx, std::slice::from_ref(&transition_ix)).await,
                "legacy mutation {case}"
            );
            unchanged(
                ctx.banks_client
                    .get_account(metadata)
                    .await
                    .unwrap()
                    .unwrap(),
                &bad,
            );
            unchanged(
                ctx.banks_client.get_account(auth).await.unwrap().unwrap(),
                &donated_authority,
            );
            unchanged(
                ctx.banks_client.get_account(pkey).await.unwrap().unwrap(),
                &transition_account,
            );
        }
        ctx.set_account(&metadata, &AccountSharedData::from(legacy.clone()));
        reset(&mut ctx, &transition);
        ctx.warp_to_slot(5800).unwrap();
        // Exhaust compute inside the actual verifier CPI and confirm that
        // partial reallocation/refund work cannot survive a failed transaction.
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let interrupted = Transaction::new_signed_with_payer(
            &[
                ComputeBudgetInstruction::set_compute_unit_limit(95_000),
                transition_ix.clone(),
            ],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        let failure = ctx
            .banks_client
            .process_transaction_with_metadata(interrupted)
            .await
            .unwrap();
        assert!(failure.result.is_err(), "limited verifier CPI must fail");
        let logs = failure.metadata.unwrap().log_messages;
        assert!(
            logs.iter()
                .any(|line| line == &format!("Program {VERIFY_PROGRAM} invoke [2]")),
            "failure must reach the actual verifier: {logs:?}"
        );
        assert!(
            logs.iter()
                .any(|line| line.contains("Computational budget exceeded")
                    || line.contains("exceeded CUs meter at BPF instruction")),
            "expected compute exhaustion: {logs:?}"
        );
        unchanged(
            ctx.banks_client
                .get_account(metadata)
                .await
                .unwrap()
                .unwrap(),
            &legacy,
        );
        unchanged(
            ctx.banks_client.get_account(auth).await.unwrap().unwrap(),
            &donated_authority,
        );
        unchanged(
            ctx.banks_client.get_account(pkey).await.unwrap().unwrap(),
            &transition_account,
        );
        ctx.warp_to_slot(5900).unwrap();
        assert!(send(&mut ctx, std::slice::from_ref(&transition_ix)).await);
        let migrated = ctx
            .banks_client
            .get_account(metadata)
            .await
            .unwrap()
            .unwrap();
        let expected = record(
            &target,
            &auth,
            &transition
                .action
                .verification_policy()
                .unwrap()
                .params(&controller)
                .unwrap(),
            bump,
        );
        if target == DEVNET_SPREAD {
            assert!(expected.len() < legacy.data.len());
        } else {
            assert!(expected.len() > legacy.data.len());
        }
        assert_eq!(migrated.data, expected);
        assert_eq!(migrated.lamports, legacy.lamports);
        unchanged(
            ctx.banks_client.get_account(auth).await.unwrap().unwrap(),
            &donated_authority,
        );
        assert_eq!(
            Proposal::try_from_slice(
                &ctx.banks_client
                    .get_account(pkey)
                    .await
                    .unwrap()
                    .unwrap()
                    .data
            )
            .unwrap()
            .state,
            EXECUTED
        );
        println!("AMOEBA_VERIFICATION_RECIPE_TRANSITION old_bytes={} new_bytes={} metadata_lamports={} authority_lamports={}", legacy.data.len(), migrated.data.len(), migrated.lamports, donated_authority.lamports);
        assert_eq!(
            ctx.banks_client
                .get_account(gate_key)
                .await
                .unwrap()
                .unwrap()
                .data,
            gate.try_to_vec().unwrap()
        );
    }
}
#[tokio::test]
#[ignore = "requires this worktree controller and finalized Mainnet Otter Verify SBF artifacts"]
async fn actual_sbf_controller_and_frozen_target_metadata() {
    run(None).await;
    run(Some(DEVNET_SPREAD)).await;
    run(Some(MAINNET_ORACLE)).await;
    run(Some(MAINNET_FUTURES)).await;
}
