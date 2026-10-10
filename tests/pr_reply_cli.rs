// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook pr reply`, with `gh` replaced by a stub on PATH that records its
//! arguments and the body file it was handed, so no test reaches GitHub.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pb-pr-reply-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `gh` that logs `argv` and the contents of any `body=@file` field, and
/// prints a URL for `api` calls.
fn stub_gh(dir: &Path, fail: bool) {
    let body = format!(
        r#"echo "$@" >> '{d}/argv'
for a in "$@"; do case "$a" in body=@*) cat "${{a#body=@}}" > '{d}/posted' ;; esac; done
case "$1" in
  repo) echo "acme/widget" ;;
  api) {fail} echo "https://github.com/acme/widget/pull/12#discussion_r345" ;;
esac"#,
        d = dir.display(),
        fail = if fail {
            "echo 'HTTP 404' >&2; exit 1;"
        } else {
            ""
        }
    );
    let path = dir.join("gh");
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn reply(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["pr", "reply"])
        .args(args)
        .current_dir(dir)
        .env("HOME", dir)
        .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
        .output()
        .unwrap()
}

fn write_body(dir: &Path, text: &str) -> PathBuf {
    let p = dir.join("reply.md");
    fs::write(&p, text).unwrap();
    p
}

#[test]
fn a_thread_reply_posts_the_file_verbatim_and_prints_the_url_and_body() {
    let dir = scratch("thread");
    stub_gh(&dir, false);
    let text = "Fixed in `abc1234`: the \"cap\" is now 8.\nThanks for the catch.";
    let body = write_body(&dir, text);

    let out = reply(
        &dir,
        &[
            "--pr",
            "12",
            "--thread",
            "345",
            "--body-file",
            body.to_str().unwrap(),
        ],
    );

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("Posted: https://github.com/acme/widget/pull/12#discussion_r345\n"));
    assert!(stdout.contains(text));
    let argv = fs::read_to_string(dir.join("argv")).unwrap();
    assert!(
        argv.contains("/repos/acme/widget/pulls/12/comments/345/replies"),
        "{argv}"
    );
    assert_eq!(fs::read_to_string(dir.join("posted")).unwrap(), text);
}

#[test]
fn a_pr_level_reply_uses_the_issue_comments_endpoint_and_an_explicit_repo() {
    let dir = scratch("issue");
    stub_gh(&dir, false);
    let body = write_body(&dir, "Done.");

    let out = reply(
        &dir,
        &[
            "--pr",
            "7",
            "--issue",
            "--repo",
            "o/r",
            "--body-file",
            body.to_str().unwrap(),
        ],
    );

    assert!(out.status.success());
    let argv = fs::read_to_string(dir.join("argv")).unwrap();
    assert!(argv.contains("/repos/o/r/issues/7/comments"), "{argv}");
    assert!(
        !argv.contains("repo view"),
        "an explicit --repo needs no lookup"
    );
}

#[test]
fn bad_input_exits_1_and_never_calls_the_api() {
    let dir = scratch("bad");
    stub_gh(&dir, false);
    for (text, needle) in [
        ("", "empty"),
        ("fixed \u{2014} done", "dash"),
        (
            "ok\n\nCo-Authored-By: Claude <noreply@anthropic.com>",
            "attribution",
        ),
    ] {
        let body = write_body(&dir, text);
        let out = reply(
            &dir,
            &[
                "--pr",
                "12",
                "--thread",
                "1",
                "--body-file",
                body.to_str().unwrap(),
            ],
        );
        assert_eq!(out.status.code(), Some(1), "{text:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(needle),
            "{text:?}"
        );
    }
    let body = write_body(&dir, "fine");
    let out = reply(
        &dir,
        &[
            "--pr",
            "x1",
            "--issue",
            "--body-file",
            body.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(!dir.join("argv").exists(), "no gh call may be made");
}

#[test]
fn a_failed_post_is_reported_plainly() {
    let dir = scratch("fail");
    stub_gh(&dir, true);
    let body = write_body(&dir, "Done.");

    let out = reply(
        &dir,
        &[
            "--pr",
            "12",
            "--thread",
            "1",
            "--body-file",
            body.to_str().unwrap(),
            "--repo",
            "o/r",
        ],
    );

    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("posting the reply failed") && err.contains("404"),
        "{err}"
    );
}

#[test]
fn a_thread_or_issue_is_required_and_they_conflict() {
    let dir = scratch("flags");
    let body = write_body(&dir, "x");
    let b = body.to_str().unwrap();
    assert!(!reply(&dir, &["--pr", "1", "--body-file", b])
        .status
        .success());
    assert!(!reply(
        &dir,
        &["--pr", "1", "--thread", "2", "--issue", "--body-file", b]
    )
    .status
    .success());
}

#[test]
fn address_pr_comments_applies_edits_and_posts_replies_itself_without_an_agent() {
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("commands/address-pr-comments.md"),
    )
    .unwrap();
    assert!(text.contains("playbook pr reply --pr"));
    assert!(!text.contains("patch-applier"));
    let tools = text
        .lines()
        .find_map(|l| l.strip_prefix("allowed-tools:"))
        .unwrap();
    assert!(
        !tools.split(',').any(|t| t.trim() == "Agent"),
        "the command spawns nothing, so it holds no Agent tool"
    );
}
