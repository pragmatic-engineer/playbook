// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Binary tests for `playbook code`. Each test gets a scratch git repo with a
//! fake `origin`, a scratch `HOME` and a scratch `PATH` that holds only git.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    repo: PathBuf,
    home: PathBuf,
    bin: PathBuf,
}

fn which_git() -> PathBuf {
    for dir in ["/usr/bin", "/opt/homebrew/bin", "/usr/local/bin", "/bin"] {
        let candidate = Path::new(dir).join("git");
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("git")
}

impl Fixture {
    fn new(tag: &str, with_origin: bool) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "playbook-code-cli-{tag}-{}-{n}",
            playbook::testing::run_id()
        ));
        let repo = base.join("repo");
        let home = base.join("home");
        let bin = base.join("bin");
        for dir in [&repo, &home, &bin] {
            fs::create_dir_all(dir).expect("scratch dir");
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(which_git(), bin.join("git")).expect("git link");
        let fx = Self {
            repo: repo.canonicalize().expect("repo"),
            home,
            bin,
        };
        fx.git(&["init", "--quiet"]);
        if with_origin {
            fx.git(&[
                "remote",
                "add",
                "origin",
                "https://github.com/test-owner/test-repo.git",
            ]);
        }
        fx
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new(which_git())
            .arg("-C")
            .arg(&self.repo)
            .args(args)
            .env_clear()
            .env("HOME", &self.home)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?}");
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.repo.join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        fs::write(path, text).expect("write");
    }

    fn playbook(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(args)
            .current_dir(&self.repo)
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", &self.bin)
            .output()
            .expect("playbook runs")
    }
}

fn json_of(out: &Output) -> serde_json::Value {
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(text.trim())
        .unwrap_or_else(|e| panic!("not one JSON document ({e}): {text}"))
}

#[test]
fn index_prints_the_envelope_and_a_second_run_is_all_unchanged() {
    // Arrange
    let fx = Fixture::new("envelope", true);
    fx.write("a.rs", "fn a() {}\n");
    fx.write("b.ts", "export const b = 1;\n");
    fx.write("c.py", "def c():\n    pass\n");

    // Act
    let first = fx.playbook(&["code", "index", "--json"]);
    let second = fx.playbook(&["code", "index", "--json"]);
    let rebuilt = fx.playbook(&["code", "index", "--json", "--rebuild"]);

    // Assert
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first = json_of(&first);
    assert_eq!(first["version"], 1);
    assert_eq!(first["command"], "index");
    assert_eq!(first["results"]["indexed"], 3);
    assert_eq!(first["results"]["changed"], 3);
    let second = json_of(&second);
    assert_eq!(second["results"]["unchanged"], 3);
    assert_eq!(second["results"]["changed"], 0);
    let rebuilt = json_of(&rebuilt);
    assert_eq!(rebuilt["results"]["indexed"], 3);
    assert_eq!(rebuilt["results"]["changed"], 3);
}

#[test]
fn index_in_text_mode_prints_a_one_line_summary() {
    // Arrange
    let fx = Fixture::new("text", true);
    fx.write("a.rs", "fn a() {}\n");

    // Act
    let out = fx.playbook(&["code", "index"]);

    // Assert
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.starts_with("indexed 1 files: 1 changed"), "{text}");
}

#[test]
fn a_repo_without_origin_exits_one_and_names_the_missing_remote() {
    // Arrange
    let fx = Fixture::new("no-origin", false);
    fx.write("a.rs", "fn a() {}\n");

    // Act
    let out = fx.playbook(&["code", "index"]);

    // Assert
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(err.contains("origin"), "{err}");
}

#[test]
fn the_index_lands_under_the_home_state_dir_and_not_in_the_repo() {
    // Arrange
    let fx = Fixture::new("location", true);
    fx.write("a.rs", "fn a() {}\n");

    // Act
    let out = fx.playbook(&["code", "index", "--json"]);

    // Assert
    let doc = json_of(&out);
    let path = doc["index"]["path"].as_str().expect("path").to_string();
    assert!(
        path.starts_with(fx.home.to_str().expect("utf8 home")),
        "{path}"
    );
    assert!(path.contains("test-owner/test-repo"), "{path}");
    assert!(Path::new(&path).is_file());
    let listing = fs::read_dir(&fx.repo)
        .expect("repo")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(!listing.iter().any(|n| n == ".playbook"), "{listing:?}");
}
