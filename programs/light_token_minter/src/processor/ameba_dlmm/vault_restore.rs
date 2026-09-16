//! Restore only the pool's canonical vault from a Light token proof. The PDA
//! signs solely a fixed full-decompression CPI; callers cannot select a payee.
use super::*;
use crate::{fixed_codec::CheckedCursor, instruction::deserialize_validity_proof_cursor};
use solana_program::instruction::{AccountMeta, Instruction};

#[inline(never)]
pub(in crate::processor) fn process_restore_vault(
    program_id: &Pubkey,
    a: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    // payer, pool, mint, vault, token config/sponsor, System, token program,
    // Light System, token CPI authority, registered program, compression authority,
    // compression program, state tree, output queue.
    if a.len() != 15
        || !a[0].is_signer
        || !a[0].is_writable
        || !a[3].is_writable
        || a[3].is_signer
        || !a[1].is_writable
        || a[1].is_signer
        || *a[4].key != crate::constants::LIGHT_TOKEN_COMPRESSIBLE_CONFIG
        || *a[5].key != crate::constants::LIGHT_TOKEN_RENT_SPONSOR
        || *a[6].key != system_program::id()
        || *a[7].key != light_token_program_id()
        || *a[8].key != Pubkey::new_from_array(light_sdk::constants::LIGHT_SYSTEM_PROGRAM_ID)
        || *a[9].key != cpi_authority()
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut cursor = CheckedCursor::new(payload);
    let amount = cursor.u64();
    let leaf_index = cursor.u32();
    let root_index = cursor.u16();
    let prove = cursor.u8();
    let proof = deserialize_validity_proof_cursor(&mut cursor);
    cursor
        .finish_exact()
        .map_err(|_| VaultError::InvalidInstructionData)?;
    if prove > 1 || (prove == 1 && root_index != 0) || (prove == 0 && proof.0.is_none()) {
        return Err(VaultError::InvalidCompressionWitness.into());
    }
    let pool = load_pool(program_id, &a[1])?;
    let expected = if *a[2].key == pool.option_mint {
        pool.option_vault
    } else if *a[2].key == pool.quote_mint {
        pool.quote_vault
    } else {
        return Err(VaultError::InvalidMint.into());
    };
    let (vault, bump) = derive_ameba_dlmm_vault_pda(program_id, a[1].key, a[2].key);
    if vault != expected || vault != *a[3].key {
        return Err(VaultError::InvalidAmoebaDlmmVault.into());
    }
    let authority = derive_ameba_dlmm_authority_pda(program_id, a[1].key).0;
    create_light_vault(
        program_id, &a[0], a[1].key, &a[2], &authority, &a[3], &a[4], &a[5], &a[6], &a[7],
    )?;
    let before = load_canonical_light_token_account(&a[3], &authority, a[2].key)?;
    if before.amount != 0 {
        return Err(VaultError::InvalidAmoebaDlmmVault.into());
    }

    // Exact Light 0.23 Transfer2 encoding. Packed keys: tree, queue, vault, mint.
    // One full input, one decompression into the same canonical hot vault, no
    // output compressed tokens, delegates, fees, lamports, or optional TLVs.
    let mut data = vec![101, 0, 0, 0, 0, 0, 255, 255, 0, 1];
    data.extend_from_slice(&1u32.to_le_bytes());
    data.push(1);
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&[3, 2, 0, 0, 0, 0, 0]);
    if let Some(p) = proof.0 {
        data.push(1);
        data.extend_from_slice(&p.a);
        data.extend_from_slice(&p.b);
        data.extend_from_slice(&p.c);
    } else {
        data.push(0);
    }
    data.extend_from_slice(&1u32.to_le_bytes());
    data.push(2);
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&[0, 0, 3, 3, 0, 1]);
    data.extend_from_slice(&leaf_index.to_le_bytes());
    data.push(prove);
    data.extend_from_slice(&root_index.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&[0, 0, 0, 0]);
    let indices = [8usize, 0, 9, 10, 11, 12, 6, 13, 14, 3, 2];
    let metas = indices
        .iter()
        .map(|i| AccountMeta {
            pubkey: *a[*i].key,
            is_signer: matches!(*i, 0 | 3),
            is_writable: matches!(*i, 0 | 3 | 13 | 14),
        })
        .collect();
    let infos = indices.iter().map(|i| a[*i].clone()).collect::<Vec<_>>();
    let ix = Instruction {
        program_id: light_token_program_id(),
        accounts: metas,
        data,
    };
    let bump = [bump];
    let seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        AMOEBA_DLMM_VAULT_PDA_SEED,
        a[1].key.as_ref(),
        a[2].key.as_ref(),
        &bump,
    ];
    invoke_signed(&ix, &infos, &[seeds])?;
    let after = load_canonical_light_token_account(&a[3], &authority, a[2].key)?;
    if after.amount != amount {
        return Err(VaultError::AmoebaDlmmInvariantViolation.into());
    }
    Ok(())
}
