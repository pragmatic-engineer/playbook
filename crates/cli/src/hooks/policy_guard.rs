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
//! - changing the `rm` guard's own allowance: `playbook config set` on a
//!   `hooks.rmGuard.*` key, `config import`, `config unset` or `config edit`,
//!   anything naming the config database, a write to a config file, or an
//!   inline `WORKTREE_BASE_DIR=` or `PLAYBOOK_SAFE_ROOTS=` assignment. A model
//!   that could widen the roots could delete anything it likes, so this holds
//!   in `ask` mode too, unlike the `auto_cost` guard, which covers `mode`,
//!   `auto.*` and `fix.*` in `auto` mode only. The inline assignments cannot
//!   reach the hook process, so denying them is defense in depth.
//!
//! - changing the hook entries `playbook init` writes into Claude Code's
//!   `settings.json`: an Edit or Write that adds, changes or removes a
//!   `playbook hook` command or turns hooks off, a Bash write to a settings
//!   file that mentions `hooks`, and `playbook uninstall`. An agent that could
//!   drop the guard entries could run without them, so these are the user's to
//!   change, in `ask` mode too.
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

const NO_VERIFY_REASON: &str = "Skipping git hooks (--no-verify, commit -n) is not allowed. Fix what the hook reports instead. If a hook is wrong, say so and ask the user. In an unattended run, report the hook's error text and stop.";

const NO_SIGN_REASON: &str = "Turning off commit or tag signing (--no-gpg-sign, commit.gpgsign=false) is not allowed. Sign every commit and tag. If signing fails, report the error text and stop.";

const PR_CREATE_REASON: &str = "Do not hand-run `gh pr create`. Use `playbook deliver` (it commits, pushes and opens or updates the PR from any worktree, `--dir <worktree>`), or the /playbook:create-pull-request skill, which writes the title and body and opens the PR. If the command itself fails, run `playbook pr create --help` or `playbook deliver --help`, report the error text to the user, and stop; do not fall back to `gh pr create`.";

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

const RM_GUARD_CONFIG_REASON: &str = "The rm guard's allowed roots are the user's to change, not the model's. Ask the user to run the command themselves in the prompt with `!`, for example `! playbook config set --global hooks.rmGuard.allowedRoots '[\"~/scratch/builds\"]'`. Until then, clean up inside the repo, or leave the path and say so.";

const HOOK_ENTRIES_REASON: &str = "The hook entries in settings.json are the user's to change, not the model's. They are the guards that keep this session safe. Ask the user to run `playbook init` (to add them) or `playbook uninstall` (to remove them) themselves in the prompt with `!`, or to edit settings.json by hand.";

