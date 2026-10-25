// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Runs one hook with a payload on stdin.

use super::auto_env::{hook_command, Scratch};
use std::io::Write;
use std::process::Stdio;

/// Run the hook with `stdin` and the extra `env` pairs, returning stdout and
/// the exit code.
pub fn run_hook(scratch: &Scratch, hook: &str, stdin: &str, env: &[(&str, &str)]) -> (String, i32) {
    let mut command = hook_command(scratch, hook);
    command
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().expect("playbook spawns");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("stdin accepts the payload");
    let out = child.wait_with_output().expect("playbook exits");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}
