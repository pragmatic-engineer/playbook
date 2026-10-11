// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The period comparison: today, this week and this month, each against the
//! one before it over the same elapsed span, so a Thursday is never compared
//! with a whole week. Every boundary is a UTC second, so no local time zone or
//! daylight saving enters.

use super::aggregate::{Group, SECONDS_PER_DAY};
use super::query::{day_of, month_first_day, week_first_day, Range, Totals, WeekStart};
use super::UsageEvent;
use std::collections::BTreeMap;

/// A calendar period that is compared with the one before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Today,
    Week(WeekStart),
    /// Month to date.
    Month,
}

impl Period {
    /// `today`, `week` or `month`, with the configured first day of the week.
    pub fn parse(value: &str, week: WeekStart) -> Option<Period> {
        match value {
            "today" => Some(Period::Today),
            "week" => Some(Period::Week(week)),
            "month" => Some(Period::Month),
            _ => None,
        }
    }

    /// The period a range is the current whole of, if it is one.
    pub fn of_range(range: Range) -> Option<Period> {
        match range {
            Range::Today => Some(Period::Today),
            Range::ThisWeek(w) => Some(Period::Week(w)),
            Range::CurrentMonth => Some(Period::Month),
            _ => None,
        }
    }

    /// The whole current period as a range, including days still to come.
    pub fn range(self) -> Range {
        match self {
            Period::Today => Range::Today,
            Period::Week(w) => Range::ThisWeek(w),
            Period::Month => Range::CurrentMonth,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Period::Today => "today",
            Period::Week(_) => "week",
            Period::Month => "month",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Period::Today => "today",
            Period::Week(_) => "this week",
            Period::Month => "this month",
        }
    }

    pub fn previous_label(self) -> &'static str {
        match self {
            Period::Today => "yesterday",
            Period::Week(_) => "last week",
            Period::Month => "last month",
        }
    }

    /// The same period with weeks starting on `week`.
    pub fn with_week_start(self, week: WeekStart) -> Period {
        match self {
            Period::Week(_) => Period::Week(week),
            other => other,
        }
    }

    /// The current period so far and the previous one over the same elapsed
    /// span. The current window runs from the period start through the second
    /// of `now`. The previous one starts at its own start and lasts as long,
    /// but never runs past its period, so on the 31st a 30-day month is
    /// compared whole.
    pub fn windows(self, now: i64) -> Windows {
        let today = day_of(now);
        let (first, previous_first) = match self {
            Period::Today => (today, today - 1),
            Period::Week(w) => {
                let first = week_first_day(today, w);
                (first, first - 7)
            }
            Period::Month => {
                let first = month_first_day(today);
                (first, month_first_day(first - 1))
            }
        };
        let current = Span {
            start: first * SECONDS_PER_DAY,
            end: now + 1,
        };
        let previous_start = previous_first * SECONDS_PER_DAY;
        let previous = Span {
            start: previous_start,
            end: (previous_start + current.seconds()).min(current.start),
        };
        Windows { current, previous }
    }
}

/// A half open span of UTC seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: i64,
    pub end: i64,
}

impl Span {
    pub fn seconds(self) -> i64 {
        self.end - self.start
    }

    pub fn contains(self, timestamp: i64) -> bool {
        (self.start..self.end).contains(&timestamp)
    }
}

/// The two spans a period is compared over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Windows {
    pub current: Span,
    pub previous: Span,
}

impl Windows {
    /// The events of each span, current first.
    pub fn split(&self, events: &[UsageEvent]) -> (Vec<UsageEvent>, Vec<UsageEvent>) {
        let pick = |span: Span| {
            events
                .iter()
                .filter(|e| span.contains(e.timestamp))
                .cloned()
                .collect()
        };
        (pick(self.current), pick(self.previous))
    }
}

/// How long a span is, in its two largest units: `3d 1h`, `5h 12m`, `42m`.
pub fn elapsed_text(seconds: i64) -> String {
    let s = seconds.max(0);
    let (d, h, m) = (
        s / SECONDS_PER_DAY,
        s % SECONDS_PER_DAY / 3600,
        s % 3600 / 60,
    );
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

/// The difference of a value against the one before it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Change {
    pub amount: f64,
    /// `None` when the previous value was zero, where a percent is undefined.
    pub percent: Option<f64>,
}

