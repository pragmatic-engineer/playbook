// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The dashboard's JSON: totals, every grouping, tool counts, and the two
//! server-rendered charts. Built from the same aggregation the terminal
//! summary uses.

use super::aggregate::{count_tools, group_usage, Dimension, Group};
use super::svg::bar_chart;
use super::{ToolInvocationEvent, ToolKind, UsageEvent};
use serde_json::{json, Value};

const CHART_DAYS: usize = 30;

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

pub fn data_json(usage: &[UsageEvent], tools: &[ToolInvocationEvent], now: i64) -> Value {
    let days = group_usage(usage, Dimension::Day);
    let models = group_usage(usage, Dimension::Model);
    let recent = days.len().saturating_sub(CHART_DAYS);

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
        "totals": {
            "messages": usage.len(),
            "input_tokens": usage.iter().map(|e| e.input_tokens).sum::<u64>(),
            "output_tokens": usage.iter().map(|e| e.output_tokens).sum::<u64>(),
            "cache_creation_tokens": usage.iter().map(|e| e.cache_creation_tokens).sum::<u64>(),
            "cache_read_tokens": usage.iter().map(|e| e.cache_read_tokens).sum::<u64>(),
            "cost_usd": usage.iter().map(|e| e.cost_usd).sum::<f64>(),
        },
        "groups": groups,
        "skills": counts_json(count_tools(tools, ToolKind::Skill)),
        "agents": counts_json(count_tools(tools, ToolKind::Agent)),
        "charts": {
            "cost_by_day": bar_chart("Cost per day", "USD", &cost_bars(&days[recent..])),
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
        }
    }

    #[test]
    fn totals_and_groups_match_hand_computed_sums() {
        let usage = vec![
            event(1788252682, "sonnet", 0.25),
            event(1788307201, "opus", 0.5),
        ];

        let data = data_json(&usage, &[], 1788400000);

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
    fn empty_input_is_valid_json_with_empty_groups() {
        let data = data_json(&[], &[], 0);

        assert_eq!(data["totals"]["messages"], 0);
        assert!(data["groups"]["day"].as_array().unwrap().is_empty());
        assert!(data["skills"].as_array().unwrap().is_empty());
    }
}
