use super::*;

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq)]
pub struct CompressionOutput {
    pub address_tree_info: PackedAddressTreeInfo,
    pub output_state_tree_index: u8,
}

pub(crate) fn deserialize_compression_output(data: &mut &[u8]) -> io::Result<CompressionOutput> {
    let mut cursor = CheckedCursor::new(data);
    let value = deserialize_compression_output_cursor(&mut cursor);
    *data = cursor.finish()?;
    Ok(value)
}

#[inline(never)]
fn deserialize_compression_output_cursor(cursor: &mut CheckedCursor<'_>) -> CompressionOutput {
    CompressionOutput {
        address_tree_info: PackedAddressTreeInfo {
            address_merkle_tree_pubkey_index: cursor.u8(),
            address_queue_pubkey_index: cursor.u8(),
            root_index: cursor.u16(),
        },
        output_state_tree_index: cursor.u8(),
    }
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq)]
pub struct CompressedMarketPageWitness {
    pub meta: CompressedAccountMeta,
    pub expected_commitment: [u8; 32],
    pub page: CompressedMarketPageLeaf,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq)]
pub struct CompressedSettlementWitness {
    pub meta: CompressedAccountMeta,
    pub expected_commitment: [u8; 32],
    pub settlement: CompressedSettlementLeaf,
}

/// How one logical typed PDA participates in an atomic compressed-state session.
#[derive(Clone, Debug, PartialEq)]
pub enum CompressedStateAccess {
    /// Prove and materialize the leaf for the unchanged inner state machine, but reject any write.
    ReadOnly {
        account_index: u8,
        meta: CompressedAccountMetaReadOnly,
        leaf: CompressedAmebaStateLeaf,
    },
    /// Consume the exact old leaf and emit its revision-incremented successor.
    Mutable {
        account_index: u8,
        meta: CompressedAccountMeta,
        leaf: CompressedAmebaStateLeaf,
    },
    /// Capture a typed PDA created by the inner instruction, initialize its compressed address,
    /// then close and refund the temporary native account before the transaction completes.
    Initialize {
        account_index: u8,
        domain: CompressedStateDomain,
        output: CompressionOutput,
    },
}

impl BorshSerialize for CompressedStateAccess {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        match self {
            Self::ReadOnly {
                account_index,
                meta,
                leaf,
            } => {
                0_u8.serialize(writer)?;
                account_index.serialize(writer)?;
                meta.tree_info.serialize(writer)?;
                serialize_compact_access_leaf(writer, leaf)
            }
            Self::Mutable {
                account_index,
                meta,
                leaf,
            } => {
                1_u8.serialize(writer)?;
                account_index.serialize(writer)?;
                meta.tree_info.serialize(writer)?;
                meta.output_state_tree_index.serialize(writer)?;
                serialize_compact_access_leaf(writer, leaf)
            }
            Self::Initialize {
                account_index,
                domain,
                output,
            } => {
                2_u8.serialize(writer)?;
                account_index.serialize(writer)?;
                domain.serialize(writer)?;
                output.serialize(writer)
            }
        }
    }
}

impl BorshDeserialize for CompressedStateAccess {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let kind = u8::deserialize_reader(reader)?;
        let account_index = u8::deserialize_reader(reader)?;
        match kind {
            0 => {
                let tree_info = PackedStateTreeInfo::deserialize_reader(reader)?;
                let leaf = deserialize_compact_access_leaf(reader)?;
                Ok(Self::ReadOnly {
                    account_index,
                    meta: CompressedAccountMetaReadOnly {
                        tree_info,
                        address: [0; 32],
                    },
                    leaf,
                })
            }
            1 => {
                let tree_info = PackedStateTreeInfo::deserialize_reader(reader)?;
                let output_state_tree_index = u8::deserialize_reader(reader)?;
                let leaf = deserialize_compact_access_leaf(reader)?;
                Ok(Self::Mutable {
                    account_index,
                    meta: CompressedAccountMeta {
                        tree_info,
                        address: [0; 32],
                        output_state_tree_index,
                    },
                    leaf,
                })
            }
            2 => Ok(Self::Initialize {
                account_index,
                domain: CompressedStateDomain::deserialize_reader(reader)?,
                output: CompressionOutput::deserialize_reader(reader)?,
            }),
            _ => Err(io::ErrorKind::InvalidData.into()),
        }
    }
}

