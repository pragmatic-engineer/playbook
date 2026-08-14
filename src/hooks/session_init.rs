// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! SessionStart hook (ports hooks/session-init.py). Prepares the per-session
//! runtime directory and zeroes its counters, warns on a resumed session
//! whose config has drifted since it was created, and injects SessionStart
//! additionalContext: the project memory slice, an auto-learn nudge, a
//! skills/commands primer, and the async/deferred-tool discipline reminder.
//!
//! Two steps shell out to the bash scripts that remain the single source of
//! truth for their computation: `hooks/lib/config-hash.sh` (config_hash) and
//! `shell/memory-context.sh` (the memory slice), both resolved under
//! `CLAUDE_PLUGIN_ROOT`. Their output is folded in unchanged; either
//! shelling out itself, or the script it calls, failing degrades quietly
//! rather than breaking the hook.

use crate::common::{home_dir, repo_slug, run_with_timeout, session_dir, Payload};
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long to wait for a shelled-out `bash` or `git` call before giving up.
/// Matches hooks/session-init.py:29's `timeout=15`.
const SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(15);

/// The five per-session counter/state files zeroed at the start of every
/// session. Matches hooks/session-init.py:88 exactly; anything else in the
/// session directory (config-hash, start-ts, clean-exit, ...) is untouched.
const SESSION_COUNTER_FILES: [&str; 5] = [
    "search-count",
    "tool-count",
    "edit-count",
    "edits.jsonl",
    "seen-reads",
];

const DEFAULT_AUTO_LEARN_MAX_AGE_DAYS: i64 = 14;

const DRIFT_SYSTEM_MESSAGE: &str = "\u{26a0} Claude config (settings.json + hooks) has drifted \
    since this session was created. Plugins, output style, model default, and new hooks will \
    NOT take effect on this resumed session: they're frozen at the original startup snapshot. \
    To apply current config: exit and run `cc fresh` (or `claude` without --resume).";

const DRIFT_EXTRA_CONTEXT: &str = "The user resumed this session, but the config hash has \
    changed since session creation. The harness has the OLD settings loaded. If the user asks \
    about why a recent settings change isn't showing up, point them to 'cc fresh' or starting \
    a new `claude` invocation.";

const TOOLKIT_PREAMBLE: &str = "Your toolkit. Before substantive work, check whether one of \
    these fits and use it instead of ad-hoc steps: plan a feature with /scope, execute a ready \
    plan with /implement, record a decision with /adr, commit and push with /commit-and-push, \
    open a PR with /create-pull-request, review a PR with /quick-review or /deep-review, debug \
    a failure with the systematic-debugging skill. Invoke skills via the Skill tool, commands \
    as slash commands. Full catalog (name: what it is for):";

const ASYNC_DISCIPLINE_TEXT: &str = "Async and deferred-tool discipline. (1) Deferred tools \
    are surfaced by name only (e.g. Monitor, TaskCreate, TaskStop, TaskUpdate, ScheduleWakeup): \
    their schemas are NOT loaded, so calling them with guessed parameters fails validation. \
    Before calling any tool that is not already in your active tool list, load it first with \
    ToolSearch (query \"select:NAME\"), then call it; never guess its parameters. (2) Don't run \
    a command in the background when the next step needs its result (installs, builds, \
    typechecks): run it in the foreground with an extended timeout (up to 600000ms). A \
    backgrounded job re-invokes you only when it exits, and shell state (including `wait`) \
    does not persist across Bash calls, so there is nothing to poll.";

/// Run the session-init hook: reset per-session state, warn on config drift,
/// and emit a single SessionStart payload with whatever additionalContext
/// applies. Never panics; every failure along the way degrades to "say
/// nothing" rather than breaking the session.
pub fn run(payload: &Payload) {
    let home = home_dir().to_string_lossy().into_owned();
    let plugin_root = std::env::var("CLAUDE_PLUGIN_ROOT").unwrap_or_default();
    let dir = session_dir(payload);
    let repo_root = git_toplevel();

    zero_session_state(&dir);
    clear_statusline_cache();

    let (system_message, mut extra_context) = check_config_drift(payload, &dir, &plugin_root);

    append_memory_slice(&mut extra_context, &plugin_root, &home, &repo_root);
    append_auto_learn_nudge(&mut extra_context, &home, &repo_root);
    append_skills_primer(&mut extra_context, &home);
    append_async_discipline(&mut extra_context);

    emit(&system_message, &extra_context);
}

