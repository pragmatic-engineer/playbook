// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Behavioural tests for the `search-counter` and `post-edit-track` hooks,
//! ported from the retired shell test and
//! the retired shell test. Each test spawns the compiled
//! `playbook` binary exactly like the shell tests spawned `python3`: HOME
//! points at a scratch directory (never the real `$HOME`) and the hook
//! payload is piped on stdin, so the full CLI path (stdin read, JSON parse,
//! dispatch, hook body) is exercised end to end.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique per-call scratch directory under the OS temp dir, never the
/// real `$HOME`, so these tests cannot touch a developer's actual state.
fn scratch_dir(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "playbook-hooks-counter-test-{}-{tag}-{n}",
        playbook::testing::run_id()
    ))
}

/// Run `playbook hook <hook_name>` with HOME set to `home` and `payload`
/// fed on stdin, mirroring the shell tests' `HOME=$WORK python3 hook.py`.
/// Returns `(stdout, exit_code)`.
fn fire(hook_name: &str, home: &Path, payload: &str) -> (String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .env_remove("CI")
        .env_remove("PLAYBOOK_HEADLESS")
        .args(["hook", hook_name])
        .env("HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("playbook binary should spawn");
    child
        .stdin
        .take()
        .expect("stdin should be piped")
        .write_all(payload.as_bytes())
        .expect("payload should write to stdin");
    let output = child
        .wait_with_output()
        .expect("playbook binary should run to completion");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be valid utf8");
    (stdout, output.status.code().unwrap_or(-1))
}

/// Read one file out of a session's state directory under `home`. `None`
/// when the file (or the directory) does not exist.
fn read_state_file(home: &Path, session_id: &str, name: &str) -> Option<String> {
    std::fs::read_to_string(
        home.join(".config")
            .join("playbook")
            .join("runtime")
            .join(session_id)
            .join(name),
    )
    .ok()
}

mod search_counter {
    use super::*;

    #[test]
    fn grep_bumps_search_count_and_tool_count() {
        // Arrange
        let home = scratch_dir("counts");
        let sid = "sctest";
        let payload = format!(r#"{{"session_id":"{sid}","tool_name":"Grep"}}"#);

        // Act
        for _ in 0..4 {
            fire("search-counter", &home, &payload);
        }

        // Assert
        assert_eq!(
            read_state_file(&home, sid, "search-count").as_deref(),
            Some("4"),
            "search-count should reach 4 after 4 Grep calls"
        );
        assert_eq!(
            read_state_file(&home, sid, "tool-count").as_deref(),
            Some("4"),
            "tool-count should track every call"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_single_nudge_fires_at_eight_and_every_other_count_is_silent() {
        // Arrange: each case is a run of Grep calls and the substring the
        // nudge on the last call must contain, or `None` when the last call
        // must stay silent. Pins the one threshold and its neighbours,
        // plus the old 4 and 12 thresholds, which are retired.
        let cases: [(u32, Option<&str>); 6] = [
            (4, None),
            (7, None),
            (8, Some("count is 8")),
            (9, None),
            (12, None),
            (13, None),
        ];
        let mut failures = Vec::new();

        for (call_count, expected) in cases {
            let home = scratch_dir(&format!("threshold-{call_count}"));
            let sid = "sctest";
            let payload = format!(r#"{{"session_id":"{sid}","tool_name":"Grep"}}"#);

            // Act
            let mut last_stdout = String::new();
            for _ in 0..call_count {
                let (stdout, _) = fire("search-counter", &home, &payload);
                last_stdout = stdout;
            }

            // Assert (collected, so one failing case does not hide the rest)
            match expected {
                Some(substring) if !last_stdout.contains(substring) => failures.push(format!(
                    "call {call_count}: expected stdout to contain '{substring}', got '{last_stdout}'"
                )),
                None if !last_stdout.trim().is_empty() => failures.push(format!(
                    "call {call_count}: expected silence, got '{last_stdout}'"
                )),
                _ => {}
            }

            let _ = std::fs::remove_dir_all(&home);
        }

        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// The `additionalContext` of a hook's stdout, asserting stdout is exactly
    /// one PreToolUse JSON object.
    fn additional_context(stdout: &str) -> String {
        assert!(
            !stdout.is_empty(),
            "expected one PreToolUse context, got silence"
        );
        let parsed: serde_json::Value =
            serde_json::from_str(stdout).expect("stdout should be exactly one JSON object");
        assert_eq!(
            parsed["hookSpecificOutput"]["hookEventName"], "PreToolUse",
            "the hint is a PreToolUse context: {stdout}"
        );
        parsed["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additionalContext should be a string")
            .to_owned()
    }

    fn tool_payload(sid: &str, tool: &str, extra: &str) -> String {
        format!(r#"{{"session_id":"{sid}","tool_name":"{tool}"{extra}}}"#)
    }

    fn read_payload(sid: &str, path: &str) -> String {
        tool_payload(
            sid,
            "Read",
            &format!(r#","tool_input":{{"file_path":"{path}"}}"#),
        )
    }

    const HINT_SENTENCE: &str = "Put the same rule in any `Explore` prompt";

    #[test]
    fn first_grep_prints_the_code_hint_once_and_the_second_stays_silent() {
        // Arrange
        let home = scratch_dir("hint-grep");
        let sid = "hint";
        let payload = tool_payload(sid, "Grep", "");

        // Act
        let (first, _) = fire("search-counter", &home, &payload);
        let (second, _) = fire("search-counter", &home, &payload);

        // Assert
        let context = additional_context(&first);
        for needle in ["playbook code status", "playbook code index", HINT_SENTENCE] {
            assert!(
                context.contains(needle),
                "hint should hold '{needle}': {context}"
            );
        }
        assert!(
            read_state_file(&home, sid, "code-hint").is_some(),
            "the first hint should create the code-hint marker"
        );
        assert_eq!(second, "", "the second Grep should stay silent");

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn first_glob_prints_the_hint_and_a_later_grep_stays_silent() {
        // Arrange
        let home = scratch_dir("hint-glob");
        let sid = "hint";

        // Act
        let (glob, _) = fire("search-counter", &home, &tool_payload(sid, "Glob", ""));
        let (grep, _) = fire("search-counter", &home, &tool_payload(sid, "Grep", ""));

        // Assert
        assert!(
            additional_context(&glob).contains("playbook code status"),
            "the first Glob should print the hint: {glob}"
        );
        assert_eq!(grep, "", "a Grep after the hint should stay silent");

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_read_does_not_use_up_the_hint() {
        // Arrange
        let home = scratch_dir("hint-read");
        let sid = "hint";

        // Act
        let (read, _) = fire("search-counter", &home, &read_payload(sid, "/etc/hosts"));
        let marker_after_read = read_state_file(&home, sid, "code-hint");
        let (grep, _) = fire("search-counter", &home, &tool_payload(sid, "Grep", ""));

        // Assert
        assert_eq!(read, "", "a Read should print nothing");
        assert!(
            marker_after_read.is_none(),
            "a Read should not create code-hint"
        );
        assert!(
            additional_context(&grep).contains("playbook code status"),
            "the next Grep should print the hint: {grep}"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_subagent_grep_neither_gets_the_hint_nor_uses_it_up() {
        // Arrange
        let home = scratch_dir("hint-subagent");
        let sid = "hint";
        let subagent = tool_payload(sid, "Grep", r#","agent_id":"a1","agent_type":"Explore""#);

        // Act
        let (from_subagent, _) = fire("search-counter", &home, &subagent);
        let marker_after_subagent = read_state_file(&home, sid, "code-hint");
        let (from_main, _) = fire("search-counter", &home, &tool_payload(sid, "Grep", ""));

        // Assert
        assert_eq!(from_subagent, "", "a subagent Grep should print nothing");
        assert!(
            marker_after_subagent.is_none(),
            "a subagent Grep should not create code-hint"
        );
        assert!(
            additional_context(&from_main).contains("playbook code status"),
            "the main conversation should still get its hint: {from_main}"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_nudge_at_eight_wins_over_the_hint_and_names_the_rule() {
        // Arrange: seven Reads of distinct files, so the eighth counted call
        // is the session's first Grep.
        let home = scratch_dir("hint-nudge");
        let sid = "hint";
        for n in 0..7 {
            fire(
                "search-counter",
                &home,
                &read_payload(sid, &format!("/etc/f{n}")),
            );
        }

        // Act
        let (eighth, _) = fire("search-counter", &home, &tool_payload(sid, "Grep", ""));
        let marker_after_eighth = read_state_file(&home, sid, "code-hint");
        let (ninth, _) = fire("search-counter", &home, &tool_payload(sid, "Grep", ""));

        // Assert
        let nudge = additional_context(&eighth);
        assert!(nudge.contains("count is 8"), "nudge should fire: {nudge}");
        assert!(
            nudge.contains("playbook code status"),
            "nudge should name the rule: {nudge}"
        );
        assert!(
            !nudge.contains(HINT_SENTENCE),
            "nudge should not carry the hint's own sentence: {nudge}"
        );
        assert!(
            marker_after_eighth.is_none(),
            "the nudge call should not create code-hint"
        );
        assert!(
            additional_context(&ninth).contains(HINT_SENTENCE),
            "the ninth call should print the hint: {ninth}"
        );
        assert!(read_state_file(&home, sid, "code-hint").is_some());

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn read_of_same_path_counts_once() {
        // Arrange: the seen-reads file is what makes a repeated Read of the
        // same absolute path count only the first time.
        let home = scratch_dir("read-dedup");
        let sid = "dd";
        let payload = format!(
            r#"{{"session_id":"{sid}","tool_name":"Read","tool_input":{{"file_path":"/etc/hosts"}}}}"#
        );

        // Act
        fire("search-counter", &home, &payload);
        fire("search-counter", &home, &payload);

        // Assert
        assert_eq!(
            read_state_file(&home, sid, "search-count").as_deref(),
            Some("1"),
            "reading the same path twice should bump search-count once"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn no_session_id_is_a_silent_noop() {
        // Arrange
        let home = scratch_dir("no-session");
        let payload = r#"{"tool_name":"Grep"}"#;

        // Act
        let (stdout, code) = fire("search-counter", &home, payload);

        // Assert
        assert_eq!(code, 0, "hook should exit 0 with no session id");
        assert_eq!(stdout, "", "hook should print nothing with no session id");

        let _ = std::fs::remove_dir_all(&home);
    }
}

mod post_edit_track {
    use super::*;

    #[test]
    fn edit_write_notebookedit_accumulate_in_session() {
        // Arrange
        let home = scratch_dir("pet-sequence");
        let sid = "pet";

        // Act: Edit records a {path,ts} line and bumps edit-count.
        let edit_payload = format!(
            r#"{{"session_id":"{sid}","tool_name":"Edit","tool_input":{{"file_path":"/tmp/a/x.txt"}}}}"#
        );
        let (stdout, _) = fire("post-edit-track", &home, &edit_payload);

        // Assert: exact on-disk line shape, key order path-then-ts, compact
        // separators, matching python's
        // json.dumps({"path":...,"ts":...}, separators=(",", ":")).
        assert_eq!(stdout, "", "post-edit-track must never print to stdout");
        let jsonl = read_state_file(&home, sid, "edits.jsonl").expect("edits.jsonl should exist");
        let first_line = jsonl
            .lines()
            .next()
            .expect("edits.jsonl should have a line");
        let parsed: serde_json::Value =
            serde_json::from_str(first_line).expect("line should be valid JSON");
        let recorded_path = parsed["path"].as_str().expect("path should be a string");
        assert!(recorded_path.ends_with("/x.txt"));
        let recorded_ts = parsed["ts"].as_i64().expect("ts should be an integer");
        let expected_line = format!(
            "{{\"path\":{},\"ts\":{recorded_ts}}}",
            serde_json::to_string(recorded_path).unwrap()
        );
        assert_eq!(first_line, expected_line);
        assert_eq!(
            read_state_file(&home, sid, "edit-count").as_deref(),
            Some("1")
        );

        // Act: Write also records (append), edit-count now 2.
        let write_payload = format!(
            r#"{{"session_id":"{sid}","tool_name":"Write","tool_input":{{"file_path":"/tmp/a/y.txt"}}}}"#
        );
        fire("post-edit-track", &home, &write_payload);

        // Assert
        let jsonl = read_state_file(&home, sid, "edits.jsonl").unwrap();
        assert_eq!(
            jsonl.lines().count(),
            2,
            "Write should append a second line"
        );
        assert_eq!(
            read_state_file(&home, sid, "edit-count").as_deref(),
            Some("2")
        );

        // Act: NotebookEdit uses the notebook_path fallback.
        let notebook_payload = format!(
            r#"{{"session_id":"{sid}","tool_name":"NotebookEdit","tool_input":{{"notebook_path":"/tmp/a/nb.ipynb"}}}}"#
        );
        fire("post-edit-track", &home, &notebook_payload);

        // Assert
        let jsonl = read_state_file(&home, sid, "edits.jsonl").unwrap();
        let last_line = jsonl.lines().last().unwrap();
        assert!(
            last_line.contains("nb.ipynb"),
            "NotebookEdit should honour the notebook_path fallback"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn non_edit_tool_is_a_noop() {
        // Arrange
        let home = scratch_dir("pet-noop");
        let sid = "pet";
        let baseline_payload = format!(
            r#"{{"session_id":"{sid}","tool_name":"Edit","tool_input":{{"file_path":"/tmp/a/x.txt"}}}}"#
        );
        fire("post-edit-track", &home, &baseline_payload);
        let before = read_state_file(&home, sid, "edit-count");

        // Act
        let read_payload = format!(
            r#"{{"session_id":"{sid}","tool_name":"Read","tool_input":{{"file_path":"/tmp/a/x.txt"}}}}"#
        );
        fire("post-edit-track", &home, &read_payload);

        // Assert
        assert_eq!(
            read_state_file(&home, sid, "edit-count"),
            before,
            "Read should not bump edit-count"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn no_session_id_is_a_silent_noop() {
        // Arrange
        let home = scratch_dir("pet-no-session");
        let payload = r#"{"tool_name":"Edit","tool_input":{"file_path":"/tmp/z"}}"#;

        // Act
        let (stdout, code) = fire("post-edit-track", &home, payload);

        // Assert
        assert_eq!(code, 0, "hook should exit 0 with no session id");
        assert_eq!(stdout, "", "hook should print nothing with no session id");

        let _ = std::fs::remove_dir_all(&home);
    }
}
