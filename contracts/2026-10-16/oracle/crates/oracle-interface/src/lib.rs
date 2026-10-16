//! Account layouts and validated readers for programs that consume Ameba oracle
//! state. A consumer binds the expected oracle program id from its own
//! deployment profile; nothing here trusts a caller-supplied program identity.
//!
//! Every reader checks owner, exact length, discriminator, version, stored-bump
//! PDA, status invariants and zero reserved bytes before returning a value.

use solana_program::{account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey};

/// Every oracle PDA begins with this seed.
pub const NAMESPACE_SEED: &[u8] = b"ameba-oracle-v1";
pub const PERIOD_SEED: &[u8] = b"period";
pub const MAX_VALUE_DECIMALS: u8 = 18;

pub fn find_period_address(
    program: &Pubkey,
    product_id: &[u8; 32],
    expiry_ts: u64,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            NAMESPACE_SEED,
            PERIOD_SEED,
            product_id,
            &expiry_ts.to_le_bytes(),
        ],
        program,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PeriodStatus {
    /// Sources and the opening value are still being formed. Not tradable.
    Forming = 0,
    /// The opening value is final; trading may run from `effective_listing_ts`.
    Live = 1,
    /// The settlement value is final and immutable.
    Settled = 2,
    /// The period will never list or settle through the ordinary path.
    Failed = 3,
}

impl PeriodStatus {
    fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Forming,
            1 => Self::Live,
            2 => Self::Settled,
            3 => Self::Failed,
            _ => return None,
        })
    }
}

/// One product's index over one fixed expiry. This is the whole contract between
/// the oracle and its consumers: how the oracle forms the values is internal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Period {
    pub bump: u8,
    pub status: PeriodStatus,
    pub value_decimals: u8,
    pub product_id: [u8; 32],
    pub planned_listing_ts: u64,
    /// Immutable. Late formation may move effective listing, never expiry.
    pub expiry_ts: u64,
    /// Zero until Live; then at or after planned listing and before expiry.
    pub effective_listing_ts: u64,
    /// Zero before settlement, and also a valid settled value in version 2.
    pub settlement_value: u64,
    /// Zero until Settled; then at or after expiry.
    pub settled_ts: u64,
    /// Commitment to the full settlement computation. Zero until Settled.
    pub settlement_digest: [u8; 32],
}

/// A final settlement value, available only from a Settled period.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Settlement {
    pub value: u64,
    pub decimals: u8,
    pub settled_ts: u64,
    pub digest: [u8; 32],
}

// Byte offsets. Reserved bytes must be zero.
const DISCRIMINATOR: core::ops::Range<usize> = 0..8;
const VERSION: usize = 8;
const BUMP: usize = 9;
const STATUS: usize = 10;
const VALUE_DECIMALS: usize = 11;
const PRODUCT_ID: core::ops::Range<usize> = 12..44;
const PLANNED_LISTING_TS: usize = 44;
const EXPIRY_TS: usize = 52;
const EFFECTIVE_LISTING_TS: usize = 60;
const SETTLEMENT_VALUE: usize = 68;
const SETTLED_TS: usize = 76;
const SETTLEMENT_DIGEST: core::ops::Range<usize> = 84..116;
const RESERVED: core::ops::Range<usize> = 116..148;

impl Period {
    pub const LEN: usize = 148;
    pub const DISCRIMINATOR: [u8; 8] = *b"AOPERIOD";
    pub const VERSION: u8 = 2;
    /// Version one has the same layout, but rejects zero-valued settlements.
    pub const LEGACY_VERSION: u8 = 1;

