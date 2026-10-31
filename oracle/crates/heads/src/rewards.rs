//! Money in (spec §4): the bucket's market-making revenue splits into the fetcher prize pool
//! and the delegators' share. Prizes are fixed cash amounts that scale down pro rata when the
//! pool cannot cover them; delegators are paid pro rata to their delegation on live sources
//! through a reward index (what each live atom has earned since the board opened).

use crate::BPS;

/// The reward index's fixed point: USDC atoms per AMBA atom, times this.
pub const INDEX_SCALE: u128 = 1_000_000_000_000_000_000;

/// Splits `inflow` into `(prize_pool, delegator_share)`; the prize pool is `prize_cut` off the
/// top.
pub fn split_inflow(inflow: u64, prize_cut_bps: u16) -> (u64, u64) {
    let prizes = (u128::from(inflow) * u128::from(prize_cut_bps) / u128::from(BPS)) as u64;
    (prizes, inflow - prizes)
}

fn linear(base: u64, slope_ppb: u64, delegation: u64) -> u64 {
    let extra = u128::from(slope_ppb) * u128::from(delegation) / 1_000_000_000;
    base.saturating_add(u64::try_from(extra).unwrap_or(u64::MAX))
}

/// `fill_prize = base_fill + slope * delegation(head)`: bigger pike, bigger bounty.
pub fn fill_prize(params: &crate::Params, head_delegation: u64) -> u64 {
    linear(
        params.base_fill_prize,
        params.fill_prize_slope_ppb,
        head_delegation,
    )
}

/// `update_prize = base_update + slope * delegation(source)`.
pub fn update_prize(params: &crate::Params, source_delegation: u64) -> u64 {
    linear(
        params.base_update_prize,
        params.update_prize_slope_ppb,
        source_delegation,
    )
}

/// What a prize of `points` pays out of an epoch whose prizes add up to `total_points` and
/// whose pool holds `pool`: in full while the pool covers every prize, else pro rata.
pub fn prize_payout(points: u64, total_points: u64, pool: u64) -> u64 {
    if total_points <= pool {
        return points;
    }
    (u128::from(points) * u128::from(pool) / u128::from(total_points)) as u64
}

/// The reward index after `amount` USDC atoms reach delegators holding `live_delegation` AMBA
/// atoms on live sources. With nobody live the amount is not distributed (`None`: the caller
/// keeps it in the reserve for the next distribution).
pub fn index_after(index: u128, amount: u64, live_delegation: u64) -> Option<u128> {
    if live_delegation == 0 {
        return None;
    }
    Some(index + u128::from(amount) * INDEX_SCALE / u128::from(live_delegation))
}

/// What a delegation of `amount` atoms earned while the index moved from `from` to `to`.
pub fn earned(amount: u64, from: u128, to: u128) -> u64 {
    let gained = to.saturating_sub(from);
    let amount = u128::from(amount);
    // Split the index so `amount * gained` cannot overflow.
    let whole = amount.saturating_mul(gained / INDEX_SCALE);
    let part = amount * (gained % INDEX_SCALE) / INDEX_SCALE;
    u64::try_from(whole.saturating_add(part)).unwrap_or(u64::MAX)
}

/// What an ended prize epoch holds that no prize can claim: with `pool` covering every prize
/// (`total_points <= pool`), everything above the points (each prize is paid in full); short,
/// what is left once every prize is paid (`all_paid`), the pro-rata rounding; else nothing yet.
pub fn prize_surplus(pool: u64, total_points: u64, paid: u64, all_paid: bool) -> u64 {
    if total_points <= pool {
        pool - total_points
    } else if all_paid {
        pool.saturating_sub(paid)
    } else {
        0
    }
}
