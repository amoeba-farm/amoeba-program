use crate::{state::*, FuturesError, FuturesInstruction, Knot, MODE_IOC, MODE_POST, NO_CLAIM};
use ameba_futures_math::{
    self as rm,
    codec::Proposal,
    payoff::{contribution, long_per_lot, set_value, settle_fine, short_per_lot},
    ArcId, Side, Taker, VerifyError,
};
use ameba_oracle_interface::{load_period_for, PeriodStatus};
use borsh::BorshDeserialize;
use solana_program::{
    account_info::AccountInfo,
    clock::Clock,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    log::sol_log_data,
    program::{invoke, invoke_signed, set_return_data},
    program_error::ProgramError,
    program_pack::Pack,
    pubkey::Pubkey,
    rent::Rent,
    sysvar::Sysvar,
};
use solana_system_interface::instruction as system_instruction;

type R<T> = Result<T, ProgramError>;

#[cfg(feature = "cu-trace")]
fn trace(stage: &str) {
    solana_program::msg!("cu {}", stage);
    solana_program::log::sol_log_compute_units();
}
#[cfg(not(feature = "cu-trace"))]
fn trace(_: &str) {}

const MAX_INSTRUCTION: usize = 1232;
/// Reachability work (edges scanned) one package verification may use.
/// Capacity for one transaction's compute budget; chosen and measured in
/// `docs/futures.md` (handoff 0.2: a documented finite dev capacity
/// that fails explicitly instead of truncating the search).
pub const WORK_LIMIT: u64 = 30_000;
/// Widest maker ladder, in steps: room for a whole dev grid (200 steps), so
/// one ladder can quote every piece a wide range needs. What one book can
/// verify is bounded by `MAX_BOOK_LADDER_ARCS`.
pub const MAX_LADDER_STEPS: u16 = 256;
/// Most ladder arcs (steps x tiers x sides with lots) in one market's book:
/// the measured capacity of one verification (`r14`: 1,024 ladder arcs and
/// 64 orders), same source as `WORK_LIMIT`.
pub const MAX_BOOK_LADDER_ARCS: usize = 1024;

fn need_signer(a: &AccountInfo) -> R<()> {
    if a.is_signer {
        Ok(())
    } else {
        Err(FuturesError::Unauthorized.into())
    }
}

fn need_writable(a: &AccountInfo) -> R<()> {
    if a.is_writable {
        Ok(())
    } else {
        Err(FuturesError::InvalidAccount.into())
    }
}

fn now_ts() -> R<u64> {
    Ok(u64::try_from(Clock::get()?.unix_timestamp).unwrap_or(0))
}

fn add(a: u64, b: u64) -> R<u64> {
    a.checked_add(b)
        .ok_or_else(|| FuturesError::Overflow.into())
}

fn sub(a: u64, b: u64) -> R<u64> {
    a.checked_sub(b)
        .ok_or_else(|| FuturesError::InsufficientFunds.into())
}

fn mul(a: u64, b: u64) -> R<u64> {
    a.checked_mul(b)
        .ok_or_else(|| FuturesError::Overflow.into())
}

fn to_u64(v: i128) -> R<u64> {
    u64::try_from(v).map_err(|_| FuturesError::Overflow.into())
}

fn side_of(v: u8) -> R<Side> {
    Side::from_u8(v).ok_or_else(|| FuturesError::InvalidShape.into())
}

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let (accounts, data, _) =
        ameba_governance_interface::validate_top_level_envelope(program_id, accounts, data)?;
    if data.len() > MAX_INSTRUCTION {
        return Err(FuturesError::InvalidInstruction.into());
    }
    let ix =
        FuturesInstruction::try_from_slice(data).map_err(|_| FuturesError::InvalidInstruction)?;
    match ix {
        FuturesInstruction::CreateMarket {
            product_id,
            expiry_ts,
            grid,
            k,
            step_value,
            value_decimals,
            ref_value,
            trader_cap,
            order_cap,
            maker_cap,
            claim_cap,
        } => create_market(
            program_id,
            accounts,
            CreateParams {
                product_id,
                expiry_ts,
                grid,
                k,
                step_value,
                value_decimals,
                ref_value,
                trader_cap,
                order_cap,
                maker_cap,
                claim_cap,
            },
        ),
        FuturesInstruction::Deposit { amount } => deposit(program_id, accounts, amount),
        FuturesInstruction::Withdraw { amount } => withdraw(program_id, accounts, amount),
        FuturesInstruction::Submit {
            side,
            lo,
            hi,
            lots,
            limit,
            min_fill,
            mode,
            close_claim,
            close_seq,
            proposal,
        } => submit(
            program_id,
            accounts,
            SubmitParams {
                side,
                lo,
                hi,
                lots,
                limit,
                min_fill,
                mode,
                close_claim,
                close_seq,
            },
            &proposal,
        ),
        FuturesInstruction::Cancel { slot, seq } => cancel(program_id, accounts, slot, seq),
        FuturesInstruction::Quote {
            lo,
            hi,
            knots,
            bid_spread,
            ask_spread,
            bid_lots,
            ask_lots,
            keep,
            tiers,
            tier_ratio,
            tier_spread,
            core_lo,
            core_hi,
            proposal,
        } => quote(
            program_id,
            accounts,
            Ladder {
                lo,
                hi,
                bid_spread,
                ask_spread,
                bid_lots,
                ask_lots,
                keep,
                tiers: Tiers {
                    count: tiers,
                    ratio: tier_ratio,
                    spread: tier_spread,
                },
            },
            Core {
                lo: core_lo,
                hi: core_hi,
            },
            &knots,
            &proposal,
        ),
        FuturesInstruction::CancelQuote => cancel_quote(program_id, accounts),
        FuturesInstruction::Split { claim, seq, at } => split(program_id, accounts, claim, seq, at),
        FuturesInstruction::Merge {
            keep,
            keep_seq,
            absorb,
            absorb_seq,
        } => merge(program_id, accounts, (keep, keep_seq), (absorb, absorb_seq)),
        FuturesInstruction::Redeem {
            long,
            long_seq,
            short,
            short_seq,
            lots,
        } => redeem(
            program_id,
            accounts,
            (long, long_seq),
            (short, short_seq),
            lots,
        ),
        FuturesInstruction::Transfer {
            claim,
            seq,
            lots,
            to,
        } => transfer(program_id, accounts, claim, seq, lots, &to),
        FuturesInstruction::Finalize => finalize(program_id, accounts),
        FuturesInstruction::Settle { claims } => settle(program_id, accounts, &claims),
        FuturesInstruction::SettleMaker { maker } => settle_maker(program_id, accounts, maker),
        FuturesInstruction::Prune { orders } => prune(program_id, accounts, &orders),
        // Retired: the one Earn vault replaced the per-market pools.
        FuturesInstruction::CreatePool | FuturesInstruction::PoolDeposit { .. } => {
            Err(FuturesError::InvalidInstruction.into())
        }
        #[cfg(feature = "legacy-pools")]
        FuturesInstruction::PoolWithdraw { shares } => pool_withdraw(program_id, accounts, shares),
        #[cfg(feature = "legacy-pools")]
        FuturesInstruction::PoolMigrate => pool_migrate(program_id, accounts),
        #[cfg(not(feature = "legacy-pools"))]
        FuturesInstruction::PoolWithdraw { .. } | FuturesInstruction::PoolMigrate => {
            Err(FuturesError::InvalidInstruction.into())
        }
        FuturesInstruction::CouncilSettle { value } => council_settle(program_id, accounts, value),
        FuturesInstruction::CreateVault => create_vault(program_id, accounts),
        FuturesInstruction::VaultDeposit { amount } => vault_deposit(program_id, accounts, amount),
        FuturesInstruction::VaultWithdraw { shares } => {
            vault_withdraw(program_id, accounts, shares)
        }
        FuturesInstruction::VaultFund { amount } => vault_fund(program_id, accounts, amount),
        FuturesInstruction::VaultDefund { amount } => vault_defund(program_id, accounts, amount),
        FuturesInstruction::VaultRewards {
            start_ts,
            end_ts,
            amount,
            house,
        } => vault_rewards(program_id, accounts, (start_ts, end_ts, amount), &house),
        FuturesInstruction::VaultRewardsClaim => vault_rewards_claim(program_id, accounts),
        FuturesInstruction::VaultRewardsWithdraw { amount } => {
            vault_rewards_withdraw(program_id, accounts, amount)
        }
        FuturesInstruction::CreateStakePool {
            haircut_bps,
            profit_bps,
            buffer_bps,
            twap_window,
            expiry_window,
        } => create_stake_pool(
            program_id,
            accounts,
            StakeParams {
                haircut_bps,
                profit_bps,
                buffer_bps,
                twap_window,
                expiry_window,
            },
        ),
        FuturesInstruction::StakeConfig {
            haircut_bps,
            profit_bps,
            buffer_bps,
            twap_window,
            expiry_window,
        } => stake_config(
            program_id,
            accounts,
            StakeParams {
                haircut_bps,
                profit_bps,
                buffer_bps,
                twap_window,
                expiry_window,
            },
        ),
        #[cfg(feature = "operator-price")]
        FuturesInstruction::StakePrice { price } => stake_price(program_id, accounts, price),
        #[cfg(not(feature = "operator-price"))]
        FuturesInstruction::StakePrice { .. } => Err(FuturesError::Unauthorized.into()),
        FuturesInstruction::Stake { amount } => stake(program_id, accounts, amount),
        FuturesInstruction::Unstake { shares } => unstake(program_id, accounts, shares),
        FuturesInstruction::StakeClaim => stake_claim(program_id, accounts),
        FuturesInstruction::StakeRewards {
            start_ts,
            end_ts,
            amount,
        } => stake_rewards(program_id, accounts, (start_ts, end_ts, amount)),
        FuturesInstruction::StakeRewardsWithdraw { amount } => {
            stake_rewards_withdraw(program_id, accounts, amount)
        }
        FuturesInstruction::CreditFund { amount } => credit_fund(program_id, accounts, amount),
        FuturesInstruction::CreditDefund { amount } => credit_defund(program_id, accounts, amount),
        FuturesInstruction::CreditSettle => credit_settle(program_id, accounts),
        FuturesInstruction::SettleRecovery { claims, maker } => {
            settle_recovery(program_id, accounts, &claims, maker)
        }
        FuturesInstruction::PoolPrice => pool_price(program_id, accounts),
        FuturesInstruction::StartUnstake { shares } => start_unstake(program_id, accounts, shares),
        FuturesInstruction::CancelUnstake => cancel_unstake(program_id, accounts),
    }
}

// ---------------------------------------------------------------- loading

fn load<'a, 'b>(
    program_id: &Pubkey,
    market: &'a AccountInfo<'b>,
) -> R<(std::cell::RefMut<'a, &'b mut [u8]>, Header)> {
    if market.owner != program_id {
        return Err(FuturesError::InvalidAccount.into());
    }
    need_writable(market)?;
    let data = market.try_borrow_mut_data()?;
    let h = Header::read(&data)?;
    Ok((data, h))
}

/// Writes the header after checking the market's invariants. The credit
/// owner's free cash repays its debt first.
fn store(b: &mut [u8], h: &Header) -> R<()> {
    let mut h = *h;
    repay(b, &mut h)?;
    audit(b, &h)?;
    h.write(b);
    Ok(())
}

/// The credit owner's free cash goes to its debt before anything else, until
/// the market's credit settles (then `debt` is what the winners give up).
/// Then the debt must stay carried by open sets: debt <= backing plus the
/// credit owner's own cash here. So everyone else's free and reserved cash is
/// always in custody: a loss the credit owner realizes beyond its cash can
/// only come out of settlement payouts (where the stakers' buffer and AMBA
/// make winners whole), never out of someone's withdrawable cash (the review
/// of 2026-10-18, finding 2). While the market is open this can't fail for
/// anyone's instruction: the credit owner's own instructions keep its debt
/// within its own cash and the backing its claims and inventory hold
/// ([`check_credit`], `state::credit_risk`), which is never more than the
/// backing, at worst over every fill of its resting liquidity; nobody
/// else's instruction moves either (the re-review of 2026-10-18). It stays
/// as a check.
fn repay(b: &mut [u8], h: &mut Header) -> R<()> {
    if h.debt == 0 || !h.has_credit() {
        return Ok(());
    }
    let owner = h.credit_owner;
    let slot = find_trader(b, h, &owner);
    if let (Some(t), false) = (slot, h.credit_settled()) {
        let r = trader(b, h, t).free.min(h.debt);
        if r > 0 {
            debit_free(b, h, t, r)?;
            h.debt -= r;
        }
    }
    let own = slot.map_or(0, |t| {
        let tr = trader(b, h, t);
        u128::from(tr.free) + u128::from(tr.reserved)
    });
    let own = if h.credit_settled() { 0 } else { own };
    if u128::from(h.debt) > u128::from(h.backing) + own {
        return Err(FuturesError::CreditLimit.into());
    }
    Ok(())
}

/// Cash `need` in trader `t`'s free balance: the credit owner borrows what
/// its free cash lacks (`debt`) while its market lends (open, before
/// `credit_until`). Returns what it borrowed; the caller then checks the
/// credit ([`check_credit`]). Anyone else must have it.
fn fund(b: &mut [u8], h: &mut Header, t: u16, need: u64, now: u64) -> R<u64> {
    let tr = trader(b, h, t);
    if tr.free >= need
        || !h.has_credit()
        || tr.owner != h.credit_owner
        || h.status != STATUS_OPEN
        || h.credit_settled()
        || now >= h.credit_until
    {
        return Ok(0);
    }
    let lend = need - tr.free;
    credit_free(b, h, t, lend)?;
    h.debt = add(h.debt, lend)?;
    Ok(lend)
}

/// After the credit owner's own `Quote` or `Submit` while it owes anything
/// (it borrowed now, or before): at worst over every later fill of its
/// resting orders, close orders and ladder (`state::credit_admits`), its
/// exposure stays within the market's credit limit and its own cash and the
/// backing its claims and inventory hold carry its debt. So no fill of its
/// liquidity can break either rule, and no taker's package through it is
/// refused for its sake (the re-review of 2026-10-18). A no-op for anyone
/// else.
fn check_credit(b: &[u8], h: &Header, t: u16) -> R<()> {
    if !credit_admits(b, h, t).ok_or(FuturesError::Overflow)? {
        return Err(FuturesError::CreditLimit.into());
    }
    Ok(())
}

/// After a `Submit` that fills the credit owner's resting liquidity: both
/// rules hold now. [`check_credit`] admitted that liquidity at its worst
/// fill, so this can't fail; it stays as a check.
fn assert_credit(b: &[u8], h: &Header, t: u16) -> R<()> {
    if h.debt == 0 || !h.has_credit() || trader(b, h, t).owner != h.credit_owner {
        return Ok(());
    }
    let r = credit_risk(b, h, t, false).ok_or(FuturesError::Overflow)?;
    if r.exposure > h.credit_limit || h.debt > r.carry {
        return Err(FuturesError::CreditLimit.into());
    }
    Ok(())
}

fn find_trader(b: &[u8], h: &Header, owner: &Pubkey) -> Option<u16> {
    (0..h.trader_cap).find(|&i| key_at(b, h.trader_off(i)) == *owner)
}

/// The owner's trader slot, allocating a free (or idle) one if needed.
fn trader_slot(b: &mut [u8], h: &Header, owner: &Pubkey) -> R<u16> {
    if let Some(i) = find_trader(b, h, owner) {
        return Ok(i);
    }
    let i = (0..h.trader_cap)
        .find(|&i| {
            let t = Trader::read(b, h.trader_off(i));
            (t.owner == Pubkey::default() || t.idle())
                // The credit owner's slot holds its debt until its credit
                // settles (the review of 2026-10-18, finding 5).
                && !(h.has_credit() && !h.credit_settled() && t.owner == h.credit_owner)
        })
        .ok_or(FuturesError::TraderCapacity)?;
    Trader {
        owner: *owner,
        ..Trader::default()
    }
    .write(b, h.trader_off(i));
    Ok(i)
}

/// Whether `key` is an ed25519 point, as a wallet's key is; program
/// addresses (the Earn vault's and pools' among them) are off the curve.
#[allow(unsafe_code)]
fn on_curve(key: &Pubkey) -> bool {
    #[cfg(target_os = "solana")]
    {
        let mut result = 0u8;
        // Curve 0 is curve25519 in Edwards form; 0 means a valid point.
        unsafe {
            solana_program::syscalls::sol_curve_validate_point(
                0,
                key.as_ref().as_ptr(),
                &mut result,
            ) == 0
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        key.is_on_curve()
    }
}

fn owned_trader(b: &[u8], h: &Header, owner: &AccountInfo) -> R<u16> {
    need_signer(owner)?;
    find_trader(b, h, owner.key).ok_or_else(|| FuturesError::Unauthorized.into())
}

/// A pool account of `market`, checked against its address.
#[cfg(feature = "legacy-pools")]
fn load_pool(program_id: &Pubkey, market: &Pubkey, pool: &AccountInfo) -> R<Pool> {
    if pool.owner != program_id {
        return Err(FuturesError::InvalidAccount.into());
    }
    let p = Pool::read(&pool.try_borrow_data()?)?;
    let at = Pubkey::create_program_address(
        &[
            NAMESPACE,
            POOL_SEED,
            market.as_ref(),
            p.operator.as_ref(),
            &[p.bump],
        ],
        program_id,
    )
    .map_err(|_| FuturesError::InvalidAccount)?;
    if p.market != *market || at != *pool.key {
        return Err(FuturesError::InvalidAccount.into());
    }
    Ok(p)
}

/// What an operator may do for the account it runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Operated {
    /// Trade: orders, quotes, cancels and claim operations.
    Trade,
    /// Cancel only (an order or the ladder): all a legacy pool still takes.
    Cancel,
}

/// `owner` signs; or `owner` is the Earn vault, `market` is one of its
/// markets and its operator signs as one of `rest` (the instruction's later
/// accounts); or (dev builds) `owner` is a legacy pool of `market`, the
/// action only cancels, and its operator signs.
fn authorize(
    program_id: &Pubkey,
    market: &Pubkey,
    owner: &AccountInfo,
    rest: &[AccountInfo],
    action: Operated,
) -> R<()> {
    if owner.is_signer {
        return Ok(());
    }
    if owner.owner != program_id {
        return Err(FuturesError::Unauthorized.into());
    }
    let vault = (owner.data_len() == VAULT_LEN || owner.data_len() == VAULT_REWARDS_LEN)
        && owner.try_borrow_data()?[..8] == VAULT_MAGIC;
    let operator = if vault {
        let v = load_vault(program_id, owner)?;
        if !v.markets.contains(market) {
            return Err(FuturesError::Unauthorized.into());
        }
        v.operator
    } else {
        legacy_operator(program_id, market, owner, action)?
    };
    if rest.iter().any(|a| a.is_signer && *a.key == operator) {
        Ok(())
    } else {
        Err(FuturesError::Unauthorized.into())
    }
}

/// A legacy pool's operator, for a cancel (dev builds only).
#[cfg(feature = "legacy-pools")]
fn legacy_operator(
    program_id: &Pubkey,
    market: &Pubkey,
    owner: &AccountInfo,
    action: Operated,
) -> R<Pubkey> {
    if action != Operated::Cancel {
        return Err(FuturesError::Unauthorized.into());
    }
    Ok(load_pool(program_id, market, owner)?.operator)
}

#[cfg(not(feature = "legacy-pools"))]
fn legacy_operator(_: &Pubkey, _: &Pubkey, _: &AccountInfo, _: Operated) -> R<Pubkey> {
    Err(FuturesError::Unauthorized.into())
}

/// The trader an instruction acts for: the signing owner's, or the vault's
/// (or a legacy pool's) when its operator signs (see [`authorize`]).
fn acting_trader(
    program_id: &Pubkey,
    b: &[u8],
    h: &Header,
    market: &Pubkey,
    owner: &AccountInfo,
    rest: &[AccountInfo],
    action: Operated,
) -> R<u16> {
    authorize(program_id, market, owner, rest, action)?;
    find_trader(b, h, owner.key).ok_or_else(|| FuturesError::Unauthorized.into())
}

fn trader(b: &[u8], h: &Header, i: u16) -> Trader {
    Trader::read(b, h.trader_off(i))
}

fn put_trader(b: &mut [u8], h: &Header, i: u16, t: &Trader) {
    t.write(b, h.trader_off(i));
}

/// Moves cash between a trader's buckets and the market's totals.
fn credit_free(b: &mut [u8], h: &mut Header, i: u16, amount: u64) -> R<()> {
    let mut t = trader(b, h, i);
    t.free = add(t.free, amount)?;
    h.total_free = add(h.total_free, amount)?;
    put_trader(b, h, i, &t);
    Ok(())
}

fn debit_free(b: &mut [u8], h: &mut Header, i: u16, amount: u64) -> R<()> {
    let mut t = trader(b, h, i);
    t.free = sub(t.free, amount)?;
    h.total_free = sub(h.total_free, amount)?;
    put_trader(b, h, i, &t);
    Ok(())
}

fn reserve(b: &mut [u8], h: &mut Header, i: u16, amount: u64) -> R<()> {
    let mut t = trader(b, h, i);
    t.free = sub(t.free, amount)?;
    t.reserved = add(t.reserved, amount)?;
    h.total_free = sub(h.total_free, amount)?;
    h.total_reserved = add(h.total_reserved, amount)?;
    put_trader(b, h, i, &t);
    Ok(())
}

fn release(b: &mut [u8], h: &mut Header, i: u16, amount: u64) -> R<()> {
    let mut t = trader(b, h, i);
    t.reserved = sub(t.reserved, amount).map_err(|_| FuturesError::Insolvent)?;
    t.free = add(t.free, amount)?;
    h.total_reserved = sub(h.total_reserved, amount).map_err(|_| FuturesError::Insolvent)?;
    h.total_free = add(h.total_free, amount)?;
    put_trader(b, h, i, &t);
    Ok(())
}

