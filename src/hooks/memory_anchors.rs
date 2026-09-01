// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Two events, one cache. `PreToolUse` on Edit|Write: when the target path
//! is anchored in the graph-first memory store
//! (`~/.claude/memory/memory.graph.json`), surface the facts that describe it, plus
//! their `depends_on` and `contradicts` neighbours, as `additionalContext`
//! before the edit lands. `UserPromptSubmit` (ADR 0008 WU-0): match prompt
//! text and this-session touched files against the same index, injecting
//! the matched facts' BODIES (not just names), deduped per session. Both
//! emit nothing on no match. Neither ever blocks. `PreToolUse` ports
//! `hooks/memory-anchors.py`; `UserPromptSubmit` has no python precedent.
//!
//! Performance: `PreToolUse` fires on every single Edit and Write, so it
//! must not parse the graph on every call. The anchor index is built once
//! per session into a flat, tab-separated file under the session dir, and
//! every lookup after that is a plain scan of that file, no JSON parsing.
//! `UserPromptSubmit` reuses the identical cache: whichever event fires
//! first in a session builds it.
//!
//! Staleness: the index is built once, on the first Edit, Write, or prompt
//! of the session, and never rebuilt within that session. A fact added to
//! the graph mid-session (via `rebuild_memory_graph.rs`, the sole writer of
//! the file this hook reads) will not appear here until the next session
//! starts with a fresh cache. This is deliberate and pinned by the stale
//! cache scenario in `hooks/memory-anchors.test.sh`; see that file's own
//! comment for the full rationale.

use crate::common::payload::Payload;
use crate::common::{
    emit_pre_context, emit_prompt_context, home_dir, repo_slug, run_with_timeout, session_dir,
};
use crate::hooks::memory_signals;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// How long to wait for `git rev-parse --show-toplevel` before giving up.
/// Matches hooks/memory-anchors.py:114's `timeout=5`.
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

pub fn run(payload: &Payload) {
    let dir = session_dir(payload);
    if dir.is_empty() {
        return;
    }

    if payload.field(".hook_event_name") == "UserPromptSubmit" {
        run_prompt(payload, &dir);
        return;
    }

    let raw_path = payload.field(".tool_input.file_path");
    if raw_path.is_empty() {
        return;
    }
    let relpath = repo_relative_path(&raw_path);
    if relpath.is_empty() {
        return;
    }

    let idx_path = Path::new(&dir).join("memory-anchor-index.tsv");
    if !idx_path.exists() {
        build_index(&idx_path);
    }

    let Ok(metadata) = fs::metadata(&idx_path) else {
        return;
    };
    if metadata.len() == 0 {
        return;
    }
    let Ok(contents) = fs::read_to_string(&idx_path) else {
        return;
    };

    let matches = matching_rows(&contents, &relpath);
    if matches.is_empty() {
        return;
    }

    let bump_seen_path = Path::new(&dir).join("anchor-bump-seen.tsv");
    let bump_seen = read_seen(&bump_seen_path);
    let mut newly_bumped = Vec::new();
    for row in &matches {
        let from_id = row.get(1).map(String::as_str).unwrap_or("");
        if from_id.is_empty() || bump_seen.contains(from_id) {
            continue;
        }
        memory_signals::bump_hit(&mem_dir(), from_id);
        newly_bumped.push(from_id.to_string());
    }
    if !newly_bumped.is_empty() {
        append_seen(&bump_seen_path, &newly_bumped);
    }

    let msg = format_message(&relpath, &matches);
    emit_pre_context("PreToolUse", &msg);
}

fn mem_dir() -> PathBuf {
    home_dir().join(".claude").join("memory")
}

