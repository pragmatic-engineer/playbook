// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Shared isolation for tests that spawn the real `playbook hook` binary and
//! depend on the resolved mode. `hook_command` strips every variable that can
//! change a hook's result (mode, headless and nudge switches, plugin and XDG
//! roots) from the child only and pins `$HOME` to a scratch directory, so
//! neither a developer's exported variables nor their real config can flip a
//! case, and the parent process env is never mutated.

#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
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
    let root = std::env::temp_dir().join(format!("auto-env-{}-{tag}-{n}", std::process::id()));
    let home = root.join("home");
    let cwd = root.join("cwd");
    fs::create_dir_all(&home).expect("scratch home should be creatable");
    fs::create_dir_all(&cwd).expect("scratch cwd should be creatable");
    Scratch { home, cwd }
}

impl Scratch {
    /// Plant `{"mode": <mode>}` as the global config under the scratch HOME.
    pub fn seed_mode_config(&self, mode: &str) {
        let dir = self.home.join(".config").join("playbook");
        fs::create_dir_all(&dir).expect("config dir should be creatable");
        fs::write(dir.join("config.json"), format!(r#"{{"mode":"{mode}"}}"#))
            .expect("config file should be writable");
    }
}

/// A `git` shim that logs each call to a file and answers every call with the
/// `acme/widgets` origin URL, plus the PATH that puts it first.
#[cfg(unix)]
pub struct GitShim {
    pub path: String,
    pub log: PathBuf,
}

#[cfg(unix)]
impl GitShim {
    pub fn calls(&self) -> usize {
        fs::read_to_string(&self.log).map_or(0, |log| log.lines().count())
    }
}

#[cfg(unix)]
pub fn git_shim(scratch: &Scratch) -> GitShim {
    use std::os::unix::fs::PermissionsExt;
    let bin = scratch.home.join("shim-bin");
    fs::create_dir_all(&bin).expect("shim dir should be creatable");
    let log = scratch.home.join("git-calls.log");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> '{}'\necho git@github.com:acme/widgets.git\n",
        log.display()
    );
    let shim = bin.join("git");
    fs::write(&shim, script).expect("shim should be writable");
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("shim should be exec");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    GitShim { path, log }
}

impl Scratch {
    /// Plant `{"mode": <mode>}` as the repo tier config for `owner/repo`.
    pub fn seed_repo_mode_config(&self, owner: &str, repo: &str, mode: &str) {
        let dir = self
            .home
            .join(".config/playbook/repos")
            .join(owner)
            .join(repo)
            .join(".config");
        fs::create_dir_all(&dir).expect("repo config dir should be creatable");
        fs::write(dir.join("config.json"), format!(r#"{{"mode":"{mode}"}}"#))
            .expect("repo config should be writable");
    }
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

/// Run the hook with `stdin` and the extra `env` pairs, returning stdout and
/// the exit code.
pub fn run_hook(scratch: &Scratch, hook: &str, stdin: &str, env: &[(&str, &str)]) -> (String, i32) {
    let mut command = hook_command(scratch, hook);
    command
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().expect("playbook spawns");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("stdin accepts the payload");
    let out = child.wait_with_output().expect("playbook exits");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}
