// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! What the terminal view draws: one immutable snapshot of the store for a
//! range and a filter. Built by the same query layer the web JSON uses
//! (`usage::query`), so both views report the same numbers.

use crate::usage::aggregate::{group_usage, Dimension, Group};
use crate::usage::query::{self, LiveView, Range, Totals};
use crate::usage::UsageEvent;
use rusqlite::Connection;

#[derive(Debug, Clone)]
pub struct Data {
    pub now: i64,
    pub range: Range,
    pub filter: String,
    pub totals: Totals,
    /// Per UTC day, oldest first. Idle days are absent.
    pub days: Vec<Group>,
    pub models: Vec<Group>,
    pub repos: Vec<Group>,
    pub live: LiveView,
}

/// Whether an event matches the filter text: a case insensitive substring of
/// its model, repo, branch or session id. Empty text matches everything.
pub fn matches(event: &UsageEvent, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let needle = filter.to_lowercase();
    [&event.model, &event.repo, &event.branch, &event.session_id]
        .iter()
        .any(|field| field.to_lowercase().contains(&needle))
}

impl Data {
    /// Builds the snapshot from the events of the range. `window` is every
    /// event the range could contain (events outside it are dropped here).
    pub fn build(
        window: &[UsageEvent],
        live: LiveView,
        now: i64,
        range: Range,
        filter: &str,
    ) -> Data {
        let (inside, _) = query::in_range(window, &[], now, range);
        let inside: Vec<UsageEvent> = inside.into_iter().filter(|e| matches(e, filter)).collect();
        Data {
            now,
            range,
            filter: filter.to_string(),
            totals: Totals::of(&inside),
            days: group_usage(&inside, Dimension::Day),
            models: group_usage(&inside, Dimension::Model),
            repos: group_usage(&inside, Dimension::Repo),
            live,
        }
    }
}

/// Reads the store and builds the snapshot.
pub fn load(conn: &Connection, range: Range, filter: &str, now: i64) -> Result<Data, String> {
    let window = query::load_window(conn, range, now)?;
    let live = query::load_live(conn, now)?;
    Ok(Data::build(&window, live, now, range, filter))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_788_400_000;

    fn event(ts: i64, model: &str, repo: &str, cost: f64) -> UsageEvent {
        UsageEvent {
            event_id: format!("{model}{repo}{ts}"),
            timestamp: ts,
            session_id: "s1".into(),
            model: model.into(),
            repo: repo.into(),
            branch: "main".into(),
            input_tokens: 10,
            output_tokens: 5,
            cost_usd: cost,
            ..UsageEvent::default()
        }
    }

    #[test]
    fn the_filter_matches_model_repo_branch_and_session_case_insensitively() {
        let e = event(NOW, "claude-Opus", "playbook", 1.0);
        for hit in ["opus", "PLAYBOOK", "main", "s1", ""] {
            assert!(matches(&e, hit), "{hit}");
        }
        assert!(!matches(&e, "sonnet"));
    }

    #[test]
    fn build_groups_the_range_and_applies_the_filter() {
        let events = vec![
            event(NOW - 60, "opus", "a", 2.0),
            event(NOW - 30, "sonnet", "b", 1.0),
            event(NOW - 100 * 86_400, "opus", "a", 50.0),
        ];
        let all = Data::build(&events, LiveView::default(), NOW, Range::LastDays(30), "");
        assert_eq!(all.totals.messages, 2);
        assert!((all.totals.cost_usd - 3.0).abs() < 1e-9);
        assert_eq!(all.models.len(), 2);
        assert_eq!(all.repos.len(), 2);

        let opus = Data::build(
            &events,
            LiveView::default(),
            NOW,
            Range::LastDays(30),
            "opus",
        );
        assert_eq!(opus.totals.messages, 1);
        assert_eq!(opus.models[0].key, "opus");
    }
}
