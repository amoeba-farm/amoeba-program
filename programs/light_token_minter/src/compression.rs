use borsh::BorshSerialize;
use light_compressed_account::compressed_account::{
    PackedMerkleContext, PackedReadOnlyCompressedAccount,
};
use light_compressed_account::InstructionDiscriminator;
use light_sdk::{
    address::v2::derive_address,
    address::AddressSeed,
    constants::{CPI_AUTHORITY_PDA_SEED, LIGHT_SYSTEM_PROGRAM_ID},
    cpi::v2::{
        lowlevel::{CompressedAccountInfo, InAccountInfo, OutAccountInfo},
        LightSystemProgramCpi,
    },
    instruction::account_meta::{CompressedAccountMeta, CompressedAccountMetaReadOnly},
    light_hasher::{hash_to_field_size::hash_to_bn254_field_size_be, Hasher, Poseidon},
    proof::borsh_compat::ValidityProof,
    LightDiscriminator,
};
use solana_program::{
    account_info::AccountInfo,
    hash::hash,
    instruction::{AccountMeta, Instruction},
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
};

use crate::{
    constants::{
        COMPRESSED_STATE_LEAF_ADDRESS_SEED, CURRENT_STATE_NAMESPACE_SEED,
        LIGHT_DEFAULT_ADDRESS_TREE_V2, MARKET_PAGE_LEAF_ADDRESS_SEED, SETTLEMENT_LEAF_ADDRESS_SEED,
    },
    error::VaultError,
    instruction::CompressionOutput,
    state::{
        CompressedAmebaStateLeaf, CompressedMarketPageLeaf, CompressedSettlementLeaf,
        CompressedStateDomain,
    },
    LIGHT_CPI_SIGNER,
};

#[inline(never)]
pub(crate) fn new_light_system_cpi(
    proof: light_compressed_account::instruction_data::compressed_proof::ValidityProof,
) -> LightSystemProgramCpi {
    LightSystemProgramCpi::new(
        LIGHT_CPI_SIGNER.program_id.into(),
        LIGHT_CPI_SIGNER.bump,
        proof.0,
    )
}

#[derive(Clone, Debug)]
pub struct MarketPageLeafUpdate<'a> {
    pub meta: &'a CompressedAccountMeta,
    pub page: &'a CompressedMarketPageLeaf,
}

#[derive(Clone, Debug)]
pub struct MarketPageLeafCreate<'a> {
    pub output: &'a CompressionOutput,
    pub page: &'a CompressedMarketPageLeaf,
}

#[derive(Clone, Debug)]
pub struct SettlementLeafUpdate<'a> {
    pub meta: &'a CompressedAccountMeta,
    pub settlement: &'a CompressedSettlementLeaf,
}

#[derive(Clone, Debug)]
pub struct SettlementLeafCreate<'a> {
    pub output: &'a CompressionOutput,
    pub settlement: &'a CompressedSettlementLeaf,
}

pub(crate) struct CompressedStateLeafReadOnly<'a> {
    pub(crate) meta: &'a CompressedAccountMetaReadOnly,
    pub(crate) leaf: &'a CompressedAmebaStateLeaf,
}

pub(crate) struct CompressedStateLeafUpdate<'a> {
    pub(crate) meta: &'a CompressedAccountMeta,
    pub(crate) old_leaf: &'a CompressedAmebaStateLeaf,
    pub(crate) new_leaf: CompressedAmebaStateLeaf,
}

pub(crate) struct CompressedStateLeafClose<'a> {
    pub(crate) meta: &'a CompressedAccountMeta,
    pub(crate) old_leaf: &'a CompressedAmebaStateLeaf,
}

pub(crate) struct CompressedStateLeafCreate<'a> {
    pub(crate) output: &'a CompressionOutput,
    pub(crate) leaf: CompressedAmebaStateLeaf,
}

fn packed_light_accounts<'a, 'info>(
    remaining_accounts: &'a [AccountInfo<'info>],
) -> Result<&'a [AccountInfo<'info>], ProgramError> {
    remaining_accounts
        .get(6..)
        .ok_or(VaultError::InvalidRemainingAccounts.into())
}

