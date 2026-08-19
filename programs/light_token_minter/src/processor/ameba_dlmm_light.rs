//! Narrow Light 0.23 account adapter for native DLMM state.
//!
//! Lifecycle payloads and accounts retain the Light account ABI. The current
//! program's one-byte instruction tags select the four fixed-layout state
//! transitions without linking the generic variant/dispatch framework.

use std::io::{Result as IoResult, Write};

use borsh::{BorshDeserialize, BorshSerialize};
use light_compressed_account::{
    address::derive_address,
    compressed_account::PackedMerkleContext,
    instruction_data::{compressed_proof::ValidityProof, data::NewAddressParamsAssignedPacked},
};
use light_compressible::rent::{AccountRentState, RentConfig};
use light_sdk::cpi::v2::lowlevel::{CompressedAccountInfo, InAccountInfo, OutAccountInfo};
use light_sdk_types::{
    constants::RENT_SPONSOR_SEED,
    instruction::{account_meta::CompressedAccountMetaNoLamportsNoAddress, PackedStateTreeInfo},
    interface::{
        account::compression_info::{CompressionInfo, CompressionState},
        create_accounts_proof::CreateAccountsProof,
    },
};
use solana_program::{
    account_info::AccountInfo,
    clock::Clock,
    entrypoint::ProgramResult,
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar::{rent::Rent, Sysvar},
};
use solana_sdk_ids::system_program;

use super::super::{
    ameba_dlmm_instruction::AmoebaDlmmInstructionTag,
    ameba_dlmm_state::{
        AmoebaDlmmBinPageV1, AmoebaDlmmLightState, AmoebaDlmmPoolV1, AmoebaDlmmPositionV1,
        AmoebaDlmmSharePageV1, AMOEBA_DLMM_BIN_PAGE_LIGHT_DISCRIMINATOR,
        AMOEBA_DLMM_POOL_LIGHT_DISCRIMINATOR, AMOEBA_DLMM_POSITION_LIGHT_DISCRIMINATOR,
        AMOEBA_DLMM_SHARE_PAGE_LIGHT_DISCRIMINATOR,
    },
    compression::{hash_leaf_data, invoke_light_cpi, new_light_system_cpi},
    constants::{
        AMOEBA_DLMM_BIN_PAGE_PDA_SEED, AMOEBA_DLMM_POOL_PDA_SEED, AMOEBA_DLMM_POSITION_PDA_SEED,
        AMOEBA_DLMM_SHARE_PAGE_PDA_SEED, CURRENT_STATE_NAMESPACE_SEED,
    },
    error::VaultError,
    fixed_codec::{CheckedCursor, FixedCursor, FixedField, FixedWriter},
    instruction::{deserialize_packed_state_tree_info_cursor, deserialize_validity_proof_cursor},
};
use super::{
    invoke_create_or_allocate_account, invoke_system_allocate, invoke_system_assign,
    invoke_system_create_account, invoke_system_transfer,
};

const DECOMPRESSED_PDA_DISCRIMINATOR: [u8; 8] = [255, 255, 255, 255, 255, 255, 255, 0];
const LIGHT_FIXED_ACCOUNTS: usize = 6;
const LIGHT_CONFIG_DISCRIMINATOR: [u8; 8] = *b"LightCfg";
const LIGHT_CONFIG_SEED: &[u8] = b"compressible_config";
const LIGHT_CONFIG_LEN: usize = 156;
const BPF_LOADER_UPGRADEABLE_ID: Pubkey = Pubkey::new_from_array([
    2, 168, 246, 145, 78, 136, 161, 110, 57, 90, 225, 40, 148, 143, 144, 16, 207, 227, 47, 228,
    248, 212, 16, 185, 221, 165, 30, 160, 42, 103, 43, 122,
]);

