//! Settlement wire: the signer registry and rotations (tags 90-94) and the settlement leaf that
//! tag 116 publishes once its signers attest to it.

use super::*;
use crate::constants::{
    MAX_SETTLEMENT_SIGNER_COUNT, SETTLEMENT_EXPIRY_ID_MAX_BYTES, SETTLEMENT_ITEM_ID_MAX_BYTES,
    SETTLEMENT_MAX_OBSERVATIONS, SETTLEMENT_SIGNING_DOMAIN_MAX_BYTES,
    SETTLEMENT_SOURCE_URI_MAX_BYTES,
};
use solana_program::program_error::ProgramError;

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct InitializeSettlementSignerRegistryParams {
    pub signer_set_version: u64,
    pub threshold: u8,
    pub signer_count: u8,
    pub signers: [Pubkey; MAX_SETTLEMENT_SIGNER_COUNT],
    pub rotation_delay_slots: u64,
    pub recovery_authority: Pubkey,
}

crate::fixed_codec::fixed_instruction_deserialize!(InitializeSettlementSignerRegistryParams, 306, {
    signer_set_version: u64,
    threshold: u8,
    signer_count: u8,
    signers: [Pubkey; MAX_SETTLEMENT_SIGNER_COUNT],
    rotation_delay_slots: u64,
    recovery_authority: Pubkey,
});

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct ProposeSettlementSignerRotationParams {
    pub target_version: u64,
    pub threshold: u8,
    pub signer_count: u8,
    pub signers: [Pubkey; MAX_SETTLEMENT_SIGNER_COUNT],
    pub rotation_delay_slots: u64,
    /// Exact activation slot approved by governance/current signers. The processor requires this
    /// to remain at least the governed delay beyond the proposal's execution slot.
    pub activate_after_slot: u64,
}

crate::fixed_codec::fixed_instruction_deserialize!(ProposeSettlementSignerRotationParams, 282, {
    target_version: u64,
    threshold: u8,
    signer_count: u8,
    signers: [Pubkey; MAX_SETTLEMENT_SIGNER_COUNT],
    rotation_delay_slots: u64,
    activate_after_slot: u64,
});

/// How the leaf's settlement value is computed. The byte is part of the signed payload.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum SettlementComputation {
    /// `round(base × (10_000 + Δ) / 10_000)` from the month's base and index delta.
    #[default]
    OracleIndexDelta = 1,
}

impl BorshSerialize for SettlementComputation {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        (*self as u8).serialize(writer)
    }
}

impl BorshDeserialize for SettlementComputation {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        match u8::deserialize_reader(reader)? {
            1 => Ok(Self::OracleIndexDelta),
            _ => Err(io::ErrorKind::InvalidData.into()),
        }
    }
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default, Eq, PartialEq)]
pub struct SettlementObservation {
    pub observed_at_ts: u64,
    pub price_atomic: u64,
}

/// The exact bytes the settlement signers attest to (through the attestation digest).
#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct SignedSettlementPayload {
    pub signing_domain: String,
    pub schema_version: u8,
    pub oracle_month: Pubkey,
    pub recipe_hash: [u8; 32],
    pub underlying_id: [u8; 32],
    pub item_id: String,
    pub expiry_id: String,
    pub settlement_ts: u64,
    pub price_display_decimals: u8,
    pub computation: SettlementComputation,
    pub trailing_window_days: u8,
    pub observations: Vec<SettlementObservation>,
    pub settlement_price_atomic: u64,
    pub source_uri: String,
    pub source_digest: [u8; 32],
    pub base_oracle_atomic: u64,
    pub index_delta_bps: i64,
    pub signer_set_version: u64,
    pub signer_set_hash: [u8; 32],
}

impl BorshDeserialize for SignedSettlementPayload {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        Ok(Self {
            signing_domain: deserialize_bounded_string(
                reader,
                SETTLEMENT_SIGNING_DOMAIN_MAX_BYTES,
                "signing_domain",
            )?,
            schema_version: u8::deserialize_reader(reader)?,
            oracle_month: Pubkey::deserialize_reader(reader)?,
            recipe_hash: <[u8; 32]>::deserialize_reader(reader)?,
            underlying_id: <[u8; 32]>::deserialize_reader(reader)?,
            item_id: deserialize_bounded_string(reader, SETTLEMENT_ITEM_ID_MAX_BYTES, "item_id")?,
            expiry_id: deserialize_bounded_string(
                reader,
                SETTLEMENT_EXPIRY_ID_MAX_BYTES,
                "expiry_id",
            )?,
            settlement_ts: u64::deserialize_reader(reader)?,
            price_display_decimals: u8::deserialize_reader(reader)?,
            computation: SettlementComputation::deserialize_reader(reader)?,
            trailing_window_days: u8::deserialize_reader(reader)?,
            observations: deserialize_bounded_vec(
                reader,
                SETTLEMENT_MAX_OBSERVATIONS,
                "observations",
            )?,
            settlement_price_atomic: u64::deserialize_reader(reader)?,
            source_uri: deserialize_bounded_string(
                reader,
                SETTLEMENT_SOURCE_URI_MAX_BYTES,
                "source_uri",
            )?,
            source_digest: <[u8; 32]>::deserialize_reader(reader)?,
            base_oracle_atomic: u64::deserialize_reader(reader)?,
            index_delta_bps: i64::deserialize_reader(reader)?,
            signer_set_version: u64::deserialize_reader(reader)?,
            signer_set_hash: <[u8; 32]>::deserialize_reader(reader)?,
        })
    }
}

