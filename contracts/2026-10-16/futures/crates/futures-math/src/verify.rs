//! On-chain package verification.
//!
//! The program rebuilds the complete arc list from its own book (every live
//! order and ladder step, minus the taker's own liquidity) and checks a
//! proposed package against it. Nothing in the proposal can hide liquidity:
//! the potentials must price *every* arc consistently, so an omitted cheaper
//! or older source makes the check fail.
//!
//! Checks, all exact integer arithmetic:
//!
//! * **Shape:** fills within each arc's lots (whole-shape fills only; a ladder
//!   step is its own shape).
//! * **Balance:** at every node, filled lots in equal filled lots out, so long
//!   and short coverage match at every grid step.
//! * **Price (rule 1):** complementary slackness with the potentials `pi`.
//!   With reduced cost `r = cost + pi[tail] - pi[head]`, an arc with lots left
//!   has `r >= 0` and a filled arc `r <= 0`. By LP duality no package with
//!   the taker at its limit costs less.
//! * **Quantity (rule 2):** if the taker is short of its quantity, no
//!   zero-reduced-cost residual path leads from its head back to its tail
//!   (more lots would cost more than its limit).
//! * **FIFO (rule 3):** for every arc with lots left and `r == 0`, no
//!   zero-reduced-cost residual path from its head back to its tail uses only
//!   arcs newer than it. Every optimal package differs from this one by
//!   zero-cost cycles, so such a path is exactly a cost-neutral exchange that
//!   would fill an older arc more at the expense of newer ones.
//!
//! Rules 2 and 3 do not depend on which valid potentials are supplied: every
//! optimal package satisfies complementary slackness with every optimal dual.
use crate::{Arc, Nodes, Pos, Taker};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyError {
    /// Wrong number of potentials for the canonical node set.
    Potentials,
    /// An arc filled beyond its lots, or an invalid shape.
    Capacity { arc: usize },
    /// Long and short coverage differ at a node.
    Unbalanced { node: usize },
    /// Complementary slackness fails: a cheaper package exists.
    Price { arc: usize },
    /// The taker could fill more lots at its limit.
    Quantity,
    /// An older arc could fill more at the same price.
    Fifo { arc: usize },
    /// The search exceeded the work limit.
    Work,
}

/// What a verified package amounts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verified {
    /// Edges scanned by the reachability checks.
    pub work: u64,
}