/// Escrow leaves a trader's reserve for the backing of new sets.
fn reserved_to_backing(b: &mut [u8], h: &mut Header, i: u16, amount: u64) -> R<()> {
    let mut t = trader(b, h, i);
    t.reserved = sub(t.reserved, amount).map_err(|_| FuturesError::Insolvent)?;
    h.total_reserved = sub(h.total_reserved, amount).map_err(|_| FuturesError::Insolvent)?;
    h.backing = add(h.backing, amount)?;
    put_trader(b, h, i, &t);
    Ok(())
}

/// Backing released to a trader's free cash (redemption or settlement).
fn backing_to_free(b: &mut [u8], h: &mut Header, i: u16, amount: u64) -> R<()> {
    h.backing = sub(h.backing, amount).map_err(|_| FuturesError::Insolvent)?;
    credit_free(b, h, i, amount)
}

fn next_seq(h: &mut Header) -> R<u64> {
    h.next_seq = add(h.next_seq, 1)?;
    Ok(h.next_seq)
}

/// The trading gate of `Submit` and `Quote`, the only instructions that open
/// exposure: the market is open and its own oracle period is Live, from its
/// effective listing until expiry (`Period::is_tradable_at`); anything else
/// is `NotTradable`. A market may be created on a Forming period (listed
/// ahead of its month) but takes no order or ladder until the oracle lists
/// it. The oracle fails a period only while it is Forming, never once Live,
/// so no market that traded can end on a Failed period without a price.
/// Source: the user, 2026-10-15, preflight Decisions 5
/// (`ship/releases/2026-10-15-preflight.md`). No other instruction needs a
/// Live period: deposits, withdrawals, cancels, splits, merges, redemptions
/// and transfers work in any phase, and the settlement paths keep their own
/// rules. Claims come only from `Submit` fills, so none exists before the
/// period is Live, and closing one (a `Submit` with `close_claim`) passes
/// this gate whenever an opening order does.
fn tradable(h: &Header, period: &AccountInfo) -> R<()> {
    if h.status != STATUS_OPEN || *period.key != h.period {
        return Err(FuturesError::NotTradable.into());
    }
    let p = load_period_for(&h.oracle_program, period, &h.product_id, h.expiry_ts)?;
    if !p.is_tradable_at(now_ts()?) {
        return Err(FuturesError::NotTradable.into());
    }
    Ok(())
}

fn check_shape(h: &Header, lo: u16, hi: u16) -> R<()> {
    if lo >= hi || hi > h.grid {
        return Err(FuturesError::InvalidShape.into());
    }
    Ok(())
}

// ---------------------------------------------------------------- invariants

/// Cash conservation and full backing, recomputed from every record:
///
/// * trader buckets sum to the totals; reserves are exactly the escrow of
///   resting orders and maker ladders;
/// * custody (deposits minus withdrawals) plus the credit owner's debt =
///   free + reserved + backing;
/// * while open, backing = `k` × (short coverage) = `k` × (long coverage):
///   every promised payoff is backed by exactly one complete set;
/// * once final, backing = what the remaining claims and inventories pay.
#[inline(never)]
fn audit(b: &[u8], h: &Header) -> R<()> {
    let fail = || -> ProgramError { FuturesError::Insolvent.into() };
    let (mut free, mut reserved) = (0u128, 0u128);
    for i in 0..h.trader_cap {
        let at = h.trader_off(i);
        free += u128::from(u64_at(b, at + 32));
        reserved += u128::from(u64_at(b, at + 40));
    }
    if free != u128::from(h.total_free) || reserved != u128::from(h.total_reserved) {
        return Err(fail());
    }
    let open = h.status == STATUS_OPEN;
    let mut escrow = 0u128;
    // Each held claim slot is held by one resting opening order of its
    // owner, side and range.
    let mut holds = 0u32;
    for i in 0..h.order_cap {
        let at = h.order_off(i);
        if u64_at(b, at) == 0 {
            continue;
        }
        escrow += u128::from(u64_at(b, at + 16)) * u128::from(u64_at(b, at + 32));
        let o = Order::read(b, at);
        if o.flags & ORDER_CLOSE == 0 && o.claim != NO_CLAIM {
            if o.claim >= h.claim_cap {
                return Err(fail());
            }
            let c = Claim::read(b, h.claim_off(o.claim));
            if !c.held || (c.trader, c.side, c.lo, c.hi) != (o.trader, o.side, o.lo, o.hi) {
                return Err(fail());
            }
            holds += 1;
        }
    }
    let k = u128::from(h.k);
    let s = h.settlement_fine;
    let (mut long, mut short, mut owed) = (0u128, 0u128, 0u128);
    for m in 0..h.maker_cap {
        let head = h.maker_off(m);
        if b[head + 10] == 0 {
            continue;
        }
        escrow += u128::from(u64_at(b, head + 16));
        for x in 0..h.grid {
            let inv = i64_at(b, h.step_off(m, x) + 24);
            if inv == 0 {
                continue;
            }
            let lots = u128::from(inv.unsigned_abs());
            if inv > 0 {
                long += lots;
                if !open {
                    owed += lots * u128::from(long_per_lot(x, x + 1, s, h.k));
                }
            } else {
                short += lots;
                if !open {
                    owed += lots * u128::from(short_per_lot(x, x + 1, s, h.k));
                }
            }
        }
    }
    long *= k;
    short *= k;
    if escrow != reserved {
        return Err(fail());
    }
    let mut held = 0u32;
    for i in 0..h.claim_cap {
        let at = h.claim_off(i);
        held += u32::from(b[at + 19] != 0);
        let lots = u64_at(b, at);
        if lots == 0 {
            continue;
        }
        let (side, lo, hi) = (b[at + 18], u16_at(b, at + 20), u16_at(b, at + 22));
        let lots = u128::from(lots);
        let set = lots * u128::from(set_value(lo, hi, h.k));
        if side == Side::Long as u8 {
            long += set;
            if !open {
                owed += lots * u128::from(long_per_lot(lo, hi, s, h.k));
            }
        } else {
            short += set;
            if !open {
                owed += lots * u128::from(short_per_lot(lo, hi, s, h.k));
            }
        }
    }
    if held != holds {
        return Err(fail());
    }
    let backing = u128::from(h.backing);
    let balanced = if open {
        long == backing && short == backing
    } else {
        owed == backing
    };
    if !balanced || u128::from(h.custody) + u128::from(h.debt) != free + reserved + backing {
        return Err(fail());
    }
    Ok(())
}

// ---------------------------------------------------------------- tokens

fn token_account(info: &AccountInfo) -> R<spl_token::state::Account> {
    if *info.owner != spl_token::id() {
        return Err(FuturesError::Custody.into());
    }
    spl_token::state::Account::unpack(&info.try_borrow_data()?)
        .map_err(|_| FuturesError::Custody.into())
}

fn check_vault(h: &Header, vault: &AccountInfo) -> R<()> {
    if *vault.key != h.vault {
        return Err(FuturesError::InvalidAccount.into());
    }
    if token_account(vault)?.amount < h.custody {
        return Err(FuturesError::Custody.into());
    }
    Ok(())
}

// ---------------------------------------------------------------- create

struct CreateParams {
    product_id: [u8; 32],
    expiry_ts: u64,
    grid: u16,
    k: u64,
    step_value: u64,
    value_decimals: u8,
    ref_value: u64,
    trader_cap: u16,
    order_cap: u16,
    maker_cap: u8,
    claim_cap: u32,
}

/// `admin` is this program's upgrade authority, or the council approved
/// (governed). An immutable program passes only when `immutable_ok`: anyone
/// may list markets on it, but nobody may choose a settlement.
fn check_upgrade_authority(
    program_id: &Pubkey,
    program: &AccountInfo,
    program_data: &AccountInfo,
    admin: &Pubkey,
    council: &[AccountInfo],
    immutable_ok: bool,
) -> R<()> {
    use solana_sdk_ids::{bpf_loader, bpf_loader_upgradeable};
    if program.key != program_id {
        return Err(FuturesError::InvalidAccount.into());
    }
    if *program.owner == bpf_loader::ID {
        return if immutable_ok {
            Ok(())
        } else {
            Err(FuturesError::Unauthorized.into())
        };
    }
    if *program.owner != bpf_loader_upgradeable::ID {
        return Err(FuturesError::InvalidAccount.into());
    }
    let (expected, _) =
        Pubkey::find_program_address(&[program_id.as_ref()], &bpf_loader_upgradeable::ID);
    if *program_data.key != expected || *program_data.owner != bpf_loader_upgradeable::ID {
        return Err(FuturesError::InvalidAccount.into());
    }
    let d = program_data.try_borrow_data()?;
    // UpgradeableLoaderState::ProgramData { slot, upgrade_authority_address: Option<Pubkey> }
    if d.len() < 45 || u32::from_le_bytes(d[0..4].try_into().unwrap()) != 3 || d[12] != 1 {
        return Err(FuturesError::Unauthorized.into());
    }
    if d[13..45] == admin.to_bytes() {
        return Ok(());
    }
    let controller = ameba_governance_interface::PINNED_CONTROLLER_PROGRAM_ID;
    let authority = Pubkey::find_program_address(
        &[
            b"ameba-governance-v3",
            b"target-authority",
            program_id.as_ref(),
        ],
        &controller,
    )
    .0;
    if d[13..45] != authority.to_bytes() {
        return Err(FuturesError::Unauthorized.into());
    }
    let [config, seats @ ..] = council else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    ameba_governance_interface::validate_council_authorization(config, seats)?;
    Ok(())
}

#[inline(never)]
fn create_market(program_id: &Pubkey, a: &[AccountInfo], p: CreateParams) -> ProgramResult {
    let [market, registry, period, oracle_program, mint, vault, creator, system, this_program, program_data, ..] =
        a
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(creator)?;
    need_writable(market)?;
    need_writable(registry)?;
    check_upgrade_authority(
        program_id,
        this_program,
        program_data,
        creator.key,
        &a[10..],
        true,
    )?;
    if market.owner != program_id
        || market.data_len()
            != market_len(p.grid, p.trader_cap, p.order_cap, p.maker_cap, p.claim_cap)
        || market.try_borrow_data()?.iter().any(|&x| x != 0)
    {
        return Err(FuturesError::InvalidAccount.into());
    }
    if p.grid < 2
        || p.k == 0
        || p.step_value == 0
        || p.trader_cap == 0
        || p.order_cap == 0
        || p.order_cap == u16::MAX
        || p.claim_cap == 0
        || p.claim_cap == NO_CLAIM
    {
        return Err(FuturesError::InvalidInstruction.into());
    }
    // Settlement identity: the oracle period for exactly this index and expiry,
    // in its value decimals (`finalize` refuses a settlement in any other).
    let period_state = load_period_for(oracle_program.key, period, &p.product_id, p.expiry_ts)?;
    if period_state.value_decimals != p.value_decimals {
        return Err(FuturesError::InvalidInstruction.into());
    }
    let (authority, authority_bump) = authority_address(program_id, market.key);
    let v = token_account(vault)?;
    if *mint.owner != spl_token::id() || v.mint != *mint.key || v.owner != authority {
        return Err(FuturesError::Custody.into());
    }
    let (reg_key, reg_bump) = registry_address(program_id, &p.product_id, p.expiry_ts);
    if reg_key != *registry.key {
        return Err(FuturesError::InvalidAccount.into());
    }
    if *system.key != solana_system_interface::program::ID
        || *registry.owner != solana_system_interface::program::ID
        || registry.lamports() != 0
    {
        return Err(FuturesError::InvalidAccount.into());
    }
    let need = Rent::get()?.minimum_balance(REGISTRY_LEN);
    let expiry = p.expiry_ts.to_le_bytes();
    let seeds: &[&[u8]] = &[
        NAMESPACE,
        REGISTRY_SEED,
        &p.product_id,
        &expiry,
        &[reg_bump],
    ];
    invoke_signed(
        &system_instruction::create_account(
            creator.key,
            registry.key,
            need,
            REGISTRY_LEN as u64,
            program_id,
        ),
        &[creator.clone(), registry.clone(), system.clone()],
        &[seeds],
    )?;
    Registry {
        bump: reg_bump,
        market: *market.key,
    }
    .write(&mut registry.try_borrow_mut_data()?);

    let h = Header {
        status: STATUS_OPEN,
        authority_bump,
        value_decimals: p.value_decimals,
        grid: p.grid,
        trader_cap: p.trader_cap,
        order_cap: p.order_cap,
        maker_cap: p.maker_cap,
        registry_bump: reg_bump,
        claim_cap: p.claim_cap,
        product_id: p.product_id,
        expiry_ts: p.expiry_ts,
        period: *period.key,
        oracle_program: *oracle_program.key,
        quote_mint: *mint.key,
        vault: *vault.key,
        creator: *creator.key,
        k: p.k,
        step_value: p.step_value,
        ref_fine: settle_fine(p.ref_value, p.step_value, p.k),
        settlement_fine: 0,
        ..Header::default()
    };
    let mut data = market.try_borrow_mut_data()?;
    h.write(&mut data);
    sol_log_data(&[
        b"ARMRKT01",
        market.key.as_ref(),
        &p.product_id,
        &expiry,
        registry.key.as_ref(),
    ]);
    Ok(())
}

// ---------------------------------------------------------------- cash

