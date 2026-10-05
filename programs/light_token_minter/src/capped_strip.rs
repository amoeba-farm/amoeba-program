//! A separate, explicitly quoted writer lane for atomic option strips.
//! It retains every canonical option mint and the existing sleeve's cash.
use crate::{
    state::{OptionKind, WriterDlmmBinV1},
    writer_sleeve_math::WriterSeries,
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

pub const SEED: &[u8] = b"shared-strip-v1";
pub const MAGIC: [u8; 8] = *b"STRIPLV1";
pub const MAX_LEGS: usize = 8;
pub const SERIES: usize = 20;
pub const BINS: usize = 8;
pub const TICK: u64 = 50_000;
pub const WIDTH: u64 = 5_000_000;
pub const LEN: usize = 3047;
pub const COMMON: usize = 27;
pub const FILL_EVENT: &[u8; 8] = b"STRIPFIL";

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct FilledLeg {
    pub series_index: u8,
    pub market: Pubkey,
    pub mint: Pubkey,
    pub gross_quote_atoms: u64,
    pub sponsor_fee_atoms: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct Filled {
    pub version: u8,
    pub owner: Pubkey,
    pub trading_pda: Pubkey,
    pub book: Pubkey,
    pub lane: Pubkey,
    pub direction: u8,
    pub option_quantity: u64,
    pub aggregate_quote_atoms: u64,
    pub legs: Vec<FilledLeg>,
}

crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Witness {
        pub leaf_index: u32,
        pub root_index: u16,
        pub prove_by_index: bool,
    }
}

crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct RetiredLeg {
        pub series_index: u8,
        pub input_amount: u64,
        pub input: Witness,
    }
}
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct Cleanup {
    pub expected_policy_hash: [u8; 32],
    pub proof: Option<[u8; 128]>,
    pub tail_proof: Option<[u8; 128]>,
    pub legs: Vec<RetiredLeg>,
}
impl BorshDeserialize for Cleanup {
    fn deserialize_reader<R: std::io::Read>(r: &mut R) -> std::io::Result<Self> {
        let expected_policy_hash = <[u8; 32]>::deserialize_reader(r)?;
        let proof = Option::<[u8; 128]>::deserialize_reader(r)?;
        let tail_proof = Option::<[u8; 128]>::deserialize_reader(r)?;
        let count = u32::deserialize_reader(r)? as usize;
        if !(1..=8).contains(&count) {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let mut legs = Vec::with_capacity(count);
        for _ in 0..count {
            legs.push(RetiredLeg::deserialize_reader(r)?);
        }
        Ok(Self {
            expected_policy_hash,
            proof,
            tail_proof,
            legs,
        })
    }

    #[inline]
    fn deserialize(buf: &mut &[u8]) -> std::io::Result<Self> {
        crate::fixed_codec::cursor_deserialize(buf)
    }

    #[inline]
    fn try_from_slice(data: &[u8]) -> std::io::Result<Self> {
        crate::fixed_codec::cursor_from_slice(data)
    }
}

/// `Cleanup::deserialize_reader` on a cursor: the same fields, the same leg-count bound checked
/// before the same exact-capacity allocation, then the legs in order.
impl crate::fixed_codec::CursorField for Cleanup {
    #[inline(never)]
    fn read(c: &mut crate::fixed_codec::CheckedCursor<'_>) -> Self {
        let expected_policy_hash = c.bytes();
        let proof = Option::<[u8; 128]>::read(c);
        let tail_proof = Option::<[u8; 128]>::read(c);
        let count = c.u32() as usize;
        let mut legs = Vec::new();
        if c.invalid || !(1..=8).contains(&count) {
            c.invalid = true;
        } else {
            legs = Vec::with_capacity(count);
            for _ in 0..count {
                let leg = RetiredLeg::read(c);
                if c.invalid {
                    break;
                }
                legs.push(leg);
            }
        }
        Self {
            expected_policy_hash,
            proof,
            tail_proof,
            legs,
        }
    }
}

crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Leg {
        pub series_index: u8,
        pub maximum_quote_input: u64,
        pub limit_bin_id: u16,
        pub holder_input_amount: u64,
        pub holder_input: Option<Witness>,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct Trade {
    pub direction: u8,
    pub option_quantity: u64,
    pub aggregate_quote_bound: u64,
    pub deadline: u64,
    pub user_cash_amount: u64,
    pub user_cash: Option<Witness>,
    pub writer_cash: Option<Witness>,
    pub proof: Option<[u8; 128]>,
    pub tail_proof: Option<[u8; 128]>,
    pub legs: Vec<Leg>,
}

impl BorshDeserialize for Trade {
    fn deserialize_reader<R: std::io::Read>(r: &mut R) -> std::io::Result<Self> {
        let mut value = Self {
            direction: u8::deserialize_reader(r)?,
            option_quantity: u64::deserialize_reader(r)?,
            aggregate_quote_bound: u64::deserialize_reader(r)?,
            deadline: u64::deserialize_reader(r)?,
            user_cash_amount: u64::deserialize_reader(r)?,
            user_cash: Option::<Witness>::deserialize_reader(r)?,
            writer_cash: Option::<Witness>::deserialize_reader(r)?,
            proof: Option::<[u8; 128]>::deserialize_reader(r)?,
            tail_proof: Option::<[u8; 128]>::deserialize_reader(r)?,
            legs: Vec::new(),
        };
        let count = u32::deserialize_reader(r)? as usize;
        if !(2..=8).contains(&count) {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        value.legs.reserve(count);
        for _ in 0..count {
            value.legs.push(Leg::deserialize_reader(r)?);
        }
        Ok(value)
    }

    #[inline]
    fn deserialize(buf: &mut &[u8]) -> std::io::Result<Self> {
        crate::fixed_codec::cursor_deserialize(buf)
    }

    #[inline]
    fn try_from_slice(data: &[u8]) -> std::io::Result<Self> {
        crate::fixed_codec::cursor_from_slice(data)
    }
}

/// `Trade::deserialize_reader` on a cursor: the same fields, the same leg-count bound checked
/// before the same `reserve(count)` on the empty leg vector, then the legs in order.
impl crate::fixed_codec::CursorField for Trade {
    #[inline(never)]
    fn read(c: &mut crate::fixed_codec::CheckedCursor<'_>) -> Self {
        let mut value = Self {
            direction: c.u8(),
            option_quantity: c.u64(),
            aggregate_quote_bound: c.u64(),
            deadline: c.u64(),
            user_cash_amount: c.u64(),
            user_cash: Option::<Witness>::read(c),
            writer_cash: Option::<Witness>::read(c),
            proof: Option::<[u8; 128]>::read(c),
            tail_proof: Option::<[u8; 128]>::read(c),
            legs: Vec::new(),
        };
        let count = c.u32() as usize;
        if c.invalid || !(2..=8).contains(&count) {
            c.invalid = true;
            return value;
        }
        value.legs.reserve(count);
        for _ in 0..count {
            let leg = Leg::read(c);
            if c.invalid {
                break;
            }
            value.legs.push(leg);
        }
        value
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct Row {
    pub bin_count: u8,
    pub bins: [WriterDlmmBinV1; BINS],
}
impl Default for Row {
    fn default() -> Self {
        Self {
            bin_count: 0,
            bins: [WriterDlmmBinV1::default(); BINS],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct Lane {
    pub magic: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub sleeve: Pubkey,
    pub group: Pubkey,
    pub book: Pubkey,
    pub policy_hash: [u8; 32],
    pub series_count: u8,
    pub rows: [Row; SERIES],
    pub last_updated_slot: u64,
}

pub fn derive(program: &Pubkey, sleeve: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            SEED,
            sleeve.as_ref(),
        ],
        program,
    )
}

pub fn validate_strip(series: &[WriterSeries], legs: &[Leg]) -> bool {
    if !(2..=8).contains(&legs.len()) {
        return false;
    }
    let mut previous: Option<WriterSeries> = None;
    for leg in legs {
        let Some(current) = series.get(usize::from(leg.series_index)).copied() else {
            return false;
        };
        if current.contract_size_atoms != 1_000_000
            || current.max_payout_per_contract_atoms != WIDTH
        {
            return false;
        }
        let valid = match current.kind {
            OptionKind::CallSpread => {
                current.strike_price_atomic.checked_add(WIDTH) == Some(current.cap_price_atomic)
            }
            OptionKind::PutSpread => {
                current.cap_price_atomic.checked_add(WIDTH) == Some(current.strike_price_atomic)
            }
        };
        if !valid {
            return false;
        }
        if let Some(old) = previous {
            if old.kind != current.kind
                || old.strike_price_atomic.checked_add(WIDTH) != Some(current.strike_price_atomic)
            {
                return false;
            }
        }
        previous = Some(current);
    }
    true
}

pub fn quote_leg(
    trade: &Trade,
    leg: &Leg,
    row: &Row,
    policy: &crate::writer_dlmm_quote::WriterDlmmSwapPolicy<'_>,
) -> Result<
    crate::writer_dlmm_quote::WriterDlmmRouteQuote,
    crate::ameba_dlmm_math::AmoebaDlmmMathError,
> {
    quote_leg_inner(trade, leg, row, policy, None)
}

pub(crate) fn quote_leg_prepared(
    trade: &Trade,
    leg: &Leg,
    row: &Row,
    policy: &crate::writer_dlmm_quote::WriterDlmmSwapPolicy<'_>,
    prepared: &crate::writer_sleeve_math::PreparedWriterReserve,
) -> Result<
    crate::writer_dlmm_quote::WriterDlmmRouteQuote,
    crate::ameba_dlmm_math::AmoebaDlmmMathError,
> {
    quote_leg_inner(trade, leg, row, policy, Some(prepared))
}

fn quote_leg_inner(
    trade: &Trade,
    leg: &Leg,
    row: &Row,
    policy: &crate::writer_dlmm_quote::WriterDlmmSwapPolicy<'_>,
    prepared: Option<&crate::writer_sleeve_math::PreparedWriterReserve>,
) -> Result<
    crate::writer_dlmm_quote::WriterDlmmRouteQuote,
    crate::ameba_dlmm_math::AmoebaDlmmMathError,
> {
    use crate::{
        ameba_dlmm_math::{AmoebaDlmmMathError, AmoebaDlmmSwapDirection},
        writer_dlmm_quote::*,
    };
    let buy = trade.direction == 0;
    if trade.direction > 1
        || trade.option_quantity == 0
        || row.bin_count == 0
        || usize::from(row.bin_count) > BINS
    {
        return Err(AmoebaDlmmMathError::InvalidRoute);
    }
    let config = WriterDlmmRouteConfig {
        direction: if buy {
            AmoebaDlmmSwapDirection::QuoteForOption
        } else {
            AmoebaDlmmSwapDirection::OptionForQuote
        },
        amount_in: if buy {
            leg.maximum_quote_input
        } else {
            trade.option_quantity
        },
        minimum_amount_out: if buy { trade.option_quantity } else { 1 },
        limit_bin_id: leg.limit_bin_id,
        tick_size_quote_atomic: TICK,
        maximum_bin_id: 100,
        maximum_bins: BINS as u8,
        unloaded_ordinary_boundary: None,
    };
    let limits = PublicOrderRouteLimits {
        // Only quote budget may remain unused. The explicit equality below
        // prevents an incomplete option strip at every price and depth.
        allow_partial: buy,
        maximum_option_output: if buy { trade.option_quantity } else { u64::MAX },
        maximum_order_fills: crate::dlmm_order_math::MAX_ORDER_FILLS,
    };
    let bins = &row.bins[..usize::from(row.bin_count)];
    let result = if let Some(prepared) = prepared {
        quote_shared_strip_writer_prepared(config, bins, policy, limits, prepared)
    } else {
        quote_shared_strip_writer(config, bins, policy, limits)
    }?;
    if (buy && result.quote.amount_out != trade.option_quantity)
        || (!buy && result.quote.amount_in != trade.option_quantity)
    {
        return Err(AmoebaDlmmMathError::InsufficientLiquidity);
    }
    Ok(result)
}
