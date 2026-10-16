//! The exact package solver (off-chain; anyone can run it).
//!
//! It searches the complete admitted book, not a top-N subset:
//!
//! 1. cancel any negative cycle among resting arcs (a crossed book; the
//!    program's admission keeps it from arising);
//! 2. successive shortest paths for the taker: route lots from the taker's
//!    head back to its tail along the cheapest residual path while the path
//!    costs no more than the taker's limit (equal cost included, rule 2);
//! 3. potentials: shortest distances from a virtual source over the final
//!    residual graph, a valid dual for every arc;
//! 4. FIFO: for each arc from oldest to newest, push lots around
//!    zero-reduced-cost cycles that touch only newer arcs (and never the
//!    taker) until none is left. This is a max-flow per arc within the optimal
//!    face, so the result is the lexicographic maximum by rank.
//!
//! The output passes [`crate::verify`] by construction; the tests check it
//! against brute force over every feasible package.
use crate::{taker_premium, Arc, Nodes, Pos, Side, Taker};

const INF: i64 = i64::MAX / 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Solution {
    /// Lots filled per arc, parallel to the input.
    pub fills: Vec<u64>,
    pub taker_fill: u64,
    /// One per canonical node.
    pub potentials: Vec<i64>,
    /// Premium the taker pays (long) or receives (short), in atoms. Can be
    /// negative when resting prices are inconsistent in the taker's favour.
    pub premium: i128,
}

#[derive(Clone, Copy)]
struct Edge {
    from: usize,
    to: usize,
    cost: i64,
    arc: usize,
    forward: bool,
}

struct Net<'a> {
    arcs: Vec<Arc>,
    flow: Vec<u64>,
    taker: Option<usize>,
    nodes: &'a Nodes,
}

impl Net<'_> {
    fn residual(&self, skip_taker: bool) -> Vec<Edge> {
        let mut out = Vec::with_capacity(self.arcs.len() * 2);
        for (i, a) in self.arcs.iter().enumerate() {
            if skip_taker && Some(i) == self.taker {
                continue;
            }
            let (t, h) = (self.nodes.of(a.tail()), self.nodes.of(a.head()));
            if self.flow[i] < a.cap {
                out.push(Edge {
                    from: t,
                    to: h,
                    cost: a.cost(),
                    arc: i,
                    forward: true,
                });
            }
            if self.flow[i] > 0 {
                out.push(Edge {
                    from: h,
                    to: t,
                    cost: -a.cost(),
                    arc: i,
                    forward: false,
                });
            }
        }
        out
    }

    fn room(&self, e: &Edge) -> u64 {
        if e.forward {
            self.arcs[e.arc].cap - self.flow[e.arc]
        } else {
            self.flow[e.arc]
        }
    }

    fn push(&mut self, e: &Edge, d: u64) {
        if e.forward {
            self.flow[e.arc] += d;
        } else {
            self.flow[e.arc] -= d;
        }
    }

    /// Bellman-Ford from `sources`: distances and predecessor edges, or on a
    /// negative cycle a node it reaches and the predecessors.
    #[allow(clippy::type_complexity)]
    fn bellman_ford(
        &self,
        edges: &[Edge],
        sources: &[usize],
    ) -> Result<(Vec<i64>, Vec<Option<usize>>), (usize, Vec<Option<usize>>)> {
        let n = self.nodes.len();
        let mut dist = vec![INF; n];
        let mut pred = vec![None; n];
        for &s in sources {
            dist[s] = 0;
        }
        for round in 0..=n {
            let mut changed = None;
            for (k, e) in edges.iter().enumerate() {
                if dist[e.from] < INF && dist[e.from] + e.cost < dist[e.to] {
                    dist[e.to] = dist[e.from] + e.cost;
                    pred[e.to] = Some(k);
                    changed = Some(e.to);
                }
            }
            match changed {
                None => return Ok((dist, pred)),
                Some(v) if round == n => return Err((v, pred)),
                _ => {}
            }
        }
        unreachable!()
    }

    fn cancel_negative_cycles(&mut self) {
        loop {
            let edges = self.residual(true);
            let all: Vec<usize> = (0..self.nodes.len()).collect();
            let (v, pred) = match self.bellman_ford(&edges, &all) {
                Ok(_) => return,
                Err(found) => found,
            };
            // Walk back n steps to land on the cycle, then collect it.
            let mut x = v;
            for _ in 0..self.nodes.len() {
                x = edges[pred[x].expect("pred")].from;
            }
            let mut cycle = Vec::new();
            let mut y = x;
            loop {
                let k = pred[y].expect("pred");
                cycle.push(k);
                y = edges[k].from;
                if y == x {
                    break;
                }
            }
            let d = cycle
                .iter()
                .map(|&k| self.room(&edges[k]))
                .min()
                .unwrap_or(0);
            for &k in &cycle {
                self.push(&edges[k], d);
            }
        }
    }
}

impl Net<'_> {
    /// Successive shortest paths for arc `ti`: route lots from its head back
    /// to its tail along the cheapest residual path while the path costs no
    /// more than the arc's own (negated) cost. `level(cost, lots)` sees each
    /// increment's path cost per lot.
    fn route(&mut self, ti: usize, mut level: impl FnMut(i64, u64)) {
        let ta = self.arcs[ti];
        let (s, z) = (self.nodes.of(ta.head()), self.nodes.of(ta.tail()));
        while self.flow[ti] < ta.cap {
            let edges = self.residual(true);
            let (dist, pred) = self.bellman_ford(&edges, &[s]).expect("no negative cycle");
            if dist[z] >= INF || dist[z] + ta.cost() > 0 {
                break;
            }
            let mut path = Vec::new();
            let mut y = z;
            while y != s {
                let k = pred[y].expect("pred");
                path.push(k);
                y = edges[k].from;
            }
            let d = path
                .iter()
                .map(|&k| self.room(&edges[k]))
                .min()
                .unwrap_or(u64::MAX)
                .min(ta.cap - self.flow[ti]);
            for &k in &path {
                self.push(&edges[k], d);
            }
            self.flow[ti] += d;
            level(dist[z], d);
        }
    }
}