/// What marks an entry `playbook init` owns.
const HOOK_MARK: &str = "playbook hook ";

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
    } else if let Some(reason) = check_hook_edit(
        &path,
        &payload.field(".tool_input.old_string"),
        &format!(
            "{}{}",
            payload.field(".tool_input.content"),
            payload.field(".tool_input.new_string")
        ),
        payload.field(".tool_name") == "Write",
    ) {
        Some(reason)
    } else {
        let content = format!(
            "{}{}",
            payload.field(".tool_input.content"),
            payload.field(".tool_input.new_string")
        );
        check_path(&path, &home, &content)
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

/// Programs that run another command given as plain words, so `playbook` can
/// sit after them.
const RUNNER_PROGRAMS: [&str; 9] = [
    "nohup",
    "xargs",
    "setsid",
    "stdbuf",
    "watch",
    "parallel",
    "ionice",
    "caffeinate",
    "unbuffer",
];

/// Shells that run the string after `-c`.
const SHELL_PROGRAMS: [&str; 4] = ["sh", "bash", "zsh", "dash"];

/// Programs that can write a file or database they are handed, on top of
/// `WRITER_PROGRAMS`.
const DB_WRITER_PROGRAMS: [&str; 8] = [
    "sqlite3", "python", "python3", "node", "ruby", "perl", "sed", "jq",
];

/// Whether this simple command can widen the `rm` guard's allowance.
fn changes_rm_guard_config(words: &[String], program_at: usize, home: &str) -> bool {
    changes_rm_guard_config_at(words, program_at, home, 0)
}

fn changes_rm_guard_config_at(
    words: &[String],
    program_at: usize,
    home: &str,
    depth: usize,
) -> bool {
    let assigns = words[..program_at].iter().any(|w| {
        w.split_once('=')
            .is_some_and(|(name, _)| matches!(name, "WORKTREE_BASE_DIR" | "PLAYBOOK_SAFE_ROOTS"))
    });
    if assigns {
        return true;
    }
    let program = program_name(&words[program_at]);
    let rest = &words[program_at + 1..];
    // A config file or the database named as the target of something that can
    // write. Read-only programs, `git commit -m "mention playbook.db"` and
    // searches are left alone.
    let writes = WRITER_PROGRAMS.contains(&program.as_str())
        || DB_WRITER_PROGRAMS.contains(&program.as_str())
        || SHELL_PROGRAMS.contains(&program.as_str())
        || words.iter().any(|w| redirects(w));
    if writes && words.iter().any(|w| names_config_store(w, home)) {
        return true;
    }
    if depth < 3 {
        if SHELL_PROGRAMS.contains(&program.as_str()) {
            if let Some(at) = rest
                .iter()
                .position(|w| w == "-c" || w.ends_with("c") && w.starts_with('-'))
            {
                if let Some(script) = rest.get(at + 1) {
                    return simple_commands(script).iter().any(|inner| {
                        program_index(inner)
                            .is_some_and(|i| changes_rm_guard_config_at(inner, i, home, depth + 1))
                    });
                }
            }
        }
        if RUNNER_PROGRAMS.contains(&program.as_str()) {
            return rest
                .iter()
                .position(|w| program_name(w) == "playbook")
                .is_some_and(|i| {
                    let inner = &rest[i..];
                    changes_rm_guard_config_at(inner, 0, home, depth + 1)
                });
        }
    }
    let mut args = rest;
    match program.as_str() {
        "playbook" => {}
        "cargo" => match super::auto_cost::cargo_run_args(args) {
            Some(after) => args = after,
            None => return false,
        },
        _ => return false,
    }
    if args
        .iter()
        .find(|w| !w.starts_with('-'))
        .map(String::as_str)
        != Some("config")
    {
        return false;
    }
    let at = args.iter().position(|w| w == "config").unwrap_or(0);
    match args[at + 1..].first().map(String::as_str) {
        // `unset` and `edit` do not exist yet. They are denied ahead of time so
        // adding them later cannot open a gap.
        Some("import" | "unset" | "edit") => true,
        // Any positional can be the key (`--org acme` takes a value in some
        // spellings), so all of them are checked.
        Some("set") => args[at + 2..]
            .iter()
            .filter(|w| !w.starts_with('-'))
            .any(|w| w.to_lowercase().starts_with("hooks.rmguard")),
        _ => false,
    }
}

/// Whether `word` names one of playbook's databases: the settings store (now
/// `config.db`, earlier `playbook.db`), the memory index or the data folder.
pub(crate) fn names_database(word: &str) -> bool {
    let last = word.rsplit('/').next().unwrap_or(word);
    let db_file =
        |name: &str| last == name || last == format!("{name}-wal") || last == format!("{name}-shm");
    word.contains("playbook.db")
        || db_file("config.db")
        || db_file("memory.db")
        || word
            .split('/')
            .collect::<Vec<_>>()
            .windows(2)
            .any(|w| w == ["playbook", "data"])
}

/// Whether `word` names playbook's config database or one of its config
/// files, or a glob inside the playbook folder.
fn names_config_store(word: &str, home: &str) -> bool {
    if names_database(&word.to_lowercase()) {
        return true;
    }
    if home.is_empty() {
        return false;
    }
    let root = crate::common::paths::playbook_root_from(Path::new(home));
    let expanded = expand_home(word, home);
    super::auto_cost::is_config_file(Path::new(&expanded), &root)
        || (expanded.contains(['*', '?'])
            && Path::new(&expanded).starts_with(&root)
            && matches!(
                Path::new(&expanded).components().count() - root.components().count(),
                1 | 2
            ))
}

/// Whether `path` is a Claude Code settings file: `settings*.json` directly
/// under a `.claude` directory.
fn is_claude_settings(path: &str) -> bool {
    let p = Path::new(path);
    let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase());
    p.parent()
        .and_then(Path::file_name)
        .is_some_and(|d| d == ".claude")
        && name.is_some_and(|n| n.starts_with("settings") && n.ends_with(".json"))
}

