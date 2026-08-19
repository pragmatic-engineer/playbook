// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports the decision and path logic of `shell/shared/worktree.sh`.
//!
//! First slice of WU-17. The git-driving half (worktree creation, upstream
//! setup, stale cleanup, rebase recovery) and the orchestration follow
//! separately, because 496 lines of shell across 86 git invocations does not
//! fit one reviewable change.

use crate::common::run_with_timeout;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Never collapse worktrees into the repo root, which `WORKTREE_BASE_DIR="."`
/// would otherwise do.
const DEFAULT_BASE_DIR: &str = ".worktrees";

#[derive(Debug, PartialEq)]
pub enum ConflictAction {
    Spawn,
    Abort,
}

/// What to do when a rebase conflicts and AI-resolve is available.
///
/// Silent mode spawns without asking. Otherwise only an interactive terminal
/// may consent, and a non-tty aborts rather than assuming yes: the alternative
/// is an unattended run rewriting someone's conflicts.
pub fn conflict_action(silent: bool, is_tty: bool, answer: &str) -> ConflictAction {
    if silent {
        return ConflictAction::Spawn;
    }
    if !is_tty {
        return ConflictAction::Abort;
    }
    // Empty means the user pressed return on a default-yes prompt.
    match answer.trim().to_ascii_lowercase().as_str() {
        "" | "y" | "yes" => ConflictAction::Spawn,
        _ => ConflictAction::Abort,
    }
}

/// The directory holding this repo's worktrees: `<base>/<repo>`.
///
/// A relative base sits under the repo's parent, an absolute one is used as is.
/// The `<repo>` leaf is what stops same-named branches in sibling repos from
/// colliding.
pub fn resolve_base(repo_root: &Path, repo_parent: &Path, configured: Option<&str>) -> PathBuf {
    let repo_name = repo_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let base = match configured {
        Some(b) if !b.is_empty() && b != "." => b,
        _ => DEFAULT_BASE_DIR,
    };

    if base.starts_with('/') {
        Path::new(base).join(repo_name)
    } else {
        repo_parent.join(base).join(repo_name)
    }
}

/// Where the `.env` lives, relative to the repo root, or `None`.
///
/// `Some(".")` means the repo root itself. Without an explicit hint it looks
/// one level down, matching the shell's `find -mindepth 2 -maxdepth 2`.
pub fn find_env_base(repo_root: &Path, hint: Option<&str>) -> Option<String> {
    if let Some(hint) = hint.filter(|h| !h.is_empty()) {
        if hint == "." && repo_root.join(".env").is_file() {
            return Some(".".to_string());
        }
        if repo_root.join(hint).join(".env").is_file() {
            return Some(hint.to_string());
        }
        return None;
    }

    if repo_root.join(".env").is_file() {
        return Some(".".to_string());
    }

    // One level down only, and sorted so the answer does not depend on
    // readdir order the way the shell's `head -n1` did.
    let mut candidates: Vec<String> = std::fs::read_dir(repo_root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join(".env").is_file())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

#[derive(Debug, PartialEq)]
pub enum EnvCopy {
    Copied(PathBuf),
    NoEnvConfigured,
    SourceMissing,
    /// The reason this guard exists. Reported, never silent.
    RefusedNotGitignored(String),
}

/// Copies the `.env` into a new worktree, but ONLY when it is gitignored in the
/// source repo.
///
/// This is a secret-leak control, not a convenience. A tracked `.env` copied
/// into a fresh worktree is a file `git add` will happily stage, so an
/// unignored one is refused and the refusal is reported. No-clobber: an
/// existing destination file is left alone.
pub fn copy_env(repo_root: &Path, dest: &Path, env_base: Option<&str>) -> EnvCopy {
    let Some(env_base) = env_base.filter(|b| !b.is_empty()) else {
        return EnvCopy::NoEnvConfigured;
    };
    let rel = if env_base == "." {
        ".env".to_string()
    } else {
        format!("{env_base}/.env")
    };
    let src = repo_root.join(&rel);
    if !src.is_file() {
        return EnvCopy::SourceMissing;
    }
    if !is_gitignored(repo_root, &rel) {
        return EnvCopy::RefusedNotGitignored(rel);
    }

    let target = dest.join(&rel);
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if target.exists() {
        // cp -n: an existing file is never overwritten.
        return EnvCopy::Copied(target);
    }
    match std::fs::copy(&src, &target) {
        Ok(_) => EnvCopy::Copied(target),
        Err(_) => EnvCopy::SourceMissing,
    }
}

/// `git check-ignore -q`, where exit 0 means the path is ignored.
///
/// A git failure or timeout reads as NOT ignored, which refuses the copy. That
/// direction is deliberate: the failure mode of guessing wrong here is staging
/// a secret.
fn is_gitignored(repo_root: &Path, rel: &str) -> bool {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo_root)
        .args(["check-ignore", "-q", rel]);
    matches!(run_with_timeout(&mut command, GIT_TIMEOUT), Some(o) if o.status.success())
}

/// Branches tried when the remote publishes no `origin/HEAD`, in order.
const BASE_BRANCH_CANDIDATES: [&str; 4] = ["main", "master", "trunk", "develop"];

/// Last resort when the remote is unreachable and no candidate exists locally.
const BASE_BRANCH_FALLBACK: &str = "master";

/// A worktree is stale once its last commit is older than this.
const STALE_AFTER_DAYS: i64 = 30;

const SECS_PER_DAY: i64 = 86_400;

/// Cleanup runs at most this often per repo. Kept as its own name so callers
/// reference the rate limit rather than SECS_PER_DAY directly: retuning this
/// must not move the staleness cutoff, which is a separate decision that
/// happens to share the same number today.
const CLEANUP_INTERVAL_SECS: i64 = SECS_PER_DAY;

