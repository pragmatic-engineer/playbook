// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The terminal view's state and what each key does. Pure: no terminal, no
//! store, so every key is a unit test.

use super::data::Data;
use super::sort::{event_rows, group_rows, session_rows, Panel, Sort, PANELS};
use super::theme::{ColorDepth, Theme};
use super::view;
use crate::usage::query::Range;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};

/// The ranges `r` steps through.
pub const RANGES: [Range; 5] = [
    Range::LastDays(30),
    Range::LastDays(60),
    Range::LastDays(90),
    Range::CurrentMonth,
    Range::All,
];

#[derive(Debug)]
pub struct App {
    pub data: Option<Data>,
    pub range: Range,
    pub filter: String,
    /// The last store error, shown in the header until the next good refresh.
    pub error: Option<String>,
    pub quit: bool,
    pub theme: Theme,
    pub depth: ColorDepth,
    /// `NO_COLOR` is set: the theme stays `mono`.
    pub no_color: bool,
    /// True while the filter is being typed; keys then edit `draft`.
    pub editing: bool,
    pub draft: String,
    /// The panel the keys act on.
    pub focus: Panel,
    /// The selected row of each panel, by `Panel::index`.
    pub selected: [usize; 6],
    pub sorts: [Sort; 6],
    /// Whether the terminal reports the mouse.
    pub mouse: bool,
    /// The screen size, for turning a click into a panel and a row.
    pub area: Rect,
}

/// What the loop must do after a key.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    /// The range or filter changed: re-query now.
    Requery,
    /// `mouse` changed: tell the terminal.
    Mouse,
}

impl App {
    pub fn new(range: Range) -> App {
        App {
            data: None,
            range,
            filter: String::new(),
            error: None,
            quit: false,
            theme: Theme::default(),
            depth: ColorDepth::default(),
            no_color: false,
            editing: false,
            draft: String::new(),
            focus: Panel::Models,
            selected: [0; 6],
            sorts: PANELS.map(Panel::default_sort),
            mouse: true,
            area: Rect::new(0, 0, 100, 30),
        }
    }

    pub fn sort(&self, panel: Panel) -> Sort {
        self.sorts[panel.index()]
    }

    /// How many rows the panel's table has.
    fn rows(&self, panel: Panel) -> usize {
        let Some(d) = &self.data else { return 0 };
        match panel {
            Panel::Models => d.models.len(),
            Panel::Projects => d.repos.len(),
            Panel::Sessions => d.live.active.len(),
            Panel::Events => d.live.feed.len(),
            Panel::Spend | Panel::Tokens => 0,
        }
    }

    /// The selected row of `panel`, kept inside its rows.
    pub fn selection(&self, panel: Panel) -> usize {
        self.selected[panel.index()].min(self.rows(panel).saturating_sub(1))
    }

    fn select(&mut self, panel: Panel, to: usize) {
        self.selected[panel.index()] = to.min(self.rows(panel).saturating_sub(1));
    }

    fn step(&mut self, panel: Panel, by: isize) {
        let at = self.selection(panel) as isize;
        self.select(panel, (at + by).max(0) as usize);
    }

    /// The text that filters the view to the selected row: its model, project
    /// or session id.
    fn selected_key(&self) -> Option<String> {
        let d = self.data.as_ref()?;
        let panel = self.focus;
        let at = self.selection(panel);
        let sort = self.sort(panel);
        match panel {
            Panel::Models => group_rows(&d.models, sort).get(at).map(|g| g.key.clone()),
            Panel::Projects => group_rows(&d.repos, sort).get(at).map(|g| g.key.clone()),
            Panel::Sessions => session_rows(&d.live.active, sort)
                .get(at)
                .map(|s| s.id.clone()),
            Panel::Events => event_rows(&d.live.feed, sort)
                .get(at)
                .map(|e| e.session_id.clone()),
            Panel::Spend | Panel::Tokens => None,
        }
    }

    fn focus_next(&mut self, by: isize) {
        let at = self.focus.index() as isize;
        self.focus = PANELS[(at + by).rem_euclid(PANELS.len() as isize) as usize];
    }

