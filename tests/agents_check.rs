// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Every scenario in `shell/check-agents.test.sh` (24 scenarios), ported,
//! plus CLI-wiring cases the shell suite never exercises as an automated
//! test: the optional `AGENTS_DIR` argument's default resolution, the
//! not-in-a-git-repo error, and the directory-not-found error.
//!
//! `src/agents/check.rs` already covers the parsing and rule-checking pure
//! functions with unit tests; these drive the real CLI binary against real
//! scratch directories so the directory walk and CLI wiring are proven too.
//! `check_agents_shell_parity_scenarios` table-drives 23 of the 24 shell
//! scenarios (Arrange is the table row, Act/Assert run once per row);
//! `_TEMPLATE.md` skipping and the CLI-wiring cases need different
//! postconditions so they stay as their own tests.

#![allow(dead_code)] // cargo test compiles each tests/*.rs separately.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

const GUARDRAILS_FULL: &str = "\n## Non-negotiable guardrails\n\n1. No dashes, no em dash, no en dash.\n2. Ground every claim, quote exact code.\n3. Zero AI attribution.\n";
const STRICT_DESC: &str = "A structurally read-only fixture.";
const LOOSE_DESC: &str = "An isolated read-only fixture.";

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "playbook-agents-check-{tag}-{}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir should be creatable");
    dir
}

/// The standard fixture shape: frontmatter, one blank body line, then `tail`
/// (usually a guardrails section).
fn agent(name: &str, desc: &str, tools: &str, model: &str, effort: &str, tail: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {desc}\ntools: {tools}\nmodel: {model}\neffort: {effort}\n---\n\nbody.\n{tail}"
    )
}

fn run(agents_dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["agents", "check"])
        .arg(agents_dir)
        .output()
        .expect("playbook binary should spawn")
}

/// Runs `playbook agents check` with NO directory argument, from `cwd`. Sets
/// the CHILD process's working directory only, never the test harness's own
/// (`set_current_dir` is process-global and would corrupt parallel tests).
fn run_default_from(cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["agents", "check"])
        .current_dir(cwd)
        .output()
        .expect("playbook binary should spawn")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// One shell-suite scenario: a tag for the scratch dir, the `sample.md`
/// content, whether the CLI should exit 0, and (for a failure) a substring
/// its stderr must contain.
type Case = (&'static str, String, bool, &'static str);

fn pass(tag: &'static str, content: String) -> Case {
    (tag, content, true, "")
}

fn fail(tag: &'static str, content: String, needle: &'static str) -> Case {
    (tag, content, false, needle)
}