fn packed_tree_pubkey(
    remaining_accounts: &[AccountInfo<'_>],
    packed_index: u8,
) -> Result<Pubkey, ProgramError> {
    packed_light_accounts(remaining_accounts)?
        .get(packed_index as usize)
        .map(|account| *account.key)
        .ok_or(VaultError::InvalidRemainingAccounts.into())
}

#[inline(always)]
fn serialize_leaf<T: BorshSerialize>(leaf: &T) -> Result<Vec<u8>, ProgramError> {
    leaf.try_to_vec().map_err(|_| ProgramError::Custom(16016))
}

#[inline(never)]
/// Light's canonical BN254-field-compatible data hash for compressed account
/// bodies and decompressed-PDA placeholders.
pub(crate) fn hash_leaf_data(data: &[u8]) -> Result<[u8; 32], ProgramError> {
    // `light_hasher::Sha256::hash` routes the Solana SHA-256 syscall through a
    // function pointer.  SBF cannot relocate that syscall pointer, so otherwise
    // valid instructions can compile to `callx 0` and abort at runtime.  Call
    // Solana's canonical hash syscall directly and retain Light's SHA256BE
    // field-size normalization below; the resulting bytes are identical.
    let mut hash = hash(data).to_bytes();
    hash[0] = 0;
    Ok(hash)
}

pub(crate) fn init_leaf_account_info(
    address: [u8; 32],
    output_state_tree_index: u8,
    discriminator: [u8; 8],
    data: Vec<u8>,
) -> Result<CompressedAccountInfo, ProgramError> {
    let data_hash = hash_leaf_data(&data)?;
    Ok(CompressedAccountInfo {
        address: Some(address),
        input: None,
        output: Some(OutAccountInfo {
            discriminator,
            data_hash,
            output_merkle_tree_index: output_state_tree_index,
            lamports: 0,
            data,
        }),
    })
}

#[inline(never)]
fn write_leaf_account_info(
    meta: &CompressedAccountMeta,
    discriminator: [u8; 8],
    old_data: &[u8],
    new_data: Option<Vec<u8>>,
) -> Result<CompressedAccountInfo, ProgramError> {
    let input = InAccountInfo {
        discriminator,
        data_hash: hash_leaf_data(old_data)?,
        merkle_context: meta.tree_info.into(),
        root_index: if meta.tree_info.prove_by_index {
            0
        } else {
            meta.tree_info.root_index
        },
        lamports: 0,
    };
    let output = if let Some(data) = new_data {
        OutAccountInfo {
            discriminator,
            data_hash: hash_leaf_data(&data)?,
            output_merkle_tree_index: meta.output_state_tree_index,
            lamports: 0,
            data,
        }
    } else {
        OutAccountInfo {
            output_merkle_tree_index: meta.output_state_tree_index,
            ..OutAccountInfo::default()
        }
    };
    Ok(CompressedAccountInfo {
        address: Some(meta.address),
        input: Some(input),
        output: Some(output),
    })
}

fn read_only_leaf_account_info(
    program_id: &Pubkey,
    meta: &CompressedAccountMetaReadOnly,
    discriminator: [u8; 8],
    data: &[u8],
    packed_accounts: &[AccountInfo<'_>],
) -> Result<PackedReadOnlyCompressedAccount, ProgramError> {
    let data_hash = hash_leaf_data(data)?;
    let merkle_tree_pubkey = packed_accounts
        .get(meta.tree_info.merkle_tree_pubkey_index as usize)
        .ok_or(ProgramError::Custom(16037))?
        .key
        .to_bytes();
    let mut leaf_index = [0u8; 32];
    leaf_index[28..].copy_from_slice(&meta.tree_info.leaf_index.to_be_bytes());
    let mut padded_discriminator = [0u8; 32];
    padded_discriminator[23] = 2;
    padded_discriminator[24..].copy_from_slice(&discriminator);
    let account_hash = Poseidon::hashv(&[
        &hash_to_bn254_field_size_be(program_id.as_ref()),
        &leaf_index,
        &hash_to_bn254_field_size_be(&merkle_tree_pubkey),
        &meta.address,
        &padded_discriminator,
        &data_hash,
    ])
    .map_err(|error| ProgramError::Custom(error.into()))?;
    Ok(PackedReadOnlyCompressedAccount {
        root_index: if meta.tree_info.prove_by_index {
            0
        } else {
            meta.tree_info.root_index
        },
        merkle_context: meta.tree_info.into(),
        account_hash,
    })
}

#[inline(always)]
fn push_vec_len(output: &mut Vec<u8>, len: usize) {
    output.extend_from_slice(&(len as u32).to_le_bytes());
}

#[inline(always)]
fn push_packed_merkle_context(output: &mut Vec<u8>, context: &PackedMerkleContext) {
    output.push(context.merkle_tree_pubkey_index);
    output.push(context.queue_pubkey_index);
    output.extend_from_slice(&context.leaf_index.to_le_bytes());
    output.push(context.prove_by_index as u8);
}

fn encode_light_cpi(instruction: &LightSystemProgramCpi) -> Vec<u8> {
    let mut output = Vec::new();
    output.extend_from_slice(instruction.discriminator());
    output.push(instruction.mode);
    output.push(instruction.bump);
    output.extend_from_slice(instruction.invoking_program_id.array_ref());
    output.extend_from_slice(&instruction.compress_or_decompress_lamports.to_le_bytes());
    output.push(instruction.is_compress as u8);
    output.push(instruction.with_cpi_context as u8);
    output.push(instruction.with_transaction_hash as u8);
    output.push(instruction.cpi_context.set_context as u8);
    output.push(instruction.cpi_context.first_set_context as u8);
    output.push(instruction.cpi_context.cpi_context_account_index);

    if let Some(proof) = &instruction.proof {
        output.push(1);
        output.extend_from_slice(&proof.a);
        output.extend_from_slice(&proof.b);
        output.extend_from_slice(&proof.c);
    } else {
        output.push(0);
    }

    push_vec_len(&mut output, instruction.new_address_params.len());
    for address in &instruction.new_address_params {
        output.extend_from_slice(&address.seed);
        output.push(address.address_queue_account_index);
        output.push(address.address_merkle_tree_account_index);
        output.extend_from_slice(&address.address_merkle_tree_root_index.to_le_bytes());
        output.push(address.assigned_to_account as u8);
        output.push(address.assigned_account_index);
    }

    push_vec_len(&mut output, instruction.account_infos.len());
    for account in &instruction.account_infos {
        if let Some(address) = &account.address {
            output.push(1);
            output.extend_from_slice(address);
        } else {
            output.push(0);
        }
        if let Some(input) = &account.input {
            output.push(1);
            output.extend_from_slice(&input.discriminator);
            output.extend_from_slice(&input.data_hash);
            push_packed_merkle_context(&mut output, &input.merkle_context);
            output.extend_from_slice(&input.root_index.to_le_bytes());
            output.extend_from_slice(&input.lamports.to_le_bytes());
        } else {
            output.push(0);
        }
        if let Some(account_output) = &account.output {
            output.push(1);
            output.extend_from_slice(&account_output.discriminator);
            output.extend_from_slice(&account_output.data_hash);
            output.push(account_output.output_merkle_tree_index);
            output.extend_from_slice(&account_output.lamports.to_le_bytes());
            push_vec_len(&mut output, account_output.data.len());
            output.extend_from_slice(&account_output.data);
        } else {
            output.push(0);
        }
    }

    push_vec_len(&mut output, instruction.read_only_addresses.len());
    for address in &instruction.read_only_addresses {
        output.extend_from_slice(&address.address);
        output.extend_from_slice(&address.address_merkle_tree_root_index.to_le_bytes());
        output.push(address.address_merkle_tree_account_index);
    }

    push_vec_len(&mut output, instruction.read_only_accounts.len());
    for account in &instruction.read_only_accounts {
        output.extend_from_slice(&account.account_hash);
        push_packed_merkle_context(&mut output, &account.merkle_context);
        output.extend_from_slice(&account.root_index.to_le_bytes());
    }
    output
}

pub fn derive_compressed_state_leaf_address(
    program_id: &Pubkey,
    tree_pubkey: &Pubkey,
    domain: CompressedStateDomain,
    canonical_pda: &Pubkey,
) -> ([u8; 32], AddressSeed) {
    let domain_seed = [domain.address_tag()];
    derive_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            COMPRESSED_STATE_LEAF_ADDRESS_SEED,
            &domain_seed,
            canonical_pda.as_ref(),
        ],
        tree_pubkey,
        program_id,
    )
}

