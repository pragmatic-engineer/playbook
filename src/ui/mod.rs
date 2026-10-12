// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Terminal building blocks shared by `init`, `uninstall`, `update` and
//! `doctor`: what the terminal can show (`Caps`), pure functions that turn a
//! step, a choice, a summary or a progress line into text (`render`), and a
//! small question loop with back, skip and quit (`wizard`).
//!
//! Every renderer returns a `String` and takes the capabilities as an argument,
//! so tests compare plain text with colors off.

pub mod render;
pub mod wizard;

use std::io::IsTerminal;

/// What the output can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Use ANSI colors.
    pub color: bool,
    /// Use Unicode symbols. Off gives ASCII.
    pub unicode: bool,
    /// Columns to wrap to, never below `MIN_WIDTH`.
    pub width: usize,
}

/// The narrowest width text is wrapped to.
pub const MIN_WIDTH: usize = 40;
/// The widest width text is wrapped to, so lines stay readable.
pub const MAX_WIDTH: usize = 100;

impl Caps {
    /// Plain ASCII, no color, at `width` columns.
    pub fn plain(width: usize) -> Self {
        Self {
            color: false,
            unicode: false,
            width: width.clamp(MIN_WIDTH, MAX_WIDTH),
        }
    }

    /// Build from already-read facts. `env` looks up an environment variable.
    pub fn from_parts(
        env: &dyn Fn(&str) -> Option<String>,
        is_tty: bool,
        columns: Option<usize>,
    ) -> Self {
        let set = |k: &str| env(k).is_some_and(|v| !v.is_empty());
        let dumb = env("TERM").is_some_and(|t| t == "dumb");
        // Off Unix, only a terminal known to understand escapes gets color.
        let vt = cfg!(unix) || set("WT_SESSION") || set("TERM");
        let color = is_tty && !dumb && vt && !set("NO_COLOR");
        let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .find_map(|k| env(k).filter(|v| !v.is_empty()))
            .unwrap_or_default()
            .to_ascii_lowercase();
        let unicode = is_tty && !dumb && (locale.contains("utf-8") || locale.contains("utf8"));
        let env_cols = env("COLUMNS").and_then(|c| c.trim().parse::<usize>().ok());
        let width = columns.or(env_cols).unwrap_or(80);
        Self {
            color,
            unicode,
            width: width.clamp(MIN_WIDTH, MAX_WIDTH),
        }
    }

    /// Read the real environment and standard output.
    pub fn detect() -> Self {
        let tty = std::io::stdout().is_terminal();
        let env = |k: &str| std::env::var(k).ok();
        Self::from_parts(&env, tty, if tty { terminal_columns() } else { None })
    }
}

#[cfg(unix)]
fn terminal_columns() -> Option<usize> {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCGWINSZ writes one `winsize` through the pointer, which is valid for the call.
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) };
    (rc == 0 && size.ws_col > 0).then_some(usize::from(size.ws_col))
}

#[cfg(not(unix))]
fn terminal_columns() -> Option<usize> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn a_utf8_terminal_gets_color_and_unicode() {
        let c = Caps::from_parts(&env_of(&[("LANG", "en_US.UTF-8")]), true, Some(90));
        assert_eq!(
            c,
            Caps {
                color: true,
                unicode: true,
                width: 90
            }
        );
    }

    #[test]
    fn no_color_turns_color_off_but_keeps_symbols() {
        let c = Caps::from_parts(
            &env_of(&[("NO_COLOR", "1"), ("LANG", "en_US.UTF-8")]),
            true,
            None,
        );
        assert!(!c.color && c.unicode);
    }

    #[test]
    fn an_empty_no_color_is_ignored() {
        let c = Caps::from_parts(&env_of(&[("NO_COLOR", "")]), true, None);
        assert!(c.color);
    }

    #[test]
    fn a_dumb_terminal_gets_plain_ascii() {
        let c = Caps::from_parts(
            &env_of(&[("TERM", "dumb"), ("LANG", "en_US.UTF-8")]),
            true,
            None,
        );
        assert!(!c.color && !c.unicode);
    }

    #[test]
    fn a_pipe_gets_plain_ascii() {
        let c = Caps::from_parts(&env_of(&[("LANG", "en_US.UTF-8")]), false, None);
        assert!(!c.color && !c.unicode);
    }

    #[test]
    fn lc_all_beats_lang_and_columns_beat_env() {
        let c = Caps::from_parts(
            &env_of(&[("LC_ALL", "C"), ("LANG", "en_US.UTF-8"), ("COLUMNS", "70")]),
            true,
            Some(90),
        );
        assert!(!c.unicode);
        assert_eq!(c.width, 90);
        let bad = Caps::from_parts(&env_of(&[("COLUMNS", "abc")]), true, None);
        assert_eq!(bad.width, 80);
    }

    #[test]
    fn width_comes_from_columns_then_env_and_is_clamped() {
        assert_eq!(
            Caps::from_parts(&env_of(&[]), true, Some(10)).width,
            MIN_WIDTH
        );
        assert_eq!(
            Caps::from_parts(&env_of(&[]), true, Some(500)).width,
            MAX_WIDTH
        );
        assert_eq!(
            Caps::from_parts(&env_of(&[("COLUMNS", "70")]), true, None).width,
            70
        );
        assert_eq!(Caps::from_parts(&env_of(&[]), true, None).width, 80);
    }
}
