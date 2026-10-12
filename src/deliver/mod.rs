// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `deliver`: commit, rebase, push and open or update a pull request from any
//! worktree, with no dependency on the session's working directory.
//!
//! The library imports only `common` helpers and its own adapter (`sys`), so
//! it can be extracted. The caller supplies the commit message, the PR title
//! and the PR body; this module checks them, never writes them.
//!
//! Exit codes: 0 delivered, 2 nothing to do, 3 refused (a safety or text
//! rule), 4 rebase conflict, 5 a git or gh call failed, 6 the PR closed or a
//! required check failed while waiting, 7 the wait timed out, 8 auto-merge
//! did not queue.

pub mod check;
pub mod pr;
pub mod sys;

use pr::{Check, Pr};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use sys::{Sys, Wait};

pub mod exit {
    pub const OK: i32 = 0;
    pub const NOTHING: i32 = 2;
    pub const REFUSED: i32 = 3;
    pub const CONFLICT: i32 = 4;
    pub const FAILED: i32 = 5;
    pub const CHECKS: i32 = 6;
    pub const TIMEOUT: i32 = 7;
    pub const NOT_QUEUED: i32 = 8;
}

/// What to stage before committing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    /// Only what is already staged (`--staged`, the default).
    Staged,
    /// Tracked files only (`-u`).
    Tracked,
    /// Everything, untracked files included (`-A`).
    All,
}

/// A check that runs before a commit; `Err` refuses it.
pub type BeforeCommit = fn(&Path) -> Result<(), String>;

/// Everything a delivery needs. Build with [`Options::new`] and set fields.
#[derive(Clone)]
pub struct Options {
    pub dir: PathBuf,
    pub stage: Stage,
    pub message: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub base: Option<String>,
    /// `Some(true)` for `--ready`, `Some(false)` for `--draft`.
    pub ready: Option<bool>,
    /// The `pr.draft` setting, used when `ready` is `None`.
    pub default_draft: bool,
    pub auto_merge: bool,
    pub wait: Option<Duration>,
    pub poll: Duration,
    pub dry_run: bool,
    pub allow_large: bool,
    pub size_limit: u64,
    /// Branch names refused besides the default branch. A trailing `/*` matches a prefix.
    pub protected: Vec<String>,
    pub git: String,
    pub gh: String,
    pub env: Vec<(String, String)>,
    /// Runs before a commit (secret and readiness checks); `Err` refuses.
    pub before_commit: Option<BeforeCommit>,
}

impl Options {
    pub fn new(dir: PathBuf) -> Self {
        Options {
            dir,
            stage: Stage::Staged,
            message: None,
            title: None,
            body: None,
            base: None,
            ready: None,
            default_draft: true,
            auto_merge: false,
            wait: None,
            poll: Duration::from_secs(15),
            dry_run: false,
            allow_large: false,
            size_limit: 1500,
            protected: ["main", "master", "develop", "trunk", "release/*"]
                .map(String::from)
                .to_vec(),
            git: "git".into(),
            gh: "gh".into(),
            env: Vec::new(),
            before_commit: None,
        }
    }
}

/// A delivery that stopped before the PR stage.
#[derive(Debug)]
pub struct Failure {
    pub code: i32,
    pub message: String,
    pub details: Vec<String>,
}

fn fail(code: i32, message: impl Into<String>) -> Failure {
    Failure {
        code,
        message: message.into(),
        details: Vec::new(),
    }
}

/// The result of a delivery.
#[derive(Debug, Serialize, Default)]
pub struct Report {
    pub branch: String,
    pub base: String,
    pub commits: Vec<String>,
    pub pr_number: Option<u64>,
    pub url: Option<String>,
    pub draft: Option<bool>,
    pub auto_merge_queued: bool,
    pub state: Option<String>,
    pub checks: Vec<Check>,
    pub rebased: bool,
    pub pushed: bool,
    pub dry_run: bool,
    pub actions: Vec<String>,
    pub outcome: String,
    #[serde(skip)]
    pub code: i32,
}

