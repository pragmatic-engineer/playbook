// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `deliver::run` against real temp repos with a bare local `origin` and a
//! fake `gh` script. Every child runs with a scratch HOME and no global git
//! configuration.

use playbook::deliver::{self, exit, Options, Stage};
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
  "pr edit")
    t="$(val --title "$@")"; [ -n "$t" ] && echo "$t" > "$d/title"
    b="$(val --base "$@")"; [ -n "$b" ] && echo "$b" > "$d/base"
    f="$(val --body-file "$@")"; [ -n "$f" ] && cp "$f" "$d/body"; true;;
  "pr ready") case "$*" in *--undo*) echo true > "$d/draft";; *) echo false > "$d/draft";; esac;;
  "pr merge") [ -f "$d/noqueue" ] || touch "$d/queued";;
  "pr view")
    q=null; [ -f "$d/queued" ] && q='{"enabledAt":"x"}'
    printf '{"state":"%s","autoMergeRequest":%s}' "$(cat $d/state)" "$q";;
  "pr checks") [ -f "$d/checks" ] && cat "$d/checks" || echo "[]"; exit 8;;
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
    for k in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"] {
        c.env_remove(k);
    }
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

    fn fake_file(&self, name: &str, text: &str) {
        fs::write(self.fake.join(name), text).unwrap();
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

#[test]
fn ready_creates_a_non_draft_pr() {
    let rig = Rig::new();
    let mut opts = dirty(&rig);
    opts.ready = Some(true);

    let report = deliver::run(&opts).expect("delivered");

    assert_eq!(report.draft, Some(false));
    assert!(!rig.gh_log().contains("--draft"));
}

#[test]
fn the_default_branch_is_refused() {
    let rig = Rig::new();
    rig.git(&["checkout", "-q", "main"]);
    let mut opts = dirty(&rig);
    opts.protected = Vec::new();

    let err = deliver::run(&opts).expect_err("refused");

    assert_eq!(err.code, exit::REFUSED);
    assert!(err.message.contains("main"), "{}", err.message);
}

#[test]
fn a_detached_head_is_refused() {
    let rig = Rig::new();
    rig.git(&["checkout", "-q", "--detach"]);

    let err = deliver::run(&rig.opts()).expect_err("refused");

    assert!(err.message.contains("detached"), "got {}", err.message);
}

#[test]
fn a_rebased_branch_with_nothing_staged_is_pushed_with_a_lease() {
    let rig = Rig::new();
    deliver::run(&dirty(&rig)).expect("first delivery");
    let old_remote = rig.remote_head("feat/x");
    rig.git(&["checkout", "-q", "main"]);
    rig.write("c.txt", "main moved\n");
    rig.git(&["add", "-A"]);
    rig.git(&["commit", "-q", "-m", "chore: main moves"]);
    rig.git(&["push", "-q", "origin", "main"]);
    rig.git(&["checkout", "-q", "feat/x"]);

    let report = deliver::run(&rig.opts()).expect("push only");

    assert!(report.rebased && report.pushed);
    assert_ne!(rig.remote_head("feat/x"), old_remote);
    assert_eq!(rig.remote_head("feat/x"), rig.git(&["rev-parse", "HEAD"]));
}

#[test]
fn a_diverged_remote_without_our_rebase_is_refused() {
    let rig = Rig::new();
    deliver::run(&dirty(&rig)).expect("first delivery");
    rig.git(&["commit", "-q", "--amend", "-m", "feat(x): reworded"]);

    let err = deliver::run(&rig.opts()).expect_err("refused");

    assert_eq!(err.code, exit::REFUSED);
    assert!(err
        .message
        .contains("only force-pushes after its own rebase"));
}

#[test]
fn a_rebase_conflict_reports_files_and_leaves_the_tree_clean() {
    let rig = Rig::new();
    rig.write("a.txt", "mine\n");
    rig.git(&["commit", "-q", "-am", "feat: mine"]);
    rig.git(&["checkout", "-q", "main"]);
    rig.write("a.txt", "theirs\n");
    rig.git(&["commit", "-q", "-am", "fix: theirs"]);
    rig.git(&["push", "-q", "origin", "main"]);
    rig.git(&["checkout", "-q", "feat/x"]);

    let err = deliver::run(&rig.opts()).expect_err("conflict");

    assert_eq!(err.code, exit::CONFLICT);
    assert_eq!(err.details, vec!["a.txt".to_string()]);
    assert!(rig.git(&["status", "--porcelain"]).is_empty());
}

#[test]
fn a_long_non_conventional_title_is_refused_before_anything_changes() {
    let rig = Rig::new();
    let mut opts = dirty(&rig);
    opts.title = Some(format!("Add {}", "x".repeat(80)));

    let err = deliver::run(&opts).expect_err("refused");

    assert_eq!(err.code, exit::REFUSED);
    assert_eq!(err.details.len(), 2, "got {:?}", err.details);
    assert!(rig.git(&["status", "--porcelain"]).contains("b.txt"));
}

#[test]
fn an_em_dash_in_the_commit_message_is_refused() {
    let rig = Rig::new();
    let mut opts = dirty(&rig);
    opts.message = Some("feat: add b \u{2014} quickly".into());

    let err = deliver::run(&opts).expect_err("refused");

    assert!(
        err.details.iter().any(|d| d.contains("em dash")),
        "got {:?}",
        err.details
    );
}

#[test]
fn ai_attribution_in_the_pr_body_is_refused() {
    let rig = Rig::new();
    let mut opts = dirty(&rig);
    opts.body = Some("Summary\n\nGenerated with Claude Code".into());

    let err = deliver::run(&opts).expect_err("refused");

    assert!(
        err.details.iter().any(|d| d.contains("AI attribution")),
        "got {:?}",
        err.details
    );
}

#[test]
fn staged_content_without_a_message_is_refused() {
    let rig = Rig::new();
    let mut opts = dirty(&rig);
    opts.message = None;

    let err = deliver::run(&opts).expect_err("refused");

    assert!(err.message.contains("no commit message"));
}

#[test]
fn a_commit_is_signed_off_and_signed_only_when_signing_is_configured() {
    let rig = Rig::new();
    let mut opts = dirty(&rig);
    opts.dry_run = true;
    let plain = deliver::run(&opts).expect("plan");
    rig.git(&["config", "gpg.format", "ssh"]);
    rig.git(&["config", "user.signingkey", "/nonexistent.pub"]);

    let configured = deliver::run(&opts).expect("plan");

    assert!(plain.actions.iter().any(|a| a.contains("commit with -s (")));
    assert!(configured.actions.iter().any(|a| a.contains("-s -S")));
}

#[test]
fn a_stacked_branch_targets_its_parent_branch() {
    let rig = Rig::new();
    deliver::run(&dirty(&rig)).expect("parent");
    rig.git(&["checkout", "-q", "-b", "feat/y"]);
    rig.write("d.txt", "child\n");
    fs::remove_file(rig.fake.join("title")).unwrap();
    rig.fake_file(
        "heads",
        r#"[{"headRefName":"feat/x","baseRefName":"main"}]"#,
    );
    let mut opts = rig.opts();
    opts.stage = Stage::All;
    opts.message = Some("feat(y): add d".into());

    let report = deliver::run(&opts).expect("child");

    assert_eq!(report.base, "feat/x");
    assert!(rig.gh_log().contains("--base feat/x"));
}

#[test]
fn an_existing_pr_is_left_alone_unless_a_title_is_passed() {
    let rig = Rig::new();
    deliver::run(&dirty(&rig)).expect("create");
    rig.write("e.txt", "more\n");
    let mut opts = rig.opts();
    opts.stage = Stage::All;
    opts.message = Some("feat(x): add e".into());

    deliver::run(&opts).expect("second commit");
    let untouched = rig.gh_log().contains("pr edit");
    rig.write("f.txt", "more\n");
    opts.title = Some("feat(x): add b and more".into());
    deliver::run(&opts).expect("with title");

    assert!(!untouched);
    assert!(rig
        .gh_log()
        .contains("pr edit 7 --title feat(x): add b and more"));
}

#[test]
fn auto_merge_reports_queued_and_fails_loudly_when_it_is_not() {
    let rig = Rig::new();
    let mut opts = dirty(&rig);
    opts.ready = Some(true);
    opts.auto_merge = true;
    let report = deliver::run(&opts).expect("queued");
    rig.git(&["checkout", "-q", "main"]);
    rig.git(&["checkout", "-q", "-b", "feat/z"]);
    rig.write("z.txt", "z\n");
    rig.fake_file("noqueue", "");
    rig.fake_file("heads", "[]");
    fs::remove_file(rig.fake.join("title")).unwrap();
    fs::remove_file(rig.fake.join("queued")).unwrap();
    let mut second = rig.opts();
    second.stage = Stage::All;
    second.message = Some("feat(z): add z".into());
    second.ready = Some(true);
    second.auto_merge = true;

    let not_queued = deliver::run(&second).expect("report");

    assert!(report.auto_merge_queued);
    assert_eq!(not_queued.code, exit::NOT_QUEUED);
}

#[test]
fn auto_merge_on_a_draft_is_refused() {
    let rig = Rig::new();
    let mut opts = dirty(&rig);
    opts.auto_merge = true;

    let err = deliver::run(&opts).expect_err("refused");

    assert_eq!(err.code, exit::REFUSED);
    assert!(!rig.gh_log().contains("pr create"));
    assert!(!rig.gh_log().contains("pr merge"));
}

#[test]
fn wait_ends_merged_with_success() {
    let rig = Rig::new();
    let mut opts = dirty(&rig);
    opts.wait = Some(Duration::from_secs(5));
    rig.fake_file("state", "MERGED");

    let report = deliver::run(&opts).expect("report");

    assert_eq!((report.outcome.as_str(), report.code), ("merged", exit::OK));
}

#[test]
fn wait_names_the_failing_required_check() {
    let rig = Rig::new();
    let mut opts = dirty(&rig);
    opts.wait = Some(Duration::from_secs(5));
    rig.fake_file(
        "checks",
        r#"[{"name":"unit","bucket":"fail"},{"name":"lint","bucket":"pass"}]"#,
    );

    let report = deliver::run(&opts).expect("report");

    assert_eq!(report.code, exit::CHECKS);
    assert!(report.to_text().contains("Failing check: unit"));
}

#[test]
fn wait_times_out_while_checks_stay_pending() {
    let rig = Rig::new();
    let mut opts = dirty(&rig);
    opts.wait = Some(Duration::from_millis(20));
    rig.fake_file("checks", r#"[{"name":"unit","bucket":"pending"}]"#);

    let report = deliver::run(&opts).expect("report");

    assert_eq!(report.code, exit::TIMEOUT);
}

#[test]
fn dry_run_changes_nothing() {
    let rig = Rig::new();
    let mut opts = dirty(&rig);
    opts.dry_run = true;
    let head = rig.git(&["rev-parse", "HEAD"]);

    let report = deliver::run(&opts).expect("plan");

    assert!(report.dry_run && !report.actions.is_empty());
    assert_eq!(rig.git(&["rev-parse", "HEAD"]), head);
    assert!(rig.git(&["status", "--porcelain"]).contains("?? b.txt"));
    assert!(!rig.gh_log().contains("pr create"));
    assert!(sh(
        &rig.origin,
        &rig.env(),
        "git",
        &["branch", "--list", "feat/x"]
    )
    .is_empty());
}

#[test]
fn running_twice_leaves_one_pr_and_the_second_run_has_nothing_to_do() {
    let rig = Rig::new();
    deliver::run(&dirty(&rig)).expect("first");

    let err = deliver::run(&rig.opts()).expect_err("second");

    assert_eq!(err.code, exit::NOTHING);
    assert_eq!(rig.gh_log().matches("pr create").count(), 1);
}

#[test]
fn a_branch_with_no_commits_beyond_the_base_has_nothing_to_do() {
    let rig = Rig::new();

    let err = deliver::run(&rig.opts()).expect_err("nothing");

    assert_eq!(err.code, exit::NOTHING);
}

#[test]
fn a_diff_over_the_size_limit_is_refused_unless_allowed() {
    let rig = Rig::new();
    let mut opts = dirty(&rig);
    rig.write("big.txt", &"line\n".repeat(30));
    opts.size_limit = 20;

    let err = deliver::run(&opts).expect_err("too large");
    opts.allow_large = true;
    let allowed = deliver::run(&opts);

    assert_eq!(err.code, exit::REFUSED);
    assert!(allowed.is_ok());
}

#[test]
fn closing_lines_in_commits_survive_into_the_pr_body() {
    let rig = Rig::new();
    let mut opts = dirty(&rig);
    opts.message = Some("feat(x): add b\n\nCloses #41\nTicket: ABC-9".into());

    deliver::run(&opts).expect("delivered");

    let body = fs::read_to_string(rig.fake.join("body")).unwrap();
    assert!(
        body.contains("Closes #41") && body.contains("Ticket: ABC-9"),
        "got {body}"
    );
}

#[test]
fn a_remote_commit_we_never_had_is_not_overwritten_after_a_rebase() {
    let rig = Rig::new();
    deliver::run(&dirty(&rig)).expect("first delivery");
    let other = rig.root.join("other");
    let env = rig.env();
    let origin = rig.origin.to_str().unwrap();
    sh(&rig.root, &env, "git", &["clone", "-q", origin, "other"]);
    sh(&other, &env, "git", &["checkout", "-q", "feat/x"]);
    fs::write(other.join("theirs.txt"), "x\n").unwrap();
    sh(&other, &env, "git", &["add", "-A"]);
    sh(
        &other,
        &env,
        "git",
        &["commit", "-q", "-m", "feat(x): theirs"],
    );
    sh(&other, &env, "git", &["push", "-q", "origin", "feat/x"]);
    let theirs = rig.remote_head("feat/x");
    rig.git(&["checkout", "-q", "main"]);
    rig.write("c.txt", "main moved\n");
    rig.git(&["add", "-A"]);
    rig.git(&["commit", "-q", "-m", "chore: main moves"]);
    rig.git(&["push", "-q", "origin", "main"]);
    rig.git(&["checkout", "-q", "feat/x"]);

    let err = deliver::run(&rig.opts()).expect_err("refused");

    assert_eq!(err.code, exit::REFUSED);
    assert_eq!(rig.remote_head("feat/x"), theirs);
}

#[test]
fn the_default_branch_is_found_from_the_remote_when_origin_head_is_missing() {
    let rig = Rig::new();
    rig.git(&["push", "-q", "origin", "main:dev"]);
    sh(
        &rig.origin,
        &rig.env(),
        "git",
        &["symbolic-ref", "HEAD", "refs/heads/dev"],
    );
    rig.git(&["fetch", "-q", "origin"]);
    rig.git(&["remote", "set-head", "origin", "-d"]);
    rig.git(&["checkout", "-q", "-B", "dev", "origin/dev"]);
    let mut opts = dirty(&rig);
    opts.protected = Vec::new();
    let head = Command::new("git")
        .args(["symbolic-ref", "-q", "refs/remotes/origin/HEAD"])
        .current_dir(&rig.work)
        .envs(rig.env())
        .output()
        .unwrap();
    assert!(!head.status.success(), "origin/HEAD must be missing");

    let err = deliver::run(&opts).expect_err("refused");

    assert_eq!(err.code, exit::REFUSED);
    assert!(err.message.contains("dev"), "{}", err.message);
}

#[test]
fn auto_merge_on_a_pr_that_already_merged_reports_merged() {
    let rig = Rig::new();
    rig.fake_file("state", "MERGED");
    let mut opts = dirty(&rig);
    opts.ready = Some(true);
    opts.auto_merge = true;

    let report = deliver::run(&opts).expect("delivered");

    assert_eq!(report.outcome, "merged");
    assert_eq!(report.code, exit::OK);
}

#[test]
fn an_earlier_commit_with_ai_attribution_is_refused_before_the_push() {
    let rig = Rig::new();
    rig.write("b.txt", "new\n");
    rig.git(&["add", "-A"]);
    let trailer = format!("Co-Authored-By: {} <noreply@{}.com>", "Claude", "anthropic");
    rig.git(&[
        "commit",
        "-q",
        "-m",
        &format!("feat(x): add b\n\n{trailer}"),
    ]);

    let err = deliver::run(&rig.opts()).expect_err("refused");

    assert_eq!(err.code, exit::REFUSED);
    assert!(!rig.gh_log().contains("pr create"));
    assert!(rig.git(&["ls-remote", "origin", "feat/x"]).is_empty());
}
