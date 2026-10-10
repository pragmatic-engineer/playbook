// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook learn collect` reads a real git repo and prints counted JSON.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Ada")
        .env("GIT_AUTHOR_EMAIL", "ada@example.test")
        .env("GIT_COMMITTER_NAME", "Ada")
        .env("GIT_COMMITTER_EMAIL", "ada@example.test")
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn repo(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pb-learn-collect-{}-{tag}",
        playbook::testing::run_id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.as_path();
    git(p, &["init", "-q"]);
    git(p, &["config", "commit.gpgsign", "false"]);
    git(p, &["config", "tag.gpgsign", "false"]);
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(p.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(p.join("Makefile"), "build:\n\tcargo build\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: first"]);
    std::fs::write(p.join("src/main.rs"), "fn main() { }\n").unwrap();
    git(p, &["commit", "-q", "-am", "fix(core): second"]);
    git(p, &["tag", "v0.1.0"]);
    dir
}

fn run(dir: &Path, roles: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["learn", "collect"])
        .args(roles)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn git_role_counts_history_without_emails() {
    let dir = repo("git");
    let raw = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["learn", "collect", "git"])
        .current_dir(dir)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&raw.stdout);
    assert!(!text.contains("example.test"), "no emails: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let g = &v["git"];
    assert_eq!(g["commits_total"], 2);
    assert_eq!(g["contributors"][0]["name"], "Ada");
    assert_eq!(g["contributors"][0]["count"], 2);
    assert_eq!(g["churn_hotspots"][0]["path"], "src/main.rs");
    assert_eq!(g["churn_hotspots"][0]["count"], 2);
    assert_eq!(g["conventional_commit_share"], 1.0);
    assert!(g["recent_tags"][0].as_str().unwrap().starts_with("v0.1.0"));
    assert!(v.get("structure").is_none());
}

#[test]
fn structure_role_reads_tracked_files_and_make_targets() {
    let dir = repo("structure");
    let v = run(&dir, &["structure"]);
    let s = &v["structure"];
    assert_eq!(s["tracked_files"], 2);
    assert_eq!(s["entry_points"][0], "src/main.rs");
    assert_eq!(s["build_manifests"]["Makefile"]["targets"][0], "build");
}

#[test]
fn both_roles_print_in_one_object() {
    let dir = repo("both");
    let v = run(&dir, &["git", "structure"]);
    assert!(v.get("git").is_some() && v.get("structure").is_some());
}

#[test]
fn an_unknown_role_is_refused() {
    let dir = repo("bad");
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["learn", "collect", "jira"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(!out.status.success());
}
