// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Behavioural tests for the `memory-anchors` hook, the sole reader of
//! `~/.claude/memory/memory.graph.json`. Exercised black-box, through the
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
    home.join(".claude")
        .join("memory")
        .join("memory.graph.json")
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

// --- memory-anchors: UserPromptSubmit prompt-time recall ------------------
//
// ADR 0008 WU-0: the same anchor index above, now also matched on
// UserPromptSubmit (prompt text and this-session touched files), injecting
// fact BODIES rather than the name/description line PreToolUse emits.
// Scenario numbering follows the WU-0 brief in
// docs/adr/0008-bounded-memory-injection-with-prompt-recall-and-handoff-continuity-blueprint.md.

/// Writes a fact's markdown body under `~/.claude/memory/<relpath>`, the
/// path `Node.file` is relative to (`rebuild_memory_graph.rs` builds it via
/// `strip_prefix` against the memory root). Creates parent dirs as needed.
fn write_fact_body(home: &Path, relpath: &str, content: &str) {
    let path = home.join(".claude").join("memory").join(relpath);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Appends one `edits.jsonl` line in the exact shape `post_edit_track.rs`
/// writes (`EditRecord { path, ts }`), under this session's runtime dir, so
/// the touched-file signal has something real to match against.
fn write_edit_record(home: &Path, session_id: &str, abs_path: &str, ts: u64) {
    let dir = home.join(".claude").join("runtime").join(session_id);
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

    let _ = fs::remove_dir_all(&home);
}

/// WU-0 scenario 4: no match emits nothing. Not a regression pin (both old
/// and new code emit nothing here), a boundary test catching a malformed or
/// empty-but-present block specifically.
#[test]
fn prompt_with_no_match_emits_no_additional_context() {
    // Arrange
    let home = scratch_home("prompt-nomatch");
    write_graph(&home, BASE_GRAPH);

    // Act
    let output = run_prompt_hook(&home, "completely unrelated words here", "p4");

    // Assert
    assert_eq!(
        stdout_of(&output),
        "",
        "a non-matching prompt should emit nothing, not an empty block"
    );

    let _ = fs::remove_dir_all(&home);
}

/// WU-0 scenario 5: a missing or corrupted anchor index degrades to
/// silence, not a crash, matching this codebase's "never panics, degrades
/// to say nothing" invariant.
#[test]
fn corrupted_anchor_index_degrades_to_silence_not_a_crash() {
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
    let idx_dir = home.join(".claude").join("runtime").join(sid);
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
    assert_eq!(
        stdout_of(&output),
        "",
        "a corrupted index should degrade to no additionalContext, not garbage output"
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
    home.join(".claude")
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
/// hooks/memory-anchors.py, feeding both implementations the same
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
