//! One focused, real-SBF rehearsal: bootstrap -> verify -> late approvals -> self-upgrade.
use borsh::{BorshDeserialize, BorshSerialize};
use governance_controller_v3::{instruction::Instruction as GovInstruction, state::*};
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
    artifact_merkle::{
        artifact_merkle_proof, artifact_merkle_root, RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as CHUNK,
    },
    release1_loader_accounts::parse_upgradeable_programdata,
};

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
fn rw(key: Pubkey) -> AccountMeta {
    AccountMeta::new(key, false)
}
fn sig(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, true)
}
fn ix(program: Pubkey, accounts: Vec<AccountMeta>, value: GovInstruction) -> Instruction {
    Instruction {
        program_id: program,
        accounts,
        data: value.try_to_vec().unwrap(),
    }
}
async fn send(
    ctx: &mut ProgramTestContext,
    instruction: Instruction,
    signers: &[&Keypair],
) -> bool {
    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let mut keys = vec![&ctx.payer];
    keys.extend_from_slice(signers);
    let transaction = Transaction::new_signed_with_payer(
        &[
            ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
            instruction,
        ],
        Some(&ctx.payer.pubkey()),
        &keys,
        blockhash,
    );
    let result = ctx.banks_client.process_transaction(transaction).await;
    if let Err(error) = &result {
        eprintln!("transaction: {error:?}");
    }
    result.is_ok()
}

