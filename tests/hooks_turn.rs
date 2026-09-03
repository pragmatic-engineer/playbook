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
/// touches the real `~/.config/playbook/runtime`.
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

/// `$HOME/.config/playbook/runtime/<session_id>`, matching how the hooks
/// derive their per-session state directory.
fn session_dir_for(home: &Path, session_id: &str) -> PathBuf {
    home.join(".config")
        .join("playbook")
        .join("runtime")
        .join(session_id)
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
        home.join(".config")
            .join("playbook")
            .join("runtime")
            .join("compactions.log")
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

    #[test]
    fn append_and_trim_succeed_when_the_lock_is_already_held() {
        // Arrange: pre-create the lock directory to simulate another
        // session's append-and-trim already in progress. The lock is
        // advisory and fails open after its retry budget (a hook must never
        // hang on contention), so this run must still append its own line
        // and the log must still end up valid, not skipped or torn.
        let home = scratch_home("pcw-lock-held");
        let log = compactions_log(&home);
        fs::create_dir_all(log.parent().expect("log path should have a parent")).unwrap();
        fs::write(&log, "old line 1\n").unwrap();
        let lock_dir = PathBuf::from(format!("{}.lock", log.display()));
        fs::create_dir(&lock_dir).expect("pre-creating the lock dir should succeed");

        // Act
        let (_stdout, code) = run_hook("precompact-warn", &home, r#"{"trigger":"manual"}"#);

        // Assert
        assert_eq!(
            code, 0,
            "the hook must fail open, not hang or error, when the lock is already held"
        );
        let contents = fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "the append must still land: {contents}");
        assert!(
            lines[1].contains("trigger=manual"),
            "the freshly appended line must be intact, got: {}",
            lines[1]
        );
        assert!(
            lock_dir.exists(),
            "a writer that did not acquire the lock must not remove it"
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

    /// `$HOME/.config/playbook/memory`, matching how the hooks derive the shared
    /// memory root.
    fn memory_dir_for(home: &Path) -> PathBuf {
        home.join(".config").join("playbook").join("memory")
    }

    /// Write `older_path`, sleep briefly, write `newer_path`, then poll up
    /// to 500ms confirming the mtime ordering actually holds.
    fn write_with_older_then_newer_mtime(older_path: &Path, newer_path: &Path) {
        fs::write(older_path, "").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(newer_path, "{}").unwrap();

        let older_mtime = fs::metadata(older_path).unwrap().modified().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        loop {
            let newer_mtime = fs::metadata(newer_path).unwrap().modified().unwrap();
            if newer_mtime > older_mtime {
                return;
            }
            if std::time::Instant::now() >= deadline {
                panic!(
                    "filesystem mtime resolution could not distinguish {} from {} within 500ms",
                    older_path.display(),
                    newer_path.display()
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn marker_present_fires_once_and_clears_the_marker() {
        // Arrange: graph.json mtime newer than the marker's, so this hits
        // the write-detected release path, not the retain-and-reblock path.
        let home = scratch_home("mc-fires");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("capture-due");
        let mem_dir = memory_dir_for(&home);
        fs::create_dir_all(&mem_dir).unwrap();
        let graph = mem_dir.join("memory.graph.json");
        write_with_older_then_newer_mtime(&marker, &graph);

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert: a detected write releases silently and consumes the marker.
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
        assert!(
            !marker.exists(),
            "marker should be cleared once a write is detected"
        );
    }

    #[test]
    fn legacy_home_memory_root_is_migrated_before_checking_for_a_write() {
        // Arrange: memory still sitting at the pre-ADR-0012 ~/.claude/memory location.
        let home = scratch_home("mc-legacy-root");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("capture-due");
        let legacy_mem_dir = home.join(".claude").join("memory");
        fs::create_dir_all(&legacy_mem_dir).unwrap();
        let legacy_graph = legacy_mem_dir.join("memory.graph.json");
        write_with_older_then_newer_mtime(&marker, &legacy_graph);

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert: migration ran before the write check, so it sees the graph at the new location.
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
        assert!(
            !marker.exists(),
            "marker should be cleared once the migrated write is detected"
        );
        assert!(
            memory_dir_for(&home).join("memory.graph.json").is_file(),
            "the graph should now live at the new location"
        );
        assert!(
            !home.join(".claude").join("memory").exists(),
            "the legacy memory tree should be gone after a completed migration"
        );
    }

    #[test]
    fn second_call_with_marker_already_consumed_is_silent() {
        // Arrange: same write-detected precondition, so the first call
        // consumes the marker via the silent-release path.
        let home = scratch_home("mc-second-call");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("capture-due");
        let mem_dir = memory_dir_for(&home);
        fs::create_dir_all(&mem_dir).unwrap();
        let graph = mem_dir.join("memory.graph.json");
        write_with_older_then_newer_mtime(&marker, &graph);
        run_hook("memory-capture", &home, &payload());

        // Act: the first call already consumed the marker, so a second run
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

    #[test]
    fn write_detected_releases_marker_silently() {
        // Arrange: capture-attempts pre-seeded, simulating a prior
        // re-block, plus a graph write newer than the marker.
        let home = scratch_home("mc-write-detected");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("capture-due");
        let attempts = dir.join("capture-attempts");
        fs::write(&attempts, "1").unwrap();
        let mem_dir = memory_dir_for(&home);
        fs::create_dir_all(&mem_dir).unwrap();
        let graph = mem_dir.join("memory.graph.json");
        write_with_older_then_newer_mtime(&marker, &graph);

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert: no block, and the stale attempts state is cleared too.
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
        assert!(!marker.exists());
        assert!(!attempts.exists());
    }

    #[test]
    fn no_graph_file_present_retains_marker_and_reblocks() {
        // Arrange: no memory.graph.json at all.
        let home = scratch_home("mc-no-graph");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("capture-due");
        fs::write(&marker, "").unwrap();

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert
        assert_eq!(code, 0);
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        assert_eq!(value["decision"], "block");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(reason.contains("re-block 1 of 2"), "reason: {reason}");
        assert!(marker.exists(), "marker should be retained for a re-block");
        assert_eq!(
            fs::read_to_string(dir.join("capture-attempts"))
                .unwrap()
                .trim(),
            "1"
        );
    }

    #[test]
    fn graph_file_older_than_marker_retains_marker_and_reblocks() {
        // Arrange: a stale graph from before this crossing.
        let home = scratch_home("mc-stale-graph");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        let mem_dir = memory_dir_for(&home);
        fs::create_dir_all(&mem_dir).unwrap();
        let graph = mem_dir.join("memory.graph.json");
        let marker = dir.join("capture-due");
        write_with_older_then_newer_mtime(&graph, &marker);

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert
        assert_eq!(code, 0);
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        assert_eq!(value["decision"], "block");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(reason.contains("re-block 1 of 2"), "reason: {reason}");
        assert!(marker.exists());
        assert_eq!(
            fs::read_to_string(dir.join("capture-attempts"))
                .unwrap()
                .trim(),
            "1"
        );
    }

    #[test]
    fn no_write_at_cap_minus_one_still_reblocks() {
        // Arrange
        let home = scratch_home("mc-cap-minus-one");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        fs::write(dir.join("capture-attempts"), "1").unwrap();

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert
        assert_eq!(code, 0);
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        assert_eq!(value["decision"], "block");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(reason.contains("re-block 2 of 2"), "reason: {reason}");
        assert_eq!(
            fs::read_to_string(dir.join("capture-attempts"))
                .unwrap()
                .trim(),
            "2"
        );
    }

    #[test]
    fn no_write_at_cap_releases_without_blocking() {
        // Arrange
        let home = scratch_home("mc-at-cap");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        fs::write(dir.join("capture-attempts"), "2").unwrap();

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
        assert!(!dir.join("capture-due").exists());
        assert!(!dir.join("capture-attempts").exists());
    }

    /// Restores a directory's permissions on drop, so a panic between the
    /// chmod and the restore still leaves the fixture cleaned up.
    #[cfg(unix)]
    struct RestorePerms {
        path: PathBuf,
        mode: u32,
    }

    #[cfg(unix)]
    impl Drop for RestorePerms {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
        }
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_marker_mtime_fails_open() {
        use std::io::ErrorKind;
        use std::os::unix::fs::PermissionsExt;

        // Arrange: strip the session dir's own permissions so stat'ing the
        // marker fails with PermissionDenied, not NotFound.
        let home = scratch_home("mc-unreadable-marker");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("capture-due");
        fs::write(&marker, "").unwrap();

        let original_mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).unwrap();
        let _guard = RestorePerms {
            path: dir.clone(),
            mode: original_mode,
        };

        match fs::metadata(&marker) {
            Ok(_) => {
                eprintln!("skipping: chmod 000 was bypassed (root or CAP_DAC_OVERRIDE)");
                return;
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                panic!("fixture broke: expected PermissionDenied, got NotFound");
            }
            Err(_) => {}
        }

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert
        assert_eq!(code, 0);
        assert_eq!(stdout, "", "an unreadable marker must fail open, not block");
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_graph_mtime_fails_open() {
        use std::io::ErrorKind;
        use std::os::unix::fs::PermissionsExt;

        // Arrange: the marker reads fine; only the memory dir is
        // restricted, so stat'ing memory.graph.json fails.
        let home = scratch_home("mc-unreadable-graph");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        let mem_dir = memory_dir_for(&home);
        fs::create_dir_all(&mem_dir).unwrap();

        let original_mode = fs::metadata(&mem_dir).unwrap().permissions().mode() & 0o777;
        fs::set_permissions(&mem_dir, fs::Permissions::from_mode(0o000)).unwrap();
        let _guard = RestorePerms {
            path: mem_dir.clone(),
            mode: original_mode,
        };

        let graph = mem_dir.join("memory.graph.json");
        match fs::metadata(&graph) {
            Ok(_) => {
                eprintln!("skipping: chmod 000 was bypassed (root or CAP_DAC_OVERRIDE)");
                return;
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                panic!("fixture broke: expected PermissionDenied, got NotFound");
            }
            Err(_) => {}
        }

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert
        assert_eq!(code, 0);
        assert_eq!(
            stdout, "",
            "an unreadable memory.graph.json must fail open, not block"
        );
    }

    #[test]
    fn corrupt_attempts_file_fails_open() {
        // Arrange
        let home = scratch_home("mc-corrupt-attempts");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        fs::write(dir.join("capture-attempts"), "garbage").unwrap();

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert: fail open, not "treated as 0 and re-blocked".
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
    }

    // ── handoff nudge escalation ──────────────────────────────────────────

    /// Matches `src/cc/mod.rs::project_slug`, duplicated here since this is
    /// a black-box test of the compiled binary.
    fn project_slug(path: &str) -> String {
        path.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    }

    /// `Command::current_dir` alone does not set the child's `PWD`, and
    /// `logical_cwd()` prefers `PWD`, so this sets both explicitly.
    fn run_hook_at(cwd: &Path, home: &Path, stdin: &str) -> (String, i32) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(["hook", "memory-capture"])
            .current_dir(cwd)
            .env("HOME", home)
            .env("PWD", cwd)
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

    fn write_start_ts(dir: &Path, epoch_secs: u64) {
        fs::write(dir.join("start-ts"), epoch_secs.to_string()).unwrap();
    }

    /// Writes a handoff file the way `skills/session-handoff/SKILL.md` does:
    /// `<slug>-<suffix>.md` under `~/.config/playbook/runtime/handoff`.
    fn write_handoff(home: &Path, cwd: &Path) -> PathBuf {
        let slug = project_slug(&cwd.to_string_lossy());
        let dir = home
            .join(".config")
            .join("playbook")
            .join("runtime")
            .join("handoff");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{slug}-test.md"));
        fs::write(&path, "HANDOFF").unwrap();
        path
    }

    /// The marker string unique to the stronger escalation sentence, absent
    /// from every other block reason this hook builds.
    const STRONG_NUDGE_MARKER: &str = "no handoff saved yet";

    #[test]
    fn handoff_nudge_appears_when_crossings_at_threshold_and_no_handoff_written() {
        // Arrange: no memory.graph.json (the no-write precondition),
        // crossings at the escalation threshold, no handoff file at all.
        let home = scratch_home("mc-nudge-at-threshold");
        let cwd = scratch_home("mc-nudge-at-threshold-cwd");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        fs::write(dir.join("capture-crossings"), "3").unwrap();
        write_start_ts(&dir, 1_000_000_000);

        // Act
        let (stdout, _code) = run_hook_at(&cwd, &home, &payload());

        // Assert
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(
            reason.contains(STRONG_NUDGE_MARKER),
            "reason should include the stronger handoff nudge: {reason}"
        );
    }

    #[test]
    fn handoff_nudge_appears_when_crossings_above_threshold() {
        // Arrange: same preconditions, crossings past the threshold, guards
        // a `== 3` implementation instead of `>= 3`.
        let home = scratch_home("mc-nudge-above-threshold");
        let cwd = scratch_home("mc-nudge-above-threshold-cwd");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        fs::write(dir.join("capture-crossings"), "5").unwrap();
        write_start_ts(&dir, 1_000_000_000);

        // Act
        let (stdout, _code) = run_hook_at(&cwd, &home, &payload());

        // Assert
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(
            reason.contains(STRONG_NUDGE_MARKER),
            "reason should include the stronger handoff nudge: {reason}"
        );
    }

    #[test]
    fn handoff_nudge_absent_below_crossing_threshold() {
        // Arrange: same preconditions, crossings below the threshold.
        let home = scratch_home("mc-nudge-below-threshold");
        let cwd = scratch_home("mc-nudge-below-threshold-cwd");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        fs::write(dir.join("capture-crossings"), "2").unwrap();
        write_start_ts(&dir, 1_000_000_000);

        // Act
        let (stdout, _code) = run_hook_at(&cwd, &home, &payload());

        // Assert: the baseline reblock reason, unchanged.
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(reason.contains("re-block 1 of 2"), "reason: {reason}");
        assert!(
            !reason.contains(STRONG_NUDGE_MARKER),
            "reason should not include the stronger handoff nudge below threshold: {reason}"
        );
    }

    #[test]
    fn handoff_nudge_absent_when_handoff_already_written() {
        // Arrange: start-ts far in the past, so the handoff file (written
        // with a real, current mtime) is newer than it.
        let home = scratch_home("mc-nudge-handoff-written");
        let cwd = scratch_home("mc-nudge-handoff-written-cwd");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        fs::write(dir.join("capture-crossings"), "3").unwrap();
        write_start_ts(&dir, 1);
        write_handoff(&home, &cwd);

        // Act
        let (stdout, _code) = run_hook_at(&cwd, &home, &payload());

        // Assert
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(
            !reason.contains(STRONG_NUDGE_MARKER),
            "reason should not nudge once a fresh handoff exists: {reason}"
        );
    }

    #[test]
    fn handoff_nudge_appears_when_handoff_is_stale() {
        // Arrange: start-ts far in the future, so a handoff written with a
        // real, current mtime reads as older than it, i.e. stale.
        let home = scratch_home("mc-nudge-handoff-stale");
        let cwd = scratch_home("mc-nudge-handoff-stale-cwd");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        fs::write(dir.join("capture-crossings"), "3").unwrap();
        write_start_ts(&dir, 4_102_444_800);
        write_handoff(&home, &cwd);

        // Act
        let (stdout, _code) = run_hook_at(&cwd, &home, &payload());

        // Assert
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(
            reason.contains(STRONG_NUDGE_MARKER),
            "reason should nudge when the only handoff found is stale: {reason}"
        );
    }

    #[test]
    fn handoff_nudge_never_fires_without_a_reblock() {
        // Arrange: a detected write (the silent-release path), crossings
        // well above the threshold.
        let home = scratch_home("mc-nudge-no-reblock");
        let cwd = scratch_home("mc-nudge-no-reblock-cwd");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("capture-due");
        let mem_dir = memory_dir_for(&home);
        fs::create_dir_all(&mem_dir).unwrap();
        let graph = mem_dir.join("memory.graph.json");
        write_with_older_then_newer_mtime(&marker, &graph);
        fs::write(dir.join("capture-crossings"), "5").unwrap();
        write_start_ts(&dir, 1);

        // Act
        let (stdout, code) = run_hook_at(&cwd, &home, &payload());

        // Assert: still no block at all.
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
    }

    #[test]
    fn crossings_unparseable_treated_as_zero_no_nudge() {
        // Arrange: no-write preconditions met, capture-crossings holds
        // non-numeric content.
        let home = scratch_home("mc-nudge-crossings-unparseable");
        let cwd = scratch_home("mc-nudge-crossings-unparseable-cwd");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();
        fs::write(dir.join("capture-crossings"), "not-a-number").unwrap();
        write_start_ts(&dir, 1);

        // Act
        let (stdout, code) = run_hook_at(&cwd, &home, &payload());

        // Assert: reblock still happens, but without the handoff sentence,
        // per the fail-safe rule.
        assert_eq!(code, 0);
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(reason.contains("re-block 1 of 2"), "reason: {reason}");
        assert!(
            !reason.contains(STRONG_NUDGE_MARKER),
            "unparseable crossings should default to 0, not nudge: {reason}"
        );
    }

    // ── consolidation nudge ──────────────────────────────────────────────

    /// Matches `memory_capture.rs::CONSOLIDATION_NODE_THRESHOLD`.
    const CONSOLIDATION_NODE_THRESHOLD: usize = 10;

    /// Matches `memory_capture.rs::OVERSIZED_FACT_BYTES`.
    const OVERSIZED_FACT_BYTES: usize = 2000;

    /// The substring common to every consolidation-mention sentence.
    const CONSOLIDATION_MARKER: &str = "consolidation candidate";

    /// Writes `memory.graph.json` with one node per id (each pointing at
    /// `<id>.md`) plus the given edges, then writes each `<id>.md` file.
    fn write_graph(
        mem_dir: &Path,
        node_ids: &[&str],
        edges: &[(&str, &str, &str)],
        body_bytes: usize,
    ) {
        fs::create_dir_all(mem_dir).unwrap();
        let nodes: Vec<serde_json::Value> = node_ids
            .iter()
            .map(|id| serde_json::json!({"id": id, "file": format!("{id}.md")}))
            .collect();
        let edges: Vec<serde_json::Value> = edges
            .iter()
            .map(|(from, to, relation)| {
                serde_json::json!({"from": from, "to": to, "relation": relation})
            })
            .collect();
        let graph = serde_json::json!({"nodes": nodes, "edges": edges, "version": 1});
        fs::write(
            mem_dir.join("memory.graph.json"),
            serde_json::to_string(&graph).unwrap(),
        )
        .unwrap();
        for id in node_ids {
            fs::write(mem_dir.join(format!("{id}.md")), "a".repeat(body_bytes)).unwrap();
        }
    }

    /// Writes `memory.signals.json` with only a cursor set to `epoch_secs`.
    fn write_cursor(mem_dir: &Path, epoch_secs: u64) {
        fs::create_dir_all(mem_dir).unwrap();
        let signals = serde_json::json!({"version": 1, "cursor": {"last_run_at": epoch_secs.to_string()}, "nodes": {}});
        fs::write(
            mem_dir.join("memory.signals.json"),
            serde_json::to_string(&signals).unwrap(),
        )
        .unwrap();
    }

    /// Auto-generated node ids `fact-0..count`.
    fn node_ids(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("fact-{i}")).collect()
    }

    #[test]
    fn consolidation_mention_absent_below_node_threshold() {
        // Arrange: one node short of the threshold, with a supersedes edge
        // that would otherwise qualify, pinning the count gate itself.
        let home = scratch_home("mc-consolidation-below-threshold");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        let mem_dir = memory_dir_for(&home);
        let ids = node_ids(CONSOLIDATION_NODE_THRESHOLD - 1);
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        write_graph(
            &mem_dir,
            &id_refs,
            &[("fact-0", "fact-1", "supersedes")],
            10,
        );
        let marker = dir.join("capture-due");
        fs::write(&marker, "").unwrap();

        // Act
        let (stdout, _code) = run_hook("memory-capture", &home, &payload());

        // Assert: baseline reblock reason, unchanged.
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(reason.contains("re-block 1 of 2"), "reason: {reason}");
        assert!(
            !reason.contains(CONSOLIDATION_MARKER),
            "reason should not mention consolidation below the node threshold: {reason}"
        );
    }

    #[test]
    fn consolidation_mention_appears_at_threshold_for_touched_superseded_fact() {
        // Arrange: node count exactly at the threshold, a supersedes edge
        // whose target is touched (no cursor yet, so everything counts).
        let home = scratch_home("mc-consolidation-at-threshold");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        let mem_dir = memory_dir_for(&home);
        let ids = node_ids(CONSOLIDATION_NODE_THRESHOLD);
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        write_graph(
            &mem_dir,
            &id_refs,
            &[("fact-0", "fact-1", "supersedes")],
            10,
        );
        fs::write(dir.join("capture-due"), "").unwrap();

        // Act
        let (stdout, _code) = run_hook("memory-capture", &home, &payload());

        // Assert
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(
            reason.contains(CONSOLIDATION_MARKER),
            "reason should mention a consolidation candidate at the threshold: {reason}"
        );
    }

    #[test]
    fn consolidation_mention_absent_for_facts_untouched_since_cursor() {
        // Arrange: same qualifying store, cursor stamped far in the future.
        let home = scratch_home("mc-consolidation-untouched");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        let mem_dir = memory_dir_for(&home);
        let ids = node_ids(CONSOLIDATION_NODE_THRESHOLD);
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        write_graph(
            &mem_dir,
            &id_refs,
            &[("fact-0", "fact-1", "supersedes")],
            10,
        );
        write_cursor(&mem_dir, 4_102_444_800);
        fs::write(dir.join("capture-due"), "").unwrap();

        // Act
        let (stdout, _code) = run_hook("memory-capture", &home, &payload());

        // Assert
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(
            !reason.contains(CONSOLIDATION_MARKER),
            "reason should not mention a candidate untouched since the cursor: {reason}"
        );
    }

    #[test]
    fn consolidation_mention_appears_for_touched_oversized_fact() {
        // Arrange: no supersedes edge, one fact's body past the byte cap.
        let home = scratch_home("mc-consolidation-oversized");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        let mem_dir = memory_dir_for(&home);
        let ids = node_ids(CONSOLIDATION_NODE_THRESHOLD);
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        write_graph(&mem_dir, &id_refs, &[], 10);
        fs::write(
            mem_dir.join("fact-0.md"),
            "a".repeat(OVERSIZED_FACT_BYTES + 1),
        )
        .unwrap();
        fs::write(dir.join("capture-due"), "").unwrap();

        // Act
        let (stdout, _code) = run_hook("memory-capture", &home, &payload());

        // Assert
        let value: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");
        let reason = value["reason"].as_str().unwrap_or_default();
        assert!(
            reason.contains(CONSOLIDATION_MARKER),
            "reason should mention the oversized fact: {reason}"
        );
    }

    #[test]
    fn consolidation_scan_advances_the_cursor() {
        // Arrange: a qualifying store, no cursor written yet.
        let home = scratch_home("mc-consolidation-cursor-advance");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        let mem_dir = memory_dir_for(&home);
        let ids = node_ids(CONSOLIDATION_NODE_THRESHOLD);
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        write_graph(
            &mem_dir,
            &id_refs,
            &[("fact-0", "fact-1", "supersedes")],
            10,
        );
        fs::write(dir.join("capture-due"), "").unwrap();
        assert!(!mem_dir.join("memory.signals.json").exists());

        // Act
        run_hook("memory-capture", &home, &payload());

        // Assert: the scan wrote a fresh cursor back.
        let signals: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(mem_dir.join("memory.signals.json"))
                .expect("consolidation scan should have written memory.signals.json"),
        )
        .expect("memory.signals.json should be valid JSON");
        let last_run_at = signals["cursor"]["last_run_at"]
            .as_str()
            .expect("cursor.last_run_at should be a string");
        assert!(
            last_run_at.parse::<u64>().is_ok(),
            "cursor.last_run_at should parse as an epoch-seconds timestamp, got {last_run_at}"
        );
    }

    #[test]
    fn consolidation_scan_never_blocks_reblock_cap_from_releasing() {
        // Arrange: a qualifying, candidate-bearing store, attempts at cap.
        let home = scratch_home("mc-consolidation-cap-releases");
        let dir = session_dir_for(&home, SID);
        fs::create_dir_all(&dir).unwrap();
        let mem_dir = memory_dir_for(&home);
        let ids = node_ids(CONSOLIDATION_NODE_THRESHOLD);
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        write_graph(
            &mem_dir,
            &id_refs,
            &[("fact-0", "fact-1", "supersedes")],
            10,
        );
        fs::write(dir.join("capture-due"), "").unwrap();
        fs::write(dir.join("capture-attempts"), "2").unwrap();

        // Act
        let (stdout, code) = run_hook("memory-capture", &home, &payload());

        // Assert: released, no block, regardless of the qualifying store.
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
        assert!(!dir.join("capture-due").exists());
        assert!(!dir.join("capture-attempts").exists());
    }
}
