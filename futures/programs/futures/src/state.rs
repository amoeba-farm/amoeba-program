//! The market account: a fixed header followed by four arenas (traders,
//! orders, maker ladders, claims), all little-endian at fixed offsets.
//!
//! | Region | Record | Size |
//! |---|---|---|
//! | header | [`Header`] | 512 |
//! | traders | [`Trader`] × `trader_cap` | 64 each |
//! | orders | [`Order`] × `order_cap` | 64 each |
//! | makers | [`Maker`] head + `grid` × [`Step`] | 64 + 32·grid each |
//! | claims | [`Claim`] × `claim_cap` | 56 each |
use crate::{FuturesError, Knot};
use ameba_futures_math::{self as rm, Arc, ArcId, Side};
use solana_program::{program_error::ProgramError, pubkey::Pubkey};

/// "range": the deployed program's on-chain identity (PDA seeds); never changes.
pub const NAMESPACE: &[u8] = b"ameba-range-v1";
pub const REGISTRY_SEED: &[u8] = b"registry";
pub const AUTHORITY_SEED: &[u8] = b"authority";

/// "RANGE": the deployed program's on-chain identity (account magic); never changes.
pub const MAGIC: [u8; 8] = *b"AFRANGE1";
pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 512;
pub const TRADER_LEN: usize = 64;
pub const ORDER_LEN: usize = 64;
pub const CLAIM_LEN: usize = 56;
pub const MAKER_HEAD: usize = 64;
pub const STEP_LEN: usize = 32;
pub const REGISTRY_LEN: usize = 48;
/// "RNG": the deployed program's on-chain identity (account magic); never changes.
pub const REGISTRY_MAGIC: [u8; 8] = *b"AFRNGREG";
pub const POOL_SEED: &[u8] = b"pool";
pub const LP_SEED: &[u8] = b"lp";
pub const POOL_LEN: usize = 128;
pub const POOL_MAGIC: [u8; 8] = *b"AFRPOOL1";
pub const LP_LEN: usize = 128;
pub const LP_MAGIC: [u8; 8] = *b"AFRLPOS1";
pub const VAULT_SEED: &[u8] = b"earn";
pub const VAULT_LP_SEED: &[u8] = b"earn-lp";
pub const VAULT_MAGIC: [u8; 8] = *b"AFRVALT1";
pub const VAULT_LP_MAGIC: [u8; 8] = *b"AFRVLPS1";
/// Markets one Earn vault can be in at once. `VaultDeposit` and
/// `VaultWithdraw` value the vault in every one of its markets, so all of
/// them travel in one transaction: a withdrawal over 13 markets that draws on
/// two of them and creates the owner's token account, every instruction bound
/// to the governance gate, fills a legacy transaction's 1,232 bytes (`v5`;
/// `docs/futures.md`, "The Earn vault"). Funding a 14th market would leave
/// deposits and withdrawals unable to name them all, so it is refused
/// (`VaultCapacity`): a lost-funds protection, not a product limit. An
/// address lookup table would raise it to what one transaction can lock and
/// compute.
pub const VAULT_MARKETS: usize = 13;
const VAULT_HEAD: usize = 192;
pub const VAULT_LEN: usize = VAULT_HEAD + 32 * VAULT_MARKETS;
pub const VAULT_LP_LEN: usize = 128;
/// The reward stream's section ([`Rewards`]), appended to the vault account
/// after its markets the first time the operator sets a stream
/// (`VaultRewards`): the vault is `VAULT_LEN` bytes before, `VAULT_REWARDS_LEN`
/// after.
pub const REWARDS_LEN: usize = 192;
pub const VAULT_REWARDS_LEN: usize = VAULT_LEN + REWARDS_LEN;
pub const REWARDS_MAGIC: [u8; 8] = *b"AFRVRWD1";
/// Fixed point of [`Rewards::acc`]: reward atoms per share, times this.
pub const REWARD_SCALE: u128 = 1_000_000_000_000;

pub const STATUS_OPEN: u8 = 0;
pub const STATUS_SETTLED: u8 = 1;
/// The period failed (or the oracle never settled it): claims settle at the
/// market's reference value. A dev placeholder rule.
pub const STATUS_FAILED: u8 = 2;

/// Order flag: closes (and reserves) one of the owner's claims.
pub const ORDER_CLOSE: u8 = 1;

pub fn maker_len(grid: u16) -> usize {
    MAKER_HEAD + usize::from(grid) * STEP_LEN
}

pub fn market_len(grid: u16, traders: u16, orders: u16, makers: u8, claims: u32) -> usize {
    HEADER_LEN
        + usize::from(traders) * TRADER_LEN
        + usize::from(orders) * ORDER_LEN
        + usize::from(makers) * maker_len(grid)
        + claims as usize * CLAIM_LEN
}

pub fn registry_address(
    program_id: &Pubkey,
    product_id: &[u8; 32],
    expiry_ts: u64,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            NAMESPACE,
            REGISTRY_SEED,
            product_id,
            &expiry_ts.to_le_bytes(),
        ],
        program_id,
    )
}

pub fn authority_address(program_id: &Pubkey, market: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[NAMESPACE, AUTHORITY_SEED, market.as_ref()], program_id)
}

/// An Earn pool: `operator` runs its trader slot in `market`.
pub fn pool_address(program_id: &Pubkey, market: &Pubkey, operator: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[NAMESPACE, POOL_SEED, market.as_ref(), operator.as_ref()],
        program_id,
    )
}

/// One depositor's shares in a pool.
pub fn lp_address(program_id: &Pubkey, pool: &Pubkey, owner: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[NAMESPACE, LP_SEED, pool.as_ref(), owner.as_ref()],
        program_id,
    )
}

/// The Earn vault `operator` runs: one for the whole product.
pub fn vault_address(program_id: &Pubkey, operator: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[NAMESPACE, VAULT_SEED, operator.as_ref()], program_id)
}

/// One depositor's shares in an Earn vault.
pub fn vault_lp_address(program_id: &Pubkey, vault: &Pubkey, owner: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[NAMESPACE, VAULT_LP_SEED, vault.as_ref(), owner.as_ref()],
        program_id,
    )
}

