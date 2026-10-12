// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The host used when none is installed: the usage tool as its own program.
//!
//! State goes to `$XDG_STATE_HOME/playbook-usage` (default
//! `~/.local/state/playbook-usage`), unless a database already exists under
//! `~/.config/playbook/usage`, which is then shared with playbook. Config is
//! an optional flat JSON object of dotted keys in
//! `$XDG_CONFIG_HOME/playbook-usage/config.json`
//! (default `~/.config/playbook-usage/config.json`), for example
//! `{"usage.theme": "default", "usage.budget.monthlyUsd": 200}`.

use super::host::Host;
use super::run::Paths;
use super::session_view::Record;
use serde_json::Value;
use std::path::{Path, PathBuf};

const NAME: &str = "playbook-usage";

pub struct Standalone;

fn env_dir(var: &str, home: &Path, fallback: &str) -> PathBuf {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() && Path::new(&v).is_absolute() => PathBuf::from(v),
        _ => home.join(fallback),
    }
}

fn config_file(home: &Path) -> PathBuf {
    env_dir("XDG_CONFIG_HOME", home, ".config")
        .join(NAME)
        .join("config.json")
}

fn lookup(text: &str, key: &str) -> Option<Value> {
    let doc: Value = serde_json::from_str(text).ok()?;
    doc.get(key).cloned()
}

impl Host for Standalone {
    fn program(&self) -> &str {
        NAME
    }

    fn command(&self, args: &str) -> String {
        let rest = args.strip_prefix("usage").unwrap_or(args).trim_start();
        if rest.is_empty() {
            NAME.to_string()
        } else {
            format!("{NAME} {rest}")
        }
    }

    fn launch_args(&self) -> Vec<String> {
        Vec::new()
    }

    fn repos_marker(&self) -> Option<&str> {
        None
    }

    fn home_dir(&self) -> PathBuf {
        super::util::home_dir()
    }

    fn config_value(&self, key: &str, home: &Path) -> Option<Value> {
        let text = std::fs::read_to_string(config_file(home)).ok()?;
        lookup(&text, key)
    }

    fn state_dir_from(&self, home: &Path) -> PathBuf {
        let shared = home.join(".config/playbook/usage");
        if shared.join("usage.db").is_file() {
            return shared;
        }
        env_dir("XDG_STATE_HOME", home, ".local/state").join(NAME)
    }

    fn state_dir(&self) -> PathBuf {
        self.state_dir_from(&self.home_dir())
    }

    fn sessions(&self, _paths: &Paths, _home: &Path) -> Result<(Vec<Record>, i64), String> {
        Ok((Vec::new(), super::util::time::now_secs()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_drops_the_usage_word() {
        let h = Standalone;
        assert_eq!(h.command("usage --summary"), "playbook-usage --summary");
        assert_eq!(h.command("usage"), "playbook-usage");
        assert_eq!(h.command("usage collect"), "playbook-usage collect");
    }

    #[test]
    fn a_key_is_read_from_the_flat_object() {
        let text = r#"{"usage.theme":"default","usage.budget.monthlyUsd":200}"#;
        assert_eq!(lookup(text, "usage.theme"), Some(Value::from("default")));
        assert_eq!(
            lookup(text, "usage.budget.monthlyUsd"),
            Some(Value::from(200))
        );
        assert_eq!(lookup(text, "usage.missing"), None);
        assert_eq!(lookup("not json", "usage.theme"), None);
    }

    #[test]
    fn state_falls_back_to_the_xdg_default_without_a_shared_database() {
        let home = crate::usage::util::scratch_dir("standalone-state");
        std::fs::create_dir_all(&home).unwrap();
        let dir = Standalone.state_dir_from(&home);
        assert!(dir.ends_with("playbook-usage"), "{dir:?}");
        let shared = home.join(".config/playbook/usage");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("usage.db"), b"").unwrap();
        assert_eq!(Standalone.state_dir_from(&home), shared);
    }
}
