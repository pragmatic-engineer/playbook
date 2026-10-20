// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The data of the burn-up chart for one limit: how much of it is used over
//! its period, a straight projection to the period end at the average pace,
//! and the lines where 50, 80 and 100 percent of the limit are crossed. A
//! dollar limit follows the local cost per day. A percent window follows the
//! readings Claude Code gave, or a straight line from the window start to the
//! latest reading when there are none: local cost is never scaled into a
//! percent, and no curve is invented. Pure: the limit, the events, the
//! readings and the clock come in as arguments. The chart is drawn in
//! `tui::burnup` and the plain table here serves `--summary`, a pipe and a
//! narrow panel.

use super::limits::{Crossing, Limit, LimitPeriod, Unit, MIN_PROJECTION_SECONDS};
use super::limits_source::{self, Sources};
use super::tz::Zone;
use super::UsageEvent;
use serde_json::{json, Value};

const PERCENT: f64 = 100.0;

/// The shares of the limit that get a cutoff line.
pub const CUTOFF_PCTS: [f64; 3] = [50.0, 80.0, 100.0];

/// Where the points of a series came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// The local cost, summed per day.
    Daily,
    /// Readings Claude Code gave, in time order.
    Readings,
    /// No readings: a straight line from the start to the latest reading.
    Line,
}

impl Basis {
    pub fn key(self) -> &'static str {
        match self {
            Basis::Daily => "daily",
            Basis::Readings => "readings",
            Basis::Line => "line",
        }
    }
}

/// How much of one limit is used over its period.
#[derive(Debug, Clone, PartialEq)]
pub struct Burnup {
    pub unit: Unit,
    pub period: LimitPeriod,
    pub start: i64,
    pub end: i64,
    pub now: i64,
    /// The zone whose midnights split a dollar period into days.
    pub zone: Zone,
    pub basis: Basis,
    /// Used over time as `(UTC second, amount)`, from the period start to
    /// `now`. Empty before the period starts. Never decreases.
    pub points: Vec<(i64, f64)>,
    /// Cumulative local cost at the end of each day, from the first day
    /// through today. Dollar limits only, empty otherwise.
    pub actual: Vec<f64>,
    /// Used at `now`, in the unit of the limit.
    pub total: f64,
    /// Used at the period end at the average pace so far, once an hour of the
    /// period has passed and the period is not over.
    pub projected_end: Option<f64>,
    /// The limit in the unit, above zero when there is one. A spend-only
    /// chart has none and never draws a zero limit.
    pub limit: Option<f64>,
    /// What Claude reported as used and when it read that, for a dollar limit.
    pub billed: Option<(i64, f64)>,
    /// 50, 80 and 100 percent of the limit.
    pub crossings: Vec<Crossing>,
    /// When Claude Code logged an HTTP 429 inside `[start, end)`, oldest
    /// first, one entry per second. The only token-free proof that a cap was
    /// hit.
    pub rate_limited: Vec<i64>,
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

    /// The cutoff lines as `(percent, amount in the unit)`.
    pub fn cutoffs(&self) -> Vec<(f64, f64)> {
        self.limit.map_or_else(Vec::new, |limit| {
            CUTOFF_PCTS
                .iter()
                .map(|p| (*p, limit * p / PERCENT))
                .collect()
        })
    }

    /// Keeps the rate limit times that fall inside `[start, end)`: a second
    /// before the start or at the end is not part of this chart.
    pub fn with_rate_limits(mut self, hits: &[i64]) -> Burnup {
        let mut inside: Vec<i64> = hits
            .iter()
            .copied()
            .filter(|t| *t >= self.start && *t < self.end)
            .collect();
        inside.sort_unstable();
        inside.dedup();
        self.rate_limited = inside;
        self
    }

    /// The line that says Claude Code was rate limited, for the newest time,
    /// or `None` when it was not.
    pub fn rate_limited_line(&self) -> Option<String> {
        let last = *self.rate_limited.iter().max()?;
        Some(limits_source::rate_limited_text(last))
    }

