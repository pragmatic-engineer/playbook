// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook init` as `/playbook:setup` runs it, against a scratch `$HOME`:
//! the golden settings a fresh and a contested install must produce, what a
//! plain run leaves alone, and what `--aliases` and `--system-prompt` add.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

const SHELL_INIT_LINE: &str =
    "command -v playbook >/dev/null 2>&1 && eval \"$(playbook shell-init)\"";

struct Sandbox {
    home: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let home = std::env::temp_dir().join(format!(
            "playbook-init-cli-{tag}-{}-{n}",
            playbook::testing::run_id()
        ));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join(".claude")).expect("scratch home");
        Self { home }
    }

    fn init(&self, shell: &str, flags: &[&str]) -> Output {
        let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .arg("init")
            .args(flags)
            .env("HOME", &self.home)
            .env("SHELL", shell)
            .env("CLAUDE_PLUGIN_ROOT", env!("CARGO_MANIFEST_DIR"))
            .current_dir(&self.home)
            .output()
            .expect("playbook should spawn");
        assert!(out.status.success(), "init failed: {}", text(&out));
        out
    }

    fn config_set(&self, key: &str, value: &str) {
        let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(["config", "set", "--global", key, value])
            .env("HOME", &self.home)
            .current_dir(&self.home)
            .output()
            .expect("playbook should spawn");
        assert!(out.status.success(), "config set failed: {}", text(&out));
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.home.join(rel)
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path(rel)).unwrap_or_default()
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.path(rel);
        fs::create_dir_all(path.parent().unwrap()).expect("parent dir");
        fs::write(path, content).expect("scratch file");
    }

    fn count(&self, dir: &str, prefix: &str) -> usize {
        fs::read_dir(self.path(dir))
            .map(|d| {
                d.filter_map(Result::ok)
                    .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
                    .count()
            })
            .unwrap_or(0)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// `(settings_json, settings_base_json)` from a golden fixture, as raw strings.
fn golden(name: &str) -> (String, String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/golden")
        .join(name);
    let doc: Value = serde_json::from_str(&fs::read_to_string(path).expect("fixture")).unwrap();
    let field = |key: &str| doc[key].as_str().expect("fixture string field").to_string();
    (field("settings_json"), field("settings_base_json"))
}

fn json(raw: &str) -> Value {
    serde_json::from_str(raw).expect("valid JSON")
}

#[test]
fn a_clean_install_matches_the_golden_settings() {
    let sb = Sandbox::new("golden-clean");
    let (settings, base) = golden("init.clean-install.json");
    sb.init("/bin/bash", &[]);
    assert_eq!(json(&sb.read(".claude/settings.json")), json(&settings));
    assert_eq!(json(&sb.read(".claude/.settings.base.json")), json(&base));
}

#[test]
fn a_contested_key_matches_the_golden_bytes_and_leaves_a_skip_report() {
    let sb = Sandbox::new("golden-skip");
    let (settings, base) = golden("init.skip-triggering.json");
    sb.write(".claude/settings.json", "{\"cleanupPeriodDays\": 999}\n");
    sb.init("/bin/bash", &[]);
    assert_eq!(sb.read(".claude/settings.json"), settings);
    assert_eq!(sb.read(".claude/.settings.base.json"), base);
    assert!(sb.count(".claude", "settings-merge-skipped.") >= 1);
}

#[test]
fn a_plain_run_places_settings_and_guards_and_touches_no_rc_or_launcher() {
    let sb = Sandbox::new("plain");
    sb.init("/bin/zsh", &[]);
    let settings = json(&sb.read(".claude/settings.json"));
    let hooks = settings["hooks"].to_string();
    for guard in ["rm-workspace-guard", "bg-await-guard", "no-slop-guard"] {
        assert!(
            hooks.contains(&format!("playbook hook {guard}")),
            "{guard} unwired"
        );
        assert!(!sb.path(&format!(".claude/hooks/{guard}.sh")).exists());
    }
    assert!(sb.path(".claude/.settings.base.json").is_file());
    assert!(!sb.path(".config/playbook/statusline.sh").exists());
    assert!(!sb.path(".zshrc").exists());
    assert!(!sb.path(".bashrc").exists());
    assert!(!sb.path(".claude/shell").exists());
    assert!(!sb.path(".config/playbook/shell").exists());
    assert!(!sb
        .path(".config/playbook/prompts/SYSTEM_PROMPT.md")
        .exists());
}

#[test]
fn aliases_wire_only_the_rc_file_of_the_current_shell() {
    for (shell, wired, other) in [
        ("/bin/bash", ".bashrc", ".zshrc"),
        ("/bin/zsh", ".zshrc", ".bashrc"),
    ] {
        let sb = Sandbox::new("aliases");
        let out = sb.init(shell, &["--aliases"]);
        assert!(!text(&out).contains("shim: skipped"), "{}", text(&out));
        assert!(sb.read(wired).lines().any(|l| l == SHELL_INIT_LINE));
        assert!(!sb.path(other).exists());
        assert!(!sb.path(".config/playbook/shell").exists());
    }
}

#[test]
fn a_second_aliases_run_changes_nothing() {
    let sb = Sandbox::new("aliases-twice");
    sb.init("/bin/bash", &["--aliases"]);
    let rc = sb.read(".bashrc");
    let settings = sb.read(".claude/settings.json");
    sb.init("/bin/bash", &["--aliases"]);
    assert_eq!(sb.read(".bashrc"), rc);
    assert_eq!(rc.matches("playbook shell-init").count(), 1);
    assert_eq!(sb.read(".claude/settings.json"), settings);
}

#[test]
fn the_system_prompt_flag_places_the_prompt_where_the_launcher_reads_it() {
    let sb = Sandbox::new("prompt");
    sb.init("/bin/bash", &["--system-prompt"]);
    let shipped =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("prompts/SYSTEM_PROMPT.md"))
            .unwrap();
    assert_eq!(
        sb.read(".config/playbook/prompts/SYSTEM_PROMPT.md"),
        shipped
    );
    assert!(!sb.path(".claude/prompts").exists());
}