fn serialize_compact_access_leaf<W: io::Write>(
    writer: &mut W,
    leaf: &CompressedAmebaStateLeaf,
) -> io::Result<()> {
    if leaf.schema_version != CompressedAmebaStateLeaf::CURRENT_SCHEMA_VERSION
        || leaf.data.len() != leaf.domain.compact_data_len()
    {
        return Err(io::ErrorKind::InvalidData.into());
    }
    leaf.domain.serialize(writer)?;
    leaf.revision.serialize(writer)?;
    writer.write_all(&leaf.data)
}

fn deserialize_compact_access_leaf<R: io::Read>(
    reader: &mut R,
) -> io::Result<CompressedAmebaStateLeaf> {
    let domain = CompressedStateDomain::deserialize_reader(reader)?;
    let revision = u64::deserialize_reader(reader)?;
    let mut data = vec![0; domain.compact_data_len()];
    reader.read_exact(&mut data)?;
    Ok(CompressedAmebaStateLeaf {
        schema_version: CompressedAmebaStateLeaf::CURRENT_SCHEMA_VERSION,
        domain,
        canonical_pda: Pubkey::default(),
        revision,
        data,
    })
}

impl CompressedStateAccess {
    pub fn account_index(&self) -> u8 {
        match self {
            Self::ReadOnly { account_index, .. }
            | Self::Mutable { account_index, .. }
            | Self::Initialize { account_index, .. } => *account_index,
        }
    }

    pub fn domain(&self) -> CompressedStateDomain {
        match self {
            Self::ReadOnly { leaf, .. } | Self::Mutable { leaf, .. } => leaf.domain,
            Self::Initialize { domain, .. } => *domain,
        }
    }
}

#[inline(always)]
fn envelope_domain(cursor: &mut CheckedCursor<'_>) -> CompressedStateDomain {
    match cursor.u8() {
        1 => CompressedStateDomain::OracleSkuCoverageRecord,
        2 => CompressedStateDomain::OracleUsdcSkuPool,
        3 => CompressedStateDomain::OracleUsdcSourceReward,
        4 => CompressedStateDomain::OracleUsdcRewardRegistration,
        5 => CompressedStateDomain::OracleUsdcRewardReceipt,
        6 => CompressedStateDomain::OracleSambaWinningVote,
        7 => CompressedStateDomain::OracleSambaVoteSettlementReceipt,
        8 => CompressedStateDomain::OracleSupportPosition,
        9 => CompressedStateDomain::OracleSourceState,
        10 => CompressedStateDomain::OracleSourceDescriptor,
        _ => {
            cursor.invalid = true;
            CompressedStateDomain::OracleSkuCoverageRecord
        }
    }
}

fn envelope_leaf(cursor: &mut CheckedCursor<'_>) -> CompressedAmebaStateLeaf {
    let domain = envelope_domain(cursor);
    CompressedAmebaStateLeaf {
        schema_version: CompressedAmebaStateLeaf::CURRENT_SCHEMA_VERSION,
        domain,
        canonical_pda: Pubkey::default(),
        revision: cursor.u64(),
        data: cursor.vec(domain.compact_data_len()),
    }
}

fn envelope_access(cursor: &mut CheckedCursor<'_>) -> CompressedStateAccess {
    let kind = cursor.u8();
    let account_index = cursor.u8();
    match kind {
        0 => CompressedStateAccess::ReadOnly {
            account_index,
            meta: CompressedAccountMetaReadOnly {
                tree_info: deserialize_packed_state_tree_info_cursor(cursor),
                address: [0; 32],
            },
            leaf: envelope_leaf(cursor),
        },
        1 => CompressedStateAccess::Mutable {
            account_index,
            meta: CompressedAccountMeta {
                tree_info: deserialize_packed_state_tree_info_cursor(cursor),
                address: [0; 32],
                output_state_tree_index: cursor.u8(),
            },
            leaf: envelope_leaf(cursor),
        },
        2 => CompressedStateAccess::Initialize {
            account_index,
            domain: envelope_domain(cursor),
            output: deserialize_compression_output_cursor(cursor),
        },
        _ => {
            cursor.invalid = true;
            CompressedStateAccess::Initialize {
                account_index,
                domain: CompressedStateDomain::OracleSkuCoverageRecord,
                output: CompressionOutput {
                    address_tree_info: PackedAddressTreeInfo::default(),
                    output_state_tree_index: 0,
                },
            }
        }
    }
}

