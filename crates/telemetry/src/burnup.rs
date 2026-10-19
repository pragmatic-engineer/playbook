// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The data of the burn-up chart: cumulative spend over the billing period,
//! a straight projection to the period end at the average pace, and where the
//! limit's 50, 80 and 100 percent are crossed. Pure: the period, the events
//! and the clock come in as arguments. The chart is drawn in `tui::burnup`
//! and the plain table here serves `--summary`, a pipe and a narrow panel.

use super::limits::{Crossing, Limit, LimitPeriod, Origin, MIN_PROJECTION_SECONDS};
use super::limits_source::{self, Sources};
use super::query::WeekStart;
use super::spend::dollars;
use super::tz::Zone;
use super::UsageEvent;
use serde_json::{json, Value};

const PERCENT: f64 = 100.0;

/// Cumulative spend over one billing period.
#[derive(Debug, Clone, PartialEq)]
pub struct Burnup {
    pub start: i64,
    pub end: i64,
    pub now: i64,
    /// The zone whose midnights split the period into days.
    pub zone: Zone,
    /// Cumulative spend at the end of each day, from the first day through
    /// today. Empty before the period starts. Never decreases.
    pub actual: Vec<f64>,
    /// Spend from the start through `now`.
    pub total: f64,
    /// Spend at the period end at the average pace so far, once an hour of
    /// the period has passed and the period is not over.
    pub projected_end: Option<f64>,
    pub limit: Option<f64>,
    /// What the server billed and when it was fetched.
    pub billed: Option<(i64, f64)>,
    /// 50, 80 and 100 percent of the limit, empty without one.
    pub crossings: Vec<Crossing>,
}

impl Burnup {
    /// The calendar days the period spans in its zone, the last one possibly
    /// partial.
    pub fn days(&self) -> usize {
        (self.zone.day_of(self.end - 1) - self.zone.day_of(self.start) + 1).max(0) as usize
    }

    fn first_day(&self) -> i64 {
        self.zone.day_of(self.start)
    }

    /// Spend at `at` along the straight line from now to the period end.
    pub fn projected_at(&self, at: i64) -> Option<f64> {
        let end = self.projected_end?;
        let span = (self.end - self.now) as f64;
        let share = ((at - self.now) as f64 / span).clamp(0.0, 1.0);
        Some(self.total + (end - self.total) * share)
    }

    /// The date of day `i` of the period, `YYYY-MM-DD` in its zone.
    fn day_text(&self, i: usize) -> String {
        self.zone
            .iso(self.zone.day_start(self.first_day() + i as i64))[..10]
            .to_string()
    }

    /// The share of the limit spent at the end of day `i`, in percent.
    pub fn percent_of_limit(&self, cumulative: f64) -> Option<f64> {
        self.limit
            .filter(|l| *l > 0.0)
            .map(|l| cumulative / l * PERCENT)
    }

    /// The series for `--json`: one entry per day through today.
    pub fn series_json(&self) -> Value {
        json!({
            "days": self.actual.iter().enumerate().map(|(i, c)| json!({
                "day": self.day_text(i),
                "cumulative_usd": c,
                "percent_of_limit": self.percent_of_limit(*c),
            })).collect::<Vec<_>>(),
            "projected_end_usd": self.projected_end,
        })
    }

    /// A plain table, ASCII only, one row per day through today. `rows`
    /// keeps the last rows when the room is short.
    pub fn table(&self, rows: usize) -> Vec<String> {
        let skip = self.actual.len().saturating_sub(rows);
        let mut out = vec![format!(
            "{:<10}  {:>12}  {:>8}",
            "date", "cumulative", "of limit"
        )];
        for (i, c) in self.actual.iter().enumerate().skip(skip) {
            let share = self
                .percent_of_limit(*c)
                .map_or("n/a".to_string(), |p| format!("{p:.1}%"));
            out.push(format!(
                "{:<10}  {:>12}  {share:>8}",
                self.day_text(i),
                dollars(*c)
            ));
        }
        if self.actual.is_empty() {
            out.push("the period has not started".to_string());
        }
        out
    }

    /// One line for each of the 50, 80 and 100 percent crossings.
    pub fn crossing_lines(&self) -> Vec<String> {
        self.crossings
            .iter()
            .map(limits_source::crossing_text)
            .collect()
    }
}