/// `arcs[i]` filled `fills[i]` lots, the taker `taker_fill` lots.
/// `potentials` has one entry per canonical node (sorted grid positions).
pub fn verify(
    arcs: &[Arc],
    taker: Option<&Taker>,
    fills: &[u64],
    taker_fill: u64,
    potentials: &[i64],
    grid: Pos,
    work_limit: u64,
) -> Result<Verified, VerifyError> {
    if fills.len() != arcs.len() {
        return Err(VerifyError::Potentials);
    }
    for (i, a) in arcs.iter().enumerate() {
        if a.lo >= a.hi || a.hi > grid || fills[i] > a.cap {
            return Err(VerifyError::Capacity { arc: i });
        }
    }
    let t_arc = taker.map(Taker::arc);
    if let Some(t) = &t_arc {
        if t.lo >= t.hi || t.hi > grid || taker_fill > t.cap {
            return Err(VerifyError::Capacity { arc: arcs.len() });
        }
    } else if taker_fill != 0 {
        return Err(VerifyError::Capacity { arc: arcs.len() });
    }
    let nodes = Nodes::new(arcs, taker, grid);
    if potentials.len() != nodes.len() {
        return Err(VerifyError::Potentials);
    }

    // Every arc's (tail, head) nodes, the taker's last.
    let mut ends: Vec<(u16, u16)> = Vec::with_capacity(arcs.len() + 1);
    for a in arcs.iter().chain(t_arc.iter()) {
        ends.push((nodes.of(a.tail()) as u16, nodes.of(a.head()) as u16));
    }
    let all_fills = |i: usize| if i < arcs.len() { fills[i] } else { taker_fill };
    // `ends` has the taker's entry only when there is a taker.
    let arc_at = |i: usize| match arcs.get(i) {
        Some(a) => a,
        None => t_arc.as_ref().expect("the taker's arc"),
    };

    // Balance.
    let mut net = vec![0i128; nodes.len()];
    for (i, &(t, h)) in ends.iter().enumerate() {
        let m = i128::from(all_fills(i));
        net[usize::from(t)] -= m;
        net[usize::from(h)] += m;
    }
    if let Some(node) = net.iter().position(|&n| n != 0) {
        return Err(VerifyError::Unbalanced { node });
    }

    // Price: complementary slackness.
    let reduced = |i: usize| -> i128 {
        let (t, h) = ends[i];
        i128::from(arc_at(i).cost()) + i128::from(potentials[usize::from(t)])
            - i128::from(potentials[usize::from(h)])
    };
    let mut zero = Vec::new();
    for i in 0..ends.len() {
        let (a, m) = (arc_at(i), all_fills(i));
        let r = reduced(i);
        if (m < a.cap && r < 0) || (m > 0 && r > 0) {
            return Err(VerifyError::Price { arc: i });
        }
        if r == 0 && i < arcs.len() {
            zero.push(i);
        }
    }

    // Zero-reduced-cost residual graph (taker excluded), as adjacency lists.
    let g = ZeroGraph::new(arcs, fills, &zero, &ends, nodes.len());
    let mut search = Search::new(nodes.len(), work_limit);

    // Quantity.
    if let Some(t) = &t_arc {
        if taker_fill < t.cap && reduced(arcs.len()) == 0 {
            let (from, to) = (nodes.of(t.head()), nodes.of(t.tail()));
            if search.reaches(&g, from, to, None)? {
                return Err(VerifyError::Quantity);
            }
        }
    }

    // FIFO. An exchange through arc `i` is a zero-cost cycle containing its
    // forward edge, so both ends must share a strongly connected component of
    // the zero graph, and the way back from its head to its tail must cross
    // every cut between them through an edge newer than `i`; only then is
    // the rank-restricted search needed. (Along a maker ladder that a package
    // fills partly, each step's way back is only its own reverse edge, so the
    // cuts settle every step without a search.)
    let scc = g.components();
    let cuts = Cuts::new(&g);
    for &i in &zero {
        let a = &arcs[i];
        if fills[i] < a.cap {
            let (from, to) = (usize::from(ends[i].1), usize::from(ends[i].0));
            if scc[from] != scc[to] || !cuts.passable(from, to, a.rank) {
                continue;
            }
            if search.reaches(&g, from, to, Some(a.rank))? {
                return Err(VerifyError::Fifo { arc: i });
            }
        }
    }
    Ok(Verified { work: search.work })
}

/// Residual edges of the zero-reduced-cost arcs: forward while lots remain,
/// backward while filled. Compressed adjacency by source node.
pub(crate) struct ZeroGraph {
    start: Vec<u32>,
    to: Vec<u16>,
    rank: Vec<u128>,
}

impl ZeroGraph {
    pub(crate) fn new(
        arcs: &[Arc],
        fills: &[u64],
        zero: &[usize],
        ends: &[(u16, u16)],
        nodes: usize,
    ) -> Self {
        let mut edges: Vec<(u16, u16, u128)> = Vec::with_capacity(zero.len() * 2);
        for &i in zero {
            let a = &arcs[i];
            let (t, h) = ends[i];
            if fills[i] < a.cap {
                edges.push((t, h, a.rank));
            }
            if fills[i] > 0 {
                edges.push((h, t, a.rank));
            }
        }
        let mut start = vec![0u32; nodes + 1];
        for e in &edges {
            start[usize::from(e.0) + 1] += 1;
        }
        for i in 0..nodes {
            start[i + 1] += start[i];
        }
        let mut fill = start.clone();
        let mut to = vec![0u16; edges.len()];
        let mut rank = vec![0u128; edges.len()];
        for e in &edges {
            let at = fill[usize::from(e.0)] as usize;
            fill[usize::from(e.0)] += 1;
            to[at] = e.1;
            rank[at] = e.2;
        }
        ZeroGraph { start, to, rank }
    }
}

