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
