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
/// Optional signed credit assessment, appended after the existing arenas.
/// Existing accounts and every record offset remain unchanged.
pub const CREDIT_APPROVAL_LEN: usize = 16;
pub const CREDIT_APPROVAL_MAGIC: [u8; 8] = *b"AFCRISK1";
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

/// Staked AMBA behind the Earn vault's maker (2026-10-17; `docs/futures.md`,
/// "Staked AMBA backs the maker"): one [`StakePool`] per vault, a
/// [`StakePosition`] per staker.
pub const STAKE_SEED: &[u8] = b"stake";
pub const STAKE_POS_SEED: &[u8] = b"stake-pos";
pub const STAKE_MAGIC: [u8; 8] = *b"AFRSTAK1";
pub const STAKE_POS_MAGIC: [u8; 8] = *b"AFRSTKP1";
const STAKE_HEAD: usize = 512;
/// The stake pool remembers the ends of its last `EPOCH_ENDS` share epochs
/// (all its header has room for): a position that many rollovers behind is
/// still paid what it earned, and converted into the current epoch.
pub const EPOCH_ENDS: usize = 4;
/// A pool from before the epochs' conversion rates (2026-10-18): the head
/// and its AMBA subsidy stream ([`Rewards`]). It reads its rates as zero,
/// which is what its rollovers were (full wipe-outs), and grows to
/// `STAKE_LEN` at its next rollover (`Stake`) or `StartUnstake`.
pub const STAKE_V1_LEN: usize = STAKE_HEAD + REWARDS_LEN;
/// A pool from before the unstake timer (2026-10-18): the head, its stream,
/// then each kept epoch end's conversion rate ([`StakePool::epoch_rates`])
/// and the running share index ([`StakePool::share_index`]). It reads its
/// unstaking shares as none and grows to `STAKE_LEN` at its next
/// `StartUnstake` (or rollover).
pub const STAKE_V2_LEN: usize = STAKE_V1_LEN + 16 * EPOCH_ENDS + 16;
/// A pool from before it counted the markets holding its credit
/// (2026-10-18): as `STAKE_V2_LEN`, then the shares unstaking
/// ([`StakePool::unbonding`]). It reads its count as unknown
/// ([`StakePool::credit_counted`]) and grows to `STAKE_LEN` as a
/// `STAKE_V2_LEN` pool does.
pub const STAKE_V3_LEN: usize = STAKE_V2_LEN + 8;
/// The pool: as `STAKE_V3_LEN`, then the markets holding its credit
/// unsettled ([`StakePool::credit_markets`], u32) and whether that count is
/// complete (a byte, [`StakePool::credit_counted`]; three spare).
pub const STAKE_LEN: usize = STAKE_V3_LEN + 8;
/// How old the AMBA price may be before the pool lends nothing and a
/// shortfall's settlement waits for a fresh one; in the production build
/// also how long a new pool's prices run before it lends at all (the user,
/// 2026-10-18: one hour, as the averaging window).
pub const MAX_PRICE_AGE: u64 = 3_600;
/// [`StakePool::share_index`] before any rollover: one, 2^126 (stored as 0
/// in a pool or position from before any). So fine a scale (Codex's review
/// of the index, 2026-10-18; it was 10^18) that rounding the index to
/// nothing wipes out only what is worth less than an atom: a rollover that
/// zeroes an index of at least 2^64 has a rate under one over it, and an old
/// position holds under 2^64 shares. Only once the rates multiplied up fall
/// below 2^-62 (the index below 2^64) can that rounding take an atom.
pub const INDEX_ONE: u128 = 1 << 126;
/// A stake position from before the unstake timer: it reads its unstaking
/// shares as none and grows to `STAKE_POS_LEN` at its `StartUnstake`.
pub const STAKE_POS_V1_LEN: usize = 192;
/// A stake position: as `STAKE_POS_V1_LEN`, then its unstaking shares and
/// when their timer started ([`StakePosition::unbonding`], `unbond_ts`).
pub const STAKE_POS_LEN: usize = STAKE_POS_V1_LEN + 16;
/// How long unstaking takes: `Unstake` pays a position's unstaking shares
/// this long after its last `StartUnstake` (the user, 2026-10-18: 7 days).
pub const UNBOND_SECS: u64 = 604_800;
/// Fixed point of a posted AMBA price: USDC atoms per whole AMBA, times this.
pub const PRICE_SCALE: u128 = 1_000_000;
pub const BPS: u64 = 10_000;
/// Fixed point of [`StakePool::usdc_acc`]: USDC atoms per share, times this.
/// Shares are AMBA atoms (nine decimals), so a finer scale than the Earn
/// stream's `REWARD_SCALE` keeps the stakers' USDC exact to the atom.
pub const USDC_SCALE: u128 = 1_000_000_000_000_000_000;
/// Fixed point of the credit weight (credit's time-weighted share of the
/// maker's capacity in a market).
pub const WEIGHT_SCALE: u128 = 1_000_000_000;
/// `Header::credit_flags`: the market's credit is settled (`CreditSettle`).
pub const CREDIT_SETTLED: u8 = 1;
/// ... and its winners are paid part in AMBA (`SettleRecovery`).
pub const CREDIT_SHORTFALL: u8 = 2;
/// ... the market is in its pool's [`StakePool::credit_markets`]: set by the
/// `CreditFund` that counts it, cleared by its `CreditSettle`.
pub const CREDIT_COUNTED: u8 = 4;

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

/// The stake pool behind an Earn vault's maker.
pub fn stake_address(program_id: &Pubkey, vault: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[NAMESPACE, STAKE_SEED, vault.as_ref()], program_id)
}

