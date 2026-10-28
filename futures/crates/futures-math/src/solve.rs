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
//!
//! **Speed** (2026-10-23). The graph is built once: arc `i` owns residual
//! edge `2i` (tail to head, while it has lots left) and `2i + 1` (head to
//! tail, while it carries lots), and each node lists its edges by id. Steps
//! 1 and 3 take the distances from a queue of improved nodes (SPFA) and fall
//! back to Bellman-Ford only for a negative cycle. Step 2 runs Dijkstra on
//! reduced costs (potentials from step 1, updated after every increment)
//! instead of a Bellman-Ford pass over a rebuilt edge list per increment,
//! and among equally cheap paths it takes the one that pass took
//! ([`Net::shortest`]). Step 4 searches lists filtered once to the
//! zero-reduced-cost arcs, and skips the searches that can't succeed (an
//! arc whose ends lie in different strongly connected components). Every
//! increment, fill, potential and depth level is the same as before:
//! `tests/equivalence.rs` holds this solver to the previous one, kept as a
//! test-only reference (`tests/support/reference.rs`), phase by phase.
use crate::{taker_premium, Arc, Nodes, Pos, Side, Taker};
use std::collections::VecDeque;

const INF: i64 = i64::MAX / 4;
/// No place yet: a node Tarjan's search hasn't numbered, or one not in the
/// heap.
const NEW: u32 = u32::MAX;

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

/// A live residual edge, as a Bellman-Ford pass scans it.
#[derive(Clone, Copy)]
struct Edge {
    from: usize,
    to: usize,
    cost: i64,
    id: u32,
}

/// Per-node edge lists: the edges leaving node `x`, by id, are
/// `out[first[x]..first[x + 1]]`; `to` and `cost` hold where each one leads
/// and its cost per lot.
struct Adjacency {
    first: Vec<u32>,
    out: Vec<u32>,
    to: Vec<u32>,
    cost: Vec<i64>,
}

impl Adjacency {
    /// The edges leaving `x`: id, the node it leads to, cost per lot.
    fn of(&self, x: usize) -> impl Iterator<Item = (u32, usize, i64)> + '_ {
        let span = self.first[x] as usize..self.first[x + 1] as usize;
        let (out, to, cost) = (
            &self.out[span.clone()],
            &self.to[span.clone()],
            &self.cost[span],
        );
        out.iter()
            .zip(to)
            .zip(cost)
            .map(|((&e, &y), &c)| (e, y as usize, c))
    }
}

/// The flow on the arcs and its residual graph. Arc `i` owns edge `2i`,
/// tail to head at the arc's cost while it has lots left, and edge `2i + 1`,
/// head to tail at minus its cost while it carries lots. Bellman-Ford scans
/// the live edges in id order (as the plain formulation's rebuilt lists
/// held them), and so does the FIFO search at each node.
struct Net {
    /// Per arc: lots, FIFO rank, cost per lot, `[tail, head]` nodes, flow.
    cap: Vec<u64>,
    rank: Vec<u128>,
    cost: Vec<i64>,
    ends: Vec<[u32; 2]>,
    flow: Vec<u64>,
    taker: Option<usize>,
    /// Nodes.
    n: usize,
    /// Every edge.
    all: Adjacency,
}

impl Net {
    /// The resting arcs, then the taker's (if any).
    fn new(arcs: &[Arc], taker: Option<&Taker>, nodes: &Nodes) -> Self {
        let taker_arc = taker.map(Taker::arc);
        let all = || arcs.iter().chain(&taker_arc);
        let node = |p: Pos| nodes.of(p) as u32;
        let mut net = Net {
            cap: all().map(|a| a.cap).collect(),
            rank: all().map(|a| a.rank).collect(),
            cost: all().map(Arc::cost).collect(),
            ends: all().map(|a| [node(a.tail()), node(a.head())]).collect(),
            flow: vec![0; arcs.len() + usize::from(taker.is_some())],
            taker: taker.map(|_| arcs.len()),
            n: nodes.len(),
            all: Adjacency {
                first: Vec::new(),
                out: Vec::new(),
                to: Vec::new(),
                cost: Vec::new(),
            },
        };
        net.all = net.adjacency(|_| true);
        net
    }

