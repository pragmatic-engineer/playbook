// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The host used when none is installed: `pbtop` as its own program.
//!
//! Data is the single file `~/.config/playbook/data/usage.db`, shared with
//! playbook. Settings are playbook's own (see `config`), so both tools always
//! see the same ones.

use super::host::Host;
use super::run::Paths;
use super::session_view::Record;
use serde_json::Value;
use std::path::{Path, PathBuf};

const NAME: &str = "pbtop";

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

    /// `pbtop` may run next to a playbook from before the rename to `setup`,
    /// or from before the part existed.
    fn setup_command(&self, part: &str) -> String {
        match part {
            "telemetry" => {
                "playbook setup --only telemetry (older versions: playbook init --telemetry)".into()
            }
            other => format!(
                "playbook setup --only {other} (older versions: playbook init has no {other} part)"
            ),
        }
    }

    /// `pbtop` does not start a collector, so name the command that does.
    fn start_hint(&self) -> String {
        self.command("usage collect")
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
        assert_eq!(h.command("usage --summary"), "pbtop --summary");
        assert_eq!(h.command("usage"), "pbtop");
        assert_eq!(h.command("usage collect"), "pbtop collect");
    }

    #[test]
    fn a_down_collector_names_the_collect_command() {
        use crate::telemetry_status::{empty_panel_lines_for, Mode};

        let down = empty_panel_lines_for(Some(Mode::OnCollectorDown), &Standalone);

        assert_eq!(Standalone.start_hint(), "pbtop collect");
        assert!(down.contains(&"pbtop collect".to_string()), "{down:?}");
        assert!(
            !down.iter().any(|l| l == crate::daemon::START_HINT),
            "{down:?}"
        );
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
