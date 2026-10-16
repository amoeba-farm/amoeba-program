//! Ameba futures: the math shared by the on-chain program and its
//! off-chain clients.
//!
//! A market quotes one family of claims per index and expiry. Prices sit on a
//! grid of `0..=grid` positions (dev: one index point per step). The rising
//! range claim on `[lo, hi)` pays, per lot, `clamp(S - lo, 0, hi - lo)` steps
//! of `k` atoms at settlement `S`; its complement pays the rest of
//! `(hi - lo) * k`. One long plus one short claim on the same interval is a
//! complete set, backed by exactly `(hi - lo) * k` atoms per lot.
//!
//! An order is one claim shape with a limit premium per lot: a long pays at
//! most `price`, a short receives at least `price` (and funds the rest of the
//! set). Maker ladders offer every one-step claim `[x, x+1)` separately, so
//! they can fill any missing or surplus piece.
//!
//! **Packages.** Executions are balanced packages: at every grid step the
//! filled long lots covering the step equal the filled short lots covering it.
//! Read as a graph whose nodes are grid positions, a long fill is flow
//! `lo -> hi` and a short fill flow `hi -> lo`, so a balanced package is a
//! circulation. With arc costs `-price` (long) and `+price` (short), the
//! circulation cost is what shorts receive minus what longs pay; resting
//! orders trade at their own limits and the incoming order takes the rest.
//!
//! [`solve`] finds the package an incoming order is owed (off-chain, by
//! anyone); [`verify`] checks it on-chain against the complete book with a
//! dual certificate (node potentials). The priority rule, in order:
//!
//! 1. **Price:** the circulation with the incoming order's arc at its limit
//!    has minimum cost (the order gets the best total price).
//! 2. **Quantity:** among those, the incoming order fills as many lots as it
//!    can at its limit (equal-price liquidity is taken, as in any book).
//! 3. **FIFO:** among those, the vector of resting fills, ordered by arrival
//!    rank (oldest first), is lexicographically largest: an older order fills
//!    as much as any equal-price package allows before a newer one is used.
//!
//! See `docs/futures.md` for the product and accounting rules.
#![forbid(unsafe_code)]

pub mod codec;
pub mod payoff;
pub mod solve;
pub mod verify;

pub use solve::{depth, solve, Solution};
pub use verify::{verify, VerifyError};

/// A grid position (an index level in market steps).
pub type Pos = u16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Side {
    /// Holds the rising claim `R[lo,hi)`; pays the premium.
    Long = 0,
    /// Holds the complement; receives the premium, funds the rest of the set.
    Short = 1,
}

impl Side {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Side::Long),
            1 => Some(Side::Short),
            _ => None,
        }
    }
    pub fn opposite(self) -> Self {
        match self {
            Side::Long => Side::Short,
            Side::Short => Side::Long,
        }
    }
}

/// What an arc of the package graph stands for. The derived order is the
/// canonical order of arcs and of proposal fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArcId {
    /// A resting order, by slot.
    Order(u16),
    /// One depth tier of one step `[step, step+1)` of a maker ladder; `Long`
    /// is the maker's bid (it buys the step), `Short` its ask (it sells the
    /// step). Tier 0 is the best price.
    Maker {
        maker: u8,
        step: Pos,
        side: Side,
        tier: u8,
    },
}

/// One source of liquidity: an order, or one step of one side of a maker
/// ladder. `price` is the premium per lot for the whole interval, in atoms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arc {
    pub id: ArcId,
    pub side: Side,
    pub lo: Pos,
    pub hi: Pos,
    pub price: u64,
    /// Lots available.
    pub cap: u64,
    /// FIFO rank: smaller is older. Unique per arc.
    pub rank: u128,
}

/// FIFO rank of an order or ladder: its arrival sequence, then a sub-index
/// (a ladder's steps rank by position, bid before ask).
pub fn rank(seq: u64, sub: u32) -> u128 {
    (u128::from(seq) << 32) | u128::from(sub)
}

/// Most depth tiers a ladder step has per side.
pub const MAX_TIERS: u8 = 8;

/// The sub-index of a ladder arc within its ladder: by step, bid before ask,
/// then tier.
pub fn maker_sub(step: Pos, side: Side, tier: u8) -> u32 {
    (u32::from(step) * 2 + side as u32) * u32::from(MAX_TIERS) + u32::from(tier)
}

impl Arc {
    pub fn tail(&self) -> Pos {
        match self.side {
            Side::Long => self.lo,
            Side::Short => self.hi,
        }
    }
    pub fn head(&self) -> Pos {
        match self.side {
            Side::Long => self.hi,
            Side::Short => self.lo,
        }
    }
    /// Cost per lot in the circulation (shorts receive, longs pay).
    pub fn cost(&self) -> i64 {
        match self.side {
            Side::Long => -(self.price as i64),
            Side::Short => self.price as i64,
        }
    }
}

/// The incoming order: a whole claim shape at a limit premium per lot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Taker {
    pub side: Side,
    pub lo: Pos,
    pub hi: Pos,
    /// Long: most premium per lot it pays. Short: least premium per lot it
    /// receives.
    pub limit: u64,
    /// Lots wanted.
    pub qty: u64,
}

impl Taker {
    pub fn arc(&self) -> Arc {
        Arc {
            id: ArcId::Order(u16::MAX),
            side: self.side,
            lo: self.lo,
            hi: self.hi,
            price: self.limit,
            cap: self.qty,
            rank: u128::MAX,
        }
    }
}

/// Premium the taker pays (long, positive) or receives (short, as a positive
/// number) for a package: what resting shorts receive minus what resting
/// longs pay, signed for the taker's side.
pub fn taker_premium(side: Side, arcs: &[Arc], fills: &[u64]) -> i128 {
    let net: i128 = arcs
        .iter()
        .zip(fills)
        .map(|(a, &m)| i128::from(a.cost()) * i128::from(m))
        .sum();
    match side {
        Side::Long => net,
        Side::Short => -net,
    }
}

/// Canonical node set: the sorted distinct endpoints of every arc (and the
/// taker's), as a lookup from grid position to node index.
pub struct Nodes {
    pub positions: Vec<Pos>,
    index: Vec<u16>,
}

impl Nodes {
    pub fn new(arcs: &[Arc], taker: Option<&Taker>, grid: Pos) -> Self {
        let mut index = vec![u16::MAX; usize::from(grid) + 1];
        let mut mark = |p: Pos| {
            if let Some(slot) = index.get_mut(usize::from(p)) {
                *slot = 0;
            }
        };
        for a in arcs {
            mark(a.lo);
            mark(a.hi);
        }
        if let Some(t) = taker {
            mark(t.lo);
            mark(t.hi);
        }
        let mut positions = Vec::new();
        for (p, slot) in index.iter_mut().enumerate() {
            if *slot == 0 {
                *slot = positions.len() as u16;
                positions.push(p as Pos);
            }
        }
        Nodes { positions, index }
    }
    pub fn len(&self) -> usize {
        self.positions.len()
    }
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
    /// Node index of a grid position that is an endpoint.
    pub fn of(&self, p: Pos) -> usize {
        usize::from(self.index[usize::from(p)])
    }
}
