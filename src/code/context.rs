// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `context`: retrieval and formatting for a task described in words. It
//! splits terms, finds symbols and text, widens text hits to their enclosing
//! declaration, adds imports and tests, merges overlaps and fills a byte
//! budget. Nothing here calls a model.

use super::search::{fts_hits, load, Loaded, SearchOptions};
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::Path;

pub const DEFAULT_MAX_TOKENS: usize = 4000;
pub const MIN_TOKENS: usize = 200;
/// The byte ceiling never exceeds this, whatever the budget.
pub const ABS_CEILING: usize = 256 * 1024;
/// Longest single snippet, in lines.
const MAX_SNIPPET_LINES: usize = 120;
/// Files that get their imports added.
const TOP_FILES: usize = 3;
/// Rows read per lookup.
const LOOKUP_LIMIT: usize = 200;
/// Omitted items listed in the output.
const MAX_LISTED_OMITTED: usize = 50;

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "of", "to", "in", "on", "for", "and", "or", "with", "from", "by", "is",
    "are", "be", "this", "that", "it", "as", "at", "how", "what", "where", "when", "why", "add",
    "make", "use", "get", "do", "does", "we", "i",
];

#[derive(Debug, Clone)]
pub struct ContextOptions {
    pub query: String,
    pub max_tokens: usize,
    /// Replaces the byte ceiling, for tests.
    pub ceiling: Option<usize>,
}

impl ContextOptions {
    pub fn new(query: &str) -> Self {
        Self {
            query: query.to_string(),
            max_tokens: DEFAULT_MAX_TOKENS,
            ceiling: None,
        }
    }

    /// Bytes allowed in the output: four per token, never above the absolute ceiling.
    pub fn byte_ceiling(&self) -> usize {
        self.ceiling
            .unwrap_or_else(|| (self.max_tokens * 4).min(ABS_CEILING))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub reason: String,
    pub tier: u8,
    pub text: String,
    pub stale: bool,
}

/// A candidate that did not fit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Omitted {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct ContextResult {
    pub terms: Vec<String>,
    pub snippets: Vec<Snippet>,
    pub omitted: Vec<Omitted>,
    /// How many candidates did not fit, which may exceed the listed `omitted`.
    pub omitted_count: usize,
    /// Candidates whose file could no longer be read.
    pub unreadable: usize,
    pub truncated: bool,
    pub bytes: usize,
}

/// Lowercase words of the query without stopwords, each once, in order.
pub fn terms(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in super::search::tokens(query) {
        let lower = word.to_lowercase();
        if STOPWORDS.contains(&lower.as_str()) || out.contains(&lower) {
            continue;
        }
        out.push(lower);
    }
    out
}

#[derive(Debug, Clone)]
struct Cand {
    path: String,
    sha: String,
    start: usize,
    end: usize,
    reason: String,
    tier: u8,
}

fn sql_err(e: rusqlite::Error) -> String {
    format!("context failed: {e}")
}

fn to_usize(n: i64) -> usize {
    usize::try_from(n).unwrap_or(0)
}

/// Symbols whose name holds `term`, with a tier from how well the name fits.
fn symbol_cands(conn: &Connection, term: &str) -> Result<Vec<Cand>, String> {
    let pattern = format!("%{}%", term.replace('\\', "\\\\").replace('_', "\\_"));
    let mut stmt = conn
        .prepare(
            "SELECT f.path, f.sha256, s.name, s.qualified_name, s.start_line, s.end_line
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE lower(s.name) LIKE ?1 ESCAPE '\\'
             ORDER BY (lower(s.name) = ?3) DESC, f.path, s.start_line, s.id LIMIT ?2",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map(params![pattern, LOOKUP_LIMIT as i64, term], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                to_usize(r.get(4)?),
                to_usize(r.get(5)?),
            ))
        })
        .map_err(sql_err)?;
    let mut out = Vec::new();
    for row in rows {
        let (path, sha, name, qualified, start, end) = row.map_err(sql_err)?;
        let lower = name.to_lowercase();
        let pieces = super::index::identifier_pieces(&name);
        let tier = if lower == term {
            2
        } else if pieces.iter().any(|p| p == term) {
            4
        } else {
            6
        };
        out.push(Cand {
            path,
            sha,
            start,
            end,
            reason: format!("symbol {qualified}"),
            tier,
        });
    }
    Ok(out)
}

