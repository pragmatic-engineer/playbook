// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook memory context`: scope, anchor, edge, and missing-graph
//! behavior, ported from `shell/memory-context.test.sh`.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("playbook-memctx-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs the command against `graph` JSON for `repo`; returns (output, stdout).
fn run(graph: Option<&str>, repo: &str) -> (Output, String) {
    let dir = scratch("run");
    let path = dir.join("memory.graph.json");
    if let Some(json) = graph {
        fs::write(&path, json).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["memory", "context", "--repo", repo, "--graph"])
        .arg(&path)
        .env("HOME", &dir)
        .output()
        .expect("playbook");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    (out, text)
}

fn edges_section(text: &str) -> String {
    let mut on = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        if line.starts_with("Anchors:") {
            on = false;
        }
        if on {
            lines.push(line);
        }
        if line.starts_with("Edges:") {
            on = true;
        }
    }
    lines.join("\n")
}

fn fact(id: &str, scope: &str, name: &str, project: Option<&str>) -> String {
    let project = project
        .map(|p| format!(r#","project":"{p}""#))
        .unwrap_or_default();
    format!(
        r#"{{"id":"{id}","file":"{id}.md","scope":"{scope}","type":"user","name":"{name}","description":"d-{name}"{project}}}"#
    )
}

fn graph(nodes: &[String], edges: &[&str]) -> String {
    format!(
        r#"{{"nodes":[{}],"edges":[{}]}}"#,
        nodes.join(","),
        edges.join(",")
    )
}

#[test]
fn only_global_and_the_requested_repos_facts_appear() {
    let g = graph(
        &[
            fact("g1", "global", "global-fact-one", None),
            fact("a1", "project", "repo-a-fact", Some("ownerA/repoA")),
            fact("b1", "project", "repo-b-fact", Some("ownerB/repoB")),
        ],
        &[],
    );

    let (_, text) = run(Some(&g), "ownerA/repoA");

    assert!(text.contains("global-fact-one") && text.contains("repo-a-fact"));
    assert!(!text.contains("repo-b-fact"));
}

#[test]
fn an_unknown_repo_gets_globals_only() {
    let g = graph(
        &[
            fact("g1", "global", "global-only-fact", None),
            fact("a1", "project", "repo-a-fact", Some("ownerA/repoA")),
        ],
        &[],
    );

    let (_, text) = run(Some(&g), "nobody/some-repo");

    assert!(text.contains("global-only-fact"));
    assert!(!text.contains("repo-a-fact"));
}

#[test]
fn the_anchor_index_maps_a_path_to_the_fact_name() {
    let g = graph(
        &[
            fact("login", "global", "login-handling", None),
            r#"{"id":"code:src/auth/login.py","file":"src/auth/login.py","scope":"code","type":"code"}"#.to_string(),
        ],
        &[r#"{"from":"login","to":"code:src/auth/login.py","relation":"anchors"}"#],
    );

    let (_, text) = run(Some(&g), "owner/repo");

    assert!(text.contains("src/auth/login.py: login-handling"), "{text}");
}

#[test]
fn typed_edges_render_but_anchors_edges_stay_out_of_the_edges_section() {
    let g = graph(
        &[
            fact("f1", "global", "fact-one", None),
            fact("f2", "global", "fact-two", None),
            r#"{"id":"code:src/thing.py","file":"src/thing.py","scope":"code","type":"code"}"#
                .to_string(),
        ],
        &[
            r#"{"from":"f1","to":"f2","relation":"depends_on"}"#,
            r#"{"from":"f1","to":"code:src/thing.py","relation":"anchors"}"#,
        ],
    );

    let (_, text) = run(Some(&g), "owner/repo");

    let edges = edges_section(&text);
    assert!(edges.contains("fact-one depends_on fact-two"), "{text}");
    assert!(!edges.contains("anchors"));
    assert!(text.contains("src/thing.py: fact-one"));
}

#[test]
fn org_scope_matches_by_owner_not_by_full_repo() {
    let g = graph(
        &[
            fact("o", "org", "org-wide-fact", Some("ownerA")),
            fact("a1", "project", "repo-a-fact", Some("ownerA/repoA")),
            fact("b1", "project", "repo-b-fact", Some("ownerB/repoB")),
        ],
        &[],
    );

    let (_, own) = run(Some(&g), "ownerA/repoA");
    let (_, sibling) = run(Some(&g), "ownerA/repoB");
    let (_, other) = run(Some(&g), "ownerB/repoB");

    assert!(own.contains("org-wide-fact") && own.contains("repo-a-fact"));
    assert!(!own.contains("repo-b-fact"));
    assert!(sibling.contains("org-wide-fact"));
    assert!(!other.contains("org-wide-fact"));
}

#[test]
fn a_missing_graph_prints_nothing_and_exits_zero() {
    let (out, text) = run(None, "owner/repo");

    assert!(out.status.success());
    assert_eq!(text, "");
}

#[test]
fn the_default_graph_path_and_repo_come_from_home_and_origin() {
    let home = scratch("home");
    let mem = home.join(".config/playbook/memory");
    fs::create_dir_all(&mem).unwrap();
    fs::write(
        mem.join("memory.graph.json"),
        graph(&[fact("g1", "global", "default-path-fact", None)], &[]),
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["memory", "context"])
        .env("HOME", &home)
        .current_dir(&home)
        .output()
        .unwrap();

    assert!(String::from_utf8_lossy(&out.stdout).contains("default-path-fact"));
}
