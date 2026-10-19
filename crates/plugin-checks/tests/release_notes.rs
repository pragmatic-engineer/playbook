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
    /// A repository with one commit, tagged v0.1.0, and a commit before the
    /// tag that no notes may list.
    fn new() -> Repo {
        assert!(
            Command::new("jq").arg("--version").output().is_ok(),
            "these checks need jq on PATH"
        );
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("release-notes-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scripts")).expect("scratch repo");
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/release-notes.sh");
        fs::copy(script, dir.join("scripts/release-notes.sh")).expect("copy the script");
        let repo = Repo { dir };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.change("crates/cli/src/lib.rs", "feat(cli): before the tag (#1)");
        repo.git(&["tag", "v0.1.0"]);
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

/// A history with one commit of each kind on playbook paths, and commits on
/// the telemetry crate, a shared crate, the plugin, the plugin checks and
/// Cargo.lock.
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
    repo.change(
        "crates/common/src/a.rs",
        "fix(common): share a helper (#18)",
    );
    repo.change(
        "crates/plugin-checks/tests/a.rs",
        "test: check the plugin (#19)",
    );
    repo.change("Cargo.lock", "build(deps): bump a crate (#21)");
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
            r#"["features",["add a flag","add a panel"]],"#,
            r#"["fixes",["repair the parser","reword the fix command","share a helper"]],"#,
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
    assert_eq!(
        repo.query(&notes, r#".date | test("^[0-9]{4}-[0-9]{2}-[0-9]{2}$")"#),
        "true"
    );
}

#[test]
fn the_notes_list_only_the_commits_on_playbook_paths() {
    // Arrange
    let repo = mixed_history();

    // Act
    let playbook = repo.notes("playbook", "0.2.0");

    // Assert: a commit before the previous tag shows nowhere. The plugin
    // ships with playbook. The telemetry crate and the shared crates count,
    // and the plugin checks and Cargo.lock do not.
    assert_eq!(
        repo.query(&playbook, "[.sections[].items[].pr | numbers] | sort"),
        "[10,11,12,14,15,16,17,18]"
    );
}

#[test]
fn the_tag_of_the_version_itself_is_not_the_previous_tag() {
    // Arrange: the release workflow runs on the pushed tag, so the tag of the
    // version exists at HEAD.
    let repo = mixed_history();
    repo.git(&["tag", "v0.2.0"]);

    // Act
    let notes = repo.notes("playbook", "0.2.0");

    // Assert
    assert_eq!(
        repo.query(
            &notes,
            "[.previous_tag, ([.sections[].items[].pr | numbers] | length)]"
        ),
        r#"["v0.1.0",8]"#
    );
}

#[test]
fn versions_compare_as_numbers() {
    // Arrange
    let repo = Repo::new();
    repo.git(&["tag", "v0.9.0"]);
    repo.change("crates/cli/src/a.rs", "fix: one (#40)");
    repo.git(&["tag", "v0.10.0"]);
    repo.change("crates/cli/src/b.rs", "fix: two (#41)");

    // Act
    let notes = repo.notes("playbook", "0.10.1");

    // Assert
    assert_eq!(
        repo.query(&notes, "[.previous_tag, [.sections[].items[].pr]]"),
        r#"["v0.10.0",[41]]"#
    );
}

#[test]
fn merge_commits_are_left_out() {
    // Arrange
    let repo = Repo::new();
    repo.git(&["checkout", "-q", "-b", "topic"]);
    repo.change("crates/cli/src/a.rs", "feat: on a branch (#50)");
    repo.git(&["checkout", "-q", "main"]);
    repo.change("crates/cli/src/b.rs", "fix: on main (#51)");
    repo.git(&[
        "merge",
        "-q",
        "--no-ff",
        "-m",
        "Merge pull request #52 from topic",
        "topic",
    ]);

    // Act
    let notes = repo.notes("playbook", "0.2.0");

    // Assert
    assert_eq!(
        repo.query(&notes, "[.sections[].items[].text]"),
        r#"["on a branch","on main"]"#
    );
}

#[test]
fn a_footer_moves_a_feature_to_breaking_and_takes_its_text() {
    // Arrange: the hyphen form of the footer, on a feat commit with a scope.
    let repo = Repo::new();
    repo.change(
        "crates/cli/src/a.rs",
        "feat(cli): rename the flag (#60)\n\nBody.\n\nBREAKING-CHANGE: use --new; --old is gone\n",
    );
    repo.change("crates/cli/src/b.rs", "fix:no space after the colon (#61)");

    // Act
    let notes = repo.notes("playbook", "0.2.0");

    // Assert: the footer text wins, and a subject without the space after the
    // colon is not a conventional subject.
    assert_eq!(
        repo.query(&notes, "[.sections[] | [.name, .items]]"),
        concat!(
            r#"[["breaking",[{"type":"feat","scope":"cli","text":"use --new; --old is gone","pr":60,"breaking":true}]],"#,
            r#"["other",[{"type":"other","scope":null,"text":"fix:no space after the colon","pr":61,"breaking":false}]]]"#
        )
    );
}

