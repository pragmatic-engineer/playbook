// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The six panels, in key order.

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_panel_has_a_hint() {
        let hints: Vec<&str> = PANELS.iter().map(|p| p.hint()).collect();
        assert_eq!(hints, ["¹", "²", "³", "⁴", "⁵", "⁶"]);
    }
}
