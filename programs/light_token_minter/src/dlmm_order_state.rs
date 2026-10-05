//! Canonical linked price/FIFO queues with independently reclaimable order records.
//! Proceeds occupy their own balances until claimed; donations create no rights.
use crate::constants::CURRENT_STATE_NAMESPACE_SEED;
use crate::dlmm_order_math::{OrderBalance, OrderError, OrderSide};
use crate::fixed_codec::{
    fixed_state_deserialize, invalid_fixed_borsh, FixedCursor, FixedField, FixedStateDecode,
    FixedStateEncode, FixedWriter,
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;
use std::io::{self, Read, Write};

pub const ORDER_BOOK_SEED: &[u8] = b"dlmm-order-book-v1";
pub const ORDER_POOL_VERSION: u8 = 4;
pub const ORDER_RECORD_SEED: &[u8] = b"order-record-g3";
pub const MAX_ORDER_WITNESSES: usize = 24;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DlmmOrder {
    pub owner: Pubkey,
    pub sequence: u64,
    pub side: u8,
    pub limit_bin: u16,
    pub original_quantity: u64,
    pub remaining_quantity: u64,
    pub remaining_input: u64,
    pub claimable_option: u64,
    pub claimable_quote: u64,
    pub previous: u64,
    pub next: u64,
}
impl DlmmOrder {
    pub const LEN: usize = 99;
    pub const COMPRESSED_DELEGATE_CONSENT: u8 = 0x80;
    pub const COMPRESSED_ESCROW_FUNDING: u8 = 0x40;
    pub fn order_side(&self) -> u8 {
        self.side & 1
    }
    pub fn has_compressed_delegate_consent(&self) -> bool {
        self.order_side() == 0 && self.side & Self::COMPRESSED_DELEGATE_CONSENT != 0
    }
    pub fn has_compressed_escrow_funding(&self) -> bool {
        self.side & Self::COMPRESSED_ESCROW_FUNDING != 0
    }
    pub fn valid_side_flags(&self) -> bool {
        self.side & Self::COMPRESSED_ESCROW_FUNDING != 0
            && self.side
                & !(1 | Self::COMPRESSED_ESCROW_FUNDING | Self::COMPRESSED_DELEGATE_CONSENT)
                == 0
            && (self.side & Self::COMPRESSED_DELEGATE_CONSENT == 0 || self.order_side() == 0)
    }
    pub fn balance(&self, tick: u64) -> Result<OrderBalance, OrderError> {
        Ok(OrderBalance {
            side: match self.order_side() {
                0 => OrderSide::Bid,
                1 => OrderSide::Ask,
                _ => return Err(OrderError::InvalidOrder),
            },
            limit_price: tick
                .checked_mul(u64::from(self.limit_bin))
                .ok_or(OrderError::Overflow)?,
            original_quantity: self.original_quantity,
            remaining_quantity: self.remaining_quantity,
            remaining_input: self.remaining_input,
            claimable_option: self.claimable_option,
            claimable_quote: self.claimable_quote,
        })
    }
    pub fn set_balance(&mut self, balance: OrderBalance) {
        self.remaining_quantity = balance.remaining_quantity;
        self.remaining_input = balance.remaining_input;
        self.claimable_option = balance.claimable_option;
        self.claimable_quote = balance.claimable_quote;
    }
}
fixed_state_deserialize!(DlmmOrder, DlmmOrder::LEN, {
    owner: Pubkey, sequence: u64, side: u8, limit_bin: u16,
    original_quantity: u64, remaining_quantity: u64, remaining_input: u64,
    claimable_option: u64, claimable_quote: u64, previous: u64, next: u64,
});

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DlmmOrderBookHeader {
    pub initialized: bool,
    pub bump: u8,
    pub discriminator: [u8; 3],
    pub version: u8,
    pub pool: Pubkey,
    pub market: Pubkey,
    pub option_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub rent_payer: Pubkey,
    pub next_sequence: u64,
    pub expiry_ts: u64,
    pub option_obligations: u64,
    pub quote_obligations: u64,
    pub continuation_sequence: u64,
    pub bid_head: u64,
    pub ask_head: u64,
    pub record_count: u64,
}
impl DlmmOrderBookHeader {
    pub const LEN: usize = 230;
}
fixed_state_deserialize!(DlmmOrderBookHeader, DlmmOrderBookHeader::LEN, {
    initialized: bool, bump: u8, discriminator: [u8; 3], version: u8,
    pool: Pubkey, market: Pubkey, option_mint: Pubkey, quote_mint: Pubkey, rent_payer: Pubkey,
    next_sequence: u64, expiry_ts: u64, option_obligations: u64, quote_obligations: u64,
    continuation_sequence: u64, bid_head: u64, ask_head: u64, record_count: u64,
});

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DlmmOrderBook {
    pub header: DlmmOrderBookHeader,
    pub orders: Vec<DlmmOrder>,
    pub unloaded_option: u64,
    pub unloaded_quote: u64,
}
impl DlmmOrderBook {
    pub const LEN: usize = DlmmOrderBookHeader::LEN;
    pub fn recompute_obligations(&mut self, tick: u64) -> Result<(), OrderError> {
        let mut option = self.unloaded_option;
        let mut quote = self.unloaded_quote;
        for order in &self.orders {
            let (o, q) = order.balance(tick)?.obligations()?;
            option = option.checked_add(o).ok_or(OrderError::Overflow)?;
            quote = quote.checked_add(q).ok_or(OrderError::Overflow)?;
        }
        self.header.option_obligations = option;
        self.header.quote_obligations = quote;
        if !self.orders.iter().any(|order| {
            order.sequence == self.header.continuation_sequence
                && order.remaining_input > 0
                && order.remaining_quantity > 0
        }) {
            self.header.continuation_sequence = 0;
        }
        Ok(())
    }
    /// Only a contiguous authenticated prefix from the canonical head can execute.
    pub fn priority(&self, side: u8) -> Vec<usize> {
        let mut next = if side == 0 {
            self.header.bid_head
        } else {
            self.header.ask_head
        };
        let mut out = Vec::new();
        for _ in 0..MAX_ORDER_WITNESSES {
            if next == 0 {
                break;
            }
            let Some(index) = self.orders.iter().position(|order| order.sequence == next) else {
                break;
            };
            let order = &self.orders[index];
            if order.order_side() != side || out.contains(&index) {
                break;
            }
            if order.remaining_quantity > 0 && order.remaining_input > 0 {
                out.push(index);
            }
            next = order.next;
        }
        out
    }
    /// A crossing pair continues with the newer side as taker. Otherwise the
    /// requested side's canonical head can continue against LP/writer liquidity.
    pub fn next_match_sequence(&self, side: u8) -> u64 {
        let bids = self.priority(0);
        let asks = self.priority(1);
        if let (Some(bid), Some(ask)) = (bids.first(), asks.first()) {
            let bid = &self.orders[*bid];
            let ask = &self.orders[*ask];
            if bid.limit_bin >= ask.limit_bin {
                return bid.sequence.max(ask.sequence);
            }
        }
        let queue = if side == 0 { bids } else { asks };
        queue
            .first()
            .map_or(0, |index| self.orders[*index].sequence)
    }
}
impl FixedStateDecode for DlmmOrderBook {
    const REQUIRED_DATA_LEN: usize = Self::LEN;
    unsafe fn decode_fixed(data: &[u8]) -> std::io::Result<Self> {
        if data.len() != Self::LEN {
            return Err(invalid_fixed_borsh());
        }
        let header = unsafe { DlmmOrderBookHeader::decode_fixed(data)? };
        Ok(Self {
            unloaded_option: header.option_obligations,
            unloaded_quote: header.quote_obligations,
            header,
            orders: Vec::new(),
        })
    }
}
impl FixedStateEncode for DlmmOrderBook {
    fn maximum_encoded_len(&self) -> usize {
        Self::LEN
    }
    fn encode_fixed(&self, data: &mut [u8]) {
        self.header.encode_fixed(data);
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DlmmOrderRecord {
    pub initialized: bool,
    pub bump: u8,
    pub discriminator: [u8; 3],
    pub version: u8,
    pub book: Pubkey,
    pub order: DlmmOrder,
}
impl DlmmOrderRecord {
    pub const LEN: usize = 137;
}
fixed_state_deserialize!(DlmmOrderRecord, DlmmOrderRecord::LEN, {
    initialized: bool, bump: u8, discriminator: [u8; 3], version: u8, book: Pubkey, order: DlmmOrder,
});
pub fn derive_order_record(program: &Pubkey, book: &Pubkey, sequence: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORDER_RECORD_SEED,
            book.as_ref(),
            &sequence.to_le_bytes(),
        ],
        program,
    )
}

pub fn derive_order_book(program: &Pubkey, pool: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[CURRENT_STATE_NAMESPACE_SEED, ORDER_BOOK_SEED, pool.as_ref()],
        program,
    )
}

crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, PartialEq, BorshSerialize)]
    pub struct CompressedOrderExitV1Params {
        pub sequence: u64,
        pub record_count: u8,
        pub merkle_account_count: u8,
        pub output_tree_index: u8,
        pub output_queue_index: u8,
        pub expected_option_atoms: u64,
        pub expected_quote_atoms: u64,
        pub book_option_input: Option<crate::ameba_dlmm_instruction::CompressedSwapLeafWitnessV1>,
        pub book_quote_input: Option<crate::ameba_dlmm_instruction::CompressedSwapLeafWitnessV1>,
        pub fee_input_amount: u64,
        pub fee_input: Option<crate::ameba_dlmm_instruction::CompressedSwapLeafWitnessV1>,
        pub proof: Option<[u8; 128]>,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DlmmOrderAction {
    Initialize,
    /// Recovery only for the deployed classic owner-funded records.
    Cancel {
        sequence: u64,
    },
    Claim {
        sequence: u64,
    },
    Close {
        sequence: u64,
    },
    CloseBook,
    /// Permissionless post-expiry recovery to the authenticated classic owner.
    ExpireClassic {
        sequence: u64,
        record_count: u8,
    },
    /// Future owner-signed regular compressed Book funding.
    PlaceCompressedEscrow {
        expected_sequence: u64,
        side: u8,
        limit_bin: u16,
        quantity: u64,
        post_only: bool,
        record_count: u8,
        page_count: u8,
        merkle_account_count: u8,
        output_tree_index: u8,
        output_queue_index: u8,
        funding_mode: u8,
        user_input_amount: u64,
        user_input_has_delegate: bool,
        user_input: Option<crate::ameba_dlmm_instruction::CompressedSwapLeafWitnessV1>,
        book_option_input: Option<crate::ameba_dlmm_instruction::CompressedSwapLeafWitnessV1>,
        book_quote_input: Option<crate::ameba_dlmm_instruction::CompressedSwapLeafWitnessV1>,
        pool_option_input: Option<crate::ameba_dlmm_instruction::CompressedSwapLeafWitnessV1>,
        pool_quote_input: Option<crate::ameba_dlmm_instruction::CompressedSwapLeafWitnessV1>,
        writer_quote_input: Option<crate::ameba_dlmm_instruction::CompressedSwapLeafWitnessV1>,
        sponsor_fee_atoms: u64,
        fee_input_mode: u8,
        fee_input_amount: u64,
        fee_input: Option<crate::ameba_dlmm_instruction::CompressedSwapLeafWitnessV1>,
        proof: Option<[u8; 128]>,
    },
    CancelCompressedEscrow {
        params: CompressedOrderExitV1Params,
    },
    ClaimCompressedEscrow {
        params: CompressedOrderExitV1Params,
    },
}

// Historical owner recovery bytes 2/3/4/7 and permissionless expired classic
// recovery 15 remain live. Retired placement/matching/consent bytes still reject.
impl BorshSerialize for DlmmOrderAction {
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        match self {
            Self::Initialize => 0u8.serialize(writer),
            Self::Cancel { sequence } => {
                2u8.serialize(writer)?;
                sequence.serialize(writer)
            }
            Self::Claim { sequence } => {
                3u8.serialize(writer)?;
                sequence.serialize(writer)
            }
            Self::Close { sequence } => {
                4u8.serialize(writer)?;
                sequence.serialize(writer)
            }
            Self::CloseBook => 7u8.serialize(writer),
            Self::ExpireClassic {
                sequence,
                record_count,
            } => {
                15u8.serialize(writer)?;
                sequence.serialize(writer)?;
                record_count.serialize(writer)
            }
            Self::PlaceCompressedEscrow {
                expected_sequence,
                side,
                limit_bin,
                quantity,
                post_only,
                record_count,
                page_count,
                merkle_account_count,
                output_tree_index,
                output_queue_index,
                funding_mode,
                user_input_amount,
                user_input_has_delegate,
                user_input,
                book_option_input,
                book_quote_input,
                pool_option_input,
                pool_quote_input,
                writer_quote_input,
                sponsor_fee_atoms,
                fee_input_mode,
                fee_input_amount,
                fee_input,
                proof,
            } => {
                12u8.serialize(writer)?;
                expected_sequence.serialize(writer)?;
                side.serialize(writer)?;
                limit_bin.serialize(writer)?;
                quantity.serialize(writer)?;
                post_only.serialize(writer)?;
                record_count.serialize(writer)?;
                page_count.serialize(writer)?;
                merkle_account_count.serialize(writer)?;
                output_tree_index.serialize(writer)?;
                output_queue_index.serialize(writer)?;
                funding_mode.serialize(writer)?;
                user_input_amount.serialize(writer)?;
                user_input_has_delegate.serialize(writer)?;
                user_input.serialize(writer)?;
                book_option_input.serialize(writer)?;
                book_quote_input.serialize(writer)?;
                pool_option_input.serialize(writer)?;
                pool_quote_input.serialize(writer)?;
                writer_quote_input.serialize(writer)?;
                sponsor_fee_atoms.serialize(writer)?;
                fee_input_mode.serialize(writer)?;
                fee_input_amount.serialize(writer)?;
                fee_input.serialize(writer)?;
                proof.serialize(writer)
            }
            Self::CancelCompressedEscrow { params } => {
                13u8.serialize(writer)?;
                params.serialize(writer)
            }
            Self::ClaimCompressedEscrow { params } => {
                14u8.serialize(writer)?;
                params.serialize(writer)
            }
        }
    }
}

