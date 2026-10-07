// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook deps ensure`, ported from the former shell test. Each case runs
//! the binary with a PATH of stub tools (a fake `brew` that logs its
//! arguments, fake commands that count as already installed).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("playbook-deps-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn stub(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A `brew` stub that appends its arguments to `log`; `tap_list` is what a
/// bare `brew tap` prints; `install_exit` is the exit code of `brew install`.
fn brew(dir: &Path, log: &Path, tap_list: &str, install_exit: i32) {
    stub(
        dir,
        "brew",
        &format!(
            "if [ \"$1\" = tap ] && [ $# -eq 1 ]; then echo '{tap_list}'; fi\n\
             echo \"$@\" >> '{}'\n\
             if [ \"$1\" = install ]; then exit {install_exit}; fi",
            log.display()
        ),
    );
}

fn run(stubs: &Path, brewfile: &str) -> (Output, String, String) {
    let file = stubs.join("Brewfile");
    fs::write(&file, brewfile).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["deps", "ensure"])
        .arg(&file)
        .env("PATH", format!("{}:/usr/bin:/bin", stubs.display()))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    (out, stdout, stderr)
}

fn log(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn a_dep_already_on_path_is_kept_and_not_installed() {
    let d = scratch("present");
    let l = d.join("brew.log");
    brew(&d, &l, "", 0);
    stub(&d, "pbk_present", "echo ok");

    let (out, stdout, _) = run(&d, "brew \"pbk_present\"\n");

    assert!(out.status.success());
    assert!(stdout.contains("pbk_present already installed"), "{stdout}");
    assert!(!log(&l).contains("install"));
}

#[test]
fn a_missing_dep_is_installed_with_brew() {
    let d = scratch("missing");
    let l = d.join("brew.log");
    brew(&d, &l, "", 0);

    let (out, stdout, _) = run(&d, "brew \"pbk_absent\"\n");

    assert!(out.status.success());
    assert!(stdout.contains("installing pbk_absent via Homebrew"));
    assert!(log(&l).contains("install pbk_absent"));
}

#[test]
fn without_brew_a_missing_dep_prints_guidance_and_exits_zero() {
    let d = scratch("nobrew");

    let (out, _, stderr) = run(&d, "brew \"pbk_absent\"\n");

    assert!(out.status.success());
    assert!(stderr.contains("Homebrew is unavailable"), "{stderr}");
}

#[test]
fn only_the_missing_formulae_are_installed() {
    let d = scratch("only-missing");
    let l = d.join("brew.log");
    brew(&d, &l, "", 0);
    stub(&d, "pbk_present", "echo ok");

    let (_, stdout, _) = run(
        &d,
        "# a comment, not a formula\nbrew \"pbk_present\"  # stubbed\nbrew \"pbk_a1\"\nbrew \"pbk_a2\"\n",
    );

    let installed = log(&l);
    assert!(installed.contains("install pbk_a1") && installed.contains("install pbk_a2"));
    assert!(!installed.contains("install pbk_present"));
    assert!(stdout.contains("pbk_present already installed"));
}

#[test]
fn a_tap_is_added_before_the_formula_that_needs_it() {
    let d = scratch("tap-order");
    let l = d.join("brew.log");
    brew(&d, &l, "other/tap", 0);

    let (_, stdout, _) = run(&d, "tap \"pbk_tap/thing\"\nbrew \"pbk_tapped\"\n");

    let lines: Vec<String> = log(&l).lines().map(String::from).collect();
    let tap = lines.iter().position(|x| x == "tap pbk_tap/thing").unwrap();
    let install = lines
        .iter()
        .position(|x| x == "install pbk_tapped")
        .unwrap();
    assert!(tap < install);
    assert!(stdout.contains("adding tap pbk_tap/thing"));
}

#[test]
fn a_tap_already_present_is_not_tapped_again() {
    let d = scratch("tap-present");
    let l = d.join("brew.log");
    brew(&d, &l, "pbk_tap/thing", 0);

    let (_, stdout, _) = run(&d, "tap \"pbk_tap/thing\"\nbrew \"pbk_tapped\"\n");

    assert!(stdout.contains("tap pbk_tap/thing already present"));
    assert!(!log(&l).lines().any(|x| x == "tap pbk_tap/thing"));
}

#[test]
fn a_tapped_formula_checks_path_for_its_last_segment() {
    let d = scratch("qualified");
    let l = d.join("brew.log");
    brew(&d, &l, "", 0);
    stub(&d, "pbk_qualified", "echo ok");

    let (_, stdout, _) = run(&d, "brew \"pbk_tap/thing/pbk_qualified\"\n");

    assert!(stdout.contains("pbk_qualified already installed"));
    assert!(!log(&l).contains("install"));
}

#[test]
fn a_failed_tapped_install_prints_the_trust_hint_and_fails() {
    let d = scratch("trust");
    let l = d.join("brew.log");
    brew(&d, &l, "", 1);

    let (out, _, stderr) = run(&d, "brew \"pbk_tap/thing\"\n");

    assert!(!out.status.success());
    assert!(stderr.contains("brew trust pbk_tap"), "{stderr}");
    assert!(!log(&l).contains("brew trust"), "trust must never be run");
}

#[test]
fn a_missing_brewfile_is_a_clean_no_op() {
    let d = scratch("nofile");

    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["deps", "ensure"])
        .arg(d.join("does-not-exist"))
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();

    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no Brewfile at"));
}
