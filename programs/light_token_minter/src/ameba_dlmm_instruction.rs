//! Wire contract for the in-program DLMM. These tags use the Borsh ABI required
//! by the pinned Light account proof types used by initialization.

use std::io::{Error, ErrorKind, Read, Result as IoResult};

use borsh::{BorshDeserialize, BorshSerialize};
use light_compressed_account::instruction_data::{
    compressed_proof::{CompressedProof, ValidityProof},
    data::PackedAddressTreeInfo,
};
use light_sdk_types::interface::create_accounts_proof::CreateAccountsProof;
use solana_program::pubkey::Pubkey;

use crate::{ameba_dlmm_state::AmoebaDlmmPoolStatus, constants::MAX_AMOEBA_DLMM_LIQUIDITY_ENTRIES};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AmoebaDlmmInstructionTag {
    InitializeBinPageV1 = 207,
    InitializePositionV1 = 208,
    AddLiquidityV1 = 209,
    RemoveLiquidityV1 = 210,
    CollectProtocolFeesV1 = 213,
    ClosePoolV1 = 215,
    InitializeLightConfig = 216,
    UpdateLightConfig = 217,
    CompressLightState = 218,
    DecompressLightState = 219,
    InitializeCollectiveDlmmPoolV1 = 252,
    SetCollectiveDlmmPoolStatusV1 = 253,
    SwapCollectiveDlmmExactInV1 = 254,
    SettleCollectiveDlmmPoolV1 = 255,
}

