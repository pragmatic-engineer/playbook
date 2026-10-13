// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook doctor check`: the seven layer checks and the three
//! informational ones that `commands/doctor.md` used to run as bash blocks.
//! Every probe reads from an `Env`, so tests point it at a scratch home.

use crate::config;
use crate::doctor::field;
use crate::update::{pathcheck, resolve};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// How a row reads in the status table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Pass,
    Info,
    Warn,
    Fail,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Pass => "PASS",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Fail => "FAIL",
        }
    }
}

/// One line of the status table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// `1` to `7` for a layer, `-` for an informational row.
    pub layer: &'static str,
    pub level: Level,
    pub message: String,
    pub hint: Option<String>,
}

impl Row {
    fn new(layer: &'static str, level: Level, message: impl Into<String>) -> Row {
        Row {
            layer,
            level,
            message: message.into(),
            hint: None,
        }
    }

    fn hint(mut self, hint: impl Into<String>) -> Row {
        self.hint = Some(hint.into());
        self
    }
}

/// Everything a probe reads from the machine.
pub struct Env {
    pub home: PathBuf,
    pub claude_home: PathBuf,
    pub plugin_root: Option<PathBuf>,
    pub shell: String,
    pub path_var: OsString,
    pub repo_root: Option<PathBuf>,
    pub repo_slug: Option<String>,
}

impl Env {
    fn settings(&self) -> PathBuf {
        self.claude_home.join("settings.json")
    }
}

const PRE_GUARDS: [&str; 6] = [
    "rm-workspace-guard",
    "bg-await-guard",
    "no-slop-guard",
    "precommit-check",
    "commit-message-sanitizer",
    "policy-guard",
];

/// The guards that must also be wired on PostToolUse.
const POST_GUARDS: [&str; 1] = ["commit-message-sanitizer"];

const INSTALL_HINT: &str = "run: curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash, or brew install pragmatic-engineer/tap/playbook, and make sure its directory is on PATH";

/// One probe: reads the machine and returns its rows.
type Probe = fn(&Env) -> Vec<Row>;

/// Every probe, in table order.
const PROBES: [Probe; 15] = [
    |env| vec![layer1(env)],
    |env| vec![layer2(env)],
    |env| vec![layer3(env)],
    |env| vec![layer4(env)],
    |env| layer5(env).into_iter().collect(),
    layer6,
    layer7,
    config_rows,
    worktree_rows,
    migration_rows,
    data_rows,
    plugin_only_rows,
    plugin_split_rows,
    atlassian_rows,
    old_launcher_rows,
];

/// Run every check. The probes only read, and several wait on child processes
/// (`claude plugin list`, `git`, the binary's own `--version`), so they run
/// side by side. Rows are joined in table order, so the output is unchanged.
pub fn run(env: &Env) -> Vec<Row> {
    crate::common::par::map(&PROBES, crate::common::par::MAX_CONCURRENT, |probe| {
        probe(env)
    })
    .into_iter()
    .flatten()
    .collect()
}

