// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `pb` alias step of `install.sh`: it links `pb` to `playbook` and never
//! replaces a different `pb`. Runs the real function, cut out of the script.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;
use std::process::Command;

fn function_text() -> String {
    let script =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../install.sh"))
            .unwrap();
    let start = script.find("install_pb_alias() {").unwrap();
    let end = start + script[start..].find("\n}\n").unwrap() + 3;
    script[start..end].to_string()
}

/// Runs the function with `bin` as the install dir and `path_dirs` on PATH.
fn run(bin: &PathBuf, path_dirs: &[&PathBuf]) -> String {
    let path: Vec<String> = path_dirs.iter().map(|d| d.display().to_string()).collect();
    let program = format!(
        "log() {{ printf '%s\\n' \"$*\"; }}\nPLAYBOOK_BIN_DIR='{}'\n{}\ninstall_pb_alias",
        bin.display(),
        function_text()
    );
    let out = Command::new("/bin/bash")
        .args(["-c", &program])
        .env("PATH", format!("{}:/usr/bin:/bin", path.join(":")))
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pb-alias-install-{tag}-{}-{}",
        std::process::id(),
        playbook::testing::run_id()
    ));
    fs::create_dir_all(&dir).unwrap();
    fs::canonicalize(&dir).unwrap()
}

#[test]
fn pb_is_linked_to_playbook_when_the_name_is_free() {
    // Arrange
    let bin = scratch("free");
    fs::write(bin.join("playbook"), "x").unwrap();

    // Act
    let said = run(&bin, &[&bin]);

    // Assert
    assert_eq!(
        fs::read_link(bin.join("pb")).unwrap(),
        PathBuf::from("playbook")
    );
    assert!(said.contains("Linked"), "{said}");
}

#[test]
fn an_existing_different_pb_file_is_never_replaced() {
    // Arrange
    let bin = scratch("file");
    fs::write(bin.join("pb"), "mine").unwrap();

    // Act
    let said = run(&bin, &[&bin]);

    // Assert
    assert_eq!(fs::read_to_string(bin.join("pb")).unwrap(), "mine");
    assert!(said.contains("Left"), "{said}");
    assert_eq!(said.lines().count(), 1);
}

#[test]
fn another_pb_earlier_on_path_is_left_alone_and_nothing_is_linked() {
    // Arrange
    let bin = scratch("path-bin");
    let other = scratch("path-other");
    let tool = other.join("pb");
    fs::write(&tool, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();

    // Act
    let said = run(&bin, &[&other, &bin]);

    // Assert
    assert!(!bin.join("pb").exists());
    assert!(said.contains("already a command"), "{said}");
}

#[test]
fn rerunning_with_our_own_link_in_place_is_silent() {
    // Arrange
    let bin = scratch("again");
    symlink("playbook", bin.join("pb")).unwrap();

    // Act
    let said = run(&bin, &[&bin]);

    // Assert
    assert!(said.is_empty(), "{said}");
}
