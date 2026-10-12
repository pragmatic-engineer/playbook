// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The thin command line layer: argument shapes, and rendering of the results
//! the library functions in `code` return.

use super::host;
use super::output::{Envelope, IndexState};
use super::search::SearchOptions;
use super::status;
use clap::Subcommand;
use serde_json::json;
use std::path::PathBuf;

/// `playbook code` subcommands.
#[derive(Subcommand, Debug)]
pub enum CodeCommand {
    /// Build or refresh the code index for this repository
    ///
    /// Hashes every file git lists, and reads only the ones that changed.
    /// Only one run at a time is allowed.
    ///
    /// Example: `playbook code index --json`
    Index {
        /// Print one JSON document
        #[arg(long)]
        json: bool,
        /// Delete the cache first and index everything again
        #[arg(long)]
        rebuild: bool,
    },
    /// Report whether the index matches the files on disk
    ///
    /// Hashes every file git lists, so it is slower than the quick check that
    /// search commands run first.
    ///
    /// Example: `playbook code status --json`
    Status {
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Search symbols, imports and text of this repository
    ///
    /// Without the text flag, results are ranked in tiers: exact qualified
    /// name, exact name, name ignoring case, name prefix, import, text, then a
    /// plain substring scan. With it, only indexed text is searched. Words in
    /// the query are matched as plain text, so operators have no special
    /// meaning. Results are marked stale when a file changed after indexing.
    ///
    /// Example: `playbook code search retry --lang python`
    Search {
        /// Words to look for
        query: String,
        /// Search indexed text only
        #[arg(long)]
        text: bool,
        /// Only files under this path prefix, for example `lib/payment/`
        #[arg(long)]
        path: Option<String>,
        /// Only files of this language: rust, typescript, javascript or python
        #[arg(long, value_parser = ["rust", "typescript", "javascript", "python"])]
        lang: Option<String>,
        /// Most results to show, from 1 to 200
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=200))]
        limit: u32,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Find where a name is declared
    ///
    /// Matches the exact qualified name first (a top level symbol's qualified
    /// name is its plain name), then the name, then the name ignoring case,
    /// then the name as a prefix.
    ///
    /// Example: `playbook code symbol PaymentService --kind class`
    Symbol {
        /// The name to look up
        name: String,
        /// Only this kind, for example `function`, `class` or `method`
        #[arg(long)]
        kind: Option<String>,
        /// Only files of this language: rust, typescript, javascript or python
        #[arg(long, value_parser = ["rust", "typescript", "javascript", "python"])]
        lang: Option<String>,
        /// Only files under this path prefix, for example `lib/payment/`
        #[arg(long)]
        path: Option<String>,
        /// Most results to show, from 1 to 200
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=200))]
        limit: u32,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// List the declarations in one file
    ///
    /// Parses the file as it is on disk, so it works without an index and never
    /// shows old text. Prints at most 500 declarations, indented by nesting.
    ///
    /// Example: `playbook code outline lib/payment/service`
    Outline {
        /// The file to outline
        path: String,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Print numbered lines of one file
    ///
    /// Reads the file as it is on disk. Prints at most 400 lines or 32 KiB. A
    /// range that ends past the file is cut at the end with a warning.
    ///
    /// Example: `playbook code read lib/payment/service 10:40`
    Read {
        /// The file to read
        path: String,
        /// First and last line, 1 based and inclusive, as START:END
        #[arg(value_parser = parse_range_arg)]
        lines: (usize, usize),
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
}

fn parse_range_arg(spec: &str) -> Result<(usize, usize), String> {
    super::source::parse_range(spec)
}

/// What a command needs from its surroundings.
pub struct Env {
    pub cwd: PathBuf,
}

impl Env {
    pub fn real() -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_default(),
        }
    }
}

/// Runs one subcommand and returns what to print.
pub fn run(sub: CodeCommand, env: &Env) -> Result<String, String> {
    match sub {
        CodeCommand::Index { json, rebuild } => render_index(env, json, rebuild),
        CodeCommand::Status { json } => render_status(env, json),
        CodeCommand::Search {
            query,
            text,
            path,
            lang,
            limit,
            json,
        } => {
            let mut opts = SearchOptions::new(&query);
            opts.path = path.map(|p| normalize_prefix(&p));
            opts.lang = lang;
            opts.limit = limit as usize;
            let mode = if text { Mode::Text } else { Mode::Ranked };
            render_search(env, &opts, mode, json)
        }
        CodeCommand::Outline { path, json } => render_outline(env, &path, json),
        CodeCommand::Read { path, lines, json } => render_read(env, &path, lines, json),
        CodeCommand::Symbol {
            name,
            kind,
            lang,
            path,
            limit,
            json,
        } => {
            let mut opts = SearchOptions::new(&name);
            opts.kind = kind;
            opts.path = path.map(|p| normalize_prefix(&p));
            opts.lang = lang;
            opts.limit = limit as usize;
            render_search(env, &opts, Mode::Symbol, json)
        }
    }
}

