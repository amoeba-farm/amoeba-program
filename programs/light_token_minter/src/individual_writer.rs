//! Individually collateralized asks. No pooled receipt, manager authority or fee.
use crate::fixed_codec::{
    fixed_state_deserialize, invalid_fixed_borsh, FixedCursor, FixedField, FixedStateDecode,
    FixedStateEncode, FixedWriter,
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

pub const POSITION_SEED: &[u8] = b"individual-writer-v1";
pub const SCALE: u64 = 1_000_000;

pub fn derive_position(
    program: &Pubkey,
    book: &Pubkey,
    owner: &Pubkey,
    nonce: u64,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            POSITION_SEED,
            book.as_ref(),
            owner.as_ref(),
            &nonce.to_le_bytes(),
        ],
        program,
    )
}

/// Never rounds collateral or buyer payment down. Products fit exactly in u128.
pub fn amount(quantity: u64, price: u64) -> Option<u64> {
    u64::try_from((u128::from(quantity) * u128::from(price)).div_ceil(u128::from(SCALE))).ok()
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IndividualSeries {
    /// Issued liability remains the writer's obligation even after a holder forfeits.
    pub issued: u64,
    pub outstanding: u64,
}
impl FixedField for [IndividualSeries; 20] {
    fn read(input: &mut FixedCursor<'_>) -> Self {
        core::array::from_fn(|_| IndividualSeries::read(input))
    }
    fn write(&self, output: &mut FixedWriter<'_>) {
        for entry in self {
            entry.write(output);
        }
    }
}
fixed_state_deserialize!(IndividualSeries, 16, { issued: u64, outstanding: u64 });

/// Reclaims only the formerly mandatory-zero 12 unused series slots. The book
/// remains 8,312 bytes; its header and all 20 supported series retain their offsets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndividualTotals {
    pub cash_obligations: u64,
    pub open_positions: u64,
    pub funded: bool,
    pub funded_long_liability: u64,
    pub funded_stranded: u64,
    pub series: [IndividualSeries; 20],
    pub reserved: [u8; 2719],
}
impl Default for IndividualTotals {
    fn default() -> Self {
        Self {
            cash_obligations: 0,
            open_positions: 0,
            funded: false,
            funded_long_liability: 0,
            funded_stranded: 0,
            series: [IndividualSeries::default(); 20],
            reserved: [0; 2719],
        }
    }
}
fixed_state_deserialize!(IndividualTotals, 3072, {
    cash_obligations: u64, open_positions: u64, funded: bool, funded_long_liability: u64, funded_stranded: u64,
    series: [IndividualSeries; 20], reserved: [u8; 2719],
});

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IndividualWriterPosition {
    pub initialized: bool,
    pub bump: u8,
    pub discriminator: [u8; 3],
    pub version: u8,
    pub book: Pubkey,
    pub owner: Pubkey,
    pub nonce: u64,
    pub series_index: u8,
    pub payoff_digest: [u8; 32],
    pub expiry_ts: u64,
    pub price: u64,
    pub quantity: u64,
    pub filled: u64,
    pub collateral: u64,
    pub premium: u64,
    pub cancelled: bool,
    pub claimed: bool,
}
impl IndividualWriterPosition {
    pub const LEN: usize = 161;
    pub fn fill(&mut self, quantity: u64, maximum_payment: u64) -> Option<u64> {
        if self.cancelled || self.claimed || quantity == 0 {
            return None;
        }
        let filled = self.filled.checked_add(quantity)?;
        if filled > self.quantity {
            return None;
        }
        let premium = amount(quantity, self.price)?;
        if premium == 0 || premium > maximum_payment {
            return None;
        }
        let total = self.premium.checked_add(premium)?;
        self.filled = filled;
        self.premium = total;
        Some(premium)
    }
    /// Cancel releases only unsold backing and already-earned premiums.
    pub fn cancel(&mut self, max_payout: u64) -> Option<u64> {
        if self.claimed {
            return None;
        }
        let retained = amount(self.filled, max_payout)?;
        let refund = self
            .collateral
            .checked_sub(retained)?
            .checked_add(self.premium)?;
        self.cancelled = true;
        self.collateral = retained;
        self.premium = 0;
        Some(refund)
    }
    pub fn claim(&mut self, payout: u64) -> Option<u64> {
        if self.claimed {
            return None;
        }
        let residual = self
            .collateral
            .checked_sub(amount(self.filled, payout)?)?
            .checked_add(self.premium)?;
        self.claimed = true;
        self.cancelled = true;
        self.collateral = 0;
        self.premium = 0;
        Some(residual)
    }
}
fixed_state_deserialize!(IndividualWriterPosition, IndividualWriterPosition::LEN, {
    initialized: bool, bump: u8, discriminator: [u8; 3], version: u8,
    book: Pubkey, owner: Pubkey, nonce: u64, series_index: u8, payoff_digest: [u8; 32],
    expiry_ts: u64, price: u64, quantity: u64, filled: u64, collateral: u64, premium: u64,
    cancelled: bool, claimed: bool,
});

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub enum IndividualWriterAction {
    Open {
        nonce: u64,
        series_index: u8,
        quantity: u64,
        price: u64,
        maximum_collateral: u64,
    },
    Fill {
        quantity: u64,
        maximum_payment: u64,
    },
    Cancel,
    FundSettlement,
    Claim,
    Close,
}
