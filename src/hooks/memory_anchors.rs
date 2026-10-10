// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Two events, one cache. `PreToolUse` on Edit|Write: when the target path
//! is anchored in the graph-first memory store
//! (`~/.config/playbook/memory/memory.graph.json`), surface the facts that describe it, plus
//! their `depends_on` and `contradicts` neighbours, as `additionalContext`
//! before the edit lands. `UserPromptSubmit` (ADR 0008 WU-0): match prompt
//! text and this-session touched files against the same index, injecting
//! the matched facts' BODIES (not just names), deduped per session. Both
//! emit nothing on no match. Neither ever blocks. `PreToolUse` ports
//! the retired shell original; `UserPromptSubmit` has no python precedent.
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
//! cache scenario in the retired shell test; see that file's own
//! comment for the full rationale.

use crate::common::paths::memory_dir;
use crate::common::payload::Payload;
use crate::common::{emit_pre_context, emit_prompt_context, repo_slug, session_dir};
use crate::hooks::memory_signals;
use crate::hooks::staleness::{self, check_staleness};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Total budget for `run_prompt`'s staleness-check loop across all matched
/// facts; `git::TIMEOUT` only bounds one call, not the chain of them.
const STALENESS_BUDGET: Duration = Duration::from_secs(3);

/// Most facts one prompt may inject.
const MAX_FACTS_PER_PROMPT: usize = 3;

/// Most facts recalled over a whole session, across prompts. Past it the hook
/// goes quiet, so a long session cannot keep adding recall to its history.
const MAX_FACTS_PER_SESSION: usize = 9;

/// Longest fact body injected, in characters. Longer bodies are cut and point
/// at the file, which the model can read when it needs the rest.
const BODY_CAP_CHARS: usize = 1500;

/// Hard ceiling for one prompt's whole recall message.
const PROMPT_RECALL_CAP_CHARS: usize = 5000;

/// Shortest prompt word that can match.
const MIN_TOKEN_CHARS: usize = 3;

/// Words that appear in most prompts and say nothing about a topic. Without
/// this list, "fix the failing test" matched 180 of 261 facts. Sorted, so a
/// binary search finds a word.
const STOPWORDS: [&str; 117] = [
    "about", "above", "add", "after", "again", "all", "also", "and", "any", "are", "because",
    "been", "before", "being", "both", "but", "can", "change", "check", "code", "could", "did",
    "does", "doing", "done", "each", "else", "file", "files", "find", "fix", "for", "from", "get",
    "give", "going", "have", "help", "here", "how", "into", "its", "just", "keep", "let", "like",
    "look", "make", "may", "more", "most", "need", "new", "not", "now", "off", "one", "only",
    "other", "our", "out", "over", "please", "put", "really", "run", "same", "see", "set",
    "should", "show", "some", "still", "such", "take", "tell", "test", "tests", "than", "that",
    "the", "their", "them", "then", "there", "these", "they", "thing", "things", "this", "those",
    "through", "too", "try", "update", "use", "using", "very", "want", "was", "way", "well",
    "were", "what", "when", "where", "which", "while", "who", "why", "will", "with", "work",
    "would", "yes", "you", "your",
];

/// Pure wrapper around the deadline comparison, so the boundary condition is
/// directly testable without depending on real elapsed wall-clock time or a
/// real `git` subprocess in a test.
fn within_staleness_budget(deadline: Instant) -> bool {
    Instant::now() < deadline
}

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
    let root = git_toplevel();
    let relpath = repo_relative_path(&root, &raw_path);
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
        newly_bumped.push(from_id.to_string());
    }
    if !newly_bumped.is_empty() {
        memory_signals::bump_hits(&mem_dir(), &newly_bumped);
        append_seen(&bump_seen_path, &newly_bumped);
    }

    let msg = format_message(&root, &relpath, &matches);
    emit_pre_context("PreToolUse", &msg);
}

