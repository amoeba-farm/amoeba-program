use borsh::{BorshDeserialize, BorshSerialize};
use light_sdk::LightDiscriminator;
use solana_program::{hash::hashv, program_error::ProgramError, pubkey::Pubkey};

use crate::constants::{
    CURRENT_STATE_NAMESPACE_SEED, MARKET_PAGE_EXPIRY_ID_MAX_BYTES,
    MARKET_PAGE_EXPIRY_LABEL_MAX_BYTES, MARKET_PAGE_INFO_HREF_MAX_BYTES,
    MARKET_PAGE_ITEM_ID_MAX_BYTES, MARKET_PAGE_MAX_EXPIRIES,
    MARKET_PAGE_META_DESCRIPTION_MAX_BYTES, MARKET_PAGE_NAME_MAX_BYTES,
    MARKET_PAGE_PAGE_TITLE_MAX_BYTES, MARKET_PAGE_SUBTITLE_MAX_BYTES, MARKET_PAGE_SYMBOL_MAX_BYTES,
    MARKET_PAGE_TITLE_MAX_BYTES, MAX_COMPRESSED_STATE_LEAF_BYTES, MAX_SETTLEMENT_SIGNER_COUNT,
    ORACLE_BUCKET_MEDIAN_PDA_SEED, ORACLE_RECIPE_WEIGHT_MANIFEST_PDA_SEED,
    ORACLE_SETTLEMENT_SOURCE_MANIFEST_PDA_SEED, SETTLEMENT_EXPIRY_ID_MAX_BYTES,
    SETTLEMENT_ITEM_ID_MAX_BYTES, SETTLEMENT_MAX_OBSERVATIONS, SETTLEMENT_SIGNER_REGISTRY_PDA_SEED,
    SETTLEMENT_SIGNER_SET_PDA_SEED, SETTLEMENT_SIGNING_DOMAIN_MAX_BYTES,
    SETTLEMENT_SOURCE_URI_MAX_BYTES,
};
use crate::fixed_codec::CheckedCursor;

fn invalid_bounded_length(_field: &'static str, _actual: usize, _maximum: usize) -> std::io::Error {
    std::io::ErrorKind::InvalidData.into()
}

fn deserialize_bounded_vec<R: std::io::Read, T: BorshDeserialize>(
    reader: &mut R,
    maximum: usize,
    field: &'static str,
) -> std::io::Result<Vec<T>> {
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

fn deserialize_bounded_string<R: std::io::Read>(
    reader: &mut R,
    maximum: usize,
    field: &'static str,
) -> std::io::Result<String> {
    let bytes = deserialize_bounded_vec::<R, u8>(reader, maximum, field)?;
    String::from_utf8(bytes).map_err(|_| std::io::ErrorKind::InvalidData.into())
}

/// Borsh derives encode fieldless enums by declaration order. Current state uses explicit bytes so
/// reordering source code cannot reinterpret current accounts.
macro_rules! stable_borsh_enum {
    ($name:ident { $($variant:ident = $value:expr),+ $(,)? }) => {
        impl BorshSerialize for $name {
            #[inline(always)]
            fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
                let value: u8 = match self { $(Self::$variant => $value),+ };
                BorshSerialize::serialize(&value, writer)
            }
        }

        impl BorshDeserialize for $name {
            #[inline(always)]
            fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
                match u8::deserialize_reader(reader)? {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(std::io::ErrorKind::InvalidData.into()),
                }
            }
        }
    };
}

mod compressed;
mod emergency;
mod manifests;
mod market;
mod median;
mod oracle_activity;
mod oracle_common;
mod rewards;
mod sku;
mod staking;
mod vault;
mod writer_sleeve;

pub use compressed::*;
pub use emergency::*;
pub use manifests::*;
pub use market::*;
pub use median::*;
pub use oracle_activity::*;
pub use oracle_common::*;
pub use rewards::*;
pub use sku::*;
pub use staking::*;
pub use vault::*;
pub use writer_sleeve::*;
