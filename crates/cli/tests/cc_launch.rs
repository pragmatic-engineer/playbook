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
    let dir = std::env::temp_dir().join(format!(
        "pb-launch-{tag}-{}-{n}",
        playbook::testing::run_id()
    ));
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
        "#!/bin/sh\nprintf '%s\\n' \"$PATH\" >> \"$FAKE_LOG.path\"\nprintf '%s\\n' \"$*\" >> \"$FAKE_LOG\"\nprintf '%s\\n' \"$PWD\" >> \"$FAKE_LOG.pwd\"\nprintf '%s\\n' \"${CLAUDE_CODE_DISABLE_AUTO_MEMORY:-unset}\" >> \"$FAKE_LOG.mem\"\nprintf '%s\\n' \"${PLAYBOOK_AGENT_VARIANTS:-unset}\" >> \"$FAKE_LOG.var\"\nprintf '%s\\n' \"${ANTHROPIC_DEFAULT_SONNET_MODEL:-unset}\" >> \"$FAKE_LOG.env\"\nprintf '%s\\n' \"${PLAYBOOK_AGENT_VARIANTS_PLUGIN:-unset}\" >> \"$FAKE_LOG.vplug\"\nprev=; for a in \"$@\"; do if [ \"$prev\" = --plugin-dir ]; then cp -R \"$a\" \"$FAKE_LOG.plug\"; fi; prev=$a; done\nexit ${FAKE_EXIT:-0}\n",
    );
    Env { root, home, work }
}

impl Env {
    fn log(&self) -> PathBuf {
        self.root.join("claude.log")
    }
    /// Every call, as the fake `claude` saw it.
    fn raw_calls(&self) -> Vec<String> {
        fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }
    /// Every call without the fallback pair the launcher always adds, so the
    /// other tests keep reading the flags they care about.
    fn calls(&self) -> Vec<String> {
        const PAIR: &str = "--fallback-model claude-opus-5,claude-sonnet-5,claude-haiku-4-5";
        self.raw_calls()
            .into_iter()
            .map(|l| {
                let l = l.replacen(&format!("{PAIR} "), "", 1);
                let l = l.replacen(&format!(" {PAIR}"), "", 1).replacen(PAIR, "", 1);
                l.replacen("--effort medium ", "", 1)
                    .replacen(" --effort medium", "", 1)
                    .replacen("--effort medium", "", 1)
            })
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
            .env_remove("CLAUDE_CODE_DISABLE_AUTO_MEMORY")
            .env_remove("CLAUDE_CODE_EFFORT_LEVEL")
            .env_remove("CLAUDE_CODE_ENABLE_TELEMETRY")
            .env_remove("OTEL_LOGS_EXPORTER")
            .env_remove("OTEL_METRICS_EXPORTER")
            .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
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
    assert!(String::from_utf8_lossy(&out.stdout).contains("cc clean: cloned"));
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
            "--dangerously-skip-permissions --append-system-prompt-file {} -n proj",
            prompt.display()
        )
    );
}

