// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integrity and provenance: SHA256SUMS, then the build attestation with the
//! same policy as install.sh (lenient by default, strict with
//! `PLAYBOOK_REQUIRE_ATTESTATION=1`).

use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::{Command, Stdio};

pub const REPO: &str = "pragmatic-engineer/playbook";

/// Lowercase hex SHA256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Checks `bytes` against the `<hash>  <asset>` line of a SHA256SUMS file.
pub fn verify_checksum(bytes: &[u8], sums: &str, asset: &str) -> Result<(), String> {
    let expected = sums
        .lines()
        .filter_map(|line| line.split_once("  "))
        .find(|(_, name)| name.trim() == asset)
        .map(|(hash, _)| hash.trim().to_ascii_lowercase())
        .ok_or_else(|| {
            format!("no checksum line for {asset} in SHA256SUMS; the release may be incomplete")
        })?;
    let actual = sha256_hex(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(format!("checksum mismatch for {asset}: expected {expected}, got {actual}; nothing was installed"))
    }
}

/// Provenance check seam. `Ok(None)` is verified, `Ok(Some(w))` is a lenient
/// pass with a warning to show, `Err` aborts the update.
pub trait Attest {
    fn verify(&self, file: &Path, strict: bool) -> Result<Option<String>, String>;
}

/// Shells out to `gh attestation verify`, pinned to release.yml.
pub struct GhAttest;

/// Failure texts that mean "could not check", not "checked and wrong".
fn is_gap(err: &str) -> bool {
    let err = err.to_ascii_lowercase();
    [
        "no attestations found",
        "failed to fetch",
        "gh auth login",
        "gh_token",
        "http 401",
    ]
    .iter()
    .any(|needle| err.contains(needle))
}

impl Attest for GhAttest {
    fn verify(&self, file: &Path, strict: bool) -> Result<Option<String>, String> {
        let has_gh = Command::new("gh")
            .args(["attestation", "--help"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !has_gh {
            return if strict {
                Err("PLAYBOOK_REQUIRE_ATTESTATION=1 but gh with 'attestation' support is not available".into())
            } else {
                Ok(Some("checksum only, provenance not verified: install gh to check the build attestation".into()))
            };
        }
        let output = Command::new("gh")
            .args(["attestation", "verify"])
            .arg(file)
            .args(["--repo", REPO, "--signer-workflow"])
            .arg(format!("{REPO}/.github/workflows/release.yml"))
            .stdin(Stdio::null())
            .output()
            .map_err(|err| format!("could not run gh: {err}"))?;
        if output.status.success() {
            return Ok(None);
        }
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        if !strict && is_gap(&text) {
            let first = text.lines().next().unwrap_or("").trim();
            return Ok(Some(format!(
                "checksum only, provenance not verified: {first}"
            )));
        }
        Err(format!("attestation verification failed: {}", text.trim()))
    }
}

#[cfg(test)]
pub mod fake {
    use super::Attest;
    use std::path::Path;

    use std::cell::RefCell;

    /// An attestation result fixed by the test, recording each `strict` it saw.
    pub struct FakeAttest(pub Result<Option<String>, String>, pub RefCell<Vec<bool>>);

    impl FakeAttest {
        pub fn new(result: Result<Option<String>, String>) -> Self {
            FakeAttest(result, RefCell::new(Vec::new()))
        }
    }

    impl Attest for FakeAttest {
        fn verify(&self, _: &Path, strict: bool) -> Result<Option<String>, String> {
            self.1.borrow_mut().push(strict);
            self.0.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_known_empty_digest() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_matching_checksum_passes_and_a_mismatch_aborts() {
        let sums = format!(
            "{}  playbook-1-x\n{}  other\n",
            sha256_hex(b"good"),
            sha256_hex(b"zzz")
        );
        assert!(verify_checksum(b"good", &sums, "playbook-1-x").is_ok());
        let err = verify_checksum(b"evil", &sums, "playbook-1-x").unwrap_err();
        assert!(err.contains("checksum mismatch"), "{err}");
    }

    #[test]
    fn a_missing_checksum_line_aborts() {
        assert!(verify_checksum(b"x", "abc  other\n", "playbook-1-x").is_err());
    }

    #[test]
    fn gap_detection_matches_the_installer_patterns() {
        assert!(is_gap("Error: no attestations found for subject"));
        assert!(!is_gap("verification failed: signer mismatch"));
    }
}
