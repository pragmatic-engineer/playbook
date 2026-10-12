// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `usage` subcommands, as a clap enum the host program embeds.

use clap::Subcommand;

/// `playbook usage` subcommands.
#[derive(Subcommand, Debug)]
pub enum UsageCommand {
    /// Warm the usage store in the background (run by the `usage-warm` hook)
    #[command(hide = true)]
    Warm,
    /// Removed: use `playbook usage --summary`, the terminal view warms itself
    #[command(hide = true)]
    Ingest,
    /// Run the local OpenTelemetry receiver in the foreground
    ///
    /// Listens on 127.0.0.1 only (default port 4318, the OTLP/HTTP port; a free
    /// port when that one is taken) and stores API request cost and token rows,
    /// tool call timings and API errors from Claude Code's JSON telemetry.
    /// Prompts, tool content, paths, emails and user ids are never stored. A
    /// second start reports the running collector (`playbook usage status`
    /// shows it too). It stops by itself after the idle time, counted from the
    /// last real telemetry request, so a status check does not keep it alive.
    /// Use `--idle-seconds 0` to keep it running in a tmux window. Sessions that sent no
    /// telemetry are read from transcripts as before. While it runs, `cc`
    /// points its sessions at it unless `usage.telemetry` is `off` or you set
    /// telemetry yourself. When none runs, `cc` starts one itself in the
    /// background (once the usage database exists) under the same conditions.
    ///
    /// Example: `playbook usage collect --idle-seconds 1800`
    Collect {
        /// Port to listen on (an error when busy). Default: the last port, 4318, then any free port.
        #[arg(long)]
        port: Option<u16>,
        /// Stop after this many seconds with no request; 0 means never
        #[arg(long, default_value_t = 600)]
        idle_seconds: u64,
    },
    /// Show whether the telemetry receiver is running
    ///
    /// Prints how telemetry is configured (on with the collector running, off because you
    /// disabled it in your Claude Code settings, or off because nothing sets it) with a one-line
    /// fix, then the pid, port, idle limit, how many telemetry rows are stored
    /// (API requests, tool calls, API errors, metric points), the retention limits and the time of the newest
    /// event. It starts nothing and never prints the token. With `--json` it
    /// prints the same as JSON. Exit code 0 whether or not a receiver runs.
    ///
    /// Example: `playbook usage status --json`
    Status {
        /// Print the status as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete old telemetry rows
    ///
    /// Removes telemetry rows older than `usage.telemetryRetentionDays` (default
    /// 90, 0 keeps them) and the oldest rows beyond `usage.telemetryMaxRows` per
    /// table (default 500000, 0 means no limit). The receiver does the same when
    /// it starts and every few hours. Transcript data and every total are left
    /// alone, because totals come from transcripts.
    ///
    /// Example: `playbook usage prune --dry-run`
    Prune {
        /// Say what would be removed without deleting anything
        #[arg(long)]
        dry_run: bool,
    },
}
