//! Compact proposal encoding for the `Submit` instruction.
//!
//! A proposal names the lots filled on each arc (sparse, by [`ArcId`]) and one
//! potential per canonical node. Order fills are `(slot, lots)`; ladder fills
//! are runs of consecutive steps with equal lots on one side and tier of one
//! ladder (the side byte carries the tier: `side + 2 * tier`, so tier 0 runs
//! encode as before tiers existed).
//! Integers are LEB128 varints, potentials zigzag-coded deltas, so a typical
//! potential costs two bytes.
use crate::{Arc, ArcId, Pos, Side};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Proposal {
    /// Sorted by arc id, no duplicates, no zero lots.
    pub fills: Vec<(ArcId, u64)>,
    pub taker_fill: u64,
    pub potentials: Vec<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeError;

impl Proposal {
    /// From a solved package over `arcs`.
    pub fn from_fills(arcs: &[Arc], fills: &[u64], taker_fill: u64, potentials: &[i64]) -> Self {
        let mut v: Vec<(ArcId, u64)> = arcs
            .iter()
            .zip(fills)
            .filter(|(_, &m)| m > 0)
            .map(|(a, &m)| (a.id, m))
            .collect();
        v.sort();
        Proposal {
            fills: v,
            taker_fill,
            potentials: potentials.to_vec(),
        }
    }

    /// Dense fills parallel to `arcs` (which must be sorted by id). Fails on
    /// a fill for an arc that is not in the book.
    pub fn dense(&self, arcs: &[Arc]) -> Result<Vec<u64>, DecodeError> {
        let mut out = vec![0u64; arcs.len()];
        let mut i = 0;
        for &(id, m) in &self.fills {
            while i < arcs.len() && arcs[i].id < id {
                i += 1;
            }
            if i == arcs.len() || arcs[i].id != id {
                return Err(DecodeError);
            }
            out[i] = m;
            i += 1;
        }
        Ok(out)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let orders: Vec<(u16, u64)> = self
            .fills
            .iter()
            .filter_map(|&(id, m)| match id {
                ArcId::Order(s) => Some((s, m)),
                _ => None,
            })
            .collect();
        put(&mut out, orders.len() as u64);
        let mut last = 0u64;
        for (s, m) in orders {
            put(&mut out, u64::from(s) - last);
            last = u64::from(s);
            put(&mut out, m);
        }
        // Ladder runs: (maker, side) then consecutive steps with equal lots.
        // (maker, side byte, step, lots), the side byte carrying the tier.
        let mut steps: Vec<(u8, u8, Pos, u64)> = self
            .fills
            .iter()
            .filter_map(|&(id, m)| match id {
                ArcId::Maker {
                    maker,
                    step,
                    side,
                    tier,
                } => Some((maker, side as u8 + 2 * tier, step, m)),
                _ => None,
            })
            .collect();
        steps.sort();
        let mut runs: Vec<(u8, u8, Pos, u16, u64)> = Vec::new();
        for (mk, sb, step, m) in steps {
            match runs.last_mut() {
                Some(r) if r.0 == mk && r.1 == sb && r.2 + r.3 == step && r.4 == m => r.3 += 1,
                _ => runs.push((mk, sb, step, 1, m)),
            }
        }
        put(&mut out, runs.len() as u64);
        for (mk, sb, step, len, m) in runs {
            out.push(mk);
            out.push(sb);
            put(&mut out, u64::from(step));
            put(&mut out, u64::from(len));
            put(&mut out, m);
        }
        put(&mut out, self.taker_fill);
        put(&mut out, self.potentials.len() as u64);
        let mut prev = 0i64;
        for &p in &self.potentials {
            put(&mut out, zigzag(p.wrapping_sub(prev)));
            prev = p;
        }
        out
    }

    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader { b, at: 0 };
        let mut fills = Vec::new();
        let n = r.var()?;
        let mut slot = 0u64;
        for i in 0..n {
            let d = r.var()?;
            if i > 0 && d == 0 {
                return Err(DecodeError);
            }
            slot = slot.checked_add(d).ok_or(DecodeError)?;
            let m = r.var()?;
            let s = u16::try_from(slot).map_err(|_| DecodeError)?;
            if m == 0 || s == u16::MAX {
                return Err(DecodeError);
            }
            fills.push((ArcId::Order(s), m));
        }
        let runs = r.var()?;
        for _ in 0..runs {
            let maker = r.byte()?;
            let sb = r.byte()?;
            let side = Side::from_u8(sb & 1).ok_or(DecodeError)?;
            let tier = sb >> 1;
            if tier >= crate::MAX_TIERS {
                return Err(DecodeError);
            }
            let step = u16::try_from(r.var()?).map_err(|_| DecodeError)?;
            let len = u16::try_from(r.var()?).map_err(|_| DecodeError)?;
            let m = r.var()?;
            if m == 0 || len == 0 {
                return Err(DecodeError);
            }
            for s in 0..len {
                let step = step.checked_add(s).ok_or(DecodeError)?;
                fills.push((
                    ArcId::Maker {
                        maker,
                        step,
                        side,
                        tier,
                    },
                    m,
                ));
            }
        }
        fills.sort();
        if fills.windows(2).any(|w| w[0].0 == w[1].0) {
            return Err(DecodeError);
        }
        let taker_fill = r.var()?;
        let n = r.var()?;
        if n > 4096 {
            return Err(DecodeError);
        }
        let mut potentials = Vec::with_capacity(n as usize);
        let mut prev = 0i64;
        for _ in 0..n {
            prev = prev.wrapping_add(unzigzag(r.var()?));
            potentials.push(prev);
        }
        if r.at != b.len() {
            return Err(DecodeError);
        }
        Ok(Proposal {
            fills,
            taker_fill,
            potentials,
        })
    }
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

fn put(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, DecodeError> {
        let v = *self.b.get(self.at).ok_or(DecodeError)?;
        self.at += 1;
        Ok(v)
    }
    fn var(&mut self) -> Result<u64, DecodeError> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.byte()?;
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(DecodeError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mk = |maker, step, side, tier| ArcId::Maker {
            maker,
            step,
            side,
            tier,
        };
        let mut fills = vec![
            (ArcId::Order(3), 7),
            (ArcId::Order(9), 1),
            (mk(0, 10, Side::Long, 0), 5),
            (mk(0, 10, Side::Long, 1), 6),
            (mk(0, 10, Side::Short, 0), 2),
            (mk(0, 11, Side::Long, 0), 5),
            (mk(0, 11, Side::Long, 1), 6),
            (mk(0, 12, Side::Long, 0), 5),
            (mk(2, 4, Side::Short, 7), 9),
        ];
        fills.sort();
        let p = Proposal {
            fills,
            taker_fill: 12,
            potentials: vec![0, -10_000, 25, i64::MIN / 8, 7],
        };
        let b = p.encode();
        assert_eq!(Proposal::decode(&b).unwrap(), p);
        assert!(Proposal::decode(&b[..b.len() - 1]).is_err());
        // A tier-0 run reads exactly as before tiers existed (side byte 0/1).
        let old = Proposal {
            fills: vec![(mk(1, 5, Side::Short, 0), 3)],
            taker_fill: 0,
            potentials: vec![],
        };
        let e = old.encode();
        assert_eq!(&e[..4], &[0, 1, 1, 1]);
        let mut longer = b.clone();
        longer.push(0);
        assert!(Proposal::decode(&longer).is_err());
    }
}
