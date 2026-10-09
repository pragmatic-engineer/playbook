// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook commit prepare`: refuse the default branch, stage if asked,
//! format the staged files with the formatter the repo configures, and print
//! the context a commit message is drafted from.

use super::{current_branch, git_in, git_success, git_text};
use std::path::Path;
use std::process::{Command, Stdio};

/// What to stage and whether the last commit is being replaced.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub stage_all: bool,
    pub stage_update: bool,
    pub amend: bool,
}

/// The repo's default branch: the local `origin/HEAD`, else what `gh` says,
/// else `main`. The ref is read first and the fallback only runs when it was
/// empty, so a repo that never ran `git remote set-head origin -a` still gets
/// a real value.
pub fn default_branch(dir: &Path) -> String {
    if let Some(r) = git_text(dir, &["symbolic-ref", "refs/remotes/origin/HEAD"]) {
        if !r.is_empty() {
            return r.trim_start_matches("refs/remotes/origin/").to_string();
        }
    }
    crate::pr::shared::gh(&[
        "repo",
        "view",
        "--json",
        "defaultBranchRef",
        "-q",
        ".defaultBranchRef.name",
    ])
    .ok()
    .filter(|b| !b.is_empty())
    .unwrap_or_else(|| "main".to_string())
}

/// Whether `branch` is one this command never commits on.
pub fn is_protected(branch: &str, default: &str) -> bool {
    branch == default || branch == "main" || branch == "master"
}

/// The formatter command for the files, picked from the config file the repo
/// has: Biome, then dprint, then Prettier.
pub fn formatter(dir: &Path) -> Option<Vec<&'static str>> {
    let has = |names: &[&str]| names.iter().any(|n| dir.join(n).is_file());
    if has(&["biome.json", "biome.jsonc"]) {
        Some(vec!["npx", "biome", "check", "--write"])
    } else if has(&["dprint.json", "dprint.jsonc", ".dprint.json"]) {
        Some(vec!["dprint", "fmt"])
    } else if has(&[
        ".prettierrc",
        ".prettierrc.json",
        "prettier.config.js",
        "prettier.config.mjs",
    ]) {
        Some(vec!["npx", "prettier", "--write"])
    } else {
        None
    }
}

fn lines(text: Option<String>) -> Vec<String> {
    text.unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// The text the message drafter reads, or an error for the protected branch.
pub fn run(dir: &Path, opts: Options) -> Result<String, String> {
    let branch = current_branch(dir);
    let default = default_branch(dir);
    if is_protected(&branch, &default) {
        return Err(format!(
            "HEAD is on '{branch}', the repo's default/protected branch. This skill never commits there. Create a feature branch first (e.g. git checkout -b <name>) and re-run."
        ));
    }
    if opts.stage_all {
        git_success(dir, &["add", "-A"]);
    } else if opts.stage_update {
        git_success(dir, &["add", "-u"]);
    }

    let mut files = lines(git_text(
        dir,
        &["diff", "--staged", "--name-only", "--diff-filter=d"],
    ));
    if opts.amend {
        files.extend(lines(git_text(
            dir,
            &["diff", "--name-only", "--diff-filter=d", "HEAD~1", "HEAD"],
        )));
        files.sort();
        files.dedup();
    }
    if !files.is_empty() {
        if let Some(fmt) = formatter(dir) {
            // A formatter that is missing or fails must not stop the commit.
            let _ = Command::new(fmt[0])
                .args(&fmt[1..])
                .args(&files)
                .current_dir(dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = Command::new("git")
            .arg("add")
            .args(&files)
            .current_dir(dir)
            .stderr(Stdio::null())
            .status();
    }

    let nothing_staged = git_success(dir, &["diff", "--staged", "--quiet"]);
    if nothing_staged && !opts.amend {
        return Ok("NO_STAGED_CHANGES".to_string());
    }

    let mut out = format!("BRANCH={}\n", current_branch(dir));
    let diff = |args: &[&str]| {
        git_in(dir, args)
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    if opts.amend {
        let last = git_text(dir, &["log", "-1", "--oneline"]).unwrap_or_default();
        out.push_str(&format!("AMENDING: {last}\n"));
        out.push_str(&diff(&["--no-pager", "diff", "HEAD~1", "--name-status"]));
        out.push_str("---DIFF_START---\n");
        out.push_str(&diff(&["--no-pager", "diff", "HEAD~1...HEAD"]));
        out.push_str(&diff(&["--no-pager", "diff", "--staged"]));
    } else {
        out.push_str(&diff(&["--no-pager", "diff", "--staged", "--name-status"]));
        out.push_str("---DIFF_START---\n");
        out.push_str(&diff(&["--no-pager", "diff", "--staged"]));
    }
    Ok(out.trim_end().to_string())
}
