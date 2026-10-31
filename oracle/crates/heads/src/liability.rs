//! Slashes and the escrow that holds them (spec §5).
//!
//! A slash takes `slash_rate` of a delegation's remaining principal, and a source retires at
//! `max_strikes`, so a delegation can lose at most `1 - (1 - slash_rate)^max_strikes` of it.
//! On undelegation that much stays in escrow until every exposure has cleared; the rest comes
//! back after the cooldown. Escrow is computed by running the actual slashes, so it always
//! covers exactly what the remaining strikes can take, to the atom.

use crate::BPS;

/// What one strike at `rate_bps` takes from `principal` (rounded down).
pub fn slash(principal: u64, rate_bps: u16) -> u64 {
    (u128::from(principal) * u128::from(rate_bps) / u128::from(BPS)) as u64
}

/// `principal` after `strikes` slashes at `rate_bps`, each on what the last one left.
pub fn after_strikes(principal: u64, rate_bps: u16, strikes: u8) -> u64 {
    let mut left = principal;
    for _ in 0..strikes {
        left -= slash(left, rate_bps);
    }
    left
}

/// What `strikes` more slashes can take from `principal`: the escrow an undelegated
/// delegation keeps while its source has `strikes` strikes left.
pub fn escrow_required(principal: u64, rate_bps: u16, strikes: u8) -> u64 {
    principal - after_strikes(principal, rate_bps, strikes)
}

/// The escrow of a head's proposer: each lost source challenge takes `proposer_slash_bps` of
/// what their delegation still holds, so what every remaining strike's case can take stays (all
/// of it at the default 100%), or the ordinary escrow if that is more.
pub fn proposer_escrow_required(
    principal: u64,
    rate_bps: u16,
    strikes: u8,
    proposer_slash_bps: u16,
) -> u64 {
    escrow_required(principal, rate_bps, strikes).max(escrow_required(
        principal,
        proposer_slash_bps,
        strikes,
    ))
}

/// `1 - (1 - rate)^strikes` in basis points, rounded down (spec §5's `max_liability`; 1,900 at
/// 10% and two strikes).
pub fn liability_bps(rate_bps: u16, strikes: u8) -> u64 {
    let mut keep = u128::from(BPS) * u128::from(BPS);
    for _ in 0..strikes {
        keep = keep * (u128::from(BPS) - u128::from(rate_bps)) / u128::from(BPS);
    }
    ((u128::from(BPS) * u128::from(BPS) - keep) / u128::from(BPS)) as u64
}

/// A delegation's money after undelegation: `principal` is what it would hold if still
/// delegated (slashes come off it), `held` what is still locked. `held - escrow` may go back
/// to the staker once the cooldown is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    pub principal: u64,
    pub held: u64,
}

impl Held {
    /// A strike on a held delegation: the slash comes off the principal and out of what is
    /// held (escrow is slashable exactly as if still delegated). Returns the amount burned.
    pub fn strike(&mut self, rate_bps: u16) -> u64 {
        let taken = slash(self.principal, rate_bps).min(self.held);
        self.principal -= slash(self.principal, rate_bps);
        self.held -= taken;
        taken
    }

    /// What may be released now, given the escrow the remaining strikes still need.
    pub fn releasable(&self, escrow: u64) -> u64 {
        self.held.saturating_sub(escrow)
    }
}
