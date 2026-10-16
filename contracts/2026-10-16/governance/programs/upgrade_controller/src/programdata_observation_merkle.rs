//! Deterministic, bounded Merkle accumulation for raw Loader-v3 ProgramData.
//!
//! Every real leaf commits the observation subject, its ordered chunk index,
//! the exact actual chunk length, and the exact raw ProgramData account bytes.
//! The tree is padded to the next power of two with subject- and
//! index-separated empty leaves. A fixed binary frontier permits an observation
//! to advance in ordered transactions without a persisted bitmap or an
//! allocation proportional to the ProgramData account size.

use solana_program::hash::hashv;

use crate::{GovernanceError, GovernanceResult};

pub const PROGRAMDATA_OBSERVATION_CHUNK_LEAF_DOMAIN_V1: &[u8] =
    b"AMOEBA_PROGRAMDATA_OBSERVATION_CHUNK_V1";
pub const PROGRAMDATA_OBSERVATION_CHUNK_NODE_DOMAIN_V1: &[u8] =
    b"AMOEBA_PROGRAMDATA_OBSERVATION_NODE_V1";
pub const PROGRAMDATA_OBSERVATION_CHUNK_EMPTY_DOMAIN_V1: &[u8] =
    b"AMOEBA_PROGRAMDATA_OBSERVATION_EMPTY_V1";
pub const PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_MATERIAL_V1: &[u8] = b"AMOEBA_PROGRAMDATA_OBSERVATION_MERKLE_V1\0AMOEBA_PROGRAMDATA_OBSERVATION_CHUNK_V1\0AMOEBA_PROGRAMDATA_OBSERVATION_NODE_V1\0AMOEBA_PROGRAMDATA_OBSERVATION_EMPTY_V1\0SUBJECT_DIGEST_U32LE_INDEX_U32LE_ACTUAL_LENGTH_U8_PARENT_LEVEL_NEXT_POWER_OF_TWO_ORDERED_FRONTIER";
pub const PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1: [u8; 32] = [
    0x6f, 0x7a, 0xfe, 0x51, 0xac, 0xf1, 0xd7, 0x01, 0xbd, 0xc5, 0xd9, 0xde, 0x71, 0x45, 0xde, 0x24,
    0xde, 0x10, 0xd6, 0xd4, 0x54, 0x18, 0x2d, 0xcd, 0xc6, 0x9e, 0x68, 0xb5, 0x49, 0x24, 0x2e, 0xd2,
];

/// Loader-v3's pinned runtime ceiling for the complete ProgramData account,
/// including its 45-byte metadata header.
pub const MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1: u64 = 10_485_760;
pub const MIN_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1: u64 = 45;

pub const PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB: u32 = 16 * 1024;
pub const PROGRAMDATA_OBSERVATION_CHUNK_SIZE_32_KIB: u32 = 32 * 1024;
pub const PROGRAMDATA_OBSERVATION_CHUNK_SIZE_64_KIB: u32 = 64 * 1024;
pub const PROGRAMDATA_OBSERVATION_CHUNK_SIZE_128_KIB: u32 = 128 * 1024;
pub const PROGRAMDATA_OBSERVATION_CHUNK_SIZE_CANDIDATES_V1: [u32; 4] = [
    PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB,
    PROGRAMDATA_OBSERVATION_CHUNK_SIZE_32_KIB,
    PROGRAMDATA_OBSERVATION_CHUNK_SIZE_64_KIB,
    PROGRAMDATA_OBSERVATION_CHUNK_SIZE_128_KIB,
];

/// The 16 KiB candidate produces the greatest number of real chunks at the
/// pinned runtime maximum: exactly 640, padded to 1,024 leaves.
pub const MAX_PROGRAMDATA_OBSERVATION_CHUNKS_V1: u32 = 640;
pub const MAX_PADDED_PROGRAMDATA_OBSERVATION_CHUNKS_V1: u32 = 1_024;
pub const MAX_PROGRAMDATA_OBSERVATION_TREE_DEPTH_V1: usize = 10;
/// Level zero stores an unpaired leaf. Level ten stores the 1,024-leaf root.
pub const PROGRAMDATA_OBSERVATION_FRONTIER_SLOTS_V1: usize =
    MAX_PROGRAMDATA_OBSERVATION_TREE_DEPTH_V1 + 1;

