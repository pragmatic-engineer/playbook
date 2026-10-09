// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The terminal view and the web JSON read one query layer. These tests pin
//! that: for the same events and range, both report the same totals and
//! groups, so the two views cannot drift apart.

use super::app::RANGES;
use super::data::Data;
use crate::usage::api::data_json;
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
    for range in RANGES {
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
    for range in RANGES {
        let web = data_json(&events, &[], NOW, range);
        let tui = Data::build(&events, LiveView::default(), NOW, range, "");
        for (dim, groups) in [
            ("model", &tui.models),
            ("repo", &tui.repos),
            ("day", &tui.days),
        ] {
            let ours: Vec<(String, u64, f64)> = groups
                .iter()
                .map(|g| (g.key.clone(), g.messages as u64, g.cost_usd))
                .collect();
            assert_eq!(ours, group_rows(&web, dim), "{dim} for {range:?}");
        }
    }
}

#[test]
fn the_graph_series_sums_to_the_same_cost_as_the_totals() {
    let events = events();
    for range in RANGES {
        let tui = Data::build(&events, LiveView::default(), NOW, range, "");
        let graphed: f64 = tui.series.iter().map(|p| p.cost_usd).sum();
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
