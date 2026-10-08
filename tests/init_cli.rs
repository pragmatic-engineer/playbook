// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

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
            std::process::id()
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
    assert!(sb.path(".config/playbook/statusline.sh").is_file());
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
    sb.init("/bin/bash", &[]);
    let settings = sb.read(".claude/settings.json");
    let base = sb.read(".claude/.settings.base.json");
    let backups = sb.count(".claude", "settings.json.bak.");
    let skips = sb.count(".claude", "settings-merge-skipped.");
    sb.init("/bin/bash", &[]);
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
        assert_eq!(rc.matches("launchers (cc/ccd)").count(), 1, "{rc}");
        assert!(rc.starts_with("export FOO=1\n\nexport BAR=2\n"), "{rc}");

        sb.init("/bin/zsh", &["--aliases"]);
        assert_eq!(sb.read(".zshrc"), rc);
    }
}
