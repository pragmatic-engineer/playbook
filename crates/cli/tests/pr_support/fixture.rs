// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Shared fixtures for the `pr prepare` / `pr create` integration tests: a
//! real work repo with a local bare `origin`. The fake `GhClient` is in
//! `fake_gh.rs` and the cwd guard in `cwd.rs`, so a test file includes only
//! what it uses.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

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
        Self { work }
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
    // Only the `pr create` and `pr prepare` tests call it; the rules test does not.
    #[allow(dead_code)]
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
}
