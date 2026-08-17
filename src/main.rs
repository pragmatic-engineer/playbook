// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

use clap::Parser;
use playbook::common::payload::Payload;
use playbook::{hooks, Cli, Command};
use std::io::{IsTerminal, Read};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Hook { name } => {
            let raw = read_hook_input();
            let payload = Payload::parse(&raw);
            hooks::dispatch(name, &payload);
        }
        // Launcher subcommands land in a later Work Unit; stub for now.
        Command::Cc { sub: _ } => {}
        // RESERVED, not planned. No ADR 0007 Work Unit ports this: `statusline.sh`
        // stays a shell script and WU-9 only places it where `settings.json`
        // points. Do not read this arm as work in flight; see the blueprint's
        // third 2026-08-17 amendment.
        Command::Statusline => {}
        // Installer/repair flow is wired by WU-11, together with retiring
        // `hooks/hooks.json` and regenerating the seed. Those three are one
        // atomic switchover: any two without the third leaves users with no
        // functional hooks, or hooks pointing at a binary that is not installed.
        Command::Init => {}
    }
}

/// Read the hook payload the same way common.py does: `HOOK_INPUT` env var
/// if set and non-empty, else all of stdin when stdin is not a tty, else
/// empty. Never panics; a read failure yields an empty payload.
fn read_hook_input() -> String {
    if let Ok(value) = std::env::var("HOOK_INPUT") {
        if !value.is_empty() {
            return value;
        }
    }
    if std::io::stdin().is_terminal() {
        return String::new();
    }
    let mut buf = String::new();
    let _ = std::io::stdin().read_to_string(&mut buf);
    buf
}
