//! The explicitly versioned router suffix of a current Market account. The original Market
//! bytes stay canonical. Ordinary market mutations write that prefix and retain router state.
use crate::{state::Market, ProgramError};
use core::ops::Range;

pub const MAGIC: [u8; 8] = *b"MKTRTR01";
pub const VERSION: u8 = 1;
pub const PREFIX_LEN: usize = Market::LEN;
pub const HEADER_LEN: usize = 8 + 1 + 4;
pub const PAYLOAD_OFFSET: usize = PREFIX_LEN + HEADER_LEN;

fn invalid() -> ProgramError {
    ProgramError::InvalidAccountData
}

// Keep strict whole-allocation validation without an SBF byte loop over spare
// account capacity. A fixed read-only buffer also avoids allocating from the
// transaction heap, which is shared with the authenticated resident payload.
fn zero_padding(data: &[u8]) -> bool {
    static ZEROES: [u8; 1024] = [0; 1024];
    data.chunks(ZEROES.len())
        .all(|chunk| solana_program::program_memory::sol_memcmp(chunk, &ZEROES, chunk.len()) == 0)
}

/// Validate the entire allocation envelope. The router separately decodes its typed payload.
/// An unextended Market keeps its exact original allocation. Unknown trailers are rejected.
pub fn payload_range(data: &[u8]) -> Result<Option<Range<usize>>, ProgramError> {
    if data.len() == PREFIX_LEN {
        return Ok(None);
    }
    if data.len() <= PAYLOAD_OFFSET
        || data.get(PREFIX_LEN..PREFIX_LEN + 8) != Some(MAGIC.as_slice())
        || data[PREFIX_LEN + 8] != VERSION
    {
        return Err(invalid());
    }
    let length = u32::from_le_bytes(
        data[PREFIX_LEN + 9..PAYLOAD_OFFSET]
            .try_into()
            .map_err(|_| invalid())?,
    ) as usize;
    let end = PAYLOAD_OFFSET.checked_add(length).ok_or_else(invalid)?;
    if length == 0 || end > data.len() || !zero_padding(&data[end..]) {
        return Err(invalid());
    }
    Ok(Some(PAYLOAD_OFFSET..end))
}

/// The strict original Market slice, after checking any known extension and unused allocation.
pub fn prefix(data: &[u8]) -> Result<&[u8], ProgramError> {
    payload_range(data)?;
    data.get(..PREFIX_LEN).ok_or_else(invalid)
}

/// Write one complete resident payload without touching Market accounting or pause state.
/// Allocation/rent and the payload's authority are checked by the governed router handler.
pub fn write_payload(data: &mut [u8], payload: &[u8]) -> Result<(), ProgramError> {
    let length = u32::try_from(payload.len()).map_err(|_| invalid())?;
    let end = PAYLOAD_OFFSET
        .checked_add(payload.len())
        .ok_or_else(invalid)?;
    if payload.is_empty() || end > data.len() {
        return Err(invalid());
    }
    data[PREFIX_LEN..PREFIX_LEN + 8].copy_from_slice(&MAGIC);
    data[PREFIX_LEN + 8] = VERSION;
    data[PREFIX_LEN + 9..PAYLOAD_OFFSET].copy_from_slice(&length.to_le_bytes());
    data[PAYLOAD_OFFSET..end].copy_from_slice(payload);
    let unused = data.len() - end;
    solana_program::program_memory::sol_memset(&mut data[end..], 0, unused);
    Ok(())
}
