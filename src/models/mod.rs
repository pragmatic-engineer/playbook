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

use crate::config;
use crate::effort;
use serde_json::Value;
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

/// Levels the previous generation Sonnet and Opus reject with HTTP 400
/// `Invalid effort level` (measured on Claude Code 2.1.293, 2026-10-09). Haiku
/// 4.5 accepts them.
const OLD_GEN_REJECTS: [&str; 2] = ["xhigh", "max"];

/// The value for `--fallback-model`: the previous generation of each tier,
/// deepest first. Claude Code caps a chain at three models.
#[cfg(test)]
fn fallback_chain() -> String {
    fallback_chain_for(None)
}

/// The chain for a session running at `effort`. At `xhigh` or `max` the
/// previous Sonnet and Opus would answer 400 and Claude Code would then skip
/// them one by one, so the chain starts at the model that accepts the level.
fn fallback_chain_for(effort: Option<&str>) -> String {
    let rejects = effort.is_some_and(|e| OLD_GEN_REJECTS.contains(&e));
    [TIERS[2], TIERS[1], TIERS[0]]
        .iter()
        .filter(|t| !(rejects && t.alias != "haiku"))
        .map(|t| t.fallback)
        .collect::<Vec<_>>()
        .join(",")
}

/// The model id `models.<alias>` overrides, if one is set and valid.
fn override_for(alias: &str, home: &Path) -> Option<String> {
    let key = format!("models.{alias}");
    match config::resolve_valid(&key, home, None) {
        Ok((Value::String(id), _, None)) if !id.is_empty() => Some(id),
        _ => None,
    }
}

/// `ANTHROPIC_DEFAULT_<ALIAS>_MODEL` pairs for each tier with an override.
/// Claude Code reads these to resolve `haiku`, `sonnet` and `opus`, so the
/// override reaches subagents and skills that name the alias. A variable the
/// user already exported is left alone.
pub fn env_overrides(home: &Path) -> Vec<(String, String)> {
    TIERS
        .iter()
        .filter_map(|t| {
            let var = format!("ANTHROPIC_DEFAULT_{}_MODEL", t.alias.to_uppercase());
            if std::env::var_os(&var).is_some() {
                return None;
            }
            override_for(t.alias, home).map(|id| (var, id))
        })
        .collect()
}

/// The effort a session will run at, when it is stated: `--effort` in `args`,
/// else `effortLevel` in the user settings, lowered to the effective ceiling.
/// `None` when nothing states one, since the model default then applies.
pub fn effort_in_force(
    args: &[String],
    user_settings: &Path,
    ceiling: Option<&str>,
) -> Option<String> {
    let from_args = args.iter().enumerate().find_map(|(i, a)| {
        a.strip_prefix("--effort=").map(str::to_string).or_else(|| {
            (a == "--effort")
                .then(|| args.get(i + 1).cloned())
                .flatten()
        })
    });
    let stated = from_args.or_else(|| {
        let v: Value = serde_json::from_str(&fs::read_to_string(user_settings).ok()?).ok()?;
        v.get("effortLevel")?.as_str().map(str::to_string)
    })?;
    Some(
        effort::lower(Some(&stated), ceiling)
            .unwrap_or(&stated)
            .to_string(),
    )
}

