use super::*;

#[derive(BorshSerialize, Clone, Debug, Default, Eq, PartialEq)]
pub struct CompressedMarketPageExpiry {
    pub id: String,
    pub label: String,
    pub settlement_ts: u64,
    pub oracle_fix_interval_hours: u16,
    pub fixes_remaining: u16,
    pub market_alpha_bps: u16,
    pub base_oracle_atomic: u64,
    pub absolute_risk_cap_usd: u64,
    pub position_cap_usd: u64,
}

impl BorshDeserialize for CompressedMarketPageExpiry {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(Self {
            id: deserialize_bounded_string(reader, MARKET_PAGE_EXPIRY_ID_MAX_BYTES, "expiry.id")?,
            label: deserialize_bounded_string(
                reader,
                MARKET_PAGE_EXPIRY_LABEL_MAX_BYTES,
                "expiry.label",
            )?,
            settlement_ts: u64::deserialize_reader(reader)?,
            oracle_fix_interval_hours: u16::deserialize_reader(reader)?,
            fixes_remaining: u16::deserialize_reader(reader)?,
            market_alpha_bps: u16::deserialize_reader(reader)?,
            base_oracle_atomic: u64::deserialize_reader(reader)?,
            absolute_risk_cap_usd: u64::deserialize_reader(reader)?,
            position_cap_usd: u64::deserialize_reader(reader)?,
        })
    }
}

