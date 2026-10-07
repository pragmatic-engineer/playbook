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

use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How often to poll the child for exit while waiting on the deadline.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Reads a pipe to the end on its own thread, so a child that writes more
/// than the pipe buffer holds never blocks waiting for us to read.
fn drain<R: Read + Send + 'static>(mut pipe: R) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        let _ = tx.send(bytes);
    });
    rx
}

fn collect_by(rx: &mpsc::Receiver<Vec<u8>>, deadline: Instant) -> Option<Vec<u8>> {
    rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .ok()
}

/// Run `command` to completion, capturing stdout and stderr, but give up and
/// kill the child if it has not exited within `timeout`. Returns `None` on
/// spawn failure or timeout, mirroring python's "an exception becomes an
/// empty value" contract; callers still inspect `Output::status` themselves
/// for a non-zero exit, exactly as they did with `Command::output()`. Output
/// is drained while the child runs, so a chatty child (a git hook, a push)
/// cannot stall on a full pipe. Never panics.
pub fn run_with_timeout(command: &mut Command, timeout: Duration) -> Option<Output> {
    run_bounded(command, None, timeout)
}

/// Like [`run_with_timeout`], with `input` written to the child's standard
/// input, then closed. The write happens on its own thread, so a child that
/// does not read it cannot stall the deadline.
pub fn run_with_input(command: &mut Command, input: &[u8], timeout: Duration) -> Option<Output> {
    run_bounded(command, Some(input.to_vec()), timeout)
}

fn run_bounded(command: &mut Command, input: Option<Vec<u8>>, timeout: Duration) -> Option<Output> {
    let stdin = if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    let mut child = command
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    if let (Some(bytes), Some(mut pipe)) = (input, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = pipe.write_all(&bytes);
        });
    }
    let (Some(out_pipe), Some(err_pipe)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let out_rx = drain(out_pipe);
    let err_rx = drain(err_pipe);

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    // A grandchild that keeps a pipe open past the child's exit must not hang
    // us either: the same deadline bounds the final read.
    let stdout = collect_by(&out_rx, deadline)?;
    let stderr = collect_by(&err_rx, deadline)?;
    Some(Output {
        status,
        stdout,
        stderr,
    })
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
    fn run_with_input_feeds_standard_input_to_the_child() {
        let mut command = Command::new("cat");

        let got = run_with_input(&mut command, b"from stdin", Duration::from_secs(5));

        let output = got.expect("cat should finish well within the deadline");
        assert_eq!(String::from_utf8_lossy(&output.stdout), "from stdin");
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

    #[test]
    fn a_child_writing_far_more_than_the_pipe_buffer_does_not_stall() {
        // Arrange: 300 KB on each stream, several times the 64 KB pipe buffer.
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "head -c 300000 /dev/zero | tr '\\0' a; head -c 300000 /dev/zero | tr '\\0' b >&2",
        ]);
        let started = Instant::now();

        // Act
        let got = run_with_timeout(&mut command, Duration::from_secs(10));

        // Assert
        let output = got.expect("the child finishes once its output is drained");
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 300_000);
        assert_eq!(output.stderr.len(), 300_000);
        assert!(output.stdout.iter().all(|&b| b == b'a'));
        assert!(output.stderr.iter().all(|&b| b == b'b'));
        assert!(started.elapsed() < Duration::from_secs(8));
    }
}