/// A path prefix as the index stores paths: forward slashes, no leading `./`.
fn normalize_prefix(prefix: &str) -> String {
    let slashed = prefix.replace('\\', "/");
    let mut rest = slashed.as_str();
    while let Some(next) = rest.strip_prefix("./") {
        rest = next;
    }
    rest.to_string()
}

/// Which lookup a search-shaped command runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Text,
    Ranked,
    Symbol,
}

fn render_search(
    env: &Env,
    opts: &SearchOptions,
    mode: Mode,
    json: bool,
) -> Result<String, String> {
    let command = match mode {
        Mode::Symbol => "symbol",
        Mode::Text | Mode::Ranked => "search",
    };
    let out = match mode {
        Mode::Text => super::search_text(&env.cwd, opts)?,
        Mode::Ranked => super::search_ranked(&env.cwd, opts)?,
        Mode::Symbol => super::symbol_lookup(&env.cwd, opts)?,
    };
    let state = status::state_of(&out.drift);
    let mut warnings = Vec::new();
    if state == IndexState::Stale {
        warnings.push(status::stale_warning(&out.drift));
    }
    if out.result.hits.iter().any(|h| h.stale) {
        warnings.push("some results changed after they were indexed".to_string());
    }
    if out.result.scan_capped {
        warnings.push(format!(
            "searched only the first {} files, so results may be incomplete",
            out.result.files_scanned.unwrap_or(0)
        ));
    }
    if json {
        let results: Vec<serde_json::Value> = out
            .result
            .hits
            .iter()
            .map(|h| {
                let mut row = json!({
                    "path": h.path,
                    "start_line": h.start_line,
                    "end_line": h.end_line,
                    "line": h.line,
                    "match_type": h.match_type,
                    "tier": h.tier,
                    "excerpt": h.excerpt,
                    "stale": h.stale,
                });
                if let (Some(sym), Some(map)) = (&h.symbol, row.as_object_mut()) {
                    map.insert("name".into(), json!(sym.name));
                    map.insert("qualified_name".into(), json!(sym.qualified_name));
                    map.insert("kind".into(), json!(sym.kind));
                    map.insert("lang".into(), json!(sym.lang));
                    map.insert("signature".into(), json!(sym.signature));
                    map.insert("exported".into(), json!(sym.exported));
                }
                row
            })
            .collect();
        let mut envelope = Envelope::new(
            command,
            state,
            out.index_path.display().to_string(),
            serde_json::Value::Array(results),
        );
        envelope.truncated = out.result.truncated;
        envelope.omitted = out.result.omitted;
        envelope.warnings = warnings;
        if let Some(n) = out.result.files_scanned {
            envelope.stats = Some(json!({"files_scanned": n}));
        }
        return Ok(envelope.to_json());
    }
    for warning in &warnings {
        eprintln!("code: {warning}");
    }
    let mut text = String::new();
    for h in &out.result.hits {
        let stale = if h.stale { " (stale)" } else { "" };
        let shown = match &h.symbol {
            Some(sym) => format!(
                "{} {}  {}",
                sym.kind,
                sym.qualified_name,
                sym.signature.as_deref().unwrap_or("")
            ),
            None => h
                .excerpt
                .lines()
                .nth(h.line.saturating_sub(h.excerpt_start))
                .unwrap_or("")
                .to_string(),
        };
        text.push_str(&format!(
            "{}:{}-{}{stale}  {}\n",
            h.path,
            h.start_line,
            h.end_line,
            shown.trim()
        ));
    }
    if out.result.omitted > 0 {
        text.push_str(&format!("{} more results not shown\n", out.result.omitted));
    }
    if out.result.scan_capped {
        let n = out.result.files_scanned.unwrap_or(0);
        text.push_str(&format!(
            "searched only the first {n} files, so results may be incomplete\n"
        ));
    } else if out.result.hits.is_empty() {
        text.push_str("no matches\n");
    }
    Ok(text)
}