pub fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
pub fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
pub fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
pub fn i64_at(b: &[u8], at: usize) -> i64 {
    i64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
pub fn u128_at(b: &[u8], at: usize) -> u128 {
    u128::from_le_bytes(b[at..at + 16].try_into().unwrap())
}
pub fn key_at(b: &[u8], at: usize) -> Pubkey {
    Pubkey::new_from_array(b[at..at + 32].try_into().unwrap())
}
fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
fn puti64(b: &mut [u8], at: usize, v: i64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
fn put128(b: &mut [u8], at: usize, v: u128) {
    b[at..at + 16].copy_from_slice(&v.to_le_bytes());
}
fn put_key(b: &mut [u8], at: usize, k: &Pubkey) {
    b[at..at + 32].copy_from_slice(k.as_ref());
}

// ---------------------------------------------------------------- Header

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub status: u8,
    pub authority_bump: u8,
    pub value_decimals: u8,
    pub grid: u16,
    pub trader_cap: u16,
    pub order_cap: u16,
    pub maker_cap: u8,
    pub registry_bump: u8,
    pub claim_cap: u32,
    /// Where the search for a free claim slot starts.
    pub claim_hint: u32,
    pub product_id: [u8; 32],
    pub expiry_ts: u64,
    pub period: Pubkey,
    pub oracle_program: Pubkey,
    pub quote_mint: Pubkey,
    pub vault: Pubkey,
    pub creator: Pubkey,
    pub k: u64,
    pub step_value: u64,
    /// Reference settlement (fine units) used if the period fails.
    pub ref_fine: u64,
    pub settlement_fine: u64,
    /// Arrival sequence: every order, ladder update and claim takes the next.
    pub next_seq: u64,
    pub total_free: u64,
    pub total_reserved: u64,
    /// Cash backing every open complete set.
    pub backing: u64,
    /// Deposits minus withdrawals: must equal free + reserved + backing.
    pub custody: u64,
    pub trades: u64,
    pub volume_lots: u64,
    pub volume_premium: u128,
    /// Average entry of the last trade, in fine units (0: none yet).
    pub last_entry_fine: u64,
    pub claims_open: u64,
    pub total_paid_out: u64,
    pub settled_ts: u64,
    pub last_trade_ts: u64,
}

impl Header {
    pub fn read(b: &[u8]) -> Result<Self, ProgramError> {
        if b.len() < HEADER_LEN || b[..8] != MAGIC || b[8] != VERSION {
            return Err(FuturesError::InvalidAccount.into());
        }
        let mut product_id = [0u8; 32];
        product_id.copy_from_slice(&b[32..64]);
        let h = Header {
            status: b[9],
            authority_bump: b[10],
            value_decimals: b[11],
            grid: u16_at(b, 12),
            trader_cap: u16_at(b, 14),
            order_cap: u16_at(b, 16),
            maker_cap: b[18],
            registry_bump: b[19],
            claim_cap: u32_at(b, 20),
            claim_hint: u32_at(b, 24),
            product_id,
            expiry_ts: u64_at(b, 64),
            period: key_at(b, 72),
            oracle_program: key_at(b, 104),
            quote_mint: key_at(b, 136),
            vault: key_at(b, 168),
            creator: key_at(b, 200),
            k: u64_at(b, 232),
            step_value: u64_at(b, 240),
            ref_fine: u64_at(b, 248),
            settlement_fine: u64_at(b, 256),
            next_seq: u64_at(b, 264),
            total_free: u64_at(b, 272),
            total_reserved: u64_at(b, 280),
            backing: u64_at(b, 288),
            custody: u64_at(b, 296),
            trades: u64_at(b, 304),
            volume_lots: u64_at(b, 312),
            volume_premium: u128_at(b, 320),
            last_entry_fine: u64_at(b, 336),
            claims_open: u64_at(b, 344),
            total_paid_out: u64_at(b, 352),
            settled_ts: u64_at(b, 360),
            last_trade_ts: u64_at(b, 368),
        };
        if b.len() != market_len(h.grid, h.trader_cap, h.order_cap, h.maker_cap, h.claim_cap) {
            return Err(FuturesError::InvalidAccount.into());
        }
        Ok(h)
    }

    pub fn write(&self, b: &mut [u8]) {
        b[..8].copy_from_slice(&MAGIC);
        b[8] = VERSION;
        b[9] = self.status;
        b[10] = self.authority_bump;
        b[11] = self.value_decimals;
        put16(b, 12, self.grid);
        put16(b, 14, self.trader_cap);
        put16(b, 16, self.order_cap);
        b[18] = self.maker_cap;
        b[19] = self.registry_bump;
        put32(b, 20, self.claim_cap);
        put32(b, 24, self.claim_hint);
        b[32..64].copy_from_slice(&self.product_id);
        put64(b, 64, self.expiry_ts);
        put_key(b, 72, &self.period);
        put_key(b, 104, &self.oracle_program);
        put_key(b, 136, &self.quote_mint);
        put_key(b, 168, &self.vault);
        put_key(b, 200, &self.creator);
        put64(b, 232, self.k);
        put64(b, 240, self.step_value);
        put64(b, 248, self.ref_fine);
        put64(b, 256, self.settlement_fine);
        put64(b, 264, self.next_seq);
        put64(b, 272, self.total_free);
        put64(b, 280, self.total_reserved);
        put64(b, 288, self.backing);
        put64(b, 296, self.custody);
        put64(b, 304, self.trades);
        put64(b, 312, self.volume_lots);
        put128(b, 320, self.volume_premium);
        put64(b, 336, self.last_entry_fine);
        put64(b, 344, self.claims_open);
        put64(b, 352, self.total_paid_out);
        put64(b, 360, self.settled_ts);
        put64(b, 368, self.last_trade_ts);
    }

    pub fn traders_at(&self) -> usize {
        HEADER_LEN
    }
    pub fn orders_at(&self) -> usize {
        self.traders_at() + usize::from(self.trader_cap) * TRADER_LEN
    }
    pub fn makers_at(&self) -> usize {
        self.orders_at() + usize::from(self.order_cap) * ORDER_LEN
    }
    pub fn claims_at(&self) -> usize {
        self.makers_at() + usize::from(self.maker_cap) * maker_len(self.grid)
    }
    pub fn trader_off(&self, i: u16) -> usize {
        self.traders_at() + usize::from(i) * TRADER_LEN
    }
    pub fn order_off(&self, i: u16) -> usize {
        self.orders_at() + usize::from(i) * ORDER_LEN
    }
    pub fn maker_off(&self, i: u8) -> usize {
        self.makers_at() + usize::from(i) * maker_len(self.grid)
    }
    pub fn step_off(&self, maker: u8, step: u16) -> usize {
        self.maker_off(maker) + MAKER_HEAD + usize::from(step) * STEP_LEN
    }
    pub fn claim_off(&self, i: u32) -> usize {
        self.claims_at() + i as usize * CLAIM_LEN
    }
}

// ---------------------------------------------------------------- Records

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Trader {
    /// Zero: an empty slot.
    pub owner: Pubkey,
    pub free: u64,
    pub reserved: u64,
    pub claims: u32,
    pub orders: u16,
    /// Maker slot + 1 (0: none).
    pub maker: u8,
    pub paid_out: u64,
}

impl Trader {
    pub fn read(b: &[u8], at: usize) -> Self {
        Trader {
            owner: key_at(b, at),
            free: u64_at(b, at + 32),
            reserved: u64_at(b, at + 40),
            claims: u32_at(b, at + 48),
            orders: u16_at(b, at + 52),
            maker: b[at + 54],
            paid_out: u64_at(b, at + 56),
        }
    }
    pub fn write(&self, b: &mut [u8], at: usize) {
        put_key(b, at, &self.owner);
        put64(b, at + 32, self.free);
        put64(b, at + 40, self.reserved);
        put32(b, at + 48, self.claims);
        put16(b, at + 52, self.orders);
        b[at + 54] = self.maker;
        b[at + 55] = 0;
        put64(b, at + 56, self.paid_out);
    }
    /// Holds nothing: the slot can be reused.
    pub fn idle(&self) -> bool {
        self.free == 0
            && self.reserved == 0
            && self.claims == 0
            && self.orders == 0
            && self.maker == 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Order {
    /// Arrival sequence (0: an empty slot).
    pub seq: u64,
    pub trader: u16,
    pub side: u8,
    pub flags: u8,
    pub lo: u16,
    pub hi: u16,
    /// Lots remaining.
    pub lots: u64,
    /// Limit premium per lot; resting orders trade at exactly this.
    pub price: u64,
    /// Cash reserved per remaining lot (0 for a close order).
    pub escrow_per_lot: u64,
    /// The claim a close order reserves.
    pub claim: u32,
    pub placed_lots: u64,
    pub placed_ts: u64,
}

impl Order {
    pub fn read(b: &[u8], at: usize) -> Self {
        Order {
            seq: u64_at(b, at),
            trader: u16_at(b, at + 8),
            side: b[at + 10],
            flags: b[at + 11],
            lo: u16_at(b, at + 12),
            hi: u16_at(b, at + 14),
            lots: u64_at(b, at + 16),
            price: u64_at(b, at + 24),
            escrow_per_lot: u64_at(b, at + 32),
            claim: u32_at(b, at + 40),
            placed_lots: u64_at(b, at + 48),
            placed_ts: u64_at(b, at + 56),
        }
    }
    pub fn write(&self, b: &mut [u8], at: usize) {
        put64(b, at, self.seq);
        put16(b, at + 8, self.trader);
        b[at + 10] = self.side;
        b[at + 11] = self.flags;
        put16(b, at + 12, self.lo);
        put16(b, at + 14, self.hi);
        put64(b, at + 16, self.lots);
        put64(b, at + 24, self.price);
        put64(b, at + 32, self.escrow_per_lot);
        put32(b, at + 40, self.claim);
        put32(b, at + 44, 0);
        put64(b, at + 48, self.placed_lots);
        put64(b, at + 56, self.placed_ts);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Claim {
    /// Lots (0: an empty slot).
    pub lots: u64,
    /// Changes on every split, merge, transfer and partial use.
    pub seq: u64,
    pub trader: u16,
    pub side: u8,
    pub lo: u16,
    pub hi: u16,
    /// Cash put in for these lots (cost basis, atoms).
    pub cost: u64,
    /// Lots reserved by the owner's resting close orders.
    pub reserved: u64,
    pub opened_ts: u64,
}

impl Claim {
    pub fn read(b: &[u8], at: usize) -> Self {
        Claim {
            lots: u64_at(b, at),
            seq: u64_at(b, at + 8),
            trader: u16_at(b, at + 16),
            side: b[at + 18],
            lo: u16_at(b, at + 20),
            hi: u16_at(b, at + 22),
            cost: u64_at(b, at + 24),
            reserved: u64_at(b, at + 32),
            opened_ts: u64_at(b, at + 40),
        }
    }
    pub fn write(&self, b: &mut [u8], at: usize) {
        put64(b, at, self.lots);
        put64(b, at + 8, self.seq);
        put16(b, at + 16, self.trader);
        b[at + 18] = self.side;
        b[at + 19] = 0;
        put16(b, at + 20, self.lo);
        put16(b, at + 22, self.hi);
        put64(b, at + 24, self.cost);
        put64(b, at + 32, self.reserved);
        put64(b, at + 40, self.opened_ts);
        put64(b, at + 48, 0);
    }
}

/// A maker ladder's head. Its steps follow at `MAKER_HEAD`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Maker {
    /// Arrival sequence of the current quote (0: no live quote).
    pub seq: u64,
    pub trader: u16,
    pub used: bool,
    pub lo: u16,
    pub hi: u16,
    /// Cash reserved for every step's remaining lots.
    pub escrow: u64,
    pub bid_spread: u32,
    pub ask_spread: u32,
    pub quoted_ts: u64,
    /// Depth tiers per step and side (0 and 1: a single price).
    pub tiers: Tiers,
    /// Lots each step quoted per side when the ladder was posted; with the
    /// remaining lots in the step they give each tier's remaining lots.
    pub bid_total: u64,
    pub ask_total: u64,
}

/// Lots per tier.
pub type TierLots = [u64; rm::MAX_TIERS as usize];

/// A ladder's depth tiers: each step quotes `count` prices per side,
/// `spread` atoms apart (tier 0 best), its lots split so each tier holds
/// `ratio` percent of the one before. Lots trade best tier first: the price
/// rule fills a cheaper tier before a dearer one, and the program checks it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tiers {
    pub count: u8,
    pub ratio: u16,
    pub spread: u32,
}

impl Tiers {
    pub fn n(&self) -> usize {
        usize::from(self.count.max(1))
    }

    /// Each tier's share of `total` lots (exact: the last takes the rest).
    pub fn sizes(&self, total: u64) -> TierLots {
        let mut out = [0u64; rm::MAX_TIERS as usize];
        let n = self.n();
        if n == 1 {
            out[0] = total;
            return out;
        }
        let mut w = [0u128; rm::MAX_TIERS as usize];
        w[0] = 100;
        for i in 1..n {
            w[i] = w[i - 1] * u128::from(self.ratio) / 100;
        }
        let sum: u128 = w[..n].iter().sum::<u128>().max(1);
        let mut used = 0u64;
        for i in 0..n - 1 {
            out[i] = (u128::from(total) * w[i] / sum) as u64;
            used += out[i];
        }
        out[n - 1] = total - used;
        out
    }

    /// Each tier's remaining lots when `remaining` of `total` are left.
    pub fn remaining(&self, total: u64, remaining: u64) -> TierLots {
        self.remaining_in(&self.sizes(total), total, remaining)
    }

    /// As [`Tiers::remaining`], with the tier sizes of `total` already known
    /// (they are the same for every step of a ladder).
    pub fn remaining_in(&self, sizes: &TierLots, total: u64, remaining: u64) -> TierLots {
        let mut out = [0u64; rm::MAX_TIERS as usize];
        if self.n() == 1 {
            out[0] = remaining;
            return out;
        }
        if remaining >= total {
            return *sizes;
        }
        let mut filled = total.saturating_sub(remaining);
        for i in 0..self.n() {
            let used = filled.min(sizes[i]);
            out[i] = sizes[i] - used;
            filled -= used;
        }
        out
    }

    /// How many tiers have lots left when `remaining` of `total` are left
    /// ([`Tiers::remaining_in`]'s nonzero entries): the remaining lots sit in
    /// the last tiers.
    pub fn live(&self, sizes: &TierLots, total: u64, remaining: u64) -> usize {
        let n = self.n();
        if n == 1 {
            return usize::from(remaining > 0);
        }
        if remaining >= total {
            return sizes[..n].iter().filter(|&&s| s > 0).count();
        }
        let (mut left, mut live) = (remaining, 0);
        for &size in sizes[..n].iter().rev() {
            if left == 0 {
                break;
            }
            if size > 0 {
                live += 1;
                left = left.saturating_sub(size);
            }
        }
        live
    }

    /// Tier `i`'s price from the step's best: bids step down, asks up,
    /// within `0..=k`.
    pub fn price(&self, best: u32, side: Side, i: usize, k: u64) -> u64 {
        let off = u64::from(self.spread) * i as u64;
        match side {
            Side::Long => u64::from(best).saturating_sub(off),
            Side::Short => (u64::from(best) + off).min(k),
        }
    }
}

impl Maker {
    pub fn read(b: &[u8], at: usize) -> Self {
        Maker {
            seq: u64_at(b, at),
            trader: u16_at(b, at + 8),
            used: b[at + 10] != 0,
            lo: u16_at(b, at + 12),
            hi: u16_at(b, at + 14),
            escrow: u64_at(b, at + 16),
            bid_spread: u32_at(b, at + 24),
            ask_spread: u32_at(b, at + 28),
            quoted_ts: u64_at(b, at + 32),
            tiers: Tiers {
                count: b[at + 40],
                ratio: u16_at(b, at + 42),
                spread: u32_at(b, at + 44),
            },
            bid_total: u64_at(b, at + 48),
            ask_total: u64_at(b, at + 56),
        }
    }
    pub fn write(&self, b: &mut [u8], at: usize) {
        put64(b, at, self.seq);
        put16(b, at + 8, self.trader);
        b[at + 10] = u8::from(self.used);
        b[at + 11] = 0;
        put16(b, at + 12, self.lo);
        put16(b, at + 14, self.hi);
        put64(b, at + 16, self.escrow);
        put32(b, at + 24, self.bid_spread);
        put32(b, at + 28, self.ask_spread);
        put64(b, at + 32, self.quoted_ts);
        b[at + 40] = self.tiers.count;
        b[at + 41] = 0;
        put16(b, at + 42, self.tiers.ratio);
        put32(b, at + 44, self.tiers.spread);
        put64(b, at + 48, self.bid_total);
        put64(b, at + 56, self.ask_total);
    }

    /// Tier sizes of each side's per-step lots (bids, asks): the same for
    /// every step, so computed once per ladder.
    pub fn sizes(&self) -> (TierLots, TierLots) {
        (
            self.tiers.sizes(self.bid_total),
            self.tiers.sizes(self.ask_total),
        )
    }

    /// Each tier's remaining lots on a step's bid and ask sides, given
    /// `sizes`.
    pub fn step_lots(&self, sizes: &(TierLots, TierLots), st: &Step) -> (TierLots, TierLots) {
        (
            self.tiers
                .remaining_in(&sizes.0, self.bid_total, st.bid_lots),
            self.tiers
                .remaining_in(&sizes.1, self.ask_total, st.ask_lots),
        )
    }

    /// The live arcs of a step: (side, tier, price, lots), given `sizes`, in
    /// canonical order (bids, then asks, each best tier first).
    pub fn step_arcs(
        &self,
        sizes: &(TierLots, TierLots),
        st: &Step,
        k: u64,
    ) -> impl Iterator<Item = (Side, usize, u64, u64)> + '_ {
        let (bids, asks) = self.step_lots(sizes, st);
        let (bid, ask) = (st.bid, st.ask);
        let n = self.tiers.n();
        (0..n)
            .map(move |i| {
                (
                    Side::Long,
                    i,
                    self.tiers.price(bid, Side::Long, i, k),
                    bids[i],
                )
            })
            .chain((0..n).map(move |i| {
                (
                    Side::Short,
                    i,
                    self.tiers.price(ask, Side::Short, i, k),
                    asks[i],
                )
            }))
            .filter(|a| a.3 > 0)
    }

    /// Cash a step's remaining lots need: `bid` per bid lot, `k - ask` per
    /// ask lot, tier by tier, except the lots its inventory backs. A step's
    /// first ask lots to fill (best tier first), as many as it holds long,
    /// sell that inventory: the fill redeems the set and pays the ask's
    /// share from it, as a close order pays from the claim it reserves.
    /// Likewise its first bid lots, as many as it holds short. `Quote`
    /// reserves this, summed over the ladder's steps; a fill pays its
    /// escrowed lots' share from it, and what else it frees (inventory now
    /// backing the step's other side) stays reserved until the next quote,
    /// so the escrow never falls short of what the steps need.
    pub fn step_escrow(&self, sizes: &(TierLots, TierLots), st: &Step, k: u64) -> Option<u64> {
        let (bids, asks) = self.step_lots(sizes, st);
        let mut long = u64::try_from(st.inv.max(0)).ok()?;
        let mut short = if st.inv < 0 { st.inv.unsigned_abs() } else { 0 };
        let mut total = 0u64;
        for i in 0..self.tiers.n() {
            let backed = bids[i].min(short);
            short -= backed;
            if bids[i] > backed {
                let per = self.tiers.price(st.bid, Side::Long, i, k);
                total = total.checked_add((bids[i] - backed).checked_mul(per)?)?;
            }
            let backed = asks[i].min(long);
            long -= backed;
            if asks[i] > backed {
                let per = k.checked_sub(self.tiers.price(st.ask, Side::Short, i, k))?;
                total = total.checked_add((asks[i] - backed).checked_mul(per)?)?;
            }
        }
        Some(total)
    }
}

/// One ladder step `[x, x+1)`: the maker's quote and its inventory there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Step {
    pub ask: u32,
    pub bid: u32,
    pub ask_lots: u64,
    pub bid_lots: u64,
    /// Net one-step claims held: positive long, negative short.
    pub inv: i64,
}

impl Step {
    pub fn read(b: &[u8], at: usize) -> Self {
        Step {
            ask: u32_at(b, at),
            bid: u32_at(b, at + 4),
            ask_lots: u64_at(b, at + 8),
            bid_lots: u64_at(b, at + 16),
            inv: i64_at(b, at + 24),
        }
    }
    pub fn write(&self, b: &mut [u8], at: usize) {
        put32(b, at, self.ask);
        put32(b, at + 4, self.bid);
        put64(b, at + 8, self.ask_lots);
        put64(b, at + 16, self.bid_lots);
        puti64(b, at + 24, self.inv);
    }
    /// Cash reserved for this step's remaining lots at a single price (a
    /// one-tier ladder; see [`Maker::step_escrow`]).
    pub fn escrow(&self, k: u64) -> Option<u64> {
        let ask = self
            .ask_lots
            .checked_mul(k.checked_sub(u64::from(self.ask))?)?;
        let bid = self.bid_lots.checked_mul(u64::from(self.bid))?;
        ask.checked_add(bid)
    }
}

/// One redemption a fresh claim's netting plans ([`plan_net`]): `lots` of
/// the fresh claim's piece `[piece.0, piece.1)` against the owner's claim in
/// slot `claim`, over their overlap `[lo, hi)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetMatch {
    pub claim: u32,
    pub piece: (u16, u16),
    pub lo: u16,
    pub hi: u16,
    pub lots: u64,
}