#[test]
fn a_plain_run_refreshes_an_installed_prompt_and_never_adds_one() {
    let sb = Sandbox::new("prompt-refresh");
    sb.write(
        ".config/playbook/prompts/SYSTEM_PROMPT.md",
        "a stale copy\n",
    );
    sb.init("/bin/bash", &[]);
    let shipped =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("prompts/SYSTEM_PROMPT.md"))
            .unwrap();
    assert_eq!(
        sb.read(".config/playbook/prompts/SYSTEM_PROMPT.md"),
        shipped
    );

    let fresh = Sandbox::new("prompt-absent");
    fresh.init("/bin/bash", &[]);
    assert!(!fresh
        .path(".config/playbook/prompts/SYSTEM_PROMPT.md")
        .exists());
}

#[test]
fn a_custom_settings_key_survives() {
    let sb = Sandbox::new("merge");
    sb.write(
        ".claude/settings.json",
        "{\"my_custom_key\":\"sentinel_value\"}\n",
    );
    sb.init("/bin/bash", &[]);
    assert_eq!(
        json(&sb.read(".claude/settings.json"))["my_custom_key"],
        "sentinel_value"
    );
}

#[test]
fn a_plain_run_twice_is_byte_identical_and_makes_no_new_backups() {
    let sb = Sandbox::new("idempotent");
    sb.init("/bin/bash", &[]);
    let settings = sb.read(".claude/settings.json");
    let base = sb.read(".claude/.settings.base.json");
    let backups = sb.count(".claude", "settings.json.bak.");
    let skips = sb.count(".claude", "settings-merge-skipped.");
    let again = text(&sb.init("/bin/bash", &[]));
    for step in ["settings", "hooks"] {
        assert!(
            again.contains(&format!("{step}: ok")),
            "{step} rewrote: {again}"
        );
    }
    assert_eq!(sb.read(".claude/settings.json"), settings);
    assert_eq!(sb.read(".claude/.settings.base.json"), base);
    assert_eq!(sb.count(".claude", "settings.json.bak."), backups);
    assert_eq!(sb.count(".claude", "settings-merge-skipped."), skips);
}

#[test]
fn an_old_launcher_line_is_migrated_and_the_users_lines_and_blanks_survive() {
    for old in [
        "source \"$HOME/.claude/shell/cc.zsh\"",
        "source \"$HOME/.claude/shell/zsh/cc.zsh\"",
    ] {
        let sb = Sandbox::new("migrate");
        sb.write(
            ".zshrc",
            &format!("export FOO=1\n\nexport BAR=2\n\n# playbook launchers (cc/ccd)\n{old}\n"),
        );
        sb.init("/bin/zsh", &["--aliases"]);
        let rc = sb.read(".zshrc");
        assert!(!rc.contains(old), "{rc}");
        assert_eq!(rc.matches("playbook shell-init").count(), 1, "{rc}");
        assert_eq!(rc.matches("launchers (ccc/ccd)").count(), 1, "{rc}");
        assert!(rc.lines().any(|l| l == SHELL_INIT_LINE), "{rc}");
        assert!(rc.starts_with("export FOO=1\n\nexport BAR=2\n"), "{rc}");

        sb.init("/bin/zsh", &["--aliases"]);
        assert_eq!(sb.read(".zshrc"), rc);
    }
}

#[test]
fn no_hooks_leaves_every_hook_entry_out_and_says_so() {
    let h = Sandbox::new("no-hooks");
    let out = h.init("/bin/zsh", &["--no-hooks"]);
    let t = text(&out);
    assert!(t.contains("hooks") && t.contains("skipped"), "{t}");
    let settings = h.read(".claude/settings.json");
    assert!(!settings.contains("playbook hook"), "{settings}");
    assert!(
        settings.contains("cleanupPeriodDays"),
        "shared settings still merged"
    );
}

