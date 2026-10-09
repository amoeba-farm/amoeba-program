//! Permissionless same-owner WriterCash leaf consolidation. Neither ledger nor
//! financial claims change; donated surplus remains with its original owner.
use super::*;
use crate::compact_error::CompactAccountInfo;
use crate::{
    compressed_custody as custody, multi_order as mo, regular_compressed_transfer as transfer,
};
use solana_program::instruction::AccountMeta;
fn invalid() -> ProgramError {
    VaultError::InvalidAccountList.into()
}
pub(super) fn consolidate(
    program: &Pubkey,
    a: &[AccountInfo],
    p: mo::ConsolidateWriterCash,
) -> ProgramResult {
    // payer, canonical cash sidecar, quote mint, Light Token, CPI authority,
    // System, Light System, registered, compression authority/program, Merkle.
    if a.len() != 10 + usize::from(p.merkle_accounts)
        || p.merkle_accounts < 2
        || !a[0].is_signer
        || !a[0].is_writable
        || a[0].executable
        || a[1].is_writable
        || a[1].is_signer
        || a[1].executable
        || a[1].owner != program
        || !crate::light_token_instruction::is_program(a[3].key)
        || !crate::light_token_instruction::is_cpi_authority(a[4].key)
        || !crate::is_system_program(a[5].key)
        || !crate::light_token_instruction::is_light_system_program(a[6].key)
        || !crate::light_token_instruction::is_registered_program(a[7].key)
        || !crate::light_token_instruction::is_compression_authority(a[8].key)
        || !crate::light_token_instruction::is_compression_program(a[9].key)
    {
        return Err(invalid());
    }
    let data = a[1].try_data()?;
    if data.len() != custody::CompressedCustodyV1::ACCOUNT_LEN
        || data[..8] != custody::COMPRESSED_CUSTODY_DISCRIMINATOR
        || solana_program::hash::hash(&data).to_bytes() != p.expected_hash
    {
        return Err(invalid());
    }
    let record = custody::CompressedCustodyV1::try_from_slice(&data[8..]).map_err(|_| invalid())?;
    drop(data);
    custody::load_observation(
        program,
        Some(&a[1]),
        custody::CustodyKind::WriterCash,
        &record.parent,
        &Pubkey::default(),
        a[2].key,
    )?;
    if record.option_atoms != 0 {
        return Err(invalid());
    }
    let merkle = &a[10..];
    if merkle.iter().enumerate().any(|(i, v)| {
        !v.is_writable || v.is_signer || v.executable || merkle[..i].iter().any(|q| q.key == v.key)
    }) {
        return Err(invalid());
    }
    let sum = validate_batch(&p.batch, merkle.len())?;
    // Transfer2's seven fixed roles, followed by tree/queue-first packed roles.
    let mut roles = vec![
        (6, false),
        (0, true),
        (4, false),
        (7, false),
        (8, false),
        (9, false),
        (5, false),
    ];
    roles.extend((10..a.len()).map(|i| (i, false)));
    let owner = u8::try_from(merkle.len()).map_err(|_| invalid())?;
    let mint = owner.checked_add(1).ok_or_else(invalid)?;
    roles.push((1, true));
    roles.push((2, false));
    let leaves = p
        .batch
        .inputs
        .iter()
        .map(|i| transfer::InputLeaf {
            owner,
            amount: i.amount,
            has_delegate: false,
            delegate: 0,
            mint,
            tree: i.witness.tree_index,
            queue: i.witness.queue_index,
            leaf_index: i.witness.leaf_index,
            root_index: i.witness.root_index,
            prove_by_index: i.witness.prove_by_index,
        })
        .collect::<Vec<_>>();
    let outputs = [transfer::OutputLeaf {
        owner,
        amount: sum,
        has_delegate: false,
        delegate: 0,
        mint,
    }];
    let metas = roles
        .iter()
        .map(|(i, signer)| AccountMeta {
            pubkey: *a[*i].key,
            is_signer: *signer,
            is_writable: a[*i].is_writable,
        })
        .collect();
    let ix = transfer::instruction(
        *a[3].key,
        metas,
        p.batch.output_queue,
        p.batch.proof,
        &leaves,
        &outputs,
    )?;
    let mut infos = roles.iter().map(|(i, _)| a[*i].clone()).collect::<Vec<_>>();
    infos.push(a[3].clone());
    let kind = [custody::CustodyKind::WriterCash as u8];
    let bump = [record.bump];
    invoke_signed(
        &ix,
        &infos,
        &[&[
            CURRENT_STATE_NAMESPACE_SEED,
            custody::COMPRESSED_CUSTODY_SEED,
            &kind,
            record.parent.as_ref(),
            &bump,
        ]],
    )
}
fn validate_batch(batch: &mo::Batch, merkle_count: usize) -> Result<u64, ProgramError> {
    if batch.inputs.len() < 2
        || batch.output_tree == batch.output_queue
        || usize::from(batch.output_tree) >= merkle_count
        || usize::from(batch.output_queue) >= merkle_count
    {
        return Err(invalid());
    }
    let mut seen = vec![];
    let mut sum = 0u64;
    for i in &batch.inputs {
        let w = &i.witness;
        if i.asset != 0
            || i.party != 0
            || i.delegated
            || i.amount == 0
            || w.tree_index == w.queue_index
            || usize::from(w.tree_index) >= merkle_count
            || usize::from(w.queue_index) >= merkle_count
            || batch.proof.is_none() && !w.prove_by_index
            || seen.contains(&(w.tree_index, w.queue_index, w.leaf_index))
        {
            return Err(invalid());
        }
        seen.push((w.tree_index, w.queue_index, w.leaf_index));
        sum = sum.checked_add(i.amount).ok_or_else(invalid)?;
    }
    Ok(sum)
}
