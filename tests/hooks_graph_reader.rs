// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Behavioural tests for the `memory-anchors` hook, the sole reader of
//! `~/.claude/memory/graph.json`. Exercised black-box, through the
//! compiled `playbook` binary, the same way
//! `hooks/memory-anchors.test.sh` exercises the python original. Every
//! assertion in that shell script has a corresponding case below.
//!
//! Each test gets its own scratch `HOME` (never the real `~/.claude`) and
//! invokes the binary as a subprocess with `HOME` and `HOOK_INPUT` set on
//! the child only, so tests stay parallel-safe: no test here mutates this
//! process's own environment.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

// --- Test infrastructure ---------------------------------------------------

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory, with `.claude/memory` already created, unique
/// per call so parallel test threads never collide.
fn scratch_home(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let home = std::env::temp_dir().join(format!("playbook-wu5-{}-{tag}-{n}", std::process::id()));
    fs::create_dir_all(home.join(".claude").join("memory")).unwrap();
    // The hook relativises the edited path with `git rev-parse --show-toplevel`,
    // so it needs a real repo. Giving each scratch home its own removes the
    // dependency on THIS tree being a checkout, which is what made all eight
    // tests here panic under `cargo mutants` and would break them in a tarball
    // or the debian:stable-slim container WU-14 requires.
    let repo = home.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let ok = Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "init", "--quiet"])
        .status()
        .expect("git should be available")
        .success();
    assert!(ok, "scratch repo should initialise");
    home
}

/// The scratch repo inside `home`, resolved through its symlinks so it matches
/// what `git rev-parse --show-toplevel` reports back to the hook.
fn repo_dir(home: &Path) -> PathBuf {
    fs::canonicalize(home.join("repo")).expect("scratch repo should resolve")
}

fn graph_path(home: &Path) -> PathBuf {
    home.join(".claude").join("memory").join("graph.json")
}

fn write_graph(home: &Path, content: &str) {
    fs::write(graph_path(home), content).unwrap();
}

fn run_playbook(home: &Path, args: &[&str], hook_input: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(args)
        .current_dir(repo_dir(home))
        .env("HOME", home)
        .env("HOOK_INPUT", hook_input)
        .stdin(Stdio::null())
        .output()
        .expect("playbook binary should run")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout)
        .trim_end_matches('\n')
        .to_string()
}

// --- memory-anchors ----------------------------------------------------------

const BASE_GRAPH: &str = r#"{
  "nodes": [
    {"id": "global/fact-a", "file": "fact-a.md", "scope": "global", "type": "project", "name": "fact-a", "description": "Fact A describes src/a.py"},
    {"id": "global/fact-dir", "file": "fact-dir.md", "scope": "global", "type": "project", "name": "fact-dir", "description": "Fact dir describes everything under src/"},
    {"id": "global/fact-neighbour", "file": "fact-b.md", "scope": "global", "type": "project", "name": "fact-neighbour", "description": "Neighbour reached via depends_on"},
    {"id": "code:src/a.py", "file": "src/a.py", "scope": "code", "type": "code"},
    {"id": "code:src/", "file": "src/", "scope": "code", "type": "code"}
  ],
  "edges": [
    {"from": "global/fact-a", "to": "code:src/a.py", "relation": "anchors"},
    {"from": "global/fact-dir", "to": "code:src/", "relation": "anchors"},
    {"from": "global/fact-a", "to": "global/fact-neighbour", "relation": "depends_on"}
  ]
}"#;

/// An absolute path inside this test's own scratch repo, built the same way the
/// real Edit tool's `file_path` arrives, so the hook's relativisation is
/// genuinely exercised rather than bypassed.
fn edit_path(home: &Path, relpath: &str) -> String {
    format!("{}/{relpath}", repo_dir(home).display())
}

fn run_anchors_hook(home: &Path, file_path: &str, session_id: &str) -> Output {
    let hook_input = anchors_hook_input(file_path, session_id);
    run_playbook(home, &["hook", "memory-anchors"], &hook_input)
}

fn anchors_hook_input(file_path: &str, session_id: &str) -> String {
    json!({
        "session_id": session_id,
        "tool_name": "Edit",
        "tool_input": {"file_path": file_path}
    })
    .to_string()
}

/// hooks/memory-anchors.test.sh scenarios 1 and 3: an exactly anchored file
/// names its fact, and that fact's `depends_on` neighbour is named too.
#[test]
fn anchored_file_names_matching_fact_and_depends_on_neighbour() {
    // Arrange
    let home = scratch_home("anchors-hit");
    write_graph(&home, BASE_GRAPH);

    // Act
    let output = run_anchors_hook(&home, &edit_path(&home, "src/a.py"), "s1");

    // Assert: the exact line, not a substring, so a column reorder or a
    // garbled "(relation:name)" format is caught. "src/" is also a
    // directory anchor of src/a.py, so fact-dir's line is expected too.
    let out = stdout_of(&output);
    assert_eq!(
        out,
        r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"Memory facts anchored to src/a.py:\n- fact-a: Fact A describes src/a.py (depends_on:fact-neighbour)\n- fact-dir: Fact dir describes everything under src/"}}"#
    );

    let _ = fs::remove_dir_all(&home);
}

