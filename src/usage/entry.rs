// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook usage` and its subcommands: picks the terminal view, the text or
//! JSON output, or the deprecated web dashboard.

use super::query::{self, Range};
use super::run::{ingest_new, run_ingest, run_summary, Paths};
use super::{api, dashboard, db, tui};
use crate::{DashboardCommand, UsageCommand};
use rusqlite::Connection;

/// The release that removes the web dashboard and the old subcommands.
const REMOVED_IN: &str = "v0.22.0";

/// The flags of `playbook usage` itself.
pub struct Opts {
    pub json: bool,
    pub summary: bool,
    pub web: bool,
    pub range: String,
}

/// Whether the terminal view can run: it needs a terminal on both ends.
pub fn interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn deprecated(what: &str, instead: &str) {
    eprintln!("usage: {what} is deprecated and will be removed in {REMOVED_IN}. {instead}");
}

/// The `--json` document: the dashboard's data without its SVG charts, plus
/// the live view under `live`.
pub fn json_report(conn: &Connection, range: Range, now: i64) -> Result<String, String> {
    let usage = query::load_window(conn, range, now)?;
    let tools = db::load_tool_events(conn)?;
    let mut data = api::data_json(&usage, &tools, now, range);
    let i = query::load_live_inputs(conn, now)?;
    let mut live = api::live_json(&i.window, &i.sessions, &i.latest, now);
    if let Some(map) = data.as_object_mut() {
        map.remove("charts");
    }
    if let Some(map) = live.as_object_mut() {
        map.remove("burn_chart");
    }
    if let Some(map) = data.as_object_mut() {
        map.insert("live".to_string(), live);
    }
    serde_json::to_string_pretty(&data).map_err(|e| format!("failed to encode usage data: {e}"))
}

fn web(paths: &Paths) -> Result<String, String> {
    let exe = std::env::current_exe().unwrap_or_default();
    // Test seam: lets spawned-binary tests skip launching a real browser.
    // Never set in production.
    if std::env::var_os("PLAYBOOK_USAGE_NO_BROWSER").is_some() {
        dashboard::run_dashboard(paths, &exe, &dashboard::no_browser)
    } else {
        dashboard::run_dashboard(paths, &exe, &dashboard::open_in_browser)
    }
}

/// Runs the command. The returned text, when not empty, is printed on stdout.
pub fn run(
    sub: Option<UsageCommand>,
    opts: &Opts,
    paths: &Paths,
    interactive: bool,
) -> Result<String, String> {
    match sub {
        Some(UsageCommand::Ingest) => {
            deprecated(
                "`usage ingest`",
                "Use `playbook usage --summary`, which also reads new session history.",
            );
            run_ingest(paths)
        }
        Some(UsageCommand::Dashboard { serve: true, .. }) => {
            dashboard::serve(paths).map(|()| String::new())
        }
        Some(UsageCommand::Dashboard {
            sub: Some(DashboardCommand::Stop),
            ..
        }) => {
            deprecated(
                "`usage dashboard stop`",
                "The web dashboard is replaced by the terminal view, `playbook usage`.",
            );
            dashboard::run_stop(paths)
        }
        Some(UsageCommand::Dashboard { .. }) => {
            deprecated(
                "`usage dashboard`",
                "Use `playbook usage` for the terminal view, or `playbook usage --web` until then.",
            );
            web(paths)
        }
        None if opts.web => {
            deprecated(
                "the web dashboard (`--web`)",
                "Use `playbook usage` for the terminal view.",
            );
            web(paths)
        }
        None if opts.summary => run_summary(paths),
        None if opts.json => {
            let range = parse_range(&opts.range)?;
            let (conn, _) = ingest_new(paths)?;
            json_report(&conn, range, crate::common::time::now_secs())
        }
        None if interactive => {
            tui::run(paths, parse_range(&opts.range)?)?;
            Ok(String::new())
        }
        None => run_summary(paths),
    }
}

fn parse_range(value: &str) -> Result<Range, String> {
    Range::parse(Some(value))
        .ok_or_else(|| format!("unknown range `{value}`; use 30d, 60d, 90d, month or all"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_names_one_of_the_five_values() {
        for ok in ["30d", "60d", "90d", "month", "all"] {
            assert!(parse_range(ok).is_ok(), "{ok}");
        }
        let err = parse_range("7d").unwrap_err();
        assert!(err.contains("unknown range `7d`"), "{err}");
    }
}
