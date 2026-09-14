//! A canonical bounded book authenticates every executable price and FIFO head.
//! Proceeds occupy their own balances until claimed; donations create no rights.
use crate::constants::CURRENT_STATE_NAMESPACE_SEED;
use crate::dlmm_order_math::{OrderBalance, OrderError, OrderSide, MAX_OPEN_ORDERS};
use crate::fixed_codec::{
    fixed_state_deserialize, invalid_fixed_borsh, FixedCursor, FixedField, FixedStateDecode,
    FixedStateEncode, FixedWriter,
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

pub const ORDER_BOOK_SEED: &[u8] = b"dlmm-order-book-v1";
pub const ORDER_POOL_VERSION: u8 = 2;

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
}
impl DlmmOrder {
    pub const LEN: usize = 83;
    pub fn balance(&self, tick: u64) -> Result<OrderBalance, OrderError> {
        Ok(OrderBalance {
            side: match self.side {
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
    claimable_option: u64, claimable_quote: u64,
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
}
impl DlmmOrderBookHeader {
    pub const LEN: usize = 206;
}
fixed_state_deserialize!(DlmmOrderBookHeader, DlmmOrderBookHeader::LEN, {
    initialized: bool, bump: u8, discriminator: [u8; 3], version: u8,
    pool: Pubkey, market: Pubkey, option_mint: Pubkey, quote_mint: Pubkey, rent_payer: Pubkey,
    next_sequence: u64, expiry_ts: u64, option_obligations: u64, quote_obligations: u64,
    continuation_sequence: u64,
});

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DlmmOrderBook {
    pub header: DlmmOrderBookHeader,
    pub orders: Vec<DlmmOrder>,
}
impl DlmmOrderBook {
    pub const LEN: usize = DlmmOrderBookHeader::LEN + 1 + MAX_OPEN_ORDERS * DlmmOrder::LEN;
    pub fn recompute_obligations(&mut self, tick: u64) -> Result<(), OrderError> {
        let mut option = 0u64;
        let mut quote = 0u64;
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
    /// Full bounded book access makes omitted-better-order attacks impossible.
    pub fn priority(&self, side: u8) -> Vec<usize> {
        let mut positions: Vec<_> = self
            .orders
            .iter()
            .enumerate()
            .filter(|(_, order)| {
                order.side == side && order.remaining_quantity > 0 && order.remaining_input > 0
            })
            .map(|(index, _)| index)
            .collect();
        positions.sort_unstable_by(|a, b| {
            let a = &self.orders[*a];
            let b = &self.orders[*b];
            (if side == 0 {
                b.limit_bin.cmp(&a.limit_bin)
            } else {
                a.limit_bin.cmp(&b.limit_bin)
            })
            .then(a.sequence.cmp(&b.sequence))
        });
        positions
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
        let header =
            unsafe { DlmmOrderBookHeader::decode_fixed(&data[..DlmmOrderBookHeader::LEN])? };
        let count = usize::from(data[DlmmOrderBookHeader::LEN]);
        if count > MAX_OPEN_ORDERS {
            return Err(invalid_fixed_borsh());
        }
        let mut orders = Vec::with_capacity(count);
        let mut offset = DlmmOrderBookHeader::LEN + 1;
        for _ in 0..count {
            orders
                .push(unsafe { DlmmOrder::decode_fixed(&data[offset..offset + DlmmOrder::LEN])? });
            offset += DlmmOrder::LEN;
        }
        if data[offset..].iter().any(|byte| *byte != 0) {
            return Err(invalid_fixed_borsh());
        }
        Ok(Self { header, orders })
    }
}
impl FixedStateEncode for DlmmOrderBook {
    fn maximum_encoded_len(&self) -> usize {
        Self::LEN
    }
    fn encode_fixed(&self, data: &mut [u8]) {
        self.header
            .encode_fixed(&mut data[..DlmmOrderBookHeader::LEN]);
        data[DlmmOrderBookHeader::LEN] = self.orders.len() as u8;
        let mut offset = DlmmOrderBookHeader::LEN + 1;
        for order in &self.orders {
            order.encode_fixed(&mut data[offset..offset + DlmmOrder::LEN]);
            offset += DlmmOrder::LEN;
        }
        data[offset..].fill(0);
    }
}
pub fn derive_order_book(program: &Pubkey, pool: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[CURRENT_STATE_NAMESPACE_SEED, ORDER_BOOK_SEED, pool.as_ref()],
        program,
    )
}

#[derive(Clone, Debug, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum DlmmOrderAction {
    Initialize,
    Place {
        expected_sequence: u64,
        side: u8,
        limit_bin: u16,
        quantity: u64,
        post_only: bool,
    },
    Cancel {
        sequence: u64,
    },
    Claim {
        sequence: u64,
    },
    Close {
        sequence: u64,
    },
    Match {
        side: u8,
        maximum_fills: u8,
    },
    Swap {
        params: crate::ameba_dlmm_instruction::SwapAmoebaDlmmExactInV1Params,
    },
    CloseBook,
}
