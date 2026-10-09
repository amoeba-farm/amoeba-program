//! Earn Fund accounts and instruction payloads (tag 12, `ManageEarnFundV1`).
//! Fund layout version 4 (no market targets, any number of open slots,
//! two-sided slot values and the entry bucket).
//!
//! The fund is one program-owned singleton plus a classic SPL USDC vault whose
//! authority is the fund PDA. Each investor holds one rent-free compressed
//! (Light) `EarnFundPositionV1` at a canonical address per (fund, owner); each
//! roll writes an `EarnFundEpochV1` used for lazy conversion of pending
//! deposits and for settling redemption batches completed at that roll. Each
//! writer sleeve the fund holds has one `EarnFundSlotV1` (its lots and their
//! latest valuation); the fund keeps only their O(1) aggregate (`SlotBook`).
use crate::buyback_mark_math::BUYBACK_PARAMS_LEN;
use crate::constants::CURRENT_STATE_NAMESPACE_SEED;
use crate::earn_fund_math::{
    CompletedBatch, EpochLedger, FundLedger, FundParams, PositionLedger, QueueBatch, RollOutcome,
    SharePrice, SlotBook, SlotMark, MAX_COMPLETED_BATCHES, MAX_OPEN_BATCHES,
};
use crate::fixed_codec::{
    fixed_state_deserialize_flat, invalid_fixed_borsh, FixedCursor, FixedField, FixedStateDecode,
    FixedStateEncode, FixedWriter,
};
use crate::instruction::CompressionOutput;
use borsh::{BorshDeserialize, BorshSerialize};
use light_sdk::{
    address::AddressSeed,
    instruction::{
        account_meta::CompressedAccountMeta, PackedAddressTreeInfo, PackedStateTreeInfo,
    },
    LightDiscriminator,
};
use solana_program::pubkey::Pubkey;

pub const EARN_FUND_SEED: &[u8] = b"earn-fund-v1";
pub const EARN_FUND_VAULT_SEED: &[u8] = b"earn-fund-usdc";
pub const EARN_FUND_POSITION_SEED: &[u8] = b"earn-fund-pos";
pub const EARN_FUND_EPOCH_SEED: &[u8] = b"earn-fund-epoch";
pub const EARN_FUND_SLOT_SEED: &[u8] = b"earn-fund-slot";
/// Layout version of the fund and its slots (v3: no targets, slot accounts;
/// v4: two-sided slot values, the upper aggregate and the entry bucket).
pub const EARN_FUND_VERSION: u8 = 4;
/// Layout version of investor positions and epoch records (unchanged, v2).
pub const EARN_FUND_RECORD_VERSION: u8 = 2;
pub const EARN_FUND_DISCRIMINATOR: [u8; 3] = *b"EFD";
pub const EARN_FUND_POSITION_DISCRIMINATOR: [u8; 3] = *b"EFP";
pub const EARN_FUND_EPOCH_DISCRIMINATOR: [u8; 3] = *b"EFE";
pub const EARN_FUND_SLOT_DISCRIMINATOR: [u8; 3] = *b"EFS";
/// Serialized `FundParams` length (71 fund bytes + 32 buy-back bytes).
pub const EARN_FUND_PARAMS_LEN: usize = 103;

pub fn derive_earn_fund(program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[CURRENT_STATE_NAMESPACE_SEED, EARN_FUND_SEED], program)
}

pub fn derive_earn_fund_vault(program: &Pubkey, fund: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            EARN_FUND_VAULT_SEED,
            fund.as_ref(),
        ],
        program,
    )
}

/// The canonical compressed address of `owner`'s position in `fund`, on the
/// default V2 address tree (Light `derive_address`).
pub fn derive_earn_fund_position_address(
    program: &Pubkey,
    fund: &Pubkey,
    owner: &Pubkey,
) -> ([u8; 32], AddressSeed) {
    crate::local_direct_address::derive_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            EARN_FUND_POSITION_SEED,
            fund.as_ref(),
            owner.as_ref(),
        ],
        &crate::constants::LIGHT_DEFAULT_ADDRESS_TREE_V2,
        program,
    )
}

pub fn derive_earn_fund_epoch(program: &Pubkey, epoch: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            EARN_FUND_EPOCH_SEED,
            &epoch.to_le_bytes(),
        ],
        program,
    )
}

/// The fund's slot of one writer sleeve.
pub fn derive_earn_fund_slot(program: &Pubkey, sleeve: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            EARN_FUND_SLOT_SEED,
            sleeve.as_ref(),
        ],
        program,
    )
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EarnFundV1 {
    pub initialized: bool,
    pub bump: u8,
    pub discriminator: [u8; 3],
    pub version: u8,
    pub vault_config: Pubkey,
    pub usdc_mint: Pubkey,
    pub usdc_vault: Pubkey,
    pub usdc_vault_bump: u8,
    // Admin parameters (`FundParams`, contiguous).
    pub buffer_bps: u16,
    pub instant_daily_cap_bps: u16,
    pub max_third_party_bps: u16,
    pub min_tenor_secs: u64,
    pub max_tenor_secs: u64,
    pub min_epoch_secs: u64,
    pub min_allocation_atoms: u64,
    pub allocator: Pubkey,
    pub paused: bool,
    /// Buy-back pricing settings in their 32-byte form
    /// (`BuybackParams::decode`; nonzero padding makes the fund invalid).
    pub buyback_params: [u8; BUYBACK_PARAMS_LEN],
    // Share ledger and last roll price.
    pub total_shares: u64,
    pub epoch: u64,
    pub epoch_started_ts: u64,
    pub price_assets: u64,
    pub price_shares: u64,
    pub cap_base_atoms: u64,
    // Cash buckets (all inside `usdc_vault`).
    pub free_cash_atoms: u64,
    pub pending_deposit_atoms: u64,
    pub pending_carry_from: u64,
    pub reserved_withdrawal_atoms: u64,
    // Redemption queue.
    pub queued_shares: u64,
    pub accumulating_batch_id: u64,
    pub accumulating_shares: u64,
    pub open_batch_count: u8,
    pub open_batch_id: [u64; MAX_OPEN_BATCHES],
    pub open_batch_shares: [u64; MAX_OPEN_BATCHES],
    pub open_batch_filled_shares: [u64; MAX_OPEN_BATCHES],
    pub open_batch_filled_atoms: [u64; MAX_OPEN_BATCHES],
    pub open_batch_paid_atoms: [u64; MAX_OPEN_BATCHES],
    // Instant-withdrawal and marked-exit token buckets.
    pub window_updated_ts: u64,
    pub window_level_atoms: u64,
    pub marked_window_updated_ts: u64,
    pub marked_window_level_atoms: u64,
    /// The lot nonce, the deployed principal and the O(1) aggregate of every
    /// open slot.
    pub book: SlotBook,
    /// The entry bucket `[updated_ts, level_atoms]`: immediate conversions
    /// of deposits while invested, drained linearly by `entry_cap_bps ×
    /// cap_base` per 24h.
    pub entry_window: [u64; 2],
}

