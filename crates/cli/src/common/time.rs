// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The one place that reads the wall clock as a plain number. A clock set
//! before 1970 reads as 0, so callers never handle an error for it.

use std::time::{SystemTime, UNIX_EPOCH};

fn since_epoch() -> std::time::Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

/// Whole seconds since the unix epoch, for stamps and age maths.
pub fn now_secs() -> i64 {
    i64::try_from(since_epoch().as_secs()).unwrap_or(i64::MAX)
}

/// Whole seconds since the unix epoch, unsigned, for file names and stamps.
pub fn now_epoch_secs() -> u64 {
    since_epoch().as_secs()
}

/// Whole milliseconds since the unix epoch.
pub fn now_ms() -> u64 {
    u64::try_from(since_epoch().as_millis()).unwrap_or(u64::MAX)
}

/// Nanoseconds since the unix epoch, for names that must not collide.
pub fn now_nanos() -> u128 {
    since_epoch().as_nanos()
}

pub use playbook_common::time::iso_utc;

/// Whole seconds between the unix epoch and `time`, or `None` before it.
pub fn epoch_secs_of(time: SystemTime) -> Option<i64> {
    let secs = time.duration_since(UNIX_EPOCH).ok()?.as_secs();
    i64::try_from(secs).ok()
}

/// Parses a `YYYY-MM-DDTHH:MM:SSZ` UTC timestamp (fractional seconds are
/// dropped) into unix seconds, the inverse of `iso_utc`. Anything else, an
/// out of range field included, is `None`.
pub fn parse_iso8601_utc_epoch(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    if !(0..=9999).contains(&year) {
        return None;
    }
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next()?.split('.').next()?.parse().ok()?;

    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return None;
    }

    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Howard Hinnant's `days_from_civil`: days since the Unix epoch for a
/// proleptic-Gregorian `(year, month, day)`.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (i64::from(month) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_helpers_agree_and_are_after_2020() {
        let secs = now_secs();
        assert!(secs > 1_577_836_800);
        assert!(now_epoch_secs() as i64 >= secs);
        assert!(now_ms() / 1000 >= secs as u64);
        assert!(now_nanos() / 1_000_000_000 >= secs as u128);
    }

    #[test]
    fn epoch_secs_of_reads_a_time_and_rejects_one_before_the_epoch() {
        let later = UNIX_EPOCH + std::time::Duration::from_secs(1234);
        assert_eq!(epoch_secs_of(later), Some(1234));
        let before = UNIX_EPOCH - std::time::Duration::from_secs(1);
        assert_eq!(epoch_secs_of(before), None);
    }
}

#[cfg(test)]
mod iso_tests {
    use super::iso_utc;

    #[test]
    fn utc_timestamps_parse_back_to_what_iso_utc_wrote() {
        for secs in [0, 1_709_208_000, 1_792_024_626] {
            assert_eq!(super::parse_iso8601_utc_epoch(&iso_utc(secs)), Some(secs));
        }
        assert_eq!(
            super::parse_iso8601_utc_epoch("2026-10-15T00:37:06.5Z"),
            Some(1_792_024_626)
        );
        for bad in [
            "2026-13-01T00:00:00Z",
            "2026-10-15T-1:00:00Z",
            "99999-01-01T00:00:00Z",
            "2026-10-15T00:00:60Z",
            "yesterday",
        ] {
            assert_eq!(super::parse_iso8601_utc_epoch(bad), None, "{bad}");
        }
    }
}
