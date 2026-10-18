// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A test that starts the built `playbook` with the real `HOME` runs the
//! first-run heal against the real home: it once wrote the real migration
//! record and moved the real collector agent. This check fails for any test
//! file that starts the built `playbook` without `support/isolated.rs` or a
//! `HOME` of its own.

#[path = "support/isolated.rs"]
mod isolated;

use std::fs;
use std::path::{Path, PathBuf};

const SPAWN: &str = "::new(env!(\"CARGO_BIN_EXE_playbook\"))";

/// Files that start the binary with the real `HOME` on purpose. Each only
/// runs `playbook hook ...`, a hot path that never runs the first-run heal,
/// and the binary refuses to heal the real home from a cargo test anyway.
const EXEMPT: [&str; 1] = [
    // The rm guard resolves `$HOME/...` paths against the real home.
    "cli/tests/hooks_guards.rs",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The files under `crates/*/tests` that start the built `playbook` with neither
/// the helper nor their own `HOME`.
fn unisolated(crates: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for krate in fs::read_dir(crates).into_iter().flatten().flatten() {
        rust_files(&krate.path().join("tests"), &mut files);
    }
    files
        .iter()
        .filter(|f| !f.ends_with("support/isolated.rs"))
        .filter(|f| !EXEMPT.iter().any(|e| f.ends_with(e)))
        .filter_map(|f| {
            let text = fs::read_to_string(f).ok()?;
            let isolated = text.contains("isolated::") || text.contains(".env(\"HOME\"");
            (text.contains(SPAWN) && !isolated).then(|| f.display().to_string())
        })
        .collect()
}

#[test]
fn every_test_that_starts_playbook_gives_it_a_scratch_home() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");

    let bad = unisolated(&crates);

    assert!(
        bad.is_empty(),
        "start the binary with isolated::playbook() from support/isolated.rs, or set HOME: {bad:?}"
    );
}

#[test]
fn the_check_catches_a_spawn_without_a_home() {
    let root = isolated::fresh_home();
    let tests = root.join("demo/tests");
    fs::create_dir_all(&tests).unwrap();
    fs::write(
        tests.join("bad.rs"),
        "fn f() { std::process::Command::new(env!(\"CARGO_BIN_EXE_playbook\")); }",
    )
    .unwrap();
    fs::write(tests.join("good.rs"), "fn f() { isolated::playbook(); }").unwrap();

    let bad = unisolated(&root);

    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].ends_with("bad.rs"));
}

#[test]
fn the_helper_points_every_home_at_the_scratch_home_and_drops_playbook_variables() {
    let home = isolated::fresh_home();
    let mut cmd = std::process::Command::new("/usr/bin/env");
    isolated::isolate(&mut cmd, &home);
    cmd.env("KEEP", "1");

    let out = cmd.env("PLAYBOOK_ALLOW_REAL_HOME", "1").output().unwrap();
    let mut probe = std::process::Command::new("/usr/bin/env");
    std::env::set_var("PLAYBOOK_LEAK_PROBE", "1");
    isolated::isolate(&mut probe, &home);
    let dropped = probe.output().unwrap();
    std::env::remove_var("PLAYBOOK_LEAK_PROBE");

    let env = String::from_utf8_lossy(&out.stdout);
    for want in [
        "HOME=",
        "XDG_CONFIG_HOME=",
        "XDG_CACHE_HOME=",
        "XDG_DATA_HOME=",
        "CLAUDE_CONFIG_DIR=",
    ] {
        let line = env
            .lines()
            .find(|l| l.starts_with(want))
            .unwrap_or_default();
        assert!(line.contains(&home.display().to_string()), "{want}: {line}");
    }
    assert!(
        env.contains("PLAYBOOK_ALLOW_REAL_HOME=1"),
        "a variable set after isolate reaches the child"
    );
    assert!(!String::from_utf8_lossy(&dropped.stdout).contains("PLAYBOOK_LEAK_PROBE"));
}