/// Canonical fixed frontier for ordered ProgramData observation.
///
/// `occupied_mask` bit `n` identifies a hash covering `2^n` leaves. For every
/// canonical frontier, the mask is the binary representation of `next_index`,
/// and every unoccupied hash slot is zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgramDataObservationMerkleFrontierV1 {
    pub hashes: [[u8; 32]; PROGRAMDATA_OBSERVATION_FRONTIER_SLOTS_V1],
    pub occupied_mask: u16,
    pub next_index: u32,
}

impl Default for ProgramDataObservationMerkleFrontierV1 {
    fn default() -> Self {
        Self {
            hashes: [[0; 32]; PROGRAMDATA_OBSERVATION_FRONTIER_SLOTS_V1],
            occupied_mask: 0,
            next_index: 0,
        }
    }
}

impl ProgramDataObservationMerkleFrontierV1 {
    pub fn validate(&self) -> GovernanceResult<()> {
        if self.next_index > MAX_PADDED_PROGRAMDATA_OBSERVATION_CHUNKS_V1
            || self.occupied_mask != self.next_index as u16
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }

        for (level, hash) in self.hashes.iter().enumerate() {
            let occupied = self.occupied_mask & (1u16 << level) != 0;
            if !occupied && *hash != [0; 32] {
                return Err(GovernanceError::InvalidRelease1Account);
            }
        }
        Ok(())
    }
}

pub const fn is_programdata_observation_chunk_size_candidate(chunk_size: u32) -> bool {
    matches!(
        chunk_size,
        PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB
            | PROGRAMDATA_OBSERVATION_CHUNK_SIZE_32_KIB
            | PROGRAMDATA_OBSERVATION_CHUNK_SIZE_64_KIB
            | PROGRAMDATA_OBSERVATION_CHUNK_SIZE_128_KIB
    )
}

