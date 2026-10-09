// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `repo_slug`: the `<owner>/<repo>` slug for the current git repo's origin
//! remote. Ports the retired shell original. Canonical definition: the memory
//! store keys project facts on this exact string, so every consumer must
//! derive it identically or facts silently fail to resolve (see
//! the retired shell original).

use crate::common::proc::run_with_timeout;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// How long to wait for `git remote get-url origin` before giving up.
/// Matches the retired shell original's `timeout=5`.
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Return the `<owner>/<repo>` slug for the current git repo's origin
/// remote. Empty outside a repo, when no origin remote is configured, or
/// when `git` does not finish within `GIT_TIMEOUT`. Never panics.
pub fn repo_slug() -> String {
    // Keyed by cwd so a process that changes directory never sees a stale slug.
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    let Ok(cwd) = std::env::current_dir() else {
        return spawn_slug();
    };
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&cwd) {
        return hit.clone();
    }
    let slug = spawn_slug();
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(cwd, slug.clone());
    slug
}

fn spawn_slug() -> String {
    // Read `.git/config` directly. A layout the reader does not model falls
    // through to `git`, which also applies URL rewrites.
    if let Some(url) = std::env::current_dir()
        .ok()
        .and_then(|cwd| crate::common::gitfacts::origin_url(&cwd))
    {
        return normalize_remote_url(&url);
    }
    #[cfg(test)]
    SPAWNS.with(|n| n.set(n.get() + 1));
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

#[cfg(test)]
thread_local! {
    static SPAWNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Apply the same normalisation as the retired shell original's sed pipeline:
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

    fn git_in(dir: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?}");
    }

    fn repo_with_origin(tag: &str, url: &str) -> PathBuf {
        let dir = crate::common::test_support::scratch_dir(tag);
        std::fs::create_dir_all(&dir).expect("create repo dir");
        git_in(&dir, &["init", "-q", "-b", "main"]);
        git_in(&dir, &["remote", "add", "origin", url]);
        dir
    }

    #[test]
    fn two_calls_in_one_directory_read_the_disk_and_spawn_nothing() {
        // Arrange
        let _guard = crate::common::test_support::lock_cwd();
        let repo = repo_with_origin("slug-memo", "git@github.com:acme/widgets.git");
        let previous = std::env::current_dir().expect("read cwd");
        std::env::set_current_dir(&repo).expect("cd into repo");
        let before = SPAWNS.with(|n| n.get());

        // Act
        let first = repo_slug();
        let second = repo_slug();
        let spawned = SPAWNS.with(|n| n.get()) - before;

        // Assert
        std::env::set_current_dir(&previous).expect("restore cwd");
        let _ = std::fs::remove_dir_all(&repo);
        assert_eq!(
            (first.as_str(), second.as_str()),
            ("acme/widgets", "acme/widgets")
        );
        assert_eq!(spawned, 0);
    }

    #[test]
    fn a_rewrite_rule_falls_back_to_one_git_spawn_that_applies_it() {
        // Arrange
        let _guard = crate::common::test_support::lock_cwd();
        let repo = repo_with_origin("slug-instead", "gh:acme/widgets");
        git_in(
            &repo,
            &["config", "url.https://github.com/.insteadOf", "gh:"],
        );
        let previous = std::env::current_dir().expect("read cwd");
        std::env::set_current_dir(&repo).expect("cd into repo");
        let before = SPAWNS.with(|n| n.get());

        // Act
        let slug = repo_slug();
        let spawned = SPAWNS.with(|n| n.get()) - before;

        // Assert
        std::env::set_current_dir(&previous).expect("restore cwd");
        let _ = std::fs::remove_dir_all(&repo);
        assert_eq!(slug, "acme/widgets");
        assert_eq!(spawned, 1);
    }

    #[test]
    fn a_different_directory_gets_its_own_slug_not_the_cached_one() {
        // Arrange
        let _guard = crate::common::test_support::lock_cwd();
        let one = repo_with_origin("slug-one", "git@github.com:acme/one.git");
        let two = repo_with_origin("slug-two", "git@github.com:acme/two.git");
        let previous = std::env::current_dir().expect("read cwd");

        // Act
        std::env::set_current_dir(&one).expect("cd one");
        let got_one = repo_slug();
        std::env::set_current_dir(&two).expect("cd two");
        let got_two = repo_slug();

        // Assert
        std::env::set_current_dir(&previous).expect("restore cwd");
        let _ = std::fs::remove_dir_all(&one);
        let _ = std::fs::remove_dir_all(&two);
        assert_eq!(
            (got_one.as_str(), got_two.as_str()),
            ("acme/one", "acme/two")
        );
    }

    /// Asserts the CONTRACT, which holds everywhere, not the shape of the
    /// developer's checkout.
    ///
    /// This previously asserted `!got.is_empty()`, which is true only when the
    /// test happens to run inside a git repo that has an `origin` remote. That
    /// is a property of the environment, not of the code: `repo_slug` documents
    /// empty as the correct result outside a repo or with no origin. The old
    /// form failed in a copied tree, in a clone whose remote is not named
    /// `origin`, and in the `debian:stable-slim` container WU-14 requires, and
    /// it blocked `cargo mutants` outright, since that runs from a copy with no
    /// `.git`.
    #[test]
    fn repo_slug_is_empty_or_a_single_segment_pair() {
        // Arrange, Act
        let got = repo_slug();

        // Assert: either the documented empty result, or exactly `owner/repo`.
        if !got.is_empty() {
            assert_eq!(
                got.matches('/').count(),
                1,
                "a non-empty slug must be exactly owner/repo, got '{got}'"
            );
            assert!(
                !got.contains(char::is_whitespace),
                "a slug must not carry whitespace, got '{got}'"
            );
        }
    }
}
