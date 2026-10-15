// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Runs `scripts/check-breaking.sh` against small throwaway repositories: a
//! commit marked breaking since the previous tag needs an entry in
//! breaking-changes.json, a marked pull request needs an entry above the
//! latest tag, and the notes and changelog modes read the same file.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

const EMPTY: &str = r#"{"entries":[]}"#;

struct Repo {
    dir: PathBuf,
}

impl Repo {
    /// A repository with one commit, tagged v0.1.0 and playbook-plugin-v1.0.0.
    fn new() -> Repo {
        assert!(
            Command::new("jq").arg("--version").output().is_ok(),
            "these checks need jq on PATH"
        );
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("breaking-guard-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scripts")).expect("scratch repo");
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/check-breaking.sh");
        fs::copy(script, dir.join("scripts/check-breaking.sh")).expect("copy the script");
        let repo = Repo { dir };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.write("breaking-changes.json", EMPTY);
        repo.write("crates/cli/src/lib.rs", "// one\n");
        repo.write("plugins/playbook/commands/fix.md", "one\n");
        repo.write(
            "CHANGELOG.md",
            "# Changelog\n\n## playbook 1.0.0\n\n- first\n",
        );
        repo.commit("chore: start");
        repo.git(&["tag", "v0.1.0"]);
        repo.git(&["tag", "playbook-plugin-v1.0.0"]);
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
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
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

    /// Changes `rel` and commits it with `msg`.
    fn change(&self, rel: &str, msg: &str) {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        self.write(rel, &format!("// change {n}\n"));
        self.commit(msg);
    }

    fn entry(&self, component: &str, version: &str) {
        self.write(
            "breaking-changes.json",
            &format!(
                r#"{{"entries":[{{"version":"{version}","component":"{component}","summary":"Something changed.","action":"playbook init","migration":null}}]}}"#
            ),
        );
    }