    /// The date of day `i` of the period, `YYYY-MM-DD` in its zone.
    fn day_text(&self, i: usize) -> String {
        self.zone
            .iso(self.zone.day_start(self.first_day() + i as i64))[..10]
            .to_string()
    }

    /// The share of the limit that `amount` is, in percent.
    pub fn percent_of_limit(&self, amount: f64) -> Option<f64> {
        self.limit.map(|limit| amount / limit * PERCENT)
    }

    /// The share of the limit as text, `n/a` without a limit.
    fn share_text(&self, amount: f64) -> String {
        self.percent_of_limit(amount)
            .map_or("n/a".to_string(), |p| format!("{p:.1}%"))
    }

    /// The series for `--json`. A dollar limit lists one entry per day through
    /// today. A percent window lists its points.
    pub fn series_json(&self) -> Value {
        match self.unit {
            Unit::Dollars => json!({
                "basis": self.basis.key(),
                "days": self.actual.iter().enumerate().map(|(i, c)| json!({
                    "day": self.day_text(i),
                    "cumulative_usd": c,
                    "percent_of_limit": self.percent_of_limit(*c),
                })).collect::<Vec<_>>(),
                "projected_end_usd": self.projected_end,
                "rate_limited_at": self.rate_limited,
            }),
            Unit::Percent => json!({
                "basis": self.basis.key(),
                "points": self.points.iter()
                    .map(|(at, p)| json!({"at": at, "percent": p}))
                    .collect::<Vec<_>>(),
                "projected_end_percent": self.projected_end,
                "rate_limited_at": self.rate_limited,
            }),
        }
    }

    /// A plain table, ASCII only. A dollar limit gets one row per day through
    /// today, a percent window one row per point. `rows` keeps the last rows
    /// when the room is short.
    pub fn table(&self, rows: usize) -> Vec<String> {
        match self.unit {
            Unit::Dollars => self.day_table(rows),
            Unit::Percent => self.point_table(rows),
        }
    }

    fn day_table(&self, rows: usize) -> Vec<String> {
        let skip = self.actual.len().saturating_sub(rows);
        let mut out = vec![format!(
            "{:<10}  {:>12}  {:>8}",
            "date", "cumulative", "of limit"
        )];
        for (i, c) in self.actual.iter().enumerate().skip(skip) {
            out.push(format!(
                "{:<10}  {:>12}  {:>8}",
                self.day_text(i),
                limits_source::amount(Unit::Dollars, *c),
                self.share_text(*c)
            ));
        }
        if self.actual.is_empty() {
            out.push("the period has not started".to_string());
        }
        out
    }

    fn point_table(&self, rows: usize) -> Vec<String> {
        let skip = self.points.len().saturating_sub(rows);
        let mut out = vec![format!("{:<16}  {:>8}", "time", "of limit")];
        for (at, p) in self.points.iter().skip(skip) {
            let time = self.zone.iso(*at)[..16].replace('T', " ");
            out.push(format!("{time:<16}  {:>8}", self.share_text(*p)));
        }
        if self.points.is_empty() {
            out.push("the period has not started".to_string());
        }
        out
    }

    /// One line for each of the 50, 80 and 100 percent crossings, then the
    /// newest 429 of the period when there is one.
    pub fn crossing_lines(&self) -> Vec<String> {
        self.crossings
            .iter()
            .map(limits_source::crossing_text)
            .chain(self.rate_limited_line())
            .collect()
    }
}

/// What a burn-up reads besides the limit.
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    pub events: &'a [UsageEvent],
    /// Percent readings `(UTC second, percent)` of a window, any order.
    pub readings: &'a [(i64, f64)],
    pub billed: Option<(i64, f64)>,
    pub now: i64,
}

