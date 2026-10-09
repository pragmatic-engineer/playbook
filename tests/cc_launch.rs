// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! End-to-end tests for `playbook cc launch` and `playbook shell-init`, with a
//! fake `claude` on PATH that records its argv. They replace the shell launcher
//! suites that covered `dispatch.sh`, `config-drift.sh` and the bash/zsh parity.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);
const SID: &str = "11111111-2222-3333-4444-555555555555";

struct Env {
    root: PathBuf,
    home: PathBuf,
    work: PathBuf,
}

fn make_exec(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn env(tag: &str) -> Env {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pb-launch-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let root = fs::canonicalize(&dir).unwrap();
    let home = root.join("home");
    let work = root.join("proj");
    let bin = root.join("bin");
    for d in [&home, &work, &bin] {
        fs::create_dir_all(d).unwrap();
    }
    make_exec(
        &bin.join("claude"),
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$FAKE_LOG\"\nprintf '%s\\n' \"$PWD\" >> \"$FAKE_LOG.pwd\"\nexit ${FAKE_EXIT:-0}\n",
    );
    Env { root, home, work }
}

impl Env {
    fn log(&self) -> PathBuf {
        self.root.join("claude.log")
    }
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }
    fn path(&self) -> String {
        format!(
            "{}:{}",
            self.root.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }
    fn launch(&self, args: &[&str]) -> Output {
        self.launch_with(&["--"], args, &[])
    }
    fn launch_with(&self, pre: &[&str], args: &[&str], extra: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_playbook"));
        cmd.args(["cc", "launch"])
            .args(pre)
            .args(args)
            .current_dir(&self.work)
            .env("PWD", &self.work)
            .env("HOME", &self.home)
            .env("PATH", self.path())
            .env("FAKE_LOG", self.log())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null");
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output().expect("run playbook")
    }
    /// Writes a transcript titled `proj` for the work dir.
    fn session(&self) -> PathBuf {
        let slug: String = self
            .work
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let dir = self.home.join(".claude/projects").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(format!("{SID}.jsonl")),
            format!("{{\"customTitle\":\"proj\",\"sessionId\":\"{SID}\"}}\n"),
        )
        .unwrap();
        dir
    }
}

#[test]
fn no_session_starts_fresh_with_the_dir_name() {
    let e = env("fresh-default");
    let out = e.launch(&[]);
    assert!(out.status.success());
    assert_eq!(e.calls(), vec!["-n proj"]);
}

#[test]
fn an_existing_titled_session_is_resumed_and_forks_only_on_drift() {
    let e = env("resume");
    e.session();
    e.launch(&[]);
    e.launch(&[]);
    let calls = e.calls();
    assert!(
        calls[0].starts_with(&format!("-n proj --resume {SID}")),
        "{calls:?}"
    );
    // The first launch had no baseline and re-stamped, so the second is quiet.
    assert_eq!(calls[1], format!("-n proj --resume {SID}"));
}

#[test]
fn config_drift_forks_the_resume_once_per_change() {
    let e = env("drift");
    e.session();
    let hash_script = e.home.join(".config/playbook/hooks/lib/config-hash.sh");
    fs::create_dir_all(hash_script.parent().unwrap()).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("hooks/lib/config-hash.sh"),
        &hash_script,
    )
    .unwrap();
    let settings = e.home.join(".claude/settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, "{\"a\":1}").unwrap();

    e.launch(&[]);
    e.launch(&[]);
    fs::write(&settings, "{\"a\":2}").unwrap();
    e.launch(&[]);

    let calls = e.calls();
    assert_eq!(calls[0], format!("-n proj --resume {SID} --fork-session"));
    assert_eq!(calls[1], format!("-n proj --resume {SID}"));
    assert_eq!(calls[2], format!("-n proj --resume {SID} --fork-session"));
}

#[test]
fn launching_trusts_the_dir_but_listing_does_not() {
    let e = env("trust");
    let claude_json = e.home.join(".claude.json");
    fs::write(&claude_json, "{\"projects\":{}}").unwrap();

    e.launch(&["list"]);
    assert!(!fs::read_to_string(&claude_json)
        .unwrap()
        .contains("hasTrustDialogAccepted"));
    e.launch(&["fresh"]);

    let doc: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&claude_json).unwrap()).unwrap();
    assert_eq!(
        doc["projects"][e.work.to_str().unwrap()]["hasTrustDialogAccepted"],
        true
    );
}

