// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The host used when none is installed: playbook-telemetry as its own
//! program.
//!
//! Data is the single file `~/.config/playbook/data/usage.db`, shared with
//! playbook. Settings are playbook's own (see `config`), so both tools always
//! see the same ones.

use super::host::Host;
use super::run::Paths;
use super::session_view::Record;
use serde_json::Value;
use std::path::{Path, PathBuf};

const NAME: &str = "playbook-telemetry";

pub struct Standalone;

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
        super::config::resolve(home, key)
    }

    fn state_dir_from(&self, home: &Path) -> PathBuf {
        super::data::data_dir(home)
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
        assert_eq!(h.command("usage --summary"), "playbook-telemetry --summary");
        assert_eq!(h.command("usage"), "playbook-telemetry");
        assert_eq!(h.command("usage collect"), "playbook-telemetry collect");
    }

    #[test]
    fn data_lives_next_to_playbooks_own_files() {
        let home = crate::util::scratch_dir("standalone-state");
        assert_eq!(
            Standalone.state_dir_from(&home),
            crate::data::data_dir(&home)
        );
    }
}
