// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A closed pipe on stdout ends the command quietly: no panic text, no 101.

#![cfg(unix)]

use std::process::{Command, Stdio};

#[test]
fn a_closed_reader_does_not_panic_the_cli() {
    let home =
        std::env::temp_dir().join(format!("playbook-sigpipe-{}", playbook::testing::run_id()));
    std::fs::create_dir_all(&home).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["config", "list"])
        .env("HOME", &home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Close our read end before the child writes, like `| head -0`.
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !err.contains("panicked") && !err.contains("Broken pipe"),
        "{err}"
    );
    assert_ne!(out.status.code(), Some(101), "{err}");
}
