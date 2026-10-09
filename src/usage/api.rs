// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The dashboard's JSON: totals, every grouping, tool counts, and the two
//! server-rendered charts. Built from the same aggregation the terminal
//! summary uses.

use super::aggregate::{count_tools, date_key, group_usage, unpriced_models, Dimension, Group};
pub use super::query::{active_session_ids, live_window_start, Range, FEED_LIMIT};
use super::query::{
    clock, day_of, in_range, in_window, live_view, total_tokens, LiveView, Spend, Totals,
    BURN_MINUTES,
};
use super::svg::bar_chart;
use super::{ToolInvocationEvent, ToolKind, UsageEvent};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Days drawn in the per-day chart when the range is all time.
const CHART_DAYS: usize = 90;

fn range_json(range: Range, now: i64, usage: &[UsageEvent]) -> Value {
    let (start, end) = match range.days(now) {
        Some((first, past)) => (Some(first), Some(past - 1)),
        None => (
            usage.iter().map(|e| day_of(e.timestamp)).min(),
            usage.iter().map(|e| day_of(e.timestamp)).max(),
        ),
    };
    json!({
        "key": range.key(),
        "start": start.map(date_key),
        "end": end.map(date_key),
    })
}

/// One bar per day in the window up to today, with zero for idle days, so the
/// axis reads as a real calendar. All time keeps only days that have data.
fn day_bars(days: &[Group], window: Option<(i64, i64)>, now: i64) -> Vec<(String, f64)> {
    let Some((first, past)) = window else {
        let recent = days.len().saturating_sub(CHART_DAYS);
        return cost_bars(&days[recent..]);
    };
    let last = past.min(day_of(now) + 1);
    (first..last)
        .map(|day| {
            let key = date_key(day);
            let cost = days
                .iter()
                .find(|g| g.key == key)
                .map_or(0.0, |g| g.cost_usd);
            (key, cost)
        })
        .collect()
}

fn group_json(g: &Group) -> Value {
    json!({
        "key": g.key,
        "messages": g.messages,
        "input_tokens": g.input_tokens,
        "output_tokens": g.output_tokens,
        "cache_creation_tokens": g.cache_creation_tokens,
        "cache_read_tokens": g.cache_read_tokens,
        "cost_usd": g.cost_usd,
        "unpriced": g.unpriced,
    })
}

fn counts_json(counts: Vec<(String, u64)>) -> Value {
    Value::Array(
        counts
            .into_iter()
            .map(|(name, count)| json!({ "name": name, "count": count }))
            .collect(),
    )
}

fn cost_bars(groups: &[Group]) -> Vec<(String, f64)> {
    groups.iter().map(|g| (g.key.clone(), g.cost_usd)).collect()
}

pub fn data_json(
    usage: &[UsageEvent],
    tools: &[ToolInvocationEvent],
    now: i64,
    range: Range,
) -> Value {
    let window = range.days(now);
    let (usage, tools) = in_range(usage, tools, now, range);
    let (usage, tools) = (usage.as_slice(), tools.as_slice());

    let totals = Totals::of(usage);
    let days = group_usage(usage, Dimension::Day);
    let models = group_usage(usage, Dimension::Model);
    let (unpriced_messages, unpriced_model_names) = unpriced_models(usage);

    let dims = [
        ("day", Dimension::Day),
        ("week", Dimension::Week),
        ("model", Dimension::Model),
        ("repo", Dimension::Repo),
        ("branch", Dimension::Branch),
        ("effort", Dimension::Effort),
        ("account", Dimension::Account),
        ("agent", Dimension::Agent),
    ];
    let mut groups = serde_json::Map::new();
    for (name, dim) in dims {
        let rows: Vec<Value> = group_usage(usage, dim).iter().map(group_json).collect();
        groups.insert(name.to_string(), Value::Array(rows));
    }

    json!({
        "generated_at": now,
        "range": range_json(range, now, usage),
        "totals": {
            "messages": totals.messages,
            "input_tokens": totals.input_tokens,
            "output_tokens": totals.output_tokens,
            "cache_creation_tokens": totals.cache_creation_tokens,
            "cache_read_tokens": totals.cache_read_tokens,
            "cost_usd": totals.cost_usd,
            "unpriced_messages": unpriced_messages,
            "unpriced_models": unpriced_model_names,
        },
        "groups": groups,
        "skills": counts_json(count_tools(tools, ToolKind::Skill)),
        "agents": counts_json(count_tools(tools, ToolKind::Agent)),
        "charts": {
            "cost_by_day": bar_chart("Cost per day", "USD, UTC", &day_bars(&days, window, now)),
            "cost_by_model": bar_chart("Cost per model", "USD", &cost_bars(&models)),
        },
    })
}