fn render_outline(env: &Env, input: &str, json: bool) -> Result<String, String> {
    let root = super::location::repo_root(&env.cwd)?;
    let out = super::source::outline(&root, &env.cwd, input, super::source::OUTLINE_CAP)?;
    let mut warnings = Vec::new();
    if out.partial {
        warnings
            .push("the file has syntax errors, so some declarations may be missing".to_string());
    }
    if json {
        let rows: Vec<serde_json::Value> = out
            .entries
            .iter()
            .map(|e| {
                json!({
                    "depth": e.depth,
                    "kind": e.kind,
                    "name": e.name,
                    "qualified_name": e.qualified_name,
                    "signature": e.signature,
                    "start_line": e.start_line,
                    "end_line": e.end_line,
                    "exported": e.exported,
                })
            })
            .collect();
        let mut envelope = Envelope::new(
            "outline",
            IndexState::NotUsed,
            String::new(),
            json!({"path": out.path, "lang": out.lang, "entries": rows}),
        );
        envelope.truncated = out.truncated;
        envelope.omitted = out.omitted;
        envelope.warnings = warnings;
        return Ok(envelope.to_json());
    }
    for warning in &warnings {
        eprintln!("code: {warning}");
    }
    let mut text = String::new();
    for e in &out.entries {
        text.push_str(&format!(
            "{}{}:{}-{}  {}\n",
            "  ".repeat(e.depth),
            e.kind,
            e.start_line,
            e.end_line,
            e.signature.as_deref().unwrap_or(&e.name)
        ));
    }
    if out.truncated {
        text.push_str(&format!("{} more declarations not shown\n", out.omitted));
    }
    if out.entries.is_empty() {
        text.push_str("no declarations\n");
    }
    Ok(text)
}

fn render_read(
    env: &Env,
    input: &str,
    range: (usize, usize),
    json: bool,
) -> Result<String, String> {
    let root = super::location::repo_root(&env.cwd)?;
    let out = super::source::read_lines(
        &root,
        &env.cwd,
        input,
        range,
        super::source::Caps::default(),
    )?;
    let mut warnings = Vec::new();
    if out.clamped {
        warnings.push(format!(
            "the range ends past the file; showing up to line {} of {}",
            out.end, out.total_lines
        ));
    }
    if json {
        let mut envelope = Envelope::new(
            "read",
            IndexState::NotUsed,
            String::new(),
            json!({
                "path": out.path,
                "start_line": out.start,
                "end_line": out.end,
                "total_lines": out.total_lines,
                "text": out.text,
            }),
        );
        envelope.truncated = out.truncated;
        envelope.warnings = warnings;
        return Ok(envelope.to_json());
    }
    for warning in &warnings {
        eprintln!("code: {warning}");
    }
    let mut text = String::new();
    for (offset, line) in out.text.lines().enumerate() {
        text.push_str(&format!("{:>6}  {line}\n", out.start + offset));
    }
    if out.truncated {
        text.push_str("output truncated\n");
    }
    Ok(text)
}

fn render_status(env: &Env, json: bool) -> Result<String, String> {
    match super::status(&env.cwd)? {
        super::StatusOutcome::Missing { index_path } => {
            if json {
                let mut envelope = Envelope::new(
                    "status",
                    IndexState::Missing,
                    index_path.display().to_string(),
                    json!({}),
                );
                envelope.warnings = vec![format!("no index yet; run `{}`", host::command("index"))];
                return Ok(envelope.to_json());
            }
            Ok(format!(
                "index: missing; run `{}`\n",
                host::command("index")
            ))
        }
        super::StatusOutcome::Found { index_path, status } => {
            if json {
                let mut envelope = Envelope::new(
                    "status",
                    status.state,
                    index_path.display().to_string(),
                    status.to_json(),
                );
                if status.state == IndexState::Stale {
                    envelope.warnings = vec![super::status::stale_warning(&status.drift)];
                }
                return Ok(envelope.to_json());
            }
            Ok(status.to_text())
        }
    }
}

fn render_index(env: &Env, json: bool, rebuild: bool) -> Result<String, String> {
    let outcome = super::index(&env.cwd, rebuild)?;
    let report = &outcome.report;
    let mut warnings = Vec::new();
    if report.failed > 0 {
        warnings.push(format!(
            "{} files could not be read and keep their previous index rows",
            report.failed
        ));
    }
    if json {
        let mut envelope = Envelope::new(
            "index",
            IndexState::Fresh,
            outcome.index_path.display().to_string(),
            json!({
                "indexed": report.indexed,
                "changed": report.changed,
                "unchanged": report.unchanged,
                "parsed": report.parsed,
                "removed": report.removed,
                "skipped": report.skipped,
                "failed": report.failed,
            }),
        );
        envelope.warnings = warnings;
        return Ok(envelope.to_json());
    }
    for warning in &warnings {
        eprintln!("code: {warning}");
    }
    Ok(format!(
        "indexed {} files: {} changed, {} unchanged, {} removed, {} skipped, {} failed\n",
        report.indexed,
        report.changed,
        report.unchanged,
        report.removed,
        report.skipped,
        report.failed
    ))
}

#[cfg(test)]
mod tests {
    use super::normalize_prefix;

    #[test]
    fn a_prefix_loses_leading_dot_slashes_and_backslashes() {
        // Arrange, Act, Assert
        assert_eq!(normalize_prefix("./lib/"), "lib/");
        assert_eq!(normalize_prefix("././lib"), "lib");
        assert_eq!(normalize_prefix("lib\\pay"), "lib/pay");
        assert_eq!(normalize_prefix("lib/"), "lib/");
    }
}