impl BorshDeserialize for DlmmOrderAction {
    fn deserialize_reader<R: Read>(reader: &mut R) -> io::Result<Self> {
        match u8::deserialize_reader(reader)? {
            0 => Ok(Self::Initialize),
            2 => Ok(Self::Cancel {
                sequence: u64::deserialize_reader(reader)?,
            }),
            3 => Ok(Self::Claim {
                sequence: u64::deserialize_reader(reader)?,
            }),
            4 => Ok(Self::Close {
                sequence: u64::deserialize_reader(reader)?,
            }),
            7 => Ok(Self::CloseBook),
            15 => Ok(Self::ExpireClassic {
                sequence: u64::deserialize_reader(reader)?,
                record_count: u8::deserialize_reader(reader)?,
            }),
            12 => Ok(Self::PlaceCompressedEscrow {
                expected_sequence: u64::deserialize_reader(reader)?,
                side: u8::deserialize_reader(reader)?,
                limit_bin: u16::deserialize_reader(reader)?,
                quantity: u64::deserialize_reader(reader)?,
                post_only: bool::deserialize_reader(reader)?,
                record_count: u8::deserialize_reader(reader)?,
                page_count: u8::deserialize_reader(reader)?,
                merkle_account_count: u8::deserialize_reader(reader)?,
                output_tree_index: u8::deserialize_reader(reader)?,
                output_queue_index: u8::deserialize_reader(reader)?,
                funding_mode: u8::deserialize_reader(reader)?,
                user_input_amount: u64::deserialize_reader(reader)?,
                user_input_has_delegate: bool::deserialize_reader(reader)?,
                user_input: Option::deserialize_reader(reader)?,
                book_option_input: Option::deserialize_reader(reader)?,
                book_quote_input: Option::deserialize_reader(reader)?,
                pool_option_input: Option::deserialize_reader(reader)?,
                pool_quote_input: Option::deserialize_reader(reader)?,
                writer_quote_input: Option::deserialize_reader(reader)?,
                sponsor_fee_atoms: u64::deserialize_reader(reader)?,
                fee_input_mode: u8::deserialize_reader(reader)?,
                fee_input_amount: u64::deserialize_reader(reader)?,
                fee_input: Option::deserialize_reader(reader)?,
                proof: Option::deserialize_reader(reader)?,
            }),
            13 => Ok(Self::CancelCompressedEscrow {
                params: CompressedOrderExitV1Params::deserialize_reader(reader)?,
            }),
            14 => Ok(Self::ClaimCompressedEscrow {
                params: CompressedOrderExitV1Params::deserialize_reader(reader)?,
            }),
            // Retired order selector.
            _ => Err(io::Error::from(io::ErrorKind::InvalidData)),
        }
    }