/// One staker's shares in a stake pool.
pub fn stake_position_address(program_id: &Pubkey, pool: &Pubkey, owner: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[NAMESPACE, STAKE_POS_SEED, pool.as_ref(), owner.as_ref()],
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
    // Stake-backed credit (2026-10-17; `docs/futures.md`, "Staked AMBA backs
    // the maker"). All zero in a market that never had any.
    /// The one trader that may borrow here: an Earn vault's address, whose
    /// stake pool lent it `credit_limit` (default: nobody).
    pub credit_owner: Pubkey,
    /// The most the credit owner's position may cost its stakers here, at
    /// worst ([`credit_exposure`]).
    pub credit_limit: u64,
    /// Cash the credit owner borrowed that custody doesn't hold: custody +
    /// debt = free + reserved + backing. After a shortfall, what the winners'
    /// payouts still give up.
    pub debt: u64,
    /// The credit owner borrows nothing from here on: the expiry less its
    /// pool's window (no new credit-backed exposure near expiry: the user,
    /// 2026-10-17).
    pub credit_until: u64,
    /// Cash the vault moved in (`VaultFund`) and out (its draws), for its
    /// profit here.
    pub funded_in: u64,
    pub funded_out: u64,
    /// Time-weighted credit limit and net funding (atom-seconds), up to
    /// `tw_ts`; they stop at expiry. Credit's share of their sum weighs the
    /// stakers' part of the profit.
    pub tw_ts: u64,
    pub tw_credit: u128,
    pub tw_cash: u128,
    /// The shortfall the stakers' USDC buffer did not cover (`CreditSettle`),
    /// the AMBA set aside for it and the winners' payouts it is spread over.
    pub shortfall: u64,
    pub recovery_amba: u64,
    pub winners: u64,
    /// `CREDIT_SETTLED`, `CREDIT_SHORTFALL`.
    pub credit_flags: u8,
    /// The stakers' share of the credit-weighted profit, from the pool.
    pub profit_bps: u16,
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
            credit_owner: key_at(b, 376),
            credit_limit: u64_at(b, 408),
            debt: u64_at(b, 416),
            credit_until: u64_at(b, 424),
            funded_in: u64_at(b, 432),
            funded_out: u64_at(b, 440),
            tw_ts: u64_at(b, 448),
            tw_credit: u128_at(b, 456),
            tw_cash: u128_at(b, 472),
            shortfall: u64_at(b, 488),
            recovery_amba: u64_at(b, 496),
            winners: u64_at(b, 504),
            credit_flags: b[28],
            profit_bps: u16_at(b, 30),
        };
        let base = market_len(h.grid, h.trader_cap, h.order_cap, h.maker_cap, h.claim_cap);
        if b.len() != base
            && (b.len() != base + CREDIT_APPROVAL_LEN || b[base..base + 8] != CREDIT_APPROVAL_MAGIC)
        {
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
        b[28] = self.credit_flags;
        b[29] = 0;
        put16(b, 30, self.profit_bps);
        put_key(b, 376, &self.credit_owner);
        put64(b, 408, self.credit_limit);
        put64(b, 416, self.debt);
        put64(b, 424, self.credit_until);
        put64(b, 432, self.funded_in);
        put64(b, 440, self.funded_out);
        put64(b, 448, self.tw_ts);
        put128(b, 456, self.tw_credit);
        put128(b, 472, self.tw_cash);
        put64(b, 488, self.shortfall);
        put64(b, 496, self.recovery_amba);
        put64(b, 504, self.winners);
    }

    /// A stake pool lent this market credit.
    pub fn has_credit(&self) -> bool {
        self.credit_owner != Pubkey::default()
    }

    pub fn credit_settled(&self) -> bool {
        self.credit_flags & CREDIT_SETTLED != 0
    }

    /// The market holds credit of the pool behind `vault`, unsettled, and is
    /// in that pool's [`StakePool::credit_markets`]: one of the markets
    /// `Unstake` takes.
    pub fn counts_credit_of(&self, vault: &Pubkey) -> bool {
        self.credit_owner == *vault
            && !self.credit_settled()
            && self.credit_flags & CREDIT_COUNTED != 0
    }

    /// Whether unstaking waits for this market at `now`, given its pool's
    /// `expiry_window`: from the earlier of its `credit_until` and its expiry
    /// less that window (its last day on dev) until its credit settles
    /// (Codex's review of the unstake timer, 2026-10-18: a shortfall there
    /// may be certain by then, and an unstake would leave it to the rest).
    pub fn holds_unstaking(&self, expiry_window: u64, now: u64) -> bool {
        let from = self
            .credit_until
            .min(self.expiry_ts.saturating_sub(expiry_window));
        !self.credit_settled() && now >= from
    }

    /// The vault's net funding here (it may have taken out more than it put
    /// in).
    pub fn funded(&self) -> i128 {
        i128::from(self.funded_in) - i128::from(self.funded_out)
    }

    /// Brings the time-weighted credit and funding up to `now` (they stop at
    /// expiry); the first call only starts the clock. `None` on overflow.
    pub fn credit_clock(&mut self, now: u64) -> Option<()> {
        let t = now.min(self.expiry_ts);
        if self.tw_ts == 0 {
            self.tw_ts = t;
            return Some(());
        }
        if t > self.tw_ts {
            let dt = u128::from(t - self.tw_ts);
            let credit = u128::from(self.credit_limit).checked_mul(dt)?;
            let cash = u128::try_from(self.funded().max(0)).ok()?.checked_mul(dt)?;
            self.tw_credit = self.tw_credit.checked_add(credit)?;
            self.tw_cash = self.tw_cash.checked_add(cash)?;
            self.tw_ts = t;
        }
        Some(())
    }

    /// Credit's time-weighted share of the maker's capacity here (credit
    /// plus net funding), times `WEIGHT_SCALE`.
    pub fn credit_weight(&self) -> u128 {
        let total = self.tw_credit.saturating_add(self.tw_cash);
        if total == 0 {
            return 0;
        }
        self.tw_credit.saturating_mul(WEIGHT_SCALE) / total
    }

    /// The stakers' part of a vault equity of `equity` here: `profit_bps` of
    /// its profit over the net funding, times the credit weight (as of the
    /// clock; the user, 2026-10-17: 50% of the credit-weighted profit).
    /// [`Header::staker_share`] at full credit weight: the most the stakers'
    /// share of `equity` can become by expiry (once the vault's funding
    /// falls, credit's weight only rises).
    pub fn staker_share_max(&self, equity: u64) -> u64 {
        let profit = i128::from(equity) - self.funded();
        if profit <= 0 {
            return 0;
        }
        let part = profit as u128 * u128::from(self.profit_bps) / u128::from(BPS);
        u64::try_from(part).unwrap_or(u64::MAX).min(equity)
    }

    pub fn staker_share(&self, equity: u64) -> u64 {
        let profit = i128::from(equity) - self.funded();
        if profit <= 0 {
            return 0;
        }
        let part = profit as u128 * u128::from(self.profit_bps) / u128::from(BPS);
        let share = part.saturating_mul(self.credit_weight()) / WEIGHT_SCALE;
        u64::try_from(share).unwrap_or(u64::MAX).min(equity)
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
    /// The claim a close order reserves. For an opening order, the claim
    /// slot it holds for its fills ([`Claim::held`]); `NO_CLAIM` for none
    /// (an order placed before 2026-10-18 takes hold at its first fill).
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
    /// Held by a resting opening order of the same owner, side and range
    /// (its `claim`): the order's fills join this claim, and the slot stays
    /// the order's while it rests, as an empty placeholder (0 lots) when the
    /// claim is gone, so a fill never waits for a free slot
    /// (`docs/futures.md`, "Claim slots").
    pub held: bool,
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
            held: b[at + 19] != 0,
        }
    }
    pub fn write(&self, b: &mut [u8], at: usize) {
        put64(b, at, self.lots);
        put64(b, at + 8, self.seq);
        put16(b, at + 16, self.trader);
        b[at + 18] = self.side;
        b[at + 19] = u8::from(self.held);
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
        // One bounds check for the record (a credit check reads every step).
        let w: &[u8; STEP_LEN] = b[at..at + STEP_LEN].try_into().unwrap();
        let word = |i: usize| -> [u8; 8] { w[i..i + 8].try_into().unwrap() };
        let half = |i: usize| -> [u8; 4] { w[i..i + 4].try_into().unwrap() };
        Step {
            ask: u32::from_le_bytes(half(0)),
            bid: u32::from_le_bytes(half(4)),
            ask_lots: u64::from_le_bytes(word(8)),
            bid_lots: u64::from_le_bytes(word(16)),
            inv: i64::from_le_bytes(word(24)),
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

/// The stake pool behind an Earn vault's maker (`STAKE_LEN` bytes): stakers'
/// AMBA in `amba_account`, which backs the credit the operator lends the
/// vault's trader in its markets (`Header::credit_limit`, summed in
/// `allocated`), and their USDC (the buffer and what they are owed) in
/// `usdc_account`. Both accounts are owned by the pool's address; the pool
/// counts their balances itself, so gifts are not counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StakePool {
    pub bump: u8,
    pub amba_decimals: u8,
    /// What the credit capacity counts the staked AMBA at: this share of its
    /// average price (the user, 2026-10-17: a 50% haircut).
    pub haircut_bps: u16,
    /// The stakers' share of the maker's credit-weighted profit in each
    /// market (the user, 2026-10-17: 50%).
    pub profit_bps: u16,
    /// The buffer's target: this share of the allocated credit. The stakers'
    /// USDC fills it before it is paid out, and it pays a shortfall before
    /// any AMBA does.
    pub buffer_bps: u16,
    pub vault: Pubkey,
    pub operator: Pubkey,
    pub amba_mint: Pubkey,
    pub amba_account: Pubkey,
    pub usdc_account: Pubkey,
    pub usdc_mint: Pubkey,
    /// AMBA atoms staked (less what shortfalls took).
    pub staked: u64,
    pub shares: u64,
    /// Credit lent to markets, summed.
    pub allocated: u64,
    /// USDC atoms in the buffer.
    pub buffer: u64,
    /// AMBA atoms set aside for shortfalls' winners, not paid yet.
    pub recovery_owed: u64,
    /// The last posted price (USDC atoms per whole AMBA, times
    /// `PRICE_SCALE`), when, and the average as of then.
    pub price: u64,
    pub price_ts: u64,
    pub twap: u64,
    /// Seconds the average takes to reach a posted price.
    pub twap_window: u64,
    /// Seconds before a market's expiry from which its maker borrows nothing
    /// new (`Header::credit_until`).
    pub expiry_window: u64,
    /// The stakers' USDC: atoms per share times `USDC_SCALE`, ever
    /// distributed, ever paid.
    pub usdc_acc: u128,
    pub usdc_distributed: u64,
    pub usdc_paid: u64,
    /// Informational: USDC ever received from the maker's profit, buffer
    /// ever paid into shortfalls, AMBA ever set aside for them, AMBA ever
    /// staked and unstaked.
    pub profit_in: u64,
    pub covered: u64,
    pub slashed: u64,
    pub staked_in: u64,
    pub unstaked: u64,
    pub created_ts: u64,
    /// Share epoch: the next stake starts a new epoch at one share per atom
    /// when a shortfall took every staked atom (the review of 2026-10-18,
    /// finding 9) or the shares for it wouldn't fit a `u64` (a near-total
    /// wipe-out: a few atoms left against many shares; its re-review). A
    /// position of an old epoch keeps what it earned up to its end, then its
    /// shares convert at that end's rate (`epoch_rates`).
    pub epoch: u32,
    /// `(AMBA stream acc, usdc_acc)` at the end of epoch `epoch - 1 - i`.
    pub epoch_ends: [(u128, u128); EPOCH_ENDS],
    /// `(staked, shares)` at the end of epoch `epoch - 1 - i`: its `shares`
    /// held `staked` atoms, and the next epoch starts with one share per
    /// atom of them, so a share of it is `staked / shares` of the next's.
    /// Zero in a `STAKE_V1_LEN` pool (its rollovers left nothing to
    /// convert).
    pub epoch_rates: [(u64, u64); EPOCH_ENDS],
    /// Every rollover's rate multiplied up since the last full wipe-out,
    /// times `INDEX_ONE` (2^126), each product rounded down: a share of an
    /// epoch older than the kept ends is `share_index` / its epoch's index
    /// of a share now, so its principal converts however far back it is
    /// (Codex's review of the conversion, 2026-10-18). Only what it earned
    /// there and never collected is lost.
    /// 0 reads as `INDEX_ONE` ([`StakePool::index`]): a pool before any.
    pub share_index: u128,
    /// The epoch the last full wipe-out (or an index too small to keep)
    /// started: shares of earlier epochs are worth nothing.
    pub wipe_epoch: u32,
    /// When the first price was posted (0 in a pool from before: taken as
    /// long ago).
    pub first_price_ts: u64,
    /// Shares unstaking (`StartUnstake`), summed over the positions: they
    /// stay in `shares`, earn and bear shortfalls like any, but back no new
    /// credit ([`StakePool::capacity`]). Zero in a pool from before the
    /// timer (`STAKE_V1_LEN`, `STAKE_V2_LEN`).
    pub unbonding: u64,
    /// The markets holding this pool's credit unsettled that it counts
    /// (`Header::credit_flags` has `CREDIT_COUNTED`): the first `CreditFund`
    /// in a market counts it, its `CreditSettle` uncounts it. `Unstake`
    /// takes every one of them, so none is in its last `expiry_window` or
    /// past expiry unsettled (Codex's review of the unstake timer,
    /// 2026-10-18). Zero in a pool from before (`STAKE_V3_LEN` or shorter).
    pub credit_markets: u32,
    /// Whether `credit_markets` covers every market holding this pool's
    /// credit: true from `CreateStakePool`, false in a pool from before
    /// (shorter than `STAKE_LEN`, it may have lent in markets it never
    /// counted) until the credit lent returns to 0 ([`StakePool::counted`];
    /// written as true from then on).
    pub credit_counted: bool,
    /// The AMBA subsidy stream to stakers, once the operator sets one.
    pub rewards: Option<Rewards>,
}

impl StakePool {
    pub fn read(b: &[u8]) -> Result<Self, ProgramError> {
        if !matches!(
            b.len(),
            STAKE_V1_LEN | STAKE_V2_LEN | STAKE_V3_LEN | STAKE_LEN
        ) || b[..8] != STAKE_MAGIC
        {
            return Err(FuturesError::InvalidAccount.into());
        }
        let rates = b.len() >= STAKE_V2_LEN;
        let r = &b[STAKE_HEAD..STAKE_V1_LEN];
        let rewards = if r[..8] == REWARDS_MAGIC {
            Some(Rewards::read(r)?)
        } else {
            None
        };
        Ok(StakePool {
            bump: b[8],
            amba_decimals: b[9],
            haircut_bps: u16_at(b, 10),
            profit_bps: u16_at(b, 12),
            buffer_bps: u16_at(b, 14),
            vault: key_at(b, 16),
            operator: key_at(b, 48),
            amba_mint: key_at(b, 80),
            amba_account: key_at(b, 112),
            usdc_account: key_at(b, 144),
            usdc_mint: key_at(b, 176),
            staked: u64_at(b, 208),
            shares: u64_at(b, 216),
            allocated: u64_at(b, 224),
            buffer: u64_at(b, 232),
            recovery_owed: u64_at(b, 240),
            price: u64_at(b, 248),
            price_ts: u64_at(b, 256),
            twap: u64_at(b, 264),
            twap_window: u64_at(b, 272),
            expiry_window: u64_at(b, 280),
            usdc_acc: u128_at(b, 288),
            usdc_distributed: u64_at(b, 304),
            usdc_paid: u64_at(b, 312),
            profit_in: u64_at(b, 320),
            covered: u64_at(b, 328),
            slashed: u64_at(b, 336),
            staked_in: u64_at(b, 344),
            unstaked: u64_at(b, 352),
            created_ts: u64_at(b, 360),
            epoch: u32_at(b, 368),
            epoch_ends: core::array::from_fn(|i| {
                (u128_at(b, 384 + 32 * i), u128_at(b, 400 + 32 * i))
            }),
            epoch_rates: core::array::from_fn(|i| match rates {
                true => (
                    u64_at(b, STAKE_V1_LEN + 16 * i),
                    u64_at(b, STAKE_V1_LEN + 8 + 16 * i),
                ),
                false => (0, 0),
            }),
            share_index: match rates {
                true => u128_at(b, STAKE_V1_LEN + 16 * EPOCH_ENDS),
                false => 0,
            },
            // A pool from before the rates rolled over only at full
            // wipe-outs: its epoch is its last wipe-out's.
            wipe_epoch: match rates {
                true => u32_at(b, 372),
                false => u32_at(b, 368),
            },
            first_price_ts: u64_at(b, 376),
            unbonding: match b.len() >= STAKE_V3_LEN {
                true => u64_at(b, STAKE_V2_LEN),
                false => 0,
            },
            credit_markets: match b.len() {
                STAKE_LEN => u32_at(b, STAKE_V3_LEN),
                _ => 0,
            },
            credit_counted: b.len() == STAKE_LEN && b[STAKE_V3_LEN + 4] != 0,
            rewards,
        })
    }

    /// `b` is the pool's account data, of any of its lengths: a
    /// `STAKE_V1_LEN` one gets no rates (the caller grows it before a
    /// rollover), a shorter one than `STAKE_V3_LEN` no unstaking shares (the
    /// caller grows it before `StartUnstake`), a shorter one than
    /// `STAKE_LEN` no count of its credit markets (`CreditFund` counts only
    /// in a pool that holds it).
    pub fn write(&self, b: &mut [u8]) {
        b[..8].copy_from_slice(&STAKE_MAGIC);
        b[8] = self.bump;
        b[9] = self.amba_decimals;
        put16(b, 10, self.haircut_bps);
        put16(b, 12, self.profit_bps);
        put16(b, 14, self.buffer_bps);
        put_key(b, 16, &self.vault);
        put_key(b, 48, &self.operator);
        put_key(b, 80, &self.amba_mint);
        put_key(b, 112, &self.amba_account);
        put_key(b, 144, &self.usdc_account);
        put_key(b, 176, &self.usdc_mint);
        put64(b, 208, self.staked);
        put64(b, 216, self.shares);
        put64(b, 224, self.allocated);
        put64(b, 232, self.buffer);
        put64(b, 240, self.recovery_owed);
        put64(b, 248, self.price);
        put64(b, 256, self.price_ts);
        put64(b, 264, self.twap);
        put64(b, 272, self.twap_window);
        put64(b, 280, self.expiry_window);
        put128(b, 288, self.usdc_acc);
        put64(b, 304, self.usdc_distributed);
        put64(b, 312, self.usdc_paid);
        put64(b, 320, self.profit_in);
        put64(b, 328, self.covered);
        put64(b, 336, self.slashed);
        put64(b, 344, self.staked_in);
        put64(b, 352, self.unstaked);
        put64(b, 360, self.created_ts);
        b[368..STAKE_HEAD].fill(0);
        put32(b, 368, self.epoch);
        put32(b, 372, self.wipe_epoch);
        put64(b, 376, self.first_price_ts);
        for (i, (reward, usdc)) in self.epoch_ends.iter().enumerate() {
            put128(b, 384 + 32 * i, *reward);
            put128(b, 400 + 32 * i, *usdc);
        }
        match &self.rewards {
            Some(r) => r.write(&mut b[STAKE_HEAD..STAKE_V1_LEN]),
            None => b[STAKE_HEAD..STAKE_V1_LEN].fill(0),
        }
        if let Some(at) = b.get_mut(STAKE_V1_LEN..STAKE_V2_LEN) {
            for (i, (staked, shares)) in self.epoch_rates.iter().enumerate() {
                put64(at, 16 * i, *staked);
                put64(at, 8 + 16 * i, *shares);
            }
            put128(at, 16 * EPOCH_ENDS, self.share_index);
        }
        if let Some(at) = b.get_mut(STAKE_V2_LEN..STAKE_V3_LEN) {
            put64(at, 0, self.unbonding);
        }
        if let Some(at) = b.get_mut(STAKE_V3_LEN..STAKE_LEN) {
            put32(at, 0, self.credit_markets);
            at[4..].fill(0);
            at[4] = u8::from(self.counted());
        }
    }

    /// Whether [`StakePool::credit_markets`] counts every market holding
    /// this pool's credit unsettled: always in a pool created with the
    /// count, and in one from before once no credit is lent (every market
    /// that has its credit and isn't counted then has a zero limit, which no
    /// shortfall can reach; a later `CreditFund` there counts it).
    pub fn counted(&self) -> bool {
        self.credit_counted || self.allocated == 0
    }

    /// The time-weighted average of the posted prices at `now`: it moves
    /// from its value at the last post toward that post linearly, reaching it
    /// a full `twap_window` later (0: before any post).
    pub fn twap_at(&self, now: u64) -> u64 {
        if self.price_ts == 0 {
            return 0;
        }
        let dt = now.saturating_sub(self.price_ts);
        if self.twap_window == 0 || dt >= self.twap_window {
            return self.price;
        }
        let (a, p) = (i128::from(self.twap), i128::from(self.price));
        (a + (p - a) * i128::from(dt) / i128::from(self.twap_window)) as u64
    }

    /// Posts `price` at `now`: the average so far becomes its start.
    pub fn post(&mut self, price: u64, now: u64) {
        self.twap = if self.price_ts == 0 {
            self.first_price_ts = now;
            price
        } else {
            self.twap_at(now)
        };
        self.price = price;
        self.price_ts = now;
    }

    fn unit(&self) -> u128 {
        PRICE_SCALE * 10u128.pow(u32::from(self.amba_decimals))
    }

    /// `amba` atoms in USDC atoms at `price`, rounded down. `None` on
    /// overflow.
    pub fn usdc_of(&self, amba: u64, price: u64) -> Option<u64> {
        let v = u128::from(amba).checked_mul(u128::from(price))? / self.unit();
        u64::try_from(v).ok()
    }

    /// AMBA atoms worth `usdc` atoms at `price` (> 0), rounded up.
    pub fn amba_for(&self, usdc: u64, price: u64) -> Option<u64> {
        if price == 0 {
            return None;
        }
        let v = u128::from(usdc)
            .checked_mul(self.unit())?
            .div_ceil(u128::from(price));
        u64::try_from(v).ok()
    }

    /// As [`StakePool::amba_for`], at most `cap`: a price too low to buy
    /// `usdc` with every atom asks for every atom, never an overflow (the
    /// review of 2026-10-18, finding 8).
    pub fn amba_for_at_most(&self, usdc: u64, price: u64, cap: u64) -> u64 {
        if price == 0 {
            return cap;
        }
        u128::from(usdc)
            .checked_mul(self.unit())
            .map_or(u128::MAX, |v| v.div_ceil(u128::from(price)))
            .min(u128::from(cap)) as u64
    }

    /// Whether a stake of `received` atoms starts a new share epoch: a
    /// shortfall took every staked atom while shares are out, or the shares
    /// for it (in `u128`) don't fit a `u64`, alone or added to the pool's.
    pub fn rolls_over(&self, received: u64) -> bool {
        (self.staked == 0 && self.shares > 0)
            || self
                .shares_for(received)
                .and_then(|s| self.shares.checked_add(s))
                .is_none()
    }

    /// [`StakePool::share_index`], 0 read as one.
    pub fn index(&self) -> u128 {
        match self.share_index {
            0 => INDEX_ONE,
            v => v,
        }
    }

    /// Starts a new share epoch (the stream accrued to the old shares
    /// first): the `staked` atoms left become as many shares, one per atom,
    /// and the old `shares` convert into them as positions are touched.
    pub fn begin_epoch(&mut self, now: u64) -> Option<()> {
        let shares = self.shares;
        if let Some(r) = self.rewards.as_mut() {
            r.accrue(shares, now)?;
        }
        self.epoch_ends.copy_within(..EPOCH_ENDS - 1, 1);
        self.epoch_ends[0] = (self.rewards.map_or(0, |r| r.acc), self.usdc_acc);
        self.epoch_rates.copy_within(..EPOCH_ENDS - 1, 1);
        self.epoch_rates[0] = (self.staked, self.shares);
        // The unstaking shares convert as every share does.
        self.unbonding = convert_shares(self.unbonding, (self.staked, self.shares));
        self.epoch = self.epoch.checked_add(1)?;
        // The running index: times this rollover's rate (in 256 bits,
        // rounded down; at most one, as a pool never holds more atoms than
        // shares), restarted at one when nothing is left to convert (a full
        // wipe-out, or a rate so small the index rounds to nothing).
        let next = match self.shares {
            0 => self.index(),
            n => mul_div(self.index(), u128::from(self.staked.min(n)), u128::from(n))?,
        };
        if next == 0 {
            self.share_index = INDEX_ONE;
            self.wipe_epoch = self.epoch;
        } else {
            self.share_index = next;
        }
        self.shares = self.staked;
        Some(())
    }

    /// The credit `staked` AMBA atoms back at `now`: their value at the
    /// average price, haircut.
    pub fn capacity_of(&self, staked: u64, now: u64) -> Option<u64> {
        if !self.price_lends(now) {
            return Some(0);
        }
        let value = self.usdc_of(staked, self.twap_at(now))?;
        u64::try_from(u128::from(value) * u128::from(self.haircut_bps) / u128::from(BPS)).ok()
    }

    /// Whether the last price is at most `MAX_PRICE_AGE` old at `now`.
    pub fn price_fresh(&self, now: u64) -> bool {
        self.price_ts != 0 && now.saturating_sub(self.price_ts) <= MAX_PRICE_AGE
    }

    /// Whether the price may back credit at `now`: fresh, and in the
    /// production build (no operator posting) a pool's prices have run for
    /// `MAX_PRICE_AGE`, so no first sample lends alone (Codex's review of
    /// `PoolPrice`, 2026-10-18). A stale price lends nothing: no new credit,
    /// and no unstaking while credit is lent.
    pub fn price_lends(&self, now: u64) -> bool {
        #[cfg(not(feature = "operator-price"))]
        if self.first_price_ts != 0 && now.saturating_sub(self.first_price_ts) < MAX_PRICE_AGE {
            return false;
        }
        self.price_fresh(now)
    }

    /// The credit the pool backs at `now`: its staked AMBA less what its
    /// unstaking shares hold ([`StakePool::unbonding`]), so the maker lends
    /// nothing against them and hands back what they backed. (Credit
    /// already lent keeps all of the staked AMBA behind it: `Unstake` checks
    /// [`StakePool::capacity_of`] what stays.)
    pub fn capacity(&self, now: u64) -> Option<u64> {
        let unbonding = self.amba_of(self.unbonding.min(self.shares));
        self.capacity_of(self.staked.saturating_sub(unbonding), now)
    }

    /// When `p`'s unstaking shares may leave (`Unstake`): `UNBOND_SECS`
    /// after its last `StartUnstake`; `None` with nothing unstaking.
    pub fn unbond_ends(&self, p: &StakePosition) -> Option<u64> {
        (p.unbonding > 0).then(|| p.unbond_ts.saturating_add(UNBOND_SECS))
    }

    /// Shares for `received` AMBA atoms at the pool's AMBA per share (one
    /// virtual share and atom keep a pool that shortfalls emptied priced).
    pub fn shares_for(&self, received: u64) -> Option<u64> {
        let v =
            u128::from(received) * (u128::from(self.shares) + 1) / (u128::from(self.staked) + 1);
        u64::try_from(v).ok()
    }

    /// AMBA atoms `shares` are worth, rounded down.
    pub fn amba_of(&self, shares: u64) -> u64 {
        if self.shares == 0 {
            return 0;
        }
        (u128::from(shares) * u128::from(self.staked) / u128::from(self.shares)) as u64
    }

    fn buffer_target(&self) -> u64 {
        (u128::from(self.allocated) * u128::from(self.buffer_bps) / u128::from(BPS)) as u64
    }

    /// Spreads `amount` USDC atoms over the shares (no shares: into the
    /// buffer). `None` on overflow.
    pub fn spread(&mut self, amount: u64) -> Option<()> {
        if amount == 0 {
            return Some(());
        }
        if self.shares == 0 {
            self.buffer = self.buffer.checked_add(amount)?;
            return Some(());
        }
        let per = u128::from(amount).checked_mul(USDC_SCALE)? / u128::from(self.shares);
        self.usdc_acc = self.usdc_acc.checked_add(per)?;
        self.usdc_distributed = self.usdc_distributed.checked_add(amount)?;
        Some(())
    }

    /// The stakers' share of a market's profit: the buffer first, up to its
    /// target, then every share.
    pub fn take_profit(&mut self, amount: u64) -> Option<()> {
        let fill = amount.min(self.buffer_target().saturating_sub(self.buffer));
        self.buffer = self.buffer.checked_add(fill)?;
        self.spread(amount - fill)
    }

    /// Pays the buffer above its target out to the shares (after the
    /// allocated credit falls).
    pub fn release_buffer(&mut self) -> Option<()> {
        let excess = self.buffer.saturating_sub(self.buffer_target());
        if excess == 0 || self.shares == 0 {
            return Some(());
        }
        self.buffer -= excess;
        self.spread(excess)
    }

    /// USDC the stakers are owed and not paid yet.
    pub fn usdc_owed(&self) -> u64 {
        self.usdc_distributed.saturating_sub(self.usdc_paid)
    }

    /// Accrues the AMBA stream to `now` and settles `p`'s AMBA and USDC
    /// (before its shares change, and before a payment). `None` on
    /// overflow.
    pub fn touch(&mut self, p: &mut StakePosition, now: u64) -> Option<()> {
        let shares = self.shares;
        if let Some(r) = self.rewards.as_mut() {
            r.accrue(shares, now)?;
        }
        let reward_acc = self.rewards.map_or(0, |r| r.acc);
        while p.epoch != self.epoch {
            // A position of an earlier epoch: what its shares earned up to
            // its epoch's end (where the next one's accumulators start),
            // then its shares converted at that end's rate, rounded down;
            // so on to the current epoch. Only an epoch more than
            // `EPOCH_ENDS` back has no end kept: its position (untouched
            // through that many rollovers) earns nothing more.
            let back = self.epoch.checked_sub(p.epoch)? as usize;
            let i = back.checked_sub(1)?;
            match (self.epoch_ends.get(i), self.epoch_rates.get(i)) {
                (Some(&(reward, usdc)), Some(&rate)) => {
                    p.settle(reward, usdc)?;
                    p.shares = convert_shares(p.shares, rate);
                    p.unbonding = convert_shares(p.unbonding, rate).min(p.shares);
                    p.epoch = p.epoch.checked_add(1)?;
                }
                _ => {
                    // Older than the kept ends: its principal converts by
                    // the running index (in 256 bits, rounded down; none
                    // before the last wipe-out); what it earned there and
                    // never collected is lost.
                    let (wiped, index) = (p.epoch < self.wipe_epoch, self.index());
                    let from = if p.index == 0 { INDEX_ONE } else { p.index };
                    let convert = |shares: u64| -> Option<u64> {
                        if wiped {
                            return Some(0);
                        }
                        let v = mul_div(u128::from(shares), index, from)?;
                        Some(v.min(u128::from(shares)) as u64)
                    };
                    p.shares = convert(p.shares)?;
                    p.unbonding = convert(p.unbonding)?.min(p.shares);
                    p.epoch = self.epoch;
                    p.reward_snapshot = reward_acc;
                    p.usdc_snapshot = self.usdc_acc;
                }
            }
            p.index = self.share_index;
        }
        p.settle(reward_acc, self.usdc_acc)
    }
}

/// `a * b / c`, rounded down, the product held in 256 bits (64-bit limbs):
/// `None` when `c` is 0 or the quotient doesn't fit a `u128`. The share
/// index's arithmetic (`INDEX_ONE` = 2^126 times a `u64` overflows `u128`).
fn mul_div(a: u128, b: u128, c: u128) -> Option<u128> {
    if c == 0 {
        return None;
    }
    if let Some(v) = a.checked_mul(b) {
        return Some(v / c);
    }
    let (hi, lo) = mul_wide(a, b);
    if hi >= c {
        return None;
    }
    let mut rem = hi;
    let mut q = 0u128;
    if c <= u128::from(u64::MAX) {
        // A divisor of one limb: a limb at a time, each partial dividend
        // (the remainder, below `c`, then the limb) within a `u128`.
        for limb in [lo >> 64, lo & u128::from(u64::MAX)] {
            let n = (rem << 64) | limb;
            q = (q << 64) | (n / c);
            rem = n % c;
        }
        return Some(q);
    }
    // A bit at a time through the low half, the remainder kept below `c`
    // (doubled it may pass 2^128: the carry).
    for i in (0..128).rev() {
        let carry = rem >> 127;
        rem = (rem << 1) | ((lo >> i) & 1);
        q <<= 1;
        if carry == 1 || rem >= c {
            rem = rem.wrapping_sub(c);
            q |= 1;
        }
    }
    Some(q)
}

/// `a * b` in 256 bits: `(high, low)` halves.
fn mul_wide(a: u128, b: u128) -> (u128, u128) {
    let m = u128::from(u64::MAX);
    let (a1, a0, b1, b0) = (a >> 64, a & m, b >> 64, b & m);
    let (p00, p01, p10, p11) = (a0 * b0, a0 * b1, a1 * b0, a1 * b1);
    // Below 3 * 2^64: no overflow.
    let mid = (p00 >> 64) + (p01 & m) + (p10 & m);
    (
        p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64),
        (mid << 64) | (p00 & m),
    )
}

