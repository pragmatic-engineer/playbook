// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Spending limits and budget headroom. A limit is a dollar budget (manual, or
//! measured behind a Claude apps gateway) or a Pro and Max usage window in
//! percent (measured, from Claude Code's statusline input). The math here is
//! pure: it takes the period bounds, the amount used so far and the limit, and
//! says how fast the amount is being used, how fast it may be used for the
//! rest of the period, and where that ends. Every instant is a UTC second.
//!
//! The same math serves dollars and percent: for a window the limit is 100
//! and `used` is the percent consumed.

use super::aggregate::SECONDS_PER_DAY;
use super::query::{day_of, month_first_day, week_first_day, WeekStart};
use super::UsageEvent;
use serde_json::{json, Value};

const HOUR: f64 = 3600.0;
const DAY: f64 = SECONDS_PER_DAY as f64;

/// A projection needs at least this much elapsed time. Before that the burn
/// rate is one message scaled up by a thousand, which says nothing.
pub const MIN_PROJECTION_SECONDS: i64 = 3600;

/// A projected total at or above this share of the limit is "on track" (close
/// to the limit) rather than "under".
const NEAR_LIMIT: f64 = 0.8;

/// What the number measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Dollars,
    Percent,
}

/// The period a limit covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitPeriod {
    Daily,
    Weekly,
    Monthly,
    /// The Pro and Max session window.
    FiveHour,
    /// The Pro and Max weekly window.
    SevenDay,
}

impl LimitPeriod {
    pub fn key(self) -> &'static str {
        match self {
            LimitPeriod::Daily => "daily",
            LimitPeriod::Weekly => "weekly",
            LimitPeriod::Monthly => "monthly",
            LimitPeriod::FiveHour => "five_hour",
            LimitPeriod::SevenDay => "seven_day",
        }
    }

    pub fn parse(value: &str) -> Option<LimitPeriod> {
        match value {
            "daily" => Some(LimitPeriod::Daily),
            "weekly" => Some(LimitPeriod::Weekly),
            "monthly" => Some(LimitPeriod::Monthly),
            _ => None,
        }
    }

    /// The period that ends at `end` (a reset time), as `(start, end)`.
    /// Windows have a fixed length. A monthly period starts on the 1st of the
    /// month before `end`, so a leap February is 29 days long.
    pub fn ending_at(self, end: i64) -> (i64, i64) {
        let start = match self {
            LimitPeriod::Daily => end - SECONDS_PER_DAY,
            LimitPeriod::Weekly | LimitPeriod::SevenDay => end - 7 * SECONDS_PER_DAY,
            LimitPeriod::FiveHour => end - 5 * 3600,
            LimitPeriod::Monthly => {
                // `end` is a reset time chosen by the server, not a local
                // calendar boundary, so its month is found in UTC.
                let utc = crate::tz::Zone::Utc;
                utc.day_start(month_first_day(utc.day_of(end - 1)))
            }
        };
        (start, end)
    }

    /// The calendar period that holds `now`, as `(start, end)`.
    pub fn containing(self, now: i64, week: WeekStart) -> (i64, i64) {
        let today = day_of(now);
        let (first, past) = match self {
            LimitPeriod::Daily => (today, today + 1),
            LimitPeriod::Weekly | LimitPeriod::SevenDay => {
                let first = week_first_day(today, week);
                (first, first + 7)
            }
            LimitPeriod::Monthly => {
                let first = month_first_day(today);
                (first, month_first_day(first + 31))
            }
            LimitPeriod::FiveHour => {
                return (
                    now - now.rem_euclid(5 * 3600),
                    now - now.rem_euclid(5 * 3600) + 5 * 3600,
                )
            }
        };
        (crate::tz::day_start(first), crate::tz::day_start(past))
    }
}

/// Where a limit comes from. The UI says which, so a measured number is never
/// mistaken for one the user typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Read from Claude Code's own statusline input (`rate_limits`).
    Measured,
    /// Set by the user in `usage.budget.*`.
    Manual,
}

