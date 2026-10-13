// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The one way to run a short `git` command and read its answer. Every call
//! is bounded by a timeout, so a wedged repo never hangs a hook, and every
//! failure (git missing, timeout, non-zero exit) reads as `None` or `false`.

use crate::common::proc::run_with_timeout;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

/// Upper bound for one git call from a hook or a status render.
pub const TIMEOUT: Duration = Duration::from_secs(5);
/// For callers that run in parallel test suites or walk many worktrees, where
/// a 5 second bound flakes under load.
pub const SLOW_TIMEOUT: Duration = Duration::from_secs(15);

/// `git -C <dir>`, ready for arguments, for callers that need to set an
/// environment variable or feed standard input.
pub fn command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command
}

/// Runs `git [-C dir] <args>`. `None` when git cannot start or times out.
pub fn run(dir: Option<&Path>, args: &[&str], timeout: Duration) -> Option<Output> {
    let mut command = match dir {
        Some(dir) => command(dir),
        None => Command::new("git"),
    };
    command.args(args);
    run_with_timeout(&mut command, timeout)
}

/// Stdout of a run that succeeded, as it was printed.
pub fn untrimmed(out: Option<Output>) -> Option<String> {
    out.filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Stdout of a run that succeeded, without surrounding whitespace.
pub fn trimmed(out: Option<Output>) -> Option<String> {
    untrimmed(out).map(|text| text.trim().to_string())
}

/// Whether `git -C dir <args>` exited zero.
pub fn ok(dir: &Path, args: &[&str]) -> bool {
    run(Some(dir), args, TIMEOUT).is_some_and(|o| o.status.success())
}

/// Stdout of `git -C dir <args>` as printed, `None` on any failure.
pub fn raw(dir: &Path, args: &[&str]) -> Option<String> {
    untrimmed(run(Some(dir), args, TIMEOUT))
}

/// Trimmed stdout of `git -C dir <args>`, `None` on any failure.
pub fn output(dir: &Path, args: &[&str]) -> Option<String> {
    trimmed(run(Some(dir), args, TIMEOUT))
}

/// [`output`] with the longer [`SLOW_TIMEOUT`].
pub fn output_slow(dir: &Path, args: &[&str]) -> Option<String> {
    trimmed(run(Some(dir), args, SLOW_TIMEOUT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failing_command_reads_as_none_and_false() {
        let dir = std::env::temp_dir();
        assert!(!ok(&dir, &["definitely-not-a-git-command"]));
        assert_eq!(output(&dir, &["definitely-not-a-git-command"]), None);
    }

    #[test]
    fn output_trims_and_raw_keeps_the_newline() {
        let dir = std::env::temp_dir();
        let args = ["--version"];
        let trimmed = output(&dir, &args).expect("git is installed");
        let raw = raw(&dir, &args).expect("git is installed");
        assert!(trimmed.starts_with("git version"));
        assert!(raw.ends_with('\n'));
        assert_eq!(raw.trim(), trimmed);
    }

    #[test]
    fn a_missing_directory_fails_instead_of_running_in_the_cwd() {
        let missing = std::env::temp_dir().join("playbook-git-helper-no-such-dir");
        assert_eq!(output(&missing, &["--version"]), None);
    }
}
