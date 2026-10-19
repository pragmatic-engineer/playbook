// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook memory stats` and the summary `playbook memory rebuild` prints,
//! run black-box against a scratch HOME.

#[path = "support/isolated.rs"]
mod isolated;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "playbook-memstats-{tag}-{}-{n}",
        playbook::testing::run_id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `playbook <args>` with `home` as HOME and `cwd` as the directory.
fn run(home: &Path, cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let out = isolated::playbook_in(home)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("playbook");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A store with one global fact linking a missing fact, and two project
/// facts, one anchored to a file.
fn seed_store(home: &Path) {
    let mem = home.join(".config/playbook/memory");
    let project = mem.join("acme/web");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        mem.join("tone.md"),
        "---\nname: tone\ndescription: d\ntype: feedback\nlinks:\n  relates_to: [gone]\n---\nbody\n",
    )
    .unwrap();
    fs::write(
        project.join("auth.md"),
        "---\nname: auth\ndescription: d\ntype: project\nanchors:\n  - src/auth.rs\n---\nbody\n",
    )
    .unwrap();
    fs::write(
        project.join("deploy.md"),
        "---\nname: deploy\ndescription: d\ntype: project\n---\nother words\n",
    )
    .unwrap();
}

#[test]
fn stats_reports_a_missing_graph_and_creates_nothing() {
    let home = scratch("missing");
    let cwd = scratch("cwd");

    let (code, out, _) = run(&home, &cwd, &["memory", "stats"]);

    assert_eq!(code, 0, "{out}");
    assert!(out.contains(".config/playbook/data/memory.db"), "{out}");
    assert!(out.contains("playbook memory rebuild"), "{out}");
    assert!(!home.join(".config/playbook/data/memory.db").exists());
}

#[test]
fn rebuild_prints_the_counts_and_stats_reads_the_same_graph() {
    let home = scratch("rebuild");
    let cwd = scratch("cwd");
    seed_store(&home);

    let (code, rebuilt, err) = run(&home, &cwd, &["memory", "rebuild"]);
    assert_eq!(code, 0, "{err}");
    let (code, stats, err) = run(&home, &cwd, &["memory", "stats"]);
    assert_eq!(code, 0, "{err}");

    assert!(rebuilt.contains("memory.db rebuilt"), "{rebuilt}");
    assert!(
        rebuilt.ends_with(&stats),
        "rebuild:\n{rebuilt}\nstats:\n{stats}"
    );
    assert!(
        stats.contains("facts: 3 (global 1, org 0, project 2)"),
        "{stats}"
    );
    assert!(stats.contains("nodes: 4 (facts 3, anchors 1)"), "{stats}");
    assert!(stats.contains("  anchors: 1\n"), "{stats}");
    assert!(stats.contains("  relates_to: 1\n"), "{stats}");
    assert!(stats.contains("dangling edges: 1\n"), "{stats}");
    assert!(
        stats.contains("global/tone -relates_to-> global/gone"),
        "{stats}"
    );
    assert!(stats.contains("dangling anchors: not checked"), "{stats}");
    assert!(home.join(".config/playbook/data/memory.db").is_file());
    // The graph never lands at the playbook root, a path older docs named.
    assert!(!home.join(".config/playbook/memory.db").exists());
}

#[test]
fn rebuild_json_prints_one_json_document() {
    let home = scratch("rebuild-json");
    let cwd = scratch("cwd");
    seed_store(&home);

    let (code, out, err) = run(&home, &cwd, &["memory", "rebuild", "--json"]);

    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("only JSON on stdout");
    assert_eq!(v["facts"]["by_scope"]["project"], 2);
}

#[test]
fn stats_json_checks_the_anchors_of_the_repo_it_runs_in() {
    let home = scratch("json");
    seed_store(&home);
    let repo = scratch("repo");
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("HOME", &home)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["remote", "add", "origin", "https://github.com/acme/web.git"]);
    let (code, _, err) = run(&home, &repo, &["memory", "rebuild"]);
    assert_eq!(code, 0, "{err}");

    let (code, out, err) = run(&home, &repo, &["memory", "stats", "--json"]);

    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("json");
    assert_eq!(v["facts"]["by_scope"]["project"], 2);
    assert_eq!(v["nodes"]["anchors"], 1);
    assert_eq!(v["edges"]["by_relation"]["relates_to"], 1);
    assert_eq!(v["dangling"]["anchor_repo"], "acme/web");
    assert_eq!(v["dangling"]["anchors"], serde_json::json!(["src/auth.rs"]));
    assert_eq!(v["database"]["present"], true);
    assert!(v["database"]["rebuilt_at"].is_string());
}
