// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook segment`: the git recipes `/playbook:implement` runs for a
//! Segment. `branch` puts HEAD on the right branch for the delivery topology,
//! `size` measures a Segment against its base, and `resplit` cuts an
//! over-budget Segment at a Work Unit boundary. They live here so the
//! branch names, the base choice and the cut point are tested code, not
//! paragraphs the model re-derives each run.

use crate::review::size::parse_numstat;
use std::path::Path;
use std::process::Command;

/// Changed lines at which a Segment must be split. Segments target well under
/// this; it is the hard limit a pull request never crosses.
pub const HARD_LIMIT_LINES: u64 = 1500;

/// Longest the title part of a branch name gets.
const SLUG_MAX: usize = 40;

/// How the Segments of a plan are delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topology {
    Stacked,
    Independent,
    Single,
}

/// Lowercase, dash separated, at most `SLUG_MAX` characters, no edge dashes.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(SLUG_MAX);
    out.trim_matches('-').to_string()
}

/// `<type>/<plan-slug>-s<N><suffix>-<seg-slug>`; `suffix` is `b` for the
/// excess half of a re-split and empty otherwise.
pub fn branch_name(kind: &str, plan_slug: &str, n: u32, suffix: &str, title: &str) -> String {
    format!("{kind}/{plan_slug}-s{n}{suffix}-{}", slug(title))
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn ref_exists(dir: &Path, name: &str) -> bool {
    git(dir, &["rev-parse", "--verify", "--quiet", name]).is_ok()
}

/// Everything `branch` needs to know.
pub struct BranchArgs<'a> {
    pub kind: &'a str,
    pub plan_slug: &'a str,
    pub n: u32,
    pub title: &'a str,
    pub topology: Topology,
    pub default_branch: &'a str,
    pub prev_branch: Option<&'a str>,
    pub land: bool,
}

/// The ref a Segment branches from. Under `land` it is always the fetched
/// `origin/<default>`: the previous Segment's branch is already merged and may
/// be deleted, and the local default branch is stale. A stacked Segment past
/// the first branches off the previous Segment's branch, fetched when it is
/// not local (a resume after `/clear` or on a fresh checkout).
fn base_ref(dir: &Path, a: &BranchArgs<'_>) -> Result<String, String> {
    if a.land {
        git(dir, &["fetch", "origin", a.default_branch])?;
        return Ok(format!("origin/{}", a.default_branch));
    }
    match (a.topology, a.prev_branch) {
        (Topology::Stacked, Some(prev)) if a.n > 1 => {
            if ref_exists(dir, prev) {
                return Ok(prev.to_string());
            }
            git(dir, &["fetch", "origin", prev])?;
            Ok(format!("origin/{prev}"))
        }
        _ => Ok(a.default_branch.to_string()),
    }
}

/// Put HEAD on the Segment's branch and say where it started. Under the
/// single topology the plan shares one branch, so nothing is created and the
/// base is the current tip. Prints `branch=<name> base=<sha> base_ref=<ref>`.
pub fn branch(dir: &Path, a: &BranchArgs<'_>) -> Result<String, String> {
    if a.topology == Topology::Single && !a.land {
        let name = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])?;
        if name == a.default_branch {
            return Err(format!(
                "HEAD is on the default branch {name}; switch to the plan's branch first"
            ));
        }
        let sha = git(dir, &["rev-parse", "HEAD"])?;
        return Ok(format!("branch={name} base={sha} base_ref=HEAD"));
    }
    let name = branch_name(a.kind, a.plan_slug, a.n, "", a.title);
    if ref_exists(dir, &format!("refs/heads/{name}")) {
        return Err(format!(
            "branch {name} already exists; resume on it with git switch"
        ));
    }
    let base = base_ref(dir, a)?;
    let sha = git(dir, &["rev-parse", &base])?;
    git(dir, &["switch", "-c", &name, &sha])?;
    Ok(format!("branch={name} base={sha} base_ref={base}"))
}

/// Changed lines and files of `base...head`.
fn measure(dir: &Path, base: &str, head: &str) -> Result<(u64, u64), String> {
    let text = git(dir, &["diff", "--numstat", &format!("{base}...{head}")])?;
    Ok(parse_numstat(&text))
}

/// `lines=N files=M over=true|false` for `base...HEAD`.
pub fn size(dir: &Path, base: &str, limit: u64) -> Result<String, String> {
    let (lines, files) = measure(dir, base, "HEAD")?;
    Ok(format!(
        "lines={lines} files={files} over={}",
        lines > limit
    ))
}

/// The newest commit of `commits` (oldest first) whose cumulative diff from
/// `base` stays within `limit`, given each commit's cumulative line count.
pub fn pick_split(cumulative: &[(String, u64)], limit: u64) -> Option<&str> {
    cumulative
        .iter()
        .take_while(|(_, lines)| *lines <= limit)
        .last()
        .map(|(sha, _)| sha.as_str())
}

/// Cut a Segment that went over `limit` at the last commit that keeps it in
/// budget: the current branch is renamed to hold the whole run as `s<N>b`,
/// and the original name is recreated at the split point and checked out.
/// Prints `split=none` when the Segment fits, otherwise
/// `split=<sha> trimmed=<branch> excess=<branch>`.
pub fn resplit(dir: &Path, base: &str, a: &BranchArgs<'_>, limit: u64) -> Result<String, String> {
    let (total, _) = measure(dir, base, "HEAD")?;
    if total <= limit {
        return Ok("split=none".to_string());
    }
    let current = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let commits = git(
        dir,
        &[
            "rev-list",
            "--reverse",
            "--first-parent",
            &format!("{base}..HEAD"),
        ],
    )?;
    let mut cumulative = Vec::new();
    for sha in commits.lines() {
        cumulative.push((sha.to_string(), measure(dir, base, sha)?.0));
    }
    let Some(split) = pick_split(&cumulative, limit) else {
        return Err(format!(
            "the first commit alone is over {limit} changed lines, so the Segment cannot be split at a Work Unit boundary"
        ));
    };
    let excess = branch_name(a.kind, a.plan_slug, a.n, "b", a.title);
    git(dir, &["branch", "-m", &current, &excess])?;
    git(dir, &["switch", "-c", &current, split])?;
    Ok(format!("split={split} trimmed={current} excess={excess}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_lowercase_dashed_and_bounded() {
        assert_eq!(slug("Add the Parser (v2)!"), "add-the-parser-v2");
        assert_eq!(slug("  --Edge--  "), "edge");
        assert_eq!(slug(&"a".repeat(80)).len(), SLUG_MAX);
        assert_eq!(slug(&format!("{} b", "a".repeat(39))), "a".repeat(39));
    }

    #[test]
    fn branch_names_follow_the_plan_convention() {
        assert_eq!(
            branch_name("feat", "my-plan", 2, "", "Parser core"),
            "feat/my-plan-s2-parser-core"
        );
        assert_eq!(
            branch_name("feat", "my-plan", 2, "b", "Parser core"),
            "feat/my-plan-s2b-parser-core"
        );
    }

    #[test]
    fn the_split_is_the_last_commit_inside_the_budget() {
        let rows = vec![
            ("a".to_string(), 400),
            ("b".to_string(), 1400),
            ("c".to_string(), 1600),
        ];
        assert_eq!(pick_split(&rows, 1500), Some("b"));
        assert_eq!(pick_split(&rows, 1400), Some("b"));
        assert_eq!(pick_split(&rows, 399), None);
        assert_eq!(pick_split(&[], 1500), None);
    }
}
