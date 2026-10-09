use super::*;
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, PartialEq, BorshSerialize)]
    pub struct ExtendOctoberLadderParams {
        pub product: u8,
        pub next_policy_version: u64,
        pub expected_book_digest: [u8; 32],
        pub expected_policy_hash: [u8; 32],
        pub beta_ppm: u32,
        pub lambda_ppm: u64,
        pub model_margin_vector_hash: [u8; 32],
        pub execution_cost_vector_hash: [u8; 32],
        /// Eight new price pairs: CALL06..09 then PUT06..09.
        pub series_prices: [u8; 128],
    }
}
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, PartialEq, BorshSerialize)]
    pub struct InstallOctoberLadderParams {
        pub product: u8,
        pub next_policy_version: u64,
        pub expected_book_digest: [u8; 32],
        pub expected_policy_hash: [u8; 32],
        pub beta_ppm: u32,
        pub lambda_ppm: u64,
        pub model_margin_vector_hash: [u8; 32],
        pub execution_cost_vector_hash: [u8; 32],
        /// Ten pairs of little-endian u64 claim values and seller floors, in
        /// CALL01..05, PUT01..05 order. Buyback caps retain each old side's caps.
        pub series_prices: [u8; 160],
    }
}
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, PartialEq, BorshSerialize)]
    pub struct TerminalCleanupParams {
        pub kind: u8,
        pub bucket_id: [u8; 32],
        pub expected_data_hash: [u8; 32],
        pub expected_lamports: u64,
    }
}
#[derive(Clone, Debug, PartialEq, BorshSerialize, BorshDeserialize)]
// Keep this wire action allocation free.
#[allow(clippy::large_enum_variant)]
pub enum OracleSponsoredActionV1 {
    Activate {
        terms: crate::oracle_sponsorship::SponsorTerms,
        source: ProposeOracleSourceV3Params,
    },
    Reconcile,
    ClaimBounty,
    WithdrawReserve {
        amount: u64,
    },
}
/// Cursor reader with the derive's tags (declaration order 0..=3).
impl crate::fixed_codec::CursorField for OracleSponsoredActionV1 {
    #[inline(never)]
    fn read(input: &mut CheckedCursor<'_>) -> Self {
        use crate::fixed_codec::CursorField;
        match input.u8() {
            0 => Self::Activate {
                terms: CursorField::read(input),
                source: CursorField::read(input),
            },
            1 => Self::Reconcile,
            2 => Self::ClaimBounty,
            3 => Self::WithdrawReserve {
                amount: input.u64(),
            },
            _ => {
                input.invalid = true;
                Self::Reconcile
            }
        }
    }
}

crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, PartialEq, BorshSerialize)]
    pub struct OracleCouncilActionV1 {
        pub operation: u8,
        pub kind: OracleEmergencyDisputeKind,
        pub target_id: [u8; 32],
        pub expected_case_hash: [u8; 32],
        pub expected_epoch: u64,
        pub expected_seats_hash: [u8; 32],
        pub choice: u8,
    }
}

