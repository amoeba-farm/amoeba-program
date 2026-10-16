//! Instruction wire. Every instruction is one tag byte followed by an exact-length payload;
//! truncated and trailing bytes reject. Oversized data rejects before the tag is classified, and
//! an unknown tag fails before any payload decoding or account access.

use std::io;

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

use crate::constants::{
    MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS, MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH,
    ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES,
};
use crate::fixed_codec::CheckedCursor;
use crate::state::{
    OracleEconomicParams, OracleEscrowKind, OracleOpeningChallengeOutcome,
    OracleSourceChallengeOutcome, OracleUpdateClaimOutcome, OracleUsdcRewardKind,
};

fn invalid_bounded_length(_field: &'static str, _actual: usize, _maximum: usize) -> io::Error {
    io::ErrorKind::InvalidData.into()
}

fn deserialize_bounded_vec<R: io::Read, T: BorshDeserialize>(
    reader: &mut R,
    maximum: usize,
    field: &'static str,
) -> io::Result<Vec<T>> {
    let length = usize::try_from(u32::deserialize_reader(reader)?)
        .map_err(|_| invalid_bounded_length(field, usize::MAX, maximum))?;
    if length > maximum {
        return Err(invalid_bounded_length(field, length, maximum));
    }
    let mut values = Vec::with_capacity(length);
    for _ in 0..length {
        values.push(T::deserialize_reader(reader)?);
    }
    Ok(values)
}

fn deserialize_bounded_string<R: io::Read>(
    reader: &mut R,
    maximum: usize,
    field: &'static str,
) -> io::Result<String> {
    let bytes = deserialize_bounded_vec::<R, u8>(reader, maximum, field)?;
    String::from_utf8(bytes).map_err(|_| io::ErrorKind::InvalidData.into())
}

fn deserialize_bounded_bytes32_vec_cursor(
    cursor: &mut CheckedCursor<'_>,
    maximum: usize,
) -> Vec<[u8; 32]> {
    let length = cursor.u32() as usize;
    if length > maximum {
        cursor.invalid = true;
        return Vec::new();
    }
    let mut values = Vec::with_capacity(length);
    for _ in 0..length {
        values.push(cursor.bytes());
    }
    values
}

fn invalid_instruction_tag_error(_tag: u8) -> io::Error {
    io::ErrorKind::InvalidData.into()
}

#[cfg(not(target_os = "solana"))]
mod builders;
mod carry;
mod cash;
mod cfm;
mod config;
mod deserialize;
#[cfg(any(feature = "localnet-dev-authority", feature = "localnet-governance"))]
mod dev;
mod model;
mod month;
pub(crate) mod payloads;
mod serialize;
mod settlement;
mod tags;
mod work;

#[cfg(not(target_os = "solana"))]
pub use builders::*;
pub use carry::*;
pub use cash::*;
pub use cfm::*;
pub use config::*;
#[cfg(any(feature = "localnet-dev-authority", feature = "localnet-governance"))]
pub use dev::*;
pub use model::*;
pub use month::*;
pub use settlement::*;
pub use tags::*;
pub use work::*;
