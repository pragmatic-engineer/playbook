// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! In `auto`, tracks the session's spend against the budget on every tool call.
//!
//! Spend is the larger of two per-source deltas from a baseline taken the
//! first time the hook runs: the priced transcripts (main plus subagents,
//! each read from its own byte offset) and the statusline telemetry. Values
//! are rounded to integer cents before they are subtracted. The result is
//! persisted as `<session dir>/auto-cost.json`.
//!
//! At the warn tier the hook adds one note, and at the cap it denies every
//! tool outside a short safe list. When the spend cannot be read, including a
//! transcript with unpriced models and no telemetry, it fails closed the same
//! way. Under the cap it also denies the model changing its
//! own guard: the `mode`, `auto.*` and `fix.*` settings and the config files.

use crate::common::atomic::{remove_stale_lock_dir, with_dir_lock, STALE_LOCK_AGE};
use crate::common::mode::{hook_slug, resolve, resolve_for_hook_at, Mode, Source};
use crate::common::paths::playbook_root_from;
use crate::common::payload::Payload;
use crate::common::shell::{program_index, program_name, simple_commands};
use crate::common::{emit_pre_context, emit_pre_deny, home_dir, session_dir, session_id};
use crate::config;
use crate::usage::claude_code::{one_hour_cache_tokens, token};
use crate::usage::UsageEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

const INTERVAL_VAR: &str = "PLAYBOOK_AUTO_COST_INTERVAL_MS";
const DEFAULT_INTERVAL_MS: u64 = 2000;
const STATE_FILE: &str = "auto-cost.json";
const TELEMETRY_TAIL_BYTES: u64 = 16 * 1024;

#[derive(Default, Serialize, Deserialize)]
struct State {
    effective_cents: i64,
    computed_at_ms: u64,
    transcript: Transcript,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    telemetry: Option<Telemetry>,
}

