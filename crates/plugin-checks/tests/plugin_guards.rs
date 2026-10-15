// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Runs `scripts/check-plugin.sh` against small throwaway repositories: the
//! payload guard rejects a file that is not markdown, and the version guard
//! asks for a bump and a changelog section whenever the plugin changes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Repo {
    dir: PathBuf,
}

impl Repo {
    fn new() -> Repo {
        assert!(
            Command::new("jq").arg("--version").output().is_ok(),
            "these checks need jq on PATH"
        );
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("plugin-guards-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scripts")).expect("scratch repo");
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/check-plugin.sh");
        fs::copy(script, dir.join("scripts/check-plugin.sh")).expect("copy the script");
        let repo = Repo { dir };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.write(
            "plugins/playbook/.claude-plugin/plugin.json",
            r#"{"name":"playbook","version":"1.0.0"}"#,
        );
        repo.write("plugins/playbook/commands/fix.md", "one\n");
        repo.write(
            "CHANGELOG.md",
            "# Changelog\n\n## playbook 1.0.0\n\n- first\n",
        );
        repo.commit("base");
        repo
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.dir.join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        fs::write(path, text).expect("write");
    }

    fn git(&self, args: &[&str]) {
        let out = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .current_dir(&self.dir)
            .output()
            .expect("git");
        assert!(out.status.success(), "{args:?}: {}", text_of(&out));
    }

    fn commit(&self, msg: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", msg]);
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new("bash")
            .arg("scripts/check-plugin.sh")
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .current_dir(&self.dir)
            .output()
            .expect("bash")
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn text_of(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn the_payload_guard_passes_for_markdown_and_the_manifest() {
    let r = Repo::new();

    let out = r.run(&["payload"]);

    assert!(out.status.success(), "{}", text_of(&out));
}

#[test]
fn the_payload_guard_rejects_a_file_that_is_not_markdown() {
    let r = Repo::new();
    r.write("plugins/playbook/skills/x/run.sh", "echo hi\n");
    r.commit("script");

    let out = r.run(&["payload"]);

    assert!(!out.status.success());
    assert!(text_of(&out).contains("run.sh"), "{}", text_of(&out));
}

#[test]
fn a_plugin_change_without_a_version_bump_fails() {
    let r = Repo::new();
    r.write("plugins/playbook/commands/fix.md", "two\n");
    r.commit("edit");

    let out = r.run(&["version", "HEAD~1"]);

    assert!(!out.status.success());
    assert!(
        text_of(&out).contains("bump the version"),
        "{}",
        text_of(&out)
    );
}

#[test]
fn a_bump_without_a_changelog_section_fails() {
    let r = Repo::new();
    r.write("plugins/playbook/commands/fix.md", "two\n");
    r.write(
        "plugins/playbook/.claude-plugin/plugin.json",
        r#"{"name":"playbook","version":"1.0.1"}"#,
    );
    r.commit("bump only");

    let out = r.run(&["version", "HEAD~1"]);

    assert!(!out.status.success());
    assert!(
        text_of(&out).contains("starts at playbook 1.0.0"),
        "{}",
        text_of(&out)
    );
}

#[test]
fn a_bump_with_a_changelog_section_passes() {
    let r = Repo::new();
    r.write("plugins/playbook/commands/fix.md", "two\n");
    r.write(
        "plugins/playbook/.claude-plugin/plugin.json",
        r#"{"name":"playbook","version":"1.0.1"}"#,
    );
    r.write(
        "CHANGELOG.md",
        "# Changelog\n\n## playbook 1.0.1\n\n- fix\n\n## playbook 1.0.0\n\n- first\n",
    );
    r.commit("release");

    let out = r.run(&["version", "HEAD~1"]);

    assert!(out.status.success(), "{}", text_of(&out));
}

#[test]
fn a_lower_version_fails() {
    let r = Repo::new();
    r.write("plugins/playbook/commands/fix.md", "two\n");
    r.write(
        "plugins/playbook/.claude-plugin/plugin.json",
        r#"{"name":"playbook","version":"0.9.0"}"#,
    );
    r.write(
        "CHANGELOG.md",
        "# Changelog\n\n## playbook 0.9.0\n\n- back\n",
    );
    r.commit("downgrade");

    let out = r.run(&["version", "HEAD~1"]);

    assert!(!out.status.success());
    assert!(
        text_of(&out).contains("bump the version"),
        "{}",
        text_of(&out)
    );
}

#[test]
fn a_change_outside_the_plugin_needs_no_bump() {
    let r = Repo::new();
    r.write("README.md", "hello\n");
    r.commit("readme");

    let out = r.run(&["version", "HEAD~1"]);

    assert!(out.status.success(), "{}", text_of(&out));
}

#[test]
fn the_payload_guard_rejects_a_symlink_named_like_markdown() {
    let r = Repo::new();
    std::os::unix::fs::symlink(
        "/etc/hosts",
        r.dir.join("plugins/playbook/commands/link.md"),
    )
    .expect("symlink");
    r.commit("link");

    let out = r.run(&["payload"]);

    assert!(!out.status.success());
    assert!(text_of(&out).contains("plain file"), "{}", text_of(&out));
}

#[test]
fn the_payload_guard_rejects_markdown_nested_too_deep() {
    let r = Repo::new();
    r.write("plugins/playbook/commands/a/b.md", "x\n");
    r.commit("deep");

    let out = r.run(&["payload"]);

    assert!(!out.status.success());
}

#[test]
fn the_payload_guard_accepts_a_path_with_non_ascii_letters() {
    let r = Repo::new();
    r.write("plugins/playbook/skills/caf\u{e9}/SKILL.md", "x\n");
    r.commit("accent");

    let out = r.run(&["payload"]);

    assert!(out.status.success(), "{}", text_of(&out));
}

#[test]
fn an_invalid_manifest_is_reported() {
    let r = Repo::new();
    r.write("plugins/playbook/.claude-plugin/plugin.json", "{not json");
    r.commit("broken");

    let out = r.run(&["version", "HEAD~1"]);

    assert!(!out.status.success());
}

const PLUGIN_ENTRY: &str =
    r#"{"version":"1.1.0","component":"plugin","summary":"s","action":"a","migration":null}"#;

fn record(entries: &str) -> String {
    format!(r#"{{"entries":[{entries}]}}"#)
}

#[test]
fn the_mirror_guard_passes_with_no_plugin_entries_and_no_key() {
    // Arrange
    let r = Repo::new();
    r.write(
        "breaking-changes.json",
        &record(r#"{"version":"1.0.0","component":"playbook","summary":"s","action":"a","migration":null}"#),
    );

    // Act
    let out = r.run(&["mirror"]);

    // Assert
    assert!(out.status.success(), "{}", text_of(&out));
}

#[test]
fn the_mirror_guard_fails_when_a_plugin_entry_is_not_in_the_manifest() {
    // Arrange
    let r = Repo::new();
    r.write("breaking-changes.json", &record(PLUGIN_ENTRY));

    // Act
    let out = r.run(&["mirror"]);

    // Assert
    assert!(!out.status.success());
    assert!(
        text_of(&out).contains("metadata.breakingChanges"),
        "{}",
        text_of(&out)
    );
}

#[test]
fn the_mirror_guard_passes_when_the_manifest_lists_the_same_entries() {
    // Arrange
    let r = Repo::new();
    r.write("breaking-changes.json", &record(PLUGIN_ENTRY));
    r.write(
        "plugins/playbook/.claude-plugin/plugin.json",
        r#"{"name":"playbook","version":"1.0.0","metadata":{"breakingChanges":[
          {"migration":null,"action":"a","summary":"s","component":"plugin","version":"1.1.0"}]}}"#,
    );

    // Act
    let out = r.run(&["mirror"]);

    // Assert
    assert!(out.status.success(), "{}", text_of(&out));
}

#[test]
fn the_mirror_guard_fails_when_the_manifest_has_an_entry_the_record_lacks() {
    // Arrange
    let r = Repo::new();
    r.write("breaking-changes.json", &record(""));
    r.write(
        "plugins/playbook/.claude-plugin/plugin.json",
        &format!(r#"{{"name":"playbook","version":"1.0.0","metadata":{{"breakingChanges":[{PLUGIN_ENTRY}]}}}}"#),
    );

    // Act
    let out = r.run(&["mirror"]);

    // Assert
    assert!(!out.status.success());
}
