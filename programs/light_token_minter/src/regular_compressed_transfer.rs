//! Fixed regular v3 Transfer2 encoding for authenticated leaves and optional
//! compression and decompression of canonical hot custody. No top-up or TLV is represented.
use crate::ProgramError;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

const TRANSFER2: u8 = 101;
const REGULAR_V3: u8 = 3;

/// Packed mint indices are bytes. Remember which complete mint sums were
/// checked without allocating or repeating them for every fragment/output.
#[inline]
fn first_mint(seen: &mut [u64; 4], mint: u8) -> bool {
    let word = usize::from(mint >> 6);
    let bit = 1u64 << (mint & 63);
    let first = seen[word] & bit == 0;
    seen[word] |= bit;
    first
}

#[inline]
fn contains_mint(seen: &[u64; 4], mint: u8) -> bool {
    seen[usize::from(mint >> 6)] & (1u64 << (mint & 63)) != 0
}

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
/// as the authenticated regular leaves.
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

/// Canonical hot destination authenticated by the business handler. A Light
/// token account uses zero interface fields; an SPL target uses its validated
/// interface pool and mint decimals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotDecompression {
    pub amount: u64,
    pub mint: u8,
    pub recipient: u8,
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
    instruction_with_hot_actions(
        program,
        accounts,
        output_queue,
        proof,
        inputs,
        compressions,
        &[],
        outputs,
    )
}

/// Transfer regular leaves and typed hot custody in one conserving Light CPI.
/// Every packed identity, destination and exact balance delta remains the
/// responsibility of the calling business handler.
#[allow(clippy::too_many_arguments)]
pub fn instruction_with_hot_actions(
    program: Pubkey,
    accounts: Vec<AccountMeta>,
    output_queue: u8,
    proof: Option<[u8; 128]>,
    inputs: &[InputLeaf],
    compressions: &[HotCompression],
    decompressions: &[HotDecompression],
    outputs: &[OutputLeaf],
) -> Result<Instruction, ProgramError> {
    if compressions.is_empty() && decompressions.is_empty() {
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
        || (outputs.is_empty() && decompressions.is_empty())
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
        || decompressions.iter().any(|x| {
            x.amount == 0
                || [x.mint, x.recipient, x.pool_account_index]
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
    let mut seen_mints = [0u64; 4];
    for mint in inputs
        .iter()
        .map(|x| x.mint)
        .chain(compressions.iter().map(|x| x.mint))
        .chain(decompressions.iter().map(|x| x.mint))
        .chain(outputs.iter().map(|x| x.mint))
    {
        if !first_mint(&mut seen_mints, mint) {
            continue;
        }
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
            .sum::<u128>()
            + decompressions
                .iter()
                .filter(|x| x.mint == mint)
                .map(|x| u128::from(x.amount))
                .sum::<u128>();
        if debit != credit {
            return Err(ProgramError::InvalidInstructionData);
        }
    }
    let mut data = Vec::with_capacity(
        32 + proof.map_or(0, |_| 128)
            + 16 * (compressions.len() + decompressions.len())
            + 22 * inputs.len()
            + 13 * outputs.len(),
    );
    data.extend_from_slice(&[TRANSFER2, 0, 0, 0, 0, output_queue, 0, 0, 0, 1]);
    let action_count = compressions
        .len()
        .checked_add(decompressions.len())
        .and_then(|n| u32::try_from(n).ok())
        .ok_or(ProgramError::InvalidInstructionData)?;
    data.extend_from_slice(&action_count.to_le_bytes());
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
    for x in decompressions {
        data.push(1); // CompressionMode::Decompress
        data.extend_from_slice(&x.amount.to_le_bytes());
        data.extend_from_slice(&[
            x.mint,
            x.recipient,
            0,
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
    let mut seen_mints = [0u64; 4];
    for mint in inputs.iter().map(|x| x.mint) {
        if !first_mint(&mut seen_mints, mint) {
            continue;
        }
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
    if outputs.iter().any(|x| !contains_mint(&seen_mints, x.mint)) {
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

/// One SPL-pool decompression of regular leaves into a classic token account.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SplDecompression {
    pub amount: u64,
    pub mint: u8,
    pub recipient: u8,
    pub pool_account_index: u8,
    pub pool_index: u8,
    pub bump: u8,
    pub decimals: u8,
}

/// Build one Light Transfer2 that consumes authenticated regular leaves and
/// decompresses `decompression.amount` into a classic SPL account through the
/// mint's SPL interface pool. Outputs carry change; per-mint debit equals
/// outputs plus the decompression. The caller authenticates every account and
/// checks the recipient's exact balance delta.
pub fn decompress_to_spl_instruction(
    program: Pubkey,
    accounts: Vec<AccountMeta>,
    output_queue: u8,
    proof: Option<[u8; 128]>,
    inputs: &[InputLeaf],
    decompression: SplDecompression,
    outputs: &[OutputLeaf],
) -> Result<Instruction, ProgramError> {
    let count = accounts
        .len()
        .checked_sub(7)
        .ok_or(ProgramError::InvalidInstructionData)?;
    if count == 0
        || count > 248
        || usize::from(output_queue) >= count
        || inputs.is_empty()
        || decompression.amount == 0
        || [
            decompression.mint,
            decompression.recipient,
            decompression.pool_account_index,
        ]
        .iter()
        .any(|index| usize::from(*index) >= count)
        || inputs.iter().any(|x| {
            x.amount == 0
                || x.mint != decompression.mint
                || [x.owner, x.mint, x.tree, x.queue]
                    .iter()
                    .any(|index| usize::from(*index) >= count)
                || x.has_delegate && usize::from(x.delegate) >= count
        })
        || outputs.iter().any(|x| {
            x.amount == 0
                || x.mint != decompression.mint
                || usize::from(x.owner) >= count
                || x.has_delegate && usize::from(x.delegate) >= count
        })
        || (proof.is_none() && inputs.iter().any(|x| !x.prove_by_index))
    {
        return Err(ProgramError::InvalidInstructionData);
    }
    let debit: u128 = inputs.iter().map(|x| u128::from(x.amount)).sum();
    let credit: u128 = outputs.iter().map(|x| u128::from(x.amount)).sum::<u128>()
        + u128::from(decompression.amount);
    if debit != credit {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mut data = Vec::with_capacity(
        32 + 16 + proof.map_or(0, |_| 128) + 22 * inputs.len() + 13 * outputs.len(),
    );
    data.extend_from_slice(&[TRANSFER2, 0, 0, 0, 0, output_queue, 0, 0, 0, 1]);
    data.extend_from_slice(&1u32.to_le_bytes());
    data.push(1); // CompressionMode::Decompress
    data.extend_from_slice(&decompression.amount.to_le_bytes());
    data.extend_from_slice(&[
        decompression.mint,
        decompression.recipient,
        0,
        decompression.pool_account_index,
        decompression.pool_index,
        decompression.bump,
        decompression.decimals,
    ]);
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
