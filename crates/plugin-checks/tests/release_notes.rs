// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Runs `scripts/release-notes.sh` against a small throwaway repository
//! (ADR 0037): the commits since the previous tag of a component that touch
//! its paths, sorted into sections, as release-notes.json and as markdown.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Repo {
    dir: PathBuf,
}

impl Repo {
    /// A repository with one commit, tagged v0.1.0, telemetry-v0.1.0 and
    /// playbook-plugin-v1.0.0, and a commit before the tags that no notes
    /// may list.
    fn new() -> Repo {
        assert!(
            Command::new("jq").arg("--version").output().is_ok(),
            "these checks need jq on PATH"
        );
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("release-notes-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scripts")).expect("scratch repo");
        fs::create_dir_all(dir.join(".github")).expect("scratch repo");
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/release-notes.sh");
        fs::copy(script, dir.join("scripts/release-notes.sh")).expect("copy the script");
        let repo = Repo { dir };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.write(
            ".github/release-notes-upgrade.md",
            "## Upgrade note\n\nRun it.\n",
        );
        repo.change("crates/cli/src/lib.rs", "feat(cli): before the tag (#1)");
        repo.git(&["tag", "v0.1.0"]);
        repo.git(&["tag", "telemetry-v0.1.0"]);
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

    /// Changes `rel` and commits it with `msg`, which may span lines.
    fn change(&self, rel: &str, msg: &str) {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        self.write(rel, &format!("// change {n}\n"));
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", msg]);
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new("bash")
            .arg("scripts/release-notes.sh")
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GITHUB_REPOSITORY", "o/r")
            .current_dir(&self.dir)
            .output()
            .expect("bash")
    }

    /// The release-notes.json of `component` at `version`, written to a file.
    fn notes(&self, component: &str, version: &str) -> PathBuf {
        let out = self.run(&["json", component, version]);
        assert!(out.status.success(), "{}", text_of(&out));
        let path = self.dir.join(format!("{component}-{version}.json"));
        fs::write(&path, &out.stdout).expect("write the notes");
        path
    }