/// Scenarios 1 to 8 and 10 to 24 of `shell/check-agents.test.sh`; scenario 9
/// (`_TEMPLATE.md` skipped) needs a second file in the fixture dir so it is
/// its own test below. One case per line on purpose (`#[rustfmt::skip]`): a
/// data table reads better dense than wrapped across rustfmt's default
/// width, and each row already carries its own tag and expectation.
#[rustfmt::skip]
fn scenarios() -> Vec<Case> {
    let unquoted_desc = "A fixture. Each spawn takes a focus from the orchestrator's prompt: a lens.";
    vec![
        pass("valid", agent("sample", STRICT_DESC, "Read, Grep, Glob", "sonnet", "medium", GUARDRAILS_FULL)),
        pass("write-capable", agent("sample", "A write-capable fixture, holds Edit, Write, and Bash on purpose.", "Read, Edit, Write, Bash", "sonnet", "high", GUARDRAILS_FULL)),
        fail("no-model", format!("---\nname: sample\ndescription: x\ntools: Read\neffort: medium\n---\n\nbody.\n{GUARDRAILS_FULL}"), "missing required frontmatter key 'model'"),
        fail("strict-write", agent("sample", STRICT_DESC, "Read, Write, Glob", "sonnet", "medium", GUARDRAILS_FULL), "or Bash, found: Write"),
        fail("strict-bash", agent("sample", STRICT_DESC, "Bash, Read, Grep", "sonnet", "medium", GUARDRAILS_FULL), "or Bash, found: Bash"),
        fail("bad-model", agent("sample", STRICT_DESC, "Read, Grep, Glob", "gpt", "medium", GUARDRAILS_FULL), "'gpt' is not one of"),
        fail("bad-effort", agent("sample", STRICT_DESC, "Read, Grep, Glob", "sonnet", "extreme", GUARDRAILS_FULL), "'extreme' is not one of"),
        fail("no-dash-missing", agent("sample", STRICT_DESC, "Read, Grep, Glob", "sonnet", "medium", "\n## Non-negotiable guardrails\n\n1. Stay in scope.\n"), "missing no-dash guardrail clause"),
        pass("loose-bash-ok", agent("sample", LOOSE_DESC, "Bash, Read, Grep", "sonnet", "medium", GUARDRAILS_FULL)),
        fail("loose-bash-write", agent("sample", LOOSE_DESC, "Bash, Write, Read, Grep", "sonnet", "medium", GUARDRAILS_FULL), "or NotebookEdit, found: Write"),
        fail("no-open-delim", format!("no opening delimiter at all.\n{GUARDRAILS_FULL}"), "missing opening --- frontmatter delimiter"),
        fail("no-close-delim", format!("---\nname: sample\ndescription: x\ntools: Read\nmodel: sonnet\neffort: medium\nno closing delimiter{GUARDRAILS_FULL}"), "missing closing --- frontmatter delimiter"),
        fail("name-mismatch", agent("not-sample", STRICT_DESC, "Read, Grep, Glob", "sonnet", "medium", GUARDRAILS_FULL), "does not match filename"),
        fail("no-name", format!("---\ndescription: x\ntools: Read\nmodel: sonnet\neffort: medium\n---\n\nbody.\n{GUARDRAILS_FULL}"), "missing required frontmatter key 'name'"),
        fail("no-description", format!("---\nname: sample\ntools: Read\nmodel: sonnet\neffort: medium\n---\n\nbody.\n{GUARDRAILS_FULL}"), "missing required frontmatter key 'description'"),
        fail("no-tools", format!("---\nname: sample\ndescription: x\nmodel: sonnet\neffort: medium\n---\n\nbody.\n{GUARDRAILS_FULL}"), "missing required frontmatter key 'tools'"),
        fail("no-guardrails-heading", agent("sample", STRICT_DESC, "Read, Grep, Glob", "sonnet", "medium", "\n## Guardrails\n\n1. No dashes, no em dash, no en dash.\n"), "missing '## Non-negotiable guardrails' heading"),
        fail("unknown-tool", agent("sample", STRICT_DESC, "Read, Grepp, Glob", "sonnet", "medium", GUARDRAILS_FULL), "Grepp"),
        fail("missing-grounding", agent("sample", STRICT_DESC, "Read, Grep, Glob", "sonnet", "medium", "\n## Non-negotiable guardrails\n\n1. No dashes, no em dash, no en dash.\n2. Zero AI attribution.\n"), "missing grounding guardrail clause"),
        fail("missing-attribution", agent("sample", STRICT_DESC, "Read, Grep, Glob", "sonnet", "medium", "\n## Non-negotiable guardrails\n\n1. No dashes, no em dash, no en dash.\n2. Ground every claim, quote exact code.\n"), "missing attribution guardrail clause"),
        fail("no-dash-outside", format!("---\nname: sample\ndescription: {STRICT_DESC}\ntools: Read, Grep, Glob\nmodel: sonnet\neffort: medium\n---\n\nIntro names em dash and en dash on purpose, outside the section below.\n\n## Non-negotiable guardrails\n\n1. Ground every claim, quote exact code.\n2. Zero AI attribution.\n"), "missing no-dash guardrail clause"),
        fail("unquoted-colon", agent("sample", unquoted_desc, "Read, Grep, Glob", "sonnet", "medium", GUARDRAILS_FULL), "colon-space"),
        pass("quoted-colon", agent("sample", &format!("\"{unquoted_desc}\""), "Read, Grep, Glob", "sonnet", "medium", GUARDRAILS_FULL)),
    ]
}