/// Whether text touches the hook entries or switches hooks off.
fn mentions_hook_entries(text: &str) -> bool {
    text.contains(HOOK_MARK) || text.contains("disableAllHooks")
}

/// An Edit or Write on a settings file that adds, changes or removes a
/// `playbook hook` entry. A Write replaces the whole file, so it is also
/// denied when the file on disk carries entries the new text may drop.
fn check_hook_edit(path: &str, old: &str, new: &str, whole_file: bool) -> Option<&'static str> {
    if !is_claude_settings(path) {
        return None;
    }
    let on_disk =
        whole_file && std::fs::read_to_string(path).is_ok_and(|text| mentions_hook_entries(&text));
    (mentions_hook_entries(old) || mentions_hook_entries(new) || on_disk)
        .then_some(HOOK_ENTRIES_REASON)
}

/// A Bash command that rewrites the hook entries: `playbook uninstall`, or a
/// write program naming a settings file in a command that mentions hooks.
fn edits_hook_entries(words: &[String], program_at: usize, whole: &str) -> bool {
    let program = program_name(&words[program_at]);
    if program == "playbook" {
        return words[program_at + 1..]
            .iter()
            .find(|w| !w.starts_with('-'))
            .is_some_and(|w| w == "uninstall")
            && !words.iter().any(|w| w == "--dry-run");
    }
    // `jq` and `sed` only write through a redirect or `sed -i`.
    let in_place_sed = program == "sed" && words.iter().any(|w| w.starts_with("-i"));
    let writes = WRITER_PROGRAMS.contains(&program.as_str())
        || (DB_WRITER_PROGRAMS.contains(&program.as_str())
            && !matches!(program.as_str(), "jq" | "sed"))
        || in_place_sed
        || SHELL_PROGRAMS.contains(&program.as_str())
        || words.iter().any(|w| redirects(w));
    writes
        && whole.to_lowercase().contains("hook")
        && words.iter().any(|w| {
            let w = w.trim_matches(|c| c == '"' || c == '\'');
            let name = w.rsplit('/').next().unwrap_or(w);
            w.contains(".claude/") && name.starts_with("settings") && name.ends_with(".json")
        })
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
        if changes_rm_guard_config(&words, at, home) {
            return Some(RM_GUARD_CONFIG_REASON);
        }
        if edits_hook_entries(&words, at, command) {
            return Some(HOOK_ENTRIES_REASON);
        }
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
    [
        "git",
        "gh",
        ".claude",
        "~",
        "playbook",
        "cargo",
        "worktree_base_dir",
        "safe_roots",
    ]
    .iter()
    .any(|needle| bare.contains(needle))
}

