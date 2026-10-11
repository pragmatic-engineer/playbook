// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The terminal view and the web JSON read one query layer. These tests pin
//! that: for the same events and range, both report the same totals and
//! groups, so the two views cannot drift apart.

use super::app::ranges;
use super::data::Data;
use crate::usage::aggregate::date_key;
use crate::usage::api::{comparison_json, data_json};
use crate::usage::query::WeekStart;
use crate::usage::query::{LiveView, Range};
use crate::usage::UsageEvent;

const NOW: i64 = 1_788_400_000;
const DAY: i64 = 86_400;

fn event(age_days: i64, model: &str, repo: &str, cost: f64, tokens: u64) -> UsageEvent {
    UsageEvent {
        event_id: format!("{age_days}{model}{repo}{tokens}"),
        timestamp: NOW - age_days * DAY - 30,
        session_id: "s1".into(),
        model: model.into(),
        repo: repo.into(),
        input_tokens: tokens,
        output_tokens: tokens / 2,
        cache_read_tokens: tokens / 4,
        cost_usd: cost,
        ..UsageEvent::default()
    }
}

fn events() -> Vec<UsageEvent> {
    vec![
        event(0, "claude-opus-5", "alpha", 4.0, 4_000),
        event(1, "claude-sonnet-5", "beta", 1.5, 2_000),
        event(12, "claude-opus-5", "beta", 7.25, 9_000),
        event(40, "claude-sonnet-5", "alpha", 2.0, 3_000),
        event(75, "claude-opus-5", "alpha", 11.0, 12_000),
        event(200, "claude-haiku-5", "gamma", 0.5, 500),
    ]
}

