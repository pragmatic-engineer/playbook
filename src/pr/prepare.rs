// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `pr prepare`: every judgment-free step that comes before drafting a PR's
//! title and body (branch and base, size and test checks, the diff, the
//! ticket), printed as labeled lines the calling command copies verbatim.

use crate::pr::shared::{
    current_branch, git, git_net, git_to_file, pr_state_dir, resolve_base, GhClient,
};
use regex::Regex;
use std::fs;
use std::path::Path;

/// Above this many changed lines a PR is refused outright.
const HARD_LIMIT: u64 = 1500;
/// Above this the PR needs written justification.
const ENFORCED_LIMIT: u64 = 1000;
/// Above this the PR is flagged as large.
const SOFT_LIMIT: u64 = 500;

/// Test-file name patterns, anchored with `(^|/)` because git prints
/// repo-relative paths with no leading slash.
const TEST_PATH_PATTERN: &str =
    r"(?i)(\.test\.|\.spec\.|_test\.|test_|(^|/)tests?/|(^|/)__tests__/)";
/// A Rust inline test: a new `mod tests` block or just a new `#[test]` fn.
const INLINE_TEST_PATTERN: &str = r"^\+.*#\[(cfg\(test\)|test)\]";

/// `Ok` carries both a normal prepare and the "a PR already exists" redirect
/// (both exit 0); `Err` is reserved for the hard stops.
pub fn run(
    gh: &dyn GhClient,
    base_arg: Option<&str>,
    ticket_arg: Option<&str>,
) -> Result<String, String> {
    let branch = current_branch()?;
    let state_dir = pr_state_dir(&branch)?;
    prepare(&state_dir, &branch, gh, base_arg, ticket_arg)
}

/// The injectable core of `run`: `state_dir` and `branch` are parameters so a
/// test can point it at a scratch directory.
pub fn prepare(
    state_dir: &Path,
    branch: &str,
    gh: &dyn GhClient,
    base_arg: Option<&str>,
    ticket_arg: Option<&str>,
) -> Result<String, String> {
    fs::create_dir_all(state_dir)
        .map_err(|e| format!("could not create {}: {e}", state_dir.display()))?;

    if let Some(pr) = gh.pr_view(branch)? {
        if pr.state == "OPEN" {
            return Ok(format!(
                "A PR already exists: {}\nUse /playbook:address-pr-comments or /playbook:quick-review instead.",
                pr.url
            ));
        }
    }

    let (base, source) = resolve_base(gh, base_arg)?;
    if branch == base {
        return Err(format!(
            "on the base branch ({base}); create a feature branch first"
        ));
    }
    // After the on-base check so an immediate abort never pays for a fetch.
    // A failed fetch is not fatal: the local ref may still be good enough.
    let _ = git_net(&["fetch", "origin", &base, "--quiet"]);

    let remote_base = format!("origin/{base}");
    let ahead = git(&["rev-list", "--count", &format!("{remote_base}..HEAD")])
        .map_err(|e| format!("could not compare against {remote_base}: {e}"))?
        .parse::<u64>()
        .unwrap_or(0);
    if ahead == 0 {
        return Err(format!(
            "nothing ahead of {base}; there is nothing to open a PR for"
        ));
    }

    let range = format!("{remote_base}...HEAD");
    let changed = changed_lines(&git(&["diff", "--shortstat", &range]).unwrap_or_default());
    if changed > HARD_LIMIT {
        return Err(format!(
            "{changed} changed lines is over the {HARD_LIMIT}-line hard size limit; split the work into smaller PRs"
        ));
    }

    let dirty = git(&["status", "--porcelain"])
        .map(|out| out.lines().filter(|l| !l.is_empty()).count())
        .unwrap_or(0);
    let tests = test_blocks_touched(&range, state_dir)?;

    let diff_file = state_dir.join("pr-diff.txt");
    git_to_file(&["diff", &range], &diff_file)?;
    let stat = git(&["diff", "--stat", &range]).unwrap_or_default();
    let log = git(&["log", &format!("{remote_base}..HEAD"), "--format=%h %s"]).unwrap_or_default();

    let out = vec![
        format!("branch={branch}"),
        format!("state_dir={}", state_dir.display()),
        format!("base={base} (source: {source})"),
        format!("commits_ahead={ahead} changed_lines={changed} dirty_files={dirty} test_files_touched={tests}"),
        dirty_verdict(dirty),
        size_verdict(changed),
        tests_verdict(tests),
        "=== diff stat ===".to_string(),
        stat,
        "=== commit log ===".to_string(),
        log,
        format!("diff_file={}", diff_file.display()),
        format!("ticket={}", ticket(branch, ticket_arg)),
    ];
    Ok(out.join("\n"))
}

