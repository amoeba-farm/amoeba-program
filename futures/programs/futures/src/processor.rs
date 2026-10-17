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

/// Writes the header after checking the market's invariants.
fn store(b: &mut [u8], h: &Header) -> R<()> {
    audit(b, h)?;
    h.write(b);
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
            t.owner == Pubkey::default() || t.idle()
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
/// * custody (deposits minus withdrawals) = free + reserved + backing;
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
    for i in 0..h.order_cap {
        let at = h.order_off(i);
        if u64_at(b, at) != 0 {
            escrow += u128::from(u64_at(b, at + 16)) * u128::from(u64_at(b, at + 32));
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
    for i in 0..h.claim_cap {
        let at = h.claim_off(i);
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
    let backing = u128::from(h.backing);
    let balanced = if open {
        long == backing && short == backing
    } else {
        owed == backing
    };
    if !balanced || u128::from(h.custody) != free + reserved + backing {
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

/// Opens a claim slot for trader `t`.
fn new_claim(b: &mut [u8], h: &mut Header, mut c: Claim) -> R<u32> {
    let cap = h.claim_cap;
    let i = (0..cap)
        .map(|j| (h.claim_hint + j) % cap)
        .find(|&i| u64_at(b, h.claim_off(i)) == 0)
        .ok_or(FuturesError::ClaimCapacity)?;
    h.claim_hint = (i + 1) % cap;
    c.seq = next_seq(h)?;
    put_claim(b, h, i, &c);
    let mut t = trader(b, h, c.trader);
    t.claims += 1;
    put_trader(b, h, c.trader, &t);
    h.claims_open += 1;
    Ok(i)
}

fn drop_claim(b: &mut [u8], h: &mut Header, i: u32) {
    let c = Claim::read(b, h.claim_off(i));
    let mut t = trader(b, h, c.trader);
    t.claims = t.claims.saturating_sub(1);
    put_trader(b, h, c.trader, &t);
    h.claims_open = h.claims_open.saturating_sub(1);
    put_claim(b, h, i, &Claim::default());
}

/// Removes `lots` from a claim with its share of the cost basis.
fn shrink(c: &mut Claim, lots: u64) -> u64 {
    let part = (u128::from(c.cost) * u128::from(lots) / u128::from(c.lots.max(1))) as u64;
    let part = if lots == c.lots { c.cost } else { part };
    c.cost -= part;
    c.lots -= lots;
    part
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
    if cx.trader != cy.trader
        || cx.lo != cy.lo
        || cx.hi != cy.hi
        || cx.side == cy.side
        || cx.lots == 0
        || cy.lots == 0
    {
        return Err(FuturesError::NotComplement.into());
    }
    for (c, r) in [(&mut cx, from_reserved[0]), (&mut cy, from_reserved[1])] {
        let usable = if r { c.lots } else { c.lots - c.reserved };
        if lots > usable {
            return Err(FuturesError::ClaimReserved.into());
        }
        if r {
            c.reserved = c
                .reserved
                .checked_sub(lots)
                .ok_or(FuturesError::ClaimReserved)?;
        }
        shrink(c, lots);
    }
    let cash = mul(lots, set_value(cx.lo, cx.hi, h.k))?;
    let t = cx.trader;
    for (i, c) in [(x, cx), (y, cy)] {
        if c.lots == 0 {
            drop_claim(b, h, i);
        } else {
            let mut c = c;
            c.seq = next_seq(h)?;
            put_claim(b, h, i, &c);
        }
    }
    backing_to_free(b, h, t, cash)?;
    let owner = trader(b, h, t).owner;
    sol_log_data(&[
        b"ARREDM01",
        owner.as_ref(),
        &cx.lo.to_le_bytes(),
        &cx.hi.to_le_bytes(),
        &lots.to_le_bytes(),
        &cash.to_le_bytes(),
    ]);
    Ok(cash)
}

/// Nets a fresh claim against the owner's exact complements: `first` (a
/// close order's claim, using its reserved lots) and then any other.
fn net(b: &mut [u8], h: &mut Header, fresh: u32, first: Option<(u32, bool)>) -> R<()> {
    if let Some((c, reserved)) = first {
        let left = Claim::read(b, h.claim_off(fresh)).lots;
        let other = Claim::read(b, h.claim_off(c));
        let usable = if reserved {
            other.reserved
        } else {
            other.lots - other.reserved
        };
        let lots = left.min(usable);
        if lots > 0 {
            redeem_pair(b, h, fresh, c, lots, [false, reserved])?;
        }
    }
    let n = Claim::read(b, h.claim_off(fresh));
    if n.lots == 0 {
        return Ok(());
    }
    if trader(b, h, n.trader).claims < 2 {
        return Ok(());
    }
    for i in 0..h.claim_cap {
        let at = h.claim_off(i);
        if i == fresh || u64_at(b, at) == 0 || u16_at(b, at + 16) != n.trader {
            continue;
        }
        let c = Claim::read(b, at);
        if c.lots > c.reserved
            && c.trader == n.trader
            && c.side != n.side
            && c.lo == n.lo
            && c.hi == n.hi
        {
            let left = Claim::read(b, h.claim_off(fresh)).lots;
            let lots = left.min(c.lots - c.reserved);
            redeem_pair(b, h, fresh, i, lots, [false, false])?;
            if lots == left {
                return Ok(());
            }
        }
    }
    Ok(())
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
    // Per maker: contribution out of its escrow and lots netted per step.
    let mut makers: Vec<(u8, u64, u64)> = Vec::new();
    // Per maker: its head and tier sizes (the same for every step).
    let mut heads: Vec<(u8, Maker, (TierLots, TierLots))> = Vec::new();
    for (arc, &m) in arcs.iter().zip(&fills) {
        if m == 0 {
            continue;
        }
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
                match makers.iter_mut().find(|x| x.0 == maker) {
                    Some(x) => {
                        x.1 = add(x.1, c)?;
                        x.2 = add(x.2, netted)?;
                    }
                    None => makers.push((maker, c, netted)),
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
    for (maker, c, netted) in makers {
        let at = h.maker_off(maker);
        let mut mk = Maker::read(&b, at);
        mk.escrow = mk.escrow.checked_sub(c).ok_or(FuturesError::Insolvent)?;
        mk.write(&mut b, at);
        reserved_to_backing(&mut b, &mut h, mk.trader, c)?;
        if netted > 0 {
            let cash = mul(netted, h.k)?;
            backing_to_free(&mut b, &mut h, mk.trader, cash)?;
        }
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
        let fresh = new_claim(
            &mut b,
            &mut h,
            Claim {
                lots: filled,
                trader: t,
                side: side as u8,
                lo: p.lo,
                hi: p.hi,
                cost,
                opened_ts: now,
                ..Claim::default()
            },
        )?;
        // Backing comes in first (any refund of a negative premium goes out
        // of it), then a close nets against its claim, then the taker pays.
        if paid >= 0 {
            h.backing = add(h.backing, to_u64(paid)?)?;
        } else {
            backing_to_free(&mut b, &mut h, t, to_u64(-paid)?)?;
        }
        net(&mut b, &mut h, fresh, close.map(|c| (c, false)))?;
        if paid > 0 {
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
        let (escrow_per_lot, flags, claim_ref) = match close {
            Some(c) => {
                let mut cl = Claim::read(&b, h.claim_off(c));
                if cl.lots == 0 || left > cl.lots - cl.reserved {
                    return Err(FuturesError::ClaimReserved.into());
                }
                cl.reserved += left;
                put_claim(&mut b, &h, c, &cl);
                (0, ORDER_CLOSE, c)
            }
            None => (contribution(side, p.lo, p.hi, p.limit, h.k), 0, NO_CLAIM),
        };
        let escrow = mul(left, escrow_per_lot)?;
        reserve(&mut b, &mut h, t, escrow)?;
        Order {
            seq,
            trader: t,
            side: p.side,
            flags,
            lo: p.lo,
            hi: p.hi,
            lots: left,
            price: p.limit,
            escrow_per_lot,
            claim: claim_ref,
            placed_lots: left,
            placed_ts: now,
        }
        .write(&mut b, h.order_off(slot));
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
    let fresh = new_claim(
        b,
        h,
        Claim {
            lots: m,
            trader: t,
            side: o.side,
            lo: o.lo,
            hi: o.hi,
            cost: c,
            opened_ts: now,
            ..Claim::default()
        },
    )?;
    net(b, h, fresh, close.then_some((o.claim, true)))?;
    if close {
        debit_free(b, h, t, c)?;
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
/// Returns the lots netted against opposite inventory (redeemed sets).
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
    // Admission is balance-based: the ladder must be fully funded now.
    reserve(&mut b, &mut h, t, escrow)?;
    // The head as `write_ladder` left it (tiers and per-step lots).
    let mut mk = Maker::read(&b, h.maker_off(m));
    mk.seq = next_seq(&mut h)?;
    mk.lo = p.lo;
    mk.hi = p.hi;
    mk.escrow = escrow;
    mk.bid_spread = p.bid_spread;
    mk.ask_spread = p.ask_spread;
    mk.quoted_ts = now_ts()?;
    mk.write(&mut b, h.maker_off(m));

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
    c.seq = next_seq(&mut h)?;
    put_claim(&mut b, &h, i, &c);
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
    k.seq = next_seq(&mut h)?;
    put_claim(&mut b, &h, keep.0, &k);
    drop_claim(&mut b, &mut h, absorb.0);
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
    let fresh = new_claim(&mut b, &mut h, moved)?;
    net(&mut b, &mut h, fresh, None)?;
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

fn need_final(h: &Header) -> R<()> {
    if h.status == STATUS_OPEN {
        return Err(FuturesError::NotSettled.into());
    }
    Ok(())
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
    let mut payout = 0u64;
    for x in 0..h.grid {
        let at = h.step_off(m, x);
        let mut st = Step::read(&b, at);
        let lots = st.inv.unsigned_abs();
        let per = if st.inv > 0 {
            long_per_lot(x, x + 1, h.settlement_fine, h.k)
        } else {
            short_per_lot(x, x + 1, h.settlement_fine, h.k)
        };
        payout = add(payout, mul(lots, per)?)?;
        st.inv = 0;
        st.write(&mut b, at);
    }
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
/// the extremes of the sum are the sums of the extremes.
fn vault_value(
    program_id: &Pubkey,
    vault: &Pubkey,
    v: &Vault,
    markets: &[AccountInfo],
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
            let (l, hi) = trader_value_with(&b, &h, t, &mut work).ok_or(FuturesError::Overflow)?;
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
    let (_, high) = vault_value(program_id, vault_info.key, &v, markets)?;
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
    reward_touch(&mut v, &mut lp, now_ts()?)?;
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
) -> R<u64> {
    let (mut b, mut h) = load(program_id, market)?;
    if *market_vault.key != h.vault || h.quote_mint != v.quote_mint {
        return Err(FuturesError::InvalidAccount.into());
    }
    let Some(t) = find_trader(&b, &h, vault) else {
        return Ok(0);
    };
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
    let amount = want.min(trader(&b, &h, t).free);
    if amount > 0 {
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
    let (low, high) = vault_value(program_id, vault_info.key, &v, markets)?;
    let amount = mul_div(shares, low, v.shares)?;
    // The last shares can't leave while the vault may still pay more than
    // they're paid: nobody would own the rest, and the next deposit would get
    // it for nothing. A lost-funds protection; they leave once its positions
    // settle or close.
    if shares == v.shares && high > amount {
        return Err(FuturesError::NotSettled.into());
    }
    // Idle cash first, then the vault's free cash in the markets named.
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
    reward_touch(&mut v, &mut lp, now_ts()?)?;
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
    let (_, high) = vault_value(program_id, vault_info.key, &v, markets)?;
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
