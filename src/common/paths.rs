// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Canonical path resolution for the directories playbook itself owns:
//! memory, runtime state, config-drift baselines, per-repo scoped storage.

use crate::common::repo::repo_slug;
use crate::common::session::home_dir;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// `$HOME/.config/playbook`, the root for every directory playbook itself
/// owns. Reuses `session::home_dir()`'s documented symlink/env-var caveat.
pub fn playbook_root() -> PathBuf {
    playbook_root_from(&home_dir())
}

/// Core of `playbook_root`, taking `home` explicitly so tests can assert the
/// shape without touching the real `$HOME` env var.
fn playbook_root_from(home: &Path) -> PathBuf {
    home.join(".config").join("playbook")
}

/// `$HOME/.config/playbook/memory`: saved memory facts and the rebuilt
/// `memory.graph.json`.
pub fn memory_dir() -> PathBuf {
    playbook_root().join("memory")
}

/// `$HOME/.config/playbook/runtime`: per-session state (`session_dir`'s
/// parent).
pub fn runtime_root() -> PathBuf {
    playbook_root().join("runtime")
}

/// `$HOME/.config/playbook/cc-state`: per-project config-drift baselines.
pub fn cc_state_dir() -> PathBuf {
    playbook_root().join("cc-state")
}

/// How long to wait for a `git` invocation before giving up. Wider than
/// `repo_slug`'s 5s since this crate's fully parallel tests flake at 5s.
const GIT_TIMEOUT: Duration = Duration::from_secs(15);

/// A stable identity for the CURRENT git worktree: `cc::project_slug`'s
/// slugifier applied to `git rev-parse --show-toplevel`, not the raw cwd.
pub fn worktree_id() -> String {
    worktree_id_at(&std::env::current_dir().unwrap_or_default())
}

/// Core of `worktree_id`, taking the starting directory explicitly so tests
/// can point it at a real scratch worktree instead of mutating cwd.
fn worktree_id_at(dir: &Path) -> String {
    slugify(&git_toplevel(dir))
}