    pub fn new_forming(
        bump: u8,
        product_id: [u8; 32],
        planned_listing_ts: u64,
        expiry_ts: u64,
        value_decimals: u8,
    ) -> Self {
        Self {
            bump,
            status: PeriodStatus::Forming,
            value_decimals,
            product_id,
            planned_listing_ts,
            expiry_ts,
            effective_listing_ts: 0,
            settlement_value: 0,
            settled_ts: 0,
            settlement_digest: [0; 32],
        }
    }

    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut data = [0; Self::LEN];
        data[DISCRIMINATOR].copy_from_slice(&Self::DISCRIMINATOR);
        data[VERSION] = Self::VERSION;
        data[BUMP] = self.bump;
        data[STATUS] = self.status as u8;
        data[VALUE_DECIMALS] = self.value_decimals;
        data[PRODUCT_ID].copy_from_slice(&self.product_id);
        write_u64(&mut data, PLANNED_LISTING_TS, self.planned_listing_ts);
        write_u64(&mut data, EXPIRY_TS, self.expiry_ts);
        write_u64(&mut data, EFFECTIVE_LISTING_TS, self.effective_listing_ts);
        write_u64(&mut data, SETTLEMENT_VALUE, self.settlement_value);
        write_u64(&mut data, SETTLED_TS, self.settled_ts);
        data[SETTLEMENT_DIGEST].copy_from_slice(&self.settlement_digest);
        data
    }

    /// Decodes exact bytes and checks every invariant except account identity.
    pub fn decode(data: &[u8]) -> Result<Self, ProgramError> {
        if data.len() != Self::LEN
            || data[DISCRIMINATOR] != Self::DISCRIMINATOR
            || ![Self::LEGACY_VERSION, Self::VERSION].contains(&data[VERSION])
            || data[RESERVED].iter().any(|&b| b != 0)
        {
            return Err(ProgramError::InvalidAccountData);
        }
        let period = Self {
            bump: data[BUMP],
            status: PeriodStatus::from_u8(data[STATUS]).ok_or(ProgramError::InvalidAccountData)?,
            value_decimals: data[VALUE_DECIMALS],
            product_id: data[PRODUCT_ID].try_into().unwrap(),
            planned_listing_ts: read_u64(data, PLANNED_LISTING_TS),
            expiry_ts: read_u64(data, EXPIRY_TS),
            effective_listing_ts: read_u64(data, EFFECTIVE_LISTING_TS),
            settlement_value: read_u64(data, SETTLEMENT_VALUE),
            settled_ts: read_u64(data, SETTLED_TS),
            settlement_digest: data[SETTLEMENT_DIGEST].try_into().unwrap(),
        };
        period.validate()?;
        if data[VERSION] == Self::LEGACY_VERSION
            && period.status == PeriodStatus::Settled
            && period.settlement_value == 0
        {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(period)
    }

    pub fn validate(&self) -> Result<(), ProgramError> {
        let listed = self.effective_listing_ts >= self.planned_listing_ts
            && self.effective_listing_ts < self.expiry_ts;
        let unsettled =
            self.settlement_value == 0 && self.settled_ts == 0 && self.settlement_digest == [0; 32];
        let consistent = match self.status {
            PeriodStatus::Forming => self.effective_listing_ts == 0 && unsettled,
            PeriodStatus::Live => listed && unsettled,
            PeriodStatus::Settled => {
                listed && self.settled_ts >= self.expiry_ts && self.settlement_digest != [0; 32]
            }
            PeriodStatus::Failed => (self.effective_listing_ts == 0 || listed) && unsettled,
        };
        if !consistent
            || self.product_id == [0; 32]
            || self.planned_listing_ts >= self.expiry_ts
            || self.value_decimals > MAX_VALUE_DECIMALS
        {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }

    /// Live, listed and not yet expired at `now`.
    pub fn is_tradable_at(&self, now: u64) -> bool {
        self.status == PeriodStatus::Live
            && now >= self.effective_listing_ts
            && now < self.expiry_ts
    }

    pub fn settlement(&self) -> Option<Settlement> {
        (self.status == PeriodStatus::Settled).then_some(Settlement {
            value: self.settlement_value,
            decimals: self.value_decimals,
            settled_ts: self.settled_ts,
            digest: self.settlement_digest,
        })
    }
}

/// Loads any period owned by `oracle_program` after full layout and PDA checks.
/// The oracle creates periods only at their canonical bump, so the stored bump
/// is re-derived rather than searched for.
pub fn load_period(oracle_program: &Pubkey, info: &AccountInfo) -> Result<Period, ProgramError> {
    if info.owner != oracle_program {
        return Err(ProgramError::IllegalOwner);
    }
    let period = Period::decode(&info.try_borrow_data()?)?;
    let expected = Pubkey::create_program_address(
        &[
            NAMESPACE_SEED,
            PERIOD_SEED,
            &period.product_id,
            &period.expiry_ts.to_le_bytes(),
            &[period.bump],
        ],
        oracle_program,
    )
    .map_err(|_| ProgramError::InvalidSeeds)?;
    if info.key != &expected {
        return Err(ProgramError::InvalidSeeds);
    }
    Ok(period)
}

/// Loads the period for exactly `product_id` and `expiry_ts`.
pub fn load_period_for(
    oracle_program: &Pubkey,
    info: &AccountInfo,
    product_id: &[u8; 32],
    expiry_ts: u64,
) -> Result<Period, ProgramError> {
    let period = load_period(oracle_program, info)?;
    if &period.product_id != product_id || period.expiry_ts != expiry_ts {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(period)
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn write_u64(data: &mut [u8], offset: usize, value: u64) {
    data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRODUCT: [u8; 32] = [7; 32];
    const PLANNED: u64 = 1_793_491_200; // 2026-11-01T00:00:00Z
    const EXPIRY: u64 = 1_796_083_200; // 2026-12-01T00:00:00Z

    fn forming() -> Period {
        Period::new_forming(254, PRODUCT, PLANNED, EXPIRY, 6)
    }

    fn live() -> Period {
        Period {
            status: PeriodStatus::Live,
            effective_listing_ts: PLANNED + 3_600,
            ..forming()
        }
    }

    fn settled() -> Period {
        Period {
            status: PeriodStatus::Settled,
            settlement_value: 101_250_000,
            settled_ts: EXPIRY + 7_200,
            settlement_digest: [9; 32],
            ..live()
        }
    }

    #[test]
    fn every_valid_status_roundtrips() {
        let failed_unlisted = Period {
            status: PeriodStatus::Failed,
            ..forming()
        };
        let failed_listed = Period {
            status: PeriodStatus::Failed,
            ..live()
        };
        for period in [forming(), live(), settled(), failed_unlisted, failed_listed] {
            assert_eq!(Period::decode(&period.encode()).unwrap(), period);
        }
    }

    #[test]
    fn zero_settlement_is_explicitly_versioned() {
        let period = Period {
            settlement_value: 0,
            ..settled()
        };
        let bytes = period.encode();
        assert_eq!(
            Period::decode(&bytes).unwrap().settlement().unwrap().value,
            0
        );
        let mut legacy = bytes;
        legacy[VERSION] = Period::LEGACY_VERSION;
        assert_eq!(
            Period::decode(&legacy),
            Err(ProgramError::InvalidAccountData)
        );
        let mut legacy = settled().encode();
        legacy[VERSION] = Period::LEGACY_VERSION;
        assert_eq!(Period::decode(&legacy).unwrap(), settled());
    }

    #[test]
    fn layout_corruption_rejects() {
        let good = settled().encode();
        assert!(Period::decode(&good[..Period::LEN - 1]).is_err());
        assert!(Period::decode(&[good.as_slice(), &[0]].concat()).is_err());
        for (offset, value) in [
            (0, b'X'),
            (VERSION, 3),
            (STATUS, 4),
            (VALUE_DECIMALS, MAX_VALUE_DECIMALS + 1),
            (RESERVED.start, 1),
            (RESERVED.end - 1, 1),
        ] {
            let mut bad = good;
            bad[offset] = value;
            assert_eq!(Period::decode(&bad), Err(ProgramError::InvalidAccountData));
        }
    }

    #[test]
    fn status_invariants_reject() {
        let invalid = [
            Period {
                product_id: [0; 32],
                ..forming()
            },
            Period {
                planned_listing_ts: EXPIRY,
                ..forming()
            },
            Period {
                effective_listing_ts: PLANNED,
                ..forming()
            },
            Period {
                settlement_value: 1,
                ..forming()
            },
            Period {
                effective_listing_ts: PLANNED - 1,
                ..live()
            },
            Period {
                effective_listing_ts: EXPIRY,
                ..live()
            },
            Period {
                settled_ts: EXPIRY,
                ..live()
            },
            Period {
                settlement_digest: [1; 32],
                ..live()
            },
            Period {
                settled_ts: EXPIRY - 1,
                ..settled()
            },
            Period {
                settlement_digest: [0; 32],
                ..settled()
            },
            Period {
                effective_listing_ts: 0,
                ..settled()
            },
            Period {
                status: PeriodStatus::Failed,
                ..settled()
            },
            Period {
                status: PeriodStatus::Failed,
                effective_listing_ts: EXPIRY,
                ..live()
            },
        ];
        for period in invalid {
            assert_eq!(
                period.validate(),
                Err(ProgramError::InvalidAccountData),
                "{period:?}"
            );
            assert!(Period::decode(&period.encode()).is_err());
        }
    }

    #[test]
    fn tradability_and_settlement_follow_status() {
        let live = live();
        assert!(!live.is_tradable_at(live.effective_listing_ts - 1));
        assert!(live.is_tradable_at(live.effective_listing_ts));
        assert!(live.is_tradable_at(EXPIRY - 1));
        assert!(!live.is_tradable_at(EXPIRY));
        assert_eq!(live.settlement(), None);
        assert!(!forming().is_tradable_at(PLANNED));
        let settled = settled();
        assert!(!settled.is_tradable_at(PLANNED + 3_600));
        assert_eq!(
            settled.settlement(),
            Some(Settlement {
                value: 101_250_000,
                decimals: 6,
                settled_ts: EXPIRY + 7_200,
                digest: [9; 32],
            })
        );
    }

    fn with_account<R>(
        key: Pubkey,
        owner: Pubkey,
        data: &[u8],
        f: impl FnOnce(&AccountInfo) -> R,
    ) -> R {
        let mut lamports = 1;
        let mut data = data.to_vec();
        let info = AccountInfo::new(
            &key,
            false,
            false,
            &mut lamports,
            &mut data,
            &owner,
            false,
            0,
        );
        f(&info)
    }

    #[test]
    fn loader_binds_owner_pda_and_identity() {
        let program = Pubkey::new_unique();
        let (key, bump) = find_period_address(&program, &PRODUCT, EXPIRY);
        let period = Period { bump, ..settled() };
        let data = period.encode();

        with_account(key, program, &data, |info| {
            assert_eq!(load_period(&program, info).unwrap(), period);
            assert_eq!(
                load_period_for(&program, info, &PRODUCT, EXPIRY).unwrap(),
                period
            );
            assert_eq!(
                load_period_for(&program, info, &[8; 32], EXPIRY),
                Err(ProgramError::InvalidArgument)
            );
            assert_eq!(
                load_period_for(&program, info, &PRODUCT, EXPIRY + 1),
                Err(ProgramError::InvalidArgument)
            );
        });
        with_account(key, Pubkey::new_unique(), &data, |info| {
            assert_eq!(load_period(&program, info), Err(ProgramError::IllegalOwner));
        });
        with_account(Pubkey::new_unique(), program, &data, |info| {
            assert_eq!(load_period(&program, info), Err(ProgramError::InvalidSeeds));
        });
        // A period copied under another product's address does not verify.
        let (other_key, _) = find_period_address(&program, &[8; 32], EXPIRY);
        with_account(other_key, program, &data, |info| {
            assert_eq!(load_period(&program, info), Err(ProgramError::InvalidSeeds));
        });
        let mut wrong_bump = period.clone();
        wrong_bump.bump = bump.wrapping_sub(1);
        with_account(key, program, &wrong_bump.encode(), |info| {
            assert!(load_period(&program, info).is_err());
        });
    }
}