fn on_path(env: &Env, name: &str) -> Option<PathBuf> {
    std::env::split_paths(&env.path_var)
        .filter(|d| d.is_absolute())
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Longest one doctor probe may run.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

fn output_of(mut cmd: Command) -> Option<String> {
    // A hung `claude` or `playbook` must not hang the doctor: a probe that
    // does not answer in time reads as one that could not run.
    let out = crate::common::proc::run_with_timeout(&mut cmd, PROBE_TIMEOUT)?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn layer1(env: &Env) -> Row {
    let miss = "run: claude plugin marketplace add pragmatic-engineer/skills && claude plugin install playbook@pragmatic-engineer";
    let Some(claude) = on_path(env, "claude") else {
        return Row::new(
            "1",
            Level::Info,
            "claude CLI not found, plugin check skipped",
        );
    };
    let mut cmd = Command::new(claude);
    cmd.args(["plugin", "list"]);
    let Some(text) = output_of(cmd) else {
        return Row::new("1", Level::Info, "could not run `claude plugin list`");
    };
    let lower = text.to_lowercase();
    // Match the plugin's own id: the hooks plugin is `playbook-hooks@...` and
    // sorts first in the list.
    let Some(at) = lower.find("playbook@") else {
        return Row::new("1", Level::Fail, "plugin not installed").hint(miss);
    };
    let block_end = lower[at..].find("\n\n").map_or(lower.len(), |n| at + n);
    if lower[at..block_end].contains("disabled") {
        return Row::new("1", Level::Fail, "plugin installed but disabled")
            .hint("run: claude plugin enable playbook@pragmatic-engineer");
    }
    Row::new("1", Level::Pass, "plugin enabled")
}

fn layer2(env: &Env) -> Row {
    let settings = env.settings();
    let mut total = 0;
    let mut wired = 0;
    let mut problems = String::new();
    for (event, guards, suffix) in [
        ("PreToolUse", &PRE_GUARDS[..], ""),
        ("PostToolUse", &POST_GUARDS[..], "(PostToolUse)"),
    ] {
        for (guard, count) in field::hook_commands_for_event(&settings, event, guards) {
            total += 1;
            if count > 0 {
                wired += 1;
            } else {
                problems.push_str(&format!(" {guard}{suffix}:NOT_WIRED"));
            }
        }
    }
    if problems.is_empty() {
        Row::new(
            "2",
            Level::Pass,
            format!("safety guards wired ({wired} of {total})"),
        )
    } else {
        Row::new(
            "2",
            Level::Fail,
            format!("safety guards wired {wired}/{total}, not wired:{problems}"),
        )
        .hint("run `playbook init`, which rewrites every guard to its bare form")
    }
}

fn layer3(env: &Env) -> Row {
    let rc = match env.shell.rsplit('/').next().unwrap_or("") {
        "zsh" => ".zshrc",
        "bash" => ".bashrc",
        _ => {
            return Row::new(
                "3",
                Level::Info,
                "shell not detected, launcher check skipped",
            )
        }
    };
    let text = fs::read_to_string(env.home.join(rc)).unwrap_or_default();
    if text.contains("shell/zsh/cc.zsh") || text.contains("shell/bash/cc.sh") {
        return Row::new(
            "3",
            Level::Info,
            "outdated launcher line (run `playbook init`)",
        );
    }
    if text.contains("playbook shell-init") && on_path(env, "playbook").is_some() {
        return Row::new("3", Level::Pass, "launcher installed");
    }
    Row::new(
        "3",
        Level::Info,
        "launcher not installed (opt-in; run /playbook:setup)",
    )
    .hint("run /playbook:setup and choose Yes for the launcher question")
}

fn layer4(env: &Env) -> Row {
    let file = crate::common::paths::playbook_root_from(&env.home)
        .join("prompts")
        .join("SYSTEM_PROMPT.md");
    if file.is_file() {
        Row::new("4", Level::Pass, "system prompt installed")
    } else {
        Row::new(
            "4",
            Level::Info,
            "system prompt not installed (opt-in, recommended)",
        )
        .hint("run /playbook:setup and choose Yes for the system prompt question")
    }
}

fn expand(path: &str, home: &Path) -> String {
    let home = home.to_string_lossy();
    let path = match path.strip_prefix('~') {
        Some(rest) => format!("{home}{rest}"),
        None => path.to_string(),
    };
    path.replace("$HOME", &home)
}

fn layer5(env: &Env) -> Option<Row> {
    let cmd = field::statusline_command(&env.settings());
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return Some(Row::new(
            "5",
            Level::Info,
            "no status line configured (opt-in)",
        ));
    }
    if cmd == "playbook statusline" {
        return Some(Row::new(
            "5",
            Level::Pass,
            "status line runs `playbook statusline`",
        ));
    }
    if [
        "bash $HOME/.config/playbook/statusline.sh",
        "bash ~/.config/playbook/statusline.sh",
    ]
    .contains(&cmd)
    {
        return Some(
            Row::new(
                "5",
                Level::Fail,
                format!("status line runs the retired statusline.sh: {cmd}"),
            )
            .hint("run `playbook init`, which rewrites it to `playbook statusline`"),
        );
    }
    let last = cmd.split_whitespace().next_back().unwrap_or("");
    let path = expand(last, &env.home);
    if !Path::new(&path).is_file() {
        return Some(
            Row::new(
                "5",
                Level::Fail,
                format!("status line file is missing: {path}"),
            )
            .hint("point statusLine.command at `playbook statusline`, or restore your script"),
        );
    }
    Some(Row::new(
        "5",
        Level::Info,
        format!("status line runs your own command: {cmd}"),
    ))
}

fn version_of(bin: &Path) -> String {
    let mut cmd = Command::new(bin);
    cmd.arg("--version");
    output_of(cmd)
        .and_then(|t| t.split_whitespace().next_back().map(str::to_string))
        .unwrap_or_default()
}

fn manifest_version(env: &Env) -> Option<String> {
    let direct = env
        .plugin_root
        .as_ref()
        .map(|r| r.join(".claude-plugin/plugin.json"))
        .filter(|p| p.is_file());
    let path = direct.or_else(|| newest_cached_manifest(&env.home))?;
    let v = field::plugin_version(&path);
    (!v.is_empty()).then_some(v)
}

fn newest_cached_manifest(home: &Path) -> Option<PathBuf> {
    let cache = home.join(".claude/plugins/cache");
    let mut found = Vec::new();
    for market in fs::read_dir(&cache).ok()?.flatten() {
        let Ok(versions) = fs::read_dir(market.path().join("playbook")) else {
            continue;
        };
        for v in versions.flatten() {
            let manifest = v.path().join(".claude-plugin/plugin.json");
            if manifest.is_file() {
                found.push((v.file_name().to_string_lossy().into_owned(), manifest));
            }
        }
    }
    found.sort_by(|a, b| resolve::compare(&a.0, &b.0));
    found.pop().map(|(_, p)| p)
}

fn layer6(env: &Env) -> Vec<Row> {
    let entries = pathcheck::scan(&env.path_var);
    let Some(first) = entries.first() else {
        return vec![Row::new("6", Level::Fail, "playbook binary not on PATH")
            .hint(format!("every ported hook is dead. {INSTALL_HINT}"))];
    };
    let bin_ver = version_of(first);
    let mut rows = Vec::new();
    if bin_ver.is_empty() {
        rows.push(
            Row::new(
                "6",
                Level::Fail,
                format!(
                    "{} printed no version, so it is not the binary this plugin expects",
                    first.display()
                ),
            )
            .hint(
                "a stale shim or a name collision with another tool: check `command -v playbook`",
            ),
        );
    } else {
        rows.push(match manifest_version(env) {
            Some(m) if m == bin_ver => {
                Row::new("6", Level::Pass, format!("playbook binary on PATH, version {bin_ver}"))
            }
            Some(m) => Row::new(
                "6",
                Level::Info,
                format!("binary {bin_ver} and plugin {m} differ"),
            )
            .hint("update whichever is behind: `playbook update` or `claude plugin update playbook@pragmatic-engineer`"),
            None => Row::new(
                "6",
                Level::Info,
                format!("playbook binary on PATH, version {bin_ver}, no plugin manifest to compare"),
            ),
        });
    }
    let mut gate = Command::new(first);
    gate.args(["gate", "record", "--help"]);
    if !output_of(gate).is_some_and(|t| t.contains("--source")) {
        rows.push(
            Row::new("6", Level::Fail, "gate record has no --source flag, so every quality gate call fails")
                .hint("run `playbook update`, or re-run install.sh if the binary is too old to have `update`"),
        );
    }
    if entries.len() > 1 {
        let versions: Vec<String> = entries.iter().map(|e| version_of(e)).collect();
        let list: Vec<String> = entries
            .iter()
            .zip(&versions)
            .map(|(e, v)| format!("{} {}", e.display(), if v.is_empty() { "?" } else { v }))
            .collect();
        let first_v = &versions[0];
        let stale = !first_v.is_empty()
            && versions[1..]
                .iter()
                .any(|v| !v.is_empty() && resolve::compare(first_v, v) == std::cmp::Ordering::Less);
        rows.push(if stale {
            Row::new("6", Level::Warn, format!("the first playbook on PATH is older than a later one: {}", list.join(", ")))
                .hint("remove the stale one (Homebrew: `brew uninstall playbook`) or put the newer directory first on PATH")
        } else {
            Row::new("6", Level::Info, format!("several playbook binaries on PATH: {}", list.join(", ")))
        });
    }
    rows
}

/// The Atlassian command line tools the `playbook:atlassian` skill can use.
/// Absent tools are informational: the skill falls back to the next route.
fn atlassian_rows(env: &Env) -> Vec<Row> {
    use crate::atlassian::{acli_line, acli_status, twg_line, twg_status, Status, UPGRADE_HINT};
    let acli = acli_status(&env.path_var);
    let acli_row = match &acli {
        Status::Ok(_) => Row::new("-", Level::Pass, acli_line(&acli)),
        Status::Missing => Row::new("-", Level::Info, acli_line(&acli)),
        Status::TooOld(_) | Status::Unreadable => {
            Row::new("-", Level::Warn, acli_line(&acli)).hint(format!("run: {UPGRADE_HINT}"))
        }
    };
    let twg = twg_status(&env.path_var);
    vec![acli_row, Row::new("-", Level::Info, twg_line(&twg))]
}

/// Reports the old `ccc` and `ccd` launchers, which `playbook cc` and
/// `playbook ccd` replace. Informational: it never changes a file.
fn old_launcher_rows(env: &Env) -> Vec<Row> {
    super::launchers::find(&env.home, &env.path_var)
        .iter()
        .map(|f| {
            Row::new("-", Level::Info, super::launchers::describe(f))
                .hint("`pb cc` and `pb ccd` work too when `pb` is installed")
        })
        .collect()
}

fn layer7(env: &Env) -> Vec<Row> {
    let settings = env.settings();
    if !settings.is_file() {
        return vec![Row::new(
            "7",
            Level::Info,
            "no ~/.claude/settings.json, hook paths not checked",
        )];
    }
    let mut checked = 0;
    let mut dangling: Vec<String> = Vec::new();
    for cmd in field::hook_commands(&settings) {
        if cmd.starts_with("playbook hook ") {
            continue;
        }
        let last = cmd.split_whitespace().next_back().unwrap_or("");
        if !last.contains('/') {
            continue;
        }
        let path = expand(last, &env.home);
        if path.contains('$') {
            continue;
        }
        checked += 1;
        if !Path::new(&path).exists() {
            dangling.push(cmd);
        }
    }
    dangling.sort();
    dangling.dedup();
    if dangling.is_empty() {
        return vec![Row::new(
            "7",
            Level::Pass,
            format!("no hook command points at a missing file ({checked} checked)"),
        )];
    }
    dangling
        .into_iter()
        .map(|cmd| {
            Row::new("7", Level::Fail, format!("hook command points at a missing file: {cmd}"))
                .hint("this hook does nothing every time it fires; `playbook init` will not remove it, so delete the entry from ~/.claude/settings.json by hand")
        })
        .collect()
}

fn config_rows(env: &Env) -> Vec<Row> {
    let slug = env.repo_slug.as_deref();
    [
        "autoReview.enabled",
        "autoReview.type",
        "autoReview.fix",
        "autoMerge.enabled",
        "commit.signOff",
        "pr.draft",
    ]
    .iter()
    .map(|key| match config::resolve_valid(key, &env.home, slug) {
        Ok((value, source, ignored)) => {
            let mut origin = source.label().to_string();
            if let Some(ig) = ignored {
                origin = format!("{origin}, {} value ignored", ig.tier.label());
            }
            Row::new(
                "-",
                Level::Info,
                format!(
                    "{key}: {} (source: {origin})",
                    crate::common::mode::value_text(&value)
                ),
            )
        }
        Err(err) => Row::new("-", Level::Info, format!("{key}: could not check: {err}")),
    })
    .collect()
}

fn worktree_rows(env: &Env) -> Vec<Row> {
    let Some(root) = &env.repo_root else {
        return vec![Row::new(
            "-",
            Level::Info,
            "not in a git repo, worktrees not checked",
        )];
    };
    let now = crate::common::time::now_secs();
    match crate::worktree::sweep(root, &env.home, env.repo_slug.as_deref(), true, now) {
        Ok(lines) if lines.is_empty() => {
            vec![Row::new("-", Level::Info, "no stale worktrees found")]
        }
        Ok(lines) => lines
            .into_iter()
            .map(|l| Row::new("-", Level::Info, l))
            .collect(),
        Err(err) => vec![Row::new(
            "-",
            Level::Info,
            format!("worktree check failed: {err}"),
        )],
    }
}

/// One row for the data folder: path, files, sizes and any `*.migrated`
/// leftovers from a finished migration.
fn data_rows(env: &Env) -> Vec<Row> {
    let dir = crate::common::data::data_dir_from(&env.home);
    let files = crate::common::data::migrate::list_files(&dir);
    if files.is_empty() {
        return vec![Row::new(
            "-",
            Level::Info,
            format!("data folder {} is empty or missing", dir.display()),
        )];
    }
    let total: u64 = files.iter().map(|f| f.bytes).sum();
    let left: Vec<String> = files
        .iter()
        .filter(|f| crate::common::data::migrate::is_migrated_leftover(&f.path))
        .map(|f| f.path.display().to_string())
        .collect();
    let mut msg = format!(
        "data folder {}: {} files, {} KB",
        dir.display(),
        files.len(),
        total / 1024
    );
    if !left.is_empty() {
        msg.push_str(&format!(", migrated leftovers: {}", left.join(", ")));
    }
    let mut rows = vec![Row::new("-", Level::Info, msg)];
    rows.extend(orphan_rows(env, &dir));
    rows
}

/// Databases in the data folder with no worktree of this clone. They belong
/// to removed worktrees, or to another clone of the same repo, so the row
/// only reports them.
fn orphan_rows(env: &Env, data: &Path) -> Vec<Row> {
    let (Some(root), Some(slug)) = (&env.repo_root, &env.repo_slug) else {
        return Vec::new();
    };
    let Some(live) = crate::worktree::live_worktree_ids(root) else {
        return Vec::new();
    };
    let orphans = crate::common::data::orphan_dbs(data, slug, &live);
    if orphans.is_empty() {
        return Vec::new();
    }
    let list: Vec<String> = orphans
        .iter()
        .map(|(kind, p)| format!("{kind}: {}", p.display()))
        .collect();
    vec![Row::new(
        "-",
        Level::Info,
        format!(
            "{} database file(s) of {slug} match no worktree of this clone (removed, or another clone): {}",
            orphans.len(),
            list.join(", ")
        ),
    )
    .hint("the code index is a cache and safe to delete; check other clones before deleting gate records")]
}

fn migration_rows(env: &Env) -> Vec<Row> {
    let ctx = crate::init::migrate::Ctx {
        home: env.home.clone(),
        claude_home: env.claude_home.clone(),
        self_root: crate::init::self_root::resolve(
            env.plugin_root.as_ref().and_then(|p| p.to_str()),
            &env.claude_home,
        ),
        repo: None,
    };
    crate::init::migrate::pending_manual(&ctx)
        .into_iter()
        .map(|l| Row::new("-", Level::Info, format!("migration {l}")))
        .collect()
}

/// A `settings.json` with no playbook hooks means a plugin-only install: the
/// binary runs this check but nothing is wired. One row names it and gives the
/// single fix. (With no binary at all, commands/doctor.md prints the message.)
fn plugin_only_rows(env: &Env) -> Vec<Row> {
    if crate::hooks::migration_check::is_wired(&env.settings()) {
        return Vec::new();
    }
    vec![Row::new(
        "-",
        Level::Warn,
        "plugin only: playbook hooks and guards are not wired in settings.json, so they do not run",
    )
    .hint("run /playbook:setup")]
}

/// The two plugins: `playbook` (skills, commands, agents) and `playbook-hooks`
/// (hooks, shim, installer), which are versioned apart. Reads Claude Code's `installed_plugins.json` and
/// stays silent when it is missing, as on a Homebrew-only machine. Neither
/// plugin is required by the seven layers, so none of these rows fails.
fn plugin_split_rows(env: &Env) -> Vec<Row> {
    use crate::common::plugin_state::{
        hooks_plugin_missing, installed_in, listed_in, HOOKS_PLUGIN_KEY,
    };
    let mut rows = Vec::new();
    if hooks_plugin_missing(&env.home) {
        rows.push(
            Row::new(
                "-",
                Level::Warn,
                "playbook-hooks plugin not installed: the worktree hooks are off",
            )
            .hint("run: claude plugin marketplace update pragmatic-engineer && claude plugin install playbook-hooks@pragmatic-engineer"),
        );
    }
    if listed_in(&env.home, HOOKS_PLUGIN_KEY) == Some(true)
        && installed_in(&env.home) == Some(false)
    {
        rows.push(
            Row::new(
                "-",
                Level::Info,
                "playbook plugin not installed: the skills and commands are absent",
            )
            .hint("run: claude plugin install playbook@pragmatic-engineer"),
        );
    }
    rows
}

/// The lines under the table: what to run for each failed layer, or that all
/// required layers pass.
pub fn closing(rows: &[Row]) -> Vec<&'static str> {
    let mut out = Vec::new();
    let failed: Vec<&str> = rows
        .iter()
        .filter(|r| r.level == Level::Fail)
        .map(|r| r.layer)
        .collect();
    if failed.iter().any(|l| ["1", "2", "3", "4", "5"].contains(l)) {
        out.push("Run /playbook:setup to fix layers 1 to 5.");
    }
    if failed.contains(&"6") {
        out.push("/playbook:setup cannot fix layer 6, it does not install the binary. Install it with install.sh or Homebrew and put its directory on PATH.");
    }
    if failed.contains(&"7") {
        out.push(
            "Layer 7 needs a hand edit: remove or fix the named entry in ~/.claude/settings.json.",
        );
    }
    if failed.is_empty() {
        out.push("All required layers pass.");
    }
    out
}

/// The table as text: `LEVEL  message -- hint`, one row per line, then the
/// closing remediation line when a required layer failed.
pub fn render(rows: &[Row]) -> String {
    let mut out = String::new();
    for r in rows {
        out.push_str(&format!("{}  {}", r.level.label(), r.message));
        if let Some(h) = &r.hint {
            out.push_str(&format!(" -- {h}"));
        }
        out.push('\n');
    }
    for line in closing(rows) {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The table as JSON.
pub fn to_json(rows: &[Row]) -> Value {
    json!(rows
        .iter()
        .map(|r| json!({
            "layer": r.layer,
            "level": r.level.label(),
            "message": r.message,
            "hint": r.hint,
        }))
        .collect::<Vec<_>>())
}

/// Whether any required layer failed.
pub fn failed(rows: &[Row]) -> bool {
    rows.iter().any(|r| r.level == Level::Fail)
}
