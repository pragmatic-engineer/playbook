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
    pack(work, &src)
}

/// Packs the `pbtop` entry of `src` (a file, a link or a folder) into the
/// release asset, and returns it with its SHA256SUMS line.
fn pack(work: &Work, src: &Path) -> (PathBuf, String) {
    let dir = work.dir("asset");
    let tarball = dir.join(ASSET);
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&tarball)
        .arg("-C")
        .arg(src)
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
    install_from(work, tarball, sums, RELEASE_BODY)
}

fn install_from(work: &Work, tarball: &Path, sums: &str, release: &str) -> Run {
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
        .env("STUB_BODY", release)
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
    assert!(text.contains("pbtop is installed to"), "{text}");
    let bashrc = fs::read_to_string(work.path("home/.bashrc")).unwrap_or_default();
    assert!(
        bashrc.contains(&run.bindir.display().to_string()),
        "the bin folder is put on PATH: {bashrc}"
    );
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

#[test]
fn a_release_before_pbtop_had_its_own_asset_is_refused_before_any_download() {
    // Arrange
    let work = Work::new("pbtop-old");
    let (tarball, sums) = asset(&work, "0.34.0");

    // Act
    let run = install_from(&work, &tarball, &sums, r#"{"tag_name": "v0.34.0"}"#);

    // Assert
    let text = both(&run.out);
    assert!(!run.out.status.success(), "{text}");
    assert!(
        text.contains("v0.34.0 has no pbtop asset; pbtop alone ships from playbook 0.35.0 on"),
        "{text}"
    );
    assert!(
        !run.curl_log.contains("/releases/download/"),
        "{}",
        run.curl_log
    );
    assert!(!run.bindir.join("pbtop").exists());
}

#[test]
fn an_asset_without_a_regular_pbtop_is_not_installed() {
    // Arrange
    let linked_work = Work::new("pbtop-link");
    let src = linked_work.dir("asset-src");
    write_exec(&src.join("other"), "#!/usr/bin/env bash\n");
    std::os::unix::fs::symlink("other", src.join("pbtop")).expect("symlink");
    let (linked, linked_sums) = pack(&linked_work, &src);
    let folder_work = Work::new("pbtop-folder");
    let src = folder_work.dir("asset-src");
    fs::create_dir_all(src.join("pbtop")).expect("a folder named pbtop");
    let (folder, folder_sums) = pack(&folder_work, &src);

    // Act
    let link_run = install(&linked_work, &linked, &linked_sums);
    let folder_run = install(&folder_work, &folder, &folder_sums);

    // Assert
    for run in [&link_run, &folder_run] {
        let text = both(&run.out);
        assert!(!run.out.status.success(), "{text}");
        assert!(text.contains("is not a regular file"), "{text}");
        assert!(!run.bindir.join("pbtop").exists());
    }
}
