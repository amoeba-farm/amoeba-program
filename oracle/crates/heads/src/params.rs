//! Per-bucket parameters (spec §7). Amoeba sets them when it opens a board; they never change
//! for a contract once it lists.

/// One bucket's parameters. Amounts: AMBA atoms for delegation, USDC atoms for bonds and prizes
/// (AMBA has nine decimals, USDC six). Rates in basis points. Durations in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    /// Fraction of a delegation's remaining principal one strike slashes (burned). Spec: 10%.
    pub slash_rate_bps: u16,
    /// Lost challenges before a source retires. Spec: 2.
    pub max_strikes: u8,
    /// What a lost source challenge takes of the proposer's delegation, paid to the winning
    /// challengers. Spec: 100%.
    pub proposer_slash_bps: u16,
    /// Share of the bucket's market-making revenue that funds the fetcher prize pool. Spec: 20%.
    pub prize_cut_bps: u16,
    /// `fill_prize = base_fill_prize + fill_prize_slope_ppb * delegation(head) / 1e9`.
    pub base_fill_prize: u64,
    pub fill_prize_slope_ppb: u64,
    /// `update_prize = base_update_prize + update_prize_slope_ppb * delegation(source) / 1e9`.
    pub base_update_prize: u64,
    pub update_prize_slope_ppb: u64,
    /// The USDC bond a claim (fill or update) carries.
    pub claim_bond: u64,
    /// The USDC bond a claim or source challenge carries.
    pub challenge_bond: u64,
    /// How long a claim stays open to challenge.
    pub challenge_window_secs: u64,
    /// How far a late challenge pushes the window out so it can be judged.
    pub challenge_extension_secs: u64,
    /// An OPEN head that has not activated by then expires. Spec: 14 days.
    pub head_ttl_secs: u64,
    /// The unstake cooldown on undelegation. Spec: 14 days.
    pub cooldown_secs: u64,
    /// Delegators a head needs to activate (the proposer alone cannot). Spec: 2; at most 255.
    pub min_delegators: u16,
    /// Independent origins a component needs to list and to settle on live data. Default 1 (the
    /// spec's 3 is a hard number limit, which the user ruled out on 2026-10-29: "obviously hard
    /// number limit are bad"); duplicates never count.
    pub min_live_sources: u16,
    /// Effective weight a component needs to settle on live data.
    pub min_settle_weight: u64,
    /// `min_post = max(floor_post, alpha * D / max(n, 1))`. Spec: alpha 0.25.
    pub alpha_bps: u16,
    pub floor_post: u64,
    /// `activation_threshold = max(floor_activate, beta * m)`. Spec: beta 0.5.
    pub beta_bps: u16,
    pub floor_activate: u64,
    /// Business days before expiry whose accepted prints make a source's settlement level
    /// (the temporal median the oracle has always used).
    pub settlement_business_days: u8,
}

impl Params {
    /// The spec's defaults, with the amounts the spec leaves per bucket filled in for a dev
    /// board (AMBA with nine decimals, USDC with six). Dev boards shorten the windows on their own.
    pub const fn spec_defaults() -> Self {
        const DAY: u64 = 86_400;
        const AMBA: u64 = 1_000_000_000;
        const USDC: u64 = 1_000_000;
        Self {
            slash_rate_bps: 1_000,
            max_strikes: 2,
            proposer_slash_bps: 10_000,
            prize_cut_bps: 2_000,
            base_fill_prize: 5 * USDC,
            fill_prize_slope_ppb: 0,
            base_update_prize: USDC,
            update_prize_slope_ppb: 0,
            claim_bond: USDC,
            challenge_bond: USDC,
            challenge_window_secs: DAY,
            challenge_extension_secs: DAY / 4,
            head_ttl_secs: 14 * DAY,
            cooldown_secs: 14 * DAY,
            min_delegators: 2,
            min_live_sources: 1,
            min_settle_weight: 1_000 * AMBA,
            alpha_bps: 2_500,
            floor_post: 1_000 * AMBA,
            beta_bps: 5_000,
            floor_activate: 2_000 * AMBA,
            settlement_business_days: 5,
        }
    }

    /// `max_liability` of spec §5 in basis points: what `max_strikes` slashes of the remaining
    /// principal take, `1 - (1 - slash_rate)^max_strikes` (19% at the defaults).
    pub fn max_liability_bps(&self) -> u64 {
        crate::liability::liability_bps(self.slash_rate_bps, self.max_strikes)
    }

    /// The checks every implementation applies before accepting a board's parameters.
    pub fn validate(&self) -> bool {
        let bps_ok = |v: u16| u64::from(v) <= crate::BPS;
        bps_ok(self.slash_rate_bps)
            && self.slash_rate_bps > 0
            && self.max_strikes > 0
            && bps_ok(self.proposer_slash_bps)
            && bps_ok(self.prize_cut_bps)
            && bps_ok(self.alpha_bps)
            && bps_ok(self.beta_bps)
            && self.min_delegators >= 1
            // The board's live list keeps each source's delegators in a byte (round 6 review,
            // finding 1); the spec's minimum is 2.
            && self.min_delegators <= u16::from(u8::MAX)
            && self.min_live_sources >= 1
            && self.challenge_window_secs > 0
            && self.settlement_business_days > 0
    }
}

impl Default for Params {
    fn default() -> Self {
        Self::spec_defaults()
    }
}
