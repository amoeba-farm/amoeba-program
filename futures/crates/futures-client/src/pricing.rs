//! Maker pricing: the normal model's fair value of one-step claims, the
//! ladders quoted around it and how many lots a budget funds. The treasury
//! maker (`futures-maker`) and petri's trading bots (`cli/src/bots/`) quote
//! with the same functions.
use crate::{program, Core, Knot, Ladder, Tiers};

/// `erfc` (Numerical Recipes' Chebyshev fit; fractional error < 1.2e-7).
pub fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let r = t
        * (-z * z - 1.265_512_23
            + t * (1.000_023_68
                + t * (0.374_091_96
                    + t * (0.096_784_18
                        + t * (-0.186_288_06
                            + t * (0.278_868_07
                                + t * (-1.135_203_98
                                    + t * (1.488_515_87
                                        + t * (-0.822_152_23 + t * 0.170_872_77)))))))))
            .exp();
    if x >= 0.0 {
        r
    } else {
        2.0 - r
    }
}

/// `P(S > t)` for `S ~ Normal(mu, sigma)`.
pub fn survival(t: f64, mu: f64, sigma: f64) -> f64 {
    0.5 * erfc((t - mu) / (sigma * std::f64::consts::SQRT_2))
}

/// Fair value, in units of the full step, of the one-step claim
/// `[x, x+1)`: `E[clamp(S - x, 0, 1)] = ∫_x^{x+1} P(S > t) dt` (Simpson).
pub fn step_fair(x: f64, mu: f64, sigma: f64) -> f64 {
    let n = 8;
    let h = 1.0 / n as f64;
    let mut sum = survival(x, mu, sigma) + survival(x + 1.0, mu, sigma);
    for i in 1..n {
        let w = if i % 2 == 1 { 4.0 } else { 2.0 };
        sum += w * survival(x + i as f64 * h, mu, sigma);
    }
    (sum * h / 3.0).clamp(0.0, 1.0)
}

/// Standard deviation of the index at expiry, in points: `index * vol *
/// sqrt(years)`, at least a quarter point.
pub fn sigma_points(index: f64, vol: f64, years: f64) -> f64 {
    (index * vol * years.max(0.0).sqrt()).max(0.25)
}

/// A maker ladder around the index: `[lo, hi)` of `2 * half_width` steps
/// (clipped to the grid and to the program's widest ladder), mids from the
/// normal model every four steps (the program interpolates between knots),
/// shifted down by `skew` atoms when the maker is long and kept a spread
/// inside `0..=k` (so every step quotes both sides), symmetric spreads, and
/// `tiers` of depth per step and side.
#[allow(clippy::too_many_arguments)]
pub fn plan_ladder(
    index: f64,
    sigma: f64,
    k: u64,
    grid: u16,
    half_width: u16,
    half_spread: u32,
    skew: i64,
    lots: (u64, u64),
    tiers: Tiers,
) -> (Ladder, Vec<Knot>) {
    let fair = |x: u16| step_fair(f64::from(x), index, sigma);
    plan_ladder_by(
        &fair,
        index,
        k,
        grid,
        half_width,
        half_spread,
        skew,
        lots,
        tiers,
    )
}

/// [`plan_ladder`] under any model, as [`plan_wide_by`] is [`plan_wide`].
#[allow(clippy::too_many_arguments)]
pub fn plan_ladder_by(
    fair: &dyn Fn(u16) -> f64,
    centre: f64,
    k: u64,
    grid: u16,
    half_width: u16,
    half_spread: u32,
    skew: i64,
    lots: (u64, u64),
    tiers: Tiers,
) -> (Ladder, Vec<Knot>) {
    let (lo, hi) = core_span(centre, grid, half_width);
    let mut positions: Vec<u16> = (lo..hi).step_by(4).collect();
    if positions.last() != Some(&(hi - 1)) {
        positions.push(hi - 1);
    }
    let knots = positions
        .into_iter()
        .map(|x| knot(x, fair(x), k, half_spread, skew))
        .collect();
    let ladder = Ladder {
        lo,
        hi,
        bid_spread: half_spread,
        ask_spread: half_spread,
        bid_lots: lots.0,
        ask_lots: lots.1,
        keep: false,
        tiers,
    };
    (ladder, knots)
}

