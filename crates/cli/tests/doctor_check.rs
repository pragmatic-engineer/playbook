// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook doctor check` end to end against a scratch HOME. These are the
//! scenarios the retired doctor shell test used to run against the bash blocks in
//! `commands/doctor.md`, now against the compiled check.

use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

struct Box {
    root: PathBuf,
}

impl Box {
    fn new(tag: &str) -> Box {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pb-doctor-{tag}-{}-{n}",
            playbook::testing::run_id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home/.claude")).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        Box { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn settings(&self, body: &str) {
        fs::write(self.home().join(".claude/settings.json"), body).unwrap();
    }

    /// A fake `playbook` that answers `--version` and, optionally, a
    /// `gate record --help` listing `--source`.
    fn stub(&self, dir: &str, version: &str, gate_source: bool) -> PathBuf {
        let d = self.root.join(dir);
        fs::create_dir_all(&d).unwrap();
        let gate = if gate_source {
            "echo '  --source <S>'"
        } else {
            "echo usage"
        };
        let body = format!(
            "#!/bin/sh\ncase \"$1\" in --version) echo 'playbook {version}';; gate) {gate};; esac\n"
        );
        let f = d.join("playbook");
        fs::write(&f, body).unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).unwrap();
        d
    }

    fn run_with_env(&self, extra: &[(&str, &str)]) -> Vec<Value> {
        self.run_inner(&[], "/bin/zsh", None, extra)
    }

    fn run(&self, path: &[&Path], shell: &str, plugin_root: Option<&Path>) -> Vec<Value> {
        self.run_inner(path, shell, plugin_root, &[])
    }

    fn run_inner(
        &self,
        path: &[&Path],
        shell: &str,
        plugin_root: Option<&Path>,
        extra: &[(&str, &str)],
    ) -> Vec<Value> {
        let mut paths: Vec<PathBuf> = path.iter().map(|p| p.to_path_buf()).collect();
        paths.push(PathBuf::from("/usr/bin"));
        paths.push(PathBuf::from("/bin"));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_playbook"));
        cmd.args(["doctor", "check", "--json"])
            .env("HOME", self.home())
            .env("SHELL", shell)
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env_remove("CLAUDE_PLUGIN_ROOT")
            .env_remove("PLAYBOOK_IGNORE_PLUGIN_DISABLED")
            .current_dir(&self.root);
        if let Some(root) = plugin_root {
            cmd.env("CLAUDE_PLUGIN_ROOT", root);
        }
        cmd.envs(extra.iter().copied());
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<Vec<Value>>(&out.stdout).unwrap()
    }
}

fn layer<'a>(rows: &'a [Value], n: &str) -> Vec<&'a Value> {
    rows.iter().filter(|r| r["layer"] == n).collect()
}