#[test]
fn no_hooks_keeps_the_hooks_a_user_already_has() {
    let h = Sandbox::new("keep-hooks");
    h.write(
        ".claude/settings.json",
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo mine"}]}]}}"#,
    );
    h.init("/bin/zsh", &["--no-hooks"]);
    let settings = h.read(".claude/settings.json");
    assert!(settings.contains("echo mine"), "{settings}");
    assert!(!settings.contains("playbook hook"), "{settings}");
}

#[test]
fn hooks_are_wired_by_default_and_a_rerun_with_no_hooks_is_stable() {
    let h = Sandbox::new("hooks-default");
    h.init("/bin/zsh", &[]);
    assert!(h.read(".claude/settings.json").contains("playbook hook"));
    let before = h.read(".claude/settings.json");
    h.init("/bin/zsh", &["--no-hooks"]);
    assert_eq!(
        h.read(".claude/settings.json"),
        before,
        "existing hooks stay"
    );
}

#[test]
fn a_run_without_a_terminal_uses_defaults_and_never_blocks() {
    let h = Sandbox::new("defaults");
    let out = h.init("/bin/zsh", &[]);
    let t = text(&out);
    assert!(t.contains("hooks"), "{t}");
    assert!(t.contains("shim"), "{t}");
}

#[test]
fn a_no_flag_overrides_its_yes_twin() {
    let h = Sandbox::new("override");
    let t = text(&h.init("/bin/zsh", &["--aliases", "--no-aliases"]));
    assert!(t.contains("not installed"), "{t}");
}

const SECURITY_ENV: &str = "DISABLE_AUTOUPDATER";

fn has_security(settings: &Value) -> bool {
    settings.get("permissions").is_some() || settings["env"].get(SECURITY_ENV).is_some()
}

#[test]
fn a_plain_init_applies_no_security_defaults() {
    let sb = Sandbox::new("sec-off");
    let out = sb.init("/bin/bash", &[]);
    let settings = json(&sb.read(".claude/settings.json"));
    assert!(!has_security(&settings), "{settings}");
    assert!(settings["env"].get("DO_NOT_TRACK").is_some(), "{settings}");
    assert!(text(&out).contains("--security"), "{}", text(&out));
}

#[test]
fn the_security_flag_applies_the_defaults_and_a_rerun_is_stable() {
    let sb = Sandbox::new("sec-flag");
    sb.init("/bin/bash", &["--security"]);
    let first = sb.read(".claude/settings.json");
    let settings = json(&first);
    assert_eq!(settings["env"][SECURITY_ENV], "1");
    assert_eq!(settings["permissions"]["deny"][0], "Read(**/.env)");
    assert!(settings["permissions"]["ask"]
        .to_string()
        .contains("Bash(python3:*)"));
    let backups = sb.count(".claude", "settings.json.bak.");

    sb.init("/bin/bash", &["--security"]);
    assert_eq!(sb.read(".claude/settings.json"), first);
    assert_eq!(sb.count(".claude", "settings.json.bak."), backups);
}

#[test]
fn the_config_key_applies_the_defaults_and_no_security_overrides_it() {
    let sb = Sandbox::new("sec-config");
    sb.config_set("security.defaults", "true");
    sb.init("/bin/bash", &[]);
    assert!(has_security(&json(&sb.read(".claude/settings.json"))));

    let other = Sandbox::new("sec-config-override");
    other.config_set("security.defaults", "true");
    other.init("/bin/bash", &["--no-security"]);
    assert!(!has_security(&json(&other.read(".claude/settings.json"))));
}

#[test]
fn a_plain_init_never_strips_security_entries_the_user_already_has() {
    let sb = Sandbox::new("sec-keep");
    sb.init("/bin/bash", &["--security"]);
    let applied = sb.read(".claude/settings.json");
    let base = sb.read(".claude/.settings.base.json");

    sb.init("/bin/bash", &[]);
    assert_eq!(sb.read(".claude/settings.json"), applied);
    assert_eq!(sb.read(".claude/.settings.base.json"), base);

    // A hand-written block survives too, and a later opt-in does not
    // overwrite it.
    let mine = Sandbox::new("sec-mine");
    mine.write(
        ".claude/settings.json",
        r#"{"permissions":{"allow":["Read"]},"env":{"DISABLE_AUTOUPDATER":"0","MINE":"1"}}"#,
    );
    mine.init("/bin/bash", &[]);
    let settings = json(&mine.read(".claude/settings.json"));
    assert_eq!(settings["permissions"], json(r#"{"allow":["Read"]}"#));
    assert_eq!(settings["env"][SECURITY_ENV], "0");
    assert_eq!(settings["env"]["MINE"], "1");
    mine.init("/bin/bash", &["--security"]);
    let settings = json(&mine.read(".claude/settings.json"));
    assert_eq!(settings["permissions"], json(r#"{"allow":["Read"]}"#));
}