/// The burn-up of `limit` at `inputs.now`, or `None` when the limit is not
/// above zero or is not a finite number.
pub fn burnup(limit: &Limit, zone: Zone, inputs: Inputs<'_>) -> Option<Burnup> {
    if !(limit.limit.is_finite() && limit.limit > 0.0) || limit.end <= limit.start {
        return None;
    }
    let mut b = Burnup {
        unit: limit.unit,
        period: limit.period,
        start: limit.start,
        end: limit.end,
        now: inputs.now,
        zone,
        basis: match limit.unit {
            Unit::Dollars => Basis::Daily,
            Unit::Percent => Basis::Line,
        },
        points: Vec::new(),
        actual: Vec::new(),
        total: 0.0,
        projected_end: None,
        limit: Some(limit.limit),
        billed: inputs.billed.filter(|_| limit.unit == Unit::Dollars),
        crossings: limit
            .thresholds(inputs.now)
            .map_or_else(Vec::new, |c| c.to_vec()),
        rate_limited: Vec::new(),
    };
    if inputs.now < limit.start {
        return Some(b);
    }
    match limit.unit {
        Unit::Dollars => b.fill_days(inputs.events),
        Unit::Percent => b.fill_readings(limit.used, inputs.readings),
    }
    b.project();
    Some(b)
}

/// The local spend over `(start, end)` with a projection and no limit, for a
/// login that has none to show. Dollars only: there are no cutoff lines and
/// no crossings.
pub fn spend_only(
    events: &[UsageEvent],
    (start, end): (i64, i64),
    zone: Zone,
    now: i64,
) -> Option<Burnup> {
    if end <= start {
        return None;
    }
    let mut b = Burnup {
        unit: Unit::Dollars,
        period: LimitPeriod::Monthly,
        start,
        end,
        now,
        zone,
        basis: Basis::Daily,
        points: Vec::new(),
        actual: Vec::new(),
        total: 0.0,
        projected_end: None,
        limit: None,
        billed: None,
        crossings: Vec::new(),
        rate_limited: Vec::new(),
    };
    if now >= start {
        b.fill_days(events);
        b.project();
    }
    Some(b)
}

impl Burnup {
    /// The straight projection to the period end at the average pace so far,
    /// once an hour of the period has passed and the period is not over.
    fn project(&mut self) {
        let elapsed = self.now - self.start;
        if elapsed >= MIN_PROJECTION_SECONDS && self.now < self.end {
            self.projected_end =
                Some(self.total + self.total / elapsed as f64 * (self.end - self.now) as f64);
        }
    }

    fn fill_days(&mut self, events: &[UsageEvent]) {
        let first = self.first_day();
        let last_day = self.days().saturating_sub(1);
        let today = ((self.zone.day_of(self.now) - first).max(0) as usize).min(last_day);
        let mut per_day = vec![0.0; today + 1];
        for e in events.iter().filter(|e| {
            e.timestamp >= self.start && e.timestamp <= self.now && e.timestamp < self.end
        }) {
            let day = ((self.zone.day_of(e.timestamp) - first).max(0) as usize).min(today);
            per_day[day] += e.cost_usd;
        }
        let mut cumulative = 0.0;
        self.actual = per_day
            .into_iter()
            .map(|d| {
                cumulative += d;
                cumulative
            })
            .collect();
        self.total = cumulative;
        self.points = vec![(self.start, 0.0)];
        for (i, c) in self.actual.iter().enumerate() {
            let day_end = self.zone.day_start(first + i as i64 + 1);
            let at = day_end.min(self.now).max(self.start);
            self.points.push((at, *c));
        }
    }

    fn fill_readings(&mut self, used: f64, readings: &[(i64, f64)]) {
        let used = used.max(0.0);
        let mut held: Vec<(i64, f64)> = readings
            .iter()
            .copied()
            .filter(|(at, p)| {
                *at > self.start && *at < self.now && *at < self.end && p.is_finite() && *p >= 0.0
            })
            .collect();
        held.sort_by_key(|(at, _)| *at);
        self.basis = if held.is_empty() {
            Basis::Line
        } else {
            Basis::Readings
        };
        self.points = vec![(self.start, 0.0)];
        let mut top = 0.0_f64;
        for (at, p) in held {
            // A window only fills up, so a lower reading is an older value
            // for the same window and the line stays where it was.
            top = top.max(p);
            self.points.push((at, top));
        }
        self.points.push((self.now.min(self.end), used.max(top)));
        self.total = used.max(top);
    }
}