fn text(rows: &[&Value]) -> String {
    rows.iter()
        .map(|r| {
            format!(
                "{} {}",
                r["level"].as_str().unwrap(),
                r["message"].as_str().unwrap()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const GUARDS: [&str; 6] = [
    "rm-workspace-guard",
    "bg-await-guard",
    "no-slop-guard",
    "precommit-check",
    "commit-message-sanitizer",
    "policy-guard",
];

fn hook(cmd: &str) -> String {
    format!(r#"{{"hooks":[{{"type":"command","command":"{cmd}"}}]}}"#)
}

fn guard_settings(pre: &[&str], post: &[&str]) -> String {
    let pre: Vec<String> = pre
        .iter()
        .map(|g| hook(&format!("playbook hook {g}")))
        .collect();
    let post: Vec<String> = post
        .iter()
        .map(|g| hook(&format!("playbook hook {g}")))
        .collect();
    format!(
        r#"{{"hooks":{{"PreToolUse":[{}],"PostToolUse":[{}]}}}}"#,
        pre.join(","),
        post.join(",")
    )
}

// A paused plugin

const PAUSED: &str = r#""enabledPlugins":{"playbook@pragmatic-engineer":false}"#;

const STATUS: &str = r#""statusLine":{"type":"command","command":"playbook statusline"}"#;

fn paused_settings() -> String {
    let live = guard_settings(&GUARDS, &["commit-message-sanitizer"]);
    live.replacen('{', &format!("{{{PAUSED},{STATUS},"), 1)
}

#[test]
fn a_paused_plugin_turns_the_guard_row_into_a_warning_and_says_why() {
    let b = Box::new("paused");
    b.settings(&paused_settings());

    let rows = b.run(&[], "/bin/zsh", None);

    let (l1, l2, l5) = (layer(&rows, "1"), layer(&rows, "2"), layer(&rows, "5"));
    assert_eq!(l1[0]["level"], "WARN", "{}", text(&l1));
    assert!(text(&l1).contains("paused"), "{}", text(&l1));
    assert_eq!(l2[0]["level"], "WARN", "{}", text(&l2));
    assert!(text(&l2).contains("7 of 7"), "{}", text(&l2));
    assert!(text(&l2).contains("plugin is paused"), "{}", text(&l2));
    assert_eq!(l5[0]["level"], "WARN", "{}", text(&l5));
    assert!(text(&l5).contains("plugin is paused"), "{}", text(&l5));
}

#[test]
fn the_escape_hatch_keeps_the_pass_rows_of_a_paused_plugin() {
    let b = Box::new("paused-ignored");
    b.settings(&paused_settings());

    let rows = b.run_with_env(&[("PLAYBOOK_IGNORE_PLUGIN_DISABLED", "1")]);

    assert_eq!(layer(&rows, "2")[0]["level"], "PASS");
    assert!(!text(&layer(&rows, "1")).contains("paused"));
}

#[test]
fn a_plugin_that_is_enabled_or_not_listed_keeps_its_pass_rows() {
    let live = guard_settings(&GUARDS, &["commit-message-sanitizer"]);
    let enabled = live.replacen(
        '{',
        r#"{"enabledPlugins":{"playbook@pragmatic-engineer":true},"#,
        1,
    );
    for (tag, settings) in [("enabled", enabled), ("not-listed", live)] {
        let b = Box::new(tag);
        b.settings(&settings);

        let rows = b.run(&[], "/bin/zsh", None);

        assert_eq!(layer(&rows, "2")[0]["level"], "PASS", "{tag}");
        assert!(!text(&layer(&rows, "1")).contains("paused"), "{tag}");
    }
}

// Layer 2

#[test]
fn l2_all_guards_and_the_backstop_wired_passes() {
    let b = Box::new("l2a");
    b.settings(&guard_settings(&GUARDS, &["commit-message-sanitizer"]));
    let rows = b.run(&[], "/bin/zsh", None);
    let l2 = layer(&rows, "2");
    assert_eq!(l2[0]["level"], "PASS", "{}", text(&l2));
    assert!(text(&l2).contains("7 of 7"));
}

#[test]
fn l2_a_legacy_command_is_not_wired() {
    let b = Box::new("l2b");
    let pre = [
        hook("playbook hook rm-workspace-guard"),
        hook("playbook hook bg-await-guard"),
        hook("playbook hook no-slop-guard"),
        hook("~/.claude/hooks/precommit-check.sh"),
        hook("playbook hook commit-message-sanitizer"),
        hook("playbook hook policy-guard"),
    ]
    .join(",");
    b.settings(&format!(
        r#"{{"hooks":{{"PreToolUse":[{pre}],"PostToolUse":[{}]}}}}"#,
        hook("playbook hook commit-message-sanitizer")
    ));
    let rows = b.run(&[], "/bin/zsh", None);
    let t = text(&layer(&rows, "2"));
    assert!(t.starts_with("FAIL"), "{t}");
    assert!(
        t.contains("6/7") && t.contains("precommit-check:NOT_WIRED"),
        "{t}"
    );
}

#[test]
fn l2_a_near_miss_command_does_not_count() {
    let b = Box::new("l2c");
    let pre = [
        hook("playbook hook rm-workspace-guard-legacy"),
        hook("playbook hook bg-await-guard"),
        hook("playbook hook no-slop-guard"),
        hook("playbook hook precommit-check"),
        hook("playbook hook commit-message-sanitizer"),
        hook("playbook hook policy-guard"),
    ]
    .join(",");
    b.settings(&format!(
        r#"{{"hooks":{{"PreToolUse":[{pre}],"PostToolUse":[{}]}}}}"#,
        hook("playbook hook commit-message-sanitizer")
    ));
    let t = text(&layer(&b.run(&[], "/bin/zsh", None), "2"));
    assert!(t.contains("rm-workspace-guard:NOT_WIRED"), "{t}");
}

#[test]
fn l2_each_missing_guard_drops_the_count_and_is_named() {
    for missing in GUARDS {
        let b = Box::new("l2e");
        let pre: Vec<&str> = GUARDS.iter().copied().filter(|g| *g != missing).collect();
        b.settings(&guard_settings(&pre, &["commit-message-sanitizer"]));
        let t = text(&layer(&b.run(&[], "/bin/zsh", None), "2"));
        assert!(t.contains("6/7"), "{missing}: {t}");
        assert!(
            t.contains(&format!(" {missing}:NOT_WIRED")),
            "{missing}: {t}"
        );
    }
}

#[test]
fn l2_the_backstop_is_checked_on_posttooluse() {
    let b = Box::new("l2g");
    b.settings(&guard_settings(&GUARDS, &[]));
    let t = text(&layer(&b.run(&[], "/bin/zsh", None), "2"));
    assert!(t.contains("6/7"), "{t}");
    assert!(
        t.contains("commit-message-sanitizer(PostToolUse):NOT_WIRED"),
        "{t}"
    );
    assert!(!t.contains(" commit-message-sanitizer:NOT_WIRED"), "{t}");
}

// Layer 3 and 4

#[test]
fn l3_and_l4_are_opt_in_and_never_fail() {
    let b = Box::new("l34");
    let rows = b.run(&[], "/bin/zsh", None);
    assert_eq!(layer(&rows, "3")[0]["level"], "INFO");
    assert_eq!(layer(&rows, "4")[0]["level"], "INFO");
    let rows = b.run(&[], "/usr/bin/fish", None);
    assert!(text(&layer(&rows, "3")).contains("shell not detected"));
}

#[test]
fn l3_passes_with_the_shell_init_line_and_a_binary_on_path() {
    let b = Box::new("l3p");
    fs::write(b.home().join(".zshrc"), "eval \"$(playbook shell-init)\"\n").unwrap();
    let bin = b.stub("bin1", "0.19.0", true);
    let rows = b.run(&[&bin], "/bin/zsh", None);
    assert_eq!(layer(&rows, "3")[0]["level"], "PASS");
}

#[test]
fn l3_an_old_source_line_reads_as_outdated() {
    let b = Box::new("l3o");
    fs::write(
        b.home().join(".zshrc"),
        "source ~/.config/playbook/shell/zsh/cc.zsh\n",
    )
    .unwrap();
    let t = text(&layer(&b.run(&[], "/bin/zsh", None), "3"));
    assert!(t.contains("outdated launcher line"), "{t}");
}

#[test]
fn l4_passes_when_the_system_prompt_is_placed() {
    let b = Box::new("l4");
    let dir = b.home().join(".config/playbook/prompts");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SYSTEM_PROMPT.md"), "x").unwrap();
    assert_eq!(
        layer(&b.run(&[], "/bin/zsh", None), "4")[0]["level"],
        "PASS"
    );
}

#[cfg(unix)]
#[test]
fn l4_warns_when_the_system_prompt_is_unreadable() {
    use std::os::unix::fs::PermissionsExt;
    let b = Box::new("l4u");
    let dir = b.home().join(".config/playbook/prompts");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("SYSTEM_PROMPT.md");
    fs::write(&file, "x").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::File::open(&file).is_ok() {
        return; // running as root, the file stays readable
    }
    assert_eq!(
        layer(&b.run(&[], "/bin/zsh", None), "4")[0]["level"],
        "WARN"
    );
}

#[test]
fn doctor_shows_the_effort_ceiling_read_only_and_its_limit() {
    let b = Box::new("effort-rows");
    b.settings(r#"{"maxEffortLevel":"medium"}"#);
    let before = fs::read_to_string(b.home().join(".claude/settings.json")).unwrap();

    let rows = b.run(&[], "/bin/zsh", None);

    let t: Vec<String> = layer(&rows, "-")
        .iter()
        .map(|r| r["message"].as_str().unwrap().to_string())
        .collect();
    assert!(
        t.iter().any(|m| m.contains("effort ceiling: playbook auto, Claude Code medium (read only, never written), effective medium")),
        "{t:?}"
    );
    assert!(
        t.iter()
            .any(|m| m.contains("does not cap a command, skill or base agent")),
        "{t:?}"
    );
    assert_eq!(
        fs::read_to_string(b.home().join(".claude/settings.json")).unwrap(),
        before
    );
}

// Layer 5

fn statusline(b: &Box, cmd: &str) {
    b.settings(&format!(
        r#"{{"statusLine":{{"type":"command","command":"{cmd}"}}}}"#
    ));
}

#[test]
fn l5_the_binary_status_line_passes() {
    let b = Box::new("l5r");
    statusline(&b, "playbook statusline");
    assert_eq!(
        layer(&b.run(&[], "/bin/zsh", None), "5")[0]["level"],
        "PASS"
    );
}

#[test]
fn l5_the_retired_script_forms_fail_as_outdated() {
    for cmd in [
        "bash $HOME/.config/playbook/statusline.sh",
        "bash ~/.config/playbook/statusline.sh",
    ] {
        let b = Box::new("l5o");
        statusline(&b, cmd);
        let t = text(&layer(&b.run(&[], "/bin/zsh", None), "5"));
        assert!(
            t.starts_with("FAIL") && t.contains("retired statusline.sh"),
            "{cmd}: {t}"
        );
    }
}

#[test]
fn l5_a_custom_command_is_info_and_a_missing_file_fails() {
    let b = Box::new("l5c");
    let script = b.home().join("mine.sh");
    fs::write(&script, "x").unwrap();
    statusline(&b, &format!("bash {}", script.display()));
    assert_eq!(
        layer(&b.run(&[], "/bin/zsh", None), "5")[0]["level"],
        "INFO"
    );
    statusline(&b, "bash /nowhere/missing.sh");
    let t = text(&layer(&b.run(&[], "/bin/zsh", None), "5"));
    assert!(
        t.starts_with("FAIL") && t.contains("/nowhere/missing.sh"),
        "{t}"
    );
}

#[test]
fn l5_no_status_line_is_info() {
    let b = Box::new("l5n");
    b.settings("{}");
    assert_eq!(
        layer(&b.run(&[], "/bin/zsh", None), "5")[0]["level"],
        "INFO"
    );
}

// Layer 6

fn manifest(root: &Path, version: &str) {
    manifest_with(root, version, "");
}

/// A plugin folder the resolver accepts, with `extra` JSON fields in its manifest.
fn manifest_with(root: &Path, version: &str, extra: &str) {
    fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    fs::create_dir_all(root.join("skills")).unwrap();
    fs::write(
        root.join(".claude-plugin/plugin.json"),
        format!(r#"{{"name":"playbook","version":"{version}"{extra}}}"#),
    )
    .unwrap();
}

/// The layer 6 version row, which comes first when a binary is found.
fn version_row(rows: &[Value]) -> String {
    text(&layer(rows, "6")[..1])
}

/// The layer 6 upgrade record row, which comes second when a binary is found.
fn record_row(rows: &[Value]) -> String {
    text(&layer(rows, "6")[1..2])
}

#[test]
fn l6_no_binary_on_path_fails() {
    let b = Box::new("l6m");
    let t = text(&layer(&b.run(&[], "/bin/zsh", None), "6"));
    assert!(t.starts_with("FAIL") && t.contains("not on PATH"), "{t}");
}

#[test]
fn l6_a_plugin_inside_the_supported_range_passes_even_when_versions_differ() {
    let b = Box::new("l6k");
    let bin = b.stub("bin1", "0.19.0", true);
    let root = b.root.join("plugin");
    manifest(&root, "0.31.1");
    let rows = b.run(&[&bin], "/bin/zsh", Some(&root));
    let t = version_row(&rows);
    assert!(t.starts_with("PASS") && t.contains("0.31.1"), "{t}");
}

#[test]
fn l6_a_plugin_older_than_the_supported_range_warns_with_the_update_command() {
    let b = Box::new("l6s");
    let bin = b.stub("bin1", "0.19.0", true);
    let root = b.root.join("plugin");
    manifest(&root, "0.25.0");
    let t = version_row(&b.run(&[&bin], "/bin/zsh", Some(&root)));
    assert!(
        t.starts_with("WARN") && t.contains("claude plugin update playbook@pragmatic-engineer"),
        "{t}"
    );
}

#[test]
fn l6_a_binary_older_than_the_plugin_needs_warns_with_the_upgrade_command() {
    let b = Box::new("l6o");
    let bin = b.stub("bin1", "0.19.0", true);
    let root = b.root.join("plugin");
    manifest_with(
        &root,
        "0.31.1",
        r#","metadata":{"minBinaryVersion":"0.26.0"}"#,
    );
    let t = version_row(&b.run(&[&bin], "/bin/zsh", Some(&root)));
    assert!(
        t.starts_with("WARN") && t.contains("older than this plugin needs (0.26.0)"),
        "{t}"
    );
}

#[test]
fn l6_no_manifest_is_info_not_fail() {
    let b = Box::new("l6n");
    let bin = b.stub("bin1", "0.19.0", true);
    let t = version_row(&b.run(&[&bin], "/bin/zsh", None));
    assert!(
        t.starts_with("INFO") && t.contains("no plugin manifest"),
        "{t}"
    );
}

#[test]
fn l6_the_newest_cached_plugin_is_the_baseline() {
    let b = Box::new("l6c");
    let bin = b.stub("bin1", "0.19.0", true);
    for v in ["0.9.0", "0.31.1"] {
        manifest(
            &b.home().join(format!(
                ".claude/plugins/cache/pragmatic-engineer/playbook/{v}"
            )),
            v,
        );
    }
    let t = version_row(&b.run(&[&bin], "/bin/zsh", None));
    assert!(t.starts_with("PASS") && t.contains("0.31.1"), "{t}");
}

#[test]
fn l6_the_upgrade_record_row_follows_the_record_of_the_binary_on_path() {
    let b = Box::new("l6r");
    let bin = b.stub("bin1", "0.19.0", true);
    let root = playbook::common::paths::playbook_root_from(&b.home());
    let set = |pairs: &[(&str, &str)]| {
        playbook::state::transaction(&root, |tx| {
            for (k, v) in pairs {
                tx.set(k, v)?;
            }
            Ok(())
        })
        .unwrap();
    };
    let row = || record_row(&b.run(&[&bin], "/bin/zsh", None));

    let t = row();
    assert!(
        t.starts_with("INFO") && t.contains("no upgrade record yet"),
        "{t}"
    );
    set(&[("versions/binary", "0.19.0")]);
    let t = row();
    assert!(t.starts_with("PASS") && t.contains("binary 0.19.0"), "{t}");
    set(&[("versions/binary", "0.18.0")]);
    let t = row();
    assert!(
        t.starts_with("INFO") && t.contains("next playbook command or session start updates it"),
        "{t}"
    );
    set(&[("versions/binary", "0.20.0")]);
    let t = row();
    assert!(
        t.starts_with("INFO") && t.contains("nothing runs after a downgrade"),
        "{t}"
    );
    set(&[("versions/pending", "0011-dead-hook-scripts")]);
    let t = row();
    assert!(
        t.starts_with("WARN") && t.contains("1 upgrade item(s) need you: 0011-dead-hook-scripts"),
        "{t}"
    );
    set(&[("versions/failed-at", "1000")]);
    let t = row();
    assert!(
        t.starts_with("WARN") && t.contains("last upgrade heal failed"),
        "{t}"
    );
    let rows = b.run(&[&bin], "/bin/zsh", None);
    let hint = hints(&layer(&rows, "6")[1..2]);
    assert_eq!(hint, "run: playbook migrate");
    let doc = fs::read_to_string(
        std::path::Path::new(env!("PLAYBOOK_REPO_ROOT"))
            .join("plugins/playbook/commands/doctor.md"),
    )
    .unwrap();
    assert!(
        doc.contains(&format!("`{hint}`")),
        "doctor.md must quote the hint {hint}"
    );
}

#[test]
fn l6_a_binary_without_the_gate_source_flag_fails() {
    let b = Box::new("l6g");
    let bin = b.stub("bin1", "0.19.0", false);
    let t = text(&layer(&b.run(&[&bin], "/bin/zsh", None), "6"));
    assert!(t.contains("FAIL gate record has no --source"), "{t}");
}

#[test]
fn l6_a_stale_first_binary_warns_and_a_newer_first_is_info() {
    let b = Box::new("l6w");
    let old = b.stub("old", "0.17.0", true);
    let new = b.stub("new", "0.19.0", true);
    let t = text(&layer(&b.run(&[&old, &new], "/bin/zsh", None), "6"));
    assert!(
        t.contains("WARN") && t.contains("older than a later one"),
        "{t}"
    );
    let t = text(&layer(&b.run(&[&new, &old], "/bin/zsh", None), "6"));
    assert!(t.contains("INFO several playbook binaries"), "{t}");
}

// Layer 7

#[test]
fn l7_bare_commands_are_never_checked() {
    let b = Box::new("l7a");
    b.settings(&guard_settings(&GUARDS, &[]));
    let t = text(&layer(&b.run(&[], "/bin/zsh", None), "7"));
    assert!(t.starts_with("PASS") && t.contains("(0 checked)"), "{t}");
}

#[test]
fn l7_a_dangling_leftover_hook_fails_and_is_reported_once() {
    let b = Box::new("l7d");
    let cmd = "python3 ~/.claude/hooks/memory_context.py";
    b.settings(&format!(
        r#"{{"hooks":{{"SessionStart":[{}],"Stop":[{}]}}}}"#,
        hook(cmd),
        hook(cmd)
    ));
    let rows = b.run(&[], "/bin/zsh", None);
    let l7 = layer(&rows, "7");
    assert_eq!(l7.len(), 1);
    assert!(text(&l7).starts_with("FAIL") && text(&l7).contains("memory_context.py"));
}

#[test]
fn l7_an_existing_path_and_an_unresolved_variable_are_not_flagged() {
    let b = Box::new("l7e");
    let script = b.home().join("ok.sh");
    fs::write(&script, "x").unwrap();
    b.settings(&format!(
        r#"{{"hooks":{{"Stop":[{},{}]}}}}"#,
        hook(&format!("bash {}", script.display())),
        hook("bash $SOMEWHERE/x.sh")
    ));
    let t = text(&layer(&b.run(&[], "/bin/zsh", None), "7"));
    assert!(t.starts_with("PASS") && t.contains("(1 checked)"), "{t}");
}

#[test]
fn l7_a_missing_settings_file_is_info_not_a_silent_pass() {
    let b = Box::new("l7m");
    let t = text(&layer(&b.run(&[], "/bin/zsh", None), "7"));
    assert!(t.starts_with("INFO"), "{t}");
}

// Table and extras

#[test]
fn the_table_lists_review_config_and_text_mode_ends_with_a_verdict() {
    let b = Box::new("tbl");
    b.settings(&guard_settings(&GUARDS, &["commit-message-sanitizer"]));
    let rows = b.run(&[], "/bin/zsh", None);
    let all: Vec<&Value> = rows.iter().collect();
    let t = text(&all);
    assert!(
        t.contains("autoReview.enabled: true (source: default)"),
        "{t}"
    );
    assert!(t.contains("pr.draft:"), "{t}");
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["doctor", "check"])
        .env("HOME", b.home())
        .env("PATH", "/usr/bin:/bin")
        .current_dir(&b.root)
        .output()
        .unwrap();
    let text_out = String::from_utf8_lossy(&out.stdout);
    assert!(
        text_out.contains("FAIL  playbook binary not on PATH"),
        "{text_out}"
    );
    assert!(text_out.contains("cannot fix layer 6"), "{text_out}");
}

#[test]
fn the_probes_run_side_by_side_but_the_table_keeps_its_order() {
    let b = Box::new("order");
    b.settings("{}");
    let dir = b.stub("bin", "1.2.3", true);
    // The data folder row lists files and sizes, which settings reads change
    // (the config store and its write-ahead log), so it is not part of the
    // comparison of order.
    let stable = |rows: Vec<Value>| -> Vec<Value> {
        rows.into_iter()
            .filter(|r| {
                !r["message"]
                    .as_str()
                    .is_some_and(|m| m.starts_with("data folder"))
            })
            .collect()
    };
    let first = stable(b.run(&[&dir], "/bin/zsh", None));
    assert!(!first.is_empty());
    for _ in 0..5 {
        assert_eq!(stable(b.run(&[&dir], "/bin/zsh", None)), first);
    }
}

// The hooks `playbook init` writes into settings.json

fn wire_all(b: &Box) -> PathBuf {
    let path = b.home().join(".claude/settings.json");
    playbook::init::wire::wire(&path).unwrap();
    path
}

fn hook_rows(rows: &[Value]) -> Vec<String> {
    layer(rows, "-")
        .iter()
        .map(|r| {
            format!(
                "{} {} -- {}",
                r["level"].as_str().unwrap(),
                r["message"].as_str().unwrap(),
                r["hint"].as_str().unwrap_or("")
            )
        })
        .filter(|t| {
            !t.contains("data folder")
                && (t.to_lowercase().contains("hook") || t.contains("minimal PATH"))
        })
        .collect()
}

/// A fake `playbook` whose `hook --help` lists every hook `init` writes.
fn stub_with_hooks(b: &Box, listed: &[&str]) -> PathBuf {
    let d = b.root.join("hookbin");
    fs::create_dir_all(&d).unwrap();
    let names = listed.join(" ");
    let body = format!(
        "#!/bin/sh\ncase \"$1\" in --version) echo 'playbook 0.1.0';; hook) echo '{names}';; esac\n"
    );
    let f = d.join("playbook");
    fs::write(&f, body).unwrap();
    fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).unwrap();
    d
}

/// A host whose system profile already puts a playbook on the minimal PATH
/// cannot show the blind case.
fn host_finds_playbook() -> bool {
    let host = Command::new("/bin/sh")
        .args(["-lc", "command -v playbook"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .output()
        .unwrap();
    !String::from_utf8_lossy(&host.stdout).trim().is_empty()
}

fn every_hook_name() -> Vec<&'static str> {
    playbook::init::wire::wired_hooks()
        .map(|h| h.name)
        .collect()
}

#[test]
fn nothing_wired_is_a_warning_with_the_one_command_to_fix_it() {
    let b = Box::new("hooks-absent");
    b.settings(r#"{"hooks":{}}"#);

    let rows = hook_rows(&b.run(&[], "/bin/sh", None));

    assert!(
        rows[0].starts_with("WARN playbook hooks are not in settings.json"),
        "{rows:?}"
    );
    assert!(rows[0].ends_with("run: playbook init --yes"), "{rows:?}");
}

#[test]
fn a_fully_wired_settings_file_passes() {
    let b = Box::new("hooks-present");
    wire_all(&b);
    let dir = stub_with_hooks(&b, &every_hook_name());

    let rows = hook_rows(&b.run(&[&dir], "/bin/sh", None));

    let total = every_hook_name().len();
    assert!(
        rows[0].starts_with(&format!(
            "PASS playbook hook entries in settings.json ({total} of {total})"
        )),
        "{rows:?}"
    );
    assert!(rows[0].contains("including session-init"), "{rows:?}");
    assert!(
        !rows
            .iter()
            .any(|r| r.starts_with("WARN") && r.contains("does not know")),
        "{rows:?}"
    );
}

#[test]
fn a_removed_entry_is_named_as_missing() {
    let b = Box::new("hooks-missing-one");
    let path = wire_all(&b);
    let mut v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    v["hooks"].as_object_mut().unwrap().remove("WorktreeRemove");
    fs::write(&path, v.to_string()).unwrap();

    let rows = hook_rows(&b.run(&[], "/bin/sh", None));

    assert!(rows[0].starts_with("WARN 1 of "), "{rows:?}");
    assert!(rows[0].contains("worktree-remove"), "{rows:?}");
}

#[test]
fn an_entry_for_a_hook_the_binary_no_longer_writes_is_stale() {
    let b = Box::new("hooks-stale");
    let path = wire_all(&b);
    let mut v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    v["hooks"]["SessionStart"].as_array_mut().unwrap().push(
        serde_json::json!({"hooks":[{"type":"command","command":"playbook hook migration-check"}]}),
    );
    fs::write(&path, v.to_string()).unwrap();

    let rows = hook_rows(&b.run(&[], "/bin/sh", None));

    assert!(
        rows.iter().any(|r| r.starts_with("WARN settings.json has a hook the binary no longer writes: playbook hook migration-check")),
        "{rows:?}"
    );
}

#[test]
fn a_binary_that_does_not_know_a_wired_hook_is_reported() {
    let b = Box::new("hooks-old-binary");
    wire_all(&b);
    let dir = stub_with_hooks(&b, &["session-init"]);

    let rows = hook_rows(&b.run(&[&dir], "/bin/sh", None));

    let row = rows
        .iter()
        .find(|r| r.contains("does not know these hooks"))
        .expect("row");
    assert!(
        row.starts_with("WARN") && row.contains("worktree-create"),
        "{row}"
    );
    assert!(row.ends_with("run: playbook update"), "{row}");
}

#[test]
fn a_login_shell_with_a_minimal_path_must_find_the_binary() {
    if host_finds_playbook() {
        return;
    }
    let b = Box::new("hooks-minimal-path");
    wire_all(&b);
    let dir = stub_with_hooks(&b, &every_hook_name());
    let blind = hook_rows(&b.run(&[&dir], "/bin/sh", None));
    assert!(
        blind
            .iter()
            .any(|r| r.contains("minimal PATH cannot find playbook")),
        "{blind:?}"
    );

    fs::write(
        b.home().join(".profile"),
        format!("export PATH=\"{}:$PATH\"\n", dir.display()),
    )
    .unwrap();
    let seen = hook_rows(&b.run(&[&dir], "/bin/sh", None));

    assert!(!seen.iter().any(|r| r.contains("minimal PATH")), "{seen:?}");
}

#[test]
fn an_old_ccc_alias_and_ccd_script_are_reported_with_their_replacements_and_left_alone() {
    // Arrange
    let b = Box::new("oldlaunch");
    let rc = b.home().join(".zshrc");
    fs::write(&rc, "alias ccc='claude --foo'\n").unwrap();
    let bin = b.root.join("old");
    fs::create_dir_all(&bin).unwrap();
    let ccd = bin.join("ccd");
    fs::write(&ccd, "#!/bin/sh\nexec claude \"$@\"\n").unwrap();
    fs::set_permissions(&ccd, fs::Permissions::from_mode(0o755)).unwrap();

    // Act
    let rows = b.run(&[&bin], "/bin/zsh", None);

    // Assert
    let all: Vec<&Value> = rows.iter().collect();
    let t = text(&all);
    assert!(t.contains("playbook cc` instead"), "{t}");
    assert!(t.contains("playbook ccd` instead"), "{t}");
    assert_eq!(
        fs::read_to_string(&rc).unwrap(),
        "alias ccc='claude --foo'\n"
    );
    assert!(ccd.exists());
}

/// A fake `claude` whose `plugin list` prints `body`, returned as the dir to put on PATH.
fn fake_claude(b: &Box, body: &str) -> PathBuf {
    let dir = b.root.join("claude-bin");
    fs::create_dir_all(&dir).unwrap();
    let claude = dir.join("claude");
    fs::write(&claude, format!("#!/bin/sh\nprintf '{body}'\n")).unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

#[test]
fn layer_one_reads_the_playbook_block_even_when_a_longer_named_plugin_is_listed_first() {
    let b = Box::new("layer1-exact");
    let dir = fake_claude(
        &b,
        "  playbook-hooks@pragmatic-engineer\\n    Status: disabled\\n\\n  playbook@pragmatic-engineer\\n    Status: enabled\\n",
    );

    let rows = b.run(&[&dir], "/bin/bash", None);

    assert!(text(&layer(&rows, "1")).contains("PASS plugin enabled"));
}

#[test]
fn layer_one_is_info_not_a_failure_for_a_binary_only_install() {
    let b = Box::new("layer1-absent");
    let dir = fake_claude(&b, "No plugins installed\\n");

    let rows = b.run(&[&dir], "/bin/bash", None);

    let row = text(&layer(&rows, "1"));
    assert!(
        row.contains("INFO skills plugin not installed (optional)"),
        "{row}"
    );
}

#[test]
fn layer_one_fails_when_the_playbook_plugin_is_disabled() {
    let b = Box::new("layer1-disabled");
    let dir = fake_claude(
        &b,
        "  playbook@pragmatic-engineer\\n    Status: disabled\\n",
    );

    let rows = b.run(&[&dir], "/bin/bash", None);

    assert!(text(&layer(&rows, "1")).contains("FAIL plugin installed but disabled"));
}

#[test]
fn a_login_shell_that_prints_something_is_not_taken_for_a_hit() {
    if host_finds_playbook() {
        return;
    }
    let b = Box::new("hooks-noisy-profile");
    wire_all(&b);
    let dir = stub_with_hooks(&b, &every_hook_name());
    fs::write(b.home().join(".profile"), "echo Agent pid 4242\n").unwrap();

    let rows = hook_rows(&b.run(&[&dir], "/bin/sh", None));

    assert!(
        rows.iter()
            .any(|r| r.contains("minimal PATH cannot find playbook")),
        "{rows:?}"
    );
}

#[test]
fn the_real_binarys_hook_help_lists_every_wired_hook() {
    let b = Box::new("hooks-real-binary");
    wire_all(&b);
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_playbook"))
        .parent()
        .unwrap()
        .to_path_buf();

    let rows = hook_rows(&b.run(&[&bin], "/bin/sh", None));

    assert!(
        !rows.iter().any(|r| r.contains("does not know")),
        "{rows:?}"
    );
}

#[test]
fn an_installed_hooks_plugin_is_reported_because_its_hooks_would_run_twice() {
    let b = Box::new("hooks-plugin-overlap");
    wire_all(&b);
    let dir = b.home().join(".claude/plugins");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("installed_plugins.json"),
        r#"{"version":2,"plugins":{"playbook-hooks@pragmatic-engineer":[{"scope":"user"}]}}"#,
    )
    .unwrap();

    let rows = hook_rows(&b.run(&[], "/bin/sh", None));

    let row = rows
        .iter()
        .find(|r| r.contains("playbook-hooks plugin is still installed"))
        .expect("overlap row");
    assert!(
        row.ends_with("run: claude plugin uninstall playbook-hooks@pragmatic-engineer"),
        "{row}"
    );
}

/// Rows from the telemetry checks: the ones whose message mentions telemetry
/// or the collector.
fn telemetry_rows(rows: &[Value]) -> Vec<&Value> {
    rows.iter()
        .filter(|r| {
            let m = r["message"].as_str().unwrap_or("");
            m.contains("telemetry") || m.contains("collector")
        })
        .collect()
}

fn hints(rows: &[&Value]) -> String {
    rows.iter()
        .filter_map(|r| r["hint"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Records a collector on `port` and settings that point at it.
fn point_settings_at_collector(b: &Box, port: u16) {
    let data = b.home().join(".config/playbook/data");
    fs::create_dir_all(&data).unwrap();
    fs::write(
        data.join("collector.json"),
        format!(r#"{{"pid":0,"port":{port},"token":"t"}}"#),
    )
    .unwrap();
    b.settings(&format!(
        r#"{{"env":{{"CLAUDE_CODE_ENABLE_TELEMETRY":"1","OTEL_EXPORTER_OTLP_ENDPOINT":"http://127.0.0.1:{port}"}}}}"#
    ));
}

/// A port nothing listens on: an unprivileged run cannot bind port 1.
fn free_port() -> u16 {
    1
}

#[test]
fn telemetry_that_is_not_set_up_warns_with_the_init_command() {
    let b = Box::new("tel-none");
    b.settings("{}");

    let rows = b.run_with_env(&[]);

    let mine = telemetry_rows(&rows);
    assert!(
        text(&mine).contains("WARN telemetry is not set up"),
        "{}",
        text(&mine)
    );
    assert!(hints(&mine).contains("playbook init --only telemetry"));
}

#[test]
fn an_off_switch_warns_with_the_init_command() {
    let b = Box::new("tel-off");
    b.settings(r#"{"env":{"CLAUDE_CODE_ENABLE_TELEMETRY":"0"}}"#);

    let rows = b.run_with_env(&[]);

    let mine = telemetry_rows(&rows);
    assert!(
        text(&mine).contains("WARN telemetry is switched off"),
        "{}",
        text(&mine)
    );
    assert!(hints(&mine).contains("playbook init --only telemetry"));
}

#[test]
fn a_collector_that_is_not_running_and_has_no_rows_warns_with_each_fix() {
    let b = Box::new("tel-down");
    point_settings_at_collector(&b, free_port());

    let rows = b.run_with_env(&[]);

    let mine = telemetry_rows(&rows);
    let all = text(&mine);
    assert!(
        all.contains("PASS telemetry env points at the playbook collector"),
        "{all}"
    );
    assert!(all.contains("WARN collector not running"), "{all}");
    assert!(all.contains("WARN no telemetry rows yet"), "{all}");
    assert!(hints(&mine).contains("playbook usage collect"));
}

#[test]
fn another_program_on_the_collector_port_fails_with_the_move_command() {
    let b = Box::new("tel-conflict");
    let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    point_settings_at_collector(&b, holder.local_addr().unwrap().port());
    // Another program that answers, but is not a playbook collector.
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for stream in holder.incoming().flatten().take(20) {
            let mut stream = stream;
            let mut buf = [0u8; 512];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        }
    });

    let rows = b.run_with_env(&[]);

    let mine = telemetry_rows(&rows);
    assert!(
        text(&mine).contains("FAIL another program owns the collector port"),
        "{}",
        text(&mine)
    );
    assert!(hints(&mine).contains("playbook usage collect --port"));
}

#[test]
fn a_port_in_settings_that_differs_from_the_collector_warns() {
    let b = Box::new("tel-stale-port");
    point_settings_at_collector(&b, free_port());
    b.settings(
        r#"{"env":{"CLAUDE_CODE_ENABLE_TELEMETRY":"1","OTEL_EXPORTER_OTLP_ENDPOINT":"http://127.0.0.1:2"}}"#,
    );
    fs::write(
        b.home().join(".config/playbook/telemetry-env.json"),
        r#"{"OTEL_EXPORTER_OTLP_ENDPOINT":"http://127.0.0.1:2"}"#,
    )
    .unwrap();

    let rows = b.run_with_env(&[]);

    let mine = telemetry_rows(&rows);
    assert!(
        text(&mine).contains("send telemetry to http://127.0.0.1:2"),
        "{}",
        text(&mine)
    );
    assert!(hints(&mine).contains("playbook init --only telemetry"));
}

#[test]
fn rows_from_the_last_day_pass() {
    let b = Box::new("tel-rows");
    point_settings_at_collector(&b, free_port());
    let db = b.home().join(".config/playbook/data/usage.db");
    let conn = playbook::usage::db::open_db(&db).unwrap();
    playbook::usage::telemetry_db::migrate_up(&conn).unwrap();
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap();
    playbook::usage::telemetry_db::insert_error(
        &conn,
        &playbook::usage::telemetry_db::TelemetryError {
            session_id: "s".into(),
            sequence: 1,
            timestamp: now,
            model: "m".into(),
            status_code: 500,
            attempt: 1,
            duration_ms: 10,
        },
    )
    .unwrap();
    drop(conn);

    let rows = b.run_with_env(&[]);

    let mine = telemetry_rows(&rows);
    assert!(
        text(&mine).contains("PASS telemetry rows arrived in the last day"),
        "{}",
        text(&mine)
    );
}

#[test]
fn the_config_key_off_is_reported_as_info() {
    let b = Box::new("tel-config-off");
    b.settings("{}");
    let cfg = b.home().join(".config/playbook");
    fs::create_dir_all(&cfg).unwrap();
    fs::write(cfg.join("config.json"), r#"{"usage":{"telemetry":"off"}}"#).unwrap();

    let rows = b.run_with_env(&[]);

    let mine = telemetry_rows(&rows);
    assert!(
        text(&mine).contains("INFO telemetry is off by the usage.telemetry setting"),
        "{}",
        text(&mine)
    );
}

#[test]
fn the_users_own_local_collector_is_not_called_stale() {
    let b = Box::new("tel-own-local");
    point_settings_at_collector(&b, free_port());
    b.settings(
        r#"{"env":{"CLAUDE_CODE_ENABLE_TELEMETRY":"1","OTEL_EXPORTER_OTLP_ENDPOINT":"http://127.0.0.1:4318"}}"#,
    );

    let rows = b.run_with_env(&[]);

    let mine = telemetry_rows(&rows);
    assert!(
        text(&mine).contains("INFO telemetry is set up by you"),
        "{}",
        text(&mine)
    );
}

#[test]
fn settings_that_name_the_collector_without_the_enable_switch_warn() {
    let b = Box::new("tel-no-enable");
    point_settings_at_collector(&b, free_port());
    b.settings(&format!(
        r#"{{"env":{{"OTEL_EXPORTER_OTLP_ENDPOINT":"http://127.0.0.1:{}"}}}}"#,
        free_port()
    ));

    let rows = b.run_with_env(&[]);

    let mine = telemetry_rows(&rows);
    assert!(
        text(&mine).contains("CLAUDE_CODE_ENABLE_TELEMETRY is not 1"),
        "{}",
        text(&mine)
    );
}
