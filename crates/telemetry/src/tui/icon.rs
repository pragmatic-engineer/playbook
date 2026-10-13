// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The small TV shown before the title in the header. It lives only in the
//! TUI: it is not in `--help`, JSON or any CLI text. The icon sits in a block
//! of exactly three cells, so the title starts in the same column whether the
//! terminal draws the emoji one or two cells wide.

/// The TV emoji.
const TV: &str = "\u{1F4FA}";
/// Cells the icon block takes: the emoji plus padding.
const BLOCK_CELLS: usize = 3;

/// How the header icon is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Icon {
    /// No icon: plain text, for non-UTF-8, `NO_COLOR` or dumb terminals.
    #[default]
    Off,
    /// The terminal draws the emoji one cell wide.
    Narrow,
    /// The terminal draws the emoji two cells wide.
    Wide,
}

impl Icon {
    /// Picks the mode from the environment values. `width` is the optional
    /// `PBTOP_ICON_WIDTH` override (`1` or `2`); anything else means two.
    pub fn detect(
        no_color: bool,
        term: Option<&str>,
        locale: [Option<&str>; 3],
        width: Option<&str>,
    ) -> Self {
        let dumb = term.is_some_and(|t| t.eq_ignore_ascii_case("dumb"));
        if no_color || dumb || !utf8(locale) {
            return Self::Off;
        }
        match width.map(str::trim) {
            Some("1") => Self::Narrow,
            _ => Self::Wide,
        }
    }

    /// The text placed before the title: the emoji and padding to three
    /// cells, or nothing.
    pub fn prefix(self) -> String {
        let cells = match self {
            Self::Off => return String::new(),
            Self::Narrow => 1,
            Self::Wide => 2,
        };
        let pad = BLOCK_CELLS - cells;
        format!("{TV}{}", " ".repeat(pad))
    }
}

/// The first of `LC_ALL`, `LC_CTYPE`, `LANG` that is set and not empty names
/// the locale; it must say UTF-8.
fn utf8(locale: [Option<&str>; 3]) -> bool {
    locale
        .into_iter()
        .flatten()
        .find(|v| !v.is_empty())
        .is_some_and(|v| {
            let v = v.to_ascii_lowercase();
            v.contains("utf-8") || v.contains("utf8")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const UTF8: [Option<&str>; 3] = [None, None, Some("en_US.UTF-8")];

    #[test]
    fn wide_mode_pads_one_cell() {
        let i = Icon::detect(false, Some("xterm"), UTF8, None);
        assert_eq!(i, Icon::Wide);
        assert_eq!(i.prefix(), "\u{1F4FA} ");
    }

    #[test]
    fn narrow_mode_pads_two_cells() {
        let i = Icon::detect(false, Some("xterm"), UTF8, Some("1"));
        assert_eq!(i, Icon::Narrow);
        assert_eq!(i.prefix(), "\u{1F4FA}  ");
    }

    #[test]
    fn both_modes_fill_the_same_block() {
        // Emoji cells plus pad cells is three in each mode.
        assert_eq!(Icon::Wide.prefix().chars().count(), 2);
        assert_eq!(Icon::Narrow.prefix().chars().count(), 3);
    }

    #[test]
    fn falls_back_to_plain_text() {
        assert_eq!(Icon::detect(true, None, UTF8, None), Icon::Off);
        assert_eq!(Icon::detect(false, Some("dumb"), UTF8, None), Icon::Off);
        let latin = [None, None, Some("en_US.ISO-8859-1")];
        assert_eq!(Icon::detect(false, None, latin, None), Icon::Off);
        assert_eq!(Icon::detect(false, None, [None; 3], None), Icon::Off);
        assert_eq!(Icon::Off.prefix(), "");
    }

    #[test]
    fn locale_precedence_is_all_then_ctype_then_lang() {
        let l = [Some("C"), None, Some("en_US.UTF-8")];
        assert_eq!(Icon::detect(false, None, l, None), Icon::Off);
        let l = [Some(""), None, Some("en_US.utf8")];
        assert_eq!(Icon::detect(false, None, l, None), Icon::Wide);
    }
}