/// `UserPromptSubmit` branch: match prompt text and this-session touched
/// files against the same anchor index `PreToolUse` builds, inject the
/// matched facts' BODIES (not their names), deduped per session. Never
/// panics: a missing or unparsable index, or a fact whose `file` has been
/// deleted since the graph was last rebuilt, degrades to skipping that one
/// fact rather than aborting the whole match.
fn run_prompt(payload: &Payload, dir: &str) {
    let idx_path = Path::new(dir).join("memory-anchor-index.tsv");
    if !idx_path.exists() {
        build_index(&idx_path);
    }

    let Ok(metadata) = fs::metadata(&idx_path) else {
        return;
    };
    if metadata.len() == 0 {
        return;
    }
    let Ok(contents) = fs::read_to_string(&idx_path) else {
        return;
    };

    let mut prompt = payload.field(".prompt");
    if prompt.is_empty() {
        // The repo's own live code reads `.prompt` (auto_model_detect.rs),
        // but the official docs report `.user_prompt`; read both rather
        // than guess which name is real. See the ADR 0008 blueprint's
        // resolved open items for the citations behind this.
        prompt = payload.field(".user_prompt");
    }

    let mut matches = prompt_token_matches(&contents, &prompt);
    for touched_abs in touched_paths(dir) {
        let relpath = repo_relative_path(&touched_abs);
        if relpath.is_empty() {
            continue;
        }
        for row in matching_rows(&contents, &relpath) {
            let from_id = row.get(1).cloned().unwrap_or_default();
            if !matches
                .iter()
                .any(|m: &Vec<String>| m.get(1) == Some(&from_id))
            {
                matches.push(row);
            }
        }
    }
    if matches.is_empty() {
        return;
    }

    let seen_path = Path::new(dir).join("prompt-recall-seen.tsv");
    let seen = read_seen(&seen_path);

    let mut newly_seen = Vec::new();
    let mut bodies = Vec::new();
    for row in &matches {
        let from_id = row.get(1).cloned().unwrap_or_default();
        if from_id.is_empty() || seen.contains(&from_id) {
            continue;
        }
        let name = row.get(2).cloned().unwrap_or_default();
        let file = row.get(5).cloned().unwrap_or_default();
        if file.is_empty() {
            continue;
        }
        // A deleted or unreadable fact file is skipped, not fatal: the rest
        // of the matches still get their chance.
        let Some(body) = read_fact_body(&file) else {
            continue;
        };
        bodies.push(format!("### {name}\n{body}"));
        newly_seen.push(from_id);
    }
    if bodies.is_empty() {
        return;
    }

    // Only ids that are genuinely new this session, not every raw match: a
    // fact already injected earlier this session and matched again should
    // not keep re-bumping every prompt for the rest of the session, or the
    // promotion threshold would be trivially easy to cross from repetition
    // within one session rather than genuine cross-session recurrence.
    for id in &newly_seen {
        memory_signals::bump_hit(&mem_dir(), id);
    }

    append_seen(&seen_path, &newly_seen);
    let msg = format!(
        "Recalled from memory, matching this prompt:\n\n{}",
        bodies.join("\n\n")
    );
    emit_prompt_context(&msg);
}

/// Any whitespace-separated prompt word of at least 3 characters, lowercased,
/// found as a substring of a row's name or description (also lowercased).
/// Deliberately crude: a plain substring scan over a corpus this small costs
/// single-digit milliseconds, and no ranking model is proposed (ADR 0008).
/// Deduplicated by from_id, same as `matching_rows`.
fn prompt_token_matches(idx_contents: &str, prompt: &str) -> Vec<Vec<String>> {
    let lower_prompt = prompt.to_lowercase();
    let tokens: Vec<&str> = lower_prompt
        .split(|c: char| c.is_whitespace())
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-'))
        .filter(|w| w.len() >= 3)
        .collect();
    if tokens.is_empty() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    let mut seen_from: HashSet<String> = HashSet::new();
    for line in idx_contents.lines() {
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        let name = cols.get(2).copied().unwrap_or("").to_lowercase();
        let desc = cols.get(3).copied().unwrap_or("").to_lowercase();
        let haystack = format!("{name} {desc}");
        if haystack.trim().is_empty() || !tokens.iter().any(|t| haystack.contains(t)) {
            continue;
        }
        let from_id = cols.get(1).copied().unwrap_or("").to_string();
        if from_id.is_empty() || seen_from.contains(&from_id) {
            continue;
        }
        seen_from.insert(from_id);
        matches.push(cols.iter().map(|s| s.to_string()).collect());
    }
    matches
}