impl State {
    /// Whether the spend can be trusted: a transcript with every message
    /// priced, or the statusline telemetry. A model missing from the price
    /// table counts as free, so without telemetry that cost is a guess.
    fn is_readable(&self) -> bool {
        let files = &self.transcript.files;
        let priced = !files.is_empty() && files.values().all(|file| !file.unpriced);
        priced || self.telemetry.is_some()
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Transcript {
    files: BTreeMap<String, FileCost>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct FileCost {
    baseline_cents: i64,
    usd: f64,
    offset: u64,
    inode: u64,
    /// The last message read and the cost already counted for it, so the
    /// rest of a message streamed across two reads is not counted twice.
    #[serde(default)]
    last_id: String,
    #[serde(default)]
    last_usd: f64,
    /// A message used a model missing from the price table, so `usd` is a
    /// lower bound.
    #[serde(default)]
    unpriced: bool,
}

impl FileCost {
    fn restart(&mut self) {
        self.usd = 0.0;
        self.offset = 0;
        self.last_id.clear();
        self.last_usd = 0.0;
        self.unpriced = false;
    }
}

#[derive(Serialize, Deserialize)]
struct Telemetry {
    baseline_cents: i64,
    cents: i64,
}

const DEFAULT_BUDGET_CENTS: i64 = 500;
const DEFAULT_WARN_PCT: u64 = 70;
const SAFE_TOOLS: [&str; 3] = ["Read", "Grep", "Glob"];
const SAFE_COMMANDS: [&str; 4] = [
    "git status",
    "git diff",
    "playbook mode status",
    "playbook mode status --json",
];
const MODE_ESCAPE: &str = "playbook mode ask";

pub fn run(payload: &Payload) {
    let env = std::env::var("PLAYBOOK_MODE").ok();
    let from_env = resolve(None, env.as_deref(), None);
    if from_env.source == Source::Env && from_env.mode == Mode::Ask {
        return;
    }
    let home = home_dir();
    let slug = hook_slug(&home);
    let slug = slug.as_deref();
    let root = playbook_root_from(&home);
    let is_auto = resolve_for_hook_at(env.as_deref(), &home, slug).mode == Mode::Auto;
    if is_auto || config_broke_mid_session(payload, &root, &home, slug) {
        enforce(payload, &Limits::read(&home, slug), &root);
    }
}

/// An unreadable config resolves to `ask`, which would switch the cap off for
/// a session it already tracks, so such a session stays guarded.
fn config_broke_mid_session(
    payload: &Payload,
    root: &Path,
    home: &Path,
    slug: Option<&str>,
) -> bool {
    let sid = session_id(payload);
    !sid.is_empty()
        && config::resolve("mode", home, slug).is_err()
        && root.join("runtime").join(sid).join(STATE_FILE).is_file()
}

/// The cap and the warn tier in integer cents.
struct Limits {
    cap: i64,
    warn: i64,
    pct: u64,
    budget_is_valid: bool,
}

impl Limits {
    fn read(home: &Path, slug: Option<&str>) -> Self {
        let value = |key: &str| {
            config::resolve(key, home, slug)
                .ok()
                .map(|(value, _)| value)
        };
        let budget = value("auto.budgetUsd")
            .and_then(|value| value.as_f64())
            .map(to_cents)
            .filter(|cents| *cents > 0);
        let pct = value("auto.warnPct")
            .and_then(|value| value.as_u64())
            .filter(|pct| (1..=100).contains(pct))
            .unwrap_or(DEFAULT_WARN_PCT);
        let cap = budget.unwrap_or(DEFAULT_BUDGET_CENTS);
        Limits {
            cap,
            warn: (cap as f64 * pct as f64 / 100.0).round() as i64,
            pct,
            budget_is_valid: budget.is_some(),
        }
    }
}

fn enforce(payload: &Payload, limits: &Limits, root: &Path) {
    let state = refresh(payload).filter(State::is_readable);
    let dir = PathBuf::from(session_dir(payload));
    let park_note = dir.join("park-note.md");
    let call = ToolCall::of(payload);
    let Some(state) = state else {
        if !call.is_safe(None) && !call.is_mode_escape() {
            emit_pre_deny(UNREADABLE_REASON);
        }
        return;
    };
    if state.effective_cents >= limits.cap {
        if !call.is_safe(Some(&park_note)) {
            emit_pre_deny(&budget_reached_reason(&park_note));
        }
        return;
    }
    if call.changes_the_guard(root) {
        emit_pre_deny(SELF_PROTECTION_REASON);
        return;
    }
    let mut notes = Vec::new();
    if !limits.budget_is_valid && claim(&dir.join("budget-note")) {
        notes.push(INVALID_BUDGET_NOTE.to_string());
    }
    if state.effective_cents >= limits.warn && claim(&dir.join("warned")) {
        notes.push(warn_note(limits));
    }
    if !notes.is_empty() {
        emit_pre_context("PreToolUse", &notes.join(" "));
    }
}

const UNREADABLE_REASON: &str = "cost unreadable: the session spend can't be checked. Stop here and report to the user. Reading files still works, and `playbook mode ask` is allowed.";
const SELF_PROTECTION_REASON: &str =
    "Auto mode settings are locked while auto is on. Ask the user to run this command themselves.";
const INVALID_BUDGET_NOTE: &str = "auto.budgetUsd must be a number of at least 0.01, so the default cap of $5 applies. Fix it with `playbook config set auto.budgetUsd <usd>`.";

fn budget_reached_reason(park_note: &Path) -> String {
    format!(
        "budget reached: write a park note to {} describing where you stopped, then report to the user. Only the user can raise the cap or start a new session.",
        park_note.display()
    )
}

fn warn_note(limits: &Limits) -> String {
    format!(
        "AUTO MODE budget: {}% of the ${:.2} cap is spent. Finish the current step, then report to the user before the cap parks the session.",
        limits.pct,
        limits.cap as f64 / 100.0
    )
}

/// Creates `marker` exclusively, so of concurrent hook processes only one wins.
fn claim(marker: &Path) -> bool {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker)
        .is_ok()
}

struct ToolCall {
    tool: String,
    command: String,
    file_path: String,
}

impl ToolCall {
    fn of(payload: &Payload) -> Self {
        ToolCall {
            tool: payload.field(".tool_name"),
            command: payload.field(".tool_input.command"),
            file_path: payload.field(".tool_input.file_path"),
        }
    }

    /// Reading, the exact read-only commands, and the model's own park note
    /// when the caller has one to allow.
    fn is_safe(&self, park_note: Option<&Path>) -> bool {
        match self.tool.as_str() {
            tool if SAFE_TOOLS.contains(&tool) => true,
            "Bash" => SAFE_COMMANDS.contains(&self.command.as_str()),
            "Write" => park_note.is_some_and(|note| is_same_path(Path::new(&self.file_path), note)),
            _ => false,
        }
    }

    fn is_mode_escape(&self) -> bool {
        self.tool == "Bash" && self.command == MODE_ESCAPE
    }

    fn changes_the_guard(&self, root: &Path) -> bool {
        match self.tool.as_str() {
            "Bash" => is_guarded_command(&self.command),
            "Write" | "Edit" => is_config_file(Path::new(&self.file_path), root),
            _ => false,
        }
    }
}

/// True when some simple command in `command` runs `playbook` to switch the
/// mode, to set `mode`, `auto.*` or `fix.*`, to import a config document, or
/// names the config database. Quoted text is data, so prose
/// that merely mentions such a command is left alone.
///
/// This is a guardrail against the model drifting, not a security boundary.
/// It does not look into `bash -c`, `eval` or a script fed to a shell through
/// a heredoc, nor into `$(...)` inside double quotes, and it cannot see what
/// variables, aliases or functions expand to.
fn is_guarded_command(command: &str) -> bool {
    simple_commands(command)
        .iter()
        .any(|words| is_guarded_invocation(words))
}

fn is_guarded_invocation(words: &[String]) -> bool {
    // Any program pointed at the config database can change guarded settings.
    if words.iter().any(|word| word.contains("playbook.db")) {
        return true;
    }
    let Some(start) = program_index(words) else {
        return false;
    };
    let mut args = &words[start + 1..];
    match program_name(&words[start]).as_str() {
        "playbook" => {}
        "cargo" => match cargo_run_args(args) {
            Some(rest) => args = rest,
            None => return false,
        },
        _ => return false,
    }
    match args {
        [mode, level, ..] if mode == "mode" => level == "ask" || level == "auto",
        [config, import, ..] if config == "config" && import == "import" => true,
        [config, set, rest @ ..] if config == "config" && set == "set" => rest
            .iter()
            .find(|word| !word.starts_with('-'))
            .is_some_and(|key| {
                key == "mode" || key.starts_with("auto.") || key.starts_with("fix.")
            }),
        _ => false,
    }
}

/// What follows `--` in `cargo run ... -- <args>`, which a workspace binary
/// receives as its own arguments.
fn cargo_run_args(args: &[String]) -> Option<&[String]> {
    let subcommand = args.iter().find(|word| !word.starts_with(['-', '+']))?;
    if subcommand != "run" && subcommand != "r" {
        return None;
    }
    let dashes = args.iter().position(|word| word == "--")?;
    Some(&args[dashes + 1..])
}

/// The three config tier files under `root`, with the owner and repo as
/// wildcards. `.` and `..` are resolved lexically, never on disk, and case is
/// ignored because the default macOS and Windows filesystems ignore it.
fn is_config_file(path: &Path, root: &Path) -> bool {
    let lowercase = |path: &Path| PathBuf::from(path.to_string_lossy().to_lowercase());
    let (path, root) = (lowercase(path), lowercase(root));
    let mut normal = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            part => normal.push(part),
        }
    }
    let Ok(rel) = normal.strip_prefix(&root) else {
        return false;
    };
    let parts: Vec<&str> = rel
        .components()
        .map(|part| part.as_os_str().to_str().unwrap_or_default())
        .collect();
    matches!(
        parts.as_slice(),
        ["config.json" | "playbook.db" | "playbook.db-wal" | "playbook.db-shm"]
            | ["orgs", _, "config.json"]
            | ["repos", _, _, ".config", "config.json"]
    )
}