impl Change {
    pub fn of(current: f64, previous: f64) -> Change {
        Change {
            amount: current - previous,
            percent: (previous != 0.0).then(|| (current - previous) / previous * 100.0),
        }
    }
}

/// A period and the one before it, as totals.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub period: Period,
    pub windows: Windows,
    pub current: Totals,
    pub previous: Totals,
}

impl Comparison {
    /// Compares the events of the two spans of `windows`.
    pub fn new(
        period: Period,
        windows: Windows,
        current: &[UsageEvent],
        previous: &[UsageEvent],
    ) -> Comparison {
        Comparison {
            period,
            windows,
            current: Totals::of(current),
            previous: Totals::of(previous),
        }
    }

    /// Compares `period` at `now` over `events`, which may hold any span.
    pub fn of(events: &[UsageEvent], now: i64, period: Period) -> Comparison {
        let windows = period.windows(now);
        let (current, previous) = windows.split(events);
        Comparison::new(period, windows, &current, &previous)
    }

    pub fn cost(&self) -> Change {
        Change::of(self.current.cost_usd, self.previous.cost_usd)
    }

    pub fn tokens(&self) -> Change {
        Change::of(self.current.tokens() as f64, self.previous.tokens() as f64)
    }

    pub fn messages(&self) -> Change {
        Change::of(self.current.messages as f64, self.previous.messages as f64)
    }
}

/// One key with its group in the current span and in the previous one, either
/// of which is empty when the key did not appear there.
#[derive(Debug, Clone, PartialEq)]
pub struct Paired {
    pub key: String,
    pub current: Group,
    pub previous: Group,
}