/// `shares` of an ended epoch in the next one's: `shares * staked /
/// total`, rounded down, at its end's rate `(staked, total)` (none at a
/// full wipe-out). The rounding leaves less than a share per position
/// unowned in the pool (`docs/futures.md`, Epochs).
fn convert_shares(shares: u64, (staked, total): (u64, u64)) -> u64 {
    if total == 0 {
        return 0;
    }
    // At most `shares`: a pool never holds more atoms than shares.
    (u128::from(shares) * u128::from(staked) / u128::from(total)).min(u128::from(shares)) as u64
}

/// A staker's shares in a stake pool (`STAKE_POS_LEN` bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StakePosition {
    pub bump: u8,
    pub pool: Pubkey,
    pub owner: Pubkey,
    pub shares: u64,
    /// AMBA atoms this owner ever staked (as received) and unstaked
    /// (informational).
    pub staked_in: u64,
    pub unstaked: u64,
    /// The AMBA stream: the pool's `Rewards::acc` at the last settlement,
    /// settled and unpaid, ever paid.
    pub reward_snapshot: u128,
    pub reward_pending: u64,
    pub reward_paid: u64,
    /// The stakers' USDC: likewise against `StakePool::usdc_acc`.
    pub usdc_snapshot: u128,
    pub usdc_pending: u64,
    pub usdc_paid: u64,
    /// The pool's share epoch these shares are of.
    pub epoch: u32,
    /// The pool's `share_index` in that epoch (0: one, as before any).
    pub index: u128,
    /// Of `shares`, those unstaking (`StartUnstake`): they earn and bear
    /// shortfalls like the rest, and `Unstake` pays them `UNBOND_SECS`
    /// after `unbond_ts`, the last `StartUnstake` (adding more restarts the
    /// timer for all of them). Zero in a `STAKE_POS_V1_LEN` position.
    pub unbonding: u64,
    pub unbond_ts: u64,
}

impl StakePosition {
    /// What the shares earned up to the accumulators `reward_acc` and
    /// `usdc_acc` into pending, snapshots moved there. `None` on overflow.
    fn settle(&mut self, reward_acc: u128, usdc_acc: u128) -> Option<()> {
        let per = reward_acc.checked_sub(self.reward_snapshot)?;
        let earned = u128::from(self.shares).checked_mul(per)? / REWARD_SCALE;
        self.reward_pending = self
            .reward_pending
            .checked_add(u64::try_from(earned).ok()?)?;
        self.reward_snapshot = reward_acc;
        let per = usdc_acc.checked_sub(self.usdc_snapshot)?;
        let earned = u128::from(self.shares).checked_mul(per)? / USDC_SCALE;
        self.usdc_pending = self.usdc_pending.checked_add(u64::try_from(earned).ok()?)?;
        self.usdc_snapshot = usdc_acc;
        Some(())
    }

    pub fn read(b: &[u8]) -> Result<Self, ProgramError> {
        if !matches!(b.len(), STAKE_POS_V1_LEN | STAKE_POS_LEN) || b[..8] != STAKE_POS_MAGIC {
            return Err(FuturesError::InvalidAccount.into());
        }
        let timer = b.len() == STAKE_POS_LEN;
        Ok(StakePosition {
            bump: b[8],
            pool: key_at(b, 16),
            owner: key_at(b, 48),
            shares: u64_at(b, 80),
            staked_in: u64_at(b, 88),
            unstaked: u64_at(b, 96),
            reward_snapshot: u128_at(b, 104),
            reward_pending: u64_at(b, 120),
            reward_paid: u64_at(b, 128),
            usdc_snapshot: u128_at(b, 136),
            usdc_pending: u64_at(b, 152),
            usdc_paid: u64_at(b, 160),
            epoch: u32_at(b, 168),
            index: u128_at(b, 176),
            unbonding: if timer { u64_at(b, 192) } else { 0 },
            unbond_ts: if timer { u64_at(b, 200) } else { 0 },
        })
    }

    pub fn write(&self, b: &mut [u8]) {
        b.fill(0);
        b[..8].copy_from_slice(&STAKE_POS_MAGIC);
        b[8] = self.bump;
        put_key(b, 16, &self.pool);
        put_key(b, 48, &self.owner);
        put64(b, 80, self.shares);
        put64(b, 88, self.staked_in);
        put64(b, 96, self.unstaked);
        put128(b, 104, self.reward_snapshot);
        put64(b, 120, self.reward_pending);
        put64(b, 128, self.reward_paid);
        put128(b, 136, self.usdc_snapshot);
        put64(b, 152, self.usdc_pending);
        put64(b, 160, self.usdc_paid);
        put32(b, 168, self.epoch);
        put128(b, 176, self.index);
        // A `STAKE_POS_V1_LEN` position has no timer (the caller grows it
        // before `StartUnstake`).
        if b.len() >= STAKE_POS_LEN {
            put64(b, 192, self.unbonding);
            put64(b, 200, self.unbond_ts);
        }
    }
}

/// An operator-approved bound on exposure over all later fills of the
/// current resting liquidity. The policy used to calculate it is not
/// stored in the account. An account without an approval uses the full
/// settlement grid. Only `CreditAuthorize` may replace this bound with a
/// privately calculated one; permissionless cash draws increase it.
pub fn credit_approval(b: &[u8], h: &Header) -> Option<u64> {
    let base = market_len(h.grid, h.trader_cap, h.order_cap, h.maker_cap, h.claim_cap);
    (b.len() == base + CREDIT_APPROVAL_LEN && b[base..base + 8] == CREDIT_APPROVAL_MAGIC)
        .then(|| u64_at(b, base + 8))
}

pub fn set_credit_approval(b: &mut [u8], h: &Header, exposure: u64) {
    if credit_approval(b, h).is_some() {
        let base = b.len() - CREDIT_APPROVAL_LEN;
        put64(b, base + 8, exposure);
    }
}

/// Binds an approval to the exact program, market, and account bytes. A
/// changed book, withdrawal, credit allocation, or prior approval makes
/// an outstanding assessment unusable. No policy parameter is transmitted.
pub fn credit_snapshot(program: &Pubkey, market: &Pubkey, data: &[u8]) -> [u8; 32] {
    solana_program::hash::hashv(&[
        b"ameba-credit-approval-v1",
        program.as_ref(),
        market.as_ref(),
        data,
    ])
    .to_bytes()
}

/// What the credit owner's position in this market could cost its stakers,
/// and what carries its debt ([`credit_risk`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CreditRisk {
    /// Its debt less its free cash and the least it holds at any settlement
    /// in the assessed range: what its claims and inventory pay there and its
    /// reserved cash, less what its fills could give up there (0 at best).
    pub exposure: u64,
    /// Its own free and reserved cash plus the backing its claims and
    /// inventory hold: `k` times the larger of its long and short lots at
    /// each step. Long and short coverage match at every step, so this is
    /// never more than the market's backing, and only the credit owner's own
    /// instructions and the fills of its resting liquidity move it.
    pub carry: u64,
}