/// Sessions listed at most; the response says how many more there were.
const SESSION_LIMIT: usize = 500;
/// Messages returned for one session (the newest), and drawn in its chart.
const TIMELINE_LIMIT: usize = 2000;
const TIMELINE_CHART_BARS: usize = 120;
const SESSION_ID_MAX: usize = 128;

/// A session id is a transcript's own id (a UUID in practice). Anything else is
/// refused before it reaches a lookup.
pub fn valid_session_id(id: &str) -> bool {
    id.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && id.len() <= SESSION_ID_MAX
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// One row per openable session (a valid id) with a message in `range`, newest
/// first.
/// Stats cover the messages inside the range. `usage` is ordered by time.
pub fn sessions_json(usage: &[UsageEvent], now: i64, range: Range) -> Value {
    let window = range.days(now);
    let mut by_session: BTreeMap<&str, Vec<&UsageEvent>> = BTreeMap::new();
    for e in usage
        .iter()
        .filter(|e| in_window(window, e.timestamp) && valid_session_id(&e.session_id))
    {
        by_session.entry(e.session_id.as_str()).or_default().push(e);
    }
    let mut rows: Vec<(i64, &str, Value)> = by_session
        .into_iter()
        .map(|(id, events)| {
            let start = events.iter().map(|e| e.timestamp).min().unwrap_or(0);
            let end = events.iter().map(|e| e.timestamp).max().unwrap_or(0);
            let mut per_model: BTreeMap<&str, u64> = BTreeMap::new();
            for e in &events {
                *per_model.entry(e.model.as_str()).or_insert(0) += 1;
            }
            let mut models: Vec<(&str, u64)> = per_model.into_iter().collect();
            models.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
            #[allow(
                clippy::expect_used,
                reason = "a session entry is only created by pushing its first event"
            )]
            let latest = events
                .iter()
                .max_by_key(|e| e.timestamp)
                .copied()
                .expect("a session has at least one event");
            let row = json!({
                "id": id,
                "start": start,
                "end": end,
                "duration": end - start,
                "repo": latest.repo,
                "branch": latest.branch,
                "agent": latest.agent,
                "model": models.first().map_or("", |m| m.0),
                "models": models.iter().map(|m| m.0).collect::<Vec<_>>(),
                "messages": events.len(),
                "tokens": events.iter().map(|e| total_tokens(e)).sum::<u64>(),
                "cost_usd": events.iter().map(|e| e.cost_usd).sum::<f64>(),
                "unpriced": events.iter().filter(|e| e.unpriced).count(),
            });
            (end, id, row)
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    let total = rows.len();
    let shown: Vec<Value> = rows
        .into_iter()
        .take(SESSION_LIMIT)
        .map(|(_, _, row)| row)
        .collect();
    json!({
        "generated_at": now,
        "range": range_json(range, now, usage),
        "total": total,
        "more": total - shown.len(),
        "sessions": shown,
    })
}

/// One session's messages in time order (the newest `TIMELINE_LIMIT`), with a
/// small cost-per-message chart. `None` when no event has that id.
pub fn session_json(usage: &[UsageEvent], id: &str) -> Option<Value> {
    let events: Vec<&UsageEvent> = usage.iter().filter(|e| e.session_id == id).collect();
    let latest = events.iter().max_by_key(|e| e.timestamp).copied()?;
    let omitted = events.len().saturating_sub(TIMELINE_LIMIT);
    let shown = &events[omitted..];
    let messages: Vec<Value> = shown
        .iter()
        .map(|e| {
            json!({
                "time": e.timestamp,
                "model": e.model,
                "tokens": total_tokens(e),
                "cost_usd": e.cost_usd,
                "unpriced": u64::from(e.unpriced),
            })
        })
        .collect();
    let tail = &shown[shown.len().saturating_sub(TIMELINE_CHART_BARS)..];
    let bars: Vec<(String, f64)> = tail
        .iter()
        .map(|e| (clock(e.timestamp), e.cost_usd))
        .collect();
    Some(json!({
        "id": id,
        "repo": latest.repo,
        "branch": latest.branch,
        "agent": latest.agent,
        "total": events.len(),
        "omitted": omitted,
        "messages": messages,
        "chart": bar_chart("Cost per message", "USD, UTC", &bars),
    }))
}

fn spend_json(spend: &Spend) -> Value {
    json!({
        "messages": spend.messages,
        "tokens": spend.tokens,
        "cost_usd": spend.cost_usd,
        "unpriced": spend.unpriced,
    })
}

/// What the Live tab shows, as JSON. See `query::live_view` for the inputs.
pub fn live_json(
    window: &[UsageEvent],
    sessions: &[UsageEvent],
    latest: &[UsageEvent],
    now: i64,
) -> Value {
    let LiveView {
        generated_at,
        active,
        hour,
        today,
        burn,
        feed,
    } = live_view(window, sessions, latest, now);
    let active: Vec<Value> = active
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "repo": a.repo,
                "agent": a.agent,
                "model": a.model,
                "last": a.last,
                "messages": a.messages,
                "cost_usd": a.cost_usd,
                "unpriced": a.unpriced,
            })
        })
        .collect();
    let feed: Vec<Value> = feed
        .iter()
        .map(|e| {
            json!({
                "time": e.timestamp,
                "session": e.session_id,
                "repo": e.repo,
                "agent": e.agent,
                "model": e.model,
                "tokens": total_tokens(e),
                "cost_usd": e.cost_usd,
                "unpriced": u64::from(e.unpriced),
            })
        })
        .collect();
    json!({
        "generated_at": generated_at,
        "active": active,
        "hour": spend_json(&hour),
        "today": spend_json(&today),
        "burn_chart": bar_chart(
            "Spend per minute",
            &format!("USD, UTC, last {BURN_MINUTES} minutes"),
            &burn
        ),
        "feed": feed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::query::{hour_start, ACTIVE_SECONDS};

    fn event(ts: i64, model: &str, cost: f64) -> UsageEvent {
        UsageEvent {
            event_id: format!("{ts}{model}"),
            timestamp: ts,
            session_id: "s".into(),
            account: "a".into(),
            model: model.into(),
            effort: "high".into(),
            repo: "r".into(),
            branch: "b".into(),
            input_tokens: 10,
            output_tokens: 20,
            cache_creation_tokens: 1,
            cache_read_tokens: 2,
            cost_usd: cost,
            ..UsageEvent::default()
        }
    }

    #[test]
    fn every_row_kind_carries_its_unpriced_count() {
        let now = 1788400000;
        let mut codex = in_session("c", now - 60, "gpt-5-codex", 0.0);
        codex.unpriced = true;
        let priced = in_session("c", now - 30, "sonnet", 1.0);
        let events = vec![codex, priced];

        let data = data_json(&events, &[], now, Range::All);
        let models = data["groups"]["model"].as_array().unwrap().clone();
        let counts: Vec<u64> = models
            .iter()
            .map(|r| r["unpriced"].as_u64().unwrap())
            .collect();
        assert_eq!(counts.iter().sum::<u64>(), 1, "{models:?}");

        let sessions = sessions_json(&events, now, Range::All);
        assert_eq!(sessions["sessions"][0]["unpriced"], 1);

        let refs: Vec<&UsageEvent> = events.iter().collect();
        assert_eq!(Spend::of(&refs).unpriced, 1);
        let live = live_json(&events, &events, &events, now);
        assert_eq!(live["active"][0]["unpriced"], 1);
        let flags: Vec<u64> = live["feed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["unpriced"].as_u64().unwrap())
            .collect();
        assert_eq!(flags.iter().sum::<u64>(), 1);
    }

    #[test]
    fn totals_report_unpriced_messages_and_their_models() {
        let mut usage = vec![event(1788252682, "sonnet", 0.25)];
        usage[0].unpriced = true;

        let data = data_json(&usage, &[], 1788400000, Range::All);

        assert_eq!(data["totals"]["unpriced_messages"], 1);
        assert_eq!(data["totals"]["unpriced_models"][0], "sonnet");
    }

    #[test]
    fn totals_and_groups_match_hand_computed_sums() {
        let usage = vec![
            event(1788252682, "sonnet", 0.25),
            event(1788307201, "opus", 0.5),
        ];

        let data = data_json(&usage, &[], 1788400000, Range::All);

        assert_eq!(data["generated_at"], 1788400000);
        assert_eq!(data["totals"]["messages"], 2);
        assert_eq!(data["totals"]["output_tokens"], 40);
        assert!((data["totals"]["cost_usd"].as_f64().unwrap() - 0.75).abs() < 1e-9);
        assert_eq!(data["groups"]["day"].as_array().unwrap().len(), 2);
        assert_eq!(data["groups"]["model"][0]["key"], "opus");
        assert_eq!(
            data["charts"]["cost_by_day"]
                .as_str()
                .unwrap()
                .matches("<rect")
                .count(),
            2
        );
        assert_eq!(
            data["charts"]["cost_by_model"]
                .as_str()
                .unwrap()
                .matches("<rect")
                .count(),
            2
        );
    }

    #[test]
    fn chart_titles_have_one_unit_note_and_model_axis_labels_are_short() {
        let usage = vec![event(1788252682, "claude-sonnet-5-5", 0.25)];

        let data = data_json(&usage, &[], 1788400000, Range::All);

        let day = data["charts"]["cost_by_day"].as_str().unwrap();
        let model = data["charts"]["cost_by_model"].as_str().unwrap();
        assert!(day.contains(">Cost per day (USD, UTC)</text>"), "{day}");
        assert!(model.contains(">Cost per model (USD)</text>"), "{model}");
        assert!(model.contains(">sonnet-5-5</text>"), "{model}");
        assert!(model.contains("<title>claude-sonnet-5-5: "), "{model}");
    }

    #[test]
    fn empty_input_is_valid_json_with_empty_groups() {
        let data = data_json(&[], &[], 0, Range::All);

        assert_eq!(data["totals"]["messages"], 0);
        assert!(data["groups"]["day"].as_array().unwrap().is_empty());
        assert!(data["skills"].as_array().unwrap().is_empty());
        assert!(data["range"]["start"].is_null());
    }

    const SEP_3_NOON: i64 = 1788436800;

    fn span(range: Range, now: i64) -> (String, String) {
        let (first, past) = range.days(now).expect("a bounded range");
        (date_key(first), date_key(past - 1))
    }

    #[test]
    fn parses_every_range_and_rejects_anything_else() {
        assert_eq!(Range::parse(None), Some(Range::All));
        assert_eq!(Range::parse(Some("all")), Some(Range::All));
        assert_eq!(Range::parse(Some("30d")), Some(Range::LastDays(30)));
        assert_eq!(Range::parse(Some("60d")), Some(Range::LastDays(60)));
        assert_eq!(Range::parse(Some("90d")), Some(Range::LastDays(90)));
        assert_eq!(Range::parse(Some("month")), Some(Range::CurrentMonth));
        assert_eq!(Range::parse(Some("7d")), None);
        assert_eq!(Range::parse(Some("")), None);
    }

    #[test]
    fn last_n_days_include_today_and_months_run_first_to_last_day() {
        assert_eq!(
            span(Range::LastDays(30), SEP_3_NOON),
            ("2026-08-05".into(), "2026-09-03".into())
        );
        assert_eq!(
            span(Range::LastDays(90), SEP_3_NOON),
            ("2026-06-06".into(), "2026-09-03".into())
        );
        assert_eq!(
            span(Range::CurrentMonth, SEP_3_NOON),
            ("2026-09-01".into(), "2026-09-30".into())
        );
        assert_eq!(
            span(Range::CurrentMonth, 1834185600),
            ("2028-02-01".into(), "2028-02-29".into())
        );
        assert_eq!(
            span(Range::CurrentMonth, 1798761599),
            ("2026-12-01".into(), "2026-12-31".into())
        );
        assert_eq!(Range::All.days(SEP_3_NOON), None);
    }

    #[test]
    fn a_range_filters_usage_and_tools_at_the_utc_day_edge() {
        let usage = vec![
            event(1785887999, "sonnet", 1.0),
            event(1785888000, "sonnet", 0.25),
            event(1788252682, "opus", 0.5),
        ];
        let tool = |ts: i64| ToolInvocationEvent {
            event_id: ts.to_string(),
            timestamp: ts,
            session_id: "s".into(),
            account: "a".into(),
            kind: ToolKind::Skill,
            name: "plan".into(),
        };
        let tools = vec![tool(1785887999), tool(1788252682)];

        let data = data_json(&usage, &tools, SEP_3_NOON, Range::LastDays(30));

        assert_eq!(data["totals"]["messages"], 2);
        assert!((data["totals"]["cost_usd"].as_f64().unwrap() - 0.75).abs() < 1e-9);
        assert_eq!(data["skills"][0]["count"], 1);
        assert_eq!(data["range"]["key"], "30d");
        assert_eq!(data["range"]["start"], "2026-08-05");
        assert_eq!(data["range"]["end"], "2026-09-03");
    }

    #[test]
    fn the_day_chart_draws_every_day_of_the_window_up_to_today() {
        let usage = vec![event(1788252682, "opus", 0.5)];

        let data = data_json(&usage, &[], SEP_3_NOON, Range::CurrentMonth);

        let day = data["charts"]["cost_by_day"].as_str().unwrap();
        assert_eq!(
            day.matches("<rect").count(),
            3,
            "Sep 1 to Sep 3, not the whole month"
        );
        assert!(day.contains("<title>2026-09-01: 0.5000</title>"), "{day}");
        assert!(day.contains("<title>2026-09-02: 0.0000</title>"), "{day}");
        assert!(day.contains("<title>2026-09-03: 0.0000</title>"), "{day}");
        assert!(
            !day.contains("2026-08-31") && !day.contains("2026-09-04"),
            "{day}"
        );
        assert_eq!(data["range"]["end"], "2026-09-30");

        let last30 = data_json(&usage, &[], SEP_3_NOON, Range::LastDays(30));
        let chart = last30["charts"]["cost_by_day"].as_str().unwrap();
        assert_eq!(chart.matches("<rect").count(), 30);
        assert!(
            chart.contains("<title>2026-08-05: 0.0000</title>"),
            "{chart}"
        );
        assert!(
            chart.contains("<title>2026-09-03: 0.0000</title>"),
            "{chart}"
        );
    }

    #[test]
    fn all_time_draws_only_the_latest_90_days_with_data() {
        let usage: Vec<UsageEvent> = (0..100)
            .map(|i| event(1785888000 + i * 86_400, "opus", 1.0))
            .collect();

        let data = data_json(&usage, &[], SEP_3_NOON, Range::All);

        let day = data["charts"]["cost_by_day"].as_str().unwrap();
        assert_eq!(day.matches("<rect").count(), 90);
        assert!(
            day.contains("<title>2026-08-15: 1.0000</title>"),
            "oldest kept day"
        );
        assert!(
            day.contains("<title>2026-11-12: 1.0000</title>"),
            "newest day"
        );
        assert!(!day.contains("2026-08-14"), "older days dropped");
    }

    #[test]
    fn ranges_hold_at_midnight_across_the_year_and_in_a_short_february() {
        const JAN_1_2027: i64 = 1798761600;
        const FEB_15_2027: i64 = 1802649600;

        assert_eq!(
            span(Range::CurrentMonth, JAN_1_2027),
            ("2027-01-01".into(), "2027-01-31".into())
        );
        assert_eq!(
            span(Range::LastDays(30), JAN_1_2027),
            ("2026-12-03".into(), "2027-01-01".into())
        );
        assert_eq!(
            span(Range::LastDays(30), JAN_1_2027 - 1),
            ("2026-12-02".into(), "2026-12-31".into())
        );
        assert_eq!(
            span(Range::CurrentMonth, FEB_15_2027),
            ("2027-02-01".into(), "2027-02-28".into())
        );

        let usage = vec![
            event(JAN_1_2027 - 1, "opus", 1.0),
            event(JAN_1_2027, "opus", 2.0),
        ];
        let data = data_json(&usage, &[], JAN_1_2027 - 1, Range::LastDays(30));
        assert_eq!(
            data["totals"]["messages"], 1,
            "tomorrow is outside the window"
        );
        let month = data_json(&[], &[], JAN_1_2027, Range::CurrentMonth);
        let chart = month["charts"]["cost_by_day"].as_str().unwrap();
        assert_eq!(chart.matches("<rect").count(), 1);
    }

    #[test]
    fn all_time_reports_the_first_and_last_day_with_data() {
        let usage = vec![
            event(1788252682, "opus", 0.5),
            event(1785888000, "opus", 0.5),
        ];

        let data = data_json(&usage, &[], SEP_3_NOON, Range::All);

        assert_eq!(data["range"]["key"], "all");
        assert_eq!(data["range"]["start"], "2026-08-05");
        assert_eq!(data["range"]["end"], "2026-09-01");
    }

    fn in_session(id: &str, ts: i64, model: &str, cost: f64) -> UsageEvent {
        UsageEvent {
            session_id: id.into(),
            agent: "codex".into(),
            ..event(ts, model, cost)
        }
    }

    #[test]
    fn the_by_agent_group_splits_claude_code_from_codex() {
        let mut usage = vec![event(1788252682, "sonnet", 0.25)];
        usage[0].agent = "claude-code".into();
        usage.push(in_session("c", 1788252700, "gpt", 0.0));

        let data = data_json(&usage, &[], 1788400000, Range::All);

        let keys: Vec<&str> = data["groups"]["agent"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["key"].as_str().unwrap())
            .collect();
        assert_eq!(keys, ["claude-code", "codex"]);
    }

    #[test]
    fn session_ids_are_a_short_safe_charset() {
        for ok in ["s1", "0199aaaa-0000-7000-8000-000000000002", "a_b.c-D9"] {
            assert!(valid_session_id(ok), "{ok}");
        }
        let long = "a".repeat(129);
        for bad in [
            "", "..", ".x", "-x", "a b", "a/b", "../x", "a%2Fb", "a:b", "<s>", "é", &long,
        ] {
            assert!(!valid_session_id(bad), "{bad}");
        }
        assert!(valid_session_id(&"a".repeat(128)));
    }

    #[test]
    fn sessions_are_one_row_each_newest_first_with_hand_computed_sums() {
        let usage = vec![
            in_session("old", 1788252000, "sonnet", 0.5),
            in_session("new", 1788300000, "opus", 1.0),
            in_session("new", 1788300060, "opus", 2.0),
            in_session("new", 1788300090, "sonnet", 0.25),
        ];

        let data = sessions_json(&usage, 1788400000, Range::All);

        let rows = data["sessions"].as_array().unwrap();
        assert_eq!(
            (data["total"].as_u64(), data["more"].as_u64()),
            (Some(2), Some(0))
        );
        assert_eq!(rows[0]["id"], "new");
        assert_eq!(rows[1]["id"], "old");
        assert_eq!(rows[0]["start"], 1788300000);
        assert_eq!(rows[0]["end"], 1788300090);
        assert_eq!(rows[0]["duration"], 90);
        assert_eq!(rows[0]["messages"], 3);
        assert_eq!(rows[0]["tokens"], 3 * 33);
        assert_eq!(rows[0]["model"], "opus");
        assert_eq!(rows[0]["models"], json!(["opus", "sonnet"]));
        assert_eq!(rows[0]["agent"], "codex");
        assert!((rows[0]["cost_usd"].as_f64().unwrap() - 3.25).abs() < 1e-9);
    }

    #[test]
    fn sessions_respect_the_range_and_cap_with_a_count_of_the_rest() {
        let now = 1788400000;
        let mut usage = vec![in_session("ancient", now - 200 * 86_400, "m", 1.0)];
        for i in 0..(SESSION_LIMIT + 7) {
            usage.push(in_session(
                &format!("s{i:04}"),
                now - 1000 + i as i64,
                "m",
                1.0,
            ));
        }

        let recent = sessions_json(&usage, now, Range::LastDays(30));
        let all = sessions_json(&usage, now, Range::All);

        assert_eq!(recent["total"], SESSION_LIMIT + 7);
        assert_eq!(recent["sessions"].as_array().unwrap().len(), SESSION_LIMIT);
        assert_eq!(recent["more"], 7);
        assert_eq!(
            recent["sessions"][0]["id"],
            format!("s{:04}", SESSION_LIMIT + 6)
        );
        assert_eq!(all["total"], SESSION_LIMIT + 8);
    }

    #[test]
    fn a_session_timeline_lists_its_messages_in_order_with_a_chart() {
        let usage = vec![
            in_session("a", 1788300000, "sonnet", 0.5),
            in_session("b", 1788300030, "other", 9.0),
            in_session("a", 1788300065, "opus", 1.0),
        ];

        let data = session_json(&usage, "a").expect("known session");

        let messages = data["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["model"], "sonnet");
        assert_eq!(messages[1]["time"], 1788300065);
        assert_eq!(messages[1]["tokens"], 33);
        assert_eq!(data["omitted"], 0);
        let chart = data["chart"].as_str().unwrap();
        assert_eq!(chart.matches("<rect").count(), 2);
        assert!(chart.contains("Cost per message"));
        assert!(session_json(&usage, "missing").is_none());
    }

    #[test]
    fn a_long_session_keeps_only_the_newest_messages_and_says_so() {
        let usage: Vec<UsageEvent> = (0..(TIMELINE_LIMIT + 5) as i64)
            .map(|i| in_session("big", 1788300000 + i, "m", 0.0))
            .collect();

        let data = session_json(&usage, "big").unwrap();

        assert_eq!(data["messages"].as_array().unwrap().len(), TIMELINE_LIMIT);
        assert_eq!(data["omitted"], 5);
        assert_eq!(data["total"], TIMELINE_LIMIT + 5);
        assert_eq!(data["messages"][0]["time"], 1788300005);
        let chart = data["chart"].as_str().unwrap();
        assert_eq!(chart.matches("<rect").count(), TIMELINE_CHART_BARS);
        let newest = clock(1788300000 + (TIMELINE_LIMIT + 4) as i64);
        assert!(
            chart.contains(&newest),
            "the chart ends at the newest message"
        );
        assert!(!chart.contains(&format!("<title>{}:", clock(1788300005))));
    }

    #[test]
    fn hostile_names_travel_as_json_text_and_the_chart_holds_only_clock_labels() {
        let mut usage = vec![in_session("ok", 1788300000, "<script>x</script>", 1.0)];
        usage[0].repo = "<img onerror=x>".into();

        let data = session_json(&usage, "ok").unwrap();

        assert_eq!(data["messages"][0]["model"], "<script>x</script>");
        assert_eq!(data["repo"], "<img onerror=x>");
        let chart = data["chart"].as_str().unwrap();
        assert!(!chart.contains("<script") && !chart.contains("<img"));
        assert!(chart.contains(&clock(1788300000)));
    }

    #[test]
    fn a_session_crossing_the_range_edge_counts_only_the_messages_inside() {
        let now = 1788400000;
        let usage = vec![
            in_session("x", now - 40 * 86_400, "m", 5.0),
            in_session("x", now - 60, "m", 1.0),
        ];

        let inside = sessions_json(&usage, now, Range::LastDays(30));
        let all = sessions_json(&usage, now, Range::All);

        let row = &inside["sessions"][0];
        assert_eq!(
            (row["messages"].as_u64(), row["duration"].as_i64()),
            (Some(1), Some(0))
        );
        assert_eq!(row["start"], now - 60);
        assert!((row["cost_usd"].as_f64().unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(all["sessions"][0]["messages"], 2);
        assert_eq!(inside["range"]["key"], "30d");
        assert!(inside["range"]["start"].is_string());
    }

    #[test]
    fn sessions_without_a_usable_id_are_not_listed() {
        let usage = vec![
            in_session("", 1788300000, "m", 1.0),
            in_session("a/b", 1788300001, "m", 1.0),
            in_session("fine", 1788300002, "m", 1.0),
        ];

        let list = sessions_json(&usage, 1788400000, Range::All);

        assert_eq!(list["total"], 1);
        assert_eq!(list["sessions"][0]["id"], "fine");
    }

    #[test]
    fn repo_branch_and_agent_come_from_the_latest_message() {
        let mut usage = vec![
            in_session("n", 1788300000, "m", 0.0),
            in_session("n", 1788300100, "m", 0.0),
        ];
        usage[1].branch = "feature".into();
        usage[1].repo = "later".into();

        let list = sessions_json(&usage, 1788400000, Range::All);
        let one = session_json(&usage, "n").unwrap();

        assert_eq!(list["sessions"][0]["branch"], "feature");
        assert_eq!(list["sessions"][0]["repo"], "later");
        assert_eq!(one["branch"], "feature");
    }

    #[test]
    fn the_live_window_starts_at_midnight_or_an_hour_ago_whichever_is_earlier() {
        let midnight = 1788393600;
        assert_eq!(midnight % 86_400, 0);
        assert_eq!(live_window_start(midnight + 1800), midnight + 1800 - 3540);
        assert_eq!(live_window_start(midnight + 5 * 3600), midnight);
    }

    #[test]
    fn live_sums_the_last_hour_and_today_and_lists_only_recent_sessions() {
        let now = 1788393600 + 5 * 3600;
        let window = vec![
            in_session("early", now - 4 * 3600, "m", 4.0),
            in_session("hour", now - 3000, "m", 2.0),
            in_session("busy", now - 600, "m", 1.0),
            in_session("busy", now - 30, "opus", 0.5),
        ];
        let sessions = vec![
            in_session("busy", now - 7200, "m", 8.0),
            window[2].clone(),
            window[3].clone(),
        ];
        let latest: Vec<UsageEvent> = window.iter().rev().cloned().collect();

        let live = live_json(&window, &sessions, &latest, now);

        assert_eq!(live["hour"]["messages"], 3);
        assert!((live["hour"]["cost_usd"].as_f64().unwrap() - 3.5).abs() < 1e-9);
        assert_eq!(live["today"]["messages"], 4);
        assert!((live["today"]["cost_usd"].as_f64().unwrap() - 7.5).abs() < 1e-9);
        let active = live["active"].as_array().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0]["id"], "busy");
        assert_eq!(active[0]["model"], "opus");
        assert_eq!(active[0]["last"], now - 30);
        assert_eq!(active[0]["messages"], 3);
        assert!((active[0]["cost_usd"].as_f64().unwrap() - 9.5).abs() < 1e-9);
        assert_eq!(live["feed"][0]["time"], now - 30);
        let chart = live["burn_chart"].as_str().unwrap();
        assert_eq!(chart.matches("<rect").count(), 60);
    }

    #[test]
    fn a_session_exactly_fifteen_minutes_quiet_is_no_longer_active() {
        let now = 1788400000;
        let window = vec![
            in_session("edge", now - ACTIVE_SECONDS, "m", 1.0),
            in_session("in", now - ACTIVE_SECONDS + 1, "m", 1.0),
        ];

        assert_eq!(active_session_ids(&window, now), ["in"]);
    }

    #[test]
    fn the_feed_is_capped_and_the_burn_buckets_by_minute() {
        let now = 1788400000;
        let latest: Vec<UsageEvent> = (0..30)
            .map(|i| in_session("f", now - i, "m", 0.0))
            .collect();
        let hot = vec![
            in_session("a", now - 10, "m", 1.0),
            in_session("a", now - 20, "m", 1.0),
        ];

        let live = live_json(&hot, &[], &latest, now);

        assert_eq!(live["feed"].as_array().unwrap().len(), FEED_LIMIT);
        let chart = live["burn_chart"].as_str().unwrap();
        assert!(chart.contains("<title>"));
        assert!(
            chart.contains(">2.00</text>"),
            "both messages land in one minute: {chart}"
        );
    }

    #[test]
    fn the_hour_crossing_midnight_is_not_the_same_as_today() {
        let midnight = 1788393600;
        let now = midnight + 1800;
        let window = vec![
            in_session("a", midnight - 600, "m", 3.0),
            in_session("b", midnight + 600, "m", 1.0),
        ];

        let live = live_json(&window, &[], &[], now);

        assert_eq!(live["hour"]["messages"], 2);
        assert!((live["hour"]["cost_usd"].as_f64().unwrap() - 4.0).abs() < 1e-9);
        assert_eq!(live["today"]["messages"], 1);
        assert!((live["today"]["cost_usd"].as_f64().unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_burn_chart_buckets_by_minute_and_adds_up_to_the_hour() {
        let now = 1788400000;
        let window = vec![
            in_session("a", now - 10, "m", 1.0),
            in_session("a", now - 3500, "m", 0.5),
            in_session("a", hour_start(now), "m", 0.25),
        ];

        let live = live_json(&window, &[], &[], now);

        let chart = live["burn_chart"].as_str().unwrap();
        let titles: Vec<f64> = chart
            .split("<title>")
            .skip(1)
            .map(|t| {
                t.split(": ")
                    .nth(1)
                    .unwrap()
                    .split('<')
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap()
            })
            .collect();
        assert_eq!(titles.len(), 60);
        assert!(
            (titles[59] - 1.0).abs() < 1e-9,
            "the last bar is this minute"
        );
        assert!(
            (titles[0] - 0.25).abs() < 1e-9,
            "the first bar is 59 minutes ago"
        );
        let total: f64 = titles.iter().sum();
        assert!((total - live["hour"]["cost_usd"].as_f64().unwrap()).abs() < 1e-9);
        assert!(chart.contains(&format!("<title>{}: 1.0000", &clock(now)[..5])));
    }
}
