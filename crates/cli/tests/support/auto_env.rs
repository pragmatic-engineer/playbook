// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Shared isolation for tests that spawn the real `playbook hook` binary and
//! depend on the resolved mode. `hook_command` strips every variable that can
//! change a hook's result (mode, headless and nudge switches, plugin and XDG
//! roots) from the child only and pins `$HOME` to a scratch directory, so
//! neither a developer's exported variables nor their real config can flip a
//! case, and the parent process env is never mutated.
//!
//! The rest of the helpers are in `auto_run.rs`, `auto_config.rs`,
//! `auto_warm_up.rs`, `auto_git_shim.rs` and `auto_feature_repo.rs`, so a test
//! file includes only what it uses.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Variables removed from the child on top of any `PLAYBOOK_HEADLESS_*` found
/// in the parent env.
pub const CLEARED_VARS: [&str; 13] = [
    "PLAYBOOK_MODE",
    "PLAYBOOK_AUTO_COST_INTERVAL_MS",
    "CI",
    "PLAYBOOK_HEADLESS",
    "PLAYBOOK_HEADLESS_MEMORY",
    "CLAUDE_PLUGIN_ROOT",
    "AUTO_LEARN_NUDGE",
    "SKILLS_PRIMER",
    "ASYNC_DISCIPLINE",
    "STATUSLINE_CACHE_DIR",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
];

/// A scratch `$HOME` and a working directory outside any git repo.
pub struct Scratch {
    pub home: PathBuf,
    pub cwd: PathBuf,
}

pub fn scratch(tag: &str) -> Scratch {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "auto-env-{}-{tag}-{n}",
        playbook::testing::run_id()
    ));
    let home = root.join("home");
    let cwd = root.join("cwd");
    fs::create_dir_all(&home).expect("scratch home should be creatable");
    fs::create_dir_all(&cwd).expect("scratch cwd should be creatable");
    Scratch { home, cwd }
}

/// `playbook hook <hook>` with the environment isolated; callers add the
/// per-case variables (for example `PLAYBOOK_MODE`) afterwards.
pub fn hook_command(scratch: &Scratch, hook: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_playbook"));
    command
        .args(["hook", hook])
        .current_dir(&scratch.cwd)
        .env("HOME", &scratch.home);
    for var in CLEARED_VARS {
        command.env_remove(var);
    }
    for (key, _) in std::env::vars() {
        if key.starts_with("PLAYBOOK_HEADLESS_") {
            command.env_remove(key);
        }
    }
    command
}
