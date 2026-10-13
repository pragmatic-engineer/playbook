// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `pbtop` is the same program as `playbook-telemetry` under a shorter name.

use std::process::Command;

fn run(bin: &str, arg: &str) -> String {
    let path = match bin {
        "playbook-telemetry" => env!("CARGO_BIN_EXE_playbook-telemetry"),
        _ => env!("CARGO_BIN_EXE_pbtop"),
    };
    let out = Command::new(path).arg(arg).output().expect("spawn");
    assert!(out.status.success(), "{bin} {arg} failed");
    String::from_utf8(out.stdout).expect("utf8")
}

#[test]
fn both_binaries_print_the_same_version_under_their_own_name() {
    let full = run("playbook-telemetry", "--version");
    let short = run("pbtop", "--version");
    assert_eq!(
        full.trim(),
        format!("playbook-telemetry {}", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(short.trim(), format!("pbtop {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_shows_the_invoked_name() {
    assert!(run("pbtop", "--help").contains("Usage: pbtop"));
    assert!(run("playbook-telemetry", "--help").contains("Usage: playbook-telemetry"));
}

#[test]
fn help_names_the_alias_under_both_commands() {
    assert!(run("playbook-telemetry", "--help").contains("playbook-telemetry (alias: pbtop)"));
    assert!(run("pbtop", "--help").contains("pbtop (long name: playbook-telemetry)"));
}

#[test]
fn a_copy_of_the_binary_named_pbtop_prints_pbtop_in_the_usage_line() {
    let dir = std::env::temp_dir().join(format!("pbtop-copy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let copy = dir.join(if cfg!(windows) { "pbtop.exe" } else { "pbtop" });
    std::fs::copy(env!("CARGO_BIN_EXE_playbook-telemetry"), &copy).expect("copy");
    let out = Command::new(&copy).arg("--help").output().expect("spawn");
    let bad = Command::new(&copy).arg("--bogus").output().expect("spawn");
    let _ = std::fs::remove_dir_all(&dir);
    let help = String::from_utf8(out.stdout).expect("utf8");
    assert!(help.contains("Usage: pbtop"), "{help}");
    assert!(!help.contains("Usage: playbook-telemetry"), "{help}");
    let err = String::from_utf8(bad.stderr).expect("utf8");
    assert!(err.contains("Usage: pbtop"), "{err}");
}