fn mem_dir() -> PathBuf {
    memory_dir()
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

    let root = git_toplevel();
    let mut matches = prompt_token_matches(&contents, &prompt);
    let index = AnchorIndex::parse(&contents);
    for touched_abs in touched_paths(dir) {
        let relpath = repo_relative_path(&root, &touched_abs);
        if relpath.is_empty() {
            continue;
        }
        for row in index.matching_rows(&relpath) {
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
    let room = MAX_FACTS_PER_SESSION
        .saturating_sub(seen.len())
        .min(MAX_FACTS_PER_PROMPT);
    if room == 0 {
        return;
    }

    let mut newly_seen = Vec::new();
    let mut entries: Vec<String> = Vec::new();
    let mut used_chars = 0;
    let staleness_deadline = Instant::now() + STALENESS_BUDGET;
    for row in &matches {
        if newly_seen.len() >= room {
            break;
        }
        let from_id = row.get(1).cloned().unwrap_or_default();
        if from_id.is_empty() || seen.contains(&from_id) {
            continue;
        }
        let name = row.get(2).cloned().unwrap_or_default();
        let desc = row.get(3).cloned().unwrap_or_default();
        let file = row.get(5).cloned().unwrap_or_default();
        if file.is_empty() {
            continue;
        }
        // A deleted or unreadable fact file is skipped, not fatal: the rest
        // of the matches still get their chance.
        let Some(body) = read_fact_body(&file) else {
            continue;
        };
        // Past budget: skip the staleness check, not the recall itself.
        let note = if within_staleness_budget(staleness_deadline) {
            let anchor = row.first().cloned().unwrap_or_default();
            staleness_note(&root, &from_id, &anchor)
        } else {
            ""
        };
        let entry = format_recalled_fact(&name, &desc, note, &body, &file);
        if used_chars + entry.chars().count() > PROMPT_RECALL_CAP_CHARS {
            break;
        }
        used_chars += entry.chars().count();
        entries.push(entry);
        newly_seen.push(from_id);
    }
    if entries.is_empty() {
        return;
    }

    // Only ids that are genuinely new this session, not every raw match: a
    // fact already injected earlier this session and matched again should
    // not keep re-bumping every prompt for the rest of the session, or the
    // promotion threshold would be trivially easy to cross from repetition
    // within one session rather than genuine cross-session recurrence.
    memory_signals::bump_hits(&mem_dir(), &newly_seen);

    append_seen(&seen_path, &newly_seen);
    let msg = format!(
        "Recalled from memory, matching this prompt:\n\n{}",
        entries.join("\n\n")
    );
    emit_prompt_context(&msg);
}

/// One recalled fact: its name, its description, then the body, cut at
/// `BODY_CAP_CHARS` with a pointer to the file when it is longer.
fn format_recalled_fact(name: &str, desc: &str, note: &str, body: &str, file: &str) -> String {
    let mut out = format!("### {name}{note}");
    if !desc.is_empty() {
        out.push('\n');
        out.push_str(desc);
    }
    let body = strip_frontmatter(body).trim();
    if body.chars().count() > BODY_CAP_CHARS {
        let head: String = body.chars().take(BODY_CAP_CHARS).collect();
        out.push_str(&format!(
            "\n{head}\n[cut; full fact: ~/.config/playbook/memory/{file}]"
        ));
    } else if !body.is_empty() {
        out.push('\n');
        out.push_str(body);
    }
    out
}

/// `text` without a leading `---` frontmatter block, which the graph already
/// carries as the name and description.
fn strip_frontmatter(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---\n") else {
        return text;
    };
    match rest.find("\n---") {
        Some(end) => rest[end + 4..].trim_start_matches(['\n', '\r']),
        None => text,
    }
}

/// Prompt words that can match: lowercased, trimmed of punctuation, at least
/// `MIN_TOKEN_CHARS` long, and not a stopword. Order and repeats are dropped.
fn prompt_tokens(prompt: &str) -> Vec<String> {
    let lower = prompt.to_lowercase();
    let mut tokens: Vec<String> = Vec::new();
    for word in lower.split(char::is_whitespace) {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-');
        if word.chars().count() < MIN_TOKEN_CHARS
            || STOPWORDS.binary_search(&word).is_ok()
            || tokens.iter().any(|t| t == word)
        {
            continue;
        }
        tokens.push(word.to_string());
    }
    tokens
}

/// Whether `token` matches `text`. A hyphenated token (a fact name pasted
/// from the prompt) matches as a substring. Any other token matches a word of
/// `text` that equals it, or, from 4 characters, one that extends it or that
/// it extends ("test" and "testing"), which skips the accidental hits a plain
/// substring scan made ("add" inside "address", "cli" inside "client").
fn token_matches(token: &str, text: &str) -> bool {
    if token.contains('-') {
        return text.contains(token);
    }
    text.split(|c: char| !c.is_alphanumeric()).any(|word| {
        word == token
            || (token.chars().count() >= 4
                && word.chars().count() >= 4
                && (word.starts_with(token) || token.starts_with(word)))
    })
}

/// Rows whose name or description matches a prompt token, best first: each
/// distinct token that hits counts one, two when it hits the name. Ties keep
/// index order. Deduplicated by from_id, same as `matching_rows`.
fn prompt_token_matches(idx_contents: &str, prompt: &str) -> Vec<Vec<String>> {
    let tokens = prompt_tokens(prompt);
    if tokens.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(usize, Vec<String>)> = Vec::new();
    let mut seen_from: HashSet<String> = HashSet::new();
    for line in idx_contents.lines() {
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        let name = cols.get(2).copied().unwrap_or("").to_lowercase();
        let desc = cols.get(3).copied().unwrap_or("").to_lowercase();
        let score: usize = tokens
            .iter()
            .map(|t| {
                if token_matches(t, &name) {
                    2
                } else {
                    usize::from(token_matches(t, &desc))
                }
            })
            .sum();
        if score == 0 {
            continue;
        }
        let from_id = cols.get(1).copied().unwrap_or("").to_string();
        if from_id.is_empty() || seen_from.contains(&from_id) {
            continue;
        }
        seen_from.insert(from_id);
        scored.push((score, cols.iter().map(|s| s.to_string()).collect()));
    }
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, row)| row).collect()
}

