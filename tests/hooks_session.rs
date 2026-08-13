// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for the `session-init` and `session-clean-exit` hooks,
//! ported from `hooks/session-init.test.sh` and
//! `hooks/session-clean-exit.test.sh`. Runs the built `playbook` binary as
//! a subprocess, exactly as Claude Code would, against a scratch `$HOME`
//! and a scratch git repo, never the real `~/.claude`.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

fn playbook_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_playbook"))
}

/// The repo checkout root: where `hooks/lib/config-hash.sh` and
/// `shell/memory-context.sh` actually live, so tests can point
/// `CLAUDE_PLUGIN_ROOT` at real scripts the same way Claude Code would.
fn plugin_root() -> &'static str {
    env!("CARGO_MANIFEST_DIR")
}

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory under the OS temp dir, unique per call so
/// parallel tests never collide. Never the real `$HOME`.
fn scratch_dir(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = env::temp_dir().join(format!(
        "playbook-hooks-session-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir should be creatable");
    dir
}

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git should be on PATH");
    assert!(status.success(), "git {args:?} failed in {dir:?}");
}

/// A throwaway git repo with an `origin` remote, so the repo-slug and
/// git-toplevel derivations resolve the same way they would in a real
/// checkout. Local-only git identity and no signing, so the init commit
/// never touches the real user's global git config.
fn init_repo_with_origin(dir: &Path, origin_url: &str) {
    fs::create_dir_all(dir).expect("repo dir should be creatable");
    run_git(dir, &["init", "--quiet"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    run_git(dir, &["config", "commit.gpgsign", "false"]);
    run_git(dir, &["remote", "add", "origin", origin_url]);
    run_git(dir, &["commit", "--quiet", "--allow-empty", "-m", "init"]);
}

struct Outcome {
    exit_code: i32,
    stdout: String,
}

/// Run `playbook hook <hook>` the way Claude Code would: cwd, HOME, stdin
/// payload, plus any extra env vars the scenario needs.
fn run_hook(
    hook: &str,
    cwd: &Path,
    home: &Path,
    stdin: &str,
    extra_env: &[(&str, &str)],
) -> Outcome {
    let mut command = Command::new(playbook_bin());
    command
        .arg("hook")
        .arg(hook)
        .current_dir(cwd)
        .env("HOME", home)
        .env_remove("HOOK_INPUT")
        .env_remove("CLAUDE_PLUGIN_ROOT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("playbook binary should spawn");
    child
        .stdin
        .take()
        .expect("stdin should be piped")
        .write_all(stdin.as_bytes())
        .expect("writing stdin should succeed");
    let output = child
        .wait_with_output()
        .expect("playbook binary should exit");
    Outcome {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    }
}

/// `.hookSpecificOutput.additionalContext` from a hook's stdout, or empty
/// if absent or the stdout does not parse as JSON.
fn additional_context(stdout: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout) else {
        return String::new();
    };
    value
        .get("hookSpecificOutput")
        .and_then(|h| h.get("additionalContext"))
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string()
}

/// `.systemMessage` from a hook's stdout, or empty if absent or the stdout
/// does not parse as JSON.
fn system_message(stdout: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout) else {
        return String::new();
    };
    value
        .get("systemMessage")
        .and_then(|m| m.as_str())
        .unwrap_or_default()
        .to_string()
}

// ---------------------------------------------------------------------
// session-init: project memory slice (hooks/session-init.test.sh cases 1-4)
// ---------------------------------------------------------------------

#[test]
fn session_init_injects_the_graph_backed_slice() {
    // Arrange: a fake HOME with a graph.json carrying one fact in scope for
    // a repo whose origin matches REPO_SLUG.
    let work = scratch_dir("graph-slice");
    let repo_slug = "acme/widget";
    let repo_dir = work.join("repo");
    init_repo_with_origin(&repo_dir, &format!("git@github.com:{repo_slug}.git"));

    let home = work.join("home-graph");
    let memory_dir = home.join(".claude").join("memory");
    fs::create_dir_all(&memory_dir).unwrap();
    fs::write(
        memory_dir.join("graph.json"),
        format!(
            r#"{{"nodes":[{{"id":"{repo_slug}/f1","file":"{repo_slug}/f1.md","scope":"project","type":"project","name":"widget-fact-one","description":"The widget module talks to the sprocket service.","project":"{repo_slug}"}}],"edges":[]}}"#
        ),
    )
    .unwrap();

    // Act
    let outcome = run_hook(
        "session-init",
        &repo_dir,
        &home,
        "{}",
        &[("CLAUDE_PLUGIN_ROOT", plugin_root())],
    );
    let context = additional_context(&outcome.stdout);

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert!(
        serde_json::from_str::<serde_json::Value>(&outcome.stdout).is_ok(),
        "stdout should be valid JSON: {}",
        outcome.stdout
    );
    assert!(
        context.contains("widget-fact-one"),
        "additionalContext should carry the fact name: {context}"
    );
}

#[test]
fn session_init_falls_back_to_the_legacy_memory_index() {
    // Arrange: a fake HOME with the legacy MEMORY.md index but no graph.json.
    let work = scratch_dir("legacy-index");
    let repo_slug = "acme/widget";
    let repo_dir = work.join("repo");
    init_repo_with_origin(&repo_dir, &format!("git@github.com:{repo_slug}.git"));

    let home = work.join("home-index");
    let legacy_dir = home.join(".claude").join("memory").join(repo_slug);
    fs::create_dir_all(&legacy_dir).unwrap();
    fs::write(
        legacy_dir.join("MEMORY.md"),
        "- legacy-fact-two: an old style index entry\n",
    )
    .unwrap();

    // Act
    let outcome = run_hook(
        "session-init",
        &repo_dir,
        &home,
        "{}",
        &[("CLAUDE_PLUGIN_ROOT", plugin_root())],
    );
    let context = additional_context(&outcome.stdout);

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert!(
        context.contains("legacy-fact-two"),
        "additionalContext should carry the index line: {context}"
    );
}

#[test]
fn session_init_no_memory_store_emits_no_memory_block() {
    // Arrange: a fake HOME with no memory dir whatsoever.
    let work = scratch_dir("no-store");
    let repo_slug = "acme/widget";
    let repo_dir = work.join("repo");
    init_repo_with_origin(&repo_dir, &format!("git@github.com:{repo_slug}.git"));
    let home = scratch_dir("no-store-home");

    // Act
    let outcome = run_hook(
        "session-init",
        &repo_dir,
        &home,
        "{}",
        &[("CLAUDE_PLUGIN_ROOT", plugin_root())],
    );
    let context = additional_context(&outcome.stdout);

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert!(
        !context.contains("Project memory for this repo"),
        "no memory block should be emitted: {context}"
    );
}

#[test]
fn session_init_outside_a_git_repo_emits_no_memory_block() {
    // Arrange: the graph-backed HOME from the first scenario, but run from
    // outside any git repo, so the slug never resolves.
    let work = scratch_dir("non-repo");
    let non_repo_dir = work.join("not-a-repo");
    fs::create_dir_all(&non_repo_dir).unwrap();

    let home = work.join("home-graph");
    let memory_dir = home.join(".claude").join("memory");
    fs::create_dir_all(&memory_dir).unwrap();
    fs::write(
        memory_dir.join("graph.json"),
        r#"{"nodes":[{"id":"acme/widget/f1","file":"acme/widget/f1.md","scope":"project","type":"project","name":"widget-fact-one","description":"desc","project":"acme/widget"}],"edges":[]}"#,
    )
    .unwrap();

    // Act
    let outcome = run_hook(
        "session-init",
        &non_repo_dir,
        &home,
        "{}",
        &[("CLAUDE_PLUGIN_ROOT", plugin_root())],
    );
    let context = additional_context(&outcome.stdout);

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert!(
        !context.contains("Project memory for this repo"),
        "no memory block should be emitted outside a git repo: {context}"
    );
    assert!(
        !context.contains("widget-fact-one"),
        "the fact from the slice should be absent: {context}"
    );
}

// ---------------------------------------------------------------------
// session-init: the five zeroed counters plus start-ts
// ---------------------------------------------------------------------

#[test]
fn session_init_zeroes_exactly_the_five_counter_files() {
    // Arrange: a session directory pre-seeded with non-empty content in
    // every file the hook is expected to zero, plus one unrelated file that
    // must survive untouched.
    let home = scratch_dir("zero-counters");
    let session_id = "sid-zero";
    let session_dir = home.join(".claude").join("runtime").join(session_id);
    fs::create_dir_all(&session_dir).unwrap();
    for name in [
        "search-count",
        "tool-count",
        "edit-count",
        "edits.jsonl",
        "seen-reads",
    ] {
        fs::write(session_dir.join(name), "stale-content").unwrap();
    }
    fs::write(session_dir.join("clean-exit"), "logout").unwrap();

    let repo_dir = scratch_dir("zero-counters-repo");

    // Act
    let outcome = run_hook(
        "session-init",
        &repo_dir,
        &home,
        &format!(r#"{{"session_id":"{session_id}"}}"#),
        &[],
    );

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    for name in [
        "search-count",
        "tool-count",
        "edit-count",
        "edits.jsonl",
        "seen-reads",
    ] {
        let contents = fs::read_to_string(session_dir.join(name)).unwrap();
        assert_eq!(contents, "", "{name} should be zeroed, got '{contents}'");
    }
    let start_ts = fs::read_to_string(session_dir.join("start-ts")).unwrap();
    assert!(
        start_ts.parse::<u64>().is_ok(),
        "start-ts should hold a unix timestamp, got '{start_ts}'"
    );
    let untouched = fs::read_to_string(session_dir.join("clean-exit")).unwrap();
    assert_eq!(
        untouched, "logout",
        "clean-exit is not one of the five counters and must survive untouched"
    );
}

// ---------------------------------------------------------------------
// session-init: resume-only config drift warning
// ---------------------------------------------------------------------

/// The config-hash value `hooks/lib/config-hash.sh` computes for an empty
/// `$HOME/.claude` tree, by running the exact same script the hook shells
/// out to. Avoids hard-coding a sha256 constant in the test.
fn empty_config_hash(home: &Path) -> String {
    let script = Path::new(plugin_root())
        .join("hooks")
        .join("lib")
        .join("config-hash.sh");
    let output = Command::new("bash")
        .arg("-c")
        .arg(". \"$1\"; config_hash")
        .arg("_")
        .arg(&script)
        .env("HOME", home)
        .output()
        .expect("bash should run config-hash.sh");
    assert!(output.status.success(), "config-hash.sh should succeed");
    String::from_utf8(output.stdout)
        .expect("config-hash.sh output should be UTF-8")
        .trim()
        .to_string()
}

#[test]
fn session_init_drift_warning_fires_only_on_resume() {
    // Arrange: a session directory whose stored config-hash is deliberately
    // stale, so the current hash (of this empty scratch HOME) will not
    // match it.
    let home = scratch_dir("drift-resume");
    let repo_dir = scratch_dir("drift-resume-repo");
    let session_id = "sid-resume";
    let session_dir = home.join(".claude").join("runtime").join(session_id);
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(session_dir.join("config-hash"), "stale0000stale00").unwrap();
    let expected_hash = empty_config_hash(&home);
    assert_ne!(
        expected_hash, "stale0000stale00",
        "test setup requires the real hash to differ from the seeded stale one"
    );

    // Act: resume.
    let resumed = run_hook(
        "session-init",
        &repo_dir,
        &home,
        &format!(r#"{{"session_id":"{session_id}","source":"resume"}}"#),
        &[("CLAUDE_PLUGIN_ROOT", plugin_root())],
    );

    // Assert: the drift warning fires.
    assert_eq!(resumed.exit_code, 0, "hook should exit 0 on resume");
    let message = system_message(&resumed.stdout);
    assert!(
        message.contains("drifted"),
        "resume with a mismatched hash should warn: {message}"
    );
    let context = additional_context(&resumed.stdout);
    assert!(
        context.contains("config hash has changed"),
        "resume additionalContext should explain the drift: {context}"
    );
    // The stale hash on disk is untouched by a resume (only startup rewrites it).
    let hash_after_resume = fs::read_to_string(session_dir.join("config-hash")).unwrap();
    assert_eq!(hash_after_resume, "stale0000stale00");

    // Arrange: reseed the same stale hash for the startup case.
    fs::write(session_dir.join("config-hash"), "stale0000stale00").unwrap();

    // Act: startup, with the exact same mismatched hash on disk.
    let started = run_hook(
        "session-init",
        &repo_dir,
        &home,
        &format!(r#"{{"session_id":"{session_id}","source":"startup"}}"#),
        &[("CLAUDE_PLUGIN_ROOT", plugin_root())],
    );

    // Assert: startup never warns, but refreshes the stored hash.
    assert_eq!(started.exit_code, 0, "hook should exit 0 on startup");
    assert!(
        system_message(&started.stdout).is_empty(),
        "startup must never emit the drift systemMessage"
    );
    let hash_after_startup = fs::read_to_string(session_dir.join("config-hash")).unwrap();
    assert_eq!(
        hash_after_startup, expected_hash,
        "startup should refresh the stored hash to the freshly computed one"
    );
}

// ---------------------------------------------------------------------
// session-init: shell-out failure degrades quietly
// ---------------------------------------------------------------------

#[test]
fn session_init_degrades_quietly_when_both_shell_outs_are_unreachable() {
    // Arrange: a plugin root that does not exist, so both
    // hooks/lib/config-hash.sh and shell/memory-context.sh are unreachable.
    // Every other additionalContext source is disabled so the only thing
    // left that could emit is the (failed) memory slice, proving the
    // failure produces nothing rather than malformed output.
    let home = scratch_dir("shellout-fail");
    let repo_slug = "acme/widget";
    let repo_dir = scratch_dir("shellout-fail-repo");
    init_repo_with_origin(&repo_dir, &format!("git@github.com:{repo_slug}.git"));

    // Act
    let outcome = run_hook(
        "session-init",
        &repo_dir,
        &home,
        r#"{"session_id":"sid-fail","source":"startup"}"#,
        &[
            ("CLAUDE_PLUGIN_ROOT", "/nonexistent-plugin-root-xyz"),
            ("SKILLS_PRIMER", "0"),
            ("ASYNC_DISCIPLINE", "0"),
            ("AUTO_LEARN_NUDGE", "0"),
        ],
    );

    // Assert: the hook still exits cleanly and prints nothing at all, since
    // there was nothing left to say once both shell-outs failed.
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert_eq!(
        outcome.stdout.trim(),
        "",
        "with everything else disabled, a missing plugin root should yield no output: {}",
        outcome.stdout
    );
}

// ---------------------------------------------------------------------
// session-clean-exit: the three `.reason` cases
// (hooks/session-clean-exit.test.sh cases 1-2, plus the absent case)
// ---------------------------------------------------------------------

fn seeded_session_dir(home: &Path, session_id: &str) -> PathBuf {
    let dir = home.join(".claude").join("runtime").join(session_id);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn session_clean_exit_reason_absent_refreshes_ts_without_marker() {
    // Arrange: a Stop event, which carries no `.reason` at all.
    let home = scratch_dir("reason-absent");
    let session_dir = seeded_session_dir(&home, "sid-absent");
    let repo_dir = scratch_dir("reason-absent-repo");
    init_repo_with_origin(&repo_dir, "https://github.com/acme/widget.git");

    // Act
    let outcome = run_hook(
        "session-clean-exit",
        &repo_dir,
        &home,
        r#"{"session_id":"sid-absent"}"#,
        &[],
    );

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert!(
        session_dir.join("last-clean-ts").is_file(),
        "last-clean-ts should be refreshed even with no reason"
    );
    assert!(
        !session_dir.join("clean-exit").is_file(),
        "no reason should write no clean-exit marker"
    );
}

#[test]
fn session_clean_exit_reason_other_refreshes_ts_without_marker() {
    // Arrange
    let home = scratch_dir("reason-other");
    let session_dir = seeded_session_dir(&home, "sid-other");
    let repo_dir = scratch_dir("reason-other-repo");
    init_repo_with_origin(&repo_dir, "https://github.com/acme/widget.git");

    // Act
    let outcome = run_hook(
        "session-clean-exit",
        &repo_dir,
        &home,
        r#"{"session_id":"sid-other","reason":"other"}"#,
        &[],
    );

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert!(
        session_dir.join("last-clean-ts").is_file(),
        "last-clean-ts should be refreshed"
    );
    assert!(
        !session_dir.join("clean-exit").is_file(),
        "reason 'other' should write no clean-exit marker"
    );
}

#[test]
fn session_clean_exit_real_reason_writes_the_marker() {
    // Arrange
    let home = scratch_dir("reason-real");
    let session_dir = seeded_session_dir(&home, "sid-real");
    let repo_dir = scratch_dir("reason-real-repo");
    init_repo_with_origin(&repo_dir, "https://github.com/acme/widget.git");

    // Act
    let outcome = run_hook(
        "session-clean-exit",
        &repo_dir,
        &home,
        r#"{"session_id":"sid-real","reason":"logout"}"#,
        &[],
    );

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    let marker = fs::read_to_string(session_dir.join("clean-exit")).unwrap();
    assert_eq!(marker.trim(), "logout");
}

// ---------------------------------------------------------------------
// session-clean-exit: auto-learn queueing
// (hooks/session-clean-exit.test.sh cases 3-5)
// ---------------------------------------------------------------------

#[test]
fn session_clean_exit_queues_auto_learn_flag_with_expected_shape() {
    // Arrange: enough edits recorded to clear the default threshold.
    let home = scratch_dir("auto-learn-flag");
    let session_dir = seeded_session_dir(&home, "sid-flag");
    fs::write(session_dir.join("edit-count"), "9").unwrap();
    let repo_dir = scratch_dir("auto-learn-flag-repo");
    init_repo_with_origin(&repo_dir, "https://github.com/acme/widget.git");

    // Act
    let outcome = run_hook(
        "session-clean-exit",
        &repo_dir,
        &home,
        r#"{"session_id":"sid-flag","reason":"logout"}"#,
        &[],
    );

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    let to_learn_dir = home.join(".claude").join("runtime").join("to-learn");
    let entries: Vec<_> = fs::read_dir(&to_learn_dir)
        .expect("to-learn dir should exist")
        .flatten()
        .collect();
    assert_eq!(entries.len(), 1, "exactly one flag should be queued");
    let contents = fs::read_to_string(entries[0].path()).unwrap();
    let flag: serde_json::Value = serde_json::from_str(&contents).unwrap();
    let object = flag.as_object().unwrap();
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, vec!["edits", "repo_root", "session_id", "ts"]);
    assert_eq!(flag["edits"], 9);
    assert_eq!(flag["session_id"], "sid-flag");
}

#[test]
fn session_clean_exit_below_threshold_queues_no_flag() {
    // Arrange: fewer edits than the default threshold of 5.
    let home = scratch_dir("below-threshold");
    let session_dir = seeded_session_dir(&home, "sid-low");
    fs::write(session_dir.join("edit-count"), "2").unwrap();
    let repo_dir = scratch_dir("below-threshold-repo");
    init_repo_with_origin(&repo_dir, "https://github.com/acme/widget.git");

    // Act
    let outcome = run_hook(
        "session-clean-exit",
        &repo_dir,
        &home,
        r#"{"session_id":"sid-low","reason":"clear"}"#,
        &[],
    );

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    let to_learn_dir = home.join(".claude").join("runtime").join("to-learn");
    assert!(
        !to_learn_dir.is_dir() || fs::read_dir(&to_learn_dir).unwrap().next().is_none(),
        "below the threshold, no flag should be queued"
    );
}

#[test]
fn session_clean_exit_auto_learn_nudge_disabled_skips_queue() {
    // Arrange: enough edits, but AUTO_LEARN_NUDGE=0.
    let home = scratch_dir("nudge-disabled");
    let session_dir = seeded_session_dir(&home, "sid-off");
    fs::write(session_dir.join("edit-count"), "9").unwrap();
    let repo_dir = scratch_dir("nudge-disabled-repo");
    init_repo_with_origin(&repo_dir, "https://github.com/acme/widget.git");

    // Act
    let outcome = run_hook(
        "session-clean-exit",
        &repo_dir,
        &home,
        r#"{"session_id":"sid-off","reason":"clear"}"#,
        &[("AUTO_LEARN_NUDGE", "0")],
    );

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    let to_learn_dir = home.join(".claude").join("runtime").join("to-learn");
    assert!(
        !to_learn_dir.is_dir() || fs::read_dir(&to_learn_dir).unwrap().next().is_none(),
        "AUTO_LEARN_NUDGE=0 should disable the queue even above threshold"
    );
}
