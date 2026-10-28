//! Books for the solver's tests and bench.
use ameba_futures_math::{maker_sub, rank, Arc, ArcId, Pos, Side, Taker};

/// xorshift64: deterministic, no dependencies.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Standard normal CDF (Abramowitz-Stegun 7.1.26: plenty for test prices).
fn phi(z: f64) -> f64 {
    let x = z / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let poly = ((((1.061_405_429 * t - 1.453_152_027) * t + 1.421_413_741) * t - 0.284_496_736)
        * t
        + 0.254_829_592)
        * t;
    let erf = 1.0 - poly * (-x * x).exp();
    0.5 * (1.0 + if x >= 0.0 { erf } else { -erf })
}

/// Fair premium of the one-step claim `[x, x+1)`: `k * P(S > x + 0.5)` with
/// the index normal around 100, sd 25.
pub fn fair(x: Pos, k: u64) -> f64 {
    k as f64 * (1.0 - phi((f64::from(x) + 0.5 - 100.0) / 25.0))
}

/// A book shaped like the worst case `docs/futures.md` measures ("wide"):
/// grid 200, 1,024 one-step ladder arcs over the whole grid from two makers
/// (two tiers over every step, one tier over 112 steps) and all 64 order
/// slots resting over varied ranges.
pub fn worst_case_book(grid: Pos, k: u64) -> Vec<Arc> {
    let mut arcs = Vec::new();
    let mut seq = 1u64;
    for (maker, (tiers, steps)) in [(2u8, 0..grid), (1u8, 0..112)].into_iter().enumerate() {
        for x in steps {
            let f = fair(x, k);
            for tier in 0..tiers {
                let w = 0.02 + 0.01 * f64::from(tier);
                let bid = (f * (1.0 - w)).floor().max(0.0) as u64;
                let ask = ((f * (1.0 + w)).ceil() as u64).clamp(1, k);
                for (side, price) in [(Side::Long, bid), (Side::Short, ask)] {
                    if side == Side::Long && price == 0 {
                        continue;
                    }
                    arcs.push(Arc {
                        id: ArcId::Maker {
                            maker: maker as u8,
                            step: x,
                            side,
                            tier,
                        },
                        side,
                        lo: x,
                        hi: x + 1,
                        price,
                        cap: 4_000,
                        rank: rank(seq, maker_sub(x, side, tier)),
                    });
                }
            }
        }
        seq += 1;
    }
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for slot in 0..64u16 {
        let lo = rng.below(180) as Pos;
        let hi = (lo + 1 + rng.below(40) as Pos).min(grid);
        let f: f64 = (lo..hi).map(|x| fair(x, k)).sum();
        let side = if rng.below(2) == 0 {
            Side::Long
        } else {
            Side::Short
        };
        let price = match side {
            Side::Long => (f * 0.97) as u64,
            Side::Short => (f * 1.03) as u64 + 1,
        };
        arcs.push(Arc {
            id: ArcId::Order(slot),
            side,
            lo,
            hi,
            price,
            cap: 50 + rng.below(500),
            rank: rank(seq, 0),
        });
        seq += 1;
    }
    arcs.sort_by_key(|a| a.id);
    arcs
}

/// The three orders measured on that book: a small 1x-style long, a medium
/// range and a whole-grid long.
pub fn worst_case_takers(grid: Pos, k: u64) -> [(&'static str, Taker); 3] {
    let fair_total: u64 = (0..grid).map(|x| fair(x, k) as u64).sum();
    [
        (
            "small: long [95,105), 10 lots",
            Taker {
                side: Side::Long,
                lo: 95,
                hi: 105,
                limit: k * 10,
                qty: 10,
            },
        ),
        (
            "medium: long [60,140), 500 lots",
            Taker {
                side: Side::Long,
                lo: 60,
                hi: 140,
                limit: k * 80,
                qty: 500,
            },
        ),
        (
            "wide: long [0,200), 6,000 lots",
            Taker {
                side: Side::Long,
                lo: 0,
                hi: grid,
                limit: fair_total * 2,
                qty: 6_000,
            },
        ),
    ]
}
