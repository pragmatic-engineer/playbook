// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook commands`: the list of every visible command, built at run time
//! from the clap tree, so it shows exactly the commands this binary has.
//!
//! The clap tree gives the names, the flags and the purpose (the first
//! sentence of each command's doc comment). `META` adds what clap cannot
//! hold: who may run the command and, where the doc comment is too long or
//! too vague, a shorter purpose. A test walks the whole tree and fails when a
//! visible command has no entry here, or an entry names a command that is
//! gone, so a new command cannot ship without one. Hidden commands, which
//! include the deprecated names, are never listed. The intent ids come from
//! the tool map (`tools_map::TOOLS`), so they are not repeated here.

use clap::{Arg, Command};
use std::fmt::Write as _;

/// Who may run a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Changes the repo, the worktree or playbook's own state. A model runs it
    /// when the task needs it.
    Default,
    /// Reads and prints. A model may run it at any time.
    ReadOnly,
    /// Writes the user's home or Claude Code settings, needs their consent,
    /// spends money, or starts a session. Only the user runs it, typed with
    /// `!` in the prompt.
    UserOnly,
}

impl Access {
    /// The word `--json` prints.
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Default => "default",
            Access::ReadOnly => "read-only",
            Access::UserOnly => "user-only",
        }
    }
}

/// What the clap tree cannot say about one command.
pub struct Meta {
    /// The command words after `playbook`, for example `disk clean`.
    pub path: &'static str,
    pub access: Access,
    /// Used instead of the doc comment's first sentence when that is too
    /// long or too vague for one line.
    pub purpose: Option<&'static str>,
}

impl Meta {
    const fn purpose(self, text: &'static str) -> Meta {
        Meta {
            purpose: Some(text),
            ..self
        }
    }
}

const fn free(path: &'static str) -> Meta {
    Meta {
        path,
        access: Access::Default,
        purpose: None,
    }
}

const fn ro(path: &'static str) -> Meta {
    Meta {
        path,
        access: Access::ReadOnly,
        purpose: None,
    }
}

const fn user(path: &'static str) -> Meta {
    Meta {
        path,
        access: Access::UserOnly,
        purpose: None,
    }
}

