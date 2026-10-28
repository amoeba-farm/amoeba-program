//! Times `solve` against the pre-speed-up reference solver (and `verify`) on
//! the worst-case book `docs/futures.md` measures: grid 200, 1,024 ladder
//! arcs from two makers over the whole grid and all 64 order slots. Asserts
//! the two solvers return the same `Solution` first.
//!
//!   cargo bench -p ameba-futures-math --bench solve
//!   BENCH_RUNS=400 cargo bench -p ameba-futures-math --bench solve
#[path = "../tests/support/mod.rs"]
mod support;

use ameba_futures_math::{solve::solve_with_marks, verify, Arc, Pos, Taker};
use std::hint::black_box;
use std::time::Instant;
use support::{books, reference, Solver};

const PHASES: [&str; 4] = ["cancel", "route", "potentials", "fifo"];

struct Timing {
    total: (f64, f64),
    phases: Vec<f64>,
}

fn median(xs: &mut [f64]) -> f64 {
    xs.sort_by(f64::total_cmp);
    xs[xs.len() / 2]
}

fn p95(xs: &mut [f64]) -> f64 {
    xs.sort_by(f64::total_cmp);
    xs[xs.len() * 95 / 100]
}

/// Median and p95 of the whole call, and the median of each phase, in µs.
fn time(runs: usize, arcs: &[Arc], taker: &Taker, grid: Pos, f: Solver) -> Timing {
    let mut total = Vec::with_capacity(runs);
    let mut phases = vec![Vec::with_capacity(runs); PHASES.len()];
    for _ in 0..runs {
        let mut marks: Vec<(&'static str, Instant)> = Vec::with_capacity(PHASES.len());
        let start = Instant::now();
        black_box(f(black_box(arcs), Some(taker), grid, &mut |p, _| {
            marks.push((p, Instant::now()))
        }));
        let end = Instant::now();
        total.push((end - start).as_secs_f64() * 1e6);
        let mut last = start;
        for (p, at) in marks {
            let i = PHASES.iter().position(|&q| q == p).expect("known phase");
            phases[i].push((at - last).as_secs_f64() * 1e6);
            last = at;
        }
    }
    Timing {
        total: (median(&mut total.clone()), p95(&mut total)),
        phases: phases.iter_mut().map(|xs| median(xs)).collect(),
    }
}

fn main() {
    let runs: usize = std::env::var("BENCH_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let (grid, k) = (200, 1_000_000);
    let arcs = books::worst_case_book(grid, k);
    let orders = arcs
        .iter()
        .filter(|a| matches!(a.id, ameba_futures_math::ArcId::Order(_)));
    println!(
        "book: {} arcs ({} ladder, {} orders), grid {grid}; {runs} runs per solver, µs",
        arcs.len(),
        arcs.len() - orders.clone().count(),
        orders.count()
    );
    for (name, taker) in books::worst_case_takers(grid, k) {
        let old = reference::solve(&arcs, Some(&taker), grid);
        let new = ameba_futures_math::solve(&arcs, Some(&taker), grid);
        assert_eq!(new, old, "{name}: the solvers disagree");
        verify(
            &arcs,
            Some(&taker),
            &new.fills,
            new.taker_fill,
            &new.potentials,
            grid,
            u64::MAX,
        )
        .expect("the package verifies");
        let filled = new.fills.iter().filter(|&&f| f > 0).count();
        println!(
            "\n{name}: taker fills {} lots through {filled} arcs, premium {}",
            new.taker_fill, new.premium
        );
        let before = time(runs, &arcs, &taker, grid, reference::solve_with_marks);
        let after = time(runs, &arcs, &taker, grid, solve_with_marks);
        let mut v: Vec<f64> = (0..runs)
            .map(|_| {
                let t = Instant::now();
                black_box(verify(
                    &arcs,
                    Some(&taker),
                    &new.fills,
                    new.taker_fill,
                    &new.potentials,
                    grid,
                    u64::MAX,
                ))
                .ok();
                t.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        println!(
            "  {:<11} {:>10} {:>10} {:>8}",
            "", "before", "after", "speed-up"
        );
        let row = |label: &str, b: f64, a: f64| {
            println!("  {label:<11} {b:>10.1} {a:>10.1} {:>7.1}x", b / a);
        };
        row("solve p50", before.total.0, after.total.0);
        row("solve p95", before.total.1, after.total.1);
        for (i, p) in PHASES.iter().enumerate() {
            row(p, before.phases[i], after.phases[i]);
        }
        println!("  {:<11} {:>10.1}", "verify p50", median(&mut v));
    }
}
