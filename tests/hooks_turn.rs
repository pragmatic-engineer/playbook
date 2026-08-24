// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for the turn-scoped hooks this Work Unit ports:
//! auto-model-detect, precompact-warn, and memory-capture. Each test spawns
//! the compiled `playbook` binary exactly as Claude Code would (JSON on
//! stdin, `HOME` pointed at a scratch directory), so a real process boundary
//! sits between the test and the hook, the same way hooks/auto-model-detect.
//! test.sh, hooks/precompact-warn.test.sh, and hooks/memory-capture.test.sh
//! drive the python originals. Every assertion in those three shell scripts
//! has a counterpart below.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh, empty directory to use as `$HOME` for one test, so no test ever
/// touches the real `~/.claude/runtime`.
fn scratch_home(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("hooks-turn-test-{}-{tag}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch home should be creatable");
    dir
}

/// Run `playbook hook <name>` with `stdin` on its stdin and `home` as
/// `HOME`, the same way Claude Code invokes a hook. Returns stdout with the
/// trailing newline stripped, and the exit code.
fn run_hook(name: &str, home: &Path, stdin: &str) -> (String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["hook", name])
        .env("HOME", home)
        .env_remove("HOOK_INPUT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("playbook binary should spawn");
    child
        .stdin
        .take()
        .expect("child stdin should be piped")
        .write_all(stdin.as_bytes())
        .expect("writing hook input should succeed");
    let output = child.wait_with_output().expect("child process should exit");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be valid utf8");
    (
        stdout.trim_end_matches('\n').to_string(),
        output.status.code().unwrap_or(-1),
    )
}

/// `$HOME/.claude/runtime/<session_id>`, matching how the hooks derive
/// their per-session state directory.
fn session_dir_for(home: &Path, session_id: &str) -> PathBuf {
    home.join(".claude").join("runtime").join(session_id)
}

// ---------------------------------------------------------------------
// auto-model-detect (hooks/auto-model-detect.test.sh)
// ---------------------------------------------------------------------

mod auto_model_detect {
    use super::*;