impl Origin {
    pub fn key(self) -> &'static str {
        match self {
            Origin::Measured => "measured",
            Origin::Manual => "manual",
        }
    }
}

/// One known limit with the amount used against it.
#[derive(Debug, Clone, PartialEq)]
pub struct Limit {
    pub unit: Unit,
    pub period: LimitPeriod,
    pub origin: Origin,
    /// Dollars or percent used so far in the period.
    pub used: f64,
    /// The limit in the same unit (100 for a window).
    pub limit: f64,
    /// Period bounds, UTC seconds, half open.
    pub start: i64,
    pub end: i64,
}

/// How the period is going.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum State {
    /// The limit is already reached.
    Exceeded,
    /// Not enough elapsed time (or the period is over) to project.
    Unknown,
    /// The projected total stays below 80 percent of the limit.
    Under,
    /// The projected total lands between 80 percent and the limit.
    OnTrack,
    /// At the current rate the limit is reached at this UTC second.
    WillExceed { at: i64 },
}

impl State {
    pub fn key(self) -> &'static str {
        match self {
            State::Exceeded => "exceeded",
            State::Unknown => "unknown",
            State::Under => "under",
            State::OnTrack => "on_track",
            State::WillExceed { .. } => "will_exceed",
        }
    }
}

/// The headroom of one limit at one instant. Rates are per hour and per day in
/// the limit's unit; `None` where the input has no meaning (no elapsed time, no
/// time left).
#[derive(Debug, Clone, PartialEq)]
pub struct Headroom {
    pub used: f64,
    pub limit: f64,
    pub remaining: f64,
    pub elapsed_seconds: i64,
    pub remaining_seconds: i64,
    pub avg_per_hour: Option<f64>,
    pub avg_per_day: Option<f64>,
    pub allowed_per_hour: Option<f64>,
    pub allowed_per_day: Option<f64>,
    pub projected: Option<f64>,
    pub state: State,
}

/// The headroom for `used` against `limit` over `[start, end)` at `now`. A
/// `limit` of zero or less, or an empty period, yields `None`: there is
/// nothing to measure against.
pub fn headroom(start: i64, end: i64, now: i64, used: f64, limit: f64) -> Option<Headroom> {
    if !(limit.is_finite() && limit > 0.0) || end <= start || !used.is_finite() {
        return None;
    }
    let used = used.max(0.0);
    let elapsed = (now - start).clamp(0, end - start);
    let left = (end - now).clamp(0, end - start);
    let remaining = (limit - used).max(0.0);

    let rate = (elapsed > 0).then(|| used / elapsed as f64);
    let avg_per_hour = rate.map(|r| r * HOUR);
    let avg_per_day = rate.map(|r| r * DAY);
    let allowed_per_hour = (left > 0).then(|| remaining / (left as f64 / HOUR));
    let allowed_per_day = (left > 0).then(|| remaining / (left as f64 / DAY));
    let projected = match rate {
        Some(r) if elapsed >= MIN_PROJECTION_SECONDS => Some(used + r * left as f64),
        _ => None,
    };

    let state = if used >= limit {
        State::Exceeded
    } else {
        match (projected, rate) {
            (Some(p), Some(r)) if p > limit && r > 0.0 => State::WillExceed {
                at: now + ((limit - used) / r).ceil() as i64,
            },
            (Some(p), _) if p >= limit * NEAR_LIMIT => State::OnTrack,
            (Some(_), _) => State::Under,
            (None, _) => State::Unknown,
        }
    };
    Some(Headroom {
        used,
        limit,
        remaining,
        elapsed_seconds: elapsed,
        remaining_seconds: left,
        avg_per_hour,
        avg_per_day,
        allowed_per_hour,
        allowed_per_day,
        projected,
        state,
    })
}

impl Limit {
    pub fn headroom(&self, now: i64) -> Option<Headroom> {
        headroom(self.start, self.end, now, self.used, self.limit)
    }