    /// Edge ids, ascending.
    fn ids(&self) -> std::ops::Range<u32> {
        0..2 * self.cap.len() as u32
    }

    /// Per-node lists, by id, of the edges of the arcs `keep` admits.
    fn adjacency(&self, keep: impl Fn(usize) -> bool) -> Adjacency {
        let mut first = vec![0u32; self.n + 1];
        for (i, ends) in self.ends.iter().enumerate() {
            if keep(i) {
                for x in ends {
                    first[*x as usize + 1] += 1;
                }
            }
        }
        for x in 0..self.n {
            first[x + 1] += first[x];
        }
        let len = first[self.n] as usize;
        let (mut out, mut to, mut cost) = (vec![0u32; len], vec![0u32; len], vec![0i64; len]);
        let mut next = first.clone();
        // Arc by arc, edge 2i (leaving the tail) before 2i + 1 (leaving the
        // head): every list in id order.
        for (i, ends) in self.ends.iter().enumerate() {
            if !keep(i) {
                continue;
            }
            for (dir, c) in [(0, self.cost[i]), (1, -self.cost[i])] {
                let x = ends[dir] as usize;
                let at = next[x] as usize;
                out[at] = (2 * i + dir) as u32;
                to[at] = ends[dir ^ 1];
                cost[at] = c;
                next[x] += 1;
            }
        }
        Adjacency {
            first,
            out,
            to,
            cost,
        }
    }

    fn from(&self, e: u32) -> usize {
        self.ends[arc_of(e)][(e & 1) as usize] as usize
    }

    fn to(&self, e: u32) -> usize {
        self.ends[arc_of(e)][((e & 1) ^ 1) as usize] as usize
    }

    fn edge_cost(&self, e: u32) -> i64 {
        let c = self.cost[arc_of(e)];
        if e & 1 == 0 {
            c
        } else {
            -c
        }
    }

    fn room(&self, e: u32) -> u64 {
        let i = arc_of(e);
        if e & 1 == 0 {
            self.cap[i] - self.flow[i]
        } else {
            self.flow[i]
        }
    }

    fn push(&mut self, e: u32, d: u64) {
        let i = arc_of(e);
        if e & 1 == 0 {
            self.flow[i] += d;
        } else {
            self.flow[i] -= d;
        }
    }