#[test]
fn replace_mode_keeps_the_system_prompt_file_flag() {
    let e = env("replace");
    let prompt = e.home.join(".config/playbook/prompts/SYSTEM_PROMPT.md");
    fs::create_dir_all(prompt.parent().unwrap()).unwrap();
    fs::write(&prompt, "p").unwrap();
    let cfg = e.home.join(".config/playbook/config.json");
    fs::write(&cfg, r#"{"launcher":{"systemPromptMode":"replace"}}"#).unwrap();
    let out = e.launch_with(&["--"], &["fresh"], &[]);
    assert!(out.status.success());
    assert_eq!(
        e.calls()[0],
        format!("--system-prompt-file {} -n proj", prompt.display())
    );
}

#[test]
fn a_user_system_prompt_flag_means_playbook_adds_no_prompt() {
    let e = env("userprompt");
    let prompt = e.home.join(".config/playbook/prompts/SYSTEM_PROMPT.md");
    fs::create_dir_all(prompt.parent().unwrap()).unwrap();
    fs::write(&prompt, "p").unwrap();
    for flag in ["--system-prompt=x", "--append-system-prompt-file=/x"] {
        let out = e.launch_with(&["--"], &[flag, "fresh"], &[]);
        assert!(out.status.success());
        let call = e.calls().pop().unwrap();
        assert!(!call.contains("SYSTEM_PROMPT.md"), "{call}");
    }
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
        let script = format!(
            "{}\nplaybook ccd one two; echo \"rc=$?\"; pwd -P",
            shell_init(shell)
        );

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

#[test]
fn the_launcher_passes_the_previous_generation_as_the_fallback() {
    let e = env("fallback");
    e.launch(&["fresh"]);
    let raw = e.raw_calls();
    assert!(
        raw[0].contains("--fallback-model claude-opus-5,claude-sonnet-5,claude-haiku-4-5"),
        "{raw:?}"
    );
}

#[test]
fn a_session_that_states_no_effort_runs_at_medium() {
    let e = env("effort-default");
    e.launch(&["fresh"]);
    let raw = e.raw_calls();
    assert_eq!(raw[0].matches("--effort").count(), 1, "{raw:?}");
    assert!(raw[0].contains("--effort medium"), "{raw:?}");
}

#[test]
fn an_effort_the_user_asked_for_is_left_alone() {
    let e = env("effort-asked");
    e.launch_with(&["--effort", "high", "--"], &["fresh"], &[]);
    let raw = e.raw_calls();
    assert_eq!(raw[0].matches("--effort").count(), 1, "{raw:?}");
    assert!(raw[0].contains("--effort high"), "{raw:?}");
    assert!(!raw[0].contains("--effort medium"), "{raw:?}");
}

#[test]
fn an_effort_level_in_the_user_settings_is_left_alone_and_never_written() {
    let e = env("effort-settings");
    let settings = e.home.join(".claude/settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, r#"{"effortLevel":"high"}"#).unwrap();
    e.launch(&["fresh"]);
    let raw = e.raw_calls();
    assert!(!raw[0].contains("--effort"), "{raw:?}");
    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        r#"{"effortLevel":"high"}"#,
        "the launcher must never edit Claude Code settings"
    );
}

#[test]
fn a_fallback_the_user_chose_is_left_alone() {
    let e = env("fallback-own");
    e.launch_with(&["--fallback-model", "haiku", "--"], &["fresh"], &[]);
    let raw = e.raw_calls();
    assert_eq!(raw[0].matches("--fallback-model").count(), 1, "{raw:?}");
    assert!(raw[0].contains("--fallback-model haiku"), "{raw:?}");
}

fn set_memory_source(e: &Env, value: &str) {
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["config", "set", "--global", "memory.source", value])
        .current_dir(&e.work)
        .env("HOME", &e.home)
        .output()
        .expect("run playbook config set");
    assert!(out.status.success(), "{out:?}");
}

fn mem_env_seen(e: &Env) -> String {
    fs::read_to_string(e.root.join("claude.log.mem")).unwrap_or_default()
}

#[test]
fn playbook_only_memory_turns_claude_auto_memory_off_for_the_session() {
    let e = env("mem-playbook");
    set_memory_source(&e, "playbook");
    e.launch(&["fresh"]);
    assert_eq!(mem_env_seen(&e).trim(), "1");
}

#[test]
fn the_default_leaves_claude_auto_memory_alone() {
    let e = env("mem-both");
    e.launch(&["fresh"]);
    assert_eq!(mem_env_seen(&e).trim(), "unset");
    set_memory_source(&e, "both");
    e.launch(&["fresh"]);
    assert!(mem_env_seen(&e).lines().all(|l| l == "unset"));
}

#[test]
fn choosing_a_memory_source_never_edits_claude_code_settings() {
    let e = env("mem-no-write");
    let settings = e.home.join(".claude/settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let body = r#"{"autoMemoryEnabled":true,"theme":"dark"}"#;
    fs::write(&settings, body).unwrap();
    set_memory_source(&e, "playbook");
    e.launch(&["fresh"]);
    set_memory_source(&e, "both");
    assert_eq!(fs::read_to_string(&settings).unwrap(), body);
}

#[test]
fn the_session_can_find_the_binary_that_launched_it() {
    let e = env("child-path");
    e.launch(&["fresh"]);
    let exe_dir = Path::new(env!("CARGO_BIN_EXE_playbook")).parent().unwrap();
    let seen = fs::read_to_string(format!("{}.path", e.log().display())).unwrap();
    let first = std::env::split_paths(seen.lines().next().unwrap())
        .next()
        .unwrap();
    assert_eq!(first, exe_dir, "{seen}");
}

#[test]
fn a_path_that_already_has_the_binary_is_left_alone() {
    let e = env("child-path-same");
    let exe_dir = Path::new(env!("CARGO_BIN_EXE_playbook")).parent().unwrap();
    let path = format!("{}:{}", exe_dir.display(), e.path());
    e.launch_with(&["--"], &["fresh"], &[("PATH", path.as_str())]);
    let seen = fs::read_to_string(format!("{}.path", e.log().display())).unwrap();
    assert_eq!(seen.lines().next().unwrap(), path);
}

// --- Effort-tier variants through --agents (ADR-0017) ---

fn plugin_with_agents(e: &Env) -> PathBuf {
    let root = e.root.join("plugin");
    fs::create_dir_all(root.join("agents")).unwrap();
    fs::write(
        root.join("agents/reviewer.md"),
        "---\nname: reviewer\ndescription: d\ntools: Read, Grep\nmodel: sonnet\neffort: high\n---\n\nReview.\n",
    )
    .unwrap();
    root
}

/// The variant agent files the first call's `--plugin-dir` held, by name:
/// (frontmatter and body text). The fake `claude` copies the directory while
/// it still exists, because the launcher removes it after the session.
fn variant_files(e: &Env) -> Option<std::collections::BTreeMap<String, String>> {
    let dir = e.root.join("claude.log.plug/agents");
    let mut out = std::collections::BTreeMap::new();
    for entry in fs::read_dir(dir).ok()? {
        let path = entry.ok()?.path();
        let name = path.file_stem()?.to_string_lossy().into_owned();
        out.insert(name, fs::read_to_string(&path).ok()?);
    }
    Some(out)
}

/// The `--agents` JSON in the first call, if any (the fallback route).
fn agents_arg(e: &Env) -> Option<serde_json::Value> {
    let line = e.raw_calls().into_iter().next()?;
    let rest = line.split_once("--agents ")?.1.to_string();
    serde_json::Deserializer::from_str(&rest)
        .into_iter::<serde_json::Value>()
        .next()?
        .ok()
}

fn launch_with_plugin(e: &Env, root: &Path, args: &[&str]) {
    let out = e.launch_with(
        &["--"],
        args,
        &[("CLAUDE_PLUGIN_ROOT", root.to_str().unwrap())],
    );
    assert!(out.status.success(), "{out:?}");
}

#[test]
fn the_launcher_passes_variants_as_a_session_plugin_and_removes_it_after() {
    let e = env("variants");
    let root = plugin_with_agents(&e);
    launch_with_plugin(&e, &root, &["fresh"]);

    let files = variant_files(&e).expect("--plugin-dir was passed");
    for name in ["reviewer-low", "reviewer-medium", "reviewer-xhigh"] {
        let text = &files[name];
        assert!(text.contains(&format!("name: {name}\n")), "{text}");
        assert!(text.contains("model: sonnet\n"), "{text}");
        assert!(text.contains("tools: Read, Grep\n"), "{text}");
        assert!(text.ends_with("Review.\n"), "{text}");
    }
    assert!(files["reviewer-low"].contains("effort: low\n"));
    assert!(!files.contains_key("reviewer"), "the base is not repeated");
    assert!(
        e.root
            .join("claude.log.plug/.claude-plugin/plugin.json")
            .is_file(),
        "the throwaway plugin needs a manifest"
    );
    assert!(
        agents_arg(&e).is_none(),
        "no --agents when the plugin route works"
    );
    let seen = fs::read_to_string(e.root.join("claude.log.var")).unwrap();
    assert_eq!(seen.trim(), "reviewer-low,reviewer-medium,reviewer-xhigh");
    let plug = fs::read_to_string(e.root.join("claude.log.vplug")).unwrap();
    assert_eq!(plug.trim(), "playbook-variants");

    let line = e.raw_calls().remove(0);
    let dir = line
        .split_once("--plugin-dir ")
        .and_then(|(_, r)| r.split_whitespace().next())
        .expect("plugin dir in the args");
    assert!(
        !Path::new(dir).exists(),
        "{dir} should be removed after the session"
    );
}

#[test]
fn a_ceiling_keeps_the_higher_variants_out() {
    let e = env("variants-ceiling");
    let root = plugin_with_agents(&e);
    set_effort(&e, "medium");
    launch_with_plugin(&e, &root, &["fresh"]);
    let files = variant_files(&e).expect("--plugin-dir was passed");
    assert!(files.contains_key("reviewer-medium"));
    assert!(!files.contains_key("reviewer-xhigh"));
}

#[test]
fn agents_variants_off_and_a_user_agents_flag_add_nothing() {
    let e = env("variants-off");
    let root = plugin_with_agents(&e);
    let set = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["config", "set", "--global", "agents.variants", "off"])
        .env("HOME", &e.home)
        .output()
        .unwrap();
    assert!(set.status.success(), "{set:?}");
    launch_with_plugin(&e, &root, &["fresh"]);
    assert!(variant_files(&e).is_none());
    assert!(agents_arg(&e).is_none());

    let e2 = env("variants-user-flag");
    let root2 = plugin_with_agents(&e2);
    launch_with_plugin(&e2, &root2, &["--agents", "{}", "fresh"]);
    let line = e2.raw_calls().remove(0);
    assert_eq!(line.matches("--agents").count(), 1, "{line}");
    assert!(!line.contains("--plugin-dir"), "{line}");
}

#[test]
fn without_a_plugin_root_no_variants_are_passed() {
    let e = env("variants-no-root");
    e.launch(&["fresh"]);
    assert!(agents_arg(&e).is_none());
    assert!(variant_files(&e).is_none());
    let seen = fs::read_to_string(e.root.join("claude.log.var")).unwrap();
    assert_eq!(seen.trim(), "unset");
}

#[test]
fn at_xhigh_the_fallback_chain_starts_at_the_model_that_accepts_it() {
    let e = env("fallback-xhigh");
    e.launch_with(&["--effort", "xhigh", "--"], &["fresh"], &[]);
    let raw = e.raw_calls();
    assert!(
        raw[0].contains("--fallback-model claude-haiku-4-5 "),
        "{raw:?}"
    );
    assert!(!raw[0].contains("claude-sonnet-5,"), "{raw:?}");
}

#[test]
fn a_models_override_reaches_claude_as_the_alias_variable() {
    let e = env("models-override");
    let set = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args([
            "config",
            "set",
            "--global",
            "models.sonnet",
            "claude-sonnet-5",
        ])
        .current_dir(&e.work)
        .env("HOME", &e.home)
        .output()
        .unwrap();
    assert!(set.status.success(), "{set:?}");
    e.launch(&["fresh"]);
    let logged = fs::read_to_string(format!("{}.env", e.log().display())).unwrap();
    assert_eq!(logged.trim(), "claude-sonnet-5");
}

#[test]
fn without_an_override_the_alias_variable_is_not_set() {
    let e = env("models-no-override");
    e.launch(&["fresh"]);
    let logged = fs::read_to_string(format!("{}.env", e.log().display())).unwrap();
    assert_eq!(logged.trim(), "unset");
}

/// Reads the pid from `collector.json` and stops that process.
fn stop_collector(state: &Path) {
    let text = fs::read_to_string(state).unwrap_or_default();
    let pid = text
        .split("\"pid\":")
        .nth(1)
        .and_then(|t| t.split([',', '}']).next())
        .unwrap_or("0")
        .trim()
        .to_string();
    if pid != "0" {
        let _ = Command::new("kill").arg(&pid).status();
    }
}

#[test]
fn cc_starts_the_collector_once_when_the_usage_db_exists_and_points_the_session_at_it() {
    let e = env("collector-spawn");
    let usage = e.home.join(".config/playbook/data");
    fs::create_dir_all(&usage).unwrap();
    fs::write(usage.join("usage.db"), "").unwrap();
    make_exec(
        &e.root.join("bin/claude"),
        "#!/bin/sh\nprintf '%s %s\\n' \"$OTEL_EXPORTER_OTLP_ENDPOINT\" \"$OTEL_METRICS_EXPORTER\" >> \"$FAKE_LOG.otel\"\nexit 0\n",
    );
    let state = usage.join("collector.json");
    let first = e.launch(&[]);
    let second = e.launch(&[]);
    let seen = fs::read_to_string(e.root.join("claude.log.otel")).unwrap_or_default();
    let note = String::from_utf8_lossy(&first.stderr).to_string();
    let again = String::from_utf8_lossy(&second.stderr).to_string();
    stop_collector(&state);
    let lines: Vec<&str> = seen.lines().collect();
    assert_eq!(lines.len(), 2, "{seen}");
    assert!(
        lines[0].starts_with("http://127.0.0.1:") && lines[0].ends_with(" otlp"),
        "seen={seen:?} note={note:?}"
    );
    assert_eq!(lines[0], lines[1], "the second session reuses the receiver");
    assert!(note.contains("started the usage collector"), "{note}");
    assert!(!again.contains("started the usage collector"), "{again}");
}

#[test]
fn cc_starts_no_collector_without_a_usage_db_or_with_telemetry_off() {
    let e = env("collector-none");
    make_exec(
        &e.root.join("bin/claude"),
        "#!/bin/sh\nprintf '%s\\n' \"${OTEL_EXPORTER_OTLP_ENDPOINT:-unset}\" >> \"$FAKE_LOG.otel\"\nexit 0\n",
    );
    e.launch(&[]);
    let usage = e.home.join(".config/playbook/data");
    fs::create_dir_all(&usage).unwrap();
    fs::write(usage.join("usage.db"), "").unwrap();
    fs::create_dir_all(e.home.join(".claude")).unwrap();
    fs::write(
        e.home.join(".claude/settings.json"),
        r#"{"env":{"OTEL_LOGS_EXPORTER":"none"}}"#,
    )
    .unwrap();
    e.launch(&[]);
    let seen = fs::read_to_string(e.root.join("claude.log.otel")).unwrap();
    assert_eq!(seen, "unset\nunset\n");
    assert!(!usage.join("collector.json").exists());
}
