// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The one way a test starts the built `playbook` without a home of its
//! own: in a fresh scratch home, with no `PLAYBOOK_*`, `CLAUDE_*` or `XDG_*`
//! variables from the caller. A test that inherits the real `HOME` would run
//! the first-run heal against the real home. `test_isolation.rs` fails when
//! a test file starts the binary without this helper or its own `HOME`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

/// A fresh, empty scratch home.
pub fn fresh_home() -> PathBuf {
    let n = N.fetch_add(1, Ordering::Relaxed);
    let home = std::env::temp_dir().join(format!("pb-it-home-{}-{n}", playbook::testing::run_id()));
    std::fs::create_dir_all(&home).expect("scratch home");
    home
}

/// The built `playbook` in a fresh scratch home.
pub fn playbook() -> Command {
    playbook_in(&fresh_home())
}

/// The built `playbook` with `home` as its home.
pub fn playbook_in(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_playbook"));
    isolate(&mut cmd, home);
    cmd
}

/// Points every home `cmd` could read at `home` and drops the caller's
/// `PLAYBOOK_*`, `CLAUDE_*` and `XDG_*` variables. Set a variable after this
/// call to give it to the child.
pub fn isolate(cmd: &mut Command, home: &Path) {
    for (key, _) in std::env::vars_os() {
        let drop = key.to_str().is_some_and(|k| {
            ["PLAYBOOK_", "CLAUDE_", "XDG_"]
                .iter()
                .any(|p| k.starts_with(p))
        });
        if drop {
            cmd.env_remove(&key);
        }
    }
    cmd.env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"));
}