impl EarnFundV1 {
    pub const LEN: usize = 615;

    pub fn has_current_layout(&self) -> bool {
        self.initialized
            && self.discriminator == EARN_FUND_DISCRIMINATOR
            && self.version == EARN_FUND_VERSION
    }

    pub fn params(&self) -> FundParams {
        FundParams {
            buffer_bps: self.buffer_bps,
            instant_daily_cap_bps: self.instant_daily_cap_bps,
            max_third_party_bps: self.max_third_party_bps,
            min_tenor_secs: self.min_tenor_secs,
            max_tenor_secs: self.max_tenor_secs,
            min_epoch_secs: self.min_epoch_secs,
            min_allocation_atoms: self.min_allocation_atoms,
            allocator: self.allocator.to_bytes(),
            paused: self.paused,
            buyback: self.buyback_params,
        }
    }

    pub fn set_params(&mut self, params: &FundParams) {
        self.buffer_bps = params.buffer_bps;
        self.instant_daily_cap_bps = params.instant_daily_cap_bps;
        self.max_third_party_bps = params.max_third_party_bps;
        self.min_tenor_secs = params.min_tenor_secs;
        self.max_tenor_secs = params.max_tenor_secs;
        self.min_epoch_secs = params.min_epoch_secs;
        self.min_allocation_atoms = params.min_allocation_atoms;
        self.allocator = Pubkey::new_from_array(params.allocator);
        self.buyback_params = params.buyback;
        self.paused = params.paused;
    }

    fn open_batches(&self) -> [QueueBatch; MAX_OPEN_BATCHES] {
        core::array::from_fn(|index| QueueBatch {
            id: self.open_batch_id[index],
            shares: self.open_batch_shares[index],
            filled_shares: self.open_batch_filled_shares[index],
            filled_atoms: self.open_batch_filled_atoms[index],
            paid_atoms: self.open_batch_paid_atoms[index],
        })
    }

    /// The fund-wide ledger view; `None` if the stored state is inconsistent.
    pub fn ledger(&self) -> Option<FundLedger> {
        let ledger = FundLedger {
            total_shares: self.total_shares,
            epoch: self.epoch,
            epoch_started_ts: self.epoch_started_ts,
            price: SharePrice::new(self.price_assets, self.price_shares),
            cap_base_atoms: self.cap_base_atoms,
            free_cash_atoms: self.free_cash_atoms,
            pending_deposit_atoms: self.pending_deposit_atoms,
            pending_carry_from: self.pending_carry_from,
            reserved_withdrawal_atoms: self.reserved_withdrawal_atoms,
            queued_shares: self.queued_shares,
            accumulating_batch_id: self.accumulating_batch_id,
            accumulating_shares: self.accumulating_shares,
            open_batch_count: self.open_batch_count,
            open_batches: self.open_batches(),
            deployed_atoms: self.book.deployed_atoms,
            window_updated_ts: self.window_updated_ts,
            window_level_atoms: self.window_level_atoms,
        };
        // Open slots and deployed principal come and go together.
        (ledger.is_consistent()
            && self.book.is_consistent()
            && (self.book.slots == 0) == (self.book.deployed_atoms == 0))
            .then_some(ledger)
    }

    /// Store a ledger (including the deployed principal).
    pub fn set_ledger(&mut self, ledger: &FundLedger) {
        self.total_shares = ledger.total_shares;
        self.epoch = ledger.epoch;
        self.epoch_started_ts = ledger.epoch_started_ts;
        self.price_assets = ledger.price.assets;
        self.price_shares = ledger.price.shares;
        self.cap_base_atoms = ledger.cap_base_atoms;
        self.free_cash_atoms = ledger.free_cash_atoms;
        self.pending_deposit_atoms = ledger.pending_deposit_atoms;
        self.pending_carry_from = ledger.pending_carry_from;
        self.reserved_withdrawal_atoms = ledger.reserved_withdrawal_atoms;
        self.queued_shares = ledger.queued_shares;
        self.accumulating_batch_id = ledger.accumulating_batch_id;
        self.accumulating_shares = ledger.accumulating_shares;
        self.open_batch_count = ledger.open_batch_count;
        for (index, batch) in ledger.open_batches.iter().enumerate() {
            self.open_batch_id[index] = batch.id;
            self.open_batch_shares[index] = batch.shares;
            self.open_batch_filled_shares[index] = batch.filled_shares;
            self.open_batch_filled_atoms[index] = batch.filled_atoms;
            self.open_batch_paid_atoms[index] = batch.paid_atoms;
        }
        self.book.deployed_atoms = ledger.deployed_atoms;
        self.window_updated_ts = ledger.window_updated_ts;
        self.window_level_atoms = ledger.window_level_atoms;
    }
}

