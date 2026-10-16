//! Program-owned account layouts. The consumer-facing `Period` layout lives in the interface
//! crate; everything here is internal and versioned by its own discriminator.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{hash::hashv, pubkey::Pubkey};

use crate::constants::{
    CURRENT_STATE_NAMESPACE_SEED, MAX_SETTLEMENT_SIGNER_COUNT, ORACLE_BUCKET_MEDIAN_PDA_SEED,
    ORACLE_RECIPE_WEIGHT_MANIFEST_PDA_SEED, ORACLE_SETTLEMENT_SOURCE_MANIFEST_PDA_SEED,
    SETTLEMENT_SIGNER_REGISTRY_PDA_SEED, SETTLEMENT_SIGNER_SET_PDA_SEED,
};

/// Borsh derives encode fieldless enums by declaration order. Current state uses explicit bytes so
/// reordering source code cannot reinterpret current accounts.
macro_rules! stable_borsh_enum {
    ($name:ident { $first:ident = $first_value:expr $(, $variant:ident = $value:expr)* $(,)? }) => {
        stable_borsh_enum!(@impl $name, $first, { $first = $first_value $(, $variant = $value)* });
    };
    (@impl $name:ident, $fallback:ident, { $($variant:ident = $value:expr),+ }) => {
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

        impl crate::fixed_codec::CursorField for $name {
            /// Same tag mapping as `deserialize_reader`; an unknown tag poisons the cursor.
            #[inline(never)]
            fn read(input: &mut crate::fixed_codec::CheckedCursor<'_>) -> Self {
                match input.u8() {
                    $($value => Self::$variant,)+
                    _ => {
                        input.invalid = true;
                        Self::$fallback
                    }
                }
            }
        }
    };
}

mod config;
mod custody;
mod manifests;
mod median;
mod oracle_activity;
mod oracle_common;
mod oracle_membership;
mod rewards;
mod settlement;
mod sku;

pub use config::*;
pub use custody::*;
pub use manifests::*;
pub use median::*;
pub use oracle_activity::*;
pub use oracle_common::*;
pub use oracle_membership::*;
pub use rewards::*;
pub use settlement::*;
pub use sku::*;

#[cfg(test)]
mod tests;