#[test]
fn claude_does_not_inherit_the_cd_file_variable() {
    let e = env("cd-env");
    make_exec(
        &e.root.join("bin/claude"),
        "#!/bin/sh\nprintf '%s\\n' \"${PLAYBOOK_CC_CD_FILE:-unset}\" >> \"$FAKE_LOG\"\n",
    );
    e.launch_with(&["--"], &[], &[("PLAYBOOK_CC_CD_FILE", "/tmp/x")]);
    assert_eq!(e.calls(), vec!["unset"]);
}

#[test]
fn a_value_flag_is_not_taken_for_the_subcommand() {
    let e = env("flag-value");
    e.session();
    e.launch(&["--model", "list", "hello"]);
    let call = &e.calls()[0];
    assert!(call.starts_with("--model list -n proj --resume"), "{call}");
    assert!(call.ends_with("hello"), "{call}");
}

#[test]
fn fresh_skips_resume_and_raw_resumes_without_forking() {
    let e = env("fresh-raw");
    e.session();
    e.launch(&["fresh", "--x"]);
    e.launch(&["raw"]);
    e.launch(&["raw", "abc"]);
    assert_eq!(
        e.calls(),
        vec![
            "-n proj --x".to_string(),
            format!("--resume {SID} -n proj"),
            "--resume abc -n proj".to_string()
        ]
    );
}

#[test]
fn clean_resumes_a_clone_and_leaves_the_original() {
    let e = env("clean");
    let dir = e.session();
    let out = e.launch(&["clean"]);
    let call = &e.calls()[0];
    assert!(
        call.starts_with("--resume ") && call.ends_with(" -n proj"),
        "{call}"
    );
    assert!(!call.contains(SID));
    assert!(String::from_utf8_lossy(&out.stdout).contains("ccc clean: cloned"));
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
}

#[test]
fn a_missing_conversation_falls_back_to_a_fresh_session() {
    let e = env("not-found");
    e.session();
    make_exec(
        &e.root.join("bin/claude"),
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$FAKE_LOG\"\ncase \"$*\" in *--resume*) echo 'No conversation found' >&2; exit 1;; esac\n",
    );
    let out = e.launch(&[]);
    assert!(out.status.success());
    assert_eq!(e.calls().last().unwrap(), "-n proj");
}

#[test]
fn skip_permissions_and_the_system_prompt_lead_the_command_line() {
    let e = env("flags");
    let prompt = e.home.join(".config/playbook/prompts/SYSTEM_PROMPT.md");
    fs::create_dir_all(prompt.parent().unwrap()).unwrap();
    fs::write(&prompt, "p").unwrap();
    let out = e.launch_with(&["--skip-permissions", "--"], &["fresh"], &[]);
    assert!(out.status.success());
    assert_eq!(
        e.calls()[0],
        format!(
            "--dangerously-skip-permissions --system-prompt-file {} -n proj",
            prompt.display()
        )
    );
}

#[test]
fn the_claude_exit_code_is_the_launchers_exit_code() {
    let e = env("exit");
    let out = e.launch_with(&["--"], &[], &[("FAKE_EXIT", "7")]);
    assert_eq!(out.status.code(), Some(7));
}

