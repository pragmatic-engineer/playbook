// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! One of the two places `src/usage` touches the host program (the other is
//! `util`). Config values, the state directory, the program name in user
//! facing text, the host's repo path layout and the sessions panel data all
//! come from here. Moving usage to its own crate means replacing these two
//! files.
//!
//! Parts of playbook that depend on usage (the statusline limits capture, the
//! `cc` launcher starting the collector, the hook pause check) stay on the
//! playbook side and call the generic functions in `usage::api`,
//! `usage::limits_source` and `usage::collector`.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// The program name used in user facing messages.
pub const PROGRAM: &str = "playbook";

/// The command line a user runs for `args`, for messages: `usage --summary`
/// becomes `playbook usage --summary`.
pub fn command(args: &str) -> String {
    format!("{PROGRAM} {args}")
}

/// Path fragment under which the host keeps one folder per cloned repo
/// (`<marker><owner>/<repo>/<worktree>`). Used to name a repo from a cwd.
pub const REPOS_MARKER: &str = "/.config/playbook/repos/";

/// The user's home directory.
pub fn home_dir() -> PathBuf {
    crate::common::session::home_dir()
}

/// The value of config key `key` (for example `usage.theme`), or `None` when
/// the key cannot be read. A standalone host returns its own default there.
pub fn config_value(key: &str, home: &Path) -> Option<Value> {
    crate::config::resolve_valid(key, home, None)
        .ok()
        .map(|(value, _, _)| value)
}

/// [`config_value`] for the current user's home.
pub fn config_value_now(key: &str) -> Option<Value> {
    config_value(key, &home_dir())
}

/// The folder holding the usage database under `home`.
pub fn state_dir_from(home: &Path) -> PathBuf {
    crate::common::paths::playbook_root_from(home).join("usage")
}

/// The folder holding the usage database for the real environment.
pub fn state_dir() -> PathBuf {
    crate::common::paths::usage_db_dir()
}

/// The sessions panel data, built by the host's `sessions` feature.
pub use crate::sessions::load as load_sessions;