/// The base branch as a remote-tracking ref, for example `origin/main`.
///
/// Prefers what the remote itself publishes as `origin/HEAD`, then the common
/// names, so a repo whose default is `trunk` is not silently rebased onto a
/// `main` that does not exist.
pub fn base_branch(repo_root: &Path) -> String {
    let published = git_stdout(
        repo_root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .map(|s| s.trim().trim_start_matches("origin/").to_string())
    .filter(|s| !s.is_empty());

    if let Some(name) = published {
        return format!("origin/{name}");
    }

    for candidate in BASE_BRANCH_CANDIDATES {
        let reference = format!("refs/remotes/origin/{candidate}");
        if git_ok(repo_root, &["show-ref", "--verify", "--quiet", &reference]) {
            return format!("origin/{candidate}");
        }
    }
    format!("origin/{BASE_BRANCH_FALLBACK}")
}

/// `_wt_main`'s own fallback (worktree.sh:272) when `origin/HEAD` is
/// unpublished. Named separately from [`BASE_BRANCH_FALLBACK`] on purpose:
/// same string today, but it belongs to a different function with a
/// different contract (see [`main_base_ref`]'s doc), and this constant
/// exists so that stays true even if the two values are ever tuned apart.
const MAIN_BASE_REF_FALLBACK: &str = "master";

/// The bare base branch name `_wt_main` computes directly (worktree.sh:271-272),
/// e.g. `main`, never `origin/main`.
///
/// THIS IS A SECOND, DIFFERENTLY-SHAPED NOTION OF "THE BASE BRANCH," and the
/// difference is deliberate, not an oversight to reconcile. [`base_branch`]
/// (used by `_wt_make`, worktree.sh:72-88) returns a remote-tracking ref
/// PREFIXED with `origin/`, and falls back through four candidate names
/// (`main`, `master`, `trunk`, `develop`) before settling on `origin/master`.
/// This function returns the BARE name with no prefix, checks only
/// `origin/HEAD`, and on that being unpublished falls straight to a bare
/// `master` with no candidate loop at all. The two shapes are consumed
/// differently too: `_wt_main` builds `"$REMOTE/$BASE_REF"` itself from this
/// bare value (worktree.sh:274, and again by `_wt_maybe_rebase`), whereas
/// `base_branch`'s result is already a complete ref. Do not unify the two or
/// route this through `base_branch` plus a prefix strip: that would also
/// wrongly pull in the four-candidate fallback this one never had.
pub fn main_base_ref(repo_root: &Path) -> String {
    git_stdout(
        repo_root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .map(|s| s.trim().trim_start_matches("origin/").to_string())
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| MAIN_BASE_REF_FALLBACK.to_string())
}

/// Fetches `base_ref` and `branch` from `remote` before `_wt_main` decides
/// how to source the worktree (worktree.sh:274-275).
///
/// Falls back to fetching `base_ref` alone when the combined fetch fails,
/// since a brand-new branch that does not exist on the remote yet fails the
/// combined form, but the base ref still needs fetching regardless. The
/// whole call tolerates failure (the shell's trailing `|| true`): an
/// unreachable remote just means the rest of the run works with whatever
/// refs already exist locally. Shares [`REBASE_TIMEOUT`] rather than the 5s
/// `GIT_TIMEOUT` other calls in this file use, since this is a network fetch
/// like `rebase_onto`'s, not a local read.
pub fn initial_fetch(repo_root: &Path, remote: &str, base_ref: &str, branch: &str) {
    let mut combined = Command::new("git");
    combined
        .arg("-C")
        .arg(repo_root)
        .args(["fetch", remote, "--quiet", base_ref, branch]);
    if matches!(run_with_timeout(&mut combined, REBASE_TIMEOUT), Some(o) if o.status.success()) {
        return;
    }

    let mut base_only = Command::new("git");
    base_only
        .arg("-C")
        .arg(repo_root)
        .args(["fetch", remote, "--quiet", base_ref]);
    let _ = run_with_timeout(&mut base_only, REBASE_TIMEOUT);
}

/// Everything the staleness decision needs, gathered by the caller so the
/// decision itself stays pure and exhaustively testable.
pub struct WorktreeStatus<'a> {
    pub path: &'a Path,
    pub branch: &'a str,
    /// The worktree this run is creating, which must never be reaped.
    pub is_target: bool,
    /// Something is cwd'd into it.
    pub in_use: bool,
    pub has_open_pr: bool,
    pub merged_into_base: bool,
    /// Unix seconds of the last commit.
    pub last_commit_epoch: i64,
}

/// Whether a worktree may be removed.
///
/// The four skips come first and are absolute: the target of this run, anything
/// in use, a branch with an open pull request, and a detached or unreadable
/// HEAD. Only then does merged-or-old decide. Ordering matters, since an old
/// branch with an open PR is still wanted.
pub fn is_stale(status: &WorktreeStatus, now_epoch: i64) -> bool {
    if status.is_target || status.in_use || status.has_open_pr || status.branch.is_empty() {
        return false;
    }
    if status.merged_into_base {
        return true;
    }
    let cutoff = now_epoch - (STALE_AFTER_DAYS * SECS_PER_DAY);
    status.last_commit_epoch < cutoff
}

/// Whether the daily cleanup is due, given the marker's mtime.
///
/// `None` means the marker is absent, which counts as due: a repo that has
/// never been cleaned should be.
pub fn cleanup_due(marker_mtime_epoch: Option<i64>, now_epoch: i64) -> bool {
    match marker_mtime_epoch {
        None => true,
        Some(stamped) => now_epoch - stamped >= CLEANUP_INTERVAL_SECS,
    }
}

/// Directory and filename prefix for the per-repo cleanup rate-limit marker
/// (worktree.sh:203): `/tmp/.git-wt-cleanup-<hash>`.
const CLEANUP_MARKER_DIR: &str = "/tmp";
const CLEANUP_MARKER_PREFIX: &str = ".git-wt-cleanup-";

/// Directory and filename prefix for the per-repo fetch-cache marker
/// (worktree.sh:270): `/tmp/.git-fetch-<hash>`. This slice only computes
/// where the marker lives; touching it belongs to the background
/// housekeeping block (`touch "$FETCH_CACHE"`, worktree.sh:373), a later
/// slice's job.
const FETCH_CACHE_MARKER_DIR: &str = "/tmp";
const FETCH_CACHE_MARKER_PREFIX: &str = ".git-fetch-";

/// Ports `_wt_cleanup_stale` (worktree.sh:201-242): the outer entry point.
///
/// Gathers the two inputs that reach outside this process, a real
/// `/tmp/.git-wt-cleanup-*` marker path and a real `gh` call, then delegates
/// the actual git-driving work to [`cleanup_stale_with`]. That split exists
/// for testability: [`cleanup_stale_with`] is what the test suite calls
/// directly, with a scratch marker and an injected PR list, so no test
/// depends on `gh` being installed and authenticated or touches this
/// machine's real markers.
pub fn cleanup_stale(repo_root: &Path, target: &Path, now_epoch: i64) -> usize {
    let Some(marker) = cleanup_marker_path(repo_root) else {
        return 0;
    };
    let open_prs = open_pr_branches(repo_root);
    cleanup_stale_with(repo_root, target, &marker, &open_prs, now_epoch)
}

