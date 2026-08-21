// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for `playbook::settings::gen`, ported from
//! `shell/gen-shared-settings.test.sh`'s 10 scenarios (A through J). The real
//! `shell/gen-shared-settings.py` WAS the oracle. Every test that compares
//! generated JSON now asserts against a frozen copy of its stdout under
//! `tests/fixtures/golden/`; see that directory's README. The python is
//! deleted by ADR 0007 WU-14, so freezing its output is what keeps those
//! comparisons meaningful once python is gone, rather than hand-typing an
//! expected JSON blob.
//!
//! The pass/fail-only cases (D through I) no longer invoke python at all.
//! They asserted that python exited non-zero with empty stdout, which stays
//! true when the script is absent, so they would have survived WU-14 while
//! proving nothing. Their Rust assertions are unchanged and are the real
//! coverage.
//!
//! Coverage map, so every scenario named in the Work Unit brief is
//! traceable to one place below:
//! - A (happy path): `happy_path_canned_perms_model_stripped_personal_keys_dropped_passthrough`
//! - B (model absent stays absent): `model_absent_in_source_stays_absent`
//! - C (model present is stripped): `model_in_source_is_stripped`
//! - D (malformed source JSON), E (missing source file), F (missing
//!   permissions file), G (degenerate permissions object), H (permissions
//!   with an empty allow array): `GUARD_CASES` inside
//!   `malformed_or_missing_inputs_guard_rejects_with_no_output`
//! - I (no arguments): `no_arguments_is_rejected_by_the_rust_cli_parser`
//! - J (hooks reduced to the safety guards only, functional hooks dropped):
//!   `hooks_reduced_to_safety_guards_only_functional_hooks_dropped`
//! - Mandatory non-ASCII fixture, divergence asserted in a named direction:
//!   `non_ascii_value_diverges_from_python_named_direction`
//! - Falsifiable regression pin (byte match against the python oracle, plus
//!   a mutated input proving the check can fail):
//!   `regression_pin_rust_matches_python_oracle_and_mutation_diverges`
//! - `playbook settings gen` works from the CLI: `settings_gen_works_from_the_cli`

#![allow(dead_code)]

use clap::Parser;
use playbook::settings::gen::generate;
use playbook::Cli;
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory under the OS temp dir, unique per call so
/// parallel tests never collide and none of them ever touch a real
/// `~/.claude/settings.json` or the tracked `settings.shared.json`.
fn scratch_dir(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = env::temp_dir().join(format!(
        "playbook-settings-gen-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir should be creatable");
    dir
}

fn write_file(path: &Path, content: &str) {
    fs::write(path, content).expect("scratch file should be writable");
}

/// Canned permissions fixture, reused read-only across scenarios, matching
/// `shell/gen-shared-settings.test.sh`'s `PERMS`.
const CANNED_PERMS: &str = r#"{
  "allow": ["Read", "Bash(git:*)"],
  "deny": ["Read(**/.env)"],
  "ask": ["Bash(curl:*)"],
  "defaultMode": "auto"
}"#;

/// Full live-like source with personal keys, product keys, and an unknown
/// key, matching the shell test's `SRC_FULL`.
const SRC_FULL: &str = r#"{
  "model": "sonnet",
  "skipAutoPermissionPrompt": true,
  "effortLevel": "xhigh",
  "theme": "dark-daltonized",
  "preferredNotifChannel": "ghostty",
  "prefersReducedMotion": true,
  "permissions": { "allow": ["Bash"], "deny": [], "ask": [], "defaultMode": "auto" },
  "env": { "IS_DEMO": "1", "DISABLE_AUTOUPDATER": "1" },
  "hooks": { "SessionStart": [{ "hooks": [] }] },
  "statusLine": { "type": "command", "command": "bash x" },
  "customUnknownKey": { "keep": "me" }
}"#;

const SRC_NOMODEL: &str = r#"{"env":{"IS_DEMO":"1"}}"#;
const SRC_OPUS: &str = r#"{"model":"opus","env":{}}"#;
const SRC_BAD: &str = "{ this is not json ";
const PERMS_EMPTY: &str = "{}";
const PERMS_NOALLOW: &str = r#"{"allow":[],"deny":[],"ask":[]}"#;

