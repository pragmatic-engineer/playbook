// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The panels, which of them are tables, and how a table's rows are sorted.
//! Pure, so `app` (which row is selected) and `panels` (what is drawn) agree.

use crate::usage::aggregate::Group;
use crate::usage::query::{total_tokens, ActiveSession};
use crate::usage::UsageEvent;
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Spend,
    Tokens,
    Models,
    Projects,
    Sessions,
    Events,
}

pub const PANELS: [Panel; 6] = [
    Panel::Spend,
    Panel::Tokens,
    Panel::Models,
    Panel::Projects,
    Panel::Sessions,
    Panel::Events,
];

impl Panel {
    pub fn index(self) -> usize {
        PANELS.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// The key that focuses this panel, shown as a superscript in its title.
    pub fn hint(self) -> &'static str {
        [
            "\u{b9}", "\u{b2}", "\u{b3}", "\u{2074}", "\u{2075}", "\u{2076}",
        ][self.index()]
    }

    /// Columns of the panel's table that `s` can sort by, as indexes into its
    /// column list. Empty for a graph.
    pub fn sortable(self) -> &'static [usize] {
        match self {
            Panel::Spend | Panel::Tokens => &[],
            Panel::Models | Panel::Projects => &[0, 1, 2, 3],
            Panel::Sessions => &[1, 2, 3, 4, 5],
            Panel::Events => &[0, 1, 2, 3, 4],
        }
    }

    pub fn default_sort(self) -> Sort {
        match self {
            Panel::Models | Panel::Projects => Sort::new(3, true),
            Panel::Sessions => Sort::new(5, false),
            Panel::Events => Sort::new(0, true),
            Panel::Spend | Panel::Tokens => Sort::new(0, false),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub col: usize,
    pub desc: bool,
}

impl Sort {
    pub const fn new(col: usize, desc: bool) -> Sort {
        Sort { col, desc }
    }

    /// The next sortable column of `panel`, wrapping. A new column starts in
    /// the direction that suits it: text ascending, numbers descending.
    pub fn next(self, panel: Panel) -> Sort {
        let cols = panel.sortable();
        let Some(at) = cols.iter().position(|c| *c == self.col) else {
            return self;
        };
        let col = cols[(at + 1) % cols.len()];
        Sort::new(col, !is_text(panel, col))
    }

    pub fn flip(self) -> Sort {
        Sort::new(self.col, !self.desc)
    }

    fn order(self, ord: Ordering) -> Ordering {
        if self.desc {
            ord.reverse()
        } else {
            ord
        }
    }
}

fn is_text(panel: Panel, col: usize) -> bool {
    match panel {
        Panel::Models | Panel::Projects => col == 0,
        Panel::Sessions => matches!(col, 1 | 2),
        Panel::Events => matches!(col, 1 | 2),
        Panel::Spend | Panel::Tokens => false,
    }
}

pub fn group_tokens(g: &Group) -> u64 {
    g.input_tokens + g.output_tokens + g.cache_creation_tokens + g.cache_read_tokens
}

pub fn group_rows(groups: &[Group], sort: Sort) -> Vec<&Group> {
    let mut rows: Vec<&Group> = groups.iter().collect();
    rows.sort_by(|a, b| {
        let by = match sort.col {
            0 => a.key.cmp(&b.key),
            1 => a.messages.cmp(&b.messages),
            2 => group_tokens(a).cmp(&group_tokens(b)),
            _ => a.cost_usd.total_cmp(&b.cost_usd),
        };
        sort.order(by).then_with(|| a.key.cmp(&b.key))
    });
    rows
}

/// Sessions. Column 5 is the age, so ascending shows the newest first.
pub fn session_rows(sessions: &[ActiveSession], sort: Sort) -> Vec<&ActiveSession> {
    let mut rows: Vec<&ActiveSession> = sessions.iter().collect();
    rows.sort_by(|a, b| {
        let by = match sort.col {
            1 => a.repo.cmp(&b.repo),
            2 => a.model.cmp(&b.model),
            3 => a.messages.cmp(&b.messages),
            4 => a.cost_usd.total_cmp(&b.cost_usd),
            _ => b.last.cmp(&a.last),
        };
        sort.order(by).then_with(|| a.id.cmp(&b.id))
    });
    rows
}

pub fn event_rows(events: &[UsageEvent], sort: Sort) -> Vec<&UsageEvent> {
    let mut rows: Vec<&UsageEvent> = events.iter().collect();
    rows.sort_by(|a, b| {
        let by = match sort.col {
            1 => a.repo.cmp(&b.repo),
            2 => a.model.cmp(&b.model),
            3 => total_tokens(a).cmp(&total_tokens(b)),
            4 => a.cost_usd.total_cmp(&b.cost_usd),
            _ => a.timestamp.cmp(&b.timestamp),
        };
        sort.order(by).then_with(|| a.event_id.cmp(&b.event_id))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(key: &str, messages: u64, cost: f64) -> Group {
        Group {
            key: key.into(),
            messages,
            input_tokens: messages * 10,
            output_tokens: 0,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            cost_usd: cost,
            unpriced: 0,
        }
    }

    fn keys(rows: &[&Group]) -> Vec<String> {
        rows.iter().map(|g| g.key.clone()).collect()
    }

    #[test]
    fn groups_sort_by_each_column_in_both_directions() {
        let g = [group("b", 1, 5.0), group("a", 9, 1.0), group("c", 4, 3.0)];
        assert_eq!(keys(&group_rows(&g, Sort::new(3, true))), ["b", "c", "a"]);
        assert_eq!(keys(&group_rows(&g, Sort::new(3, false))), ["a", "c", "b"]);
        assert_eq!(keys(&group_rows(&g, Sort::new(1, true))), ["a", "c", "b"]);
        assert_eq!(keys(&group_rows(&g, Sort::new(2, false))), ["b", "c", "a"]);
        assert_eq!(keys(&group_rows(&g, Sort::new(0, false))), ["a", "b", "c"]);
    }

    #[test]
    fn a_tie_falls_back_to_the_name() {
        let g = [group("b", 1, 2.0), group("a", 1, 2.0)];
        assert_eq!(keys(&group_rows(&g, Sort::new(3, true))), ["a", "b"]);
    }

    #[test]
    fn s_walks_the_sortable_columns_and_wraps() {
        let mut s = Panel::Models.default_sort();
        let mut seen = vec![s.col];
        for _ in 0..4 {
            s = s.next(Panel::Models);
            seen.push(s.col);
        }
        assert_eq!(seen, [3, 0, 1, 2, 3]);
        // Text starts ascending, numbers descending.
        assert!(!Sort::new(3, true).next(Panel::Models).desc);
        assert!(Sort::new(0, false).next(Panel::Models).desc);
    }

    #[test]
    fn a_graph_has_nothing_to_sort() {
        let s = Panel::Spend.default_sort();
        assert_eq!(s.next(Panel::Spend), s);
        assert!(Panel::Tokens.sortable().is_empty());
    }

    #[test]
    fn sessions_by_age_show_the_newest_first_when_ascending() {
        let s = |id: &str, last: i64| ActiveSession {
            id: id.into(),
            repo: "r".into(),
            agent: String::new(),
            model: "m".into(),
            last,
            messages: 1,
            cost_usd: 1.0,
            unpriced: 0,
        };
        let sessions = [s("old", 10), s("new", 90)];
        let rows = session_rows(&sessions, Panel::Sessions.default_sort());
        assert_eq!(rows[0].id, "new");
        let rows = session_rows(&sessions, Sort::new(5, true));
        assert_eq!(rows[0].id, "old");
    }

    #[test]
    fn events_default_to_newest_first() {
        let e = |id: &str, ts: i64, cost: f64| UsageEvent {
            event_id: id.into(),
            timestamp: ts,
            cost_usd: cost,
            ..UsageEvent::default()
        };
        let events = [e("a", 1, 9.0), e("b", 2, 1.0)];
        let by_time = event_rows(&events, Panel::Events.default_sort());
        assert_eq!(by_time[0].event_id, "b");
        let by_cost = event_rows(&events, Sort::new(4, true));
        assert_eq!(by_cost[0].event_id, "a");
    }

    #[test]
    fn every_panel_has_a_hint() {
        let hints: Vec<&str> = PANELS.iter().map(|p| p.hint()).collect();
        assert_eq!(hints, ["¹", "²", "³", "⁴", "⁵", "⁶"]);
    }
}