/// Absolute paths touched this session, from `edits.jsonl`, in file order.
/// Same record shape `post_edit_track.rs` writes and `memory_capture.rs`'s
/// `unique_paths_recent_first` reads; duplicated here rather than shared
/// cross-module, since a hook binary keeps its reads local. Never panics: a
/// missing, empty, or unreadable file yields an empty list, and any line
/// that fails to parse or lacks a string `path` is skipped.
fn touched_paths(dir: &str) -> Vec<String> {
    let Ok(contents) = fs::read_to_string(Path::new(dir).join("edits.jsonl")) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return None;
            }
            serde_json::from_str::<Value>(trimmed)
                .ok()?
                .get("path")?
                .as_str()
                .map(str::to_string)
        })
        .collect()
}

/// Fact ids already injected this session, one per line. Never panics: a
/// missing or unreadable marker is treated as "nothing seen yet".
fn read_seen(path: &Path) -> HashSet<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Appends newly injected fact ids to the session's dedup marker. Creates
/// the file on first write; a fresh session gets a fresh `session_dir`
/// (keyed by `session_id`), so no explicit reset is needed.
fn append_seen(path: &Path, ids: &[String]) {
    let mut content = fs::read_to_string(path).unwrap_or_default();
    for id in ids {
        content.push_str(id);
        content.push('\n');
    }
    let _ = fs::write(path, content);
}

/// Reads a fact's markdown body from `~/.claude/memory/<file>` (`file` is
/// relative to that root, per `rebuild_memory_graph.rs`'s node construction).
/// Capped at 16000 chars, matching the legacy `MEMORY.md` fallback cap
/// (`session_init.rs:246`), so one huge fact cannot dominate a turn. `None`
/// on any read failure (deleted, unreadable): the caller skips this one
/// fact rather than treating it as fatal.
fn read_fact_body(file: &str) -> Option<String> {
    let path = home_dir().join(".claude").join("memory").join(file);
    let contents = fs::read_to_string(path).ok()?;
    Some(contents.chars().take(16000).collect())
}

/// Anchors in the graph are repo-relative paths; the tool gives us an
/// (usually absolute) `file_path`, so strip the git worktree root off it. No
/// worktree root, or a `file_path` outside it: fall back to stripping a
/// single leading slash, matching `hooks/memory-anchors.py`.
fn repo_relative_path(raw_path: &str) -> String {
    let root = git_toplevel();
    let prefix = format!("{root}/");
    if !root.is_empty() {
        if let Some(stripped) = raw_path.strip_prefix(&prefix) {
            return stripped.to_string();
        }
    }
    raw_path.strip_prefix('/').unwrap_or(raw_path).to_string()
}

fn git_toplevel() -> String {
    let mut command = Command::new("git");
    command.args(["--no-optional-locks", "rev-parse", "--show-toplevel"]);
    match run_with_timeout(&mut command, GIT_TIMEOUT) {
        Some(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }
        _ => String::new(),
    }
}

