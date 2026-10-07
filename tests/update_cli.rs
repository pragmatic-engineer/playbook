// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook update` wiring through the real binary. Nothing here touches the
//! network.

#![cfg(unix)]

use std::process::Command;

fn playbook() -> Command {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
}

#[test]
fn update_and_its_hidden_alias_share_one_help() {
    for name in ["update", "upgrade"] {
        let out = playbook().args([name, "--help"]).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{name} --help failed");
        for flag in ["--check", "--list", "--pre", "--yes"] {
            assert!(text.contains(flag), "{name} help lacks {flag}: {text}");
        }
    }
}
