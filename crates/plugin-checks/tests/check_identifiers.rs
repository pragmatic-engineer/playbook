// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Runs `scripts/check-identifiers.sh` in throwaway git repositories: a clean
//! tree passes, a tracked file with a flagged string fails, and the same
//! string under `docs/adr/` or in an untracked file passes. The flagged
//! strings are built at run time, so this file never trips the check.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn script() -> PathBuf {
    PathBuf::from(env!("PLAYBOOK_REPO_ROOT")).join("scripts/check-identifiers.sh")
}

/// A scratch git repository, removed on drop.
struct Repo(PathBuf);

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Repo {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("check-ids-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch repo");
        let status = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .expect("git init");
        assert!(status.success());
        Self(dir)
    }

    /// Writes `text` to `rel`, and adds it to the index when `track` is set.
    fn write(&self, rel: &str, text: &str, track: bool) {
        let path = self.0.join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        fs::write(&path, text).expect("write");
        if track {
            let status = Command::new("git")
                .args(["add", rel])
                .current_dir(&self.0)
                .status()
                .expect("git add");
            assert!(status.success());
        }
    }

    fn run(&self) -> (bool, String) {
        run_in(&self.0)
    }
}

fn run_in(dir: &Path) -> (bool, String) {
    let out = Command::new("bash")
        .arg(script())
        .current_dir(dir)
        .output()
        .expect("run the script");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

/// The maintainer's username, built so this file does not contain it.
fn username() -> String {
    ["isa", "ntos"].concat()
}

#[test]
fn a_clean_tree_passes() {
    let repo = Repo::new();
    repo.write("README.md", "Nothing personal here.\n", true);
    let (ok, out) = repo.run();
    assert!(ok, "{out}");
    assert!(out.contains("PASS"), "{out}");
}

#[test]
fn each_flagged_string_in_a_tracked_file_fails() {
    let flagged = [
        format!("/Users/{}/x", username()),
        ["~/Work", "space/x"].concat(),
        ["acme.atlassian", ".net"].concat(),
        ["run r", "tk hook"].concat(),
    ];
    for text in flagged {
        let repo = Repo::new();
        repo.write("docs/guide.md", &format!("{text}\n"), true);
        let (ok, out) = repo.run();
        assert!(!ok, "{text}: {out}");
        assert!(out.contains("docs/guide.md:1:"), "{text}: {out}");
    }
}

#[test]
fn decision_records_and_untracked_files_are_skipped() {
    let repo = Repo::new();
    repo.write("docs/adr/0001-x.md", &format!("{}\n", username()), true);
    repo.write("notes.md", &format!("{}\n", username()), false);
    repo.write("README.md", "clean\n", true);
    let (ok, out) = repo.run();
    assert!(ok, "{out}");
}

#[test]
fn this_repository_passes() {
    let (ok, out) = run_in(Path::new(env!("PLAYBOOK_REPO_ROOT")));
    assert!(ok, "{out}");
}