pub fn derive_market_page_leaf_address(
    program_id: &Pubkey,
    tree_pubkey: &Pubkey,
    page: &CompressedMarketPageLeaf,
) -> ([u8; 32], AddressSeed) {
    derive_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            MARKET_PAGE_LEAF_ADDRESS_SEED,
            page.item_id.as_bytes(),
            &page.underlying_id,
        ],
        tree_pubkey,
        program_id,
    )
}

pub fn derive_settlement_leaf_address(
    program_id: &Pubkey,
    tree_pubkey: &Pubkey,
    settlement: &CompressedSettlementLeaf,
) -> ([u8; 32], AddressSeed) {
    derive_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            SETTLEMENT_LEAF_ADDRESS_SEED,
            settlement.oracle_month.as_ref(),
            settlement.item_id.as_bytes(),
            &settlement.underlying_id,
            settlement.expiry_id.as_bytes(),
        ],
        tree_pubkey,
        program_id,
    )
}

enum DisplayLeafMutations<'a, 'b> {
    Market {
        updates: &'a [MarketPageLeafUpdate<'b>],
        creates: &'a [MarketPageLeafCreate<'b>],
    },
    Settlement {
        updates: &'a [SettlementLeafUpdate<'b>],
        creates: &'a [SettlementLeafCreate<'b>],
    },
}

