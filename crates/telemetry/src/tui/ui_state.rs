// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! What pbtop remembers between runs: the selected tab, the range, the chart
//! and breakdown views, the sort of each table and the filter. Only the
//! screen setup is stored, never data. The value is one line of JSON in the
//! `usage.ui.state` config key (app tier), written by pbtop. A missing,
//! unreadable or odd value gives the defaults; a bad part is dropped alone.

use super::app::{ProjectsView, SpendView};
use super::sort::{Panel, Sort, PANELS};
use serde_json::{json, Value};
use std::path::Path;

/// The key that holds the state.
pub const KEY: &str = "usage.ui.state";

/// The longest filter that is stored, in characters.
const MAX_FILTER: usize = 64;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiState {
    /// The selected tab, by name so a reordered layout keeps the choice.
    pub tab: Option<String>,
    /// A range name such as `week` or `30d`.
    pub range: Option<String>,
    pub spend_view: Option<SpendView>,
    pub projects_view: Option<ProjectsView>,
    /// Column and direction of each table, in `PANELS` order.
    pub sorts: Vec<Option<Sort>>,
    pub filter: Option<String>,
}

impl UiState {
    pub fn to_text(&self) -> String {
        let sorts: Vec<Value> = PANELS
            .iter()
            .enumerate()
            .map(|(i, _)| match self.sorts.get(i).copied().flatten() {
                Some(s) => json!([s.col, s.desc]),
                None => Value::Null,
            })
            .collect();
        json!({
            "tab": self.tab,
            "range": self.range,
            "spend": self.spend_view.map(|v| v.name()),
            "projects": self.projects_view.map(|v| v.name()),
            "sorts": sorts,
            "filter": self.filter,
        })
        .to_string()
    }

