// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Every scenario in the retired shell test (24 scenarios), ported,
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
        playbook::testing::run_id()
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

/// One shell-suite scenario: a tag for the scratch dir, the `reviewer.md`
/// content, whether the CLI should exit 0, and (for a failure) a substring
/// its stderr must contain.
type Case = (&'static str, String, bool, &'static str);

fn pass(tag: &'static str, content: String) -> Case {
    (tag, content, true, "")
}

fn fail(tag: &'static str, content: String, needle: &'static str) -> Case {
    (tag, content, false, needle)
}

/// Scenarios 1 to 8 and 10 to 24 of the retired shell test; scenario 9
/// (`_TEMPLATE.md` skipped) needs a second file in the fixture dir so it is
/// its own test below. One case per line on purpose (`#[rustfmt::skip]`): a
/// data table reads better dense than wrapped across rustfmt's default
/// width, and each row already carries its own tag and expectation.
#[rustfmt::skip]
fn scenarios() -> Vec<Case> {
    let unquoted_desc = "A fixture. Each spawn takes a focus from the orchestrator's prompt: a lens.";
    vec![
        pass("valid", agent("reviewer", STRICT_DESC, "Read, Grep, Glob", "sonnet", "medium", GUARDRAILS_FULL)),
        pass("write-capable", agent("reviewer", "A write-capable fixture, holds Edit, Write, and Bash on purpose.", "Read, Edit, Write, Bash", "sonnet", "high", GUARDRAILS_FULL)),
        fail("no-model", format!("---\nname: reviewer\ndescription: x\ntools: Read\neffort: medium\n---\n\nbody.\n{GUARDRAILS_FULL}"), "missing required frontmatter key 'model'"),
        fail("strict-write", agent("reviewer", STRICT_DESC, "Read, Write, Glob", "sonnet", "medium", GUARDRAILS_FULL), "or Bash, found: Write"),
        fail("strict-bash", agent("reviewer", STRICT_DESC, "Bash, Read, Grep", "sonnet", "medium", GUARDRAILS_FULL), "or Bash, found: Bash"),
        fail("bad-model", agent("reviewer", STRICT_DESC, "Read, Grep, Glob", "gpt", "medium", GUARDRAILS_FULL), "'gpt' is not one of"),
        fail("bad-effort", agent("reviewer", STRICT_DESC, "Read, Grep, Glob", "sonnet", "extreme", GUARDRAILS_FULL), "'extreme' is not one of"),
        fail("no-dash-missing", agent("reviewer", STRICT_DESC, "Read, Grep, Glob", "sonnet", "medium", "\n## Non-negotiable guardrails\n\n1. Stay in scope.\n"), "missing no-dash guardrail clause"),
        pass("loose-bash-ok", agent("reviewer", LOOSE_DESC, "Bash, Read, Grep", "sonnet", "medium", GUARDRAILS_FULL)),
        fail("loose-bash-write", agent("reviewer", LOOSE_DESC, "Bash, Write, Read, Grep", "sonnet", "medium", GUARDRAILS_FULL), "or NotebookEdit, found: Write"),
        fail("no-open-delim", format!("no opening delimiter at all.\n{GUARDRAILS_FULL}"), "missing opening --- frontmatter delimiter"),
        fail("no-close-delim", format!("---\nname: reviewer\ndescription: x\ntools: Read\nmodel: sonnet\neffort: medium\nno closing delimiter{GUARDRAILS_FULL}"), "missing closing --- frontmatter delimiter"),
        fail("name-mismatch", agent("not-reviewer", STRICT_DESC, "Read, Grep, Glob", "sonnet", "medium", GUARDRAILS_FULL), "does not match filename"),
        fail("no-name", format!("---\ndescription: x\ntools: Read\nmodel: sonnet\neffort: medium\n---\n\nbody.\n{GUARDRAILS_FULL}"), "missing required frontmatter key 'name'"),
        fail("no-description", format!("---\nname: reviewer\ntools: Read\nmodel: sonnet\neffort: medium\n---\n\nbody.\n{GUARDRAILS_FULL}"), "missing required frontmatter key 'description'"),
        fail("no-tools", format!("---\nname: reviewer\ndescription: x\nmodel: sonnet\neffort: medium\n---\n\nbody.\n{GUARDRAILS_FULL}"), "missing required frontmatter key 'tools'"),
        fail("no-guardrails-heading", agent("reviewer", STRICT_DESC, "Read, Grep, Glob", "sonnet", "medium", "\n## Guardrails\n\n1. No dashes, no em dash, no en dash.\n"), "missing '## Non-negotiable guardrails' heading"),
        fail("unknown-tool", agent("reviewer", STRICT_DESC, "Read, Grepp, Glob", "sonnet", "medium", GUARDRAILS_FULL), "Grepp"),
        fail("missing-grounding", agent("reviewer", STRICT_DESC, "Read, Grep, Glob", "sonnet", "medium", "\n## Non-negotiable guardrails\n\n1. No dashes, no em dash, no en dash.\n2. Zero AI attribution.\n"), "missing grounding guardrail clause"),
        fail("missing-attribution", agent("reviewer", STRICT_DESC, "Read, Grep, Glob", "sonnet", "medium", "\n## Non-negotiable guardrails\n\n1. No dashes, no em dash, no en dash.\n2. Ground every claim, quote exact code.\n"), "missing attribution guardrail clause"),
        fail("no-dash-outside", format!("---\nname: reviewer\ndescription: {STRICT_DESC}\ntools: Read, Grep, Glob\nmodel: sonnet\neffort: medium\n---\n\nIntro names em dash and en dash on purpose, outside the section below.\n\n## Non-negotiable guardrails\n\n1. Ground every claim, quote exact code.\n2. Zero AI attribution.\n"), "missing no-dash guardrail clause"),
        fail("unquoted-colon", agent("reviewer", unquoted_desc, "Read, Grep, Glob", "sonnet", "medium", GUARDRAILS_FULL), "colon-space"),
        pass("quoted-colon", agent("reviewer", &format!("\"{unquoted_desc}\""), "Read, Grep, Glob", "sonnet", "medium", GUARDRAILS_FULL)),
    ]
}

