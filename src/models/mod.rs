// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The model table: one preferred model per tier (the 5.5 generation) and the
//! previous generation to fall back to.
//!
//! Plugin files name a tier by alias (`haiku`, `sonnet`, `opus`) and never pin
//! a model id. Claude Code resolves each alias to the newest model the
//! provider offers. The fallback is Claude Code's own `--fallback-model`,
//! which switches for one turn when the preferred model is overloaded or
//! unavailable. `ccc` and `ccd` pass the chain from this table.

use std::fs;
use std::path::Path;

/// One tier of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tier {
    pub name: &'static str,
    /// The alias plugin files use.
    pub alias: &'static str,
    /// The 5.5 model.
    pub preferred: &'static str,
    /// The previous generation, used when `preferred` is unavailable.
    pub fallback: &'static str,
}

pub const TIERS: [Tier; 3] = [
    Tier {
        name: "fast",
        alias: "haiku",
        preferred: "claude-haiku-5-5",
        fallback: "claude-haiku-4-5",
    },
    Tier {
        name: "balanced",
        alias: "sonnet",
        preferred: "claude-sonnet-5-5",
        fallback: "claude-sonnet-5",
    },
    Tier {
        name: "deep",
        alias: "opus",
        preferred: "claude-opus-5-5",
        fallback: "claude-opus-5",
    },
];

/// The value for `--fallback-model`: the previous generation of each tier,
/// deepest first. Claude Code caps a chain at three models.
pub fn fallback_chain() -> String {
    [TIERS[2], TIERS[1], TIERS[0]]
        .iter()
        .map(|t| t.fallback)
        .collect::<Vec<_>>()
        .join(",")
}

/// Whether Claude Code already has a fallback the user chose: the flag in
/// `args`, or `fallbackModel` in the user settings file.
pub fn user_has_fallback(args: &[String], user_settings: &Path) -> bool {
    if args
        .iter()
        .any(|a| a == "--fallback-model" || a.starts_with("--fallback-model="))
    {
        return true;
    }
    fs::read_to_string(user_settings)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .is_some_and(|v| v.get("fallbackModel").is_some())
}

/// The flag pair the launcher adds, or `None` when the user chose their own.
pub fn launcher_flags(args: &[String], user_settings: &Path) -> Option<[String; 2]> {
    if user_has_fallback(args, user_settings) {
        return None;
    }
    Some(["--fallback-model".to_string(), fallback_chain()])
}

/// Every full model id in `text` (`claude-opus-5-5`, `claude-haiku-4-5`) that
/// is not in the table. Dated ids such as `claude-sonnet-4-5-20250929` count as
/// their undated form.
pub fn unknown_pins(text: &str) -> Vec<String> {
    let known: Vec<&str> = TIERS
        .iter()
        .flat_map(|t| [t.preferred, t.fallback])
        .collect();
    let mut found = Vec::new();
    for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        let Some(rest) = word.strip_prefix("claude-") else {
            continue;
        };
        let family = rest.split('-').next().unwrap_or("");
        if !["opus", "sonnet", "haiku"].contains(&family) {
            continue;
        }
        let mut parts: Vec<&str> = word.split('-').collect();
        while parts
            .last()
            .is_some_and(|p| p.len() == 8 && p.bytes().all(|b| b.is_ascii_digit()))
        {
            parts.pop();
        }
        let id = parts.join("-");
        if parts.len() > 2 && !known.contains(&id.as_str()) && !found.contains(&id) {
            found.push(id);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    #[test]
    fn every_preferred_model_is_5_5_and_every_fallback_is_older() {
        for t in TIERS {
            assert!(t.preferred.ends_with("-5-5"), "{}", t.preferred);
            assert!(!t.fallback.ends_with("-5-5"), "{}", t.fallback);
        }
    }

    #[test]
    fn the_chain_is_the_previous_generation_deepest_first_and_within_the_cap() {
        let chain = fallback_chain();
        assert_eq!(chain, "claude-opus-5,claude-sonnet-5,claude-haiku-4-5");
        assert!(chain.split(',').count() <= 3);
    }

    #[test]
    fn the_launcher_adds_the_flag_unless_the_user_chose_a_fallback() {
        let dir = scratch_dir("models-fallback");
        std::fs::create_dir_all(&dir).unwrap();
        let settings = dir.join("settings.json");
        let none: Vec<String> = vec![];
        let flags = launcher_flags(&none, &settings).unwrap();
        assert_eq!(flags[0], "--fallback-model");

        let own = vec!["--fallback-model=sonnet".to_string()];
        assert!(launcher_flags(&own, &settings).is_none());

        std::fs::write(&settings, r#"{"fallbackModel":["sonnet"]}"#).unwrap();
        assert!(launcher_flags(&none, &settings).is_none());
    }

    #[test]
    fn unknown_pins_finds_ids_outside_the_table_and_ignores_aliases() {
        assert!(
            unknown_pins("model: sonnet and claude-sonnet-5-5 and claude-haiku-4-5").is_empty()
        );
        assert_eq!(
            unknown_pins("use claude-opus-4-1-20250805 here"),
            vec!["claude-opus-4-1"]
        );
        assert_eq!(
            unknown_pins("claude-code and claude-plugin"),
            Vec::<String>::new()
        );
    }
}