#[tokio::test]
#[ignore = "requires AMOEBA_GOV_V3_SBF; explicitly run by the SBF CI job"]
async fn actual_sbf_target_adoption_extension_and_upgrade_remain_frozen_after_450_slots() {
    let file = std::env::var("AMOEBA_GOV_V3_SBF")
        .expect("AMOEBA_GOV_V3_SBF must select this worktree's final ELF");
    let original_artifact = std::fs::read(file).unwrap();
    let mut artifact = original_artifact.clone();
    artifact.extend_from_slice(&[0; 16]);
    assert!(artifact.starts_with(b"\x7fELF"));
    let program = Pubkey::new_unique();
    let programdata = Pubkey::find_program_address(&[program.as_ref()], &LOADER).0;
    let target = Pubkey::new_unique();
    let target_pd = Pubkey::find_program_address(&[target.as_ref()], &LOADER).0;
    let target_authority = target_authority_pda(&program, &target).0;
    let gate_key = gate_pda(&program, &target).0;
    let deployer = Keypair::new();
    let uploader = Keypair::new();
    let seats: [Keypair; 5] = std::array::from_fn(|_| Keypair::new());
    let treasury = Pubkey::new_unique();
    let buffer = Pubkey::new_unique();
    let config_key = config_pda(&program).0;
    let authority = authority_pda(&program).0;
    let proposal_key = proposal_pda(&program, 1).0;
    let buffer_authority = buffer_authority_pda(&program, &proposal_key).0;
    let mut program_bytes = 2u32.to_le_bytes().to_vec();
    program_bytes.extend_from_slice(programdata.as_ref());
    let mut pd = 3u32.to_le_bytes().to_vec();
    pd.extend_from_slice(&0u64.to_le_bytes());
    pd.push(1);
    pd.extend_from_slice(deployer.pubkey().as_ref());
    pd.extend_from_slice(&original_artifact);
    let mut buffer_bytes = 1u32.to_le_bytes().to_vec();
    buffer_bytes.push(1);
    buffer_bytes.extend_from_slice(uploader.pubkey().as_ref());
    buffer_bytes.extend_from_slice(&artifact);
    let mut test = ProgramTest::default();
    test.prefer_bpf(true);
    test.deactivate_feature(solana_program::pubkey!(
        "2oMRZEDWT2tqtYMofhmmfQ8SsjqUFzT6sYXppQDavxwz"
    ));
    test.add_genesis_account(program, account(LOADER, program_bytes, true));
    test.add_genesis_account(programdata, account(LOADER, pd.clone(), false));
    let mut target_bytes = 2u32.to_le_bytes().to_vec();
    target_bytes.extend_from_slice(target_pd.as_ref());
    test.add_genesis_account(target, account(LOADER, target_bytes, true));
    test.add_genesis_account(target_pd, account(LOADER, pd, false));
    test.add_account(buffer, account(LOADER, buffer_bytes, false));
    for key in [
        deployer.pubkey(),
        uploader.pubkey(),
        treasury,
        authority,
        target_authority,
        buffer_authority,
    ]
    .into_iter()
    .chain(seats.iter().map(Signer::pubkey))
    {
        test.add_account(key, account(system_program::ID, vec![], false));
    }
    let mut ctx = test.start_with_context().await;
    ctx.warp_to_slot(100).unwrap();
    let initialize = ix(
        program,
        vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            sig(deployer.pubkey()),
            ro(program),
            rw(programdata),
            rw(config_key),
            ro(authority),
            ro(LOADER),
            ro(system_program::ID),
            sig(seats[0].pubkey()),
            sig(seats[1].pubkey()),
            sig(seats[2].pubkey()),
        ],
        GovInstruction::Initialize {
            seats: std::array::from_fn(|i| seats[i].pubkey()),
            treasury,
        },
    );
    assert!(
        send(
            &mut ctx,
            initialize,
            &[&deployer, &seats[0], &seats[1], &seats[2]]
        )
        .await,
        "bootstrap"
    );
    let register = ix(
        program,
        vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            sig(deployer.pubkey()),
            ro(config_key),
            ro(target),
            rw(target_pd),
            rw(gate_key),
            ro(target_authority),
            ro(LOADER),
            ro(system_program::ID),
            sig(seats[0].pubkey()),
            sig(seats[1].pubkey()),
            sig(seats[2].pubkey()),
            ro(sysvar::instructions::ID),
        ],
        GovInstruction::RegisterTarget,
    );
    let mut unauthorized = register.clone();
    unauthorized.accounts[11] = sig(uploader.pubkey());
    assert!(
        !send(
            &mut ctx,
            unauthorized,
            &[&deployer, &seats[0], &seats[1], &uploader]
        )
        .await
    );
    assert!(ctx
        .banks_client
        .get_account(gate_key)
        .await
        .unwrap()
        .is_none());
    assert!(
        send(
            &mut ctx,
            register.clone(),
            &[&deployer, &seats[0], &seats[1], &seats[2]]
        )
        .await
    );
    ctx.warp_to_slot(101).unwrap();
    assert!(
        !send(
            &mut ctx,
            register,
            &[&deployer, &seats[0], &seats[1], &seats[2]]
        )
        .await,
        "no reinitialization"
    );
    let extend = solana_loader_v3_interface::instruction::extend_program(
        &target,
        Some(&ctx.payer.pubkey()),
        16,
    );
    assert!(
        send(&mut ctx, extend, &[]).await,
        "top-level extension before proposal"
    );
    ctx.warp_to_slot(102).unwrap();
    let gate_account = ctx
        .banks_client
        .get_account(gate_key)
        .await
        .unwrap()
        .unwrap();
    let gate =
        upgrade_controller::state::ProtocolGateV1::try_from_slice(&gate_account.data).unwrap();
    assert_eq!(gate.epoch, 1);
    assert_eq!(
        gate.status,
        upgrade_controller::state::GateStatusV1::EmergencyFrozen
    );
    assert_eq!(gate.controller_config, config_key);
    let pd = ctx
        .banks_client
        .get_account(target_pd)
        .await
        .unwrap()
        .unwrap();
    let before = parse_upgradeable_programdata(&pd.data).unwrap();
    assert_eq!(before.upgrade_authority, Some(target_authority));
    let upgrade = TargetUpgrade {
        target,
        gate_epoch: 1,
        buffer,
        artifact_length: artifact.len() as u64,
        merkle_root: artifact_merkle_root(&artifact, CHUNK).unwrap(),
        deployed_slot: before.deployed_slot,
        capacity: before.capacity as u64,
        source_commitment: [1; 32],
        build_commitment: [2; 32],
    };
    let mut data = [0; 192];
    data.copy_from_slice(&upgrade.try_to_vec().unwrap());
    let create = ix(
        program,
        vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            sig(seats[0].pubkey()),
            rw(config_key),
            rw(proposal_key),
            ro(system_program::ID),
        ],
        GovInstruction::Create {
            expected_id: 1,
            action: Action {
                kind: UPGRADE_TARGET,
                data,
            },
        },
    );
    assert!(send(&mut ctx, create, &[&seats[0]]).await, "create");
    let proposal = ctx
        .banks_client
        .get_account(proposal_key)
        .await
        .unwrap()
        .unwrap();
    let proposal = Proposal::try_from_slice(&proposal.data).unwrap();
    assert_eq!(proposal.review_end - proposal.created, WEEK_SLOTS);
    let digest = proposal.digest;
    let seal = ix(
        program,
        vec![
            ro(config_key),
            ro(proposal_key),
            rw(buffer),
            sig(uploader.pubkey()),
            ro(buffer_authority),
            ro(LOADER),
        ],
        GovInstruction::SealBuffer { digest },
    );
    assert!(send(&mut ctx, seal, &[&uploader]).await, "seal");
    let premature = ix(
        program,
        vec![ro(config_key), rw(proposal_key), sig(seats[0].pubkey())],
        GovInstruction::Approve { digest },
    );
    assert!(
        !send(&mut ctx, premature, &[&seats[0]]).await,
        "approval must require verification"
    );
    for index in 0..artifact.len().div_ceil(CHUNK as usize) {
        let branch = artifact_merkle_proof(&artifact, CHUNK, index as u32).unwrap();
        let mut proof = [[0; 32]; 7];
        proof[..branch.len()].copy_from_slice(&branch);
        let verify = ix(
            program,
            vec![ro(config_key), rw(proposal_key), ro(buffer)],
            GovInstruction::VerifyChunk {
                digest,
                index: index as u32,
                proof_len: branch.len() as u8,
                proof,
            },
        );
        assert!(send(&mut ctx, verify, &[]).await, "verify chunk {index}");
    }
    ctx.warp_to_slot(proposal.not_before + 1).unwrap();
    for seat in &seats[..2] {
        let approve = ix(
            program,
            vec![ro(config_key), rw(proposal_key), sig(seat.pubkey())],
            GovInstruction::Approve { digest },
        );
        assert!(send(&mut ctx, approve, &[seat]).await, "late approval");
    }
    let execute = ix(
        program,
        vec![
            ro(config_key),
            rw(proposal_key),
            rw(target),
            rw(target_pd),
            rw(buffer),
            rw(treasury),
            ro(target_authority),
            ro(buffer_authority),
            ro(LOADER),
            ro(sysvar::rent::ID),
            ro(sysvar::clock::ID),
            ro(sysvar::instructions::ID),
            rw(gate_key),
        ],
        GovInstruction::ExecuteTargetUpgrade { digest },
    );
    assert!(
        !send(&mut ctx, execute.clone(), &[]).await,
        "two seats cannot upgrade"
    );
    let approve = ix(
        program,
        vec![ro(config_key), rw(proposal_key), sig(seats[2].pubkey())],
        GovInstruction::Approve { digest },
    );
    assert!(send(&mut ctx, approve, &[&seats[2]]).await);
    ctx.warp_to_slot(proposal.not_before + 2).unwrap();
    assert!(
        send(&mut ctx, execute, &[]).await,
        "three seats execute real self-upgrade"
    );
    let pd = ctx
        .banks_client
        .get_account(target_pd)
        .await
        .unwrap()
        .unwrap();
    let after = parse_upgradeable_programdata(&pd.data).unwrap();
    assert_eq!(after.upgrade_authority, Some(target_authority));
    let gate_account = ctx
        .banks_client
        .get_account(gate_key)
        .await
        .unwrap()
        .unwrap();
    let gate =
        upgrade_controller::state::ProtocolGateV1::try_from_slice(&gate_account.data).unwrap();
    assert_eq!(gate.epoch, 2);
    assert_eq!(
        gate.status,
        upgrade_controller::state::GateStatusV1::EmergencyFrozen
    );
    assert_eq!(gate.last_completed_proposal, proposal_key);
    let own = ctx
        .banks_client
        .get_account(programdata)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&own.data[45..], original_artifact);
    assert_eq!(
        parse_upgradeable_programdata(&own.data)
            .unwrap()
            .upgrade_authority,
        Some(authority)
    );
    assert_eq!(after.deployed_slot, proposal.not_before + 2);
    assert_eq!(&pd.data[45..], artifact);
    let stored = ctx
        .banks_client
        .get_account(proposal_key)
        .await
        .unwrap()
        .unwrap();
    let stored = Proposal::try_from_slice(&stored.data).unwrap();
    assert_eq!(stored.state, EXECUTED);
    ctx.warp_to_slot(proposal.not_before + 3).unwrap();
    // The upgraded controller still executes and can open its next proposal.
    let next_key = proposal_pda(&program, 2).0;
    let mut timing_data = [0; 192];
    timing_data[..24].copy_from_slice(&Timing::default().try_to_vec().unwrap());
    let next = ix(
        program,
        vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            sig(seats[0].pubkey()),
            rw(config_key),
            rw(next_key),
            ro(system_program::ID),
        ],
        GovInstruction::Create {
            expected_id: 2,
            action: Action {
                kind: SET_TIMING,
                data: timing_data,
            },
        },
    );
    assert!(
        send(&mut ctx, next, &[&seats[0]]).await,
        "upgraded program remains usable"
    );
}