/// Joins two group lists by key, in key order.
pub fn pair_groups(current: &[Group], previous: &[Group]) -> Vec<Paired> {
    let mut by_key: BTreeMap<&str, (Option<&Group>, Option<&Group>)> = BTreeMap::new();
    for g in current {
        by_key.entry(&g.key).or_default().0 = Some(g);
    }
    for g in previous {
        by_key.entry(&g.key).or_default().1 = Some(g);
    }
    by_key
        .into_iter()
        .map(|(key, (c, p))| Paired {
            key: key.to_string(),
            current: c.cloned().unwrap_or_default(),
            previous: p.cloned().unwrap_or_default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::aggregate::date_key;

    const MON: WeekStart = WeekStart::Monday;
    const DAY: i64 = SECONDS_PER_DAY;
    // Thursday 2026-09-03 12:00:00 UTC.
    const THU_NOON: i64 = 1_788_436_800;
    const THU_MIDNIGHT: i64 = THU_NOON - 12 * 3600;
    // 2028-03-30 (after a leap February) and 2027-03-31 12:00:00 UTC.
    const MAR_30_2028_NOON: i64 = 1_835_481_600 + 29 * DAY + 12 * 3600;
    const MAR_31_2027_NOON: i64 = 1_803_859_200 + 30 * DAY + 12 * 3600;

    fn dates(span: Span) -> (String, String) {
        (date_key(day_of(span.start)), date_key(day_of(span.end - 1)))
    }

    fn event(ts: i64, cost: f64) -> UsageEvent {
        UsageEvent {
            event_id: ts.to_string(),
            timestamp: ts,
            input_tokens: 10,
            output_tokens: 5,
            cost_usd: cost,
            ..UsageEvent::default()
        }
    }

    #[test]
    fn today_is_compared_with_yesterday_up_to_the_same_time_of_day() {
        let w = Period::Today.windows(THU_NOON);
        assert_eq!(
            w.current,
            Span {
                start: THU_MIDNIGHT,
                end: THU_NOON + 1
            }
        );
        assert_eq!(
            w.previous,
            Span {
                start: THU_MIDNIGHT - DAY,
                end: THU_NOON + 1 - DAY
            }
        );
        assert_eq!(w.current.seconds(), w.previous.seconds());
    }

    #[test]
    fn the_week_runs_from_its_first_day_and_the_previous_week_matches_its_length() {
        let w = Period::Week(MON).windows(THU_NOON);
        assert_eq!(dates(w.current), ("2026-08-31".into(), "2026-09-03".into()));
        assert_eq!(
            dates(w.previous),
            ("2026-08-24".into(), "2026-08-27".into())
        );
        assert_eq!(w.current.seconds(), w.previous.seconds());
        // 3 days and 12 hours so far.
        assert_eq!(elapsed_text(w.current.seconds()), "3d 12h");
    }

    #[test]
    fn the_configured_week_start_moves_both_weeks_in_utc() {
        let sun = Period::Week(WeekStart::Sunday).windows(THU_NOON);
        assert_eq!(
            dates(sun.current),
            ("2026-08-30".into(), "2026-09-03".into())
        );
        assert_eq!(
            dates(sun.previous),
            ("2026-08-23".into(), "2026-08-27".into())
        );
        let sat = Period::Week(WeekStart::Saturday).windows(THU_NOON);
        assert_eq!(
            dates(sat.current),
            ("2026-08-29".into(), "2026-09-03".into())
        );
        // On the first day itself the week has just begun.
        let sunday_noon = THU_NOON + 3 * DAY;
        let w = Period::Week(WeekStart::Sunday).windows(sunday_noon);
        assert_eq!(dates(w.current), ("2026-09-06".into(), "2026-09-06".into()));
        assert_eq!(
            dates(w.previous),
            ("2026-08-30".into(), "2026-08-30".into())
        );
    }

    #[test]
    fn the_period_turns_over_at_midnight_utc() {
        let last_second = THU_MIDNIGHT - 1; // Wednesday 23:59:59
        let w = Period::Today.windows(last_second);
        assert_eq!(dates(w.current), ("2026-09-02".into(), "2026-09-02".into()));
        assert_eq!(w.current.seconds(), DAY);
        // One second later it is a new day with one second elapsed.
        let w = Period::Today.windows(THU_MIDNIGHT);
        assert_eq!(dates(w.current), ("2026-09-03".into(), "2026-09-03".into()));
        assert_eq!(w.current.seconds(), 1);
        assert_eq!(w.previous.seconds(), 1);
        // The week turns over at midnight on its first day.
        let monday = THU_MIDNIGHT + 4 * DAY;
        let before = Period::Week(MON).windows(monday - 1);
        let after = Period::Week(MON).windows(monday);
        assert_eq!(dates(before.current).0, "2026-08-31");
        assert_eq!(
            dates(after.current),
            ("2026-09-07".into(), "2026-09-07".into())
        );
        assert_eq!(
            dates(after.previous),
            ("2026-08-31".into(), "2026-08-31".into())
        );
    }

    #[test]
    fn an_event_one_second_either_side_of_midnight_lands_in_the_right_day() {
        let events = [event(THU_MIDNIGHT - 1, 1.0), event(THU_MIDNIGHT, 2.0)];
        let c = Comparison::of(&events, THU_NOON, Period::Today);
        assert_eq!((c.current.cost_usd, c.previous.cost_usd), (2.0, 0.0));
        // Yesterday's last second is outside yesterday's same-span window
        // (which ends at noon), so it is not counted against today either.
        let c = Comparison::of(&events, THU_NOON + DAY, Period::Today);
        assert_eq!((c.current.cost_usd, c.previous.cost_usd), (0.0, 2.0));
    }

    #[test]
    fn the_month_is_month_to_date_against_the_same_span_of_last_month() {
        let w = Period::Month.windows(THU_NOON);
        assert_eq!(dates(w.current), ("2026-09-01".into(), "2026-09-03".into()));
        assert_eq!(
            dates(w.previous),
            ("2026-08-01".into(), "2026-08-03".into())
        );
        // From January the previous month is December of the year before.
        let jan = 1_800_014_400; // 2027-01-15 12:00 UTC
        let w = Period::Month.windows(jan);
        assert_eq!(
            dates(w.previous),
            ("2026-12-01".into(), "2026-12-15".into())
        );
    }

    #[test]
    fn a_shorter_previous_month_is_compared_whole_and_a_leap_february_is_29_days() {
        // March 31 against February 2027 (28 days): all of February.
        let w = Period::Month.windows(MAR_31_2027_NOON);
        assert_eq!(dates(w.current), ("2027-03-01".into(), "2027-03-31".into()));
        assert_eq!(
            dates(w.previous),
            ("2027-02-01".into(), "2027-02-28".into())
        );
        assert!(w.previous.seconds() < w.current.seconds());
        // March 30 2028 against a leap February: all 29 days.
        let w = Period::Month.windows(MAR_30_2028_NOON);
        assert_eq!(
            dates(w.previous),
            ("2028-02-01".into(), "2028-02-29".into())
        );
        assert_eq!(w.previous.end, w.current.start);
        // The day before, the same span ends part way through the 29th.
        let w = Period::Month.windows(MAR_30_2028_NOON - DAY);
        assert_eq!(
            dates(w.previous),
            ("2028-02-01".into(), "2028-02-29".into())
        );
        assert!(w.previous.end < w.current.start);
    }

    #[test]
    fn the_first_of_a_long_month_after_a_short_one_keeps_equal_spans() {
        // May 1 12:00 against April (30 days): spans are equal.
        let may_1 = 1_777_593_600 + 12 * 3600; // 2026-05-01 12:00 UTC
        let w = Period::Month.windows(may_1);
        assert_eq!(dates(w.current), ("2026-05-01".into(), "2026-05-01".into()));
        assert_eq!(
            dates(w.previous),
            ("2026-04-01".into(), "2026-04-01".into())
        );
        assert_eq!(w.current.seconds(), w.previous.seconds());
    }

    #[test]
    fn a_zero_previous_period_has_no_percent_and_never_divides_by_zero() {
        let c = Change::of(5.0, 0.0);
        assert_eq!(c.amount, 5.0);
        assert_eq!(c.percent, None);
        let c = Change::of(0.0, 0.0);
        assert_eq!((c.amount, c.percent), (0.0, None));
        let c = Change::of(0.0, 4.0);
        assert_eq!((c.amount, c.percent), (-4.0, Some(-100.0)));
        let c = Change::of(6.0, 4.0);
        assert_eq!(c.percent, Some(50.0));
    }

    #[test]
    fn the_comparison_sums_each_span_and_reports_every_change() {
        let events = [
            event(THU_NOON - 60, 3.0),
            event(THU_MIDNIGHT + 10, 1.0),
            event(THU_NOON - DAY - 60, 2.0),
            // Later than the same time yesterday, so it is left out.
            event(THU_NOON - DAY + 600, 50.0),
            // Two weeks back is outside both spans.
            event(THU_NOON - 14 * DAY, 99.0),
        ];
        let c = Comparison::of(&events, THU_NOON, Period::Today);
        assert_eq!(c.current.messages, 2);
        assert_eq!(c.previous.messages, 1);
        assert_eq!(c.cost().amount, 2.0);
        assert_eq!(c.cost().percent, Some(100.0));
        assert_eq!(c.tokens().amount, 15.0);
        assert_eq!(c.messages().amount, 1.0);
    }

    #[test]
    fn pairing_joins_by_key_and_fills_the_missing_side_with_zero() {
        let g = |key: &str, cost: f64| Group {
            key: key.into(),
            messages: 1,
            cost_usd: cost,
            ..Group::default()
        };
        let pairs = pair_groups(&[g("b", 2.0), g("a", 1.0)], &[g("c", 9.0), g("b", 1.0)]);
        let keys: Vec<&str> = pairs.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["a", "b", "c"]);
        assert_eq!(pairs[0].previous.messages, 0);
        assert_eq!(pairs[1].current.cost_usd, 2.0);
        assert_eq!(pairs[1].previous.cost_usd, 1.0);
        assert_eq!(pairs[2].current.messages, 0);
    }

    #[test]
    fn names_parse_and_map_to_ranges() {
        for (key, period) in [
            ("today", Period::Today),
            ("week", Period::Week(WeekStart::Sunday)),
            ("month", Period::Month),
        ] {
            let p = Period::parse(key, WeekStart::Sunday).unwrap();
            assert_eq!(p, period);
            assert_eq!(p.key(), key);
            assert_eq!(Period::of_range(p.range()), Some(p));
        }
        assert_eq!(Period::parse("30d", MON), None);
        assert_eq!(Period::of_range(Range::LastWeek(MON)), None);
        assert_eq!(
            Period::Week(MON).with_week_start(WeekStart::Saturday),
            Period::Week(WeekStart::Saturday)
        );
        assert_eq!(
            Period::Month.with_week_start(WeekStart::Saturday),
            Period::Month
        );
    }

    #[test]
    fn elapsed_time_reads_in_its_two_largest_units() {
        assert_eq!(elapsed_text(42 * 60 + 5), "42m");
        assert_eq!(elapsed_text(5 * 3600 + 12 * 60), "5h 12m");
        assert_eq!(elapsed_text(3 * DAY + 3600 + 59), "3d 1h");
        assert_eq!(elapsed_text(-5), "0m");
    }
}