/// The zone whose midnights split the period of `limit` into days: the chosen
/// zone when the period starts at its midnight, else UTC when it starts at a
/// UTC midnight (a period Claude's reset time gave), else the chosen zone.
fn period_zone(limit: &Limit) -> Zone {
    let aligned = |z: Zone| z.day_start(z.day_of(limit.start)) == limit.start;
    let chosen = crate::tz::current();
    if aligned(chosen) || !aligned(Zone::Utc) {
        chosen
    } else {
        Zone::Utc
    }
}

/// The burn-ups for the limits shown: every limit above zero, in the order
/// given. The local cost feeds dollar limits and the saved readings of
/// `Sources::history` feed windows.
pub fn burnups_for(
    s: &Sources,
    limits: &[Limit],
    events: &[UsageEvent],
    now: i64,
) -> Vec<Option<Burnup>> {
    let billed = limits_source::billed_anchor(s, now);
    limits
        .iter()
        .map(|l| {
            let zone = period_zone(l);
            let own: Vec<(i64, f64)> = s
                .history
                .iter()
                .filter(|(period, _, _)| *period == l.period)
                .map(|(_, at, p)| (*at, *p))
                .collect();
            let inputs = Inputs {
                events,
                readings: &own,
                billed: billed.filter(|_| limits_source::is_monthly_dollars(l)),
                now,
            };
            burnup(l, zone, inputs).map(|b| b.with_rate_limits(&s.rate_limited))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::SECONDS_PER_DAY;
    use crate::limits::{CrossState, Origin};

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

    fn dollars_limit(days: i64, used: f64, limit: f64) -> Limit {
        Limit {
            unit: Unit::Dollars,
            period: LimitPeriod::Monthly,
            origin: Origin::Manual,
            used,
            limit,
            start: JAN,
            end: JAN + days * D,
        }
    }

    fn window(start: i64, used: f64) -> Limit {
        Limit {
            unit: Unit::Percent,
            period: LimitPeriod::FiveHour,
            origin: Origin::Measured,
            used,
            limit: 100.0,
            start,
            end: start + 5 * 3600,
        }
    }

    fn month(days: i64, now: i64, events: &[UsageEvent]) -> Burnup {
        let l = dollars_limit(days, 0.0, 100.0);
        burnup(
            &l,
            Zone::Utc,
            Inputs {
                events,
                readings: &[],
                billed: None,
                now,
            },
        )
        .unwrap()
    }

    fn percent(l: &Limit, now: i64, readings: &[(i64, f64)]) -> Burnup {
        burnup(
            l,
            Zone::Utc,
            Inputs {
                events: &[],
                readings,
                billed: None,
                now,
            },
        )
        .unwrap()
    }

    fn history_burnup(name: &str, file: &[u8]) -> Burnup {
        let dir =
            std::env::temp_dir().join(format!("pb-burnup-hist-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(crate::limits_history::history_path(&dir), file).unwrap();
        let mut s = Sources::new(Some(&dir), &dir.join("none.json"), Vec::new(), false);
        s.history = crate::limits_history::load(&dir);
        let l = window(JAN, 40.0);
        burnups_for(&s, &[l], &[], JAN + 4_000).remove(0).unwrap()
    }

    const HISTORY: &str = concat!(
        "{\"t\":1798761500,\"five_hour\":90,\"seven_day\":5}\n",
        "{\"t\":1798762200,\"five_hour\":10,\"seven_day\":6}\n",
        "not a line\n",
        "{\"t\":1798763100,\"five_hour\":25,\"seven_day\":null}\n",
        "{\"t\":1798763700,\"five_hour\":15}\n",
        "{\"t\":1798764600,\"five_hour\":40}\n",
    );

    #[test]
    fn a_history_file_gives_the_window_its_measured_points() {
        let b = history_burnup("good", HISTORY.as_bytes());

        assert_eq!(b.basis, Basis::Readings);
        assert_eq!(
            b.points,
            vec![
                (JAN, 0.0),
                (JAN + 600, 10.0),
                (JAN + 1_500, 25.0),
                (JAN + 2_100, 25.0),
                (JAN + 3_000, 40.0),
                (JAN + 4_000, 40.0)
            ]
        );
        assert!(b.points.windows(2).all(|w| w[0].1 <= w[1].1));
    }

    #[test]
    fn a_corrupt_or_oversized_history_falls_back_to_the_two_point_line() {
        let big = HISTORY.repeat(crate::limits_history::MAX_BYTES as usize / HISTORY.len() + 2);
        for (name, file) in [
            ("garbage", b"\x00\xff not json\n{".to_vec()),
            ("big", big.into_bytes()),
        ] {
            let b = history_burnup(name, &file);

            assert_eq!(b.basis, Basis::Line, "{name}");
            assert_eq!(b.points, vec![(JAN, 0.0), (JAN + 4_000, 40.0)], "{name}");
        }
    }

    #[test]
    fn only_rate_limit_times_inside_the_period_are_kept_oldest_first() {
        let l = window(JAN, 40.0);
        let end = l.end;
        let b = percent(&l, JAN + 4_000, &[]);

        let b = b.with_rate_limits(&[end, end - 1, JAN + 9, JAN, JAN - 1, JAN + 9]);

        assert_eq!(b.rate_limited, vec![JAN, JAN + 9, end - 1]);
        assert_eq!(
            b.series_json()["rate_limited_at"],
            json!([JAN, JAN + 9, end - 1])
        );
        assert_eq!(
            b.rate_limited_line(),
            Some(limits_source::rate_limited_text(end - 1))
        );
        let none = percent(&l, JAN + 4_000, &[]).with_rate_limits(&[]);
        assert!(none.rate_limited.is_empty());
        assert_eq!(none.rate_limited_line(), None);
        assert_eq!(none.crossing_lines().len(), 3);
        assert_eq!(b.crossing_lines().len(), 4);
    }

    #[test]
    fn the_series_never_decreases_and_idle_days_stay_flat() {
        let events = [cost(JAN + 10, 5.0), cost(JAN + 3 * D + 10, 2.5)];

        let b = month(31, JAN + 5 * D + 100, &events);

        assert_eq!(b.actual, [5.0, 5.0, 5.0, 7.5, 7.5, 7.5]);
        assert!(b.actual.windows(2).all(|w| w[1] >= w[0]));
        assert_eq!(b.total, 7.5);
        assert_eq!(b.points.first(), Some(&(JAN, 0.0)));
        assert_eq!(b.points.last(), Some(&(JAN + 5 * D + 100, 7.5)));
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

        assert!(b.actual.is_empty() && b.points.is_empty() && b.projected_end.is_none());
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
    fn nothing_used_at_an_hour_is_a_zero_pace_with_every_threshold_beyond() {
        let l = dollars_limit(30, 0.0, 100.0);
        let b = burnup(
            &l,
            Zone::Utc,
            Inputs {
                events: &[],
                readings: &[],
                billed: None,
                now: JAN + MIN_PROJECTION_SECONDS,
            },
        )
        .unwrap();

        assert_eq!(b.projected_end, Some(0.0));
        assert!(b.crossings.iter().all(|c| c.state
            == CrossState::Beyond {
                period_end: JAN + 30 * D
            }));
    }

    #[test]
    fn a_spend_only_chart_has_a_projection_and_no_limit_lines() {
        let b = spend_only(
            &[cost(JAN + 5, 10.0)],
            (JAN, JAN + 30 * D),
            Zone::Utc,
            JAN + 10 * D,
        )
        .unwrap();

        assert_eq!(b.limit, None);
        assert_eq!(b.projected_end, Some(30.0));
        assert!(b.cutoffs().is_empty() && b.crossings.is_empty());
        assert!(b.table(5)[1].ends_with("n/a"));
        assert_eq!(b.series_json()["days"][0]["percent_of_limit"], Value::Null);
        assert!(spend_only(&[], (JAN, JAN), Zone::Utc, JAN).is_none());
    }

    #[test]
    fn a_limit_of_zero_or_less_or_not_a_number_has_no_chart() {
        for limit in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            let l = dollars_limit(30, 1.0, limit);
            let b = burnup(
                &l,
                Zone::Utc,
                Inputs {
                    events: &[],
                    readings: &[],
                    billed: None,
                    now: JAN + D,
                },
            );

            assert!(b.is_none(), "{limit}");
        }
    }

    #[test]
    fn a_monthly_budget_of_200_with_50_spent_has_cutoffs_at_100_160_and_200() {
        let l = dollars_limit(30, 50.0, 200.0);
        let b = burnup(
            &l,
            Zone::Utc,
            Inputs {
                events: &[cost(JAN + 5, 50.0)],
                readings: &[],
                billed: None,
                now: JAN + 10 * D,
            },
        )
        .unwrap();

        assert_eq!(b.cutoffs(), [(50.0, 100.0), (80.0, 160.0), (100.0, 200.0)]);
        assert_eq!(b.total, 50.0);
    }

    #[test]
    fn a_five_hour_window_at_12_percent_after_4320_seconds_projects_50_percent() {
        let l = window(1000, 12.0);

        let b = percent(&l, 1000 + 4320, &[]);

        assert_eq!(b.basis, Basis::Line);
        assert_eq!(b.points, [(1000, 0.0), (5320, 12.0)]);
        assert!((b.projected_end.unwrap() - 50.0).abs() < 1e-9);
        // 12 percent in 4320 s reaches 50 exactly at the window end, which is
        // beyond the half open window.
        assert_eq!(
            b.crossings[0].state,
            CrossState::Beyond { period_end: 19_000 }
        );
        assert_eq!(b.cutoffs(), [(50.0, 50.0), (80.0, 80.0), (100.0, 100.0)]);
    }

    #[test]
    fn readings_make_the_series_and_a_lower_reading_never_pulls_it_down() {
        let l = window(1000, 30.0);
        let readings = [(5000, 20.0), (3000, 10.0), (7000, 15.0), (9000, 25.0)];

        let b = percent(&l, 10_000, &readings);

        assert_eq!(b.basis, Basis::Readings);
        assert_eq!(
            b.points,
            [
                (1000, 0.0),
                (3000, 10.0),
                (5000, 20.0),
                (7000, 20.0),
                (9000, 25.0),
                (10_000, 30.0)
            ]
        );
        assert!(b
            .points
            .windows(2)
            .all(|w| w[1].1 >= w[0].1 && w[1].0 >= w[0].0));
    }

    #[test]
    fn readings_outside_the_window_or_not_numbers_are_ignored() {
        let l = window(1000, 10.0);
        let readings = [
            (999, 90.0),
            (1000, 50.0),
            (6000, f64::NAN),
            (6000, -1.0),
            (20_000, 99.0),
        ];

        let b = percent(&l, 10_000, &readings);

        assert_eq!(b.basis, Basis::Line);
        assert_eq!(b.points, [(1000, 0.0), (10_000, 10.0)]);
    }

    #[test]
    fn a_reading_above_the_latest_used_amount_is_the_newer_value_and_is_kept() {
        let l = window(1000, 10.0);

        let b = percent(&l, 10_000, &[(5000, 40.0)]);

        assert_eq!(b.points.last(), Some(&(10_000, 40.0)));
        assert_eq!(b.total, 40.0);
    }

    #[test]
    fn a_reading_after_the_window_end_or_an_event_after_it_is_not_counted() {
        let l = window(1000, 10.0);
        let b = percent(&l, 30_000, &[(19_500, 90.0)]);
        let over = month(
            30,
            JAN + 31 * D,
            &[cost(JAN, 1.0), cost(JAN + 30 * D + 5, 50.0)],
        );

        assert_eq!(b.points, [(1000, 0.0), (19_000, 10.0)]);
        assert_eq!(over.total, 1.0);
    }

    #[test]
    fn a_period_that_starts_at_a_utc_midnight_is_split_in_utc_whatever_the_chosen_zone() {
        let mut l = dollars_limit(30, 0.0, 100.0);
        l.start = JAN;

        // The chosen zone is UTC in unit tests, so both agree here.
        assert_eq!(period_zone(&l), Zone::Utc);
        l.start = JAN + 15 * 3600;
        // Not a UTC midnight and not aligned to the chosen zone: the chosen
        // zone is kept.
        assert_eq!(period_zone(&l), crate::tz::current());
    }

    #[test]
    fn a_window_that_has_not_started_has_no_points() {
        let b = percent(&window(5000, 10.0), 4000, &[]);

        assert!(b.points.is_empty() && b.projected_end.is_none());
    }

    #[test]
    fn the_series_is_non_decreasing_for_generated_inputs() {
        // A small deterministic generator in place of a property test crate.
        let mut seed = 12_345_u64;
        let mut next = move |modulo: u64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 33) % modulo
        };
        for _ in 0..200 {
            let events: Vec<UsageEvent> = (0..next(40))
                .map(|_| cost(JAN + next(31 * D as u64) as i64, next(1000) as f64 / 10.0))
                .collect();
            let readings: Vec<(i64, f64)> = (0..next(30))
                .map(|_| (1000 + next(18_000) as i64, next(120) as f64))
                .collect();
            let now = JAN + next(32 * D as u64) as i64;

            let d = month(31, now, &events);
            let p = percent(
                &window(1000, next(100) as f64),
                1000 + next(18_000) as i64,
                &readings,
            );

            for b in [d, p] {
                assert!(b.points.windows(2).all(|w| w[1].1 >= w[0].1));
                assert!(b.actual.windows(2).all(|w| w[1] >= w[0]));
            }
        }
    }

    #[test]
    fn the_table_has_a_row_per_day_with_the_share_of_the_limit_and_the_crossings() {
        crate::tz::set(crate::tz::Zone::Utc);
        let l = dollars_limit(30, 50.0, 100.0);
        let b = burnup(
            &l,
            Zone::Utc,
            Inputs {
                events: &[cost(JAN + 10, 30.0), cost(JAN + D + 10, 20.0)],
                readings: &[],
                billed: None,
                now: JAN + 2 * D,
            },
        )
        .unwrap();

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
    fn a_short_table_keeps_the_latest_days_and_a_percent_table_lists_its_points() {
        let b = month(30, JAN + 4 * D, &[cost(JAN, 1.0)]);
        let w = percent(&window(JAN, 10.0), JAN + 3600, &[]);

        let t = b.table(2);
        let p = w.table(5);

        assert_eq!(t.len(), 3);
        assert!(t[1].starts_with("2027-01-04") && t[2].starts_with("2027-01-05"));
        assert_eq!(p[0], "time              of limit");
        assert_eq!(p[1], "2027-01-01 00:00      0.0%");
        assert_eq!(p[2], "2027-01-01 01:00     10.0%");
    }

    #[test]
    fn the_json_series_lists_each_day_or_each_point() {
        let l = dollars_limit(30, 25.0, 100.0);
        let d = burnup(
            &l,
            Zone::Utc,
            Inputs {
                events: &[cost(JAN + 10, 25.0)],
                readings: &[],
                billed: None,
                now: JAN + D,
            },
        )
        .unwrap()
        .series_json();
        let p = percent(&window(1000, 12.0), 5320, &[]).series_json();

        assert_eq!(d["basis"], "daily");
        assert_eq!(d["days"].as_array().unwrap().len(), 2);
        assert_eq!(d["days"][1]["cumulative_usd"], 25.0);
        assert_eq!(d["days"][1]["percent_of_limit"], 25.0);
        assert!(d["projected_end_usd"].is_number());
        assert_eq!(p["basis"], "line");
        assert_eq!(p["points"][1]["percent"], 12.0);
        assert_eq!(p["points"][1]["at"], 5320);
    }
}
