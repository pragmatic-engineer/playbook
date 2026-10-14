// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Denies these things the system prompt used to ask for in prose:
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
//!   `hooks.rmGuard.*`, `hooks.proseLint.*`, `pr.rules.*`, `pr.bodyMaxChars` or
//!   `pr.ticketPatterns` key, `config import`, `config unset` or `config edit`,
//!   anything naming the config database, a write to a config file, or an
//!   inline `WORKTREE_BASE_DIR=` or `PLAYBOOK_SAFE_ROOTS=` assignment. A model
//!   that could widen the roots could delete anything it likes, so this holds
//!   in `ask` mode too, unlike the `auto_cost` guard, which covers `mode`,
//!   `auto.*` and `fix.*` in `auto` mode only. The inline assignments cannot
//!   reach the hook process, so denying them is defense in depth.
//!
//! - dropping the hook entries `playbook init` writes into Claude Code's
//!   `settings.json`. An Edit or Write is denied when the file it would
//!   produce (worked out from the file on disk) lacks a `playbook hook`
//!   command the file has now, sets `disableAllHooks`, or switches the
//!   playbook plugin off. Bash is denied when it runs `playbook uninstall
//!   --yes`. An agent that could drop the guard entries could run without
//!   them, so these are the user's to change, in `ask` mode too. Other
//!   settings changes pass.
//!
//! It also shows the memory fact format (`MEMORY_SCHEMA`) when a fact file is
//! written to playbook's store, so the system prompt does not carry it.
//!
//! One hook on two matchers: Bash looks at the command, Edit and Write look at
//! the target path, and for a settings file also the file on disk. It fails safe: a payload it cannot read, a command the
//! reader cannot split, or anything unexpected allows the call, and it never
//! spawns a process. `POLICY_GUARD=0` turns it off. It only denies, it never
//! writes anything.
//!
//! A guardrail against an agent drifting, not a security boundary: it does not
//! expand variables, aliases or scripts, so an obfuscated spelling passes.

use crate::common::payload::Payload;
use crate::common::shell::{program_index, program_name, simple_commands};
use crate::common::{emit_pre_context, emit_pre_deny};
use std::collections::BTreeSet;
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

const RM_GUARD_CONFIG_REASON: &str = "The rm guard's allowed roots, the prose lint mode and the pull request rules are the user's to change, not the model's. Ask the user to run the command themselves in the prompt with `!`, for example `! playbook config set --global hooks.rmGuard.allowedRoots '[\"~/scratch/builds\"]'`. Until then, work within the current setting and say so.";

const HOOK_ENTRIES_REASON: &str = "The hook entries in settings.json are the user's to change, not the model's. They are the guards that keep this session safe. Ask the user to run `playbook init --yes` (to add them) or `playbook uninstall` (to remove them) themselves in the prompt with `!`, or to edit settings.json by hand. Until then, leave the hook entries as they are and say so; in an unattended run, report this and stop. A settings change that keeps every hook entry is fine with the Edit tool.";

/// What marks an entry `playbook init` owns.
const HOOK_MARK: &str = crate::init::wire::HOOK_COMMAND_PREFIX;

const MEMORY_REASON: &str = "Claude Code's own memory is off limits here. Save the fact in playbook's store instead: ~/.config/playbook/memory/ for a global fact, <owner>/<repo>/ under it for a project fact.";