    /// A manual dollar budget for the calendar period holding `now`, with
    /// `events` priced and summed from the period start through `now`.
    pub fn manual_budget(
        period: LimitPeriod,
        limit_usd: f64,
        events: &[UsageEvent],
        now: i64,
        week: WeekStart,
    ) -> Limit {
        let (start, end) = period.containing(now, week);
        Limit {
            unit: Unit::Dollars,
            period,
            origin: Origin::Manual,
            used: events
                .iter()
                .filter(|e| e.timestamp >= start && e.timestamp <= now)
                .map(|e| e.cost_usd)
                .sum::<f64>()
                + 0.0,
            limit: limit_usd,
            start,
            end,
        }
    }

    pub fn to_json(&self, now: i64) -> Value {
        let h = self.headroom(now);
        let mut v = json!({
            "unit": match self.unit { Unit::Dollars => "usd", Unit::Percent => "percent" },
            "period": self.period.key(),
            "origin": self.origin.key(),
            "used": self.used,
            "limit": self.limit,
            "start": self.start,
            "end": self.end,
        });
        if let (Some(map), Some(h)) = (v.as_object_mut(), h) {
            map.insert("remaining".into(), json!(h.remaining));
            map.insert("avg_per_hour".into(), json!(h.avg_per_hour));
            map.insert("avg_per_day".into(), json!(h.avg_per_day));
            map.insert("allowed_per_hour".into(), json!(h.allowed_per_hour));
            map.insert("allowed_per_day".into(), json!(h.allowed_per_day));
            map.insert("projected".into(), json!(h.projected));
            map.insert("state".into(), json!(h.state.key()));
            if let State::WillExceed { at } = h.state {
                map.insert("exceeds_at".into(), json!(at));
            }
        }
        v
    }
}

/// Average cost per hour and per day over the elapsed part of a span.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Burn {
    pub cost_usd: f64,
    pub seconds: i64,
    pub per_hour: Option<f64>,
    pub per_day: Option<f64>,
}

impl Burn {
    /// `None` rates when no time has elapsed. The first day is partial: the
    /// daily average divides by elapsed days as a fraction, not whole days.
    pub fn new(cost_usd: f64, seconds: i64) -> Burn {
        let seconds = seconds.max(0);
        let per = |unit: f64| (seconds > 0).then(|| cost_usd / (seconds as f64 / unit));
        Burn {
            cost_usd,
            seconds,
            per_hour: per(HOUR),
            per_day: per(DAY),
        }
    }

