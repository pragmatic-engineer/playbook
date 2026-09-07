// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Every scenario in `shell/check-shared-settings.test.sh`, ported.
//!
//! Each rejection case is asserted individually rather than as "some error":
//! a validator whose failure paths are untested is worse than none, because it
//! reports success on inputs it never actually examined.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

const PERMISSIONS: &str = r#"{
  "allow": ["Read", "Bash(git:*)"],
  "deny": ["Read(**/.env)"],
  "ask": ["Bash(curl:*)"],
  "defaultMode": "auto"
}"#;

struct Fixture {
    dir: PathBuf,
    repo: PathBuf,
    perms: PathBuf,
}

impl Fixture {
    /// A scratch repo holding the two files the fixture hook commands point at,
    /// plus the tracked permissions object the template must deep-equal.
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "playbook-settings-check-{tag}-{}-{n}",
            std::process::id()
        ));
        let repo = dir.join("repo");
        fs::create_dir_all(repo.join("hooks")).expect("scratch repo should be creatable");
        fs::write(repo.join("hooks/session-init.sh"), "").expect("fixture hook");
        fs::write(repo.join("x.sh"), "").expect("fixture hook");

        let perms = dir.join("permissions.json");
        fs::write(&perms, PERMISSIONS).expect("permissions fixture");

        Self { dir, repo, perms }
    }

    fn template(&self, name: &str, json: &str) -> PathBuf {
        let path = self.dir.join(format!("{name}.json"));
        fs::write(&path, json).expect("template fixture");
        path
    }

    fn run(&self, template: &Path) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(["settings", "check"])
            .arg(template)
            .arg(&self.perms)
            .arg(&self.repo)
            .output()
            .expect("playbook binary should spawn")
    }
}

/// A template that satisfies every rule, as a base for the defect variants.
/// The PreToolUse command is a real `playbook hook <name>` invocation, the
/// only external-command shape the shipped product ever writes.
fn good_template() -> String {
    format!(
        r#"{{
  "skipAutoPermissionPrompt": false,
  "permissions": {PERMISSIONS},
  "hooks": {{
    "SessionStart": [
      {{ "hooks": [ {{ "type": "command", "command": "~/.claude/hooks/session-init.sh" }} ] }}
    ],
    "PreToolUse": [
      {{ "matcher": "Bash", "hooks": [ {{ "type": "command", "command": "playbook hook rm-workspace-guard" }} ] }}
    ]
  }}
}}"#
    )
}

fn stderr_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn good_template_validates() {
    let f = Fixture::new("good");
    let t = f.template("good", &good_template());
    let out = f.run(&t);
    assert!(
        out.status.success(),
        "expected exit 0, got {:?}: {}",
        out.status.code(),
        stderr_of(&out)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("check-shared-settings: OK"));
}

#[test]
fn wrong_permissions_is_rejected() {
    let f = Fixture::new("bad-perms");
    let bad = good_template().replace(
        r#""allow": ["Read", "Bash(git:*)"]"#,
        r#""allow": ["Sneaky"]"#,
    );
    let t = f.template("bad-perms", &bad);
    let out = f.run(&t);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_of(&out).contains("does not deep-equal"),
        "got: {}",
        stderr_of(&out)
    );
}

#[test]
fn a_pinned_model_is_rejected() {
    let f = Fixture::new("bad-model");
    let bad = good_template().replace(
        r#"{
  "skipAutoPermissionPrompt": false,"#,
        r#"{
  "model": "opus",
  "skipAutoPermissionPrompt": false,"#,
    );
    let t = f.template("bad-model", &bad);
    let out = f.run(&t);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_of(&out).contains(".model must not ship"),
        "got: {}",
        stderr_of(&out)
    );
}

#[test]
fn skip_auto_permission_prompt_must_be_false() {
    let f = Fixture::new("bad-skip");
    let bad = good_template().replace(
        r#""skipAutoPermissionPrompt": false"#,
        r#""skipAutoPermissionPrompt": true"#,
    );
    let t = f.template("bad-skip", &bad);
    let out = f.run(&t);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_of(&out).contains("skipAutoPermissionPrompt must be false"),
        "got: {}",
        stderr_of(&out)
    );
}

#[test]
fn missing_hook_path_is_rejected() {
    let f = Fixture::new("bad-hook");
    let bad = good_template().replace("session-init.sh", "does-not-exist.sh");
    let t = f.template("bad-hook", &bad);
    let out = f.run(&t);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_of(&out);
    assert!(err.contains("hook command path not found"), "got: {err}");
    assert!(
        err.contains("does-not-exist.sh"),
        "the message must name the offending command: {err}"
    );
}

