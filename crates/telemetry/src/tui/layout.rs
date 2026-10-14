// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The tab layout of pbtop: which tabs exist and which panels each one shows.
//! The layout is a plain text line in the `usage.ui.layout` config key, for
//! example `Charts=spend,tokens;Numbers=models,breakdown,budget`. Anything
//! that does not parse falls back to the default layout, so a bad value never
//! stops pbtop from starting.

/// A panel that can be placed on a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelId {
    Spend,
    Tokens,
    Models,
    Breakdown,
    Live,
    Sessions,
    Budget,
}

/// Every panel, in the order the layout editor lists them.
pub const ALL: [PanelId; 7] = [
    PanelId::Spend,
    PanelId::Tokens,
    PanelId::Models,
    PanelId::Breakdown,
    PanelId::Live,
    PanelId::Sessions,
    PanelId::Budget,
];

impl PanelId {
    /// The word used in the config value.
    pub fn word(self) -> &'static str {
        match self {
            PanelId::Spend => "spend",
            PanelId::Tokens => "tokens",
            PanelId::Models => "models",
            PanelId::Breakdown => "breakdown",
            PanelId::Live => "live",
            PanelId::Sessions => "sessions",
            PanelId::Budget => "budget",
        }
    }

    /// One line saying what the panel shows.
    pub fn describe(self) -> &'static str {
        match self {
            PanelId::Spend => "cost per day as a chart or a daily table",
            PanelId::Tokens => "tokens per day as a chart",
            PanelId::Models => "cost and tokens per model",
            PanelId::Breakdown => "projects, agents, skills, tools or health (v switches)",
            PanelId::Live => "sessions with a message in the last 15 minutes",
            PanelId::Sessions => "every session of the period, grouped by day",
            PanelId::Budget => "limits and how fast you are using them",
        }
    }

    pub fn parse(word: &str) -> Option<PanelId> {
        let word = word.trim().to_ascii_lowercase();
        ALL.into_iter().find(|p| p.word() == word)
    }
}

/// One tab: a name and its panels, top left to bottom right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub name: String,
    pub panels: Vec<PanelId>,
}

/// The most tabs, so each has a function key and the tab bar fits.
pub const MAX_TABS: usize = 9;
/// The longest tab name, in characters.
pub const MAX_NAME: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub tabs: Vec<Tab>,
}

impl Default for Layout {
    fn default() -> Layout {
        let tab = |name: &str, panels: &[PanelId]| Tab {
            name: name.to_string(),
            panels: panels.to_vec(),
        };
        Layout {
            tabs: vec![
                tab("Charts", &[PanelId::Spend, PanelId::Tokens]),
                tab(
                    "Numbers",
                    &[PanelId::Models, PanelId::Breakdown, PanelId::Budget],
                ),
                tab("Sessions", &[PanelId::Live, PanelId::Sessions]),
            ],
        }
    }
}

impl Layout {
    /// Reads `Name=panel,panel;Name=panel`. The error says what is wrong.
    pub fn parse(text: &str) -> Result<Layout, String> {
        let mut tabs: Vec<Tab> = Vec::new();
        for part in text.split(';').filter(|p| !p.trim().is_empty()) {
            let (name, list) = part
                .split_once('=')
                .ok_or_else(|| format!("{:?} needs a name, `=` and panels", part.trim()))?;
            let name = name.trim();
            if name.is_empty() || name.chars().count() > MAX_NAME {
                return Err(format!("a tab name needs 1 to {MAX_NAME} characters"));
            }
            if name.chars().any(char::is_control) {
                return Err("a tab name cannot hold control characters".to_string());
            }
            if tabs.iter().any(|t| t.name.eq_ignore_ascii_case(name)) {
                return Err(format!("the tab `{name}` appears twice"));
            }
            let mut panels = Vec::new();
            for word in list.split(',').filter(|w| !w.trim().is_empty()) {
                let panel = PanelId::parse(word).ok_or_else(|| {
                    format!(
                        "{:?} is not a panel; use {}",
                        word.trim(),
                        ALL.map(PanelId::word).join(", ")
                    )
                })?;
                if panels.contains(&panel) {
                    return Err(format!(
                        "the panel `{}` appears twice in `{name}`",
                        panel.word()
                    ));
                }
                panels.push(panel);
            }
            if panels.is_empty() {
                return Err(format!("the tab `{name}` has no panels"));
            }
            tabs.push(Tab {
                name: name.to_string(),
                panels,
            });
        }
        if tabs.is_empty() {
            return Err("the layout has no tabs".to_string());
        }
        if tabs.len() > MAX_TABS {
            return Err(format!("at most {MAX_TABS} tabs"));
        }
        Ok(Layout { tabs })
    }

    /// The text form that `parse` reads back.
    pub fn to_text(&self) -> String {
        self.tabs
            .iter()
            .map(|t| {
                let words: Vec<&str> = t.panels.iter().map(|p| p.word()).collect();
                format!("{}={}", t.name, words.join(","))
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    /// The layout from a stored value. An empty or missing value is the
    /// default without a message; an invalid one is the default and a reason.
    pub fn from_stored(text: Option<&str>) -> (Layout, Option<String>) {
        match text.map(str::trim).filter(|t| !t.is_empty()) {
            None => (Layout::default(), None),
            Some(t) => match Layout::parse(t) {
                Ok(layout) => (layout, None),
                Err(why) => (
                    Layout::default(),
                    Some(format!("ignoring usage.ui.layout: {why}")),
                ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_layout_has_the_charts_numbers_and_sessions_tabs() {
        let layout = Layout::default();

        let names: Vec<&str> = layout.tabs.iter().map(|t| t.name.as_str()).collect();

        assert_eq!(names, ["Charts", "Numbers", "Sessions"]);
    }

    #[test]
    fn a_layout_round_trips_through_its_text() {
        let layout = Layout::default();

        let back = Layout::parse(&layout.to_text()).unwrap();

        assert_eq!(back, layout);
    }

    #[test]
    fn a_custom_layout_keeps_its_order_and_names() {
        let layout = Layout::parse("Money = budget , models ; Live=live").unwrap();

        assert_eq!(layout.tabs[0].name, "Money");
        assert_eq!(layout.tabs[0].panels, [PanelId::Budget, PanelId::Models]);
        assert_eq!(layout.tabs[1].panels, [PanelId::Live]);
    }

    #[test]
    fn invalid_layouts_are_refused_with_a_reason() {
        for bad in [
            "",
            "NoPanels=",
            "A=nothing",
            "A=spend,spend",
            "A=spend;a=tokens",
            "just text",
            "=spend",
            "ThisNameIsFarTooLongToFit=spend",
        ] {
            assert!(Layout::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn more_than_nine_tabs_are_refused() {
        let text: Vec<String> = (0..10).map(|i| format!("T{i}=spend")).collect();

        assert!(Layout::parse(&text.join(";")).is_err());
    }

    #[test]
    fn a_fresh_start_uses_the_default_without_a_message() {
        let (layout, warning) = Layout::from_stored(None);

        assert_eq!(layout, Layout::default());
        assert!(warning.is_none());
    }

    #[test]
    fn an_invalid_stored_value_falls_back_to_the_default_with_a_message() {
        let (layout, warning) = Layout::from_stored(Some("A=bogus"));

        assert_eq!(layout, Layout::default());
        assert!(warning.unwrap().contains("bogus"));
    }
}
