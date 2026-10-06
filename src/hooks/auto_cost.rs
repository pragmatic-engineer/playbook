// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! In `auto`, tracks the session's spend against the budget on every tool call.
//!
//! Spend is the larger of two per-source deltas from a baseline taken the
//! first time the hook runs: the priced transcripts (main plus subagents,
//! each read from its own byte offset) and the statusline telemetry. Values
//! are rounded to integer cents before they are subtracted. The result is
//! persisted as `<session dir>/auto-cost.json`.

use crate::common::atomic::{remove_stale_lock_dir, with_dir_lock, STALE_LOCK_AGE};
use crate::common::mode::{resolve, resolve_for_hook_at, Mode, Source};
use crate::common::payload::Payload;
use crate::common::{home_dir, paths::playbook_root_from, repo_slug, session_dir};
use crate::config;
use crate::usage::claude_code::{one_hour_cache_tokens, token};
use crate::usage::UsageEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{hash_map::DefaultHasher, BTreeMap};
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const INTERVAL_VAR: &str = "PLAYBOOK_AUTO_COST_INTERVAL_MS";
const DEFAULT_INTERVAL_MS: u64 = 2000;
const TELEMETRY_TAIL_BYTES: u64 = 16 * 1024;

#[derive(Default, Serialize, Deserialize)]
struct State {
    effective_cents: i64,
    computed_at_ms: u64,
    transcript: Transcript,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    telemetry: Option<Telemetry>,
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

pub fn run(payload: &Payload) {
    if auto_mode() {
        refresh(payload);
    }
}

fn refresh(payload: &Payload) -> Option<State> {
    let dir = PathBuf::from(session_dir(payload));
    if dir.as_os_str().is_empty() {
        return None;
    }
    let state_path = dir.join("auto-cost.json");
    let now = now_ms();
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn auto_mode() -> bool {
    let env = std::env::var("PLAYBOOK_MODE").ok();
    let from_env = resolve(None, env.as_deref(), None);
    if from_env.source == Source::Env {
        return from_env.mode == Mode::Auto;
    }
    config_mode_is_auto()
}

#[derive(Serialize, Deserialize)]
struct ModeCache {
    cwd: String,
    slug: String,
    stamps: Vec<Option<(u64, u64)>>,
    auto: bool,
}

/// The configured mode, cached across hook processes. An entry is reused
/// while the cwd and the mtime and length of every config tier are unchanged,
/// which saves the `git` call that finds the repo slug. It lives in the temp
/// dir so ask mode never writes under the runtime root.
fn config_mode_is_auto() -> bool {
    let home = home_dir();
    let cwd = std::env::current_dir()
        .map(|dir| dir.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cache_path = mode_cache_path(&home);
    if let Some(cache) = load::<ModeCache>(&cache_path) {
        if cache.cwd == cwd && cache.stamps == config_stamps(&home, &cache.slug) {
            return cache.auto;
        }
    }
    let slug = repo_slug();
    let resolved = resolve_for_hook_at(None, &home, (!slug.is_empty()).then_some(slug.as_str()));
    let auto = resolved.mode == Mode::Auto;
    let stamps = config_stamps(&home, &slug);
    write_atomic(
        &cache_path,
        &ModeCache {
            cwd,
            slug,
            stamps,
            auto,
        },
    );
    auto
}

fn mode_cache_path(home: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    home.hash(&mut hasher);
    std::env::temp_dir().join(format!(
        "playbook-auto-cost-mode-{:x}.json",
        hasher.finish()
    ))
}

fn config_stamps(home: &Path, slug: &str) -> Vec<Option<(u64, u64)>> {
    let root = playbook_root_from(home);
    let mut paths = vec![config::global_config_path(&root)];
    if let Some((owner, repo)) = slug.split_once('/') {
        paths.push(config::org_config_path(&root, owner));
        paths.push(config::repo_config_path(&root, owner, repo));
    }
    paths.iter().map(|path| stamp(path)).collect()
}

fn stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(path).ok()?;
    let nanos = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((nanos.as_nanos() as u64, meta.len()))
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
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    if fs::write(&tmp, bytes).is_err() || fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
        return false;
    }
    true
}
