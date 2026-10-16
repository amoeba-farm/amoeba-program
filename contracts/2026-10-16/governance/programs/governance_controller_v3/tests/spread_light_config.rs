//! Real controller SBF signs the sole approved, create-once Spread Light CPI.
use borsh::{BorshDeserialize, BorshSerialize};
use governance_controller_v3::{instruction::Instruction as GovInstruction, state::*};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey,
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
use upgrade_controller::state::{GateStatusV1, ProtocolGateV1};

fn untrusted_proxy(
    _program: &Pubkey,
    accounts: &[solana_program::account_info::AccountInfo],
    data: &[u8],
) -> solana_program::entrypoint::ProgramResult {
    solana_program::program::invoke(
        &Instruction {
            program_id: DEVNET_SPREAD,
            accounts: accounts[1..]
                .iter()
                .map(|a| AccountMeta {
                    pubkey: *a.key,
                    is_signer: a.is_signer,
                    is_writable: a.is_writable,
                })
                .collect(),
            data: data.to_vec(),
        },
        accounts,
    )
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
fn ro(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, false)
}
fn rw(key: Pubkey) -> AccountMeta {
    AccountMeta::new(key, false)
}
fn sig(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, true)
}
fn ix(program_id: Pubkey, accounts: Vec<AccountMeta>, value: GovInstruction) -> Instruction {
    Instruction {
        program_id,
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
#[ignore = "requires the exact controller/Spread SBF artifacts; explicitly run by the SBF CI job"]
async fn actual_sbf_council_initializes_exact_spread_light_config_once() {
    let controller = pubkey!("8fhNi6QHU5TYNhoPDM4vs89ZBztnpxp3LnBXRgkBVKtx");
    let controller_pd = Pubkey::find_program_address(&[controller.as_ref()], &LOADER).0;
    let target_pd = Pubkey::find_program_address(&[DEVNET_SPREAD.as_ref()], &LOADER).0;
    let config_key = config_pda(&controller).0;
    let proposal_key = proposal_pda(&controller, 1).0;
    let authority = target_authority_pda(&controller, &DEVNET_SPREAD).0;
    let (gate_key, gate_bump) = gate_pda(&controller, &DEVNET_SPREAD);
    let (light_key, light_bump) =
        Pubkey::find_program_address(&[b"compressible_config", &[0, 0]], &DEVNET_SPREAD);
    let (sponsor, sponsor_bump) = Pubkey::find_program_address(&[b"rent_sponsor"], &DEVNET_SPREAD);
    let seats: [Keypair; 5] = std::array::from_fn(|_| Keypair::new());
    let config = Config {
        discriminator: *b"AG3CFG01",
        version: 1,
        bump: config_pda(&controller).1,
        initialized: true,
        controller,
        programdata: controller_pd,
        authority: authority_pda(&controller).0,
        treasury: Pubkey::new_unique(),
        seats: std::array::from_fn(|i| seats[i].pubkey()),
        council_epoch: 1,
        timing_version: 2,
        timing: Timing {
            review_slots: 4_000_000,
            delay_slots: 4_500,
            expiry_slots: 7_000_000,
        },
        next_id: 1,
        reserved: [0; 37],
    };
    config.validate(&controller, &config_key).unwrap();
    let gate = ProtocolGateV1 {
        discriminator: *b"AGVGAT01",
        version: 1,
        bump: gate_bump,
        initialized: true,
        status: GateStatusV1::Active,
        controller_config: config_key,
        target_program: DEVNET_SPREAD,
        target_programdata: target_pd,
        epoch: 3,
        active_proposal: Pubkey::default(),
        freeze_slot: 0,
        freeze_reason_code: 0,
        last_completed_proposal: Pubkey::default(),
        reserved: [0; 2],
    };
    let mut test = ProgramTest::default();
    test.prefer_bpf(true);
    let proxy = Pubkey::new_unique();
    test.prefer_bpf(false);
    test.add_program(
        "untrusted_proxy",
        proxy,
        solana_program_test::processor!(untrusted_proxy),
    );
    test.prefer_bpf(true);
    for (program, pd, authority, file) in [
        (
            controller,
            controller_pd,
            config.authority,
            "AMOEBA_GOV_V3_SBF",
        ),
        (
            DEVNET_SPREAD,
            target_pd,
            authority,
            "AMOEBA_FRESH_SPREAD_SBF",
        ),
    ] {
        let artifact =
            std::fs::read(std::env::var(file).expect("exact SBF artifact required")).unwrap();
        let mut pbytes = 2u32.to_le_bytes().to_vec();
        pbytes.extend_from_slice(pd.as_ref());
        let mut dbytes = 3u32.to_le_bytes().to_vec();
        dbytes.extend_from_slice(&0u64.to_le_bytes());
        dbytes.push(1);
        dbytes.extend_from_slice(authority.as_ref());
        dbytes.extend_from_slice(&artifact);
        test.add_genesis_account(program, account(LOADER, pbytes, true));
        test.add_genesis_account(pd, account(LOADER, dbytes, false));
    }
    test.add_account(
        config_key,
        account(controller, config.try_to_vec().unwrap(), false),
    );
    test.add_account(
        gate_key,
        account(controller, gate.try_to_vec().unwrap(), false),
    );
    test.add_account(authority, account(system_program::ID, vec![], false));
    for seat in &seats {
        test.add_account(seat.pubkey(), account(system_program::ID, vec![], false));
    }
    // A system-owned prefund must be preserved, not mistaken for initialized state.
    let mut prefund = account(system_program::ID, vec![], false);
    prefund.lamports = 123_456;
    test.add_account(light_key, prefund.clone());
    let policy = SpreadLightConfigPolicy {
        target: DEVNET_SPREAD,
        expected_epoch: 3,
        deployed_slot: 0,
        compression_authority: Pubkey::new_from_array([42; 32]),
        rent: LightRentPolicy {
            base_rent: 123,
            compression_cost: 456,
            lamports_per_byte_per_epoch: 7,
            max_funded_epochs: 8,
            max_top_up: 901,
        },
        write_top_up: 2345,
        address_tree: Pubkey::new_from_array([43; 32]),
    };
    let body = policy.try_to_vec().unwrap();
    assert_eq!(body.len(), 124);
    let mut action = Action {
        kind: INITIALIZE_SPREAD_LIGHT_CONFIG,
        data: [0; ACTION_LEN],
    };
    action.data[..124].copy_from_slice(&body);
    assert!(!action.is_upgrade());
    let mut ctx = test.start_with_context().await;
    ctx.warp_to_slot(100).unwrap();
    let mut unauthorized_data = vec![216];
    unauthorized_data.extend_from_slice(sponsor.as_ref());
    unauthorized_data.extend_from_slice(policy.compression_authority.as_ref());
    unauthorized_data.extend_from_slice(&policy.rent.try_to_vec().unwrap());
    unauthorized_data.extend_from_slice(&policy.write_top_up.to_le_bytes());
    unauthorized_data.extend_from_slice(&1u32.to_le_bytes());
    unauthorized_data.extend_from_slice(policy.address_tree.as_ref());
    unauthorized_data.extend_from_slice(b"\0AGV1\x01\0\0\0");
    unauthorized_data.extend_from_slice(&3u64.to_le_bytes());
    for supplied_authority in [ro(authority), sig(seats[4].pubkey())] {
        let signed = supplied_authority.is_signer;
        let instruction = Instruction {
            program_id: proxy,
            data: unauthorized_data.clone(),
            accounts: vec![
                ro(DEVNET_SPREAD),
                AccountMeta::new(ctx.payer.pubkey(), true),
                rw(light_key),
                ro(target_pd),
                supplied_authority,
                ro(system_program::ID),
                ro(gate_key),
            ],
        };
        let signers = if signed { vec![&seats[4]] } else { vec![] };
        assert!(
            !send(&mut ctx, instruction, &signers).await,
            "foreign caller cannot use the council-only CPI admission"
        );
        assert_eq!(
            ctx.banks_client
                .get_account(light_key)
                .await
                .unwrap()
                .unwrap(),
            prefund
        );
    }
    let create = ix(
        controller,
        vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            sig(seats[0].pubkey()),
            rw(config_key),
            rw(proposal_key),
            ro(system_program::ID),
        ],
        GovInstruction::Create {
            expected_id: 1,
            action,
        },
    );
    assert!(send(&mut ctx, create, &[&seats[0]]).await);
    let proposal = Proposal::try_from_slice(
        &ctx.banks_client
            .get_account(proposal_key)
            .await
            .unwrap()
            .unwrap()
            .data,
    )
    .unwrap();
    assert_eq!(proposal.review_end - proposal.created, 4_000_000);
    // All approvals occur after the old 450-slot cutoff.
    ctx.warp_to_slot(5_000).unwrap();
    let execute = ix(
        controller,
        vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            ro(config_key),
            rw(proposal_key),
            ro(DEVNET_SPREAD),
            ro(target_pd),
            ro(gate_key),
            ro(authority),
            rw(light_key),
            ro(system_program::ID),
            ro(sysvar::instructions::ID),
        ],
        GovInstruction::ExecuteSpreadLightConfig {
            digest: proposal.digest,
        },
    );
    for seat in seats.iter().take(2) {
        let approve = ix(
            controller,
            vec![ro(config_key), rw(proposal_key), sig(seat.pubkey())],
            GovInstruction::Approve {
                digest: proposal.digest,
            },
        );
        assert!(send(&mut ctx, approve, &[seat]).await);
    }
    assert!(
        !send(&mut ctx, execute.clone(), &[]).await,
        "two seats cannot initialize"
    );
    assert_eq!(
        ctx.banks_client
            .get_account(light_key)
            .await
            .unwrap()
            .unwrap(),
        prefund
    );
    let approve = ix(
        controller,
        vec![ro(config_key), rw(proposal_key), sig(seats[2].pubkey())],
        GovInstruction::Approve {
            digest: proposal.digest,
        },
    );
    assert!(send(&mut ctx, approve, &[&seats[2]]).await);
    let mut stale_gate = gate.clone();
    stale_gate.epoch = 4;
    ctx.set_account(
        &gate_key,
        &account(controller, stale_gate.try_to_vec().unwrap(), false).into(),
    );
    ctx.warp_to_slot(5_001).unwrap();
    assert!(
        !send(&mut ctx, execute.clone(), &[]).await,
        "stale gate rejected"
    );
    ctx.set_account(
        &gate_key,
        &account(controller, gate.try_to_vec().unwrap(), false).into(),
    );
    let mut foreign = prefund.clone();
    foreign.owner = Pubkey::new_unique();
    ctx.set_account(&light_key, &foreign.clone().into());
    ctx.warp_to_slot(5_002).unwrap();
    assert!(
        !send(&mut ctx, execute.clone(), &[]).await,
        "foreign squat rejected"
    );
    assert_eq!(
        ctx.banks_client
            .get_account(light_key)
            .await
            .unwrap()
            .unwrap(),
        foreign
    );
    ctx.set_account(&light_key, &prefund.into());
    ctx.warp_to_slot(5_003).unwrap();
    let before_pd = ctx
        .banks_client
        .get_account(target_pd)
        .await
        .unwrap()
        .unwrap();
    assert!(
        send(&mut ctx, execute.clone(), &[]).await,
        "real controller-to-Spread CPI"
    );
    let light = ctx
        .banks_client
        .get_account(light_key)
        .await
        .unwrap()
        .unwrap();
    let mut expected = b"LightCfg\x01".to_vec();
    expected.extend_from_slice(&2345u32.to_le_bytes());
    for key in [authority, sponsor, policy.compression_authority] {
        expected.extend_from_slice(key.as_ref());
    }
    expected.extend_from_slice(&[
        123,
        0,
        200,
        1,
        7,
        8,
        133,
        3,
        0,
        light_bump,
        sponsor_bump,
        1,
        0,
        0,
        0,
    ]);
    expected.extend_from_slice(policy.address_tree.as_ref());
    assert_eq!(expected.len(), 156);
    assert_eq!(light.owner, DEVNET_SPREAD);
    assert_eq!(light.data, expected);
    assert_eq!(light.lamports, Rent::default().minimum_balance(156));
    assert_eq!(
        ctx.banks_client
            .get_account(target_pd)
            .await
            .unwrap()
            .unwrap(),
        before_pd
    );
    assert_eq!(
        ctx.banks_client
            .get_account(gate_key)
            .await
            .unwrap()
            .unwrap()
            .data,
        gate.try_to_vec().unwrap()
    );
    let executed = Proposal::try_from_slice(
        &ctx.banks_client
            .get_account(proposal_key)
            .await
            .unwrap()
            .unwrap()
            .data,
    )
    .unwrap();
    assert_eq!(executed.state, EXECUTED);
    assert_eq!(executed.approval_count, 3);
    ctx.warp_to_slot(5_004).unwrap();
    assert!(
        !send(&mut ctx, execute, &[]).await,
        "execution cannot repeat"
    );
    assert_eq!(
        ctx.banks_client
            .get_account(light_key)
            .await
            .unwrap()
            .unwrap(),
        light
    );
}