pub fn programdata_observation_chunk_count(
    raw_programdata_length: u64,
    chunk_size: u32,
) -> GovernanceResult<u32> {
    if !(MIN_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1..=MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1)
        .contains(&raw_programdata_length)
        || !is_programdata_observation_chunk_size_candidate(chunk_size)
    {
        return Err(GovernanceError::InvalidMerkleParameters);
    }

    let chunk_size = u64::from(chunk_size);
    let count = raw_programdata_length
        .checked_add(chunk_size - 1)
        .ok_or(GovernanceError::ArithmeticOverflow)?
        / chunk_size;
    let count = u32::try_from(count).map_err(|_| GovernanceError::InvalidMerkleParameters)?;
    if count == 0 || count > MAX_PROGRAMDATA_OBSERVATION_CHUNKS_V1 {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    Ok(count)
}

pub fn programdata_observation_chunk_leaf_hash(
    subject_digest: &[u8; 32],
    chunk_index: u32,
    chunk_size: u32,
    exact_raw_programdata_chunk: &[u8],
) -> GovernanceResult<[u8; 32]> {
    if !is_programdata_observation_chunk_size_candidate(chunk_size)
        || exact_raw_programdata_chunk.is_empty()
        || exact_raw_programdata_chunk.len() > chunk_size as usize
        || chunk_index >= max_chunk_count_for_size(chunk_size)?
    {
        return Err(GovernanceError::InvalidMerkleParameters);
    }

    let actual_length = u32::try_from(exact_raw_programdata_chunk.len())
        .map_err(|_| GovernanceError::InvalidMerkleParameters)?;
    Ok(hashv(&[
        PROGRAMDATA_OBSERVATION_CHUNK_LEAF_DOMAIN_V1,
        subject_digest,
        &chunk_index.to_le_bytes(),
        &actual_length.to_le_bytes(),
        exact_raw_programdata_chunk,
    ])
    .to_bytes())
}

pub fn programdata_observation_chunk_node_hash(
    parent_level: u8,
    left: &[u8; 32],
    right: &[u8; 32],
) -> GovernanceResult<[u8; 32]> {
    if parent_level == 0 || usize::from(parent_level) > MAX_PROGRAMDATA_OBSERVATION_TREE_DEPTH_V1 {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    Ok(hashv(&[
        PROGRAMDATA_OBSERVATION_CHUNK_NODE_DOMAIN_V1,
        &[parent_level],
        left,
        right,
    ])
    .to_bytes())
}

pub fn programdata_observation_chunk_empty_hash(
    subject_digest: &[u8; 32],
    padded_index: u32,
) -> GovernanceResult<[u8; 32]> {
    if padded_index >= MAX_PADDED_PROGRAMDATA_OBSERVATION_CHUNKS_V1 {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    Ok(hashv(&[
        PROGRAMDATA_OBSERVATION_CHUNK_EMPTY_DOMAIN_V1,
        subject_digest,
        &padded_index.to_le_bytes(),
    ])
    .to_bytes())
}

/// Append exactly the next real chunk. Failed validation leaves the frontier
/// byte-identical.
pub fn append_programdata_observation_chunk(
    frontier: &mut ProgramDataObservationMerkleFrontierV1,
    subject_digest: &[u8; 32],
    raw_programdata_length: u64,
    chunk_size: u32,
    chunk_index: u32,
    exact_raw_programdata_chunk: &[u8],
) -> GovernanceResult<()> {
    frontier.validate()?;
    let chunk_count = programdata_observation_chunk_count(raw_programdata_length, chunk_size)?;
    if chunk_index != frontier.next_index || chunk_index >= chunk_count {
        return Err(GovernanceError::InvalidStateTransition);
    }

    let expected_length =
        expected_real_chunk_length(raw_programdata_length, chunk_size, chunk_index, chunk_count)?;
    if exact_raw_programdata_chunk.len() as u64 != expected_length {
        return Err(GovernanceError::InvalidMerkleParameters);
    }

    let leaf = programdata_observation_chunk_leaf_hash(
        subject_digest,
        chunk_index,
        chunk_size,
        exact_raw_programdata_chunk,
    )?;
    let mut next = *frontier;
    append_hash_at_next_index(&mut next, leaf)?;
    next.validate()?;
    *frontier = next;
    Ok(())
}

/// Finish a complete real-chunk frontier using canonical subject-separated
/// empty leaves up to the next power-of-two width. The input frontier is not
/// modified.
pub fn finalize_programdata_observation_frontier(
    frontier: &ProgramDataObservationMerkleFrontierV1,
    subject_digest: &[u8; 32],
    raw_programdata_length: u64,
    chunk_size: u32,
) -> GovernanceResult<[u8; 32]> {
    frontier.validate()?;
    let chunk_count = programdata_observation_chunk_count(raw_programdata_length, chunk_size)?;
    if frontier.next_index != chunk_count {
        return Err(GovernanceError::InvalidStateTransition);
    }

    let padded_count = chunk_count
        .checked_next_power_of_two()
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if padded_count > MAX_PADDED_PROGRAMDATA_OBSERVATION_CHUNKS_V1 {
        return Err(GovernanceError::InvalidMerkleParameters);
    }

    let mut completed = *frontier;
    for padded_index in chunk_count..padded_count {
        let empty = programdata_observation_chunk_empty_hash(subject_digest, padded_index)?;
        append_hash_at_next_index(&mut completed, empty)?;
    }
    completed.validate()?;

    let root_level = padded_count.trailing_zeros() as usize;
    let expected_mask = 1u16 << root_level;
    if completed.next_index != padded_count || completed.occupied_mask != expected_mask {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(completed.hashes[root_level])
}

/// Compute the same root as the transaction-friendly frontier path without
/// allocating a leaf vector proportional to the account size.
pub fn programdata_observation_merkle_root(
    subject_digest: &[u8; 32],
    exact_raw_programdata: &[u8],
    chunk_size: u32,
) -> GovernanceResult<[u8; 32]> {
    let raw_programdata_length = u64::try_from(exact_raw_programdata.len())
        .map_err(|_| GovernanceError::InvalidMerkleParameters)?;
    programdata_observation_chunk_count(raw_programdata_length, chunk_size)?;

    let mut frontier = ProgramDataObservationMerkleFrontierV1::default();
    for (chunk_index, chunk) in exact_raw_programdata
        .chunks(chunk_size as usize)
        .enumerate()
    {
        let chunk_index =
            u32::try_from(chunk_index).map_err(|_| GovernanceError::InvalidMerkleParameters)?;
        append_programdata_observation_chunk(
            &mut frontier,
            subject_digest,
            raw_programdata_length,
            chunk_size,
            chunk_index,
            chunk,
        )?;
    }
    finalize_programdata_observation_frontier(
        &frontier,
        subject_digest,
        raw_programdata_length,
        chunk_size,
    )
}

fn max_chunk_count_for_size(chunk_size: u32) -> GovernanceResult<u32> {
    programdata_observation_chunk_count(MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1, chunk_size)
}

fn expected_real_chunk_length(
    raw_programdata_length: u64,
    chunk_size: u32,
    chunk_index: u32,
    chunk_count: u32,
) -> GovernanceResult<u64> {
    if chunk_index >= chunk_count {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    let start = u64::from(chunk_index)
        .checked_mul(u64::from(chunk_size))
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    raw_programdata_length
        .checked_sub(start)
        .map(|remaining| remaining.min(u64::from(chunk_size)))
        .filter(|length| *length != 0)
        .ok_or(GovernanceError::InvalidMerkleParameters)
}

fn append_hash_at_next_index(
    frontier: &mut ProgramDataObservationMerkleFrontierV1,
    mut current: [u8; 32],
) -> GovernanceResult<()> {
    frontier.validate()?;
    if frontier.next_index >= MAX_PADDED_PROGRAMDATA_OBSERVATION_CHUNKS_V1 {
        return Err(GovernanceError::InvalidStateTransition);
    }

    let mut level = 0usize;
    while frontier.occupied_mask & (1u16 << level) != 0 {
        if level >= MAX_PROGRAMDATA_OBSERVATION_TREE_DEPTH_V1 {
            return Err(GovernanceError::InvalidMerkleParameters);
        }
        current = programdata_observation_chunk_node_hash(
            u8::try_from(level + 1).map_err(|_| GovernanceError::InvalidMerkleParameters)?,
            &frontier.hashes[level],
            &current,
        )?;
        frontier.hashes[level] = [0; 32];
        frontier.occupied_mask &= !(1u16 << level);
        level += 1;
    }

    if level >= PROGRAMDATA_OBSERVATION_FRONTIER_SLOTS_V1 {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    frontier.hashes[level] = current;
    frontier.occupied_mask |= 1u16 << level;
    frontier.next_index = frontier
        .next_index
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterned_bytes(length: usize) -> Vec<u8> {
        (0..length)
            .map(|index| ((index * 29 + index / 7 + 11) & 0xff) as u8)
            .collect()
    }

    fn hex(value: &[u8]) -> String {
        value.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn full_reduction_root(subject_digest: &[u8; 32], raw: &[u8], chunk_size: u32) -> [u8; 32] {
        let chunk_count = programdata_observation_chunk_count(raw.len() as u64, chunk_size)
            .expect("valid geometry") as usize;
        let padded_count = chunk_count.next_power_of_two();
        let mut level = raw
            .chunks(chunk_size as usize)
            .enumerate()
            .map(|(index, chunk)| {
                programdata_observation_chunk_leaf_hash(
                    subject_digest,
                    index as u32,
                    chunk_size,
                    chunk,
                )
                .expect("valid leaf")
            })
            .collect::<Vec<_>>();
        for padded_index in chunk_count..padded_count {
            level.push(
                programdata_observation_chunk_empty_hash(subject_digest, padded_index as u32)
                    .expect("valid empty leaf"),
            );
        }

        let mut parent_level = 1u8;
        while level.len() > 1 {
            level = level
                .chunks_exact(2)
                .map(|pair| {
                    programdata_observation_chunk_node_hash(parent_level, &pair[0], &pair[1])
                        .expect("valid node")
                })
                .collect();
            parent_level += 1;
        }
        level[0]
    }

    #[test]
    fn candidate_geometry_accepts_header_minimum_and_runtime_maximum() {
        let expected_at_max = [640, 320, 160, 80];
        for (chunk_size, expected_count) in PROGRAMDATA_OBSERVATION_CHUNK_SIZE_CANDIDATES_V1
            .into_iter()
            .zip(expected_at_max)
        {
            assert!(is_programdata_observation_chunk_size_candidate(chunk_size));
            assert_eq!(
                programdata_observation_chunk_count(
                    MIN_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
                    chunk_size
                ),
                Ok(1)
            );
            assert_eq!(
                programdata_observation_chunk_count(
                    MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
                    chunk_size
                ),
                Ok(expected_count)
            );
        }

        assert_eq!(
            programdata_observation_chunk_count(MIN_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 - 1, 16_384),
            Err(GovernanceError::InvalidMerkleParameters)
        );
        assert_eq!(
            programdata_observation_chunk_count(MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1, 16_384),
            Err(GovernanceError::InvalidMerkleParameters)
        );
        assert_eq!(
            programdata_observation_chunk_count(MIN_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1, 8_192),
            Err(GovernanceError::InvalidMerkleParameters)
        );
    }

    #[test]
    fn scheme_material_recomputes_the_pinned_identifier() {
        assert_eq!(
            hashv(&[PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_MATERIAL_V1]).to_bytes(),
            PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1
        );
    }

    #[test]
    fn parity_golden_vector_is_stable() {
        let subject = [0x42; 32];
        let chunk_size = PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB;
        let raw = patterned_bytes(chunk_size as usize * 2 + 17);
        let chunks = raw.chunks(chunk_size as usize).collect::<Vec<_>>();
        let leaf0 =
            programdata_observation_chunk_leaf_hash(&subject, 0, chunk_size, chunks[0]).unwrap();
        let leaf1 =
            programdata_observation_chunk_leaf_hash(&subject, 1, chunk_size, chunks[1]).unwrap();
        let leaf2 =
            programdata_observation_chunk_leaf_hash(&subject, 2, chunk_size, chunks[2]).unwrap();
        let empty3 = programdata_observation_chunk_empty_hash(&subject, 3).unwrap();
        let node1_left = programdata_observation_chunk_node_hash(1, &leaf0, &leaf1).unwrap();
        let node1_right = programdata_observation_chunk_node_hash(1, &leaf2, &empty3).unwrap();
        let root = programdata_observation_chunk_node_hash(2, &node1_left, &node1_right).unwrap();

        assert_eq!(raw.len(), 32_785);
        assert_eq!(
            hex(&leaf0),
            "67e5d9066a9b80f18154e6115f34224c69ff3f5626ec7a29d0c558566abb1ca8"
        );
        assert_eq!(
            hex(&leaf2),
            "8c5e86ca599bd47e0b8a3580268994ee6f53709c93f9066497c1866a5fbb8d76"
        );
        assert_eq!(
            hex(&empty3),
            "adc423d11767d8626164bcfb37561df0449852a3e98a54c9cbf46d041921f316"
        );
        assert_eq!(
            hex(&node1_left),
            "22f438d6c3930cba8c0c8c053a8f69b338cef2c4b56d6c49c00707bacd4222b2"
        );
        assert_eq!(
            hex(&root),
            "f025377e9a851d802a11ade6952fa734fa7af349e23d1f2098bc5ad5f4d899bb"
        );
        assert_eq!(
            programdata_observation_merkle_root(&subject, &raw, chunk_size).unwrap(),
            root
        );
    }

    #[test]
    fn typescript_golden_roots_match_exactly() {
        let mut subject = [0u8; 32];
        for (index, byte) in subject.iter_mut().enumerate() {
            *byte = index as u8;
        }
        let chunk_size = PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB;
        let first = (0..chunk_size as usize + 37)
            .map(|index| ((index * 17 + 3) & 0xff) as u8)
            .collect::<Vec<_>>();
        let second = (0..chunk_size as usize * 3)
            .map(|index| ((index * 17 + 11) & 0xff) as u8)
            .collect::<Vec<_>>();

        assert_eq!(
            hex(&programdata_observation_merkle_root(&subject, &first, chunk_size).unwrap()),
            "21d6a5fc9bb40303c13cb4c7f9c7d8cde9c6c64108afbadc75ac5602acd84a60"
        );
        assert_eq!(
            hex(&programdata_observation_merkle_root(&subject, &second, chunk_size).unwrap()),
            "1a9a553ee1ef86be2f7f9610736b6248fdd678b49b4efb0f7256857c7b167c24"
        );
    }

    #[test]
    fn first_middle_and_final_partial_leaves_bind_every_required_field() {
        let subject = [0x51; 32];
        let chunk_size = PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB;
        let raw = patterned_bytes(chunk_size as usize * 2 + 19);
        let chunks = raw.chunks(chunk_size as usize).collect::<Vec<_>>();

        for index in [0usize, 1, 2] {
            let chunk = chunks[index];
            let expected = hashv(&[
                PROGRAMDATA_OBSERVATION_CHUNK_LEAF_DOMAIN_V1,
                &subject,
                &(index as u32).to_le_bytes(),
                &(chunk.len() as u32).to_le_bytes(),
                chunk,
            ])
            .to_bytes();
            assert_eq!(
                programdata_observation_chunk_leaf_hash(&subject, index as u32, chunk_size, chunk),
                Ok(expected)
            );
        }

        assert_ne!(
            programdata_observation_chunk_leaf_hash(&subject, 0, chunk_size, chunks[0]).unwrap(),
            programdata_observation_chunk_leaf_hash(&subject, 1, chunk_size, chunks[0]).unwrap()
        );
        assert_ne!(
            programdata_observation_chunk_leaf_hash(&subject, 2, chunk_size, chunks[2]).unwrap(),
            programdata_observation_chunk_leaf_hash(&[0x52; 32], 2, chunk_size, chunks[2]).unwrap()
        );
    }

    #[test]
    fn node_level_and_empty_index_are_domain_separated() {
        let subject = [0x63; 32];
        let left = [1; 32];
        let right = [2; 32];
        assert_eq!(
            programdata_observation_chunk_node_hash(1, &left, &right).unwrap(),
            hashv(&[
                PROGRAMDATA_OBSERVATION_CHUNK_NODE_DOMAIN_V1,
                &[1],
                &left,
                &right
            ])
            .to_bytes()
        );
        assert_ne!(
            programdata_observation_chunk_node_hash(1, &left, &right).unwrap(),
            programdata_observation_chunk_node_hash(2, &left, &right).unwrap()
        );
        assert_eq!(
            programdata_observation_chunk_empty_hash(&subject, 7).unwrap(),
            hashv(&[
                PROGRAMDATA_OBSERVATION_CHUNK_EMPTY_DOMAIN_V1,
                &subject,
                &7u32.to_le_bytes()
            ])
            .to_bytes()
        );
        assert_ne!(
            programdata_observation_chunk_empty_hash(&subject, 7).unwrap(),
            programdata_observation_chunk_empty_hash(&subject, 8).unwrap()
        );
        assert_ne!(
            programdata_observation_chunk_empty_hash(&subject, 7).unwrap(),
            programdata_observation_chunk_empty_hash(&[0x64; 32], 7).unwrap()
        );
        assert_eq!(
            programdata_observation_chunk_node_hash(0, &left, &right),
            Err(GovernanceError::InvalidMerkleParameters)
        );
        assert_eq!(
            programdata_observation_chunk_node_hash(11, &left, &right),
            Err(GovernanceError::InvalidMerkleParameters)
        );
    }

    #[test]
    fn ordered_frontier_matches_full_reduction_for_every_candidate() {
        let subject = [0x75; 32];
        for chunk_size in PROGRAMDATA_OBSERVATION_CHUNK_SIZE_CANDIDATES_V1 {
            let raw = patterned_bytes(chunk_size as usize * 3 + 73);
            let expected = full_reduction_root(&subject, &raw, chunk_size);
            let direct = programdata_observation_merkle_root(&subject, &raw, chunk_size).unwrap();
            assert_eq!(direct, expected, "chunk size {chunk_size}");

            let mut frontier = ProgramDataObservationMerkleFrontierV1::default();
            for (index, chunk) in raw.chunks(chunk_size as usize).enumerate() {
                append_programdata_observation_chunk(
                    &mut frontier,
                    &subject,
                    raw.len() as u64,
                    chunk_size,
                    index as u32,
                    chunk,
                )
                .unwrap();
            }
            assert_eq!(frontier.next_index, 4);
            assert_eq!(frontier.occupied_mask, 4);
            assert_eq!(
                finalize_programdata_observation_frontier(
                    &frontier,
                    &subject,
                    raw.len() as u64,
                    chunk_size
                )
                .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn duplicate_out_of_order_and_wrong_length_fail_without_mutation() {
        let subject = [0x86; 32];
        let chunk_size = PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB;
        let raw = patterned_bytes(chunk_size as usize + 17);
        let mut frontier = ProgramDataObservationMerkleFrontierV1::default();
        let first = &raw[..chunk_size as usize];

        let before = frontier;
        assert_eq!(
            append_programdata_observation_chunk(
                &mut frontier,
                &subject,
                raw.len() as u64,
                chunk_size,
                1,
                &raw[chunk_size as usize..]
            ),
            Err(GovernanceError::InvalidStateTransition)
        );
        assert_eq!(frontier, before);

        append_programdata_observation_chunk(
            &mut frontier,
            &subject,
            raw.len() as u64,
            chunk_size,
            0,
            first,
        )
        .unwrap();
        let after_first = frontier;
        assert_eq!(
            append_programdata_observation_chunk(
                &mut frontier,
                &subject,
                raw.len() as u64,
                chunk_size,
                0,
                first
            ),
            Err(GovernanceError::InvalidStateTransition)
        );
        assert_eq!(frontier, after_first);

        assert_eq!(
            append_programdata_observation_chunk(
                &mut frontier,
                &subject,
                raw.len() as u64,
                chunk_size,
                1,
                &[9; 16]
            ),
            Err(GovernanceError::InvalidMerkleParameters)
        );
        assert_eq!(frontier, after_first);
        assert_eq!(
            finalize_programdata_observation_frontier(
                &frontier,
                &subject,
                raw.len() as u64,
                chunk_size
            ),
            Err(GovernanceError::InvalidStateTransition)
        );
    }

    #[test]
    fn padding_is_deterministic_and_subject_bound() {
        let subject = [0x97; 32];
        let raw = patterned_bytes(PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB as usize * 2 + 5);
        let first = programdata_observation_merkle_root(
            &subject,
            &raw,
            PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB,
        )
        .unwrap();
        let second = programdata_observation_merkle_root(
            &subject,
            &raw,
            PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB,
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(
            first,
            full_reduction_root(&subject, &raw, PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB)
        );
        assert_ne!(
            first,
            programdata_observation_merkle_root(
                &[0x98; 32],
                &raw,
                PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB
            )
            .unwrap()
        );
    }

    #[test]
    fn maximum_geometry_reaches_depth_ten_with_canonical_mask() {
        let subject = [0xa8; 32];
        let chunk_size = PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB;
        let chunk = [0x3d; PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB as usize];
        let mut frontier = ProgramDataObservationMerkleFrontierV1::default();
        for index in 0..MAX_PROGRAMDATA_OBSERVATION_CHUNKS_V1 {
            append_programdata_observation_chunk(
                &mut frontier,
                &subject,
                MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
                chunk_size,
                index,
                &chunk,
            )
            .unwrap();
        }
        assert_eq!(frontier.next_index, 640);
        assert_eq!(frontier.occupied_mask, 640);
        let root = finalize_programdata_observation_frontier(
            &frontier,
            &subject,
            MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
            chunk_size,
        )
        .unwrap();
        assert_ne!(root, [0; 32]);

        let mut complete = ProgramDataObservationMerkleFrontierV1::default();
        for index in 0..MAX_PADDED_PROGRAMDATA_OBSERVATION_CHUNKS_V1 {
            append_hash_at_next_index(
                &mut complete,
                programdata_observation_chunk_empty_hash(&subject, index).unwrap(),
            )
            .unwrap();
        }
        assert_eq!(complete.next_index, 1_024);
        assert_eq!(complete.occupied_mask, 1 << 10);
        assert_ne!(complete.hashes[10], [0; 32]);
        complete.validate().unwrap();
    }

    #[test]
    fn malformed_frontier_is_rejected() {
        let mut noncanonical = ProgramDataObservationMerkleFrontierV1::default();
        noncanonical.hashes[4] = [1; 32];
        assert_eq!(
            noncanonical.validate(),
            Err(GovernanceError::InvalidRelease1Account)
        );
        noncanonical.occupied_mask = 1 << 4;
        assert_eq!(
            noncanonical.validate(),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let subject = [0xb9; 32];
        assert_eq!(
            programdata_observation_chunk_empty_hash(
                &subject,
                MAX_PADDED_PROGRAMDATA_OBSERVATION_CHUNKS_V1
            ),
            Err(GovernanceError::InvalidMerkleParameters)
        );
    }
}