impl SignedSettlementPayload {
    pub const SIGNING_DOMAIN: &'static str = "ameba_signed_settlement_v2";
}

/// The settlement of one month, the payload of tag 116. The settlement record keeps its
/// commitment; the period keeps its value and the same commitment as its digest.
#[derive(BorshSerialize, Clone, Debug, Default, Eq, PartialEq)]
pub struct SettlementLeaf {
    pub schema_version: u8,
    pub oracle_month: Pubkey,
    pub recipe_hash: [u8; 32],
    pub underlying_id: [u8; 32],
    pub item_id: String,
    pub expiry_id: String,
    pub settlement_ts: u64,
    pub price_display_decimals: u8,
    pub computation: SettlementComputation,
    pub trailing_window_days: u8,
    pub observations: Vec<SettlementObservation>,
    pub settlement_price_atomic: u64,
    pub source_uri: String,
    pub source_digest: [u8; 32],
    pub base_oracle_atomic: u64,
    pub index_delta_bps: i64,
    pub submitted_by: Pubkey,
    pub submitted_slot: u64,
    pub signer_set_version: u64,
    pub signer_set_hash: [u8; 32],
}

impl BorshDeserialize for SettlementLeaf {
    fn deserialize(data: &mut &[u8]) -> io::Result<Self> {
        Self::deserialize_slice(data)
    }

    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        Ok(Self {
            schema_version: u8::deserialize_reader(reader)?,
            oracle_month: Pubkey::deserialize_reader(reader)?,
            recipe_hash: <[u8; 32]>::deserialize_reader(reader)?,
            underlying_id: <[u8; 32]>::deserialize_reader(reader)?,
            item_id: deserialize_bounded_string(reader, SETTLEMENT_ITEM_ID_MAX_BYTES, "item_id")?,
            expiry_id: deserialize_bounded_string(
                reader,
                SETTLEMENT_EXPIRY_ID_MAX_BYTES,
                "expiry_id",
            )?,
            settlement_ts: u64::deserialize_reader(reader)?,
            price_display_decimals: u8::deserialize_reader(reader)?,
            computation: SettlementComputation::deserialize_reader(reader)?,
            trailing_window_days: u8::deserialize_reader(reader)?,
            observations: deserialize_bounded_vec(
                reader,
                SETTLEMENT_MAX_OBSERVATIONS,
                "observations",
            )?,
            settlement_price_atomic: u64::deserialize_reader(reader)?,
            source_uri: deserialize_bounded_string(
                reader,
                SETTLEMENT_SOURCE_URI_MAX_BYTES,
                "source_uri",
            )?,
            source_digest: <[u8; 32]>::deserialize_reader(reader)?,
            base_oracle_atomic: u64::deserialize_reader(reader)?,
            index_delta_bps: i64::deserialize_reader(reader)?,
            submitted_by: Pubkey::deserialize_reader(reader)?,
            submitted_slot: u64::deserialize_reader(reader)?,
            signer_set_version: u64::deserialize_reader(reader)?,
            signer_set_hash: <[u8; 32]>::deserialize_reader(reader)?,
        })
    }
}

impl SettlementLeaf {
    pub const CURRENT_SCHEMA_VERSION: u8 = 4;
    pub const EXPECTED_WINDOW_DAYS: u8 = 5;

    /// Cursor decoder with the same acceptance as `deserialize_reader`; a short read or an
    /// out-of-bound length poisons the cursor.
    pub(crate) fn deserialize_slice(data: &mut &[u8]) -> io::Result<Self> {
        let mut cursor = CheckedCursor::new(data);
        let schema_version = cursor.u8();
        let oracle_month = cursor.pubkey();
        let recipe_hash = cursor.bytes();
        let underlying_id = cursor.bytes();
        let item_id = cursor.bounded_string(SETTLEMENT_ITEM_ID_MAX_BYTES);
        let expiry_id = cursor.bounded_string(SETTLEMENT_EXPIRY_ID_MAX_BYTES);
        let settlement_ts = cursor.u64();
        let price_display_decimals = cursor.u8();
        let computation = match cursor.u8() {
            1 => SettlementComputation::OracleIndexDelta,
            _ => {
                cursor.invalid = true;
                SettlementComputation::OracleIndexDelta
            }
        };
        let trailing_window_days = cursor.u8();
        let observation_count = cursor.bounded_count(SETTLEMENT_MAX_OBSERVATIONS);
        let mut observations = Vec::with_capacity(observation_count);
        for _ in 0..observation_count {
            observations.push(SettlementObservation {
                observed_at_ts: cursor.u64(),
                price_atomic: cursor.u64(),
            });
        }
        let settlement_price_atomic = cursor.u64();
        let source_uri = cursor.bounded_string(SETTLEMENT_SOURCE_URI_MAX_BYTES);
        let source_digest = cursor.bytes();
        let base_oracle_atomic = cursor.u64();
        let index_delta_bps = cursor.i64();
        let submitted_by = cursor.pubkey();
        let submitted_slot = cursor.u64();
        let signer_set_version = cursor.u64();
        let signer_set_hash = cursor.bytes();
        let remaining = cursor.finish()?;
        *data = remaining;
        Ok(Self {
            schema_version,
            oracle_month,
            recipe_hash,
            underlying_id,
            item_id,
            expiry_id,
            settlement_ts,
            price_display_decimals,
            computation,
            trailing_window_days,
            observations,
            settlement_price_atomic,
            source_uri,
            source_digest,
            base_oracle_atomic,
            index_delta_bps,
            submitted_by,
            submitted_slot,
            signer_set_version,
            signer_set_hash,
        })
    }