impl CompressedMarketPageExpiry {
    fn deserialize_cursor(cursor: &mut CheckedCursor<'_>) -> Self {
        Self {
            id: cursor.bounded_string(MARKET_PAGE_EXPIRY_ID_MAX_BYTES),
            label: cursor.bounded_string(MARKET_PAGE_EXPIRY_LABEL_MAX_BYTES),
            settlement_ts: cursor.u64(),
            oracle_fix_interval_hours: cursor.u16(),
            fixes_remaining: cursor.u16(),
            market_alpha_bps: cursor.u16(),
            base_oracle_atomic: cursor.u64(),
            absolute_risk_cap_usd: cursor.u64(),
            position_cap_usd: cursor.u64(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum SettlementComputation {
    #[default]
    SpreadOracleIndexDelta = 1,
}
stable_borsh_enum!(SettlementComputation { SpreadOracleIndexDelta = 1 });

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SettlementObservation {
    pub observed_at_ts: u64,
    pub price_atomic: u64,
}

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
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
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

/// Current typed PDA families whose per-entity state may be carried by the shared Light envelope.
/// The numeric values are permanent compressed-address inputs and must never be reordered.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum CompressedStateDomain {
    #[default]
    OracleSkuCoverageRecord = 1,
    OracleUsdcSkuPool = 2,
    OracleUsdcSourceReward = 3,
    OracleUsdcRewardRegistration = 4,
    OracleUsdcRewardReceipt = 5,
    OracleSambaWinningVote = 6,
    OracleSambaVoteSettlementReceipt = 7,
    OracleSupportPosition = 8,
    OracleSourceState = 9,
    OracleSourceDescriptor = 10,
}
stable_borsh_enum!(CompressedStateDomain {
    OracleSkuCoverageRecord = 1,
    OracleUsdcSkuPool = 2,
    OracleUsdcSourceReward = 3,
    OracleUsdcRewardRegistration = 4,
    OracleUsdcRewardReceipt = 5,
    OracleSambaWinningVote = 6,
    OracleSambaVoteSettlementReceipt = 7,
    OracleSupportPosition = 8,
    OracleSourceState = 9,
    OracleSourceDescriptor = 10,
});

impl CompressedStateDomain {
    pub fn address_tag(self) -> u8 {
        self as u8
    }

    /// Exact compact payload length for the current compressed-state ABI. The surrounding leaf
    /// carries the canonical identity and revision, so these bytes contain only non-derivable
    /// typed state.
    pub const fn compact_data_len(self) -> usize {
        match self {
            Self::OracleSkuCoverageRecord => 44,
            Self::OracleUsdcSkuPool => 156,
            Self::OracleUsdcSourceReward => 112,
            Self::OracleUsdcRewardRegistration => 105,
            Self::OracleUsdcRewardReceipt => 81,
            Self::OracleSambaWinningVote => 16,
            Self::OracleSambaVoteSettlementReceipt => 17,
            Self::OracleSupportPosition => 107,
            Self::OracleSourceState => 205,
            Self::OracleSourceDescriptor => 96,
        }
    }
}

/// One bounded representation shared by all low-volume, high-cardinality typed state. The
/// `canonical_pda` is the logical address used by the unchanged state machine while `revision`
/// makes every mutable successor explicit.
#[derive(BorshSerialize, Clone, Debug, Default, Eq, PartialEq, LightDiscriminator)]
pub struct CompressedAmebaStateLeaf {
    pub schema_version: u8,
    pub domain: CompressedStateDomain,
    pub canonical_pda: Pubkey,
    pub revision: u64,
    pub data: Vec<u8>,
}

impl BorshDeserialize for CompressedAmebaStateLeaf {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(Self {
            schema_version: u8::deserialize_reader(reader)?,
            domain: CompressedStateDomain::deserialize_reader(reader)?,
            canonical_pda: Pubkey::deserialize_reader(reader)?,
            revision: u64::deserialize_reader(reader)?,
            data: deserialize_bounded_vec(
                reader,
                MAX_COMPRESSED_STATE_LEAF_BYTES,
                "compressed_state_data",
            )?,
        })
    }
}

impl CompressedAmebaStateLeaf {
    pub const CURRENT_SCHEMA_VERSION: u8 = 1;

    pub fn has_canonical_envelope(&self) -> bool {
        self.schema_version == Self::CURRENT_SCHEMA_VERSION
            && !crate::pubkey_is_default(&self.canonical_pda)
            && self.data.len() == self.domain.compact_data_len()
            && self.data.len() <= MAX_COMPRESSED_STATE_LEAF_BYTES
    }
}

#[derive(BorshSerialize, Clone, Debug, Default, Eq, PartialEq, LightDiscriminator)]
pub struct CompressedMarketPageLeaf {
    pub underlying_id: [u8; 32],
    pub item_id: String,
    pub name: String,
    pub symbol: String,
    pub title: String,
    pub subtitle: String,
    pub info_href: String,
    pub page_title: String,
    pub meta_description: String,
    pub price_display_decimals: u8,
    pub qty_display_decimals: u8,
    pub quote_display_decimals: u8,
    pub expiries: Vec<CompressedMarketPageExpiry>,
    pub active: bool,
}

impl BorshDeserialize for CompressedMarketPageLeaf {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(Self {
            underlying_id: <[u8; 32]>::deserialize_reader(reader)?,
            item_id: deserialize_bounded_string(reader, MARKET_PAGE_ITEM_ID_MAX_BYTES, "item_id")?,
            name: deserialize_bounded_string(reader, MARKET_PAGE_NAME_MAX_BYTES, "name")?,
            symbol: deserialize_bounded_string(reader, MARKET_PAGE_SYMBOL_MAX_BYTES, "symbol")?,
            title: deserialize_bounded_string(reader, MARKET_PAGE_TITLE_MAX_BYTES, "title")?,
            subtitle: deserialize_bounded_string(
                reader,
                MARKET_PAGE_SUBTITLE_MAX_BYTES,
                "subtitle",
            )?,
            info_href: deserialize_bounded_string(
                reader,
                MARKET_PAGE_INFO_HREF_MAX_BYTES,
                "info_href",
            )?,
            page_title: deserialize_bounded_string(
                reader,
                MARKET_PAGE_PAGE_TITLE_MAX_BYTES,
                "page_title",
            )?,
            meta_description: deserialize_bounded_string(
                reader,
                MARKET_PAGE_META_DESCRIPTION_MAX_BYTES,
                "meta_description",
            )?,
            price_display_decimals: u8::deserialize_reader(reader)?,
            qty_display_decimals: u8::deserialize_reader(reader)?,
            quote_display_decimals: u8::deserialize_reader(reader)?,
            expiries: deserialize_bounded_vec(reader, MARKET_PAGE_MAX_EXPIRIES, "expiries")?,
            active: bool::deserialize_reader(reader)?,
        })
    }
}

impl CompressedMarketPageLeaf {
    pub(crate) fn deserialize_slice(data: &mut &[u8]) -> std::io::Result<Self> {
        let mut cursor = CheckedCursor::new(data);
        let underlying_id = cursor.bytes();
        let item_id = cursor.bounded_string(MARKET_PAGE_ITEM_ID_MAX_BYTES);
        let name = cursor.bounded_string(MARKET_PAGE_NAME_MAX_BYTES);
        let symbol = cursor.bounded_string(MARKET_PAGE_SYMBOL_MAX_BYTES);
        let title = cursor.bounded_string(MARKET_PAGE_TITLE_MAX_BYTES);
        let subtitle = cursor.bounded_string(MARKET_PAGE_SUBTITLE_MAX_BYTES);
        let info_href = cursor.bounded_string(MARKET_PAGE_INFO_HREF_MAX_BYTES);
        let page_title = cursor.bounded_string(MARKET_PAGE_PAGE_TITLE_MAX_BYTES);
        let meta_description = cursor.bounded_string(MARKET_PAGE_META_DESCRIPTION_MAX_BYTES);
        let price_display_decimals = cursor.u8();
        let qty_display_decimals = cursor.u8();
        let quote_display_decimals = cursor.u8();
        let expiry_count = cursor.bounded_count(MARKET_PAGE_MAX_EXPIRIES);
        let mut expiries = Vec::with_capacity(expiry_count);
        for _ in 0..expiry_count {
            expiries.push(CompressedMarketPageExpiry::deserialize_cursor(&mut cursor));
        }
        let active = cursor.boolean();
        let remaining = cursor.finish()?;
        *data = remaining;
        Ok(Self {
            underlying_id,
            item_id,
            name,
            symbol,
            title,
            subtitle,
            info_href,
            page_title,
            meta_description,
            price_display_decimals,
            qty_display_decimals,
            quote_display_decimals,
            expiries,
            active,
        })
    }

