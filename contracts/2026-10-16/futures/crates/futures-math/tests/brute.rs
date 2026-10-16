//! The solver against brute force, and the verifier against every package.
//!
//! For random small books (and random takers) every feasible fill vector is
//! enumerated. The owed package is the one with (1) minimum circulation cost
//! with the taker at its limit, then (2) the most taker lots, then (3) the
//! lexicographically largest resting fills by arrival rank. The solver must
//! return exactly it, the verifier must accept it, and the verifier must
//! reject every other feasible package under the best potentials it admits.
use ameba_futures_math::{solve, verify, Arc, ArcId, Nodes, Pos, Side, Taker, VerifyError};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const K: u64 = 10;

fn random_book(rng: &mut Rng, grid: Pos, n: usize) -> Vec<Arc> {
    let mut ranks: Vec<u128> = (0..n as u128).collect();
    for i in (1..n).rev() {
        ranks.swap(i, rng.below(i as u64 + 1) as usize);
    }
    (0..n)
        .map(|i| {
            let lo = rng.below(u64::from(grid)) as Pos;
            let hi = lo + 1 + rng.below(u64::from(grid - lo)) as Pos;
            let w = u64::from(hi - lo) * K;
            Arc {
                id: ArcId::Order(i as u16),
                side: if rng.below(2) == 0 {
                    Side::Long
                } else {
                    Side::Short
                },
                lo,
                hi,
                price: rng.below(w + 1),
                cap: 1 + rng.below(3),
                rank: ranks[i],
            }
        })
        .collect()
}

fn random_taker(rng: &mut Rng, grid: Pos) -> Taker {
    let lo = rng.below(u64::from(grid)) as Pos;
    let hi = lo + 1 + rng.below(u64::from(grid - lo)) as Pos;
    Taker {
        side: if rng.below(2) == 0 {
            Side::Long
        } else {
            Side::Short
        },
        lo,
        hi,
        limit: rng.below(u64::from(hi - lo) * K + 1),
        qty: 1 + rng.below(3),
    }
}

fn balanced(arcs: &[Arc], m: &[u64], t: Option<(&Arc, u64)>, grid: Pos) -> bool {
    let mut net = vec![0i64; usize::from(grid) + 1];
    for (a, &f) in arcs.iter().zip(m).chain(t.iter().map(|(a, f)| (*a, f))) {
        net[usize::from(a.tail())] -= f as i64;
        net[usize::from(a.head())] += f as i64;
    }
    net.iter().all(|&x| x == 0)
}

/// Potentials for an arbitrary flow: shortest distances over its residual
/// graph, if it has no negative cycle.
fn potentials_for(
    arcs: &[Arc],
    m: &[u64],
    t: Option<(&Arc, u64)>,
    nodes: &Nodes,
) -> Option<Vec<i64>> {
    let mut edges = Vec::new();
    for (a, &f) in arcs.iter().zip(m).chain(t.iter().map(|(a, f)| (*a, f))) {
        let (u, v) = (nodes.of(a.tail()), nodes.of(a.head()));
        if f < a.cap {
            edges.push((u, v, a.cost()));
        }
        if f > 0 {
            edges.push((v, u, -a.cost()));
        }
    }
    let mut d = vec![0i64; nodes.len()];
    for round in 0..=nodes.len() {
        let mut changed = false;
        for &(u, v, c) in &edges {
            if d[u] + c < d[v] {
                d[v] = d[u] + c;
                changed = true;
            }
        }
        if !changed {
            return Some(d);
        }
        if round == nodes.len() {
            return None;
        }
    }
    None
}

fn enumerate(caps: &[u64], mut f: impl FnMut(&[u64])) {
    let mut m = vec![0u64; caps.len()];
    loop {
        f(&m);
        let mut i = 0;
        loop {
            if i == caps.len() {
                return;
            }
            if m[i] < caps[i] {
                m[i] += 1;
                break;
            }
            m[i] = 0;
            i += 1;
        }
    }
}

fn check_instance(arcs: &[Arc], taker: Option<&Taker>, grid: Pos) -> usize {
    let t_arc = taker.map(Taker::arc);
    let mut caps: Vec<u64> = arcs.iter().map(|a| a.cap).collect();
    if let Some(t) = &t_arc {
        caps.push(t.cap);
    }
    let mut order: Vec<usize> = (0..arcs.len()).collect();
    order.sort_by_key(|&i| arcs[i].rank);
    // Key: (cost, -taker, -fills by rank...) minimal.
    let key = |m: &[u64]| -> (i64, i64, Vec<i64>) {
        let n = arcs.len();
        let tf = if t_arc.is_some() { m[n] } else { 0 };
        let mut cost: i64 = arcs.iter().zip(m).map(|(a, &f)| a.cost() * f as i64).sum();
        if let Some(t) = &t_arc {
            cost += t.cost() * tf as i64;
        }
        (
            cost,
            -(tf as i64),
            order.iter().map(|&i| -(m[i] as i64)).collect(),
        )
    };
    let split = |m: &[u64]| -> (Vec<u64>, u64) {
        let n = arcs.len();
        (m[..n].to_vec(), if t_arc.is_some() { m[n] } else { 0 })
    };
    type Key = (i64, i64, Vec<i64>);
    let mut best: Option<(Key, Vec<u64>)> = None;
    let mut feasible = Vec::new();
    enumerate(&caps, |m| {
        let (f, tf) = split(m);
        if !balanced(arcs, &f, t_arc.as_ref().map(|t| (t, tf)), grid) {
            return;
        }
        feasible.push(m.to_vec());
        let k = key(m);
        if best.as_ref().is_none_or(|(bk, _)| k < *bk) {
            best = Some((k, m.to_vec()));
        }
    });
    let (_, want) = best.expect("the empty package is feasible");
    let (want_f, want_t) = split(&want);

    let sol = solve(arcs, taker, grid);
    assert_eq!(
        (sol.fills.clone(), sol.taker_fill),
        (want_f.clone(), want_t),
        "solver vs brute force\n{arcs:#?}\n{taker:?}"
    );
    verify(
        arcs,
        taker,
        &sol.fills,
        sol.taker_fill,
        &sol.potentials,
        grid,
        u64::MAX,
    )
    .unwrap_or_else(|e| panic!("verifier rejects the solution: {e:?}\n{arcs:#?}\n{taker:?}"));

    let nodes = Nodes::new(arcs, taker, grid);
    for m in &feasible {
        if *m == want {
            continue;
        }
        let (f, tf) = split(m);
        // Under the solver's potentials and under the flow's own best ones.
        let mut tries = vec![sol.potentials.clone()];
        if let Some(p) = potentials_for(arcs, &f, t_arc.as_ref().map(|t| (t, tf)), &nodes) {
            tries.push(p);
        }
        for p in tries {
            let r = verify(arcs, taker, &f, tf, &p, grid, u64::MAX);
            assert!(
                r.is_err(),
                "verifier accepts a package that is not owed: {f:?} {tf}\n{arcs:#?}\n{taker:?}"
            );
        }
    }
    feasible.len()
}

