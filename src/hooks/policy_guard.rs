// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Denies three things the system prompt used to ask for in prose:
//!
//! - skipping git hooks or signing (`--no-verify`, `-n` on commit,
//!   `--no-gpg-sign`, `-c commit.gpgsign=false`),
//! - a hand-run `gh pr create`, which skips the pre-flight checks the
//!   `/playbook:create-pull-request` skill runs (that skill calls
//!   `playbook pr create`, which this hook never sees),
//! - writing memory outside playbook's store: Claude Code's own auto memory
//!   (`~/.claude/projects/*/memory/`) or the retired `~/.claude/memory/`.
//!
//! It also shows the memory fact format (`MEMORY_SCHEMA`) when a fact file is
//! written to playbook's store, so the system prompt does not carry it.
//!
//! One hook on two matchers: Bash looks at the command, Edit and Write look at
//! the target path. It fails safe: a payload it cannot read, a command the
//! reader cannot split, or anything unexpected allows the call, and it never
//! spawns a process. `POLICY_GUARD=0` turns it off. It only denies, it never
//! writes anything.
//!
//! A guardrail against an agent drifting, not a security boundary: it does not
//! expand variables, aliases or scripts, so an obfuscated spelling passes.

use crate::common::payload::Payload;
use crate::common::shell::{program_index, program_name, simple_commands};
use crate::common::{emit_pre_context, emit_pre_deny};
use std::path::{Component, Path, PathBuf};

/// Subcommands that run git hooks, so `--no-verify` matters.
const HOOKED_SUBCOMMANDS: [&str; 6] = ["commit", "push", "merge", "rebase", "cherry-pick", "am"];

/// Subcommands that sign, so `--no-gpg-sign` matters.
const SIGNING_SUBCOMMANDS: [&str; 7] = [
    "commit",
    "tag",
    "merge",
    "rebase",
    "cherry-pick",
    "revert",
    "am",
];

/// `git` options that take a separate value, skipped while finding the
/// subcommand.
const GIT_VALUE_OPTIONS: [&str; 6] = [
    "-C",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--exec-path",
    "--super-prefix",
];

/// `gh` options that take a separate value, skipped while finding `pr create`.
const GH_VALUE_OPTIONS: [&str; 2] = ["-R", "--repo"];

/// `git commit` short options that take the rest of the cluster or the next
/// word as a value, so a later `n` in the same cluster is part of that value.
const COMMIT_VALUE_SHORTS: &str = "mFCcts";

/// Programs that write to the path they are given.
const WRITER_PROGRAMS: [&str; 10] = [
    "tee", "cp", "mv", "rm", "touch", "mkdir", "ln", "install", "rsync", "dd",
];

/// The shortest prefix of a long option git still accepts, which the guard
/// treats as the option itself.
const MIN_ABBREVIATION: usize = 8;

const NO_VERIFY_REASON: &str = "Skipping git hooks (--no-verify, commit -n) is not allowed. Fix what the hook reports instead. If a hook is wrong, say so and ask the user.";

const NO_SIGN_REASON: &str = "Turning off commit or tag signing (--no-gpg-sign, commit.gpgsign=false) is not allowed. Sign every commit and tag. If signing fails, report the error to the user.";

const PR_CREATE_REASON: &str = "Do not hand-run `gh pr create`. Use the /playbook:create-pull-request skill: it runs the pre-flight checks, writes a conventional-commit title and the team PR template, then calls `playbook pr create`.";

/// Shown when a fact file is written in playbook's store: the format the
/// system prompt no longer carries.
pub const MEMORY_SCHEMA: &str = "Memory fact format. One fact per file, kebab-case name, `.md`. \
Frontmatter: `name`, `description` (start with \"Use when ...\"), `type` (user, feedback, project or \
reference), optional `links:` and optional `anchors:`. Body for feedback and project facts: the rule, \
then **Why:** and **How to apply:**. Edges in `links:` use bare basenames with no path or extension: \
`supersedes` (this replaces an older fact; act on the newest in a chain, treat the rest as history), \
`depends_on` (read the prerequisite first), `relates_to` (symmetric, pull the neighbour), \
`contradicts` (symmetric; if both are live, surface the conflict and do not pick silently). Store \
each edge once, on the fact that authors it. A basename that resolves in neither the fact's scope nor \
the global scope is dangling: surface it, do not fail. `anchors:` lists the repo-relative code a fact \
describes: a directory (`src/auth/`), a file (`src/auth/login.py`) or a symbol \
(`src/auth/login.py#authenticate`). Scope comes from the folder: `<owner>/<repo>/` for one repo, \
`<owner>/` for one owner's repos, the root for everything. Inside a scoped folder do not repeat the \
owner or repo in the text.";