    pub fn key(&mut self, key: KeyEvent) -> Effect {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.editing {
            return self.edit(key.code, ctrl);
        }
        let focus = self.focus;
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Char('t') if !self.no_color => self.theme = self.theme.next(),
            KeyCode::Char('m') => {
                self.mouse = !self.mouse;
                return Effect::Mouse;
            }
            KeyCode::Char('/') => {
                self.editing = true;
                self.draft = self.filter.clone();
            }
            KeyCode::Char('c') if !self.filter.is_empty() => {
                self.filter.clear();
                return Effect::Requery;
            }
            KeyCode::Char('r') => {
                let at = RANGES.iter().position(|r| *r == self.range).unwrap_or(0);
                self.range = RANGES[(at + 1) % RANGES.len()];
                return Effect::Requery;
            }
            KeyCode::Char(c @ '1'..='6') => self.focus = PANELS[usize::from(c as u8 - b'1')],
            KeyCode::Tab => self.focus_next(1),
            KeyCode::BackTab => self.focus_next(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step(focus, 1),
            KeyCode::Up | KeyCode::Char('k') => self.step(focus, -1),
            KeyCode::PageDown => self.step(focus, 5),
            KeyCode::PageUp => self.step(focus, -5),
            KeyCode::Home | KeyCode::Char('g') => self.select(focus, 0),
            KeyCode::End | KeyCode::Char('G') => self.select(focus, usize::MAX),
            KeyCode::Char('s') => self.sorts[focus.index()] = self.sort(focus).next(focus),
            KeyCode::Char('S') if !focus.sortable().is_empty() => {
                self.sorts[focus.index()] = self.sort(focus).flip();
            }
            KeyCode::Enter => {
                // Enter on the row already used as the filter lifts it.
                if let Some(key) = self.selected_key() {
                    self.filter = if self.filter == key {
                        String::new()
                    } else {
                        key
                    };
                    return Effect::Requery;
                }
            }
            _ => {}
        }
        Effect::None
    }

    /// The panel under a screen cell and the table row there, if it is one.
    fn hit(&self, x: u16, y: u16) -> Option<(Panel, Option<usize>)> {
        if self.area.width < view::MIN_WIDTH || self.area.height < view::MIN_HEIGHT {
            return None;
        }
        let p = view::layout(self.area);
        let rects = [
            p.spend, p.tokens, p.models, p.projects, p.sessions, p.events,
        ];
        let at = Position::new(x, y);
        let (panel, rect) = PANELS.iter().zip(rects).find(|(_, r)| r.contains(at))?;
        // Border, then the header row, then the rows.
        let first = rect.y + 2;
        let row = (!panel.sortable().is_empty() && y >= first)
            .then(|| {
                view::window_start(self.selection(*panel), view::visible_rows(rect))
                    + usize::from(y - first)
            })
            .filter(|r| *r < self.rows(*panel));
        Some((*panel, row))
    }

