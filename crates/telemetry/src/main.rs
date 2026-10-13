// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `playbook-telemetry` command: a terminal view and reports of Claude Code
//! token and cost usage, read from local session history.

fn main() {
    playbook_telemetry::cli::main();
}