fn group_rows(doc: &serde_json::Value, dim: &str) -> Vec<(String, u64, f64)> {
    doc["groups"][dim]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            (
                g["key"].as_str().unwrap().to_string(),
                g["messages"].as_u64().unwrap(),
                g["cost_usd"].as_f64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn totals_match_the_web_json_for_every_range() {
    let events = events();
    for range in ranges(WeekStart::Monday) {
        let web = data_json(&events, &[], NOW, range);
        let tui = Data::build(&events, LiveView::default(), NOW, range, "");
        let t = &web["totals"];
        assert_eq!(tui.totals.messages, t["messages"], "{range:?}");
        assert_eq!(tui.totals.input_tokens, t["input_tokens"], "{range:?}");
        assert_eq!(tui.totals.output_tokens, t["output_tokens"], "{range:?}");
        assert_eq!(
            tui.totals.cache_read_tokens, t["cache_read_tokens"],
            "{range:?}"
        );
        let cost = t["cost_usd"].as_f64().unwrap();
        assert!((tui.totals.cost_usd - cost).abs() < 1e-9, "{range:?}");
    }
}

#[test]
fn model_repo_and_day_groups_match_the_web_json_for_every_range() {
    let events = events();
    for range in ranges(WeekStart::Monday) {
        let web = data_json(&events, &[], NOW, range);
        let tui = Data::build(&events, LiveView::default(), NOW, range, "");
        for (dim, groups) in [
            ("model", &tui.models),
            ("repo", &tui.repos),
            ("day", &tui.days),
        ] {
            let ours: Vec<(String, u64, f64)> = groups
                .iter()
                .map(|g| (g.key.clone(), g.messages, g.cost_usd))
                .collect();
            assert_eq!(ours, group_rows(&web, dim), "{dim} for {range:?}");
        }
    }
}

#[test]
fn the_graph_series_sums_to_the_same_cost_as_the_totals() {
    let events = events();
    for range in ranges(WeekStart::Monday) {
        let tui = Data::build(&events, LiveView::default(), NOW, range, "");
        // Today charts seven days, so only its last bar is the total.
        let graphed: f64 = match range {
            Range::Today => tui.series.last().map_or(0.0, |p| p.cost_usd),
            _ => tui.series.iter().map(|p| p.cost_usd).sum(),
        };
        assert!((graphed - tui.totals.cost_usd).abs() < 1e-9, "{range:?}");
    }
}

#[test]
fn an_empty_filter_equals_no_filter_and_a_filter_only_narrows() {
    let events = events();
    let all = Data::build(&events, LiveView::default(), NOW, Range::All, "");
    let opus = Data::build(&events, LiveView::default(), NOW, Range::All, "OPUS");
    assert_eq!(all.totals.messages, 6);
    assert_eq!(opus.totals.messages, 3);
    assert!(opus.totals.cost_usd < all.totals.cost_usd);
    assert!(opus.models.iter().all(|g| g.key.contains("opus")));
}

/// Events on both sides of every calendar edge, with the clock on Thursday
/// 2026-09-03 12:00 UTC. Times are UTC, so there is no daylight saving and
/// the second before midnight always belongs to the day before.
mod calendar {
    use super::*;
    use crate::usage::query::WeekStart;

    const NOON: i64 = 1_788_436_800;

    fn at(ts: i64, cost: f64) -> UsageEvent {
        UsageEvent {
            event_id: format!("e{ts}"),
            timestamp: ts,
            session_id: "s1".into(),
            model: "claude-opus-5".into(),
            repo: "alpha".into(),
            input_tokens: 100,
            cost_usd: cost,
            ..UsageEvent::default()
        }
    }

    /// (timestamp, cost) pairs, each cost a distinct power of two so a total
    /// names exactly the events it holds.
    fn fixture() -> Vec<UsageEvent> {
        [
            (1_785_542_399, 1.0),   // 2026-07-31 23:59:59, before last month
            (1_785_542_400, 2.0),   // 2026-08-01 00:00:00, first second of last month
            (1_788_047_999, 4.0),   // 2026-08-29 23:59:59, Saturday, before last week
            (1_788_048_000, 8.0),   // 2026-08-30 00:00:00, Sunday, last week (Monday start)
            (1_788_134_399, 16.0),  // 2026-08-30 23:59:59, last second of last week
            (1_788_134_400, 32.0),  // 2026-08-31 00:00:00, Monday, this week, last month ends
            (1_788_220_799, 64.0),  // 2026-08-31 23:59:59, last second of last month
            (1_788_220_800, 128.0), // 2026-09-01 00:00:00, first second of this month
            (NOON, 256.0),          // 2026-09-03 12:00:00, today
            (1_788_739_199, 512.0), // 2026-09-06 23:59:59, future, inside the week
        ]
        .iter()
        .map(|(t, c)| at(*t, *c))
        .collect()
    }

    fn cost_in(events: &[UsageEvent], first: &str, last: &str) -> f64 {
        use crate::usage::aggregate::date_key;
        events
            .iter()
            .filter(|e| {
                let d = date_key(e.timestamp.div_euclid(86_400));
                d.as_str() >= first && d.as_str() <= last
            })
            .map(|e| e.cost_usd)
            .sum()
    }

    #[test]
    fn each_new_range_holds_the_events_between_its_dates() {
        // The clock is noon on the 3rd, so the event on the 6th is in the
        // future: it still counts for the week, as the web view counts it.
        let events = fixture();
        let cases = [
            (Range::Today, "2026-09-03", "2026-09-03", 256.0),
            (
                Range::ThisWeek(WeekStart::Monday),
                "2026-08-31",
                "2026-09-06",
                32.0 + 64.0 + 128.0 + 256.0 + 512.0,
            ),
            (
                Range::LastWeek(WeekStart::Monday),
                "2026-08-24",
                "2026-08-30",
                4.0 + 8.0 + 16.0,
            ),
            (
                Range::CurrentMonth,
                "2026-09-01",
                "2026-09-30",
                128.0 + 256.0 + 512.0,
            ),
            (
                Range::LastMonth,
                "2026-08-01",
                "2026-08-31",
                2.0 + 4.0 + 8.0 + 16.0 + 32.0 + 64.0,
            ),
        ];
        for (range, first, last, want) in cases {
            // The expected total is derived from the dates alone, not from
            // the range code under test.
            assert!(
                (cost_in(&events, first, last) - want).abs() < 1e-9,
                "{range:?}"
            );
            let web = data_json(&events, &[], NOON, range);
            let tui = Data::build(&events, LiveView::default(), NOON, range, "");
            assert_eq!(web["range"]["start"], first, "{range:?}");
            assert_eq!(web["range"]["end"], last, "{range:?}");
            let web_cost = web["totals"]["cost_usd"].as_f64().unwrap();
            assert!((web_cost - want).abs() < 1e-9, "{range:?} web {web_cost}");
            assert!((tui.totals.cost_usd - want).abs() < 1e-9, "{range:?} tui");
            assert_eq!(tui.totals.messages, web["totals"]["messages"], "{range:?}");
        }
    }

    #[test]
    fn a_sunday_week_start_moves_the_edge_in_both_views() {
        let events = fixture();
        let range = Range::ThisWeek(WeekStart::Sunday);
        let web = data_json(&events, &[], NOON, range);
        let tui = Data::build(&events, LiveView::default(), NOON, range, "");
        // Sunday 2026-08-30 to Saturday 2026-09-05: 8 + 16 + 32 + 64 + 128 + 256.
        assert_eq!(web["range"]["start"], "2026-08-30");
        assert_eq!(web["range"]["end"], "2026-09-05");
        assert!((tui.totals.cost_usd - 504.0).abs() < 1e-9);
        assert_eq!(
            web["totals"]["cost_usd"].as_f64(),
            Some(tui.totals.cost_usd)
        );
    }

    #[test]
    fn the_graph_covers_the_whole_range_and_adds_up_to_the_totals() {
        let events = fixture();
        for (range, days) in [
            (Range::LastWeek(WeekStart::Monday), 7),
            (Range::LastMonth, 31),
            // Today charts the last seven days, and its last bar is today.
            (Range::Today, 7),
            // A week and a month in progress show their days to come.
            (Range::ThisWeek(WeekStart::Monday), 7),
            (Range::CurrentMonth, 30),
        ] {
            let tui = Data::build(&events, LiveView::default(), NOON, range, "");
            assert_eq!(tui.series.len(), days, "{range:?}");
            if range == Range::Today {
                let last = tui.series.last().unwrap();
                assert!((last.cost_usd - tui.totals.cost_usd).abs() < 1e-9);
                continue;
            }
            let graphed: f64 = tui.series.iter().map(|p| p.cost_usd).sum();
            assert!((graphed - tui.totals.cost_usd).abs() < 1e-9, "{range:?}");
            let messages: usize = tui.series.iter().map(|p| p.messages).sum();
            assert_eq!(messages, tui.totals.messages, "{range:?}");
        }
    }

    /// Events either side of each edge of the previous spans, a distinct power
    /// of two each so a total names exactly the events it holds.
    fn comparison_fixture() -> Vec<UsageEvent> {
        [
            (1_785_542_399, 1.0),    // 2026-07-31 23:59:59, before last month
            (1_785_542_400, 2.0),    // 2026-08-01 00:00:00, first second of last month
            (1_785_758_400, 4.0),    // 2026-08-03 12:00:00, last second of the month span
            (1_785_758_401, 8.0),    // 2026-08-03 12:00:01, one second past it
            (1_787_738_400, 16.0),   // 2026-08-26 10:00:00, last week, inside the span
            (1_787_835_600, 32.0),   // 2026-08-27 13:00:00, last week, past the span
            (1_788_047_999, 64.0),   // 2026-08-29 23:59:59, Saturday
            (1_788_048_000, 128.0),  // 2026-08-30 00:00:00, Sunday
            (1_788_134_399, 256.0),  // 2026-08-30 23:59:59
            (1_788_134_400, 512.0),  // 2026-08-31 00:00:00, Monday
            (1_788_220_799, 1024.0), // 2026-08-31 23:59:59
            (1_788_220_800, 2048.0), // 2026-09-01 00:00:00, first second of this month
            (1_788_346_800, 4096.0), // 2026-09-02 11:00:00, yesterday, inside the span
            (1_788_354_000, 8192.0), // 2026-09-02 13:00:00, yesterday, past the span
            (NOON, 16384.0),         // 2026-09-03 12:00:00, the second of now
            (NOON + 1, 32768.0),     // one second after now
        ]
        .iter()
        .map(|(t, c)| at(*t, *c))
        .collect()
    }

    /// The cost of the events on the dates `first` to `last` (inclusive) at or
    /// before `until`. It reads dates, not the period code under test.
    fn oracle(events: &[UsageEvent], first: &str, last: &str, until: i64) -> f64 {
        events
            .iter()
            .filter(|e| {
                let d = date_key(e.timestamp.div_euclid(86_400));
                d.as_str() >= first && d.as_str() <= last && e.timestamp <= until
            })
            .map(|e| e.cost_usd)
            .sum()
    }

    #[test]
    fn each_period_and_its_previous_span_hold_the_events_between_their_dates() {
        let events = comparison_fixture();
        let day = 86_400;
        // (range, current dates, previous dates, the previous span's last second)
        let cases = [
            (
                Range::Today,
                ("2026-09-03", "2026-09-03"),
                ("2026-09-02", "2026-09-02"),
                NOON - day,
            ),
            (
                Range::ThisWeek(WeekStart::Monday),
                ("2026-08-31", "2026-09-06"),
                ("2026-08-24", "2026-08-30"),
                NOON - 7 * day,
            ),
            (
                Range::ThisWeek(WeekStart::Sunday),
                ("2026-08-30", "2026-09-05"),
                ("2026-08-23", "2026-08-29"),
                NOON - 7 * day,
            ),
            (
                Range::CurrentMonth,
                ("2026-09-01", "2026-09-30"),
                ("2026-08-01", "2026-08-31"),
                NOON - 31 * day,
            ),
        ];
        for (range, now_dates, before_dates, before_until) in cases {
            let tui = Data::build(&events, LiveView::default(), NOON, range, "");
            let c = tui.comparison.expect("a period compares");
            let want_now = oracle(&events, now_dates.0, now_dates.1, NOON);
            let want_before = oracle(&events, before_dates.0, before_dates.1, before_until);
            assert!(
                (c.current.cost_usd - want_now).abs() < 1e-9,
                "{range:?} now"
            );
            assert!(
                (c.previous.cost_usd - want_before).abs() < 1e-9,
                "{range:?} before"
            );
            // The `--json` comparison agrees with the view.
            let period = crate::usage::period::Period::of_range(range).unwrap();
            let json = comparison_json(&events, NOON, period);
            let cost = |side: &str| json[side]["cost_usd"].as_f64().unwrap();
            assert!(
                (cost("current") - want_now).abs() < 1e-9,
                "{range:?} json now"
            );
            assert!(
                (cost("previous") - want_before).abs() < 1e-9,
                "{range:?} json before"
            );
        }
    }

    #[test]
    fn the_second_before_midnight_belongs_to_the_day_before_in_every_view() {
        let midnight = 1_788_393_600; // 2026-09-03 00:00:00 UTC
        let events = [at(midnight - 1, 1.0), at(midnight, 2.0)];
        // Late on the day, so yesterday's whole day is the previous span.
        let late = midnight + 86_400 - 1;
        let tui = Data::build(&events, LiveView::default(), late, Range::Today, "");
        let c = tui.comparison.unwrap();
        assert_eq!((c.current.cost_usd, c.previous.cost_usd), (2.0, 1.0));
        let json = comparison_json(&events, late, crate::usage::period::Period::Today);
        assert_eq!(json["previous"]["messages"], 1);
    }
}