/// Bounded carry-forward subactions. The import proof count is one byte, not an
/// unbounded Borsh Vec prefix; the wire shape matches the client carry builders.
#[derive(Clone, Debug, PartialEq)]
pub enum OracleCarryForwardActionV1 {
    /// Historical setup payloads retained for archived wire construction.
    /// Their retired selectors are rejected by every decoder.
    ExtendOctoberLadder(ExtendOctoberLadderParams),
    InstallOctoberLadder(InstallOctoberLadderParams),
    TerminalCleanup(TerminalCleanupParams),
    SponsoredSource(OracleSponsoredActionV1),
    /// September-only council bootstrap: begin, append one parent, finish. Not part of the
    /// mainnet-v3 artifact: the window closed 2026-10-01.
    #[cfg(not(feature = "mainnet-v3"))]
    SeptemberBootstrap {
        operation: u8,
        market: u8,
        row: u8,
        plan_hash: [u8; 32],
    },
    /// October council bootstrap. Not part of the mainnet-v3 artifact: all four cohorts finished
    /// on Mainnet on 2026-10-03 and the window closes 2026-11-01.
    #[cfg(not(feature = "mainnet-v3"))]
    OctoberBootstrap {
        operation: u8,
        market: u8,
        row: u8,
        plan_hash: [u8; 32],
    },
    /// Retired September-only setup wire; decoding rejects selector 20.
    InitializeCfmMonth {
        product: u8,
        settlement_base_oracle_atomic: u64,
    },
    /// Retired September-only setup wire; decoding rejects selector 19.
    InitializeCfmPolicy {
        product: u8,
    },
    Council(OracleCouncilActionV1),
    RegisterRoot,
    RegisterSuccessor,
    CaptureCurrent {
        kind: u8,
        previous_hash: [u8; 32],
    },
    Import {
        sku_index: u16,
        proof: Vec<[u8; 32]>,
    },
    SkipInactive,
    BeginSelection,
    ScanCheckpoint,
    FreezeOpening,
    BeginHistoryMedian {
        mode: u8,
        lower: u64,
        upper: u64,
    },
    ScanHistoryMedian,
    CloseHistoryMedian,
    BeginBucketRank {
        mode: u8,
        lower: u64,
        upper: u64,
        nonce: [u8; 32],
    },
    ScanBucketRank,
    CloseBucketRank,
    WriteEvidence {
        kind: u8,
        hash: [u8; 32],
        total: u16,
        offset: u16,
        bytes: Vec<u8>,
    },
    CloseEvidenceDraft,
    BackfillSourceEvidence,
    BackfillClaimEvidence {
        role: u8,
    },
}

impl OracleCarryForwardActionV1 {
    /// Branch-for-branch mirror of `deserialize_reader` on a [`CheckedCursor`]. A short read
    /// poisons the cursor (rejected by the caller's `finish`); every value check rejects
    /// exactly where the reader does.
    #[inline(never)]
    fn read_cursor(c: &mut CheckedCursor<'_>) -> io::Result<Self> {
        use crate::fixed_codec::CursorField;
        let invalid = || Err(io::Error::from(io::ErrorKind::InvalidData));
        Ok(match c.u8() {
            19 | 20 | 25 | 26 => return invalid(),
            24 => {
                if c.bytes::<4>() != *b"TRC1" {
                    return invalid();
                }
                Self::TerminalCleanup(CursorField::read(c))
            }
            22 => {
                if c.bytes::<4>() != *b"OSB1" {
                    return invalid();
                }
                Self::SponsoredSource(CursorField::read(c))
            }
            #[cfg(not(feature = "mainnet-v3"))]
            tag @ (21 | 23) => {
                let magic = if tag == 21 { *b"SCB1" } else { *b"OCB1" };
                if c.bytes::<4>() != magic {
                    return invalid();
                }
                let operation = c.u8();
                let market = c.u8();
                let row = c.u8();
                let plan_hash: [u8; 32] = c.bytes();
                if operation > 2
                    || market > 3
                    || (operation != 1 && row != 0)
                    || (operation == 1 && row >= if market < 2 { 13 } else { 22 })
                    || crate::bytes32_is_zero(&plan_hash)
                {
                    return invalid();
                }
                if tag == 21 {
                    Self::SeptemberBootstrap {
                        operation,
                        market,
                        row,
                        plan_hash,
                    }
                } else {
                    Self::OctoberBootstrap {
                        operation,
                        market,
                        row,
                        plan_hash,
                    }
                }
            }
            18 => {
                if c.bytes::<4>() != *b"CV01" {
                    return invalid();
                }
                let action = OracleCouncilActionV1::read(c);
                if action.operation > 3 || action.choice > 2 || action.expected_epoch == 0 {
                    return invalid();
                }
                Self::Council(action)
            }
            0 => Self::RegisterRoot,
            1 => Self::RegisterSuccessor,
            2 => {
                let kind = c.u8();
                if kind > 2 {
                    return invalid();
                }
                Self::CaptureCurrent {
                    kind,
                    previous_hash: c.bytes(),
                }
            }
            3 => {
                let sku_index = c.u16();
                let count = usize::from(c.u8());
                if count > MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH {
                    return invalid();
                }
                let mut proof = Vec::with_capacity(count);
                for _ in 0..count {
                    proof.push(c.bytes());
                }
                Self::Import { sku_index, proof }
            }
            4 => Self::SkipInactive,
            5 => Self::BeginSelection,
            6 => Self::ScanCheckpoint,
            7 => Self::FreezeOpening,
            8 => {
                let mode = c.u8();
                if mode > 1 {
                    return invalid();
                }
                Self::BeginHistoryMedian {
                    mode,
                    lower: c.u64(),
                    upper: c.u64(),
                }
            }
            9 => Self::ScanHistoryMedian,
            10 => Self::CloseHistoryMedian,
            11 => {
                let mode = c.u8();
                if mode > 2 {
                    return invalid();
                }
                Self::BeginBucketRank {
                    mode,
                    lower: c.u64(),
                    upper: c.u64(),
                    nonce: c.bytes(),
                }
            }
            12 => Self::ScanBucketRank,
            13 => Self::CloseBucketRank,
            14 => {
                let kind = c.u8();
                let hash = c.bytes();
                let total = c.u16();
                let offset = c.u16();
                let len = usize::from(c.u8());
                if !(1..=3).contains(&kind) || len == 0 || len > 192 || total == 0 || total > 4096 {
                    return invalid();
                }
                Self::WriteEvidence {
                    kind,
                    hash,
                    total,
                    offset,
                    bytes: c.vec(len),
                }
            }
            15 => Self::CloseEvidenceDraft,
            16 => Self::BackfillSourceEvidence,
            17 => {
                let role = c.u8();
                if !(2..=5).contains(&role) {
                    return invalid();
                }
                Self::BackfillClaimEvidence { role }
            }
            _ => return invalid(),
        })
    }
}