/// Compares paths lexically, ignoring `.` segments. A `..` never matches, and
/// the path is not resolved on disk, so a symlinked home still compares equal.
fn is_same_path(path: &Path, expected: &Path) -> bool {
    !path.components().any(|part| part == Component::ParentDir)
        && path.components().eq(expected.components())
}

fn refresh(payload: &Payload) -> Option<State> {
    let dir = PathBuf::from(session_dir(payload));
    if dir.as_os_str().is_empty() {
        return None;
    }
    let state_path = dir.join(STATE_FILE);
    let now = crate::common::time::now_ms();
    if let Some(state) = load::<State>(&state_path) {
        if now.saturating_sub(state.computed_at_ms) < interval_ms() {
            return Some(state);
        }
    }

    let lock = dir.join("auto-cost.lock");
    remove_stale_lock_dir(&lock, STALE_LOCK_AGE);
    let (acquired, state) = with_dir_lock(&lock, 50, Duration::from_millis(10), || {
        let loaded = load::<State>(&state_path);
        let has_baseline = loaded.is_some();
        let mut state = loaded.unwrap_or_default();
        recompute(&mut state, payload, &dir, now);
        let persisted = write_atomic(&state_path, &state);
        // A baseline that cannot be stored would be retaken on every call, so
        // the spend would never grow past it.
        (persisted || has_baseline).then_some(state)
    });
    if acquired {
        let _ = fs::remove_dir(&lock);
    }
    state
}