/// [`CreditRisk`] of trader `t` (the credit owner): now (`resting` false),
/// or at worst over every fill of its resting orders, close orders and
/// ladder lots, in any amounts and order (`resting` true). The worst case
/// is what its own `Quote` and `Submit` (and `CreditDefund`, its
/// withdrawals) must keep within the rules, so that no later fill can
/// break them and no taker is ever refused for its sake (the re-review of
/// 2026-10-18).
///
/// A fill of its liquidity is, before netting, a new claim (or inventory)
/// of the order's side paid by its contribution `c` per lot: from escrow
/// (an opening order, a ladder lot its inventory doesn't back), or from the
/// sets its netting then redeems (a close order, a lot its inventory
/// backs). Netting and `repay` change neither the payoff less free cash nor
/// cash plus backing held, so only that gross step counts:
///
/// * **Exposure.** At settlement `s` its reserved cash is its own, but for
///   what a fill there could take: a lot costs its contribution `c` and
///   becomes a claim paying `pays(s)`, so at most `(c - pays(s))^+`. For an
///   escrowed ladder lot that is its escrow on a step that pays its side
///   nothing (bids at or above `s`, asks below it), counted at the step's
///   best price for every lot, a bound; until 2026-10-27 every escrow
///   counted as spent at every `s`, which funded both ends of the grid at
///   once. Pieces paid from free cash cost the same: every close-order lot
///   and every ladder lot its step's inventory backs (the first ones to
///   fill, best tier first; any other lot a fill turns inventory-backed
///   pairs with that fill, a whole set: it costs `c - k <= 0`). The bound
///   holds for every partial fill. Between grid positions payoffs are
///   linear and these terms convex, so the least is at a grid position.
///   Only the assessed settlements count; with no fill to come
///   (`resting` false) nothing is given up.
/// * **Carry.** Per step, with net position `D` (long minus short lots), a
///   fill adds `k` per lot to the backing held except on the side that
///   offsets `D`, and costs its per-step price: a ladder tier's bid, or
///   `k` less its ask; an order's contribution over its width, rounded up.
///   The least over every fill is exact per step (the costliest `|D|` lots
///   of the offsetting side, then any pairs of opposite lots that cost more
///   than the `k` they hold), and summing the steps' least bounds the whole.
///
/// `None` on overflow.
pub fn credit_risk(b: &[u8], h: &Header, t: u16, resting: bool) -> Option<CreditRisk> {
    credit_risk_with(b, h, t, resting, &mut Vec::new())
}

/// [`credit_risk`] with a reusable work buffer: the program's allocator
/// never frees, so an instruction that checks many markets reuses one.
///
/// An open market's worst case goes through [`credit_risk_fast`], which the
/// SBF target runs about four times cheaper; where its values would leave
/// its range, the reviewed form ([`credit_risk_exact`]) decides.
pub fn credit_risk_with(
    b: &[u8],
    h: &Header,
    t: u16,
    resting: bool,
    work: &mut Vec<i128>,
) -> Option<CreditRisk> {
    let mut r = credit_risk_in_range_with(b, h, t, resting, (0, h.grid), work)?;
    if h.status == STATUS_OPEN && Trader::read(b, h.trader_off(t)).owner == h.credit_owner {
        if let Some(approved) = credit_approval(b, h) {
            // Both are upper bounds. An independently calculated full-grid
            // bound can release headroom after cancellation or a cash deposit
            // without waiting for the backend to issue another assessment.
            r.exposure = r.exposure.min(approved);
        }
    }
    Some(r)
}

/// Calculates exposure for a caller-selected settlement interval. This is
/// the backend's calculation; it ignores any cached assessment. Neither
/// the selected interval nor a default private policy is stored on chain.
/// Carry is always checked over every grid step, independent of the range.
pub fn credit_risk_in_range(
    b: &[u8],
    h: &Header,
    t: u16,
    resting: bool,
    range: (u16, u16),
) -> Option<CreditRisk> {
    credit_risk_in_range_with(b, h, t, resting, range, &mut Vec::new())
}

fn credit_risk_in_range_with(
    b: &[u8],
    h: &Header,
    t: u16,
    resting: bool,
    range: (u16, u16),
    work: &mut Vec<i128>,
) -> Option<CreditRisk> {
    if range.0 > range.1 || range.1 > h.grid {
        return None;
    }
    if h.status == STATUS_OPEN {
        if let Some(r) = credit_risk_fast(b, h, t, resting, range, work) {
            return Some(r);
        }
    }
    credit_risk_exact(b, h, t, resting, range, work)
}

/// Below this, a product of two values fits an `i64` with room: lots,
/// prices and `k` in [`credit_risk_fast`].
const FAST: u64 = 1 << 31;

/// `v` as an `i64` if below [`FAST`].
fn fast(v: u64) -> Option<i64> {
    (v < FAST).then_some(v as i64)
}

/// `a * b` of [`credit_risk_fast`]'s values: one below [`FAST`] and the
/// other below `2^32` in magnitude, so it fits. Unchecked: the SBF target
/// checks a multiplication's overflow in a call of a hundred-odd compute
/// units.
#[inline(always)]
fn mul(a: i64, b: i64) -> i64 {
    debug_assert!(a.unsigned_abs() < FAST && b.unsigned_abs() < 2 * FAST);
    a.wrapping_mul(b)
}

/// [`credit_risk`] of an open market in `i64` arithmetic, which the SBF
/// target runs natively (its 128-bit operations are chains of branches and
/// stack spills, a checked one a call): the reviewed form
/// ([`credit_risk_exact`]) ran an indebted vault's whole-grid `Quote` out of
/// the 1.4M compute cap on dev (2026-10-18). One pass over the grid, each
/// ladder step read once. Lots, prices and `k` must be below [`FAST`], so
/// every product fits; sums are checked. `None` when a value leaves that
/// range (the exact form then decides); otherwise exactly the exact form's
/// values (checked over random books in the tests). With `W(x)` the long
/// less short lots below grid position `x`, `A(x)` and `B(x)` what the
/// inventory-backed asks and bids below it could give up, `C(x)` the
/// orders', `S` the short lots and `B` all of `B`,
/// `E(x)` and `F(x)` the escrowed bids and asks below it, `E` all of `E`
/// and `R` its reserved cash, the exact form's least is `S k - B + R - E +
/// min_x (W(x) k - A(x) + B(x) - C(x) + E(x) - F(x))` over `x` in the
/// band. Never inlined: in one SBF frame with the exact form it overran
/// the 4 KiB stack (2026-10-27).
#[inline(never)]
fn credit_risk_fast(
    b: &[u8],
    h: &Header,
    t: u16,
    resting: bool,
    range: (u16, u16),
    work: &mut Vec<i128>,
) -> Option<CreditRisk> {
    let tr = Trader::read(b, h.trader_off(t));
    let k = fast(h.k)?;
    let grid = usize::from(h.grid);
    // Its claims' long and short lots per step, as difference arrays.
    work.clear();
    work.resize(2 * (grid + 1), 0);
    let (dl, ds) = work.split_at_mut(grid + 1);
    let mut at = h.claims_at();
    for _ in 0..h.claim_cap {
        // Lots and trader first: most slots are empty or someone else's.
        if u64_at(b, at) != 0 && u16_at(b, at + 16) == t {
            let c = Claim::read(b, at);
            let d = if c.side == Side::Long as u8 {
                &mut *dl
            } else {
                &mut *ds
            };
            d[usize::from(c.lo)] += i128::from(c.lots);
            d[usize::from(c.hi)] -= i128::from(c.lots);
        }
        at += CLAIM_LEN;
    }
    // Its ladder, live or holding inventory.
    let ladder = match tr.maker {
        0 => None,
        m => {
            let mk = Maker::read(b, h.maker_off(m - 1));
            let live = mk.used && mk.seq != 0 && resting;
            if live {
                fast(mk.bid_total)?;
                fast(mk.ask_total)?;
            }
            Some((h.step_off(m - 1, 0), mk, mk.sizes(), live))
        }
    };
    // Its resting orders, costliest per lot-step first.
    let mut orders: Vec<Resting> = Vec::new();
    if resting {
        for slot in 0..h.order_cap {
            let at = h.order_off(slot);
            if u64_at(b, at) == 0 || u16_at(b, at + 8) != t {
                continue;
            }
            let o = Order::read(b, at);
            if o.lots == 0 {
                continue;
            }
            let long = o.side == Side::Long as u8;
            let side = if long { Side::Long } else { Side::Short };
            let c = rm::payoff::contribution(side, o.lo, o.hi, o.price, h.k);
            orders.push(Resting {
                long,
                lo: o.lo,
                hi: o.hi,
                lots: fast(o.lots)?,
                c: fast(c)?,
                per: fast(c.div_ceil(u64::from(o.hi - o.lo)))?,
                span: (
                    u64::from(o.lo) * h.k,
                    rm::payoff::set_value(o.lo, o.hi, h.k),
                ),
            });
        }
        orders.sort_unstable_by(|p, q| q.per.cmp(&p.per));
    }
    let n = ladder.as_ref().map_or(1, |l| l.1.tiers.n());
    // Its tiers' sizes summed per side (`StepSide::lots`).
    let tier_sums = ladder.as_ref().map_or((0, 0), |l| {
        let sum = |t: &TierLots| t[..n].iter().fold(0u64, |a, &q| a.saturating_add(q));
        (sum(&l.2 .0), sum(&l.2 .1))
    });
    let (mut long, mut short) = if orders.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        (
            Vec::with_capacity(n + orders.len()),
            Vec::with_capacity(n + orders.len()),
        )
    };
    // Claims' lots at the step; over the steps below the position: long
    // less short lots times `k`, short lots, the larger side's lots.
    let (mut l, mut s) = (0i64, 0i64);
    let (mut net_k, mut shorts, mut wide) = (0i64, 0i64, 0i64);
    // What the lots its inventory backs could give up below the position:
    // its asks (long inventory) and bids (short inventory).
    let (mut ask_backed, mut bid_backed) = (0i64, 0i64);
    // The escrow of its ladder lots below the position: a fill takes an
    // ask's there and a bid's at or above it (`bid_escrow` in all, at the
    // end), at the step's best price for every lot, a bound.
    let (mut bid_escrow, mut ask_escrow) = (0i64, 0i64);
    let (band_lo, band_hi) = range;
    let mut least = i64::MAX;
    let mut floor = 0i64;
    let mut x_k = 0u64;
    let mut step_at = ladder.as_ref().map_or(0, |l| l.0);
    // A step's live lots by side and tier, `(cost per lot, lots)`, best
    // (costliest) first.
    let mut bid_lots = [(0i64, 0i64); rm::MAX_TIERS as usize];
    let mut ask_lots = [(0i64, 0i64); rm::MAX_TIERS as usize];
    for x in 0..=h.grid {
        // Settlement at grid position `x`: what it holds pays, less what its
        // fills could give up there (less the constant `S k - B`, plus its
        // reserved cash less every bid's escrow).
        let mut g = net_k
            .checked_sub(ask_backed)?
            .checked_add(bid_backed)?
            .checked_add(bid_escrow)?
            .checked_sub(ask_escrow)?;
        for o in &orders {
            // `payoff::long_per_lot` at `x`; an order's fill costs its
            // contribution, from escrow or (a close order) free cash.
            let (lo_k, value) = o.span;
            let pays = x_k.saturating_sub(lo_k).min(value);
            let pays = if o.long { pays } else { value - pays };
            let lose = (o.c as u64).saturating_sub(pays) as i64;
            g = g.checked_sub(mul(o.lots, lose))?;
        }
        if x >= band_lo && x <= band_hi {
            least = least.min(g);
        }
        if x == h.grid {
            break;
        }
        x_k += h.k;
        let xi = usize::from(x);
        l = l.checked_add(i64::try_from(dl[xi]).ok()?)?;
        s = s.checked_add(i64::try_from(ds[xi]).ok()?)?;
        let (mut sl, mut ss) = (l, s);
        // The live ladder step's two sides, if any.
        let mut sides = None;
        if let Some((_, mk, sizes, live)) = &ladder {
            let st = Step::read(b, step_at);
            step_at += STEP_LEN;
            let inv = fast(st.inv.unsigned_abs())?;
            if st.inv > 0 {
                sl = sl.checked_add(inv)?;
            } else {
                ss = ss.checked_add(inv)?;
            }
            if *live && x >= mk.lo && x < mk.hi {
                let side = |sizes, total, left, best, sum| -> Option<StepSide<'_>> {
                    Some(StepSide {
                        sizes,
                        n,
                        total,
                        // Below `FAST`, as its totals are.
                        left: if n == 1 { fast(left)? as u64 } else { left },
                        best,
                        spread: i64::from(mk.tiers.spread),
                        sum,
                    })
                };
                let bids = side(
                    &sizes.0,
                    mk.bid_total,
                    st.bid_lots,
                    fast(u64::from(st.bid))?,
                    tier_sums.0,
                )?;
                let asks = side(
                    &sizes.1,
                    mk.ask_total,
                    st.ask_lots,
                    k - i64::from(st.ask),
                    tier_sums.1,
                )?;
                // Long inventory backs its first asks, short its first bids;
                // the rest are escrowed.
                let (mut bid_q, mut ask_q) = (bids.lots(), asks.lots());
                if st.inv > 0 {
                    ask_backed = ask_backed.checked_add(asks.prefix(inv))?;
                    ask_q -= ask_q.min(inv);
                } else if st.inv < 0 {
                    bid_backed = bid_backed.checked_add(bids.prefix(inv))?;
                    bid_q -= bid_q.min(inv);
                }
                bid_escrow = bid_escrow.checked_add(mul(bid_q, bids.best.max(0)))?;
                ask_escrow = ask_escrow.checked_add(mul(ask_q, asks.best.max(0)))?;
                let crossed = u64::from(st.bid) > u64::from(st.ask).min(h.k);
                sides = Some((bids, asks, crossed));
            }
        }
        wide = wide.checked_add(sl.max(ss))?;
        shorts = shorts.checked_add(ss)?;
        let d = sl - ss;
        fast(d.unsigned_abs())?;
        net_k = net_k.checked_add(mul(d, k))?;
        if !resting {
            continue;
        }
        let least_here = match &sides {
            // A ladder alone, its step uncrossed: no pair of its lots costs
            // more than the `k` it holds (a bid at most an ask), so the
            // least is the costliest `|d|` lots of the side that offsets.
            Some((bids, asks, crossed)) if orders.is_empty() => {
                if *crossed && bids.any() && asks.any() {
                    return None;
                }
                -if d >= 0 {
                    asks.prefix(d)
                } else {
                    bids.prefix(-d)
                }
            }
            _ => {
                // The step's resting lots by side, costliest first.
                let (mut nb, mut na) = (0, 0);
                if let Some((bids, asks, _)) = &sides {
                    nb = bids.list(&mut bid_lots);
                    na = asks.list(&mut ask_lots);
                }
                let (bids, asks) = if orders.is_empty() {
                    (&bid_lots[..nb], &ask_lots[..na])
                } else {
                    merge_step(&mut long, &bid_lots[..nb], &orders, true, x);
                    merge_step(&mut short, &ask_lots[..na], &orders, false, x);
                    (&long[..], &short[..])
                };
                if d >= 0 {
                    step_least_fast(asks, bids, d, k)?
                } else {
                    step_least_fast(bids, asks, -d, k)?
                }
            }
        };
        floor = floor.checked_add(least_here)?;
    }
    // Every short pays in full at the bottom of the grid; the bids its
    // inventory backs above the position could give up; its reserved cash
    // is its own but for the bids' escrow a fill above them takes.
    let kk = i128::from(k);
    let least = i128::from(least) + i128::from(shorts) * kk - i128::from(bid_backed)
        + i128::from(tr.reserved)
        - i128::from(bid_escrow);
    let own = i128::from(wide) * kk;
    let debt = i128::from(h.debt) - i128::from(tr.free);
    let exposure = (debt - least).max(0);
    let carry = (i128::from(tr.free) + i128::from(tr.reserved) + own + i128::from(floor)).max(0);
    Some(CreditRisk {
        exposure: u64::try_from(exposure).ok()?,
        carry: u64::try_from(carry).unwrap_or(u64::MAX),
    })
}

