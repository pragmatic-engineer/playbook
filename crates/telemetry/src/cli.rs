// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The command line of `pbtop`, the standalone usage dashboard (ADR 0041).
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
/// Example: `pbtop --range today`. With playbook installed, `playbook usage`
/// runs the same code.
#[derive(Parser, Debug)]
#[command(name = "pbtop", bin_name = "pbtop", version)]
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

/// The entry point of `pbtop`.
/// The command line of `pbtop`. `usage spend` belongs to playbook, which holds
/// the credential code, so it is hidden here.
fn command() -> clap::Command {
    Cli::command().mut_subcommand("spend", |spend| spend.hide(true))
}

pub fn main() {
    let cli = match Cli::from_arg_matches(&command().get_matches()) {
        Ok(cli) => cli,
        Err(e) => e.exit(),
    };
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
            eprintln!("pbtop: {err}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbtop_does_not_list_the_spend_command() {
        let help = command().render_help().to_string();

        assert!(!help.contains("spend"), "{help}");
        assert!(help.contains("doctor"));
    }
}
