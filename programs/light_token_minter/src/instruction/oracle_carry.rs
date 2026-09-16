use super::*;
#[derive(Clone, Debug, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct OracleCouncilActionV1 {
    pub operation: u8,
    pub kind: OracleEmergencyDisputeKind,
    pub target_id: [u8; 32],
    pub expected_case_hash: [u8; 32],
    pub expected_epoch: u64,
    pub expected_seats_hash: [u8; 32],
    pub choice: u8,
}

/// Bounded carry-forward subactions. The import proof count is one byte, not an
/// unbounded Borsh Vec prefix; the wire shape matches the client carry builders.
#[derive(Clone, Debug, PartialEq)]
pub enum OracleCarryForwardActionV1 {
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

impl BorshDeserialize for OracleCarryForwardActionV1 {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        Ok(match u8::deserialize_reader(reader)? {
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