/// One entry for every visible command and subcommand, sorted by path. A
/// group's own entry describes the group when it runs with no subcommand.
pub const META: &[Meta] = &[
    free("adr").purpose("Helpers for ADRs, such as the next free record number"),
    free("adr next"),
    ro("agents").purpose("Check agent definition files and their effort variants"),
    ro("agents check"),
    ro("agents variants"),
    ro("atlassian").purpose("Check the Atlassian tools the `playbook:atlassian` skill uses"),
    ro("atlassian check").purpose("Check the `acli` version and whether `twg` is installed"),
    ro("breaking"),
    user("cc"),
    free("cc bust-cache"),
    user("cc clean").purpose("Launcher mode: clean up and resume a session"),
    user("cc fresh").purpose("Launcher mode: start a fresh session"),
    user("cc launch"),
    ro("cc list"),
    free("cc prune"),
    user("cc raw").purpose("Launcher mode: resume a session as it was"),
    free("cc worktree"),
    user("ccd"),
    ro("ci"),
    free("code"),
    ro("code context"),
    free("code index"),
    ro("code outline"),
    ro("code read"),
    ro("code refs"),
    ro("code search"),
    ro("code status"),
    ro("code symbol"),
    ro("commands"),
    free("commit").purpose("Prepare and run the signed commit of `/playbook:commit-and-push`"),
    free("commit prepare"),
    free("commit run"),
    free("config").purpose("Read and change playbook settings in the repo, org or global tier"),
    ro("config export"),
    ro("config get"),
    user("config import"),
    ro("config list"),
    free("config set").purpose("Change a setting; protected keys such as `disk.*` are the user's"),
    free("deliver"),
    user("deps"),
    user("deps ensure").purpose("Install the missing tools of a Brewfile with Homebrew"),
    free("disk"),
    user("disk clean"),
    user("disk release"),
    user("disk reserve").purpose("Write a 1 GB ballast file that `disk release` gives back"),
    ro("disk status").purpose("Show free space, the thresholds and the biggest folders"),
    ro("doc").purpose("Read a PDF or Office file as text with anydoc"),
    ro("doc read"),
    ro("doc status"),
    free("doctor").purpose("Check the playbook setup, or read one fact the check uses"),
    ro("doctor check"),
    ro("doctor hook-commands"),
    ro("doctor hook-commands-for-event"),
    ro("doctor hook-commands-matching"),
    ro("doctor models"),
    ro("doctor pending-migrations"),
    ro("doctor plugin-version"),
    ro("doctor statusline-command"),
    user("effort"),
    user("effort apply"),
    free("effort choose"),
    ro("effort list"),
    ro("effort log"),
    user("effort reset"),
    ro("effort resolve"),
    ro("effort suggest"),
    user("eval"),
    user("eval bench"),
    user("eval review-triage").purpose("Run the review triage classifier on known cases"),
    free("gate"),
    ro("gate check"),
    free("gate record"),
    free("gate snapshot"),
    free("glossary").purpose("Append one term to the repo's `GLOSSARY.md`"),
    free("glossary add"),
    free("handoff").purpose("Save and show the notes that carry a session to the next"),
    ro("handoff check"),
    ro("handoff facts"),
    free("handoff save"),
    ro("handoff show"),
    ro("handoff status"),
    ro("handoff template"),
    ro("hooks"),
    ro("hooks list"),
    ro("learn").purpose("Gather repo facts for the `/playbook:learn-project` collectors"),
    ro("learn collect"),
    ro("learn preflight"),
    ro("manifest"),
    ro("manifest check"),
    free("memory"),
    free("memory context").purpose("Print the memory facts, edges and anchors in scope for a repo"),
    ro("memory export"),
    user("memory import-claude"),
    free("memory rebuild"),
    ro("memory stats"),
    user("migrate"),
    free("mode"),
    free("mode ask"),
    user("mode auto"),
    ro("mode status"),
    free("path").purpose("Print a playbook storage folder of this repo, or the plugin folder"),
    free("plan").purpose("Plan helpers: checkpoints, Work Unit briefs and scratch cleanup"),
    free("plan brief"),
    free("plan checkpoint"),
    free("plan cleanup"),
    ro("plans"),
    free("pr"),
    ro("pr ci-wait"),
    ro("pr comments"),
    free("pr create"),
    ro("pr land-wait"),
    free("pr merge"),
    free("pr prepare"),
    free("pr reply"),
    ro("pr review-triage"),
    ro("pr rules"),
    ro("pr stack"),
    ro("prose"),
    ro("prose check"),
    free("review").purpose("Prepare a review: size the diff, pick a depth, run checks"),
    free("review checks"),
    free("review prepare"),
    ro("review size"),
    ro("review suggest"),
    ro("review triage-merge"),
    ro("route"),
    ro("run-context").purpose("Read the run mode and effort ceiling for a command's Step 0"),
    free("sanitize"),
    free("sanitize commit-msg"),
    free("sanitize pr-text"),
    free("segment"),
    free("segment branch"),
    free("segment resplit"),
    ro("segment size"),
    ro("sessions").purpose("List recent sessions with their branch, PR, state and cost"),
    ro("sessions list"),
    free("settings"),
    ro("settings check"),
    free("settings gen"),
    user("setup"),
    ro("state"),
    ro("state list"),
    ro("statusline"),
    ro("tools"),
    ro("tools map"),
    user("trust"),
    user("uninstall"),
    user("update"),
    ro("usage"),
    user("usage collect"),
    ro("usage doctor"),
    user("usage prune"),
    ro("usage status"),
    ro("version"),
    free("worktree"),
    free("worktree remove"),
    free("worktree review").purpose("Create or remove the locked worktrees a review runs in"),
    free("worktree review setup")
        .purpose("Fetch a PR head into a locked worktree and print its path"),
    free("worktree review teardown"),
    free("worktree sweep"),
];

