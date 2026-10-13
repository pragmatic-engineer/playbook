// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Cheap, bounded git lookups. One probe decides whether `cwd` is a repo; the
//! remaining reads then run concurrently, each under its own timeout.

#[derive(Debug, Default, PartialEq, Eq)]
pub struct GitInfo {
    /// Current branch, `detached` when HEAD is not on one.
    pub branch: String,
    /// `origin`'s URL, empty when there is none.
    pub remote_url: String,
    pub dirty: bool,
}

fn git(cwd: &str, args: &[&str]) -> Option<std::process::Output> {
    let mut full = vec!["--no-optional-locks"];
    full.extend_from_slice(args);
    crate::common::git::run(
        Some(std::path::Path::new(cwd)),
        &full,
        crate::common::git::TIMEOUT,
    )
}

fn text(out: Option<std::process::Output>) -> String {
    out.filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim_end_matches('\n')
                .to_string()
        })
        .unwrap_or_default()
}

fn clean(out: Option<std::process::Output>) -> bool {
    out.is_some_and(|o| o.status.success())
}

/// Reads branch, origin URL and dirtiness, or `None` outside a git repo.
///
/// The branch and the origin URL come from `.git` on disk. Dirtiness is the
/// one fact only git can answer, so a single `git status` stays. Any layout
/// the disk reader does not model takes the all-git path.
pub fn inspect(cwd: &str) -> Option<GitInfo> {
    let path = std::path::Path::new(cwd);
    let from_disk =
        crate::common::gitfacts::head(path).zip(crate::common::gitfacts::config_origin_url(path));
    let Some((head, remote_url)) = from_disk else {
        return inspect_with_git(cwd);
    };
    let branch = match head {
        crate::common::gitfacts::Head::Branch(name) => name,
        crate::common::gitfacts::Head::Detached(_) => "detached".to_string(),
    };
    // `-uno`: untracked files never count, like `diff --quiet`. A failed
    // call counts as dirty, as the two `diff` calls did.
    let dirty = git(cwd, &["status", "--porcelain=v1", "-uno"])
        .is_none_or(|o| !o.status.success() || !o.stdout.is_empty());
    Some(GitInfo {
        branch,
        remote_url,
        dirty,
    })
}

