// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook doctor check` end to end against a scratch HOME. These are the
//! scenarios `shell/doctor.test.sh` used to run against the bash blocks in
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
        let root = std::env::temp_dir().join(format!("pb-doctor-{tag}-{}-{n}", std::process::id()));
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

    fn run(&self, path: &[&Path], shell: &str, plugin_root: Option<&Path>) -> Vec<Value> {
        let mut paths: Vec<PathBuf> = path.iter().map(|p| p.to_path_buf()).collect();
        paths.push(PathBuf::from("/usr/bin"));
        paths.push(PathBuf::from("/bin"));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_playbook"));
        cmd.args(["doctor", "check", "--json"])
            .env("HOME", self.home())
            .env("SHELL", shell)
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env_remove("CLAUDE_PLUGIN_ROOT")
            .current_dir(&self.root);
        if let Some(root) = plugin_root {
            cmd.env("CLAUDE_PLUGIN_ROOT", root);
        }
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

const GUARDS: [&str; 5] = [
    "rm-workspace-guard",
    "bg-await-guard",
    "no-slop-guard",
    "precommit-check",
    "commit-message-sanitizer",
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

// Layer 2

#[test]
fn l2_all_guards_and_the_backstop_wired_passes() {
    let b = Box::new("l2a");
    b.settings(&guard_settings(&GUARDS, &["commit-message-sanitizer"]));
    let rows = b.run(&[], "/bin/zsh", None);
    let l2 = layer(&rows, "2");
    assert_eq!(l2[0]["level"], "PASS", "{}", text(&l2));
    assert!(text(&l2).contains("6 of 6"));
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
        t.contains("5/6") && t.contains("precommit-check:NOT_WIRED"),
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
        assert!(t.contains("5/6"), "{missing}: {t}");
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
    assert!(t.contains("5/6"), "{t}");
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
    fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    fs::write(
        root.join(".claude-plugin/plugin.json"),
        format!(r#"{{"name":"playbook","version":"{version}"}}"#),
    )
    .unwrap();
}

#[test]
fn l6_no_binary_on_path_fails() {
    let b = Box::new("l6m");
    let t = text(&layer(&b.run(&[], "/bin/zsh", None), "6"));
    assert!(t.starts_with("FAIL") && t.contains("not on PATH"), "{t}");
}

#[test]
fn l6_binary_and_plugin_that_agree_pass() {
    let b = Box::new("l6k");
    let bin = b.stub("bin1", "0.19.0", true);
    let root = b.root.join("plugin");
    manifest(&root, "0.19.0");
    let rows = b.run(&[&bin], "/bin/zsh", Some(&root));
    let l6 = layer(&rows, "6");
    assert_eq!(l6.len(), 1, "{}", text(&l6));
    assert!(text(&l6).starts_with("PASS"), "{}", text(&l6));
}

#[test]
fn l6_a_version_skew_is_info_naming_both() {
    let b = Box::new("l6s");
    let bin = b.stub("bin1", "0.19.0", true);
    let root = b.root.join("plugin");
    manifest(&root, "0.18.0");
    let t = text(&layer(&b.run(&[&bin], "/bin/zsh", Some(&root)), "6"));
    assert!(
        t.starts_with("INFO") && t.contains("0.19.0") && t.contains("0.18.0"),
        "{t}"
    );
}

#[test]
fn l6_no_manifest_is_info_not_fail() {
    let b = Box::new("l6n");
    let bin = b.stub("bin1", "0.19.0", true);
    let t = text(&layer(&b.run(&[&bin], "/bin/zsh", None), "6"));
    assert!(
        t.starts_with("INFO") && t.contains("no plugin manifest"),
        "{t}"
    );
}

#[test]
fn l6_the_newest_cached_manifest_is_the_baseline() {
    let b = Box::new("l6c");
    let bin = b.stub("bin1", "0.19.0", true);
    for v in ["0.9.0", "0.19.0"] {
        manifest(
            &b.home()
                .join(format!(".claude/plugins/cache/m/playbook/{v}")),
            v,
        );
    }
    let t = text(&layer(&b.run(&[&bin], "/bin/zsh", None), "6"));
    assert!(t.starts_with("PASS"), "{t}");
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
