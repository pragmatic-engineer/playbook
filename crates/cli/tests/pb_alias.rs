// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook cc`, `playbook ccd` and the `pb` alias, run against the compiled
//! binary with a fake `claude` on PATH. The real claude is never started.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let dir = std::env::temp_dir().join(format!(
            "pb-alias-{tag}-{}-{}",
            std::process::id(),
            playbook::testing::run_id()
        ));
        for d in ["home", "proj", "bin"] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        let root = fs::canonicalize(&dir).unwrap();
        let claude = root.join("bin/claude");
        fs::write(
            &claude,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$FAKE_LOG\"\nexit ${FAKE_EXIT:-0}\n",
        )
        .unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
        Sandbox { root }
    }

    /// A copy of the binary reached through a link called `name`, so argv[0] is `name`.
    fn linked(&self, name: &str) -> PathBuf {
        let link = self.root.join("bin").join(name);
        let _ = fs::remove_file(&link);
        symlink(env!("CARGO_BIN_EXE_playbook"), &link).unwrap();
        link
    }

    fn run(&self, program: &Path, args: &[&str], exit: &str) -> Output {
        Command::new(program)
            .args(args)
            .current_dir(self.root.join("proj"))
            .env("HOME", self.root.join("home"))
            .env("FAKE_LOG", self.root.join("claude.log"))
            .env("FAKE_EXIT", exit)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.root.join("bin").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env_remove("PLAYBOOK_CC_CD_FILE")
            .output()
            .unwrap()
    }

    fn claude_calls(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("claude.log"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }
}

#[test]
fn cc_passes_its_arguments_to_claude_and_returns_its_exit_code() {
    // Arrange
    let sb = Sandbox::new("cc");

    // Act
    let out = sb.run(
        Path::new(env!("CARGO_BIN_EXE_playbook")),
        &["cc", "--model", "sonnet", "fix the bug"],
        "7",
    );

    // Assert
    assert_eq!(out.status.code(), Some(7));
    let calls = sb.claude_calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].contains("--model sonnet"), "{}", calls[0]);
    assert!(calls[0].ends_with("fix the bug"), "{}", calls[0]);
    assert!(!calls[0].contains("--dangerously-skip-permissions"));
}

#[test]
fn ccd_adds_skip_permissions_and_passes_the_rest_through() {
    // Arrange
    let sb = Sandbox::new("ccd");

    // Act
    let out = sb.run(
        Path::new(env!("CARGO_BIN_EXE_playbook")),
        &["ccd", "--model", "opus"],
        "0",
    );

    // Assert
    assert_eq!(out.status.code(), Some(0));
    let calls = sb.claude_calls();
    assert!(
        calls[0].contains("--dangerously-skip-permissions"),
        "{calls:?}"
    );
    assert!(calls[0].contains("--model opus"), "{calls:?}");
}

#[test]
fn pb_cc_behaves_like_playbook_cc_when_invoked_as_pb() {
    // Arrange
    let sb = Sandbox::new("pb");
    let pb = sb.linked("pb");

    // Act
    let out = sb.run(&pb, &["cc", "--model", "haiku"], "5");

    // Assert
    assert_eq!(out.status.code(), Some(5));
    assert!(sb.claude_calls()[0].contains("--model haiku"));
}

#[test]
fn pb_ccd_adds_skip_permissions() {
    // Arrange
    let sb = Sandbox::new("pbd");
    let pb = sb.linked("pb");

    // Act
    let out = sb.run(&pb, &["ccd"], "0");

    // Assert
    assert!(out.status.success());
    assert!(sb.claude_calls()[0].contains("--dangerously-skip-permissions"));
}

#[test]
fn help_names_playbook_and_mentions_pb_as_an_alias() {
    // Arrange
    let sb = Sandbox::new("help");
    let pb = sb.linked("pb");

    // Act
    let out = sb.run(&pb, &["--help"], "0");

    // Assert
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("Usage: playbook "), "{text}");
    assert!(text.contains("`pb` is a short alias"), "{text}");
}

#[test]
fn cc_subcommands_still_parse_after_the_passthrough_change() {
    // Arrange
    let sb = Sandbox::new("sub");

    // Act
    let out = sb.run(
        Path::new(env!("CARGO_BIN_EXE_playbook")),
        &["cc", "list"],
        "0",
    );

    // Assert
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(sb.claude_calls().is_empty());
}
