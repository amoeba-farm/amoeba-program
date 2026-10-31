//! Settlement (spec §2 "Settle", §3, as the user changed it on 2026-10-29): per component, the
//! weighted median of its eligible sources' changes, each weighted by its raw time-weighted
//! delegation over the contract's accumulation window; the index change is the basket-weighted
//! sum of the component changes.
//!
//! No source cap and no per-address cap (the user: "i dont want a hard source cap lol. i wanted
//! to incentivize less sources and more sniping. rather than total spam being incentivized.
//! obviously hard number limit are bad"; on the address cap: "drop it"). A source's weight is
//! exactly the delegation behind it. With the channel layer (`oracle/docs/lta.md`) a component
//! is one `(item, channel)` pair, and eligibility counts independent origins: of the eligible
//! sources sharing an origin only the first (earliest live, then lowest head id) keeps its
//! weight. What a contract settles on across items is [`crate::channels::settlement_change_ppm`].

use crate::channels::{self, Origin, OriginEntry};
use crate::{Params, BPS, PPM};

/// A source's change from `opening` to `level` in ppm, rounded toward zero (`None` for a zero
/// opening or level).
pub fn change_ppm(opening: u64, level: u64) -> Option<i64> {
    if opening == 0 || level == 0 {
        return None;
    }
    let delta = (i128::from(level) - i128::from(opening)) * i128::from(PPM) / i128::from(opening);
    i64::try_from(delta).ok()
}

/// A source's settlement level from its accepted prints `(source_ts, value)`: the median of
/// the prints inside `[start, end]`, else the latest print at or before `start` (the oracle's
/// temporal median, `oracle_temporal_median_state`). `None` without either.
pub fn temporal_level(prints: &[(u64, u64)], start: u64, end: u64) -> Option<u64> {
    let mut inside: Vec<u64> = prints
        .iter()
        .filter(|(ts, _)| (start..=end).contains(ts))
        .map(|&(_, v)| v)
        .collect();
    if !inside.is_empty() {
        return crate::median::median_u64(&mut inside);
    }
    prints
        .iter()
        .filter(|(ts, _)| *ts <= start)
        .max_by_key(|(ts, _)| *ts)
        .map(|&(_, v)| v)
}

/// One source of a component at settlement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceInput {
    /// Live and not retired at settlement.
    pub live: bool,
    /// Its change over the contract (`None` without a level; for an LTA component, a print
    /// past its own next reset is no level: [`crate::channels::lta_level`]).
    pub change_ppm: Option<i64>,
    /// Its raw weight: the time-weighted delegation behind it over the contract's accumulation
    /// window (the window integral divided by the window).
    pub weight: u64,
    /// The origin its current claim declares.
    pub origin: Origin,
    /// When it went live, and its head id: the earlier of two sources with one origin keeps it.
    pub live_at: u64,
    pub head_id: u32,
}

/// Why a component settled on its degraded path (rule 2: its last valid value).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Degraded {
    /// Fewer than `min_live_sources` independent live origins at freeze: it does not settle on
    /// stale sources, whatever is eligible later (the brief, section 5 rule 3).
    FreezeCheckFailed,
    /// Fewer than `min_live_sources` eligible independent origins at settlement.
    TooFewOrigins,
    /// The eligible sources' weight is below `min_settle_weight`.
    TooLittleWeight,
}

/// How one component settled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComponentSettlement {
    /// On live data.
    Live {
        change_ppm: i64,
        /// Eligible sources, one per independent origin.
        eligible_sources: u16,
        /// Otherwise eligible sources dropped because an earlier one has their origin.
        duplicate_sources: u16,
        /// The eligible sources' weight.
        weight: u64,
    },
    /// Failed the freeze check, or below `min_live_sources` eligible origins or
    /// `min_settle_weight`: the component settles at its last value computed with enough of
    /// them (degraded rule 2, flagged with the reason).
    LastValid { change_ppm: i64, reason: Degraded },
}

impl ComponentSettlement {
    pub fn change_ppm(&self) -> i64 {
        match self {
            Self::Live { change_ppm, .. } | Self::LastValid { change_ppm, .. } => *change_ppm,
        }
    }

    pub fn is_live(&self) -> bool {
        matches!(self, Self::Live { .. })
    }
}

/// Which sources are eligible at settlement, after the origin rule: live, with a level and
/// weight, and the first of their origin. Returns `(eligible, duplicate)` per source.
pub fn eligibility(sources: &[SourceInput]) -> (Vec<bool>, Vec<bool>) {
    let entries: Vec<OriginEntry> = sources
        .iter()
        .map(|s| OriginEntry {
            origin: s.origin,
            live_at: s.live_at,
            head_id: s.head_id,
            counts: s.live && s.change_ppm.is_some() && s.weight > 0,
        })
        .collect();
    let duplicate = channels::duplicate_flags(&entries);
    let eligible = entries
        .iter()
        .zip(&duplicate)
        .map(|(e, &d)| e.counts && !d)
        .collect();
    (eligible, duplicate)
}

