// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `deliver::run` against real temp repos with a bare local `origin` and a
//! fake `gh` script. Every child runs with a scratch HOME and no global git
//! configuration.

use playbook::deliver::{self, Options, Stage};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static N: AtomicU64 = AtomicU64::new(0);

const FAKE_GH: &str = r#"#!/bin/bash
d="$FAKE_GH_DIR"
echo "$*" >> "$d/log"
val() { shift_to="$1"; shift; while [ $# -gt 0 ]; do [ "$1" = "$shift_to" ] && { echo "$2"; return; }; shift; done; }
json() {
  printf '{"number":7,"url":"https://github.com/o/r/pull/7","isDraft":%s,"state":"%s","title":"%s","baseRefName":"%s"}' \
    "$(cat $d/draft)" "$(cat $d/state)" "$(cat $d/title)" "$(cat $d/base)"
}
case "$1 $2" in
  "pr list")
    case "$*" in
      *headRefName*) [ -f "$d/heads" ] && cat "$d/heads" || echo "[]";;
      *) [ -f "$d/title" ] && echo "[$(json)]" || echo "[]";;
    esac;;
  "pr create")
    val --title "$@" > "$d/title"; val --base "$@" > "$d/base"
    case "$*" in *--draft*) echo true > "$d/draft";; *) echo false > "$d/draft";; esac
    [ -f "$d/state" ] || echo OPEN > "$d/state"; cp "$(val --body-file "$@")" "$d/body"
    echo https://github.com/o/r/pull/7;;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#;

struct Rig {
    root: PathBuf,
    work: PathBuf,
    origin: PathBuf,
    fake: PathBuf,
}

fn sh(dir: &Path, env: &[(String, String)], program: &str, args: &[&str]) -> String {
    let mut c = Command::new(program);
    c.args(args).current_dir(dir);
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("spawn");
    assert!(
        out.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

impl Rig {
    fn new() -> Rig {
        let root = std::env::temp_dir().join(format!(
            "playbook-deliver-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&root);
        let (home, work, origin, fake) = (
            root.join("home"),
            root.join("work"),
            root.join("origin.git"),
            root.join("fake"),
        );
        for d in [&home, &work, &origin, &fake] {
            fs::create_dir_all(d).unwrap();
        }
        let gh = fake.join("gh");
        fs::write(&gh, FAKE_GH).unwrap();
        sh(&fake, &[], "chmod", &["+x", gh.to_str().unwrap()]);
        let rig = Rig {
            root,
            work,
            origin,
            fake,
        };
        let env = rig.env();
        sh(
            &rig.origin,
            &env,
            "git",
            &["init", "-q", "--bare", "-b", "main"],
        );
        sh(&rig.work, &env, "git", &["init", "-q", "-b", "main"]);
        sh(
            &rig.work,
            &env,
            "git",
            &["remote", "add", "origin", rig.origin.to_str().unwrap()],
        );
        rig.write("a.txt", "one\n");
        rig.git(&["add", "-A"]);
        rig.git(&["commit", "-q", "-m", "chore: seed"]);
        rig.git(&["push", "-q", "-u", "origin", "main"]);
        rig.git(&["remote", "set-head", "origin", "main"]);
        rig.git(&["checkout", "-q", "-b", "feat/x"]);
        rig
    }

    fn env(&self) -> Vec<(String, String)> {
        let v = |k: &str, s: &str| (k.to_string(), s.to_string());
        vec![
            v("HOME", self.root.join("home").to_str().unwrap()),
            v("GIT_CONFIG_GLOBAL", "/dev/null"),
            v("GIT_CONFIG_SYSTEM", "/dev/null"),
            v("GIT_AUTHOR_NAME", "t"),
            v("GIT_AUTHOR_EMAIL", "t@t.test"),
            v("GIT_COMMITTER_NAME", "t"),
            v("GIT_COMMITTER_EMAIL", "t@t.test"),
            v("FAKE_GH_DIR", self.fake.to_str().unwrap()),
        ]
    }

    fn git(&self, args: &[&str]) -> String {
        sh(&self.work, &self.env(), "git", args)
    }

    fn write(&self, name: &str, text: &str) {
        fs::write(self.work.join(name), text).unwrap();
    }

    fn opts(&self) -> Options {
        let mut o = Options::new(self.work.clone());
        o.gh = self.fake.join("gh").to_string_lossy().into_owned();
        o.env = self.env();
        o.poll = Duration::from_millis(1);
        o
    }

    fn gh_log(&self) -> String {
        fs::read_to_string(self.fake.join("log")).unwrap_or_default()
    }

    fn remote_head(&self, branch: &str) -> String {
        sh(
            &self.origin,
            &self.env(),
            "git",
            &["rev-parse", &format!("refs/heads/{branch}")],
        )
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn dirty(rig: &Rig) -> Options {
    rig.write("b.txt", "new\n");
    let mut o = rig.opts();
    o.stage = Stage::All;
    o.message = Some("feat(x): add b".into());
    o
}

#[test]
fn a_new_branch_is_committed_pushed_and_opened_as_a_draft_pr() {
    let rig = Rig::new();
    let opts = dirty(&rig);

    let report = deliver::run(&opts).expect("delivered");

    assert_eq!(report.pr_number, Some(7));
    assert_eq!(report.draft, Some(true));
    assert!(rig.gh_log().contains("--draft"));
    assert_eq!(rig.remote_head("feat/x"), rig.git(&["rev-parse", "HEAD"]));
    assert!(rig
        .gh_log()
        .contains("pr create --head feat/x --base main --title feat(x): add b"));
    assert!(rig
        .git(&["log", "-1", "--format=%B"])
        .contains("Signed-off-by:"));
}