/// `git -C <dir> rev-parse --show-toplevel`, trimmed. Empty on any failure,
/// including a timeout. Never panics.
fn git_toplevel(dir: &Path) -> String {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(["--no-optional-locks", "rev-parse", "--show-toplevel"]);
    let Some(output) = crate::common::proc::run_with_timeout(&mut command, GIT_TIMEOUT) else {
        return String::new();
    };
    if !output.status.success() {
        return String::new();
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Every non-alphanumeric character becomes `-`. Duplicates
/// `cc::project_slug`'s rule since `common` sits below `cc` in this layering.
fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Which tier of a repo's scoped storage a path resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoScope {
    /// `repos/<owner>/<repo>/.config/`: shared across every worktree.
    /// Ships unpopulated; nothing writes here yet.
    Config,
    /// `repos/<owner>/<repo>/<worktree-id>/`: scoped to one worktree, so
    /// two worktrees of the same repo never collide.
    Worktree,
}

/// Resolve `kind`'s directory for the CURRENT repo, under
/// `playbook_root()/repos/<owner>/<repo>/...`. `None` on any git failure.
pub fn repo_scoped_dir(kind: RepoScope) -> Option<PathBuf> {
    let slug = repo_slug();
    let (owner, repo) = slug.split_once('/')?;
    let id = worktree_id();
    if id.is_empty() {
        return None;
    }
    let base = playbook_root().join("repos").join(owner).join(repo);
    Some(match kind {
        RepoScope::Config => base.join(".config"),
        RepoScope::Worktree => base.join(id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;
    use std::process::Output;

    #[test]
    fn playbook_root_is_config_playbook_under_home() {
        // Arrange
        let home = Path::new("/scratch-home-for-test");

        // Act
        let got = playbook_root_from(home);

        // Assert
        assert_eq!(got, home.join(".config").join("playbook"));
    }

    #[test]
    fn memory_dir_runtime_root_and_cc_state_dir_sit_flat_under_playbook_root() {
        // Arrange, Act
        let root = playbook_root();

        // Assert: each is a direct child of playbook_root().
        assert_eq!(memory_dir(), root.join("memory"));
        assert_eq!(runtime_root(), root.join("runtime"));
        assert_eq!(cc_state_dir(), root.join("cc-state"));
    }

    /// Isolated from the host's own git config, matching
    /// `tests/cc_worktree.rs`'s `git`/`git_ok` helpers.
    fn git(repo_path: &Path, args: &[&str]) -> Output {
        Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .expect("git command should spawn")
    }

    fn git_ok(repo_path: &Path, args: &[&str]) {
        let out = git(repo_path, args);
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A real repo with one commit on `main`, canonicalized: macOS resolves
    /// `/tmp` through `/private/tmp`, and `git` reports the resolved path.
    fn seeded_repo(tag: &str) -> PathBuf {
        let dir = scratch_dir(tag);
        fs::create_dir_all(&dir).expect("create scratch repo dir");
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "T"],
        ] {
            git_ok(&dir, &args);
        }
        fs::write(dir.join("README.md"), "seed\n").expect("write seed file");
        git_ok(&dir, &["add", "."]);
        git_ok(&dir, &["commit", "-q", "-m", "seed"]);
        dir.canonicalize().expect("seeded repo should resolve")
    }

    /// Adds a real linked worktree on a new branch, returning its
    /// canonical path.
    fn add_worktree(repo_root: &Path, branch: &str) -> PathBuf {
        let dest = repo_root.join(format!("wt-{branch}"));
        git_ok(
            repo_root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                branch,
                dest.to_str().expect("utf8 path"),
            ],
        );
        dest.canonicalize().expect("worktree should resolve")
    }

    #[test]
    fn worktree_id_stable_regardless_of_cwd_depth_within_one_worktree() {
        // Arrange
        let repo = seeded_repo("wtid-stable");
        let nested = repo.join("a").join("b");
        fs::create_dir_all(&nested).expect("nested dir");

        // Act
        let at_top = worktree_id_at(&repo);
        let at_depth = worktree_id_at(&nested);

        // Assert
        assert!(!at_top.is_empty());
        assert_eq!(at_top, at_depth);

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn worktree_id_differs_between_two_worktrees_of_the_same_repo() {
        // Arrange: a real `git worktree add`, not a mocked toplevel.
        let repo = seeded_repo("wtid-differ");
        let worktree = add_worktree(&repo, "feature");

        // Act
        let id_repo = worktree_id_at(&repo);
        let id_worktree = worktree_id_at(&worktree);

        // Assert
        assert!(!id_repo.is_empty());
        assert!(!id_worktree.is_empty());
        assert_ne!(id_repo, id_worktree);

        let _ = fs::remove_dir_all(&repo);
    }

    /// The only tests in this binary that mutate the process cwd, since
    /// `repo_slug()` has no dir param to inject. Guarded: cwd is process-global.
    static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn lock_cwd() -> std::sync::MutexGuard<'static, ()> {
        CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn repo_scoped_dir_none_outside_a_git_repo() {
        // Arrange: a scratch dir with no `.git` anywhere in its ancestry.
        let _guard = lock_cwd();
        let outside = scratch_dir("repo-scoped-outside");
        fs::create_dir_all(&outside).expect("create scratch dir");
        let previous = std::env::current_dir().expect("read current dir");
        std::env::set_current_dir(&outside).expect("cd into scratch dir");

        // Act
        let got = repo_scoped_dir(RepoScope::Worktree);

        // Assert
        std::env::set_current_dir(&previous).expect("restore cwd");
        let _ = fs::remove_dir_all(&outside);
        assert_eq!(got, None);
    }

    #[test]
    fn repo_scoped_dir_none_with_no_origin_remote() {
        // Arrange: a real git repo with no `origin` remote configured.
        let _guard = lock_cwd();
        let repo = scratch_dir("repo-scoped-no-origin");
        fs::create_dir_all(&repo).expect("create scratch repo dir");
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "T"],
        ] {
            git_ok(&repo, &args);
        }
        let previous = std::env::current_dir().expect("read current dir");
        std::env::set_current_dir(&repo).expect("cd into scratch repo");

        // Act
        let got = repo_scoped_dir(RepoScope::Config);

        // Assert
        std::env::set_current_dir(&previous).expect("restore cwd");
        let _ = fs::remove_dir_all(&repo);
        assert_eq!(got, None);
    }
}
