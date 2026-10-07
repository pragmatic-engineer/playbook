// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The dashboard's JSON: totals, every grouping, tool counts, and the two
//! server-rendered charts. Built from the same aggregation the terminal
//! summary uses.

use super::aggregate::{
    civil_from_days, count_tools, date_key, group_usage, unpriced_models, Dimension, Group,
    SECONDS_PER_DAY,
};
use super::svg::bar_chart;
use super::{ToolInvocationEvent, ToolKind, UsageEvent};
use serde_json::{json, Value};

/// Days drawn in the per-day chart when the range is all time.
const CHART_DAYS: usize = 90;

/// The date window the dashboard shows. Days are UTC, like every grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    LastDays(i64),
    CurrentMonth,
    All,
}

impl Range {
    /// The `range` query value. No value means all time, so older callers
    /// keep their behaviour.
    pub fn parse(value: Option<&str>) -> Option<Range> {
        match value {
            None | Some("all") => Some(Range::All),
            Some("30d") => Some(Range::LastDays(30)),
            Some("60d") => Some(Range::LastDays(60)),
            Some("90d") => Some(Range::LastDays(90)),
            Some("month") => Some(Range::CurrentMonth),
            Some(_) => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Range::LastDays(30) => "30d",
            Range::LastDays(60) => "60d",
            Range::LastDays(_) => "90d",
            Range::CurrentMonth => "month",
            Range::All => "all",
        }
    }

    /// First and one-past-last UTC day, or `None` for all time. "Last N days"
    /// includes today; the current month runs from its 1st to its last day.
    pub fn days(self, now: i64) -> Option<(i64, i64)> {
        let today = now.div_euclid(SECONDS_PER_DAY);
        match self {
            Range::All => None,
            Range::LastDays(n) => Some((today - n + 1, today + 1)),
            Range::CurrentMonth => {
                let first = today - (civil_from_days(today).2 - 1);
                let probe = first + 31;
                Some((first, probe - (civil_from_days(probe).2 - 1)))
            }
        }
    }
}

fn day_of(timestamp: i64) -> i64 {
    timestamp.div_euclid(SECONDS_PER_DAY)
}

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
    let inside = |timestamp: i64| {
        window.is_none_or(|(first, past)| (first..past).contains(&day_of(timestamp)))
    };
    let usage: Vec<UsageEvent> = usage
        .iter()
        .filter(|e| inside(e.timestamp))
        .cloned()
        .collect();
    let tools: Vec<ToolInvocationEvent> = tools
        .iter()
        .filter(|t| inside(t.timestamp))
        .cloned()
        .collect();
    let (usage, tools) = (usage.as_slice(), tools.as_slice());

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
            "messages": usage.len(),
            "input_tokens": usage.iter().map(|e| e.input_tokens).sum::<u64>(),
            "output_tokens": usage.iter().map(|e| e.output_tokens).sum::<u64>(),
            "cache_creation_tokens": usage.iter().map(|e| e.cache_creation_tokens).sum::<u64>(),
            "cache_read_tokens": usage.iter().map(|e| e.cache_read_tokens).sum::<u64>(),
            "cost_usd": usage.iter().map(|e| e.cost_usd).sum::<f64>(),
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(data["range"]["end"], "2026-09-30");
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
}