fn recompute(state: &mut State, payload: &Payload, dir: &Path, now: u64) {
    let first_call = state.computed_at_ms == 0;
    for path in transcript_files(payload) {
        advance_file(&mut state.transcript.files, &path, first_call);
    }
    if let Some(cents) = telemetry_cents(&dir.join("telemetry.jsonl")) {
        state
            .telemetry
            .get_or_insert(Telemetry {
                baseline_cents: cents,
                cents,
            })
            .cents = cents;
    }
    state.effective_cents = effective_cents(state);
    state.computed_at_ms = now;
}

fn effective_cents(state: &State) -> i64 {
    let files = &state.transcript.files;
    let transcript = (!files.is_empty()).then(|| {
        files
            .values()
            .map(|f| to_cents(f.usd) - f.baseline_cents)
            .sum::<i64>()
    });
    let telemetry = state.telemetry.as_ref().map(|t| t.cents - t.baseline_cents);
    transcript.into_iter().chain(telemetry).max().unwrap_or(0)
}

fn to_cents(usd: f64) -> i64 {
    (usd * 100.0).round() as i64
}

fn transcript_files(payload: &Payload) -> Vec<PathBuf> {
    let main = payload.field(".transcript_path");
    if main.is_empty() {
        return Vec::new();
    }
    let mut files = vec![PathBuf::from(&main)];
    if let Some(stem) = main.strip_suffix(".jsonl") {
        let subagents = fs::read_dir(Path::new(stem).join("subagents"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| is_agent_transcript(path));
        files.extend(subagents);
    }
    files
}

fn is_agent_transcript(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("agent-") && name.ends_with(".jsonl"))
}

/// Reads the complete lines appended to `path` since its stored offset. A file
/// that shrank or was replaced restarts from zero but keeps its baseline.
fn advance_file(files: &mut BTreeMap<String, FileCost>, path: &Path, first_call: bool) {
    let Ok(mut file) = File::open(path) else {
        return;
    };
    let Ok(meta) = file.metadata() else {
        return;
    };
    let key = path.to_string_lossy().into_owned();
    let is_new = !files.contains_key(&key);
    let mut entry = files.get(&key).cloned().unwrap_or_default();
    let inode = inode_of(&meta);
    if meta.len() < entry.offset || (!is_new && entry.inode != inode) {
        entry.restart();
    }
    entry.inode = inode;

    let mut buf = Vec::new();
    if file.seek(SeekFrom::Start(entry.offset)).is_err() || file.read_to_end(&mut buf).is_err() {
        return;
    }
    let complete = buf
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(&buf[..0], |last| &buf[..=last]);
    let batch = read_batch(complete, &entry);
    entry.usd += batch.usd;
    entry.unpriced |= batch.unpriced;
    if let Some((id, usd)) = batch.last {
        entry.last_id = id;
        entry.last_usd = usd;
    }
    entry.offset += complete.len() as u64;
    if is_new && first_call {
        entry.baseline_cents = to_cents(entry.usd);
    }
    files.insert(key, entry);
}

