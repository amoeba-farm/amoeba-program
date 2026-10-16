//! Deterministic, bounded Release 1 artifact Merkle commitments.
//!
//! The tree commits the exact loader payload bytes.  The fixed 512-bit bitmap
//! ABI remains available, while the selected 1.5 MiB/16 KiB policy admits at
//! most 96 real chunks and pads its tree to at most 128 leaves (depth seven).
//! Release 1's final chunk size is the 16 KiB candidate selected by actual SBF
//! v0/v2 benchmarking. The smaller constants remain named only as benchmark
//! history; production commitment utilities reject them.

use solana_program::hash::hashv;

use crate::{GovernanceError, GovernanceResult};

pub const ARTIFACT_CHUNK_LEAF_DOMAIN_V1: &[u8] = b"AMOEBA_ARTIFACT_CHUNK_V1";
pub const ARTIFACT_CHUNK_NODE_DOMAIN_V1: &[u8] = b"AMOEBA_ARTIFACT_NODE_V1";
pub const ARTIFACT_CHUNK_EMPTY_DOMAIN_V1: &[u8] = b"AMOEBA_ARTIFACT_EMPTY_V1";
pub const ARTIFACT_MERKLE_SCHEME_MATERIAL_V1: &[u8] = b"AMOEBA_ARTIFACT_MERKLE_V1\0AMOEBA_ARTIFACT_CHUNK_V1\0AMOEBA_ARTIFACT_NODE_V1\0AMOEBA_ARTIFACT_EMPTY_V1\0U32LE_INDEX_U32LE_ACTUAL_LENGTH_NEXT_POWER_OF_TWO";
pub const ARTIFACT_MERKLE_SCHEME_ID: [u8; 32] = [
    0x8a, 0x85, 0x96, 0x39, 0x69, 0x79, 0x74, 0xc1, 0x6e, 0x2e, 0xb3, 0x74, 0x3d, 0x26, 0x0d, 0x30,
    0x3c, 0x11, 0x31, 0x3c, 0x7d, 0x3c, 0x5c, 0x0d, 0xdc, 0xd0, 0x56, 0x3f, 0x5d, 0x1a, 0x32, 0xa5,
];

pub const ARTIFACT_CHUNK_SIZE_4_KIB: u32 = 4 * 1024;
pub const ARTIFACT_CHUNK_SIZE_8_KIB: u32 = 8 * 1024;
pub const ARTIFACT_CHUNK_SIZE_16_KIB: u32 = 16 * 1024;
pub const BENCHMARK_ARTIFACT_CHUNK_SIZE_CANDIDATES_V1: [u32; 3] = [
    ARTIFACT_CHUNK_SIZE_4_KIB,
    ARTIFACT_CHUNK_SIZE_8_KIB,
    ARTIFACT_CHUNK_SIZE_16_KIB,
];
pub const RELEASE1_ARTIFACT_CHUNK_SIZE_V1: u32 = ARTIFACT_CHUNK_SIZE_16_KIB;
pub const MAX_ARTIFACT_BYTES_V1: u64 = 1_572_864;
/// The audited fixed bitmap capacity remains 512 bits even though the selected
/// 16 KiB scheme can populate at most 96 of them. The bitmap ABI remains fixed
/// at 64 bytes; lowering the Release 1 executable payload ceiling must not
/// silently resize the account schemas.
pub const MAX_ARTIFACT_CHUNKS_V1: usize = 512;
pub const MAX_SELECTED_ARTIFACT_CHUNKS_V1: usize =
    MAX_ARTIFACT_BYTES_V1 as usize / RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as usize;
/// Next-power-of-two tree width for the 96-real-chunk Release 1 maximum.
/// This bound is only for canonical empty leaves and padding subtrees.
pub const MAX_PADDED_ARTIFACT_CHUNKS_V1: usize = 128;
pub const MAX_ARTIFACT_PROOF_DEPTH_V1: usize = 7;

pub const fn is_benchmark_chunk_size_candidate(chunk_size: u32) -> bool {
    matches!(
        chunk_size,
        ARTIFACT_CHUNK_SIZE_4_KIB | ARTIFACT_CHUNK_SIZE_8_KIB | ARTIFACT_CHUNK_SIZE_16_KIB
    )
}

pub const fn is_release1_chunk_size(chunk_size: u32) -> bool {
    chunk_size == RELEASE1_ARTIFACT_CHUNK_SIZE_V1
}