    /// Reads a stored value. Never fails: whatever cannot be read is left out.
    pub fn parse(text: &str) -> UiState {
        let Ok(doc) = serde_json::from_str::<Value>(text) else {
            return UiState::default();
        };
        let word = |key: &str| doc.get(key).and_then(Value::as_str).map(str::to_string);
        let sorts = doc
            .get("sorts")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .take(PANELS.len())
                    .map(|item| {
                        let pair = item.as_array()?;
                        let col = usize::try_from(pair.first()?.as_u64()?).ok()?;
                        Some(Sort::new(col, pair.get(1)?.as_bool()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        UiState {
            tab: word("tab").filter(|t| !t.is_empty()),
            range: word("range"),
            spend_view: word("spend").and_then(|n| find_spend(&n)),
            projects_view: word("projects").and_then(|n| find_projects(&n)),
            sorts,
            filter: word("filter").map(|f| f.chars().take(MAX_FILTER).collect()),
        }
    }

    /// The sort stored for `panel`, when it is one the panel accepts.
    pub fn sort_for(&self, panel: Panel, compared: bool) -> Option<Sort> {
        let sort = self.sorts.get(panel.index()).copied().flatten()?;
        panel.sortable(compared).contains(&sort.col).then_some(sort)
    }
}

/// The view named `name`, found by stepping `next` once around the cycle.
fn find_view<T: Copy + PartialEq>(
    start: T,
    next: impl Fn(T) -> T,
    name: impl Fn(T) -> &'static str,
    wanted: &str,
) -> Option<T> {
    let mut view = start;
    loop {
        if name(view) == wanted {
            return Some(view);
        }
        view = next(view);
        if view == start {
            return None;
        }
    }
}

fn find_spend(wanted: &str) -> Option<SpendView> {
    find_view(
        SpendView::default(),
        SpendView::next,
        SpendView::name,
        wanted,
    )
}

fn find_projects(wanted: &str) -> Option<ProjectsView> {
    find_view(
        ProjectsView::default(),
        ProjectsView::next,
        ProjectsView::name,
        wanted,
    )
}

/// The stored state for the user at `home`.
pub fn load(home: &Path) -> UiState {
    crate::config::resolve(home, KEY)
        .and_then(|v| v.as_str().map(UiState::parse))
        .unwrap_or_default()
}

/// Stores `state`. Quiet on failure: losing the screen setup must never
/// disturb the view.
pub fn save(home: &Path, state: &UiState) {
    let _ = crate::config::save_app(home, KEY, &Value::from(state.to_text()));
}

/// Stores the layout text; empty text goes back to the default tabs.
/// `Ok(false)` means there is no config store to write to; the layout then
/// lasts only for this run.
pub fn save_layout(home: &Path, text: &str) -> Result<bool, String> {
    crate::config::save_app(home, "usage.ui.layout", &Value::from(text))
}

/// `text` as one single-quoted shell word.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// What to tell the user after a layout save, `None` when nothing needs saying.
pub fn save_notice(result: Result<bool, String>, text: &str) -> Option<String> {
    match result {
        Ok(true) => None,
        // The default layout needs no stored value.
        Ok(false) if text.is_empty() => None,
        Ok(false) => Some(format!(
            "Layout kept for this run only. To keep it, run: playbook config set --global usage.ui.layout {}",
            shell_quote(text)
        )),
        Err(e) => Some(e),
    }
}

/// The stored layout text and the layout it gives, with a message when the
/// stored text is invalid.
pub fn load_layout(home: &Path) -> (super::layout::Layout, Option<String>) {
    let stored = crate::config::stored_raw(home, "usage.ui.layout");
    super::layout::Layout::from_stored(stored.as_ref().and_then(Value::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> UiState {
        UiState {
            tab: Some("Numbers".into()),
            range: Some("30d".into()),
            spend_view: Some(SpendView::Daily),
            projects_view: Some(ProjectsView::Agents),
            sorts: vec![None, None, Some(Sort::new(1, false)), None, None, None],
            filter: Some("alpha".into()),
        }
    }

    #[test]
    fn the_state_round_trips_through_its_text() {
        let state = sample();

        let back = UiState::parse(&state.to_text());

        assert_eq!(back, state);
    }

    #[test]
    fn text_that_is_not_json_gives_the_defaults() {
        assert_eq!(UiState::parse("not json"), UiState::default());
        assert_eq!(UiState::parse(""), UiState::default());
        assert_eq!(UiState::parse("[1,2]"), UiState::default());
    }

    #[test]
    fn a_bad_part_is_dropped_and_the_rest_is_kept() {
        let state = UiState::parse(r#"{"tab":"Charts","spend":"nope","sorts":"x","range":5}"#);

        assert_eq!(state.tab.as_deref(), Some("Charts"));
        assert_eq!(state.spend_view, None);
        assert_eq!(state.range, None);
        assert!(state.sorts.is_empty());
    }

    #[test]
    fn a_sort_column_the_panel_does_not_have_is_ignored() {
        let mut state = sample();
        state.sorts[Panel::Models.index()] = Some(Sort::new(99, true));

        assert_eq!(state.sort_for(Panel::Models, false), None);
    }

    #[test]
    fn a_long_filter_is_cut() {
        let long = "x".repeat(500);
        let text = json!({ "filter": long }).to_string();

        let state = UiState::parse(&text);

        assert_eq!(state.filter.unwrap().chars().count(), MAX_FILTER);
    }

    #[test]
    fn a_fresh_home_loads_the_defaults() {
        let home = crate::util::scratch_dir("ui-fresh");

        assert_eq!(load(&home), UiState::default());
    }

    #[test]
    fn saving_without_a_config_store_does_nothing_and_does_not_fail() {
        let home = crate::util::scratch_dir("ui-nostore");

        save(&home, &sample());

        assert_eq!(load(&home), UiState::default());
    }

    #[test]
    fn a_saved_state_is_restored_on_the_next_load() {
        let home = crate::util::scratch_dir("ui-roundtrip");
        let data = crate::config::root(&home).join("data");
        std::fs::create_dir_all(&data).unwrap();
        let db = playbook_data::config::db_in(&data);
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(playbook_data::config::SCHEMA_SQL)
            .unwrap();
        drop(conn);

        save(&home, &sample());

        assert_eq!(load(&home), sample());
    }

    #[test]
    fn an_invalid_stored_layout_falls_back_to_the_default_layout_and_says_why() {
        let home = crate::util::scratch_dir("ui-layout");
        let data = crate::config::root(&home).join("data");
        std::fs::create_dir_all(&data).unwrap();
        let db = playbook_data::config::db_in(&data);
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(playbook_data::config::SCHEMA_SQL)
            .unwrap();
        conn.execute(
            "INSERT INTO config VALUES ('global', '', 'usage.ui.layout', '\"A=bogus\"', 0)",
            [],
        )
        .unwrap();
        drop(conn);

        let (layout, warning) = load_layout(&home);

        assert_eq!(layout, super::super::layout::Layout::default());
        assert!(warning.is_some_and(|w| w.contains("bogus")));
    }

    fn store(tag: &str) -> std::path::PathBuf {
        let home = crate::util::scratch_dir(tag);
        let data = crate::config::root(&home).join("data");
        std::fs::create_dir_all(&data).unwrap();
        let conn = rusqlite::Connection::open(playbook_data::config::db_in(&data)).unwrap();
        conn.execute_batch(playbook_data::config::SCHEMA_SQL)
            .unwrap();
        home
    }

    #[test]
    fn a_saved_layout_is_read_back_on_the_next_load() {
        let home = store("ui-save-layout");

        let wrote = save_layout(&home, "Money=budget;Live=live").unwrap();
        let (layout, warning) = load_layout(&home);

        assert!(wrote);
        assert!(warning.is_none());
        assert_eq!(layout.to_text(), "Money=budget;Live=live");
    }

    #[test]
    fn resetting_the_layout_clears_the_saved_layout() {
        let home = store("ui-reset-layout");
        save_layout(&home, "Money=budget").unwrap();

        save_layout(&home, "").unwrap();

        assert_eq!(
            load_layout(&home).0,
            super::super::layout::Layout::default()
        );
    }

    #[test]
    fn the_no_store_notice_quotes_a_tab_name_with_a_single_quote() {
        let notice = save_notice(Ok(false), "Bob's=spend").unwrap();

        assert!(notice.ends_with("'Bob'\\''s=spend'"), "{notice}");
    }

    #[test]
    fn the_save_notice_is_silent_on_success_and_for_a_reset_without_a_store() {
        assert_eq!(save_notice(Ok(true), "A=spend"), None);
        assert_eq!(save_notice(Ok(false), ""), None);
        assert_eq!(
            save_notice(Err("boom".into()), "A=spend"),
            Some("boom".into())
        );
    }

    #[test]
    fn saving_a_layout_without_a_config_store_reports_that_nothing_was_written() {
        let home = crate::util::scratch_dir("ui-save-nostore");

        let wrote = save_layout(&home, "Money=budget").unwrap();

        assert!(!wrote);
    }
}
