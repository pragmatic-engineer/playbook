// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The terminal view's state and what each key does. Pure: no terminal, no
//! store, so every key is a unit test.

use super::data::Data;
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
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Effect {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Char('r') => {
                let at = RANGES.iter().position(|r| *r == self.range).unwrap_or(0);
                self.range = RANGES[(at + 1) % RANGES.len()];
                return Effect::Requery;
            }
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

    #[test]
    fn an_unbound_key_does_nothing() {
        let mut app = App::new(Range::All);
        assert_eq!(app.key(press(KeyCode::Char('z'))), Effect::None);
        assert!(!app.quit);
    }
}
