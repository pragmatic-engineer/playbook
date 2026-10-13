// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `pr create`: everything after the title and body are written (validate,
//! push, confirm the push landed, open the PR, confirm its base).

use crate::pr::guard::{attribution_problems, dash_problems};
use crate::pr::shared::{current_branch, git, git_net, resolve_base, GhClient};
use std::path::Path;

/// A PR title is refused above this many characters, prefix included.
const TITLE_LIMIT: usize = 72;

/// `Err` naming both SHAs when the remote does not carry local HEAD.
fn verify_pushed_sha(local: &str, remote: &str) -> Result<(), String> {
    if local == remote {
        return Ok(());
    }
    let remote = if remote.is_empty() {
        "<missing>"
    } else {
        remote
    };
    Err(format!(
        "the remote branch is at {remote}, local HEAD is {local}; the push did not land"
    ))
}

/// Runs the attribution and dash checks over the title, the body file, and
/// every commit message about to be published. Never edits anything.
fn refuse_bad_text(title: &str, body_file: &str, base: &str) -> Result<(), String> {
    let body = std::fs::read_to_string(body_file)
        .map_err(|e| format!("could not read PR body file {body_file}: {e}"))?;
    let log = git(&[
        "log",
        &format!("origin/{base}..HEAD"),
        "--format=%h%x1f%B%x1e",
    ])?;
    let commits: Vec<(String, String)> = log
        .split('\x1e')
        .filter_map(|rec| rec.trim().split_once('\x1f'))
        .map(|(sha, msg)| (sha.to_string(), msg.to_string()))
        .collect();
    let mut problems = attribution_problems(title, &body, &commits);
    if !problems.is_empty() {
        problems.push(
            "fix: rewrite the title or body, or amend the named commit, before pushing".to_string(),
        );
    }
    let dashes = dash_problems(title, &body);
    if !dashes.is_empty() {
        problems.extend(dashes);
        problems.push("fix: replace each dash with a comma, colon, or parentheses".to_string());
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(format!(
        "refusing to push or create:\n{}",
        problems.join("\n")
    ))
}

/// Whether `pr.draft` says to open the PR as a draft; only a resolved `false`
/// opens it ready, so an unreadable or non-bool value stays a draft.
pub fn draft_setting(home: &Path, repo_slug: Option<&str>) -> bool {
    crate::config::resolve_valid("pr.draft", home, repo_slug)
        .ok()
        .and_then(|(value, _, _)| value.as_bool())
        .unwrap_or(true)
}

/// Pushes the current branch and opens a PR for it, as a draft when `draft`.
/// The push gates the create: a rejected push must never open a PR missing
/// the local commits.
pub fn run(
    gh: &dyn GhClient,
    title: &str,
    body_file: &str,
    base_arg: Option<&str>,
    draft: bool,
) -> Result<String, String> {
    let length = title.chars().count();
    if length > TITLE_LIMIT {
        return Err(format!(
            "PR title is {length} characters (limit {TITLE_LIMIT}); tighten it before creating the PR: {title}"
        ));
    }
    if !Path::new(body_file).is_file() {
        return Err(format!("PR body file {body_file} does not exist"));
    }

    let branch = current_branch()?;
    let (base, _) = resolve_base(gh, base_arg)?;
    if branch == base {
        return Err(format!(
            "on the base branch ({base}); create a feature branch first"
        ));
    }

    refuse_bad_text(title, body_file, &base)?;

    git_net(&["push", "-u", "origin", &format!("HEAD:refs/heads/{branch}")]).map_err(|e| {
        format!("push of {branch} failed; not creating a PR (it would be missing your local commits): {e}")
    })?;

    let local = git(&["rev-parse", "HEAD"])?;
    let remote = git_net(&["ls-remote", "origin", &format!("refs/heads/{branch}")])?
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    verify_pushed_sha(&local, &remote)?;

    // A re-run after a partial success (the PR was created but its base fix
    // failed, or gh timed out after creating it) reuses the open PR.
    let (url, summary) = match gh.pr_view(&branch)? {
        Some(pr) if pr.state == "OPEN" => (
            pr.url,
            format!("Using the already open PR ({branch} -> {base})"),
        ),
        _ => (
            gh.pr_create(title, body_file, &base, draft)?,
            format!(
                "Created {} PR: {title} ({branch} -> {base})",
                if draft { "draft" } else { "ready" }
            ),
        ),
    };

    // The create command's own success message is not proof of the base. The
    // PR exists by now, so a failed check is a warning that keeps the URL.
    let mut report = format!("PR: {url}\n{summary}");
    match gh.pr_view_base(&branch) {
        Ok(actual) if actual == base => {}
        Ok(_) => gh.pr_edit_base(&branch, &base).map_err(|e| {
            format!("PR {url} was opened against the wrong base and correcting it to {base} failed: {e}")
        })?,
        Err(e) => report.push_str(&format!(
            "\nWARN: could not verify the PR's base ({e}); check that it is {base}"
        )),
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    #[test]
    fn matching_shas_verify() {
        assert_eq!(verify_pushed_sha(A, A), Ok(()));
    }

    #[test]
    fn different_shas_fail_and_name_both() {
        // Act
        let err = verify_pushed_sha(A, B).expect_err("different SHAs");

        // Assert
        assert!(err.contains(A) && err.contains(B), "got {err}");
    }

    #[test]
    fn a_missing_remote_sha_fails_and_says_so() {
        // Act
        let err = verify_pushed_sha(A, "").expect_err("nothing on the remote");

        // Assert
        assert!(err.contains("<missing>") && err.contains(A), "got {err}");
    }
}
