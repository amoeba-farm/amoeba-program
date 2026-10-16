//! V3-local artifact bound: 2 MiB (128 chunks of 16 KiB).
//!
//! The Merkle scheme is unchanged from Release 1 (same leaf/node/empty
//! domains, 16 KiB chunks, index-separated canonical padding and a
//! next-power-of-two tree). The Release 1 helpers in `upgrade_controller`
//! stay pinned at 1,572,864 bytes (96 chunks) for the historical controller,
//! so V3 keeps its own bound here. At 128 chunks the padded width is still
//! 128 leaves and every proof still has at most seven siblings, so the
//! `VerifyChunk` wire format is unchanged and every root accepted under the
//! old bound is computed identically.
use crate::state::{require, Error, Result};
use solana_program::hash::hashv;
use upgrade_controller::artifact_merkle::{
    artifact_chunk_empty_hash, artifact_chunk_node_hash, ARTIFACT_CHUNK_LEAF_DOMAIN_V1,
    MAX_ARTIFACT_PROOF_DEPTH_V1, MAX_PADDED_ARTIFACT_CHUNKS_V1, RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
};

pub const CHUNK: u32 = RELEASE1_ARTIFACT_CHUNK_SIZE_V1;
pub const MAX_ARTIFACT_BYTES: u64 = 2_097_152;
pub const MAX_CHUNKS: usize = 128;
pub const BITMAP_BYTES: usize = MAX_CHUNKS / 8;
pub const MAX_PROOF_DEPTH: usize = 7;
const _: () = assert!(MAX_ARTIFACT_BYTES == MAX_CHUNKS as u64 * CHUNK as u64);
const _: () = assert!(MAX_CHUNKS == 1 << MAX_PROOF_DEPTH);
const _: () = assert!(MAX_CHUNKS == MAX_PADDED_ARTIFACT_CHUNKS_V1);
const _: () = assert!(MAX_PROOF_DEPTH == MAX_ARTIFACT_PROOF_DEPTH_V1);

pub fn chunk_count(artifact_length: u64) -> Result<u32> {
    require(
        artifact_length > 0 && artifact_length <= MAX_ARTIFACT_BYTES,
        Error::InvalidArtifact,
    )?;
    Ok(artifact_length.div_ceil(u64::from(CHUNK)) as u32)
}

pub fn leaf_hash(index: u32, chunk: &[u8]) -> Result<[u8; 32]> {
    require(
        (index as usize) < MAX_CHUNKS && !chunk.is_empty() && chunk.len() <= CHUNK as usize,
        Error::InvalidArtifact,
    )?;
    Ok(hashv(&[
        ARTIFACT_CHUNK_LEAF_DOMAIN_V1,
        &index.to_le_bytes(),
        &(chunk.len() as u32).to_le_bytes(),
        chunk,
    ])
    .to_bytes())
}

fn empty_hash(index: usize) -> Result<[u8; 32]> {
    artifact_chunk_empty_hash(index as u32).map_err(|_| Error::InvalidArtifact.into())
}

fn reduce(mut level: Vec<[u8; 32]>) -> Result<[u8; 32]> {
    require(
        !level.is_empty() && level.len().is_power_of_two(),
        Error::InvalidArtifact,
    )?;
    while level.len() > 1 {
        let half = level.len() / 2;
        for i in 0..half {
            level[i] = artifact_chunk_node_hash(&level[2 * i], &level[2 * i + 1]);
        }
        level.truncate(half);
    }
    Ok(level[0])
}

/// Canonical hash of an aligned run of padding leaves.
fn padding_hash(start: usize, width: usize) -> Result<[u8; 32]> {
    require(
        width.is_power_of_two() && start % width == 0 && start + width <= MAX_CHUNKS,
        Error::InvalidArtifact,
    )?;
    let mut level = Vec::with_capacity(width);
    for index in start..start + width {
        level.push(empty_hash(index)?);
    }
    reduce(level)
}

/// Verifies one exact chunk against the committed root. Any sibling that
/// covers only padding must be the canonical padding subtree, so a proof
/// cannot choose hashes for leaves beyond the real artifact.
pub fn verify_chunk(
    root: &[u8; 32],
    artifact_length: u64,
    index: u32,
    chunk: &[u8],
    proof: &[[u8; 32]],
) -> Result<()> {
    let count = chunk_count(artifact_length)? as usize;
    let index = index as usize;
    require(index < count, Error::InvalidArtifact)?;
    let start = index as u64 * u64::from(CHUNK);
    require(
        chunk.len() as u64 == (artifact_length - start).min(u64::from(CHUNK)),
        Error::InvalidArtifact,
    )?;
    let padded = count.next_power_of_two();
    require(
        proof.len() == padded.trailing_zeros() as usize && proof.len() <= MAX_PROOF_DEPTH,
        Error::InvalidArtifact,
    )?;
    let mut current = leaf_hash(index as u32, chunk)?;
    let mut node = index;
    for (level, sibling) in proof.iter().enumerate() {
        let width = 1usize << level;
        let sibling_start = (node ^ 1) * width;
        require(sibling_start + width <= padded, Error::InvalidArtifact)?;
        if sibling_start >= count {
            require(
                *sibling == padding_hash(sibling_start, width)?,
                Error::InvalidArtifact,
            )?;
        }
        current = if node & 1 == 0 {
            artifact_chunk_node_hash(&current, sibling)
        } else {
            artifact_chunk_node_hash(sibling, &current)
        };
        node >>= 1;
    }
    require(current == *root, Error::InvalidArtifact)
}

#[cfg(not(target_os = "solana"))]
fn leaves(artifact: &[u8]) -> Result<Vec<[u8; 32]>> {
    let count = chunk_count(artifact.len() as u64)? as usize;
    let mut level = artifact
        .chunks(CHUNK as usize)
        .enumerate()
        .map(|(i, chunk)| leaf_hash(i as u32, chunk))
        .collect::<Result<Vec<_>>>()?;
    for i in count..count.next_power_of_two() {
        level.push(empty_hash(i)?);
    }
    Ok(level)
}

/// Off-chain root for proposal construction and tests.
#[cfg(not(target_os = "solana"))]
pub fn merkle_root(artifact: &[u8]) -> Result<[u8; 32]> {
    reduce(leaves(artifact)?)
}

/// Off-chain sibling path for `VerifyChunk`.
#[cfg(not(target_os = "solana"))]
pub fn merkle_proof(artifact: &[u8], index: u32) -> Result<Vec<[u8; 32]>> {
    let mut level = leaves(artifact)?;
    let mut node = index as usize;
    require(
        node < chunk_count(artifact.len() as u64)? as usize,
        Error::InvalidArtifact,
    )?;
    let mut proof = Vec::with_capacity(MAX_PROOF_DEPTH);
    while level.len() > 1 {
        proof.push(level[node ^ 1]);
        level = level
            .chunks_exact(2)
            .map(|pair| artifact_chunk_node_hash(&pair[0], &pair[1]))
            .collect();
        node >>= 1;
    }
    Ok(proof)
}
