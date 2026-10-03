//! Fixed regular v3 Transfer2 encoding for authenticated leaves and optional
//! compression of hot inputs. No decompression, top-up, or TLV is represented.
use crate::ProgramError;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

const TRANSFER2: u8 = 101;
const REGULAR_V3: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputLeaf {
    pub owner: u8,
    pub amount: u64,
    pub has_delegate: bool,
    pub delegate: u8,
    pub mint: u8,
    pub tree: u8,
    pub queue: u8,
    pub leaf_index: u32,
    pub prove_by_index: bool,
    pub root_index: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputLeaf {
    pub owner: u8,
    pub amount: u64,
    pub has_delegate: bool,
    pub delegate: u8,
    pub mint: u8,
}

/// Canonical SPL/Light hot input that is compressed inside the same Transfer2
/// as the authenticated regular leaves. No decompression mode is exposed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotCompression {
    pub amount: u64,
    pub mint: u8,
    pub source: u8,
    pub authority: u8,
    pub pool_account_index: u8,
    pub pool_index: u8,
    pub bump: u8,
    pub decimals: u8,
}

/// Build one Light Transfer2 with both regular compressed inputs and hot token
/// compressions. The business handler must authenticate every packed account.
pub fn instruction_with_compressions(
    program: Pubkey,
    accounts: Vec<AccountMeta>,
    output_queue: u8,
    proof: Option<[u8; 128]>,
    inputs: &[InputLeaf],
    compressions: &[HotCompression],
    outputs: &[OutputLeaf],
) -> Result<Instruction, ProgramError> {
    if compressions.is_empty() {
        return instruction(program, accounts, output_queue, proof, inputs, outputs);
    }
    let count = accounts
        .len()
        .checked_sub(7)
        .ok_or(ProgramError::InvalidInstructionData)?;
    if count == 0
        || count > 248
        || usize::from(output_queue) >= count
        || (inputs.is_empty() && compressions.is_empty())
        || outputs.is_empty()
        || inputs.iter().any(|x| {
            x.amount == 0
                || [x.owner, x.mint, x.tree, x.queue]
                    .iter()
                    .any(|index| usize::from(*index) >= count)
                || x.has_delegate && usize::from(x.delegate) >= count
        })
        || compressions.iter().any(|x| {
            x.amount == 0
                || [x.mint, x.source, x.authority, x.pool_account_index]
                    .iter()
                    .any(|index| usize::from(*index) >= count)
        })
        || outputs.iter().any(|x| {
            x.amount == 0
                || [x.owner, x.mint]
                    .iter()
                    .any(|index| usize::from(*index) >= count)
                || x.has_delegate && usize::from(x.delegate) >= count
        })
        || (proof.is_none() && inputs.iter().any(|x| !x.prove_by_index))
    {
        return Err(ProgramError::InvalidInstructionData);
    }
    for mint in inputs
        .iter()
        .map(|x| x.mint)
        .chain(compressions.iter().map(|x| x.mint))
        .chain(outputs.iter().map(|x| x.mint))
    {
        let debit: u128 = inputs
            .iter()
            .filter(|x| x.mint == mint)
            .map(|x| u128::from(x.amount))
            .sum::<u128>()
            + compressions
                .iter()
                .filter(|x| x.mint == mint)
                .map(|x| u128::from(x.amount))
                .sum::<u128>();
        let credit: u128 = outputs
            .iter()
            .filter(|x| x.mint == mint)
            .map(|x| u128::from(x.amount))
            .sum();
        if debit != credit {
            return Err(ProgramError::InvalidInstructionData);
        }
    }
    let mut data = Vec::with_capacity(
        32 + proof.map_or(0, |_| 128)
            + 16 * compressions.len()
            + 22 * inputs.len()
            + 13 * outputs.len(),
    );
    data.extend_from_slice(&[TRANSFER2, 0, 0, 0, 0, output_queue, 0, 0, 0, 1]);
    data.extend_from_slice(&(compressions.len() as u32).to_le_bytes());
    for x in compressions {
        data.push(0); // CompressionMode::Compress
        data.extend_from_slice(&x.amount.to_le_bytes());
        data.extend_from_slice(&[
            x.mint,
            x.source,
            x.authority,
            x.pool_account_index,
            x.pool_index,
            x.bump,
            x.decimals,
        ]);
    }
    if let Some(proof) = proof {
        data.push(1);
        data.extend_from_slice(&proof);
    } else {
        data.push(0);
    }
    data.extend_from_slice(&(inputs.len() as u32).to_le_bytes());
    for leaf in inputs {
        push_input(&mut data, leaf);
    }
    data.extend_from_slice(&(outputs.len() as u32).to_le_bytes());
    for leaf in outputs {
        push_output(&mut data, leaf);
    }
    data.extend_from_slice(&[0, 0, 0, 0]);
    Ok(Instruction {
        program_id: program,
        accounts,
        data,
    })
}

