//! Strict, bounded action payload carried by ManageWriterDlmmV1 (159).
use crate::state::{WriterDlmmBinV1, WriterDlmmSeriesPolicyV1, WRITER_DLMM_ACTION_ENTRIES};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct BeginWriterDlmmPolicyV1Params {
    pub management_authority: Pubkey,
    pub expected_policy_hash: [u8; 32],
    pub monthly_buyback_cap_atoms: u64,
    pub transaction_buyback_cap_atoms: u64,
    pub reserve_release_spend_ratio_ppm: u64,
    pub price_separation_ticks: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManageWriterDlmmV1Params {
    BeginPolicy(BeginWriterDlmmPolicyV1Params),
    AppendPolicySeries {
        start_index: u8,
        entries: Vec<WriterDlmmSeriesPolicyV1>,
    },
    SealPolicy,
    InitializePosition {
        series_index: u8,
    },
    AddLiquidity {
        series_index: u8,
        issue_amount_atoms: u64,
        entries: Vec<WriterDlmmBinV1>,
    },
    RemoveLiquidity {
        series_index: u8,
        entries: Vec<WriterDlmmBinV1>,
    },
    SweepCash {
        series_index: u8,
    },
}

impl ManageWriterDlmmV1Params {
    pub fn selector(&self) -> u8 {
        match self {
            Self::BeginPolicy(_) => 0,
            Self::AppendPolicySeries { .. } => 1,
            Self::SealPolicy => 2,
            Self::InitializePosition { .. } => 3,
            Self::AddLiquidity { .. } => 4,
            Self::RemoveLiquidity { .. } => 5,
            Self::SweepCash { .. } => 6,
        }
    }

    pub fn decode_exact(payload: &[u8]) -> std::io::Result<Self> {
        let mut bytes = payload;
        let value = Self::deserialize(&mut bytes)?;
        if !bytes.is_empty() {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        Ok(value)
    }
}

fn read_entries<R: std::io::Read, T: BorshDeserialize>(reader: &mut R) -> std::io::Result<Vec<T>> {
    let count = u32::deserialize_reader(reader)? as usize;
    if count == 0 || count > WRITER_DLMM_ACTION_ENTRIES {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(T::deserialize_reader(reader)?);
    }
    Ok(entries)
}

fn write_entries<W: std::io::Write, T: BorshSerialize>(
    entries: &[T],
    writer: &mut W,
) -> std::io::Result<()> {
    if entries.is_empty() || entries.len() > WRITER_DLMM_ACTION_ENTRIES {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    BorshSerialize::serialize(&(entries.len() as u32), writer)?;
    for entry in entries {
        BorshSerialize::serialize(entry, writer)?;
    }
    Ok(())
}

impl BorshDeserialize for ManageWriterDlmmV1Params {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(match u8::deserialize_reader(reader)? {
            0 => Self::BeginPolicy(BeginWriterDlmmPolicyV1Params::deserialize_reader(reader)?),
            1 => Self::AppendPolicySeries {
                start_index: u8::deserialize_reader(reader)?,
                entries: read_entries(reader)?,
            },
            2 => Self::SealPolicy,
            3 => Self::InitializePosition {
                series_index: u8::deserialize_reader(reader)?,
            },
            4 => Self::AddLiquidity {
                series_index: u8::deserialize_reader(reader)?,
                issue_amount_atoms: u64::deserialize_reader(reader)?,
                entries: read_entries(reader)?,
            },
            5 => Self::RemoveLiquidity {
                series_index: u8::deserialize_reader(reader)?,
                entries: read_entries(reader)?,
            },
            6 => Self::SweepCash {
                series_index: u8::deserialize_reader(reader)?,
            },
            _ => return Err(std::io::ErrorKind::InvalidData.into()),
        })
    }
}

impl BorshSerialize for ManageWriterDlmmV1Params {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        BorshSerialize::serialize(&self.selector(), writer)?;
        match self {
            Self::BeginPolicy(params) => BorshSerialize::serialize(params, writer),
            Self::AppendPolicySeries {
                start_index,
                entries,
            } => {
                BorshSerialize::serialize(start_index, writer)?;
                write_entries(entries, writer)
            }
            Self::SealPolicy => Ok(()),
            Self::InitializePosition { series_index } | Self::SweepCash { series_index } => {
                BorshSerialize::serialize(series_index, writer)
            }
            Self::AddLiquidity {
                series_index,
                issue_amount_atoms,
                entries,
            } => {
                BorshSerialize::serialize(series_index, writer)?;
                BorshSerialize::serialize(issue_amount_atoms, writer)?;
                write_entries(entries, writer)
            }
            Self::RemoveLiquidity {
                series_index,
                entries,
            } => {
                BorshSerialize::serialize(series_index, writer)?;
                write_entries(entries, writer)
            }
        }
    }
}
