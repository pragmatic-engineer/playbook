// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `pr create`: everything after the title and body are written (validate,
//! push, confirm the push landed, open the draft PR, confirm its base).

use crate::pr::shared::{current_branch, git, git_net, resolve_base, GhClient};
use std::path::Path;

/// A PR title is refused above this many characters, prefix included.
const TITLE_LIMIT: usize = 72;

/// `Err` naming both SHAs when the remote does not carry local HEAD.
pub fn verify_pushed_sha(local: &str, remote: &str) -> Result<(), String> {
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

/// Pushes the current branch and opens a draft PR for it. The push gates the
/// create: a rejected push must never open a PR missing the local commits.
pub fn run(
    gh: &dyn GhClient,
    title: &str,
    body_file: &str,
    base_arg: Option<&str>,
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

    let url = gh.pr_create(title, body_file, &base)?;

    // The create command's own success message is not proof of the base.
    if gh.pr_view_base(&branch)? != base {
        gh.pr_edit_base(&branch, &base)?;
    }

    Ok(format!(
        "PR: {url}\nCreated draft PR: {title} ({branch} -> {base})"
    ))
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