/// The same facts, every one from a `git` child process.
fn inspect_with_git(cwd: &str) -> Option<GitInfo> {
    if !clean(git(cwd, &["rev-parse", "--git-dir"])) {
        return None;
    }
    let (branch, remote_url, unstaged_clean, staged_clean) = std::thread::scope(|s| {
        let branch = s.spawn(|| text(git(cwd, &["branch", "--show-current"])));
        let remote = s.spawn(|| text(git(cwd, &["config", "--get", "remote.origin.url"])));
        let unstaged = s.spawn(|| clean(git(cwd, &["diff", "--quiet"])));
        let staged = s.spawn(|| clean(git(cwd, &["diff", "--cached", "--quiet"])));
        (
            branch.join().unwrap_or_default(),
            remote.join().unwrap_or_default(),
            unstaged.join().unwrap_or(false),
            staged.join().unwrap_or(false),
        )
    });
    Some(GitInfo {
        branch: if branch.is_empty() {
            "detached".to_string()
        } else {
            branch
        },
        remote_url,
        dirty: !(unstaged_clean && staged_clean),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::{lock_cwd, scratch_dir};
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    fn run(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .expect("run git");
        assert!(out.status.success(), "git {args:?}");
    }

    fn fixture(tag: &str) -> std::path::PathBuf {
        let dir = scratch_dir(tag);
        fs::create_dir_all(&dir).expect("dir");
        let dir = fs::canonicalize(dir).expect("canonical");
        run(&dir, &["init", "-q", "-b", "main"]);
        dir
    }

    fn commit_file(dir: &Path, name: &str) {
        fs::write(dir.join(name), name).expect("write");
        run(dir, &["add", name]);
        run(dir, &["commit", "-q", "-m", name]);
    }

    /// The disk reader and the all-git path print the same facts.
    fn same(dir: &Path) {
        let cwd = dir.to_str().expect("utf8");
        let disk = inspect(cwd);
        assert_eq!(disk, inspect_with_git(cwd), "{dir:?}");
    }

    fn with_clean_git_env(f: impl FnOnce()) {
        let _guard = lock_cwd();
        let keep = [
            ("GIT_CONFIG_GLOBAL", std::env::var_os("GIT_CONFIG_GLOBAL")),
            ("GIT_CONFIG_SYSTEM", std::env::var_os("GIT_CONFIG_SYSTEM")),
        ];
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");
        f();
        for (k, v) in keep {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }

    #[test]
    fn every_fixture_state_prints_what_git_prints() {
        with_clean_git_env(|| {
            // Fresh repo, no commits, no remote.
            let fresh = fixture("sl-fresh");
            same(&fresh);

            // Clean, with a remote.
            let dir = fixture("sl-states");
            run(
                &dir,
                &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
            );
            commit_file(&dir, "a");
            same(&dir);
            assert_eq!(
                inspect(dir.to_str().expect("utf8")).expect("repo").branch,
                "main"
            );

            // Untracked file only: still clean.
            fs::write(dir.join("untracked"), "x").expect("write");
            same(&dir);
            assert!(!inspect(dir.to_str().expect("utf8")).expect("repo").dirty);

            // Modified tracked file: dirty.
            fs::write(dir.join("a"), "changed").expect("write");
            same(&dir);
            assert!(inspect(dir.to_str().expect("utf8")).expect("repo").dirty);

            // Staged change only: dirty.
            run(&dir, &["add", "a"]);
            same(&dir);
            assert!(inspect(dir.to_str().expect("utf8")).expect("repo").dirty);
            run(&dir, &["commit", "-q", "-m", "second"]);

            // Detached HEAD.
            run(&dir, &["checkout", "-q", "--detach", "HEAD~1"]);
            same(&dir);
            assert_eq!(
                inspect(dir.to_str().expect("utf8")).expect("repo").branch,
                "detached"
            );
            run(&dir, &["checkout", "-q", "main"]);

            // A subdirectory of the repo.
            let sub = dir.join("deep").join("er");
            fs::create_dir_all(&sub).expect("subdir");
            same(&sub);

            // Two origin URLs: `config --get` prints the last one.
            run(
                &dir,
                &[
                    "config",
                    "--add",
                    "remote.origin.url",
                    "https://x.example/a/b",
                ],
            );
            same(&dir);

            // A linked worktree.
            let wt = dir.parent().expect("parent").join("sl-states-wt");
            let _ = fs::remove_dir_all(&wt);
            run(
                &dir,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    "side",
                    wt.to_str().expect("utf8"),
                ],
            );
            same(&fs::canonicalize(&wt).expect("canonical"));
        });
    }

    #[test]
    fn a_non_repo_is_none_on_both_paths() {
        with_clean_git_env(|| {
            let dir = scratch_dir("sl-none");
            fs::create_dir_all(&dir).expect("dir");
            let cwd = dir.to_str().expect("utf8");
            assert_eq!(inspect(cwd), None);
            assert_eq!(inspect_with_git(cwd), None);
        });
    }

    #[test]
    fn a_rewrite_rule_still_gives_the_raw_config_url() {
        with_clean_git_env(|| {
            let dir = fixture("sl-instead");
            run(&dir, &["remote", "add", "origin", "gh:acme/widgets"]);
            run(
                &dir,
                &["config", "url.https://github.com/.insteadOf", "gh:"],
            );
            commit_file(&dir, "a");
            // `config --get` applies no rewrite, but the reader steps aside
            // for rewrite rules anyway; both paths agree either way.
            same(&dir);
        });
    }
}