/// How a fresh claim of `lots` on `[lo, hi)` nets against its owner's
/// opposite claims `opposite` (slot and claim; their unreserved lots): every
/// overlap is redeemed for its sets and the parts outside it kept, `Split`
/// and `Redeem` in one, so the owner's payoff is unchanged at every
/// settlement. Exact complements come first, then the other overlapping
/// claims in slot order, each once; what is left of the fresh claim (its
/// unmatched lots, and the parts outside a match) goes on to the next.
pub fn plan_net(lo: u16, hi: u16, lots: u64, opposite: &[(u32, Claim)]) -> Vec<NetMatch> {
    let mut order: Vec<&(u32, Claim)> = opposite
        .iter()
        .filter(|(_, c)| c.lo < hi && lo < c.hi && c.lots > c.reserved)
        .collect();
    order.sort_by_key(|(i, c)| (c.lo != lo || c.hi != hi, *i));
    let mut pieces: Vec<(u16, u16, u64)> = vec![(lo, hi, lots)];
    let mut out = Vec::new();
    for (i, c) in order {
        let mut usable = c.lots - c.reserved;
        let mut k = 0;
        while k < pieces.len() && usable > 0 {
            let (pl, ph, pn) = pieces[k];
            let (a, z) = (pl.max(c.lo), ph.min(c.hi));
            k += 1;
            if pn == 0 || a >= z {
                continue;
            }
            let m = pn.min(usable);
            out.push(NetMatch {
                claim: *i,
                piece: (pl, ph),
                lo: a,
                hi: z,
                lots: m,
            });
            usable -= m;
            pieces[k - 1].2 -= m;
            // The parts outside `c` (they never overlap it again).
            if pl < a {
                pieces.push((pl, a, m));
            }
            if z < ph {
                pieces.push((z, ph, m));
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Registry {
    pub bump: u8,
    pub market: Pubkey,
}

impl Registry {
    pub fn read(b: &[u8]) -> Result<Self, ProgramError> {
        if b.len() != REGISTRY_LEN || b[..8] != REGISTRY_MAGIC {
            return Err(FuturesError::InvalidAccount.into());
        }
        Ok(Registry {
            bump: b[8],
            market: key_at(b, 16),
        })
    }
    pub fn write(&self, b: &mut [u8]) {
        b[..8].copy_from_slice(&REGISTRY_MAGIC);
        b[8] = self.bump;
        put_key(b, 16, &self.market);
    }
}

/// An Earn pool (`POOL_LEN` bytes). Its cash and inventory are the market
/// trader whose owner is the pool's address.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pool {
    pub bump: u8,
    pub market: Pubkey,
    pub operator: Pubkey,
    pub shares: u64,
    /// Cash ever deposited and withdrawn (informational).
    pub deposited: u64,
    pub withdrawn: u64,
    pub created_ts: u64,
}

impl Pool {
    pub fn read(b: &[u8]) -> Result<Self, ProgramError> {
        if b.len() != POOL_LEN || b[..8] != POOL_MAGIC {
            return Err(FuturesError::InvalidAccount.into());
        }
        Ok(Pool {
            bump: b[8],
            market: key_at(b, 16),
            operator: key_at(b, 48),
            shares: u64_at(b, 80),
            deposited: u64_at(b, 88),
            withdrawn: u64_at(b, 96),
            created_ts: u64_at(b, 104),
        })
    }
    pub fn write(&self, b: &mut [u8]) {
        b[..8].copy_from_slice(&POOL_MAGIC);
        b[8] = self.bump;
        put_key(b, 16, &self.market);
        put_key(b, 48, &self.operator);
        put64(b, 80, self.shares);
        put64(b, 88, self.deposited);
        put64(b, 96, self.withdrawn);
        put64(b, 104, self.created_ts);
    }
}

/// A depositor's position in a pool (`LP_LEN` bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LpPosition {
    pub bump: u8,
    pub pool: Pubkey,
    pub owner: Pubkey,
    pub shares: u64,
    /// Cash this owner ever deposited and withdrawn (informational).
    pub deposited: u64,
    pub withdrawn: u64,
}

impl LpPosition {
    pub fn read(b: &[u8]) -> Result<Self, ProgramError> {
        if b.len() != LP_LEN || b[..8] != LP_MAGIC {
            return Err(FuturesError::InvalidAccount.into());
        }
        Ok(LpPosition {
            bump: b[8],
            pool: key_at(b, 16),
            owner: key_at(b, 48),
            shares: u64_at(b, 80),
            deposited: u64_at(b, 88),
            withdrawn: u64_at(b, 96),
        })
    }
    pub fn write(&self, b: &mut [u8]) {
        b[..8].copy_from_slice(&LP_MAGIC);
        b[8] = self.bump;
        put_key(b, 16, &self.pool);
        put_key(b, 48, &self.owner);
        put64(b, 80, self.shares);
        put64(b, 88, self.deposited);
        put64(b, 96, self.withdrawn);
    }
}

/// The Earn vault (`VAULT_LEN` bytes): depositors' shares of everything it
/// holds, its idle cash in `cash_account` and its trader slot (owner: the
/// vault's address) in each of `markets`, which its operator trades and
/// moves cash between but never withdraws.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vault {
    pub bump: u8,
    pub operator: Pubkey,
    pub quote_mint: Pubkey,
    /// The vault's USDC token account (owner: the vault's address).
    pub cash_account: Pubkey,
    /// Idle cash: what `cash_account` holds for the vault (a gift to that
    /// account is not counted).
    pub cash: u64,
    pub shares: u64,
    /// Cash ever deposited and withdrawn (informational).
    pub deposited: u64,
    pub withdrawn: u64,
    pub created_ts: u64,
    /// Every market where the vault may hold a trader slot, in order.
    pub markets: Vec<Pubkey>,
    /// The reward stream, once the operator has set one.
    pub rewards: Option<Rewards>,
}

impl Vault {
    pub fn read(b: &[u8]) -> Result<Self, ProgramError> {
        if (b.len() != VAULT_LEN && b.len() != VAULT_REWARDS_LEN)
            || b[..8] != VAULT_MAGIC
            || usize::from(b[9]) > VAULT_MARKETS
        {
            return Err(FuturesError::InvalidAccount.into());
        }
        let rewards = match b.get(VAULT_LEN..) {
            Some(r) if !r.is_empty() => Some(Rewards::read(r)?),
            _ => None,
        };
        Ok(Vault {
            bump: b[8],
            operator: key_at(b, 16),
            quote_mint: key_at(b, 48),
            cash_account: key_at(b, 80),
            cash: u64_at(b, 112),
            shares: u64_at(b, 120),
            deposited: u64_at(b, 128),
            withdrawn: u64_at(b, 136),
            created_ts: u64_at(b, 144),
            markets: (0..usize::from(b[9]))
                .map(|i| key_at(b, VAULT_HEAD + 32 * i))
                .collect(),
            rewards,
        })
    }
    pub fn write(&self, b: &mut [u8]) {
        b[..8].copy_from_slice(&VAULT_MAGIC);
        b[8] = self.bump;
        b[9] = self.markets.len() as u8;
        put_key(b, 16, &self.operator);
        put_key(b, 48, &self.quote_mint);
        put_key(b, 80, &self.cash_account);
        put64(b, 112, self.cash);
        put64(b, 120, self.shares);
        put64(b, 128, self.deposited);
        put64(b, 136, self.withdrawn);
        put64(b, 144, self.created_ts);
        b[VAULT_HEAD..VAULT_LEN].fill(0);
        for (i, m) in self.markets.iter().enumerate() {
            put_key(b, VAULT_HEAD + 32 * i, m);
        }
        if let (Some(r), Some(at)) = (&self.rewards, b.get_mut(VAULT_LEN..VAULT_REWARDS_LEN)) {
            r.write(at);
        }
    }
}

/// The Earn vault's reward stream (`REWARDS_LEN` bytes after the vault's
/// markets): `amount` atoms of `mint`, held in `account` (a token account of
/// the mint owned by the vault's address), released linearly from `start_ts`
/// to `end_ts` to the depositors by their shares, except `house`'s (the
/// protocol's own deposit). A share's part accumulates in `acc` (atoms per
/// share, times `REWARD_SCALE`), which only grows; each position settles its
/// part when its shares change or it is paid (`VaultRewardsClaim`).
/// What is released while only the house holds shares goes to nobody and
/// stays with the operator's surplus (`VaultRewardsWithdraw`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rewards {
    pub mint: Pubkey,
    pub account: Pubkey,
    /// The owner whose shares earn nothing (`Pubkey::default()`: none).
    pub house: Pubkey,
    /// The mint's decimals (its transfers are checked).
    pub decimals: u8,
    pub start_ts: u64,
    pub end_ts: u64,
    pub amount: u64,
    /// Accrued up to here.
    pub last_ts: u64,
    pub acc: u128,
    /// The house's shares, left out of every share's part.
    pub house_shares: u64,
    /// Atoms ever accrued to depositors, and ever paid to them.
    pub distributed: u64,
    pub paid: u64,
}