/// Absolute paths touched this session, from `edits.jsonl`, in file order.
/// Same record shape `post_edit_track.rs` writes and `memory_capture.rs`'s
/// `unique_paths_recent_first` reads; duplicated here rather than shared
/// cross-module, since a hook binary keeps its reads local. Repeats are
/// dropped, keeping first-seen order. Never panics: a
/// missing, empty, or unreadable file yields an empty list, and any line
/// that fails to parse or lacks a string `path` is skipped.
fn touched_paths(dir: &str) -> Vec<String> {
    let Ok(contents) = fs::read_to_string(Path::new(dir).join("edits.jsonl")) else {
        return Vec::new();
    };
    let mut seen: HashSet<String> = HashSet::new();
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
        .filter(|path| seen.insert(path.clone()))
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

/// Reads a fact's markdown body from `~/.config/playbook/memory/<file>` (`file` is
/// relative to that root, per `rebuild_memory_graph.rs`'s node construction).
/// Read whole and cut later by `format_recalled_fact`, so one huge fact cannot
/// dominate a turn. `None` on any read failure
/// (deleted, unreadable): the caller skips this one fact rather than
/// treating it as fatal.
fn read_fact_body(file: &str) -> Option<String> {
    let path = memory_dir().join(file);
    let contents = fs::read_to_string(path).ok()?;
    Some(contents)
}

/// Anchors in the graph are repo-relative paths; the tool gives us an
/// (usually absolute) `file_path`, so strip the git worktree root off it. No
/// worktree root, or a `file_path` outside it: fall back to stripping a
/// single leading slash, matching the retired shell original.
fn repo_relative_path(root: &str, raw_path: &str) -> String {
    let prefix = format!("{root}/");
    if !root.is_empty() {
        if let Some(stripped) = raw_path.strip_prefix(&prefix) {
            return stripped.to_string();
        }
    }
    raw_path.strip_prefix('/').unwrap_or(raw_path).to_string()
}

fn git_toplevel() -> String {
    if let Some(top) = std::env::current_dir()
        .ok()
        .and_then(|cwd| crate::common::gitfacts::toplevel(&cwd))
    {
        return top.to_string_lossy().into_owned();
    }
    let args = ["--no-optional-locks", "rev-parse", "--show-toplevel"];
    crate::common::git::trimmed(crate::common::git::run(
        None,
        &args,
        crate::common::git::TIMEOUT,
    ))
    .unwrap_or_default()
}

/// The anchor index parsed once: rows split into columns, plus a map from
/// anchor text to row numbers so a lookup does not rescan every line.
struct AnchorIndex<'a> {
    rows: Vec<Vec<&'a str>>,
    by_anchor: HashMap<&'a str, Vec<usize>>,
}

