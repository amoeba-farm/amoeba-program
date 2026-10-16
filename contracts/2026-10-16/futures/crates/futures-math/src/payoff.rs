//! Claim payoffs and settlement in exact integers.
//!
//! Prices are atoms per lot. `k` is the atoms one lot of a one-step claim
//! pays in full, so a complete set on `[lo, hi)` is backed by `(hi - lo) * k`
//! per lot. Settlement is measured in *fine units* of `1/k` step, so every
//! claim's payout per lot is an integer and a long plus a short on the same
//! interval always pays exactly the set's backing: no rounding dust, no
//! rounding-created value.
use crate::Pos;

/// Backing per lot of a complete set on `[lo, hi)`.
pub fn set_value(lo: Pos, hi: Pos, k: u64) -> u64 {
    u64::from(hi - lo) * k
}

/// Settlement in fine units: `floor(value * k / step_value)`, where
/// `step_value` is the oracle value of one grid step (same decimals).
pub fn settle_fine(value: u64, step_value: u64, k: u64) -> u64 {
    (u128::from(value) * u128::from(k) / u128::from(step_value.max(1))) as u64
}

/// The long claim's payout per lot at settlement `s` (fine units).
pub fn long_per_lot(lo: Pos, hi: Pos, s: u64, k: u64) -> u64 {
    s.saturating_sub(u64::from(lo) * k)
        .min(set_value(lo, hi, k))
}

/// The short (complement) claim's payout per lot.
pub fn short_per_lot(lo: Pos, hi: Pos, s: u64, k: u64) -> u64 {
    set_value(lo, hi, k) - long_per_lot(lo, hi, s, k)
}

/// Cash a side puts into the set per lot at premium `price`: the long pays
/// the premium, the short funds the rest of the backing.
pub fn contribution(side: crate::Side, lo: Pos, hi: Pos, price: u64, k: u64) -> u64 {
    match side {
        crate::Side::Long => price,
        crate::Side::Short => set_value(lo, hi, k).saturating_sub(price),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_pay_exactly_their_backing() {
        let k = 10_000;
        for s in [
            0, 1, 979_999, 980_000, 1_000_001, 1_199_999, 1_200_000, 5_000_000,
        ] {
            let l = long_per_lot(98, 120, s, k);
            let sh = short_per_lot(98, 120, s, k);
            assert_eq!(l + sh, 22 * k);
        }
        assert_eq!(long_per_lot(98, 120, 1_000_000, k), 20_000);
        assert_eq!(long_per_lot(98, 120, 0, k), 0);
        assert_eq!(long_per_lot(98, 120, 9_999_999, k), 220_000);
    }

    #[test]
    fn adjacent_pieces_add_up() {
        let k = 10_000;
        for s in (0..2_000_000).step_by(3_331) {
            let whole = long_per_lot(98, 120, s, k);
            let parts = long_per_lot(98, 102, s, k)
                + long_per_lot(102, 110, s, k)
                + long_per_lot(110, 120, s, k);
            assert_eq!(whole, parts);
            // R[100,112) + R[110,120) == R[100,120) + R[110,112)
            assert_eq!(
                long_per_lot(100, 112, s, k) + long_per_lot(110, 120, s, k),
                long_per_lot(100, 120, s, k) + long_per_lot(110, 112, s, k)
            );
        }
    }

    #[test]
    fn fine_units() {
        // One point per step, six decimals, k = 10_000: fine unit 0.0001 points.
        assert_eq!(settle_fine(100_000_000, 1_000_000, 10_000), 1_000_000);
        assert_eq!(settle_fine(100_123_456, 1_000_000, 10_000), 1_001_234);
    }
}
