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

/// `secs` since the unix epoch as `YYYY-MM-DDTHH:MM:SSZ` (UTC), for logs a
/// person reads. A negative value reads as the epoch.
pub fn iso_utc(secs: i64) -> String {
    let secs = secs.max(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// Whole seconds between the unix epoch and `time`, or `None` before it.
pub fn epoch_secs_of(time: SystemTime) -> Option<i64> {
    let secs = time.duration_since(UNIX_EPOCH).ok()?.as_secs();
    i64::try_from(secs).ok()
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
    fn formats_known_instants_in_utc() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso_utc(1_791_676_800), "2026-10-11T00:00:00Z");
        assert_eq!(iso_utc(-5), "1970-01-01T00:00:00Z");
    }
}