/// Ports the loop body of `_wt_cleanup_stale` (worktree.sh:201-242): given a
/// marker path and an open-PR list, decides and executes removals. Returns
/// how many worktrees were removed.
///
/// THIS FUNCTION DELETES BRANCHES AND WORKTREES. Every skip in [`is_stale`]
/// is a safety control, not a nicety, and the ordering below, remove first
/// and only delete the branch once that succeeds, is one too: a branch is
/// never destroyed while its worktree still exists on disk.
pub fn cleanup_stale_with(
    repo_root: &Path,
    target: &Path,
    marker: &Path,
    open_prs: &[String],
    now_epoch: i64,
) -> usize {
    if !cleanup_due(marker_mtime_epoch(marker), now_epoch) {
        return 0;
    }
    // Touched BEFORE any removal work (worktree.sh:206): a crash partway
    // through still rate-limits the next run, rather than retrying the same
    // destructive pass on every invocation until one finally finishes.
    touch_marker(marker, now_epoch);

    let base = base_branch(repo_root);
    let merged = merged_branches(repo_root, &base);

    let Some(porcelain) = git_stdout(repo_root, &["worktree", "list", "--porcelain"]) else {
        return 0;
    };

    let mut removed = 0;
    for path in cleanup_candidates(&porcelain) {
        let wt_path = Path::new(&path);
        let branch = git_stdout(wt_path, &["rev-parse", "--abbrev-ref", "HEAD"])
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let commit_epoch = git_stdout(wt_path, &["log", "-1", "--format=%ct"])
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);

        let status = WorktreeStatus {
            path: wt_path,
            branch: &branch,
            is_target: wt_path == target,
            in_use: is_in_use(wt_path),
            has_open_pr: contains_line(open_prs, &branch),
            merged_into_base: contains_line(&merged, &branch),
            last_commit_epoch: commit_epoch,
        };
        if !is_stale(&status, now_epoch) {
            continue;
        }

        if !git_ok(repo_root, &["worktree", "remove", "--force", path.as_str()]) {
            // The shell's `|| continue` (worktree.sh:236): a failed remove
            // must never be followed by a branch delete, so a branch is
            // never destroyed while its worktree still exists on disk.
            continue;
        }
        removed += 1;
        if branch != "HEAD" {
            let _ = git_ok(repo_root, &["branch", "-D", branch.as_str()]);
        }
    }

    let _ = git_ok(repo_root, &["worktree", "prune"]);
    removed
}

/// The default cleanup rate-limit marker path for `repo_root`
/// (worktree.sh:203). `None` when the hash could not be computed at all (no
/// `bash`, or it timed out).
///
/// Divergence: the shell in that state still proceeds, with an un-suffixed,
/// effectively repo-agnostic marker (`/tmp/.git-wt-cleanup-`). This port
/// instead treats an unknown hash as "cannot safely rate-limit" and skips the
/// run entirely, per this file's "when in doubt, keep" rule for anything
/// that decides what gets deleted.
fn cleanup_marker_path(repo_root: &Path) -> Option<PathBuf> {
    hashed_marker_path(CLEANUP_MARKER_DIR, CLEANUP_MARKER_PREFIX, repo_root)
}

/// `<dir>/<prefix><hash-of-repo_root>`, the shape both the cleanup marker
/// above and the fetch-cache marker below share. Factored out so the two
/// stay byte-for-byte consistent rather than each hand-rolling its own
/// `format!`; behaviour of the existing cleanup marker is unchanged, only
/// where its path construction lives.
fn hashed_marker_path(dir: &str, prefix: &str, repo_root: &Path) -> Option<PathBuf> {
    let hash = wt_hash(&repo_root.to_string_lossy())?;
    Some(Path::new(dir).join(format!("{prefix}{hash}")))
}

/// The fetch-cache marker path `_wt_main` computes at worktree.sh:270. See
/// [`FETCH_CACHE_MARKER_DIR`]'s doc for why nothing reads or writes it yet.
pub fn fetch_cache_marker_path(repo_root: &Path) -> Option<PathBuf> {
    hashed_marker_path(FETCH_CACHE_MARKER_DIR, FETCH_CACHE_MARKER_PREFIX, repo_root)
}

/// The shell's exact `_wt_hash` body (worktree.sh:69). `<<<` is a bash/zsh
/// here-string, so this runs under `bash`, matching this file's own
/// "sourceable in bash and zsh" contract, not `/bin/sh` (often dash on
/// Linux, which rejects `<<<`).
///
/// macOS's `md5 -q` prints the full 32-character digest; the Linux fallback
/// pipes through `cut -c1-8` and keeps only 8, so the two platforms produce
/// cache filenames of different lengths for the same repo. That quirk is
/// preserved on purpose, not normalized away.
const WT_HASH_SCRIPT: &str =
    r#"printf '%s\n' "$1" | md5 -q 2>/dev/null || md5sum <<< "$1" | cut -c1-8"#;