pub fn run(payload: &Payload) {
    if std::env::var("POLICY_GUARD").as_deref() == Ok("0") {
        return;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let command = payload.field(".tool_input.command");
    let path = payload.field(".tool_input.file_path");
    let mut new = payload.field(".tool_input.content");
    if new.is_empty() {
        new = payload.field(".tool_input.new_string");
    }
    let reason = if !command.is_empty() {
        check_command(&command, &home)
    } else if let Some(settings) = settings_path(&path, &home) {
        let edit = SettingsEdit {
            old: payload.field(".tool_input.old_string"),
            replace_all: payload.field(".tool_input.replace_all") == "true",
            whole_file: payload.field(".tool_name") == "Write",
        };
        check_settings_edit(&settings, &edit, &new).or_else(|| check_path(&path, &home, &new))
    } else {
        check_path(&path, &home, &new)
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

/// The config keys the model may not change: the `rm` guard's roots, the
/// prose lint mode and the pull request rules. Matched on lowercase prefixes.
const PROTECTED_KEY_PREFIXES: [&str; 5] = [
    "hooks.rmguard",
    "hooks.proselint",
    "pr.rules.",
    "pr.bodymaxchars",
    "pr.ticketpatterns",
];

fn is_protected_config_key(word: &str) -> bool {
    let key = word.to_lowercase();
    PROTECTED_KEY_PREFIXES.iter().any(|p| key.starts_with(p))
}

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
                .position(|w| w == "-c" || is_short_cluster_with(w, 'c'))
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
                .position(|w| matches!(program_name(w).as_str(), "playbook" | "pb"))
                .is_some_and(|i| {
                    let inner = &rest[i..];
                    changes_rm_guard_config_at(inner, 0, home, depth + 1)
                });
        }
    }
    let mut args = rest;
    match program.as_str() {
        "playbook" | "pb" => {}
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
            .any(|w| is_protected_config_key(w)),
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
/// under a `.claude` directory. Case is ignored because the default macOS and
/// Windows filesystems ignore it.
fn is_claude_settings(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase());
    path.parent()
        .and_then(Path::file_name)
        .is_some_and(|d| d.to_string_lossy().eq_ignore_ascii_case(".claude"))
        && name.is_some_and(|n| is_settings_name(&n))
}

/// `path` as a Claude Code settings file, looking through a symlink, or
/// `None` when it is some other file.
fn settings_path(path: &str, home: &str) -> Option<PathBuf> {
    if path.is_empty() {
        return None;
    }
    let direct = lexical(Path::new(path));
    if is_claude_settings(&direct) {
        return Some(direct);
    }
    let real = match std::fs::canonicalize(&direct) {
        Ok(real) => real,
        // A file that does not exist yet: resolve the folder it goes in.
        Err(_) => std::fs::canonicalize(direct.parent()?)
            .ok()?
            .join(direct.file_name()?),
    };
    if is_claude_settings(&real) {
        return Some(real);
    }
    // A dotfiles setup links ~/.claude/settings.json to a file elsewhere, and
    // an edit of that file is an edit of the settings.
    ["settings.json", "settings.local.json"]
        .iter()
        .filter(|_| !home.is_empty())
        .any(|name| {
            std::fs::canonicalize(Path::new(home).join(".claude").join(name))
                .is_ok_and(|linked| linked == real)
        })
        .then_some(real)
}

/// `path` with `.` and `..` resolved by text, as `is_claude_memory_path` does,
/// so `.claude/agents/../settings.json` is seen for what it is.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            part => out.push(part),
        }
    }
    out
}

