// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Behavioural tests for the `preread-edit-check` and `preread-size-check`
//! PreToolUse hooks, Rust ports of hooks/preread-edit-check.py and
//! hooks/preread-size-check.py. Every assertion in
//! hooks/preread-edit-check.test.sh and hooks/preread-size-check.test.sh has
//! a counterpart here. Each test spawns the built `playbook` binary with a
//! JSON payload on stdin and a scratch `HOME`, the same way the bash suites
//! invoke the python scripts with `HOME="$WORK"`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh, unique scratch directory for one test, so parallel test threads
/// never collide on the same path.
fn scratch_dir(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "playbook-hooks-preread-{tag}-{}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir should be creatable");
    dir
}

/// Run `playbook hook <name>` with `stdin_json` piped in and `HOME` pointed
/// at `home`.
fn run_hook(name: &str, home: &Path, stdin_json: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["hook", name])
        .env("HOME", home)
        .env_remove("HOOK_INPUT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("playbook binary should spawn");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(stdin_json.as_bytes())
        .expect("writing stdin should succeed");
    child.wait_with_output().expect("playbook should exit")
}

fn stdout_string(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout should be valid UTF-8")
}

fn stderr_string(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr should be valid UTF-8")
}

mod preread_edit_check {
    use super::*;

    fn payload(session_id: &str, file_path: &str) -> String {
        format!(r#"{{"session_id":"{session_id}","tool_input":{{"file_path":"{file_path}"}}}}"#)
    }

    /// Write a single-record edits.jsonl for `session_id` under `home`,
    /// mirroring the bash suite's `seed()` helper.
    fn seed_edits(home: &Path, session_id: &str, path: &str, ts: i64) {
        let dir = home.join(".claude").join("runtime").join(session_id);
        fs::create_dir_all(&dir).expect("session dir should be creatable");
        fs::write(
            dir.join("edits.jsonl"),
            format!(r#"{{"path":"{path}","ts":{ts}}}"#),
        )
        .expect("edits.jsonl should be writable");
    }

    fn now() -> i64 {
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap()
    }

    /// Write raw `edits.jsonl` content for `session_id` under `home`, for
    /// scenarios that need more than one record or a non-integer `ts`,
    /// which `seed_edits` above cannot express.
    fn seed_edits_raw(home: &Path, session_id: &str, content: &str) {
        let dir = home.join(".claude").join("runtime").join(session_id);
        fs::create_dir_all(&dir).expect("session dir should be creatable");
        fs::write(dir.join("edits.jsonl"), content).expect("edits.jsonl should be writable");
    }

    #[test]
    fn recent_edit_nudges_with_age() {
        // Arrange: this exact file was edited 2 minutes ago.
        let home = scratch_dir("edit-recent");
        seed_edits(&home, "pec", "/tmp/x/file.py", now() - 120);

        // Act
        let output = run_hook(
            "preread-edit-check",
            &home,
            &payload("pec", "/tmp/x/file.py"),
        );

        // Assert
        let out = stdout_string(&output);
        assert!(
            out.contains("2m ago"),
            "expected a 2m ago nudge, got: {out}"
        );
    }

    #[test]
    fn nudge_emits_a_valid_pretooluse_additional_context_object() {
        // Arrange
        let home = scratch_dir("edit-shape");
        seed_edits(&home, "pec", "/tmp/x/file.py", now() - 120);

        // Act
        let output = run_hook(
            "preread-edit-check",
            &home,
            &payload("pec", "/tmp/x/file.py"),
        );

        // Assert
        let out = stdout_string(&output);
        let parsed: serde_json::Value =
            serde_json::from_str(out.trim()).expect("stdout should be valid JSON");
        assert_eq!(
            parsed["hookSpecificOutput"]["hookEventName"], "PreToolUse",
            "unexpected shape: {out}"
        );
        assert!(
            parsed["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
            "additionalContext should carry the nudge message: {out}"
        );
    }

    #[test]
    fn edit_older_than_the_window_stays_silent() {
        // Arrange: 31 minutes ago, one minute past the 30 minute window.
        let home = scratch_dir("edit-outside-window");
        seed_edits(&home, "pec", "/tmp/x/file.py", now() - 1860);

        // Act
        let output = run_hook(
            "preread-edit-check",
            &home,
            &payload("pec", "/tmp/x/file.py"),
        );

        // Assert
        assert_eq!(
            stdout_string(&output),
            "",
            "should stay silent outside the window"
        );
    }

    #[test]
    fn unrelated_path_stays_silent() {
        // Arrange: the only edit on record is a different file.
        let home = scratch_dir("edit-unrelated-path");
        seed_edits(&home, "pec", "/tmp/x/other.py", now() - 60);

        // Act
        let output = run_hook(
            "preread-edit-check",
            &home,
            &payload("pec", "/tmp/x/file.py"),
        );

        // Assert
        assert_eq!(
            stdout_string(&output),
            "",
            "should stay silent for an unrelated path"
        );
    }

    #[test]
    fn seconds_scale_age_renders_as_n_seconds_ago() {
        // Arrange
        let home = scratch_dir("edit-seconds-scale");
        seed_edits(&home, "pec", "/tmp/x/file.py", now() - 10);

        // Act
        let output = run_hook(
            "preread-edit-check",
            &home,
            &payload("pec", "/tmp/x/file.py"),
        );

        // Assert
        let out = stdout_string(&output);
        assert!(
            out.contains("s ago"),
            "expected a seconds-scale age, got: {out}"
        );
    }

    #[test]
    fn no_edits_file_is_a_silent_no_op() {
        // Arrange: the session directory exists (created by the hook itself
        // via session_dir), but no edits.jsonl was ever written.
        let home = scratch_dir("edit-no-file");

        // Act
        let output = run_hook(
            "preread-edit-check",
            &home,
            &payload("pec", "/tmp/x/file.py"),
        );

        // Assert
        assert!(output.status.success(), "exit code should be 0");
        assert_eq!(stdout_string(&output), "");
    }

    #[test]
    fn edit_just_inside_the_1800s_window_still_nudges() {
        // Arrange: 1750 seconds ago, close to but short of the 1800s window.
        // Not the exact 1799s boundary: parallel test threads spawning
        // subprocesses can add enough scheduling delay between seeding and
        // the hook reading its own clock to flip a razor-thin margin.
        let home = scratch_dir("edit-window-inside");
        seed_edits(&home, "pec", "/tmp/x/file.py", now() - 1750);

        // Act
        let output = run_hook(
            "preread-edit-check",
            &home,
            &payload("pec", "/tmp/x/file.py"),
        );

        // Assert
        let out = stdout_string(&output);
        assert!(
            out.contains("Edit/Write"),
            "expected a nudge just inside the window, got: {out}"
        );
    }

    #[test]
    fn edit_at_the_1800s_window_boundary_stays_silent() {
        // Arrange: exactly 1800 seconds ago; the python source compares with
        // strict `<`, so the boundary itself is excluded.
        let home = scratch_dir("edit-window-boundary");
        seed_edits(&home, "pec", "/tmp/x/file.py", now() - 1800);

        // Act
        let output = run_hook(
            "preread-edit-check",
            &home,
            &payload("pec", "/tmp/x/file.py"),
        );

        // Assert
        assert_eq!(
            stdout_string(&output),
            "",
            "the window boundary itself should not nudge"
        );
    }

    #[test]
    fn float_ts_inside_window_still_nudges() {
        // Arrange: a JSON float ts, the same as python's plain arithmetic
        // (`now - rec.get("ts", 0)`) would accept, since python does not
        // care whether the number is an int or a float.
        let home = scratch_dir("edit-float-ts");
        let ts = now() as f64 - 120.5;
        seed_edits_raw(
            &home,
            "pec",
            &format!(r#"{{"path":"/tmp/x/file.py","ts":{ts}}}"#),
        );

        // Act
        let output = run_hook(
            "preread-edit-check",
            &home,
            &payload("pec", "/tmp/x/file.py"),
        );

        // Assert
        let out = stdout_string(&output);
        assert!(
            out.contains("Edit/Write"),
            "a float ts inside the window should still nudge, got: {out}"
        );
    }

    #[test]
    fn string_ts_record_does_not_abandon_a_later_match() {
        // Arrange: the first record for this path carries a non-numeric
        // "ts". In python that makes `now - rec.get("ts", 0)` raise and
        // abandon the whole scan via the outer except, silently missing the
        // later, genuinely matching record. This port must keep scanning
        // instead and still nudge on that later record.
        let home = scratch_dir("edit-string-ts");
        let recent = now() - 90;
        seed_edits_raw(
            &home,
            "pec",
            &format!(
                "{{\"path\":\"/tmp/x/file.py\",\"ts\":\"garbage\"}}\n{{\"path\":\"/tmp/x/file.py\",\"ts\":{recent}}}\n"
            ),
        );

        // Act
        let output = run_hook(
            "preread-edit-check",
            &home,
            &payload("pec", "/tmp/x/file.py"),
        );

        // Assert
        let out = stdout_string(&output);
        assert!(
            out.contains("Edit/Write"),
            "a later valid record should still nudge despite an earlier malformed ts, got: {out}"
        );
    }
}

mod preread_size_check {
    use super::*;

    /// `{"tool_input":{"file_path":"<path>"<extra>}}`, matching the bash
    /// suite's `payload()` helper.
    fn payload(file_path: &str, extra: &str) -> String {
        format!(r#"{{"tool_input":{{"file_path":"{file_path}"{extra}}}}}"#)
    }

    /// A file with exactly `n` lines, each terminated by a newline, matching
    /// `seq 1 N` (line count = newline count).
    fn write_numbered_lines(path: &Path, n: usize) {
        let content: String = (1..=n).map(|i| format!("{i}\n")).collect();
        fs::write(path, content).expect("fixture file should be writable");
    }

    /// A file with `bytes` total bytes and no newlines at all.
    fn write_bytes_no_newlines(path: &Path, bytes: usize) {
        fs::write(path, "a".repeat(bytes)).expect("fixture file should be writable");
    }

    fn write_text(path: &Path, content: &str) {
        fs::write(path, content).expect("fixture file should be writable");
    }

    #[test]
    fn large_non_allowlisted_file_is_denied_with_counts() {
        // Arrange
        let home = scratch_dir("size-large-denied");
        let big = home.join("big.log");
        write_numbered_lines(&big, 1500);

        // Act
        let output = run_hook(
            "preread-size-check",
            &home,
            &payload(big.to_str().unwrap(), ""),
        );

        // Assert
        let out = stdout_string(&output);
        assert!(
            out.contains(r#""permissionDecision":"deny""#),
            "should deny, got: {out}"
        );
        assert!(
            out.contains("1500 lines"),
            "should report the line count, got: {out}"
        );
    }

    #[test]
    fn small_file_passes() {
        // Arrange
        let home = scratch_dir("size-small-passes");
        let small = home.join("small.txt");
        write_text(&small, "a\nb\nc\n");

        // Act
        let output = run_hook(
            "preread-size-check",
            &home,
            &payload(small.to_str().unwrap(), ""),
        );

        // Assert
        assert_eq!(
            stdout_string(&output),
            "",
            "a small file should pass silently"
        );
    }

    #[test]
    fn allowlisted_large_file_passes() {
        // Arrange
        let home = scratch_dir("size-allowlisted-passes");
        let package_json = home.join("package.json");
        write_numbered_lines(&package_json, 1500);

        // Act
        let output = run_hook(
            "preread-size-check",
            &home,
            &payload(package_json.to_str().unwrap(), ""),
        );

        // Assert
        assert_eq!(
            stdout_string(&output),
            "",
            "an allowlisted file should pass even when large"
        );
    }

    #[test]
    fn explicit_offset_bypasses_the_guard() {
        // Arrange
        let home = scratch_dir("size-offset-bypass");
        let big = home.join("big.log");
        write_numbered_lines(&big, 1500);

        // Act
        let output = run_hook(
            "preread-size-check",
            &home,
            &payload(big.to_str().unwrap(), r#","offset":10"#),
        );

        // Assert
        assert_eq!(
            stdout_string(&output),
            "",
            "an explicit offset should bypass the guard"
        );
    }

    #[test]
    fn explicit_limit_bypasses_the_guard() {
        // Arrange
        let home = scratch_dir("size-limit-bypass");
        let big = home.join("big.log");
        write_numbered_lines(&big, 1500);

        // Act
        let output = run_hook(
            "preread-size-check",
            &home,
            &payload(big.to_str().unwrap(), r#","limit":50"#),
        );

        // Assert
        assert_eq!(
            stdout_string(&output),
            "",
            "an explicit limit should bypass the guard"
        );
    }

    #[test]
    fn missing_file_is_a_silent_no_op() {
        // Arrange
        let home = scratch_dir("size-missing-file");
        let missing = home.join("does-not-exist");

        // Act
        let output = run_hook(
            "preread-size-check",
            &home,
            &payload(missing.to_str().unwrap(), ""),
        );

        // Assert
        assert!(output.status.success(), "exit code should be 0");
        assert!(
            !stderr_string(&output).contains("panicked"),
            "should never panic"
        );
        assert_eq!(stdout_string(&output), "");
    }

    #[test]
    fn tsconfig_glob_allowlist_matches() {
        // Arrange: tsconfig.build.json matches the tsconfig.*.json glob.
        let home = scratch_dir("size-tsconfig-glob");
        let tsconfig = home.join("tsconfig.build.json");
        write_numbered_lines(&tsconfig, 1500);

        // Act
        let output = run_hook(
            "preread-size-check",
            &home,
            &payload(tsconfig.to_str().unwrap(), ""),
        );

        // Assert
        assert_eq!(
            stdout_string(&output),
            "",
            "tsconfig.*.json glob should allowlist it"
        );
    }

    /// Table-driven over several allowlisted basenames, each backed by an
    /// oversized fixture, so the allowlist bypass is pinned across more than
    /// the one or two names the bash suite happens to cover.
    #[test]
    fn allowlisted_basenames_never_deny() {
        // Arrange, Act, Assert
        let names = [
            "README.md",
            "Cargo.lock",
            ".gitignore",
            "Makefile",
            "go.sum",
        ];
        for name in names {
            let home = scratch_dir(&format!("size-allowlist-{name}"));
            let fixture = home.join(name);
            write_numbered_lines(&fixture, 1500);

            let output = run_hook(
                "preread-size-check",
                &home,
                &payload(fixture.to_str().unwrap(), ""),
            );

            assert_eq!(
                stdout_string(&output),
                "",
                "{name} is allowlisted and should never be denied"
            );
        }
    }

    #[test]
    fn exactly_1000_lines_passes() {
        // Arrange: at the line limit, not over it.
        let home = scratch_dir("size-line-limit-at");
        let fixture = home.join("at-limit.log");
        write_numbered_lines(&fixture, 1000);

        // Act
        let output = run_hook(
            "preread-size-check",
            &home,
            &payload(fixture.to_str().unwrap(), ""),
        );

        // Assert
        assert_eq!(
            stdout_string(&output),
            "",
            "1000 lines is at the limit, not over it"
        );
    }

    #[test]
    fn exactly_1001_lines_denies() {
        // Arrange: one line past the limit.
        let home = scratch_dir("size-line-limit-over");
        let fixture = home.join("over-limit.log");
        write_numbered_lines(&fixture, 1001);

        // Act
        let output = run_hook(
            "preread-size-check",
            &home,
            &payload(fixture.to_str().unwrap(), ""),
        );

        // Assert
        let out = stdout_string(&output);
        assert!(
            out.contains("1001 lines"),
            "should deny one line past the limit, got: {out}"
        );
    }

    #[test]
    fn exactly_200kb_passes() {
        // Arrange: at the byte limit, with a line count nowhere near its own
        // limit, so only the byte threshold is on trial.
        let home = scratch_dir("size-byte-limit-at");
        let fixture = home.join("at-byte-limit.log");
        write_bytes_no_newlines(&fixture, 204_800);

        // Act
        let output = run_hook(
            "preread-size-check",
            &home,
            &payload(fixture.to_str().unwrap(), ""),
        );

        // Assert
        assert_eq!(
            stdout_string(&output),
            "",
            "204800 bytes is at the limit, not over it"
        );
    }

    #[test]
    fn one_byte_over_200kb_denies_even_with_few_lines() {
        // Arrange: one byte past the limit, still with no newlines at all.
        let home = scratch_dir("size-byte-limit-over");
        let fixture = home.join("over-byte-limit.log");
        write_bytes_no_newlines(&fixture, 204_801);

        // Act
        let output = run_hook(
            "preread-size-check",
            &home,
            &payload(fixture.to_str().unwrap(), ""),
        );

        // Assert
        let out = stdout_string(&output);
        assert!(
            out.contains(r#""permissionDecision":"deny""#),
            "should deny purely on byte size, got: {out}"
        );
        assert!(
            out.contains("204801 bytes"),
            "should report the byte count, got: {out}"
        );
    }

    #[test]
    fn unreadable_small_file_is_a_silent_no_op() {
        // Arrange: the file exists and is small, but its content cannot be
        // read (mode 0000); stat still reports its (small) size.
        let home = scratch_dir("size-unreadable");
        let fixture = home.join("locked.txt");
        write_text(&fixture, "a\nb\nc\n");
        let mut perms = fs::metadata(&fixture).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o000);
        fs::set_permissions(&fixture, perms).expect("chmod should succeed");

        // Act
        let output = run_hook(
            "preread-size-check",
            &home,
            &payload(fixture.to_str().unwrap(), ""),
        );

        // Assert
        assert!(output.status.success(), "exit code should be 0");
        assert!(
            !stderr_string(&output).contains("panicked"),
            "should never panic"
        );
        assert_eq!(
            stdout_string(&output),
            "",
            "an unreadable small file should not be denied"
        );

        // Cleanup: restore permissions so the scratch dir can be removed.
        let mut perms = fs::metadata(&fixture).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o644);
        let _ = fs::set_permissions(&fixture, perms);
    }

    #[test]
    fn large_and_unreadable_file_is_still_denied_on_byte_size() {
        // Arrange: the file is both too large and unreadable (mode 0000).
        // `fs::metadata` is a separate stat call that still succeeds for an
        // existing, unreadable file, so the byte-size check alone must
        // still deny it even though the content read fails and defaults
        // the line count to 0.
        let home = scratch_dir("size-large-unreadable");
        let fixture = home.join("locked-big.log");
        write_bytes_no_newlines(&fixture, 204_801);
        let mut perms = fs::metadata(&fixture).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o000);
        fs::set_permissions(&fixture, perms).expect("chmod should succeed");

        // Act
        let output = run_hook(
            "preread-size-check",
            &home,
            &payload(fixture.to_str().unwrap(), ""),
        );

        // Assert
        let out = stdout_string(&output);
        assert!(
            out.contains(r#""permissionDecision":"deny""#),
            "a large unreadable file should still be denied on byte size, got: {out}"
        );
        assert!(
            out.contains("204801 bytes"),
            "should report the byte count from stat, got: {out}"
        );
        assert!(
            out.contains("0 lines"),
            "the unreadable content should default the line count to 0, got: {out}"
        );

        // Cleanup: restore permissions so the scratch dir can be removed.
        let mut perms = fs::metadata(&fixture).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o644);
        let _ = fs::set_permissions(&fixture, perms);
    }

    #[test]
    fn deny_output_matches_the_python_reference_byte_for_byte() {
        // Arrange: the same oversized fixture and payload, fed to both the
        // Rust binary and the python hook it ports.
        let home = scratch_dir("size-byte-identical");
        let big = home.join("big.log");
        write_numbered_lines(&big, 1500);
        let payload_json = payload(big.to_str().unwrap(), "");

        // Act
        let rust_output = run_hook("preread-size-check", &home, &payload_json);
        let python_script = concat!(env!("CARGO_MANIFEST_DIR"), "/hooks/preread-size-check.py");
        let mut python_child = Command::new("python3")
            .arg(python_script)
            .env("HOME", &home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("python3 should be available to run the reference hook");
        python_child
            .stdin
            .as_mut()
            .expect("stdin should be piped")
            .write_all(payload_json.as_bytes())
            .expect("writing stdin should succeed");
        let python_output = python_child
            .wait_with_output()
            .expect("python3 should exit");

        // Assert
        let rust_stdout = stdout_string(&rust_output);
        let python_stdout = stdout_string(&python_output);
        assert!(
            rust_stdout.contains(r#""permissionDecision":"deny""#),
            "sanity: should deny"
        );
        assert_eq!(
            rust_stdout, python_stdout,
            "the deny JSON must be byte-identical to the python hook's output"
        );
    }
}