/// The first line `playbook commands` prints.
pub const LEGEND: &str = "playbook commands: one line per command. `(read-only)` runs freely. `(user-only)`, or a name ending in `!`, is for the user: never run it yourself, ask them to type it with `!`. `playbook commands <group>` gives flags, `playbook <command> --help` the details.";

/// The entry for `path`, if any.
pub fn meta(path: &str) -> Option<&'static Meta> {
    META.iter().find(|m| m.path == path)
}

/// The visible subcommands of `cmd`, in `--help` order.
pub fn visible(cmd: &Command) -> impl Iterator<Item = &Command> {
    cmd.get_subcommands()
        .filter(|s| !s.is_hide_set() && s.get_name() != "help")
}

/// One visible command with its path.
pub struct Entry<'a> {
    pub path: String,
    pub cmd: &'a Command,
}

/// Every visible command under `cmd`, depth first, `cmd` itself excluded.
pub fn walk<'a>(cmd: &'a Command, prefix: &str, out: &mut Vec<Entry<'a>>) {
    for sub in visible(cmd) {
        let path = if prefix.is_empty() {
            sub.get_name().to_string()
        } else {
            format!("{prefix} {}", sub.get_name())
        };
        out.push(Entry {
            path: path.clone(),
            cmd: sub,
        });
        walk(sub, &path, out);
    }
}

/// The clap tree, built once per call.
pub fn tree() -> Command {
    let mut cmd = <crate::Cli as clap::CommandFactory>::command();
    cmd.build();
    cmd
}

/// The one-line purpose of a command: the override, or the first sentence of
/// its doc comment.
pub fn purpose(path: &str, cmd: &Command) -> String {
    if let Some(text) = meta(path).and_then(|m| m.purpose) {
        return text.to_string();
    }
    let about = cmd.get_about().map(|a| a.to_string()).unwrap_or_default();
    let first = about.lines().next().unwrap_or_default();
    let sentence = first.split(". ").next().unwrap_or_default();
    sentence.trim().trim_end_matches('.').to_string()
}

fn access(path: &str) -> Access {
    meta(path).map_or(Access::Default, |m| m.access)
}

/// Every runnable leaf below `path` (the command itself when it has none).
fn leaves<'a>(path: &str, cmd: &'a Command) -> Vec<(String, &'a Command)> {
    let mut all = Vec::new();
    walk(cmd, path, &mut all);
    let leaves: Vec<_> = all
        .into_iter()
        .filter(|e| visible(e.cmd).next().is_none())
        .map(|e| (e.path, e.cmd))
        .collect();
    if leaves.is_empty() {
        vec![(path.to_string(), cmd)]
    } else {
        leaves
    }
}

/// The tag a line ends with, from the access of every name it covers.
fn tag(accesses: &[Access]) -> &'static str {
    if accesses.iter().all(|a| *a == Access::ReadOnly) {
        " (read-only)"
    } else if accesses.iter().all(|a| *a == Access::UserOnly) {
        " (user-only)"
    } else {
        ""
    }
}

fn bang(access: Access) -> &'static str {
    if access == Access::UserOnly {
        "!"
    } else {
        ""
    }
}

/// The overview line of one top-level command: its name, its subcommands,
/// its purpose and a tag. In a line with mixed access, each user-only name
/// ends in `!`.
fn overview_line(cmd: &Command) -> String {
    let name = cmd.get_name();
    let subs: Vec<&Command> = visible(cmd).collect();
    let mut covered: Vec<Access> = leaves(name, cmd).iter().map(|(p, _)| access(p)).collect();
    // A group that also runs with no subcommand (`doctor --fix`, `cc`)
    // counts as a name of its own.
    if !subs.is_empty() && !cmd.is_subcommand_required_set() {
        covered.push(access(name));
    }
    let tag = tag(&covered);
    let mark = |path: &str| {
        if tag.is_empty() {
            bang(access(path))
        } else {
            ""
        }
    };
    let runs_bare = !subs.is_empty() && !cmd.is_subcommand_required_set();
    let mut line = format!("{name}{}", if runs_bare { mark(name) } else { "" });
    if !subs.is_empty() {
        let names: Vec<String> = subs
            .iter()
            .map(|s| {
                let path = format!("{name} {}", s.get_name());
                let all_user = leaves(&path, s)
                    .iter()
                    .all(|(p, _)| access(p) == Access::UserOnly);
                let flag = if tag.is_empty() && all_user { "!" } else { "" };
                format!("{}{flag}", s.get_name())
            })
            .collect();
        let _ = write!(line, " {}", names.join("|"));
    }
    let _ = write!(line, ": {}{tag}", purpose(name, cmd));
    line
}

