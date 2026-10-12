// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! One of the two places `src/usage` touches the program that embeds it (the
//! other is `util`). The embedder
//! implements [`Host`] once and installs it with [`install`] before calling
//! into usage. Without an installed host the standalone one is used (see
//! `standalone`). Config values, the state directory, the program name in user
//! facing text, the repo path layout and the sessions panel data all come from
//! here.

use super::run::Paths;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::session_view::Record;

/// What usage needs from the program that embeds it.
pub trait Host: Send + Sync {
    /// The program name in user facing messages.
    fn program(&self) -> &str;

    /// The command line a user runs for `args`, for messages. `args` always
    /// starts with `usage`: `usage --summary` becomes `playbook usage
    /// --summary` for a host that nests the command, or `playbook-usage
    /// --summary` for a program that is the usage tool itself.
    fn command(&self, args: &str) -> String;

    /// The arguments, before any usage subcommand, that make the program run
    /// usage. Used to start a detached `warm` run of the same program.
    fn launch_args(&self) -> Vec<String>;

    /// Path fragment under which the host keeps one folder per cloned repo
    /// (`<marker><owner>/<repo>/<worktree>`), used to name a repo from a cwd.
    /// `None` when the host has no such layout.
    fn repos_marker(&self) -> Option<&str>;

    /// The user's home directory.
    fn home_dir(&self) -> PathBuf;

    /// The value of config key `key` (for example `usage.theme`) for the user
    /// at `home`, or `None` when it is not set or cannot be read. Usage falls
    /// back to its own default then.
    fn config_value(&self, key: &str, home: &Path) -> Option<Value>;

    /// The folder holding the usage database under `home`.
    fn state_dir_from(&self, home: &Path) -> PathBuf;

    /// The folder holding the usage database for the real environment.
    fn state_dir(&self) -> PathBuf;

    /// The records for the sessions panel, with the time they were read at.
    fn sessions(&self, paths: &Paths, home: &Path) -> Result<(Vec<Record>, i64), String>;
}

static HOST: OnceLock<Box<dyn Host>> = OnceLock::new();

/// Installs the host. Returns `false` when one is already installed (the first
/// one stays).
pub fn install(host: Box<dyn Host>) -> bool {
    HOST.set(host).is_ok()
}

fn current() -> &'static dyn Host {
    HOST.get_or_init(default_host).as_ref()
}

/// Inside playbook's own tests there is no `main` to install the host, so the
/// playbook host is the default there. A standalone build deletes this one
/// function body's test branch.
#[cfg(test)]
fn default_host() -> Box<dyn Host> {
    crate::usage_host::playbook()
}

#[cfg(not(test))]
fn default_host() -> Box<dyn Host> {
    Box::new(super::standalone::Standalone)
}

/// The program name used in user facing messages.
pub fn program() -> &'static str {
    current().program()
}

/// The command line a user runs for `args`, for messages: `usage --summary`
/// becomes `playbook usage --summary`.
pub fn command(args: &str) -> String {
    current().command(args)
}

/// The arguments that make the program run usage, for a detached child.
pub fn launch_args() -> Vec<String> {
    current().launch_args()
}

/// The host's repo folder marker, if it has one.
pub fn repos_marker() -> Option<&'static str> {
    current().repos_marker()
}

/// The user's home directory.
pub fn home_dir() -> PathBuf {
    current().home_dir()
}

/// The value of config key `key`, or `None`.
pub fn config_value(key: &str, home: &Path) -> Option<Value> {
    current().config_value(key, home)
}

/// [`config_value`] for the current user's home.
pub fn config_value_now(key: &str) -> Option<Value> {
    config_value(key, &home_dir())
}

/// The folder holding the usage database under `home`.
pub fn state_dir_from(home: &Path) -> PathBuf {
    current().state_dir_from(home)
}

/// The folder holding the usage database for the real environment.
pub fn state_dir() -> PathBuf {
    current().state_dir()
}

/// The sessions panel data.
pub fn load_sessions(paths: &Paths, home: &Path) -> Result<(Vec<Record>, i64), String> {
    current().sessions(paths, home)
}