/// The aggregate is stored as twelve u64 words: slots, live, unpriced,
/// counted, sum_lower, sum_upper, round, round_start_ts, round_min_ts,
/// fresh_since_ts, next_lot_id, deployed_atoms (96 bytes, read as three
/// `[u64; 4]` like the batch arrays).
impl FixedField for SlotBook {
    #[inline(always)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        let [slots, live, unpriced, counted] = <[u64; 4] as FixedField>::read(input);
        let [sum_lower, sum_upper, round, round_start_ts] = <[u64; 4] as FixedField>::read(input);
        let [round_min_ts, fresh_since_ts, next_lot_id, deployed_atoms] =
            <[u64; 4] as FixedField>::read(input);
        Self {
            slots,
            live,
            unpriced,
            counted,
            sum_lower,
            sum_upper,
            round,
            round_start_ts,
            round_min_ts,
            fresh_since_ts,
            next_lot_id,
            deployed_atoms,
        }
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        let [w0, w1, w2, w3, w4, w5, w6, w7, w8, w9, w10, w11] = self.words();
        for chunk in [[w0, w1, w2, w3], [w4, w5, w6, w7], [w8, w9, w10, w11]] {
            <[u64; 4] as FixedField>::write(&chunk, output);
        }
    }
}

impl SlotBook {
    fn words(&self) -> [u64; 12] {
        [
            self.slots,
            self.live,
            self.unpriced,
            self.counted,
            self.sum_lower,
            self.sum_upper,
            self.round,
            self.round_start_ts,
            self.round_min_ts,
            self.fresh_since_ts,
            self.next_lot_id,
            self.deployed_atoms,
        ]
    }
}

fixed_state_deserialize_flat!(EarnFundV1, EarnFundV1::LEN, {
    initialized: bool, bump: u8, discriminator: [u8; 3], version: u8,
    vault_config: Pubkey, usdc_mint: Pubkey, usdc_vault: Pubkey, usdc_vault_bump: u8,
    buffer_bps: u16, instant_daily_cap_bps: u16, max_third_party_bps: u16,
    min_tenor_secs: u64, max_tenor_secs: u64, min_epoch_secs: u64, min_allocation_atoms: u64,
    allocator: Pubkey, paused: bool, buyback_params: [u8; BUYBACK_PARAMS_LEN],
    total_shares: u64, epoch: u64, epoch_started_ts: u64, price_assets: u64, price_shares: u64,
    cap_base_atoms: u64, free_cash_atoms: u64, pending_deposit_atoms: u64, pending_carry_from: u64,
    reserved_withdrawal_atoms: u64, queued_shares: u64, accumulating_batch_id: u64,
    accumulating_shares: u64, open_batch_count: u8,
    open_batch_id: [u64; MAX_OPEN_BATCHES], open_batch_shares: [u64; MAX_OPEN_BATCHES],
    open_batch_filled_shares: [u64; MAX_OPEN_BATCHES], open_batch_filled_atoms: [u64; MAX_OPEN_BATCHES],
    open_batch_paid_atoms: [u64; MAX_OPEN_BATCHES],
    window_updated_ts: u64, window_level_atoms: u64,
    marked_window_updated_ts: u64, marked_window_level_atoms: u64,
    book: SlotBook, entry_window: [u64; 2],
}, flat {
    initialized: bool, bump: u8, discriminator: [u8; 3], version: u8,
    vault_config: Pubkey, usdc_mint: Pubkey, usdc_vault: Pubkey, usdc_vault_bump: u8,
    buffer_bps: u16, instant_daily_cap_bps: u16, max_third_party_bps: u16,
    min_tenor_secs: u64, max_tenor_secs: u64, min_epoch_secs: u64, min_allocation_atoms: u64,
    allocator: Pubkey, paused: bool, buyback_params: [u8; BUYBACK_PARAMS_LEN],
    total_shares: u64, epoch: u64, epoch_started_ts: u64, price_assets: u64, price_shares: u64,
    cap_base_atoms: u64, free_cash_atoms: u64, pending_deposit_atoms: u64, pending_carry_from: u64,
    reserved_withdrawal_atoms: u64, queued_shares: u64, accumulating_batch_id: u64,
    accumulating_shares: u64, open_batch_count: u8,
    open_batch_id: [u64; MAX_OPEN_BATCHES], open_batch_shares: [u64; MAX_OPEN_BATCHES],
    open_batch_filled_shares: [u64; MAX_OPEN_BATCHES], open_batch_filled_atoms: [u64; MAX_OPEN_BATCHES],
    open_batch_paid_atoms: [u64; MAX_OPEN_BATCHES],
    window_updated_ts: u64, window_level_atoms: u64,
    marked_window_updated_ts: u64, marked_window_level_atoms: u64,
    book.slots: u64, book.live: u64, book.unpriced: u64, book.counted: u64,
    book.sum_lower: u64, book.sum_upper: u64, book.round: u64, book.round_start_ts: u64,
    book.round_min_ts: u64, book.fresh_since_ts: u64, book.next_lot_id: u64,
    book.deployed_atoms: u64, entry_window: [u64; 2],
});