/// The treasury's whole-grid ladder: every depth tier over the core
/// [`core_span`] around the index, and only the last (widest) tier's lots on
/// every other step of the grid, so a range of any width (down to 1x
/// leverage) finds a maker buyer and seller for each of its pieces, to open
/// and to close. Mids follow the model as in [`plan_ladder`], every four steps
/// in the core and every eight outside it.
#[allow(clippy::too_many_arguments)]
pub fn plan_wide(
    index: f64,
    sigma: f64,
    k: u64,
    grid: u16,
    half_width: u16,
    half_spread: u32,
    skew: i64,
    lots: (u64, u64),
    tiers: Tiers,
) -> (Ladder, Core, Vec<Knot>) {
    let fair = |x: u16| step_fair(f64::from(x), index, sigma);
    plan_wide_by(
        &fair,
        index,
        k,
        grid,
        half_width,
        half_spread,
        skew,
        lots,
        tiers,
    )
}

/// [`plan_wide`] under any model: `fair(x)` is what the step `[x, x+1)` is
/// worth as a fraction of the full step, and the core sits around `centre`
/// (a grid position).
#[allow(clippy::too_many_arguments)]
pub fn plan_wide_by(
    fair: &dyn Fn(u16) -> f64,
    centre: f64,
    k: u64,
    grid: u16,
    half_width: u16,
    half_spread: u32,
    skew: i64,
    lots: (u64, u64),
    tiers: Tiers,
) -> (Ladder, Core, Vec<Knot>) {
    let (lo, hi) = core_span(centre, grid, half_width);
    let mut positions: Vec<u16> = (0..lo)
        .step_by(8)
        .chain((lo..hi).step_by(4))
        .chain((hi..grid).step_by(8))
        .collect();
    if positions.last() != Some(&(grid - 1)) {
        positions.push(grid - 1);
    }
    let knots = positions
        .into_iter()
        .map(|x| knot(x, fair(x), k, half_spread, skew))
        .collect();
    let ladder = Ladder {
        lo: 0,
        hi: grid,
        bid_spread: half_spread,
        ask_spread: half_spread,
        bid_lots: lots.0,
        ask_lots: lots.1,
        keep: false,
        tiers,
    };
    (ladder, Core { lo, hi }, knots)
}

/// Lots a step outside the core quotes per side, of `lots` a core step does.
pub fn outer_lots(tiers: Tiers, lots: u64) -> u64 {
    tiers.sizes(lots)[tiers.n() - 1]
}

/// The mid for step `[x, x+1)` worth `fair` of a full step, less the
/// inventory skew. It stays at least a spread inside 0..=k so every step
/// quotes both sides: a far out-of-the-money step bids 0, a deep
/// in-the-money one asks the full step. A range then always has a maker
/// buyer and seller for every piece inside the ladder.
fn knot(x: u16, fair: f64, k: u64, half_spread: u32, skew: i64) -> Knot {
    let fair = fair * k as f64;
    let edge = i64::from(half_spread).min(k as i64 / 2);
    let price = (fair.round() as i64 - skew).clamp(edge, k as i64 - edge);
    Knot {
        pos: x,
        price: price as u32,
    }
}

/// Where the core ladder of `2 * half_width` steps sits around `index`
/// (as [`plan_ladder`] places it; [`plan_wide`]'s core).
pub fn core_span(index: f64, grid: u16, half_width: u16) -> (u16, u16) {
    let width = (2 * half_width).min(program::processor::MAX_LADDER_STEPS);
    let centre = index.round().clamp(1.0, f64::from(grid - 1)) as u16;
    let lo = centre.saturating_sub(width / 2).max(1);
    (lo, (lo + width).min(grid - 1))
}

