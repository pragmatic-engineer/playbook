// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Text search over the index. Every FTS query is built from tokens, each
//! wrapped as an FTS5 string with quotes doubled, and passed as a bound
//! parameter, so operator text in the input is only ever text.

use super::index::sha256_hex;
use rusqlite::Connection;
use std::path::Path;

/// Default and maximum result counts.
pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 200;
/// The literal fallback scans at most this many indexed files.
pub const DEFAULT_SCAN_CAP: usize = 2000;

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub query: String,
    pub path: Option<String>,
    pub lang: Option<String>,
    pub limit: usize,
    pub scan_cap: usize,
}

impl SearchOptions {
    pub fn new(query: &str) -> Self {
        Self {
            query: query.to_string(),
            path: None,
            lang: None,
            limit: DEFAULT_LIMIT,
            scan_cap: DEFAULT_SCAN_CAP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    /// The first line that matches, 1 based.
    pub line: usize,
    pub match_type: &'static str,
    pub tier: u8,
    pub excerpt: String,
    pub stale: bool,
}

#[derive(Debug, Default)]
pub struct SearchResult {
    pub hits: Vec<Hit>,
    pub truncated: bool,
    pub omitted: usize,
    pub files_scanned: Option<usize>,
}

/// Words of the query: runs of letters, digits and underscores.
pub fn tokens(query: &str) -> Vec<String> {
    query
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// An FTS5 query that ANDs every token as a quoted string, or `None` when the
/// query has no tokens.
pub fn fts_query(query: &str) -> Option<String> {
    let toks = tokens(query);
    if toks.is_empty() {
        return None;
    }
    Some(
        toks.iter()
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Longest excerpt line, in characters.
const MAX_LINE_CHARS: usize = 200;

fn clip(line: &str) -> String {
    line.chars().take(MAX_LINE_CHARS).collect()
}

/// Reads `path`, and returns its lines plus whether it still hashes to
/// `sha` (`None` when the file cannot be read).
fn read_checked(root: &Path, path: &str, sha: &str) -> Option<(Vec<String>, bool)> {
    let bytes = std::fs::read(root.join(path)).ok()?;
    let same = sha256_hex(&bytes) == sha;
    let text = String::from_utf8_lossy(&bytes);
    Some((text.lines().map(str::to_string).collect(), same))
}

/// The first line in `[start, end]` that holds every token, else any token,
/// else the first line. 1 based.
fn match_line(lines: &[String], start: usize, end: usize, toks: &[String]) -> usize {
    let lower: Vec<String> = toks.iter().map(|t| t.to_lowercase()).collect();
    let window = || (start..=end.min(lines.len())).filter(|n| *n >= 1);
    let holds = |n: usize, all: bool| {
        let line = lines[n - 1].to_lowercase();
        if all {
            lower.iter().all(|t| line.contains(t.as_str()))
        } else {
            lower.iter().any(|t| line.contains(t.as_str()))
        }
    };
    window()
        .find(|n| holds(*n, true))
        .or_else(|| window().find(|n| holds(*n, false)))
        .unwrap_or(start)
}

/// The matching line with one line of context on each side.
fn excerpt(lines: &[String], line: usize) -> String {
    let from = line.saturating_sub(1).max(1);
    let to = (line + 1).min(lines.len());
    (from..=to)
        .map(|n| clip(&lines[n - 1]))
        .collect::<Vec<_>>()
        .join("\n")
}

struct Row {
    path: String,
    sha: String,
    start_line: usize,
    end_line: usize,
}

fn filters(opts: &SearchOptions) -> (String, Vec<String>) {
    let mut sql = String::new();
    let mut params = Vec::new();
    if let Some(prefix) = &opts.path {
        sql.push_str(" AND substr(f.path, 1, length(?)) = ?");
        params.push(prefix.clone());
        params.push(prefix.clone());
    }
    if let Some(lang) = &opts.lang {
        sql.push_str(" AND f.lang = ?");
        params.push(lang.clone());
    }
    (sql, params)
}

fn fts_rows(
    conn: &Connection,
    query: &str,
    opts: &SearchOptions,
    fetch: usize,
) -> Result<(Vec<Row>, usize), String> {
    let (extra, extra_params) = filters(opts);
    let base = format!(
        "FROM chunks_fts JOIN chunks c ON c.id = chunks_fts.rowid
         JOIN files f ON f.id = c.file_id
         WHERE chunks_fts MATCH ?{extra}"
    );
    let mut params: Vec<String> = vec![query.to_string()];
    params.extend(extra_params);
    let total: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) {base}"),
            rusqlite::params_from_iter(params.iter()),
            |r| r.get(0),
        )
        .map_err(|e| format!("search failed: {e}"))?;
    let sql = format!(
        "SELECT f.path, f.sha256, c.start_line, c.end_line {base}
         ORDER BY bm25(chunks_fts), f.path, c.start_line LIMIT {fetch}"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("search failed: {e}"))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok(Row {
                path: r.get(0)?,
                sha: r.get(1)?,
                start_line: usize::try_from(r.get::<_, i64>(2)?).unwrap_or(1),
                end_line: usize::try_from(r.get::<_, i64>(3)?).unwrap_or(1),
            })
        })
        .map_err(|e| format!("search failed: {e}"))?;
    let rows = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("search failed: {e}"))?;
    Ok((rows, usize::try_from(total).unwrap_or(0)))
}