/// One argument as usage text: `<NAME>`, `--flag` or `--flag <VALUE>`, in
/// brackets when optional.
fn arg_text(arg: &Arg) -> String {
    let value = || {
        arg.get_value_names()
            .and_then(|v| v.first().map(ToString::to_string))
            .unwrap_or_else(|| arg.get_id().to_string().to_uppercase())
    };
    let text = if arg.is_positional() {
        format!("<{}>", value())
    } else {
        let mut t = match (arg.get_long(), arg.get_short()) {
            (Some(long), _) => format!("--{long}"),
            (None, Some(short)) => format!("-{short}"),
            (None, None) => arg.get_id().to_string(),
        };
        if arg.get_action().takes_values() {
            let _ = write!(t, " <{}>", value());
        }
        t
    };
    if arg.is_required_set() {
        text
    } else {
        format!("[{text}]")
    }
}

/// The usage text of a command's own arguments, help and version left out.
pub fn args(cmd: &Command) -> Vec<String> {
    cmd.get_arguments()
        .filter(|a| !a.is_hide_set() && !matches!(a.get_id().as_str(), "help" | "version"))
        .map(arg_text)
        .collect()
}

/// The detail line of one command: path, arguments, purpose and tag.
fn detail_line(path: &str, cmd: &Command) -> String {
    let mut line = path.to_string();
    for a in args(cmd) {
        let _ = write!(line, " {a}");
    }
    let tag = match access(path) {
        Access::ReadOnly => " (read-only)",
        Access::UserOnly => " (user-only)",
        Access::Default => "",
    };
    let _ = write!(line, ": {}{tag}", purpose(path, cmd));
    line
}

/// Finds the command at `words` (for example `["worktree", "review"]`).
fn find<'a>(root: &'a Command, words: &[String]) -> Option<&'a Command> {
    let mut cmd = root;
    for word in words {
        cmd = visible(cmd).find(|s| s.get_name() == word)?;
    }
    Some(cmd)
}

/// The text `playbook commands [GROUP]` prints, or an error line.
pub fn render(root: &Command, group: &[String]) -> Result<String, String> {
    let mut out = String::new();
    if group.is_empty() {
        out.push_str(LEGEND);
        out.push('\n');
        let mut tops: Vec<&Command> = visible(root).collect();
        tops.sort_by_key(|c| c.get_name());
        for cmd in tops {
            out.push_str(&overview_line(cmd));
            out.push('\n');
        }
        return Ok(out);
    }
    let path = group.join(" ");
    let cmd = find(root, group).ok_or_else(|| unknown(&path))?;
    out.push_str(&detail_line(&path, cmd));
    out.push('\n');
    let mut below = Vec::new();
    walk(cmd, &path, &mut below);
    for e in below {
        out.push_str(&detail_line(&e.path, e.cmd));
        out.push('\n');
    }
    Ok(out)
}

fn unknown(path: &str) -> String {
    format!("playbook commands: no command `{path}`. Run `playbook commands` for the list.")
}

/// The intent ids of the tool map whose command is `path`.
fn intents(root: &Command, path: &str) -> Vec<&'static str> {
    crate::tools_map::TOOLS
        .iter()
        .filter(|t| {
            let words: Vec<String> = t.sample.split_whitespace().map(String::from).collect();
            deepest(root, &words) == path
        })
        .map(|t| t.intent)
        .collect()
}

/// The longest command path that `words` starts with.
fn deepest(root: &Command, words: &[String]) -> String {
    let mut cmd = root;
    let mut path: Vec<&str> = Vec::new();
    for word in words {
        match visible(cmd).find(|s| s.get_name() == word) {
            Some(sub) => {
                cmd = sub;
                path.push(word);
            }
            None => break,
        }
    }
    path.join(" ")
}