    /// The live edges in id order, without the taker's if `skip_taker`.
    fn residual(&self, skip_taker: bool, out: &mut Vec<Edge>) {
        out.clear();
        for e in self.ids() {
            if (skip_taker && Some(arc_of(e)) == self.taker) || self.room(e) == 0 {
                continue;
            }
            out.push(Edge {
                from: self.from(e),
                to: self.to(e),
                cost: self.edge_cost(e),
                id: e,
            });
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
        let n = self.n;
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

    /// Shortest distances from a virtual source (0 to every node) over the
    /// live edges (without the taker's if `skip_taker`), by a queue of the
    /// nodes whose distance fell (SPFA). Shortest distances are unique, so
    /// they are the Bellman-Ford pass's. `None` if there is a negative
    /// cycle, found as a distance that came from a path of `n` edges or
    /// more: that path repeats a node, and its stretch between the two
    /// visits costs less than nothing, since the later visit improved on the
    /// earlier one.
    fn distances(&self, skip_taker: bool) -> Option<Vec<i64>> {
        let n = self.n;
        let mut dist = vec![0i64; n];
        // Edges on the path each distance came from.
        let mut edges = vec![0u32; n];
        let mut queued = vec![true; n];
        let mut queue: VecDeque<u32> = (0..n as u32).collect();
        while let Some(x) = queue.pop_front() {
            let x = x as usize;
            queued[x] = false;
            for (e, y, c) in self.all.of(x) {
                if self.room(e) == 0 || (skip_taker && Some(arc_of(e)) == self.taker) {
                    continue;
                }
                let d = dist[x] + c;
                if d < dist[y] {
                    dist[y] = d;
                    edges[y] = edges[x] + 1;
                    if edges[y] as usize >= n {
                        return None;
                    }
                    if !queued[y] {
                        queued[y] = true;
                        queue.push_back(y as u32);
                    }
                }
            }
        }
        Some(dist)
    }

    /// Cancels every negative cycle among the resting arcs and returns the
    /// shortest distances from a virtual source over what is left: valid
    /// potentials for the residual graph without the taker. A crossed book
    /// cancels its cycles as Bellman-Ford finds them.
    fn cancel_negative_cycles(&mut self) -> Vec<i64> {
        let mut edges = Vec::with_capacity(2 * self.cap.len());
        let all: Vec<usize> = (0..self.n).collect();
        loop {
            if let Some(dist) = self.distances(true) {
                return dist;
            }
            self.residual(true, &mut edges);
            let (v, pred) = match self.bellman_ford(&edges, &all) {
                Ok((dist, _)) => return dist,
                Err(found) => found,
            };
            // Walk back n steps to land on the cycle, then collect it.
            let mut x = v;
            for _ in 0..self.n {
                x = edges[pred[x].expect("pred")].from;
            }
            let mut cycle = Vec::new();
            let mut y = x;
            loop {
                let k = pred[y].expect("pred");
                cycle.push(edges[k].id);
                y = edges[k].from;
                if y == x {
                    break;
                }
            }
            let d = cycle.iter().map(|&e| self.room(e)).min().unwrap_or(0);
            for &e in &cycle {
                self.push(e, d);
            }
        }
    }
}

fn arc_of(e: u32) -> usize {
    (e >> 1) as usize
}

/// When a Bellman-Ford pass whose rounds scan the edge ids `0..w - 1` in
/// order next scans edge `e`, after time `round * w + at`. Time
/// `r * w + k + 1` is the scan of edge `k` in round `r`; time 0 is before
/// round 0.
fn next_scan(e: u32, (round, at): (u64, u64), w: u64) -> u64 {
    let k = u64::from(e) + 1;
    if k > at {
        round * w + k
    } else {
        (round + 1) * w + k
    }
}

/// Scratch space for [`Net::shortest`], reused across increments.
struct Search {
    /// Per node: its best label, reduced distance (never negative) and
    /// Bellman-Ford time packed as `distance << 64 | time`, so labels
    /// compare as pairs; its predecessor edge; settled.
    label: Vec<u128>,
    pred: Vec<u32>,
    done: Vec<bool>,
    /// A binary min-heap of the labelled nodes not yet settled, by label,
    /// and each node's place in it (`NEW`: not in it).
    heap: Vec<u32>,
    at: Vec<u32>,
}

impl Search {
    fn new(n: usize) -> Self {
        Search {
            label: vec![u128::MAX; n],
            pred: vec![u32::MAX; n],
            done: vec![false; n],
            heap: Vec::with_capacity(n),
            at: vec![NEW; n],
        }
    }

    fn reset(&mut self) {
        self.label.fill(u128::MAX);
        self.done.fill(false);
        self.heap.clear();
        self.at.fill(NEW);
    }

    /// Reduced distance of a labelled node.
    fn dist(&self, v: usize) -> i64 {
        (self.label[v] >> 64) as i64
    }

    /// Node `v`'s label just fell: into the heap, or up in it.
    fn up(&mut self, v: usize) {
        let mut i = match self.at[v] {
            NEW => {
                self.heap.push(v as u32);
                self.heap.len() - 1
            }
            i => i as usize,
        };
        while i > 0 {
            let parent = self.heap[(i - 1) / 2] as usize;
            if self.label[parent] < self.label[v] {
                break;
            }
            self.heap[i] = parent as u32;
            self.at[parent] = i as u32;
            i = (i - 1) / 2;
        }
        self.heap[i] = v as u32;
        self.at[v] = i as u32;
    }

    /// The node with the least label, out of the heap.
    fn pop(&mut self) -> Option<usize> {
        let top = *self.heap.first()? as usize;
        let last = self.heap.pop()? as usize;
        self.at[top] = NEW;
        if last != top {
            let len = self.heap.len();
            let mut i = 0;
            loop {
                let mut c = 2 * i + 1;
                if c >= len {
                    break;
                }
                if c + 1 < len
                    && self.label[self.heap[c + 1] as usize] < self.label[self.heap[c] as usize]
                {
                    c += 1;
                }
                let child = self.heap[c] as usize;
                if self.label[last] < self.label[child] {
                    break;
                }
                self.heap[i] = child as u32;
                self.at[child] = i as u32;
                i = c;
            }
            self.heap[i] = last as u32;
            self.at[last] = i as u32;
        }
        Some(top)
    }
}

impl Net {
    /// The cheapest residual path from `s` to `z` that avoids the taker,
    /// under potentials `pi` (every live edge's reduced cost
    /// `cost + pi[from] - pi[to]` is non-negative): `false` if there is none,
    /// else `sp.dist(z)` is its reduced cost and `sp.pred` leads back from
    /// `z`. Nodes settled before `z` have their exact distance, others at
    /// least `z`'s.
    ///
    /// Of equally cheap paths it is the one a Bellman-Ford pass from `s`
    /// takes: relaxing every live edge in id order, round after round, in
    /// place, keeping an edge as a node's predecessor only when it strictly
    /// improves the node. That pass gives a node its final distance, and its
    /// predecessor, at the first scan of a tight edge (one on a shortest
    /// path) out of a node that already has its own; anything earlier is
    /// improved on later, anything later is no improvement. So with each
    /// node's time being that moment and `s` at 0, a node's time is the
    /// least next scan of `e` after `time(from)` over its tight in-edges
    /// `e` ([`next_scan`]). That grows along every edge and never falls when
    /// the departure time does, so Dijkstra on the pair (reduced distance,
    /// time) settles each node at Bellman-Ford's distance and with
    /// Bellman-Ford's predecessor (distinct edges never scan at the same
    /// time, so there are no ties). The order it looks at a node's edges in
    /// doesn't matter.
    fn shortest(&self, s: usize, z: usize, pi: &[i64], sp: &mut Search) -> bool {
        let taker = self.taker.unwrap_or(usize::MAX);
        let w = 2 * self.cap.len() as u64 + 1;
        sp.reset();
        sp.label[s] = 0;
        sp.up(s);
        while let Some(x) = sp.pop() {
            sp.done[x] = true;
            if x == z {
                return true;
            }
            let (d, t) = (sp.dist(x), sp.label[x] as u64);
            let at = (t / w, t % w);
            for (e, y, c) in self.all.of(x) {
                if sp.done[y] || self.room(e) == 0 || arc_of(e) == taker {
                    continue;
                }
                let nd = d + (c + pi[x] - pi[y]);
                debug_assert!(nd >= d, "a negative reduced cost");
                let high = u128::from(nd as u64) << 64;
                if high > sp.label[y] {
                    continue;
                }
                let label = high | u128::from(next_scan(e, at, w));
                if label < sp.label[y] {
                    sp.label[y] = label;
                    sp.pred[y] = e;
                    sp.up(y);
                }
            }
        }
        false
    }

    /// Successive shortest paths for arc `ti`: route lots from its head back
    /// to its tail along the cheapest residual path while the path costs no
    /// more than the arc's own (negated) cost. `pi` are potentials for the
    /// residual graph without `ti` and stay so. `level(cost, lots)` sees
    /// each increment's path cost per lot.
    fn route(&mut self, ti: usize, pi: &mut [i64], mut level: impl FnMut(i64, u64)) {
        let (cap, own) = (self.cap[ti], self.cost[ti]);
        let [z, s] = self.ends[ti].map(|x| x as usize);
        let mut sp = Search::new(self.n);
        let mut path = Vec::new();
        while self.flow[ti] < cap {
            if !self.shortest(s, z, pi, &mut sp) {
                break;
            }
            let cost = sp.dist(z) - pi[s] + pi[z];
            if cost + own > 0 {
                break;
            }
            path.clear();
            let mut y = z;
            while y != s {
                let e = sp.pred[y];
                path.push(e);
                y = self.from(e);
            }
            let d = path
                .iter()
                .map(|&e| self.room(e))
                .min()
                .unwrap_or(u64::MAX)
                .min(cap - self.flow[ti]);
            for &e in &path {
                self.push(e, d);
            }
            self.flow[ti] += d;
            level(cost, d);
            // Add each node's distance, capped at z's: every reduced cost
            // stays non-negative, and the path's reversed edges cost 0.
            for (v, p) in pi.iter_mut().enumerate() {
                *p += sp.dist(if sp.done[v] { v } else { z });
            }
        }
    }
}

/// Scratch space for the FIFO searches.
struct Bfs {
    /// `seen[x] == stamp`: reached in the current search.
    seen: Vec<u32>,
    stamp: u32,
    pred: Vec<u32>,
    queue: Vec<u32>,
    /// The last path found, from its end back to its start.
    path: Vec<u32>,
}

impl Net {
    /// Strongly connected components of the live edges in `adj`: the
    /// component of each node (Tarjan's algorithm, without recursion).
    fn components(&self, adj: &Adjacency) -> Vec<u32> {
        let n = self.n;
        let mut index = vec![NEW; n];
        let mut low = vec![0u32; n];
        let mut comp = vec![NEW; n];
        let mut stack: Vec<u32> = Vec::with_capacity(n);
        // The search's own stack: a node and the next of its edges to take.
        let mut calls: Vec<(usize, u32)> = Vec::with_capacity(n);
        let (mut numbered, mut comps) = (0u32, 0u32);
        for root in 0..n {
            if index[root] != NEW {
                continue;
            }
            index[root] = numbered;
            low[root] = numbered;
            numbered += 1;
            stack.push(root as u32);
            calls.push((root, adj.first[root]));
            while let Some(&(x, at)) = calls.last() {
                if at < adj.first[x + 1] {
                    let top = calls.len() - 1;
                    calls[top].1 += 1;
                    let (e, y) = (adj.out[at as usize], adj.to[at as usize] as usize);
                    if self.room(e) == 0 {
                        continue;
                    }
                    if index[y] == NEW {
                        index[y] = numbered;
                        low[y] = numbered;
                        numbered += 1;
                        stack.push(y as u32);
                        calls.push((y, adj.first[y]));
                    } else if comp[y] == NEW {
                        // Still on the stack: in x's component or an
                        // enclosing one.
                        low[x] = low[x].min(index[y]);
                    }
                    continue;
                }
                calls.pop();
                if let Some(&(parent, _)) = calls.last() {
                    low[parent] = low[parent].min(low[x]);
                }
                if low[x] == index[x] {
                    while let Some(v) = stack.pop() {
                        comp[v as usize] = comps;
                        if v as usize == x {
                            break;
                        }
                    }
                    comps += 1;
                }
            }
        }
        comp
    }

    /// Breadth-first, in the order of the plain search over a rebuilt edge
    /// list: a residual path from `s` to `z` over the live edges in `adj`
    /// whose arcs rank after `rank`, into `b.path`.
    fn exchange_path(&self, adj: &Adjacency, rank: u128, s: usize, z: usize, b: &mut Bfs) -> bool {
        b.path.clear();
        if s == z {
            return true;
        }
        b.stamp += 1;
        b.seen[s] = b.stamp;
        b.queue.clear();
        b.queue.push(s as u32);
        let mut next = 0;
        while let Some(&x) = b.queue.get(next) {
            next += 1;
            for (e, y, _) in adj.of(x as usize) {
                if b.seen[y] == b.stamp || self.rank[arc_of(e)] <= rank || self.room(e) == 0 {
                    continue;
                }
                b.seen[y] = b.stamp;
                b.pred[y] = e;
                if y == z {
                    let mut v = z;
                    while v != s {
                        let e = b.pred[v];
                        b.path.push(e);
                        v = self.from(e);
                    }
                    return true;
                }
                b.queue.push(y as u32);
            }
        }
        false
    }

    /// Step 4 under `potentials`.
    fn fifo(&mut self, potentials: &[i64]) {
        // The arcs (never the taker) with zero reduced cost, oldest first.
        let zero: Vec<bool> = (0..self.cap.len())
            .map(|i| {
                let [tail, head] = self.ends[i].map(|x| x as usize);
                Some(i) != self.taker && self.cost[i] + potentials[tail] - potentials[head] == 0
            })
            .collect();
        let mut order: Vec<usize> = (0..self.cap.len()).filter(|&i| zero[i]).collect();
        order.sort_by_key(|&i| self.rank[i]);
        let adj = self.adjacency(|i| zero[i]);
        // A path that lets arc i fill more closes a cycle with i's own
        // forward edge, so i's ends share a strongly connected component.
        // Pushing lots around such a cycle adds edges (its reverses) only
        // inside one component, so no edge between components ever appears
        // and no later cycle crosses components: an arc whose ends lie in
        // different components now can't fill more in this step, and its
        // search, which would fail, is skipped.
        let comp = self.components(&adj);
        let mut b = Bfs {
            seen: vec![0; self.n],
            stamp: 0,
            pred: vec![u32::MAX; self.n],
            queue: Vec::with_capacity(self.n),
            path: Vec::new(),
        };
        for i in order {
            let [z, s] = self.ends[i].map(|x| x as usize);
            if comp[s] != comp[z] {
                continue;
            }
            while self.flow[i] < self.cap[i] {
                if !self.exchange_path(&adj, self.rank[i], s, z, &mut b) {
                    break;
                }
                let d = b
                    .path
                    .iter()
                    .map(|&e| self.room(e))
                    .min()
                    .unwrap_or(u64::MAX)
                    .min(self.cap[i] - self.flow[i]);
                for &e in &b.path {
                    self.push(e, d);
                }
                self.flow[i] += d;
            }
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
    let ti = arcs.len();
    let mut net = Net::new(arcs, Some(&taker), &nodes);
    let mut pi = net.cancel_negative_cycles();
    let mut levels: Vec<(i64, u64)> = Vec::new();
    net.route(ti, &mut pi, |cost, lots| {
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
    solve_with_marks(arcs, taker, grid, &mut |_, _| {})
}

/// [`solve`], calling `mark` as each phase ends with its name ("cancel",
/// "route", "potentials", "fifo") and the flow on every arc (the taker's
/// last): the bench times the phases and the tests compare them.
#[doc(hidden)]
pub fn solve_with_marks(
    arcs: &[Arc],
    taker: Option<&Taker>,
    grid: Pos,
    mark: &mut dyn FnMut(&'static str, &[u64]),
) -> Solution {
    let nodes = Nodes::new(arcs, taker, grid);
    let mut net = Net::new(arcs, taker, &nodes);
    let t = net.taker;

    let mut pi = net.cancel_negative_cycles();
    mark("cancel", &net.flow);

    // Successive shortest paths for the taker.
    if let Some(ti) = t {
        net.route(ti, &mut pi, |_, _| {});
    }
    mark("route", &net.flow);

    // Potentials: shortest distances from a virtual source.
    let potentials = net.distances(false).unwrap_or_else(|| {
        let mut edges = Vec::new();
        net.residual(false, &mut edges);
        let every: Vec<usize> = (0..nodes.len()).collect();
        net.bellman_ford(&edges, &every).expect("optimal flow").0
    });
    mark("potentials", &net.flow);

    // FIFO: oldest first, saturate zero-cost exchanges through newer arcs.
    net.fifo(&potentials);
    mark("fifo", &net.flow);

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