#[derive(Clone, Debug, Eq, PartialEq)]
struct AmoebaLightConfig {
    version: u8,
    write_top_up: u32,
    update_authority: [u8; 32],
    rent_sponsor: [u8; 32],
    compression_authority: [u8; 32],
    rent_config: RentConfig,
    config_bump: u8,
    bump: u8,
    rent_sponsor_bump: u8,
    address_tree: [u8; 32],
}

#[inline(always)]
fn decompressed_compression_info(config: &AmoebaLightConfig, slot: u64) -> CompressionInfo {
    CompressionInfo {
        last_claimed_slot: slot,
        lamports_per_write: config.write_top_up,
        config_version: config.version as u16,
        state: CompressionState::Decompressed,
        _padding: 0,
        rent_config: config.rent_config,
    }
}

fn invalid_light() -> ProgramError {
    VaultError::InvalidAmoebaDlmmLightLifecycle.into()
}

#[inline(always)]
fn configured_address_tree_key(
    configured: &[u8; 32],
    selected: &Pubkey,
) -> Result<[u8; 32], ProgramError> {
    let key = selected.to_bytes();
    if &key != configured {
        return Err(invalid_light());
    }
    Ok(key)
}

fn read_rent_config(reader: &mut CheckedCursor<'_>) -> RentConfig {
    RentConfig {
        base_rent: reader.u16(),
        compression_cost: reader.u16(),
        lamports_per_byte_per_epoch: reader.u8(),
        max_funded_epochs: reader.u8(),
        max_top_up: reader.u16(),
    }
}

fn read_optional_bytes32(reader: &mut CheckedCursor<'_>) -> Option<[u8; 32]> {
    match reader.u8() {
        0 => None,
        1 => Some(reader.bytes()),
        _ => {
            reader.invalid = true;
            None
        }
    }
}

fn read_optional_rent_config(reader: &mut CheckedCursor<'_>) -> Option<RentConfig> {
    match reader.u8() {
        0 => None,
        1 => Some(read_rent_config(reader)),
        _ => {
            reader.invalid = true;
            None
        }
    }
}

fn read_optional_u32(reader: &mut CheckedCursor<'_>) -> Option<u32> {
    match reader.u8() {
        0 => None,
        1 => Some(reader.u32()),
        _ => {
            reader.invalid = true;
            None
        }
    }
}

fn read_optional_address_space(reader: &mut CheckedCursor<'_>) -> Option<[u8; 32]> {
    match reader.u8() {
        0 => None,
        1 if reader.u32() == 1 => Some(reader.bytes()),
        _ => {
            reader.invalid = true;
            None
        }
    }
}

fn fixed_bytes<const LENGTH: usize>(data: &[u8], offset: usize) -> [u8; LENGTH] {
    // All callers first check the exact fixed config allocation.
    unsafe { *(data.as_ptr().add(offset).cast::<[u8; LENGTH]>()) }
}

fn load_light_config(
    program_id: &Pubkey,
    config_info: &AccountInfo,
) -> Result<AmoebaLightConfig, ProgramError> {
    if config_info.owner != program_id
        || config_info.executable
        || config_info.data_len() != LIGHT_CONFIG_LEN
    {
        return Err(invalid_light());
    }
    let data = config_info.try_borrow_data().map_err(|_| invalid_light())?;
    if data[..8] != LIGHT_CONFIG_DISCRIMINATOR {
        return Err(invalid_light());
    }
    let body = &data[8..];
    if body[0] != 1 || body[109] != 0 || u32::from_le_bytes(fixed_bytes(body, 112)) != 1 {
        return Err(invalid_light());
    }
    let config_bump_seed = (body[109] as u16).to_le_bytes();
    let (expected, expected_bump) =
        Pubkey::find_program_address(&[LIGHT_CONFIG_SEED, &config_bump_seed], program_id);
    if expected != *config_info.key || expected_bump != body[110] {
        return Err(invalid_light());
    }
    Ok(AmoebaLightConfig {
        version: body[0],
        write_top_up: u32::from_le_bytes(fixed_bytes(body, 1)),
        update_authority: fixed_bytes(body, 5),
        rent_sponsor: fixed_bytes(body, 37),
        compression_authority: fixed_bytes(body, 69),
        rent_config: RentConfig {
            base_rent: u16::from_le_bytes(fixed_bytes(body, 101)),
            compression_cost: u16::from_le_bytes(fixed_bytes(body, 103)),
            lamports_per_byte_per_epoch: body[105],
            max_funded_epochs: body[106],
            max_top_up: u16::from_le_bytes(fixed_bytes(body, 107)),
        },
        config_bump: body[109],
        bump: body[110],
        rent_sponsor_bump: body[111],
        address_tree: fixed_bytes(body, 116),
    })
}