fn check_git(args: &[String]) -> Option<&'static str> {
    let mut i = 0;
    while let Some(word) = args.get(i) {
        if word == "-c" {
            let value = args.get(i + 1)?.to_lowercase();
            if let Some((key, setting)) = value.split_once('=') {
                let off = matches!(setting.trim(), "false" | "no" | "off" | "0" | "");
                if off && (key == "commit.gpgsign" || key == "tag.gpgsign") {
                    return Some(NO_SIGN_REASON);
                }
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
fn check_path(path: &str, home: &str, content: &str) -> Option<&'static str> {
    if writes_guard_env_to_settings(path, content) {
        return Some(RM_GUARD_CONFIG_REASON);
    }
    if !home.is_empty()
        && super::auto_cost::is_config_file(
            Path::new(path),
            &crate::common::paths::playbook_root_from(Path::new(home)),
        )
    {
        return Some(RM_GUARD_CONFIG_REASON);
    }
    is_claude_memory_path(path, home).then_some(MEMORY_REASON)
}

/// A Claude Code settings file that is given an `env` entry able to move the
/// `rm` guard's roots or switch the guards off. Other edits to settings pass.
fn writes_guard_env_to_settings(path: &str, content: &str) -> bool {
    let p = Path::new(path);
    let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase());
    let in_claude_dir = p
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|d| d == ".claude");
    let is_settings =
        in_claude_dir && name.is_some_and(|n| n.starts_with("settings") && n.ends_with(".json"));
    is_settings
        && ["PLAYBOOK_SAFE_ROOTS", "WORKTREE_BASE_DIR", "POLICY_GUARD"]
            .iter()
            .any(|key| content.contains(key))
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

    #[test]
    fn changes_to_the_rm_guard_allowance_are_denied() {
        for command in [
            "playbook config set --global hooks.rmGuard.allowedRoots '[\"/opt/a/b\"]'",
            "playbook config set hooks.rmGuard.allowedRoots ~/x/y",
            "playbook config set --org hooks.rmguard.allowedroots x",
            "/usr/local/bin/playbook config import cfg.json",
            "playbook config unset hooks.rmGuard.allowedRoots",
            "playbook config edit",
            "cargo run -q -- config set hooks.rmGuard.allowedRoots /a/b",
            "cd x && playbook config set hooks.rmGuard.allowedRoots /a/b",
            "env playbook config set hooks.rmGuard.allowedRoots /a/b",
            "sqlite3 ~/.config/playbook/playbook.db 'update x'",
            "WORKTREE_BASE_DIR=/tmp/x rm -rf /tmp/x/y",
            "PLAYBOOK_SAFE_ROOTS=/ rm -rf /etc/x",
            "env WORKTREE_BASE_DIR=/Users/me playbook worktree sweep",
            "nohup playbook config set --global hooks.rmGuard.allowedRoots /a/b",
            "xargs playbook config set hooks.rmGuard.allowedRoots /a/b",
            "bash -c 'playbook config set hooks.rmGuard.allowedRoots /a/b'",
            "sh -c \"cd x && playbook config import f.json\"",
            "playbook config set --org acme hooks.rmGuard.allowedRoots /a/b",
            "printf x > ~/.config/playbook/config.json",
            "echo '{}' | tee $HOME/.config/playbook/orgs/acme/config.json",
            "python3 -c 'x' ~/.config/playbook/playbook.db",
            "cp evil.db ~/.config/playbook/playbook.db",
            "sed -i s/a/b/ ~/.config/playbook/*.db",
            "sqlite3 ~/.config/playbook/data/config.db 'update x'",
            "cp evil.db ~/.config/playbook/data/memory.db",
            "cd ~/.config/playbook/data && sqlite3 config.db 'update x'",
            "sed -i s/a/b/ ~/.config/playbook/d*/c*.db",
        ] {
            assert_eq!(deny(command), Some(RM_GUARD_CONFIG_REASON), "{command}");
        }
    }

    #[test]
    fn ordinary_config_use_and_prose_are_left_alone() {
        for command in [
            "playbook config get hooks.rmGuard.allowedRoots",
            "playbook config list",
            "playbook config export",
            "playbook config set --global commit.signOff true",
            "echo \"playbook config set hooks.rmGuard.allowedRoots x\"",
            "git commit -m 'document hooks.rmGuard.allowedRoots'",
            "cargo test --test hooks_guards",
            "cargo test -- config import",
            "git commit -m 'handle playbook.db lock'",
            "rg -n playbook.db src/",
            "cat ~/.config/playbook/config.json",
            "playbook config set --global commit.signOff true",
            "xargs echo playbook",
            "cd /work/.worktrees/playbook/data-config && cargo test",
            "git commit -m 'move the settings to config.db'",
            "cat src/config.db.ts",
            "cp a /work/.worktrees/playbook/data-config/b",
        ] {
            assert_eq!(deny(command), None, "{command}");
        }
    }

    #[test]
    fn settings_files_may_not_gain_a_guard_env_entry() {
        let settings = format!("{HOME}/.claude/settings.json");
        let local = "/work/repo/.claude/settings.local.json";
        for path in [settings.as_str(), local] {
            assert_eq!(
                check_path(path, HOME, r#"{"env":{"WORKTREE_BASE_DIR":"/"}}"#),
                Some(RM_GUARD_CONFIG_REASON)
            );
            assert_eq!(
                check_path(path, HOME, r#"{"env":{"POLICY_GUARD":"0"}}"#),
                Some(RM_GUARD_CONFIG_REASON)
            );
            assert_eq!(check_path(path, HOME, r#"{"model":"opus"}"#), None);
        }
        assert_eq!(
            check_path("/work/notes/settings.json", HOME, "WORKTREE_BASE_DIR"),
            None
        );
    }

    #[test]
    fn writes_to_the_config_files_are_denied() {
        let root = format!("{HOME}/.config/playbook");
        for path in [
            format!("{root}/playbook.db"),
            format!("{root}/config.json"),
            format!("{root}/orgs/acme/config.json"),
            format!("{root}/x/../playbook.db"),
        ] {
            assert_eq!(
                check_path(&path, HOME, "x"),
                Some(RM_GUARD_CONFIG_REASON),
                "{path}"
            );
        }
        assert_eq!(check_path(&format!("{root}/memory/a.md"), HOME, "x"), None);
    }

    #[test]
    fn edits_that_touch_the_hook_entries_in_settings_are_denied() {
        let settings = format!("{HOME}/.claude/settings.json");
        for (old, new) in [
            (
                "",
                r#"{"hooks":{"Stop":[{"hooks":[{"command":"playbook hook x"}]}]}}"#,
            ),
            ("\"command\": \"playbook hook policy-guard\"", ""),
            ("", r#"{"disableAllHooks":true}"#),
        ] {
            assert_eq!(
                check_hook_edit(&settings, old, new, false),
                Some(HOOK_ENTRIES_REASON),
                "{old} {new}"
            );
        }
        assert_eq!(
            check_hook_edit(&settings, "", r#"{"model":"opus"}"#, false),
            None
        );
        assert_eq!(
            check_hook_edit("/work/notes/settings.json", "", "playbook hook x", false),
            None
        );
    }

    #[test]
    fn a_whole_file_write_is_denied_when_the_file_on_disk_has_our_entries() {
        let dir = crate::common::test_support::scratch_dir("hook-entries").join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"hooks":{"Stop":[{"hooks":[{"command":"playbook hook a"}]}]}}"#,
        )
        .unwrap();
        let path = path.to_string_lossy().into_owned();

        assert_eq!(
            check_hook_edit(&path, "", r#"{"model":"opus"}"#, true),
            Some(HOOK_ENTRIES_REASON)
        );
        assert_eq!(
            check_hook_edit(&path, "", r#"{"model":"opus"}"#, false),
            None
        );
    }

    #[test]
    fn bash_cannot_uninstall_or_rewrite_the_hook_entries() {
        for command in [
            "playbook uninstall --yes",
            "cd x && playbook uninstall",
            "jq '.hooks = {}' ~/.claude/settings.json > /tmp/s && mv /tmp/s ~/.claude/settings.json",
            "sed -i '' '/playbook hook/d' ~/.claude/settings.json",
            "echo '{\"hooks\":{}}' > ~/.claude/settings.json",
        ] {
            assert_eq!(deny(command), Some(HOOK_ENTRIES_REASON), "{command}");
        }
        for command in [
            "playbook uninstall --dry-run",
            "cat ~/.claude/settings.json",
            "jq '.hooks' ~/.claude/settings.json",
            "echo '{\"model\":\"opus\"}' > ~/.claude/settings.json",
        ] {
            assert_eq!(deny(command), None, "{command}");
        }
    }

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
            assert_eq!(check_path(path, HOME, "x"), Some(MEMORY_REASON), "{path}");
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
            assert_eq!(check_path(path, HOME, "x"), None, "{path}");
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
        assert_eq!(check_path("/.claude/memory/a.md", "", "x"), None);
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