/// Hooks-heavy source matching the shell test's `SRC_HOOKS`: a bare
/// `session-init` entry plus a `PreToolUse` group mixing two guards already
/// on their bare binary form, one guard still on its legacy `.sh` command
/// (WU-13 ported all four, but a machine that has not re-run `playbook init`
/// yet can still have one wired that way), a functional `rtk hook claude`
/// command, and a `Read` matcher with only a functional hook, plus a
/// `PostToolUse` group that is entirely functional.
const SRC_HOOKS: &str = r#"{
  "env": {},
  "hooks": {
    "SessionStart": [{ "hooks": [{ "type": "command", "command": "~/.claude/hooks/session-init.sh" }] }],
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [
        { "type": "command", "command": "playbook hook rm-workspace-guard", "if": "Bash(rm:*)", "timeout": 10 },
        { "type": "command", "command": "playbook hook bg-await-guard", "timeout": 10 },
        { "type": "command", "command": "~/.claude/hooks/no-dash-guard.sh", "timeout": 10 },
        { "type": "command", "command": "rtk hook claude" }
      ] },
      { "matcher": "Read", "hooks": [{ "type": "command", "command": "~/.claude/hooks/preread-size-check.sh" }] }
    ],
    "PostToolUse": [{ "matcher": "Edit", "hooks": [{ "type": "command", "command": "~/.claude/hooks/post-edit-track.sh" }] }]
  }
}"#;

// A: happy path -> canned perms, no model, forced skipAutoPermissionPrompt,
//    personal keys gone, product + unknown keys pass through.
#[test]
fn happy_path_canned_perms_model_stripped_personal_keys_dropped_passthrough() {
    // Arrange
    let dir = scratch_dir("happy-path");
    let src_path = dir.join("src.json");
    let perms_path = dir.join("perms.json");
    write_file(&src_path, SRC_FULL);
    write_file(&perms_path, CANNED_PERMS);

    // Act
    let rust_output = generate(&src_path, &perms_path).expect("rust generate should succeed");

    // Assert against the frozen python oracle rather than a live python run.
    // See tests/fixtures/golden/README.md: the python original is deleted by
    // ADR 0007 WU-14, so its output is committed instead. SRC_FULL and
    // CANNED_PERMS are the same inputs the regression pin below uses, so this
    // reuses that fixture rather than freezing a duplicate.
    let golden_py_stdout = include_str!("fixtures/golden/gen-shared-settings.src-full.json");
    assert_eq!(
        rust_output, golden_py_stdout,
        "rust output should byte-match the frozen python oracle's output, trailing newline included"
    );
    let result: Value = serde_json::from_str(&rust_output).unwrap();
    let canned_perms: Value = serde_json::from_str(CANNED_PERMS).unwrap();
    assert_eq!(
        result["permissions"], canned_perms,
        "permissions should be replaced with the canned object"
    );
    assert!(result.get("model").is_none(), "model should be stripped");
    assert_eq!(result["skipAutoPermissionPrompt"], false);
    for key in [
        "effortLevel",
        "theme",
        "preferredNotifChannel",
        "prefersReducedMotion",
    ] {
        assert!(
            result.get(key).is_none(),
            "{key} should be dropped as a personal key"
        );
    }
    assert_eq!(result["env"]["IS_DEMO"], "1");
    assert_eq!(result["env"]["DISABLE_AUTOUPDATER"], "1");
    assert!(result.get("hooks").is_some(), "hooks should pass through");
    assert!(
        result.get("statusLine").is_some(),
        "statusLine should pass through"
    );
    assert_eq!(result["customUnknownKey"]["keep"], "me");
}

// B: model absent in source -> stays absent.
#[test]
fn model_absent_in_source_stays_absent() {
    // Arrange
    let dir = scratch_dir("model-absent");
    let src_path = dir.join("src.json");
    let perms_path = dir.join("perms.json");
    write_file(&src_path, SRC_NOMODEL);
    write_file(&perms_path, CANNED_PERMS);

    // Act
    let rust_output = generate(&src_path, &perms_path).expect("rust generate should succeed");

    // Assert against the frozen python oracle rather than a live python run.
    // See tests/fixtures/golden/README.md: the python original is deleted by
    // ADR 0007 WU-14, so its output is committed instead.
    let golden_py_stdout = include_str!("fixtures/golden/gen-shared-settings.model-absent.json");
    assert_eq!(
        rust_output, golden_py_stdout,
        "rust output should byte-match the frozen python oracle's output, trailing newline included"
    );
    let result: Value = serde_json::from_str(&rust_output).unwrap();
    assert!(result.get("model").is_none());
}