/// The fund's holding in one writer sleeve: its open lots (contiguous in the
/// sleeve's capital-seconds ledger, so their payouts telescope) and their
/// latest valuation. It exists while `principal` is nonzero: created by the
/// first Allocate into the sleeve (the Allocate payer pays the rent) and
/// closed by the Collect of its last lot (the rent returns to that receipt's
/// rent payer).
///
/// The account bytes are exactly this `repr(C)` value (little-endian, no
/// padding, 136 bytes), copied in and out whole: `initialized 0, bump 1,
/// discriminator "EFS" 2, version 5, zero 6..8, sleeve 8, principal 40,
/// weight_start u128 48, weight_end u128 64`, then the valuation: `state 80,
/// round 88, value_lower 96, value_upper 104, valued_ts 112`, then
/// `sample_slot 120, book_slot 128`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EarnFundSlotV1 {
    /// `[1, bump, b'E', b'F', b'S', version, 0, 0]`.
    pub header: [u8; 8],
    pub sleeve: Pubkey,
    /// The open lots' principal.
    pub principal: u64,
    /// Their capital-seconds range `[start, end)`, two little-endian u128.
    pub weights: [u64; 4],
    pub mark: SlotMark,
    /// The sample slot of the valuation (the mark head's slot; the crank's
    /// slot for a Funding or settled sleeve) and the series book's
    /// `last_updated_slot` it priced.
    pub sample_slot: u64,
    pub book_slot: u64,
}

const _: () = assert!(core::mem::size_of::<EarnFundSlotV1>() == EarnFundSlotV1::LEN);

impl EarnFundSlotV1 {
    pub const LEN: usize = 136;

    pub fn new(bump: u8, sleeve: Pubkey) -> Self {
        let [d0, d1, d2] = EARN_FUND_SLOT_DISCRIMINATOR;
        Self {
            header: [1, bump, d0, d1, d2, EARN_FUND_VERSION, 0, 0],
            sleeve,
            ..Self::default()
        }
    }

    pub fn bump(&self) -> u8 {
        self.header[1]
    }

    pub fn start(&self) -> u128 {
        u128::from(self.weights[0]) | (u128::from(self.weights[1]) << 64)
    }

    pub fn end(&self) -> u128 {
        u128::from(self.weights[2]) | (u128::from(self.weights[3]) << 64)
    }

    pub fn set_range(&mut self, start: u128, end: u128) {
        self.weights = [
            start as u64,
            (start >> 64) as u64,
            end as u64,
            (end >> 64) as u64,
        ];
    }

    pub fn range(&self) -> crate::buyback_mark_math::FundRange {
        crate::buyback_mark_math::FundRange {
            principal: self.principal,
            start: self.start(),
            end: self.end(),
        }
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: `repr(C)` with no padding (asserted size) and only integer
        // fields: every byte pattern is a valid value.
        unsafe { core::slice::from_raw_parts_mut((self as *mut Self).cast::<u8>(), Self::LEN) }
    }

    /// The slot in `data`: exact length, initialized, discriminator, version
    /// and zero padding (identity is the loader's job). One syscall copy.
    pub fn read(data: &[u8]) -> Option<Self> {
        let mut value = Self::default();
        if data.len() != Self::LEN {
            return None;
        }
        solana_program::program_memory::sol_memcpy(value.bytes_mut(), data, Self::LEN);
        let [d0, d1, d2] = EARN_FUND_SLOT_DISCRIMINATOR;
        (value.header == [1, value.bump(), d0, d1, d2, EARN_FUND_VERSION, 0, 0]).then_some(value)
    }

    /// Write the whole account (`LEN` bytes). One syscall copy.
    pub fn write(&self, data: &mut [u8]) {
        let mut value = *self;
        solana_program::program_memory::sol_memcpy(data, value.bytes_mut(), Self::LEN);
    }
}

/// An investor position: a compressed account owned by this program at
/// `derive_earn_fund_position_address(fund, owner)`. No rent is paid; the
/// account is closed (Light's empty placeholder) once shares, pending,
/// queued shares and claimable atoms are all zero, and reopened by the next
/// deposit.
#[derive(
    BorshSerialize, BorshDeserialize, Clone, Debug, Default, Eq, PartialEq, LightDiscriminator,
)]
pub struct EarnFundPositionV1 {
    pub schema_version: u8,
    pub fund: Pubkey,
    pub owner: Pubkey,
    pub shares: u64,
    pub pending_atoms: u64,
    pub pending_epoch: u64,
    pub queue_shares: u64,
    pub queue_batch_id: u64,
    pub queue_paid_atoms: u64,
    pub claimable_atoms: u64,
    /// Cumulative atoms credited to pending by Deposit.
    pub deposited_atoms: u64,
    /// Cumulative atoms paid to the owner: par refunds, instant payouts and
    /// CompleteWithdrawal payouts.
    pub withdrawn_atoms: u64,
}

impl EarnFundPositionV1 {
    pub const LEN: usize = 137;

    pub fn new(fund: Pubkey, owner: Pubkey, ledger: &PositionLedger) -> Self {
        let mut position = Self {
            schema_version: EARN_FUND_RECORD_VERSION,
            fund,
            owner,
            ..Self::default()
        };
        position.set_ledger(ledger);
        position
    }

    pub fn has_current_layout(&self) -> bool {
        self.schema_version == EARN_FUND_RECORD_VERSION
    }

    /// Nothing left to own, convert, redeem or pay.
    pub fn is_empty(&self) -> bool {
        self.shares == 0
            && self.pending_atoms == 0
            && self.queue_shares == 0
            && self.claimable_atoms == 0
    }

