// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Behavioural tests for the `memory-anchors` hook, a reader of
//! the memory graph in `~/.config/playbook/data/memory.db`. Exercised black-box, through the
//! compiled `playbook` binary, the same way
//! the retired shell test exercises the python original. Every
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

/// A fresh scratch directory, with `.config/playbook/memory` already created, unique
/// per call so parallel test threads never collide.
fn scratch_home(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let home = std::env::temp_dir().join(format!(
        "playbook-wu5-{}-{tag}-{n}",
        playbook::testing::run_id()
    ));
    fs::create_dir_all(home.join(".config").join("playbook").join("memory")).unwrap();
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

/// Points the scratch repo's `origin` remote at `url`, so `repo_slug()`
/// resolves to a real `<owner>/<repo>` instead of the empty string a bare `git init` leaves it at.
fn set_origin(home: &Path, url: &str) {
    let ok = Command::new("git")
        .args([
            "-C",
            repo_dir(home).to_str().unwrap(),
            "remote",
            "add",
            "origin",
            url,
        ])
        .status()
        .expect("git should be available")
        .success();
    assert!(ok, "scratch repo should accept an origin remote");
}

fn graph_path(home: &Path) -> PathBuf {
    home.join(".config")
        .join("playbook")
        .join("memory")
        .join("memory.graph.json")
}

fn write_graph(home: &Path, content: &str) {
    playbook::testing::write_graph_fixture(graph_path(home).parent().unwrap(), content);
}

fn run_playbook(home: &Path, args: &[&str], hook_input: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .env_remove("CI")
        .env_remove("PLAYBOOK_HEADLESS")
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

/// the retired shell test scenarios 1 and 3: an exactly anchored file
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

/// the retired shell test scenario 2: a directory anchor matches an
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

/// the retired shell test scenario 4: an unanchored path emits nothing.
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

/// the retired shell test scenario 5: the hook never blocks, on a
/// malformed payload, a missing `file_path`, or a missing memory.graph.json.
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

    // 5c: missing memory.graph.json entirely (fresh store, no graph ever written)
    let home_c = scratch_home("anchors-missing-graph");
    let out_c = run_anchors_hook(&home_c, &edit_path(&home_c, "src/a.py"), "s5c");
    assert_eq!(out_c.status.code(), Some(0), "missing graph should exit 0");
    assert_eq!(stdout_of(&out_c), "", "missing graph should emit nothing");
    let _ = fs::remove_dir_all(&home_c);
}

/// the retired shell test scenario 6: the index is built once, on the
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
        .join(".config")
        .join("playbook")
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

/// A second edit of the same file in one session stays quiet, a new file in the
/// same session still gets its anchors, and a new session starts fresh.
#[test]
fn anchors_are_emitted_once_per_file_per_session() {
    let home = scratch_home("anchors-once-per-file");
    write_graph(&home, BASE_GRAPH);
    let a = edit_path(&home, "src/a.py");

    let first = stdout_of(&run_anchors_hook(&home, &a, "s-once"));
    assert!(
        first.contains("Memory facts anchored to src/a.py"),
        "{first}"
    );

    let second = stdout_of(&run_anchors_hook(&home, &a, "s-once"));
    assert!(second.is_empty(), "same file, same session: {second}");

    let other = stdout_of(&run_anchors_hook(
        &home,
        &edit_path(&home, "src/b.py"),
        "s-once",
    ));
    assert!(
        other.contains("Memory facts anchored to src/b.py"),
        "{other}"
    );

    let fresh = stdout_of(&run_anchors_hook(&home, &a, "s-new"));
    assert!(
        fresh.contains("Memory facts anchored to src/a.py"),
        "{fresh}"
    );

    let _ = fs::remove_dir_all(&home);
}

/// A file with many anchored facts is cut to about 600 characters and says how
/// many were left out.
#[test]
fn the_edit_anchor_message_is_capped_at_about_600_chars() {
    let home = scratch_home("anchors-cap");
    let mut nodes =
        String::from(r#"{"id":"code:src/a.py","file":"src/a.py","scope":"code","type":"code"}"#);
    let mut edges = String::new();
    for i in 0..30 {
        nodes.push_str(&format!(
            r#",{{"id":"global/f{i}","file":"f{i}.md","scope":"global","type":"project","name":"fact-{i}","description":"{}"}}"#,
            "long description ".repeat(6)
        ));
        if !edges.is_empty() {
            edges.push(',');
        }
        edges.push_str(&format!(
            r#"{{"from":"global/f{i}","to":"code:src/a.py","relation":"anchors"}}"#
        ));
    }
    write_graph(
        &home,
        &format!(r#"{{"nodes":[{nodes}],"edges":[{edges}]}}"#),
    );

    let out = stdout_of(&run_anchors_hook(
        &home,
        &edit_path(&home, "src/a.py"),
        "s-cap",
    ));
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    let msg = parsed["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        msg.chars().count() <= 600,
        "{} chars: {msg}",
        msg.chars().count()
    );
    assert!(msg.starts_with("Memory facts anchored to src/a.py:"));
    assert!(msg.contains("more anchored facts omitted"), "{msg}");

    let _ = fs::remove_dir_all(&home);
}

/// the retired shell test scenario 7: staleness is pinned behaviour, a
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

/// the retired shell test scenario 8: whenever the hook emits
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

/// `in_scope`'s `Some("org")` arm: a fact scoped to an org matches any repo
/// under the same owner, and is excluded for a repo under a different owner.
#[test]
fn org_scoped_fact_anchors_only_within_its_own_owner() {
    const GRAPH: &str = r#"{
      "nodes": [
        {"id": "acme/org-fact", "file": "org-fact.md", "scope": "org", "type": "reference", "name": "org-fact", "description": "Shared across every acme repo.", "project": "acme"},
        {"id": "code:src/a.py", "file": "src/a.py", "scope": "code", "type": "code"}
      ],
      "edges": [
        {"from": "acme/org-fact", "to": "code:src/a.py", "relation": "anchors"}
      ]
    }"#;

    let home_a = scratch_home("org-scope-same-owner");
    set_origin(&home_a, "git@github.com:acme/widget.git");
    write_graph(&home_a, GRAPH);
    let out_a = run_anchors_hook(&home_a, &edit_path(&home_a, "src/a.py"), "s-org-a");
    assert!(
        stdout_of(&out_a).contains("org-fact"),
        "an org-scoped fact should anchor for any repo under its owner: {}",
        stdout_of(&out_a)
    );
    let _ = fs::remove_dir_all(&home_a);

    let home_b = scratch_home("org-scope-different-owner");
    set_origin(&home_b, "git@github.com:other-org/widget.git");
    write_graph(&home_b, GRAPH);
    let out_b = run_anchors_hook(&home_b, &edit_path(&home_b, "src/a.py"), "s-org-b");
    assert_eq!(
        stdout_of(&out_b),
        "",
        "an org-scoped fact must not leak to a repo under a different owner"
    );
    let _ = fs::remove_dir_all(&home_b);
}

/// `in_scope`'s `Some("project")` arm: a fact scoped to one repo does not
/// anchor in a sibling repo, even under the same owner.
#[test]
fn project_scoped_fact_anchors_only_for_its_exact_repo() {
    const GRAPH: &str = r#"{
      "nodes": [
        {"id": "acme/widget/proj-fact", "file": "proj-fact.md", "scope": "project", "type": "reference", "name": "proj-fact", "description": "Specific to acme/widget.", "project": "acme/widget"},
        {"id": "code:src/a.py", "file": "src/a.py", "scope": "code", "type": "code"}
      ],
      "edges": [
        {"from": "acme/widget/proj-fact", "to": "code:src/a.py", "relation": "anchors"}
      ]
    }"#;

    let home_a = scratch_home("project-scope-match");
    set_origin(&home_a, "git@github.com:acme/widget.git");
    write_graph(&home_a, GRAPH);
    let out_a = run_anchors_hook(&home_a, &edit_path(&home_a, "src/a.py"), "s-proj-a");
    assert!(
        stdout_of(&out_a).contains("proj-fact"),
        "a project-scoped fact should anchor in its own repo: {}",
        stdout_of(&out_a)
    );
    let _ = fs::remove_dir_all(&home_a);

    let home_b = scratch_home("project-scope-sibling");
    set_origin(&home_b, "git@github.com:acme/other.git");
    write_graph(&home_b, GRAPH);
    let out_b = run_anchors_hook(&home_b, &edit_path(&home_b, "src/a.py"), "s-proj-b");
    assert_eq!(
        stdout_of(&out_b),
        "",
        "a project-scoped fact must not leak to a sibling repo under the same owner"
    );
    let _ = fs::remove_dir_all(&home_b);
}

// --- memory-anchors: UserPromptSubmit prompt-time recall ------------------
//
// ADR 0008 WU-0: the same anchor index above, now also matched on
// UserPromptSubmit (prompt text and this-session touched files), injecting
// fact BODIES rather than the name/description line PreToolUse emits.
// Scenario numbering follows the WU-0 brief in
// docs/adr/0008-bounded-memory-injection-with-prompt-recall-and-handoff-continuity-blueprint.md.

/// Writes a fact's markdown body under `~/.config/playbook/memory/<relpath>`, the
/// path `Node.file` is relative to (`rebuild_memory_graph.rs` builds it via
/// `strip_prefix` against the memory root). Creates parent dirs as needed.
fn write_fact_body(home: &Path, relpath: &str, content: &str) {
    let path = home
        .join(".config")
        .join("playbook")
        .join("memory")
        .join(relpath);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Appends one `edits.jsonl` line in the exact shape `post_edit_track.rs`
/// writes (`EditRecord { path, ts }`), under this session's runtime dir, so
/// the touched-file signal has something real to match against.
fn write_edit_record(home: &Path, session_id: &str, abs_path: &str, ts: u64) {
    let dir = home
        .join(".config")
        .join("playbook")
        .join("runtime")
        .join(session_id);
    fs::create_dir_all(&dir).unwrap();
    let line = json!({"path": abs_path, "ts": ts}).to_string();
    let mut contents = fs::read_to_string(dir.join("edits.jsonl")).unwrap_or_default();
    contents.push_str(&line);
    contents.push('\n');
    fs::write(dir.join("edits.jsonl"), contents).unwrap();
}

fn prompt_hook_input(prompt: &str, session_id: &str) -> String {
    json!({
        "hook_event_name": "UserPromptSubmit",
        "session_id": session_id,
        "prompt": prompt
    })
    .to_string()
}

/// A payload with `.prompt` absent and `.user_prompt` set instead, matching
/// the official docs' field name rather than this repo's own `.prompt`
/// convention.
fn user_prompt_hook_input(prompt: &str, session_id: &str) -> String {
    json!({
        "hook_event_name": "UserPromptSubmit",
        "session_id": session_id,
        "user_prompt": prompt
    })
    .to_string()
}

fn run_prompt_hook(home: &Path, prompt: &str, session_id: &str) -> Output {
    let hook_input = prompt_hook_input(prompt, session_id);
    run_playbook(home, &["hook", "memory-anchors"], &hook_input)
}

/// Parses stdout as JSON and pulls `additionalContext`, or "" if the hook
/// emitted nothing. Unlike the PreToolUse tests above (which pin the exact
/// line format), these tests check content is present, not exact
/// formatting, since the preamble around an injected fact body is an
/// implementation choice, not part of the contract under test.
fn additional_context(output: &Output) -> String {
    let out = stdout_of(output);
    if out.is_empty() {
        return String::new();
    }
    let parsed: Value = match serde_json::from_str(&out) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    parsed["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// WU-0 scenario 1: a prompt naming a fact surfaces its BODY, not just its
/// name or description. The body carries a string absent from both the name
/// and the description, so this fails against unmodified code (which has no
/// UserPromptSubmit branch at all) and would also fail a buggy
/// implementation that only echoed the description.
#[test]
fn prompt_mentions_a_fact_by_name_surfaces_its_body_not_just_its_name() {
    // Arrange
    let home = scratch_home("prompt-body");
    let graph = json!({
        "nodes": [
            {"id": "global/guard-fact", "file": "guard-fact.md", "scope": "global",
             "type": "feedback", "name": "guard-default-roots-untested",
             "description": "A test helper hid the zero-config fallback."}
        ],
        "edges": []
    })
    .to_string();
    write_graph(&home, &graph);
    write_fact_body(
        &home,
        "guard-fact.md",
        "---\nname: guard-default-roots-untested\n---\n\nBody mentions PLAYBOOK_SAFE_ROOTS explicitly, a string the description never uses.\n",
    );

    // Act
    let output = run_prompt_hook(&home, "why does guard-default-roots-untested happen", "p1");

    // Assert
    let context = additional_context(&output);
    assert!(
        context.contains("PLAYBOOK_SAFE_ROOTS"),
        "additionalContext should carry the fact BODY, not just its description: {context}"
    );

    let _ = fs::remove_dir_all(&home);
}

/// A fact with `aliases: [pbcode]` in its frontmatter is recalled by a prompt
/// that uses only the alias, and the recall is strong (no weak hint).
#[test]
fn a_fact_alias_in_the_prompt_recalls_the_fact() {
    // Arrange
    let home = scratch_home("prompt-alias");
    let graph = json!({
        "nodes": [
            {"id": "global/idx-fact", "file": "idx-fact.md", "scope": "global",
             "type": "reference", "name": "source-index-notes",
             "description": "How the symbol store is laid out."}
        ],
        "edges": []
    })
    .to_string();
    write_graph(&home, &graph);
    write_fact_body(
        &home,
        "idx-fact.md",
        "---\nname: source-index-notes\naliases: [pbcode]\n---\n\nALIASBODYMARKER\n",
    );

    // Act
    let output = run_prompt_hook(&home, "explain pbcode", "p-alias");

    // Assert
    let context = additional_context(&output);
    assert!(context.contains("ALIASBODYMARKER"), "{context}");
    assert!(
        !context.contains("Memory:"),
        "an alias hit is strong: {context}"
    );

    let _ = fs::remove_dir_all(&home);
}

/// A fact anchored to a crate folder is recalled by "<name> crate" in the
/// prompt, and the recall is strong (no weak hint).
#[test]
fn a_crate_name_in_the_prompt_recalls_the_fact_anchored_to_the_crate() {
    // Arrange
    let home = scratch_home("prompt-crate-name");
    let graph = json!({
        "nodes": [
            {"id": "global/idx-fact", "file": "idx-fact.md", "scope": "global",
             "type": "reference", "name": "source-index-notes",
             "description": "How the symbol store is laid out."},
            {"id": "code:crates/code/", "file": "crates/code/", "scope": "code", "type": "code"}
        ],
        "edges": [
            {"from": "global/idx-fact", "to": "code:crates/code/", "relation": "anchors"}
        ]
    })
    .to_string();
    write_graph(&home, &graph);
    write_fact_body(
        &home,
        "idx-fact.md",
        "---\nname: source-index-notes\n---\n\nCRATEBODYMARKER\n",
    );

    // Act
    let output = run_prompt_hook(&home, "tell me about the code crate", "p-crate");

    // Assert
    let context = additional_context(&output);
    assert!(context.contains("CRATEBODYMARKER"), "{context}");
    assert!(
        !context.contains("Memory:"),
        "a crate hit is strong: {context}"
    );

    let _ = fs::remove_dir_all(&home);
}

/// Refinement pass finding: the `.prompt` empty, fall back to `.user_prompt`
/// branch (added defensively because the official docs and this repo's own
/// live code disagree on the field name, see the blueprint's resolved open
/// items) had no test at all. Added here rather than left uncovered.
#[test]
fn prompt_field_absent_falls_back_to_user_prompt_field() {
    // Arrange
    let home = scratch_home("prompt-fallback-field");
    let graph = json!({
        "nodes": [
            {"id": "global/fallback-fact", "file": "fallback-fact.md", "scope": "global",
             "type": "feedback", "name": "fallback-fact", "description": "reached via user_prompt"}
        ],
        "edges": []
    })
    .to_string();
    write_graph(&home, &graph);
    write_fact_body(
        &home,
        "fallback-fact.md",
        "FALLBACKBODY only reachable via user_prompt.\n",
    );

    // Act: a payload with `.user_prompt` set and `.prompt` entirely absent.
    let hook_input = user_prompt_hook_input("tell me about fallback-fact", "pfallback");
    let output = run_playbook(&home, &["hook", "memory-anchors"], &hook_input);

    // Assert
    let context = additional_context(&output);
    assert!(
        context.contains("FALLBACKBODY"),
        "a payload carrying only .user_prompt should still match, via the fallback: {context}"
    );

    let _ = fs::remove_dir_all(&home);
}

/// WU-0 scenario 2: a fact already surfaced this session is not repeated,
/// but a newly matching fact still appears. Both halves are required: a
/// test asserting only "not repeated" would pass identically against
/// unmodified code, since nothing injects on UserPromptSubmit today either.
#[test]
fn dedup_skips_a_repeated_fact_but_still_injects_a_new_one() {
    // Arrange
    let home = scratch_home("prompt-dedup");
    let graph = json!({
        "nodes": [
            {"id": "global/fact-one", "file": "fact-one.md", "scope": "global",
             "type": "feedback", "name": "fact-one", "description": "First fact"},
            {"id": "global/fact-two", "file": "fact-two.md", "scope": "global",
             "type": "feedback", "name": "fact-two", "description": "Second fact"}
        ],
        "edges": []
    })
    .to_string();
    write_graph(&home, &graph);
    write_fact_body(&home, "fact-one.md", "BODYONE marks fact one's content.\n");
    write_fact_body(&home, "fact-two.md", "BODYTWO marks fact two's content.\n");
    let sid = "p2";

    // Act: turn 1 matches fact-one only.
    let turn1 = run_prompt_hook(&home, "tell me about fact-one", sid);
    let turn1_context = additional_context(&turn1);
    assert!(
        turn1_context.contains("BODYONE"),
        "turn 1 should inject fact-one's body: {turn1_context}"
    );

    // Act: turn 2 matches both fact-one (already seen) and fact-two (new).
    let turn2 = run_prompt_hook(&home, "fact-one and fact-two both matter", sid);

    // Assert
    let turn2_context = additional_context(&turn2);
    assert!(
        !turn2_context.contains("BODYONE"),
        "turn 2 should not repeat fact-one, already injected this session: {turn2_context}"
    );
    assert!(
        turn2_context.contains("BODYTWO"),
        "turn 2 should still inject fact-two, matched for the first time: {turn2_context}"
    );

    let _ = fs::remove_dir_all(&home);
}

/// WU-0 scenario 3: a file touched earlier this session, but not named in
/// the current prompt, still surfaces its anchored fact. Pins the exact
/// defect this WU exists to fix: asking about a file, without editing it,
/// used to surface nothing.
#[test]
fn a_file_touched_this_session_surfaces_its_anchored_fact_even_if_unmentioned() {
    // Arrange
    let home = scratch_home("prompt-touched-file");
    let touched_abs = edit_path(&home, "src/rm_workspace_guard.rs");
    let graph = json!({
        "nodes": [
            {"id": "global/guard-anchor-fact", "file": "guard-anchor-fact.md", "scope": "global",
             "type": "feedback", "name": "guard-anchor-fact", "description": "About the rm guard"},
            {"id": "code:src/rm_workspace_guard.rs", "file": "src/rm_workspace_guard.rs", "scope": "code", "type": "code"}
        ],
        "edges": [
            {"from": "global/guard-anchor-fact", "to": "code:src/rm_workspace_guard.rs", "relation": "anchors"}
        ]
    })
    .to_string();
    write_graph(&home, &graph);
    write_fact_body(
        &home,
        "guard-anchor-fact.md",
        "TOUCHEDFILEBODY is the guard's gotcha.\n",
    );
    let sid = "p3";
    write_edit_record(&home, sid, &touched_abs, 1_700_000_000);

    // Act: the prompt names neither the file nor the fact.
    let output = run_prompt_hook(&home, "what should I work on next", sid);

    // Assert
    let context = additional_context(&output);
    assert!(
        context.contains("TOUCHEDFILEBODY"),
        "a fact anchored to a file touched earlier this session should surface: {context}"
    );
    assert!(
        !context.contains("Memory:"),
        "a touched-file match is strong, so no weak hint: {context}"
    );

    let _ = fs::remove_dir_all(&home);
}

/// A prompt with searchable words and no matching fact gets one hint line, so
/// the model can tell "searched, found nothing" from "did not search".
#[test]
fn prompt_with_no_match_emits_a_hint_naming_the_searched_terms() {
    // Arrange
    let home = scratch_home("prompt-nomatch");
    write_graph(&home, BASE_GRAPH);

    // Act
    let output = run_prompt_hook(&home, "completely unrelated words here", "p4");

    // Assert
    let context = additional_context(&output);
    assert!(context.starts_with("Memory: no fact matched"), "{context}");
    assert!(context.contains("completely"), "{context}");

    let _ = fs::remove_dir_all(&home);
}

/// WU-0 scenario 5: a missing or corrupted anchor index degrades to
/// silence or one hint line, not a crash, matching this codebase's "never panics, degrades
/// to say nothing" invariant.
#[test]
fn corrupted_anchor_index_degrades_to_at_most_a_hint_not_a_crash() {
    // Arrange: pre-plant a garbage index file. The cache is "built once and
    // reused" by file EXISTENCE, so a pre-existing garbage file is not
    // rebuilt, it must be tolerated on read instead.
    let home = scratch_home("prompt-corrupt-index");
    let graph = json!({
        "nodes": [
            {"id": "global/fact-a", "file": "fact-a.md", "scope": "global",
             "type": "feedback", "name": "fact-a", "description": "describes something"}
        ],
        "edges": []
    })
    .to_string();
    write_graph(&home, &graph);
    let sid = "p5";
    let idx_dir = home
        .join(".config")
        .join("playbook")
        .join("runtime")
        .join(sid);
    fs::create_dir_all(&idx_dir).unwrap();
    fs::write(
        idx_dir.join("memory-anchor-index.tsv"),
        "this is not\tvalid\ttsv\tin the\texpected\tcolumn\tshape\tat all\textra\tcolumns",
    )
    .unwrap();

    // Act
    let output = run_prompt_hook(&home, "tell me about fact-a", sid);

    // Assert
    assert_eq!(
        output.status.code(),
        Some(0),
        "a corrupted index should not crash the hook"
    );
    let context = additional_context(&output);
    assert!(!context.contains("### "), "no fact body from garbage rows");
    assert!(
        context.starts_with("Memory: no fact matched"),
        "a corrupted index is silent or one hint line, never garbage: {context}"
    );

    let _ = fs::remove_dir_all(&home);
}

/// WU-0 scenario 6: a fact whose `file` path has been deleted since the
/// graph was last rebuilt is skipped, not fatal; a second, real match still
/// injects.
#[test]
fn a_fact_with_a_deleted_file_path_is_skipped_without_blocking_a_real_match() {
    // Arrange: two facts match the same prompt. Only the second has a body
    // on disk; the first's `file` points nowhere.
    let home = scratch_home("prompt-deleted-file");
    let graph = json!({
        "nodes": [
            {"id": "global/fact-missing", "file": "fact-missing.md", "scope": "global",
             "type": "feedback", "name": "fact-missing", "description": "shared-keyword fact one"},
            {"id": "global/fact-present", "file": "fact-present.md", "scope": "global",
             "type": "feedback", "name": "fact-present", "description": "shared-keyword fact two"}
        ],
        "edges": []
    })
    .to_string();
    write_graph(&home, &graph);
    // Deliberately do NOT write fact-missing.md.
    write_fact_body(
        &home,
        "fact-present.md",
        "PRESENTBODY is real and on disk.\n",
    );

    // Act
    let output = run_prompt_hook(&home, "shared-keyword fact one and fact two", "p6");

    // Assert
    assert_eq!(
        output.status.code(),
        Some(0),
        "a deleted fact file should not crash the hook"
    );
    let context = additional_context(&output);
    assert!(
        context.contains("PRESENTBODY"),
        "the real, present fact should still inject even though its sibling's file is missing: {context}"
    );

    let _ = fs::remove_dir_all(&home);
}

// --- memory-anchors: usage-based promotion signals -------------------------

fn signals_path(home: &Path) -> PathBuf {
    home.join(".config")
        .join("playbook")
        .join("memory")
        .join("memory.signals.json")
}

fn hits_for(home: &Path, node_id: &str) -> u64 {
    let content = fs::read_to_string(signals_path(home)).expect("memory.signals.json should exist");
    let parsed: Value = serde_json::from_str(&content).expect("memory.signals.json should parse");
    parsed["nodes"][node_id]["hits"]
        .as_u64()
        .expect("node should have a numeric hits field")
}

/// An anchor match on `PreToolUse` bumps the matched fact's hit count in
/// `memory.signals.json`.
#[test]
fn anchor_match_bumps_the_matched_facts_hit_count() {
    // Arrange
    let home = scratch_home("anchor-bump");
    write_graph(&home, BASE_GRAPH);

    // Act
    run_anchors_hook(&home, &edit_path(&home, "src/a.py"), "sbump1");

    // Assert
    assert_eq!(hits_for(&home, "global/fact-a"), 1);

    let _ = fs::remove_dir_all(&home);
}

/// Editing the same anchored file three times in one session must NOT
/// promote the fact: that is repetition within a single sitting, not the
/// cross-session recurrence the promotion threshold exists to catch. The
/// same session id is reused across all three calls (matching what a real
/// session looks like: one `session_id` for the whole run), so the second
/// and third bumps must be suppressed by the same per-session dedup the
/// prompt-match path already had.
#[test]
fn repeated_edits_in_one_session_do_not_promote_the_fact() {
    // Arrange
    let home = scratch_home("anchor-bump-same-session");
    write_graph(&home, BASE_GRAPH);
    let path = edit_path(&home, "src/a.py");

    // Act
    run_anchors_hook(&home, &path, "ssame1");
    run_anchors_hook(&home, &path, "ssame1");
    run_anchors_hook(&home, &path, "ssame1");

    // Assert
    assert_eq!(
        hits_for(&home, "global/fact-a"),
        1,
        "three edits in one session should count as one hit, not three"
    );
    let content =
        fs::read_to_string(signals_path(&home)).expect("memory.signals.json should exist");
    let parsed: Value = serde_json::from_str(&content).expect("memory.signals.json should parse");
    assert_eq!(
        parsed["nodes"]["global/fact-a"]["promoted"], false,
        "one real hit must not promote a fact whose threshold is 3"
    );

    let _ = fs::remove_dir_all(&home);
}

/// A prompt match on `UserPromptSubmit` bumps the matched fact's hit count
/// in `memory.signals.json`, the same as the `PreToolUse` anchor match.
#[test]
fn prompt_match_bumps_the_matched_facts_hit_count() {
    // Arrange
    let home = scratch_home("prompt-bump");
    let graph = json!({
        "nodes": [
            {"id": "global/guard-fact", "file": "guard-fact.md", "scope": "global",
             "type": "feedback", "name": "guard-default-roots-untested",
             "description": "A test helper hid the zero-config fallback."}
        ],
        "edges": []
    })
    .to_string();
    write_graph(&home, &graph);
    write_fact_body(
        &home,
        "guard-fact.md",
        "Body mentions PLAYBOOK_SAFE_ROOTS explicitly.\n",
    );

    // Act
    run_prompt_hook(
        &home,
        "why does guard-default-roots-untested happen",
        "pbump1",
    );

    // Assert
    assert_eq!(hits_for(&home, "global/guard-fact"), 1);

    let _ = fs::remove_dir_all(&home);
}

/// No shell equivalent for this hook (unlike rebuild-memory-graph, which
/// hooks/graph_writer's tests compare against a frozen golden of that
/// script's output, see tests/fixtures/golden/README.md);
/// this is the equivalent cross-implementation comparison against
/// the retired shell original, feeding both implementations the same
/// memory.graph.json fixture and the same edit.
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

// --- session-init: legacy graph.json fallback -------------------------------

fn old_graph_path(home: &Path) -> PathBuf {
    home.join(".config")
        .join("playbook")
        .join("memory")
        .join("graph.json")
}

fn run_session_init_hook(home: &Path, session_id: &str) -> Output {
    let hook_input = json!({"session_id": session_id, "source": "startup"}).to_string();
    run_playbook(home, &["hook", "session-init"], &hook_input)
}

/// The `session-init` hook's own fallback covers a session starting before
/// `playbook setup` is re-run after an upgrade, and stays idempotent.
#[test]
fn session_init_fallback_migrates_legacy_graph_file_and_is_idempotent() {
    // Arrange: only the pre-rename filename exists.
    let home = scratch_home("session-init-fallback");
    fs::write(old_graph_path(&home), BASE_GRAPH).unwrap();

    // Act
    let first = run_session_init_hook(&home, "sfallback1");

    // Assert: the fallback renamed the file in place, no data lost.
    assert_eq!(first.status.code(), Some(0), "session-init should exit 0");
    assert!(
        !old_graph_path(&home).is_file(),
        "the legacy graph.json should be renamed away, not left behind"
    );
    assert_eq!(
        fs::read_to_string(graph_path(&home)).expect("memory.graph.json should now exist"),
        BASE_GRAPH,
        "the fallback must preserve the old file's content exactly"
    );

    // Act: a second session start, now that only the new filename remains.
    let second = run_session_init_hook(&home, "sfallback2");

    // Assert: idempotent, no crash, content untouched.
    assert_eq!(
        second.status.code(),
        Some(0),
        "a second session-init run must not fail once the store is already migrated"
    );
    assert_eq!(
        fs::read_to_string(graph_path(&home)).expect("memory.graph.json should still exist"),
        BASE_GRAPH,
        "a second run must not alter the already-migrated store"
    );

    let _ = fs::remove_dir_all(&home);
}

// ---------------------------------------------------------------------
// Prompt recall is bounded: stopwords, at most three facts, capped bodies.
// ---------------------------------------------------------------------

/// `count` global facts whose descriptions all mention `widgets`, each with a
/// 5,000 character body, written under `home`.
fn write_widget_facts(home: &Path, count: usize) {
    let nodes: Vec<Value> = (0..count)
        .map(|n| {
            json!({"id": format!("global/widget-{n:02}"), "file": format!("widget-{n:02}.md"),
                   "scope": "global", "type": "feedback", "name": format!("widget-{n:02}"),
                   "description": "Use when widgets misbehave"})
        })
        .collect();
    write_graph(home, &json!({"nodes": nodes, "edges": []}).to_string());
    for n in 0..count {
        write_fact_body(home, &format!("widget-{n:02}.md"), &"w".repeat(5000));
    }
}

#[test]
fn a_prompt_matching_many_facts_injects_three_with_capped_bodies() {
    // Arrange
    let home = scratch_home("recall-cap");
    write_widget_facts(&home, 20);

    // Act
    let context = additional_context(&run_prompt_hook(&home, "widgets misbehave", "rc1"));

    // Assert
    assert_eq!(context.matches("### widget-").count(), 3, "{context}");
    assert!(context.contains("Use when widgets misbehave"));
    assert!(context.contains("[cut; full fact:"), "bodies are cut");
    assert!(
        context.chars().count() < 5200,
        "{}",
        context.chars().count()
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn a_prompt_of_only_common_words_injects_nothing() {
    // Arrange: every description holds the words a generic prompt uses.
    let home = scratch_home("recall-stopwords");
    let graph = json!({"nodes": [
        {"id": "global/a", "file": "a.md", "scope": "global", "type": "feedback",
         "name": "avoid-flaky", "description": "Use when you fix the failing test and add a flag to the cli"}
    ], "edges": []})
    .to_string();
    write_graph(&home, &graph);
    write_fact_body(&home, "a.md", "BODY\n");

    // Act
    let context = additional_context(&run_prompt_hook(
        &home,
        "can you fix this and add one",
        "rc2",
    ));

    // Assert
    assert_eq!(context, "");

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn recall_goes_quiet_after_nine_facts_in_one_session() {
    // Arrange
    let home = scratch_home("recall-session-cap");
    write_widget_facts(&home, 20);

    // Act: five prompts, each able to match fresh facts.
    let counts: Vec<usize> = (0..5)
        .map(|_| {
            additional_context(&run_prompt_hook(&home, "widgets misbehave", "rc3"))
                .matches("### widget-")
                .count()
        })
        .collect();

    // Assert
    assert_eq!(counts, vec![3, 3, 3, 0, 0]);

    let _ = fs::remove_dir_all(&home);
}

// --- memory-anchors: recall hint on zero or weak match -----------------------

/// Graph with one fact per `(name, description)`, bodies `BODY-<name>`.
fn write_named_facts(home: &Path, facts: &[(&str, &str)]) {
    let nodes: Vec<Value> = facts
        .iter()
        .map(|(name, desc)| {
            json!({"id": format!("global/{name}"), "file": format!("{name}.md"),
                   "scope": "global", "type": "feedback", "name": name,
                   "description": desc})
        })
        .collect();
    write_graph(home, &json!({"nodes": nodes, "edges": []}).to_string());
    for (name, _) in facts {
        write_fact_body(home, &format!("{name}.md"), &format!("BODY-{name}\n"));
    }
}

fn session_file(home: &Path, session_id: &str, name: &str) -> PathBuf {
    home.join(".config")
        .join("playbook")
        .join("runtime")
        .join(session_id)
        .join(name)
}

const HINTS_FILE: &str = "prompt-recall-hints.tsv";

#[test]
fn a_prompt_that_hits_only_a_description_word_gets_the_fact_and_the_weak_hint() {
    // Arrange
    let home = scratch_home("hint-weak-description");
    write_named_facts(&home, &[("tidy-notes", "Covers the gizmo setup")]);

    // Act
    let context = additional_context(&run_prompt_hook(&home, "gizmo", "h1"));

    // Assert
    assert!(context.contains("### tidy-notes"), "{context}");
    assert!(context.contains("Memory: only weak matches"), "{context}");

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn a_prompt_that_hits_one_plain_token_on_a_fact_name_gets_the_fact_and_the_weak_hint() {
    // Arrange
    let home = scratch_home("hint-weak-name-token");
    write_named_facts(
        &home,
        &[("rust-monorepo-crates-layout", "How the workspace is split")],
    );

    // Act
    let context = additional_context(&run_prompt_hook(&home, "PB code crate", "h2"));

    // Assert
    assert!(
        context.contains("### rust-monorepo-crates-layout"),
        "{context}"
    );
    assert!(context.contains("Memory: only weak matches"), "{context}");

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn a_prompt_that_hits_a_fact_name_gets_no_hint() {
    // Arrange
    let home = scratch_home("hint-strong");
    write_named_facts(&home, &[("guard-default-roots", "Roots fallback")]);

    // Act
    let context = additional_context(&run_prompt_hook(&home, "why guard-default-roots", "h3"));

    // Assert
    assert!(context.contains("### guard-default-roots"), "{context}");
    assert!(!context.contains("Memory:"), "{context}");

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn the_sixth_zero_match_prompt_in_a_session_gets_no_hint() {
    // Arrange
    let home = scratch_home("hint-session-cap");
    write_graph(&home, BASE_GRAPH);
    let prompts = [
        "zebra", "yak", "walrus", "vulture", "urchin", "tapir", "newt",
    ];

    // Act
    let contexts: Vec<String> = prompts
        .iter()
        .map(|p| additional_context(&run_prompt_hook(&home, p, "h4")))
        .collect();

    // Assert
    let hinted = contexts.iter().filter(|c| c.starts_with("Memory:")).count();
    assert_eq!(hinted, 5, "{contexts:?}");
    assert!(contexts[5..].iter().all(String::is_empty), "{contexts:?}");

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn matches_whose_fact_file_is_missing_get_the_zero_match_hint() {
    // Arrange: the node exists, its body does not.
    let home = scratch_home("hint-missing-body");
    write_named_facts(&home, &[("ghost-fact", "A fact with no file")]);
    fs::remove_file(
        home.join(".config")
            .join("playbook")
            .join("memory")
            .join("ghost-fact.md"),
    )
    .unwrap();

    // Act
    let context = additional_context(&run_prompt_hook(&home, "ghost-fact", "h5"));

    // Assert
    assert!(context.starts_with("Memory: no fact matched"), "{context}");
    assert!(!context.contains("### "), "{context}");

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn an_unreadable_hint_counter_means_no_hint_and_a_clean_exit() {
    // Arrange: a directory where the counter file should be.
    let home = scratch_home("hint-counter-unreadable");
    write_graph(&home, BASE_GRAPH);
    fs::create_dir_all(session_file(&home, "h6", HINTS_FILE)).unwrap();

    // Act
    let output = run_prompt_hook(&home, "zebra", "h6");

    // Assert
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(additional_context(&output), "");

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn an_unreadable_hint_counter_keeps_the_weak_facts_but_drops_the_hint() {
    // Arrange
    let home = scratch_home("hint-counter-weak");
    write_named_facts(&home, &[("tidy-notes", "Covers the gizmo setup")]);
    fs::create_dir_all(session_file(&home, "h7", HINTS_FILE)).unwrap();

    // Act
    let output = run_prompt_hook(&home, "gizmo", "h7");

    // Assert
    assert_eq!(output.status.code(), Some(0));
    let context = additional_context(&output);
    assert!(context.contains("### tidy-notes"), "{context}");
    assert!(!context.contains("Memory:"), "{context}");

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn no_hint_follows_once_the_session_fact_cap_is_reached() {
    // Arrange: widget facts match by description only, so they are all weak.
    let home = scratch_home("hint-after-fact-cap");
    write_widget_facts(&home, 20);

    // Act: three prompts use the nine-fact session budget.
    let contexts: Vec<String> = (0..4)
        .map(|_| additional_context(&run_prompt_hook(&home, "widgets misbehave", "h8")))
        .collect();

    // Assert
    assert!(contexts[..3].iter().all(|c| c.contains("### widget-")));
    assert_eq!(contexts[3], "");

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn a_strong_fact_shown_earlier_suppresses_a_later_weak_hint() {
    // Arrange: one strong fact and three weak ones, so the per-prompt cap of
    // three leaves a weak fact for the second prompt.
    let home = scratch_home("hint-strong-seen");
    write_named_facts(
        &home,
        &[
            ("alpha-beta-guide", "Plain notes"),
            ("note-one", "Covers the gizmo setup"),
            ("note-two", "Covers the gizmo setup"),
            ("note-three", "Covers the gizmo setup"),
        ],
    );
    let prompt = "alpha-beta-guide gizmo";

    // Act
    let first = additional_context(&run_prompt_hook(&home, prompt, "h9"));
    let second = additional_context(&run_prompt_hook(&home, prompt, "h9"));

    // Assert
    assert!(first.contains("### alpha-beta-guide"), "{first}");
    assert!(second.contains("### note-"), "{second}");
    assert!(!first.contains("Memory:") && !second.contains("Memory:"));

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn the_hint_points_at_the_memory_root_when_the_repo_has_no_slug() {
    // Arrange: the scratch repo has no origin remote.
    let home = scratch_home("hint-empty-slug");
    write_graph(&home, BASE_GRAPH);

    // Act
    let context = additional_context(&run_prompt_hook(&home, "zebra", "h10"));

    // Assert
    assert!(context.contains("grep ~/.config/playbook/memory/ and the repo"));
    assert!(!context.contains("memory//"), "{context}");

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn the_hint_names_the_repo_fact_folder_and_stays_short_for_a_very_long_slug() {
    // Arrange
    let home = scratch_home("hint-long-slug");
    write_graph(&home, BASE_GRAPH);
    let owner = "o".repeat(200);
    let name = "r".repeat(200);
    set_origin(&home, &format!("https://github.com/{owner}/{name}.git"));
    let prompt = (0..20)
        .map(|n| format!("term{n}{}", "x".repeat(40)))
        .collect::<Vec<_>>()
        .join(" ");

    // Act
    let context = additional_context(&run_prompt_hook(&home, &prompt, "h11"));

    // Assert
    assert!(context.starts_with("Memory: no fact matched"), "{context}");
    assert!(
        context.contains("~/.config/playbook/memory/ooo"),
        "{context}"
    );
    assert!(
        context.chars().count() <= 400,
        "{}",
        context.chars().count()
    );

    let _ = fs::remove_dir_all(&home);
}