/// hooks/memory-anchors.test.sh scenario 2: a directory anchor matches an
/// edit to a file underneath it.
#[test]
fn directory_anchor_names_the_containing_directory_fact() {
    // Arrange
    let home = scratch_home("anchors-dir");
    write_graph(&home, BASE_GRAPH);

    // Act
    let output = run_anchors_hook(&home, &edit_path(&home, "src/deep/b.py"), "s2");

    // Assert: the exact line, not a substring.
    let out = stdout_of(&output);
    assert_eq!(
        out,
        r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"Memory facts anchored to src/deep/b.py:\n- fact-dir: Fact dir describes everything under src/"}}"#
    );

    let _ = fs::remove_dir_all(&home);
}

/// hooks/memory-anchors.test.sh scenario 4: an unanchored path emits nothing.
#[test]
fn unanchored_path_emits_nothing() {
    // Arrange
    let home = scratch_home("anchors-nomatch");
    write_graph(&home, BASE_GRAPH);

    // Act
    let output = run_anchors_hook(&home, &edit_path(&home, "other/unrelated.py"), "s4");

    // Assert
    assert_eq!(stdout_of(&output), "");

    let _ = fs::remove_dir_all(&home);
}

/// hooks/memory-anchors.test.sh scenario 5: the hook never blocks, on a
/// malformed payload, a missing `file_path`, or a missing graph.json.
#[test]
fn never_blocks_on_malformed_payload_missing_file_path_or_missing_graph() {
    // 5a: malformed payload
    let home_a = scratch_home("anchors-malformed");
    write_graph(&home_a, BASE_GRAPH);
    let out_a = run_playbook(&home_a, &["hook", "memory-anchors"], "not-json-at-all");
    assert_eq!(
        out_a.status.code(),
        Some(0),
        "malformed payload should exit 0"
    );
    assert_eq!(
        stdout_of(&out_a),
        "",
        "malformed payload should emit nothing"
    );
    let _ = fs::remove_dir_all(&home_a);

    // 5b: missing file_path
    let home_b = scratch_home("anchors-missing-path");
    write_graph(&home_b, BASE_GRAPH);
    let hook_input_b =
        json!({"session_id": "s5b", "tool_name": "Edit", "tool_input": {}}).to_string();
    let out_b = run_playbook(&home_b, &["hook", "memory-anchors"], &hook_input_b);
    assert_eq!(
        out_b.status.code(),
        Some(0),
        "missing file_path should exit 0"
    );
    assert_eq!(
        stdout_of(&out_b),
        "",
        "missing file_path should emit nothing"
    );
    let _ = fs::remove_dir_all(&home_b);

    // 5c: missing graph.json entirely (fresh store, no graph ever written)
    let home_c = scratch_home("anchors-missing-graph");
    let out_c = run_anchors_hook(&home_c, &edit_path(&home_c, "src/a.py"), "s5c");
    assert_eq!(out_c.status.code(), Some(0), "missing graph should exit 0");
    assert_eq!(stdout_of(&out_c), "", "missing graph should emit nothing");
    let _ = fs::remove_dir_all(&home_c);
}

/// hooks/memory-anchors.test.sh scenario 6: the index is built once, on the
/// first edit, and reused (not rebuilt) on the second.
#[test]
fn anchor_index_is_built_once_and_reused_on_second_edit() {
    // Arrange
    let home = scratch_home("anchors-cache-once");
    write_graph(&home, BASE_GRAPH);
    let sid = "s6";

    // Act (first edit: builds the cache)
    run_anchors_hook(&home, &edit_path(&home, "src/a.py"), sid);

    // Assert: index was built and is non-empty
    let idx = home
        .join(".claude")
        .join("runtime")
        .join(sid)
        .join("memory-anchor-index.tsv");
    assert!(
        fs::metadata(&idx).map(|m| m.len() > 0).unwrap_or(false),
        "index file should be built on first edit"
    );

    // Plant a marker; if the second edit rebuilds the index, it is wiped.
    let mut contents = fs::read_to_string(&idx).unwrap();
    contents.push_str("MARKERLINE\n");
    fs::write(&idx, contents).unwrap();

    // Act (second edit)
    run_anchors_hook(&home, &edit_path(&home, "src/dir-two.py"), sid);

    // Assert: marker survived, meaning the cache was reused, not rebuilt.
    let after = fs::read_to_string(&idx).unwrap();
    assert!(
        after.contains("MARKERLINE"),
        "index should be reused, not rebuilt, on the second edit"
    );

    let _ = fs::remove_dir_all(&home);
}