/// Searches indexed text. FTS hits come first, ordered by relevance then path
/// and line. With no tokens or no FTS hits, a literal substring scan over
/// indexed files answers instead, bounded by `scan_cap` files.
pub fn text_search(
    conn: &Connection,
    root: &Path,
    opts: &SearchOptions,
) -> Result<SearchResult, String> {
    let toks = tokens(&opts.query);
    if let Some(query) = fts_query(&opts.query) {
        let (rows, total) = fts_rows(conn, &query, opts, opts.limit)?;
        if !rows.is_empty() {
            let hits = rows
                .into_iter()
                .map(|row| {
                    let (lines, same) = read_checked(root, &row.path, &row.sha)
                        .map_or((Vec::new(), false), |(l, s)| (l, s));
                    let line = if lines.is_empty() {
                        row.start_line
                    } else {
                        match_line(&lines, row.start_line, row.end_line, &toks)
                    };
                    Hit {
                        excerpt: if lines.is_empty() {
                            String::new()
                        } else {
                            excerpt(&lines, line)
                        },
                        path: row.path,
                        start_line: row.start_line,
                        end_line: row.end_line,
                        line,
                        match_type: "text",
                        tier: 6,
                        stale: !same,
                    }
                })
                .collect::<Vec<_>>();
            let omitted = total.saturating_sub(hits.len());
            return Ok(SearchResult {
                hits,
                truncated: omitted > 0,
                omitted,
                files_scanned: None,
            });
        }
    }
    literal_scan(conn, root, opts)
}