    /// The jq `filter` applied to the JSON at `path`, compact.
    fn query(&self, path: &Path, filter: &str) -> String {
        let out = Command::new("jq")
            .args(["-c", filter])
            .arg(path)
            .output()
            .expect("jq");
        assert!(out.status.success(), "{}", text_of(&out));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn markdown(&self, path: &Path) -> String {
        let out = self.run(&["markdown", path.to_str().expect("utf-8 path")]);
        assert!(out.status.success(), "{}", text_of(&out));
        String::from_utf8_lossy(&out.stdout).into_owned()
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

/// A history with one commit of each kind on playbook paths, one on
/// telemetry paths and one on the plugin.
fn mixed_history() -> Repo {
    let repo = Repo::new();
    repo.change("crates/cli/src/a.rs", "feat(cli): add a flag (#10)");
    repo.change("crates/cli/src/b.rs", "fix: repair the parser (#11)");
    repo.change("crates/cli/src/c.rs", "feat(cli)!: drop the old flag (#12)");
    repo.change(
        "install.sh",
        "refactor(install): move the config\n\nLonger text.\n\nBREAKING CHANGE: the config moved; run playbook init\n",
    );
    repo.change("crates/cli/src/d.rs", "perf: read the file once (#14)");
    repo.change("crates/cli/src/e.rs", "docs(cli): explain the flag (#15)");
    repo.change("crates/cli/src/f.rs", "Update the readme");
    repo.change(
        "crates/telemetry/src/a.rs",
        "feat(telemetry): add a panel (#16)",
    );
    repo.change(
        "plugins/playbook/commands/fix.md",
        "fix(plugin): reword the fix command (#17)",
    );
    repo
}

#[test]
fn commits_land_in_their_section_in_order() {
    // Arrange
    let repo = mixed_history();

    // Act
    let notes = repo.notes("playbook", "0.2.0");

    // Assert
    assert_eq!(
        repo.query(&notes, "[.sections[].name]"),
        r#"["breaking","features","fixes","performance","other"]"#
    );
    assert_eq!(
        repo.query(&notes, "[.sections[] | [.name, [.items[].text]]]"),
        concat!(
            r#"[["breaking",["drop the old flag","the config moved; run playbook init"]],"#,
            r#"["features",["add a flag"]],["fixes",["repair the parser"]],"#,
            r#"["performance",["read the file once"]],"#,
            r#"["other",["explain the flag","Update the readme"]]]"#
        )
    );
    assert_eq!(
        repo.query(&notes, ".sections[0].items[0]"),
        r#"{"type":"feat","scope":"cli","text":"drop the old flag","pr":12,"breaking":true}"#
    );
    assert_eq!(
        repo.query(&notes, ".sections[0].items[1] | [.type, .scope, .pr]"),
        r#"["refactor","install",null]"#
    );
    assert_eq!(
        repo.query(&notes, ".sections[-1].items[1] | [.type, .scope]"),
        r#"["other",null]"#
    );
    assert_eq!(
        repo.query(
            &notes,
            "[.format, .component, .version, .tag, .previous, .previous_tag]"
        ),
        r#"[1,"playbook","0.2.0","v0.2.0","0.1.0","v0.1.0"]"#
    );
    assert!(repo
        .query(&notes, ".date")
        .trim_matches('"')
        .chars()
        .all(|c| c.is_ascii_digit() || c == '-'));
}

#[test]
fn each_component_lists_only_the_commits_on_its_paths() {
    // Arrange
    let repo = mixed_history();

    // Act
    let telemetry = repo.notes("telemetry", "0.2.0");
    let plugin = repo.notes("plugin", "1.0.1");
    let playbook = repo.notes("playbook", "0.2.0");

    // Assert: a commit before the previous tag shows nowhere, and a commit
    // shows only for the component whose paths it touches.
    assert_eq!(
        repo.query(&telemetry, "[.sections[] | [.name, [.items[].pr]]]"),
        r#"[["features",[16]]]"#
    );
    assert_eq!(
        repo.query(&plugin, "[.sections[] | [.name, [.items[].pr]]]"),
        r#"[["fixes",[17]]]"#
    );
    assert_eq!(
        repo.query(
            &playbook,
            "[.sections[].items[].pr] | map(select(. == 1 or . == 16 or . == 17))"
        ),
        "[]"
    );
}

#[test]
fn the_previous_tag_is_the_highest_one_below_the_version() {
    // Arrange: a later tag above the version and a tag that is not a version.
    let repo = mixed_history();
    repo.git(&["tag", "v0.0.9"]);
    repo.git(&["tag", "v0.3.0"]);
    repo.git(&["tag", "v0.1.0-rc.1"]);
    repo.change("crates/cli/src/g.rs", "fix: after the tags (#20)");

    // Act
    let at_030 = repo.notes("playbook", "0.3.1");
    let at_010 = repo.notes("playbook", "0.2.0");

    // Assert
    assert_eq!(
        repo.query(&at_030, "[.previous_tag, [.sections[].items[].pr]]"),
        r#"["v0.3.0",[20]]"#
    );
    assert_eq!(repo.query(&at_010, ".previous_tag"), r#""v0.1.0""#);
}

#[test]
fn a_component_without_a_previous_tag_lists_every_commit() {
    // Arrange
    let repo = Repo::new();
    repo.git(&["tag", "-d", "v0.1.0"]);

    // Act
    let notes = repo.notes("playbook", "0.1.0");

    // Assert
    assert_eq!(
        repo.query(
            &notes,
            "[.previous, .previous_tag, [.sections[].items[].pr]]"
        ),
        "[null,null,[1]]"
    );
    assert!(!repo.markdown(&notes).contains("Full changelog"));
}

#[test]
fn the_markdown_skips_empty_sections_and_links_each_pull_request() {
    // Arrange
    let repo = mixed_history();
    let telemetry = repo.notes("telemetry", "0.2.0");
    let playbook = repo.notes("playbook", "0.2.0");

    // Act
    let telemetry_md = repo.markdown(&telemetry);
    let playbook_md = repo.markdown(&playbook);

    // Assert
    assert_eq!(
        telemetry_md,
        "## Features\n\n- **telemetry**: add a panel ([#16](https://github.com/o/r/pull/16))\n\n\
         **Full changelog**: https://github.com/o/r/compare/telemetry-v0.1.0...telemetry-v0.2.0\n"
    );
    let headings: Vec<&str> = playbook_md
        .lines()
        .filter(|l| l.starts_with("## "))
        .collect();
    assert_eq!(
        headings,
        [
            "## Breaking changes",
            "## Features",
            "## Fixes",
            "## Performance",
            "## Other changes"
        ]
    );
    assert!(playbook_md.contains("\n- **install**: the config moved; run playbook init\n"));
    assert!(playbook_md
        .contains("\n- docs(cli): explain the flag ([#15](https://github.com/o/r/pull/15))\n"));
    assert!(playbook_md.contains("\n- other: Update the readme\n"));
}

#[test]
fn a_component_with_no_commits_says_so() {
    // Arrange
    let repo = Repo::new();

    // Act
    let notes = repo.notes("telemetry", "0.2.0");

    // Assert
    assert_eq!(repo.query(&notes, ".sections"), "[]");
    assert!(repo
        .markdown(&notes)
        .starts_with("## Changes\n\nNo commits since telemetry-v0.1.0 touch this component.\n"));
}

#[test]
fn markup_in_a_subject_is_escaped_in_the_markdown_and_kept_in_the_json() {
    // Arrange
    let repo = Repo::new();
    repo.change(
        "crates/cli/src/a.rs",
        "fix(cli): keep `code`, *stars*, _under_, <b>tags</b>, [x](y), a|b, ~s~, & and # (#30)",
    );

    // Act
    let notes = repo.notes("playbook", "0.2.0");
    let md = repo.markdown(&notes);

    // Assert
    assert_eq!(
        repo.query(&notes, ".sections[0].items[0].text"),
        r#""keep `code`, *stars*, _under_, <b>tags</b>, [x](y), a|b, ~s~, & and #""#
    );
    assert!(
        md.contains(
            r"- **cli**: keep \`code\`, \*stars\*, \_under\_, \<b\>tags\</b\>, \[x\](y), a\|b, \~s\~, \& and \# ([#30]"
        ),
        "{md}"
    );
}

#[test]
fn the_body_puts_the_upgrade_note_or_the_assets_note_above_the_notes() {
    // Arrange
    let repo = mixed_history();
    let out_json = repo.dir.join("out.json");
    let out_path = out_json.to_str().expect("utf-8 path");

    // Act
    let playbook = repo.run(&["body", "playbook", "0.2.0", out_path]);
    let written = fs::read_to_string(&out_json).expect("the json was written");
    let telemetry = repo.run(&["body", "telemetry", "0.2.0", out_path]);
    let plugin = repo.run(&["body", "plugin", "1.0.1", out_path]);

    // Assert
    assert!(playbook.status.success(), "{}", text_of(&playbook));
    let playbook = String::from_utf8_lossy(&playbook.stdout);
    assert!(
        playbook.starts_with("## Upgrade note\n\nRun it.\n\n## Breaking changes\n"),
        "{playbook}"
    );
    assert!(written.contains(r#""component": "playbook""#), "{written}");
    let telemetry = String::from_utf8_lossy(&telemetry.stdout);
    assert!(telemetry.starts_with("## Assets\n"), "{telemetry}");
    assert!(telemetry.contains("\n## Features\n"), "{telemetry}");
    assert!(String::from_utf8_lossy(&plugin.stdout).starts_with("## Fixes\n"));
}

#[test]
fn a_bad_component_or_version_fails() {
    // Arrange
    let repo = Repo::new();

    // Act
    let component = repo.run(&["json", "code", "0.1.0"]);
    let version = repo.run(&["json", "playbook", "v0.1"]);

    // Assert
    assert!(!component.status.success() && text_of(&component).contains("unknown component"));
    assert!(!version.status.success() && text_of(&version).contains("MAJOR.MINOR.PATCH"));
}
