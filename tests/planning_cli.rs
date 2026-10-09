// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook plan checkpoint`, `glossary add` and `skill ref`: the helpers
//! that replaced bash blocks in `plan.md`, `implement.md` and `deep-review.md`.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

struct Sandbox {
    root: PathBuf,
    repo: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("pb-plan-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        for args in [
            vec!["init", "-q"],
            vec![
                "remote",
                "add",
                "origin",
                "https://github.com/acme/widgets.git",
            ],
        ] {
            let o = Command::new("git")
                .args(&args)
                .current_dir(&repo)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .unwrap();
            assert!(o.status.success());
        }
        Sandbox { root, repo }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_playbook"));
        c.args(args)
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("home/.config"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .current_dir(&self.repo);
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).output().unwrap()
    }

    fn run_stdin(&self, args: &[&str], input: &str) -> Output {
        let mut child = self
            .cmd(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn a_checkpoint_is_not_found_until_it_is_written_then_found_with_its_content() {
    let s = Sandbox::new("cp");
    let first = out(&s.run(&["plan", "checkpoint", "proj-123"]));
    let mut lines = first.lines();
    let path = PathBuf::from(lines.next().unwrap());
    assert!(path.ends_with("plans/proj-123.checkpoint.md"), "{first}");
    assert_eq!(lines.next(), Some("not-found"));

    let w = s.run_stdin(
        &["plan", "checkpoint", "proj-123", "--write"],
        "Goal: ship it\n",
    );
    assert!(w.status.success());
    assert_eq!(out(&w).trim(), path.display().to_string());
    assert_eq!(fs::read_to_string(&path).unwrap(), "Goal: ship it\n");

    let again = out(&s.run(&["plan", "checkpoint", "proj-123"]));
    assert!(
        again.ends_with("found\n") && !again.ends_with("not-found\n"),
        "{again}"
    );
}

#[test]
fn a_second_write_replaces_the_first() {
    let s = Sandbox::new("cp2");
    s.run_stdin(&["plan", "checkpoint", "t", "--write"], "one");
    let w = s.run_stdin(&["plan", "checkpoint", "t", "--write"], "two");
    assert_eq!(fs::read_to_string(out(&w).trim()).unwrap(), "two");
}

#[test]
fn a_slug_that_escapes_the_plans_folder_is_refused() {
    let s = Sandbox::new("cp3");
    let o = s.run(&["plan", "checkpoint", "../evil"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("invalid topic slug"));
}

#[test]
fn glossary_add_creates_the_file_then_appends() {
    let s = Sandbox::new("gl");
    s.run(&["glossary", "add", "- **Seat**: one licence"]);
    let o = s.run(&["glossary", "add", "- **Tenant**: one customer"]);
    assert!(o.status.success());
    let text = fs::read_to_string(s.repo.join("GLOSSARY.md")).unwrap();
    assert_eq!(
        text,
        "- **Seat**: one licence\n- **Tenant**: one customer\n"
    );
}

#[test]
fn skill_ref_prints_the_reference_or_falls_back_to_the_skill() {
    let s = Sandbox::new("ref");
    let root = env!("CARGO_MANIFEST_DIR");
    let a = out(&s.run(&[
        "skill",
        "ref",
        "grounding-review",
        "security",
        "--plugin-root",
        root,
    ]));
    assert!(
        a.trim()
            .ends_with("skills/grounding-review/references/security.md"),
        "{a}"
    );
    let b = out(&s.run(&[
        "skill",
        "ref",
        "grounding-review",
        "nope",
        "--plugin-root",
        root,
    ]));
    assert!(
        b.trim().ends_with("skills/grounding-review/SKILL.md"),
        "{b}"
    );
    let c = out(&s.run(&["skill", "ref", "grounding-review", "--plugin-root", root]));
    assert_eq!(b, c);
}
