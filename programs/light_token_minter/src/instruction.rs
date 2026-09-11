pub use crate::writer_dlmm_instruction::{BeginWriterDlmmPolicyV1Params, ManageWriterDlmmV1Params};
use std::io;

use borsh::{BorshDeserialize, BorshSerialize};
use light_sdk::{
    instruction::{
        account_meta::{CompressedAccountMeta, CompressedAccountMetaReadOnly},
        PackedAddressTreeInfo, PackedStateTreeInfo,
    },
    proof::borsh_compat::{CompressedProof, ValidityProof},
};
use solana_program::pubkey::Pubkey;

use crate::constants::{
    MAX_COMPRESSED_INNER_INSTRUCTION_BYTES, MAX_COMPRESSED_STATE_SESSION_RECORDS,
    MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS, MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH,
    MAX_SETTLEMENT_SIGNER_COUNT, ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES,
};
use crate::fixed_codec::CheckedCursor;
use crate::state::{
    CompressedAmebaStateLeaf, CompressedMarketPageLeaf, CompressedSettlementLeaf,
    CompressedStateDomain, InstrumentDefinition, MarketParameters, OracleEconomicParams,
    OracleEmergencyDisputeKind, OracleEscrowKind, OracleOpeningChallengeOutcome,
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

pub(crate) fn deserialize_validity_proof(data: &mut &[u8]) -> io::Result<ValidityProof> {
    let mut cursor = CheckedCursor::new(data);
    let proof = deserialize_validity_proof_cursor(&mut cursor);
    *data = cursor.finish()?;
    Ok(proof)
}

#[inline(never)]
pub(crate) fn deserialize_validity_proof_cursor(cursor: &mut CheckedCursor<'_>) -> ValidityProof {
    match cursor.u8() {
        0 => ValidityProof(None),
        1 => ValidityProof(Some(CompressedProof {
            a: cursor.bytes(),
            b: cursor.bytes(),
            c: cursor.bytes(),
        })),
        _ => {
            cursor.invalid = true;
            ValidityProof(None)
        }
    }
}

#[inline(never)]
pub(crate) fn deserialize_packed_state_tree_info_cursor(
    cursor: &mut CheckedCursor<'_>,
) -> PackedStateTreeInfo {
    PackedStateTreeInfo {
        root_index: cursor.u16(),
        prove_by_index: cursor.boolean(),
        merkle_tree_pubkey_index: cursor.u8(),
        queue_pubkey_index: cursor.u8(),
        leaf_index: cursor.u32(),
    }
}

pub(crate) fn deserialize_compressed_account_meta(
    data: &mut &[u8],
) -> io::Result<CompressedAccountMeta> {
    let mut cursor = CheckedCursor::new(data);
    let value = CompressedAccountMeta {
        tree_info: deserialize_packed_state_tree_info_cursor(&mut cursor),
        address: cursor.bytes(),
        output_state_tree_index: cursor.u8(),
    };
    *data = cursor.finish()?;
    Ok(value)
}

fn invalid_instruction_tag_error(_tag: u8) -> io::Error {
    io::ErrorKind::InvalidData.into()
}

mod compression;
mod deserialize;
mod market;
mod model;
mod oracle_carry;
mod oracle_cash;
mod oracle_work;
mod serialize;
mod tags;
mod writer_sleeve;

pub use compression::*;
pub use market::*;
pub use model::*;
pub use oracle_carry::*;
pub use oracle_cash::*;
pub use oracle_work::*;
pub use tags::*;
pub use writer_sleeve::*;
