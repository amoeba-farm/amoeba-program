//! Decode the unchanged resident Borsh layout directly from its account slice.
//!
//! The generic reader remains the derive-equivalent reference. On-chain slice
//! reads use each nested state's checked fixed decoder, avoiding an additional
//! allocation and byte copy for every Pool, Book, record, page and position.
use super::*;
use crate::fixed_codec::deserialize_slice_bytes;

pub(super) fn market(mut input: &[u8]) -> Result<Market, ProgramError> {
    let value = Market::deserialize(&mut input).map_err(|_| invalid())?;
    if !input.is_empty() {
        return Err(invalid());
    }
    Ok(value)
}

#[inline(never)]
fn read_u64(input: &mut &[u8]) -> std::io::Result<u64> {
    Ok(u64::from_le_bytes(deserialize_slice_bytes(input)?))
}

fn read_optional<T: BorshDeserialize>(input: &mut &[u8]) -> std::io::Result<Option<T>> {
    match deserialize_slice_bytes::<1>(input)?[0] {
        0 => Ok(None),
        1 => T::deserialize(input).map(Some),
        _ => Err(crate::fixed_codec::invalid_fixed_borsh()),
    }
}

fn read_rows<T: BorshDeserialize>(input: &mut &[u8]) -> std::io::Result<Vec<T>> {
    let len = u32::from_le_bytes(deserialize_slice_bytes(input)?);
    if len == 0 {
        return Ok(Vec::new());
    }
    // These rows are fixed, non-zero-size state types. Match Borsh's cautious
    // initial capacity so an untrusted length never forces a large allocation.
    let size = core::mem::size_of::<T>().max(1);
    let capacity = ((len as usize).min(4096 / size)).max(1);
    let mut rows = Vec::with_capacity(capacity);
    for _ in 0..len {
        rows.push(T::deserialize(input)?);
    }
    Ok(rows)
}

impl BorshDeserialize for ResidentRouterState {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(Self {
            pool: BorshDeserialize::deserialize_reader(reader)?,
            book: BorshDeserialize::deserialize_reader(reader)?,
            records: BorshDeserialize::deserialize_reader(reader)?,
            bin_pages: BorshDeserialize::deserialize_reader(reader)?,
            writer_position: BorshDeserialize::deserialize_reader(reader)?,
            pool_option: BorshDeserialize::deserialize_reader(reader)?,
            pool_quote: BorshDeserialize::deserialize_reader(reader)?,
            book_option: BorshDeserialize::deserialize_reader(reader)?,
            book_quote: BorshDeserialize::deserialize_reader(reader)?,
            pool_hot_option: BorshDeserialize::deserialize_reader(reader)?,
            pool_hot_quote: BorshDeserialize::deserialize_reader(reader)?,
            book_hot_option: BorshDeserialize::deserialize_reader(reader)?,
            book_hot_quote: BorshDeserialize::deserialize_reader(reader)?,
        })
    }

    #[inline(never)]
    fn deserialize(input: &mut &[u8]) -> std::io::Result<Self> {
        Ok(Self {
            pool: AmoebaDlmmPoolV1::deserialize(input)?,
            book: read_optional(input)?,
            records: read_rows(input)?,
            bin_pages: read_rows(input)?,
            writer_position: read_optional(input)?,
            pool_option: read_u64(input)?,
            pool_quote: read_u64(input)?,
            book_option: read_u64(input)?,
            book_quote: read_u64(input)?,
            pool_hot_option: read_u64(input)?,
            pool_hot_quote: read_u64(input)?,
            book_hot_option: read_u64(input)?,
            book_hot_quote: read_u64(input)?,
        })
    }

    fn try_from_slice(mut input: &[u8]) -> std::io::Result<Self> {
        let value = Self::deserialize(&mut input)?;
        if !input.is_empty() {
            return Err(crate::fixed_codec::invalid_fixed_borsh());
        }
        Ok(value)
    }
}