impl Rewards {
    /// `b`: the `REWARDS_LEN` bytes after the vault's markets.
    pub fn read(b: &[u8]) -> Result<Self, ProgramError> {
        if b.len() != REWARDS_LEN || b[..8] != REWARDS_MAGIC {
            return Err(FuturesError::InvalidAccount.into());
        }
        Ok(Rewards {
            decimals: b[8],
            mint: key_at(b, 16),
            account: key_at(b, 48),
            house: key_at(b, 80),
            start_ts: u64_at(b, 112),
            end_ts: u64_at(b, 120),
            amount: u64_at(b, 128),
            last_ts: u64_at(b, 136),
            acc: u128_at(b, 144),
            house_shares: u64_at(b, 160),
            distributed: u64_at(b, 168),
            paid: u64_at(b, 176),
        })
    }
    pub fn write(&self, b: &mut [u8]) {
        b.fill(0);
        b[..8].copy_from_slice(&REWARDS_MAGIC);
        b[8] = self.decimals;
        put_key(b, 16, &self.mint);
        put_key(b, 48, &self.account);
        put_key(b, 80, &self.house);
        put64(b, 112, self.start_ts);
        put64(b, 120, self.end_ts);
        put64(b, 128, self.amount);
        put64(b, 136, self.last_ts);
        put128(b, 144, self.acc);
        put64(b, 160, self.house_shares);
        put64(b, 168, self.distributed);
        put64(b, 176, self.paid);
    }