/// Insertions plus deletions from a `git diff --shortstat` line, or 0.
fn changed_lines(shortstat: &str) -> u64 {
    shortstat
        .split(',')
        .filter(|part| part.contains("insertion") || part.contains("deletion"))
        .filter_map(|part| part.split_whitespace().next()?.parse::<u64>().ok())
        .sum()
}

/// Test files in the diff plus Rust inline test blocks added.
fn test_blocks_touched(range: &str, state_dir: &Path) -> Result<usize, String> {
    let paths = Regex::new(TEST_PATH_PATTERN).map_err(|e| e.to_string())?;
    let files = git(&["diff", "--name-only", range]).unwrap_or_default();
    let by_name = files.lines().filter(|f| paths.is_match(f)).count();

    // The Rust diff goes through a file: it can exceed what a pipe holds.
    let rs_diff = state_dir.join("pr-rs-diff.txt");
    git_to_file(&["diff", "-U0", range, "--", "*.rs"], &rs_diff)?;
    let inline = Regex::new(INLINE_TEST_PATTERN).map_err(|e| e.to_string())?;
    let by_inline = fs::read_to_string(&rs_diff)
        .unwrap_or_default()
        .lines()
        .filter(|l| inline.is_match(l))
        .count();
    Ok(by_name + by_inline)
}

fn dirty_verdict(dirty: usize) -> String {
    if dirty > 0 {
        format!("VERDICT dirty: WARN - {dirty} uncommitted file(s) will NOT be in the PR")
    } else {
        "VERDICT dirty: OK - nothing uncommitted".to_string()
    }
}

fn size_verdict(changed: u64) -> String {
    if changed > ENFORCED_LIMIT {
        format!("VERDICT size: OVER - {changed} lines is above the {ENFORCED_LIMIT}-line enforced limit and needs explicit justification in the PR body")
    } else if changed > SOFT_LIMIT {
        format!("VERDICT size: SOFT - {changed} lines is above the {SOFT_LIMIT}-line soft limit")
    } else {
        format!("VERDICT size: OK - {changed} lines")
    }
}

fn tests_verdict(tests: usize) -> String {
    if tests == 0 {
        "VERDICT tests: NONE - the diff adds no test files or inline test blocks; the readiness criteria expect tests for behaviour changes".to_string()
    } else {
        format!("VERDICT tests: OK - {tests} test file(s) or inline test block(s) touched")
    }
}

/// `--ticket` verbatim (it may be the literal `none`), else the first
/// `PROJECT-1234` token in the branch name, else empty.
fn ticket(branch: &str, ticket_arg: Option<&str>) -> String {
    if let Some(arg) = ticket_arg.filter(|t| !t.is_empty()) {
        return arg.to_string();
    }
    Regex::new(r"[A-Z][A-Z0-9]+-[0-9]+")
        .ok()
        .and_then(|re| re.find(branch).map(|m| m.as_str().to_string()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_lines_sums_insertions_and_deletions() {
        // Arrange
        let stat = " 3 files changed, 120 insertions(+), 30 deletions(-)";

        // Act / Assert
        assert_eq!(changed_lines(stat), 150);
        assert_eq!(changed_lines(" 1 file changed, 7 insertions(+)"), 7);
        assert_eq!(changed_lines(" 1 file changed, 4 deletions(-)"), 4);
        assert_eq!(changed_lines(""), 0);
    }
}
