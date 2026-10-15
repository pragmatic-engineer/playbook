// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Runs `scripts/release-gh.sh` (ADR 0039) against a fake `gh` that answers
//! from a table and logs every call.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn text_of(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A fake `gh` on PATH. Each line of `table` is `pattern<TAB>exit<TAB>output`;
/// the first pattern that matches the arguments as a shell glob answers,
/// on stdout, and also on stderr when the exit is not 0. Every call is
/// logged, and a `--input` file is copied next to the log.
struct FakeGh {
    dir: PathBuf,
}

impl FakeGh {
    fn new(table: &[(&str, i32, &str)]) -> FakeGh {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("fake-gh-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("bin")).expect("fake gh dir");
        let mut rows = String::new();
        for (pattern, code, output) in table {
            rows.push_str(&format!("{pattern}\t{code}\t{output}\n"));
        }
        fs::write(dir.join("table"), rows).expect("table");
        let gh = r#"#!/usr/bin/env bash
dir="$(cd "$(dirname "$0")/.." && pwd)"
args="$*"
printf '%s\n' "$args" >> "$dir/log"
prev=""
for a in "$@"; do
  if [ "$prev" = "--input" ]; then cp "$a" "$dir/input.json"; fi
  prev="$a"
done
while IFS=$'\t' read -r pattern code output; do
  # shellcheck disable=SC2053
  if [[ "$args" == $pattern ]]; then
    # Like gh, an answer goes to stdout, and an error body goes to stdout
    # with its message on stderr.
    if [ -n "$output" ]; then printf '%b\n' "$output"; fi
    if [ -n "$output" ] && [ "$code" != 0 ]; then printf '%b\n' "$output" >&2; fi
    exit "$code"
  fi
done < "$dir/table"
echo "fake gh: no answer for: $args" >&2
exit 99
"#;
        let path = dir.join("bin/gh");
        fs::write(&path, gh).expect("gh");
        Command::new("chmod")
            .args(["+x"])
            .arg(&path)
            .status()
            .expect("chmod");
        FakeGh { dir }
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new("bash")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/release-gh.sh"))
            .args(args)
            .env("PATH", path)
            .env("RELEASE_GH_POLL", "0")
            .current_dir(cwd)
            .output()
            .expect("bash")
    }

    /// Replaces the table, for a test that needs the fake's folder first.
    fn answer(&self, table: &[(&str, i32, &str)]) {
        let mut rows = String::new();
        for (pattern, code, output) in table {
            rows.push_str(&format!("{pattern}\t{code}\t{output}\n"));
        }
        fs::write(self.dir.join("table"), rows).expect("table");
    }

    fn log(&self) -> String {
        fs::read_to_string(self.dir.join("log")).unwrap_or_default()
    }
}

impl Drop for FakeGh {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// A git checkout under the fake's folder, as the caller of `pr` has, and
/// its HEAD.
fn checkout(gh: &FakeGh) -> (PathBuf, String) {
    let work = gh.dir.join("work");
    fs::create_dir_all(work.join("Formula")).expect("work");
    fs::write(work.join("Formula/pbtop.rb"), "class Pbtop\nend\n").expect("formula");
    fs::write(work.join("body.md"), "Body.\n").expect("body");
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .current_dir(&work)
            .output()
            .expect("git");
        assert!(out.status.success(), "{}", text_of(&out));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "base"]);
    let head = git(&["rev-parse", "HEAD"]);
    (work, head)
}

const PR_ARGS: [&str; 7] = [
    "pr",
    "o/tap",
    "release/v1.0.0",
    "chore(release): playbook 1.0.0",
    "body.md",
    ".",
    "Formula/pbtop.rb",
];

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const NOT_FOUND: &str = r#"{"message":"Not Found","status":"404"}"#;

#[test]
fn an_open_or_merged_pull_request_is_reused() {
    // Arrange
    let open = FakeGh::new(&[
        ("pr list * --state open *", 0, "12"),
        ("pr merge 12 --repo o/tap --auto --squash", 0, ""),
    ]);
    let merged = FakeGh::new(&[
        ("pr list * --state open *", 0, ""),
        ("pr list * --state merged *", 0, "7"),
    ]);

    // Act
    let reused = open.run(&open.dir, &PR_ARGS);
    let done = merged.run(&merged.dir, &PR_ARGS);

    // Assert
    assert_eq!(text_of(&reused), "pr=12\nstate=open\n");
    assert_eq!(text_of(&done), "pr=7\nstate=merged\n");
    assert!(
        open.log().contains("pr merge 12 --repo o/tap --auto"),
        "auto-merge is turned on again"
    );
    assert!(
        open.log().contains("--json number,isCrossRepository"),
        "{}",
        open.log()
    );
    assert!(!open.log().contains("graphql") && !merged.log().contains("graphql"));
}

