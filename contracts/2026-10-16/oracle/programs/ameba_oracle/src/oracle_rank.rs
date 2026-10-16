//! Exact order statistics verified by a complete authenticated streaming walk.
//! Candidate values are untrusted. Every value must pass through `observe`, and
//! the caller must authenticate completeness before consuming `median`.
use crate::error::OracleError;
use crate::ProgramError;

crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Default, borsh::BorshSerialize)]
    pub struct MedianRank {
        pub lower: u64,
        pub upper: u64,
        pub count: u32,
        pub less_lower: u32,
        pub equal_lower: u32,
        pub less_upper: u32,
        pub equal_upper: u32,
    }
}

impl MedianRank {
    pub fn observe(&mut self, value: u64) -> Result<(), ProgramError> {
        self.count = self
            .count
            .checked_add(1)
            .ok_or(OracleError::ArithmeticOverflow)?;
        for (counter, increment) in [
            (&mut self.less_lower, value < self.lower),
            (&mut self.equal_lower, value == self.lower),
            (&mut self.less_upper, value < self.upper),
            (&mut self.equal_upper, value == self.upper),
        ] {
            *counter = counter
                .checked_add(u32::from(increment))
                .ok_or(OracleError::ArithmeticOverflow)?;
            if *counter > self.count {
                return Err(OracleError::InvalidOracleMedian.into());
            }
        }
        Ok(())
    }

    pub fn verify(&self) -> Result<(), ProgramError> {
        if self.count == 0 || self.lower > self.upper {
            return Err(OracleError::InvalidOracleMedian.into());
        }
        let low_rank = (self.count - 1) / 2;
        let high_rank = self.count / 2;
        for (rank, less, equal) in [
            (low_rank, self.less_lower, self.equal_lower),
            (high_rank, self.less_upper, self.equal_upper),
        ] {
            let end = less
                .checked_add(equal)
                .ok_or(OracleError::ArithmeticOverflow)?;
            if less > rank || rank >= end || end > self.count {
                return Err(OracleError::InvalidOracleMedian.into());
            }
        }
        Ok(())
    }

    pub fn median(&self) -> Result<u64, ProgramError> {
        self.verify()?;
        Ok((self.lower & self.upper) + ((self.lower ^ self.upper) >> 1))
    }

    /// Preserve signed bucket rounding toward zero, including the i64 extremes.
    pub fn signed_median(&self) -> Result<i64, ProgramError> {
        self.verify()?;
        let a = decode_signed(self.lower);
        let b = decode_signed(self.upper);
        Ok(((i128::from(a) + i128::from(b)) / 2) as i64)
    }
}

pub fn encode_signed(value: i64) -> u64 {
    (value as u64) ^ (1u64 << 63)
}
pub fn decode_signed(value: u64) -> i64 {
    (value ^ (1u64 << 63)) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_rank_matches_sorted_reference_and_rejects_false_candidates() {
        for n in [1usize, 2, 8, 9, 16, 32, 33, 64, 100, 1000] {
            let values: Vec<u64> = (0..n).map(|i| ((i * 7919 + 17) % 37) as u64 + 1).collect();
            let mut sorted = values.clone();
            sorted.sort_unstable();
            let mut rank = MedianRank {
                lower: sorted[(n - 1) / 2],
                upper: sorted[n / 2],
                ..Default::default()
            };
            for v in &values {
                rank.observe(*v).unwrap();
            }
            let expected =
                ((u128::from(sorted[(n - 1) / 2]) + u128::from(sorted[n / 2])) / 2) as u64;
            assert_eq!(rank.median().unwrap(), expected);
            let mut false_rank = MedianRank {
                lower: 999,
                upper: 999,
                ..Default::default()
            };
            for v in &values {
                false_rank.observe(*v).unwrap();
            }
            assert!(false_rank.median().is_err());
        }
        let mut rank = MedianRank {
            lower: encode_signed(i64::MIN),
            upper: encode_signed(i64::MAX),
            ..Default::default()
        };
        rank.observe(encode_signed(i64::MIN)).unwrap();
        rank.observe(encode_signed(i64::MAX)).unwrap();
        assert_eq!(rank.signed_median().unwrap(), 0);
    }
}

#[cfg(test)]
mod compact_borsh_equivalence {
    use super::*;
    use crate::fixed_codec::equivalence::check;

    #[test]
    fn compact_decoder_matches_derive_for_median_rank() {
        check::<MedianRank>(0x5ef6, 64);
    }
}