#[inline(never)]
pub(crate) fn invoke_light_cpi<'a>(
    instruction: LightSystemProgramCpi,
    fee_payer: &AccountInfo<'a>,
    remaining_accounts: &[AccountInfo<'a>],
) -> Result<(), ProgramError> {
    let data = encode_light_cpi(&instruction);
    let fixed = remaining_accounts
        .get(1..6)
        .ok_or(ProgramError::Custom(16031))?;

    let mut account_infos = Vec::with_capacity(remaining_accounts.len());
    account_infos.push(fee_payer.clone());
    account_infos.extend_from_slice(&remaining_accounts[1..]);

    let mut account_metas = Vec::with_capacity(remaining_accounts.len());
    account_metas.push(AccountMeta {
        pubkey: *fee_payer.key,
        is_signer: true,
        is_writable: true,
    });
    account_metas.push(AccountMeta {
        pubkey: *fixed[0].key,
        is_signer: true,
        is_writable: false,
    });
    for account in &fixed[1..] {
        account_metas.push(AccountMeta {
            pubkey: *account.key,
            is_signer: false,
            is_writable: false,
        });
    }
    for account in &remaining_accounts[6..] {
        account_metas.push(AccountMeta {
            pubkey: *account.key,
            is_signer: account.is_signer,
            is_writable: account.is_writable,
        });
    }

    let instruction = Instruction {
        program_id: LIGHT_SYSTEM_PROGRAM_ID.into(),
        accounts: account_metas,
        data,
    };
    let signer_seeds = [CPI_AUTHORITY_PDA_SEED, &[LIGHT_CPI_SIGNER.bump]];
    invoke_signed(&instruction, &account_infos, &[signer_seeds.as_slice()])
}

