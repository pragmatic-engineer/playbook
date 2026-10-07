// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook update` wiring and `playbook doctor path-shadow`, through the
//! real binary. Nothing here touches the network.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
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

#[test]
fn doctor_path_shadow_lists_each_playbook_in_path_order() {
    let root = std::env::temp_dir().join(format!("playbook-update-cli-{}", std::process::id()));
    let mut dirs = Vec::new();
    for (name, version) in [("first", "0.14.0"), ("second", "0.17.0")] {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("playbook");
        fs::write(&bin, format!("#!/bin/sh\necho 'playbook {version}'\n")).unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        dirs.push(dir);
    }

    let out = playbook()
        .args(["doctor", "path-shadow"])
        .env("PATH", std::env::join_paths(&dirs).unwrap())
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&root);
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "STALE_FIRST", "{text}");
    assert!(lines[1].ends_with(" 0.14.0"), "{text}");
    assert!(lines[2].ends_with(" 0.17.0"), "{text}");
}