const MEMORY_REASON: &str = "Claude Code's own memory is off limits here. Save the fact in playbook's store instead: ~/.config/playbook/memory/ for a global fact, <owner>/<repo>/ under it for a project fact.";

pub fn run(payload: &Payload) {
    if std::env::var("POLICY_GUARD").as_deref() == Ok("0") {
        return;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let command = payload.field(".tool_input.command");
    let path = payload.field(".tool_input.file_path");
    let reason = if !command.is_empty() {
        check_command(&command, &home)
    } else {
        check_path(&path, &home)
    };
    if let Some(reason) = reason {
        emit_pre_deny(reason);
    } else if payload.field(".tool_name") == "Write"
        && is_fact_file(&path, &crate::common::paths::memory_dir())
    {
        emit_pre_context("PreToolUse", MEMORY_SCHEMA);
    }
}

/// Whether `path` is a fact file (`.md`) inside playbook's memory store.
fn is_fact_file(path: &str, memory_dir: &Path) -> bool {
    let path = Path::new(path);
    path.is_absolute()
        && path.extension().is_some_and(|ext| ext == "md")
        && path.starts_with(memory_dir)
        && !path.components().any(|c| c == Component::ParentDir)
}

/// The reason to deny `command`, or `None` to allow it.
fn check_command(command: &str, home: &str) -> Option<&'static str> {
    if !mentions_a_guarded_program(command) {
        return None;
    }
    for words in simple_commands(command) {
        let Some(at) = program_index(&words) else {
            continue;
        };
        let args = &words[at + 1..];
        let found = match program_name(&words[at]).as_str() {
            "git" => check_git(args),
            "gh" => check_gh(args),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
        if writes_claude_memory(&words, at, home) {
            return Some(MEMORY_REASON);
        }
    }
    None
}

/// A cheap check that rules out nearly every Bash call before any parsing.
fn mentions_a_guarded_program(command: &str) -> bool {
    let bare: String = command
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\'))
        .collect::<String>()
        .to_lowercase();
    bare.contains("git") || bare.contains("gh") || bare.contains(".claude") || bare.contains('~')
}

fn check_git(args: &[String]) -> Option<&'static str> {
    let mut i = 0;
    while let Some(word) = args.get(i) {
        if word == "-c" {
            let value = args.get(i + 1)?.to_lowercase();
            if value == "commit.gpgsign=false" || value == "tag.gpgsign=false" {
                return Some(NO_SIGN_REASON);
            }
            i += 2;
        } else if GIT_VALUE_OPTIONS.contains(&word.as_str()) {
            i += 2;
        } else if word.starts_with('-') {
            i += 1;
        } else {
            break;
        }
    }
    let subcommand = args.get(i)?.as_str();
    let rest: Vec<&str> = args[i + 1..]
        .iter()
        .map(String::as_str)
        .take_while(|w| *w != "--")
        .collect();

    if HOOKED_SUBCOMMANDS.contains(&subcommand)
        && rest.iter().any(|w| is_long_option(w, "--no-verify"))
    {
        return Some(NO_VERIFY_REASON);
    }
    if subcommand == "commit" && rest.iter().any(|w| is_commit_short_n(w)) {
        return Some(NO_VERIFY_REASON);
    }
    if SIGNING_SUBCOMMANDS.contains(&subcommand)
        && rest.iter().any(|w| is_long_option(w, "--no-gpg-sign"))
    {
        return Some(NO_SIGN_REASON);
    }
    None
}

/// Whether `word` is `name` or an abbreviation git accepts for it.
fn is_long_option(word: &str, name: &str) -> bool {
    word == name
        || (word.len() >= MIN_ABBREVIATION && word.starts_with("--") && name.starts_with(word))
}

/// A short cluster for `git commit` that holds `n` (`--no-verify`) before any
/// letter that takes a value, such as `-n` or `-anm`.
fn is_commit_short_n(word: &str) -> bool {
    let Some(cluster) = word.strip_prefix('-') else {
        return false;
    };
    if cluster.is_empty() || !cluster.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    for letter in cluster.chars() {
        if COMMIT_VALUE_SHORTS.contains(letter) {
            return false;
        }
        if letter == 'n' {
            return true;
        }
    }
    false
}

fn check_gh(args: &[String]) -> Option<&'static str> {
    let mut positional = Vec::new();
    let mut i = 0;
    while let Some(word) = args.get(i) {
        if GH_VALUE_OPTIONS.contains(&word.as_str()) {
            i += 2;
        } else if word.starts_with('-') {
            i += 1;
        } else {
            positional.push(word.as_str());
            i += 1;
            if positional.len() == 2 {
                break;
            }
        }
    }
    (positional == ["pr", "create"]).then_some(PR_CREATE_REASON)
}