/// `playbook commands --json [GROUP]`: one object per command, the stable
/// form for tools.
pub fn render_json(root: &Command, group: &[String]) -> Result<String, String> {
    let mut entries = Vec::new();
    let base = if group.is_empty() {
        root
    } else {
        let path = group.join(" ");
        let cmd = find(root, group).ok_or_else(|| unknown(&path))?;
        entries.push(Entry { path, cmd });
        cmd
    };
    let prefix = group.join(" ");
    walk(base, &prefix, &mut entries);
    // Sorted by path, so a parent comes before its children and reordering
    // the enums does not change the output.
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let rows: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "command": e.path,
                "purpose": purpose(&e.path, e.cmd),
                "access": access(&e.path).as_str(),
                "args": args(e.cmd),
                "subcommands": visible(e.cmd).map(|s| s.get_name().to_string()).collect::<Vec<_>>(),
                "intents": intents(root, &e.path),
            })
        })
        .collect();
    serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn all() -> Vec<(String, Command)> {
        let root = tree();
        let mut out = Vec::new();
        walk(&root, "", &mut out);
        out.into_iter().map(|e| (e.path, e.cmd.clone())).collect()
    }

    fn words(text: &str) -> usize {
        text.split_whitespace().count()
    }

    fn overview() -> String {
        render(&tree(), &[]).unwrap()
    }

    fn line_of<'a>(text: &'a str, name: &str) -> &'a str {
        text.lines()
            .find(|l| {
                l.starts_with(&format!("{name} "))
                    || l.starts_with(&format!("{name}:"))
                    || l.starts_with(&format!("{name}!"))
            })
            .unwrap_or_else(|| panic!("no line for {name}:\n{text}"))
    }

    #[test]
    fn every_visible_command_has_an_entry() {
        let missing: Vec<String> = all()
            .into_iter()
            .filter(|(p, _)| meta(p).is_none())
            .map(|(p, _)| p)
            .collect();
        assert!(
            missing.is_empty(),
            "add an entry to META in crates/cli/src/command_index.rs for: {missing:?}"
        );
    }

    #[test]
    fn every_entry_names_a_visible_command_once() {
        let visible: BTreeSet<String> = all().into_iter().map(|(p, _)| p).collect();
        let mut seen = BTreeSet::new();
        for m in META {
            assert!(
                visible.contains(m.path),
                "META names `{}`, which is not a visible command",
                m.path
            );
            assert!(seen.insert(m.path), "META lists `{}` twice", m.path);
        }
    }

    #[test]
    fn meta_is_sorted_by_path() {
        let paths: Vec<&str> = META.iter().map(|m| m.path).collect();
        let mut sorted = paths.clone();
        sorted.sort_unstable();
        assert_eq!(paths, sorted, "keep META sorted by path");
    }

    /// Top-level purposes take 6 to 12 words and subcommand ones 3 to 14, so
    /// each line stays short. A longer doc comment needs a purpose in META.
    #[test]
    fn every_purpose_fits_one_short_line() {
        let mut bad = Vec::new();
        for (path, cmd) in all() {
            let text = purpose(&path, &cmd);
            let n = words(&text);
            let range = if path.contains(' ') { 3..=14 } else { 6..=12 };
            if !range.contains(&n) {
                bad.push(format!("`{path}`: {n} words, want {range:?}: {text}"));
            }
        }
        assert!(
            bad.is_empty(),
            "set a shorter purpose in META for:\n{}",
            bad.join("\n")
        );
    }

    #[test]
    fn hidden_and_deprecated_commands_are_never_listed() {
        let root = tree();
        let text = overview();
        let json = render_json(&root, &[]).unwrap();
        for name in ["init", "hook", "json"] {
            assert!(root.find_subcommand(name).is_some_and(Command::is_hide_set));
            assert!(
                !text.lines().any(|l| l.starts_with(&format!("{name}:"))),
                "{name} is listed:\n{text}"
            );
            assert!(
                !json.contains(&format!("\"command\": \"{name}\"")),
                "{name} in json"
            );
        }
        assert!(
            !json.contains("\"cc housekeep\""),
            "a hidden subcommand is listed"
        );
        fn deprecated_are_hidden(cmd: &Command) {
            for sub in cmd.get_subcommands() {
                let about = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
                if about.starts_with("Deprecated") {
                    assert!(
                        sub.is_hide_set(),
                        "{} is deprecated but visible",
                        sub.get_name()
                    );
                }
                deprecated_are_hidden(sub);
            }
        }
        deprecated_are_hidden(&root);
    }

    #[test]
    fn the_overview_has_one_line_per_top_level_command() {
        let root = tree();
        let text = overview();
        assert!(text.starts_with(LEGEND));
        assert_eq!(text.lines().count(), visible(&root).count() + 1);
    }

    #[test]
    fn commands_that_touch_the_users_machine_are_user_only() {
        for path in [
            "setup",
            "uninstall",
            "update",
            "trust",
            "migrate",
            "config import",
            "ccd",
            "deps ensure",
            "effort apply",
            "mode auto",
        ] {
            assert_eq!(access(path), Access::UserOnly, "{path}");
        }
        let text = overview();
        assert!(line_of(&text, "setup").ends_with("(user-only)"), "{text}");
        let names = |name: &str| -> Vec<String> {
            let head = line_of(&text, name).split(':').next().unwrap_or_default();
            head.split([' ', '|']).map(String::from).collect()
        };
        assert!(names("config").contains(&"import!".to_string()), "{text}");
        assert!(names("config").contains(&"get".to_string()), "{text}");
        assert!(names("mode").contains(&"auto!".to_string()), "{text}");
    }

    #[test]
    fn a_line_is_read_only_only_when_every_name_on_it_is() {
        let text = overview();
        for name in ["sessions", "hooks", "route", "plans", "prose"] {
            assert!(
                line_of(&text, name).ends_with("(read-only)"),
                "{name}: {text}"
            );
        }
        for name in ["config", "handoff", "doctor", "pr"] {
            assert!(
                !line_of(&text, name).ends_with("(read-only)"),
                "{name}: {text}"
            );
        }
        assert!(line_of(&text, "cc").starts_with("cc! "), "{text}");
    }

    #[test]
    fn a_group_lists_each_subcommand_with_its_flags() {
        let root = tree();
        let text = render(&root, &["disk".to_string()]).unwrap();
        assert!(text.starts_with("disk: "), "{text}");
        assert!(text.contains("\ndisk status"), "{text}");
        assert!(text.contains("(user-only)"), "{text}");
        let nested = render(&root, &["worktree".into(), "review".into()]).unwrap();
        assert!(nested.contains("\nworktree review setup <"), "{nested}");
        assert!(render(&root, &["nope".to_string()]).is_err());
    }

    #[test]
    fn json_carries_access_args_and_intents() {
        let root = tree();
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(&render_json(&root, &[]).unwrap()).unwrap();
        let row = |c: &str| rows.iter().find(|r| r["command"] == c).unwrap().clone();
        assert_eq!(row("setup")["access"], "user-only");
        assert_eq!(row("sessions")["access"], "read-only");
        assert_eq!(row("pr reply")["intents"][0], "pr-reply");
        assert!(row("path")["intents"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("plans-dir")));
        assert!(row("route")["args"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("[--json]")));
        let group: Vec<serde_json::Value> =
            serde_json::from_str(&render_json(&root, &["pr".to_string()]).unwrap()).unwrap();
        assert_eq!(group[0]["command"], "pr");
        assert!(group
            .iter()
            .all(|r| r["command"].as_str().unwrap().starts_with("pr")));
    }

    #[test]
    fn every_tool_map_intent_lands_on_a_command() {
        let root = tree();
        for tool in crate::tools_map::TOOLS {
            let words: Vec<String> = tool.sample.split_whitespace().map(String::from).collect();
            assert!(
                !deepest(&root, &words).is_empty(),
                "{} lands on no command",
                tool.intent
            );
        }
    }
}