    #[test]
    fn design_intent_nudges_with_user_prompt_submit_object() {
        // Arrange
        let home = scratch_home("amd-nudge");
        let payload = r#"{"prompt":"Should we design a new schema and evaluate the tradeoffs between the two approaches?"}"#;

        // Act
        let (stdout, code) = run_hook("auto-model-detect", &home, payload);

        // Assert
        assert_eq!(code, 0);
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("nudge output should be valid JSON");
        assert_eq!(
            value["hookSpecificOutput"]["hookEventName"],
            "UserPromptSubmit"
        );
        assert!(!value["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap_or_default()
            .is_empty());
    }

    #[test]
    fn slash_command_stays_silent() {
        // Arrange
        let home = scratch_home("amd-slash");
        let payload = r#"{"prompt":"/implement do the whole thing now for me please and thanks"}"#;

        // Act
        let (stdout, code) = run_hook("auto-model-detect", &home, payload);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
    }

    #[test]
    fn short_prompt_stays_silent() {
        // Arrange
        let home = scratch_home("amd-short");
        let payload = r#"{"prompt":"design?"}"#;

        // Act
        let (stdout, code) = run_hook("auto-model-detect", &home, payload);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
    }

    #[test]
    fn plain_prose_stays_silent() {
        // Arrange
        let home = scratch_home("amd-prose");
        let payload = r#"{"prompt":"please rename this variable to totalCount across the whole file thanks"}"#;

        // Act
        let (stdout, code) = run_hook("auto-model-detect", &home, payload);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
    }

    #[test]
    fn empty_prompt_stays_silent_and_exits_zero() {
        // Arrange
        let home = scratch_home("amd-empty");
        let payload = r#"{"prompt":""}"#;

        // Act
        let (stdout, code) = run_hook("auto-model-detect", &home, payload);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
    }

    #[test]
    fn architecture_and_migration_keywords_trigger() {
        // Arrange
        let home = scratch_home("amd-arch");
        let payload =
            r#"{"prompt":"what is the best architecture for this migration and data model?"}"#;

        // Act
        let (stdout, code) = run_hook("auto-model-detect", &home, payload);

        // Assert
        assert_eq!(code, 0);
        assert!(!stdout.is_empty());
    }

    #[test]
    fn representative_sample_of_regex_branches_all_trigger() {
        // Arrange: table-driven, one prompt per otherwise-untested branch of
        // the design-intent alternation (nouns, verbs, and questions).
        let cases = [
            (
                "tradeoffs noun",
                "Let's talk through the tradeoffs of this plan before we start writing code today.",
            ),
            (
                "brainstorm verb",
                "Can we brainstorm a few options before committing to one direction here?",
            ),
            (
                "pros and cons question",
                "What are the pros and cons of switching to the new queue system?",
            ),
            (
                "data model noun",
                "I want to sketch out a data model for the new billing service first.",
            ),
            (
                "should we question",
                "Should we split this service in two before the next release ships?",
            ),
        ];

        for (label, prompt) in cases {
            let home = scratch_home("amd-sample");
            let payload = serde_json::json!({ "prompt": prompt }).to_string();

            // Act
            let (stdout, code) = run_hook("auto-model-detect", &home, &payload);

            // Assert
            assert_eq!(code, 0, "{label}: exit code");
            assert!(!stdout.is_empty(), "{label}: expected a nudge, got silence");
        }
    }

    #[test]
    fn word_boundary_near_miss_does_not_trigger() {
        // Arrange: "redesigning" contains "design" as a substring but not as
        // a whole word on either side, so the word-boundary matching this
        // hook mirrors from the python regex must not treat it as a hit.
        let home = scratch_home("amd-near-miss");
        let payload =
            r#"{"prompt":"We are redesigning the login form layout for the release next week."}"#;

        // Act
        let (stdout, code) = run_hook("auto-model-detect", &home, payload);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
    }

    #[test]
    fn keyword_glued_to_cjk_text_does_not_trigger() {
        // Arrange: "schema" sits directly against CJK characters with no
        // spaces on either side. CJK has no word separators, so a keyword
        // embedded in it has no ASCII word boundary either, but python's
        // Unicode-aware `\b` still treats the adjacent ideographs as word
        // characters and stays silent; this word-boundary scan must agree.
        let home = scratch_home("amd-cjk-glued");
        let payload = serde_json::json!({
            "prompt": "这个项目的核心schema结构非常复杂而且难以理解"
        })
        .to_string();

        // Act
        let (stdout, code) = run_hook("auto-model-detect", &home, &payload);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
    }

    #[test]
    fn keyword_glued_to_accented_text_does_not_trigger() {
        // Arrange: "schema" sits directly against an accented Latin word
        // ("café") with no space, so the character immediately before it is
        // a Unicode letter, not a word boundary.
        let home = scratch_home("amd-accented-glued");
        let payload = serde_json::json!({
            "prompt": "We reviewed the caféschema quickly during our lunch meeting today"
        })
        .to_string();

        // Act
        let (stdout, code) = run_hook("auto-model-detect", &home, &payload);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
    }
}

// ---------------------------------------------------------------------
// precompact-warn (hooks/precompact-warn.test.sh)
// ---------------------------------------------------------------------

mod precompact_warn {
    use super::*;

    fn compactions_log(home: &Path) -> PathBuf {
        home.join(".claude").join("runtime").join("compactions.log")
    }

    #[test]
    fn emits_a_valid_system_message_object() {
        // Arrange
        let home = scratch_home("pcw-valid");
        let payload = r#"{"trigger":"auto","session_id":"s1"}"#;

        // Act
        let (stdout, code) = run_hook("precompact-warn", &home, payload);

        // Assert
        assert_eq!(code, 0);
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        assert!(value.get("systemMessage").is_some());
    }

    #[test]
    fn interpolates_the_trigger() {
        // Arrange
        let home = scratch_home("pcw-interp");
        let payload = r#"{"trigger":"auto","session_id":"s1"}"#;

        // Act
        let (stdout, _code) = run_hook("precompact-warn", &home, payload);

        // Assert
        assert!(
            stdout.contains("(auto)"),
            "expected the trigger interpolated, got: {stdout}"
        );
    }

    #[test]
    fn missing_trigger_defaults_to_auto_in_message_and_unknown_in_log() {
        // Arrange
        let home = scratch_home("pcw-missing");
        let payload = r#"{"session_id":"s2"}"#;

        // Act
        let (stdout, _code) = run_hook("precompact-warn", &home, payload);

        // Assert
        assert!(
            stdout.contains("(auto)"),
            "expected message default 'auto', got: {stdout}"
        );
        let log = fs::read_to_string(compactions_log(&home)).expect("log file should exist");
        assert!(
            log.contains("trigger=unknown"),
            "expected log default 'unknown', got: {log}"
        );
    }

