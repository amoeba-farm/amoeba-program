//! The solver against the one it replaced (2026-10-23), kept as a test-only
//! reference in `support/reference.rs`. The rules fix the fills, the taker
//! fill and the premium; the potentials and each phase's intermediate flow
//! also depend on which of several equally cheap paths the routing takes.
//! The proposal carries the potentials, so the new solver must take the same
//! paths: on many random books, every phase's flow, the whole `Solution`
//! and the depth levels must be identical, and the package must verify.
mod support;

use ameba_futures_math::{
    depth, maker_sub, rank, solve::solve_with_marks, verify, Arc, ArcId, Pos, Side, Solution, Taker,
};
use support::books::{self, Rng};
use support::{reference, Solver};

type Phases = Vec<(&'static str, Vec<u64>)>;

/// Rounds of a randomized test: `n`, times `EQUIVALENCE_SCALE` if set (a
/// longer soak, e.g. `EQUIVALENCE_SCALE=50 cargo test --release`).
fn rounds(n: usize) -> usize {
    let scale = std::env::var("EQUIVALENCE_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    n * scale
}

fn run(f: Solver, arcs: &[Arc], taker: Option<&Taker>, grid: Pos) -> (Solution, Phases) {
    let mut phases = Vec::new();
    let s = f(arcs, taker, grid, &mut |p, flow| {
        phases.push((p, flow.to_vec()))
    });
    (s, phases)
}

/// Same phases, same flows, same `Solution`; and it verifies (where ranks
/// are unique, as the program keeps them).
fn check(arcs: &[Arc], taker: Option<&Taker>, grid: Pos) -> Solution {
    let (old, old_phases) = run(reference::solve_with_marks, arcs, taker, grid);
    let (new, new_phases) = run(solve_with_marks, arcs, taker, grid);
    assert_eq!(new_phases.len(), 4);
    for ((p, a), (q, b)) in old_phases.iter().zip(&new_phases) {
        assert_eq!(p, q);
        assert_eq!(a, b, "flow after {p}\n{arcs:?}\n{taker:?}");
    }
    assert_eq!(new, old, "solution\n{arcs:?}\n{taker:?}");
    assert_eq!(new, ameba_futures_math::solve(arcs, taker, grid));
    let mut ranks: Vec<u128> = arcs.iter().map(|a| a.rank).collect();
    ranks.sort_unstable();
    ranks.dedup();
    if ranks.len() < arcs.len() {
        return new;
    }
    verify(
        arcs,
        taker,
        &new.fills,
        new.taker_fill,
        &new.potentials,
        grid,
        u64::MAX,
    )
    .unwrap_or_else(|e| panic!("{e:?}\n{arcs:?}\n{taker:?}"));
    new
}

fn check_depth(arcs: &[Arc], t: &Taker, grid: Pos, k: u64) {
    assert_eq!(
        depth(arcs, t.side, t.lo, t.hi, t.qty, grid, k),
        reference::depth(arcs, t.side, t.lo, t.hi, t.qty, grid, k),
        "depth\n{arcs:?}\n{t:?}"
    );
}

fn side(rng: &mut Rng) -> Side {
    if rng.below(2) == 0 {
        Side::Long
    } else {
        Side::Short
    }
}

fn range(rng: &mut Rng, grid: Pos, widest: Pos) -> (Pos, Pos) {
    let lo = rng.below(u64::from(grid)) as Pos;
    let hi = lo + 1 + rng.below(u64::from((grid - lo).min(widest))) as Pos;
    (lo, hi)
}

/// Price of `[lo, hi)` around a fair `step` a step, rounded to `tick` (a
/// coarse tick makes many equal prices: ties in every rule).
fn price(rng: &mut Rng, lo: Pos, hi: Pos, step: u64, spread: u64, tick: u64, side: Side) -> u64 {
    let fair = u64::from(hi - lo) * step;
    let off = rng.below(spread + 1);
    let p = match side {
        Side::Long => fair.saturating_sub(off),
        Side::Short => fair + off,
    };
    p / tick * tick
}

/// Orders over random ranges; crossed books included unless `uncrossed`.
fn random_orders(rng: &mut Rng, grid: Pos, n: usize, k: u64, uncrossed: bool) -> Vec<Arc> {
    let mut ranks: Vec<u128> = (0..n as u128).collect();
    for i in (1..n).rev() {
        ranks.swap(i, rng.below(i as u64 + 1) as usize);
    }
    let tick = [1, k / 2, k][rng.below(3) as usize].max(1);
    (0..n)
        .map(|i| {
            let (lo, hi) = range(rng, grid, grid);
            let s = side(rng);
            let p = if uncrossed {
                price(rng, lo, hi, k / 2, k / 2, tick, s)
            } else {
                rng.below(u64::from(hi - lo) * k + 1) / tick * tick
            };
            Arc {
                id: ArcId::Order(i as u16),
                side: s,
                lo,
                hi,
                price: p,
                // Now and then an arc with no lots left.
                cap: rng.below(13),
                rank: ranks[i],
            }
        })
        .collect()
}

/// Maker ladders of one-step arcs (bid below, ask above a fair price per
/// step, a few tiers) plus resting orders, ranked like the program ranks
/// them.
fn ladder_book(rng: &mut Rng, grid: Pos, makers: u8, orders: usize, k: u64) -> Vec<Arc> {
    let mut arcs = Vec::new();
    let mut seq = 1;
    let tick = [1, k / 4, k / 2][rng.below(3) as usize].max(1);
    for maker in 0..makers {
        let (lo, hi) = range(rng, grid, grid);
        let tiers = 1 + rng.below(3) as u8;
        for x in lo..hi {
            let fair = k / 2 + rng.below(k / 4 + 1);
            for tier in 0..tiers {
                let w = (k / 10) * (1 + u64::from(tier)) + rng.below(3);
                for (s, p) in [
                    (Side::Long, fair.saturating_sub(w)),
                    (Side::Short, fair + w),
                ] {
                    arcs.push(Arc {
                        id: ArcId::Maker {
                            maker,
                            step: x,
                            side: s,
                            tier,
                        },
                        side: s,
                        lo: x,
                        hi: x + 1,
                        price: p / tick * tick,
                        cap: 1 + rng.below(40),
                        rank: rank(seq, maker_sub(x, s, tier)),
                    });
                }
            }
        }
        seq += 1;
    }
    for slot in 0..orders {
        let (lo, hi) = range(rng, grid, 12);
        let s = side(rng);
        arcs.push(Arc {
            id: ArcId::Order(slot as u16),
            side: s,
            lo,
            hi,
            price: price(rng, lo, hi, k * 5 / 8, k / 2, tick, s),
            cap: 1 + rng.below(30),
            rank: rank(seq, 0),
        });
        seq += 1;
    }
    arcs.sort_by_key(|a| a.id);
    arcs
}

fn random_taker(rng: &mut Rng, grid: Pos, k: u64, qty: u64) -> Taker {
    let (lo, hi) = range(rng, grid, grid);
    let s = side(rng);
    let set = u64::from(hi - lo) * k;
    let limit = match rng.below(4) {
        0 => set,
        1 => 0,
        _ => price(rng, lo, hi, k / 2, k / 2, 1, s.opposite()).min(set),
    };
    Taker {
        side: s,
        lo,
        hi,
        limit,
        qty: rng.below(qty + 1),
    }
}

#[test]
fn random_order_books_match_the_reference() {
    let mut rng = Rng(0xA5A5_0123_4567_89AB);
    let mut filled = 0;
    for round in 0..rounds(3000) {
        let grid = 2 + rng.below(30) as Pos;
        let n = 1 + rng.below(40) as usize;
        let k = [1, 10, 1_000][round % 3];
        let arcs = random_orders(&mut rng, grid, n, k, round % 2 == 0);
        let taker = random_taker(&mut rng, grid, k, 40);
        if check(&arcs, Some(&taker), grid).taker_fill > 0 {
            filled += 1;
        }
        if round % 5 == 0 {
            check(&arcs, None, grid);
        }
        check_depth(&arcs, &taker, grid, k);
    }
    assert!(filled > rounds(500), "{filled}");
}

/// Equal prices everywhere: every package costs the same, so routing has
/// the most equally cheap paths to choose from and FIFO decides everything.
#[test]
fn equal_price_books_match_the_reference() {
    let mut rng = Rng(77);
    for round in 0..rounds(2000) {
        let grid = 2 + rng.below(20) as Pos;
        let n = 2 + rng.below(30) as usize;
        let mut arcs = random_orders(&mut rng, grid, n, 10, false);
        for a in &mut arcs {
            a.price = u64::from(a.hi - a.lo) * 5;
        }
        // Now and then two arcs share a rank: the program never does that
        // (and `verify` refuses it), but the solvers must still agree.
        if round % 4 == 0 && n > 2 {
            arcs[1].rank = arcs[0].rank;
        }
        let mut taker = random_taker(&mut rng, grid, 10, 60);
        taker.limit = u64::from(taker.hi - taker.lo) * 5;
        check(&arcs, Some(&taker), grid);
        check_depth(&arcs, &taker, grid, 10);
    }
}

#[test]
fn ladder_books_match_the_reference() {
    let mut rng = Rng(0x0DDB_A11C_AFE5_EED5);
    for round in 0..rounds(600) {
        let grid = 4 + rng.below(60) as Pos;
        let k = [100, 10_000][round % 2];
        let (makers, orders) = (1 + rng.below(3) as u8, rng.below(20) as usize);
        let arcs = ladder_book(&mut rng, grid, makers, orders, k);
        for _ in 0..3 {
            let taker = random_taker(&mut rng, grid, k, 400);
            check(&arcs, Some(&taker), grid);
            check_depth(&arcs, &taker, grid, k);
        }
        check(&arcs, None, grid);
    }
}

/// The worst-case book the bench times, with its three orders and more.
#[test]
fn the_worst_case_book_matches_the_reference() {
    let (grid, k) = (200, 1_000_000);
    let arcs = books::worst_case_book(grid, k);
    for (_, taker) in books::worst_case_takers(grid, k) {
        assert!(check(&arcs, Some(&taker), grid).taker_fill > 0);
    }
    let mut rng = Rng(3);
    for _ in 0..6 {
        let taker = random_taker(&mut rng, grid, k, 8_000);
        check(&arcs, Some(&taker), grid);
    }
    check(&arcs, None, grid);
    for (_, taker) in books::worst_case_takers(grid, k) {
        check_depth(&arcs, &taker, grid, k);
    }
}