    /// A click focuses a panel and selects a row, the wheel moves the
    /// selection of the panel under the pointer.
    pub fn mouse(&mut self, ev: MouseEvent) {
        if self.editing {
            return;
        }
        let Some((panel, row)) = self.hit(ev.column, ev.row) else {
            return;
        };
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.focus = panel;
                if let Some(row) = row {
                    self.select(panel, row);
                }
            }
            MouseEventKind::ScrollDown => {
                self.focus = panel;
                self.step(panel, 1);
            }
            MouseEventKind::ScrollUp => {
                self.focus = panel;
                self.step(panel, -1);
            }
            _ => {}
        }
    }

    /// Keys while the filter is being typed. Enter applies, Esc drops it.
    fn edit(&mut self, code: KeyCode, ctrl: bool) -> Effect {
        match code {
            KeyCode::Enter => {
                self.editing = false;
                if self.draft != self.filter {
                    self.filter = std::mem::take(&mut self.draft);
                    return Effect::Requery;
                }
            }
            KeyCode::Esc => self.editing = false,
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Backspace => {
                self.draft.pop();
            }
            KeyCode::Char(c) if !ctrl => self.draft.push(c),
            _ => {}
        }
        Effect::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn q_esc_and_ctrl_c_quit() {
        for key in [
            press(KeyCode::Char('q')),
            press(KeyCode::Esc),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            let mut app = App::new(Range::All);
            app.key(key);
            assert!(app.quit);
        }
    }

    #[test]
    fn r_steps_through_every_range_and_wraps() {
        let mut app = App::new(Range::LastDays(30));
        let mut seen = Vec::new();
        for _ in 0..RANGES.len() {
            assert_eq!(app.key(press(KeyCode::Char('r'))), Effect::Requery);
            seen.push(app.range);
        }
        assert_eq!(seen.last(), Some(&Range::LastDays(30)));
        assert!(seen.contains(&Range::All));
    }

    fn type_filter(app: &mut App, text: &str) {
        app.key(press(KeyCode::Char('/')));
        for c in text.chars() {
            app.key(press(KeyCode::Char(c)));
        }
    }

    #[test]
    fn slash_types_a_filter_and_enter_applies_it() {
        let mut app = App::new(Range::All);
        type_filter(&mut app, "opus");
        assert!(app.editing);
        assert_eq!(app.filter, "");
        assert_eq!(app.key(press(KeyCode::Enter)), Effect::Requery);
        assert_eq!((app.editing, app.filter.as_str()), (false, "opus"));
    }

    #[test]
    fn typing_q_or_r_while_editing_is_text_not_a_command() {
        let mut app = App::new(Range::All);
        type_filter(&mut app, "qr");
        assert!(!app.quit);
        assert_eq!(app.range, Range::All);
        assert_eq!(app.draft, "qr");
    }

    #[test]
    fn backspace_edits_and_esc_cancels_without_a_requery() {
        let mut app = App::new(Range::All);
        type_filter(&mut app, "abc");
        app.key(press(KeyCode::Backspace));
        assert_eq!(app.draft, "ab");
        assert_eq!(app.key(press(KeyCode::Esc)), Effect::None);
        assert!(!app.editing && !app.quit);
        assert_eq!(app.filter, "");
    }

    #[test]
    fn enter_on_an_unchanged_filter_does_not_requery() {
        let mut app = App::new(Range::All);
        app.key(press(KeyCode::Char('/')));
        assert_eq!(app.key(press(KeyCode::Enter)), Effect::None);
    }

    #[test]
    fn editing_an_existing_filter_starts_from_it() {
        let mut app = App::new(Range::All);
        app.filter = "sonnet".into();
        app.key(press(KeyCode::Char('/')));
        assert_eq!(app.draft, "sonnet");
    }

    #[test]
    fn c_clears_a_filter_and_requeries_only_when_one_is_set() {
        let mut app = App::new(Range::All);
        assert_eq!(app.key(press(KeyCode::Char('c'))), Effect::None);
        app.filter = "x".into();
        assert_eq!(app.key(press(KeyCode::Char('c'))), Effect::Requery);
        assert_eq!(app.filter, "");
    }

    #[test]
    fn t_cycles_the_theme() {
        let mut app = App::new(Range::All);
        assert_eq!(app.theme, Theme::Btop);
        app.key(press(KeyCode::Char('t')));
        assert_eq!(app.theme, Theme::Dark);
    }

    #[test]
    fn no_color_keeps_the_theme_locked() {
        let mut app = App::new(Range::All);
        app.no_color = true;
        app.theme = Theme::Mono;
        app.key(press(KeyCode::Char('t')));
        assert_eq!(app.theme, Theme::Mono);
    }

    fn loaded() -> App {
        use crate::usage::query::LiveView;
        use crate::usage::UsageEvent;
        let ev = |id: &str, model: &str, repo: &str, cost: f64| UsageEvent {
            event_id: id.into(),
            timestamp: 1_788_400_000 - 30,
            session_id: format!("sess-{id}"),
            model: model.into(),
            repo: repo.into(),
            cost_usd: cost,
            ..UsageEvent::default()
        };
        let events = [
            ev("a", "opus", "alpha", 5.0),
            ev("b", "sonnet", "beta", 1.0),
            ev("c", "haiku", "gamma", 3.0),
        ];
        let live = crate::usage::query::live_view(&events, &events, &events, 1_788_400_000);
        let mut app = App::new(Range::All);
        app.data = Some(Data::build(&events, live, 1_788_400_000, Range::All, ""));
        let _ = LiveView::default();
        app
    }

    #[test]
    fn number_keys_and_tab_move_the_focus() {
        let mut app = App::new(Range::All);
        app.key(press(KeyCode::Char('5')));
        assert_eq!(app.focus, Panel::Sessions);
        app.key(press(KeyCode::Tab));
        assert_eq!(app.focus, Panel::Events);
        app.key(press(KeyCode::Tab));
        assert_eq!(app.focus, Panel::Spend);
        app.key(press(KeyCode::BackTab));
        assert_eq!(app.focus, Panel::Events);
        app.key(press(KeyCode::Char('7')));
        assert_eq!(app.focus, Panel::Events);
    }

    #[test]
    fn arrows_move_the_selection_and_stay_inside_the_rows() {
        let mut app = loaded();
        app.key(press(KeyCode::Char('3')));
        app.key(press(KeyCode::Down));
        app.key(press(KeyCode::Char('j')));
        assert_eq!(app.selection(Panel::Models), 2);
        app.key(press(KeyCode::Down));
        assert_eq!(app.selection(Panel::Models), 2);
        app.key(press(KeyCode::Up));
        assert_eq!(app.selection(Panel::Models), 1);
        app.key(press(KeyCode::Home));
        assert_eq!(app.selection(Panel::Models), 0);
        app.key(press(KeyCode::End));
        assert_eq!(app.selection(Panel::Models), 2);
    }

    #[test]
    fn s_sorts_the_focused_table_and_capital_s_reverses_it() {
        let mut app = loaded();
        app.key(press(KeyCode::Char('3')));
        assert_eq!(app.sort(Panel::Models), Sort::new(3, true));
        app.key(press(KeyCode::Char('s')));
        assert_eq!(app.sort(Panel::Models), Sort::new(0, false));
        app.key(press(KeyCode::Char('S')));
        assert_eq!(app.sort(Panel::Models), Sort::new(0, true));
        // Other panels keep their own sort.
        assert_eq!(app.sort(Panel::Projects), Sort::new(3, true));
        // A graph has nothing to sort.
        app.key(press(KeyCode::Char('1')));
        app.key(press(KeyCode::Char('S')));
        assert_eq!(app.sort(Panel::Spend), Panel::Spend.default_sort());
    }

    #[test]
    fn enter_filters_to_the_selected_row_and_again_lifts_it() {
        let mut app = loaded();
        app.key(press(KeyCode::Char('3')));
        app.key(press(KeyCode::Down));
        // Models by cost: opus 5, haiku 3, sonnet 1.
        assert_eq!(app.key(press(KeyCode::Enter)), Effect::Requery);
        assert_eq!(app.filter, "haiku");
        assert_eq!(app.key(press(KeyCode::Enter)), Effect::Requery);
        assert_eq!(app.filter, "");
    }

    #[test]
    fn enter_on_a_session_filters_by_its_id() {
        let mut app = loaded();
        app.key(press(KeyCode::Char('5')));
        assert_eq!(app.key(press(KeyCode::Enter)), Effect::Requery);
        assert!(app.filter.starts_with("sess-"), "{}", app.filter);
        // A graph has no row to pick.
        app.filter.clear();
        app.key(press(KeyCode::Char('1')));
        assert_eq!(app.key(press(KeyCode::Enter)), Effect::None);
    }

    #[test]
    fn m_toggles_the_mouse() {
        let mut app = App::new(Range::All);
        assert!(app.mouse);
        assert_eq!(app.key(press(KeyCode::Char('m'))), Effect::Mouse);
        assert!(!app.mouse);
    }

    fn click(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn a_click_focuses_a_panel_and_selects_the_row_under_it() {
        let mut app = loaded();
        let p = view::layout(app.area);
        let left = MouseEventKind::Down(MouseButton::Left);
        // Third model row: border, header, then the rows.
        app.mouse(click(left, p.models.x + 3, p.models.y + 4));
        assert_eq!(app.focus, Panel::Models);
        assert_eq!(app.selection(Panel::Models), 2);
        // The header row selects nothing but still focuses.
        app.mouse(click(left, p.projects.x + 3, p.projects.y + 1));
        assert_eq!(app.focus, Panel::Projects);
        assert_eq!(app.selection(Panel::Projects), 0);
    }

    #[test]
    fn the_wheel_moves_the_selection_of_the_panel_under_the_pointer() {
        let mut app = loaded();
        let p = view::layout(app.area);
        app.mouse(click(
            MouseEventKind::ScrollDown,
            p.projects.x + 3,
            p.projects.y + 3,
        ));
        assert_eq!(
            (app.focus, app.selection(Panel::Projects)),
            (Panel::Projects, 1)
        );
        app.mouse(click(
            MouseEventKind::ScrollUp,
            p.projects.x + 3,
            p.projects.y + 3,
        ));
        assert_eq!(app.selection(Panel::Projects), 0);
    }

    #[test]
    fn a_click_outside_every_panel_or_on_a_tiny_screen_does_nothing() {
        let mut app = loaded();
        app.mouse(click(MouseEventKind::Down(MouseButton::Left), 3, 0));
        assert_eq!(app.focus, Panel::Models);
        app.area = Rect::new(0, 0, 40, 10);
        app.mouse(click(MouseEventKind::Down(MouseButton::Left), 3, 5));
        assert_eq!(app.focus, Panel::Models);
    }

    #[test]
    fn an_unbound_key_does_nothing() {
        let mut app = App::new(Range::All);
        assert_eq!(app.key(press(KeyCode::Char('z'))), Effect::None);
        assert!(!app.quit);
    }
}
