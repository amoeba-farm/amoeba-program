//! Ameba futures market (dev build).
//!
//! One market account per index and expiry holds every trader balance, resting
//! order, maker ladder and claim, bound to an oracle `Period` and one classic
//! SPL USDC vault. Users buy or sell range claims: a long on `[lo, hi)` pays
//! `clamp(S - lo, 0, hi - lo)` points per lot at settlement `S`, a short the
//! rest of the complete set. Different leverages and ranges trade against each
//! other through balanced packages that any client proposes and this program
//! verifies against the complete book (`ameba-futures-math`): best total price,
//! then the most lots, then older-first FIFO by arrival sequence.
//!
//! Every package is fully funded: a complete set's backing is paid in by its
//! long's premium and its short's contribution when it is created, and is
//! released only by settlement or by the redemption of an exact complement.
//! There are no protocol fees and no funding payments.
//!
//! Not audited; dev chain only. See `docs/futures.md`.
pub mod heap;
pub mod processor;
pub mod state;

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{program_error::ProgramError, pubkey::Pubkey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum FuturesError {
    InvalidInstruction = 1,
    InvalidAccount,
    Unauthorized,
    /// `Submit` or `Quote` on a market that is not open, or whose oracle
    /// period is not Live inside its trading window (before its listing,
    /// Forming, at or after expiry, Settled or Failed).
    NotTradable,
    InvalidShape,
    InvalidPrice,
    InsufficientFunds,
    TraderCapacity,
    OrderCapacity,
    ClaimCapacity,
    MakerCapacity,
    /// The proposal does not decode or names liquidity that is not in the book.
    BadProposal,
    /// The package is unbalanced or over an order's lots.
    Unbalanced,
    /// A cheaper package exists for the taker.
    NotBestPrice,
    /// The taker could fill more lots at its limit.
    NotMaxFill,
    /// An older order could fill more at the same price.
    FifoViolation,
    /// The route search exceeded the supported work limit.
    SearchLimit,
    /// Fewer lots than `min_fill`.
    MinFill,
    /// The order would cross the book (post-only and maker ladders).
    WouldCross,
    /// The claim is not the exact complement (or not the caller's).
    NotComplement,
    ClaimNotFound,
    OrderNotFound,
    NotSettled,
    AlreadyFinal,
    Overflow,
    Custody,
    /// The book's own invariants failed (backing or cash conservation).
    Insolvent,
    LimitExceeded,
    NotExpired,
    ClaimReserved,
    /// The Earn vault is in as many markets as one transaction can name
    /// (`state::VAULT_MARKETS`).
    VaultCapacity,
}

pub const ERROR_BASE: u32 = 0x4652_0000;

impl From<FuturesError> for ProgramError {
    fn from(e: FuturesError) -> Self {
        ProgramError::Custom(ERROR_BASE + e as u32)
    }
}

/// `Submit` modes.
pub const MODE_IOC: u8 = 0;
/// Match, then rest the remainder.
pub const MODE_GTC: u8 = 1;
/// Rest only; fail if anything would fill.
pub const MODE_POST: u8 = 2;

/// No claim (an ordinary opening order).
pub const NO_CLAIM: u32 = u32::MAX;

/// One knot of a maker ladder's mid curve: the premium per lot of the
/// one-step claim `[pos, pos+1)`.
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Knot {
    pub pos: u16,
    pub price: u32,
}

/// The Earn vault: `Submit`, `Cancel`, `Quote`, `CancelQuote`, `Split`,
/// `Merge` and `Redeem` also take the vault account as their owner (not a
/// signer) when the vault's operator signs as any later account and the
/// market is one of the vault's; they then act for the vault's trader there.
/// `Withdraw` and `Transfer` never do: vault cash moves only between the
/// vault and its markets (`VaultFund`, `VaultDefund`) and leaves only to
/// depositors (`VaultWithdraw`).
///
/// Legacy per-market Earn pools (dev only, feature `legacy-pools`):
/// `CreatePool` and `PoolDeposit` are retired everywhere; `PoolWithdraw`,
/// `PoolMigrate` and the operator's `Cancel`/`CancelQuote` for a pool remain
/// in dev builds so the dev chain's pool shares can leave.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub enum FuturesInstruction {
    /// Creates a market for one index and expiry, bound to its oracle period.
    /// The period may still be Forming: the market then takes no trade until
    /// the oracle lists it (Live), so markets can be listed ahead.
    ///
    /// Accounts: 0 market (w; zeroed, owned by this program, exactly
    /// `state::market_len(..)` bytes); 1 registry (w, PDA
    /// `["ameba-range-v1", "registry", product_id, expiry_ts]`); 2 period;
    /// 3 oracle program; 4 quote mint; 5 vault (token account of the mint
    /// owned by the authority PDA `["ameba-range-v1", "authority", market]`);
    /// 6 creator (s, w; this program's upgrade authority); 7 system program;
    /// 8 this program; 9 its ProgramData.
    CreateMarket {
        product_id: [u8; 32],
        expiry_ts: u64,
        /// Highest grid position; positions are `0..=grid`.
        grid: u16,
        /// Atoms one lot of a one-step claim pays in full.
        k: u64,
        /// Oracle value units per grid step (at `value_decimals`).
        step_value: u64,
        value_decimals: u8,
        /// Reference index value at listing (display and quoting); never a
        /// settlement: a failed period settles through `CouncilSettle`.
        ref_value: u64,
        trader_cap: u16,
        order_cap: u16,
        maker_cap: u8,
        claim_cap: u32,
    },
    /// Accounts: 0 market (w); 1 owner (s); 2 owner token (w); 3 vault (w);
    /// 4 token program.
    Deposit { amount: u64 },
    /// Pays free balance only; `u64::MAX` = all of it. Accounts as `Deposit`,
    /// then 5 the vault authority PDA.
    Withdraw { amount: u64 },
    /// Trades one claim shape against the book with a verified package, then
    /// (GTC/post-only) rests the remainder. Only while the market's period
    /// is Live and before expiry (`NotTradable` otherwise; the user,
    /// 2026-10-15, preflight Decisions 5).
    ///
    /// Accounts: 0 market (w); 1 owner (s); 2 period.
    Submit {
        side: u8,
        lo: u16,
        hi: u16,
        lots: u64,
        /// Long: most premium per lot. Short: least premium per lot.
        limit: u64,
        min_fill: u64,
        mode: u8,
        /// A claim of the owner this order closes (`NO_CLAIM` for none): its
        /// exact complement. A resting close order reserves the claim instead
        /// of cash.
        close_claim: u32,
        close_seq: u64,
        /// `ameba_futures_math::codec::Proposal`, encoded.
        proposal: Vec<u8>,
    },
    /// Accounts: 0 market (w); 1 owner (s).
    Cancel { slot: u16, seq: u64 },
    /// Publishes the owner's maker ladder over `[lo, hi)`: per step, bids from
    /// `mid - bid_spread` and asks from `mid + ask_spread` with the given lots
    /// in depth tiers, fully escrowed. Post-only: the proposal carries the
    /// book's potentials. Only while trading is, as `Submit`.
    ///
    /// Accounts: 0 market (w); 1 owner (s); 2 period.
    Quote {
        lo: u16,
        hi: u16,
        knots: Vec<Knot>,
        bid_spread: u32,
        ask_spread: u32,
        bid_lots: u64,
        ask_lots: u64,
        /// Keep consumption: each step's lots become at most what is left.
        keep: bool,
        /// Depth tiers per step and side (0 or 1: one price), up to
        /// `MAX_TIERS`: tier `i` sits `i * tier_spread` atoms further from
        /// the mid and holds `tier_ratio` percent of tier `i - 1`'s lots.
        tiers: u8,
        tier_ratio: u16,
        tier_spread: u32,
        /// The dense part `[core_lo, core_hi)` within `[lo, hi)`: its steps
        /// quote every tier; the other steps quote only the last (widest)
        /// tier's lots, a thinner, wider ladder for wide (low-leverage)
        /// ranges. `[lo, hi)`: every step quotes every tier.
        core_lo: u16,
        core_hi: u16,
        proposal: Vec<u8>,
    },
    /// Accounts: 0 market (w); 1 owner (s).
    CancelQuote,
    /// Splits a claim at `at` into `[lo, at)` and `[at, hi)`, same lots.
    /// Accounts: 0 market (w); 1 owner (s).
    Split { claim: u32, seq: u64, at: u16 },
    /// Merges two of the owner's claims on one side: identical intervals, or
    /// adjacent ones with equal lots. Accounts as `Split`.
    Merge {
        keep: u32,
        keep_seq: u64,
        absorb: u32,
        absorb_seq: u64,
    },
    /// Redeems `lots` of an exact complement pair (same interval) for its
    /// backing. Accounts as `Split`.
    Redeem {
        long: u32,
        long_seq: u64,
        short: u32,
        short_seq: u64,
        lots: u64,
    },
    /// Gives `lots` of a claim to `to` unchanged. `to` must be a wallet key
    /// (on the ed25519 curve), never a program address such as an Earn pool.
    /// Accounts as `Split`.
    Transfer {
        claim: u32,
        seq: u64,
        lots: u64,
        to: Pubkey,
    },
    /// Permissionless: settles at a Settled period's value, at or after
    /// expiry. A failed period settles only through `CouncilSettle`.
    /// Accounts: 0 market (w); 1 period.
    Finalize,
    /// Permissionless after finalization: pays claims into free balances.
    /// Accounts: 0 market (w).
    Settle { claims: Vec<u32> },
    /// Permissionless after finalization: pays a maker's ladder inventory and
    /// releases its escrow. Accounts: 0 market (w).
    SettleMaker { maker: u8 },
    /// Permissionless after finalization: releases resting orders' escrow.
    /// Accounts: 0 market (w).
    Prune { orders: Vec<u16> },
    /// Retired: per-market Earn pools gave way to the one Earn vault
    /// (`CreateVault`). Always `InvalidInstruction`; kept so the later
    /// variants keep their tags.
    CreatePool,
    /// Retired with `CreatePool`: always `InvalidInstruction`.
    PoolDeposit { amount: u64 },
    /// Legacy pools' exit (dev builds, feature `legacy-pools`; elsewhere
    /// `InvalidInstruction`): redeems shares for cash at the pool's lowest
    /// value over every settlement outcome, paid from the pool's free cash;
    /// `u64::MAX` = all. The pool's last shares wait while it may still pay
    /// more (`NotSettled`).
    ///
    /// Accounts: 0 market (w); 1 pool (w); 2 position (w); 3 owner (s);
    /// 4 owner token (w); 5 vault (w); 6 token program; 7 vault authority.
    PoolWithdraw { shares: u64 },
    /// Settles a market whose oracle period failed, is neither settled nor
    /// failed at or after expiry (any time after expiry: the user,
    /// 2026-10-16), or settled in decimals
    /// other than the market's (it can't finalize), at `value` (oracle
    /// units at the market's `value_decimals`). The status becomes Failed;
    /// claims then pay as after `Finalize`. Only the program's upgrade
    /// authority signs alone; a governed program needs three council seats.
    /// An immutable program has no one to set it.
    ///
    /// Accounts: 0 market (w); 1 period; 2 authority (s); 3 this program;
    /// 4 its ProgramData; then, when governed, the council config and three
    /// seats (s).
    CouncilSettle { value: u64 },
    /// Creates the Earn vault `operator` runs, over `cash_account`: a token
    /// account of the quote mint owned by the vault's address, empty.
    /// Permissionless: whoever signs as operator runs that vault.
    ///
    /// Accounts: 0 vault (w, PDA `["ameba-range-v1", "earn", operator]`);
    /// 1 operator (s); 2 payer (s, w); 3 cash account; 4 quote mint;
    /// 5 system program.
    CreateVault,
    /// Deposits `amount` for vault shares priced at the vault's highest
    /// value over every settlement outcome (idle cash plus its value in
    /// every market), so a deposit never dilutes the others.
    ///
    /// Accounts: 0 vault (w); 1 position (w, PDA
    /// `["ameba-range-v1", "earn-lp", vault, owner]`, created on first use);
    /// 2 owner (s); 3 owner token (w); 4 cash account (w); 5 token program;
    /// 6 system program; 7 payer (s, w); then every market of the vault, in
    /// its order.
    VaultDeposit { amount: u64 },
    /// Redeems `shares` (`u64::MAX` = all) for cash at the vault's lowest
    /// value over every settlement outcome, paid from its idle cash and then
    /// from its free cash in the markets named after the markets list, in
    /// order; where that is short, the vault's ladder in that market is
    /// retired first to release its escrow (depositors come before the
    /// maker's quotes; the operator quotes again). The vault's last shares
    /// wait while it may still pay more (`NotSettled`).
    ///
    /// Accounts: 0 vault (w); 1 position (w); 2 owner (s); 3 owner token
    /// (w); 4 cash account (w); 5 token program; then every market of the
    /// vault, in its order (writable where drawn from); then any number of
    /// pairs (market token vault (w), market authority) to draw from.
    VaultWithdraw { shares: u64 },
    /// The operator moves `amount` of the vault's idle cash into a market's
    /// trader slot of the vault, adding the market to the vault's markets.
    ///
    /// Accounts: 0 market (w); 1 vault (w); 2 cash account (w); 3 market
    /// token vault (w); 4 token program; 5 operator (s).
    VaultFund { amount: u64 },
    /// The operator moves `amount` (`u64::MAX` = all) of the vault's free
    /// cash in a market back to its idle cash; a market where the vault
    /// then holds nothing leaves its markets.
    ///
    /// Accounts: 0 market (w); 1 vault (w); 2 cash account (w); 3 market
    /// token vault (w); 4 token program; 5 market authority; 6 operator (s).
    VaultDefund { amount: u64 },
    /// Legacy pools (dev builds, feature `legacy-pools`): moves a pool whose
    /// shares one holder owns into the vault. The pool's trader slot (cash,
    /// claims, ladder) becomes the vault's in that market, and the holder
    /// gets vault shares for its lowest value at the vault's highest. The
    /// pool and the vault have the same operator, who signs.
    ///
    /// Accounts: 0 market (w); 1 pool (w); 2 holder's pool position (w);
    /// 3 vault (w); 4 holder's vault position (w, created on first use);
    /// 5 holder (s); 6 operator (s); 7 payer (s, w); 8 system program; then
    /// every market of the vault, in its order.
    PoolMigrate,
}

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(entry);

#[cfg(not(feature = "no-entrypoint"))]
fn entry(
    program_id: &Pubkey,
    accounts: &[solana_program::account_info::AccountInfo],
    data: &[u8],
) -> solana_program::entrypoint::ProgramResult {
    processor::process(program_id, accounts, data)
}