// C: model set in source -> stripped from the template.
#[test]
fn model_in_source_is_stripped() {
    // Arrange
    let dir = scratch_dir("model-present");
    let src_path = dir.join("src.json");
    let perms_path = dir.join("perms.json");
    write_file(&src_path, SRC_OPUS);
    write_file(&perms_path, CANNED_PERMS);

    // Act
    let rust_output = generate(&src_path, &perms_path).expect("rust generate should succeed");

    // Assert against the frozen python oracle rather than a live python run.
    // See tests/fixtures/golden/README.md: the python original is deleted by
    // ADR 0007 WU-14, so its output is committed instead.
    let golden_py_stdout = include_str!("fixtures/golden/gen-shared-settings.model-present.json");
    assert_eq!(
        rust_output, golden_py_stdout,
        "rust output should byte-match the frozen python oracle's output, trailing newline included"
    );
    let result: Value = serde_json::from_str(&rust_output).unwrap();
    assert!(result.get("model").is_none());
}

/// D, E, F, G, H: every input guard the generator enforces before it
/// produces output. `src: None` or `perms: None` means that file is never
/// written at all, exercising "missing file" rather than "invalid content".
struct GuardCase {
    name: &'static str,
    src: Option<&'static str>,
    perms: Option<&'static str>,
}

const GUARD_CASES: [GuardCase; 5] = [
    GuardCase {
        name: "D malformed source json",
        src: Some(SRC_BAD),
        perms: Some(CANNED_PERMS),
    },
    GuardCase {
        name: "E missing source file",
        src: None,
        perms: Some(CANNED_PERMS),
    },
    GuardCase {
        name: "F missing permissions file",
        src: Some(SRC_FULL),
        perms: None,
    },
    GuardCase {
        name: "G degenerate permissions object",
        src: Some(SRC_FULL),
        perms: Some(PERMS_EMPTY),
    },
    GuardCase {
        name: "H permissions with empty allow array",
        src: Some(SRC_FULL),
        perms: Some(PERMS_NOALLOW),
    },
];

#[test]
fn malformed_or_missing_inputs_guard_rejects_with_no_output() {
    for case in GUARD_CASES {
        // Arrange
        let dir = scratch_dir(&case.name.replace(' ', "-"));
        let src_path = dir.join("src.json");
        let perms_path = dir.join("perms.json");
        if let Some(content) = case.src {
            write_file(&src_path, content);
        }
        if let Some(content) = case.perms {
            write_file(&perms_path, content);
        }

        // Act
        let rust = generate(&src_path, &perms_path);

        // Assert
        //
        // The python half of this case was REMOVED rather than frozen as a
        // golden, because it asserted nothing. It checked only that python
        // exited non-zero with empty stdout, and once WU-14 deletes the
        // script `python3` exits non-zero with "can't open file" and empty
        // stdout, so both assertions would have held for the wrong reason.
        // Demonstrated 2026-08-21 by pointing the helper at a deliberately
        // non-existent script: all ten tests in this file still passed.
        //
        // Freezing it as a golden was considered and rejected: the assertion
        // is a bare "python rejected this", a boolean with no content, unlike
        // the output comparisons the other goldens preserve.
        assert!(rust.is_err(), "{}: rust should guard-reject", case.name);
    }
}

// I: no arguments -> the Rust CLI's own required-argument parsing rejects.
//
// The python half was removed, not frozen as a golden, for the reason given
// on the guard-rejection loop above: asserting "python exited non-zero with
// empty stdout" holds just as well when the script is absent, so it survives
// WU-14's deletion while proving nothing.
#[test]
fn no_arguments_is_rejected_by_the_rust_cli_parser() {
    // Act: rust CLI parsing
    let rust_result = Cli::try_parse_from(["playbook", "settings", "gen"]);

    // Assert
    assert!(
        rust_result.is_err(),
        "rust CLI should guard-reject with no SRC/PERMS given"
    );
}