impl Report {
    pub fn to_json(&self) -> String {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        v["exit_code"] = self.code.into();
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let lead = if self.dry_run {
            "Plan (nothing changed)"
        } else {
            "Delivered"
        };
        out.push_str(&format!("{lead}: {} -> {}\n", self.branch, self.base));
        for action in &self.actions {
            out.push_str(&format!("  - {action}\n"));
        }
        if let Some(url) = &self.url {
            out.push_str(&format!("PR: {url}\n"));
        }
        if let Some(state) = &self.state {
            out.push_str(&format!("State: {state}\n"));
        }
        for c in self
            .checks
            .iter()
            .filter(|c| c.bucket == "fail" || c.bucket == "cancel")
        {
            out.push_str(&format!("Failing check: {}\n", c.name));
        }
        out.push_str(&format!("Outcome: {}\n", self.outcome));
        out
    }
}

fn lines_changed(numstat: &str) -> u64 {
    numstat
        .lines()
        .map(|l| {
            l.split('\t')
                .take(2)
                .map(|n| n.parse::<u64>().unwrap_or(0))
                .sum::<u64>()
        })
        .sum()
}

fn default_branch(sys: &Sys) -> String {
    if let Some(r) = sys.git_out(&["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
        if let Some(name) = r.strip_prefix("origin/") {
            return name.to_string();
        }
    }
    for name in ["main", "master"] {
        if sys
            .git(&[
                "rev-parse",
                "--verify",
                "-q",
                &format!("refs/remotes/origin/{name}"),
            ])
            .ok
        {
            return name.to_string();
        }
    }
    "main".to_string()
}

fn is_protected(branch: &str, default: &str, protected: &[String]) -> bool {
    branch == default
        || protected.iter().any(|p| match p.strip_suffix("/*") {
            Some(prefix) => branch.starts_with(&format!("{prefix}/")),
            None => p == branch,
        })
}

/// The nearest open PR branch that this branch is built on, else `None`.
fn parent_branch(sys: &Sys, branch: &str) -> Option<String> {
    let mut best: Option<(u64, String)> = None;
    for (head, _) in pr::open_heads(sys) {
        if head == branch {
            continue;
        }
        let remote = format!("refs/remotes/origin/{head}");
        if !sys
            .git(&["merge-base", "--is-ancestor", &remote, "HEAD"])
            .ok
        {
            continue;
        }
        let range = format!("{remote}..HEAD");
        let n = sys
            .git_out(&["rev-list", "--count", &range])
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(u64::MAX);
        if best.as_ref().is_none_or(|(m, _)| n < *m) {
            best = Some((n, head));
        }
    }
    best.map(|(_, head)| head)
}

fn signing_flag(sys: &Sys) -> bool {
    let set = |key: &str| {
        sys.git_out(&["config", "--get", key])
            .is_some_and(|v| !v.is_empty())
    };
    set("user.signingkey")
}

fn text_problems(opts: &Options) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(title) = &opts.title {
        out.extend(check::pr_title(title));
    }
    if let Some(body) = &opts.body {
        out.extend(check::pr_body(body));
    }
    out
}

fn refuse_all(problems: Vec<String>, what: &str) -> Failure {
    let mut f = fail(exit::REFUSED, format!("refusing: {what}"));
    f.details = problems;
    f
}

fn body_path(sys: &Sys) -> Result<PathBuf, Failure> {
    let git_dir = sys
        .git_out(&["rev-parse", "--absolute-git-dir"])
        .ok_or_else(|| fail(exit::FAILED, "could not find the git directory"))?;
    Ok(PathBuf::from(git_dir).join(format!("playbook-deliver-body-{}.md", std::process::id())))
}

