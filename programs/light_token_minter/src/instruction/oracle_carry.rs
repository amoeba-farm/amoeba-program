use super::*;

/// Bounded carry-forward subactions. The import proof count is one byte, not an
/// unbounded Borsh Vec prefix; the wire shape matches the client carry builders.
#[derive(Clone, Debug, PartialEq)]
pub enum OracleCarryForwardActionV1 {
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
}

impl BorshDeserialize for OracleCarryForwardActionV1 {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        Ok(match u8::deserialize_reader(reader)? {
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
            _ => return Err(io::ErrorKind::InvalidData.into()),
        })
    }
}

impl BorshSerialize for OracleCarryForwardActionV1 {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        let tag: u8 = match self {
            Self::RegisterRoot => 0,
            Self::RegisterSuccessor => 1,
            Self::CaptureCurrent { .. } => 2,
            Self::Import { .. } => 3,
            Self::SkipInactive => 4,
            Self::BeginSelection => 5,
            Self::ScanCheckpoint => 6,
            Self::FreezeOpening => 7,
        };
        match self {
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
