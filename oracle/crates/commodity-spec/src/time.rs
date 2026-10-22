//! The two time forms submissions use: `YYYY-MM-DD` and `YYYY-MM-DDTHH:MM:SS[.fff]Z`.

/// Unix seconds of a date (its 00:00 UTC) or a UTC timestamp. `None` for anything else.
pub fn unix_seconds(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, time) = match text.split_once('T') {
        Some((date, time)) => (date, Some(time)),
        None => (text, None),
    };
    let days = days(date)?;
    let seconds = match time {
        None => 0,
        Some(time) => {
            let time = time.strip_suffix('Z')?;
            let time = time.split_once('.').map_or(time, |(whole, fraction)| {
                if fraction.bytes().all(|b| b.is_ascii_digit()) {
                    whole
                } else {
                    "invalid"
                }
            });
            let mut parts = time.split(':');
            let hours: i64 = number(parts.next()?, 2)?;
            let minutes: i64 = number(parts.next()?, 2)?;
            let secs: i64 = number(parts.next()?, 2)?;
            if parts.next().is_some() || hours > 23 || minutes > 59 || secs > 60 {
                return None;
            }
            hours * 3_600 + minutes * 60 + secs
        }
    };
    Some(days * 86_400 + seconds)
}

/// Days since 1970-01-01 of a `YYYY-MM-DD` date.
pub fn days(date: &str) -> Option<i64> {
    let mut parts = date.split('-');
    let year: i64 = number(parts.next()?, 4)?;
    let month: i64 = number(parts.next()?, 2)?;
    let day: i64 = number(parts.next()?, 2)?;
    if parts.next().is_some()
        || !(1..=12).contains(&month)
        || day < 1
        || day > month_days(year, month)
    {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn number(text: &str, width: usize) -> Option<i64> {
    (text.len() == width && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn month_days(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_timestamps() {
        assert_eq!(days("1970-01-01"), Some(0));
        assert_eq!(unix_seconds("2026-12-01"), Some(1_796_083_200));
        assert_eq!(unix_seconds("2026-12-01T00:00:01Z"), Some(1_796_083_201));
        assert_eq!(
            unix_seconds("2026-12-01T00:00:01.250Z"),
            Some(1_796_083_201)
        );
        assert_eq!(days("2024-02-29").map(|d| d + 1), days("2024-03-01"));
        for bad in [
            "2026-13-01",
            "2026-02-29",
            "26-12-01",
            "2026-12-01T00:00:00",
            "2026-12-01 00:00",
        ] {
            assert_eq!(unix_seconds(bad), None, "{bad}");
        }
    }
}
