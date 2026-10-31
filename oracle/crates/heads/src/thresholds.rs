//! Posting and activation thresholds, computed from the board (spec §6).

use crate::BPS;

/// `min_post = max(floor_post, alpha * D / max(n, 1))`: posting a head locks about a quarter
/// of a typical live source's backing. `total_delegation` is `D`, every delegation in the
/// bucket; `live_sources` is `n`.
pub fn min_post(
    floor_post: u64,
    alpha_bps: u16,
    total_delegation: u64,
    live_sources: usize,
) -> u64 {
    let n = u128::try_from(live_sources.max(1)).unwrap_or(u128::MAX);
    let scaled = u128::from(total_delegation) * u128::from(alpha_bps) / u128::from(BPS) / n;
    floor_post.max(scaled as u64)
}

/// `activation_threshold = max(floor_activate, beta * m)`, `m` the median delegation of the
/// live sources.
pub fn activation_threshold(
    floor_activate: u64,
    beta_bps: u16,
    live_delegations: &mut [u64],
) -> u64 {
    let m = crate::median::median_u64(live_delegations).unwrap_or(0);
    let scaled = u128::from(m) * u128::from(beta_bps) / u128::from(BPS);
    floor_activate.max(scaled as u64)
}

/// Whether an OPEN head activates: enough delegation from enough delegators. A head clears the
/// threshold once; it does not fall back when the threshold later rises.
pub fn activates(delegation: u64, delegators: u16, threshold: u64, min_delegators: u16) -> bool {
    delegation >= threshold && delegators >= min_delegators
}
