//! Locked contribution lots: exact capital-time profit AND loss participation.
//!
//! Each lot owns an immutable interval in the pool's capital-seconds ledger.
//! Differences of cumulative floors allocate every atom independently of claim
//! order. Splitting an interval cannot create dust or change its total payout.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParticipationError {
    InvalidTime,
    InvalidPrincipal,
    InvalidInterval,
    Overflow,
    Insolvent,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ParticipationTotals {
    pub principal: u64,
    pub capital_seconds: u128,
    pub maximum_duration: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ContributionInterval {
    pub principal: u64,
    pub entry_ts: u64,
    pub expiry_ts: u64,
    pub weight_offset: u128,
}

impl ContributionInterval {
    pub fn weight(self) -> Result<u128, ParticipationError> {
        if self.principal == 0 {
            return Err(ParticipationError::InvalidPrincipal);
        }
        let duration = self
            .expiry_ts
            .checked_sub(self.entry_ts)
            .filter(|value| *value > 0)
            .ok_or(ParticipationError::InvalidTime)?;
        Ok(u128::from(self.principal) * u128::from(duration))
    }

    /// The returned prefix and suffix partition both principal and entitlement.
    pub fn split(self, prefix_principal: u64) -> Result<(Self, Self), ParticipationError> {
        if prefix_principal == 0 || prefix_principal >= self.principal {
            return Err(ParticipationError::InvalidPrincipal);
        }
        let prefix = Self {
            principal: prefix_principal,
            ..self
        };
        let suffix = Self {
            principal: self.principal - prefix_principal,
            weight_offset: self
                .weight_offset
                .checked_add(prefix.weight()?)
                .ok_or(ParticipationError::Overflow)?,
            ..self
        };
        Ok((prefix, suffix))
    }
}

impl ParticipationTotals {
    pub fn contribute(
        self,
        amount: u64,
        now: u64,
        start: u64,
        expiry: u64,
    ) -> Result<(Self, ContributionInterval), ParticipationError> {
        if start == 0 || start >= expiry || now >= expiry {
            return Err(ParticipationError::InvalidTime);
        }
        let lot = ContributionInterval {
            principal: amount,
            entry_ts: now.max(start),
            expiry_ts: expiry,
            weight_offset: self.capital_seconds,
        };
        let weight = lot.weight()?;
        Ok((
            Self {
                principal: self
                    .principal
                    .checked_add(amount)
                    .ok_or(ParticipationError::Overflow)?,
                capital_seconds: self
                    .capital_seconds
                    .checked_add(weight)
                    .ok_or(ParticipationError::Overflow)?,
                maximum_duration: self.maximum_duration.max(expiry - lot.entry_ts),
            },
            lot,
        ))
    }

    /// Global buyer solvency does not imply payable time-weighted writer losses.
    /// This division-free guard is equivalent to checking every constant lot.
    pub fn admit(self, assets: u64, exact_reserve: u64) -> Result<(), ParticipationError> {
        if self.principal == 0 || self.maximum_duration == 0 || self.capital_seconds == 0 {
            return Err(ParticipationError::InvalidPrincipal);
        }
        if assets < exact_reserve {
            return Err(ParticipationError::Insolvent);
        }
        // A >= R makes P + R - A <= P; subtract first to avoid u64 overflow.
        let worst_loss = self.principal.saturating_sub(assets - exact_reserve);
        if u128::from(worst_loss) * u128::from(self.maximum_duration) > self.capital_seconds {
            return Err(ParticipationError::Insolvent);
        }
        Ok(())
    }
}

/// floor(amount * numerator / denominator), with a 192-bit mathematical
/// product. Long multiplication/division avoids narrowing capital-seconds and
/// does not require an SBF-unfriendly arbitrary precision dependency.
pub fn proportional_floor(
    amount: u64,
    numerator: u128,
    denominator: u128,
) -> Result<u64, ParticipationError> {
    if denominator == 0 || numerator > denominator {
        return Err(ParticipationError::InvalidInterval);
    }
    let mut quotient = 0u64;
    let mut remainder = 0u128;
    for bit in (0..64).rev() {
        // Double the remainder modulo denominator without overflowing u128.
        let carry = remainder >= denominator - remainder;
        remainder = if carry {
            remainder - (denominator - remainder)
        } else {
            remainder * 2
        };
        quotient = quotient
            .checked_mul(2)
            .ok_or(ParticipationError::Overflow)?;
        if carry {
            quotient = quotient
                .checked_add(1)
                .ok_or(ParticipationError::Overflow)?;
        }
        if (amount >> bit) & 1 != 0 {
            let carry = remainder >= denominator - numerator;
            remainder = if carry {
                remainder - (denominator - numerator)
            } else {
                remainder + numerator
            };
            if carry {
                quotient = quotient
                    .checked_add(1)
                    .ok_or(ParticipationError::Overflow)?;
            }
        }
    }
    Ok(quotient)
}

pub fn final_payout(
    lot: ContributionInterval,
    total_principal: u64,
    total_weight: u128,
    residual: u64,
) -> Result<u64, ParticipationError> {
    let end = lot
        .weight_offset
        .checked_add(lot.weight()?)
        .ok_or(ParticipationError::Overflow)?;
    if end > total_weight || lot.principal > total_principal {
        return Err(ParticipationError::InvalidInterval);
    }
    let magnitude = residual.abs_diff(total_principal);
    let allocation = proportional_floor(magnitude, end, total_weight)?
        .checked_sub(proportional_floor(
            magnitude,
            lot.weight_offset,
            total_weight,
        )?)
        .ok_or(ParticipationError::InvalidInterval)?;
    if residual >= total_principal {
        lot.principal
            .checked_add(allocation)
            .ok_or(ParticipationError::Overflow)
    } else {
        lot.principal
            .checked_sub(allocation)
            .ok_or(ParticipationError::Insolvent)
    }
}
