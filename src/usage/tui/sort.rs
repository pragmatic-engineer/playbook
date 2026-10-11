// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The panels, which of them are tables, and how a table's rows are sorted.
//! Pure, so `app` (which row is selected) and `panels` (what is drawn) agree.

use super::data::DayPoint;
use crate::usage::aggregate::Group;
use crate::usage::query::{ActiveSession, SessionDay};
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Spend,
    Tokens,
    Models,
    Projects,
    /// Sessions with a message in the last 15 minutes.
    Live,
    /// Every session of the period, grouped by day.
    Sessions,
}

pub const PANELS: [Panel; 6] = [
    Panel::Spend,
    Panel::Tokens,
    Panel::Models,
    Panel::Projects,
    Panel::Live,
    Panel::Sessions,
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

    /// Whether the panel is a chart, which the left and right keys move a
    /// cursor along.
    pub fn is_chart(self) -> bool {
        matches!(self, Panel::Spend | Panel::Tokens)
    }

    /// Columns of the panel's table that `s` can sort by, as indexes into its
    /// column list. Empty for a graph. The spend panel is a graph until it is
    /// switched to its daily table, which `App` knows.
    pub fn sortable(self) -> &'static [usize] {
        match self {
            Panel::Tokens => &[],
            Panel::Spend => &[0, 1, 2, 3, 4],
            Panel::Models | Panel::Projects => &[0, 1, 2, 3],
            Panel::Live => &[1, 2, 3, 4, 5],
            Panel::Sessions => &[1, 2, 3, 4, 5, 6],
        }
    }

    pub fn default_sort(self) -> Sort {
        match self {
            Panel::Models | Panel::Projects => Sort::new(3, true),
            Panel::Live => Sort::new(5, false),
            // The dearest session of each day first.
            Panel::Sessions => Sort::new(6, true),
            // The daily table starts with the newest day.
            Panel::Spend => Sort::new(0, true),
            Panel::Tokens => Sort::new(0, false),
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
        Panel::Live => matches!(col, 1 | 2),
        Panel::Sessions => matches!(col, 1..=3),
        Panel::Spend => col == 4,
        Panel::Tokens => false,
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

/// The days of the daily table, up to and including today. Idle days stay, so
/// the table is a calendar.
pub fn day_rows(series: &[DayPoint], today: i64, sort: Sort) -> Vec<&DayPoint> {
    let mut rows: Vec<&DayPoint> = series.iter().filter(|d| d.day <= today).collect();
    rows.sort_by(|a, b| {
        let by = match sort.col {
            1 => a.messages.cmp(&b.messages),
            2 => a.tokens.cmp(&b.tokens),
            3 => a.cost_usd.total_cmp(&b.cost_usd),
            4 => a.top_model().unwrap_or("").cmp(b.top_model().unwrap_or("")),
            _ => a.day.cmp(&b.day),
        };
        sort.order(by).then_with(|| a.day.cmp(&b.day))
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

/// The sessions of the period grouped by day, newest day first whatever the
/// sort. The sort orders the sessions inside each day.
pub fn session_day_rows(rows: &[SessionDay], sort: Sort) -> Vec<&SessionDay> {
    let mut rows: Vec<&SessionDay> = rows.iter().collect();
    rows.sort_by(|a, b| {
        let by = match sort.col {
            1 => a.id.cmp(&b.id),
            2 => a.repo.cmp(&b.repo),
            3 => a.model.cmp(&b.model),
            4 => a.messages.cmp(&b.messages),
            5 => a.tokens.cmp(&b.tokens),
            _ => a.cost_usd.total_cmp(&b.cost_usd),
        };
        b.day
            .cmp(&a.day)
            .then_with(|| sort.order(by))
            .then_with(|| a.id.cmp(&b.id))
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
    fn the_token_graph_has_nothing_to_sort() {
        let s = Panel::Tokens.default_sort();
        assert_eq!(s.next(Panel::Tokens), s);
        assert!(Panel::Tokens.sortable().is_empty());
    }

    fn day(day: i64, cost: f64, messages: usize, model: &str) -> DayPoint {
        DayPoint {
            day,
            cost_usd: cost,
            tokens: (cost * 100.0) as u64,
            messages,
            models: vec![(model.to_string(), cost)],
        }
    }

    #[test]
    fn the_daily_table_sorts_by_each_column_and_skips_days_to_come() {
        let series = [
            day(10, 5.0, 2, "opus"),
            day(11, 1.0, 9, "haiku"),
            day(12, 3.0, 4, "sonnet"),
            day(13, 9.0, 1, "opus"),
        ];
        let days = |sort| {
            day_rows(&series, 12, sort)
                .iter()
                .map(|d| d.day)
                .collect::<Vec<_>>()
        };
        // Day 13 is after today, so it never shows.
        assert_eq!(days(Panel::Spend.default_sort()), [12, 11, 10]);
        assert_eq!(days(Sort::new(0, false)), [10, 11, 12]);
        assert_eq!(days(Sort::new(1, true)), [11, 12, 10]);
        assert_eq!(days(Sort::new(2, true)), [10, 12, 11]);
        assert_eq!(days(Sort::new(3, true)), [10, 12, 11]);
        assert_eq!(days(Sort::new(4, false)), [11, 10, 12]);
    }

    #[test]
    fn the_daily_table_walks_its_columns_with_text_ascending() {
        let mut s = Panel::Spend.default_sort();
        let mut seen = vec![s.col];
        for _ in 0..5 {
            s = s.next(Panel::Spend);
            seen.push(s.col);
        }
        assert_eq!(seen, [0, 1, 2, 3, 4, 0]);
        assert!(
            !Sort::new(3, true).next(Panel::Spend).desc,
            "model: ascending"
        );
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
        let rows = session_rows(&sessions, Panel::Live.default_sort());
        assert_eq!(rows[0].id, "new");
        let rows = session_rows(&sessions, Sort::new(5, true));
        assert_eq!(rows[0].id, "old");
    }

    fn day_row(day: i64, id: &str, cost: f64, messages: usize) -> SessionDay {
        SessionDay {
            day,
            id: id.into(),
            repo: "r".into(),
            model: "m".into(),
            messages,
            tokens: messages as u64 * 10,
            cost_usd: cost,
            unpriced: 0,
        }
    }

    #[test]
    fn sessions_stay_grouped_by_day_and_the_sort_orders_inside_each_day() {
        let rows = [
            day_row(10, "a", 1.0, 9),
            day_row(11, "b", 2.0, 1),
            day_row(11, "c", 5.0, 3),
            day_row(10, "d", 4.0, 2),
        ];
        let ids = |sort| {
            session_day_rows(&rows, sort)
                .iter()
                .map(|r| r.id.clone())
                .collect::<Vec<_>>()
        };
        // Newest day first, the dearest session first inside it.
        assert_eq!(ids(Panel::Sessions.default_sort()), ["c", "b", "d", "a"]);
        assert_eq!(ids(Sort::new(6, false)), ["b", "c", "a", "d"]);
        assert_eq!(ids(Sort::new(4, true)), ["c", "b", "a", "d"]);
        assert_eq!(ids(Sort::new(1, false)), ["b", "c", "a", "d"]);
    }

    #[test]
    fn every_panel_has_a_hint() {
        let hints: Vec<&str> = PANELS.iter().map(|p| p.hint()).collect();
        assert_eq!(hints, ["¹", "²", "³", "⁴", "⁵", "⁶"]);
    }
}