    /// What the stream has released by `t`: nothing before `start_ts`,
    /// `amount` from `end_ts`, linear between (rounded down).
    pub fn released_at(&self, t: u64) -> u64 {
        if t <= self.start_ts || self.end_ts <= self.start_ts {
            0
        } else if t >= self.end_ts {
            self.amount
        } else {
            let part = u128::from(self.amount) * u128::from(t - self.start_ts)
                / u128::from(self.end_ts - self.start_ts);
            part as u64
        }
    }

    /// Accrues what the stream released since `last_ts` up to `now` to a
    /// vault of `shares` shares (before they change). `None` on overflow.
    pub fn accrue(&mut self, shares: u64, now: u64) -> Option<()> {
        let from = self.last_ts.max(self.start_ts);
        let to = now.min(self.end_ts);
        if to > from {
            let released = self.released_at(to) - self.released_at(from);
            let earning = shares.saturating_sub(self.house_shares);
            if earning > 0 && released > 0 {
                let per = u128::from(released).checked_mul(REWARD_SCALE)? / u128::from(earning);
                self.acc = self.acc.checked_add(per)?;
                self.distributed = self.distributed.checked_add(released)?;
            }
        }
        self.last_ts = self.last_ts.max(now);
        Some(())
    }

    /// Settles what `p` earned since its last settlement into its pending
    /// rewards (the house earns nothing). `None` on overflow.
    pub fn settle(&self, p: &mut VaultPosition) -> Option<()> {
        if p.owner != self.house {
            let per = self.acc.checked_sub(p.reward_snapshot)?;
            let earned = u128::from(p.shares).checked_mul(per)? / REWARD_SCALE;
            p.reward_pending = p.reward_pending.checked_add(u64::try_from(earned).ok()?)?;
        }
        p.reward_snapshot = self.acc;
        Some(())
    }

