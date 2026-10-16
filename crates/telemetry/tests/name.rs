// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `pbtop` is the one standalone command, and it names itself `pbtop` under
//! any file name (ADR 0041).

use std::process::Command;

#[test]
fn pbtop_prints_its_version_under_its_own_name() {
    // Act
    let out = Command::new(env!("CARGO_BIN_EXE_pbtop"))
        .arg("--version")
        .output()
        .expect("spawn");

    // Assert
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("pbtop {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn a_copy_under_the_old_long_name_still_calls_itself_pbtop() {
    // Arrange
    let dir = std::env::temp_dir().join(format!("pbtop-name-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let name = if cfg!(windows) {
        "playbook-telemetry.exe"
    } else {
        "playbook-telemetry"
    };
    let copy = dir.join(name);
    std::fs::copy(env!("CARGO_BIN_EXE_pbtop"), &copy).expect("copy");

    // Act
    let help = Command::new(&copy).arg("--help").output().expect("spawn");
    let bad = Command::new(&copy).arg("--bogus").output().expect("spawn");
    let _ = std::fs::remove_dir_all(&dir);

    // Assert
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("Usage: pbtop"), "{help}");
    assert!(!help.contains("playbook-telemetry"), "{help}");
    let err = String::from_utf8_lossy(&bad.stderr);
    assert!(err.contains("Usage: pbtop"), "{err}");
}
