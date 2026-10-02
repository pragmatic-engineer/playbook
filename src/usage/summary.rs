// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Plain-text usage report for the terminal. Structured on purpose: whatever
//! agent is driving the session can read it and give the insights itself.

use super::aggregate::{count_tools, group_usage, unpriced_models, Dimension, Group};
use super::{ToolInvocationEvent, ToolKind, UsageEvent};
use std::fmt::Write;

const RECENT_DAYS: usize = 14;
const TOP_TOOLS: usize = 10;

pub const EMPTY_MESSAGE: &str =
    "No usage recorded yet. Run a Claude Code session, then run `playbook usage` again.";

fn group_table(out: &mut String, title: &str, groups: &[Group]) {
    let _ = writeln!(out, "\nBy {title}");
    let width = groups.iter().map(|g| g.key.len()).max().unwrap_or(0).max(8);
    let _ = writeln!(
        out,
        "  {:<width$}  {:>8}  {:>12}  {:>12}  {:>14}  {:>10}",
        "", "messages", "input", "output", "cache r/w", "cost USD"
    );
    for g in groups {
        let cache = format!("{}/{}", g.cache_read_tokens, g.cache_creation_tokens);
        let _ = writeln!(
            out,
            "  {:<width$}  {:>8}  {:>12}  {:>12}  {:>14}  {:>10.4}",
            g.key, g.messages, g.input_tokens, g.output_tokens, cache, g.cost_usd
        );
    }
}

fn tool_list(out: &mut String, title: &str, counts: &[(String, u64)]) {
    if counts.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n{title}");
    for (name, n) in counts.iter().take(TOP_TOOLS) {
        let _ = writeln!(out, "  {n:>6}  {name}");
    }
}

pub fn render(usage: &[UsageEvent], tools: &[ToolInvocationEvent]) -> String {
    if usage.is_empty() && tools.is_empty() {
        return format!("{EMPTY_MESSAGE}\n");
    }
    let total_cost: f64 = usage.iter().map(|e| e.cost_usd).sum();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Usage: {} messages, ${:.4} estimated cost across all recorded sessions",
        usage.len(),
        total_cost
    );
    let (unpriced, models) = unpriced_models(usage);
    if unpriced > 0 {
        let _ = writeln!(
            out,
            "Note: {unpriced} messages are unpriced (no known price for {}); they count as $0.",
            models.join(", ")
        );
    }

    let days = group_usage(usage, Dimension::Day);
    let recent = days.len().saturating_sub(RECENT_DAYS);
    group_table(
        &mut out,
        &format!("day (UTC, last {RECENT_DAYS})"),
        &days[recent..],
    );
    for dim in [
        Dimension::Week,
        Dimension::Model,
        Dimension::Repo,
        Dimension::Branch,
        Dimension::Effort,
        Dimension::Account,
    ] {
        group_table(&mut out, dim.label(), &group_usage(usage, dim));
    }
    tool_list(&mut out, "Top skills", &count_tools(tools, ToolKind::Skill));
    tool_list(&mut out, "Top agents", &count_tools(tools, ToolKind::Agent));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_prints_the_empty_message() {
        assert_eq!(render(&[], &[]), format!("{EMPTY_MESSAGE}\n"));
    }

    #[test]
    fn report_has_a_row_with_the_hand_computed_model_total() {
        let event = UsageEvent {
            event_id: "e".into(),
            timestamp: 1788252682,
            session_id: "s".into(),
            account: "dev@example.com".into(),
            model: "claude-sonnet-5".into(),
            effort: "high".into(),
            repo: "proj".into(),
            branch: "main".into(),
            input_tokens: 2,
            output_tokens: 770,
            cache_creation_tokens: 29653,
            cache_read_tokens: 22178,
            cost_usd: 0.12940815,
            ..UsageEvent::default()
        };

        let text = render(&[event], &[]);

        assert!(text.contains("1 messages, $0.1294 estimated cost"));
        let row = text
            .lines()
            .find(|l| l.contains("claude-sonnet-5"))
            .unwrap();
        assert!(row.contains("770") && row.contains("22178/29653") && row.contains("0.1294"));
        assert!(text.contains("2026-09-01"));
        assert!(text.contains("dev@example.com"));
    }

    #[test]
    fn unpriced_messages_are_reported_honestly_under_the_header() {
        let mut event = UsageEvent {
            event_id: "e".into(),
            timestamp: 1788252682,
            model: "claude-future-9".into(),
            input_tokens: 5,
            ..UsageEvent::default()
        };
        event.apply_pricing();

        let text = render(&[event], &[]);

        assert!(text.contains(
            "Note: 1 messages are unpriced (no known price for claude-future-9); they count as $0."
        ));
        assert!(!render(&[], &[]).contains("unpriced"));
    }
}