    /// Accrued to depositors and not paid yet.
    pub fn owed(&self) -> u64 {
        self.distributed.saturating_sub(self.paid)
    }

    /// What the stream still releases after `now` (accrued to `now`).
    pub fn left(&self, now: u64) -> u64 {
        self.amount - self.released_at(now.max(self.last_ts))
    }
}

/// A depositor's shares in the Earn vault (`VAULT_LP_LEN` bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VaultPosition {
    pub bump: u8,
    pub vault: Pubkey,
    pub owner: Pubkey,
    pub shares: u64,
    /// Cash this owner ever deposited and withdrawn (informational).
    pub deposited: u64,
    pub withdrawn: u64,
    /// The vault's `Rewards::acc` at this position's last settlement.
    pub reward_snapshot: u128,
    /// Reward atoms settled and not paid yet.
    pub reward_pending: u64,
    /// Reward atoms ever paid (informational): stored in seven bytes, so it
    /// stops at `REWARD_PAID_MAX`.
    pub reward_paid: u64,
}

/// The most `VaultPosition::reward_paid` holds (2^56 - 1 atoms).
pub const REWARD_PAID_MAX: u64 = (1 << 56) - 1;

impl VaultPosition {
    pub fn read(b: &[u8]) -> Result<Self, ProgramError> {
        if b.len() != VAULT_LP_LEN || b[..8] != VAULT_LP_MAGIC {
            return Err(FuturesError::InvalidAccount.into());
        }
        Ok(VaultPosition {
            bump: b[8],
            vault: key_at(b, 16),
            owner: key_at(b, 48),
            shares: u64_at(b, 80),
            deposited: u64_at(b, 88),
            withdrawn: u64_at(b, 96),
            reward_snapshot: u128_at(b, 104),
            reward_pending: u64_at(b, 120),
            reward_paid: u64_at(b, 8) >> 8,
        })
    }
    pub fn write(&self, b: &mut [u8]) {
        b[..8].copy_from_slice(&VAULT_LP_MAGIC);
        put64(b, 8, self.reward_paid.min(REWARD_PAID_MAX) << 8);
        b[8] = self.bump;
        put_key(b, 16, &self.vault);
        put_key(b, 48, &self.owner);
        put64(b, 80, self.shares);
        put64(b, 88, self.deposited);
        put64(b, 96, self.withdrawn);
        put128(b, 104, self.reward_snapshot);
        put64(b, 120, self.reward_pending);
    }
}

/// What trader `t` holds is worth, as `(lowest, highest)` atoms over every
/// settlement outcome: free and reserved cash plus everything its claims and
/// ladder inventory pay. Once the market is final it is the one outcome.
///
/// Payoffs are linear between grid positions, so the extremes are at grid
/// positions; a slope sweep over the grid finds them in one pass.
pub fn trader_value(b: &[u8], h: &Header, t: u16) -> Option<(u64, u64)> {
    trader_value_with(b, h, t, &mut Vec::new())
}

/// [`trader_value`] with a reusable work buffer: the program's allocator
/// never frees, so valuing many markets in one instruction reuses one.
pub fn trader_value_with(
    b: &[u8],
    h: &Header,
    t: u16,
    delta: &mut Vec<i128>,
) -> Option<(u64, u64)> {
    let tr = Trader::read(b, h.trader_off(t));
    let cash = u128::from(tr.free) + u128::from(tr.reserved);
    let open = h.status == STATUS_OPEN;
    let s = h.settlement_fine;
    let n = usize::from(h.grid) + 1;
    // Per-grid-step slope changes, in lots; `base`: the shorts' full sets.
    delta.clear();
    delta.resize(n + 1, 0);
    let mut base = 0i128;
    let mut owed = 0u128;
    let mut add = |long: bool, lo: u16, hi: u16, lots: u64| {
        let q = i128::from(lots);
        let sign = if long { 1 } else { -1 };
        if !long {
            base += q * i128::from(hi - lo);
        }
        delta[usize::from(lo)] += sign * q;
        delta[usize::from(hi)] -= sign * q;
        if !open {
            let per = if long {
                rm::payoff::long_per_lot(lo, hi, s, h.k)
            } else {
                rm::payoff::short_per_lot(lo, hi, s, h.k)
            };
            owed += u128::from(lots) * u128::from(per);
        }
    };
    if tr.maker > 0 {
        let m = tr.maker - 1;
        for x in 0..h.grid {
            let inv = Step::read(b, h.step_off(m, x)).inv;
            if inv != 0 {
                add(inv > 0, x, x + 1, inv.unsigned_abs());
            }
        }
    }
    for i in 0..h.claim_cap {
        // Lots and trader first: most slots are empty or someone else's.
        let at = h.claim_off(i);
        if u64_at(b, at) == 0 || u16_at(b, at + 16) != t {
            continue;
        }
        let c = Claim::read(b, at);
        add(c.side == Side::Long as u8, c.lo, c.hi, c.lots);
    }
    if !open {
        let v = u64::try_from(cash + owed).ok()?;
        return Some((v, v));
    }
    let (mut lo, mut hi) = (i128::MAX, i128::MIN);
    let (mut d, mut slope) = (0i128, 0i128);
    for dx in &delta[..n] {
        let v = base + d;
        lo = lo.min(v);
        hi = hi.max(v);
        slope += dx;
        d += slope;
    }
    let k = i128::from(h.k);
    let value = |v: i128| u64::try_from(cash as i128 + v * k).ok();
    Some((value(lo)?, value(hi)?))
}

// ---------------------------------------------------------------- the book