fn write_light_config(config_info: &AccountInfo, config: &AmoebaLightConfig) -> ProgramResult {
    if config_info.data_len() != LIGHT_CONFIG_LEN {
        return Err(invalid_light());
    }
    let mut data = config_info
        .try_borrow_mut_data()
        .map_err(|_| invalid_light())?;
    data[..8].copy_from_slice(&LIGHT_CONFIG_DISCRIMINATOR);
    let body = &mut data[8..];
    body[0] = config.version;
    body[1..5].copy_from_slice(&config.write_top_up.to_le_bytes());
    body[5..37].copy_from_slice(&config.update_authority);
    body[37..69].copy_from_slice(&config.rent_sponsor);
    body[69..101].copy_from_slice(&config.compression_authority);
    body[101..103].copy_from_slice(&config.rent_config.base_rent.to_le_bytes());
    body[103..105].copy_from_slice(&config.rent_config.compression_cost.to_le_bytes());
    body[105] = config.rent_config.lamports_per_byte_per_epoch;
    body[106] = config.rent_config.max_funded_epochs;
    body[107..109].copy_from_slice(&config.rent_config.max_top_up.to_le_bytes());
    body[109] = config.config_bump;
    body[110] = config.bump;
    body[111] = config.rent_sponsor_bump;
    body[112..116].copy_from_slice(&1u32.to_le_bytes());
    body[116..148].copy_from_slice(&config.address_tree);
    Ok(())
}

fn create_config_account<'info>(
    program_id: &Pubkey,
    payer: &AccountInfo<'info>,
    config_info: &AccountInfo<'info>,
    system_program_info: &AccountInfo<'info>,
    bump: u8,
) -> ProgramResult {
    if !payer.is_signer
        || !payer.is_writable
        || !config_info.is_writable
        || config_info.executable
        || config_info.owner != &system_program::id()
        || config_info.data_len() != 0
        || *system_program_info.key != system_program::id()
    {
        return Err(invalid_light());
    }
    let config_bump = 0u16.to_le_bytes();
    let bump_seed = [bump];
    let seeds: &[&[u8]] = &[LIGHT_CONFIG_SEED, &config_bump, &bump_seed];
    invoke_create_or_allocate_account(
        payer,
        config_info,
        system_program_info,
        program_id,
        LIGHT_CONFIG_LEN,
        seeds,
    )
    .map_err(|_| invalid_light())
}

fn check_upgrade_authority(
    program_id: &Pubkey,
    program_data: &AccountInfo,
    authority: &AccountInfo,
) -> ProgramResult {
    let (expected, _) =
        Pubkey::find_program_address(&[program_id.as_ref()], &BPF_LOADER_UPGRADEABLE_ID);
    if expected != *program_data.key || !authority.is_signer {
        return Err(invalid_light());
    }
    let data = program_data
        .try_borrow_data()
        .map_err(|_| invalid_light())?;
    if data.len() < 45 || u32::from_le_bytes(fixed_bytes(&data, 0)) != 3 || data[12] != 1 {
        return Err(invalid_light());
    }
    let expected_authority: [u8; 32] = fixed_bytes(&data, 13);
    if crate::bytes32_is_zero(&expected_authority) || expected_authority != authority.key.to_bytes()
    {
        return Err(invalid_light());
    }
    Ok(())
}

