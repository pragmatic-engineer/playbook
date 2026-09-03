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
    // Arrange: a fake HOME with a memory.graph.json carrying one fact in scope for
    // a repo whose origin matches REPO_SLUG.
    let work = scratch_dir("graph-slice");
    let repo_slug = "acme/widget";
    let repo_dir = work.join("repo");
    init_repo_with_origin(&repo_dir, &format!("git@github.com:{repo_slug}.git"));

    let home = work.join("home-graph");
    let memory_dir = home.join(".claude").join("memory");
    fs::create_dir_all(&memory_dir).unwrap();
    fs::write(
        memory_dir.join("memory.graph.json"),
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

/// ADR 0008 WU-1: the graph-backed slice has no cap today, unlike the legacy
/// fallback (`read_legacy_memory`, capped at 16000 chars). A repo-slice with
/// enough facts to exceed that cap must still be truncated: an early fact
/// (guaranteed within the first 16000 chars) survives, a fact deliberately
/// placed past that boundary does not.
#[test]
fn session_init_caps_the_graph_backed_slice_like_the_legacy_fallback() {
    // Arrange: ~120 facts, each with a ~150-char description, so the
    // rendered "Facts:" section alone exceeds 16000 chars well before the
    // last node. Zero-padded names sort in the same order memory-context.sh
    // renders them (`sort_by(.name)`), so "fact-001" is early and
    // "fact-120" is guaranteed past the cap.
    let work = scratch_dir("graph-cap");
    let repo_slug = "acme/widget";
    let repo_dir = work.join("repo");
    init_repo_with_origin(&repo_dir, &format!("git@github.com:{repo_slug}.git"));

    let home = work.join("home-cap");
    let memory_dir = home.join(".claude").join("memory");
    fs::create_dir_all(&memory_dir).unwrap();
    let padding = "x".repeat(140);
    let nodes: Vec<String> = (1..=120)
        .map(|n| {
            format!(
                r#"{{"id":"{repo_slug}/f{n:03}","file":"{repo_slug}/f{n:03}.md","scope":"project","type":"project","name":"fact-{n:03}","description":"desc-{n:03}-{padding}","project":"{repo_slug}"}}"#
            )
        })
        .collect();
    fs::write(
        memory_dir.join("memory.graph.json"),
        format!(r#"{{"nodes":[{}],"edges":[]}}"#, nodes.join(",")),
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
        context.contains("fact-001"),
        "an early fact, well within the cap, should survive: {context}"
    );
    assert!(
        !context.contains("fact-120"),
        "a fact placed past the 16000-char cap should be truncated away: {context}"
    );
}

#[test]
fn session_init_falls_back_to_the_legacy_memory_index() {
    // Arrange: a fake HOME with the legacy MEMORY.md index but no memory.graph.json.
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
        memory_dir.join("memory.graph.json"),
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
// session-init: pinned and usage-promoted facts, injected unconditionally
// ---------------------------------------------------------------------

/// A fact marked `pinned: true` in `memory.graph.json` must inject even
/// though nothing anchors or prompt-matches it, and even with the general
/// memory-slice machinery entirely inactive (no `CLAUDE_PLUGIN_ROOT`, no
/// legacy `MEMORY.md`): the pinned/promoted block reads the graph directly
/// and does not depend on that machinery at all.
#[test]
fn session_init_injects_a_pinned_fact_independent_of_general_memory_slice() {
    // Arrange
    let work = scratch_dir("pinned-fact");
    let repo_slug = "acme/widget";
    let repo_dir = work.join("repo");
    init_repo_with_origin(&repo_dir, &format!("git@github.com:{repo_slug}.git"));

    let home = work.join("home-pinned");
    let memory_dir = home.join(".claude").join("memory");
    fs::create_dir_all(&memory_dir).unwrap();
    fs::write(
        memory_dir.join("memory.graph.json"),
        format!(
            r#"{{"nodes":[{{"id":"{repo_slug}/pinned-fact","file":"{repo_slug}/pinned-fact.md","scope":"project","type":"project","name":"pinned-fact-name","description":"Pinned fact description text.","project":"{repo_slug}","pinned":true}}],"edges":[]}}"#
        ),
    )
    .unwrap();

    // Act: no CLAUDE_PLUGIN_ROOT, so the general memory-slice machinery
    // never fires and cannot be the reason this fact shows up.
    let outcome = run_hook("session-init", &repo_dir, &home, "{}", &[]);
    let context = additional_context(&outcome.stdout);

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert!(
        context.contains("pinned-fact-name"),
        "a pinned fact should inject with no general memory slice active: {context}"
    );
    assert!(
        context.contains("Pinned fact description text."),
        "the pinned fact's description should be included: {context}"
    );
}

/// A fact marked `promoted: true` in `memory.signals.json` injects the same
/// way a pinned fact does, with the general memory-slice machinery entirely
/// inactive.
#[test]
fn session_init_injects_a_promoted_fact_independent_of_general_memory_slice() {
    // Arrange
    let work = scratch_dir("promoted-fact");
    let repo_slug = "acme/widget";
    let repo_dir = work.join("repo");
    init_repo_with_origin(&repo_dir, &format!("git@github.com:{repo_slug}.git"));

    let home = work.join("home-promoted");
    let memory_dir = home.join(".claude").join("memory");
    fs::create_dir_all(&memory_dir).unwrap();
    fs::write(
        memory_dir.join("memory.graph.json"),
        format!(
            r#"{{"nodes":[{{"id":"{repo_slug}/promoted-fact","file":"{repo_slug}/promoted-fact.md","scope":"project","type":"project","name":"promoted-fact-name","description":"Promoted fact description text.","project":"{repo_slug}"}}],"edges":[]}}"#
        ),
    )
    .unwrap();
    fs::write(
        memory_dir.join("memory.signals.json"),
        format!(r#"{{"nodes":{{"{repo_slug}/promoted-fact":{{"hits":3,"promoted":true}}}}}}"#),
    )
    .unwrap();

    // Act
    let outcome = run_hook("session-init", &repo_dir, &home, "{}", &[]);
    let context = additional_context(&outcome.stdout);

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert!(
        context.contains("promoted-fact-name"),
        "a promoted fact should inject with no general memory slice active: {context}"
    );
    assert!(
        context.contains("Promoted fact description text."),
        "the promoted fact's description should be included: {context}"
    );
}

/// A pinned fact belonging to a DIFFERENT repo than the current session must
/// not inject: `pinned`/`promoted` are still subject to the same
/// global-or-matching-project scope check every other memory injection path
/// in this codebase applies. A regression that dropped that check would
/// leak one project's pinned facts into every other repo's sessions.
#[test]
fn a_pinned_fact_from_a_different_repo_does_not_inject() {
    // Arrange
    let work = scratch_dir("pinned-fact-other-repo");
    let repo_slug = "acme/widget";
    let other_slug = "acme/other-repo";
    let repo_dir = work.join("repo");
    init_repo_with_origin(&repo_dir, &format!("git@github.com:{repo_slug}.git"));

    let home = work.join("home-pinned-other-repo");
    let memory_dir = home.join(".claude").join("memory");
    fs::create_dir_all(&memory_dir).unwrap();
    fs::write(
        memory_dir.join("memory.graph.json"),
        format!(
            r#"{{"nodes":[{{"id":"{other_slug}/other-repo-fact","file":"{other_slug}/other-repo-fact.md","scope":"project","type":"project","name":"other-repo-fact-name","description":"Should never appear here.","project":"{other_slug}","pinned":true}}],"edges":[]}}"#
        ),
    )
    .unwrap();

    // Act
    let outcome = run_hook("session-init", &repo_dir, &home, "{}", &[]);
    let context = additional_context(&outcome.stdout);

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert!(
        !context.contains("other-repo-fact-name"),
        "a pinned fact scoped to a different repo must not leak into this session: {context}"
    );
}

/// A global-scope promoted fact injects regardless of which repo the
/// session is in, the same global-always-in-scope rule every other memory
/// injection path in this codebase already follows.
#[test]
fn a_global_promoted_fact_injects_regardless_of_repo() {
    // Arrange
    let work = scratch_dir("promoted-fact-global");
    let repo_slug = "acme/widget";
    let repo_dir = work.join("repo");
    init_repo_with_origin(&repo_dir, &format!("git@github.com:{repo_slug}.git"));

    let home = work.join("home-promoted-global");
    let memory_dir = home.join(".claude").join("memory");
    fs::create_dir_all(&memory_dir).unwrap();
    fs::write(
        memory_dir.join("memory.graph.json"),
        r#"{"nodes":[{"id":"global/global-promoted-fact","file":"global-promoted-fact.md","scope":"global","type":"reference","name":"global-promoted-fact-name","description":"Global fact, any repo."}],"edges":[]}"#,
    )
    .unwrap();
    fs::write(
        memory_dir.join("memory.signals.json"),
        r#"{"nodes":{"global/global-promoted-fact":{"hits":3,"promoted":true}}}"#,
    )
    .unwrap();

    // Act
    let outcome = run_hook("session-init", &repo_dir, &home, "{}", &[]);
    let context = additional_context(&outcome.stdout);

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert!(
        context.contains("global-promoted-fact-name"),
        "a global-scope promoted fact should inject regardless of repo: {context}"
    );
}

// ---------------------------------------------------------------------
// session-init: ADR 0008 WU-3, reload a persisted handoff at SessionStart
// ---------------------------------------------------------------------

/// The exact slugify `src/cc/mod.rs::project_slug` uses: every
/// non-alphanumeric character becomes `-`. Duplicated here rather than
/// imported, since this is a black-box test of the compiled binary.
fn project_slug(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

static HANDOFF_SUFFIX_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write a handoff file the way `skills/session-handoff/SKILL.md` does:
/// `<slug>-<unique-suffix>.md`, never a single fixed path, so two sessions
/// in the same directory get distinct files. The counter (rather than a
/// real timestamp+pid) keeps this deterministic and collision-free across
/// repeated calls within one test.
fn write_handoff(home: &Path, cwd: &Path, contents: &str) -> PathBuf {
    let n = HANDOFF_SUFFIX_COUNTER.fetch_add(1, Ordering::Relaxed);
    write_handoff_suffixed(home, cwd, contents, &format!("{n}-test"))
}

fn write_handoff_suffixed(home: &Path, cwd: &Path, contents: &str, suffix: &str) -> PathBuf {
    let slug = project_slug(&cwd.to_string_lossy());
    let dir = home.join(".claude").join("runtime").join("handoff");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{slug}-{suffix}.md"));
    fs::write(&path, contents).unwrap();
    path
}

/// Run session-init with `PWD` explicitly set to `cwd`. `Command::current_dir`
/// alone does not update the child's `PWD` env var (that is a shell
/// convention, not something chdir does), and `logical_cwd()` prefers `PWD`
/// over `current_dir()` specifically for symlink-consistency reasons
/// (macOS `/tmp` and `/var` resolve through `/private`). Real Claude Code
/// sets `PWD` to match; a test that omits it would silently exercise the
/// wrong path and pass for the wrong reason.
fn run_session_init_at(cwd: &Path, home: &Path) -> Outcome {
    run_hook(
        "session-init",
        cwd,
        home,
        "{}",
        &[
            ("CLAUDE_PLUGIN_ROOT", plugin_root()),
            ("PWD", cwd.to_str().unwrap()),
        ],
    )
}

fn run_session_init_with_source(cwd: &Path, home: &Path, source: &str) -> Outcome {
    run_hook(
        "session-init",
        cwd,
        home,
        &format!(r#"{{"source":"{source}"}}"#),
        &[
            ("CLAUDE_PLUGIN_ROOT", plugin_root()),
            ("PWD", cwd.to_str().unwrap()),
        ],
    )
}

#[test]
fn session_init_injects_a_present_handoff_and_deletes_it() {
    // Arrange
    let work = scratch_dir("handoff-present");
    let repo_dir = work.join("repo");
    fs::create_dir_all(&repo_dir).unwrap();
    let home = work.join("home");
    let handoff_path = write_handoff(&home, &repo_dir, "HANDOFFMARKER decisions from last time");

    // Act
    let outcome = run_session_init_at(&repo_dir, &home);
    let context = additional_context(&outcome.stdout);

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert!(
        context.contains("HANDOFFMARKER"),
        "a present handoff should be injected: {context}"
    );
    assert!(
        !handoff_path.exists(),
        "the handoff file should be deleted after being read (read-once)"
    );
}

#[test]
fn session_init_no_handoff_file_emits_no_handoff_block_and_no_error() {
    // Arrange: no handoff file written at all.
    let work = scratch_dir("handoff-absent");
    let repo_dir = work.join("repo");
    fs::create_dir_all(&repo_dir).unwrap();
    let home = work.join("home");

    // Act
    let outcome = run_session_init_at(&repo_dir, &home);

    // Assert
    assert_eq!(outcome.exit_code, 0, "an absent handoff should not error");
    assert!(
        !additional_context(&outcome.stdout).contains("Handoff from your previous session"),
        "no handoff block should be emitted when there is nothing to reload"
    );
}

#[test]
fn session_init_reloads_the_handoff_identically_on_clear_and_on_startup() {
    // Arrange: two independent scratch setups, one per source value, since
    // the handoff is consumed (deleted) by the first read.
    for source in ["clear", "startup"] {
        let work = scratch_dir(&format!("handoff-source-{source}"));
        let repo_dir = work.join("repo");
        fs::create_dir_all(&repo_dir).unwrap();
        let home = work.join("home");
        write_handoff(&home, &repo_dir, "HANDOFFMARKER present for this source");

        // Act
        let outcome = run_session_init_with_source(&repo_dir, &home, source);

        // Assert
        let context = additional_context(&outcome.stdout);
        assert!(
            context.contains("HANDOFFMARKER"),
            "source={source} should inject the handoff exactly like any other source: {context}"
        );
    }
}

#[test]
fn session_init_treats_a_handoff_older_than_14_days_as_stale_and_clears_it() {
    // Arrange
    let work = scratch_dir("handoff-stale");
    let repo_dir = work.join("repo");
    fs::create_dir_all(&repo_dir).unwrap();
    let home = work.join("home");
    let handoff_path = write_handoff(&home, &repo_dir, "HANDOFFMARKER should never surface");

    let fifteen_days_ago =
        std::time::SystemTime::now() - std::time::Duration::from_secs(15 * 86400);
    let file = fs::File::open(&handoff_path).unwrap();
    file.set_modified(fifteen_days_ago).unwrap();

    // Act
    let outcome = run_session_init_at(&repo_dir, &home);
    let context = additional_context(&outcome.stdout);

    // Assert
    assert_eq!(outcome.exit_code, 0, "a stale handoff should not error");
    assert!(
        !context.contains("HANDOFFMARKER"),
        "a handoff older than the 14-day cap should not be injected: {context}"
    );
    assert!(
        !handoff_path.exists(),
        "a stale handoff should still be cleared, ending the backstop it exists for"
    );
}

#[test]
fn session_init_never_reads_a_handoff_written_under_a_different_worktree() {
    // Arrange: two distinct working directories under the same HOME
    // (simulating two worktrees of one repo), a handoff only under A's slug.
    let work = scratch_dir("handoff-worktree");
    let home = work.join("home");
    let worktree_a = work.join("worktree-a");
    let worktree_b = work.join("worktree-b");
    fs::create_dir_all(&worktree_a).unwrap();
    fs::create_dir_all(&worktree_b).unwrap();
    let path_a = write_handoff(
        &home,
        &worktree_a,
        "HANDOFFMARKER belongs only to worktree A",
    );

    // Act: SessionStart runs from worktree B.
    let outcome = run_session_init_at(&worktree_b, &home);
    let context = additional_context(&outcome.stdout);

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert!(
        !context.contains("HANDOFFMARKER"),
        "worktree B must never see worktree A's handoff: {context}"
    );
    // Worktree A's own handoff must be untouched by B's run: not read, not deleted.
    assert!(
        path_a.exists(),
        "worktree B's SessionStart must not delete worktree A's handoff file"
    );
}

#[test]
fn session_init_injects_both_handoffs_from_two_concurrent_sessions_in_the_same_directory() {
    // Arrange: two `cc` sessions working the same directory both ran
    // /playbook:session-handoff close together. Each writes its own
    // uniquely-suffixed file (skills/session-handoff/SKILL.md's design),
    // so neither overwrites the other the way a single fixed path would.
    let work = scratch_dir("handoff-concurrent");
    let repo_dir = work.join("repo");
    fs::create_dir_all(&repo_dir).unwrap();
    let home = work.join("home");
    let path_1 = write_handoff_suffixed(&home, &repo_dir, "HANDOFFMARKER-ONE", "1000-111");
    let path_2 = write_handoff_suffixed(&home, &repo_dir, "HANDOFFMARKER-TWO", "1001-222");

    // Act
    let outcome = run_session_init_at(&repo_dir, &home);
    let context = additional_context(&outcome.stdout);

    // Assert: both surface in one SessionStart, neither silently lost.
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert!(
        context.contains("HANDOFFMARKER-ONE"),
        "the first concurrent session's handoff should still surface: {context}"
    );
    assert!(
        context.contains("HANDOFFMARKER-TWO"),
        "the second concurrent session's handoff should still surface: {context}"
    );
    assert!(
        context.contains("2 of 2 found"),
        "more than one match should say so, not read like a single handoff: {context}"
    );
    assert!(
        !path_1.exists() && !path_2.exists(),
        "both matched files should be consumed (read-once, mirroring the single-handoff case)"
    );
}

#[test]
fn session_init_caps_injected_handoffs_but_still_deletes_every_match() {
    // Arrange: four fresh handoffs in the same directory, more than
    // HANDOFF_MAX_INJECTED (3). Explicit, spread-out mtimes make the
    // freshest-first ordering deterministic instead of relying on write
    // order landing in the same filesystem-time quantum.
    let work = scratch_dir("handoff-cap");
    let repo_dir = work.join("repo");
    fs::create_dir_all(&repo_dir).unwrap();
    let home = work.join("home");

    let now = std::time::SystemTime::now();
    let mut paths = Vec::new();
    for (n, age_hours) in [(1, 4), (2, 3), (3, 2), (4, 1)] {
        let marker = format!("HANDOFFMARKER-{n}");
        let path = write_handoff_suffixed(&home, &repo_dir, &marker, &format!("cap-{n}"));
        let file = fs::File::open(&path).unwrap();
        file.set_modified(now - std::time::Duration::from_secs(age_hours * 3600))
            .unwrap();
        paths.push(path);
    }

    // Act
    let outcome = run_session_init_at(&repo_dir, &home);
    let context = additional_context(&outcome.stdout);

    // Assert: only the 3 freshest (4, 3, 2) are injected; the oldest (1) is
    // not, but every file is still gone afterward.
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    assert!(
        context.contains("HANDOFFMARKER-4")
            && context.contains("HANDOFFMARKER-3")
            && context.contains("HANDOFFMARKER-2"),
        "the 3 freshest matches should be injected: {context}"
    );
    assert!(
        !context.contains("HANDOFFMARKER-1"),
        "a 4th match beyond the injection cap should not be injected: {context}"
    );
    assert!(
        paths.iter().all(|p| !p.exists()),
        "every matched file should still be deleted, capped or not"
    );
}

// ---------------------------------------------------------------------
// session-init: the six zeroed counters plus start-ts
// ---------------------------------------------------------------------

#[test]
fn session_init_zeroes_exactly_the_six_counter_files() {
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
        "capture-crossings",
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
        "capture-crossings",
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

#[test]
fn session_init_resume_with_matching_hash_emits_no_drift_warning() {
    // Arrange: the session's stored config-hash is seeded with the CURRENT
    // hash for this empty scratch HOME, so there is no drift at all. An
    // inverted `!=` in the drift check would still warn here.
    let home = scratch_dir("drift-resume-match");
    let repo_dir = scratch_dir("drift-resume-match-repo");
    let session_id = "sid-resume-match";
    let session_dir = home.join(".claude").join("runtime").join(session_id);
    fs::create_dir_all(&session_dir).unwrap();
    let current_hash = empty_config_hash(&home);
    fs::write(session_dir.join("config-hash"), &current_hash).unwrap();

    // Act: resume, with the stored hash already matching.
    let resumed = run_hook(
        "session-init",
        &repo_dir,
        &home,
        &format!(r#"{{"session_id":"{session_id}","source":"resume"}}"#),
        &[("CLAUDE_PLUGIN_ROOT", plugin_root())],
    );

    // Assert: no drift warning anywhere in the output.
    assert_eq!(resumed.exit_code, 0, "hook should exit 0 on resume");
    let message = system_message(&resumed.stdout);
    assert!(
        !message.contains("drifted"),
        "resume with a matching hash should not warn: {message}"
    );
    let context = additional_context(&resumed.stdout);
    assert!(
        !context.contains("config hash has changed"),
        "resume additionalContext should not mention drift when the hash matches: {context}"
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
    assert_eq!(
        marker, "logout\n",
        "marker should hold the reason plus a trailing newline, byte for byte"
    );
}

// ---------------------------------------------------------------------
// session-clean-exit: auto-learn queueing
// (hooks/session-clean-exit.test.sh cases 3-5)
// ---------------------------------------------------------------------

/// The real git worktree root for `dir`, resolved via `git rev-parse
/// --show-toplevel`, so a test can predict the auto-learn queue's slugified
/// filename without hard-coding a path that may differ once the OS resolves
/// symlinks (e.g. macOS's `/tmp` -> `/private/tmp`).
fn git_toplevel(dir: &Path) -> String {
    let output = Command::new("git")
        .args(["-C", dir.to_str().unwrap(), "rev-parse", "--show-toplevel"])
        .output()
        .expect("git should be available");
    assert!(
        output.status.success(),
        "git rev-parse --show-toplevel failed"
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

/// Mirrors `session_clean_exit::slugify`: replace every character outside
/// `[A-Za-z0-9_.-]` with `_`, matching the regex in
/// hooks/session-clean-exit.py:71.
fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[test]
fn session_clean_exit_queues_auto_learn_flag_with_expected_shape() {
    // Arrange: enough edits recorded to clear the default threshold.
    let home = scratch_dir("auto-learn-flag");
    let session_dir = seeded_session_dir(&home, "sid-flag");
    fs::write(session_dir.join("edit-count"), "9").unwrap();
    let repo_dir = scratch_dir("auto-learn-flag-repo");
    init_repo_with_origin(&repo_dir, "https://github.com/acme/widget.git");
    let expected_filename = format!("{}.json", slugify(&git_toplevel(&repo_dir)));

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
    assert_eq!(
        entries[0].file_name().to_string_lossy(),
        expected_filename,
        "queued flag filename should be the slugified repo root"
    );
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
fn session_clean_exit_queues_correctly_when_the_lock_is_already_held() {
    // Arrange: the queue is keyed by repo, not session, so two sessions in
    // the same repo can race here. Pre-create the lock directory to
    // simulate another session already writing this repo's flag. The lock
    // is advisory and fails open after its retry budget (a hook must never
    // hang on contention), so this must still succeed and queue a correct,
    // complete flag, not a half-written or missing one.
    let home = scratch_dir("auto-learn-lock-held");
    let session_dir = seeded_session_dir(&home, "sid-lock-held");
    fs::write(session_dir.join("edit-count"), "9").unwrap();
    let repo_dir = scratch_dir("auto-learn-lock-held-repo");
    init_repo_with_origin(&repo_dir, "https://github.com/acme/widget.git");
    let slug = slugify(&git_toplevel(&repo_dir));
    let to_learn_dir = home.join(".claude").join("runtime").join("to-learn");
    fs::create_dir_all(&to_learn_dir).unwrap();
    let lock_dir = to_learn_dir.join(format!("{slug}.json.lock"));
    fs::create_dir(&lock_dir).expect("pre-creating the lock dir should succeed");

    // Act
    let outcome = run_hook(
        "session-clean-exit",
        &repo_dir,
        &home,
        r#"{"session_id":"sid-lock-held","reason":"logout"}"#,
        &[],
    );

    // Assert
    assert_eq!(
        outcome.exit_code, 0,
        "the hook must fail open, not hang or error, when the lock is already held"
    );
    let dest = to_learn_dir.join(format!("{slug}.json"));
    let contents = fs::read_to_string(&dest).expect("the flag should still be queued correctly");
    let flag: serde_json::Value = serde_json::from_str(&contents)
        .expect("the queued flag must be complete, valid JSON, not a torn write");
    assert_eq!(flag["edits"], 9);
    assert!(
        lock_dir.exists(),
        "a writer that did not acquire the lock must not remove it"
    );
}

#[test]
fn session_clean_exit_at_default_threshold_queues_a_flag() {
    // Arrange: edit-count sits exactly at the default threshold of 5,
    // pinning the `<` comparison `queue_auto_learn` uses against a `<=`
    // mutation that would wrongly skip queuing right at the boundary.
    let home = scratch_dir("at-threshold");
    let session_dir = seeded_session_dir(&home, "sid-at-threshold");
    fs::write(session_dir.join("edit-count"), "5").unwrap();
    let repo_dir = scratch_dir("at-threshold-repo");
    init_repo_with_origin(&repo_dir, "https://github.com/acme/widget.git");

    // Act
    let outcome = run_hook(
        "session-clean-exit",
        &repo_dir,
        &home,
        r#"{"session_id":"sid-at-threshold","reason":"logout"}"#,
        &[],
    );

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    let to_learn_dir = home.join(".claude").join("runtime").join("to-learn");
    let entries: Vec<_> = fs::read_dir(&to_learn_dir)
        .expect("to-learn dir should exist")
        .flatten()
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "exactly at the default threshold of 5 edits should still queue a flag"
    );
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
fn session_clean_exit_padded_min_edits_env_var_still_parses() {
    // Arrange: AUTO_LEARN_MIN_EDITS carries leading whitespace, the same way
    // python's `int(...)` would still accept it and parse 3. edit-count sits
    // exactly at that padded threshold, so a Rust parse that instead fell
    // back to the default of 5 would wrongly skip the queue.
    let home = scratch_dir("padded-min-edits");
    let session_dir = seeded_session_dir(&home, "sid-padded");
    fs::write(session_dir.join("edit-count"), "3").unwrap();
    let repo_dir = scratch_dir("padded-min-edits-repo");
    init_repo_with_origin(&repo_dir, "https://github.com/acme/widget.git");

    // Act
    let outcome = run_hook(
        "session-clean-exit",
        &repo_dir,
        &home,
        r#"{"session_id":"sid-padded","reason":"logout"}"#,
        &[("AUTO_LEARN_MIN_EDITS", " 3")],
    );

    // Assert
    assert_eq!(outcome.exit_code, 0, "hook should exit 0");
    let to_learn_dir = home.join(".claude").join("runtime").join("to-learn");
    let entries: Vec<_> = fs::read_dir(&to_learn_dir)
        .expect("to-learn dir should exist")
        .flatten()
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "a padded threshold should still parse to 3, so 3 edits should queue a flag"
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