/// Runs a delivery.
pub fn run(opts: &Options) -> Result<Report, Failure> {
    let mut sys = Sys {
        dir: opts.dir.clone(),
        git: opts.git.clone(),
        gh: opts.gh.clone(),
        env: opts.env.clone(),
    };
    // 1. Resolve and refuse.
    let top = sys
        .git_out(&["rev-parse", "--show-toplevel"])
        .ok_or_else(|| {
            fail(
                exit::REFUSED,
                format!("{} is not a git worktree", opts.dir.display()),
            )
        })?;
    sys.dir = PathBuf::from(top);
    if sys.git_out(&["remote", "get-url", "origin"]).is_none() {
        return Err(fail(exit::REFUSED, "the repository has no origin remote"));
    }
    let branch = sys
        .git_out(&["branch", "--show-current"])
        .unwrap_or_default();
    if branch.is_empty() {
        return Err(fail(
            exit::REFUSED,
            "detached HEAD; check out a branch first",
        ));
    }
    if branch.starts_with('-') {
        return Err(fail(exit::REFUSED, "the branch name looks like an option"));
    }
    let default = default_branch(&sys);
    if is_protected(&branch, &default, &opts.protected) {
        return Err(fail(
            exit::REFUSED,
            format!("{branch} is the default or a protected branch; deliver from a feature branch"),
        ));
    }
    let problems = text_problems(opts);
    if !problems.is_empty() {
        return Err(refuse_all(
            problems,
            "the PR title or body breaks a text rule",
        ));
    }

    let mut report = Report {
        branch: branch.clone(),
        dry_run: opts.dry_run,
        ..Report::default()
    };

    if let Some(message) = &opts.message {
        let problems = check::commit_message(message);
        if !problems.is_empty() {
            return Err(refuse_all(
                problems,
                "the commit message breaks a text rule",
            ));
        }
    }

    // 2. Staging.
    if !opts.dry_run {
        let flag = match opts.stage {
            Stage::All => Some("-A"),
            Stage::Tracked => Some("-u"),
            Stage::Staged => None,
        };
        if let Some(flag) = flag {
            let out = sys.git(&["add", flag]);
            if !out.ok {
                return Err(fail(
                    exit::FAILED,
                    format!("git add {flag} failed: {}", out.stderr),
                ));
            }
            report.actions.push(format!("staged with git add {flag}"));
        }
    }
    let has_staged = if opts.dry_run {
        let status = sys.git_out(&["status", "--porcelain"]).unwrap_or_default();
        let staged = !sys.git(&["diff", "--cached", "--quiet"]).ok;
        staged
            || match opts.stage {
                Stage::All => !status.is_empty(),
                Stage::Tracked => status.lines().any(|l| !l.starts_with("??")),
                Stage::Staged => false,
            }
    } else {
        !sys.git(&["diff", "--cached", "--quiet"]).ok
    };

    // 3. Remote state, the existing PR and the base.
    if !opts.dry_run {
        let out = sys.git_wait(&["fetch", "origin"], Wait::Net);
        if !out.ok {
            return Err(fail(
                exit::FAILED,
                format!("git fetch failed: {}", out.stderr),
            ));
        }
    }
    let existing = pr::find(&sys, &branch).map_err(|e| fail(exit::FAILED, e))?;
    let base = opts
        .base
        .clone()
        .or_else(|| {
            existing
                .as_ref()
                .map(|p| p.base.clone())
                .filter(|b| !b.is_empty())
        })
        .or_else(|| parent_branch(&sys, &branch))
        .unwrap_or_else(|| default.clone());
    if base.starts_with('-') {
        return Err(fail(
            exit::REFUSED,
            "the base branch name looks like an option",
        ));
    }
    let was_draft = existing.as_ref().map_or(opts.default_draft, |p| p.draft);
    if opts.auto_merge && opts.ready.map_or(was_draft, |r| !r) {
        return Err(fail(
            exit::REFUSED,
            "auto-merge needs a ready PR; pass --ready",
        ));
    }
    let base_ref = format!("refs/remotes/origin/{base}");
    if !sys.git(&["rev-parse", "--verify", "-q", &base_ref]).ok {
        return Err(fail(exit::FAILED, format!("origin/{base} does not exist")));
    }
    report.base = base.clone();

    // Size gate over the staged diff plus what the branch already carries.
    let cached = sys
        .git_out(&["diff", "--cached", "--numstat"])
        .unwrap_or_default();
    let range = format!("{base_ref}...HEAD");
    let committed = sys
        .git_out(&["diff", "--numstat", &range])
        .unwrap_or_default();
    let size = lines_changed(&cached) + lines_changed(&committed);
    if size > opts.size_limit && !opts.allow_large {
        return Err(fail(
            exit::REFUSED,
            format!(
                "the change is {size} lines, over the {} line limit; split it or pass --allow-large",
                opts.size_limit
            ),
        ));
    }

    // 4. Commit.
    let signed = signing_flag(&sys);
    if has_staged {
        let message = opts.message.as_deref().ok_or_else(|| {
            fail(
                exit::REFUSED,
                "something is staged but no commit message was given",
            )
        })?;
        if !opts.dry_run {
            if let Some(before) = opts.before_commit {
                before(&sys.dir).map_err(|e| fail(exit::REFUSED, e))?;
            }
            let mut args = vec!["commit", "-s"];
            if signed {
                args.push("-S");
            }
            args.extend(["-m", message]);
            let out = sys.git_wait(&args, Wait::Hooks);
            if !out.ok {
                return Err(fail(
                    exit::FAILED,
                    format!("git commit failed: {}{}", out.stderr, out.stdout),
                ));
            }
        }
        report.actions.push(format!(
            "commit with -s{} (never --no-verify)",
            if signed { " -S" } else { "" }
        ));
    } else if opts.message.is_some() {
        report
            .actions
            .push("commit message ignored, nothing is staged".into());
    }

    // 5. Rebase.
    let contains_base = sys
        .git(&["merge-base", "--is-ancestor", &base_ref, "HEAD"])
        .ok;
    let mut rebased = false;
    let mut before = String::new();
    if !contains_base {
        report.actions.push(format!("rebase onto origin/{base}"));
        if !opts.dry_run {
            before = sys.git_out(&["rev-parse", "HEAD"]).unwrap_or_default();
            let onto = format!("origin/{base}");
            let out = sys.git_wait(&["rebase", "--autostash", &onto], Wait::Hooks);
            if !out.ok {
                let files = sys
                    .git_out(&["diff", "--name-only", "--diff-filter=U"])
                    .unwrap_or_default();
                let aborted = sys.git(&["rebase", "--abort"]).ok;
                let note = if aborted {
                    "it was aborted and nothing was pushed"
                } else {
                    "the abort failed and the tree is still mid-rebase; run git rebase --abort"
                };
                let mut f = fail(
                    exit::CONFLICT,
                    format!("rebase onto origin/{base} conflicts; {note}. Resolve these files and run again"),
                );
                f.details = files.lines().map(String::from).collect();
                return Err(f);
            }
            rebased = sys.git_out(&["rev-parse", "HEAD"]).unwrap_or_default() != before;
        }
    }
    report.rebased = rebased;

    let ahead_range = format!("{base_ref}..HEAD");
    let commits: Vec<String> = sys
        .git_out(&["rev-list", "--reverse", &ahead_range])
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect();
    let would_commit = opts.dry_run && has_staged;
    if commits.is_empty() && !would_commit {
        return Err(fail(
            exit::NOTHING,
            format!("nothing to deliver: {branch} has no commits beyond origin/{base}"),
        ));
    }
    report.commits = commits.clone();

    // 6. Push.
    let head = sys.git_out(&["rev-parse", "HEAD"]).unwrap_or_default();
    let remote_ref = format!("refs/heads/{branch}");
    let remote = sys
        .git_wait(&["ls-remote", "origin", &remote_ref], Wait::Net)
        .stdout
        .split_whitespace()
        .next()
        .map(String::from);
    let refspec = format!("HEAD:{remote_ref}");
    // Force only over commits we already had before our own rebase.
    let lease_base = if rebased {
        before.clone()
    } else {
        "HEAD".to_string()
    };
    let push_args: Option<Vec<String>> = match &remote {
        None => Some(vec!["push".into(), "-u".into(), "origin".into(), refspec]),
        Some(r) if *r == head => None,
        Some(r) if sys.git(&["merge-base", "--is-ancestor", r, "HEAD"]).ok => {
            Some(vec!["push".into(), "origin".into(), refspec])
        }
        Some(r)
            if (rebased || (opts.dry_run && !contains_base))
                && sys.git(&["merge-base", "--is-ancestor", r, &lease_base]).ok =>
        {
            Some(vec![
                "push".into(),
                format!("--force-with-lease={remote_ref}:{r}"),
                "origin".into(),
                refspec,
            ])
        }
        Some(_) if opts.dry_run => Some(vec!["push".into(), "(refused: remote diverged)".into()]),
        Some(_) => {
            return Err(fail(
                exit::REFUSED,
                "the remote branch has commits this branch lacks; this command only force-pushes after its own rebase of commits it already had",
            ))
        }
    };
    let need_push = push_args.is_some() || would_commit;
    if let Some(args) = &push_args {
        report.actions.push(
            if args.iter().any(|a| a.starts_with("--force-with-lease")) {
                "push with --force-with-lease".to_string()
            } else {
                "push".to_string()
            },
        );
        if !opts.dry_run {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let out = sys.git_wait(&refs, Wait::Net);
            if !out.ok {
                return Err(fail(
                    exit::FAILED,
                    format!("git push failed: {}", out.stderr),
                ));
            }
            let landed = sys
                .git_wait(&["ls-remote", "origin", &remote_ref], Wait::Net)
                .stdout
                .split_whitespace()
                .next()
                .map(String::from);
            if landed.as_deref() != Some(head.as_str()) {
                return Err(fail(
                    exit::FAILED,
                    "the push did not land: the remote branch is not at HEAD",
                ));
            }
            report.pushed = true;
        }
    }

    // 7. Pull request.
    let nothing_else = existing.is_some()
        && opts.title.is_none()
        && opts.body.is_none()
        && opts.ready.is_none()
        && !opts.auto_merge
        && opts.wait.is_none()
        && opts.base.is_none();
    if nothing_else && !need_push && !has_staged {
        return Err(fail(
            exit::NOTHING,
            "nothing to do: the branch is pushed and its PR is up to date",
        ));
    }
    let draft_wanted = match opts.ready {
        Some(ready) => !ready,
        None => opts.default_draft,
    };
    let mut messages: Vec<String> = commits
        .iter()
        .filter_map(|c| sys.git_out(&["log", "-1", "--format=%B", c]))
        .collect();
    if would_commit {
        messages.extend(opts.message.clone());
    }
    let kept = pr::kept_lines(messages.iter().map(String::as_str));
    let mut pr_now: Option<Pr> = existing.clone();
    let body_file = body_path(&sys)?;

    match &existing {
        None => {
            let subjects: Vec<String> = messages
                .iter()
                .map(|m| m.lines().next().unwrap_or("").to_string())
                .collect();
            let title = match (&opts.title, subjects.as_slice()) {
                (Some(t), _) => t.clone(),
                (None, [only]) => only.clone(),
                _ => {
                    return Err(fail(
                        exit::REFUSED,
                        "several commits and no --title; pass a PR title",
                    ))
                }
            };
            let body = match &opts.body {
                Some(b) => pr::with_kept(b, &kept),
                None => pr::compose_body(&subjects, &messages, pr::template(&sys.dir).as_deref()),
            };
            let mut problems = check::pr_title(&title);
            problems.extend(check::pr_body(&body));
            if !problems.is_empty() {
                return Err(refuse_all(
                    problems,
                    "the PR title or body breaks a text rule",
                ));
            }
            report.actions.push(format!(
                "open a {} PR",
                if draft_wanted { "draft" } else { "ready" }
            ));
            if !opts.dry_run {
                std::fs::write(&body_file, &body).map_err(|e| fail(exit::FAILED, e.to_string()))?;
                let made = pr::create(&sys, &branch, &base, &title, &body_file, draft_wanted);
                let _ = std::fs::remove_file(&body_file);
                pr_now = Some(made.map_err(|e| fail(exit::FAILED, e))?);
            }
        }
        Some(found) => {
            let new_base = opts.base.as_deref().filter(|b| *b != found.base);
            let new_title = opts.title.as_deref().filter(|t| *t != found.title);
            if new_title.is_some() || opts.body.is_some() || new_base.is_some() {
                report.actions.push("update the PR".into());
                if !opts.dry_run {
                    let body = opts.body.as_deref().map(|b| pr::with_kept(b, &kept));
                    if let Some(body) = &body {
                        let problems = check::pr_body(body);
                        if !problems.is_empty() {
                            return Err(refuse_all(problems, "the PR body breaks a text rule"));
                        }
                        std::fs::write(&body_file, body)
                            .map_err(|e| fail(exit::FAILED, e.to_string()))?;
                    }
                    let edited = pr::edit(
                        &sys,
                        found.number,
                        new_title,
                        body.as_ref().map(|_| body_file.as_path()),
                        new_base,
                    );
                    let _ = std::fs::remove_file(&body_file);
                    edited.map_err(|e| fail(exit::FAILED, e))?;
                }
            }
            if let Some(ready) = opts.ready {
                if found.draft == ready {
                    report.actions.push(if ready {
                        "mark the PR ready".into()
                    } else {
                        "return the PR to draft".into()
                    });
                    if !opts.dry_run {
                        pr::set_ready(&sys, found.number, ready)
                            .map_err(|e| fail(exit::FAILED, e))?;
                    }
                }
            }
        }
    }

    report.pr_number = pr_now.as_ref().map(|p| p.number);
    report.url = pr_now.as_ref().map(|p| p.url.clone());
    report.draft = match (&existing, opts.ready) {
        (None, _) => Some(draft_wanted),
        (Some(_), Some(r)) => Some(!r),
        (Some(p), None) => Some(p.draft),
    };
    report.state = pr_now.as_ref().map(|p| p.state.clone());
    report.outcome = "delivered".into();

    if opts.dry_run {
        if opts.auto_merge {
            report.actions.push("queue auto-merge".into());
        }
        if opts.wait.is_some() {
            report
                .actions
                .push("wait for the PR to merge or its required checks to fail".into());
        }
        report.outcome = "dry-run".into();
        return Ok(report);
    }

    let Some(number) = pr_now.as_ref().map(|p| p.number) else {
        return Err(fail(exit::FAILED, "no pull request to act on"));
    };

    // 8. Auto-merge.
    if opts.auto_merge {
        if report.draft == Some(true) {
            report.outcome = "auto-merge-refused-draft".into();
            report.code = exit::REFUSED;
            report
                .actions
                .push("auto-merge needs a ready PR; pass --ready".into());
            return Ok(report);
        }
        pr::enable_auto_merge(&sys, number).map_err(|e| fail(exit::FAILED, e))?;
        let (state, queued) = pr::status(&sys, number).map_err(|e| fail(exit::FAILED, e))?;
        if state == "MERGED" {
            report.state = Some(state);
            report.outcome = "merged".into();
            return Ok(report);
        }
        report.auto_merge_queued = queued;
        report.actions.push(if queued {
            "auto-merge queued".into()
        } else {
            "auto-merge did not queue".into()
        });
        if !queued {
            report.outcome = "auto-merge-not-queued".into();
            report.code = exit::NOT_QUEUED;
            return Ok(report);
        }
    }

    // 9. Wait.
    if let Some(timeout) = opts.wait {
        let deadline = Instant::now() + timeout;
        loop {
            let (state, queued) = pr::status(&sys, number).map_err(|e| fail(exit::FAILED, e))?;
            report.state = Some(state.clone());
            report.auto_merge_queued = report.auto_merge_queued || queued;
            report.checks = pr::checks(&sys, number);
            if state == "MERGED" {
                report.outcome = "merged".into();
                return Ok(report);
            }
            if state == "CLOSED" {
                report.outcome = "closed".into();
                report.code = exit::CHECKS;
                return Ok(report);
            }
            if report
                .checks
                .iter()
                .any(|c| c.bucket == "fail" || c.bucket == "cancel")
            {
                report.outcome = "checks-failed".into();
                report.code = exit::CHECKS;
                return Ok(report);
            }
            if Instant::now() >= deadline {
                report.outcome = "timeout".into();
                report.code = exit::TIMEOUT;
                return Ok(report);
            }
            std::thread::sleep(
                opts.poll
                    .min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
    Ok(report)
}
