// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Connects `usage` to playbook: the layered config, playbook's state folder
//! and repo layout, and the sessions view. Installed once at startup.

use crate::usage::command::SpendAction;
use crate::usage::host::{self, Host};
use crate::usage::run::Paths;
use crate::usage::session_view::Record;
use crate::usage::spend_fetch::SpendFetcher;
use crate::usage_spend::{self, UreqSpendFetcher};
use serde_json::Value;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

struct Playbook;

impl Host for Playbook {
    fn program(&self) -> &str {
        "playbook"
    }

    fn command(&self, args: &str) -> String {
        format!("playbook {args}")
    }

    fn setup_command(&self, part: &str) -> String {
        format!("playbook setup --only {part}")
    }

    fn launch_args(&self) -> Vec<String> {
        vec!["usage".to_string()]
    }

    fn repos_marker(&self) -> Option<&str> {
        Some("/.config/playbook/repos/")
    }

    fn home_dir(&self) -> PathBuf {
        crate::common::session::home_dir()
    }

    fn config_value(&self, key: &str, home: &Path) -> Option<Value> {
        crate::config::resolve_valid(key, home, None)
            .ok()
            .map(|(value, _, _)| value)
    }

    fn state_dir_from(&self, home: &Path) -> PathBuf {
        crate::usage::data::data_dir(home)
    }

    fn state_dir(&self) -> PathBuf {
        crate::common::paths::usage_db_dir()
    }

    fn sessions(&self, paths: &Paths, home: &Path) -> Result<(Vec<Record>, i64), String> {
        crate::sessions::load(paths, home)
    }

    fn spend_fetcher(&self) -> Option<Box<dyn SpendFetcher>> {
        Some(Box::new(UreqSpendFetcher::real(&self.home_dir())))
    }

    fn spend_ack(&self) -> bool {
        usage_spend::acknowledged(&self.home_dir())
    }

    fn spend_command(&self, action: &SpendAction) -> Result<String, String> {
        let home = self.home_dir();
        let data_dir = self.state_dir_from(&home);
        let env = usage_spend::Env {
            home: &home,
            data_dir: &data_dir,
            now: crate::usage::util::time::now_secs(),
            interactive: std::io::stdin().is_terminal(),
        };
        usage_spend::run(action, &env, &mut std::io::stdout(), &mut || {
            crate::init::attribution::ask_on_terminal("Turn it on?", false)
        })
    }
}

/// The playbook host.
pub fn playbook() -> Box<dyn Host> {
    Box::new(Playbook)
}

/// Makes playbook the host of `usage`. Call before any usage code runs.
pub fn install() {
    host::install(playbook());
}