pub fn artifact_chunk_count(artifact_length: u64, chunk_size: u32) -> GovernanceResult<u32> {
    if artifact_length == 0
        || artifact_length > MAX_ARTIFACT_BYTES_V1
        || !is_release1_chunk_size(chunk_size)
    {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    let chunk_size = u64::from(chunk_size);
    let count = artifact_length
        .checked_add(chunk_size - 1)
        .ok_or(GovernanceError::ArithmeticOverflow)?
        / chunk_size;
    let count = u32::try_from(count).map_err(|_| GovernanceError::InvalidMerkleParameters)?;
    if count == 0 || count as usize > MAX_SELECTED_ARTIFACT_CHUNKS_V1 {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    Ok(count)
}

pub fn artifact_chunk_leaf_hash(chunk_index: u32, chunk: &[u8]) -> GovernanceResult<[u8; 32]> {
    if chunk_index as usize >= MAX_SELECTED_ARTIFACT_CHUNKS_V1
        || chunk.is_empty()
        || chunk.len() > ARTIFACT_CHUNK_SIZE_16_KIB as usize
    {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    let actual_length =
        u32::try_from(chunk.len()).map_err(|_| GovernanceError::InvalidMerkleParameters)?;
    Ok(hashv(&[
        ARTIFACT_CHUNK_LEAF_DOMAIN_V1,
        &chunk_index.to_le_bytes(),
        &actual_length.to_le_bytes(),
        chunk,
    ])
    .to_bytes())
}

pub fn artifact_chunk_node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    hashv(&[ARTIFACT_CHUNK_NODE_DOMAIN_V1, left, right]).to_bytes()
}

pub fn artifact_chunk_empty_hash(padded_index: u32) -> GovernanceResult<[u8; 32]> {
    if padded_index as usize >= MAX_PADDED_ARTIFACT_CHUNKS_V1 {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    Ok(hashv(&[ARTIFACT_CHUNK_EMPTY_DOMAIN_V1, &padded_index.to_le_bytes()]).to_bytes())
}

pub fn artifact_merkle_root(artifact: &[u8], chunk_size: u32) -> GovernanceResult<[u8; 32]> {
    let artifact_length =
        u64::try_from(artifact.len()).map_err(|_| GovernanceError::InvalidMerkleParameters)?;
    let chunk_count = artifact_chunk_count(artifact_length, chunk_size)? as usize;
    let padded_count = chunk_count.next_power_of_two();
    let chunk_size = chunk_size as usize;

    let mut level = Vec::with_capacity(padded_count);
    for (index, chunk) in artifact.chunks(chunk_size).enumerate() {
        level.push(artifact_chunk_leaf_hash(index as u32, chunk)?);
    }
    for index in chunk_count..padded_count {
        level.push(artifact_chunk_empty_hash(index as u32)?);
    }

    reduce_tree(level)
}

pub fn artifact_merkle_proof(
    artifact: &[u8],
    chunk_size: u32,
    chunk_index: u32,
) -> GovernanceResult<Vec<[u8; 32]>> {
    let artifact_length =
        u64::try_from(artifact.len()).map_err(|_| GovernanceError::InvalidMerkleParameters)?;
    let chunk_count = artifact_chunk_count(artifact_length, chunk_size)? as usize;
    let chunk_index = chunk_index as usize;
    if chunk_index >= chunk_count {
        return Err(GovernanceError::InvalidMerkleProof);
    }
    let padded_count = chunk_count.next_power_of_two();
    let chunk_size = chunk_size as usize;
    let mut level = Vec::with_capacity(padded_count);
    for (index, chunk) in artifact.chunks(chunk_size).enumerate() {
        level.push(artifact_chunk_leaf_hash(index as u32, chunk)?);
    }
    for index in chunk_count..padded_count {
        level.push(artifact_chunk_empty_hash(index as u32)?);
    }

    let mut proof = Vec::with_capacity(padded_count.trailing_zeros() as usize);
    let mut index = chunk_index;
    while level.len() > 1 {
        proof.push(level[index ^ 1]);
        let mut next = Vec::with_capacity(level.len() / 2);
        for pair in level.chunks_exact(2) {
            next.push(artifact_chunk_node_hash(&pair[0], &pair[1]));
        }
        index /= 2;
        level = next;
    }
    Ok(proof)
}

pub fn verify_artifact_chunk_proof(
    expected_root: &[u8; 32],
    artifact_length: u64,
    chunk_size: u32,
    chunk_index: u32,
    exact_chunk: &[u8],
    proof: &[[u8; 32]],
) -> GovernanceResult<()> {
    let chunk_count = artifact_chunk_count(artifact_length, chunk_size)?;
    if chunk_index >= chunk_count {
        return Err(GovernanceError::InvalidMerkleProof);
    }
    let chunk_start = u64::from(chunk_index)
        .checked_mul(u64::from(chunk_size))
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let expected_length = artifact_length
        .checked_sub(chunk_start)
        .ok_or(GovernanceError::InvalidMerkleProof)?
        .min(u64::from(chunk_size));
    if exact_chunk.len() as u64 != expected_length {
        return Err(GovernanceError::InvalidMerkleProof);
    }

    let padded_count = (chunk_count as usize).next_power_of_two();
    let expected_depth = padded_count.trailing_zeros() as usize;
    if proof.len() != expected_depth || proof.len() > MAX_ARTIFACT_PROOF_DEPTH_V1 {
        return Err(GovernanceError::InvalidMerkleProof);
    }

    let mut current = artifact_chunk_leaf_hash(chunk_index, exact_chunk)?;
    let mut index = chunk_index;
    for (proof_level, sibling) in proof.iter().enumerate() {
        validate_canonical_padding_sibling(
            sibling,
            index,
            proof_level,
            chunk_count as usize,
            padded_count,
        )?;
        current = if index & 1 == 0 {
            artifact_chunk_node_hash(&current, sibling)
        } else {
            artifact_chunk_node_hash(sibling, &current)
        };
        index >>= 1;
    }
    if &current != expected_root {
        return Err(GovernanceError::InvalidMerkleProof);
    }
    Ok(())
}

/// A Merkle proof cannot choose its own hashes for the portion of the final
/// power-of-two tree that lies beyond the artifact's real chunks.  Whenever a
/// proof sibling covers only padded leaves, recompute that entire sibling from
/// the canonical index-separated empty leaves and require exact equality.
///
/// A sibling that contains at least one real chunk remains opaque to this
/// single-chunk verifier.  Complete buffer verification checks every real
/// chunk, so the final real chunk necessarily exposes every all-padding range
/// at the right edge of a partial tree.
fn validate_canonical_padding_sibling(
    sibling: &[u8; 32],
    node_index: u32,
    proof_level: usize,
    chunk_count: usize,
    padded_count: usize,
) -> GovernanceResult<()> {
    if proof_level >= MAX_ARTIFACT_PROOF_DEPTH_V1 {
        return Err(GovernanceError::InvalidMerkleProof);
    }
    let sibling_leaf_count = 1usize
        .checked_shl(proof_level as u32)
        .ok_or(GovernanceError::InvalidMerkleProof)?;
    let sibling_node_index = (node_index as usize) ^ 1;
    let sibling_start = sibling_node_index
        .checked_mul(sibling_leaf_count)
        .ok_or(GovernanceError::InvalidMerkleProof)?;
    let sibling_end = sibling_start
        .checked_add(sibling_leaf_count)
        .ok_or(GovernanceError::InvalidMerkleProof)?;
    if sibling_end > padded_count {
        return Err(GovernanceError::InvalidMerkleProof);
    }
    if sibling_start < chunk_count {
        return Ok(());
    }

    let canonical = artifact_padding_subtree_hash(sibling_start, sibling_leaf_count)?;
    if sibling != &canonical {
        return Err(GovernanceError::InvalidMerkleProof);
    }
    Ok(())
}

fn artifact_padding_subtree_hash(
    padded_start: usize,
    padded_leaf_count: usize,
) -> GovernanceResult<[u8; 32]> {
    let padded_end = padded_start
        .checked_add(padded_leaf_count)
        .ok_or(GovernanceError::InvalidMerkleProof)?;
    if padded_leaf_count == 0
        || !padded_leaf_count.is_power_of_two()
        || padded_start % padded_leaf_count != 0
        || padded_end > MAX_PADDED_ARTIFACT_CHUNKS_V1
    {
        return Err(GovernanceError::InvalidMerkleProof);
    }

    let mut level = Vec::with_capacity(padded_leaf_count);
    for padded_index in padded_start..padded_end {
        let padded_index =
            u32::try_from(padded_index).map_err(|_| GovernanceError::InvalidMerkleProof)?;
        level.push(
            artifact_chunk_empty_hash(padded_index)
                .map_err(|_| GovernanceError::InvalidMerkleProof)?,
        );
    }
    reduce_tree(level).map_err(|_| GovernanceError::InvalidMerkleProof)
}

fn reduce_tree(mut level: Vec<[u8; 32]>) -> GovernanceResult<[u8; 32]> {
    if level.is_empty() || !level.len().is_power_of_two() {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len() / 2);
        for pair in level.chunks_exact(2) {
            next.push(artifact_chunk_node_hash(&pair[0], &pair[1]));
        }
        level = next;
    }
    level.pop().ok_or(GovernanceError::InvalidMerkleParameters)
}