#[test]
fn check_agents_shell_parity_scenarios() {
    for (tag, content, expect_pass, needle) in scenarios() {
        // Arrange
        let dir = scratch(tag);
        fs::write(dir.join("sample.md"), &content).expect("fixture should be writable");

        // Act
        let out = run(&dir);

        // Assert
        if expect_pass {
            assert!(
                out.status.success(),
                "{tag}: expected exit 0, got {:?}: {}",
                out.status.code(),
                stderr_of(&out)
            );
        } else {
            assert_eq!(out.status.code(), Some(1), "{tag}");
            let err = stderr_of(&out);
            assert!(err.contains(needle), "{tag}: expected '{needle}' in: {err}");
        }
    }
}

// 1: the real repo agents dir passes.
#[test]
fn real_repo_agents_dir_passes() {
    let agents_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("agents");
    let out = run(&agents_dir);
    assert!(
        out.status.success(),
        "got {:?}: {}",
        out.status.code(),
        stderr_of(&out)
    );
    assert!(stdout_of(&out).contains("check-agents: OK"));
}

// 9: _TEMPLATE.md is skipped, even though it would fail every rule.
#[test]
fn template_md_is_skipped() {
    // Arrange
    let dir = scratch("template-skip");
    let content = agent(
        "sample",
        STRICT_DESC,
        "Read, Grep, Glob",
        "sonnet",
        "medium",
        GUARDRAILS_FULL,
    );
    fs::write(dir.join("sample.md"), content).expect("fixture");
    fs::write(
        dir.join("_TEMPLATE.md"),
        "Not a real agent. No frontmatter, no guardrails, nothing valid at all.\n",
    )
    .expect("template fixture");

    // Act
    let out = run(&dir);

    // Assert
    assert!(out.status.success(), "got: {}", stderr_of(&out));
    assert!(stdout_of(&out).contains("OK (1 agent definitions"));
}

// --- CLI wiring: the optional AGENTS_DIR argument's default resolution ---

#[test]
fn omitted_agents_dir_argument_defaults_to_repo_root_slash_agents() {
    // Arrange: a scratch git repo with an agents/ directory holding one
    // valid agent, run with NO agents_dir argument.
    let repo = scratch("default-resolution");
    let init = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["init", "-q"])
        .output()
        .expect("git init should spawn");
    assert!(init.status.success(), "git init failed");
    let agents_dir = repo.join("agents");
    fs::create_dir_all(&agents_dir).expect("agents dir should be creatable");
    let content = agent(
        "sample",
        STRICT_DESC,
        "Read, Grep, Glob",
        "sonnet",
        "medium",
        GUARDRAILS_FULL,
    );
    fs::write(agents_dir.join("sample.md"), content).expect("fixture");

    // Act
    let out = run_default_from(&repo);

    // Assert: the default resolved to <repo>/agents and validated it.
    assert!(
        out.status.success(),
        "expected the default to resolve to <repo>/agents, got {:?}: {}",
        out.status.code(),
        stderr_of(&out)
    );
}

#[test]
fn omitted_agents_dir_argument_outside_a_git_repo_errors() {
    // Arrange: a scratch directory that is not a git repository at all.
    let not_a_repo = scratch("not-a-repo");

    // Act
    let out = run_default_from(&not_a_repo);

    // Assert
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr_of(&out)
        .contains("check-agents: not inside a git repository and no AGENTS_DIR argument given"));
}

#[test]
fn nonexistent_agents_dir_argument_errors() {
    // Arrange
    let missing = scratch("missing-target").join("does-not-exist");

    // Act
    let out = run(&missing);

    // Assert
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_of(&out);
    assert!(err.contains("check-agents: agents directory not found"));
    assert!(err.contains(&missing.display().to_string()));
}