/// Whether Claude Code already has a fallback the user chose: the flag in
/// `args`, or `fallbackModel` in the user settings file.
fn user_has_fallback(args: &[String], user_settings: &Path) -> bool {
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
/// `effort` is the level the session runs at, if known.
pub fn launcher_flags(
    args: &[String],
    user_settings: &Path,
    effort: Option<&str>,
) -> Option<[String; 2]> {
    if user_has_fallback(args, user_settings) {
        return None;
    }
    Some(["--fallback-model".to_string(), fallback_chain_for(effort)])
}

/// What `playbook doctor models` reports.
pub fn report(home: &Path, claude_home: &Path, cwd: &Path, json: bool) -> String {
    let settings = claude_home.join("settings.json");
    let ceiling = effort::ceiling(home, claude_home, cwd);
    let effort = effort_in_force(&[], &settings, ceiling.effective.as_deref());
    let user_fallback = user_has_fallback(&[], &settings);
    let chain = fallback_chain_for(effort.as_deref());
    let tiers: Vec<Value> = TIERS
        .iter()
        .map(|t| {
            serde_json::json!({
                "tier": t.name,
                "alias": t.alias,
                "preferred": t.preferred,
                "fallback": t.fallback,
                "override": override_for(t.alias, home),
            })
        })
        .collect();
    if json {
        return serde_json::json!({
            "tiers": tiers,
            "chain": chain,
            "chainSource": if user_fallback { "user" } else { "playbook" },
            "effort": effort,
            "haikuCap": {
                "default": effort::model_cap::HAIKU_DEFAULT_CAP,
                "ceiling": effort::model_cap::HAIKU_HARD_CEILING,
                "optIns": effort::model_cap::OPT_INS
                    .iter()
                    .map(|(name, level, reason)| serde_json::json!({
                        "component": name, "level": level, "reason": reason
                    }))
                    .collect::<Vec<_>>(),
            },
        })
        .to_string();
    }
    let mut out = String::new();
    for t in TIERS {
        let over = override_for(t.alias, home)
            .map(|id| format!(", override {id}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "{} ({}): {}, falls back to {}{over}\n",
            t.alias, t.name, t.preferred, t.fallback
        ));
    }
    out.push_str(&format!(
        "haiku effort cap: {} by default, never above {}\n",
        effort::model_cap::HAIKU_DEFAULT_CAP,
        effort::model_cap::HAIKU_HARD_CEILING
    ));
    for (name, level, reason) in effort::model_cap::OPT_INS {
        out.push_str(&format!("  {name} may use {level}: {reason}\n"));
    }
    if user_fallback {
        out.push_str("fallback chain: set by you (flag or fallbackModel), playbook adds none\n");
    } else {
        out.push_str(&format!("fallback chain ccc passes: {chain}\n"));
        if let Some(level) = effort.as_deref().filter(|l| OLD_GEN_REJECTS.contains(l)) {
            out.push_str(&format!(
                "effort {level} in force: the previous Sonnet and Opus reject it, so the chain starts at Haiku\n"
            ));
        }
    }
    out.trim_end().to_string()
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
        let flags = launcher_flags(&none, &settings, None).unwrap();
        assert_eq!(flags[0], "--fallback-model");

        let own = vec!["--fallback-model=sonnet".to_string()];
        assert!(launcher_flags(&own, &settings, None).is_none());

        std::fs::write(&settings, r#"{"fallbackModel":["sonnet"]}"#).unwrap();
        assert!(launcher_flags(&none, &settings, None).is_none());
    }

    #[test]
    fn at_xhigh_and_max_the_chain_keeps_only_the_model_that_accepts_them() {
        assert_eq!(fallback_chain_for(Some("xhigh")), "claude-haiku-4-5");
        assert_eq!(fallback_chain_for(Some("max")), "claude-haiku-4-5");
        assert_eq!(fallback_chain_for(Some("high")), fallback_chain());
        assert_eq!(fallback_chain_for(None), fallback_chain());
    }

    #[test]
    fn effort_in_force_reads_the_flag_then_settings_and_the_ceiling_lowers_it() {
        let dir = scratch_dir("models-effort");
        std::fs::create_dir_all(&dir).unwrap();
        let settings = dir.join("settings.json");
        std::fs::write(&settings, r#"{"effortLevel":"xhigh"}"#).unwrap();
        let none: Vec<String> = vec![];
        assert_eq!(
            effort_in_force(&none, &settings, None).as_deref(),
            Some("xhigh")
        );
        assert_eq!(
            effort_in_force(&none, &settings, Some("high")).as_deref(),
            Some("high")
        );
        let flag = vec!["--effort".to_string(), "max".to_string()];
        assert_eq!(
            effort_in_force(&flag, &settings, None).as_deref(),
            Some("max")
        );
        let eq = vec!["--effort=low".to_string()];
        assert_eq!(
            effort_in_force(&eq, &settings, Some("high")).as_deref(),
            Some("low")
        );
        assert_eq!(
            effort_in_force(&none, &dir.join("missing.json"), None),
            None
        );
    }

    #[test]
    fn a_valid_override_becomes_an_env_pair_and_an_invalid_one_is_ignored() {
        let home = scratch_dir("models-override");
        std::fs::create_dir_all(&home).unwrap();
        assert!(env_overrides(&home).is_empty());
        config::write::set(
            config::write::Tier::Global,
            "models.sonnet",
            Value::String("claude-sonnet-5".into()),
            &home,
            None,
        )
        .unwrap();
        assert_eq!(
            override_for("sonnet", &home).as_deref(),
            Some("claude-sonnet-5")
        );
        assert_eq!(override_for("opus", &home), None);
        assert!(config::write::set(
            config::write::Tier::Global,
            "models.sonnet",
            Value::String("claude-opus-5".into()),
            &home,
            None,
        )
        .is_err());
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
