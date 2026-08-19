//! Minimal, byte-exact Light Token CPI constructors used by the program.
//!
//! The runtime needs index-zero SPL-interface creation and checked token movement between classic
//! SPL and Light token accounts. Encoding those fixed cases directly avoids linking the SDK's
//! general transfer router while preserving the exact target program, metas, and data bytes.

use solana_program::{
    instruction::{AccountMeta, Instruction},
    program_error::ProgramError,
    pubkey::Pubkey,
};

use crate::{
    constants::{LIGHT_TOKEN_COMPRESSIBLE_CONFIG, LIGHT_TOKEN_RENT_SPONSOR},
    token_instruction,
};

pub(crate) const LIGHT_TOKEN_PROGRAM_ID: Pubkey =
    solana_program::pubkey!("cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m");
pub(crate) const LIGHT_TOKEN_CPI_AUTHORITY: Pubkey =
    solana_program::pubkey!("GXtd2izAiMJPwMEjfgTRH3d7k9mjn4Jq3JrWFv9gySYy");
pub(crate) const CANONICAL_COMPRESSIBLE_TOKEN_ACCOUNT_LEN: usize = 272;

const CREATE_TOKEN_POOL: [u8; 8] = [23, 169, 27, 122, 147, 169, 209, 152];
const CREATE_ASSOCIATED_TOKEN_ACCOUNT_IDEMPOTENT: u8 = 102;
const TRANSFER2: u8 = 101;
const LIGHT_CANNOT_DETERMINE_ACCOUNT_TYPE: u32 = 17_503;

pub(crate) const fn light_token_program_id() -> Pubkey {
    LIGHT_TOKEN_PROGRAM_ID
}

pub(crate) const fn cpi_authority() -> Pubkey {
    LIGHT_TOKEN_CPI_AUTHORITY
}

pub(crate) const fn compressible_config() -> Pubkey {
    LIGHT_TOKEN_COMPRESSIBLE_CONFIG
}

pub(crate) const fn rent_sponsor() -> Pubkey {
    LIGHT_TOKEN_RENT_SPONSOR
}

pub(crate) fn has_canonical_compressible_token_layout(data: &[u8]) -> bool {
    data.len() == CANONICAL_COMPRESSIBLE_TOKEN_ACCOUNT_LEN
        && data[165] == 2
        && data[166] == 1
        && data[167..171] == 1_u32.to_le_bytes()
        && data[171] == 32
}

pub(crate) fn get_spl_interface_pda_and_bump(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"pool", mint.as_ref()], &light_token_program_id())
}

pub(crate) fn get_associated_token_address(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    let light = light_token_program_id();
    Pubkey::find_program_address(&[owner.as_ref(), light.as_ref(), mint.as_ref()], &light).0
}

pub(crate) fn create_associated_token_account_idempotent(
    payer: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    compressible_config: &Pubkey,
    rent_sponsor: &Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(10);
    data.extend_from_slice(&[
        CREATE_ASSOCIATED_TOKEN_ACCOUNT_IDEMPOTENT,
        1,  // Some(compressible extension)
        3,  // TokenDataVersion::ShaFlat
        16, // initial rent epochs
        1,  // compression-only ATA
    ]);
    data.extend_from_slice(&766u32.to_le_bytes());
    data.push(0); // no alternate compression destination
    Instruction {
        program_id: light_token_program_id(),
        accounts: vec![
            AccountMeta::new_readonly(*owner, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new(*payer, true),
            AccountMeta::new(get_associated_token_address(owner, mint), false),
            AccountMeta::new_readonly(Pubkey::default(), false),
            AccountMeta::new_readonly(*compressible_config, false),
            AccountMeta::new(*rent_sponsor, false),
        ],
        data,
    }
}

/// Byte-exact rent-free Light token-account creation for a program-derived
/// vault. `signer_seeds` includes its one-byte bump as the final seed.
pub(crate) fn create_token_account_rent_free(
    payer: &Pubkey,
    account: &Pubkey,
    mint: &Pubkey,
    owner: &Pubkey,
    compressible_config: &Pubkey,
    rent_sponsor: &Pubkey,
    system_program: &Pubkey,
    invoking_program_id: &Pubkey,
    signer_seeds: &[&[u8]],
) -> Result<Instruction, ProgramError> {
    let bump = signer_seeds
        .last()
        .and_then(|seed| (seed.len() == 1).then_some(seed[0]))
        .ok_or(ProgramError::InvalidSeeds)?;
    let base_seeds = signer_seeds
        .get(..signer_seeds.len().saturating_sub(1))
        .ok_or(ProgramError::InvalidSeeds)?;
    let mut data = Vec::with_capacity(
        78 + base_seeds
            .iter()
            .map(|seed| 4usize.saturating_add(seed.len()))
            .sum::<usize>(),
    );
    data.push(18); // InitializeAccount3 / CreateTokenAccount
    data.extend_from_slice(owner.as_ref());
    data.extend_from_slice(&[
        1,  // Some(compressible extension)
        3,  // TokenDataVersion::ShaFlat
        16, // initial rent epochs
        0,  // program vaults may decompress for use
    ]);
    data.extend_from_slice(&766u32.to_le_bytes());
    data.push(1); // Some(program-derived compression destination)
    data.push(bump);
    data.extend_from_slice(invoking_program_id.as_ref());
    data.extend_from_slice(&(base_seeds.len() as u32).to_le_bytes());
    for seed in base_seeds {
        data.extend_from_slice(&(seed.len() as u32).to_le_bytes());
        data.extend_from_slice(seed);
    }
    Ok(Instruction {
        program_id: light_token_program_id(),
        accounts: vec![
            AccountMeta::new(*account, true),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(*compressible_config, false),
            AccountMeta::new_readonly(*system_program, false),
            AccountMeta::new(*rent_sponsor, false),
        ],
        data,
    })
}

pub(crate) fn create_spl_interface_pda(
    payer: &Pubkey,
    mint: &Pubkey,
    spl_token_program: &Pubkey,
) -> Instruction {
    let spl_interface = get_spl_interface_pda_and_bump(mint).0;
    Instruction {
        program_id: light_token_program_id(),
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(spl_interface, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
            AccountMeta::new(*mint, false),
            AccountMeta::new_readonly(*spl_token_program, false),
            AccountMeta::new_readonly(cpi_authority(), false),
        ],
        data: CREATE_TOKEN_POOL.to_vec(),
    }
}

#[derive(Clone, Copy)]
enum CompressionMode {
    Compress = 0,
    Decompress = 1,
}

#[allow(clippy::too_many_arguments)]
fn append_compression(
    data: &mut Vec<u8>,
    mode: CompressionMode,
    amount: u64,
    mint: u8,
    source_or_recipient: u8,
    authority: u8,
    pool_account_index: u8,
    pool_index: u8,
    bump: u8,
    decimals: u8,
) {
    data.push(mode as u8);
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&[
        mint,
        source_or_recipient,
        authority,
        pool_account_index,
        pool_index,
        bump,
        decimals,
    ]);
}

