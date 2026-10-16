// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `install.sh --pbtop` installs pbtop alone from the
//! `pbtop-<version>-<target>.tar.gz` asset, checked like the playbook binary:
//! its SHA256SUMS line, its build attestation and its `--version`.

#[path = "support/shell_fixture.rs"]
mod fx;

use fx::{both, install_stubs, repo_root, sha256_hex, write_exec, Work};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const RELEASE_BODY: &str = r#"{"tag_name": "v1.2.3"}"#;
const ASSET: &str = "pbtop-1.2.3-x86_64-unknown-linux-musl.tar.gz";

/// A release asset at `<work>/asset/<ASSET>` holding a `pbtop` that prints
/// `version`, and its SHA256SUMS line.
fn asset(work: &Work, version: &str) -> (PathBuf, String) {
    let src = work.dir("asset-src");
    write_exec(
        &src.join("pbtop"),
        &format!("#!/usr/bin/env bash\necho \"pbtop {version}\"\n"),
    );
    let dir = work.dir("asset");
    let tarball = dir.join(ASSET);
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&tarball)
        .arg("-C")
        .arg(&src)
        .arg("pbtop")
        .status()
        .expect("tar");
    assert!(status.success());
    let bytes = fs::read(&tarball).expect("tarball");
    (tarball, format!("{}  {ASSET}\n", sha256_hex(&bytes)))
}

struct Run {
    out: Output,
    bindir: PathBuf,
    gh_log: String,
    curl_log: String,
}

fn install(work: &Work, tarball: &Path, sums: &str) -> Run {
    let stubs = work.dir("stubs");
    install_stubs(&stubs);
    let home = work.dir("home");
    let bindir = home.join(".local/bin");
    let gh_log = work.path("gh.log");
    let curl_log = work.path("curl.log");
    let out = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .arg("--pbtop")
        .env(
            "PATH",
            format!(
                "{}:{}",
                stubs.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("HOME", &home)
        .env("CLAUDE_HOME", home.join(".claude"))
        .env("PLAYBOOK_BIN_DIR", &bindir)
        .env("SHELL", "/bin/bash")
        .env("STUB_CODE", "200")
        .env("STUB_BODY", RELEASE_BODY)
        .env("STUB_ASSET_FILE", tarball)
        .env("STUB_SUMS_BODY", sums)
        .env("STUB_GH", "pass")
        .env("GH_LOG", &gh_log)
        .env("CURL_LOG", &curl_log)
        .env_remove("PLAYBOOK_REF")
        .output()
        .expect("bash spawns");
    Run {
        out,
        bindir,
        gh_log: fs::read_to_string(gh_log).unwrap_or_default(),
        curl_log: fs::read_to_string(curl_log).unwrap_or_default(),
    }
}

#[test]
fn pbtop_alone_is_installed_from_its_own_verified_asset() {
    // Arrange
    let work = Work::new("pbtop-ok");
    let (tarball, sums) = asset(&work, "1.2.3");

    // Act
    let run = install(&work, &tarball, &sums);

    // Assert
    let text = both(&run.out);
    assert!(run.out.status.success(), "{text}");
    assert!(text.contains("Installed pbtop 1.2.3"), "{text}");
    let version = Command::new(run.bindir.join("pbtop"))
        .arg("--version")
        .output()
        .expect("pbtop runs");
    assert_eq!(String::from_utf8_lossy(&version.stdout), "pbtop 1.2.3\n");
    assert!(
        !run.bindir.join("playbook").exists(),
        "only pbtop is installed"
    );
    assert!(
        run.curl_log
            .contains(&format!("/releases/download/v1.2.3/{ASSET}")),
        "{}",
        run.curl_log
    );
    assert!(
        run.gh_log.contains("attestation verify") && run.gh_log.contains(ASSET),
        "{}",
        run.gh_log
    );
    assert!(!text.contains("playbook setup"), "{text}");
}

#[test]
fn a_pbtop_asset_that_does_not_match_its_checksum_is_not_installed() {
    // Arrange
    let work = Work::new("pbtop-sum");
    let (tarball, _) = asset(&work, "1.2.3");
    let wrong = format!("{}  {ASSET}\n", "0".repeat(64));

    // Act
    let run = install(&work, &tarball, &wrong);

    // Assert
    let text = both(&run.out);
    assert!(!run.out.status.success(), "{text}");
    assert!(text.contains("checksum mismatch for"), "{text}");
    assert!(!run.bindir.join("pbtop").exists());
}

#[test]
fn a_pbtop_that_reports_another_version_is_not_installed() {
    // Arrange
    let work = Work::new("pbtop-ver");
    let (tarball, sums) = asset(&work, "9.9.9");

    // Act
    let run = install(&work, &tarball, &sums);

    // Assert
    let text = both(&run.out);
    assert!(!run.out.status.success(), "{text}");
    assert!(
        text.contains("version mismatch: the downloaded pbtop reports 'pbtop 9.9.9'"),
        "{text}"
    );
    assert!(!run.bindir.join("pbtop").exists());
}
