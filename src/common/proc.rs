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
//! polling `try_wait()` against `std::time::Instant` on a short sleep. A child
//! still running at the deadline is asked to stop with SIGTERM, so a program
//! such as `git` can remove its own lock files, and is killed only if it is
//! still there a moment later.

use std::io::{Read, Write};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Pause between polls of the child: fine-grained while it is young, so a
/// quick `git` call is caught within half a millisecond, coarse once it runs long.
const POLL_FAST: Duration = Duration::from_micros(500);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const FAST_WINDOW: Duration = Duration::from_millis(200);

/// How long a child gets to exit after SIGTERM before it is killed.
const TERM_GRACE: Duration = Duration::from_secs(1);

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
/// stop the child if it has not exited within `timeout`. Returns `None` on
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

/// Asks `child` to stop with SIGTERM and waits for it to go, then kills it if
/// it is still running after [`TERM_GRACE`].
fn stop(child: &mut Child) {
    #[cfg(unix)]
    {
        // SAFETY: `kill` only reads its two integer arguments.
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
        let until = Instant::now() + TERM_GRACE;
        while Instant::now() < until {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
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

    let started = Instant::now();
    let deadline = started + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(_) => {
                stop(&mut child);
                return None;
            }
        }
        if Instant::now() >= deadline {
            stop(&mut child);
            return None;
        }
        let young = started.elapsed() < FAST_WINDOW;
        std::thread::sleep(if young { POLL_FAST } else { POLL_INTERVAL });
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

    #[cfg(unix)]
    #[test]
    fn quick_children_are_not_held_back_by_the_poll_interval() {
        // Arrange
        let started = Instant::now();

        // Act
        for _ in 0..20 {
            let mut command = Command::new("true");
            run_with_timeout(&mut command, Duration::from_secs(5)).expect("true should run");
        }

        // Assert
        assert!(
            started.elapsed() < POLL_INTERVAL * 20 - Duration::from_millis(10),
            "20 quick children took {:?}, a 20ms poll needs at least 400ms",
            started.elapsed()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_child_past_the_deadline_gets_sigterm_before_it_is_killed() {
        let marker = std::env::temp_dir().join(format!("proc-term-{}", std::process::id()));
        let script = format!(
            "trap 'echo stopped > {}; exit 0' TERM; while :; do sleep 0.05; done",
            marker.display()
        );
        let mut command = Command::new("sh");
        command.args(["-c", &script]);

        let got = run_with_timeout(&mut command, Duration::from_millis(300));

        assert!(got.is_none(), "a timeout reports no output");
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap_or_default().trim(),
            "stopped",
            "the child ran its SIGTERM handler"
        );
        let _ = std::fs::remove_file(&marker);
    }

    #[cfg(unix)]
    #[test]
    fn a_child_that_ignores_sigterm_is_killed_after_the_grace_period() {
        let mut command = Command::new("sh");
        command.args(["-c", "trap '' TERM; while :; do sleep 0.05; done"]);
        let started = Instant::now();

        let got = run_with_timeout(&mut command, Duration::from_millis(200));

        assert!(got.is_none());
        let waited = started.elapsed();
        assert!(
            waited >= TERM_GRACE,
            "it was given the grace period: {waited:?}"
        );
        assert!(
            waited < Duration::from_secs(10),
            "and then killed: {waited:?}"
        );
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