#[test]
fn control_characters_are_removed_from_the_text() {
    // Arrange
    let repo = Repo::new();
    repo.change(
        "crates/cli/src/a.rs",
        "fix: a\ttab and an \u{1b}[31mescape (#70)",
    );

    // Act
    let notes = repo.notes("playbook", "0.2.0");

    // Assert
    assert_eq!(
        repo.query(&notes, ".sections[0].items[0].text"),
        r#""atab and an [31mescape""#
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
fn the_markdown_puts_one_blank_line_between_sections() {
    // Arrange
    let repo = Repo::new();
    repo.change("crates/cli/src/a.rs", "feat(cli): add a flag (#2)");
    repo.change("crates/cli/src/b.rs", "fix(cli): repair the parser (#3)");
    let notes = repo.notes("playbook", "0.2.0");

    // Act
    let md = repo.markdown(&notes);

    // Assert
    assert_eq!(
        md,
        "## Features\n\n- **cli**: add a flag ([#2](https://github.com/o/r/pull/2))\n\n\
         ## Fixes\n\n- **cli**: repair the parser ([#3](https://github.com/o/r/pull/3))\n\n\
         **Full changelog**: https://github.com/o/r/compare/v0.1.0...v0.2.0\n"
    );
}

#[test]
fn the_markdown_skips_empty_sections_and_links_each_pull_request() {
    // Arrange
    let repo = mixed_history();
    let playbook = repo.notes("playbook", "0.2.0");

    // Act
    let playbook_md = repo.markdown(&playbook);

    // Assert
    assert!(playbook_md
        .contains("\n- **telemetry**: add a panel ([#16](https://github.com/o/r/pull/16))\n"));
    assert!(playbook_md
        .ends_with("\n**Full changelog**: https://github.com/o/r/compare/v0.1.0...v0.2.0\n"));
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
    repo.change(
        "crates/plugin-checks/tests/a.rs",
        "test: check the plugin (#19)",
    );

    // Act
    let notes = repo.notes("playbook", "0.2.0");

    // Assert
    assert_eq!(repo.query(&notes, ".sections"), "[]");
    assert!(repo
        .markdown(&notes)
        .starts_with("## Changes\n\nNo commits since v0.1.0 touch this component.\n"));
}

#[test]
fn markup_in_a_subject_is_escaped_in_the_markdown_and_kept_in_the_json() {
    // Arrange
    let repo = Repo::new();
    repo.change(
        "crates/cli/src/a.rs",
        r"fix(cli): keep `code`, *stars*, _under_, <b>tags</b>, [x](y), a|b, ~s~, & and # \ @me (#30)",
    );
    repo.change("crates/cli/src/b.rs", "docs(<x>): y (#31)");

    // Act
    let notes = repo.notes("playbook", "0.2.0");
    let md = repo.markdown(&notes);

    // Assert
    assert_eq!(
        repo.query(&notes, ".sections[0].items[0].text"),
        r#""keep `code`, *stars*, _under_, <b>tags</b>, [x](y), a|b, ~s~, & and # \\ @me""#
    );
    assert!(
        md.contains(
            r"- **cli**: keep `code`, \*stars\*, \_under\_, \<b\>tags\</b\>, \[x\](y), a\|b, \~s\~, \& and \# \\ \@me ([#30]"
        ),
        "{md}"
    );
    assert!(md.contains(r"- docs(\<x\>): y ([#31]"), "{md}");
}

#[test]
fn the_body_starts_with_the_notes() {
    // Arrange
    let repo = mixed_history();
    let out_json = repo.dir.join("out.json");
    let out_path = out_json.to_str().expect("utf-8 path");

    // Act
    let playbook = repo.run(&["body", "playbook", "0.2.0", out_path]);
    let written = fs::read_to_string(&out_json).expect("the json was written");

    // Assert
    assert!(playbook.status.success(), "{}", text_of(&playbook));
    let playbook = String::from_utf8_lossy(&playbook.stdout);
    assert!(playbook.starts_with("## Breaking changes\n"), "{playbook}");
    assert!(written.contains(r#""component": "playbook""#), "{written}");
}

#[test]
fn a_bad_component_or_version_fails() {
    // Arrange
    let repo = Repo::new();

    // Act
    let component = repo.run(&["json", "plugin", "0.1.0"]);
    let telemetry = repo.run(&["json", "telemetry", "0.1.0"]);
    let version = repo.run(&["json", "playbook", "v0.1"]);

    // Assert
    assert!(!component.status.success() && text_of(&component).contains("unknown component"));
    assert!(!telemetry.status.success() && text_of(&telemetry).contains("unknown component"));
    assert!(!version.status.success() && text_of(&version).contains("MAJOR.MINOR.PATCH"));
}

#[test]
fn the_release_commit_of_a_version_is_left_out() {
    // Arrange
    let r = Repo::new();
    r.change("Cargo.toml", "chore(release): playbook 0.1.1 (#40)");
    r.change("crates/cli/src/lib.rs", "chore(deps): bump a crate (#41)");
    r.change(
        "crates/cli/src/main.rs",
        "chore(release): check the tag first (#42)",
    );

    // Act
    let notes = r.notes("playbook", "0.1.1");

    // Assert
    assert_eq!(r.query(&notes, "[.sections[].items[].pr]"), "[41,42]");
}

/// The full message of 6989495b, "feat(setup)!: retire the aliases and
/// claude-override parts (#1119)", as it is on main.
const PR_1119: &str = r#"feat(setup)!: retire the aliases and claude-override parts (#1119)

## Summary

Retires the `aliases` and `claude-override` setup parts (ADR 0044
decisions 1 and 7). Setup stops writing the `eval "$(playbook
shell-init)"` line and the `launcher.claudeOverride` key. The four old
flags and the two `--only` names stay accepted until 1.0, hidden from
`--help`, and do nothing. In a terminal each prints one stderr line that
names ADR 0044, also with `--explain`. Migration 0007 no longer rewrites
old `source` lines. setup.md and install.sh drop the retired words with
a note, install.sh no longer asks the shell wrapper question, and doctor
layer 3 passes without a shell line.

BREAKING CHANGE: `playbook setup --aliases` and `--claude-override` do
nothing now. Start sessions with `pb cc` or `playbook cc`, which load
the playbook system prompt. A plain `claude` no longer goes through the
launcher.

## Notes for reviewers

An rc file that still evaluates `shell-init` keeps working here. The
next PRs make `shell-init` print a comment, delete the now unused rc
writers in shim.rs, and add migration 0018, which removes the old line
after consent. Both land before the next release.

---------

Signed-off-by: igorjs <2278164+igorjs@users.noreply.github.com>
"#;

/// The full message of 845e5a2c, "feat(setup)!: load the system prompt in pb
/// cc by default (#1122)", as it is on main.
const PR_1122: &str = r#"feat(setup)!: load the system prompt in pb cc by default (#1122)

## Summary

`pb cc` and `playbook cc` now load the playbook system prompt by
default, and `setup --yes` turns every part on (ADR 0044 decisions 4, 5,
8 and 9). A new per-user key `systemPrompt.enabled`, default true,
decides it. With false, cc adds no prompt flag and passes your own
prompt flags through, append flags included. Setup stores the answer: a
yes stores true, a no or `--no-system-prompt` stores false, and a run
that asked nothing stores nothing. A plain `claude` still loads no
playbook prompt.

`--yes` keeps the prompt off when the key is false, or on an earlier
install that had none, and prints the `--only system-prompt` command.
Migration 0019 asks those installs, and goes quiet once an answer is
stored. install.sh asks the prompt question on its own and passes
`--no-system-prompt` on a no. With `--yes` it asks nothing and passes no
prompt flag, so a stored no stays, and `--binary-only` passes
`--no-system-prompt`. Doctor layer 3 warns about the old rc lines and
notes a missing `pb`, and layer 4 says whether `pb cc` loads the prompt.

BREAKING CHANGE: `setup --yes` now installs the system prompt and links
`pb`. Pass `--no-system-prompt` or `--no-short-name` to keep them off.

## Notes for reviewers

The `--yes` defaults change for two parts: system-prompt and short-name
go from off to on. Hooks, status line, PATH, telemetry,
collector-service and attribution-off stay on, and security stays behind
its flag. Tests run `pb cc` through a `pb` symlink with a fake `claude`
and assert the argv with the prompt on and off, and run `setup --yes` in
a scratch HOME and check every part.

---------

Signed-off-by: igorjs <2278164+igorjs@users.noreply.github.com>
"#;

#[test]
fn a_footer_over_several_lines_is_kept_whole() {
    // Arrange: the two footers that v0.37.0 cut after their first line.
    let repo = Repo::new();
    repo.change("crates/cli/src/a.rs", PR_1119);
    repo.change("crates/cli/src/b.rs", PR_1122);

    // Act
    let notes = repo.notes("playbook", "0.2.0");
    let md = repo.markdown(&notes);

    // Assert: the lines join with single spaces, the footer stops at the
    // blank line, and backticks stay plain in the json and the markdown.
    assert_eq!(
        repo.query(&notes, "[.sections[] | [.name, [.items[] | [.pr, .text]]]]"),
        concat!(
            r#"[["breaking",[[1119,"`playbook setup --aliases` and `--claude-override` do nothing now. "#,
            r#"Start sessions with `pb cc` or `playbook cc`, which load the playbook system prompt. "#,
            r#"A plain `claude` no longer goes through the launcher."],"#,
            r#"[1122,"`setup --yes` now installs the system prompt and links `pb`. "#,
            r#"Pass `--no-system-prompt` or `--no-short-name` to keep them off."]]]]"#
        )
    );
    assert!(
        md.contains(concat!(
            "- **setup**: `setup --yes` now installs the system prompt and links `pb`. ",
            "Pass `--no-system-prompt` or `--no-short-name` to keep them off. ([#1122]"
        )),
        "{md}"
    );
    assert!(
        md.contains(
            "- **setup**: `playbook setup --aliases` and `--claude-override` do nothing now."
        ),
        "{md}"
    );
}

#[test]
fn a_footer_stops_at_a_run_of_trailers_and_not_at_a_prose_line() {
    // Arrange: "Note: " inside the footer is prose, since the line after it
    // is no trailer. "Fixes #78" and the sign-off are a run of trailers. An
    // indented footer line, as in a quoted example, is no footer.
    let repo = Repo::new();
    repo.change(
        "crates/cli/src/a.rs",
        "feat(cli)!: move it (#80)\n\nBREAKING CHANGE: the a flag is gone.\nNote: run setup\nagain after this.\nFixes #78\nSigned-off-by: t <t@example.invalid>\n",
    );
    repo.change(
        "crates/cli/src/b.rs",
        "fix(cli): quote it (#81)\n\nAn example:\n\n    BREAKING CHANGE: not this one\n",
    );

    // Act
    let notes = repo.notes("playbook", "0.2.0");

    // Assert
    assert_eq!(
        repo.query(&notes, "[.sections[] | [.name, [.items[] | [.pr, .text]]]]"),
        concat!(
            r#"[["breaking",[[80,"the a flag is gone. Note: run setup again after this."]]],"#,
            r#"["fixes",[[81,"quote it"]]]]"#
        )
    );
}

#[test]
fn markup_inside_a_code_span_stays_raw_and_a_backtick_run_escapes_all() {
    // Arrange: a pair of single backticks is a code span. A run of three has
    // no closing run, so GitHub would show the text after it as markup.
    let repo = Repo::new();
    repo.change(
        "crates/cli/src/a.rs",
        "fix(cli): keep `[x](y) <b>` and *s* (#82)",
    );
    repo.change("crates/cli/src/b.rs", "fix(cli): a ```<b>x</b> @me ` (#83)");

    // Act
    let notes = repo.notes("playbook", "0.2.0");
    let md = repo.markdown(&notes);

    // Assert
    assert!(
        md.contains(r"- **cli**: keep `[x](y) <b>` and \*s\* ([#82]"),
        "{md}"
    );
    assert!(
        md.contains(r"- **cli**: a \`\`\`\<b\>x\</b\> \@me \` ([#83]"),
        "{md}"
    );
}

#[test]
fn a_footer_stops_at_the_next_trailer_and_an_odd_backtick_is_escaped() {
    // Arrange
    let repo = Repo::new();
    repo.change(
        "crates/cli/src/a.rs",
        "feat(cli)!: move it (#80)\n\nBREAKING CHANGE: the `a` flag\n  is gone, use b`\nFixes #78\nRefs: #79\nSigned-off-by: t <t@example.invalid>\n",
    );

    // Act
    let notes = repo.notes("playbook", "0.2.0");
    let md = repo.markdown(&notes);

    // Assert
    assert_eq!(
        repo.query(&notes, ".sections[0].items[0].text"),
        r#""the `a` flag is gone, use b`""#
    );
    assert!(
        md.contains(r"- **cli**: the \`a\` flag is gone, use b\` ([#80]"),
        "{md}"
    );
}