/// Match the exact repo-relative path first, then any anchor that is a
/// containing directory of it (an anchor of `src/` matches an edit to
/// `src/deep/b.py`). Deduplicated by the anchoring fact's node id (column
/// 1), keeping the first row for each.
fn matching_rows(idx_contents: &str, relpath: &str) -> Vec<Vec<String>> {
    let mut matches = Vec::new();
    let mut seen_from: HashSet<String> = HashSet::new();
    for line in idx_contents.lines() {
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        let anchor = cols.first().copied().unwrap_or("");
        let dirp = if anchor.ends_with('/') {
            anchor.to_string()
        } else {
            format!("{anchor}/")
        };
        if anchor == relpath || relpath.starts_with(&dirp) {
            let from_id = cols.get(1).copied().unwrap_or("").to_string();
            if seen_from.contains(&from_id) {
                continue;
            }
            seen_from.insert(from_id);
            matches.push(cols.iter().map(|s| s.to_string()).collect());
        }
    }
    matches
}

fn format_message(relpath: &str, matches: &[Vec<String>]) -> String {
    let mut msg = format!("Memory facts anchored to {relpath}:");
    for cols in matches {
        let name = cols.get(2).map(String::as_str).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let desc = cols.get(3).map(String::as_str).unwrap_or("");
        let neigh = cols.get(4).map(String::as_str).unwrap_or("");
        let mut line = format!("- {name}");
        if !desc.is_empty() {
            line = format!("{line}: {desc}");
        }
        if !neigh.is_empty() {
            line = format!("{line} ({neigh})");
        }
        msg.push('\n');
        msg.push_str(&line);
    }
    msg
}

/// Build the tab-separated anchor index from `memory.graph.json` for the current
/// repo scope. One row per in-scope `anchors` edge, columns anchor, from_id,
/// name, description, neighbours, file. The `file` column exists solely for
/// `run_prompt`'s body reads; `PreToolUse`'s `format_message` only reads
/// columns 2 through 4, so this addition is safe for that path. Any failure
/// (missing or malformed graph)
/// yields an empty index rather than an error, so a stale or absent graph
/// still leaves this hook silent instead of breaking the edit.
fn build_index(idx_path: &Path) {
    let graph_path = memory_graph_path();
    let repo = repo_slug();
    let rows = compute_index_rows(&graph_path, &repo);
    write_index_atomically(idx_path, &rows);
}

fn memory_graph_path() -> PathBuf {
    home_dir()
        .join(".claude")
        .join("memory")
        .join("memory.graph.json")
}

fn compute_index_rows(graph_path: &Path, repo: &str) -> Vec<String> {
    let Ok(content) = fs::read_to_string(graph_path) else {
        return Vec::new();
    };
    let Ok(graph) = serde_json::from_str::<Value>(&content) else {
        return Vec::new();
    };

    let nodes = graph
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let edges = graph
        .get("edges")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let byid: HashMap<&str, &Value> = nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str).map(|id| (id, n)))
        .collect();

    let inscope: HashSet<&str> = nodes
        .iter()
        .filter(|n| in_scope(n, repo))
        .filter_map(|n| n.get("id").and_then(Value::as_str))
        .collect();

    let neigh = collect_neighbours(&edges, &byid, &inscope);
    let mut rows = collect_anchor_rows(&edges, &byid, &inscope, &neigh);
    append_unanchored_fact_rows(&nodes, &inscope, &mut rows);
    rows
}

/// A fact with no `anchors` edge still needs a row, or `run_prompt`'s
/// keyword matching could never find it: not every useful fact (a
/// preference, a gotcha with no single owning file) is anchored to code.
/// Anchor column is left empty, which `matching_rows` (both `PreToolUse` and
/// `run_prompt`'s touched-file matching) never matches against a real
/// repo-relative path, so this addition is invisible to those two callers
/// and changes nothing about their existing behaviour.
fn append_unanchored_fact_rows(nodes: &[Value], inscope: &HashSet<&str>, rows: &mut Vec<String>) {
    let already: HashSet<String> = rows
        .iter()
        .filter_map(|r| r.split('\t').nth(1).map(str::to_string))
        .collect();
    for node in nodes {
        let Some(id) = node.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !inscope.contains(id) || already.contains(id) {
            continue;
        }
        if node.get("scope").and_then(Value::as_str) == Some("code") {
            continue;
        }
        let name = node.get("name").and_then(Value::as_str).unwrap_or("");
        let desc = node
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .replace('\n', " ");
        let file = node.get("file").and_then(Value::as_str).unwrap_or("");
        rows.push(format!("\t{id}\t{name}\t{desc}\t\t{file}"));
    }
}