/// Zero the five per-session counter files and stamp `start-ts`, matching
/// hooks/session-init.py:86-98. A no-op when there is no session directory
/// (no session id in the payload).
fn zero_session_state(dir: &str) {
    if dir.is_empty() {
        return;
    }
    let base = Path::new(dir);
    for name in SESSION_COUNTER_FILES {
        let _ = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(base.join(name));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = fs::write(base.join("start-ts"), now.to_string());
}

/// Clear the statusline PR/CI cache entries for the current repo+branch, so
/// the first render of a new session fetches fresh data. Matches
/// hooks/session-init.py:100-113.
fn clear_statusline_cache() {
    let home = home_dir().to_string_lossy().into_owned();
    let sl_cache = std::env::var("STATUSLINE_CACHE_DIR").unwrap_or_else(|_| {
        let xdg = std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| format!("{home}/.cache"));
        format!("{xdg}/statusline")
    });
    let branch = git_branch();
    if branch.is_empty() || branch == "HEAD" {
        return;
    }
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let slug = slugify(&format!("{cwd}::{branch}"));
    for prefix in ["pr", "ci"] {
        let _ = fs::remove_file(Path::new(&sl_cache).join(format!("{prefix}-{slug}.json")));
    }
}

/// Resume-only config-drift check: computes the current config hash by
/// shelling out to `hooks/lib/config-hash.sh`, compares it against the hash
/// stored at session creation, and returns the `(system_message,
/// extra_context)` warning pair to emit if they differ. Always refreshes
/// the stored hash on a `startup` source, which becomes the new baseline.
/// Matches hooks/session-init.py:120-152.
fn check_config_drift(payload: &Payload, dir: &str, plugin_root: &str) -> (String, String) {
    let current_hash = config_hash(plugin_root);
    if current_hash.is_empty() || dir.is_empty() {
        return (String::new(), String::new());
    }
    let hash_file = Path::new(dir).join("config-hash");
    let source = payload.field(".source");

    let mut system_message = String::new();
    let mut extra_context = String::new();
    if source == "resume" {
        let prev_hash = fs::read_to_string(&hash_file)
            .unwrap_or_default()
            .trim()
            .to_string();
        if !prev_hash.is_empty() && prev_hash != current_hash {
            system_message = DRIFT_SYSTEM_MESSAGE.to_string();
            extra_context = DRIFT_EXTRA_CONTEXT.to_string();
        }
    }

    if source == "startup" {
        let _ = fs::write(&hash_file, &current_hash);
    }

    (system_message, extra_context)
}

