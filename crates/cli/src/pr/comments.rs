// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook pr comments`: Steps 1 and 2 of `/playbook:address-pr-comments`.
//! It parses the flags, resolves the PR, and fetches the review threads and
//! the PR-level comments to two files, leaving the filtering to the model.

use super::shared::gh;
use std::fs;

/// The flags and the PR the arguments name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parsed {
    pub include_bots: bool,
    pub dry_run: bool,
    pub auto_commit: bool,
    pub pr: Option<String>,
    pub warnings: Vec<String>,
}

/// Split the argument string. `--bots`, `--dry-run`, `-y`/`--yes`, a PR number
/// with or without `#`; `--auto` and `--ask` are accepted and ignored; any
/// other word is reported and skipped. The last PR number wins.
pub fn parse(args: &str) -> Parsed {
    let mut p = Parsed::default();
    for tok in args.split_whitespace() {
        match tok {
            "--bots" => p.include_bots = true,
            "--dry-run" => p.dry_run = true,
            "-y" | "--yes" => p.auto_commit = true,
            "--auto" | "--ask" => {}
            _ => {
                let digits = tok.strip_prefix('#').unwrap_or(tok);
                if digits.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    p.pr = Some(tok.strip_prefix('#').unwrap_or(tok).to_string());
                } else {
                    p.warnings
                        .push(format!("warning: ignoring unknown arg '{tok}'"));
                }
            }
        }
    }
    p
}

const THREADS_QUERY: &str = r#"
  query($owner: String!, $name: String!, $pr: Int!) {
    repository(owner: $owner, name: $name) {
      pullRequest(number: $pr) {
        reviewThreads(first: 100) {
          nodes {
            id
            isResolved
            isOutdated
            path
            line
            originalLine
            comments(first: 50) {
              nodes {
                id
                databaseId
                author { login }
                body
                url
                createdAt
              }
            }
          }
        }
      }
    }
  }"#;

/// Resolve the PR, fetch both comment sources, and report. `Err` is the text
/// for stderr (exit 1).
pub fn run(args: &str) -> Result<String, String> {
    let parsed = parse(args);
    for w in &parsed.warnings {
        eprintln!("{w}");
    }
    let pr = match &parsed.pr {
        Some(n) => n.clone(),
        None => gh(&["pr", "view", "--json", "number", "-q", ".number"])
            .ok()
            .filter(|n| !n.is_empty())
            .ok_or("error: no PR for current branch; pass a PR number")?,
    };
    let repo = gh(&[
        "repo",
        "view",
        "--json",
        "nameWithOwner",
        "-q",
        ".nameWithOwner",
    ])?;
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| format!("unexpected repo name: {repo}"))?;
    let head = gh(&[
        "pr",
        "view",
        &pr,
        "--json",
        "headRefOid",
        "-q",
        ".headRefOid",
    ])?;
    let me = gh(&["api", "/user", "-q", ".login"])?;

    let threads = gh(&[
        "api",
        "graphql",
        "-f",
        &format!("query={THREADS_QUERY}"),
        "-F",
        &format!("owner={owner}"),
        "-F",
        &format!("name={name}"),
        "-F",
        &format!("pr={pr}"),
    ])?;
    let issues = gh(&[
        "api",
        &format!("/repos/{owner}/{name}/issues/{pr}/comments"),
        "--paginate",
    ])?;
    let threads_path = format!("/tmp/pr-comments-{pr}-threads.json");
    let issues_path = format!("/tmp/pr-comments-{pr}-issues.json");
    fs::write(&threads_path, threads)
        .map_err(|e| format!("could not write {threads_path}: {e}"))?;
    fs::write(&issues_path, issues).map_err(|e| format!("could not write {issues_path}: {e}"))?;

    Ok(format!(
        "PR: {repo}#{pr}\nHead SHA: {head}\nMe: {me}\nFlags: bots={} dry-run={} auto-commit={}\nThreads: {threads_path}\nIssue comments: {issues_path}",
        parsed.include_bots, parsed.dry_run, parsed.auto_commit
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_and_a_pr_number_parse() {
        let p = parse("#12 --bots --dry-run -y");
        assert_eq!(p.pr.as_deref(), Some("12"));
        assert!(p.include_bots && p.dry_run && p.auto_commit);
    }

    #[test]
    fn a_bare_number_and_the_run_mode_flags_parse() {
        let p = parse("--auto 7 --ask");
        assert_eq!(p.pr.as_deref(), Some("7"));
        assert!(!p.include_bots && !p.dry_run && !p.auto_commit);
    }

    #[test]
    fn an_unknown_word_is_reported_and_skipped() {
        let p = parse("--wat 7");
        assert_eq!(p.warnings, vec!["warning: ignoring unknown arg '--wat'"]);
        assert_eq!(p.pr.as_deref(), Some("7"));
    }

    #[test]
    fn no_pr_means_the_current_branch() {
        assert_eq!(parse("--bots").pr, None);
        assert_eq!(parse("").pr, None);
    }
}
