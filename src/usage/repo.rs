// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Maps a transcript's working directory to the real repository name, so a
//! worktree or an agent folder counts toward its repo instead of showing up
//! as one of its own. Order: path markers (work for deleted directories),
//! then git, then the directory's own name.

use crate::common::run_with_timeout;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const GIT_TIMEOUT: Duration = Duration::from_secs(5);
const CLAUDE_WORKTREES: &str = "/.claude/worktrees/";
const GIT_DIR: &str = "/.git/";
const GIT_DIR_END: &str = "/.git";
const PLAYBOOK_REPOS: &str = "/.config/playbook/repos/";

/// Resolves many working directories, asking git once per distinct one.
#[derive(Default)]
pub struct RepoResolver {
    cache: HashMap<String, String>,
}

impl RepoResolver {
    pub fn resolve(&mut self, cwd: &str) -> String {
        if let Some(found) = self.cache.get(cwd) {
            return found.clone();
        }
        let repo = resolve_uncached(cwd);
        self.cache.insert(cwd.to_string(), repo.clone());
        repo
    }
}

fn resolve_uncached(cwd: &str) -> String {
    if cwd.is_empty() {
        return String::new();
    }
    repo_from_markers(cwd)
        .or_else(|| repo_from_git(Path::new(cwd)))
        .unwrap_or_else(|| last_segment(cwd))
}

fn last_segment(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string()
}

/// Rules that need no filesystem, so they also work once a worktree is gone:
/// a `.claude/worktrees/<name>` or `.git/<anything>` path belongs to the repo
/// above it, and playbook's own `repos/<owner>/<repo>/...` storage names it.
pub fn repo_from_markers(cwd: &str) -> Option<String> {
    for marker in [CLAUDE_WORKTREES, GIT_DIR] {
        if let Some((root, _)) = cwd.split_once(marker) {
            return named(last_segment(root));
        }
    }
    if let Some(root) = cwd.strip_suffix(GIT_DIR_END) {
        return named(last_segment(root));
    }
    let (_, rest) = cwd.split_once(PLAYBOOK_REPOS)?;
    let mut parts = rest.split('/');
    let _owner = parts.next()?;
    named(parts.next()?.to_string())
}

fn named(name: String) -> Option<String> {
    (!name.is_empty()).then_some(name)
}

/// Asks git from the directory, or from its nearest existing parent when the
/// directory itself is gone. The common dir is shared by every worktree.
fn repo_from_git(cwd: &Path) -> Option<String> {
    let dir = nearest_existing(cwd)?;
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(&dir)
        .args(["rev-parse", "--git-common-dir"]);
    let out = run_with_timeout(&mut command, GIT_TIMEOUT)?;
    if !out.status.success() {
        return None;
    }
    let printed = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if printed.is_empty() {
        return None;
    }
    // From a subfolder git prints a relative path like `../../.git`; resolve
    // it so the `..` parts do not hide the repo's folder name.
    let joined = dir.join(printed);
    let common = joined.canonicalize().unwrap_or(joined);
    name_from_common_dir(&common)
}

fn nearest_existing(path: &Path) -> Option<std::path::PathBuf> {
    path.ancestors().find(|p| p.is_dir()).map(Path::to_path_buf)
}

/// `/work/proj/.git` is `proj`; a bare `/srv/proj.git` is `proj`.
fn name_from_common_dir(common: &Path) -> Option<String> {
    let file = common.file_name()?.to_str()?;
    if file == ".git" {
        return named(common.parent()?.file_name()?.to_str()?.to_string());
    }
    named(file.strip_suffix(".git").unwrap_or(file).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .expect("git runs");
        assert!(status.status.success(), "git {args:?} failed");
    }

    #[test]
    fn markers_name_the_repo_for_deleted_worktrees() {
        let cases = [
            ("/w/org/playbook/.claude/worktrees/agent-a165f", "playbook"),
            ("/w/org/ward/.git/review-worktrees/154-52560ea", "ward"),
            ("/w/org/ward/.git/review-worktrees/12-a/worker", "ward"),
            ("/w/org/ward/.git", "ward"),
            ("/h/.config/playbook/repos/pe/playbook/wt-1/src", "playbook"),
            ("/w/org/x/.claude/worktrees/enterprise-hardening/wu-1", "x"),
        ];
        for (cwd, want) in cases {
            assert_eq!(repo_from_markers(cwd).as_deref(), Some(want), "{cwd}");
        }
        assert_eq!(repo_from_markers("/w/org/plain"), None);
    }

    #[test]
    fn a_plain_repo_a_subfolder_and_a_linked_worktree_share_one_name() {
        let base = scratch_dir("usage-repo-git");
        let repo = base.join("proj");
        fs::create_dir_all(repo.join("crates/inner")).unwrap();
        git(&repo, &["init", "--quiet"]);
        git(&repo, &["commit", "--allow-empty", "--quiet", "-m", "x"]);
        let linked = base.join("linked-checkout");
        git(
            &repo,
            &["worktree", "add", "--quiet", linked.to_str().unwrap()],
        );
        let mut resolver = RepoResolver::default();

        let root = resolver.resolve(repo.to_str().unwrap());
        let sub = resolver.resolve(repo.join("crates/inner").to_str().unwrap());
        let wt = resolver.resolve(linked.to_str().unwrap());

        assert_eq!(
            (root.as_str(), sub.as_str(), wt.as_str()),
            ("proj", "proj", "proj")
        );
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn a_deleted_directory_uses_its_nearest_existing_parent_repo() {
        let base = scratch_dir("usage-repo-deleted");
        let repo = base.join("proj");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        let mut resolver = RepoResolver::default();

        let got = resolver.resolve(repo.join(".scratch/gone").to_str().unwrap());

        assert_eq!(got, "proj");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn a_path_outside_any_repo_falls_back_to_its_own_name() {
        let base = scratch_dir("usage-repo-none");
        let plain = base.join("not-a-repo");
        fs::create_dir_all(&plain).unwrap();
        let mut resolver = RepoResolver::default();

        assert_eq!(resolver.resolve(plain.to_str().unwrap()), "not-a-repo");
        assert_eq!(resolver.resolve("/no/such/place/thing"), "thing");
        assert_eq!(resolver.resolve(""), "");
        let _ = fs::remove_dir_all(base);
    }
}