/// The burn-up of the period `[start, end)` at `now`.
pub fn burnup(
    events: &[UsageEvent],
    (start, end): (i64, i64),
    zone: Zone,
    now: i64,
    limit: Option<f64>,
    billed: Option<(i64, f64)>,
    crossings: Vec<Crossing>,
) -> Burnup {
    let mut b = Burnup {
        start,
        end,
        now,
        zone,
        actual: Vec::new(),
        total: 0.0,
        projected_end: None,
        limit,
        billed,
        crossings,
    };
    if now < start || end <= start {
        return b;
    }
    let first = b.first_day();
    let today = ((zone.day_of(now) - first).max(0) as usize).min(b.days().saturating_sub(1));
    let mut per_day = vec![0.0; today + 1];
    for e in events
        .iter()
        .filter(|e| e.timestamp >= start && e.timestamp <= now)
    {
        let day = ((zone.day_of(e.timestamp) - first).max(0) as usize).min(today);
        per_day[day] += e.cost_usd;
    }
    let mut cumulative = 0.0;
    b.actual = per_day
        .into_iter()
        .map(|d| {
            cumulative += d;
            cumulative
        })
        .collect();
    b.total = cumulative;
    let elapsed = now - start;
    if elapsed >= MIN_PROJECTION_SECONDS && now < end {
        b.projected_end = Some(b.total + b.total / elapsed as f64 * (end - now) as f64);
    }
    b
}