/// A resting order of the credit owner, as [`credit_risk_fast`] counts it
/// (its lots and costs below [`FAST`]).
struct Resting {
    long: bool,
    lo: u16,
    hi: u16,
    lots: i64,
    /// Its contribution per lot, and per lot-step (rounded up).
    c: i64,
    per: i64,
    /// Where a long payoff on its range starts and its set's value, in
    /// fine units.
    span: (u64, u64),
}

/// One side of a live ladder step, as [`credit_risk_fast`] counts it: its
/// tiers of `sizes`, `left` of the `total` lots a step quotes remaining
/// (`Tiers::remaining_in`; below [`FAST`]), the best tier's cost per lot
/// `best` and each further tier's `spread` less, never below 0
/// (`Tiers::price`: a bid's price, `k` less an ask's).
struct StepSide<'a> {
    sizes: &'a TierLots,
    n: usize,
    total: u64,
    left: u64,
    best: i64,
    spread: i64,
    /// Its tiers' sizes summed (the same for every step).
    sum: u64,
}

impl StepSide<'_> {
    /// Whether it has lots.
    fn any(&self) -> bool {
        self.left
            .min(if self.n == 1 { u64::MAX } else { self.total })
            > 0
    }

    /// Its tiers' remaining lots and costs, best first, to `f` until it
    /// returns `false`.
    #[inline(always)]
    fn each(&self, mut f: impl FnMut(i64, i64) -> bool) {
        if self.n == 1 {
            if self.left > 0 {
                f(self.best.max(0), self.left as i64);
            }
            return;
        }
        let mut filled = self.total.saturating_sub(self.left);
        let mut cost = self.best;
        for &size in &self.sizes[..self.n] {
            let used = filled.min(size);
            filled -= used;
            if size > used && !f(cost.max(0), (size - used) as i64) {
                return;
            }
            cost -= self.spread;
        }
    }

    /// Its remaining lots over every tier, as [`StepSide::each`] counts
    /// them (without walking the tiers: the credit check reads every step).
    fn lots(&self) -> i64 {
        if self.n == 1 {
            return self.left as i64;
        }
        let filled = self.total.saturating_sub(self.left);
        (self.sum - filled.min(self.sum)) as i64
    }

    /// What its first `q` lots cost, best tier first (at most its lots).
    fn prefix(&self, q: i64) -> i64 {
        let (mut left, mut total) = (q, 0i64);
        if left > 0 {
            self.each(|cost, lots| {
                let take = left.min(lots);
                // `take` below `FAST` and costs at most the best's: the sum
                // is under `2^62`.
                total += mul(take, cost);
                left -= take;
                left > 0
            });
        }
        total
    }

    /// Its tiers with lots as `(cost per lot, lots)`, best first, into
    /// `out`; how many.
    fn list(&self, out: &mut [(i64, i64)]) -> usize {
        let mut n = 0;
        self.each(|cost, lots| {
            out[n] = (cost, lots);
            n += 1;
            true
        });
        n
    }
}

/// `tiers` (costliest first) and the `long` side's `orders` (costliest
/// first) that rest on step `x`, merged costliest first into `out`.
fn merge_step(
    out: &mut Vec<(i64, i64)>,
    tiers: &[(i64, i64)],
    orders: &[Resting],
    long: bool,
    x: u16,
) {
    out.clear();
    let mut i = 0;
    for o in orders
        .iter()
        .filter(|o| o.long == long && o.lo <= x && x < o.hi)
    {
        while i < tiers.len() && tiers[i].0 >= o.per {
            out.push(tiers[i]);
            i += 1;
        }
        out.push((o.per, o.lots));
    }
    out.extend_from_slice(&tiers[i..]);
}

/// [`step_least`] in `i64`: costs, lots, `need` and `k` below [`FAST`], so
/// each product fits; `None` if the sum would not.
fn step_least_fast(offset: &[(i64, i64)], other: &[(i64, i64)], need: i64, k: i64) -> Option<i64> {
    let mut total = 0i64;
    let mut need = need;
    let (mut i, mut left) = (0usize, offset.first().map_or(0, |o| o.1));
    while need > 0 && i < offset.len() {
        let q = left.min(need);
        total = total.checked_sub(mul(q, offset[i].0))?;
        need -= q;
        left -= q;
        if left == 0 {
            i += 1;
            left = offset.get(i).map_or(0, |o| o.1);
        }
    }
    let (mut j, mut right) = (0usize, other.first().map_or(0, |o| o.1));
    while i < offset.len() && j < other.len() {
        // A pair costs `k - cost - cost`: only one dearer than `k` counts.
        let over = offset[i].0 + other[j].0 - k;
        if over <= 0 {
            break;
        }
        let q = left.min(right);
        total = total.checked_sub(mul(q, over))?;
        left -= q;
        right -= q;
        if left == 0 {
            i += 1;
            left = offset.get(i).map_or(0, |o| o.1);
        }
        if right == 0 {
            j += 1;
            right = other.get(j).map_or(0, |o| o.1);
        }
    }
    Some(total)
}

/// [`credit_risk`] in its reviewed form (c4b063e6, the re-review of
/// 2026-10-18): two passes over the grid in `i128`. Every value
/// [`credit_risk_fast`] can't hold, and every final market, goes here.
#[inline(never)]
fn credit_risk_exact(
    b: &[u8],
    h: &Header,
    t: u16,
    resting: bool,
    range: (u16, u16),
    work: &mut Vec<i128>,
) -> Option<CreditRisk> {
    let tr = Trader::read(b, h.trader_off(t));
    if h.status != STATUS_OPEN {
        // Final: one outcome, and the debt is carried by the backing (and its
        // cash until the credit settles).
        let (low, _) = trader_value_with(b, h, t, work)?;
        let at_risk = u128::from(h.debt) + u128::from(tr.reserved);
        let cash = if h.credit_settled() {
            0
        } else {
            u128::from(tr.free) + u128::from(tr.reserved)
        };
        return Some(CreditRisk {
            exposure: u64::try_from(at_risk.saturating_sub(u128::from(low))).ok()?,
            // Saturated: a carry past a `u64` covers any debt (Codex's
            // review, 2026-10-18: an admitted fill must never overflow).
            carry: u64::try_from(cash + u128::from(h.backing)).unwrap_or(u64::MAX),
        });
    }
    let kk = i128::from(h.k);
    let grid = usize::from(h.grid);
    // Long and short lots per step, as difference arrays.
    work.clear();
    work.resize(2 * (grid + 1), 0);
    let (dl, ds) = work.split_at_mut(grid + 1);
    for i in 0..h.claim_cap {
        // Lots and trader first: most slots are empty or someone else's.
        let at = h.claim_off(i);
        if u64_at(b, at) == 0 || u16_at(b, at + 16) != t {
            continue;
        }
        let c = Claim::read(b, at);
        let d = if c.side == Side::Long as u8 {
            &mut *dl
        } else {
            &mut *ds
        };
        d[usize::from(c.lo)] += i128::from(c.lots);
        d[usize::from(c.hi)] -= i128::from(c.lots);
    }
    // Its ladder, live or holding inventory.
    let ladder = (tr.maker > 0).then(|| {
        let mk = Maker::read(b, h.maker_off(tr.maker - 1));
        (
            tr.maker - 1,
            mk,
            mk.sizes(),
            mk.used && mk.seq != 0 && resting,
        )
    });
    // Its resting orders: side, lo, hi, lots, contribution per lot, close.
    let mut orders: Vec<(Side, u16, u16, u64, u64, bool)> = Vec::new();
    if resting {
        for slot in 0..h.order_cap {
            let at = h.order_off(slot);
            if u64_at(b, at) == 0 || u16_at(b, at + 8) != t {
                continue;
            }
            let o = Order::read(b, at);
            if o.lots == 0 {
                continue;
            }
            let side = if o.side == Side::Long as u8 {
                Side::Long
            } else {
                Side::Short
            };
            let c = rm::payoff::contribution(side, o.lo, o.hi, o.price, h.k);
            orders.push((side, o.lo, o.hi, o.lots, c, o.flags & ORDER_CLOSE != 0));
        }
    }
    // A step's live ladder lots per tier (bids, asks) and its inventory.
    let step = |x: u16| -> (TierLots, TierLots, [u64; rm::MAX_TIERS as usize * 2], i64) {
        let mut prices = [0u64; rm::MAX_TIERS as usize * 2];
        let Some((m, mk, sizes, live)) = &ladder else {
            return (
                [0; rm::MAX_TIERS as usize],
                [0; rm::MAX_TIERS as usize],
                prices,
                0,
            );
        };
        let st = Step::read(b, h.step_off(*m, x));
        if !*live || x < mk.lo || x >= mk.hi {
            return (
                [0; rm::MAX_TIERS as usize],
                [0; rm::MAX_TIERS as usize],
                prices,
                st.inv,
            );
        }
        let (bids, asks) = mk.step_lots(sizes, &st);
        let n = mk.tiers.n();
        for i in 0..n {
            prices[i] = mk.tiers.price(st.bid, Side::Long, i, h.k);
            prices[n + i] = mk.tiers.price(st.ask, Side::Short, i, h.k);
        }
        (bids, asks, prices, st.inv)
    };
    let n = ladder.as_ref().map_or(1, |l| l.1.tiers.n());
    // Inventory into the coverage; what the lots it backs could cost at
    // settlement: asks (long inventory) from the step's top up, bids (short
    // inventory) from its bottom down.
    let mut bid_backed_total = 0i128;
    // The escrow of its ladder lots, at each step's best price for every lot
    // (a bound): a fill takes an ask's below the position, a bid's at or
    // above it.
    let escrow = |lots: &TierLots, best: u64, backed: u64| -> i128 {
        let q: u64 = lots.iter().sum();
        i128::from(q - q.min(backed)) * i128::from(best)
    };
    let mut bid_escrow_total = 0i128;
    if ladder.is_some() {
        for x in 0..h.grid {
            let (bids, _, prices, inv) = step(x);
            let q = i128::from(inv.unsigned_abs());
            let xi = usize::from(x);
            if inv > 0 {
                dl[xi] += q;
                dl[xi + 1] -= q;
            } else if inv < 0 {
                ds[xi] += q;
                ds[xi + 1] -= q;
            }
            bid_backed_total += backed_cost(&bids, &prices[..n], inv.min(0).unsigned_abs(), |p| p);
            bid_escrow_total += escrow(&bids, prices[0], inv.min(0).unsigned_abs());
        }
    }
    // The shorts pay in full at the bottom of the grid.
    let mut v = 0i128;
    {
        let mut s = 0i128;
        for d in &ds[..grid] {
            s += d;
            v += s;
        }
    }
    let (mut l, mut s) = (0i128, 0i128);
    let mut own = 0i128;
    let mut least = i128::MAX;
    let mut floor = 0i128;
    let mut ask_backed = 0i128;
    let mut bid_backed = bid_backed_total;
    // Its ladder lots' escrow below the position (`escrow`).
    let (mut bid_escrow, mut ask_escrow) = (0i128, 0i128);
    let (band_lo, band_hi) = range;
    let mut long: Vec<(i128, i128)> = Vec::with_capacity(n + orders.len());
    let mut short: Vec<(i128, i128)> = Vec::with_capacity(n + orders.len());
    for x in 0..=h.grid {
        // Settlement at grid position `x`: what it holds pays, its reserved
        // cash, less what its fills could give up there: the ladder lots'
        // escrow, and an order's contribution `c` (from escrow, or a close
        // order's from free cash) beyond what its claim pays.
        let mut pen = ask_backed + bid_backed + (bid_escrow_total - bid_escrow) + ask_escrow;
        for &(side, lo, hi, lots, c, _) in &orders {
            let pays = i128::from(rm::payoff::long_per_lot(lo, hi, u64::from(x) * h.k, h.k));
            let pays = match side {
                Side::Long => pays,
                Side::Short => i128::from(rm::payoff::set_value(lo, hi, h.k)) - pays,
            };
            pen += i128::from(lots) * (i128::from(c) - pays).max(0);
        }
        if x >= band_lo && x <= band_hi {
            least = least.min(v * kk + i128::from(tr.reserved) - pen);
        }
        if x == h.grid {
            break;
        }
        let xi = usize::from(x);
        l += dl[xi];
        s += ds[xi];
        own += kk * l.max(s);
        v += l - s;
        let (bids, asks, prices, inv) = step(x);
        ask_backed += backed_cost(&asks, &prices[n..2 * n], inv.max(0).unsigned_abs(), |p| {
            h.k.saturating_sub(p)
        });
        bid_backed -= backed_cost(&bids, &prices[..n], inv.min(0).unsigned_abs(), |p| p);
        ask_escrow += escrow(
            &asks,
            h.k.saturating_sub(prices[n]),
            inv.max(0).unsigned_abs(),
        );
        bid_escrow += escrow(&bids, prices[0], inv.min(0).unsigned_abs());
        if !resting {
            continue;
        }
        // The step's resting lots by side, costliest first.
        long.clear();
        short.clear();
        for i in 0..n {
            if bids[i] > 0 {
                long.push((i128::from(prices[i]), i128::from(bids[i])));
            }
            if asks[i] > 0 {
                short.push((kk - i128::from(prices[n + i]), i128::from(asks[i])));
            }
        }
        let mut sorted = true;
        for &(side, lo, hi, lots, c, _) in &orders {
            if lo <= x && x < hi {
                let w = i128::from(hi - lo);
                let per = (i128::from(c) + w - 1) / w;
                match side {
                    Side::Long => long.push((per, i128::from(lots))),
                    Side::Short => short.push((per, i128::from(lots))),
                }
                sorted = false;
            }
        }
        if !sorted {
            long.sort_unstable_by(|p, q| q.0.cmp(&p.0));
            short.sort_unstable_by(|p, q| q.0.cmp(&p.0));
        }
        let (offset, other, need) = if l >= s {
            (&short, &long, l - s)
        } else {
            (&long, &short, s - l)
        };
        floor += step_least(offset, other, need, kk);
    }
    let debt = i128::from(h.debt) - i128::from(tr.free);
    let exposure = (debt - least).max(0);
    let carry = (i128::from(tr.free) + i128::from(tr.reserved) + own + floor).max(0);
    Some(CreditRisk {
        exposure: u64::try_from(exposure).ok()?,
        carry: u64::try_from(carry).unwrap_or(u64::MAX),
    })
}

