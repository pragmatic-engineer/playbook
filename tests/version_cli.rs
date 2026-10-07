// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook version` must print exactly what `playbook --version` prints.

use std::process::Command;

fn run(arg: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .arg(arg)
        .output()
        .expect("spawn playbook")
}

#[test]
fn version_subcommand_matches_version_flag() {
    let sub = run("version");
    let flag = run("--version");

    assert!(sub.status.success(), "exit: {:?}", sub.status);
    assert!(flag.status.success());
    assert_eq!(sub.stdout, flag.stdout);

    let stdout = String::from_utf8(sub.stdout).expect("utf8");
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")), "got: {stdout}");
}
