// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Shared fixtures for the `pr prepare` / `pr create` integration tests: a
//! real work repo with a local bare `origin`, and a fake `GhClient`.

#![allow(dead_code)]

use playbook::pr::shared::{ExistingPr, GhClient};
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);
static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs `f` with the process cwd set to `dir`: the code under test runs git
/// in the cwd, so tests serialise on it like `tests/gate_record.rs`.
pub fn in_dir<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let prev = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(dir).expect("cd");
    let out = f();
    std::env::set_current_dir(prev).expect("restore cwd");
    out
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git should spawn");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A work repo on a feature branch whose `origin` is a local bare repo.
pub struct Fixture {
    pub work: PathBuf,
    pub bare: PathBuf,
    pub state: PathBuf,
}

impl Fixture {
    pub fn new(tag: &str, branch: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "playbook-pr-{tag}-{}-{n}",
            playbook::testing::run_id()
        ));
        fs::create_dir_all(&root).expect("root");
        let root = root.canonicalize().expect("canonicalize");
        let bare = root.join("origin.git");
        let work = root.join("work");
        fs::create_dir_all(&bare).expect("bare dir");
        fs::create_dir_all(&work).expect("work dir");
        let hooks = root.join("no-hooks");
        fs::create_dir_all(&hooks).expect("hooks dir");
        git(&bare, &["init", "--quiet", "--bare", "-b", "main"]);
        git(&work, &["init", "--quiet", "-b", "main"]);
        // Local config beats the global one the code under test also reads, so
        // a global hook or signing setting cannot reach these repos.
        for (k, v) in [
            ("user.name", "Test"),
            ("user.email", "test@example.test"),
            ("commit.gpgsign", "false"),
            ("push.gpgSign", "false"),
            ("core.hooksPath", hooks.to_str().expect("utf8")),
        ] {
            git(&work, &["config", k, v]);
        }
        git(
            &work,
            &["remote", "add", "origin", bare.to_str().expect("utf8")],
        );
        fs::write(work.join("README.md"), "base\n").expect("readme");
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "--quiet", "-m", "base"]);
        git(&work, &["push", "--quiet", "-u", "origin", "main"]);
        git(&work, &["checkout", "--quiet", "-b", branch]);
        Self {
            work,
            bare,
            state: root.join("state"),
        }
    }

    pub fn commit_file(&self, path: &str, content: &str) {
        let full = self.work.join(path);
        fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
        fs::write(&full, content).expect("write");
        git(&self.work, &["add", "-A"]);
        git(
            &self.work,
            &["commit", "--quiet", "-m", &format!("add {path}")],
        );
    }

    pub fn commit_lines(&self, path: &str, n: usize) {
        self.commit_file(path, &"line\n".repeat(n));
    }

    /// Publish `main`'s tip under another branch name and refresh the local
    /// remote-tracking refs, so `origin/<name>` resolves.
    pub fn publish_base_as(&self, name: &str) {
        git(
            &self.work,
            &[
                "push",
                "--quiet",
                "origin",
                &format!("main:refs/heads/{name}"),
            ],
        );
        git(&self.work, &["fetch", "--quiet", "origin"]);
    }

    /// Checks out a branch whose name starts with `-`, which `git branch`
    /// refuses to create but `update-ref` can.
    pub fn checkout_dash_branch(&self, name: &str) {
        git(
            &self.work,
            &["update-ref", &format!("refs/heads/{name}"), "HEAD"],
        );
        git(
            &self.work,
            &["symbolic-ref", "HEAD", &format!("refs/heads/{name}")],
        );
    }

    pub fn branch(&self) -> String {
        git(&self.work, &["branch", "--show-current"])
    }
}

/// A scripted `GhClient` that records every call it receives.
#[derive(Default)]
pub struct FakeGh {
    pub existing: Option<ExistingPr>,
    pub default_branch: Option<String>,
    pub created_url: String,
    /// What `pr_view_base` reports; `None` echoes the base the PR was created with.
    pub reported_base: Option<String>,
    /// Makes `pr_view_base` fail with this message.
    pub base_view_error: Option<String>,
    pub calls: RefCell<Vec<String>>,
    pub created_base: RefCell<String>,
    /// The `draft` flag the last `pr_create` call received.
    pub created_draft: std::cell::Cell<Option<bool>>,
}

impl FakeGh {
    pub fn creating(url: &str) -> Self {
        Self {
            created_url: url.to_string(),
            ..Self::default()
        }
    }

    pub fn count(&self, prefix: &str) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|c| c.starts_with(prefix))
            .count()
    }
}

impl GhClient for FakeGh {
    fn pr_view(&self, _branch: &str) -> Result<Option<ExistingPr>, String> {
        self.calls.borrow_mut().push("pr_view".into());
        Ok(self.existing.clone())
    }
    fn repo_default_branch(&self) -> Result<Option<String>, String> {
        self.calls.borrow_mut().push("repo_default_branch".into());
        Ok(self.default_branch.clone())
    }
    fn pr_create(
        &self,
        _title: &str,
        _body_file: &str,
        base: &str,
        draft: bool,
    ) -> Result<String, String> {
        self.calls.borrow_mut().push("pr_create".into());
        self.created_draft.set(Some(draft));
        *self.created_base.borrow_mut() = base.to_string();
        Ok(self.created_url.clone())
    }
    fn pr_view_base(&self, _branch: &str) -> Result<String, String> {
        self.calls.borrow_mut().push("pr_view_base".into());
        if let Some(err) = &self.base_view_error {
            return Err(err.clone());
        }
        Ok(self
            .reported_base
            .clone()
            .unwrap_or_else(|| self.created_base.borrow().clone()))
    }
    fn pr_edit_base(&self, branch: &str, base: &str) -> Result<(), String> {
        self.calls
            .borrow_mut()
            .push(format!("pr_edit_base:{branch}:{base}"));
        Ok(())
    }
}