/// The text of a settings file, or `None` when it is missing, not a regular
/// file or larger than a settings file can sensibly be.
fn read_settings(path: &Path) -> Option<String> {
    const LIMIT: u64 = 1 << 20;
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > LIMIT {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// What the guard watches in a settings file.
#[derive(Debug, Default)]
struct Guarded {
    /// Event, matcher, command, `type` and `if` of every `playbook hook`
    /// entry. Changing any of them can stop the hook from running.
    entries: BTreeSet<(String, String, String, String, String)>,
    /// `disableAllHooks` is on.
    all_off: bool,
    /// The playbook plugin is switched off, which pauses the hooks.
    paused: bool,
}

/// The guarded parts of a settings text, or `None` when it is not JSON.
fn guarded_state(text: &str) -> Option<Guarded> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut state = Guarded {
        all_off: !matches!(
            root.get("disableAllHooks"),
            None | Some(serde_json::Value::Null | serde_json::Value::Bool(false))
        ),
        paused: crate::common::plugin_state::disables_plugin(&root),
        ..Guarded::default()
    };
    let events = root.get("hooks").and_then(serde_json::Value::as_object);
    for (event, groups) in events.into_iter().flatten() {
        for group in groups.as_array().into_iter().flatten() {
            let matcher = group.get("matcher").and_then(serde_json::Value::as_str);
            let hooks = group.get("hooks").and_then(serde_json::Value::as_array);
            for hook in hooks.into_iter().flatten() {
                let command = hook.get("command").and_then(serde_json::Value::as_str);
                if let Some(command) = command.filter(|c| c.starts_with(HOOK_MARK)) {
                    let text = |key: &str| {
                        hook.get(key)
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    };
                    state.entries.insert((
                        event.clone(),
                        matcher.unwrap_or_default().to_string(),
                        command.to_string(),
                        text("type"),
                        text("if"),
                    ));
                }
            }
        }
    }
    Some(state)
}

/// Whether going from `before` to `after` drops a `playbook hook` entry,
/// switches all hooks off or pauses the playbook plugin. A `before` that is
/// not JSON has nothing to lose.
fn loses_guard(before: &str, after: &str) -> bool {
    let Some(was) = guarded_state(before) else {
        return false;
    };
    let Some(now) = guarded_state(after) else {
        // Claude Code stops reading a settings file that is not JSON.
        return !was.entries.is_empty();
    };
    !was.entries.is_subset(&now.entries)
        || (now.all_off && !was.all_off)
        || (now.paused && !was.paused)
}

/// The parts of an Edit or Write the settings check needs besides the new text.
struct SettingsEdit {
    /// The text an Edit replaces.
    old: String,
    replace_all: bool,
    /// A Write replaces the whole file.
    whole_file: bool,
}

/// An Edit or Write on a settings file that drops a `playbook hook` entry,
/// turns hooks off or pauses the plugin. The result of the edit is worked out
/// from the file on disk, so an edit that never names the entry (renaming the
/// `hooks` key, say) is caught, and one that keeps every entry is allowed.
fn check_settings_edit(path: &Path, edit: &SettingsEdit, new: &str) -> Option<&'static str> {
    let on_disk = read_settings(path);
    // A file that is there but cannot be read (a device, or too large to be
    // settings) cannot be judged, so it is not touched.
    if on_disk.is_none() && std::fs::metadata(path).is_ok() {
        return Some(HOOK_ENTRIES_REASON);
    }
    let loses = match on_disk {
        Some(before) if edit.whole_file => loses_guard(&before, new),
        // An Edit always names text; do not guess at the result of an empty one.
        Some(_) if edit.old.is_empty() => false,
        Some(before) => {
            // JSON does not care about line endings, so replay on LF text.
            let (before, old) = (before.replace("\r\n", "\n"), edit.old.replace("\r\n", "\n"));
            if !before.contains(&old) {
                // The text the Edit names is not in the file, so the result
                // cannot be replayed. Judge the text of the edit.
                edit_names_the_guard(&edit.old, new)
            } else {
                let after = if edit.replace_all {
                    before.replace(&old, new)
                } else {
                    before.replacen(&old, new, 1)
                };
                loses_guard(&before, &after)
            }
        }
        // A new file has nothing to lose, but may still switch hooks off.
        None if edit.whole_file => loses_guard("{}", new),
        // No file to read: fall back to the text of the edit.
        None => edit_names_the_guard(&edit.old, new),
    };
    loses.then_some(HOOK_ENTRIES_REASON)
}

/// Whether the text of an Edit names a `playbook hook` entry or
/// `disableAllHooks`, used when the file cannot be replayed.
fn edit_names_the_guard(old: &str, new: &str) -> bool {
    [old, new]
        .iter()
        .any(|text| text.contains(HOOK_MARK) || text.contains("disableAllHooks"))
}

/// The simple commands inside a shell `-c` script or behind a runner program
/// (`nohup`, `xargs`), each as a word list starting at its program.
fn wrapped_commands(words: &[String], program_at: usize) -> Vec<Vec<String>> {
    let program = program_name(&words[program_at]);
    let rest = &words[program_at + 1..];
    if SHELL_PROGRAMS.contains(&program.as_str()) {
        let script = rest
            .iter()
            .position(|w| w == "-c" || is_short_cluster_with(w, 'c'))
            .and_then(|at| rest.get(at + 1));
        return script.map(|s| simple_commands(s)).unwrap_or_default();
    }
    if RUNNER_PROGRAMS.contains(&program.as_str()) {
        return rest
            .iter()
            .position(|w| matches!(program_name(w).as_str(), "playbook" | "pb"))
            .map(|at| vec![rest[at..].to_vec()])
            .unwrap_or_default();
    }
    Vec::new()
}