fn push_input(data: &mut Vec<u8>, leaf: &InputLeaf) {
    data.push(leaf.owner);
    data.extend_from_slice(&leaf.amount.to_le_bytes());
    data.extend_from_slice(&[
        u8::from(leaf.has_delegate),
        leaf.delegate,
        leaf.mint,
        REGULAR_V3,
        leaf.tree,
        leaf.queue,
    ]);
    data.extend_from_slice(&leaf.leaf_index.to_le_bytes());
    data.push(u8::from(leaf.prove_by_index));
    data.extend_from_slice(&leaf.root_index.to_le_bytes());
}

fn push_output(data: &mut Vec<u8>, leaf: &OutputLeaf) {
    data.push(leaf.owner);
    data.extend_from_slice(&leaf.amount.to_le_bytes());
    data.extend_from_slice(&[
        u8::from(leaf.has_delegate),
        leaf.delegate,
        leaf.mint,
        REGULAR_V3,
    ]);
}

/// `accounts` follow Light's seven fixed metas. Packed indices refer to the
/// remaining metas and must be validated by the business handler against its
/// canonical PDA, owner, mint, tree, queue, and delegate identities.
pub fn instruction(
    program: Pubkey,
    accounts: Vec<AccountMeta>,
    output_queue: u8,
    proof: Option<[u8; 128]>,
    inputs: &[InputLeaf],
    outputs: &[OutputLeaf],
) -> Result<Instruction, ProgramError> {
    if accounts.len() < 8
        || accounts.len() > 255
        || inputs.is_empty()
        || outputs.is_empty()
        || output_queue as usize >= accounts.len() - 7
        || inputs.iter().any(|x| {
            x.amount == 0
                || x.owner as usize >= accounts.len() - 7
                || x.mint as usize >= accounts.len() - 7
                || x.tree as usize >= accounts.len() - 7
                || x.queue as usize >= accounts.len() - 7
                || (x.has_delegate && x.delegate as usize >= accounts.len() - 7)
        })
        || outputs.iter().any(|x| {
            x.amount == 0
                || x.owner as usize >= accounts.len() - 7
                || x.mint as usize >= accounts.len() - 7
                || (x.has_delegate && x.delegate as usize >= accounts.len() - 7)
        })
        || (proof.is_none() && inputs.iter().any(|x| !x.prove_by_index))
    {
        return Err(ProgramError::InvalidInstructionData);
    }
    // Light itself authenticates each input. Reject accidental mint creation
    // or loss before invoking it; callers also check their economic deltas.
    for mint in inputs.iter().map(|x| x.mint) {
        let debit: u128 = inputs
            .iter()
            .filter(|x| x.mint == mint)
            .map(|x| u128::from(x.amount))
            .sum();
        let credit: u128 = outputs
            .iter()
            .filter(|x| x.mint == mint)
            .map(|x| u128::from(x.amount))
            .sum();
        if debit != credit {
            return Err(ProgramError::InvalidInstructionData);
        }
    }
    if outputs
        .iter()
        .any(|x| !inputs.iter().any(|i| i.mint == x.mint))
    {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mut data =
        Vec::with_capacity(16 + proof.map_or(0, |_| 128) + 22 * inputs.len() + 13 * outputs.len());
    data.extend_from_slice(&[TRANSFER2, 0, 0, 0, 0, output_queue, 0, 0, 0, 0]);
    if let Some(proof) = proof {
        data.push(1);
        data.extend_from_slice(&proof);
    } else {
        data.push(0);
    }
    data.extend_from_slice(&(inputs.len() as u32).to_le_bytes());
    for leaf in inputs {
        push_input(&mut data, leaf);
    }
    data.extend_from_slice(&(outputs.len() as u32).to_le_bytes());
    for leaf in outputs {
        push_output(&mut data, leaf);
    }
    data.extend_from_slice(&[0, 0, 0, 0]);
    Ok(Instruction {
        program_id: program,
        accounts,
        data,
    })
}
