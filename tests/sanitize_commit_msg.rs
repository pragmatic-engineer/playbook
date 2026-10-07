// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook sanitize commit-msg|pr-text <file> [--check]` from the real
//! binary: the default rewrites the file and exits 0, `--check` exits 1 when
//! anything would be removed, and stderr never carries a removed line's text.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn message_file(text: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("sanitize-msg-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join("COMMIT_EDITMSG");
    fs::write(&file, text).expect("message file");
    file
}

fn sanitize(kind: &str, file: &PathBuf, check: bool) -> (i32, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_playbook"));
    command.args(["sanitize", kind]).arg(file);
    if check {
        command.arg("--check");
    }
    let out = command.output().expect("playbook spawns");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const DIRTY: &str = "feat: x\n\nbody\n\nFixes: #1\nCo-Authored-By:Claude <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_01AbCdEfGhIjKlMnOpQr\n";

#[test]
fn a_clean_message_exits_zero_and_prints_nothing() {
    let text = "feat(hooks): add it\n\n- a body\n\nRefs: PLAT-1\nSigned-off-by: Sam Lee <sam@example.com>\nCo-authored-by: Kim Wu <kim@example.com>\n";
    let file = message_file(text);

    assert_eq!(
        sanitize("commit-msg", &file, false),
        (0, String::new(), String::new())
    );
    assert_eq!(
        sanitize("commit-msg", &file, true),
        (0, String::new(), String::new())
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), text);
}

#[test]
fn the_default_rewrites_the_file_reports_each_line_and_exits_zero() {
    let file = message_file(DIRTY);

    let (code, stdout, stderr) = sanitize("commit-msg", &file, false);

    assert_eq!(code, 0);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "sanitize: removed line 6 (credit trailer naming an AI)\n\
         sanitize: removed line 7 (Claude-Session line)\n"
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "feat: x\n\nbody\n\nFixes: #1\n"
    );
}

#[test]
fn check_exits_one_and_leaves_the_file_alone() {
    let file = message_file(DIRTY);

    let (code, _, stderr) = sanitize("commit-msg", &file, true);

    assert_eq!(code, 1);
    assert_eq!(
        stderr,
        "sanitize: would remove line 6 (credit trailer naming an AI)\n\
         sanitize: would remove line 7 (Claude-Session line)\n"
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), DIRTY);
}

#[test]
fn stderr_never_carries_a_removed_value() {
    let file = message_file("feat: x\n\nCo-authored-by: Claude <jane.doe@example.com>\n");

    let (_, _, stderr) = sanitize("commit-msg", &file, true);

    assert!(!stderr.contains("jane.doe"), "{stderr}");
    assert!(!stderr.contains("Claude <"), "{stderr}");
}

#[test]
fn the_editor_template_survives_untouched() {
    let template = "\n# Please enter the commit message.\n#\n# On branch main\n";
    let file = message_file(&format!(
        "feat: x\n\nGenerated with Claude Code\n{template}"
    ));

    let (code, _, _) = sanitize("commit-msg", &file, false);

    assert_eq!(code, 0);
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        format!("feat: x\n{template}")
    );
}

#[test]
fn pr_text_is_sanitised_as_free_text() {
    let file = message_file(
        "## Summary\n\nbody\n\nGenerated with [Claude Code](https://claude.com/claude-code)\n",
    );

    let (code, _, stderr) = sanitize("pr-text", &file, false);

    assert_eq!(code, 0);
    assert_eq!(stderr, "sanitize: removed line 5 (generated-with footer)\n");
    assert_eq!(fs::read_to_string(&file).unwrap(), "## Summary\n\nbody\n");
}

#[test]
fn a_missing_file_exits_one_naming_the_path() {
    let file = PathBuf::from("/nonexistent/COMMIT_EDITMSG");

    let (code, _, stderr) = sanitize("commit-msg", &file, false);

    assert_eq!(code, 1);
    assert!(stderr.contains("/nonexistent/COMMIT_EDITMSG"), "{stderr}");
}