/// The burn-up for the monthly dollar limit if there is one, else for the
/// month that holds `now`, from the sources and the limits already built. A
/// period the server's reset time gave starts at a UTC midnight, so its days
/// are UTC days.
pub fn burnup_for(s: &Sources, limits: &[Limit], events: &[UsageEvent], now: i64) -> Burnup {
    let monthly = limits.iter().find(|l| limits_source::is_monthly_dollars(l));
    let (period, zone) = match monthly {
        Some(l) => {
            let from_server = l.origin == Origin::Server
                && s.server
                    .as_ref()
                    .and_then(|f| f.resets_at)
                    .is_some_and(|r| r > now);
            let zone = if from_server {
                Zone::Utc
            } else {
                crate::tz::current()
            };
            ((l.start, l.end), zone)
        }
        None => (
            LimitPeriod::Monthly.containing(now, WeekStart::default()),
            crate::tz::current(),
        ),
    };
    let billed = limits_source::server_in_use(s, now)
        .and_then(|f| f.used_usd().map(|used| (f.fetched_at, used)));
    let crossings = monthly
        .and_then(|l| l.thresholds(now))
        .map_or_else(Vec::new, |c| c.to_vec());
    burnup(
        events,
        period,
        zone,
        now,
        monthly.map(|l| l.limit),
        billed,
        crossings,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::SECONDS_PER_DAY;
    use crate::limits::{thresholds, CrossState};

    const D: i64 = SECONDS_PER_DAY;
    /// 2027-01-01 00:00 UTC.
    const JAN: i64 = 1_798_761_600;

    fn cost(ts: i64, usd: f64) -> UsageEvent {
        UsageEvent {
            timestamp: ts,
            cost_usd: usd,
            ..UsageEvent::default()
        }
    }

    fn month(days: i64, now: i64, events: &[UsageEvent]) -> Burnup {
        burnup(
            events,
            (JAN, JAN + days * D),
            Zone::Utc,
            now,
            None,
            None,
            Vec::new(),
        )
    }

    #[test]
    fn the_series_never_decreases_and_idle_days_stay_flat() {
        let events = [cost(JAN + 10, 5.0), cost(JAN + 3 * D + 10, 2.5)];

        let b = month(31, JAN + 5 * D + 100, &events);

        assert_eq!(b.actual, [5.0, 5.0, 5.0, 7.5, 7.5, 7.5]);
        assert!(b.actual.windows(2).all(|w| w[1] >= w[0]));
        assert_eq!(b.total, 7.5);
    }

    #[test]
    fn it_covers_each_day_through_today_in_months_of_28_30_and_31_days() {
        for days in [28, 30, 31] {
            let last = month(days, JAN + days * D - 1, &[]);
            let early = month(days, JAN + D, &[]);

            assert_eq!(last.days() as i64, days);
            assert_eq!(last.actual.len() as i64, days, "{days}");
            assert_eq!(early.actual.len(), 2, "{days}");
        }
    }

    #[test]
    fn an_event_after_now_or_before_the_start_is_not_counted() {
        let events = [
            cost(JAN - 1, 9.0),
            cost(JAN + D, 1.0),
            cost(JAN + 2 * D, 100.0),
        ];

        let b = month(31, JAN + D + 5, &events);

        assert_eq!(b.total, 1.0);
        assert_eq!(b.actual, [0.0, 1.0]);
    }

    #[test]
    fn a_period_that_has_not_started_has_no_series_and_no_projection() {
        let b = month(31, JAN - 1, &[cost(JAN + 5, 3.0)]);

        assert!(b.actual.is_empty() && b.projected_end.is_none());
        assert_eq!(b.total, 0.0);
        assert!(b.table(5).iter().any(|l| l.contains("has not started")));
    }

    #[test]
    fn the_projection_is_a_straight_line_at_the_average_pace_to_the_end() {
        // $10 in 10 days of a 30 day period: $30 at the end.
        let b = month(30, JAN + 10 * D, &[cost(JAN + 5, 10.0)]);

        assert_eq!(b.projected_end, Some(30.0));
        assert_eq!(b.projected_at(JAN + 10 * D), Some(10.0));
        assert_eq!(b.projected_at(JAN + 20 * D), Some(20.0));
        assert_eq!(b.projected_at(JAN + 30 * D), Some(30.0));
        assert_eq!(b.projected_at(JAN + 40 * D), Some(30.0));
    }

    #[test]
    fn there_is_no_projection_before_an_hour_or_after_the_period() {
        let early = month(30, JAN + MIN_PROJECTION_SECONDS - 1, &[cost(JAN, 1.0)]);
        let on_time = month(30, JAN + MIN_PROJECTION_SECONDS, &[cost(JAN, 1.0)]);
        let over = month(30, JAN + 31 * D, &[cost(JAN, 1.0)]);

        assert!(early.projected_end.is_none() && early.projected_at(JAN).is_none());
        assert!(on_time.projected_end.is_some());
        assert!(over.projected_end.is_none());
        assert_eq!(over.actual.len(), 30);
    }

    #[test]
    fn the_table_has_a_row_per_day_with_the_share_of_the_limit_and_the_crossings() {
        let crossings = thresholds(JAN, JAN + 30 * D, JAN + 2 * D, 50.0, 100.0)
            .unwrap()
            .to_vec();
        crate::tz::set(crate::tz::Zone::Utc);
        let b = burnup(
            &[cost(JAN + 10, 30.0), cost(JAN + D + 10, 20.0)],
            (JAN, JAN + 30 * D),
            Zone::Utc,
            JAN + 2 * D,
            Some(100.0),
            None,
            crossings,
        );

        let t = b.table(31);

        assert_eq!(t[0], "date          cumulative  of limit");
        assert_eq!(t[1], "2027-01-01        $30.00     30.0%");
        assert_eq!(t[2], "2027-01-02        $50.00     50.0%");
        assert_eq!(t[3], "2027-01-03        $50.00     50.0%");
        assert_eq!(t.len(), 4);
        let c = b.crossing_lines();
        assert!(c[0].starts_with("50% crossed about 2027-01-03"), "{}", c[0]);
        assert!(t.iter().chain(&c).all(|l| l.is_ascii()), "{t:?}");
    }

    #[test]
    fn a_short_table_keeps_the_latest_days_and_no_limit_says_n_a() {
        let b = month(30, JAN + 4 * D, &[cost(JAN, 1.0)]);

        let t = b.table(2);

        assert_eq!(t.len(), 3);
        assert!(t[1].starts_with("2027-01-04") && t[2].starts_with("2027-01-05"));
        assert!(t[1].ends_with("n/a"));
    }

    #[test]
    fn the_json_series_lists_each_day() {
        let b = burnup(
            &[cost(JAN + 10, 25.0)],
            (JAN, JAN + 30 * D),
            Zone::Utc,
            JAN + D,
            Some(100.0),
            None,
            Vec::new(),
        );

        let v = b.series_json();

        assert_eq!(v["days"].as_array().unwrap().len(), 2);
        assert_eq!(v["days"][1]["cumulative_usd"], 25.0);
        assert_eq!(v["days"][1]["percent_of_limit"], 25.0);
        assert!(v["projected_end_usd"].is_number());
    }

    fn server_limit(resets_at: Option<i64>) -> (Sources, Vec<Limit>) {
        let s = Sources {
            server: Some(crate::spend::SpendFigure {
                limit: Some(crate::spend::Money {
                    minor: 10_000,
                    decimals: 2,
                    currency: Some("USD".to_string()),
                }),
                used: Some(crate::spend::Money {
                    minor: 2_500,
                    decimals: 2,
                    currency: Some("USD".to_string()),
                }),
                percent: None,
                severity: None,
                resets_at,
                limit_reached: None,
                scope: crate::spend::Scope::Unspecified,
                windows: Vec::new(),
                fetched_at: JAN + 3 * D - 600,
                origin: Origin::Server,
            }),
            ..Sources::default()
        };
        let limits = limits_source::all_limits(&s, &[], JAN + 3 * D, WeekStart::Monday);
        (s, limits)
    }

    #[test]
    fn the_period_follows_the_monthly_limit_and_carries_the_billed_anchor() {
        crate::tz::set(crate::tz::Zone::Utc);
        let (s, limits) = server_limit(None);

        let b = burnup_for(&s, &limits, &[cost(JAN + 5, 20.0)], JAN + 3 * D);

        assert_eq!((b.start, b.end), (JAN, JAN + 31 * D));
        assert_eq!(b.limit, Some(100.0));
        assert_eq!(b.billed, Some((JAN + 3 * D - 600, 25.0)));
        assert_eq!(b.crossings.len(), 3);
        assert!(matches!(b.crossings[0].state, CrossState::Projected { .. }));
    }

    #[test]
    fn a_reset_time_from_the_server_sets_the_period_end() {
        crate::tz::set(crate::tz::Zone::Utc);
        let reset = JAN + 28 * D;
        let (s, limits) = server_limit(Some(reset));

        let b = burnup_for(&s, &limits, &[], JAN + 3 * D);

        assert_eq!(b.end, reset);
    }

    #[test]
    fn with_no_limit_the_month_that_holds_now_is_used_and_there_are_no_crossings() {
        crate::tz::set(crate::tz::Zone::Utc);

        let b = burnup_for(&Sources::default(), &[], &[cost(JAN + 5, 2.0)], JAN + 3 * D);

        assert_eq!((b.start, b.end), (JAN, JAN + 31 * D));
        assert!(b.limit.is_none() && b.crossings.is_empty() && b.billed.is_none());
        assert_eq!(b.total, 2.0);
    }

    #[test]
    fn a_period_from_the_server_labels_its_days_in_utc_whatever_the_machine_zone() {
        crate::tz::set(Zone::Fixed(-3 * 3600));
        let reset = JAN + 31 * D;
        let (s, limits) = server_limit(Some(reset));

        let b = burnup_for(&s, &limits, &[cost(JAN + 5, 20.0)], JAN + 3 * D);

        assert_eq!(b.zone, Zone::Utc);
        assert_eq!(b.table(31)[1].split_whitespace().next(), Some("2027-01-01"));
        assert_eq!(b.series_json()["days"][0]["day"], "2027-01-01");
        assert_eq!(b.days(), 31);
    }

    #[test]
    fn a_local_period_splits_days_at_local_midnights() {
        let east = Zone::Fixed(10 * 3600);
        crate::tz::set(east);
        let start = east.day_start(east.day_of(JAN) + 1);
        let events = [
            cost(start + 10, 1.0),
            cost(start + D - 10, 2.0),
            cost(start + D + 10, 4.0),
        ];

        let b = burnup(
            &events,
            (start, start + 3 * D),
            east,
            start + D + 100,
            None,
            None,
            Vec::new(),
        );

        assert_eq!(b.actual, [3.0, 7.0]);
        assert_eq!(b.table(31)[1].split_whitespace().next(), Some("2027-01-02"));
        assert_eq!(b.days(), 3);
    }
}
