// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The terminal view's state and what each key does. Pure: no terminal, no
//! store, so every key is a unit test.

use super::data::Data;
use super::panel::{Panel, PANELS};
use super::theme::{ColorDepth, Theme};
use crate::usage::query::Range;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
}

/// What the loop must do after a key.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    /// The range or filter changed: re-query now.
    Requery,
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
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Char('t') if !self.no_color => self.theme = self.theme.next(),
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
            _ => {}
        }
        Effect::None
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
    fn an_unbound_key_does_nothing() {
        let mut app = App::new(Range::All);
        assert_eq!(app.key(press(KeyCode::Char('z'))), Effect::None);
        assert!(!app.quit);
    }
}
