// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The command line shared by `playbook-telemetry` and its `pbtop` alias.
use crate::command::UsageCommand;
use crate::entry::{interactive, run, Opts};
use crate::run::Paths;
use clap::{CommandFactory, FromArgMatches, Parser};

/// Show token and cost usage across your Claude Code sessions
///
/// With no subcommand, opens the terminal view, which opens on this week
/// compared with last week. When stdout is not a terminal, or with
/// `--summary`, prints a text summary of estimated cost instead. `--json`
/// prints the same numbers as JSON for scripts. Times and days use your local
/// time zone, shown in the output; `--utc` switches to UTC, or set `usage.timeZone`
/// or `PLAYBOOK_TELEMETRY_TZ` to `local` or `utc`. Ranges: today,
/// week, last-week, month, last-month, 30d, 60d, 90d, all.
///
/// Example: `pbtop --range today`. `pbtop` is the short alias of
/// `playbook-telemetry`, which is the long name for scripts.
#[derive(Parser, Debug)]
#[command(name = "playbook-telemetry", version)]
struct Cli {
    #[command(subcommand)]
    sub: Option<UsageCommand>,
    /// Print the totals, groupings and live view as JSON
    #[arg(long, conflicts_with_all = ["summary"])]
    json: bool,
    /// Print the text summary and exit (every recorded session unless `--range` is given)
    #[arg(long, conflicts_with_all = ["json"])]
    summary: bool,
    /// Date range (days in the chosen time zone): today, week, last-week, month, last-month, 30d, 60d, 90d or all. Default week for the terminal view, 30d for `--json`, all for `--summary`
    #[arg(long, value_name = "RANGE")]
    range: Option<String>,
    /// Show times and group days in UTC (overrides `usage.timeZone` and `PLAYBOOK_TELEMETRY_TZ`)
    #[arg(long, global = true, conflicts_with = "local")]
    utc: bool,
    /// Show times and group days in your local time zone, the default (overrides `usage.timeZone` and `PLAYBOOK_TELEMETRY_TZ`)
    #[arg(long, global = true, conflicts_with = "utc")]
    local: bool,
}

/// The name this program was invoked as: `pbtop` or `playbook-telemetry`.
fn invoked_name() -> &'static str {
    let name = std::env::args_os()
        .next()
        .and_then(|a| {
            std::path::Path::new(&a)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .filter(|n| n == "pbtop")
        .unwrap_or_else(|| "playbook-telemetry".to_string());
    Box::leak(name.into_boxed_str())
}

/// The first help line, which names both commands.
fn alias_line(name: &str) -> &'static str {
    if name == "pbtop" {
        "pbtop (long name: playbook-telemetry)"
    } else {
        "playbook-telemetry (alias: pbtop)"
    }
}

/// Shared entry point of both binaries, `playbook-telemetry` and its `pbtop` alias.
pub fn main() {
    let name = invoked_name();
    let matches = Cli::command()
        .name(name)
        .before_help(alias_line(name))
        .get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    let flag = match (cli.utc, cli.local) {
        (true, _) => Some(crate::tz::Zone::Utc),
        (_, true) => Some(crate::tz::Zone::Local),
        _ => None,
    };
    crate::tz::init(flag);
    let paths = Paths::real();
    let opts = Opts {
        json: cli.json,
        summary: cli.summary,
        range: cli.range,
    };
    match run(cli.sub, &opts, &paths, interactive()) {
        Ok(output) => {
            if !output.is_empty() {
                println!("{}", output.trim_end());
            }
        }
        Err(err) => {
            eprintln!("{name}: {err}");
            std::process::exit(1);
        }
    }
}
