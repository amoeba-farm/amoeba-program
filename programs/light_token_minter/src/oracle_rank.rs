//! Exact order statistics verified by a complete authenticated streaming walk.
//! Candidate values are untrusted. Every value must pass through `observe`, and
//! the caller must authenticate completeness before consuming `median`.
use crate::error::VaultError;
use solana_program::program_error::ProgramError;

#[derive(Clone, Debug, Default, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct MedianRank {
    pub lower: u64,
    pub upper: u64,
    pub count: u32,
    pub less_lower: u32,
    pub equal_lower: u32,
    pub less_upper: u32,
    pub equal_upper: u32,
}

impl MedianRank {
    pub fn observe(&mut self, value: u64) -> Result<(), ProgramError> {
        self.count = self
            .count
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        for (counter, increment) in [
            (&mut self.less_lower, value < self.lower),
            (&mut self.equal_lower, value == self.lower),
            (&mut self.less_upper, value < self.upper),
            (&mut self.equal_upper, value == self.upper),
        ] {
            *counter = counter
                .checked_add(u32::from(increment))
                .ok_or(VaultError::ArithmeticOverflow)?;
            if *counter > self.count {
                return Err(VaultError::InvalidOracleMedian.into());
            }
        }
        Ok(())
    }

    pub fn verify(&self) -> Result<(), ProgramError> {
        if self.count == 0 || self.lower > self.upper {
            return Err(VaultError::InvalidOracleMedian.into());
        }
        let low_rank = (self.count - 1) / 2;
        let high_rank = self.count / 2;
        for (rank, less, equal) in [
            (low_rank, self.less_lower, self.equal_lower),
            (high_rank, self.less_upper, self.equal_upper),
        ] {
            let end = less
                .checked_add(equal)
                .ok_or(VaultError::ArithmeticOverflow)?;
            if less > rank || rank >= end || end > self.count {
                return Err(VaultError::InvalidOracleMedian.into());
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
