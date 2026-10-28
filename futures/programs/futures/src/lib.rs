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
//! There are no protocol fees and no funding payments. The one exception
//! (2026-10-17, the user's call): an Earn vault's maker may borrow what its
//! stake pool lends it in a market, the stakers' AMBA carrying the borrowed
//! part (`docs/futures.md`, "Staked AMBA backs the maker").
//!
//! Not audited; dev chain only. See `docs/futures.md`.
pub mod heap;
pub mod price_source;
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
    /// Forming, at or after expiry, Settled or Failed); `CreditDefund` on a
    /// market that is not open.
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
    /// Not yet: it waits for a settlement (the oracle's, the market's, its
    /// credit's), a fresh price, or the positions a pool's last shares hold;
    /// or `Unstake` before the position's unstake timer is done
    /// (`state::UNBOND_SECS` after its last `StartUnstake`).
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
    /// The stake-backed credit can't carry it: the credit owner's exposure
    /// over its market's limit (after its own orders, or a fill of them),
    /// its debt over what carries it there (backing plus its own cash: any
    /// instruction that would leave more), credit over the pool's capacity,
    /// or unstaking AMBA that backs lent credit.
    CreditLimit,
    /// The market's winners are paid part in AMBA: `SettleRecovery`. Also
    /// `Transfer` and `Redeem` after finalization while the market's credit
    /// isn't settled or settled short: claims leave only through settlement.
    NeedsRecovery,
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
    /// 2026-10-15, preflight Decisions 5). The fill's claim nets before it
    /// takes a claim slot, so a close never needs a free one; a resting
    /// opening order holds a slot for its fills (`ClaimCapacity` when none
    /// is free; `docs/futures.md`, "Claim slots").
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
    /// Splits a claim at `at` into `[lo, at)` and `[at, hi)`, same lots. A
    /// claim a resting order holds moves out whole: both parts take free
    /// slots and the order keeps its slot.
    /// Accounts: 0 market (w); 1 owner (s).
    Split { claim: u32, seq: u64, at: u16 },
    /// Merges two of the owner's claims on one side: identical intervals, or
    /// adjacent ones with equal lots. When a resting order holds `keep` and
    /// the intervals differ, the merged claim takes a free slot and the order
    /// keeps its slot. Accounts as `Split`.
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
    /// Gives `lots` of a claim to `to` unchanged (they net against and join
    /// `to`'s claims as a fill would). `to` must be a wallet key
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
    /// The operator sets the vault's reward stream (`state::Rewards`):
    /// `amount` atoms of the reward mint released linearly from `start_ts`
    /// to `end_ts` (unix seconds) to depositors by their shares, except
    /// `house`'s (the protocol's own deposit; `Pubkey::default()`: none).
    /// What the previous stream released up to now stays accrued; the rest
    /// of it, and any part of the new one before now, is not released. The
    /// first stream fixes the mint, the reward account (a token account of
    /// the mint, either token program, owned by the vault's address) and
    /// the house, and grows the vault account by `REWARDS_LEN` (rent from
    /// `payer`). The reward account is funded by plain transfers; the
    /// stream doesn't check its balance.
    ///
    /// Accounts: 0 vault (w); 1 operator (s); 2 payer (s, w); 3 reward mint;
    /// 4 reward account; 5 the house's vault position (its address, whether
    /// or not it exists); 6 system program.
    VaultRewards {
        start_ts: u64,
        end_ts: u64,
        amount: u64,
        house: Pubkey,
    },
    /// Permissionless: pays a position's rewards accrued so far into its
    /// owner's token account of the reward mint (checked transfer: the
    /// mint's transfer fee, if any, comes out of what arrives).
    ///
    /// Accounts: 0 vault (w); 1 position (w); 2 reward account (w); 3 reward
    /// mint; 4 owner's token account (w); 5 the mint's token program.
    VaultRewardsClaim,
    /// The operator takes back `amount` (`u64::MAX`: all) of the reward
    /// account's surplus: what it holds beyond what depositors are owed and
    /// what the stream has still to release.
    ///
    /// Accounts: 0 vault (w); 1 operator (s); 2 reward account (w); 3 reward
    /// mint; 4 destination token account (w); 5 the mint's token program.
    VaultRewardsWithdraw { amount: u64 },
    /// Creates the stake pool behind the Earn vault's maker
    /// (`state::StakePool`): stakers' AMBA in `amba account`, their USDC in
    /// `usdc account`, both token accounts owned by the pool's address, empty.
    /// The vault's operator signs and runs it.
    ///
    /// Only the program's upgrade authority creates a pool (`payer`; a
    /// governed program needs three council seats, as `CreateMarket`): a
    /// pool lends credit in any market.
    ///
    /// Accounts: 0 pool (w, PDA `["ameba-range-v1", "stake", vault]`);
    /// 1 vault; 2 operator (s); 3 payer (s, w; the upgrade authority);
    /// 4 AMBA mint; 5 AMBA account; 6 USDC account; 7 system program; 8 this
    /// program; 9 its ProgramData; then, when governed, the council config
    /// and three seats (s).
    CreateStakePool {
        haircut_bps: u16,
        profit_bps: u16,
        buffer_bps: u16,
        twap_window: u64,
        expiry_window: u64,
    },
    /// The operator changes the pool's parameters (as `CreateStakePool`).
    /// Accounts: 0 pool (w); 1 operator (s).
    StakeConfig {
        haircut_bps: u16,
        profit_bps: u16,
        buffer_bps: u16,
        twap_window: u64,
        expiry_window: u64,
    },
    /// The operator posts the AMBA price: USDC atoms per whole AMBA, times
    /// `state::PRICE_SCALE`. The pool's credit and shortfalls use the
    /// time-weighted average of the posts (the user, 2026-10-17). Dev
    /// builds only (`operator-price`, as the dev chain's has it): the
    /// production build refuses it (`Unauthorized`) and takes the price from
    /// mainnet's pool through `PoolPrice` (the user, 2026-10-18).
    /// Accounts: 0 pool (w); 1 operator (s).
    StakePrice { price: u64 },
    /// Stakes `amount` AMBA atoms (a checked transfer: the mint's transfer
    /// fee comes out of what arrives, and shares are for what arrives).
    /// It starts a new share epoch when a shortfall took every staked atom
    /// or the shares wouldn't fit a `u64` (`StakePool::rolls_over`).
    ///
    /// Accounts: 0 pool (w); 1 position (w, PDA `["ameba-range-v1",
    /// "stake-pos", pool, owner]`, created on first use); 2 owner (s);
    /// 3 owner's AMBA account (w); 4 pool's AMBA account (w); 5 AMBA mint;
    /// 6 its token program; 7 system program; 8 payer (s, w: the position's
    /// rent, and an older pool's growth to `STAKE_LEN` at a rollover).
    Stake { amount: u64 },
    /// Pays `shares` (`u64::MAX`: all) of the position's unstaking shares
    /// for their AMBA once their timer is done: `UNBOND_SECS` (7 days) after
    /// the last `StartUnstake` (`NotSettled` before), as long as the AMBA
    /// left, unstaking AMBA included, still backs the credit lent
    /// (`CreditLimit`: the rest comes out as markets settle and credit
    /// returns). It also waits (`NotSettled`) while any market the pool
    /// lends to is in its last `expiry_window` before expiry (from its
    /// `credit_until`, if earlier) or past expiry with its credit unsettled,
    /// and while a pool from before the count (`StakePool::credit_counted`)
    /// lends anything. Permissionless: any signer may send it (the keeper
    /// completes finished timers), and it pays only into the owner's own
    /// AMBA account; anyone but the owner pays all of the position's
    /// unstaking shares (`Unauthorized` for a part). Nothing to pay:
    /// `InsufficientFunds`.
    ///
    /// Accounts: 0 pool (w); 1 position (w); 2 the owner or anyone (s);
    /// 3 owner's AMBA account (w); 4 pool's AMBA account (w); 5 AMBA mint;
    /// 6 its token program; then every market holding the pool's credit
    /// unsettled (`StakePool::credit_markets` of them, distinct, each with
    /// `Header::counts_credit_of` the pool's vault: `InvalidAccount`
    /// otherwise).
    Unstake { shares: u64 },
    /// Permissionless: pays a position's AMBA stream rewards and USDC into
    /// its owner's own accounts (either part may be nothing).
    ///
    /// Accounts: 0 pool (w); 1 position (w); 2 the stream's reward account
    /// (w); 3 AMBA mint; 4 owner's AMBA account (w); 5 AMBA token program;
    /// 6 pool's USDC account (w); 7 owner's USDC account (w); 8 SPL token
    /// program.
    StakeClaim,
    /// The operator sets the pool's AMBA subsidy stream: `amount` atoms
    /// released linearly from `start_ts` to `end_ts` to stakers by their
    /// shares (as `VaultRewards`, no house). The first fixes the reward
    /// account: an AMBA account owned by the pool, not its staked one.
    ///
    /// Accounts: 0 pool (w); 1 operator (s); 2 AMBA mint; 3 reward account.
    StakeRewards {
        start_ts: u64,
        end_ts: u64,
        amount: u64,
    },
    /// The operator takes back `amount` (`u64::MAX`: all) of the reward
    /// account's surplus (as `VaultRewardsWithdraw`).
    ///
    /// Accounts: 0 pool (w); 1 operator (s); 2 reward account (w); 3 AMBA
    /// mint; 4 destination (w); 5 AMBA token program.
    StakeRewardsWithdraw { amount: u64 },
    /// The operator lends `amount` more of the pool's credit to the vault's
    /// trader in an open market, within the pool's capacity (the staked AMBA
    /// at its average price, haircut, less what is lent already). The first
    /// counts the market in the pool's `credit_markets` (a pool of
    /// `STAKE_LEN`), marking its header `CREDIT_COUNTED`.
    ///
    /// Accounts: 0 market (w); 1 pool (w); 2 operator (s).
    CreditFund { amount: u64 },
    /// The operator takes back `amount` (`u64::MAX`: all it can) of a
    /// market's credit, down to the vault trader's exposure there (its whole
    /// debt if it holds no trader slot). Open markets only (`NotTradable`).
    /// Accounts as `CreditFund`.
    CreditDefund { amount: u64 },
    /// Permissionless, after finalization and once the vault's own claims,
    /// inventory and orders there are settled (their payouts repay its debt
    /// first): pays the stakers their share of its profit there, or covers
    /// its shortfall from their USDC buffer and then sets their AMBA aside
    /// for the winners at the average price; returns the credit to the pool
    /// and uncounts the market from its `credit_markets`. Everyone else's
    /// settlement waits for it.
    ///
    /// Accounts: 0 market (w); 1 pool (w); 2 market token vault (w);
    /// 3 market authority; 4 pool's USDC account (w); 5 SPL token program.
    CreditSettle,
    /// Permissionless, in a market with a shortfall: settles one owner's
    /// `claims` (and with `maker`, its ladder inventory) paying the USDC
    /// part of each payout into free cash and its share of the AMBA set
    /// aside into the owner's AMBA account.
    ///
    /// Accounts: 0 market (w); 1 pool (w); 2 pool's AMBA account (w); 3 AMBA
    /// mint; 4 owner's AMBA account (w); 5 AMBA token program.
    SettleRecovery { claims: Vec<u32>, maker: bool },
    /// Permissionless: posts the AMBA price from mainnet's own accounts
    /// (`price_source`): the Meteora DAMM v2 AMBA/SOL pool's price times
    /// Pyth's fully verified SOL/USD EMA, at most `PYTH_MAX_AGE` old, into
    /// the pool's average as `StakePrice` would. The pool's AMBA mint must
    /// be mainnet's, and the transaction must hold nothing but this and
    /// compute-budget instructions (`InvalidInstruction`), so a swap in the
    /// same transaction can't move the sample.
    ///
    /// Accounts: 0 pool (w); 1 the Meteora pool (`AMBA_SOL_POOL`); 2 Pyth's
    /// SOL/USD feed (`SOL_USD_FEED`); 3 the instructions sysvar.
    PoolPrice,
    /// Starts unstaking `shares` (`u64::MAX`: all the position's shares not
    /// unstaking yet; more is capped at them; none left:
    /// `InsufficientFunds`): `Unstake` pays them `UNBOND_SECS` (7 days, the
    /// user, 2026-10-18) from now. Adding more restarts the timer for all of
    /// them. They keep earning and bearing shortfalls like every share, but
    /// back no new credit (`StakePool::capacity`), so the maker hands back
    /// what they backed. A pool or position from before the timer grows to
    /// `STAKE_LEN` / `STAKE_POS_LEN` first, the rent from `payer`.
    ///
    /// Accounts: 0 pool (w); 1 position (w); 2 owner (s); 3 payer (s, w);
    /// 4 system program.
    StartUnstake { shares: u64 },
    /// Returns the position's unstaking shares to active (none:
    /// `InsufficientFunds`).
    ///
    /// Accounts: 0 pool (w); 1 position (w); 2 owner (s).
    CancelUnstake,
    /// The Earn vault's operator approves a privately calculated upper
    /// bound on exposure after `action` (a serialized Quote or Submit), or
    /// refreshes the current bound when `action` is empty. The assessment
    /// includes every possible later fill. It is trusted, not a proof.
    /// The program independently enforces carry, custody and accounting.
    ///
    /// `snapshot` binds this program, market address and exact pre-action
    /// account bytes (`state::credit_snapshot`). No settlement range is
    /// included. A changed account requires a new assessment and signature.
    ///
    /// Accounts: the action's usual accounts (0 market, 1 Earn vault), then
    /// its operator (s, w) and the system program. A refresh takes 0 market
    /// (w), 1 Earn vault, 2 operator (s, w), 3 system. The first approval
    /// appends 16 bytes to the market, with rent paid by the operator.
    CreditAuthorize {
        snapshot: [u8; 32],
        exposure: u64,
        action: Vec<u8>,
    },
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