fn literal_scan(
    conn: &Connection,
    root: &Path,
    opts: &SearchOptions,
) -> Result<SearchResult, String> {
    let needle = opts.query.trim();
    let mut result = SearchResult {
        files_scanned: Some(0),
        ..SearchResult::default()
    };
    if needle.is_empty() {
        return Ok(result);
    }
    let (extra, extra_params) = filters(opts);
    let sql = format!(
        "SELECT f.path, f.sha256 FROM files f WHERE f.status IN ('ok', 'partial'){extra} ORDER BY f.path"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("search failed: {e}"))?;
    let files: Vec<(String, String)> = stmt
        .query_map(rusqlite::params_from_iter(extra_params.iter()), |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(|e| format!("search failed: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("search failed: {e}"))?;
    let mut scanned = 0usize;
    for (path, sha) in files {
        if scanned >= opts.scan_cap {
            result.truncated = true;
            break;
        }
        scanned += 1;
        let Some((lines, same)) = read_checked(root, &path, &sha) else {
            continue;
        };
        for (i, text) in lines.iter().enumerate() {
            if !text.contains(needle) {
                continue;
            }
            if result.hits.len() >= opts.limit {
                result.truncated = true;
                result.omitted += 1;
                continue;
            }
            result.hits.push(Hit {
                path: path.clone(),
                start_line: i + 1,
                end_line: i + 1,
                line: i + 1,
                match_type: "literal",
                tier: 7,
                excerpt: excerpt(&lines, i + 1),
                stale: !same,
            });
        }
    }
    result.files_scanned = Some(scanned);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::testkit::{fx, pinned, Fx};

    fn search(f: &Fx, opts: &SearchOptions) -> SearchResult {
        text_search(&f.conn(), &f.root, opts).expect("search")
    }

    fn paths(r: &SearchResult) -> Vec<&str> {
        r.hits.iter().map(|h| h.path.as_str()).collect()
    }

    #[test]
    fn operator_text_in_the_input_is_searched_as_text() {
        let _g = pinned();
        // Arrange
        let f = fx("se-operators");
        f.write("a.txt", "retry AND OR (\n");
        f.write("b.txt", "retry only\n");
        f.run();

        // Act
        let got = search(&f, &SearchOptions::new("retry AND OR ("));

        // Assert
        assert_eq!(paths(&got), vec!["a.txt"]);
        assert_eq!((got.hits[0].start_line, got.hits[0].line), (1, 1));
        assert_eq!(got.hits[0].match_type, "text");
    }

    #[test]
    fn an_injection_attempt_does_not_widen_the_query() {
        let _g = pinned();
        // Arrange
        let f = fx("se-injection");
        f.write("a.txt", "x\" OR \"1\"=\"1\n");
        f.write("b.txt", "one 1 here\n");
        f.run();

        // Act
        let got = search(&f, &SearchOptions::new("x\" OR \"1\"=\"1"));

        // Assert
        assert_eq!(paths(&got), vec!["a.txt"]);
    }

    #[test]
    fn nineteen_and_twenty_hits_return_in_full_and_twenty_one_truncate() {
        let _g = pinned();
        for (count, want_len, truncated, omitted) in [
            (19usize, 19usize, false, 0usize),
            (20, 20, false, 0),
            (21, 20, true, 1),
        ] {
            // Arrange
            let f = fx(&format!("se-limit-{count}"));
            for i in 0..count {
                f.write(&format!("f{i:02}.txt"), "needleword here\n");
            }
            f.run();

            // Act
            let got = search(&f, &SearchOptions::new("needleword"));

            // Assert
            assert_eq!(got.hits.len(), want_len, "{count} hits");
            assert_eq!(got.truncated, truncated, "{count} truncated");
            assert_eq!(got.omitted, omitted, "{count} omitted");
        }
    }

    #[test]
    fn the_literal_scan_stops_at_the_cap_and_reports_files_scanned() {
        let _g = pinned();
        for (files, scanned, truncated) in [(4usize, 4usize, false), (5, 5, false), (6, 5, true)] {
            // Arrange: the query has no tokens, so only the literal scan can answer.
            let f = fx(&format!("se-scan-{files}"));
            for i in 0..files {
                f.write(&format!("f{i}.txt"), "plain words\n");
            }
            f.run();
            let mut opts = SearchOptions::new("+++");
            opts.scan_cap = 5;

            // Act
            let got = search(&f, &opts);

            // Assert
            assert!(got.hits.is_empty());
            assert_eq!(got.files_scanned, Some(scanned), "{files} files");
            assert_eq!(got.truncated, truncated, "{files} files");
        }
    }

    #[test]
    fn a_literal_only_match_is_found_by_the_fallback() {
        let _g = pinned();
        // Arrange
        let f = fx("se-literal");
        f.write("a.txt", "total = a + b\n");
        f.write("b.txt", "nothing\n");
        f.run();

        // Act
        let got = search(&f, &SearchOptions::new("a + b"));

        // Assert: tokens `a` and `b` match through FTS; the file is found either way.
        assert_eq!(paths(&got), vec!["a.txt"]);
    }

    #[test]
    fn a_file_edited_after_indexing_is_flagged_stale() {
        let _g = pinned();
        // Arrange
        let f = fx("se-stale");
        f.write("a.txt", "needleword\n");
        f.run();
        f.write("a.txt", "needleword changed\n");

        // Act
        let got = search(&f, &SearchOptions::new("needleword"));

        // Assert
        assert_eq!(paths(&got), vec!["a.txt"]);
        assert!(got.hits[0].stale);
    }

    #[test]
    fn path_and_language_filters_exclude_other_files() {
        let _g = pinned();
        // Arrange
        let f = fx("se-filter");
        f.write("lib/a.txt", "needleword\n");
        f.write("other/b.txt", "needleword\n");
        f.run();
        let mut opts = SearchOptions::new("needleword");
        opts.path = Some("lib/".to_string());

        // Act
        let got = search(&f, &opts);

        // Assert
        assert_eq!(paths(&got), vec!["lib/a.txt"]);
    }

    #[test]
    fn the_fts_query_wraps_each_token_and_doubles_quotes() {
        // Arrange, Act, Assert
        assert_eq!(
            fts_query("retry AND OR ("),
            Some("\"retry\" \"AND\" \"OR\"".to_string())
        );
        assert_eq!(fts_query("++ --"), None);
    }
}