impl<'a> AnchorIndex<'a> {
    fn parse(idx_contents: &'a str) -> Self {
        let mut rows = Vec::new();
        let mut by_anchor: HashMap<&str, Vec<usize>> = HashMap::new();
        for line in idx_contents.lines().filter(|l| !l.is_empty()) {
            let cols: Vec<&str> = line.split('\t').collect();
            by_anchor
                .entry(cols.first().copied().unwrap_or(""))
                .or_default()
                .push(rows.len());
            rows.push(cols);
        }
        AnchorIndex { rows, by_anchor }
    }

    /// Match the exact repo-relative path first, then any anchor that is a
    /// containing directory of it (an anchor of `src/` matches an edit to
    /// `src/deep/b.py`). Deduplicated by the anchoring fact's node id
    /// (column 1), keeping the first row for each, in index order.
    fn matching_rows(&self, relpath: &str) -> Vec<Vec<String>> {
        let mut hits: Vec<usize> = Vec::new();
        let mut collect = |key: &str| {
            if let Some(found) = self.by_anchor.get(key) {
                hits.extend(found);
            }
        };
        collect(relpath);
        for (i, _) in relpath.match_indices('/') {
            collect(&relpath[..i]);
            collect(&relpath[..=i]);
        }
        hits.sort_unstable();
        hits.dedup();
        let mut seen_from: HashSet<&str> = HashSet::new();
        let mut matches = Vec::new();
        for n in hits {
            let cols = &self.rows[n];
            if seen_from.insert(cols.get(1).copied().unwrap_or("")) {
                matches.push(cols.iter().map(|s| s.to_string()).collect());
            }
        }
        matches
    }
}

fn matching_rows(idx_contents: &str, relpath: &str) -> Vec<Vec<String>> {
    AnchorIndex::parse(idx_contents).matching_rows(relpath)
}

fn format_message(root: &str, relpath: &str, matches: &[Vec<String>]) -> String {
    let mut msg = format!("Memory facts anchored to {relpath}:");
    for cols in matches {
        let name = cols.get(2).map(String::as_str).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let desc = cols.get(3).map(String::as_str).unwrap_or("");
        let neigh = cols.get(4).map(String::as_str).unwrap_or("");
        let from_id = cols.get(1).map(String::as_str).unwrap_or("");
        let anchor = cols.first().map(String::as_str).unwrap_or("");
        let mut line = format!("- {name}");
        if !desc.is_empty() {
            line = format!("{line}: {desc}");
        }
        if !neigh.is_empty() {
            line = format!("{line} ({neigh})");
        }
        line.push_str(staleness_note(root, from_id, anchor));
        msg.push('\n');
        msg.push_str(&line);
    }
    msg
}

/// Appends a soft note if `anchor_relpath` drifted since `from_id` was last
/// touched. An empty `root`, `from_id`, or anchor just skips the marker.
fn staleness_note(root: &str, from_id: &str, anchor_relpath: &str) -> &'static str {
    if root.is_empty() || anchor_relpath.is_empty() || from_id.is_empty() {
        return "";
    }
    let anchor_path = Path::new(root).join(anchor_relpath);
    let result = check_staleness(&mem_dir(), from_id, &anchor_path, &git_last_commit_epoch);
    if result.stale {
        " (may be stale: the anchor changed since this fact was last touched)"
    } else {
        ""
    }
}