/// The smallest symbol in `path` that holds `line`, as a range.
fn enclosing(conn: &Connection, path: &str, line: usize) -> Result<Option<(usize, usize)>, String> {
    let line = i64::try_from(line).unwrap_or(i64::MAX);
    let mut stmt = conn
        .prepare(
            "SELECT s.start_line, s.end_line FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE f.path = ?1 AND s.start_line <= ?2 AND s.end_line >= ?2
             ORDER BY (s.end_line - s.start_line), s.id LIMIT 1",
        )
        .map_err(sql_err)?;
    let mut rows = stmt
        .query_map(params![path, line], |r| {
            Ok((to_usize(r.get(0)?), to_usize(r.get(1)?)))
        })
        .map_err(sql_err)?;
    rows.next().transpose().map_err(sql_err)
}

fn sha_of(conn: &Connection, path: &str) -> String {
    conn.query_row("SELECT sha256 FROM files WHERE path = ?1", [path], |r| {
        r.get(0)
    })
    .unwrap_or_default()
}

/// Text hits for `words`, each widened to its enclosing declaration.
fn text_cands(
    conn: &Connection,
    root: &Path,
    words: &[String],
    reason: &str,
    fetch: usize,
) -> Result<Vec<Cand>, String> {
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let mut opts = SearchOptions::new(&words.join(" "));
    opts.any_word = true;
    let (hits, _) = fts_hits(conn, root, &opts, fetch)?;
    let mut out = Vec::new();
    for hit in hits {
        let (start, end) = match enclosing(conn, &hit.path, hit.line)? {
            Some(range) => range,
            None => (hit.line.saturating_sub(3).max(1), hit.line + 3),
        };
        out.push(Cand {
            sha: sha_of(conn, &hit.path),
            path: hit.path,
            start,
            end,
            reason: reason.to_string(),
            tier: 6,
        });
    }
    Ok(out)
}