    #[inline]
    fn deserialize(buf: &mut &[u8]) -> io::Result<Self> {
        crate::fixed_codec::cursor_deserialize(buf)
    }

    /// Borsh's rule: decode, then reject any unread byte.
    #[inline]
    fn try_from_slice(data: &[u8]) -> io::Result<Self> {
        crate::fixed_codec::cursor_from_slice(data)
    }
}

/// The slice decoder the program uses: the same tags, fields and order as
/// `deserialize_reader` above (the tested reference), read from a
/// `CheckedCursor`; an unknown tag poisons the cursor where Borsh errors.
impl crate::fixed_codec::CursorField for DlmmOrderAction {
    #[inline(never)]
    fn read(input: &mut crate::fixed_codec::CheckedCursor<'_>) -> Self {
        use crate::fixed_codec::CursorField as F;
        match input.u8() {
            0 => Self::Initialize,
            2 => Self::Cancel {
                sequence: input.u64(),
            },
            3 => Self::Claim {
                sequence: input.u64(),
            },
            4 => Self::Close {
                sequence: input.u64(),
            },
            7 => Self::CloseBook,
            15 => Self::ExpireClassic {
                sequence: input.u64(),
                record_count: input.u8(),
            },
            12 => Self::PlaceCompressedEscrow {
                expected_sequence: input.u64(),
                side: input.u8(),
                limit_bin: input.u16(),
                quantity: input.u64(),
                post_only: input.boolean(),
                record_count: input.u8(),
                page_count: input.u8(),
                merkle_account_count: input.u8(),
                output_tree_index: input.u8(),
                output_queue_index: input.u8(),
                funding_mode: input.u8(),
                user_input_amount: input.u64(),
                user_input_has_delegate: input.boolean(),
                user_input: F::read(input),
                book_option_input: F::read(input),
                book_quote_input: F::read(input),
                pool_option_input: F::read(input),
                pool_quote_input: F::read(input),
                writer_quote_input: F::read(input),
                sponsor_fee_atoms: input.u64(),
                fee_input_mode: input.u8(),
                fee_input_amount: input.u64(),
                fee_input: F::read(input),
                proof: F::read(input),
            },
            13 => Self::CancelCompressedEscrow {
                params: F::read(input),
            },
            14 => Self::ClaimCompressedEscrow {
                params: F::read(input),
            },
            _ => {
                input.invalid = true;
                Self::Initialize
            }
        }
    }
}
