// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Colors for the terminal view. `t` cycles the themes. `Mono` uses no color
//! at all, for terminals and pipes that cannot show it.

use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    Mono,
}

/// Every color the view uses, by role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub accent: Color,
    pub tokens: Color,
    pub live: Color,
    pub warn: Color,
    pub error: Color,
}

impl Theme {
    pub fn next(self) -> Theme {
        match self {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Mono,
            Theme::Mono => Theme::Dark,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Mono => "mono",
        }
    }

    pub fn palette(self) -> Palette {
        match self {
            Theme::Dark => Palette {
                accent: Color::Cyan,
                tokens: Color::Magenta,
                live: Color::Green,
                warn: Color::Yellow,
                error: Color::Red,
            },
            // Bright colors vanish on a white ground, so use the dark ones.
            Theme::Light => Palette {
                accent: Color::Blue,
                tokens: Color::Magenta,
                live: Color::Green,
                warn: Color::Rgb(160, 100, 0),
                error: Color::Red,
            },
            Theme::Mono => Palette {
                accent: Color::Reset,
                tokens: Color::Reset,
                live: Color::Reset,
                warn: Color::Reset,
                error: Color::Reset,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_visits_every_theme_and_wraps() {
        let mut t = Theme::Dark;
        let mut seen = vec![t];
        for _ in 0..3 {
            t = t.next();
            seen.push(t);
        }
        assert_eq!(seen, [Theme::Dark, Theme::Light, Theme::Mono, Theme::Dark]);
    }

    #[test]
    fn mono_sets_no_color() {
        let p = Theme::Mono.palette();
        for c in [p.accent, p.tokens, p.live, p.warn, p.error] {
            assert_eq!(c, Color::Reset);
        }
    }
}
