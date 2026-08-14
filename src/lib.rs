// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Library root for the `playbook` binary. Exposes the CLI shape that
//! `main.rs` parses and dispatches on.

use clap::{Parser, Subcommand};

/// The `playbook` command-line entry point.
#[derive(Parser, Debug)]
#[command(name = "playbook", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level subcommand groups. Each is a stub until its own Work Unit
/// implements the behaviour.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run a named hook.
    Hook { name: String },
    /// Launcher subcommands (session, worktree, retention, and so on).
    Cc { sub: String },
    /// Print the Claude Code status line.
    Statusline,
    /// Install or repair the local Claude Code configuration.
    Init,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn version_flag_prints_cargo_toml_version() {
        // Arrange
        let expected = env!("CARGO_PKG_VERSION");

        // Act
        let result = Cli::command().try_get_matches_from(["playbook", "--version"]);

        // Assert
        let err =
            result.expect_err("--version should short-circuit parsing with a version message");
        assert!(err.to_string().contains(expected));
    }
}
