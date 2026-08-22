// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! PostToolUse hook: rebuild `~/.claude/memory/graph.json` after any
//! fact-file save. Ports `hooks/rebuild-memory-graph.py`. No-op unless the
//! edited file is inside `~/.claude/memory`. Walks the whole memory tree
//! (not incremental), writes atomically (temp file plus rename), and emits
//! nothing on stdout.
//!
//! `memory-anchors.rs` (the sole reader of the file this hook writes) must
//! change in lockstep with this one; they ship in the same commit on purpose.

use crate::common::home_dir;
use crate::common::payload::Payload;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub fn run(payload: &Payload) {
    if should_skip(payload) {
        return;
    }
    rebuild();
}

/// Mirror the bash/python guard: skip the rebuild unless the edited file's
/// path (after expanding a leading `~`) is inside `MEMORY_DIR`.
///
/// Divergence from python: `hooks/rebuild-memory-graph.py`'s `_should_skip`
/// reads its own raw stdin (or `HOOK_INPUT`) and treats completely empty
/// input as a signal to always rebuild, distinct from a non-empty payload
/// that merely lacks `tool_input.file_path` (which it skips). `main.rs`
/// already consumes that raw input before calling `Payload::parse`, and only
/// the parsed `Payload` reaches this hook: an empty raw string, a bare `{}`,
/// and malformed JSON all collapse to the identical empty object, so those
/// three cases are indistinguishable from inside this function. This port
/// always skips when `file_path` is missing or outside `MEMORY_DIR`, which
/// matches 2 of python's 3 branches (the non-empty-but-fieldless case and
/// the malformed-JSON case) and never triggers an unwanted full-tree
/// rebuild; only the "truly no input at all" manual-invocation case behaves
/// differently, and it is not exercised by rebuild-memory-graph.test.sh.
fn should_skip(payload: &Payload) -> bool {
    let raw_path = payload.field(".tool_input.file_path");
    let file_path = expand_tilde(&raw_path);
    if file_path.is_empty() {
        return true;
    }
    let mem_dir = memory_dir().to_string_lossy().into_owned();
    !file_path.starts_with(&mem_dir)
}

fn expand_tilde(path: &str) -> String {
    match path.strip_prefix('~') {
        Some(rest) => format!("{}{rest}", home_dir().to_string_lossy()),
        None => path.to_string(),
    }
}

fn memory_dir() -> PathBuf {
    home_dir().join(".claude").join("memory")
}

// --- Frontmatter parsing (hand-rolled YAML subset, no yaml crate) ---------

/// A single top-level frontmatter value: a bare scalar, a block or inline
/// list, or a dict of sub-keys (each of which is itself a scalar or a
/// list). Mirrors the three shapes `hooks/rebuild-memory-graph.py:
/// parse_frontmatter` can produce for a python dict value.
#[derive(Debug, Clone)]
enum TopValue {
    Scalar(String),
    List(Vec<String>),
    Dict(HashMap<String, SubValue>),
}

/// All top-level frontmatter values, keyed by name. Kept in one map, rather
/// than one map per shape, so a later top-level redeclaration of a key
/// evicts whatever shape the earlier declaration held, regardless of shape:
/// python's `parse_frontmatter` keeps a single `result` dict and gets this
/// for free, since a later `result[current_key] = ...` simply overwrites.
#[derive(Debug, Default, Clone)]
struct Frontmatter {
    values: HashMap<String, TopValue>,
}

impl Frontmatter {
    fn scalar(&self, key: &str) -> Option<&String> {
        match self.values.get(key) {
            Some(TopValue::Scalar(s)) => Some(s),
            _ => None,
        }
    }

    fn list(&self, key: &str) -> Option<&Vec<String>> {
        match self.values.get(key) {
            Some(TopValue::List(l)) => Some(l),
            _ => None,
        }
    }