    pub fn compute_commitment(&self) -> Result<[u8; 32], ProgramError> {
        let data = self
            .try_to_vec()
            .map_err(|_| ProgramError::InvalidInstructionData)?;
        Ok(hashv(&[b"ameba_market_page", data.as_slice()]).to_bytes())
    }
}

#[derive(BorshSerialize, Clone, Debug, Default, Eq, PartialEq, LightDiscriminator)]
pub struct CompressedSettlementLeaf {
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

impl BorshDeserialize for CompressedSettlementLeaf {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
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

impl CompressedSettlementLeaf {
    pub const CURRENT_SCHEMA_VERSION: u8 = 4;
    pub const EXPECTED_WINDOW_DAYS: u8 = 5;

    pub(crate) fn deserialize_slice(data: &mut &[u8]) -> std::io::Result<Self> {
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
            1 => SettlementComputation::SpreadOracleIndexDelta,
            _ => {
                cursor.invalid = true;
                SettlementComputation::SpreadOracleIndexDelta
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
    pub fn signed_payload_bytes(&self) -> std::io::Result<Vec<u8>> {
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

    pub fn computed_settlement_price_atomic(&self) -> Result<u64, ProgramError> {
        let observation_count = self.observations.len();
        if observation_count == 0 {
            return Err(ProgramError::InvalidInstructionData);
        }

        let sum = self
            .observations
            .iter()
            .try_fold(0u128, |accumulator, observation| {
                accumulator
                    .checked_add(observation.price_atomic as u128)
                    .ok_or(ProgramError::InvalidInstructionData)
            })?;
        let divisor = observation_count as u128;
        let rounded = sum
            .checked_add(divisor / 2)
            .ok_or(ProgramError::InvalidInstructionData)?
            / divisor;
        u64::try_from(rounded).map_err(|_| ProgramError::InvalidInstructionData)
    }

    pub fn computed_spread_oracle_index_delta_price_atomic(&self) -> Result<u64, ProgramError> {
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

    pub fn compute_commitment(&self) -> Result<[u8; 32], ProgramError> {
        let data = self
            .try_to_vec()
            .map_err(|_| ProgramError::InvalidInstructionData)?;
        Ok(hashv(&[b"ameba_settlement_leaf", data.as_slice()]).to_bytes())
    }
}