/// What the first `inv` lots of a step's side (`lots` per tier, best first,
/// at `prices`) cost, `cost(price)` each: the lots its inventory backs.
fn backed_cost(lots: &TierLots, prices: &[u64], inv: u64, cost: impl Fn(u64) -> u64) -> i128 {
    let mut left = inv;
    let mut total = 0i128;
    for (i, &p) in prices.iter().enumerate() {
        if left == 0 {
            break;
        }
        let q = lots[i].min(left);
        left -= q;
        total += i128::from(q) * i128::from(cost(p));
    }
    total
}

/// The least a step's fills can add to the backing held less what they
/// cost: `offset` (the side that offsets its net position, `need` lots of
/// it) and `other`, each `(cost per lot, lots)` costliest first. The
/// costliest `need` offsetting lots hold nothing new; past them, each lot
/// paired with one of the other side holds `k` for both.
fn step_least(offset: &[(i128, i128)], other: &[(i128, i128)], need: i128, k: i128) -> i128 {
    let mut total = 0i128;
    let mut need = need;
    let (mut i, mut left) = (0usize, offset.first().map_or(0, |o| o.1));
    while need > 0 && i < offset.len() {
        let q = left.min(need);
        total -= q * offset[i].0;
        need -= q;
        left -= q;
        if left == 0 {
            i += 1;
            left = offset.get(i).map_or(0, |o| o.1);
        }
    }
    let (mut j, mut right) = (0usize, other.first().map_or(0, |o| o.1));
    while i < offset.len() && j < other.len() {
        let pair = k - offset[i].0 - other[j].0;
        if pair >= 0 {
            break;
        }
        let q = left.min(right);
        total += q * pair;
        left -= q;
        right -= q;
        if left == 0 {
            i += 1;
            left = offset.get(i).map_or(0, |o| o.1);
        }
        if right == 0 {
            j += 1;
            right = other.get(j).map_or(0, |o| o.1);
        }
    }
    total
}

/// What the credit owner's position here could cost its stakers at worst
/// over every fill of its resting liquidity ([`credit_risk`]): the bound its
/// own instructions keep within the credit limit. `None` on overflow.
pub fn credit_exposure(b: &[u8], h: &Header, t: u16) -> Option<u64> {
    credit_risk(b, h, t, true).map(|r| r.exposure)
}

/// Whether the credit owner `t` may act as it just did: while its open
/// market lends it anything (`debt`), at worst over every fill of its
/// resting liquidity its exposure stays within the credit limit and its own
/// cash and backing held carry its debt ([`credit_risk`]). `None` on
/// overflow.
pub fn credit_admits(b: &[u8], h: &Header, t: u16) -> Option<bool> {
    if h.debt == 0
        || !h.has_credit()
        || h.credit_settled()
        || h.status != STATUS_OPEN
        || Trader::read(b, h.trader_off(t)).owner != h.credit_owner
    {
        return Some(true);
    }
    let r = credit_risk(b, h, t, true)?;
    Some(r.exposure <= h.credit_limit && h.debt <= r.carry)
}