/// A component's settlement as `SettleComponent` completes it, from its recorded sources: on the
/// sources live at the cutoff (`live`) when the freeze check passed; else, or when they fall short,
/// at its last valid value, which is the settlement of the sources live at the freeze
/// (`at_freeze`, their freeze changes), zero for a failed freeze check or when those fall short
/// too. Returns it with `live`'s eligibility and duplicates (the record keeps them). The contract
/// indication computes the same (round 14 review, N2).
pub fn settle_recorded(
    params: &Params,
    live: &[SourceInput],
    at_freeze: &[SourceInput],
    freeze_passed: bool,
) -> (ComponentSettlement, Vec<bool>, Vec<bool>) {
    let (eligible, duplicate) = eligibility(live);
    let settlement = if freeze_passed {
        settle_eligible(params, live, &eligible, &duplicate, 0)
    } else {
        ComponentSettlement::LastValid {
            change_ppm: 0,
            reason: Degraded::FreezeCheckFailed,
        }
    };
    let settlement = match settlement {
        ComponentSettlement::LastValid { reason, .. } => {
            let change_ppm = match reason {
                // A component that failed its freeze check had too few origins at the freeze
                // too, so its freeze value is the listing value.
                Degraded::FreezeCheckFailed => 0,
                _ => {
                    let (e, d) = eligibility(at_freeze);
                    match settle_eligible(params, at_freeze, &e, &d, 0) {
                        ComponentSettlement::Live { change_ppm, .. } => change_ppm,
                        ComponentSettlement::LastValid { .. } => 0,
                    }
                }
            };
            ComponentSettlement::LastValid { change_ppm, reason }
        }
        live => live,
    };
    (settlement, eligible, duplicate)
}

/// Settles one component: the weighted median of its eligible sources' changes, each at its
/// raw weight. `last_valid_change_ppm` is the component's last change computed on live data
/// (zero at listing); `freeze_passed` whether the component had `min_live_sources` independent
/// live origins at freeze ([`crate::channels::origins_pass`]): one that failed settles on its
/// degraded path.
pub fn settle_component(
    params: &Params,
    sources: &[SourceInput],
    last_valid_change_ppm: i64,
    freeze_passed: bool,
) -> ComponentSettlement {
    let last_valid = |reason| ComponentSettlement::LastValid {
        change_ppm: last_valid_change_ppm,
        reason,
    };
    if !freeze_passed {
        return last_valid(Degraded::FreezeCheckFailed);
    }
    let (eligible, duplicate) = eligibility(sources);
    settle_eligible(
        params,
        sources,
        &eligible,
        &duplicate,
        last_valid_change_ppm,
    )
}

/// [`settle_component`] past its freeze check, with the sources' [`eligibility`] already
/// computed (the program records it, so it is computed once).
pub fn settle_eligible(
    params: &Params,
    sources: &[SourceInput],
    eligible: &[bool],
    duplicate: &[bool],
    last_valid_change_ppm: i64,
) -> ComponentSettlement {
    let last_valid = |reason| ComponentSettlement::LastValid {
        change_ppm: last_valid_change_ppm,
        reason,
    };
    let eligible_count = eligible.iter().filter(|&&e| e).count();
    let duplicate_count = duplicate.iter().filter(|&&d| d).count();
    if !channels::origins_pass(eligible_count, params.min_live_sources) {
        return last_valid(Degraded::TooFewOrigins);
    }
    let mut pairs: Vec<(i64, u64)> = sources
        .iter()
        .zip(eligible)
        .filter(|(_, &e)| e)
        .map(|(s, _)| (s.change_ppm.unwrap_or(0), s.weight))
        .collect();
    let weight: u128 = pairs.iter().map(|&(_, w)| u128::from(w)).sum();
    if weight < u128::from(params.min_settle_weight) {
        return last_valid(Degraded::TooLittleWeight);
    }
    let Some(change_ppm) = crate::median::weighted_median(&mut pairs) else {
        return last_valid(Degraded::TooLittleWeight);
    };
    ComponentSettlement::Live {
        change_ppm,
        eligible_sources: u16::try_from(eligible_count).unwrap_or(u16::MAX),
        duplicate_sources: u16::try_from(duplicate_count).unwrap_or(u16::MAX),
        weight: u64::try_from(weight).unwrap_or(u64::MAX),
    }
}

/// `Σ basket_w_j * change_j` over the components, `(basket_weight_bps, change_ppm)`, rounded
/// toward zero. `None` unless the basket weights add up to 10,000.
pub fn index_change_ppm(components: &[(u16, i64)]) -> Option<i64> {
    let weights: u64 = components.iter().map(|&(w, _)| u64::from(w)).sum();
    if weights != BPS {
        return None;
    }
    let sum: i128 = components
        .iter()
        .map(|&(w, c)| i128::from(w) * i128::from(c))
        .sum();
    i64::try_from(sum / i128::from(BPS)).ok()
}

/// The settlement value `round(base * (1e6 + change) / 1e6)`, half up, at the base's scale;
/// zero at or below a change of -100%.
pub fn settlement_value(base: u64, change_ppm: i64) -> Option<u64> {
    let factor = i128::from(PPM) + i128::from(change_ppm);
    if factor <= 0 {
        return Some(0);
    }
    let scaled = (i128::from(base) * factor + i128::from(PPM) / 2) / i128::from(PPM);
    u64::try_from(scaled).ok()
}