    pub fn to_json(self) -> Value {
        json!({
            "cost_usd": self.cost_usd,
            "elapsed_seconds": self.seconds,
            "avg_per_hour": self.per_hour,
            "avg_per_day": self.per_day,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: i64 = SECONDS_PER_DAY;

    fn ev(ts: i64, cost: f64) -> UsageEvent {
        UsageEvent {
            timestamp: ts,
            cost_usd: cost,
            ..UsageEvent::default()
        }
    }

    #[test]
    fn a_month_to_date_projects_and_allows_the_rest() {
        // 10 days into a 30 day month: $100 used of $300.
        let h = headroom(0, 30 * D, 10 * D, 100.0, 300.0).unwrap();
        assert_eq!(h.avg_per_day, Some(10.0));
        assert_eq!(h.avg_per_hour, Some(100.0 / 240.0));
        assert_eq!(h.remaining, 200.0);
        assert_eq!(h.allowed_per_day, Some(10.0));
        assert_eq!(h.projected, Some(300.0));
        assert_eq!(h.state, State::OnTrack);
    }

    #[test]
    fn a_fast_burn_names_when_the_limit_is_reached() {
        // $200 in 10 days of 30, limit $300: 20 a day, 100 left lasts 5 days.
        let h = headroom(0, 30 * D, 10 * D, 200.0, 300.0).unwrap();
        assert_eq!(h.projected, Some(600.0));
        assert_eq!(h.state, State::WillExceed { at: 15 * D });
    }

    #[test]
    fn a_slow_burn_is_under() {
        let h = headroom(0, 30 * D, 10 * D, 30.0, 300.0).unwrap();
        assert_eq!(h.projected, Some(90.0));
        assert_eq!(h.state, State::Under);
    }

    #[test]
    fn zero_elapsed_time_divides_nothing() {
        let h = headroom(100, 100 + 30 * D, 100, 0.0, 300.0).unwrap();
        assert_eq!(h.avg_per_hour, None);
        assert_eq!(h.projected, None);
        assert_eq!(h.state, State::Unknown);
        assert_eq!(h.allowed_per_day, Some(10.0));
    }

    #[test]
    fn before_the_period_starts_counts_as_zero_elapsed() {
        let h = headroom(1000, 1000 + D, 500, 0.0, 10.0).unwrap();
        assert_eq!(h.elapsed_seconds, 0);
        assert_eq!(h.remaining_seconds, D);
    }

    #[test]
    fn a_projection_waits_for_an_hour_of_data() {
        let h = headroom(0, D, 600, 5.0, 10.0).unwrap();
        assert!(h.avg_per_hour.is_some());
        assert_eq!(h.projected, None);
        assert_eq!(h.state, State::Unknown);
    }

    #[test]
    fn no_time_left_allows_nothing_per_hour() {
        let h = headroom(0, D, D, 4.0, 10.0).unwrap();
        assert_eq!(h.allowed_per_hour, None);
        assert_eq!(h.allowed_per_day, None);
        assert_eq!(h.projected, Some(4.0));
        assert_eq!(h.state, State::Under);
    }

    #[test]
    fn a_limit_already_exceeded_keeps_zero_remaining() {
        let h = headroom(0, 30 * D, 10 * D, 350.0, 300.0).unwrap();
        assert_eq!(h.remaining, 0.0);
        assert_eq!(h.allowed_per_day, Some(0.0));
        assert_eq!(h.state, State::Exceeded);
        let at = headroom(0, 30 * D, 10 * D, 300.0, 300.0).unwrap();
        assert_eq!(at.state, State::Exceeded);
    }

    #[test]
    fn no_limit_or_a_bad_period_is_none() {
        assert!(headroom(0, D, 10, 1.0, 0.0).is_none());
        assert!(headroom(0, D, 10, 1.0, -5.0).is_none());
        assert!(headroom(0, D, 10, 1.0, f64::NAN).is_none());
        assert!(headroom(D, D, 10, 1.0, 5.0).is_none());
        assert!(headroom(0, D, 10, f64::INFINITY, 5.0).is_none());
    }

    #[test]
    fn zero_use_is_under_with_a_zero_rate() {
        let h = headroom(0, D, 7200, 0.0, 10.0).unwrap();
        assert_eq!(h.avg_per_hour, Some(0.0));
        assert_eq!(h.state, State::Under);
    }

    #[test]
    fn percent_windows_use_the_same_math() {
        // 5 hour window, 2 hours in, 50 percent used: 25 a hour, full at hour 4.
        let h = headroom(0, 5 * 3600, 2 * 3600, 50.0, 100.0).unwrap();
        assert_eq!(h.avg_per_hour, Some(25.0));
        assert_eq!(h.projected, Some(125.0));
        assert_eq!(h.state, State::WillExceed { at: 4 * 3600 });
    }

    // 2026-03-01 00:00 UTC is day 20513.
    const MARCH_1: i64 = 20_513 * D;

    #[test]
    fn a_calendar_month_starts_on_the_first_utc_midnight() {
        let now = MARCH_1 + 10 * D + 3600; // 2026-03-11 01:00
        let (s, e) = LimitPeriod::Monthly.containing(now, WeekStart::Monday);
        assert_eq!(s, MARCH_1);
        assert_eq!(e, MARCH_1 + 31 * D);
        // The last second of March is still March; the next second is April.
        let (s, _) = LimitPeriod::Monthly.containing(MARCH_1 + 31 * D - 1, WeekStart::Monday);
        assert_eq!(s, MARCH_1);
        let (s, e) = LimitPeriod::Monthly.containing(MARCH_1 + 31 * D, WeekStart::Monday);
        assert_eq!((s, e), (MARCH_1 + 31 * D, MARCH_1 + 61 * D));
    }

    #[test]
    fn a_leap_february_has_29_days() {
        // 2028-02-01 is day 21215.
        let feb = 21_215 * D;
        let (s, e) = LimitPeriod::Monthly.containing(feb + 5 * D, WeekStart::Monday);
        assert_eq!((s, e), (feb, feb + 29 * D));
        let (s, e) = LimitPeriod::Monthly.ending_at(feb + 29 * D);
        assert_eq!((s, e), (feb, feb + 29 * D));
    }

    #[test]
    fn a_week_starts_on_the_configured_day() {
        // 2026-03-11 is a Wednesday. Monday 03-09, Sunday 03-08, Saturday 03-07.
        let now = MARCH_1 + 10 * D;
        let monday = MARCH_1 + 8 * D;
        assert_eq!(
            LimitPeriod::Weekly.containing(now, WeekStart::Monday),
            (monday, monday + 7 * D)
        );
        assert_eq!(
            LimitPeriod::Weekly.containing(now, WeekStart::Sunday).0,
            monday - D
        );
        assert_eq!(
            LimitPeriod::Weekly.containing(now, WeekStart::Saturday).0,
            monday - 2 * D
        );
    }

    #[test]
    fn windows_end_at_their_reset_time() {
        assert_eq!(LimitPeriod::FiveHour.ending_at(100_000), (82_000, 100_000));
        assert_eq!(LimitPeriod::SevenDay.ending_at(10 * D), (3 * D, 10 * D));
        assert_eq!(LimitPeriod::Daily.ending_at(5 * D), (4 * D, 5 * D));
    }

    #[test]
    fn a_manual_budget_sums_only_this_period_up_to_now() {
        let now = MARCH_1 + 10 * D;
        let events = vec![
            ev(MARCH_1 - 1, 99.0), // February
            ev(MARCH_1, 4.0),      // first second of March
            ev(now, 6.0),          // now itself counts
            ev(now + 1, 50.0),     // the future is ignored
        ];
        let l = Limit::manual_budget(LimitPeriod::Monthly, 100.0, &events, now, WeekStart::Monday);
        assert_eq!(l.used, 10.0);
        assert_eq!(l.origin, Origin::Manual);
        assert_eq!((l.start, l.end), (MARCH_1, MARCH_1 + 31 * D));
    }

    #[test]
    fn a_partial_first_day_is_a_fraction_of_a_day() {
        // $6 in the first 6 hours: $24 a day, $1 an hour.
        let b = Burn::new(6.0, 6 * 3600);
        assert_eq!(b.per_hour, Some(1.0));
        assert_eq!(b.per_day, Some(24.0));
    }

    #[test]
    fn burn_with_no_time_has_no_rate() {
        let b = Burn::new(3.0, 0);
        assert_eq!((b.per_hour, b.per_day), (None, None));
        assert_eq!(Burn::new(3.0, -5).seconds, 0);
    }

    #[test]
    fn json_carries_the_origin_and_the_state() {
        let l = Limit {
            unit: Unit::Dollars,
            period: LimitPeriod::Monthly,
            origin: Origin::Manual,
            used: 200.0,
            limit: 300.0,
            start: 0,
            end: 30 * D,
        };
        let v = l.to_json(10 * D);
        assert_eq!(v["origin"], "manual");
        assert_eq!(v["state"], "will_exceed");
        assert_eq!(v["exceeds_at"], 15 * D);
        assert_eq!(v["remaining"], 100.0);
    }
}