#[test]
fn a_new_pull_request_gets_one_signed_commit_and_auto_merge() {
    // Arrange
    let gh = FakeGh::new(&[]);
    let (work, head) = checkout(&gh);
    gh.answer(&[
        ("pr list *", 0, ""),
        ("api repos/o/tap --jq .default_branch", 0, "main"),
        ("api repos/o/tap/git/ref/heads/main *", 0, &head),
        ("api repos/o/tap/git/ref/heads/release/v1.0.0", 1, NOT_FOUND),
        ("api -X POST repos/o/tap/git/refs *", 0, ""),
        ("api graphql *", 0, "def456"),
        ("api -X POST repos/o/tap/pulls *", 0, "34"),
        ("pr merge 34 --repo o/tap --auto --squash", 0, ""),
    ]);

    // Act
    let out = gh.run(&work, &PR_ARGS);
    let input = fs::read_to_string(gh.dir.join("input.json")).expect("the commit payload");

    // Assert
    assert!(out.status.success(), "{}", text_of(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "pr=34\nstate=open\n");
    assert!(
        input.contains(&format!("\"expectedHeadOid\": \"{head}\"")),
        "{input}"
    );
    assert!(input.contains("\"path\": \"Formula/pbtop.rb\""), "{input}");
    assert!(input.contains("Y2xhc3MgUGJ0b3AKZW5kCg=="), "{input}");
    assert!(
        input.contains("\"headline\": \"chore(release): playbook 1.0.0\""),
        "{input}"
    );
    assert!(
        gh.log()
            .contains(&format!("-f ref=refs/heads/release/v1.0.0 -f sha={head}")),
        "{}",
        gh.log()
    );
    assert!(!gh.log().contains("-X DELETE"));
}

#[test]
fn a_rerun_drops_a_stale_branch_and_falls_back_for_a_merge_queue() {
    // Arrange
    let gh = FakeGh::new(&[]);
    let (work, head) = checkout(&gh);
    gh.answer(&[
        ("pr list *", 0, ""),
        ("api repos/o/tap --jq .default_branch", 0, "main"),
        ("api repos/o/tap/git/ref/heads/main *", 0, &head),
        ("api repos/o/tap/git/ref/heads/release/v1.0.0", 0, "{}"),
        (
            "api -X DELETE repos/o/tap/git/refs/heads/release/v1.0.0",
            0,
            "",
        ),
        ("api -X POST repos/o/tap/git/refs *", 0, ""),
        ("api graphql *", 0, "def456"),
        ("api -X POST repos/o/tap/pulls *", 0, "34"),
        (
            "pr merge 34 --repo o/tap --auto --squash",
            1,
            "merge queue: no merge method",
        ),
        ("pr merge 34 --repo o/tap --auto", 0, ""),
    ]);

    // Act
    let out = gh.run(&work, &PR_ARGS);

    // Assert
    assert!(out.status.success(), "{}", text_of(&out));
    assert!(gh
        .log()
        .contains("api -X DELETE repos/o/tap/git/refs/heads/release/v1.0.0"));
    assert!(
        gh.log().ends_with("pr merge 34 --repo o/tap --auto\n"),
        "{}",
        gh.log()
    );
}

#[test]
fn a_pull_request_is_refused_when_main_moved_or_the_branch_is_not_a_release() {
    // Arrange
    let gh = FakeGh::new(&[]);
    let (work, head) = checkout(&gh);
    gh.answer(&[
        ("pr list *", 0, ""),
        ("api repos/o/tap --jq .default_branch", 0, "main"),
        ("api repos/o/tap/git/ref/heads/main *", 0, B),
    ]);
    let mut feature = PR_ARGS;
    feature[2] = "main";

    // Act
    let moved = gh.run(&work, &PR_ARGS);
    let wrong = gh.run(&work, &feature);

    // Assert
    assert!(!moved.status.success());
    assert!(
        text_of(&moved).contains(&format!("moved from {head} to {B}")),
        "{}",
        text_of(&moved)
    );
    assert!(
        !gh.log().contains("refs"),
        "nothing is written: {}",
        gh.log()
    );
    assert!(
        text_of(&wrong).contains("branch 'main' is not release/vX.Y.Z"),
        "{}",
        text_of(&wrong)
    );
}

#[test]
fn a_tag_is_created_once_and_never_moved() {
    // Arrange
    let lightweight = format!(r#"{{"object":{{"type":"commit","sha":"{A}"}}}}"#);
    let annotated = format!(r#"{{"object":{{"type":"tag","sha":"{B}"}}}}"#);
    let fresh = FakeGh::new(&[
        ("api repos/o/r/git/ref/tags/v1.0.0", 1, NOT_FOUND),
        ("api -X POST repos/o/r/git/refs *", 0, ""),
    ]);
    let same = FakeGh::new(&[("api repos/o/r/git/ref/tags/v1.0.0", 0, &lightweight)]);
    let signed = FakeGh::new(&[
        ("api repos/o/r/git/ref/tags/v1.0.0", 0, &annotated),
        (
            &format!("api repos/o/r/git/tags/{B} --jq .object.sha"),
            0,
            A,
        ),
    ]);
    let moved = FakeGh::new(&[(
        "api repos/o/r/git/ref/tags/v1.0.0",
        0,
        &lightweight.replace(A, B),
    )]);
    let broken = FakeGh::new(&[(
        "api repos/o/r/git/ref/tags/v1.0.0",
        1,
        r#"{"message":"Server Error","status":"502"}"#,
    )]);
    let refused = FakeGh::new(&[
        ("api repos/o/r/git/ref/tags/v1.0.0", 1, NOT_FOUND),
        (
            "api -X POST repos/o/r/git/refs *",
            1,
            "Repository rule violations found",
        ),
    ]);
    let args = ["tag", "o/r", "v1.0.0", A];

    // Act
    let created = fresh.run(&fresh.dir, &args);
    let exists = same.run(&same.dir, &args);
    let followed = signed.run(&signed.dir, &args);
    let elsewhere = moved.run(&moved.dir, &args);
    let unknown = broken.run(&broken.dir, &args);
    let blocked = refused.run(&refused.dir, &args);

    // Assert
    assert_eq!(text_of(&created), "tag=v1.0.0\nstate=created\n");
    assert!(
        fresh
            .log()
            .contains(&format!("-f ref=refs/tags/v1.0.0 -f sha={A}")),
        "{}",
        fresh.log()
    );
    assert_eq!(text_of(&exists), "tag=v1.0.0\nstate=exists\n");
    assert_eq!(text_of(&followed), "tag=v1.0.0\nstate=exists\n");
    assert!(text_of(&elsewhere).contains(&format!("points at {B}, not {A}")));
    assert!(!unknown.status.success());
    assert!(
        text_of(&unknown).contains("could not read tag v1.0.0"),
        "{}",
        text_of(&unknown)
    );
    assert!(!broken.log().contains("POST"));
    assert_eq!(blocked.status.code(), Some(3), "{}", text_of(&blocked));
    assert!(text_of(&blocked).contains("Repository rule violations"));
}

#[test]
fn a_release_is_created_only_when_missing() {
    // Arrange
    let exists = FakeGh::new(&[("release view v1.0.0 *", 0, "https://x/v1.0.0")]);
    let missing = FakeGh::new(&[
        ("release view *", 1, ""),
        ("release create *", 0, "https://x/new"),
    ]);
    let args = ["release", "o/r", "v1.0.0", "playbook 1.0.0", "notes.md"];

    // Act
    let kept = exists.run(&exists.dir, &args);
    let made = missing.run(&missing.dir, &args);

    // Assert
    assert_eq!(text_of(&kept), "url=https://x/v1.0.0\n");
    assert!(!exists.log().contains("release create"));
    assert!(missing
        .log()
        .contains("release create v1.0.0 --repo o/r --verify-tag --latest"));
    assert_eq!(text_of(&made), "url=https://x/new\n");
}

#[test]
fn waiting_reports_the_merge_commit_or_why_it_stopped() {
    // Arrange
    let merged = FakeGh::new(&[("pr view 34 *", 0, &format!("MERGED {A}"))]);
    let closed = FakeGh::new(&[("pr view 34 *", 0, "CLOSED ")]);
    let open = FakeGh::new(&[("pr view 34 *", 0, "OPEN ")]);
    let failing = FakeGh::new(&[("pr view 34 *", 1, "HTTP 502")]);

    // Act
    let done = merged.run(&merged.dir, &["wait", "o/r", "34", "60"]);
    let gone = closed.run(&closed.dir, &["wait", "o/r", "34", "60"]);
    let late = open.run(&open.dir, &["wait", "o/r", "34", "0"]);
    let flaky = failing.run(&failing.dir, &["wait", "o/r", "34", "0"]);
    let bad = open.run(&open.dir, &["wait", "o/r", "34", "1+1"]);

    // Assert
    assert_eq!(text_of(&done), format!("sha={A}\n"));
    assert!(
        text_of(&gone).contains("closed without a merge"),
        "{}",
        text_of(&gone)
    );
    assert!(
        text_of(&late).contains("did not merge within 0 seconds"),
        "{}",
        text_of(&late)
    );
    assert!(
        text_of(&flaky).contains("did not merge within 0 seconds"),
        "{}",
        text_of(&flaky)
    );
    assert_eq!(bad.status.code(), Some(2));
}