/// Every live order and ladder step as package arcs, in canonical order
/// (orders by slot, then ladders by maker, step, bid before ask). The program
/// verifies packages against exactly this list; clients solve over it.
pub fn book_arcs(b: &[u8], h: &Header) -> Vec<Arc> {
    // Exact capacity: the program's allocator never frees.
    let mut cap = 0usize;
    for slot in 0..h.order_cap {
        let at = h.order_off(slot);
        cap += usize::from(u64_at(b, at) != 0 && u64_at(b, at + 16) != 0);
    }
    for m in 0..h.maker_cap {
        let mk = Maker::read(b, h.maker_off(m));
        if mk.used && mk.seq != 0 {
            cap += 2 * mk.tiers.n() * usize::from(mk.hi.saturating_sub(mk.lo));
        }
    }
    let mut arcs = Vec::with_capacity(cap);
    for slot in 0..h.order_cap {
        let o = Order::read(b, h.order_off(slot));
        if o.seq == 0 || o.lots == 0 {
            continue;
        }
        arcs.push(Arc {
            id: ArcId::Order(slot),
            side: if o.side == 0 { Side::Long } else { Side::Short },
            lo: o.lo,
            hi: o.hi,
            price: o.price,
            cap: o.lots,
            rank: rm::rank(o.seq, 0),
        });
    }
    for m in 0..h.maker_cap {
        let mk = Maker::read(b, h.maker_off(m));
        if !mk.used || mk.seq == 0 {
            continue;
        }
        let sizes = mk.sizes();
        let n = mk.tiers.n();
        let seq = rm::rank(mk.seq, 0);
        for x in mk.lo..mk.hi {
            let st = Step::read(b, h.step_off(m, x));
            let (bids, asks) = mk.step_lots(&sizes, &st);
            for (side, lots, best) in [(Side::Long, &bids, st.bid), (Side::Short, &asks, st.ask)] {
                for (tier, &cap) in lots[..n].iter().enumerate() {
                    if cap == 0 {
                        continue;
                    }
                    let tier = tier as u8;
                    arcs.push(Arc {
                        id: ArcId::Maker {
                            maker: m,
                            step: x,
                            side,
                            tier,
                        },
                        side,
                        lo: x,
                        hi: x + 1,
                        price: mk.tiers.price(best, side, usize::from(tier), h.k),
                        cap,
                        rank: seq | u128::from(rm::maker_sub(x, side, tier)),
                    });
                }
            }
        }
    }
    arcs
}

/// How many ladder arcs (steps x tiers x sides with lots) the book holds,
/// counted without building them.
pub fn ladder_arcs(b: &[u8], h: &Header) -> usize {
    let mut n = 0;
    for m in 0..h.maker_cap {
        let mk = Maker::read(b, h.maker_off(m));
        if !mk.used || mk.seq == 0 {
            continue;
        }
        let sizes = mk.sizes();
        for x in mk.lo..mk.hi {
            let st = Step::read(b, h.step_off(m, x));
            n += mk.tiers.live(&sizes.0, mk.bid_total, st.bid_lots)
                + mk.tiers.live(&sizes.1, mk.ask_total, st.ask_lots);
        }
    }
    n
}

/// A maker ladder quote over `[lo, hi)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ladder {
    pub lo: u16,
    pub hi: u16,
    pub bid_spread: u32,
    pub ask_spread: u32,
    pub bid_lots: u64,
    pub ask_lots: u64,
    /// Keep consumption: each step's lots become at most what is left.
    pub keep: bool,
    /// Depth tiers (0 or 1: one price per step and side).
    pub tiers: Tiers,
}

impl Ladder {
    /// The whole ladder as its core: every step quotes every tier.
    pub fn full(&self) -> Core {
        Core {
            lo: self.lo,
            hi: self.hi,
        }
    }
}

/// The dense part `[lo, hi)` of a ladder: its steps quote every tier, the
/// ladder's other steps only the last (widest) tier's lots, a thinner and
/// wider quote for wide (low-leverage) ranges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Core {
    pub lo: u16,
    pub hi: u16,
}

/// Piecewise-linear mid at `x` through knots sorted by position (flat beyond
/// the ends).
pub fn mid_at(knots: &[Knot], x: u16) -> u64 {
    Mids::new(knots).at(x)
}

/// [`mid_at`] along increasing positions, walking the knots once.
pub struct Mids<'a> {
    knots: &'a [Knot],
    w: usize,
}

impl<'a> Mids<'a> {
    pub fn new(knots: &'a [Knot]) -> Self {
        Mids { knots, w: 0 }
    }

    /// The mid at `x`, no lower than the last `x` asked for.
    pub fn at(&mut self, x: u16) -> u64 {
        let k = self.knots;
        if x <= k[0].pos {
            return u64::from(k[0].price);
        }
        // Prices fit u32 and spans u16, so the product fits i64.
        while self.w + 1 < k.len() && k[self.w + 1].pos < x {
            self.w += 1;
        }
        if self.w + 1 == k.len() {
            return u64::from(k[self.w].price);
        }
        let (a, c) = (k[self.w], k[self.w + 1]);
        let (pa, pc) = (i64::from(a.price), i64::from(c.price));
        (pa + (pc - pa) * i64::from(x - a.pos) / i64::from(c.pos - a.pos)) as u64
    }
}