/// Shell out to `hooks/lib/config-hash.sh`'s `config_hash` function and
/// return its trimmed stdout, or an empty string on any failure (missing
/// plugin root, missing bash, non-zero exit, ...). Never panics. Matches
/// hooks/session-init.py:35-38.
fn config_hash(plugin_root: &str) -> String {
    if plugin_root.is_empty() {
        return String::new();
    }
    let script = Path::new(plugin_root)
        .join("hooks")
        .join("lib")
        .join("config-hash.sh");
    let mut command = Command::new("bash");
    command
        .arg("-c")
        .arg(". \"$1\"; config_hash")
        .arg("_")
        .arg(&script);
    match run_with_timeout(&mut command, SUBPROCESS_TIMEOUT) {
        Some(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    }
}

/// Inject the project memory slice into `extra_context`: the graph-backed
/// slice from `shell/memory-context.sh` when available, else the legacy
/// `MEMORY.md` index, else nothing. Matches hooks/session-init.py:154-188.
fn append_memory_slice(extra_context: &mut String, plugin_root: &str, home: &str, repo_root: &str) {
    let mem_slug = repo_slug();
    if repo_root.is_empty() || mem_slug.is_empty() {
        return;
    }

    let mem_script = if plugin_root.is_empty() {
        None
    } else {
        Some(
            Path::new(plugin_root)
                .join("shell")
                .join("memory-context.sh"),
        )
    };
    let mut mem_body = match &mem_script {
        Some(script) if script.is_file() => {
            run_memory_context(script, &mem_slug).unwrap_or_default()
        }
        _ => String::new(),
    };

    let mem_preamble = if !mem_body.is_empty() {
        format!(
            "Project memory for this repo ({mem_slug}), stored in the central memory store at \
            ~/.claude/memory/{mem_slug}/. A scoped slice: facts in scope, their typed edges, \
            and an anchor index mapping code paths to the facts that describe them. Fact \
            bodies are read on demand."
        )
    } else {
        let legacy = Path::new(home)
            .join(".claude")
            .join("memory")
            .join(&mem_slug)
            .join("MEMORY.md");
        if legacy.is_file() {
            mem_body = read_legacy_memory(&legacy);
        }
        format!(
            "Project memory for this repo ({mem_slug}), stored in the central memory store at \
            ~/.claude/memory/{mem_slug}/. These facts apply only in this repo; read the \
            referenced fact files on demand. Index:"
        )
    };

    if mem_body.is_empty() {
        return;
    }
    let mem_ctx = format!("{mem_preamble}\n{mem_body}");
    push_context(extra_context, &mem_ctx);
}

/// Shell out to `shell/memory-context.sh --repo <slug>` and return its
/// stdout with surrounding newlines trimmed, or `None` on any failure
/// (missing bash, non-zero exit, ...). Never panics.
fn run_memory_context(script: &Path, mem_slug: &str) -> Option<String> {
    let mut command = Command::new("bash");
    command.arg(script).arg("--repo").arg(mem_slug);
    match run_with_timeout(&mut command, SUBPROCESS_TIMEOUT) {
        Some(o) if o.status.success() => Some(
            String::from_utf8_lossy(&o.stdout)
                .trim_matches('\n')
                .to_string(),
        ),
        _ => None,
    }
}

/// Read up to the first 16000 characters of the legacy `MEMORY.md` index.
/// Empty on any read failure. Matches hooks/session-init.py:173-179.
fn read_legacy_memory(path: &Path) -> String {
    let Ok(contents) = fs::read_to_string(path) else {
        return String::new();
    };
    contents.chars().take(16000).collect()
}

/// Nudge the user to refresh project memory if a previous session queued an
/// auto-learn flag for this repo. Prunes stale flags first. Matches
/// hooks/session-init.py:190-211.
fn append_auto_learn_nudge(extra_context: &mut String, home: &str, repo_root: &str) {
    if std::env::var("AUTO_LEARN_NUDGE").unwrap_or_else(|_| "1".to_string()) == "0" {
        return;
    }
    if repo_root.is_empty() {
        return;
    }
    let qdir = Path::new(home)
        .join(".claude")
        .join("runtime")
        .join("to-learn");
    // Trim before parsing: python's `int(...)` strips surrounding
    // whitespace, so a padded value must parse the same way here rather
    // than silently falling back to the default. Matches
    // hooks/session-init.py:193.
    let max_age_days = std::env::var("AUTO_LEARN_MAX_AGE_DAYS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_AUTO_LEARN_MAX_AGE_DAYS);
    prune_old(&qdir, max_age_days);

    let learn_flag = qdir.join(format!("{}.json", slugify(repo_root)));
    if !learn_flag.is_file() {
        return;
    }
    let edits = learn_flag_edits(&learn_flag);
    let nudge = format!(
        "A previous session in this repo made {edits} edits, so project memory may be stale. \
        Consider running /learn-project to refresh it, or /learn-project --stage to queue \
        candidate facts for review."
    );
    push_context(extra_context, &nudge);
    let _ = fs::remove_file(&learn_flag);
}

/// Remove `*.json` flags older than `max_age_days` from `qdir`. Silently
/// does nothing if `qdir` does not exist. Never panics.
fn prune_old(qdir: &Path, max_age_days: i64) {
    let Ok(entries) = fs::read_dir(qdir) else {
        return;
    };
    let seconds = (max_age_days.max(0) as u64).saturating_mul(86400);
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(seconds))
        .unwrap_or(UNIX_EPOCH);
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        if let Ok(modified) = metadata.modified() {
            if modified < cutoff {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

/// The `edits` count to report in the auto-learn nudge: the flag file's
/// `edits` field if it parses as a JSON object, `"0"` if the field is
/// absent, `"some"` on any read or parse failure. Matches
/// hooks/session-init.py:196-201.
fn learn_flag_edits(path: &Path) -> String {
    let Ok(contents) = fs::read_to_string(path) else {
        return "some".to_string();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return "some".to_string();
    };
    let Some(object) = value.as_object() else {
        return "some".to_string();
    };
    match object.get("edits") {
        None => "0".to_string(),
        Some(serde_json::Value::Number(n)) => n
            .as_i64()
            .map(|i| i.to_string())
            .unwrap_or_else(|| n.to_string()),
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// Build the "Your toolkit" primer from the installed skills and commands,
/// as `- name: description` lines. Matches hooks/session-init.py:213-231.
fn append_skills_primer(extra_context: &mut String, home: &str) {
    if std::env::var("SKILLS_PRIMER").unwrap_or_else(|_| "1".to_string()) == "0" {
        return;
    }
    let skills_root = Path::new(home).join(".claude").join("skills");
    let commands_root = Path::new(home).join(".claude").join("commands");
    let skill_lines = catalog_skills(&skills_root);
    let cmd_lines = catalog_commands(&commands_root);
    if skill_lines.is_empty() && cmd_lines.is_empty() {
        return;
    }
    let mut toolkit = TOOLKIT_PREAMBLE.to_string();
    if !skill_lines.is_empty() {
        toolkit.push_str("\n\nSkills:\n");
        toolkit.push_str(&skill_lines);
    }
    if !cmd_lines.is_empty() {
        toolkit.push_str("\nCommands:\n");
        toolkit.push_str(&cmd_lines);
    }
    push_context(extra_context, &toolkit);
}

/// `- name: one-line description` for every `<root>/<skill>/SKILL.md`,
/// sorted the same way the bash glob `*/SKILL.md` would be: by the entry
/// name with `/SKILL.md` appended, so the `/` participates in the sort key.
/// Matches hooks/session-init.py:282-310 (the `kind == "skill"` branch).
fn catalog_skills(root: &Path) -> String {
    if !root.is_dir() {
        return String::new();
    }
    let Ok(read) = fs::read_dir(root) else {
        return String::new();
    };
    let mut entries: Vec<String> = read
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    entries.sort_by_cached_key(|entry| format!("{entry}/SKILL.md"));

    let mut lines = Vec::new();
    for entry in entries {
        let skill_file = root.join(&entry).join("SKILL.md");
        if !skill_file.is_file() {
            continue;
        }
        let name = frontmatter_field(&skill_file, "name");
        let name = if name.is_empty() { entry } else { name };
        let description = one_line(&frontmatter_field(&skill_file, "description"));
        lines.push(format!("- {name}: {description}"));
    }
    finalize_catalog_lines(lines)
}

/// `- /name: one-line description` for every `<root>/*.md`, sorted by file
/// name. Matches hooks/session-init.py:282-310 (the `kind == "command"`
/// branch).
fn catalog_commands(root: &Path) -> String {
    if !root.is_dir() {
        return String::new();
    }
    let Ok(read) = fs::read_dir(root) else {
        return String::new();
    };
    let mut entries: Vec<String> = read
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    entries.sort();

    let mut lines = Vec::new();
    for entry in entries {
        let Some(base) = entry.strip_suffix(".md") else {
            continue;
        };
        let command_file = root.join(&entry);
        if !command_file.is_file() {
            continue;
        }
        let description = one_line(&frontmatter_field(&command_file, "description"));
        lines.push(format!("- /{base}: {description}"));
    }
    finalize_catalog_lines(lines)
}

fn finalize_catalog_lines(lines: Vec<String>) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

/// The value of `field` in `path`'s YAML frontmatter (the block between the
/// first line `---` and the next `---`), with leading whitespace and one
/// pair of enclosing quotes stripped. Empty when the file is unreadable,
/// has no frontmatter, or lacks the field. Matches
/// hooks/session-init.py:55-75.
fn frontmatter_field(path: &Path, field: &str) -> String {
    let Ok(contents) = fs::read_to_string(path) else {
        return String::new();
    };
    let mut lines = contents.split('\n');
    if lines.next() != Some("---") {
        return String::new();
    }
    let prefix = format!("{field}:");
    for line in lines {
        if line == "---" {
            break;
        }
        let Some(rest) = line.strip_prefix(&prefix) else {
            continue;
        };
        let value = rest.trim_start_matches([' ', '\t']);
        let value = value.strip_prefix('"').unwrap_or(value);
        let value = value.strip_suffix('"').unwrap_or(value);
        return value.to_string();
    }
    String::new()
}

/// Collapse newlines and tabs to spaces, then truncate to 150 characters
/// (147 plus an ellipsis) so a runaway description cannot blow up the
/// primer. Matches hooks/session-init.py:78-82.
fn one_line(text: &str) -> String {
    let collapsed: String = text
        .chars()
        .map(|c| match c {
            '\n' | '\t' => ' ',
            other => other,
        })
        .collect();
    if collapsed.chars().count() > 150 {
        let truncated: String = collapsed.chars().take(147).collect();
        format!("{truncated}...")
    } else {
        collapsed
    }
}

/// Append the async and deferred-tool discipline reminder, unless disabled.
/// Matches hooks/session-init.py:234-247.
fn append_async_discipline(extra_context: &mut String) {
    if std::env::var("ASYNC_DISCIPLINE").unwrap_or_else(|_| "1".to_string()) == "0" {
        return;
    }
    push_context(extra_context, ASYNC_DISCIPLINE_TEXT);
}

/// Append `addition` to `ctx`, separated by a blank line if `ctx` already
/// has content. Matches the `extra_context = extra_context + "\n\n" + x if
/// extra_context else x` pattern repeated through hooks/session-init.py.
fn push_context(ctx: &mut String, addition: &str) {
    if ctx.is_empty() {
        ctx.push_str(addition);
    } else {
        ctx.push_str("\n\n");
        ctx.push_str(addition);
    }
}

/// Replace every character outside `[A-Za-z0-9_.-]` with `_`. Matches
/// hooks/session-init.py:51-52's `slugify`.
fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `git rev-parse --show-toplevel`, trimmed. Empty outside a repo or on any
/// failure. Never panics.
fn git_toplevel() -> String {
    run_git(&["--no-optional-locks", "rev-parse", "--show-toplevel"])
}

/// `git rev-parse --abbrev-ref HEAD`, trimmed. Empty outside a repo or on
/// any failure. Never panics.
fn git_branch() -> String {
    run_git(&["--no-optional-locks", "rev-parse", "--abbrev-ref", "HEAD"])
}

fn run_git(args: &[&str]) -> String {
    let mut command = Command::new("git");
    command.args(args);
    match run_with_timeout(&mut command, SUBPROCESS_TIMEOUT) {
        Some(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    }
}

#[derive(Serialize)]
struct SessionStartOutput<'a> {
    #[serde(rename = "systemMessage", skip_serializing_if = "Option::is_none")]
    system_message: Option<&'a str>,
    #[serde(rename = "hookSpecificOutput", skip_serializing_if = "Option::is_none")]
    hook_specific_output: Option<SessionStartContext<'a>>,
}

#[derive(Serialize)]
struct SessionStartContext<'a> {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'static str,
    #[serde(rename = "additionalContext")]
    additional_context: &'a str,
}

/// Print the single SessionStart payload, or nothing at all if there is
/// nothing to say. Matches hooks/session-init.py:249-259.
fn emit(system_message: &str, extra_context: &str) {
    if system_message.is_empty() && extra_context.is_empty() {
        return;
    }
    let output = SessionStartOutput {
        system_message: (!system_message.is_empty()).then_some(system_message),
        hook_specific_output: (!extra_context.is_empty()).then_some(SessionStartContext {
            hook_event_name: "SessionStart",
            additional_context: extra_context,
        }),
    };
    if let Ok(rendered) = serde_json::to_string(&output) {
        println!("{rendered}");
    }
}