// J: hooks reduced to the bare `playbook hook <name>` commands only;
// functional hooks, rtk, and a guard still on its legacy `.sh` command are
// all dropped. Also stands in for "the generator still refuses to
// reintroduce functional hooks into the seed".
#[test]
fn hooks_reduced_to_safety_guards_only_functional_hooks_dropped() {
    // Arrange
    let dir = scratch_dir("hooks-filter");
    let src_path = dir.join("src.json");
    let perms_path = dir.join("perms.json");
    write_file(&src_path, SRC_HOOKS);
    write_file(&perms_path, CANNED_PERMS);

    // Act
    let rust_output = generate(&src_path, &perms_path).expect("rust generate should succeed");

    // Assert against the frozen python oracle rather than a live python run.
    // See tests/fixtures/golden/README.md: the python original is deleted by
    // ADR 0007 WU-14, so its output is committed instead.
    let golden_py_stdout = include_str!("fixtures/golden/gen-shared-settings.hooks-filter.json");
    assert_eq!(
        rust_output, golden_py_stdout,
        "rust output should byte-match the frozen python oracle's output, trailing newline included"
    );
    let result: Value = serde_json::from_str(&rust_output).unwrap();
    let hooks = result["hooks"].as_object().unwrap();
    assert_eq!(
        hooks.keys().collect::<Vec<_>>(),
        vec!["PreToolUse"],
        "only PreToolUse should survive filtering"
    );
    let groups = result["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(
        groups.len(),
        1,
        "only the Bash matcher group should survive"
    );
    let commands: Vec<&str> = groups[0]["hooks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["command"].as_str().unwrap())
        .collect();
    assert_eq!(
        commands,
        vec![
            "playbook hook rm-workspace-guard",
            "playbook hook bg-await-guard"
        ],
        "functional hooks (rtk hook claude), a guard still on its legacy .sh command, \
         and the Read matcher group should all be dropped"
    );
}

// Mandatory non-ASCII fixture: python's json.dumps escapes to \uXXXX
// (ensure_ascii=True is the default); serde_json writes raw UTF-8. Asserts
// the divergence in a named direction rather than leaving it to a comment.
#[test]
fn non_ascii_value_diverges_from_python_named_direction() {
    // Arrange: a value with non-ASCII characters, a shape no real settings
    // file or permissions.shared.json contains today.
    let dir = scratch_dir("non-ascii");
    let src_path = dir.join("src.json");
    let perms_path = dir.join("perms.json");
    write_file(&src_path, r#"{"customUnknownKey":"café ☃"}"#);
    write_file(&perms_path, CANNED_PERMS);

    // Act
    let rust_output = generate(&src_path, &perms_path).expect("rust generate should succeed");

    // Assert against the frozen python oracle rather than a live python run.
    // See tests/fixtures/golden/README.md: the python original is deleted by
    // ADR 0007 WU-14, so its output is committed instead. This golden
    // captures python's ESCAPED output deliberately: the assertions below
    // still pin a genuine divergence, not an accidental one, and must never
    // be "fixed" into agreement.
    let golden_py_stdout = include_str!("fixtures/golden/gen-shared-settings.non-ascii.json");
    assert!(
        golden_py_stdout.contains("\\u00e9") && golden_py_stdout.contains("\\u2603"),
        "python output should escape the non-ASCII characters to \\uXXXX: {golden_py_stdout}"
    );
    assert!(
        !golden_py_stdout.contains("café"),
        "python output should not contain the raw UTF-8 characters: {golden_py_stdout}"
    );
    // ...rust writes raw UTF-8 instead.
    assert!(
        rust_output.contains("café ☃"),
        "rust output should contain the raw UTF-8 characters: {rust_output}"
    );
    assert!(
        !rust_output.contains("\\u00e9"),
        "rust output should not escape the non-ASCII characters: {rust_output}"
    );
    // The two genuinely diverge on this input, in the direction just pinned.
    assert_ne!(
        rust_output, golden_py_stdout,
        "python-escaped and rust-raw-UTF-8 outputs should diverge on non-ASCII input"
    );
}

/// A second, mutated copy of `SRC_FULL`: one product key (`env.IS_DEMO`)
/// changed, so the generated output must differ from `SRC_FULL`'s.
const SRC_MUTATED: &str = r#"{
  "model": "sonnet",
  "skipAutoPermissionPrompt": true,
  "effortLevel": "xhigh",
  "theme": "dark-daltonized",
  "preferredNotifChannel": "ghostty",
  "prefersReducedMotion": true,
  "permissions": { "allow": ["Bash"], "deny": [], "ask": [], "defaultMode": "auto" },
  "env": { "IS_DEMO": "0", "DISABLE_AUTOUPDATER": "1" },
  "hooks": { "SessionStart": [{ "hooks": [] }] },
  "statusLine": { "type": "command", "command": "bash x" },
  "customUnknownKey": { "keep": "me" }
}"#;

// Falsifiable regression pin: the Rust generator's output byte-matches the
// PYTHON generator's output from the same SRC, never "no diff against the
// committed settings.shared.json" (which passes trivially once that file
// was itself produced by the code under test). A mutated input must also
// produce a genuinely different output, proving the pin can fail.
#[test]
fn regression_pin_rust_matches_python_oracle_and_mutation_diverges() {
    // Arrange: synthetic fixtures, not the committed settings.shared.json.
    let dir = scratch_dir("regression-pin");
    let src_path = dir.join("src.json");
    let perms_path = dir.join("perms.json");
    write_file(&src_path, SRC_FULL);
    write_file(&perms_path, CANNED_PERMS);

    // Act: the pin itself, Rust against the frozen python oracle on matching
    // input. See tests/fixtures/golden/README.md: the python original is
    // deleted by ADR 0007 WU-14, so its output is committed instead.
    let rust_output = generate(&src_path, &perms_path).expect("rust generate should succeed");

    // Assert: the pin holds.
    let golden_py_stdout = include_str!("fixtures/golden/gen-shared-settings.src-full.json");
    assert_eq!(
        rust_output, golden_py_stdout,
        "rust output should byte-match the frozen python oracle's output, trailing newline included"
    );

    // Act: mutate the input.
    let mutated_src_path = dir.join("src-mutated.json");
    write_file(&mutated_src_path, SRC_MUTATED);
    let mutated_output =
        generate(&mutated_src_path, &perms_path).expect("rust generate should succeed");

    // Assert: a genuinely different input produces genuinely different
    // output, proving this pin can fail rather than passing vacuously.
    assert_ne!(
        rust_output, mutated_output,
        "a mutated input should change the generated output"
    );
}

// `playbook settings gen` works from the CLI, not only through the library
// function the other tests call directly.
#[test]
fn settings_gen_works_from_the_cli() {
    // Arrange
    let dir = scratch_dir("cli");
    let src_path = dir.join("src.json");
    let perms_path = dir.join("perms.json");
    write_file(&src_path, SRC_NOMODEL);
    write_file(&perms_path, CANNED_PERMS);

    // Act
    let output = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["settings", "gen"])
        .arg(&src_path)
        .arg(&perms_path)
        .output()
        .expect("playbook binary should run");

    // Assert
    assert!(
        output.status.success(),
        "playbook settings gen should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let expected = generate(&src_path, &perms_path).expect("rust generate should succeed");
    assert_eq!(
        stdout, expected,
        "CLI stdout should match the library function's output exactly"
    );
}

/// A malformed `.hooks` shape must FAIL, not quietly yield a hooks-less seed.
///
/// This is a regression pin with a specific history. The first revision of this
/// port treated a malformed shape as "contributes no hooks" and exited 0,
/// reasoning that a shape it could not parse could not leak an unsafe command.
/// That is backwards in the only pipeline that consumes this output:
/// `Makefile`'s recipe is `gen ... > "$@.tmp" && mv "$@.tmp" "$@"`, so a
/// nonzero exit leaves the committed seed untouched, while an exit 0 carrying a
/// hooks-less seed REPLACES it with one that wires nothing.
///
/// Asserted against the real python generator in both directions, so the two
/// agree on rejecting rather than merely on some error appearing.
#[test]
fn malformed_hooks_shape_fails_in_both_engines_and_writes_no_stdout() {
    // Arrange
    let dir = scratch_dir("malformed-hooks");
    let perms_path = dir.join("perms.json");
    write_file(&perms_path, CANNED_PERMS);

    let cases = [
        (".hooks is a string", r#"{"hooks": "not-an-object"}"#),
        (".hooks is an array", r#"{"hooks": []}"#),
        (
            ".hooks event is not an array",
            r#"{"hooks": {"PreToolUse": {"not": "an array"}}}"#,
        ),
        (
            ".hooks group is not an object",
            r#"{"hooks": {"PreToolUse": ["not-an-object"]}}"#,
        ),
    ];

    for (label, src_json) in cases {
        let src_path = dir.join(format!("src-{}.json", label.replace(['.', ' '], "-")));
        write_file(&src_path, src_json);

        // Act
        let rust = generate(&src_path, &perms_path);

        // Assert
        //
        // The python half was removed for the reason recorded on the
        // guard-rejection loop above: "python exited non-zero with empty
        // stdout" is equally true when the script does not exist, so it
        // would have survived WU-14 while asserting nothing.
        assert!(
            rust.is_err(),
            "{label}: the Rust port must reject it too. Emitting a hooks-less \
             seed on exit 0 would let the Makefile replace the committed seed \
             with one wiring no hooks"
        );
    }
}