    pub fn ledger(&self) -> PositionLedger {
        PositionLedger {
            shares: self.shares,
            pending_atoms: self.pending_atoms,
            pending_epoch: self.pending_epoch,
            queue_shares: self.queue_shares,
            queue_batch_id: self.queue_batch_id,
            queue_paid_atoms: self.queue_paid_atoms,
            claimable_atoms: self.claimable_atoms,
            deposited_atoms: self.deposited_atoms,
            withdrawn_atoms: self.withdrawn_atoms,
        }
    }

    pub fn set_ledger(&mut self, ledger: &PositionLedger) {
        self.shares = ledger.shares;
        self.pending_atoms = ledger.pending_atoms;
        self.pending_epoch = ledger.pending_epoch;
        self.queue_shares = ledger.queue_shares;
        self.queue_batch_id = ledger.queue_batch_id;
        self.queue_paid_atoms = ledger.queue_paid_atoms;
        self.claimable_atoms = ledger.claimable_atoms;
        self.deposited_atoms = ledger.deposited_atoms;
        self.withdrawn_atoms = ledger.withdrawn_atoms;
    }
}

/// Where a position's compressed account is, as supplied by the client.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EarnFundPositionWitness {
    /// 0: the canonical address was never created (first deposit).
    New {
        address_tree_info: PackedAddressTreeInfo,
        output_state_tree_index: u8,
    },
    /// 1: a live position; `state` is its current ledger (owner and fund are
    /// bound by the address and the account hash).
    Live {
        tree_info: PackedStateTreeInfo,
        output_state_tree_index: u8,
        state: PositionLedger,
    },
    /// 2: the address holds a closed placeholder (a fully exited position).
    Closed {
        tree_info: PackedStateTreeInfo,
        output_state_tree_index: u8,
    },
}

/// Serialized witness lengths: New 6, Live 83, Closed 11.
pub const EARN_FUND_POSITION_WITNESS_LIVE_LEN: usize = 83;

impl EarnFundPositionWitness {
    /// The creation output of a `New` witness.
    pub fn output(&self) -> Option<CompressionOutput> {
        match *self {
            Self::New {
                address_tree_info,
                output_state_tree_index,
            } => Some(CompressionOutput {
                address_tree_info,
                output_state_tree_index,
            }),
            _ => None,
        }
    }

    /// The Light input meta of an existing account at `address`.
    pub fn meta(&self, address: [u8; 32]) -> Option<CompressedAccountMeta> {
        match *self {
            Self::Live {
                tree_info,
                output_state_tree_index,
                ..
            }
            | Self::Closed {
                tree_info,
                output_state_tree_index,
            } => Some(CompressedAccountMeta {
                tree_info,
                address,
                output_state_tree_index,
            }),
            Self::New { .. } => None,
        }
    }
}

fn read_tree_info<R: std::io::Read>(reader: &mut R) -> std::io::Result<PackedStateTreeInfo> {
    Ok(PackedStateTreeInfo {
        root_index: u16::deserialize_reader(reader)?,
        prove_by_index: bool::deserialize_reader(reader)?,
        merkle_tree_pubkey_index: u8::deserialize_reader(reader)?,
        queue_pubkey_index: u8::deserialize_reader(reader)?,
        leaf_index: u32::deserialize_reader(reader)?,
    })
}

fn write_tree_info<W: std::io::Write>(
    info: &PackedStateTreeInfo,
    writer: &mut W,
) -> std::io::Result<()> {
    info.root_index.serialize(writer)?;
    info.prove_by_index.serialize(writer)?;
    info.merkle_tree_pubkey_index.serialize(writer)?;
    info.queue_pubkey_index.serialize(writer)?;
    info.leaf_index.serialize(writer)
}

fn read_ledger<R: std::io::Read>(reader: &mut R) -> std::io::Result<PositionLedger> {
    let mut values = [0u64; 9];
    for value in &mut values {
        *value = u64::deserialize_reader(reader)?;
    }
    Ok(PositionLedger {
        shares: values[0],
        pending_atoms: values[1],
        pending_epoch: values[2],
        queue_shares: values[3],
        queue_batch_id: values[4],
        queue_paid_atoms: values[5],
        claimable_atoms: values[6],
        deposited_atoms: values[7],
        withdrawn_atoms: values[8],
    })
}

fn write_ledger<W: std::io::Write>(ledger: &PositionLedger, writer: &mut W) -> std::io::Result<()> {
    for value in [
        ledger.shares,
        ledger.pending_atoms,
        ledger.pending_epoch,
        ledger.queue_shares,
        ledger.queue_batch_id,
        ledger.queue_paid_atoms,
        ledger.claimable_atoms,
        ledger.deposited_atoms,
        ledger.withdrawn_atoms,
    ] {
        value.serialize(writer)?;
    }
    Ok(())
}

impl BorshDeserialize for EarnFundPositionWitness {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(match u8::deserialize_reader(reader)? {
            0 => Self::New {
                address_tree_info: PackedAddressTreeInfo {
                    address_merkle_tree_pubkey_index: u8::deserialize_reader(reader)?,
                    address_queue_pubkey_index: u8::deserialize_reader(reader)?,
                    root_index: u16::deserialize_reader(reader)?,
                },
                output_state_tree_index: u8::deserialize_reader(reader)?,
            },
            1 => Self::Live {
                tree_info: read_tree_info(reader)?,
                output_state_tree_index: u8::deserialize_reader(reader)?,
                state: read_ledger(reader)?,
            },
            2 => Self::Closed {
                tree_info: read_tree_info(reader)?,
                output_state_tree_index: u8::deserialize_reader(reader)?,
            },
            _ => return Err(invalid_fixed_borsh()),
        })
    }
}