    fn dict(&self, key: &str) -> Option<&HashMap<String, SubValue>> {
        match self.values.get(key) {
            Some(TopValue::Dict(d)) => Some(d),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
enum SubValue {
    Scalar(String),
    List(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    None,
    List,
    Dict,
}

/// Accumulator for the frontmatter state machine, one instance per file.
/// Fields mirror the nonlocal variables `parse_frontmatter`'s python
/// closures capture: `current_key`/`current_kind` track which top-level key
/// is being built, `buf_list`/`buf_dict` hold that key's in-progress value,
/// and `pending_key`/`pending_list` hold a dict sub-key whose value is a
/// block list still being read.
struct ParserState {
    result: Frontmatter,
    current_key: Option<String>,
    current_kind: Kind,
    buf_list: Vec<String>,
    buf_dict: HashMap<String, SubValue>,
    pending_key: Option<String>,
    pending_list: Vec<String>,
}

impl ParserState {
    fn new() -> Self {
        Self {
            result: Frontmatter::default(),
            current_key: None,
            current_kind: Kind::None,
            buf_list: Vec::new(),
            buf_dict: HashMap::new(),
            pending_key: None,
            pending_list: Vec::new(),
        }
    }

    fn flush_pending(&mut self) {
        if let Some(key) = self.pending_key.take() {
            self.buf_dict
                .insert(key, SubValue::List(std::mem::take(&mut self.pending_list)));
        }
    }

    /// Finalize whatever is buffered for `current_key`. A key that was
    /// opened but never accumulated a list or dict value (kind stayed
    /// `None`) is silently dropped, matching python: `flush()` only writes
    /// to `result` in the list/dict branches. The insert below evicts
    /// whatever value (of any shape) an earlier declaration of the same key
    /// left behind, matching python's single-dict overwrite semantics.
    fn flush(&mut self) {
        let Some(key) = self.current_key.take() else {
            return;
        };
        self.flush_pending();
        match self.current_kind {
            Kind::List => {
                self.result
                    .values
                    .insert(key, TopValue::List(std::mem::take(&mut self.buf_list)));
            }
            Kind::Dict => {
                self.result
                    .values
                    .insert(key, TopValue::Dict(std::mem::take(&mut self.buf_dict)));
            }
            Kind::None => {}
        }
        self.current_kind = Kind::None;
    }
}

/// Parse YAML frontmatter between `---` delimiters. Absent, unclosed, or
/// otherwise malformed frontmatter all yield an empty `Frontmatter`, never
/// an error: a fact file with no frontmatter still gets a node, using the
/// filename-derived defaults `rebuild()` applies below.
fn parse_frontmatter(content: &str) -> Frontmatter {
    let mut state = ParserState::new();
    let Some(fm) = extract_frontmatter_block(content) else {
        return state.result;
    };
    let fm = normalize_line_endings(&fm);

    for line in fm.split('\n') {
        if let Some((key, rest)) = match_top_level(line) {
            state.flush();
            state.current_key = Some(key.to_string());
            let val = rest.trim();
            if !val.is_empty() {
                let value = if val.starts_with('[') && val.ends_with(']') {
                    TopValue::List(parse_inline_list(val))
                } else {
                    TopValue::Scalar(val.to_string())
                };
                state.result.values.insert(key.to_string(), value);
                state.current_key = None;
            }
        } else if state.current_key.is_some() && state.pending_key.is_some() && is_block_item(line)
        {
            state.pending_list.push(extract_block_item(line));
        } else if state.current_key.is_some() && is_block_item(line) {
            state.current_kind = Kind::List;
            state.buf_list.push(extract_block_item(line));
        } else if state.current_key.is_some() && is_sub_kv_line(line) {
            if let Some((sub_key, sub_val)) = match_sub_kv(line) {
                state.flush_pending();
                state.current_kind = Kind::Dict;
                let sub_val = sub_val.trim();
                if sub_val.starts_with('[') && sub_val.ends_with(']') {
                    state.buf_dict.insert(
                        sub_key.to_string(),
                        SubValue::List(parse_inline_list(sub_val)),
                    );
                } else if !sub_val.is_empty() {
                    state
                        .buf_dict
                        .insert(sub_key.to_string(), SubValue::Scalar(sub_val.to_string()));
                } else {
                    state.pending_key = Some(sub_key.to_string());
                    state.pending_list = Vec::new();
                }
            }
        }
    }
    state.flush();
    state.result
}

/// Return the text strictly between a `---` opening line and the first
/// `---` closing line, both required to be a full line (only trailing
/// spaces/tabs allowed) and the closing line must itself be followed by
/// another line, matching python's `^---[ \t]*\n(.*?)\n---[ \t]*\n` (DOTALL,
/// anchored at the start of `content`). Returns `None` when the delimiters
/// are missing or unclosed.
fn extract_frontmatter_block(content: &str) -> Option<String> {
    let mut lines = content.split('\n').peekable();
    let first = lines.next()?;
    if !is_delimiter_line(first) {
        return None;
    }
    let mut fm_lines = Vec::new();
    for line in lines.by_ref() {
        if is_delimiter_line(line) {
            return if lines.peek().is_some() {
                Some(fm_lines.join("\n"))
            } else {
                None
            };
        }
        fm_lines.push(line);
    }
    None
}

/// Normalize `\r\n` and a lone `\r` to `\n` before the frontmatter body is
/// split into lines, so a stray carriage return cannot corrupt a scalar
/// value. Mirrors python's `str.splitlines()`, which treats a bare `\r` as
/// a line break, closely enough for this parser: the handful of exotic
/// Unicode line separators `splitlines()` also recognizes (`\v`, `\f`,
/// `\x1c`-`\x1e`, `\x85`, `U+2028`, `U+2029`) are not chased here, since
/// carriage-return coverage is the only gap that shows up in real fact
/// files. Applied only to the already-extracted frontmatter body, not to
/// the raw file content, so a whole-file-CRLF fact still fails to match the
/// opening delimiter the same way in both implementations.
fn normalize_line_endings(fm: &str) -> String {
    fm.replace("\r\n", "\n").replace('\r', "\n")
}

fn is_delimiter_line(line: &str) -> bool {
    line.starts_with("---") && line[3..].bytes().all(|b| b == b' ' || b == b'\t')
}

fn leading_ws_len(line: &str) -> usize {
    line.bytes()
        .take_while(|&b| b == b' ' || b == b'\t')
        .count()
}

/// `^([A-Za-z_]\w*):\s*(.*)`, only ever attempted against an unindented
/// line: the leading char class already excludes whitespace, so python's
/// extra `not line[0].isspace()` guard is redundant and not reproduced.
fn match_top_level(line: &str) -> Option<(&str, &str)> {
    match_key_colon_rest(line, 0)
}

/// `^\s{2,}- `: at least two leading spaces/tabs, then a literal `- `.
fn is_block_item(line: &str) -> bool {
    let n = leading_ws_len(line);
    n >= 2 && line[n..].starts_with("- ")
}

/// `re.sub(r'^\s+-\s+', '', line).strip()`.
fn extract_block_item(line: &str) -> String {
    let n = leading_ws_len(line);
    let after_dash = line[n..].strip_prefix('-').unwrap_or(&line[n..]);
    after_dash
        .trim_start_matches([' ', '\t'])
        .trim()
        .to_string()
}

/// Check for `^\s{2,}\w`: at least two leading spaces/tabs, then a word
/// character.
fn is_sub_kv_line(line: &str) -> bool {
    let n = leading_ws_len(line);
    n >= 2
        && line[n..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `^\s+([A-Za-z_]\w*):\s*(.*)`, applied to the whole line: the leading
/// `\s+` is greedy, so it consumes every leading space/tab regardless of how
/// many `is_sub_kv_line` required.
fn match_sub_kv(line: &str) -> Option<(&str, &str)> {
    let n = leading_ws_len(line);
    match_key_colon_rest(line, n)
}

fn match_key_colon_rest(line: &str, start: usize) -> Option<(&str, &str)> {
    let bytes = line.as_bytes();
    if start >= bytes.len() || !(bytes[start].is_ascii_alphabetic() || bytes[start] == b'_') {
        return None;
    }
    let mut end = start + 1;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    if end >= bytes.len() || bytes[end] != b':' {
        return None;
    }
    Some((&line[start..end], &line[end + 1..]))
}

/// Parse an inline YAML flow sequence like `[a, b, "c"]` into a list. Strips
/// the surrounding brackets, splits on commas, trims whitespace, strips
/// matching quotes on each item, and drops empty items so `[]` yields an
/// empty list rather than one empty string.
fn parse_inline_list(val: &str) -> Vec<String> {
    let inner = &val[1..val.len() - 1];
    inner
        .split(',')
        .filter_map(|raw| {
            let item = raw.trim();
            let bytes = item.as_bytes();
            let unquoted = if item.len() >= 2
                && bytes[0] == bytes[item.len() - 1]
                && (bytes[0] == b'"' || bytes[0] == b'\'')
            {
                &item[1..item.len() - 1]
            } else {
                item
            };
            if unquoted.is_empty() {
                None
            } else {
                Some(unquoted.to_string())
            }
        })
        .collect()
}

// --- Node/edge id derivation ----------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Scope {
    Global,
    Project,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Scope::Global => "global",
            Scope::Project => "project",
        }
    }
}

/// A repo-root-relative path (`owner/repo/tail...`) is project-scoped; a
/// one- or two-segment path is global.
fn scope_and_project(rel: &str) -> (Scope, Option<String>) {
    let normalized = rel.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').collect();
    if parts.len() >= 3 {
        (Scope::Project, Some(format!("{}/{}", parts[0], parts[1])))
    } else {
        (Scope::Global, None)
    }
}

fn node_id(rel: &str, scope: Scope, project: Option<&str>) -> String {
    let normalized = rel.replace('\\', "/");
    let base = normalized.strip_suffix(".md").unwrap_or(&normalized);
    match scope {
        Scope::Global => format!("global/{base}"),
        Scope::Project => {
            let parts: Vec<&str> = base.split('/').collect();
            let tail = parts.get(2..).unwrap_or(&[]).join("/");
            format!("{}/{tail}", project.unwrap_or(""))
        }
    }
}

// --- Graph shape and rebuild ------------------------------------------------

#[derive(Serialize)]
struct Graph {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}

#[derive(Serialize)]
struct Node {
    id: String,
    file: String,
    scope: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
}

#[derive(Serialize)]
struct Edge {
    from: String,
    to: String,
    relation: String,
}

/// Rebuild the graph unconditionally, with no payload and no skip check.
///
/// Exists because `run` deliberately skips when the payload names no file
/// under the memory dir, which is correct for a PostToolUse hook but leaves
/// no way to force a rebuild. `commands/learn-project.md`'s `--graph-only`
/// path needs exactly that: the python original treated empty stdin as
/// "rebuild everything", and this port dropped that branch (see `should_skip`)
/// on the grounds it was unexercised by the test suite. It was exercised, just
/// by a slash command rather than a test.
pub fn rebuild_now() {
    rebuild();
}

fn rebuild() {
    let mem_dir = memory_dir();
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_code: HashSet<String> = HashSet::new();
    // (from_id, relation, raw_target, source_scope, source_project): buffered
    // here, resolved in pass 2 once every node id is known.
    let mut pending_links: Vec<(String, String, String, Scope, Option<String>)> = Vec::new();

    for fpath in walk_markdown_files(&mem_dir) {
        let Ok(rel_path) = fpath.strip_prefix(&mem_dir) else {
            continue;
        };
        let rel = rel_path.to_string_lossy().replace('\\', "/");
        let Ok(content) = fs::read_to_string(&fpath) else {
            continue;
        };

        let fm = parse_frontmatter(&content);
        let (scope, proj) = scope_and_project(&rel);
        let nid = node_id(&rel, scope, proj.as_deref());

        let file_name = fpath
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let default_name = file_name
            .strip_suffix(".md")
            .unwrap_or(&file_name)
            .to_string();

        let node_type = fm
            .scalar("type")
            .cloned()
            .unwrap_or_else(|| "reference".to_string());
        let name = fm.scalar("name").cloned().unwrap_or(default_name);
        let description = fm.scalar("description").cloned().unwrap_or_default();

        nodes.push(Node {
            id: nid.clone(),
            file: rel.clone(),
            scope: scope.as_str().to_string(),
            kind: node_type,
            name: Some(name),
            description: Some(description),
            project: proj.clone(),
        });

        if let Some(links) = fm.dict("links") {
            for (relation, target) in links {
                let targets = match target {
                    SubValue::List(items) => items.clone(),
                    SubValue::Scalar(one) => vec![one.clone()],
                };
                for one_target in targets {
                    pending_links.push((
                        nid.clone(),
                        relation.clone(),
                        one_target,
                        scope,
                        proj.clone(),
                    ));
                }
            }
        }

        if let Some(anchors) = fm.list("anchors") {
            for anchor in anchors {
                let cid = match &proj {
                    Some(p) => format!("code:{p}/{anchor}"),
                    None => format!("code:{anchor}"),
                };
                if !seen_code.contains(&cid) {
                    nodes.push(Node {
                        id: cid.clone(),
                        file: anchor.clone(),
                        scope: "code".to_string(),
                        kind: "code".to_string(),
                        name: None,
                        description: None,
                        project: proj.clone(),
                    });
                    seen_code.insert(cid.clone());
                }
                edges.push(Edge {
                    from: nid.clone(),
                    to: cid,
                    relation: "anchors".to_string(),
                });
            }
        }
    }

    // Pass 2: resolve buffered links now that every node id is known. A
    // project-scoped source resolves in its own scope first, then falls
    // back to global. A target found nowhere still emits the same-scope id,
    // so the edge is written and reads as dangling instead of being
    // silently dropped.
    let node_ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
    for (from_id, relation, raw_target, src_scope, src_project) in pending_links {
        let target_id = match src_scope {
            Scope::Global => format!("global/{raw_target}"),
            Scope::Project => {
                let project = src_project.unwrap_or_default();
                let same_scope_id = format!("{project}/{raw_target}");
                let global_id = format!("global/{raw_target}");
                if node_ids.contains(&same_scope_id) {
                    same_scope_id
                } else if node_ids.contains(&global_id) {
                    global_id
                } else {
                    same_scope_id
                }
            }
        };
        edges.push(Edge {
            from: from_id,
            to: target_id,
            relation,
        });
    }

    write_graph_atomically(&mem_dir, &Graph { nodes, edges });
}

/// Recursively collect every `.md` file under `dir` except `MEMORY.md`,
/// pruning dot-directories. Mirrors the `os.walk` filter in
/// `hooks/rebuild-memory-graph.py:rebuild`. A directory that cannot be read
/// (missing, permissions) contributes no files rather than aborting the walk.
fn walk_markdown_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_markdown_files_into(dir, &mut out);
    out
}

fn walk_markdown_files_into(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !name.starts_with('.') {
                walk_markdown_files_into(&path, out);
            }
        } else if name.ends_with(".md") && name != "MEMORY.md" {
            out.push(path);
        }
    }
}

/// Write `graph` to `graph.json` inside `mem_dir` via a temp file in the
/// same directory plus a rename, so a reader (or a crash mid-write) never
/// observes a partially written file, and a failed write leaves the
/// previous `graph.json` untouched. Mirrors
/// `tempfile.mkstemp(dir=MEMORY_DIR, ...)` plus `os.replace`.
fn write_graph_atomically(mem_dir: &Path, graph: &Graph) {
    let Ok(rendered) = serde_json::to_string_pretty(graph) else {
        return;
    };
    let tmp_path = mem_dir.join(format!(
        ".graph-{}-{:?}.json.tmp",
        std::process::id(),
        std::thread::current().id()
    ));
    if fs::write(&tmp_path, rendered).is_err() {
        let _ = fs::remove_file(&tmp_path);
        return;
    }
    if fs::rename(&tmp_path, mem_dir.join("graph.json")).is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
}