fn in_scope(node: &Value, repo: &str) -> bool {
    match node.get("scope").and_then(Value::as_str) {
        Some("global") => true,
        Some("project") => node.get("project").and_then(Value::as_str) == Some(repo),
        _ => false,
    }
}

/// `depends_on`/`contradicts` neighbours, keyed by source node id, in the
/// order their edges appear in `memory.graph.json`.
fn collect_neighbours<'a>(
    edges: &'a [Value],
    byid: &HashMap<&'a str, &'a Value>,
    inscope: &HashSet<&'a str>,
) -> HashMap<&'a str, Vec<(String, String)>> {
    let mut neigh: HashMap<&str, Vec<(String, String)>> = HashMap::new();
    for edge in edges {
        let relation = edge.get("relation").and_then(Value::as_str).unwrap_or("");
        if relation != "depends_on" && relation != "contradicts" {
            continue;
        }
        let Some(from) = edge.get("from").and_then(Value::as_str) else {
            continue;
        };
        if !inscope.contains(from) {
            continue;
        }
        let to = edge.get("to").and_then(Value::as_str).unwrap_or("");
        let name = byid
            .get(to)
            .and_then(|n| n.get("name"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(to);
        neigh
            .entry(from)
            .or_default()
            .push((relation.to_string(), name.to_string()));
    }
    neigh
}

fn collect_anchor_rows(
    edges: &[Value],
    byid: &HashMap<&str, &Value>,
    inscope: &HashSet<&str>,
    neigh: &HashMap<&str, Vec<(String, String)>>,
) -> Vec<String> {
    let mut rows = Vec::new();
    for edge in edges {
        if edge.get("relation").and_then(Value::as_str) != Some("anchors") {
            continue;
        }
        let Some(from) = edge.get("from").and_then(Value::as_str) else {
            continue;
        };
        if !inscope.contains(from) {
            continue;
        }
        let Some(f_node) = byid.get(from) else {
            continue;
        };
        let Some(to) = edge.get("to").and_then(Value::as_str) else {
            continue;
        };
        let Some(c_node) = byid.get(to) else {
            continue;
        };
        let cfile = c_node.get("file").and_then(Value::as_str).unwrap_or("");
        if cfile.is_empty() {
            continue;
        }
        let anchor = cfile.split('#').next().unwrap_or("");
        let name = f_node.get("name").and_then(Value::as_str).unwrap_or("");
        let desc = f_node
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .replace('\n', " ");
        let nb = neigh
            .get(from)
            .map(|pairs| {
                pairs
                    .iter()
                    .map(|(rel, nm)| format!("{rel}:{nm}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let ffile = f_node.get("file").and_then(Value::as_str).unwrap_or("");
        rows.push(format!("{anchor}\t{from}\t{name}\t{desc}\t{nb}\t{ffile}"));
    }
    rows
}

/// Write `rows` to `idx_path` via a temp file in the same directory plus a
/// rename, so a concurrent reader never observes a partially written index.
/// Mirrors `_build_index`'s `<idx>.tmp.<pid>` plus `os.replace`.
fn write_index_atomically(idx_path: &Path, rows: &[String]) {
    let mut content = String::new();
    for row in rows {
        content.push_str(row);
        content.push('\n');
    }
    let tmp_path = PathBuf::from(format!("{}.tmp.{}", idx_path.display(), std::process::id()));
    if fs::write(&tmp_path, content).is_err() {
        let _ = fs::remove_file(&tmp_path);
        return;
    }
    if fs::rename(&tmp_path, idx_path).is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
}