/// Merges overlapping ranges within a file. Reasons join, the best tier wins.
fn merge(mut cands: Vec<Cand>) -> Vec<Cand> {
    cands.sort_by(|a, b| (&a.path, a.start, a.end).cmp(&(&b.path, b.start, b.end)));
    let mut out: Vec<Cand> = Vec::new();
    for c in cands {
        if let Some(last) = out.last_mut() {
            let union = last.end.max(c.end) - last.start + 1;
            if last.path == c.path && c.start <= last.end && union <= MAX_SNIPPET_LINES {
                last.end = last.end.max(c.end);
                last.tier = last.tier.min(c.tier);
                if !last.reason.split("; ").any(|r| r == c.reason) {
                    last.reason = format!("{}; {}", last.reason, c.reason);
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}

fn is_test_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    let mut parts = lower.split('/').collect::<Vec<_>>();
    let file = parts.pop().unwrap_or("");
    let in_test_dir = parts
        .iter()
        .any(|d| matches!(*d, "test" | "tests" | "spec" | "specs" | "__tests__"));
    let stem = file.split('.').next().unwrap_or("");
    let marked = file.contains(".test.")
        || file.contains(".spec.")
        || stem.ends_with("_test")
        || stem.ends_with("_spec")
        || stem.starts_with("test_");
    in_test_dir || marked
}

/// Import lines of `paths` that mention a term or a top symbol name.
fn import_cands(
    conn: &Connection,
    paths: &[String],
    needles: &[String],
) -> Result<Vec<Cand>, String> {
    let mut out = Vec::new();
    for path in paths {
        let mut stmt = conn
            .prepare(
                "SELECT i.module, i.names, i.line FROM imports i JOIN files f ON f.id = i.file_id
                 WHERE f.path = ?1 ORDER BY i.line",
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([path], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    to_usize(r.get(2)?),
                ))
            })
            .map_err(sql_err)?;
        for row in rows {
            let (module, names, line) = row.map_err(sql_err)?;
            let hay = format!("{} {}", module, names.unwrap_or_default()).to_lowercase();
            if needles.iter().any(|n| hay.contains(n.as_str())) {
                out.push(Cand {
                    path: path.clone(),
                    sha: sha_of(conn, path),
                    start: line,
                    end: line,
                    reason: "import".to_string(),
                    tier: 5,
                });
            }
        }
    }
    Ok(out)
}

fn cut_at_boundary(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// How much each term tells apart: rare names weigh more than common ones.
fn term_weights(conn: &Connection, words: &[String]) -> Result<Vec<f64>, String> {
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
        .map_err(sql_err)?;
    let mut out = Vec::new();
    for word in words {
        let pattern = format!("%{}%", word.replace('\\', "\\\\").replace('_', "\\_"));
        let df: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM symbols WHERE lower(name) LIKE ?1 ESCAPE '\\'",
                [pattern],
                |r| r.get(0),
            )
            .map_err(sql_err)?;
        out.push((2.0 + total as f64 / (1.0 + df as f64)).ln());
    }
    Ok(out)
}

/// Weighted count of the query terms a candidate holds in its path, its
/// symbol names and its text. A term in a symbol name counts twice.
fn score(
    cand: &Cand,
    words: &[String],
    weights: &[f64],
    root: &Path,
    cache: &mut HashMap<String, Option<Loaded>>,
) -> f64 {
    let loaded = cache
        .entry(cand.path.clone())
        .or_insert_with(|| load(root, &cand.path));
    let body = loaded.as_ref().map_or(String::new(), |l| {
        let end = cand
            .end
            .min(l.lines.len())
            .min(cand.start + MAX_SNIPPET_LINES - 1);
        if cand.start == 0 || cand.start > end {
            String::new()
        } else {
            l.lines[cand.start - 1..end].join("\n")
        }
    });
    let haystack = format!("{}\n{}", cand.path, body).to_lowercase();
    let names = cand.reason.to_lowercase();
    words
        .iter()
        .zip(weights)
        .map(|(w, weight)| {
            let mut got = 0.0;
            if haystack.contains(w.as_str()) {
                got += weight;
            }
            if names.contains(w.as_str()) {
                got += weight;
            }
            got
        })
        .sum()
}

/// Candidates best first: score, then tier, path and line.
fn rank(cands: &[Cand], scores: &[f64]) -> Vec<Cand> {
    let mut idx: Vec<usize> = (0..cands.len()).collect();
    idx.sort_by(|&a, &b| {
        scores[b]
            .total_cmp(&scores[a])
            .then_with(|| cands[a].tier.cmp(&cands[b].tier))
            .then_with(|| cands[a].path.cmp(&cands[b].path))
            .then_with(|| cands[a].start.cmp(&cands[b].start))
    });
    idx.into_iter().map(|i| cands[i].clone()).collect()
}

/// Builds the context for `opts.query`.
pub fn build(
    conn: &Connection,
    root: &Path,
    opts: &ContextOptions,
) -> Result<ContextResult, String> {
    let words = terms(&opts.query);
    let mut result = ContextResult {
        terms: words.clone(),
        ..ContextResult::default()
    };
    if words.is_empty() {
        return Ok(result);
    }
    let mut cands: Vec<Cand> = Vec::new();
    for word in &words {
        cands.extend(symbol_cands(conn, word)?);
    }
    let text = text_cands(conn, root, &words, "text", 20)?;
    cands.extend(text.into_iter().filter(|c| !is_test_path(&c.path)));
    let merged = merge(cands);
    let weights = term_weights(conn, &words)?;
    let mut cache: HashMap<String, Option<Loaded>> = HashMap::new();
    let scores: Vec<f64> = merged
        .iter()
        .map(|c| score(c, &words, &weights, root, &mut cache))
        .collect();
    let mut ordered = rank(&merged, &scores);

    let mut top_paths: Vec<String> = Vec::new();
    for c in &ordered {
        if !top_paths.contains(&c.path) && top_paths.len() < TOP_FILES {
            top_paths.push(c.path.clone());
        }
    }
    let top_names: Vec<String> = ordered
        .iter()
        .flat_map(|c| c.reason.split("; "))
        .filter_map(|r| r.strip_prefix("symbol "))
        .take(3)
        .map(|q| q.rsplit(['.', ':']).next().unwrap_or(q).to_string())
        .collect();
    let mut needles = words.clone();
    needles.extend(top_names.iter().map(|n| n.to_lowercase()));

    let mut extra = import_cands(conn, &top_paths, &needles)?;
    let tests = text_cands(conn, root, &top_names, "test", 50)?;
    extra.extend(tests.into_iter().filter(|c| is_test_path(&c.path)));
    // An extra inside a snippet already chosen adds its reason to that snippet.
    let mut merged = merged;
    let mut extra: Vec<Cand> = merge(extra)
        .into_iter()
        .filter(|c| {
            let host = merged
                .iter_mut()
                .find(|p| p.path == c.path && p.start <= c.start && c.end <= p.end);
            match host {
                Some(p) => {
                    if !p.reason.split("; ").any(|r| r == c.reason) {
                        p.reason = format!("{}; {}", p.reason, c.reason);
                    }
                    false
                }
                None => true,
            }
        })
        .collect();
    extra.sort_by(|a, b| (a.tier, &a.path, a.start).cmp(&(b.tier, &b.path, b.start)));
    let scores: Vec<f64> = merged
        .iter()
        .map(|c| score(c, &words, &weights, root, &mut cache))
        .collect();
    ordered = rank(&merged, &scores);
    ordered.extend(extra);

    fill(&mut result, root, &ordered, opts.byte_ceiling());
    Ok(result)
}

/// Reads each candidate from disk in order until the byte ceiling is reached.
fn fill(result: &mut ContextResult, root: &Path, ordered: &[Cand], ceiling: usize) {
    let mut cache: HashMap<String, Option<Loaded>> = HashMap::new();
    let mut full = false;
    let mut stale_cache: HashMap<String, bool> = HashMap::new();
    for cand in ordered {
        let omit = |result: &mut ContextResult, cand: &Cand| {
            result.omitted_count += 1;
            if result.omitted.len() < MAX_LISTED_OMITTED {
                result.omitted.push(Omitted {
                    path: cand.path.clone(),
                    start_line: cand.start,
                    end_line: cand.end,
                    reason: cand.reason.clone(),
                });
            }
        };
        if full {
            omit(result, cand);
            continue;
        }
        let loaded = cache
            .entry(cand.path.clone())
            .or_insert_with(|| load(root, &cand.path));
        let Some(loaded) = loaded.as_ref() else {
            result.unreadable += 1;
            continue;
        };
        let end = cand
            .end
            .min(loaded.lines.len())
            .min(cand.start + MAX_SNIPPET_LINES - 1);
        if cand.start == 0 || cand.start > end {
            omit(result, cand);
            continue;
        }
        let text = loaded.lines[cand.start - 1..end].join("\n");
        let room = ceiling.saturating_sub(result.bytes);
        let (text, cut) = if text.len() > room {
            (cut_at_boundary(&text, room).to_string(), true)
        } else {
            (text, false)
        };
        // A cut snippet ends on the last line it still shows.
        let end = if cut {
            cand.start + text.matches('\n').count()
        } else {
            end
        };
        if text.is_empty() {
            full = true;
            result.truncated = true;
            omit(result, cand);
            continue;
        }
        result.bytes += text.len();
        result.snippets.push(Snippet {
            path: cand.path.clone(),
            start_line: cand.start,
            end_line: end,
            reason: cand.reason.clone(),
            tier: cand.tier,
            stale: *stale_cache
                .entry(cand.path.clone())
                .or_insert_with(|| !loaded.same(&cand.sha)),
            text,
        });
        if cut || result.bytes >= ceiling {
            full = true;
            if cut {
                result.truncated = true;
            }
        }
    }
    if result.omitted_count > 0 {
        result.truncated = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::testkit::{fx, pinned, Fx};

    fn ctx(f: &Fx, q: &str) -> ContextResult {
        build(&f.conn(), &f.root, &ContextOptions::new(q)).expect("context")
    }

    #[test]
    fn stopwords_are_dropped_and_terms_keep_their_order() {
        // Arrange, Act, Assert
        assert_eq!(terms("the payment of retries"), vec!["payment", "retries"]);
        assert_eq!(
            terms("add retry support to payment processing"),
            vec!["retry", "support", "payment", "processing"]
        );
    }

    #[test]
    fn the_first_snippets_are_the_matching_declarations() {
        let _g = pinned();
        // Arrange
        let f = fx("cx-first");
        f.write(
            "pay.ts",
            "export function processPayment() {\n  return 1;\n}\n",
        );
        f.write(
            "retry.ts",
            "export function withRetry() {\n  return 2;\n}\n",
        );
        f.write("other.ts", "export function unrelated() {}\n");
        f.run();

        // Act
        let got = ctx(&f, "add retry support to payment processing");

        // Assert
        let paths: Vec<&str> = got.snippets.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths[..2], ["pay.ts", "retry.ts"]);
        assert!(!paths.contains(&"other.ts"));
        assert!(got.snippets[0].reason.starts_with("symbol "));
    }

    #[test]
    fn a_piece_of_a_camel_case_name_matches() {
        let _g = pinned();
        // Arrange
        let f = fx("cx-pieces");
        f.write("pay.ts", "export function processPayment() {}\n");
        f.run();

        // Act
        let got = ctx(&f, "the payment of retries");

        // Assert
        assert_eq!(got.terms, vec!["payment", "retries"]);
        assert_eq!(got.snippets[0].path, "pay.ts");
    }

    #[test]
    fn overlapping_hits_in_one_function_merge_into_one_snippet() {
        let _g = pinned();
        // Arrange: the name matches `retry` and the body matches it as text too.
        let f = fx("cx-merge");
        f.write(
            "a.ts",
            "function retryLoop() {\n  // retry again\n  retry();\n}\n",
        );
        f.run();

        // Act
        let got = ctx(&f, "retry");

        // Assert
        assert_eq!(got.snippets.len(), 1);
        assert_eq!(
            (got.snippets[0].start_line, got.snippets[0].end_line),
            (1, 4)
        );
        assert!(got.snippets[0].reason.contains("symbol retryLoop"));
        assert!(got.snippets[0].reason.contains("text"));
    }

    #[test]
    fn imports_of_a_top_file_and_only_matching_tests_are_added() {
        let _g = pinned();
        // Arrange
        let f = fx("cx-extras");
        f.write(
            "pay.ts",
            "import { helper } from './paymentGateway';\nimport { other } from './unrelated';\n\n\n\n\n\n\n\nexport function processPayment() {}\n",
        );
        f.write(
            "pay.test.ts",
            "test('processPayment works', () => processPayment());\n",
        );
        f.write("misc.test.ts", "test('nothing', () => 1);\n");
        f.run();

        // Act
        let got = ctx(&f, "payment");

        // Assert
        let reasons: Vec<(&str, &str)> = got
            .snippets
            .iter()
            .map(|s| (s.path.as_str(), s.reason.as_str()))
            .collect();
        assert!(
            reasons
                .iter()
                .any(|(p, r)| *p == "pay.ts" && r.contains("import")),
            "{reasons:?}"
        );
        assert!(
            reasons
                .iter()
                .any(|(p, r)| *p == "pay.test.ts" && *r == "test"),
            "{reasons:?}"
        );
        assert!(
            !reasons.iter().any(|(p, _)| *p == "misc.test.ts"),
            "{reasons:?}"
        );
        let imports = got
            .snippets
            .iter()
            .filter(|s| s.reason.contains("import"))
            .count();
        assert_eq!(imports, 1);
    }

    #[test]
    fn the_budget_has_a_minimum_and_a_ceiling_of_four_bytes_per_token() {
        let _g = pinned();
        // Arrange
        let f = fx("cx-budget");
        for i in 0..6 {
            f.write(
                &format!("f{i}.ts"),
                &format!("function retry{i}() {{\n{}}}\n", "  work();\n".repeat(40)),
            );
        }
        f.run();
        let mut opts = ContextOptions::new("retry");
        opts.max_tokens = MIN_TOKENS;

        // Act
        let got = build(&f.conn(), &f.root, &opts).expect("context");

        // Assert
        assert!(got.bytes <= 800, "{}", got.bytes);
        assert!(!got.omitted.is_empty());
        assert!(got.truncated);
    }

    #[test]
    fn an_injected_ceiling_stops_output_at_1024_bytes() {
        let _g = pinned();
        for (body_bytes, truncated) in [(1023usize, false), (1024, false), (1025, true)] {
            // Arrange: one function whose snippet text is exactly `body_bytes` long.
            let f = fx(&format!("cx-ceiling-{body_bytes}"));
            let head = "function retry() {";
            let tail = "}";
            let filler = body_bytes - head.len() - tail.len() - 2;
            f.write("a.ts", &format!("{head}\n{}\n{tail}\n", "x".repeat(filler)));
            f.run();
            let mut opts = ContextOptions::new("retry");
            opts.ceiling = Some(1024);

            // Act
            let got = build(&f.conn(), &f.root, &opts).expect("context");

            // Assert
            assert_eq!(got.bytes, body_bytes.min(1024), "{body_bytes}");
            assert_eq!(got.truncated, truncated, "{body_bytes}");
        }
    }

    #[test]
    fn a_multi_term_query_finds_text_where_the_terms_sit_on_different_lines() {
        let _g = pinned();
        // Arrange
        let f = fx("cx-any-word");
        f.write(
            "notes.txt",
            "gather alpha here\nsome filler line\nand omega over there\n",
        );
        f.run();

        // Act
        let got = ctx(&f, "alpha omega");

        // Assert
        assert!(got
            .snippets
            .iter()
            .any(|s| s.path == "notes.txt" && s.reason == "text"));
    }

    #[test]
    fn a_cut_inside_a_multibyte_character_keeps_valid_text_and_a_true_end_line() {
        let _g = pinned();
        // Arrange: each e-acute is two bytes, so a 5 byte ceiling lands inside the third.
        let f = fx("cx-multibyte");
        f.write(
            "a.ts",
            "function retry() {\n  // \u{e9}\u{e9}\u{e9}\u{e9}\n}\n",
        );
        f.run();
        let mut opts = ContextOptions::new("retry");
        opts.ceiling = Some(25);

        // Act
        let got = build(&f.conn(), &f.root, &opts).expect("context");

        // Assert
        assert!(got.bytes <= 25);
        assert!(got.truncated);
        let s = &got.snippets[0];
        assert_eq!(s.end_line, s.start_line + s.text.matches('\n').count());
    }

    #[test]
    fn a_long_declaration_is_capped_at_120_lines() {
        let _g = pinned();
        // Arrange
        let f = fx("cx-line-cap");
        let body = "  work();\n".repeat(200);
        f.write("a.ts", &format!("function retry() {{\n{body}}}\n"));
        f.run();

        // Act
        let got = ctx(&f, "retry");

        // Assert
        let s = &got.snippets[0];
        assert_eq!(s.end_line - s.start_line + 1, 120);
    }

    #[test]
    fn paths_are_tests_by_directory_or_name_pattern_only() {
        // Arrange, Act, Assert
        for yes in [
            "tests/a.rs",
            "src/__tests__/a.ts",
            "a.test.ts",
            "b/a.spec.js",
            "x_test.go",
            "test_x.py",
        ] {
            assert!(is_test_path(yes), "{yes}");
        }
        for no in [
            "src/inspect.ts",
            "respect.py",
            "src/specification/a.rs",
            "latest.ts",
            "contest/a.rs",
        ] {
            assert!(!is_test_path(no), "{no}");
        }
    }

    #[test]
    fn the_checkpoint_task_fixture_holds_twenty_complete_tasks() {
        // Arrange
        let text = include_str!("../../tests/fixtures/code_c1_tasks.json");

        // Act
        let doc: serde_json::Value = serde_json::from_str(text).expect("fixture parses");

        // Assert
        let tasks = doc["tasks"].as_array().expect("tasks");
        assert_eq!(tasks.len(), 20);
        for task in tasks {
            let repo = task["repo"].as_str().expect("repo");
            assert!(doc["repos"][repo]["commit"]
                .as_str()
                .is_some_and(|c| c.len() == 40));
            assert!(task["query"].as_str().is_some_and(|q| !q.is_empty()));
            let first = &task["expected"][0];
            assert!(first["path"].as_str().is_some_and(|p| !p.is_empty()));
            assert!(first["line"].as_u64().is_some_and(|l| l > 0));
        }
    }

    #[test]
    fn two_runs_give_identical_snippets() {
        let _g = pinned();
        // Arrange
        let f = fx("cx-stable");
        f.write("a.ts", "function retry() {}\nfunction retryAgain() {}\n");
        f.write("b.ts", "function retryLater() {}\n");
        f.run();

        // Act
        let a = format!("{:?}", ctx(&f, "retry").snippets);
        let b = format!("{:?}", ctx(&f, "retry").snippets);

        // Assert
        assert_eq!(a, b);
        assert!(a.contains("retry"));
    }
}