fn transfer2_data(amount: u64, bump: u8, decimals: u8, from_spl: bool) -> Vec<u8> {
    let mut data = Vec::with_capacity(59);
    data.push(TRANSFER2);
    // Fixed Transfer2 header through max_top_up. The SDK's no-limit default is u16::MAX.
    data.extend_from_slice(&[0, 0, 0, 0, 0, 0xff, 0xff]);
    // cpi_context = None, compressions = Some(two entries)
    data.extend_from_slice(&[0, 1]);
    data.extend_from_slice(&2u32.to_le_bytes());
    if from_spl {
        append_compression(
            &mut data,
            CompressionMode::Compress,
            amount,
            0,
            3,
            2,
            4,
            0,
            bump,
            decimals,
        );
        append_compression(
            &mut data,
            CompressionMode::Decompress,
            amount,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
        );
    } else {
        append_compression(
            &mut data,
            CompressionMode::Compress,
            amount,
            0,
            1,
            3,
            0,
            0,
            0,
            0,
        );
        append_compression(
            &mut data,
            CompressionMode::Decompress,
            amount,
            0,
            2,
            0,
            4,
            0,
            bump,
            decimals,
        );
    }
    // proof = None; in/out token vectors are empty; in/out lamports and TLVs are None.
    data.push(0);
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&[0, 0, 0, 0]);
    data
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn transfer_interface(
    source: &Pubkey,
    destination: &Pubkey,
    amount: u64,
    decimals: u8,
    authority: &Pubkey,
    payer: &Pubkey,
    mint: &Pubkey,
    spl_interface: &Pubkey,
    spl_interface_bump: u8,
    spl_token_program: &Pubkey,
    source_owner: &Pubkey,
    destination_owner: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let light = light_token_program_id();
    let spl = token_instruction::id();
    match (*source_owner == light, *destination_owner == light) {
        (true, true) => {
            let mut data = vec![12];
            data.extend_from_slice(&amount.to_le_bytes());
            data.push(decimals);
            Ok(Instruction {
                program_id: light,
                accounts: vec![
                    AccountMeta::new(*source, false),
                    AccountMeta::new_readonly(*mint, false),
                    AccountMeta::new(*destination, false),
                    AccountMeta::new_readonly(*authority, true),
                    AccountMeta::new_readonly(Pubkey::default(), false),
                    AccountMeta::new(*payer, true),
                ],
                data,
            })
        }
        (false, true) if *source_owner == spl => Ok(Instruction {
            program_id: light,
            accounts: vec![
                AccountMeta::new_readonly(cpi_authority(), false),
                AccountMeta::new(*payer, true),
                AccountMeta::new_readonly(*mint, false),
                AccountMeta::new(*destination, false),
                AccountMeta::new_readonly(*authority, true),
                AccountMeta::new(*source, false),
                AccountMeta::new(*spl_interface, false),
                AccountMeta::new_readonly(*spl_token_program, false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: transfer2_data(amount, spl_interface_bump, decimals, true),
        }),
        (true, false) if *destination_owner == spl => Ok(Instruction {
            program_id: light,
            accounts: vec![
                AccountMeta::new_readonly(cpi_authority(), false),
                AccountMeta::new(*payer, true),
                AccountMeta::new_readonly(*mint, false),
                AccountMeta::new(*source, false),
                AccountMeta::new(*destination, false),
                AccountMeta::new_readonly(*authority, true),
                AccountMeta::new(*spl_interface, false),
                AccountMeta::new_readonly(*spl_token_program, false),
            ],
            data: transfer2_data(amount, spl_interface_bump, decimals, false),
        }),
        (false, false) if source_owner == destination_owner && *source_owner == spl => {
            token_instruction::transfer_checked(
                source_owner,
                source,
                mint,
                destination,
                authority,
                &[],
                amount,
                decimals,
            )
        }
        _ => Err(ProgramError::Custom(LIGHT_CANNOT_DETERMINE_ACCOUNT_TYPE)),
    }
}
