// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `repo_slug`: the `<owner>/<repo>` slug for the current git repo's origin
//! remote. Ports hooks/lib/common.py:243. Canonical definition: the memory
//! store keys project facts on this exact string, so every consumer must
//! derive it identically or facts silently fail to resolve (see
//! hooks/lib/common.sh:140-146).

use crate::common::proc::run_with_timeout;
use std::process::Command;
use std::time::Duration;

/// How long to wait for `git remote get-url origin` before giving up.
/// Matches hooks/lib/common.py:252's `timeout=5`.
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Return the `<owner>/<repo>` slug for the current git repo's origin
/// remote. Empty outside a repo, when no origin remote is configured, or
/// when `git` does not finish within `GIT_TIMEOUT`. Never panics.
pub fn repo_slug() -> String {
    let mut command = Command::new("git");
    command.args(["--no-optional-locks", "remote", "get-url", "origin"]);
    let Some(output) = run_with_timeout(&mut command, GIT_TIMEOUT) else {
        return String::new();
    };
    if !output.status.success() {
        return String::new();
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    normalize_remote_url(&url)
}

/// Apply the same normalisation as hooks/lib/common.sh:149's sed pipeline:
/// strip a trailing `.git` (or `.git/`), a leading scheme (`https://`,
/// `ssh://`, ...), a leading `user@`, then a leading host up to its first
/// `/` or `:`.
fn normalize_remote_url(url: &str) -> String {
    let mut s = url;

    if let Some(stripped) = s.strip_suffix(".git/") {
        s = stripped;
    } else if let Some(stripped) = s.strip_suffix(".git") {
        s = stripped;
    }

    if let Some(idx) = s.find("://") {
        let scheme = &s[..idx];
        if !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphabetic()) {
            s = &s[idx + 3..];
        }
    }

    if let Some(idx) = s.find('@') {
        let user = &s[..idx];
        if !user.is_empty() && !user.contains('/') {
            s = &s[idx + 1..];
        }
    }

    if let Some(idx) = s.find(['/', ':']) {
        if idx > 0 {
            s = &s[idx + 1..];
        }
    }

    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_https_url_with_git_suffix() {
        // Arrange
        let url = "https://github.com/owner/repo.git";

        // Act
        let got = normalize_remote_url(url);

        // Assert
        assert_eq!(got, "owner/repo");
    }

    #[test]
    fn normalizes_ssh_shorthand_url() {
        // Arrange
        let url = "git@github.com:owner/repo.git";

        // Act
        let got = normalize_remote_url(url);

        // Assert
        assert_eq!(got, "owner/repo");
    }

    #[test]
    fn normalizes_ssh_scheme_url_with_trailing_slash() {
        // Arrange
        let url = "ssh://git@github.com/owner/repo.git/";

        // Act
        let got = normalize_remote_url(url);

        // Assert
        assert_eq!(got, "owner/repo");
    }

    #[test]
    fn leaves_url_without_git_suffix_unchanged_in_shape() {
        // Arrange
        let url = "https://github.com/owner/repo";

        // Act
        let got = normalize_remote_url(url);

        // Assert
        assert_eq!(got, "owner/repo");
    }

    #[test]
    fn repo_slug_returns_owner_repo_format_in_this_checkout() {
        // Arrange, Act
        let got = repo_slug();

        // Assert: this test runs inside the playbook git checkout, which has
        // an origin remote, matching the same loose contract
        // hooks/lib/common.test.sh's repo_slug case asserts.
        assert!(
            !got.is_empty() && got.contains('/'),
            "expected owner/repo format, got '{got}'"
        );
    }
}