/// hooks/memory-anchors.test.sh scenario 7: staleness is pinned behaviour, a
/// fact added to the graph after the cache was built does not surface
/// within the same session.
#[test]
fn stale_cache_does_not_surface_a_fact_added_after_it_was_built() {
    // Arrange: deliberately no directory anchor here (unlike BASE_GRAPH), so
    // there is no other way the new file could legitimately match.
    let home = scratch_home("anchors-stale");
    let graph_before = json!({
        "nodes": [
            {"id": "global/fact-a", "file": "fact-a.md", "scope": "global", "type": "project", "name": "fact-a", "description": "Fact A describes src/a.py"},
            {"id": "code:src/a.py", "file": "src/a.py", "scope": "code", "type": "code"}
        ],
        "edges": [
            {"from": "global/fact-a", "to": "code:src/a.py", "relation": "anchors"}
        ]
    })
    .to_string();
    write_graph(&home, &graph_before);
    let sid = "s7";
    run_anchors_hook(&home, &edit_path(&home, "src/a.py"), sid); // builds the cache

    let graph_with_new_fact = json!({
        "nodes": [
            {"id": "global/fact-a", "file": "fact-a.md", "scope": "global", "type": "project", "name": "fact-a", "description": "Fact A describes src/a.py"},
            {"id": "global/fact-new", "file": "fact-new.md", "scope": "global", "type": "project", "name": "fact-new", "description": "Added to the graph after the cache was built"},
            {"id": "code:src/a.py", "file": "src/a.py", "scope": "code", "type": "code"},
            {"id": "code:src/new-file.py", "file": "src/new-file.py", "scope": "code", "type": "code"}
        ],
        "edges": [
            {"from": "global/fact-a", "to": "code:src/a.py", "relation": "anchors"},
            {"from": "global/fact-new", "to": "code:src/new-file.py", "relation": "anchors"}
        ]
    })
    .to_string();
    write_graph(&home, &graph_with_new_fact);

    // Act
    let output = run_anchors_hook(&home, &edit_path(&home, "src/new-file.py"), sid);

    // Assert
    assert_eq!(
        stdout_of(&output),
        "",
        "a fact added after the cache was built should not surface this session"
    );

    let _ = fs::remove_dir_all(&home);
}

/// hooks/memory-anchors.test.sh scenario 8: whenever the hook emits
/// anything, it is valid JSON with `hookEventName: PreToolUse`.
#[test]
fn additional_context_output_is_valid_json_with_pretooluse_event_name() {
    // Arrange
    let home = scratch_home("anchors-json-shape");
    write_graph(&home, BASE_GRAPH);

    // Act
    let output = run_anchors_hook(&home, &edit_path(&home, "src/a.py"), "s8");

    // Assert: the shape AND the message body, so a garbled or truncated
    // additionalContext would fail here rather than only a hookEventName
    // check that stays green as long as the envelope is right.
    let out = stdout_of(&output);
    let parsed: Value = serde_json::from_str(&out).expect("output should be valid JSON");
    assert_eq!(parsed["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(
        parsed["hookSpecificOutput"]["additionalContext"],
        "Memory facts anchored to src/a.py:\n- fact-a: Fact A describes src/a.py (depends_on:fact-neighbour)\n- fact-dir: Fact dir describes everything under src/"
    );

    let _ = fs::remove_dir_all(&home);
}

/// No shell equivalent for this hook (unlike rebuild-memory-graph, which
/// hooks/graph_writer's tests compare against a frozen golden of that
/// script's output, see tests/fixtures/golden/README.md);
/// this is the equivalent cross-implementation comparison against
/// hooks/memory-anchors.py, feeding both implementations the same
/// graph.json fixture and the same edit.
#[test]
fn python_and_rust_readers_agree_on_the_same_fixture() {
    // Arrange
    let home_rs = scratch_home("cross-impl-rs");
    write_graph(&home_rs, BASE_GRAPH);
    let target = edit_path(&home_rs, "src/a.py");

    // Act
    let out_rs = run_anchors_hook(&home_rs, &target, "cross1");

    // Assert against the frozen python oracle rather than a live python run.
    // See tests/fixtures/golden/README.md: the python original is deleted by
    // ADR 0007 WU-14, so its output is committed instead.
    let golden = include_str!("fixtures/golden/memory-anchors.src-a.txt");
    assert_eq!(
        stdout_of(&out_rs),
        golden,
        "python and rust memory-anchors outputs differ"
    );

    let _ = fs::remove_dir_all(&home_rs);
}