/// Shells out to compute the shell's cache-marker hash of `input`, rather
/// than hashing in Rust: the crate graph is deliberately clap + serde only,
/// with no hashing crate, and shelling out guarantees byte-for-byte parity
/// with the shell, digest length included (see [`WT_HASH_SCRIPT`]'s doc). If
/// this port and a still-installed shell ever computed the hash differently,
/// they would write to two different marker files for the same repo during
/// the coexistence period, and the once-a-day cleanup would effectively run
/// twice as often, once per implementation.
fn wt_hash(input: &str) -> Option<String> {
    let mut command = Command::new("bash");
    command.arg("-c").arg(WT_HASH_SCRIPT).arg("bash").arg(input);
    let out = run_with_timeout(&mut command, GIT_TIMEOUT)?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Unix seconds of `marker`'s mtime, or `None` when it does not exist or is
/// unreadable. Feeds directly into [`cleanup_due`], which already treats
/// `None` as due, so this does not need to replicate the shell's
/// `stat || stat || echo 0` fallback chain (worktree.sh:204): a missing
/// marker and an unreadable one both mean "we do not know when this last
/// ran," and that reads as due either way.
fn marker_mtime_epoch(marker: &Path) -> Option<i64> {
    let modified = std::fs::metadata(marker).ok()?.modified().ok()?;
    Some(
        modified
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    )
}

/// Updates `marker`'s mtime, creating it first if absent, matching the
/// shell's `touch "$cache"` (worktree.sh:206). Writes the current epoch
/// rather than merely opening the file: POSIX only guarantees an mtime bump
/// on `truncate` when the size actually changes, but a `write()` updates it
/// unconditionally, so writing is the reliable "touch."
fn touch_marker(marker: &Path, now_epoch: i64) {
    let _ = std::fs::write(marker, now_epoch.to_string());
}

/// The `worktree` field paths from `git worktree list --porcelain`, in
/// listed order. The first is always the main worktree.
fn worktree_paths(porcelain: &str) -> Vec<String> {
    porcelain
        .lines()
        .filter_map(|line| line.strip_prefix("worktree ").map(str::to_string))
        .collect()
}

/// The worktree paths eligible for cleanup: every entry from
/// `worktree_paths` EXCEPT the first, which is always the main worktree
/// (worktree.sh:218's `awk '$1=="worktree" && c++>0{print $2}'`) and must
/// never be a cleanup candidate. Getting this off by one would delete the
/// user's main checkout.
///
/// Pulled out as its own function, rather than an inline `.skip(1)` in
/// [`cleanup_stale_with`], so this exact decision has a direct unit test: a
/// live-git test cannot tell this bug apart from correct behaviour, since
/// `git worktree remove` already refuses to remove the main working tree on
/// its own, no matter what this function passes it.
pub fn cleanup_candidates(porcelain: &str) -> Vec<String> {
    worktree_paths(porcelain).into_iter().skip(1).collect()
}

/// The main worktree's path: the FIRST entry from `git worktree list
/// --porcelain` (worktree.sh:258's `awk '$1=="worktree" && !f{print $2;
/// f=1}'`), what `_wt_main` `cd`'s into before anything else.
///
/// The exact counterpart of [`cleanup_candidates`]: one keeps the first
/// entry, the other drops it, and both are built on the same
/// [`worktree_paths`] so a change to how paths are parsed cannot put the
/// pair out of sync with each other.
pub fn main_worktree(porcelain: &str) -> Option<String> {
    worktree_paths(porcelain).into_iter().next()
}

/// `git branch --merged <base>` (worktree.sh:213), with the shell's
/// `sed 's/^[[:space:]*+]*//'` prefix-strip applied: `*` marks the branch
/// checked out in the worktree this command ran from, `+` marks one checked
/// out in another linked worktree, and either can mix with leading spaces.
fn merged_branches(repo_root: &Path, base: &str) -> Vec<String> {
    git_stdout(repo_root, &["branch", "--merged", base])
        .map(|out| {
            out.lines()
                .map(|line| strip_branch_marker(line).to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn strip_branch_marker(line: &str) -> &str {
    line.trim_start_matches(|c: char| c.is_whitespace() || c == '*' || c == '+')
}

/// Whole-line, fixed-string membership, matching the shell's `grep -qxF`
/// (`-x` whole line, `-F` fixed string): worktree.sh:225 (open PRs) and
/// worktree.sh:228 (merged branches). A substring match here would spare or
/// reap the wrong branch, e.g. an open-PR entry of `feat-two` must not spare
/// a branch named `feat`.
fn contains_line(haystack: &[String], needle: &str) -> bool {
    !needle.is_empty() && haystack.iter().any(|line| line == needle)
}

/// Whether some process's cwd is inside `path`, mirroring `lsof -d cwd +c0`
/// piped to a plain, non-anchored `grep -qF` (worktree.sh:220). Absent
/// `lsof` reads as "not in use": the shell's own `command -v lsof` gate means
/// a machine without it never treats any worktree as busy.
fn is_in_use(path: &Path) -> bool {
    if !command_exists("lsof") {
        return false;
    }
    let mut command = Command::new("lsof");
    command.args(["-d", "cwd", "+c0"]);
    let Some(out) = run_with_timeout(&mut command, GIT_TIMEOUT) else {
        return false;
    };
    let path_str = path.to_string_lossy();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|line| line.contains(path_str.as_ref()))
}

/// The current user's open PR head branches (worktree.sh:214-216), or empty
/// when `gh` is absent or the call fails. Gathered here, in the outer
/// [`cleanup_stale`], rather than in [`cleanup_stale_with`], so tests can
/// inject the list directly instead of depending on `gh` being installed and
/// authenticated.
fn open_pr_branches(repo_root: &Path) -> Vec<String> {
    if !command_exists("gh") {
        return Vec::new();
    }
    let mut command = Command::new("gh");
    command.current_dir(repo_root).args([
        "pr",
        "list",
        "--state",
        "open",
        "--author",
        "@me",
        "--limit",
        "200",
        "--json",
        "headRefName",
        "--jq",
        ".[].headRefName",
    ]);
    match run_with_timeout(&mut command, GIT_TIMEOUT) {
        Some(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn git_stdout(repo_root: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo_root).args(args);
    let out = run_with_timeout(&mut command, GIT_TIMEOUT)?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

fn git_ok(repo_root: &Path, args: &[&str]) -> bool {
    git_stdout(repo_root, args).is_some()
}

/// Whether the new worktree can reuse the main checkout's `node_modules`.
///
/// Requires a `package.json` in the worktree, and a lockfile plus installed
/// modules in the source, then that the two lockfiles are byte-identical.
///
/// The shell hashed both lockfiles with sha256sum or shasum and compared the
/// digests, which also meant skipping the reuse entirely when neither tool was
/// present. Comparing the bytes answers the same question, is strictly
/// stronger than a digest, and drops both the external dependency and that
/// silent skip.
pub fn node_modules_reusable(worktree: &Path, repo_root: &Path) -> bool {
    if !worktree.join("package.json").is_file() {
        return false;
    }
    let source_lock = repo_root.join("package-lock.json");
    if !source_lock.is_file() || !repo_root.join("node_modules").is_dir() {
        return false;
    }

    // A worktree without its own lockfile inherits the source's, which is the
    // copy the shell made before comparing, so that case matches by definition.
    let worktree_lock = worktree.join("package-lock.json");
    if !worktree_lock.is_file() {
        return true;
    }
    match (std::fs::read(&source_lock), std::fs::read(&worktree_lock)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// A `node_modules` tree can hold tens of thousands of files, and the copy
/// ladder's last resort (`cp -R`, once copy-on-write and reflink both fail)
/// walks every one of them. Generous on purpose: a short timeout here would
/// abort a real, still-progressing copy rather than a stuck one.
const NODE_MODULES_COPY_TIMEOUT: Duration = Duration::from_secs(120);

/// Bounds the best-effort `npm install --prefer-offline` refresh after
/// cloning. It only has to reconcile an already-installed tree rather than
/// fetch one from scratch, so this is far shorter than the copy timeout: a
/// machine with no network should fail fast instead of stalling worktree
/// setup.
const NPM_INSTALL_TIMEOUT: Duration = Duration::from_secs(15);

/// Ports the imperative half of `_wt_node_modules` (worktree.sh:174-197):
/// once [`node_modules_reusable`] says the reuse is safe, this performs it.
///
/// Order mirrors the shell with one deliberate exception. The shell seeds a
/// missing worktree lockfile BEFORE its hash comparison runs (worktree.sh:184);
/// this instead decides first and seeds only when the decision is `true`.
/// The two orders are provably equivalent: `node_modules_reusable` already
/// re-checks every precondition the shell checks before its own seed line, so
/// a `false` decision means the shell would never have reached that line
/// either, and a `true` decision comes only from a lockfile that already
/// matches (nothing to seed) or one that is missing (seeded now, and a copy
/// of an identical file always matches). Deciding first also means a
/// worktree that fails a precondition is never touched, which a literal
/// seed-first port would get wrong for a worktree with no `package.json` at
/// all: the shell returns before ever reaching its seed line, but seeding
/// unconditionally here would still write a lockfile into a non-node
/// worktree.
///
/// The shell's diagnostic ("Cloned node_modules (copy-on-write)") is left to
/// the caller, since printing is not this module's job; that message also
/// claims copy-on-write unconditionally even when the ladder below fell
/// through to the plain `cp -R`, a pre-existing inaccuracy this port does not
/// fix. The best-effort `npm install --prefer-offline --no-audit --no-fund`
/// refresh that follows it is kept, since it changes what ends up on disk
/// rather than just announcing it, and the shell runs it regardless of
/// whether the copy ladder actually succeeded.
pub fn reuse_node_modules(worktree: &Path, repo_root: &Path) -> bool {
    if !node_modules_reusable(worktree, repo_root) {
        return false;
    }

    let worktree_lock = worktree.join("package-lock.json");
    if !worktree_lock.is_file() {
        // Best-effort, matching the shell's `2>/dev/null || true`: a failed
        // copy must not undo a reuse that was otherwise judged safe.
        let _ = std::fs::copy(repo_root.join("package-lock.json"), &worktree_lock);
    }

    let source = repo_root.join("node_modules");
    let dest = worktree.join("node_modules");
    // `-e`, not `-d` (worktree.sh:191): a plain FILE named `node_modules` is
    // removed too, not just a stray directory.
    remove_node_modules(&dest);

    let copied = clone_node_modules(&source, &dest);

    if command_exists("npm") {
        let mut command = Command::new("npm");
        command.current_dir(worktree).args([
            "install",
            "--prefer-offline",
            "--no-audit",
            "--no-fund",
        ]);
        let _ = run_with_timeout(&mut command, NPM_INSTALL_TIMEOUT);
    }

    copied
}

/// The shell's three-tier `cp` fallback (worktree.sh:192-194): a macOS
/// copy-on-write clone, then a GNU reflink copy, then a plain recursive copy.
/// Each retry removes whatever the previous attempt left behind first, since
/// the shell repeats `rm -rf node_modules` before every retry, not just once:
/// a `cp` that fails partway can still leave a partial destination that would
/// make the next attempt fail on "already exists" instead of actually
/// retrying.
fn clone_node_modules(source: &Path, dest: &Path) -> bool {
    if run_cp(&["-cR"], source, dest) {
        return true;
    }
    remove_node_modules(dest);
    if run_cp(&["-R", "--reflink=auto"], source, dest) {
        return true;
    }
    remove_node_modules(dest);
    run_cp(&["-R"], source, dest)
}

/// One `cp` attempt from the ladder above.
///
/// The shell suppresses stderr on the first two tiers and lets the third
/// flow to the terminal, so a real failure is visible only once the cheaper
/// fallbacks are exhausted. This port has no output channel of its own here
/// (`run_with_timeout` always captures both streams and nothing forwards
/// them), so that distinction has no observable effect in this function;
/// noted rather than silently dropped.
fn run_cp(args: &[&str], source: &Path, dest: &Path) -> bool {
    let mut command = Command::new("cp");
    command.args(args).arg(source).arg(dest);
    matches!(
        run_with_timeout(&mut command, NODE_MODULES_COPY_TIMEOUT),
        Some(o) if o.status.success()
    )
}

/// Removes whatever is at `path`, mirroring `rm -rf`: a no-op when nothing is
/// there, and correct for either a directory or a plain file.
fn remove_node_modules(path: &Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}

/// A stand-in for the shell's `command -v name`: true when spawning `name
/// --version` succeeds at all. `--version` is near-universal and returns
/// almost instantly, so this shares the install's timeout rather than
/// needing its own.
fn command_exists(name: &str) -> bool {
    let mut command = Command::new(name);
    command.arg("--version");
    run_with_timeout(&mut command, NPM_INSTALL_TIMEOUT).is_some()
}

/// Whether the main worktree is dirty enough to need an auto-stash, the
/// gate at worktree.sh:278.
///
/// Two checks, not one: `git diff-index --quiet HEAD --` catches changes
/// against HEAD (staged or unstaged), and `git diff --quiet` catches changes
/// against the index. Either alone would still flag a plain unstaged edit,
/// but only the first sees a staged-but-uncommitted change whose working
/// tree content happens to already match the index, so collapsing to one
/// check would miss that case.
pub fn needs_stash(repo_root: &Path) -> bool {
    !git_ok(repo_root, &["diff-index", "--quiet", "HEAD", "--"])
        || !git_ok(repo_root, &["diff", "--quiet"])
}

/// The message `_wt_main` tags its auto-stash with (worktree.sh:279), so
/// `restore_stash`'s pop targets exactly the entry this pushed.
const AUTO_STASH_MESSAGE: &str = "worktree: auto-stash";

/// Auto-stashes a dirty main worktree, worktree.sh:279's
/// `git stash push -m "worktree: auto-stash" --quiet`. Returns whether a
/// stash was actually created.
///
/// `git stash push` exits 0 even when there is nothing to stash ("No local
/// changes to save" is not an error to git), so a bare exit-code check would
/// wrongly report success on a clean tree. That never happens in the shell,
/// which only calls this after [`needs_stash`] has already confirmed the
/// tree is dirty, but this function is a public, independently callable
/// port, and its return value is exactly what tells a caller whether a later
/// `git stash pop` (see [`restore_stash`]) is popping a stash this call
/// pushed versus an unrelated one that predates it. So this compares the
/// stash count before and after, and only reports success when it actually
/// grew.
pub fn auto_stash(repo_root: &Path) -> bool {
    let before = stash_count(repo_root);
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo_root)
        .args(["stash", "push", "-m", AUTO_STASH_MESSAGE, "--quiet"]);
    let succeeded =
        matches!(run_with_timeout(&mut command, GIT_TIMEOUT), Some(o) if o.status.success());
    succeeded && stash_count(repo_root) > before
}

fn stash_count(repo_root: &Path) -> usize {
    git_stdout(repo_root, &["stash", "list"])
        .map(|out| out.lines().filter(|line| !line.is_empty()).count())
        .unwrap_or(0)
}

/// Ports `_wt_restore_stash` (worktree.sh:248-251): pops the auto-stash taken
/// on the main worktree before this run began, best-effort. `stash_applied`
/// stands in for the shell's `STASH_APPLIED` global, passed in explicitly
/// rather than read from process state.
pub fn restore_stash(main_worktree: &Path, stash_applied: bool) {
    if !stash_applied {
        return;
    }
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(main_worktree)
        .args(["stash", "pop", "--quiet"]);
    // The shell's `|| true`: a failed pop (nothing to pop, a conflict) is not
    // this function's problem to report.
    let _ = run_with_timeout(&mut command, GIT_TIMEOUT);
}

/// The worktree path already checked out on `branch`, from
/// `git worktree list --porcelain`.
///
/// The recovery path when `git worktree add` fails: a branch can only live in
/// one worktree, so the usual cause is that it is already checked out
/// somewhere, and reporting that beats a bare failure.
pub fn worktree_for_branch(porcelain: &str, branch: &str) -> Option<String> {
    let wanted = format!("refs/heads/{branch}");
    let mut current: Option<&str> = None;
    for line in porcelain.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            current = Some(path);
        } else if let Some(found) = line.strip_prefix("branch ") {
            if found == wanted {
                return current.map(|p| p.to_string());
            }
        }
    }
    None
}

/// The `git worktree add` argument list, in the shell's exact order.
///
/// The shell builds `[-b <new_branch>] <dest> <ref>` once (worktree.sh:104-106),
/// then only prepends `-f` on the retry (`git worktree add -f
/// "${cmd_args[@]}"`, worktree.sh:111), so `-f` lands BEFORE `-b`, not after.
/// A line-for-line port exists precisely so this order is not left to guessing.
pub fn worktree_add_args(
    dest: &Path,
    reference: &str,
    new_branch: Option<&str>,
    force: bool,
) -> Vec<OsString> {
    let mut args = Vec::new();
    if force {
        args.push(OsString::from("-f"));
    }
    if let Some(branch) = named_branch(new_branch) {
        args.push(OsString::from("-b"));
        args.push(OsString::from(branch));
    }
    args.push(dest.as_os_str().to_os_string());
    args.push(OsString::from(reference));
    args
}

/// The branch name for the fallback lookup once both `git worktree add`
/// attempts fail: the shell's `refs/heads/${new_branch:-$ref}`
/// (worktree.sh:113), minus the `refs/heads/` prefix that `worktree_for_branch`
/// already adds.
///
/// When a new branch is being created, that new branch is what to look up.
/// Otherwise the REFERENCE ITSELF is looked up as though it were a local
/// branch name, which only resolves when it happens to name one, e.g. a plain
/// branch rather than a remote-tracking ref like `origin/main`. That mismatch
/// is a genuine quirk of the shell, preserved here deliberately rather than
/// fixed.
pub fn fallback_lookup_branch<'a>(reference: &'a str, new_branch: Option<&'a str>) -> &'a str {
    named_branch(new_branch).unwrap_or(reference)
}

/// Collapses an empty branch name to `None`.
///
/// Both shell tests that read `$new_branch` treat empty and unset as the same
/// thing: `[[ -n "$new_branch" ]]` gates the `-b` flag, and `${new_branch:-$ref}`
/// falls back on empty as well as unset. `Option<&str>` draws that line
/// differently, so without this an empty name would pass `-b ""` to git and
/// look up an empty branch, neither of which the shell can do.
fn named_branch(new_branch: Option<&str>) -> Option<&str> {
    new_branch.filter(|b| !b.is_empty())
}

/// Outcome of [`create_worktree`].
#[derive(Debug, PartialEq)]
pub enum CreateOutcome {
    /// A fresh worktree was created at the given path.
    Created(PathBuf),
    /// Nothing was created: the wanted branch was already checked out at the
    /// given path, discovered by the fallback lookup.
    AlreadyAt(PathBuf),
    /// Both `git worktree add` attempts failed and no fallback path resolved.
    Failed,
}

/// Creates a worktree, recovering from a stale registration if needed.
///
/// Mirrors the shell's three-tier ladder in `_wt_create_worktree`
/// (worktree.sh:102-124): try a plain `git worktree add`; on failure run
/// `worktree prune` then `worktree repair`, each allowed to fail (the shell's
/// `|| true`) since they are best-effort recovery, not preconditions; retry
/// with `-f`; and on a second failure, fall back to whatever worktree already
/// holds the branch via [`worktree_for_branch`], only accepting it when that
/// worktree's directory still exists (the shell's `-d "$existing"`).
///
/// The shell prints two diagnostics here ("branch already at $existing", "git
/// worktree add failed"); this function returns the outcome instead and
/// leaves emitting messages to the caller, so that omission is a documented
/// divergence rather than a silently dropped one.
pub fn create_worktree(
    repo_root: &Path,
    dest: &Path,
    reference: &str,
    new_branch: Option<&str>,
) -> CreateOutcome {
    if run_worktree_add(repo_root, dest, reference, new_branch, false) {
        return CreateOutcome::Created(dest.to_path_buf());
    }

    let _ = git_ok(repo_root, &["worktree", "prune"]);
    let _ = git_ok(repo_root, &["worktree", "repair"]);

    if run_worktree_add(repo_root, dest, reference, new_branch, true) {
        return CreateOutcome::Created(dest.to_path_buf());
    }

    let Some(porcelain) = git_stdout(repo_root, &["worktree", "list", "--porcelain"]) else {
        return CreateOutcome::Failed;
    };
    let lookup_branch = fallback_lookup_branch(reference, new_branch);
    if let Some(existing) = worktree_for_branch(&porcelain, lookup_branch) {
        let path = PathBuf::from(existing);
        if path.is_dir() {
            return CreateOutcome::AlreadyAt(path);
        }
    }
    CreateOutcome::Failed
}

/// What a new worktree checks out, and whether that means creating a branch.
#[derive(Debug, PartialEq)]
pub struct MakePlan {
    /// The ref `git worktree add` checks out.
    pub reference: String,
    /// `Some` when the branch does not exist yet and `-b` must create it.
    pub new_branch: Option<String>,
    /// Whether to clear the new branch's inherited upstream afterwards.
    pub unset_upstream: bool,
}

/// Ports the branch-source choice in `_wt_make` (worktree.sh:390-401): check
/// out an existing local branch, else start one from its remote counterpart,
/// else start one from the base ref.
///
/// Only the third case unsets the upstream (worktree.sh:398). Branching off
/// the base ref makes git inherit the BASE's tracking, so without this a later
/// bare `git push` on the new branch would target the base branch. The other
/// two cases already have correct tracking, so clearing it there would break
/// them.
pub fn make_plan(
    branch: &str,
    remote: &str,
    base: &str,
    local_exists: bool,
    remote_exists: bool,
) -> MakePlan {
    if local_exists {
        return MakePlan {
            reference: branch.to_string(),
            new_branch: None,
            unset_upstream: false,
        };
    }
    if remote_exists {
        return MakePlan {
            reference: format!("refs/remotes/{remote}/{branch}"),
            new_branch: Some(branch.to_string()),
            unset_upstream: false,
        };
    }
    MakePlan {
        reference: base.to_string(),
        new_branch: Some(branch.to_string()),
        unset_upstream: true,
    }
}

/// Creates the worktree for `branch`, choosing its source via [`make_plan`].
///
/// The two `show-ref --verify` probes use fully qualified refs so a tag or a
/// remote ref sharing the branch's name cannot be mistaken for a local branch.
pub fn make_worktree(repo_root: &Path, target: &Path, branch: &str, remote: &str) -> CreateOutcome {
    let local_exists = git_ok(
        repo_root,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    );
    let remote_exists = git_ok(
        repo_root,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/remotes/{remote}/{branch}"),
        ],
    );
    let plan = make_plan(
        branch,
        remote,
        &base_branch(repo_root),
        local_exists,
        remote_exists,
    );

    let outcome = create_worktree(
        repo_root,
        target,
        &plan.reference,
        plan.new_branch.as_deref(),
    );

    // Only after a successful create, since the shell's `|| return $?` means a
    // failed create never reaches the unset.
    if plan.unset_upstream && !matches!(outcome, CreateOutcome::Failed) {
        let _ = git_ok(repo_root, &["branch", "--unset-upstream", branch]);
    }
    outcome
}

/// Runs `git worktree add` with [`worktree_add_args`], reporting only success.
fn run_worktree_add(
    repo_root: &Path,
    dest: &Path,
    reference: &str,
    new_branch: Option<&str>,
    force: bool,
) -> bool {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo_root).arg("worktree").arg("add");
    command.args(worktree_add_args(dest, reference, new_branch, force));
    matches!(run_with_timeout(&mut command, GIT_TIMEOUT), Some(o) if o.status.success())
}

/// What the launcher found at the target path, gathered by the caller so the
/// decision stays pure.
pub struct TargetState<'a> {
    pub target_exists: bool,
    /// Registered with `git worktree list`, as opposed to a stray directory.
    pub registered: bool,
    /// `None` means detached or unreadable HEAD.
    pub current_branch: Option<&'a str>,
    pub wanted_branch: &'a str,
    /// Whether the branch currently checked out there still exists on the remote.
    pub current_branch_on_remote: bool,
    /// A different worktree already holding the wanted branch, and whether its
    /// directory is still present.
    pub existing_for_wanted: Option<(&'a str, bool)>,
}

#[derive(Debug, PartialEq)]
pub enum WorktreePlan {
    /// Detached HEAD in a registered worktree: abort any rebase or merge and
    /// check the branch back out. The caller re-evaluates afterwards, since
    /// recovery can fail and fall through to a rebuild.
    RecoverDetached,
    /// A directory git does not know about. Prune, remove, recreate.
    CleanOrphanAndCreate,
    /// Already on the wanted branch: fetch and fast-forward in place.
    ReuseTarget,
    /// The occupying branch still exists on the remote, so it is unfinished
    /// work. Refuse rather than destroy it.
    RefuseOccupied(String),
    /// The occupying branch is gone from the remote, so it was merged or
    /// deleted and the worktree can be recycled.
    RecycleTarget(String),
    /// The wanted branch is checked out in another worktree that still exists.
    ReuseExisting(String),
    /// Git has a worktree registered for the branch but the directory is gone.
    PruneStaleAndCreate,
    Create,
}

/// Chooses the recovery path for the target worktree.
///
/// The ordering is the whole point and mirrors the shell. In particular
/// `RefuseOccupied` comes before `RecycleTarget`: recycling removes the
/// worktree and deletes its branch, so a branch that still exists on the remote
/// must stop the operation rather than be destroyed. Getting that pair the
/// wrong way round would silently discard unfinished work.
pub fn plan_for_target(state: &TargetState) -> WorktreePlan {
    if state.target_exists {
        return match (state.current_branch, state.registered) {
            (None, true) => WorktreePlan::RecoverDetached,
            (None, false) => WorktreePlan::CleanOrphanAndCreate,
            (Some(current), _) if current == state.wanted_branch => WorktreePlan::ReuseTarget,
            (Some(current), _) if state.current_branch_on_remote => {
                WorktreePlan::RefuseOccupied(current.to_string())
            }
            (Some(current), _) => WorktreePlan::RecycleTarget(current.to_string()),
        };
    }

    match state.existing_for_wanted {
        Some((path, true)) => WorktreePlan::ReuseExisting(path.to_string()),
        Some((_, false)) => WorktreePlan::PruneStaleAndCreate,
        None => WorktreePlan::Create,
    }
}

/// Minimum letters in a JIRA project key, so a branch like `a-1` is not
/// mistaken for one.
const MIN_JIRA_PREFIX_LEN: usize = 2;

/// Trims whitespace and carriage returns from a branch name.
///
/// The CR matters: a branch name pasted from a Windows-authored ticket or a CI
/// variable carries one, and git rejects it while the error names a branch that
/// looks identical to the one requested.
pub fn sanitize_branch(raw: &str) -> String {
    raw.replace('\r', "").trim().to_string()
}

/// Whether `branch` is a syntactically valid git branch name:
/// `git check-ref-format --branch <branch>` (worktree.sh:263).
///
/// Delegates to git rather than hand-rolling the ref-name grammar (no
/// leading `-`, no `..`, no trailing `/`, no embedded whitespace, and more):
/// git owns those rules, and a re-implementation here would just be a second
/// copy of them to keep in sync.
pub fn valid_branch_name(repo_root: &Path, branch: &str) -> bool {
    git_ok(repo_root, &["check-ref-format", "--branch", branch])
}

/// The first JIRA-style key in the branch, uppercased.
///
/// Matches the shell's case-insensitive `[A-Z]{2,}-[0-9]+`, so `fix/abc-45`
/// yields `ABC-45`. Scanned by hand rather than with a regex, since the crate
/// graph is deliberately clap plus serde.
pub fn jira_key(branch: &str) -> Option<String> {
    let bytes = branch.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        let letters = i - start;
        if letters < MIN_JIRA_PREFIX_LEN || i >= bytes.len() || bytes[i] != b'-' {
            continue;
        }
        let dash = i;
        i += 1;
        let digits_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i > digits_start {
            return Some(branch[start..i].to_uppercase());
        }
        i = dash + 1;
    }
    None
}

/// Everything after the final slash, matching `${BRANCH##*/}`.
pub fn branch_leaf(branch: &str) -> &str {
    branch.rsplit('/').next().unwrap_or(branch)
}

/// The folder name a branch gets under the worktree root.
///
/// A JIRA key wins, so `feature/PROJ-1/spike` and `fix/PROJ-1` share one
/// folder, which is the point: one ticket, one worktree. `occupied_by` is the
/// branch already checked out in that key's folder, if any. When it is a
/// DIFFERENT branch the key is already taken, so this falls back to the leaf
/// rather than colliding.
pub fn folder_for_branch(branch: &str, occupied_by: Option<&str>) -> String {
    match jira_key(branch) {
        Some(key) => match occupied_by {
            Some(other) if other != branch => branch_leaf(branch).to_string(),
            _ => key,
        },
        None => branch_leaf(branch).to_string(),
    }
}

/// Branches that never auto-rebase, even when you authored their last commit.
///
/// Without this list, the ownership heuristic below would happily rebase
/// `develop` or a `release/*` onto base, because whoever committed last looks
/// like the owner. Shared history is not personal history.
/// Slices rather than fixed-size arrays so adding a name stays a one-line edit
/// with no length to keep in sync.
const PROTECTED_BRANCHES: &[&str] = &[
    "main", "master", "trunk", "develop", "dev", "staging", "release", "hotfix",
];

/// Prefixes whose whole namespace is protected. `release` and `hotfix` appear
/// in both lists: bare, and as the root of a namespace.
const PROTECTED_PREFIXES: &[&str] = &["release/", "hotfix/"];

fn is_protected(branch: &str) -> bool {
    PROTECTED_BRANCHES.contains(&branch) || PROTECTED_PREFIXES.iter().any(|p| branch.starts_with(p))
}

/// What the rebase decision needs, gathered by the caller.
pub struct RebaseContext<'a> {
    pub current_branch: &'a str,
    /// `git config user.name`, empty when unset.
    pub git_user: &'a str,
    /// Author of the branch's last commit.
    pub branch_author: &'a str,
    /// GitHub login, empty when `gh` is absent or unauthenticated.
    pub gh_user: &'a str,
    /// The branch the launcher was asked for, which may embed the login.
    pub wanted_branch: &'a str,
    pub base_ref: &'a str,
}

/// Whether this branch may be auto-rebased onto base.
///
/// Protection is checked FIRST and is absolute. Only then does ownership apply,
/// via either the last commit's author or a login embedded in the branch name.
/// Rebasing rewrites history, so every uncertain case declines.
pub fn should_rebase(ctx: &RebaseContext) -> bool {
    if is_protected(ctx.current_branch) {
        return false;
    }
    // Detached HEAD, or already sitting on base: nothing to rebase onto.
    if ctx.current_branch == ctx.base_ref || ctx.current_branch == "HEAD" {
        return false;
    }

    let authored_by_me = !ctx.git_user.is_empty() && ctx.branch_author == ctx.git_user;
    let named_for_me = !ctx.gh_user.is_empty() && ctx.wanted_branch.contains(ctx.gh_user);
    authored_by_me || named_for_me
}

/// The `git rebase` argument list, worktree.sh:427-429.
///
/// `--rebase-merges` is appended only when the branch actually carries merge
/// commits against upstream: rewriting merge topology a linear history
/// doesn't have would be pointless churn. Argument ORDER matches the shell's
/// array construction exactly.
pub fn rebase_args(upstream: &str, has_merge_commits: bool) -> Vec<&str> {
    let mut args = vec![upstream, "--quiet"];
    if has_merge_commits {
        args.push("--rebase-merges");
    }
    args
}

/// How long `git fetch` and `git rebase` may run before being killed.
///
/// Both touch the network or replay an unbounded number of commits, unlike
/// every other git call in this file, which only reads local state; the 5s
/// `GIT_TIMEOUT` is sized for that, not this.
///
/// Divergence: the shell caps neither, so any value here can cut short a call
/// the shell would let finish. Sized high because the failure mode is quiet
/// rather than loud: a timed-out fetch is indistinguishable from a failed
/// one, and both are tolerated, so the rebase would then run against a STALE
/// `<remote>/<base_ref>`. A cap tight enough to fire on a large repo or a
/// slow link would silently rebase onto the wrong base. This is a bound on
/// genuinely hung processes only, not a latency budget.
const REBASE_TIMEOUT: Duration = Duration::from_secs(300);

/// Outcome of attempting to bring the current branch up to date with base.
#[derive(Debug, PartialEq)]
pub enum RebaseOutcome {
    /// HEAD already contains `<remote>/<base_ref>`; nothing was rebased.
    UpToDate,
    /// The rebase completed cleanly.
    Rebased,
    /// The rebase failed and is STILL IN PROGRESS on disk.
    ///
    /// Mirrors the instant worktree.sh:431 finds `git rebase` failed: at that
    /// point the shell has not yet decided between spawning the AI resolver
    /// and aborting, so this port stops at the same spot and hands that
    /// decision to the caller. A caller that drops this outcome on the floor
    /// leaves the worktree mid-rebase, an actually broken state rather than a
    /// resolved one; it MUST eventually call [`abort_rebase`], directly or
    /// after a failed resolution attempt.
    Conflicted,
}

/// Fetches `base_ref` from `remote` and rebases the current branch onto it,
/// worktree.sh:424-431.
///
/// A failed fetch (offline, unreachable remote) is not fatal: the shell's
/// `|| true` lets the ancestry check and rebase proceed against whatever
/// `<remote>/<base_ref>` already exists locally.
pub fn rebase_onto(worktree: &Path, remote: &str, base_ref: &str) -> RebaseOutcome {
    let mut fetch = Command::new("git");
    fetch
        .arg("-C")
        .arg(worktree)
        .args(["fetch", remote, base_ref, "--quiet"]);
    let _ = run_with_timeout(&mut fetch, REBASE_TIMEOUT);

    let upstream = format!("{remote}/{base_ref}");
    if git_ok(
        worktree,
        &["merge-base", "--is-ancestor", &upstream, "HEAD"],
    ) {
        return RebaseOutcome::UpToDate;
    }

    let range = format!("{upstream}..HEAD");
    let has_merge_commits = git_stdout(worktree, &["log", "--merges", "--oneline", &range])
        .is_some_and(|out| !out.trim().is_empty());

    let mut rebase = Command::new("git");
    rebase
        .arg("-C")
        .arg(worktree)
        .arg("rebase")
        .args(rebase_args(&upstream, has_merge_commits));
    match run_with_timeout(&mut rebase, REBASE_TIMEOUT) {
        Some(out) if out.status.success() => RebaseOutcome::Rebased,
        _ => RebaseOutcome::Conflicted,
    }
}

/// Aborts an in-progress rebase.
///
/// Matches the shell's `git rebase --abort 2>/dev/null || true`, used at all
/// three of its call sites: failing because there was no rebase in progress
/// is not an error worth reporting.
pub fn abort_rebase(worktree: &Path) {
    let mut command = Command::new("git");
    command.arg("-C").arg(worktree).args(["rebase", "--abort"]);
    let _ = run_with_timeout(&mut command, GIT_TIMEOUT);
}

/// The detached-HEAD safety net, worktree.sh:453-458.
///
/// Runs after a rebase attempt, successful or not: if HEAD somehow ended up
/// detached, abort whatever rebase might still be in progress and restore a
/// real branch. `current_branch` (what the worktree started on) is tried
/// FIRST, `wanted_branch` (what the launcher was asked for) only as a
/// fallback once that checkout fails. The shell's `||` chain fixes that
/// order and it is load-bearing: collapsing it to "try either" would recover
/// onto the wrong branch whenever both exist.
///
/// Returns whether HEAD was actually detached and this had to act. The shell
/// also prints "HEAD detached after rebase. Aborting and restoring." here;
/// that is left to the caller, which is what the return value is for.
pub fn recover_detached_head(worktree: &Path, current_branch: &str, wanted_branch: &str) -> bool {
    let detached = git_stdout(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])
        .is_some_and(|head| head.trim() == "HEAD");
    if !detached {
        return false;
    }
    abort_rebase(worktree);
    if !git_ok(worktree, &["checkout", current_branch]) {
        git_ok(worktree, &["checkout", wanted_branch]);
    }
    true
}

#[derive(Debug, PartialEq)]
pub enum UpstreamAction {
    /// Tracking is already correct.
    None,
    /// The branch exists on the remote, so point at it without pushing.
    SetTracking,
    /// The branch is remote-only-absent and pushing is allowed.
    PushAndTrack,
    /// Absent remotely and `--no-push` was given, so leave it local.
    SkipNoPush,
}

pub fn upstream_action(
    current_upstream: Option<&str>,
    expected: &str,
    exists_on_remote: bool,
    no_push: bool,
) -> UpstreamAction {
    if current_upstream == Some(expected) {
        return UpstreamAction::None;
    }
    if exists_on_remote {
        return UpstreamAction::SetTracking;
    }
    if no_push {
        return UpstreamAction::SkipNoPush;
    }
    UpstreamAction::PushAndTrack
}
