// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook check commit-msg <file>` from the real binary: exit 0 when the
//! message is pristine, exit 1 with one line per problem otherwise, so a git
//! `commit-msg` hook or CI can call it directly.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn message_file(text: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("check-commit-msg-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join("COMMIT_EDITMSG");
    fs::write(&file, text).expect("message file");
    file
}

fn check(file: &PathBuf) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["check", "commit-msg"])
        .arg(file)
        .output()
        .expect("playbook spawns");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_pristine_message_exits_zero_and_prints_nothing() {
    let file = message_file(
        "feat(hooks): block attribution\n\n- stops AI trailers\n\nRefs: PLAT-1\nSigned-off-by: Sam Lee <sam@example.com>\nCo-authored-by: Kim Wu <kim@example.com>\n",
    );

    assert_eq!(check(&file), (0, String::new(), String::new()));
}

#[test]
fn each_problem_is_one_line_on_stderr_and_the_exit_code_is_one() {
    let file = message_file(
        "feat: x\n\nbody\n\nFixes: #1\nClaude-Session: https://claude.ai/code/session_01AbCdEfGhIjKlMnOpQr\n",
    );

    let (code, stdout, stderr) = check(&file);

    assert_eq!(code, 1);
    assert_eq!(stdout, "");
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 3, "{stderr}");
    assert!(lines[0].contains("Claude-Session"), "{stderr}");
    assert!(lines[1].contains("\"Fixes\""), "{stderr}");
    assert!(lines[2].contains("\"Claude-Session\""), "{stderr}");
}

#[test]
fn a_missing_file_exits_one_naming_the_path() {
    let file = PathBuf::from("/nonexistent/COMMIT_EDITMSG");

    let (code, _, stderr) = check(&file);

    assert_eq!(code, 1);
    assert!(stderr.contains("/nonexistent/COMMIT_EDITMSG"), "{stderr}");
}