impl BorshSerialize for EarnFundPositionWitness {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        match self {
            Self::New {
                address_tree_info,
                output_state_tree_index,
            } => {
                0u8.serialize(writer)?;
                address_tree_info
                    .address_merkle_tree_pubkey_index
                    .serialize(writer)?;
                address_tree_info
                    .address_queue_pubkey_index
                    .serialize(writer)?;
                address_tree_info.root_index.serialize(writer)?;
                output_state_tree_index.serialize(writer)
            }
            Self::Live {
                tree_info,
                output_state_tree_index,
                state,
            } => {
                1u8.serialize(writer)?;
                write_tree_info(tree_info, writer)?;
                output_state_tree_index.serialize(writer)?;
                write_ledger(state, writer)
            }
            Self::Closed {
                tree_info,
                output_state_tree_index,
            } => {
                2u8.serialize(writer)?;
                write_tree_info(tree_info, writer)?;
                output_state_tree_index.serialize(writer)
            }
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EarnFundEpochV1 {
    pub initialized: bool,
    pub bump: u8,
    pub discriminator: [u8; 3],
    pub version: u8,
    pub fund: Pubkey,
    pub epoch: u64,
    pub roll_ts: u64,
    pub price_assets: u64,
    pub price_shares: u64,
    pub pending_converted: bool,
    pub carry_from: u64,
    pub converted_atoms: u64,
    pub converted_shares: u64,
    pub converted_remaining_atoms: u64,
    pub converted_remaining_shares: u64,
    pub queue_before_shares: u64,
    pub fill_shares: u64,
    pub fill_atoms: u64,
    pub completed_count: u8,
    pub completed_id: [u64; MAX_COMPLETED_BATCHES],
    pub completed_shares: [u64; MAX_COMPLETED_BATCHES],
    pub completed_filled_atoms: [u64; MAX_COMPLETED_BATCHES],
    pub completed_paid_atoms: [u64; MAX_COMPLETED_BATCHES],
    pub completed_settled_shares: [u64; MAX_COMPLETED_BATCHES],
    pub post_roll_assets: u64,
    pub post_roll_shares: u64,
}

impl EarnFundEpochV1 {
    pub const LEN: usize = 352;

    pub fn has_current_layout(&self) -> bool {
        self.initialized
            && self.discriminator == EARN_FUND_EPOCH_DISCRIMINATOR
            && self.version == EARN_FUND_RECORD_VERSION
    }

    pub fn from_roll(fund: Pubkey, bump: u8, outcome: &RollOutcome, roll_ts: u64) -> Self {
        let mut record = Self {
            initialized: true,
            bump,
            discriminator: EARN_FUND_EPOCH_DISCRIMINATOR,
            version: EARN_FUND_RECORD_VERSION,
            fund,
            epoch: outcome.closed_epoch,
            roll_ts,
            price_assets: outcome.price.assets,
            price_shares: outcome.price.shares,
            pending_converted: outcome.pending_converted,
            carry_from: outcome.carry_from,
            converted_atoms: outcome.converted_atoms,
            converted_shares: outcome.converted_shares,
            queue_before_shares: outcome.queue_before_shares,
            fill_shares: outcome.fill_shares,
            fill_atoms: outcome.fill_atoms,
            post_roll_assets: outcome.post_assets,
            post_roll_shares: outcome.post_shares,
            ..Self::default()
        };
        record.set_ledger(&EpochLedger::from_roll(outcome));
        record
    }

    pub fn ledger(&self) -> EpochLedger {
        EpochLedger {
            epoch: self.epoch,
            price: SharePrice::new(self.price_assets, self.price_shares),
            pending_converted: self.pending_converted,
            carry_from: self.carry_from,
            converted_remaining_atoms: self.converted_remaining_atoms,
            converted_remaining_shares: self.converted_remaining_shares,
            completed_count: self.completed_count,
            completed: core::array::from_fn(|index| CompletedBatch {
                id: self.completed_id[index],
                shares: self.completed_shares[index],
                filled_atoms: self.completed_filled_atoms[index],
                paid_atoms: self.completed_paid_atoms[index],
                settled_shares: self.completed_settled_shares[index],
            }),
        }
    }