/// The most lots per core step whose whole-grid ladder fits in `budget`: a
/// step escrows at most one full step (`k`) per lot (the bid plus the ask's
/// complement), the `core` steps quoting `lots` and the `outer` steps the last
/// tier's share. An upper bound, so the program always admits the ladder.
pub fn ladder_lots(budget: u64, k: u64, core: u64, outer: u64, tiers: Tiers) -> u64 {
    let cost = |lots: u64| {
        u128::from(k)
            * (u128::from(lots) * u128::from(core)
                + u128::from(outer_lots(tiers, lots)) * u128::from(outer))
    };
    let share = outer_lots(tiers, 1_000_000) as f64 / 1e6;
    let per_lot = k as f64 * (core as f64 + outer as f64 * share);
    let mut lots = if per_lot > 0.0 {
        (budget as f64 / per_lot).floor() as u64
    } else {
        0
    };
    while lots > 0 && cost(lots) > u128::from(budget) {
        lots -= 1;
    }
    lots
}

/// The most lots `fits` (monotonically false as lots grow; 0 always fits):
/// from `start` up if it fits ([`most_lots`]), else below it by halving.
pub fn fit_lots(start: u64, fits: impl Fn(u64) -> bool) -> u64 {
    if start == 0 || fits(start) {
        return most_lots(start, fits);
    }
    let (mut lo, mut hi) = (0u64, start);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// The most lots `fits`, from `start` (which fits) up: doubling while it
/// fits, then halving the gap. `fits` must grow monotonically false.
pub fn most_lots(start: u64, fits: impl Fn(u64) -> bool) -> u64 {
    let (mut lo, mut hi) = (start, start.max(1));
    while fits(hi) {
        lo = hi;
        match hi.checked_mul(2) {
            Some(next) => hi = next,
            None => return lo,
        }
    }
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_fair_is_a_digital_strip() {
        // Deep in the money pays the full step, far out of it nothing, and the
        // steps of a range add up to the range's fair value.
        assert!((step_fair(50.0, 100.0, 5.0) - 1.0).abs() < 1e-9);
        assert!(step_fair(150.0, 100.0, 5.0) < 1e-9);
        assert!(step_fair(100.0, 99.5, 1e-6) < 1e-6);
        assert!((step_fair(99.0, 100.5, 1e-6) - 1.0).abs() < 1e-6);
        // Around the mean: P(S > 100.5) ~ 0.46 for sigma 5.
        let v = step_fair(100.0, 100.0, 5.0);
        assert!((v - 0.46).abs() < 0.01, "{v}");
        // E[S] = sum of steps: from 0, E[clamp(S, 0, 200)] ~ 100.
        let total: f64 = (0..200).map(|x| step_fair(f64::from(x), 100.0, 5.0)).sum();
        assert!((total - 100.0).abs() < 1e-3, "{total}");
    }

    #[test]
    fn every_step_quotes_both_sides() {
        let (l, knots) = plan_ladder(
            96.7,
            8.5,
            10_000,
            200,
            32,
            100,
            0,
            (10, 10),
            Tiers::default(),
        );
        for kn in &knots {
            assert!(u64::from(kn.price) >= u64::from(l.bid_spread));
            assert!(u64::from(kn.price) + u64::from(l.ask_spread) <= 10_000);
        }
    }

    #[test]
    fn ladder_is_monotone_and_inside_the_grid() {
        let (l, knots) = plan_ladder(
            103.4,
            8.0,
            10_000,
            200,
            40,
            100,
            0,
            (10, 10),
            Tiers::default(),
        );
        assert_eq!((l.lo, l.hi), (63, 143));
        assert!(knots
            .windows(2)
            .all(|w| w[0].pos < w[1].pos && w[0].price >= w[1].price));
        assert_eq!(knots.last().unwrap().pos, 142);
        let (l, _) = plan_ladder(3.0, 1.0, 10_000, 200, 40, 100, 0, (1, 1), Tiers::default());
        assert!(l.lo >= 1 && l.hi <= 199);
    }

    #[test]
    fn wide_ladder_covers_the_grid_around_its_core() {
        let tiers = Tiers {
            count: 5,
            ratio: 120,
            spread: 40,
        };
        let (l, core, knots) =
            plan_wide(103.4, 8.0, 10_000, 200, 36, 100, 0, (7_440, 7_440), tiers);
        assert_eq!((l.lo, l.hi), (0, 200));
        assert_eq!((core.lo, core.hi), core_span(103.4, 200, 36));
        assert_eq!((core.lo, core.hi), (67, 139));
        assert!(knots
            .windows(2)
            .all(|w| w[0].pos < w[1].pos && w[0].price >= w[1].price));
        assert_eq!((knots[0].pos, knots.last().unwrap().pos), (0, 199));
        // Both sides everywhere, the deep ends included.
        for kn in &knots {
            assert!(kn.price >= 100 && kn.price <= 9_900);
        }
        // The outer steps hold the last tier: 7,440 lots in tiers weighted
        // 100:120:144:172:206 (each 120% of the one before, rounded down)
        // put 2,068 in the last.
        assert_eq!(outer_lots(tiers, 7_440), 2_068);
        assert_eq!(outer_lots(Tiers::default(), 7_440), 7_440);
    }

    #[test]
    fn wide_ladder_under_its_own_model_is_the_normal_one() {
        let tiers = Tiers {
            count: 5,
            ratio: 120,
            spread: 40,
        };
        let normal = |x: u16| step_fair(f64::from(x), 103.4, 8.0);
        assert_eq!(
            plan_wide_by(&normal, 103.4, 10_000, 200, 36, 100, 7, (50, 50), tiers),
            plan_wide(103.4, 8.0, 10_000, 200, 36, 100, 7, (50, 50), tiers)
        );
        assert_eq!(
            plan_ladder_by(&normal, 103.4, 10_000, 200, 36, 100, 7, (50, 50), tiers),
            plan_ladder(103.4, 8.0, 10_000, 200, 36, 100, 7, (50, 50), tiers)
        );
    }

    #[test]
    fn fit_lots_searches_below_a_start_that_does_not_fit() {
        for limit in [0u64, 1, 2, 7, 64, 99] {
            assert_eq!(fit_lots(100, |n| n <= limit), limit);
        }
        assert_eq!(fit_lots(10, |n| n <= 1_000), 1_000);
    }

    #[test]
    fn most_lots_is_the_largest_that_fits() {
        assert_eq!(most_lots(10, |n| n * 7 <= 1_000), 142);
        assert_eq!(most_lots(0, |n| n <= 5), 5);
        assert_eq!(most_lots(0, |_| false), 0);
        assert_eq!(most_lots(3, |n| n <= 3), 3);
        assert_eq!(most_lots(1, |_| true), u64::MAX / 2 + 1);
    }

    #[test]
    fn ladder_lots_always_fit_the_budget() {
        let tiers = Tiers {
            count: 5,
            ratio: 120,
            spread: 40,
        };
        let (k, core, outer) = (10_000u64, 64u64, 136u64);
        let cost = |lots: u64| {
            u128::from(k)
                * (u128::from(lots) * u128::from(core)
                    + u128::from(outer_lots(tiers, lots)) * u128::from(outer))
        };
        for budget in [0, 1, 999_999, 1_019_000, 123_456_789, 215_000_000_000u64] {
            let lots = ladder_lots(budget, k, core, outer, tiers);
            assert!(
                cost(lots) <= u128::from(budget),
                "{budget}: {lots} lots overspend"
            );
        }
        // A large budget is spent almost whole: all of the capital quotes.
        let budget = 215_000_000_000u64;
        let lots = ladder_lots(budget, k, core, outer, tiers);
        assert!(
            cost(lots) * 1_000 >= u128::from(budget) * 999,
            "{lots} lots leave cash idle"
        );
    }
}
