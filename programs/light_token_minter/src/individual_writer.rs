//! Owner-portfolio collateralized asks. Each record is a price/quantity witness;
//! its `collateral` wire field records only that Open's initial cash top-up.
use crate::fixed_codec::{
    fixed_state_deserialize, invalid_fixed_borsh, FixedCursor, FixedField, FixedStateDecode,
    FixedStateEncode, FixedWriter,
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

pub const POSITION_SEED: &[u8] = b"individual-writer-v1";
pub const SCALE: u64 = 1_000_000;
pub const MAX_BUYBACK_LEGS: usize = 8;

/// Fixed-size payload: malformed lengths cannot allocate an unbounded leg list.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct IndividualBuybackLegV1 {
    pub series_index: u8,
    pub quantity: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct IndividualBuybackV1 {
    pub leg_count: u8,
    pub legs: [IndividualBuybackLegV1; MAX_BUYBACK_LEGS],
    pub maximum_payment: u64,
    pub minimum_refund: u64,
    pub deadline_ts: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct IndividualHedgeLeafWitnessV1 {
    pub amount: u64,
    pub leaf_index: u32,
    pub root_index: u16,
    pub prove_by_index: bool,
    pub tree_index: u8,
    pub queue_index: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct IndividualHedgeTransferV1 {
    pub series_index: u8,
    pub quantity: u64,
    pub input: IndividualHedgeLeafWitnessV1,
    pub input_has_delegate: bool,
    pub output_queue_index: u8,
    pub merkle_account_count: u8,
    pub maximum_topup: u64,
    pub proof: Option<[u8; 128]>,
}

/// Optional whole WriterCash leaf consumed when its compressed custody backs
/// a deferred owner credit. The same proof covers one input and its change.
#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct IndividualPortfolioCashWitnessV1 {
    pub amount: u64,
    pub leaf_index: u32,
    pub root_index: u16,
    pub prove_by_index: bool,
    pub proof: Option<[u8; 128]>,
}

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
    /// Filled once when the sleeve's payout becomes redeemable, never at oracle submission.
    pub settlement_finalized_ts: u64,
    /// Tokens permanently retired in compressed custody; no token account is allocated.
    pub compressed_retired_atoms: [u64; 20],
    /// Expired entitlement whose token may remain in its holder's wallet.
    pub forfeited_atoms: [u64; 20],
    pub pending_portfolio_funding: u64,
    pub funding_base_initialized: bool,
    pub hedges_consolidated: bool,
    pub funding_managed_numerator_le: [u8; 16],
    pub funding_prefix_numerator_le: [u8; 16],
    pub active_locked: [u64; 20],
    pub hedge_retired: [u64; 20],
    pub total_portfolio_credit: u64,
    pub remaining_portfolio_credit: u64,
    pub reserved: [u8; 2013],
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
            settlement_finalized_ts: 0,
            compressed_retired_atoms: [0; 20],
            forfeited_atoms: [0; 20],
            pending_portfolio_funding: 0,
            funding_base_initialized: false,
            hedges_consolidated: false,
            funding_managed_numerator_le: [0; 16],
            funding_prefix_numerator_le: [0; 16],
            active_locked: [0; 20],
            hedge_retired: [0; 20],
            total_portfolio_credit: 0,
            remaining_portfolio_credit: 0,
            reserved: [0; 2013],
        }
    }
}
fixed_state_deserialize!(IndividualTotals, 3072, {
    cash_obligations: u64, open_positions: u64, funded: bool, funded_long_liability: u64, funded_stranded: u64,
    series: [IndividualSeries; 20], settlement_finalized_ts: u64,
    compressed_retired_atoms: [u64; 20], forfeited_atoms: [u64; 20],
    pending_portfolio_funding: u64, funding_base_initialized: bool, hedges_consolidated: bool,
    funding_managed_numerator_le: [u8; 16], funding_prefix_numerator_le: [u8; 16],
    active_locked: [u64; 20], hedge_retired: [u64; 20],
    total_portfolio_credit: u64, remaining_portfolio_credit: u64, reserved: [u8; 2013],
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
    /// Informational initial deposit; portfolio.cash_atoms alone authorizes refunds.
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
    /// Buyer signs for a regular wallet-owned compressed leaf carrying the
    /// deterministic expiry delegate. Existing Fill stays undelegated.
    FillCompressed {
        quantity: u64,
        maximum_payment: u64,
    },
    LockHedge(IndividualHedgeTransferV1),
    UnlockHedge(IndividualHedgeTransferV1),
    RetireHedge(IndividualHedgeTransferV1),
    ClaimPortfolio(Option<IndividualPortfolioCashWitnessV1>),
    /// Buy exact matching options from funded asks using only this owner's
    /// portfolio cash. Delivery into portfolio custody and refund are atomic.
    BuybackFromAsks(IndividualBuybackV1),
}
