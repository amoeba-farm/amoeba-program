//! The weighted median (spec §3): the index change of one component is the weighted median of
//! its sources' changes under their effective weights.

/// The weighted median of `(value, weight)` pairs, `None` when no pair has weight.
///
/// Read in value order (`sorted_positions`; the pairs stay as they are). The median is the first
/// value whose cumulative weight passes half the total. When the cumulative weight lands exactly on half, it is the
/// midpoint of that value and the next weighted one, rounded toward zero: with equal weights
/// this is the oracle's equal-weight median (`deterministic_bucket_median`) exactly.
pub fn weighted_median(pairs: &mut [(i64, u64)]) -> Option<i64> {
    let order = sorted_positions(pairs.len(), &|a, b| pairs[a].0 < pairs[b].0);
    let total: u128 = pairs.iter().map(|&(_, w)| u128::from(w)).sum();
    if total == 0 {
        return None;
    }
    let mut cumulative = 0u128;
    for (at, &k) in order.iter().enumerate() {
        let (value, weight) = pairs[k];
        if weight == 0 {
            continue;
        }
        cumulative += u128::from(weight);
        if cumulative * 2 > total {
            return Some(value);
        }
        if cumulative * 2 == total {
            let next = order[at + 1..]
                .iter()
                .map(|&m| pairs[m])
                .find(|&(_, w)| w > 0)
                .map(|(v, _)| v)?;
            return Some(midpoint_toward_zero(value, next));
        }
    }
    None
}

/// `(a + b) / 2` rounded toward zero, without overflow.
pub fn midpoint_toward_zero(a: i64, b: i64) -> i64 {
    let floor = (a & b) + ((a ^ b) >> 1);
    floor + i64::from(floor < 0 && (a ^ b) & 1 != 0)
}

/// The median of unsigned values (the midpoint of the middle pair rounded down), `None` when
/// empty. Thresholds (spec §6) use it over live sources' delegations.
pub fn median_u64(values: &mut [u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let order = sorted_positions(values.len(), &|a, b| values[a] < values[b]);
    let upper = values.len() / 2;
    if values.len() % 2 == 1 {
        return Some(values[order[upper]]);
    }
    let (a, b) = (values[order[upper - 1]], values[order[upper]]);
    Some((a & b) + ((a ^ b) >> 1))
}

/// The positions `0..n` in ascending order by `less` (on positions), stably: a bottom-up merge
/// sort, at most about `n log2 n` comparisons whatever the order, over two vectors of positions
/// (round 13 review: a settlement's 192 members, which an insertion sort took quadratically). One
/// instance for every caller (the comparison is dynamic), and in the program the core sorts it
/// replaces cost about 18 KB (size audit, 2026-10-30).
pub fn sorted_positions(n: usize, less: &dyn Fn(usize, usize) -> bool) -> Vec<usize> {
    let mut from: Vec<usize> = (0..n).collect();
    if n < 2 {
        return from;
    }
    let mut to = from.clone();
    let mut width = 1;
    while width < n {
        let mut start = 0;
        while start < n {
            let mid = (start + width).min(n);
            let end = (start + 2 * width).min(n);
            let (mut i, mut j) = (start, mid);
            for slot in &mut to[start..end] {
                // The left run's first unless the right run's is strictly less: stable.
                if i < mid && (j >= end || !less(from[j], from[i])) {
                    *slot = from[i];
                    i += 1;
                } else {
                    *slot = from[j];
                    j += 1;
                }
            }
            start = end;
        }
        core::mem::swap(&mut from, &mut to);
        width *= 2;
    }
    from
}

#[cfg(test)]
mod sort_tests {
    /// The merge sort orders as the standard (stable) sort does, on every length up to 200.
    #[test]
    fn the_sort_matches_the_standard_sort() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for len in 0..=200 {
            let values: Vec<u64> = (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed % 50
                })
                .collect();
            let mut expected: Vec<usize> = (0..values.len()).collect();
            expected.sort_by_key(|&i| values[i]);
            let order = super::sorted_positions(values.len(), &|a, b| values[a] < values[b]);
            assert_eq!(order, expected);
        }
    }
}