    pub fn to_signed_payload(&self) -> SignedSettlementPayload {
        SignedSettlementPayload {
            signing_domain: SignedSettlementPayload::SIGNING_DOMAIN.to_string(),
            schema_version: self.schema_version,
            oracle_month: self.oracle_month,
            recipe_hash: self.recipe_hash,
            underlying_id: self.underlying_id,
            item_id: self.item_id.clone(),
            expiry_id: self.expiry_id.clone(),
            settlement_ts: self.settlement_ts,
            price_display_decimals: self.price_display_decimals,
            computation: self.computation,
            trailing_window_days: self.trailing_window_days,
            observations: self.observations.clone(),
            settlement_price_atomic: self.settlement_price_atomic,
            source_uri: self.source_uri.clone(),
            source_digest: self.source_digest,
            base_oracle_atomic: self.base_oracle_atomic,
            index_delta_bps: self.index_delta_bps,
            signer_set_version: self.signer_set_version,
            signer_set_hash: self.signer_set_hash,
        }
    }

    #[inline(never)]
    pub fn signed_payload_bytes(&self) -> io::Result<Vec<u8>> {
        let mut encoded = Vec::new();
        SignedSettlementPayload::SIGNING_DOMAIN.serialize(&mut encoded)?;
        self.schema_version.serialize(&mut encoded)?;
        self.oracle_month.serialize(&mut encoded)?;
        self.recipe_hash.serialize(&mut encoded)?;
        self.underlying_id.serialize(&mut encoded)?;
        self.item_id.serialize(&mut encoded)?;
        self.expiry_id.serialize(&mut encoded)?;
        self.settlement_ts.serialize(&mut encoded)?;
        self.price_display_decimals.serialize(&mut encoded)?;
        self.computation.serialize(&mut encoded)?;
        self.trailing_window_days.serialize(&mut encoded)?;
        self.observations.serialize(&mut encoded)?;
        self.settlement_price_atomic.serialize(&mut encoded)?;
        self.source_uri.serialize(&mut encoded)?;
        self.source_digest.serialize(&mut encoded)?;
        self.base_oracle_atomic.serialize(&mut encoded)?;
        self.index_delta_bps.serialize(&mut encoded)?;
        self.signer_set_version.serialize(&mut encoded)?;
        self.signer_set_hash.serialize(&mut encoded)?;
        Ok(encoded)
    }

    pub fn signing_message(&self) -> Result<Vec<u8>, ProgramError> {
        self.signed_payload_bytes()
            .map_err(|_| ProgramError::InvalidInstructionData)
    }

    /// The value this leaf must carry: the index-delta settlement of its base and delta.
    pub fn computed_index_delta_price_atomic(&self) -> Result<u64, ProgramError> {
        if self.base_oracle_atomic == 0 {
            return Err(ProgramError::InvalidInstructionData);
        }
        let factor_bps = 10_000i128
            .checked_add(i128::from(self.index_delta_bps))
            .ok_or(ProgramError::InvalidInstructionData)?;
        if factor_bps < 0 {
            return Err(ProgramError::InvalidInstructionData);
        }
        let scaled = (self.base_oracle_atomic as i128)
            .checked_mul(factor_bps)
            .ok_or(ProgramError::InvalidInstructionData)?;
        let rounded = scaled
            .checked_add(5_000)
            .ok_or(ProgramError::InvalidInstructionData)?
            / 10_000;
        u64::try_from(rounded).map_err(|_| ProgramError::InvalidInstructionData)
    }
}

/// Tag 116 payload.
#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct UpsertSettlementParams {
    pub settlement: SettlementLeaf,
}

impl BorshDeserialize for UpsertSettlementParams {
    fn deserialize(data: &mut &[u8]) -> io::Result<Self> {
        Ok(Self {
            settlement: SettlementLeaf::deserialize_slice(data)?,
        })
    }

    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        Ok(Self {
            settlement: SettlementLeaf::deserialize_reader(reader)?,
        })
    }
}
