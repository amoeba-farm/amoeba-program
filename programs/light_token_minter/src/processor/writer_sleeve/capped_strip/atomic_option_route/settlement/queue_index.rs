//! Read one queue field while preserving the SDK's complete structural checks.
//!
//! Matches light-batched-merkle-tree 0.11's output_from_account_info and
//! light-zero-copy 0.6's four ZeroCopyVecU64<[u8; 32]> views. Field offsets and
//! header size come from the SDK type; no protocol offsets are duplicated.
use super::*;
use core::mem::{align_of, offset_of, size_of};
use light_batched_merkle_tree::{
    constants::ACCOUNT_COMPRESSION_PROGRAM_ID, queue::BatchedQueueMetadata,
};
use light_compressed_account::OUTPUT_STATE_QUEUE_TYPE_V2;

const HEADER: usize = size_of::<BatchedQueueMetadata>();
const TYPE: usize = offset_of!(BatchedQueueMetadata, metadata.queue_type);
const TREE: usize = offset_of!(BatchedQueueMetadata, metadata.associated_merkle_tree);
const NEXT: usize = offset_of!(BatchedQueueMetadata, batch_metadata.next_index);

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    let mut value = [0; 8];
    value.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(value)
}

fn from_bytes(data: &[u8], tree: &Pubkey) -> Result<u64, ProgramError> {
    if data.get(..8) != Some(b"queueacc") {
        return Err(invalid());
    }
    let header = data.get(8..8 + HEADER).ok_or_else(invalid)?;
    if !(header.as_ptr() as usize).is_multiple_of(align_of::<BatchedQueueMetadata>())
        || u64_at(header, TYPE) != OUTPUT_STATE_QUEUE_TYPE_V2
        || &header[TREE..TREE + 32] != tree.as_ref()
    {
        return Err(invalid());
    }
    let mut rest = &data[8 + HEADER..];
    // Each SDK vector has an aligned [length, capacity] u64 header, followed
    // by capacity * 32 bytes. The final SDK view also permits trailing bytes.
    for _ in 0..4 {
        let metadata = rest.get(..16).ok_or_else(invalid)?;
        if !(metadata.as_ptr() as usize).is_multiple_of(align_of::<u64>()) {
            return Err(invalid());
        }
        let capacity = u64_at(metadata, 8) as usize;
        if u64_at(metadata, 0) as usize > capacity {
            return Err(invalid());
        }
        let bytes = capacity.checked_mul(32).ok_or_else(invalid)?;
        rest = rest
            .get(16..)
            .and_then(|r| r.get(bytes..))
            .ok_or_else(invalid)?;
    }
    Ok(u64_at(header, NEXT))
}

pub(super) fn read(queue: &AccountInfo, tree: &Pubkey) -> Result<u64, ProgramError> {
    if queue.owner.to_bytes() != ACCOUNT_COMPRESSION_PROGRAM_ID {
        return Err(invalid());
    }
    // Preserve the SDK's exclusive data-borrow requirement and error mapping.
    let data = queue.try_data_mut().map_err(|_| invalid())?;
    from_bytes(&data, tree)
}