#[inline(never)]
fn process_initialize_config(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    if accounts.len() < 5 || payload.len() != 113 {
        return Err(invalid_light());
    }
    let mut reader = CheckedCursor::new(payload);
    let rent_sponsor = reader.bytes();
    let compression_authority = reader.bytes();
    let rent_config = read_rent_config(&mut reader);
    let write_top_up = reader.u32();
    if reader.u32() != 1 {
        return Err(invalid_light());
    }
    let address_tree = reader.bytes();
    let config_bump = reader.u8();
    reader.finish_exact().map_err(|_| invalid_light())?;
    if config_bump != 0 {
        return Err(invalid_light());
    }
    check_upgrade_authority(program_id, &accounts[2], &accounts[3])?;
    let config_bump_seed = 0u16.to_le_bytes();
    let (expected_config, bump) =
        Pubkey::find_program_address(&[LIGHT_CONFIG_SEED, &config_bump_seed], program_id);
    let (expected_sponsor, sponsor_bump) =
        Pubkey::find_program_address(&[RENT_SPONSOR_SEED], program_id);
    if expected_config != *accounts[1].key || expected_sponsor.to_bytes() != rent_sponsor {
        return Err(invalid_light());
    }
    create_config_account(program_id, &accounts[0], &accounts[1], &accounts[4], bump)?;
    write_light_config(
        &accounts[1],
        &AmoebaLightConfig {
            version: 1,
            write_top_up,
            update_authority: accounts[3].key.to_bytes(),
            rent_sponsor,
            compression_authority,
            rent_config,
            config_bump,
            bump,
            rent_sponsor_bump: sponsor_bump,
            address_tree,
        },
    )
}

#[inline(never)]
fn process_update_config(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    if accounts.len() < 2 || !accounts[0].is_writable || !accounts[1].is_signer {
        return Err(invalid_light());
    }
    let mut reader = CheckedCursor::new(payload);
    let new_update_authority = read_optional_bytes32(&mut reader);
    let new_rent_sponsor = read_optional_bytes32(&mut reader);
    let new_compression_authority = read_optional_bytes32(&mut reader);
    let new_rent_config = read_optional_rent_config(&mut reader);
    let new_write_top_up = read_optional_u32(&mut reader);
    let new_address_tree = read_optional_address_space(&mut reader);
    reader.finish_exact().map_err(|_| invalid_light())?;
    let mut config = load_light_config(program_id, &accounts[0])?;
    if config.update_authority != accounts[1].key.to_bytes() {
        return Err(invalid_light());
    }
    if let Some(value) = new_update_authority {
        config.update_authority = value;
    }
    if let Some(value) = new_rent_sponsor {
        config.rent_sponsor = value;
    }
    if let Some(value) = new_compression_authority {
        config.compression_authority = value;
    }
    if let Some(value) = new_rent_config {
        config.rent_config = value;
    }
    if let Some(value) = new_write_top_up {
        config.write_top_up = value;
    }
    if let Some(value) = new_address_tree {
        if config.address_tree != value {
            return Err(invalid_light());
        }
    }
    write_light_config(&accounts[0], &config)
}

mod lifecycle;

use lifecycle::*;