#[test]
fn solver_matches_brute_force_and_verifier_is_exact() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut packages = 0;
    for round in 0..8000 {
        let grid = 2 + (round % 5) as Pos;
        let n = 1 + rng.below(5) as usize;
        let arcs = random_book(&mut rng, grid, n);
        let taker = random_taker(&mut rng, grid);
        packages += check_instance(&arcs, Some(&taker), grid);
        if round % 7 == 0 {
            packages += check_instance(&arcs, None, grid);
        }
    }
    assert!(packages > 25_000, "{packages}");
}

/// Equal prices everywhere: every package is a FIFO tie, the hardest case
/// for rule 3.
#[test]
fn fifo_ties_match_brute_force() {
    let mut rng = Rng(42);
    for round in 0..2000 {
        let grid = 2 + (round % 4) as Pos;
        let n = 2 + rng.below(4) as usize;
        let mut arcs = random_book(&mut rng, grid, n);
        // Price every arc at a fair 5 per step: all compositions cost the same.
        for a in &mut arcs {
            a.price = u64::from(a.hi - a.lo) * 5;
        }
        let mut taker = random_taker(&mut rng, grid);
        taker.limit = u64::from(taker.hi - taker.lo) * 5;
        check_instance(&arcs, Some(&taker), grid);
    }
}

#[test]
fn verifier_rejects_a_hidden_older_order() {
    // Two equal-price shorts on [0,2); the older one (rank 0) must fill first.
    let arcs = vec![
        Arc {
            id: ArcId::Order(0),
            side: Side::Short,
            lo: 0,
            hi: 2,
            price: 10,
            cap: 1,
            rank: 5,
        },
        Arc {
            id: ArcId::Order(1),
            side: Side::Short,
            lo: 0,
            hi: 2,
            price: 10,
            cap: 1,
            rank: 1,
        },
    ];
    let t = Taker {
        side: Side::Long,
        lo: 0,
        hi: 2,
        limit: 10,
        qty: 1,
    };
    let sol = solve(&arcs, Some(&t), 4);
    assert_eq!(sol.fills, vec![0, 1]);
    let r = verify(&arcs, Some(&t), &[1, 0], 1, &sol.potentials, 4, u64::MAX);
    assert_eq!(r, Err(VerifyError::Fifo { arc: 1 }));
}

/// Depth levels are the solver's cost curve: buying the first `q` lots at
/// the levels costs exactly what the solver charges a taker for `q`.
#[test]
fn depth_is_the_cost_curve() {
    let mut rng = Rng(7);
    for round in 0..4000 {
        let grid = 2 + (round % 5) as Pos;
        let n = 1 + rng.below(5) as usize;
        let arcs = random_book(&mut rng, grid, n);
        // The program keeps the resting book uncrossed (every entry is
        // post-only or match-first); a crossed book's surplus is no level.
        if solve(&arcs, None, grid).fills.iter().any(|&f| f > 0) {
            continue;
        }
        let t = random_taker(&mut rng, grid);
        let levels = ameba_futures_math::depth(&arcs, t.side, t.lo, t.hi, 6, grid, K);
        let mut cum = 0u64;
        let mut cost = 0i128;
        for w in levels.windows(2) {
            match t.side {
                Side::Long => assert!(w[0].0 <= w[1].0, "{levels:?}"),
                Side::Short => assert!(w[0].0 >= w[1].0, "{levels:?}"),
            }
        }
        for &(p, l) in &levels {
            for _ in 0..l {
                cum += 1;
                cost += i128::from(p);
                let limit = match t.side {
                    Side::Long => u64::from(t.hi - t.lo) * K,
                    Side::Short => 0,
                };
                let s = solve(
                    &arcs,
                    Some(&Taker {
                        qty: cum,
                        limit,
                        ..t
                    }),
                    grid,
                );
                assert_eq!(s.taker_fill, cum);
                assert_eq!(s.premium, cost, "{levels:?}\n{arcs:#?}\n{t:?} {s:?}");
            }
        }
    }
}