impl ZeroGraph {
    /// Strongly connected component of every node (iterative Tarjan).
    pub(crate) fn components(&self) -> Vec<u16> {
        const NONE: u16 = u16::MAX;
        let n = self.start.len() - 1;
        let mut index = vec![NONE; n];
        let mut low = vec![0u16; n];
        let mut comp = vec![NONE; n];
        let mut on = vec![false; n];
        let mut stack: Vec<u16> = Vec::with_capacity(n);
        let mut calls: Vec<(u16, u32)> = Vec::with_capacity(n);
        let (mut next, mut comps) = (0u16, 0u16);
        for root in 0..n {
            if index[root] != NONE {
                continue;
            }
            calls.push((root as u16, self.start[root]));
            index[root] = next;
            low[root] = next;
            next += 1;
            stack.push(root as u16);
            on[root] = true;
            while let Some(&mut (v, ref mut at)) = calls.last_mut() {
                let v = usize::from(v);
                if *at < self.start[v + 1] {
                    let w = usize::from(self.to[*at as usize]);
                    *at += 1;
                    if index[w] == NONE {
                        index[w] = next;
                        low[w] = next;
                        next += 1;
                        stack.push(w as u16);
                        on[w] = true;
                        calls.push((w as u16, self.start[w]));
                    } else if on[w] {
                        low[v] = low[v].min(index[w]);
                    }
                    continue;
                }
                calls.pop();
                if let Some(&(parent, _)) = calls.last() {
                    let p = usize::from(parent);
                    low[p] = low[p].min(low[v]);
                }
                if low[v] == index[v] {
                    while let Some(w) = stack.pop() {
                        on[usize::from(w)] = false;
                        comp[usize::from(w)] = comps;
                        if usize::from(w) == v {
                            break;
                        }
                    }
                    comps += 1;
                }
            }
        }
        comp
    }
}

/// For each cut between consecutive nodes `c` and `c + 1`, the newest rank
/// of a zero-graph edge crossing it downward and upward (0: none; every arc
/// rank is positive).
pub(crate) struct Cuts {
    down: Vec<u128>,
    up: Vec<u128>,
}

impl Cuts {
    pub(crate) fn new(g: &ZeroGraph) -> Self {
        let n = g.start.len() - 1;
        let cuts = n.saturating_sub(1);
        let (mut down, mut up) = (vec![0u128; cuts], vec![0u128; cuts]);
        for u in 0..n {
            for k in g.start[u] as usize..g.start[u + 1] as usize {
                let (v, r) = (usize::from(g.to[k]), g.rank[k]);
                let (side, range) = if v < u {
                    (&mut down, v..u)
                } else {
                    (&mut up, u..v)
                };
                for c in &mut side[range] {
                    *c = (*c).max(r);
                }
            }
        }
        Cuts { down, up }
    }

    /// Whether every cut between `from` and `to` has an edge newer than
    /// `rank` crossing it toward `to`: a path from `from` to `to` through
    /// newer edges needs one.
    pub(crate) fn passable(&self, from: usize, to: usize, rank: u128) -> bool {
        if to < from {
            self.down[to..from].iter().all(|&r| r > rank)
        } else {
            self.up[from..to].iter().all(|&r| r > rank)
        }
    }
}

pub(crate) struct Search {
    seen: Vec<u32>,
    stamp: u32,
    queue: Vec<u16>,
    pub(crate) work: u64,
    limit: u64,
}

impl Search {
    pub(crate) fn new(nodes: usize, limit: u64) -> Self {
        Search {
            seen: vec![0; nodes],
            stamp: 0,
            queue: Vec::with_capacity(nodes),
            work: 0,
            limit,
        }
    }

    /// Breadth-first reachability over edges newer than `newer_than`.
    pub(crate) fn reaches(
        &mut self,
        g: &ZeroGraph,
        from: usize,
        to: usize,
        newer_than: Option<u128>,
    ) -> Result<bool, VerifyError> {
        self.stamp += 1;
        self.queue.clear();
        self.queue.push(from as u16);
        self.seen[from] = self.stamp;
        let mut head = 0;
        while head < self.queue.len() {
            let n = usize::from(self.queue[head]);
            head += 1;
            if n == to {
                return Ok(true);
            }
            let (s, e) = (g.start[n] as usize, g.start[n + 1] as usize);
            self.work += (e - s) as u64 + 1;
            if self.work > self.limit {
                return Err(VerifyError::Work);
            }
            for k in s..e {
                if newer_than.is_some_and(|r| g.rank[k] <= r) {
                    continue;
                }
                let m = usize::from(g.to[k]);
                if self.seen[m] != self.stamp {
                    self.seen[m] = self.stamp;
                    self.queue.push(m as u16);
                }
            }
        }
        Ok(false)
    }
}
