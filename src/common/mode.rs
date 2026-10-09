// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The single resolver for whether playbook runs in `ask` or `auto` mode.
//! Precedence, highest first: `--auto`/`--ask` flag, `PLAYBOOK_MODE` env, the `mode`
//! config key, then the `ask` default. An invalid value at any level is
//! ignored with a warning and the next level decides.

use crate::common::{home_dir, repo_slug};
use crate::config;
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Ask,
    Auto,
}

/// Which level of the precedence chain supplied the resolved mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Flag,
    Env,
    Config,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub mode: Mode,
    pub source: Source,
    /// One entry per ignored invalid value, naming the level it came from.
    pub warnings: Vec<String>,
}

fn parse_mode(value: &str) -> Option<Mode> {
    match value {
        "ask" => Some(Mode::Ask),
        "auto" => Some(Mode::Auto),
        _ => None,
    }
}

/// Parse one level's raw string. Blank means unset; anything else that is
/// not a mode is ignored with a warning naming `level`.
fn parse_level(
    level: &str,
    raw: Option<&str>,
    case_insensitive: bool,
    warnings: &mut Vec<String>,
) -> Option<Mode> {
    let raw = raw?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = if case_insensitive {
        trimmed.to_ascii_lowercase()
    } else {
        trimmed.to_string()
    };
    let parsed = parse_mode(&candidate);
    if parsed.is_none() {
        warnings.push(format!(
            "ignoring invalid {level} value '{trimmed}', valid values are: ask, auto"
        ));
    }
    parsed
}

/// Pure form of the resolver: `env` and `config` are the raw strings found at
/// each level, `None` when that level is unset. The env value is matched
/// case-insensitively, the config value exactly as `config set` stores it.
pub fn resolve(flag: Option<Mode>, env: Option<&str>, config: Option<&str>) -> Resolved {
    let mut warnings = Vec::new();
    let env_mode = parse_level("PLAYBOOK_MODE", env, true, &mut warnings);
    let config_mode = parse_level("mode config", config, false, &mut warnings);
    let (mode, source) = match (flag, env_mode, config_mode) {
        (Some(mode), _, _) => (mode, Source::Flag),
        (None, Some(mode), _) => (mode, Source::Env),
        (None, None, Some(mode)) => (mode, Source::Config),
        (None, None, None) => (Mode::Ask, Source::Default),
    };
    Resolved {
        mode,
        source,
        warnings,
    }
}

/// Hook-side resolution: env then config only, since a hook has no flag.
/// `home` and `repo_slug` locate the config tiers, as in `config::resolve`.
pub fn resolve_for_hook_at(env: Option<&str>, home: &Path, repo_slug: Option<&str>) -> Resolved {
    let (config, config_warning) = match config::resolve("mode", home, repo_slug) {
        Ok((_, config::Source::Default)) => (None, None),
        Ok((value, _)) => (Some(value_text(&value)), None),
        Err(err) => (
            None,
            Some(format!("ignoring unreadable mode config: {err}")),
        ),
    };
    let mut resolved = resolve(None, env, config.as_deref());
    resolved.warnings.extend(config_warning);
    resolved
}

/// A JSON config value as plain text: a string without its quotes, anything
/// else as its JSON form.
pub fn value_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `resolve_for_hook_at` reading the real process environment and `$HOME`.
pub fn resolve_for_hook() -> Resolved {
    let home = home_dir();
    let slug = hook_slug(&home);
    resolve_for_hook_at(
        std::env::var("PLAYBOOK_MODE").ok().as_deref(),
        &home,
        slug.as_deref(),
    )
}

/// The repo slug for config lookups, `None` when empty or when no org or repo
/// tier file exists (the slug then cannot change any resolved value, so the
/// `git` spawn behind it is skipped).
pub fn hook_slug(home: &Path) -> Option<String> {
    if !config::store::any_scoped(&crate::common::paths::playbook_root_from(home)) {
        return None;
    }
    Some(repo_slug()).filter(|slug| !slug.is_empty())
}
