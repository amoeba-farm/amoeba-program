//! Time: the settlement window, time-weighted delegation, and calendar months.

const DAY: u64 = 86_400;

/// Monday to Friday, UTC (1970-01-01 was a Thursday).
pub fn is_business_day(ts: u64) -> bool {
    (ts / DAY + 3) % 7 < 5
}

/// The start of the settlement window: `business_days` UTC business days back from expiry,
/// counted in whole days as the oracle always has (`oracle_settlement_window_start`).
pub fn settlement_window_start(expiry_ts: u64, business_days: u8) -> u64 {
    let mut cursor = expiry_ts;
    let mut remaining = business_days;
    while remaining > 0 && cursor >= DAY {
        cursor -= DAY;
        if is_business_day(cursor) {
            remaining -= 1;
        }
    }
    cursor
}

/// `amount` held from `from` to `to`, in atom-seconds (zero when `to <= from`).
pub fn integral(amount: u64, from: u64, to: u64) -> u128 {
    u128::from(amount) * u128::from(to.saturating_sub(from))
}

/// A running time integral of a piecewise-constant amount: what a delegation contributes to a
/// contract's settlement weight is `integral(freeze) - integral(listing)`, divided by the
/// window (spec §3, "When delegation changes bite").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Accrual {
    pub amount: u64,
    pub since: u64,
    pub integral: u128,
}

impl Accrual {
    /// The integral at `at` (no earlier than `since`), without changing anything.
    pub fn at(&self, at: u64) -> u128 {
        self.integral + integral(self.amount, self.since, at)
    }

    /// Brings the integral up to `now`.
    pub fn advance(&mut self, now: u64) {
        if now > self.since {
            self.integral = self.at(now);
            self.since = now;
        }
    }

    /// Changes the amount from `now` on.
    pub fn set(&mut self, amount: u64, now: u64) {
        self.advance(now);
        self.amount = amount;
    }
}

/// The average over a window of `window_secs` whose integral is `integral`.
pub fn average(integral: u128, window_secs: u64) -> u64 {
    if window_secs == 0 {
        return 0;
    }
    u64::try_from(integral / u128::from(window_secs)).unwrap_or(u64::MAX)
}

/// Days since 1970-01-01 to `(year, month 1..=12, day)` (Howard Hinnant's civil_from_days).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `(year, month)` to days since 1970-01-01 of its first day.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The calendar month (UTC) of `ts` as `year * 12 + month - 1`: the board's revenue epoch.
pub fn month_id(ts: u64) -> u32 {
    let (year, month, _) = civil_from_days((ts / DAY) as i64);
    (year * 12 + i64::from(month) - 1) as u32
}

/// The first second of month `id`.
pub fn month_start(id: u32) -> u64 {
    let year = i64::from(id / 12);
    let month = id % 12 + 1;
    (days_from_civil(year, month, 1) as u64) * DAY
}

/// The calendar quarter (UTC) a contract expiring at `expiry_ts` covers: the one holding its last
/// second, as `(year, quarter 1..=4)`. A contract expiring 2027-04-01 00:00 is Q1 2027's.
pub fn contract_quarter(expiry_ts: u64) -> (i64, u8) {
    let (year, month, _) = civil_from_days((expiry_ts.saturating_sub(1) / DAY) as i64);
    (year, ((month - 1) / 3 + 1) as u8)
}

/// The first second of `quarter` (1..=4) of `year`, UTC.
pub fn quarter_start(year: i64, quarter: u8) -> u64 {
    let month = u32::from(quarter.clamp(1, 4) - 1) * 3 + 1;
    (days_from_civil(year, month, 1) as u64) * DAY
}

/// Whether `ts` is a calendar-quarter boundary: 00:00 UTC on Jan 1, Apr 1, Jul 1 or Oct 1.
pub fn is_quarter_boundary(ts: u64) -> bool {
    let (_, month, day) = civil_from_days((ts / DAY) as i64);
    ts % DAY == 0 && day == 1 && (month - 1) % 3 == 0
}