/// The default `git_lookup`: the anchor's last commit date, or `None` if
/// untracked, uncommitted, or the clone has no history for that path.
fn git_last_commit_epoch(path: &Path) -> Option<staleness::DateTime> {
    let dir = path.parent()?;
    let path = path.to_string_lossy();
    let args = [
        "--no-optional-locks",
        "log",
        "-1",
        "--format=%ct",
        "--",
        &path,
    ];
    crate::common::git::output(dir, &args)?.parse::<i64>().ok()
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
    memory_dir().join("memory.graph.json")
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
        Some("org") => node.get("project").and_then(Value::as_str) == repo.split('/').next(),
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
    let _ = crate::common::atomic::write_atomic(idx_path, content);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staleness_budget_is_three_seconds() {
        // Arrange / Act / Assert: pins the chosen constant so a future
        // change to it is a deliberate edit, not an accidental drift.
        assert_eq!(STALENESS_BUDGET, Duration::from_secs(3));
    }

    #[test]
    fn within_budget_before_deadline() {
        // Arrange
        let deadline = Instant::now() + Duration::from_secs(10);

        // Act
        let got = within_staleness_budget(deadline);

        // Assert
        assert!(
            got,
            "a deadline well in the future must still be within budget"
        );
    }

    #[test]
    fn not_within_budget_after_deadline() {
        // Arrange: a deadline already in the past, as the staleness loop
        // would see once the aggregate budget has been spent by earlier,
        // slow per-fact checks.
        let deadline = Instant::now() - Duration::from_secs(1);

        // Act
        let got = within_staleness_budget(deadline);

        // Assert
        assert!(
            !got,
            "an expired deadline must report over budget, so the caller skips \
             the next per-fact staleness check instead of starting another \
             potentially slow git call"
        );
    }

    /// A small index shaped like the real store: every fact has a name and a
    /// description, and the descriptions use everyday words.
    const RECALL_INDEX: &str =
        "\tg/a\tavoid-flaky-tests\tUse when a test fails at random in CI\t\ta.md\n\
        \tg/b\tcli-flag-parsing\tUse when you add a flag to the cli\t\tb.md\n\
        \tg/c\tclient-retries\tUse when the http client retries a request\t\tc.md\n\
        \tg/d\taddress-parsing\tUse when you change how an address is parsed\t\td.md\n\
        \tg/e\tcommit-signing\tUse when you commit and the signature fails\t\te.md\n\
        \tg/f\tthis-does-that\tWhat this does and what that does\t\tf.md\n";

    fn matched_names(prompt: &str) -> Vec<String> {
        prompt_token_matches(RECALL_INDEX, prompt)
            .into_iter()
            .map(|row| row[2].clone())
            .collect()
    }

    #[test]
    fn stopwords_are_sorted_and_unique_so_binary_search_is_valid() {
        // Arrange / Act / Assert
        assert!(STOPWORDS.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn generic_prompts_match_almost_nothing() {
        // Arrange: the three prompts that matched 180, 181 and 61 of 261 facts.
        let prompts = [
            "fix the failing test",
            "add a flag to the cli",
            "what does this do",
        ];

        // Act
        let counts: Vec<usize> = prompts.iter().map(|p| matched_names(p).len()).collect();

        // Assert
        assert!(counts.iter().all(|n| *n < 4), "{counts:?}");
    }

    #[test]
    fn a_short_word_matches_a_whole_word_not_a_substring() {
        // Arrange / Act
        let names = matched_names("the cli");

        // Assert: "cli" is in "cli-flag-parsing" but not inside "client-retries".
        assert_eq!(names, vec!["cli-flag-parsing"]);
    }

    #[test]
    fn a_stem_matches_its_longer_form() {
        // Arrange / Act
        let names = matched_names("signing problems");

        // Assert: "signing" hits the name; "problems" hits nothing.
        assert_eq!(names, vec!["commit-signing"]);
    }

    #[test]
    fn matches_are_ranked_by_how_many_prompt_words_hit() {
        // Arrange / Act: "commit" hits e (name) and "signature" hits e (desc).
        let names = matched_names("commit signature http");

        // Assert: e scores 3, c scores 1.
        assert_eq!(names, vec!["commit-signing", "client-retries"]);
    }

    #[test]
    fn a_hyphenated_fact_name_matches_as_a_whole() {
        // Arrange / Act
        let names = matched_names("why does avoid-flaky-tests apply");

        // Assert
        assert_eq!(names, vec!["avoid-flaky-tests"]);
    }

    #[test]
    fn a_recalled_fact_carries_description_and_a_capped_body() {
        // Arrange
        let body = format!("---\nname: x\n---\n{}", "b".repeat(BODY_CAP_CHARS + 50));

        // Act
        let out = format_recalled_fact("x", "Use when y", "", &body, "x.md");

        // Assert
        assert!(out.starts_with("### x\nUse when y\n"));
        assert!(!out.contains("name: x"), "frontmatter should be stripped");
        assert!(out.contains("[cut; full fact: ~/.config/playbook/memory/x.md]"));
        assert!(out.chars().count() < BODY_CAP_CHARS + 200);
    }

    #[test]
    fn a_short_body_is_kept_whole() {
        // Arrange / Act
        let out = format_recalled_fact("x", "", "", "short body", "x.md");

        // Assert
        assert_eq!(out, "### x\nshort body");
    }

    /// The pre-index linear scan, kept as the oracle for equivalence.
    fn reference_matching_rows(idx: &str, relpath: &str) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for line in idx.lines().filter(|l| !l.is_empty()) {
            let cols: Vec<&str> = line.split('\t').collect();
            let anchor = cols.first().copied().unwrap_or("");
            let dirp = if anchor.ends_with('/') {
                anchor.to_string()
            } else {
                format!("{anchor}/")
            };
            if (anchor == relpath || relpath.starts_with(&dirp))
                && seen.insert(cols.get(1).copied().unwrap_or("").to_string())
            {
                out.push(cols.iter().map(|s| s.to_string()).collect());
            }
        }
        out
    }

    const SAMPLE_INDEX: &str = "src/a.rs\tf1\tone\td\t\tf1.md\n\
        src/\tf2\ttwo\td\t\tf2.md\n\
        src\tf3\tthree\td\t\tf3.md\n\
        src/deep/b.py\tf1\tdup\td\t\tf1.md\n\
        \tf4\tunanchored\td\t\tf4.md\n\
        docs//\tf5\tdouble\td\t\tf5.md\n\
        README.md\tf6\tsix\td\t\tf6.md\n";

    #[test]
    fn index_lookup_matches_the_linear_scan_for_every_shape() {
        // Arrange
        let paths = [
            "src/a.rs",
            "src/deep/b.py",
            "src/x",
            "src",
            "docs//x",
            "docs/x",
            "README.md",
            "other/file",
            "/abs/path",
            "",
        ];

        // Act, Assert
        for path in paths {
            assert_eq!(
                matching_rows(SAMPLE_INDEX, path),
                reference_matching_rows(SAMPLE_INDEX, path),
                "path {path:?}"
            );
        }
    }

    #[test]
    fn touched_paths_drops_repeats_and_keeps_first_seen_order() {
        // Arrange
        let dir = crate::common::test_support::scratch_dir("anchors-touched");
        fs::create_dir_all(&dir).unwrap();
        let lines = ["/r/b.rs", "/r/a.rs", "/r/b.rs", "/r/a.rs", "/r/c.rs"]
            .map(|p| format!("{{\"path\":\"{p}\"}}\n"))
            .concat();
        fs::write(dir.join("edits.jsonl"), lines).unwrap();

        // Act
        let got = touched_paths(dir.to_str().unwrap());

        // Assert
        assert_eq!(got, ["/r/b.rs", "/r/a.rs", "/r/c.rs"]);
    }

    #[test]
    fn many_lookups_against_a_large_index_stay_fast() {
        // Arrange
        let idx: String = (0..2000)
            .map(|i| format!("src/f{i}.rs\tid{i}\tname\tdesc\t\tf{i}.md\n"))
            .collect();
        let index = AnchorIndex::parse(&idx);
        let started = Instant::now();

        // Act
        let hits: usize = (0..20_000)
            .map(|i| index.matching_rows(&format!("src/f{}.rs", i % 4000)).len())
            .sum();

        // Assert: loose bound, the per-row rescan took several seconds here.
        assert_eq!(hits, 10_000);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
