use super::*;
use crate::state::OracleEmergencyDisputeKind;

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
        terms: crate::sponsorship::SponsorTerms,
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

/// Bounded carry-forward subactions behind tag 30. Each selector is one byte followed by its
/// exact fields; every value check rejects in the decoder and the serializer alike.
#[derive(Clone, Debug, PartialEq)]
pub enum OracleCarryForwardActionV1 {
    /// Selector 24 ("TRC1"): reclaim the paid terminal bookkeeping of a settled month.
    TerminalCleanup(TerminalCleanupParams),
    /// Selector 22 ("OSB1"): underwritten source activation, reconciliation, bounty and reserve.
    SponsoredSource(OracleSponsoredActionV1),
    /// Selector 18 ("CV01"): council adjudication of an unresolved dispute.
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

/// Evidence object kinds 1-4 (`processor/oracle_evidence.rs`): locator, definition, Wayback
/// archive URL, authenticated evidence record.
const EVIDENCE_KINDS: u8 = crate::constants::ORACLE_EVIDENCE_RECORD_KIND;

impl OracleCarryForwardActionV1 {
    /// Branch-for-branch mirror of `deserialize_reader` on a [`CheckedCursor`]. A short read
    /// poisons the cursor (rejected by the caller's `finish`); every value check rejects
    /// exactly where the reader does.
    #[inline(never)]
    fn read_cursor(c: &mut CheckedCursor<'_>) -> io::Result<Self> {
        let invalid = || Err(io::Error::from(io::ErrorKind::InvalidData));
        Ok(match c.u8() {
            24 => {
                if c.bytes::<4>() != *b"TRC1" {
                    return invalid();
                }
                Self::TerminalCleanup(crate::fixed_codec::CursorField::read(c))
            }
            22 => {
                if c.bytes::<4>() != *b"OSB1" {
                    return invalid();
                }
                Self::SponsoredSource(crate::fixed_codec::CursorField::read(c))
            }
            18 => {
                if c.bytes::<4>() != *b"CV01" {
                    return invalid();
                }
                let action: OracleCouncilActionV1 = crate::fixed_codec::CursorField::read(c);
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
                if !(1..=EVIDENCE_KINDS).contains(&kind)
                    || len == 0
                    || len > 192
                    || total == 0
                    || total > 4096
                {
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
                if !(1..=EVIDENCE_KINDS).contains(&kind)
                    || len == 0
                    || len > 192
                    || total == 0
                    || total > 4096
                {
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
            Self::TerminalCleanup(_) => 24,
            Self::SponsoredSource(_) => 22,
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
            Self::Council(action) => {
                if action.operation > 3 || action.choice > 2 || action.expected_epoch == 0 {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                tag.serialize(writer)?;
                writer.write_all(b"CV01")?;
                action.serialize(writer)
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
                if !(1..=EVIDENCE_KINDS).contains(kind)
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
            _ => tag.serialize(writer),
        }
    }
}

#[cfg(test)]
mod compact_borsh_equivalence {
    use super::*;
    use crate::fixed_codec::equivalence::check_cursor;

    fn source() -> ProposeOracleSourceV3Params {
        ProposeOracleSourceV3Params {
            source_id: [1; 32],
            bucket_id: [2; 32],
            source_type_hash: [3; 32],
            canonical_locator_hash: [4; 32],
            source_definition_hash: [5; 32],
            listing_bond: 77,
            sku_index: 9,
            sku_proof: vec![[6; 32], [7; 32]],
        }
    }

    fn carry_samples() -> Vec<Vec<u8>> {
        use OracleCarryForwardActionV1 as A;
        let actions = vec![
            A::TerminalCleanup(TerminalCleanupParams {
                kind: 3,
                bucket_id: [14; 32],
                expected_data_hash: [15; 32],
                expected_lamports: 16,
            }),
            A::SponsoredSource(OracleSponsoredActionV1::Activate {
                terms: crate::sponsorship::SponsorTerms {
                    agent_id: [10; 32],
                    researcher: Pubkey::new_from_array([11; 32]),
                    candidate_hash: [12; 32],
                    funding: 1,
                    researcher_bps: 10_000,
                    retention_bps: 2_000,
                    maximum_bond: 13,
                    maximum_outstanding: 14,
                },
                source: source(),
            }),
            A::SponsoredSource(OracleSponsoredActionV1::Reconcile),
            A::SponsoredSource(OracleSponsoredActionV1::ClaimBounty),
            A::SponsoredSource(OracleSponsoredActionV1::WithdrawReserve { amount: 17 }),
            A::Council(OracleCouncilActionV1 {
                operation: 2,
                kind: OracleEmergencyDisputeKind::Update,
                target_id: [16; 32],
                expected_case_hash: [17; 32],
                expected_epoch: 3,
                expected_seats_hash: [18; 32],
                choice: 1,
            }),
            A::RegisterRoot,
            A::RegisterSuccessor,
            A::CaptureCurrent {
                kind: 2,
                previous_hash: [18; 32],
            },
            A::Import {
                sku_index: 3,
                proof: vec![[19; 32]; 3],
            },
            A::Import {
                sku_index: 0,
                proof: vec![],
            },
            A::SkipInactive,
            A::BeginSelection,
            A::ScanCheckpoint,
            A::FreezeOpening,
            A::BeginHistoryMedian {
                mode: 1,
                lower: 2,
                upper: 3,
            },
            A::ScanHistoryMedian,
            A::CloseHistoryMedian,
            A::BeginBucketRank {
                mode: 2,
                lower: 4,
                upper: 5,
                nonce: [20; 32],
            },
            A::ScanBucketRank,
            A::CloseBucketRank,
            A::WriteEvidence {
                kind: 3,
                hash: [21; 32],
                total: 4096,
                offset: 7,
                bytes: vec![22; 5],
            },
            A::WriteEvidence {
                kind: 4,
                hash: [23; 32],
                total: 512,
                offset: 0,
                bytes: vec![24; 192],
            },
            A::CloseEvidenceDraft,
            A::BackfillSourceEvidence,
            A::BackfillClaimEvidence { role: 5 },
        ];
        actions
            .iter()
            .map(|action| action.try_to_vec().unwrap())
            .collect()
    }

    #[test]
    fn carry_action_cursor_decoder_matches_reader() {
        // The serializer additionally rejects lower > upper; the decoder does not.
        crate::fixed_codec::equivalence::check_debug_with_samples::<OracleCarryForwardActionV1>(
            0xca77,
            400,
            &carry_samples(),
        );
    }

    #[test]
    fn propose_source_cursor_reader_matches_reader() {
        let mut empty_proof = source();
        empty_proof.sku_proof.clear();
        let samples = vec![
            source().try_to_vec().unwrap(),
            empty_proof.try_to_vec().unwrap(),
        ];
        check_cursor::<ProposeOracleSourceV3Params>(0x9905, 400, &samples);
        crate::fixed_codec::equivalence::check_with_samples::<ProposeOracleSourceV3Params>(
            0x9906, 400, &samples,
        );
    }
}