/// Whether this simple command writes to a Claude Code memory path: it names
/// one and either redirects output or runs a program that writes.
fn writes_claude_memory(words: &[String], program_at: usize, home: &str) -> bool {
    if !words.iter().any(|w| is_claude_memory_path(w, home)) {
        return false;
    }
    let program = program_name(&words[program_at]);
    let in_place = matches!(program.as_str(), "sed" | "perl")
        && words[program_at + 1..]
            .iter()
            .any(|w| w.starts_with("-i") || w == "--in-place");
    WRITER_PROGRAMS.contains(&program.as_str()) || in_place || words.iter().any(|w| redirects(w))
}

/// A redirection that writes: `>`, `>>`, `2>`, `>file`, but not `<` or `>&`.
fn redirects(word: &str) -> bool {
    let operator = word.trim_start_matches(|c: char| c.is_ascii_digit());
    operator.starts_with('>') && !operator.starts_with(">&")
}

/// The reason to deny a write to `path`, or `None` to allow it.
fn check_path(path: &str, home: &str) -> Option<&'static str> {
    is_claude_memory_path(path, home).then_some(MEMORY_REASON)
}

/// Whether `path` is inside Claude Code's auto memory
/// (`~/.claude/projects/<project>/memory`) or the retired `~/.claude/memory`.
/// `~`, `$HOME` and `${HOME}` count as `home`, and `.` and `..` segments are
/// resolved lexically so `x/../memory` cannot hide it.
fn is_claude_memory_path(path: &str, home: &str) -> bool {
    if home.is_empty() {
        return false;
    }
    let expanded = expand_home(path, home);
    if !expanded.starts_with('/') {
        return false;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in expanded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let home_parts: Vec<&str> = home.split('/').filter(|p| !p.is_empty()).collect();
    let Some(rest) = parts.strip_prefix(home_parts.as_slice()) else {
        return false;
    };
    matches!(
        rest,
        [".claude", "memory", ..] | [".claude", "projects", _, "memory", ..]
    )
}

fn expand_home(path: &str, home: &str) -> String {
    for prefix in ["~", "$HOME", "${HOME}"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            if rest.is_empty() || rest.starts_with('/') {
                return format!("{}{rest}", PathBuf::from(home).display());
            }
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/me";

    fn deny(command: &str) -> Option<&'static str> {
        check_command(command, HOME)
    }

    #[test]
    fn no_verify_is_denied_on_every_hooked_subcommand() {
        // Arrange
        let commands = [
            "git commit --no-verify -m x",
            "git commit -m x --no-verify",
            "git push --no-verify origin main",
            "git -C /repo commit --no-verify",
            "git commit --no-ver -m x",
            "cd x && git rebase --no-verify main",
            "FOO=1 git commit --no-verify",
        ];

        // Act / Assert
        for command in commands {
            assert_eq!(deny(command), Some(NO_VERIFY_REASON), "{command}");
        }
    }

    #[test]
    fn commit_short_n_is_denied_but_a_message_value_is_not() {
        // Arrange / Act / Assert
        assert_eq!(deny("git commit -n -m x"), Some(NO_VERIFY_REASON));
        assert_eq!(deny("git commit -anm x"), Some(NO_VERIFY_REASON));
        assert_eq!(deny("git commit -m n"), None);
        assert_eq!(deny("git commit -mn"), None);
        assert_eq!(deny("git commit -m \"-n fix\""), None);
        assert_eq!(
            deny("git push -n origin main"),
            None,
            "-n is --dry-run on push"
        );
    }

    #[test]
    fn signing_off_is_denied() {
        // Arrange / Act / Assert
        assert_eq!(deny("git commit --no-gpg-sign -m x"), Some(NO_SIGN_REASON));
        assert_eq!(deny("git tag --no-gpg-sign v1"), Some(NO_SIGN_REASON));
        assert_eq!(
            deny("git -c commit.gpgsign=false commit -m x"),
            Some(NO_SIGN_REASON)
        );
        assert_eq!(deny("git commit -S -s -m x"), None);
        assert_eq!(deny("git commit --no-signoff -m x"), None);
    }

    #[test]
    fn hand_run_gh_pr_create_is_denied() {
        // Arrange / Act / Assert
        assert_eq!(
            deny("gh pr create --title t --body b"),
            Some(PR_CREATE_REASON)
        );
        assert_eq!(deny("gh -R a/b pr create"), Some(PR_CREATE_REASON));
        assert_eq!(deny("gh pr create"), Some(PR_CREATE_REASON));
        assert_eq!(deny("gh pr view 12"), None);
        assert_eq!(deny("gh pr merge 12 --auto"), None);
        assert_eq!(deny("gh issue create"), None);
        assert_eq!(
            deny("playbook pr create --title t --body-file f"),
            None,
            "the skill's own command stays allowed"
        );
    }

    #[test]
    fn an_ordinary_command_passes() {
        // Arrange
        let commands = [
            "ls -la",
            "git status",
            "git commit -S -s -m 'fix: x'",
            "git log --oneline",
            "echo \"git commit --no-verify\"",
            "cat ~/.claude/projects/x/memory/a.md",
            "grep -r foo ~/.claude/memory",
        ];

        // Act / Assert
        for command in commands {
            assert_eq!(deny(command), None, "{command}");
        }
    }

    #[test]
    fn a_shell_write_to_claude_memory_is_denied_and_a_read_is_not() {
        // Arrange / Act / Assert
        assert_eq!(
            deny("echo x > ~/.claude/projects/-Users-me-repo/memory/a.md"),
            Some(MEMORY_REASON)
        );
        assert_eq!(
            deny("cat note.md >> $HOME/.claude/memory/a.md"),
            Some(MEMORY_REASON)
        );
        assert_eq!(
            deny("cp a.md /Users/me/.claude/projects/p/memory/a.md"),
            Some(MEMORY_REASON)
        );
        assert_eq!(
            deny("sed -i s/a/b/ ~/.claude/projects/p/memory/a.md"),
            Some(MEMORY_REASON)
        );
        assert_eq!(deny("ls ~/.claude/projects/p/memory"), None);
        assert_eq!(deny("cp a.md ~/.config/playbook/memory/a.md"), None);
    }

    #[test]
    fn file_tools_are_denied_inside_claude_memory_only() {
        // Arrange / Act / Assert
        let inside = [
            "/Users/me/.claude/projects/-Users-me-repo/memory/MEMORY.md",
            "/Users/me/.claude/projects/p/memory/sub/a.md",
            "/Users/me/.claude/memory/a.md",
            "~/.claude/projects/p/memory/a.md",
            "/Users/me/.claude/projects/p/x/../memory/a.md",
        ];
        for path in inside {
            assert_eq!(check_path(path, HOME), Some(MEMORY_REASON), "{path}");
        }
        let outside = [
            "/Users/me/.config/playbook/memory/a.md",
            "/Users/me/.claude/projects/p/session.jsonl",
            "/Users/me/.claude/settings.json",
            "/Users/me/.claude/CLAUDE.md",
            "/Users/me/repo/memory/a.md",
            "/Users/other/.claude/memory/a.md",
            "relative/memory/a.md",
            "",
        ];
        for path in outside {
            assert_eq!(check_path(path, HOME), None, "{path}");
        }
    }

    #[test]
    fn only_md_files_inside_the_store_are_fact_files() {
        // Arrange
        let store = Path::new("/Users/me/.config/playbook/memory");

        // Act / Assert
        assert!(is_fact_file(
            "/Users/me/.config/playbook/memory/a.md",
            store
        ));
        assert!(is_fact_file(
            "/Users/me/.config/playbook/memory/o/r/a.md",
            store
        ));
        assert!(!is_fact_file(
            "/Users/me/.config/playbook/memory/memory.graph.json",
            store
        ));
        assert!(!is_fact_file(
            "/Users/me/.config/playbook/memory/../x/a.md",
            store
        ));
        assert!(!is_fact_file("/Users/me/repo/a.md", store));
        assert!(!is_fact_file("memory/a.md", store));
    }

    #[test]
    fn the_schema_names_every_edge_type_and_the_anchor_forms() {
        // Arrange / Act / Assert
        for needle in [
            "supersedes",
            "depends_on",
            "relates_to",
            "contradicts",
            "anchors:",
            "#authenticate",
            "Why:",
        ] {
            assert!(MEMORY_SCHEMA.contains(needle), "{needle}");
        }
        assert!(MEMORY_SCHEMA.chars().count() < 1700);
    }

    #[test]
    fn an_empty_home_never_denies_a_path() {
        // Arrange / Act / Assert
        assert_eq!(check_path("/.claude/memory/a.md", ""), None);
    }

    #[test]
    fn garbage_input_is_allowed() {
        // Arrange / Act / Assert
        for command in [
            "",
            "git",
            "git -c",
            "gh",
            "'unterminated git commit --no-ver",
            ")(",
        ] {
            let _ = deny(command);
        }
        assert_eq!(deny("git -c"), None);
        assert_eq!(deny("gh"), None);
    }
}