pub fn apply_market_page_leaf_mutations<'a>(
    program_id: &Pubkey,
    fee_payer: &AccountInfo<'a>,
    remaining_accounts: &[AccountInfo<'a>],
    proof: &ValidityProof,
    updates: &[MarketPageLeafUpdate<'_>],
    creates: &[MarketPageLeafCreate<'_>],
) -> Result<(), ProgramError> {
    apply_display_leaf_mutations(
        program_id,
        fee_payer,
        remaining_accounts,
        proof,
        DisplayLeafMutations::Market { updates, creates },
    )
}

pub fn apply_settlement_leaf_mutations<'a>(
    program_id: &Pubkey,
    fee_payer: &AccountInfo<'a>,
    remaining_accounts: &[AccountInfo<'a>],
    proof: &ValidityProof,
    updates: &[SettlementLeafUpdate<'_>],
    creates: &[SettlementLeafCreate<'_>],
) -> Result<(), ProgramError> {
    apply_display_leaf_mutations(
        program_id,
        fee_payer,
        remaining_accounts,
        proof,
        DisplayLeafMutations::Settlement { updates, creates },
    )
}

#[inline(never)]
fn apply_display_leaf_mutations<'a>(
    program_id: &Pubkey,
    fee_payer: &AccountInfo<'a>,
    remaining_accounts: &[AccountInfo<'a>],
    proof: &ValidityProof,
    mutations: DisplayLeafMutations<'_, '_>,
) -> Result<(), ProgramError> {
    let empty = match &mutations {
        DisplayLeafMutations::Market { updates, creates } => {
            updates.is_empty() && creates.is_empty()
        }
        DisplayLeafMutations::Settlement { updates, creates } => {
            updates.is_empty() && creates.is_empty()
        }
    };
    if empty {
        return Ok(());
    }

    let mut instruction = new_light_system_cpi((*proof).into());

    match mutations {
        DisplayLeafMutations::Market { updates, creates } => {
            for update in updates {
                let data = serialize_leaf(update.page)?;
                instruction.account_infos.push(write_leaf_account_info(
                    update.meta,
                    CompressedMarketPageLeaf::LIGHT_DISCRIMINATOR,
                    &data,
                    Some(data.clone()),
                )?);
            }
            let assigned_start = updates.len();
            for (offset, create) in creates.iter().enumerate() {
                let tree_pubkey = packed_tree_pubkey(
                    remaining_accounts,
                    create
                        .output
                        .address_tree_info
                        .address_merkle_tree_pubkey_index,
                )?;
                let assigned_index = assigned_start
                    .checked_add(offset)
                    .ok_or(VaultError::ArithmeticOverflow)?
                    as u8;
                let (address, seed) =
                    derive_market_page_leaf_address(program_id, &tree_pubkey, create.page);
                let data = serialize_leaf(create.page)?;
                instruction.account_infos.push(init_leaf_account_info(
                    address,
                    create.output.output_state_tree_index,
                    CompressedMarketPageLeaf::LIGHT_DISCRIMINATOR,
                    data,
                )?);
                instruction.new_address_params.push(
                    create
                        .output
                        .address_tree_info
                        .into_new_address_params_assigned_packed(seed, Some(assigned_index)),
                );
            }
        }
        DisplayLeafMutations::Settlement { updates, creates } => {
            for update in updates {
                let data = serialize_leaf(update.settlement)?;
                instruction.account_infos.push(write_leaf_account_info(
                    update.meta,
                    CompressedSettlementLeaf::LIGHT_DISCRIMINATOR,
                    &data,
                    Some(data.clone()),
                )?);
            }
            let assigned_start = updates.len();
            for (offset, create) in creates.iter().enumerate() {
                let tree_pubkey = packed_tree_pubkey(
                    remaining_accounts,
                    create
                        .output
                        .address_tree_info
                        .address_merkle_tree_pubkey_index,
                )?;
                let assigned_index = assigned_start
                    .checked_add(offset)
                    .ok_or(VaultError::ArithmeticOverflow)?
                    as u8;
                let (address, seed) =
                    derive_settlement_leaf_address(program_id, &tree_pubkey, create.settlement);
                let data = serialize_leaf(create.settlement)?;
                instruction.account_infos.push(init_leaf_account_info(
                    address,
                    create.output.output_state_tree_index,
                    CompressedSettlementLeaf::LIGHT_DISCRIMINATOR,
                    data,
                )?);
                instruction.new_address_params.push(
                    create
                        .output
                        .address_tree_info
                        .into_new_address_params_assigned_packed(seed, Some(assigned_index)),
                );
            }
        }
    }

    invoke_light_cpi(instruction, fee_payer, remaining_accounts)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_compressed_state_leaf_mutations<'a>(
    program_id: &Pubkey,
    fee_payer: &AccountInfo<'a>,
    remaining_accounts: &[AccountInfo<'a>],
    proof: &ValidityProof,
    read_only: &[CompressedStateLeafReadOnly<'_>],
    updates: &[CompressedStateLeafUpdate<'_>],
    closes: &[CompressedStateLeafClose<'_>],
    creates: &[CompressedStateLeafCreate<'_>],
) -> Result<(), ProgramError> {
    if read_only.is_empty() && updates.is_empty() && closes.is_empty() && creates.is_empty() {
        return Err(VaultError::InvalidCompressionWitness.into());
    }

    let packed_accounts = packed_light_accounts(remaining_accounts)?;
    let mut instruction = new_light_system_cpi((*proof).into());

    for witness in read_only {
        validate_compressed_state_leaf(witness.leaf)?;
        let expected = derive_compressed_state_leaf_address(
            program_id,
            &LIGHT_DEFAULT_ADDRESS_TREE_V2,
            witness.leaf.domain,
            &witness.leaf.canonical_pda,
        )
        .0;
        if witness.meta.address != expected {
            return Err(VaultError::InvalidCompressionWitness.into());
        }
        let data = witness
            .leaf
            .try_to_vec()
            .map_err(|_| ProgramError::Custom(16016))?;
        let account = read_only_leaf_account_info(
            program_id,
            witness.meta,
            CompressedAmebaStateLeaf::LIGHT_DISCRIMINATOR,
            &data,
            packed_accounts,
        )?;
        instruction.read_only_accounts.push(account);
    }

    for update in updates {
        validate_compressed_state_leaf(update.old_leaf)?;
        validate_compressed_state_leaf(&update.new_leaf)?;
        let expected_revision = update
            .old_leaf
            .revision
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if update.old_leaf.domain != update.new_leaf.domain
            || update.old_leaf.canonical_pda != update.new_leaf.canonical_pda
            || update.new_leaf.revision != expected_revision
        {
            return Err(VaultError::InvalidCompressionWitness.into());
        }
        let expected = derive_compressed_state_leaf_address(
            program_id,
            &LIGHT_DEFAULT_ADDRESS_TREE_V2,
            update.old_leaf.domain,
            &update.old_leaf.canonical_pda,
        )
        .0;
        if update.meta.address != expected {
            return Err(VaultError::InvalidCompressionWitness.into());
        }
        let old_data = serialize_leaf(update.old_leaf)?;
        let new_data = serialize_leaf(&update.new_leaf)?;
        instruction.account_infos.push(write_leaf_account_info(
            update.meta,
            CompressedAmebaStateLeaf::LIGHT_DISCRIMINATOR,
            &old_data,
            Some(new_data),
        )?);
    }

    for close in closes {
        validate_compressed_state_leaf(close.old_leaf)?;
        let expected = derive_compressed_state_leaf_address(
            program_id,
            &LIGHT_DEFAULT_ADDRESS_TREE_V2,
            close.old_leaf.domain,
            &close.old_leaf.canonical_pda,
        )
        .0;
        if close.meta.address != expected {
            return Err(VaultError::InvalidCompressionWitness.into());
        }
        let old_data = serialize_leaf(close.old_leaf)?;
        instruction.account_infos.push(write_leaf_account_info(
            close.meta,
            CompressedAmebaStateLeaf::LIGHT_DISCRIMINATOR,
            &old_data,
            None,
        )?);
    }

    let assigned_start = updates
        .len()
        .checked_add(closes.len())
        .ok_or(VaultError::ArithmeticOverflow)?;
    for (offset, create) in creates.iter().enumerate() {
        validate_compressed_state_leaf(&create.leaf)?;
        if create.leaf.revision != 0 {
            return Err(VaultError::InvalidCompressionWitness.into());
        }
        let tree_pubkey = packed_tree_pubkey(
            remaining_accounts,
            create
                .output
                .address_tree_info
                .address_merkle_tree_pubkey_index,
        )?;
        if tree_pubkey != LIGHT_DEFAULT_ADDRESS_TREE_V2 {
            return Err(VaultError::InvalidCompressionWitness.into());
        }
        let (address, seed) = derive_compressed_state_leaf_address(
            program_id,
            &tree_pubkey,
            create.leaf.domain,
            &create.leaf.canonical_pda,
        );
        let assigned_index = assigned_start
            .checked_add(offset)
            .ok_or(VaultError::ArithmeticOverflow)? as u8;
        let data = serialize_leaf(&create.leaf)?;
        instruction.account_infos.push(init_leaf_account_info(
            address,
            create.output.output_state_tree_index,
            CompressedAmebaStateLeaf::LIGHT_DISCRIMINATOR,
            data,
        )?);
        instruction.new_address_params.push(
            create
                .output
                .address_tree_info
                .into_new_address_params_assigned_packed(seed, Some(assigned_index)),
        );
    }

    invoke_light_cpi(instruction, fee_payer, remaining_accounts)
}

fn validate_compressed_state_leaf(leaf: &CompressedAmebaStateLeaf) -> Result<(), ProgramError> {
    if !leaf.has_canonical_envelope() {
        return Err(VaultError::InvalidCompressionWitness.into());
    }
    Ok(())
}