    #[test]
    fn exits_zero_on_empty_payload() {
        // Arrange
        let home = scratch_home("pcw-empty");

        // Act
        let (_stdout, code) = run_hook("precompact-warn", &home, "{}");

        // Assert
        assert_eq!(code, 0);
    }

    #[test]
    fn log_is_capped_at_500_lines() {
        // Arrange: 600 distinguishable, numbered lines, so the surviving
        // window can be checked for content, not just count. A mutation
        // that kept the oldest 500 lines instead of the newest 500 would
        // still pass a bare `line_count <= 500` check, so this also pins
        // which lines survive and in what order.
        let home = scratch_home("pcw-cap");
        let log = compactions_log(&home);
        fs::create_dir_all(log.parent().expect("log path should have a parent")).unwrap();
        let mut seeded = String::new();
        for i in 1..=600 {
            seeded.push_str(&format!("old line {i}\n"));
        }
        fs::write(&log, seeded).unwrap();

        // Act
        let (_stdout, code) = run_hook("precompact-warn", &home, r#"{"trigger":"manual"}"#);

        // Assert
        assert_eq!(code, 0);
        let contents = fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(
            lines.len(),
            500,
            "expected exactly 500 lines after capping, got {}",
            lines.len()
        );
        assert_eq!(
            lines[0], "old line 102",
            "the oldest surviving line should be the start of the newest window, not line 1"
        );
        assert_eq!(
            lines[498], "old line 600",
            "the newest seeded line should be the second to last, right before the freshly appended entry"
        );
        assert!(
            lines[499].contains("trigger=manual"),
            "the last line should be the entry this run just appended, got: {}",
            lines[499]
        );
    }
}

// ---------------------------------------------------------------------
// memory-capture (hooks/memory-capture.test.sh)
// ---------------------------------------------------------------------

mod memory_capture {
    use super::*;

    const SID: &str = "test-session-abc";