#[inline(never)]
pub fn process_lifecycle_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction: AmoebaDlmmInstructionTag,
    payload: &[u8],
) -> ProgramResult {
    match instruction {
        AmoebaDlmmInstructionTag::InitializeLightConfig => {
            process_initialize_config(program_id, accounts, payload)
        }
        AmoebaDlmmInstructionTag::UpdateLightConfig => {
            process_update_config(program_id, accounts, payload)
        }
        AmoebaDlmmInstructionTag::CompressLightState => {
            process_compress(program_id, accounts, payload)
        }
        AmoebaDlmmInstructionTag::DecompressLightState => {
            process_decompress(program_id, accounts, payload)
        }
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

/// Register newly-created hot PDAs as Light-address placeholders and reimburse
/// their payer from the program's configured rent sponsor.
pub fn register_initialized_pdas<'info>(
    program_id: &Pubkey,
    payer: &AccountInfo<'info>,
    config_info: &AccountInfo<'info>,
    rent_sponsor: &AccountInfo<'info>,
    payer_rent_lamports: u64,
    created_accounts: &[AccountInfo<'info>],
    light_tail_accounts: &[AccountInfo<'info>],
    proof: &CreateAccountsProof,
) -> ProgramResult {
    if created_accounts.is_empty()
        || created_accounts.len() > u8::MAX as usize
        || !payer.is_signer
        || !payer.is_writable
    {
        return Err(invalid_light());
    }
    let (config, rent_sponsor_bump) =
        load_config_and_sponsor(program_id, config_info, rent_sponsor)?;
    let system_accounts = light_tail_accounts
        .get(proof.system_accounts_offset as usize..)
        .ok_or_else(invalid_light)?;
    if system_accounts.len() < LIGHT_FIXED_ACCOUNTS {
        return Err(invalid_light());
    }
    let system_program_info = &system_accounts[5];
    if *system_program_info.key != system_program::id() {
        return Err(invalid_light());
    }
    let address_tree = system_accounts
        .get(
            LIGHT_FIXED_ACCOUNTS
                + proof.address_tree_info.address_merkle_tree_pubkey_index as usize,
        )
        .ok_or_else(invalid_light)?;
    let address_tree_key = configured_address_tree_key(&config.address_tree, address_tree.key)?;
    let mut new_address_params = Vec::with_capacity(created_accounts.len());
    let mut account_infos = Vec::with_capacity(created_accounts.len());
    for (index, account) in created_accounts.iter().enumerate() {
        if account.owner != program_id || account.executable || account.data_len() < 8 {
            return Err(invalid_light());
        }
        let key = account.key.to_bytes();
        new_address_params.push(NewAddressParamsAssignedPacked {
            seed: key,
            address_merkle_tree_account_index: proof
                .address_tree_info
                .address_merkle_tree_pubkey_index,
            address_queue_account_index: proof.address_tree_info.address_queue_pubkey_index,
            address_merkle_tree_root_index: proof.address_tree_info.root_index,
            assigned_to_account: true,
            assigned_account_index: index as u8,
        });
        account_infos.push(CompressedAccountInfo {
            address: Some(derive_address(
                &key,
                &address_tree_key,
                &program_id.to_bytes(),
            )),
            input: None,
            output: Some(OutAccountInfo {
                discriminator: DECOMPRESSED_PDA_DISCRIMINATOR,
                data_hash: hash_leaf_data(&key).map_err(|_| invalid_light())?,
                output_merkle_tree_index: proof.output_state_tree_index,
                lamports: 0,
                data: key.to_vec(),
            }),
        });
    }

    let mut instruction = new_light_system_cpi(proof.proof.clone());
    instruction.new_address_params = new_address_params;
    instruction.account_infos = account_infos;
    invoke_light_cpi(instruction, payer, system_accounts).map_err(|_| invalid_light())?;

    if payer_rent_lamports > 0 {
        let bump = [rent_sponsor_bump];
        invoke_system_transfer(
            rent_sponsor,
            payer,
            system_program_info,
            payer_rent_lamports,
            &[&[RENT_SPONSOR_SEED, &bump]],
        )
        .map_err(|_| invalid_light())?;
    }
    Ok(())
}

pub fn initialize_compression_info<T: AmoebaDlmmLightState>(
    program_id: &Pubkey,
    state: &mut T,
    config_info: &AccountInfo,
    slot: u64,
) -> ProgramResult {
    let config = load_light_config(program_id, config_info)?;
    *state.compression_info_mut() = decompressed_compression_info(&config, slot);
    Ok(())
}