impl BorshDeserialize for OracleCarryForwardActionV1 {
    fn deserialize(data: &mut &[u8]) -> io::Result<Self> {
        let mut cursor = CheckedCursor::new(data);
        let value = Self::read_cursor(&mut cursor);
        let rest = cursor.finish()?;
        let value = value?;
        *data = rest;
        Ok(value)
    }

    fn try_from_slice(data: &[u8]) -> io::Result<Self> {
        let mut cursor = CheckedCursor::new(data);
        let value = Self::read_cursor(&mut cursor);
        cursor.finish_exact()?;
        value
    }

    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        Ok(match u8::deserialize_reader(reader)? {
            19 | 20 | 25 | 26 => return Err(io::ErrorKind::InvalidData.into()),
            24 => {
                if <[u8; 4]>::deserialize_reader(reader)? != *b"TRC1" {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                Self::TerminalCleanup(TerminalCleanupParams::deserialize_reader(reader)?)
            }
            22 => {
                if <[u8; 4]>::deserialize_reader(reader)? != *b"OSB1" {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                Self::SponsoredSource(OracleSponsoredActionV1::deserialize_reader(reader)?)
            }
            #[cfg(not(feature = "mainnet-v3"))]
            21 => {
                if <[u8; 4]>::deserialize_reader(reader)? != *b"SCB1" {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                let operation = u8::deserialize_reader(reader)?;
                let market = u8::deserialize_reader(reader)?;
                let row = u8::deserialize_reader(reader)?;
                let plan_hash = <[u8; 32]>::deserialize_reader(reader)?;
                if operation > 2
                    || market > 3
                    || (operation != 1 && row != 0)
                    || (operation == 1 && row >= if market < 2 { 13 } else { 22 })
                    || plan_hash == [0; 32]
                {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                Self::SeptemberBootstrap {
                    operation,
                    market,
                    row,
                    plan_hash,
                }
            }
            #[cfg(not(feature = "mainnet-v3"))]
            23 => {
                if <[u8; 4]>::deserialize_reader(reader)? != *b"OCB1" {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                let operation = u8::deserialize_reader(reader)?;
                let market = u8::deserialize_reader(reader)?;
                let row = u8::deserialize_reader(reader)?;
                let plan_hash = <[u8; 32]>::deserialize_reader(reader)?;
                if operation > 2
                    || market > 3
                    || (operation != 1 && row != 0)
                    || (operation == 1 && row >= if market < 2 { 13 } else { 22 })
                    || plan_hash == [0; 32]
                {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                Self::OctoberBootstrap {
                    operation,
                    market,
                    row,
                    plan_hash,
                }
            }
            18 => {
                if <[u8; 4]>::deserialize_reader(reader)? != *b"CV01" {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                let action = OracleCouncilActionV1::deserialize_reader(reader)?;
                if action.operation > 3 || action.choice > 2 || action.expected_epoch == 0 {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                Self::Council(action)
            }
            0 => Self::RegisterRoot,
            1 => Self::RegisterSuccessor,
            2 => {
                let kind = u8::deserialize_reader(reader)?;
                if kind > 2 {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                Self::CaptureCurrent {
                    kind,
                    previous_hash: <[u8; 32]>::deserialize_reader(reader)?,
                }
            }
            3 => {
                let sku_index = u16::deserialize_reader(reader)?;
                let count = usize::from(u8::deserialize_reader(reader)?);
                if count > MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                let mut proof = Vec::with_capacity(count);
                for _ in 0..count {
                    proof.push(<[u8; 32]>::deserialize_reader(reader)?);
                }
                Self::Import { sku_index, proof }
            }
            4 => Self::SkipInactive,
            5 => Self::BeginSelection,
            6 => Self::ScanCheckpoint,
            7 => Self::FreezeOpening,
            8 => {
                let mode = u8::deserialize_reader(reader)?;
                if mode > 1 {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                Self::BeginHistoryMedian {
                    mode,
                    lower: u64::deserialize_reader(reader)?,
                    upper: u64::deserialize_reader(reader)?,
                }
            }
            9 => Self::ScanHistoryMedian,
            10 => Self::CloseHistoryMedian,
            11 => {
                let mode = u8::deserialize_reader(reader)?;
                if mode > 2 {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                Self::BeginBucketRank {
                    mode,
                    lower: u64::deserialize_reader(reader)?,
                    upper: u64::deserialize_reader(reader)?,
                    nonce: <[u8; 32]>::deserialize_reader(reader)?,
                }
            }
            12 => Self::ScanBucketRank,
            13 => Self::CloseBucketRank,
            14 => {
                let kind = u8::deserialize_reader(reader)?;
                let hash = <[u8; 32]>::deserialize_reader(reader)?;
                let total = u16::deserialize_reader(reader)?;
                let offset = u16::deserialize_reader(reader)?;
                let len = usize::from(u8::deserialize_reader(reader)?);
                if !(1..=3).contains(&kind) || len == 0 || len > 192 || total == 0 || total > 4096 {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                let mut bytes = vec![0; len];
                reader.read_exact(&mut bytes)?;
                Self::WriteEvidence {
                    kind,
                    hash,
                    total,
                    offset,
                    bytes,
                }
            }
            15 => Self::CloseEvidenceDraft,
            16 => Self::BackfillSourceEvidence,
            17 => {
                let role = u8::deserialize_reader(reader)?;
                if !(2..=5).contains(&role) {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                Self::BackfillClaimEvidence { role }
            }
            _ => return Err(io::ErrorKind::InvalidData.into()),
        })
    }
}

impl BorshSerialize for OracleCarryForwardActionV1 {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        let tag: u8 = match self {
            Self::ExtendOctoberLadder(_) => 26,
            Self::InstallOctoberLadder(_) => 25,
            Self::TerminalCleanup(_) => 24,
            Self::SponsoredSource(_) => 22,
            #[cfg(not(feature = "mainnet-v3"))]
            Self::SeptemberBootstrap { .. } => 21,
            #[cfg(not(feature = "mainnet-v3"))]
            Self::OctoberBootstrap { .. } => 23,
            Self::InitializeCfmMonth { .. } => 20,
            Self::InitializeCfmPolicy { .. } => 19,
            Self::Council(_) => 18,
            Self::RegisterRoot => 0,
            Self::RegisterSuccessor => 1,
            Self::CaptureCurrent { .. } => 2,
            Self::Import { .. } => 3,
            Self::SkipInactive => 4,
            Self::BeginSelection => 5,
            Self::ScanCheckpoint => 6,
            Self::FreezeOpening => 7,
            Self::BeginHistoryMedian { .. } => 8,
            Self::ScanHistoryMedian => 9,
            Self::CloseHistoryMedian => 10,
            Self::BeginBucketRank { .. } => 11,
            Self::ScanBucketRank => 12,
            Self::CloseBucketRank => 13,
            Self::WriteEvidence { .. } => 14,
            Self::CloseEvidenceDraft => 15,
            Self::BackfillSourceEvidence => 16,
            Self::BackfillClaimEvidence { .. } => 17,
        };
        match self {
            Self::ExtendOctoberLadder(params) => {
                tag.serialize(writer)?;
                writer.write_all(b"OCE1")?;
                params.serialize(writer)
            }
            Self::InstallOctoberLadder(params) => {
                tag.serialize(writer)?;
                writer.write_all(b"OCL1")?;
                params.serialize(writer)
            }
            Self::TerminalCleanup(params) => {
                tag.serialize(writer)?;
                writer.write_all(b"TRC1")?;
                params.serialize(writer)
            }
            Self::SponsoredSource(action) => {
                tag.serialize(writer)?;
                writer.write_all(b"OSB1")?;
                action.serialize(writer)
            }
            #[cfg(not(feature = "mainnet-v3"))]
            Self::SeptemberBootstrap {
                operation,
                market,
                row,
                plan_hash,
            } => {
                if *operation > 2
                    || *market > 3
                    || (*operation != 1 && *row != 0)
                    || (*operation == 1 && *row >= if *market < 2 { 13 } else { 22 })
                    || *plan_hash == [0; 32]
                {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                tag.serialize(writer)?;
                writer.write_all(b"SCB1")?;
                operation.serialize(writer)?;
                market.serialize(writer)?;
                row.serialize(writer)?;
                plan_hash.serialize(writer)
            }
            #[cfg(not(feature = "mainnet-v3"))]
            Self::OctoberBootstrap {
                operation,
                market,
                row,
                plan_hash,
            } => {
                if *operation > 2
                    || *market > 3
                    || (*operation != 1 && *row != 0)
                    || (*operation == 1 && *row >= if *market < 2 { 13 } else { 22 })
                    || *plan_hash == [0; 32]
                {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                tag.serialize(writer)?;
                writer.write_all(b"OCB1")?;
                operation.serialize(writer)?;
                market.serialize(writer)?;
                row.serialize(writer)?;
                plan_hash.serialize(writer)
            }
            Self::InitializeCfmMonth {
                product,
                settlement_base_oracle_atomic,
            } => {
                if *product > 1 || *settlement_base_oracle_atomic == 0 {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                tag.serialize(writer)?;
                writer.write_all(b"CFM1")?;
                product.serialize(writer)?;
                settlement_base_oracle_atomic.serialize(writer)
            }
            Self::InitializeCfmPolicy { product } => {
                if *product > 1 {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                tag.serialize(writer)?;
                writer.write_all(b"CFM1")?;
                product.serialize(writer)
            }
            Self::Council(action) => {
                tag.serialize(writer)?;
                writer.write_all(b"CV01")?;
                action.serialize(writer)
            }
            Self::BackfillClaimEvidence { role } => {
                if !(2..=5).contains(role) {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                tag.serialize(writer)?;
                role.serialize(writer)
            }
            Self::WriteEvidence {
                kind,
                hash,
                total,
                offset,
                bytes,
            } => {
                if !(1..=3).contains(kind)
                    || bytes.is_empty()
                    || bytes.len() > 192
                    || *total == 0
                    || *total > 4096
                {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                tag.serialize(writer)?;
                kind.serialize(writer)?;
                hash.serialize(writer)?;
                total.serialize(writer)?;
                offset.serialize(writer)?;
                (bytes.len() as u8).serialize(writer)?;
                writer.write_all(bytes)
            }
            Self::BeginBucketRank {
                mode,
                lower,
                upper,
                nonce,
            } => {
                if *mode > 2 || lower > upper {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                tag.serialize(writer)?;
                mode.serialize(writer)?;
                lower.serialize(writer)?;
                upper.serialize(writer)?;
                nonce.serialize(writer)
            }
            Self::BeginHistoryMedian { mode, lower, upper } => {
                if *mode > 1 || lower > upper {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                tag.serialize(writer)?;
                mode.serialize(writer)?;
                lower.serialize(writer)?;
                upper.serialize(writer)
            }
            Self::CaptureCurrent {
                kind,
                previous_hash,
            } => {
                if *kind > 2 {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                tag.serialize(writer)?;
                kind.serialize(writer)?;
                previous_hash.serialize(writer)
            }
            Self::Import { sku_index, proof } => {
                if proof.len() > MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                let count = u8::try_from(proof.len()).map_err(|_| io::ErrorKind::InvalidData)?;
                tag.serialize(writer)?;
                sku_index.serialize(writer)?;
                count.serialize(writer)?;
                for node in proof {
                    node.serialize(writer)?;
                }
                Ok(())
            }
            _ => tag.serialize(writer),
        }
    }
}