    /// Store the mutable remainders. Identity, price and totals are immutable.
    pub fn set_ledger(&mut self, ledger: &EpochLedger) {
        self.converted_remaining_atoms = ledger.converted_remaining_atoms;
        self.converted_remaining_shares = ledger.converted_remaining_shares;
        self.completed_count = ledger.completed_count;
        for (index, batch) in ledger.completed.iter().enumerate() {
            self.completed_id[index] = batch.id;
            self.completed_shares[index] = batch.shares;
            self.completed_filled_atoms[index] = batch.filled_atoms;
            self.completed_paid_atoms[index] = batch.paid_atoms;
            self.completed_settled_shares[index] = batch.settled_shares;
        }
    }
}

fixed_state_deserialize_flat!(EarnFundEpochV1, EarnFundEpochV1::LEN, {
    initialized: bool, bump: u8, discriminator: [u8; 3], version: u8,
    fund: Pubkey, epoch: u64, roll_ts: u64, price_assets: u64, price_shares: u64,
    pending_converted: bool, carry_from: u64, converted_atoms: u64, converted_shares: u64,
    converted_remaining_atoms: u64, converted_remaining_shares: u64,
    queue_before_shares: u64, fill_shares: u64, fill_atoms: u64, completed_count: u8,
    completed_id: [u64; MAX_COMPLETED_BATCHES], completed_shares: [u64; MAX_COMPLETED_BATCHES],
    completed_filled_atoms: [u64; MAX_COMPLETED_BATCHES],
    completed_paid_atoms: [u64; MAX_COMPLETED_BATCHES],
    completed_settled_shares: [u64; MAX_COMPLETED_BATCHES],
    post_roll_assets: u64, post_roll_shares: u64,
});

/// How a Withdraw redeems its shares, and (`EarnFundDepositMode`, the same
/// bytes) when a Deposit converts the position's pending money to shares.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EarnFundWithdrawMode {
    /// Withdraw: pay instantly at the marked NAV when available, otherwise
    /// queue. Deposit: convert now at the upper NAV when available,
    /// otherwise stay pending for the next roll.
    #[default]
    InstantElseQueue = 0,
    /// Withdraw: queue for the next roll without quoting. Deposit: pending
    /// only (no vault config account).
    QueueOnly = 1,
    /// Withdraw: pay instantly or fail. Deposit: convert now or fail.
    InstantOnly = 2,
}

/// Deposit modes: 0 convert now else stay pending, 1 pending only, 2
/// convert now or fail.
pub type EarnFundDepositMode = EarnFundWithdrawMode;

impl EarnFundWithdrawMode {
    fn from_byte(byte: u8) -> std::io::Result<Self> {
        Ok(match byte {
            0 => Self::InstantElseQueue,
            1 => Self::QueueOnly,
            2 => Self::InstantOnly,
            _ => return Err(invalid_fixed_borsh()),
        })
    }
}

/// `ManageEarnFundV1` actions. Selector zero is never assigned.
#[derive(Clone, Debug, PartialEq)]
pub enum EarnFundActionV1 {
    /// 1: config admin creates the fund singleton and its classic USDC vault.
    InitializeFund { params: FundParams },
    /// 2: config admin replaces bounded parameters; buckets are untouched.
    ConfigureFund { params: FundParams },
    /// 3: owner moves USDC (a compressed leaf or a classic account) into the
    /// pending bucket and, per `mode`, converts the position's whole pending
    /// deposit to at least `min_shares` shares now at the upper NAV (`amount
    /// = 0` only converts). Fees and gas sponsorship live outside the program
    /// (the transaction envelope); no fee field exists.
    Deposit {
        amount_atoms: u64,
        mode: EarnFundDepositMode,
        min_shares: u64,
        source: EarnFundDepositSource,
        position: EarnFundPositionWitness,
        proof: Option<[u8; 128]>,
    },
    /// 4: refund pending at par, then redeem shares per `mode`.
    Withdraw {
        pending_atoms: u64,
        shares: u64,
        min_instant_atoms: u64,
        mode: EarnFundWithdrawMode,
        position: EarnFundPositionWitness,
        proof: Option<[u8; 128]>,
    },
    /// 5: anyone settles an owner's position and pays its claimable atoms to
    /// the owner as compressed USDC.
    CompleteWithdrawal {
        position: EarnFundPositionWitness,
        proof: Option<[u8; 128]>,
    },
    /// 6: the allocator deploys free cash into any writer sleeve (the live
    /// minimum cash buffer and the entry rules apply).
    Allocate { amount_atoms: u64 },
    /// 7: anyone collects a settled fund receipt (the oldest open lot of its
    /// slot) into the fund vault.
    Collect {
        cash_amount: u64,
        cash_leaf_index: u32,
        cash_root_index: u16,
        cash_prove_by_index: bool,
        proof: Option<[u8; 128]>,
    },
    /// 8: anyone closes an epoch: exactly with nothing open, else (every
    /// live slot priced) pending converts at the upper NAV and queued shares
    /// fill at the lower NAV.
    Roll,
    /// 9: the permissionless buy-back mark crank of one sleeve:
    /// `op` 0 Create, 1 Sample, 2 Close, 3 ValueSlot, 4 BeginCreate,
    /// 5 BeginSample, 6 SampleChunk, 7 Finalize, 8 CloseRound.
    ManageBuybackMark { op: u8 },
}

/// Where a deposit's USDC comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EarnFundDepositSource {
    /// 0: the owner's classic USDC token account.
    Classic,
    /// 1: one whole compressed USDC leaf owned by the owner; the deposit is
    /// decompressed into the fund vault and the change returns compressed.
    Compressed {
        amount: u64,
        leaf_index: u32,
        root_index: u16,
        prove_by_index: bool,
        tree_index: u8,
        queue_index: u8,
        proof: Option<[u8; 128]>,
    },
}

impl BorshDeserialize for EarnFundDepositSource {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(match u8::deserialize_reader(reader)? {
            0 => Self::Classic,
            1 => Self::Compressed {
                amount: u64::deserialize_reader(reader)?,
                leaf_index: u32::deserialize_reader(reader)?,
                root_index: u16::deserialize_reader(reader)?,
                prove_by_index: bool::deserialize_reader(reader)?,
                tree_index: u8::deserialize_reader(reader)?,
                queue_index: u8::deserialize_reader(reader)?,
                proof: Option::<[u8; 128]>::deserialize_reader(reader)?,
            },
            _ => return Err(invalid_fixed_borsh()),
        })
    }
}

impl BorshSerialize for EarnFundDepositSource {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        match self {
            Self::Classic => 0u8.serialize(writer),
            Self::Compressed {
                amount,
                leaf_index,
                root_index,
                prove_by_index,
                tree_index,
                queue_index,
                proof,
            } => {
                1u8.serialize(writer)?;
                amount.serialize(writer)?;
                leaf_index.serialize(writer)?;
                root_index.serialize(writer)?;
                prove_by_index.serialize(writer)?;
                tree_index.serialize(writer)?;
                queue_index.serialize(writer)?;
                proof.serialize(writer)
            }
        }
    }
}