/// Each novel key gets its own assertion: a loop that stopped after the
/// first would leave the rest unproven. Covers the old 4-key denylist names
/// plus several keys that were never on any denylist, since the allowlist
/// must reject anything not explicitly shippable, not just a known list.
#[test]
fn novel_keys_not_in_allowlist_are_rejected() {
    for key in [
        "effortLevel",
        "theme",
        "preferredNotifChannel",
        "prefersReducedMotion",
        "totallyNovelKey",
        "someFutureFlag",
    ] {
        let f = Fixture::new(&format!("novel-{key}"));
        let bad = good_template().replace(
            r#"{
  "skipAutoPermissionPrompt": false,"#,
            &format!(
                r#"{{
  "{key}": "x",
  "skipAutoPermissionPrompt": false,"#
            ),
        );
        let t = f.template("novel", &bad);
        let out = f.run(&t);
        assert_eq!(out.status.code(), Some(1), "key {key} should be rejected");
        assert!(
            stderr_of(&out).contains(key),
            "the message must name {key}: {}",
            stderr_of(&out)
        );
    }
}

#[test]
fn env_key_not_in_allowlist_is_rejected() {
    let f = Fixture::new("bad-env");
    let bad = good_template().replace(
        r#""hooks": {"#,
        r#""env": { "NOT_SHIPPABLE_ENV_KEY": "1" },
  "hooks": {"#,
    );
    let t = f.template("bad-env", &bad);
    let out = f.run(&t);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_of(&out).contains("NOT_SHIPPABLE_ENV_KEY"),
        "got: {}",
        stderr_of(&out)
    );
}

// An external command that is neither `playbook hook <name>` nor a path
// under the repo root now gets rejected instead of silently skipped: the
// old EXTERNAL_COMMANDS bypass gave zero real protection since every real
// hook command the product ships is `playbook hook <name>`.
#[test]
fn external_non_playbook_non_repo_command_is_rejected() {
    let f = Fixture::new("external");
    let bad = good_template().replace(
        r#""command": "playbook hook rm-workspace-guard""#,
        r#""command": "rtk hook claude""#,
    );
    let t = f.template("external", &bad);
    let out = f.run(&t);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_of(&out).contains("hook command path not found"),
        "got: {}",
        stderr_of(&out)
    );
}

#[test]
fn playbook_hook_command_naming_a_nonexistent_hook_is_rejected() {
    let f = Fixture::new("bad-hook-name");
    let bad = good_template().replace(
        r#""command": "playbook hook rm-workspace-guard""#,
        r#""command": "playbook hook not-a-real-hook""#,
    );
    let t = f.template("bad-hook-name", &bad);
    let out = f.run(&t);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_of(&out).contains("not-a-real-hook"),
        "the message must name the bad hook: {}",
        stderr_of(&out)
    );
}

// The real, tracked settings.shared.json must validate once this change
// regenerates it, exercised the same way the justfile's own recipe does.
#[test]
fn real_settings_shared_json_validates() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let template = repo_root.join("settings.shared.json");
    let perms = repo_root.join("permissions.shared.json");
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["settings", "check"])
        .arg(&template)
        .arg(&perms)
        .arg(repo_root)
        .output()
        .expect("playbook binary should spawn");
    assert!(out.status.success(), "expected exit 0: {}", stderr_of(&out));
}

#[test]
fn both_install_prefixes_resolve() {
    let f = Fixture::new("dual");
    let dual = good_template().replace(
        r#"{ "hooks": [ { "type": "command", "command": "~/.claude/hooks/session-init.sh" } ] }"#,
        r#"{ "hooks": [ { "type": "command", "command": "~/.claude/x.sh" }, { "type": "command", "command": "$HOME/.claude/x.sh" } ] }"#,
    );
    let t = f.template("dual", &dual);
    let out = f.run(&t);
    assert!(
        out.status.success(),
        "both ~/.claude/ and $HOME/.claude/ must map to the repo root: {}",
        stderr_of(&out)
    );
}

#[test]
fn unreadable_and_malformed_inputs_are_named() {
    let f = Fixture::new("io");

    let missing = f.dir.join("nope.json");
    let out = f.run(&missing);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr_of(&out).contains("template not readable"));

    let bad_json = f.template("malformed", "{not json");
    let out = f.run(&bad_json);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr_of(&out).contains("template is not valid JSON"));
}

#[test]
fn missing_permissions_block_is_rejected() {
    let f = Fixture::new("no-perms");
    let bad = r#"{
  "skipAutoPermissionPrompt": false,
  "hooks": {}
}"#;
    let t = f.template("no-perms", bad);
    let out = f.run(&t);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_of(&out).contains(".permissions is missing or not an object"),
        "got: {}",
        stderr_of(&out)
    );
}