/// Executable depth for one claim shape: buying (`Long`) or selling
/// (`Short`) `[lo, hi)`, the successive premiums per lot and the lots
/// available at each, up to `max_qty` lots. This is the cost curve of the
/// exact solver (each level is one more cheapest package increment), so it
/// is what a sequence of market orders for that shape would pay, not a sum
/// over unrelated ranges. A premium can be negative when resting prices are
/// inconsistent in the taker's favour.
pub fn depth(
    arcs: &[Arc],
    side: Side,
    lo: Pos,
    hi: Pos,
    max_qty: u64,
    grid: Pos,
    k: u64,
) -> Vec<(i64, u64)> {
    let limit = match side {
        Side::Long => u64::from(hi - lo) * k,
        Side::Short => 0,
    };
    let taker = Taker {
        side,
        lo,
        hi,
        limit,
        qty: max_qty,
    };
    let nodes = Nodes::new(arcs, Some(&taker), grid);
    let mut all = arcs.to_vec();
    all.push(taker.arc());
    let ti = all.len() - 1;
    let mut net = Net {
        flow: vec![0; all.len()],
        arcs: all,
        taker: Some(ti),
        nodes: &nodes,
    };
    net.cancel_negative_cycles();
    let mut levels: Vec<(i64, u64)> = Vec::new();
    net.route(ti, |cost, lots| {
        // A long pays the path cost; a short receives its negation.
        let premium = match side {
            Side::Long => cost,
            Side::Short => -cost,
        };
        match levels.last_mut() {
            Some(l) if l.0 == premium => l.1 += lots,
            _ => levels.push((premium, lots)),
        }
    });
    levels
}

/// The package owed to `taker` (or, with no taker, the book's own
/// potentials: any fills then mean the book is crossed).
pub fn solve(arcs: &[Arc], taker: Option<&Taker>, grid: Pos) -> Solution {
    let nodes = Nodes::new(arcs, taker, grid);
    let mut all = arcs.to_vec();
    let t = taker.map(|t| {
        all.push(t.arc());
        all.len() - 1
    });
    let mut net = Net {
        flow: vec![0; all.len()],
        arcs: all,
        taker: t,
        nodes: &nodes,
    };

    net.cancel_negative_cycles();

    // Successive shortest paths for the taker.
    if let Some(ti) = t {
        net.route(ti, |_, _| {});
    }

    // Potentials: shortest distances from a virtual source.
    let edges = net.residual(false);
    let every: Vec<usize> = (0..nodes.len()).collect();
    let (potentials, _) = net.bellman_ford(&edges, &every).expect("optimal flow");

    // FIFO: oldest first, saturate zero-cost exchanges through newer arcs.
    let mut order: Vec<usize> = (0..arcs.len()).collect();
    order.sort_by_key(|&i| arcs[i].rank);
    let reduced =
        |a: &Arc| a.cost() + potentials[nodes.of(a.tail())] - potentials[nodes.of(a.head())];
    for &i in &order {
        let a = net.arcs[i];
        if reduced(&a) != 0 {
            continue;
        }
        let (s, z) = (nodes.of(a.head()), nodes.of(a.tail()));
        while net.flow[i] < a.cap {
            let edges: Vec<Edge> = net
                .residual(true)
                .into_iter()
                .filter(|e| {
                    let b = &net.arcs[e.arc];
                    b.rank > a.rank && reduced(b) == 0
                })
                .collect();
            let Some(path) = bfs(&edges, s, z, nodes.len()) else {
                break;
            };
            let d = path
                .iter()
                .map(|&k| net.room(&edges[k]))
                .min()
                .unwrap_or(u64::MAX)
                .min(a.cap - net.flow[i]);
            for &k in &path {
                net.push(&edges[k], d);
            }
            net.flow[i] += d;
        }
    }

    let taker_fill = t.map_or(0, |ti| net.flow[ti]);
    let fills = net.flow[..arcs.len()].to_vec();
    let premium = taker.map_or(0, |tk| taker_premium(tk.side, arcs, &fills));
    Solution {
        fills,
        taker_fill,
        potentials,
        premium,
    }
}

fn bfs(edges: &[Edge], s: usize, z: usize, n: usize) -> Option<Vec<usize>> {
    let mut adj = vec![Vec::new(); n];
    for (k, e) in edges.iter().enumerate() {
        adj[e.from].push(k);
    }
    let mut pred: Vec<Option<usize>> = vec![None; n];
    let mut seen = vec![false; n];
    seen[s] = true;
    let mut queue = std::collections::VecDeque::from([s]);
    while let Some(x) = queue.pop_front() {
        if x == z {
            let mut path = Vec::new();
            let mut y = z;
            while y != s {
                let k = pred[y].expect("pred");
                path.push(k);
                y = edges[k].from;
            }
            return Some(path);
        }
        for &k in &adj[x] {
            let to = edges[k].to;
            if !seen[to] {
                seen[to] = true;
                pred[to] = Some(k);
                queue.push_back(to);
            }
        }
    }
    None
}