    fn payload() -> String {
        format!(r#"{{"session_id":"{SID}"}}"#)
    }

    #[test]
    fn marker_present_fires_once_and_clears_the_marker() {
        // Arrange
        let home = scratch_home("mc-fires");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert
        assert_eq!(code, 0);
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        assert_eq!(value["decision"], "block");
        assert!(!value["reason"].as_str().unwrap_or_default().is_empty());
        assert!(
            !dir.join("capture-due").exists(),
            "marker should be cleared after firing"
        );
    }

    #[test]
    fn second_call_with_marker_already_consumed_is_silent() {
        // Arrange
        let home = scratch_home("mc-second-call");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        run_hook("memory-capture", &home, &payload());

        // Act: the first call already deleted the marker, so a second run
        // in the same session must emit nothing.
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
    }

    #[test]
    fn no_marker_at_all_is_silent() {
        // Arrange
        let home = scratch_home("mc-no-marker");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
    }

    #[test]
    fn edited_paths_are_named_in_the_reason() {
        // Arrange
        let home = scratch_home("mc-paths");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        fs::write(
            dir.join("edits.jsonl"),
            "{\"path\":\"/repo/src/one.sh\",\"ts\":1}\n{\"path\":\"/repo/src/two.sh\",\"ts\":2}\n",
        )
        .unwrap();

        // Act
        let (stdout, _code) = run_hook("memory-capture", &home, &payload());

        // Assert: both paths are present, and the more recently edited
        // path (two.sh, ts=2) is listed before the earlier one (one.sh,
        // ts=1), pinning the documented "most recent first" order rather
        // than plain append order.
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(reason.contains("/repo/src/one.sh"), "reason: {reason}");
        assert!(reason.contains("/repo/src/two.sh"), "reason: {reason}");
        let two_pos = reason
            .find("/repo/src/two.sh")
            .expect("two.sh should be listed");
        let one_pos = reason
            .find("/repo/src/one.sh")
            .expect("one.sh should be listed");
        assert!(
            two_pos < one_pos,
            "the more recently edited path should be listed first, got: {reason}"
        );
    }

    #[test]
    fn path_list_with_exactly_five_paths_has_no_more_note() {
        // Arrange: exactly at the five-path cap, so no "more" note should
        // appear.
        let home = scratch_home("mc-cap-five");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        let mut edits = String::new();
        for i in 1..=5 {
            edits.push_str(&format!(
                "{{\"path\":\"/repo/src/file{i}.sh\",\"ts\":{i}}}\n"
            ));
        }
        fs::write(dir.join("edits.jsonl"), edits).unwrap();

        // Act
        let (stdout, _code) = run_hook("memory-capture", &home, &payload());

        // Assert
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        let listed = reason.lines().filter(|line| line.starts_with("- ")).count();
        assert_eq!(listed, 5, "expected exactly 5 listed paths, got {listed}");
        assert!(
            !reason.contains("more"),
            "expected no 'more' note at exactly 5 paths, got: {reason}"
        );
    }

    #[test]
    fn path_list_with_exactly_six_paths_notes_one_more() {
        // Arrange: one path past the cap, so the note should read exactly
        // "1 more", not a generic plural.
        let home = scratch_home("mc-cap-six");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        let mut edits = String::new();
        for i in 1..=6 {
            edits.push_str(&format!(
                "{{\"path\":\"/repo/src/file{i}.sh\",\"ts\":{i}}}\n"
            ));
        }
        fs::write(dir.join("edits.jsonl"), edits).unwrap();

        // Act
        let (stdout, _code) = run_hook("memory-capture", &home, &payload());

        // Assert
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        let listed = reason.lines().filter(|line| line.starts_with("- ")).count();
        assert_eq!(listed, 5, "expected at most 5 listed paths, got {listed}");
        assert!(
            reason.contains("1 more"),
            "expected a note about exactly 1 more path, got: {reason}"
        );
    }

    /// ADR 0008 WU-4: the same threshold trigger that already asks the model
    /// to write down durable facts now also nudges a session handoff, so a
    /// long session gets one persisted even if the user never runs the
    /// command manually.
    #[test]
    fn reason_also_instructs_a_session_handoff() {
        // Arrange
        let home = scratch_home("mc-handoff-nudge");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();

        // Act
        let (stdout, _code) = run_hook("memory-capture", &home, &payload());

        // Assert
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(
            reason.contains("/playbook:session-handoff"),
            "reason should instruct running session-handoff, not just fact capture: {reason}"
        );
    }

    #[test]
    fn path_list_is_capped_at_five_with_a_more_note() {
        // Arrange
        let home = scratch_home("mc-cap");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        let mut edits = String::new();
        for i in 1..=50 {
            edits.push_str(&format!(
                "{{\"path\":\"/repo/src/file{i}.sh\",\"ts\":{i}}}\n"
            ));
        }
        fs::write(dir.join("edits.jsonl"), edits).unwrap();

        // Act
        let (stdout, _code) = run_hook("memory-capture", &home, &payload());

        // Assert
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        let listed = reason.lines().filter(|line| line.starts_with("- ")).count();
        assert!(listed <= 5, "expected at most 5 listed paths, got {listed}");
        assert!(
            reason.contains("more"),
            "expected a note about additional paths, got: {reason}"
        );
    }

    #[test]
    fn malformed_non_object_line_does_not_discard_the_other_paths() {
        // Arrange: a valid record, then a bare non-object line (a lone JSON
        // number), then another valid record. Python's `rec.get("path")`
        // would raise on the middle line and abandon the whole scan; this
        // port must instead skip only that line and still report both
        // valid paths, deliberately diverging from python here.
        let home = scratch_home("mc-non-object-line");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        fs::write(
            dir.join("edits.jsonl"),
            "{\"path\":\"/repo/src/one.sh\",\"ts\":1}\n42\n{\"path\":\"/repo/src/two.sh\",\"ts\":2}\n",
        )
        .unwrap();

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert
        assert_eq!(code, 0);
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(reason.contains("/repo/src/one.sh"), "reason: {reason}");
        assert!(reason.contains("/repo/src/two.sh"), "reason: {reason}");
    }

    #[test]
    fn missing_edits_log_still_blocks_with_a_reason() {
        // Arrange
        let home = scratch_home("mc-no-edits");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert
        assert_eq!(code, 0);
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        assert_eq!(value["decision"], "block");
        assert!(!value["reason"].as_str().unwrap_or_default().is_empty());
    }
}