/// Proof-aware transport around one existing instruction. `accounts` are laid out as the exact
/// inner account prefix, one appended classic system-program account, then the ordinary packed
/// Light CPI accounts. Public clients build this wrapper internally; the inner payload and state
/// transition remain unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct ExecuteCompressedStateParams {
    pub core_account_count: u8,
    pub rent_payer_index: u8,
    pub proof: ValidityProof,
    pub accesses: Vec<CompressedStateAccess>,
    pub inner_instruction: Vec<u8>,
}

impl BorshSerialize for ExecuteCompressedStateParams {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        if self.accesses.len() > MAX_COMPRESSED_STATE_SESSION_RECORDS
            || self.inner_instruction.len() > MAX_COMPRESSED_INNER_INSTRUCTION_BYTES
            || self.inner_instruction.len() > usize::from(u16::MAX)
        {
            return Err(invalid_bounded_length(
                "compressed state session",
                self.accesses.len().max(self.inner_instruction.len()),
                MAX_COMPRESSED_INNER_INSTRUCTION_BYTES,
            ));
        }
        self.core_account_count.serialize(writer)?;
        self.rent_payer_index.serialize(writer)?;
        self.proof.serialize(writer)?;
        (self.accesses.len() as u8).serialize(writer)?;
        for access in &self.accesses {
            access.serialize(writer)?;
        }
        (self.inner_instruction.len() as u16).serialize(writer)?;
        writer.write_all(&self.inner_instruction)
    }
}

impl BorshDeserialize for ExecuteCompressedStateParams {
    fn deserialize(data: &mut &[u8]) -> io::Result<Self> {
        let mut cursor = CheckedCursor::new(data);
        let core_account_count = cursor.u8();
        let rent_payer_index = cursor.u8();
        let proof = deserialize_validity_proof_cursor(&mut cursor);
        let access_count = usize::from(cursor.u8());
        if access_count > MAX_COMPRESSED_STATE_SESSION_RECORDS {
            cursor.invalid = true;
        }
        let bounded_access_count = access_count.min(MAX_COMPRESSED_STATE_SESSION_RECORDS);
        let mut accesses = Vec::with_capacity(bounded_access_count);
        for _ in 0..bounded_access_count {
            accesses.push(envelope_access(&mut cursor));
        }
        let inner_length = usize::from(cursor.u16());
        if inner_length > MAX_COMPRESSED_INNER_INSTRUCTION_BYTES {
            cursor.invalid = true;
        }
        let inner_instruction =
            cursor.vec(inner_length.min(MAX_COMPRESSED_INNER_INSTRUCTION_BYTES));
        *data = cursor.finish()?;
        Ok(Self {
            core_account_count,
            rent_payer_index,
            proof,
            accesses,
            inner_instruction,
        })
    }

    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let core_account_count = u8::deserialize_reader(reader)?;
        let rent_payer_index = u8::deserialize_reader(reader)?;
        let proof = ValidityProof::deserialize_reader(reader)?;
        let access_count = usize::from(u8::deserialize_reader(reader)?);
        if access_count > MAX_COMPRESSED_STATE_SESSION_RECORDS {
            return Err(invalid_bounded_length(
                "compressed_state_accesses",
                access_count,
                MAX_COMPRESSED_STATE_SESSION_RECORDS,
            ));
        }
        let mut accesses = Vec::with_capacity(access_count);
        for _ in 0..access_count {
            accesses.push(CompressedStateAccess::deserialize_reader(reader)?);
        }
        let inner_length = usize::from(u16::deserialize_reader(reader)?);
        if inner_length > MAX_COMPRESSED_INNER_INSTRUCTION_BYTES {
            return Err(invalid_bounded_length(
                "compressed_inner_instruction",
                inner_length,
                MAX_COMPRESSED_INNER_INSTRUCTION_BYTES,
            ));
        }
        let mut inner_instruction = vec![0; inner_length];
        reader.read_exact(&mut inner_instruction)?;
        Ok(Self {
            core_account_count,
            rent_payer_index,
            proof,
            accesses,
            inner_instruction,
        })
    }
}