#[test]
fn check_agents_shell_parity_scenarios() {
    for (tag, content, expect_pass, needle) in scenarios() {
        // Arrange
        let dir = scratch(tag);
        fs::write(dir.join("reviewer.md"), &content).expect("fixture should be writable");

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
        "reviewer",
        STRICT_DESC,
        "Read, Grep, Glob",
        "sonnet",
        "medium",
        GUARDRAILS_FULL,
    );
    fs::write(dir.join("reviewer.md"), content).expect("fixture");
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
        "reviewer",
        STRICT_DESC,
        "Read, Grep, Glob",
        "sonnet",
        "medium",
        GUARDRAILS_FULL,
    );
    fs::write(agents_dir.join("reviewer.md"), content).expect("fixture");

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

// --- Effort-tier variants: rendered per session, never committed ---

fn run_sub(sub: &str, agents_dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["agents", sub])
        .arg(agents_dir)
        .output()
        .expect("playbook binary should spawn")
}

/// A scratch dir holding just a valid `reviewer.md`.
fn reviewer_fixture(tag: &str) -> PathBuf {
    let dir = scratch(tag);
    let content = agent(
        "reviewer",
        STRICT_DESC,
        "Read, Grep, Glob",
        "sonnet",
        "high",
        GUARDRAILS_FULL,
    );
    fs::write(dir.join("reviewer.md"), content).expect("fixture");
    dir
}