#[test]
fn list_launches_nothing() {
    let e = env("list");
    e.session();
    let out = e.launch(&["list"]);
    assert!(out.status.success());
    assert!(e.calls().is_empty());
    assert!(String::from_utf8_lossy(&out.stdout).contains(&SID[..8]));
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

#[test]
fn worktree_launches_inside_the_new_tree_and_records_the_cd_target() {
    let e = env("worktree");
    for args in [
        vec!["init", "-q", "-b", "master"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "T"],
    ] {
        git(&e.work, &args);
    }
    fs::write(e.work.join("f"), "x").unwrap();
    git(&e.work, &["add", "f"]);
    git(&e.work, &["commit", "-q", "-m", "init"]);
    // A protected branch needs no `origin`, keeping the test hermetic.
    git(&e.work, &["branch", "develop"]);
    let cd = e.root.join("cd-file");

    let out = e.launch_with(
        &["--skip-permissions", "--"],
        &["worktree", "develop"],
        &[
            ("PLAYBOOK_CC_CD_FILE", cd.to_str().unwrap()),
            ("WORKTREE_NO_PUSH", "1"),
        ],
    );

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let target = fs::read_to_string(&cd).expect("cd file written");
    let target = target.trim();
    assert!(target.ends_with("develop"), "{target}");
    assert!(Path::new(target).is_dir());
    let pwd = fs::read_to_string(format!("{}.pwd", e.log().display())).unwrap();
    assert_eq!(pwd.trim(), target);
    assert_eq!(e.calls()[0], "--dangerously-skip-permissions -n develop");
}

#[test]
fn worktree_outside_a_repo_fails_without_launching() {
    let e = env("worktree-norepo");
    let cd = e.root.join("cd-file");
    let out = e.launch_with(
        &["--"],
        &["worktree", "x"],
        &[("PLAYBOOK_CC_CD_FILE", cd.to_str().unwrap())],
    );
    assert_eq!(out.status.code(), Some(10));
    assert!(e.calls().is_empty());
    assert!(!cd.exists());
}

fn shell_init(shell: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["shell-init", "--shell", shell])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn shell_init_defines_working_functions_in_every_available_shell() {
    for shell in ["bash", "zsh"] {
        if Command::new(shell).arg("--version").output().is_err() {
            eprintln!("SKIP: {shell} not available");
            continue;
        }
        let e = env(&format!("fn-{shell}"));
        let target = e.root.join("elsewhere");
        fs::create_dir_all(&target).unwrap();
        // A fake playbook that records argv and names a cd target.
        make_exec(
            &e.root.join("bin/playbook"),
            &format!(
                "#!/bin/sh\necho \"$*\" > \"{}\"\nprintf '%s\\n' \"{}\" > \"$PLAYBOOK_CC_CD_FILE\"\nexit 3\n",
                e.root.join("argv").display(),
                target.display()
            ),
        );
        let script = format!("{}\nccd one two; echo \"rc=$?\"; pwd -P", shell_init(shell));

        let out = Command::new(shell)
            .arg("-c")
            .arg(&script)
            .current_dir(&e.work)
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", e.root.join("bin").display()),
            )
            .env("TMPDIR", &e.root)
            .env("HOME", &e.home)
            .env("ZDOTDIR", &e.home)
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("rc=3"), "{shell}: {stdout}");
        assert!(
            stdout.trim_end().ends_with("elsewhere"),
            "{shell}: {stdout}"
        );
        assert_eq!(
            fs::read_to_string(e.root.join("argv")).unwrap().trim(),
            "cc launch --skip-permissions -- one two"
        );
        let leftovers = fs::read_dir(&e.root)
            .unwrap()
            .flatten()
            .filter(|f| f.file_name().to_string_lossy().starts_with("playbook-cc."))
            .count();
        assert_eq!(leftovers, 0, "{shell}: the cd temp file is removed");
    }
}

fn set_effort(e: &Env, level: &str) {
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["effort", level])
        .current_dir(&e.work)
        .env("HOME", &e.home)
        .output()
        .expect("run playbook effort");
    assert!(out.status.success(), "{out:?}");
}

#[test]
fn a_playbook_effort_ceiling_reaches_the_session_through_settings() {
    let e = env("effort-ceiling");
    set_effort(&e, "xhigh");
    e.launch(&["fresh"]);
    assert_eq!(
        e.calls()[0],
        r#"--settings {"maxEffortLevel":"xhigh"} -n proj"#
    );
}

#[test]
fn a_lower_claude_code_ceiling_adds_nothing_and_is_never_edited() {
    let e = env("effort-claude-lower");
    let settings = e.home.join(".claude/settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, r#"{"maxEffortLevel":"medium"}"#).unwrap();
    set_effort(&e, "xhigh");
    e.launch(&["fresh"]);
    assert_eq!(e.calls()[0], "-n proj");
    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        r#"{"maxEffortLevel":"medium"}"#
    );
}

#[test]
fn a_higher_claude_code_ceiling_is_read_and_left_alone() {
    let e = env("effort-claude-higher");
    let settings = e.home.join(".claude/settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, r#"{"maxEffortLevel":"max"}"#).unwrap();
    set_effort(&e, "xhigh");
    e.launch(&["fresh"]);
    assert_eq!(
        e.calls()[0],
        r#"--settings {"maxEffortLevel":"xhigh"} -n proj"#
    );
    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        r#"{"maxEffortLevel":"max"}"#
    );
}

#[test]
fn max_and_an_own_settings_flag_add_nothing() {
    let e = env("effort-max");
    e.launch(&["fresh"]);
    assert_eq!(e.calls()[0], "-n proj");
    set_effort(&e, "low");
    e.launch_with(&["--settings", "my.json", "--"], &["fresh"], &[]);
    assert!(!e.calls()[1].contains("maxEffortLevel"), "{:?}", e.calls());
}