/// What a raw value `raw` of the credit owner's trader here ([`trader_value`])
/// is worth to its vault at `now`: less its debt, never below zero (the
/// stakers carry the rest), and less the stakers' share of the profit.
pub fn credit_equity(h: &Header, raw: u64, now: u64) -> u64 {
    if h.credit_settled() {
        return raw;
    }
    let equity = raw.saturating_sub(h.debt);
    let mut clock = *h;
    if clock.credit_clock(now).is_none() {
        return equity;
    }
    equity - clock.staker_share(equity)
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

    /// `step_least` is the least over every fill of a step's lots, by brute
    /// force: `k` per lot of the larger side's growth, less the fills' cost.
    #[test]
    fn step_least_is_the_least_over_every_fill() {
        let k = 10i128;
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for _ in 0..3_000 {
            let (l, s) = (next(5) as i128, next(5) as i128);
            let mut side = |n: u64| {
                let mut v: Vec<(i128, i128)> = (0..next(n))
                    .map(|_| (next(11) as i128, 1 + next(3) as i128))
                    .collect();
                v.sort_by(|p, q| q.0.cmp(&p.0));
                v
            };
            let (long, short) = (side(4), side(4));
            let all: Vec<(bool, i128, i128)> = long
                .iter()
                .map(|&(c, n)| (true, c, n))
                .chain(short.iter().map(|&(c, n)| (false, c, n)))
                .collect();
            let mut fill = vec![0i128; all.len()];
            let mut least = i128::MAX;
            loop {
                let (mut a, mut b, mut cost) = (0, 0, 0);
                for (&(is_long, c, _), &m) in all.iter().zip(&fill) {
                    if is_long {
                        a += m;
                    } else {
                        b += m;
                    }
                    cost += m * c;
                }
                least = least.min(k * ((l + a).max(s + b) - l.max(s)) - cost);
                let mut i = 0;
                while i < all.len() && fill[i] == all[i].2 {
                    fill[i] = 0;
                    i += 1;
                }
                if i == all.len() {
                    break;
                }
                fill[i] += 1;
            }
            let got = if l >= s {
                step_least(&short, &long, l - s, k)
            } else {
                step_least(&long, &short, s - l, k)
            };
            assert_eq!(got, least, "L {l} S {s} long {long:?} short {short:?}");
            // And its `i64` form.
            let small = |v: &[(i128, i128)]| -> Vec<(i64, i64)> {
                v.iter().map(|&(c, n)| (c as i64, n as i64)).collect()
            };
            let (long, short) = (small(&long), small(&short));
            let fast = if l >= s {
                step_least_fast(&short, &long, (l - s) as i64, k as i64)
            } else {
                step_least_fast(&long, &short, (s - l) as i64, k as i64)
            };
            assert_eq!(fast, Some(least as i64));
        }
    }

    /// `credit_risk_fast` gives exactly the reviewed form's values, now and
    /// at the worst fill, over random books: claims of both sides, opening
    /// and close orders, a ladder of up to 8 tiers with inventory either
    /// way, live or not, over a reference's band or the whole grid. Lots past `FAST` make it defer to the exact form,
    /// as `credit_risk` then does.
    #[test]
    fn credit_risk_matches_the_reviewed_form() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n.max(1)
        };
        let (traders, orders, makers, claims) = (3u16, 6u16, 2u8, 12u32);
        let (mut counted, mut deferred, mut alone) = (0, 0, 0);
        for round in 0..4_000 {
            let grid = 4 + next(10) as u16;
            let k = [10u64, 100, 10_000][next(3) as usize];
            let big = round % 7 == 0;
            let lots = |n: u64| {
                if big && n.is_multiple_of(3) {
                    (1u64 << 33) + n
                } else {
                    1 + n % 40
                }
            };
            let h = Header {
                status: STATUS_OPEN,
                grid,
                trader_cap: traders,
                order_cap: orders,
                maker_cap: makers,
                claim_cap: claims,
                k,
                debt: next(3_000_000),
                // Half the books cover a band around a reference, the rest
                // the whole grid (no reference).
                ref_fine: if round % 2 == 0 {
                    next(u64::from(grid) + 1) * k
                } else {
                    0
                },
                ..Header::default()
            };
            let mut b = vec![0u8; market_len(grid, traders, orders, makers, claims)];
            h.write(&mut b);
            let t = 1u16;
            let maker = next(3) as u8;
            Trader {
                owner: Pubkey::new_unique(),
                free: next(1_000_000),
                reserved: next(1_000_000),
                maker,
                ..Trader::default()
            }
            .write(&mut b, h.trader_off(t));
            let range = |next: &mut dyn FnMut(u64) -> u64| {
                let lo = next(u64::from(grid)) as u16;
                (lo, lo + 1 + next(u64::from(grid - lo)) as u16)
            };
            for i in 0..claims {
                if next(3) == 0 {
                    continue;
                }
                let (lo, hi) = range(&mut next);
                let n = next(1_000);
                Claim {
                    lots: lots(n),
                    seq: 1 + u64::from(i),
                    trader: if next(4) == 0 { 2 } else { t },
                    side: next(2) as u8,
                    lo,
                    hi,
                    ..Claim::default()
                }
                .write(&mut b, h.claim_off(i));
            }
            for i in 0..orders {
                // A third of the books hold no orders: a ladder alone.
                if round % 3 == 0 || next(2) == 0 {
                    continue;
                }
                let (lo, hi) = range(&mut next);
                let n = next(1_000);
                Order {
                    seq: 1 + u64::from(i),
                    trader: if next(4) == 0 { 0 } else { t },
                    side: next(2) as u8,
                    flags: if next(3) == 0 { ORDER_CLOSE } else { 0 },
                    lo,
                    hi,
                    lots: if next(5) == 0 { 0 } else { lots(n) },
                    price: next(u64::from(hi - lo) * k + 1),
                    ..Order::default()
                }
                .write(&mut b, h.order_off(i));
            }
            for m in 0..makers {
                let (lo, hi) = range(&mut next);
                let (n1, n2) = (next(1_000), next(1_000));
                let mk = Maker {
                    seq: next(2),
                    trader: t,
                    used: next(5) != 0,
                    lo,
                    hi,
                    tiers: Tiers {
                        count: next(u64::from(rm::MAX_TIERS) + 1) as u8,
                        ratio: [50, 100, 120, 300][next(4) as usize],
                        spread: next(k / 4 + 1) as u32,
                    },
                    bid_total: lots(n1),
                    ask_total: lots(n2),
                    ..Maker::default()
                };
                mk.write(&mut b, h.maker_off(m));
                for x in 0..grid {
                    let bid = next(k);
                    let mut ask = bid + 1 + next(k - bid);
                    // Now and then a crossed step (the exact form decides).
                    if next(200) == 0 {
                        ask = bid / 2;
                    }
                    let n = next(1_000);
                    let inv = lots(n) as i64 * [-1, 0, 1][next(3) as usize];
                    Step {
                        ask: ask as u32,
                        bid: bid as u32,
                        ask_lots: next(mk.ask_total + 2),
                        bid_lots: next(mk.bid_total + 2),
                        inv,
                    }
                    .write(&mut b, h.step_off(m, x));
                }
            }
            for resting in [false, true] {
                let lo = next(u64::from(h.grid) + 1) as u16;
                let hi = lo + next(u64::from(h.grid - lo) + 1) as u16;
                let range = if round % 2 == 0 {
                    (lo, hi)
                } else {
                    (0, h.grid)
                };
                let want = credit_risk_exact(&b, &h, t, resting, range, &mut Vec::new());
                assert_eq!(credit_risk_in_range(&b, &h, t, resting, range), want);
                match credit_risk_fast(&b, &h, t, resting, range, &mut Vec::new()) {
                    Some(got) => {
                        assert_eq!(Some(got), want, "round {round} resting {resting}");
                        counted += usize::from(got.exposure > 0 && got.carry > 0);
                        alone += usize::from(resting && maker > 0 && round % 3 == 0);
                    }
                    None => deferred += 1,
                }
            }
        }
        assert!(counted > 2_000, "{counted}");
        assert!((100..2_000).contains(&deferred), "{deferred}");
        assert!(alone > 500, "{alone}");
    }

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
    fn credit_fields_round_trip_in_the_header() {
        let h = Header {
            grid: 4,
            trader_cap: 1,
            order_cap: 1,
            maker_cap: 0,
            claim_cap: 1,
            claim_hint: 7,
            last_trade_ts: 9,
            credit_owner: Pubkey::new_unique(),
            credit_limit: 1,
            debt: 2,
            credit_until: 3,
            funded_in: 4,
            funded_out: 5,
            tw_ts: 6,
            tw_credit: u128::MAX - 1,
            tw_cash: 8,
            shortfall: 10,
            recovery_amba: 11,
            winners: 12,
            credit_flags: CREDIT_SETTLED | CREDIT_SHORTFALL,
            profit_bps: 5_000,
            ..Header::default()
        };
        let mut b = vec![0u8; market_len(4, 1, 1, 0, 1)];
        h.write(&mut b);
        assert_eq!(Header::read(&b).unwrap(), h);
        // A market from before credit reads as none.
        let plain = Header {
            credit_owner: Pubkey::default(),
            ..Header::default()
        };
        assert!(!plain.has_credit() && !plain.credit_settled());
    }

    fn pool() -> StakePool {
        StakePool {
            amba_decimals: 9,
            haircut_bps: 5_000,
            profit_bps: 5_000,
            buffer_bps: 1_000,
            twap_window: 3_600,
            credit_counted: true,
            ..StakePool::default()
        }
    }

    #[test]
    fn the_average_follows_the_posts_and_backs_half() {
        let mut p = pool();
        assert_eq!(p.twap_at(100), 0, "no price, no credit");
        // $0.02 an AMBA: 20,000 USDC atoms per AMBA.
        p.post(20_000 * PRICE_SCALE as u64, 1_000);
        assert_eq!(p.twap_at(5_000), 20_000_000_000);
        // A jump to $0.04 reaches the average over the window.
        p.post(40_000 * PRICE_SCALE as u64, 2_000);
        assert_eq!(p.twap_at(2_000), 20_000_000_000);
        assert_eq!(p.twap_at(2_900), 25_000_000_000);
        assert_eq!(p.twap_at(5_600), 40_000_000_000);
        // A post halfway keeps the average where it was.
        p.post(10_000 * PRICE_SCALE as u64, 2_900);
        assert_eq!(p.twap_at(2_900), 25_000_000_000);
        assert_eq!(p.twap_at(3_800), 21_250_000_000);
        // 1,000,000 AMBA at $0.02, haircut 50%: $10,000 of credit.
        p.post(20_000 * PRICE_SCALE as u64, 10_000);
        p.twap = p.price;
        p.staked = 1_000_000 * 1_000_000_000;
        assert_eq!(p.capacity(10_000), Some(10_000 * 1_000_000));
        // A $50 shortfall takes 2,500 AMBA at the average (rounded up).
        assert_eq!(
            p.amba_for(50 * 1_000_000, p.twap),
            Some(2_500 * 1_000_000_000)
        );
        assert_eq!(p.amba_for(1, p.twap), Some(50_000));
        assert_eq!(
            p.usdc_of(2_500 * 1_000_000_000, p.twap),
            Some(50 * 1_000_000)
        );
    }

    #[test]
    fn shares_follow_what_arrives_and_survive_an_emptied_pool() {
        let mut p = pool();
        assert_eq!(p.shares_for(1_000), Some(1_000));
        p.staked = 1_000;
        p.shares = 1_000;
        // A shortfall takes half the AMBA: a new staker's atom buys two
        // shares (less the virtual share's rounding).
        p.staked = 500;
        assert_eq!(p.shares_for(500), Some(999));
        assert_eq!(p.amba_of(1_000), 500);
        // Everything taken: the old shares are worth nothing, new ones price.
        p.staked = 0;
        assert_eq!(p.amba_of(1_000), 0);
        assert_eq!(p.shares_for(10), Some(10 * 1_001));
    }

    #[test]
    fn stakers_share_the_credit_weighted_profit() {
        let mut h = Header {
            expiry_ts: 1_500,
            funded_in: 1_000,
            profit_bps: 5_000,
            ..Header::default()
        };
        // Funding alone from t = 500; credit equal to it from t = 1,000.
        h.credit_clock(500).unwrap();
        h.credit_clock(1_000).unwrap();
        h.credit_limit = 1_000;
        // The clock stops at expiry.
        h.credit_clock(5_000).unwrap();
        assert_eq!(
            (h.tw_cash, h.tw_credit, h.tw_ts),
            (1_000_000, 500_000, 1_500)
        );
        // A third of the capacity was credit: 50% of a $300 profit, a third.
        assert_eq!(h.credit_weight(), WEIGHT_SCALE / 3);
        assert_eq!(h.staker_share(1_300), 49);
        assert_eq!(h.staker_share(1_000), 0, "no profit, nothing");
        assert_eq!(h.staker_share(400), 0, "a loss, nothing");
        // The vault's equity: less the debt, never below zero, less the share.
        h.credit_owner = Pubkey::new_unique();
        h.debt = 200;
        assert_eq!(credit_equity(&h, 1_500, 5_000), 1_300 - 49);
        assert_eq!(credit_equity(&h, 100, 5_000), 0);
        h.credit_flags = CREDIT_SETTLED;
        assert_eq!(credit_equity(&h, 1_500, 5_000), 1_500, "settled: its cash");
    }

    #[test]
    fn the_buffer_fills_to_its_target_first() {
        let mut p = pool();
        p.allocated = 10_000;
        p.shares = 100;
        // Target 10% of the credit: 1,000 to the buffer, the rest to shares.
        p.take_profit(1_500).unwrap();
        assert_eq!((p.buffer, p.usdc_distributed), (1_000, 500));
        assert_eq!(p.usdc_acc, 5 * USDC_SCALE);
        let mut pos = StakePosition {
            shares: 40,
            ..StakePosition::default()
        };
        p.touch(&mut pos, 0).unwrap();
        assert_eq!(pos.usdc_pending, 200);
        // The credit comes back: the buffer above its new target goes out.
        p.allocated = 4_000;
        p.release_buffer().unwrap();
        assert_eq!(
            (p.buffer, p.usdc_distributed, p.usdc_owed()),
            (400, 1_100, 1_100)
        );
        p.touch(&mut pos, 0).unwrap();
        assert_eq!(pos.usdc_pending, 440);
        let mut b = vec![0u8; STAKE_LEN];
        p.write(&mut b);
        assert_eq!(StakePool::read(&b).unwrap(), p);
        let mut b = vec![0u8; STAKE_POS_LEN];
        pos.write(&mut b);
        assert_eq!(StakePosition::read(&b).unwrap(), pos);
    }

    /// A price too low to buy a debt with every staked atom asks for them
    /// all, never an overflow (the review of 2026-10-18, finding 8).
    #[test]
    fn a_low_price_asks_for_every_atom() {
        let p = pool();
        let price = 20_000 * PRICE_SCALE as u64;
        assert_eq!(
            p.amba_for_at_most(150_000_000, price, u64::MAX),
            p.amba_for(150_000_000, price).unwrap()
        );
        assert_eq!(p.amba_for(u64::MAX, 1), None);
        assert_eq!(p.amba_for_at_most(u64::MAX, 1, 500), 500);
        assert_eq!(p.amba_for_at_most(1_000, 0, 500), 500);
    }

    /// After a shortfall took every staked atom, the next stake starts a new
    /// epoch: the old shares keep what they earned and earn nothing more,
    /// and the new ones aren't priced against them (finding 9); positions up
    /// to `EPOCH_ENDS` epochs behind are paid (its re-review).
    #[test]
    fn a_full_slash_starts_a_new_epoch() {
        let mut p = pool();
        p.staked = 100;
        p.shares = 100;
        let mut a = StakePosition {
            shares: 100,
            ..StakePosition::default()
        };
        let mut old = a;
        p.spread(1_000).unwrap();
        // The shortfall takes it all; a new staker comes.
        p.staked = 0;
        p.begin_epoch(0).unwrap();
        assert_eq!((p.epoch, p.shares), (1, 0));
        assert_eq!(p.shares_for(50), Some(50), "one share an atom again");
        p.staked = 50;
        p.shares = 50;
        let mut b = StakePosition {
            shares: 50,
            epoch: 1,
            usdc_snapshot: p.usdc_acc,
            ..StakePosition::default()
        };
        p.spread(500).unwrap();
        p.touch(&mut a, 0).unwrap();
        assert_eq!((a.usdc_pending, a.shares, a.epoch), (1_000, 0, 1));
        p.touch(&mut b, 0).unwrap();
        assert_eq!(b.usdc_pending, 500);
        // Two epochs on, an untouched position of the first is still paid
        // what it earned in it; the second epoch's, what it earned there.
        p.staked = 0;
        p.begin_epoch(0).unwrap();
        p.staked = 10;
        p.shares = 10;
        p.spread(100).unwrap();
        let mut older = old;
        p.touch(&mut old, 0).unwrap();
        assert_eq!((old.usdc_pending, old.shares, old.epoch), (1_000, 0, 2));
        p.touch(&mut b, 0).unwrap();
        assert_eq!((b.usdc_pending, b.shares), (500, 0));
        p.touch(&mut a, 0).unwrap();
        assert_eq!((a.usdc_pending, a.shares), (1_000, 0));
        // The ends kept: `EPOCH_ENDS` back still pays; further back, nothing.
        for _ in 2..EPOCH_ENDS {
            p.staked = 0;
            p.begin_epoch(0).unwrap();
        }
        let mut c = older;
        p.touch(&mut c, 0).unwrap();
        assert_eq!(c.usdc_pending, 1_000);
        p.staked = 0;
        p.begin_epoch(0).unwrap();
        p.touch(&mut older, 0).unwrap();
        assert_eq!((older.usdc_pending, older.shares), (0, 0));
        let mut buf = vec![0u8; STAKE_LEN];
        p.write(&mut buf);
        assert_eq!(StakePool::read(&buf).unwrap(), p);
    }

    /// A shortfall that leaves a few atoms against many shares: a stake too
    /// large for the shares it would need starts a new epoch, and the old
    /// shares convert into the atoms they held (the re-review of
    /// 2026-10-18).
    #[test]
    fn a_near_total_wipe_out_starts_a_new_epoch() {
        let mut p = pool();
        let big = 1_000_000_000_000_000; // a million AMBA, in atoms
        p.staked = big;
        p.shares = big;
        // An ordinary stake doesn't roll over, even one as large again.
        assert!(!p.rolls_over(big));
        let mut a = StakePosition {
            shares: big,
            ..StakePosition::default()
        };
        p.spread(1_000).unwrap();
        // The shortfall leaves 3 atoms: the shares a million AMBA would
        // need overflow, so it starts epoch 1 at those 3 atoms' 3 shares.
        p.staked = 3;
        assert_eq!(p.shares_for(big), None);
        assert!(p.rolls_over(big) && !p.rolls_over(1));
        p.begin_epoch(0).unwrap();
        assert_eq!((p.epoch, p.shares, p.staked), (1, 3, 3));
        assert_eq!(p.epoch_rates[0], (3, big));
        let shares = p.shares_for(big).unwrap();
        assert_eq!(shares, big, "one share an atom");
        p.staked += big;
        p.shares += shares;
        let mut b = StakePosition {
            shares,
            epoch: 1,
            usdc_snapshot: p.usdc_acc,
            ..StakePosition::default()
        };
        // The new epoch's USDC goes by the new shares, the old cohort's 3
        // among them.
        p.spread(big + 3).unwrap();
        p.touch(&mut a, 0).unwrap();
        assert_eq!((a.usdc_pending, a.shares, a.epoch), (1_000 + 3, 3, 1));
        assert_eq!(p.amba_of(a.shares), 3, "the residual is still its own");
        p.touch(&mut b, 0).unwrap();
        assert_eq!((b.usdc_pending, b.shares), (big, big));
        // Shares that fit alone but not added to the pool's also roll over.
        let mut q = pool();
        q.staked = 1;
        q.shares = u64::MAX - 10;
        assert!(q.shares_for(1).is_some());
        assert!(q.rolls_over(1) && !q.rolls_over(0));
    }

    /// Old positions keep their part of the residual: `shares * R / N`,
    /// rounded down; the rounding stays unowned in the pool.
    #[test]
    fn old_positions_keep_the_residual_they_held() {
        let mut p = pool();
        p.staked = 7;
        p.shares = 10;
        let mut pos: Vec<StakePosition> = [3, 3, 4]
            .map(|shares| StakePosition {
                shares,
                ..StakePosition::default()
            })
            .to_vec();
        let before: Vec<u64> = pos.iter().map(|x| p.amba_of(x.shares)).collect();
        assert_eq!(before, [2, 2, 2]);
        p.begin_epoch(0).unwrap();
        assert_eq!((p.shares, p.staked, p.epoch_rates[0]), (7, 7, (7, 10)));
        for x in pos.iter_mut() {
            p.touch(x, 0).unwrap();
        }
        let after: Vec<u64> = pos.iter().map(|x| p.amba_of(x.shares)).collect();
        assert_eq!(after, before, "worth what they were");
        // 2 + 2 + 2 of the 7 shares: one is nobody's (rounding dust).
        assert_eq!(pos.iter().map(|x| x.shares).sum::<u64>(), 6);
        // A position already of the current epoch is left as it is.
        let mut cur = pos[0];
        p.touch(&mut cur, 0).unwrap();
        assert_eq!(cur, pos[0]);
    }

    /// A position untouched through several rollovers converts through each
    /// one's rate in turn, and earns in every epoch on the way.
    #[test]
    fn conversions_chain_through_the_kept_ends() {
        let mut p = pool();
        p.staked = 1_000;
        p.shares = 1_000;
        let mut old = StakePosition {
            shares: 400,
            ..StakePosition::default()
        };
        p.spread(1_000).unwrap(); // 400 to `old`
                                  // Epoch 1 starts at 100 atoms of the 1,000 shares: `old` holds 40.
        p.staked = 100;
        p.begin_epoch(0).unwrap();
        p.staked += 900;
        p.shares += 900;
        p.spread(2_000).unwrap(); // 40 / 1,000 of it: 80 to `old`
                                  // Another epoch-0 position, untouched too.
        let mut large = StakePosition {
            shares: 600,
            ..StakePosition::default()
        };
        // Epoch 2 at 50 atoms of epoch 1's 1,000 shares.
        p.staked = 50;
        p.begin_epoch(0).unwrap();
        assert_eq!(p.epoch_rates[..2], [(50, 1_000), (100, 1_000)]);
        p.staked += 950;
        p.shares += 950;
        p.spread(1_000).unwrap(); // one USDC atom a share
        p.touch(&mut old, 0).unwrap();
        // 400 shares (400), then 40 (80), then 2 (2).
        assert_eq!((old.usdc_pending, old.shares, old.epoch), (482, 2, 2));
        p.touch(&mut large, 0).unwrap();
        // 600 shares (600), then 60 (120), then 3 (3).
        assert_eq!((large.usdc_pending, large.shares), (723, 3));
        assert_eq!(p.amba_of(large.shares), 3);
        // Beyond the kept ends its principal still converts, by the running
        // index (1,000 -> 100 -> 5, then 1:1); only its earnings there are
        // lost. The index is binary (2^126 / 10 / 20 rounds down a hair), so
        // the exact 5 comes out a share under: rounding never gives more.
        let mut lost = StakePosition {
            shares: 1_000,
            ..StakePosition::default()
        };
        for _ in 0..EPOCH_ENDS - 1 {
            p.begin_epoch(0).unwrap();
        }
        p.touch(&mut lost, 0).unwrap();
        assert_eq!((lost.usdc_pending, lost.shares), (0, 4));
        assert_eq!(lost.index, p.share_index);
        // A full wipe-out restarts the index: anything older is worth
        // nothing, however far back.
        p.staked = 0;
        p.begin_epoch(0).unwrap();
        assert_eq!((p.share_index, p.wipe_epoch), (INDEX_ONE, p.epoch));
        let mut gone = StakePosition {
            shares: 1_000,
            ..StakePosition::default()
        };
        for _ in 0..EPOCH_ENDS {
            p.staked = 10;
            p.shares = 10;
            p.begin_epoch(0).unwrap();
        }
        p.touch(&mut gone, 0).unwrap();
        assert_eq!(gone.shares, 0);
    }

    /// `mul_div` is `a * b / c` rounded down, in 256 bits.
    #[test]
    fn mul_div_rounds_down_in_256_bits() {
        assert_eq!(mul_div(7, 3, 2), Some(10));
        assert_eq!(mul_div(7, 3, 0), None);
        assert_eq!(mul_div(0, u128::MAX, 1), Some(0));
        // Against `u128` math where the product fits, both divisor paths.
        let mut x = 0x9e37_79b9_7f4a_7c15_f39c_c060_5ced_c834_u128;
        let mut next = |bits: u32| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            match bits {
                128 => x,
                b => x & ((1u128 << b) - 1),
            }
        };
        for i in 0..2_000u32 {
            let (a, b) = (next(1 + i % 64), next(1 + (i / 64) % 64));
            let c = next(1 + i % 128).max(1);
            assert_eq!(mul_wide(a, b), (0, a * b));
            assert_eq!(mul_div(a, b, c), Some(a * b / c), "{a} {b} {c}");
        }
        // Products past 2^128: the quotient times `c` is at most the product
        // and the next quotient's is over it.
        for i in 0..2_000u32 {
            let (a, b) = (next(128), next(64 + i % 65));
            let c = next(64 + i % 65).max(1);
            match mul_div(a, b, c) {
                Some(q) => {
                    assert!(mul_wide(q, c) <= mul_wide(a, b), "{a} {b} {c}");
                    if let Some(q1) = q.checked_add(1) {
                        assert!(mul_wide(q1, c) > mul_wide(a, b), "{a} {b} {c}");
                    }
                }
                None => assert!(mul_wide(a, b).0 >= c, "{a} {b} {c}"),
            }
        }
        // Known 256-bit cases (checked with Python's integers).
        let m = u128::MAX;
        assert_eq!(mul_wide(m, m), (m - 1, 1));
        assert_eq!(mul_div(m, m, m), Some(m));
        assert_eq!(mul_div(m, m - 1, m), Some(m - 1));
        assert_eq!(mul_div(m, m, m - 1), None, "over u128");
        assert_eq!(mul_div(m, 2, 1), None);
        assert_eq!(mul_div(1 << 127, 1 << 127, 1 << 127), Some(1 << 127));
        let (a, b) = ((1u128 << 100) + 7, (1u128 << 90) + 3);
        assert_eq!(
            mul_div(a, b, (1 << 64) + 1),
            Some(85_070_591_730_234_615_861_231_966_046_142_857_216)
        );
        assert_eq!(
            mul_div(a, b, (1 << 64) - 1),
            Some(85_070_591_730_234_615_870_455_338_082_997_633_024)
        );
        assert_eq!(
            mul_div(3u128.pow(80), 5u128.pow(50), 7u128.pow(45)),
            Some(122_684_244_754_218_009_249_635_313_156_626_970)
        );
        assert_eq!(
            mul_div(INDEX_ONE, 12_345_678_901_234_567, 98_765_432_109_876_543),
            Some(10_633_823_869_378_605_347_739_146_715_586_660_850)
        );
        assert_eq!(
            mul_div(INDEX_ONE, 1, 1_000_000_000_000_000_000),
            Some(85_070_591_730_234_615_865)
        );
    }

    /// Codex's review of the index (2026-10-18): 10^18 shares to one atom,
    /// then rollovers each leaving about 1%. At 10^18 the index was 1 after
    /// the first and rounded to nothing at the next, wiping a position still
    /// worth ~10^7 atoms. At 2^126 a position staked at the tiny index
    /// converts beyond the kept ends to what the chain through them gives.
    #[test]
    fn a_tiny_index_still_converts_beyond_the_kept_ends() {
        let mut p = pool();
        p.staked = 1;
        p.shares = 1_000_000_000_000_000_000;
        p.begin_epoch(0).unwrap();
        assert_eq!(p.share_index, 85_070_591_730_234_615_865);
        let received = 100_000_000_000_000_000;
        let shares = p.shares_for(received).unwrap();
        assert_eq!(shares, received);
        p.staked += received;
        p.shares += shares;
        let mut ring = StakePosition {
            shares,
            epoch: p.epoch,
            index: p.share_index,
            ..StakePosition::default()
        };
        let mut far = ring;
        let wipe = p.wipe_epoch;
        for k in 0..5 {
            // About 1% left, then a stake whose shares don't fit.
            p.staked = p.shares / 100 + k;
            assert!(p.rolls_over(1_000_000_000_000_000_000));
            p.begin_epoch(0).unwrap();
            // Touched at every rollover: the chain through the kept ends.
            p.touch(&mut ring, 0).unwrap();
        }
        assert_eq!(p.wipe_epoch, wipe, "no wipe-out");
        p.touch(&mut far, 0).unwrap();
        assert_eq!((far.epoch, far.index), (p.epoch, p.share_index));
        assert_eq!(ring.shares, 10_000_003);
        assert!(far.shares.abs_diff(ring.shares) <= 1, "{}", far.shares);
        assert_eq!(p.amba_of(far.shares), 10_000_003, "~10^7 atoms");
    }

    /// Rounding the index to nothing wipes out only what is worth under an
    /// atom: two rollovers of 2^64 - 1 shares to one atom each.
    #[test]
    fn an_index_rounded_to_nothing_held_less_than_an_atom() {
        let mut p = pool();
        p.staked = 1;
        p.shares = u64::MAX;
        p.begin_epoch(0).unwrap();
        let received = u64::MAX - 1;
        p.shares += p.shares_for(received).unwrap();
        p.staked += received;
        assert_eq!((p.staked, p.shares), (u64::MAX, u64::MAX));
        let mut ring = StakePosition {
            shares: received,
            epoch: p.epoch,
            index: p.share_index,
            ..StakePosition::default()
        };
        let mut far = ring;
        p.staked = 1;
        p.begin_epoch(0).unwrap();
        assert_eq!((p.share_index, p.wipe_epoch), (INDEX_ONE, p.epoch));
        p.touch(&mut ring, 0).unwrap();
        assert_eq!(ring.shares, 0, "under an atom by the kept rate too");
        for _ in 0..EPOCH_ENDS {
            p.staked = 10;
            p.shares = 10;
            p.begin_epoch(0).unwrap();
        }
        p.touch(&mut far, 0).unwrap();
        assert_eq!(far.shares, 0);
    }

    /// A pool from before the rates (`STAKE_V1_LEN`) reads them as zero and
    /// writes what fits; the full size keeps them.
    #[test]
    fn a_legacy_pool_reads_and_writes_without_rates() {
        let mut p = pool();
        p.staked = 7;
        p.shares = 10;
        p.rewards = Some(Rewards {
            acc: 5,
            amount: 9,
            ..Rewards::default()
        });
        let mut v1 = vec![0u8; STAKE_V1_LEN];
        p.write(&mut v1);
        // (Shorter than `STAKE_LEN`: no count of its credit markets.)
        let uncounted = |p: StakePool| StakePool {
            credit_counted: false,
            ..p
        };
        assert_eq!(StakePool::read(&v1).unwrap(), uncounted(p));
        p.begin_epoch(0).unwrap();
        let mut full = vec![0u8; STAKE_LEN];
        p.write(&mut full);
        assert_eq!(StakePool::read(&full).unwrap(), p);
        // The head and stream are where they were: the full account cut to
        // the legacy size reads the same but for the rates.
        let legacy = StakePool::read(&full[..STAKE_V1_LEN]).unwrap();
        assert_eq!(
            legacy,
            StakePool {
                epoch_rates: Default::default(),
                share_index: 0,
                wipe_epoch: p.epoch,
                ..uncounted(p)
            }
        );
        p.write(&mut v1);
        assert_eq!(v1[..], full[..STAKE_V1_LEN]);
        assert!(StakePool::read(&full[..STAKE_V1_LEN - 1]).is_err());
        assert!(StakePool::read(&[full.clone(), vec![0]].concat()).is_err());
    }

    /// Unstaking shares (the user, 2026-10-18: a 7-day timer) back no new
    /// credit, yet stay staked: the AMBA left after an unstake still counts
    /// all of them. Their timer ends `UNBOND_SECS` after its start.
    #[test]
    fn unstaking_shares_back_no_new_credit() {
        let mut p = pool();
        let price = 20_000 * PRICE_SCALE as u64;
        let t = 10_000;
        p.post(price, t);
        p.first_price_ts = 1;
        // 1,000 AMBA at $0.02, haircut 50%: $10.
        p.staked = 1_000_000_000_000;
        p.shares = 1_000_000_000_000;
        let full = p.capacity(t).unwrap();
        assert_eq!(full, 10_000_000);
        p.unbonding = p.shares / 4;
        assert_eq!(p.capacity(t), Some(full * 3 / 4));
        assert_eq!(p.capacity_of(p.staked, t), Some(full), "still staked");
        // Never more than every share, however it is counted.
        p.unbonding = u64::MAX;
        assert_eq!(p.capacity(t), Some(0));
        let mut pos = StakePosition {
            shares: 10,
            ..StakePosition::default()
        };
        assert_eq!(p.unbond_ends(&pos), None, "nothing unstaking");
        pos.unbonding = 4;
        pos.unbond_ts = 1_000;
        assert_eq!(p.unbond_ends(&pos), Some(1_000 + UNBOND_SECS));
        assert_eq!(UNBOND_SECS, 7 * 86_400);
    }

    /// A rollover converts unstaking shares as it converts every share: the
    /// pool's at the rollover's rate, a position's through its chain of
    /// kept ends or the running index, never above its shares; a full
    /// wipe-out leaves none.
    #[test]
    fn unstaking_shares_convert_at_a_rollover() {
        let mut p = pool();
        p.staked = 7;
        p.shares = 10;
        p.unbonding = 4;
        let mut a = StakePosition {
            shares: 6,
            unbonding: 4,
            unbond_ts: 9,
            ..StakePosition::default()
        };
        let mut b = StakePosition {
            shares: 4,
            ..StakePosition::default()
        };
        p.begin_epoch(0).unwrap();
        assert_eq!((p.shares, p.unbonding), (7, 2));
        p.touch(&mut a, 0).unwrap();
        p.touch(&mut b, 0).unwrap();
        assert_eq!((a.shares, a.unbonding, a.unbond_ts), (4, 2, 9));
        assert_eq!((b.shares, b.unbonding), (2, 0));
        // Further back than the kept ends: by the index, capped at its
        // shares (1,000 -> 500, then 1:1).
        let mut q = pool();
        q.staked = 1_000;
        q.shares = 1_000;
        q.unbonding = 1_000;
        let mut far = StakePosition {
            shares: 1_000,
            unbonding: 1_000,
            ..StakePosition::default()
        };
        q.staked = 500;
        q.begin_epoch(0).unwrap();
        assert_eq!(q.unbonding, 500);
        for _ in 0..EPOCH_ENDS {
            q.begin_epoch(0).unwrap();
        }
        q.touch(&mut far, 0).unwrap();
        assert_eq!((far.shares, far.unbonding), (500, 500));
        assert_eq!(q.unbonding, 500);
        // A full wipe-out: nothing left unstaking.
        q.staked = 0;
        q.begin_epoch(0).unwrap();
        q.touch(&mut far, 0).unwrap();
        assert_eq!((q.unbonding, far.shares, far.unbonding), (0, 0, 0));
    }

    /// The timer's fields: a pool of each size (704, 784, 792, 800 bytes)
    /// and a position of either (192, 208) read; the older ones read nothing
    /// unstaking and write what fits.
    #[test]
    fn the_timer_reads_and_writes_every_size() {
        let mut p = pool();
        p.staked = 7;
        p.shares = 10;
        p.epoch_rates[0] = (3, 4);
        p.share_index = 5;
        p.unbonding = 6;
        let mut full = vec![0u8; STAKE_LEN];
        p.write(&mut full);
        assert_eq!(full.len(), 800);
        assert_eq!(StakePool::read(&full).unwrap(), p);
        assert_eq!(u64_at(&full, 784), 6);
        let v3 = StakePool::read(&full[..STAKE_V3_LEN]).unwrap();
        assert_eq!(STAKE_V3_LEN, 792);
        let uncounted = StakePool {
            credit_counted: false,
            ..p
        };
        assert_eq!(v3, uncounted);
        let v2 = StakePool::read(&full[..STAKE_V2_LEN]).unwrap();
        assert_eq!(STAKE_V2_LEN, 784);
        assert_eq!(
            v2,
            StakePool {
                unbonding: 0,
                ..uncounted
            }
        );
        let mut cut = vec![0u8; STAKE_V2_LEN];
        p.write(&mut cut);
        assert_eq!(cut[..], full[..STAKE_V2_LEN]);
        for len in [
            STAKE_V1_LEN - 1,
            STAKE_V1_LEN + 1,
            STAKE_V2_LEN + 1,
            STAKE_V3_LEN + 1,
            STAKE_LEN + 1,
        ] {
            let mut b = vec![0u8; len];
            b[..8].copy_from_slice(&STAKE_MAGIC);
            assert!(StakePool::read(&b).is_err(), "{len}");
        }
        let pos = StakePosition {
            shares: 10,
            epoch: 2,
            index: 3,
            unbonding: 4,
            unbond_ts: 5,
            ..StakePosition::default()
        };
        let mut b = vec![0u8; STAKE_POS_LEN];
        pos.write(&mut b);
        assert_eq!(b.len(), 208);
        assert_eq!((u64_at(&b, 192), u64_at(&b, 200)), (4, 5));
        assert_eq!(StakePosition::read(&b).unwrap(), pos);
        let old = StakePosition::read(&b[..STAKE_POS_V1_LEN]).unwrap();
        assert_eq!(
            old,
            StakePosition {
                unbonding: 0,
                unbond_ts: 0,
                ..pos
            }
        );
        let mut v1 = vec![0u8; STAKE_POS_V1_LEN];
        pos.write(&mut v1);
        assert_eq!(v1[..], b[..STAKE_POS_V1_LEN]);
        assert!(StakePosition::read(&b[..STAKE_POS_V1_LEN - 1]).is_err());
        assert!(StakePosition::read(&[b.clone(), vec![0]].concat()).is_err());
    }

    /// The count of the markets holding the pool's credit (Codex's review of
    /// the unstake timer, 2026-10-18): at 792 (u32) and 796 (complete, a
    /// byte) of an 800-byte pool; an older pool reads it as unknown, known
    /// again once nothing is lent.
    #[test]
    fn the_credit_markets_count_reads_every_size() {
        let mut p = pool();
        p.credit_markets = 3;
        p.allocated = 9;
        let mut full = vec![0u8; STAKE_LEN];
        p.write(&mut full);
        assert_eq!((u32_at(&full, 792), full[796]), (3, 1));
        assert_eq!(full[797..800], [0, 0, 0]);
        assert_eq!(StakePool::read(&full).unwrap(), p);
        assert!(p.counted());
        // Grown from 792 bytes: lent in markets it may not count.
        let mut old = StakePool::read(&full[..STAKE_V3_LEN]).unwrap();
        assert_eq!((old.credit_markets, old.credit_counted), (0, false));
        assert!(!old.counted());
        old.credit_markets = 1;
        old.write(&mut full);
        assert_eq!((u32_at(&full, 792), full[796]), (1, 0));
        // Once nothing is lent its count is complete, and written so.
        old.allocated = 0;
        assert!(old.counted());
        old.write(&mut full);
        assert_eq!(full[796], 1);
        assert!(StakePool::read(&full).unwrap().credit_counted);
    }

    /// Which markets hold unstaking (`Unstake` takes the pool's counted
    /// ones): from the earlier of `credit_until` and the expiry less the
    /// pool's window, until the credit settles.
    #[test]
    fn a_market_near_expiry_holds_unstaking() {
        let vault = Pubkey::new_unique();
        let mut h = Header {
            expiry_ts: 100_000,
            credit_owner: vault,
            credit_until: 100_000 - 86_400,
            credit_flags: CREDIT_COUNTED,
            ..Header::default()
        };
        assert!(h.counts_credit_of(&vault));
        assert!(!h.counts_credit_of(&Pubkey::new_unique()));
        assert!(!h.holds_unstaking(86_400, 100_000 - 86_401));
        assert!(h.holds_unstaking(86_400, 100_000 - 86_400));
        assert!(h.holds_unstaking(86_400, 200_000), "past expiry, unsettled");
        // A longer window than its `credit_until` holds from the window.
        assert!(h.holds_unstaking(90_000, 100_000 - 90_000));
        // A shorter one from its `credit_until`.
        assert!(h.holds_unstaking(3_600, 100_000 - 86_400));
        h.credit_flags = CREDIT_SETTLED;
        assert!(!h.holds_unstaking(86_400, 200_000));
        assert!(!h.counts_credit_of(&vault));
        h.credit_flags = 0;
        assert!(!h.counts_credit_of(&vault), "not counted");
    }

    /// The timer's instructions are the next tags after `PoolPrice` (40).
    #[test]
    fn the_unstake_timer_tags() {
        let tag = |ix: crate::FuturesInstruction| borsh::to_vec(&ix).unwrap()[0];
        assert_eq!(tag(crate::FuturesInstruction::PoolPrice), 40);
        assert_eq!(
            tag(crate::FuturesInstruction::StartUnstake { shares: 1 }),
            41
        );
        assert_eq!(tag(crate::FuturesInstruction::CancelUnstake), 42);
        assert_eq!(tag(crate::FuturesInstruction::Unstake { shares: 1 }), 32);
    }

    /// A stale price lends nothing; in the production build neither does a
    /// pool's first hour of prices (the user, 2026-10-18).
    #[test]
    fn a_stale_or_new_price_lends_nothing() {
        let mut p = pool();
        p.staked = 1_000_000_000_000;
        let price = 20_000 * PRICE_SCALE as u64;
        p.post(price, 1_000);
        let full = p.usdc_of(p.staked, price).unwrap() / 2;
        #[cfg(not(feature = "operator-price"))]
        assert_eq!(p.capacity(1_000 + MAX_PRICE_AGE - 1), Some(0), "too new");
        let t = 1_000 + MAX_PRICE_AGE;
        p.post(price, t);
        assert_eq!(p.capacity(t), Some(full));
        assert!(p.price_fresh(t + MAX_PRICE_AGE));
        assert_eq!(p.capacity(t + MAX_PRICE_AGE + 1), Some(0), "stale");
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
