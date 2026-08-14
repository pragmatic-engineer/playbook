// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Deadline-bounded subprocess execution, standing in for python's
//! `subprocess.run(..., timeout=N)` (hooks/session-init.py:27-32,
//! hooks/lib/common.py:250-253, hooks/session-clean-exit.py:88-96).
//! `std::process::Command` has no built-in timeout, so an unbounded `git` or
//! `bash` call can hang a hook forever; `session-init` in particular runs on
//! every session start, so a hang there blocks the session from ever
//! beginning.
//!
//! No extra crate: the deadline is enforced by spawning the child, then
//! polling `try_wait()` against `std::time::Instant` on a short sleep,
//! killing the child if the deadline passes before it exits on its own.

use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// How often to poll the child for exit while waiting on the deadline.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Run `command` to completion, capturing stdout and stderr, but give up and
/// kill the child if it has not exited within `timeout`. Returns `None` on
/// spawn failure or timeout, mirroring python's "an exception becomes an
/// empty value" contract; callers still inspect `Output::status` themselves
/// for a non-zero exit, exactly as they did with `Command::output()`. Never
/// panics.
pub fn run_with_timeout(command: &mut Command, timeout: Duration) -> Option<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_stdout_of_a_process_that_finishes_in_time() {
        // Arrange
        let mut command = Command::new("echo");
        command.arg("hello");

        // Act
        let got = run_with_timeout(&mut command, Duration::from_secs(5));

        // Assert
        let output = got.expect("echo should finish well within the deadline");
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }

    #[test]
    fn returns_none_when_the_child_outlives_the_deadline() {
        // Arrange
        let mut command = Command::new("sleep");
        command.arg("2");
        let started = Instant::now();

        // Act
        let got = run_with_timeout(&mut command, Duration::from_millis(50));

        // Assert
        assert!(got.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "should give up at the deadline instead of waiting for the child"
        );
    }

    #[test]
    fn returns_none_on_spawn_failure() {
        // Arrange
        let mut command = Command::new("playbook-proc-test-binary-that-does-not-exist");

        // Act
        let got = run_with_timeout(&mut command, Duration::from_secs(1));

        // Assert
        assert!(got.is_none());
    }
}
