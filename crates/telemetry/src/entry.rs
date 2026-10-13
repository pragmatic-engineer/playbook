// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook usage` and its subcommands: picks the terminal view or the text or
//! JSON output.

use super::command::UsageCommand;
use super::period::Period;
use super::query::{self, Range, WeekStart};
use super::run::{ingest_new, run_summary, Paths};
use super::{api, db, tui};
use rusqlite::Connection;

/// What the removed `usage ingest` says to anyone who still calls it.
const INGEST_REMOVED: &str =
    "`usage ingest` was removed: use `playbook usage --summary`, the terminal view warms itself";

/// The flags of `playbook usage` itself.
pub struct Opts {
    pub json: bool,
    pub summary: bool,
    /// The `--range` value, or `None` when the flag was not given.
    pub range: Option<String>,
}

/// Whether the terminal view can run: it needs a terminal on both ends.
pub fn interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// The `--json` document: the totals and groups, the limits and burn rate
/// under `limits`, plus
/// the live view under `live`, and for today, week and month the comparison
/// with the previous period under `comparison`.
pub fn json_report(
    conn: &Connection,
    paths: &Paths,
    range: Range,
    now: i64,
) -> Result<String, String> {
    let usage = query::load_window(conn, range, now)?;
    let tools = db::load_tool_events(conn)?;
    let mut data = api::data_json(&usage, &tools, now, range);
    let i = query::load_live_inputs(conn, now)?;
    let live = api::live_json(&i.window, &i.sessions, &i.latest, now);
    let since = super::limits_source::events_since(now, configured_week_start());
    let limit_events = db::load_usage_events_since(conn, since)?;
    let limits = super::run::limits_json(paths, &limit_events, now);
    if let Some(map) = data.as_object_mut() {
        map.insert("live".to_string(), live);
        map.insert("limits".to_string(), limits);
        if let Some(period) = Period::of_range(range) {
            let events = db::load_usage_events_since(conn, period.windows(now).previous.start)?;
            map.insert(
                "comparison".to_string(),
                api::comparison_json(&events, now, period),
            );
        }
    }
    serde_json::to_string_pretty(&data).map_err(|e| format!("failed to encode usage data: {e}"))
}

/// Runs the command. The returned text, when not empty, is printed on stdout.
pub fn run(
    sub: Option<UsageCommand>,
    opts: &Opts,
    paths: &Paths,
    interactive: bool,
) -> Result<String, String> {
    // Every command starts from a data file that exists (created owner-only,
    // with older data moved in first), so no command errors on a fresh home.
    // This runs before the terminal view takes over the screen.
    super::tz::ensure_init();
    super::db::open_db(&paths.db).map(drop)?;
    match sub {
        Some(UsageCommand::Ingest) => Err(INGEST_REMOVED.to_string()),
        Some(UsageCommand::Warm) => super::warm::run_warm(paths).map(|()| String::new()),
        Some(UsageCommand::Collect { port, idle_seconds }) => {
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            super::collector::run_collect(paths, port, idle_seconds, &stop)
        }
        Some(UsageCommand::Prune { dry_run }) => super::collector::run_prune(paths, dry_run),
        Some(UsageCommand::Status { json }) => {
            let status = super::collector::status(paths, super::collector::configured_retention())?;
            if json {
                serde_json::to_string_pretty(&status)
                    .map_err(|e| format!("failed to encode the collector status: {e}"))
            } else {
                Ok(status.text(crate::util::time::now_secs()))
            }
        }
        None if opts.summary => {
            let range = opts.range.as_deref().map(parse_range).transpose()?;
            run_summary(paths, range, crate::util::time::now_secs())
        }
        None if opts.json => {
            let range = parse_range(opts.range.as_deref().unwrap_or(DEFAULT_RANGE))?;
            let (conn, _) = ingest_new(paths)?;
            json_report(&conn, paths, range, crate::util::time::now_secs())
        }
        None if interactive => {
            tui::run(
                paths,
                parse_range(opts.range.as_deref().unwrap_or(TUI_DEFAULT_RANGE))?,
            )?;
            Ok(String::new())
        }
        None => {
            let range = opts.range.as_deref().map(parse_range).transpose()?;
            run_summary(paths, range, crate::util::time::now_secs())
        }
    }
}

/// The range of `--json` when `--range` is not given.
const DEFAULT_RANGE: &str = "30d";

/// The range the terminal view opens on when `--range` is not given: this week
/// against last week.
const TUI_DEFAULT_RANGE: &str = "week";

/// The first day of the week, from the `usage.weekStart` config key. Monday
/// when the key is unset or unreadable.
pub fn configured_week_start() -> WeekStart {
    super::host::config_value_now("usage.weekStart")
        .and_then(|value| value.as_str().and_then(WeekStart::parse))
        .unwrap_or_default()
}

/// Seconds between refreshes of the terminal view, from the
/// `usage.refreshSeconds` config key.
pub fn configured_refresh_seconds() -> u64 {
    super::host::config_value_now("usage.refreshSeconds")
        .and_then(|value| value.as_u64())
        .filter(|n| *n > 0)
        .unwrap_or(tui::worker::DEFAULT_REFRESH_SECONDS)
}

/// Parses a range name with the configured week start.
pub fn parse_range(value: &str) -> Result<Range, String> {
    Range::parse_with(Some(value), configured_week_start()).ok_or_else(|| {
        format!(
            "unknown range `{value}`; use today, week, last-week, month, last-month, 30d, 60d, 90d or all"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_names_one_of_the_nine_values() {
        for ok in [
            "today",
            "week",
            "last-week",
            "month",
            "last-month",
            "30d",
            "60d",
            "90d",
            "all",
        ] {
            assert!(parse_range(ok).is_ok(), "{ok}");
        }
        let err = parse_range("7d").unwrap_err();
        assert!(err.contains("unknown range `7d`"), "{err}");
    }
}