#[inline(never)]
fn deposit(program_id: &Pubkey, a: &[AccountInfo], amount: u64) -> ProgramResult {
    let [market, owner, owner_token, vault, token_program, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(owner)?;
    if *token_program.key != spl_token::id() || amount == 0 {
        return Err(FuturesError::InvalidInstruction.into());
    }
    let (mut b, mut h) = load(program_id, market)?;
    if *vault.key != h.vault || owner_token.key == vault.key {
        return Err(FuturesError::InvalidAccount.into());
    }
    let i = trader_slot(&mut b, &h, owner.key)?;
    let before = token_account(vault)?.amount;
    invoke(
        &spl_token::instruction::transfer(
            &spl_token::id(),
            owner_token.key,
            vault.key,
            owner.key,
            &[],
            amount,
        )?,
        &[
            owner_token.clone(),
            vault.clone(),
            owner.clone(),
            token_program.clone(),
        ],
    )?;
    if token_account(vault)?.amount != add(before, amount)? {
        return Err(FuturesError::Custody.into());
    }
    credit_free(&mut b, &mut h, i, amount)?;
    h.custody = add(h.custody, amount)?;
    check_vault(&h, vault)?;
    sol_log_data(&[b"ARDEPO01", owner.key.as_ref(), &amount.to_le_bytes()]);
    store(&mut b, &h)
}

#[inline(never)]
fn withdraw(program_id: &Pubkey, a: &[AccountInfo], amount: u64) -> ProgramResult {
    let [market, owner, owner_token, vault, token_program, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if *token_program.key != spl_token::id() {
        return Err(FuturesError::InvalidInstruction.into());
    }
    let (mut b, mut h) = load(program_id, market)?;
    let i = owned_trader(&b, &h, owner)?;
    let free = trader(&b, &h, i).free;
    let amount = if amount == u64::MAX { free } else { amount };
    if amount > 0 {
        let dest = token_account(owner_token)?;
        if *vault.key != h.vault
            || owner_token.key == vault.key
            || dest.owner != *owner.key
            || dest.mint != h.quote_mint
        {
            return Err(FuturesError::Custody.into());
        }
        debit_free(&mut b, &mut h, i, amount)?;
        h.custody = sub(h.custody, amount)?;
        let bump = [h.authority_bump];
        let seeds: &[&[u8]] = &[NAMESPACE, AUTHORITY_SEED, market.key.as_ref(), &bump];
        let authority = Pubkey::create_program_address(seeds, program_id)
            .map_err(|_| FuturesError::InvalidAccount)?;
        let auth_info = a.get(5).ok_or(ProgramError::NotEnoughAccountKeys)?; // vault authority
        if *auth_info.key != authority {
            return Err(FuturesError::InvalidAccount.into());
        }
        invoke_signed(
            &spl_token::instruction::transfer(
                &spl_token::id(),
                vault.key,
                owner_token.key,
                &authority,
                &[],
                amount,
            )?,
            &[
                vault.clone(),
                owner_token.clone(),
                auth_info.clone(),
                token_program.clone(),
            ],
            &[seeds],
        )?;
        check_vault(&h, vault)?;
    }
    sol_log_data(&[b"ARWDRW01", owner.key.as_ref(), &amount.to_le_bytes()]);
    store(&mut b, &h)
}

// ---------------------------------------------------------------- claims

fn claim(b: &[u8], h: &Header, i: u32) -> R<Claim> {
    if i >= h.claim_cap {
        return Err(FuturesError::ClaimNotFound.into());
    }
    Ok(Claim::read(b, h.claim_off(i)))
}

fn owned_claim(b: &[u8], h: &Header, i: u32, seq: u64, t: u16) -> R<Claim> {
    let c = claim(b, h, i)?;
    if c.lots == 0 || c.seq != seq {
        return Err(FuturesError::ClaimNotFound.into());
    }
    if c.trader != t {
        return Err(FuturesError::Unauthorized.into());
    }
    Ok(c)
}

fn put_claim(b: &mut [u8], h: &Header, i: u32, c: &Claim) {
    c.write(b, h.claim_off(i));
}

/// A free claim slot: no lots and not held by a resting order.
fn free_slot(b: &[u8], h: &mut Header) -> R<u32> {
    let cap = h.claim_cap;
    let i = (0..cap)
        .map(|j| (h.claim_hint + j) % cap)
        .find(|&i| {
            let at = h.claim_off(i);
            u64_at(b, at) == 0 && b[at + 19] == 0
        })
        .ok_or(FuturesError::ClaimCapacity)?;
    h.claim_hint = (i + 1) % cap;
    Ok(i)
}

/// Opens a claim slot for trader `t`.
fn new_claim(b: &mut [u8], h: &mut Header, mut c: Claim) -> R<u32> {
    let i = free_slot(b, h)?;
    c.seq = next_seq(h)?;
    put_claim(b, h, i, &c);
    let mut t = trader(b, h, c.trader);
    t.claims += 1;
    put_trader(b, h, c.trader, &t);
    h.claims_open += 1;
    Ok(i)
}

/// Empties claim slot `i`. A held slot stays its order's, as a placeholder.
fn drop_claim(b: &mut [u8], h: &mut Header, i: u32) {
    let c = Claim::read(b, h.claim_off(i));
    let mut t = trader(b, h, c.trader);
    t.claims = t.claims.saturating_sub(1);
    put_trader(b, h, c.trader, &t);
    h.claims_open = h.claims_open.saturating_sub(1);
    let empty = if c.held {
        Claim {
            seq: c.seq,
            trader: c.trader,
            side: c.side,
            lo: c.lo,
            hi: c.hi,
            held: true,
            ..Claim::default()
        }
    } else {
        Claim::default()
    };
    put_claim(b, h, i, &empty);
}

/// Empty slots that resting orders hold (placeholders).
fn placeholders(b: &[u8], h: &Header) -> u64 {
    (0..h.order_cap)
        .map(|i| h.order_off(i))
        .filter(|&at| {
            let claim = u32_at(b, at + 40);
            u64_at(b, at) != 0
                && b[at + 11] & ORDER_CLOSE == 0
                && claim < h.claim_cap
                && u64_at(b, h.claim_off(claim)) == 0
        })
        .count() as u64
}

/// Claim slots neither in use nor held.
fn free_slots(b: &[u8], h: &Header) -> u64 {
    u64::from(h.claim_cap)
        .saturating_sub(h.claims_open)
        .saturating_sub(placeholders(b, h))
}

/// The slot resting opening order `o` of trader `t` holds for its fills:
/// `prefer` (where the order's own taker fill went) or another claim of the
/// owner with its side and range that no order holds yet, or else a
/// placeholder in a free slot (`ClaimCapacity` when there is none).
fn take_hold(b: &mut [u8], h: &mut Header, o: &Order, prefer: Option<u32>) -> R<u32> {
    let fits = |b: &[u8], i: u32| {
        let at = h.claim_off(i);
        u64_at(b, at) > 0 && u16_at(b, at + 16) == o.trader && {
            let c = Claim::read(b, at);
            !c.held && c.side == o.side && (c.lo, c.hi) == (o.lo, o.hi)
        }
    };
    let found = prefer.filter(|&i| fits(b, i)).or_else(|| {
        (trader(b, h, o.trader).claims > 0)
            .then(|| (0..h.claim_cap).find(|&i| fits(b, i)))
            .flatten()
    });
    let i = match found {
        Some(i) => {
            let mut c = Claim::read(b, h.claim_off(i));
            c.held = true;
            put_claim(b, h, i, &c);
            i
        }
        None => {
            let i = free_slot(b, h)?;
            let c = Claim {
                seq: next_seq(h)?,
                trader: o.trader,
                side: o.side,
                lo: o.lo,
                hi: o.hi,
                held: true,
                ..Claim::default()
            };
            put_claim(b, h, i, &c);
            i
        }
    };
    Ok(i)
}

/// A resting order gives up the slot it held: an empty one is free again.
fn release_hold(b: &mut [u8], h: &Header, i: u32) {
    let mut c = Claim::read(b, h.claim_off(i));
    c.held = false;
    put_claim(b, h, i, &if c.lots == 0 { Claim::default() } else { c });
}

/// Removes `lots` from a claim with its share of the cost basis.
fn shrink(c: &mut Claim, lots: u64) -> u64 {
    let part = (u128::from(c.cost) * u128::from(lots) / u128::from(c.lots.max(1))) as u64;
    let part = if lots == c.lots { c.cost } else { part };
    c.cost -= part;
    c.lots -= lots;
    part
}

/// `c` gives up `lots` to a redemption, from its reserved lots (a resting
/// close order's) or from the others.
fn give_up(c: &mut Claim, lots: u64, from_reserved: bool) -> R<()> {
    let usable = if from_reserved {
        c.lots
    } else {
        c.lots - c.reserved
    };
    if lots > usable {
        return Err(FuturesError::ClaimReserved.into());
    }
    if from_reserved {
        c.reserved = c
            .reserved
            .checked_sub(lots)
            .ok_or(FuturesError::ClaimReserved)?;
    }
    shrink(c, lots);
    Ok(())
}

/// Writes back claim `i` after a redemption took some of its lots.
fn put_shrunk(b: &mut [u8], h: &mut Header, i: u32, mut c: Claim) -> R<()> {
    if c.lots == 0 {
        drop_claim(b, h, i);
    } else {
        c.seq = next_seq(h)?;
        put_claim(b, h, i, &c);
    }
    Ok(())
}

/// Pays trader `t` the backing of `lots` redeemed sets on `[lo, hi)`.
fn pay_redeemed(b: &mut [u8], h: &mut Header, t: u16, lo: u16, hi: u16, lots: u64) -> R<u64> {
    let cash = mul(lots, set_value(lo, hi, h.k))?;
    backing_to_free(b, h, t, cash)?;
    let owner = trader(b, h, t).owner;
    sol_log_data(&[
        b"ARREDM01",
        owner.as_ref(),
        &lo.to_le_bytes(),
        &hi.to_le_bytes(),
        &lots.to_le_bytes(),
        &cash.to_le_bytes(),
    ]);
    Ok(cash)
}

fn complements(x: &Claim, y: &Claim) -> bool {
    x.trader == y.trader
        && (x.lo, x.hi) == (y.lo, y.hi)
        && x.side != y.side
        && x.lots > 0
        && y.lots > 0
}

/// Redeems `lots` of an exact complement pair held by one trader.
fn redeem_pair(
    b: &mut [u8],
    h: &mut Header,
    x: u32,
    y: u32,
    lots: u64,
    from_reserved: [bool; 2],
) -> R<u64> {
    let mut cx = Claim::read(b, h.claim_off(x));
    let mut cy = Claim::read(b, h.claim_off(y));
    if !complements(&cx, &cy) {
        return Err(FuturesError::NotComplement.into());
    }
    give_up(&mut cx, lots, from_reserved[0])?;
    give_up(&mut cy, lots, from_reserved[1])?;
    put_shrunk(b, h, x, cx)?;
    put_shrunk(b, h, y, cy)?;
    pay_redeemed(b, h, cx.trader, cx.lo, cx.hi, lots)
}

/// Redeems `lots` of a fresh claim, not yet in a slot, against its owner's
/// exact complement in slot `y` (from its reserved lots: a resting close
/// order's).
fn redeem_fresh(
    b: &mut [u8],
    h: &mut Header,
    fresh: &mut Claim,
    y: u32,
    lots: u64,
    from_reserved: bool,
) -> R<()> {
    let mut cy = Claim::read(b, h.claim_off(y));
    if !complements(fresh, &cy) {
        return Err(FuturesError::NotComplement.into());
    }
    give_up(fresh, lots, false)?;
    give_up(&mut cy, lots, from_reserved)?;
    put_shrunk(b, h, y, cy)?;
    pay_redeemed(b, h, fresh.trader, fresh.lo, fresh.hi, lots)?;
    Ok(())
}

/// Puts a fill's (or a transfer's) fresh claim in the table, taking a slot
/// only for what is left (`docs/futures.md`, "Claim slots"): it first
/// redeems against `first` (a close's claim, its exact complement; `true`:
/// its reserved lots) and the owner's other exact complements, in slot
/// order, without a slot of its own. What is left joins `hold` (the slot a
/// resting order holds) or the owner's claim of the same side and range, or
/// else takes a free slot, and nets against the owner's overlapping opposite
/// claims ([`net_overlaps`]). So a fill that closes a position never needs
/// a free slot. Returns the slot holding what is left (`None`: all of it
/// redeemed).
fn place(
    b: &mut [u8],
    h: &mut Header,
    mut fresh: Claim,
    first: Option<(u32, bool)>,
    hold: Option<u32>,
) -> R<Option<u32>> {
    if let Some((c, reserved)) = first {
        let other = Claim::read(b, h.claim_off(c));
        let usable = if reserved {
            other.reserved
        } else {
            other.lots - other.reserved
        };
        let lots = fresh.lots.min(usable);
        if lots > 0 {
            redeem_fresh(b, h, &mut fresh, c, lots, reserved)?;
        }
    }
    let t = fresh.trader;
    let mut join = hold;
    let mut overlaps = false;
    if fresh.lots > 0 && trader(b, h, t).claims > 0 {
        for i in 0..h.claim_cap {
            let at = h.claim_off(i);
            if u64_at(b, at) == 0 || u16_at(b, at + 16) != t {
                continue;
            }
            let c = Claim::read(b, at);
            let exact = (c.lo, c.hi) == (fresh.lo, fresh.hi);
            if c.side == fresh.side {
                if exact && join.is_none() {
                    join = Some(i);
                }
            } else if exact {
                let lots = fresh.lots.min(c.lots - c.reserved);
                if lots > 0 {
                    redeem_fresh(b, h, &mut fresh, i, lots, false)?;
                    if fresh.lots == 0 {
                        return Ok(None);
                    }
                }
            } else if c.lo < fresh.hi && fresh.lo < c.hi && c.lots > c.reserved {
                overlaps = true;
            }
        }
    }
    if fresh.lots == 0 {
        return Ok(None);
    }
    let lots = fresh.lots;
    let f = match join {
        Some(i) => {
            let mut c = Claim::read(b, h.claim_off(i));
            if c.lots == 0 {
                // A placeholder: the order's first fill since it held it.
                c.opened_ts = fresh.opened_ts;
                let mut tr = trader(b, h, t);
                tr.claims += 1;
                put_trader(b, h, t, &tr);
                h.claims_open += 1;
            }
            c.lots = add(c.lots, fresh.lots)?;
            c.cost = add(c.cost, fresh.cost)?;
            c.opened_ts = c.opened_ts.min(fresh.opened_ts);
            c.seq = next_seq(h)?;
            put_claim(b, h, i, &c);
            i
        }
        None => new_claim(b, h, fresh)?,
    };
    if overlaps {
        net_overlaps(b, h, f, lots)?;
    }
    Ok(Some(f))
}

/// Nets `lots` of claim `fresh` (what a fill just put there) against its
/// owner's overlapping opposite claims ([`plan_net`]; the exact complements
/// [`place`] redeemed already). Each overlap's matched lots are redeemed for
/// their sets and the parts outside it kept, so the owner's payoff is the
/// same at every settlement and the overlap's backing is free cash at once.
/// Best effort: a match whose parts find no claim slot, and every match
/// after it, is left as it is.
fn net_overlaps(b: &mut [u8], h: &mut Header, fresh: u32, lots: u64) -> R<()> {
    let n = Claim::read(b, h.claim_off(fresh));
    let mut opposite = Vec::new();
    for i in 0..h.claim_cap {
        let at = h.claim_off(i);
        if i == fresh || u64_at(b, at) == 0 || u16_at(b, at + 16) != n.trader {
            continue;
        }
        let c = Claim::read(b, at);
        if c.side != n.side {
            opposite.push((i, c));
        }
    }
    // The fresh claim's pieces (slots), as the matches split it.
    let mut pieces = vec![fresh];
    for m in plan_net(n.lo, n.hi, lots, &opposite) {
        let piece = pieces.iter().copied().find(|&s| {
            let c = Claim::read(b, h.claim_off(s));
            c.trader == n.trader && c.side == n.side && (c.lo, c.hi) == m.piece && c.lots >= m.lots
        });
        let Some(f) = piece else { break };
        match net_overlap(b, h, f, m.claim, (m.lo, m.hi), m.lots)? {
            Some(parts) => pieces.extend(parts),
            None => break,
        }
    }
    Ok(())
}

/// Redeems `lots` of the overlap `[a, z)` of one owner's opposite claims `f`
/// and `c`: each gives up `lots` of its unreserved lots, whose parts outside
/// `[a, z)` stay as claims of `lots` lots (the cost basis split by width, as
/// `Split`; a claim left empty keeps one of its parts in its slot unless a
/// resting order holds the slot), and the overlap's sets pay into free cash.
/// Returns the slots now holding new parts of `f`, or `None`, changing
/// nothing, when the market has no slots for the parts.
fn net_overlap(
    b: &mut [u8],
    h: &mut Header,
    f: u32,
    c: u32,
    (a, z): (u16, u16),
    lots: u64,
) -> R<Option<Vec<u32>>> {
    let claims = [
        Claim::read(b, h.claim_off(f)),
        Claim::read(b, h.claim_off(c)),
    ];
    let parts = |x: &Claim| {
        [
            (x.lo < a).then_some((x.lo, a)),
            (z < x.hi).then_some((z, x.hi)),
        ]
    };
    let mut need = 0u64;
    for x in &claims {
        if lots > x.lots - x.reserved || a < x.lo || x.hi < z {
            return Err(FuturesError::NotComplement.into());
        }
        let n = parts(x).iter().flatten().count() as u64;
        need += if x.lots == lots && !x.held {
            n.saturating_sub(1)
        } else {
            n
        };
    }
    if need > free_slots(b, h) {
        return Ok(None);
    }
    let mut fresh_parts = Vec::new();
    for (slot, mut x) in [f, c].into_iter().zip(claims) {
        let part = shrink(&mut x, lots);
        let (template, width) = (x, u128::from(x.hi - x.lo));
        let piece = |(lo, hi): (u16, u16)| Claim {
            lots,
            trader: template.trader,
            side: template.side,
            lo,
            hi,
            cost: (u128::from(part) * u128::from(hi - lo) / width) as u64,
            opened_ts: template.opened_ts,
            ..Claim::default()
        };
        let mut outside = parts(&template).into_iter().flatten();
        if x.lots > 0 {
            x.seq = next_seq(h)?;
            put_claim(b, h, slot, &x);
        } else if x.held {
            // The slot stays its order's; every part moves out.
            drop_claim(b, h, slot);
        } else if let Some(p) = outside.next() {
            // The emptied slot keeps the first part (for `f`, a piece
            // already).
            let mut p = piece(p);
            p.seq = next_seq(h)?;
            put_claim(b, h, slot, &p);
        } else {
            drop_claim(b, h, slot);
        }
        for p in outside {
            let s = new_claim(b, h, piece(p))?;
            if slot == f {
                fresh_parts.push(s);
            }
        }
    }
    pay_redeemed(b, h, claims[0].trader, a, z, lots)?;
    Ok(Some(fresh_parts))
}

// ---------------------------------------------------------------- the book

fn verify_error(e: VerifyError) -> ProgramError {
    match e {
        VerifyError::Potentials => FuturesError::BadProposal,
        VerifyError::Capacity { .. } | VerifyError::Unbalanced { .. } => FuturesError::Unbalanced,
        VerifyError::Price { .. } => FuturesError::NotBestPrice,
        VerifyError::Quantity => FuturesError::NotMaxFill,
        VerifyError::Fifo { .. } => FuturesError::FifoViolation,
        VerifyError::Work => FuturesError::SearchLimit,
    }
    .into()
}

struct SubmitParams {
    side: u8,
    lo: u16,
    hi: u16,
    lots: u64,
    limit: u64,
    min_fill: u64,
    mode: u8,
    close_claim: u32,
    close_seq: u64,
}

#[inline(never)]
fn submit(
    program_id: &Pubkey,
    a: &[AccountInfo],
    p: SubmitParams,
    proposal: &[u8],
) -> ProgramResult {
    let [market, owner, period, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    authorize(program_id, market.key, owner, &a[3..], Operated::Trade)?;
    let (mut b, mut h) = load(program_id, market)?;
    tradable(&h, period)?;
    check_shape(&h, p.lo, p.hi)?;
    let side = side_of(p.side)?;
    let width = set_value(p.lo, p.hi, h.k);
    if p.lots == 0 || p.min_fill > p.lots || p.limit > width || p.mode > MODE_POST {
        return Err(FuturesError::InvalidInstruction.into());
    }
    let t = trader_slot(&mut b, &h, owner.key)?;
    let close = if p.close_claim == NO_CLAIM {
        None
    } else {
        let c = owned_claim(&b, &h, p.close_claim, p.close_seq, t)?;
        if c.side == p.side || c.lo != p.lo || c.hi != p.hi {
            return Err(FuturesError::NotComplement.into());
        }
        if p.lots > c.lots - c.reserved {
            return Err(FuturesError::ClaimReserved.into());
        }
        Some(p.close_claim)
    };

    // Verify the package against the complete book.
    trace("start");
    let arcs = book_arcs(&b, &h);
    trace("arcs");
    let prop = Proposal::decode(proposal).map_err(|_| FuturesError::BadProposal)?;
    let fills = prop.dense(&arcs).map_err(|_| FuturesError::BadProposal)?;
    trace("decode");
    let taker = Taker {
        side,
        lo: p.lo,
        hi: p.hi,
        limit: p.limit,
        qty: p.lots,
    };
    rm::verify(
        &arcs,
        Some(&taker),
        &fills,
        prop.taker_fill,
        &prop.potentials,
        h.grid,
        WORK_LIMIT,
    )
    .map_err(verify_error)?;
    trace("verify");
    let filled = prop.taker_fill;
    if filled < p.min_fill || (p.mode == MODE_POST && filled > 0) {
        return Err(FuturesError::MinFill.into());
    }
    let seq = next_seq(&mut h)?;
    let now = now_ts()?;

    // Resting fills: each pays exactly its own price. Also what shorts
    // receive minus what longs pay (the taker's premium) and the sets the
    // resting shorts open.
    let mut resting = 0i128;
    let mut net_cost = 0i128;
    let mut short_sets = 0i128;
    let mut ladders: Vec<(u8, Side, u16, u16, u64, u64)> = Vec::new();
    // Per maker: the contribution of its escrowed lots, of the lots its
    // inventory backs, and the lots netted per step.
    let mut makers: Vec<(u8, [u64; 3])> = Vec::new();
    // Per maker: its head and tier sizes (the same for every step).
    let mut heads: Vec<(u8, Maker, (TierLots, TierLots))> = Vec::new();
    // Whether this package fills the credit owner's orders or ladder: its
    // admission bounded every such fill (`check_credit`), so the rules are
    // only asserted again (the review of 2026-10-18, finding 4, and its
    // re-review).
    let credit_slot = if h.has_credit() && !h.credit_settled() {
        let owner = h.credit_owner;
        find_trader(&b, &h, &owner)
    } else {
        None
    };
    let mut credit_filled = false;
    for (arc, &m) in arcs.iter().zip(&fills) {
        if m == 0 {
            continue;
        }
        credit_filled |= credit_slot.is_some_and(|c| match arc.id {
            ArcId::Order(slot) => u16_at(&b, h.order_off(slot) + 8) == c,
            ArcId::Maker { maker, .. } => u16_at(&b, h.maker_off(maker) + 8) == c,
        });
        let c = mul(m, contribution(arc.side, arc.lo, arc.hi, arc.price, h.k))?;
        resting += i128::from(c);
        net_cost += i128::from(arc.cost()) * i128::from(m);
        if arc.side == Side::Short {
            short_sets += i128::from(set_value(arc.lo, arc.hi, h.k)) * i128::from(m);
        }
        match arc.id {
            ArcId::Order(slot) => fill_order(&mut b, &mut h, slot, m, c, owner.key, now)?,
            ArcId::Maker {
                maker,
                step,
                side,
                tier,
            } => {
                let at = match heads.iter().position(|x| x.0 == maker) {
                    Some(at) => at,
                    None => {
                        let mk = Maker::read(&b, h.maker_off(maker));
                        heads.push((maker, mk, mk.sizes()));
                        heads.len() - 1
                    }
                };
                let (_, mk, sz) = &heads[at];
                let netted = fill_step(&mut b, &h, mk, maker, step, side, tier, m, sz)?;
                // A step's netted lots are the ones its inventory backs.
                let backed = mul(netted, c / m)?;
                let fill = [sub(c, backed)?, backed, netted];
                match makers.iter_mut().find(|x| x.0 == maker) {
                    Some(x) => {
                        for (a, v) in x.1.iter_mut().zip(fill) {
                            *a = add(*a, v)?;
                        }
                    }
                    None => makers.push((maker, fill)),
                }
                let premium = arc.price.saturating_mul(m);
                match ladders.iter_mut().find(|l| l.0 == maker && l.1 == side) {
                    Some(l) => {
                        l.2 = l.2.min(step);
                        l.3 = l.3.max(step + 1);
                        l.4 = l.4.saturating_add(m);
                        l.5 = l.5.saturating_add(premium);
                    }
                    None => ladders.push((maker, side, step, step + 1, m, premium)),
                }
            }
        }
    }
    for (maker, [escrowed, backed, netted]) in makers {
        let at = h.maker_off(maker);
        let mut mk = Maker::read(&b, at);
        // The escrowed lots pay from the escrow. It never falls short of
        // what the steps need: a fill takes exactly its escrowed lots' share,
        // and what it frees on the other side of a step stays reserved until
        // the next `Quote` reserves anew.
        mk.escrow = mk
            .escrow
            .checked_sub(escrowed)
            .ok_or(FuturesError::Insolvent)?;
        mk.write(&mut b, at);
        reserved_to_backing(&mut b, &mut h, mk.trader, escrowed)?;
        // The lots inventory backs pay from the sets their netting redeems.
        h.backing = add(h.backing, backed)?;
        if netted > 0 {
            let cash = mul(netted, h.k)?;
            backing_to_free(&mut b, &mut h, mk.trader, cash)?;
        }
        debit_free(&mut b, &mut h, mk.trader, backed)?;
    }
    // One line per ladder side: steps spanned, lot-steps filled, premium.
    for (maker, side, lo, hi, lot_steps, premium) in ladders {
        let mk = Maker::read(&b, h.maker_off(maker));
        let who = trader(&b, &h, mk.trader).owner;
        sol_log_data(&[
            b"ARLADR01",
            who.as_ref(),
            owner.key.as_ref(),
            &[side as u8],
            &lo.to_le_bytes(),
            &hi.to_le_bytes(),
            &lot_steps.to_le_bytes(),
            &premium.to_le_bytes(),
        ]);
    }

    // The taker funds the rest of the new sets.
    let premium = match side {
        Side::Long => net_cost,
        Side::Short => -net_cost,
    };
    let mut paid = 0i128;
    // Where the taker's fresh claim went (`None`: none, or all of it netted).
    let mut taken = None;
    if filled > 0 {
        let sets: i128 = short_sets
            + if side == Side::Short {
                i128::from(width) * i128::from(filled)
            } else {
                0
            };
        paid = sets - resting;
        let within = match side {
            Side::Long => premium <= i128::from(p.limit) * i128::from(filled),
            Side::Short => premium >= i128::from(p.limit) * i128::from(filled),
        };
        if !within {
            return Err(FuturesError::LimitExceeded.into());
        }
        let cost = u64::try_from(paid.max(0)).map_err(|_| FuturesError::Overflow)?;
        // Backing comes in first (any refund of a negative premium goes out
        // of it), then the fresh claim nets (a close against its claim, with
        // no slot of its own) and takes its slot, then the taker pays.
        if paid >= 0 {
            h.backing = add(h.backing, to_u64(paid)?)?;
        } else {
            backing_to_free(&mut b, &mut h, t, to_u64(-paid)?)?;
        }
        let fresh = Claim {
            lots: filled,
            trader: t,
            side: side as u8,
            lo: p.lo,
            hi: p.hi,
            cost,
            opened_ts: now,
            ..Claim::default()
        };
        taken = place(&mut b, &mut h, fresh, close.map(|c| (c, false)), None)?;
        if paid > 0 {
            fund(&mut b, &mut h, t, to_u64(paid)?, now)?;
            debit_free(&mut b, &mut h, t, to_u64(paid)?)?;
        }
        h.trades += 1;
        h.volume_lots = add(h.volume_lots, filled)?;
        h.volume_premium = h.volume_premium.saturating_add(premium.unsigned_abs());
        h.last_entry_fine = u64::from(p.lo) * h.k + to_u64(premium.max(0))? / filled;
        h.last_trade_ts = now;
    }

    // Rest the remainder.
    let left = p.lots - filled;
    let mut rested = u16::MAX;
    if left > 0 && p.mode != MODE_IOC {
        let slot = (0..h.order_cap)
            .find(|&i| u64_at(&b, h.order_off(i)) == 0)
            .ok_or(FuturesError::OrderCapacity)?;
        let mut o = Order {
            seq,
            trader: t,
            side: p.side,
            flags: 0,
            lo: p.lo,
            hi: p.hi,
            lots: left,
            price: p.limit,
            escrow_per_lot: contribution(side, p.lo, p.hi, p.limit, h.k),
            claim: NO_CLAIM,
            placed_lots: left,
            placed_ts: now,
        };
        match close {
            Some(c) => {
                let mut cl = Claim::read(&b, h.claim_off(c));
                if cl.lots == 0 || left > cl.lots - cl.reserved {
                    return Err(FuturesError::ClaimReserved.into());
                }
                cl.reserved += left;
                put_claim(&mut b, &h, c, &cl);
                (o.escrow_per_lot, o.flags, o.claim) = (0, ORDER_CLOSE, c);
            }
            // An opening order holds the claim slot its fills go to, so a
            // fill never waits for a free one (on a full table it can't
            // rest: `ClaimCapacity`).
            None => o.claim = take_hold(&mut b, &mut h, &o, taken)?,
        }
        let escrow = mul(left, o.escrow_per_lot)?;
        fund(&mut b, &mut h, t, escrow, now)?;
        reserve(&mut b, &mut h, t, escrow)?;
        o.write(&mut b, h.order_off(slot));
        let flags = o.flags;
        let mut tr = trader(&b, &h, t);
        tr.orders += 1;
        put_trader(&mut b, &h, t, &tr);
        rested = slot;
        sol_log_data(&[
            b"ARPLACE1",
            owner.key.as_ref(),
            &slot.to_le_bytes(),
            &seq.to_le_bytes(),
            &[p.side, flags],
            &p.lo.to_le_bytes(),
            &p.hi.to_le_bytes(),
            &left.to_le_bytes(),
            &p.limit.to_le_bytes(),
            &escrow.to_le_bytes(),
        ]);
    }
    sol_log_data(&[
        b"ARTAKE01",
        owner.key.as_ref(),
        &[p.side, p.mode],
        &p.lo.to_le_bytes(),
        &p.hi.to_le_bytes(),
        &p.lots.to_le_bytes(),
        &filled.to_le_bytes(),
        &premium.to_le_bytes(),
        &paid.to_le_bytes(),
        &seq.to_le_bytes(),
        &rested.to_le_bytes(),
    ]);
    check_credit(&b, &h, t)?;
    if let (Some(c), true) = (credit_slot, credit_filled) {
        assert_credit(&b, &h, c)?;
    }
    let mut ret = Vec::with_capacity(42);
    ret.extend_from_slice(&filled.to_le_bytes());
    ret.extend_from_slice(&premium.to_le_bytes());
    ret.extend_from_slice(&paid.to_le_bytes());
    ret.extend_from_slice(&rested.to_le_bytes());
    set_return_data(&ret);
    trace("execute");
    let r = store(&mut b, &h);
    trace("audit");
    r
}

/// A resting order fills `m` lots and puts `c` into backing.
fn fill_order(
    b: &mut [u8],
    h: &mut Header,
    slot: u16,
    m: u64,
    c: u64,
    taker: &Pubkey,
    now: u64,
) -> R<()> {
    let at = h.order_off(slot);
    let mut o = Order::read(b, at);
    o.lots = o.lots.checked_sub(m).ok_or(FuturesError::Unbalanced)?;
    let t = o.trader;
    let close = o.flags & ORDER_CLOSE != 0;
    if close {
        // Pays from the redemption of the claim it closes.
        h.backing = add(h.backing, c)?;
    } else {
        if mul(m, o.escrow_per_lot)? != c {
            return Err(FuturesError::Insolvent.into());
        }
        reserved_to_backing(b, h, t, c)?;
    }
    let fresh = Claim {
        lots: m,
        trader: t,
        side: o.side,
        lo: o.lo,
        hi: o.hi,
        cost: c,
        opened_ts: now,
        ..Claim::default()
    };
    // A close nets against the claim it reserves; an opening order's fill
    // joins the slot it holds.
    let hold = (!close && o.claim != NO_CLAIM).then_some(o.claim);
    let placed = place(b, h, fresh, close.then_some((o.claim, true)), hold)?;
    if close {
        debit_free(b, h, t, c)?;
    } else if hold.is_none() {
        // An order placed before slots were held takes hold of where its
        // fill went, if netting left a claim of its range there.
        if let Some(i) = placed {
            let mut cl = Claim::read(b, h.claim_off(i));
            if cl.lots > 0
                && !cl.held
                && (cl.trader, cl.side, cl.lo, cl.hi) == (t, o.side, o.lo, o.hi)
            {
                cl.held = true;
                put_claim(b, h, i, &cl);
                o.claim = i;
            }
        }
    }
    let owner = trader(b, h, t).owner;
    sol_log_data(&[
        b"ARFILL01",
        owner.as_ref(),
        taker.as_ref(),
        &[o.side, o.flags],
        &o.lo.to_le_bytes(),
        &o.hi.to_le_bytes(),
        &m.to_le_bytes(),
        &o.price.to_le_bytes(),
        &slot.to_le_bytes(),
        &o.seq.to_le_bytes(),
    ]);
    if o.lots == 0 {
        if !close && o.claim != NO_CLAIM {
            release_hold(b, h, o.claim);
        }
        Order::default().write(b, at);
        let mut tr = trader(b, h, t);
        tr.orders = tr.orders.saturating_sub(1);
        put_trader(b, h, t, &tr);
    } else {
        o.write(b, at);
    }
    Ok(())
}

/// A ladder step's tier fills `m` lots; the maker's inventory nets per step.
/// Returns the lots netted against opposite inventory (redeemed sets): the
/// lots that inventory backs (`Maker::step_escrow`), which pay from the sets
/// they redeem; the others pay from the ladder's escrow.
#[allow(clippy::too_many_arguments)]
fn fill_step(
    b: &mut [u8],
    h: &Header,
    mk: &Maker,
    maker: u8,
    step: u16,
    side: Side,
    tier: u8,
    m: u64,
    sizes: &(TierLots, TierLots),
) -> R<u64> {
    let at = h.step_off(maker, step);
    let mut st = Step::read(b, at);
    // Tiers trade best first (the price rule guarantees it; checked here).
    let (total, left, sz) = match side {
        Side::Long => (mk.bid_total, st.bid_lots, &sizes.0),
        Side::Short => (mk.ask_total, st.ask_lots, &sizes.1),
    };
    let rem = mk.tiers.remaining_in(sz, total, left);
    let t = usize::from(tier);
    if t >= mk.tiers.n() || m > rem[t] || rem[..t].iter().any(|&r| r > 0) {
        return Err(FuturesError::Unbalanced.into());
    }
    let before = st.inv;
    let delta = i64::try_from(m).map_err(|_| FuturesError::Overflow)?;
    let netted = match side {
        Side::Long => {
            st.bid_lots -= m;
            st.inv = st.inv.checked_add(delta).ok_or(FuturesError::Overflow)?;
            if before < 0 {
                m.min(before.unsigned_abs())
            } else {
                0
            }
        }
        Side::Short => {
            st.ask_lots -= m;
            st.inv = st.inv.checked_sub(delta).ok_or(FuturesError::Overflow)?;
            if before > 0 {
                m.min(before.unsigned_abs())
            } else {
                0
            }
        }
    };
    st.write(b, at);
    Ok(netted)
}

#[inline(never)]
fn cancel(program_id: &Pubkey, a: &[AccountInfo], slot: u16, seq: u64) -> ProgramResult {
    let [market, owner, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let (mut b, mut h) = load(program_id, market)?;
    let t = acting_trader(
        program_id,
        &b,
        &h,
        market.key,
        owner,
        &a[2..],
        Operated::Cancel,
    )?;
    if slot >= h.order_cap {
        return Err(FuturesError::OrderNotFound.into());
    }
    let o = Order::read(&b, h.order_off(slot));
    if o.seq == 0 || o.seq != seq {
        return Err(FuturesError::OrderNotFound.into());
    }
    if o.trader != t {
        return Err(FuturesError::Unauthorized.into());
    }
    let released = remove_order(&mut b, &mut h, slot)?;
    sol_log_data(&[
        b"ARCANCL1",
        owner.key.as_ref(),
        &slot.to_le_bytes(),
        &seq.to_le_bytes(),
        &released.to_le_bytes(),
    ]);
    store(&mut b, &h)
}

fn remove_order(b: &mut [u8], h: &mut Header, slot: u16) -> R<u64> {
    let at = h.order_off(slot);
    let o = Order::read(b, at);
    let escrow = mul(o.lots, o.escrow_per_lot)?;
    release(b, h, o.trader, escrow)?;
    if o.flags & ORDER_CLOSE != 0 {
        let mut c = Claim::read(b, h.claim_off(o.claim));
        if c.lots > 0 {
            c.reserved = c.reserved.saturating_sub(o.lots);
            put_claim(b, h, o.claim, &c);
        }
    } else if o.claim != NO_CLAIM {
        release_hold(b, h, o.claim);
    }
    Order::default().write(b, at);
    let mut tr = trader(b, h, o.trader);
    tr.orders = tr.orders.saturating_sub(1);
    put_trader(b, h, o.trader, &tr);
    Ok(escrow)
}

// ---------------------------------------------------------------- makers

type QuoteParams = Ladder;

#[inline(never)]
fn quote(
    program_id: &Pubkey,
    a: &[AccountInfo],
    p: QuoteParams,
    core: Core,
    knots: &[Knot],
    proposal: &[u8],
) -> ProgramResult {
    let [market, owner, period, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let (mut b, mut h) = load(program_id, market)?;
    tradable(&h, period)?;
    let t = acting_trader(
        program_id,
        &b,
        &h,
        market.key,
        owner,
        &a[3..],
        Operated::Trade,
    )?;
    check_shape(&h, p.lo, p.hi)?;
    let multi = p.tiers.count > 1;
    if p.hi - p.lo > MAX_LADDER_STEPS
        || !(p.lo <= core.lo && core.lo <= core.hi && core.hi <= p.hi)
        || p.tiers.count > ameba_futures_math::MAX_TIERS
        || (multi && (p.tiers.ratio == 0 || p.tiers.spread == 0))
        || knots.is_empty()
        || knots.windows(2).any(|w| w[0].pos >= w[1].pos)
        || knots.iter().any(|kn| u64::from(kn.price) > h.k)
        || u64::from(p.bid_spread) + u64::from(p.ask_spread) == 0
    {
        return Err(FuturesError::InvalidPrice.into());
    }
    let mut tr = trader(&b, &h, t);
    let m = if tr.maker > 0 {
        tr.maker - 1
    } else {
        let m = (0..h.maker_cap)
            .find(|&m| !Maker::read(&b, h.maker_off(m)).used)
            .ok_or(FuturesError::MakerCapacity)?;
        tr.maker = m + 1;
        put_trader(&mut b, &h, t, &tr);
        Maker {
            used: true,
            trader: t,
            ..Maker::default()
        }
        .write(&mut b, h.maker_off(m));
        m
    };
    let old = Maker::read(&b, h.maker_off(m));
    release(&mut b, &mut h, t, old.escrow)?;
    trace("start");
    let escrow = write_ladder(&mut b, &h, m, &p, core, knots).ok_or(FuturesError::Overflow)?;
    trace("write");
    // Admission is balance-based: the ladder must be fully funded now, the
    // credit owner's partly by what its stake pool lends it.
    let now = now_ts()?;
    fund(&mut b, &mut h, t, escrow, now)?;
    reserve(&mut b, &mut h, t, escrow)?;
    // The head as `write_ladder` left it (tiers and per-step lots).
    let mut mk = Maker::read(&b, h.maker_off(m));
    mk.seq = next_seq(&mut h)?;
    mk.lo = p.lo;
    mk.hi = p.hi;
    mk.escrow = escrow;
    mk.bid_spread = p.bid_spread;
    mk.ask_spread = p.ask_spread;
    mk.quoted_ts = now;
    mk.write(&mut b, h.maker_off(m));
    check_credit(&b, &h, t)?;
    trace("credit");

    if ladder_arcs(&b, &h) > MAX_BOOK_LADDER_ARCS {
        return Err(FuturesError::MakerCapacity.into());
    }
    trace("count");
    // Post-only: the whole book, ladder included, must stay uncrossed.
    let arcs = book_arcs(&b, &h);
    trace("arcs");
    let prop = Proposal::decode(proposal).map_err(|_| FuturesError::BadProposal)?;
    if !prop.fills.is_empty() || prop.taker_fill != 0 {
        return Err(FuturesError::WouldCross.into());
    }
    let zeros = vec![0u64; arcs.len()];
    rm::verify(&arcs, None, &zeros, 0, &prop.potentials, h.grid, WORK_LIMIT).map_err(
        |e| match e {
            VerifyError::Price { .. } | VerifyError::Fifo { .. } => FuturesError::WouldCross.into(),
            other => verify_error(other),
        },
    )?;
    trace("verify");
    sol_log_data(&[
        b"ARQUOTE1",
        owner.key.as_ref(),
        &[m],
        &mk.seq.to_le_bytes(),
        &p.lo.to_le_bytes(),
        &p.hi.to_le_bytes(),
        &escrow.to_le_bytes(),
        &p.bid_lots.to_le_bytes(),
        &p.ask_lots.to_le_bytes(),
    ]);
    store(&mut b, &h)
}

fn retire_ladder(b: &mut [u8], h: &mut Header, m: u8) -> R<u64> {
    let mut mk = Maker::read(b, h.maker_off(m));
    let released = mk.escrow;
    release(b, h, mk.trader, released)?;
    let mut empty = true;
    for x in 0..h.grid {
        let at = h.step_off(m, x);
        let mut st = Step::read(b, at);
        st.bid_lots = 0;
        st.ask_lots = 0;
        empty &= st.inv == 0;
        st.write(b, at);
    }
    mk.seq = 0;
    mk.escrow = 0;
    if empty {
        let mut tr = trader(b, h, mk.trader);
        tr.maker = 0;
        put_trader(b, h, mk.trader, &tr);
        mk = Maker::default();
    }
    mk.write(b, h.maker_off(m));
    Ok(released)
}

#[inline(never)]
fn cancel_quote(program_id: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    let [market, owner, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let (mut b, mut h) = load(program_id, market)?;
    let t = acting_trader(
        program_id,
        &b,
        &h,
        market.key,
        owner,
        &a[2..],
        Operated::Cancel,
    )?;
    let tr = trader(&b, &h, t);
    if tr.maker == 0 {
        return Err(FuturesError::OrderNotFound.into());
    }
    let released = retire_ladder(&mut b, &mut h, tr.maker - 1)?;
    sol_log_data(&[
        b"ARCANCL1",
        owner.key.as_ref(),
        &u16::MAX.to_le_bytes(),
        &0u64.to_le_bytes(),
        &released.to_le_bytes(),
    ]);
    store(&mut b, &h)
}

// ---------------------------------------------------------------- claim ops

#[inline(never)]
fn split(program_id: &Pubkey, a: &[AccountInfo], i: u32, seq: u64, at: u16) -> ProgramResult {
    let [market, owner, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let (mut b, mut h) = load(program_id, market)?;
    let t = acting_trader(
        program_id,
        &b,
        &h,
        market.key,
        owner,
        &a[2..],
        Operated::Trade,
    )?;
    let mut c = owned_claim(&b, &h, i, seq, t)?;
    if at <= c.lo || at >= c.hi || c.reserved > 0 {
        return Err(FuturesError::InvalidShape.into());
    }
    let right_cost = (u128::from(c.cost) * u128::from(c.hi - at) / u128::from(c.hi - c.lo)) as u64;
    let right = Claim {
        lots: c.lots,
        trader: t,
        side: c.side,
        lo: at,
        hi: c.hi,
        cost: right_cost,
        opened_ts: c.opened_ts,
        ..Claim::default()
    };
    c.hi = at;
    c.cost -= right_cost;
    if c.held {
        // The slot stays its resting order's: both parts move out.
        drop_claim(&mut b, &mut h, i);
        c.held = false;
        new_claim(&mut b, &mut h, c)?;
    } else {
        c.seq = next_seq(&mut h)?;
        put_claim(&mut b, &h, i, &c);
    }
    new_claim(&mut b, &mut h, right)?;
    store(&mut b, &h)
}

#[inline(never)]
fn merge(
    program_id: &Pubkey,
    a: &[AccountInfo],
    keep: (u32, u64),
    absorb: (u32, u64),
) -> ProgramResult {
    let [market, owner, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let (mut b, mut h) = load(program_id, market)?;
    let t = acting_trader(
        program_id,
        &b,
        &h,
        market.key,
        owner,
        &a[2..],
        Operated::Trade,
    )?;
    if keep.0 == absorb.0 {
        return Err(FuturesError::InvalidShape.into());
    }
    let mut k = owned_claim(&b, &h, keep.0, keep.1, t)?;
    let x = owned_claim(&b, &h, absorb.0, absorb.1, t)?;
    if k.side != x.side || k.reserved > 0 || x.reserved > 0 {
        return Err(FuturesError::InvalidShape.into());
    }
    if k.lo == x.lo && k.hi == x.hi {
        k.lots = add(k.lots, x.lots)?;
    } else if k.lots == x.lots && (k.hi == x.lo || x.hi == k.lo) {
        k.lo = k.lo.min(x.lo);
        k.hi = k.hi.max(x.hi);
    } else {
        return Err(FuturesError::InvalidShape.into());
    }
    k.cost = add(k.cost, x.cost)?;
    k.opened_ts = k.opened_ts.min(x.opened_ts);
    if k.held && (k.lo, k.hi) != (x.lo, x.hi) {
        // The kept slot stays its resting order's range: the wider claim
        // moves out.
        drop_claim(&mut b, &mut h, keep.0);
        drop_claim(&mut b, &mut h, absorb.0);
        k.held = false;
        new_claim(&mut b, &mut h, k)?;
    } else {
        k.seq = next_seq(&mut h)?;
        put_claim(&mut b, &h, keep.0, &k);
        drop_claim(&mut b, &mut h, absorb.0);
    }
    store(&mut b, &h)
}

#[inline(never)]
fn redeem(
    program_id: &Pubkey,
    a: &[AccountInfo],
    long: (u32, u64),
    short: (u32, u64),
    lots: u64,
) -> ProgramResult {
    let [market, owner, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let (mut b, mut h) = load(program_id, market)?;
    let t = acting_trader(
        program_id,
        &b,
        &h,
        market.key,
        owner,
        &a[2..],
        Operated::Trade,
    )?;
    no_redemption_past_final(&b, &h, t)?;
    let l = owned_claim(&b, &h, long.0, long.1, t)?;
    let s = owned_claim(&b, &h, short.0, short.1, t)?;
    // Exact complements only: same absolute interval, opposite sides.
    if l.side != Side::Long as u8
        || s.side != Side::Short as u8
        || l.lo != s.lo
        || l.hi != s.hi
        || lots == 0
    {
        return Err(FuturesError::NotComplement.into());
    }
    redeem_pair(&mut b, &mut h, long.0, short.0, lots, [false, false])?;
    store(&mut b, &h)
}

#[inline(never)]
fn transfer(
    program_id: &Pubkey,
    a: &[AccountInfo],
    i: u32,
    seq: u64,
    lots: u64,
    to: &Pubkey,
) -> ProgramResult {
    let [market, owner, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let (mut b, mut h) = load(program_id, market)?;
    let t = owned_trader(&b, &h, owner)?;
    no_redemption_past_final(&b, &h, t)?;
    let mut c = owned_claim(&b, &h, i, seq, t)?;
    if lots == 0 || lots > c.lots - c.reserved || *to == Pubkey::default() {
        return Err(FuturesError::ClaimReserved.into());
    }
    // A theft protection: a claim given to the Earn vault (or a legacy pool)
    // would raise its highest value, which deposits pay, for nothing (a far
    // out-of-the-money piece costs its giver nothing net), and let whoever
    // holds its few shares take what the next deposit overpays. Both are
    // program addresses, off the curve.
    if !on_curve(to) {
        return Err(FuturesError::InvalidAccount.into());
    }
    let r = trader_slot(&mut b, &h, to)?;
    let cost = shrink(&mut c, lots);
    let moved = Claim {
        lots,
        trader: r,
        side: c.side,
        lo: c.lo,
        hi: c.hi,
        cost,
        opened_ts: c.opened_ts,
        ..Claim::default()
    };
    if c.lots == 0 {
        drop_claim(&mut b, &mut h, i);
    } else {
        c.seq = next_seq(&mut h)?;
        put_claim(&mut b, &h, i, &c);
    }
    place(&mut b, &mut h, moved, None, None)?;
    sol_log_data(&[
        b"ARXFER01",
        owner.key.as_ref(),
        to.as_ref(),
        &[moved.side],
        &moved.lo.to_le_bytes(),
        &moved.hi.to_le_bytes(),
        &lots.to_le_bytes(),
    ]);
    store(&mut b, &h)
}

// ---------------------------------------------------------------- settlement

#[inline(never)]
fn finalize(program_id: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    let [market, period, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let (mut b, mut h) = load(program_id, market)?;
    if h.status != STATUS_OPEN {
        return Err(FuturesError::AlreadyFinal.into());
    }
    if *period.key != h.period {
        return Err(FuturesError::InvalidAccount.into());
    }
    let p = load_period_for(&h.oracle_program, period, &h.product_id, h.expiry_ts)?;
    let now = now_ts()?;
    // A failed period, or one the oracle hasn't settled, waits for
    // `CouncilSettle` (any time after expiry).
    if p.status != PeriodStatus::Settled
        || now < h.expiry_ts
        || p.value_decimals != h.value_decimals
    {
        return Err(FuturesError::NotSettled.into());
    }
    h.settlement_fine = settle_fine(p.settlement_value, h.step_value, h.k);
    h.status = STATUS_SETTLED;
    h.settled_ts = now;
    sol_log_data(&[
        b"ARFINL01",
        market.key.as_ref(),
        &[h.status],
        &h.settlement_fine.to_le_bytes(),
        &p.settlement_value.to_le_bytes(),
    ]);
    store(&mut b, &h)
}

/// The fallback when the oracle can't settle: the council (or the upgrade
/// authority of an ungoverned program) names the settlement value. Every value
/// pays out exactly the backing, so any value is solvent.
#[inline(never)]
fn council_settle(program_id: &Pubkey, a: &[AccountInfo], value: u64) -> ProgramResult {
    let [market, period, authority, this_program, program_data, council @ ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(authority)?;
    check_upgrade_authority(
        program_id,
        this_program,
        program_data,
        authority.key,
        council,
        false,
    )?;
    let (mut b, mut h) = load(program_id, market)?;
    if h.status != STATUS_OPEN {
        return Err(FuturesError::AlreadyFinal.into());
    }
    if *period.key != h.period {
        return Err(FuturesError::InvalidAccount.into());
    }
    let p = load_period_for(&h.oracle_program, period, &h.product_id, h.expiry_ts)?;
    let now = now_ts()?;
    match p.status {
        PeriodStatus::Failed => {}
        // A period the oracle hasn't settled: the council may set the value
        // any time after expiry, with no further wait (the user, 2026-10-16:
        // any time after expiry; `docs/futures.md`, "Settlement").
        PeriodStatus::Forming | PeriodStatus::Live if now >= h.expiry_ts => {}
        // Settled in other decimals never finalizes (a market listed before
        // `CreateMarket` checked them).
        PeriodStatus::Settled if p.value_decimals != h.value_decimals => {}
        // The oracle settled: `Finalize` pays its value.
        PeriodStatus::Settled => return Err(FuturesError::InvalidInstruction.into()),
        _ => return Err(FuturesError::NotExpired.into()),
    }
    h.settlement_fine = settle_fine(value, h.step_value, h.k);
    h.status = STATUS_FAILED;
    h.settled_ts = now;
    sol_log_data(&[
        b"ARFINL01",
        market.key.as_ref(),
        &[h.status],
        &h.settlement_fine.to_le_bytes(),
        &value.to_le_bytes(),
    ]);
    store(&mut b, &h)
}

/// After finalization, in a market whose credit hasn't settled or settled
/// short, claims leave only through settlement (`Settle`, `SettleRecovery`):
/// a redemption or a transfer's netting would pay a set in full past the
/// shortfall's cut (the review of 2026-10-18, finding 3). The credit owner's
/// own redemptions repay its debt and stay open to it.
fn no_redemption_past_final(b: &[u8], h: &Header, t: u16) -> R<()> {
    let unsettled = !h.credit_settled() || h.credit_flags & CREDIT_SHORTFALL != 0;
    if h.status != STATUS_OPEN
        && h.has_credit()
        && unsettled
        && trader(b, h, t).owner != h.credit_owner
    {
        return Err(FuturesError::NeedsRecovery.into());
    }
    Ok(())
}

fn need_final(h: &Header) -> R<()> {
    if h.status == STATUS_OPEN {
        return Err(FuturesError::NotSettled.into());
    }
    Ok(())
}

/// Whether trader `t`'s settlement of `payout` may go through `Settle` or
/// `SettleMaker`: in a market with credit, the credit owner's first (its
/// payouts repay its debt), everyone else's once the credit is settled, and
/// a shortfall's winners only through `SettleRecovery`.
fn may_settle(b: &[u8], h: &Header, t: u16, payout: u64) -> R<()> {
    if !h.has_credit() || trader(b, h, t).owner == h.credit_owner {
        return Ok(());
    }
    if !h.credit_settled() {
        return Err(FuturesError::NotSettled.into());
    }
    if h.credit_flags & CREDIT_SHORTFALL != 0 && payout > 0 {
        return Err(FuturesError::NeedsRecovery.into());
    }
    Ok(())
}

/// What maker `m`'s ladder inventory pays at the settlement, zeroing it.
fn inventory_payout(b: &mut [u8], h: &Header, m: u8) -> R<u64> {
    let mut payout = 0u64;
    for x in 0..h.grid {
        let at = h.step_off(m, x);
        let mut st = Step::read(b, at);
        let lots = st.inv.unsigned_abs();
        let per = if st.inv > 0 {
            long_per_lot(x, x + 1, h.settlement_fine, h.k)
        } else {
            short_per_lot(x, x + 1, h.settlement_fine, h.k)
        };
        payout = add(payout, mul(lots, per)?)?;
        st.inv = 0;
        st.write(b, at);
    }
    Ok(payout)
}

#[inline(never)]
fn settle(program_id: &Pubkey, a: &[AccountInfo], claims: &[u32]) -> ProgramResult {
    let [market, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let (mut b, mut h) = load(program_id, market)?;
    need_final(&h)?;
    for &i in claims {
        let c = claim(&b, &h, i)?;
        if c.lots == 0 {
            continue;
        }
        let per = if c.side == Side::Long as u8 {
            long_per_lot(c.lo, c.hi, h.settlement_fine, h.k)
        } else {
            short_per_lot(c.lo, c.hi, h.settlement_fine, h.k)
        };
        let payout = mul(c.lots, per)?;
        may_settle(&b, &h, c.trader, payout)?;
        drop_claim(&mut b, &mut h, i);
        backing_to_free(&mut b, &mut h, c.trader, payout)?;
        h.total_paid_out = add(h.total_paid_out, payout)?;
        let mut tr = trader(&b, &h, c.trader);
        tr.paid_out = add(tr.paid_out, payout)?;
        put_trader(&mut b, &h, c.trader, &tr);
        sol_log_data(&[
            b"ARSETL01",
            tr.owner.as_ref(),
            &i.to_le_bytes(),
            &[c.side],
            &c.lo.to_le_bytes(),
            &c.hi.to_le_bytes(),
            &c.lots.to_le_bytes(),
            &payout.to_le_bytes(),
        ]);
    }
    // Orders whose claims settled cannot fill any more.
    store(&mut b, &h)
}

#[inline(never)]
fn settle_maker(program_id: &Pubkey, a: &[AccountInfo], m: u8) -> ProgramResult {
    let [market, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let (mut b, mut h) = load(program_id, market)?;
    need_final(&h)?;
    if m >= h.maker_cap {
        return Err(FuturesError::OrderNotFound.into());
    }
    let mk = Maker::read(&b, h.maker_off(m));
    if !mk.used {
        return Err(FuturesError::OrderNotFound.into());
    }
    let payout = inventory_payout(&mut b, &h, m)?;
    may_settle(&b, &h, mk.trader, payout)?;
    backing_to_free(&mut b, &mut h, mk.trader, payout)?;
    h.total_paid_out = add(h.total_paid_out, payout)?;
    retire_ladder(&mut b, &mut h, m)?;
    let mut tr = trader(&b, &h, mk.trader);
    tr.paid_out = add(tr.paid_out, payout)?;
    put_trader(&mut b, &h, mk.trader, &tr);
    sol_log_data(&[b"ARMKSE01", tr.owner.as_ref(), &[m], &payout.to_le_bytes()]);
    store(&mut b, &h)
}

#[inline(never)]
fn prune(program_id: &Pubkey, a: &[AccountInfo], orders: &[u16]) -> ProgramResult {
    let [market, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let (mut b, mut h) = load(program_id, market)?;
    need_final(&h)?;
    for &slot in orders {
        if slot < h.order_cap && u64_at(&b, h.order_off(slot)) != 0 {
            remove_order(&mut b, &mut h, slot)?;
        }
    }
    store(&mut b, &h)
}

// ---------------------------------------------------------------- accounts

/// Creates a program-owned PDA account of `len` bytes, paid by `payer`. An
/// address someone already sent lamports to is topped up and taken over, so
/// a gift can't block it.
fn create_pda<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    account: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    len: usize,
    seeds: &[&[u8]],
) -> ProgramResult {
    if *system.key != solana_system_interface::program::ID
        || *account.owner != solana_system_interface::program::ID
        || !account.data_is_empty()
    {
        return Err(FuturesError::InvalidAccount.into());
    }
    need_signer(payer)?;
    let need = Rent::get()?.minimum_balance(len);
    let have = account.lamports();
    if have == 0 {
        return invoke_signed(
            &system_instruction::create_account(
                payer.key,
                account.key,
                need,
                len as u64,
                program_id,
            ),
            &[payer.clone(), account.clone(), system.clone()],
            &[seeds],
        );
    }
    if have < need {
        invoke(
            &system_instruction::transfer(payer.key, account.key, need - have),
            &[payer.clone(), account.clone(), system.clone()],
        )?;
    }
    invoke_signed(
        &system_instruction::allocate(account.key, len as u64),
        &[account.clone(), system.clone()],
        &[seeds],
    )?;
    invoke_signed(
        &system_instruction::assign(account.key, program_id),
        &[account.clone(), system.clone()],
        &[seeds],
    )
}

/// `amount * num / den`, rounded down.
fn mul_div(amount: u64, num: u64, den: u64) -> R<u64> {
    if den == 0 {
        return Err(FuturesError::InsufficientFunds.into());
    }
    to_u64((i128::from(amount) * i128::from(num)) / i128::from(den))
}

/// The cash a token account of the program's holds, for checks after moves.
fn token_amount(info: &AccountInfo) -> R<u64> {
    Ok(token_account(info)?.amount)
}

/// A token transfer `from` -> `to` by `authority`, a PDA of this program
/// (`seeds`).
fn transfer_signed<'a>(
    token_program: &AccountInfo<'a>,
    from: &AccountInfo<'a>,
    to: &AccountInfo<'a>,
    authority: &AccountInfo<'a>,
    amount: u64,
    seeds: &[&[u8]],
) -> ProgramResult {
    invoke_signed(
        &spl_token::instruction::transfer(
            &spl_token::id(),
            from.key,
            to.key,
            authority.key,
            &[],
            amount,
        )?,
        &[
            from.clone(),
            to.clone(),
            authority.clone(),
            token_program.clone(),
        ],
        &[seeds],
    )
}

/// A token transfer from the signing `owner`'s account `from` to `to`.
fn transfer_from<'a>(
    token_program: &AccountInfo<'a>,
    from: &AccountInfo<'a>,
    to: &AccountInfo<'a>,
    owner: &AccountInfo<'a>,
    amount: u64,
) -> ProgramResult {
    let before = token_amount(to)?;
    invoke(
        &spl_token::instruction::transfer(
            &spl_token::id(),
            from.key,
            to.key,
            owner.key,
            &[],
            amount,
        )?,
        &[
            from.clone(),
            to.clone(),
            owner.clone(),
            token_program.clone(),
        ],
    )?;
    if token_amount(to)? != add(before, amount)? {
        return Err(FuturesError::Custody.into());
    }
    Ok(())
}

// ---------------------------------------------------------------- Earn vault

/// The Earn vault account, checked against its address.
fn load_vault(program_id: &Pubkey, info: &AccountInfo) -> R<Vault> {
    if info.owner != program_id {
        return Err(FuturesError::InvalidAccount.into());
    }
    let v = Vault::read(&info.try_borrow_data()?)?;
    let at = Pubkey::create_program_address(
        &[NAMESPACE, VAULT_SEED, v.operator.as_ref(), &[v.bump]],
        program_id,
    )
    .map_err(|_| FuturesError::InvalidAccount)?;
    if at != *info.key {
        return Err(FuturesError::InvalidAccount.into());
    }
    Ok(v)
}

fn store_vault(info: &AccountInfo, v: &Vault, cash: &AccountInfo) -> R<()> {
    // The cash account holds at least the idle cash the vault counts.
    if *cash.key != v.cash_account || token_amount(cash)? < v.cash {
        return Err(FuturesError::Custody.into());
    }
    v.write(&mut info.try_borrow_mut_data()?);
    Ok(())
}

/// The vault's (lowest, highest) value over every settlement outcome: its
/// idle cash plus its trader's value in each of its markets, which `markets`
/// must name exactly, in the vault's order. Markets settle independently, so
/// the extremes of the sum are the sums of the extremes. Where its stake pool
/// lent it credit, a market's value is its equity there at `now`
/// (`state::credit_equity`: less the debt, never below zero, less the
/// stakers' share of the profit), which only grows with the raw value, so
/// the extremes map.
fn vault_value(
    program_id: &Pubkey,
    vault: &Pubkey,
    v: &Vault,
    markets: &[AccountInfo],
    now: u64,
) -> R<(u64, u64)> {
    if markets.len() < v.markets.len() {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let (mut low, mut high) = (u128::from(v.cash), u128::from(v.cash));
    let mut work = Vec::new();
    for (info, want) in markets.iter().zip(&v.markets) {
        if info.key != want || info.owner != program_id {
            return Err(FuturesError::InvalidAccount.into());
        }
        let b = info.try_borrow_data()?;
        let h = Header::read(&b)?;
        if let Some(t) = find_trader(&b, &h, vault) {
            let (mut l, mut hi) =
                trader_value_with(&b, &h, t, &mut work).ok_or(FuturesError::Overflow)?;
            if h.credit_owner == *vault {
                l = credit_equity(&h, l, now);
                hi = credit_equity(&h, hi, now);
            }
            low += u128::from(l);
            high += u128::from(hi);
        }
    }
    let fit = |x: u128| u64::try_from(x).map_err(|_| ProgramError::from(FuturesError::Overflow));
    Ok((fit(low)?, fit(high)?))
}

/// The owner's vault position: loaded, or created (rent from `payer`).
fn vault_position<'a>(
    program_id: &Pubkey,
    vault: &AccountInfo<'a>,
    owner: &Pubkey,
    lp: &AccountInfo<'a>,
    payer: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
) -> R<VaultPosition> {
    if *lp.owner == *program_id {
        let p = VaultPosition::read(&lp.try_borrow_data()?)?;
        if p.vault != *vault.key || p.owner != *owner {
            return Err(FuturesError::InvalidAccount.into());
        }
        return Ok(p);
    }
    let (at, bump) = vault_lp_address(program_id, vault.key, owner);
    if at != *lp.key {
        return Err(FuturesError::InvalidAccount.into());
    }
    let seeds: &[&[u8]] = &[
        NAMESPACE,
        VAULT_LP_SEED,
        vault.key.as_ref(),
        owner.as_ref(),
        &[bump],
    ];
    create_pda(program_id, payer, lp, system, VAULT_LP_LEN, seeds)?;
    Ok(VaultPosition {
        bump,
        vault: *vault.key,
        owner: *owner,
        ..VaultPosition::default()
    })
}

/// Adds `market` to the vault's markets (once).
fn join(v: &mut Vault, market: &Pubkey) -> R<()> {
    if !v.markets.contains(market) {
        if v.markets.len() >= VAULT_MARKETS {
            return Err(FuturesError::VaultCapacity.into());
        }
        v.markets.push(*market);
    }
    Ok(())
}

#[inline(never)]
fn create_vault(program_id: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    let [vault, operator, payer, cash, mint, system, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(operator)?;
    let (at, bump) = vault_address(program_id, operator.key);
    if at != *vault.key {
        return Err(FuturesError::InvalidAccount.into());
    }
    // The vault's own token account of the quote mint. Only its owner (the
    // vault's address) can give it a delegate or close authority; a balance
    // someone sent it is not counted.
    let c = token_account(cash)?;
    if *mint.owner != spl_token::id()
        || c.mint != *mint.key
        || c.owner != at
        || c.delegate.is_some()
        || c.close_authority.is_some()
    {
        return Err(FuturesError::Custody.into());
    }
    let seeds: &[&[u8]] = &[NAMESPACE, VAULT_SEED, operator.key.as_ref(), &[bump]];
    create_pda(program_id, payer, vault, system, VAULT_LEN, seeds)?;
    Vault {
        bump,
        operator: *operator.key,
        quote_mint: *mint.key,
        cash_account: *cash.key,
        created_ts: now_ts()?,
        ..Vault::default()
    }
    .write(&mut vault.try_borrow_mut_data()?);
    sol_log_data(&[
        b"ARVCRT01",
        vault.key.as_ref(),
        operator.key.as_ref(),
        cash.key.as_ref(),
    ]);
    Ok(())
}

#[inline(never)]
fn vault_deposit(program_id: &Pubkey, a: &[AccountInfo], amount: u64) -> ProgramResult {
    let [vault_info, lp_info, owner, owner_token, cash, token_program, system, payer, markets @ ..] =
        a
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(owner)?;
    need_writable(vault_info)?;
    if *token_program.key != spl_token::id() || amount == 0 {
        return Err(FuturesError::InvalidInstruction.into());
    }
    let mut v = load_vault(program_id, vault_info)?;
    if *cash.key != v.cash_account || owner_token.key == cash.key {
        return Err(FuturesError::InvalidAccount.into());
    }
    // Shares at the vault's highest value: a deposit never dilutes the others.
    let now = now_ts()?;
    let (_, high) = vault_value(program_id, vault_info.key, &v, markets, now)?;
    let shares = if v.shares == 0 {
        amount
    } else {
        mul_div(amount, v.shares, high)?
    };
    if shares == 0 {
        return Err(FuturesError::InsufficientFunds.into());
    }
    let mut lp = vault_position(program_id, vault_info, owner.key, lp_info, payer, system)?;
    transfer_from(token_program, owner_token, cash, owner, amount)?;
    reward_touch(&mut v, &mut lp, now)?;
    house_moves(&mut v, &lp, shares, 0)?;
    v.cash = add(v.cash, amount)?;
    v.shares = add(v.shares, shares)?;
    v.deposited = add(v.deposited, amount)?;
    lp.shares = add(lp.shares, shares)?;
    lp.deposited = add(lp.deposited, amount)?;
    lp.write(&mut lp_info.try_borrow_mut_data()?);
    store_vault(vault_info, &v, cash)?;
    sol_log_data(&[
        b"ARVDEP01",
        vault_info.key.as_ref(),
        owner.key.as_ref(),
        &amount.to_le_bytes(),
        &shares.to_le_bytes(),
    ]);
    Ok(())
}

/// Moves up to `want` of the vault's free cash in `market` to its cash
/// account; returns what moved. With `retire`, a withdrawal that needs more
/// than the free cash first retires the vault's ladder there, releasing its
/// escrow (its inventory stays; the operator quotes again): depositors are
/// paid before the maker's quotes.
#[allow(clippy::too_many_arguments)]
fn draw<'a>(
    program_id: &Pubkey,
    market: &AccountInfo<'a>,
    v: &Vault,
    vault: &Pubkey,
    market_vault: &AccountInfo<'a>,
    authority: &AccountInfo<'a>,
    cash: &AccountInfo<'a>,
    token_program: &AccountInfo<'a>,
    want: u64,
    retire: bool,
    work: &mut Vec<i128>,
) -> R<u64> {
    let (mut b, mut h) = load(program_id, market)?;
    if *market_vault.key != h.vault || h.quote_mint != v.quote_mint {
        return Err(FuturesError::InvalidAccount.into());
    }
    let Some(t) = find_trader(&b, &h, vault) else {
        return Ok(0);
    };
    // After expiry the vault's cash there waits for its credit to settle
    // (its stakers' share, or its shortfall).
    if h.credit_owner == *vault && h.status != STATUS_OPEN && !h.credit_settled() {
        return Err(FuturesError::NotSettled.into());
    }
    let tr = trader(&b, &h, t);
    if retire
        && tr.free < want
        && tr.maker > 0
        && Maker::read(&b, h.maker_off(tr.maker - 1)).seq != 0
    {
        let released = retire_ladder(&mut b, &mut h, tr.maker - 1)?;
        sol_log_data(&[
            b"ARCANCL1",
            vault.as_ref(),
            &u16::MAX.to_le_bytes(),
            &0u64.to_le_bytes(),
            &released.to_le_bytes(),
        ]);
    }
    let credit = h.credit_owner == *vault && h.has_credit() && !h.credit_settled();
    // The most the stakers' share of what the vault has earned here can
    // become stays in the market for `CreditSettle`: its best case at full
    // credit weight, since a withdrawal leaves credit a larger part of the
    // capacity from then on (the review of 2026-10-18, finding 7, and its
    // re-review).
    let keep = if credit {
        let (_, high) = trader_value_with(&b, &h, t, work).ok_or(FuturesError::Overflow)?;
        h.staker_share_max(high.saturating_sub(h.debt))
    } else {
        0
    };
    // While it owes, what it takes out raises its exposure and lowers what
    // carries its debt one for one: no more than both rules leave, at worst
    // over every fill of what still rests (`state::credit_risk`).
    let room = if credit && h.debt > 0 {
        let r = credit_risk_with(&b, &h, t, true, work).ok_or(FuturesError::Overflow)?;
        r.carry
            .saturating_sub(h.debt)
            .min(h.credit_limit.saturating_sub(r.exposure))
    } else {
        u64::MAX
    };
    let amount = want
        .min(trader(&b, &h, t).free.saturating_sub(keep))
        .min(room);
    if amount > 0 {
        if h.credit_owner == *vault {
            h.credit_clock(now_ts()?).ok_or(FuturesError::Overflow)?;
            h.funded_out = add(h.funded_out, amount)?;
        }
        debit_free(&mut b, &mut h, t, amount)?;
        h.custody = sub(h.custody, amount)?;
        let bump = [h.authority_bump];
        let seeds: &[&[u8]] = &[NAMESPACE, AUTHORITY_SEED, market.key.as_ref(), &bump];
        let at = Pubkey::create_program_address(seeds, program_id)
            .map_err(|_| FuturesError::InvalidAccount)?;
        if *authority.key != at {
            return Err(FuturesError::InvalidAccount.into());
        }
        let before = token_amount(cash)?;
        transfer_signed(token_program, market_vault, cash, authority, amount, seeds)?;
        if token_amount(cash)? != add(before, amount)? {
            return Err(FuturesError::Custody.into());
        }
        check_vault(&h, market_vault)?;
    }
    store(&mut b, &h)?;
    Ok(amount)
}

#[inline(never)]
fn vault_withdraw(program_id: &Pubkey, a: &[AccountInfo], shares: u64) -> ProgramResult {
    let [vault_info, lp_info, owner, owner_token, cash, token_program, rest @ ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(owner)?;
    need_writable(vault_info)?;
    if *token_program.key != spl_token::id() {
        return Err(FuturesError::InvalidInstruction.into());
    }
    let mut v = load_vault(program_id, vault_info)?;
    if lp_info.owner != program_id {
        return Err(FuturesError::InvalidAccount.into());
    }
    let mut lp = VaultPosition::read(&lp_info.try_borrow_data()?)?;
    if lp.vault != *vault_info.key || lp.owner != *owner.key {
        return Err(FuturesError::InvalidAccount.into());
    }
    let shares = if shares == u64::MAX {
        lp.shares
    } else {
        shares
    };
    if shares == 0 || shares > lp.shares {
        return Err(FuturesError::InsufficientFunds.into());
    }
    if rest.len() < v.markets.len() {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let (markets, draws) = rest.split_at(v.markets.len());
    // Paid at the vault's lowest value: a withdrawal never takes from the rest.
    let now = now_ts()?;
    let (low, high) = vault_value(program_id, vault_info.key, &v, markets, now)?;
    let amount = mul_div(shares, low, v.shares)?;
    // The last shares can't leave while the vault may still pay more than
    // they're paid: nobody would own the rest, and the next deposit would get
    // it for nothing. A lost-funds protection; they leave once its positions
    // settle or close.
    if shares == v.shares && high > amount {
        return Err(FuturesError::NotSettled.into());
    }
    // Idle cash first, then the vault's free cash in the markets named.
    let mut work = Vec::new();
    for pair in draws.chunks(2) {
        let short = amount.saturating_sub(v.cash);
        if short == 0 {
            break;
        }
        let [market_vault, authority] = pair else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };
        let market = markets
            .iter()
            .find(|m| {
                m.try_borrow_data()
                    .ok()
                    .and_then(|d| Header::read(&d).ok())
                    .is_some_and(|h| h.vault == *market_vault.key)
            })
            .ok_or(FuturesError::InvalidAccount)?;
        let got = draw(
            program_id,
            market,
            &v,
            vault_info.key,
            market_vault,
            authority,
            cash,
            token_program,
            short,
            true,
            &mut work,
        )?;
        v.cash = add(v.cash, got)?;
    }
    if amount > v.cash {
        return Err(FuturesError::InsufficientFunds.into());
    }
    if amount > 0 {
        let dest = token_account(owner_token)?;
        if *cash.key != v.cash_account
            || owner_token.key == cash.key
            || dest.owner != *owner.key
            || dest.mint != v.quote_mint
        {
            return Err(FuturesError::Custody.into());
        }
        let bump = [v.bump];
        let seeds: &[&[u8]] = &[NAMESPACE, VAULT_SEED, v.operator.as_ref(), &bump];
        transfer_signed(token_program, cash, owner_token, vault_info, amount, seeds)?;
        v.cash -= amount;
    }
    reward_touch(&mut v, &mut lp, now)?;
    house_moves(&mut v, &lp, 0, shares)?;
    v.shares -= shares;
    v.withdrawn = add(v.withdrawn, amount)?;
    lp.shares -= shares;
    lp.withdrawn = add(lp.withdrawn, amount)?;
    lp.write(&mut lp_info.try_borrow_mut_data()?);
    store_vault(vault_info, &v, cash)?;
    sol_log_data(&[
        b"ARVWDR01",
        vault_info.key.as_ref(),
        owner.key.as_ref(),
        &shares.to_le_bytes(),
        &amount.to_le_bytes(),
    ]);
    Ok(())
}

#[inline(never)]
fn vault_fund(program_id: &Pubkey, a: &[AccountInfo], amount: u64) -> ProgramResult {
    let [market, vault_info, cash, market_vault, token_program, operator, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(operator)?;
    need_writable(vault_info)?;
    if *token_program.key != spl_token::id() || amount == 0 {
        return Err(FuturesError::InvalidInstruction.into());
    }
    let mut v = load_vault(program_id, vault_info)?;
    if *operator.key != v.operator {
        return Err(FuturesError::Unauthorized.into());
    }
    if *cash.key != v.cash_account {
        return Err(FuturesError::InvalidAccount.into());
    }
    if amount > v.cash {
        return Err(FuturesError::InsufficientFunds.into());
    }
    let (mut b, mut h) = load(program_id, market)?;
    if *market_vault.key != h.vault || h.quote_mint != v.quote_mint {
        return Err(FuturesError::InvalidAccount.into());
    }
    join(&mut v, market.key)?;
    let t = trader_slot(&mut b, &h, vault_info.key)?;
    let before = token_amount(market_vault)?;
    let bump = [v.bump];
    let seeds: &[&[u8]] = &[NAMESPACE, VAULT_SEED, v.operator.as_ref(), &bump];
    transfer_signed(token_program, cash, market_vault, vault_info, amount, seeds)?;
    if token_amount(market_vault)? != add(before, amount)? {
        return Err(FuturesError::Custody.into());
    }
    credit_free(&mut b, &mut h, t, amount)?;
    h.custody = add(h.custody, amount)?;
    // Only the credit owner's funding weighs its stakers' share (the review
    // of 2026-10-18, finding 6).
    if h.credit_owner == *vault_info.key {
        h.credit_clock(now_ts()?).ok_or(FuturesError::Overflow)?;
        h.funded_in = add(h.funded_in, amount)?;
    }
    check_vault(&h, market_vault)?;
    store(&mut b, &h)?;
    drop(b);
    v.cash -= amount;
    store_vault(vault_info, &v, cash)?;
    sol_log_data(&[
        b"ARVFND01",
        vault_info.key.as_ref(),
        market.key.as_ref(),
        &amount.to_le_bytes(),
    ]);
    Ok(())
}

#[inline(never)]
fn vault_defund(program_id: &Pubkey, a: &[AccountInfo], amount: u64) -> ProgramResult {
    let [market, vault_info, cash, market_vault, token_program, authority, operator, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(operator)?;
    need_writable(vault_info)?;
    if *token_program.key != spl_token::id() {
        return Err(FuturesError::InvalidInstruction.into());
    }
    let mut v = load_vault(program_id, vault_info)?;
    if *operator.key != v.operator {
        return Err(FuturesError::Unauthorized.into());
    }
    if *cash.key != v.cash_account {
        return Err(FuturesError::InvalidAccount.into());
    }
    let moved = draw(
        program_id,
        market,
        &v,
        vault_info.key,
        market_vault,
        authority,
        cash,
        token_program,
        amount,
        false,
        &mut Vec::new(),
    )?;
    v.cash = add(v.cash, moved)?;
    // A market where the vault now holds nothing leaves its markets.
    let empty = {
        let b = market.try_borrow_data()?;
        let h = Header::read(&b)?;
        find_trader(&b, &h, vault_info.key).is_none_or(|t| trader(&b, &h, t).idle())
    };
    if empty {
        v.markets.retain(|m| m != market.key);
    }
    store_vault(vault_info, &v, cash)?;
    sol_log_data(&[
        b"ARVDFD01",
        vault_info.key.as_ref(),
        market.key.as_ref(),
        &moved.to_le_bytes(),
    ]);
    Ok(())
}

// ---------------------------------------------------------------- Earn rewards

/// SPL Token-2022, the other token program a reward mint may be of (AMBA is
/// a Token-2022 mint with a transfer fee).
const TOKEN_2022: Pubkey = solana_program::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
/// Token-2022's account-type byte after the base layout of an extended
/// account (`AccountType::Mint`, `AccountType::Account`).
const TYPE_MINT: u8 = 1;
const TYPE_ACCOUNT: u8 = 2;

fn is_token_program(key: &Pubkey) -> bool {
    *key == spl_token::id() || *key == TOKEN_2022
}

/// A token account of either token program, by its base layout (a
/// Token-2022 account's extensions follow it).
fn reward_token_account(info: &AccountInfo) -> R<spl_token::state::Account> {
    let base = spl_token::state::Account::LEN;
    let d = info.try_borrow_data()?;
    if !is_token_program(info.owner)
        || d.len() < base
        || (d.len() > base && d[base] != TYPE_ACCOUNT)
    {
        return Err(FuturesError::Custody.into());
    }
    let a = spl_token::state::Account::unpack_from_slice(&d[..base])
        .map_err(|_| FuturesError::Custody)?;
    if a.state != spl_token::state::AccountState::Initialized {
        return Err(FuturesError::Custody.into());
    }
    Ok(a)
}

/// A mint's decimals, either token program.
fn reward_mint_decimals(info: &AccountInfo) -> R<u8> {
    let (base, account) = (spl_token::state::Mint::LEN, spl_token::state::Account::LEN);
    let d = info.try_borrow_data()?;
    if !is_token_program(info.owner)
        || d.len() < base
        || (d.len() > base && (d.len() <= account || d[account] != TYPE_MINT))
    {
        return Err(FuturesError::Custody.into());
    }
    let m =
        spl_token::state::Mint::unpack_from_slice(&d[..base]).map_err(|_| FuturesError::Custody)?;
    if !m.is_initialized {
        return Err(FuturesError::Custody.into());
    }
    Ok(m.decimals)
}

/// Brings the vault's reward stream up to `now` and settles `lp`'s part of
/// it: before their shares change, and before a payment.
fn reward_touch(v: &mut Vault, lp: &mut VaultPosition, now: u64) -> R<()> {
    let shares = v.shares;
    if let Some(r) = v.rewards.as_mut() {
        r.accrue(shares, now).ok_or(FuturesError::Overflow)?;
        r.settle(lp).ok_or(FuturesError::Overflow)?;
    }
    Ok(())
}

/// Keeps the house's shares in step when its position gains `plus` or loses
/// `minus` shares.
fn house_moves(v: &mut Vault, lp: &VaultPosition, plus: u64, minus: u64) -> R<()> {
    if let Some(r) = v.rewards.as_mut().filter(|r| r.house == lp.owner) {
        r.house_shares = sub(add(r.house_shares, plus)?, minus)?;
    }
    Ok(())
}

/// The vault's reward stream and its accounts, checked: the reward account,
/// its mint and the mint's token program.
fn reward_accounts<'v>(
    v: &'v mut Vault,
    account: &AccountInfo,
    mint: &AccountInfo,
    token_program: &AccountInfo,
) -> R<&'v mut Rewards> {
    let r = v.rewards.as_mut().ok_or(FuturesError::InvalidAccount)?;
    if *account.key != r.account
        || *mint.key != r.mint
        || mint.owner != token_program.key
        || !is_token_program(token_program.key)
    {
        return Err(FuturesError::InvalidAccount.into());
    }
    Ok(r)
}

/// A checked transfer of `amount` from the vault's reward account, signed by
/// the vault (`seeds`).
#[allow(clippy::too_many_arguments)]
fn reward_transfer<'a>(
    token_program: &AccountInfo<'a>,
    from: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    to: &AccountInfo<'a>,
    vault: &AccountInfo<'a>,
    amount: u64,
    decimals: u8,
    seeds: &[&[u8]],
) -> ProgramResult {
    // TransferChecked (12): the same instruction in both token programs.
    let mut data = Vec::with_capacity(10);
    data.push(12);
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(decimals);
    invoke_signed(
        &Instruction {
            program_id: *token_program.key,
            accounts: vec![
                AccountMeta::new(*from.key, false),
                AccountMeta::new_readonly(*mint.key, false),
                AccountMeta::new(*to.key, false),
                AccountMeta::new_readonly(*vault.key, true),
            ],
            data,
        },
        &[
            from.clone(),
            mint.clone(),
            to.clone(),
            vault.clone(),
            token_program.clone(),
        ],
        &[seeds],
    )
}

#[inline(never)]
fn vault_rewards(
    program_id: &Pubkey,
    a: &[AccountInfo],
    (start_ts, end_ts, amount): (u64, u64, u64),
    house: &Pubkey,
) -> ProgramResult {
    let [vault_info, operator, payer, mint, account, house_lp, system, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(operator)?;
    need_writable(vault_info)?;
    if end_ts <= start_ts {
        return Err(FuturesError::InvalidInstruction.into());
    }
    let mut v = load_vault(program_id, vault_info)?;
    if *operator.key != v.operator {
        return Err(FuturesError::Unauthorized.into());
    }
    let now = now_ts()?;
    let shares = v.shares;
    match v.rewards.as_mut() {
        Some(r) => {
            if *mint.key != r.mint || *account.key != r.account || *house != r.house {
                return Err(FuturesError::InvalidAccount.into());
            }
            r.accrue(shares, now).ok_or(FuturesError::Overflow)?;
        }
        None => {
            let decimals = reward_mint_decimals(mint)?;
            let t = reward_token_account(account)?;
            if account.owner != mint.owner
                || t.mint != *mint.key
                || t.owner != *vault_info.key
                || t.delegate.is_some()
                || t.close_authority.is_some()
            {
                return Err(FuturesError::Custody.into());
            }
            // The house's shares so far: its position, if it has one.
            if *house_lp.key != vault_lp_address(program_id, vault_info.key, house).0 {
                return Err(FuturesError::InvalidAccount.into());
            }
            let house_shares = if house_lp.owner == program_id {
                VaultPosition::read(&house_lp.try_borrow_data()?)?.shares
            } else {
                0
            };
            if *system.key != solana_system_interface::program::ID {
                return Err(FuturesError::InvalidAccount.into());
            }
            need_signer(payer)?;
            let need = Rent::get()?.minimum_balance(VAULT_REWARDS_LEN);
            let have = vault_info.lamports();
            if have < need {
                invoke(
                    &system_instruction::transfer(payer.key, vault_info.key, need - have),
                    &[payer.clone(), vault_info.clone(), system.clone()],
                )?;
            }
            vault_info.resize(VAULT_REWARDS_LEN)?;
            v.rewards = Some(Rewards {
                mint: *mint.key,
                account: *account.key,
                house: *house,
                decimals,
                last_ts: now,
                house_shares,
                ..Rewards::default()
            });
        }
    }
    let r = v.rewards.as_mut().ok_or(FuturesError::InvalidAccount)?;
    r.start_ts = start_ts;
    r.end_ts = end_ts;
    r.amount = amount;
    v.write(&mut vault_info.try_borrow_mut_data()?);
    sol_log_data(&[
        b"ARVRWD01",
        vault_info.key.as_ref(),
        mint.key.as_ref(),
        &start_ts.to_le_bytes(),
        &end_ts.to_le_bytes(),
        &amount.to_le_bytes(),
    ]);
    Ok(())
}

#[inline(never)]
fn vault_rewards_claim(program_id: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    let [vault_info, lp_info, account, mint, owner_token, token_program, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_writable(vault_info)?;
    need_writable(lp_info)?;
    let mut v = load_vault(program_id, vault_info)?;
    if lp_info.owner != program_id {
        return Err(FuturesError::InvalidAccount.into());
    }
    let mut lp = VaultPosition::read(&lp_info.try_borrow_data()?)?;
    if lp.vault != *vault_info.key {
        return Err(FuturesError::InvalidAccount.into());
    }
    reward_touch(&mut v, &mut lp, now_ts()?)?;
    let bump = [v.bump];
    let operator = v.operator;
    let r = reward_accounts(&mut v, account, mint, token_program)?;
    let pay = lp.reward_pending;
    if pay > 0 {
        // Only into the position owner's own account of the mint.
        let dest = reward_token_account(owner_token)?;
        if owner_token.owner != token_program.key
            || owner_token.key == account.key
            || dest.owner != lp.owner
            || dest.mint != r.mint
        {
            return Err(FuturesError::Custody.into());
        }
        let seeds: &[&[u8]] = &[NAMESPACE, VAULT_SEED, operator.as_ref(), &bump];
        reward_transfer(
            token_program,
            account,
            mint,
            owner_token,
            vault_info,
            pay,
            r.decimals,
            seeds,
        )?;
        r.paid = add(r.paid, pay)?;
        lp.reward_pending = 0;
        lp.reward_paid = lp.reward_paid.saturating_add(pay);
    }
    lp.write(&mut lp_info.try_borrow_mut_data()?);
    v.write(&mut vault_info.try_borrow_mut_data()?);
    sol_log_data(&[
        b"ARVRCL01",
        vault_info.key.as_ref(),
        lp.owner.as_ref(),
        &pay.to_le_bytes(),
    ]);
    Ok(())
}

#[inline(never)]
fn vault_rewards_withdraw(program_id: &Pubkey, a: &[AccountInfo], amount: u64) -> ProgramResult {
    let [vault_info, operator, account, mint, dest, token_program, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(operator)?;
    need_writable(vault_info)?;
    let mut v = load_vault(program_id, vault_info)?;
    if *operator.key != v.operator {
        return Err(FuturesError::Unauthorized.into());
    }
    let (bump, shares, now) = ([v.bump], v.shares, now_ts()?);
    let r = reward_accounts(&mut v, account, mint, token_program)?;
    r.accrue(shares, now).ok_or(FuturesError::Overflow)?;
    // Depositors' accrued rewards and the stream's rest stay.
    let held = reward_token_account(account)?.amount;
    let surplus = held.saturating_sub(r.owed().saturating_add(r.left(now)));
    let amount = if amount == u64::MAX { surplus } else { amount };
    if amount == 0 || amount > surplus {
        return Err(FuturesError::InsufficientFunds.into());
    }
    let to = reward_token_account(dest)?;
    if dest.owner != token_program.key || dest.key == account.key || to.mint != r.mint {
        return Err(FuturesError::Custody.into());
    }
    let decimals = r.decimals;
    let seeds: &[&[u8]] = &[NAMESPACE, VAULT_SEED, operator.key.as_ref(), &bump];
    reward_transfer(
        token_program,
        account,
        mint,
        dest,
        vault_info,
        amount,
        decimals,
        seeds,
    )?;
    v.write(&mut vault_info.try_borrow_mut_data()?);
    sol_log_data(&[
        b"ARVRWX01",
        vault_info.key.as_ref(),
        dest.key.as_ref(),
        &amount.to_le_bytes(),
    ]);
    Ok(())
}

// ---------------------------------------------------------------- staked AMBA

/// The pool's parameters, as `CreateStakePool` and `StakeConfig` take them.
struct StakeParams {
    haircut_bps: u16,
    profit_bps: u16,
    buffer_bps: u16,
    twap_window: u64,
    expiry_window: u64,
}

impl StakeParams {
    fn check(&self) -> R<()> {
        let bps = BPS as u16;
        if self.haircut_bps > bps || self.profit_bps > bps || self.buffer_bps > bps {
            return Err(FuturesError::InvalidInstruction.into());
        }
        Ok(())
    }

    fn apply(&self, p: &mut StakePool) {
        p.haircut_bps = self.haircut_bps;
        p.profit_bps = self.profit_bps;
        p.buffer_bps = self.buffer_bps;
        p.twap_window = self.twap_window;
        p.expiry_window = self.expiry_window;
    }
}

fn load_stake(program_id: &Pubkey, info: &AccountInfo) -> R<StakePool> {
    if info.owner != program_id {
        return Err(FuturesError::InvalidAccount.into());
    }
    let p = StakePool::read(&info.try_borrow_data()?)?;
    let at = Pubkey::create_program_address(
        &[NAMESPACE, STAKE_SEED, p.vault.as_ref(), &[p.bump]],
        program_id,
    )
    .map_err(|_| FuturesError::InvalidAccount)?;
    if at != *info.key {
        return Err(FuturesError::InvalidAccount.into());
    }
    Ok(p)
}

fn store_stake(info: &AccountInfo, p: &StakePool) -> R<()> {
    need_writable(info)?;
    p.write(&mut info.try_borrow_mut_data()?);
    Ok(())
}

/// A pool from before (`STAKE_V1_LEN`, `STAKE_V2_LEN`, `STAKE_V3_LEN`)
/// grown to `STAKE_LEN` (its rent topped up from `payer`), so a rollover's
/// conversion rate, the unstaking shares and the credit markets are kept.
fn grow_stake<'a>(
    info: &AccountInfo<'a>,
    payer: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
) -> R<()> {
    grow_to(info, payer, system, STAKE_LEN)
}

/// A program account shorter than `len` (a pool or position from before)
/// grown to it, its rent topped up from `payer`.
fn grow_to<'a>(
    info: &AccountInfo<'a>,
    payer: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    len: usize,
) -> R<()> {
    if info.data_len() >= len {
        return Ok(());
    }
    if *system.key != solana_system_interface::program::ID {
        return Err(FuturesError::InvalidAccount.into());
    }
    need_signer(payer)?;
    need_writable(info)?;
    let need = Rent::get()?.minimum_balance(len);
    let have = info.lamports();
    if have < need {
        invoke(
            &system_instruction::transfer(payer.key, info.key, need - have),
            &[payer.clone(), info.clone(), system.clone()],
        )?;
    }
    info.resize(len)?;
    Ok(())
}

fn need_operator(p: &StakePool, operator: &AccountInfo) -> R<()> {
    need_signer(operator)?;
    if *operator.key != p.operator {
        return Err(FuturesError::Unauthorized.into());
    }
    Ok(())
}

/// The pool's staked AMBA account, its mint and the mint's token program.
fn stake_amba(
    p: &StakePool,
    account: &AccountInfo,
    mint: &AccountInfo,
    program: &AccountInfo,
) -> R<()> {
    if *account.key != p.amba_account
        || *mint.key != p.amba_mint
        || mint.owner != program.key
        || !is_token_program(program.key)
    {
        return Err(FuturesError::InvalidAccount.into());
    }
    Ok(())
}

/// `info` is `owner`'s own token account of `mint` under `program`, not
/// `not`.
fn owners_account(
    info: &AccountInfo,
    owner: &Pubkey,
    mint: &Pubkey,
    program: &Pubkey,
    not: &Pubkey,
) -> R<()> {
    let t = reward_token_account(info)?;
    if info.owner != program || info.key == not || t.owner != *owner || t.mint != *mint {
        return Err(FuturesError::Custody.into());
    }
    Ok(())
}

/// A checked transfer of `amount` from the signing `owner`'s account.
#[allow(clippy::too_many_arguments)]
fn checked_transfer_from<'a>(
    token_program: &AccountInfo<'a>,
    from: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    to: &AccountInfo<'a>,
    owner: &AccountInfo<'a>,
    amount: u64,
    decimals: u8,
) -> ProgramResult {
    // TransferChecked (12): the same instruction in both token programs.
    let mut data = Vec::with_capacity(10);
    data.push(12);
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(decimals);
    invoke(
        &Instruction {
            program_id: *token_program.key,
            accounts: vec![
                AccountMeta::new(*from.key, false),
                AccountMeta::new_readonly(*mint.key, false),
                AccountMeta::new(*to.key, false),
                AccountMeta::new_readonly(*owner.key, true),
            ],
            data,
        },
        &[
            from.clone(),
            mint.clone(),
            to.clone(),
            owner.clone(),
            token_program.clone(),
        ],
    )
}

/// The owner's stake position: loaded, or created (rent from `payer`).
fn stake_position<'a>(
    program_id: &Pubkey,
    pool: &AccountInfo<'a>,
    owner: &Pubkey,
    info: &AccountInfo<'a>,
    payer: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
) -> R<StakePosition> {
    if *info.owner == *program_id {
        return load_stake_position(program_id, pool.key, info, Some(owner));
    }
    let (at, bump) = stake_position_address(program_id, pool.key, owner);
    if at != *info.key {
        return Err(FuturesError::InvalidAccount.into());
    }
    let seeds: &[&[u8]] = &[
        NAMESPACE,
        STAKE_POS_SEED,
        pool.key.as_ref(),
        owner.as_ref(),
        &[bump],
    ];
    create_pda(program_id, payer, info, system, STAKE_POS_LEN, seeds)?;
    let p = StakePool::read(&pool.try_borrow_data()?)?;
    Ok(StakePosition {
        bump,
        pool: *pool.key,
        owner: *owner,
        epoch: p.epoch,
        index: p.share_index,
        ..StakePosition::default()
    })
}

fn load_stake_position(
    program_id: &Pubkey,
    pool: &Pubkey,
    info: &AccountInfo,
    owner: Option<&Pubkey>,
) -> R<StakePosition> {
    if info.owner != program_id {
        return Err(FuturesError::InvalidAccount.into());
    }
    let p = StakePosition::read(&info.try_borrow_data()?)?;
    if p.pool != *pool || owner.is_some_and(|o| *o != p.owner) {
        return Err(FuturesError::InvalidAccount.into());
    }
    Ok(p)
}

#[inline(never)]
fn create_stake_pool(program_id: &Pubkey, a: &[AccountInfo], params: StakeParams) -> ProgramResult {
    let [pool_info, vault_info, operator, payer, mint, amba, usdc, system, this_program, program_data, council @ ..] =
        a
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(operator)?;
    params.check()?;
    // A pool lends credit against its collateral in any market: only the
    // program's upgrade authority (the council, once governed) creates one,
    // so nobody can lend themselves credit against a token they made (the
    // review of 2026-10-18, finding 1). An immutable program has no pools.
    need_signer(payer)?;
    check_upgrade_authority(
        program_id,
        this_program,
        program_data,
        payer.key,
        council,
        false,
    )?;
    let v = load_vault(program_id, vault_info)?;
    if *operator.key != v.operator {
        return Err(FuturesError::Unauthorized.into());
    }
    let (at, bump) = stake_address(program_id, vault_info.key);
    if at != *pool_info.key {
        return Err(FuturesError::InvalidAccount.into());
    }
    // The pool's own AMBA and USDC accounts: only their owner (the pool's
    // address) can give them a delegate or close authority.
    let decimals = reward_mint_decimals(mint)?;
    let t = reward_token_account(amba)?;
    let u = token_account(usdc)?;
    if amba.owner != mint.owner
        || t.mint != *mint.key
        || t.owner != at
        || t.delegate.is_some()
        || t.close_authority.is_some()
        || u.mint != v.quote_mint
        || u.owner != at
        || u.delegate.is_some()
        || u.close_authority.is_some()
    {
        return Err(FuturesError::Custody.into());
    }
    let seeds: &[&[u8]] = &[NAMESPACE, STAKE_SEED, vault_info.key.as_ref(), &[bump]];
    create_pda(program_id, payer, pool_info, system, STAKE_LEN, seeds)?;
    let mut p = StakePool {
        bump,
        amba_decimals: decimals,
        vault: *vault_info.key,
        operator: v.operator,
        amba_mint: *mint.key,
        amba_account: *amba.key,
        usdc_account: *usdc.key,
        usdc_mint: v.quote_mint,
        created_ts: now_ts()?,
        // It counts every market it lends to from the start.
        credit_counted: true,
        ..StakePool::default()
    };
    params.apply(&mut p);
    p.write(&mut pool_info.try_borrow_mut_data()?);
    sol_log_data(&[
        b"ARSCRT01",
        pool_info.key.as_ref(),
        vault_info.key.as_ref(),
        mint.key.as_ref(),
    ]);
    Ok(())
}

#[inline(never)]
fn stake_config(program_id: &Pubkey, a: &[AccountInfo], params: StakeParams) -> ProgramResult {
    let [pool_info, operator, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let mut p = load_stake(program_id, pool_info)?;
    need_operator(&p, operator)?;
    params.check()?;
    params.apply(&mut p);
    store_stake(pool_info, &p)?;
    sol_log_data(&[
        b"ARSCFG01",
        pool_info.key.as_ref(),
        &p.haircut_bps.to_le_bytes(),
        &p.profit_bps.to_le_bytes(),
        &p.buffer_bps.to_le_bytes(),
        &p.twap_window.to_le_bytes(),
        &p.expiry_window.to_le_bytes(),
    ]);
    Ok(())
}

#[inline(never)]
#[cfg(feature = "operator-price")]
fn stake_price(program_id: &Pubkey, a: &[AccountInfo], price: u64) -> ProgramResult {
    let [pool_info, operator, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let mut p = load_stake(program_id, pool_info)?;
    need_operator(&p, operator)?;
    if price == 0 {
        return Err(FuturesError::InvalidPrice.into());
    }
    let now = now_ts()?;
    p.post(price, now);
    store_stake(pool_info, &p)?;
    sol_log_data(&[
        b"ARSPRC01",
        pool_info.key.as_ref(),
        &price.to_le_bytes(),
        &p.twap.to_le_bytes(),
        &now.to_le_bytes(),
    ]);
    Ok(())
}

#[inline(never)]
fn stake(program_id: &Pubkey, a: &[AccountInfo], amount: u64) -> ProgramResult {
    let [pool_info, pos_info, owner, owner_amba, pool_amba, mint, token_program, system, payer, ..] =
        a
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(owner)?;
    need_writable(pool_info)?;
    if amount == 0 {
        return Err(FuturesError::InvalidInstruction.into());
    }
    let mut p = load_stake(program_id, pool_info)?;
    stake_amba(&p, pool_amba, mint, token_program)?;
    if owner_amba.key == pool_amba.key {
        return Err(FuturesError::InvalidAccount.into());
    }
    let mut pos = stake_position(program_id, pool_info, owner.key, pos_info, payer, system)?;
    let now = now_ts()?;
    // Shares for what arrives (the mint's transfer fee comes out of it).
    let before = reward_token_account(pool_amba)?.amount;
    checked_transfer_from(
        token_program,
        owner_amba,
        mint,
        pool_amba,
        owner,
        amount,
        p.amba_decimals,
    )?;
    let received = sub(reward_token_account(pool_amba)?.amount, before)?;
    // Every staked atom gone to a shortfall, or too few left for the shares
    // this needs to fit: a new share epoch, the old shares converted (a
    // pool from before the rates grows to hold them first).
    if p.rolls_over(received) {
        grow_stake(pool_info, payer, system)?;
        p.begin_epoch(now).ok_or(FuturesError::Overflow)?;
    }
    p.touch(&mut pos, now).ok_or(FuturesError::Overflow)?;
    let shares = p.shares_for(received).ok_or(FuturesError::Overflow)?;
    if shares == 0 {
        return Err(FuturesError::InsufficientFunds.into());
    }
    p.staked = add(p.staked, received)?;
    p.shares = add(p.shares, shares)?;
    p.staked_in = add(p.staked_in, received)?;
    pos.shares = add(pos.shares, shares)?;
    pos.staked_in = add(pos.staked_in, received)?;
    pos.write(&mut pos_info.try_borrow_mut_data()?);
    store_stake(pool_info, &p)?;
    sol_log_data(&[
        b"ARSSTK01",
        pool_info.key.as_ref(),
        owner.key.as_ref(),
        &received.to_le_bytes(),
        &shares.to_le_bytes(),
    ]);
    Ok(())
}

/// `StartUnstake`: the position's `shares` (`u64::MAX`: all not unstaking
/// yet) start their `UNBOND_SECS` timer, restarting it for any already
/// unstaking.
#[inline(never)]
fn start_unstake(program_id: &Pubkey, a: &[AccountInfo], shares: u64) -> ProgramResult {
    let [pool_info, pos_info, owner, payer, system, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(owner)?;
    need_writable(pool_info)?;
    need_writable(pos_info)?;
    let mut p = load_stake(program_id, pool_info)?;
    let mut pos = load_stake_position(program_id, pool_info.key, pos_info, Some(owner.key))?;
    // Settled first: earnings to now, an earlier epoch's shares converted.
    let now = now_ts()?;
    p.touch(&mut pos, now).ok_or(FuturesError::Overflow)?;
    let active = pos.shares.saturating_sub(pos.unbonding);
    let added = shares.min(active);
    if added == 0 {
        return Err(FuturesError::InsufficientFunds.into());
    }
    // Accounts from before the timer grow to hold it.
    grow_to(pool_info, payer, system, STAKE_LEN)?;
    grow_to(pos_info, payer, system, STAKE_POS_LEN)?;
    pos.unbonding = add(pos.unbonding, added)?;
    pos.unbond_ts = now;
    p.unbonding = add(p.unbonding, added)?.min(p.shares);
    pos.write(&mut pos_info.try_borrow_mut_data()?);
    store_stake(pool_info, &p)?;
    let ends = now.saturating_add(UNBOND_SECS);
    sol_log_data(&[
        b"ARSUBD01",
        pool_info.key.as_ref(),
        owner.key.as_ref(),
        &added.to_le_bytes(),
        &pos.unbonding.to_le_bytes(),
        &ends.to_le_bytes(),
    ]);
    Ok(())
}

/// `CancelUnstake`: the position's unstaking shares are active again.
#[inline(never)]
fn cancel_unstake(program_id: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    let [pool_info, pos_info, owner, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(owner)?;
    need_writable(pos_info)?;
    let mut p = load_stake(program_id, pool_info)?;
    let mut pos = load_stake_position(program_id, pool_info.key, pos_info, Some(owner.key))?;
    p.touch(&mut pos, now_ts()?).ok_or(FuturesError::Overflow)?;
    let shares = pos.unbonding;
    if shares == 0 {
        return Err(FuturesError::InsufficientFunds.into());
    }
    p.unbonding = p.unbonding.saturating_sub(shares);
    pos.unbonding = 0;
    pos.unbond_ts = 0;
    pos.write(&mut pos_info.try_borrow_mut_data()?);
    store_stake(pool_info, &p)?;
    sol_log_data(&[
        b"ARSUBC01",
        pool_info.key.as_ref(),
        owner.key.as_ref(),
        &shares.to_le_bytes(),
    ]);
    Ok(())
}

/// `Unstake`: pays unstaking shares whose timer is done. Anyone may send it
/// (the keeper completes finished timers); the AMBA goes only into the
/// owner's own account.
#[inline(never)]
fn unstake(program_id: &Pubkey, a: &[AccountInfo], shares: u64) -> ProgramResult {
    let [pool_info, pos_info, sender, owner_amba, pool_amba, mint, token_program, markets @ ..] = a
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(sender)?;
    need_writable(pos_info)?;
    let mut p = load_stake(program_id, pool_info)?;
    stake_amba(&p, pool_amba, mint, token_program)?;
    let mut pos = load_stake_position(program_id, pool_info.key, pos_info, None)?;
    // Settled first: a position of an earlier epoch holds no shares now.
    let now = now_ts()?;
    p.touch(&mut pos, now).ok_or(FuturesError::Overflow)?;
    let shares = if shares == u64::MAX {
        pos.unbonding
    } else {
        shares
    };
    if shares == 0 || shares > pos.unbonding {
        return Err(FuturesError::InsufficientFunds.into());
    }
    // Anyone but the owner completes all of it, never a part (Codex's
    // review of the timer, 2026-10-18).
    if *sender.key != pos.owner && shares != pos.unbonding {
        return Err(FuturesError::Unauthorized.into());
    }
    // Only once their timer is done (the user, 2026-10-18: 7 days).
    match p.unbond_ends(&pos) {
        Some(ends) if now >= ends => {}
        _ => return Err(FuturesError::NotSettled.into()),
    }
    // Nor while a market it lends to is near expiry or past it unsettled.
    lending_markets(program_id, &p, markets, now)?;
    let amba = p.amba_of(shares);
    // Nobody burns shares for nothing.
    if amba == 0 {
        return Err(FuturesError::InsufficientFunds.into());
    }
    // What stays (unstaking AMBA included) must still back the credit lent;
    // the rest comes out later, as markets settle and the credit returns.
    let left = p.staked - amba;
    if p.allocated > 0 && p.capacity_of(left, now).ok_or(FuturesError::Overflow)? < p.allocated {
        return Err(FuturesError::CreditLimit.into());
    }
    if amba > 0 {
        owners_account(
            owner_amba,
            &pos.owner,
            mint.key,
            token_program.key,
            pool_amba.key,
        )?;
        let seeds: &[&[u8]] = &[NAMESPACE, STAKE_SEED, p.vault.as_ref(), &[p.bump]];
        reward_transfer(
            token_program,
            pool_amba,
            mint,
            owner_amba,
            pool_info,
            amba,
            p.amba_decimals,
            seeds,
        )?;
    }
    p.staked = left;
    p.shares -= shares;
    p.unbonding = p.unbonding.saturating_sub(shares);
    p.unstaked = add(p.unstaked, amba)?;
    pos.shares -= shares;
    pos.unbonding -= shares;
    if pos.unbonding == 0 {
        pos.unbond_ts = 0;
    }
    pos.unstaked = add(pos.unstaked, amba)?;
    pos.write(&mut pos_info.try_borrow_mut_data()?);
    store_stake(pool_info, &p)?;
    sol_log_data(&[
        b"ARSUNS01",
        pool_info.key.as_ref(),
        pos.owner.as_ref(),
        &shares.to_le_bytes(),
        &amba.to_le_bytes(),
    ]);
    Ok(())
}

/// `Unstake`'s check of the markets the pool lends to (Codex's review of the
/// unstake timer, 2026-10-18): once a market is in its last `expiry_window`
/// (the user's rule of no new credit near expiry), its shortfall may be
/// certain, and an unstake then would leave it all to the stakers who stay.
/// So `markets` must be exactly the pool's `credit_markets`, distinct,
/// each holding its credit unsettled (`Header::counts_credit_of`;
/// `InvalidAccount` otherwise), and none may be in that window or past
/// expiry (`NotSettled`: unstaking waits for its `CreditSettle`). A pool
/// from before the count waits while it lends anything.
fn lending_markets(program_id: &Pubkey, p: &StakePool, markets: &[AccountInfo], now: u64) -> R<()> {
    if !p.counted() {
        return Err(FuturesError::NotSettled.into());
    }
    if markets.len() != p.credit_markets as usize {
        return Err(FuturesError::InvalidAccount.into());
    }
    for (i, m) in markets.iter().enumerate() {
        if m.owner != program_id || markets[..i].iter().any(|o| o.key == m.key) {
            return Err(FuturesError::InvalidAccount.into());
        }
        let h = Header::read(&m.try_borrow_data()?)?;
        if !h.counts_credit_of(&p.vault) {
            return Err(FuturesError::InvalidAccount.into());
        }
        if h.holds_unstaking(p.expiry_window, now) {
            return Err(FuturesError::NotSettled.into());
        }
    }
    Ok(())
}

#[inline(never)]
fn stake_claim(program_id: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    let [pool_info, pos_info, reward_account, mint, owner_amba, amba_program, pool_usdc, owner_usdc, token_program, ..] =
        a
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_writable(pos_info)?;
    let mut p = load_stake(program_id, pool_info)?;
    let mut pos = load_stake_position(program_id, pool_info.key, pos_info, None)?;
    p.touch(&mut pos, now_ts()?).ok_or(FuturesError::Overflow)?;
    let (vault, bump) = (p.vault, [p.bump]);
    let seeds: &[&[u8]] = &[NAMESPACE, STAKE_SEED, vault.as_ref(), &bump];
    let amba = pos.reward_pending;
    if amba > 0 {
        let r = p.rewards.as_mut().ok_or(FuturesError::InvalidAccount)?;
        if *reward_account.key != r.account
            || *mint.key != r.mint
            || mint.owner != amba_program.key
            || !is_token_program(amba_program.key)
        {
            return Err(FuturesError::InvalidAccount.into());
        }
        owners_account(
            owner_amba,
            &pos.owner,
            mint.key,
            amba_program.key,
            reward_account.key,
        )?;
        reward_transfer(
            amba_program,
            reward_account,
            mint,
            owner_amba,
            pool_info,
            amba,
            r.decimals,
            seeds,
        )?;
        r.paid = add(r.paid, amba)?;
        pos.reward_pending = 0;
        pos.reward_paid = add(pos.reward_paid, amba)?;
    }
    let usdc = pos.usdc_pending;
    if usdc > 0 {
        if *pool_usdc.key != p.usdc_account || *token_program.key != spl_token::id() {
            return Err(FuturesError::InvalidAccount.into());
        }
        let dest = token_account(owner_usdc)?;
        if owner_usdc.key == pool_usdc.key || dest.owner != pos.owner || dest.mint != p.usdc_mint {
            return Err(FuturesError::Custody.into());
        }
        transfer_signed(token_program, pool_usdc, owner_usdc, pool_info, usdc, seeds)?;
        p.usdc_paid = add(p.usdc_paid, usdc)?;
        pos.usdc_pending = 0;
        pos.usdc_paid = add(pos.usdc_paid, usdc)?;
    }
    pos.write(&mut pos_info.try_borrow_mut_data()?);
    store_stake(pool_info, &p)?;
    sol_log_data(&[
        b"ARSCLM01",
        pool_info.key.as_ref(),
        pos.owner.as_ref(),
        &amba.to_le_bytes(),
        &usdc.to_le_bytes(),
    ]);
    Ok(())
}

#[inline(never)]
fn stake_rewards(
    program_id: &Pubkey,
    a: &[AccountInfo],
    (start_ts, end_ts, amount): (u64, u64, u64),
) -> ProgramResult {
    let [pool_info, operator, mint, account, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let mut p = load_stake(program_id, pool_info)?;
    need_operator(&p, operator)?;
    if end_ts <= start_ts {
        return Err(FuturesError::InvalidInstruction.into());
    }
    let now = now_ts()?;
    let shares = p.shares;
    match p.rewards.as_mut() {
        Some(r) => {
            if *mint.key != r.mint || *account.key != r.account {
                return Err(FuturesError::InvalidAccount.into());
            }
            r.accrue(shares, now).ok_or(FuturesError::Overflow)?;
        }
        None => {
            // AMBA, in an account of the pool's other than its staked one.
            if *mint.key != p.amba_mint || *account.key == p.amba_account {
                return Err(FuturesError::InvalidAccount.into());
            }
            let decimals = reward_mint_decimals(mint)?;
            let t = reward_token_account(account)?;
            if account.owner != mint.owner
                || t.mint != *mint.key
                || t.owner != *pool_info.key
                || t.delegate.is_some()
                || t.close_authority.is_some()
            {
                return Err(FuturesError::Custody.into());
            }
            p.rewards = Some(Rewards {
                mint: *mint.key,
                account: *account.key,
                decimals,
                last_ts: now,
                ..Rewards::default()
            });
        }
    }
    let r = p.rewards.as_mut().ok_or(FuturesError::InvalidAccount)?;
    r.start_ts = start_ts;
    r.end_ts = end_ts;
    r.amount = amount;
    store_stake(pool_info, &p)?;
    sol_log_data(&[
        b"ARSRWD01",
        pool_info.key.as_ref(),
        &start_ts.to_le_bytes(),
        &end_ts.to_le_bytes(),
        &amount.to_le_bytes(),
    ]);
    Ok(())
}

#[inline(never)]
fn stake_rewards_withdraw(program_id: &Pubkey, a: &[AccountInfo], amount: u64) -> ProgramResult {
    let [pool_info, operator, account, mint, dest, token_program, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let mut p = load_stake(program_id, pool_info)?;
    need_operator(&p, operator)?;
    let (vault, bump, shares, now) = (p.vault, [p.bump], p.shares, now_ts()?);
    let r = p.rewards.as_mut().ok_or(FuturesError::InvalidAccount)?;
    if *account.key != r.account
        || *mint.key != r.mint
        || mint.owner != token_program.key
        || !is_token_program(token_program.key)
    {
        return Err(FuturesError::InvalidAccount.into());
    }
    r.accrue(shares, now).ok_or(FuturesError::Overflow)?;
    // Stakers' accrued rewards and the stream's rest stay.
    let held = reward_token_account(account)?.amount;
    let surplus = held.saturating_sub(r.owed().saturating_add(r.left(now)));
    let amount = if amount == u64::MAX { surplus } else { amount };
    if amount == 0 || amount > surplus {
        return Err(FuturesError::InsufficientFunds.into());
    }
    let to = reward_token_account(dest)?;
    if dest.owner != token_program.key || dest.key == account.key || to.mint != r.mint {
        return Err(FuturesError::Custody.into());
    }
    let decimals = r.decimals;
    let seeds: &[&[u8]] = &[NAMESPACE, STAKE_SEED, vault.as_ref(), &bump];
    reward_transfer(
        token_program,
        account,
        mint,
        dest,
        pool_info,
        amount,
        decimals,
        seeds,
    )?;
    store_stake(pool_info, &p)?;
    sol_log_data(&[
        b"ARSRWX01",
        pool_info.key.as_ref(),
        dest.key.as_ref(),
        &amount.to_le_bytes(),
    ]);
    Ok(())
}

// ---------------------------------------------------------------- credit

#[inline(never)]
fn credit_fund(program_id: &Pubkey, a: &[AccountInfo], amount: u64) -> ProgramResult {
    let [market, pool_info, operator, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let mut p = load_stake(program_id, pool_info)?;
    need_operator(&p, operator)?;
    if amount == 0 {
        return Err(FuturesError::InvalidInstruction.into());
    }
    let (mut b, mut h) = load(program_id, market)?;
    if h.status != STATUS_OPEN {
        return Err(FuturesError::NotTradable.into());
    }
    if h.quote_mint != p.usdc_mint || (h.has_credit() && h.credit_owner != p.vault) {
        return Err(FuturesError::InvalidAccount.into());
    }
    let now = now_ts()?;
    let capacity = p.capacity(now).ok_or(FuturesError::Overflow)?;
    if add(p.allocated, amount)? > capacity {
        return Err(FuturesError::CreditLimit.into());
    }
    if !h.has_credit() {
        // The stakers' share counts from here: what the vault holds here now
        // is its funding, at its best case (they share only what it earns
        // beyond that), and the weight's clock starts.
        h.credit_owner = p.vault;
        h.funded_in = match find_trader(&b, &h, &p.vault) {
            Some(t) => trader_value(&b, &h, t).ok_or(FuturesError::Overflow)?.1,
            None => 0,
        };
        h.funded_out = 0;
        h.tw_credit = 0;
        h.tw_cash = 0;
        h.tw_ts = 0;
    }
    // The pool counts the markets holding its credit unsettled, for
    // `Unstake`: this one from its first `CreditFund` (in a pool from before
    // the count, from the first one since it grew to hold it). A count that
    // was complete with nothing lent stays complete.
    if pool_info.data_len() == STAKE_LEN {
        p.credit_counted = p.counted();
        if h.credit_flags & CREDIT_COUNTED == 0 {
            h.credit_flags |= CREDIT_COUNTED;
            p.credit_markets = p
                .credit_markets
                .checked_add(1)
                .ok_or(FuturesError::Overflow)?;
        }
    }
    h.credit_clock(now).ok_or(FuturesError::Overflow)?;
    h.credit_limit = add(h.credit_limit, amount)?;
    h.credit_until = h.expiry_ts.saturating_sub(p.expiry_window);
    h.profit_bps = p.profit_bps;
    p.allocated = add(p.allocated, amount)?;
    let limit = h.credit_limit;
    store(&mut b, &h)?;
    drop(b);
    store_stake(pool_info, &p)?;
    sol_log_data(&[
        b"ARCFND01",
        market.key.as_ref(),
        pool_info.key.as_ref(),
        &amount.to_le_bytes(),
        &limit.to_le_bytes(),
    ]);
    Ok(())
}

#[inline(never)]
fn credit_defund(program_id: &Pubkey, a: &[AccountInfo], amount: u64) -> ProgramResult {
    let [market, pool_info, operator, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let mut p = load_stake(program_id, pool_info)?;
    need_operator(&p, operator)?;
    let (mut b, mut h) = load(program_id, market)?;
    if !h.has_credit() || h.credit_owner != p.vault || h.credit_settled() {
        return Err(FuturesError::InvalidAccount.into());
    }
    // Once final the credit stays for `CreditSettle` (the review of
    // 2026-10-18, finding 5).
    if h.status != STATUS_OPEN {
        return Err(FuturesError::NotTradable.into());
    }
    // Never below what the vault's position there could still cost at worst
    // over every fill of its resting liquidity (`state::credit_exposure`;
    // all of its debt, without a slot), so no fill can exceed the limit.
    let floor = match find_trader(&b, &h, &p.vault) {
        Some(t) => credit_exposure(&b, &h, t).ok_or(FuturesError::Overflow)?,
        None => h.debt,
    };
    let most = h.credit_limit.saturating_sub(floor);
    let amount = if amount == u64::MAX { most } else { amount };
    if amount == 0 || amount > most {
        return Err(FuturesError::CreditLimit.into());
    }
    h.credit_clock(now_ts()?).ok_or(FuturesError::Overflow)?;
    h.credit_limit -= amount;
    p.allocated = p.allocated.saturating_sub(amount);
    p.release_buffer().ok_or(FuturesError::Overflow)?;
    let limit = h.credit_limit;
    store(&mut b, &h)?;
    drop(b);
    store_stake(pool_info, &p)?;
    sol_log_data(&[
        b"ARCDFD01",
        market.key.as_ref(),
        pool_info.key.as_ref(),
        &amount.to_le_bytes(),
        &limit.to_le_bytes(),
    ]);
    Ok(())
}

/// The automatic, published tail rule (the user's notes, 2026-10-17): the
/// vault's own payouts repay its debt (`repay`, as they settle); a profit
/// pays the stakers their credit-weighted share; a shortfall drains their
/// USDC buffer, then their AMBA is set aside for the market's winners at the
/// average price. No vote.
#[inline(never)]
fn credit_settle(program_id: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    let [market, pool_info, market_vault, authority, pool_usdc, token_program, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let mut p = load_stake(program_id, pool_info)?;
    let (mut b, mut h) = load(program_id, market)?;
    need_final(&h)?;
    if !h.has_credit() || h.credit_owner != p.vault {
        return Err(FuturesError::InvalidAccount.into());
    }
    if h.credit_settled() {
        return Err(FuturesError::AlreadyFinal.into());
    }
    if *market_vault.key != h.vault
        || *pool_usdc.key != p.usdc_account
        || *token_program.key != spl_token::id()
    {
        return Err(FuturesError::InvalidAccount.into());
    }
    let now = now_ts()?;
    h.credit_clock(now).ok_or(FuturesError::Overflow)?;
    // The vault's own claims, inventory and orders settle first.
    let vt = find_trader(&b, &h, &p.vault);
    let free = match vt {
        Some(t) => {
            let tr = trader(&b, &h, t);
            if tr.claims > 0 || tr.orders > 0 || tr.reserved > 0 || tr.maker > 0 {
                return Err(FuturesError::NotSettled.into());
            }
            tr.free
        }
        None => 0,
    };
    let bump = [h.authority_bump];
    let market_seeds: &[&[u8]] = &[NAMESPACE, AUTHORITY_SEED, market.key.as_ref(), &bump];
    let at = Pubkey::create_program_address(market_seeds, program_id)
        .map_err(|_| FuturesError::InvalidAccount)?;
    if *authority.key != at {
        return Err(FuturesError::InvalidAccount.into());
    }
    let (vault, pool_bump) = (p.vault, [p.bump]);
    let pool_seeds: &[&[u8]] = &[NAMESPACE, STAKE_SEED, vault.as_ref(), &pool_bump];
    let (mut share, mut cover, mut amba) = (0u64, 0u64, 0u64);
    if h.debt == 0 {
        share = h.staker_share(free);
        if let (Some(t), true) = (vt, share > 0) {
            debit_free(&mut b, &mut h, t, share)?;
            h.custody = sub(h.custody, share)?;
            transfer_signed(
                token_program,
                market_vault,
                pool_usdc,
                authority,
                share,
                market_seeds,
            )?;
            p.profit_in = add(p.profit_in, share)?;
            p.take_profit(share).ok_or(FuturesError::Overflow)?;
        }
    } else {
        cover = p.buffer.min(h.debt);
        if cover > 0 {
            transfer_signed(
                token_program,
                pool_usdc,
                market_vault,
                pool_info,
                cover,
                pool_seeds,
            )?;
            h.custody = add(h.custody, cover)?;
            h.debt -= cover;
            p.buffer -= cover;
            p.covered = add(p.covered, cover)?;
        }
        // `repay` keeps the debt within the open sets' backing (and the
        // vault's cash, all repaid by now), so the winners' payouts carry
        // what the buffer didn't.
        if h.debt > h.backing {
            return Err(FuturesError::Insolvent.into());
        }
        if h.debt > 0 {
            // Never at a stale price: a fresh `PoolPrice` first.
            if !p.price_fresh(now) {
                return Err(FuturesError::NotSettled.into());
            }
            amba = p.amba_for_at_most(h.debt, p.twap_at(now), p.staked);
            p.staked -= amba;
            p.recovery_owed = add(p.recovery_owed, amba)?;
            p.slashed = add(p.slashed, amba)?;
            h.shortfall = h.debt;
            h.recovery_amba = amba;
            h.winners = h.backing;
            h.credit_flags |= CREDIT_SHORTFALL;
        }
    }
    p.allocated = p.allocated.saturating_sub(h.credit_limit);
    h.credit_limit = 0;
    if h.credit_flags & CREDIT_COUNTED != 0 {
        p.credit_markets = p.credit_markets.saturating_sub(1);
    }
    h.credit_flags = (h.credit_flags & !CREDIT_COUNTED) | CREDIT_SETTLED;
    p.release_buffer().ok_or(FuturesError::Overflow)?;
    check_vault(&h, market_vault)?;
    let shortfall = h.shortfall;
    store(&mut b, &h)?;
    drop(b);
    // The pool's USDC account holds its buffer and what stakers are owed.
    if token_amount(pool_usdc)? < add(p.buffer, p.usdc_owed())? {
        return Err(FuturesError::Custody.into());
    }
    store_stake(pool_info, &p)?;
    sol_log_data(&[
        b"ARCSET01",
        market.key.as_ref(),
        pool_info.key.as_ref(),
        &share.to_le_bytes(),
        &cover.to_le_bytes(),
        &shortfall.to_le_bytes(),
        &amba.to_le_bytes(),
    ]);
    Ok(())
}

#[inline(never)]
fn settle_recovery(
    program_id: &Pubkey,
    a: &[AccountInfo],
    claims: &[u32],
    with_maker: bool,
) -> ProgramResult {
    let [market, pool_info, pool_amba, mint, owner_amba, token_program, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let mut p = load_stake(program_id, pool_info)?;
    stake_amba(&p, pool_amba, mint, token_program)?;
    let (mut b, mut h) = load(program_id, market)?;
    need_final(&h)?;
    if h.credit_flags & CREDIT_SHORTFALL == 0 || h.credit_owner != p.vault || h.winners == 0 {
        return Err(FuturesError::InvalidInstruction.into());
    }
    // The owner is whoever owns the AMBA account paid.
    let owner = reward_token_account(owner_amba)?.owner;
    owners_account(
        owner_amba,
        &owner,
        mint.key,
        token_program.key,
        pool_amba.key,
    )?;
    let t = find_trader(&b, &h, &owner).ok_or(FuturesError::InvalidAccount)?;
    let mut gross = 0u64;
    for &i in claims {
        let c = claim(&b, &h, i)?;
        if c.lots == 0 {
            continue;
        }
        if c.trader != t {
            return Err(FuturesError::Unauthorized.into());
        }
        let per = if c.side == Side::Long as u8 {
            long_per_lot(c.lo, c.hi, h.settlement_fine, h.k)
        } else {
            short_per_lot(c.lo, c.hi, h.settlement_fine, h.k)
        };
        let payout = mul(c.lots, per)?;
        drop_claim(&mut b, &mut h, i);
        gross = add(gross, payout)?;
        sol_log_data(&[
            b"ARSETL01",
            owner.as_ref(),
            &i.to_le_bytes(),
            &[c.side],
            &c.lo.to_le_bytes(),
            &c.hi.to_le_bytes(),
            &c.lots.to_le_bytes(),
            &payout.to_le_bytes(),
        ]);
    }
    let tr = trader(&b, &h, t);
    if with_maker && tr.maker > 0 {
        let m = tr.maker - 1;
        gross = add(gross, inventory_payout(&mut b, &h, m)?)?;
        retire_ladder(&mut b, &mut h, m)?;
    }
    // The payout less its part of the shortfall (rounded up, so the
    // shortfall is spread in full), and its part of the AMBA set aside
    // (rounded down, so it never runs out).
    let winners = u128::from(h.winners);
    let part = (u128::from(gross) * u128::from(h.shortfall)).div_ceil(winners);
    let cut = u64::try_from(part)
        .unwrap_or(u64::MAX)
        .min(h.debt)
        .min(gross);
    let amba =
        ((u128::from(gross) * u128::from(h.recovery_amba) / winners) as u64).min(p.recovery_owed);
    let usdc = gross - cut;
    h.backing = sub(h.backing, gross).map_err(|_| FuturesError::Insolvent)?;
    h.debt -= cut;
    credit_free(&mut b, &mut h, t, usdc)?;
    h.total_paid_out = add(h.total_paid_out, usdc)?;
    let mut tr = trader(&b, &h, t);
    tr.paid_out = add(tr.paid_out, usdc)?;
    put_trader(&mut b, &h, t, &tr);
    if amba > 0 {
        let seeds: &[&[u8]] = &[NAMESPACE, STAKE_SEED, p.vault.as_ref(), &[p.bump]];
        reward_transfer(
            token_program,
            pool_amba,
            mint,
            owner_amba,
            pool_info,
            amba,
            p.amba_decimals,
            seeds,
        )?;
        p.recovery_owed -= amba;
    }
    store(&mut b, &h)?;
    drop(b);
    store_stake(pool_info, &p)?;
    sol_log_data(&[
        b"ARRCVR01",
        market.key.as_ref(),
        owner.as_ref(),
        &gross.to_le_bytes(),
        &usdc.to_le_bytes(),
        &amba.to_le_bytes(),
    ]);
    Ok(())
}

// ---------------------------------------------------------------- legacy pools (dev)

/// The owner's position in a legacy pool, checked against its address.
#[cfg(feature = "legacy-pools")]
fn load_lp(program_id: &Pubkey, pool: &Pubkey, owner: &Pubkey, lp: &AccountInfo) -> R<LpPosition> {
    if lp.owner != program_id {
        return Err(FuturesError::InvalidAccount.into());
    }
    let p = LpPosition::read(&lp.try_borrow_data()?)?;
    if p.pool != *pool || p.owner != *owner {
        return Err(FuturesError::InvalidAccount.into());
    }
    Ok(p)
}

#[cfg(feature = "legacy-pools")]
fn pool_value(b: &[u8], h: &Header, t: Option<u16>) -> R<(u64, u64)> {
    match t {
        Some(t) => trader_value(b, h, t).ok_or_else(|| FuturesError::Overflow.into()),
        None => Ok((0, 0)),
    }
}

/// Moves a legacy pool one holder owns into the vault: the pool's trader
/// slot (cash, claims, orders, ladder) becomes the vault's, and the holder
/// gets vault shares for the slot's lowest value at the vault's highest, as
/// a deposit of that value would.
#[cfg(feature = "legacy-pools")]
#[inline(never)]
fn pool_migrate(program_id: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    let [market, pool_info, lp_info, vault_info, vlp_info, holder, operator, payer, system, markets @ ..] =
        a
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(holder)?;
    need_signer(operator)?;
    need_writable(pool_info)?;
    need_writable(lp_info)?;
    need_writable(vault_info)?;
    let mut pool = load_pool(program_id, market.key, pool_info)?;
    let mut lp = load_lp(program_id, pool_info.key, holder.key, lp_info)?;
    let mut v = load_vault(program_id, vault_info)?;
    if pool.operator != v.operator || *operator.key != v.operator {
        return Err(FuturesError::Unauthorized.into());
    }
    // Only a pool one holder owns entirely: nobody else's share moves.
    if pool.shares == 0 || lp.shares != pool.shares {
        return Err(FuturesError::Unauthorized.into());
    }
    let (_, high) = vault_value(program_id, vault_info.key, &v, markets, now_ts()?)?;
    let (mut b, h) = load(program_id, market)?;
    if h.quote_mint != v.quote_mint || find_trader(&b, &h, vault_info.key).is_some() {
        return Err(FuturesError::InvalidAccount.into());
    }
    let t = find_trader(&b, &h, pool_info.key).ok_or(FuturesError::InsufficientFunds)?;
    let (value, _) = pool_value(&b, &h, Some(t))?;
    let shares = if v.shares == 0 {
        value
    } else {
        mul_div(value, v.shares, high)?
    };
    if shares == 0 {
        return Err(FuturesError::InsufficientFunds.into());
    }
    join(&mut v, market.key)?;
    let mut tr = trader(&b, &h, t);
    tr.owner = *vault_info.key;
    put_trader(&mut b, &h, t, &tr);
    store(&mut b, &h)?;
    drop(b);
    let mut vlp = vault_position(program_id, vault_info, holder.key, vlp_info, payer, system)?;
    reward_touch(&mut v, &mut vlp, now_ts()?)?;
    house_moves(&mut v, &vlp, shares, 0)?;
    pool.shares = 0;
    pool.withdrawn = add(pool.withdrawn, value)?;
    lp.shares = 0;
    lp.withdrawn = add(lp.withdrawn, value)?;
    v.shares = add(v.shares, shares)?;
    v.deposited = add(v.deposited, value)?;
    vlp.shares = add(vlp.shares, shares)?;
    vlp.deposited = add(vlp.deposited, value)?;
    pool.write(&mut pool_info.try_borrow_mut_data()?);
    lp.write(&mut lp_info.try_borrow_mut_data()?);
    vlp.write(&mut vlp_info.try_borrow_mut_data()?);
    v.write(&mut vault_info.try_borrow_mut_data()?);
    sol_log_data(&[
        b"ARVMIG01",
        vault_info.key.as_ref(),
        pool_info.key.as_ref(),
        holder.key.as_ref(),
        &value.to_le_bytes(),
        &shares.to_le_bytes(),
    ]);
    Ok(())
}

#[cfg(feature = "legacy-pools")]
#[inline(never)]
fn pool_withdraw(program_id: &Pubkey, a: &[AccountInfo], shares: u64) -> ProgramResult {
    let [market, pool_info, lp_info, owner, owner_token, vault, token_program, auth_info, ..] = a
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    need_signer(owner)?;
    if *token_program.key != spl_token::id() {
        return Err(FuturesError::InvalidInstruction.into());
    }
    let mut pool = load_pool(program_id, market.key, pool_info)?;
    let mut lp = load_lp(program_id, pool_info.key, owner.key, lp_info)?;
    let shares = if shares == u64::MAX {
        lp.shares
    } else {
        shares
    };
    if shares == 0 || shares > lp.shares {
        return Err(FuturesError::InsufficientFunds.into());
    }
    let (mut b, mut h) = load(program_id, market)?;
    let t = find_trader(&b, &h, pool_info.key);
    // Paid at the pool's lowest value: a withdrawal never takes from the rest.
    let (low, high) = pool_value(&b, &h, t)?;
    let amount = mul_div(shares, low, pool.shares)?;
    // The last shares can't leave while the pool still holds value above what
    // they're paid: nobody would own it, and the next deposit would get it
    // for nothing. A lost-funds protection; they leave at settlement, or once
    // the operator closes the open inventory.
    if shares == pool.shares && high > amount {
        return Err(FuturesError::NotSettled.into());
    }
    if amount > 0 {
        let t = t.ok_or(FuturesError::InsufficientFunds)?;
        let dest = token_account(owner_token)?;
        if *vault.key != h.vault
            || owner_token.key == vault.key
            || dest.owner != *owner.key
            || dest.mint != h.quote_mint
        {
            return Err(FuturesError::Custody.into());
        }
        debit_free(&mut b, &mut h, t, amount)?;
        h.custody = sub(h.custody, amount)?;
        let bump = [h.authority_bump];
        let seeds: &[&[u8]] = &[NAMESPACE, AUTHORITY_SEED, market.key.as_ref(), &bump];
        let authority = Pubkey::create_program_address(seeds, program_id)
            .map_err(|_| FuturesError::InvalidAccount)?;
        if *auth_info.key != authority {
            return Err(FuturesError::InvalidAccount.into());
        }
        invoke_signed(
            &spl_token::instruction::transfer(
                &spl_token::id(),
                vault.key,
                owner_token.key,
                &authority,
                &[],
                amount,
            )?,
            &[
                vault.clone(),
                owner_token.clone(),
                auth_info.clone(),
                token_program.clone(),
            ],
            &[seeds],
        )?;
        check_vault(&h, vault)?;
    }
    pool.shares -= shares;
    pool.withdrawn = add(pool.withdrawn, amount)?;
    lp.shares -= shares;
    lp.withdrawn = add(lp.withdrawn, amount)?;
    pool.write(&mut pool_info.try_borrow_mut_data()?);
    lp.write(&mut lp_info.try_borrow_mut_data()?);
    sol_log_data(&[
        b"ARPWDR01",
        pool_info.key.as_ref(),
        owner.key.as_ref(),
        &shares.to_le_bytes(),
        &amount.to_le_bytes(),
    ]);
    store(&mut b, &h)
}

/// `PoolPrice`: the AMBA price from mainnet's pool and Pyth
/// (`crate::price_source`), posted as `StakePrice` posts it.
#[inline(never)]
fn pool_price(program_id: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    let [pool_info, amm, feed, ixs, ..] = a else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    crate::price_source::need_alone(program_id, ixs)?;
    let mut p = load_stake(program_id, pool_info)?;
    let clock = Clock::get()?;
    let now = u64::try_from(clock.unix_timestamp).unwrap_or(0);
    let price = crate::price_source::sample(&p, amm, feed, now, clock.slot)?;
    p.post(price, now);
    store_stake(pool_info, &p)?;
    sol_log_data(&[
        b"ARSPRC01",
        pool_info.key.as_ref(),
        &price.to_le_bytes(),
        &p.twap.to_le_bytes(),
        &now.to_le_bytes(),
    ]);
    Ok(())
}