    fn run(&self, args: &[&str], title: &str, body: &str) -> Output {
        Command::new("bash")
            .arg("scripts/check-breaking.sh")
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("PR_TITLE", title)
            .env("PR_BODY", body)
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

fn passes(out: &Output) -> bool {
    out.status.success()
}

#[test]
fn a_release_without_a_breaking_marker_passes() {
    let r = Repo::new();
    r.change("crates/cli/src/lib.rs", "feat(cli): add a flag");

    let out = r.run(&["release", "playbook", "0.2.0"], "", "");

    assert!(passes(&out), "{}", text_of(&out));
}

#[test]
fn a_marked_commit_since_the_previous_tag_needs_an_entry_for_the_version() {
    // Arrange
    let r = Repo::new();
    r.change("crates/cli/src/lib.rs", "feat(cli)!: drop the old flag");

    // Act
    let missing = r.run(&["release", "playbook", "0.2.0"], "", "");
    r.entry("playbook", "0.1.0");
    let wrong_version = r.run(&["release", "playbook", "0.2.0"], "", "");
    r.entry("playbook", "0.2.0");
    let present = r.run(&["release", "playbook", "0.2.0"], "", "");

    // Assert
    assert!(!passes(&missing));
    assert!(
        text_of(&missing).contains("drop the old flag"),
        "{}",
        text_of(&missing)
    );
    assert!(
        text_of(&missing).contains("playbook 0.2.0"),
        "{}",
        text_of(&missing)
    );
    assert!(!passes(&wrong_version));
    assert!(passes(&present), "{}", text_of(&present));
}

#[test]
fn a_breaking_change_footer_counts_as_a_marker() {
    let r = Repo::new();
    r.change(
        "crates/cli/src/lib.rs",
        "feat(cli): rename a key\n\nBREAKING CHANGE: the old key is gone",
    );

    let out = r.run(&["release", "playbook", "0.2.0"], "", "");

    assert!(!passes(&out), "{}", text_of(&out));
}

#[test]
fn a_marker_before_the_previous_tag_or_in_another_component_does_not_count() {
    // Arrange
    let r = Repo::new();
    r.change("crates/cli/src/lib.rs", "feat(cli)!: released already");
    r.git(&["tag", "v0.2.0"]);
    r.change("crates/telemetry/src/lib.rs", "feat(telemetry)!: new port");
    r.change(
        "plugins/playbook/commands/fix.md",
        "feat(plugin)!: new command",
    );

    // Act
    let playbook = r.run(&["release", "playbook", "0.3.0"], "", "");
    let telemetry = r.run(&["release", "telemetry", "0.1.0"], "", "");

    // Assert
    assert!(passes(&playbook), "{}", text_of(&playbook));
    assert!(!passes(&telemetry), "{}", text_of(&telemetry));
}

#[test]
fn a_plugin_release_counts_a_marked_pull_request_title() {
    let r = Repo::new();
    r.change("plugins/playbook/commands/fix.md", "docs(plugin): reword");

    let unmarked = r.run(&["release", "plugin", "1.1.0"], "docs(plugin): reword", "");
    let marked = r.run(
        &["release", "plugin", "2.0.0"],
        "feat(plugin)!: rename fix",
        "",
    );

    assert!(passes(&unmarked), "{}", text_of(&unmarked));
    assert!(!passes(&marked), "{}", text_of(&marked));
    assert!(
        text_of(&marked).contains("pull request"),
        "{}",
        text_of(&marked)
    );
}

#[test]
fn a_marked_pull_request_needs_an_entry_above_the_latest_tag_of_a_component_it_touches() {
    // Arrange
    let r = Repo::new();
    r.git(&["branch", "base"]);
    r.change("crates/cli/src/lib.rs", "wip");
    let title = "feat(cli)!: drop the old flag";

    // Act
    let missing = r.run(&["pr", "base"], title, "");
    r.entry("playbook", "0.1.0");
    let released = r.run(&["pr", "base"], title, "");
    r.entry("plugin", "9.0.0");
    let other_component = r.run(&["pr", "base"], title, "");
    r.entry("playbook", "0.2.0");
    let next = r.run(&["pr", "base"], title, "");
    let footer = r.run(
        &["pr", "base"],
        "feat(cli): x",
        "Text.\n\nBREAKING CHANGE: gone",
    );

    // Assert
    assert!(!passes(&missing), "{}", text_of(&missing));
    assert!(!passes(&released), "{}", text_of(&released));
    assert!(!passes(&other_component), "{}", text_of(&other_component));
    assert!(passes(&next), "{}", text_of(&next));
    assert!(passes(&footer), "{}", text_of(&footer));
}

#[test]
fn an_unmarked_pull_request_passes_and_so_does_one_that_touches_no_component() {
    let r = Repo::new();
    r.git(&["branch", "base"]);
    r.change("docs/guide.md", "docs: x");

    let unmarked = r.run(&["pr", "base"], "docs: x", "");
    let docs_only = r.run(&["pr", "base"], "docs!: x", "");

    assert!(passes(&unmarked), "{}", text_of(&unmarked));
    assert!(passes(&docs_only), "{}", text_of(&docs_only));
}

#[test]
fn notes_print_a_section_only_when_the_release_has_entries() {
    let r = Repo::new();
    r.entry("playbook", "0.2.0");

    let some = r.run(&["notes", "playbook", "0.2.0"], "", "");
    let none = r.run(&["notes", "playbook", "0.3.0"], "", "");

    assert!(passes(&some) && passes(&none));
    assert_eq!(
        String::from_utf8_lossy(&some.stdout),
        "## Breaking changes\n\n- Something changed.\n  Action: playbook init\n\n"
    );
    assert_eq!(String::from_utf8_lossy(&none.stdout), "");
}

#[test]
fn a_plugin_entry_needs_a_breaking_heading_in_its_changelog_section() {
    // Arrange
    let r = Repo::new();
    r.entry("plugin", "1.0.0");

    // Act
    let missing = r.run(&["changelog", "1.0.0"], "", "");
    r.write(
        "CHANGELOG.md",
        "# Changelog\n\n## playbook 1.0.0\n\n### Breaking\n\n- renamed\n",
    );
    let present = r.run(&["changelog", "1.0.0"], "", "");
    let other_version = r.run(&["changelog", "1.1.0"], "", "");

    // Assert
    assert!(!passes(&missing), "{}", text_of(&missing));
    assert!(passes(&present), "{}", text_of(&present));
    assert!(passes(&other_version), "{}", text_of(&other_version));
}

#[test]
fn text_a_shell_ruby_or_markdown_would_run_fails_the_format_check() {
    let r = Repo::new();
    for bad in ["run $(id)", "#{system}", "a `b`", "<b>"] {
        r.write(
            "breaking-changes.json",
            &format!(
                r#"{{"entries":[{{"version":"0.2.0","component":"playbook","summary":"s","action":"{bad}","migration":null}}]}}"#
            ),
        );

        let out = r.run(&["notes", "playbook", "0.2.0"], "", "");

        assert!(!passes(&out), "{bad}: {}", text_of(&out));
    }
}