impl AmoebaDlmmInstructionTag {
    pub fn from_byte(value: u8) -> Option<Self> {
        const VALID_TAGS: u64 = 0xf000_0000_0fa7_8000;
        if value < 192 || VALID_TAGS & (1u64 << (value - 192)) == 0 {
            None
        } else {
            // SAFETY: the bitmap contains exactly the declared repr(u8) discriminants above.
            Some(unsafe { core::mem::transmute::<u8, Self>(value) })
        }
    }
}

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct InitializeAmoebaDlmmPoolV1Params {
    /// Light address-registration proof.  This is lifecycle infrastructure,
    /// not a caller-selected economic parameter.
    pub create_accounts_proof: CreateAccountsProof,
    pub liquidity_manager: Pubkey,
    pub protocol_fee_share_bps: u16,
}

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct InitializeAmoebaDlmmBinPageV1Params {
    pub create_accounts_proof: CreateAccountsProof,
    pub page_index: u16,
}

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct InitializeAmoebaDlmmPositionV1Params {
    pub create_accounts_proof: CreateAccountsProof,
    pub position_nonce: u64,
    pub lower_bin_id: u16,
    pub bin_count: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct AmoebaDlmmDepositEntry {
    pub bin_id: u16,
    pub maximum_option_amount: u64,
    pub maximum_quote_amount: u64,
    pub minimum_shares: u128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddAmoebaDlmmLiquidityV1Params {
    pub position_nonce: u64,
    pub entries: Vec<AmoebaDlmmDepositEntry>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct AmoebaDlmmRemoveEntry {
    pub bin_id: u16,
    pub shares: u128,
    pub minimum_option_out: u64,
    pub minimum_quote_out: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoveAmoebaDlmmLiquidityV1Params {
    pub position_nonce: u64,
    pub entries: Vec<AmoebaDlmmRemoveEntry>,
    pub close_position_when_empty: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
#[repr(u8)]
pub enum AmoebaDlmmSwapDirection {
    QuoteForOption = 0,
    OptionForQuote = 1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct SwapAmoebaDlmmExactInV1Params {
    pub direction: AmoebaDlmmSwapDirection,
    pub amount_in: u64,
    pub minimum_amount_out: u64,
    pub limit_bin_id: u16,
    pub deadline_ts: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct SetAmoebaDlmmPoolStatusV1Params {
    pub status: AmoebaDlmmPoolStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct CollectAmoebaDlmmProtocolFeesV1Params {
    pub option_amount: u64,
    pub quote_amount: u64,
}

pub trait AmoebaDlmmDecode: Sized {
    fn decode_exact(payload: &[u8]) -> Result<Self, ()>;
}

struct PayloadReader<'a> {
    remaining: &'a [u8],
}

impl<'a> PayloadReader<'a> {
    #[inline(always)]
    fn new(payload: &'a [u8]) -> Self {
        Self { remaining: payload }
    }

    #[inline(always)]
    fn take<const LENGTH: usize>(&mut self) -> Result<[u8; LENGTH], ()> {
        let (value, remaining) = self.remaining.split_at_checked(LENGTH).ok_or(())?;
        self.remaining = remaining;
        value.try_into().map_err(|_| ())
    }

    #[inline(always)]
    fn u8(&mut self) -> Result<u8, ()> {
        Ok(self.take::<1>()?[0])
    }

    #[inline(always)]
    fn u16(&mut self) -> Result<u16, ()> {
        Ok(u16::from_le_bytes(self.take()?))
    }

    #[inline(always)]
    fn u32(&mut self) -> Result<u32, ()> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    #[inline(always)]
    fn u64(&mut self) -> Result<u64, ()> {
        Ok(u64::from_le_bytes(self.take()?))
    }

    #[inline(always)]
    fn u128(&mut self) -> Result<u128, ()> {
        Ok(u128::from_le_bytes(self.take()?))
    }

    #[inline(always)]
    fn boolean(&mut self) -> Result<bool, ()> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(()),
        }
    }

    #[inline(always)]
    fn finish(self) -> Result<(), ()> {
        if self.remaining.is_empty() {
            Ok(())
        } else {
            Err(())
        }
    }
}

#[inline(always)]
fn read_create_accounts_proof(reader: &mut PayloadReader<'_>) -> Result<CreateAccountsProof, ()> {
    let proof = match reader.u8()? {
        0 => ValidityProof(None),
        1 => ValidityProof(Some(CompressedProof {
            a: reader.take()?,
            b: reader.take()?,
            c: reader.take()?,
        })),
        _ => return Err(()),
    };
    let address_tree_info = PackedAddressTreeInfo {
        address_merkle_tree_pubkey_index: reader.u8()?,
        address_queue_pubkey_index: reader.u8()?,
        root_index: reader.u16()?,
    };
    let output_state_tree_index = reader.u8()?;
    let state_tree_index = match reader.u8()? {
        0 => None,
        1 => Some(reader.u8()?),
        _ => return Err(()),
    };
    Ok(CreateAccountsProof {
        proof,
        address_tree_info,
        output_state_tree_index,
        state_tree_index,
        system_accounts_offset: reader.u8()?,
    })
}

impl AmoebaDlmmDecode for InitializeAmoebaDlmmPoolV1Params {
    fn decode_exact(payload: &[u8]) -> Result<Self, ()> {
        let mut reader = PayloadReader::new(payload);
        let value = Self {
            create_accounts_proof: read_create_accounts_proof(&mut reader)?,
            liquidity_manager: Pubkey::new_from_array(reader.take()?),
            protocol_fee_share_bps: reader.u16()?,
        };
        reader.finish()?;
        Ok(value)
    }
}

impl AmoebaDlmmDecode for InitializeAmoebaDlmmBinPageV1Params {
    fn decode_exact(payload: &[u8]) -> Result<Self, ()> {
        let mut reader = PayloadReader::new(payload);
        let value = Self {
            create_accounts_proof: read_create_accounts_proof(&mut reader)?,
            page_index: reader.u16()?,
        };
        reader.finish()?;
        Ok(value)
    }
}

impl AmoebaDlmmDecode for InitializeAmoebaDlmmPositionV1Params {
    fn decode_exact(payload: &[u8]) -> Result<Self, ()> {
        let mut reader = PayloadReader::new(payload);
        let value = Self {
            create_accounts_proof: read_create_accounts_proof(&mut reader)?,
            position_nonce: reader.u64()?,
            lower_bin_id: reader.u16()?,
            bin_count: reader.u8()?,
        };
        reader.finish()?;
        Ok(value)
    }
}

impl AmoebaDlmmDecode for AddAmoebaDlmmLiquidityV1Params {
    fn decode_exact(payload: &[u8]) -> Result<Self, ()> {
        let mut reader = PayloadReader::new(payload);
        let position_nonce = reader.u64()?;
        let count = reader.u32()? as usize;
        if !(1..=MAX_AMOEBA_DLMM_LIQUIDITY_ENTRIES).contains(&count) {
            return Err(());
        }
        let mut entries = Vec::with_capacity(count);
        let mut previous_bin = None;
        for _ in 0..count {
            let bin_id = reader.u16()?;
            if previous_bin.is_some_and(|previous| previous >= bin_id) {
                return Err(());
            }
            previous_bin = Some(bin_id);
            entries.push(AmoebaDlmmDepositEntry {
                bin_id,
                maximum_option_amount: reader.u64()?,
                maximum_quote_amount: reader.u64()?,
                minimum_shares: reader.u128()?,
            });
        }
        reader.finish()?;
        Ok(Self {
            position_nonce,
            entries,
        })
    }
}

impl AmoebaDlmmDecode for RemoveAmoebaDlmmLiquidityV1Params {
    fn decode_exact(payload: &[u8]) -> Result<Self, ()> {
        let mut reader = PayloadReader::new(payload);
        let position_nonce = reader.u64()?;
        let count = reader.u32()? as usize;
        if !(1..=MAX_AMOEBA_DLMM_LIQUIDITY_ENTRIES).contains(&count) {
            return Err(());
        }
        let mut entries = Vec::with_capacity(count);
        let mut previous_bin = None;
        for _ in 0..count {
            let bin_id = reader.u16()?;
            if previous_bin.is_some_and(|previous| previous >= bin_id) {
                return Err(());
            }
            previous_bin = Some(bin_id);
            entries.push(AmoebaDlmmRemoveEntry {
                bin_id,
                shares: reader.u128()?,
                minimum_option_out: reader.u64()?,
                minimum_quote_out: reader.u64()?,
            });
        }
        let close_position_when_empty = reader.boolean()?;
        reader.finish()?;
        Ok(Self {
            position_nonce,
            entries,
            close_position_when_empty,
        })
    }
}

impl AmoebaDlmmDecode for SwapAmoebaDlmmExactInV1Params {
    fn decode_exact(payload: &[u8]) -> Result<Self, ()> {
        let mut reader = PayloadReader::new(payload);
        let direction = match reader.u8()? {
            0 => AmoebaDlmmSwapDirection::QuoteForOption,
            1 => AmoebaDlmmSwapDirection::OptionForQuote,
            _ => return Err(()),
        };
        let value = Self {
            direction,
            amount_in: reader.u64()?,
            minimum_amount_out: reader.u64()?,
            limit_bin_id: reader.u16()?,
            deadline_ts: reader.u64()?,
        };
        reader.finish()?;
        Ok(value)
    }
}

impl AmoebaDlmmDecode for SetAmoebaDlmmPoolStatusV1Params {
    fn decode_exact(payload: &[u8]) -> Result<Self, ()> {
        let status = match payload {
            [0] => AmoebaDlmmPoolStatus::Pending,
            [1] => AmoebaDlmmPoolStatus::Active,
            [2] => AmoebaDlmmPoolStatus::Paused,
            [3] => AmoebaDlmmPoolStatus::Settled,
            [4] => AmoebaDlmmPoolStatus::Closed,
            _ => return Err(()),
        };
        Ok(Self { status })
    }
}

impl AmoebaDlmmDecode for CollectAmoebaDlmmProtocolFeesV1Params {
    fn decode_exact(payload: &[u8]) -> Result<Self, ()> {
        let mut reader = PayloadReader::new(payload);
        let value = Self {
            option_amount: reader.u64()?,
            quote_amount: reader.u64()?,
        };
        reader.finish()?;
        Ok(value)
    }
}

fn invalid_data(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

fn read_bounded_entries<T: BorshDeserialize, R: Read>(reader: &mut R) -> IoResult<Vec<T>> {
    let count = u32::deserialize_reader(reader)? as usize;
    if !(1..=MAX_AMOEBA_DLMM_LIQUIDITY_ENTRIES).contains(&count) {
        return Err(invalid_data("invalid Amoeba DLMM liquidity entry count"));
    }
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(T::deserialize_reader(reader)?);
    }
    Ok(entries)
}

impl BorshDeserialize for AddAmoebaDlmmLiquidityV1Params {
    fn deserialize_reader<R: Read>(reader: &mut R) -> IoResult<Self> {
        let position_nonce = u64::deserialize_reader(reader)?;
        let entries: Vec<AmoebaDlmmDepositEntry> = read_bounded_entries(reader)?;
        if entries
            .windows(2)
            .any(|pair| pair[0].bin_id >= pair[1].bin_id)
        {
            return Err(invalid_data("DLMM deposit bins are not strictly ascending"));
        }
        Ok(Self {
            position_nonce,
            entries,
        })
    }
}

impl BorshSerialize for AddAmoebaDlmmLiquidityV1Params {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> IoResult<()> {
        if !(1..=MAX_AMOEBA_DLMM_LIQUIDITY_ENTRIES).contains(&self.entries.len())
            || self
                .entries
                .windows(2)
                .any(|pair| pair[0].bin_id >= pair[1].bin_id)
        {
            return Err(invalid_data("invalid DLMM deposit entries"));
        }
        self.position_nonce.serialize(writer)?;
        (self.entries.len() as u32).serialize(writer)?;
        for entry in &self.entries {
            entry.serialize(writer)?;
        }
        Ok(())
    }
}

impl BorshDeserialize for RemoveAmoebaDlmmLiquidityV1Params {
    fn deserialize_reader<R: Read>(reader: &mut R) -> IoResult<Self> {
        let position_nonce = u64::deserialize_reader(reader)?;
        let entries: Vec<AmoebaDlmmRemoveEntry> = read_bounded_entries(reader)?;
        if entries
            .windows(2)
            .any(|pair| pair[0].bin_id >= pair[1].bin_id)
        {
            return Err(invalid_data("DLMM removal bins are not strictly ascending"));
        }
        let close_position_when_empty = bool::deserialize_reader(reader)?;
        Ok(Self {
            position_nonce,
            entries,
            close_position_when_empty,
        })
    }
}

impl BorshSerialize for RemoveAmoebaDlmmLiquidityV1Params {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> IoResult<()> {
        if !(1..=MAX_AMOEBA_DLMM_LIQUIDITY_ENTRIES).contains(&self.entries.len())
            || self
                .entries
                .windows(2)
                .any(|pair| pair[0].bin_id >= pair[1].bin_id)
        {
            return Err(invalid_data("invalid DLMM removal entries"));
        }
        self.position_nonce.serialize(writer)?;
        (self.entries.len() as u32).serialize(writer)?;
        for entry in &self.entries {
            entry.serialize(writer)?;
        }
        self.close_position_when_empty.serialize(writer)
    }
}

pub fn decode_exact<T: AmoebaDlmmDecode>(payload: &[u8]) -> Result<T, ()> {
    T::decode_exact(payload)
}