fn read_params<R: std::io::Read>(reader: &mut R) -> std::io::Result<FundParams> {
    let buffer_bps = u16::deserialize_reader(reader)?;
    let instant_daily_cap_bps = u16::deserialize_reader(reader)?;
    let max_third_party_bps = u16::deserialize_reader(reader)?;
    let min_tenor_secs = u64::deserialize_reader(reader)?;
    let max_tenor_secs = u64::deserialize_reader(reader)?;
    let min_epoch_secs = u64::deserialize_reader(reader)?;
    let min_allocation_atoms = u64::deserialize_reader(reader)?;
    let allocator = <[u8; 32]>::deserialize_reader(reader)?;
    let paused = bool::deserialize_reader(reader)?;
    let buyback = <[u8; BUYBACK_PARAMS_LEN]>::deserialize_reader(reader)?;
    Ok(FundParams {
        buffer_bps,
        instant_daily_cap_bps,
        max_third_party_bps,
        min_tenor_secs,
        max_tenor_secs,
        min_epoch_secs,
        min_allocation_atoms,
        allocator,
        paused,
        buyback,
    })
}

fn write_params<W: std::io::Write>(params: &FundParams, writer: &mut W) -> std::io::Result<()> {
    params.buffer_bps.serialize(writer)?;
    params.instant_daily_cap_bps.serialize(writer)?;
    params.max_third_party_bps.serialize(writer)?;
    params.min_tenor_secs.serialize(writer)?;
    params.max_tenor_secs.serialize(writer)?;
    params.min_epoch_secs.serialize(writer)?;
    params.min_allocation_atoms.serialize(writer)?;
    params.allocator.serialize(writer)?;
    params.paused.serialize(writer)?;
    params.buyback.serialize(writer)
}

impl BorshDeserialize for EarnFundActionV1 {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(match u8::deserialize_reader(reader)? {
            1 => Self::InitializeFund {
                params: read_params(reader)?,
            },
            2 => Self::ConfigureFund {
                params: read_params(reader)?,
            },
            3 => Self::Deposit {
                amount_atoms: u64::deserialize_reader(reader)?,
                mode: EarnFundWithdrawMode::from_byte(u8::deserialize_reader(reader)?)?,
                min_shares: u64::deserialize_reader(reader)?,
                source: EarnFundDepositSource::deserialize_reader(reader)?,
                position: EarnFundPositionWitness::deserialize_reader(reader)?,
                proof: Option::<[u8; 128]>::deserialize_reader(reader)?,
            },
            4 => Self::Withdraw {
                pending_atoms: u64::deserialize_reader(reader)?,
                shares: u64::deserialize_reader(reader)?,
                min_instant_atoms: u64::deserialize_reader(reader)?,
                mode: EarnFundWithdrawMode::from_byte(u8::deserialize_reader(reader)?)?,
                position: EarnFundPositionWitness::deserialize_reader(reader)?,
                proof: Option::<[u8; 128]>::deserialize_reader(reader)?,
            },
            5 => Self::CompleteWithdrawal {
                position: EarnFundPositionWitness::deserialize_reader(reader)?,
                proof: Option::<[u8; 128]>::deserialize_reader(reader)?,
            },
            6 => Self::Allocate {
                amount_atoms: u64::deserialize_reader(reader)?,
            },
            7 => Self::Collect {
                cash_amount: u64::deserialize_reader(reader)?,
                cash_leaf_index: u32::deserialize_reader(reader)?,
                cash_root_index: u16::deserialize_reader(reader)?,
                cash_prove_by_index: bool::deserialize_reader(reader)?,
                proof: Option::<[u8; 128]>::deserialize_reader(reader)?,
            },
            8 => Self::Roll,
            9 => Self::ManageBuybackMark {
                op: match u8::deserialize_reader(reader)? {
                    op @ 0..=8 => op,
                    _ => return Err(invalid_fixed_borsh()),
                },
            },
            _ => return Err(invalid_fixed_borsh()),
        })
    }
}

impl BorshSerialize for EarnFundActionV1 {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        match self {
            Self::InitializeFund { params } => {
                1u8.serialize(writer)?;
                write_params(params, writer)
            }
            Self::ConfigureFund { params } => {
                2u8.serialize(writer)?;
                write_params(params, writer)
            }
            Self::Deposit {
                amount_atoms,
                mode,
                min_shares,
                source,
                position,
                proof,
            } => {
                3u8.serialize(writer)?;
                amount_atoms.serialize(writer)?;
                (*mode as u8).serialize(writer)?;
                min_shares.serialize(writer)?;
                source.serialize(writer)?;
                position.serialize(writer)?;
                proof.serialize(writer)
            }
            Self::Withdraw {
                pending_atoms,
                shares,
                min_instant_atoms,
                mode,
                position,
                proof,
            } => {
                4u8.serialize(writer)?;
                pending_atoms.serialize(writer)?;
                shares.serialize(writer)?;
                min_instant_atoms.serialize(writer)?;
                (*mode as u8).serialize(writer)?;
                position.serialize(writer)?;
                proof.serialize(writer)
            }
            Self::CompleteWithdrawal { position, proof } => {
                5u8.serialize(writer)?;
                position.serialize(writer)?;
                proof.serialize(writer)
            }
            Self::Allocate { amount_atoms } => {
                6u8.serialize(writer)?;
                amount_atoms.serialize(writer)
            }
            Self::Collect {
                cash_amount,
                cash_leaf_index,
                cash_root_index,
                cash_prove_by_index,
                proof,
            } => {
                7u8.serialize(writer)?;
                cash_amount.serialize(writer)?;
                cash_leaf_index.serialize(writer)?;
                cash_root_index.serialize(writer)?;
                cash_prove_by_index.serialize(writer)?;
                proof.serialize(writer)
            }
            Self::Roll => 8u8.serialize(writer),
            Self::ManageBuybackMark { op } => {
                9u8.serialize(writer)?;
                op.serialize(writer)
            }
        }
    }
}

pub(crate) mod wire;
