// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! PreToolUse hook on Edit|Write: when the target path is anchored in the
//! graph-first memory store (`~/.claude/memory/graph.json`), surface the
//! facts that describe it, plus their `depends_on` and `contradicts`
//! neighbours, as `additionalContext` before the edit lands. Emits nothing
//! when there is no match. Never blocks. Ports `hooks/memory-anchors.py`.
//!
//! Performance: this hook fires on every single Edit and Write, so it must
//! not parse the graph on every call. The anchor index is built once per
//! session into a flat, tab-separated file under the session dir, and every
//! lookup after that is a plain scan of that file, no JSON parsing.
//!
//! Staleness: the index is built once, on the first Edit or Write of the
//! session, and never rebuilt within that session. A fact added to the
//! graph mid-session (via `rebuild_memory_graph.rs`, the sole writer of the
//! file this hook reads) will not appear here until the next session starts
//! with a fresh cache. This is deliberate and pinned by the stale cache
//! scenario in `hooks/memory-anchors.test.sh`; see that file's own comment
//! for the full rationale.

use crate::common::payload::Payload;
use crate::common::{emit_pre_context, home_dir, repo_slug, run_with_timeout, session_dir};
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

    let msg = format_message(&relpath, &matches);
    emit_pre_context("PreToolUse", &msg);
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

/// Build the tab-separated anchor index from `graph.json` for the current
/// repo scope. One row per in-scope `anchors` edge, columns anchor, from_id,
/// name, description, neighbours. Any failure (missing or malformed graph)
/// yields an empty index rather than an error, so a stale or absent graph
/// still leaves this hook silent instead of breaking the edit.
fn build_index(idx_path: &Path) {
    let graph_path = memory_graph_path();
    let repo = repo_slug();
    let rows = compute_index_rows(&graph_path, &repo);
    write_index_atomically(idx_path, &rows);
}

fn memory_graph_path() -> PathBuf {
    home_dir().join(".claude").join("memory").join("graph.json")
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
    collect_anchor_rows(&edges, &byid, &inscope, &neigh)
}

fn in_scope(node: &Value, repo: &str) -> bool {
    match node.get("scope").and_then(Value::as_str) {
        Some("global") => true,
        Some("project") => node.get("project").and_then(Value::as_str) == Some(repo),
        _ => false,
    }
}

/// `depends_on`/`contradicts` neighbours, keyed by source node id, in the
/// order their edges appear in `graph.json`.
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
        rows.push(format!("{anchor}\t{from}\t{name}\t{desc}\t{nb}"));
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
