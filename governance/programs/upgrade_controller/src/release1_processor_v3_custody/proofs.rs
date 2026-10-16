use super::*;

pub(super) fn exact_region_chunk(
    payload: &[u8],
    region_start: u64,
    region_length: u64,
    chunk_size: u32,
    chunk_index: u32,
) -> Result<&[u8], ProgramError> {
    let relative_start = u64::from(chunk_index)
        .checked_mul(u64::from(chunk_size))
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let remaining = region_length
        .checked_sub(relative_start)
        .ok_or(GovernanceError::InvalidMerkleProof)?;
    let length = remaining.min(u64::from(chunk_size));
    if length == 0 {
        return Err(GovernanceError::InvalidMerkleProof.into());
    }
    let start = region_start
        .checked_add(relative_start)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let end = start
        .checked_add(length)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let start = usize::try_from(start).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let end = usize::try_from(end).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    payload
        .get(start..end)
        .ok_or_else(|| GovernanceError::InvalidMerkleProof.into())
}

pub(super) fn validate_expected_leaf_proof(
    expected_root: &[u8; 32],
    artifact_length: u64,
    chunk_size: u32,
    chunk_index: u32,
    expected_leaf: &[u8; 32],
    proof_len: u8,
    proof_nodes: &[[u8; 32]; MAX_ARTIFACT_PROOF_DEPTH_V1],
) -> ProgramResult {
    let chunk_count = artifact_chunk_count(artifact_length, chunk_size)?;
    if chunk_index >= chunk_count || *expected_leaf == [0; 32] {
        return Err(GovernanceError::InvalidMerkleProof.into());
    }
    let padded_count = (chunk_count as usize).next_power_of_two();
    let expected_depth = padded_count.trailing_zeros() as usize;
    let proof_len = usize::from(proof_len);
    if proof_len != expected_depth
        || proof_len > MAX_ARTIFACT_PROOF_DEPTH_V1
        || proof_nodes[proof_len..].iter().any(|node| *node != [0; 32])
    {
        return Err(GovernanceError::InvalidMerkleProof.into());
    }
    let mut current = *expected_leaf;
    let mut index = chunk_index;
    for (level, sibling) in proof_nodes[..proof_len].iter().enumerate() {
        validate_padding_sibling(sibling, index, level, chunk_count as usize, padded_count)?;
        current = if index & 1 == 0 {
            artifact_chunk_node_hash(&current, sibling)
        } else {
            artifact_chunk_node_hash(sibling, &current)
        };
        index >>= 1;
    }
    if current != *expected_root {
        return Err(GovernanceError::InvalidMerkleProof.into());
    }
    Ok(())
}

fn validate_padding_sibling(
    sibling: &[u8; 32],
    node_index: u32,
    proof_level: usize,
    chunk_count: usize,
    padded_count: usize,
) -> ProgramResult {
    if proof_level >= MAX_ARTIFACT_PROOF_DEPTH_V1 {
        return Err(GovernanceError::InvalidMerkleProof.into());
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
        return Err(GovernanceError::InvalidMerkleProof.into());
    }
    if sibling_start >= chunk_count
        && *sibling != padding_subtree_hash(sibling_start, sibling_leaf_count)?
    {
        return Err(GovernanceError::InvalidMerkleProof.into());
    }
    Ok(())
}

fn padding_subtree_hash(
    padded_start: usize,
    padded_leaf_count: usize,
) -> Result<[u8; 32], ProgramError> {
    let padded_end = padded_start
        .checked_add(padded_leaf_count)
        .ok_or(GovernanceError::InvalidMerkleProof)?;
    if padded_leaf_count == 0
        || !padded_leaf_count.is_power_of_two()
        || padded_start % padded_leaf_count != 0
        || padded_end > MAX_PADDED_ARTIFACT_CHUNKS_V1
    {
        return Err(GovernanceError::InvalidMerkleProof.into());
    }
    let mut level = Vec::with_capacity(padded_leaf_count);
    for index in padded_start..padded_end {
        level.push(artifact_chunk_empty_hash(
            u32::try_from(index).map_err(|_| GovernanceError::InvalidMerkleProof)?,
        )?);
    }
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len() / 2);
        for pair in level.chunks_exact(2) {
            next.push(artifact_chunk_node_hash(&pair[0], &pair[1]));
        }
        level = next;
    }
    level
        .pop()
        .ok_or_else(|| GovernanceError::InvalidMerkleProof.into())
}

pub(super) fn programdata_zero_tail_chunk_hash(
    chunk_index: u32,
    exact_chunk: &[u8],
) -> Result<[u8; 32], ProgramError> {
    if exact_chunk.is_empty() || exact_chunk.len() > 16_384 {
        return Err(GovernanceError::InvalidMerkleParameters.into());
    }
    let actual_length =
        u32::try_from(exact_chunk.len()).map_err(|_| GovernanceError::InvalidMerkleParameters)?;
    Ok(hashv(&[
        PROGRAMDATA_ZERO_TAIL_CHUNK_DOMAIN_V1,
        &chunk_index.to_le_bytes(),
        &actual_length.to_le_bytes(),
        exact_chunk,
    ])
    .to_bytes())
}

pub(super) fn programdata_zero_tail_zero_hash(
    chunk_index: u32,
    actual_length: usize,
) -> Result<[u8; 32], ProgramError> {
    if actual_length == 0 || actual_length > 16_384 {
        return Err(GovernanceError::InvalidMerkleParameters.into());
    }
    let length_bytes = u32::try_from(actual_length)
        .map_err(|_| GovernanceError::InvalidMerkleParameters)?
        .to_le_bytes();
    let index_bytes = chunk_index.to_le_bytes();
    let mut slices: [&[u8]; MAX_PROGRAMDATA_ZERO_HASH_BLOCKS_V1 + 4] =
        [&[]; MAX_PROGRAMDATA_ZERO_HASH_BLOCKS_V1 + 4];
    slices[0] = PROGRAMDATA_ZERO_TAIL_CHUNK_DOMAIN_V1;
    slices[1] = &index_bytes;
    slices[2] = &length_bytes;
    let full_blocks = actual_length / PROGRAMDATA_ZERO_HASH_BLOCK_BYTES_V1;
    let remainder = actual_length % PROGRAMDATA_ZERO_HASH_BLOCK_BYTES_V1;
    for slice in &mut slices[3..3 + full_blocks] {
        *slice = &PROGRAMDATA_ZERO_HASH_BLOCK_V1;
    }
    let count = if remainder == 0 {
        3 + full_blocks
    } else {
        slices[3 + full_blocks] = &PROGRAMDATA_ZERO_HASH_BLOCK_V1[..remainder];
        4 + full_blocks
    };
    Ok(hashv(&slices[..count]).to_bytes())
}
