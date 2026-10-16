#![allow(deprecated)]

use solana_loader_v3_interface::instruction::{
    deploy_with_max_program_len, UpgradeableLoaderInstruction,
};
use solana_pubkey::Pubkey;
use solana_sdk_ids::{bpf_loader_upgradeable, system_program, sysvar as sysvar_ids};
use solana_system_interface::instruction as system_instruction;

const CONTROLLER_ARTIFACT_LEN: usize = 1_114_592;
const DEPLOY_WITH_MAX_DATA_LEN_GOLDEN: [u8; 12] = [
    0x02, 0x00, 0x00, 0x00, 0xe0, 0x01, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00,
];

#[test]
fn pinned_loader_v3_deploy_bincode_matches_operator_golden() {
    let direct = bincode::serialize(&UpgradeableLoaderInstruction::DeployWithMaxDataLen {
        max_data_len: CONTROLLER_ARTIFACT_LEN,
    })
    .expect("serialize pinned Loader-v3 instruction");
    assert_eq!(direct, DEPLOY_WITH_MAX_DATA_LEN_GOLDEN);

    let payer = Pubkey::new_unique();
    let program = Pubkey::new_unique();
    let buffer = Pubkey::new_unique();
    let authority = Pubkey::new_unique();
    let instructions = deploy_with_max_program_len(
        &payer,
        &program,
        &buffer,
        &authority,
        1_234_567,
        CONTROLLER_ARTIFACT_LEN,
    )
    .expect("construct pinned Loader-v3 deployment envelope");
    assert_eq!(instructions.len(), 2);
    assert_eq!(
        instructions[0],
        system_instruction::create_account(
            &payer,
            &program,
            1_234_567,
            36,
            &bpf_loader_upgradeable::ID,
        ),
    );
    assert_eq!(instructions[1].data, DEPLOY_WITH_MAX_DATA_LEN_GOLDEN);
    assert_eq!(instructions[1].accounts.len(), 8);
    assert_eq!(instructions[1].accounts[0].pubkey, payer);
    assert!(instructions[1].accounts[0].is_signer);
    assert!(instructions[1].accounts[0].is_writable);
    let (programdata, _) =
        Pubkey::find_program_address(&[program.as_ref()], &bpf_loader_upgradeable::ID);
    assert_eq!(instructions[1].accounts[1].pubkey, programdata);
    assert!(!instructions[1].accounts[1].is_signer);
    assert!(instructions[1].accounts[1].is_writable);
    assert_eq!(instructions[1].accounts[2].pubkey, program);
    assert!(!instructions[1].accounts[2].is_signer);
    assert!(instructions[1].accounts[2].is_writable);
    assert_eq!(instructions[1].accounts[3].pubkey, buffer);
    assert!(!instructions[1].accounts[3].is_signer);
    assert!(instructions[1].accounts[3].is_writable);
    assert_eq!(instructions[1].accounts[4].pubkey, sysvar_ids::rent::ID);
    assert!(!instructions[1].accounts[4].is_signer);
    assert!(!instructions[1].accounts[4].is_writable);
    assert_eq!(instructions[1].accounts[5].pubkey, sysvar_ids::clock::ID);
    assert!(!instructions[1].accounts[5].is_signer);
    assert!(!instructions[1].accounts[5].is_writable);
    assert_eq!(instructions[1].accounts[6].pubkey, system_program::ID);
    assert!(!instructions[1].accounts[6].is_signer);
    assert!(!instructions[1].accounts[6].is_writable);
    assert_eq!(instructions[1].accounts[7].pubkey, authority);
    assert!(instructions[1].accounts[7].is_signer);
    assert!(!instructions[1].accounts[7].is_writable);
}