/// Writes maker `m`'s ladder quote: its tiers and per-step lots in the head,
/// and every step's best prices and lots (inventory untouched); returns the
/// escrow its lots need. Steps outside the ladder get no lots; steps outside
/// its core start with only the last tier's lots (tiers trade best first, so
/// a step's remaining lots sit in its last tiers). A step quotes a bid only
/// where `mid >= bid_spread` and an ask only where `mid + ask_spread <= k`;
/// dearer tiers clip at 0 and `k`.
pub fn write_ladder(
    b: &mut [u8],
    h: &Header,
    m: u8,
    l: &Ladder,
    core: Core,
    knots: &[Knot],
) -> Option<u64> {
    let at = h.maker_off(m);
    let mut mk = Maker::read(b, at);
    mk.tiers = l.tiers;
    mk.bid_total = l.bid_lots;
    mk.ask_total = l.ask_lots;
    mk.write(b, at);
    let sizes = mk.sizes();
    let last = mk.tiers.n() - 1;
    let mut mids = Mids::new(knots);
    let mut escrow = 0u64;
    for x in 0..h.grid {
        let sat = h.step_off(m, x);
        let mut st = Step::read(b, sat);
        // What is left of the previous quote, in lots (all tiers).
        let (left_bid, left_ask) = (st.bid_lots, st.ask_lots);
        st.bid_lots = 0;
        st.ask_lots = 0;
        if x >= l.lo && x < l.hi {
            let mid = mids.at(x);
            let dense = x >= core.lo && x < core.hi;
            let (bid_lots, ask_lots) = if dense {
                (l.bid_lots, l.ask_lots)
            } else {
                (sizes.0[last], sizes.1[last])
            };
            if mid >= u64::from(l.bid_spread) {
                st.bid = (mid - u64::from(l.bid_spread)) as u32;
                st.bid_lots = if l.keep {
                    bid_lots.min(left_bid)
                } else {
                    bid_lots
                };
            }
            let ask = mid + u64::from(l.ask_spread);
            if ask <= h.k {
                st.ask = ask as u32;
                st.ask_lots = if l.keep {
                    ask_lots.min(left_ask)
                } else {
                    ask_lots
                };
            }
            escrow = escrow.checked_add(mk.step_escrow(&sizes, &st, h.k)?)?;
        }
        st.write(b, sat);
    }
    Some(escrow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_tiers_count_the_nonzero_remaining_tiers() {
        for count in 0..=rm::MAX_TIERS {
            for ratio in [50, 100, 120, 300] {
                let t = Tiers {
                    count,
                    ratio,
                    spread: 1,
                };
                for total in [0, 1, 2, 3, 7, 10, 100, 7_440] {
                    let sizes = t.sizes(total);
                    for remaining in 0..=total + 2 {
                        let want = t.remaining_in(&sizes, total, remaining)[..t.n()]
                            .iter()
                            .filter(|&&l| l > 0)
                            .count();
                        assert_eq!(
                            t.live(&sizes, total, remaining),
                            want,
                            "{t:?} {total} {remaining}"
                        );
                    }
                }
            }
        }
    }

    /// The straightforward interpolation `Mids` walks.
    fn reference(knots: &[Knot], x: u16) -> u64 {
        if x <= knots[0].pos {
            return u64::from(knots[0].price);
        }
        for w in knots.windows(2) {
            if x <= w[1].pos {
                let (pa, pc) = (i128::from(w[0].price), i128::from(w[1].price));
                return (pa + (pc - pa) * i128::from(x - w[0].pos) / i128::from(w[1].pos - w[0].pos))
                    as u64;
            }
        }
        u64::from(knots[knots.len() - 1].price)
    }

    #[test]
    fn mids_walk_matches_the_interpolation() {
        let knots = [
            Knot {
                pos: 3,
                price: 9_900,
            },
            Knot {
                pos: 11,
                price: 9_871,
            },
            Knot {
                pos: 12,
                price: 5_000,
            },
            Knot {
                pos: 40,
                price: 100,
            },
            Knot {
                pos: 41,
                price: 4_294_967_295,
            },
            Knot { pos: 199, price: 0 },
        ];
        let mut mids = Mids::new(&knots);
        for x in 0..=200 {
            assert_eq!(mids.at(x), reference(&knots, x), "x {x}");
            assert_eq!(mid_at(&knots, x), reference(&knots, x), "x {x}");
        }
        let one = [Knot { pos: 7, price: 42 }];
        let mut mids = Mids::new(&one);
        for x in 0..20 {
            assert_eq!(mids.at(x), 42);
        }
    }

    #[test]
    fn rewards_round_trip_and_split_by_shares() {
        let mut r = Rewards {
            mint: Pubkey::new_unique(),
            account: Pubkey::new_unique(),
            house: Pubkey::new_unique(),
            decimals: 9,
            start_ts: 1_000,
            end_ts: 4_000,
            amount: 3_000_000,
            last_ts: 1_000,
            house_shares: 600,
            ..Rewards::default()
        };
        // 1,000 of 3,000 seconds: a third, to the 400 shares outside the house.
        r.accrue(1_000, 2_000).unwrap();
        assert_eq!((r.distributed, r.last_ts), (1_000_000, 2_000));
        let mut p = VaultPosition {
            owner: Pubkey::new_unique(),
            shares: 100,
            ..VaultPosition::default()
        };
        r.settle(&mut p).unwrap();
        assert_eq!((p.reward_pending, p.reward_snapshot), (250_000, r.acc));
        let mut house = VaultPosition {
            owner: r.house,
            shares: 600,
            ..VaultPosition::default()
        };
        r.settle(&mut house).unwrap();
        assert_eq!(house.reward_pending, 0, "the house earns nothing");
        assert_eq!(r.left(2_000), 2_000_000);
        // Past the end nothing more is released; only the house: nobody's.
        r.house_shares = 1_000;
        r.accrue(1_000, 9_000).unwrap();
        assert_eq!((r.distributed, r.left(9_000)), (1_000_000, 0));
        // The vault account grows by the section; the position keeps its bytes.
        let v = Vault {
            markets: vec![Pubkey::new_unique()],
            rewards: Some(r),
            ..Vault::default()
        };
        let mut b = vec![0u8; VAULT_REWARDS_LEN];
        v.write(&mut b);
        assert_eq!(Vault::read(&b).unwrap(), v);
        let plain = Vault {
            rewards: None,
            ..v.clone()
        };
        assert_eq!(Vault::read(&b[..VAULT_LEN]).unwrap(), plain);
        p.bump = 254;
        p.reward_paid = u64::MAX;
        let mut b = vec![0u8; VAULT_LP_LEN];
        p.write(&mut b);
        let back = VaultPosition::read(&b).unwrap();
        assert_eq!(
            (back.bump, back.reward_paid),
            (254, REWARD_PAID_MAX),
            "paid stops at 2^56 - 1"
        );
        assert_eq!(
            VaultPosition {
                reward_paid: u64::MAX,
                ..back
            },
            p
        );
    }

    #[test]
    fn inventory_backs_the_first_lots_to_fill() {
        let k = 10_000;
        // Three tiers 100 atoms apart, 50/40/30 lots... (ratio 80%).
        let mk = Maker {
            tiers: Tiers {
                count: 3,
                ratio: 80,
                spread: 100,
            },
            bid_total: 100,
            ask_total: 100,
            ..Maker::default()
        };
        let sizes = mk.sizes();
        let st = Step {
            ask: 6_000,
            bid: 4_000,
            ask_lots: 100,
            bid_lots: 100,
            inv: 0,
        };
        let full = mk.step_escrow(&sizes, &st, k).unwrap();
        let (bids, asks) = mk.step_lots(&sizes, &st);
        let price = |side, i| {
            mk.tiers
                .price(if side == Side::Long { st.bid } else { st.ask }, side, i, k)
        };
        let need = |side, i: usize, lots: u64| {
            lots * if side == Side::Long {
                price(side, i)
            } else {
                k - price(side, i)
            }
        };
        // Long inventory of the first tier's lots plus 1: the asks' first
        // tier and one lot of the second need nothing.
        let long = Step {
            inv: asks[0] as i64 + 1,
            ..st
        };
        assert_eq!(
            mk.step_escrow(&sizes, &long, k).unwrap(),
            full - need(Side::Short, 0, asks[0]) - need(Side::Short, 1, 1)
        );
        // Short inventory backs the bids the same way; inventory beyond
        // every lot backs them all.
        let short = Step {
            inv: -(bids[0] as i64),
            ..st
        };
        assert_eq!(
            mk.step_escrow(&sizes, &short, k).unwrap(),
            full - need(Side::Long, 0, bids[0])
        );
        let lots: i64 = asks.iter().sum::<u64>() as i64;
        let all = Step {
            inv: lots + 5,
            ..st
        };
        let bids_only: u64 = (0..3).map(|i| need(Side::Long, i, bids[i])).sum();
        assert_eq!(mk.step_escrow(&sizes, &all, k).unwrap(), bids_only);
    }

    #[test]
    fn netting_plans_every_overlap() {
        let claim = |side, lo, hi, lots, reserved| Claim {
            lots,
            side,
            lo,
            hi,
            reserved,
            ..Claim::default()
        };
        // A long 95-105 x 10 against shorts: 98-108 x 4, the exact 95-105 x 3
        // (first, though later in slot order), and 50-60 (no overlap).
        let opposite = [
            (7, claim(1, 98, 108, 4, 0)),
            (9, claim(1, 95, 105, 5, 2)),
            (3, claim(1, 50, 60, 9, 0)),
        ];
        let plan = plan_net(95, 105, 10, &opposite);
        let m = |claim, piece, lo, hi, lots| NetMatch {
            claim,
            piece,
            lo,
            hi,
            lots,
        };
        assert_eq!(
            plan,
            [m(9, (95, 105), 95, 105, 3), m(7, (95, 105), 98, 105, 4)]
        );
        // A short inside the fresh long: the parts outside it go on.
        let plan = plan_net(
            90,
            110,
            2,
            &[(1, claim(1, 95, 100, 5, 0)), (2, claim(1, 92, 108, 5, 0))],
        );
        assert_eq!(
            plan,
            [
                m(1, (90, 110), 95, 100, 2),
                m(2, (90, 95), 92, 95, 2),
                m(2, (100, 110), 100, 108, 2)
            ]
        );
        assert!(
            plan_net(90, 110, 2, &[(1, claim(1, 95, 100, 5, 5))]).is_empty(),
            "reserved lots stay"
        );
    }
}
