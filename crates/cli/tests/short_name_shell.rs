// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `pb` short name from `playbook shell-init` with `launcher.shortName` on,
//! in bash and zsh, with a fake `playbook` on PATH and a scratch HOME. `pb` is
//! defined only when no command, alias, function or builtin already has the name.

use playbook::init::shell_init::{script_with, script_with_options};
use playbook::init::shim::ShellKind;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Sandbox {
    root: PathBuf,
}

fn exec(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

impl Sandbox {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("pb-short-{}-{n}", playbook::testing::run_id()));
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        exec(
            &root.join("bin/playbook"),
            "#!/bin/sh\necho \"PLAYBOOK $*\" >> \"$LOG\"\n",
        );
        Sandbox { root }
    }

    fn run(&self, shell: &str, prelude: &str, body: &str) -> Vec<String> {
        let kind = if shell == "zsh" {
            ShellKind::Zsh
        } else {
            ShellKind::Bash
        };
        let script = self.root.join("init.sh");
        fs::write(&script, script_with_options(kind, false, true)).unwrap();
        let program = format!("{prelude}\n. '{}'\n{body}\n", script.display());
        let log = self.root.join("log");
        let _ = fs::remove_file(&log);
        let out = Command::new(shell)
            .args(["-c", &program])
            .env("HOME", self.root.join("home"))
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root.join("bin").display()),
            )
            .env("LOG", &log)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{shell}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }
}

fn shells() -> Vec<&'static str> {
    ["bash", "zsh"]
        .into_iter()
        .filter(|s| Command::new(s).arg("-c").arg("true").status().is_ok())
        .collect()
}

#[test]
fn pb_runs_playbook_when_the_name_is_free() {
    for shell in shells() {
        let sb = Sandbox::new();
        let log = sb.run(shell, "", "pb doctor list");
        assert_eq!(log, vec!["PLAYBOOK doctor list".to_string()], "{shell}");
    }
}

#[test]
fn a_command_named_pb_on_path_is_left_alone() {
    for shell in shells() {
        let sb = Sandbox::new();
        exec(
            &sb.root.join("bin/pb"),
            "#!/bin/sh\necho \"OTHER $*\" >> \"$LOG\"\n",
        );
        let log = sb.run(shell, "", "pb hi");
        assert_eq!(log, vec!["OTHER hi".to_string()], "{shell}");
    }
}

#[test]
fn a_user_function_named_pb_is_left_alone() {
    for shell in shells() {
        let sb = Sandbox::new();
        let prelude = "pb() { echo \"MINE $*\" >> \"$LOG\"; }";
        let log = sb.run(shell, prelude, "pb hi");
        assert_eq!(log, vec!["MINE hi".to_string()], "{shell}");
    }
}

#[test]
fn a_user_alias_named_pb_is_left_alone() {
    for shell in shells() {
        let sb = Sandbox::new();
        let prelude = "alias pb='echo MINE >> \"$LOG\" #'\nsetopt aliases 2>/dev/null; shopt -s expand_aliases 2>/dev/null";
        let log = sb.run(shell, prelude, "eval 'pb hi'");
        assert_eq!(log, vec!["MINE".to_string()], "{shell}");
    }
}

#[test]
fn sourcing_twice_keeps_one_working_definition() {
    for shell in shells() {
        let sb = Sandbox::new();
        let script = sb.root.join("init.sh");
        let body = format!(". '{}'\npb once", script.display());
        let log = sb.run(shell, "", &body);
        assert_eq!(log, vec!["PLAYBOOK once".to_string()], "{shell}");
    }
}

#[test]
fn the_short_name_is_absent_unless_asked_for() {
    let text = script_with(ShellKind::Bash, true);
    assert!(!text.contains("_playbook_pb_taken"));
    assert!(text.contains("\nfunction playbook {"));
}

#[test]
fn golden_text_of_the_short_name_snippet() {
    let text = script_with_options(ShellKind::Zsh, false, true);
    let tail = text.split_once("# Optional short name").unwrap().1;
    assert!(tail.contains("_playbook_pb_taken() {"));
    assert!(tail.contains("_PLAYBOOK_PB_DEF='pb() { playbook \"$@\"; }'"));
    assert!(tail.ends_with("unset _PLAYBOOK_PB_DEF\n"));
}

#[test]
fn our_own_pb_link_to_playbook_does_not_block_the_pb_function() {
    for shell in shells() {
        // Arrange
        let sb = Sandbox::new();
        std::os::unix::fs::symlink("playbook", sb.root.join("bin/pb")).unwrap();

        // Act
        let log = sb.run(shell, "", "pb doctor");

        // Assert
        assert_eq!(log, vec!["PLAYBOOK doctor".to_string()], "{shell}");
        let type_of = sb.run(shell, "", "type pb > \"$LOG\" 2>&1");
        assert!(type_of[0].contains("function"), "{shell}: {type_of:?}");
    }
}