/// A `playbook uninstall` that goes ahead (`--yes`, no `--dry-run`), alone or
/// behind `cargo run`, a runner program or a shell `-c`.
fn uninstalls_playbook(words: &[String], program_at: usize, depth: usize) -> bool {
    if depth < 3
        && wrapped_commands(words, program_at).iter().any(|inner| {
            program_index(inner).is_some_and(|at| uninstalls_playbook(inner, at, depth + 1))
        })
    {
        return true;
    }
    let mut args = &words[program_at + 1..];
    match program_name(&words[program_at]).as_str() {
        "playbook" | "pb" => {}
        "cargo" => match super::auto_cost::cargo_run_args(args) {
            Some(after) => args = after,
            None => return false,
        },
        _ => return false,
    }
    args.iter()
        .find(|w| !w.starts_with('-'))
        .map(String::as_str)
        == Some("uninstall")
        && args.iter().any(|w| w == "--yes")
        && !args.iter().any(|w| w == "--dry-run")
}

/// Whether a lower-case file name is `settings*.json`.
fn is_settings_name(name: &str) -> bool {
    name.starts_with("settings") && name.ends_with(".json")
}

/// Whether `word` is a cluster of short options (one dash) that includes `flag`.
fn is_short_cluster_with(word: &str, flag: char) -> bool {
    word.starts_with('-') && !word.starts_with("--") && word.contains(flag)
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
        if uninstalls_playbook(&words, at, 0) {
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
        "config",
        "uninstall",
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
    if writes_guard_env_to_settings(path, home, content) {
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
fn writes_guard_env_to_settings(path: &str, home: &str, content: &str) -> bool {
    settings_path(path, home).is_some()
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
    fn changes_to_protected_config_are_denied() {
        for command in [
            "playbook config set --global hooks.rmGuard.allowedRoots '[\"/opt/a/b\"]'",
            "playbook config set hooks.rmGuard.allowedRoots ~/x/y",
            "playbook config set --org hooks.rmguard.allowedroots x",
            "playbook config set hooks.proseLint.mode off",
            "pb config set hooks.proseLint.mode off",
            "nohup pb config set pr.rules.template off",
            "playbook config set --global pr.rules.prose off",
            "playbook config set pr.bodyMaxChars 100000",
            "playbook config set --org acme pr.ticketPatterns '[\"X-1\"]'",
            "playbook config unset pr.rules.length",
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
            "playbook config get hooks.proseLint.mode",
            "playbook config set --global pr.draft false",
            "playbook config set pr.rulesX true",
            "playbook config get pr.rules.prose",
            "echo \"playbook config set hooks.proseLint.mode off\"",
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

    const ENTRIES: &str = r#"{"model":"opus","hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"playbook hook policy-guard"}]}],"Stop":[{"hooks":[{"command":"playbook hook session-clean-exit"},{"command":"echo mine"}]}]}}"#;

    /// A scratch `.claude` folder holding `body` as settings.json.
    fn settings_with(tag: &str, body: &str) -> PathBuf {
        let dir = crate::common::test_support::scratch_dir(tag).join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, body).unwrap();
        path
    }

    fn edit(old: &str) -> SettingsEdit {
        SettingsEdit {
            old: old.to_string(),
            replace_all: false,
            whole_file: false,
        }
    }

    fn write() -> SettingsEdit {
        SettingsEdit {
            old: String::new(),
            replace_all: false,
            whole_file: true,
        }
    }

    #[test]
    fn a_change_that_drops_or_disables_an_entry_is_caught() {
        for after in [
            ENTRIES.replace("\"hooks\":{", "\"hooks_off\":{"),
            ENTRIES.replace("playbook hook policy-guard", "playbook hook policy-guardx"),
            ENTRIES.replace("\"matcher\":\"Bash\"", "\"matcher\":\"Nope\""),
            ENTRIES.replace("\"PreToolUse\"", "\"PreToolUseOff\""),
            ENTRIES.replace("{\"model\"", "{\"disableAllHooks\":true,\"model\""),
            ENTRIES.replace("{\"model\"", "{\"disable\\u0041llHooks\":true,\"model\""),
            ENTRIES.replace(
                "{\"model\"",
                "{\"enabledPlugins\":{\"playbook@pragmatic-engineer\":false},\"model\"",
            ),
            r#"{"model":"opus"}"#.to_string(),
            "not json".to_string(),
        ] {
            assert!(loses_guard(ENTRIES, &after), "{after}");
        }
    }

    #[test]
    fn a_change_that_keeps_every_entry_is_allowed() {
        for after in [
            ENTRIES.replace("opus", "sonnet"),
            ENTRIES.replace("echo mine", "echo yours"),
            ENTRIES.replace(
                "{\"model\"",
                "{\"permissions\":{\"allow\":[\"Bash(ls)\"]},\"model\"",
            ),
            ENTRIES.replace("{\"model\"", "{\"disableAllHooks\":false,\"model\""),
            ENTRIES.replace(
                "]}]}}",
                "]}],\"Notification\":[{\"hooks\":[{\"command\":\"playbook hook x\"}]}]}}",
            ),
        ] {
            assert!(!loses_guard(ENTRIES, &after), "{after}");
        }
        // Nothing to lose when the file was not ours or not JSON.
        assert!(!loses_guard("not json", "{}"));
        assert!(!loses_guard(r#"{"model":"opus"}"#, r#"{"model":"sonnet"}"#));
    }

    #[test]
    fn an_edit_is_judged_by_the_file_it_produces() {
        let path = settings_with("hook-edit", ENTRIES);

        // Renames the key without naming an entry.
        assert_eq!(
            check_settings_edit(&path, &edit("\"hooks\":{"), "\"hooks_off\":{"),
            Some(HOOK_ENTRIES_REASON)
        );
        // Removes the entry by name.
        assert_eq!(
            check_settings_edit(&path, &edit("playbook hook policy-guard"), "true"),
            Some(HOOK_ENTRIES_REASON)
        );
        // Unrelated changes pass, and an Edit with no text to match is left alone.
        assert_eq!(check_settings_edit(&path, &edit("opus"), "sonnet"), None);
        assert_eq!(check_settings_edit(&path, &edit(""), "x"), None);
    }

    #[test]
    fn replace_all_decides_whether_a_later_match_is_reached() {
        // The first "policy-guard" is a harmless note, the second is the entry.
        let body = r#"{"note":"policy-guard","hooks":{"Stop":[{"hooks":[{"command":"playbook hook policy-guard"}]}]}}"#;
        let path = settings_with("replace-all", body);
        let all = SettingsEdit {
            replace_all: true,
            ..edit("policy-guard")
        };

        assert_eq!(check_settings_edit(&path, &edit("policy-guard"), "x"), None);
        assert_eq!(
            check_settings_edit(&path, &all, "x"),
            Some(HOOK_ENTRIES_REASON)
        );
    }

    #[test]
    fn an_edit_that_does_not_match_the_file_is_judged_by_its_text() {
        // CRLF on disk, LF in the edit: the edit is replayed on LF text, so a
        // key rename that spans a line break is still caught.
        let path = settings_with("edit-nomatch", &ENTRIES.replace(',', ",\r\n"));
        assert_eq!(
            check_settings_edit(&path, &edit("\"opus\",\n\"hooks\""), "\"opus\",\n\"off\""),
            Some(HOOK_ENTRIES_REASON)
        );

        assert_eq!(
            check_settings_edit(
                &path,
                &edit("\"type\":\"command\",\n\"command\":\"playbook hook policy-guard\""),
                ""
            ),
            Some(HOOK_ENTRIES_REASON)
        );
        assert_eq!(check_settings_edit(&path, &edit("no such text"), "x"), None);
    }

    #[test]
    fn a_settings_file_that_cannot_be_read_is_left_alone() {
        let dir = crate::common::test_support::scratch_dir("settings-big");
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        let big = dir.join(".claude/settings.json");
        std::fs::write(&big, vec![b' '; (1 << 20) + 1]).unwrap();

        assert_eq!(
            check_settings_edit(&big, &write(), r#"{"model":"opus"}"#),
            Some(HOOK_ENTRIES_REASON)
        );
        assert_eq!(
            check_settings_edit(&big, &edit(" "), "x"),
            Some(HOOK_ENTRIES_REASON)
        );
    }

    #[test]
    fn a_path_with_dots_or_a_missing_file_still_reaches_the_settings_check() {
        let dir = crate::common::test_support::scratch_dir("settings-dots");
        std::fs::create_dir_all(dir.join(".claude/agents")).unwrap();
        let sneaky = format!("{}/.claude/agents/../settings.local.json", dir.display());

        assert!(settings_path(&sneaky, "").is_some());
        assert_eq!(lexical(Path::new("/a/b/../c/./d")), PathBuf::from("/a/c/d"));
    }

    #[test]
    fn a_file_that_is_not_a_regular_small_file_is_not_read() {
        let dir = crate::common::test_support::scratch_dir("settings-read");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_settings(&dir).is_none());
        let big = dir.join("big.json");
        std::fs::write(&big, vec![b' '; (1 << 20) + 1]).unwrap();
        assert!(read_settings(&big).is_none());
        let ok = dir.join("ok.json");
        std::fs::write(&ok, "{}").unwrap();
        assert_eq!(read_settings(&ok).as_deref(), Some("{}"));
    }

    #[test]
    fn a_write_is_judged_against_the_file_on_disk() {
        let path = settings_with("hook-write", ENTRIES);

        assert_eq!(
            check_settings_edit(&path, &write(), r#"{"model":"opus"}"#),
            Some(HOOK_ENTRIES_REASON)
        );
        assert_eq!(
            check_settings_edit(&path, &write(), &ENTRIES.replace("opus", "sonnet")),
            None
        );
    }

    #[test]
    fn a_file_without_our_entries_can_be_written_but_not_used_to_turn_hooks_off() {
        let plain = settings_with("hook-plain", r#"{"model":"opus"}"#);
        let missing = plain.with_file_name("settings.local.json");

        for path in [&plain, &missing] {
            assert_eq!(
                check_settings_edit(path, &write(), r#"{"model":"sonnet"}"#),
                None
            );
        }
        assert_eq!(
            check_settings_edit(&missing, &write(), r#"{"disableAllHooks":true}"#),
            Some(HOOK_ENTRIES_REASON)
        );
        assert_eq!(
            check_settings_edit(&missing, &edit("a"), "playbook hook x"),
            Some(HOOK_ENTRIES_REASON)
        );
    }

    #[test]
    fn settings_files_are_recognised_whatever_the_case_or_the_link() {
        assert!(is_claude_settings(Path::new(
            "/Users/me/.claude/settings.json"
        )));
        assert!(is_claude_settings(Path::new(
            "/Users/me/.CLAUDE/Settings.local.json"
        )));
        assert!(!is_claude_settings(Path::new("/work/notes/settings.json")));
        assert!(settings_path("", "").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_settings_file_is_followed_both_ways() {
        let real = settings_with("hook-link", ENTRIES);
        let link = real.parent().unwrap().parent().unwrap().join("link.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(settings_path(&link.to_string_lossy(), "").is_some());

        // A dotfiles setup: ~/.claude/settings.json links to a file elsewhere.
        let home = crate::common::test_support::scratch_dir("dotfiles-home");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let dotfile = home.join("dotfiles-claude.json");
        std::fs::write(&dotfile, ENTRIES).unwrap();
        std::os::unix::fs::symlink(&dotfile, home.join(".claude/settings.json")).unwrap();
        let home = home.to_string_lossy().into_owned();
        assert!(settings_path(&dotfile.to_string_lossy(), &home).is_some());
        assert!(settings_path(&dotfile.to_string_lossy(), "").is_none());
    }

    #[test]
    fn bash_cannot_uninstall_playbook() {
        for command in [
            "playbook uninstall --yes",
            "cd x && playbook uninstall --yes --remove-binary",
            "nohup playbook uninstall --yes",
            "xargs playbook uninstall --yes",
            "bash -c 'playbook uninstall --yes'",
            "cargo run -q -- uninstall --yes",
            "/usr/local/bin/playbook uninstall --yes",
            "pb uninstall --yes",
            "nohup pb uninstall --yes",
        ] {
            assert_eq!(deny(command), Some(HOOK_ENTRIES_REASON), "{command}");
        }
        for command in [
            "playbook uninstall",
            "playbook uninstall --dry-run",
            "playbook uninstall --yes --dry-run",
            "playbook init",
            "cargo run -q -- doctor",
        ] {
            assert_eq!(deny(command), None, "{command}");
        }
    }

    #[test]
    fn a_shell_with_option_flags_before_c_is_still_unwrapped() {
        assert_eq!(
            deny("bash --norc -c 'playbook uninstall --yes'"),
            Some(HOOK_ENTRIES_REASON)
        );
        assert_eq!(deny("bash --norc -c 'playbook doctor'"), None);
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
