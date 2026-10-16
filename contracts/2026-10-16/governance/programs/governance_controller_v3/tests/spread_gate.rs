//! Verify the exact fresh Spread SBF enforces the V3-owned frozen gate.
use borsh::BorshSerialize;
use governance_controller_v3::state::{config_pda, gate_pda, target_authority_pda};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey,
    rent::Rent,
};
use solana_program_test::ProgramTest;
use solana_sdk::{
    account::Account, instruction::InstructionError, signature::Signer, transaction::Transaction,
    transaction::TransactionError,
};
use solana_sdk_ids::bpf_loader_upgradeable::ID as LOADER;
use upgrade_controller::state::{GateStatusV1, ProtocolGateV1};

#[tokio::test]
#[ignore = "requires the exact controller/Spread SBF artifacts; explicitly run by the SBF CI job"]
async fn fresh_spread_elf_rejects_business_dispatch_while_v3_gate_is_frozen() {
    let program = pubkey!("2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw");
    let controller = pubkey!("8fhNi6QHU5TYNhoPDM4vs89ZBztnpxp3LnBXRgkBVKtx");
    let pd = solana_program::pubkey::Pubkey::find_program_address(&[program.as_ref()], &LOADER).0;
    let artifact = std::fs::read(
        std::env::var("AMOEBA_FRESH_SPREAD_SBF").expect("exact fresh Spread ELF required"),
    )
    .unwrap();
    let (gate_key, bump) = gate_pda(&controller, &program);
    let gate = ProtocolGateV1 {
        discriminator: *b"AGVGAT01",
        version: 1,
        bump,
        initialized: true,
        status: GateStatusV1::EmergencyFrozen,
        controller_config: config_pda(&controller).0,
        target_program: program,
        target_programdata: pd,
        epoch: 1,
        active_proposal: Default::default(),
        freeze_slot: 1,
        freeze_reason_code: 1,
        last_completed_proposal: Default::default(),
        reserved: [0; 2],
    };
    let account = |owner, data: Vec<u8>, executable| Account {
        lamports: Rent::default().minimum_balance(data.len()),
        owner,
        data,
        executable,
        rent_epoch: 0,
    };
    let mut pbytes = 2u32.to_le_bytes().to_vec();
    pbytes.extend_from_slice(pd.as_ref());
    let mut dbytes = 3u32.to_le_bytes().to_vec();
    dbytes.extend_from_slice(&0u64.to_le_bytes());
    dbytes.push(1);
    dbytes.extend_from_slice(target_authority_pda(&controller, &program).0.as_ref());
    dbytes.extend_from_slice(&artifact);
    let mut test = ProgramTest::default();
    test.prefer_bpf(true);
    test.add_genesis_account(program, account(LOADER, pbytes, true));
    test.add_genesis_account(pd, account(LOADER, dbytes, false));
    test.add_account(
        gate_key,
        account(controller, gate.try_to_vec().unwrap(), false),
    );
    let mut ctx = test.start_with_context().await;
    ctx.warp_to_slot(100).unwrap();
    let mut data = vec![2];
    data.extend_from_slice(b"AGV1");
    data.extend_from_slice(&[1, 0, 0, 0]);
    data.extend_from_slice(&1u64.to_le_bytes());
    let instruction = Instruction {
        program_id: program,
        accounts: vec![AccountMeta::new_readonly(gate_key, false)],
        data,
    };
    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[instruction],
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    let result = ctx
        .banks_client
        .process_transaction(tx)
        .await
        .unwrap_err()
        .unwrap();
    assert_eq!(
        result,
        TransactionError::InstructionError(0, InstructionError::Custom(6263))
    );
}
