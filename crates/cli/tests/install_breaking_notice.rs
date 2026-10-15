// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The breaking changes notice that `install.sh` prints when it replaces an
//! older binary (ADR 0035). The installer runs as the real script under bash,
//! with the shared `curl`, `uname` and `gh` stubs serving a fake 0.17.0
//! release. `$HOME` and `$PLAYBOOK_BIN_DIR` are scratch directories and every
//! inherited `PLAYBOOK_*`, `CLAUDE_*`, `XDG_*` and `CI` variable is removed.

#![cfg(unix)]

#[path = "support/shell_fixture.rs"]
mod fx;

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use fx::{both, install_stubs, repo_root, sha256_hex, write_exec, Work};

/// A 0.17.0 binary that knows `breaking` and `migrate`. It lists one entry
/// only for `breaking --since 0.16.0`. Every other call except `--version`
/// logs its arguments and whether the installer marked the notice as shown.
const NEW_BINARY: &str = r#"#!/usr/bin/env bash
case "$*" in
  --version|version) echo "playbook 0.17.0"; exit 0 ;;
esac
printf '%s|%s\n' "$*" "${PLAYBOOK_BREAKING_SHOWN:-unset}" >> "$HOME/calls.log"
case "$*" in
  "breaking --since 0.16.0") printf 'Breaking changes in this upgrade:\n  playbook 0.17.0: The old flag is gone.\n    Action: playbook init\n' ;;
  breaking*) exit 2 ;;
  *) exit 0 ;;
esac"#;

/// A 0.17.0 binary from before the `breaking` and `migrate` commands: it
/// rejects every subcommand the way clap does.
const OLD_STYLE_BINARY: &str = r#"#!/usr/bin/env bash
case "$1" in
  --version|version) echo "playbook 0.17.0" ;;
  *) echo "error: unrecognized subcommand '$1'" >&2; exit 2 ;;
esac"#;

struct Sandbox {
    work: Work,
    stubs: PathBuf,
    home: PathBuf,
    bin_dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let work = Work::new(&format!("install-breaking-{tag}"));
        let stubs = work.dir("stubs");
        let home = work.dir("home");
        let bin_dir = home.join("bin");
        install_stubs(&stubs);
        Self {
            work,
            stubs,
            home,
            bin_dir,
        }
    }

    /// Puts an installed playbook in the bin dir that answers `--version`
    /// with `reply`.
    fn with_installed(self, reply: &str) -> Self {
        write_exec(
            &self.bin_dir.join("playbook"),
            &format!("#!/usr/bin/env bash\necho \"{reply}\"\n"),
        );
        self
    }

    /// Installs `body` as release 0.17.0 with `flags`, returning stdout then
    /// stderr.
    fn install(&self, body: &str, flags: &[&str]) -> String {
        let sums = format!(
            "{}  playbook-0.17.0-x86_64-unknown-linux-musl",
            sha256_hex(body.as_bytes())
        );
        let mut path = vec![self.stubs.clone()];
        path.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let mut command = Command::new("bash");
        command
            .arg(repo_root().join("install.sh"))
            .args(flags)
            .current_dir(&self.work.0)
            .stdin(Stdio::null());
        for (key, _) in std::env::vars_os() {
            let key_text = key.to_string_lossy();
            if ["PLAYBOOK_", "CLAUDE_", "XDG_"]
                .iter()
                .any(|prefix| key_text.starts_with(prefix))
                || key_text == "CI"
            {
                command.env_remove(&key);
            }
        }
        command
            .env("HOME", &self.home)
            .env("CLAUDE_HOME", self.home.join(".claude"))
            .env("PLAYBOOK_BIN_DIR", &self.bin_dir)
            .env(
                "PATH",
                std::env::join_paths(path).expect("PATH should join"),
            )
            .env("SHELL", "/bin/bash")
            .env("STUB_BODY", "{\"tag_name\": \"v0.17.0\"}")
            .env("STUB_ASSET_BODY", body)
            .env("STUB_SUMS_BODY", sums);
        let out = both(&command.output().expect("bash should spawn"));
        assert!(
            out.contains("Installed playbook 0.17.0"),
            "install did not reach the end of the binary step: {out}"
        );
        out
    }

    /// What the stub saw in PLAYBOOK_BREAKING_SHOWN for the calls whose
    /// arguments are exactly `args`.
    fn shown_for(&self, args: &str) -> Vec<String> {
        fs::read_to_string(self.home.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_once('|'))
            .filter(|(a, _)| *a == args)
            .map(|(_, shown)| shown.to_string())
            .collect()
    }
}

#[test]
fn an_upgrade_prints_the_breaking_changes_since_the_old_version() {
    // Arrange
    let sandbox = Sandbox::new("upgrade").with_installed("playbook 0.16.0");

    // Act
    let out = sandbox.install(NEW_BINARY, &["--no-init"]);

    // Assert
    assert!(
        out.contains("Breaking changes in this upgrade:\n  playbook 0.17.0: The old flag is gone.\n    Action: playbook init"),
        "{out}"
    );
}

#[test]
fn a_shown_notice_is_marked_only_for_the_migrate_and_init_that_run_next() {
    // Arrange
    let upgrade = Sandbox::new("shown").with_installed("playbook 0.16.0");
    let fresh = Sandbox::new("not-shown");

    // Act
    upgrade.install(NEW_BINARY, &["--yes"]);
    fresh.install(NEW_BINARY, &["--yes"]);

    // Assert
    assert_eq!(upgrade.shown_for("migrate"), ["1"]);
    assert_eq!(upgrade.shown_for("init"), ["1"]);
    assert_eq!(upgrade.shown_for("init --help"), ["unset"]);
    assert_eq!(fresh.shown_for("migrate"), ["unset"]);
    assert_eq!(fresh.shown_for("init"), ["unset"]);
}

#[test]
fn a_fresh_install_prints_no_breaking_changes() {
    // Arrange
    let sandbox = Sandbox::new("fresh");

    // Act
    let out = sandbox.install(NEW_BINARY, &["--no-init"]);

    // Assert
    assert!(!out.contains("Breaking changes"), "{out}");
}

#[test]
fn a_reinstall_of_the_same_version_prints_no_breaking_changes() {
    // Arrange
    let sandbox = Sandbox::new("same").with_installed("playbook 0.17.0");

    // Act
    let out = sandbox.install(NEW_BINARY, &["--no-init"]);

    // Assert
    assert!(!out.contains("Breaking changes"), "{out}");
}

#[test]
fn an_installed_binary_without_a_plain_version_prints_no_breaking_changes() {
    // Arrange
    let sandbox = Sandbox::new("unreadable").with_installed("error: broken!");

    // Act
    let out = sandbox.install(NEW_BINARY, &["--no-init"]);

    // Assert
    assert!(!out.contains("Breaking changes"), "{out}");
    assert!(out.contains("playbook.unknown-"), "{out}");
}

#[test]
fn a_binary_without_the_breaking_command_prints_nothing_extra() {
    // Arrange
    let sandbox = Sandbox::new("old-style").with_installed("playbook 0.16.0");

    // Act
    let out = sandbox.install(OLD_STYLE_BINARY, &["--no-init"]);

    // Assert
    assert!(!out.contains("Breaking changes"), "{out}");
    assert!(!out.contains("unrecognized subcommand"), "{out}");
}