#[cfg(unix)]
fn inode_of(meta: &fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::ino(meta)
}

#[cfg(not(unix))]
fn inode_of(_meta: &fs::Metadata) -> u64 {
    0
}

/// What one read added to a file: the new cost, whether any of it is
/// unpriced, and the last message seen with the cost now counted for it.
struct Batch {
    usd: f64,
    unpriced: bool,
    last: Option<(String, f64)>,
}

/// A streamed message repeats its usage on several lines, so only the largest
/// cost per message id counts. When the first message of the read continues
/// the last one of the previous read, only the growth beyond `prior.last_usd`
/// is added.
fn read_batch(lines: &[u8], prior: &FileCost) -> Batch {
    let mut by_id: BTreeMap<String, f64> = BTreeMap::new();
    let mut last_id = None;
    let mut unpriced = false;
    for line in lines.split(|&b| b == b'\n') {
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let Some(message) = message_cost(&value) else {
            continue;
        };
        unpriced |= message.unpriced;
        let slot = by_id.entry(message.id.clone()).or_insert(message.usd);
        *slot = slot.max(message.usd);
        last_id = Some(message.id);
    }
    let usd = by_id
        .iter()
        .map(|(id, usd)| match *id == prior.last_id {
            true => (usd - prior.last_usd).max(0.0),
            false => *usd,
        })
        .sum();
    let last = last_id.map(|id| {
        let counted = by_id[&id];
        let counted = match id == prior.last_id {
            true => counted.max(prior.last_usd),
            false => counted,
        };
        (id, counted)
    });
    Batch {
        usd,
        unpriced,
        last,
    }
}

struct MessageCost {
    id: String,
    usd: f64,
    unpriced: bool,
}

fn message_cost(line: &Value) -> Option<MessageCost> {
    if line.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let message = line.get("message")?;
    let usage = message.get("usage")?;
    let mut event = UsageEvent {
        model: message.get("model")?.as_str()?.to_string(),
        input_tokens: token(usage, "input_tokens"),
        output_tokens: token(usage, "output_tokens"),
        cache_creation_tokens: token(usage, "cache_creation_input_tokens"),
        cache_creation_1h_tokens: one_hour_cache_tokens(usage),
        cache_read_tokens: token(usage, "cache_read_input_tokens"),
        ..UsageEvent::default()
    };
    event.apply_pricing();
    Some(MessageCost {
        id: message.get("id")?.as_str()?.to_string(),
        usd: event.cost_usd,
        unpriced: event.unpriced,
    })
}

/// The `cost_usd` of the newest telemetry line, read from the file's tail.
fn telemetry_cents(path: &Path) -> Option<i64> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TELEMETRY_TAIL_BYTES)))
        .ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    buf.split(|&b| b == b'\n')
        .rev()
        .find_map(|line| {
            serde_json::from_slice::<Value>(line)
                .ok()?
                .get("cost_usd")?
                .as_f64()
        })
        .map(to_cents)
}

fn interval_ms() -> u64 {
    std::env::var(INTERVAL_VAR)
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
        .unwrap_or(DEFAULT_INTERVAL_MS)
}

fn load<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Writes to a sibling temp file and renames it over `path`, so a concurrent
/// reader sees the old or the new file, never a torn one. Returns whether the
/// file was stored.
fn write_atomic(path: &Path, value: &impl Serialize) -> bool {
    let Ok(bytes) = serde_json::to_vec(value) else {
        return false;
    };
    crate::common::atomic::write_atomic(path, bytes).is_ok()
}