#[test]
fn an_agent_with_no_variants_entry_fails_naming_the_file() {
    let dir = scratch("variant-no-entry");
    let content = agent(
        "newcomer",
        STRICT_DESC,
        "Read, Grep, Glob",
        "sonnet",
        "high",
        GUARDRAILS_FULL,
    );
    fs::write(dir.join("newcomer.md"), content).expect("fixture");

    let out = run_sub("check", &dir);

    assert_eq!(out.status.code(), Some(1));
    assert!(stderr_of(&out).contains("newcomer.md: no entry in VARIANTS"));
}

#[test]
fn a_committed_generated_variant_file_fails() {
    let dir = reviewer_fixture("variant-committed");
    let body = format!(
        "---\nname: reviewer-low\n---\n{}\n",
        playbook::agents::variants::MARKER
    );
    fs::write(dir.join("reviewer-low.md"), body).expect("stale variant");

    let out = run_sub("check", &dir);

    assert_eq!(out.status.code(), Some(1));
    assert!(stderr_of(&out).contains("reviewer-low.md: a generated variant file, delete it"));
}

#[test]
fn a_base_that_cannot_render_a_tier_fails() {
    let dir = reviewer_fixture("variant-unrenderable");
    let base = dir.join("reviewer.md");
    let body = fs::read_to_string(&base).expect("base");
    fs::write(&base, body.replace("effort: high\n", "")).expect("edit");

    let out = run_sub("check", &dir);

    assert_eq!(out.status.code(), Some(1));
    let err = stderr_of(&out);
    assert!(
        err.contains("reviewer.md: cannot render the 'low' variant"),
        "{err}"
    );
}

#[test]
fn the_variants_command_shows_what_a_session_would_hold() {
    let dir = reviewer_fixture("variant-show");
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["agents", "variants", "--json"])
        .arg(&dir)
        .output()
        .expect("spawn");
    assert!(out.status.success(), "{}", stderr_of(&out));
    let map: serde_json::Value = serde_json::from_str(&stdout_of(&out)).expect("json");
    for name in ["reviewer-low", "reviewer-medium", "reviewer-xhigh"] {
        assert_eq!(
            map[name]["tools"],
            serde_json::json!(["Read", "Grep", "Glob"]),
            "{name}"
        );
        assert_eq!(map[name]["model"], "sonnet");
    }
    assert_eq!(map["reviewer-low"]["effort"], "low");
    assert!(map.get("reviewer-max").is_none(), "max needs mode all");

    let capped = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["agents", "variants", "--ceiling", "medium", "--json"])
        .arg(&dir)
        .output()
        .expect("spawn");
    let map: serde_json::Value = serde_json::from_str(&stdout_of(&capped)).expect("json");
    assert!(map.get("reviewer-xhigh").is_none());
    assert!(map.get("reviewer-medium").is_some());

    let off = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["agents", "variants", "--mode", "off"])
        .arg(&dir)
        .output()
        .expect("spawn");
    assert!(stdout_of(&off).contains("no variants"));
}

#[test]
fn the_real_agents_dir_has_only_base_files_and_every_agent_has_every_tier() {
    use playbook::agents::variants::{render_definition, TIERS, VARIANTS};
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("agents");
    let mut files: Vec<String> = fs::read_dir(&dir)
        .expect("agents dir")
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".md"))
        .collect();
    files.sort();
    let table: Vec<String> = VARIANTS.iter().map(|(b, _)| format!("{b}.md")).collect();
    let mut sorted = table;
    sorted.sort();
    assert_eq!(
        files, sorted,
        "agents/ holds exactly the base agents in VARIANTS"
    );
    for (base, tiers) in VARIANTS {
        assert_eq!(tiers, &TIERS);
        let content = fs::read_to_string(dir.join(format!("{base}.md"))).expect("base");
        let rendered = TIERS
            .iter()
            .filter(|t| {
                render_definition(base, &content, t)
                    .expect("renders")
                    .is_some()
            })
            .count();
        assert_eq!(rendered, TIERS.len() - 1, "{base}: every tier but its own");
    }
}
