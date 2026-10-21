// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `install.sh` installs the pbdoc helper next to playbook by default, from the
//! `pbdoc-<version>-<target>.tar.gz` asset of the same release, checked like
//! the playbook binary. A release without the asset still installs playbook.

#[path = "support/shell_fixture.rs"]
mod fx;

use fx::{both, install_stubs, repo_root, sha256_hex, write_exec, Work};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

const RELEASE_BODY: &str = r#"{"tag_name": "v1.2.3"}"#;
const PLAYBOOK_ASSET: &str = "playbook-1.2.3-x86_64-unknown-linux-musl";
const PBDOC_ASSET: &str = "pbdoc-1.2.3-x86_64-unknown-linux-musl.tar.gz";

/// Serves the playbook binary and the pbdoc tarball from their own files, and
/// defers every other URL to the shared curl stub. A pbdoc URL fails like a
/// 404 when no tarball is set.
const CURL_WRAPPER: &str = r#"#!/usr/bin/env bash
out=""; url=""
for a in "$@"; do
  if [ "$prev" = "-o" ]; then out="$a"; fi
  case "$a" in http*) url="$a" ;; esac
  prev="$a"
done
case "$url" in
  */releases/download/*/playbook-*)
    cp "$STUB_PLAYBOOK_FILE" "$out"; exit 0 ;;
  */releases/download/*/pbdoc-*)
    [ -n "${STUB_PBDOC_FILE:-}" ] || exit 22
    cp "$STUB_PBDOC_FILE" "$out"; exit 0 ;;
esac
exec "$STUB_REAL_CURL" "$@"
"#;

fn sums_line(bytes: &[u8], name: &str) -> String {
    format!("{}  {name}\n", sha256_hex(bytes))
}

struct Run {
    out: Output,
    bindir: PathBuf,
}

/// Runs the real `install.sh --no-init` against a fake release. `pbdoc` is the
/// tarball and its SHA256SUMS line, or `None` for a release without the asset.
fn install(work: &Work, pbdoc: Option<(PathBuf, String)>) -> Run {
    let stubs = work.dir("stubs");
    install_stubs(&stubs);
    let wrap = work.dir("wrap");
    write_exec(&wrap.join("curl"), CURL_WRAPPER);

    let playbook = work.path("playbook-asset");
    write_exec(&playbook, "#!/usr/bin/env bash\necho \"playbook 1.2.3\"\n");
    let mut sums = sums_line(&fs::read(&playbook).expect("playbook"), PLAYBOOK_ASSET);

    let home = work.dir("home");
    let bindir = home.join(".local/bin");
    let mut cmd = Command::new("bash");
    cmd.arg(repo_root().join("install.sh")).arg("--no-init");
    if let Some((tarball, line)) = pbdoc {
        sums.push_str(&line);
        cmd.env("STUB_PBDOC_FILE", tarball);
    } else {
        cmd.env_remove("STUB_PBDOC_FILE");
    }
    let out = cmd
        .env(
            "PATH",
            format!(
                "{}:{}:{}",
                wrap.display(),
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
        .env("STUB_SUMS_BODY", sums)
        .env("STUB_PLAYBOOK_FILE", &playbook)
        .env("STUB_REAL_CURL", stubs.join("curl"))
        .env("STUB_GH", "pass")
        .env_remove("PLAYBOOK_REF")
        .env_remove("PLAYBOOK_INSTALL_ONLY")
        .output()
        .expect("bash spawns");
    Run { out, bindir }
}

/// A pbdoc tarball printing `version`, and its SHA256SUMS line.
fn pbdoc_asset(work: &Work, version: &str) -> (PathBuf, String) {
    let src = work.dir("pbdoc-src");
    write_exec(
        &src.join("pbdoc"),
        &format!("#!/usr/bin/env bash\necho \"pbdoc {version}\"\n"),
    );
    fs::write(src.join("NOTICE-pbdoc"), "notice\n").expect("notice");
    let tarball = work.dir("pbdoc-asset").join(PBDOC_ASSET);
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&tarball)
        .arg("-C")
        .arg(&src)
        .args(["pbdoc", "NOTICE-pbdoc"])
        .status()
        .expect("tar");
    assert!(status.success());
    let line = sums_line(&fs::read(&tarball).expect("tarball"), PBDOC_ASSET);
    (tarball, line)
}

#[test]
fn pbdoc_is_installed_next_to_playbook_from_its_verified_asset() {
    // Arrange
    let work = Work::new("pbdoc-ok");
    let pbdoc = pbdoc_asset(&work, "1.2.3");

    // Act
    let run = install(&work, Some(pbdoc));

    // Assert
    let text = both(&run.out);
    assert!(run.out.status.success(), "{text}");
    assert!(run.bindir.join("playbook").exists(), "{text}");
    assert!(text.contains("Installed pbdoc 1.2.3"), "{text}");
    let version = Command::new(run.bindir.join("pbdoc"))
        .arg("--version")
        .output()
        .expect("pbdoc runs");
    assert_eq!(String::from_utf8_lossy(&version.stdout), "pbdoc 1.2.3\n");
}

#[test]
fn a_release_without_a_pbdoc_asset_warns_and_still_installs_playbook() {
    // Arrange
    let work = Work::new("pbdoc-absent");

    // Act
    let run = install(&work, None);

    // Assert
    let text = both(&run.out);
    assert!(run.out.status.success(), "{text}");
    assert!(text.contains("warning:"), "{text}");
    assert!(text.contains(PBDOC_ASSET), "{text}");
    assert!(run.bindir.join("playbook").exists(), "{text}");
    assert!(!run.bindir.join("pbdoc").exists());
}

#[test]
fn a_pbdoc_asset_with_a_wrong_checksum_fails_and_installs_nothing() {
    // Arrange
    let work = Work::new("pbdoc-sum");
    let (tarball, _) = pbdoc_asset(&work, "1.2.3");
    let wrong = format!("{}  {PBDOC_ASSET}\n", "0".repeat(64));

    // Act
    let run = install(&work, Some((tarball, wrong)));

    // Assert
    let text = both(&run.out);
    assert!(!run.out.status.success(), "{text}");
    assert!(text.contains("checksum mismatch for"), "{text}");
    assert!(!run.bindir.join("pbdoc").exists());
    assert!(!run.bindir.join("playbook").exists());
}

/// A pbdoc tarball holding `names` from a source folder `prepare` fills.
fn custom_asset(
    work: &Work,
    prepare: impl FnOnce(&std::path::Path),
    names: &[&str],
) -> (PathBuf, String) {
    let src = work.dir("pbdoc-custom-src");
    prepare(&src);
    let tarball = work.dir("pbdoc-custom-asset").join(PBDOC_ASSET);
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&tarball)
        .arg("-C")
        .arg(&src)
        .args(names)
        .status()
        .expect("tar");
    assert!(status.success());
    let line = sums_line(&fs::read(&tarball).expect("tarball"), PBDOC_ASSET);
    (tarball, line)
}

#[test]
fn a_pbdoc_that_is_a_symlink_fails_and_installs_nothing() {
    // Arrange
    let work = Work::new("pbdoc-symlink");
    let asset = custom_asset(
        &work,
        |src| std::os::unix::fs::symlink("/bin/sh", src.join("pbdoc")).expect("symlink"),
        &["pbdoc"],
    );

    // Act
    let run = install(&work, Some(asset));

    // Assert
    let text = both(&run.out);
    assert!(!run.out.status.success(), "{text}");
    assert!(text.contains("is not a regular file"), "{text}");
    assert!(!run.bindir.join("pbdoc").exists());
}

#[test]
fn a_tarball_without_a_pbdoc_entry_fails_and_installs_nothing() {
    // Arrange
    let work = Work::new("pbdoc-missing-entry");
    let asset = custom_asset(
        &work,
        |src| fs::write(src.join("NOTICE-pbdoc"), "notice\n").expect("notice"),
        &["NOTICE-pbdoc"],
    );

    // Act
    let run = install(&work, Some(asset));

    // Assert
    let text = both(&run.out);
    assert!(!run.out.status.success(), "{text}");
    assert!(text.contains("holds no pbdoc"), "{text}");
    assert!(!run.bindir.join("pbdoc").exists());
}

#[test]
fn a_pbdoc_that_reports_another_version_fails_and_installs_nothing() {
    // Arrange
    let work = Work::new("pbdoc-version");
    let asset = pbdoc_asset(&work, "9.9.9");

    // Act
    let run = install(&work, Some(asset));

    // Assert
    let text = both(&run.out);
    assert!(!run.out.status.success(), "{text}");
    assert!(text.contains("version mismatch"), "{text}");
    assert!(!run.bindir.join("pbdoc").exists());
}
